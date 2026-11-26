//! What the app's two processes ask each other.
//!
//! The tracker runs in a process of its own, so what used to be a look into the other program's memory is
//! a question over 127.0.0.1 to the other one's own server, under `/api/local/`: never a page's request.
//! Each carries the secret the app makes for this run (`FACE_LINK` in the environment of the process it
//! starts: no page, file or log has it) and the tools' own header.
//!
//! ```text
//! the app, the label side  ->  the tracker:    hello (is it the process this app started?), where its
//!                                              models run, the Cameras page's own port, who is in view
//! the tracker              ->  the label side: a photo for another device, "something was saved"
//! ```
//!
//! `hello` is the one question that takes no secret: it gives nothing away, it proves. The app sends the
//! secret to nobody who hasn't shown that it has it already.

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Mutex,
        atomic::{AtomicU16, Ordering},
    },
    time::Duration,
};

use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{Map, Value};

/// what a local route answers a request that doesn't carry this run's secret
pub const REFUSAL: &str = "only the app's own processes may ask for that";
/// seconds to wait for the other process's answer
pub const TIMEOUT: Duration = Duration::from_secs(4);

/// this run's secret: made by the app, inherited by the process it starts
static TOKEN: Mutex<Option<String>> = Mutex::new(None);
/// the tracker's process's port while it runs (0: down, or the tracker isn't a process of its own)
static TRACKER: AtomicU16 = AtomicU16::new(0);
/// in the tracker's process: the port the Cameras page is also served on (0: none)
static CAMERAS: AtomicU16 = AtomicU16::new(0);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(bytes: usize) -> String {
    let mut raw = vec![0u8; bytes];
    // (the system's own source: an error leaves zeros, and then nothing proves anything, which fails closed
    // since the two sides' secrets are made by one process)
    let _ = SystemRandom::new().fill(&mut raw);
    hex(&raw)
}

/// This run's secret: the one made here, else the one this process was started with.
pub fn token() -> String {
    let mut kept = TOKEN.lock().unwrap_or_else(|e| e.into_inner());
    kept.get_or_insert_with(|| std::env::var("FACE_LINK").unwrap_or_default()).clone()
}

/// Make this run's secret (the app, once, before it starts the tracker's process).
pub fn new_token() -> String {
    let made = random_hex(32);
    *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(made.clone());
    made
}

/// Two texts compared without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Is this request one of the app's own two processes'? `headers` by lower-case name.
pub fn allowed(headers: &HashMap<String, String>) -> bool {
    let token = token();
    !token.is_empty() && same(headers.get("x-face-link").map_or("", String::as_str), &token)
}

/// What only a process that has this run's secret can answer to `nonce`.
pub fn proof(nonce: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, token().as_bytes());
    hex(hmac::sign(&key, nonce.as_bytes()).as_ref())
}

/// A GET (or, with a body, a POST of JSON) to a server on this computer, with the tools' own header and,
/// when `link`, this run's secret: (status, body), or None when nothing answers there in time.
pub fn ask(port: u16, path: &str, body: Option<&Value>, timeout: Duration, link: bool) -> Option<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let data = body.map(|b| b.to_string()).unwrap_or_default();
    let mut ask = format!(
        "{} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Face-Tool: 1\r\nConnection: close\r\n",
        if body.is_some() { "POST" } else { "GET" }
    );
    if link {
        ask.push_str(&format!("X-Face-Link: {}\r\n", token()));
    }
    if body.is_some() {
        ask.push_str(&format!(
            "Content-Type: application/json\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Length: {}\r\n",
            data.len()
        ));
    }
    ask.push_str("\r\n");
    stream.write_all(ask.as_bytes()).ok()?;
    stream.write_all(data.as_bytes()).ok()?;
    let mut answer = Vec::new();
    stream.take(70_000_000).read_to_end(&mut answer).ok()?;
    let cut = answer.windows(4).position(|w| w == b"\r\n\r\n")?;
    let status = std::str::from_utf8(&answer[..cut]).ok()?.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, answer[cut + 4..].to_vec()))
}

fn asked(port: u16, path: &str) -> Option<Value> {
    let (status, body) = ask(port, path, None, TIMEOUT, true)?;
    (status == 200).then(|| serde_json::from_slice(&body).ok()).flatten()
}

/// True when the server on this port is the tracker this app started: it can answer what only a holder of
/// this run's secret can. The secret itself isn't sent.
pub fn hello(port: u16, timeout: Duration) -> bool {
    if token().is_empty() {
        return false;
    }
    let nonce = random_hex(16);
    let said = ask(port, &format!("/api/local/hello?n={nonce}"), None, timeout, false)
        .and_then(|(_, body)| serde_json::from_slice::<Value>(&body).ok());
    said.is_some_and(|got| same(got["proof"].as_str().unwrap_or(""), &proof(&nonce)))
}

/// The tracker's process serves on this port now (None: it is down).
pub fn set_tracker(port: Option<u16>) {
    TRACKER.store(port.unwrap_or(0), Ordering::SeqCst);
}

fn tracker() -> Option<u16> {
    Some(TRACKER.load(Ordering::SeqCst)).filter(|p| *p != 0)
}

/// ({job: where it runs}, {model file: why}) of the tracker's models, for Settings' part about this
/// computer. Empty while the tracker is in no process of its own or doesn't answer.
pub fn devices() -> (Map<String, Value>, Map<String, Value>) {
    let got = tracker().and_then(|port| asked(port, "/api/local/devices")).unwrap_or(Value::Null);
    let part = |name: &str| got[name].as_object().cloned().unwrap_or_default();
    (part("devices"), part("decisions"))
}

/// What every device's bus says now, as the tracker's process gathers it for Analyze: {nodes: [{id, name, me,
/// whole, said}]} (each node's tree, and what it said of who is in view and their readings in the next `wait`).
/// No nodes while the tracker is in no process of its own or doesn't answer.
pub fn now(wait: Duration) -> Value {
    let ms = wait.as_millis().clamp(500, 6000) as u64;
    let got = tracker().and_then(|port| ask(port, &format!("/api/local/now?wait={ms}"), None, wait + TIMEOUT, true));
    got.filter(|(status, _)| *status == 200)
        .and_then(|(_, body)| serde_json::from_slice(&body).ok())
        .unwrap_or_else(|| serde_json::json!({"nodes": []}))
}

/// The port the tracker's process serves the Cameras page on besides its own, or None.
pub fn cameras_port() -> Option<u16> {
    let port = tracker().and_then(|port| asked(port, "/api/local/cameras"))?["port"].as_u64()?;
    u16::try_from(port).ok().filter(|p| *p >= 1024)
}

/// In the tracker's process: the Cameras page is also served on this port.
pub fn set_cameras(port: u16) {
    CAMERAS.store(port, Ordering::SeqCst);
}

/// In the tracker's process: the Cameras page's own port, when it has one.
pub fn cameras() -> Option<u16> {
    Some(CAMERAS.load(Ordering::SeqCst)).filter(|p| *p != 0)
}

/// in the tracker's process: the labeling tool's port, once that side has said it
static LABEL: AtomicU16 = AtomicU16::new(0);
/// in the label side's process: its own port
static OWN_LABEL: AtomicU16 = AtomicU16::new(0);

/// The label side: this is the port the labeling tool serves on.
pub fn set_own_label(port: u16) {
    OWN_LABEL.store(port, Ordering::SeqCst);
}

/// The tracker's process: the labeling tool said where it serves (`http://127.0.0.1:<port>/`, nothing else).
pub fn set_label(said: &str) {
    let port = said.strip_prefix("http://127.0.0.1:").and_then(|rest| rest.strip_suffix('/'));
    let port = port.filter(|p| (2..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()));
    if let Some(port) = port.and_then(|p| p.parse::<u16>().ok()) {
        LABEL.store(port, Ordering::SeqCst);
    }
}

/// Whether the tracker runs in a process of its own that this one knows (the app).
pub fn remote() -> bool {
    tracker().is_some()
}

/// The label side: the peer service on or off, where the camera's frames are. Ok(it runs). A tracker that
/// doesn't answer is being started again: not shared for this step, asked again at the next, never an error.
/// Err: the tracker's process couldn't start the service (its port is taken).
pub fn peer_service(want: bool) -> Result<bool, String> {
    let Some(port) = tracker() else { return Ok(false) };
    let label = format!("http://127.0.0.1:{}/", OWN_LABEL.load(Ordering::SeqCst));
    let said = ask(port, "/api/local/peer", Some(&serde_json::json!({"run": want, "label": label})), TIMEOUT, true)
        .filter(|(status, _)| *status == 200)
        .and_then(|(_, body)| serde_json::from_slice::<Value>(&body).ok());
    let Some(got) = said else { return Ok(false) };
    match got["error"].as_str() {
        Some(why) => Err(why.to_string()),
        None => Ok(got["running"] == true),
    }
}

/// The label side: the camera's own part in the shared database (that the device is there, whether it
/// streams) is kept going where the camera's frames are: the tracker's process starts it when asked this.
pub fn camera_services() {
    if let Some(port) = tracker() {
        let _ = ask(port, "/api/local/inview", None, TIMEOUT, true);
    }
}

/// The label side: the names this device's camera sees now (the sync's sightings). Nobody while the tracker
/// is starting again.
pub fn in_view() -> Vec<String> {
    let got = tracker().and_then(|port| asked(port, "/api/local/inview?within=6")).unwrap_or(Value::Null);
    got["names"].as_object().map(|names| names.keys().cloned().collect()).unwrap_or_default()
}

/// something the sync sends was saved in the tracker's process since it was last told
static SAVED: (Mutex<bool>, std::sync::Condvar) = (Mutex::new(false), std::sync::Condvar::new());

/// The tracker's process: something the sync sends was saved here (an alert, a visit). The label side is
/// told, so that it goes out now and not at the sync's next look a minute later. Never waits.
pub fn note_saved() {
    *SAVED.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    SAVED.1.notify_all();
}

/// The tracker's process: the thread that tells the label side (a few saves in a row: one word for all).
pub fn start_telling() {
    let _ = std::thread::Builder::new().name("tell the sync".into()).spawn(|| {
        loop {
            {
                let mut saved = SAVED.0.lock().unwrap_or_else(|e| e.into_inner());
                while !*saved {
                    saved = SAVED.1.wait(saved).unwrap_or_else(|e| e.into_inner());
                }
            }
            std::thread::sleep(Duration::from_millis(300));
            *SAVED.0.lock().unwrap_or_else(|e| e.into_inner()) = false;
            if let Some(port) = Some(LABEL.load(Ordering::SeqCst)).filter(|p| *p != 0) {
                // (not answered: the sync finds it at its next look)
                let _ = ask(port, "/api/local/saved", Some(&serde_json::json!({})), TIMEOUT, true);
            }
        }
    });
}

/// The tracker's process: a photo of this device for another one. The labeling tool has the photos.
pub fn photo(id: &str, frame: Option<i64>) -> Option<Vec<u8>> {
    let port = Some(LABEL.load(Ordering::SeqCst)).filter(|p| *p != 0)?;
    let path = match frame {
        Some(k) => format!("/api/local/photo/{id}?frame={k}"),
        None => format!("/api/local/photo/{id}"),
    };
    let (status, body) = ask(port, &path, None, Duration::from_secs(20), true)?;
    (status == 200 && !body.is_empty()).then_some(body)
}

/// A photo's name in a request, as the peer service takes them: its content hash, or the name of one whose
/// file couldn't be read.
pub fn photo_id(id: &str) -> bool {
    let hex = |text: &str, n: usize| text.len() == n && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    hex(id, 40)
        || id.strip_prefix("path-").is_some_and(|t| hex(t, 24))
        || id.strip_prefix("nofile-").is_some_and(|t| hex(t, 24))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_holder_of_the_secret_is_let_in_and_can_prove_it() {
        // (one test for everything that uses the process's secret: tests run at the same time)
        let secret = new_token();
        assert_eq!(secret.len(), 64);
        assert_ne!(secret, new_token(), "a secret of its own for each run");
        let secret = token();
        let with = |value: &str| HashMap::from([("x-face-link".to_string(), value.to_string())]);
        assert!(allowed(&with(&secret)));
        assert!(!allowed(&with("")) && !allowed(&HashMap::new()));
        assert!(!allowed(&with(&secret[..63])) && !allowed(&with(&format!("{secret}0"))));
        let mut other = secret.clone().into_bytes();
        other[10] ^= 1;
        assert!(!allowed(&with(&String::from_utf8(other).unwrap())));
        // the proof is the keyed check value of what was asked
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        assert_eq!(proof("abc"), hex(hmac::sign(&key, b"abc").as_ref()));
        assert_ne!(proof("abc"), proof("abd"));
        assert_eq!(proof("abc").len(), 64);
        // nothing answers on a port nobody listens on: not the tracker
        assert!(!hello(9, Duration::from_millis(200)));
        assert_eq!(devices(), (Map::new(), Map::new()));
        assert_eq!(cameras_port(), None);
    }

    #[test]
    fn the_labeling_tool_is_only_ever_on_this_computer() {
        // (one test for the label's port: tests run at the same time)
        for bad in [
            "http://192.168.1.5:8765/",
            "http://127.0.0.1:8765",
            "https://127.0.0.1:8765/",
            "http://127.0.0.1:1/",
            "http://127.0.0.1:876543/",
            "http://127.0.0.1:87a5/",
            "http://127.0.0.1:8765/x",
            "",
        ] {
            set_label(bad);
            assert_eq!(LABEL.load(Ordering::SeqCst), 0, "{bad}");
        }
        assert_eq!(photo(&"ab12".repeat(10), None), None, "no labeling tool known: no photo");
        set_label("http://127.0.0.1:99/");
        assert_eq!(LABEL.load(Ordering::SeqCst), 99);
        assert_eq!(photo(&"ab12".repeat(10), Some(3)), None, "nothing answers there: no photo");
        LABEL.store(0, Ordering::SeqCst);
        // no tracker's process known: nothing is shared, and that is no error
        assert_eq!(peer_service(true), Ok(false));
        assert!(!remote());
    }

    #[test]
    fn a_photo_is_named_by_its_hash_only() {
        assert!(photo_id(&"ab12".repeat(10)));
        assert!(photo_id(&format!("path-{}", "0f".repeat(12))) && photo_id(&format!("nofile-{}", "0f".repeat(12))));
        for bad in [
            "",
            "abc",
            &"AB12".repeat(10),
            &"ab12".repeat(11),
            "../../etc/passwd",
            "path-12",
            &format!("{}/x", "ab12".repeat(10)),
        ] {
            assert!(!photo_id(bad), "{bad}");
        }
    }
}
