//! The signal history, as far as the private folder has to do with it (the owner, 2026-11-24: "Keep a
//! history of ALL signals in the database ... it is opt-in per device in settings. There needs to be a
//! setting that can adjust how long they are kept for").
//!
//! The tracker page makes the rows (one a stretch of time for the device and for each named person in
//! view, every reading's part in it: `web/signals_chart.js` `History`) and sends them to its own server,
//! which checks each (`check`) and hands them to its writer (`face-server` `history.rs`). The shared database
//! is where they are kept. Rows the database couldn't take (it is away, or the device is) wait in
//! `HOME/signals/<UTC day>.jsonl`, a line a row, until it can (`Spill`); nothing is written there otherwise,
//! and nothing at all without a shared database or with the switch off, which is how a new copy starts.
//!
//! One row: {"t": epoch ms of the stretch's start, "name": the person or "" for the device's own row,
//! "s": the stretch's seconds, "n": readings in it, "data": numbers, yes-or-no and short words only}.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{TimeZone, Utc};
use serde_json::{Map, Value, json};

use crate::{filejson, paths::SharedLock};

/// in HOME: rows waiting for the shared database
const FOLDER: &str = "signals";
/// the tracker page's "Keep a history of every signal" (off unless switched on)
pub const OPTION: &str = "ft-o-optHistory";
/// the tracker page's "Keep the history for" (days; without one, `KEEP_DAYS`)
pub const KEEP: &str = "ft-o-optHistoryDays";
/// days the history is kept where the owner set none
pub const KEEP_DAYS: i32 = 90;
/// the fewest and the most days that can be set
const KEEP_RANGE: (i32, i32) = (1, 3650);
/// rows in one request
pub const MAX_ROWS: usize = 400;
/// bytes of one row's data, in the files' one-line spelling
const MAX_DATA: usize = 24_000;
/// bytes of rows that wait at the most (a phone away from its database for days): beyond it the oldest
/// day's file goes
const MOST_WAITING: u64 = 64 * 1024 * 1024;

/// The days the history is kept, out of the page's setting as it is kept (a text): a whole number of days
/// within what can be set, else the days a new copy has.
pub fn keep_days(setting: Option<&str>) -> i32 {
    setting
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|d| (KEEP_RANGE.0..=KEEP_RANGE.1).contains(d))
        .unwrap_or(KEEP_DAYS)
}

/// A row's data as the page sends it: objects and short lists of numbers, yes-or-no, nothing, and short
/// words (120 characters at most: the name of a sound interface, a look; never a picture or a sound), at
/// most 4 deep. The same with its numbers to four places, or why not.
fn checked(d: &Value, depth: usize) -> Result<Value, String> {
    match d {
        Value::Null | Value::Bool(_) => Ok(d.clone()),
        Value::Number(n) => match n.as_f64().filter(|f| f.is_finite()) {
            Some(f) if n.is_f64() => Ok(json!(format!("{f:.4}").parse::<f64>().unwrap_or(f))),
            Some(_) => Ok(d.clone()),
            None => Err("bad number in a row".to_string()),
        },
        Value::String(s) if s.chars().count() <= 120 && !s.chars().any(char::is_control) => Ok(d.clone()),
        Value::String(_) => Err("a word in a row is too long".to_string()),
        Value::Array(a) if depth < 4 && a.len() <= 64 => {
            Ok(Value::Array(a.iter().map(|v| checked(v, depth + 1)).collect::<Result<_, _>>()?))
        }
        Value::Object(m) if depth < 4 && m.len() <= 200 => {
            let mut out = Map::new();
            for (k, v) in m {
                if k.is_empty() || k.chars().count() > 60 || k.chars().any(char::is_control) {
                    return Err("bad name in a row".to_string());
                }
                out.insert(k.clone(), checked(v, depth + 1)?);
            }
            Ok(Value::Object(out))
        }
        _ => Err("a row's data must be numbers and short words".to_string()),
    }
}

/// One row as the page sends it, checked; why not if it isn't one.
pub fn check(r: &Value, now_ms: i64) -> Result<Value, String> {
    let bad = || "bad row".to_string();
    if !r.is_object() {
        return Err("a row is an object".to_string());
    }
    let name: String = r.get("name").and_then(Value::as_str).ok_or_else(bad)?.trim().chars().take(100).collect();
    // (a face nobody is named for has no row of its own, and the label for ignored faces is no person)
    if name == "Unknown" || name.starts_with('#') || name.chars().any(char::is_control) {
        return Err("a row is the device's or a named person's".to_string());
    }
    let whole = |k: &str| {
        r.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)))
    };
    let t = whole("t").ok_or_else(bad)?;
    if (t - now_ms).abs() > 86_400_000 {
        return Err("row time out of range".to_string());
    }
    let s = r.get("s").and_then(Value::as_f64).filter(|s| (0.5..=3600.0).contains(s)).ok_or_else(bad)?;
    let n = whole("n").filter(|n| (1..=1_000_000).contains(n)).ok_or_else(bad)?;
    let data = r.get("data").filter(|d| d.is_object()).ok_or_else(bad)?;
    if filejson::dumps(data).len() > MAX_DATA {
        return Err("a row's data is too large".to_string());
    }
    Ok(json!({"t": t, "name": name, "s": s, "n": n, "data": checked(data, 0)?}))
}

/// The rows of one request, each checked: one that is no row refuses them all.
pub fn rows(sent: Option<&Value>, now_ms: i64) -> Result<Vec<Value>, String> {
    let sent = sent.and_then(Value::as_array).filter(|r| r.len() <= MAX_ROWS).ok_or("expected a short list of rows")?;
    sent.iter().map(|r| check(r, now_ms)).collect()
}

/// The rows the database couldn't take yet, in the private folder: written and read by the history's own
/// thread, never by a request.
pub struct Spill {
    folder: PathBuf,
    /// (a person's forget rewrites these files too)
    lock: SharedLock,
}

impl Spill {
    pub fn new(home: &Path) -> Spill {
        Spill { folder: home.join(FOLDER), lock: SharedLock::new(home, &format!("waitlog-{FOLDER}")) }
    }

    fn files(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = (std::fs::read_dir(&self.folder).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        found.sort();
        found
    }

    /// Bytes that wait.
    pub fn size(&self) -> u64 {
        self.files().iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum()
    }

    /// Keep rows until the database can take them: each a line at the end of the file of its UTC day. When
    /// more waits than may, the oldest day's file goes first.
    pub fn keep(&self, rows: &[Value]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut by_day: Vec<(String, String)> = Vec::new();
        for r in rows {
            let day = Utc
                .timestamp_millis_opt(r.get("t").and_then(Value::as_i64).unwrap_or(0))
                .single()
                .map(|t| t.format("%Y-%m-%d").to_string())
                .ok_or("bad row")?;
            let line = filejson::dumps_utf8(r) + "\n";
            match by_day.iter_mut().find(|(d, _)| *d == day) {
                Some((_, lines)) => lines.push_str(&line),
                None => by_day.push((day, line)),
            }
        }
        let _held = self.lock.hold();
        std::fs::create_dir_all(&self.folder).map_err(|e| format!("can't make the signals folder: {e}"))?;
        for (day, lines) in by_day {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.folder.join(format!("{day}.jsonl")))
                .and_then(|mut f| f.write_all(lines.as_bytes()))
                .map_err(|e| format!("can't keep the signal history's waiting rows: {e}"))?;
        }
        while self.size() > MOST_WAITING {
            let Some(oldest) = self.files().into_iter().next() else { break };
            if std::fs::remove_file(&oldest).is_err() {
                break;
            }
        }
        Ok(())
    }

    /// The oldest rows that wait, `most` at the most, and the file they are in. A file that holds no row
    /// (lines that are none, or nothing) is removed on the way.
    pub fn oldest(&self, most: usize) -> Option<(PathBuf, Vec<Value>)> {
        let _held = self.lock.hold();
        for file in self.files() {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let rows: Vec<Value> = text
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
                .filter(a_row)
                .take(most)
                .collect();
            if rows.is_empty() {
                gone(&file);
                continue;
            }
            return Some((file, rows));
        }
        None
    }

    /// The database has these rows of that file: they wait no longer. Told by what they are (their time and
    /// name), not by where they stood: a person's forget may have written the file anew meanwhile. Lines that
    /// are no rows go too.
    pub fn done(&self, file: &Path, sent: &[Value]) -> Result<(), String> {
        let _held = self.lock.hold();
        let said = |e: std::io::Error| format!("can't write the signal history's waiting rows: {e}");
        let key =
            |r: &Value| (r.get("t").and_then(Value::as_i64), r.get("name").and_then(Value::as_str).map(str::to_string));
        let taken: std::collections::HashSet<_> = sent.iter().map(key).collect();
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(said(e)),
        };
        let rest: String = (text.lines())
            .filter(|line| {
                serde_json::from_str::<Value>(line.trim()).ok().filter(a_row).is_some_and(|r| !taken.contains(&key(&r)))
            })
            .map(|line| format!("{}\n", line.trim_end()))
            .collect();
        if rest.is_empty() {
            gone(file);
            return Ok(());
        }
        crate::paths::atomic_write(file, rest.as_bytes()).map_err(said)
    }
}

/// A waiting file is done with: it goes, and with it the mark a person's forget may have put beside it (the
/// other waiting logs are sent anew from their start by that mark; this one's rows are told by what they are).
fn gone(file: &Path) {
    let _ = std::fs::remove_file(file);
    let mut mark = file.as_os_str().to_os_string();
    mark.push(".again");
    let _ = std::fs::remove_file(PathBuf::from(mark));
}

/// Whether a line of a waiting file is a row: it has its time, its name (which may be empty) and its data.
fn a_row(r: &Value) -> bool {
    r.get("t").is_some_and(Value::is_i64)
        && r.get("name").is_some_and(Value::is_string)
        && r.get("data").is_some_and(Value::is_object)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_161_900_000; // 2026-11-20 08:05 UTC

    fn row(name: &str, t: i64) -> Value {
        json!({"t": t, "name": name, "s": 10, "n": 3, "data": {"pulse": [70.123456, 60, 80, 3], "look": {"happy": 2}, "odd": 1}})
    }

    #[test]
    fn a_row_is_numbers_and_short_words_for_the_device_or_a_named_person() {
        let r = check(&row(" Zoë ", NOW), NOW).unwrap();
        assert_eq!(
            r,
            json!({"t": NOW, "name": "Zoë", "s": 10.0, "n": 3, "data": {"pulse": [70.1235, 60, 80, 3], "look": {"happy": 2}, "odd": 1}})
        );
        // the device's own row has no name, and holds what its sound interfaces heard
        let device = json!({"t": NOW, "name": "", "s": 10, "n": 200,
                            "data": {"checks": [20, 18, 22, 200], "heat": {"fair": 200},
                                     "sound": [{"name": "Speakers (Realtek)", "kind": "output", "what": {"music": 2}, "level": [-20, -22, -18, 2]}]}});
        assert_eq!(check(&device, NOW).unwrap()["data"]["sound"][0]["what"]["music"], 2);
        for (key, value, why) in [
            ("name", json!("Unknown"), "named person"),
            ("name", json!("#ignore"), "named person"),
            ("name", json!(7), "bad row"),
            ("t", json!("soon"), "bad row"),
            ("t", json!(NOW - 2 * 86_400_000), "time out of range"),
            ("s", json!(0), "bad row"),
            ("s", json!(99_999), "bad row"),
            ("n", json!(0), "bad row"),
            ("n", json!(2.5), "bad row"),
            ("data", json!([1]), "bad row"),
            ("data", json!({"word": "x".repeat(121)}), "too long"),
            ("data", json!({"a": {"b": {"c": {"d": {"e": 1}}}}}), "numbers and short words"),
            ("data", json!({"list": vec![1; 65]}), "numbers and short words"),
            ("data", json!({"": 1}), "bad name"),
            (
                "data",
                json!({"big": vec!["x".repeat(100); 60], "more": vec!["y".repeat(100); 60], "and": vec!["z".repeat(100); 60], "on": vec!["w".repeat(100); 60]}),
                "too large",
            ),
        ] {
            let mut bad = row("A", NOW);
            bad[key] = value;
            let e = check(&bad, NOW).unwrap_err();
            assert!(e.contains(why), "{key}: {e}");
            assert!(rows(Some(&json!([row("A", NOW), bad])), NOW).is_err(), "one bad row refuses its whole request");
        }
        assert_eq!(rows(Some(&json!([row("A", NOW), row("", NOW)])), NOW).unwrap().len(), 2);
        for not_a_list in [None, Some(json!({"rows": 1})), Some(json!(vec![row("A", NOW); MAX_ROWS + 1]))] {
            assert_eq!(rows(not_a_list.as_ref(), NOW).unwrap_err(), "expected a short list of rows");
        }
    }

    #[test]
    fn the_days_the_history_is_kept_are_the_owners_or_a_new_copys() {
        assert_eq!(keep_days(None), KEEP_DAYS);
        assert_eq!(keep_days(Some("30")), 30);
        assert_eq!(keep_days(Some(" 365 ")), 365);
        for not in ["", "0", "-5", "forever", "99999", "30.5"] {
            assert_eq!(keep_days(Some(not)), KEEP_DAYS, "{not}");
        }
    }

    #[test]
    fn rows_the_database_could_not_take_wait_in_the_file_of_their_day_and_go_oldest_first() {
        let dir = std::env::temp_dir().join(format!("face-siglog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let spill = Spill::new(&dir);
        assert!(spill.oldest(10).is_none() && spill.size() == 0);
        let day = 86_400_000;
        let waiting: Vec<Value> = [row("A", NOW), row("", NOW + 1000), row("B", NOW - day), row("A", NOW + 2000)]
            .iter()
            .map(|r| check(r, NOW).unwrap())
            .collect();
        spill.keep(&waiting).unwrap();
        assert!(
            dir.join("signals").join("2026-11-19.jsonl").is_file()
                && dir.join("signals").join("2026-11-20.jsonl").is_file()
        );
        // the oldest day first, then the next, a few at a time
        let (file, rows) = spill.oldest(10).unwrap();
        assert_eq!(rows.iter().map(|r| r["name"].as_str().unwrap()).collect::<Vec<_>>(), ["B"]);
        spill.done(&file, &rows).unwrap();
        assert!(!file.exists(), "a file whose rows all went is gone");
        let (file, rows) = spill.oldest(2).unwrap();
        assert_eq!(rows.iter().map(|r| r["t"].as_i64().unwrap()).collect::<Vec<_>>(), [NOW, NOW + 1000]);
        // rows that came meanwhile, and a file written anew meanwhile (a person forgotten), change nothing:
        // what the database has goes, by what it is
        spill.keep(&[check(&row("C", NOW + 3000), NOW).unwrap()]).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let anew: String = text.lines().rev().map(|l| format!("{l}\r\n")).collect();
        std::fs::write(&file, anew).unwrap();
        spill.done(&file, &rows).unwrap();
        let (file, rows) = spill.oldest(5).unwrap();
        let mut left: Vec<i64> = rows.iter().map(|r| r["t"].as_i64().unwrap()).collect();
        left.sort_unstable();
        assert_eq!(left, [NOW + 2000, NOW + 3000]);
        spill.done(&file, &rows).unwrap();
        assert!(spill.oldest(5).is_none() && spill.size() == 0);
        // a line that is no row doesn't hold the others up, and a file of nothing but such lines goes
        std::fs::create_dir_all(dir.join("signals")).unwrap();
        std::fs::write(dir.join("signals").join("2026-11-18.jsonl"), b"not a row\n{\"t\": \"soon\"}\n").unwrap();
        std::fs::write(
            dir.join("signals").join("2026-11-19.jsonl"),
            b"not a row\n{\"t\": 5, \"name\": \"D\", \"s\": 10, \"n\": 1, \"data\": {}}\n",
        )
        .unwrap();
        let (file, rows) = spill.oldest(5).unwrap();
        assert_eq!((rows.len(), rows[0]["name"].as_str()), (1, Some("D")));
        assert!(!dir.join("signals").join("2026-11-18.jsonl").exists());
        spill.done(&file, &rows).unwrap();
        assert!(spill.oldest(5).is_none() && spill.size() == 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
