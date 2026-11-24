//! Shrinking a picture: a bilinear resize in whole numbers, which is how a photo is
//! brought down to the size faces are looked for in. Where a face's box lies in the shrunk picture makes
//! its permanent ID, so the shrinking is fixed to the last bit: for each place of the smaller picture a
//! triangle of weights over the places it covers in the larger one (wider the more it shrinks), the weights as
//! whole numbers of 22 bits, one direction and then the other, each sum rounded once. Which direction comes
//! first is fixed too (the two don't give the same picture the other way round): across first,
//! unless the height shrinks by more than twice as many places as the width.

/// Weights are whole numbers with this many bits behind the point.
const BITS: u32 = 32 - 8 - 2;
const HALF: i32 = 1 << (BITS - 1);

/// For each place of the smaller side: where its weights begin in the larger side, and the weights.
struct Weights {
    first: Vec<usize>,
    /// `size` weights for each place, the unused ones zero
    k: Vec<i32>,
    size: usize,
}

fn weights(from: usize, to: usize) -> Weights {
    let scale = from as f64 / to as f64;
    let wider = scale.max(1.0);
    // the triangle reaches one place either way, as many times wider as the picture shrinks
    let support = wider;
    let size = support.ceil() as usize * 2 + 1;
    let narrower = 1.0 / wider;
    let (mut first, mut k) = (Vec::with_capacity(to), vec![0i32; to * size]);
    let mut row = vec![0f64; size];
    for xx in 0..to {
        let centre = (xx as f64 + 0.5) * scale;
        let lo = ((centre - support + 0.5) as i64).max(0) as usize;
        let hi = ((centre + support + 0.5) as i64).min(from as i64) as usize;
        let n = hi - lo;
        let mut sum = 0.0;
        for (x, w) in row.iter_mut().enumerate().take(n) {
            let at = ((x + lo) as f64 - centre + 0.5) * narrower;
            *w = (1.0 - at.abs()).max(0.0);
            sum += *w;
        }
        for (x, w) in row.iter().enumerate().take(n) {
            let w = if sum != 0.0 { w / sum } else { *w };
            k[xx * size + x] = if w < 0.0 {
                (-0.5 + w * (1u32 << BITS) as f64) as i32
            } else {
                (0.5 + w * (1u32 << BITS) as f64) as i32
            };
        }
        first.push(lo);
    }
    Weights { first, k, size }
}

#[inline]
fn clip(sum: i32) -> u8 {
    (sum >> BITS).clamp(0, 255) as u8
}

/// Every row made `to_w` wide.
fn across(px: &[u8], w: usize, h: usize, channels: usize, to_w: usize) -> Vec<u8> {
    let weights = weights(w, to_w);
    let mut out = vec![0u8; h * to_w * channels];
    for (line, to) in px.chunks_exact(w * channels).zip(out.chunks_exact_mut(to_w * channels)) {
        for xx in 0..to_w {
            let (lo, k) = (weights.first[xx], &weights.k[xx * weights.size..(xx + 1) * weights.size]);
            let n = weights.size.min(w - lo);
            for c in 0..channels {
                let mut sum = HALF;
                for (x, weight) in k.iter().enumerate().take(n) {
                    sum += line[(lo + x) * channels + c] as i32 * weight;
                }
                to[xx * channels + c] = clip(sum);
            }
        }
    }
    out
}

/// Every column made `to_h` high; `width` is a row's bytes.
fn down(px: &[u8], width: usize, h: usize, to_h: usize) -> Vec<u8> {
    let weights = weights(h, to_h);
    let mut out = vec![0u8; to_h * width];
    let mut sums = vec![0i32; width];
    for (yy, line) in out.chunks_exact_mut(width).enumerate() {
        let (lo, k) = (weights.first[yy], &weights.k[yy * weights.size..(yy + 1) * weights.size]);
        // a row of sums, a weighted row of the larger picture at a time
        sums.fill(HALF);
        for (y, weight) in k.iter().enumerate().take(weights.size.min(h - lo)) {
            if *weight != 0 {
                for (s, v) in sums.iter_mut().zip(&px[(lo + y) * width..(lo + y + 1) * width]) {
                    *s += *v as i32 * weight;
                }
            }
        }
        for (o, s) in line.iter_mut().zip(&sums) {
            *o = clip(*s);
        }
    }
    out
}

/// A picture of `channels` bytes a place, `w` x `h`, made `to_w` x `to_h` by that bilinear resize.
pub fn resize(px: &[u8], w: usize, h: usize, channels: usize, to_w: usize, to_h: usize) -> Vec<u8> {
    if (w, h) == (to_w, to_h) || w == 0 || h == 0 || to_w == 0 || to_h == 0 {
        return px.to_vec();
    }
    // across first, unless the height shrinks by more than twice as many places as the width
    let (less_h, less_w) = (h as i64 - to_h as i64, w as i64 - to_w as i64);
    let across_first = !(less_h > 0 && less_h > less_w * 2);
    match (w != to_w, h != to_h, across_first) {
        (true, false, _) => across(px, w, h, channels, to_w),
        (false, _, _) => down(px, w * channels, h, to_h),
        (true, true, true) => down(&across(px, w, h, channels, to_w), to_w * channels, h, to_h),
        (true, true, false) => across(&down(px, w * channels, h, to_h), w, to_h, channels, to_w),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_shrunk_to_its_fixed_numbers() {
        // a ramp across, shrunk to half (the numbers are fixed), and the same down
        let ramp: Vec<u8> = (0..8).map(|x| x * 10).collect();
        assert_eq!(resize(&ramp, 8, 1, 1, 4, 1), [7, 25, 45, 63]);
        assert_eq!(resize(&ramp, 1, 8, 1, 1, 4), [7, 25, 45, 63]);
        // every channel on its own
        let two: Vec<u8> = (0..8).flat_map(|x| [x * 10, 255 - x * 10, 7]).collect();
        assert_eq!(resize(&two, 8, 1, 3, 4, 1), [7, 248, 7, 25, 230, 7, 45, 210, 7, 63, 192, 7]);
        // one flat colour stays that colour, whatever the sizes and whichever direction comes first
        let flat = vec![200u8; 13 * 70];
        assert!(resize(&flat, 13, 70, 1, 5, 3).iter().all(|v| *v == 200));
        assert!(resize(&flat, 70, 13, 1, 3, 5).iter().all(|v| *v == 200));
        // nothing to do
        assert_eq!(resize(&ramp, 8, 1, 1, 8, 1), ramp);
    }

    #[test]
    fn the_direction_that_shrinks_far_more_comes_first() {
        // 4 x 12 to 3 x 4 (the height loses 8 places, the width 1): down first. The numbers are fixed
        let px: Vec<u8> = (0..48u32).map(|i| ((i * 37 + (i / 4) * 11) % 256) as u8).collect();
        let fixed = [104, 127, 104, 118, 109, 139, 140, 116, 112, 143, 134, 101];
        assert_eq!(resize(&px, 4, 12, 1, 3, 4), fixed);
        assert_eq!(across(&down(&px, 4, 12, 4), 4, 4, 1, 3), fixed);
    }
}
