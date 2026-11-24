//! The opt-in presence log (`log_visit` and `visits`): who was in view and when, names
//! and times only, one file a day in `HOME/presence/<day>.jsonl`, a line a visit, in the files' fixed
//! spelling. The day is the one the visit started on by this device's clock.

use std::path::{Path, PathBuf};

use chrono::{Local, TimeZone};
use serde_json::{Value, json};

use crate::filejson;

/// how a line ends: the file is text, so as this system ends a line
pub const END: &str = if cfg!(windows) { "\r\n" } else { "\n" };

pub struct Presence {
    folder: PathBuf,
}

/// The whole number in what a page can send as a time (a fraction is cut off).
fn whole(v: Option<&Value>) -> Result<i64, String> {
    match v {
        Some(Value::Number(n)) => n.as_f64().map(|f| f as i64).ok_or_else(|| "bad visit".to_string()),
        Some(Value::String(s)) => s.trim().parse().map_err(|_| "bad visit".to_string()),
        Some(Value::Bool(b)) => Ok(*b as i64),
        _ => Err("bad visit".to_string()),
    }
}

/// The day (YYYY-MM-DD) a time in epoch ms falls on, by this device's clock.
pub fn day_of(ms: i64) -> Result<String, String> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .ok_or_else(|| "bad visit".to_string())
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

impl Presence {
    pub fn new(home: &Path) -> Presence {
        Presence { folder: home.join("presence") }
    }

    /// Append one finished visit {name, start, end} (epoch ms) to the file of the day it started.
    pub fn log_visit(&self, v: &Value) -> Result<(), String> {
        use std::io::Write;
        if !v.is_object() {
            return Err("expected a visit".to_string());
        }
        let name: String = match v.get("name") {
            None => String::new(),
            Some(Value::String(s)) => s.chars().take(100).collect(),
            Some(_) => return Err("bad visit".to_string()),
        };
        let (start, end) = (whole(v.get("start"))?, whole(v.get("end"))?);
        if name.is_empty() || name == "Unknown" || !(0 < start && start <= end) || end - start > 7 * 86_400_000 {
            return Err("bad visit".to_string());
        }
        // the file is text: on Windows its lines end as Windows' do
        let line = filejson::dumps(&json!({"name": name, "start": start, "end": end})) + END;
        std::fs::create_dir_all(&self.folder).map_err(|e| format!("can't make the presence folder: {e}"))?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.folder.join(format!("{}.jsonl", day_of(start)?)))
            .and_then(|mut f| f.write_all(line.as_bytes()))
            .map_err(|e| format!("can't write the presence log: {e}"))
    }

    /// The visits logged for a day (YYYY-MM-DD), earliest first.
    pub fn visits(&self, day: &str) -> Result<Vec<Value>, String> {
        let b = day.as_bytes();
        let shaped = b.len() == 10
            && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() });
        if !shaped {
            return Err("day must be YYYY-MM-DD".to_string());
        }
        let text = match std::fs::read_to_string(self.folder.join(format!("{day}.jsonl"))) {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };
        // a line that isn't a visit stops nothing
        let mut rows: Vec<Value> =
            text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(Value::is_object).collect();
        rows.sort_by_key(|r| r.get("start").and_then(Value::as_f64).unwrap_or(0.0) as i64);
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visit_is_a_line_of_its_days_file() {
        let dir = std::env::temp_dir().join(format!("face-presence-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = Presence::new(&dir);
        let start = 1_795_161_900_000i64; // 2026-11-20 08:05 UTC
        let day = day_of(start).unwrap();
        assert!(p.visits(&day).unwrap().is_empty(), "nothing logged, no folder yet");
        p.log_visit(&json!({"name": "Zoë", "start": start + 60_000, "end": start + 90_000})).unwrap();
        p.log_visit(&json!({"name": "A", "start": start, "end": start as f64 + 1000.7, "more": "ignored"})).unwrap();
        // the fixed bytes: ", " and ": " between things, ASCII only
        let file = std::fs::read_to_string(dir.join("presence").join(format!("{day}.jsonl"))).unwrap();
        assert_eq!(
            file,
            format!(
                "{{\"name\": \"Zo\\u00eb\", \"start\": {}, \"end\": {}}}\n{{\"name\": \"A\", \"start\": {start}, \"end\": {}}}\n",
                start + 60_000,
                start + 90_000,
                start + 1000
            )
            .replace('\n', END)
        );
        let rows = p.visits(&day).unwrap();
        assert_eq!((rows.len(), rows[0]["name"].clone(), rows[1]["name"].clone()), (2, json!("A"), json!("Zoë")));
        for bad in [
            json!("a visit"),
            json!({"start": start, "end": start}),
            json!({"name": "Unknown", "start": start, "end": start}),
            json!({"name": "A", "start": 0, "end": 5}),
            json!({"name": "A", "start": start, "end": start - 1}),
            json!({"name": "A", "start": start, "end": start + 8 * 86_400_000}),
            json!({"name": "A", "start": "soon", "end": start}),
            json!({"name": "A", "end": start}),
            json!({"name": 5, "start": start, "end": start}),
        ] {
            assert!(p.log_visit(&bad).is_err(), "logged: {bad}");
        }
        for bad in ["", "today", "2026-1-01", "2026-11-20x", "2026/11/20", "../../x"] {
            assert_eq!(p.visits(bad).unwrap_err(), "day must be YYYY-MM-DD", "{bad}");
        }
        assert_eq!(today().len(), 10);
        let _ = std::fs::remove_dir_all(dir);
    }
}
