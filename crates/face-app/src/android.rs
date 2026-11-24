//! The Android shell's way in (apps/android: `io.github.singularityresearch.facerecognizer.Native`). Java can call native code only
//! through JNI, with these names.

use std::{ffi::CString, os::fd::AsRawFd};

use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::{jint, jstring},
};

fn text(env: &mut JNIEnv, s: &JString) -> Option<CString> {
    let s: String = env.get_string(s).ok()?.into();
    CString::new(s).ok()
}

/// `Native.start(root, home, pages, runtime)`: the tracker's port, or -1. `runtime` is the model runtime's
/// library as Android unpacked it beside this one. What the programs print goes to `<home>/app.log`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_singularityresearch_facerecognizer_Native_start<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    root: JString<'l>,
    home: JString<'l>,
    pages: JString<'l>,
    runtime: JString<'l>,
) -> jint {
    face_server::no_telemetry();
    let (Some(root), Some(home), Some(pages), Some(runtime)) =
        (text(&mut env, &root), text(&mut env, &home), text(&mut env, &pages), text(&mut env, &runtime))
    else {
        return -1;
    };
    let home_dir = std::path::PathBuf::from(home.to_string_lossy().into_owned());
    let _ = std::fs::create_dir_all(&home_dir);
    if let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open(home_dir.join("app.log")) {
        // SAFETY: both are open files of this process; standard output and error are given the log file
        unsafe {
            libc::dup2(log.as_raw_fd(), 1);
            libc::dup2(log.as_raw_fd(), 2);
        }
    }
    // For measuring on a phone, where a program can't be given an environment from outside: a number in
    // `<home>/threads.txt` is the thread count a model's run uses (`FACE_THREADS`).
    let threads =
        std::fs::read_to_string(home_dir.join("threads.txt")).ok().and_then(|t| t.trim().parse::<usize>().ok());
    // SAFETY: set once, before any other thread of the library runs
    unsafe {
        std::env::set_var("FACE_ORT_LIB", runtime.to_string_lossy().as_ref());
        if let Some(n) = threads {
            std::env::set_var("FACE_THREADS", n.to_string());
        }
    }
    crate::face_app_start(root.as_ptr(), home.as_ptr(), pages.as_ptr())
}

/// `Native.device(name, os)`: what the phone is called and its system ("Android 16"), before `start`.
/// What the system's own interfaces give the app, said every few seconds for the Signals chart (as
/// `face_app_health`): the heat state 0 to 3, the battery in percent, on power 0 or 1; anything else: not known.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_singularityresearch_facerecognizer_Native_health<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    heat: jint,
    battery: jni::sys::jfloat,
    on_power: jint,
) {
    face_server::devicelive::told(heat, battery, on_power);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_singularityresearch_facerecognizer_Native_device<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    name: JString<'l>,
    os: JString<'l>,
) {
    if let (Some(name), Some(os)) = (text(&mut env, &name), text(&mut env, &os)) {
        crate::face_app_device(name.as_ptr(), os.as_ptr());
    }
}

/// `Native.locationWanted()`: whether the phone's location service is to be asked now (as
/// `face_app_location_wanted`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_singularityresearch_facerecognizer_Native_locationWanted<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
) -> jni::sys::jboolean {
    u8::from(crate::face_app_location_wanted() != 0)
}

/// `Native.location(lat, lon, accuracy)`: where the system says the phone is (as `face_app_location`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_singularityresearch_facerecognizer_Native_location<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    lat: jni::sys::jdouble,
    lon: jni::sys::jdouble,
    accuracy: jni::sys::jfloat,
) {
    crate::face_app_location(lat, lon, f64::from(accuracy));
}

/// `Native.locationRefused(why)`: the system gives no location, and why (as `face_app_location_refused`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_singularityresearch_facerecognizer_Native_locationRefused<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    why: jint,
) {
    crate::face_app_location_refused(why);
}

/// `Native.error()`: why `start` returned -1, or null.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_singularityresearch_facerecognizer_Native_error<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
) -> jstring {
    let why = crate::face_app_error();
    if why.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: a text ending in a zero byte that stays valid until the next failed start
    let why = unsafe { std::ffi::CStr::from_ptr(why) }.to_string_lossy().into_owned();
    env.new_string(why).map_or(std::ptr::null_mut(), |s| s.into_raw())
}
