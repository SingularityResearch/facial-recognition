//! What a system's own shell (an iOS or Android app around a web view) calls to start the programs inside
//! the app: two functions with C's calling rules, since that is what Swift and Kotlin can call.

use std::{
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    sync::Mutex,
};

static LAST_ERROR: Mutex<Option<CString>> = Mutex::new(None);

fn text(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller hands over a text that ends in a zero byte and stays alive during the call
    Some(PathBuf::from(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()))
}

/// Start the live tracker inside this app and return its port on 127.0.0.1, or -1 (then `face_app_error`
/// says why). `root` holds `models/` (and `data/`), `home` is the app's private folder, `pages` the page
/// files. Called once, when the app starts.
#[unsafe(no_mangle)]
pub extern "C" fn face_app_start(root: *const c_char, home: *const c_char, pages: *const c_char) -> i32 {
    face_server::no_telemetry();
    let started = match (text(root), text(home), text(pages)) {
        (Some(root), Some(home), Some(pages)) => face_server::start_tracker(&root, &home, &pages, 0),
        _ => Err("the app gave no folders".to_string()),
    };
    match started {
        Ok(port) => port as i32,
        Err(e) => {
            eprintln!("The tracker didn't start: {e}");
            *LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()) = CString::new(e.replace('\0', " ")).ok();
            -1
        }
    }
}

/// What this device is called and which system it runs ("iOS 26.1"), for the shared database's list of
/// devices: the shell knows, this library can't find out. Before `face_app_start`.
/// What only the system's own interfaces give the app's shell, said every few seconds for the Signals chart:
/// the heat state (0 nominal, 1 fair, 2 serious, 3 critical; anything else: not known), the battery in percent
/// (outside 0 to 100: not known) and whether the phone is on power (0 no, 1 yes; anything else: not known).
#[unsafe(no_mangle)]
pub extern "C" fn face_app_health(heat: i32, battery: f32, on_power: i32) {
    face_server::devicelive::told(heat, battery, on_power);
}

#[unsafe(no_mangle)]
pub extern "C" fn face_app_device(name: *const c_char, os: *const c_char) {
    if let (Some(name), Some(os)) = (text(name), text(os)) {
        face_server::set_device(&name.to_string_lossy(), &os.to_string_lossy());
    }
}

/// Why `face_app_start` returned -1: a text that stays valid until the next failed start, or null.
#[unsafe(no_mangle)]
pub extern "C" fn face_app_error() -> *const c_char {
    LAST_ERROR.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map_or(std::ptr::null(), |e| e.as_ptr())
}

#[cfg(target_os = "android")]
mod android;
