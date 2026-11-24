//! Joining: a new device gets what a device of a shared database has (the database's login, and the owner's
//! passphrase for keys when that device holds it) from a device that has it, by a code read off a screen (a
//! QR code).
//!
//! Either device can be the one that shows the code:
//!
//! - a device of the database shows one and the new device reads it (`Offer`, `ask`);
//! - the new device shows one and a device of the database reads it (`Want`, `give`): a phone that is in
//!   the database lets in a computer whose screen it is pointed at.
//!
//! The code is a second factor, not the secret. It holds a key made for this one showing, where the showing
//! device listens on the local network, until when, which of the two ways it is for, and the showing
//! device's name (to ask the owner about, never trusted). The device that read it connects there, each
//! proves to the other with the key that it is the one that showed or saw the screen, and what is handed
//! over crosses encrypted under a key that both derive from the code's key and a key exchange made for this
//! one time. So:
//!
//! - a picture of the code is worth nothing once the code was used or its two minutes are over;
//! - what crosses the network gives nothing to anybody who watches it, and not later either to someone who
//!   then gets to see the code (the exchange's own keys are gone by then);
//! - a code is for one device: the first right exchange ends it, and so do `WRONG_ASKS` wrong ones;
//! - a code only ever sends a device to an address of a local network;
//! - a device of the database hands over only to a device that proved it holds the code's key, so nothing
//!   that merely sits at the code's address gets anything.
//!
//! Codes of the first kind (a device of the database shows, the login alone is handed
//! over) are still read, so a new device can join from a device that was not updated yet. None is shown.

use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use ring::{
    aead, agreement, hmac,
    rand::{SecureRandom, SystemRandom},
};

use crate::{Result, schema_name};

/// What a join code starts with, of any version: anything else a camera reads is not one.
pub const PREFIX: &str = "facetracker-join:";
/// seconds a code is good for
pub const VALID: u64 = 120;
/// where a showing device listens when the port is free (the programs' ports 8764 to 8769 come before it)
pub const PORT: u16 = 8770;
/// characters of a device's name a code carries
pub const NAME_MAX: usize = 40;
/// characters of a passphrase that is handed over
const PASSPHRASE_MAX: usize = 1000;
/// exchanges with a wrong key before a code is ended
const WRONG_ASKS: u32 = 5;
const CONNECT: Duration = Duration::from_secs(3);
const TALK: Duration = Duration::from_secs(3);
/// the first version's exchange
const MAGIC1: &[u8; 4] = b"FTJ1";
const MAGIC: &[u8; 4] = b"FTJ2";
/// the one that connects asks for what the showing device has
const ASKS: u8 = 1;
/// the one that connects brings it to the showing device
const GIVES: u8 = 2;
const TAKEN: &[u8] = b"taken";
const KEY: usize = 16;
const PUBLIC: usize = 32;
const TAG: usize = 32;
const MAX_ANSWER: usize = 16 * 1024;
const MAX_ADDRESSES: usize = 4;

/// The shared database's login, as one device hands it to another. Says nothing of itself in a log.
pub struct Login {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) dbname: String,
    pub(crate) user: String,
    pub(crate) password: String,
    pub(crate) schema: String,
}

impl Login {
    /// A login as the owner types it in Settings.
    pub fn new(host: &str, port: u16, dbname: &str, user: &str, password: &str, schema: &str) -> Login {
        Login {
            host: host.trim().to_string(),
            port,
            dbname: dbname.trim().to_string(),
            user: user.trim().to_string(),
            password: password.to_string(),
            schema: schema.to_string(),
        }
    }

    fn to_value(&self) -> serde_json::Value {
        serde_json::json!({"host": self.host, "port": self.port, "dbname": self.dbname, "user": self.user,
                           "password": self.password, "schema": self.schema})
    }

    /// (the first version's exchange carried the login alone, written so)
    #[cfg(test)]
    fn to_bytes(&self) -> Vec<u8> {
        self.to_value().to_string().into_bytes()
    }

    /// None for anything that is not a whole login: it came from another device.
    fn from_value(c: &serde_json::Value) -> Option<Login> {
        let text = |k: &str| c.get(k).and_then(|v| v.as_str()).filter(|v| v.len() <= 1000).map(str::to_string);
        let login = Login {
            host: text("host").filter(|h| !h.is_empty())?,
            port: c.get("port").and_then(|v| v.as_u64()).filter(|p| (1..=65535).contains(p))? as u16,
            dbname: text("dbname")?,
            user: text("user")?,
            password: text("password")?,
            schema: text("schema").filter(|s| schema_name(s))?,
        };
        Some(login)
    }

    fn from_bytes(raw: &[u8]) -> Option<Login> {
        Login::from_value(&serde_json::from_slice(raw).ok()?)
    }

    /// The same login for a device elsewhere: a server this device reaches as itself (the database runs
    /// here) is at this device's own address for the others.
    fn for_others(&self, here: &[IpAddr]) -> Login {
        let local = self.host.eq_ignore_ascii_case("localhost")
            || self.host.parse::<IpAddr>().is_ok_and(|a| a.is_loopback() || a.is_unspecified());
        Login {
            host: match here.first() {
                Some(a) if local => a.to_string(),
                _ => self.host.clone(),
            },
            port: self.port,
            dbname: self.dbname.clone(),
            user: self.user.clone(),
            password: self.password.clone(),
            schema: self.schema.clone(),
        }
    }
}

/// A device's name as a code or a handover may carry it: what another device said, so only ever shown as
/// text, without control characters and up to `NAME_MAX` characters.
pub fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(NAME_MAX).collect::<String>().trim().to_string()
}

/// What one device hands another to let it in: the database's login, the owner's passphrase for keys when
/// the handing device holds it, and that device's name. Says nothing of itself in a log.
pub struct Handover {
    pub login: Login,
    /// the passphrase for keys as the handing device has it (the taking device checks it against the
    /// database before it keeps it)
    pub passphrase: Option<String>,
    /// the handing device's name, as it says it
    pub from: String,
}

impl Handover {
    pub fn new(login: Login, passphrase: Option<String>, from: &str) -> Handover {
        Handover { login, passphrase: passphrase.filter(|p| !p.is_empty()), from: clean_name(from) }
    }

    fn to_bytes(&self, here: &[IpAddr]) -> Vec<u8> {
        serde_json::json!({"login": self.login.for_others(here).to_value(), "passphrase": self.passphrase,
                           "from": self.from})
        .to_string()
        .into_bytes()
    }

    /// None for anything that is not a whole handover: it came from another device.
    fn from_bytes(raw: &[u8]) -> Option<Handover> {
        let c: serde_json::Value = serde_json::from_slice(raw).ok()?;
        let passphrase = match c.get("passphrase") {
            None | Some(serde_json::Value::Null) => None,
            Some(p) => Some(p.as_str().filter(|p| !p.is_empty() && p.chars().count() <= PASSPHRASE_MAX)?.to_string()),
        };
        Some(Handover {
            login: Login::from_value(c.get("login")?)?,
            passphrase,
            from: clean_name(c.get("from").and_then(|v| v.as_str()).unwrap_or("")),
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// An address a code may send a device to: one of a local network (or this device itself), never one out
/// on the internet.
fn local(a: &IpAddr) -> bool {
    match a {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_private() || v.is_loopback() || v.is_link_local() || (o[0] == 100 && o[1] & 0xc0 == 64)
        }
        IpAddr::V6(v) => v.is_loopback() || v.segments()[0] & 0xfe00 == 0xfc00,
    }
}

/// What the device that shows a code does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// it is in a shared database and hands that on to the device that reads the code
    Gives,
    /// it wants to join: a device of a shared database that reads the code hands it over
    Wants,
}

/// A code as read off a screen: where to connect, the key that shows the reader saw it, and what for.
pub struct Code {
    key: [u8; KEY],
    port: u16,
    addrs: Vec<IpAddr>,
    kind: Kind,
    /// a code of the first kind (a device that was not updated shows it)
    first: bool,
    name: String,
}

impl Code {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The showing device's name as the code says it ("" when it says none): for the owner to read, not a
    /// proof of anything.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the showing device says it is.
    pub fn address(&self) -> Option<IpAddr> {
        self.addrs.first().copied()
    }
}

fn write_code(kind: Kind, key: &[u8; KEY], port: u16, expires: u64, name: &str, addrs: &[IpAddr]) -> String {
    let addrs: Vec<String> = addrs.iter().map(IpAddr::to_string).collect();
    let kind = if kind == Kind::Gives { "g" } else { "w" };
    let name = B64URL.encode(clean_name(name));
    // (the addresses last: one of the newer kind has colons of its own)
    format!("{PREFIX}2:{kind}:{}:{port}:{expires}:{name}:{}", B64URL.encode(key), addrs.join(","))
}

/// The code in a text a camera read, or why it can't be used.
pub fn read_code(text: &str, now: u64) -> Result<Code> {
    let bad = || "That is not a join code of this app.".to_string();
    let rest = text.trim().strip_prefix(PREFIX).ok_or_else(bad)?;
    let (version, rest) = rest.split_once(':').ok_or_else(bad)?;
    let (first, kind, mut parts) = match version {
        "1" => (true, Kind::Gives, rest.splitn(4, ':')),
        "2" => {
            let (kind, rest) = rest.split_once(':').ok_or_else(bad)?;
            let kind = match kind {
                "g" => Kind::Gives,
                "w" => Kind::Wants,
                _ => return Err(bad()),
            };
            (false, kind, rest.splitn(5, ':'))
        }
        v if !v.is_empty() && v.len() <= 3 && v.bytes().all(|b| b.is_ascii_digit()) => {
            return Err("That join code is from a newer version of the app: update this device first.".to_string());
        }
        _ => return Err(bad()),
    };
    let key: [u8; KEY] =
        parts.next().and_then(|k| B64URL.decode(k).ok()).and_then(|k| k.try_into().ok()).ok_or_else(bad)?;
    let port: u16 = parts.next().and_then(|p| p.parse().ok()).filter(|p| *p != 0).ok_or_else(bad)?;
    let expires: u64 = parts.next().and_then(|e| e.parse().ok()).ok_or_else(bad)?;
    let name = if first {
        String::new()
    } else {
        let raw = parts.next().filter(|n| n.len() <= 4 * NAME_MAX * 4 / 3 + 4).and_then(|n| B64URL.decode(n).ok());
        clean_name(&String::from_utf8(raw.ok_or_else(bad)?).map_err(|_| bad())?)
    };
    let addrs: Vec<IpAddr> =
        parts.next().ok_or_else(bad)?.split(',').map(|a| a.parse().ok()).collect::<Option<_>>().ok_or_else(bad)?;
    if addrs.is_empty() || addrs.len() > MAX_ADDRESSES || !addrs.iter().all(local) {
        return Err(bad());
    }
    if now > expires {
        return Err("That join code is no longer good: show a new one.".to_string());
    }
    if expires > now.saturating_add(2 * VALID) {
        return Err("That join code is dated too far ahead: are both devices' clocks right?".to_string());
    }
    Ok(Code { key, port, addrs, kind, first, name })
}

/// The key the two sides' check values are made with. `label` keeps the versions apart.
fn tag_key(label: &[u8], key: &[u8; KEY]) -> hmac::Key {
    let made = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, label), key);
    hmac::Key::new(hmac::HMAC_SHA256, made.as_ref())
}

fn tag_key1(key: &[u8; KEY]) -> hmac::Key {
    tag_key(b"facetracker-join:1 ask", key)
}

fn tag_key2(key: &[u8; KEY]) -> hmac::Key {
    tag_key(b"facetracker-join:2 tags", key)
}

const SEAL1: &[u8] = b"facetracker-join:1 login";
const SEAL_ASKED: &[u8] = b"facetracker-join:2 asked";
const SEAL_GIVEN: &[u8] = b"facetracker-join:2 given";

/// The key what is handed over is encrypted with: from the code's key, what the two sides' key exchange
/// gave, and both public halves of it (the connecting device's first). `label` keeps the versions and the
/// two ways apart.
fn seal_key(label: &[u8], key: &[u8; KEY], shared: &[u8], came: &[u8], shows: &[u8]) -> Option<aead::LessSafeKey> {
    let salt = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, label);
    let info = [came, shows];
    let prk = salt.extract(&[&key[..], shared].concat());
    let okm = prk.expand(&info, &aead::CHACHA20_POLY1305).ok()?;
    Some(aead::LessSafeKey::new(aead::UnboundKey::from(okm)))
}

/// One key exchange's own half: (the private key, its public half).
fn exchange() -> Option<(agreement::EphemeralPrivateKey, Vec<u8>)> {
    let private = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &SystemRandom::new()).ok()?;
    let public = private.compute_public_key().ok()?.as_ref().to_vec();
    Some((private, public))
}

/// A key made for one exchange encrypts at most two messages, each with its own number.
fn number(n: u8) -> aead::Nonce {
    let mut bytes = [0u8; 12];
    bytes[11] = n;
    aead::Nonce::assume_unique_for_key(bytes)
}

fn seal(k: &aead::LessSafeKey, n: u8, plain: &[u8]) -> Option<Vec<u8>> {
    let mut out = plain.to_vec();
    k.seal_in_place_append_tag(number(n), aead::Aad::from(MAGIC), &mut out).ok()?;
    Some(out)
}

fn open<'a>(k: &aead::LessSafeKey, n: u8, sealed: &'a mut [u8]) -> Option<&'a [u8]> {
    k.open_in_place(number(n), aead::Aad::from(MAGIC), sealed).ok().map(|plain| &*plain)
}

/// (the showing device's public half, `plain` encrypted for the device that asked, whose public half is
/// `asker`).
fn sealed(key: &[u8; KEY], asker: &[u8], plain: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let (private, public) = exchange()?;
    let theirs = agreement::UnparsedPublicKey::new(&agreement::X25519, asker);
    let k = agreement::agree_ephemeral(private, &theirs, |shared| seal_key(SEAL_ASKED, key, shared, asker, &public))
        .ok()??;
    Some((public, seal(&k, 0, plain)?))
}

fn talking(stream: &TcpStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(TALK));
    let _ = stream.set_write_timeout(Some(TALK));
    let _ = stream.set_nodelay(true);
}

/// The device that shows `code`, connected to.
fn reach(code: &Code) -> Result<TcpStream> {
    let stream = code
        .addrs
        .iter()
        .find_map(|a| TcpStream::connect_timeout(&SocketAddr::new(*a, code.port), CONNECT).ok())
        .ok_or("The device that shows the code can't be reached from this one: both have to be on the same network.")?;
    talking(&stream);
    Ok(stream)
}

/// How a showing ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// a device that read the code asked and was answered, from this address
    Given(IpAddr),
    /// nobody asked in time
    Expired,
    /// too many asks with a wrong key
    Refused,
    /// the one who showed it ended it
    Stopped,
}

enum Asked {
    Right,
    Wrong,
    Nothing,
}

/// What a showing device listens with, and until when.
struct Showing {
    key: [u8; KEY],
    expires: u64,
    until: Instant,
    listener: TcpListener,
    addrs: Vec<IpAddr>,
    name: String,
}

impl Showing {
    fn open(port: u16, addrs: Vec<IpAddr>, name: &str) -> Result<Showing> {
        if addrs.is_empty() {
            return Err("This device has no address on a local network: another device can't reach it.".to_string());
        }
        let listener = TcpListener::bind(("0.0.0.0", port))
            .or_else(|_| TcpListener::bind(("0.0.0.0", 0)))
            .map_err(|e| format!("This device can't listen for the other device: {e}"))?;
        Showing::on(listener, addrs, name)
    }

    fn on(listener: TcpListener, addrs: Vec<IpAddr>, name: &str) -> Result<Showing> {
        let mut key = [0u8; KEY];
        SystemRandom::new().fill(&mut key).map_err(|_| "This device couldn't make a key.".to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Showing {
            key,
            expires: unix_now() + VALID,
            until: Instant::now() + Duration::from_secs(VALID),
            listener,
            addrs,
            name: clean_name(name),
        })
    }

    fn code(&self, kind: Kind) -> String {
        let port = self.listener.local_addr().map_or(0, |a| a.port());
        write_code(kind, &self.key, port, self.expires, &self.name, &self.addrs)
    }

    fn left(&self) -> u64 {
        self.until.saturating_duration_since(Instant::now()).as_secs()
    }

    /// The next device that connects, or how the showing ended meanwhile.
    fn next(&self, stop: &AtomicBool) -> std::result::Result<(TcpStream, IpAddr), Outcome> {
        loop {
            if stop.load(Ordering::Relaxed) {
                return Err(Outcome::Stopped);
            }
            if Instant::now() >= self.until {
                return Err(Outcome::Expired);
            }
            match self.listener.accept() {
                Ok((stream, from)) => {
                    talking(&stream);
                    return Ok((stream, from.ip()));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}

/// A code being shown by a device of the database: it listens for the device that reads it.
pub struct Offer(Showing);

impl Offer {
    /// Start listening on the local network, on `port` (any free one when it is taken) for `VALID` seconds.
    /// `toward` is the database server's host and port: the address this device reaches it from is the one
    /// another device of that network finds this one at. `name` is this device's, for the code.
    pub fn open(port: u16, toward: (&str, u16), name: &str) -> Result<Offer> {
        Ok(Offer(Showing::open(port, own_addresses(toward), name)?))
    }

    /// The same on a listener of the caller's own, said to be at `addrs`: for a caller that listens where it
    /// chooses (a test on this computer alone).
    pub fn on(listener: TcpListener, addrs: Vec<IpAddr>, name: &str) -> Result<Offer> {
        Ok(Offer(Showing::on(listener, addrs, name)?))
    }

    /// The text to show as a QR code.
    pub fn code(&self) -> String {
        self.0.code(Kind::Gives)
    }

    /// seconds until the code is no longer good
    pub fn left(&self) -> u64 {
        self.0.left()
    }

    /// Wait for the device that read the code and hand it `what`: until one was answered, the time is
    /// over, too many asked with a wrong key, or `stop` is set. The listening ends with it.
    pub fn serve(self, what: &Handover, stop: &AtomicBool) -> Outcome {
        let plain = what.to_bytes(&self.0.addrs);
        let mut wrong = 0;
        loop {
            let (stream, from) = match self.0.next(stop) {
                Ok(came) => came,
                Err(ended) => return ended,
            };
            match self.answer(stream, &plain) {
                Asked::Right => return Outcome::Given(from),
                Asked::Wrong => {
                    wrong += 1;
                    if wrong >= WRONG_ASKS {
                        return Outcome::Refused;
                    }
                }
                Asked::Nothing => {}
            }
        }
    }

    fn answer(&self, mut stream: TcpStream, plain: &[u8]) -> Asked {
        let mut ask = [0u8; 4 + 1 + PUBLIC + TAG];
        if stream.read_exact(&mut ask).is_err() || &ask[..4] != MAGIC || ask[4] != ASKS {
            return Asked::Nothing;
        }
        let (theirs, tag) = ask[5..].split_at(PUBLIC);
        if hmac::verify(&tag_key2(&self.0.key), &[&b"ask"[..], theirs].concat(), tag).is_err() {
            return Asked::Wrong;
        }
        // the code is used from here on, whether or not the answer arrives
        if let Some((public, sealed)) = sealed(&self.0.key, theirs, plain) {
            let _ = stream.write_all(&[&public[..], &(sealed.len() as u32).to_be_bytes(), &sealed].concat());
            let _ = stream.flush();
        }
        Asked::Right
    }
}

/// How a new device's showing ended.
pub enum Received {
    /// a device of a shared database read the code and handed this over, from this address
    Taken(Box<Handover>, IpAddr),
    /// nobody read it in time
    Expired,
    /// too many tries with a wrong key
    Refused,
    /// the one who showed it ended it
    Stopped,
}

/// A code being shown by a device that wants to join: it listens for a device of a shared database that
/// reads it.
pub struct Want(Showing);

impl Want {
    /// Start listening on the local network, on `port` (any free one when it is taken) for `VALID` seconds.
    /// `name` is this device's, for the code.
    pub fn open(port: u16, name: &str) -> Result<Want> {
        Ok(Want(Showing::open(port, addresses(&[("192.0.2.1", 9)]), name)?))
    }

    /// The same on a listener of the caller's own, said to be at `addrs`: for a caller that listens where it
    /// chooses (a test on this computer alone).
    pub fn on(listener: TcpListener, addrs: Vec<IpAddr>, name: &str) -> Result<Want> {
        Ok(Want(Showing::on(listener, addrs, name)?))
    }

    /// The text to show as a QR code.
    pub fn code(&self) -> String {
        self.0.code(Kind::Wants)
    }

    /// seconds until the code is no longer good
    pub fn left(&self) -> u64 {
        self.0.left()
    }

    /// Wait for the device that read the code and take what it hands over: until one did, the time is
    /// over, too many came with a wrong key, or `stop` is set. The listening ends with it.
    pub fn receive(self, stop: &AtomicBool) -> Received {
        let mut wrong = 0;
        loop {
            let (stream, from) = match self.0.next(stop) {
                Ok(came) => came,
                Err(Outcome::Stopped) => return Received::Stopped,
                Err(_) => return Received::Expired,
            };
            match self.take(stream) {
                Ok(Some(handed)) => return Received::Taken(Box::new(handed), from),
                // (it had the key and the rest didn't arrive whole: the same device may come again)
                Ok(None) => {}
                Err(()) => {
                    wrong += 1;
                    if wrong >= WRONG_ASKS {
                        return Received::Refused;
                    }
                }
            }
        }
    }

    /// Err: it came with a wrong key. None: nothing of use came.
    fn take(&self, mut stream: TcpStream) -> std::result::Result<Option<Handover>, ()> {
        let mut hello = [0u8; 4 + 1 + PUBLIC + TAG];
        if stream.read_exact(&mut hello).is_err() || &hello[..4] != MAGIC || hello[4] != GIVES {
            return Ok(None);
        }
        let (theirs, tag) = hello[5..].split_at(PUBLIC);
        let tags = tag_key2(&self.0.key);
        if hmac::verify(&tags, &[&b"give"[..], theirs].concat(), tag).is_err() {
            return Err(());
        }
        let Some((private, public)) = exchange() else { return Ok(None) };
        // this device shows that it is the one whose screen was read, before anything is sent to it
        let mine = hmac::sign(&tags, &[&b"want"[..], &public, theirs].concat());
        if stream.write_all(&[&public[..], mine.as_ref()].concat()).is_err() {
            return Ok(None);
        }
        let mut size = [0u8; 4];
        if stream.read_exact(&mut size).is_err() || u32::from_be_bytes(size) as usize > MAX_ANSWER {
            return Ok(None);
        }
        let mut sealed = vec![0u8; u32::from_be_bytes(size) as usize];
        if stream.read_exact(&mut sealed).is_err() {
            return Ok(None);
        }
        let giver = agreement::UnparsedPublicKey::new(&agreement::X25519, theirs);
        let agreed = agreement::agree_ephemeral(private, &giver, |shared| {
            seal_key(SEAL_GIVEN, &self.0.key, shared, theirs, &public)
        });
        let Some(k) = agreed.ok().flatten() else { return Ok(None) };
        let Some(handed) = open(&k, 0, &mut sealed).and_then(Handover::from_bytes) else { return Ok(None) };
        // the handing device learns that it arrived (its page says so)
        if let Some(ack) = seal(&k, 1, TAKEN) {
            let _ = stream.write_all(&ack);
            let _ = stream.flush();
        }
        Ok(Some(handed))
    }
}

/// The addresses this device would send from toward each of `targets`, those of a local network. Nothing
/// is sent: the system is only asked which of its addresses it would send from.
fn addresses(targets: &[(&str, u16)]) -> Vec<IpAddr> {
    let mut out = Vec::new();
    for target in targets {
        let from = UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| s.connect(*target).and_then(|_| s.local_addr()));
        if let Ok(a) = from {
            let ip = a.ip();
            if local(&ip) && !ip.is_loopback() && !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// This device's addresses on the networks it reaches `toward` and the wider network from.
pub fn own_addresses(toward: (&str, u16)) -> Vec<IpAddr> {
    addresses(&[toward, ("192.0.2.1", 9)])
}

/// Ask the device of a shared database that shows `code` for what it hands over.
pub fn ask(code: &Code) -> Result<Handover> {
    let failed = || {
        "The device that shows the code didn't hand over the login: show a new code there and read it again."
            .to_string()
    };
    if code.kind != Kind::Gives {
        return Err("That code is shown by a device that wants to join, and this one isn't in a shared database \
                    either: read a code that a device of the database shows."
            .to_string());
    }
    let (magic, tags, label) =
        if code.first { (MAGIC1, tag_key1(&code.key), SEAL1) } else { (MAGIC, tag_key2(&code.key), SEAL_ASKED) };
    let mut stream = reach(code)?;
    let (private, public) = exchange().ok_or_else(failed)?;
    let tag = hmac::sign(&tags, &[&b"ask"[..], &public].concat());
    let role: &[u8] = if code.first { &[] } else { &[ASKS] };
    stream.write_all(&[&magic[..], role, &public, tag.as_ref()].concat()).map_err(|_| failed())?;
    let mut head = [0u8; PUBLIC + 4];
    stream.read_exact(&mut head).map_err(|_| failed())?;
    let (theirs, size) = head.split_at(PUBLIC);
    let size = u32::from_be_bytes(size.try_into().unwrap_or_default()) as usize;
    if size > MAX_ANSWER {
        return Err(failed());
    }
    let mut answer = vec![0u8; size];
    stream.read_exact(&mut answer).map_err(|_| failed())?;
    let holder = agreement::UnparsedPublicKey::new(&agreement::X25519, theirs);
    let key =
        agreement::agree_ephemeral(private, &holder, |shared| seal_key(label, &code.key, shared, &public, theirs))
            .ok()
            .flatten()
            .ok_or_else(failed)?;
    // only a device that has the code's key can have made this: nothing between the two can put in a login
    // of its own
    if code.first {
        // (the first version's exchange: the login alone, its own mark bound to it)
        let plain = key
            .open_in_place(aead::Nonce::assume_unique_for_key([0; 12]), aead::Aad::from(MAGIC1), &mut answer)
            .map_err(|_| failed())?;
        return Ok(Handover {
            login: Login::from_bytes(plain).ok_or_else(failed)?,
            passphrase: None,
            from: String::new(),
        });
    }
    open(&key, 0, &mut answer).and_then(Handover::from_bytes).ok_or_else(failed)
}

/// Hand `what` to the device that shows `code` and wants to join. `toward` is the database server's host and
/// port (as for `Offer::open`). Nothing is sent before that device showed that it holds the code's key.
pub fn give(code: &Code, what: &Handover, toward: (&str, u16)) -> Result<()> {
    let failed = || "The device that shows the code didn't take the login: show a new code there and read it again.";
    if code.kind != Kind::Wants || code.first {
        return Err(
            "That code is shown by a device that is in a shared database already: nothing is handed to it.".to_string()
        );
    }
    let mut stream = reach(code)?;
    let (private, public) = exchange().ok_or_else(failed)?;
    let tags = tag_key2(&code.key);
    let tag = hmac::sign(&tags, &[&b"give"[..], &public].concat());
    stream.write_all(&[&MAGIC[..], &[GIVES], &public, tag.as_ref()].concat()).map_err(|_| failed())?;
    let mut head = [0u8; PUBLIC + TAG];
    stream.read_exact(&mut head).map_err(|_| failed())?;
    let (theirs, shown) = head.split_at(PUBLIC);
    if hmac::verify(&tags, &[&b"want"[..], theirs, &public].concat(), shown).is_err() {
        return Err(
            "What answers at the code's address isn't the device that shows the code: nothing was handed over."
                .to_string(),
        );
    }
    let wants = agreement::UnparsedPublicKey::new(&agreement::X25519, theirs);
    let k =
        agreement::agree_ephemeral(private, &wants, |shared| seal_key(SEAL_GIVEN, &code.key, shared, &public, theirs))
            .ok()
            .flatten()
            .ok_or_else(failed)?;
    let sealed = seal(&k, 0, &what.to_bytes(&own_addresses(toward))).ok_or_else(failed)?;
    stream.write_all(&[&(sealed.len() as u32).to_be_bytes()[..], &sealed].concat()).map_err(|_| failed())?;
    stream.flush().map_err(|_| failed())?;
    let mut ack = [0u8; 5 + 16];
    stream.read_exact(&mut ack).map_err(|_| failed())?;
    match open(&k, 1, &mut ack) {
        Some(said) if said == TAKEN => Ok(()),
        _ => Err(failed().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn a_login() -> Login {
        Login {
            host: "db.example".into(),
            port: 5432,
            dbname: "people".into(),
            user: "app".into(),
            password: "a pass word / with + signs".into(),
            schema: "facerec".into(),
        }
    }

    fn a_handover() -> Handover {
        Handover::new(a_login(), Some("the owner's own passphrase".into()), "Desk computer")
    }

    fn here() -> Vec<IpAddr> {
        vec!["127.0.0.1".parse().unwrap()]
    }

    fn showing_here(name: &str) -> Showing {
        Showing::on(TcpListener::bind(("127.0.0.1", 0)).unwrap(), here(), name).unwrap()
    }

    /// The first kind's code, as a device that was not updated writes it.
    fn first_code(key: &[u8; KEY], port: u16, expires: u64, addrs: &str) -> String {
        format!("{PREFIX}1:{}:{port}:{expires}:{addrs}", B64URL.encode(key))
    }

    #[test]
    fn a_code_says_where_to_connect_what_for_and_goes_stale() {
        let key = [7u8; KEY];
        let addrs: Vec<IpAddr> = vec!["192.168.1.20".parse().unwrap(), "fd00::20".parse().unwrap()];
        for kind in [Kind::Gives, Kind::Wants] {
            let text = write_code(kind, &key, 8770, 1_000 + VALID, "Kitchen tablet", &addrs);
            assert!(text.starts_with(PREFIX) && text.len() < 160, "{} characters", text.len());
            let code = read_code(&format!("  {text}\n"), 1_000).unwrap();
            assert_eq!((code.key, code.port, &code.addrs, code.kind()), (key, 8770, &addrs, kind));
            assert_eq!((code.name(), code.first, code.address()), ("Kitchen tablet", false, Some(addrs[0])));
            let late = read_code(&text, 1_001 + VALID).err().unwrap();
            assert!(late.contains("no longer good"), "{late}");
            assert!(read_code(&text, 1).is_err(), "dated far ahead of this device's clock");
        }
        // the longest name a code carries, and one that isn't only letters
        let long = write_code(Kind::Wants, &key, 8770, 100, &"é".repeat(90), &addrs);
        assert!(long.len() < 260, "{} characters", long.len());
        assert_eq!(read_code(&long, 50).unwrap().name().chars().count(), NAME_MAX);
        let odd = write_code(Kind::Gives, &key, 8770, 100, " a\u{7}b\nc<script> ", &addrs);
        assert_eq!(read_code(&odd, 50).unwrap().name(), "abc<script>");
        assert_eq!(read_code(&write_code(Kind::Gives, &key, 1, 100, "", &addrs), 50).unwrap().name(), "");
    }

    #[test]
    fn a_code_of_the_first_version_is_still_read() {
        let key = [9u8; KEY];
        let code = read_code(&first_code(&key, 8770, 100, "192.168.1.20,fd00::20"), 50).unwrap();
        assert_eq!((code.key, code.port, code.kind(), code.first, code.name()), (key, 8770, Kind::Gives, true, ""));
        assert_eq!(code.addrs.len(), 2);
        let newer = read_code(&format!("{PREFIX}3:x:y"), 50).err().unwrap();
        assert!(newer.contains("newer version"), "{newer}");
    }

    #[test]
    fn what_is_not_a_code_is_refused() {
        let key = B64URL.encode([7u8; KEY]);
        let p1 = format!("{PREFIX}1:");
        let p2 = format!("{PREFIX}2:g:");
        for bad in [
            String::new(),
            "https://example.com/".to_string(),
            PREFIX.to_string(),
            format!("{PREFIX}{key}:8770:100:192.168.1.20"),
            format!("{PREFIX}x:{key}:8770:100:192.168.1.20"),
            format!("{PREFIX}1234:{key}:8770:100:192.168.1.20"),
            format!("{p1}{key}"),
            format!("{p1}{key}:8770:100"),
            format!("{p1}short:8770:100:192.168.1.20"),
            format!("{p1}{key}:0:100:192.168.1.20"),
            format!("{p1}{key}:99999:100:192.168.1.20"),
            format!("{p1}{key}:8770:soon:192.168.1.20"),
            format!("{p1}{key}:8770:100:"),
            format!("{p1}{key}:8770:100:a.name.example"),
            // an address out on the internet: a code never sends a device there
            format!("{p1}{key}:8770:100:203.0.113.9"),
            format!("{p1}{key}:8770:100:192.168.1.20,2001:db8::1"),
            format!("{p1}{key}:8770:100:172.16.0.1,172.16.0.2,172.16.0.3,172.16.0.4,172.16.0.5"),
            // the second version: a kind it doesn't have, no name part, a name that isn't text
            format!("{PREFIX}2:x:{key}:8770:100::192.168.1.20"),
            format!("{PREFIX}2:{key}:8770:100::192.168.1.20"),
            format!("{p2}{key}:8770:100:192.168.1.20"),
            format!("{p2}{key}:8770:100:!!:192.168.1.20"),
            format!("{p2}{key}:8770:100:{}:192.168.1.20", B64URL.encode([0xff, 0xfe])),
            format!("{p2}{key}:8770:100:{}:192.168.1.20", "A".repeat(400)),
            format!("{p2}{key}:8770:100::203.0.113.9"),
        ] {
            assert!(read_code(&bad, 50).is_err(), "taken: {bad}");
        }
        assert!(read_code(&format!("{p1}{key}:8770:100:192.168.1.20"), 50).is_ok());
        assert!(read_code(&format!("{p1}{key}:8770:100:100.64.3.1,169.254.7.7"), 50).is_ok());
        assert!(read_code(&format!("{p2}{key}:8770:100::192.168.1.20"), 50).is_ok());
        assert!(read_code(&format!("{PREFIX}2:w:{key}:8770:100::fd00::20"), 50).is_ok());
    }

    #[test]
    fn the_device_that_read_the_code_gets_the_login_and_the_passphrase_once() {
        let offer = Offer(showing_here("Desk computer"));
        let text = offer.code();
        assert!(offer.left() > VALID - 5);
        let stop = Arc::new(AtomicBool::new(false));
        let serving = std::thread::spawn({
            let stop = stop.clone();
            move || offer.serve(&a_handover(), &stop)
        });
        let code = read_code(&text, unix_now()).unwrap();
        assert_eq!((code.kind(), code.name()), (Kind::Gives, "Desk computer"));
        let got = ask(&code).unwrap_or_else(|e| panic!("{e}"));
        let login = &got.login;
        assert_eq!(
            (login.host.as_str(), login.port, login.dbname.as_str(), login.user.as_str()),
            ("db.example", 5432, "people", "app")
        );
        assert_eq!((login.password.as_str(), login.schema.as_str()), ("a pass word / with + signs", "facerec"));
        assert_eq!(
            (got.passphrase.as_deref(), got.from.as_str()),
            (Some("the owner's own passphrase"), "Desk computer")
        );
        assert_eq!(serving.join().unwrap(), Outcome::Given("127.0.0.1".parse().unwrap()));
        // the same code a second time: nobody listens any more
        assert!(ask(&code).is_err());
        // a device that wants to join can't be asked, and nothing is handed to one that gives
        let wants = Want(showing_here("New"));
        let e = ask(&read_code(&wants.code(), unix_now()).unwrap()).err().unwrap();
        assert!(e.contains("wants to join"), "{e}");
        let e = give(&code, &a_handover(), ("127.0.0.1", 5432)).err().unwrap();
        assert!(e.contains("already"), "{e}");
    }

    #[test]
    fn a_device_without_the_passphrase_hands_over_the_login_alone() {
        let offer = Offer(showing_here(""));
        let code = read_code(&offer.code(), unix_now()).unwrap();
        let serving = std::thread::spawn(move || {
            offer.serve(&Handover::new(a_login(), Some(String::new()), ""), &AtomicBool::new(false))
        });
        let got = ask(&code).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((got.passphrase, got.from.as_str(), got.login.user.as_str()), (None, "", "app"));
        assert!(matches!(serving.join().unwrap(), Outcome::Given(_)));
    }

    #[test]
    fn a_wrong_key_gets_nothing_and_ends_the_code() {
        let offer = Offer(showing_here(""));
        let text = offer.code();
        let stop = Arc::new(AtomicBool::new(false));
        let serving = std::thread::spawn({
            let stop = stop.clone();
            move || offer.serve(&a_handover(), &stop)
        });
        let mut code = read_code(&text, unix_now()).unwrap();
        code.key[0] ^= 1; // someone who knows where the device listens and not what its screen showed
        for _ in 0..WRONG_ASKS {
            let e = ask(&code).err().expect("a wrong key was answered");
            assert!(e.contains("didn't hand over"), "{e}");
        }
        assert_eq!(serving.join().unwrap(), Outcome::Refused);
    }

    #[test]
    fn a_showing_ends_when_told_or_when_its_time_is_over() {
        let stop = Arc::new(AtomicBool::new(true));
        assert_eq!(Offer(showing_here("")).serve(&a_handover(), &stop), Outcome::Stopped);
        assert!(matches!(Want(showing_here("")).receive(&stop), Received::Stopped));
        let mut showing = showing_here("");
        showing.until = Instant::now();
        assert_eq!(Offer(showing).serve(&a_handover(), &AtomicBool::new(false)), Outcome::Expired);
        let mut showing = showing_here("");
        showing.until = Instant::now();
        assert!(matches!(Want(showing).receive(&AtomicBool::new(false)), Received::Expired));
    }

    #[test]
    fn a_device_of_the_database_hands_over_to_the_new_device_that_shows_a_code() {
        let want = Want(showing_here("New laptop"));
        let text = want.code();
        let receiving = std::thread::spawn(move || want.receive(&AtomicBool::new(false)));
        let code = read_code(&text, unix_now()).unwrap();
        assert_eq!((code.kind(), code.name()), (Kind::Wants, "New laptop"));
        give(&code, &a_handover(), ("127.0.0.1", 5432)).unwrap_or_else(|e| panic!("{e}"));
        let Received::Taken(got, from) = receiving.join().unwrap() else { panic!("nothing was taken") };
        assert_eq!(from, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(
            (got.login.host.as_str(), got.login.password.as_str()),
            ("db.example", "a pass word / with + signs")
        );
        assert_eq!(
            (got.passphrase.as_deref(), got.from.as_str()),
            (Some("the owner's own passphrase"), "Desk computer")
        );
        // the same code a second time: nobody listens any more
        assert!(give(&code, &a_handover(), ("127.0.0.1", 5432)).is_err());
    }

    #[test]
    fn nothing_is_handed_to_what_only_sits_at_the_codes_address() {
        // something that listens where the code says, and never saw the code
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let got = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut hello = [0u8; 4 + 1 + PUBLIC + TAG];
            stream.read_exact(&mut hello).unwrap();
            let (_, public) = exchange().unwrap();
            stream.write_all(&[&public[..], &[3u8; TAG]].concat()).unwrap();
            let mut more = Vec::new();
            let _ = stream.read_to_end(&mut more);
            more
        });
        let text = write_code(Kind::Wants, &[5u8; KEY], port, unix_now() + VALID, "New", &here());
        let e = give(&read_code(&text, unix_now()).unwrap(), &a_handover(), ("127.0.0.1", 5432)).err().unwrap();
        assert!(e.contains("isn't the device that shows the code"), "{e}");
        assert!(got.join().unwrap().is_empty(), "something was sent to it");
    }

    #[test]
    fn a_wrong_key_brings_a_new_device_nothing_and_ends_its_code() {
        let want = Want(showing_here("New"));
        let text = want.code();
        let receiving = std::thread::spawn(move || want.receive(&AtomicBool::new(false)));
        let mut code = read_code(&text, unix_now()).unwrap();
        code.key[0] ^= 1;
        for _ in 0..WRONG_ASKS {
            assert!(give(&code, &a_handover(), ("127.0.0.1", 5432)).is_err(), "a wrong key was taken");
        }
        assert!(matches!(receiving.join().unwrap(), Received::Refused));
    }

    /// A device that was not updated shows a code: the first kind's answer, as such a device writes it.
    fn first_holder(listener: TcpListener, key: [u8; KEY], login: Vec<u8>) {
        let (mut stream, _) = listener.accept().unwrap();
        let mut ask = [0u8; 4 + PUBLIC + TAG];
        stream.read_exact(&mut ask).unwrap();
        assert_eq!(&ask[..4], MAGIC1);
        let (theirs, tag) = ask[4..].split_at(PUBLIC);
        hmac::verify(&tag_key1(&key), &[&b"ask"[..], theirs].concat(), tag).unwrap();
        let (private, public) = exchange().unwrap();
        let asker = agreement::UnparsedPublicKey::new(&agreement::X25519, theirs);
        let k = agreement::agree_ephemeral(private, &asker, |shared| seal_key(SEAL1, &key, shared, theirs, &public))
            .unwrap()
            .unwrap();
        let mut out = login;
        k.seal_in_place_append_tag(aead::Nonce::assume_unique_for_key([0; 12]), aead::Aad::from(MAGIC1), &mut out)
            .unwrap();
        stream.write_all(&[&public[..], &(out.len() as u32).to_be_bytes(), &out].concat()).unwrap();
    }

    #[test]
    fn a_new_device_joins_from_a_device_that_was_not_updated() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let (port, key) = (listener.local_addr().unwrap().port(), [3u8; KEY]);
        let holding = std::thread::spawn(move || first_holder(listener, key, a_login().to_bytes()));
        let code = read_code(&first_code(&key, port, unix_now() + VALID, "127.0.0.1"), unix_now()).unwrap();
        let got = ask(&code).unwrap_or_else(|e| panic!("{e}"));
        holding.join().unwrap();
        assert_eq!((got.login.user.as_str(), got.login.password.as_str()), ("app", "a pass word / with + signs"));
        assert_eq!((got.passphrase, got.from.as_str()), (None, ""));
    }

    #[test]
    fn what_crosses_the_network_does_not_show_what_is_handed_over() {
        let (_, asker) = exchange().unwrap();
        let plain = a_handover().to_bytes(&here());
        let (holder, sealed) = sealed(&[7u8; KEY], &asker, &plain).unwrap();
        assert_eq!((holder.len(), sealed.len()), (PUBLIC, plain.len() + 16));
        let seen = String::from_utf8_lossy(&sealed).into_owned();
        assert!(!seen.contains("pass word") && !seen.contains("db.example") && !seen.contains("passphrase"));
        // two answers to the same asker are encrypted under different keys
        assert_ne!(sealed, super::sealed(&[7u8; KEY], &asker, &plain).unwrap().1);
        // the two ways and the two versions never share a key
        let k = |label| seal(&seal_key(label, &[7u8; KEY], b"shared", b"a", b"b").unwrap(), 0, b"x").unwrap();
        assert!(k(SEAL1) != k(SEAL_ASKED) && k(SEAL_ASKED) != k(SEAL_GIVEN) && k(SEAL1) != k(SEAL_GIVEN));
        // and the two messages of one exchange never share a number
        let key = seal_key(SEAL_GIVEN, &[7u8; KEY], b"shared", b"a", b"b").unwrap();
        assert_ne!(seal(&key, 0, TAKEN).unwrap(), seal(&key, 1, TAKEN).unwrap());
    }

    #[test]
    fn what_another_device_hands_over_is_checked() {
        let good = a_handover().to_bytes(&here());
        assert!(Handover::from_bytes(&good).is_some());
        let changed = |at: &str, key: &str, value: serde_json::Value| {
            let mut v: serde_json::Value = serde_json::from_slice(&good).unwrap();
            if at.is_empty() {
                v[key] = value
            } else {
                v[at][key] = value
            }
            Handover::from_bytes(v.to_string().as_bytes())
        };
        for (key, value) in [
            ("schema", serde_json::json!("a;drop")),
            ("port", serde_json::json!(0)),
            ("port", serde_json::json!(70000)),
            ("host", serde_json::json!("")),
            ("password", serde_json::json!(7)),
            ("user", serde_json::Value::Null),
        ] {
            assert!(changed("login", key, value).is_none(), "taken with the login's {key} changed");
            let mut v = a_login().to_value();
            v[key] = serde_json::json!(null);
            assert!(Login::from_bytes(v.to_string().as_bytes()).is_none());
        }
        assert!(changed("", "login", serde_json::json!("none")).is_none());
        assert!(changed("", "passphrase", serde_json::json!(7)).is_none());
        assert!(changed("", "passphrase", serde_json::json!("p".repeat(PASSPHRASE_MAX + 1))).is_none());
        assert!(changed("", "passphrase", serde_json::Value::Null).unwrap().passphrase.is_none());
        assert_eq!(changed("", "from", serde_json::json!(7)).unwrap().from, "");
        assert_eq!(changed("", "from", serde_json::json!("a\nb".repeat(40))).unwrap().from.chars().count(), NAME_MAX);
        assert!(Handover::from_bytes(b"not json").is_none());
        assert!(Login::from_bytes(b"not json").is_none());
    }

    #[test]
    fn a_server_on_the_showing_device_is_at_that_devices_address_for_the_others() {
        let here: Vec<IpAddr> = vec!["192.168.1.20".parse().unwrap()];
        for host in ["127.0.0.1", "localhost", "::1"] {
            let login = Login { host: host.into(), ..a_login() };
            assert_eq!(login.for_others(&here).host, "192.168.1.20", "{host}");
            let handed = Handover::from_bytes(&Handover::new(login, None, "").to_bytes(&here)).unwrap();
            assert_eq!(handed.login.host, "192.168.1.20", "{host}");
        }
        assert_eq!(a_login().for_others(&here).host, "db.example");
    }
}
