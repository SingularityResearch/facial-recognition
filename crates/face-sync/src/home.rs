//! The private folder's part of the sync: photo descriptions, Collect
//! faces pictures, the alert inbox, the presence log, the fitted documents, people forgotten on another
//! device, this device's settings and each person's signals. (The keys, which are kept encrypted, are not
//! here yet.)
//!
//! Each table is read from and written to the files the programs use, through the stores that keep them
//! (`face-home`, `face-files`) and under their locks, so the programs don't change: they keep reading their
//! files. A device's own presence visits and pictures keep their device; those from other devices are
//! written into this device's files too, so its pages show everyone's.

use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use face_files::{filejson, forgotten, paths::atomic_write};
use face_home::{
    alerts::Inbox,
    descriptions, presence, saved,
    store::{ALL_KINDS, Store},
};
use face_library::labels::NOT_NAMES;
use indexmap::IndexMap;
use ring::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use serde_json::{Value, json};
use tokio_postgres::{Client, Transaction};

use crate::{
    digest::sorted,
    keys::{self, Keys},
    local::{utc_text, utc_time},
    sent,
    server::{Param, run_all},
    state::State,
};

type Result<T> = std::result::Result<T, String>;

/// Count one Web search in the shared database before it is sent: the number used
/// this month with it, or None past the cap (every device's searches count).
pub async fn count_use(client: &Client, service: &str, month: &str, cap: i64) -> Result<Option<i64>> {
    let row = client
        .query_opt(
            "INSERT INTO usage AS u (service, month, used) VALUES ($1, $2, 1)
             ON CONFLICT (service, month) DO UPDATE SET used = u.used + 1 WHERE u.used < $3::int
             RETURNING used",
            &[&service, &month, &(cap.clamp(0, i32::MAX as i64) as i32)],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| r.get::<_, i32>(0) as i64))
}

fn said(e: tokio_postgres::Error) -> String {
    crate::server::words(&e)
}

/// The tables, in the order they are sent and taken.
pub const TABLES: [&str; 9] = [
    "descriptions",
    "webcam_snapshots",
    "alerts",
    "presence",
    "documents",
    "secrets",
    "forgotten",
    "device_settings",
    "person_signals",
];
/// `data/<name>.json`, fitted from everyone's reports: shared by every device
const SHARED_DOCS: [&str; 2] = ["expr_calibration", "va_fit"];
/// `data/<name>.json`, a device's own measurements: kept in the database, never taken from it
const DEVICE_DOCS: [&str; 3] = ["model_eval", "hw_bench", "webcam_eval"];
/// ms, the year 2100: a visit's times from the database are below this
const PRESENCE_MAX: i64 = 4_102_444_800_000;
/// Settings that say where this device sends photos or fetches files, as (app, key). They are set on the
/// device and never come from the database: not with Copy settings either.
pub const STAY_HERE: [(&str, &str); 4] =
    [("describe", "url"), ("describe", "spread"), ("lmstudio", "share"), ("lmstudio", "distributed")];
/// the digest of a settings row this device never wrote: deleted at the next sending
const NOT_OURS: &str = "not this device's";
/// seconds: seen on another camera this recently counts as moving between cameras
const MOVE_WINDOW: f64 = 600.0;
/// The id of the alert about a device that joined: the same on every device that notices it.
fn ids_changed(
    before: usize,
    now: usize,
    certs_before: &serde_json::Map<String, Value>,
    certs: &serde_json::Map<String, Value>,
) -> bool {
    before != now || certs_before != certs
}

fn joined_alert(device: &str) -> String {
    let made = digest(&ring::digest::SHA256, format!("a new device joined: {device}").as_bytes());
    made.as_ref().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// seconds between two looks at which devices are in the shared database
const DEVICES_EVERY: u64 = 30;
/// seconds this device may have been away from the shared database and still raise an alert for a device it
/// finds there on its return: after a longer absence it notes who is there and raises nothing (it must not
/// bring back an alert the owner deleted long ago). Counted from this device's own last look, never from a
/// date in the row: whoever writes a row sets its dates.
const NEW_DEVICE: f64 = 7.0 * 86_400.0;
/// the devices this one has seen in the shared database (their ids): who is new is told by it
const KNOWN_DEVICES: &str = "known-devices.json";
/// seconds: older sightings are deleted, every device's (only the move alert uses them)
const KEEP_SIGHTINGS: f64 = 2.0 * MOVE_WINDOW;
/// A row of one of these tables.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// what the table keeps as JSON, or a row made of its columns
    Json(Value),
    /// a Collect faces picture with the tracker's verdict
    Snapshot { jpeg: Vec<u8>, verdict: Option<Value> },
    /// a key as the database holds it: encrypted under the passphrase for keys
    Secret { nonce: Vec<u8>, ciphertext: Vec<u8> },
}

/// A row's digest as every device has kept it so far (the SHA-1 of the row as
/// JSON with its names in order).
pub fn h(v: &Value) -> String {
    let sum = digest(&SHA1_FOR_LEGACY_USE_ONLY, filejson::dumps(&sorted(v)).as_bytes());
    sum.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// A description or a verdict with its time as UTC text (older files kept local time).
fn utc(v: &Value) -> Value {
    match v.get("when").and_then(Value::as_str) {
        Some(when) if v.is_object() => {
            let mut v = v.clone();
            v["when"] = json!(utc_text(when));
            v
        }
        _ => v.clone(),
    }
}

/// The whole number of a time in a line of a log (a fraction is cut off).
fn whole(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A line of the presence log as its row: (id, {name, start, end, device}). A visit without a device is
/// this device's own.
fn visit(line: &str, me: &str) -> Option<(String, Value)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let name = v.get("name")?.as_str()?;
    let (start, end) = (whole(v.get("start")?)?, whole(v.get("end")?)?);
    let device = v.get("device").and_then(Value::as_str).filter(|d| !d.is_empty()).unwrap_or(me);
    Some((format!("{device}:{start}:{name}"), json!({"name": name, "start": start, "end": end, "device": device})))
}

/// The files of a folder whose names end so, in the order of their names.
fn files(folder: &Path, begins: &str, ends: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = (std::fs::read_dir(folder).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(begins) && n.ends_with(ends)))
        .collect();
    found.sort();
    found
}

/// A file's size and when it was changed, in nanoseconds since 1970.
fn stamp(file: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(file).ok()?;
    let at = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((m.len(), at.as_nanos() as u64))
}

fn read_json(file: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

/// A name as a pattern that matches it and nothing else.
fn pattern_of(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 8);
    for c in name.chars() {
        if "()[]{}?*+-|^$\\.&~# \t\n\r\u{b}\u{c}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A folder's path in its one spelling: the system's separators, none at the end.
fn folder_text(text: &str) -> String {
    let sep = std::path::MAIN_SEPARATOR;
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let c = if cfg!(windows) && c == '/' { sep } else { c };
        if !(c == sep && out.ends_with(sep) && out.len() > 1) {
            out.push(c);
        }
    }
    while out.len() > 1 && out.ends_with(sep) && !out.ends_with(&format!(":{sep}")) {
        out.pop();
    }
    out
}

/// Delete what the server holds about a forgotten person that no device's files hold (the change log can't
/// carry it).
async fn forget_on_server(tx: &Transaction<'_>, name: &str) -> Result<()> {
    let word = format!(r"(^|\W){}(\W|$)", pattern_of(name));
    tx.execute("DELETE FROM app_log WHERE line ~* $1", &[&word]).await.map_err(said)?;
    // (the name inside the faces column is looked for as it is, not as a pattern: "Ann (work)" has brackets;
    // and as the database writes it too: it keeps "é" as "é", and a search for the escaped spelling
    // alone would leave such a person's lines)
    let (ascii, as_kept) =
        (format!("\"n\": {}", filejson::dumps(&json!(name))), format!("\"n\": {}", filejson::dumps_utf8(&json!(name))));
    tx.execute(
        "DELETE FROM label_history WHERE what ~* $1 OR position($2 in faces::text) > 0 OR position($3 in faces::text) > 0",
        &[&word, &ascii, &as_kept],
    )
    .await
    .map_err(said)?;
    tx.execute("UPDATE faces SET fact_name = NULL WHERE fact_name = $1", &[&name]).await.map_err(said)?;
    for table in ["expr_eval", "person_signals", "presence", "alerts", "sightings", "expression_log", "music_log"] {
        tx.execute(&format!("DELETE FROM {table} WHERE name = $1"), &[&name]).await.map_err(said)?;
    }
    Ok(())
}

/// The database password's fingerprint, for a database's own id.
pub type Fingerprint = Box<dyn Fn(&str) -> String + Send>;

/// What to run when another device forgot someone: every place a person's data lives on this device.
pub type Forget = Box<dyn FnMut(&str) -> std::result::Result<(), String> + Send>;

/// The private folder's part of the sync on one device.
pub struct Home {
    data: PathBuf,
    home: PathBuf,
    /// this device's id
    device: String,
    store: Store,
    inbox: Inbox,
    /// None: nobody is forgotten here because another device forgot them (and the name isn't noted either,
    /// so it is done once there is a way to)
    pub forget: Option<Forget>,
    /// The database password's fingerprint for a database's own id (the password stays with whoever
    /// connects); None: this device doesn't show one.
    pub password_fingerprint: Option<Fingerprint>,
    /// the people this device's camera saw at the last step (who has arrived since)
    seen_before: std::collections::BTreeSet<String>,
    /// this device's copy of the passphrase for keys
    keys: Keys,
    /// this device has the current passphrase (`every_step`): only then are keys sent and read
    key_current: AtomicBool,
    /// when this device last looked at which devices are in the shared database
    devices_looked: Option<Instant>,
}

impl Home {
    /// `data`: the library's folder; `home`: the private folder; `device`: this device's id.
    pub fn new(data: &Path, home: &Path, device: &str) -> Home {
        Home {
            data: data.to_path_buf(),
            home: home.to_path_buf(),
            device: device.to_string(),
            store: Store::new(home),
            inbox: Inbox::new(home),
            forget: None,
            password_fingerprint: None,
            seen_before: Default::default(),
            keys: Keys::new(home, device),
            key_current: AtomicBool::new(false),
            devices_looked: None,
        }
    }

    fn refuse<T>(tbl: &str) -> Result<T> {
        Err(format!("the sync has no place to put rows of {tbl:?}"))
    }

    fn webcam(&self) -> PathBuf {
        self.home.join("webcam")
    }

    // ---- this device's rows

    fn presence(&self) -> Result<IndexMap<String, Value>> {
        let mut out = IndexMap::new();
        for file in files(&self.home.join("presence"), "", ".jsonl") {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("can't read the presence log: {e}"))?;
            out.extend(text.lines().filter_map(|line| visit(line, &self.device)));
        }
        Ok(out)
    }

    fn settings(&self) -> Result<IndexMap<String, Value>> {
        let me = &self.device;
        let mut out = IndexMap::new();
        for (app, kept) in self.store.settings()? {
            for (key, value) in kept.as_object().into_iter().flatten() {
                out.insert(format!("{me}|{app}|{key}"), json!({"app": app, "key": key, "value": value}));
            }
        }
        for (key, value) in saved::describer(&self.home) {
            out.insert(format!("{me}|describe|{key}"), json!({"app": "describe", "key": key, "value": value}));
        }
        let lm = read_json(&self.home.join("lmstudio.json")).unwrap_or(Value::Null);
        let share = lm.get("share").and_then(Value::as_str).map(folder_text).unwrap_or_default();
        let anywhere = if lm.get("distributed") == Some(&Value::Bool(true)) { "1" } else { "0" };
        out.insert(format!("{me}|lmstudio|share"), json!({"app": "lmstudio", "key": "share", "value": share}));
        out.insert(
            format!("{me}|lmstudio|distributed"),
            json!({"app": "lmstudio", "key": "distributed", "value": anywhere}),
        );
        Ok(out)
    }

    /// {key: row} of a table as this device has it (not the pictures: `snapshots`).
    fn local(&self, tbl: &str) -> Result<IndexMap<String, Value>> {
        Ok(match tbl {
            "descriptions" => {
                let _one = descriptions::hold();
                descriptions::load(&self.home)?.into_iter().collect()
            }
            "alerts" => {
                (self.inbox.all()?.into_iter()).filter_map(|a| Some((a.get("id")?.as_str()?.to_string(), a))).collect()
            }
            "presence" => self.presence()?,
            "documents" => (SHARED_DOCS.iter().map(|n| (*n, true)).chain(DEVICE_DOCS.iter().map(|n| (*n, false))))
                .filter_map(|(name, shared)| {
                    let body = read_json(&self.data.join(format!("{name}.json")))?;
                    Some(if shared {
                        (name.to_string(), json!({"name": name, "body": body, "device": null}))
                    } else {
                        (format!("{}:{name}", self.device), json!({"name": name, "body": body, "device": self.device}))
                    })
                })
                .collect(),
            "forgotten" => {
                let names = forgotten::names(&self.home, true).map_err(|e| e.to_string())?;
                names.into_iter().map(|n| (n, json!({}))).collect()
            }
            "device_settings" => self.settings()?,
            "person_signals" => (self.store.signals_every()?.into_iter())
                .flat_map(|(kind, people)| {
                    people.into_iter().map(move |(name, data)| {
                        (format!("{name}|{kind}"), json!({"name": name, "kind": kind, "data": data}))
                    })
                })
                .collect(),
            _ => return Home::refuse(tbl),
        })
    }

    /// The Collect faces pictures' digests: from each picture's and verdict's size and time, not their bytes.
    fn snapshots(&self) -> IndexMap<String, String> {
        (files(&self.webcam(), "webcam-", ".jpg").into_iter())
            .filter_map(|jpg| {
                let (size, at) = stamp(&jpg)?;
                let verdict = stamp(&jpg.with_extension("json"));
                let name = jpg.file_stem()?.to_str()?.to_string();
                Some((name, h(&json!([size, at, verdict.map(|v| v.0), verdict.map(|v| v.1)]))))
            })
            .collect()
    }

    fn digests_of(&self, tbl: &str) -> Result<IndexMap<String, String>> {
        Ok(match tbl {
            "webcam_snapshots" => self.snapshots(),
            // (a forgotten name's digest is of the name itself)
            "forgotten" => self.local(tbl)?.into_keys().map(|n| (n.clone(), h(&json!(n)))).collect(),
            _ => self.local(tbl)?.into_iter().map(|(k, v)| (k, h(&v))).collect(),
        })
    }

    /// {name: the key as text} of the keys saved on this device (its files only).
    fn secrets(&self) -> IndexMap<String, String> {
        let mut out = IndexMap::new();
        let google = saved::google_key(&self.home);
        if !google.is_empty() {
            out.insert("google_vision".to_string(), google);
        }
        if let Some((id, secret, region)) = saved::aws(&self.home) {
            let kept = json!({"access_key_id": id, "region": region, "secret_access_key": secret});
            out.insert("aws_rekognition".to_string(), filejson::dumps(&kept));
        }
        let (url, key) = saved::describe_key(&self.home);
        if !key.is_empty() {
            out.insert(format!("describe_key:{url}"), key);
        }
        out
    }

    /// The passphrase key the keys go under: only while this device has the current one.
    fn secrets_key(&self) -> Option<Vec<u8>> {
        self.keys.local_key().filter(|_| self.key_current.load(Ordering::Relaxed))
    }

    /// The digest of every row this device has, by table. The keys only while this device can read them
    /// (a device that isn't on the current passphrase neither sends nor reads keys).
    pub fn digests(&self) -> Result<HashMap<&'static str, IndexMap<String, String>>> {
        let mut out = HashMap::new();
        for tbl in TABLES {
            if tbl == "secrets" {
                if let Some(key) = self.secrets_key() {
                    out.insert(
                        tbl,
                        self.secrets()
                            .into_iter()
                            .map(|(n, plain)| (n.clone(), keys::digest(&key, &n, plain.as_bytes())))
                            .collect(),
                    );
                }
            } else {
                out.insert(tbl, self.digests_of(tbl)?);
            }
        }
        Ok(out)
    }

    /// The rows to send for these keys of a table, as they are now.
    pub fn rows(&self, tbl: &str, keys: &[String]) -> Result<Vec<(String, Row)>> {
        if tbl == "secrets" {
            let mut local = self.secrets();
            return Ok(keys
                .iter()
                .filter_map(|k| Some((k.clone(), Row::Json(json!(local.swap_remove(k)?)))))
                .collect());
        }
        if tbl == "webcam_snapshots" {
            return Ok((keys.iter())
                .filter_map(|k| {
                    let jpg = self.webcam().join(format!("{k}.jpg"));
                    Some((
                        k.clone(),
                        Row::Snapshot {
                            jpeg: std::fs::read(&jpg).ok()?,
                            verdict: read_json(&jpg.with_extension("json")),
                        },
                    ))
                })
                .collect());
        }
        let mut local = self.local(tbl)?;
        Ok(keys.iter().filter_map(|k| Some((k.clone(), Row::Json(local.swap_remove(k)?)))).collect())
    }

    // ---- to the server

    /// The keys, encrypted here under the passphrase's key. False, and nothing sent, when the passphrase was
    /// changed elsewhere meanwhile: the new one is taken first.
    async fn push_secrets(&self, client: &mut Client, rows: &[(String, Row)], deletes: &[String]) -> Result<bool> {
        let fault = |e: keys::Fault| e.text().to_string();
        let tx = client.transaction().await.map_err(said)?;
        let cur = keys::current(&tx).await.map_err(fault)?;
        let Some(key) = self.keys.local_key().filter(|_| self.keys.on_current(cur.as_ref())) else {
            self.key_current.store(false, Ordering::Relaxed);
            return Ok(false);
        };
        let mut all: Vec<Vec<Param>> = Vec::new();
        for (name, row) in rows {
            if let Row::Json(Value::String(plain)) = row {
                let (nonce, ciphertext) = keys::encrypt(&key, name, plain.as_bytes()).map_err(fault)?;
                all.push(vec![Box::new(name.clone()), Box::new(nonce), Box::new(ciphertext)]);
            }
        }
        if !all.is_empty() {
            let put = "INSERT INTO secrets (name, nonce, ciphertext) VALUES ($1, $2, $3)
                       ON CONFLICT (name) DO UPDATE SET nonce = excluded.nonce, ciphertext = excluded.ciphertext";
            let put = tx.prepare(put).await.map_err(said)?;
            run_all(&tx, &put, &all).await?;
        }
        // a describe key leaves this device when its server's address changes: that's not a removal
        let gone: Vec<&String> = deletes.iter().filter(|k| !k.starts_with("describe_key:")).collect();
        if !gone.is_empty() {
            tx.execute("DELETE FROM secrets WHERE name = ANY($1)", &[&gone]).await.map_err(said)?;
        }
        tx.commit().await.map_err(said)?;
        Ok(true)
    }

    /// Send rows and deletions of one table, all of them or none. False: the table held them back (the
    /// keys, while this device hasn't got the current passphrase), and they stay as not sent.
    pub async fn push(
        &self,
        client: &mut Client,
        tbl: &str,
        rows: &[(String, Row)],
        deletes: &[String],
    ) -> Result<bool> {
        if tbl == "secrets" {
            return self.push_secrets(client, rows, deletes).await;
        }
        let me = &self.device;
        let tx = client.transaction().await.map_err(said)?;
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let plain = rows.iter().filter_map(|(k, r)| if let Row::Json(v) = r { Some((k, v)) } else { None });
        // the keys still to delete from the table by its key
        let mut by_key: Option<&str> = None;
        let (statement, all): (&str, Vec<Vec<Param>>) = match tbl {
            "descriptions" => {
                by_key = Some("key");
                let all =
                    plain.map(|(k, v)| -> Vec<Param> { vec![Box::new(k.clone()), Box::new(filejson::dumps(&utc(v)))] });
                (
                    "INSERT INTO descriptions (key, data) VALUES ($1, $2::text::jsonb)
                     ON CONFLICT (key) DO UPDATE SET data = excluded.data",
                    all.collect(),
                )
            }
            "webcam_snapshots" => {
                by_key = Some("name");
                let all = rows.iter().filter_map(|(k, r)| {
                    let Row::Snapshot { jpeg, verdict } = r else { return None };
                    let taken = verdict.as_ref().and_then(|v| v.get("when")).and_then(Value::as_str).and_then(utc_time);
                    let args: Vec<Param> = vec![
                        Box::new(k.clone()),
                        Box::new(me.clone()),
                        Box::new(taken),
                        Box::new(jpeg.clone()),
                        Box::new(verdict.as_ref().map(|v| filejson::dumps(&utc(v)))),
                    ];
                    Some(args)
                });
                (
                    "INSERT INTO webcam_snapshots (name, device, taken, jpeg, verdict)
                     VALUES ($1, $2::text::uuid, $3::text::timestamptz, $4::bytea, $5::text::jsonb)
                     ON CONFLICT (name) DO UPDATE SET jpeg = excluded.jpeg, verdict = excluded.verdict,
                     taken = excluded.taken, device = coalesce(webcam_snapshots.device, excluded.device)",
                    all.collect(),
                )
            }
            "alerts" => {
                by_key = Some("id");
                let all = plain.map(|(k, v)| -> Vec<Param> {
                    vec![Box::new(k.clone()), Box::new(me.clone()), Box::new(filejson::dumps(v))]
                });
                (
                    "INSERT INTO alerts (id, device, data) VALUES ($1, $2::text::uuid, $3::text::jsonb)
                     ON CONFLICT (id) DO UPDATE SET data = excluded.data",
                    all.collect(),
                )
            }
            "presence" => {
                by_key = Some("id");
                let all = plain.map(|(k, v)| -> Vec<Param> {
                    vec![
                        Box::new(k.clone()),
                        Box::new(text(v, "device")),
                        Box::new(text(v, "name")),
                        Box::new(v.get("start").and_then(Value::as_i64)),
                        Box::new(v.get("end").and_then(Value::as_i64)),
                    ]
                });
                (
                    "INSERT INTO presence (id, device, name, start_ms, end_ms) VALUES ($1, $2::text::uuid, $3, $4::bigint, $5::bigint)
                     ON CONFLICT (id) DO UPDATE SET end_ms = excluded.end_ms",
                    all.collect(),
                )
            }
            // (a file that is gone here stays in the database)
            "documents" => {
                let all = plain.map(|(k, v)| -> Vec<Param> {
                    vec![
                        Box::new(k.clone()),
                        Box::new(text(v, "device")),
                        Box::new(text(v, "name")),
                        Box::new(filejson::dumps(v.get("body").unwrap_or(&Value::Null))),
                    ]
                });
                (
                    "INSERT INTO documents (id, device, name, body) VALUES ($1, $2::text::uuid, $3, $4::text::jsonb)
                     ON CONFLICT (id) DO UPDATE SET body = excluded.body",
                    all.collect(),
                )
            }
            "forgotten" => (
                "INSERT INTO forgotten (name) VALUES ($1) ON CONFLICT (name) DO NOTHING",
                rows.iter().map(|(k, _)| -> Vec<Param> { vec![Box::new(k.clone())] }).collect(),
            ),
            "device_settings" => {
                let all = plain.map(|(_, v)| -> Vec<Param> {
                    vec![
                        Box::new(me.clone()),
                        Box::new(text(v, "app")),
                        Box::new(text(v, "key")),
                        Box::new(filejson::dumps(v.get("value").unwrap_or(&Value::Null))),
                    ]
                });
                (
                    "INSERT INTO device_settings (device, app, key, value) VALUES ($1::text::uuid, $2, $3, $4::text::jsonb)
                     ON CONFLICT (device, app, key) DO UPDATE SET value = excluded.value",
                    all.collect(),
                )
            }
            "person_signals" => {
                let all = plain.map(|(_, v)| -> Vec<Param> {
                    vec![
                        Box::new(text(v, "name")),
                        Box::new(text(v, "kind")),
                        Box::new(filejson::dumps(v.get("data").unwrap_or(&Value::Null))),
                    ]
                });
                (
                    "INSERT INTO person_signals (name, kind, data) VALUES ($1, $2, $3::text::jsonb)
                     ON CONFLICT (name, kind) DO UPDATE SET data = excluded.data",
                    all.collect(),
                )
            }
            _ => return Home::refuse(tbl),
        };
        if !all.is_empty() {
            let send = tx.prepare(statement).await.map_err(said)?;
            run_all(&tx, &send, &all).await?;
        }
        match tbl {
            "forgotten" => {
                // what the server holds about them goes at once
                for (name, _) in rows {
                    forget_on_server(&tx, name).await?;
                }
                // named again here: no longer an instruction for the others
                if !deletes.is_empty() {
                    tx.execute("DELETE FROM forgotten WHERE name = ANY($1)", &[&deletes]).await.map_err(said)?;
                }
            }
            "device_settings" => {
                for k in deletes {
                    let mut parts = k.splitn(3, '|').skip(1);
                    if let (Some(app), Some(key)) = (parts.next(), parts.next()) {
                        tx.execute(
                            "DELETE FROM device_settings WHERE device = $1::text::uuid AND app = $2 AND key = $3",
                            &[me, &app, &key],
                        )
                        .await
                        .map_err(said)?;
                    }
                }
            }
            "person_signals" => {
                for k in deletes {
                    if let Some((name, kind)) = k.rsplit_once('|') {
                        tx.execute("DELETE FROM person_signals WHERE name = $1 AND kind = $2", &[&name, &kind])
                            .await
                            .map_err(said)?;
                    }
                }
            }
            _ => {}
        }
        if let (Some(key), false) = (by_key, deletes.is_empty()) {
            tx.execute(&format!("DELETE FROM {tbl} WHERE {key} = ANY($1)"), &[&deletes]).await.map_err(said)?;
        }
        tx.commit().await.map_err(said)?;
        Ok(true)
    }

    /// Every key the database has of a table that is this device's to take.
    pub async fn all_keys(&self, client: &Client, tbl: &str) -> Result<Vec<String>> {
        let me = &self.device;
        let rows = match tbl {
            "descriptions" => client.query("SELECT key FROM descriptions", &[]).await,
            "webcam_snapshots" => client.query("SELECT name FROM webcam_snapshots", &[]).await,
            "alerts" => client.query("SELECT id FROM alerts", &[]).await,
            "presence" => client.query("SELECT id FROM presence", &[]).await,
            "forgotten" => client.query("SELECT name FROM forgotten", &[]).await,
            "secrets" => client.query("SELECT name FROM secrets", &[]).await,
            "documents" => {
                client.query("SELECT id FROM documents WHERE device IS NULL OR device = $1::text::uuid", &[me]).await
            }
            "device_settings" => {
                client
                    .query(
                        "SELECT $1 || '|' || app || '|' || key FROM device_settings WHERE device = $1::text::uuid",
                        &[me],
                    )
                    .await
            }
            "person_signals" => client.query("SELECT name || '|' || kind FROM person_signals", &[]).await,
            _ => return Home::refuse(tbl),
        };
        Ok(rows.map_err(said)?.iter().map(|r| r.get(0)).collect())
    }

    /// The database's rows for these keys (None: gone there). A key that is not this device's to take
    /// (another device's settings or measurements) is left out.
    pub async fn fetch(&self, client: &Client, tbl: &str, keys: &[String]) -> Result<Vec<(String, Option<Row>)>> {
        let me = &self.device;
        let mut out: IndexMap<String, Option<Row>> = keys.iter().map(|k| (k.clone(), None)).collect();
        let json_of =
            |text: Option<String>| -> Value { text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null) };
        if tbl == "device_settings" {
            // other devices' settings stay theirs (and a key that is none)
            out.retain(|k, _| {
                k.split_once('|').is_some_and(|(device, rest)| {
                    device == me && rest.split_once('|').is_some_and(|(_, key)| !key.is_empty())
                })
            });
            let ask = "SELECT app, key, value::text FROM device_settings WHERE device = $1::text::uuid";
            for r in client.query(ask, &[me]).await.map_err(said)? {
                let (app, key): (String, String) = (r.get(0), r.get(1));
                if let Some(row) = out.get_mut(&format!("{me}|{app}|{key}")) {
                    *row = Some(Row::Json(json!({"app": app, "key": key, "value": json_of(r.get(2))})));
                }
            }
            return Ok(out.into_iter().collect());
        }
        if tbl == "secrets" {
            for r in client
                .query("SELECT name, nonce, ciphertext FROM secrets WHERE name = ANY($1)", &[&keys])
                .await
                .map_err(said)?
            {
                out.insert(r.get(0), Some(Row::Secret { nonce: r.get(1), ciphertext: r.get(2) }));
            }
            return Ok(out.into_iter().collect());
        }
        for part in keys.chunks(500) {
            let ask = match tbl {
                "descriptions" => "SELECT key, data::text FROM descriptions WHERE key = ANY($1)",
                "webcam_snapshots" => "SELECT name, verdict::text, jpeg FROM webcam_snapshots WHERE name = ANY($1)",
                "alerts" => "SELECT id, data::text FROM alerts WHERE id = ANY($1)",
                "presence" => "SELECT id, device::text, name, start_ms, end_ms FROM presence WHERE id = ANY($1)",
                "documents" => "SELECT id, body::text, device::text, name FROM documents WHERE id = ANY($1)",
                "forgotten" => "SELECT name FROM forgotten WHERE name = ANY($1)",
                "person_signals" => {
                    "SELECT name || '|' || kind, data::text, name, kind FROM person_signals WHERE name || '|' || kind = ANY($1)"
                }
                _ => return Home::refuse(tbl),
            };
            for r in client.query(ask, &[&part]).await.map_err(said)? {
                let key: String = r.get(0);
                let row = match tbl {
                    "descriptions" | "alerts" => Row::Json(json_of(r.get(1))),
                    "webcam_snapshots" => {
                        let verdict: Option<String> = r.get(1);
                        Row::Snapshot { jpeg: r.get(2), verdict: verdict.map(|t| json_of(Some(t))) }
                    }
                    "presence" => {
                        let (device, name): (Option<String>, String) = (r.get(1), r.get(2));
                        let (start, end): (i64, i64) = (r.get(3), r.get(4));
                        Row::Json(json!({"name": name, "start": start, "end": end, "device": device}))
                    }
                    "documents" => {
                        let (device, name): (Option<String>, String) = (r.get(2), r.get(3));
                        if device.as_ref().is_some_and(|d| d != me) {
                            out.shift_remove(&key); // other devices' measurements stay theirs
                            continue;
                        }
                        Row::Json(json!({"name": name, "body": json_of(r.get(1)), "device": device}))
                    }
                    "forgotten" => Row::Json(json!({})),
                    _ => {
                        let (name, kind): (String, String) = (r.get(2), r.get(3));
                        Row::Json(json!({"name": name, "kind": kind, "data": json_of(r.get(1))}))
                    }
                };
                out.insert(key, Some(row));
            }
        }
        Ok(out.into_iter().collect())
    }

    // ---- from the server

    /// Why a row from the database can't be used here as it is, if it can't. The json columns hold whatever
    /// was written: an alert without its id was once saved with the others and then stopped every later
    /// step. Only what would break this device is refused; values it doesn't know are left alone, so a
    /// newer version's rows still pass.
    pub fn check(&self, tbl: &str, key: &str, row: &Row) -> Result<()> {
        let need = |ok: bool, what: &str| if ok { Ok(()) } else { Err(what.to_string()) };
        need(!key.is_empty(), "no key")?;
        let v = match (tbl, row) {
            ("webcam_snapshots", Row::Snapshot { verdict, .. }) => {
                return need(verdict.as_ref().is_none_or(Value::is_object), "verdict: not an object");
            }
            ("secrets", Row::Secret { .. }) => return Ok(()),
            ("webcam_snapshots" | "secrets", _) | (_, Row::Snapshot { .. } | Row::Secret { .. }) => {
                return Err("not a row of this table".to_string());
            }
            (_, Row::Json(v)) => v,
        };
        match tbl {
            "descriptions" => need(v.is_object(), "not an object"),
            "alerts" => {
                need(v.is_object() && v.get("id").and_then(Value::as_str) == Some(key), "not an alert with this id")?;
                need(v.get("t").and_then(Value::as_f64).is_some_and(|t| t.is_finite() && t >= 0.0), "no time")?;
                let empty = |x: &Value| match x {
                    Value::Null | Value::Bool(false) => true,
                    Value::Number(n) => n.as_f64() == Some(0.0),
                    Value::Array(a) => a.is_empty(),
                    Value::Object(o) => o.is_empty(),
                    _ => false,
                };
                let texts = ["kind", "name", "text", "state"]
                    .iter()
                    .all(|k| v.get(*k).is_none_or(|x| x.is_string() || empty(x)));
                need(texts, "not text")?;
                let details = v.get("details").filter(|d| !d.is_null());
                let sets = details.is_none_or(|d| {
                    d.as_object().is_some_and(|sets| sets.values().all(|s| s.is_null() || s.is_object()))
                });
                need(sets, "details: not objects")
            }
            "presence" => {
                let name = v.get("name").and_then(Value::as_str).filter(|n| !n.is_empty());
                need(name.is_some(), "no name")?;
                let time = |k: &str| v.get(k).and_then(Value::as_i64).filter(|t| 0 < *t && *t < PRESENCE_MAX);
                let (Some(start), Some(_)) = (time("start"), time("end")) else {
                    return Err("times out of range".to_string());
                };
                let device = v.get("device").and_then(Value::as_str).unwrap_or("None");
                need(key == format!("{device}:{start}:{}", name.unwrap_or("")), "not this visit's id")
            }
            "documents" => {
                let name = v.get("name").and_then(Value::as_str);
                need(name.is_some(), "no name")?;
                need(
                    !SHARED_DOCS.contains(&name.unwrap_or("")) || v.get("body").is_some_and(Value::is_object),
                    "body: not an object",
                )
            }
            "forgotten" => need(key.chars().count() <= 200 && !key.starts_with('#'), "not a person's name"),
            "device_settings" => need(key.matches('|').count() >= 2, "not device|app|key"),
            "person_signals" => {
                need(key.contains('|') && v.get("data").is_some_and(Value::is_object), "not name|kind with an object")?;
                need(!NOT_NAMES.contains(&key.rsplit_once('|').map_or("", |(name, _)| name)), "a name no page can keep")
            }
            _ => Home::refuse(tbl),
        }
    }

    /// Put rows (None: deleted in the database) where this device keeps them. For each key the digest of
    /// what is here now, or None; a key that is left out is remembered as before.
    pub fn apply(
        &mut self,
        tbl: &str,
        rows: Vec<(String, Option<Row>)>,
        log: &mut dyn FnMut(String),
    ) -> Result<HashMap<String, Option<String>>> {
        let keys: Vec<String> = rows.iter().map(|(k, _)| k.clone()).collect();
        let plain = |row: Option<Row>| match row {
            Some(Row::Json(v)) => Some(v),
            _ => None,
        };
        let now = |home: &Home, tbl: &str| -> Result<HashMap<String, Option<String>>> {
            let mut here = home.digests_of(tbl)?;
            Ok(keys.iter().map(|k| (k.clone(), here.swap_remove(k))).collect())
        };
        match tbl {
            "secrets" => {
                // (an old passphrase reads nothing, and nothing is remembered: read again with the current one)
                let Some(key) = self.secrets_key() else { return Ok(HashMap::new()) };
                for (name, row) in rows {
                    let plain = match &row {
                        Some(Row::Secret { nonce, ciphertext }) => keys::decrypt(&key, &name, nonce, ciphertext)
                            .map_err(|e| e.text().to_string())
                            .and_then(|b| String::from_utf8(b).map_err(|_| "not text".to_string())),
                        _ => Ok(String::new()),
                    };
                    let kept = plain.and_then(|plain| match name.as_str() {
                        "google_vision" => saved::save_google_key(&self.home, &plain),
                        "aws_rekognition" => {
                            let a: Value = if plain.is_empty() {
                                json!({})
                            } else {
                                serde_json::from_str(&plain).map_err(|e| e.to_string())?
                            };
                            let text = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("");
                            saved::save_aws(
                                &self.home,
                                text("access_key_id"),
                                text("secret_access_key"),
                                a.get("region").and_then(Value::as_str),
                            )
                        }
                        // (a describe server's key is kept only for the server this device asks)
                        other => match other.strip_prefix("describe_key:") {
                            Some(url) if saved::describe_key(&self.home).0 == url => {
                                saved::save_describe_key(&self.home, &plain)
                            }
                            _ => Ok(()),
                        },
                    });
                    // this key is left as it is; the others go on
                    if let Err(e) = kept {
                        log(format!(
                            "Shared database: key {}: {e}",
                            name.chars().take(80).map(|c| if c.is_control() { '?' } else { c }).collect::<String>()
                        ));
                    }
                }
                let mut here = self.secrets();
                Ok(keys
                    .into_iter()
                    .map(|k| (k.clone(), here.swap_remove(&k).map(|plain| keys::digest(&key, &k, plain.as_bytes()))))
                    .collect())
            }
            "descriptions" => {
                let _one = descriptions::hold();
                let mut all = descriptions::load(&self.home)?;
                for (k, row) in rows {
                    match plain(row) {
                        Some(v) => all.insert(k, v),
                        None => all.shift_remove(&k),
                    };
                }
                descriptions::write(&self.home, &all)?;
                Ok(keys.iter().map(|k| (k.clone(), all.get(k).map(h))).collect())
            }
            "webcam_snapshots" => {
                let folder = self.webcam();
                std::fs::create_dir_all(&folder).map_err(|e| format!("can't make the Collect faces folder: {e}"))?;
                let said = |e: std::io::Error| format!("can't write a Collect faces picture: {e}");
                let mut taken = Vec::new();
                for (k, row) in rows {
                    // (a name is only ever a file's name here)
                    let named = k.strip_prefix("webcam-").is_some_and(|r| {
                        !r.is_empty() && r.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    });
                    if !named {
                        continue;
                    }
                    let jpg = folder.join(format!("{k}.jpg"));
                    match row {
                        Some(Row::Snapshot { jpeg, verdict }) => {
                            // the verdict first: the labeling tool reads it with the picture
                            if let Some(v) = verdict {
                                atomic_write(&jpg.with_extension("json"), filejson::dumps(&v).as_bytes())
                                    .map_err(said)?;
                            }
                            atomic_write(&jpg, &jpeg).map_err(said)?;
                        }
                        _ => {
                            let _ = std::fs::remove_file(&jpg);
                            let _ = std::fs::remove_file(jpg.with_extension("json"));
                        }
                    }
                    taken.push(k);
                }
                let mut here = self.snapshots();
                Ok(taken.into_iter().map(|k| (k.clone(), here.swap_remove(&k))).collect())
            }
            "alerts" => {
                let kept = self.inbox.change(|have| {
                    let mut have: IndexMap<String, Value> =
                        have.into_iter().filter_map(|a| Some((a.get("id")?.as_str()?.to_string(), a))).collect();
                    for (k, row) in rows {
                        match plain(row) {
                            Some(v) => have.insert(k, v),
                            None => have.shift_remove(&k),
                        };
                    }
                    have.into_values().collect()
                })?;
                let have: HashMap<&str, &Value> =
                    kept.iter().filter_map(|a| Some((a.get("id")?.as_str()?, a))).collect();
                Ok(keys.iter().map(|k| (k.clone(), have.get(k.as_str()).map(|a| h(a)))).collect())
            }
            "presence" => {
                let folder = self.home.join("presence");
                std::fs::create_dir_all(&folder).map_err(|e| format!("can't make the presence folder: {e}"))?;
                let said = |e: std::io::Error| format!("can't write the presence log: {e}");
                let have = self.presence()?;
                let mut gone = Vec::new();
                for (k, row) in rows {
                    let Some(v) = plain(row) else {
                        if have.contains_key(&k) {
                            gone.push(k);
                        }
                        continue;
                    };
                    let (Some(start), false) = (v.get("start").and_then(Value::as_i64), have.contains_key(&k)) else {
                        continue;
                    };
                    let mut line = json!({"name": v["name"], "start": start, "end": v["end"]});
                    if v.get("device").and_then(Value::as_str) != Some(self.device.as_str()) {
                        line["device"] = v["device"].clone();
                    }
                    // the tracker's day files, and their lines as it ends them
                    let file = folder.join(format!("{}.jsonl", presence::day_of(start)?));
                    let text = filejson::dumps(&line) + presence::END;
                    (std::fs::OpenOptions::new().create(true).append(true).open(file))
                        .and_then(|mut f| f.write_all(text.as_bytes()))
                        .map_err(said)?;
                }
                if !gone.is_empty() {
                    for file in files(&folder, "", ".jsonl") {
                        let text = std::fs::read_to_string(&file).map_err(said)?;
                        let keep = text
                            .lines()
                            .filter(|line| visit(line, &self.device).is_none_or(|(k, _)| !gone.contains(&k)));
                        atomic_write(&file, keep.map(|l| format!("{l}\n")).collect::<String>().as_bytes())
                            .map_err(said)?;
                    }
                }
                now(self, tbl)
            }
            "documents" => {
                for v in rows.into_iter().filter_map(|(_, row)| plain(row)) {
                    if let Some(name) = v.get("name").and_then(Value::as_str).filter(|n| SHARED_DOCS.contains(n)) {
                        let text = filejson::dumps_lines(v.get("body").unwrap_or(&Value::Null), true, 1);
                        atomic_write(&self.data.join(format!("{name}.json")), text.as_bytes())
                            .map_err(|e| format!("can't write {name}.json: {e}"))?;
                    }
                }
                now(self, tbl)
            }
            "forgotten" => {
                // forgotten on another device: forget here too, once; named again there (the row is gone):
                // no longer forgotten
                let io = |e: std::io::Error| e.to_string();
                let done = forgotten::names(&self.home, false).map_err(io)?;
                for (name, row) in rows {
                    if row.is_none() {
                        forgotten::unforget(&self.home, &name).map_err(io)?;
                    } else if let (false, Some(forget)) = (done.contains(&name), self.forget.as_mut()) {
                        // (never stop the sync over one person)
                        match forget(&name) {
                            Ok(()) => log(format!("Shared database: forgot {name}, as another device did.")),
                            Err(e) => log(format!("Shared database: couldn't forget {name} here: {e}")),
                        }
                        forgotten::note(&self.home, &name).map_err(io)?;
                    }
                }
                let here = forgotten::names(&self.home, false).map_err(io)?;
                Ok(keys.into_iter().filter(|n| here.contains(n)).map(|n| (n.clone(), Some(h(&json!(n))))).collect())
            }
            "device_settings" => {
                // A device's settings are its own: the database keeps a copy of them and never changes them
                // (whoever has the database's password could otherwise switch on a device's camera stream,
                // or point its photo describer at a server of theirs). A row that isn't as this device has
                // it is sent again as it is here (None: not synced); one this device never wrote is deleted
                // (NOT_OURS); both at the next sending.
                let mine = self.settings()?;
                let (mut out, mut differ) = (HashMap::new(), Vec::new());
                for (k, row) in rows {
                    let theirs = plain(row);
                    let (same, kept) = match mine.get(&k) {
                        Some(here) => {
                            let same = theirs.as_ref().is_some_and(|v| v.get("value") == here.get("value"));
                            (same, same.then(|| h(here)))
                        }
                        None => (theirs.is_none(), theirs.is_some().then(|| NOT_OURS.to_string())),
                    };
                    if !same {
                        let name = k.split_once('|').map_or("", |(_, rest)| rest).replace('|', ": ");
                        differ.push(
                            name.chars().take(60).map(|c| if c.is_control() { '?' } else { c }).collect::<String>(),
                        );
                    }
                    out.insert(k, kept);
                }
                if !differ.is_empty() {
                    log(format!(
                        "Shared database: {} of this device's settings are different in the database ({}{}). A device's settings are its own: they stay as they are here, and the database gets them again.",
                        differ.len(),
                        differ.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                        if differ.len() > 5 { ", ..." } else { "" }
                    ));
                }
                Ok(out)
            }
            "person_signals" => {
                for (k, row) in rows {
                    let data = plain(row).map(|v| v.get("data").cloned().unwrap_or(Value::Null));
                    if let Some((name, kind)) = k.rsplit_once('|').filter(|(_, kind)| ALL_KINDS.contains(kind)) {
                        match self.store.put_signal(kind, name, data.as_ref()) {
                            Err(e) if e == "bad signals" => log(format!("Shared database: person signals: {e}")),
                            other => other?,
                        }
                    }
                }
                now(self, tbl)
            }
            _ => Home::refuse(tbl),
        }
    }

    /// Set this device's settings from rows of its own in the database: only for `copy_settings`, the
    /// owner's Copy settings on this device. Never the settings that say where this device sends photos or
    /// fetches files (`STAY_HERE`).
    fn take_settings(&self, rows: Vec<(String, Option<Row>)>, log: &mut dyn FnMut(String)) {
        let mut pages: IndexMap<String, serde_json::Map<String, Value>> = IndexMap::new();
        for (k, row) in rows {
            let mut parts = k.splitn(3, '|').skip(1);
            let (Some(app), Some(key)) = (parts.next(), parts.next()) else { continue };
            let value = match row {
                Some(Row::Json(v)) => v.get("value").cloned().filter(|v| !v.is_null()),
                _ => None,
            };
            if STAY_HERE.contains(&(app, key)) {
                continue;
            }
            if app == "track" || app == "label" {
                // (a page keeps every setting as text)
                let as_text = value.map(|v| match v {
                    Value::String(s) => s,
                    Value::Bool(b) => if b { "True" } else { "False" }.to_string(),
                    other => other.to_string(),
                });
                pages
                    .entry(app.to_string())
                    .or_default()
                    .insert(key.to_string(), as_text.map_or(Value::Null, Value::String));
            } else if let (true, Some(value)) = (app == "describe" && (key == "use" || key == "model"), value)
                && let Err(e) = saved::set_describer(&self.home, key, &value)
            {
                log(format!("Shared database: describe setting {key}: {e}"));
            }
        }
        for (app, changes) in pages {
            if let Err(e) = self.store.update(&app, &Value::Object(changes)) {
                log(format!("Shared database: settings: {e}"));
            }
        }
    }

    /// This device takes another device's page settings and which describer and model it uses: replaced in
    /// the database, then set here. Not the settings that say where a device sends photos or fetches files
    /// (`STAY_HERE`): those are set on each device, never from a row. How many were copied.
    pub async fn copy_settings(
        &self,
        client: &mut Client,
        from: &str,
        log: &mut dyn FnMut(String),
    ) -> std::result::Result<usize, keys::Fault> {
        use keys::Fault;
        let db = |e: tokio_postgres::Error| Fault::Failed(said(e));
        // (an id in any of the ways one is written)
        let hex: String = from
            .trim()
            .trim_start_matches("urn:uuid:")
            .chars()
            .filter(|c| !matches!(c, '-' | '{' | '}'))
            .collect::<String>()
            .to_lowercase();
        if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Fault::Bad("badly formed hexadecimal UUID string".to_string()));
        }
        let source = format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]);
        let me = &self.device;
        if source == *me {
            return Err(Fault::Bad("That's this device.".to_string()));
        }
        let stay: Vec<String> = STAY_HERE.iter().map(|(app, key)| format!("{app}|{key}")).collect();
        let tx = client.transaction().await.map_err(db)?;
        let count =
            "SELECT count(*) FROM device_settings WHERE device = $1::text::uuid AND NOT (app || '|' || key = ANY($2))";
        let n: i64 = tx.query_one(count, &[&source, &stay]).await.map_err(db)?.get(0);
        if n == 0 {
            return Err(Fault::Bad("That device has no settings in the shared database.".to_string()));
        }
        tx.execute("DELETE FROM device_settings WHERE device = $1::text::uuid AND app IN ('track', 'label')", &[me])
            .await
            .map_err(db)?;
        let copy = "INSERT INTO device_settings (device, app, key, value)
                    SELECT $1::text::uuid, app, key, value FROM device_settings
                    WHERE device = $2::text::uuid AND NOT (app || '|' || key = ANY($3))
                    ON CONFLICT (device, app, key) DO UPDATE SET value = excluded.value";
        tx.execute(copy, &[me, &source, &stay]).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        let mut keys = self.all_keys(client, "device_settings").await.map_err(Fault::Failed)?;
        // what this device had and the other hasn't: removed here
        let gone: Vec<String> = (self.settings().map_err(Fault::Failed)?.into_keys())
            .filter(|k| k.split('|').nth(1).is_some_and(|app| app == "track" || app == "label") && !keys.contains(k))
            .collect();
        keys.extend(gone);
        let rows = self.fetch(client, "device_settings", &keys).await.map_err(Fault::Failed)?;
        self.take_settings(rows, log);
        Ok(n as usize)
    }

    /// Each step: the passphrase for keys. Take the current one if it was changed by a device that held the
    /// one this device has, or given in answer to this device's request (the database password alone gets
    /// nobody the passphrase), and show this device's fingerprints and proof. Taking a new one reads every
    /// key again under it. `database`: the database's own id.
    pub async fn every_step(
        &mut self,
        client: &Client,
        state: &State,
        database: &str,
        log: &mut dyn FnMut(String),
    ) -> Result<()> {
        let fault = |e: keys::Fault| e.text().to_string();
        let had = self.key_current.load(Ordering::Relaxed);
        // what the device that let this one in handed over with the login: taken when it is the current
        // passphrase, as if the owner had typed it here; anything else is dropped and changes nothing
        if let Some(handed) = face_home::passphrase::received(&self.home) {
            let taken = self.keys.enter(client, &handed).await.map_err(fault)?;
            face_home::passphrase::drop_received(&self.home);
            log(if taken {
                "Keys: this device has the passphrase for keys, handed over by the device that let it in.".to_string()
            } else {
                "Keys: the passphrase that came with joining isn't the current one. Type it in Settings, or get it \
                 from another device with a code."
                    .to_string()
            });
        }
        let (current, took) = self.keys.catch_up(client).await.map_err(fault)?;
        self.key_current.store(current, Ordering::Relaxed);
        let password = self.password_fingerprint.as_ref().map(|of| of(database));
        self.keys.register(client, password.as_deref()).await.map_err(fault)?;
        // (typed here since the last step, or the first step: read too)
        if took || (current && !had) {
            let names = self.all_keys(client, "secrets").await?;
            let rows = self.fetch(client, "secrets", &names).await?;
            state.set_digests("secrets", &self.apply("secrets", rows, log)?)?;
        }
        Ok(())
    }

    /// Each step, a look every `DEVICES_EVERY` seconds (`new_devices`).
    pub async fn new_devices_now_and_then(&mut self, client: &Client, log: &mut dyn FnMut(String)) -> Result<()> {
        if self.devices_looked.is_some_and(|at| at.elapsed() < Duration::from_secs(DEVICES_EVERY)) {
            return Ok(());
        }
        self.devices_looked = Some(Instant::now());
        self.new_devices(client, log).await.map(|_| ())
    }

    /// A device that this one has not seen in the shared database before raises an alert here, "a new device
    /// joined" (the owner's word, 2026-11-22: a device that gets in is in at once, "but an alert is
    /// generated"). Every device raises the same alert about the same new device, its id made from the new
    /// device's, so the owner has one on every device, whichever let the new one in, and also when a device
    /// got in with the database's login and no device's help. The first look on a device notes who is there
    /// and raises nothing. How many alerts were raised.
    pub async fn new_devices(&mut self, client: &Client, log: &mut dyn FnMut(String)) -> Result<usize> {
        // (under the name it is shown by: the number it was given when another device goes by its own name)
        let ask = "SELECT id::text, coalesce(nullif(btrim(alias), ''), name), os, app_version, peer_cert_sha256
                   FROM devices WHERE id <> $1::text::uuid ORDER BY first_seen, id LIMIT 1000";
        let rows = client.query(ask, &[&self.device]).await.map_err(said)?;
        let file = self.home.join(KNOWN_DEVICES);
        let kept: Option<Value> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let ids = kept.as_ref().and_then(|v| v.get("ids")).and_then(Value::as_array);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        // (a file of before this was kept has no time of a look: its device was there all along)
        let looked = kept.as_ref().and_then(|v| v.get("looked")).and_then(Value::as_f64).unwrap_or(now);
        // The first look notes who is there and raises nothing; so does the first look after a long absence.
        // Whether a device is new is this device's own knowledge: it has not seen that id before.
        let quiet = ids.is_none() || now - looked > NEW_DEVICE;
        let mut known: Vec<String> =
            ids.into_iter().flatten().filter_map(|id| id.as_str().map(str::to_string)).collect();
        // (the certificate each known device showed, as far as it had one: a device is the same one only
        // with the same certificate)
        let mut certs: serde_json::Map<String, Value> =
            kept.as_ref().and_then(|v| v.get("certs")).and_then(Value::as_object).cloned().unwrap_or_default();
        let (before, certs_before, mut raised) = (known.len(), certs.clone(), 0);
        for row in rows {
            // (a row is read without trusting its shape: one that can't be read is left out, never a panic)
            let Ok(id) = row.try_get::<_, String>(0) else { continue };
            let text = |n: usize| row.try_get::<_, Option<String>>(n).ok().flatten().unwrap_or_default();
            let (name, os, version, cert) = (text(1), text(2), text(3), text(4));
            let had = certs.get(&id).and_then(Value::as_str).unwrap_or("").to_string();
            let mut tell = |alert_id: String, alert: Value, line: String| {
                // (what can't be an alert, a device without a name, is not raised; one that another device's
                // alert already told of is there already)
                if let Ok(Some(_)) = self.inbox.add_as(&alert_id, &alert) {
                    log(line);
                    raised += 1;
                }
            };
            if !known.contains(&id) {
                if !quiet {
                    let alert = json!({"kind": "joined", "name": name,
                                       "text": format!("A new device joined your shared database: {name}"),
                                       "details": {"device": name, "system": os, "version": version}});
                    tell(joined_alert(&id), alert, format!("Shared database: a new device joined, {name}."));
                }
                known.push(id.clone());
            } else if !quiet && !had.is_empty() && !cert.is_empty() && had != cert {
                // A device this one knows shows another certificate than before: the same device set up
                // again, or another machine under its id. The owner is told, as for a device that joined.
                let alert = json!({"kind": "joined", "name": name,
                                   "text": format!("A device of your shared database shows a new certificate: {name}. \
                                                    If you did not set it up again, look at who it is."),
                                   "details": {"device": name, "system": os, "version": version}});
                let line = format!("Shared database: {name} shows a new certificate.");
                tell(joined_alert(&format!("{id} with the certificate {cert}")), alert, line);
            }
            if !cert.is_empty() && had != cert {
                certs.insert(id, json!(cert));
            }
        }
        // (kept when something changed, and now and then for the time of this look)
        if ids_changed(before, known.len(), &certs_before, &certs) || kept.is_none() || now - looked > 3600.0 {
            let note = json!({"ids": known, "certs": certs, "looked": now});
            atomic_write(&file, filejson::dumps(&note).as_bytes())
                .map_err(|e| format!("can't note the shared database's devices: {e}"))?;
        }
        Ok(raised)
    }

    /// This device's copy of the passphrase for keys (Settings: set, ask, approve, restore, overview).
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Each step, for this device's camera: who it sees (`in_view`, named people) is noted as sightings, and
    /// a "moved" alert is raised when someone it sees now was on another device's camera within the last
    /// ten minutes (only this device raises it: the one they arrived at; the alert reaches every device
    /// like the others). `here`: this device's name.
    ///
    /// Sightings are opt-in like the presence log: only while "They move between cameras" is on, and only
    /// people on this device's alert list; nobody else's name and times are written. Older rows are
    /// deleted, every device's, so the table never becomes a record of who was where.
    pub async fn cameras(&mut self, client: &Client, here: &str, in_view: &[String]) -> Result<()> {
        let me = &self.device;
        let settings = self.store.settings()?;
        let track = |key: &str| settings.get("track").and_then(|t| t.get(key)).and_then(Value::as_str).unwrap_or("");
        let on = track("ft-o-optAlertMove") == "1"; // off until switched on
        let who: Vec<String> =
            track("ft-alertWho").split(',').map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).collect();
        let seen: std::collections::BTreeSet<String> =
            in_view.iter().filter(|n| on && who.contains(&n.to_lowercase())).cloned().collect();
        client
            .execute("DELETE FROM sightings WHERE last_seen < now() - make_interval(secs => $1)", &[&KEEP_SIGHTINGS])
            .await
            .map_err(said)?;
        for name in &seen {
            let note = "INSERT INTO sightings (device, name) VALUES ($1::text::uuid, $2)
                        ON CONFLICT (device, name) DO UPDATE SET last_seen = now(),
                        first_seen = CASE WHEN sightings.last_seen < now() - interval '1 minute'
                                          THEN now() ELSE sightings.first_seen END";
            client.execute(note, &[me, name]).await.map_err(said)?;
        }
        let arrived: Vec<String> = seen.difference(&self.seen_before).cloned().collect();
        self.seen_before = seen;
        for name in arrived {
            let ask = "SELECT coalesce(nullif(btrim(d.alias), ''), d.name), extract(epoch FROM now() - s.last_seen)::float8
                       FROM sightings s JOIN devices d ON d.id = s.device
                       WHERE s.name = $1 AND s.device <> $2::text::uuid AND s.last_seen > now() - make_interval(secs => $3)
                       ORDER BY s.last_seen DESC LIMIT 1";
            if let Some(row) = client.query_opt(ask, &[&name, me, &MOVE_WINDOW]).await.map_err(said)? {
                let (from, ago): (String, f64) = (row.get(0), row.get(1));
                let alert = json!({"kind": "moved", "name": name, "text": format!("{name} moved from {from}'s camera to {here}'s"),
                                   "details": {"from": from, "to": here, "secondsBetween": ago.max(0.0).round() as i64}});
                // (what can't be an alert, a name of 300 letters, is not raised)
                let _ = self.inbox.add(&alert);
            }
        }
        Ok(())
    }

    /// After each sending: what is only ever sent (the expression log's and the music log's waiting rows,
    /// the app's log, the posed tests' lines). Then, once a day (and after joining): what the server holds
    /// about forgotten people is cleared again. A device that was away during a forget sends its log,
    /// history and test lines about them when it is back, before it hears of the forget.
    pub async fn after_push(&self, client: &mut Client, state: &State) -> Result<()> {
        sent::EXPRESSION.send(client, state, &self.home, &self.device).await?;
        sent::MUSIC.send(client, state, &self.home, &self.device).await?;
        sent::app_log(client, state, &self.home, &self.device).await?;
        sent::posed_tests(client, state, &self.data, &self.device).await?;
        let today = sent::today_number();
        if state.get("forgot_swept_day")?.as_deref() != Some(today.as_str()) {
            let names = forgotten::names(&self.home, false).map_err(|e| e.to_string())?;
            if !names.is_empty() {
                let tx = client.transaction().await.map_err(said)?;
                for name in &names {
                    forget_on_server(&tx, name).await?;
                }
                tx.commit().await.map_err(said)?;
            }
            state.put("forgot_swept_day", &today)?;
        }
        Ok(())
    }

    /// After joining: of what is only sent, only what is written from now on.
    pub fn on_join(&self, state: &State) -> Result<()> {
        sent::on_join(state, &self.home, &self.data)
    }

    /// Merge this device's private folder into a database it joins:
    /// what is in the database already stays (the first description of a photo, an alert, a visit). The
    /// device's settings, signals and forgotten names go with the first sending after it.
    pub async fn merge_in(&self, client: &mut Client) -> Result<()> {
        let me = &self.device;
        let tx = client.transaction().await.map_err(said)?;
        let send = |statement: &'static str, rows: Vec<Vec<Param>>| {
            let tx = &tx;
            async move {
                if rows.is_empty() {
                    return Ok(());
                }
                let prepared = tx.prepare(statement).await.map_err(said)?;
                run_all(tx, &prepared, &rows).await
            }
        };
        let described = {
            let _one = descriptions::hold();
            descriptions::load(&self.home)?
        };
        let rows = described
            .iter()
            .map(|(k, v)| -> Vec<Param> { vec![Box::new(k.clone()), Box::new(filejson::dumps(&utc(v)))] });
        send(
            "INSERT INTO descriptions (key, data) VALUES ($1, $2::text::jsonb) ON CONFLICT DO NOTHING",
            rows.collect(),
        )
        .await?;

        let names: Vec<String> = self.snapshots().into_keys().collect();
        let rows = self.rows("webcam_snapshots", &names)?.into_iter().filter_map(|(k, row)| {
            let Row::Snapshot { jpeg, verdict } = row else { return None };
            let taken = verdict.as_ref().and_then(|v| v.get("when")).and_then(Value::as_str).and_then(utc_time);
            let args: Vec<Param> = vec![
                Box::new(k),
                Box::new(me.clone()),
                Box::new(taken),
                Box::new(jpeg),
                Box::new(verdict.map(|v| filejson::dumps(&utc(&v)))),
            ];
            Some(args)
        });
        send(
            "INSERT INTO webcam_snapshots (name, device, taken, jpeg, verdict)
             VALUES ($1, $2::text::uuid, $3::text::timestamptz, $4::bytea, $5::text::jsonb) ON CONFLICT DO NOTHING",
            rows.collect(),
        )
        .await?;

        let rows = self
            .local("alerts")?
            .into_iter()
            .map(|(k, v)| -> Vec<Param> { vec![Box::new(k), Box::new(me.clone()), Box::new(filejson::dumps(&v))] });
        send(
            "INSERT INTO alerts (id, device, data) VALUES ($1, $2::text::uuid, $3::text::jsonb) ON CONFLICT DO NOTHING",
            rows.collect(),
        )
        .await?;

        // (a visit taken from another device before keeps that device: it is not sent as this device's own)
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let rows = self.presence()?.into_iter().map(|(k, v)| -> Vec<Param> {
            vec![
                Box::new(k),
                Box::new(text(&v, "device")),
                Box::new(text(&v, "name")),
                Box::new(v.get("start").and_then(Value::as_i64)),
                Box::new(v.get("end").and_then(Value::as_i64)),
            ]
        });
        send(
            "INSERT INTO presence (id, device, name, start_ms, end_ms) VALUES ($1, $2::text::uuid, $3, $4::bigint, $5::bigint)
             ON CONFLICT DO NOTHING",
            rows.collect(),
        )
        .await?;

        // the documents, and the photo folders this device reads
        let mut docs = self.local("documents")?;
        if let Some(folders) =
            read_json(&self.data.join("settings.json")).and_then(|s| s.get("folders").filter(|f| !f.is_null()).cloned())
        {
            docs.insert(format!("{me}:photo_folders"), json!({"name": "photo_folders", "body": folders, "device": me}));
        }
        let rows = docs.into_iter().map(|(k, v)| -> Vec<Param> {
            vec![
                Box::new(k),
                Box::new(text(&v, "device")),
                Box::new(text(&v, "name")),
                Box::new(filejson::dumps(&v["body"])),
            ]
        });
        send(
            "INSERT INTO documents (id, device, name, body) VALUES ($1, $2::text::uuid, $3, $4::text::jsonb) ON CONFLICT DO NOTHING",
            rows.collect(),
        )
        .await?;

        // Web search use this month: the larger count stays, so a cap is never undercounted
        let rows = [("vision", "vision_usage.json"), ("rekognition", "rekognition_usage.json")].into_iter().filter_map(
            |(service, file)| {
                let used = read_json(&self.home.join(file))?;
                let month = used.get("month")?.as_str().filter(|m| !m.is_empty())?.to_string();
                let args: Vec<Param> = vec![
                    Box::new(service),
                    Box::new(month),
                    Box::new(used.get("used").and_then(Value::as_i64).unwrap_or(0) as i32),
                ];
                Some(args)
            },
        );
        send(
            "INSERT INTO usage (service, month, used) VALUES ($1, $2, $3::int)
             ON CONFLICT (service, month) DO UPDATE SET used = greatest(usage.used, excluded.used)",
            rows.collect(),
        )
        .await?;
        // every line of the posed tests, and the app's log
        sent::merge_in(&tx, &self.home, &self.data, me).await?;
        tx.commit().await.map_err(said)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const ME: &str = "11111111-1111-1111-1111-111111111111";
    const OTHER: &str = "22222222-2222-2222-2222-222222222222";
    /// a time in November 2026, in ms
    const T0: i64 = 1_762_765_200_000;

    fn part(name: &str) -> (Home, PathBuf) {
        let dir = std::env::temp_dir().join(format!("face-sync-home-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::create_dir_all(dir.join("home")).unwrap();
        (Home::new(&dir.join("data"), &dir.join("home"), ME), dir)
    }

    fn plain(key: &str, v: Value) -> (String, Option<Row>) {
        (key.to_string(), Some(Row::Json(v)))
    }

    fn gone(key: &str) -> (String, Option<Row>) {
        (key.to_string(), None)
    }

    #[test]
    fn a_rows_digest_is_what_the_devices_have_kept_so_far() {
        // (the values are fixed for these rows: a digest that came out differently would
        // have a device send every row of its private folder again)
        assert_eq!(
            h(&json!({"name": "Ann", "start": 1, "end": 2, "device": "d"})),
            "e0c37fcbdfcf2e945e55c098f213a5d0e82dcb65"
        );
        assert_eq!(h(&json!("Jos\u{e9}")), "2bf63829e72dd33e2078fd9d5610cda6f64213a0");
        assert_eq!(
            h(&json!({"text": "caf\u{e9}", "secs": 3.5, "n": [1, 2.0, null, true]})),
            "015a17efe52d588935c44ab23a8781890fbda7ae"
        );
        assert_eq!(
            h(&json!([10, 1_731_400_000_123_456_700u64, null, null])),
            "cb2b92c80dea385c85d78fdfb42e0d4cd7be5dd4"
        );
    }

    #[test]
    fn a_name_is_looked_for_as_itself_and_a_line_of_the_log_is_a_visit() {
        assert_eq!(pattern_of("Ann (work) a-b.c_d \u{e9}#~&"), "Ann\\ \\(work\\)\\ a\\-b\\.c_d\\ \u{e9}\\#\\~\\&");
        let line = r#"{"name": "Ann", "start": 1000.0, "end": 2000}"#;
        assert_eq!(
            visit(line, ME),
            Some((format!("{ME}:1000:Ann"), json!({"name": "Ann", "start": 1000, "end": 2000, "device": ME})))
        );
        let theirs = format!(r#"{{"name": "Ann", "start": 1000, "end": 2000, "device": "{OTHER}"}}"#);
        assert_eq!(visit(&theirs, ME).unwrap().0, format!("{OTHER}:1000:Ann"));
        for line in ["", "not a line", r#"{"name": "Ann"}"#, r#"{"start": 1, "end": 2}"#, "[1]"] {
            assert_eq!(visit(line, ME), None, "{line}");
        }
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(folder_text(&format!("a{sep}{sep}b{sep}")), format!("a{sep}b"));
        assert_eq!(folder_text(""), "");
    }

    #[test]
    fn this_devices_rows_are_read_from_its_files() {
        let (part, dir) = part("local");
        let (data, home) = (dir.join("data"), dir.join("home"));
        // nothing kept yet: only what a device always has, its settings as they are when nothing is set
        let d = part.digests().unwrap();
        assert!(TABLES.iter().filter(|t| !["device_settings", "secrets"].contains(*t)).all(|t| d[t].is_empty()));
        assert!(!d.contains_key("secrets"), "no passphrase here: the keys are not this device's to send or read");
        let settings = part.local("device_settings").unwrap();
        let value = |app: &str, key: &str| settings[&format!("{ME}|{app}|{key}")]["value"].clone();
        assert_eq!(
            (value("describe", "use"), value("describe", "model"), value("describe", "spread")),
            (json!("auto"), json!(""), json!(false))
        );
        assert_eq!((value("lmstudio", "share"), value("lmstudio", "distributed")), (json!(""), json!("0")));
        assert_eq!(settings.len(), 6);

        std::fs::write(home.join("descriptions.json"), r#"{"h1": {"text": "one"}, "h2:30": {"text": "two"}}"#).unwrap();
        std::fs::write(home.join("alerts.json"), r#"[{"id": "a1", "t": 5, "kind": "left"}, {"t": 6}, "x"]"#).unwrap();
        std::fs::create_dir_all(home.join("presence")).unwrap();
        let theirs = format!("{{\"name\": \"Bob\", \"start\": 3, \"end\": 4, \"device\": \"{OTHER}\"}}");
        std::fs::write(
            home.join("presence").join("2026-11-10.jsonl"),
            format!("{{\"name\": \"Ann\", \"start\": 1, \"end\": 2}}\r\nnot a visit\n{theirs}\n"),
        )
        .unwrap();
        std::fs::write(data.join("va_fit.json"), r#"{"a": 1}"#).unwrap();
        std::fs::write(data.join("hw_bench.json"), r#"{"ms": 2.5}"#).unwrap();
        std::fs::write(data.join("another.json"), "{}").unwrap();
        std::fs::write(home.join("forgotten.json"), r#"["Gone"]"#).unwrap();
        std::fs::write(home.join("settings.json"), r#"{"track": {"ft-strict": "0.35"}, "label": {}}"#).unwrap();
        std::fs::write(
            home.join("person_signals.json"),
            r#"{"usual": {"Ann": {"n": 1}}, "music": {"Ann": {"k": 2}}, "else": {"Ann": {}}}"#,
        )
        .unwrap();
        std::fs::write(home.join("describe.json"), r#"{"use": "server", "url": "http://10.1.2.3:1234", "model": "m", "spread": true, "key": "kept elsewhere"}"#).unwrap();
        std::fs::write(home.join("lmstudio.json"), r#"{"distributed": true}"#).unwrap();
        std::fs::create_dir_all(home.join("webcam")).unwrap();
        std::fs::write(home.join("webcam").join("webcam-1.jpg"), b"\xff\xd8one").unwrap();
        std::fs::write(home.join("webcam").join("webcam-1.json"), r#"{"name": "Ann"}"#).unwrap();
        std::fs::write(home.join("webcam").join("webcam-2.jpg"), b"\xff\xd8two").unwrap();
        std::fs::write(home.join("webcam").join("other.jpg"), b"x").unwrap();

        let keys = |tbl: &str| part.digests().unwrap()[tbl].keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys("descriptions"), ["h1", "h2:30"]);
        assert_eq!(keys("alerts"), ["a1"], "what has no id is not an alert");
        assert_eq!(keys("presence"), [format!("{ME}:1:Ann"), format!("{OTHER}:3:Bob")]);
        assert_eq!(keys("documents"), ["va_fit".to_string(), format!("{ME}:hw_bench")]);
        assert_eq!(keys("forgotten"), ["Gone"]);
        assert_eq!(keys("person_signals"), ["Ann|usual", "Ann|music"]);
        assert_eq!(keys("webcam_snapshots"), ["webcam-1", "webcam-2"]);
        let settings = part.local("device_settings").unwrap();
        assert_eq!(
            settings[&format!("{ME}|track|ft-strict")],
            json!({"app": "track", "key": "ft-strict", "value": "0.35"})
        );
        assert_eq!(settings[&format!("{ME}|describe|url")]["value"], json!("http://10.1.2.3:1234"));
        assert_eq!(
            (
                settings[&format!("{ME}|describe|spread")]["value"].clone(),
                settings[&format!("{ME}|lmstudio|distributed")]["value"].clone()
            ),
            (json!(true), json!("1"))
        );
        assert!(!settings.keys().any(|k| k.contains("|describe|key")), "a key is a secret, never a setting");
        // the rows as they are sent
        let rows = part
            .rows("webcam_snapshots", &["webcam-2".to_string(), "webcam-1".to_string(), "webcam-9".to_string()])
            .unwrap();
        assert_eq!(rows[0], ("webcam-2".to_string(), Row::Snapshot { jpeg: b"\xff\xd8two".to_vec(), verdict: None }));
        assert_eq!(rows[1].1, Row::Snapshot { jpeg: b"\xff\xd8one".to_vec(), verdict: Some(json!({"name": "Ann"})) });
        assert_eq!(rows.len(), 2);
        let rows = part.rows("documents", &["va_fit".to_string()]).unwrap();
        assert_eq!(
            rows,
            [("va_fit".to_string(), Row::Json(json!({"name": "va_fit", "body": {"a": 1}, "device": null})))]
        );
        // a file that is there and can't be read is an error, never "nothing here": that would be sent on
        // as everything deleted
        std::fs::write(home.join("forgotten.json"), "{").unwrap();
        assert!(part.digests().is_err());
        assert!(part.local("no such table").is_err() && part.rows("no such table", &[]).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rows_from_the_database_are_put_where_this_device_keeps_them() {
        let (mut part, dir) = part("apply");
        let (data, home) = (dir.join("data"), dir.join("home"));
        let mut said: Vec<String> = Vec::new();
        let text = |file: PathBuf| std::fs::read_to_string(file).unwrap();

        // descriptions
        let got = part
            .apply(
                "descriptions",
                vec![plain("h1", json!({"text": "caf\u{e9}"})), plain("h2", json!({"text": "two"}))],
                &mut |l| said.push(l),
            )
            .unwrap();
        assert_eq!(got["h1"], Some(h(&json!({"text": "caf\u{e9}"}))));
        let got = part.apply("descriptions", vec![gone("h2"), gone("h3")], &mut |l| said.push(l)).unwrap();
        assert_eq!((&got["h2"], &got["h3"]), (&None, &None));
        assert_eq!(text(home.join("descriptions.json")), "{\n\"h1\": {\"text\": \"caf\u{e9}\"}\n}");

        // Collect faces pictures: a name is only ever a file's name in the folder
        let shot = |name: &str, verdict: Option<Value>| {
            (name.to_string(), Some(Row::Snapshot { jpeg: b"\xff\xd8x".to_vec(), verdict }))
        };
        let rows = vec![
            shot("webcam-1", Some(json!({"name": "Ann"}))),
            shot("webcam-2", None),
            shot("../webcam-3", None),
            shot("webcam-", None),
            shot("other", None),
        ];
        let got = part.apply("webcam_snapshots", rows, &mut |l| said.push(l)).unwrap();
        assert_eq!(got.len(), 2);
        assert!(
            got["webcam-1"].is_some()
                && got["webcam-1"] == part.digests().unwrap()["webcam_snapshots"].get("webcam-1").cloned()
        );
        let folder = home.join("webcam");
        assert_eq!(text(folder.join("webcam-1.json")), r#"{"name": "Ann"}"#);
        assert_eq!(std::fs::read(folder.join("webcam-2.jpg")).unwrap(), b"\xff\xd8x");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 3);
        assert!(!home.join("webcam-3.jpg").exists() && !dir.join("webcam-3.jpg").exists());
        let got = part.apply("webcam_snapshots", vec![gone("webcam-1")], &mut |l| said.push(l)).unwrap();
        assert_eq!((got["webcam-1"].clone(), std::fs::read_dir(&folder).unwrap().count()), (None, 1));

        // alerts: kept oldest first
        let alert = |id: &str, t: i64| json!({"id": id, "t": t, "kind": "left", "name": "Ann", "text": "Ann left", "state": "new"});
        let got = part
            .apply("alerts", vec![plain("a2", alert("a2", 2000)), plain("a1", alert("a1", 1000))], &mut |l| {
                said.push(l)
            })
            .unwrap();
        assert_eq!(got["a1"], Some(h(&alert("a1", 1000))));
        let ids = |part: &Home| {
            part.inbox.all().unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        };
        assert_eq!(ids(&part), ["a1", "a2"]);
        let got = part.apply("alerts", vec![gone("a1"), plain("a2", alert("a2", 500))], &mut |l| said.push(l)).unwrap();
        assert_eq!((ids(&part), got["a1"].clone()), (vec!["a2".to_string()], None));

        // presence: another device's visit keeps its device, this device's own says none; a visit that is
        // here already is not written twice
        let (theirs, mine) = (format!("{OTHER}:{T0}:Ann"), format!("{ME}:{}:Bob", T0 + 5000));
        let rows = || {
            vec![
                plain(&theirs, json!({"name": "Ann", "start": T0, "end": T0 + 1000, "device": OTHER})),
                plain(&mine, json!({"name": "Bob", "start": T0 + 5000, "end": T0 + 6000, "device": ME})),
            ]
        };
        let got = part.apply("presence", rows(), &mut |l| said.push(l)).unwrap();
        assert!(got[&theirs].is_some() && got[&mine].is_some());
        part.apply("presence", rows(), &mut |l| said.push(l)).unwrap();
        let file = home.join("presence").join(format!("{}.jsonl", presence::day_of(T0).unwrap()));
        let end = presence::END;
        let ann = format!("{{\"name\": \"Ann\", \"start\": {T0}, \"end\": {}, \"device\": \"{OTHER}\"}}", T0 + 1000);
        let bob = format!("{{\"name\": \"Bob\", \"start\": {}, \"end\": {}}}", T0 + 5000, T0 + 6000);
        assert_eq!(text(file.clone()), format!("{ann}{end}{bob}{end}"));
        let got = part.apply("presence", vec![gone(&theirs), gone("nobody's")], &mut |l| said.push(l)).unwrap();
        assert_eq!((got[&theirs].clone(), text(file)), (None, format!("{bob}\n")));

        // documents: the fitted ones are written, a device's own measurements never
        let rows = vec![
            plain(
                "expr_calibration",
                json!({"name": "expr_calibration", "body": {"w": 0.5, "n": "\u{e9}"}, "device": null}),
            ),
            plain(&format!("{ME}:model_eval"), json!({"name": "model_eval", "body": {"ms": 1}, "device": ME})),
        ];
        let got = part.apply("documents", rows, &mut |l| said.push(l)).unwrap();
        assert_eq!(text(data.join("expr_calibration.json")), "{\n \"w\": 0.5,\n \"n\": \"\\u00e9\"\n}");
        assert!(!data.join("model_eval.json").exists());
        assert_eq!((got["expr_calibration"].is_some(), got[&format!("{ME}:model_eval")].clone()), (true, None));

        // a person forgotten on another device: forgotten here once, and the name held; named again there:
        // no longer held
        let asked = Arc::new(Mutex::new(Vec::new()));
        let noted = asked.clone();
        part.forget = Some(Box::new(move |name| {
            noted.lock().unwrap().push(name.to_string());
            if name == "Bad" { Err("a file is locked".to_string()) } else { Ok(()) }
        }));
        let got = part
            .apply("forgotten", vec![plain("Ann", json!({})), plain("Bad", json!({}))], &mut |l| said.push(l))
            .unwrap();
        assert_eq!(got["Ann"], Some(h(&json!("Ann"))));
        part.apply("forgotten", vec![plain("Ann", json!({}))], &mut |l| said.push(l)).unwrap();
        assert_eq!(*asked.lock().unwrap(), ["Ann", "Bad"], "asked once for each");
        assert!(said.iter().any(|l| l == "Shared database: forgot Ann, as another device did."));
        assert!(said.iter().any(|l| l == "Shared database: couldn't forget Bad here: a file is locked"));
        let got = part.apply("forgotten", vec![gone("Ann")], &mut |l| said.push(l)).unwrap();
        assert!(!got.contains_key("Ann"));
        assert_eq!(forgotten::names(&home, true).unwrap().into_iter().collect::<Vec<_>>(), ["Bad"]);
        // without a way to forget here, nothing is done and the name is not held, so it is done once there is one
        part.forget = None;
        let got = part.apply("forgotten", vec![plain("Cy", json!({}))], &mut |l| said.push(l)).unwrap();
        assert!(got.is_empty() && !forgotten::names(&home, true).unwrap().contains("Cy"));

        // this device's settings never come from the database: a row that differs is sent again as it is
        // here, one this device never wrote is deleted
        part.store.update("track", &json!({"ft-strict": "0.35"})).unwrap();
        let key = |k: &str| format!("{ME}|track|{k}");
        let row = |k: &str, v: &str| plain(&key(k), json!({"app": "track", "key": k, "value": v}));
        said.clear();
        let got = part
            .apply("device_settings", vec![row("ft-strict", "0.35"), gone(&key("ft-none"))], &mut |l| said.push(l))
            .unwrap();
        assert_eq!(got[&key("ft-strict")], Some(h(&json!({"app": "track", "key": "ft-strict", "value": "0.35"}))));
        assert!(got[&key("ft-none")].is_none() && said.is_empty());
        let got = part
            .apply(
                "device_settings",
                vec![row("ft-strict", "0.9"), row("ft-stream", "1"), gone(&key("ft-strict"))],
                &mut |l| said.push(l),
            )
            .unwrap();
        assert_eq!((got[&key("ft-strict")].clone(), got[&key("ft-stream")].as_deref()), (None, Some(NOT_OURS)));
        assert_eq!(part.store.setting("track", "ft-strict").as_deref(), Some("0.35"));
        assert!(part.store.setting("track", "ft-stream").is_none());
        assert!(said[0].starts_with("Shared database: 3 of this device's settings are different in the database (track: ft-strict, track: ft-stream, track: ft-strict)."), "{said:?}");

        // person signals: every kind the programs keep, and nothing else
        let signal = |name: &str, kind: &str, n: i64| {
            plain(&format!("{name}|{kind}"), json!({"name": name, "kind": kind, "data": {"n": n}}))
        };
        said.clear();
        let rows = vec![
            signal("Ann", "usual", 1),
            signal("Ann", "music", 2),
            signal("Ann", "unknown", 3),
            signal("#x", "usual", 4),
            signal("A|B", "baseline", 5),
        ];
        let got = part.apply("person_signals", rows, &mut |l| said.push(l)).unwrap();
        assert_eq!(got["Ann|usual"], Some(h(&json!({"name": "Ann", "kind": "usual", "data": {"n": 1}}))));
        assert!(
            got["Ann|music"].is_some()
                && got["A|B|baseline"].is_some()
                && got["Ann|unknown"].is_none()
                && got["#x|usual"].is_none()
        );
        assert_eq!(said, ["Shared database: person signals: bad signals"]);
        let got = part.apply("person_signals", vec![gone("Ann|usual")], &mut |l| said.push(l)).unwrap();
        assert!(got["Ann|usual"].is_none());
        assert_eq!(
            part.digests().unwrap()["person_signals"].keys().cloned().collect::<Vec<_>>(),
            ["A|B|baseline", "Ann|music"]
        );
        assert!(part.apply("no such table", vec![], &mut |_| ()).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_row_this_device_cannot_use_is_refused_with_the_reason() {
        let (part, dir) = part("check");
        let why = |tbl: &str, key: &str, v: Value| part.check(tbl, key, &Row::Json(v)).err();
        assert_eq!(why("descriptions", "h1", json!({"text": "x"})), None);
        assert_eq!(why("descriptions", "", json!({})).as_deref(), Some("no key"));
        assert_eq!(why("descriptions", "h1", json!("text")).as_deref(), Some("not an object"));
        let alert = |change: &dyn Fn(&mut Value)| {
            let mut a = json!({"id": "a1", "t": 5, "kind": "left", "name": "Ann", "text": "", "state": null, "details": {"first": {"n": 1}, "later": null}});
            change(&mut a);
            why("alerts", "a1", a)
        };
        assert_eq!(alert(&|_| ()), None);
        // (an alert without its id was once saved with the others and then stopped every later step)
        assert_eq!(alert(&|a| a["id"] = json!("a2")).as_deref(), Some("not an alert with this id"));
        assert_eq!(alert(&|a| a["t"] = json!("soon")).as_deref(), Some("no time"));
        assert_eq!(alert(&|a| a["t"] = json!(-1)).as_deref(), Some("no time"));
        assert_eq!(alert(&|a| a["name"] = json!(["Ann"])).as_deref(), Some("not text"));
        assert_eq!(alert(&|a| a["details"] = json!({"first": 3})).as_deref(), Some("details: not objects"));
        assert_eq!(alert(&|a| a["details"] = json!(null)), None);
        let visit = |key: &str, v: Value| why("presence", key, v);
        assert_eq!(
            visit(&format!("{OTHER}:5:Ann"), json!({"name": "Ann", "start": 5, "end": 9, "device": OTHER})),
            None
        );
        assert_eq!(
            visit("d:5:Ann", json!({"name": "", "start": 5, "end": 9, "device": "d"})).as_deref(),
            Some("no name")
        );
        assert_eq!(
            visit("d:5:Ann", json!({"name": "Ann", "start": 5, "end": PRESENCE_MAX, "device": "d"})).as_deref(),
            Some("times out of range")
        );
        assert_eq!(
            visit("d:5:Ann", json!({"name": "Ann", "start": 0, "end": 9, "device": "d"})).as_deref(),
            Some("times out of range")
        );
        assert_eq!(
            visit("d:6:Ann", json!({"name": "Ann", "start": 5, "end": 9, "device": "d"})).as_deref(),
            Some("not this visit's id")
        );
        assert_eq!(why("documents", "va_fit", json!({"name": "va_fit", "body": {}})), None);
        assert_eq!(
            why("documents", "va_fit", json!({"name": "va_fit", "body": [1]})).as_deref(),
            Some("body: not an object")
        );
        assert_eq!(why("documents", "d:hw_bench", json!({"name": "hw_bench", "body": [1]})), None);
        assert_eq!(why("documents", "x", json!({"body": {}})).as_deref(), Some("no name"));
        assert_eq!(why("forgotten", "Ann", json!({})), None);
        assert_eq!(why("forgotten", "#ignore", json!({})).as_deref(), Some("not a person's name"));
        assert_eq!(why("forgotten", &"n".repeat(201), json!({})).as_deref(), Some("not a person's name"));
        assert_eq!(why("device_settings", "d|track|ft-x", json!({"value": 1})), None);
        assert_eq!(why("device_settings", "d|track", json!({})).as_deref(), Some("not device|app|key"));
        assert_eq!(why("person_signals", "Ann|usual", json!({"data": {}})), None);
        assert_eq!(
            why("person_signals", "Ann|usual", json!({"data": [1]})).as_deref(),
            Some("not name|kind with an object")
        );
        assert_eq!(
            why("person_signals", "__proto__|usual", json!({"data": {}})).as_deref(),
            Some("a name no page can keep")
        );
        let shot = |verdict: Option<Value>| {
            part.check("webcam_snapshots", "webcam-1", &Row::Snapshot { jpeg: vec![1], verdict }).err()
        };
        assert_eq!((shot(None), shot(Some(json!({})))), (None, None));
        assert_eq!(shot(Some(json!([1]))).as_deref(), Some("verdict: not an object"));
        assert!(
            why("webcam_snapshots", "webcam-1", json!({})).is_some() && why("no such table", "k", json!({})).is_some()
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
