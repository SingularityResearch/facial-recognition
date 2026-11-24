//! The shared database's sync, running beside the labeling tool:
//! a thread of its own that connects when a shared database is set up,
//! joins it the first time, sends what was saved here a moment after it was saved and once a minute, and
//! takes the other devices' changes every few seconds. Without `HOME/database.json` it does nothing. When
//! the server can't be reached, the device works from its own copy and what changed is sent when it is back.
//!
//! The library is held only while this device's own data is read or written (`face_sync::sync::Library`):
//! the pages never wait for the network.

use std::{
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use face_db::{Device, Settings};
use face_sync::{
    home::Home,
    state::State,
    sync::{self, Library},
};
use serde_json::{Value, json};
use tokio_postgres::Client;

/// between looks for other devices' changes (one cheap question)
const EVERY: Duration = Duration::from_secs(3);
/// between looks at everything this device has, when nothing said that something was saved
const LOCAL_EVERY: Duration = Duration::from_secs(60);
/// before connecting again after a failure (then every minute)
const RETRY: [u64; 3] = [5, 15, 60];
/// between looks for a shared database when none is set up
const UNSET_EVERY: Duration = Duration::from_secs(30);

/// What the sync is doing, for Settings.
#[derive(Clone)]
struct Info {
    /// off | joining | online | offline
    state: &'static str,
    text: String,
    /// when the database was last asked (UTC)
    last: Option<String>,
    error: Option<String>,
    /// rows changed here and not sent (counted while the server can't be reached)
    waiting: usize,
}

/// What to run when another device forgot someone: every place a person's data lives on this device.
pub type Forget = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub struct Syncer {
    forget: Option<Forget>,
    info: Mutex<Info>,
    /// something was saved since the last step
    dirty: Mutex<bool>,
    woken: Condvar,
    stopping: AtomicBool,
}

/// The database this device is connected to, and for which settings.
struct Open {
    client: Client,
    /// what the connection was opened for: another server or schema set up in Settings is not the old one still
    key: String,
    settings: Arc<Settings>,
}

impl Syncer {
    /// Something in this device's data was saved: look soon.
    pub fn note(&self) {
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.woken.notify_all();
    }

    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.woken.notify_all();
    }

    /// {state: off | joining | online | offline, text, last, error}
    pub fn status(&self) -> Value {
        let info = self.info.lock().unwrap_or_else(|e| e.into_inner()).clone();
        json!({"state": info.state, "text": info.text, "waiting": info.waiting, "last": info.last, "error": info.error})
    }

    fn say(&self, state: &'static str, text: String, error: Option<String>) {
        let mut info = self.info.lock().unwrap_or_else(|e| e.into_inner());
        let last = if state == "online" { Some(face_files::paths::utc_now()) } else { info.last.take() };
        *info = Info { state, text, last, error, waiting: 0 };
    }

    /// Start the sync for a library, on a thread of its own. `log`: told what happened, a line at a time.
    pub fn start<L: Library + Send + Sync + 'static>(
        lib: Arc<L>,
        data: &Path,
        home: &Path,
        forget: Option<Forget>,
        log: impl Fn(String) + Send + 'static,
    ) -> Arc<Syncer> {
        let syncer = Arc::new(Syncer {
            forget,
            info: Mutex::new(Info { state: "off", text: String::new(), last: None, error: None, waiting: 0 }),
            dirty: Mutex::new(true),
            woken: Condvar::new(),
            stopping: AtomicBool::new(false),
        });
        let (me, data, home) = (syncer.clone(), data.to_path_buf(), home.to_path_buf());
        let started =
            std::thread::Builder::new().name("shared database".into()).spawn(move || me.run(&*lib, &data, &home, &log));
        if let Err(e) = started {
            syncer.say("off", format!("The shared database's sync didn't start: {e}"), None);
        }
        syncer
    }

    fn run(&self, lib: &impl Library, data: &Path, home: &Path, log: &dyn Fn(String)) {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        let (mut open, mut part): (Option<Open>, Option<Home>) = (None, None);
        let state = match State::open(&home.join("sync.sqlite")) {
            Ok(s) => s,
            Err(e) => return self.say("off", format!("The shared database's sync can't keep its notes: {e}"), Some(e)),
        };
        let (mut fails, mut last_local) = (0usize, None::<Instant>);
        while !self.stopping.load(Ordering::SeqCst) {
            let dirty = std::mem::take(&mut *self.dirty.lock().unwrap_or_else(|e| e.into_inner()));
            let send = dirty || last_local.is_none_or(|t| t.elapsed() > LOCAL_EVERY);
            let began = Instant::now();
            let round = rt.block_on(self.round(&mut open, &mut part, lib, data, home, &state, send, log));
            let wait = match round {
                Ok(None) => {
                    self.say("off", "No shared database: data is kept on this device only.".to_string(), None);
                    UNSET_EVERY
                }
                Ok(Some(did)) => {
                    if send {
                        last_local = Some(began);
                    }
                    if did.dirty {
                        self.note(); // something taken is kept here its own way: sent back at the next step
                    }
                    if began.elapsed() > Duration::from_secs(2) {
                        let took = began.elapsed().as_secs_f64();
                        log(format!(
                            "Shared database: a sync step took {took:.1} s (sent {}, received {})",
                            did.pushed, did.pulled
                        ));
                    }
                    fails = 0;
                    let mut text = "Connected.".to_string();
                    if did.pushed > 0 || did.pulled > 0 {
                        text += &format!(" Sent {}, received {}.", did.pushed, did.pulled);
                    }
                    self.say("online", text, None);
                    EVERY
                }
                Err(e) => {
                    open = None;
                    // The devices go on answering each other while the server can't be reached: the peer
                    // service runs on the cluster's key as this device keeps it (the tracker's process makes
                    // it from the member list's file, `face_db::members`).
                    if crate::trackerlink::remote() {
                        let running = crate::trackerlink::peer_service(face_db::peer::sharing(home)).unwrap_or(false);
                        PEER_RUNNING.store(running, Ordering::SeqCst);
                    }
                    if dirty {
                        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true; // still to be sent
                    }
                    let first: String = e.lines().next().unwrap_or("").chars().take(200).collect();
                    if fails == 0 {
                        log(format!("Shared database: {first}"));
                    }
                    let wait = RETRY[fails.min(RETRY.len() - 1)];
                    fails += 1;
                    let text = format!(
                        "Can't reach the shared database ({first}); working from this device's copy. Changes are sent when it's back."
                    );
                    self.say("offline", text, Some(first));
                    let waiting = sync::waiting(lib, &state, part.as_ref()).unwrap_or(0);
                    self.info.lock().unwrap_or_else(|e| e.into_inner()).waiting = waiting;
                    Duration::from_secs(wait)
                }
            };
            // until the next look is due, or something is saved
            let dirty = self.dirty.lock().unwrap_or_else(|e| e.into_inner());
            if !*dirty && !self.stopping.load(Ordering::SeqCst) {
                let _ = self.woken.wait_timeout(dirty, wait);
            }
        }
    }

    /// One look: connect if needed, then a step. None: no shared database is set up.
    #[allow(clippy::too_many_arguments)]
    async fn round(
        &self,
        open: &mut Option<Open>,
        part: &mut Option<Home>,
        lib: &impl Library,
        data: &Path,
        home: &Path,
        state: &State,
        send: bool,
        log: &dyn Fn(String),
    ) -> Result<Option<sync::Did>, String> {
        let Some(settings) = Settings::load(home)? else {
            *open = None;
            return Ok(None);
        };
        let (name, os) = crate::device();
        let device = Device::load(home, &name)?;
        let key =
            format!("{}|{}|{}|{}|{}", settings.host, settings.port, settings.dbname, settings.user, settings.schema);
        if open.as_ref().is_none_or(|o| o.key != key || o.client.is_closed()) {
            let mut client = face_db::connect(&settings, &settings.schema, &device).await?;
            face_db::migrate(&mut client).await?;
            face_db::register(&client, &device, &os, env!("CARGO_PKG_VERSION")).await?;
            *open = Some(Open { client, key, settings: Arc::new(settings) });
        }
        let Some(open) = open.as_mut() else { return Ok(None) };
        let part = part.get_or_insert_with(|| Home::new(data, home, &device.id));
        if let (true, Some(forget)) = (part.forget.is_none(), self.forget.clone()) {
            part.forget = Some(Box::new(move |name| forget(name)));
        }
        let settings = open.settings.clone();
        part.password_fingerprint = Some(Box::new(move |instance| settings.password_fingerprint(instance)));
        let joined: Option<String> = (open.client.query_opt("SELECT id::text FROM instance", &[]).await)
            .map_err(|e| format!("The shared database: {e}"))?
            .map(|r| r.get(0));
        if state.get("instance")? != joined {
            self.say(
                "joining",
                "Joining the shared database: sending this device's data and taking every device's ...".to_string(),
                None,
            );
        }
        let did =
            sync::step(&mut open.client, &device.id, &device.name, lib, state, Some(part), send, &mut |line| log(line))
                .await?;
        self.peer_step(&open.client, &device, home, log).await?;
        if crate::trackerlink::remote() {
            // who this device's camera sees: sightings, and the alert for someone who moved between cameras
            // (opt-in, and only people on this device's alert list: `Home::cameras`)
            let alerts = home.join("alerts.json");
            let written = |file: &Path| std::fs::metadata(file).and_then(|m| m.modified()).ok();
            let before = written(&alerts);
            part.cameras(&open.client, &device.name, &crate::trackerlink::in_view()).await?;
            if written(&alerts) != before {
                self.note(); // an alert was raised here: sent at the next step
            }
        }
        Ok(Some(did))
    }

    /// The camera's and the peer service's part of a step, where the tracker is a process of its own (the
    /// app): its heartbeat kept going from there, the peer service on or off as this device shares, and
    /// the shared database told where the other devices reach this one (or that it doesn't share). A
    /// tracker that doesn't answer is being started again: not shared for this step, tried at the next.
    async fn peer_step(
        &self,
        client: &Client,
        device: &Device,
        home: &Path,
        log: &dyn Fn(String),
    ) -> Result<(), String> {
        use crate::trackerlink;
        if !trackerlink::remote() {
            return Ok(());
        }
        trackerlink::camera_services();
        let port = crate::peerphoto::own_port();
        let running = match trackerlink::peer_service(face_db::peer::sharing(home)) {
            Ok(running) => running,
            Err(why) => {
                let mut said = PEER_SAID.lock().unwrap_or_else(|e| e.into_inner());
                if *said != why {
                    log(format!("Couldn't share photos with the other devices (port {port}): {why}"));
                    *said = why;
                }
                false
            }
        };
        if running && !PEER_RUNNING.swap(running, Ordering::SeqCst) {
            log(format!("Sharing photos with the other devices on port {port}."));
        }
        PEER_RUNNING.store(running, Ordering::SeqCst);
        if running {
            let sha = crate::peer::cert_sha256(home, &device.name)?;
            let here: Vec<String> =
                crate::hardware::reachable_addresses(home, None).iter().map(ToString::to_string).collect();
            face_db::peer::register(client, device, &here, port, &sha).await
        } else {
            face_db::peer::unshare(client, device).await
        }
    }
}

/// why the peer service last couldn't be started, said once
static PEER_SAID: Mutex<String> = Mutex::new(String::new());
static PEER_RUNNING: AtomicBool = AtomicBool::new(false);
