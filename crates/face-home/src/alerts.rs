//! The tracker's alert inbox: every alert it raised (a chosen person
//! arrived, left or had unusual signals; an unknown face stayed; someone moved between cameras; a new device
//! joined the shared database), kept until the owner deletes it, in `HOME/alerts.json`.
//!
//! An alert is {id, t (epoch ms), kind, name, text, state, details}; `details` holds what the tracker
//! measured when the alert was raised ("first") and 15 seconds later ("later"): numbers and short text
//! only, never a picture or sound (`check_details`). The newest `KEEP` are kept.

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde_json::{Map, Value, json};

use crate::{
    filejson,
    paths::{SharedLock, atomic_write},
};

const KINDS: [&str; 6] = ["arrived", "left", "unusual", "unknown", "moved", "joined"];
const STATES: [&str; 3] = ["new", "ack", "archived"];
const KEEP: usize = 500;
/// bytes of JSON in one set of details, in the files' one-line spelling
pub const DETAILS_MAX: usize = 20_000;

pub struct Inbox {
    file: PathBuf,
    /// the tracker and the sync both rewrite the inbox
    lock: SharedLock,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A set of details as the page sends it: objects of numbers, short text (500 characters at most: an
/// evidence line, never a picture), true, false and null, at most 4 deep and `DETAILS_MAX` bytes. The same
/// with its numbers rounded, or why not.
pub fn check_details(d: &Value) -> Result<Value, String> {
    if filejson::dumps(d).len() > DETAILS_MAX {
        return Err("details too large".to_string());
    }
    fn checked(d: &Value, depth: usize) -> Result<Value, String> {
        match d {
            Value::Null | Value::Bool(_) => Ok(d.clone()),
            Value::Number(n) if n.is_f64() => {
                let f = n.as_f64().unwrap_or(0.0);
                Ok(json!(format!("{f:.4}").parse::<f64>().unwrap_or(f)))
            }
            Value::Number(_) => Ok(d.clone()),
            Value::String(s) if s.chars().count() > 500 => Err("detail text too long".to_string()),
            Value::String(_) => Ok(d.clone()),
            Value::Object(m) if depth < 4 => {
                if m.keys().any(|k| k.chars().count() > 40) {
                    return Err("bad detail name".to_string());
                }
                let mut out = Map::new();
                for (k, v) in m {
                    out.insert(k.clone(), checked(v, depth + 1)?);
                }
                Ok(Value::Object(out))
            }
            _ => Err("details must be objects of numbers and short text".to_string()),
        }
    }
    checked(d, 0)
}

fn t_of(row: &Value) -> i64 {
    row.get("t").and_then(Value::as_f64).unwrap_or(0.0) as i64
}

fn state_of(row: &Value) -> &str {
    row.get("state").and_then(Value::as_str).unwrap_or("")
}

fn counts(rows: &[Value]) -> Value {
    let n = |s: &str| rows.iter().filter(|r| state_of(r) == s).count();
    json!({"new": n("new"), "ack": n("ack"), "archived": n("archived")})
}

impl Inbox {
    pub fn new(home: &Path) -> Inbox {
        Inbox { file: home.join("alerts.json"), lock: SharedLock::new(home, "alerts") }
    }

    /// The inbox's alerts. A file that is there but can't be read is an error, never taken as empty: it
    /// would be written back empty.
    fn load(&self) -> Result<Vec<Value>, String> {
        let bytes = match std::fs::read(&self.file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("can't read alerts.json: {e}")),
        };
        // not JSON, or not a list: an empty inbox
        let rows = serde_json::from_slice::<Value>(&bytes).ok();
        let rows = rows.and_then(|v| if let Value::Array(a) = v { Some(a) } else { None }).unwrap_or_default();
        Ok(rows
            .into_iter()
            .filter(|r| r.get("id").is_some_and(|i| i.as_str().is_some_and(|s| !s.is_empty())))
            .collect())
    }

    fn save(&self, rows: &[Value]) -> Result<(), String> {
        let text = filejson::dumps_indent1(&Value::Array(rows.to_vec()));
        atomic_write(&self.file, text.as_bytes()).map_err(|e| format!("can't write alerts.json: {e}"))
    }

    pub fn counts(&self) -> Result<Value, String> {
        let _held = self.lock.hold();
        Ok(counts(&self.load()?))
    }

    /// The alerts naming a person (their export).
    pub fn about(&self, name: &str) -> Result<Vec<Value>, String> {
        Ok(self.all()?.into_iter().filter(|r| r.get("name").and_then(Value::as_str) == Some(name)).collect())
    }

    /// Delete the alerts naming a person; how many.
    pub fn forget(&self, name: &str) -> Result<usize, String> {
        let _held = self.lock.hold();
        let rows = self.load()?;
        let keep: Vec<Value> =
            rows.iter().filter(|r| r.get("name").and_then(Value::as_str) != Some(name)).cloned().collect();
        if keep.len() < rows.len() {
            self.save(&keep)?;
        }
        Ok(rows.len() - keep.len())
    }

    /// Every alert, in the file's order (the sync sends them).
    pub fn all(&self) -> Result<Vec<Value>, String> {
        let _held = self.lock.hold();
        self.load()
    }

    /// The inbox changed as a whole, under its lock (the sync takes other devices' alerts): `change` is
    /// handed every alert and gives back the alerts to keep, which are written oldest first.
    pub fn change(&self, change: impl FnOnce(Vec<Value>) -> Vec<Value>) -> Result<Vec<Value>, String> {
        let _held = self.lock.hold();
        let mut rows = change(self.load()?);
        rows.sort_by(|a, b| {
            let t = |r: &Value| r.get("t").and_then(Value::as_f64).unwrap_or(0.0);
            t(a).total_cmp(&t(b))
        });
        self.save(&rows)?;
        Ok(rows)
    }

    /// {"alerts": newest first, "counts": {state: n}}
    pub fn listing(&self) -> Result<Value, String> {
        let mut rows = {
            let _held = self.lock.hold();
            self.load()?
        };
        let all = counts(&rows);
        rows.sort_by_key(|r| std::cmp::Reverse(t_of(r)));
        Ok(json!({"alerts": rows, "counts": all}))
    }

    /// Keep one new alert from the tracker page ({kind, name, text, t, details}), checked. The alert as kept.
    pub fn add(&self, a: &Value) -> Result<Value, String> {
        let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(12).collect();
        self.add_as(&id, a)?.ok_or_else(|| "bad alert".to_string())
    }

    /// The same with an id of the caller's (twelve hex digits, as every alert's): what several devices each
    /// notice for themselves (a new device in the shared database) is one alert, whichever of them raised it.
    /// None when an alert of that id is here already.
    pub fn add_as(&self, id: &str, a: &Value) -> Result<Option<Value>, String> {
        if id.len() != 12 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err("bad alert".to_string());
        }
        let kind = a.get("kind").and_then(Value::as_str).filter(|k| KINDS.contains(k));
        let (Some(kind), true) = (kind, a.is_object()) else {
            return Err(format!("expected an alert of kind {}", KINDS.join(" or ")));
        };
        let bad = || "bad alert".to_string();
        let name: String = filejson::said(a.get("name")).ok_or_else(bad)?.chars().take(100).collect();
        // runs of white space become one space; then 200 characters, without white space at the ends
        let text = filejson::said(a.get("text")).ok_or_else(bad)?;
        let mut one = String::new();
        for c in text.chars() {
            if !c.is_whitespace() {
                one.push(c);
            } else if !one.ends_with(' ') {
                one.push(' ');
            }
        }
        let text: String = one.chars().take(200).collect::<String>().trim().to_string();
        let now = now_ms();
        let t = match a.get("t") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => now,
            Some(Value::Number(n)) if n.as_f64() == Some(0.0) => now,
            Some(Value::Number(n)) => n.as_f64().map(|f| f as i64).ok_or("bad alert")?,
            Some(Value::String(s)) if s.is_empty() => now,
            Some(Value::String(s)) => s.trim().parse::<i64>().map_err(|_| "bad alert".to_string())?,
            Some(Value::Bool(true)) => 1,
            Some(_) => return Err("bad alert".to_string()),
        };
        if text.is_empty() || (kind != "unknown" && (name.is_empty() || name == "Unknown")) {
            return Err("bad alert".to_string());
        }
        if (t - now).abs() > 7 * 86_400_000 {
            return Err("alert time out of range".to_string());
        }
        let mut rec = json!({"id": id, "t": t, "kind": kind, "name": if kind != "unknown" { name.as_str() } else { "" },
                             "text": text, "state": "new"});
        if let Some(details) = a.get("details").filter(|d| !d.is_null()) {
            rec["details"] = json!({"first": check_details(details)?});
        }
        let _held = self.lock.hold();
        let mut rows = self.load()?;
        if rows.iter().any(|r| r.get("id").and_then(Value::as_str) == Some(id)) {
            return Ok(None);
        }
        rows.push(rec.clone());
        if rows.len() > KEEP {
            // the oldest archived go first, then the oldest of the rest
            rows.sort_by_key(|r| (state_of(r) != "archived", t_of(r)));
            rows.drain(..rows.len() - KEEP);
        }
        self.save(&rows)?;
        Ok(Some(rec))
    }

    /// Move alerts to a state ("new", "ack", "archived") or delete them ("delete"): those listed in `ids`,
    /// or every alert now in state `where_`. The counts afterwards.
    pub fn set_state(
        &self,
        ids: Option<&Value>,
        state: Option<&Value>,
        where_: Option<&Value>,
    ) -> Result<Value, String> {
        let present = |v: Option<&Value>| v.filter(|v| !v.is_null()).cloned();
        let (ids, where_) = (present(ids), present(where_));
        // a request that names no state acknowledges
        let state = match present(state) {
            None => "ack".to_string(),
            Some(Value::String(s)) => s,
            Some(_) => return Err("unknown state".to_string()),
        };
        if !STATES.contains(&state.as_str()) && state != "delete" {
            return Err("unknown state".to_string());
        }
        let where_ = match where_ {
            None => None,
            Some(Value::String(s)) if STATES.contains(&s.as_str()) => Some(s),
            Some(_) => return Err("unknown state".to_string()),
        };
        let ids: Option<Vec<String>> = match ids {
            None => None,
            Some(Value::Array(list)) => Some(
                list.iter()
                    .map(|i| i.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or("ids must be a list")?,
            ),
            Some(_) => return Err("ids must be a list".to_string()),
        };
        if ids.is_none() && where_.is_none() {
            return Err("say which alerts".to_string());
        }
        let pick = |r: &Value| match (&ids, &where_) {
            (Some(ids), _) => r.get("id").and_then(Value::as_str).is_some_and(|id| ids.iter().any(|i| i == id)),
            (None, Some(w)) => state_of(r) == w,
            (None, None) => false,
        };
        let _held = self.lock.hold();
        let mut rows = self.load()?;
        if state == "delete" {
            rows.retain(|r| !pick(r));
        } else {
            for r in rows.iter_mut().filter(|r| pick(r)) {
                r["state"] = json!(state);
            }
        }
        self.save(&rows)?;
        Ok(counts(&rows))
    }

    /// Add the later set of details to an alert (the page measures again 15 s after raising it). Whether
    /// the alert is still there.
    pub fn add_details(
        &self,
        id: Option<&Value>,
        which: Option<&Value>,
        details: Option<&Value>,
    ) -> Result<bool, String> {
        if which.and_then(Value::as_str) != Some("later") {
            return Err("which must be 'later'".to_string());
        }
        let id = id.and_then(Value::as_str).ok_or("bad alert id")?;
        let details = check_details(details.unwrap_or(&Value::Null))?;
        let _held = self.lock.hold();
        let mut rows = self.load()?;
        let Some(row) = rows.iter_mut().find(|r| r.get("id").and_then(Value::as_str) == Some(id)) else {
            return Ok(false);
        };
        if !row.get("details").is_some_and(Value::is_object) {
            row["details"] = json!({});
        }
        row["details"]["later"] = details;
        self.save(&rows)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox(name: &str) -> (Inbox, PathBuf) {
        let dir = std::env::temp_dir().join(format!("face-alerts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Inbox::new(&dir), dir)
    }

    #[test]
    fn an_alert_is_kept_listed_and_moved_between_states() {
        let (inbox, dir) = inbox("states");
        assert_eq!(inbox.listing().unwrap(), json!({"alerts": [], "counts": {"new": 0, "ack": 0, "archived": 0}}));
        let now = now_ms();
        let first = inbox
            .add(&json!({"kind": "arrived", "name": "Zoë", "text": "  Zoë \n arrived ", "t": now - 5000,
                         "details": {"sim": 0.61234567, "label": "neutral", "n": 3}}))
            .unwrap();
        assert_eq!(first["id"].as_str().unwrap().len(), 12);
        assert_eq!((first["text"].clone(), first["state"].clone()), (json!("Zoë arrived"), json!("new")));
        assert_eq!(first["details"], json!({"first": {"sim": 0.6123, "label": "neutral", "n": 3}}));
        let second =
            inbox.add(&json!({"kind": "unknown", "name": "whoever", "text": "An unknown face stayed"})).unwrap();
        assert_eq!(second["name"], json!(""), "an unknown face has no name");
        let list = inbox.listing().unwrap();
        assert_eq!(list["counts"], json!({"new": 2, "ack": 0, "archived": 0}));
        assert_eq!(list["alerts"][0]["id"], second["id"], "newest first");
        // the file's fixed form: a list, a line a thing, names as they are
        let file = std::fs::read_to_string(dir.join("alerts.json")).unwrap();
        assert!(file.starts_with("[\n {\n  \"id\": \"") && file.contains("\"name\": \"Zoë\""), "{file}");

        assert!(inbox.add_details(Some(&first["id"]), Some(&json!("later")), Some(&json!({"sim": 0.5}))).unwrap());
        assert!(!inbox.add_details(Some(&json!("gone")), Some(&json!("later")), Some(&json!({}))).unwrap());
        assert!(inbox.add_details(Some(&first["id"]), Some(&json!("first")), Some(&json!({}))).is_err());
        assert_eq!(
            inbox.set_state(Some(&json!([first["id"]])), Some(&json!("ack")), None).unwrap(),
            json!({"new": 1, "ack": 1, "archived": 0})
        );
        assert_eq!(
            inbox.set_state(None, Some(&json!("archived")), Some(&json!("new"))).unwrap(),
            json!({"new": 0, "ack": 1, "archived": 1})
        );
        let kept = inbox.listing().unwrap();
        let acked = kept["alerts"].as_array().unwrap().iter().find(|a| a["id"] == first["id"]).unwrap();
        assert_eq!(acked["details"]["later"], json!({"sim": 0.5}));
        assert_eq!(
            inbox.set_state(None, Some(&json!("delete")), Some(&json!("archived"))).unwrap(),
            json!({"new": 0, "ack": 1, "archived": 0})
        );
        for (ids, state, where_) in [
            (None, Some(json!("ack")), None),
            (Some(json!("not a list")), Some(json!("ack")), None),
            (Some(json!([1])), Some(json!("ack")), None),
            (Some(json!([])), Some(json!("gone")), None),
            (None, Some(json!("ack")), Some(json!("delete"))),
        ] {
            assert!(
                inbox.set_state(ids.as_ref(), state.as_ref(), where_.as_ref()).is_err(),
                "{ids:?} {state:?} {where_:?}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_is_not_an_alert_is_refused() {
        let (inbox, dir) = inbox("refused");
        let now = now_ms();
        for bad in [
            json!("an alert"),
            json!({"kind": "exploded", "name": "A", "text": "x"}),
            json!({"kind": "arrived", "name": "", "text": "x"}),
            json!({"kind": "arrived", "name": "Unknown", "text": "x"}),
            json!({"kind": "arrived", "name": "A", "text": " \n "}),
            json!({"kind": "arrived", "name": "A", "text": "x", "t": now - 8 * 86_400_000}),
            json!({"kind": "arrived", "name": "A", "text": "x", "t": "soon"}),
            json!({"kind": "arrived", "name": ["A"], "text": "x"}),
            json!({"kind": "arrived", "name": "A", "text": "x", "details": [1, 2]}),
            json!({"kind": "arrived", "name": "A", "text": "x", "details": {"a": {"b": {"c": {"d": {"e": 1}}}}}}),
            json!({"kind": "arrived", "name": "A", "text": "x", "details": {"picture": "x".repeat(501)}}),
            json!({"kind": "arrived", "name": "A", "text": "x", "details": {"n".repeat(41): 1}}),
        ] {
            assert!(inbox.add(&bad).is_err(), "kept: {bad}");
        }
        assert!(!dir.join("alerts.json").exists(), "nothing was written");
        // as large as a set of details may be, counted in the files' one-line spelling
        let many: Map<String, Value> = (0..1300).map(|i| (format!("k{i:04}"), json!(0.5))).collect();
        assert!(filejson::dumps(&Value::Object(many.clone())).len() < DETAILS_MAX);
        assert!(check_details(&Value::Object(many)).is_ok());
        let more: Map<String, Value> = (0..1700).map(|i| (format!("k{i:04}"), json!(0.5))).collect();
        assert_eq!(check_details(&Value::Object(more)).unwrap_err(), "details too large");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_newest_are_kept_and_archived_ones_go_first() {
        let (inbox, dir) = inbox("keep");
        let now = now_ms();
        let rows: Vec<Value> = (0..KEEP as i64)
            .map(|i| {
                json!({"id": format!("old{i}"), "t": now - 100_000 + i, "kind": "left", "name": "A", "text": "x",
                            "state": if i == 250 { "archived" } else { "new" }})
            })
            .collect();
        inbox.save(&rows).unwrap();
        inbox.add(&json!({"kind": "left", "name": "A", "text": "left"})).unwrap();
        let kept = inbox.load().unwrap();
        assert_eq!(kept.len(), KEEP);
        assert!(!kept.iter().any(|r| r["id"] == "old250"), "the archived one went, not the oldest");
        assert!(kept.iter().any(|r| r["id"] == "old0"));
        // a file that is not a list, or holds things that are no alerts, is an empty inbox, not an error
        std::fs::write(dir.join("alerts.json"), b"{\"not\": \"a list\"}").unwrap();
        assert_eq!(inbox.load().unwrap().len(), 0);
        std::fs::write(dir.join("alerts.json"), b"[5, {\"id\": \"\"}, {\"id\": \"a\", \"state\": \"new\"}]").unwrap();
        assert_eq!(inbox.counts().unwrap(), json!({"new": 1, "ack": 0, "archived": 0}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_several_devices_notice_is_one_alert() {
        let (inbox, dir) = inbox("once");
        let joined = json!({"kind": "joined", "name": "Kitchen tablet", "text": "A new device joined your shared database: Kitchen tablet",
                            "details": {"device": "Kitchen tablet", "system": "Android 16", "version": "2.5.0"}});
        let kept = inbox.add_as("0123456789ab", &joined).unwrap().expect("a new alert");
        assert_eq!(
            (kept["id"].as_str(), kept["kind"].as_str(), kept["state"].as_str()),
            (Some("0123456789ab"), Some("joined"), Some("new"))
        );
        assert_eq!(kept["details"]["first"]["system"], "Android 16");
        // noticed again, here or by another device whose alert came with the sync: nothing new
        assert_eq!(inbox.add_as("0123456789ab", &joined).unwrap(), None);
        inbox.set_state(Some(&json!(["0123456789ab"])), Some(&json!("ack")), None).unwrap();
        assert_eq!(inbox.add_as("0123456789ab", &joined).unwrap(), None);
        assert_eq!(inbox.counts().unwrap(), json!({"new": 0, "ack": 1, "archived": 0}));
        // an id that isn't an alert's, and a device without a name
        for bad in ["", "0123456789a", "0123456789abc", "0123456789AB", "0123456789a/"] {
            assert!(inbox.add_as(bad, &joined).is_err(), "taken as an id: {bad}");
        }
        assert!(inbox.add_as("ba9876543210", &json!({"kind": "joined", "name": "", "text": "x"})).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
