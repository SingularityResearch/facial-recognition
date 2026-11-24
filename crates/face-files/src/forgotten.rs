//! The names of forgotten people: `HOME/forgotten.json`, a list of names.
//!
//! When the owner forgets a person and there is a shared database, their name is kept here: the instruction
//! for the devices that haven't forgotten yet. While a name is here, what arrives under it is refused. Giving
//! the name to someone again (naming a face, a rename) takes it out, so a device joining later doesn't forget
//! that new person.

use std::{collections::BTreeSet, io, path::Path, sync::Mutex};

use serde_json::Value;

use crate::{filejson, paths::atomic_write};

pub const FILE: &str = "forgotten.json";
static LOCK: Mutex<()> = Mutex::new(());

/// Names forgotten here or on another device, and not given to anyone since. `strict` (the sync): a file that
/// is there but can't be read is an error instead of "no names", which the sync would send on as every forget
/// withdrawn.
pub fn names(home: &Path, strict: bool) -> io::Result<BTreeSet<String>> {
    let said = |why: String| {
        if strict { Err(io::Error::other(format!("can't read {FILE}: {why}"))) } else { Ok(BTreeSet::new()) }
    };
    match std::fs::read(home.join(FILE)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => said(e.to_string()),
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Array(list)) => Ok(list.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()),
            Ok(Value::Object(map)) => Ok(map.keys().cloned().collect()), // (an object's keys count as its names)
            Ok(Value::String(_)) => Ok(BTreeSet::new()),
            Ok(_) => said("not a list of names".to_string()),
            Err(e) => said(e.to_string()),
        },
    }
}

fn write(home: &Path, kept: &BTreeSet<String>) -> io::Result<()> {
    let list = Value::Array(kept.iter().cloned().map(Value::String).collect());
    atomic_write(&home.join(FILE), filejson::dumps(&list).as_bytes())
}

/// A person was forgotten: the forget goes to the other devices with the next sync.
pub fn note(home: &Path, name: &str) -> io::Result<()> {
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut kept = names(home, false)?;
    kept.insert(name.to_string());
    write(home, &kept)
}

/// The name was given to someone again, or another device said so: the earlier forget no longer applies.
/// True if it was forgotten.
pub fn unforget(home: &Path, name: &str) -> io::Result<bool> {
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut kept = names(home, false)?;
    if !kept.remove(name) {
        return Ok(false);
    }
    write(home, &kept)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_noted_and_taken_out_again_and_the_file_has_its_fixed_spelling() {
        let home = std::env::temp_dir().join(format!("face-forgotten-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert!(names(&home, true).unwrap().is_empty());
        assert!(!unforget(&home, "Ann").unwrap());
        assert!(!home.join(FILE).exists(), "nothing to take out: nothing written");
        note(&home, "Zo\u{eb}").unwrap();
        note(&home, "Ann").unwrap();
        note(&home, "Ann").unwrap();
        assert_eq!(std::fs::read_to_string(home.join(FILE)).unwrap(), "[\"Ann\", \"Zo\\u00eb\"]");
        assert!(unforget(&home, "Ann").unwrap() && !unforget(&home, "Ann").unwrap());
        assert_eq!(names(&home, true).unwrap().into_iter().collect::<Vec<_>>(), ["Zo\u{eb}"]);
        // a file that can't be read: no names for those who only ask, an error for the sync
        std::fs::write(home.join(FILE), b"{ not json").unwrap();
        assert!(names(&home, false).unwrap().is_empty());
        assert!(names(&home, true).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
