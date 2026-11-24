//! The devices of the shared database that this device has removed for itself (the owner's design, 2026-11-22:
//! "the settings let you remove devices that you dont want to see the data ONLY on your device. You should also
//! be able to add devices back as well - per device"; and: removing never changes the database's password or a
//! key). Kept on this device alone (`HOME/blocked-devices.json`), never synced: a device's settings never come
//! from a row.
//!
//! What it means on this device: it neither shows that device's camera nor asks it for a photo or a
//! description, and it answers none of that device's requests (its camera's stream, its readout, its photos).
//! What it can't do: that device still has the database's login, so it reads what every device shares through
//! the database, and a request it signed with another device's id is taken for that device's. The pages say so.

use std::path::Path;

use face_files::{filejson, paths::atomic_write};
use serde_json::{Value, json};

const FILE: &str = "blocked-devices.json";
/// more devices than anybody has: a list beyond it is a damaged file
const MOST: usize = 256;

/// A device's id as the shared database has it (a UUID's text): what a list may hold, whoever wrote it.
fn an_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
}

fn file(home: &Path) -> Value {
    std::fs::read(home.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

fn ids_of(v: &Value) -> Vec<String> {
    let listed = v.get("ids").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    listed.iter().filter_map(Value::as_str).filter(|id| an_id(id)).take(MOST).map(str::to_string).collect()
}

fn a_sha(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// (removed device's id, the SHA-256 of the certificate it had when it was removed), for those that had one.
fn certs_of(v: &Value) -> Vec<(String, String)> {
    let listed = v.get("certs").and_then(Value::as_object);
    let removed = ids_of(v);
    (listed.into_iter().flatten())
        .filter_map(|(id, sha)| Some((id.clone(), sha.as_str()?.to_string())))
        .filter(|(id, sha)| removed.contains(id) && a_sha(sha))
        .take(MOST)
        .collect()
}

/// The ids of the devices this device has removed for itself.
pub fn ids(home: &Path) -> Vec<String> {
    ids_of(&file(home))
}

/// The certificates (their SHA-256) of the devices removed here, as each had it when it was removed: a
/// device is removed as the machine it is, not only under the id it had then. Whoever has the shared
/// database's login can write a row with a new id, and a certificate removed here is answered under none.
pub fn certs(home: &Path) -> Vec<String> {
    certs_of(&file(home)).into_iter().map(|(_, sha)| sha).collect()
}

/// Whether this device has removed that one for itself.
pub fn is(home: &Path, device: &str) -> bool {
    ids(home).iter().any(|id| id == device)
}

/// Remove a device for this one (`blocked`), or add it back. The list afterwards.
pub fn set(home: &Path, device: &str, blocked: bool) -> Result<Vec<String>, String> {
    set_with(home, device, blocked, None)
}

/// The same, with the certificate (its SHA-256) the device has now, where this device knows it: kept with the
/// removal, so that the device stays removed under any id it shows that certificate with. Added back, the
/// device's certificate is forgotten with it.
pub fn set_with(home: &Path, device: &str, blocked: bool, cert_sha256: Option<&str>) -> Result<Vec<String>, String> {
    if !an_id(device) {
        return Err("That is not a device of the shared database.".to_string());
    }
    let before = file(home);
    let (mut list, mut certs) = (ids_of(&before), certs_of(&before));
    list.retain(|id| id != device);
    certs.retain(|(id, _)| id != device);
    if blocked {
        if list.len() >= MOST {
            return Err("Too many devices are removed on this one.".to_string());
        }
        list.push(device.to_string());
        if let Some(sha) = cert_sha256.filter(|sha| a_sha(sha)) {
            certs.push((device.to_string(), sha.to_string()));
        }
    }
    let certs: serde_json::Map<String, Value> = certs.into_iter().map(|(id, sha)| (id, json!(sha))).collect();
    let kept = if certs.is_empty() { json!({"ids": list}) } else { json!({"ids": list, "certs": certs}) };
    atomic_write(&home.join(FILE), filejson::dumps(&kept).as_bytes())
        .map_err(|e| format!("can't keep the list of removed devices: {e}"))?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
    const B: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

    #[test]
    fn a_device_is_removed_on_this_one_and_added_back() {
        let home = std::env::temp_dir().join(format!("face-home-blocked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(ids(&home).is_empty() && !is(&home, A));
        assert_eq!(set(&home, A, true).unwrap(), [A]);
        assert_eq!(set(&home, A, true).unwrap(), [A], "once");
        assert_eq!(set(&home, B, true).unwrap(), [A, B]);
        assert!(is(&home, A) && is(&home, B));
        assert_eq!(set(&home, A, false).unwrap(), [B]);
        assert!(!is(&home, A) && is(&home, B));
        assert_eq!(set(&home, A, false).unwrap(), [B], "adding back what isn't removed changes nothing");
        // what isn't a device's id is never kept, whoever asks
        for bad in ["", "abc", "../../x", &A.to_uppercase(), &A.replace('-', "_"), &format!("{A}0")] {
            assert!(set(&home, bad, true).is_err(), "taken: {bad}");
        }
        // a file someone else wrote: only ids are read from it
        std::fs::write(home.join(FILE), json!({"ids": [A, 7, "x", null, "../y", B]}).to_string()).unwrap();
        assert_eq!(ids(&home), [A, B]);
        std::fs::write(home.join(FILE), b"not json").unwrap();
        assert!(ids(&home).is_empty());
        std::fs::write(home.join(FILE), json!({"ids": A}).to_string()).unwrap();
        assert!(ids(&home).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[cfg(test)]
mod cert_tests {
    use super::*;

    const A: &str = "11111111-2222-4333-8444-555555555555";
    const B: &str = "99999999-2222-4333-8444-555555555555";

    #[test]
    fn a_device_is_removed_with_the_certificate_it_had_and_added_back_without_it() {
        let home = std::env::temp_dir().join(format!("face-home-blocked-certs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let (one, two) = ("ab".repeat(32), "cd".repeat(32));
        assert!(certs(&home).is_empty());
        set_with(&home, A, true, Some(&one)).unwrap();
        set(&home, B, true).unwrap(); // (one whose certificate this device never knew)
        assert_eq!(ids(&home), [A, B]);
        assert_eq!(certs(&home), [one.clone()]);
        // removed again with the certificate it has now: the newer one counts
        set_with(&home, A, true, Some(&two)).unwrap();
        assert_eq!(certs(&home), [two.clone()]);
        // what isn't a certificate's number is not kept as one, and the device is removed all the same
        set_with(&home, B, true, Some("not a number")).unwrap();
        assert_eq!((ids(&home).len(), certs(&home)), (2, vec![two.clone()]));
        // added back: its certificate is forgotten with it
        set(&home, A, false).unwrap();
        assert_eq!(ids(&home), [B]);
        assert!(certs(&home).is_empty());
        // a file that names a certificate for a device that isn't removed gives no certificate
        std::fs::write(home.join(FILE), json!({"ids": [B], "certs": {A: one, B: 7}}).to_string()).unwrap();
        assert!(certs(&home).is_empty());
        let _ = std::fs::remove_dir_all(home);
    }
}
