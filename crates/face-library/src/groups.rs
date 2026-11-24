//! Look-alike groups of faces nobody is suggested for (average-linkage clustering by cosine distance,
//! stopped at a distance), and
//! the stacks of faces that are the same picture again.
//!
//! The groups: every face starts as a group of its own, and the two groups that are closest on average (one
//! minus how alike their faces are, over every pair of faces between them) become one, again and again; the
//! groups that were joined below the cut-off are the answer. Worked out as SciPy does (`linkage`, method
//! "average"): the distances between all pairs as 64-bit numbers, and the joins found by following each
//! group to its nearest neighbour until two are each other's nearest.

use crate::scores::{Prints, TILE, dot, spread};

/// How unlike two prints are (the cosine distance), as 64-bit numbers.
fn apart(a: &[f32], b: &[f32], long_a: f64, long_b: f64) -> f64 {
    let mut acc = [0f64; 4];
    let (ca, cb) = (a.chunks_exact(4), b.chunks_exact(4));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for k in 0..4 {
            acc[k] += x[k] as f64 * y[k] as f64;
        }
    }
    let mut sum = (acc[0] + acc[2]) + (acc[1] + acc[3]);
    for (x, y) in ra.iter().zip(rb) {
        sum += *x as f64 * *y as f64;
    }
    1.0 - (sum / (long_a * long_b)).clamp(-1.0, 1.0)
}

/// Lowest member of each face's set, sets joined two at a time.
pub struct Sets {
    parent: Vec<usize>,
}

impl Sets {
    pub fn new(n: usize) -> Sets {
        Sets { parent: (0..n).collect() }
    }
    pub fn of(parent: Vec<usize>) -> Sets {
        Sets { parent }
    }
    pub fn len(&self) -> usize {
        self.parent.len()
    }
    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }
    pub fn grow(&mut self, n: usize) {
        let from = self.parent.len();
        self.parent.extend(from..n);
    }
    pub fn root(&mut self, mut a: usize) -> usize {
        while self.parent[a] != a {
            self.parent[a] = self.parent[self.parent[a]];
            a = self.parent[a];
        }
        a
    }
    /// One set of the two: its first face is its lowest.
    pub fn join(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra != rb {
            self.parent[ra.max(rb)] = ra.min(rb);
        }
    }
    /// A face on its own again.
    pub fn alone(&mut self, a: usize) {
        self.parent[a] = a;
    }
    pub fn into_parents(self) -> Vec<usize> {
        self.parent
    }
}

/// The faces `idx` (in rising order) in look-alike groups: joined while two groups are less than `cut` apart
/// on average. Largest group first, and of equally large ones the one whose first face comes first.
pub fn groups(e: Prints, idx: &[usize], cut: f64) -> Vec<Vec<usize>> {
    let m = idx.len();
    if m < 2 {
        return idx.iter().map(|i| vec![*i]).collect();
    }
    let long: Vec<f64> =
        idx.iter().map(|&i| e.of(i).iter().map(|v| *v as f64 * *v as f64).sum::<f64>().sqrt()).collect();
    // every pair's distance, a row a face: row a holds a's distances to the faces after it
    let mut rows: Vec<Vec<f64>> = (0..m).map(|a| vec![0.0; m - 1 - a]).collect();
    spread(&mut rows, crate::scores::threads(), 16, |first, part| {
        for (k, row) in part.iter_mut().enumerate() {
            let a = first + k;
            for (j, d) in row.iter_mut().enumerate() {
                let b = a + 1 + j;
                *d = apart(e.of(idx[a]), e.of(idx[b]), long[a], long[b]);
            }
        }
    });
    let at = |rows: &Vec<Vec<f64>>, a: usize, b: usize| if a < b { rows[a][b - a - 1] } else { rows[b][a - b - 1] };
    let mut size = vec![1usize; m];
    let mut sets = Sets::new(m);
    let mut chain: Vec<usize> = Vec::with_capacity(m);
    for _ in 0..m - 1 {
        if chain.is_empty() {
            chain.push((0..m).find(|i| size[*i] > 0).unwrap_or(0));
        }
        // from the chain's last group to its nearest, until two are each other's nearest
        let (x, y, near) = loop {
            let x = chain[chain.len() - 1];
            let (mut y, mut near) = match chain.len() {
                1 => (usize::MAX, f64::INFINITY),
                k => (chain[k - 2], at(&rows, x, chain[k - 2])),
            };
            for i in 0..m {
                if size[i] > 0 && i != x {
                    let d = at(&rows, x, i);
                    if d < near {
                        (near, y) = (d, i);
                    }
                }
            }
            if chain.len() > 1 && y == chain[chain.len() - 2] {
                break (x, y, near);
            }
            chain.push(y);
        };
        chain.truncate(chain.len() - 2);
        let (x, y) = (x.min(y), x.max(y));
        if near < cut {
            sets.join(x, y);
        }
        // the two as one group, in y's place: its distance to every other group is the average over its faces
        let (nx, ny) = (size[x] as f64, size[y] as f64);
        for i in 0..m {
            if size[i] > 0 && i != x && i != y {
                let d = (nx * at(&rows, i, x) + ny * at(&rows, i, y)) / (nx + ny);
                if i < y { rows[i][y - i - 1] = d } else { rows[y][i - y - 1] = d }
            }
        }
        size[y] += size[x];
        size[x] = 0;
    }
    let mut found: Vec<Vec<usize>> = Vec::new();
    let mut place: Vec<usize> = vec![usize::MAX; m];
    for a in 0..m {
        let r = sets.root(a);
        if place[r] == usize::MAX {
            place[r] = found.len();
            found.push(Vec::new());
        }
        found[place[r]].push(idx[a]);
    }
    found.sort_by_key(|g| std::cmp::Reverse(g.len())); // equally large ones stay in their order
    found
}

/// Every pair of faces whose prints are at least `same` alike, of the faces `rows` each with every face
/// (`later`: only with the faces after it, for when `rows` is all of them). A face without a print is alike
/// to none.
pub fn alike(e: Prints, rows: &[usize], later: bool, same: f32) -> Vec<(usize, usize)> {
    let n = e.len();
    let with_print: Vec<usize> = rows.iter().copied().filter(|r| e.has_print(*r)).collect();
    let mut found: Vec<Vec<(usize, usize)>> = vec![Vec::new(); with_print.len().div_ceil(TILE)];
    spread(&mut found, crate::scores::threads(), 2, |first, part| {
        for (t, pairs) in part.iter_mut().enumerate() {
            let tile = &with_print[(first + t) * TILE..((first + t + 1) * TILE).min(with_print.len())];
            let from = if later { tile[0] + 1 } else { 0 };
            let mut c0 = from;
            while c0 < n {
                let c1 = (c0 + TILE).min(n);
                for &r in tile {
                    let mine = e.of(r);
                    for c in c0.max(if later { r + 1 } else { 0 })..c1 {
                        if c != r && dot(mine, e.of(c)) >= same {
                            pairs.push((r, c));
                        }
                    }
                }
                c0 = c1;
            }
        }
    });
    found.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let long = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / long).collect()
    }

    /// Prints in a plane, by their angle in degrees: how unlike two are is one minus the cosine of the angle
    /// between them.
    fn at_angles(degrees: &[f32]) -> Vec<f32> {
        degrees.iter().flat_map(|d| unit(&[d.to_radians().cos(), d.to_radians().sin()])).collect()
    }

    #[test]
    fn faces_are_grouped_while_they_are_close_on_average() {
        // three bunches, 60 degrees apart (0.5 unlike), each a few degrees wide; and one face alone
        let emb = at_angles(&[0.0, 3.0, 61.0, 1.0, 120.0, 63.0, 122.0, 60.0, 200.0, 124.0, 62.0]);
        let e = Prints { emb: &emb, dim: 2 };
        let all: Vec<usize> = (0..11).collect();
        let found = groups(e, &all, 0.1);
        assert_eq!(found, [vec![2, 5, 7, 10], vec![0, 1, 3], vec![4, 6, 9], vec![8]]);
        // a wider cut-off joins the two bunches that are nearest (60 degrees: 0.5 apart). The third stays out:
        // 60 degrees from one of them and 120 from the other, it is 0.93 from the two together
        let found = groups(e, &all, 0.6);
        assert_eq!(found.iter().map(Vec::len).collect::<Vec<_>>(), [7, 3, 1]);
        assert_eq!(found[0], [0, 1, 2, 3, 5, 7, 10]);
        // only some of the faces: the others are in no group
        assert_eq!(groups(e, &[1, 2, 3, 8], 0.1), [vec![1, 3], vec![2], vec![8]]);
        assert_eq!(groups(e, &[4], 0.1), [vec![4]]);
        assert!(groups(e, &[], 0.1).is_empty());
    }

    #[test]
    fn two_groups_join_by_the_average_over_their_faces_not_by_their_nearest_two() {
        // 0 and 10 degrees, and 40 and 50: the nearest two are 30 degrees apart (0.134), the average over the
        // four pairs is 0.240 (40 degrees twice, 30 and 50)
        let emb = at_angles(&[0.0, 10.0, 40.0, 50.0]);
        let e = Prints { emb: &emb, dim: 2 };
        assert_eq!(groups(e, &[0, 1, 2, 3], 0.2).len(), 2, "not joined by the nearest two");
        assert_eq!(groups(e, &[0, 1, 2, 3], 0.25).len(), 1);
    }

    #[test]
    fn the_same_picture_again_is_found_among_all_faces_and_for_some_of_them() {
        let mut emb = at_angles(&[0.0, 90.0, 2.0, 180.0, 91.0, 300.0]);
        emb.extend([0.0, 0.0]); // a face without a print
        let e = Prints { emb: &emb, dim: 2 };
        let mut pairs = alike(e, &(0..7).collect::<Vec<_>>(), true, 0.95);
        pairs.sort_unstable();
        assert_eq!(pairs, [(0, 2), (1, 4)]);
        let mut pairs = alike(e, &[4, 5, 6], false, 0.95);
        pairs.sort_unstable();
        assert_eq!(pairs, [(4, 1)]);
        let mut sets = Sets::new(7);
        sets.join(4, 1);
        sets.join(2, 0);
        assert_eq!((sets.root(4), sets.root(2), sets.root(3)), (1, 0, 3), "a set's first face is its lowest");
    }
}
