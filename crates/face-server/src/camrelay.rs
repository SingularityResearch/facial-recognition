//! Other devices' cameras, kept flowing for this device's Cameras page (the asking side of the peer
//! service).
//!
//! One relay per watched device holds one connection to that device's peer service for as long as anyone
//! watches: TLS with the device's certificate checked against the SHA-256 the shared database has for it, the
//! request signed with the cluster's key. It reads the frames and the live readout (a JSON part about once a
//! second between the frames) as they come and keeps the newest of each; a connection that drops is made
//! again at once. The page asks for every camera's next frames in one short request and for every readout
//! in another, so its requests don't grow with the number of cameras. A relay nobody asked for in `IDLE`
//! seconds closes its connection. Frames are kept in memory only.

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU16, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use face_db::{
    Device,
    kept::{Kept, Peer},
    peer::{CLOCK_SKEW, sign},
};
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde_json::{Map, Value, json};

use crate::camstream::{Any, Stream};

/// bytes in one part of a stream: more is a stream out of step, not a frame
const MAX_PART: usize = 8_000_000;
/// a relay nobody asked for this long closes its connection
const IDLE: Duration = Duration::from_secs(60);
/// relays at a time: as many cameras as one request of the Cameras page may ask about. A device id is
/// whatever a request says, and each relay is a thread that tries to connect.
pub const MAX_RELAYS: usize = 64;
/// a readout older than this isn't shown: the device stopped sending it
const READOUT_STALE: Duration = Duration::from_secs(5);
/// to wait for one of a device's addresses to take the connection: it may list some this one can't reach
const CONNECT: Duration = Duration::from_secs(3);
/// to wait for another device's answer
const TIMEOUT: Duration = Duration::from_secs(8);
/// to wait for a stream's next part before it counts as gone: the sender writes a readout every second
const STREAM_READ: Duration = Duration::from_secs(15);
const TOO_MANY: &str = "too many cameras are being watched from this device";
/// a relay's trouble is said in the log again only after this long, however often it meets it
const SAID_AGAIN: Duration = Duration::from_secs(600);

struct Seen {
    seq: u64,
    jpeg: Option<Arc<Vec<u8>>>,
    at: Option<Instant>,
    readout: Option<Value>,
    readout_at: Option<Instant>,
    used: Instant,
    error: String,
    stopped: bool,
}

/// One watched device's newest frame and readout.
pub struct Relay {
    device: String,
    seen: Mutex<Seen>,
}

impl Relay {
    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// (number, frame) when there is a frame newer than `after`, else (after, None); never waits.
    fn peek(&self, after: u64) -> (u64, Option<Arc<Vec<u8>>>) {
        let mut s = self.seen();
        s.used = Instant::now();
        // a number from an earlier relay (this one was made after IDLE s unwatched): any frame is new
        let after = if after > s.seq { 0 } else { after };
        if s.seq > after && s.jpeg.is_some() { (s.seq, s.jpeg.clone()) } else { (after, None) }
    }

    /// The device's newest readout, or {faces: [], error} when there is none lately.
    fn live_readout(&self) -> Value {
        let mut s = self.seen();
        s.used = Instant::now();
        match (&s.readout, s.readout_at) {
            (Some(r), Some(at)) if at.elapsed() < READOUT_STALE => r.clone(),
            _ => {
                json!({"faces": [], "error": if s.error.is_empty() { "no readout from that device yet" } else { s.error.as_str() }})
            }
        }
    }
}

/// The device's certificate is the one whose SHA-256 the shared database has for it, and nothing else is
/// asked of it: it is the device's own, signed by nobody.
#[derive(Debug)]
struct Pinned {
    sha256: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if ring::digest::digest(&ring::digest::SHA256, end_entity.as_ref()).as_ref() == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("that device's certificate isn't the one in the database".to_string()))
        }
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

pub(crate) type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// A connection to a device's peer service at one of its addresses: TLS, the device's certificate checked
/// inside the handshake against the SHA-256 the shared database has for it. Nothing is sent to a device
/// that shows another certificate.
pub(crate) fn pinned(addr: &str, peer: &Peer) -> Result<Tls, String> {
    pinned_as(addr, peer, None)
}

/// The same, and with `own` this device shows its own certificate in the handshake (and proves it holds its
/// key): what a node asks of a device that wants its bus (`peer.rs`).
pub(crate) fn pinned_as(
    addr: &str,
    peer: &Peer,
    own: Option<(CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>)>,
) -> Result<Tls, String> {
    let e = |e: std::io::Error| e.to_string();
    let ip: IpAddr = addr.parse().map_err(|_| "not an address".to_string())?;
    let sha256: [u8; 32] = (0..32)
        .map(|i| peer.cert_sha256.get(2 * i..2 * i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
        .collect::<Option<Vec<u8>>>()
        .and_then(|v| v.try_into().ok())
        .ok_or("the database has no certificate for that device")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { sha256, provider }));
    let config = match own {
        Some((cert, key)) => config
            .with_client_auth_cert(vec![cert], key)
            .map_err(|e| format!("this device's own certificate and key don't fit: {e}"))?,
        None => config.with_no_client_auth(),
    };
    let tcp = TcpStream::connect_timeout(&SocketAddr::new(ip, peer.port), CONNECT).map_err(e)?;
    tcp.set_read_timeout(Some(TIMEOUT)).map_err(e)?;
    tcp.set_write_timeout(Some(TIMEOUT)).map_err(e)?;
    let _ = tcp.set_nodelay(true);
    let conn =
        rustls::ClientConnection::new(Arc::new(config), ServerName::IpAddress(ip.into())).map_err(|e| e.to_string())?;
    Ok(rustls::StreamOwned::new(conn, tcp))
}

/// (content type, body's length, where the body starts) of the first part in `buf`:
/// a `Content-Type` line, then a `Content-Length` line, then an empty one. None until all of
/// that has arrived.
fn find_part(buf: &[u8]) -> Option<(String, usize, usize)> {
    const TYPE: &[u8] = b"Content-Type:";
    const LENGTH: &[u8] = b"Content-Length:";
    let blank = |b: &[u8]| b.iter().take_while(|c| **c == b' ' || **c == b'\t').count();
    let mut from = 0;
    while let Some(at) = buf[from..].windows(TYPE.len()).position(|w| w == TYPE).map(|p| p + from) {
        from = at + TYPE.len();
        let rest = &buf[from..];
        let rest = &rest[blank(rest)..];
        let Some(end) = rest.windows(2).position(|w| w == b"\r\n") else { return None };
        let (ctype, rest) = (&rest[..end], &rest[end + 2..]);
        if ctype.is_empty() || !rest.starts_with(LENGTH) {
            if rest.len() < LENGTH.len() && LENGTH.starts_with(rest) {
                return None; // the next line is still arriving
            }
            continue;
        }
        let rest = &rest[LENGTH.len()..];
        let rest = &rest[blank(rest)..];
        let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
        let after = &rest[digits..];
        if after.len() < 4 && b"\r\n\r\n".starts_with(after) {
            return None;
        }
        if digits == 0 || digits > 12 || !after.starts_with(b"\r\n\r\n") {
            continue;
        }
        let n: usize = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
        let start = buf.len() - after.len() + 4;
        return Some((String::from_utf8_lossy(ctype).trim().to_ascii_lowercase(), n, start));
    }
    None
}

/// The parts of a multipart stream, one after another.
pub(crate) struct Parts<R> {
    from: R,
    buf: Vec<u8>,
}

impl<R: Read> Parts<R> {
    /// The parts of a stream, of which `first` came with the answer's head already.
    pub(crate) fn new(from: R, first: Vec<u8>) -> Parts<R> {
        Parts { from, buf: first }
    }

    /// The next part (content type, body); None when the stream ended.
    pub(crate) fn next(&mut self) -> std::io::Result<Option<(String, Vec<u8>)>> {
        let out_of_step = || std::io::Error::other("stream out of step");
        let mut chunk = vec![0u8; 65_536];
        let (ctype, n, start) = loop {
            if let Some(found) = find_part(&self.buf) {
                break found;
            }
            if self.buf.len() > MAX_PART {
                return Err(out_of_step());
            }
            match self.from.read(&mut chunk)? {
                0 => return Ok(None),
                got => self.buf.extend_from_slice(&chunk[..got]),
            }
        };
        if n > MAX_PART {
            return Err(out_of_step());
        }
        while self.buf.len() < start + n {
            match self.from.read(&mut chunk)? {
                0 => return Ok(None),
                got => self.buf.extend_from_slice(&chunk[..got]),
            }
        }
        let body = self.buf[start..start + n].to_vec();
        self.buf.drain(..start + n);
        Ok(Some((ctype, body)))
    }
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Seconds since 1970 of an answer's `Date` line ("Fri, 20 Nov 2026 08:05:00 GMT").
fn http_time(date: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut words = date.split_once(',')?.1.split_whitespace();
    let day: i64 = words.next()?.parse().ok()?;
    let month = MONTHS.iter().position(|m| Some(*m) == words.clone().next())? as i64 + 1;
    words.next();
    let year: i64 = words.next()?.parse().ok()?;
    let mut clock = words.next()?.split(':').map(|n| n.parse::<i64>().ok());
    let (h, m, s) = (clock.next()??, clock.next()??, clock.next()??);
    // a date of the calendar to days since 1970 (Howard Hinnant's days_from_civil)
    let y = if month <= 2 { year - 1 } else { year };
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(days * 86_400 + h * 3600 + m * 60 + s)
}

/// What a refusing device said; when it was the request's time, how far apart the two clocks are.
pub(crate) fn why(body: &[u8], now: u64) -> String {
    let text: String = String::from_utf8_lossy(body).chars().take(300).collect();
    match serde_json::from_str::<Value>(&text) {
        Ok(said) if said.is_object() => match said.get("now").and_then(Value::as_f64) {
            Some(theirs) => {
                let diff = theirs - now as f64;
                format!(
                    "its clock is {:.0} s {} this device's (more than {CLOCK_SKEW} s apart is refused): check that both set their clocks automatically",
                    diff.abs(),
                    if diff > 0.0 { "ahead of" } else { "behind" }
                )
            }
            None => said.get("error").and_then(Value::as_str).unwrap_or("None").chars().take(100).collect(),
        },
        _ => text.chars().take(100).collect(),
    }
}

/// Say in the log, at most once an hour for a device (`said`: address -> when), that its clock and this
/// one's are more than half of `CLOCK_SKEW` apart (from the `Date` of its answer): past `CLOCK_SKEW` the
/// devices refuse each other, and nothing said so beforehand.
pub(crate) fn clock_note(said: &Mutex<HashMap<String, Instant>>, addr: &str, head: &str) {
    let date = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("date")));
    let Some(theirs) = date.and_then(|(_, v)| http_time(v.trim())) else { return };
    let diff = theirs - unix_now() as i64;
    let mut said = said.lock().unwrap_or_else(|e| e.into_inner());
    if diff.unsigned_abs() > CLOCK_SKEW / 2 && said.get(addr).is_none_or(|t| t.elapsed() > Duration::from_secs(3600)) {
        said.insert(addr.to_string(), Instant::now());
        println!(
            "The clock of the device at {addr} is {} s {} this device's. At {CLOCK_SKEW} s apart the devices stop showing each other photos and cameras: check that both set their clocks automatically.",
            diff.abs(),
            if diff > 0 { "ahead of" } else { "behind" }
        );
    }
}

/// What this device needs to ask another: who it is itself, and what it may not mistake for the other.
pub struct Relays {
    home: PathBuf,
    kept: Arc<Kept>,
    any: Arc<Any>,
    /// this device's own peer service's port, 0 while it has none
    own_port: Arc<AtomicU16>,
    relays: Mutex<HashMap<String, Arc<Relay>>>,
    /// a device's certificate -> its address that answered last
    good: Mutex<HashMap<String, String>>,
    /// address -> when the log last said that device's clock is far off
    clock_said: Mutex<HashMap<String, Instant>>,
}

/// The addresses of a device that this one may ask: those of a local network (private, link-local, a shared
/// provider range). An address anywhere else is asked only where the owner switched on "Fully distributed
/// system": the member list is filled from rows anybody with the database's login can write, and a row must
/// not be able to send every device's requests to a machine out on the internet.
pub(crate) fn askable(addresses: &[String], home: &Path) -> Vec<String> {
    let anywhere = crate::hardware::distributed(home);
    addresses.iter().filter(|a| anywhere || a.parse::<IpAddr>().is_ok_and(|ip| local(&ip))).cloned().collect()
}

fn local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // (this computer itself too: a test's devices are all on it, and nothing is given away there)
            v4.is_private() || v4.is_link_local() || v4.is_loopback() || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 || v6.is_loopback()
        }
    }
}

/// A device's addresses in the order to try them: the one that answered last, then those on a network this
/// device is on too (`reach`: the addresses other devices reach this one at), then the rest, each in the
/// order the device listed them. Never one of this device's `own` addresses at its own port: a VPN client
/// can give every computer the same address, and a device asking there reached its own service.
pub(crate) fn order(
    addrs: &[String],
    port: u16,
    good: Option<&str>,
    reach: &[String],
    own: &[String],
    own_port: u16,
) -> Vec<String> {
    let net = |a: &str| a.rsplit_once('.').map_or("", |p| p.0).to_string();
    let nets: Vec<String> = reach.iter().map(|a| net(a)).collect();
    let mut out: Vec<String> = addrs.iter().filter(|a| !(port == own_port && own.contains(a))).cloned().collect();
    out.sort_by_key(|a| (Some(a.as_str()) != good, !nets.contains(&net(a))));
    out
}

impl Relays {
    pub fn new(home: &Path, kept: Arc<Kept>, any: Arc<Any>, own_port: Arc<AtomicU16>) -> Arc<Relays> {
        Arc::new(Relays {
            home: home.to_path_buf(),
            kept,
            any,
            own_port,
            relays: Mutex::new(HashMap::new()),
            good: Mutex::new(HashMap::new()),
            clock_said: Mutex::new(HashMap::new()),
        })
    }

    /// This device's own id (made the first time it is asked for, as everywhere).
    pub fn me(&self) -> Result<String, String> {
        Device::load(&self.home, &crate::device().0).map(|d| d.id)
    }

    /// The relay for a device, started when it is first asked for; None when `MAX_RELAYS` run already.
    fn get(self: &Arc<Self>, device: &str) -> Option<Arc<Relay>> {
        let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = relays.get(device).filter(|r| !r.seen().stopped) {
            r.seen().used = Instant::now();
            return Some(r.clone());
        }
        if relays.len() >= MAX_RELAYS {
            return None;
        }
        let relay = Arc::new(Relay {
            device: device.to_string(),
            seen: Mutex::new(Seen {
                seq: 0,
                jpeg: None,
                at: None,
                readout: None,
                readout_at: None,
                used: Instant::now(),
                error: String::new(),
                stopped: false,
            }),
        });
        relays.insert(device.to_string(), relay.clone());
        let (all, mine) = (self.clone(), relay.clone());
        let short: String = device.chars().take(8).collect();
        let started = std::thread::Builder::new().name(format!("camera relay {short}")).spawn(move || all.run(&mine));
        if started.is_err() {
            relays.remove(device);
            return None;
        }
        Some(relay)
    }

    /// A relay's life: connect, read frames and readouts, connect again when the stream drops, until
    /// nobody has asked for `IDLE`.
    fn run(self: &Arc<Self>, relay: &Arc<Relay>) {
        let short: String = relay.device.chars().take(8).collect();
        let over = |relay: &Relay| {
            let s = relay.seen();
            s.stopped || s.used.elapsed() > IDLE
        };
        let mut said: HashMap<String, Instant> = HashMap::new();
        while !over(relay) {
            let tried = self.open_stream(&relay.device).and_then(|(tls, first)| {
                relay.seen().error.clear();
                let mut parts = Parts { from: tls, buf: first };
                // after readouts too: a paused camera sends only those
                while !over(relay) {
                    // (a device whose app was closed drops the connection without a word: said plainly)
                    let dropped = |e: std::io::Error| match e.kind() {
                        std::io::ErrorKind::UnexpectedEof => "the device's connection ended".to_string(),
                        _ => e.to_string(),
                    };
                    let Some((ctype, body)) = parts.next().map_err(dropped)? else { break };
                    if ctype.starts_with("application/json") {
                        if let Ok(readout) = serde_json::from_slice::<Value>(&body) {
                            if readout.is_object() {
                                let mut s = relay.seen();
                                (s.readout, s.readout_at) = (Some(readout), Some(Instant::now()));
                            }
                        }
                        continue;
                    }
                    {
                        let mut s = relay.seen();
                        if let Some(gap) = s.at.map(|t| t.elapsed().as_secs_f64()).filter(|g| *g > 2.0 && *g < 600.0) {
                            println!("Camera relay {short}: no frame for {gap:.1} s");
                        }
                        s.seq += 1;
                        (s.jpeg, s.at) = (Some(Arc::new(body)), Some(Instant::now()));
                    }
                    self.any.news(); // the Cameras page's one request for every camera
                }
                Ok(())
            });
            if let Err(e) = tried {
                // unreachable, refused, dropped: try again shortly. Each trouble is said when it is new and
                // again once in a while: a device that sleeps is met with two troubles in turn, one for each
                // of its addresses, every few seconds.
                let e: String = e.chars().take(200).collect();
                if said.get(&e).is_none_or(|at| at.elapsed() > SAID_AGAIN) {
                    println!("Camera relay {short}: {e}");
                    said.retain(|_, at| at.elapsed() <= SAID_AGAIN);
                    said.insert(e.clone(), Instant::now());
                }
                relay.seen().error = e;
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        relay.seen().stopped = true;
        let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        if relays.get(&relay.device).is_some_and(|r| Arc::ptr_eq(r, relay)) {
            relays.remove(&relay.device);
        }
    }

    /// Where a device is, the certificate it must show and the cluster's key, from what this device keeps
    /// itself (`face_db::members`): no server is asked. None for a device the member list doesn't know, or
    /// before this device has the key; the device's part is None when it doesn't share.
    fn from_members(&self, device: &str) -> Option<(Option<Peer>, [u8; 32], Vec<String>)> {
        let known = face_db::members::member(&self.home, device)?;
        let key = face_db::members::kept_key(&self.home)?;
        // where this device is itself, as its own line of the list says: never asked at its own port
        let mine = self.me().ok().and_then(|me| face_db::members::member(&self.home, &me)).map(|m| m.addresses);
        let shares = known.share && known.port != 0 && !known.cert_sha256.is_empty();
        let peer = shares.then(|| Peer {
            name: known.name,
            addresses: known.addresses,
            port: known.port,
            cert_sha256: known.cert_sha256,
        });
        Some((peer, key, mine.unwrap_or_default()))
    }

    /// Start watching another device's camera: the connection, and what of the stream came with its head.
    /// Where the device is comes from this device's own member list; the shared database is asked only for
    /// a device the list doesn't know yet (one that joined a moment ago).
    fn open_stream(&self, device: &str) -> Result<(Tls, Vec<u8>), String> {
        let me = self.me()?;
        // where the member list says the device is
        let listed = self.from_members(device);
        let first = match &listed {
            Some((Some(peer), key, mine)) => match self.reach(peer, key, mine, &me) {
                Ok(open) => return Ok(open),
                Err(e) => Some(e),
            },
            _ => None,
        };
        // Not known to the list, or not reached where the list says it is: the database is asked, which says
        // at once when it is away. (A list that isn't kept fresh, a tracker run by itself, would otherwise
        // never learn that a device moved.)
        let asked = self.kept.ask(async |client, settings| {
            let peer = face_db::kept::device_peer(client, device).await?;
            let key = face_db::peer::cluster_key(client, settings).await?;
            let mine: Vec<String> =
                face_db::join::own_addresses((&settings.host, settings.port)).iter().map(IpAddr::to_string).collect();
            Ok((peer, key, mine))
        });
        let same =
            |a: &Peer, b: &Peer| (&a.addresses, a.port, &a.cert_sha256) == (&b.addresses, b.port, &b.cert_sha256);
        match (asked, listed, first) {
            // (the database says the same as the list: tried already)
            (Ok((Some(peer), ..)), Some((Some(known), ..)), Some(e)) if same(&peer, &known) => Err(e),
            (Ok((Some(peer), key, mine)), ..) => self.reach(&peer, &key, &mine, &me),
            (Ok((None, ..)), ..) | (Err(_), Some((None, ..)), _) => {
                Err("that device doesn't share its camera".to_string())
            }
            (Err(_), _, Some(e)) => Err(e),
            (Err(e), ..) => Err(e),
        }
    }

    /// A stream from a device at one of its addresses, the one that answered last first. Only addresses this
    /// device may ask (`askable`).
    fn reach(&self, peer: &Peer, key: &[u8; 32], mine: &[String], me: &str) -> Result<(Tls, Vec<u8>), String> {
        let good = self.good.lock().unwrap_or_else(|e| e.into_inner()).get(&peer.cert_sha256).cloned();
        let mut last = "no address".to_string();
        let port = self.own_port.load(Ordering::Relaxed);
        for addr in order(&askable(&peer.addresses, &self.home), peer.port, good.as_deref(), mine, mine, port) {
            match self.connect(&addr, peer, key, me) {
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

    fn connect(&self, addr: &str, peer: &Peer, key: &[u8; 32], me: &str) -> Result<(Tls, Vec<u8>), String> {
        let e = |e: std::io::Error| e.to_string();
        let mut tls = pinned(addr, peer)?;
        let when = unix_now().to_string();
        let ask = format!(
            "GET /stream HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nX-Peer-Device: {me}\r\nX-Peer-Time: {when}\r\nX-Peer-Sig: {}\r\n\r\n",
            sign(key, "GET", "/stream", &when, me)
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
                other => format!("{} answered {other}", peer.name),
            });
        }
        clock_note(&self.clock_said, addr, &text);
        tls.sock.set_read_timeout(Some(STREAM_READ)).map_err(e)?;
        Ok((tls, rest))
    }

    /// {device id: readout}: this device's from memory, the others' as their relays last got them. Never
    /// waits on the network.
    pub fn readouts(self: &Arc<Self>, own: &Stream, ids: &[(String, u64)]) -> Value {
        let me = self.me().unwrap_or_default();
        let mut out = Map::new();
        for (id, _) in ids {
            let readout = if *id == me {
                own.readout()
            } else {
                self.get(id).map_or_else(|| json!({"faces": [], "error": TOO_MANY}), |r| r.live_readout())
            };
            out.insert(id.clone(), readout);
        }
        Value::Object(out)
    }

    /// Each asked-for camera's newest frame after its number: (device id, number, frame) for those that
    /// have one, and why not for those whose relay has trouble. Waits up to `wait` for the first, then
    /// `gather` more so that frames of several cameras go in one answer.
    #[allow(clippy::type_complexity)]
    pub fn frames(
        self: &Arc<Self>,
        own: &Stream,
        after: &[(String, u64)],
        watcher: &str,
        wait: Duration,
        gather: Duration,
    ) -> (Vec<(String, u64, Arc<Vec<u8>>)>, Map<String, Value>) {
        let me = self.me().unwrap_or_default();
        let relays: Vec<(String, u64, Option<Arc<Relay>>)> =
            after.iter().map(|(id, n)| (id.clone(), *n, if *id == me { None } else { self.get(id) })).collect();
        let collect = || {
            let mut got = Vec::new();
            for (id, n, relay) in &relays {
                let (seq, jpeg) = match relay {
                    _ if *id == me => own.peek(*n, watcher),
                    Some(r) => r.peek(*n),
                    None => (*n, None),
                };
                if let Some(jpeg) = jpeg {
                    got.push((id.clone(), seq, jpeg));
                }
            }
            got
        };
        let end = Instant::now() + wait;
        // held while looking, so a frame that arrives meanwhile still wakes the wait
        let mut held = self.any.hold();
        let mut got = collect();
        while got.is_empty() && Instant::now() < end {
            held = self.any.wait(held, end.saturating_duration_since(Instant::now()).min(Duration::from_millis(500)));
            got = collect();
        }
        if !got.is_empty() && !gather.is_zero() && got.len() < after.len() {
            held = self.any.wait(held, gather);
            got = collect();
        }
        drop(held);
        let mut errors = Map::new();
        for (id, _, relay) in &relays {
            let error = match relay {
                _ if *id == me => continue,
                Some(r) => r.seen().error.clone(),
                None => TOO_MANY.to_string(),
            };
            if !error.is_empty() && !got.iter().any(|g| g.0 == *id) {
                errors.insert(id.clone(), json!(error));
            }
        }
        (got, errors)
    }
}

/// {device id: number} from "id:n,id:n" (or bare ids), checked, at most `MAX_RELAYS`, in the order asked.
pub fn camera_ids(text: &str) -> Vec<(String, u64)> {
    let is_id = |d: &str| {
        d.len() == 36
            && d.bytes().enumerate().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => c == b'-',
                _ => c.is_ascii_digit() || (b'a'..=b'f').contains(&c),
            })
    };
    let mut out: Vec<(String, u64)> = Vec::new();
    for part in text.split(',').take(MAX_RELAYS) {
        let (d, n) = part.split_once(':').unwrap_or((part, ""));
        if !is_id(d) {
            continue;
        }
        let n = if !n.is_empty() && n.len() <= 15 && n.bytes().all(|c| c.is_ascii_digit()) {
            n.parse().unwrap_or(0)
        } else {
            0
        };
        match out.iter_mut().find(|(id, _)| id == d) {
            Some(same) => same.1 = n,
            None => out.push((d.to_string(), n)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "11111111-2222-4333-8444-555555555555";
    const B: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

    #[test]
    fn where_a_device_is_comes_from_the_member_list_this_device_keeps() {
        use face_db::members::{self, Member};
        let home = std::env::temp_dir().join(format!("face-camrelay-members-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let (kept, any) = (Arc::new(Kept::new(&home)), crate::camstream::Stream::new().any.clone());
        let relays = Relays::new(&home, kept, any, Arc::new(AtomicU16::new(0)));
        let me = relays.me().unwrap();
        let one = |id: &str, port: u16, share: bool, cert: &str| Member {
            id: id.to_string(),
            name: "a device".to_string(),
            addresses: vec!["192.168.1.20".to_string()],
            port,
            cert_sha256: cert.to_string(),
            share,
        };
        let sha = "ab".repeat(32);
        let list = [one(A, 8767, true, &sha), one(B, 8767, false, &sha), one(&me, 8767, true, &sha)];
        // without the database's settings there is no key: nothing is known, and the caller asks elsewhere
        members::keep(&home, Some("0b0e7a52-6f0c-4b6e-9d53-1c2f3a4b5c6d"), &list).unwrap();
        assert!(relays.from_members(A).is_none());
        std::fs::write(
            home.join("database.json"),
            r#"{"host": "192.0.2.9", "user": "u", "password": "plain:YSBwYXNzd29yZA=="}"#,
        )
        .unwrap();
        // a member that shares: where it is, its certificate, the key, and this device's own addresses
        let (peer, key, mine) = relays.from_members(A).expect("a member of the list");
        let peer = peer.expect("it shares");
        assert_eq!(
            (peer.addresses, peer.port, peer.cert_sha256),
            (vec!["192.168.1.20".to_string()], 8767, sha.clone())
        );
        assert_eq!(Some(key), face_db::peer::derive_key("a password", "0b0e7a52-6f0c-4b6e-9d53-1c2f3a4b5c6d"));
        assert_eq!(mine, ["192.168.1.20"]);
        // a member that doesn't share is known and has no camera to ask for; a device the list doesn't know is
        // not known at all (the caller then asks the database, for a device that joined a moment ago)
        assert!(relays.from_members(B).is_some_and(|(peer, ..)| peer.is_none()));
        assert!(relays.from_members("77777777-2222-4333-8444-555555555555").is_none());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_ids_a_page_asks_about_are_checked() {
        assert_eq!(camera_ids(&format!("{A}:7,{B}")), vec![(A.to_string(), 7), (B.to_string(), 0)]);
        assert_eq!(camera_ids(&format!("{A}:x,{A}:12")), vec![(A.to_string(), 12)], "asked twice, the last counts");
        assert_eq!(camera_ids(&format!("{A}:9999999999999999")), vec![(A.to_string(), 0)], "too long to be a number");
        for bad in ["", "abc", "../etc", &B.to_uppercase(), &format!("{A}x"), &A.replace('-', "_")] {
            assert!(camera_ids(bad).is_empty(), "{bad}");
        }
        let many: Vec<String> = (0..100).map(|i| format!("{i:08x}-2222-4333-8444-555555555555")).collect();
        assert_eq!(camera_ids(&many.join(",")).len(), MAX_RELAYS);
    }

    #[test]
    fn a_streams_parts_are_read_as_they_arrive() {
        // the bytes a peer service writes, handed over in pieces of 7 bytes: no part depends on how it arrives
        let mut sent = Vec::new();
        for (ctype, body) in [
            ("image/jpeg", &b"\xff\xd8 one \r\n--facerecframe\r\n inside"[..]),
            ("application/json", b"{\"faces\":[]}"),
            ("image/jpeg", b""),
        ] {
            sent.extend_from_slice(
                format!("--facerecframe\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
            );
            sent.extend_from_slice(body);
            sent.extend_from_slice(b"\r\n");
        }
        struct Slow(Vec<u8>, usize);
        impl Read for Slow {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = 7.min(self.0.len() - self.1).min(buf.len());
                buf[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
                self.1 += n;
                Ok(n)
            }
        }
        let mut parts = Parts { from: Slow(sent, 0), buf: Vec::new() };
        let first = parts.next().unwrap().unwrap();
        assert_eq!((first.0.as_str(), first.1.len()), ("image/jpeg", 32), "a boundary inside a frame is the frame's");
        assert_eq!(parts.next().unwrap().unwrap(), ("application/json".to_string(), b"{\"faces\":[]}".to_vec()));
        assert_eq!(parts.next().unwrap().unwrap(), ("image/jpeg".to_string(), Vec::new()));
        assert!(parts.next().unwrap().is_none(), "the stream ended");
        // a part that says it is larger than any frame, and bytes that are no stream at all
        let huge = format!("Content-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", MAX_PART + 1).into_bytes();
        assert!(Parts { from: &huge[..], buf: Vec::new() }.next().is_err());
        assert!(Parts { from: &vec![b'x'; MAX_PART + 70_000][..], buf: Vec::new() }.next().is_err());
    }

    #[test]
    fn addresses_are_tried_in_their_fixed_order() {
        let addrs: Vec<String> =
            ["100.96.0.1", "192.168.5.9", "192.168.1.20", "192.168.1.21"].iter().map(|a| a.to_string()).collect();
        let mine = vec!["192.168.1.7".to_string(), "100.96.0.1".to_string()];
        // those on a network this device is on first, as listed; the one that answered last before all
        assert_eq!(
            order(&addrs, 8767, None, &mine, &mine, 0),
            ["100.96.0.1", "192.168.1.20", "192.168.1.21", "192.168.5.9"]
        );
        assert_eq!(order(&addrs, 8767, Some("192.168.5.9"), &mine, &mine, 0)[0], "192.168.5.9");
        // never this device's own address at its own port (a VPN can give two computers the same address)
        assert_eq!(order(&addrs, 8767, None, &mine, &mine, 8767), ["192.168.1.20", "192.168.1.21", "192.168.5.9"]);
        assert_eq!(
            order(&addrs, 8777, None, &mine, &mine, 8767).len(),
            4,
            "a second copy on this computer has another port"
        );
    }

    #[test]
    fn a_refusal_says_how_far_the_clocks_are_apart() {
        let now = 1_795_161_900;
        let said = why(format!("{{\"error\": \"stale or missing time\", \"now\": {}}}", now + 500).as_bytes(), now);
        assert!(
            said.starts_with("its clock is 500 s ahead of this device's (more than 120 s apart is refused)"),
            "{said}"
        );
        assert!(why(format!("{{\"now\": {}}}", now - 130).as_bytes(), now).starts_with("its clock is 130 s behind"));
        assert_eq!(
            why(b"{\"error\": \"not signed by a device of this cluster\"}", now),
            "not signed by a device of this cluster"
        );
        assert_eq!(why(b"<html>no</html>", now), "<html>no</html>");
        assert_eq!(http_time("Fri, 20 Nov 2026 08:05:00 GMT"), Some(1_795_161_900));
        assert_eq!(http_time("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(http_time("Tue, 29 Feb 2000 00:00:00 GMT"), Some(951_782_400));
        assert_eq!(http_time("yesterday"), None);
    }
}
