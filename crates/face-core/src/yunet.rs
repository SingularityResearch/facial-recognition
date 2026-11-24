//! YuNet's faces from the model's raw outputs, decoded as OpenCV's FaceDetectorYN decodes them (score =
//! sqrt(class x objectness), boxes and points per grid cell at strides 8, 16 and 32, then non-maximum
//! suppression), so the faces found here are the faces OpenCV's detector finds.

use crate::image::Bgr;

/// A found face: x, y, w, h, five points (eyes, nose, mouth corners), score.
pub type Row = [f32; 15];

/// A photo is padded to a multiple of this each way before it goes to the model on a graphics processor: few
/// distinct input sizes, so a session per size stays affordable. A measured value.
pub const DET_PAD: usize = 320;
/// What the model itself needs, and what OpenCV's own detector pads to: the padding on the processor, where
/// there is no session per size to keep few (`Faces::pad`).
pub const MODEL_PAD: usize = 32;

/// The model's input for a picture: (height, width, values) with the picture's BGR bytes as floats, plane by
/// plane, and black to the right and below up to the next multiple of `pad` each way.
pub fn input(img: &Bgr, pad: usize) -> (usize, usize, Vec<f32>) {
    let ph = img.h.div_ceil(pad) * pad;
    let pw = img.w.div_ceil(pad) * pad;
    let mut x = vec![0f32; 3 * ph * pw];
    let plane = ph * pw;
    for y in 0..img.h {
        let row = img.row(y);
        let at = y * pw;
        for xx in 0..img.w {
            x[at + xx] = row[xx * 3] as f32;
            x[plane + at + xx] = row[xx * 3 + 1] as f32;
            x[2 * plane + at + xx] = row[xx * 3 + 2] as f32;
        }
    }
    (ph, pw, x)
}

/// The rows for a padded input `pw` wide from the model's twelve outputs (class, objectness, box and points,
/// each at strides 8, 16, 32), kept at `score` and above, after non-maximum suppression.
pub fn rows(out: &[Vec<f32>], pw: usize, score: f64, nms: f32, top_k: usize) -> Vec<Row> {
    let cut = (score * score) as f32; // sqrt(class x objectness) >= score
    let mut found: Vec<Row> = Vec::new();
    for (k, stride) in [8usize, 16, 32].into_iter().enumerate() {
        let (cls, obj, bbox, kps) = (&out[k], &out[3 + k], &out[6 + k], &out[9 + k]);
        let cols = pw / stride;
        for i in 0..cls.len() {
            let both = cls[i].clamp(0.0, 1.0) * obj[i].clamp(0.0, 1.0);
            if both < cut {
                continue;
            }
            let (r, c) = ((i / cols) as f64, (i % cols) as f64);
            let s = stride as f64;
            let b = &bbox[i * 4..i * 4 + 4];
            let (cx, cy) = ((c + b[0] as f64) * s, (r + b[1] as f64) * s);
            let (bw, bh) = (b[2].exp() * stride as f32, b[3].exp() * stride as f32);
            let mut row: Row = [0.0; 15];
            row[0] = (cx - (bw / 2.0) as f64) as f32;
            row[1] = (cy - (bh / 2.0) as f64) as f32;
            row[2] = bw;
            row[3] = bh;
            for p in 0..5 {
                row[4 + 2 * p] = ((kps[i * 10 + 2 * p] as f64 + c) * s) as f32;
                row[5 + 2 * p] = ((kps[i * 10 + 2 * p + 1] as f64 + r) * s) as f32;
            }
            row[14] = both.sqrt();
            found.push(row);
        }
    }
    suppress(found, score as f32, nms, top_k)
}

/// OpenCV's NMSBoxes: best score first, a box is kept unless it overlaps a kept one by more than `nms`.
fn suppress(rows: Vec<Row>, score: f32, nms: f32, top_k: usize) -> Vec<Row> {
    let mut order: Vec<usize> = (0..rows.len()).filter(|&i| rows[i][14] > score).collect();
    order.sort_by(|&a, &b| rows[b][14].total_cmp(&rows[a][14])); // stable: equal scores keep their order
    order.truncate(top_k);
    let mut kept: Vec<usize> = Vec::new();
    for i in order {
        if kept.iter().all(|&k| overlap(&rows[i], &rows[k]) <= nms as f64) {
            kept.push(i);
        }
    }
    kept.into_iter().map(|i| rows[i]).collect()
}

fn overlap(a: &Row, b: &Row) -> f64 {
    let (ax, ay, aw, ah) = (a[0] as f64, a[1] as f64, a[2] as f64, a[3] as f64);
    let (bx, by, bw, bh) = (b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64);
    let (area_a, area_b) = (aw * ah, bw * bh);
    if area_a + area_b <= f64::EPSILON {
        return 1.0;
    }
    let (x1, y1) = (ax.max(bx), ay.max(by));
    let (x2, y2) = ((ax + aw).min(bx + bw), (ay + ah).min(by + bh));
    let inter = if x2 > x1 && y2 > y1 { (x2 - x1) * (y2 - y1) } else { 0.0 };
    inter / (area_a + area_b - inter)
}
