//! The app itself: the tab page's own server and
//! the two programs it starts.
//!
//! One process serves the tab page (`app_ui.html`, `app_ui.js`: the Face tracking, Labeling, Cameras and
//! Settings tabs) on 127.0.0.1 and runs the labeling tool; the tracker runs as a second process, this
//! program again as `tracker-child`, so that neither can hold up the other and a tracker that hangs or
//! ends can be started again without the rest. Each program is on the first free port from its own, and a
//! program already answering there that this process didn't start is never shown in the window.
//!
//! The tab page's routes are GETs: `/status` (where each program serves, or why it doesn't), `/paths` and
//! `/open` (the two data folders, and only those), `/update` and `/open-release` (GitHub, only on the
//! owner's click), `/attention`, `/alive` (the page says every 2 s that it runs, and which tabs don't),
//! `/restart` (Start again for a program) and `/restart-app`. Those that do something need the page's own
//! header, and everything but the page's files goes to the page itself only.
//!
//! What answers on the tracker's port is taken as the tracker only when it proves that it is the process
//! this app started (`trackerlink::hello`: this run's secret, which the app makes and its child inherits).
//!
//! With a window (`apps/desktop`), the app also watches it: the tab page says every 2 s that it runs; after
//! 30 s of silence, and 30 s more, the whole app is started again in place (a new copy that waits for this
//! one to end before it takes the ports), at most every 2, 4, 8 ... minutes while that keeps happening. A
//! camera that won't start even after a page reload starts the app again once an hour at most, and under a
//! locked screen once per lock. `HOME/restarted.json` keeps both records.
//!
//! Not here yet: the first-run question asked in the window.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};

use crate::{
    desktop, guard,
    http::{self, Request, Response, Service},
    trackerlink,
};

pub const TITLE: &str = "Face Recognizer";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "SingularityResearch/facial-recognition";
/// each program runs on the first free port from its own, of this many
pub const PORT_TRIES: u16 = 10;
/// s the tracker's process may take to load its models and answer
const CHILD_START: Duration = Duration::from_secs(300);
/// s without an answer before a running tracker's process is ended and started again
const CHILD_SILENT: Duration = Duration::from_secs(30);
/// at most this many starts in this many seconds; then its tab says so instead of trying forever
const CHILD_TRIES: (usize, u64) = (5, 600);
/// s between looks at the tracker's process
const CHILD_LOOK: Duration = Duration::from_secs(5);
/// the tracker's process's exit code when its port was taken between this process's look and its own
pub const PORT_TAKEN: i32 = 3;
/// s of silence from the window's page before it counts as hung, and s more before the app is started again
const HUNG_AFTER: f64 = 30.0;
const RESTART_AFTER: f64 = 30.0;
/// camera restarts: at most once in this many seconds
const RESTART_EVERY: f64 = 3600.0;
/// s: window restarts in a row wait 2, 4, 8 ... minutes, at most an hour
const WINDOW_BACKOFF: f64 = 120.0;
const NAMES: [(&str, &str); 2] = [("track", "Face tracker"), ("label", "Face Labels")];
const TABS: [(&str, &str); 3] = [("track", "Face tracking"), ("label", "Labeling"), ("cams", "Cameras")];
const HTML: &str = "text/html; charset=utf-8";
const SCRIPT: &str = "text/javascript; charset=utf-8";
/// the tab page's files
pub(crate) const PAGES: [(&str, &str, &str); 3] =
    [("/", "app_ui.html", HTML), ("/app_ui.js", "app_ui.js", SCRIPT), ("/ask.js", "ask.js", SCRIPT)];
/// what does something (contacts GitHub, opens a browser or a folder, starts a program): only with the
/// header that this page alone sends
const ACTS: [&str; 6] = ["/update", "/open-release", "/open", "/attention", "/restart", "/restart-app"];
const ALREADY: &str = "started on its own (its .bat) or by another Face Recognizer window, and a copy started that \
way can't be shown in this window. \nClose that copy (its black window) and start the app again, or keep using it \
in your browser at";

/// what a second copy of the app is told, in its log: it started nothing
const RUNS_ALREADY: &str = "Face Recognizer is already running for this user on this computer: its window is the \
one to use. This second copy has started nothing and ends here.";
/// how long a copy waits for the place of the one running copy: a copy that started this one to take its
/// place ends within it; one that goes on running still holds the place after it
const ONLY_WAIT: Duration = Duration::from_secs(10);
/// this process's place as the one running copy of the app, held until the process ends
static ONLY: Mutex<Option<face_files::paths::OnlyCopy>> = Mutex::new(None);

/// Take this process's place as the one running copy of the app for a private folder, or say that another
/// copy has it. Every way the app starts goes through here before it starts anything: the ordinary start,
/// and the start that first asks about what is missing (which begins the programs later, by another way).
fn take_place(home: &Path) -> Result<(), String> {
    let mut held = ONLY.lock().unwrap_or_else(|e| e.into_inner());
    if held.is_some() {
        return Ok(()); // (this process has it already)
    }
    match face_files::paths::only_copy(home, "running", ONLY_WAIT) {
        Some(only) => {
            *held = Some(only);
            Ok(())
        }
        None => Err(RUNS_ALREADY.to_string()),
    }
}

/// The app's ports: the tab page's, the labeling tool's, the tracker's and the Cameras page's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ports {
    pub shell: u16,
    pub label: u16,
    pub track: u16,
    pub cams: u16,
}

/// 8764, 8765, 8766 and 8769, or from `FACE_PORTS=<n>` (n, n+1, n+2, n+5), so that a second copy with other
/// folders runs beside the usual one.
pub fn ports(said: Option<&str>) -> Result<Ports, String> {
    let raw = said.unwrap_or("").trim();
    let base: u32 =
        if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) { raw.parse().unwrap_or(0) } else { 8764 };
    if !(1024..=65000).contains(&base) {
        return Err(format!("FACE_PORTS must be between 1024 and 65000, not {raw}"));
    }
    let base = base as u16;
    Ok(Ports { shell: base, label: base + 1, track: base + 2, cams: base + 5 })
}

fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether one of the programs (`app`: "face-labels" or "tracker") is serving on a port.
fn answering(port: u16, app: &str) -> bool {
    let path = if app == "face-labels" { "/api/summary" } else { "/api/people" };
    let said = trackerlink::ask(port, path, None, Duration::from_secs(3), false)
        .and_then(|(_, body)| serde_json::from_slice::<Value>(&body).ok())
        .unwrap_or(Value::Null);
    if app == "face-labels" { said["app"] == "face-labels" } else { said.get("people").is_some() }
}

fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

fn port_of(url: &str) -> Option<u16> {
    url.trim_end_matches('/').rsplit_once(':')?.1.parse().ok()
}

/// What the tab page and the programs' starters share.
pub struct App {
    root: PathBuf,
    home: PathBuf,
    pages: PathBuf,
    ports: Ports,
    /// the labeling tool keeps the library and the shared database the same
    sync: bool,
    /// {"track": {...}, "label": {...}}: {"url"} once serving, {"error"} if it failed, {} while it starts
    status: Mutex<Map<String, Value>>,
    /// the tab page's own port, once its server is up
    shell: Mutex<u16>,
    /// the tracker's process as this app started it, and when it was started (for the limit)
    tracker: Mutex<Option<Child>>,
    starts: Mutex<Vec<Instant>>,
    /// where the tracker last served: started again, it takes the port its page is loaded from
    last: Mutex<Option<u16>>,
    /// the tracker's port while its process is being started again: its page is still the one this app showed
    again: Mutex<Option<u16>>,
    /// the labeling tool runs in this process on this port
    label: Mutex<Option<u16>>,
    /// one Start again at a time
    restarting: Mutex<()>,
    /// when the tab page last said it runs, and the tabs it named as not answering
    alive: Mutex<(Option<Instant>, Vec<String>)>,
    /// the app's window, once there is one: what the app asks of it
    window: Mutex<Option<Arc<Window>>>,
    /// the first-run question of this start, when there is one: nothing else is shown or started before
    /// it is answered
    #[cfg(feature = "fetch")]
    first: Mutex<Option<Arc<crate::firstrun::FirstRun>>>,
    /// the programs were started (once)
    begun: std::sync::atomic::AtomicBool,
}

/// What the app asks of its window's program.
pub struct Window {
    /// new alerts: how many (for the title, which shows on the taskbar button), and whether to draw the eye
    pub attention: Box<dyn Fn(u64, bool) + Send + Sync>,
    /// before this process ends for a new copy: what is still to be written is written
    pub before_exit: Box<dyn Fn() + Send + Sync>,
}

/// What the window's watchdog keeps between its looks.
#[derive(Default)]
pub struct Watch {
    tick: Option<f64>,
    hung_since: Option<f64>,
    held: bool,
}

/// The watchdog's decision at one look.
#[derive(Debug, PartialEq)]
pub enum Step {
    Nothing,
    /// the computer slept: start counting again
    Reset,
    /// stop the page's own process so that it loads again (where the window's browser can: not on Windows)
    Kill,
    /// start the app again
    Restart,
}

/// The watchdog's decision at time `now` (seconds on a clock that never jumps): the page said it runs at
/// `alive_at`. `kills`: this system's window can have its page's process stopped first.
pub fn watch_step(st: &mut Watch, now: f64, alive_at: Option<f64>, kills: bool) -> Step {
    let slept = st.tick.is_some_and(|tick| now - tick > 20.0);
    st.tick = Some(now);
    if slept {
        st.hung_since = None;
        return Step::Reset;
    }
    let Some(alive_at) = alive_at.filter(|at| now - at >= HUNG_AFTER) else {
        st.hung_since = None;
        return Step::Nothing;
    };
    let _ = alive_at;
    match st.hung_since {
        None => {
            st.hung_since = Some(now);
            if kills { Step::Kill } else { Step::Nothing }
        }
        Some(since) if now - since >= RESTART_AFTER => {
            st.hung_since = None;
            Step::Restart
        }
        Some(_) => Step::Nothing,
    }
}

/// (due, n) for the watchdog's restart: the first at once; while they keep coming, each waits twice as long
/// as the one before (2, 4, 8 ... minutes, at most an hour), so that a page that always hangs can't keep the
/// app restarting every minute; an hour without one starts over. n: restarts in the current run of them.
pub fn window_restart_due(rec: &Value, now: f64) -> (bool, u32) {
    let at = rec.get("at").and_then(Value::as_f64).unwrap_or(0.0);
    let mut n = rec.get("n").and_then(Value::as_u64).unwrap_or(0) as u32;
    if now - at >= RESTART_EVERY {
        n = 0;
    }
    let wait = if n == 0 { 0.0 } else { (WINDOW_BACKOFF * 2f64.powi(n as i32 - 1)).min(RESTART_EVERY) };
    (now - at >= wait, n)
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// A copy started by `restart_app` on Windows: wait (up to 30 s) until the copy that started it has ended,
/// so that its ports are free. Call it first thing.
pub fn wait_for_previous() {
    let Some(pid) = std::env::var("FACE_RESTART_AFTER").ok().and_then(|p| p.parse::<u32>().ok()) else { return };
    // SAFETY: only this thread runs yet
    unsafe { std::env::remove_var("FACE_RESTART_AFTER") };
    let began = Instant::now();
    while !gone(pid) && began.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500)); // its window's helper processes end with it
}

impl App {
    pub fn new(root: &Path, home: &Path, pages: &Path, ports: Ports, sync: bool) -> Arc<App> {
        trackerlink::new_token(); // this run's secret: in the tracker's process's environment, nowhere else
        let status = [("track", json!({})), ("label", json!({}))].map(|(k, v)| (k.to_string(), v));
        Arc::new(App {
            root: root.to_path_buf(),
            home: home.to_path_buf(),
            pages: pages.to_path_buf(),
            ports,
            sync,
            status: Mutex::new(status.into_iter().collect()),
            shell: Mutex::new(0),
            tracker: Mutex::new(None),
            starts: Mutex::new(Vec::new()),
            last: Mutex::new(None),
            again: Mutex::new(None),
            label: Mutex::new(None),
            restarting: Mutex::new(()),
            alive: Mutex::new((None, Vec::new())),
            window: Mutex::new(None),
            #[cfg(feature = "fetch")]
            first: Mutex::new(None),
            begun: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Start the two programs and what the app keeps up beside them. Once: at the app's start, or when
    /// the first-run question is answered.
    fn begin(self: &Arc<Self>) {
        if self.begun.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.run_programs(&["track", "label"]);
        // LM Studio here: kept serving on all interfaces and announced (every minute; FACE_LMSTUDIO=off: never)
        #[cfg(feature = "import")]
        crate::keep_lmstudio(&self.home);
    }

    fn set(&self, key: &str, value: Value) {
        held(&self.status).insert(key.to_string(), value);
    }

    fn state(&self, key: &str) -> Value {
        held(&self.status).get(key).cloned().unwrap_or_else(|| json!({}))
    }

    fn fail(&self, key: &str, name: &str, why: &str) {
        println!("{name} couldn't start: {why}");
        let log = self.home.join("app.log");
        self.set(key, json!({"error": format!("{name} couldn't start.\n\n{why}\n\nDetails are in {}", log.display())}));
    }

    /// The tab page itself, on 127.0.0.1: its own port (fixed, so that the page's remembered tab survives a
    /// restart), or any free one when that is taken. Returns its address. The programs started next trust
    /// this page to frame them.
    pub fn serve_shell(self: &Arc<Self>) -> Result<String, String> {
        let listener = http::bind(self.ports.shell).or_else(|_| http::bind(0)).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        *held(&self.shell) = port;
        guard::allow_frame_parents(port);
        let shell = Arc::new(Shell::new(self.clone()));
        std::thread::Builder::new()
            .name("shell".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => return eprintln!("The tab page's server didn't start: {e}"),
                };
                if let Err(e) = rt.block_on(http::serve(listener, shell)) {
                    eprintln!("The tab page's server stopped: {e}");
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(url(port))
    }

    /// Start the programs (both at first; one again from `restart`), each on a thread of its own: the
    /// status gets {"url"} or {"error"}.
    pub fn run_programs(self: &Arc<Self>, keys: &[&str]) {
        for key in keys {
            let (app, key) = (self.clone(), key.to_string());
            let _ = std::thread::Builder::new().name(format!("starting {key}")).spawn(move || {
                let name = NAMES.iter().find(|n| n.0 == key).map_or("", |n| n.1);
                let started = match key.as_str() {
                    "track" => {
                        let started = app.start_tracker(name);
                        *held(&app.again) = None;
                        started
                    }
                    _ => app.start_label(name).map(|at| json!({"url": at})),
                };
                match started {
                    Ok(at) => app.set(&key, at),
                    Err(why) => app.fail(&key, name, &why),
                }
            });
        }
    }

    /// The labeling tool in this process, on the first free port from its own.
    fn start_label(&self, name: &str) -> Result<String, String> {
        for port in self.ports.label..self.ports.label + PORT_TRIES {
            match http::bind(port) {
                Ok(free) => drop(free),
                Err(_) if answering(port, "face-labels") => {
                    if *held(&self.label) == Some(port) {
                        println!("{name}: still serving at {}", url(port));
                        return Ok(url(port));
                    }
                    return Err(format!("It is already running at {}, {ALREADY} {}", url(port), url(port)));
                }
                Err(_) => continue,
            }
            println!("{name}: loading ...");
            crate::start_labeler(&self.root, &self.home, &self.pages, port, self.sync)?;
            *held(&self.label) = Some(port);
            trackerlink::set_own_label(port); // the tracker's process is told where to ask for this device's photos
            println!("{name}: serving at {}", url(port));
            return Ok(url(port));
        }
        Err(format!("No free port between {} and {}", self.ports.label, self.ports.label + PORT_TRIES - 1))
    }

    /// May the tracker's process be started (again) now? Not more than `CHILD_TRIES.0` times in
    /// `CHILD_TRIES.1` s.
    fn tracker_start_due(&self) -> bool {
        let mut starts = held(&self.starts);
        starts.retain(|t| t.elapsed().as_secs() < CHILD_TRIES.1);
        if starts.len() >= CHILD_TRIES.0 {
            return false;
        }
        starts.push(Instant::now());
        true
    }

    /// The tracker as a process of its own, on the first free port from its own (started again: from the
    /// port its page is loaded from). Returns its state for the tab page ({"url"}, and {"cameras"} when the
    /// Cameras page has a port of its own there) once it answers and has shown that it is the process this
    /// app started.
    fn start_tracker(self: &Arc<Self>, name: &str) -> Result<Value, String> {
        if !self.tracker_start_due() {
            return Err(format!(
                "its process was started {} times in {} minutes and ended or stopped answering each time",
                CHILD_TRIES.0,
                CHILD_TRIES.1 / 60
            ));
        }
        let first = held(&self.last).unwrap_or(self.ports.track);
        let me = std::env::current_exe().map_err(|e| format!("this program's own file isn't known: {e}"))?;
        for port in first..first.saturating_add(PORT_TRIES) {
            match http::bind(port) {
                Ok(free) => drop(free), // free now: the child takes it next
                Err(_) if answering(port, "tracker") => {
                    return Err(format!("It is already running at {}, {ALREADY} {}", url(port), url(port)));
                }
                Err(_) => continue,
            }
            let mut child = Command::new(&me)
                .args(["tracker-child", &port.to_string(), &std::process::id().to_string()])
                .args([held(&self.shell).to_string(), self.ports.cams.to_string()])
                // (the same page files, and the same two folders whatever this program took them from)
                .args(Some("--pages").filter(|_| !self.pages.as_os_str().is_empty()))
                .args(Some(&self.pages).filter(|pages| !pages.as_os_str().is_empty()))
                .env("FACE_ROOT", &self.root)
                .env("FACE_HOME", &self.home)
                .env("FACE_LINK", trackerlink::token())
                .stdin(Stdio::null())
                .spawn()
                .map_err(|e| format!("its process couldn't be started: {e}"))?;
            let began = Instant::now();
            let ended = loop {
                if let Ok(Some(code)) = child.try_wait() {
                    break Some(code.code().unwrap_or(1));
                }
                if trackerlink::hello(port, Duration::from_secs(2)) {
                    break None;
                }
                if began.elapsed() > CHILD_START {
                    end_process(&mut child);
                    return Err(format!("its process didn't answer in {} s", CHILD_START.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(500));
            };
            match ended {
                None => {
                    trackerlink::set_tracker(Some(port));
                    // the Cameras page on a port of its own, when the tracker's process could take it
                    let cameras = trackerlink::cameras_port();
                    let also = cameras.map_or_else(String::new, |p| format!("; the Cameras page on port {p}"));
                    println!("{name}: serving at {} (a process of its own, {}{also})", url(port), child.id());
                    *held(&self.tracker) = Some(child);
                    *held(&self.last) = Some(port);
                    let app = self.clone();
                    let _ = std::thread::Builder::new()
                        .name("tracker's process".into())
                        .spawn(move || app.watch_tracker(port));
                    return Ok(match cameras {
                        Some(p) => json!({"url": url(port), "cameras": format!("http://127.0.0.1:{p}/cameras")}),
                        None => json!({"url": url(port)}),
                    });
                }
                Some(PORT_TAKEN) => continue,
                Some(code) => return Err(format!("its process ended while it started (exit {code})")),
            }
        }
        Err(format!("No free port between {first} and {}", first.saturating_add(PORT_TRIES - 1)))
    }

    /// End the tracker's process (the app ends, or Start again for a tracker that hangs).
    pub fn stop_tracker(&self) {
        if let Some(mut child) = held(&self.tracker).take() {
            trackerlink::set_tracker(None);
            end_process(&mut child);
        }
    }

    /// While this is the tracker's process: start it again when it has ended, or when it runs and hasn't
    /// answered for `CHILD_SILENT`.
    fn watch_tracker(self: &Arc<Self>, port: u16) {
        let (mine, mut heard) = (held(&self.tracker).as_ref().map(Child::id), Instant::now());
        loop {
            std::thread::sleep(CHILD_LOOK);
            let running = {
                let mut tracker = held(&self.tracker);
                match tracker.as_mut() {
                    Some(child) if Some(child.id()) == mine => child.try_wait().ok().flatten().is_none(),
                    _ => return, // stopped, or another one started since
                }
            };
            if running && trackerlink::hello(port, Duration::from_secs(3)) {
                heard = Instant::now();
                continue;
            }
            if running && heard.elapsed() < CHILD_SILENT {
                continue;
            }
            let _one = held(&self.restarting);
            if held(&self.tracker).as_ref().map(Child::id) != mine {
                return;
            }
            let why = if running {
                format!("hasn't answered for {} s", heard.elapsed().as_secs())
            } else {
                "ended".to_string()
            };
            println!("Face tracker: its process {why}: starting it again.");
            *held(&self.again) = Some(port);
            self.stop_tracker();
            self.set("track", json!({}));
            self.run_programs(&["track"]);
            return;
        }
    }

    /// Start one program again (the window's Start again button): "running" (it answers: nothing to do),
    /// "starting" or "busy" (already starting).
    fn restart(self: &Arc<Self>, key: &str) -> &'static str {
        let _one = held(&self.restarting);
        let now = self.state(key);
        let (at, failed) = (now["url"].as_str().and_then(port_of), now.get("error").is_some());
        if at.is_none() && !failed {
            return "busy";
        }
        if at.is_some_and(|port| answering(port, if key == "track" { "tracker" } else { "face-labels" })) {
            return "running";
        }
        if key == "track" && at.is_some() {
            // a process of its own that doesn't answer can be ended and started again: nothing else lives in it
            println!("Face tracker: it doesn't answer: ending its process and starting it again");
            *held(&self.again) = at;
            self.stop_tracker();
            self.set(key, json!({}));
            self.run_programs(&[key]);
            return "starting";
        }
        if key == "label" && held(&self.label).is_some() {
            // it is loaded in this process and only slow: a second copy would write the same data files
            return "running";
        }
        // (a failed download stops both: the other one that failed starts again with it)
        let mut keys = vec![key];
        keys.extend(NAMES.iter().map(|n| n.0).filter(|k| *k != key && self.state(k).get("error").is_some()));
        for k in &keys {
            println!("{}: starting again", NAMES.iter().find(|n| n.0 == *k).map_or("", |n| n.1));
            self.set(k, json!({}));
        }
        self.run_programs(&keys);
        "starting"
    }

    /// Whether a request for the camera and microphone made in the name of `uri` is answered without a
    /// question: the tracker this process started, or this window's own tab page while that tracker is up
    /// (the window's browser says a framed page's request comes from the page around it; the tab page hands
    /// both to the tracker's frame alone, and its own scripts never use them). Not any other program on
    /// 127.0.0.1, and not a tracker this process didn't start. While the tracker's process is being started
    /// again its page is still the one this app showed, and is answered too.
    pub fn camera_allowed(&self, uri: &str) -> bool {
        let running = held(&self.tracker).is_some();
        let track = self.state("track")["url"].as_str().and_then(port_of).filter(|_| running).or(*held(&self.again));
        let Some(track) = track else { return false };
        let shell = *held(&self.shell);
        let asking = uri.strip_prefix("http://127.0.0.1:").and_then(|rest| rest.split('/').next()?.parse::<u16>().ok());
        asking.is_some_and(|port| port == track || (shell != 0 && port == shell))
    }

    /// Whether an address is one of this app's own pages: on 127.0.0.1, at the tab page's port or at a port
    /// where one of the programs this process started serves now (the tracker's also while its process is
    /// being started again, and the Cameras page's own). For a window whose web view asks about every frame.
    pub fn own_page(&self, url: &str) -> bool {
        let Some(asked) = url
            .strip_prefix("http://127.0.0.1:")
            .and_then(|rest| rest.split(['/', '?', '#']).next()?.parse::<u16>().ok())
        else {
            return false;
        };
        let shell = *held(&self.shell);
        let status = held(&self.status).clone();
        let mut own: Vec<u16> = status
            .values()
            .flat_map(|state| [state["url"].as_str(), state["cameras"].as_str()])
            .flatten()
            .filter_map(|url| url.strip_prefix("http://127.0.0.1:")?.split('/').next()?.parse().ok())
            .collect();
        own.extend(*held(&self.again));
        asked != 0 && (asked == shell || own.contains(&asked))
    }

    /// Whether the tab page has run in a window at all (it says so every 2 s once it does).
    pub fn page_ran(&self) -> bool {
        held(&self.alive).0.is_some()
    }

    /// The tracker's address without a slash at its end: where it serves now, else where it is started first.
    pub fn tracker_address(&self) -> Option<String> {
        let serving = self.state("track")["url"].as_str().map(|url| url.trim_end_matches('/').to_string());
        serving.or_else(|| Some(format!("http://127.0.0.1:{}", self.ports.track)))
    }

    /// The app has a window now: alerts go to its title, and it is watched (`watch_window`).
    pub fn set_window(&self, window: Window) {
        *held(&self.window) = Some(Arc::new(window));
    }

    fn window(&self) -> Option<Arc<Window>> {
        held(&self.window).clone()
    }

    /// `HOME/restarted.json`: {"camera": {at, why, locked}, "window": {at, n}} (older files held only the
    /// camera's).
    fn restarts(&self) -> Map<String, Value> {
        let kept: Option<Value> =
            std::fs::read(self.home.join("restarted.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
        match kept {
            Some(Value::Object(d)) if d.contains_key("at") => {
                [("camera".to_string(), Value::Object(d))].into_iter().collect()
            }
            Some(Value::Object(d)) => d,
            _ => Map::new(),
        }
    }

    fn note_restart(&self, kind: &str, rec: Value) -> Result<(), String> {
        let mut all = self.restarts();
        all.insert(kind.to_string(), rec);
        let text = face_files::filejson::dumps(&Value::Object(all));
        face_files::paths::atomic_write(&self.home.join("restarted.json"), text.as_bytes()).map_err(|e| e.to_string())
    }

    /// Start this app again. On macOS and Linux in place: this process becomes the new copy (the same
    /// process number; its ports are free the moment it does). Windows has no such thing: a new copy of this
    /// program is started with the same words, which waits for this process to be gone before it takes the
    /// ports (`wait_for_previous`); the same where it can't be done in place. The tracker's process ends
    /// first, so that its port and the peer service's are free. Does not return.
    pub fn restart_app(&self) -> ! {
        self.stop_tracker();
        #[cfg(unix)]
        if let Ok(me) = std::env::current_exe() {
            use std::os::unix::process::CommandExt;
            if let Some(window) = self.window() {
                (window.before_exit)();
            }
            // (the new copy makes its own secret for its own tracker)
            let failed = Command::new(me).args(std::env::args_os().skip(1)).env_remove("FACE_LINK").exec();
            println!("The app couldn't be started again in place ({failed}): starting a new copy.");
        }
        if let Ok(me) = std::env::current_exe() {
            let started = Command::new(me)
                .args(std::env::args_os().skip(1))
                .env("FACE_RESTART_AFTER", std::process::id().to_string())
                .env_remove("FACE_LINK") // the new copy makes its own secret for its own tracker
                .stdin(Stdio::null())
                .spawn();
            if let Err(e) = started {
                println!("The app couldn't be started again: {e}");
            }
        }
        if let Some(window) = self.window() {
            (window.before_exit)();
        }
        std::process::exit(0);
    }

    /// Start the app again in a moment (the answer goes out first): only with a window, and at most once an
    /// hour, so that a camera that is really broken can't keep the app restarting. True when it will.
    ///
    /// Under a locked screen (Windows) the rule is once per lock instead, whatever the hour says: a Remote
    /// Desktop disconnect locks the session, Windows stops the camera 20 s later, and the running app doesn't
    /// get it back, while the app started again has it within seconds. Locked at the computer's own screen
    /// that changes nothing, so it isn't done twice: the record keeps `locked` until the screen is seen
    /// unlocked (`unlock_seen`).
    pub fn restart_soon(self: &Arc<Self>, why: &str, locked: Option<bool>) -> bool {
        if self.window().is_none() {
            return false;
        }
        let rec = self.restarts().get("camera").cloned().unwrap_or(Value::Null);
        let last = rec.get("at").and_then(Value::as_f64).unwrap_or(0.0);
        let locked = locked.unwrap_or_else(|| desktop::screen_locked() == Some(true));
        if locked {
            if rec.get("locked") == Some(&json!(true)) {
                println!("Not starting the app again ({why}): it did so once since the screen was locked.");
                return false;
            }
        } else if unix_now() - last < RESTART_EVERY {
            println!("Not starting the app again ({why}): it did so less than {} min ago.", RESTART_EVERY as u64 / 60);
            return false;
        }
        if self.note_restart("camera", json!({"at": unix_now(), "why": why, "locked": locked})).is_err() {
            return false;
        }
        println!("Starting the app again: {why}{}.", if locked { " (the screen is locked)" } else { "" });
        let app = self.clone();
        let _ = std::thread::Builder::new().name("starting again".into()).spawn(move || {
            std::thread::sleep(Duration::from_secs(1));
            app.restart_app();
        });
        true
    }

    /// The screen isn't locked (any more): the next lock gets its one restart again. Asked by the window's
    /// watchdog every few seconds; the record is written only when there is something to clear.
    fn unlock_seen(&self, locked: Option<bool>) -> bool {
        let Some(Value::Object(mut rec)) = self.restarts().get("camera").cloned() else { return false };
        if rec.get("locked") != Some(&json!(true)) || locked != Some(false) {
            return false;
        }
        rec.insert("locked".to_string(), json!(false));
        self.note_restart("camera", Value::Object(rec)).is_ok()
    }

    /// For the log: what hasn't answered, the window's page or the tabs it named.
    fn not_answering(&self) -> String {
        let names = held(&self.alive).1.clone();
        if names.is_empty() {
            return "The window's page".to_string();
        }
        let tabs: Vec<String> =
            names.iter().filter_map(|k| TABS.iter().find(|t| t.0 == k)).map(|t| format!("the {} tab", t.1)).collect();
        format!("The page of {}", tabs.join(" and "))
    }

    /// Watch the window's page and bring it back when it stops: on a thread of its own, every 5 s.
    pub fn watch_window(self: &Arc<Self>) {
        let app = self.clone();
        let _ = std::thread::Builder::new().name("window watchdog".into()).spawn(move || {
            let (began, mut st) = (Instant::now(), Watch::default());
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let now = began.elapsed().as_secs_f64();
                app.unlock_seen(desktop::screen_locked());
                let alive_at = held(&app.alive).0.map(|at| at.duration_since(began).as_secs_f64());
                let silent = alive_at.map_or(0.0, |at| now - at);
                match watch_step(&mut st, now, alive_at, false) {
                    Step::Reset => {
                        let mut alive = held(&app.alive);
                        if alive.0.is_some() {
                            alive.0 = Some(Instant::now());
                        }
                    }
                    Step::Restart => {
                        let (due, n) = window_restart_due(&app.restarts().get("window").cloned().unwrap_or(Value::Null), unix_now());
                        let what = app.not_answering();
                        if !due {
                            if !st.held {
                                println!("{what} hasn't answered for {silent:.0} s, but the app was started again for that {n} time(s) lately: waiting longer before the next.");
                            }
                            st.held = true;
                            continue;
                        }
                        println!("{what} hasn't answered for {silent:.0} s: starting the app again.");
                        if app.note_restart("window", json!({"at": unix_now(), "n": n + 1})).is_ok() {
                            app.restart_app();
                        }
                    }
                    Step::Kill | Step::Nothing => {
                        if alive_at.is_some_and(|at| now - at < HUNG_AFTER) {
                            st.held = false;
                        }
                    }
                }
            }
        });
    }

    /// The tab page's word, every 2 s: it runs, and which of its tabs' pages haven't answered it.
    fn alive_said(&self, silent: &str, again: &str) {
        let tab = |key: &str| TABS.iter().find(|t| t.0 == key).map(|t| t.1);
        for name in again.split(',').filter_map(tab) {
            println!("Tab page: the {name} tab's page hadn't begun to load: given its address again.");
        }
        let names: Vec<String> = silent.split(',').filter(|k| tab(k).is_some()).map(str::to_string).collect();
        let mut alive = held(&self.alive);
        if names.is_empty() {
            alive.0 = Some(Instant::now());
        }
        alive.1 = names;
    }
}

/// The app started: the tab page serves, the two programs are being started, LM Studio is looked after.
pub struct Started {
    pub app: Arc<App>,
    /// the tab page's address
    pub shell: String,
}

/// Start the app's own server and its two programs (the labeling tool in this process, the tracker in a
/// process of its own, which is this program again as `tracker-child`). `sync`: the labeling tool keeps
/// the library and the shared database the same.
pub fn start(root: &Path, home: &Path, pages: &Path, sync: bool) -> Result<Started, String> {
    // One copy of the app for a private folder. Before, a copy told that another runs by that one answering
    // on its port: a second copy started while the first was still loading found the port taken and nobody
    // answering, took the next ports, and both ran on one library, each ending the other's window (a Raspberry
    // Pi, 2026-11-23: two starts eight seconds apart). The place is taken before anything else and held for
    // the run; a copy that can't take it ends here, having started nothing.
    take_place(home)?;
    let ports = ports(std::env::var("FACE_PORTS").ok().as_deref())?;
    let app = App::new(root, home, pages, ports, sync);
    // first: the programs started next trust this page to frame them
    let shell = app.serve_shell()?;
    app.begin();
    Ok(Started { app, shell })
}

/// The app started, with the first-run question (nothing may be assumed to be installed: what the app needs
/// and doesn't find beside it is said before anything is served). `get`: Some(true) fetches it without
/// asking (a start that nobody watches), Some(false) never does, None asks in the app's own page: the tab
/// page's address shows the question, and the two programs start once it is answered.
#[cfg(feature = "fetch")]
pub fn start_asking(root: &Path, home: &Path, pages: &Path, sync: bool, get: Option<bool>) -> Result<Started, String> {
    use face_tools::first_run::{fetch, missing};
    // (before the question too: two copies that both ask would both begin)
    take_place(home)?;
    let m = missing(root, true);
    if m.is_empty() {
        return start(root, home, pages, sync);
    }
    println!("Not on this computer yet ({}):", root.display());
    m.lines().iter().for_each(|line| println!("  - {line}"));
    match get {
        Some(true) if m.fetchable() => {
            fetch(root, &m, &mut |line| println!("  getting {line} ..."))?;
            println!("Done.");
        }
        None => {
            if let Some(first) = crate::firstrun::FirstRun::due(root, home) {
                println!("Asked about in the app's own page, before anything else starts.");
                let ports = ports(std::env::var("FACE_PORTS").ok().as_deref())?;
                let app = App::new(root, home, pages, ports, sync);
                *held(&app.first) = Some(first);
                let shell = app.serve_shell()?;
                return Ok(Started { app, shell });
            }
            if m.fetchable() {
                println!(
                    "Not asked about again: answered with \"not now\" before (start with --get to download them)."
                );
            }
        }
        _ => {
            if m.fetchable() {
                println!("Not downloaded (start with --get to download them).");
            }
        }
    }
    start(root, home, pages, sync)
}

#[cfg(not(feature = "fetch"))]
pub fn start_asking(root: &Path, home: &Path, pages: &Path, sync: bool, _: Option<bool>) -> Result<Started, String> {
    start(root, home, pages, sync)
}

/// This program as the app's second process: `tracker-child <port> <the app's process> <the tab page's
/// port> <the Cameras page's port> [--pages <folder>]`. Its pages may be framed by the app's tab page, and
/// it ends when the app's process is gone. Returns only with an error; ends the process with `PORT_TAKEN`
/// when its port was taken between the app's look and its own.
pub fn tracker_child(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut number =
        |what: &str| args.next().and_then(|v| v.parse::<u32>().ok()).ok_or(format!("tracker-child needs {what}"));
    let (port, parent, shell) =
        (number("its port")? as u16, number("the app's process")?, number("the tab page's port")? as u16);
    let cameras = Some(number("the Cameras page's port")? as u16).filter(|p| *p != 0);
    let mut pages = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--pages" => pages = Some(PathBuf::from(args.next().ok_or("--pages needs a folder")?)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let (root, home) = (crate::paths::root(), crate::paths::home());
    guard::allow_frame_parents(shell);
    watch_parent(parent);
    // the port first: taken between the app's look and this one's, the app tries the next
    match http::bind(port) {
        Ok(free) => drop(free),
        Err(_) => std::process::exit(PORT_TAKEN),
    }
    trackerlink::start_telling(); // an alert or a visit saved here: the sync hears of it at once
    println!("Face tracker: loading ...");
    let pages = pages.unwrap_or_default(); // (none named: the program's own)
    if let Err(e) = crate::start_tracker_with(&root, &home, &pages, port, cameras) {
        println!("Face tracker: couldn't start in its own process: {e}");
        std::process::exit(if e.starts_with("Port ") { PORT_TAKEN } else { 1 });
    }
    loop {
        std::thread::park();
    }
}

/// End a process and wait until it is gone, so that its port is free for the one started next.
fn end_process(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// {latest, newer} from GitHub's latest release of this project, or {error}. Only when asked (the window's
/// "Check for updates"): the app contacts nothing on its own.
fn latest_release() -> Value {
    #[cfg(feature = "import")]
    {
        let numbers = |text: &str| -> Vec<u64> {
            let mut out = Vec::new();
            for part in text.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).take(3) {
                out.push(part.parse().unwrap_or(0));
            }
            out
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent("FaceRecognizer")
            .build()
            .into();
        let asked = agent
            .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| e.to_string())
            .and_then(|mut answer| answer.body_mut().read_to_vec().map_err(|e| e.to_string()))
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).map_err(|e| e.to_string()));
        match asked {
            Ok(said) => {
                let tag = said["tag_name"].as_str().unwrap_or("").to_string();
                json!({"latest": tag, "newer": numbers(&tag) > numbers(VERSION)})
            }
            Err(why) => json!({"error": why}),
        }
    }
    #[cfg(not(feature = "import"))]
    {
        json!({"error": format!("this program was built without its downloads ({REPO})")})
    }
}

/// The tab page's server.
struct Shell {
    app: Arc<App>,
    policy: String,
    permissions: String,
}

impl Shell {
    fn new(app: Arc<App>) -> Shell {
        // every port the two programs' pages can be on
        let tools: Vec<u16> = (app.ports.label..app.ports.track + PORT_TRIES).collect();
        let frames: Vec<String> = tools.iter().map(|p| format!("http://127.0.0.1:{p}")).collect();
        let quoted: Vec<String> = frames.iter().map(|f| format!("\"{f}\"")).collect();
        // camera and microphone: handed on only to a program's page, and of those only to a frame that asks
        // for them (the tracker's alone, in the page). "self" has to be there: a page can't hand on what it
        // may not use; this page's own scripts never use either.
        let permissions = ["camera", "microphone"].map(|what| format!("{what}=(self {})", quoted.join(" "))).join(", ");
        // the two programs' pages in its frames; no pictures of its own beyond its server's; never framed
        let policy = guard::policy_never_framed(&[("frame-src", &frames.join(" ")), ("img-src", "'self'")]);
        Shell { app, policy, permissions }
    }
}

impl Service for Shell {
    fn page_policy(&self) -> String {
        self.policy.clone()
    }

    fn permissions(&self) -> Option<String> {
        Some(self.permissions.clone())
    }

    /// the page's files to anyone on this computer; what the window is doing only to the page itself
    fn get_rules(&self, path: &str) -> (bool, bool) {
        let page_file = path == "/favicon.ico" || path == "/first_run.js" || PAGES.iter().any(|p| p.0 == path);
        (ACTS.contains(&path), !page_file)
    }

    /// the one thing this server is told: the answer to the first-run question
    fn body_limit(&self, path: &str) -> Option<usize> {
        (cfg!(feature = "fetch") && path == "/first-run").then_some(201)
    }

    #[cfg(feature = "fetch")]
    fn post(&self, req: &Request) -> Response {
        let Some(first) = held(&self.app.first).clone().filter(|_| req.path == "/first-run") else {
            return Response::error(404, "not found");
        };
        let said: Option<Value> = serde_json::from_slice(&req.body).ok();
        let Some(get) = said.as_ref().and_then(|s| s["get"].as_bool()) else {
            return Response::error(400, "the answer is {\"get\": true} or {\"get\": false}");
        };
        let app = self.app.clone();
        first.answer(get, Box::new(move || app.begin()));
        Response::json(first.said())
    }

    #[cfg(not(feature = "fetch"))]
    fn post(&self, _: &Request) -> Response {
        Response::error(404, "not found")
    }

    fn get(&self, req: &Request) -> Response {
        let (app, path) = (&self.app, req.path.as_str());
        // the first-run question, before anything else: the app's address shows it until it is answered
        #[cfg(feature = "fetch")]
        match (path, held(&app.first).clone()) {
            ("/", Some(first)) if first.waiting() => {
                return Response::bytes(HTML, crate::firstrun::PAGE.as_bytes().to_vec());
            }
            ("/first_run.js", _) => return Response::bytes(SCRIPT, crate::firstrun::SCRIPT.as_bytes().to_vec()),
            ("/first-run", Some(first)) => return Response::json(first.said()),
            ("/first-run", None) => return Response::json(json!({"stage": "done"})),
            _ => {}
        }
        if path == "/favicon.ico" {
            return match crate::pagefiles::read(&app.pages, "icons/icon.ico") {
                Some(icon) => Response::bytes("image/x-icon", icon),
                None => Response::error(404, "not found"),
            };
        }
        if let Some((_, file, ctype)) = PAGES.iter().find(|p| p.0 == path) {
            return match crate::pagefiles::read(&app.pages, file).map(String::from_utf8) {
                // (as text, each line ending in a line feed whatever the file has)
                Some(Ok(page)) => {
                    let page = page.replace("\r\n", "\n").replace('\r', "\n").replace("{{VERSION}}", VERSION);
                    Response::bytes(ctype, page.into_bytes())
                }
                _ => Response::error(404, "not found"),
            };
        }
        match path {
            // alerts: the window's title and taskbar button (when there is a window)
            "/attention" => {
                if let Some(window) = app.window() {
                    let unread = req.param("unread").and_then(|n| n.parse::<u64>().ok()).unwrap_or(0).min(9999);
                    (window.attention)(unread, req.param("flash").as_deref() == Some("1"));
                }
                Response::json(json!({}))
            }
            "/alive" => {
                app.alive_said(&req.param("silent").unwrap_or_default(), &req.param("again").unwrap_or_default());
                Response::json(json!({}))
            }
            // the camera won't start even after a page reload: only an app window is started again
            "/restart-app" => {
                Response::json(json!({"restarting": app.restart_soon("the camera wouldn't start", None)}))
            }
            "/restart" => match req.param("what").filter(|what| NAMES.iter().any(|n| n.0 == what)) {
                Some(what) => Response::json(json!({"what": what, "now": app.restart(&what)})),
                None => Response::error(400, "unknown program"),
            },
            "/open-release" => {
                // in the normal browser, not inside this window
                desktop::open_url(&format!("https://github.com/{REPO}/releases/latest"));
                Response::json(json!({}))
            }
            "/paths" | "/open" => {
                let (data, private) = (app.root.join("data"), app.home.clone());
                if path == "/open" {
                    // only the two fixed folders, by their names
                    let folder = match req.query.split_once("what=").map_or(req.query.as_str(), |q| q.1) {
                        "data" => Some(&data),
                        "private" => Some(&private),
                        _ => None,
                    };
                    if let Some(folder) = folder {
                        let _ = std::fs::create_dir_all(folder);
                        desktop::open_folder(folder);
                    }
                }
                Response::json(json!({"data": data.display().to_string(), "private": private.display().to_string()}))
            }
            "/update" => Response::json(latest_release()),
            "/status" => Response::json(Value::Object(held(&app.status).clone())),
            _ => Response::error(404, "not found"),
        }
    }
}

/// End this process when the app's own process is gone (closed, ended, crashed): the camera's server must
/// not live on without its window. For the tracker's process.
pub fn watch_parent(parent: u32) {
    let _ = std::thread::Builder::new().name("the app's process".into()).spawn(move || {
        while !gone(parent) {
            std::thread::sleep(Duration::from_secs(1));
        }
        std::process::exit(0);
    });
}

#[cfg(windows)]
fn gone(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: a handle to a process by its number, asked for the right to wait only, and closed again
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return true;
        }
        let ended = WaitForSingleObject(handle, 0) == WAIT_OBJECT_0;
        CloseHandle(handle);
        ended
    }
}

#[cfg(unix)]
fn gone(pid: u32) -> bool {
    // SAFETY: asks the system which process started this one
    unsafe { libc::getppid() as u32 != pid }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ports_are_the_apps_fixed_ones() {
        assert_eq!(ports(None), Ok(Ports { shell: 8764, label: 8765, track: 8766, cams: 8769 }));
        assert_eq!(ports(Some(" 9100 ")), Ok(Ports { shell: 9100, label: 9101, track: 9102, cams: 9105 }));
        assert_eq!(ports(Some("abc")).map(|p| p.shell), Ok(8764), "not a number: the usual ports");
        assert_eq!(ports(Some("")).map(|p| p.shell), Ok(8764));
        for bad in ["80", "1023", "65001", "99999999999999999999"] {
            assert_eq!(ports(Some(bad)), Err(format!("FACE_PORTS must be between 1024 and 65000, not {bad}")));
        }
    }

    #[test]
    fn the_tab_page_hands_the_camera_to_the_programs_ports_only_and_is_never_framed() {
        let dir = std::env::temp_dir().join(format!("face-app-{}", std::process::id()));
        let app = App::new(&dir, &dir, &dir, ports(Some("9200")).unwrap(), false);
        let shell = Shell::new(app);
        let policy = shell.page_policy();
        assert!(policy.ends_with("frame-ancestors 'none'"), "{policy}");
        assert!(policy.contains("; img-src 'self'; "), "no pictures from anywhere else: {policy}");
        let frames = policy.split("; ").find(|rule| rule.starts_with("frame-src ")).unwrap();
        assert_eq!(frames.split(' ').count(), 1 + 11, "the labeling tool's and the tracker's ports, ten tries each");
        assert!(frames.contains("http://127.0.0.1:9201") && frames.contains("http://127.0.0.1:9211"));
        assert!(!frames.contains(":9200") && !frames.contains(":9212"));
        let permissions = shell.permissions().unwrap();
        assert!(permissions.starts_with("camera=(self \"http://127.0.0.1:9201\" "));
        assert!(permissions.contains("\"), microphone=(self \"http://127.0.0.1:9201\" "));
        // what does something needs the page's header; everything but the page's files is the page's own
        assert_eq!(shell.get_rules("/"), (false, false));
        assert_eq!(shell.get_rules("/app_ui.js"), (false, false));
        assert_eq!(shell.get_rules("/favicon.ico"), (false, false));
        assert_eq!(shell.get_rules("/status"), (false, true));
        assert_eq!(shell.get_rules("/alive"), (false, true));
        for act in ACTS {
            assert_eq!(shell.get_rules(act), (true, true), "{act}");
        }
    }

    #[test]
    fn the_apps_own_pages_are_the_tab_page_and_what_this_process_serves() {
        let dir = std::env::temp_dir();
        let app = App::new(&dir, &dir, &dir, ports(Some("9300")).unwrap(), false);
        // nothing serves yet: nothing is the app's
        assert!(!app.own_page("http://127.0.0.1:9300/") && !app.own_page("http://127.0.0.1:0/"));
        *held(&app.shell) = 9300;
        app.set("label", json!({"url": "http://127.0.0.1:9301/"}));
        app.set("track", json!({"url": "http://127.0.0.1:9302/", "cameras": "http://127.0.0.1:9305/cameras"}));
        for own in [
            "http://127.0.0.1:9300/",
            "http://127.0.0.1:9301/",
            "http://127.0.0.1:9302/?x=1",
            "http://127.0.0.1:9305/cameras",
        ] {
            assert!(app.own_page(own), "{own}");
        }
        // another program on this computer, another computer, another scheme, a port that only begins alike
        for other in [
            "http://127.0.0.1:9303/",
            "http://127.0.0.1:93000/",
            "http://localhost:9300/",
            "https://127.0.0.1:9300/",
            "http://127.0.0.1.example.org:9300/",
            "http://example.org/",
            "about:blank",
        ] {
            assert!(!app.own_page(other), "{other}");
        }
        // the tracker's process being started again: its page is still the one this app showed
        app.set("track", json!({}));
        assert!(!app.own_page("http://127.0.0.1:9302/"));
        *held(&app.again) = Some(9302);
        assert!(app.own_page("http://127.0.0.1:9302/"));
    }

    #[test]
    fn only_the_tracker_this_app_started_and_its_own_tab_page_get_the_camera_unasked() {
        let dir = std::env::temp_dir();
        let app = App::new(&dir, &dir, &dir, ports(Some("9300")).unwrap(), false);
        *held(&app.shell) = 9300;
        // nothing started yet: nobody
        assert!(!app.camera_allowed("http://127.0.0.1:9302/") && !app.camera_allowed("http://127.0.0.1:9300/"));
        // a tracker in the status that this process holds no process for (a copy started on its own): nobody
        app.set("track", json!({"url": "http://127.0.0.1:9302/"}));
        assert!(!app.camera_allowed("http://127.0.0.1:9302/") && !app.camera_allowed("http://127.0.0.1:9300/"));
        // while its own tracker's process is being started again, the page it showed is still answered
        *held(&app.again) = Some(9302);
        for ok in [
            "http://127.0.0.1:9302/",
            "http://127.0.0.1:9302",
            "http://127.0.0.1:9302/index.html",
            "http://127.0.0.1:9300/",
        ] {
            assert!(app.camera_allowed(ok), "{ok}");
        }
        for other in [
            "http://127.0.0.1:9301/",
            "http://127.0.0.1:9303/",
            "https://127.0.0.1:9302/",
            "http://localhost:9302/",
            "http://127.0.0.1.example.org:9302/",
            "http://example.org/",
            "http://127.0.0.1:93020/",
            "",
        ] {
            assert!(!app.camera_allowed(other), "{other}");
        }
    }

    #[test]
    fn a_silent_window_is_started_again_after_a_minute_and_not_after_a_sleep() {
        let (mut st, alive) = (Watch::default(), Some(100.0));
        // it answers: nothing
        assert_eq!(watch_step(&mut st, 105.0, alive, false), Step::Nothing);
        assert_eq!(watch_step(&mut st, 125.0, alive, false), Step::Nothing);
        // 30 s of silence: counted from here (on Windows the page's process isn't stopped first)
        assert_eq!(watch_step(&mut st, 130.0, alive, false), Step::Nothing);
        assert_eq!(watch_step(&mut st, 145.0, alive, false), Step::Nothing);
        // and 30 s more: the app starts again
        assert_eq!(watch_step(&mut st, 160.0, alive, false), Step::Restart);
        // where the page's own process can be stopped, that comes first
        let mut st = Watch::default();
        assert_eq!(watch_step(&mut st, 110.0, alive, true), Step::Nothing);
        assert_eq!(watch_step(&mut st, 130.0, alive, true), Step::Kill);
        assert_eq!(watch_step(&mut st, 150.0, alive, true), Step::Nothing);
        assert_eq!(watch_step(&mut st, 165.0, alive, true), Step::Restart);
        // the computer slept (a gap of more than 20 s between two looks): counting starts again
        let mut st = Watch::default();
        assert_eq!(watch_step(&mut st, 110.0, alive, false), Step::Nothing);
        assert_eq!(watch_step(&mut st, 3000.0, alive, false), Step::Reset);
        // a page that never said anything is not a hung one
        assert_eq!(watch_step(&mut Watch::default(), 500.0, None, false), Step::Nothing);
    }

    #[test]
    fn restarts_in_a_row_wait_longer_each_time() {
        let now = 1_800_000_000.0;
        assert_eq!(window_restart_due(&Value::Null, now), (true, 0));
        assert_eq!(window_restart_due(&json!({"at": now - 60.0, "n": 1}), now), (false, 1));
        assert_eq!(window_restart_due(&json!({"at": now - 121.0, "n": 1}), now), (true, 1));
        assert_eq!(window_restart_due(&json!({"at": now - 200.0, "n": 2}), now), (false, 2));
        assert_eq!(window_restart_due(&json!({"at": now - 241.0, "n": 2}), now), (true, 2));
        assert_eq!(window_restart_due(&json!({"at": now - 3000.0, "n": 9}), now), (false, 9), "at most an hour");
        assert_eq!(
            window_restart_due(&json!({"at": now - 3601.0, "n": 9}), now),
            (true, 0),
            "an hour without one starts over"
        );
        assert_eq!(window_restart_due(&json!({"at": "soon", "n": "many"}), now), (true, 0));
    }

    #[test]
    fn the_app_is_started_again_for_a_camera_once_an_hour_and_once_per_lock() {
        let dir = std::env::temp_dir().join(format!("face-app-restarts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = App::new(&dir, &dir, &dir, ports(None).unwrap(), false);
        // without a window nothing is started again
        assert!(!app.restart_soon("the camera wouldn't start", Some(false)));
        assert!(app.restarts().is_empty());
        // (the rules, from the record alone: starting again for real would end this test's process)
        app.note_restart("camera", json!({"at": unix_now() - 100.0, "why": "x", "locked": false})).unwrap();
        app.set_window(Window { attention: Box::new(|_, _| {}), before_exit: Box::new(|| {}) });
        assert!(!app.restart_soon("the camera wouldn't start", Some(false)), "less than an hour ago");
        app.note_restart("camera", json!({"at": unix_now() - 100.0, "why": "x", "locked": true})).unwrap();
        assert!(!app.restart_soon("the camera wouldn't start", Some(true)), "once since the screen was locked");
        // the screen seen unlocked: the next lock gets its one start again
        assert!(!app.unlock_seen(None) && !app.unlock_seen(Some(true)));
        assert!(app.unlock_seen(Some(false)));
        assert_eq!(app.restarts()["camera"]["locked"], false);
        assert!(!app.unlock_seen(Some(false)), "nothing to clear: nothing written");
        // an older file, which held only the camera's record
        std::fs::write(dir.join("restarted.json"), "{\"at\": 12.5, \"why\": \"old\"}").unwrap();
        assert_eq!(app.restarts()["camera"]["at"], 12.5);
        app.note_restart("window", json!({"at": 20.0, "n": 1})).unwrap();
        let kept = std::fs::read_to_string(dir.join("restarted.json")).unwrap();
        assert_eq!(kept, "{\"camera\": {\"at\": 12.5, \"why\": \"old\"}, \"window\": {\"at\": 20.0, \"n\": 1}}");
        // which tabs are named
        app.alive_said("track,cams", "");
        assert_eq!(app.not_answering(), "The page of the Face tracking tab and the Cameras tab");
        app.alive_said("", "");
        assert_eq!(app.not_answering(), "The window's page");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tracker_is_started_at_most_five_times_in_ten_minutes() {
        let dir = std::env::temp_dir();
        let app = App::new(&dir, &dir, &dir, ports(None).unwrap(), false);
        assert!((0..5).all(|_| app.tracker_start_due()));
        assert!(!app.tracker_start_due());
        held(&app.starts).clear();
        assert!(app.tracker_start_due());
    }

    #[test]
    fn the_tab_page_names_the_tabs_that_do_not_answer() {
        let dir = std::env::temp_dir();
        let app = App::new(&dir, &dir, &dir, ports(None).unwrap(), false);
        app.alive_said("", "");
        assert!(held(&app.alive).0.is_some() && held(&app.alive).1.is_empty());
        let at = held(&app.alive).0;
        app.alive_said("track,nothing,cams", "label");
        assert_eq!(held(&app.alive).1, ["track", "cams"]);
        assert_eq!(held(&app.alive).0, at, "a page whose tabs don't answer doesn't count as running");
    }
}
