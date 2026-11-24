//! What a page remembers, kept by its own server: its settings
//! in `HOME/settings.json` ({app: {key: value}}) and what the tracker learned about each person in
//! `HOME/person_signals.json` ({kind: {name: data}}), in the files' existing layout. Values are the pages'
//! own strings; nothing here is a measured threshold or a default.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

use crate::paths::{SharedLock, atomic_write};

const SETTINGS: &str = "settings.json";
const SIGNALS: &str = "person_signals.json";
const FORGOTTEN: &str = "forgotten.json";
/// kind -> the page's storage key
const KINDS: [(&str, &str); 2] = [("usual", "ft-usual"), ("baseline", "ft-baselines")];
/// kinds the programs keep themselves, never handed to a page (a person's usual music), after the pages'
pub const ALL_KINDS: [&str; 3] = ["usual", "baseline", "music"];
/// A kind of person signal with what is kept of it: {name: data}.
pub type Kind = (&'static str, Map<String, Value>);
const NOT_SETTINGS: [&str; 3] = ["ft-usual", "ft-baselines", "ft-forgot-done"];
const MAX_VALUE: usize = 20_000;
pub const MAX_SIGNALS: usize = 5_000_000;

pub struct Store {
    home: PathBuf,
    /// one change at a time, also against the app's other process (the lock of the same name)
    lock: SharedLock,
}

fn prefix(app: &str) -> Option<&'static str> {
    match app {
        "track" => Some("ft-"),
        "label" => Some("fl-"),
        _ => None,
    }
}

impl Store {
    pub fn new(home: &Path) -> Store {
        Store { home: home.to_path_buf(), lock: SharedLock::new(home, "settings") }
    }

    /// A file's object; a file that is there but can't be read is an error, never taken as empty (it would
    /// be written back empty).
    fn read(&self, name: &str) -> Result<Map<String, Value>, String> {
        match std::fs::read(self.home.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
            Err(e) => Err(format!("can't read {name}: {e}")),
            Ok(b) => match serde_json::from_slice::<Value>(&b) {
                Ok(Value::Object(m)) => Ok(m),
                Ok(_) => Ok(Map::new()),
                Err(e) => Err(format!("can't read {name}: {e}")),
            },
        }
    }

    /// (the file's fixed form: a line a thing, no indent)
    fn write(&self, name: &str, obj: &Map<String, Value>) -> Result<(), String> {
        let text = face_files::filejson::dumps_lines(&Value::Object(obj.clone()), false, 0);
        atomic_write(&self.home.join(name), text.as_bytes()).map_err(|e| format!("can't write {name}: {e}"))
    }

    /// Every app's settings: {app: {key: value}} (the sync keeps a copy of them in the shared database).
    pub fn settings(&self) -> Result<Map<String, Value>, String> {
        let _one = self.lock.hold();
        self.read(SETTINGS)
    }

    /// {kind: {name: data}} of every kind of person signal, the programs' own kinds too (the sync shares
    /// them all).
    pub fn signals_every(&self) -> Result<Vec<Kind>, String> {
        let _one = self.lock.hold();
        let s = self.read(SIGNALS)?;
        Ok(ALL_KINDS
            .iter()
            .map(|kind| (*kind, s.get(*kind).and_then(Value::as_object).cloned().unwrap_or_default()))
            .collect())
    }

    /// {kind: data} held about a person (their export).
    pub fn of(&self, name: &str) -> Result<Map<String, Value>, String> {
        Ok(self
            .signals_every()?
            .into_iter()
            .filter_map(|(kind, people)| Some((kind.to_string(), people.get(name)?.clone())))
            .collect())
    }

    /// Drop a person from every kind; how many rows went.
    pub fn forget_signals(&self, name: &str) -> Result<usize, String> {
        let _one = self.lock.hold();
        let mut all = self.read(SIGNALS)?;
        let mut n = 0;
        for kind in ALL_KINDS {
            if let Some(people) = all.get_mut(kind).and_then(Value::as_object_mut) {
                n += usize::from(people.shift_remove(name).is_some());
            }
        }
        if n > 0 {
            self.write(SIGNALS, &all)?;
        }
        Ok(n)
    }

    /// One person's row of one kind (the sync, and the programs' own kinds); no data removes it.
    pub fn put_signal(&self, kind: &str, name: &str, data: Option<&Value>) -> Result<(), String> {
        let fits = data.is_none_or(|d| d.is_object() && face_files::filejson::dumps(d).chars().count() <= MAX_SIGNALS);
        if !ALL_KINDS.contains(&kind) || name.is_empty() || name.starts_with('#') || !fits {
            return Err("bad signals".to_string());
        }
        let _one = self.lock.hold();
        let mut all = self.read(SIGNALS)?;
        let entry = all.entry(kind.to_string()).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        let people = entry.as_object_mut().expect("an object");
        match data {
            Some(d) => {
                people.insert(name.to_string(), d.clone());
            }
            None => {
                people.shift_remove(name);
            }
        }
        self.write(SIGNALS, &all)
    }

    /// Names forgotten here or on another device and not given to anyone since.
    pub fn forgotten(&self) -> BTreeSet<String> {
        std::fs::read(self.home.join(FORGOTTEN))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok())
            .map(|v| v.into_iter().filter_map(|n| n.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    fn signals(&self) -> Result<Map<String, Value>, String> {
        let s = self.read(SIGNALS)?;
        Ok(KINDS
            .iter()
            .map(|(kind, key)| (key.to_string(), s.get(*kind).filter(|v| v.is_object()).cloned().unwrap_or(json!({}))))
            .collect())
    }

    /// What a page restores at load: its settings, and for the tracker the person signals and forgotten names.
    pub fn page_state(&self, app: &str) -> Result<Value, String> {
        let _one = self.lock.hold();
        let settings = self.read(SETTINGS)?.get(app).filter(|v| v.is_object()).cloned().unwrap_or(json!({}));
        let mut out = json!({"settings": settings});
        if app == "track" {
            out["signals"] = Value::Object(self.signals()?);
            out["forgotten"] = json!(self.forgotten());
        }
        Ok(out)
    }

    /// One setting of an app's page, as the page keeps it (text), when it has one.
    pub fn setting(&self, app: &str, key: &str) -> Option<String> {
        let _one = self.lock.hold();
        self.read(SETTINGS).ok()?.get(app)?.get(key)?.as_str().map(str::to_string)
    }

    /// Set (or with null, remove) some of an app's settings; the app's settings after it. Checked: keys of
    /// that page, short text values.
    pub fn update(&self, app: &str, changes: &Value) -> Result<Value, String> {
        let (Some(prefix), Some(changes)) = (prefix(app), changes.as_object()) else {
            return Err("bad settings".to_string());
        };
        for (k, v) in changes {
            let word = !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'));
            if !k.starts_with(prefix) || k.chars().count() > 80 || NOT_SETTINGS.contains(&k.as_str()) || !word {
                return Err("bad setting name".to_string());
            }
            match v {
                Value::Null => {}
                Value::String(s) if s.chars().count() <= MAX_VALUE => {}
                _ => return Err("bad setting value".to_string()),
            }
        }
        let _one = self.lock.hold();
        let mut all = self.read(SETTINGS)?;
        let mine = all.entry(app.to_string()).or_insert_with(|| json!({}));
        if !mine.is_object() {
            *mine = json!({});
        }
        let map = mine.as_object_mut().expect("an object");
        for (k, v) in changes {
            if v.is_null() {
                map.shift_remove(k);
            } else {
                map.insert(k.clone(), v.clone());
            }
        }
        let after = Value::Object(map.clone());
        self.write(SETTINGS, &all)?;
        Ok(after)
    }

    /// What the tracker page learned since it last said: `changed` {name: data} is set, the names in `gone`
    /// are removed, nobody else is touched. A forgotten name is refused until it is given to someone again,
    /// so a page still running can't bring a forgotten person back.
    pub fn merge_signals(&self, key: &str, changed: &Value, gone: &Value) -> Result<(), String> {
        let Some((kind, _)) = KINDS.iter().find(|(_, k)| *k == key) else {
            return Err("bad signals".to_string());
        };
        let (Some(changed), Some(gone)) = (changed.as_object(), gone.as_array()) else {
            return Err("bad signals".to_string());
        };
        let refused = self.forgotten();
        let _one = self.lock.hold();
        let mut all = self.read(SIGNALS)?;
        let entry = all.entry(kind.to_string()).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        let people = entry.as_object_mut().expect("an object");
        for (name, data) in changed {
            let name: String = name.chars().take(100).collect();
            if data.is_object() && !name.is_empty() && !name.starts_with('#') && !refused.contains(&name) {
                people.insert(name, data.clone());
            }
        }
        for name in gone {
            if let Some(n) = name.as_str() {
                people.shift_remove(n);
            }
        }
        self.write(SIGNALS, &all)
    }

    /// For the tracker page's check every half minute: {v} and, unless the page already has version `have`,
    /// {signals, forgotten}: what other devices learned or forgot arrives in the files through the sync.
    pub fn signals_state(&self, have: &str) -> Result<Value, String> {
        let stamp = |name: &str| match std::fs::metadata(self.home.join(name)) {
            Ok(m) => {
                let t = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                format!("{t}.{}", m.len())
            }
            Err(_) => "-".to_string(),
        };
        let v = format!("{}/{}", stamp(SIGNALS), stamp(FORGOTTEN));
        if have == v {
            return Ok(json!({"v": v, "same": true}));
        }
        let _one = self.lock.hold();
        Ok(json!({"v": v, "signals": self.signals()?, "forgotten": self.forgotten()}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("face-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Store::new(&dir), dir)
    }

    #[test]
    fn a_pages_settings_are_kept_and_checked() {
        let (s, dir) = store("settings");
        assert_eq!(
            s.page_state("track").unwrap(),
            json!({"settings": {}, "signals": {"ft-usual": {}, "ft-baselines": {}}, "forgotten": []})
        );
        assert_eq!(s.update("track", &json!({"ft-a": "1", "ft-b": "x"})).unwrap(), json!({"ft-a": "1", "ft-b": "x"}));
        assert_eq!(s.update("track", &json!({"ft-a": null})).unwrap(), json!({"ft-b": "x"}));
        assert_eq!(s.page_state("label").unwrap(), json!({"settings": {}}));
        for bad in
            [json!({"fl-a": "1"}), json!({"ft-usual": "{}"}), json!({"ft-a b": "1"}), json!({"ft-a": 1}), json!([1])]
        {
            assert!(s.update("track", &bad).is_err(), "{bad}");
        }
        assert!(s.update("other", &json!({})).is_err());
        std::fs::write(dir.join(SETTINGS), b"{ not json").unwrap();
        assert!(s.update("track", &json!({"ft-a": "1"})).is_err(), "a file that can't be read is never taken as empty");
        assert_eq!(std::fs::read(dir.join(SETTINGS)).unwrap(), b"{ not json");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_was_learned_is_merged_per_person() {
        let (s, dir) = store("signals");
        s.merge_signals(
            "ft-usual",
            &json!({"Ann": {"n": 1}, "Bo": {"n": 2}, "#3": {"n": 9}, "Cy": "text"}),
            &json!([]),
        )
        .unwrap();
        s.merge_signals("ft-usual", &json!({"Bo": {"n": 3}}), &json!(["Ann", "nobody"])).unwrap();
        let state = s.signals_state("").unwrap();
        assert_eq!(state["signals"], json!({"ft-usual": {"Bo": {"n": 3}}, "ft-baselines": {}}));
        assert_eq!(s.signals_state(state["v"].as_str().unwrap()).unwrap()["same"], json!(true));
        std::fs::write(dir.join(FORGOTTEN), br#"["Dee"]"#).unwrap();
        s.merge_signals("ft-baselines", &json!({"Dee": {"n": 1}, "Eve": {"n": 1}}), &json!([])).unwrap();
        let state = s.signals_state("").unwrap();
        assert_eq!(state["signals"]["ft-baselines"], json!({"Eve": {"n": 1}}), "a forgotten name is refused");
        assert_eq!(state["forgotten"], json!(["Dee"]));
        assert!(s.merge_signals("ft-other", &json!({}), &json!([])).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
