//! The app on a computer: one window, the system's own web
//! view, showing the tab page that the app's own server sends, with the Face tracking, Labeling, Cameras
//! and Settings tabs.
//!
//!     FaceRecognizer [--no-sync] [--browser] [--get | --no-get] [--pages <folder with the page files>]
//!
//! The labeling tool runs in this process and the tracker in a second one: this program again, started as
//! `FaceRecognizer tracker-child ...`. `FACE_UI=browser` (or `--browser`) opens the tab page in the default
//! browser instead of the window, `FACE_UI=none` opens nothing (test copies), `FACE_UI=hidden` makes the
//! window without showing it (a check), `FACE_PORTS=<n>` moves the ports, `FACE_ROOT` and `FACE_HOME` the
//! folders (otherwise the installed app's: models and data beside the program on Windows, in the user's
//! application-data folder `FaceRecognizer` on macOS and Linux). The program
//! carries its page files; `--pages` reads them from a folder instead. Without a console (the built app) what the programs say goes to `app.log` in the private folder,
//! each line with the time in UTC.
//!
//! What the app needs and doesn't find beside it (models, the model runtime, FFmpeg ...) is asked about in
//! its own page before anything else is shown or started (`firstrun.rs`). `--get` fetches it without asking,
//! `--no-get` never does and doesn't ask; `FACE_UI=none` doesn't ask either.
//!
//! The window: on Windows WebView2, on a Mac WebKit (`mac.rs`), both through `wry`. It answers the
//! tracker's camera and microphone request itself (`App::camera_allowed`: only the tracker this process
//! started, and the tab page around it), keeps its pages running in the background (the camera's stream
//! needs it), says new alerts in its title and on its taskbar button or Dock icon, loads its page again
//! when the page's own process died, is watched (`App::watch_window`: a page that stops answering gets the
//! app started again), and ends the app, and the tracker's process with it, when it is closed. A link that
//! leaves the app's own pages is opened in the owner's browser, never in the window. On Windows what the
//! pages remember is kept in the private folder (`webview`); on a Mac WebKit keeps it for the app itself.
//! On Linux and the Raspberry Pi the window is a Chromium window of the app's own (`linux.rs`).
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;

#[cfg(windows)]
use std::sync::Arc;
use std::{
    path::{Path, PathBuf},
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(windows)]
use face_server::app::App;
use face_server::{app, desktop, paths};

/// What the window's browser is started with (Chromium's switches): the pages keep running when the window
/// is minimized, covered or not in front (it otherwise slows their timers to once a second and stops
/// drawing, and the camera's stream froze), and camera frames stay in ordinary memory (read back from the
/// graphics processor, one frame in ten took 165 ms in a desktop session nobody is connected to). The first
/// three names are the web view library's own defaults, kept.
#[cfg(windows)]
const BROWSER_FLAGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion,\
IntensiveWakeUpThrottling,MediaFoundationD3D11VideoCapture --disable-background-timer-throttling \
--disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

/// what the programs say goes through this process's own log writer (no console)
static LOGGING: AtomicBool = AtomicBool::new(false);
/// how many times the log writer was asked to have everything written, and has
static WRITTEN: (Mutex<u64>, Condvar) = (Mutex::new(0), Condvar::new());
/// a line that asks the log writer to say when everything before it is in the file (never written itself)
const SYNC: &str = "\u{0}written";

fn main() {
    face_server::no_telemetry();
    let mut args = std::env::args().skip(1).peekable();
    // this program at the owner's sign-in, for the shared database held on this computer: its server started,
    // nothing else (no window, no other program; the app need not run for the database to)
    // and as Setup's uninstaller runs it (`stop-database`): the server stopped, its data left as it is
    let database = args.peek().cloned().filter(|a| a == "start-database" || a == "stop-database");
    if let Some(what) = database {
        let named: Vec<String> = args.skip(1).collect();
        let at = |flag: &str| named.iter().position(|a| a == flag).and_then(|i| named.get(i + 1)).cloned();
        // SAFETY: only this thread runs yet
        unsafe {
            std::env::set_var("FACE_ROOT", at("--root").map_or_else(paths::app_root, Into::into));
            if let Some(home) = at("--home") {
                std::env::set_var("FACE_HOME", home);
            }
        }
        let (root, home) = (paths::root(), paths::home());
        keep_a_log(&home);
        if what == "start-database" {
            face_server::dbhost::at_sign_in(&root, &home);
        } else {
            face_server::dbhost::uninstalled(&root, &home);
        }
        log_written();
        return;
    }
    let child = args.peek().map(String::as_str) == Some("tracker-child");
    // One folder for models and data for everything in this process (the installed app's, unless one is
    // named): parts of the programs ask for it on their own.
    if std::env::var_os("FACE_ROOT").is_none_or(|named| named.is_empty()) {
        // SAFETY: only this thread runs yet
        unsafe { std::env::set_var("FACE_ROOT", paths::app_root()) };
    }
    let home = paths::home();
    if !child {
        // which version ran here before (a new copy, an update): before the log or anything else is written there
        face_server::versionnote::note(&home);
        app::wait_for_previous(); // started again by a copy of itself: that one's ports are free first
    }
    keep_a_log(&home);
    if child {
        // this program as the tracker's own process
        if let Err(e) = app::tracker_child(args.skip(1)) {
            println!("Face tracker: couldn't start in its own process: {e}");
            log_written();
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = run(args, home) {
        println!("{e}");
        log_written();
        std::process::exit(1);
    }
}

/// A line of the log as it is written: with the time in UTC before it (HH:MM:SS), so that a freeze or a
/// restart can be matched with what else happened then. A line of nothing is left as it is.
fn stamped(line: &str, seconds_today: u64) -> String {
    if line.trim().is_empty() {
        return format!("{line}\n");
    }
    format!("{:02}:{:02}:{:02} {line}\n", seconds_today / 3600, seconds_today % 3600 / 60, seconds_today % 60)
}

/// seconds within which a line said again in the same words is not written again
const SAME_FOR: u64 = 60;
/// lines whose words are remembered, at most
const SAME_KEPT: usize = 400;

/// The lines said lately, so that one said over and over is in the log once a minute and not every time: a
/// library that says the same sentence fifty times a minute (a sound library on Linux did, 2026-11-23) or a
/// device that can't be reached and is tried every few seconds would otherwise fill the log with it. Only a
/// line in exactly the same words is held back; when it is written again it says how often it was said
/// meanwhile, so nothing is lost but the repetition.
#[derive(Default)]
struct Repeats {
    /// the words -> (when they were last written, how often they were said since and held back)
    said: std::collections::HashMap<String, (u64, u32)>,
}

impl Repeats {
    /// What to write for a line said at `now` (seconds): the line, with how often it was said since it was
    /// last written; or None while it was written less than `SAME_FOR` ago.
    fn take(&mut self, line: &str, now: u64) -> Option<String> {
        if line.trim().is_empty() {
            return Some(line.to_string());
        }
        if self.said.len() >= SAME_KEPT {
            // (lines not said for a while are forgotten: said again, they are written like new ones)
            self.said.retain(|_, (at, held)| *held > 0 || now.saturating_sub(*at) < SAME_FOR);
            if self.said.len() >= SAME_KEPT {
                self.said.clear();
            }
        }
        match self.said.get_mut(line) {
            Some((at, held)) if now.saturating_sub(*at) < SAME_FOR => {
                *held += 1;
                None
            }
            Some((at, held)) => {
                let more = std::mem::take(held);
                *at = now;
                Some(match more {
                    0 => line.to_string(),
                    1 => format!("{line}  (said once more since)"),
                    n => format!("{line}  (said {n} more times since)"),
                })
            }
            None => {
                self.said.insert(line.to_string(), (now, 0));
                Some(line.to_string())
            }
        }
    }
}

/// Without a console, what the programs say goes to `app.log` in the private folder (both processes write
/// the same file, each line added at its end, each with its time).
#[cfg(windows)]
fn keep_a_log(home: &Path) {
    use std::os::windows::io::IntoRawHandle;

    use windows::Win32::{
        Foundation::HANDLE,
        System::Console::{GetConsoleWindow, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle},
    };
    // SAFETY: asks whether this process has a console
    if !unsafe { GetConsoleWindow() }.is_invalid() {
        return;
    }
    let _ = std::fs::create_dir_all(home);
    let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open(home.join("app.log")) else { return };
    let Ok((reader, writer)) = std::io::pipe() else { return };
    let handle = HANDLE(writer.into_raw_handle());
    // SAFETY: the pipe's writing end stays open for the life of the process, as its two output streams
    unsafe {
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
        let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
    }
    write_the_log(reader, log);
}

/// The same where the program wasn't started from a terminal (an app started from the Finder or a launcher
/// has nowhere to say anything).
#[cfg(unix)]
fn keep_a_log(home: &Path) {
    use std::{io::IsTerminal, os::fd::AsRawFd};
    if std::io::stdout().is_terminal() {
        return;
    }
    let _ = std::fs::create_dir_all(home);
    let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open(home.join("app.log")) else { return };
    let Ok((reader, writer)) = std::io::pipe() else { return };
    // SAFETY: this process's own two output streams become the pipe's writing end, before any other thread
    // runs; they stay open for the life of the process
    let both = unsafe { libc::dup2(writer.as_raw_fd(), 1) == 1 && libc::dup2(writer.as_raw_fd(), 2) == 2 };
    if both {
        write_the_log(reader, log);
    }
}

/// The log's writer: every line said, with its time, at the end of the file.
fn write_the_log(reader: std::io::PipeReader, mut log: std::fs::File) {
    use std::io::{BufRead, Write};
    LOGGING.store(true, Ordering::SeqCst);
    let _ = std::thread::Builder::new().name("log".into()).spawn(move || {
        let mut lines = std::io::BufReader::new(reader);
        let mut line = Vec::new();
        let mut repeats = Repeats::default();
        loop {
            line.clear();
            match lines.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches(['\r', '\n']);
            if text == SYNC {
                *WRITTEN.0.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                WRITTEN.1.notify_all();
                continue;
            }
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if let Some(text) = repeats.take(text, now) {
                let _ = log.write_all(stamped(&text, now % 86_400).as_bytes());
            }
        }
    });
}

/// Wait (a moment at most) until everything said so far is in the log: before this process ends.
fn log_written() {
    if !LOGGING.load(Ordering::SeqCst) {
        return;
    }
    let before = *WRITTEN.0.lock().unwrap_or_else(|e| e.into_inner());
    println!("{SYNC}");
    let done = WRITTEN.0.lock().unwrap_or_else(|e| e.into_inner());
    let _ = WRITTEN.1.wait_timeout_while(done, Duration::from_secs(2), |count| *count == before);
}

fn run(mut args: impl Iterator<Item = String>, home: PathBuf) -> Result<(), String> {
    // (the sync is on unless told otherwise: it does nothing until a shared database is set up)
    let (mut browser, mut sync, mut pages, mut get) = (false, true, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--browser" => browser = true,
            "--sync" => sync = true,
            "--no-sync" => sync = false,
            "--get" => get = Some(true),
            "--no-get" => get = Some(false),
            "--pages" => pages = Some(PathBuf::from(args.next().ok_or("--pages needs a folder")?)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    // (an installed app's folder for models and data; no page folder named: the program's own pages)
    let root = paths::app_root();
    let pages = pages.unwrap_or_default();
    println!("\n--- {} {} {} starting", paths::utc_now(), app::TITLE, app::VERSION);
    let ui = std::env::var("FACE_UI").unwrap_or_default();
    // (with nothing opened there is no page to ask in: said in the log, as when told not to)
    let get = get.or((ui == "none").then_some(false));
    let started = app::start_asking(&root, &home, &pages, sync, get)?;
    if ui == "none" || browser || ui == "browser" {
        println!("{} is running at {}.", app::TITLE, started.shell);
        if ui != "none" {
            desktop::open_url(&started.shell);
        }
        loop {
            std::thread::park();
        }
    }
    let why = window(&started.app, &started.shell, &home, ui != "hidden");
    // (the window returns only when it couldn't be made: without it the app is used in the browser)
    println!("No app window could open here ({why}): using the web browser instead.");
    if ui != "hidden" {
        // (a check that shows nothing gets no browser either)
        desktop::open_url(&started.shell);
    }
    loop {
        std::thread::park();
    }
}

/// The window's title: how many new alerts there are, which shows on its taskbar button too.
#[cfg(any(windows, target_os = "macos", test))]
fn title(unread: u64) -> String {
    match unread {
        0 => app::TITLE.to_string(),
        1 => format!("{} · 1 new alert", app::TITLE),
        n => format!("{} · {n} new alerts", app::TITLE),
    }
}

/// Said in the log when the number of new alerts in the window's title changes.
#[cfg(any(windows, target_os = "macos"))]
fn note_title(unread: u64) {
    static SAID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if SAID.swap(unread, Ordering::SeqCst) != unread {
        println!("Window: {}", title(unread));
    }
}

/// The app's icon for its window and taskbar button: the one in this program's own file (`build.rs`), in
/// the size the system asks for.
#[cfg(windows)]
fn icon() -> Option<tao::window::Icon> {
    use tao::platform::windows::IconExtWindows;
    tao::window::Icon::from_resource(1, None).ok()
}

/// What the window's loop is told from other threads.
#[cfg(any(windows, target_os = "macos"))]
enum Told {
    /// new alerts: how many, and whether to draw the eye to the window (from the app's own server)
    Attention(u64, bool),
    /// the pages' own process ended (a Mac's web view says so through its loop)
    #[cfg(target_os = "macos")]
    PageDied,
    /// time to say which process the pages run in
    #[cfg(target_os = "macos")]
    PageProcess,
}

#[cfg(target_os = "macos")]
use mac::window;

#[cfg(target_os = "linux")]
use linux::window;

/// The window. Returns only with why it couldn't be made; closing it ends the app.
#[cfg(windows)]
fn window(app: &Arc<App>, shell: &str, home: &Path, shown: bool) -> String {
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
        window::{UserAttentionType, WindowBuilder},
    };
    use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::DestroyWindow};
    use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

    let event_loop = EventLoopBuilder::<Told>::with_user_event().build();
    let made = WindowBuilder::new()
        .with_title(app::TITLE)
        .with_inner_size(LogicalSize::new(1440.0, 920.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .with_window_icon(icon())
        .with_taskbar_icon(icon())
        .with_visible(false) // (shown once its browser is there: never an empty window)
        .build(&event_loop);
    let window = match made {
        Ok(window) => window,
        Err(e) => return e.to_string(),
    };
    // what the pages remember (their options, each person's normals) stays between runs, in the private folder
    let mut context = WebContext::new(Some(home.join("webview")));
    let mut flags = BROWSER_FLAGS.to_string();
    // (a check looks into the window's pages through the browser's own debugging port, on this computer only)
    if let Some(port) = std::env::var("FACE_WINDOW_DEBUG").ok().and_then(|p| p.parse::<u16>().ok()) {
        flags.push_str(&format!(" --remote-debugging-port={port}"));
    }
    let own = shell.trim_end_matches('/').to_string();
    let leaves = own.clone();
    let built = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(shell)
        .with_background_color((0x0d, 0x15, 0x12, 255))
        .with_additional_browser_args(flags)
        // the window shows the app's own tab page and nothing else: a link elsewhere goes to the owner's browser
        // (WebView2 asks here about the window's own address only, not about the frames in its page; never
        // a browser from a window that isn't shown: a check)
        .with_navigation_handler(move |to| {
            let stays = to.starts_with(&leaves) || to == "about:blank";
            if !stays && shown && (to.starts_with("https://") || to.starts_with("http://")) {
                desktop::open_url(&to);
            }
            stays
        })
        .build(&window);
    let webview = match built {
        Ok(webview) => webview,
        Err(e) => {
            // no browser for the window on this computer (WebView2 isn't there or can't start). The window
            // is taken away here and now: left to its loop, which never runs, it would stay for good.
            // SAFETY: this thread's own window, which nothing uses after this
            let _ = unsafe { DestroyWindow(HWND(window.hwnd() as _)) };
            return e.to_string();
        }
    };
    window.set_visible(shown);
    allow_camera(&webview, app.clone());
    load_again_when_the_page_died(&webview);
    // alerts reach the window's loop from the app's own server; and the app watches its window from now on
    let proxy = Mutex::new(event_loop.create_proxy());
    app.set_window(app::Window {
        attention: Box::new(move |unread, flash| {
            let _ = proxy.lock().unwrap_or_else(|e| e.into_inner()).send_event(Told::Attention(unread, flash));
        }),
        before_exit: Box::new(log_written),
    });
    app.watch_window();
    println!("Window: open on {own}{}", if shown { "" } else { " (not shown)" });
    let app = app.clone();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep = &webview; // (the web view lives as long as the window's loop)
        match event {
            Event::UserEvent(Told::Attention(unread, flash)) => {
                window.set_title(&title(unread));
                note_title(unread);
                // (caption and taskbar button, until the window comes to the front)
                window.request_user_attention(flash.then_some(UserAttentionType::Critical));
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                // in the log, so that an app that is gone can be told from one that crashed
                println!("The app window was closed: the app ends.");
                app.stop_tracker();
                log_written();
                std::process::exit(0);
            }
            _ => {}
        }
    })
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn window(_: &std::sync::Arc<face_server::app::App>, _: &str, _: &Path, _: bool) -> String {
    "the window isn't part of this program on this system yet".to_string()
}

/// The window answers the tracker's camera and microphone request itself, so that no question appears that
/// nobody answers. Only for the tracker this process started and the tab page around it (`camera_allowed`);
/// anything else is left to the window's own question. Each kind of request and its answer is said once.
#[cfg(windows)]
fn allow_camera(webview: &wry::WebView, app: Arc<App>) {
    use std::collections::HashSet;

    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
            COREWEBVIEW2_PERMISSION_STATE_ALLOW,
        },
        PermissionRequestedEventHandler, take_pwstr,
    };
    use windows::core::PWSTR;
    use wry::WebViewExtWindows;

    let said: Mutex<HashSet<(String, i32, bool)>> = Mutex::new(HashSet::new());
    let handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let (mut kind, mut uri) = (COREWEBVIEW2_PERMISSION_KIND::default(), PWSTR::null());
        // SAFETY: the request's own values, read into places that live for the call
        unsafe {
            args.PermissionKind(&mut kind)?;
            args.Uri(&mut uri)?;
        }
        let uri = take_pwstr(uri);
        let wanted = kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA || kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE;
        let allowed = wanted && app.camera_allowed(&uri);
        if allowed {
            // SAFETY: the answer to this request
            unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)? };
        }
        if said.lock().unwrap_or_else(|e| e.into_inner()).insert((uri.clone(), kind.0, allowed)) {
            let what = match kind {
                COREWEBVIEW2_PERMISSION_KIND_CAMERA => "the camera".to_string(),
                COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => "the microphone".to_string(),
                other => format!("permission {}", other.0),
            };
            let answer = if allowed { "allowed" } else { "left to the window to ask" };
            println!("Permission: {uri} asked for {what}: {answer}");
        }
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: the window's own web view, on the thread that made it
    if let Err(e) = unsafe { webview.webview().add_PermissionRequested(&handler, &mut token) } {
        println!("Window: the camera's request can't be answered here ({e}): the window will ask.");
    }
}

/// When the window's page process dies (crashed, ended, out of memory) the window stays blank with nothing
/// drawn: say why in the log and load the page again. Only a page's own process needs that; the graphics
/// and utility processes (video capture, audio, network) start again by themselves, and loading the page
/// again for them would start every camera again.
#[cfg(windows)]
fn load_again_when_the_page_died(webview: &wry::WebView) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        },
        ProcessFailedEventHandler,
    };
    use wry::WebViewExtWindows;

    let handler = ProcessFailedEventHandler::create(Box::new(move |view, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        // SAFETY: the event's own value
        unsafe { args.ProcessFailedKind(&mut kind)? };
        let (what, again) = match kind {
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => ("page process ended", true),
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => ("page process doesn't answer", true),
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => ("frame's page process ended", true),
            _ => ("helper process ended", false),
        };
        println!("The window's {what} (kind {}){}.", kind.0, if again { ": loading the page again" } else { "" });
        if let (true, Some(view)) = (again, view) {
            // SAFETY: the window's own web view, on its own thread
            unsafe { view.Reload()? };
        }
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: the window's own web view, on the thread that made it
    let _ = unsafe { webview.webview().add_ProcessFailed(&handler, &mut token) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_said_over_and_over_is_written_once_a_minute_with_how_often_it_was_said() {
        let mut r = Repeats::default();
        let noise = "ALSA lib pcm_dsnoop.c:567:(snd_pcm_dsnoop_open) unable to open slave";
        assert_eq!(r.take(noise, 1000).as_deref(), Some(noise));
        // said again within the minute: held back, however often
        for at in 1001..1050 {
            assert_eq!(r.take(noise, at), None);
        }
        // other lines are written meanwhile, each as it is; lines that differ in a number are other lines
        assert_eq!(r.take("Tracker: 57 frames read in 5 s", 1010).as_deref(), Some("Tracker: 57 frames read in 5 s"));
        assert_eq!(r.take("Tracker: 58 frames read in 5 s", 1015).as_deref(), Some("Tracker: 58 frames read in 5 s"));
        // after the minute it is written again, with how often it was said meanwhile, and counted anew
        let again = r.take(noise, 1000 + SAME_FOR).unwrap();
        assert_eq!(again, format!("{noise}  (said 49 more times since)"));
        assert_eq!(r.take(noise, 1001 + SAME_FOR), None);
        assert_eq!(r.take(noise, 1000 + 2 * SAME_FOR).unwrap(), format!("{noise}  (said once more since)"));
        // said once in a long while: just the line
        assert_eq!(r.take(noise, 1000 + 9 * SAME_FOR).as_deref(), Some(noise));
        // two lines that take turns are each written once a minute
        let (a, b) = ("a device can't be reached (it is down)", "a device can't be reached (timed out)");
        let written = (0..12).filter(|i| r.take(if i % 2 == 0 { a } else { b }, 5000 + i * 20).is_some()).count();
        assert_eq!(written, 6, "two lines taking turns for four minutes, twelve sayings: each written three times");
        // an empty line is left as it is, and many different lines don't make the memory grow without end
        assert_eq!(r.take("", 6000).as_deref(), Some(""));
        for i in 0..5000u64 {
            r.take(&format!("line {i}"), 7000 + i);
        }
        assert!(r.said.len() <= SAME_KEPT);
    }

    #[test]
    fn a_line_of_the_log_starts_with_its_time_and_an_empty_one_stays_empty() {
        assert_eq!(stamped("Face tracker: loading ...", 3 * 3600 + 4 * 60 + 5), "03:04:05 Face tracker: loading ...\n");
        assert_eq!(stamped("x", 86_399), "23:59:59 x\n");
        assert_eq!(stamped("", 10), "\n");
        assert_eq!(stamped("   ", 10), "   \n");
    }

    #[test]
    fn the_title_says_how_many_alerts_are_new() {
        assert_eq!(title(0), "Face Recognizer");
        assert_eq!(title(1), "Face Recognizer · 1 new alert");
        assert_eq!(title(12), "Face Recognizer · 12 new alerts");
    }
}
