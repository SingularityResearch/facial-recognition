//! Which version of the app ran on this device before this one, for the setup page (the owner's words,
//! 2026-11-25: "this is for users who have just freshly installed the application, or for users who are upgrading
//! and need to setup new options. Keep their existing setup where possible - if not notify them about what will
//! change and how"). `HOME/app-version.json`: {version, previous, fresh}. `fresh`: this device's private folder was
//! empty when the app first started (a new copy); `previous`: the version that ran before this one ("older" for a
//! device whose app is from before the versions were noted), kept until the version changes again.

use std::path::Path;

use serde_json::{Value, json};

const FILE: &str = "app-version.json";

/// Notes this start's version; first thing at the app's start, before it writes anything of its own in the
/// private folder (an empty or missing folder is what tells a new copy).
pub fn note(home: &Path) {
    let now = env!("CARGO_PKG_VERSION");
    let kept: Option<Value> = std::fs::read(home.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let noted = match kept {
        Some(k) if k["version"] == now => return,
        // a version before this one: the one it was
        Some(k) => json!({"version": now, "previous": k["version"].clone(), "fresh": false}),
        // none noted: a new copy where the folder is empty, else a device whose app is older than this note
        None => {
            // (what a start writes before this can be asked, its log and its lock, doesn't count)
            const STARTS_WITH: [&str; 2] = ["app.log", "locks"];
            let had = std::fs::read_dir(home)
                .map(|d| d.flatten().any(|e| !STARTS_WITH.contains(&e.file_name().to_string_lossy().as_ref())))
                .unwrap_or(false);
            json!({"version": now, "previous": if had { json!("older") } else { Value::Null }, "fresh": !had})
        }
    };
    let _ = std::fs::create_dir_all(home);
    let _ = face_files::paths::atomic_write(&home.join(FILE), noted.to_string().as_bytes());
}

/// What was noted, for the setup page.
pub fn read(home: &Path) -> Value {
    std::fs::read(home.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .unwrap_or_else(|| json!({"version": env!("CARGO_PKG_VERSION"), "previous": null, "fresh": false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_copy_an_update_and_a_device_from_before_the_note_are_told_apart() {
        let dir = std::env::temp_dir().join(format!("face-version-note-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = env!("CARGO_PKG_VERSION");
        // a new copy: no private folder yet
        let new = dir.join("new");
        note(&new);
        assert_eq!(read(&new), json!({"version": now, "previous": null, "fresh": true}));
        // a new copy whose start wrote its log and its lock first is new all the same
        let started = dir.join("started");
        std::fs::create_dir_all(started.join("locks")).unwrap();
        std::fs::write(started.join("app.log"), "a start\n").unwrap();
        note(&started);
        assert_eq!(read(&started)["fresh"], true);
        note(&new);
        assert_eq!(read(&new)["fresh"], true, "the same version again changes nothing");
        // a device whose app is older than the note: its folder has its files
        let old = dir.join("old");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        note(&old);
        assert_eq!(read(&old), json!({"version": now, "previous": "older", "fresh": false}));
        // an update from a version noted before
        std::fs::write(old.join(FILE), r#"{"version": "2.6.0", "previous": "older", "fresh": false}"#).unwrap();
        note(&old);
        assert_eq!(read(&old), json!({"version": now, "previous": "2.6.0", "fresh": false}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
