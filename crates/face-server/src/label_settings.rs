//! What the app's Settings tab asks of the labeling tool about the shared database:
//! its settings and state, connecting to it, the passphrase
//! for keys, the saved keys shown masked, the Web search keys, sharing with the other devices, and taking
//! another device's settings.
//!
//! None of these touch the labeling data, so none is answered under its lock: they ask the database (on a
//! connection kept for short questions, or one of their own for what changes something) and this device's
//! own files.

use face_db::{Device, Settings, join::Login};
use face_home::saved;
use face_sync::{
    home::Home,
    keys::{Fault, Keys},
};
use serde_json::{Map, Value, json};
use tokio_postgres::Client;

use crate::{
    http::{Request, Response},
    label::{Answer, Labeler, asked},
};

/// The POSTs answered here (for the size a body may have).
pub const POSTS: [&str; 13] = [
    "/api/db/setup",
    "/api/db/device",
    "/api/db/alias",
    "/api/db/locations/forget",
    "/api/db/signals/forget",
    "/api/db/passphrase",
    "/api/db/passphrase/restore",
    "/api/db/passphrase/ask",
    "/api/db/passphrase/approve",
    "/api/db/copy",
    "/api/peer",
    "/api/websearch/aws",
    "/api/websearch/key",
];

fn refused(e: Fault) -> Response {
    if e.refused() { Response::error(400, e.text()) } else { Response::error(500, e.text()) }
}

/// A time of the database as the pages read times.
const WHEN: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'";

impl Labeler {
    fn device(&self) -> Result<Device, String> {
        Device::load(&self.home, &crate::device().0)
    }

    fn keys(&self) -> Result<Keys, String> {
        Ok(Keys::new(&self.home, &self.device()?.id))
    }

    /// Something about the shared database was set here: the sync looks at once.
    fn sync_soon(&self) {
        if let Some(syncer) = self.syncer() {
            syncer.note();
        }
    }

    /// On a connection of its own, for what changes something in the database (the owner's own, rare
    /// actions). Without a shared database: told to connect first.
    fn in_database<T>(&self, action: impl AsyncFnOnce(&mut Client, &Device) -> Result<T, Fault>) -> Result<T, Fault> {
        let settings = Settings::load(&self.home).map_err(Fault::Failed)?;
        let Some(settings) = settings else {
            return Err(Fault::Bad("Connect to the shared database first.".to_string()));
        };
        let device = self.device().map_err(Fault::Failed)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Fault::Failed(e.to_string()))?;
        rt.block_on(async {
            let mut client = face_db::connect(&settings, &settings.schema, &device).await.map_err(Fault::Failed)?;
            action(&mut client, &device).await
        })
    }

    /// The shared database for the Settings tab: {available, config (no password), device, sync, share,
    /// passphrase (this device has one), devices} and, while connected, the other devices, the keys'
    /// overview and who shares what.
    pub(crate) fn db_state(&self) -> Result<Value, String> {
        let device = self.device()?;
        let config = face_db::config(&self.home);
        let sync = self.sync_status();
        let mut out = json!({"available": true, "config": config, "device": {"id": device.id, "name": device.name}, "sync": sync,
                             "share": face_db::peer::sharing(&self.home), "passphrase": Keys::new(&self.home, &device.id).local_key().is_some(),
                             "devices": []});
        if config.is_some() && sync["state"] == "online" {
            let keys = Keys::new(&self.home, &device.id);
            // on the connection kept for short questions: Settings asks this every few seconds while it is open
            let others = self.kept.ask(async |client, _| {
                let said = |e: tokio_postgres::Error| e.to_string();
                // every device with what the owner calls it (`name`), what it calls itself (`own`) and the alias
                // alone; this device too (`me`): it can be named like the others
                let name = face_db::DEVICE_NAME;
                let ask = format!(
                    "SELECT id::text, {name}, to_char(last_seen AT TIME ZONE 'UTC', {WHEN}), name, alias, os FROM devices ORDER BY 2"
                );
                let named: Vec<Value> = (client.query(&ask, &[]).await.map_err(said)?.iter())
                    .map(|r| {
                        let id: String = r.get(0);
                        json!({"me": id == device.id, "id": id, "name": r.get::<_, String>(1), "own": r.get::<_, String>(3),
                               "alias": r.get::<_, String>(4), "os": r.get::<_, String>(5)})
                    })
                    .collect();
                let ask = format!(
                    "SELECT id::text, {name}, to_char(last_seen AT TIME ZONE 'UTC', {WHEN}) FROM devices WHERE id <> $1::text::uuid ORDER BY 2"
                );
                // (with whether this device's owner removed that one here: this device's own list, in no row)
                let removed = face_home::blocked::ids(&self.home);
                let devices: Vec<Value> = (client.query(&ask, &[&device.id]).await.map_err(said)?.iter())
                    .map(|r| {
                        let id: String = r.get(0);
                        json!({"removed": removed.contains(&id), "id": id, "name": r.get::<_, String>(1), "seen": r.get::<_, String>(2)})
                    })
                    .collect();
                let ask = "SELECT coalesce(nullif(btrim(alias), ''), name), peer_share, peer_addresses, peer_port FROM devices ORDER BY 1";
                let peers: Vec<Value> = (client.query(ask, &[]).await.map_err(said)?.iter())
                    .map(|r| {
                        let (addresses, port): (Option<Vec<String>>, Option<i32>) = (r.get(2), r.get(3));
                        json!({"name": r.get::<_, String>(0), "share": r.get::<_, bool>(1), "addresses": addresses.unwrap_or_default(), "port": port})
                    })
                    .collect();
                let overview = keys.overview(client).await.map_err(|e| e.text().to_string())?;
                Ok(json!({"devices": devices, "named": named, "keys": overview, "peers": peers}))
            });
            if let (Ok(Value::Object(others)), Some(out)) = (others, out.as_object_mut()) {
                out.extend(others);
            }
        }
        Ok(out)
    }

    /// An action's answer with the database's state after it.
    fn with_state(&self, answer: Value) -> Answer {
        let mut out = answer.as_object().cloned().unwrap_or_default();
        out.extend(self.db_state().map_err(|e| Response::error(500, e))?.as_object().cloned().unwrap_or_default());
        Ok(Response::json(Value::Object(out)))
    }

    /// Which services Web search can use: {vision, rekognition}.
    fn services(&self) -> Value {
        json!({"vision": !self.google_key().is_empty(), "rekognition": saved::aws(&self.home).is_some()})
    }

    /// The Google key in use: `GOOGLE_VISION_API_KEY`, else the one saved here.
    fn google_key(&self) -> String {
        saved::google_key_in_use(&self.home)
    }

    /// The addresses of the owner's other devices: what each device in the shared database says it can be
    /// reached at (`devices.peer_addresses`). Asked at most once a minute, on the connection kept for short
    /// questions; while the database can't be reached the last answer stands. None without a shared database.
    /// Each device's addresses together (the devices removed on this device left out).
    #[cfg(feature = "import")]
    pub(crate) fn other_devices(&self) -> Vec<Vec<String>> {
        // (each device's id with its addresses: who is removed on this device is asked of its own file
        // every time, so a device removed in Settings gets no photo from that moment on)
        type Found = Vec<(String, Vec<String>)>;
        static KEPT: std::sync::Mutex<Option<(std::time::Instant, Found)>> = std::sync::Mutex::new(None);
        if !matches!(Settings::load(&self.home), Ok(Some(_))) {
            return Vec::new();
        }
        let removed = face_home::blocked::ids(&self.home);
        let of_the_rest = |found: &Found| -> Vec<Vec<String>> {
            (found.iter().filter(|(id, _)| !removed.contains(id))).map(|(_, addresses)| addresses.clone()).collect()
        };
        let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, found)) = &*kept
            && at.elapsed() < std::time::Duration::from_secs(60)
        {
            return of_the_rest(found);
        }
        let me = self.device().map(|d| d.id).unwrap_or_default();
        let asked = self.kept.ask(async |client, _| {
            let rows = client
                .query("SELECT id::text, peer_addresses FROM devices WHERE id <> $1::text::uuid", &[&me])
                .await
                .map_err(|e| e.to_string())?;
            Ok(rows
                .iter()
                .map(|r| (r.get::<_, String>(0), r.get::<_, Option<Vec<String>>>(1).unwrap_or_default()))
                .collect::<Found>())
        });
        let found = asked.unwrap_or_else(|_| kept.as_ref().map(|(_, found)| found.clone()).unwrap_or_default());
        *kept = Some((std::time::Instant::now(), found.clone()));
        of_the_rest(&found)
    }

    /// One Web search counted in the shared database before it is sent, so that every device's searches
    /// count toward one cap a service: Ok(Some(used this month)); Ok(None) without a shared database or
    /// while it can't be reached (this device then counts alone); Err past the cap.
    #[cfg(feature = "import")]
    pub(crate) fn count_in_database(&self, service: &str, month: &str, cap: i64) -> Result<Option<i64>, String> {
        if !matches!(Settings::load(&self.home), Ok(Some(_))) {
            return Ok(None);
        }
        let counted = self.in_database(async |client, _| {
            face_sync::home::count_use(client, service, month, cap).await.map_err(Fault::Failed)
        });
        match counted {
            Ok(Some(used)) => Ok(Some(used)),
            Ok(None) => Err(format!("{service} limit reached: {cap} searches this month (keeps the bill under $10)")),
            Err(_) => Ok(None),
        }
    }

    /// Every saved key and password, for the Settings tab to show masked (each can be shown while its eye
    /// button is held): the Web search keys, the describe server's key, the shared database's password,
    /// the passphrase for keys, and this device's one-time code while it asks for the passphrase.
    fn secrets(&self) -> Result<Value, String> {
        let (id, secret, region) = saved::aws(&self.home).unwrap_or_default();
        let keys = self.keys()?;
        Ok(json!({"google": saved::google_key(&self.home),
                  "aws": {"id": id, "secret": secret, "region": region.unwrap_or_default()},
                  "describe": saved::describe_key(&self.home).1,
                  "db": face_db::saved_login(&self.home).unwrap_or_else(|| json!({})),
                  "passphrase": keys.saved_passphrase(),
                  "request": keys.request()}))
    }

    /// The Settings tab's GETs: Some when the path is one of them.
    pub(crate) fn settings_get(&self, req: &Request) -> Option<Answer> {
        let fail = |e: String| Response::error(500, e);
        Some(match req.path.as_str() {
            "/api/db" => self.db_state().map(Response::json).map_err(fail),
            // a device's location history, the newest row first: read on the connection kept for short
            // questions, each row checked as it is read
            "/api/db/locations" => {
                let device = req.param("device").unwrap_or_default();
                let read = self.kept.ask(async |client, _| {
                    let rows = face_db::location::history(client, &device, 200).await?;
                    let count = face_db::location::count(client, &device).await?;
                    let rows: Vec<Value> = rows.iter().map(face_db::location::Row::json).collect();
                    Ok(json!({"device": device, "count": count, "rows": rows}))
                });
                read.map(Response::json).map_err(asked)
            }
            "/api/websearch/key" => {
                Ok(Response::json(json!({"set": !self.google_key().is_empty(), "services": self.services()})))
            }
            "/api/secrets" => {
                // only to the tool's own page (its header), never to a page of another site
                let own = req.headers.get("x-face-tool").map(String::as_str) == Some("1")
                    && req
                        .headers
                        .get("origin")
                        .is_none_or(|o| req.headers.get("host").is_some_and(|h| *o == format!("http://{h}")));
                if !own {
                    return Some(Err(Response::error(
                        403,
                        "Missing the X-Face-Tool header; requests must come from the tool's own page.",
                    )));
                }
                self.secrets().map(Response::json).map_err(fail)
            }
            _ => return None,
        })
    }

    /// The Settings tab's POSTs: Some when the path is one of them.
    pub(crate) fn settings_post(&self, path: &str, body: &Map<String, Value>) -> Option<Answer> {
        let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("");
        let fail = |e: String| Response::error(500, e);
        Some(match path {
            // set up (or with no host, stop using) the shared database
            "/api/db/setup" => {
                let or =
                    |k: &str, other: &'static str| Some(text(k)).filter(|t| !t.is_empty()).unwrap_or(other).to_string();
                let (host, schema) = (text("host").to_string(), or("schema", "facerec"));
                let port = match body.get("port") {
                    Some(Value::Number(n)) => n.as_u64().and_then(|p| u16::try_from(p).ok()),
                    Some(Value::String(s)) if !s.trim().is_empty() => s.trim().parse().ok(),
                    _ => Some(5432),
                };
                let said = |ok: bool, text: String| json!({"ok": ok, "text": text});
                let done = if host.is_empty() {
                    match face_db::forget(&self.home) {
                        Ok(()) => said(
                            true,
                            "The shared database is no longer used: data is kept on this device only.".to_string(),
                        ),
                        Err(e) => return Some(Err(fail(e))),
                    }
                } else if !face_db::schema_name(&schema) {
                    return Some(Err(asked("The schema name may have only lower-case letters, digits and _.")));
                } else {
                    let Some(port) = port.filter(|p| *p > 0) else { return Some(Err(asked("The port is a number."))) };
                    let login =
                        Login::new(&host, port, &or("dbname", "facerec"), text("user"), text("password"), &schema);
                    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                        Ok(rt) => rt,
                        Err(e) => return Some(Err(fail(e.to_string()))),
                    };
                    let device = match self.device() {
                        Ok(d) => d,
                        Err(e) => return Some(Err(fail(e))),
                    };
                    rt.block_on(async {
                        match face_db::setup(&login, &self.home).await {
                            Ok(_) => {
                                face_db::test(&self.home, &device, &crate::device().1, env!("CARGO_PKG_VERSION")).await
                            }
                            Err(e) => said(false, e.trim().lines().next().unwrap_or("").chars().take(320).collect()),
                        }
                    })
                };
                self.sync_soon();
                self.with_state(done)
            }
            // the passphrase for keys: set, enter or change it; restore an earlier one; ask for it with a
            // one-time code; approve the code a waiting device shows
            "/api/db/passphrase"
            | "/api/db/passphrase/restore"
            | "/api/db/passphrase/ask"
            | "/api/db/passphrase/approve" => {
                let done = self.in_database(async |client, device| {
                    let keys = Keys::new(&self.home, &device.id);
                    match path.rsplit('/').next() {
                        Some("restore") => {
                            keys.restore(
                                client,
                                body.get("version")
                                    .and_then(Value::as_i64)
                                    .and_then(|v| i32::try_from(v).ok())
                                    .unwrap_or(0),
                            )
                            .await
                        }
                        Some("ask") => Ok(json!({"ok": true, "request": keys.ask(client).await?})),
                        Some("approve") => {
                            keys.approve(client, text("code"), Some(text("device")).filter(|d| !d.is_empty())).await
                        }
                        _ => keys.set_passphrase(client, text("passphrase"), Some(text("again"))).await,
                    }
                });
                match done {
                    Ok(done) => {
                        self.sync_soon();
                        self.with_state(done)
                    }
                    Err(e) => Err(refused(e)),
                }
            }
            // this device takes another device's settings
            "/api/db/copy" => {
                let copied = self.in_database(async |client, device| {
                    let part = Home::new(&self.data, &self.home, &device.id);
                    part.copy_settings(client, text("from"), &mut |line| println!("{line}")).await
                });
                copied.map(|n| Response::json(json!({"ok": true, "copied": n}))).map_err(refused)
            }
            // share this device's photos and camera with the other devices
            "/api/peer" => {
                let on = match body.get("share") {
                    None | Some(Value::Null) | Some(Value::Bool(false)) => false,
                    Some(Value::Number(n)) => n.as_f64() != Some(0.0),
                    Some(Value::String(s)) => !s.is_empty(),
                    Some(Value::Array(a)) => !a.is_empty(),
                    Some(Value::Object(o)) => !o.is_empty(),
                    Some(Value::Bool(true)) => true,
                };
                if let Err(e) = face_db::peer::set_sharing(&self.home, on) {
                    return Some(Err(fail(e)));
                }
                self.sync_soon();
                Ok(Response::json(json!({"ok": true, "share": face_db::peer::sharing(&self.home)})))
            }
            // what the owner calls a device, by its id: kept in the shared database, so every device shows it
            "/api/db/alias" => {
                let named =
                    self.device().and_then(|me| face_db::name_device(&self.home, &me, text("device"), text("alias")));
                named.map(|name| Response::json(json!({"ok": true, "name": name}))).map_err(asked)
            }
            // a device's whole location history deleted, at the owner's word here (any device may: it is the
            // owner's own data, and a device that is lost can't delete its own)
            "/api/db/locations/forget" => self
                .device()
                .and_then(|me| face_db::location::forget_device(&self.home, &me, text("device")))
                .map(|deleted| Response::json(json!({"ok": true, "deleted": deleted})))
                .map_err(asked),
            // a device's whole signal history deleted, at the owner's word here ("library": the rows worked out
            // from the library's pictures, which are no one device's)
            "/api/db/signals/forget" => self
                .device()
                .and_then(|me| {
                    let which = if text("device") == "library" { face_db::signals::LIBRARY } else { text("device") };
                    face_db::signals::forget_device(&self.home, &me, which)
                })
                .map(|deleted| Response::json(json!({"ok": true, "deleted": deleted})))
                .map_err(asked),
            // a device removed on this device alone, or added back: this device's own list, which changes
            // nothing in the shared database and no password or key
            "/api/db/device" => {
                let removed = !matches!(body.get("removed"), None | Some(Value::Null) | Some(Value::Bool(false)));
                // (with the certificate the device has now, as this device's member list knows it: it stays
                // removed under any id it shows that certificate with)
                let cert = face_db::members::member(&self.home, text("device")).map(|m| m.cert_sha256);
                face_home::blocked::set_with(&self.home, text("device"), removed, cert.as_deref())
                    .map(|list| Response::json(json!({"ok": true, "removed": list})))
                    .map_err(asked)
            }
            // the Web search keys, kept outside the project
            "/api/websearch/aws" => {
                saved::save_aws(&self.home, text("id"), text("secret"), body.get("region").and_then(Value::as_str))
                    .map(|()| Response::json(json!({"ok": true, "services": self.services()})))
                    .map_err(asked)
            }
            "/api/websearch/key" => saved::save_google_key(&self.home, text("key"))
                .map(|()| {
                    Response::json(
                        json!({"ok": true, "set": !self.google_key().is_empty(), "services": self.services()}),
                    )
                })
                .map_err(asked),
            _ => return None,
        })
    }
}
