//! Expression test data: written only when the owner starts it on the
//! tracker page, numbers only (no picture, no sound), in `data/expr_eval/`:
//!
//! - `<session>.jsonl`, a posed test: the owner is asked to make each expression in turn; every readable
//!   frame appends the expression asked for, what the model said, and the face's quality, head turn, size
//!   and brightness, with the page's talking signals.
//! - `feel-<day>.jsonl`, feeling reports: the owner's answer to "how do you feel?" (pleasant and energized,
//!   1 to 7) with the page's summary of the 30 seconds before the question.
//!
//! The model is measured against them; the lines' form is fixed, byte
//! for byte. What MediaPipe adds to a posed frame (face-shape cues, body points, a hand on the face) is not
//! written by this program: those fields say null.

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

use chrono::Utc;
use serde_json::{Map, Value, json};

use crate::filejson;

/// how a line ends: the file is text, so as this system ends a line
const END: &str = if cfg!(windows) { "\r\n" } else { "\n" };

pub struct Recorder {
    folder: PathBuf,
    lock: Mutex<()>,
}

/// A posed test's name: the time it began, "20261120-080500".
pub fn is_session(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 15 && b.iter().enumerate().all(|(i, c)| if i == 8 { *c == b'-' } else { c.is_ascii_digit() })
}

/// Now, in seconds with two decimals.
fn now() -> f64 {
    let t = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    format!("{t:.2}").parse().unwrap_or(t)
}

/// The report the page sent, keeping only what belongs in it: valence and arousal 1 to 7, the name, and a
/// flat set of named numbers (or short lists of numbers).
pub fn check_report(body: &[u8]) -> Result<Value, String> {
    let rep: Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    if !rep.is_object() {
        return Err("bad report".to_string());
    }
    let mut out = Map::new();
    for k in ["valence", "arousal"] {
        // a whole number: 5.0 is none
        let v = rep.get(k).filter(|v| v.is_i64() || v.is_u64()).and_then(Value::as_i64).filter(|v| (1..=7).contains(v));
        out.insert(k.to_string(), json!(v.ok_or_else(|| format!("{k} must be 1-7"))?));
    }
    let name =
        rep.get("name").and_then(Value::as_str).filter(|n| (1..=80).contains(&n.chars().count())).ok_or("bad name")?;
    out.insert("name".to_string(), json!(name));
    let none = Map::new();
    let feats = match rep.get("features") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => &none,
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => &none,
        Some(Value::String(s)) if s.is_empty() => &none,
        Some(Value::Array(a)) if a.is_empty() => &none,
        Some(Value::Object(m)) if m.len() <= 120 => m,
        Some(_) => return Err("bad features".to_string()),
    };
    let number = |v: &Value| v.as_f64().is_some_and(|f| v.is_number() && f.abs() < 1e6);
    let named = |k: &str| {
        let b = k.as_bytes();
        (1..=40).contains(&b.len())
            && (b[0].is_ascii_alphabetic() || b[0] == b'_')
            && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
    };
    let mut clean = Map::new();
    for (k, v) in feats {
        if !named(k) {
            return Err("bad feature name".to_string());
        }
        let fits = v.is_null() || number(v) || v.as_array().is_some_and(|l| l.len() <= 16 && l.iter().all(number));
        if !fits {
            return Err(format!("feature {k} must be a number"));
        }
        clean.insert(k.clone(), v.clone());
    }
    out.insert("features".to_string(), Value::Object(clean));
    Ok(Value::Object(out))
}

impl Recorder {
    pub fn new(root: &Path) -> Recorder {
        Recorder { folder: root.join("data").join("expr_eval"), lock: Mutex::new(()) }
    }

    fn append(&self, stem: &str, line: &Value) -> Result<(), String> {
        let _one = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        std::fs::create_dir_all(&self.folder).map_err(|e| format!("can't make the test data's folder: {e}"))?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.folder.join(format!("{stem}.jsonl")))
            .and_then(|mut f| f.write_all((filejson::dumps(line) + END).as_bytes()))
            .map_err(|e| format!("can't write the test data: {e}"))
    }

    /// One frame of a posed test: what was asked for (`posed`, `extra`) and what the tracker read of the
    /// face, taken from the answer the page gets for that face.
    pub fn add(&self, session: &str, posed: &str, extra: &Map<String, Value>, face: &Value) -> Result<(), String> {
        let width = face["box"][2].as_f64().unwrap_or(0.0);
        let mut line = json!({
            "t": now(), "posed": posed, "name": face["name"], "quality": face["quality"], "turn": face["turn"],
            "size": face_core::image::round_even(width), "light": face["light"], "expr": face["expr"],
            "blendshapes": null, "pose": face["pose"], "hand": face["hand"],
        });
        for (k, v) in extra {
            line[k] = v.clone();
        }
        self.append(session, &line)
    }

    /// One feeling report, already checked (`check_report`).
    pub fn report(&self, rep: &Value) -> Result<(), String> {
        let mut line = json!({"t": now(), "kind": "report"});
        for (k, v) in rep.as_object().into_iter().flatten() {
            line[k] = v.clone();
        }
        self.append(&format!("feel-{}", Utc::now().format("%Y%m%d")), &line)
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.folder)
            .into_iter()
            .flatten()
            .filter_map(|f| f.ok().map(|f| f.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        files.sort();
        files
    }

    /// The posed tests there are, by name.
    pub fn sessions(&self) -> Vec<String> {
        let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().into_owned());
        self.files().iter().filter_map(stem).filter(|s| is_session(s)).collect()
    }

    /// How many feeling reports there are.
    pub fn reports(&self) -> usize {
        self.files()
            .iter()
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("feel-")))
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
            .sum()
    }

    /// "Delete test data" on the page: every file of it.
    pub fn delete_all(&self) -> bool {
        let _one = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        for file in self.files() {
            let _ = std::fs::remove_file(file);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_posed_frame_and_a_report_are_lines_of_numbers() {
        let dir = std::env::temp_dir().join(format!("face-recorder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let r = Recorder::new(&dir);
        assert_eq!((r.sessions().len(), r.reports()), (0, 0));
        let face = json!({"box": [100.0, 80.0, 120.5, 150.0], "name": "Zoë", "quality": 0.612, "turn": 0.05, "light": 0.48,
                          "expr": [0.9, 0.02, 0.02, 0.02, 0.02, 0.01, 0.01], "pose": null, "hand": null, "emb": "left out"});
        let extra: Map<String, Value> =
            json!({"talk": false, "handAsked": true, "tv": 0.1234}).as_object().unwrap().clone();
        r.add("20261120-080500", "happy", &extra, &face).unwrap();
        let line = std::fs::read_to_string(dir.join("data/expr_eval/20261120-080500.jsonl")).unwrap();
        let t = line.split("\"t\": ").nth(1).unwrap().split(',').next().unwrap();
        assert_eq!(
            line,
            format!(
                "{{\"t\": {t}, \"posed\": \"happy\", \"name\": \"Zo\\u00eb\", \"quality\": 0.612, \"turn\": 0.05, \"size\": 120, \"light\": 0.48, \"expr\": [0.9, 0.02, 0.02, 0.02, 0.02, 0.01, 0.01], \"blendshapes\": null, \"pose\": null, \"hand\": null, \"talk\": false, \"handAsked\": true, \"tv\": 0.1234}}{END}"
            )
        );
        let rep = check_report(br#"{"valence": 5, "arousal": 2, "name": "A", "features": {"smile": 0.25, "n": 3, "none": null, "list": [1, 2.5]}, "more": "dropped"}"#).unwrap();
        assert_eq!(
            rep,
            json!({"valence": 5, "arousal": 2, "name": "A", "features": {"smile": 0.25, "n": 3, "none": null, "list": [1, 2.5]}})
        );
        r.report(&rep).unwrap();
        r.report(&rep).unwrap();
        assert_eq!((r.sessions(), r.reports()), (vec!["20261120-080500".to_string()], 2));
        assert!(r.delete_all());
        assert_eq!((r.sessions().len(), r.reports()), (0, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_is_not_a_report_or_a_test_is_refused() {
        for (body, why) in [
            (r#"[1]"#, "bad report"),
            (r#"{"valence": 8, "arousal": 2, "name": "A"}"#, "valence must be 1-7"),
            (r#"{"valence": 5.0, "arousal": 2, "name": "A"}"#, "valence must be 1-7"),
            (r#"{"valence": 5, "arousal": true, "name": "A"}"#, "arousal must be 1-7"),
            (r#"{"valence": 5, "arousal": 2, "name": ""}"#, "bad name"),
            (r#"{"valence": 5, "arousal": 2, "name": "A", "features": [1]}"#, "bad features"),
            (r#"{"valence": 5, "arousal": 2, "name": "A", "features": {"a b": 1}}"#, "bad feature name"),
            (r#"{"valence": 5, "arousal": 2, "name": "A", "features": {"x": "text"}}"#, "feature x must be a number"),
            (r#"{"valence": 5, "arousal": 2, "name": "A", "features": {"x": 1e7}}"#, "feature x must be a number"),
            (r#"{"valence": 5, "arousal": 2, "name": "A", "features": {"x": [1, [2]]}}"#, "feature x must be a number"),
        ] {
            assert_eq!(check_report(body.as_bytes()).unwrap_err(), why, "{body}");
        }
        for (name, is) in [
            ("20261120-080500", true),
            ("20261120-08050", false),
            ("20261120_080500", false),
            ("feel-20261120", false),
            ("20261120-080500\n", false),
        ] {
            assert_eq!(is_session(name), is, "{name:?}");
        }
    }
}
