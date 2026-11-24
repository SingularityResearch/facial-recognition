//! The owner's Spotify library, read with their permission, for a person's usual music.
//!
//! Only when the owner connects it in Settings: the program never contacts Spotify on its own. What it reads:
//! the saved ("liked") songs of the account that agreed, and the genres Spotify lists for their artists.
//! Read-only (scope user-library-read): nothing in the account can be changed.
//!
//! The permission is Spotify's "authorization code with PKCE", with no client secret: the owner makes an app
//! key of their own (a client id, which is no secret) with the redirect address `redirect_uri`; `begin` gives
//! the address of Spotify's own page, where the owner signs in and agrees; Spotify sends the browser back
//! with a one-time code, which `wait_for_code` takes on 127.0.0.1 (one request, checked against the state it
//! gave out), and `finish` trades for the tokens. The tokens are kept in `HOME/spotify.json`, protected for
//! this user, and reach no page. A redirect is never followed: it would carry a token along.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener},
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use face_files::{
    filejson,
    paths::utc_now,
    protect::{protect, unprotect, write_private},
};
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::{Map, Value, json};

const CONF: &str = "spotify.json";
const SCOPE: &str = "user-library-read";
/// seconds for one request
const TIMEOUT: Duration = Duration::from_secs(20);
/// bytes of one answer
const MAX_ANSWER: u64 = 8_000_000;
/// saved songs in one request (Spotify's most)
const PAGE: usize = 50;
/// a library larger than this is read up to here
const MAX_SONGS: usize = 20_000;
static LOCK: Mutex<()> = Mutex::new(());

/// Where Spotify is asked. A check names a stand-in on this computer; nothing else can.
pub struct Places {
    pub auth: String,
    pub token: String,
    pub api: String,
    /// a stand-in takes Spotify's place: its page is asked for by this program, never opened in a browser
    pub stand_in: bool,
}

impl Places {
    /// Spotify's own addresses; `FACE_TEST_SPOTIFY` names a stand-in for a check, and counts only when it is
    /// on this computer (http://127.0.0.1:port): a token is never sent elsewhere on the environment's word.
    pub fn from_env() -> Places {
        let here = std::env::var("FACE_TEST_SPOTIFY").ok().filter(|url| {
            url.strip_prefix("http://127.0.0.1:")
                .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        });
        match here {
            Some(at) => Places {
                auth: format!("{at}/authorize"),
                token: format!("{at}/api/token"),
                api: format!("{at}/v1"),
                stand_in: true,
            },
            None => Places {
                auth: "https://accounts.spotify.com/authorize".to_string(),
                token: "https://accounts.spotify.com/api/token".to_string(),
                api: "https://api.spotify.com/v1".to_string(),
                stand_in: false,
            },
        }
    }
}

/// What went wrong: Spotify isn't connected here (or the permission was taken back), or anything else.
#[derive(Debug, PartialEq)]
pub enum Fault {
    NotConnected(String),
    Failed(String),
}

impl Fault {
    pub fn text(&self) -> &str {
        match self {
            Fault::NotConnected(why) | Fault::Failed(why) => why,
        }
    }
}

fn failed(why: impl Into<String>) -> Fault {
    Fault::Failed(why.into())
}

/// Where the one-time code arrives: the app's tab page port + 4 (8768), beside the peer service's + 3.
pub fn callback_port() -> u16 {
    let base = std::env::var("FACE_PORTS").unwrap_or_default();
    let base: u32 =
        if !base.is_empty() && base.bytes().all(|b| b.is_ascii_digit()) { base.parse().unwrap_or(8764) } else { 8764 };
    u16::try_from(base + 4).unwrap_or(8768)
}

pub fn redirect_uri() -> String {
    format!("http://127.0.0.1:{}/callback", callback_port())
}

// ----------------------------------------------------------------------------- what is kept

fn load(home: &Path) -> Result<Map<String, Value>, String> {
    match std::fs::read(home.join(CONF)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("can't read {CONF}: {e}")),
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(kept)) => Ok(kept),
            Ok(_) => Ok(Map::new()),
            Err(e) => Err(format!("can't read {CONF}: {e}")),
        },
    }
}

fn save(home: &Path, kept: &Map<String, Value>) -> Result<(), String> {
    write_private(&home.join(CONF), filejson::dumps_lines(&Value::Object(kept.clone()), true, 1).as_bytes())
}

fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false))) && v != Some(&json!("")) && v != Some(&json!(0))
}

/// {connected, client_id, since}: what Settings shows. Never a token.
pub fn status(home: &Path) -> Result<Value, String> {
    let c = load(home)?;
    Ok(
        json!({"connected": truthy(c.get("refresh")), "client_id": c.get("client_id").filter(|id| truthy(Some(id))).cloned().unwrap_or(json!("")),
              "since": c.get("since")}),
    )
}

fn only_the_id(c: &Map<String, Value>) -> Map<String, Value> {
    let mut kept = Map::new();
    kept.insert(
        "client_id".to_string(),
        c.get("client_id").filter(|id| truthy(Some(id))).cloned().unwrap_or(json!("")),
    );
    kept
}

/// Forget the tokens here (the permission itself is taken back in the Spotify account: Apps).
pub fn disconnect(home: &Path) -> Result<(), String> {
    save(home, &only_the_id(&load(home)?))
}

// ----------------------------------------------------------------------------- asking for the permission

pub fn check_client_id(client_id: &str) -> Result<String, String> {
    let client_id = client_id.trim();
    if client_id.chars().count() == 32 && client_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(client_id.to_string());
    }
    Err("a Spotify client id is 32 letters and digits (Settings of your app at developer.spotify.com)".to_string())
}

/// Text that only this program could have made up: `bytes` random bytes, as letters and digits.
fn token(bytes: usize) -> Result<String, String> {
    let mut raw = vec![0u8; bytes];
    SystemRandom::new().fill(&mut raw).map_err(|_| "the system gave no random numbers".to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

/// A form's fields as a request holds them (percent-encoded, a space as "+").
fn form(fields: &[(&str, &str)]) -> String {
    let quoted = |text: &str| -> String {
        text.bytes()
            .map(|b| match b {
                b' ' => "+".to_string(),
                b if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) => (b as char).to_string(),
                b => format!("%{b:02X}"),
            })
            .collect()
    };
    fields.iter().map(|(k, v)| format!("{}={}", quoted(k), quoted(v))).collect::<Vec<_>>().join("&")
}

/// (the address of Spotify's page to open, the verifier, the state) for one request for permission.
pub fn begin(client_id: &str, places: &Places) -> Result<(String, String, String), String> {
    let client_id = check_client_id(client_id)?;
    let (verifier, state) = (token(64)?, token(24)?);
    let challenge = URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()));
    let redirect = redirect_uri();
    let q = form(&[
        ("client_id", &client_id),
        ("response_type", "code"),
        ("redirect_uri", &redirect),
        ("code_challenge_method", "S256"),
        ("code_challenge", &challenge),
        ("scope", SCOPE),
        ("state", &state),
    ]);
    Ok((format!("{}?{q}", places.auth), verifier, state))
}

/// What an address's question part says: its fields, as a browser sends them.
fn query(text: &str) -> Vec<(String, String)> {
    let plain = |part: &str| -> String {
        let bytes = part.replace('+', " ").into_bytes();
        let (mut out, mut i) = (Vec::new(), 0);
        while i < bytes.len() {
            let hex = (bytes[i] == b'%')
                .then(|| {
                    std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
                })
                .flatten();
            match hex {
                Some(byte) => {
                    out.push(byte);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    text.split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (plain(k), plain(v)))
        .collect()
}

/// Whether two texts are the same, in a time that doesn't say where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Listen on 127.0.0.1 for Spotify's one answer to the request `state` belongs to: the code, or why there is
/// none (refused, or nobody answered in time). Anything else that arrives is answered "not found" and
/// ignored. Not on the three servers: their rule refuses requests from another site.
pub fn wait_for_code(state: &str, wait: Duration, port: u16) -> Result<String, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .map_err(|e| format!("port {port} for Spotify's answer is taken: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let end = Instant::now() + wait;
    while Instant::now() < end {
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(copy) => copy,
            Err(_) => continue,
        });
        let mut first = String::new();
        if reader.read_line(&mut first).is_err() {
            continue;
        }
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                break;
            }
        }
        let mut words = first.split_whitespace();
        let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        let (path, asked) = target.split_once('?').unwrap_or((target, ""));
        let fields = query(asked.split('#').next().unwrap_or(""));
        let field = |name: &str| fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        let mine = method == "GET" && path == "/callback" && same(&field("state").unwrap_or_default(), state);
        let (code, refused) = (field("code").filter(|_| mine), field("error").filter(|_| mine));
        let text = match (mine, &code) {
            (true, Some(_)) => "Spotify is connected. You can close this tab.",
            (true, None) => "Spotify was not connected.",
            (false, _) => "Not found.",
        };
        let body = format!("<!doctype html><meta charset=utf-8><title>Face Recognizer</title><p>{text}</p>");
        let _ = write!(
            stream,
            "HTTP/1.1 {} X\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            if mine { 200 } else { 404 },
            body.len()
        );
        let _ = stream.flush();
        if mine {
            return match (code, refused) {
                (Some(code), _) => Ok(code),
                (None, Some(why)) => Err(format!("Spotify said no: {}", why.chars().take(100).collect::<String>())),
                (None, None) => Err("nobody agreed in time".to_string()),
            };
        }
    }
    Err("nobody agreed in time".to_string())
}

fn agent() -> ureq::Agent {
    let t = Some(TIMEOUT);
    let config =
        ureq::Agent::config_builder().max_redirects(0).max_redirects_will_error(false).http_status_as_error(false);
    let config = config.timeout_connect(t).timeout_send_request(t).timeout_send_body(t).timeout_recv_response(t);
    config.timeout_recv_body(t).build().into()
}

/// An answer's status, one of its headers, and what it said (at most `MAX_ANSWER` bytes).
fn read(
    sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    header: &str,
) -> Result<(u16, Option<String>, Vec<u8>), String> {
    let mut answer = sent.map_err(|e| format!("Spotify didn't answer: {e}"))?;
    let said = answer.headers().get(header).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut bytes = Vec::new();
    answer
        .body_mut()
        .as_reader()
        .take(MAX_ANSWER)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Spotify's answer stopped: {e}"))?;
    Ok((answer.status().as_u16(), said, bytes))
}

fn post_token(places: &Places, fields: &[(&str, &str)]) -> Result<Value, String> {
    let sent = agent()
        .post(&places.token)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form(fields).as_bytes());
    let (status, _, said) = read(sent, "location")?;
    if (200..300).contains(&status) {
        return serde_json::from_slice(&said).map_err(|_| "Spotify's answer wasn't understood".to_string());
    }
    let text = String::from_utf8_lossy(&said[..said.len().min(2000)]).into_owned();
    let told: Option<Value> = serde_json::from_str(&text).ok();
    let why = (told.as_ref())
        .and_then(|t| {
            t.get("error_description")
                .filter(|d| truthy(Some(d)))
                .or_else(|| t.get("error").filter(|e| truthy(Some(e))))
        })
        .map_or(text, |why| why.as_str().map_or_else(|| why.to_string(), str::to_string));
    Err(format!("Spotify refused ({status}): {}", why.chars().take(200).collect::<String>()))
}

fn seconds_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn lasts(told: &Value) -> f64 {
    let seconds =
        told.get("expires_in").and_then(|e| e.as_f64().or_else(|| e.as_str()?.parse().ok())).unwrap_or(3600.0);
    seconds_now() + seconds.trunc() - 60.0
}

/// Trade the one-time code for the tokens and keep them (protected).
pub fn finish(home: &Path, places: &Places, code: &str, verifier: &str, client_id: &str) -> Result<Value, String> {
    let client_id = check_client_id(client_id)?;
    let redirect = redirect_uri();
    let told = post_token(
        places,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect),
            ("client_id", &client_id),
            ("code_verifier", verifier),
        ],
    )?;
    let (Some(refresh), Some(access)) = (
        told.get("refresh_token").and_then(Value::as_str).filter(|t| !t.is_empty()),
        told.get("access_token").and_then(Value::as_str).filter(|t| !t.is_empty()),
    ) else {
        return Err("Spotify's answer had no tokens".to_string());
    };
    {
        let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let kept = json!({"client_id": client_id, "refresh": protect(refresh)?, "access": protect(access)?, "expires": lasts(&told), "since": utc_now()});
        save(home, kept.as_object().expect("an object"))?;
    }
    status(home)
}

/// A token for one request, renewed when the last one is used up.
fn access(home: &Path, places: &Places) -> Result<String, Fault> {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = load(home).map_err(failed)?;
    let Some(refresh) = c.get("refresh").and_then(Value::as_str).filter(|r| !r.is_empty()).map(str::to_string) else {
        return Err(Fault::NotConnected("Spotify isn't connected on this device".to_string()));
    };
    let until = c.get("expires").and_then(Value::as_f64).unwrap_or(0.0);
    if let Some(kept) = c.get("access").and_then(Value::as_str).filter(|a| !a.is_empty() && seconds_now() < until) {
        return unprotect(kept).map_err(failed);
    }
    let client_id = c.get("client_id").and_then(Value::as_str).unwrap_or("").to_string();
    let told = match post_token(
        places,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &unprotect(&refresh).map_err(failed)?),
            ("client_id", &client_id),
        ],
    ) {
        Ok(told) => told,
        Err(why) if why.contains("invalid_grant") || why.to_lowercase().contains("revoked") => {
            save(home, &only_the_id(&c)).map_err(failed)?;
            return Err(Fault::NotConnected("Spotify's permission was taken back: connect again".to_string()));
        }
        Err(why) => return Err(failed(why)),
    };
    let Some(new) = told.get("access_token").and_then(Value::as_str) else {
        return Err(failed("Spotify's answer had no tokens"));
    };
    c.insert("access".to_string(), json!(protect(new).map_err(failed)?));
    c.insert("expires".to_string(), json!(lasts(&told)));
    // Spotify may hand out a new one
    if let Some(again) = told.get("refresh_token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        c.insert("refresh".to_string(), json!(protect(again).map_err(failed)?));
    }
    save(home, &c).map_err(failed)?;
    Ok(new.to_string())
}

// ----------------------------------------------------------------------------- reading the library

fn get(home: &Path, places: &Places, path: &str, params: &[(&str, &str)]) -> Result<Value, Fault> {
    let url =
        format!("{}{path}{}", places.api, if params.is_empty() { String::new() } else { format!("?{}", form(params)) });
    for attempt in 0..4 {
        let sent = agent().get(&url).header("Authorization", format!("Bearer {}", access(home, places)?)).call();
        let (status, wait, said) = read(sent, "retry-after").map_err(failed)?;
        match status {
            200..300 => return serde_json::from_slice(&said).map_err(|_| failed("Spotify's answer wasn't understood")),
            // asked too fast: wait as long as Spotify says
            429 if attempt < 3 => {
                let seconds = wait.and_then(|w| w.trim().parse::<u64>().ok()).filter(|w| *w > 0).unwrap_or(2).min(30);
                std::thread::sleep(Duration::from_millis(seconds * 1000 + 500));
            }
            // the token ran out between the check and the request
            401 if attempt == 0 => {
                let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
                let mut c = load(home).map_err(failed)?;
                c.insert("expires".to_string(), json!(0));
                save(home, &c).map_err(failed)?;
            }
            _ => {
                let text: String = String::from_utf8_lossy(&said[..said.len().min(300)]).chars().take(200).collect();
                return Err(failed(format!("Spotify answered {status} for {path}: {text}")));
            }
        }
    }
    Err(failed("Spotify keeps asking to wait"))
}

/// One saved song as the picture needs it: short text and numbers only.
fn song(item: &Value) -> Option<Value> {
    let t = item.get("track")?.as_object()?;
    let text = |v: Option<&Value>, n: usize| {
        v.and_then(Value::as_str).map_or_else(String::new, |s| s.chars().take(n).collect())
    };
    if !truthy(t.get("id")) || t.get("type").is_some_and(|kind| kind != "track") {
        return None;
    }
    let album = t.get("album").filter(|a| truthy(Some(a)));
    let of_album = |k: &str| album.and_then(|a| a.get(k));
    let artists: Vec<Value> = (t.get("artists").and_then(Value::as_array).into_iter().flatten().take(8))
        .filter(|a| a.is_object())
        .map(|a| json!({"id": text(a.get("id"), 40), "name": text(a.get("name"), 200)}))
        .collect();
    let ms = t.get("duration_ms").and_then(|d| d.as_i64().or_else(|| d.as_f64().map(|f| f as i64))).unwrap_or(0);
    let popularity = t.get("popularity").filter(|p| p.is_i64() || p.is_u64());
    Some(
        json!({"id": text(t.get("id"), 40), "name": text(t.get("name"), 200), "artists": artists, "album": text(of_album("name"), 200),
                "released": text(of_album("release_date"), 10), "added": text(item.get("added_at"), 25), "ms": ms, "explicit": truthy(t.get("explicit")),
                "popularity": popularity}),
    )
}

/// Every saved song of the connected account, newest first. `progress(done, total)` is called as the pages
/// come in.
pub fn liked(home: &Path, places: &Places, progress: &mut dyn FnMut(usize, usize)) -> Result<Vec<Value>, Fault> {
    let (mut out, mut offset, mut total) = (Vec::new(), 0usize, None::<usize>);
    while total.is_none_or(|total| offset < total && out.len() < MAX_SONGS) {
        let page = get(home, places, "/me/tracks", &[("limit", &PAGE.to_string()), ("offset", &offset.to_string())])?;
        let all = page.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
        total = Some(all);
        let items = page.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        out.extend(items.iter().filter_map(song));
        offset += items.len();
        progress(offset.min(all), all);
    }
    Ok(out)
}

/// {artist id: [genres]} as Spotify lists them (many artists have none). Empty when Spotify no longer gives
/// them to this kind of key: the picture then goes without genres.
pub fn genres(
    home: &Path,
    places: &Places,
    artist_ids: &[String],
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Map<String, Value>, Fault> {
    let mut ids: Vec<&str> = artist_ids.iter().map(String::as_str).filter(|id| !id.is_empty()).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out = Map::new();
    for (k, some) in ids.chunks(50).enumerate() {
        let page = match get(home, places, "/artists", &[("ids", &some.join(","))]) {
            Ok(page) => page,
            // not allowed, or gone: no genres at all
            Err(Fault::Failed(_)) if k == 0 => return Ok(Map::new()),
            Err(why) => return Err(why),
        };
        for a in page.get("artists").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = a.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else { continue };
            let listed: Vec<String> = (a.get("genres").and_then(Value::as_array).into_iter().flatten())
                .filter_map(Value::as_str)
                .map(|g| g.chars().take(60).collect())
                .take(12)
                .collect();
            out.insert(id.to_string(), json!(listed));
        }
        progress(((k + 1) * 50).min(ids.len()), ids.len());
    }
    Ok(out)
}

/// Open an address in the user's web browser.
fn open_in_browser(url: &str) {
    let mut ask = if cfg!(windows) {
        let mut ask = std::process::Command::new("rundll32");
        ask.args(["url.dll,FileProtocolHandler", url]);
        ask
    } else {
        let mut ask = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
        ask.arg(url);
        ask
    };
    let _ = ask
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Ask for the permission and wait for the owner to agree in their browser (five minutes at most).
pub fn connect(home: &Path, places: &Places, client_id: &str) -> Result<Value, String> {
    let (url, verifier, state) = begin(client_id, places)?;
    let stand_in = places.stand_in;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        if stand_in {
            // a check's stand-in is asked in the browser's place: no browser is opened
            let _ = agent().get(&url).call();
        } else {
            open_in_browser(&url);
        }
    });
    let code = wait_for_code(&state, Duration::from_secs(300), callback_port())?;
    finish(home, places, &code, &verifier, client_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_id_is_checked_and_a_request_for_permission_is_made_up() {
        let id = "0123456789abcdefABCDEF0123456789";
        assert_eq!(check_client_id(&format!("  {id} ")).unwrap(), id);
        for bad in ["", "0123", &"g".repeat(32), &"0".repeat(33)] {
            assert!(check_client_id(bad).is_err(), "{bad}");
        }
        let places = Places {
            auth: "https://example.com/authorize".into(),
            token: String::new(),
            api: String::new(),
            stand_in: false,
        };
        let (url, verifier, state) = begin(id, &places).unwrap();
        let (to, asked) = url.split_once('?').unwrap();
        let fields = query(asked);
        let field = |name: &str| fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("");
        assert_eq!(to, "https://example.com/authorize");
        assert_eq!(
            (
                field("client_id"),
                field("response_type"),
                field("code_challenge_method"),
                field("scope"),
                field("state")
            ),
            (id, "code", "S256", SCOPE, state.as_str())
        );
        assert_eq!(field("redirect_uri"), redirect_uri());
        assert!(asked.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"), "written as a form writes it");
        // the challenge is the verifier's SHA-256, and neither is the same twice
        assert_eq!(
            field("code_challenge"),
            URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()))
        );
        assert_eq!((verifier.len(), state.len()), (86, 32));
        let again = begin(id, &places).unwrap();
        assert!(again.1 != verifier && again.2 != state);
        assert_eq!(form(&[("a b", "c&d=e/f"), ("k", "v-_.~")]), "a+b=c%26d%3De%2Ff&k=v-_.~");
        assert_eq!(
            query("a+b=c%26d&empty=&x=%zz&plain"),
            [("a b".to_string(), "c&d".to_string()), ("x".to_string(), "%zz".to_string())]
        );
    }

    #[test]
    fn only_the_answer_to_this_request_is_taken() {
        let ask = |port: u16, path: &str| -> (u16, String) {
            let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            let mut said = String::new();
            stream.read_to_string(&mut said).unwrap();
            (said.split_whitespace().nth(1).unwrap().parse().unwrap(), said)
        };
        let free = || TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        // another request first, then one with another state, then the one
        let port = free();
        let waiting = std::thread::spawn(move || wait_for_code("the-state", Duration::from_secs(20), port));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(ask(port, "/").0, 404);
        assert_eq!(ask(port, "/callback?code=stolen&state=another").0, 404);
        assert_eq!(ask(port, "/elsewhere?code=x&state=the-state").0, 404);
        let (status, said) = ask(port, "/callback?code=the%2Bcode&state=the-state");
        assert!(status == 200 && said.contains("Spotify is connected.") && said.contains("Cache-Control: no-store"));
        assert_eq!(waiting.join().unwrap(), Ok("the+code".to_string()));
        // the owner said no
        let port = free();
        let waiting = std::thread::spawn(move || wait_for_code("s2", Duration::from_secs(20), port));
        std::thread::sleep(Duration::from_millis(300));
        assert!(ask(port, "/callback?error=access_denied&state=s2").1.contains("Spotify was not connected."));
        assert_eq!(waiting.join().unwrap(), Err("Spotify said no: access_denied".to_string()));
        // nobody answers
        assert_eq!(wait_for_code("s3", Duration::from_millis(300), free()), Err("nobody agreed in time".to_string()));
    }

    #[test]
    fn a_song_is_kept_as_short_text_and_numbers() {
        let item = json!({"added_at": "2026-11-01T10:00:00Z", "track": {"id": "t1", "name": "A song", "type": "track", "duration_ms": 201500, "explicit": true, "popularity": 40,
                          "artists": [{"id": "a1", "name": "An artist"}, "odd", {"name": "Another"}], "album": {"name": "An album", "release_date": "2019-05-17 and more"}}});
        assert_eq!(
            song(&item).unwrap(),
            json!({"id": "t1", "name": "A song", "artists": [{"id": "a1", "name": "An artist"}, {"id": "", "name": "Another"}], "album": "An album",
                                                 "released": "2019-05-17", "added": "2026-11-01T10:00:00Z", "ms": 201500, "explicit": true, "popularity": 40})
        );
        assert_eq!(
            song(&json!({"track": {"id": "t2", "popularity": "high"}})).unwrap(),
            json!({"id": "t2", "name": "", "artists": [], "album": "", "released": "", "added": "",
                                                                                                    "ms": 0, "explicit": false, "popularity": null})
        );
        for not_one in [
            json!({}),
            json!({"track": null}),
            json!({"track": {"name": "no id"}}),
            json!({"track": {"id": "e1", "type": "episode"}}),
            json!({"track": "words"}),
        ] {
            assert_eq!(song(&not_one), None, "{not_one}");
        }
    }

    #[test]
    fn only_a_stand_in_on_this_computer_can_take_spotifys_place() {
        // (the name is read when asked, so this test alone sets it)
        unsafe { std::env::set_var("FACE_TEST_SPOTIFY", "http://192.168.1.20:8000") };
        assert!(!Places::from_env().stand_in && Places::from_env().token == "https://accounts.spotify.com/api/token");
        unsafe { std::env::set_var("FACE_TEST_SPOTIFY", "http://127.0.0.1:8123/elsewhere") };
        assert!(!Places::from_env().stand_in);
        unsafe { std::env::set_var("FACE_TEST_SPOTIFY", "http://127.0.0.1:8123") };
        let places = Places::from_env();
        assert_eq!(
            (places.stand_in, places.auth.as_str(), places.api.as_str()),
            (true, "http://127.0.0.1:8123/authorize", "http://127.0.0.1:8123/v1")
        );
        unsafe { std::env::remove_var("FACE_TEST_SPOTIFY") };
    }
}
