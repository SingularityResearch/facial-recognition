//! Adding photos and videos in the background: a folder is read,
//! photos already read and copies are skipped, faces and possible faces are found, and the faces the model is
//! sure of are named. One job at a time; files that can't be read are counted and the last few kept with the
//! reason. Also what the photo view asks for one photo: the possible faces not kept yet, adding one of them,
//! a face drawn by hand, and a face's picture made again from its photo.
//!
//! Each file's work that doesn't change the library (reading it once for its hash and its picture, decoding,
//! finding faces and possible faces) runs on several threads, a few files ahead of the one being added; the
//! results are added in the files' order, so the same folder always gives the same library.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use face_core::{
    faces::{Face, Faces, MIN_FACE, align},
    geometry::{ARC_TEMPLATE, iou},
    image::Bgr,
    runtime::{Choice, Runtime, find_library},
    yunet::{self, Row},
};
use face_files::filejson;
use face_library::{
    labels::{LabelDb, Remade},
    photofacts::{split, suffix},
    photofiles::{self, fingerprint, path_text, seconds, sha1_hex},
    relocate::{IMAGE_EXT, VIDEO_EXT, list_media},
    store::Store,
};
use serde_json::{Map, Value, json};

use crate::{decode, video};

/// files that couldn't be read: the last this many are kept to show why
pub const PROBLEMS_KEPT: usize = 20;
/// possible faces: the detector's confidence, far below what a face needs ...
pub const MORE_SCORE: f64 = 0.45;
/// ... at more sizes: the owner looks at what it finds and decides
pub const MORE_SIZES: [usize; 4] = [10_000, 640, 480, 320];
/// a face found again must cover its saved box this well
const REBUILD_IOU: f32 = 0.6;

type Result<T> = std::result::Result<T, String>;

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Photos an import works on at once: two threads are left for the page, the tracker and the system; at most
/// 12 (measured). `FACE_IMPORT_WORKERS` names another number.
pub fn import_workers() -> usize {
    if let Some(n) =
        std::env::var("FACE_IMPORT_WORKERS").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| *n > 0)
    {
        return n;
    }
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    threads.saturating_sub(2).clamp(2, 12)
}

fn is_video(name: &str) -> bool {
    VIDEO_EXT.contains(&suffix(&name.to_lowercase()).1)
}

/// `work(k)` for every k below `n` on `workers` threads, at most `ahead` of them past the one being taken;
/// `take(k, result)` on this thread, in order. Ends early when `take` says so.
fn in_order<T: Send>(
    n: usize,
    workers: usize,
    ahead: usize,
    work: impl Fn(usize) -> T + Sync,
    mut take: impl FnMut(usize, T) -> bool,
) {
    struct State<T> {
        next: usize,
        taken: usize,
        done: HashMap<usize, T>,
        stop: bool,
    }
    let state = Mutex::new(State { next: 0, taken: 0, done: HashMap::new(), stop: false });
    let turn = Condvar::new();
    let held = || state.lock().unwrap_or_else(|e| e.into_inner());
    std::thread::scope(|s| {
        for _ in 0..workers.max(1) {
            s.spawn(|| {
                loop {
                    let k = {
                        let mut st = held();
                        while !st.stop && st.next < n && st.next >= st.taken + ahead.max(1) {
                            st = turn.wait(st).unwrap_or_else(|e| e.into_inner());
                        }
                        if st.stop || st.next >= n {
                            return;
                        }
                        st.next += 1;
                        st.next - 1
                    };
                    let result = work(k);
                    held().done.insert(k, result);
                    turn.notify_all();
                }
            });
        }
        for k in 0..n {
            let result = {
                let mut st = held();
                loop {
                    if let Some(r) = st.done.remove(&k) {
                        st.taken = k + 1;
                        break r;
                    }
                    st = turn.wait(st).unwrap_or_else(|e| e.into_inner());
                }
            };
            turn.notify_all();
            if !take(k, result) {
                break;
            }
        }
        held().stop = true;
        turn.notify_all();
    });
}

/// What a worker found out about one file, without changing the library.
enum Loaded {
    /// it couldn't be read at all: why
    Failed(String),
    /// read: its content's hash, and what was made of it
    Read(String, Got),
}

enum Got {
    /// the library has this photo already: nothing more was done
    Known,
    /// its picture couldn't be decoded
    Unreadable,
    Video(Result<(f64, Vec<video::Found>)>),
    Photo {
        img: Bgr,
        fp: String,
        found: Vec<Face>,
        low: Vec<Row>,
    },
}

struct File {
    path: PathBuf,
    name: String,
    size: u64,
    mtime: i64,
}

type MoreKey = (String, Option<i64>);

pub struct Importer {
    db: Arc<Mutex<LabelDb>>,
    root: PathBuf,
    home: PathBuf,
    faces: Mutex<Option<Arc<Faces>>>,
    status: Mutex<Map<String, Value>>,
    stop: AtomicBool,
    busy: AtomicBool,
    /// the last few photos looked at for more faces: (photo, frame) -> its picture and what the detector saw
    more: Mutex<Vec<(MoreKey, Arc<Bgr>, Arc<Vec<Row>>)>>,
}

/// The face models of the app in `root`, loaded as the importer loads them (the same checks of where each
/// runs, kept in the private folder). For whoever needs them before there is an importer: a library whose
/// prints have to be made again is opened with them.
pub fn load_models(root: &Path, home: &Path) -> Result<Arc<Faces>> {
    let library = find_library(root).ok_or("The model runtime's library wasn't found (FACE_ORT_LIB names it).")?;
    let made_for = format!(
        "{} {} | runtime {} bytes",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        std::fs::metadata(&library).map_or(0, |m| m.len())
    );
    let runtime =
        Runtime::new(&library, Choice::from_env()).map_err(|e| e.0)?.keeping(&home.join("model-checks"), &made_for);
    Ok(Arc::new(Faces::load(&runtime, &root.join("models")).map_err(|e| e.0)?))
}

impl Importer {
    /// An importer for this library. `root` has `models/` and the model runtime, `home` is the private folder.
    pub fn new(db: Arc<Mutex<LabelDb>>, root: &Path, home: &Path) -> Arc<Importer> {
        let mut status = Map::new();
        status.insert("running".to_string(), json!(false));
        Arc::new(Importer {
            db,
            root: root.to_path_buf(),
            home: home.to_path_buf(),
            faces: Mutex::new(None),
            status: Mutex::new(status),
            stop: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            more: Mutex::new(Vec::new()),
        })
    }

    fn db(&self) -> MutexGuard<'_, LabelDb> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn st(&self) -> MutexGuard<'_, Map<String, Value>> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How far the job has got, for the page.
    pub fn status(&self) -> Value {
        Value::Object(self.st().clone())
    }

    /// Say something in the job's status (a job of `run_other`).
    pub fn set(&self, key: &str, value: Value) {
        self.st().insert(key.to_string(), value);
    }

    /// Something done with the job's whole status at once (several of its parts that belong together).
    pub fn with_status<R>(&self, f: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
        f(&mut self.st())
    }

    /// Count in the job's status.
    pub fn add(&self, key: &str, n: usize) {
        let mut st = self.st();
        let was = st.get(key).and_then(Value::as_u64).unwrap_or(0);
        st.insert(key.to_string(), json!(was + n as u64));
    }

    pub fn count(&self, key: &str) -> u64 {
        self.st().get(key).and_then(Value::as_u64).unwrap_or(0)
    }

    /// Count a file the job couldn't read, and keep the last few with the reason, so the page can say which
    /// files and why, not just how many.
    pub fn problem(&self, path: &Path, why: &str) {
        let mut st = self.st();
        let errors = st.get("errors").and_then(Value::as_u64).unwrap_or(0) + 1;
        st.insert("errors".to_string(), json!(errors));
        let why: String = why.chars().take(300).collect();
        let list = st.entry("problems").or_insert_with(|| json!([]));
        if let Some(list) = list.as_array_mut() {
            list.push(json!({"file": path.to_string_lossy(), "why": why}));
            let extra = list.len().saturating_sub(PROBLEMS_KEPT);
            list.drain(..extra);
        }
    }

    /// The face models, loaded when first needed and kept.
    pub fn models(&self) -> Result<Arc<Faces>> {
        let mut kept = self.faces.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(faces) = &*kept {
            return Ok(faces.clone());
        }
        let faces = load_models(&self.root, &self.home)?;
        *kept = Some(faces.clone());
        Ok(faces)
    }

    /// The face models when they are loaded already (never loads them: for saying where they run).
    pub fn loaded(&self) -> Option<Arc<Faces>> {
        self.faces.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Hand the importer models that are loaded already (the same ones the rest of the program uses).
    pub fn use_models(&self, faces: Arc<Faces>) {
        *self.faces.lock().unwrap_or_else(|e| e.into_inner()) = Some(faces);
    }

    /// Start a background job unless one is running.
    fn begin(self: &Arc<Self>, job: impl FnOnce(&Importer) + Send + 'static) -> Result<()> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err("An import is already running.".to_string());
        }
        self.stop.store(false, Ordering::SeqCst);
        let me = self.clone();
        std::thread::Builder::new()
            .name("import".into())
            .spawn(move || {
                job(&me);
                me.busy.store(false, Ordering::SeqCst);
            })
            .map(|_| ())
            .map_err(|e| {
                self.busy.store(false, Ordering::SeqCst);
                e.to_string()
            })
    }

    /// Whether a job is running.
    pub fn running(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Ask the running job to stop after the file it is on.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Whether the running job was asked to stop.
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Read a folder of photos and videos, in the background.
    pub fn start(self: &Arc<Self>, folder: &str) -> Result<()> {
        let folder = PathBuf::from(path_text(face_library::photofacts::strip(folder).trim_matches('"')));
        if !folder.is_dir() {
            return Err(format!("Folder not found: {}", folder.display()));
        }
        self.begin(move |me| me.run(&folder))
    }

    /// Another background job in the import's one slot, with its status, its Stop and its end (Describe all:
    /// the page's progress bar and Stop button are the import's own). `folder` and `stage` are what the status
    /// says at its start; an error the job returns is the status's.
    pub fn run_other(
        self: &Arc<Self>,
        folder: &str,
        stage: &str,
        job: impl FnOnce(&Importer) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        let (folder, stage) = (folder.to_string(), stage.to_string());
        self.begin(move |me| {
            me.begin_status(&folder, &stage);
            let failed = job(me).err();
            me.end_status(failed);
        })
    }

    /// The same, here and now (for a check, or a program that only imports).
    pub fn run_now(&self, folder: &Path) {
        self.run(folder)
    }

    fn begin_status(&self, folder: &str, stage: &str) {
        let started = now();
        *self.st() = json!({
            "running": true, "folder": folder, "stage": stage, "total": 0, "done": 0, "new": 0, "known": 0,
            "duplicates": 0, "faces": 0, "auto": 0, "errors": 0, "started": started, "error": null,
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
    }

    fn end_status(&self, failed: Option<String>) {
        let mut st = self.st();
        let stage = match &failed {
            Some(_) => "Failed",
            None if self.stopped() => "Stopped",
            None => "Done",
        };
        st.insert("stage".to_string(), json!(stage));
        if let Some(why) = failed {
            st.insert("error".to_string(), json!(why));
        }
        st.insert("running".to_string(), json!(false));
        let started = st.get("started").and_then(Value::as_f64).unwrap_or_else(now);
        st.insert("elapsed".to_string(), json!(filejson::round(now() - started, 1)));
    }

    /// Every face the detector sees with the low cut-off, at several sizes: best first, small ones and the
    /// same face found twice left out.
    fn low_rows(&self, faces: &Faces, img: &Bgr) -> Result<Vec<Row>> {
        let pad = faces.pad();
        let mut rows = Vec::new();
        for side in MORE_SIZES {
            rows.extend(faces.detect_at(img, side, MORE_SCORE, pad).map_err(|e| e.0)?);
        }
        rows.sort_by(|a, b| b[14].total_cmp(&a[14]));
        let mut kept: Vec<Row> = Vec::new();
        for r in rows {
            if r[2].min(r[3]) >= MIN_FACE && kept.iter().all(|k| iou(&r, k) < 0.3) {
                kept.push(r);
            }
        }
        Ok(kept)
    }

    /// Add rows as possible faces of photo `h` and mark the photo as looked at. Their places; the caller saves.
    fn add_possible(&self, faces: &Faces, h: &str, img: &Bgr, rows: &[Row], frame: Option<i64>) -> Result<Vec<usize>> {
        let mut new = Vec::new();
        for row in rows {
            let crop = align(img, row).map_err(|e| e.0)?;
            let (emb, q) = (faces.embed(&crop).map_err(|e| e.0)?, faces.quality(&crop).map_err(|e| e.0)?);
            let mut db = self.db();
            let name = match db.store.photos.get(h).and_then(Value::as_object) {
                Some(rec) => match rec.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                    Some(name) => name.to_string(),
                    None => split(rec.get("path").and_then(Value::as_str).unwrap_or("")).1.to_string(),
                },
                None => String::new(),
            };
            let j = db.store.add(h, &name, row, &emb, &crop, q, frame)?;
            let fid = db.store.fid[j].clone();
            if let Some(rec) = db.store.photos.get_mut(h).and_then(Value::as_object_mut) {
                let maybe = rec.entry("maybe").or_insert_with(|| json!({}));
                maybe[&fid] = json!(filejson::round(row[14] as f64, 3));
                if let Some(frame) = frame {
                    rec.entry("at").or_insert_with(|| json!({}))[&fid] = json!(frame);
                }
            }
            new.push(j);
        }
        let mut db = self.db();
        if let Some(rec) = db.store.photos.get_mut(h).and_then(Value::as_object_mut) {
            rec.entry("maybe").or_insert_with(|| json!({}));
            let had = rec.get("faces").and_then(Value::as_u64).unwrap_or(0);
            rec.insert("faces".to_string(), json!(had + new.len() as u64));
        }
        if !new.is_empty() {
            db.regroup_next(); // the unknown faces are grouped anew with the new ones
        }
        db.changed();
        Ok(new)
    }

    /// Save the library and name the faces added since the last time that the model is sure of.
    fn checkpoint(&self, pending: &mut Vec<usize>) -> Result<()> {
        let mut db = self.db();
        db.save_store().map_err(|e| e.to_string())?;
        if !pending.is_empty() {
            let named = db.autoname(Some(pending)).map_err(|e| e.to_string())?;
            drop(db);
            self.add("auto", named);
            pending.clear();
        }
        Ok(())
    }

    fn run(&self, folder: &Path) {
        self.begin_status(&folder.to_string_lossy(), "Loading the face models");
        let (mut pending, mut added) = (Vec::new(), Vec::new());
        let pinned: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
        let done = self.read_folder(folder, &mut pending, &mut added, &pinned);
        let saved = self.checkpoint(&mut pending);
        let pinned = pinned.into_inner().unwrap_or_default();
        photofiles::unpin(&pinned.iter().map(PathBuf::as_path).collect::<Vec<_>>());
        {
            let mut db = self.db();
            db.regroup_next();
            db.changed();
        }
        if !added.is_empty() {
            // the new photos' camera dates and people tags: what to read is taken under the lock, read
            // without it, taken in under it
            let (jobs, listings) = {
                let mut db = self.db();
                (db.facts_jobs(Some(&added)), db.facts.listings.clone())
            };
            let read = listings.run(&jobs);
            let _ = self.db().facts_apply(&jobs, read);
        }
        self.end_status(done.err().or(saved.err()));
    }

    fn read_folder(
        &self,
        folder: &Path,
        pending: &mut Vec<usize>,
        added: &mut Vec<String>,
        pinned: &Mutex<Vec<PathBuf>>,
    ) -> Result<()> {
        let faces = self.models()?;
        let device = faces.devices().into_iter().find(|(job, _)| *job == "recognition").and_then(|(_, d)| d);
        self.set("device", json!(device));
        self.set("stage", json!("Listing photos"));
        let both: Vec<&str> = IMAGE_EXT.iter().chain(&VIDEO_EXT).copied().collect();
        let listed = list_media(folder, &both);
        // A Live Photo's short clip sits beside its photo under the same name: the photo is enough.
        let key = |p: &Path| {
            let text = p.to_string_lossy().to_string();
            let (dir, name) = split(&text);
            (dir.to_string(), suffix(name).0.to_lowercase(), name.to_string())
        };
        let stills: HashSet<(String, String)> = listed
            .iter()
            .map(|p| key(p))
            .filter(|(_, _, name)| !is_video(name))
            .map(|(dir, stem, _)| (dir, stem))
            .collect();
        let listed: Vec<PathBuf> = (listed.into_iter())
            .filter(|p| {
                let (dir, stem, name) = key(p);
                !(is_video(&name) && stills.contains(&(dir, stem)))
            })
            .collect();
        self.set("total", json!(listed.len()));
        self.set("videos", json!(listed.iter().filter(|p| is_video(&p.to_string_lossy())).count()));
        // what the library has of these files already, by path, size and time; and videos by name and size
        // (a copy isn't read again: an online-only one would be downloaded just to find it's the same)
        let (by_path, seen_videos) = {
            let db = self.db();
            let mut by_path: HashMap<String, (Option<u64>, Option<i64>)> = HashMap::new();
            let mut seen: HashSet<(String, Option<u64>)> = HashSet::new();
            for p in db.store.photos.values().filter_map(Value::as_object) {
                let Some(path) = p.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()) else { continue };
                let size = p.get("size").and_then(Value::as_u64);
                by_path.insert(path.to_string(), (size, p.get("mtime").and_then(Value::as_i64)));
                if p.get("video").is_some_and(|v| v.as_bool() == Some(true)) {
                    seen.insert((split(path).1.to_lowercase(), size));
                }
            }
            (by_path, seen)
        };
        let mut todo = Vec::new();
        for path in listed {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let st = match std::fs::metadata(&path) {
                Ok(st) => st,
                Err(e) => {
                    self.problem(&path, &e.to_string());
                    continue;
                }
            };
            let (size, mtime) = (st.len(), st.modified().map_or(0.0, seconds).trunc() as i64);
            let known =
                by_path.get(&*path.to_string_lossy()).is_some_and(|(s, m)| *s == Some(size) && *m == Some(mtime));
            if known || (is_video(&name) && seen_videos.contains(&(name.to_lowercase(), Some(size)))) {
                self.add("known", 1);
                self.add("done", 1);
            } else {
                todo.push(File { path, name, size, mtime });
            }
        }
        {
            let data = self.db().data().to_path_buf();
            photofiles::remember_folder(&data, &folder.to_string_lossy())
                .map_err(|e| format!("can't write settings.json: {e}"))?;
        }
        self.set("stage", json!("Reading photos"));
        let workers = import_workers();
        self.set("workers", json!(workers));

        // on a worker: everything about one file that doesn't change the library
        let load = |k: usize| -> Loaded {
            let f = &todo[k];
            match photofiles::make_local(&f.path) {
                Ok(true) => pinned.lock().unwrap_or_else(|e| e.into_inner()).push(f.path.clone()),
                Ok(false) => {}
                Err(e) => return Loaded::Failed(e.to_string()),
            }
            let known = |h: &str| self.db().store.photos.contains_key(h);
            if is_video(&f.name) {
                return match photofiles::file_hash(&f.path) {
                    Err(e) => Loaded::Failed(e.to_string()),
                    Ok(h) if known(&h) => Loaded::Read(h, Got::Known),
                    Ok(h) => Loaded::Read(h, Got::Video(video::faces(&faces, &f.path, &self.stop))),
                };
            }
            let data = match std::fs::read(&f.path) {
                Ok(data) => data,
                Err(e) => return Loaded::Failed(e.to_string()),
            };
            if data.len() as u64 != f.size {
                // e.g. an online-only cloud file that wasn't all downloaded
                return Loaded::Failed(format!("read only {} of {} bytes of {}", data.len(), f.size, f.name));
            }
            let h = sha1_hex(&data);
            if known(&h) {
                return Loaded::Read(h, Got::Known);
            }
            let Ok(img) = decode::load(&data) else { return Loaded::Read(h, Got::Unreadable) };
            let found = match faces.detect(&img, &[10_000, 640]) {
                Ok(found) => found,
                Err(e) => return Loaded::Failed(e.0),
            };
            let low = match self.low_rows(&faces, &img) {
                Ok(low) => low.into_iter().filter(|r| found.iter().all(|k| iou(r, &k.row) < 0.3)).collect(),
                Err(e) => return Loaded::Failed(e),
            };
            Loaded::Read(h, Got::Photo { fp: fingerprint(&img), img, found, low })
        };

        let mut failed = None;
        in_order(todo.len(), workers, 2 * workers, load, |k, got| {
            if self.stopped() {
                return false;
            }
            let f = &todo[k];
            let path = f.path.to_string_lossy().to_string();
            let meta = |more: Value| {
                let mut rec =
                    json!({"path": path, "size": f.size, "mtime": f.mtime}).as_object().cloned().unwrap_or_default();
                rec.extend(more.as_object().cloned().unwrap_or_default());
                Value::Object(rec)
            };
            let step = (|| -> Result<()> {
                let (h, got) = match got {
                    Loaded::Failed(why) => {
                        self.problem(&f.path, &why);
                        return Ok(());
                    }
                    Loaded::Read(h, got) => (h, got),
                };
                // the library has this photo, from before or from a file earlier in this very folder
                let has = self.db().store.photos.contains_key(&h);
                match got {
                    Got::Unreadable if !has => {
                        self.problem(&f.path, "couldn't read the picture (damaged, or a format it can't open)")
                    }
                    Got::Known | Got::Unreadable | Got::Video(_) | Got::Photo { .. } if has => {
                        let mut db = self.db();
                        let old = db.store.photos.get_mut(&h).and_then(Value::as_object_mut);
                        let here = old
                            .as_ref()
                            .and_then(|o| o.get("path"))
                            .and_then(Value::as_str)
                            .is_some_and(|p| !p.is_empty() && Path::new(p).exists());
                        match old {
                            Some(old) if !here => {
                                // the same photo, moved or renamed
                                old.insert("path".to_string(), json!(path));
                                old.insert("size".to_string(), json!(f.size));
                                old.insert("mtime".to_string(), json!(f.mtime));
                                drop(db);
                                self.add("known", 1);
                            }
                            _ => {
                                drop(db);
                                self.add("duplicates", 1);
                            }
                        }
                    }
                    Got::Known | Got::Unreadable => {}
                    Got::Video(read) => match read {
                        Err(why) => self.problem(&f.path, &why),
                        Ok((fps, found)) => {
                            let mut db = self.db();
                            let mut at = Map::new();
                            for v in &found {
                                let j = db.store.add(&h, &f.name, &v.row, &v.emb, &v.crop, v.quality, Some(v.frame))?;
                                pending.push(j);
                                at.insert(db.store.fid[j].clone(), json!(v.frame));
                            }
                            let rec = meta(
                                json!({"fp": null, "faces": found.len(), "name": f.name, "video": true, "fps": filejson::round(fps, 3), "at": at}),
                            );
                            db.store.photos.insert(h, rec);
                            db.changed();
                            drop(db);
                            self.add("new", 1);
                            self.add("faces", found.len());
                        }
                    },
                    Got::Photo { img, fp, found, low } => {
                        let mut db = self.db();
                        let prints: Vec<f32> = found.iter().flat_map(|face| face.emb.iter().copied()).collect();
                        let looks_like = db.near_duplicate(&fp);
                        let copy_of = looks_like.filter(|dup| db.same_faces(dup, &prints, 0.90));
                        if let Some(dup) = copy_of {
                            db.store
                                .photos
                                .insert(h, meta(json!({"fp": fp, "faces": 0, "dup_of": dup, "name": f.name})));
                            drop(db);
                            self.add("duplicates", 1);
                        } else {
                            for face in &found {
                                pending.push(db.store.add(
                                    &h,
                                    &f.name,
                                    &face.row,
                                    &face.emb,
                                    &face.crop,
                                    face.quality,
                                    None,
                                )?);
                            }
                            db.store
                                .photos
                                .insert(h.clone(), meta(json!({"fp": fp, "faces": found.len(), "name": f.name})));
                            db.changed();
                            drop(db);
                            // and the possible faces: what the detector saw with the low cut-off too
                            let more = self.add_possible(&faces, &h, &img, &low, None)?;
                            if !found.is_empty() || !more.is_empty() {
                                added.push(h);
                            }
                            self.add("new", 1);
                            self.add("faces", found.len() + more.len());
                            pending.extend(more);
                        }
                    }
                }
                Ok(())
            })();
            if let Err(why) = step {
                failed = Some(why);
                return false;
            }
            self.add("done", 1);
            let new = self.count("new");
            if new > 0 && new % 50 == 0 && !pending.is_empty() {
                if let Err(why) = self.checkpoint(pending) {
                    failed = Some(why);
                    return false;
                }
            }
            true
        });
        failed.map_or(Ok(()), Err)
    }

    /// Look for possible faces in every photo added before possible faces were kept, in the background.
    /// Videos are left out, and files that aren't on this computer now (those another time).
    pub fn scan_possible(self: &Arc<Self>) -> Result<()> {
        self.begin(|me| {
            me.begin_status("possible faces", "Looking for possible faces");
            let mut pending = Vec::new();
            let done = me.scan(&mut pending);
            let saved = me.checkpoint(&mut pending);
            me.end_status(done.err().or(saved.err()));
        })
    }

    fn scan(&self, pending: &mut Vec<usize>) -> Result<()> {
        let faces = self.models()?;
        let todo: Vec<(String, String)> = (self.db().store.photos.iter())
            .filter(|(_, p)| photofiles::needs_possible(p))
            .map(|(h, p)| (h.clone(), p["path"].as_str().unwrap_or("").to_string()))
            .collect();
        self.set("total", json!(todo.len()));
        for (k, (h, path)) in todo.iter().enumerate() {
            if self.stopped() {
                break;
            }
            let file = Path::new(path);
            if !file.exists() || photofiles::online_only(file) {
                self.add("known", 1); // not on this computer now: another time
            } else {
                let step = (|| -> Result<usize> {
                    let img = decode::load_file(file).map_err(|_| "unreadable".to_string())?;
                    let have = self.boxes_of(h, None);
                    let rows: Vec<Row> = self
                        .low_rows(&faces, &img)?
                        .into_iter()
                        .filter(|r| have.iter().all(|b| iou(r, b) < 0.3))
                        .collect();
                    let new = self.add_possible(&faces, h, &img, &rows, None)?;
                    pending.extend(&new);
                    Ok(new.len())
                })();
                match step {
                    Ok(n) => {
                        self.add("faces", n);
                        self.add("new", usize::from(n > 0));
                    }
                    Err(why) => self.problem(file, &why),
                }
            }
            self.add("done", 1);
            if (k + 1) % 100 == 0 {
                self.checkpoint(pending)?;
            }
        }
        Ok(())
    }

    /// The boxes of a photo's faces (of a video: of the faces of one frame).
    fn boxes_of(&self, h: &str, frame: Option<Option<i64>>) -> Vec<[f32; 4]> {
        let mut db = self.db();
        let faces: Vec<usize> = db.by_photo().get(h).cloned().unwrap_or_default();
        let rec = db.store.photos.get(h).and_then(Value::as_object);
        let video = rec.is_some_and(|r| r.get("video").is_some_and(|v| v.as_bool() == Some(true)));
        let at = |j: usize| rec.and_then(|r| r.get("at")).and_then(|a| a.get(&db.store.fid[j])).and_then(Value::as_i64);
        (faces.into_iter())
            .filter(|j| !video || frame.is_none_or(|f| at(*j) == f))
            .map(|j| {
                let b = db.store.boxes[j];
                [b[0], b[1], b[2], b[3]]
            })
            .collect()
    }

    /// The photo (or video frame) that face `i` is in: its hash, frame, picture, and the possible faces the
    /// detector sees there that aren't faces of it yet.
    fn possible(&self, i: usize) -> Result<(String, Option<i64>, Arc<Bgr>, Vec<Row>)> {
        let (h, path, frame) = {
            let db = self.db();
            let h = db.store.photo.get(i).cloned().ok_or("no such face")?;
            let rec = db.store.photos.get(&h).and_then(Value::as_object);
            let video = rec.is_some_and(|r| r.get("video").is_some_and(|v| v.as_bool() == Some(true)));
            let frame = rec
                .filter(|_| video)
                .and_then(|r| r.get("at"))
                .and_then(|a| a.get(&db.store.fid[i]))
                .and_then(Value::as_i64);
            (h.clone(), rec.and_then(|r| r.get("path")).and_then(Value::as_str).unwrap_or("").to_string(), frame)
        };
        let have = self.boxes_of(&h, Some(frame));
        if path.is_empty() || !Path::new(&path).exists() {
            return Err("The original photo isn't on this computer any more.".to_string());
        }
        let key = (h.clone(), frame);
        let cached = self
            .more
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(k, ..)| *k == key)
            .map(|(_, img, rows)| (img.clone(), rows.clone()));
        let (img, rows) = match cached {
            Some(hit) => hit,
            None => {
                let img = match frame {
                    None => decode::load_file(Path::new(&path)),
                    Some(k) => video::frame(Path::new(&path), k),
                }
                .map_err(|_| "Couldn't read the photo.".to_string())?;
                let rows = self.low_rows(&*self.models()?, &img)?;
                let (img, rows) = (Arc::new(img), Arc::new(rows));
                let mut more = self.more.lock().unwrap_or_else(|e| e.into_inner());
                more.push((key, img.clone(), rows.clone()));
                // a few photos: stepping back and forth stays quick
                let extra = more.len().saturating_sub(4);
                more.drain(..extra);
                (img, rows)
            }
        };
        let rows = rows.iter().filter(|r| have.iter().all(|k| iou(*r, k) < 0.3)).copied().collect();
        Ok((h, frame, img, rows))
    }

    /// [{box: [x, y, w, h], score}] for the photo view's dashed boxes (nothing is saved): only those that
    /// look enough like a real face, like the saved possible faces.
    pub fn possible_faces(&self, i: usize) -> Result<Value> {
        let (_, _, img, mut rows) = self.possible(i)?;
        if !rows.is_empty() {
            let faces = self.models()?;
            let mut scores = Vec::new();
            for r in &rows {
                let crop = align(&img, r).map_err(|e| e.0)?;
                scores.push(faces.embed(&crop).map_err(|e| e.0)?);
            }
            let db = self.db();
            let cut = db.face_cut();
            let keep: Vec<bool> = scores.iter().map(|print| db.faceness(print).is_none_or(|v| v >= cut)).collect();
            let mut keep = keep.into_iter();
            rows.retain(|_| keep.next().unwrap_or(true));
        }
        let boxed = |r: &Row| {
            let place: Vec<f64> = r[..4].iter().map(|v| filejson::round(*v as f64, 1)).collect();
            json!({"box": place, "score": filejson::round(r[14] as f64, 2)})
        };
        Ok(Value::Array(rows.iter().map(boxed).collect()))
    }

    /// Add the possible face at `at` (as `possible_faces` gave it), or all of them, to the photo of face `i`,
    /// ready to name. The new faces' places.
    pub fn find_more(&self, i: usize, at: Option<[f32; 4]>) -> Result<Vec<usize>> {
        let (h, frame, img, mut rows) = self.possible(i)?;
        if let Some(at) = at {
            rows = rows.into_iter().filter(|r| iou(r, &at) > 0.8).take(1).collect();
        }
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let new = self.add_possible(&*self.models()?, &h, &img, &rows, frame)?;
        if !new.is_empty() {
            self.db().save_store().map_err(|e| e.to_string())?;
        }
        Ok(new)
    }

    /// The detector's best face inside a box (looked at enlarged, with almost no cut-off), or none.
    fn points_in(&self, faces: &Faces, img: &Bgr, x: f32, y: f32, w: f32, h: f32) -> Option<Row> {
        let pad = 0.4 * w.max(h);
        let (x0, y0) = ((x - pad).max(0.0) as usize, (y - pad).max(0.0) as usize);
        let (x1, y1) = (((x + w + pad) as usize).min(img.w), ((y + h + pad) as usize).min(img.h));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let region = cut(img, x0, y0, x1, y1);
        let z = 320.0 / region.w.max(region.h) as f64;
        let wanted = |v: usize| (v as f64 * z).round_ties_even().max(1.0) as usize;
        let region =
            if z > 1.0 { region.resize_cubic(z, z) } else { region.resize_area(wanted(region.w), wanted(region.h)) };
        // (padded as the detector's own model pads: see where a face's picture is made again, below)
        let found = faces.detect_at(&region, 10_000, 0.02, yunet::MODEL_PAD).ok()?;
        let (mut best, mut best_iou) = (None, 0.3f32);
        for mut r in found {
            for v in &mut r[..14] {
                *v /= z as f32;
            }
            for k in [0, 4, 6, 8, 10, 12] {
                r[k] += x0 as f32; // x and the 5 points' x (2 and 3 are width and height)
                r[k + 1] += y0 as f32;
            }
            let k = iou(&r, &[x, y, w, h]);
            if k > best_iou {
                (best, best_iou) = (Some(r), k);
            }
        }
        best
    }

    /// Add a face drawn by hand ([x, y, w, h] in the photo's pixels, as the photo view shows it) to the photo
    /// (or video frame) of face `i`, for faces the detector misses even with the low cut-off. Its eyes, nose
    /// and mouth come from the detector looking just inside the box with almost no cut-off; failing that, from
    /// where they sit in an average face. Kept as a possible face (score 0: drawn). Its place.
    pub fn add_drawn(&self, i: usize, at: [f32; 4]) -> Result<usize> {
        let [x, y, w, h] = at;
        if w < 16.0 || h < 16.0 {
            return Err("Draw a bigger box around the face.".to_string());
        }
        let (photo, path, frame) = {
            let db = self.db();
            let photo = db.store.photo.get(i).cloned().ok_or("no such face")?;
            let rec = db.store.photos.get(&photo).and_then(Value::as_object);
            let video = rec.is_some_and(|r| r.get("video").is_some_and(|v| v.as_bool() == Some(true)));
            let frame = rec
                .filter(|_| video)
                .and_then(|r| r.get("at"))
                .and_then(|a| a.get(&db.store.fid[i]))
                .and_then(Value::as_i64);
            (photo.clone(), rec.and_then(|r| r.get("path")).and_then(Value::as_str).unwrap_or("").to_string(), frame)
        };
        // the share of the drawn box inside a face that is there
        let covered = |b: &[f32; 4]| {
            let ix = ((x + w).min(b[0] + b[2]) - x.max(b[0])).max(0.0);
            let iy = ((y + h).min(b[1] + b[3]) - y.max(b[1])).max(0.0);
            ix * iy / (w * h)
        };
        if self.boxes_of(&photo, Some(frame)).iter().any(|b| covered(b) > 0.6) {
            return Err("There's already a face there.".to_string());
        }
        if path.is_empty() || !Path::new(&path).exists() {
            return Err("The original photo isn't on this computer any more.".to_string());
        }
        let img = match frame {
            None => decode::load_file(Path::new(&path)),
            Some(k) => video::frame(Path::new(&path), k),
        }
        .map_err(|_| "Couldn't read the photo.".to_string())?;
        let (x, y) = (x.max(0.0), y.max(0.0));
        let (w, h) = (w.min(img.w as f32 - x), h.min(img.h as f32 - y));
        let faces = self.models()?;
        let mut row: Row = [0.0; 15];
        row[..4].copy_from_slice(&[x, y, w, h]);
        match self.points_in(&faces, &img, x, y, w, h) {
            // the owner's box, the detector's points
            Some(found) => row[4..14].copy_from_slice(&found[4..14]),
            None => {
                // an average face's points, scaled to the box
                let (side, cx, cy) = (w.max(h) as f64, (x + w / 2.0) as f64, (y + h / 2.0) as f64);
                for (k, p) in ARC_TEMPLATE.iter().enumerate() {
                    row[4 + 2 * k] = ((p[0] as f64 / 112.0 - 0.5) * side + cx) as f32;
                    row[5 + 2 * k] = ((p[1] as f64 / 112.0 - 0.5) * side + cy) as f32;
                }
            }
        }
        let new = self.add_possible(&faces, &photo, &img, &[row], frame)?;
        self.db().save_store().map_err(|e| e.to_string())?;
        new.first().copied().ok_or_else(|| "The face couldn't be added.".to_string())
    }

    /// Face `i`'s aligned picture made again from its photo (or video frame), for a face whose picture isn't
    /// on this computer or was deleted: the face is found again near its saved box (the box alone has no eyes
    /// and mouth to line it up with). None if the photo isn't here or the face isn't found again.
    pub fn rebuild_crop(faces: &Faces, store: &Store, i: usize) -> Option<Bgr> {
        Importer::crop_from(faces, &Importer::crop_source(store, i)?)
    }

    /// Where face `i`'s picture would be made again from: its photo's file, the frame of a video, and its
    /// box. None when the photo isn't on this computer. (What `rebuild_crop` needs of the library, so that
    /// the reading and looking can be done with the library let go of.)
    pub fn crop_source(store: &Store, i: usize) -> Option<CropSource> {
        let rec = store.photos.get(store.photo.get(i)?)?.as_object()?;
        let path = rec.get("path").and_then(Value::as_str).filter(|p| !p.is_empty() && Path::new(p).exists())?;
        let video = rec.get("video").is_some_and(|v| v.as_bool() == Some(true));
        let frame = rec.get("at").filter(|_| video).and_then(|a| a.get(&store.fid[i])).and_then(Value::as_i64);
        Some(CropSource { fid: store.fid[i].clone(), path: PathBuf::from(path), frame, place: store.boxes[i] })
    }

    /// The face's aligned picture from where `crop_source` said.
    pub fn crop_from(faces: &Faces, from: &CropSource) -> Option<Bgr> {
        let img = match from.frame {
            None => decode::load_file(&from.path).ok()?,
            Some(k) => video::frame(&from.path, k).ok()?,
        };
        let b = from.place;
        let (x, y, w, h) = (b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64);
        let m = w.max(h) * 0.6; // look in the box with some room around it
        let (x0, y0) = ((x - m).max(0.0) as usize, (y - m).max(0.0) as usize);
        let (x1, y1) = (((x + w + m) as usize).min(img.w), ((y + h + m) as usize).min(img.h));
        if x1 < x0 + 8 || y1 < y0 + 8 {
            return None;
        }
        let part = cut(&img, x0, y0, x1, y1);
        // small faces: enlarged to find them
        let s = if part.w.max(part.h) >= 160 { 1.0 } else { 160.0 / part.w.max(part.h) as f64 };
        let part = if s != 1.0 { part.resize_cubic(s, s) } else { part };
        // The part is padded to the next multiple of 32, the model's own stride, as every library's faces
        // were when their pictures were made. Padded as whole photos are for the graphics processor, a face that
        // fills most of the part comes out with its eyes and mouth a third of a pixel elsewhere (measured in
        // both ways on the same faces), and the picture with them.
        let rows = faces.detect_at(&part, 10_000, 0.3, yunet::MODEL_PAD).ok()?;
        let (bx, by, bw, bh) =
            (((x - x0 as f64) * s) as f32, ((y - y0 as f64) * s) as f32, (w * s) as f32, (h * s) as f32);
        let overlap = |r: &Row| {
            let iw = ((bx + bw).min(r[0] + r[2]) - bx.max(r[0])).max(0.0);
            let ih = ((by + bh).min(r[1] + r[3]) - by.max(r[1])).max(0.0);
            iw * ih / (bw * bh + r[2] * r[3] - iw * ih)
        };
        let mut best: Option<Row> = None;
        for r in rows {
            if best.is_none_or(|b| overlap(&r) > overlap(&b)) {
                best = Some(r);
            }
        }
        let mut row = best.filter(|r| overlap(r) >= REBUILD_IOU)?; // else the face found isn't this one
        for v in &mut row[..14] {
            *v /= s as f32;
        }
        for k in [0, 4, 6, 8, 10, 12] {
            row[k] += x0 as f32;
            row[k + 1] += y0 as f32;
        }
        align(&img, &row).ok()
    }

    /// What the labeling engine asks for when an Ignored face gets a name: its print, picture and quality
    /// made again from its photo, with this importer's models.
    pub fn remake(self: &Arc<Self>) -> face_library::labels::Remake {
        let me = self.clone();
        Box::new(move |store: &Store, idx: &[usize]| {
            let Ok(faces) = me.models() else { return Vec::new() };
            (idx.iter())
                .filter_map(|&i| {
                    let crop = Importer::rebuild_crop(&faces, store, i)?;
                    let (emb, quality) = (faces.embed(&crop).ok()?, faces.quality(&crop).ok()?);
                    Some(Remade { i, emb, crop, quality })
                })
                .collect()
        })
    }
}

/// Where a face's picture is made again from (`Importer::crop_source`).
pub struct CropSource {
    /// the face's permanent ID, to tell afterwards that the place in the library is still this face's
    pub fid: String,
    path: PathBuf,
    frame: Option<i64>,
    place: [f32; 5],
}

/// A part of a picture.
fn cut(img: &Bgr, x0: usize, y0: usize, x1: usize, y1: usize) -> Bgr {
    let (w, h) = (x1 - x0, y1 - y0);
    let mut px = Vec::with_capacity(w * h * 3);
    for y in y0..y1 {
        px.extend_from_slice(&img.row(y)[x0 * 3..x1 * 3]);
    }
    Bgr { w, h, px }
}

/// How long ago this began, for a line of a log.
pub fn seconds_since(began: Instant) -> f64 {
    began.elapsed().as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_done_side_by_side_is_taken_in_order_and_not_too_far_ahead() {
        let begun = Mutex::new(Vec::new());
        let mut taken = Vec::new();
        let most_ahead = Mutex::new(0usize);
        let at = std::sync::atomic::AtomicUsize::new(0);
        in_order(
            40,
            4,
            6,
            |k| {
                begun.lock().unwrap().push(k);
                let ahead = k.saturating_sub(at.load(Ordering::SeqCst));
                let mut most = most_ahead.lock().unwrap();
                *most = (*most).max(ahead);
                std::thread::sleep(std::time::Duration::from_millis(((k * 7) % 5) as u64));
                k * 2
            },
            |k, v| {
                at.store(k + 1, Ordering::SeqCst);
                taken.push((k, v));
                true
            },
        );
        assert_eq!(taken, (0..40).map(|k| (k, k * 2)).collect::<Vec<_>>());
        assert!(*most_ahead.lock().unwrap() <= 6, "never more than the window ahead");
        // ended early: the rest is never begun
        let mut seen = 0;
        in_order(
            1000,
            3,
            4,
            |k| k,
            |_, _| {
                seen += 1;
                seen < 5
            },
        );
        assert_eq!(seen, 5);
        assert!(begun.lock().unwrap().len() == 40);
        in_order(0, 2, 2, |k| k, |_, _: usize| true);
    }

    #[test]
    fn the_kinds_of_file_and_the_workers() {
        assert!(is_video("clip.MOV") && is_video("a.mp4") && !is_video("photo.jpg") && !is_video("mov"));
        assert!((2..=12).contains(&import_workers()));
        let img = Bgr { w: 4, h: 3, px: (0..36).collect() };
        let part = cut(&img, 1, 1, 3, 3);
        assert_eq!((part.w, part.h, part.px), (2, 2, vec![15, 16, 17, 18, 19, 20, 27, 28, 29, 30, 31, 32]));
    }
}
