//! Every face's score for every person: the mean of the `TOPK` closest of that person's usable faces, as the
//! recognizer scores. A face never counts
//! toward its own score.
//!
//! Scoring every face against every person anew costs faces x named faces comparisons (hundreds of millions
//! in a real library), and the labeling page asks after every change. So every face's score for every person
//! is kept (`Top`), and brought up to date only where something changed:
//!
//! - a person whose usable faces changed: scored again for every face. For someone with more than `BIG`
//!   faces that would be most of the work again, so for them each face's `TOPK` closest are kept too: a face
//!   of theirs that came is compared with every face and taken in where it is closer; for one that went, the
//!   faces that could have counted it among their closest are looked at again;
//! - a new face, or one whose print changed: scored again for every person; and when it is one of a person's
//!   own faces, all of that person's is made anew, since what it was closest to can't be told any more.
//!
//! Which prints changed is read from the store's own log of them (`Store::print_log`), which nothing that
//! changes a print can leave out. What is kept is exactly what scoring anew gives: every comparison is made
//! by the same function (`dot`), whichever way it is arrived at, so the two can't drift apart;
//! a test holds what is kept to scoring anew.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

/// a person's score = the mean of this many closest of their faces (a measured value, labels.TOPK)
pub const TOPK: usize = 5;
/// a person with more usable faces than this also keeps every face's closest of theirs (labels.BIG)
pub const BIG: usize = 256;
/// what a row holds where a person has fewer faces to show
const NONE: f32 = -2.0;
/// Prints compared a tile of faces against a tile of faces: 48 prints of 512 numbers are 96 KB, so both
/// tiles stay in the processor's own memory while every pair of them is compared. One face after another
/// against all of a person's faces fetched those from main memory again for every face.
pub const TILE: usize = 48;

fn dot_plain(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut sum = ((acc[0] + acc[4]) + (acc[1] + acc[5])) + ((acc[2] + acc[6]) + (acc[3] + acc[7]));
    for (x, y) in ra.iter().zip(rb) {
        sum += x * y;
    }
    sum
}

/// The same with the processor's 8-wide instructions, where it has them (most computers since 2013).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn dot_wide(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    let n = a.len().min(b.len());
    let (pa, pb) = (a.as_ptr(), b.as_ptr());
    let mut acc = [_mm256_setzero_ps(); 4];
    let mut i = 0;
    // SAFETY: every read is of 8 numbers that begin before n - 7, inside both slices
    unsafe {
        while i + 32 <= n {
            for (k, sum) in acc.iter_mut().enumerate() {
                *sum = _mm256_fmadd_ps(_mm256_loadu_ps(pa.add(i + 8 * k)), _mm256_loadu_ps(pb.add(i + 8 * k)), *sum);
            }
            i += 32;
        }
        while i + 8 <= n {
            acc[0] = _mm256_fmadd_ps(_mm256_loadu_ps(pa.add(i)), _mm256_loadu_ps(pb.add(i)), acc[0]);
            i += 8;
        }
    }
    let all = _mm256_add_ps(_mm256_add_ps(acc[0], acc[1]), _mm256_add_ps(acc[2], acc[3]));
    let mut lanes = [0f32; 8];
    // SAFETY: lanes holds 8 numbers
    unsafe { _mm256_storeu_ps(lanes.as_mut_ptr(), all) };
    let mut sum = ((lanes[0] + lanes[4]) + (lanes[1] + lanes[5])) + ((lanes[2] + lanes[6]) + (lanes[3] + lanes[7]));
    while i < n {
        sum += a[i] * b[i];
        i += 1;
    }
    sum
}

/// How alike two prints are: their sum of products. Always worked out the same way while a program runs, so
/// the same two prints always give the same number; another computer's may differ in the last digit.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma") {
        // SAFETY: the processor has the instructions the function is built with
        return unsafe { dot_wide(a, b) };
    }
    dot_plain(a, b)
}

/// The prints, a row a face.
#[derive(Clone, Copy)]
pub struct Prints<'a> {
    pub emb: &'a [f32],
    pub dim: usize,
}

impl<'a> Prints<'a> {
    pub fn len(&self) -> usize {
        if self.dim == 0 { 0 } else { self.emb.len() / self.dim }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn of(&self, i: usize) -> &'a [f32] {
        &self.emb[i * self.dim..(i + 1) * self.dim]
    }
    pub fn has_print(&self, i: usize) -> bool {
        self.of(i).iter().any(|v| *v != 0.0)
    }
}

/// How many threads this computer runs side by side.
pub fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// `work(first, part)` on every stretch of `rows`, on up to `threads` threads: the rows are independent.
pub fn in_parallel<T: Send>(rows: &mut [T], threads: usize, work: impl Fn(usize, &mut [T]) + Sync) {
    spread(rows, threads, 4 * TILE, work)
}

/// The same for any list of independent things, at least `grain` of them to a thread.
pub fn spread<T: Send>(rows: &mut [T], threads: usize, grain: usize, work: impl Fn(usize, &mut [T]) + Sync) {
    let threads = threads.min(rows.len().div_ceil(grain.max(1))).max(1);
    if threads <= 1 {
        return work(0, rows);
    }
    let per = rows.len().div_ceil(threads).next_multiple_of(TILE.min(grain.max(1)));
    std::thread::scope(|s| {
        for (k, part) in rows.chunks_mut(per).enumerate() {
            let work = &work;
            s.spawn(move || work(k * per, part));
        }
    });
}

/// Take `s` into a row of closest values, kept closest first, when it is closer than the furthest of them.
#[inline]
fn take(row: &mut [f32; TOPK], s: f32) {
    if s > row[TOPK - 1] {
        let mut k = TOPK - 1;
        while k > 0 && row[k - 1] < s {
            row[k] = row[k - 1];
            k -= 1;
        }
        row[k] = s;
    }
}

/// For each row of `out`, which is face `face_of(its place)`: the faces `cols` taken into its closest (a face
/// never into its own).
fn take_all(
    e: Prints,
    out: &mut [[f32; TOPK]],
    face_of: impl Fn(usize) -> usize + Sync,
    cols: &[usize],
    threads: usize,
) {
    in_parallel(out, threads, |first, part| {
        for (t, rows) in part.chunks_mut(TILE).enumerate() {
            for some in cols.chunks(TILE) {
                for (k, row) in rows.iter_mut().enumerate() {
                    let r = face_of(first + t * TILE + k);
                    let mine = e.of(r);
                    for &c in some {
                        if c != r {
                            take(row, dot(mine, e.of(c)));
                        }
                    }
                }
            }
        }
    });
}

/// For each of the faces `which`, the `TOPK` closest of a person's faces `cols`, closest first; `NONE` where
/// there are fewer.
fn closest(e: Prints, which: &[usize], cols: &[usize], threads: usize) -> Vec<[f32; TOPK]> {
    let mut out = vec![[NONE; TOPK]; which.len()];
    take_all(e, &mut out, |k| which[k], cols, threads);
    out
}

/// A score from a face's closest of a person's `faces` faces: the mean of the `TOPK` closest, or of as many as
/// there are (`own`: the face is one of them, and doesn't count); -1 where nothing is left to compare with.
fn score(row: &[f32; TOPK], own: bool, faces: usize) -> f32 {
    let k = TOPK.min(faces - usize::from(own));
    if k == 0 {
        return -1.0;
    }
    // added up as numbers of this size and divided as larger ones (fixed: kept scores depend on it)
    (row[..k].iter().fold(0f32, |a, v| a + v) as f64 / k as f64) as f32
}

/// One person's kept scores.
struct Top {
    /// their usable faces when this was made, in rising order
    faces: Vec<usize>,
    /// every face's score for them
    score: Vec<f32>,
    /// for someone with more than `BIG` faces: every face's closest of `faces`
    rows: Option<Vec<[f32; TOPK]>>,
    /// how much of the log of changed prints this has taken in
    seen: usize,
}

impl Top {
    fn anew(e: Prints, faces: &[usize], seen: usize, threads: usize) -> Top {
        let mut rows = vec![[NONE; TOPK]; e.len()];
        take_all(e, &mut rows, |r| r, faces, threads);
        let mut top = Top { faces: faces.to_vec(), score: vec![-1.0; e.len()], rows: None, seen };
        top.rescore(&rows);
        top.rows = (faces.len() > BIG).then_some(rows);
        top
    }

    fn own(&self, i: usize) -> bool {
        self.faces.binary_search(&i).is_ok()
    }

    /// Every face's score, from every face's closest `rows`.
    fn rescore(&mut self, rows: &[[f32; TOPK]]) {
        for (i, row) in rows.iter().enumerate() {
            self.score[i] = score(row, self.own(i), self.faces.len());
        }
    }

    /// Brought up to date for the person's faces now and the prints as they are now. `changed`: the faces
    /// whose print changed since this was last brought up to date.
    fn update(&mut self, e: Prints, faces: &[usize], changed: &[usize], seen: usize, threads: usize) {
        let n = e.len();
        let was: HashSet<usize> = self.faces.iter().copied().collect();
        let same = self.faces == faces;
        let kept = self.rows.is_some() && faces.len() > BIG;
        if self.score.len() > n || changed.iter().any(|c| was.contains(c)) || !(same || kept) {
            *self = Top::anew(e, faces, seen, threads);
            return;
        }
        // new faces and changed prints are looked at again, against all of the person's faces
        let mut again: HashSet<usize> = changed.iter().copied().collect();
        again.extend(self.score.len()..n);
        self.score.resize(n, -1.0);
        let Some(mut rows) = self.rows.take().filter(|_| kept) else {
            // the same faces as before, and not many: only those faces' scores
            let mut redo: Vec<usize> = again.into_iter().collect();
            redo.sort_unstable();
            for (r, row) in redo.iter().zip(closest(e, &redo, faces, threads)) {
                self.score[*r] = score(&row, self.own(*r), faces.len());
            }
            self.seen = seen;
            return;
        };
        rows.resize(n, [NONE; TOPK]);
        let now: HashSet<usize> = faces.iter().copied().collect();
        let gone: Vec<usize> = self.faces.iter().copied().filter(|f| !now.contains(f)).collect();
        let came: Vec<usize> = faces.iter().copied().filter(|f| !was.contains(f)).collect();
        if !gone.is_empty() {
            // the faces that could have counted one of them among their closest (a blurred face can be among
            // the closest of thousands). Not the faces without a print: nothing is close to them, whoever
            // comes or goes
            let mut hit = vec![false; n];
            let rows = &rows;
            in_parallel(&mut hit, threads, |first, part| {
                for (k, h) in part.iter_mut().enumerate() {
                    let r = first + k;
                    *h = e.has_print(r) && gone.iter().any(|&g| dot(e.of(r), e.of(g)) >= rows[r][TOPK - 1]);
                }
            });
            again.extend(hit.iter().enumerate().filter(|(_, h)| **h).map(|(r, _)| r));
        }
        if !came.is_empty() && again.len() < n {
            take_all(e, &mut rows, |r| r, &came, threads);
        }
        let mut redo: Vec<usize> = again.into_iter().collect();
        redo.sort_unstable();
        for (r, row) in redo.iter().zip(closest(e, &redo, faces, threads)) {
            rows[*r] = row;
        }
        self.faces = faces.to_vec();
        self.rescore(&rows);
        self.rows = Some(rows);
        self.seen = seen;
    }
}

/// The kept scores of a library: every face's score for every person of the gallery.
#[derive(Default)]
pub struct Scores {
    tops: HashMap<String, Top>,
}

impl Scores {
    /// Brought up to date for this gallery (each person's usable faces, in rising order) and these prints:
    /// people who went are let go of, the others updated or made. `log`: every face whose print changed, in
    /// the order it happened, ever longer (`Store::print_log`).
    pub fn update(&mut self, e: Prints, log: &[usize], gallery: &[(String, Vec<usize>)]) {
        let named: HashSet<&str> = gallery.iter().map(|(n, _)| n.as_str()).collect();
        self.tops.retain(|name, _| named.contains(name.as_str()));
        let (n, seen, all) = (e.len(), log.len(), threads());
        let mut work: Vec<(usize, Option<Top>)> = Vec::new();
        for (g, (name, faces)) in gallery.iter().enumerate() {
            match self.tops.get(name) {
                Some(t) if t.faces == *faces && t.score.len() == n && t.seen == seen => {}
                _ => work.push((g, self.tops.remove(name))),
            }
        }
        let run = |g: usize, old: Option<Top>, threads: usize| match old {
            Some(mut top) if top.seen <= seen => {
                let mut changed: Vec<usize> = log[top.seen..].iter().copied().filter(|c| *c < n).collect();
                changed.sort_unstable();
                changed.dedup();
                top.update(e, &gallery[g].1, &changed, seen, threads);
                top
            }
            _ => Top::anew(e, &gallery[g].1, seen, threads),
        };
        // someone with very many faces is a job for every thread; the others are a job each, side by side
        let (large, small): (Vec<_>, Vec<_>) = work.into_iter().partition(|(g, _)| gallery[*g].1.len() > BIG);
        for (g, old) in large {
            self.tops.insert(gallery[g].0.clone(), run(g, old, all));
        }
        if small.len() < 2 * all {
            for (g, old) in small {
                self.tops.insert(gallery[g].0.clone(), run(g, old, all));
            }
            return;
        }
        let (jobs, next) = (small.into_iter().map(|j| Mutex::new(Some(j))).collect::<Vec<_>>(), AtomicUsize::new(0));
        let done: Mutex<Vec<(usize, Top)>> = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..all {
                s.spawn(|| {
                    while let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if let Some((g, old)) = job.lock().ok().and_then(|mut j| j.take()) {
                            let top = run(g, old, 1);
                            if let Ok(mut d) = done.lock() {
                                d.push((g, top));
                            }
                        }
                    }
                });
            }
        });
        for (g, top) in done.into_inner().unwrap_or_default() {
            self.tops.insert(gallery[g].0.clone(), top);
        }
    }

    /// Every face's score for a person of the gallery last brought up to date.
    pub fn column(&self, name: &str) -> Option<&[f32]> {
        self.tops.get(name).map(|t| t.score.as_slice())
    }

    /// Face i's score for a person of the gallery last brought up to date; -1 for someone not in it.
    pub fn of(&self, name: &str, i: usize) -> f32 {
        self.tops.get(name).and_then(|t| t.score.get(i)).copied().unwrap_or(-1.0)
    }
}

/// The same score worked out anew, one face against one person's faces (in rising order): what `Scores` must
/// give. For the tests, and for a question about faces that aren't the gallery's.
pub fn score_anew(e: Prints, i: usize, faces: &[usize]) -> f32 {
    score(&closest(e, &[i], faces, 1)[0], faces.binary_search(&i).is_ok(), faces.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made-up prints of length one, the same at every run: people's faces lie near their person's own
    /// direction, with a few exact copies among them.
    fn library(n: usize, dim: usize, people: usize, seed: u64) -> Vec<f32> {
        let mut seed = seed;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5
        };
        let centres: Vec<Vec<f32>> = (0..people).map(|_| (0..dim).map(|_| next()).collect()).collect();
        let mut emb = Vec::with_capacity(n * dim);
        for i in 0..n {
            let c = &centres[i % people];
            let v: Vec<f32> = (0..dim).map(|k| c[k] + 0.6 * next()).collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            emb.extend(v.iter().map(|x| x / norm));
        }
        for copy in [7usize, 70].into_iter().filter(|c| *c < n) {
            let from = emb[(copy - 1) * dim..copy * dim].to_vec();
            emb[copy * dim..(copy + 1) * dim].copy_from_slice(&from);
        }
        emb
    }

    fn check(scores: &Scores, e: Prints, gallery: &[(String, Vec<usize>)], when: &str) {
        for (name, faces) in gallery {
            for i in 0..e.len() {
                let (kept, anew) = (scores.of(name, i), score_anew(e, i, faces));
                assert!(kept.to_bits() == anew.to_bits(), "{when}: face {i} for {name}: kept {kept}, anew {anew}");
            }
        }
    }

    fn e(emb: &[f32]) -> Prints<'_> {
        Prints { emb, dim: 16 }
    }

    /// The same changes to a library where one person has very many faces (their closest are kept) and to one
    /// where nobody has: every way the kept scores are brought up to date.
    fn every_kind_of_change(n: usize, per: usize) {
        let (dim, people) = (16, 4);
        let mut emb = library(n, dim, people, 0x2545_f491_4f6c_dd1d);
        let mut log: Vec<usize> = Vec::new();
        // person 0 has the most faces: of all but the last 60 faces; the others of the first `per` x 4
        let mut gallery: Vec<(String, Vec<usize>)> = (0..people)
            .map(|p| {
                let top = if p == 0 { n - 60 } else { per * people };
                (format!("person {p}"), (0..top).filter(|i| i % people == p && i % 3 != 0).collect())
            })
            .collect();
        gallery.push(("one face".to_string(), vec![n - 59]));
        gallery.push(("nobody's".to_string(), vec![]));
        let mut scores = Scores::default();
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "made");

        // faces come to a person and go from one; a person comes, another goes
        gallery[0].1.extend([n - 56, n - 52, n - 48]);
        gallery[0].1.sort_unstable();
        gallery[0].1.retain(|i| *i != 8 && *i != 20);
        gallery[1].1.retain(|i| *i != 1 && *i != 61 && *i != 7);
        gallery[2].1.clear();
        gallery.remove(4);
        gallery.push(("new".to_string(), vec![n - 41, n - 37]));
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "faces came and went");

        // a face moves from one person to another in one step, and a copy goes
        gallery[0].1.retain(|i| *i != 4);
        gallery[1].1.push(4);
        gallery[1].1.sort_unstable();
        gallery[3].1.retain(|i| *i != 71);
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "a face moved");

        // prints change: one that is a person's own (deleted, as an Ignore does) and one that is nobody's;
        // then the first put back and theirs again
        let kept: Vec<f32> = emb[16 * dim..17 * dim].to_vec();
        emb[16 * dim..17 * dim].fill(0.0);
        log.push(16);
        emb[(n - 30) * dim..(n - 29) * dim].fill(0.0);
        log.push(n - 30);
        gallery.iter_mut().for_each(|(_, f)| f.retain(|i| *i != 16));
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "prints deleted");
        emb[16 * dim..17 * dim].copy_from_slice(&kept);
        log.push(16);
        gallery[0].1.push(16);
        gallery[0].1.sort_unstable();
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "a print put back");
        // only a print that is nobody's changes: every person's score for that one face
        emb[(n - 29) * dim..(n - 28) * dim].copy_from_slice(&kept);
        log.push(n - 29);
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "nobody's print changed");

        // new faces (an import), two of them named at once
        emb.extend(library(40, dim, people, 0x9e37_79b9_7f4a_7c15));
        gallery[0].1.push(n + 20);
        gallery[3].1.push(n + 21);
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "faces added");
        // new faces and nothing else
        emb.extend(library(9, dim, people, 0x1234_5678_9abc_def1));
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "more faces added");
        // a print of one of a person's own faces changes while it stays theirs (made again from its photo)
        let other: Vec<f32> = emb[5 * dim..6 * dim].to_vec();
        emb[(n + 20) * dim..(n + 21) * dim].copy_from_slice(&other);
        log.push(n + 20);
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "a person's own face made again");
        // nothing changed: nothing to do, and the same answers
        scores.update(e(&emb), &log, &gallery);
        check(&scores, e(&emb), &gallery, "asked again");
    }

    #[test]
    fn what_is_kept_is_what_scoring_anew_gives_after_every_kind_of_change() {
        every_kind_of_change(600, 75);
    }

    #[test]
    fn and_for_someone_with_very_many_faces_whose_closest_are_kept_too() {
        every_kind_of_change(2400, 30); // person 0: 390 faces, more than BIG
    }

    #[test]
    fn many_people_are_scored_side_by_side_to_the_same_answers() {
        let (dim, people) = (16, 80);
        let emb = library(1200, dim, people, 0x0bad_5eed_0bad_5eed);
        let gallery: Vec<(String, Vec<usize>)> =
            (0..people).map(|p| (format!("p{p:02}"), (0..800).filter(|i| i % people == p).collect())).collect();
        let mut scores = Scores::default();
        scores.update(e(&emb), &[], &gallery);
        check(&scores, e(&emb), &gallery, "made side by side");
    }

    #[test]
    fn a_score_is_the_mean_of_the_closest_and_never_counts_the_face_itself() {
        // four prints along one line and one across it: every product is 1, 0 or -1
        let emb = [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0];
        let e = Prints { emb: &emb, dim: 2 };
        assert_eq!(score_anew(e, 4, &[0, 1, 2, 3]), 0.0);
        assert_eq!(score_anew(e, 0, &[0, 1, 2, 3]), (1.0 + 1.0 - 1.0) / 3.0, "the face itself is left out");
        assert_eq!(score_anew(e, 3, &[0, 1, 2]), -1.0);
        assert_eq!(score_anew(e, 0, &[0]), -1.0, "nothing left to compare with");
        assert_eq!(score_anew(e, 0, &[]), -1.0);
        // more than TOPK faces: only the closest count
        let many: Vec<f32> = (0..9).flat_map(|k| if k < 6 { [1.0, 0.0] } else { [-1.0, 0.0] }).collect();
        let e = Prints { emb: &many, dim: 2 };
        assert_eq!(score_anew(e, 0, &[0, 1, 2, 3, 4, 5, 6, 7, 8]), 1.0);
        assert_eq!(score_anew(e, 8, &[0, 1, 2, 3, 4, 5, 6, 7]), (1.0 + 1.0 - 1.0 - 1.0 - 1.0) / 5.0);
        assert_eq!(dot(&[1.0; 19], &[2.0; 19]), 38.0, "a print's last few numbers count too");
        assert_eq!(dot(&[0.5; 512], &[2.0; 512]), 512.0);
        assert_eq!(dot_plain(&[0.5; 77], &[2.0; 77]), 77.0);
    }
}
