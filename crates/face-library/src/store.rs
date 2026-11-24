//! The store: every face a scan found, in the files every released
//! version wrote.
//!
//! - `data/faces.npz`: each face's print, permanent ID, source photo, box and quality, and the recognition
//!   model the prints were made with;
//! - `data/thumbs.npz`: each face's picture (the 112x112 aligned crop as a JPEG, all packed into one run of
//!   bytes) with the face IDs they belong to. Kept apart because pictures stay on this computer; where the
//!   file is missing or lacks a face, that face has no picture here;
//! - `data/photos.json`: each photo read, by its content hash.
//!
//! Every version opens what any other wrote, value for value;
//! `photos.json` is the same bytes from each.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use face_core::{
    image::{Bgr, round_even},
    npy::{self, Array},
};
use face_files::{
    filejson,
    paths::{atomic_write, replace_file},
};
use serde_json::{Map, Value};

pub type Result<T> = std::result::Result<T, String>;

/// the picture kept of a face: this JPEG quality, fixed
const THUMB_QUALITY: u8 = 88;
/// what a database from before model tracking was made with
const FIRST_MODEL: &str = "sface_2021dec";

pub struct Store {
    data: PathBuf,
    /// numbers in a print
    pub dim: usize,
    /// every face's print, one after another (`dim` numbers each; all zeros for a face that was forgotten).
    /// Read through `prints()`, changed only through the functions here, which note each change in `print_log`
    emb: Vec<f32>,
    /// every face whose print changed since the library was read, in the order it happened: what is kept
    /// about the prints between questions (scores, stacks) reads on from where it had got to. A new face
    /// needs no entry
    print_log: Vec<usize>,
    /// the file each face was found in, as it was called then
    pub file: Vec<String>,
    /// each face's permanent ID (none of them in a database older than the IDs)
    pub fid: Vec<String>,
    /// each face's photo, by content hash
    pub photo: Vec<String>,
    /// x, y, width, height in the photo's pixels, and the detector's score
    pub boxes: Vec<[f32; 5]>,
    pub quality: Vec<f32>,
    /// the recognition model the prints were made with
    pub model: String,
    /// each face's picture as a JPEG, empty where there is none on this computer
    pub thumbs: Vec<Vec<u8>>,
    /// {content hash: {path, size, mtime, fp, faces, dup_of, ...}}, in the file's order
    pub photos: Map<String, Value>,
    /// thumbs.npz needs writing at the next save
    pub thumbs_changed: bool,
    /// where each face ID is
    fids: HashMap<String, usize>,
    /// the faces without a print (forgotten ones), kept in step by everything here that changes a print
    gone: Vec<bool>,
}

/// A face's print and picture as `forget` took them, for an undo in the same session.
pub struct Forgotten {
    emb: Vec<f32>,
    thumb: Vec<u8>,
}

/// Permanent ID of a face: its photo's content fingerprint and where the face's centre is (and the frame
/// number, for a face from a video).
pub fn face_key(photo: &str, row: &[f32], frame: Option<i64>) -> String {
    let short: String = photo.chars().take(16).collect();
    let centre = |at: f32, size: f32| round_even(at as f64 + size as f64 / 2.0);
    let key = format!("{short}-{}-{}", centre(row[0], row[2]), centre(row[1], row[3]));
    match frame {
        Some(f) => format!("{key}-f{f}"),
        None => key,
    }
}

fn read(path: &Path) -> Result<HashMap<String, Array>> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let bytes = std::fs::read(path).map_err(|e| format!("can't read {name}: {e}"))?;
    npy::read_npz(&bytes).map_err(|e| format!("can't read {name}: {}", e.0))
}

/// The pictures of an archive that packs them: the bytes between each pair of offsets.
fn cut(z: &HashMap<String, Array>) -> Option<Vec<Vec<u8>>> {
    let (jpg, off) = (z.get("thumbs")?.bytes_of().ok()?, z.get("thumb_off")?.wholes_of().ok()?);
    off.windows(2)
        .map(|w| jpg.get(usize::try_from(w[0]).ok()?..usize::try_from(w[1]).ok()?).map(<[u8]>::to_vec))
        .collect()
}

impl Store {
    /// The library in `data` (made if it isn't there). `model` and `dim` are the recognition model in use:
    /// what an empty library's prints will be made with.
    pub fn load(data: &Path, model: &str, dim: usize) -> Result<Store> {
        std::fs::create_dir_all(data).map_err(|e| format!("can't make {}: {e}", data.display()))?;
        let mut s = Store {
            data: data.to_path_buf(),
            dim,
            emb: Vec::new(),
            print_log: Vec::new(),
            file: Vec::new(),
            fid: Vec::new(),
            photo: Vec::new(),
            boxes: Vec::new(),
            quality: Vec::new(),
            model: model.to_string(),
            thumbs: Vec::new(),
            photos: Map::new(),
            thumbs_changed: false,
            fids: HashMap::new(),
            gone: Vec::new(),
        };
        let faces = data.join("faces.npz");
        if faces.exists() {
            let z = read(&faces)?;
            let bad = |what: &str| format!("faces.npz has no usable {what}");
            let emb = z.get("emb").ok_or_else(|| bad("prints"))?;
            let n = emb.shape.first().copied().unwrap_or(0);
            s.emb = emb.f32().map_err(|_| bad("prints"))?.to_vec();
            s.dim = match emb.shape.as_slice() {
                [_, d] => *d,
                _ if n == 0 => dim,
                _ => return Err(bad("prints")),
            };
            s.file = z.get("files").and_then(|a| a.text().ok()).ok_or_else(|| bad("file names"))?.to_vec();
            let boxes = z.get("box").and_then(|a| a.f32().ok()).ok_or_else(|| bad("boxes"))?;
            s.boxes = boxes.chunks_exact(5).map(|b| [b[0], b[1], b[2], b[3], b[4]]).collect();
            s.quality = match z.get("quality") {
                Some(q) => q.f32().map_err(|_| bad("quality"))?.to_vec(),
                None => vec![f32::NAN; n],
            };
            // databases from before model tracking were all made with SFace
            s.model = match z.get("model").and_then(|a| a.text().ok()).and_then(|t| t.first()) {
                Some(m) => m.clone(),
                None => FIRST_MODEL.to_string(),
            };
            if let Some(fid) = z.get("fid") {
                // older databases get their IDs from the labeling side's migration
                s.fid = fid.text().map_err(|_| bad("face IDs"))?.to_vec();
                s.photo = z.get("photo").and_then(|a| a.text().ok()).ok_or_else(|| bad("photos"))?.to_vec();
            }
            if s.file.len() != n || s.boxes.len() != n || s.quality.len() != n || s.emb.len() != n * s.dim {
                return Err(bad("faces: its lists are of different lengths"));
            }
            if z.contains_key("thumbs") {
                // before 2026-11-09: the pictures inside faces.npz; moved out at the next save
                s.thumbs = cut(&z).ok_or_else(|| bad("pictures"))?;
                s.thumbs_changed = true;
            } else {
                s.thumbs = s.load_thumbs(n);
            }
            s.thumbs.resize(n, Vec::new());
        }
        s.photos = match std::fs::read(data.join("photos.json")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(format!("can't read photos.json: {e}")),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(Value::Object(m)) => m,
                Ok(_) => return Err("photos.json is not a list of photos".to_string()),
                Err(e) => return Err(format!("can't read photos.json: {e}")),
            },
        };
        s.fids = s.fid.iter().cloned().zip(0..).collect();
        s.gone = (0..s.len()).map(|i| s.print(i).iter().all(|v| *v == 0.0)).collect();
        Ok(s)
    }

    /// The pictures from thumbs.npz, matched to the faces by face ID (none where the file has none; none
    /// at all when the file is missing or can't be read).
    fn load_thumbs(&self, n: usize) -> Vec<Vec<u8>> {
        let Some(z) = read(&self.data.join("thumbs.npz")).ok() else { return vec![Vec::new(); n] };
        let Some(got) = cut(&z) else { return vec![Vec::new(); n] };
        match z.get("fid").and_then(|a| a.text().ok()) {
            Some(fids) if !self.fid.is_empty() => {
                let by: HashMap<&str, &Vec<u8>> = fids.iter().map(String::as_str).zip(&got).collect();
                self.fid.iter().map(|f| by.get(f.as_str()).map_or_else(Vec::new, |t| (*t).clone())).collect()
            }
            _ => got.into_iter().chain(std::iter::repeat_with(Vec::new)).take(n).collect(),
        }
    }

    /// How many faces there are.
    pub fn len(&self) -> usize {
        self.file.len()
    }

    pub fn is_empty(&self) -> bool {
        self.file.is_empty()
    }

    /// A database from before the permanent IDs: the labeling side's migration gives them.
    pub fn needs_ids(&self) -> bool {
        !self.is_empty() && self.fid.is_empty()
    }

    /// Face i's print.
    pub fn print(&self, i: usize) -> &[f32] {
        &self.emb[i * self.dim..(i + 1) * self.dim]
    }

    /// Every face's print, one after another.
    pub fn prints(&self) -> &[f32] {
        &self.emb
    }

    /// Every face whose print changed since the library was read, in the order it happened.
    pub fn print_log(&self) -> &[usize] {
        &self.print_log
    }

    /// Give face i another print (made again from its photo).
    pub fn set_print(&mut self, i: usize, emb: &[f32]) -> Result<()> {
        if emb.len() != self.dim {
            return Err(format!("a print of {} numbers in a library of {}", emb.len(), self.dim));
        }
        self.write_print(i, emb);
        Ok(())
    }

    /// Every print made again by another recognition model. Prints of two models can't be compared, so a
    /// library has the prints of one model only, all of them: `make` is asked for each face that has a print
    /// and gives its new one (of `dim` numbers), made from what the library keeps of the face. All or nothing:
    /// when a face can't be made, nothing here changes and the faces that couldn't are the answer (a face left
    /// without a print would be sent to the other devices as a face that was Ignored). How many were made.
    pub fn remake_prints(
        &mut self,
        model: &str,
        dim: usize,
        make: &mut dyn FnMut(&Store, usize) -> Option<Vec<f32>>,
    ) -> std::result::Result<usize, Vec<usize>> {
        let n = self.len();
        let (mut emb, mut made, mut failed) = (vec![0f32; n * dim], 0, Vec::new());
        for i in (0..n).filter(|&i| !self.gone[i]) {
            match make(self, i).filter(|p| p.len() == dim && p.iter().any(|v| *v != 0.0)) {
                Some(p) => {
                    emb[i * dim..(i + 1) * dim].copy_from_slice(&p);
                    made += 1;
                }
                None => failed.push(i),
            }
        }
        if !failed.is_empty() {
            return Err(failed);
        }
        (self.emb, self.dim, self.model) = (emb, dim, model.to_string());
        self.print_log.extend(0..n);
        Ok(made)
    }

    /// The one place a face's print changes: the print, whether it has one, and the note of the change.
    fn write_print(&mut self, i: usize, emb: &[f32]) {
        self.emb[i * self.dim..(i + 1) * self.dim].copy_from_slice(emb);
        self.gone[i] = emb.iter().all(|v| *v == 0.0);
        self.print_log.push(i);
    }

    /// Where the face with this permanent ID is.
    pub fn index_of(&self, fid: &str) -> Option<usize> {
        self.fids.get(fid).copied()
    }

    /// Record the permanent ID of a new face in `photo` (or in a video's frame) and return it.
    pub fn claim_fid(&mut self, photo: &str, row: &[f32], frame: Option<i64>) -> String {
        let mut fid = face_key(photo, row, frame);
        while self.fids.contains_key(&fid) {
            fid.push('x'); // practically never: two faces centred on the same pixel
        }
        self.fids.insert(fid.clone(), self.fid.len());
        self.fid.push(fid.clone());
        self.photo.push(photo.to_string());
        fid
    }

    fn as_box(row: &[f32]) -> [f32; 5] {
        std::array::from_fn(|k| row.get(k).copied().unwrap_or(0.0))
    }

    /// Append one face (`frame`: its frame number, for a face from a video). Its index.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        photo: &str,
        file: &str,
        row: &[f32],
        emb: &[f32],
        crop: &Bgr,
        quality: f32,
        frame: Option<i64>,
    ) -> Result<usize> {
        if emb.len() != self.dim {
            return Err(format!("a print of {} numbers in a library of {}", emb.len(), self.dim));
        }
        let picture = crop.to_jpeg(THUMB_QUALITY).map_err(|e| e.0)?;
        self.claim_fid(photo, row, frame);
        self.quality.push(quality);
        self.emb.extend_from_slice(emb);
        self.gone.push(emb.iter().all(|v| *v == 0.0));
        self.file.push(file.to_string());
        self.boxes.push(Self::as_box(row));
        self.thumbs.push(picture);
        self.thumbs_changed = true;
        Ok(self.len() - 1)
    }

    /// Add or update a face by its permanent ID, as another device has it (the sync): no new ID is made, the
    /// picture is already a JPEG (empty for none) and the print is all zeros for a face without one. Its index.
    #[allow(clippy::too_many_arguments)]
    pub fn put(
        &mut self,
        fid: &str,
        photo: &str,
        file: &str,
        row: &[f32],
        emb: &[f32],
        picture: &[u8],
        quality: f32,
    ) -> Result<usize> {
        if emb.len() != self.dim {
            return Err(format!("a print of {} numbers in a library of {}", emb.len(), self.dim));
        }
        let i = match self.index_of(fid) {
            None => {
                self.fids.insert(fid.to_string(), self.fid.len());
                self.fid.push(fid.to_string());
                self.photo.push(photo.to_string());
                self.quality.push(quality);
                self.emb.extend_from_slice(emb);
                self.gone.push(emb.iter().all(|v| *v == 0.0));
                self.file.push(file.to_string());
                self.boxes.push(Self::as_box(row));
                self.thumbs.push(picture.to_vec());
                self.len() - 1
            }
            Some(i) => {
                (self.photo[i], self.file[i]) = (photo.to_string(), file.to_string());
                (self.boxes[i], self.quality[i]) = (Self::as_box(row), quality);
                if self.print(i) != emb {
                    self.write_print(i, emb);
                }
                self.thumbs[i] = picture.to_vec();
                i
            }
        };
        self.thumbs_changed = true;
        Ok(i)
    }

    /// Face i's picture (112x112), or None when there is none on this computer.
    pub fn thumb(&self, i: usize) -> Option<Bgr> {
        let t = &self.thumbs[i];
        if t.is_empty() { None } else { Bgr::from_jpeg(t).ok() }
    }

    pub fn set_thumb(&mut self, i: usize, crop: &Bgr) -> Result<()> {
        self.thumbs[i] = crop.to_jpeg(THUMB_QUALITY).map_err(|e| e.0)?;
        self.thumbs_changed = true;
        Ok(())
    }

    /// Faces that should have a picture and have none here.
    pub fn missing_thumbs(&self) -> usize {
        (0..self.len()).filter(|&i| self.thumbs[i].is_empty() && !self.printless(i)).count()
    }

    /// True if face i's print and picture were deleted (`forget`): an Ignored face, or a forgotten person's.
    pub fn printless(&self, i: usize) -> bool {
        self.gone[i]
    }

    /// Delete face i's print and picture; keep only where it is (face ID, photo, box, quality), so the photo
    /// isn't read again and the face doesn't come back as a new one. What was deleted, for an undo.
    pub fn forget(&mut self, i: usize) -> Forgotten {
        let old = Forgotten { emb: self.print(i).to_vec(), thumb: std::mem::take(&mut self.thumbs[i]) };
        self.write_print(i, &vec![0.0; self.dim]);
        if !old.thumb.is_empty() {
            self.thumbs_changed = true;
        }
        old
    }

    /// Put back a print and picture that `forget` returned (Undo in the same session).
    pub fn restore(&mut self, i: usize, saved: Forgotten) {
        self.write_print(i, &saved.emb);
        self.thumbs[i] = saved.thumb;
        self.thumbs_changed = true;
    }

    /// Write one archive whole, then put it in the old one's place: never a half-written database.
    fn write(&self, name: &str, arrays: &[(&str, Array)]) -> Result<()> {
        let (tmp, dest) = (self.data.join(name.replace(".npz", ".tmp.npz")), self.data.join(name));
        let said = |e: std::io::Error| format!("can't write {name}: {e}");
        let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(said)?);
        npy::write_npz(arrays, &mut out).map_err(said)?;
        std::io::Write::flush(&mut out).map_err(said)?;
        drop(out);
        replace_file(&tmp, &dest).map_err(said)
    }

    pub fn save(&mut self) -> Result<()> {
        if self.thumbs_changed || !self.data.join("thumbs.npz").exists() {
            // the pictures first: faces.npz without them is only written once they are safe in thumbs.npz
            let mut off = Vec::with_capacity(self.thumbs.len() + 1);
            let mut all = Vec::with_capacity(self.thumbs.iter().map(Vec::len).sum());
            off.push(0i64);
            for t in &self.thumbs {
                all.extend_from_slice(t);
                off.push(all.len() as i64);
            }
            self.write(
                "thumbs.npz",
                &[
                    ("fid", Array::texts(self.fid.clone())),
                    ("thumbs", Array::bytes(all)),
                    ("thumb_off", Array::wholes(off)),
                ],
            )?;
            self.thumbs_changed = false;
        }
        let n = self.len();
        self.write(
            "faces.npz",
            &[
                ("emb", Array::table(self.emb.clone(), n, self.dim)),
                ("model", Array::one_text(&self.model)),
                ("files", Array::texts(self.file.clone())),
                ("fid", Array::texts(self.fid.clone())),
                ("photo", Array::texts(self.photo.clone())),
                ("box", Array::table(self.boxes.iter().flatten().copied().collect(), n, 5)),
                ("quality", Array::floats(self.quality.clone())),
            ],
        )?;
        self.save_photos()
    }

    /// Write photos.json alone (the photos' records changed, the faces didn't).
    pub fn save_photos(&self) -> Result<()> {
        let photos = filejson::rows_of(&self.photos, true);
        atomic_write(&self.data.join("photos.json"), photos.as_bytes())
            .map_err(|e| format!("can't write photos.json: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("face-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn crop(shade: u8) -> Bgr {
        Bgr { w: 112, h: 112, px: vec![shade; 112 * 112 * 3] }
    }

    #[test]
    fn a_library_is_saved_and_read_back() {
        let dir = folder("roundtrip");
        let mut s = Store::load(&dir, "opsis_1", 4).unwrap();
        assert!(s.is_empty() && !s.needs_ids() && s.model == "opsis_1");
        s.save().unwrap(); // an empty library is a library: a new device saves before it has a face
        let mut s = Store::load(&dir, "another model", 9).unwrap();
        assert_eq!((s.len(), s.dim, s.model.as_str()), (0, 4, "opsis_1"));

        let a = s
            .add(
                "0123456789abcdef0123",
                "IMG_1.jpg",
                &[10.4, 20.0, 31.0, 41.0, 0.93, 7.0],
                &[0.5, -0.5, 0.1, 0.7],
                &crop(90),
                0.61,
                None,
            )
            .unwrap();
        let b = s
            .add(
                "0123456789abcdef0123",
                "IMG_1.jpg",
                &[10.4, 20.0, 31.0, 41.0, 0.9],
                &[0.1, 0.2, 0.3, 0.4],
                &crop(120),
                f32::NAN,
                None,
            )
            .unwrap();
        let c = s
            .add(
                "fedcba9876543210ffff",
                "Zoë's clip.mov",
                &[5.0, 6.0, 20.0, 20.0, 0.88],
                &[1.0, 0.0, 0.0, 0.0],
                &crop(40),
                0.4,
                Some(120),
            )
            .unwrap();
        // the centre of the first face: (10.4 + 15.5, 20 + 20.5) = (25.9, 40.5): 26 and, the half to the even, 40
        assert_eq!(s.fid, ["0123456789abcdef-26-40", "0123456789abcdef-26-40x", "fedcba9876543210-15-16-f120"]);
        assert_eq!((a, b, c), (0, 1, 2));
        s.photos.insert(
            "0123456789abcdef0123".to_string(),
            json!({"path": "C:\\p\\IMG_1.jpg", "size": 5, "mtime": 1700000000.25, "faces": 2}),
        );
        s.photos.insert(
            "fedcba9876543210ffff".to_string(),
            json!({"path": "/v/Zoë's clip.mov", "video": true, "at": {"fedcba9876543210-15-16-f120": 120}}),
        );
        s.save().unwrap();

        let r = Store::load(&dir, "x", 1).unwrap();
        assert_eq!((r.len(), r.dim), (3, 4));
        assert_eq!((&r.fid, &r.photo, &r.file), (&s.fid, &s.photo, &s.file));
        assert_eq!(r.boxes[0], [10.4, 20.0, 31.0, 41.0, 0.93]);
        assert_eq!(r.print(2), [1.0, 0.0, 0.0, 0.0]);
        assert!(r.quality[1].is_nan() && r.quality[0] == 0.61);
        assert_eq!(r.thumbs, s.thumbs);
        let pic = r.thumb(0).unwrap();
        assert_eq!((pic.w, pic.h), (112, 112));
        assert_eq!(r.photos, s.photos);
        // photos.json: a line a photo, in its fixed form
        let text = std::fs::read_to_string(dir.join("photos.json")).unwrap();
        assert!(text.starts_with("{\n\"0123456789abcdef0123\": {\"path\": \"C:\\\\p\\\\IMG_1.jpg\", \"size\": 5, \"mtime\": 1700000000.25, \"faces\": 2},\n"), "{text}");
        assert!(text.contains("\"path\": \"/v/Zo\\u00eb's clip.mov\""), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_forgotten_face_keeps_only_where_it_is() {
        let dir = folder("forget");
        let mut s = Store::load(&dir, "m", 3).unwrap();
        s.add("aaaaaaaaaaaaaaaaaaaa", "a.jpg", &[0.0, 0.0, 10.0, 10.0, 0.9], &[0.6, 0.0, 0.8], &crop(50), 0.5, None)
            .unwrap();
        s.add("bbbbbbbbbbbbbbbbbbbb", "b.jpg", &[0.0, 0.0, 10.0, 10.0, 0.9], &[0.0, 1.0, 0.0], &crop(60), 0.5, None)
            .unwrap();
        s.save().unwrap();
        assert_eq!(s.missing_thumbs(), 0);
        let old = s.forget(0);
        assert!(s.printless(0) && !s.printless(1) && s.thumb(0).is_none() && s.thumbs_changed);
        assert_eq!(s.missing_thumbs(), 0, "a forgotten face is not one whose picture is missing");
        s.save().unwrap();
        let r = Store::load(&dir, "m", 3).unwrap();
        assert!(r.printless(0) && r.thumbs[0].is_empty() && !r.thumbs[1].is_empty());
        assert_eq!((r.fid[0].as_str(), r.boxes[0]), ("aaaaaaaaaaaaaaaa-5-5", [0.0, 0.0, 10.0, 10.0, 0.9]));
        // undone in the same session
        s.restore(0, old);
        assert_eq!(s.print(0), [0.6, 0.0, 0.8]);
        assert!(s.thumb(0).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_face_from_another_device_is_added_or_updated_by_its_id() {
        let dir = folder("put");
        let mut s = Store::load(&dir, "m", 2).unwrap();
        let i = s
            .put(
                "ffff-1-2",
                "ffffffffffffffff",
                "f.jpg",
                &[1.0, 2.0, 3.0, 4.0, 0.5],
                &[0.0, 1.0],
                b"\xff\xd8 a picture",
                0.3,
            )
            .unwrap();
        let j = s
            .put("eeee-1-2", "eeeeeeeeeeeeeeee", "e.jpg", &[1.0, 2.0, 3.0, 4.0, 0.5], &[0.0, 0.0], b"", f32::NAN)
            .unwrap();
        assert_eq!((i, j, s.len()), (0, 1, 2));
        assert!(s.printless(1) && s.missing_thumbs() == 0);
        let again = s
            .put("ffff-1-2", "ffffffffffffffff", "renamed.jpg", &[9.0, 9.0, 9.0, 9.0, 0.9], &[1.0, 0.0], b"", 0.7)
            .unwrap();
        assert_eq!((again, s.len(), s.file[0].as_str(), s.print(0)), (0, 2, "renamed.jpg", &[1.0f32, 0.0][..]));
        assert_eq!(s.missing_thumbs(), 1, "it has a print and no picture here");
        assert!(s.put("x", "p", "f", &[0.0; 5], &[1.0, 2.0, 3.0], b"", 0.0).is_err(), "a print of another length");
        // a new face in the same photo at the same place as one that came from another device: another ID
        assert_eq!(s.claim_fid("ffff", &[0.0, 1.0, 2.0, 2.0], None), "ffff-1-2x");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn older_databases_are_still_read() {
        let dir = folder("old");
        std::fs::create_dir_all(&dir).unwrap();
        // before the pictures moved out, and before the IDs and the model's name
        let mut out = std::fs::File::create(dir.join("faces.npz")).unwrap();
        npy::write_npz(
            &[
                ("emb", Array::table(vec![0.1, 0.2, 0.3, 0.4], 2, 2)),
                ("files", Array::texts(vec!["a.jpg".to_string(), "b.jpg".to_string()])),
                ("box", Array::table(vec![1.0; 10], 2, 5)),
                ("thumbs", Array::bytes(b"onetwo".to_vec())),
                ("thumb_off", Array::wholes(vec![0, 3, 6])),
            ],
            &mut out,
        )
        .unwrap();
        drop(out);
        let mut s = Store::load(&dir, "opsis_1", 512).unwrap();
        assert_eq!((s.len(), s.dim, s.model.as_str()), (2, 2, FIRST_MODEL));
        assert!(s.needs_ids() && s.quality.iter().all(|q| q.is_nan()));
        assert_eq!(s.thumbs, [b"one".to_vec(), b"two".to_vec()]);
        assert!(s.thumbs_changed, "the pictures move to their own file at the next save");
        s.save().unwrap();
        assert_eq!(Store::load(&dir, "x", 1).unwrap().thumbs, s.thumbs);
        // a pictures file that can't be read means no pictures, not no library
        std::fs::write(dir.join("thumbs.npz"), b"not an archive").unwrap();
        let r = Store::load(&dir, "x", 1).unwrap();
        assert!(r.len() == 2 && r.thumbs.iter().all(Vec::is_empty));
        let _ = std::fs::remove_dir_all(dir);
    }
}
