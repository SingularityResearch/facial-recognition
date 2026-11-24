//! The programs' local servers. `guard` and `http` are what every server shares (who may ask, and how an
//! answer leaves); `tracker` is the live tracker's routes; `store` and `paths` keep what the pages remember
//! in the private folder; `join` is how a device joins a shared database from a page; `camstream`
//! and `peer` are this device's camera for the owner's other devices.

// where things are kept and how files are written: shared with the library's crate
pub use face_files::{filejson, paths};
// every program switches the model runtime's telemetry off first thing (see there)
pub use face_core::runtime::no_telemetry;
// what the private folder keeps (the alert inbox, the presence log, settings and signals, the expression
// log): shared with the sync
pub use face_home::{alerts, exprlog, presence, store};

pub mod app;
pub mod camrelay;
pub mod camstream;
pub mod capture;
#[cfg(feature = "import")]
mod describe;
#[cfg(feature = "import")]
mod describejob;
pub mod desktop;
// what a device says of itself for the Signals chart: its processor, memory, warmth and battery
pub mod bus;
pub mod busrelay;
pub mod devicelive;
#[cfg(feature = "fetch")]
pub mod firstrun;
#[cfg(feature = "import")]
pub mod florence;
pub mod guard;
pub mod hardware;
pub mod http;
pub mod join;
pub mod label;
#[cfg(feature = "sync")]
mod label_settings;
#[cfg(feature = "import")]
pub mod listen;
#[cfg(feature = "import")]
mod lmsnet;
#[cfg(feature = "import")]
mod lmstudio;
pub mod location;
#[cfg(feature = "import")]
mod music;
#[cfg(feature = "import")]
mod musiclog;
pub mod pagefiles;
pub mod peer;
pub mod peerphoto;
mod persondata;
pub mod privacy;
pub mod recorder;
#[cfg(feature = "import")]
pub mod songlog;
#[cfg(feature = "sound")]
pub mod sound;
#[cfg(feature = "sound")]
pub mod soundheard;
#[cfg(feature = "import")]
pub mod soundlive;
#[cfg(feature = "import")]
mod spotify;
#[cfg(feature = "sync")]
pub mod syncer;
pub mod towns;
pub mod tracker;
pub mod trackerlink;
#[cfg(feature = "import")]
mod websearch;

use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};

static DEVICE: OnceLock<(String, String)> = OnceLock::new();

/// What this device is called and which system it runs, said by a shell that knows better than this
/// program can find out (a phone's). Before `start_tracker`; the first call counts.
pub fn set_device(name: &str, os: &str) {
    let _ = DEVICE.set((name.trim().to_string(), os.trim().to_string()));
}

/// (this device's name, its system), as the shared database's other devices see them.
pub(crate) fn device() -> (String, String) {
    DEVICE.get().cloned().unwrap_or_else(|| {
        let name = host_name()
            .or_else(|| ["COMPUTERNAME", "HOSTNAME"].iter().find_map(|v| std::env::var(v).ok()))
            .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
            .unwrap_or_default();
        let os = match std::env::consts::OS {
            "windows" => "Windows",
            "macos" => "Darwin",
            "linux" => "Linux",
            other => other,
        };
        (without_domain(&name), os.to_string())
    })
}

/// This computer's name as the system says it (the
/// name as it was set, in its own upper and lower case; the environment's COMPUTERNAME is all upper case).
#[cfg(windows)]
fn host_name() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW};
    let mut size = 0u32;
    // SAFETY: the first call only asks for the length (no buffer); the second writes at most `size` units into
    // a buffer of that length
    unsafe {
        GetComputerNameExW(ComputerNameDnsHostname, std::ptr::null_mut(), &mut size);
        let mut name = vec![0u16; size as usize];
        let ok = GetComputerNameExW(ComputerNameDnsHostname, name.as_mut_ptr(), &mut size);
        (ok != 0 && size > 0).then(|| String::from_utf16_lossy(&name[..size as usize]))
    }
}

#[cfg(unix)]
fn host_name() -> Option<String> {
    unsafe extern "C" {
        fn gethostname(name: *mut std::ffi::c_char, len: usize) -> std::ffi::c_int;
    }
    let mut name = [0u8; 256];
    // SAFETY: the system writes at most `len` bytes into the buffer handed to it
    let ok = unsafe { gethostname(name.as_mut_ptr().cast(), name.len() - 1) };
    let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
    (ok == 0 && end > 0).then(|| String::from_utf8_lossy(&name[..end]).into_owned())
}

#[cfg(not(any(windows, unix)))]
fn host_name() -> Option<String> {
    None
}

/// A computer's name without its domain (macOS says "name.local").
fn without_domain(name: &str) -> String {
    name.trim().split('.').next().unwrap_or("").to_string()
}

/// LM Studio on this computer kept as the owner wants it while the app runs: serving on all network
/// interfaces and announced on the local network, looked at every minute (`FACE_LMSTUDIO=off`: not at all).
#[cfg(feature = "import")]
pub fn keep_lmstudio(home: &Path) {
    lmstudio::keep(home);
}

/// One round of that upkeep now, and LM Studio's state afterwards as Settings shows it (for a check).
#[cfg(feature = "import")]
pub fn lmstudio_round(home: &Path) -> serde_json::Value {
    lmstudio::keep_once(home);
    lmstudio::status(home)
}

/// The live tracker started inside this process, on threads of its own: for an app whose window is the
/// system's web view (a phone, or a desktop shell), which then opens `http://127.0.0.1:<port>/`. `port` 0
/// takes any free one. Returns the port once the models are loaded and the server listens. Every few
/// seconds in which frames were read, a line says how many: numbers only.
pub fn start_tracker(root: &Path, home: &Path, pages: &Path, port: u16) -> Result<u16, String> {
    start_tracker_with(root, home, pages, port, None)
}

/// The same, with the Cameras page also served on a port of its own (`cameras`): the same pages and
/// answers from the same memory on a second address of 127.0.0.1, which gives that page a process of its
/// own in the app's window. A port that is taken leaves the page on the tracker's own port.
pub fn start_tracker_with(
    root: &Path,
    home: &Path,
    pages: &Path,
    port: u16,
    cameras: Option<u16>,
) -> Result<u16, String> {
    use face_core::{
        faces::Faces,
        runtime::{Choice, Runtime, find_library},
    };
    let library = find_library(root).ok_or("The model runtime's library wasn't found (FACE_ORT_LIB names it).")?;
    // the port first: a second copy stops here, before it loads a model
    let listener = http::bind(port).map_err(|e| format!("Port {port} is taken: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let began = std::time::Instant::now();
    // what the checks at load find is kept for the next start, for as long as this program, the model
    // runtime and the system are the same ones
    let made_for = format!(
        "{} {} | runtime {} bytes | {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        std::fs::metadata(&library).map_or(0, |m| m.len()),
        device().1
    );
    let runtime =
        Runtime::new(&library, Choice::from_env()).map_err(|e| e.0)?.keeping(&home.join("model-checks"), &made_for);
    let faces = Faces::load(&runtime, &root.join("models")).map_err(|e| e.0)?;
    let loaded = began.elapsed().as_secs_f64();
    let voice = face_core::voice::Voice::load(&runtime, &root.join("models"));
    let tracker = Arc::new(tracker::Tracker::new(faces, voice, root, home, pages));
    for (job, device) in tracker.devices() {
        println!("  {job}: {}", device.unwrap_or_else(|| "not installed".to_string()));
    }
    println!(
        "Models ready {:.1} s after the start (loaded after {loaded:.1} s, then each run once).",
        began.elapsed().as_secs_f64()
    );
    println!("Tracker: http://127.0.0.1:{port}");
    let also = cameras.and_then(|cams| match http::bind(cams) {
        Ok(second) => {
            trackerlink::set_cameras(cams);
            Some(second)
        }
        Err(_) => {
            println!("Face tracker: port {cams} is taken: the Cameras page stays on the tracker's own port");
            None
        }
    });
    let watched = tracker.clone();
    std::thread::Builder::new()
        .name("tracker".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return eprintln!("The tracker's server didn't start: {e}"),
            };
            let served = rt.block_on(async move {
                if let Some(second) = also {
                    tokio::spawn(http::serve(second, tracker.clone()));
                }
                http::serve(listener, tracker).await
            });
            if let Err(e) = served {
                eprintln!("The tracker's server stopped: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("tracker progress".into())
        .spawn(move || {
            const EVERY: u64 = 5;
            let mut before = 0;
            loop {
                std::thread::sleep(Duration::from_secs(EVERY));
                let (frames, faces, ms) = watched.progress();
                if frames > before {
                    let rate = (frames - before) as f64 / EVERY as f64;
                    println!(
                        "Tracker: {} frames read in {EVERY} s ({rate:.1} a second); the last one: {faces} face(s), {ms:.1} ms",
                        frames - before
                    );
                }
                before = frames;
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(port)
}

/// The labeling tool started inside this process, on a thread of its own. `port` 0 takes any free one.
/// Returns the port once the library is read and the server listens. `sync`: also keep the library and the
/// shared database the same, when one is set up (`HOME/database.json`).
pub fn start_labeler(root: &Path, home: &Path, pages: &Path, port: u16, sync: bool) -> Result<u16, String> {
    // the port first: a second copy stops here, before it reads the library
    let listener = http::bind(port).map_err(|e| format!("Port {port} is taken: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    println!("Loading faces ...");
    let labeler = Arc::new(label::Labeler::new(root, home, pages, &mut |line| println!("{line}"))?);
    let (faces, model) = labeler.opened();
    println!("Face Labels is open at http://127.0.0.1:{port}/  ({faces} faces, model {model})");
    labeler.start_reading(root);
    // the music log's listening: it does nothing until the owner switches the log on
    #[cfg(feature = "import")]
    labeler.start_music();
    if sync {
        #[cfg(feature = "sync")]
        labeler.start_sync(root, home);
    }
    std::thread::Builder::new()
        .name("labeling".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return eprintln!("The labeling tool's server didn't start: {e}"),
            };
            if let Err(e) = rt.block_on(http::serve(listener, labeler)) {
                eprintln!("The labeling tool's server stopped: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(port)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_computers_name_is_without_its_domain() {
        assert_eq!(super::without_domain(" studio.local\n"), "studio");
        assert_eq!(super::without_domain("pc"), "pc");
        assert_eq!(super::without_domain(""), "");
    }
}
