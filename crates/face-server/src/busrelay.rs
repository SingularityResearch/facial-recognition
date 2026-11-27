//! Copies of the other nodes' trees, kept for this device's pages (the subscribing half of the bus, `bus.rs`).
//!
//! One follower per node holds one connection to that node's peer service for as long as a page asks about
//! it: TLS with the node's certificate checked against the SHA-256 this device's member list has for it, this
//! device's own certificate shown in the handshake (the node checks it the same way), and the request signed
//! with the cluster's key. What comes back is that node's whole tree and then every change as it is published;
//! each is taken into a copy here (`Bus::apply`, which checks every path and size), and this device's pages
//! subscribe to the copy through their own server, exactly as they subscribe to this device's own tree.
//!
//! No server is asked for any of it: where a node is and which certificate it must show come from this
//! device's own member list (`face_db::members`), and the key from what this device keeps. A follower nobody
//! asked for in `IDLE` closes its connection and forgets the copy. Nothing is stored.

// What another device or the network sends is read here, and a panic ends the whole process (`panic = "abort"`):
// clippy shows each unwrap, index and plain sum of this module, to be looked at (not in its tests).
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects))]

use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};

use face_db::{Device, kept::Peer, members, peer::sign};
use serde_json::{Value, json};

use crate::{
    bus::{Bus, Cursor},
    camrelay::{Parts, askable, order, pinned_as, unix_now, why},
};

/// a follower nobody asked for this long closes its connection
const IDLE: Duration = Duration::from_secs(60);
/// followers at a time: a node id is whatever a request says, and each follower is a thread that connects
pub const MAX_FOLLOWED: usize = 64;
/// to wait for a node's next answer before the connection counts as gone: it sends one every few seconds
const STREAM_READ: Duration = Duration::from_secs(20);
const TOO_MANY: &str = "too many nodes are being followed from this device";
const REMOVED: &str = "that device is removed on this one";
/// asking the members for their people takes this long at most; then the caller asks elsewhere
const PEOPLE_WITHIN: Duration = Duration::from_secs(20);
/// an answer that began in time may take this much longer to arrive whole (a people file is megabytes)
const WHOLE_GRACE: Duration = Duration::from_secs(40);
/// a follower's trouble is said in the log again only after this long, however often it meets it
const SAID_AGAIN: Duration = Duration::from_secs(600);
/// the people file of a member that this device couldn't use: member id -> the file's SHA-256
static UNUSABLE: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
/// what a question about a node that can't be followed now waits at most
const TROUBLE_WAIT: Duration = Duration::from_secs(1);

struct Seen {
    used: Instant,
    error: String,
    stopped: bool,
}

/// What asking the members for their people gave.
pub enum People {
    /// a people file (its bytes as the member has them), with its SHA-256
    New(Vec<u8>, String),
    /// the member's file is the one this device holds already
    Same,
}

/// The people of the cluster for a device that has no library of its own, asked of the members directly: the
/// first member that shares and has a people file gives it (`peer.rs` `/people`; a phone has none to give).
/// `have`: the SHA-256 of the file this device holds already. An error when no member gave one: the caller
/// then asks elsewhere. No server is asked: the members, the key and this device's certificate are its own.
pub fn people(
    home: &Path,
    own_port: u16,
    have: Option<&str>,
    usable: &dyn Fn(&[u8]) -> bool,
) -> Result<People, String> {
    let end = Instant::now() + PEOPLE_WITHIN;
    let key = members::kept_key(home).ok_or("this device doesn't have the cluster's key yet")?;
    let me = Device::load(home, &crate::device().0)?;
    let own = crate::peer::own_certificate(home, &me.name)?;
    let removed = face_home::blocked::ids(home);
    let all = members::kept(home);
    let mine = all.iter().find(|m| m.id == me.id).map(|m| m.addresses.clone()).unwrap_or_default();
    let path = match have {
        Some(sha) => format!("/people?have={sha}"),
        None => "/people".to_string(),
    };
    let mut last = "no other device shares".to_string();
    for node in all.iter().filter(|m| m.id != me.id && m.share && m.port != 0 && !m.cert_sha256.is_empty()) {
        if removed.contains(&node.id) {
            continue; // a device removed on this one is asked for nothing
        }
        if Instant::now() >= end {
            return Err("the other devices took too long to answer".to_string());
        }
        // (a member whose file this device couldn't use is asked whether it is still that file, not for it)
        let unusable = UNUSABLE.lock().unwrap_or_else(|e| e.into_inner()).get(&node.id).cloned();
        let path = match (&unusable, have) {
            (Some(sha), _) => format!("/people?have={sha}"),
            _ => path.clone(),
        };
        let peer = Peer {
            name: node.name.clone(),
            addresses: node.addresses.clone(),
            port: node.port,
            cert_sha256: node.cert_sha256.clone(),
        };
        for addr in order(&askable(&peer.addresses, home), peer.port, None, &mine, &mine, own_port) {
            match whole(&addr, &peer, &key, &me.id, &path, (own.0.clone(), own.1.clone_key()), end) {
                Ok(Some((ctype, body))) if ctype.starts_with("application/json") => {
                    let said: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let same = said.get("same").and_then(Value::as_str);
                    if unusable.is_some() && same == unusable.as_deref() {
                        last = "a device's people file can't be used here (made for another version)".to_string();
                        break; // still the file this device can't use: the next member
                    }
                    if have.is_some() && same == have {
                        return Ok(People::Same);
                    }
                    last = "a device answered what isn't a people file".to_string();
                }
                Ok(Some((_, body))) => {
                    let sha: String = ring::digest::digest(&ring::digest::SHA256, &body)
                        .as_ref()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    let mut bad = UNUSABLE.lock().unwrap_or_else(|e| e.into_inner());
                    if usable(&body) {
                        bad.remove(&node.id);
                        return Ok(People::New(body, sha));
                    }
                    // not one this device can use (another recognition model, or no file at all): remembered,
                    // so that it isn't fetched again every minute, and the next member is asked
                    if bad.len() < MAX_FOLLOWED {
                        bad.insert(node.id.clone(), sha);
                    }
                    last = "a device's people file can't be used here (made for another version)".to_string();
                    break;
                }
                Ok(None) => {
                    last = "the devices that answered have no people of their own".to_string();
                    break; // this member has none: the next member, not its next address
                }
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}

/// One whole answer of a member's peer service: (content type, body) for a 200 that arrived whole and is at
/// most `MAX_PEOPLE` long, None for "not found", an error for anything else.
fn whole(
    addr: &str,
    peer: &Peer,
    key: &[u8; 32],
    me: &str,
    path: &str,
    own: (rustls::pki_types::CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>),
    end: Instant,
) -> Result<Option<(String, Vec<u8>)>, String> {
    let e = |e: std::io::Error| e.to_string();
    let mut tls = pinned_as(addr, peer, Some(own))?;
    let when = unix_now().to_string();
    let ask = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nX-Peer-Device: {me}\r\nX-Peer-Time: {when}\r\nX-Peer-Sig: {}\r\n\r\n",
        sign(key, "GET", path, &when, me)
    );
    tls.write_all(ask.as_bytes()).map_err(e)?;
    let most = crate::peer::MAX_PEOPLE + 65_536;
    let (mut data, mut chunk) = (Vec::new(), vec![0u8; 65_536]);
    while data.len() <= most {
        // (a device that sends a byte now and then is not waited for without end)
        if Instant::now() >= end + WHOLE_GRACE {
            return Err("the answer took too long".to_string());
        }
        match tls.read(&mut chunk) {
            Ok(0) => break,
            Ok(got) => data.extend_from_slice(&chunk[..got]),
            // (a device that closes without TLS's goodbye has still said all it had: the length is checked below)
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(e(err)),
        }
    }
    let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") else { return Err("no answer".to_string()) };
    let (head, body) = (String::from_utf8_lossy(&data[..at]).into_owned(), &data[at + 4..]);
    let line = |name: &str| {
        head.split("\r\n").skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    match head.split("\r\n").next().and_then(|l| l.split_whitespace().nth(1)) {
        Some("200") => {}
        Some("404") => return Ok(None),
        Some("403") => return Err(format!("refused: {}", why(body, unix_now()))),
        other => return Err(format!("answered {}", other.unwrap_or("?"))),
    }
    // only whole: the length the device said, and no more than a people file may be
    let length = line("content-length").and_then(|n| n.parse::<usize>().ok());
    if length != Some(body.len()) || body.len() > crate::peer::MAX_PEOPLE {
        return Err("the answer didn't arrive whole".to_string());
    }
    Ok(Some((line("content-type").unwrap_or_default().to_ascii_lowercase(), body.to_vec())))
}

/// One followed node: the copy of its tree, and how the following goes.
struct Follower {
    device: String,
    copy: Bus,
    seen: Mutex<Seen>,
}

impl Follower {
    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct Followed {
    home: PathBuf,
    /// this device's own peer service's port, 0 while it has none
    own_port: Arc<AtomicU16>,
    followers: Mutex<HashMap<String, Arc<Follower>>>,
    /// a node's certificate -> its address that answered last
    good: Mutex<HashMap<String, String>>,
}

impl Followed {
    pub fn new(home: &Path, own_port: Arc<AtomicU16>) -> Arc<Followed> {
        Arc::new(Followed {
            home: home.to_path_buf(),
            own_port,
            followers: Mutex::new(HashMap::new()),
            good: Mutex::new(HashMap::new()),
        })
    }

    /// What a page asks of another node's bus: the answer of that node's copy here (`Bus::since`), with
    /// `error` beside it while the node can't be followed. The following starts with the first question.
    pub fn since(self: &Arc<Self>, device: &str, after: Option<&Cursor>, prefixes: &[String], wait: Duration) -> Value {
        let Some(follower) = self.get(device) else { return json!({"error": TOO_MANY}) };
        // (a subscriber that begins while the copy holds nothing yet waits for the node's tree, not for an
        // empty one)
        let end = Instant::now() + wait;
        let mut answer = loop {
            let left = end.saturating_duration_since(Instant::now());
            let (began, trouble) = (follower.copy.cursor(), !follower.seen().error.is_empty());
            let begins = after.is_none() && follower.copy.get("node").is_none();
            if !begins {
                // (a node that can't be followed now: answered soon, with why, not after the whole wait)
                break follower.copy.since(after, prefixes, if trouble { left.min(TROUBLE_WAIT) } else { left });
            }
            if trouble || left.is_zero() {
                break follower.copy.since(None, prefixes, Duration::ZERO);
            }
            let _ = follower.copy.since(Some(&began), &[], left.min(Duration::from_millis(250)));
        };
        let error = follower.seen().error.clone();
        if !error.is_empty() {
            answer["error"] = json!(error);
        }
        answer
    }

    /// What a page asks of several nodes' buses in one question: {nodes: {id: answer}} with an answer for
    /// each node that has something to say (its tree, or changes), as soon as any has; after `wait` with
    /// none, the nodes that can't be followed now, each with why. One waiting question for a page however
    /// many nodes it follows: a browser gives a page a handful of connections to one address, and a question
    /// for each node took them all from the cameras' pictures.
    pub fn since_many(
        self: &Arc<Self>,
        asked: &[(String, Option<Cursor>)],
        prefixes: &[String],
        wait: Duration,
        own: Option<(&str, &Bus)>,
    ) -> Value {
        let end = Instant::now() + wait;
        // (this device's own node, where it is among them, is answered from its own bus: nothing is followed)
        let is_own = |id: &str| own.is_some_and(|(me, _)| me == id);
        let followers: Vec<Option<Arc<Follower>>> =
            asked.iter().map(|(id, _)| if is_own(id) { None } else { self.get(id) }).collect();
        loop {
            let (mut out, mut trouble) = (serde_json::Map::new(), serde_json::Map::new());
            for ((id, cursor), follower) in asked.iter().zip(&followers) {
                if let Some((_, bus)) = own.filter(|_| is_own(id)) {
                    let answer = bus.since(cursor.as_ref(), prefixes, Duration::ZERO);
                    if answer.get("whole").is_some()
                        || answer.get("events").and_then(Value::as_array).is_some_and(|e| !e.is_empty())
                    {
                        out.insert(id.clone(), answer);
                    }
                    continue;
                }
                let Some(follower) = follower else {
                    trouble.insert(id.clone(), json!({"error": TOO_MANY}));
                    continue;
                };
                follower.seen().used = Instant::now();
                let mut answer = follower.copy.since(cursor.as_ref(), prefixes, Duration::ZERO);
                let error = follower.seen().error.clone();
                if !error.is_empty() {
                    answer["error"] = json!(error);
                }
                let news = answer.get("whole").is_some()
                    || answer.get("events").and_then(Value::as_array).is_some_and(|e| !e.is_empty());
                if news {
                    out.insert(id.clone(), answer);
                } else if !error.is_empty() {
                    trouble.insert(id.clone(), answer);
                }
            }
            if !out.is_empty() {
                return json!({"nodes": out});
            }
            if Instant::now() >= end {
                return json!({"nodes": trouble});
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    /// What the copy of a node's tree holds at a path, while this device follows that node (nothing is
    /// started for the asking: a node nobody's page follows has no copy here).
    pub fn value(&self, device: &str, path: &str) -> Option<Value> {
        let followers = self.followers.lock().unwrap_or_else(|e| e.into_inner());
        followers.get(device).filter(|f| !f.seen().stopped).and_then(|f| f.copy.get(path))
    }

    /// The follower of a node, started when it is first asked for; None when `MAX_FOLLOWED` run already.
    fn get(self: &Arc<Self>, device: &str) -> Option<Arc<Follower>> {
        let mut followers = self.followers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = followers.get(device).filter(|f| !f.seen().stopped) {
            f.seen().used = Instant::now();
            return Some(f.clone());
        }
        if followers.len() >= MAX_FOLLOWED {
            return None;
        }
        let follower = Arc::new(Follower {
            device: device.to_string(),
            copy: Bus::new(),
            seen: Mutex::new(Seen { used: Instant::now(), error: String::new(), stopped: false }),
        });
        followers.insert(device.to_string(), follower.clone());
        let (all, mine) = (self.clone(), follower.clone());
        let short: String = device.chars().take(8).collect();
        let started = std::thread::Builder::new().name(format!("bus follower {short}")).spawn(move || all.run(&mine));
        if started.is_err() {
            followers.remove(device);
            return None;
        }
        Some(follower)
    }

    /// A follower's life: connect, take in what the node publishes, connect again when the connection drops,
    /// until nobody has asked for `IDLE`.
    fn run(self: &Arc<Self>, follower: &Arc<Follower>) {
        let short: String = follower.device.chars().take(8).collect();
        let over = |f: &Follower| {
            let s = f.seen();
            s.stopped || s.used.elapsed() > IDLE
        };
        // (each trouble is said when it is new and again once in a while: a device that sleeps is met with two
        // troubles in turn, one for each of its addresses, every few seconds)
        let mut said: HashMap<String, Instant> = HashMap::new();
        while !over(follower) {
            let tried = self.follow(follower, &over);
            if let Err(e) = tried {
                // unreachable, refused, dropped, or an answer that can't be taken: try again shortly
                let e: String = e.chars().take(200).collect();
                if said.get(&e).is_none_or(|at| at.elapsed() > SAID_AGAIN) {
                    println!("Bus follower {short}: {e}");
                    said.retain(|_, at| at.elapsed() <= SAID_AGAIN);
                    said.insert(e.clone(), Instant::now());
                }
                let removed = e == REMOVED;
                follower.seen().error = e;
                if removed {
                    break; // removed on this device: nothing more is asked of it
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        follower.seen().stopped = true;
        follower.copy.clear();
        let mut followers = self.followers.lock().unwrap_or_else(|e| e.into_inner());
        if followers.get(&follower.device).is_some_and(|f| Arc::ptr_eq(f, follower)) {
            followers.remove(&follower.device);
        }
    }

    /// One connection's life: Ok when it ended because nobody asks any more.
    fn follow(&self, follower: &Follower, over: &dyn Fn(&Follower) -> bool) -> Result<(), String> {
        let (tls, first) = self.open(follower)?;
        follower.seen().error.clear();
        let mut parts = Parts::new(tls, first);
        while !over(follower) {
            // (a node whose app was closed or started again drops the connection without a word: said plainly)
            let dropped = |e: std::io::Error| match e.kind() {
                std::io::ErrorKind::UnexpectedEof => "the node's connection ended".to_string(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                    "the node sent nothing for a while".to_string()
                }
                _ => e.to_string(),
            };
            let Some((ctype, body)) = parts.next().map_err(dropped)? else {
                return Err("the node closed the connection".to_string());
            };
            if !ctype.starts_with("application/json") {
                continue;
            }
            let answer: Value = serde_json::from_slice(&body).map_err(|_| "the node sent what isn't an answer")?;
            if let Err(e) = follower.copy.apply(&answer) {
                // (its numbers no longer fit this copy: the copy is dropped, and the whole tree asked for again)
                follower.copy.clear();
                return Err(e);
            }
        }
        Ok(())
    }

    /// The connection to a node's bus, asked for everything after this copy's place, and what of the stream
    /// came with its head.
    fn open(&self, follower: &Follower) -> Result<(crate::camrelay::Tls, Vec<u8>), String> {
        // (a device removed on this one is asked for nothing: also by a follower that ran before it was removed)
        if face_home::blocked::is(&self.home, &follower.device) {
            return Err(REMOVED.to_string());
        }
        let node = members::member(&self.home, &follower.device).ok_or("this device doesn't know that node")?;
        if !node.share || node.port == 0 || node.cert_sha256.is_empty() {
            return Err(format!("{} doesn't share with the other devices", node.name));
        }
        let key = members::kept_key(&self.home).ok_or("this device doesn't have the cluster's key yet")?;
        let name = crate::device().0;
        let me = Device::load(&self.home, &name)?;
        let own = crate::peer::own_certificate(&self.home, &me.name)?;
        // where this device is itself, as its own line of the member list says: never asked at its own port
        let mine = members::member(&self.home, &me.id).map(|m| m.addresses).unwrap_or_default();
        let peer =
            Peer { name: node.name.clone(), addresses: node.addresses, port: node.port, cert_sha256: node.cert_sha256 };
        let good = self.good.lock().unwrap_or_else(|e| e.into_inner()).get(&peer.cert_sha256).cloned();
        let at = follower.copy.cursor();
        // (a copy that holds nothing yet asks for the whole tree)
        let path = match follower.copy.get("node") {
            Some(_) => format!("/events?after={}&epoch={}", at.seq, at.epoch),
            None => "/events".to_string(),
        };
        let mut last = "no address".to_string();
        let port = self.own_port.load(Ordering::Relaxed);
        for addr in order(&askable(&peer.addresses, &self.home), peer.port, good.as_deref(), &mine, &mine, port) {
            match self.connect(&addr, &peer, &key, &me.id, &path, (own.0.clone(), own.1.clone_key())) {
                Ok(open) => {
                    self.good.lock().unwrap_or_else(|e| e.into_inner()).insert(peer.cert_sha256.clone(), addr);
                    return Ok(open);
                }
                Err(e) => {
                    let mut good = self.good.lock().unwrap_or_else(|e| e.into_inner());
                    if good.get(&peer.cert_sha256) == Some(&addr) {
                        good.remove(&peer.cert_sha256);
                    }
                    last = format!("{addr}:{}: {e}", peer.port);
                }
            }
        }
        Err(format!("{} can't be reached ({last})", peer.name))
    }

    #[allow(clippy::type_complexity)]
    fn connect(
        &self,
        addr: &str,
        peer: &Peer,
        key: &[u8; 32],
        me: &str,
        path: &str,
        own: (rustls::pki_types::CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>),
    ) -> Result<(crate::camrelay::Tls, Vec<u8>), String> {
        let e = |e: std::io::Error| e.to_string();
        let mut tls = pinned_as(addr, peer, Some(own))?;
        let when = unix_now().to_string();
        let ask = format!(
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nX-Peer-Device: {me}\r\nX-Peer-Time: {when}\r\nX-Peer-Sig: {}\r\n\r\n",
            sign(key, "GET", path, &when, me)
        );
        tls.write_all(ask.as_bytes()).map_err(e)?;
        let (mut head, mut chunk) = (Vec::new(), [0u8; 4096]);
        let end = loop {
            if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            if head.len() >= 65_536 {
                return Err(format!("{} answered ?", peer.name));
            }
            match tls.read(&mut chunk).map_err(e)? {
                0 => return Err(format!("{} answered ?", peer.name)),
                got => head.extend_from_slice(&chunk[..got]),
            }
        };
        let (text, rest) = (String::from_utf8_lossy(&head[..end]).into_owned(), head[end + 4..].to_vec());
        let status = text.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("?");
        if status != "200" {
            return Err(match status {
                "403" => format!("{} refused: {}", peer.name, why(&rest, unix_now())),
                // (a node that runs an earlier version has no bus to ask)
                "404" => format!("{} has no bus yet (an earlier version of the app)", peer.name),
                other => format!("{} answered {other}", peer.name),
            });
        }
        tls.sock.set_read_timeout(Some(STREAM_READ)).map_err(e)?;
        Ok((tls, rest))
    }
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, sync::RwLock};

    use super::*;
    use crate::{camstream::Stream, peer};

    const DB: &str = "0b0e7a52-6f0c-4b6e-9d53-1c2f3a4b5c6d";
    const LONG: Duration = Duration::from_secs(20);

    /// A device's private folder as a member of a made-up cluster has it: the database's settings with a
    /// password (the cluster's key is made of it), and the device's own id.
    fn a_home(tag: &str) -> (PathBuf, Device) {
        let home = std::env::temp_dir().join(format!("face-bus-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // ("a password", kept as the programs keep a password where the system has no protection for it)
        let settings = r#"{"host": "192.0.2.9", "user": "u", "password": "plain:YSBwYXNzd29yZA=="}"#;
        std::fs::write(home.join("database.json"), settings).unwrap();
        let device = Device::load(&home, &format!("node {tag}")).unwrap();
        (home, device)
    }

    fn member(home: &Path, device: &Device, port: u16) -> members::Member {
        members::Member {
            id: device.id.clone(),
            name: device.name.clone(),
            addresses: if port == 0 { vec![] } else { vec!["127.0.0.1".to_string()] },
            port,
            cert_sha256: peer::cert_sha256(home, &device.name).unwrap(),
            share: true,
        }
    }

    fn cursor(answer: &Value) -> Cursor {
        Cursor { epoch: answer["epoch"].as_str().unwrap().to_string(), seq: answer["seq"].as_u64().unwrap() }
    }

    #[test]
    fn a_node_follows_another_nodes_bus_and_only_a_member_with_its_own_certificate_may() {
        let ((home_a, a), (home_b, b), (home_c, c)) = (a_home("a"), a_home("b"), a_home("c"));
        // node A publishes, on this computer alone
        let bus = Arc::new(Bus::new());
        bus.set("node", json!({"id": a.id, "device": "a computer"})).unwrap();
        bus.set("state/off", json!(["voice"])).unwrap();
        let key: peer::Key = Arc::new(RwLock::new(face_db::peer::derive_key("a password", DB)));
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let bytes = b"a people file's bytes".to_vec();
        let sha256: String =
            ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect();
        let file = Arc::new(peer::PeopleFile { bytes, sha256 });
        let gives = peer::Gives { photos: None, people: Some(Arc::new(move || Some(file.clone()))) };
        let service = peer::start_on(&home_a, &a.name, Stream::new(), key, listener, gives, bus.clone()).unwrap();
        let at_a = member(&home_a, &a, service.port);
        // A knows B as a member (by B's certificate); B and C know where A is and which certificate it shows
        members::keep(&home_a, Some(DB), &[member(&home_b, &b, 0)]).unwrap();
        members::keep(&home_b, Some(DB), &[at_a.clone(), member(&home_b, &b, 0)]).unwrap();
        members::keep(&home_c, Some(DB), &[at_a.clone(), member(&home_c, &c, 0)]).unwrap();

        // B follows A: the whole tree first, with A's own numbers
        let follows = Followed::new(&home_b, Arc::new(AtomicU16::new(0)));
        let first = follows.since(&a.id, None, &[], LONG);
        assert_eq!(first.get("error"), None, "{first}");
        assert_eq!(first["whole"], json!({"node": {"id": a.id, "device": "a computer"}, "state": {"off": ["voice"]}}));
        assert_eq!(cursor(&first), bus.cursor());
        // then what A publishes, as it publishes it: the waiting subscriber is told at once
        let (began, at) = (Instant::now(), cursor(&first));
        let publisher = bus.clone();
        let later = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            publisher.set("readings", json!({"rows": [[1, 2, 3]]})).unwrap();
        });
        let next = follows.since(&a.id, Some(&at), &[], LONG);
        later.join().unwrap();
        assert_eq!(next["events"][0]["path"], "readings");
        assert_eq!(next["events"][0]["value"], json!({"rows": [[1, 2, 3]]}));
        assert!(began.elapsed() < Duration::from_secs(8), "told when it happened: {:?}", began.elapsed());
        // one question for several nodes: answered as soon as any has something to say, with that node's answer
        let many = follows.since_many(&[(a.id.clone(), None)], &[], LONG, None);
        assert_eq!(many["nodes"][&a.id]["whole"]["node"]["device"], "a computer", "{many}");
        let place = cursor(&many["nodes"][&a.id]);
        let publisher = bus.clone();
        let later = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            publisher.say("readings", json!({"rows": [[7, 8, 9]]})).unwrap();
        });
        // (a node asked for the first time is answered at once, with its tree, empty for one that can't be
        // followed, and the place to ask from)
        let unknown = "77777777-2222-4333-8444-555555555555".to_string();
        let first = follows.since_many(&[(unknown.clone(), None)], &[], LONG, None);
        assert_eq!(first["nodes"][&unknown]["whole"], json!({}), "{first}");
        let from = cursor(&first["nodes"][&unknown]);
        let began = Instant::now();
        let asked = [(a.id.clone(), Some(place.clone())), (unknown.clone(), Some(from.clone()))];
        let news = follows.since_many(&asked, &["readings".to_string()], LONG, None);
        later.join().unwrap();
        assert!(began.elapsed() < Duration::from_secs(8), "told when it happened: {:?}", began.elapsed());
        let heard = &news["nodes"][&a.id]["events"][0];
        assert_eq!(
            (&heard["path"], &heard["value"], &heard["once"]),
            (&json!("readings"), &json!({"rows": [[7, 8, 9]]}), &json!(true))
        );
        // (what was said once is not kept: a subscriber that comes later is not given it as if it were now)
        // (what it gets there is what was set, above)
        let kept = follows.since(&a.id, None, &[], LONG);
        assert_eq!(kept["whole"]["readings"], json!({"rows": [[1, 2, 3]]}), "{kept}");
        // with nothing to say, the nodes that can't be followed are named when the wait is over, each with why
        let began = Instant::now();
        let quiet = follows.since_many(&[(unknown.clone(), Some(from))], &[], Duration::from_millis(1500), None);
        // this device's own node among them is answered from its own bus: nothing is followed for it
        let (mine, me) = (Bus::new(), b.id.clone());
        mine.set("state/running", json!(true)).unwrap();
        let own = follows.since_many(&[(me.clone(), None)], &[], LONG, Some((&me, &mine)));
        assert_eq!(own["nodes"][&me]["whole"], json!({"state": {"running": true}}), "{own}");
        mine.say("readings", json!({"rows": []})).unwrap();
        let said = follows.since_many(&[(me.clone(), Some(cursor(&own["nodes"][&me])))], &[], LONG, Some((&me, &mine)));
        assert_eq!(said["nodes"][&me]["events"][0]["path"], "readings", "{said}");
        assert!(!follows.followers.lock().unwrap().contains_key(&me), "it followed itself");
        assert!(began.elapsed() >= Duration::from_millis(1400), "it waited: {:?}", began.elapsed());
        assert!(
            quiet["nodes"][&unknown]["error"].as_str().is_some_and(|e| e.contains("doesn't know that node")),
            "{quiet}"
        );
        // a subscriber to one path of the copy hears only that path
        bus.set("state/off", json!([])).unwrap();
        bus.set("readings", json!({"rows": [[4, 5, 6]]})).unwrap();
        let only = follows.since(&a.id, Some(&cursor(&next)), &["state".to_string()], LONG);
        let heard: Vec<&str> = only["events"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
        assert_eq!(heard, ["state/off"]);

        // B has no library: it takes A's people file directly, whole, and is told when its copy is A's already
        let any = |_: &[u8]| true;
        let People::New(taken, sha) = people(&home_b, 0, None, &any).unwrap() else {
            panic!("no people file was given")
        };
        assert_eq!(taken, b"a people file's bytes");
        assert_eq!(sha.len(), 64);
        assert!(matches!(people(&home_b, 0, Some(&sha), &any), Ok(People::Same)));
        let other = "00".repeat(32);
        assert!(matches!(people(&home_b, 0, Some(&other), &any), Ok(People::New(..))), "another copy: the file again");
        // a file this device can't use is no answer: no other member has one, so there is none, and the file is
        // not fetched again while it is still that file (the member is asked "is it still this one?")
        let unusable = people(&home_b, 0, None, &|_| false).err().expect("an unusable file is no people");
        assert!(unusable.contains("can't be used here"), "{unusable}");
        let again = people(&home_b, 0, None, &|_| panic!("the same file was fetched again")).err().unwrap();
        assert!(again.contains("can't be used here"), "{again}");
        assert!(matches!(people(&home_b, 0, None, &any), Err(_)), "still remembered as unusable while it is that file");
        UNUSABLE.lock().unwrap().clear();
        // C is no member to A: no people either
        let refused = people(&home_c, 0, None, &any).err().expect("a stranger is given no people");
        assert!(refused.contains("doesn't know that certificate"), "{refused}");

        // C has the cluster's key and knows A, but A's member list doesn't know C: nothing of the bus
        let stranger = Followed::new(&home_c, Arc::new(AtomicU16::new(0)));
        let refused = stranger.since(&a.id, None, &[], Duration::from_secs(4));
        assert!(refused["error"].as_str().is_some_and(|e| e.contains("doesn't know that certificate")), "{refused}");
        assert_eq!(refused.get("whole").and_then(|w| w.get("node")), None);
        // C says it is B (B's id in its request) and shows its own certificate: A's list has B's, not this one
        std::fs::copy(home_b.join("device.json"), home_c.join("device.json")).unwrap();
        let posing = Followed::new(&home_c, Arc::new(AtomicU16::new(0)));
        let refused = posing.since(&a.id, None, &[], Duration::from_secs(4));
        assert!(refused["error"].as_str().is_some_and(|e| e.contains("doesn't know that certificate")), "{refused}");
        // a device of another cluster (another password): refused before anything else
        std::fs::write(
            home_c.join("database.json"),
            r#"{"host": "192.0.2.9", "user": "u", "password": "plain:YW5vdGhlcg=="}"#,
        )
        .unwrap();
        let outsider = Followed::new(&home_c, Arc::new(AtomicU16::new(0)));
        let refused = outsider.since(&a.id, None, &[], Duration::from_secs(4));
        assert!(refused["error"].as_str().is_some_and(|e| e.contains("not signed")), "{refused}");
        // B, removed on A by A's owner: a new subscription of B's is refused, whatever B's certificate says
        face_home::blocked::set(&home_a, &b.id, true).unwrap();
        let removed = Followed::new(&home_b, Arc::new(AtomicU16::new(0)));
        let refused = removed.since(&a.id, None, &[], Duration::from_secs(4));
        assert!(refused["error"].as_str().is_some_and(|e| e.contains("not to share")), "{refused}");
        // B removed on A as the machine it is: with the certificate it had then. Whoever has the database's
        // login can write a row with a new id and that certificate; A's list then knows the id, and B under
        // that id is still no member to A.
        let cert_b = peer::cert_sha256(&home_b, &b.name).unwrap();
        face_home::blocked::set_with(&home_a, &b.id, true, Some(&cert_b)).unwrap();
        let other_id = "66666666-2222-4333-8444-555555555555";
        let renamed = members::Member { id: other_id.to_string(), ..member(&home_b, &b, 0) };
        members::keep(&home_a, None, &[renamed]).unwrap();
        let (home_b2, _) = a_home("b2");
        for file in ["peer-cert.pem", "peer-key.pem", "members.json", "database.json"] {
            std::fs::copy(home_b.join(file), home_b2.join(file)).unwrap();
        }
        std::fs::write(home_b2.join("device.json"), json!({"id": other_id, "name": "node b"}).to_string()).unwrap();
        let refused = people(&home_b2, 0, None, &any).err().expect("the removed machine under a new id");
        assert!(refused.contains("doesn't know that certificate"), "{refused}");
        let again = Followed::new(&home_b2, Arc::new(AtomicU16::new(0)));
        let refused = again.since(&a.id, None, &[], Duration::from_secs(4));
        assert!(refused["error"].as_str().is_some_and(|e| e.contains("doesn't know that certificate")), "{refused}");
        let _ = std::fs::remove_dir_all(home_b2);
        // and nothing of A's people for B once B is removed on A
        let refused = people(&home_b, 0, None, &any).err().expect("a removed device is given no people");
        assert!(refused.contains("not to share"), "{refused}");
        // a node this device doesn't know is not looked for anywhere
        let unknown = follows.since("77777777-2222-4333-8444-555555555555", None, &[], Duration::from_secs(2));
        assert!(unknown["error"].as_str().is_some_and(|e| e.contains("doesn't know that node")), "{unknown}");
        service.stop();
        for home in [home_a, home_b, home_c] {
            let _ = std::fs::remove_dir_all(home);
        }
    }
}
