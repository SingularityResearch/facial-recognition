//! Where a face's five points go: the similarity transform that lays them on the 112x112 template the
//! recognizers were trained with. This is `cv2.estimateAffinePartial2D(src, template, method=cv2.LMEDS)` as
//! OpenCV 5.0.0 does it (modules/geometry/src/ptsetreg.cpp), its random number generator included: which
//! points count decides the crop, the crop decides the print, and prints are shared between devices.
//!
//! One step is not followed to the letter. OpenCV refines the transform over the points that count with a
//! Levenberg-Marquardt loop (levmarq.cpp, geodesic acceleration on). The problem is linear, so that loop
//! heads for the least-squares answer, but it stops within a millionth of the remaining error and adds a
//! correction term that is rounding noise divided by 1e-8. Its result is the least-squares answer give or
//! take 3e-5 (measured on 1,200 made-up faces; writing the loop out step for step left the same gap, since the noise is
//! the arithmetic's own). Here the least-squares answer is computed outright: `tests/parity.rs` holds it to
//! OpenCV's within 1e-4, which moves a crop by a hundredth of a pixel at most.

/// eyes, nose, mouth corners on a 112x112 aligned face (the template face recognizers are trained with)
pub const ARC_TEMPLATE: [[f32; 2]; 5] =
    [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

/// OpenCV's `cv::RNG`: multiply-with-carry, the state it starts every estimate with.
struct CvRng(u64);

impl CvRng {
    fn new() -> CvRng {
        CvRng(u64::MAX) // RNG((uint64)-1)
    }
    fn next(&mut self) -> u32 {
        self.0 = (self.0 as u32 as u64).wrapping_mul(4164903690).wrapping_add(self.0 >> 32);
        self.0 as u32
    }
    fn uniform(&mut self, a: i32, b: i32) -> i32 {
        if a == b { a } else { (self.next() % (b - a) as u32) as i32 + a }
    }
}

fn update_num_iters(p: f64, ep: f64, model_points: i32, max_iters: i32) -> i32 {
    let p = p.clamp(0.0, 1.0);
    let ep = ep.clamp(0.0, 1.0);
    let num = (1.0 - p).max(f64::MIN_POSITIVE);
    let denom = 1.0 - (1.0 - ep).powi(model_points);
    if denom < f64::MIN_POSITIVE {
        return 0;
    }
    let (num, denom) = (num.ln(), denom.ln());
    if denom >= 0.0 || -num >= max_iters as f64 * (-denom) { max_iters } else { (num / denom).round_ties_even() as i32 }
}

/// The transform through two point pairs exactly (rotation, one scale, shift), or None for one point twice.
fn two_point_model(from: &[[f32; 2]; 2], to: &[[f32; 2]; 2]) -> Option<[f64; 6]> {
    let (x1, y1, x2, y2) = (from[0][0] as f64, from[0][1] as f64, from[1][0] as f64, from[1][1] as f64);
    let (bx1, by1, bx2, by2) = (to[0][0] as f64, to[0][1] as f64, to[1][0] as f64, to[1][1] as f64);
    let sq = (x1 - x2) * (x1 - x2) + (y1 - y2) * (y1 - y2);
    if sq == 0.0 {
        return None;
    }
    let d = 1.0 / sq;
    let s0 = d * ((bx1 - bx2) * (x1 - x2) + (by1 - by2) * (y1 - y2));
    let s1 = d * ((by1 - by2) * (x1 - x2) - (bx1 - bx2) * (y1 - y2));
    let s2 =
        d * ((by1 - by2) * (x1 * y2 - x2 * y1) - (bx1 * y2 - bx2 * y1) * (y1 - y2) - (bx1 * x2 - bx2 * x1) * (x1 - x2));
    let s3 = d
        * (-(bx1 - bx2) * (x1 * y2 - x2 * y1) - (by1 * x2 - by2 * x1) * (x1 - x2) - (by1 * y2 - by2 * y1) * (y1 - y2));
    Some([s0, -s1, s2, s1, s0, s3])
}

/// Each point's squared miss under a model, in single precision as OpenCV has it.
fn errors(from: &[[f32; 2]], to: &[[f32; 2]], m: &[f64; 6]) -> Vec<f32> {
    let f: [f32; 6] = [m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32, m[4] as f32, m[5] as f32];
    from.iter()
        .zip(to)
        .map(|(p, t)| {
            let a = f[0] * p[0] + f[1] * p[1] + f[2] - t[0];
            let b = f[3] * p[0] + f[4] * p[1] + f[5] - t[1];
            a * a + b * b
        })
        .collect()
}

/// The least-squares similarity through the given pairs: what OpenCV's refinement heads for.
fn least_squares(from: &[[f32; 2]], to: &[[f32; 2]]) -> Option<[f64; 6]> {
    let n = from.len() as f64;
    let (mut mx, mut my, mut tx, mut ty) = (0.0, 0.0, 0.0, 0.0);
    for (p, t) in from.iter().zip(to) {
        mx += p[0] as f64;
        my += p[1] as f64;
        tx += t[0] as f64;
        ty += t[1] as f64;
    }
    let (mx, my, tx, ty) = (mx / n, my / n, tx / n, ty / n);
    let (mut saa, mut sab, mut sp) = (0.0, 0.0, 0.0);
    for (p, t) in from.iter().zip(to) {
        let (x, y, u, v) = (p[0] as f64 - mx, p[1] as f64 - my, t[0] as f64 - tx, t[1] as f64 - ty);
        saa += x * u + y * v;
        sab += x * v - y * u;
        sp += x * x + y * y;
    }
    if sp == 0.0 {
        return None;
    }
    let (a, b) = (saa / sp, sab / sp);
    Some([a, -b, tx - a * mx + b * my, b, a, ty - b * mx - a * my])
}

/// `cv2.estimateAffinePartial2D(from, to, method=cv2.LMEDS)[0]`: the 2x3 matrix, row by row.
pub fn estimate_similarity(from: &[[f32; 2]], to: &[[f32; 2]]) -> Option<[f64; 6]> {
    const MODEL_POINTS: usize = 2;
    let count = from.len();
    if count != to.len() || count < MODEL_POINTS {
        return None;
    }
    if count == MODEL_POINTS {
        return two_point_model(&[from[0], from[1]], &[to[0], to[1]]);
    }
    // least median of squares: the pair of points whose transform leaves the smallest median miss
    let mut rng = CvRng::new();
    let niters = update_num_iters(0.99, 0.45, MODEL_POINTS as i32, 2000).max(3);
    let (mut best, mut min_median) = (None, f64::MAX);
    for _ in 0..niters {
        let mut idx = [0usize; MODEL_POINTS];
        for i in 0..MODEL_POINTS {
            let mut k = rng.uniform(0, count as i32) as usize;
            while idx[..i].contains(&k) {
                k = rng.uniform(0, count as i32) as usize;
            }
            idx[i] = k;
        }
        let Some(model) = two_point_model(&[from[idx[0]], from[idx[1]]], &[to[idx[0]], to[idx[1]]]) else {
            continue;
        };
        let mut err = errors(from, to, &model);
        err.sort_by(|a, b| a.total_cmp(b));
        let median = err[count / 2] as f64;
        if median < min_median {
            min_median = median;
            best = Some(model);
        }
    }
    let best = best?;
    // the points that count: those within the spread the best pair's median implies
    let sigma = (2.5 * 1.4826 * (1.0 + 5.0 / (count - MODEL_POINTS) as f64) * min_median.sqrt()).max(0.001);
    let t = (sigma * sigma) as f32;
    let keep: Vec<bool> = errors(from, to, &best).iter().map(|&e| e <= t).collect();
    if keep.iter().filter(|&&k| k).count() < MODEL_POINTS {
        return None;
    }
    let (f, g): (Vec<[f32; 2]>, Vec<[f32; 2]>) =
        from.iter().zip(to).zip(&keep).filter(|(_, k)| **k).map(|((p, q), _)| (*p, *q)).unzip();
    Some(least_squares(&f, &g).unwrap_or(best))
}

/// The matrix that lays a detected face (its five points) on the template.
pub fn align_matrix(kps: &[[f32; 2]; 5]) -> Option<[f64; 6]> {
    estimate_similarity(kps, &ARC_TEMPLATE)
}

/// Overlap of two boxes (x, y, w, h): intersection over union.
pub fn iou(a: &[f32], b: &[f32]) -> f32 {
    let (x1, y1) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x2, y2) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    inter / (a[2] * a[3] + b[2] * b[3] - inter + 1e-9)
}

/// How far a face is turned left or right: the nose tip's sideways offset from the line through the eyes'
/// midpoint and the mouth's, as a share of the distance between the eyes. About 0 facing the camera, 0.3 at
/// a 30 degree turn, 0.5 at 45.
pub fn head_turn(row: &[f32; 15]) -> f64 {
    let k = |i: usize| (row[4 + 2 * i] as f64, row[5 + 2 * i] as f64);
    let (e0, e1, nose, m0, m1) = (k(0), k(1), k(2), k(3), k(4));
    let eyes_x = (e0.0 + e1.0) / 2.0;
    let mouth_x = (m0.0 + m1.0) / 2.0;
    let eye_dist = (e1.0 - e0.0).hypot(e1.1 - e0.1) + 1e-6;
    (nose.0 - (eyes_x + mouth_x) / 2.0).abs() / eye_dist
}
