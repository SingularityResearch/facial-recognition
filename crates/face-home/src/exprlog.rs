//! The expression log, as far as the tracker keeps it: what each named person's face looked like,
//! every few seconds while the tracker
//! sees them. A reading says what a face looked like to the model, never what the person felt.
//!
//! The page sends its readings to `/api/expression-log`. They wait in `HOME/expression/<UTC day>.jsonl`, a
//! line a reading, until the sync has put them into the shared database, which is where they are kept.
//! Sending them is the sync's part (`face-sync`). Nothing is written without a shared database or
//! with "Keep expression readings" switched off.
//!
//! One reading: {"t": epoch ms, "name", "label", "share", "state", "data"}; `data` is numbers and short
//! text only (`alerts::check_details`).

use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use crate::{
    alerts::check_details,
    filejson::{self, said},
    paths::SharedLock,
};

/// in HOME: readings waiting to be sent
const FOLDER: &str = "expression";
/// the tracker page's "Keep expression readings" (on unless switched off)
pub const OPTION: &str = "ft-o-optExprLog";
/// readings in one request
const MAX_ROWS: usize = 100;
/// bytes of one reading's data, in the files' one-line spelling
const MAX_DATA: usize = 4000;
const STATES: [&str; 5] = ["plain", "change", "usual", "unsure", ""];
/// how a line ends: the file is text, so as this system ends a line
const END: &str = if cfg!(windows) { "\r\n" } else { "\n" };

pub struct ExprLog {
    folder: PathBuf,
    /// appended to by the tracker, sent and rewritten by the sync
    lock: SharedLock,
}

/// One reading as the page sends it, checked; why not if it isn't one.
pub fn check(r: &Value, now_ms: i64) -> Result<Value, String> {
    if !r.is_object() {
        return Err("a reading is an object".to_string());
    }
    let named = || "a reading needs a named person".to_string();
    let bad = || "bad reading".to_string();
    let name: String = said(r.get("name")).ok_or_else(named)?.trim().chars().take(100).collect();
    if name.is_empty() || name == "Unknown" || name.starts_with('#') {
        return Err(named());
    }
    let t = match r.get("t") {
        Some(Value::Number(n)) => n.as_f64().map(|f| f as i64).ok_or_else(bad)?,
        Some(Value::String(s)) => s.trim().parse::<i64>().map_err(|_| bad())?,
        Some(Value::Bool(b)) => *b as i64,
        _ => return Err(bad()),
    };
    let share = match r.get("share") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => Some(n.as_f64().ok_or_else(bad)?),
        Some(Value::String(s)) => Some(s.trim().parse::<f64>().ok().filter(|f| f.is_finite()).ok_or_else(bad)?),
        Some(Value::Bool(b)) => Some(*b as u8 as f64),
        Some(_) => return Err(bad()),
    };
    if (t - now_ms).abs() > 86_400_000 {
        return Err("reading time out of range".to_string());
    }
    let label: String = said(r.get("label")).ok_or_else(bad)?.chars().take(40).collect();
    let state: String = said(r.get("state")).ok_or_else(bad)?.chars().take(20).collect();
    if label.is_empty() || !STATES.contains(&state.as_str()) || share.is_some_and(|s| !(0.0..=1.0).contains(&s)) {
        return Err(bad());
    }
    // nothing, or an empty thing of any kind, is no data
    let empty = json!({});
    let data = match r.get("data") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => &empty,
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => &empty,
        Some(Value::String(s)) if s.is_empty() => &empty,
        Some(Value::Array(a)) if a.is_empty() => &empty,
        Some(d) => d,
    };
    if !data.is_object() || filejson::dumps(data).len() > MAX_DATA {
        return Err("bad reading data".to_string());
    }
    let share = share.map(|s| format!("{s:.4}").parse::<f64>().unwrap_or(s));
    Ok(json!({"t": t, "name": name, "label": label, "share": share, "state": state, "data": check_details(data)?}))
}

impl ExprLog {
    pub fn new(home: &Path) -> ExprLog {
        ExprLog { folder: home.join(FOLDER), lock: SharedLock::new(home, &format!("waitlog-{FOLDER}")) }
    }

    /// Keep readings until the sync sends them: each a line at the end of the file of its UTC day. How many
    /// were kept: none when readings aren't kept here (`enabled`). A forgotten person's are left out, so a
    /// page that hasn't heard of the forget yet can't write them back.
    pub fn add(
        &self,
        rows: Option<&Value>,
        enabled: bool,
        forgotten: &BTreeSet<String>,
        now_ms: i64,
    ) -> Result<usize, String> {
        let rows = rows
            .and_then(Value::as_array)
            .filter(|r| r.len() <= MAX_ROWS)
            .ok_or("expected a short list of readings")?;
        let rows: Vec<Value> = rows.iter().map(|r| check(r, now_ms)).collect::<Result<_, _>>()?;
        if rows.is_empty() || !enabled {
            return Ok(0);
        }
        // (day, its lines), the days in the order their first reading came
        let mut by_day: Vec<(String, String)> = Vec::new();
        let mut kept = 0;
        for r in rows.iter().filter(|r| !r["name"].as_str().is_some_and(|n| forgotten.contains(n))) {
            let day = Utc
                .timestamp_millis_opt(r["t"].as_i64().unwrap_or(0))
                .single()
                .map(|t| t.format("%Y-%m-%d").to_string())
                .ok_or("bad reading")?;
            let line = filejson::dumps_utf8(r) + END;
            match by_day.iter_mut().find(|(d, _)| *d == day) {
                Some((_, lines)) => lines.push_str(&line),
                None => by_day.push((day, line)),
            }
            kept += 1;
        }
        let _held = self.lock.hold();
        std::fs::create_dir_all(&self.folder).map_err(|e| format!("can't make the expression folder: {e}"))?;
        for (day, lines) in by_day {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.folder.join(format!("{day}.jsonl")))
                .and_then(|mut f| f.write_all(lines.as_bytes()))
                .map_err(|e| format!("can't write the expression log: {e}"))?;
        }
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_161_900_000; // 2026-11-20 08:05 UTC

    #[test]
    fn readings_wait_in_the_file_of_their_utc_day() {
        let dir = std::env::temp_dir().join(format!("face-exprlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = ExprLog::new(&dir);
        let nobody = BTreeSet::new();
        let rows = json!([
            {"t": NOW, "name": " Zoë ", "label": "happy", "share": 0.83333, "state": "plain", "data": {"lead": 0.412345, "votes": 9}},
            {"t": NOW - 9 * 3_600_000, "name": "A", "label": "neutral", "share": 1, "state": "", "more": "dropped"},
            {"t": NOW + 1000, "name": "B", "label": "sad"},
        ]);
        assert_eq!(log.add(Some(&rows), false, &nobody, NOW), Ok(0), "not kept here: checked, nothing written");
        assert!(!dir.join("expression").exists());
        assert_eq!(
            log.add(Some(&rows), true, &BTreeSet::from(["B".to_string()]), NOW),
            Ok(2),
            "a forgotten person's is left out"
        );
        let read = |day: &str| std::fs::read_to_string(dir.join("expression").join(format!("{day}.jsonl"))).unwrap();
        // a line a reading in the files' spelling, text as it is, ended as this system ends a line
        assert_eq!(
            read("2026-11-20"),
            format!(
                "{{\"t\": {NOW}, \"name\": \"Zoë\", \"label\": \"happy\", \"share\": 0.8333, \"state\": \"plain\", \"data\": {{\"lead\": 0.4123, \"votes\": 9}}}}{END}"
            )
        );
        assert_eq!(
            read("2026-11-19"),
            format!(
                "{{\"t\": {}, \"name\": \"A\", \"label\": \"neutral\", \"share\": 1.0, \"state\": \"\", \"data\": {{}}}}{END}",
                NOW - 9 * 3_600_000
            )
        );
        assert_eq!(log.add(Some(&json!([])), true, &nobody, NOW), Ok(0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_is_not_a_reading_is_refused_and_nothing_of_its_request_kept() {
        let dir = std::env::temp_dir().join(format!("face-exprlog-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = ExprLog::new(&dir);
        let good = json!({"t": NOW, "name": "A", "label": "happy", "share": 0.5, "state": "usual", "data": {}});
        for (key, value, why) in [
            ("name", json!(""), "named person"),
            ("name", json!("Unknown"), "named person"),
            ("name", json!("#ignore"), "named person"),
            ("t", Value::Null, "bad reading"),
            ("t", json!("soon"), "bad reading"),
            ("t", json!(NOW - 2 * 86_400_000), "time out of range"),
            ("share", json!(1.5), "bad reading"),
            ("share", json!("half"), "bad reading"),
            ("label", json!(""), "bad reading"),
            ("state", json!("certain"), "bad reading"),
            ("data", json!([1]), "bad reading data"),
            ("data", json!({"long": "x".repeat(MAX_DATA)}), "bad reading data"),
            ("data", json!({"picture": {"a": {"b": {"c": {"d": 1}}}}}), "details must be"),
        ] {
            let mut r = good.clone();
            r[key] = value;
            let e = check(&r, NOW).unwrap_err();
            assert!(e.contains(why), "{key}: {e}");
            // one bad reading refuses its whole request
            assert!(log.add(Some(&json!([good, r])), true, &BTreeSet::new(), NOW).is_err());
        }
        assert_eq!(check(&json!("a reading"), NOW).unwrap_err(), "a reading is an object");
        for not_a_list in [None, Some(json!({"readings": 1})), Some(json!(vec![good.clone(); MAX_ROWS + 1]))] {
            assert_eq!(
                log.add(not_a_list.as_ref(), true, &BTreeSet::new(), NOW).unwrap_err(),
                "expected a short list of readings"
            );
        }
        assert!(!dir.join("expression").exists(), "nothing was written");
    }
}
