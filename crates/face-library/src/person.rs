//! One person in the library: what its labels, faces, history, backups and photo tags hold about them, for
//! exporting it or forgetting them (the labeling data's part; the
//! private folder's part is the labeling tool's).
//!
//! Forgetting keeps their faces as Ignored faces without print or picture (only where each face is in its
//! photo, so the photos aren't read again and the faces don't come back as new faces), removes their name
//! everywhere here, and clears Undo. It can't be undone.

use std::path::{Path, PathBuf};

use face_files::{filejson, paths::atomic_write};
use serde_json::{Map, Value, json};

use crate::labels::{Fault, IGNORE, LabelDb};

type Result<T> = std::result::Result<T, Fault>;

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True if `text` has `name` as a word (any case): "Ann" in "Ann and Bo", not in "Anna".
pub fn mentions(text: &str, name: &str) -> bool {
    let (text, name): (Vec<char>, Vec<char>) = (text.chars().collect(), name.chars().collect());
    if name.is_empty() || name.len() > text.len() {
        return false;
    }
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    (0..=text.len() - name.len()).any(|at| {
        let end = at + name.len();
        (at == 0 || !word(text[at - 1]))
            && (end == text.len() || !word(text[end]))
            && text[at..end].iter().zip(&name).all(|(a, b)| same(*a, *b))
    })
}

/// Whether a JSON value says something: nothing, false, 0, "" and an empty list or object don't.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Anything JSON without a name: an object's entries under it or equal to it, and a list's items equal to
/// it, removed.
pub fn scrub(v: &Value, name: &str) -> Value {
    let is = |v: &Value| v.as_str() == Some(name);
    match v {
        Value::Object(o) => Value::Object(
            o.iter().filter(|(k, v)| k.as_str() != name && !is(v)).map(|(k, v)| (k.clone(), scrub(v, name))).collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().filter(|v| !is(v)).map(|v| scrub(v, name)).collect()),
        other => other.clone(),
    }
}

/// The parts of a file of fitted numbers that are about a name (an entry under it, or a list holding it).
pub fn only(v: &Value, name: &str) -> Value {
    match v {
        Value::Object(o) => {
            let got = o.iter().map(|(k, v)| (k.clone(), if k == name { v.clone() } else { only(v, name) }));
            // (what is about nobody else's name is nothing, an empty object or an empty list)
            Value::Object(got.filter(|(k, v)| k == name || truthy(v)).collect())
        }
        Value::Array(a) if a.iter().any(|v| v.as_str() == Some(name)) => json!([name]),
        _ => Value::Null,
    }
}

fn names_not(rec: &Map<String, Value>, name: &str) -> bool {
    rec.get("not").and_then(Value::as_array).is_some_and(|not| not.iter().any(|n| n.as_str() == Some(name)))
}

fn is_named(rec: &Value, name: &str) -> bool {
    rec.as_object().is_some_and(|r| r.get("n").and_then(Value::as_str) == Some(name) || names_not(r, name))
}

/// A face's label with a name forgotten: their face becomes Ignored, a "not this person" list loses the
/// name, and what says nothing goes. None: no label is left.
pub fn forget_label(rec: &Value, name: &str) -> Option<Value> {
    let mut r = rec.as_object().cloned().unwrap_or_default();
    if r.get("n").and_then(Value::as_str) == Some(name) {
        let not = r.get("not").cloned().unwrap_or(Value::Null);
        r = Map::new();
        r.insert("n".to_string(), json!(IGNORE));
        r.insert("by".to_string(), json!("me"));
        r.insert("not".to_string(), not);
    }
    if names_not(&r, name) {
        let left: Vec<Value> =
            r["not"].as_array().into_iter().flatten().filter(|n| n.as_str() != Some(name)).cloned().collect();
        r.insert("not".to_string(), Value::Array(left));
    }
    let left: Map<String, Value> = r.into_iter().filter(|(_, v)| truthy(v)).collect();
    (!left.is_empty()).then_some(Value::Object(left))
}

/// True if an entry of the history is about a name (its text, or any face's label in it).
pub fn history_names(line: &Value, name: &str) -> bool {
    mentions(line.get("what").and_then(Value::as_str).unwrap_or(""), name)
        || line.get("faces").and_then(Value::as_object).is_some_and(|faces| faces.values().any(|r| is_named(r, name)))
}

/// A .jsonl file's lines: (the line, what it says when it can be read).
pub fn jsonl(path: &Path) -> Vec<(String, Option<Value>)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines().map(|line| (line.to_string(), serde_json::from_str(line).ok())).collect()
}

/// Write a .jsonl file again without the lines `about` is true for; how many went.
pub fn drop_lines(path: &Path, about: impl Fn(&Value) -> bool) -> std::io::Result<usize> {
    let rows = jsonl(path);
    let keep: Vec<&String> = rows
        .iter()
        .filter(|(_, o)| !o.as_ref().is_some_and(|o| o.is_object() && about(o)))
        .map(|(line, _)| line)
        .collect();
    if keep.len() < rows.len() {
        atomic_write(path, keep.iter().map(|line| format!("{line}\n")).collect::<String>().as_bytes())?;
    }
    Ok(rows.len() - keep.len())
}

/// The files of a folder (and with `deep`, the folders in it) whose names end so, in the order of their paths.
pub fn files(folder: &Path, ends: &str, deep: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut look = vec![folder.to_path_buf()];
    while let Some(dir) = look.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if deep {
                    look.push(path);
                }
            } else if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(ends)) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// What the library holds about one person, for their export.
pub struct Held {
    /// each of their faces: where it is, its label, its print (base64 of 32-bit numbers) and picture's file
    pub faces: Vec<Value>,
    /// their faces' pictures: (face ID, JPEG)
    pub pictures: Vec<(String, Vec<u8>)>,
    /// the faces marked as not them
    pub not_them: Vec<Value>,
    /// the photos whose people tags name them
    pub tags: Vec<Value>,
    pub fids: Vec<String>,
    /// their faces' photos, where this device has them
    pub paths: Vec<String>,
}

/// What forgetting a person changed in the library.
pub struct Forgot {
    pub faces: usize,
    pub labels: usize,
    pub history: usize,
    pub photo_tags: usize,
    pub backups: usize,
    pub fids: Vec<String>,
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for k in 0..4 {
            out.push(if k <= chunk.len() { ABC[(n >> (18 - 6 * k)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

impl LabelDb {
    /// A name is a person in the labels: named on a face, or in a "not this person" list.
    pub fn is_person(&self, name: &str) -> bool {
        self.labels.values().any(|r| is_named(r, name))
    }

    pub fn their_faces(&self, name: &str) -> Vec<usize> {
        (0..self.n()).filter(|&i| self.name(i) == Some(name)).collect()
    }

    /// Everything the library holds about one person.
    pub fn held_about(&self, name: &str) -> Held {
        let idx = self.their_faces(name);
        let (mut faces, mut pictures, mut paths) = (Vec::new(), Vec::new(), Vec::new());
        for &i in &idx {
            let s = &self.store;
            let (fid, hash) = (s.fid[i].clone(), s.photo[i].clone());
            let record = s.photos.get(&hash);
            let said = |k: &str| record.and_then(|r| r.get(k)).and_then(Value::as_str).filter(|t| !t.is_empty());
            if let Some(path) = said("path") {
                paths.push(path.to_string());
            }
            let place: Vec<f64> = s.boxes[i][..4].iter().map(|v| filejson::round(f64::from(*v), 1)).collect();
            let print = (!s.printless(i))
                .then(|| base64(&s.print(i).iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()));
            let picture = (!s.thumbs[i].is_empty()).then(|| format!("faces/{fid}.jpg"));
            faces.push(json!({"faceId": fid, "photo": said("path").or(said("name")).unwrap_or(s.file[i].as_str()), "photoHash": hash,
                              "videoSeconds": self.video_at(i), "box": place, "quality": self.q(i), "label": self.rec(i),
                              "picture": picture, "print": print}));
            if !self.store.thumbs[i].is_empty() {
                pictures.push((fid, self.store.thumbs[i].clone()));
            }
        }
        let not_them = (self.labels.iter())
            .filter(|(_, r)| r.as_object().is_some_and(|r| names_not(r, name)))
            .map(|(fid, r)| json!({"faceId": fid, "label": r}))
            .collect();
        let tagged: Vec<String> = (self.facts.facts.iter())
            .filter(|(_, f)| f.tags.iter().any(|t| t == name) || f.faces.iter().any(|(_, n)| n == name))
            .map(|(h, _)| h.clone())
            .collect();
        let tags = (tagged.iter())
            .map(|h| json!({"photo": self.store.photos.get(h).and_then(|r| r.get("path")).cloned().unwrap_or(Value::Null), "tags": self.facts.facts[h].tags}))
            .collect();
        let fids = idx.iter().map(|&i| self.store.fid[i].clone()).collect();
        Held { faces, pictures, not_them, tags, fids, paths }
    }

    /// Forget a person in the library: their faces become Ignored faces without print or picture, their
    /// name leaves every label, the history, the backups and the photos' people tags, and Undo is cleared.
    pub fn forget_person(&mut self, name: &str) -> Result<Forgot> {
        let files = |e: std::io::Error| Fault::Files(e.to_string());
        let idx = self.their_faces(name);
        let mut fids: Vec<String> = idx.iter().map(|&i| self.store.fid[i].clone()).collect();
        let mut changed = 0;
        for (fid, rec) in self.labels.clone() {
            let new = forget_label(&rec, name);
            if new.as_ref() != Some(&rec) {
                changed += 1;
                match new {
                    Some(new) => self.labels.insert(fid, new),
                    None => self.labels.shift_remove(&fid),
                };
            }
        }
        self.clear_undo();
        if !idx.is_empty() {
            self.forget_faces(&idx)?; // prints and pictures
        }
        let history = drop_lines(&self.data().join("history.jsonl"), |o| history_names(o, name)).map_err(files)?;
        fids.sort();
        self.save(&format!("Forgot a person ({} faces)", idx.len()), &fids)?; // the labels, a line of history, the people
        let mut photo_tags = 0;
        for fact in self.facts.facts.values_mut() {
            if fact.tags.iter().any(|t| t == name) || fact.faces.iter().any(|(_, n)| n == name) {
                fact.tags.retain(|t| t != name);
                fact.faces.retain(|(_, n)| n != name);
                photo_tags += 1;
            }
        }
        if photo_tags > 0 {
            self.facts.save(&self.store).map_err(files)?;
        }
        let backups = forget_backups(&self.data().join("backups"), name).map_err(files)?;
        Ok(Forgot { faces: idx.len(), labels: changed, history, photo_tags, backups, fids })
    }
}

/// The backups: earlier copies of the labels get the same change as the labels; other JSON files there
/// (the group files from before labels were kept by face) lose the name wherever it appears; and the one copy
/// of the faces kept from before Ignored faces' prints were deleted goes, since it holds this person's too.
/// How many files changed.
fn forget_backups(root: &Path, name: &str) -> std::io::Result<usize> {
    let mut n = 0;
    for file in files(root, ".json", true) {
        let Some(old) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else {
            continue;
        };
        let labels = file.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.starts_with("labels-"));
        let new = match old.as_object().filter(|_| labels) {
            Some(recs) => Value::Object(
                (recs.iter())
                    .filter_map(|(fid, r)| {
                        Some((
                            fid.clone(),
                            if r.is_object() { forget_label(r, name)? } else { Some(r.clone()).filter(truthy)? },
                        ))
                    })
                    .collect(),
            ),
            None => scrub(&old, name),
        };
        if new != old {
            atomic_write(&file, filejson::dumps_lines(&new, true, 1).as_bytes())?;
            n += 1;
        }
    }
    let mut old_copies: Vec<PathBuf> = (std::fs::read_dir(root).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir() && p.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.starts_with("before-v1.9-ignore-"))
        })
        .collect();
    old_copies.sort();
    for folder in old_copies {
        let _ = std::fs::remove_dir_all(folder);
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_found_as_a_word_in_any_case() {
        assert!(
            mentions("Ann and Bo", "Ann")
                && mentions("with ann.", "Ann")
                && mentions("ANN", "ann")
                && mentions("(Ann)", "Ann")
        );
        assert!(
            !mentions("Anna and Bo", "Ann")
                && !mentions("Joann", "Ann")
                && !mentions("Ann_2", "Ann")
                && !mentions("", "Ann")
        );
        assert!(
            mentions("named Zo\u{eb} Q.", "zo\u{cb} q.")
                && mentions("Ann (work) left", "Ann (work)")
                && !mentions("Ann", "")
        );
    }

    #[test]
    fn a_label_loses_a_forgotten_name_and_what_says_nothing() {
        let label = |v: Value| forget_label(&v, "Ann");
        // their face becomes Ignored, and keeps who it is not
        assert_eq!(
            label(json!({"n": "Ann", "by": "auto", "s": 0.5, "not": ["Bo"]})),
            Some(json!({"n": "#ignore", "by": "me", "not": ["Bo"]}))
        );
        assert_eq!(label(json!({"n": "Ann", "by": "me"})), Some(json!({"n": "#ignore", "by": "me"})));
        // another person's face loses the name from its list
        assert_eq!(
            label(json!({"n": "Bo", "by": "me", "not": ["Ann", "Cy"]})),
            Some(json!({"n": "Bo", "by": "me", "not": ["Cy"]}))
        );
        assert_eq!(label(json!({"not": ["Ann"]})), None, "nothing left to say");
        // a label that isn't about them stays (what says nothing in it goes)
        assert_eq!(label(json!({"n": "Bo", "by": "me"})), Some(json!({"n": "Bo", "by": "me"})));
        assert_eq!(label(json!({"n": "Bo", "by": "me", "ok": false, "not": []})), Some(json!({"n": "Bo", "by": "me"})));
        assert_eq!(label(json!(null)), None);
    }

    #[test]
    fn anything_json_loses_a_name_and_the_fitted_numbers_about_it_are_found() {
        let v = json!({"Ann": {"a": 1}, "Bo": {"likes": "Ann", "with": ["Ann", "Cy"], "n": 2}, "who": ["Ann"], "x": 0});
        assert_eq!(scrub(&v, "Ann"), json!({"Bo": {"with": ["Cy"], "n": 2}, "who": [], "x": 0}));
        assert_eq!(only(&v, "Ann"), json!({"Ann": {"a": 1}, "Bo": {"with": ["Ann"]}, "who": ["Ann"]}));
        assert_eq!(only(&json!({"Bo": {"n": 2}}), "Ann"), json!({}));
        assert!(history_names(&json!({"what": "named Ann", "faces": {}}), "Ann"));
        assert!(history_names(&json!({"what": "named 1 face", "faces": {"f1": {"n": "Ann"}, "f2": null}}), "Ann"));
        assert!(history_names(&json!({"what": "x", "faces": {"f1": {"n": "Bo", "not": ["Ann"]}}}), "Ann"));
        assert!(!history_names(&json!({"what": "named Anna", "faces": {"f1": {"n": "Anna"}}}), "Ann"));
        assert_eq!(base64(b"any carnal pleas"), "YW55IGNhcm5hbCBwbGVhcw==");
        assert_eq!(
            (base64(b""), base64(b"a"), base64(b"ab"), base64(b"abc")),
            (String::new(), "YQ==".to_string(), "YWI=".to_string(), "YWJj".to_string())
        );
    }

    #[test]
    fn lines_about_someone_are_dropped_and_backups_changed() {
        let dir = std::env::temp_dir().join(format!("face-person-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("backups").join("legacy-1")).unwrap();
        std::fs::create_dir_all(dir.join("backups").join("before-v1.9-ignore-20260101")).unwrap();
        let log = dir.join("log.jsonl");
        std::fs::write(&log, "{\"name\": \"Ann\"}\r\nnot a line\n{\"name\": \"Bo\"}\n").unwrap();
        assert_eq!(drop_lines(&log, |o| o["name"] == "Ann").unwrap(), 1);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "not a line\n{\"name\": \"Bo\"}\n");
        assert_eq!(drop_lines(&log, |o| o["name"] == "Ann").unwrap(), 0);
        assert_eq!(drop_lines(&dir.join("no such file"), |_| true).unwrap(), 0);
        let backups = dir.join("backups");
        std::fs::write(
            backups.join("labels-1.json"),
            r#"{"f1": {"n": "Ann", "by": "me"}, "f2": {"n": "Bo", "by": "me"}, "f3": {"not": ["Ann"]}}"#,
        )
        .unwrap();
        std::fs::write(backups.join("labels-2.json"), r#"{"f2": {"n": "Bo", "by": "me"}}"#).unwrap();
        std::fs::write(backups.join("legacy-1").join("groups.json"), r#"{"names": {"3": "Ann", "4": "Bo"}}"#).unwrap();
        std::fs::write(backups.join("broken.json"), "{").unwrap();
        assert_eq!(forget_backups(&backups, "Ann").unwrap(), 3, "two files changed, one old copy of the faces gone");
        let read = |f: PathBuf| serde_json::from_str::<Value>(&std::fs::read_to_string(f).unwrap()).unwrap();
        assert_eq!(
            read(backups.join("labels-1.json")),
            json!({"f1": {"n": "#ignore", "by": "me"}, "f2": {"n": "Bo", "by": "me"}})
        );
        assert_eq!(read(backups.join("legacy-1").join("groups.json")), json!({"names": {"4": "Bo"}}));
        assert!(!backups.join("before-v1.9-ignore-20260101").exists());
        assert_eq!(forget_backups(&backups, "Ann").unwrap(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
