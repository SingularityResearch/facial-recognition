//! What the app asks of the operating system: show a folder in the
//! system's file manager, open a link in the owner's own browser, and whether the screen is locked.

use std::{
    path::Path,
    process::{Command, Stdio},
};

fn run(mut ask: Command) {
    let _ = ask.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Show a folder in the system's file manager.
pub fn open_folder(folder: &Path) {
    let mut ask = Command::new(if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    });
    ask.arg(folder);
    run(ask);
}

/// Some(true) while this desktop session's screen is locked, Some(false) while it isn't, None where that
/// can't be told (macOS, Linux, or Windows didn't say). On Windows a lock stops every running camera about
/// 20 s later, and every Remote Desktop disconnect locks the session.
pub fn screen_locked() -> Option<bool> {
    #[cfg(windows)]
    {
        session_locked(&session_info()?)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Locked or not, from the start of what Windows says of a session (WTSINFOEXW): its level (4 bytes, then 4
/// unused), and level 1's session number, state and flags (0 locked, 1 unlocked, anything else unknown).
pub fn session_locked(info: &[u8]) -> Option<bool> {
    let number = |at: usize| info.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if info.len() < 20 || number(0) != Some(1) {
        return None;
    }
    match number(16) {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

#[cfg(windows)]
fn session_info() -> Option<Vec<u8>> {
    use windows_sys::Win32::System::RemoteDesktop::{
        WTS_CURRENT_SESSION, WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx,
    };
    let (mut told, mut length) = (std::ptr::null_mut::<u16>(), 0u32);
    // SAFETY: this computer, this session; the system gives a block of `length` bytes, copied and given back
    unsafe {
        if WTSQuerySessionInformationW(
            std::ptr::null_mut(),
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut told,
            &mut length,
        ) == 0
            || told.is_null()
        {
            return None;
        }
        let bytes = std::slice::from_raw_parts(told.cast::<u8>(), (length as usize).min(20)).to_vec();
        WTSFreeMemory(told.cast());
        Some(bytes)
    }
}

/// Open an address in the owner's default browser.
pub fn open_url(url: &str) {
    let mut ask = if cfg!(windows) {
        let mut ask = Command::new("rundll32");
        ask.args(["url.dll,FileProtocolHandler", url]);
        ask
    } else {
        let mut ask = Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
        ask.arg(url);
        ask
    };
    ask.stdin(Stdio::null());
    run(ask);
}
