//! The live tracker's server: the tracker page and the routes it calls.
//!
//! What answers for real so far: the page files, `/api/people`, `/api/recognize` (faces, names, quality,
//! expression), `/api/who`, the page's settings and person signals, `/api/page-note`, the alert inbox, the
//! presence log, the expression log's keeping side, Collect faces, posed tests and feeling reports, the
//! voice model, this device's camera for the others and theirs for this one, and joining a shared database.
//! MediaPipe's parts (face-shape cues, mesh, posture, hands) are the page's own (`pages/parts.js`):
//! this server's answers say null for them, and the page fills them in.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
    },
    time::{Instant, SystemTime},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use face_core::{
    behind::Prints,
    faces::{EXPRESSIONS, Faces, Q_ENROLL, expression_skip},
    gallery::Identifier,
    geometry::head_turn,
    image::Bgr,
    yunet::Row,
};
use serde_json::{Value, json};

use crate::{
    alerts::Inbox,
    bus::Bus,
    busrelay::Followed,
    camrelay::{Relays, camera_ids},
    camstream::Stream,
    capture::Capture,
    exprlog::{self, ExprLog},
    http::{Request, Response, Service},
    join::{self, Joining},
    peer,
    presence::{self, Presence},
    recorder::{self, Recorder},
    store::{MAX_SIGNALS, Store},
};

const NOTES_AN_HOUR: u32 = 60;
const POSE_POINTS: [&str; 13] = [
    "nose",
    "earL",
    "earR",
    "shoulderL",
    "shoulderR",
    "elbowL",
    "elbowR",
    "wristL",
    "wristR",
    "indexL",
    "indexR",
    "hipL",
    "hipR",
];

/// (path, file, content type) of the page files, in the folder the tracker is given.
pub(crate) const PAGES: [(&str, &str, &str); 15] = [
    ("/", "tracker_ui.html", "text/html; charset=utf-8"),
    ("/index.html", "tracker_ui.html", "text/html; charset=utf-8"),
    ("/cameras", "cameras_ui.html", "text/html; charset=utf-8"),
    ("/expression.js", "expression.js", "text/javascript; charset=utf-8"),
    ("/signals.js", "signals.js", "text/javascript; charset=utf-8"),
    ("/signals_chart.js", "signals_chart.js", "text/javascript; charset=utf-8"),
    ("/signals_chart.css", "signals_chart.css", "text/css; charset=utf-8"),
    ("/settings_bridge.js", "settings_bridge.js", "text/javascript; charset=utf-8"),
    ("/page_store.js", "page_store.js", "text/javascript; charset=utf-8"),
    ("/tracker_ui.js", "tracker_ui.js", "text/javascript; charset=utf-8"),
    ("/cameras_ui.js", "cameras_ui.js", "text/javascript; charset=utf-8"),
    ("/ask.js", "ask.js", "text/javascript; charset=utf-8"),
    ("/place.js", "place.js", "text/javascript; charset=utf-8"),
    ("/jpeg_worker.js", "jpeg_worker.js", "text/javascript; charset=utf-8"),
    ("/favicon.ico", "icons/icon.ico", "image/x-icon"),
];

/// MediaPipe's web runtime as the page asks for it: (path, file below `models/`, content type).
const MP_FILES: [(&str, &str, &str); 3] = [
    ("/mp/vision_bundle.mjs", "mediapipe-web/vision_bundle.mjs", "text/javascript; charset=utf-8"),
    (
        "/mp/wasm/vision_wasm_internal.js",
        "mediapipe-web/wasm/vision_wasm_internal.js",
        "text/javascript; charset=utf-8",
    ),
    ("/mp/wasm/vision_wasm_internal.wasm", "mediapipe-web/wasm/vision_wasm_internal.wasm", "application/wasm"),
];
/// MediaPipe's models, as the page's module asks for them.
const MP_MODELS: [(&str, &str); 3] = [
    ("/mp/models/face_landmarker.task", "face_landmarker.task"),
    ("/mp/models/pose_landmarker_lite.task", "pose_landmarker_lite.task"),
    ("/mp/models/hand_landmarker.task", "hand_landmarker.task"),
];
/// The page's module for MediaPipe's parts, which only this program serves.
const PARTS_SCRIPT: &str = include_str!("../pages/parts.js");
/// The worker the page runs that module in, beside itself (`pages/parts_worker.js`).
const PARTS_WORKER: &str = include_str!("../pages/parts_worker.js");

/// `v` rounded to `places` decimals: the nearest number with that many.
fn rounded(v: f64, places: usize) -> f64 {
    format!("{v:.places$}").parse().unwrap_or(v)
}

struct People {
    ident: Option<Arc<Identifier>>,
    mtime: Option<SystemTime>,
    loaded: bool,
}

pub struct Tracker {
    faces: Arc<Faces>,
    /// prints behind the frames, on a device where a print is slow to make (`behind::wanted`)
    behind: Option<Arc<Prints>>,
    people_file: PathBuf,
    people: RwLock<People>,
    warmup_ms: Vec<(&'static str, f64)>,
    pages: PathBuf,
    root: PathBuf,
    home: PathBuf,
    store: Store,
    alerts: Inbox,
    presence: Presence,
    exprlog: ExprLog,
    capture: Capture,
    /// posed tests and feeling reports: written only when the owner starts them on the page
    recorder: Recorder,
    /// the voice model, when it is there
    voice: face_core::voice::Voice,
    /// (the hour, lines the page put in the log in it)
    notes: Mutex<(u64, u32)>,
    /// frames read since the start, and (faces, hundredths of a millisecond) of the last one
    frames: AtomicU64,
    last: AtomicU64,
    /// the people as the shared database has them, for a device without a people file (`watch_shared`)
    shared: Arc<RwLock<Option<Arc<Identifier>>>>,
    /// joining a shared database, and handing its login on (the page at `/join`)
    joining: Joining,
    /// what a shared database has this device do is running: reading the people, saying it is here
    shared_on: AtomicBool,
    device_on: AtomicBool,
    /// this device's camera as its page shows it, for the owner's other devices (`peer.rs`)
    stream: Arc<Stream>,
    /// the peer service runs and knows the cluster's key: the page's frames have somewhere to go
    peer_on: Arc<AtomicBool>,
    /// the peer service's port, 0 while there is none
    peer_port: Arc<AtomicU16>,
    /// short questions to the shared database (which devices there are, where one is reached)
    kept: Arc<face_db::kept::Kept>,
    /// the other devices' cameras, for the Cameras page
    relays: Arc<Relays>,
    /// what this node publishes: what it is, what it has, what is on, its settings, its readings (bus.rs)
    bus: Arc<Bus>,
    /// copies of the other nodes' trees, for this device's pages (busrelay.rs)
    followed: Arc<Followed>,
    /// where this device is, while its owner has switched that on (location.rs)
    location: Arc<crate::location::Location>,
    /// this device's sound for the page's Signals chart, listened to only while the page asks
    #[cfg(feature = "import")]
    sound_live: Arc<crate::soundlive::Live>,
    /// what the page heard with the device's own microphone, measured (where the program can't listen itself)
    #[cfg(feature = "sound")]
    heard: crate::soundheard::Heard,
}

impl Tracker {
    pub fn new(faces: Faces, voice: face_core::voice::Voice, root: &Path, home: &Path, pages: &Path) -> Tracker {
        let warmup_ms = faces.warmup();
        let faces = Arc::new(faces);
        let behind = face_core::behind::wanted().then(|| Prints::start(faces.clone()));
        if behind.is_some() {
            println!("Prints are made behind the frames on this device, one face at a time.");
        }
        let (stream, kept, peer_port) =
            (Stream::new(), Arc::new(face_db::kept::Kept::new(home)), Arc::new(AtomicU16::new(0)));
        let relays = Relays::new(home, kept.clone(), stream.any.clone(), peer_port.clone());
        let (bus, followed) = (Arc::new(Bus::new()), Followed::new(home, peer_port.clone()));
        let t = Tracker {
            faces,
            behind,
            people_file: root.join("data").join("people.npz"),
            people: RwLock::new(People { ident: None, mtime: None, loaded: false }),
            warmup_ms,
            pages: pages.to_path_buf(),
            root: root.to_path_buf(),
            home: home.to_path_buf(),
            store: Store::new(home),
            alerts: Inbox::new(home),
            presence: Presence::new(home),
            exprlog: ExprLog::new(home),
            capture: Capture::new(home),
            recorder: Recorder::new(root),
            voice,
            notes: Mutex::new((0, 0)),
            frames: AtomicU64::new(0),
            last: AtomicU64::new(0),
            shared: Arc::new(RwLock::new(None)),
            joining: Joining::new(home),
            shared_on: AtomicBool::new(false),
            device_on: AtomicBool::new(false),
            stream,
            peer_on: Arc::new(AtomicBool::new(false)),
            peer_port,
            kept,
            relays,
            bus,
            followed,
            location: crate::location::Location::new(home),
            #[cfg(feature = "import")]
            sound_live: crate::soundlive::Live::of(root),
            #[cfg(feature = "sound")]
            heard: crate::soundheard::Heard::new(root),
        };
        let named = t.ident().map_or(0, |i| i.names.len());
        println!("Known people: {}", if named == 0 { "none yet".to_string() } else { format!("{named} named") });
        t.say_node();
        t.start_shared();
        t.location.watch(t.bus.clone());
        t
    }

    /// What this node is, as the first branch of the tree it publishes: its id, what it calls itself, and what
    /// its system says of it (the lines the Signals chart's Device section shows).
    fn say_node(&self) {
        let (name, system) = crate::device();
        let mut node = crate::devicelive::facts(&system);
        if let Ok(me) = face_db::Device::load(&self.home, &name) {
            node["id"] = json!(me.id);
            node["name"] = json!(me.name);
        }
        let _ = self.bus.set("node", node);
    }

    /// One answer of a bus to a page of this device: this node's own tree (`node=me` or none), or the copy of
    /// another node's that is kept here for as long as a page asks (never of a node its owner removed here).
    fn bus_answer(&self, req: &Request) -> Response {
        let (after, epoch) = (req.param("after"), req.param("epoch"));
        let cursor = match (after.as_deref(), epoch.as_deref()) {
            (None, None) => None,
            (after, epoch) => match crate::bus::cursor_of(after, epoch) {
                Some(c) => Some(c),
                None => return Response::error(400, "not a place in a bus"),
            },
        };
        let Some(paths) = crate::bus::prefixes(&req.param("paths").unwrap_or_default()) else {
            return Response::error(400, "not paths");
        };
        let wait = bus_wait(req);
        let node = req.param("node").unwrap_or_default();
        let me = face_db::Device::load(&self.home, &crate::device().0).map(|d| d.id).unwrap_or_default();
        if node.is_empty() || node == "me" || node == me {
            return Response::json(self.bus.since(cursor.as_ref(), &paths, wait));
        }
        // (exactly one id and nothing beside it: what a request names becomes a follower's name)
        if camera_ids(&node) != [(node.clone(), 0)] {
            return Response::error(400, "not a node");
        }
        if face_home::blocked::is(&self.home, &node) {
            return Response::json(json!({"error": "that device is removed on this one"}));
        }
        Response::json(self.followed.since(&node, cursor.as_ref(), &paths, wait))
    }

    /// One answer for every node a page follows (`?nodes=<id>[:<seq>:<epoch>],...&paths=...&wait=...`): what
    /// each has to say since the page's place in it, as soon as any has. One waiting question for a page,
    /// however many nodes: a question for each took the page's few connections from the cameras' pictures.
    fn bus_many(&self, req: &Request) -> Response {
        let Some(paths) = crate::bus::prefixes(&req.param("paths").unwrap_or_default()) else {
            return Response::error(400, "not paths");
        };
        let removed = face_home::blocked::ids(&self.home);
        let mut asked: Vec<(String, Option<crate::bus::Cursor>)> = Vec::new();
        for item in req.param("nodes").unwrap_or_default().split(',').filter(|i| !i.is_empty()) {
            let mut parts = item.splitn(3, ':');
            let id = parts.next().unwrap_or("").to_string();
            let cursor = match (parts.next(), parts.next()) {
                (None, None) => None,
                (after, epoch) => match crate::bus::cursor_of(after, epoch) {
                    Some(c) => Some(c),
                    None => return Response::error(400, "not a place in a bus"),
                },
            };
            if camera_ids(&id) != [(id.clone(), 0)] {
                return Response::error(400, "not a node");
            }
            // (a device removed on this one is asked for nothing, whatever a page names; nor one named twice)
            if !removed.contains(&id) && !asked.iter().any(|(known, _)| *known == id) {
                asked.push((id, cursor));
            }
            if asked.len() > crate::busrelay::MAX_FOLLOWED {
                return Response::error(400, "too many nodes");
            }
        }
        // (this device's own node, which the Cameras page shows too, is answered from its own bus)
        let me = face_db::Device::load(&self.home, &crate::device().0).map(|d| d.id).unwrap_or_default();
        Response::json(self.followed.since_many(&asked, &paths, bus_wait(req), Some((&me, &self.bus))))
    }

    /// The nodes this device knows of (its own member list: no server is asked), without those its owner
    /// removed here: {nodes: [{id, name, me, share}]}.
    fn bus_nodes(&self) -> Value {
        let me = face_db::Device::load(&self.home, &crate::device().0).map(|d| d.id).unwrap_or_default();
        let removed = face_home::blocked::ids(&self.home);
        let nodes: Vec<Value> = face_db::members::kept(&self.home)
            .into_iter()
            .filter(|m| m.id == me || !removed.contains(&m.id))
            .map(|m| json!({"id": m.id, "name": m.name, "me": m.id == me, "share": m.share}))
            .collect();
        json!({"nodes": nodes, "me": me})
    }

    /// What this device's own page publishes of it: the branches a page may set (`PAGE_BRANCHES`), each a
    /// path and its value. What the server itself says of the node (`node`) is not a page's to set.
    fn bus_publish(&self, body: &Value) -> Result<Value, String> {
        let set = body.get("set").and_then(Value::as_object).ok_or("expected {set: {path: value}}")?;
        if set.len() > 64 {
            return Err("too many paths at once".to_string());
        }
        let mut changed = 0;
        for (path, value) in set {
            let branch = path.split('/').next().unwrap_or("");
            if !PAGE_BRANCHES.contains(&branch) {
                return Err(format!("a page doesn't publish {branch:?}"));
            }
            // (readings are true for a moment: said to whoever listens, never kept in the tree)
            if branch == "readings" {
                self.bus.say(path, value.clone())?;
            } else {
                changed += usize::from(self.bus.set(path, value.clone())?);
            }
        }
        // (which of a page's branches this node holds: a page whose server began again sends them anew)
        let has: Vec<&str> = PAGE_BRANCHES.iter().copied().filter(|b| self.bus.get(b).is_some()).collect();
        Ok(json!({"ok": true, "changed": changed, "seq": self.bus.cursor().seq, "has": has}))
    }

    /// What a device with a shared database does, started with the program and again when the device joins
    /// one (each once): the people are read from it where there is no people file, and the device is noted
    /// in it and says every few seconds that it is here (`registers`).
    fn start_shared(&self) {
        if matches!(face_db::Settings::load(&self.home), Ok(None)) {
            return;
        }
        if !self.people_file.is_file() && !self.shared_on.swap(true, Ordering::SeqCst) {
            watch_shared(
                self.shared.clone(),
                self.home.clone(),
                self.faces.rec_model.name,
                self.faces.rec_model.dim,
                self.peer_port.clone(),
            );
        }
        if registers() && !self.device_on.swap(true, Ordering::SeqCst) {
            let (name, os) = crate::device();
            watch_device(
                self.home.clone(),
                name,
                os,
                self.stream.clone(),
                self.peer_on.clone(),
                self.peer_port.clone(),
                self.bus.clone(),
            );
        }
    }

    /// (frames read since the start, faces in the last one, the milliseconds it took): numbers for a log line.
    pub fn progress(&self) -> (u64, u64, f64) {
        let last = self.last.load(Ordering::Relaxed);
        (self.frames.load(Ordering::Relaxed), last >> 32, (last & 0xffff_ffff) as f64 / 100.0)
    }

    pub fn devices(&self) -> Vec<(&'static str, Option<String>)> {
        self.faces.devices()
    }

    /// Everyone named, read again whenever the people file changed (the labeling side rewrites it whole). A
    /// device without a people file of its own (a phone: nothing is labeled there) takes the people from the
    /// shared database, when it has one.
    fn ident(&self) -> Option<Arc<Identifier>> {
        let mtime = std::fs::metadata(&self.people_file).and_then(|m| m.modified()).ok();
        if mtime.is_none() {
            if let Some(shared) = self.shared.read().unwrap_or_else(|e| e.into_inner()).clone() {
                return Some(shared);
            }
        }
        {
            let p = self.people.read().unwrap_or_else(|e| e.into_inner());
            if p.loaded && p.mtime == mtime {
                return p.ident.clone();
            }
        }
        let mut p = self.people.write().unwrap_or_else(|e| e.into_inner());
        if p.loaded && p.mtime == mtime {
            return p.ident.clone();
        }
        match Identifier::load(&self.people_file, self.faces.rec_model.name) {
            Ok(ident) => {
                // only once it's read, so a file being replaced right now is tried again at the next request
                *p = People { ident: ident.map(Arc::new), mtime, loaded: true };
            }
            Err(e) => eprintln!("Couldn't read people.npz yet ({e}); trying again."),
        }
        p.ident.clone()
    }

    fn file(&self, name: &str) -> Option<Value> {
        serde_json::from_slice(&std::fs::read(self.root.join("data").join(name)).ok()?).ok()
    }

    fn people_answer(&self) -> Value {
        let ident = self.ident();
        let mut devices = serde_json::Map::new();
        for (job, device) in self.faces.devices() {
            devices.insert(job.to_string(), json!(device));
        }
        for job in ["cues", "pose", "hands"] {
            devices.insert(job.to_string(), Value::Null);
        }
        devices
            .insert("voice".to_string(), if self.voice.loaded() { json!("CPU (ONNX Runtime)") } else { Value::Null });
        let warmup: serde_json::Map<String, Value> =
            self.warmup_ms.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        json!({
            "people": ident.map(|i| i.names.clone()).unwrap_or_default(),
            "threshold": self.faces.rec_model.match_at,
            "model": self.faces.rec_model.name,
            "device": devices.get("recognition").cloned().unwrap_or(Value::Null),
            "devices": devices,
            "warmup_ms": warmup,
            "expressions": if self.faces.has_expression() { json!(EXPRESSIONS) } else { Value::Null },
            "cues": false,
            "voice": self.voice.loaded(),
            "meshTriangles": Value::Null,
            "qEnroll": rounded(Q_ENROLL as f64, 4),
            "captureDir": self.home.join("webcam").to_string_lossy(),
            "pose": false,
            "posePoints": POSE_POINTS,
            "hands": false,
            "calibration": self.file("expr_calibration.json"),
            "vaFit": self.file("va_fit.json"),
            "sessions": self.recorder.sessions().len(),
            "reports": self.recorder.reports(),
            "forgotten": self.store.forgotten(),
            // where this server's page for joining a shared database is
            "join": "/join",
            // (the page words its way there by it: a device of a database lets others in, another joins)
            "joined": self.joining.joined(),
            // MediaPipe's parts run in the page here: where its module, runtime and models are
            "parts": self.parts(),
            // a test device that is to send every signal it has (FACE_SIGNALS=all in the app's environment):
            // its page then keeps every option that makes readings switched on
            "signals": std::env::var("FACE_SIGNALS").ok().filter(|all| all == "all"),
            // the way this camera's frames leave the page, named to compare the ways on one device
            // (FACE_STREAM: transfer, bitmap or page); without it the page takes its engine's own
            "stream": stream_way(),
        })
    }

    /// What the page needs to run MediaPipe's parts itself (`pages/parts.js`), or null without the web
    /// runtime (`models/mediapipe-web`, fetched by `face-tools get mediapipe` or packed with an app): where
    /// the module is, and which of the three model files are there.
    fn parts(&self) -> Value {
        let models = self.root.join("models");
        if !MP_FILES.iter().all(|(_, file, _)| models.join(file).is_file()) {
            return Value::Null;
        }
        let has = |name: &str| models.join(name).is_file();
        // on the processor, where MediaPipe in a page gives the answers the app was measured with; FACE_PARTS=gpu tries
        // the graphics processor, which is faster and gives others (pages/parts.js)
        let asked = std::env::var("FACE_PARTS").unwrap_or_default();
        let delegate = if asked == "gpu" { "GPU" } else { "CPU" };
        // costly: on a phone the page's own processor time is what a frame waits for, and a face's cues
        // take a tenth of a second of it (measured: 98 ms in the model on a Pixel 10 Pro XL, 80 ms a frame on
        // an iPhone 12 Pro Max). The page then leaves what asks for them with every frame off until the owner
        // switches it on. A phone's own flow: nothing changes on a computer.
        let costly = cfg!(any(target_os = "android", target_os = "ios"));
        // Beside the page, in workers: one for each model on a computer, one for all three on a phone (each
        // worker is the whole runtime again in memory). FACE_PARTS=page keeps them in the page itself and
        // FACE_PARTS=one gives a computer the phone's one worker, to compare the three ways.
        let beside = match asked.as_str() {
            "page" => 0,
            "one" => 1,
            _ if costly || face_core::behind::raspberry_pi() => 1,
            _ => 3,
        };
        json!({"script": "/mp/parts.js", "base": "/mp", "cues": has("face_landmarker.task"),
               "pose": has("pose_landmarker_lite.task"), "hands": has("hand_landmarker.task"), "delegate": delegate,
               "costly": costly, "beside": beside})
    }

    /// (see `stream_way` below for how this camera's frames leave the page)
    /// One of the two scripts of MediaPipe's parts: the program's own copy, or where a folder of pages was
    /// named (`--pages`, a developer's) and has the file, that one, so that an edit shows without a build.
    fn parts_script(&self, name: &str, carried: &str) -> Response {
        let named = (!self.pages.as_os_str().is_empty()).then(|| std::fs::read(self.pages.join(name)).ok()).flatten();
        Response::bytes("text/javascript; charset=utf-8", named.unwrap_or_else(|| carried.as_bytes().to_vec()))
    }

    /// {devices: [{id, name, me, share, streaming, streamingAt}], why}: every device of the shared database,
    /// and whether its camera streams now.
    /// What this device is called in the shared database: {name, own, numbered}, the last when the name is only
    /// the number the app gave it because another device goes by its own name already (its page then asks
    /// whoever is at the device for a name). {} while the database doesn't know the device yet, or can't be asked.
    fn own_name(&self) -> Value {
        let asked = face_db::Device::load(&self.home, &crate::device().0)
            .and_then(|me| self.kept.ask(async |client, _| face_db::device_names(client, &me.id).await));
        match asked {
            Ok(Some((name, own, numbered))) => json!({"name": name, "own": own, "numbered": numbered}),
            _ => json!({}),
        }
    }

    /// The cameras a page asks for, without those of a device its owner removed on this one: nothing is
    /// asked of such a device, whatever a page names.
    fn cameras_asked(&self, text: &str) -> Vec<(String, u64)> {
        let removed = face_home::blocked::ids(&self.home);
        camera_ids(text).into_iter().filter(|(id, _)| !removed.contains(id)).collect()
    }

    fn cameras(&self) -> Value {
        let listed = self.relays.me().and_then(|me| {
            let all = self.kept.ask(async |client, _| face_db::kept::cameras(client).await)?;
            // a device its owner removed on this one is not shown here, and its camera is not asked for
            let removed = face_home::blocked::ids(&self.home);
            Ok(all
                .into_iter()
                .filter(|d| d.id == me || !removed.contains(&d.id))
                .map(|d| {
                    json!({"id": d.id, "name": d.name, "me": d.id == me, "share": d.share, "streaming": d.streaming,
                           "streamingAt": d.streaming_at})
                })
                .collect::<Vec<Value>>())
        });
        match listed {
            Ok(mut devices) => {
                self.places(&mut devices, true);
                json!({"devices": devices, "why": ""})
            }
            Err(e) => {
                // The shared database can't be asked: the devices are the members this device keeps a list of
                // itself. Whether a camera streams right now only its own device says (asked when its tile
                // asks for its frames), so every member that shares is listed as one that may.
                let (me, removed) = (self.relays.me().unwrap_or_default(), face_home::blocked::ids(&self.home));
                let mut devices: Vec<Value> = face_db::members::kept(&self.home)
                    .into_iter()
                    .filter(|m| m.id == me || !removed.contains(&m.id))
                    .map(|m| {
                        json!({"id": m.id, "name": m.name, "me": m.id == me, "share": m.share, "streaming": m.share,
                               "streamingAt": Value::Null})
                    })
                    .collect();
                self.places(&mut devices, false);
                if devices.is_empty() {
                    let e: String = e.chars().take(120).collect();
                    json!({"devices": [], "why": format!("No shared database to find the other devices ({e})")})
                } else {
                    json!({"devices": devices, "why": "", "from": "members"})
                }
            }
        }
    }

    /// Where each listed device is, put beside it as `place`, for the devices whose owner switched their
    /// location on: this device's own; another's as its node publishes it, while a page of this device
    /// follows that node; else the newest row of its history, where the shared database can be asked. A
    /// place is numbers and one word, checked here whoever said them, and the town is named from this
    /// device's own list. `live` says the device itself said it (its `at` is when); a row's `at` is when
    /// the device came to be there.
    fn places(&self, devices: &mut [Value], ask_database: bool) {
        use face_home::location::{Fix, Source};
        let shown = |fix: Fix, live: bool| {
            let mut place = fix.json();
            place["town"] = json!(crate::location::town_at(fix.lat, fix.lon));
            place["live"] = json!(live);
            place
        };
        let mut kept: Option<Vec<(String, face_db::location::Row)>> = None;
        for d in devices.iter_mut() {
            let Some(id) = d.get("id").and_then(Value::as_str).map(str::to_string) else { continue };
            let place = if d["me"] == true {
                self.location.answer().get("place").cloned().filter(|p| !p.is_null()).map(|mut p| {
                    p["live"] = json!(true);
                    p
                })
            } else if let Some(fix) = self.followed.value(&id, "place").as_ref().and_then(Fix::from_json) {
                Some(shown(fix, true))
            } else if ask_database {
                // (asked once for all of them; a database that has no history yet has none)
                let rows = kept.get_or_insert_with(|| {
                    self.kept.ask(async |client, _| face_db::location::latest(client).await).unwrap_or_default()
                });
                rows.iter().find(|(device, _)| *device == id).and_then(|(_, r)| {
                    let fix = Fix::of(r.lat, r.lon, r.accuracy, r.at, Source::of(&r.source)?)?;
                    Some(shown(fix, false))
                })
            } else {
                None
            };
            if let Some(place) = place {
                d["place"] = place;
            }
        }
    }

    /// The faces of one webcam frame.
    fn recognize(&self, jpeg: &[u8], threshold: f64, record: Option<&Posed>) -> Result<Value, String> {
        let img = Bgr::from_jpeg(jpeg).map_err(|e| e.0)?;
        let t0 = Instant::now();
        let ident = self.ident(); // one snapshot: a reload mid-request can't swap it
        // one pass: webcam faces are large. Where prints are made behind the frames, a face comes without
        // one in a frame that brought no new one.
        let found: Vec<(Row, Option<Vec<f32>>, Bgr, f32)> = match &self.behind {
            Some(prints) => {
                prints.find(&img).map_err(|e| e.0)?.into_iter().map(|s| (s.row, s.emb, s.crop, s.quality)).collect()
            }
            None => {
                let all = self.faces.detect(&img, &[640]).map_err(|e| e.0)?;
                all.into_iter().map(|f| (f.row, Some(f.emb), f.crop, f.quality)).collect()
            }
        };
        let mut out = Vec::with_capacity(found.len());
        for (k, (row, emb, crop, quality)) in found.iter().enumerate() {
            let f = Shown { row, crop, quality: *quality };
            // expression only from faces the model can read: sharp enough, and facing the camera
            let skip = expression_skip(f.row, f.quality);
            let expr = if skip.is_none() { self.faces.expression(f.crop).map_err(|e| e.0)? } else { None };
            let (name, sim) = match (emb, &ident, &self.behind) {
                // no new print in this frame: the name its last print gave this face
                (None, _, Some(prints)) => prints.named(k),
                (Some(e), Some(i), _) => i.who(e, threshold),
                _ => ("Unknown".to_string(), 0.0),
            };
            if let (Some(_), Some(prints)) = (emb, &self.behind) {
                prints.name(k, &name, sim);
            }
            let emb = emb.as_ref().map(|e| B64.encode(e.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()));
            let r = |v: f32, places| rounded(v as f64, places);
            out.push(json!({
                "box": [r(f.row[0], 1), r(f.row[1], 1), r(f.row[2], 1), r(f.row[3], 1)],
                "kps": (0..5).map(|k| [r(f.row[4 + 2 * k], 1), r(f.row[5 + 2 * k], 1)]).collect::<Vec<_>>(),
                "score": r(f.row[14], 3),
                "name": name,
                "sim": r(sim, 3),
                // the face print: the page averages a face's recent prints and asks /api/who
                "emb": emb,
                "quality": if f.quality.is_nan() { Value::Null } else { json!(r(f.quality, 3)) },
                "turn": rounded(head_turn(f.row), 2),
                "light": rounded(f.crop.mean() / 255.0, 2),
                "expr": expr.map(|e| e.iter().map(|v| rounded(*v, 3)).collect::<Vec<_>>()),
                "xskip": if self.faces.has_expression() { json!(skip) } else { Value::Null },
                "cues": Value::Null, "mesh": Value::Null, "pose": Value::Null, "hand": Value::Null,
                "body": Value::Null, "hands": Value::Null,
            }));
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.last.store(((out.len() as u64) << 32) | ((ms * 100.0) as u64).min(0xffff_ffff), Ordering::Relaxed);
        if let Some(posed) = record {
            // a posed test: the largest readable face's numbers go to the test's file (the first of equals)
            let area = |f: &Value| f["box"][2].as_f64().unwrap_or(0.0) * f["box"][3].as_f64().unwrap_or(0.0);
            let mut largest: Option<&Value> = None;
            for f in out.iter().filter(|f| !f["expr"].is_null()) {
                if largest.is_none_or(|l| area(f) > area(l)) {
                    largest = Some(f);
                }
            }
            if let Some(f) = largest {
                self.recorder.add(&posed.session, &posed.label, &posed.extra, f)?;
            }
        }
        Ok(json!({"faces": out, "ms": rounded(ms, 1), "width": img.w, "height": img.h}))
    }

    /// [name, similarity] for each face print: the page sends each face's recent prints averaged.
    fn who(&self, body: &Value) -> Result<Value, String> {
        let dim = self.faces.rec_model.dim;
        let mut prints = Vec::new();
        for p in body.get("prints").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().take(20) {
            let raw = p.as_str().and_then(|s| B64.decode(s).ok()).ok_or("bad print")?;
            if raw.len() != dim * 4 {
                return Err(format!("each print must have {dim} numbers"));
            }
            prints.push(raw.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<f32>>());
        }
        let threshold = match body.get("t") {
            None => self.faces.rec_model.match_at,
            Some(t) => t.as_f64().or_else(|| t.as_str().and_then(|s| s.parse().ok())).ok_or("bad threshold")?,
        }
        .clamp(0.1, 0.9);
        let ident = self.ident();
        let who: Vec<Value> = prints
            .into_iter()
            .map(|mut e| {
                let norm = e.iter().map(|v| v * v).sum::<f32>().sqrt() + 1e-9;
                e.iter_mut().for_each(|v| *v /= norm);
                match &ident {
                    Some(i) => {
                        let (name, sim) = i.who(&e, threshold);
                        json!([name, rounded(sim as f64, 3)])
                    }
                    None => json!(["Unknown", 0.0]),
                }
            })
            .collect();
        Ok(json!({"who": who}))
    }

    /// A line in the log from the tracker page (what it does about a camera that won't run): plain text
    /// only, 200 characters, and at most NOTES_AN_HOUR lines an hour. Whether it was logged.
    fn page_note(&self, text: Option<&str>) -> bool {
        let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else { return false };
        let hour = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 3600);
        {
            let mut n = self.notes.lock().unwrap_or_else(|e| e.into_inner());
            if n.0 != hour {
                *n = (hour, 0);
            }
            n.1 += 1;
            if n.1 > NOTES_AN_HOUR {
                return false;
            }
        }
        let plain: String = text.chars().map(|c| if (' '..='~').contains(&c) { c } else { '?' }).take(200).collect();
        println!("Tracker page: {plain}");
        true
    }
}

/// seconds between two looks at the shared database for the people
const SHARED_EVERY: u64 = 60;

/// the longest a page's question to a bus waits for news, in seconds (the page then asks again)
const BUS_WAIT: f64 = 25.0;
/// The branches of this node's tree its own page publishes: every source and signal there is (the Signals
/// chart's sections), what is on and what is not, every setting of the page, and the readings as they are
/// taken (a few times a second, each batch only what is new, every reading with its own time).
const PAGE_BRANCHES: [&str; 4] = ["catalog", "state", "settings", "readings"];

/// On a thread of its own: the people of the shared database, read when the program starts and again every
/// `SHARED_EVERY` seconds, for a device that has a shared database and no people file. Reading is all it
/// does there (`face_db::connect_read_only`). The log gets a line when the people changed, with counts only.
fn watch_shared(
    into: Arc<RwLock<Option<Arc<Identifier>>>>,
    home: PathBuf,
    rec_name: &'static str,
    rec_dim: usize,
    peer_port: Arc<AtomicU16>,
) {
    let _ = std::thread::Builder::new().name("people of the shared database".into()).spawn(move || {
        let (mut had, mut said) = ((0usize, 0usize), String::new());
        // the SHA-256 of the people file another device gave, while that is what this device goes by
        let mut held: Option<String> = None;
        loop {
            // From another device of the owner's first, directly: one with a library of its own gives its
            // people file as it is (peer.rs /people), so a device without a library names people while the
            // shared database can't be reached. The database is asked only when no device gave one.
            // (a file this device can't use, made for another recognition model, is no answer: the next
            // member is asked, and the file is read as what another device gave, nothing taken on trust)
            let usable = |bytes: &[u8]| matches!(Identifier::from_foreign_npz(bytes, rec_name), Ok(Some(_)));
            match crate::busrelay::people(&home, peer_port.load(Ordering::Relaxed), held.as_deref(), &usable) {
                Ok(crate::busrelay::People::Same) => {
                    std::thread::sleep(std::time::Duration::from_secs(SHARED_EVERY));
                    continue;
                }
                Ok(crate::busrelay::People::New(bytes, sha)) => {
                    if let Ok(Some(ident)) = Identifier::from_foreign_npz(&bytes, rec_name) {
                        let now = ident.counts();
                        if now != had || held.is_none() {
                            println!("Known people: {} named, from {} prints of another device of yours", now.0, now.1);
                            had = now;
                        }
                        *into.write().unwrap_or_else(|e| e.into_inner()) = (now.0 > 0).then(|| Arc::new(ident));
                        held = Some(sha);
                        said.clear();
                        std::thread::sleep(std::time::Duration::from_secs(SHARED_EVERY));
                        continue;
                    }
                    held = None; // (made with another recognition model, or nobody named in it)
                }
                Err(_) => held = None,
            }
            match face_db::people_now(&home, rec_name) {
                Ok(None) => return, // no shared database on this device
                Ok(Some((names, emb, dim))) if dim == rec_dim || names.is_empty() => {
                    match Identifier::new(&emb, &names, rec_dim) {
                        Ok(ident) => {
                            let now = (ident.names.len(), names.len());
                            if now != had {
                                println!("Known people: {} named, from {} prints of the shared database", now.0, now.1);
                                had = now;
                            }
                            *into.write().unwrap_or_else(|e| e.into_inner()) = (now.0 > 0).then(|| Arc::new(ident));
                            said.clear();
                        }
                        Err(e) => eprintln!("The shared database's people can't be used: {e}"),
                    }
                }
                Ok(Some(_)) => eprintln!("The shared database's prints are of another recognition model."),
                Err(e) if e != said => {
                    eprintln!("{e}"); // once, not every minute: a phone away from home has no way to the server
                    said = e;
                }
                Err(_) => {}
            }
            std::thread::sleep(std::time::Duration::from_secs(SHARED_EVERY));
        }
    });
}

/// Whether this program notes its device in the shared database. A phone does: this program is the whole
/// app there. On a computer the app's sync notes the device and keeps its row, so there only when asked
/// (`FACE_REGISTER=on`, to try it).
fn registers() -> bool {
    cfg!(any(target_os = "android", target_os = "ios")) || std::env::var("FACE_REGISTER").is_ok_and(|v| v == "on")
}

/// seconds between two "this device is here"
const BEAT_EVERY: u64 = 3;
/// seconds before the server is tried again after it couldn't be reached
const BEAT_AGAIN: u64 = 30;

/// seconds between two times this device says where it can be reached (its addresses change with the network)
const WHERE_EVERY: u64 = 60;

/// Whether this device shows its camera to the owner's other devices (`FACE_PEER=off` turns it off).
fn shares() -> bool {
    !std::env::var("FACE_PEER").is_ok_and(|v| v == "off")
}

/// On a thread of its own, with a connection of its own: this device's row in the shared database (its
/// name, system and version, noted once per connection) and then, every `BEAT_EVERY` seconds, that it is
/// here and whether its camera streams. Where it shares (`shares`), the peer service is started once, the
/// cluster's key read, and the row told where the other devices reach this one. The schema is left as it
/// is: a phone never brings it up to date.
fn watch_device(
    home: PathBuf,
    name: String,
    os: String,
    stream: Arc<Stream>,
    peer_on: Arc<AtomicBool>,
    peer_port: Arc<AtomicU16>,
    bus: Arc<Bus>,
) {
    let _ = std::thread::Builder::new().name("this device in the shared database".into()).spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        let (mut said, mut noted) = (String::new(), false);
        let key: peer::Key = Arc::new(RwLock::new(None));
        let mut service: Option<peer::Service> = None;
        loop {
            // Without the server: the service runs on the cluster's key as this device keeps it (made when
            // the server last answered), so the nodes go on answering each other while it can't be reached.
            // Not after the server refused this device's login: then the key is taken away below.
            if shares() && service.is_none() && !REFUSED.load(Ordering::SeqCst) {
                if let (Some(kept), Ok(device)) =
                    (face_db::members::kept_key(&home), face_db::Device::load(&home, &name))
                {
                    *key.write().unwrap_or_else(|e| e.into_inner()) = Some(kept);
                    if let Ok(s) = peer::start(
                        &home,
                        &device.name,
                        stream.clone(),
                        key.clone(),
                        None,
                        peer::Gives::default(),
                        bus.clone(),
                    ) {
                        println!(
                            "Peer service: this device's camera and bus for the other devices, on port {}.",
                            s.port
                        );
                        peer_port.store(s.port, Ordering::Relaxed);
                        peer_on.store(true, Ordering::SeqCst);
                        service = Some(s);
                    }
                }
            }
            let ended: Result<(), String> = rt.block_on(async {
                let Some(settings) = face_db::Settings::load(&home)? else { return Ok(()) };
                let device = face_db::Device::load(&home, &name)?;
                let mut client = face_db::connect(&settings, &settings.schema, &device).await?;
                // (a phone is brought up to date by nothing else: its statements read what the newest
                // migration added, a device's alias)
                face_db::migrate(&mut client).await?;
                face_db::register(&client, &device, &os, env!("CARGO_PKG_VERSION")).await?;
                if !noted {
                    println!("This device is noted in the shared database.");
                    noted = true;
                }
                if shares() && service.is_none() {
                    match peer::start(
                        &home,
                        &device.name,
                        stream.clone(),
                        key.clone(),
                        None,
                        peer::Gives::default(),
                        bus.clone(),
                    ) {
                        Ok(s) => {
                            println!("Peer service: this device's camera for the other devices, on port {}.", s.port);
                            peer_port.store(s.port, Ordering::Relaxed);
                            service = Some(s);
                        }
                        Err(e) => eprintln!("The peer service didn't start: {e}"),
                    }
                }
                if !shares() {
                    face_db::peer::unshare(&client, &device).await?;
                }
                let mut told: Option<Instant> = None;
                let mut history = crate::location::Sending::default();
                loop {
                    if let Some(s) =
                        service.as_ref().filter(|_| told.is_none_or(|t| t.elapsed().as_secs() >= WHERE_EVERY))
                    {
                        // (the members with it, kept on this device: what the nodes find and check each
                        // other by when this server can't be reached)
                        *key.write().unwrap_or_else(|e| e.into_inner()) =
                            Some(face_db::members::refresh(&client, &settings, &home).await?);
                        REFUSED.store(false, Ordering::SeqCst);
                        let here: Vec<String> = face_db::join::own_addresses((&settings.host, settings.port))
                            .iter()
                            .map(|a| a.to_string())
                            .collect();
                        face_db::peer::register(&client, &device, &here, s.port, &s.cert_sha256).await?;
                        peer_on.store(true, Ordering::SeqCst);
                        told = Some(Instant::now());
                    }
                    face_db::heartbeat(&client, &device, peer_on.load(Ordering::SeqCst) && stream.streaming()).await?;
                    // (the location history's waiting rows go with it: a phone has no other way to the database)
                    crate::location::send(&client, &device.id, &home, &mut history).await;
                    tokio::time::sleep(std::time::Duration::from_secs(BEAT_EVERY)).await;
                }
            });
            // (the server answered and refused this device's login: it answers no other device on the
            // strength of the key it kept; a login that works again gives the key back)
            match &ended {
                Err(e) if face_db::login_refused(e) => {
                    REFUSED.store(true, Ordering::SeqCst);
                    *key.write().unwrap_or_else(|e| e.into_inner()) = None;
                    peer_on.store(false, Ordering::SeqCst);
                }
                _ => {}
            }
            match ended {
                Ok(()) => return, // no shared database on this device any more
                Err(e) if e != said => {
                    eprintln!("{e}"); // once: a phone away from home has no way to the server
                    said = e;
                }
                Err(_) => {}
            }
            noted = false;
            std::thread::sleep(std::time::Duration::from_secs(BEAT_AGAIN));
        }
    });
}

/// A posed test's frame: which test, the expression asked for, and what else the page says of the moment.
struct Posed {
    session: String,
    label: String,
    extra: serde_json::Map<String, Value>,
}

impl Posed {
    /// From a request's rec=<label>&session=<id>[&talk=1][&hand=1][&tv=<talk score>][&vad=0|1]; None when it
    /// is no test's frame.
    fn of(req: &Request) -> Result<Option<Posed>, String> {
        let Some(label) = req.param("rec") else { return Ok(None) };
        let session = req.param("session").unwrap_or_default();
        if !EXPRESSIONS.contains(&label.as_str()) || !recorder::is_session(&session) {
            return Err("bad test-session parameters".to_string());
        }
        let is = |key: &str, value: &str| req.param(key).as_deref() == Some(value);
        let mut extra = serde_json::Map::new();
        extra.insert("talk".to_string(), json!(is("talk", "1")));
        extra.insert("handAsked".to_string(), json!(is("hand", "1")));
        if let Some(tv) = req.param("tv") {
            let score = tv.trim().parse::<f64>().map_err(|_| format!("could not convert string to float: '{tv}'"))?;
            extra.insert("tv".to_string(), json!(rounded(score.clamp(0.0, 1.0), 4)));
        }
        if is("vad", "0") || is("vad", "1") {
            extra.insert("vad".to_string(), json!(is("vad", "1")));
        }
        Ok(Some(Posed { session, label, extra }))
    }
}

/// One face of a frame, as the answer is made from it.
struct Shown<'a> {
    row: &'a Row,
    crop: &'a Bgr,
    quality: f32,
}

fn parsed(body: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))
}

const NOT_YET: &str = "The Rust tracker doesn't do this yet.";

/// the shared database answered and refused this device's login: the key it kept counts for nothing until a
/// login works again
static REFUSED: AtomicBool = AtomicBool::new(false);

/// How this camera's frames for the other devices leave the tracker page, where the page is not to choose by
/// itself: "transfer" (the page's worker makes and sends them) or "page" (the page does). An iPhone keeps the
/// page's way: measured on an iPhone 12 Pro Max with every signal on (2026-11-24), the worker's way gave about a
/// tenth more frames and cost the checks, up to half of them with a face in view (the phone's processor is full
/// either way, and the worker's JPEGs come out of the same cores). A phone's own flow: nothing changes on a
/// computer, where the worker's way is the page's own choice. FACE_STREAM names either, to compare them.
fn stream_way() -> Option<String> {
    let asked = std::env::var("FACE_STREAM").ok().filter(|way| way == "transfer" || way == "page");
    asked.or_else(|| cfg!(target_os = "ios").then(|| "page".to_string()))
}

/// How long a page's question to a bus waits for news: what the request says, as far as it is a time.
fn bus_wait(req: &Request) -> std::time::Duration {
    let asked = req.param("wait").and_then(|w| w.parse::<f64>().ok()).filter(|w| w.is_finite()).unwrap_or(0.0);
    std::time::Duration::from_secs_f64(asked.clamp(0.0, BUS_WAIT))
}

/// The peer service this process runs at the sync's word (the app's other process), and the cluster's key
/// as it was last read from the shared database.
struct LinkPeer {
    service: Option<peer::Service>,
    key: peer::Key,
    key_at: Option<Instant>,
}

static LINK_PEER: Mutex<Option<LinkPeer>> = Mutex::new(None);
/// the camera's heartbeat runs (started once, at the sync's first word)
static BEATING: AtomicBool = AtomicBool::new(false);

impl Tracker {
    /// The peer service started or stopped here at the sync's word: it runs where the camera's frames are.
    /// Photos another device asks for are fetched from the labeling tool (`trackerlink::photo`); the
    /// camera's stream and readout are this process's own.
    fn local_peer(&self, req: &Request) -> Response {
        use crate::trackerlink;
        if !trackerlink::allowed(&req.headers) {
            return Response::error(403, trackerlink::REFUSAL);
        }
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        if let Some(label) = body["label"].as_str() {
            trackerlink::set_label(label);
        }
        let run = body["run"] == true;
        let mut held = LINK_PEER.lock().unwrap_or_else(|e| e.into_inner());
        let st = held.get_or_insert_with(|| LinkPeer { service: None, key: Arc::new(RwLock::new(None)), key_at: None });
        // The key the other devices sign their requests with. Until the database answered it is the key as
        // this device keeps it, so the devices go on finding and checking each other while the server is away.
        // A server that answers and REFUSES this device's login takes the key away: a device that was shut
        // out by a new password must not go on answering on the strength of what it kept.
        let refused = REFUSED.load(Ordering::SeqCst);
        if st.key.read().unwrap_or_else(|e| e.into_inner()).is_none() && !refused {
            if let Some(kept) = face_db::members::kept_key(&self.home) {
                *st.key.write().unwrap_or_else(|e| e.into_inner()) = Some(kept);
            }
        }
        // The database is asked for the key and the members at most every 30 s, on a thread of its own: the
        // app's other process waits a few seconds for this answer, and a server that can't be reached takes
        // longer than that to say so.
        if st.key_at.is_none_or(|t| t.elapsed().as_secs() > 30) {
            st.key_at = Some(Instant::now());
            let (home, key) = (self.home.clone(), st.key.clone());
            let _ = std::thread::Builder::new().name("members".into()).spawn(move || {
                let kept = face_db::kept::Kept::new(&home);
                match kept.ask(async |client, settings| face_db::members::refresh(client, settings, &home).await) {
                    Ok(fresh) => {
                        *key.write().unwrap_or_else(|e| e.into_inner()) = Some(fresh);
                        REFUSED.store(false, Ordering::SeqCst);
                    }
                    Err(e) if face_db::login_refused(&e) => {
                        *key.write().unwrap_or_else(|e| e.into_inner()) = None;
                        if !REFUSED.swap(true, Ordering::SeqCst) {
                            println!(
                                "The shared database refused this device's login: it answers no other device until it is let in again."
                            );
                        }
                    }
                    Err(_) => {} // (can't be reached: the key as this device keeps it stands)
                }
            });
        }
        if run && st.service.is_none() {
            if st.key.read().unwrap_or_else(|e| e.into_inner()).is_none() {
                return Response::json(json!({"running": false})); // no word from the database yet: at the sync's next step
            }
            let photos: peer::Provider = Arc::new(trackerlink::photo);
            // this device's people, for a device of the cluster that has no library of its own (a phone)
            let people = peer::people_from(self.people_file.clone());
            let gives = peer::Gives { photos: Some(photos), people: Some(people) };
            let name = crate::device().0;
            let name = face_db::Device::load(&self.home, &name).map_or(name, |device| device.name);
            let at = Some(crate::peerphoto::own_port());
            match peer::start(&self.home, &name, self.stream.clone(), st.key.clone(), at, gives, self.bus.clone()) {
                Ok(service) => {
                    self.peer_port.store(service.port, Ordering::Relaxed);
                    st.service = Some(service);
                }
                Err(why) => return Response::json(json!({"running": false, "error": why})),
            }
        } else if !run && let Some(service) = st.service.take() {
            service.stop();
        }
        let running = st.service.is_some();
        self.peer_on.store(running, Ordering::SeqCst);
        Response::json(json!({"running": running}))
    }

    /// Tell the shared database every few seconds that this device is here and whether its camera streams,
    /// on a thread and connection of its own: a long step of the sync must not make the camera look gone to
    /// the other devices. Started once; where this program is the whole device (a phone), the device's own
    /// watch does it.
    fn start_heartbeat(&self) {
        if registers() || BEATING.swap(true, Ordering::SeqCst) {
            return;
        }
        let (home, stream, peer_on) = (self.home.clone(), self.stream.clone(), self.peer_on.clone());
        let started = std::thread::Builder::new().name("camera heartbeat".into()).spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            loop {
                let ended: Result<(), String> = rt.block_on(async {
                    let Some(settings) = face_db::Settings::load(&home)? else { return Ok(()) };
                    let device = face_db::Device::load(&home, &crate::device().0)?;
                    let client = face_db::connect(&settings, &settings.schema, &device).await?;
                    let mut history = crate::location::Sending::default();
                    loop {
                        let streams = peer_on.load(Ordering::SeqCst) && stream.streaming();
                        face_db::heartbeat(&client, &device, streams).await?;
                        crate::location::send(&client, &device.id, &home, &mut history).await;
                        tokio::time::sleep(std::time::Duration::from_secs(BEAT_EVERY)).await;
                    }
                });
                if ended.is_ok() {
                    BEATING.store(false, Ordering::SeqCst); // no shared database: asked again when there is one
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(BEAT_AGAIN));
            }
        });
        if started.is_err() {
            BEATING.store(false, Ordering::SeqCst);
        }
    }

    /// What the app's other process asks of the tracker's (`trackerlink`). `hello` is answered to anyone: it
    /// gives nothing away, it proves; everything else only with this run's secret.
    fn local_get(&self, req: &Request) -> Response {
        use crate::trackerlink;
        if req.path == "/api/local/hello" {
            return Response::json(json!({"proof": trackerlink::proof(&req.param("n").unwrap_or_default())}));
        }
        if !trackerlink::allowed(&req.headers) {
            return Response::error(403, trackerlink::REFUSAL);
        }
        match req.path.as_str() {
            // who this device's camera sees now (for the "moved between cameras" alert). Asked at every step
            // of the sync, it also keeps the camera's own part in the shared database going from here: that
            // this device is there, and whether its camera streams.
            "/api/local/inview" => {
                self.start_heartbeat();
                let within = req.param("within").and_then(|w| w.parse::<f64>().ok()).unwrap_or(6.0);
                Response::json(json!({"names": crate::camstream::in_view(within.clamp(1.0, 60.0))}))
            }
            "/api/local/devices" => {
                let devices: serde_json::Map<String, Value> =
                    self.devices().into_iter().filter_map(|(job, at)| Some((job.to_string(), json!(at?)))).collect();
                let decisions: serde_json::Map<String, Value> =
                    face_core::runtime::decisions().into_iter().map(|(model, why)| (model, json!(why))).collect();
                Response::json(json!({"devices": devices, "decisions": decisions}))
            }
            // the port the Cameras page is served on, if it has its own
            "/api/local/cameras" => Response::json(json!({"port": trackerlink::cameras()})),
            _ => Response::error(404, "not found"),
        }
    }
}

impl Service for Tracker {
    // beyond the shared policy: the microphone's sound tap and Open video, both blob: URLs the page makes
    fn policy(&self) -> &'static [(&'static str, &'static str)] {
        // 'wasm-unsafe-eval': MediaPipe's runtime in the page is WebAssembly, which a page under a script
        // policy may only compile with this word (it allows that, not eval of text)
        &[("script-src", "'self' blob: 'wasm-unsafe-eval'"), ("media-src", "'self' blob:")]
    }

    fn body_limit(&self, path: &str) -> Option<usize> {
        Some(match path {
            "/api/recognize" | "/api/capture" => 20_000_000,
            "/api/stream/frame" | "/api/join/frame" => 3_000_000,
            "/api/join/show" | "/api/join/hide" | "/api/join/give" | "/api/join/code" | "/api/join/name" => 1_000,
            "/api/page-note" | "/api/local/peer" | "/api/sound/live" => 2_000,
            "/api/inview" => 400_000,
            "/api/bus" => 2 * crate::bus::MAX_VALUE,
            "/api/settings" | "/api/expression-log" | "/api/who" | "/api/expr-eval/report" => 200_000,
            "/api/signals" => MAX_SIGNALS + 1000,
            "/api/expr-eval/delete" => 2_000,
            "/api/voice" => 4 * 16_000 * 10 + 1,
            #[cfg(feature = "sound")]
            "/api/sound/heard" => crate::soundheard::MAX_BYTES + 1,
            "/api/alerts" | "/api/alerts/details" => 25_000,
            "/api/alerts/state" => 100_000,
            "/api/presence" => 10_000,
            _ => return None,
        })
    }

    fn get(&self, req: &Request) -> Response {
        if let Some((_, file, ctype)) = PAGES.iter().find(|(p, _, _)| *p == req.path) {
            return match crate::pagefiles::read(&self.pages, file) {
                // (from a named folder: read each time, an edit shows on refresh)
                Some(b) => Response::bytes(ctype, b),
                None => Response::error(404, "not found"),
            };
        }
        let state = |r: Result<Value, String>| r.map_or_else(|e| Response::error(500, e), Response::json);
        match req.path.as_str() {
            // the app's other process asks: never a page
            path if path.starts_with("/api/local/") => self.local_get(req),
            "/mp/parts.js" => self.parts_script("parts.js", PARTS_SCRIPT),
            "/mp/parts_worker.js" => self.parts_script("parts_worker.js", PARTS_WORKER),
            // the privacy policy, readable inside the app (a page like the others: no data, nothing of this device)
            "/privacy" => Response::bytes("text/html; charset=utf-8", crate::privacy::page().into_bytes()),
            path if path.starts_with("/mp/") => {
                // fixed names only: nothing of the request goes into a file's path
                let file = MP_FILES
                    .iter()
                    .find(|(p, _, _)| *p == path)
                    .map(|(_, file, ctype)| (*file, *ctype))
                    .or_else(|| {
                        MP_MODELS.iter().find(|(p, _)| *p == path).map(|(_, file)| (*file, "application/octet-stream"))
                    });
                match file
                    .and_then(|(file, ctype)| Some((std::fs::read(self.root.join("models").join(file)).ok()?, ctype)))
                {
                    Some((bytes, ctype)) => Response::bytes(ctype, bytes),
                    None => Response::not_found(),
                }
            }
            "/join" => Response::bytes("text/html; charset=utf-8", join::PAGE.as_bytes().to_vec()),
            "/join_ui.js" => Response::bytes("text/javascript; charset=utf-8", join::SCRIPT.as_bytes().to_vec()),
            "/api/join" => {
                // (this device joined by a code it showed: what a device of a shared database does starts now)
                if self.joining.joined_now() {
                    self.start_shared();
                }
                Response::json(self.joining.state(&crate::device().0))
            }
            // what this device is called in the shared database, for its own page to show and to ask about
            "/api/join/name" => Response::json(self.own_name()),
            "/api/join/code.svg" => match self.joining.picture() {
                Some(svg) => Response::bytes("image/svg+xml", svg.into_bytes()),
                None => Response::not_found(),
            },
            "/api/people" => Response::json(self.people_answer()),
            "/api/settings" => state(self.store.page_state("track")),
            "/api/signals" => state(self.store.signals_state(&req.param("v").unwrap_or_default())),
            // this device itself for the Signals chart: what it is, and how hard it works now (asked of the
            // system when the page asks; nothing of it is kept)
            "/api/device/live" => Response::json(json!({"reading": crate::devicelive::reading(),
                                                        "facts": crate::devicelive::facts(&crate::device().1)})),
            // where this device is, while its owner has switched that on: asked by the page every few
            // seconds and when the switch or the typed place changes (a step is taken first, so the
            // answer is the settings' as they are now)
            "/api/location" => {
                let _ = self.bus.set("place", self.location.look());
                Response::json(self.location.answer())
            }
            // is the screen locked? (Windows stops running cameras then)
            "/api/screen" => Response::json(json!({"locked": crate::desktop::screen_locked()})),
            "/api/alerts" => state(self.alerts.listing()),
            "/api/presence" => {
                let day = req.param("day").filter(|d| !d.is_empty()).unwrap_or_else(presence::today);
                match self.presence.visits(&day) {
                    Ok(visits) => Response::json(json!({"day": day, "visits": visits})),
                    Err(e) => Response::error(400, e),
                }
            }
            // the bus: this node's tree and its changes, or another node's as it is followed here; and the
            // nodes there are to ask about
            "/api/bus" => self.bus_answer(req),
            "/api/bus/many" => self.bus_many(req),
            "/api/bus/nodes" => Response::json(self.bus_nodes()),
            // the devices whose cameras can be watched
            "/api/cameras" => Response::json(self.cameras()),
            // their live readouts (faces, names, readings), as data
            "/api/cameras/readouts" => Response::json(
                self.relays.readouts(&self.stream, &self.cameras_asked(&req.param("ids").unwrap_or_default())),
            ),
            // Every asked-for camera's next frame in one answer (?after=id:n,id:n), as soon as any has one
            // newer than n, or after 3 s with none. The body: a 4-byte length, that many bytes of JSON
            // {frames: [{id, seq, size}], errors: {id: why}}, then the frames' JPEGs one after another.
            "/api/cameras/frames" => {
                let after = self.cameras_asked(&req.param("after").unwrap_or_default());
                let (got, errors) = self.relays.frames(
                    &self.stream,
                    &after,
                    "page:127.0.0.1",
                    std::time::Duration::from_secs(3),
                    std::time::Duration::from_millis(20),
                );
                let frames: Vec<Value> =
                    got.iter().map(|(id, seq, jpeg)| json!({"id": id, "seq": seq, "size": jpeg.len()})).collect();
                let head = crate::filejson::dumps(&json!({"frames": frames, "errors": errors})).into_bytes();
                let mut body = (head.len() as u32).to_be_bytes().to_vec();
                body.extend_from_slice(&head);
                for (_, _, jpeg) in &got {
                    body.extend_from_slice(jpeg);
                }
                Response::bytes("application/octet-stream", body)
            }
            _ => Response::not_found(),
        }
    }

    fn post(&self, req: &Request) -> Response {
        let done = |r: Result<Value, String>| r.map_or_else(|e| Response::error(400, e), Response::json);
        // an alert or a visit saved here: the sync, in the app's other process, hears of it at once
        let saved = |answer: Response| {
            if answer.status == 200 {
                crate::trackerlink::note_saved();
            }
            answer
        };
        match req.path.as_str() {
            // the sync (the app's other process): the peer service on or off
            "/api/local/peer" => self.local_peer(req),
            "/api/recognize" => {
                let threshold = match req.param("t") {
                    None => Ok(self.faces.rec_model.match_at),
                    Some(t) => t.parse::<f64>().map_err(|_| format!("could not convert string to float: '{t}'")),
                };
                done(threshold.and_then(|t| {
                    let posed = Posed::of(req)?;
                    self.recognize(&req.body, t.clamp(0.1, 0.9), posed.as_ref())
                }))
            }
            "/api/who" => done(parsed(&req.body).and_then(|b| self.who(&b))),
            // a feeling report, and "Delete test data"
            "/api/expr-eval/report" => done(recorder::check_report(&req.body).and_then(|rep| {
                self.recorder.report(&rep)?;
                Ok(json!({"saved": true, "reports": self.recorder.reports()}))
            })),
            "/api/expr-eval/delete" => Response::json(json!({"deleted": self.recorder.delete_all()})),
            // float32 samples at 16 kHz, 0.5 to 10 s: scored, not kept
            "/api/voice" => {
                if req.body.len() % 4 != 0 || req.body.len() < 2 * face_core::voice::RATE {
                    return Response::error(400, "expected 0.5-10 s of float32 audio");
                }
                let samples: Vec<f32> =
                    req.body.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                if !samples.iter().all(|v| v.is_finite()) {
                    return Response::error(400, "bad audio");
                }
                let t0 = Instant::now();
                done(self.voice.avd(&samples).map_err(|e| e.0).map(|avd| {
                    let avd = avd.map(|v| v.iter().map(|x| rounded(*x as f64, 3)).collect::<Vec<f64>>());
                    json!({"avd": avd, "ms": rounded(t0.elapsed().as_secs_f64() * 1000.0, 1)})
                }))
            }
            "/api/join/frame" => done(self.joining.frame(&req.body).inspect(|answer| {
                if answer.get("joined").and_then(Value::as_bool) == Some(true) {
                    self.start_shared();
                }
            })),
            // "Collect faces": each face's picture for the labeling tool (?faces=[...]&others=[...])
            "/api/capture" => {
                let list = |key: &str| {
                    serde_json::from_str::<Value>(&req.param(key).unwrap_or_else(|| "[]".to_string()))
                        .map_err(|e| format!("not JSON: {e}"))
                };
                done(
                    list("faces").and_then(|faces| {
                        Ok(json!({"saved": self.capture.save(&req.body, &faces, &list("others")?)?}))
                    }),
                )
            }
            // one new alert the page raised, with what it measured
            "/api/alerts" => saved(done(parsed(&req.body).and_then(|a| {
                let alert = self.alerts.add(&a)?;
                Ok(json!({"alert": alert, "counts": self.alerts.counts()?}))
            }))),
            // the second set of measurements, 15 s later
            "/api/alerts/details" => saved(done(parsed(&req.body).and_then(|b| {
                if !b.is_object() {
                    return Err("expected an object".to_string());
                }
                Ok(json!({"saved": self.alerts.add_details(b.get("id"), b.get("which"), b.get("details"))?}))
            }))),
            // acknowledge, archive, back to new, or delete
            "/api/alerts/state" => saved(done(parsed(&req.body).and_then(|b| {
                if !b.is_object() {
                    return Err("expected an object".to_string());
                }
                Ok(json!({"counts": self.alerts.set_state(b.get("ids"), b.get("state"), b.get("where"))?}))
            }))),
            // the opt-in presence log: one finished visit
            "/api/presence" => {
                saved(done(parsed(&req.body).and_then(|v| self.presence.log_visit(&v)).map(|_| json!({"saved": true}))))
            }
            "/api/join/code" => {
                let answer = self.joining.code(&String::from_utf8_lossy(&req.body));
                if answer.get("joined").and_then(Value::as_bool) == Some(true) {
                    self.start_shared();
                }
                Response::json(answer)
            }
            "/api/join/show" => done(self.joining.show(&crate::device().0)),
            // this device's own name, given by whoever is at it (kept in the shared database, one device's alone)
            "/api/join/name" => {
                let name = String::from_utf8_lossy(&req.body).into_owned();
                let named = face_db::Device::load(&self.home, &crate::device().0)
                    .and_then(|me| face_db::name_device(&self.home, &me, &me.id, &name));
                done(named.map(|name| json!({"ok": true, "name": name})))
            }
            // the owner said yes to the new device whose code this one read
            "/api/join/give" => done(self.joining.give(&crate::device().0)),
            "/api/join/hide" => {
                self.joining.hide();
                Response::json(json!({"ok": true}))
            }
            "/api/settings" => done(parsed(&req.body).and_then(|b| {
                if b.get("app").and_then(Value::as_str) != Some("track") {
                    return Err("bad settings".to_string());
                }
                Ok(json!({"settings": self.store.update("track", b.get("settings").unwrap_or(&json!({})))?}))
            })),
            "/api/signals" => done(parsed(&req.body).and_then(|b| {
                let key = b.get("key").and_then(Value::as_str).unwrap_or("");
                self.store.merge_signals(
                    key,
                    b.get("changed").unwrap_or(&json!({})),
                    b.get("gone").unwrap_or(&json!([])),
                )?;
                Ok(json!({"ok": true}))
            })),
            "/api/page-note" => done(
                parsed(&req.body).map(|b| json!({"logged": self.page_note(b.get("text").and_then(Value::as_str))})),
            ),
            // this camera as the page shows it, for the other devices: kept only while the peer service runs
            "/api/stream/frame" => {
                if !self.peer_on.load(Ordering::SeqCst) {
                    return Response::json(json!({"stream": false}));
                }
                if let Some(why) = req.headers.get("x-stream-failed") {
                    let why: String = why.chars().filter(|c| (' '..='~').contains(c)).take(200).collect();
                    println!("Camera: a frame from the tracker page failed ({why})");
                }
                done(self.stream.put(&req.body).map(|watchers| json!({"stream": true, "watchers": watchers})))
            }
            // This device's sound for the Signals chart: the page asks every few seconds while its switch is
            // on, and for as long as it asks every audio interface is listened to, a few seconds at a time, one
            // stretch after another (soundlive.rs). The answer is the newest listening as numbers and one word.
            #[cfg(feature = "import")]
            "/api/sound/live" => Response::json(self.sound_live.ask()),
            // A device whose program can't listen to its audio interfaces (a phone) says that its page is to
            // hear for it: the page then sends what its microphone heard to /api/sound/heard.
            #[cfg(all(feature = "sound", not(feature = "import")))]
            "/api/sound/live" => Response::json(
                json!({"available": false, "page": true, "why": "This device's sound is heard by the page's microphone."}),
            ),
            #[cfg(not(feature = "sound"))]
            "/api/sound/live" => {
                Response::json(json!({"available": false, "why": "This device doesn't listen to its sound."}))
            }
            // What the page's microphone heard, for the chart: a few seconds of one channel of float32 numbers
            // (?rate= samples a second), measured here and dropped. Numbers and one word are the answer.
            #[cfg(feature = "sound")]
            "/api/sound/heard" => {
                let rate = req.param("rate").and_then(|r| r.parse::<usize>().ok()).unwrap_or(0);
                done(self.heard.measure(&req.body, rate).map(|reading| json!({"reading": reading})))
            }
            // who this camera sees now (the moved-between-cameras alert), and the page's live readout (with
            // the device's own signals for the Signals chart), sent along with the stream
            "/api/inview" => done(parsed(&req.body).and_then(|b| {
                if let Some(names) = b.get("names").and_then(Value::as_array) {
                    crate::camstream::set_in_view(names);
                }
                if let Some(readout) = b.get("readout").filter(|r| !r.is_null()) {
                    self.stream.set_readout(readout, b.get("paused"), b.get("signals"))?;
                    // Who is in view goes out with the camera's readout, so only while the camera is streamed
                    // to the other devices, and is said, not kept: a camera that stopped leaves no names behind.
                    if let Some(names) = b.get("names").filter(|n| n.is_array()) {
                        let _ = self.bus.say("inview", names.clone());
                    }
                }
                Ok(json!({"ok": true}))
            })),
            // what this device's page publishes of it: its catalog, what is on, its settings
            "/api/bus" => done(parsed(&req.body).and_then(|b| self.bus_publish(&b))),
            // Readings for the expression log: kept only with a shared database to keep them in and while
            // the page's switch is on. Not on a phone yet: nothing there sends them on,
            // and they would only pile up; on a computer the sync sends these files.
            "/api/expression-log" => done(parsed(&req.body).and_then(|b| {
                let enabled = !cfg!(any(target_os = "android", target_os = "ios"))
                    && self.joining.joined()
                    && self.store.setting("track", exprlog::OPTION).is_none_or(|v| v != "0");
                let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
                Ok(json!({"kept": self.exprlog.add(b.get("readings"), enabled, &self.store.forgotten(), now)?}))
            })),
            _ => Response::error(501, NOT_YET),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rounded;

    #[test]
    fn numbers_are_rounded_a_half_to_the_even_neighbour() {
        assert_eq!(rounded(0.125, 2), 0.12); // a half goes to the even neighbour
        assert_eq!(rounded(2.675, 2), 2.67); // 2.675 is a little under itself in binary
        assert_eq!(rounded(12.34, 1), 12.3);
        assert_eq!(rounded(0.35f32 as f64, 4), 0.35);
    }
}
