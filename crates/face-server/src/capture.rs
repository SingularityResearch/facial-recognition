//! Collect faces: each face the page asks for is saved for the
//! labeling tool as its own small picture, the face with room around it and never the whole frame, with
//! every other face in that room greyed out so that nobody else is saved. Each is `<time>-<k>.jpg` in
//! `HOME/webcam` with `<time>-<k>.json` beside it, the tracker's verdict ({when, faces: [{box, name, sim}]},
//! the box in that picture's pixels).
//!
//! The verdict file's form is fixed, byte for byte. The picture may come from
//! any JPEG encoder: the same size, the same pixels to within what a JPEG loses.

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{TimeZone, Utc};
use face_core::image::Bgr;
use serde_json::{Value, json};

use crate::{filejson, paths::atomic_write};

/// room kept around a collected face, in face sizes: what the labeling tool's import looks in to find the
/// face again in the saved picture
const CAPTURE_MARGIN: f64 = 0.6;
/// smaller pictures are enlarged to this (the longer side), as that import does to find small faces
const CAPTURE_MIN: f64 = 160.0;
/// another face is greyed out over its box and this much more on each side
const OTHER_PAD: f64 = 0.25;
const QUALITY: u8 = 92;

type Rect = [f64; 4];

/// [[x, y, w, h], ...] from the page's list ({box: [...]} or plain boxes), checked.
fn boxes(items: &Value, what: &str) -> Result<Vec<Rect>, String> {
    let list = items.as_array().filter(|l| l.len() <= 20).ok_or_else(|| format!("expected at most 20 {what}"))?;
    list.iter()
        .map(|f| {
            let b = if f.is_object() { f.get("box") } else { Some(f) };
            let numbers: Option<Vec<f64>> = b.and_then(Value::as_array).filter(|b| b.len() == 4).and_then(|b| {
                b.iter().map(|v| if v.is_number() { v.as_f64().filter(|n| n.is_finite()) } else { None }).collect()
            });
            match numbers {
                Some(n) if n[2] > 0.0 && n[3] > 0.0 => Ok([n[0], n[1], n[2], n[3]]),
                _ => Err(format!("each of the {what} needs a box [x, y, w, h]")),
            }
        })
        .collect()
}

/// `v` rounded to `places` decimals, a half to the even neighbour.
fn rounded(v: f64, places: usize) -> f64 {
    format!("{v:.places$}").parse().unwrap_or(v)
}

/// The tracker's name for a face, as text, whatever the page sent it as.
fn name_of(face: &Value) -> String {
    let name = match face.get("name") {
        _ if !face.is_object() => "Unknown".to_string(),
        None => "Unknown".to_string(),
        Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => if *b { "True" } else { "False" }.to_string(),
        Some(other) => other.to_string(),
    };
    name.chars().take(100).collect()
}

fn sim_of(face: &Value) -> Result<f64, String> {
    let sim = match face.get("sim") {
        _ if !face.is_object() => 0.0,
        None => 0.0,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => {
            s.trim().parse::<f64>().map_err(|_| format!("could not convert string to float: '{s}'"))?
        }
        Some(Value::Bool(b)) => *b as u8 as f64,
        Some(_) => return Err("the match must be a number".to_string()),
    };
    Ok(rounded(sim.clamp(-1.0, 1.0), 3))
}

/// The pictures to save for the faces asked for: (k, the picture, its verdict). `faces` are boxes in the
/// frame's pixels with the tracker's name and match; `others` are the boxes of everyone else in view.
pub fn cut(frame: &Bgr, faces: &Value, others: &Value) -> Result<Vec<(usize, Bgr, Value)>, String> {
    let asked = boxes(faces, "faces")?;
    if asked.is_empty() {
        return Err("expected 1-20 faces".to_string());
    }
    let rest = boxes(others, "other faces")?;
    let (fw, fh) = (frame.w as i64, frame.h as i64);
    let mut out = Vec::new();
    for (k, (face, [x, y, w, h])) in faces.as_array().into_iter().flatten().zip(&asked).enumerate() {
        let m = w.max(*h) * CAPTURE_MARGIN;
        let (x0, y0) = (((x - m) as i64).max(0), ((y - m) as i64).max(0));
        let (x1, y1) = (fw.min((x + w + m).ceil() as i64), fh.min((y + h + m).ceil() as i64));
        if x1 - x0 < 8 || y1 - y0 < 8 {
            continue; // outside the frame
        }
        let (pw, ph) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut part = Bgr::new(pw, ph);
        for row in 0..ph {
            let from = &frame.row(y0 as usize + row)[x0 as usize * 3..x1 as usize * 3];
            part.px[row * pw * 3..(row + 1) * pw * 3].copy_from_slice(from);
        }
        for (j, [ox, oy, ow, oh]) in asked.iter().chain(&rest).enumerate() {
            if j == k {
                continue;
            }
            let (px, py) = (ow * OTHER_PAD, oh * OTHER_PAD);
            let (ax, ay) = ((((ox - px) as i64) - x0).max(0), (((oy - py) as i64) - y0).max(0));
            let (bx, by) = (x1.min((ox + ow + px).ceil() as i64) - x0, y1.min((oy + oh + py).ceil() as i64) - y0);
            if bx > ax && by > ay {
                for row in ay as usize..by as usize {
                    part.px[(row * pw + ax as usize) * 3..(row * pw + bx as usize) * 3].fill(128);
                }
            }
        }
        // this face itself stays, even where boxes overlap
        let (fx0, fy0) = (((*x as i64) - x0).max(0), ((*y as i64) - y0).max(0));
        let (fx1, fy1) = (x1.min((x + w).ceil() as i64) - x0, y1.min((y + h).ceil() as i64) - y0);
        if fx1 > fx0 {
            for row in fy0..fy1 {
                let from = &frame.row((y0 + row) as usize)[(x0 + fx0) as usize * 3..(x0 + fx1) as usize * 3];
                part.px[(row as usize * pw + fx0 as usize) * 3..(row as usize * pw + fx1 as usize) * 3]
                    .copy_from_slice(from);
            }
        }
        let s = (CAPTURE_MIN / pw.max(ph) as f64).max(1.0);
        if s > 1.0 {
            part = part.resize_cubic(s, s);
        }
        let verdict = json!({
            "box": [rounded((x - x0 as f64) * s, 1), rounded((y - y0 as f64) * s, 1), rounded(w * s, 1), rounded(h * s, 1)],
            "name": name_of(face),
            "sim": sim_of(face)?,
        });
        out.push((k, part, verdict));
    }
    Ok(out)
}

pub struct Capture {
    folder: PathBuf,
}

impl Capture {
    pub fn new(home: &Path) -> Capture {
        Capture { folder: home.join("webcam") }
    }

    /// Save each face the page asks for of this frame. The saved names, without their endings.
    pub fn save(&self, jpeg: &[u8], faces: &Value, others: &Value) -> Result<Vec<String>, String> {
        let frame = Bgr::from_jpeg(jpeg).map_err(|_| "not a JPEG image".to_string())?;
        let parts = cut(&frame, faces, others)?;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        let at = Utc.timestamp_opt(now.as_secs() as i64, 0).single().ok_or("the clock gives no time")?;
        let when = at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let base = format!("webcam-{}-{:03}", at.format("%Y%m%d-%H%M%S"), now.subsec_millis());
        std::fs::create_dir_all(&self.folder).map_err(|e| format!("can't make the webcam folder: {e}"))?;
        let mut stems = Vec::new();
        for (k, part, verdict) in parts {
            let stem = format!("{base}-{k}");
            let about = filejson::dumps(&json!({"when": when, "faces": [verdict]}));
            std::fs::write(self.folder.join(format!("{stem}.json")), about)
                .map_err(|e| format!("can't write {stem}.json: {e}"))?;
            // last, and whole: the labeling tool never sees a picture without its verdict, or half of one
            let picture = part.to_jpeg(QUALITY).map_err(|e| e.0)?;
            atomic_write(&self.folder.join(format!("{stem}.jpg")), &picture)
                .map_err(|e| format!("can't write {stem}.jpg: {e}"))?;
            stems.push(stem);
        }
        Ok(stems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame whose every pixel says where it is.
    fn frame(w: usize, h: usize) -> Bgr {
        let mut f = Bgr::new(w, h);
        for y in 0..h {
            for x in 0..w {
                f.px[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[(x % 251) as u8, (y % 251) as u8, 200]);
            }
        }
        f
    }

    #[test]
    fn a_face_is_cut_with_room_around_it_and_the_others_greyed_out() {
        let f = frame(640, 480);
        let faces = json!([{"box": [200.4, 150.2, 100.0, 120.0], "name": "Zoë", "sim": 0.61234},
                           {"box": [330.0, 160.0, 90.0, 110.0], "name": "Unknown", "sim": 0}]);
        let got = cut(&f, &faces, &json!([[150.0, 300.0, 60.0, 60.0]])).unwrap();
        assert_eq!(got.len(), 2);
        let (k, part, verdict) = &got[0];
        // 0.6 of the larger side on every side: from (128, 78) to (373, 343)
        assert_eq!((*k, part.w, part.h), (0, 245, 265));
        assert_eq!(*verdict, json!({"box": [72.4, 72.2, 100.0, 120.0], "name": "Zoë", "sim": 0.612}));
        let at = |x: usize, y: usize| &part.px[(y * part.w + x) * 3..(y * part.w + x) * 3 + 3];
        assert_eq!(at(0, 0), [128, 78, 200], "the frame's own pixels at the picture's corner");
        assert_eq!(at(100, 100), [228, 178, 200], "the face itself");
        assert_eq!(at(240, 150), [128, 128, 128], "the other face asked for, greyed out");
        assert_eq!(at(40, 240), [128, 128, 128], "someone else in view, greyed out");
        // a face smaller than the least size is enlarged, and its box with it
        let small = cut(&f, &json!([[300.0, 200.0, 40.0, 50.0]]), &json!([])).unwrap();
        let (_, part, verdict) = &small[0];
        assert_eq!((part.w, part.h), (145, 160));
        assert_eq!(verdict["box"], json!([43.6, 43.6, 58.2, 72.7]));
        assert_eq!((verdict["name"].clone(), verdict["sim"].clone()), (json!("Unknown"), json!(0.0)));
        // a face at the frame's edge keeps what there is (140 by 150 here, enlarged to the least size); one
        // outside the frame is left out
        let edge = cut(&f, &json!([[-20.0, -10.0, 100.0, 100.0], [900.0, 900.0, 50.0, 50.0]]), &json!([])).unwrap();
        assert_eq!((edge.len(), edge[0].1.w, edge[0].1.h), (1, 149, 160));
    }

    #[test]
    fn what_is_not_a_list_of_faces_is_refused() {
        let f = frame(64, 48);
        for (faces, others, why) in [
            (json!([]), json!([]), "expected 1-20 faces"),
            (json!("faces"), json!([]), "expected at most 20 faces"),
            (json!(vec![json!([1, 1, 5, 5]); 21]), json!([]), "expected at most 20 faces"),
            (json!([[1, 1, 5]]), json!([]), "each of the faces needs a box [x, y, w, h]"),
            (json!([[1, 1, 0, 5]]), json!([]), "each of the faces needs a box [x, y, w, h]"),
            (json!([{"box": [1, 1, true, 5]}]), json!([]), "each of the faces needs a box [x, y, w, h]"),
            (json!([{"name": "A"}]), json!([]), "each of the faces needs a box [x, y, w, h]"),
            (json!([[1, 1, 5, 5]]), json!([["a", 1, 5, 5]]), "each of the other faces needs a box [x, y, w, h]"),
            (json!([{"box": [1, 1, 30, 30], "sim": "high"}]), json!([]), "could not convert string to float: 'high'"),
        ] {
            assert_eq!(cut(&f, &faces, &others).map(|v| v.len()).unwrap_err(), why);
        }
    }

    #[test]
    fn a_collected_face_is_two_files() {
        let dir = std::env::temp_dir().join(format!("face-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let c = Capture::new(&dir);
        assert_eq!(c.save(b"not a jpeg", &json!([[1, 1, 5, 5]]), &json!([])).unwrap_err(), "not a JPEG image");
        let jpeg = frame(320, 240).to_jpeg(90).unwrap();
        let stems = c.save(&jpeg, &json!([{"box": [100, 60, 80, 90], "name": "A", "sim": 0.5}]), &json!([])).unwrap();
        assert_eq!(stems.len(), 1);
        let stem = &stems[0];
        assert!(
            stem.starts_with("webcam-20") && stem.ends_with("-0") && stem.len() == "webcam-20261120-080500-123-0".len(),
            "{stem}"
        );
        let about: Value =
            serde_json::from_slice(&std::fs::read(dir.join("webcam").join(format!("{stem}.json"))).unwrap()).unwrap();
        assert_eq!(about["faces"], json!([{"box": [54.0, 54.0, 80.0, 90.0], "name": "A", "sim": 0.5}]));
        assert_eq!(about["when"].as_str().unwrap().len(), "2026-11-20T08:05:00Z".len());
        let saved = Bgr::from_jpeg(&std::fs::read(dir.join("webcam").join(format!("{stem}.jpg"))).unwrap()).unwrap();
        assert_eq!((saved.w, saved.h), (188, 198));
        let _ = std::fs::remove_dir_all(dir);
    }
}
