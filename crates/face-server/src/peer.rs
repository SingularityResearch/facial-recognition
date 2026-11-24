//! The peer service, its serving side: this device's live camera for the
//! owner's other devices, the one part of the tracker that listens beyond this device. It answers only
//!
//! - over TLS with this device's own certificate, whose SHA-256 is in the shared database: the asking device
//!   checks it, so no other machine can pose as this one;
//! - requests signed with the cluster key (`face_db::peer`), a key every device derives from the shared
//!   database's password and the database's own id, and no older than `CLOCK_SKEW` seconds;
//! - `/stream` (the camera as the tracker page shows it, a multipart JPEG stream with the readout between
//!   the frames) and `/readout`. A device that runs only this program has no photos to show: `/photo/...`
//!   is answered "this device doesn't have that photo";
//! - `/events` (this node's bus, `bus.rs`: its whole tree, then every change as it happens), and that only to
//!   a device that showed its own certificate in the handshake, the one this device's member list has for
//!   it (`face_db::members`). The cluster's key says "a device of this cluster"; the certificate says which
//!   one, so a subscriber is a known node, and a device its owner removed here stays removed under any id
//!   it shows that certificate with (`face_home::blocked` keeps the certificate with the id). What this
//!   does not do: the member list is filled from the shared database, and whoever has its login can write a
//!   row there, so a device with the login can still make itself a member under a new certificate. What the
//!   owner gets for that is the alert every device raises for a member it has not seen before. The member
//!   list is this device's own file: no server is asked when a node subscribes.
//!
//! What goes over the wire is fixed, byte for byte in its framing: devices of every version of the app ask
//! this service, and are asked by it.

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use face_db::peer::{CLOCK_SKEW, signed};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use serde_json::json;

use crate::{
    bus::{Bus, Cursor},
    camstream::Stream,
};

/// the port when it is free: the programs' tab page port + 3
pub const PORT: u16 = 8767;
const BOUNDARY: &str = "facerecframe";
/// between the live readouts sent along a stream
const READOUT_EVERY: Duration = Duration::from_secs(1);
/// without a byte moving, the TLS handshake included: a silent connection ends
const TIMEOUT: Duration = Duration::from_secs(30);
/// a stream lasts until the watching device closes it
const STREAM_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_HEAD: usize = 16 * 1024;
/// connections at a time: more are closed at once (each watching device holds one for the camera and one
/// for the bus)
const MAX_OPEN: usize = 64;
/// a subscriber with nothing new to hear is sent an empty answer this often: the connection is known to live
const EVENTS_BEAT: Duration = Duration::from_secs(8);
/// the handshake and a request's head arrive within this, however slowly their bytes come: a connection that
/// takes longer is let go (a byte now and then kept one open for days)
const HEAD_WITHIN: Duration = Duration::from_secs(10);
/// connections at a time from one address: a device holds one for the camera and one for the bus, and asks
/// for a photo or the people now and then
const MAX_FROM_ONE: usize = 8;
/// the people file is given to one device at a time
static PEOPLE_BUSY: AtomicBool = AtomicBool::new(false);

/// The cluster key as the device last read it from the shared database; None until then.
pub type Key = Arc<RwLock<Option<[u8; 32]>>>;

/// What gives a photo of this device for another one: (its content hash, a video's frame) -> the photo
/// view's JPEG, or None when this device doesn't have it.
pub type Provider = Arc<dyn Fn(&str, Option<i64>) -> Option<Vec<u8>> + Send + Sync>;

/// What gives this device's people file (everyone named, with their prints) to a device of the cluster that
/// has no library of its own: the file's bytes as they are, or None when this device has no such file.
pub type People = Arc<dyn Fn() -> Option<Arc<PeopleFile>> + Send + Sync>;

/// A people file as it is given: its bytes and their SHA-256.
pub struct PeopleFile {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// What gives a people file from where it lies, reading it only when it changed (its length and when it was
/// written), and never one larger than `MAX_PEOPLE`: every asking device is answered from the one copy.
pub fn people_from(file: std::path::PathBuf) -> People {
    type Kept = Option<(u64, Option<SystemTime>, Arc<PeopleFile>)>;
    let kept: Mutex<Kept> = Mutex::new(None);
    Arc::new(move || {
        let about = std::fs::metadata(&file).ok()?;
        if about.len() == 0 || about.len() > MAX_PEOPLE as u64 {
            return None;
        }
        let stamp = (about.len(), about.modified().ok());
        let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((len, written, held)) = kept.as_ref() {
            if (*len, *written) == stamp {
                return Some(held.clone());
            }
        }
        let bytes = std::fs::read(&file).ok().filter(|b| !b.is_empty() && b.len() <= MAX_PEOPLE)?;
        let sha256 = hex(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref());
        let held = Arc::new(PeopleFile { bytes, sha256 });
        *kept = Some((stamp.0, stamp.1, held.clone()));
        Some(held)
    })
}

/// What this device has to give the others beside its camera and its bus: its photos, its people. A phone,
/// which keeps no library, has neither.
#[derive(Clone, Default)]
pub struct Gives {
    pub photos: Option<Provider>,
    pub people: Option<People>,
}

/// bytes of a people file that is given to another device: a larger one is not a people file
pub const MAX_PEOPLE: usize = 64_000_000;

/// A running service: where it listens and its certificate's SHA-256, for the shared database.
pub struct Service {
    pub port: u16,
    pub cert_sha256: String,
    stopped: Arc<std::sync::atomic::AtomicBool>,
}

impl Service {
    /// Stop listening (requests being answered end by themselves): the port is free a moment later.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        // (the thread that takes connections waits for one: this is it)
        let _ = TcpStream::connect_timeout(&([127, 0, 0, 1], self.port).into(), Duration::from_secs(1));
    }
}

/// The SHA-256 of this device's certificate for the service (made if there is none yet), as the other
/// devices check it.
pub fn cert_sha256(home: &Path, name: &str) -> Result<String, String> {
    certificate(home, name).map(|made| made.2)
}

/// This device's own certificate and key, as it shows them when it asks another device for its bus.
pub(crate) fn own_certificate(
    home: &Path,
    name: &str,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), String> {
    certificate(home, name).map(|made| (made.0, made.1))
}

/// Any device may show a certificate in the handshake, and none has to (the camera's and the photos' routes
/// never asked for one). The handshake proves that the device holds the certificate's key; WHICH device a
/// certificate makes it is decided afterwards, against this device's member list (`serve`).
#[derive(Debug)]
struct AnyDevice {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::server::danger::ClientCertVerifier for AnyDevice {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
    fn client_auth_mandatory(&self) -> bool {
        false
    }
}

/// A photo asked for: its name and, of a video, the frame. None for anything that isn't exactly that.
fn photo_of(path: &str) -> Option<(String, Option<i64>)> {
    let rest = path.strip_prefix("/photo/")?;
    let (id, frame) = match rest.split_once("?frame=") {
        Some((id, frame)) => (id, Some(frame)),
        None => (rest, None),
    };
    if !crate::trackerlink::photo_id(id) {
        return None;
    }
    let frame = match frame {
        Some(f) if (1..=7).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit()) => Some(f.parse().ok()?),
        Some(_) => return None,
        None => None,
    };
    Some((id.to_string(), frame))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// This device's certificate and key for the service, made once (`HOME/peer-cert.pem`, `peer-key.pem`): the
/// certificate's name is for people, devices go by its SHA-256.
///
/// A certificate or key this program can't read is replaced by a new one: a damaged one, or one kept in a
/// form this program doesn't have. Left alone, the service simply didn't
/// start, which left such a computer without its camera and photos for the
/// others (found on the owner's devices, 2026-11-21). The other devices learn
/// the new certificate's SHA-256 from the shared database at the sync's next step.
fn certificate(home: &Path, name: &str) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>, String), String> {
    // one caller at a time, of this process and of the app's other one: two that both found none each wrote a
    // key and a certificate, and what was left could be one's key with the other's certificate, for good
    let one = face_files::paths::SharedLock::new(home, "peer-cert");
    let _held = one.hold();
    let (cert_file, key_file) = (home.join("peer-cert.pem"), home.join("peer-key.pem"));
    let readable = CertificateDer::from_pem_file(&cert_file).is_ok() && PrivateKeyDer::from_pem_file(&key_file).is_ok();
    if !readable && (cert_file.exists() || key_file.exists()) {
        println!("Peer service: this device's certificate and key can't be read here: a new certificate is made.");
        // (peer-key.json: a file that lay beside a key kept in the other form)
        for old in [&cert_file, &key_file, &home.join("peer-key.json")] {
            let _ = std::fs::remove_file(old);
        }
    }
    if !readable {
        let key = rcgen::KeyPair::generate().map_err(|e| format!("no key for the peer service: {e}"))?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
        let common: String = format!("Face Recognizer {name}").chars().take(64).collect();
        params.distinguished_name = rcgen::DistinguishedName::new();
        params.distinguished_name.push(rcgen::DnType::CommonName, common);
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(3650);
        let cert = params.self_signed(&key).map_err(|e| format!("no certificate for the peer service: {e}"))?;
        std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
        write_private(&key_file, key.serialize_pem().as_bytes())?;
        write_private(&cert_file, cert.pem().as_bytes())?;
    }
    let cert =
        CertificateDer::from_pem_file(&cert_file).map_err(|_| "The peer service's certificate can't be read.")?;
    let key = PrivateKeyDer::from_pem_file(&key_file).map_err(|_| "The peer service's key can't be read.")?;
    let sha = hex(ring::digest::digest(&ring::digest::SHA256, cert.as_ref()).as_ref());
    Ok((cert, key, sha))
}

/// A file that is this user's alone from its first byte (mode 600 outside Windows).
fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    options.open(path).and_then(|mut f| f.write_all(data)).map_err(|e| format!("can't write {name}: {e}"))
}

/// Start the service on the local network. `at`: the port to listen on, and an error when it is taken;
/// None: `PORT`, or any free one when that is taken. `photos`: what gives this device's photos to the
/// others (None: it has none to give, a phone). It answers nothing until `key` holds the cluster key.
pub fn start(
    home: &Path,
    name: &str,
    stream: Arc<Stream>,
    key: Key,
    at: Option<u16>,
    gives: Gives,
    bus: Arc<Bus>,
) -> Result<Service, String> {
    let listener = match at {
        Some(port) => TcpListener::bind(("0.0.0.0", port)),
        None => TcpListener::bind(("0.0.0.0", PORT)).or_else(|_| TcpListener::bind(("0.0.0.0", 0))),
    }
    .map_err(|e| format!("The peer service can't listen: {e}"))?;
    start_on(home, name, stream, key, listener, gives, bus)
}

/// The service on a listener that is there already (`start` makes the local network's; a test listens on
/// this computer alone).
pub(crate) fn start_on(
    home: &Path,
    name: &str,
    stream: Arc<Stream>,
    key: Key,
    listener: TcpListener,
    gives: Gives,
    bus: Arc<Bus>,
) -> Result<Service, String> {
    let (cert, private, cert_sha256) = certificate(home, name)?;
    let home = home.to_path_buf();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS didn't start: {e}"))?
        .with_client_cert_verifier(Arc::new(AnyDevice { provider }))
        .with_single_cert(vec![cert], private)
        .map_err(|e| format!("The peer service's certificate and key don't fit: {e}"))?;
    let config = Arc::new(config);
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let open = Arc::new(AtomicUsize::new(0));
    let from: Arc<Mutex<HashMap<std::net::IpAddr, usize>>> = Arc::new(Mutex::new(HashMap::new()));
    let stopped = Arc::new(AtomicBool::new(false));
    let ended = stopped.clone();
    std::thread::Builder::new()
        .name("peer service".into())
        .spawn(move || {
            for (n, tcp) in listener.incoming().enumerate() {
                if ended.load(Ordering::SeqCst) {
                    break; // stopped: the listener goes with this thread
                }
                let Ok(tcp) = tcp else { continue };
                let Ok(asker) = tcp.peer_addr().map(|a| a.ip()) else { continue };
                if open.fetch_add(1, Ordering::SeqCst) >= MAX_OPEN {
                    open.fetch_sub(1, Ordering::SeqCst);
                    continue; // dropped: closed at once
                }
                {
                    // (one address may not take every place: the other devices' cameras and buses go on)
                    let mut from = from.lock().unwrap_or_else(|e| e.into_inner());
                    let held = from.entry(asker).or_insert(0);
                    if *held >= MAX_FROM_ONE {
                        open.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    *held += 1;
                }
                let let_go = {
                    let (open, from) = (open.clone(), from.clone());
                    move || {
                        open.fetch_sub(1, Ordering::SeqCst);
                        let mut from = from.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(held) = from.get_mut(&asker) {
                            *held = held.saturating_sub(1);
                            if *held == 0 {
                                from.remove(&asker);
                            }
                        }
                    }
                };
                let (config, stream, key, done) = (config.clone(), stream.clone(), key.clone(), let_go.clone());
                let (gives, home, bus, stopped) = (gives.clone(), home.clone(), bus.clone(), ended.clone());
                let spawned = std::thread::Builder::new().name("peer request".into()).spawn(move || {
                    // a failed or silent handshake, a dropped connection: no news
                    let _ = serve(tcp, config, &home, &stream, &key, &gives, &bus, &stopped, n);
                    done();
                });
                if spawned.is_err() {
                    let_go(); // counted for a thread that never ran
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Service { port, cert_sha256, stopped })
}

/// The time as an answer's `Date` says it ("Fri, 20 Nov 2026 08:05:00 GMT"): the asking device tells from
/// it how far apart the two clocks are.
fn http_date(unix: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (days, rest) = (unix / 86_400, unix % 86_400);
    // days since 1970 to a date of the calendar (Howard Hinnant's civil_from_days)
    let z = days as i64 + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (day, month) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{}, {day:02} {} {year} {:02}:{:02}:{:02} GMT",
        DAYS[(days % 7) as usize],
        MONTHS[(month - 1) as usize],
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// What a request is answered with before anything of the camera is touched.
#[derive(Debug, PartialEq)]
enum Verdict {
    Stream,
    Readout,
    /// a photo by its name, and a video's frame
    Photo(String, Option<i64>),
    /// this node's bus: where the subscriber is, and the paths it wants (none: everything)
    Events(Option<Cursor>, Vec<String>),
    /// this device's people file, for a device that has no library of its own; the SHA-256 of the one the
    /// asking device holds already, when it holds one
    People(Option<String>),
    Deny(u16, serde_json::Value),
}

/// The people file asked for: `/people`, or `/people?have=<the SHA-256 of the one the asker holds>`. None for
/// anything that isn't exactly that.
fn people_of(path: &str) -> Option<Option<String>> {
    match path.strip_prefix("/people")? {
        "" => Some(None),
        rest => {
            let have = rest.strip_prefix("?have=")?;
            let a_sha = have.len() == 64 && have.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            a_sha.then(|| Some(have.to_string()))
        }
    }
}

/// A subscription asked for: `/events`, or with where the subscriber is and which paths it wants
/// (`/events?after=7&epoch=ab12&paths=state,readings`). None for anything that isn't exactly that.
fn events_of(path: &str) -> Option<(Option<Cursor>, Vec<String>)> {
    let query = match path.strip_prefix("/events")? {
        "" => "",
        rest => rest.strip_prefix('?')?,
    };
    let (mut after, mut epoch, mut paths) = (None, None, Vec::new());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        match pair.split_once('=')? {
            ("after", v) => after = Some(v),
            ("epoch", v) => epoch = Some(v),
            ("paths", v) => paths = crate::bus::prefixes(v)?,
            _ => return None,
        }
    }
    let cursor = match (after, epoch) {
        (None, None) => None,
        (after, epoch) => Some(crate::bus::cursor_of(after, epoch)?),
    };
    Some((cursor, paths))
}

/// The checks every request goes through, in their fixed order: a known path, a time within
/// `CLOCK_SKEW` of this device's, and the cluster's signature over method, path, time and asking device.
/// Then this device's own: a device its owner removed on it (`removed`, `face_home::blocked`) is given nothing.
/// (The signature is made with a key every device of the database has, so this binds a device that says
/// who it is, which the app always does.)
fn judge(
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    key: Option<&[u8; 32]>,
    now: u64,
    removed: &[String],
) -> Verdict {
    let deny = |code, why: &str| Verdict::Deny(code, json!({"error": why}));
    if method != "GET" {
        return deny(405, "only GET");
    }
    let (photo, events, people) = (photo_of(path), events_of(path), people_of(path));
    if photo.is_none() && events.is_none() && people.is_none() && path != "/stream" && path != "/readout" {
        return deny(404, "not found");
    }
    let header = |name: &str| headers.get(name).map(String::as_str).unwrap_or("");
    let (device, when, signature) = (header("x-peer-device"), header("x-peer-time"), header("x-peer-sig"));
    let at = if !when.is_empty() && when.bytes().all(|b| b.is_ascii_digit()) { when.parse::<u64>().ok() } else { None };
    match at {
        Some(at) if at.abs_diff(now) <= CLOCK_SKEW => {}
        // the asker says from `now` how far apart the two clocks are
        _ => return Verdict::Deny(403, json!({"error": "stale or missing time", "now": now})),
    }
    match key {
        Some(key) if signed(key, "GET", path, when, device, signature) => {}
        _ => return deny(403, "not signed by a device of this cluster"),
    }
    if removed.iter().any(|id| id == device) {
        return deny(403, "this device was told not to share with that one");
    }
    match (path, photo, events, people) {
        ("/stream", ..) => Verdict::Stream,
        ("/readout", ..) => Verdict::Readout,
        (_, Some((id, frame)), ..) => Verdict::Photo(id, frame),
        (_, _, Some((cursor, paths)), _) => Verdict::Events(cursor, paths),
        (_, _, _, Some(have)) => Verdict::People(have),
        _ => deny(404, "not found"),
    }
}

/// Whether the certificate a device showed in the handshake is the one this device's member list has for the
/// device it says it is. Without one shown, or for a device the list doesn't know, the answer is no.
fn is_member(home: &Path, device: &str, shown: Option<&[u8]>) -> bool {
    let Some(shown) = shown else { return false };
    let sha = hex(ring::digest::digest(&ring::digest::SHA256, shown).as_ref());
    // the certificate of a device removed here is no member's, whatever id it is shown with: a device that
    // has the database's login can write itself a row under a new id, but not make this device forget whom
    // its owner removed
    if face_home::blocked::certs(home).iter().any(|removed| *removed == sha) {
        return false;
    }
    face_db::members::member(home, device).is_some_and(|m| !m.cert_sha256.is_empty() && m.cert_sha256 == sha)
}

/// A subscriber's connection: this node's whole tree (or what changed since the subscriber's place), then
/// every change as it is published, each answer one part of the stream. With nothing to say an empty answer
/// goes out every `EVENTS_BEAT`. It ends when the subscriber closes it, or when its device is removed here.
#[allow(clippy::too_many_arguments)]
fn serve_events(
    tls: &mut rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
    home: &Path,
    bus: &Bus,
    device: &str,
    shown: Option<&[u8]>,
    stopped: &AtomicBool,
    mut at: Option<Cursor>,
    paths: &[String],
) -> std::io::Result<()> {
    tls.write_all(
        format!(
            "HTTP/1.0 200 OK\r\nServer: FaceRecognizerPeer\r\nDate: {}\r\nContent-Type: multipart/x-mixed-replace; boundary={BOUNDARY}\r\nCache-Control: no-store\r\n\r\n",
            http_date(unix_now())
        )
        .as_bytes(),
    )?;
    let until = Instant::now() + STREAM_FOR;
    // (looked at with every answer, a few seconds apart at most: the service switched off, the device removed
    // here, or a member no longer, and the stream ends)
    let goes_on =
        || !stopped.load(Ordering::SeqCst) && !face_home::blocked::is(home, device) && is_member(home, device, shown);
    while Instant::now() < until && goes_on() {
        let answer = bus.since(at.as_ref(), paths, EVENTS_BEAT);
        if let (Some(epoch), Some(seq)) = (answer["epoch"].as_str(), answer["seq"].as_u64()) {
            at = Some(Cursor { epoch: epoch.to_string(), seq });
        }
        tls.write_all(&part("application/json", answer.to_string().as_bytes()))?;
        tls.flush()?;
    }
    tls.conn.send_close_notify();
    tls.flush()?;
    let _ = tls.conn.complete_io(&mut tls.sock);
    Ok(())
}

/// (method, path, headers by lower-case name) of a request's head.
fn parse(head: &[u8]) -> Option<(String, String, HashMap<String, String>)> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Some((method, path, headers))
}

fn answer(code: u16, ctype: &str, body: &[u8]) -> Vec<u8> {
    let reason = match code {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        503 => "Service Unavailable",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let mut out = format!(
        "HTTP/1.0 {code} {reason}\r\nServer: FaceRecognizerPeer\r\nDate: {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n",
        http_date(unix_now()),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn part(ctype: &str, body: &[u8]) -> Vec<u8> {
    let mut out =
        format!("--{BOUNDARY}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out.extend_from_slice(b"\r\n");
    out
}

/// One connection: the handshake, one request, its answer.
fn serve(
    tcp: TcpStream,
    config: Arc<rustls::ServerConfig>,
    home: &Path,
    stream: &Stream,
    key: &Key,
    gives: &Gives,
    bus: &Bus,
    stopped: &AtomicBool,
    n: usize,
) -> std::io::Result<()> {
    let photos = gives.photos.as_ref();
    // (the handshake and the head: a short wait for each byte, and one deadline for all of them)
    let began = Instant::now();
    tcp.set_read_timeout(Some(HEAD_WITHIN / 2))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    tcp.set_nodelay(true)?;
    let conn = rustls::ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    let (mut head, mut buffer) = (Vec::new(), [0u8; 2048]);
    let end = loop {
        if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if head.len() > MAX_HEAD || began.elapsed() > HEAD_WITHIN {
            return Ok(());
        }
        match tls.read(&mut buffer)? {
            0 => return Ok(()),
            got => head.extend_from_slice(&buffer[..got]),
        }
    };
    let verdict = match parse(&head[..end]) {
        Some((method, path, headers)) => {
            let key = *key.read().unwrap_or_else(|e| e.into_inner());
            // (read for each request: a device removed in Settings gets nothing from that moment on)
            let removed = face_home::blocked::ids(home);
            let verdict = judge(&method, &path, &headers, key.as_ref(), unix_now(), &removed);
            let device = headers.get("x-peer-device").cloned().unwrap_or_default();
            let shown = tls.conn.peer_certificates().and_then(|certs| certs.first()).map(|c| c.as_ref().to_vec());
            match verdict {
                Verdict::Stream => Ok(device),
                // the bus and the people only for a device whose certificate says it is the member it names
                Verdict::Events(..) | Verdict::People(_) if !is_member(home, &device, shown.as_deref()) => Err(
                    Verdict::Deny(403, json!({"error": "this device doesn't know that certificate as that member's"})),
                ),
                Verdict::Events(cursor, paths) => {
                    tls.sock.set_read_timeout(Some(TIMEOUT))?;
                    return serve_events(&mut tls, home, bus, &device, shown.as_deref(), stopped, cursor, &paths);
                }
                other => Err(other),
            }
        }
        None => Err(Verdict::Deny(400, json!({"error": "not a request"}))),
    };
    match verdict {
        Err(Verdict::Deny(code, why)) => {
            tls.write_all(&answer(code, "application/json", why.to_string().as_bytes()))?
        }
        // a photo of this device, by its content hash only, from its own records
        Err(Verdict::Photo(id, frame)) => match photos.and_then(|give| give(&id, frame)) {
            Some(jpeg) => tls.write_all(&answer(200, "image/jpeg", &jpeg))?,
            None => {
                let why = json!({"error": "this device doesn't have that photo"});
                tls.write_all(&answer(404, "application/json", why.to_string().as_bytes()))?
            }
        },
        // this device's people, for a member that has no library of its own: the file as it is, or the word
        // that the asker's copy is this one already
        Err(Verdict::People(have)) => {
            match gives.people.as_ref().and_then(|give| give()) {
                None => {
                    let why = json!({"error": "this device has no people of its own"});
                    tls.write_all(&answer(404, "application/json", why.to_string().as_bytes()))?
                }
                Some(file) if have.as_deref() == Some(file.sha256.as_str()) => tls.write_all(&answer(
                    200,
                    "application/json",
                    json!({"same": file.sha256}).to_string().as_bytes(),
                ))?,
                // the file itself: to one device at a time, its bytes written from the one copy there is
                Some(file) => {
                    if PEOPLE_BUSY.swap(true, Ordering::SeqCst) {
                        let why = json!({"error": "busy: asked again in a moment"});
                        tls.write_all(&answer(503, "application/json", why.to_string().as_bytes()))?;
                    } else {
                        let sent = tls
                            .write_all(
                                format!(
                                    "HTTP/1.0 200 OK\r\nServer: FaceRecognizerPeer\r\nDate: {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
                                    http_date(unix_now()),
                                    file.bytes.len()
                                )
                                .as_bytes(),
                            )
                            .and_then(|()| tls.write_all(&file.bytes));
                        PEOPLE_BUSY.store(false, Ordering::SeqCst);
                        sent?;
                    }
                }
            }
        }
        Err(Verdict::Readout | Verdict::Stream | Verdict::Events(..)) => {
            tls.write_all(&answer(200, "application/json", stream.readout().to_string().as_bytes()))?
        }
        Ok(device) => {
            let (mut seq, mut jpeg) = stream.latest();
            if jpeg.is_none() {
                let why = json!({"error": "this device's camera isn't streaming"});
                tls.write_all(&answer(404, "application/json", why.to_string().as_bytes()))?;
            } else {
                tls.write_all(
                    format!(
                        "HTTP/1.0 200 OK\r\nServer: FaceRecognizerPeer\r\nDate: {}\r\nContent-Type: multipart/x-mixed-replace; boundary={BOUNDARY}\r\nCache-Control: no-store\r\n\r\n",
                        http_date(unix_now())
                    )
                    .as_bytes(),
                )?;
                let (until, watcher, mut said) =
                    (Instant::now() + STREAM_FOR, format!("peer:{device}:{n}"), None::<Instant>);
                tls.sock.set_read_timeout(Some(TIMEOUT))?;
                // (the camera's stream too ends when the service is switched off)
                while Instant::now() < until && !stopped.load(Ordering::SeqCst) {
                    let mut out = Vec::new();
                    if let Some(frame) = &jpeg {
                        out = part("image/jpeg", frame);
                    }
                    // the readout rides along: no request of its own per camera
                    if said.is_none_or(|t| t.elapsed() >= READOUT_EVERY) {
                        out.extend_from_slice(&part("application/json", stream.readout().to_string().as_bytes()));
                        said = Some(Instant::now());
                    }
                    if !out.is_empty() {
                        tls.write_all(&out)?;
                        tls.flush()?;
                    }
                    (seq, jpeg) = stream.wait_next(seq, &watcher, READOUT_EVERY);
                }
            }
        }
    }
    tls.conn.send_close_notify();
    tls.flush()?;
    let _ = tls.conn.complete_io(&mut tls.sock);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: &str = "11111111-2222-4333-8444-555555555555";

    fn asked(path: &str, key: &[u8; 32], when: u64, device: &str) -> HashMap<String, String> {
        let when = when.to_string();
        HashMap::from([
            ("x-peer-sig".to_string(), face_db::peer::sign(key, "GET", path, &when, device)),
            ("x-peer-time".to_string(), when),
            ("x-peer-device".to_string(), device.to_string()),
        ])
    }

    #[test]
    fn a_device_removed_on_this_one_is_given_nothing() {
        let key = face_db::peer::derive_key("a password", "a database").unwrap();
        let (now, other) = (1_795_000_000, "99999999-2222-4333-8444-555555555555");
        let removed = [DEVICE.to_string()];
        let photo = format!("/photo/{}", "ab".repeat(20));
        for path in ["/stream", "/readout", photo.as_str()] {
            match judge("GET", path, &asked(path, &key, now, DEVICE), Some(&key), now, &removed) {
                Verdict::Deny(403, why) => assert!(why["error"].as_str().unwrap().contains("not to share"), "{why}"),
                v => panic!("{path} was answered: {v:?}"),
            }
            // another device of the database is answered as before, and so is that one once it is added back
            for (device, list) in [(other, &removed[..]), (DEVICE, &[][..])] {
                let v = judge("GET", path, &asked(path, &key, now, device), Some(&key), now, list);
                assert!(!matches!(v, Verdict::Deny(..)), "{path}: {v:?}");
            }
        }
        // what isn't signed by a device of the database is told nothing about who is removed here
        let stranger = face_db::peer::derive_key("another password", "a database").unwrap();
        match judge("GET", "/stream", &asked("/stream", &stranger, now, DEVICE), Some(&key), now, &removed) {
            Verdict::Deny(403, why) => assert!(why["error"].as_str().unwrap().contains("not signed"), "{why}"),
            v => panic!("answered: {v:?}"),
        }
    }

    #[test]
    fn only_a_device_of_the_cluster_is_answered() {
        let key = face_db::peer::derive_key("a password", "a database").unwrap();
        let other = face_db::peer::derive_key("another password", "a database").unwrap();
        let now = 1_795_000_000;
        assert_eq!(
            judge("GET", "/stream", &asked("/stream", &key, now, DEVICE), Some(&key), now, &[]),
            Verdict::Stream
        );
        assert_eq!(
            judge("GET", "/readout", &asked("/readout", &key, now - 100, DEVICE), Some(&key), now, &[]),
            Verdict::Readout
        );
        let refused = |v: Verdict, code: u16, word: &str| match v {
            Verdict::Deny(c, why) => assert!(c == code && why["error"].as_str().unwrap().contains(word), "{c} {why}"),
            v => panic!("answered: {v:?}"),
        };
        // signed with another cluster's key, for another path, by a device that then changed its name
        refused(
            judge("GET", "/stream", &asked("/stream", &other, now, DEVICE), Some(&key), now, &[]),
            403,
            "not signed",
        );
        refused(
            judge("GET", "/stream", &asked("/readout", &key, now, DEVICE), Some(&key), now, &[]),
            403,
            "not signed",
        );
        let mut renamed = asked("/stream", &key, now, DEVICE);
        renamed.insert("x-peer-device".to_string(), "another".to_string());
        refused(judge("GET", "/stream", &renamed, Some(&key), now, &[]), 403, "not signed");
        // before this device knows the key, nobody is answered
        refused(judge("GET", "/stream", &asked("/stream", &key, now, DEVICE), None, now, &[]), 403, "not signed");
        // a signature kept and sent again later, or no time at all: refused with this device's time
        for when in [now - CLOCK_SKEW - 1, now + CLOCK_SKEW + 1] {
            match judge("GET", "/stream", &asked("/stream", &key, when, DEVICE), Some(&key), now, &[]) {
                Verdict::Deny(403, why) => assert_eq!(why, json!({"error": "stale or missing time", "now": now})),
                v => panic!("answered: {v:?}"),
            }
        }
        refused(judge("GET", "/stream", &HashMap::new(), Some(&key), now, &[]), 403, "stale or missing");
        let mut odd = asked("/stream", &key, now, DEVICE);
        odd.insert("x-peer-time".to_string(), format!("+{now}"));
        refused(judge("GET", "/stream", &odd, Some(&key), now, &[]), 403, "stale or missing");
        // nothing but the camera: no other path, no photo, nothing written
        refused(judge("GET", "/", &asked("/", &key, now, DEVICE), Some(&key), now, &[]), 404, "not found");
        refused(
            judge("GET", "/api/people", &asked("/api/people", &key, now, DEVICE), Some(&key), now, &[]),
            404,
            "not found",
        );
        // a photo: by its content hash only (a video's frame by its number), signed like everything else
        let photo = "/photo/0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            judge("GET", photo, &asked(photo, &key, now, DEVICE), Some(&key), now, &[]),
            Verdict::Photo(photo[7..].to_string(), None)
        );
        let frame = format!("{photo}?frame=120");
        assert_eq!(
            judge("GET", &frame, &asked(&frame, &key, now, DEVICE), Some(&key), now, &[]),
            Verdict::Photo(photo[7..].to_string(), Some(120))
        );
        refused(judge("GET", photo, &HashMap::new(), Some(&key), now, &[]), 403, "stale or missing");
        refused(judge("GET", photo, &asked(photo, &other, now, DEVICE), Some(&key), now, &[]), 403, "not signed");
        for bad in [
            "/photo/",
            "/photo/abc",
            "/photo/../../data/labels.json",
            "/photo/0123456789ABCDEF0123456789abcdef01234567",
        ] {
            refused(judge("GET", bad, &asked(bad, &key, now, DEVICE), Some(&key), now, &[]), 404, "not found");
        }
        for bad in [
            format!("{photo}?frame="),
            format!("{photo}?frame=12345678"),
            format!("{photo}?frame=-1"),
            format!("{photo}?x=1"),
        ] {
            refused(judge("GET", &bad, &asked(&bad, &key, now, DEVICE), Some(&key), now, &[]), 404, "not found");
        }
        refused(judge("POST", "/stream", &asked("/stream", &key, now, DEVICE), Some(&key), now, &[]), 405, "only GET");
    }

    #[test]
    fn a_subscription_is_asked_for_by_its_path_alone_and_signed_like_everything_else() {
        let key = face_db::peer::derive_key("a password", "a database").unwrap();
        let now = 1_795_000_000;
        let asked_for = |path: &str| judge("GET", path, &asked(path, &key, now, DEVICE), Some(&key), now, &[]);
        assert_eq!(asked_for("/events"), Verdict::Events(None, vec![]));
        assert_eq!(
            asked_for("/events?after=7&epoch=ab12&paths=state,sources/camera"),
            Verdict::Events(
                Some(Cursor { epoch: "ab12".to_string(), seq: 7 }),
                vec!["sources/camera".to_string(), "state".to_string()]
            )
        );
        assert_eq!(asked_for("/events?paths=readings"), Verdict::Events(None, vec!["readings".to_string()]));
        // a place is a number and a run, both or neither; nothing else is a question of this route
        for bad in [
            "/events?after=7",
            "/events?epoch=ab12",
            "/events?after=x&epoch=ab12",
            "/events?after=7&epoch=a%20b",
            "/events?paths=a//b",
            "/events?other=1",
            "/events/",
            "/eventsx",
            "/events?after",
        ] {
            assert!(matches!(asked_for(bad), Verdict::Deny(404, _)), "{bad}");
        }
        // unsigned, signed for another path, or by a removed device: refused like the camera
        assert!(matches!(judge("GET", "/events", &HashMap::new(), Some(&key), now, &[]), Verdict::Deny(403, _)));
        let other = asked("/events?after=1&epoch=a", &key, now, DEVICE);
        assert!(matches!(judge("GET", "/events", &other, Some(&key), now, &[]), Verdict::Deny(403, _)));
        let removed = [DEVICE.to_string()];
        assert!(matches!(
            judge("GET", "/events", &asked("/events", &key, now, DEVICE), Some(&key), now, &removed),
            Verdict::Deny(403, _)
        ));
    }

    #[test]
    fn the_people_file_is_asked_for_by_its_path_alone() {
        let key = face_db::peer::derive_key("a password", "a database").unwrap();
        let now = 1_795_000_000;
        let asked_for = |path: &str| judge("GET", path, &asked(path, &key, now, DEVICE), Some(&key), now, &[]);
        assert_eq!(asked_for("/people"), Verdict::People(None));
        let sha = "ab".repeat(32);
        assert_eq!(asked_for(&format!("/people?have={sha}")), Verdict::People(Some(sha.clone())));
        for bad in ["/people?have=", "/people?have=abc", "/people/", "/peoplex", "/people?other=1"] {
            assert!(matches!(asked_for(bad), Verdict::Deny(404, _)), "{bad}");
        }
        let upper = format!("/people?have={}", "AB".repeat(32));
        assert!(matches!(asked_for(&upper), Verdict::Deny(404, _)));
        // unsigned, or by a device removed here: refused like everything else
        assert!(matches!(judge("GET", "/people", &HashMap::new(), Some(&key), now, &[]), Verdict::Deny(403, _)));
        let removed = [DEVICE.to_string()];
        assert!(matches!(
            judge("GET", "/people", &asked("/people", &key, now, DEVICE), Some(&key), now, &removed),
            Verdict::Deny(403, _)
        ));
    }

    #[test]
    fn a_member_is_the_device_whose_certificate_the_member_list_has() {
        let home = std::env::temp_dir().join(format!("face-peer-member-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let cert = b"a certificate's bytes";
        let sha = hex(ring::digest::digest(&ring::digest::SHA256, cert).as_ref());
        let member = |cert_sha256: &str| face_db::members::Member {
            id: DEVICE.to_string(),
            name: "a device".to_string(),
            addresses: vec![],
            port: 8767,
            cert_sha256: cert_sha256.to_string(),
            share: true,
        };
        assert!(!is_member(&home, DEVICE, Some(cert)), "nobody is a member of an empty list");
        face_db::members::keep(&home, None, &[member(&sha)]).unwrap();
        assert!(is_member(&home, DEVICE, Some(cert)));
        // no certificate shown, another one, another device's id, a member without a certificate
        assert!(!is_member(&home, DEVICE, None));
        assert!(!is_member(&home, DEVICE, Some(b"another certificate")));
        assert!(!is_member(&home, "99999999-2222-4333-8444-555555555555", Some(cert)));
        face_db::members::keep(&home, None, &[member("")]).unwrap();
        assert!(!is_member(&home, DEVICE, Some(cert)));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_request_is_read_and_an_answer_written_in_the_fixed_form() {
        let (method, path, headers) = parse(
            b"GET /stream HTTP/1.1\r\nHost: 192.168.1.20\r\nConnection: close\r\nX-Peer-Device: abc\r\nX-Peer-Time: 17",
        )
        .unwrap();
        assert_eq!((method.as_str(), path.as_str()), ("GET", "/stream"));
        assert_eq!((headers["x-peer-device"].as_str(), headers["x-peer-time"].as_str()), ("abc", "17"));
        assert!(parse(b"\xff\xfe").is_none() && parse(b"GET").is_none());
        let text = String::from_utf8(answer(403, "application/json", b"{}")).unwrap();
        assert!(text.starts_with("HTTP/1.0 403 Forbidden\r\nServer: FaceRecognizerPeer\r\nDate: "), "{text}");
        assert!(text.ends_with("GMT\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"), "{text}");
        assert_eq!(
            part("image/jpeg", b"abc"),
            b"--facerecframe\r\nContent-Type: image/jpeg\r\nContent-Length: 3\r\n\r\nabc\r\n"
        );
    }

    #[test]
    fn the_date_is_written_as_http_writes_it() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(951_782_400), "Tue, 29 Feb 2000 00:00:00 GMT");
        assert_eq!(http_date(1_795_161_900), "Fri, 20 Nov 2026 08:05:00 GMT");
        assert_eq!(http_date(1_798_761_599), "Thu, 31 Dec 2026 23:59:59 GMT");
    }

    #[test]
    fn the_certificate_is_made_once_and_one_that_cannot_be_read_is_replaced() {
        let dir = std::env::temp_dir().join(format!("face-peer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (cert, _, sha) = certificate(&dir, &"a very long device name ".repeat(5)).unwrap();
        assert_eq!(sha.len(), 64);
        let (again, _, same) = certificate(&dir, "another name").unwrap();
        assert_eq!((again, same), (cert, sha.clone()), "asked again, it is the same certificate");
        // a key kept in a form this program doesn't have, with a file beside it, can't be read:
        // the device gets a new certificate and key, and the two old files are gone
        let before = std::fs::read(dir.join("peer-cert.pem")).unwrap();
        std::fs::write(
            dir.join("peer-key.pem"),
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n",
        )
        .unwrap();
        std::fs::write(dir.join("peer-key.json"), "{}").unwrap();
        let (renewed, _, new_sha) = certificate(&dir, "x").unwrap();
        assert_ne!(std::fs::read(dir.join("peer-cert.pem")).unwrap(), before);
        assert_ne!(new_sha, sha);
        assert!(!dir.join("peer-key.json").exists());
        // and it is this one from now on
        let (again, _, same) = certificate(&dir, "x").unwrap();
        assert_eq!((again, same), (renewed, new_sha.clone()));
        // a certificate file that isn't one is replaced too
        std::fs::write(dir.join("peer-cert.pem"), "not a certificate").unwrap();
        assert!(certificate(&dir, "x").is_ok_and(|(_, _, third)| third != new_sha));
        let _ = std::fs::remove_dir_all(dir);
    }
}
