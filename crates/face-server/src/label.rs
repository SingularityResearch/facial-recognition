//! The labeling tool's server: the labeling
//! page and its scripts, the lists it shows (the summary, suggested and unknown faces, automatic names, a
//! person's faces, the checks, possible faces, the faces named by tags, the photos of people together, a
//! photo's facts and clues), every change with its Undo, the export of photos to a folder, the faces' own
//! pictures, and the page's settings. All of it is the labeling engine's (`face_library::labels`), which keeps
//! the library's files.
//!
//! Not here yet, each with the part it belongs to: the photo itself and a missing face picture made again
//! from it (they need pictures decoded: the importer's part), adding photos, looking for more faces and
//! drawing one (the importer), Describe photo, Web search, one person's export and forget, and what the
//! Settings tab asks of the labeling tool. Those routes answer "not found".

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use face_core::faces::REC_MODELS;
#[cfg(feature = "import")]
use face_import::{decode, importer::Importer, video};
use face_library::{
    faceness,
    labels::{Fault, LabelDb},
    photofiles, relocate,
};
use serde_json::{Map, Value, json};

use crate::{
    http::{Request, Response, Service},
    store::Store,
};

const HTML: &str = "text/html; charset=utf-8";
const SCRIPT: &str = "text/javascript; charset=utf-8";
pub(crate) const PAGES: [(&str, &str, &str); 7] = [
    ("/", "label_ui.html", HTML),
    ("/index.html", "label_ui.html", HTML),
    ("/label_ui.js", "label_ui.js", SCRIPT),
    ("/settings_bridge.js", "settings_bridge.js", SCRIPT),
    ("/page_store.js", "page_store.js", SCRIPT),
    ("/ask.js", "ask.js", SCRIPT),
    ("/favicon.ico", "icons/icon.ico", "image/x-icon"),
];
/// stands in for a face whose picture isn't on this computer
const NO_FACE: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 112 112\"><rect width=\"112\" height=\"112\" fill=\"#8884\"/>\
<circle cx=\"56\" cy=\"44\" r=\"20\" fill=\"#8888\"/><path d=\"M18 104c4-24 20-34 38-34s34 10 38 34z\" fill=\"#8888\"/>\
<title>No picture of this face on this computer</title></svg>";
/// bytes in a request (a list of 100,000 face numbers is about 0.7 MB)
const MAX_BODY: usize = 2_000_000;
/// in the data folder: the Web search answers by face, so that a face is searched (and paid for) once
pub(crate) const WEB_SAVED: &str = "web_suggestions.json";

pub(crate) struct Inner {
    db: Arc<Mutex<LabelDb>>,
    /// reads folders of photos into the library, and one photo for the photo view
    #[cfg(feature = "import")]
    pub(crate) importer: Arc<Importer>,
    /// the last few whole photos shown, as JPEG: (photo, frame) -> its bytes
    #[cfg(feature = "import")]
    shown: Mutex<Vec<((String, Option<i64>), Arc<Vec<u8>>)>>,
    /// faces whose picture is missing and waits to be made again from the photo, and those it couldn't be
    /// made for (by permanent ID: a face's place in the library can change)
    #[cfg(feature = "import")]
    pictures: (Mutex<(Vec<usize>, std::collections::HashSet<String>)>, std::sync::Condvar),
    /// the possible faces are being learned about on a thread of its own
    learning: AtomicBool,
    /// face ID -> its last complete Web search answer
    pub(crate) web_saved: Mutex<Map<String, Value>>,
    /// the shared database's sync, once it is started
    #[cfg(feature = "sync")]
    syncer: std::sync::OnceLock<Arc<crate::syncer::Syncer>>,
    /// asks the other devices of a shared database for photos that aren't on this computer
    pub(crate) asker: crate::peerphoto::Asker,
}

/// The sync takes the library only while it reads or writes this device's own data: the labeling data's
/// lock, then the Web search answers'.
#[cfg(feature = "sync")]
impl face_sync::sync::Library for Inner {
    fn with<R>(&self, f: impl FnOnce(&mut LabelDb, &mut Map<String, Value>) -> R) -> R {
        let mut db = self.db();
        let mut web = self.web_saved.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut db, &mut web)
    }
}

pub struct Labeler {
    pub(crate) inner: Arc<Inner>,
    pub(crate) store: Store,
    pages: PathBuf,
    #[cfg_attr(not(any(feature = "import", feature = "sync")), allow(dead_code))]
    pub(crate) home: PathBuf,
    /// the library's folder
    #[cfg_attr(not(feature = "sync"), allow(dead_code))]
    pub(crate) data: PathBuf,
    /// the shared database's connection kept for Settings' short questions
    #[cfg(feature = "sync")]
    pub(crate) kept: face_db::kept::Kept,
}

/// What a route answers, or the answer that says why not.
pub(crate) type Answer = Result<Response, Response>;

pub(crate) fn asked(why: impl Into<String>) -> Response {
    Response::error(400, why)
}

fn fault(e: Fault) -> Response {
    match e {
        Fault::Asked(why) => Response::error(400, why),
        Fault::Files(why) => Response::error(500, why),
    }
}

/// A face's number from a part of the address.
pub(crate) fn number(text: &str) -> Result<i64, Response> {
    text.trim().parse().map_err(|_| asked(format!("That is not a whole number: '{text}'")))
}

impl Inner {
    pub(crate) fn db(&self) -> MutexGuard<'_, LabelDb> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the other devices of a shared database can be asked: the sync runs here and is online.
    pub(crate) fn peer_ready(&self) -> bool {
        #[cfg(feature = "sync")]
        {
            self.syncer.get().is_some_and(|s| s.status()["state"] == "online")
        }
        #[cfg(not(feature = "sync"))]
        {
            false
        }
    }

    /// The original of a photo (a video's frame) from another device of the shared database that has it:
    /// shown or described, never stored. None when no device can be asked or none sent it.
    #[cfg_attr(not(feature = "import"), allow(dead_code))]
    pub(crate) fn from_peer(&self, photo: &str, frame: Option<i64>) -> Option<Vec<u8>> {
        if !self.peer_ready() {
            return None;
        }
        self.asker.fetch(photo, frame).ok().map(|(jpeg, _)| jpeg)
    }
}

impl Labeler {
    /// The labeling tool for the library in `root/data`, with this user's private folder and the folder
    /// the page files are in. `log`: told what was changed while opening.
    pub fn new(root: &Path, home: &Path, pages: &Path, log: &mut dyn FnMut(String)) -> Result<Labeler, String> {
        let data = root.join("data");
        #[cfg(feature = "import")]
        crate::florence::set_root(root); // the built-in describe model's files are beside the other models
        #[cfg(not(feature = "import"))]
        let db = LabelDb::open(&data, home, REC_MODELS[0], true, log).map_err(|e| e.to_string())?;
        // A library whose prints are of another recognition model than this version's gets every print made
        // again while it is opened: from the picture it keeps of each face, or from the photo where there is
        // none. The models are loaded only when that is needed, and then handed to the importer.
        #[cfg(feature = "import")]
        let (db, models) = {
            let mut models: Option<Arc<face_core::faces::Faces>> = None;
            let mut no_models: Option<String> = None;
            let mut make = |store: &face_library::store::Store, i: usize| -> Option<Vec<f32>> {
                if models.is_none() && no_models.is_none() {
                    match face_import::importer::load_models(root, home) {
                        Ok(faces) => models = Some(faces),
                        Err(e) => no_models = Some(e),
                    }
                }
                let faces = models.as_ref()?;
                let kept = store.thumb(i).filter(|c| c.w == 112 && c.h == 112);
                let crop = kept.or_else(|| Importer::rebuild_crop(faces, store, i))?;
                faces.embed(&crop).ok()
            };
            let opened = LabelDb::open_making(&data, home, REC_MODELS[0], true, log, Some(&mut make));
            match (opened, no_models) {
                (Ok(db), _) => (db, models),
                (Err(_), Some(why)) => {
                    return Err(format!(
                        "The face prints have to be made again and the models couldn't be loaded: {why}"
                    ));
                }
                (Err(e), None) => return Err(e.to_string()),
            }
        };
        let web_saved = match std::fs::read(data.join(WEB_SAVED)).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            Some(Value::Object(saved)) => saved,
            _ => Map::new(),
        };
        let db = Arc::new(Mutex::new(db));
        #[cfg(feature = "import")]
        let importer = {
            let importer = Importer::new(db.clone(), root, home);
            // an Ignored face that is given a name gets its print back from the same models
            db.lock().unwrap_or_else(|e| e.into_inner()).remake = Some(importer.remake());
            if let Some(faces) = models {
                importer.use_models(faces);
            }
            importer
        };
        let inner = Arc::new(Inner {
            db,
            #[cfg(feature = "import")]
            importer,
            #[cfg(feature = "import")]
            shown: Mutex::new(Vec::new()),
            #[cfg(feature = "import")]
            pictures: Default::default(),
            learning: AtomicBool::new(false),
            web_saved: Mutex::new(web_saved),
            #[cfg(feature = "sync")]
            syncer: std::sync::OnceLock::new(),
            asker: crate::peerphoto::Asker::new(home),
        });
        Ok(Labeler {
            inner,
            store: Store::new(home),
            pages: pages.to_path_buf(),
            home: home.to_path_buf(),
            data,
            #[cfg(feature = "sync")]
            kept: face_db::kept::Kept::new(home),
        })
    }

    /// Keep the library and the shared database the same, on a thread of its own (once): what is saved here is
    /// sent a moment later, and the other devices' changes arrive every few seconds.
    #[cfg(feature = "sync")]
    pub fn start_sync(&self, root: &Path, home: &Path) {
        // another device forgot someone: here too, everywhere a person's data is
        let (inner, data, private) = (self.inner.clone(), root.join("data"), home.to_path_buf());
        let forget = move |name: &str| {
            crate::persondata::forget(&inner, &data, &private, name, true).map(|_| ()).map_err(|e| e.to_string())
        };
        let syncer = crate::syncer::Syncer::start(
            self.inner.clone(),
            &root.join("data"),
            home,
            Some(std::sync::Arc::new(forget)),
            |line| println!("{line}"),
        );
        if self.inner.syncer.set(syncer.clone()).is_ok() {
            self.inner.db().on_saved.push(Box::new(move || syncer.note()));
        }
    }

    /// What the sync is doing: {state: off | joining | online | offline, text, waiting, last, error}.
    #[cfg(feature = "sync")]
    pub fn sync_status(&self) -> Value {
        let off = "The shared database's sync isn't running in this program (it is started with --sync).";
        let none = json!({"state": "off", "text": off, "waiting": 0, "last": null, "error": null});
        self.inner.syncer.get().map_or(none, |s| s.status())
    }

    #[cfg(feature = "sync")]
    pub(crate) fn syncer(&self) -> Option<&Arc<crate::syncer::Syncer>> {
        self.inner.syncer.get()
    }

    /// (faces, the recognition model): for the line that says the tool is open.
    pub fn opened(&self) -> (usize, String) {
        let db = self.inner.db();
        (db.n(), db.store.model.clone())
    }

    /// What is done behind the tool's start: photos that moved are found again, then every photo is
    /// read for its camera date and people tags (those that changed), and the possible faces are learned
    /// about. On a thread of its own; the page is answered meanwhile.
    pub fn start_reading(&self, root: &Path) {
        let (inner, data) = (self.inner.clone(), root.join("data"));
        std::thread::spawn(move || {
            let folders = photofiles::folders(&data);
            let found = relocate::relocate(&mut inner.db().store, &data, &folders, &mut |line| println!("{line}"));
            match found {
                Ok(r) if r["found"].as_u64().unwrap_or(0) > 0 => {
                    inner.db().changed();
                    let moved: String = r["folders"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(o, n)| format!("\n  {o} -> {}", n.as_str().unwrap_or("")))
                        .collect();
                    println!("Found {} of {} photos that had moved.{moved}", r["found"], r["missing"]);
                }
                Ok(r) if r["missing"].as_u64().unwrap_or(0) > 0 => {
                    println!("{} photos aren't where they were and weren't found in your photo folders.", r["missing"]);
                }
                Ok(_) => {}
                Err(e) => println!("Couldn't look for moved photos: {e}"),
            }
            // the photos' facts: what to read is taken under the lock, read without it, taken in under it
            let (jobs, listings) = {
                let mut db = inner.db();
                (db.facts_jobs(None), db.facts.listings.clone())
            };
            let done = listings.run(&jobs);
            if let Err(e) = inner.db().facts_apply(&jobs, done) {
                println!("Couldn't keep what the photos say about themselves: {e}");
            }
            learn(&inner);
        });
        self.watch_webcam();
        #[cfg(feature = "import")]
        crate::libhistory::follow_imports(self.inner.clone(), self.home.clone());
        self.make_pictures();
    }

    /// Face pictures that are missing are made again from the photos behind the page (a copy of a library
    /// can come without `thumbs.npz`: the pictures stay on the computer they were made on): about half a
    /// second each, one at a time, the newest asked for first, so a list of faces doesn't wait for them. The
    /// page shows the stand-in until it draws the list again.
    fn make_pictures(&self) {
        #[cfg(feature = "import")]
        {
            let inner = self.inner.clone();
            std::thread::spawn(move || {
                loop {
                    let i = {
                        let (lock, more) = &inner.pictures;
                        let mut wait = lock.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(i) = wait.0.pop() {
                                break i;
                            }
                            wait = more.wait(wait).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    // what is needed of the library, then the library let go of for the reading and looking
                    let from = {
                        let db = inner.db();
                        let wanted = i < db.n() && db.store.thumbs[i].is_empty() && !db.printless(i);
                        wanted.then(|| (db.store.fid[i].clone(), Importer::crop_source(&db.store, i)))
                    };
                    let Some((fid, from)) = from else { continue };
                    let crop = from.and_then(|from| Importer::crop_from(&*inner.importer.models().ok()?, &from));
                    let mut db = inner.db();
                    let kept = match crop {
                        Some(crop) if db.store.fid.get(i) == Some(&fid) => db.store.set_thumb(i, &crop).is_ok(),
                        _ => false,
                    };
                    drop(db);
                    if !kept {
                        inner.pictures.0.lock().unwrap_or_else(|e| e.into_inner()).1.insert(fid);
                    }
                }
            });
        }
    }

    /// Ask for face `i`'s missing picture to be made again (see `make_pictures`); an Ignored face's was
    /// deleted on purpose and is never made again.
    fn picture_soon(&self, i: usize) {
        #[cfg(feature = "import")]
        {
            let fid = {
                let db = self.inner.db();
                if db.printless(i) {
                    return;
                }
                db.store.fid[i].clone()
            };
            let (lock, more) = &self.inner.pictures;
            let mut wait = lock.lock().unwrap_or_else(|e| e.into_inner());
            if !wait.1.contains(&fid) && !wait.0.contains(&i) {
                wait.0.push(i);
                more.notify_one();
            }
        }
        #[cfg(not(feature = "import"))]
        let _ = i;
    }

    /// Import the tracker's snapshots (its "Collect faces") by themselves: whenever new ones have arrived
    /// and no other import is running. The import names the faces it is sure of; the rest wait among the
    /// suggested and unknown faces with the tracker's own verdict among the clues.
    fn watch_webcam(&self) {
        #[cfg(feature = "import")]
        {
            let (inner, folder) = (self.inner.clone(), self.home.join("webcam"));
            std::thread::spawn(move || {
                let mut seen = None;
                loop {
                    let newest = (std::fs::read_dir(&folder).into_iter().flatten().flatten())
                        .filter(|f| f.path().extension().is_some_and(|e| e == "jpg"))
                        .filter_map(|f| f.metadata().ok()?.modified().ok())
                        .max();
                    let st = inner.importer.status();
                    // a folder import's result stays on the page for five minutes before this takes its place
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0.0, |d| d.as_secs_f64());
                    let ended = st["started"].as_f64().unwrap_or(0.0) + st["elapsed"].as_f64().unwrap_or(0.0);
                    let other = st["folder"].as_str().is_some_and(|f| !f.is_empty() && Path::new(f) != folder);
                    if newest.is_some() && newest > seen && st["running"] != true && !(other && now - ended < 300.0) {
                        // a job that began meanwhile: tried again the next time round
                        if inner.importer.start(&folder.to_string_lossy()).is_ok() {
                            seen = newest;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(20));
                }
            });
        }
    }

    /// How far the import has got, for the page.
    fn import_status(&self) -> Value {
        #[cfg(feature = "import")]
        {
            self.inner.importer.status()
        }
        #[cfg(not(feature = "import"))]
        {
            json!({"running": false})
        }
    }

    /// The whole photo face `i` was found in (of a video: the frame it was found in), as JPEG, at the size
    /// the faces' boxes are in. A photo that isn't on this computer is asked of another device of the shared
    /// database that has it; None when none sends it.
    #[cfg(feature = "import")]
    pub(crate) fn photo_jpeg(&self, i: usize) -> Option<Arc<Vec<u8>>> {
        let (key, path) = {
            let db = self.inner.db();
            let h = db.store.photo.get(i)?.clone();
            let rec = db.store.photos.get(&h)?.as_object()?;
            let video = rec.get("video").is_some_and(|v| v.as_bool() == Some(true));
            let frame = rec.get("at").filter(|_| video).and_then(|a| a.get(&db.store.fid[i])).and_then(Value::as_i64);
            ((h, frame), rec.get("path").and_then(Value::as_str).unwrap_or("").to_string())
        };
        let held = || self.inner.shown.lock().unwrap_or_else(|e| e.into_inner());
        {
            // (one lock for the look and the move: a second one inside the first would wait for itself)
            let mut shown = held();
            if let Some(at) = shown.iter().position(|(k, _)| *k == key) {
                let hit = shown.remove(at);
                shown.push(hit.clone());
                return Some(hit.1);
            }
        }
        let jpg = if path.is_empty() || !Path::new(&path).exists() {
            // another device that has it shows it to this one
            Arc::new(self.inner.from_peer(&key.0, key.1)?)
        } else {
            let img = match key.1 {
                None => decode::load_file(Path::new(&path)).ok()?,
                Some(k) => video::frame(Path::new(&path), k).ok()?,
            };
            Arc::new(img.to_jpeg(85).ok()?)
        };
        let mut shown = held();
        shown.push((key, jpg.clone()));
        let extra = shown.len().saturating_sub(12);
        shown.drain(..extra);
        Some(jpg)
    }

    /// What the import's routes answer; None for a route that isn't one of them.
    #[cfg(feature = "import")]
    fn import_post(&self, path: &str, body: &Map<String, Value>) -> Option<Answer> {
        let importer = &self.inner.importer;
        let done = |r: Result<Value, String>| Some(r.map(Response::json).map_err(asked));
        // the face a request is about: a missing or bad one is refused, never taken as some other face
        let face = || -> Result<usize, String> {
            let i = match body.get("i") {
                Some(Value::Number(n)) => n.as_f64().map(|f| f.trunc() as i64),
                Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
                Some(Value::Bool(b)) => Some(*b as i64),
                _ => None,
            };
            i.and_then(|i| usize::try_from(i).ok())
                .filter(|i| *i < self.inner.db().n())
                .ok_or_else(|| "no such face".to_string())
        };
        let place = |v: Option<&Value>| -> Option<[f32; 4]> {
            let b: Vec<f32> = v?.as_array()?.iter().filter_map(|x| x.as_f64().map(|x| x as f32)).collect();
            <[f32; 4]>::try_from(b).ok()
        };
        match path {
            "/api/import" => done(
                importer.start(body.get("folder").and_then(Value::as_str).unwrap_or("")).map(|_| json!({"ok": true})),
            ),
            "/api/import/stop" => {
                importer.stop();
                done(Ok(json!({"ok": true})))
            }
            // look for possible faces in the photos added before those were kept
            "/api/possible/scan" => done(importer.scan_possible().map(|_| json!({"ok": true}))),
            // look again for faces in one photo (the photo view)
            // (one of them, by its box, or all: a box that can't be read is refused, never taken as "all")
            "/api/morefaces" => done(
                face()
                    .and_then(|i| match body.get("box") {
                        None | Some(Value::Null) => importer.find_more(i, None),
                        at => importer.find_more(i, Some(place(at).ok_or("a box is four numbers")?)),
                    })
                    .map(|new| json!({"ok": true, "new": new})),
            ),
            // a face drawn by hand in the photo view
            "/api/drawface" => done(face().and_then(|i| match place(body.get("box")) {
                Some(at) => importer.add_drawn(i, at).map(|j| json!({"ok": true, "i": j})),
                None => Err("Draw a bigger box around the face.".to_string()),
            })),
            _ => None,
        }
    }

    /// Learn about the possible faces when the faces changed, on a thread of its own: what was known before
    /// stands until it is done.
    fn learn_soon(&self) {
        if self.inner.db().faceness_due() && !self.inner.learning.swap(true, Ordering::SeqCst) {
            let inner = self.inner.clone();
            std::thread::spawn(move || {
                learn(&inner);
                inner.learning.store(false, Ordering::SeqCst);
            });
        }
    }

    fn photo_info(&self, i: usize) -> Value {
        let mut db = self.inner.db();
        let h = db.store.photo[i].clone();
        let photo = db.store.photos.get(&h).and_then(Value::as_object).cloned().unwrap_or_default();
        let path = photo.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string);
        let frame = db.video_at(i);
        let faces: Vec<usize> = db.by_photo().get(&h).cloned().unwrap_or_default();
        let mut others = Vec::new();
        for j in faces {
            if db.video_at(j) != frame {
                continue; // a video: only the faces of the same frame
            }
            let b = db.store.boxes[j];
            let place: Vec<i64> = b[..4].iter().map(|v| (*v as f64).round_ties_even() as i64).collect();
            let (maybe, shown) = (db.maybe(j), db.shown(j));
            others
                .push(json!({"i": j, "box": place, "name": db.name(j), "q": db.q(j), "maybe": maybe, "shown": shown}));
        }
        let here = path.as_deref().is_some_and(|p| Path::new(p).exists());
        let shown_path = path
            .clone()
            .or_else(|| photo.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string));
        let mut hints = db.hints(i);
        let web = self.inner.web_saved.lock().unwrap_or_else(|e| e.into_inner());
        hints["web"] = web.get(&db.store.fid[i]).cloned().unwrap_or(Value::Null);
        let mut info = json!({
            "i": i, "path": shown_path.unwrap_or_else(|| db.store.file[i].clone()), "video": frame, "exists": here,
            "remote": [],
            "date": path.as_deref().map_or_else(String::new, |p| photofiles::photo_date(Path::new(p))),
            "faces": others, "hints": hints,
        });
        drop(web);
        drop(db);
        // the other devices that have the photo: asked of the shared database, so not under the labeling lock
        if !here && self.inner.peer_ready() {
            info["remote"] = json!(self.inner.asker.holders(&h));
        }
        info
    }

    /// What the tracker's own process asks of the labeling tool's (`trackerlink`): a photo of this device for
    /// another one, by its content hash only. Never a page: only with this run's secret.
    fn local_get(&self, req: &Request) -> Response {
        use crate::trackerlink;
        if !trackerlink::allowed(&req.headers) {
            return Response::error(403, trackerlink::REFUSAL);
        }
        let frame = req.param("frame").filter(|f| !f.is_empty());
        let named = req.path.strip_prefix("/api/local/photo/").filter(|id| trackerlink::photo_id(id));
        let numbered = frame.as_ref().is_none_or(|f| f.len() <= 7 && f.bytes().all(|b| b.is_ascii_digit()));
        let (Some(id), true) = (named, numbered) else { return Response::error(404, "not found") };
        match self.own_photo(id, frame.and_then(|f| f.parse().ok())) {
            Some(jpeg) => Response::bytes("image/jpeg", jpeg),
            None => Response::error(404, "this device doesn't have that photo"),
        }
    }

    /// What this device shows another one: the photo view's JPEG (a video's frame by its number), if its
    /// records have the photo and the file is still the same (its size). A photo this device read itself has
    /// its size on record; one without is never shown, and a path is this device's own, never a row's.
    #[cfg(feature = "import")]
    fn own_photo(&self, id: &str, frame: Option<i64>) -> Option<Vec<u8>> {
        let (path, size, video) = {
            let db = self.inner.db();
            let rec = db.store.photos.get(id)?.as_object()?;
            let path = rec.get("path").and_then(Value::as_str).filter(|p| !p.is_empty())?.to_string();
            let size = rec.get("size").and_then(Value::as_u64).filter(|s| *s > 0)?;
            (path, size, rec.get("video").is_some_and(|v| v.as_bool() == Some(true)))
        };
        let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file())?;
        if meta.len() != size {
            return None; // changed since it was read: not that photo any more
        }
        let img = match frame.filter(|_| video) {
            Some(k) => video::frame(Path::new(&path), k).ok()?,
            None => decode::load_file(Path::new(&path)).ok()?,
        };
        img.to_jpeg(85).ok()
    }

    #[cfg(not(feature = "import"))]
    fn own_photo(&self, _: &str, _: Option<i64>) -> Option<Vec<u8>> {
        None
    }

    fn get_route(&self, req: &Request) -> Answer {
        if req.path.starts_with("/api/local/") {
            return Ok(self.local_get(req));
        }
        // what the Settings tab asks: its own module, and never under the labeling data's lock
        #[cfg(feature = "sync")]
        if let Some(answer) = self.settings_get(req) {
            return answer;
        }
        // Describe photo: outside the labeling data's lock, as it asks a server
        #[cfg(feature = "import")]
        if let Some(answer) = self.describe_get(req).or_else(|| self.libhistory_get(&req.path)) {
            return answer;
        }
        // the music log and a person's usual music (Settings)
        #[cfg(feature = "import")]
        if req.path == "/api/music" {
            return Ok(Response::json(self.music_status()));
        }
        // LM Studio on this computer, and the models it lacks (Settings); the downloads' progress
        #[cfg(feature = "import")]
        if req.path == "/api/lmstudio" {
            return Ok(Response::json(self.lmstudio_state()));
        }
        #[cfg(feature = "import")]
        if req.path == "/api/downloads" {
            return Ok(Response::json(json!({"items": crate::lmstudio::download().into_iter().collect::<Vec<_>>()})));
        }
        // LM Studio servers on the local network (Settings): a search of seconds, never under the labeling lock
        #[cfg(feature = "import")]
        if req.path == "/api/hardware/network" {
            return Ok(Response::json(json!({"servers": crate::lmsnet::lmstudio_network(&self.home)})));
        }
        // this computer, and where each job runs (Settings): asks the system and this computer's own ports
        #[cfg(feature = "import")]
        if req.path == "/api/hardware" {
            let devices = self.inner.importer.loaded().map(|faces| faces.devices()).unwrap_or_default();
            return Ok(Response::json(json!({
                "has": crate::hardware::describe(),
                "uses": crate::hardware::uses(&devices, &crate::trackerlink::devices()),
                "ai": crate::hardware::ai_apps(&self.home),
            })));
        }
        let path = req.path.as_str();
        let low = req.param("low").as_deref() == Some("1");
        let face = |rest: &str| -> Result<Option<usize>, Response> {
            let i = number(rest)?;
            Ok(usize::try_from(i).ok().filter(|i| *i < self.inner.db().n()))
        };
        if let Some(rest) = path.strip_prefix("/thumb/") {
            let Some(i) = face(rest.split('.').next().unwrap_or(""))? else {
                return Err(Response::error(404, "no such face"));
            };
            let picture = self.inner.db().store.thumbs[i].clone();
            return Ok(if picture.is_empty() {
                self.picture_soon(i);
                Response::bytes("image/svg+xml", NO_FACE.to_vec())
            } else {
                Response::bytes("image/jpeg", picture)
            });
        }
        #[cfg(feature = "import")]
        if let Some(rest) = path.strip_prefix("/photo/") {
            let i = face(rest.split('.').next().unwrap_or(""))?;
            return match i.and_then(|i| self.photo_jpeg(i)) {
                Some(jpg) => Ok(Response::bytes("image/jpeg", (*jpg).clone())),
                None => Err(Response::error(404, "photo not found")),
            };
        }
        // the possible faces the import didn't keep, for the photo view's dashed boxes (shown only)
        #[cfg(feature = "import")]
        if let Some(rest) = path.strip_prefix("/api/possible/") {
            let Some(i) = face(rest)? else { return Err(asked("no such face")) };
            return self
                .inner
                .importer
                .possible_faces(i)
                .map(|faces| Response::json(json!({"faces": faces})))
                .map_err(asked);
        }
        if path == "/api/import" {
            return Ok(Response::json(self.import_status()));
        }
        let import = if path == "/api/summary" { self.import_status() } else { Value::Null };
        if let Some(rest) = path.strip_prefix("/api/photoinfo/") {
            let Some(i) = face(rest)? else { return Err(asked("no such face")) };
            self.learn_soon();
            return Ok(Response::json(self.photo_info(i)));
        }
        if matches!(path, "/api/summary" | "/api/possiblefaces") {
            self.learn_soon();
        }
        let mut db = self.inner.db();
        Ok(Response::json(match path {
            "/api/settings" => {
                return self.store.page_state("label").map(Response::json).map_err(|e| Response::error(500, e));
            }
            "/api/summary" => {
                let mut s = db.summary();
                s["app"] = json!("face-labels");
                s["import"] = import;
                s
            }
            // the same picture again: {face: first face of its stack}
            "/api/stacks" => {
                let of: Map<String, Value> = {
                    let mut pairs: Vec<(usize, usize)> = db.stacks().iter().map(|(a, b)| (*a, *b)).collect();
                    pairs.sort_unstable();
                    pairs.into_iter().map(|(a, b)| (a.to_string(), json!(b))).collect()
                };
                json!({"n": db.n(), "of": of})
            }
            "/api/fids" => json!({"fids": db.store.fid}),
            "/api/suggested" => json!({"groups": db.suggested(low)}),
            "/api/unknown" => db.unknown(low, req.param("regroup").as_deref() == Some("1")),
            "/api/auto" => json!({"groups": db.auto_review()}),
            "/api/person" => {
                let name = req.param("name").unwrap_or_default();
                json!({"faces": db.person(&name), "name": name})
            }
            "/api/checks" => (*db.checks()).clone(),
            "/api/downloads" => json!({"items": []}),
            "/api/possiblefaces" => {
                let left = db.store.photos.values().filter(|p| photofiles::needs_possible(p)).count();
                json!({"faces": db.possible(), "left": left})
            }
            "/api/photos" => {
                let names = req.params("with");
                json!({"photos": db.photos_with(&names.iter().map(String::as_str).collect::<Vec<_>>())})
            }
            "/api/tagged" => json!({"groups": db.tagged(low)}),
            _ => return Err(Response::error(404, "not found")),
        }))
    }

    fn post_route(&self, req: &Request) -> Answer {
        // the tracker's own process saved an alert or a visit: the sync sends it now, not a minute later
        if req.path == "/api/local/saved" {
            if !crate::trackerlink::allowed(&req.headers) {
                return Err(Response::error(403, crate::trackerlink::REFUSAL));
            }
            #[cfg(feature = "sync")]
            if let Some(sync) = self.syncer() {
                sync.note();
            }
            return Ok(Response::json(json!({"ok": true})));
        }
        let body: Value = if req.body.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&req.body).map_err(|e| asked(e.to_string()))?
        };
        let Some(body) = body.as_object() else { return Err(asked("expected an object")) };
        let text = |k: &str| body.get(k).and_then(Value::as_str);
        #[cfg(feature = "sync")]
        if let Some(answer) = self.settings_post(&req.path, body) {
            return answer;
        }
        // everything held about one person, into a folder; or forget them everywhere (confirmed on the page)
        if req.path == "/api/person/export" {
            let done = crate::persondata::export(
                &self.inner,
                &self.data,
                &self.home,
                text("name").unwrap_or(""),
                text("folder").unwrap_or(""),
            );
            return match done {
                Ok(mut done) => {
                    done["ok"] = json!(true);
                    Ok(Response::json(done))
                }
                Err(Fault::Files(why)) => Err(asked(format!("Couldn't write into that folder: {why}"))),
                Err(other) => Err(fault(other)),
            };
        }
        if req.path == "/api/person/forget" {
            if body.get("confirm") != body.get("name") {
                return Err(asked("Type the person's name to confirm."));
            }
            let removed =
                crate::persondata::forget(&self.inner, &self.data, &self.home, text("name").unwrap_or(""), false)
                    .map_err(fault)?;
            return Ok(Response::json(json!({"ok": true, "removed": removed})));
        }
        if req.path == "/api/settings" {
            if text("app") != Some("label") {
                return Err(asked("bad settings"));
            }
            let changes = body.get("settings").filter(|s| !s.is_null()).cloned().unwrap_or(json!({}));
            return self.store.update("label", &changes).map(|s| Response::json(json!({"settings": s}))).map_err(asked);
        }
        #[cfg(feature = "import")]
        if let Some(answer) = self.import_post(&req.path, body).or_else(|| self.describe_post(&req.path, body)) {
            return answer;
        }
        // the library's part of the signal history (a job in the import's slot)
        #[cfg(feature = "import")]
        if let Some(answer) = self.libhistory_post(&req.path) {
            return answer;
        }
        // NVIDIA CUDA for the model runtime, when asked: this program's runtime is fetched with it, and has
        // none to add yet (the page offers the download only when the tool says there is one: it never does)
        if req.path == "/api/cuda/download" {
            return Err(asked(
                "This program has no CUDA support to download yet: its models run on the processor here.",
            ));
        }
        // the face to Amazon, its photo to Google: only when the owner clicks
        #[cfg(feature = "import")]
        if req.path == "/api/websearch" {
            return self.websearch_post(body);
        }
        #[cfg(feature = "import")]
        if let Some(answer) = self.lmstudio_post(&req.path, body).or_else(|| self.music_post(&req.path, body)) {
            return answer;
        }
        let mut db = self.inner.db();
        Ok(Response::json(match req.path.as_str() {
            "/api/act" => {
                let listed =
                    body.get("faces").and_then(Value::as_array).filter(|f| f.iter().all(|i| i.is_i64() || i.is_u64()));
                let Some(listed) = listed else { return Err(asked("faces must be a list of numbers")) };
                let mut faces = Vec::with_capacity(listed.len());
                for i in listed {
                    match i.as_u64() {
                        Some(i) => faces.push(i as usize),
                        None => return Err(asked(format!("no face {i}"))),
                    }
                }
                let action = body
                    .get("action")
                    .map_or_else(|| "None".to_string(), |a| a.as_str().map_or_else(|| a.to_string(), str::to_string));
                let n = db.act(&action, &faces, text("name"), None).map_err(fault)?;
                let different: std::collections::HashSet<&usize> = faces.iter().collect();
                // skipped: Ignored faces whose print and picture couldn't be made again
                json!({"ok": true, "changed": n, "skipped": different.len() - n, "undo": db.can_undo().unwrap_or("")})
            }
            "/api/rename" => {
                let n = db.rename(text("old").unwrap_or(""), text("new").unwrap_or("")).map_err(fault)?;
                json!({"ok": true, "changed": n})
            }
            "/api/undo" => json!({"ok": true, "undone": db.undo().map_err(fault)?}),
            // copy the photos with these people into a folder
            "/api/export" => {
                let names: Option<Vec<&str>> =
                    body.get("names").and_then(Value::as_array).and_then(|n| n.iter().map(Value::as_str).collect());
                let Some(names) = names.filter(|n| !n.is_empty()) else {
                    return Err(asked("names must be a list of names"));
                };
                let rows = db.photos_with(&names);
                drop(db); // copying is slow, and needs nothing of the library
                let mut done = LabelDb::export(&rows, text("folder").unwrap_or("")).map_err(|e| match e {
                    Fault::Files(why) => asked(format!("Couldn't copy into that folder: {why}")),
                    other => fault(other),
                })?;
                done["ok"] = json!(true);
                done
            }
            _ => return Err(Response::error(404, "not found")),
        }))
    }
}

/// Learn about the possible faces from a copy of the prints, without holding the library meanwhile.
fn learn(inner: &Inner) {
    let job = {
        let mut db = inner.db();
        db.faceness_due().then(|| db.faceness_job())
    };
    if let Some(job) = job {
        let learned = faceness::learn(job.prints(), &job.pos, &job.neg, &job.idx);
        inner.db().set_faceness(job.n, job.version_faces, learned);
    }
}

impl Service for Labeler {
    fn body_limit(&self, path: &str) -> Option<usize> {
        let import = cfg!(feature = "import")
            && matches!(
                path,
                "/api/import"
                    | "/api/import/stop"
                    | "/api/possible/scan"
                    | "/api/morefaces"
                    | "/api/drawface"
                    | "/api/history/library"
            );
        #[cfg(feature = "sync")]
        let import = import || crate::label_settings::POSTS.contains(&path);
        #[cfg(feature = "import")]
        let import = import || crate::describejob::POSTS.contains(&path) || path == "/api/websearch";
        #[cfg(feature = "import")]
        let import = import || crate::lmstudio::POSTS.contains(&path) || crate::musiclog::POSTS.contains(&path);
        let import = import || matches!(path, "/api/person/export" | "/api/person/forget" | "/api/cuda/download");
        let import = import || path == "/api/local/saved";
        (import || matches!(path, "/api/act" | "/api/rename" | "/api/undo" | "/api/export" | "/api/settings"))
            .then_some(MAX_BODY + 1)
    }

    fn get(&self, req: &Request) -> Response {
        if let Some((_, file, ctype)) = PAGES.iter().find(|(p, _, _)| *p == req.path) {
            return match crate::pagefiles::read(&self.pages, file) {
                // (from a named folder: read each time, an edit shows on refresh)
                Some(b) => Response::bytes(ctype, b),
                None => Response::error(404, "not found"),
            };
        }
        self.get_route(req).unwrap_or_else(|refusal| refusal)
    }

    fn post(&self, req: &Request) -> Response {
        self.post_route(req).unwrap_or_else(|refusal| refusal)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use face_core::image::Bgr;
    use face_library::store::Store;

    use super::*;

    /// A tool over a made-up library: 12 faces, three to a photo, of four people (face i is person i % 4).
    fn tool(name: &str) -> Labeler {
        let dir = std::env::temp_dir().join(format!("face-label-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // (and what earlier runs of this test left: a tool's threads outlive the test that made it, so its
        // folder can't be removed when the test ends)
        let old = |d: &std::fs::DirEntry| {
            let written = d.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            d.file_name().to_string_lossy().starts_with(&format!("face-label-{name}-"))
                && written.is_some_and(|t| t.as_secs() > 600)
        };
        for left in std::fs::read_dir(std::env::temp_dir()).into_iter().flatten().flatten().filter(old) {
            let _ = std::fs::remove_dir_all(left.path());
        }
        let model = REC_MODELS[0];
        let mut store = Store::load(&dir.join("data"), model.name, model.dim).unwrap();
        let crop = Bgr { w: 112, h: 112, px: vec![90; 112 * 112 * 3] };
        for face in 0..12usize {
            let mut print = vec![0f32; model.dim];
            (print[face % 4], print[10 + face]) = (0.9, (1.0f32 - 0.81).sqrt());
            let photo = format!("{:020x}", 0xabc0 + face / 3);
            store.photos.entry(photo.clone()).or_insert_with(|| json!({"name": format!("p{}.jpg", face / 3)}));
            store.add(&photo, "p.jpg", &[10.0 * face as f32, 5.0, 40.0, 40.0, 0.9], &print, &crop, 0.8, None).unwrap();
        }
        store.save().unwrap();
        Labeler::new(&dir, &dir.join("home"), &dir.join("pages"), &mut |_| {}).unwrap()
    }

    fn get(tool: &Labeler, address: &str) -> (u16, Value) {
        let (path, query) = address.split_once('?').unwrap_or((address, ""));
        let r = tool.get(&Request {
            path: path.to_string(),
            query: query.to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        });
        (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    fn post(tool: &Labeler, path: &str, body: Value) -> (u16, Value) {
        let r = tool.post(&Request {
            path: path.to_string(),
            query: String::new(),
            headers: HashMap::new(),
            body: body.to_string().into_bytes(),
        });
        (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    #[test]
    fn the_page_is_answered_its_lists_and_its_changes() {
        let tool = tool("routes");
        let (_, summary) = get(&tool, "/api/summary");
        assert_eq!(
            (&summary["app"], &summary["faces"], &summary["photos"], &summary["import"]),
            (&json!("face-labels"), &json!(12), &json!(4), &json!({"running": false}))
        );
        assert_eq!(get(&tool, "/api/fids").1["fids"].as_array().unwrap().len(), 12);
        // two of a person named: the third of them is suggested
        let (status, done) = post(&tool, "/api/act", json!({"action": "name", "faces": [0, 4, 4], "name": " Ann "}));
        assert_eq!((status, done), (200, json!({"ok": true, "changed": 2, "skipped": 0, "undo": "Named 2 faces Ann"})));
        assert_eq!(get(&tool, "/api/suggested").1, json!({"groups": [{"name": "Ann", "faces": [[8, 0.81, 0.8]]}]}));
        assert_eq!(
            get(&tool, "/api/person?name=Ann").1,
            json!({"faces": [[0, 0.81, 0.8, "me"], [4, 0.81, 0.8, "me"]], "name": "Ann"})
        );
        assert_eq!(get(&tool, "/api/photos?with=Ann&with=").1["photos"].as_array().unwrap().len(), 2);
        let (_, info) = get(&tool, "/api/photoinfo/4");
        assert_eq!(
            (&info["path"], &info["exists"], &info["faces"].as_array().unwrap().len(), &info["hints"]["web"]),
            (&json!("p1.jpg"), &json!(false), &3, &Value::Null)
        );
        assert_eq!(
            info["faces"][1],
            json!({"i": 4, "box": [40, 5, 40, 40], "name": "Ann", "q": 0.8, "maybe": null, "shown": true})
        );
        assert_eq!(
            post(&tool, "/api/rename", json!({"old": "Ann", "new": "Anne"})),
            (200, json!({"ok": true, "changed": 2}))
        );
        assert_eq!(post(&tool, "/api/undo", json!({})).1, json!({"ok": true, "undone": "Renamed Ann to Anne"}));
        assert_eq!(get(&tool, "/api/stacks").1, json!({"n": 12, "of": {}}));
        assert_eq!(get(&tool, "/api/possiblefaces").1, json!({"faces": [], "left": 0}));
        // a face's picture, and the stand-in for one without
        let picture = |i: usize| {
            tool.get(&Request {
                path: format!("/thumb/{i}.jpg"),
                query: String::new(),
                headers: HashMap::new(),
                body: Vec::new(),
            })
        };
        assert_eq!(picture(3).ctype, "image/jpeg");
        post(&tool, "/api/act", json!({"action": "ignore", "faces": [3]}));
        assert_eq!((picture(3).ctype, picture(3).body.as_slice()), ("image/svg+xml", NO_FACE));
        // the page's own settings, and nobody else's
        assert_eq!(
            post(&tool, "/api/settings", json!({"app": "label", "settings": {"fl-x": "1"}})).1,
            json!({"settings": {"fl-x": "1"}})
        );
        assert_eq!(
            post(&tool, "/api/settings", json!({"app": "label", "settings": {"ft-y": "2"}})),
            (400, json!({"error": "bad setting name"}))
        );
        assert_eq!(get(&tool, "/api/settings").1, json!({"settings": {"fl-x": "1"}}));
        // what is refused
        for (path, body, why) in [
            ("/api/act", json!({"action": "name", "faces": "x"}), "faces must be a list of numbers"),
            ("/api/act", json!({"action": "name", "faces": [0]}), "a name is needed"),
            ("/api/act", json!({"action": "ok", "faces": [-1]}), "no face -1"),
            ("/api/act", json!({"action": "ok", "faces": [12]}), "no face 12"),
            ("/api/act", json!({"faces": [0]}), "unknown action None"),
            ("/api/export", json!({"names": "Ann"}), "names must be a list of names"),
            ("/api/settings", json!({"app": "track"}), "bad settings"),
            ("/api/undo", json!([1]), "expected an object"),
        ] {
            assert_eq!(post(&tool, path, body), (400, json!({"error": why})), "{path}");
        }
        assert_eq!(get(&tool, "/thumb/12"), (404, json!({"error": "no such face"})));
        assert_eq!(get(&tool, "/thumb/x.jpg"), (400, json!({"error": "That is not a whole number: 'x'"})));
        assert_eq!(get(&tool, "/api/photoinfo/99"), (400, json!({"error": "no such face"})));
        // adding photos and the photo view's faces: only in a program built to read photos
        let import = cfg!(feature = "import");
        let no_photo = if import { "photo not found" } else { "not found" };
        assert_eq!((get(&tool, "/api/nothing").0, get(&tool, "/photo/0")), (404, (404, json!({"error": no_photo}))));
        assert_eq!(get(&tool, "/api/import"), (200, json!({"running": false})));
        assert_eq!(tool.body_limit("/api/act").is_some() && tool.body_limit("/api/import").is_some(), import);
        for (path, body, why) in [
            ("/api/import", json!({"folder": " \"nowhere at all\" "}), "Folder not found: nowhere at all"),
            ("/api/morefaces", json!({}), "no such face"),
            ("/api/morefaces", json!({"i": -1}), "no such face"),
            ("/api/morefaces", json!({"i": 0, "box": [1, 2, 3]}), "a box is four numbers"),
            ("/api/drawface", json!({"i": 99, "box": [0, 0, 40, 40]}), "no such face"),
            ("/api/drawface", json!({"i": 0}), "Draw a bigger box around the face."),
            ("/api/drawface", json!({"i": "0", "box": [0, 0, 8, 40]}), "Draw a bigger box around the face."),
        ] {
            let expected = if import { (400, json!({"error": why})) } else { (404, json!({"error": "not found"})) };
            assert_eq!(post(&tool, path, body), expected, "{path}");
        }
        if import {
            assert_eq!(post(&tool, "/api/import/stop", json!({})), (200, json!({"ok": true})));
            assert_eq!(get(&tool, "/api/possible/99"), (400, json!({"error": "no such face"})));
        }
    }

    /// Describe photo's routes that ask no server (those that do: `describe.rs`'s tests, with a server of their own).
    #[cfg(feature = "import")]
    #[test]
    fn describe_routes_refuse_what_they_cannot_do() {
        let tool = tool("describe");
        assert_eq!(get(&tool, "/api/describe/0"), (200, json!({"saved": null})));
        assert_eq!(get(&tool, "/api/describe/12"), (404, json!({"error": "no such face"})));
        assert_eq!(get(&tool, "/api/describe/one").0, 400);
        for body in [json!({}), json!({"i": 12}), json!({"i": "0"}), json!({"i": -1}), json!({"i": 1.5})] {
            assert_eq!(post(&tool, "/api/describe", body), (400, json!({"error": "no such face"})));
        }
        // the photos of this made-up library aren't on the computer
        assert_eq!(
            post(&tool, "/api/describe", json!({"i": 0})),
            (400, json!({"error": "The original photo isn't on this computer any more."}))
        );
        assert_eq!(
            post(&tool, "/api/describe/config", json!({"use": "something"})),
            (400, json!({"error": "Unknown choice"}))
        );
        assert_eq!(post(&tool, "/api/describe/config", json!({"url": "ftp://somewhere"})).0, 400);
        // (the built-in model's download starts in the background; in a test it reaches nothing: florence::publisher)
        assert_eq!(post(&tool, "/api/describe/getmodel", json!({})), (200, json!({"ok": true})));
        assert_eq!(post(&tool, "/api/cuda/download", json!({})).0, 400);
        assert!(tool.body_limit("/api/cuda/download").is_some());
        assert_eq!(post(&tool, "/api/describe/test", json!({"url": "http://"})).0, 400);
        // a description kept is what a photo's faces all answer with, and is answered without asking a server
        let key = crate::describejob::key(&tool.inner.db(), 4);
        face_home::descriptions::save(
            &tool.home,
            &key,
            &json!({"text": "A made-up description.", "model": "m", "secs": 1.5}),
            true,
        )
        .unwrap();
        let (code, kept) = post(&tool, "/api/describe", json!({"i": 5}));
        assert_eq!((code, &kept["text"], &kept["cached"]), (200, &json!("A made-up description."), &json!(true)));
        assert_eq!(get(&tool, "/api/describe/3").1["saved"]["model"], json!("m"));
        assert_eq!(get(&tool, "/api/describe/6").1, json!({"saved": null}), "another photo's face");
        for path in crate::describejob::POSTS {
            assert!(tool.body_limit(path).is_some(), "{path}");
        }
    }
}
