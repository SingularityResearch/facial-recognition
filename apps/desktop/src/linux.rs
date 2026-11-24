//! The app's window on Linux and the Raspberry Pi: a Chromium window of the app's own.
//!
//! Linux has no web view of the system's that runs the tracker's page well: WebKitGTK reads a camera poorly
//! and is not on every desktop. What Linux desktops do have is a Chromium (Raspberry Pi OS brings one; so do
//! most others, as Chromium, Chrome, Edge or Brave), and Chromium is what the pages were built against. So the
//! app starts that browser as a window of its own: no tabs and no address bar (`--app`), a profile of its own
//! that holds nothing but this window, the camera and microphone allowed for the app's own two addresses and
//! for nothing else, and the pages kept running when the window is covered or minimized (the switches the
//! Windows window has). Closing the window ends the app; where no such browser is found, the app says which to
//! install and opens the tab page in the default browser, as before.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use face_server::app::{self, App};
use serde_json::{Value, json};

use crate::log_written;

/// The browsers that can be the window, in the order they are looked for (`FACE_BROWSER` names another).
const BROWSERS: [&str; 7] = [
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "microsoft-edge",
    "microsoft-edge-stable",
    "brave-browser",
];

/// What the window's browser is started with, beside its address and its profile: the pages keep running
/// when the window is covered, minimized or not in front (the camera's stream needs it), and nothing of a
/// browser's own first start is shown. And it keeps no key in the desktop's keyring: on a desktop that signs
/// in by itself (a Raspberry Pi's) the keyring is locked, the desktop asks for its password, and the browser
/// shows an empty window until someone answers (2026-11-22). The window's profile holds no password and no
/// sign-in, so there is nothing for a keyring to protect.
const FLAGS: [&str; 10] = [
    "--password-store=basic",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-timer-throttling",
    "--disable-renderer-backgrounding",
    "--disable-backgrounding-occluded-windows",
    "--disable-features=CalculateNativeWinOcclusion,IntensiveWakeUpThrottling,Translate",
    "--class=FaceRecognizer",
    "--window-size=1440,920",
    "--ozone-platform-hint=auto",
];

/// A program of that name on the search path.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|folder| folder.join(name)).find(|file| file.is_file())
}

/// The browser for the window: the one `FACE_BROWSER` names, else the first of `BROWSERS` that is installed.
fn browser() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("FACE_BROWSER").filter(|b| !b.is_empty()).map(PathBuf::from) {
        return if named.is_file() { Some(named) } else { on_path(&named.to_string_lossy()) };
    }
    BROWSERS.iter().find_map(|name| on_path(name))
}

/// The folder for the window's profile. A browser installed as a snap may write only below its own folder
/// in the user's home (and not into a folder whose name starts with a dot), so its profile goes there, under
/// a name made from the app's private folder (a test copy with another private folder gets another
/// profile); every other browser keeps it in the app's private folder.
fn profile(browser: &Path, home: &Path) -> PathBuf {
    let real = std::fs::canonicalize(browser).unwrap_or_else(|_| browser.to_path_buf());
    let snap = browser.starts_with("/snap") || real.starts_with("/snap") || real.ends_with("snap");
    match (snap, std::env::var_os("HOME")) {
        (true, Some(user)) => {
            // (a number made of the folder's path: the same every time for the same folder)
            let of_home = home
                .to_string_lossy()
                .bytes()
                .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
            let name = format!("face-recognizer-window-{:08x}", of_home as u32);
            PathBuf::from(user).join("snap").join("chromium").join("common").join(name)
        }
        _ => home.join("window"),
    }
}

/// The profile's settings with the camera and the microphone allowed for the app's own two addresses (the
/// tab page, in whose name a browser asks for a frame inside it, and the tracker's page) and nothing else
/// changed. Whatever was allowed for another address before is taken out: the ports can move.
pub fn allowed(kept: Option<Value>, addresses: &[String]) -> Value {
    let mut all = kept.filter(Value::is_object).unwrap_or_else(|| json!({}));
    let grant: serde_json::Map<String, Value> =
        addresses.iter().map(|a| (format!("{a},*"), json!({"setting": 1}))).collect();
    for kind in ["media_stream_camera", "media_stream_mic"] {
        let mut at = &mut all;
        for key in ["profile", "content_settings", "exceptions"] {
            if !at[key].is_object() {
                at[key] = json!({});
            }
            at = &mut at[key];
        }
        at[kind] = Value::Object(grant.clone());
    }
    all
}

/// The window. Returns only with why it couldn't be made; closing it ends the app.
pub fn window(app: &Arc<App>, shell: &str, home: &Path, shown: bool) -> String {
    let Some(browser) = browser() else {
        return "no Chromium, Chrome, Edge or Brave is installed: with one of them the app has a window of its own"
            .to_string();
    };
    let profile = profile(&browser, home);
    let settings = profile.join("Default").join("Preferences");
    if let Err(e) = std::fs::create_dir_all(profile.join("Default")) {
        return format!("the window's profile folder can't be made: {e}");
    }
    // the camera for the app's own addresses, written before the browser reads its settings
    let own = shell.trim_end_matches('/').to_string();
    let addresses: Vec<String> = [Some(own.clone()), app.tracker_address()].into_iter().flatten().collect();
    let kept = std::fs::read(&settings).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let text = serde_json::to_vec(&allowed(kept, &addresses)).unwrap_or_default();
    if let Err(e) = std::fs::write(&settings, text) {
        return format!("the window's settings can't be written: {e}");
    }
    end_leftover(&profile);
    let mut start = Command::new(&browser);
    start.arg(format!("--user-data-dir={}", profile.display())).args(FLAGS);
    if shown {
        start.arg(format!("--app={shell}")); // a window of its own: no tabs, no address bar
        // on a Wayland desktop the browser is told so: left to find out, it looks for the session's kind in
        // the environment, which a start from outside the desktop (a service, a remote shell) doesn't have
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()) {
            start.arg("--ozone-platform=wayland");
        }
    } else {
        start.arg("--headless=new").arg(shell); // (a check: the pages run, nothing is on the screen)
    }
    // (a check looks into the window's pages through the browser's own debugging port, on this computer only)
    if let Some(port) = std::env::var("FACE_WINDOW_DEBUG").ok().and_then(|p| p.parse::<u16>().ok()) {
        start.arg(format!("--remote-debugging-port={port}"));
    }
    let child = match start.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(child) => child,
        Err(e) => return format!("{} couldn't be started: {e}", browser.display()),
    };
    let name = browser.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    println!("Window: open on {own}, in {name}{}", if shown { "" } else { " (not shown)" });
    let window = Arc::new(Mutex::new(child));
    let ending = window.clone();
    app.set_window(app::Window {
        // (the page puts new alerts into its own title, which is the window's)
        attention: Box::new(|_, _| ()),
        // the app is started again in place: its window goes first, or the new copy's would only join it
        before_exit: Box::new(move || {
            close(&ending);
            log_written();
        }),
    });
    app.watch_window();
    // the window is the browser's process: when it ends, the window was closed
    let mut again = true;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let ended = window.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
        if !matches!(ended, Ok(None)) {
            // A browser that ended before its page ever ran had no window to show. Once more first: it also
            // ends at once when another browser still holds the profile (it hands its page to that one), and
            // that one may only just have been told to end.
            if !app.page_ran() && again {
                again = false;
                std::thread::sleep(std::time::Duration::from_secs(1));
                end_leftover(&profile);
                match start.spawn() {
                    Ok(child) => {
                        println!("Window: {name} ended at once; started once more.");
                        *window.lock().unwrap_or_else(|e| e.into_inner()) = child;
                        continue;
                    }
                    Err(e) => return format!("{} couldn't be started: {e}", browser.display()),
                }
            }
            // (no desktop to show it on)
            if !app.page_ran() {
                return format!("{name} ended at once: is there a desktop to show a window on?");
            }
            // in the log, so that an app that is gone can be told from one that crashed
            println!("The app window was closed: the app ends.");
            app.stop_tracker();
            log_written();
            std::process::exit(0);
        }
    }
}

/// The process that holds a browser profile now, when it is a browser started on that very profile: Chromium
/// keeps a link named `SingletonLock` in the profile whose target ends in the holder's process number.
fn holder(profile: &Path) -> Option<i32> {
    let lock = std::fs::read_link(profile.join("SingletonLock")).ok()?;
    let pid = lock.to_string_lossy().rsplit('-').next()?.parse::<i32>().ok().filter(|pid| *pid > 1)?;
    let started_with = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let on_it = format!("--user-data-dir={}", profile.display());
    has_argument(&started_with, on_it.as_bytes()).then_some(pid)
}

/// Whether a process's command line has this argument, whole. A program just started has a zero byte between
/// its arguments; Chromium writes its own anew as one piece with spaces between them (as `ps` shows it), which
/// is why looking for the argument between zero bytes alone missed the very browser this is for (2026-11-22).
fn has_argument(command_line: &[u8], argument: &[u8]) -> bool {
    let edge = |b: Option<&u8>| b.is_none_or(|b| *b == 0 || *b == b' ');
    !argument.is_empty()
        && command_line.windows(argument.len()).enumerate().any(|(i, part)| {
            part == argument && (i == 0 || edge(command_line.get(i - 1))) && edge(command_line.get(i + argument.len()))
        })
}

/// A browser still running on the window's profile is an earlier copy's window that outlived its app (the app
/// was ended from outside while its browser was still closing: an update, a service stopped). A new browser on
/// that profile would only hand its page to the old one and end at once, which reads as "no window": the app
/// opened the default browser as well, two pages wanted the camera, and the app ended itself at its next start
/// (seen on a Linux computer, 2026-11-22). The profile holds nothing but this window, so whatever browser runs
/// on it is the app's own, and it is ended before the new one starts.
fn end_leftover(profile: &Path) {
    let Some(pid) = holder(profile) else { return };
    println!("Window: an earlier window's browser was still running on the app's profile: ending it first.");
    // SAFETY: a signal to a process number that `holder` just read as a browser started on this profile.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if holder(profile) != Some(pid) {
            return;
        }
    }
    // SAFETY: as above; it did not end when asked.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    std::thread::sleep(std::time::Duration::from_millis(300));
}

/// End the window's browser.
fn close(window: &Mutex<Child>) {
    let mut child = window.lock().unwrap_or_else(|e| e.into_inner());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_is_allowed_for_the_apps_own_addresses_and_nothing_else() {
        let own = ["http://127.0.0.1:8764".to_string(), "http://127.0.0.1:8766".to_string()];
        // a new profile
        let new = allowed(None, &own);
        let exceptions = &new["profile"]["content_settings"]["exceptions"];
        for kind in ["media_stream_camera", "media_stream_mic"] {
            assert_eq!(
                exceptions[kind],
                json!({"http://127.0.0.1:8764,*": {"setting": 1}, "http://127.0.0.1:8766,*": {"setting": 1}})
            );
        }
        // a profile that was used: what it remembers stays, what was allowed for other addresses goes
        let used = json!({"browser": {"window_placement": {"left": 10}},
            "profile": {"name": "x", "content_settings": {"exceptions": {
                "media_stream_camera": {"http://127.0.0.1:9000,*": {"setting": 1}, "https://example.org:443,*": {"setting": 1}},
                "notifications": {"https://example.org:443,*": {"setting": 2}}}}}});
        let again = allowed(Some(used), &own[..1]);
        assert_eq!(again["browser"]["window_placement"]["left"], 10);
        assert_eq!(again["profile"]["name"], "x");
        let exceptions = &again["profile"]["content_settings"]["exceptions"];
        assert_eq!(exceptions["media_stream_camera"], json!({"http://127.0.0.1:8764,*": {"setting": 1}}));
        assert_eq!(exceptions["media_stream_mic"], json!({"http://127.0.0.1:8764,*": {"setting": 1}}));
        assert_eq!(exceptions["notifications"], json!({"https://example.org:443,*": {"setting": 2}}));
        // settings that aren't what they should be are started anew
        assert!(allowed(Some(json!([1, 2])), &own)["profile"].is_object());
    }

    #[test]
    fn a_browser_left_running_on_the_windows_profile_is_ended_and_nothing_else_is() {
        let profile = std::env::temp_dir().join(format!("face-window-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        let lock = profile.join("SingletonLock");
        // no lock, a lock that names no process, a lock that names a process that isn't a browser on this
        // profile (this test itself): nothing to end
        assert_eq!(holder(&profile), None);
        std::os::unix::fs::symlink("somehost-notanumber", &lock).unwrap();
        assert_eq!(holder(&profile), None);
        std::fs::remove_file(&lock).unwrap();
        std::os::unix::fs::symlink(format!("somehost-{}", std::process::id()), &lock).unwrap();
        assert_eq!(holder(&profile), None, "a process that wasn't started on this profile is never touched");
        end_leftover(&profile);
        std::fs::remove_file(&lock).unwrap();
        // a stand-in for the browser: a process started with the profile as a browser is, which stays
        let on_it = format!("--user-data-dir={}", profile.display());
        let mut stays =
            Command::new("sh").args(["-c", "while :; do sleep 0.1; done", "browser", on_it.as_str()]).spawn().unwrap();
        std::os::unix::fs::symlink(format!("a-host-with-dashes-{}", stays.id()), &lock).unwrap();
        // (a new process has its parent's command line until its own program is loaded)
        for _ in 0..100 {
            if holder(&profile).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(holder(&profile), Some(stays.id() as i32));
        // one on another profile whose name only begins like this one's is not it
        let other = PathBuf::from(format!("{}-2", profile.display()));
        assert_eq!(holder(&other), None);
        end_leftover(&profile);
        let ended = stays.wait().unwrap();
        assert!(!ended.success(), "it was ended by the signal");
        assert_eq!(holder(&profile), None);
        // Chromium's own way of showing its command line: one piece, spaces between the arguments
        std::fs::remove_file(&lock).unwrap();
        let as_chromium = format!("/snap/chromium/1/chrome --password-store=basic {on_it} --app=http://127.0.0.1:8764");
        let mut stays =
            Command::new("sh").args(["-c", "while :; do sleep 0.1; done", as_chromium.as_str()]).spawn().unwrap();
        std::os::unix::fs::symlink(format!("somehost-{}", stays.id()), &lock).unwrap();
        for _ in 0..100 {
            if holder(&profile).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(holder(&profile), Some(stays.id() as i32), "found in a command line written as one piece");
        end_leftover(&profile);
        assert!(!stays.wait().unwrap().success());
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn an_argument_is_found_whole_in_either_kind_of_command_line() {
        let arg = b"--user-data-dir=/home/x/window";
        assert!(has_argument(b"chrome\0--user-data-dir=/home/x/window\0--app=x\0", arg));
        assert!(has_argument(b"chrome --flag --user-data-dir=/home/x/window --app=x", arg));
        assert!(has_argument(b"--user-data-dir=/home/x/window", arg));
        assert!(has_argument(b"chrome --user-data-dir=/home/x/window\0", arg));
        // another profile whose name begins the same, the argument inside another one, nothing at all
        assert!(!has_argument(b"chrome --user-data-dir=/home/x/window-2 --app=x", arg));
        assert!(!has_argument(b"chrome\0--note=--user-data-dir=/home/x/window\0", arg));
        assert!(!has_argument(b"", arg) && !has_argument(b"chrome", arg) && !has_argument(b"chrome x", b""));
    }

    #[test]
    fn a_snap_browser_keeps_the_windows_profile_where_it_may_write() {
        let home = Path::new("/home/x/.local/share/FaceLabels");
        assert_eq!(profile(Path::new("/usr/bin/no-such-browser"), home), home.join("window"));
        if std::env::var_os("HOME").is_some() {
            let mine = profile(Path::new("/snap/bin/chromium"), home);
            assert!(mine.parent().is_some_and(|folder| folder.ends_with("snap/chromium/common")));
            assert!(mine.file_name().is_some_and(|n| n.to_string_lossy().starts_with("face-recognizer-window-")));
            // the same folder every time, and another for another private folder
            assert_eq!(mine, profile(Path::new("/snap/bin/chromium"), home));
            assert_ne!(mine, profile(Path::new("/snap/bin/chromium"), Path::new("/tmp/check/home")));
        }
    }
}
