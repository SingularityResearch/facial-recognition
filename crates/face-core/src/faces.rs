//! A frame's faces: found, aligned, and each with its print, quality and (asked for) expression. The
//! steps, sizes and cut-offs are measured values.

use std::path::Path;

use crate::{
    Error, Result,
    geometry::{self, align_matrix},
    image::{Bgr, round_even},
    runtime::{Model, Probe, Runtime},
    yunet::{self, Row},
};

pub const DET_SCORE: f64 = 0.85; // detector confidence needed to keep a face
pub const MIN_FACE: f32 = 40.0; // px: smaller faces are too blurry to recognise reliably
pub const Q_ENROLL: f32 = 0.30; // faces below this quality are not used to learn a person
pub const Q_EXPR: f32 = 0.25; // faces below this quality aren't read for expression
pub const MAX_TURN: f64 = 0.35; // nor a head turned more than this (about 35 degrees)
pub const DET_SESSIONS: usize = 12; // input sizes that get a graphics session of their own
pub const EXPRESSIONS: [&str; 7] = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"];

const DET_MODEL: &str = "face_detection_yunet_2023mar.onnx";
const DET_DYNAMIC: &str = "face_detection_yunet_2023mar_dynamic.onnx";
const QUAL_MODEL: &str = "ediffiqa_tiny_jun2024.onnx";
const EXPR_MODEL: &str = "facial_expression_recognition_mobilefacenet_2022july.onnx";

/// A recognition model and its own measured settings: `match_at` is the cosine similarity needed to call a
/// face a known person, `auto` the one at which the labeling tool names a face by itself, `cluster` the
/// cosine distance for grouping faces as one person. Each model has its own scale of scores, so the settings
/// belong to the model and are measured with it (`rec_eval` in `face-library`).
#[derive(Clone, Copy, Debug)]
pub struct RecModel {
    pub name: &'static str,
    pub file: &'static str,
    pub dim: usize,
    pub match_at: f64,
    pub auto: f64,
    pub cluster: f64,
}

/// The project's own recognizer, Opsis 1 (`opsis_1.onnx`, one of `face-tools`' own models): trained by the project
/// from OpenCV Zoo's SFace (Apache 2.0) to give a poor picture of a face (blurred, tiny, moved, dark) the print
/// of a good one. A library whose prints are of another model has every print made again when it is opened
/// (`LabelDb::open_making`), so the name here is also what a library and the shared database say their
/// prints are of: a new model gets a new name.
///
/// Its settings, measured 2026-11-22 with `rec_eval --curve` on a library of 134 named people (940 faces, at
/// most 40 a person, each looked up among all the others):
/// - `match_at` 0.55: 82% of the faces of known people named, none of them wrongly, and 4.8% of strangers taken
///   for someone (0.50: 87% and 9%; 0.60: 75% and 0.6%);
/// - `auto` 0.65: the lowest score at which no stranger was named at all;
/// - `cluster` 0.58: SFace's, measured with that model; the prints lie where SFace's do for good
///   pictures, and it has not been measured again.
pub const REC_MODELS: [RecModel; 1] =
    [RecModel { name: "opsis_1", file: "opsis_1.onnx", dim: 128, match_at: 0.55, auto: 0.65, cluster: 0.58 }];

/// One face of a frame.
pub struct Face {
    pub row: Row,
    /// the face print, length 1
    pub emb: Vec<f32>,
    /// the aligned 112x112 face
    pub crop: Bgr,
    /// eDifFIQA quality, NaN without the quality model
    pub quality: f32,
}

pub struct Faces {
    det: Model,
    /// the detector takes any input size (the dynamic model); otherwise 640x640 only
    det_any_size: bool,
    rec: Model,
    qual: Option<Model>,
    expr: Option<Model>,
    pub rec_model: RecModel,
}

impl Faces {
    pub fn load(rt: &Runtime, models: &Path) -> Result<Faces> {
        let rec_model = *REC_MODELS
            .iter()
            .find(|m| models.join(m.file).is_file())
            .ok_or_else(|| Error::msg(format!("No face recognition model found in {}", models.display())))?;
        let dynamic = models.join(DET_DYNAMIC);
        let det_any_size = dynamic.is_file();
        let det_file = if det_any_size { dynamic } else { models.join(DET_MODEL) };
        let optional = |file: &str, what: &'static str| -> Result<Option<Model>> {
            let p = models.join(file);
            if p.is_file() { Ok(Some(rt.model(&p, what, &Probe::face())?)) } else { Ok(None) }
        };
        // The four models side by side: a start waits for the slowest of them, not for their sum (a check
        // that has to time its runs still takes its turn alone: `runtime::CHECKING`).
        let (det, rec, qual, expr) = std::thread::scope(|s| {
            let ended =
                |r: std::thread::Result<_>| r.unwrap_or_else(|_| Err(Error::msg("a model's load ended in a panic")));
            let det = s.spawn(|| rt.model(&det_file, "Face finding", &Probe::frame()));
            let rec = s.spawn(|| rt.model(&models.join(rec_model.file), "Recognition", &Probe::face()));
            let qual = s.spawn(|| optional(QUAL_MODEL, "Quality"));
            let expr = s.spawn(|| optional(EXPR_MODEL, "Expression"));
            (ended(det.join()), ended(rec.join()), qual.join(), expr.join())
        });
        let panicked = || Error::msg("a model's load ended in a panic");
        let mut det = det?;
        det.per_shape = DET_SESSIONS;
        Ok(Faces {
            det,
            det_any_size,
            rec: rec?,
            qual: qual.map_err(|_| panicked())??,
            expr: expr.map_err(|_| panicked())??,
            rec_model,
        })
    }

    /// Which processor runs each model now.
    pub fn devices(&self) -> Vec<(&'static str, Option<String>)> {
        vec![
            ("detection", Some(self.det.device())),
            ("recognition", Some(self.rec.device())),
            ("quality", self.qual.as_ref().map(Model::device)),
            ("expression", self.expr.as_ref().map(Model::device)),
        ]
    }

    pub fn has_expression(&self) -> bool {
        self.expr.is_some()
    }

    /// What a picture is padded to for the detector: on a graphics processor to a multiple of 320
    /// (`yunet::DET_PAD`); on the processor only to the model's own 32,
    /// as OpenCV's detector does. The two find the same faces
    /// but not the very same boxes (the black border reaches into a face near the edge differently), and
    /// which faces are found near the cut-off can differ: on 400 snapshots padded tight, 3 had another number
    /// of faces than the graphics path gives. So each path keeps the padding its thresholds were measured
    /// with, though tight padding would also be 2 ms a frame faster on a graphics processor.
    pub fn pad(&self) -> usize {
        if self.det.on_gpu() { yunet::DET_PAD } else { yunet::MODEL_PAD }
    }

    /// YuNet's rows for the faces in `img` shrunk to `side` px at most, in `img`'s own coordinates. The
    /// picture goes to the model padded to a multiple of `pad`.
    pub fn detect_at(&self, img: &Bgr, side: usize, score: f64, pad: usize) -> Result<Vec<Row>> {
        let s = (side as f64 / img.w.max(img.h) as f64).min(1.0);
        let small;
        let pic = if s < 1.0 {
            small = img.resize_area(round_even(img.w as f64 * s) as usize, round_even(img.h as f64 * s) as usize);
            &small
        } else {
            img
        };
        let (ph, pw, x) = if self.det_any_size {
            yunet::input(pic, pad)
        } else {
            if pic.w > 640 || pic.h > 640 {
                return Err(Error::msg("this face-finding model takes pictures up to 640 px only"));
            }
            let mut wide = Bgr::new(640, 640);
            for y in 0..pic.h {
                wide.px[y * 640 * 3..y * 640 * 3 + pic.w * 3].copy_from_slice(pic.row(y));
            }
            yunet::input(&wide, yunet::DET_PAD)
        };
        let out = self.det.run(&[1, 3, ph, pw], &x)?;
        if out.len() < 12 {
            return Err(Error::msg("the face-finding model gave fewer outputs than expected"));
        }
        let mut rows = yunet::rows(&out, pw, score, 0.3, 5000);
        let back = s as f32;
        for r in &mut rows {
            for v in &mut r[..14] {
                *v /= back;
            }
        }
        Ok(rows)
    }

    /// The faces of a picture, each with its print, aligned crop and quality. `sizes`: the detector runs once
    /// per size (a photo: full size for small faces and 640 px for close-ups; a webcam frame: 640 only).
    pub fn detect(&self, img: &Bgr, sizes: &[usize]) -> Result<Vec<Face>> {
        let pad = self.pad();
        let mut rows = Vec::new();
        for &side in sizes {
            rows.extend(self.detect_at(img, side, DET_SCORE, pad)?);
        }
        rows.sort_by(|a, b| b[14].total_cmp(&a[14])); // stable, best first
        let mut kept: Vec<Row> = Vec::new();
        for r in rows {
            if kept.iter().all(|k| geometry::iou(&r, k) < 0.4) {
                // the same face found at two sizes
                kept.push(r);
            }
        }
        let mut out = Vec::new();
        for row in kept {
            if row[2] < MIN_FACE || row[3] < MIN_FACE {
                continue;
            }
            let crop = align(img, &row)?;
            let emb = self.embed(&crop)?;
            let quality = self.quality(&crop)?;
            out.push(Face { row, emb, crop, quality });
        }
        Ok(out)
    }

    /// The face print of an aligned 112x112 crop, scaled to length 1 (so a dot product is the cosine).
    pub fn embed(&self, crop: &Bgr) -> Result<Vec<f32>> {
        let x = planes(crop, |v| (v as f32 - 127.5) / 127.5, false);
        let mut v = self.rec.run(&[1, 3, 112, 112], &x)?.swap_remove(0);
        let norm = v.iter().map(|a| a * a).sum::<f32>().sqrt() + 1e-9;
        for a in &mut v {
            *a /= norm;
        }
        Ok(v)
    }

    /// eDifFIQA quality of an aligned crop, roughly 0 (unusable) to 0.8 (excellent); NaN without the model.
    pub fn quality(&self, crop: &Bgr) -> Result<f32> {
        let Some(model) = &self.qual else { return Ok(f32::NAN) };
        let x = planes(crop, |v| (v as f32 / 255.0 - 0.5) / 0.5, false);
        Ok(model.run(&[1, 3, 112, 112], &x)?.first().and_then(|o| o.first().copied()).unwrap_or(f32::NAN))
    }

    /// Probabilities of the EXPRESSIONS for an aligned crop, the crop and its mirror image averaged; None
    /// without the expression model.
    pub fn expression(&self, crop: &Bgr) -> Result<Option<[f64; 7]>> {
        let Some(model) = &self.expr else { return Ok(None) };
        let mut probs = [0f64; 7];
        for mirrored in [false, true] {
            let x = planes(crop, |v| (v as f32 - 127.5) / 127.5, mirrored);
            let logits = model.run(&[1, 3, 112, 112], &x)?.swap_remove(0);
            if logits.len() != 7 {
                return Err(Error::msg("the expression model gave an unexpected answer"));
            }
            let top = logits.iter().fold(f64::MIN, |m, &v| m.max(v as f64));
            let e: Vec<f64> = logits.iter().map(|&v| (v as f64 - top).exp()).collect();
            let sum: f64 = e.iter().sum();
            for (p, v) in probs.iter_mut().zip(&e) {
                *p += v / sum;
            }
        }
        Ok(Some(probs.map(|p| p / 2.0)))
    }

    /// Each model run once on a blank face, so the first live frame isn't the slow one (a graphics session
    /// sets itself up at its first run). Milliseconds that first run took, by model.
    pub fn warmup(&self) -> Vec<(&'static str, f64)> {
        let crop = Bgr::new(112, 112);
        let mut ms = Vec::new();
        let mut time = |name: &'static str, step: &dyn Fn()| {
            let t = std::time::Instant::now();
            step();
            ms.push((name, (t.elapsed().as_secs_f64() * 10_000.0).round() / 10.0));
        };
        time("detection", &|| drop(self.detect_at(&Bgr::new(640, 360), 640, DET_SCORE, yunet::MODEL_PAD)));
        time("recognition", &|| drop(self.embed(&crop)));
        if self.qual.is_some() {
            time("quality", &|| drop(self.quality(&crop)));
        }
        if self.expr.is_some() {
            time("expression", &|| drop(self.expression(&crop)));
        }
        ms
    }
}

/// A detected face rotated, scaled and cut to the 112x112 template the recognizers expect.
pub fn align(img: &Bgr, row: &Row) -> Result<Bgr> {
    let mut kps = [[0f32; 2]; 5];
    for (i, k) in kps.iter_mut().enumerate() {
        *k = [row[4 + 2 * i], row[5 + 2 * i]];
    }
    let m = align_matrix(&kps).ok_or_else(|| Error::msg("a face's points don't fit the template"))?;
    Ok(img.warp_affine(&m, 112, 112))
}

/// Why a face can't be read for expression: "blurry", "turned", or None when it can. Blur and profile views
/// don't make the expression model unsure, they make it wrong, so such frames get no reading at all.
pub fn expression_skip(row: &Row, quality: f32) -> Option<&'static str> {
    if quality < Q_EXPR {
        // NaN (no quality model) compares false: read
        return Some("blurry");
    }
    if geometry::head_turn(row) > MAX_TURN {
        return Some("turned");
    }
    None
}

/// A 112x112 crop as the models take it: red, green and blue planes one after another, each value put
/// through `f`; `mirrored` flips it left to right.
fn planes(crop: &Bgr, f: impl Fn(u8) -> f32, mirrored: bool) -> Vec<f32> {
    let (w, h) = (crop.w, crop.h);
    let mut x = vec![0f32; 3 * w * h];
    for y in 0..h {
        let row = crop.row(y);
        for xx in 0..w {
            let s = if mirrored { w - 1 - xx } else { xx } * 3;
            let at = y * w + xx;
            x[at] = f(row[s + 2]);
            x[w * h + at] = f(row[s + 1]);
            x[2 * w * h + at] = f(row[s]);
        }
    }
    x
}
