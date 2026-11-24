//! Pictures as the models take them: 8-bit BGR, rows top-down, three bytes a pixel (OpenCV's layout, which
//! the measured thresholds were found with). The resize and the warp follow OpenCV's arithmetic step for
//! step, so a face cut out here is the face OpenCV cuts out: `tests/parity.rs` holds it to
//! numbers OpenCV itself produced.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bgr {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// OpenCV's `cvRound`: halves go to the even neighbour.
#[inline]
pub fn round_even(v: f64) -> i64 {
    v.round_ties_even() as i64
}

/// A float as a byte: clamped to 0..255, halves to the even neighbour (OpenCV's saturate_cast). Adding and
/// taking away 2^23 rounds exactly that way for these values, in two additions: the library's own
/// round-to-even is a function call per byte on a processor target without SSE4.1 (691,000 calls for one
/// camera frame: 2.8 of the shrink's 3.4 ms).
#[inline]
fn saturate_u8(v: f32) -> u8 {
    const BIG: f32 = 8_388_608.0;
    let v = v.max(0.0).min(255.0);
    ((v + BIG) - BIG) as u8
}

impl Bgr {
    pub fn new(w: usize, h: usize) -> Bgr {
        Bgr { w, h, px: vec![0; w * h * 3] }
    }

    pub fn from_px(w: usize, h: usize, px: Vec<u8>) -> Result<Bgr> {
        if px.len() != w * h * 3 {
            return Err(Error::msg("picture bytes don't fit its size"));
        }
        Ok(Bgr { w, h, px })
    }

    /// A JPEG decoded straight to BGR.
    pub fn from_jpeg(data: &[u8]) -> Result<Bgr> {
        use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
        let opts = DecoderOptions::new_fast().jpeg_set_out_colorspace(ColorSpace::BGR);
        let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), opts);
        let px = dec.decode().map_err(|_| Error::msg("not a JPEG image"))?;
        let info = dec.info().ok_or_else(|| Error::msg("not a JPEG image"))?;
        Bgr::from_px(info.width as usize, info.height as usize, px)
    }

    #[inline]
    pub fn row(&self, y: usize) -> &[u8] {
        &self.px[y * self.w * 3..(y + 1) * self.w * 3]
    }

    /// Mean of every byte (a face crop's brightness).
    pub fn mean(&self) -> f64 {
        if self.px.is_empty() {
            return 0.0;
        }
        self.px.iter().map(|&v| v as u64).sum::<u64>() as f64 / self.px.len() as f64
    }

    /// Shrunk to `dw` x `dh` as `cv2.resize(..., interpolation=cv2.INTER_AREA)` does it: each new pixel is the
    /// average of the old pixels it covers, with partly covered ones counted by their share.
    pub fn resize_area(&self, dw: usize, dh: usize) -> Bgr {
        if dw == self.w && dh == self.h {
            return self.clone();
        }
        let scale_x = 1.0 / (dw as f64 / self.w as f64);
        let scale_y = 1.0 / (dh as f64 / self.h as f64);
        let (ix, iy) = (round_even(scale_x) as usize, round_even(scale_y) as usize);
        let whole = (scale_x - ix as f64).abs() < f64::EPSILON && (scale_y - iy as f64).abs() < f64::EPSILON;
        if whole && ix >= 1 && iy >= 1 {
            return self.shrink_whole(dw, dh, ix, iy);
        }
        // OpenCV's sums, in its order and in single precision: along x into a row of floats (each new pixel
        // from its old ones, left to right), then that row times its share added down the column. A row of
        // the old picture is turned to floats once, and each new pixel knows where its old ones start.
        let xtab = area_tab(self.w, dw, scale_x);
        let ytab = area_tab(self.h, dh, scale_y);
        let mut first = vec![0usize; dw + 1]; // new pixel -> its first entry of xtab
        for x in &xtab {
            first[x.di + 1] += 1;
        }
        for d in 0..dw {
            first[d + 1] += first[d];
        }
        let mut out = Bgr::new(dw, dh);
        let n = dw * 3;
        let mut wide = vec![[0f32; 3]; self.w];
        let (mut buf, mut sum) = (vec![[0f32; 3]; dw], vec![[0f32; 3]; dw]);
        let mut have = usize::MAX; // the old row `buf` holds: two new rows can share one
        let mut at = 0;
        while at < ytab.len() {
            let dy = ytab[at].di;
            sum.fill([0.0; 3]);
            while at < ytab.len() && ytab[at].di == dy {
                let t = &ytab[at];
                if t.si != have {
                    for (f, b) in wide.iter_mut().zip(self.row(t.si).chunks_exact(3)) {
                        *f = [b[0] as f32, b[1] as f32, b[2] as f32];
                    }
                    for (d, px) in buf.iter_mut().enumerate() {
                        let mut acc = [0f32; 3];
                        for x in &xtab[first[d]..first[d + 1]] {
                            let s = wide[x.si];
                            acc[0] += s[0] * x.alpha;
                            acc[1] += s[1] * x.alpha;
                            acc[2] += s[2] * x.alpha;
                        }
                        *px = acc;
                    }
                    have = t.si;
                }
                for (s, v) in sum.iter_mut().zip(&buf) {
                    s[0] += t.alpha * v[0];
                    s[1] += t.alpha * v[1];
                    s[2] += t.alpha * v[2];
                }
                at += 1;
            }
            for (o, s) in out.px[dy * n..(dy + 1) * n].chunks_exact_mut(3).zip(&sum) {
                o[0] = saturate_u8(s[0]);
                o[1] = saturate_u8(s[1]);
                o[2] = saturate_u8(s[2]);
            }
        }
        out
    }

    /// A whole-number shrink: plain block averages (OpenCV's "area fast" path; 2x2 rounds halves up, the
    /// others go through a float and round halves to even).
    fn shrink_whole(&self, dw: usize, dh: usize, ix: usize, iy: usize) -> Bgr {
        let mut out = Bgr::new(dw, dh);
        let scale = 1.0f32 / (ix * iy) as f32;
        for y in 0..dh {
            for x in 0..dw {
                for c in 0..3 {
                    let mut sum = 0i32;
                    for yy in 0..iy {
                        let row = self.row(y * iy + yy);
                        for xx in 0..ix {
                            sum += row[(x * ix + xx) * 3 + c] as i32;
                        }
                    }
                    out.px[(y * dw + x) * 3 + c] =
                        if ix == 2 && iy == 2 { ((sum + 2) >> 2) as u8 } else { saturate_u8(sum as f32 * scale) };
                }
            }
        }
        out
    }

    /// `cv2.warpAffine(self, m, (dw, dh), borderValue=0)` with its default bilinear sampling, as OpenCV 5
    /// computes it (found by trying each order of operations against it: `tests/parity.rs` holds the bytes).
    /// `m` is the 2x3 matrix (row by row) that takes a point of this picture to a point of the new one.
    /// Positions are single-precision: a row's start `y*b + c`, then `x*a` added with one rounding (a fused
    /// multiply-add, done here through a double so every processor gives the same answer at the same speed);
    /// the four neighbours are blended along x, then along y, and halves round to even.
    pub fn warp_affine(&self, m: &[f64; 6], dw: usize, dh: usize) -> Bgr {
        // the matrix the other way round: where in this picture each new pixel comes from
        let mut q = *m;
        let det = q[0] * q[4] - q[1] * q[3];
        let d = if det != 0.0 { 1.0 / det } else { 0.0 };
        let (a11, a22) = (q[4] * d, q[0] * d);
        q[0] = a11;
        q[1] *= -d;
        q[3] *= -d;
        q[4] = a22;
        let (b1, b2) = (-q[0] * q[2] - q[1] * q[5], -q[3] * q[2] - q[4] * q[5]);
        q[2] = b1;
        q[5] = b2;
        let q: [f32; 6] = q.map(|v| v as f32);
        let fused = |x: f32, a: f32, c: f32| (x as f64 * a as f64 + c as f64) as f32;
        let (w, h) = (self.w as i64, self.h as i64);
        let mut out = Bgr::new(dw, dh);
        for y in 0..dh {
            let (row_x, row_y) = (y as f32 * q[1] + q[2], y as f32 * q[4] + q[5]);
            let dst = &mut out.px[y * dw * 3..(y + 1) * dw * 3];
            for x in 0..dw {
                let (sx, sy) = (fused(x as f32, q[0], row_x), fused(x as f32, q[3], row_y));
                let (fx, fy) = (sx.floor(), sy.floor());
                if !(fx >= -1.0 && fx < w as f32 && fy >= -1.0 && fy < h as f32) {
                    continue; // all four neighbours outside (or no number): the border's black
                }
                let (ix, iy, ax, ay) = (fx as i64, fy as i64, sx - fx, sy - fy);
                let at = |xx: i64, yy: i64| -> [f32; 3] {
                    if xx < 0 || xx >= w || yy < 0 || yy >= h {
                        [0.0; 3]
                    } else {
                        let p = &self.px[(yy as usize * self.w + xx as usize) * 3..][..3];
                        [p[0] as f32, p[1] as f32, p[2] as f32]
                    }
                };
                let (p00, p01, p10, p11) = (at(ix, iy), at(ix + 1, iy), at(ix, iy + 1), at(ix + 1, iy + 1));
                for c in 0..3 {
                    let top = p00[c] + ax * (p01[c] - p00[c]);
                    let bottom = p10[c] + ax * (p11[c] - p10[c]);
                    dst[x * 3 + c] = saturate_u8(top + ay * (bottom - top));
                }
            }
        }
        out
    }

    /// Enlarged (or shrunk) by `fx` across and `fy` down with the bicubic filter, as OpenCV's
    /// `resize(..., fx, fy, INTER_CUBIC)` does it: its size (each side rounded, halves to even), its place of
    /// every new pixel in the old picture, its four weights a side (the cubic with a = -0.75, kept as
    /// 11-bit whole numbers), the edge pixels repeated. OpenCV's faster paths round a rare sum the other
    /// way, so this is its picture to within one level here and there, not held byte for byte.
    pub fn resize_cubic(&self, fx: f64, fy: f64) -> Bgr {
        const BITS: i64 = 11;
        let (dw, dh) = (round_even(self.w as f64 * fx).max(1) as usize, round_even(self.h as f64 * fy).max(1) as usize);
        // for each new pixel along one side: the first of its four old pixels, and their weights
        let taps = |dst: usize, src: usize, scale: f64| -> Vec<(i64, [i64; 4])> {
            (0..dst)
                .map(|d| {
                    let at = ((d as f64 + 0.5) * scale - 0.5) as f32;
                    let first = at.floor();
                    let f = at - first;
                    const A: f32 = -0.75;
                    let c0 = ((A * (f + 1.0) - 5.0 * A) * (f + 1.0) + 8.0 * A) * (f + 1.0) - 4.0 * A;
                    let c1 = ((A + 2.0) * f - (A + 3.0)) * f * f + 1.0;
                    let c2 = ((A + 2.0) * (1.0 - f) - (A + 3.0)) * (1.0 - f) * (1.0 - f) + 1.0;
                    let c = [c0, c1, c2, 1.0 - c0 - c1 - c2];
                    let whole = c.map(|v| round_even((v * (1 << BITS) as f32) as f64).clamp(-32768, 32767));
                    let _ = src;
                    (first as i64 - 1, whole)
                })
                .collect()
        };
        let (xs, ys) = (taps(dw, self.w, 1.0 / fx), taps(dh, self.h, 1.0 / fy));
        let clamp = |v: i64, n: usize| v.clamp(0, n as i64 - 1) as usize;
        // across first: every old row at the new width, as whole numbers 2^11 times the value
        let mut wide = vec![0i64; self.h * dw * 3];
        for y in 0..self.h {
            let row = self.row(y);
            for (x, (first, w)) in xs.iter().enumerate() {
                for c in 0..3 {
                    let mut sum = 0;
                    for (k, weight) in w.iter().enumerate() {
                        sum += row[clamp(first + k as i64, self.w) * 3 + c] as i64 * weight;
                    }
                    wide[(y * dw + x) * 3 + c] = sum;
                }
            }
        }
        let mut out = Bgr::new(dw, dh);
        for (y, (first, w)) in ys.iter().enumerate() {
            let rows: [usize; 4] = std::array::from_fn(|k| clamp(first + k as i64, self.h) * dw * 3);
            for i in 0..dw * 3 {
                let sum: i64 = (0..4).map(|k| wide[rows[k] + i] * w[k]).sum();
                out.px[y * dw * 3 + i] = ((sum + (1 << (2 * BITS - 1))) >> (2 * BITS)).clamp(0, 255) as u8;
            }
        }
        out
    }

    /// Brought to `dw` by `dh` with the bicubic filter worked in floating point: what OpenCV's
    /// `resize(..., (dw, dh), INTER_CUBIC)` gives on an x86 computer, where Intel's library does that work
    /// (its weights are not cut to 11 bits, so a quarter of a percent of the bytes and more are one level
    /// from OpenCV's own arithmetic). Measured against it on made-up pictures: 24 bytes in 7 million are
    /// one level off, each where the sum lies within a hair of a half and the library's own order of
    /// adding decides; no order tried gave them all. Across first, each sum built up one product at a
    /// time; then down, the four products added at double width.
    pub fn resize_cubic_float(&self, dw: usize, dh: usize) -> Bgr {
        let taps = |dst: usize, src: usize| -> Vec<(i64, [f32; 4])> {
            let scale = 1.0 / (dst as f64 / src as f64);
            (0..dst)
                .map(|d| {
                    let at = (d as f64 + 0.5) * scale - 0.5;
                    let first = at.floor();
                    let f = at - first;
                    const A: f64 = -0.75;
                    let c0 = ((A * (f + 1.0) - 5.0 * A) * (f + 1.0) + 8.0 * A) * (f + 1.0) - 4.0 * A;
                    let c1 = ((A + 2.0) * f - (A + 3.0)) * f * f + 1.0;
                    let c2 = ((A + 2.0) * (1.0 - f) - (A + 3.0)) * (1.0 - f) * (1.0 - f) + 1.0;
                    (first as i64 - 1, [c0 as f32, c1 as f32, c2 as f32, (1.0 - c0 - c1 - c2) as f32])
                })
                .collect()
        };
        let (xs, ys) = (taps(dw, self.w), taps(dh, self.h));
        let clamp = |v: i64, n: usize| v.clamp(0, n as i64 - 1) as usize;
        let mut wide = vec![0f32; self.h * dw * 3];
        for y in 0..self.h {
            let row = self.row(y);
            for (x, (first, w)) in xs.iter().enumerate() {
                for c in 0..3 {
                    let s = |k: usize| row[clamp(first + k as i64, self.w) * 3 + c] as f32;
                    let mut sum = s(0) * w[0];
                    for (k, weight) in w.iter().enumerate().skip(1) {
                        sum = (sum as f64 + s(k) as f64 * *weight as f64) as f32;
                    }
                    wide[(y * dw + x) * 3 + c] = sum;
                }
            }
        }
        let mut out = Bgr::new(dw, dh);
        for (y, (first, w)) in ys.iter().enumerate() {
            let rows: [usize; 4] = std::array::from_fn(|k| clamp(first + k as i64, self.h) * dw * 3);
            for i in 0..dw * 3 {
                let sum = (0..4).fold(0f64, |sum, k| sum + wide[rows[k] + i] as f64 * w[k] as f64) as f32;
                out.px[y * dw * 3 + i] = (sum.round_ties_even() as i64).clamp(0, 255) as u8;
            }
        }
        out
    }

    /// The picture as a JPEG of this quality (1 to 100), its colour at half the resolution each way, as
    /// OpenCV's `imencode(".jpg")` writes one. Another encoder than OpenCV's: the same picture, not the same
    /// bytes. The colour is made and halved here, the way libjpeg does both (each 2 by 2 block's mean): left
    /// to the encoder, colour edges came out 2.6 dB further from the picture than OpenCV's at the same
    /// quality, whatever the quality (measured; grey pictures were the same either way).
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>> {
        let (Ok(w), Ok(h)) = (u16::try_from(self.w), u16::try_from(self.h)) else {
            return Err(Error::msg("the picture is too large for a JPEG"));
        };
        let level = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        let mut ycc = vec![0u8; self.px.len()];
        for (to, from) in ycc.chunks_exact_mut(3).zip(self.px.chunks_exact(3)) {
            let (b, g, r) = (from[0] as f32, from[1] as f32, from[2] as f32);
            to[0] = level(0.299 * r + 0.587 * g + 0.114 * b);
            to[1] = level(128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b);
            to[2] = level(128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b);
        }
        for by in (0..self.h).step_by(2) {
            for bx in (0..self.w).step_by(2) {
                let (xs, ys) = (bx..(bx + 2).min(self.w), by..(by + 2).min(self.h));
                for c in 1..3 {
                    let at = |x: usize, y: usize| (y * self.w + x) * 3 + c;
                    let (mut sum, mut n) = (0u32, 0u32);
                    for y in ys.clone() {
                        for x in xs.clone() {
                            sum += ycc[at(x, y)] as u32;
                            n += 1;
                        }
                    }
                    let mean = ((sum + n / 2) / n) as u8;
                    for y in ys.clone() {
                        for x in xs.clone() {
                            ycc[at(x, y)] = mean;
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
        encoder
            .encode(&ycc, w, h, jpeg_encoder::ColorType::Ycbcr)
            .map_err(|e| Error::msg(format!("the picture couldn't be written as a JPEG: {e}")))?;
        Ok(out)
    }

    /// Mirrored left to right.
    pub fn flipped(&self) -> Bgr {
        let mut out = Bgr::new(self.w, self.h);
        for y in 0..self.h {
            let (src, dst) = (self.row(y), &mut out.px[y * self.w * 3..(y + 1) * self.w * 3]);
            for x in 0..self.w {
                let s = (self.w - 1 - x) * 3;
                dst[x * 3..x * 3 + 3].copy_from_slice(&src[s..s + 3]);
            }
        }
        out
    }
}

struct Share {
    di: usize,
    si: usize,
    alpha: f32,
}

/// Which old pixels each new pixel covers along one axis, and by how much (OpenCV's computeResizeAreaTab).
fn area_tab(ssize: usize, dsize: usize, scale: f64) -> Vec<Share> {
    let mut tab = Vec::with_capacity(ssize * 2);
    for dx in 0..dsize {
        let fsx1 = dx as f64 * scale;
        let fsx2 = fsx1 + scale;
        let cell = scale.min(ssize as f64 - fsx1);
        let mut sx1 = fsx1.ceil() as i64;
        let mut sx2 = fsx2.floor() as i64;
        sx2 = sx2.min(ssize as i64 - 1);
        sx1 = sx1.min(sx2);
        if sx1 as f64 - fsx1 > 1e-3 {
            tab.push(Share { di: dx, si: (sx1 - 1) as usize, alpha: ((sx1 as f64 - fsx1) / cell) as f32 });
        }
        for sx in sx1..sx2 {
            tab.push(Share { di: dx, si: sx as usize, alpha: (1.0 / cell) as f32 });
        }
        if fsx2 - sx2 as f64 > 1e-3 {
            tab.push(Share { di: dx, si: sx2 as usize, alpha: ((fsx2 - sx2 as f64).min(1.0).min(cell) / cell) as f32 });
        }
    }
    tab
}
