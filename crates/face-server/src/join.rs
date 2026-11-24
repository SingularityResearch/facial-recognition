//! Joining a shared database from the pages (`face_db::join` is the exchange itself). Either device shows
//! the join code as a QR code and the other reads it with its camera:
//!
//! - a device of the database shows one, the new device reads it, asks for what that device hands over (the
//!   login, and the passphrase for keys when it holds it) and keeps it;
//! - the new device shows one, a device of the database reads it, asks its owner, and hands the same over.
//!
//! A device of the database hands nothing over without its owner: the click that shows a code, or the yes
//! to the new device whose code it read, and before either the system's own check of who is at the device
//! where it has one (`OwnerCheck`).
//!
//! Two places show a code: the tracker's own page for it (`/join`, on any device that runs this
//! program), and `face-server pair`, a command that shows codes for several new devices one after another.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use face_core::image::Bgr;
use face_db::{
    Settings,
    join::{self, Handover, Kind, Offer, Outcome, Received, Want},
};
use serde_json::{Value, json};

use crate::http::{self, Request, Response, Service};

/// The join page.
pub const PAGE: &str = include_str!("../pages/join_ui.html");
pub const SCRIPT: &str = include_str!("../pages/join_ui.js");

/// modules of white around a code, as its readers expect
const QUIET: usize = 4;

/// The code's squares, dark or not, row after row, and how many a row has.
fn modules(text: &str) -> Result<(Vec<bool>, usize), String> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("The join code couldn't be drawn: {e}"))?;
    Ok((code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect(), code.width()))
}

/// `text` as a QR code, drawn as an SVG picture.
pub fn svg(text: &str) -> Result<String, String> {
    let (dark, n) = modules(text)?;
    let side = n + 2 * QUIET;
    let mut d = String::new();
    for (i, _) in dark.iter().enumerate().filter(|(_, on)| **on) {
        let _ = write!(d, "M{} {}h1v1h-1z", i % n + QUIET, i / n + QUIET);
    }
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {side}" shape-rendering="crispEdges"><rect width="{side}" height="{side}" fill="#fff"/><path d="{d}" fill="#000"/></svg>"##
    ))
}

/// The join code in a camera's picture, when there is one to read.
pub fn read(img: &Bgr) -> Option<String> {
    let grey: Vec<u8> = img
        .px
        .chunks_exact(3)
        .map(|p| ((p[0] as u32 * 29 + p[1] as u32 * 150 + p[2] as u32 * 77) >> 8) as u8)
        .collect();
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(img.w, img.h, |x, y| grey[y * img.w + x]);
    prepared
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok())
        .map(|(_, text)| text.trim().to_string())
        .find(|text| text.starts_with(join::PREFIX))
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How a showing ended, in a word for the page.
fn word(o: &Outcome) -> &'static str {
    match o {
        Outcome::Given(_) => "given",
        Outcome::Expired => "expired",
        Outcome::Refused => "refused",
        Outcome::Stopped => "stopped",
    }
}

/// What the log says when a showing by a device of the database ended. `keys`: the passphrase went too.
fn log_line(o: &Outcome, keys: bool) -> String {
    match o {
        Outcome::Given(from) => format!(
            "Join code: a device at {from} read it and was handed the shared database's login{}.",
            if keys { " and the passphrase for keys" } else { "" }
        ),
        Outcome::Expired => "Join code: nobody read it in time.".to_string(),
        Outcome::Refused => "Join code: ended, after several asks with a wrong key.".to_string(),
        Outcome::Stopped => "Join code: no longer shown.".to_string(),
    }
}

/// What this device hands a device it lets in: the login, and the passphrase for keys when it holds it.
fn handover(settings: &Settings, home: &Path, name: &str) -> Handover {
    Handover::new(settings.login(), face_home::passphrase::to_hand_on(home), name)
}

/// Keep what another device handed over: the login (checked by connecting with it, `face_db::setup`), then the
/// passphrase for keys, which this device's sync takes when it is the current one.
fn keep(home: &Path, handed: &Handover) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(face_db::setup(&handed.login, home))?;
    if let Some(passphrase) = &handed.passphrase {
        face_home::passphrase::receive(home, passphrase)?;
    }
    Ok(())
}

/// What the device's own system says about who is at it, when it is asked before something is handed over.
pub enum Checked {
    /// the system checked (a face, a finger, the device's own code) and it is the owner
    Yes,
    /// it isn't, or the owner said no: what the page says
    No(String),
    /// this device has no such check (or none is set up): the click is all there is
    Unavailable,
}

/// The system's own check that the owner is at the device, with what the system's question says.
pub type OwnerCheck = Arc<dyn Fn(&str) -> Checked + Send + Sync>;

/// How a showing ended: a word for the page, and why when it failed.
type Ended = (&'static str, Option<String>);

struct Shown {
    svg: String,
    /// "give" (this device is in the database) or "want" (it wants to join)
    kind: &'static str,
    until: Instant,
    stop: Arc<AtomicBool>,
    ended: Arc<Mutex<Option<Ended>>>,
}

/// A new device's code that this device read, kept until the owner said yes or no.
struct Asked {
    text: String,
    name: String,
    address: String,
}

/// The tracker's part in joining: reading a code with this device's camera, and showing one.
pub struct Joining {
    home: PathBuf,
    /// a code that was read is being used right now
    busy: AtomicBool,
    /// the code that failed last and why: the camera sees it many times, it is asked about once
    failed: Mutex<Option<(String, String)>>,
    shown: Mutex<Option<Shown>>,
    asked: Mutex<Option<Asked>>,
    /// this device joined by a code it showed, and nobody was told yet (`joined_now`)
    joined_now: Arc<AtomicBool>,
    check: Mutex<Option<OwnerCheck>>,
}

impl Joining {
    pub fn new(home: &Path) -> Joining {
        Joining {
            home: home.to_path_buf(),
            busy: AtomicBool::new(false),
            failed: Mutex::new(None),
            shown: Mutex::new(None),
            asked: Mutex::new(None),
            joined_now: Arc::new(AtomicBool::new(false)),
            check: Mutex::new(None),
        }
    }

    /// The system's own check of who is at this device, asked before this device hands anything over.
    pub fn check_with(&self, check: OwnerCheck) {
        *self.check.lock().unwrap_or_else(|e| e.into_inner()) = Some(check);
    }

    /// Whether this device may hand over what it has now: yes where the system has no check of its own.
    fn allowed(&self) -> Result<(), String> {
        let check = self.check.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match check.map(|ask| ask("let a new device into your shared database")) {
            Some(Checked::No(why)) => Err(why),
            _ => Ok(()),
        }
    }

    /// Whether this device has a shared database.
    pub fn joined(&self) -> bool {
        matches!(Settings::load(&self.home), Ok(Some(_)))
    }

    /// Once after this device joined by a code it showed itself: the tracker then starts what a device of a
    /// shared database does (reading a code joins inside the request, which says so itself).
    pub fn joined_now(&self) -> bool {
        self.joined_now.swap(false, Ordering::SeqCst)
    }

    /// What the join page shows: joined or not, the code this device shows now or how its last one ended,
    /// and the new device whose code it read and asks the owner about.
    pub fn state(&self, name: &str) -> Value {
        let shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
        let (showing, last, error) = match shown.as_ref() {
            None => (Value::Null, Value::Null, Value::Null),
            Some(s) => match s.ended.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                Some((word, why)) => (Value::Null, json!(word), json!(why)),
                None => (
                    json!({"left": s.until.saturating_duration_since(Instant::now()).as_secs(), "kind": s.kind}),
                    Value::Null,
                    Value::Null,
                ),
            },
        };
        let asked = self.asked.lock().unwrap_or_else(|e| e.into_inner());
        let asked = asked.as_ref().map(|a| json!({"name": a.name, "address": a.address}));
        json!({"joined": self.joined(), "name": name, "showing": showing, "last": last, "error": error, "asked": asked})
    }

    /// One picture of this device's camera while it looks for a join code (`code`).
    pub fn frame(&self, jpeg: &[u8]) -> Result<Value, String> {
        let img = Bgr::from_jpeg(jpeg).map_err(|e| e.0)?;
        let Some(text) = read(&img) else { return Ok(json!({"found": false})) };
        Ok(self.code(&text))
    }

    /// A join code as text: read off a picture here, or by the phone's own camera, which opened the app
    /// with it (the shell hands it to the join page, and the page asks the owner first).
    ///
    /// On a device without a shared database a code that a device of one shows is used at once: this device
    /// asks for what that one hands over and keeps it. On a device with one, a code that a new device shows
    /// is only kept: nothing is handed over before the owner said so (`give`).
    pub fn code(&self, text: &str) -> Value {
        let text = text.trim().to_string();
        if let Some((code, why)) = self.failed.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if *code == text {
                return json!({"found": true, "error": why});
            }
        }
        let refused = |why: String| {
            *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some((text.clone(), why.clone()));
            json!({"found": true, "error": why})
        };
        let code = match join::read_code(&text, unix_now()) {
            Ok(code) => code,
            Err(why) => return refused(why),
        };
        match (self.joined(), code.kind()) {
            (true, Kind::Wants) => {
                let asked = Asked {
                    text: text.clone(),
                    name: code.name().to_string(),
                    address: code.address().map(|a| a.to_string()).unwrap_or_default(),
                };
                let said = json!({"found": true, "asked": {"name": asked.name, "address": asked.address}});
                *self.asked.lock().unwrap_or_else(|e| e.into_inner()) = Some(asked);
                said
            }
            (true, Kind::Gives) => refused(
                "This device is in a shared database already. That code is for a device that isn't in one yet."
                    .to_string(),
            ),
            (false, Kind::Wants) => refused(
                "That code is shown by a device that wants to join, as this one does. Read a code that a device of \
                 the shared database shows."
                    .to_string(),
            ),
            (false, Kind::Gives) => {
                if self.busy.swap(true, Ordering::SeqCst) {
                    return json!({"found": true, "busy": true});
                }
                let taken = join::ask(&code).and_then(|handed| keep(&self.home, &handed).map(|_| handed));
                self.busy.store(false, Ordering::SeqCst);
                match taken {
                    Ok(handed) => {
                        println!(
                            "This device joined a shared database, with the login{} another device handed it.",
                            if handed.passphrase.is_some() { " and the passphrase for keys" } else { "" }
                        );
                        json!({"found": true, "joined": true})
                    }
                    Err(why) => {
                        eprintln!("Joining failed: {why}");
                        refused(why)
                    }
                }
            }
        }
    }

    /// The owner said yes: hand what this device has to the new device whose code it read.
    pub fn give(&self, name: &str) -> Result<Value, String> {
        let settings = Settings::load(&self.home)?.ok_or("This device has no shared database to hand on.")?;
        let (text, shown_as) = {
            let asked = self.asked.lock().unwrap_or_else(|e| e.into_inner());
            let asked = asked.as_ref().ok_or("No new device's code was read: read it again.")?;
            (asked.text.clone(), asked.name.clone())
        };
        let code = join::read_code(&text, unix_now())?;
        self.allowed()?;
        let what = handover(&settings, &self.home, name);
        let given = join::give(&code, &what, (&settings.host, settings.port));
        // the code is used, or no longer good: either way it is asked about once, also while the camera
        // still sees it on the other device's screen
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).take();
        let seen_again = match &given {
            Ok(()) => "That code was used: the device that shows it was handed the login.".to_string(),
            Err(why) => why.clone(),
        };
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some((text, seen_again));
        given?;
        let keys = what.passphrase.is_some();
        println!(
            "Join code: this device read a new device's code and handed it the shared database's login{}.",
            if keys { " and the passphrase for keys" } else { "" }
        );
        Ok(json!({"given": true, "name": shown_as, "passphrase": keys}))
    }

    /// Show a new join code (the one before it ends) and listen for the device that reads it. A device of a
    /// shared database shows one that hands on what it has; a device without one shows one that asks for it.
    pub fn show(&self, name: &str) -> Result<Value, String> {
        self.end_shown();
        let (stop, ended) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(None)));
        let finish = {
            let ended = ended.clone();
            move |how: Ended| *ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(how)
        };
        let (kind, text, left) = match Settings::load(&self.home)? {
            Some(settings) => {
                self.allowed()?;
                let offer = Offer::open(join::PORT, (&settings.host, settings.port), name)?;
                let (text, left) = (offer.code(), offer.left());
                let (what, stop) = (handover(&settings, &self.home, name), stop.clone());
                let serve = move || {
                    let o = offer.serve(&what, &stop);
                    println!("{}", log_line(&o, what.passphrase.is_some()));
                    finish((word(&o), None));
                };
                std::thread::Builder::new().name("join code".into()).spawn(serve).map_err(|e| e.to_string())?;
                ("give", text, left)
            }
            None => {
                let want = Want::open(join::PORT, name)?;
                let (text, left) = (want.code(), want.left());
                let (home, stop, joined_now) = (self.home.clone(), stop.clone(), self.joined_now.clone());
                let receive = move || match want.receive(&stop) {
                    Received::Taken(handed, from) => match keep(&home, &handed) {
                        Ok(()) => {
                            println!(
                                "This device joined a shared database: a device at {from} read its code and handed \
                                 it the login{}.",
                                if handed.passphrase.is_some() { " and the passphrase for keys" } else { "" }
                            );
                            joined_now.store(true, Ordering::SeqCst);
                            finish(("joined", None));
                        }
                        Err(why) => {
                            eprintln!("Joining failed: {why}");
                            finish(("failed", Some(why)));
                        }
                    },
                    Received::Expired => finish(("expired", None)),
                    Received::Refused => finish(("refused", None)),
                    Received::Stopped => finish(("stopped", None)),
                };
                std::thread::Builder::new().name("join code".into()).spawn(receive).map_err(|e| e.to_string())?;
                ("want", text, left)
            }
        };
        let shown = Shown { svg: svg(&text)?, kind, until: Instant::now() + Duration::from_secs(left), stop, ended };
        println!("Join code: shown for {left} s, for one device.");
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = Some(shown);
        Ok(json!({"left": left, "kind": kind}))
    }

    /// Stop showing: the code is good for nothing from now on. A new device's code that was read and not
    /// answered is forgotten too.
    pub fn hide(&self) {
        self.end_shown();
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    fn end_shown(&self) {
        if let Some(s) = self.shown.lock().unwrap_or_else(|e| e.into_inner()).take() {
            s.stop.store(true, Ordering::Relaxed);
        }
    }

    /// The code this device shows now, as a picture.
    pub fn picture(&self) -> Option<String> {
        let shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
        let s = shown.as_ref()?;
        s.ended.lock().unwrap_or_else(|e| e.into_inner()).is_none().then(|| s.svg.clone())
    }
}

// ---- face-server pair: join codes shown by a computer, from the command line -------------------------------

#[derive(Default)]
struct Board {
    svg: Option<String>,
    /// what the page says under (or instead of) the code
    line: String,
    until: Option<Instant>,
    /// counts the codes shown, so that a new one is never taken from the browser's memory
    shown: u32,
}

struct PairPage(Mutex<Board>);

impl Service for PairPage {
    fn body_limit(&self, _: &str) -> Option<usize> {
        None
    }

    fn get(&self, req: &Request) -> Response {
        let board = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match req.path.as_str() {
            "/" => {
                let code = match &board.svg {
                    Some(_) => format!(r#"<img src="/api/code.svg?{}" alt="The join code">"#, board.shown),
                    None => String::new(),
                };
                let left = match board.until {
                    Some(t) if board.svg.is_some() => {
                        format!(" Good for {} more seconds.", t.saturating_duration_since(Instant::now()).as_secs())
                    }
                    _ => String::new(),
                };
                Response::bytes(
                    "text/html; charset=utf-8",
                    PAIR_PAGE
                        .replace("{{CODE}}", &code)
                        .replace("{{LINE}}", &format!("{}{left}", board.line))
                        .into_bytes(),
                )
            }
            "/api/code.svg" => match &board.svg {
                Some(svg) => Response::bytes("image/svg+xml", svg.clone().into_bytes()),
                None => Response::not_found(),
            },
            _ => Response::not_found(),
        }
    }

    fn post(&self, _: &Request) -> Response {
        Response::not_found()
    }
}

/// Nothing in it comes from outside this program: the code's picture and lines this file writes.
const PAIR_PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta http-equiv="refresh" content="2">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Face Tracker: add a device</title>
<style>
  body { margin: 0; background: #0f1115; color: #e8eaf0; font: 17px/1.45 system-ui, "Segoe UI", sans-serif; }
  main { max-width: 560px; margin: 0 auto; padding: 28px 20px; text-align: center; }
  h1 { font-size: 24px; margin: 0 0 8px; }
  p { color: #9aa3b2; margin: 8px 0; }
  img { width: 100%; max-width: 460px; background: #fff; border-radius: 14px; margin: 14px 0; }
  .line { color: #e8eaf0; font-weight: 600; }
</style></head>
<body><main>
<h1>Add a device</h1>
<p>On the new device open Face Tracker, choose Shared database, and point its camera at this code.</p>
{{CODE}}
<p class="line">{{LINE}}</p>
<p>A code works once, for two minutes, and only for a device on this network that sees this screen.</p>
</main></body></html>
"#;

/// Show join codes on this computer's screen, one device after another: a page on 127.0.0.1 with the code,
/// and this computer listening on the local network for the device that reads it. A code that nobody read
/// in its two minutes is followed by a new one, for `minutes` in all; then, or once `devices` were handed
/// the login, it ends. `home` is the private folder whose saved login is handed on (read, never written).
pub fn pair(home: &Path, devices: u32, minutes: u64) -> Result<(), String> {
    let settings = Settings::load(home)?.ok_or("This computer has no shared database to hand on.")?;
    let name = crate::device().0;
    let listener = http::bind(0).map_err(|e| format!("No port for the page: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let page = Arc::new(PairPage(Mutex::new(Board { line: "Making a code…".to_string(), ..Board::default() })));
    let served = page.clone();
    std::thread::Builder::new()
        .name("join codes' page".into())
        .spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                let _ = rt.block_on(http::serve(listener, served));
            }
        })
        .map_err(|e| e.to_string())?;
    println!("Join codes: http://127.0.0.1:{port}/");
    let set = |svg: Option<String>, line: String, until: Option<Instant>| {
        let mut b = page.0.lock().unwrap_or_else(|e| e.into_inner());
        b.shown += 1;
        (b.svg, b.line, b.until) = (svg, line, until);
    };
    let (stop, end, mut given) = (AtomicBool::new(false), Instant::now() + Duration::from_secs(minutes * 60), 0);
    let last = loop {
        if given == devices {
            break format!("Done: {given} device(s) were handed the login. This page can be closed.");
        }
        if Instant::now() >= end {
            break format!(
                "No more codes: {given} of {devices} device(s) were handed the login. Start again for more."
            );
        }
        let offer = Offer::open(join::PORT, (&settings.host, settings.port), &name)?;
        let what = handover(&settings, home, &name);
        let until = Instant::now() + Duration::from_secs(offer.left());
        set(Some(svg(&offer.code())?), format!("For device {} of {devices}.", given + 1), Some(until));
        println!("Join code for device {} of {devices}: shown.", given + 1);
        let outcome = offer.serve(&what, &stop);
        println!("{}", log_line(&outcome, what.passphrase.is_some()));
        match outcome {
            Outcome::Given(_) => given += 1,
            Outcome::Expired => {} // a new code
            _ => break "The codes were ended: something asked with a wrong key several times.".to_string(),
        }
    };
    println!("{last}");
    set(None, last, None);
    std::thread::sleep(Duration::from_secs(6)); // the page's last line, for a browser that asks every 2 s
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code as a camera would see it on a screen: `scale` pixels a square, grey around it.
    fn photographed(text: &str, scale: usize) -> Bgr {
        let (dark, n) = modules(text).unwrap();
        let (side, pad) = ((n + 2 * QUIET) * scale, 40);
        let (w, h) = (side + 2 * pad, side + 2 * pad + 30);
        let mut px = vec![120u8; w * h * 3];
        for y in 0..side {
            for x in 0..side {
                let (mx, my) = (x / scale, y / scale);
                let inside = mx >= QUIET && my >= QUIET && mx < n + QUIET && my < n + QUIET;
                let on = inside && dark[(my - QUIET) * n + (mx - QUIET)];
                let at = ((y + pad) * w + x + pad) * 3;
                px[at..at + 3].fill(if on { 20 } else { 235 });
            }
        }
        Bgr { w, h, px }
    }

    #[test]
    fn a_shown_code_is_read_back_from_a_picture() {
        // the longest code this app shows: a name of forty letters that aren't plain ones, four addresses
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
        let name = B64URL.encode("é".repeat(join::NAME_MAX));
        let text = format!(
            "{}2:w:AAAAAAAAAAAAAAAAAAAAAA:8770:1900000000:{name}:192.168.1.20,100.64.3.1,fd00::1234:5678:9abc:def0,10.20.30.40",
            join::PREFIX
        );
        assert_eq!(join::read_code(&text, 1_900_000_000 - 60).unwrap().name().chars().count(), join::NAME_MAX);
        for scale in [3, 6] {
            assert_eq!(read(&photographed(&text, scale)).as_deref(), Some(text.as_str()), "{scale} pixels a square");
        }
        // a picture without a code, and a code that is not a join code
        assert_eq!(read(&Bgr { w: 64, h: 48, px: vec![90; 64 * 48 * 3] }), None);
        assert_eq!(read(&photographed("https://example.com/some/page", 6)), None);
    }

    #[test]
    fn the_picture_of_a_code_is_only_squares() {
        let picture =
            svg(&format!("{}2:g:AAAAAAAAAAAAAAAAAAAAAA:8770:1900000000::192.168.1.20", join::PREFIX)).unwrap();
        assert!(picture.starts_with("<svg ") && picture.ends_with("</svg>"));
        assert!(!picture.contains("192.168") && !picture.contains("script"), "the text is not written into it");
        let path = picture.split(" d=\"").nth(1).unwrap().split('"').next().unwrap();
        assert!(path.chars().all(|c| c.is_ascii_digit() || "Mhvz -".contains(c)), "{path}");
    }

    fn a_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("face-join-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A private folder of a device that is in a shared database (whose server nothing here asks).
    fn a_home_in_a_database(name: &str) -> PathBuf {
        let dir = a_home(name);
        let password = face_files::protect::protect("the database's password").unwrap();
        let settings = json!({"host": "127.0.0.1", "port": 9, "dbname": "people", "user": "app",
                              "password": password, "schema": "facerec"});
        std::fs::write(dir.join("database.json"), settings.to_string()).unwrap();
        dir
    }

    fn now() -> u64 {
        unix_now()
    }

    /// A code as a device shows it, for a listener on this computer.
    fn code_text(kind: &str, port: u16, name: &str) -> String {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
        format!(
            "{}2:{kind}:AAAAAAAAAAAAAAAAAAAAAA:{port}:{}:{}:127.0.0.1",
            join::PREFIX,
            now() + 100,
            B64URL.encode(name)
        )
    }

    #[test]
    fn a_device_without_a_shared_database_takes_only_a_code_that_hands_one_on() {
        let dir = a_home("new");
        let j = Joining::new(&dir);
        assert!(!j.joined() && !j.joined_now());
        assert_eq!(j.picture(), None);
        assert_eq!(
            j.state("a phone"),
            json!({"joined": false, "name": "a phone", "showing": null, "last": null, "error": null, "asked": null})
        );
        // a text that is no join code is refused, and the same text is not tried twice
        let said = j.code("facetracker-join:1:nothing");
        assert!(said["error"].as_str().unwrap().contains("not a join code"), "{said}");
        assert_eq!(j.code(" facetracker-join:1:nothing\n"), said);
        // another new device's code: this device has nothing to hand it, and asks nobody
        let said = j.code(&code_text("w", 8770, "Another new one"));
        assert!(said["error"].as_str().unwrap().contains("wants to join, as this one does"), "{said}");
        assert_eq!(j.state("a phone")["asked"], Value::Null);
        assert!(j.give("a phone").unwrap_err().contains("no shared database"));
        // a code that hands a database on, whose device isn't there: said once, and nothing is kept
        let said = j.code(&code_text("g", 9, "Gone"));
        assert!(said["error"].as_str().unwrap().contains("can't be reached"), "{said}");
        assert!(!j.joined() && face_home::passphrase::received(&dir).is_none());
        let mut jpeg = Vec::new();
        assert!(j.frame(&jpeg).is_err(), "not a picture");
        jpeg.extend_from_slice(b"not a jpeg");
        assert!(j.frame(&jpeg).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_device_of_the_database_hands_over_only_after_its_owner_said_yes() {
        let dir = a_home_in_a_database("member");
        face_home::passphrase::receive(&dir, "the owner's own passphrase").unwrap();
        let j = Joining::new(&dir);
        assert!(j.joined());
        // a code of another device of a database is nothing to this one
        let said = j.code(&code_text("g", 8770, "Another member"));
        assert!(said["error"].as_str().unwrap().contains("already"), "{said}");
        assert!(j.give("Desk").unwrap_err().contains("No new device's code was read"));

        // a new device that shows a code, on this computer
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let want = Want::on(listener, vec!["127.0.0.1".parse().unwrap()], "New <b>laptop</b>").unwrap();
        let text = want.code();
        let receiving = std::thread::spawn(move || want.receive(&AtomicBool::new(false)));
        // reading it hands nothing over: the owner is asked, with the name the code says and where it is
        let said = j.code(&text);
        assert_eq!(said, json!({"found": true, "asked": {"name": "New <b>laptop</b>", "address": "127.0.0.1"}}));
        assert_eq!(j.state("Desk")["asked"], json!({"name": "New <b>laptop</b>", "address": "127.0.0.1"}));
        assert!(!receiving.is_finished());

        // the system's own check says no: nothing is handed over, and the code is still there to say yes to
        j.check_with(Arc::new(|_| Checked::No("It wasn't you.".to_string())));
        assert_eq!(j.give("Desk").unwrap_err(), "It wasn't you.");
        assert!(j.show("Desk").unwrap_err().contains("It wasn't you."), "a code is not shown either");
        assert!(!receiving.is_finished() && j.state("Desk")["asked"].is_object());

        // it says yes (or this device has no such check): the new device gets the login and the passphrase
        j.check_with(Arc::new(|why| {
            assert!(why.contains("new device"), "{why}");
            Checked::Yes
        }));
        let said = j.give("Desk").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(said, json!({"given": true, "name": "New <b>laptop</b>", "passphrase": true}));
        let Received::Taken(handed, _) = receiving.join().unwrap() else { panic!("the new device took nothing") };
        assert_eq!((handed.passphrase.as_deref(), handed.from.as_str()), (Some("the owner's own passphrase"), "Desk"));
        assert_eq!(j.state("Desk")["asked"], Value::Null);
        // once: the code is used, and the camera seeing it again on the other screen asks nothing again
        assert!(j.give("Desk").unwrap_err().contains("No new device's code was read"));
        let said = j.code(&text);
        assert!(said["error"].as_str().unwrap().contains("That code was used"), "{said}");
        assert_eq!(j.state("Desk")["asked"], Value::Null);

        // a code that was read and left unanswered is forgotten when the page is left
        let said = j.code(&code_text("w", 9, "Someone"));
        assert!(said["asked"].is_object(), "{said}");
        j.hide();
        assert_eq!(j.state("Desk")["asked"], Value::Null);
        // and one whose device is gone is asked about once
        j.check_with(Arc::new(|_| Checked::Unavailable));
        assert!(j.code(&code_text("w", 9, "Gone"))["asked"].is_object());
        assert!(j.give("Desk").unwrap_err().contains("can't be reached"));
        assert_eq!(j.state("Desk")["asked"], Value::Null);
        let _ = std::fs::remove_dir_all(dir);
    }
}
