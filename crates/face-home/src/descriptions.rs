//! Photo descriptions: `HOME/descriptions.json`, one entry a
//! photo (or a video's frame, `hash:frame`), written whole with a line for each.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use face_files::{filejson, paths::atomic_write};
use serde_json::{Map, Value};

pub const FILE: &str = "descriptions.json";
static LOCK: Mutex<()> = Mutex::new(());

/// One reader-and-writer of the file at a time in this process: hold it from `load` to `write`.
pub fn hold() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The descriptions as they were read: the file is several MB in a large library, and Describe all asks and
/// adds one photo at a time. By private folder: a program has one, a test several at once.
static KEPT: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// A file's time and size: it is read anew when they change.
type Stamp = Option<(std::time::SystemTime, u64)>;

struct Kept {
    home: PathBuf,
    all: Map<String, Value>,
    /// the file as it was when `all` was read from it or written to it
    stamp: Stamp,
    /// descriptions kept here that the file doesn't have yet (`save` without writing, until `flush`)
    unwritten: bool,
}

fn stamp(home: &Path) -> Stamp {
    let file = std::fs::metadata(home.join(FILE)).ok()?;
    Some((file.modified().ok()?, file.len()))
}

fn read(home: &Path) -> Result<Map<String, Value>, String> {
    match std::fs::read(home.join(FILE)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("can't read {FILE}: {e}")),
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(m)) => Ok(m),
            Ok(_) => Ok(Map::new()),
            Err(e) => Err(format!("can't read {FILE}: {e}")),
        },
    }
}

/// Something done with what is kept of a private folder's descriptions: read once, and again when the file
/// changed under it (another program wrote it) while nothing here waits to be written.
fn kept<R>(home: &Path, f: impl FnOnce(&mut Kept) -> R) -> Result<R, String> {
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let now = stamp(home);
    let at = match kept.iter().position(|k| k.home == home) {
        Some(at) => {
            if !kept[at].unwritten && kept[at].stamp != now {
                kept[at].all = read(home)?;
                kept[at].stamp = now;
            }
            at
        }
        None => {
            kept.push(Kept { home: home.to_path_buf(), all: read(home)?, stamp: now, unwritten: false });
            kept.len() - 1
        }
    };
    Ok(f(&mut kept[at]))
}

/// Something done with every description.
fn with<R>(home: &Path, f: impl FnOnce(&mut Map<String, Value>) -> R) -> Result<R, String> {
    kept(home, |k| f(&mut k.all))
}

fn put(k: &mut Kept) -> Result<(), String> {
    atomic_write(&k.home.join(FILE), filejson::rows_of(&k.all, false).as_bytes())
        .map_err(|e| format!("can't write {FILE}: {e}"))?;
    (k.stamp, k.unwritten) = (stamp(&k.home), false);
    Ok(())
}

/// Every description. A file that is there but can't be read is an error, never taken as empty: it would be
/// written back empty.
pub fn load(home: &Path) -> Result<Map<String, Value>, String> {
    with(home, |all| all.clone())
}

pub fn write(home: &Path, all: &Map<String, Value>) -> Result<(), String> {
    kept(home, |k| {
        k.all = all.clone();
        put(k)
    })?
}

/// A photo's (or a video frame's) saved description.
pub fn saved(home: &Path, key: &str) -> Result<Option<Value>, String> {
    with(home, |all| all.get(key).cloned())
}

/// Keep a description, with the time it was made; `now`: also write the file (Describe all writes every 25
/// photos, not after each: `flush`).
pub fn save(home: &Path, key: &str, description: &Value, now: bool) -> Result<Value, String> {
    let _one = hold();
    let mut made = description.as_object().cloned().unwrap_or_default();
    made.insert("when".to_string(), Value::String(face_files::paths::utc_now()));
    let made = Value::Object(made);
    kept(home, |k| {
        k.all.insert(key.to_string(), made.clone());
        if now {
            return put(k);
        }
        k.unwritten = true;
        Ok(())
    })??;
    Ok(made)
}

/// Write what `save` kept without writing.
pub fn flush(home: &Path) -> Result<(), String> {
    let _one = hold();
    kept(home, |k| if k.unwritten { put(k) } else { Ok(()) })?
}

pub fn count(home: &Path) -> Result<usize, String> {
    with(home, |all| all.len())
}

/// How long one description takes with a model, from this device's saved ones: the median seconds of its
/// newest 200 (none with fewer than 5). For saying how long describing many will take.
pub fn typical_secs(home: &Path, model: &str) -> Result<Option<f64>, String> {
    let mut rows: Vec<(String, f64)> = with(home, |all| {
        (all.values())
            .filter(|d| d.get("model").and_then(Value::as_str) == Some(model))
            .filter_map(|d| {
                Some((
                    d.get("when").and_then(Value::as_str).unwrap_or("").to_string(),
                    d.get("secs")?.as_f64().filter(|s| *s > 0.0)?,
                ))
            })
            .collect()
    })?;
    if model.is_empty() || rows.len() < 5 {
        return Ok(None);
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut secs: Vec<f64> = rows[rows.len().saturating_sub(200)..].iter().map(|r| r.1).collect();
    secs.sort_by(f64::total_cmp);
    Ok(Some(filejson::round(secs[secs.len() / 2], 1)))
}

/// {photo: when its saved description was made (UTC text)}.
pub fn whens(home: &Path) -> Result<Map<String, Value>, String> {
    with(home, |all| {
        all.iter()
            .filter(|(_, d)| d.is_object())
            .map(|(k, d)| (k.clone(), d.get("when").cloned().unwrap_or(Value::Null)))
            .collect()
    })
}

/// {photo: description} of the saved descriptions `matches` is true for (a person's export).
pub fn find(home: &Path, matches: impl Fn(&Value) -> bool) -> Result<Map<String, Value>, String> {
    let _one = hold();
    Ok(load(home)?.into_iter().filter(|(_, d)| matches(d)).collect())
}

/// Delete the saved descriptions `matches` is true for (forgetting a person); how many.
pub fn drop(home: &Path, matches: impl Fn(&Value) -> bool) -> Result<usize, String> {
    let _one = hold();
    let mut all = load(home)?;
    let before = all.len();
    all.retain(|_, d| !matches(d));
    if all.len() < before {
        write(home, &all)?;
    }
    Ok(before - all.len())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn descriptions_are_kept_a_line_each_and_an_unreadable_file_is_an_error() {
        let home = std::env::temp_dir().join(format!("face-descriptions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(load(&home).unwrap().is_empty());
        let mut all = Map::new();
        all.insert("h1".to_string(), json!({"text": "A caf\u{e9} table", "secs": 3.5}));
        all.insert("h2:30".to_string(), json!({"text": "two people"}));
        write(&home, &all).unwrap();
        let text = std::fs::read_to_string(home.join(FILE)).unwrap();
        assert_eq!(
            text,
            "{\n\"h1\": {\"text\": \"A caf\u{e9} table\", \"secs\": 3.5},\n\"h2:30\": {\"text\": \"two people\"}\n}"
        );
        assert_eq!(load(&home).unwrap(), all);
        let about = |d: &Value| d["text"].as_str().is_some_and(|t| t.contains("two"));
        assert_eq!(find(&home, about).unwrap().keys().collect::<Vec<_>>(), ["h2:30"]);
        assert_eq!((drop(&home, about).unwrap(), drop(&home, about).unwrap(), load(&home).unwrap().len()), (1, 0, 1));
        // one kept with its time; written at once or with the next flush
        let kept = save(&home, "h9", &json!({"text": "new", "model": "m", "secs": 2.5}), false).unwrap();
        assert_eq!(
            (kept["when"].as_str().unwrap().len(), saved(&home, "h9").unwrap(), count(&home).unwrap()),
            (20, Some(kept.clone()), 2)
        );
        assert!(!std::fs::read_to_string(home.join(FILE)).unwrap().contains("h9"));
        flush(&home).unwrap();
        assert!(std::fs::read_to_string(home.join(FILE)).unwrap().contains("\"h9\": {\"text\": \"new\""));
        assert_eq!(whens(&home).unwrap()["h9"], kept["when"]);
        assert_eq!(typical_secs(&home, "m").unwrap(), None, "too few to say");
        for (k, secs) in [3.0, 9.0, 4.0, 5.5].iter().enumerate() {
            save(&home, &format!("t{k}"), &json!({"model": "m", "secs": secs}), k == 3).unwrap();
        }
        assert_eq!(
            (
                typical_secs(&home, "m").unwrap(),
                typical_secs(&home, "other").unwrap(),
                typical_secs(&home, "").unwrap()
            ),
            (Some(4.0), None, None)
        );
        // another program's change of the file is seen, unless something here still waits to be written
        std::fs::write(home.join(FILE), "{\"other\": {\"text\": \"written by another program\"}}").unwrap();
        assert_eq!(load(&home).unwrap().keys().collect::<Vec<_>>(), ["other"]);
        save(&home, "mine", &json!({"text": "waits"}), false).unwrap();
        std::fs::write(home.join(FILE), "{}").unwrap();
        assert_eq!(count(&home).unwrap(), 2);
        flush(&home).unwrap();
        assert!(std::fs::read_to_string(home.join(FILE)).unwrap().contains("\"mine\""));
        // a file that can't be read is an error for whoever reads it anew, never "no descriptions"
        std::fs::write(home.join(FILE), "{\"h1\": ").unwrap();
        KEPT.lock().unwrap().clear();
        assert!(load(&home).is_err());
        let _ = std::fs::remove_dir_all(home);
    }
}
