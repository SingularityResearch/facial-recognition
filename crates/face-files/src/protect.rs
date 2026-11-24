//! What is kept for this user alone (`protect`, `unprotect` and `write_private`):
//! a key or a password as text safe to keep in a file ("dpapi:..." on Windows, which only this Windows user
//! can read back; "plain:<base64>" elsewhere, in a file that is the user's alone), and such a file written.

use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

type Result<T> = std::result::Result<T, String>;

/// The text `protect` kept; an error for anything else, which says nothing of what was saved.
pub fn unprotect(stored: &str) -> Result<String> {
    let bad = || "What was saved can't be read on this device.".to_string();
    let (kind, b64) = stored.split_once(':').ok_or_else(bad)?;
    let raw = B64.decode(b64).map_err(|_| bad())?;
    let plain = match kind {
        "plain" => raw,
        "dpapi" => dpapi(&raw).ok_or_else(bad)?,
        _ => return Err(bad()),
    };
    String::from_utf8(plain).map_err(|_| bad())
}

#[cfg(windows)]
fn dpapi(data: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData},
    };
    let src = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: `src` points at `data` for the call; `out` is filled by Windows with memory it allocated, which
    // is copied and handed back with LocalFree
    unsafe {
        let ok = CryptUnprotectData(
            &src,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        );
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let plain = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Some(plain)
    }
}

#[cfg(not(windows))]
fn dpapi(_: &[u8]) -> Option<Vec<u8>> {
    None
}

/// A key or a password as text safe to keep in a file for this user.
pub fn protect(text: &str) -> Result<String> {
    if cfg!(windows) {
        let sealed = dpapi_protect(text.as_bytes()).ok_or("Windows couldn't protect what is to be saved.")?;
        Ok(format!("dpapi:{}", B64.encode(sealed)))
    } else {
        Ok(format!("plain:{}", B64.encode(text)))
    }
}

#[cfg(windows)]
fn dpapi_protect(data: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData},
    };
    let src = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: as in `dpapi`: `src` points at `data` for the call, `out` is Windows' own memory, copied and freed
    unsafe {
        let ok = CryptProtectData(
            &src,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        );
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let sealed = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Some(sealed)
    }
}

#[cfg(not(windows))]
fn dpapi_protect(_: &[u8]) -> Option<Vec<u8>> {
    None
}

/// Write a file that is this user's alone from its first byte (mode 600 outside Windows, where the private
/// folder is per user), without ever leaving a half-written one.
pub fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let e = |e: std::io::Error| format!("can't write {}: {e}", path.file_name().unwrap_or_default().to_string_lossy());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(e)?;
    }
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp); // a leftover could have other rights
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&tmp).and_then(|mut f| f.write_all(data)).map_err(e)?;
    std::fs::rename(&tmp, path).map_err(e)
}

/// A saved key read back: `protect`'s form, or an older plain one as it is.
pub fn reveal(stored: &str) -> Result<String> {
    let stored = stored.trim();
    if is_protected(stored) { unprotect(stored) } else { Ok(stored.to_string()) }
}

pub fn is_protected(stored: &str) -> bool {
    matches!(stored.split_once(':'), Some(("dpapi" | "plain", _)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_kept_is_protected_and_read_back() {
        let kept = protect("the pass word").unwrap();
        assert!(!kept.contains("pass word") && is_protected(&kept));
        assert!(kept.starts_with(if cfg!(windows) { "dpapi:" } else { "plain:" }));
        assert_eq!(unprotect(&kept).unwrap(), "the pass word");
        assert_eq!(reveal(&format!(" {kept}\n")).unwrap(), "the pass word");
        // an older key, saved as it is
        assert_eq!(reveal(" AIzaOlderKey ").unwrap(), "AIzaOlderKey");
        assert!(!is_protected("AIzaOlderKey") && !is_protected("https://x"));
        assert_eq!(unprotect("plain:c2VjcmV0").unwrap(), "secret");
        for bad in ["", "secret", "plain:###", "other:c2VjcmV0"] {
            assert!(!unprotect(bad).unwrap_err().contains("secret"), "the message says nothing of what was saved");
        }
    }

    #[test]
    fn a_private_file_is_written_whole() {
        let dir = std::env::temp_dir().join(format!("face-files-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("deep").join("a.key");
        write_private(&file, b"one").unwrap();
        write_private(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(dir.join("deep")).unwrap().count(), 1, "nothing left beside it");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
