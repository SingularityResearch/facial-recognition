//! "Looks like a real face": a score from 0 to 1 for the possible faces (those kept with the detector's low
//! cut-off), which says which of them get a dashed box in the photo view. It is learned from
//! the prints, real faces against possible faces nobody named, the two kinds weighted to balance, by an
//! L2-regularized logistic regression, solved as liblinear solves it (a trust-region Newton method,
//! from all zeros, the bias a feature of its own that is kept small like the others), so that every device
//! comes to the same model and not just to a model that is about as good.
//!
//! Each possible face is scored by a model trained without it (5 folds), so it isn't simply learned as junk;
//! the possible faces someone named are scored by the model of all. The cut-off is the score of the top
//! `FACE_SHOW` share: measured (on 160 hand-sorted possible faces the top 25% kept 17
//! of the 24 real faces and 21 of the other 136).

use std::collections::HashMap;

use crate::scores::{Prints, spread, threads};

/// share of the possible faces drawn as dashed boxes: the most face-like (labels.FACE_SHOW, a measured value)
pub const FACE_SHOW: f64 = 0.25;
/// how strongly the model is kept simple (the regression's C)
const C: f64 = 0.1;
/// how exactly it is solved (the solver's tolerance), and for how many steps at most
const TOL: f64 = 1e-4;
const MAX_ITER: usize = 5000;
/// too few of either kind to learn from
pub const FEW: usize = 50;
/// rows added up a block at a time, the blocks in order: the same sum on however many threads
const BLOCK: usize = 512;

/// The model: a weight for each number of a print, and the bias.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    w: Vec<f64>,
}

impl Model {
    /// How much a print looks like a real face, 0 to 1.
    pub fn score(&self, print: &[f32]) -> f64 {
        let z: f64 = print.iter().zip(&self.w).map(|(x, w)| *x as f64 * w).sum::<f64>() + self.w[print.len()];
        1.0 / (1.0 + (-z).exp())
    }
}

/// What the model is fitted to: the rows (faces) with their side (+1 a real face, -1 not) and weight.
struct Data<'a> {
    e: Prints<'a>,
    rows: Vec<usize>,
    y: Vec<f64>,
    c: Vec<f64>,
}

impl Data<'_> {
    fn n(&self) -> usize {
        self.e.dim + 1
    }

    /// Every row's sum of weight times number (the bias's number is 1).
    fn xv(&self, v: &[f64], out: &mut [f64]) {
        let (e, rows, dim) = (self.e, &self.rows, self.e.dim);
        spread(out, threads(), BLOCK, |first, part| {
            for (k, z) in part.iter_mut().enumerate() {
                let x = e.of(rows[first + k]);
                *z = x.iter().zip(v).fold(0.0, |sum, (a, b)| sum + *a as f64 * b) + v[dim];
            }
        });
    }

    /// For every number of a print, the sum over the rows of the row's value times that number.
    fn xtv(&self, v: &[f64], out: &mut [f64]) {
        let (e, rows, dim) = (self.e, &self.rows, self.e.dim);
        let mut blocks: Vec<Vec<f64>> = vec![Vec::new(); rows.len().div_ceil(BLOCK)];
        spread(&mut blocks, threads(), 1, |first, part| {
            for (b, sum) in part.iter_mut().enumerate() {
                *sum = vec![0.0; dim + 1];
                let from = (first + b) * BLOCK;
                for i in from..(from + BLOCK).min(rows.len()) {
                    for (s, x) in sum.iter_mut().zip(e.of(rows[i])) {
                        *s += v[i] * *x as f64;
                    }
                    sum[dim] += v[i];
                }
            }
        });
        out.fill(0.0);
        for block in &blocks {
            for (o, s) in out.iter_mut().zip(block) {
                *o += s;
            }
        }
    }

    /// What is made small: half the weights' squares, and each row's weighted loss. `z` is left holding
    /// every row's sum, which the slope is worked out from.
    fn fun(&self, w: &[f64], z: &mut [f64]) -> f64 {
        self.xv(w, z);
        let mut f = w.iter().map(|v| v * v).sum::<f64>() / 2.0;
        for ((z, y), c) in z.iter().zip(&self.y).zip(&self.c) {
            let yz = y * z;
            f += if yz >= 0.0 { c * (1.0 + (-yz).exp()).ln() } else { c * (-yz + (1.0 + yz.exp()).ln()) };
        }
        f
    }

    /// The slope at `w` (from the `z` that `fun` left), and for every row what the curvature needs (`d`).
    fn grad(&self, w: &[f64], z: &mut [f64], d: &mut [f64], g: &mut [f64]) {
        for (((z, d), y), c) in z.iter_mut().zip(d.iter_mut()).zip(&self.y).zip(&self.c) {
            let p = 1.0 / (1.0 + (-y * *z).exp());
            *d = p * (1.0 - p);
            *z = c * (p - 1.0) * y;
        }
        self.xtv(z, g);
        for (g, w) in g.iter_mut().zip(w) {
            *g += w;
        }
    }

    /// The curvature times a direction.
    fn hv(&self, s: &[f64], d: &[f64], wa: &mut [f64], hs: &mut [f64]) {
        self.xv(s, wa);
        for ((wa, d), c) in wa.iter_mut().zip(d).zip(&self.c) {
            *wa *= c * d;
        }
        self.xtv(wa, hs);
        for (h, s) in hs.iter_mut().zip(s) {
            *h += s;
        }
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn norm(a: &[f64]) -> f64 {
    dot(a, a).sqrt()
}

/// A step toward the smallest value within `delta` of here, by conjugate gradients; `r` is left holding what
/// is left of the slope after it.
fn step(data: &Data, delta: f64, g: &[f64], d_rows: &[f64], s: &mut [f64], r: &mut [f64]) {
    let n = g.len();
    let (mut d, mut hd, mut wa) = (vec![0.0; n], vec![0.0; n], vec![0.0; data.rows.len()]);
    s.fill(0.0);
    for k in 0..n {
        r[k] = -g[k];
        d[k] = r[k];
    }
    let enough = 0.1 * norm(g);
    let mut rtr = dot(r, r);
    while norm(r) > enough {
        data.hv(&d, d_rows, &mut wa, &mut hd);
        let mut alpha = rtr / dot(&d, &hd);
        s.iter_mut().zip(&d).for_each(|(s, d)| *s += alpha * d);
        if norm(s) > delta {
            // the step reaches the edge of the region: back, and then just as far as the edge
            s.iter_mut().zip(&d).for_each(|(s, d)| *s -= alpha * d);
            let (std, sts, dtd, dsq) = (dot(s, &d), dot(s, s), dot(&d, &d), delta * delta);
            let rad = (std * std + dtd * (dsq - sts)).sqrt();
            alpha = if std >= 0.0 { (dsq - sts) / (std + rad) } else { (rad - std) / dtd };
            s.iter_mut().zip(&d).for_each(|(s, d)| *s += alpha * d);
            r.iter_mut().zip(&hd).for_each(|(r, h)| *r -= alpha * h);
            break;
        }
        r.iter_mut().zip(&hd).for_each(|(r, h)| *r -= alpha * h);
        let rtr_new = dot(r, r);
        let beta = rtr_new / rtr;
        d.iter_mut().zip(r.iter()).for_each(|(d, r)| *d = *d * beta + r);
        rtr = rtr_new;
    }
}

/// Fit the model to these faces of `e`: `pos` the real ones, `neg` the others.
pub fn fit(e: Prints, pos: &[usize], neg: &[usize]) -> Model {
    // the two kinds weigh the same however many there are of each ("balanced")
    let all = (pos.len() + neg.len()) as f64;
    let (cp, cn) = (C * all / (2.0 * pos.len().max(1) as f64), C * all / (2.0 * neg.len().max(1) as f64));
    let data = Data {
        e,
        rows: neg.iter().chain(pos).copied().collect(),
        y: neg.iter().map(|_| -1.0).chain(pos.iter().map(|_| 1.0)).collect(),
        c: neg.iter().map(|_| cn).chain(pos.iter().map(|_| cp)).collect(),
    };
    let (n, l) = (data.n(), data.rows.len());
    let eps = TOL * (pos.len().min(neg.len()).max(1) as f64) / l.max(1) as f64;
    // how a step is taken or shortened, by how well it did against what was expected of it
    let (eta0, eta1, eta2): (f64, f64, f64) = (1e-4, 0.25, 0.75);
    let (sigma1, sigma2, sigma3): (f64, f64, f64) = (0.25, 0.5, 4.0);
    let mut w = vec![0.0; n];
    let (mut z, mut d) = (vec![0.0; l], vec![0.0; l]);
    let (mut g, mut s, mut r, mut w_new, mut z_new) =
        (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; l]);
    let mut f = data.fun(&w, &mut z);
    data.grad(&w, &mut z, &mut d, &mut g);
    let mut delta = norm(&g);
    let first = delta;
    let mut iter = 1;
    while iter <= MAX_ITER && first > 0.0 && norm(&g) > eps * first {
        step(&data, delta, &g, &d, &mut s, &mut r);
        for k in 0..n {
            w_new[k] = w[k] + s[k];
        }
        let gs = dot(&g, &s);
        let expected = -0.5 * (gs - dot(&s, &r));
        let f_new = data.fun(&w_new, &mut z_new);
        let got = f - f_new;
        let far = norm(&s);
        if iter == 1 {
            delta = delta.min(far);
        }
        let alpha = if f_new - f - gs <= 0.0 { sigma3 } else { sigma1.max(-0.5 * (gs / (f_new - f - gs))) };
        delta = if got < eta0 * expected {
            (alpha.max(sigma1) * far).min(sigma2 * delta)
        } else if got < eta1 * expected {
            (sigma1 * delta).max((alpha * far).min(sigma2 * delta))
        } else if got < eta2 * expected {
            (sigma1 * delta).max((alpha * far).min(sigma3 * delta))
        } else {
            delta.max((alpha * far).min(sigma3 * delta))
        };
        if got > eta0 * expected {
            iter += 1;
            w.copy_from_slice(&w_new);
            z.copy_from_slice(&z_new);
            f = f_new;
            data.grad(&w, &mut z, &mut d, &mut g);
        }
        if f < -1.0e32
            || (got.abs() <= 0.0 && expected <= 0.0)
            || (got.abs() <= 1.0e-12 * f.abs() && expected.abs() <= 1.0e-12 * f.abs())
        {
            break;
        }
    }
    Model { w }
}

/// What was learned about a library's possible faces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Faceness {
    /// the model of all faces, for a print it hasn't seen (a box drawn on the spot); none while there is too
    /// little to learn from
    pub model: Option<Model>,
    /// each possible face's score
    pub scores: HashMap<usize, f64>,
    /// the score a possible face needs for its dashed box: 0 (show all) without a model
    pub cut: f64,
}

/// The quantile `q` of the values, between the two nearest of them.
fn quantile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let at = q * (values.len() - 1) as f64;
    let (low, part) = (at.floor() as usize, at - at.floor());
    let high = (low + 1).min(values.len() - 1);
    values[low] + (values[high] - values[low]) * part
}

/// Learn from the faces of `e`: `pos` the real faces, `neg` the possible faces nobody named, `idx` every
/// possible face (drawn ones aside) to be scored.
pub fn learn(e: Prints, pos: &[usize], neg: &[usize], idx: &[usize]) -> Faceness {
    if pos.len() < FEW || neg.len() < FEW {
        return Faceness::default();
    }
    let model = fit(e, pos, neg);
    let mut scores = HashMap::new();
    if !idx.is_empty() {
        // each of them by a model that hasn't seen it
        let folds: Vec<Vec<usize>> = (0..5).map(|k| neg.iter().skip(k).step_by(5).copied().collect()).collect();
        for (k, fold) in folds.iter().enumerate() {
            let others: Vec<usize> =
                folds.iter().enumerate().filter(|(kk, _)| *kk != k).flat_map(|(_, f)| f.iter().copied()).collect();
            let without = fit(e, pos, &others);
            scores.extend(fold.iter().map(|j| (*j, without.score(e.of(*j)))));
        }
        let junk: std::collections::HashSet<usize> = neg.iter().copied().collect();
        scores.extend(idx.iter().filter(|j| !junk.contains(j)).map(|j| (*j, model.score(e.of(*j)))));
    }
    let cut = if scores.is_empty() {
        0.0
    } else {
        quantile(&mut scores.values().copied().collect::<Vec<_>>(), 1.0 - FACE_SHOW)
    };
    Faceness { model: Some(model), scores, cut }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prints of two kinds: around one direction and around another, some of each close to the other kind.
    fn two_kinds(n: usize, dim: usize) -> Vec<f32> {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5
        };
        let mut emb = Vec::new();
        for i in 0..n {
            let mut v: Vec<f32> = (0..dim).map(|_| 0.8 * next()).collect();
            v[0] += if i % 2 == 0 { 0.5 } else { -0.5 };
            let long = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            emb.extend(v.iter().map(|x| x / long));
        }
        emb
    }

    #[test]
    fn the_model_is_at_the_lowest_point_and_tells_the_two_kinds_apart() {
        let (n, dim) = (600, 12);
        let emb = two_kinds(n, dim);
        let e = Prints { emb: &emb, dim };
        let pos: Vec<usize> = (0..n).filter(|i| i % 2 == 0 && i % 5 != 0).collect();
        let neg: Vec<usize> = (0..n).filter(|i| i % 2 == 1).collect();
        let model = fit(e, &pos, &neg);
        // at the lowest point the slope is (nearly) flat: far flatter than at the start
        let all = (pos.len() + neg.len()) as f64;
        let data = Data {
            e,
            rows: neg.iter().chain(&pos).copied().collect(),
            y: neg.iter().map(|_| -1.0).chain(pos.iter().map(|_| 1.0)).collect(),
            c: neg
                .iter()
                .map(|_| C * all / (2.0 * neg.len() as f64))
                .chain(pos.iter().map(|_| C * all / (2.0 * pos.len() as f64)))
                .collect(),
        };
        let slope = |w: &[f64]| {
            let (mut z, mut d, mut g) = (vec![0.0; data.rows.len()], vec![0.0; data.rows.len()], vec![0.0; dim + 1]);
            data.fun(w, &mut z);
            data.grad(w, &mut z, &mut d, &mut g);
            norm(&g)
        };
        let (start, end) = (slope(&vec![0.0; dim + 1]), slope(&model.w));
        assert!(end <= 1e-4 * start, "the slope went from {start} to {end}");
        // and a small step any way from there is no lower
        let lowest = data.fun(&model.w, &mut vec![0.0; data.rows.len()]);
        for k in 0..=dim {
            for by in [1e-3, -1e-3] {
                let mut w = model.w.clone();
                w[k] += by;
                assert!(data.fun(&w, &mut vec![0.0; data.rows.len()]) >= lowest - 1e-12, "lower at weight {k} {by:+}");
            }
        }
        let right = pos.iter().filter(|i| model.score(e.of(**i)) > 0.5).count()
            + neg.iter().filter(|i| model.score(e.of(**i)) < 0.5).count();
        assert!(right as f64 > 0.8 * all, "{right} of {all} told apart");
        // faces it never saw (every fifth even one) look real to it too, mostly
        let unseen: Vec<usize> = (0..n).filter(|i| i % 10 == 0).collect();
        assert!(unseen.iter().filter(|i| model.score(e.of(**i)) > 0.5).count() * 10 > unseen.len() * 7);
    }

    #[test]
    fn every_possible_face_is_scored_and_the_most_face_like_quarter_is_shown() {
        let (n, dim) = (600, 12);
        let emb = two_kinds(n, dim);
        let e = Prints { emb: &emb, dim };
        let pos: Vec<usize> = (0..n).filter(|i| i % 2 == 0 && i % 5 != 0).collect();
        let neg: Vec<usize> = (0..n).filter(|i| i % 2 == 1).collect();
        // the possible faces: the unnamed ones (neg) and some that were named (of the even ones left out)
        let named: Vec<usize> = (0..n).filter(|i| i % 10 == 0).collect();
        let idx: Vec<usize> = neg.iter().chain(&named).copied().collect();
        let learned = learn(e, &pos, &neg, &idx);
        assert_eq!(learned.scores.len(), idx.len());
        let shown = learned.scores.values().filter(|s| **s >= learned.cut).count();
        assert!((shown as f64 / idx.len() as f64 - FACE_SHOW).abs() < 0.01, "{shown} of {} shown", idx.len());
        let model = learned.model.as_ref().unwrap();
        assert!(named.iter().all(|j| learned.scores[j] == model.score(e.of(*j))), "the named ones by the model of all");
        assert!(
            neg.iter().any(|j| (learned.scores[j] - model.score(e.of(*j))).abs() > 1e-9),
            "the others by a model without them"
        );
        assert_eq!(learn(e, &pos[..FEW - 1], &neg, &idx), Faceness::default(), "too few to learn from");
        assert_eq!(quantile(&mut [4.0, 1.0, 3.0, 2.0], 0.75), 3.25);
        assert_eq!((quantile(&mut [5.0], 0.75), quantile(&mut [1.0, 2.0], 0.5)), (5.0, 1.5));
    }
}
