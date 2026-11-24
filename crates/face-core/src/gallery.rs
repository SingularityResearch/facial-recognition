//! Who a face print belongs to: every named person's prints (`people.npz`, written by the labeling engine)
//! and the rule the tracker names faces by. Read-only once made, so one can be shared between threads.

use std::path::Path;

use crate::{Error, Result, npy};

pub struct Identifier {
    /// everyone named, in order
    pub names: Vec<String>,
    /// per person, in `names`' order: their prints one after another
    prints: Vec<Vec<f32>>,
    dim: usize,
}

impl Identifier {
    pub fn new(emb: &[f32], names: &[String], dim: usize) -> Result<Identifier> {
        if dim == 0 || names.len().checked_mul(dim) != Some(emb.len()) {
            return Err(Error::msg("the people file's prints and names don't fit together"));
        }
        let mut order: Vec<String> = names.to_vec();
        order.sort();
        order.dedup();
        let mut prints = vec![Vec::new(); order.len()];
        for (i, n) in names.iter().enumerate() {
            let k = order.binary_search(n).expect("a name of the list");
            prints[k].extend_from_slice(&emb[i * dim..(i + 1) * dim]);
        }
        Ok(Identifier { names: order, prints, dim })
    }

    /// `people.npz`, or None when nobody is named yet or it was made with another recognition model.
    pub fn load(path: &Path, rec_name: &str) -> Result<Option<Identifier>> {
        if !path.is_file() {
            return Ok(None);
        }
        Identifier::from_npz(&std::fs::read(path)?, rec_name)
    }

    /// The same from the file's bytes, wherever they came from (another device of the owner's gives its
    /// people file as it is): None when nobody is named in it or it was made with another recognition model.
    pub fn from_npz(bytes: &[u8], rec_name: &str) -> Result<Option<Identifier>> {
        Identifier::of(npy::read_npz(bytes)?, rec_name)
    }

    /// The same from bytes another device gave: read with nothing taken on trust (`npy::read_npz_foreign`),
    /// so that a file made to break the reader stops nothing but itself.
    pub fn from_foreign_npz(bytes: &[u8], rec_name: &str) -> Result<Option<Identifier>> {
        Identifier::of(npy::read_npz_foreign(bytes)?, rec_name)
    }

    fn of(z: std::collections::HashMap<String, npy::Array>, rec_name: &str) -> Result<Option<Identifier>> {
        let names = z.get("name").ok_or_else(|| Error::msg("the people file has no names"))?.text()?;
        if names.is_empty() {
            return Ok(None);
        }
        let made_with = match z.get("model") {
            Some(m) => m.text()?.first().cloned().unwrap_or_default(),
            None => "sface_2021dec".to_string(),
        };
        if made_with != rec_name {
            eprintln!(
                "people.npz was made with another recognition model than {rec_name}: nobody is named until it is made again"
            );
            return Ok(None);
        }
        let emb = z.get("emb").ok_or_else(|| Error::msg("the people file has no prints"))?;
        let dim = emb.shape.get(1).copied().unwrap_or(0);
        Identifier::new(emb.f32()?, names, dim).map(Some)
    }

    /// (people named, prints they are known by): numbers for a log line.
    pub fn counts(&self) -> (usize, usize) {
        (self.names.len(), self.prints.iter().map(|p| p.len() / self.dim.max(1)).sum())
    }

    /// (name, similarity) for a print of length 1: the person whose five closest prints are closest on
    /// average (robust to one odd photo), or "Unknown" below `threshold`.
    pub fn who(&self, emb: &[f32], threshold: f64) -> (String, f32) {
        if emb.len() != self.dim {
            return ("Unknown".to_string(), 0.0);
        }
        let (mut best, mut best_s) = (None, -1.0f32);
        for (k, g) in self.prints.iter().enumerate() {
            let mut top = [f32::MIN; 5]; // the five largest, smallest first
            let mut n = 0;
            for p in g.chunks_exact(self.dim) {
                let s = dot(p, emb);
                n += 1;
                if s > top[0] {
                    top[0] = s;
                    let mut i = 0;
                    while i + 1 < 5 && top[i] > top[i + 1] {
                        top.swap(i, i + 1);
                        i += 1;
                    }
                }
            }
            let k5 = n.min(5);
            if k5 == 0 {
                continue;
            }
            let s = top[5 - k5..].iter().sum::<f32>() / k5 as f32;
            if s > best_s {
                best = Some(k);
                best_s = s;
            }
        }
        match best {
            Some(k) if best_s as f64 >= threshold => (self.names[k].clone(), best_s),
            _ => ("Unknown".to_string(), best_s),
        }
    }
}

/// A dot product in eight running sums, which the compiler turns into wide instructions.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

#[cfg(test)]
mod foreign_tests {
    use super::*;

    #[test]
    fn prints_and_names_that_dont_fit_are_refused_however_the_numbers_are_chosen() {
        let names = ["Ann".to_string(), "Ben".to_string()];
        // a width so large that names x width wraps round to the length of no prints at all
        assert!(Identifier::new(&[], &names, 9_223_372_036_854_775_808).is_err());
        assert!(Identifier::new(&[0.0; 4], &names, 0).is_err());
        assert!(Identifier::new(&[0.0; 5], &names, 2).is_err());
        let two = Identifier::new(&[1.0, 0.0, 0.0, 1.0], &names, 2).unwrap();
        assert_eq!(two.counts(), (2, 2));
        // bytes that are no archive, from another device: an error, nothing more
        assert!(Identifier::from_foreign_npz(b"not an archive", "opsis_1").is_err());
        assert!(Identifier::from_foreign_npz(&[], "opsis_1").is_err());
    }
}
