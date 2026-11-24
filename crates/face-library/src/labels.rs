//! The labeling engine: every face's label, who the unnamed faces
//! might be, and every change with its undo.
//!
//! A face's label is kept on the face itself, in `data/labels.json` by its permanent ID:
//!
//! ```text
//! {"<face id>": {"n": "Ann", "by": "me"}, ...}
//!   n    the person's name, or "#ignore" (a stranger, artwork, not a face: never recognized; its print
//!        and picture are deleted, only where it is in its photo is kept)
//!   by   "me" (named by hand) or "auto" (named automatically because the model was very sure)
//!   not  names this face was said NOT to be, never suggested for it again
//!   ok   a "might be mislabeled" warning was looked at and the name kept
//!   s    the model's score when the face was named automatically
//! ```
//!
//! For the faces nobody has named the engine works out who is suggested (score at or above the match
//! setting) and, for the rest, groups of look-alike faces to name. Every change is added to
//! `data/history.jsonl` and can be undone; `data/people.npz` (what the live tracker recognizes people from)
//! is written again after each.
//!
//! The files' bytes are fixed where they are text: libraries in use have them, and every device and version
//! must write the same.
//!
//! Not here yet, each with the part of the library it belongs to: the clues from the photo itself (people
//! tags, the same occasion: the photo facts), possible faces' "looks like a real face" score, and making a
//! face's print and picture again from its photo (`remake`, which the importer's models give).

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use face_core::{
    faces::{Q_ENROLL, RecModel},
    image::Bgr,
    npy::{self, Array},
};
use face_files::{
    filejson, forgotten,
    paths::{atomic_write, replace_file, utc_now, utc_stamp},
};
use serde_json::{Map, Value, json};

use crate::{
    faceness::{self, Faceness},
    groups::{self, Sets},
    photofacts::{self, CLOSEST, FOLDER_NEAR, Fact, Job, PhotoFacts, TAKEN_NEAR_H, split},
    photofiles::{self, local_day},
    scores::{Prints, Scores, dot},
    store::{Forgotten, Store},
};

pub const IGNORE: &str = "#ignore";
/// Words nobody can be named: the pages keep each person's data in objects by name, and these are every
/// object's own machinery there.
pub const NOT_NAMES: [&str; 13] = [
    "__proto__",
    "constructor",
    "prototype",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
];
/// naming an Ignored face, or taking it off the Ignored list, when its print and picture can't be made again
pub const NO_PRINT: &str = "Ignoring a face deleted its picture and face print, and they can't be made again: the photo \
                            isn't on this computer any more, or the face wasn't found in it again. The face stays as it is.";
// measured values: change them only with their measurements
/// a new face is named automatically at the model's `auto` score or above, if the next most likely person
/// scores at least this much lower
pub const MARGIN: f64 = 0.10;
/// people with fewer usable faces than this are flagged "needs more photos"
pub const FEW: usize = 5;
/// photos whose 256-bit picture fingerprints differ in at most this many bits are copies
pub const DUP_BITS: u32 = 12;
/// face prints this alike are the same picture again (`stacks`)
pub const SAME: f32 = 0.95;
/// how far under the match setting a group of unknown faces still gets a "looks like ..." hint
const HINT_UNDER: f64 = 0.12;
/// the steps that can be undone
const UNDO_KEPT: usize = 200;
/// A line's end in a file that is written as text: on Windows lines end as Windows ends them.
const END: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// Why something wasn't done.
#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    /// what was asked can't be done, in words for the page
    Asked(String),
    /// this computer's own trouble: a file that can't be read or written
    Files(String),
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fault::Asked(why) | Fault::Files(why) => f.write_str(why),
        }
    }
}

pub type Result<T> = std::result::Result<T, Fault>;

fn asked<T>(why: impl Into<String>) -> Result<T> {
    Err(Fault::Asked(why.into()))
}

fn files(why: impl ToString) -> Fault {
    Fault::Files(why.to_string())
}

/// What counts as something: not null, false, zero, or an empty text, list or object.
pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn has(r: &Map<String, Value>, key: &str) -> bool {
    r.get(key).is_some_and(truthy)
}

fn text<'a>(r: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    r.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn nots(r: &Map<String, Value>) -> impl Iterator<Item = &str> {
    r.get("not").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
}

use crate::photofacts::strip;

/// A folder's path as text that is the same however it was written:
/// one kind of separator, no doubled ones, no "." steps, ".." taken, and on Windows in lower case.
fn folder_key(path: &str) -> String {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let rooted = path.starts_with(seps);
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(seps) {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != ".." && !p.ends_with(':')) => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join(std::path::MAIN_SEPARATOR_STR);
    let key = if rooted { format!("{}{joined}", std::path::MAIN_SEPARATOR) } else { joined };
    if cfg!(windows) { key.to_lowercase() } else { key }
}

/// The folder a file is in (everything before the last separator).
fn folder_of(path: &str) -> &str {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    path.rfind(seps).map_or("", |k| &path[..k])
}

/// {name: usable faces} of the named faces: what each person is recognized from.
pub struct Gallery {
    /// each person's usable faces in rising order, the people in the order of their names
    pub people: Vec<(String, Vec<usize>)>,
    /// the people (as places in `people`) in the order their first face comes in the labels: the order
    /// `people.npz` is written in
    pub seen: Vec<usize>,
    at: HashMap<String, usize>,
}

impl Gallery {
    pub fn pos(&self, name: &str) -> Option<usize> {
        self.at.get(name).copied()
    }
    pub fn faces(&self, name: &str) -> &[usize] {
        self.pos(name).map_or(&[], |j| &self.people[j].1)
    }
}

/// An unnamed face and who it most looks like.
#[derive(Clone, Debug, PartialEq)]
pub struct Tri {
    pub i: usize,
    /// the best person, as a place in the gallery's `people` (none while nobody is named)
    pub name: Option<usize>,
    /// the best score and the second best
    pub s: f32,
    pub s2: f32,
}

/// A print and picture made again from a face's photo.
pub struct Remade {
    pub i: usize,
    pub emb: Vec<f32>,
    pub crop: Bgr,
    pub quality: f32,
}

/// Makes prints and pictures again for faces that lost theirs to Ignore: only those whose photo is on this
/// computer and whose face is found in it again.
pub type Remake = Box<dyn Fn(&Store, &[usize]) -> Vec<Remade> + Send + Sync>;

/// What the possible faces are learned from (`LabelDb::faceness_job`).
pub struct FacenessJob {
    emb: Vec<f32>,
    dim: usize,
    /// the real faces, the possible faces nobody named, and every possible face to score
    pub pos: Vec<usize>,
    pub neg: Vec<usize>,
    pub idx: Vec<usize>,
    /// the faces as they were: how many, and `version_faces`
    pub n: usize,
    pub version_faces: u64,
}

impl FacenessJob {
    pub fn prints(&self) -> Prints<'_> {
        Prints { emb: &self.emb, dim: self.dim }
    }
}

/// One change, for Undo.
struct Step {
    /// each changed face's label as it was (none: it had no label)
    before: Vec<(String, Option<Value>)>,
    text: String,
    /// an Ignore's deleted prints and pictures, kept in memory for this session
    prints: Vec<(String, Forgotten)>,
    /// faces whose print and picture were made again for this step: Undo makes them Ignored again
    remade: Vec<String>,
}

/// What is worked out once per state of the labels and dropped at every change.
#[derive(Default)]
struct Cache {
    gallery: Option<Arc<Gallery>>,
    triage: Option<Arc<Vec<Tri>>>,
    checks: Option<Arc<Value>>,
    cam: Option<Arc<HashSet<String>>>,
    maybe: Option<Arc<HashMap<String, f64>>>,
    by_photo: Option<Arc<HashMap<String, Vec<usize>>>>,
    /// {folder: its photos with faces, in name order}
    folders: Option<Arc<HashMap<String, Vec<String>>>>,
}

/// The stacks as last worked out: for how many faces, up to where in the log of changed prints.
struct Stacked {
    n: usize,
    seen: usize,
    parent: Vec<usize>,
}

/// All faces and their labels. One at a time: whoever shares it between threads keeps it behind a lock.
pub struct LabelDb {
    pub store: Store,
    /// {face ID: {n, by, not, ok, s}}, in the file's order
    pub labels: Map<String, Value>,
    data: PathBuf,
    home: PathBuf,
    /// where the live tracker saves its snapshots (the private folder's `webcam`); `changed()` after changing it
    pub webcam: PathBuf,
    model: RecModel,
    undo: Vec<Step>,
    /// rises with every change
    pub version: u64,
    /// rises when faces or their prints change
    pub version_faces: u64,
    scores: Scores,
    cache: Cache,
    /// the groups of unknown faces as last worked out, and the faces that were grouped then
    groups: Option<Vec<Vec<usize>>>,
    grouped: HashSet<usize>,
    stacked: Option<Stacked>,
    stacks: Option<(usize, usize, Arc<HashMap<usize, usize>>)>,
    /// the picture fingerprints of the photos read, for `near_duplicate`
    fps: Option<(Vec<String>, Vec<[u8; 32]>)>,
    /// what was learned about the possible faces, and the faces it was learned from (how many, and
    /// `version_faces` then)
    faceness: Option<(usize, u64, Arc<Faceness>)>,
    /// what each photo says about itself: camera dates and people tags
    pub facts: PhotoFacts,
    /// gives prints and pictures back to Ignored faces that get a name (none: they stay as they are)
    pub remake: Option<Remake>,
    /// called after anything in the data folder was saved (the sync looks for what changed)
    pub on_saved: Vec<Box<dyn Fn() + Send + Sync>>,
}

impl LabelDb {
    /// The library in `data`, with this user's private folder `home` and the recognition model in use.
    /// `backup`: keep a copy of labels.json as it is now. `log`: told what was changed while opening.
    pub fn open(
        data: &Path,
        home: &Path,
        model: RecModel,
        backup: bool,
        log: &mut dyn FnMut(String),
    ) -> Result<LabelDb> {
        LabelDb::open_making(data, home, model, backup, log, None)
    }

    /// `open`, for a program that can make prints: a library whose prints are of another recognition model
    /// than `model` gets every print made again by `make` (a face's new print, from the picture the library
    /// keeps of it), and the people file with them. The prints as they were are kept in a file beside the
    /// library's (`faces.before-new-prints.npz`). Without `make`, such a library is refused.
    pub fn open_making(
        data: &Path,
        home: &Path,
        model: RecModel,
        backup: bool,
        log: &mut dyn FnMut(String),
        make: Option<&mut dyn FnMut(&Store, usize) -> Option<Vec<f32>>>,
    ) -> Result<LabelDb> {
        let mut store = Store::load(data, model.name, model.dim).map_err(Fault::Files)?;
        if store.needs_ids() {
            return Err(files(
                "This library is from before faces had permanent IDs, and this version of the labeling tool \
                              can't open it.",
            ));
        }
        let mut made_again = false;
        if !store.is_empty() && store.model != model.name {
            let Some(make) = make else {
                return Err(files(
                    "This library's face prints were made by another recognition model than this program's. Open it \
                     with the app once: it makes every print again from the pictures the library keeps.",
                ));
            };
            let before = data.join("faces.before-new-prints.npz");
            std::fs::copy(data.join("faces.npz"), &before)
                .map_err(|e| files(format!("can't keep a copy of the prints: {e}")))?;
            match store.remake_prints(model.name, model.dim, make) {
                Ok(made) => {
                    store.save().map_err(Fault::Files)?;
                    made_again = true;
                    log(format!(
                        "This version recognizes faces with a new model: every face print was made again from the \
                         library's own pictures ({made} faces). The prints as they were are in {}: delete it once \
                         you're happy.",
                        before.display()
                    ));
                }
                Err(failed) => {
                    let _ = std::fs::remove_file(&before);
                    return Err(files(format!(
                        "This version recognizes faces with a new model and has to make every face print again, but {} \
                         faces have no picture on this computer to make it from. Nothing was changed. Start the \
                         version before once more and let it finish getting the faces' pictures (the labeling tool \
                         makes them from the photos, or takes them from the shared database), then update again.",
                        failed.len()
                    )));
                }
            }
        }
        let labels = match std::fs::read(data.join("labels.json")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(files(format!("can't read labels.json: {e}"))),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(Value::Object(m)) => m,
                Ok(_) => return Err(files("labels.json is not a list of labels")),
                Err(e) => return Err(files(format!("can't read labels.json: {e}"))),
            },
        };
        let facts = PhotoFacts::load(data, &store).map_err(Fault::Files)?;
        let mut db = LabelDb {
            store,
            labels,
            facts,
            faceness: None,
            data: data.to_path_buf(),
            home: home.to_path_buf(),
            webcam: home.join("webcam"),
            model,
            undo: Vec::new(),
            version: 0,
            version_faces: 0,
            scores: Scores::default(),
            cache: Cache::default(),
            groups: None,
            grouped: HashSet::new(),
            stacked: None,
            stacks: None,
            fps: None,
            remake: None,
            on_saved: Vec::new(),
        };
        if backup {
            db.backup(20)?;
        }
        if made_again {
            db.build_people()?; // the tracker reads its people from this file
        }
        // Ignored before v1.9, when their prints and pictures were still kept
        let kept: Vec<usize> = (0..db.n()).filter(|&i| db.name(i) == Some(IGNORE) && !db.printless(i)).collect();
        if !kept.is_empty() {
            let saved = db.backup_before_forgetting()?;
            db.forget_faces(&kept)?;
            log(format!(
                "Deleted the face prints and pictures of {} ignored faces (only where they are is kept). A copy from \
                 before is in {}: delete it once you're happy.",
                kept.len(),
                saved.display()
            ));
        }
        Ok(db)
    }

    // ----- basics

    /// How many faces there are.
    pub fn n(&self) -> usize {
        self.store.len()
    }

    fn prints(&self) -> Prints<'_> {
        Prints { emb: self.store.prints(), dim: self.store.dim }
    }

    /// Face i's label (none: it has no label).
    pub fn rec(&self, i: usize) -> Option<&Map<String, Value>> {
        self.labels.get(&self.store.fid[i]).and_then(Value::as_object)
    }

    pub fn name(&self, i: usize) -> Option<&str> {
        self.rec(i).and_then(|r| text(r, "n"))
    }

    /// Who named face i: "me" or "auto".
    pub fn by(&self, i: usize) -> &str {
        self.rec(i).and_then(|r| text(r, "by")).unwrap_or("me")
    }

    /// True if face i's print and picture were deleted (Ignored, or a forgotten person's): it can't be
    /// scored, grouped or teach the recognizer.
    pub fn printless(&self, i: usize) -> bool {
        self.store.printless(i)
    }

    /// A face the recognizer may learn from: not below the quality for it (not scored counts as good enough),
    /// and with a print.
    pub fn usable(&self, i: usize) -> bool {
        let q = self.store.quality[i];
        #[allow(clippy::neg_cmp_op_on_partial_ord)] // a quality that is not a number is "not scored": usable
        let good = !(q < Q_ENROLL);
        good && !self.printless(i)
    }

    pub fn hidden(&self, i: usize, show_low: bool) -> bool {
        !show_low && !self.usable(i)
    }

    /// Face i's quality for a page: three decimals, nothing where it wasn't scored.
    pub fn q(&self, i: usize) -> Value {
        let q = self.store.quality[i];
        if q.is_nan() { Value::Null } else { json!(filejson::round(q as f64, 3)) }
    }

    /// The match setting: the score at which a face is taken for a known person.
    pub fn match_at(&self) -> f64 {
        self.model.match_at
    }

    /// Something changed: what was worked out for the state before is dropped.
    pub fn changed(&mut self) {
        self.version += 1;
        self.cache = Cache::default();
    }

    /// Faces' prints changed (deleted, put back, another device's). `regroup` false keeps the groups of
    /// unknown faces as they are, for an Ignore and its undo: an Ignored face only leaves its group and comes
    /// back into it when the Ignore is undone.
    fn forgot(&mut self, regroup: bool) {
        self.version_faces += 1;
        if regroup {
            self.groups = None;
        }
        self.changed();
    }

    /// The labels were changed from outside the engine's own changes (the sync took other devices'): what
    /// was worked out is dropped, `labels.json` is written and the people everyone is recognized from are
    /// made anew.
    pub fn labels_changed(&mut self) -> Result<()> {
        self.changed();
        self.save_labels()?;
        self.build_people()?;
        Ok(())
    }

    /// Faces' prints were changed from outside the engine's own changes (the sync took other devices' faces).
    pub fn prints_changed(&mut self) {
        self.forgot(true);
    }

    /// The groups of unknown faces are worked out anew at the next question (faces were added).
    pub fn regroup_next(&mut self) {
        self.groups = None;
    }

    fn note_saved(&self) {
        for hook in &self.on_saved {
            hook();
        }
    }

    /// The library's folder.
    pub fn data(&self) -> &Path {
        &self.data
    }

    /// Save the faces, their pictures and the photos' records.
    pub fn save_store(&mut self) -> Result<()> {
        self.store.save().map_err(Fault::Files)?;
        self.note_saved();
        Ok(())
    }

    fn save_labels(&self) -> Result<()> {
        let rows = filejson::rows_of(&self.labels, true);
        atomic_write(&self.data.join("labels.json"), rows.as_bytes())
            .map_err(|e| files(format!("can't write labels.json: {e}")))?;
        self.note_saved();
        Ok(())
    }

    /// The photos that are snapshots the live tracker saved (in the private folder's `webcam`).
    fn cam(&mut self) -> Arc<HashSet<String>> {
        if let Some(c) = &self.cache.cam {
            return c.clone();
        }
        let cam = folder_key(&self.webcam.to_string_lossy());
        let found: HashSet<String> = (self.store.photos.iter())
            .filter(|(_, p)| {
                p.get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| !path.is_empty() && folder_key(folder_of(path)) == cam)
            })
            .map(|(h, _)| h.clone())
            .collect();
        let found = Arc::new(found);
        self.cache.cam = Some(found.clone());
        found
    }

    /// True if face i is from a webcam snapshot the live tracker saved.
    pub fn webcam(&mut self, i: usize) -> bool {
        self.cam().contains(&self.store.photo[i])
    }

    /// {face ID: detector score} of the possible faces (kept with the low cut-off) and the faces drawn by hand.
    fn maybes(&mut self) -> Arc<HashMap<String, f64>> {
        if let Some(m) = &self.cache.maybe {
            return m.clone();
        }
        let mut found = HashMap::new();
        for p in self.store.photos.values() {
            for (fid, score) in p.get("maybe").and_then(Value::as_object).into_iter().flatten() {
                found.insert(fid.clone(), score.as_f64().unwrap_or(0.0));
            }
        }
        let found = Arc::new(found);
        self.cache.maybe = Some(found.clone());
        found
    }

    /// The detector's score of face i if it is a possible face, else none.
    pub fn maybe(&mut self, i: usize) -> Option<f64> {
        self.maybes().get(&self.store.fid[i]).copied()
    }

    /// Seconds into its video where face i was found; none for a face from a photo.
    pub fn video_at(&self, i: usize) -> Option<f64> {
        let p = self.store.photos.get(&self.store.photo[i])?.as_object()?;
        if !has(p, "video") {
            return None;
        }
        let frame = p.get("at")?.get(&self.store.fid[i])?.as_f64()?;
        let fps = p.get("fps").and_then(Value::as_f64).filter(|f| *f != 0.0).unwrap_or(30.0);
        Some(filejson::round(frame / fps, 1))
    }

    /// {photo: its faces}
    pub fn by_photo(&mut self) -> Arc<HashMap<String, Vec<usize>>> {
        if let Some(b) = &self.cache.by_photo {
            return b.clone();
        }
        let mut found: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, h) in self.store.photo.iter().enumerate().take(self.n()) {
            found.entry(h.clone()).or_default().push(i);
        }
        let found = Arc::new(found);
        self.cache.by_photo = Some(found.clone());
        found
    }

    // ----- scoring

    /// The named, usable faces of each person: what they are recognized from. Webcam faces named
    /// automatically wait for a confirmation first (many snapshots of one visit come in at once, so one wrong
    /// automatic name would teach the recognizer the wrong face and name the next ones wrongly too), and so do
    /// possible faces named automatically.
    pub fn gallery(&mut self) -> Arc<Gallery> {
        if let Some(g) = &self.cache.gallery {
            return g.clone();
        }
        let (cam, maybe) = (self.cam(), self.maybes());
        let mut first: Vec<(String, Vec<usize>)> = Vec::new();
        let mut place: HashMap<&str, usize> = HashMap::new();
        for (fid, r) in &self.labels {
            let (Some(i), Some(r)) = (self.store.index_of(fid), r.as_object()) else { continue };
            let Some(name) = text(r, "n") else { continue };
            let waits =
                text(r, "by") == Some("auto") && (cam.contains(&self.store.photo[i]) || maybe.contains_key(fid));
            if self.usable(i) && !waits {
                let at = *place.entry(name).or_insert_with(|| {
                    first.push((name.to_string(), Vec::new()));
                    first.len() - 1
                });
                first[at].1.push(i);
            }
        }
        first.iter_mut().for_each(|(_, faces)| faces.sort_unstable());
        let mut order: Vec<usize> = (0..first.len()).collect();
        order.sort_by(|a, b| first[*a].0.cmp(&first[*b].0));
        let mut seen = vec![0; first.len()];
        order.iter().enumerate().for_each(|(sorted, was)| seen[*was] = sorted);
        let mut slots: Vec<Option<(String, Vec<usize>)>> = first.into_iter().map(Some).collect();
        let people: Vec<(String, Vec<usize>)> = order.iter().filter_map(|was| slots[*was].take()).collect();
        let at = people.iter().enumerate().map(|(j, (name, _))| (name.clone(), j)).collect();
        let gallery = Arc::new(Gallery { people, seen, at });
        self.cache.gallery = Some(gallery.clone());
        gallery
    }

    /// The gallery, with every face's score for each of its people up to date.
    fn scored(&mut self) -> Arc<Gallery> {
        let gallery = self.gallery();
        let e = Prints { emb: self.store.prints(), dim: self.store.dim };
        self.scores.update(e, self.store.print_log(), &gallery.people);
        gallery
    }

    /// Every face's score for each person of the gallery, a list a person (in the gallery's order).
    fn columns<'a>(&'a self, gallery: &Gallery) -> Vec<&'a [f32]> {
        gallery.people.iter().map(|(name, _)| self.scores.column(name).unwrap_or(&[])).collect()
    }

    /// The people of the gallery (as places in it) that face i was said not to be.
    fn not_these(&self, gallery: &Gallery, i: usize) -> Vec<usize> {
        self.rec(i).map_or_else(Vec::new, |r| nots(r).filter_map(|name| gallery.pos(name)).collect())
    }

    /// Face i's score for every person of the gallery, the way the recognizer scores (the mean of the 5
    /// closest of that person's faces; a face never counts toward its own score), and -1 for the people it
    /// was said not to be.
    fn row(&self, gallery: &Gallery, columns: &[&[f32]], i: usize) -> Vec<f32> {
        let not = self.not_these(gallery, i);
        columns.iter().enumerate().map(|(j, col)| if not.contains(&j) { -1.0 } else { col[i] }).collect()
    }

    /// Faces scored against every person: the people's names in order, and a row of scores for each face.
    pub fn score(&mut self, idx: &[usize]) -> (Vec<String>, Vec<Vec<f32>>) {
        let gallery = self.scored();
        let columns = self.columns(&gallery);
        let rows = idx.iter().map(|&i| self.row(&gallery, &columns, i)).collect();
        (gallery.people.iter().map(|(name, _)| name.clone()).collect(), rows)
    }

    /// Every unnamed face with a print: the best person, their score, and the second best score.
    pub fn triage(&mut self) -> Arc<Vec<Tri>> {
        if let Some(t) = &self.cache.triage {
            return t.clone();
        }
        let gallery = self.scored();
        let columns = self.columns(&gallery);
        let mut found = Vec::new();
        for i in 0..self.n() {
            if self.name(i).is_some() || self.printless(i) {
                continue;
            }
            let not = self.not_these(&gallery, i);
            let (mut s, mut name, mut s2) = (f32::NEG_INFINITY, None, f32::NEG_INFINITY);
            for (j, col) in columns.iter().enumerate() {
                let v = if not.contains(&j) { -1.0 } else { col[i] };
                if v > s {
                    (s2, s, name) = (s, v, Some(j));
                } else if v > s2 {
                    s2 = v;
                }
            }
            found.push(match name {
                Some(_) => Tri { i, name, s, s2: if columns.len() > 1 { s2 } else { -1.0 } },
                None => Tri { i, name: None, s: -1.0, s2: -1.0 },
            });
        }
        let found = Arc::new(found);
        self.cache.triage = Some(found.clone());
        found
    }

    /// Whether an unnamed face is suggested a name: the model thinks it knows who it is.
    fn suggests(&self, t: &Tri) -> bool {
        t.name.is_some() && t.s as f64 >= self.model.match_at
    }

    // ----- lists for the labeling page

    /// The unnamed faces the model suggests a name for, by that name: [{name, faces: [[face, score,
    /// quality]]}], each person's least sure first (those are the ones to check), most faces first.
    pub fn suggested(&mut self, show_low: bool) -> Value {
        let (tri, gallery) = (self.triage(), self.gallery());
        let mut found: Vec<(usize, Vec<(usize, f64)>)> = Vec::new();
        let mut place: HashMap<usize, usize> = HashMap::new();
        for t in tri.iter().filter(|t| self.suggests(t) && !self.hidden(t.i, show_low)) {
            let j = t.name.unwrap_or(0);
            let at = *place.entry(j).or_insert_with(|| {
                found.push((j, Vec::new()));
                found.len() - 1
            });
            found[at].1.push((t.i, filejson::round(t.s as f64, 3)));
        }
        found.iter_mut().for_each(|(_, faces)| faces.sort_by(|a, b| a.1.total_cmp(&b.1)));
        found.sort_by_key(|(j, faces)| (gallery.people[*j].0 == IGNORE, std::cmp::Reverse(faces.len())));
        Value::Array(
            (found.iter())
                .map(|(j, faces)| {
                    let faces: Vec<Value> = faces.iter().map(|(i, s)| json!([i, s, self.q(*i)])).collect();
                    json!({"name": gallery.people[*j].0, "faces": faces})
                })
                .collect(),
        )
    }

    /// Cluster the faces nobody is suggested for into look-alike groups.
    fn regroup(&mut self, idx: &[usize]) {
        self.grouped = idx.iter().copied().collect();
        self.groups = Some(groups::groups(self.prints(), idx, self.model.cluster));
    }

    /// The unnamed faces nobody is suggested for: {groups: [{id, faces: [[face, quality]], hint}], singles:
    /// [[face, quality]]}. A group close to a known person gets a "looks like ..." hint.
    pub fn unknown(&mut self, show_low: bool, regroup: bool) -> Value {
        let (tri, gallery, maybe) = (self.triage(), self.scored(), self.maybes());
        let unk: BTreeSet<usize> = (tri.iter())
            .filter(|t| !self.suggests(t) && !maybe.contains_key(&self.store.fid[t.i]))
            .map(|t| t.i)
            .collect();
        if regroup || self.groups.is_none() {
            self.regroup(&unk.iter().copied().collect::<Vec<_>>());
        }
        let shown = |i: &usize| unk.contains(i) && !self.hidden(*i, show_low);
        let found: Vec<Vec<usize>> = (self.groups.iter().flatten())
            .map(|g| g.iter().copied().filter(|i| shown(i)).collect::<Vec<_>>())
            .filter(|g| !g.is_empty())
            .collect();
        let mut singles: Vec<usize> = found.iter().filter(|g| g.len() == 1).map(|g| g[0]).collect();
        singles.extend(unk.iter().filter(|i| !self.grouped.contains(i) && !self.hidden(**i, show_low)));
        let mut found: Vec<Vec<usize>> = found.into_iter().filter(|g| g.len() > 1).collect();
        found.sort_by_key(|g| std::cmp::Reverse(g.len()));
        let columns = self.columns(&gallery);
        let hint_at = (self.model.match_at - HINT_UNDER) as f32;
        let mut out = Vec::new();
        for (k, g) in found.iter().enumerate() {
            // the group's mean score for each person, and the best of them
            let mut sum = vec![0f32; columns.len()];
            for &i in g {
                for (j, v) in self.row(&gallery, &columns, i).into_iter().enumerate() {
                    sum[j] += v;
                }
            }
            let mut best: Option<(usize, f32)> = None;
            for (j, total) in sum.iter().enumerate() {
                let mean = total / g.len() as f32;
                if best.is_none_or(|(_, b)| mean > b) {
                    best = Some((j, mean));
                }
            }
            let hint = match best {
                Some((j, mean)) if mean >= hint_at => {
                    json!({"name": gallery.people[j].0, "s": filejson::round(mean as f64, 3)})
                }
                _ => Value::Null,
            };
            let faces: Vec<Value> = g.iter().map(|i| json!([i, self.q(*i)])).collect();
            out.push(json!({"id": k + 1, "faces": faces, "hint": hint}));
        }
        let quality = |i: usize| self.q(i).as_f64().unwrap_or(0.0);
        singles.sort_by(|a, b| quality(*b).total_cmp(&quality(*a)));
        let singles: Vec<Value> = singles.iter().map(|i| json!([i, self.q(*i)])).collect();
        json!({"groups": out, "singles": singles})
    }

    /// {face: first face of its stack} for the faces that are the same picture as another face: prints at
    /// least `SAME` alike (screenshots of one avatar, burst shots, re-saved copies). Worked out again when
    /// faces change, and then only for the faces it has to be: new ones, and those whose print changed with
    /// the stacks they were in.
    pub fn stacks(&mut self) -> Arc<HashMap<usize, usize>> {
        let (n, seen) = (self.n(), self.store.print_log().len());
        if let Some((for_n, for_seen, st)) = &self.stacks
            && (*for_n, *for_seen) == (n, seen)
        {
            return st.clone();
        }
        let e = Prints { emb: self.store.prints(), dim: self.store.dim };
        let mut sets = match self.stacked.take() {
            Some(was) if was.n <= n && was.seen <= seen => {
                let mut sets = Sets::of(was.parent);
                sets.grow(n);
                let mut redo: BTreeSet<usize> = (was.n..n).collect();
                let moved: BTreeSet<usize> =
                    self.store.print_log()[was.seen..].iter().copied().filter(|i| *i < was.n).collect();
                if !moved.is_empty() {
                    // their stacks may fall apart: taken apart, put together again
                    let apart: HashSet<usize> = moved.iter().map(|i| sets.root(*i)).collect();
                    redo.extend((0..was.n).filter(|i| apart.contains(&sets.root(*i))));
                    redo.iter().for_each(|i| sets.alone(*i));
                }
                let rows: Vec<usize> = redo.into_iter().collect();
                for (a, b) in groups::alike(e, &rows, false, SAME) {
                    sets.join(a, b);
                }
                sets
            }
            _ => {
                let mut sets = Sets::new(n);
                for (a, b) in groups::alike(e, &(0..n).collect::<Vec<_>>(), true, SAME) {
                    sets.join(a, b);
                }
                sets
            }
        };
        let mut found = HashMap::new();
        for i in 0..n {
            let first = sets.root(i);
            if first != i {
                found.insert(i, first);
                found.insert(first, first);
            }
        }
        self.stacked = Some(Stacked { n, seen, parent: sets.into_parents() });
        let found = Arc::new(found);
        self.stacks = Some((n, seen, found.clone()));
        found
    }

    /// One person's faces: [[face, score for that person, quality, named by]], least like them first.
    pub fn person(&mut self, name: &str) -> Value {
        let idx: Vec<usize> = (0..self.n()).filter(|&i| self.name(i) == Some(name)).collect();
        let (names, rows) = self.score(&idx);
        let j = names.iter().position(|n| n == name);
        let mut found: Vec<(usize, Option<f64>)> =
            idx.iter().zip(&rows).map(|(i, row)| (*i, j.map(|j| filejson::round(row[j] as f64, 3)))).collect();
        found.sort_by(|a, b| {
            (a.1.is_none(), a.1.unwrap_or(0.0))
                .partial_cmp(&(b.1.is_none(), b.1.unwrap_or(0.0)))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Value::Array(found.iter().map(|(i, s)| json!([i, s, self.q(*i), self.by(*i)])).collect())
    }

    /// {name: faces} of the named faces (`by`: only those named by "me" or by "auto"), the people in the
    /// order of their first face.
    pub fn people_faces(&self, by: Option<&str>) -> Vec<(String, Vec<usize>)> {
        let mut found: Vec<(String, Vec<usize>)> = Vec::new();
        let mut place: HashMap<&str, usize> = HashMap::new();
        for i in 0..self.n() {
            let Some(name) = self.name(i) else { continue };
            if by.is_none_or(|b| self.by(i) == b) {
                let at = *place.entry(name).or_insert_with(|| {
                    found.push((name.to_string(), Vec::new()));
                    found.len() - 1
                });
                found[at].1.push(i);
            }
        }
        found
    }

    /// The faces named automatically, by person: [{name, faces: [[face, score, quality]]}], least sure first.
    pub fn auto_review(&mut self) -> Value {
        let mut found: Vec<(String, Vec<(usize, Option<f64>)>)> = Vec::new();
        for (name, faces) in self.people_faces(Some("auto")) {
            let (names, rows) = self.score(&faces);
            let j = names.iter().position(|n| *n == name);
            let mut scored: Vec<(usize, Option<f64>)> =
                faces.iter().zip(&rows).map(|(i, row)| (*i, j.map(|j| filejson::round(row[j] as f64, 3)))).collect();
            scored.sort_by(|a, b| a.1.unwrap_or(-1.0).total_cmp(&b.1.unwrap_or(-1.0)));
            found.push((name, scored));
        }
        found.sort_by_key(|(name, faces)| (name == IGNORE, std::cmp::Reverse(faces.len())));
        Value::Array(
            (found.iter())
                .map(|(name, faces)| {
                    let faces: Vec<Value> = faces.iter().map(|(i, s)| json!([i, s, self.q(*i)])).collect();
                    json!({"name": name, "faces": faces})
                })
                .collect(),
        )
    }

    /// Possible mistakes and gaps:
    ///   mislabeled  named faces that look more like someone else, or not like the rest of their person
    ///   few         people with too few usable faces to be recognized reliably
    ///   duplicates  photos skipped during import because they were copies of photos already read
    pub fn checks(&mut self) -> Arc<Value> {
        if let Some(c) = &self.cache.checks {
            return c.clone();
        }
        let gallery = self.scored();
        let columns = self.columns(&gallery);
        let mut bad: Vec<(f64, Value)> = Vec::new();
        for &g in &gallery.seen {
            let (name, faces) = &gallery.people[g];
            if name == IGNORE {
                continue;
            }
            for &i in faces.iter().filter(|i| !self.rec(**i).is_some_and(|r| has(r, "ok"))) {
                let mut row = self.row(&gallery, &columns, i);
                let own = row[g] as f64;
                row[g] = -1.0; // the best of the others
                let (mut k, mut other) = (0, f32::NEG_INFINITY);
                for (j, v) in row.iter().enumerate() {
                    if *v > other {
                        (k, other) = (j, *v);
                    }
                }
                let (other_name, other) =
                    if row.len() > 1 { (Some(&gallery.people[k].0), other as f64) } else { (None, -1.0) };
                if faces.len() > 1 && (own < self.model.match_at || other > own) {
                    let (s, os) = (filejson::round(own, 3), filejson::round(other, 3));
                    bad.push((s - os, json!({"i": i, "name": name, "s": s, "other": other_name, "os": os, "by": self.by(i), "q": self.q(i)})));
                }
            }
        }
        bad.sort_by(|a, b| a.0.total_cmp(&b.0));
        let everyone: BTreeSet<&str> = self
            .labels
            .values()
            .filter_map(|r| r.as_object().and_then(|r| text(r, "n")))
            .filter(|n| *n != IGNORE)
            .collect();
        let mut few: Vec<(usize, &str)> = everyone
            .iter()
            .map(|name| (gallery.faces(name).len(), *name))
            .filter(|(usable, _)| *usable < FEW)
            .collect();
        few.sort();
        let path = |p: &Value| p.get("path").cloned().unwrap_or(Value::Null);
        let dups: Vec<Value> = (self.store.photos.values())
            .filter_map(|p| {
                let of = p.get("dup_of").filter(|d| truthy(d))?;
                let original = of.as_str().and_then(|h| self.store.photos.get(h)).map_or(Value::Null, path);
                Some(json!({"path": path(p), "of": original}))
            })
            .collect();
        let found = Arc::new(json!({
            "mislabeled": bad.into_iter().map(|(_, b)| b).collect::<Vec<_>>(),
            "few": few.iter().map(|(usable, name)| json!({"name": name, "usable": usable})).collect::<Vec<_>>(),
            "duplicates": dups,
        }));
        self.cache.checks = Some(found.clone());
        found
    }

    /// The numbers the labeling page shows at its top, and each person with their faces.
    pub fn summary(&mut self) -> Value {
        let (tri, gallery, maybe) = (self.triage(), self.gallery(), self.maybes());
        let suggested = tri.iter().filter(|t| self.suggests(t) && self.usable(t.i)).count();
        let unknown = tri
            .iter()
            .filter(|t| !self.suggests(t) && self.usable(t.i) && !maybe.contains_key(&self.store.fid[t.i]))
            .count();
        let low = tri.iter().filter(|t| !self.usable(t.i)).count();
        let mut everyone = self.people_faces(None);
        everyone.sort_by_cached_key(|(name, _)| (name == IGNORE, name.to_lowercase()));
        let quality = |i: usize| Some(self.store.quality[i]).filter(|q| !q.is_nan()).unwrap_or(0.0);
        let mut people = Vec::new();
        for (name, faces) in &everyone {
            let usable = gallery.faces(name);
            // the clearest of their faces is their picture; of equally clear ones the first
            let mut cover = None;
            for &i in if usable.is_empty() { faces.as_slice() } else { usable } {
                if cover.is_none_or(|c| quality(i) > quality(c)) {
                    cover = Some(i);
                }
            }
            let auto = faces.iter().filter(|i| self.rec(**i).is_some_and(|r| text(r, "by") == Some("auto"))).count();
            people.push(
                json!({"name": name, "faces": faces.len(), "usable": usable.len(), "cover": cover, "auto": auto}),
            );
        }
        let checks = self.checks();
        let count = |key: &str| checks[key].as_array().map_or(0, Vec::len);
        let auto = (self.labels.values().filter_map(Value::as_object))
            .filter(|r| text(r, "by") == Some("auto") && has(r, "n"))
            .count();
        let possible = self.possible().as_array().map_or(0, |all| all.iter().filter(|f| f[4] == true).count());
        let tagged: usize = self
            .tagged(false)
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| g["faces"].as_array().map_or(0, Vec::len))
            .sum();
        let left = self.store.photos.values().filter(|p| photofiles::needs_possible(p)).count();
        let status = &self.facts.status;
        let facts = json!({"running": status.running, "done": status.done, "total": status.total, "read": status.read});
        let cam = self.cam();
        let webcam: Vec<usize> = (0..self.n()).filter(|&i| cam.contains(&self.store.photo[i])).collect();
        let video: Vec<usize> = (0..self.n())
            .filter(|&i| {
                self.store.photos.get(&self.store.photo[i]).and_then(Value::as_object).is_some_and(|p| has(p, "video"))
            })
            .collect();
        json!({
            "faces": self.n(), "photos": self.store.photos.len(), "suggested": suggested, "unknown": unknown,
            "hiddenLow": low, "auto": auto, "checks": count("mislabeled") + count("few"), "people": people,
            "possible": possible, "possibleLeft": left, "tagged": tagged, "facts": facts,
            "canUndo": !self.undo.is_empty(), "undoText": self.undo.last().map_or("", |s| s.text.as_str()),
            "webcam": webcam, "video": video,
            "settings": {"match": self.model.match_at, "auto": self.model.auto, "margin": MARGIN, "qEnroll": filejson::round(Q_ENROLL as f64, 6),
                         "few": FEW, "model": self.store.model, "folders": photofiles::folders(&self.data)},
        })
    }

    // ----- possible faces

    /// (real, not) faces to learn "looks like a real face" from: real are the faces from the normal detector
    /// (named or not, but not Ignored) and the possible faces someone named; not are the possible faces nobody
    /// named (most possible faces aren't faces). Faces drawn by hand count as neither, and faces without a
    /// print can't teach anything.
    pub fn face_sets(&mut self) -> (Vec<usize>, Vec<usize>) {
        let maybe = self.maybes();
        let (mut pos, mut neg) = (Vec::new(), Vec::new());
        for i in (0..self.n()).filter(|i| !self.printless(*i)) {
            let name = self.name(i);
            match maybe.get(&self.store.fid[i]) {
                None if name != Some(IGNORE) => pos.push(i),
                Some(score) if *score > 0.0 => {
                    if name.is_none_or(|n| n == IGNORE) {
                        neg.push(i)
                    } else {
                        pos.push(i)
                    }
                }
                _ => {}
            }
        }
        (pos, neg)
    }

    /// Whether the faces changed since the possible faces were last learned about (or they never were).
    pub fn faceness_due(&self) -> bool {
        self.faceness.as_ref().is_none_or(|(n, version, _)| (*n, *version) != (self.n(), self.version_faces))
    }

    /// Learn which possible faces look like real ones, from the faces as they are now: seconds on thousands
    /// of faces. (It can be done on a thread of its own, going on with what was known meanwhile:
    /// whoever shares this engine between threads does the same with `faceness::learn` on a copy of the prints
    /// and `set_faceness`.)
    pub fn learn_faceness(&mut self) {
        if !self.faceness_due() {
            return;
        }
        let (pos, neg) = self.face_sets();
        let maybe = self.maybes();
        let idx: Vec<usize> =
            (0..self.n()).filter(|i| maybe.get(&self.store.fid[*i]).is_some_and(|s| *s > 0.0)).collect();
        let learned = faceness::learn(self.prints(), &pos, &neg, &idx);
        self.set_faceness(self.n(), self.version_faces, learned);
    }

    /// Take in what was learned from the faces as they were at `n` faces and `version_faces`. If the faces
    /// changed meanwhile it is let go of: what was known before stands until they are learned from again.
    pub fn set_faceness(&mut self, n: usize, version_faces: u64, learned: Faceness) {
        if (n, version_faces) == (self.n(), self.version_faces) {
            self.faceness = Some((n, version_faces, Arc::new(learned)));
            self.changed();
        }
    }

    /// What the possible faces are learned from, with a copy of the prints: for `faceness::learn` on a
    /// thread of its own, which takes seconds and needs nothing else of the library.
    pub fn faceness_job(&mut self) -> FacenessJob {
        let (pos, neg) = self.face_sets();
        let maybe = self.maybes();
        let idx = (0..self.n()).filter(|i| maybe.get(&self.store.fid[*i]).is_some_and(|s| *s > 0.0)).collect();
        FacenessJob {
            emb: self.store.prints().to_vec(),
            dim: self.store.dim,
            pos,
            neg,
            idx,
            n: self.n(),
            version_faces: self.version_faces,
        }
    }

    fn learned(&self) -> Option<&Faceness> {
        self.faceness.as_ref().map(|(_, _, f)| &**f)
    }

    /// "Looks like a real face" (0 to 1) for a print the library doesn't have (a box drawn on the spot); none
    /// without a model.
    pub fn faceness(&self, print: &[f32]) -> Option<f64> {
        self.learned()?.model.as_ref().map(|m| m.score(print))
    }

    /// The score a possible face needs for its dashed box: that of the top quarter of them. 0 (show all)
    /// while there isn't enough to learn from, or nothing was learned yet.
    pub fn face_cut(&self) -> f64 {
        self.learned().map_or(0.0, |f| f.cut)
    }

    /// How much possible face i looks like a real one (1 for faces drawn by hand and for faces that aren't
    /// possible faces).
    pub fn face_score(&self, i: usize) -> f64 {
        self.learned().and_then(|f| f.scores.get(&i)).copied().unwrap_or(1.0)
    }

    /// Whether the photo view draws face i: every face except the unnamed possible faces below the cut.
    pub fn shown(&mut self, i: usize) -> bool {
        self.maybe(i).is_none() || self.name(i).is_some() || self.face_score(i) >= self.face_cut()
    }

    /// The possible faces nobody has named and the model doesn't suggest a name for, the most face-like
    /// first: [[face, quality, the detector's score, how face-like, shown]]. A list of their own, so that the
    /// new faces aren't buried in them.
    pub fn possible(&mut self) -> Value {
        let (tri, maybe) = (self.triage(), self.maybes());
        let mut found: Vec<(f64, Value)> = Vec::new();
        for t in tri.iter().filter(|t| !self.suggests(t)) {
            if let Some(detector) = maybe.get(&self.store.fid[t.i]) {
                // unnamed possible faces all: shown when face-like enough
                let (score, shown) =
                    (filejson::round(self.face_score(t.i), 3), self.face_score(t.i) >= self.face_cut());
                found.push((score, json!([t.i, self.q(t.i), detector, score, shown])));
            }
        }
        found.sort_by(|a, b| b.0.total_cmp(&a.0));
        Value::Array(found.into_iter().map(|(_, f)| f).collect())
    }

    // ----- what the photos say about themselves

    /// The photos in the order of their first face.
    fn photo_order(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.store.photo.iter().take(self.n()).filter(|h| seen.insert(h.as_str())).cloned().collect()
    }

    fn photo(&self, h: &str) -> Option<&Map<String, Value>> {
        self.store.photos.get(h).and_then(Value::as_object)
    }

    fn path_of(&self, h: &str) -> Option<&str> {
        self.photo(h).and_then(|p| text(p, "path"))
    }

    /// What one photo's facts are read from.
    fn job(&mut self, h: &str) -> Job {
        let faces: Vec<usize> = self.by_photo().get(h).cloned().unwrap_or_default();
        let boxes = faces
            .iter()
            .map(|i| {
                let b = self.store.boxes[*i];
                [b[0], b[1], b[2], b[3]]
            })
            .collect();
        Job {
            h: h.to_string(),
            path: self.path_of(h).unwrap_or("").to_string(),
            faces,
            boxes,
            known: self.facts.facts.get(h).cloned(),
        }
    }

    /// What to read for every photo with faces (or just these): the photos that are files here and no videos.
    /// Taken under the library's lock; `photofacts::Listings::run` reads them without it.
    pub fn facts_jobs(&mut self, hashes: Option<&[String]>) -> Vec<Job> {
        let todo = hashes.map_or_else(|| self.photo_order(), <[String]>::to_vec);
        let wanted: Vec<String> = (todo.into_iter())
            .filter(|h| self.path_of(h).is_some() && !self.photo(h).is_some_and(|p| has(p, "video")))
            .collect();
        self.facts.status = photofacts::Status { running: true, done: 0, total: wanted.len(), read: 0 };
        wanted.iter().map(|h| self.job(h)).collect()
    }

    /// Take in what was read: the facts are kept, saved if any photo was read anew, and the lists that use
    /// them are worked out again. How many photos were read anew.
    pub fn facts_apply(&mut self, jobs: &[Job], done: Vec<(Fact, bool)>) -> Result<usize> {
        let mut read = 0;
        for (job, (fact, new)) in jobs.iter().zip(done) {
            self.facts.facts.insert(job.h.clone(), fact);
            read += usize::from(new);
        }
        self.facts.status = photofacts::Status { running: false, done: jobs.len(), total: jobs.len(), read };
        if read > 0 {
            self.facts.save(&self.store).map_err(|e| files(format!("can't write {}: {e}", PhotoFacts::FILE)))?;
            self.note_saved();
        }
        self.facts.version += 1;
        Ok(read)
    }

    /// Read every photo with faces (or just these) for its camera date and people tags, here and now.
    pub fn read_facts(&mut self, hashes: Option<&[String]>) -> Result<usize> {
        let jobs = self.facts_jobs(hashes);
        let done = self.facts.listings.clone().run(&jobs);
        self.facts_apply(&jobs, done)
    }

    /// The facts of one photo, read now if they weren't yet.
    pub fn fact(&mut self, h: &str) -> Fact {
        if let Some(f) = self.facts.facts.get(h) {
            return f.clone();
        }
        let job = self.job(h);
        let fact = self.facts.listings.read_safe(&job);
        self.facts.facts.insert(h.to_string(), fact.clone());
        fact
    }

    /// (face, name) from a photo's people tags: the tag rectangles that sit on a face, plus a tag without a
    /// rectangle when it is the only name left and only one face is still unnamed.
    pub fn tag_names(&mut self, h: &str, f: &Fact) -> Vec<(usize, String)> {
        let mut out = f.faces.clone();
        let idx: Vec<usize> = self.by_photo().get(h).cloned().unwrap_or_default();
        let left: Vec<&String> = (f.tags.iter())
            .filter(|t| {
                !out.iter().any(|(_, name)| name == *t) && !idx.iter().any(|i| self.name(*i) == Some(t.as_str()))
            })
            .collect();
        let open: Vec<usize> =
            idx.iter().copied().filter(|i| !out.iter().any(|(face, _)| face == i) && self.name(*i).is_none()).collect();
        if let ([name], [face]) = (left.as_slice(), open.as_slice()) {
            out.push((*face, (*name).clone()));
        }
        out
    }

    /// The unnamed faces that the photo's own people tags name, by that name: [{name, faces: [[face,
    /// quality]], known}], most faces first.
    pub fn tagged(&mut self, show_low: bool) -> Value {
        let with_tags: Vec<(String, Fact)> = self
            .facts
            .facts
            .iter()
            .filter(|(_, f)| !f.faces.is_empty() || !f.tags.is_empty())
            .map(|(h, f)| (h.clone(), f.clone()))
            .collect();
        let mut found: Vec<(String, Vec<Value>)> = Vec::new();
        for (h, f) in &with_tags {
            for (i, name) in self.tag_names(h, f) {
                let said_not = self.rec(i).is_some_and(|r| nots(r).any(|x| x == name));
                if self.name(i).is_none() && !said_not && !self.hidden(i, show_low) {
                    let at = found.iter().position(|(n, _)| *n == name).unwrap_or_else(|| {
                        found.push((name.clone(), Vec::new()));
                        found.len() - 1
                    });
                    found[at].1.push(json!([i, self.q(i)]));
                }
            }
        }
        let known: HashSet<String> = self.people_faces(None).into_iter().map(|(name, _)| name).collect();
        found.sort_by_key(|(_, faces)| std::cmp::Reverse(faces.len()));
        let group = |(name, faces): (String, Vec<Value>)| {
            let is_known = known.contains(&name);
            json!({"name": name, "faces": faces, "known": is_known})
        };
        Value::Array(found.into_iter().map(group).collect())
    }

    /// The named people in the photos of the same occasion: taken within 12 hours of it (by the camera's
    /// date), else the photos next to it in its folder. {how, photos, people: [{name, n, i}]}.
    pub fn occasion(&mut self, h: &str) -> Value {
        let (by_photo, f) = (self.by_photo(), self.fact(h));
        let (near, how): (Vec<String>, String) = match f.t.filter(|t| *t != 0.0) {
            Some(t) => {
                let mut near: Vec<(f64, &String)> = (self.facts.facts.iter())
                    .filter(|(k, _)| k.as_str() != h && by_photo.contains_key(*k))
                    .filter_map(|(k, g)| Some(((g.t.filter(|t| *t != 0.0)? - t).abs(), k)))
                    .filter(|(apart, _)| *apart <= TAKEN_NEAR_H * 3600.0)
                    .collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
                (
                    near.into_iter().take(40).map(|(_, k)| k.clone()).collect(),
                    format!("taken within {} hours of it", TAKEN_NEAR_H as u32),
                )
            }
            None => {
                let folders = self.folders();
                let row = self.path_of(h).and_then(|p| folders.get(&photofiles::path_text(split(p).0)));
                let near = match row.and_then(|row| Some((row, row.iter().position(|k| k == h)?))) {
                    Some((row, k)) => row[k.saturating_sub(FOLDER_NEAR)..(k + FOLDER_NEAR + 1).min(row.len())]
                        .iter()
                        .filter(|x| x.as_str() != h)
                        .cloned()
                        .collect(),
                    None => Vec::new(),
                };
                (near, "next to it in its folder".to_string())
            }
        };
        let mut people: Vec<(&str, usize, usize)> = Vec::new();
        for k in &near {
            for &j in by_photo.get(k).into_iter().flatten() {
                let Some(name) = self.name(j).filter(|n| *n != IGNORE) else { continue };
                match people.iter_mut().find(|(n, ..)| *n == name) {
                    Some(person) => person.1 += 1,
                    None => people.push((name, 1, j)),
                }
            }
        }
        people.sort_by_key(|(_, n, _)| std::cmp::Reverse(*n));
        let people: Vec<Value> = people.iter().map(|(name, n, i)| json!({"name": name, "n": n, "i": i})).collect();
        json!({"how": how, "photos": near.len(), "people": people})
    }

    /// {folder: its photos with faces, in the order of their names}
    fn folders(&mut self) -> Arc<HashMap<String, Vec<String>>> {
        if let Some(f) = &self.cache.folders {
            return f.clone();
        }
        let mut found: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for h in self.photo_order() {
            if let Some(path) = self.path_of(&h) {
                let (folder, name) = split(path);
                found.entry(photofiles::path_text(folder)).or_default().push((name.to_lowercase(), h));
            }
        }
        let found: HashMap<String, Vec<String>> = (found.into_iter())
            .map(|(folder, mut row)| {
                row.sort();
                (folder, row.into_iter().map(|(_, h)| h).collect())
            })
            .collect();
        let found = Arc::new(found);
        self.cache.folders = Some(found.clone());
        found
    }

    /// For a webcam snapshot the tracker saved: who the tracker thought face i was, how sure it was and when
    /// ({name, sim, when}), from the .json saved beside the photo; nothing for other photos.
    pub fn live_verdict(&mut self, i: usize) -> Value {
        if !self.webcam(i) {
            return Value::Null;
        }
        let Some(path) = self.path_of(&self.store.photo[i]) else { return Value::Null };
        let beside = format!("{}.json", photofacts::suffix(path).0);
        let Some(saved) = std::fs::read(beside).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else {
            return Value::Null;
        };
        let b = self.store.boxes[i];
        let mine = [b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64];
        let overlap = |f: &Value| {
            let theirs: Vec<f64> = f["box"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
            <[f64; 4]>::try_from(theirs).map_or(0.0, |t| photofacts::iou(&t, &mine))
        };
        let mut best: Option<(&Value, f64)> = None;
        for f in saved["faces"].as_array().into_iter().flatten() {
            let s = overlap(f);
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((f, s));
            }
        }
        match best {
            Some((f, s)) if s >= 0.3 => {
                json!({"name": f["name"], "sim": f["sim"], "when": saved.get("when").cloned().unwrap_or(json!(""))})
            }
            _ => Value::Null,
        }
    }

    /// Everything that might say who face i is: the closest people (however weak), the photo's people tags,
    /// and who else was photographed at the same occasion. A hint, never a label.
    pub fn hints(&mut self, i: usize) -> Value {
        let h = self.store.photo[i].clone();
        let mut closest = Vec::new();
        if self.name(i).is_none() {
            let (names, rows) = self.score(&[i]);
            let mut order: Vec<usize> = (0..names.len()).collect();
            order.sort_by(|a, b| rows[0][*b].total_cmp(&rows[0][*a]));
            closest = (order.into_iter().filter(|j| names[*j] != IGNORE && rows[0][*j] > 0.0).take(CLOSEST))
                .map(|j| json!({"name": names[j], "s": filejson::round(rows[0][j] as f64, 3)}))
                .collect();
        }
        let f = self.fact(&h);
        let tag = self.tag_names(&h, &f).into_iter().find(|(face, _)| *face == i).map(|(_, name)| name);
        json!({"closest": closest, "tag": tag, "tags": f.tags, "occasion": self.occasion(&h), "live": self.live_verdict(i)})
    }

    /// The photos in which every one of `names` is named, newest first: [{i: a face of the first name there
    /// (for its picture), path, exists, t, date, people: [names]}].
    pub fn photos_with(&mut self, names: &[&str]) -> Value {
        let mut want: Vec<&str> = Vec::new();
        for name in names.iter().filter(|n| !n.is_empty()) {
            if !want.contains(name) {
                want.push(name);
            }
        }
        if want.is_empty() {
            return json!([]);
        }
        let by_photo = self.by_photo();
        let mut out: Vec<(f64, Value)> = Vec::new();
        for h in self.photo_order() {
            // the people named in the photo: each one's last face there
            let mut here: Vec<(&str, usize)> = Vec::new();
            for &i in by_photo.get(&h).into_iter().flatten() {
                let Some(name) = self.name(i).filter(|n| *n != IGNORE) else { continue };
                match here.iter_mut().find(|(n, _)| *n == name) {
                    Some(person) => person.1 = i,
                    None => here.push((name, i)),
                }
            }
            if !want.iter().all(|n| here.iter().any(|(name, _)| name == n)) {
                continue;
            }
            let path = self.path_of(&h).unwrap_or("");
            // the camera's date if it was read already; else the file's
            let t = (self.facts.facts.get(&h).filter(|f| f.t.is_some_and(|t| t != 0.0)).map(Fact::t_value))
                .or_else(|| self.photo(&h).and_then(|p| p.get("mtime")).filter(|m| truthy(m)).cloned())
                .unwrap_or(json!(0));
            let at = t.as_f64().unwrap_or(0.0);
            let mut people: Vec<&str> = here.iter().map(|(name, _)| *name).collect();
            people.sort_unstable();
            let first = here.iter().find(|(name, _)| *name == want[0]).map(|(_, i)| *i);
            let exists = !path.is_empty() && Path::new(path).exists();
            let date = if at != 0.0 { local_day(at) } else { String::new() };
            out.push((at, json!({"i": first, "path": path, "exists": exists, "t": t, "date": date, "people": people})));
        }
        out.sort_by(|a, b| b.0.total_cmp(&a.0));
        Value::Array(out.into_iter().map(|(_, row)| row).collect())
    }

    /// Copy the photos of `rows` (from `photos_with`) into `folder`, keeping their file dates. Files already
    /// there with the same name and size are skipped; other name clashes get " (2)" and so on. {copied,
    /// skipped, missing} (missing: the original photo isn't on this computer).
    pub fn export(rows: &Value, folder: &str) -> Result<Value> {
        let dest = PathBuf::from(strip(folder).trim_matches('"'));
        if !dest.is_absolute() {
            return asked("Give the full path of the folder, e.g. E:\\Pictures\\Export");
        }
        std::fs::create_dir_all(&dest).map_err(files)?;
        let (mut copied, mut skipped, mut missing) = (0, 0, 0);
        for r in rows.as_array().into_iter().flatten() {
            let src = match r["path"].as_str().filter(|_| r["exists"].as_bool() == Some(true)) {
                Some(p) => PathBuf::from(p),
                None => {
                    missing += 1;
                    continue;
                }
            };
            let name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
            let (stem, ending) = photofacts::suffix(&name);
            let size = std::fs::metadata(&src).map_err(files)?.len();
            let (mut to, mut k) = (dest.join(&name), 2);
            while to.exists() && std::fs::metadata(&to).map_err(files)?.len() != size {
                to = dest.join(format!("{stem} ({k}){ending}"));
                k += 1;
            }
            if to.exists() {
                skipped += 1;
                continue;
            }
            std::fs::copy(&src, &to).map_err(files)?;
            if let Ok(at) = std::fs::metadata(&src).and_then(|m| m.modified()) {
                let _ = std::fs::File::options().write(true).open(&to).and_then(|f| f.set_modified(at));
            }
            copied += 1;
        }
        Ok(json!({"copied": copied, "skipped": skipped, "missing": missing}))
    }

    // ----- changes

    /// Delete the prints and pictures of faces and save. Only where each face is stays (face ID, photo, box):
    /// its photo isn't read again, and a re-saved copy of it still counts as a copy. What was deleted, by
    /// face ID, for an Undo in this session.
    pub fn forget_faces(&mut self, idx: &[usize]) -> Result<Vec<(String, Forgotten)>> {
        let saved = idx.iter().map(|&i| (self.store.fid[i].clone(), self.store.forget(i))).collect();
        self.forgot(false);
        self.save_store()?;
        Ok(saved)
    }

    /// Store what was made again and save.
    fn put_back(&mut self, made: &[Remade]) -> Result<()> {
        for m in made {
            self.store.set_print(m.i, &m.emb).map_err(Fault::Files)?;
            self.store.set_thumb(m.i, &m.crop).map_err(Fault::Files)?;
            self.store.quality[m.i] = m.quality;
        }
        self.forgot(true);
        self.save_store()
    }

    /// Faces without print or picture that aren't Ignored: naming an
    /// Ignored face or taking it off the Ignored list once left it like that.
    pub fn lost_prints(&self) -> Vec<usize> {
        (0..self.n()).filter(|&i| self.printless(i) && self.name(i) != Some(IGNORE)).collect()
    }

    /// Give the faces `lost_prints` listed their prints and pictures back, from their photos where those are
    /// on this computer. How many.
    pub fn mend(&mut self, idx: &[usize]) -> Result<usize> {
        let made: Vec<Remade> = match &self.remake {
            Some(remake) => remake(&self.store, idx),
            None => Vec::new(),
        };
        let made: Vec<Remade> =
            made.into_iter().filter(|m| self.printless(m.i) && self.name(m.i) != Some(IGNORE)).collect();
        if !made.is_empty() {
            self.put_back(&made)?;
            self.build_people()?;
        }
        Ok(made.len())
    }

    /// A face's label after `action`, from its label now: empty when nothing is left of it.
    fn relabel(action: &str, old: Option<&Map<String, Value>>, name: Option<&str>) -> Result<Map<String, Value>> {
        let mut r = old.cloned().unwrap_or_default();
        fn drop(r: &mut Map<String, Value>, keys: &[&str]) {
            for k in keys {
                r.shift_remove(*k);
            }
        }
        let name = name.unwrap_or("");
        match action {
            "name" | "ignore" => {
                let n = if action == "name" { name } else { IGNORE };
                r.insert("n".to_string(), json!(n));
                r.insert("by".to_string(), json!("me"));
                drop(&mut r, &["s", "ok"]);
                if has(&r, "not") {
                    let left: Vec<Value> =
                        r["not"].as_array().into_iter().flatten().filter(|x| x.as_str() != Some(n)).cloned().collect();
                    r.insert("not".to_string(), Value::Array(left));
                }
            }
            "reject" => {
                let mut not: BTreeSet<String> = nots(&r).map(str::to_string).collect();
                not.insert(name.to_string());
                r.insert("not".to_string(), json!(not));
                if text(&r, "n") == Some(name) {
                    drop(&mut r, &["n", "by", "s", "ok"]);
                }
            }
            "clear" => drop(&mut r, &["n", "by", "s", "ok"]),
            "confirm" => {
                if has(&r, "n") {
                    r.insert("by".to_string(), json!("me"));
                    r.shift_remove("s");
                }
            }
            "ok" => {
                r.insert("ok".to_string(), json!(true));
            }
            other => return asked(format!("unknown action {other}")),
        }
        r.retain(|_, v| truthy(v));
        Ok(r)
    }

    fn describe(action: &str, n: usize, name: &str) -> String {
        let faces = format!("{n} face{}", if n == 1 { "" } else { "s" });
        match action {
            "name" => format!("Named {faces} {name}"),
            "ignore" => format!("Ignored {faces}"),
            "reject" => format!("Marked {faces} as not {name}"),
            "clear" => format!("Removed the name from {faces}"),
            "confirm" => format!("Confirmed {faces}"),
            "ok" => format!("Kept the name on {faces}"),
            other => other.to_string(),
        }
    }

    /// A name given by hand is a person again: an earlier forget of that name no longer applies to them.
    fn named_again(&self, name: &str) {
        let _ = forgotten::unforget(&self.home, name);
    }

    /// Change labels. action: name | ignore | reject (not this person) | clear | confirm | ok. How many faces
    /// were changed.
    pub fn act(&mut self, action: &str, idx: &[usize], name: Option<&str>, text: Option<&str>) -> Result<usize> {
        let name = name.map(strip).filter(|n| !n.is_empty());
        let mut action = action;
        if matches!(action, "name" | "reject") && name.is_none() {
            return asked("a name is needed");
        }
        if action == "name" && name == Some(IGNORE) {
            action = "ignore"; // the same thing: the print and picture go too
        }
        if action == "name" && name.is_some_and(|n| n.starts_with('#')) {
            return asked("names can't start with #");
        }
        if let Some(n) = name.filter(|n| action == "name" && NOT_NAMES.contains(n)) {
            return asked(format!("\"{n}\" can't be used as a name"));
        }
        // a face given twice would be undone to its changed label
        let mut seen = HashSet::new();
        let mut idx: Vec<usize> = idx.iter().copied().filter(|i| seen.insert(*i)).collect();
        // all of them first: a bad one mustn't leave the others half done
        if let Some(i) = idx.iter().find(|i| **i >= self.n()) {
            return asked(format!("no face {i}"));
        }
        let mut new: HashMap<usize, Map<String, Value>> = HashMap::new();
        for &i in &idx {
            new.insert(i, Self::relabel(action, self.rec(i), name)?);
        }
        // An Ignored face has no print or picture left. One that gets a name, or comes off the Ignored list,
        // needs them again: else it's a grey placeholder that teaches nothing, or shows in no list at all.
        let lost: Vec<usize> = (idx.iter().copied())
            .filter(|i| {
                let to = crate::labels::text(&new[i], "n");
                self.printless(*i) && to != Some(IGNORE) && to != self.name(*i)
            })
            .collect();
        let made: Vec<Remade> = match &self.remake {
            Some(remake) if !lost.is_empty() => remake(&self.store, &lost),
            _ => Vec::new(),
        };
        if made.len() < lost.len() {
            // those stay as they are; the others are still changed
            idx.retain(|i| made.iter().any(|m| m.i == *i) || !lost.contains(i));
            if idx.is_empty() {
                return asked(NO_PRINT);
            }
        }
        if !made.is_empty() {
            self.put_back(&made)?;
        }
        let mut before = Vec::new();
        for &i in &idx {
            let fid = self.store.fid[i].clone();
            before.push((fid.clone(), self.labels.get(&fid).filter(|old| truthy(old)).cloned()));
            match new.remove(&i).filter(|r| !r.is_empty()) {
                Some(r) => self.labels.insert(fid, Value::Object(r)),
                None => self.labels.shift_remove(&fid),
            };
        }
        let changed = !before.is_empty();
        let words = text.map_or_else(|| Self::describe(action, idx.len(), name.unwrap_or("")), str::to_string);
        self.commit(before, words)?;
        if changed && !made.is_empty() {
            // Undo makes them Ignored again: the print and picture go again
            let remade = made.iter().map(|m| self.store.fid[m.i].clone()).collect();
            if let Some(step) = self.undo.last_mut() {
                step.remade = remade;
            }
        }
        if changed && action == "ignore" {
            // an ignored face's print and picture aren't kept
            let prints = self.forget_faces(&idx)?;
            if let Some(step) = self.undo.last_mut() {
                step.prints = prints;
            }
        }
        if let Some(n) = name.filter(|_| action == "name" && !idx.is_empty()) {
            self.named_again(n);
        }
        Ok(idx.len())
    }

    /// Give everyone called `old` the name `new` (someone who has it already: the two become one person).
    /// How many labels changed.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<usize> {
        let new = strip(new);
        if new.is_empty() || new.starts_with('#') {
            return asked("give a new name");
        }
        if NOT_NAMES.contains(&new) {
            return asked(format!("\"{new}\" can't be used as a name"));
        }
        if old.starts_with('#') {
            return asked("that isn't a person's name"); // Ignored isn't a person
        }
        let merge = self.people_faces(None).iter().any(|(name, _)| name == new);
        let mut before = Vec::new();
        for (fid, label) in self.labels.iter_mut() {
            let Some(r) = label.as_object() else { continue };
            let (hit_n, hit_not) = (text(r, "n") == Some(old), nots(r).any(|x| x == old));
            if !(hit_n || hit_not) {
                continue;
            }
            before.push((fid.clone(), Some(label.clone())));
            let mut r = r.clone();
            if hit_n {
                r.insert("n".to_string(), json!(new));
            }
            if hit_not {
                let not: BTreeSet<&str> = nots(&r).map(|x| if x == old { new } else { x }).collect();
                let not = json!(not);
                r.insert("not".to_string(), not);
            }
            if text(&r, "n") == Some(new) && has(&r, "not") {
                // merged: a face can't be Bob and "not Bob"
                let left: Vec<Value> =
                    r["not"].as_array().into_iter().flatten().filter(|x| x.as_str() != Some(new)).cloned().collect();
                r.insert("not".to_string(), Value::Array(left));
            }
            r.retain(|_, v| truthy(v));
            *label = Value::Object(r);
        }
        let changed = before.len();
        self.commit(
            before,
            if merge { format!("Merged {old} into {new}") } else { format!("Renamed {old} to {new}") },
        )?;
        if changed > 0 {
            self.named_again(new);
        }
        Ok(changed)
    }

    /// Take back the last change. What it was, in words; none when there is nothing to undo.
    pub fn undo(&mut self) -> Result<Option<String>> {
        let Some(step) = self.undo.pop() else { return Ok(None) };
        for (fid, old) in &step.before {
            match old {
                Some(label) => self.labels.insert(fid.clone(), label.clone()),
                None => self.labels.shift_remove(fid),
            };
        }
        if !step.prints.is_empty() || !step.remade.is_empty() {
            for (fid, kept) in step.prints {
                if let Some(i) = self.store.index_of(&fid) {
                    self.store.restore(i, kept); // Undo of Ignore: the print and picture come back
                }
            }
            for fid in &step.remade {
                let ignored =
                    self.labels.get(fid).and_then(Value::as_object).and_then(|r| text(r, "n")) == Some(IGNORE);
                if let Some(i) = self.store.index_of(fid).filter(|_| ignored) {
                    self.store.forget(i); // Ignored again: without print or picture, as before
                }
            }
            self.forgot(false); // back in its group, or out of it again
            self.save_store()?;
        }
        let fids: Vec<String> = step.before.iter().map(|(fid, _)| fid.clone()).collect();
        self.save(&format!("Undo: {}", step.text), &fids)?;
        Ok(Some(step.text))
    }

    /// Whether there is a change to take back, and what it was.
    pub fn can_undo(&self) -> Option<&str> {
        self.undo.last().map(|s| s.text.as_str())
    }

    fn commit(&mut self, before: Vec<(String, Option<Value>)>, text: String) -> Result<()> {
        if before.is_empty() {
            return Ok(());
        }
        let fids: Vec<String> = before.iter().map(|(fid, _)| fid.clone()).collect();
        self.undo.push(Step { before, text: text.clone(), prints: Vec::new(), remade: Vec::new() });
        if self.undo.len() > UNDO_KEPT {
            self.undo.drain(..self.undo.len() - UNDO_KEPT);
        }
        self.save(&text, &fids)
    }

    /// Nothing can be undone from here on (a person was forgotten: an Undo could bring their name back).
    pub(crate) fn clear_undo(&mut self) {
        self.undo.clear();
    }

    /// The labels saved, the change added to the history, and the people the tracker recognizes written again.
    pub(crate) fn save(&mut self, text: &str, fids: &[String]) -> Result<()> {
        self.changed();
        self.save_labels()?;
        let mut faces = Map::new();
        for fid in fids {
            faces.insert(fid.clone(), self.labels.get(fid).cloned().unwrap_or(Value::Null));
        }
        let line = filejson::dumps(&json!({"t": utc_now(), "what": text, "faces": faces})) + END;
        let said = |e: std::io::Error| files(format!("can't write history.jsonl: {e}"));
        let mut history = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.data.join("history.jsonl"))
            .map_err(said)?;
        history.write_all(line.as_bytes()).map_err(said)?;
        self.build_people()?;
        Ok(())
    }

    /// `data/people.npz`: the prints of every named, usable face, which is what the live tracker loads. How
    /// many faces each person has in it.
    pub fn build_people(&mut self) -> Result<Vec<(String, usize)>> {
        let gallery = self.gallery();
        let (mut emb, mut who, mut counts) = (Vec::new(), Vec::new(), Vec::new());
        for &g in &gallery.seen {
            let (name, faces) = &gallery.people[g];
            if name == IGNORE {
                continue;
            }
            for &i in faces {
                emb.extend_from_slice(self.store.print(i));
                who.push(name.clone());
            }
            counts.push((name.clone(), faces.len()));
        }
        let (tmp, dest) = (self.data.join("people.tmp.npz"), self.data.join("people.npz"));
        let said = |e: std::io::Error| files(format!("can't write people.npz: {e}"));
        let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(said)?);
        let arrays = [
            ("emb", Array::table(emb, who.len(), self.store.dim)),
            ("name", Array::texts(who)),
            ("model", Array::one_text(&self.store.model)),
        ];
        npy::write_npz(&arrays, &mut out).map_err(said)?;
        out.flush().map_err(said)?;
        std::mem::drop(out);
        replace_file(&tmp, &dest).map_err(said)?;
        Ok(counts)
    }

    /// Name the unnamed faces the model is very sure about (score at least the model's `auto` and clearly ahead of the
    /// next person), of all faces or of `idx`. How many: these are marked "auto" and can be reviewed later.
    pub fn autoname(&mut self, idx: Option<&[usize]>) -> Result<usize> {
        let (tri, gallery) = (self.triage(), self.gallery());
        let want: Option<HashSet<usize>> = idx.map(|idx| idx.iter().copied().collect());
        let hits: Vec<(usize, &str, f64)> = (tri.iter())
            .filter_map(|t| {
                let (s, s2) = (t.s as f64, t.s2 as f64);
                let sure = s >= self.model.auto
                    && s - s2 >= MARGIN
                    && self.usable(t.i)
                    && want.as_ref().is_none_or(|w| w.contains(&t.i));
                Some((t.i, gallery.people[t.name?].0.as_str(), s)).filter(|_| sure)
            })
            .collect();
        for (i, name, s) in &hits {
            let mut r = self.rec(*i).cloned().unwrap_or_default();
            r.insert("n".to_string(), json!(name));
            r.insert("by".to_string(), json!("auto"));
            r.insert("s".to_string(), json!(filejson::round(*s, 3)));
            self.labels.insert(self.store.fid[*i].clone(), Value::Object(r));
        }
        if !hits.is_empty() {
            self.changed();
            self.save_labels()?;
            self.build_people()?;
        }
        Ok(hits.len())
    }

    /// One copy of faces.npz and thumbs.npz before the faces Ignored long ago lose their prints and pictures
    /// (the labels' backups don't hold them, so it couldn't be undone otherwise). The folder.
    fn backup_before_forgetting(&self) -> Result<PathBuf> {
        let dir =
            self.data.join("backups").join(format!("before-v1.9-ignore-{}", utc_stamp(std::time::SystemTime::now())));
        std::fs::create_dir_all(&dir).map_err(files)?;
        for name in ["faces.npz", "thumbs.npz"] {
            if self.data.join(name).exists() {
                std::fs::copy(self.data.join(name), dir.join(name)).map_err(files)?;
            }
        }
        Ok(dir)
    }

    /// A copy of labels.json as it is, in `data/backups`; the newest `keep` are kept.
    pub fn backup(&self, keep: usize) -> Result<()> {
        let src = self.data.join("labels.json");
        if !src.exists() {
            return Ok(());
        }
        let dir = self.data.join("backups");
        std::fs::create_dir_all(&dir).map_err(files)?;
        std::fs::copy(&src, dir.join(format!("labels-{}.json", utc_stamp(std::time::SystemTime::now()))))
            .map_err(files)?;
        let mut old: Vec<PathBuf> = (std::fs::read_dir(&dir).map_err(files)?.flatten().map(|f| f.path()))
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("labels-") && n.ends_with(".json"))
            })
            .collect();
        old.sort();
        for gone in old.iter().take(old.len().saturating_sub(keep)) {
            let _ = std::fs::remove_file(gone);
        }
        Ok(())
    }

    // ----- duplicates

    /// True if a photo's faces are the same faces as an already-read photo's (a copy of it): the same number
    /// of faces, each with a near-identical print. `embs`: the new photo's regular faces' prints, one after
    /// another.
    pub fn same_faces(&mut self, photo: &str, embs: &[f32], sim: f32) -> bool {
        let (by_photo, maybe, dim) = (self.by_photo(), self.maybes(), self.store.dim);
        // the photo's regular faces: its possible and drawn faces aren't among the new photo's either
        let old: Vec<usize> = (by_photo.get(photo).into_iter().flatten().copied())
            .filter(|i| !maybe.contains_key(&self.store.fid[*i]))
            .collect();
        let new: Vec<&[f32]> = embs.chunks_exact(dim.max(1)).collect();
        if old.len() != new.len() {
            return false;
        }
        // Ignored faces have no print left: they can't be compared, so the photo's other faces decide (with
        // only Ignored faces, the picture fingerprint alone): else a copy would bring an ignored face back
        let keep: Vec<usize> = old.iter().copied().filter(|i| !self.printless(*i)).collect();
        if new.is_empty() || keep.is_empty() {
            return true;
        }
        let alike = |a: &[f32], i: usize| dot(a, self.store.print(i)) >= sim;
        if keep.len() < old.len() {
            keep.iter().all(|i| new.iter().any(|a| alike(a, *i))) // each remaining old face is among the new ones
        } else {
            new.iter().all(|a| keep.iter().any(|i| alike(a, *i)))
        }
    }

    /// The content hash of an already-read photo that looks the same as the picture fingerprint `fp` (64
    /// hex digits), else none. Copies already skipped aren't compared with.
    pub fn near_duplicate(&mut self, fp: &str) -> Option<String> {
        fn bits(hex: &str) -> Option<[u8; 32]> {
            if hex.len() != 64 || !hex.is_ascii() {
                return None;
            }
            let mut out = [0u8; 32];
            for (k, b) in out.iter_mut().enumerate() {
                *b = u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).ok()?;
            }
            Some(out)
        }
        let counts = |p: &Value| p.as_object().is_some_and(|p| has(p, "fp") && !has(p, "dup_of"));
        let n = self.store.photos.values().filter(|p| counts(p)).count();
        if self.fps.as_ref().is_none_or(|(keys, _)| keys.len() != n) {
            let (mut keys, mut fps) = (Vec::new(), Vec::new());
            for (h, p) in self.store.photos.iter().filter(|(_, p)| counts(p)) {
                keys.push(h.clone());
                fps.push(p["fp"].as_str().and_then(bits).unwrap_or([0xff; 32]));
            }
            self.fps = Some((keys, fps));
        }
        let (keys, fps) = self.fps.as_ref()?;
        let fp = bits(fp)?;
        let apart = |other: &[u8; 32]| other.iter().zip(&fp).map(|(a, b)| (a ^ b).count_ones()).sum::<u32>();
        let (mut best, mut least) = (None, u32::MAX);
        for (k, other) in fps.iter().enumerate() {
            let d = apart(other);
            if d < least {
                (best, least) = (Some(k), d);
            }
        }
        best.filter(|_| least <= DUP_BITS).map(|k| keys[k].clone())
    }
}

#[cfg(test)]
mod tests {
    use face_core::faces::REC_MODELS;

    use super::*;
    use crate::scores::score_anew;

    const DIM: usize = 8;

    fn folders(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("face-labels-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (dir.join("data"), dir.join("home"))
    }

    fn model() -> RecModel {
        RecModel { dim: DIM, ..REC_MODELS[0] }
    }

    /// A print near one of four directions (a person each), `tilt` away from it toward a direction of the
    /// face's own.
    fn print(person: usize, face: usize, tilt: f32) -> Vec<f32> {
        let mut v = vec![0f32; DIM];
        v[person] = 1.0;
        v[4 + face % 4] = tilt;
        let long = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / long).collect()
    }

    /// A library of 4 people with `each` faces each (person p's faces are p, p + 4, p + 8, ...), nobody named.
    fn library(name: &str, each: usize) -> LabelDb {
        let (data, home) = folders(name);
        let mut store = Store::load(&data, model().name, DIM).unwrap();
        let crop = Bgr { w: 112, h: 112, px: vec![90; 112 * 112 * 3] };
        for face in 0..each * 4 {
            let (person, photo) = (face % 4, format!("{:016x}{:04}", 0xabc0 + face / 3, face / 3));
            let tilt = 0.2 + 0.1 * ((face / 4) % 5) as f32;
            let row = [10.0 * face as f32, 5.0, 40.0, 40.0, 0.9];
            store.photos.entry(photo.clone()).or_insert_with(|| json!({"path": format!("/photos/p{}.jpg", face / 3)}));
            store.add(&photo, "p.jpg", &row, &print(person, face / 4, tilt), &crop, 0.8, None).unwrap();
        }
        store.save().unwrap();
        LabelDb::open(&data, &home, model(), false, &mut |_| {}).unwrap()
    }

    fn faces_of(db: &LabelDb, person: usize) -> Vec<usize> {
        (0..db.n()).filter(|i| i % 4 == person).collect()
    }

    /// The kept scores are what scoring anew gives, and the kept stacks what working them out anew gives.
    fn fresh(db: &mut LabelDb, when: &str) {
        let gallery = db.scored();
        for (name, faces) in &gallery.people {
            for i in 0..db.n() {
                let (kept, anew) = (db.scores.of(name, i), score_anew(db.prints(), i, faces));
                assert!(kept.to_bits() == anew.to_bits(), "{when}: face {i} for {name}: kept {kept}, anew {anew}");
            }
        }
        let kept = db.stacks();
        (db.stacked, db.stacks) = (None, None);
        assert_eq!(*kept, *db.stacks(), "{when}: the stacks");
    }

    fn history(db: &LabelDb) -> Vec<Value> {
        let text = std::fs::read_to_string(db.data.join("history.jsonl")).unwrap_or_default();
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn people_file(db: &LabelDb) -> (Vec<String>, usize) {
        let z = npy::read_npz(&std::fs::read(db.data.join("people.npz")).unwrap()).unwrap();
        (z["name"].text().unwrap().to_vec(), z["emb"].shape[0])
    }

    #[test]
    fn naming_teaches_the_recognizer_and_every_change_can_be_undone() {
        let mut db = library("naming", 6);
        assert!(db.triage().iter().all(|t| t.name.is_none() && t.s == -1.0), "nobody is named yet");
        assert_eq!(db.act("name", &[0, 4, 4, 8], Some("  Ann "), None).unwrap(), 3, "a face given twice counts once");
        assert_eq!(db.act("name", &[1, 5], Some("Bob"), None).unwrap(), 2);
        fresh(&mut db, "named");
        assert_eq!(db.labels[&db.store.fid[0]], json!({"n": "Ann", "by": "me"}));
        assert_eq!(db.gallery().people, [("Ann".to_string(), vec![0, 4, 8]), ("Bob".to_string(), vec![1, 5])]);
        assert_eq!(people_file(&db), (["Ann", "Ann", "Ann", "Bob", "Bob"].map(String::from).to_vec(), 5));
        // the rest of Bob's faces are suggested for him and Ann's for her (who has more to look at comes first),
        // each person's least sure first
        let suggested = db.suggested(false);
        assert_eq!((&suggested[0]["name"], &suggested[1]["name"]), (&json!("Bob"), &json!("Ann")));
        assert_eq!(suggested[0]["faces"].as_array().unwrap().len(), 4);
        let ann: Vec<u64> = suggested[1]["faces"].as_array().unwrap().iter().map(|f| f[0].as_u64().unwrap()).collect();
        assert_eq!(ann.len(), 3);
        let scores: Vec<f64> =
            suggested[1]["faces"].as_array().unwrap().iter().map(|f| f[1].as_f64().unwrap()).collect();
        assert!(scores.windows(2).all(|w| w[0] <= w[1]) && scores[0] >= 0.35);
        // the other two people's faces are unknown, in two groups
        let unknown = db.unknown(false, false);
        assert_eq!(unknown["groups"].as_array().unwrap().len(), 2);
        assert_eq!(unknown["groups"][0]["faces"].as_array().unwrap().len(), 6);
        assert!(unknown["groups"][0]["hint"].is_null() && unknown["singles"].as_array().unwrap().is_empty());
        // the very sure ones are named automatically, and can be reviewed
        let named = db.autoname(None).unwrap();
        assert!(named >= 4, "{named} named automatically");
        fresh(&mut db, "named automatically");
        let auto = db.auto_review();
        let anns = auto.as_array().unwrap().iter().find(|g| g["name"] == "Ann").unwrap();
        let face = anns["faces"][0][0].as_u64().unwrap() as usize;
        let label = db.rec(face).unwrap().clone();
        assert_eq!((label["by"].as_str(), label["n"].as_str()), (Some("auto"), Some("Ann")));
        assert!(label["s"].as_f64().unwrap() >= model().auto);
        // confirming makes it the owner's, without the score
        db.act("confirm", &[face], None, None).unwrap();
        assert_eq!(db.labels[&db.store.fid[face]], json!({"n": "Ann", "by": "me"}));
        assert_eq!(db.undo().unwrap().as_deref(), Some("Confirmed 1 face"));
        assert_eq!(db.rec(face), Some(&label), "as it was, in the same order");
        // "not this person": never suggested again, and the name goes when it was theirs
        db.act("reject", &[face], Some("Ann"), None).unwrap();
        assert_eq!(db.labels[&db.store.fid[face]], json!({"not": ["Ann"]}));
        let again = db.triage().iter().find(|t| t.i == face).cloned().unwrap();
        assert!(again.name != db.gallery().pos("Ann") || again.s == -1.0);
        fresh(&mut db, "rejected");
        // the history has every change, the last one last
        let lines = history(&db);
        assert_eq!(lines[0]["what"], "Named 3 faces Ann");
        assert_eq!(lines.last().unwrap()["what"], "Marked 1 face as not Ann");
        assert_eq!(lines.last().unwrap()["faces"][&db.store.fid[face]], json!({"not": ["Ann"]}));
        assert_eq!(lines[3]["what"], "Undo: Confirmed 1 face");
        assert!(lines[0]["t"].as_str().unwrap().ends_with('Z'));
        // what can't be asked
        assert_eq!(db.act("name", &[2], None, None), asked("a name is needed"));
        assert_eq!(db.act("name", &[2], Some("#x"), None), asked("names can't start with #"));
        assert_eq!(db.act("name", &[2], Some("constructor"), None), asked("\"constructor\" can't be used as a name"));
        assert_eq!(db.act("name", &[2, 999], Some("Cy"), None), asked("no face 999"));
        assert_eq!(db.act("shout", &[2], None, None), asked("unknown action shout"));
        assert!(db.rec(2).is_none(), "a bad one left the others alone");
    }

    #[test]
    fn an_ignored_face_loses_its_print_and_picture_and_undo_gives_them_back() {
        let mut db = library("ignore", 5);
        db.act("name", &faces_of(&db, 0)[..3], Some("Ann"), None).unwrap();
        fresh(&mut db, "named");
        let unknown = db.unknown(false, false);
        let before = (db.store.print(7).to_vec(), db.store.thumbs[7].clone(), unknown.clone());
        assert_eq!(db.act("name", &[7, 0], Some(IGNORE), None).unwrap(), 2, "naming someone #ignore is Ignore");
        assert!(db.printless(7) && db.printless(0) && db.store.thumbs[7].is_empty());
        assert_eq!(db.labels[&db.store.fid[7]], json!({"n": IGNORE, "by": "me"}));
        assert_eq!(db.gallery().faces("Ann").len(), 2, "an Ignored face teaches nothing");
        assert!(db.triage().iter().all(|t| t.i != 7), "and is in no list");
        fresh(&mut db, "ignored");
        // read again from the files: the same
        let (data, home) = (db.data.clone(), db.home.clone());
        let again = LabelDb::open(&data, &home, model(), true, &mut |_| {}).unwrap();
        assert!(again.printless(7) && again.name(7) == Some(IGNORE) && again.name(0) == Some(IGNORE));
        assert_eq!(std::fs::read_dir(data.join("backups")).unwrap().count(), 1, "the labels were backed up");
        // naming an Ignored face without its photo's models at hand: it stays as it is
        assert_eq!(db.act("name", &[7], Some("Bob"), None), asked(NO_PRINT));
        assert_eq!(db.act("name", &[7, 2], Some("Bob"), None).unwrap(), 1, "the others are still named");
        assert_eq!((db.name(7), db.name(2)), (Some(IGNORE), Some("Bob")));
        db.undo().unwrap();
        // Undo of the Ignore: label, print and picture as before
        assert_eq!(db.undo().unwrap().as_deref(), Some("Ignored 2 faces"));
        assert_eq!((db.store.print(7), &db.store.thumbs[7]), (before.0.as_slice(), &before.1));
        assert_eq!((db.name(7), db.name(0)), (None, Some("Ann")));
        assert_eq!(db.unknown(false, false), before.2, "back in its group");
        fresh(&mut db, "ignore undone");
        assert_eq!(db.undo().unwrap().as_deref(), Some("Named 3 faces Ann"));
        assert_eq!(db.undo().unwrap(), None);
        assert!(db.labels.is_empty() && people_file(&db).1 == 0);
    }

    #[test]
    fn a_face_made_again_gets_its_name_and_undo_ignores_it_again() {
        let mut db = library("remake", 4);
        db.act("ignore", &[3], None, None).unwrap();
        let made = print(3, 0, 0.25);
        let again = made.clone();
        db.remake = Some(Box::new(move |_, idx| {
            (idx.iter().map(|&i| Remade {
                i,
                emb: again.clone(),
                crop: Bgr { w: 112, h: 112, px: vec![7; 112 * 112 * 3] },
                quality: 0.6,
            }))
            .collect()
        }));
        assert_eq!(db.act("name", &[3], Some("Dee"), None).unwrap(), 1);
        assert_eq!((db.store.print(3), db.store.quality[3], db.name(3)), (made.as_slice(), 0.6, Some("Dee")));
        assert!(!db.store.thumbs[3].is_empty() && db.gallery().faces("Dee") == [3]);
        fresh(&mut db, "made again");
        db.undo().unwrap();
        assert!(db.printless(3) && db.name(3) == Some(IGNORE), "Ignored again, without print or picture");
        // taken off the Ignored list: made again too; and a face left without a print by an older version is mended
        db.act("clear", &[3], None, None).unwrap();
        assert!(!db.printless(3) && db.name(3).is_none());
        db.store.forget(6);
        db.forgot(true);
        assert_eq!(db.lost_prints(), [6]);
        assert_eq!(db.mend(&[6]).unwrap(), 1);
        assert!(db.lost_prints().is_empty());
        fresh(&mut db, "mended");
    }

    #[test]
    fn people_are_renamed_and_merged_and_a_forgotten_name_given_again_is_a_person_again() {
        let mut db = library("rename", 5);
        db.act("name", &faces_of(&db, 0), Some("Ann"), None).unwrap();
        db.act("name", &faces_of(&db, 1)[..3], Some("Bob"), None).unwrap();
        db.act("reject", &[2, 6], Some("Bob"), None).unwrap();
        db.act("reject", &[0], Some("Bob"), None).unwrap();
        forgotten::note(&db.home, "Robert").unwrap();
        assert_eq!(db.rename("Bob", " Robert ").unwrap(), 6);
        assert_eq!(history(&db).last().unwrap()["what"], "Renamed Bob to Robert");
        assert_eq!(db.labels[&db.store.fid[2]], json!({"not": ["Robert"]}));
        assert_eq!(db.labels[&db.store.fid[0]], json!({"n": "Ann", "by": "me", "not": ["Robert"]}));
        assert!(forgotten::names(&db.home, true).unwrap().is_empty(), "the name is a person's again");
        fresh(&mut db, "renamed");
        // merged into Ann: a face can't be Ann and "not Ann"
        assert_eq!(db.rename("Robert", "Ann").unwrap(), 6);
        assert_eq!(history(&db).last().unwrap()["what"], "Merged Robert into Ann");
        assert_eq!(db.labels[&db.store.fid[0]], json!({"n": "Ann", "by": "me"}));
        assert_eq!(db.labels[&db.store.fid[2]], json!({"not": ["Ann"]}));
        assert_eq!(db.gallery().people.len(), 1);
        assert_eq!(db.gallery().faces("Ann").len(), 8);
        fresh(&mut db, "merged");
        db.undo().unwrap();
        assert_eq!(db.gallery().people.len(), 2);
        assert_eq!(db.rename("Ann", ""), asked("give a new name"));
        assert_eq!(db.rename("Ann", "#x"), asked("give a new name"));
        assert_eq!(db.rename("#ignore", "Ann"), asked("that isn't a person's name"));
        assert_eq!(db.rename("Ann", "valueOf"), asked("\"valueOf\" can't be used as a name"));
        assert_eq!(db.rename("Nobody", "Somebody").unwrap(), 0);
    }

    #[test]
    fn mislabeled_faces_and_people_with_few_faces_are_pointed_out() {
        let mut db = library("checks", 6);
        db.act("name", &faces_of(&db, 0), Some("Ann"), None).unwrap();
        db.act("name", &faces_of(&db, 1), Some("Bob"), None).unwrap();
        db.act("name", &[2], Some("Cy"), None).unwrap();
        db.act("name", &[5], Some("Ann"), None).unwrap(); // one of Bob's, named Ann
        let checks = db.checks();
        let bad = checks["mislabeled"].as_array().unwrap();
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert_eq!(
            (bad[0]["i"].as_u64(), bad[0]["name"].as_str(), bad[0]["other"].as_str()),
            (Some(5), Some("Ann"), Some("Bob"))
        );
        assert!(bad[0]["os"].as_f64() > bad[0]["s"].as_f64() && bad[0]["by"] == "me");
        assert_eq!(checks["few"], json!([{"name": "Cy", "usable": 1}]));
        // looked at and kept: no longer pointed out
        db.act("ok", &[5], None, None).unwrap();
        assert!(db.checks()["mislabeled"].as_array().unwrap().is_empty());
        assert_eq!(db.labels[&db.store.fid[5]], json!({"n": "Ann", "by": "me", "ok": true}));
        let summary = db.summary();
        assert_eq!(
            (summary["faces"].as_u64(), summary["checks"].as_u64(), summary["canUndo"].as_bool()),
            (Some(24), Some(1), Some(true))
        );
        assert_eq!(summary["people"][0], json!({"name": "Ann", "faces": 7, "usable": 7, "cover": 0, "auto": 0}));
        assert_eq!(summary["undoText"], "Kept the name on 1 face");
    }

    #[test]
    fn the_same_picture_again_is_one_stack_and_copies_of_photos_are_told() {
        let mut db = library("stacks", 4);
        assert!(db.stacks().is_empty());
        // two more faces with the print of face 5, and one with face 2's
        let crop = Bgr { w: 112, h: 112, px: vec![90; 112 * 112 * 3] };
        for (k, like) in [5usize, 5, 2].into_iter().enumerate() {
            let emb = db.store.print(like).to_vec();
            db.store
                .add("copy0000000000000000", "c.jpg", &[50.0 * k as f32, 0.0, 9.0, 9.0, 0.9], &emb, &crop, 0.8, None)
                .unwrap();
        }
        db.changed();
        let stacks = db.stacks();
        assert_eq!(
            (stacks.get(&16), stacks.get(&17), stacks.get(&5), stacks.get(&18), stacks.get(&2)),
            (Some(&5), Some(&5), Some(&5), Some(&2), Some(&2))
        );
        assert_eq!(stacks.len(), 5);
        fresh(&mut db, "copies added");
        // one of a stack is Ignored: the stack goes on without it; undone: it is back
        db.act("ignore", &[5], None, None).unwrap();
        assert_eq!(db.stacks().get(&17), Some(&16));
        fresh(&mut db, "one of a stack ignored");
        db.undo().unwrap();
        assert_eq!(db.stacks().get(&17), Some(&5));
        fresh(&mut db, "and back");
        // a copy of a photo: the same number of regular faces, each with a near-identical print
        let photo = db.store.photo[3].clone();
        let its: Vec<usize> = db.by_photo()[&photo].clone();
        let prints: Vec<f32> = its.iter().rev().flat_map(|i| db.store.print(*i).to_vec()).collect();
        assert!(db.same_faces(&photo, &prints, 0.90));
        assert!(!db.same_faces(&photo, &prints[DIM..], 0.90), "one face fewer");
        let other: Vec<f32> = its.iter().flat_map(|_| print(3, 3, 0.9)).collect();
        assert!(!db.same_faces(&photo, &other, 0.90));
        db.act("ignore", &[its[0]], None, None).unwrap();
        assert!(db.same_faces(&photo, &prints, 0.90), "the remaining faces decide");
        // picture fingerprints a few bits apart are the same picture
        let fp = |last: &str| format!("{}{last}", "ab".repeat(31));
        db.store.photos.insert("h1".to_string(), json!({"fp": fp("00")}));
        db.store.photos.insert("h2".to_string(), json!({"fp": fp("ff"), "dup_of": "h1"}));
        assert_eq!(db.near_duplicate(&fp("03")).as_deref(), Some("h1"));
        assert_eq!(
            db.near_duplicate(&format!("{}{}", "ab".repeat(29), "54a400")),
            Some("h1".to_string()),
            "12 bits apart"
        );
        assert_eq!(db.near_duplicate(&format!("{}{}", "ab".repeat(29), "54a401")), None, "13 bits apart");
        assert_eq!(db.near_duplicate("not a fingerprint"), None);
    }

    #[test]
    fn webcam_faces_named_automatically_wait_for_a_confirmation() {
        let mut db = library("webcam", 5);
        db.act("name", &faces_of(&db, 0)[..3], Some("Ann"), None).unwrap();
        // the photo of faces 12, 13, 14 is a snapshot the tracker saved; 12 is Ann's
        let (photo, shot) = (db.store.photo[12].clone(), db.home.join("webcam").join("shot.jpg"));
        db.store.photos.insert(photo, json!({"path": shot.to_string_lossy()}));
        db.changed();
        assert!(db.webcam(12) && !db.webcam(0));
        assert!(db.autoname(None).unwrap() >= 2);
        assert_eq!(db.by(12), "auto");
        assert!(!db.gallery().faces("Ann").contains(&12) && db.gallery().faces("Ann").contains(&16));
        db.act("confirm", &[12], None, None).unwrap();
        assert!(db.gallery().faces("Ann").contains(&12));
        assert_eq!(
            folder_key(if cfg!(windows) { "C:/Users//x/./a/../Webcam/" } else { "/home//x/./a/../Webcam/" }),
            if cfg!(windows) { "c:\\users\\x\\webcam" } else { "/home/x/Webcam" }
        );
    }

    #[test]
    fn the_photos_own_facts_give_clues_to_who_a_face_is() {
        use crate::exif::tests::{jpeg_file, tiff};
        let mut db = library("facts", 3); // 12 faces, three to a photo: four photos
        let dir = db.data.parent().unwrap().join("photos");
        std::fs::create_dir_all(&dir).unwrap();
        let photos = db.photo_order();
        assert_eq!(photos.len(), 4);
        // the first photo has a camera date and is tagged with a name; the second was taken two hours later;
        // the other two say nothing
        let tagged = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
            <rdf:Description xmlns:I=\"http://iptc.org/std/Iptc4xmpExt/2008-02-29/\"><I:PersonInImage><rdf:Bag><rdf:li>Zed</rdf:li>\
            </rdf:Bag></I:PersonInImage></rdf:Description></rdf:RDF></x:xmpmeta>";
        for (k, h) in photos.iter().enumerate() {
            let taken = [Some("2024:05:17 10:00:00"), Some("2024:05:17 12:00:00"), None, None][k];
            let exif = taken.map(|t| tiff(true, None, None, Some(t)));
            let file = dir.join(format!("p{k}.jpg"));
            std::fs::write(&file, jpeg_file(100, 100, exif.as_deref(), (k == 0).then_some(tagged))).unwrap();
            db.store.photos.insert(h.clone(), json!({"path": file.to_string_lossy(), "mtime": 1_700_000_000 + k}));
        }
        db.changed();
        db.act("name", &[0], Some("Cy"), None).unwrap();
        db.act("name", &[1], Some("Dee"), None).unwrap();
        db.act("name", &[3], Some("Ann"), None).unwrap();
        db.act("name", &[4], Some("Bob"), None).unwrap();
        assert_eq!(db.read_facts(None).unwrap(), 4, "every photo read anew");
        assert_eq!(db.facts.status, photofacts::Status { running: false, done: 4, total: 4, read: 4 });
        assert_eq!(db.read_facts(None).unwrap(), 0, "nothing changed: nothing read again");
        let first = db.fact(&photos[0]);
        assert_eq!(
            (first.t, first.tags.clone(), first.faces.len()),
            (photofiles::camera_time("2024:05:17 10:00:00").unwrap(), vec!["Zed".to_string()], 0)
        );
        // the one name left for the one face still unnamed
        assert_eq!(db.tag_names(&photos[0], &first), [(2, "Zed".to_string())]);
        assert_eq!(db.tagged(false), json!([{"name": "Zed", "faces": [[2, 0.8]], "known": false}]));
        // the same occasion: by the camera's date, else the photos beside it in its folder
        let occasion = db.occasion(&photos[0]);
        assert_eq!(
            occasion,
            json!({"how": "taken within 12 hours of it", "photos": 1,
                                    "people": [{"name": "Ann", "n": 1, "i": 3}, {"name": "Bob", "n": 1, "i": 4}]})
        );
        let beside = db.occasion(&photos[2]);
        assert_eq!((beside["how"].as_str(), beside["photos"].as_u64()), (Some("next to it in its folder"), Some(3)));
        let names: Vec<&str> =
            beside["people"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Cy", "Dee", "Ann", "Bob"]);
        let hints = db.hints(2);
        assert_eq!(
            (&hints["tag"], &hints["tags"], &hints["occasion"], &hints["live"]),
            (&json!("Zed"), &json!(["Zed"]), &occasion, &Value::Null)
        );
        assert!(hints["closest"].as_array().unwrap().len() <= CLOSEST);
        // the photos with both of two people, and a copy of them in a folder
        let rows = db.photos_with(&["Ann", "Bob", "Ann", ""]);
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(
            (&rows[0]["i"], &rows[0]["people"], &rows[0]["exists"], &rows[0]["date"]),
            (&json!(3), &json!(["Ann", "Bob"]), &json!(true), &json!("2024-05-17"))
        );
        assert_eq!(db.photos_with(&["Ann", "Cy"]), json!([]));
        assert_eq!(db.photos_with(&["Cy"])[0]["t"], json!(first.t));
        let dest = dir.join("out");
        assert_eq!(
            LabelDb::export(&rows, &format!(" \"{}\" ", dest.display())).unwrap(),
            json!({"copied": 1, "skipped": 0, "missing": 0})
        );
        assert_eq!(
            LabelDb::export(&rows, &dest.to_string_lossy()).unwrap(),
            json!({"copied": 0, "skipped": 1, "missing": 0})
        );
        std::fs::write(dest.join("p1.jpg"), b"something else").unwrap();
        let gone = json!([rows[0].clone(), {"path": "nowhere.jpg", "exists": false}]);
        assert_eq!(
            LabelDb::export(&gone, &dest.to_string_lossy()).unwrap(),
            json!({"copied": 1, "skipped": 0, "missing": 1})
        );
        assert!(dest.join("p1 (2).jpg").is_file());
        assert_eq!(
            LabelDb::export(&rows, "relative/folder"),
            asked("Give the full path of the folder, e.g. E:\\Pictures\\Export")
        );
        // a snapshot the tracker saved has its verdict beside it
        db.webcam = dir.clone();
        db.changed();
        std::fs::write(dir.join("p3.json"), r#"{"when": "2026-11-20T10:00:00Z", "faces": [{"box": [300, 5, 40, 40], "name": "x", "sim": 0.2}, {"box": [91, 6, 40, 40], "name": "Ann", "sim": 0.61}]}"#).unwrap();
        assert_eq!(db.live_verdict(9), json!({"name": "Ann", "sim": 0.61, "when": "2026-11-20T10:00:00Z"}));
        assert_eq!(db.live_verdict(0), Value::Null, "no verdict beside that one");
        // read again from the files: the same facts
        let (data, home) = (db.data.clone(), db.home.clone());
        let again = LabelDb::open(&data, &home, model(), false, &mut |_| {}).unwrap();
        assert_eq!(again.facts.facts, db.facts.facts);
        let summary = db.summary();
        assert_eq!(
            (&summary["tagged"], &summary["possibleLeft"], &summary["facts"]["read"]),
            (&json!(1), &json!(4), &json!(0))
        );
    }

    #[test]
    fn possible_faces_are_shown_when_they_look_like_real_ones() {
        // person 0 and 1 are real faces; the faces of "person" 2 and 3 are possible faces, most of them junk
        let mut db = library("possible", 40);
        let fids: Vec<String> = db.store.fid.clone();
        for h in db.photo_order() {
            let found: Map<String, Value> = (0..db.n())
                .filter(|i| db.store.photo[*i] == h && i % 4 >= 2)
                .map(|i| (fids[i].clone(), if i == 2 { json!(0) } else { json!(0.6) }))
                .collect();
            db.store.photos[&h]["maybe"] = Value::Object(found);
        }
        db.changed();
        assert!(
            db.faceness_due() && db.face_cut() == 0.0 && db.shown(6),
            "nothing learned yet: every possible face is shown"
        );
        let (pos, neg) = db.face_sets();
        assert_eq!((pos.len(), neg.len(), neg.contains(&2)), (80, 79, false), "a face drawn by hand is neither");
        db.act("name", &[3, 7], Some("Ann"), None).unwrap(); // two possible faces named: real after all
        db.learn_faceness();
        assert!(!db.faceness_due());
        let (pos, neg) = db.face_sets();
        assert_eq!((pos.len(), neg.len()), (82, 77));
        let possible = db.possible();
        let rows = possible.as_array().unwrap();
        // the unnamed possible faces nobody is suggested for (the drawn one too; the rest of Ann's are suggested
        // for her now), the most face-like first
        assert_eq!(rows.len(), 40);
        assert!(rows.windows(2).all(|w| w[0][3].as_f64() >= w[1][3].as_f64()));
        assert_eq!(rows[0], json!([2, 0.8, 0.0, 1.0, true]), "the one drawn by hand");
        let shown = rows.iter().filter(|f| f[4] == true).count();
        // a quarter of all the scored possible faces are over the cut
        let scored: Vec<usize> = (0..db.n()).filter(|i| i % 4 >= 2 && *i != 2).collect();
        let over = scored.iter().filter(|i| db.face_score(**i) >= db.face_cut()).count();
        assert!((19..=21).contains(&over), "{over} of {} over the cut", scored.len());
        assert!(db.shown(3) && db.shown(0), "a named possible face and a regular face are always shown");
        assert_eq!(db.summary()["possible"], json!(shown));
        assert!(
            db.faceness(db.store.print(0)).unwrap() > db.faceness(db.store.print(6)).unwrap(),
            "a real face looks more like one"
        );
        // faces change: learned again
        db.act("ignore", &[10], None, None).unwrap();
        assert!(db.faceness_due());
    }

    #[test]
    fn an_empty_library_is_a_library() {
        let (data, home) = folders("empty");
        let mut db = LabelDb::open(&data, &home, model(), true, &mut |_| {}).unwrap();
        assert!(db.triage().is_empty() && db.gallery().people.is_empty() && db.stacks().is_empty());
        assert_eq!(db.suggested(false), json!([]));
        assert_eq!(db.unknown(false, false), json!({"groups": [], "singles": []}));
        assert_eq!(*db.checks(), json!({"mislabeled": [], "few": [], "duplicates": []}));
        assert_eq!(
            (db.autoname(None).unwrap(), db.undo().unwrap(), db.act("clear", &[], None, None).unwrap()),
            (0, None, 0)
        );
        assert_eq!(db.build_people().unwrap(), []);
        assert_eq!(people_file(&db), (vec![], 0));
        assert_eq!(db.summary()["people"], json!([]));
    }
}
