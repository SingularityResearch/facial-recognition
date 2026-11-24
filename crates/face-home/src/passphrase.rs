//! The owner's passphrase for keys as this device holds it, for the two things that need it without the sync's
//! own key handling (`face-sync` `keys.rs`, which writes the file and checks a passphrase against the
//! database): a device that lets another one in hands it on, and a device that was just let in keeps what it
//! was handed until its sync has looked at it. Kept protected for this user (`face_files::protect`).
//! Nothing here reaches a page or a log.

use std::path::Path;

use face_files::{
    filejson,
    protect::{protect, unprotect, write_private},
};
use serde_json::{Value, json};

/// The sync's own file: the passphrase, its key and which version it is (`keys.rs`).
pub const KEY_FILE: &str = "secret.key";
/// What the device that let this one in handed over, until the sync took it or found it isn't the current one.
/// A phone has no such sync: there it stays, and is what the phone hands on.
const RECEIVED: &str = "passphrase-received.json";

fn kept(home: &Path, file: &str) -> Option<String> {
    let v: Value = serde_json::from_slice(&std::fs::read(home.join(file)).ok()?).ok()?;
    unprotect(v.get("passphrase")?.as_str()?).ok().filter(|p| !p.is_empty())
}

/// The passphrase the sync keeps on this device ("" when it has none, or it can't be read here).
pub fn saved(home: &Path) -> String {
    kept(home, KEY_FILE).unwrap_or_default()
}

/// Keep the passphrase another device handed this one when it let it in.
pub fn receive(home: &Path, passphrase: &str) -> Result<(), String> {
    let text = protect(passphrase).map_err(|e| format!("the passphrase can't be kept here: {e}"))?;
    write_private(&home.join(RECEIVED), filejson::dumps_lines(&json!({"passphrase": text}), true, 1).as_bytes())
        .map_err(|e| format!("the passphrase can't be kept here: {e}"))
}

/// The passphrase another device handed this one, when nothing took it yet.
pub fn received(home: &Path) -> Option<String> {
    kept(home, RECEIVED)
}

/// The sync took the handed passphrase, or found that it isn't the current one.
pub fn drop_received(home: &Path) {
    let _ = std::fs::remove_file(home.join(RECEIVED));
}

/// What this device hands a device it lets in: the passphrase its sync holds, else the one it was handed
/// itself. The device that takes it checks it against the database before keeping it.
pub fn to_hand_on(home: &Path) -> Option<String> {
    kept(home, KEY_FILE).or_else(|| received(home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handed_passphrase_is_kept_protected_until_it_is_dropped() {
        let home = std::env::temp_dir().join(format!("face-home-passphrase-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!((saved(&home), received(&home), to_hand_on(&home)), (String::new(), None, None));
        receive(&home, "the owner's own passphrase").unwrap();
        let on_disk = std::fs::read_to_string(home.join(RECEIVED)).unwrap();
        assert!(!on_disk.contains("own passphrase"), "written as it is");
        assert_eq!(received(&home).as_deref(), Some("the owner's own passphrase"));
        assert_eq!(to_hand_on(&home).as_deref(), Some("the owner's own passphrase"));
        // the sync's own copy comes first
        let text = protect("the one the sync holds").unwrap();
        std::fs::write(home.join(KEY_FILE), json!({"passphrase": text, "version": 3}).to_string()).unwrap();
        assert_eq!(saved(&home), "the one the sync holds");
        assert_eq!(to_hand_on(&home).as_deref(), Some("the one the sync holds"));
        drop_received(&home);
        drop_received(&home);
        assert_eq!(received(&home), None);
        // a file that isn't one of this app's says nothing
        std::fs::write(home.join(KEY_FILE), b"not json").unwrap();
        assert_eq!((saved(&home), to_hand_on(&home)), (String::new(), None));
        std::fs::write(home.join(KEY_FILE), json!({"passphrase": 7}).to_string()).unwrap();
        assert_eq!(saved(&home), "");
        let _ = std::fs::remove_dir_all(&home);
    }
}
