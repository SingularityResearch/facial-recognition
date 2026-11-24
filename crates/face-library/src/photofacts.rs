//! What a photo says about itself: when the camera took it, and who it
//! is tagged with: photo apps' face tags inside the file or in an .xmp file beside it (MWG regions: Lightroom,
//! Picasa, digiKam, and the unnamed ones iPhones write; Microsoft's: Windows Photo Gallery; plain names:
//! IPTC's PersonInImage), or a Google Takeout .json. Kept in `data/photo_facts.json`; a photo is read again
//! only when it, the files beside it, or the faces found in it change. Nothing here is ever a label by itself.
//!
//! Tag rectangles are taken as applying to the photo as stored (before EXIF's rotation), as MWG says: on
//! rotated iPhone photos that paired 71 of 76 tag boxes with found faces, against 32 the other way.

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use face_files::{filejson, paths::atomic_write};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::{
    exif,
    photofiles::{camera_time, seconds, sha1_hex},
    store::Store,
};

/// photos taken within this many hours of each other count as one occasion
pub const TAKEN_NEAR_H: f64 = 12.0;
/// a photo without a camera date: this many photos with faces either side in its folder
pub const FOLDER_NEAR: usize = 6;
/// closest people listed for a face (a hint, never a label)
pub const CLOSEST: usize = 3;
/// photos are shrunk to this before faces are looked for: what the faces' boxes are in
pub const MAX_SIDE: f64 = 1280.0;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const MWG: &str = "http://www.metadataworkinggroup.com/schemas/regions/";
const AREA: &str = "http://ns.adobe.com/xmp/sType/Area#";
const DIM: &str = "http://ns.adobe.com/xap/1.0/sType/Dimensions#";
const MPRI: &str = "http://ns.microsoft.com/photo/1.2/t/RegionInfo#";
const MPREG: &str = "http://ns.microsoft.com/photo/1.2/t/Region#";
const IPTC: &str = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";

/// A text without the blanks at its ends; the four separator characters below a space count as blanks too.
pub fn strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// One people tag of a photo.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: Option<String>,
    /// normalized, from the top left: x, y, width, height
    pub rect: Option<[f64; 4]>,
    /// "mwg", "mp", "iptc" or "takeout"
    pub kind: &'static str,
    /// the picture size the rectangle was drawn on, where the tag says
    pub applied: Option<(Option<f64>, Option<f64>)>,
    /// the rectangle in the pixels the faces' boxes are in
    pub boxed: Option<[f64; 4]>,
}

type Node<'a, 'b> = roxmltree::Node<'a, 'b>;

fn named<'a, 'b: 'a>(el: Node<'a, 'b>, ns: &'static str, name: &'static str) -> impl Iterator<Item = Node<'a, 'b>> {
    el.descendants().filter(move |e| e.is_element() && e.has_tag_name((ns, name)))
}

/// An XMP property written either as an attribute (here or on something inside) or as an element.
fn val(el: Node, ns: &str, name: &str) -> Option<String> {
    for e in el.descendants().filter(Node::is_element) {
        let v = if e.has_tag_name((ns, name)) { e.text() } else { e.attribute((ns, name)) };
        if let Some(v) = v.map(strip).filter(|v| !v.is_empty()) {
            return Some(v.to_string());
        }
    }
    None
}

fn num(v: Option<String>) -> Option<f64> {
    strip(v.as_deref()?).parse().ok()
}

/// The people tags in an XMP packet. Rectangles are normalized, from the top left; `kind` says whose they
/// are, which says whether they apply to the photo as stored or as shown (turned).
pub fn xmp_people(xml: &[u8]) -> Vec<Tag> {
    let Ok(text) = std::str::from_utf8(xml) else { return Vec::new() };
    let Ok(doc) = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    ) else {
        return Vec::new();
    };
    let root = doc.root_element();
    let mut out = Vec::new();
    for regions in named(root, MWG, "Regions") {
        let applied =
            named(regions, MWG, "AppliedToDimensions").next().map(|d| (num(val(d, DIM, "w")), num(val(d, DIM, "h"))));
        for li in named(regions, RDF, "li") {
            let Some(area) = named(li, MWG, "Area").next() else { continue };
            let face = val(li, MWG, "Type").is_none_or(|t| t == "Face");
            let normalized = val(area, AREA, "unit").is_none_or(|u| u == "normalized");
            let at = |k: &str| num(val(area, AREA, k));
            if let (true, true, Some(x), Some(y), Some(w), Some(h)) =
                (face, normalized, at("x"), at("y"), at("w"), at("h"))
            {
                // MWG gives the middle of the rectangle
                out.push(Tag {
                    name: val(li, MWG, "Name"),
                    rect: Some([x - w / 2.0, y - h / 2.0, w, h]),
                    kind: "mwg",
                    applied,
                    boxed: None,
                });
            }
        }
    }
    for regions in named(root, MPRI, "Regions") {
        for li in named(regions, RDF, "li") {
            let rect: Vec<Option<f64>> =
                val(li, MPREG, "Rectangle").unwrap_or_default().split(',').map(|v| num(Some(v.to_string()))).collect();
            let rect = match rect.as_slice() {
                [Some(x), Some(y), Some(w), Some(h)] => Some([*x, *y, *w, *h]),
                _ => None,
            };
            out.push(Tag { name: val(li, MPREG, "PersonDisplayName"), rect, kind: "mp", applied: None, boxed: None });
        }
    }
    for person in named(root, IPTC, "PersonInImage") {
        for li in named(person, RDF, "li") {
            if let Some(name) = li.text().map(strip).filter(|n| !n.is_empty()) {
                out.push(Tag { name: Some(name.to_string()), rect: None, kind: "iptc", applied: None, boxed: None });
            }
        }
    }
    out
}

/// The XMP packets in a run of bytes (the start of a photo file, or an .xmp file).
pub fn xmp_packets(data: &[u8]) -> Vec<&[u8]> {
    let find = |what: &[u8], from: usize| data[from..].windows(what.len()).position(|w| w == what).map(|k| from + k);
    let (mut out, mut k) = (Vec::new(), 0);
    while let Some(start) = find(b"<x:xmpmeta", k) {
        let Some(end) = find(b"</x:xmpmeta>", start) else { break };
        out.push(&data[start..end + 12]);
        k = end;
    }
    out
}

/// A normalized rectangle on the photo as stored, on the photo as shown (EXIF orientation `o`).
pub fn orient(r: [f64; 4], o: u32) -> [f64; 4] {
    let [x, y, w, h] = r;
    match o {
        2 => [1.0 - x - w, y, w, h],
        3 => [1.0 - x - w, 1.0 - y - h, w, h],
        4 => [x, 1.0 - y - h, w, h],
        5 => [y, x, h, w],
        6 => [1.0 - y - h, x, h, w],
        7 => [1.0 - y - h, 1.0 - x - w, h, w],
        8 => [y, 1.0 - x - w, h, w],
        _ => [x, y, w, h],
    }
}

/// Overlap of two boxes (x, y, w, h): what they share over what they cover together.
pub fn iou(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    let (x1, y1) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x2, y2) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let shared = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    shared / (a[2] * a[3] + b[2] * b[3] - shared + 1e-9)
}

/// Pair tagged face regions (boxes in pixels) with found faces: the best overlap first, at least 0.2. Tag
/// regions are drawn looser than the detector's boxes, so a face well inside a region also counts.
/// [(face, region)] in the order they were paired.
pub fn match_regions(regions: &[[f64; 4]], boxes: &[[f64; 4]]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for (a, r) in regions.iter().enumerate() {
        for (b, f) in boxes.iter().enumerate() {
            let mut s = iou(r, f);
            let (cx, cy) = (f[0] + f[2] / 2.0, f[1] + f[3] / 2.0);
            let inside = r[0] <= cx && cx <= r[0] + r[2] && r[1] <= cy && cy <= r[1] + r[3];
            if s < 0.2 && inside && f[2] * f[3] >= 0.15 * r[2] * r[3] {
                s = 0.2;
            }
            if s >= 0.2 {
                pairs.push((s, a, b));
            }
        }
    }
    pairs.sort_by(|x, y| y.0.total_cmp(&x.0).then(y.1.cmp(&x.1)).then(y.2.cmp(&x.2)));
    let (mut used_a, mut used_b, mut out) = (BTreeSet::new(), BTreeSet::new(), Vec::new());
    for (_, a, b) in pairs {
        if !used_a.contains(&a) && !used_b.contains(&b) {
            used_a.insert(a);
            used_b.insert(b);
            out.push((b, a));
        }
    }
    out
}

/// A path's folder and file name.
pub fn split(path: &str) -> (&str, &str) {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    match path.rfind(seps) {
        Some(k) => (&path[..k], &path[k + 1..]),
        None => ("", path),
    }
}

/// A file name's ending (".jpg"; nothing for a name that only begins or
/// ends with a dot), and the name without it.
pub fn suffix(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(k) if k > 0 && k < name.len() - 1 => (&name[..k], &name[k..]),
        _ => (name, ""),
    }
}

/// Text as the facts file's keys spell it: in single quotes, or in double quotes where it holds a single
/// one and no double one, with the unprintable characters escaped. (The usual ones are known here, but not
/// every unassigned one: a file name with such a character can get another key, and its photo is then read once more.)
fn key_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let hidden = |c: char| {
        c.is_control()
            || (c.is_whitespace() && c != ' ')
            || matches!(c as u32, 0xad | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e
                | 0x2060..=0x2064 | 0x2066..=0x206f | 0xfeff | 0xfff9..=0xfffb | 0xe000..=0xf8ff | 0xe0001 | 0xe0020..=0xe007f | 0xf0000..)
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if hidden(c) => match c as u32 {
                v @ ..=0xff => out.push_str(&format!("\\x{v:02x}")),
                v @ ..=0xffff => out.push_str(&format!("\\u{v:04x}")),
                v => out.push_str(&format!("\\U{v:08x}")),
            },
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// What one photo says about itself.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Fact {
    /// when the camera took it, in seconds since 1970
    pub t: Option<f64>,
    /// the file has that time as a whole number (as some came from another device): written back as one
    pub t_whole: bool,
    /// the names it is tagged with
    pub tags: Vec<String>,
    /// the found faces a tag's rectangle sits on: (face, name), in the order they were paired
    pub faces: Vec<(usize, String)>,
    /// what these facts depend on (the file, the files beside it, where its faces are); none for facts
    /// that were only read for a question and aren't kept
    pub key: Option<String>,
}

impl Fact {
    /// The camera time as it is written: a number as the file had it, or nothing.
    pub fn t_value(&self) -> Value {
        match self.t {
            Some(t) if self.t_whole => json!(t as i64),
            Some(t) => json!(t),
            None => Value::Null,
        }
    }
}

/// What a photo's facts are read from: taken from the library under its lock, read without it.
#[derive(Clone, Debug)]
pub struct Job {
    pub h: String,
    pub path: String,
    /// the photo's faces and their boxes (x, y, width, height)
    pub faces: Vec<usize>,
    pub boxes: Vec<[f32; 4]>,
    /// the facts kept so far
    pub known: Option<Fact>,
}

/// The files of one folder with one ending: by lower-case name, in the folder's order and in name order.
struct Listing {
    paths: HashMap<String, PathBuf>,
    order: Vec<String>,
    sorted: Vec<String>,
}

/// The folders looked into so far. A folder is listed once: going through it for every photo took a minute
/// at every start once one folder held 10,000 photos with a file beside each.
#[derive(Default)]
pub struct Listings {
    dirs: Mutex<HashMap<(String, &'static str), Arc<Listing>>>,
}

impl Listings {
    fn listing(&self, folder: &str, ext: &'static str) -> Arc<Listing> {
        let key = (folder.to_string(), ext);
        if let Some(l) = self.dirs.lock().ok().and_then(|d| d.get(&key).cloned()) {
            return l;
        }
        let (mut paths, mut order) = (HashMap::new(), Vec::new());
        for entry in std::fs::read_dir(if folder.is_empty() { "." } else { folder }).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if suffix(&name).1 == ext {
                if paths.insert(name.clone(), entry.path()).is_none() {
                    order.push(name);
                }
            }
        }
        let mut sorted = order.clone();
        sorted.sort();
        let listing = Arc::new(Listing { paths, order, sorted });
        if let Ok(mut d) = self.dirs.lock() {
            d.insert(key, listing.clone());
        }
        listing
    }

    /// The files of `folder` ending in `ext` whose names begin with one of `starts`, in name order.
    fn beside(&self, folder: &str, ext: &'static str, starts: &[&str]) -> Vec<(String, PathBuf)> {
        let listing = self.listing(folder, ext);
        let mut found = BTreeSet::new();
        for start in starts {
            let from = listing.sorted.partition_point(|n| n.as_str() < *start);
            found.extend(listing.sorted[from..].iter().take_while(|n| n.starts_with(start)).cloned());
        }
        found.into_iter().map(|name| (name.clone(), listing.paths[&name].clone())).collect()
    }

    /// What a photo's facts depend on: the file, the files beside it, and where its faces are.
    pub fn key(&self, job: &Job) -> std::io::Result<String> {
        let st = std::fs::metadata(&job.path)?;
        let (folder, name) = split(&job.path);
        let lower = name.to_lowercase();
        let first: String = lower.chars().take(40).collect();
        let mut side = Vec::new();
        for ext in [".xmp", ".json"] {
            for (k, path) in self.beside(folder, ext, &[suffix(&lower).0, &first]) {
                side.push(format!(
                    "({}, {})",
                    key_repr(&k),
                    filejson::float_repr(seconds(std::fs::metadata(path)?.modified()?))
                ));
            }
        }
        let geo: Vec<String> = (job.boxes.iter())
            .map(|b| {
                format!(
                    "[{}]",
                    b.iter().map(|v| ((*v as f64).round_ties_even() as i64).to_string()).collect::<Vec<_>>().join(", ")
                )
            })
            .collect();
        let mtime = seconds(st.modified()?).trunc() as i64;
        let said = format!("({}, {mtime}, [{}], [{}])", st.len(), side.join(", "), geo.join(", "));
        Ok(sha1_hex(said.as_bytes())[..16].to_string())
    }

    /// People named in a Google Takeout file beside the photo (photo.jpg.json,
    /// photo.jpg.supplemental-metadata.json; long file names are cut short in the export, so other .json
    /// files starting the same way are looked at).
    fn takeout(&self, folder: &str, name: &str) -> Vec<Tag> {
        let js = self.listing(folder, ".json");
        let lower = name.to_lowercase();
        let exact: Vec<&String> = [format!("{lower}.json"), format!("{lower}.supplemental-metadata.json")]
            .iter()
            .filter_map(|k| js.paths.get_key_value(k).map(|(k, _)| k))
            .collect();
        let first: String = lower.chars().take(40).collect();
        let others = js.order.iter().filter(|k| k.starts_with(&first) && !exact.contains(k)).take(6);
        for (k, sure) in exact.iter().map(|k| (*k, true)).chain(others.map(|k| (k, false))) {
            let Some(Value::Object(j)) =
                std::fs::read(&js.paths[k]).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            else {
                continue;
            };
            if !(sure || j.get("title").and_then(Value::as_str).is_some_and(|t| t.to_lowercase() == lower)) {
                continue;
            }
            if let Some(people) = j.get("people").and_then(Value::as_array) {
                return (people.iter().filter_map(|x| x.as_object()?.get("name")))
                    .map(|n| n.as_str().map_or_else(|| n.to_string(), str::to_string))
                    .filter(|n| !strip(n).is_empty())
                    .map(|n| Tag {
                        name: Some(strip(&n).to_string()),
                        rect: None,
                        kind: "takeout",
                        applied: None,
                        boxed: None,
                    })
                    .collect();
            }
        }
        Vec::new()
    }

    /// (when the camera took the photo, its people tags with `boxed` in the faces' pixels where a tag has a
    /// rectangle).
    pub fn tags(&self, path: &str) -> Result<(Option<f64>, Vec<Tag>), String> {
        let head = exif::header_of(Path::new(path))?;
        let o = head.orientation;
        let t = match &head.taken {
            Some(d) => camera_time(d)?,
            None => None,
        };
        let (folder, name) = split(path);
        let start = {
            use std::io::Read;
            let mut buf = Vec::new();
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            file.take(1 << 20).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            buf
        };
        let mut tags: Vec<Tag> = xmp_packets(&start).into_iter().flat_map(xmp_people).collect();
        let side = self.listing(folder, ".xmp");
        let lower = name.to_lowercase();
        for k in [format!("{}.xmp", suffix(&lower).0), format!("{lower}.xmp")] {
            if let Some(file) = side.paths.get(&k) {
                let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
                tags.extend(xmp_packets(&bytes).into_iter().flat_map(xmp_people));
            }
        }
        tags.extend(self.takeout(folder, name));

        // tag rectangles -> boxes on the photo the faces were found in (upright, shrunk to MAX_SIDE)
        let (w, h) = (head.width as f64, head.height as f64);
        if w.max(h) == 0.0 {
            return Err("a picture without a size".to_string());
        }
        let turned = (5..=8).contains(&o);
        let (wd, hd) = if turned { (h, w) } else { (w, h) };
        let s = (MAX_SIDE / w.max(h)).min(1.0);
        let (sw, sh) = ((wd * s).round_ties_even(), (hd * s).round_ties_even());
        for r in &mut tags {
            let Some(rect) = r.rect else { continue };
            let whole = |v: Option<f64>| v.unwrap_or(0.0).round_ties_even();
            let shown = r.kind == "mp" || r.applied.is_some_and(|a| turned && (whole(a.0), whole(a.1)) == (h, w));
            let [x, y, rw, rh] = if shown { rect } else { orient(rect, o) };
            r.boxed = Some([x * sw, y * sh, rw * sw, rh * sh]);
        }
        Ok((t, tags))
    }

    /// The photo read: its camera time, the names it is tagged with, and the found faces its tags sit on.
    pub fn read(&self, job: &Job) -> Result<Fact, String> {
        let (t, tags) = self.tags(&job.path)?;
        let named: Vec<&Tag> = tags.iter().filter(|r| r.name.as_deref().is_some_and(|n| !n.is_empty())).collect();
        let regions: Vec<&&Tag> = named.iter().filter(|r| r.boxed.is_some()).collect();
        let boxes: Vec<[f64; 4]> = job.boxes.iter().map(|b| b.map(|v| v as f64)).collect();
        let pairs = match_regions(&regions.iter().filter_map(|r| r.boxed).collect::<Vec<_>>(), &boxes);
        let names: BTreeSet<String> = named.iter().filter_map(|r| r.name.clone()).collect();
        Ok(Fact {
            t,
            t_whole: false,
            tags: names.into_iter().collect(),
            faces: pairs
                .into_iter()
                .map(|(b, a)| (job.faces[b], regions[a].name.clone().unwrap_or_default()))
                .collect(),
            key: None,
        })
    }

    /// An unreadable photo just has no facts.
    pub fn read_safe(&self, job: &Job) -> Fact {
        self.read(job).unwrap_or_default()
    }

    /// The kept facts if nothing they depend on changed, else the photo read again: (the facts, whether
    /// they were read now).
    pub fn fresh(&self, job: &Job) -> (Fact, bool) {
        let Ok(key) = self.key(job) else {
            return (job.known.clone().unwrap_or_else(|| self.read_safe(job)), false);
        };
        match &job.known {
            Some(known) if known.key.as_deref() == Some(key.as_str()) => (known.clone(), false),
            _ => (Fact { key: Some(key), ..self.read_safe(job) }, true),
        }
    }

    /// Every job's facts, eight photos at a time: reading is mostly waiting for the files.
    pub fn run(&self, jobs: &[Job]) -> Vec<(Fact, bool)> {
        let mut out: Vec<Option<(Fact, bool)>> = vec![None; jobs.len()];
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done: Mutex<&mut Vec<Option<(Fact, bool)>>> = Mutex::new(&mut out);
        std::thread::scope(|s| {
            for _ in 0..8.min(jobs.len()) {
                s.spawn(|| {
                    while let Some((k, job)) = Some(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                        .and_then(|k| Some((k, jobs.get(k)?)))
                    {
                        let fact = self.fresh(job);
                        if let Ok(mut d) = done.lock() {
                            d[k] = Some(fact);
                        }
                    }
                });
            }
        });
        out.into_iter().map(Option::unwrap_or_default).collect()
    }
}

/// How far the reading of the photos' facts has got.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub read: usize,
}

/// What each photo says about itself.
pub struct PhotoFacts {
    /// by the photo's content hash, in the file's order
    pub facts: IndexMap<String, Fact>,
    pub status: Status,
    /// rises when the photos were read
    pub version: u64,
    pub listings: Arc<Listings>,
    data: PathBuf,
}

impl PhotoFacts {
    pub const FILE: &'static str = "photo_facts.json";

    /// The facts kept in `data`, for the faces of `store`.
    pub fn load(data: &Path, store: &Store) -> Result<PhotoFacts, String> {
        let mut facts = IndexMap::new();
        let kept = match std::fs::read(data.join(Self::FILE)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(format!("can't read {}: {e}", Self::FILE)),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(Value::Object(m)) => m,
                Ok(_) => return Err(format!("{} is not a list of photos", Self::FILE)),
                Err(e) => return Err(format!("can't read {}: {e}", Self::FILE)),
            },
        };
        for (h, f) in kept {
            let faces = (f.get("faces").and_then(Value::as_object).into_iter().flatten())
                .filter_map(|(fid, name)| Some((store.index_of(fid)?, name.as_str()?.to_string())))
                .collect();
            let tags = f
                .get("tags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect();
            let key = f.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()).map(str::to_string);
            let t = f.get("t").filter(|t| t.is_number());
            facts.insert(
                h,
                Fact { t: t.and_then(Value::as_f64), t_whole: t.is_some_and(|t| !t.is_f64()), tags, faces, key },
            );
        }
        Ok(PhotoFacts {
            facts,
            status: Status::default(),
            version: 0,
            listings: Arc::default(),
            data: data.to_path_buf(),
        })
    }

    /// Write the kept facts (those with a key), the faces by their permanent IDs.
    pub fn save(&self, store: &Store) -> std::io::Result<()> {
        let mut out = Map::new();
        for (h, f) in self.facts.iter().filter(|(_, f)| f.key.is_some()) {
            let faces: Map<String, Value> =
                f.faces.iter().map(|(i, name)| (store.fid[*i].clone(), json!(name))).collect();
            out.insert(h.clone(), json!({"t": f.t_value(), "tags": f.tags, "faces": faces, "key": f.key}));
        }
        atomic_write(&self.data.join(Self::FILE), filejson::rows_of(&out, true).as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif::tests::{jpeg_file, tiff};

    const MWG_PACKET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
   xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#" xmlns:stDim="http://ns.adobe.com/xap/1.0/sType/Dimensions#"
   xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/">
  <mwg-rs:Regions rdf:parseType="Resource">
   <mwg-rs:AppliedToDimensions stDim:w="4000" stDim:h="3000" stDim:unit="pixel"/>
   <mwg-rs:RegionList><rdf:Bag>
    <rdf:li><rdf:Description mwg-rs:Name=" Ann " mwg-rs:Type="Face">
      <mwg-rs:Area stArea:x="0.25" stArea:y="0.5" stArea:w="0.1" stArea:h="0.2" stArea:unit="normalized"/></rdf:Description></rdf:li>
    <rdf:li rdf:parseType="Resource"><mwg-rs:Type>Face</mwg-rs:Type>
      <mwg-rs:Area rdf:parseType="Resource"><stArea:x>0.75</stArea:x><stArea:y>0.5</stArea:y><stArea:w>0.1</stArea:w><stArea:h>0.2</stArea:h></mwg-rs:Area></rdf:li>
    <rdf:li><rdf:Description mwg-rs:Name="A barcode" mwg-rs:Type="BarCode">
      <mwg-rs:Area stArea:x="0.5" stArea:y="0.5" stArea:w="0.1" stArea:h="0.1"/></rdf:Description></rdf:li>
    <rdf:li><rdf:Description mwg-rs:Name="In pixels" mwg-rs:Type="Face">
      <mwg-rs:Area stArea:x="10" stArea:y="10" stArea:w="5" stArea:h="5" stArea:unit="pixel"/></rdf:Description></rdf:li>
   </rdf:Bag></mwg-rs:RegionList>
  </mwg-rs:Regions>
  <Iptc4xmpExt:PersonInImage><rdf:Bag><rdf:li>Bob</rdf:li><rdf:li>  </rdf:li><rdf:li>Cy</rdf:li></rdf:Bag></Iptc4xmpExt:PersonInImage>
 </rdf:Description></rdf:RDF></x:xmpmeta>"#;

    const MP_PACKET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description xmlns:MP="http://ns.microsoft.com/photo/1.2/" xmlns:MPRI="http://ns.microsoft.com/photo/1.2/t/RegionInfo#"
   xmlns:MPReg="http://ns.microsoft.com/photo/1.2/t/Region#">
  <MP:RegionInfo rdf:parseType="Resource"><MPRI:Regions><rdf:Bag>
   <rdf:li rdf:parseType="Resource"><MPReg:Rectangle>0.1, 0.2, 0.3, 0.4</MPReg:Rectangle><MPReg:PersonDisplayName>Dee</MPReg:PersonDisplayName></rdf:li>
   <rdf:li rdf:parseType="Resource"><MPReg:PersonDisplayName>Eve</MPReg:PersonDisplayName></rdf:li>
  </rdf:Bag></MPRI:Regions></MP:RegionInfo>
 </rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn people_tags_are_read_from_each_apps_way_of_writing_them() {
        let tags = xmp_people(MWG_PACKET.as_bytes());
        let size = Some((Some(4000.0), Some(3000.0)));
        assert_eq!(tags.len(), 4, "{tags:?}");
        assert_eq!(
            tags[0],
            Tag { name: Some("Ann".into()), rect: Some([0.2, 0.4, 0.1, 0.2]), kind: "mwg", applied: size, boxed: None }
        );
        assert_eq!(
            (tags[1].name.clone(), tags[1].rect.map(|r| (r[0] * 100.0).round())),
            (None, Some(70.0)),
            "an iPhone's unnamed face"
        );
        assert_eq!((tags[2].name.as_deref(), tags[2].kind, tags[3].name.as_deref()), (Some("Bob"), "iptc", Some("Cy")));
        let tags = xmp_people(MP_PACKET.as_bytes());
        assert_eq!(
            tags[0],
            Tag { name: Some("Dee".into()), rect: Some([0.1, 0.2, 0.3, 0.4]), kind: "mp", applied: None, boxed: None }
        );
        assert_eq!((tags[1].name.as_deref(), tags[1].rect), (Some("Eve"), None));
        assert!(xmp_people(b"<x:xmpmeta>not closed").is_empty() && xmp_people(&[0xff, 0xfe]).is_empty());
        // packets are found in whatever is around them
        let data =
            [b"\xff\xd8junk".as_slice(), MWG_PACKET.as_bytes(), b"more\0junk", MP_PACKET.as_bytes(), b"<x:xmpmeta cut"]
                .concat();
        let found = xmp_packets(&data);
        assert_eq!((found.len(), found[0], found[1]), (2, MWG_PACKET.as_bytes(), MP_PACKET.as_bytes()));
    }

    #[test]
    fn a_rectangle_on_the_stored_photo_is_placed_on_the_shown_one() {
        let r = [0.1, 0.2, 0.3, 0.4];
        let near = |a: [f64; 4], b: [f64; 4]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(near(orient(r, 1), r) && near(orient(r, 0), r) && near(orient(r, 9), r));
        assert!(
            near(orient(r, 2), [0.6, 0.2, 0.3, 0.4])
                && near(orient(r, 3), [0.6, 0.4, 0.3, 0.4])
                && near(orient(r, 4), [0.1, 0.4, 0.3, 0.4])
        );
        assert!(near(orient(r, 5), [0.2, 0.1, 0.4, 0.3]) && near(orient(r, 6), [0.4, 0.1, 0.4, 0.3]));
        assert!(near(orient(r, 7), [0.4, 0.6, 0.4, 0.3]) && near(orient(r, 8), [0.2, 0.6, 0.4, 0.3]));
    }

    #[test]
    fn regions_are_paired_with_faces_the_best_overlap_first() {
        let regions = [[100.0, 100.0, 100.0, 100.0], [300.0, 100.0, 200.0, 200.0], [600.0, 600.0, 50.0, 50.0]];
        // a face right on the first region, one well inside the looser second, one that touches the first
        // less, and one far from all
        let faces = [
            [340.0, 140.0, 90.0, 90.0],
            [105.0, 105.0, 95.0, 95.0],
            [150.0, 150.0, 100.0, 100.0],
            [900.0, 900.0, 40.0, 40.0],
        ];
        assert_eq!(match_regions(&regions, &faces), [(1, 0), (0, 1)]);
        assert!(match_regions(&[], &faces).is_empty() && match_regions(&regions, &[]).is_empty());
        assert!((iou(&regions[0], &regions[0]) - 1.0).abs() < 1e-9 && iou(&regions[0], &regions[2]) == 0.0);
    }

    #[test]
    fn text_is_written_as_the_facts_files_keys_spell_it() {
        assert_eq!(key_repr("img_0001.xmp"), "'img_0001.xmp'");
        assert_eq!(key_repr("it's.json"), "\"it's.json\"");
        assert_eq!(key_repr("it's \"x\".json"), "'it\\'s \"x\".json'");
        assert_eq!(key_repr("a\\b\tc\u{e9}\u{7f}\u{a0}\u{200b}"), "'a\\\\b\\tc\u{e9}\\x7f\\xa0\\u200b'");
        assert_eq!(
            (suffix("a.b.JPG"), suffix(".hidden"), suffix("name."), suffix("plain")),
            (("a.b", ".JPG"), (".hidden", ""), ("name.", ""), ("plain", ""))
        );
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("face-facts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_photo_is_read_for_its_time_its_tags_and_the_faces_they_sit_on() {
        let dir = folder("read");
        // stored on its side (orientation 6): 4000 x 3000 stored, shown 3000 x 4000, found faces in 960 x 1280
        let photo = dir.join("IMG_1.JPG");
        std::fs::write(
            &photo,
            jpeg_file(4000, 3000, Some(&tiff(true, Some(6), None, Some("2024:05:17 14:03:22"))), Some(MWG_PACKET)),
        )
        .unwrap();
        std::fs::write(dir.join("img_1.xmp"), MP_PACKET).unwrap();
        std::fs::write(
            dir.join("IMG_1.JPG.json"),
            r#"{"title": "IMG_1.JPG", "people": [{"name": " Fay "}, {"name": ""}, {"nope": 1}]}"#,
        )
        .unwrap();
        let listings = Listings::default();
        let path = photo.to_string_lossy().to_string();
        let (t, tags) = listings.tags(&path).unwrap();
        assert_eq!(t, camera_time("2024:05:17 14:03:22").unwrap());
        let names: Vec<Option<&str>> = tags.iter().map(|r| r.name.as_deref()).collect();
        assert_eq!(names, [Some("Ann"), None, Some("Bob"), Some("Cy"), Some("Dee"), Some("Eve"), Some("Fay")]);
        // Ann's rectangle, stored: x 0.2, y 0.4, w 0.1, h 0.2 -> shown (turned): x 1 - 0.4 - 0.2, y 0.2, w 0.2, h 0.1
        let boxed = tags[0].boxed.unwrap();
        assert!(
            [(boxed[0], 384.0), (boxed[1], 256.0), (boxed[2], 192.0), (boxed[3], 128.0)]
                .iter()
                .all(|(a, b)| (a - b).abs() < 1e-6),
            "{boxed:?}"
        );
        // Microsoft's rectangles are on the photo as shown
        assert_eq!(tags[4].boxed.unwrap().map(|v| v.round()), [96.0, 256.0, 288.0, 512.0]);
        // two found faces: one on Ann's rectangle, one elsewhere
        let job = Job {
            h: "h".into(),
            path: path.clone(),
            faces: vec![7, 9],
            boxes: vec![[800.0, 100.0, 60.0, 60.0], [400.0, 270.0, 150.0, 100.0]],
            known: None,
        };
        let (fact, new) = listings.fresh(&job);
        assert!(new && fact.key.as_ref().is_some_and(|k| k.len() == 16));
        assert_eq!(
            (fact.t, fact.tags.join(" "), &fact.faces),
            (t, "Ann Bob Cy Dee Eve Fay".to_string(), &vec![(9, "Ann".to_string())])
        );
        // nothing changed: the kept facts stand; a face moved: read again
        let kept = Job { known: Some(fact.clone()), ..job.clone() };
        assert_eq!(listings.fresh(&kept), (fact.clone(), false));
        let moved = Job { boxes: vec![[801.0, 100.0, 60.0, 60.0], [400.0, 270.0, 150.0, 100.0]], ..kept.clone() };
        let (again, new) = listings.fresh(&moved);
        assert!(new && again.key != fact.key && again.faces == fact.faces);
        // all of them at once, and a file that isn't a picture, and one that isn't there
        std::fs::write(dir.join("notes.txt"), b"hello").unwrap();
        let other = |name: &str| Job {
            h: name.into(),
            path: dir.join(name).to_string_lossy().to_string(),
            faces: vec![],
            boxes: vec![],
            known: None,
        };
        let all = listings.run(&[kept, other("notes.txt"), other("gone.jpg"), moved]);
        assert_eq!(
            (all.len(), all[0].1, &all[1].0.tags, all[1].1, all[1].0.key.is_some()),
            (4, false, &vec![], true, true)
        );
        assert_eq!((all[2].0.clone(), all[2].1, all[3].1), (Fact::default(), false, true));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
