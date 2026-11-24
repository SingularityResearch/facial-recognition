//! Prints behind the frames: the flow for devices where a face's print is slow to make (a phone, a Raspberry
//! Pi).
//!
//! The general flow makes every face's print in every frame, which a graphics processor does in a few
//! milliseconds. Where a print takes a tenth of a second or a whole one, a frame then waits for it, and with
//! every core busy everything else waits too. So here a frame never waits for a print:
//! - each frame finds the faces and reads each one's quality, as everywhere;
//! - a face is followed from frame to frame by where it is (its box against the boxes of the frame before);
//! - one thread makes prints behind the frames, one face at a time: a face new in view first, then each
//!   face again every `REFRESH` seconds, from its newest picture. The print itself is `Faces::embed`;
//! - a frame carries a face's new print once, when one was made since the last frame, and the face's name
//!   as its last print gave it (unknown until its first). The tracker page takes a frame without a print as
//!   it always has: it adds none to that face's average.
//!
//! What that costs: a face new in view has its name a moment later, not in the same frame, and its average
//! is built from fewer prints. No other system enters this: `wanted()` says where it runs.

use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use crate::{
    Result,
    faces::{DET_SCORE, Faces, MIN_FACE, align},
    geometry::iou,
    image::Bgr,
    yunet::Row,
};

/// seconds between two prints of the same face
pub const REFRESH: f64 = 1.5;
/// a face is the one followed when its box overlaps that one's last box at least this much
pub const SAME: f32 = 0.3;
/// seconds without being seen before a followed face is let go
pub const FORGET: f64 = 2.0;

/// Whether this device makes its prints behind the frames: a phone, or a Raspberry Pi with a 64-bit system.
/// `FACE_PRINTS=behind` runs it elsewhere (to try it, never by itself), `FACE_PRINTS=frame` turns it off.
pub fn wanted() -> bool {
    match std::env::var("FACE_PRINTS").unwrap_or_default().to_lowercase().as_str() {
        "behind" => true,
        "frame" => false,
        _ => cfg!(any(target_os = "android", target_os = "ios")) || raspberry_pi(),
    }
}

/// Whether this is a Raspberry Pi with a 64-bit system: what is done only there asks this, and nothing else does.
pub fn raspberry_pi() -> bool {
    cfg!(all(target_os = "linux", target_arch = "aarch64"))
        && std::fs::read("/proc/device-tree/model").is_ok_and(|m| m.starts_with(b"Raspberry Pi"))
}

/// A face of the last frame, as the frames' side hands it on.
pub struct Seen {
    pub row: Row,
    /// the face's print when one was made since the last frame, else None
    pub emb: Option<Vec<f32>>,
    pub crop: Bgr,
    pub quality: f32,
}

struct Followed {
    id: u64,
    area: [f32; 4],
    crop: Bgr,
    seen: f64,
    made: Option<f64>,
    emb: Option<Vec<f32>>,
    new: bool,
    name: String,
    sim: f32,
}

/// The faces in view and what is known of each: the part without models or threads.
#[derive(Default)]
struct Follow {
    followed: Vec<Followed>,
    /// the followed face of each face in the last frame, in its order
    frame: Vec<u64>,
    next_id: u64,
    made: u64,
}

impl Follow {
    /// A frame's faces (box and aligned crop each): the print each one carries in this frame.
    fn see(&mut self, now: f64, faces: Vec<([f32; 4], Bgr)>) -> Vec<Option<Vec<f32>>> {
        self.followed.retain(|f| now - f.seen <= FORGET);
        let mut free: Vec<u64> = self.followed.iter().map(|f| f.id).collect();
        self.frame.clear();
        let mut out = Vec::with_capacity(faces.len());
        for (area, crop) in faces {
            let best = free
                .iter()
                .filter_map(|id| self.followed.iter().find(|f| f.id == *id).map(|f| (f.id, iou(&area, &f.area))))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let id = match best {
                Some((id, overlap)) if overlap >= SAME => {
                    free.retain(|f| *f != id);
                    id
                }
                _ => {
                    // new in view: no print and no name yet
                    self.next_id += 1;
                    self.followed.push(Followed {
                        id: self.next_id,
                        area,
                        crop: Bgr::new(0, 0),
                        seen: now,
                        made: None,
                        emb: None,
                        new: false,
                        name: "Unknown".to_string(),
                        sim: 0.0,
                    });
                    self.next_id
                }
            };
            let f = self.followed.iter_mut().find(|f| f.id == id).expect("the followed face");
            (f.area, f.crop, f.seen) = (area, crop, now);
            out.push(if f.new { f.emb.clone() } else { None });
            f.new = false;
            self.frame.push(id);
        }
        out
    }

    /// The followed face whose print is to be made next, with its newest picture: one without a print first
    /// (the longest in view), else the one whose print is oldest and at least `refresh` old.
    fn due(&self, now: f64, refresh: f64) -> Option<(u64, Bgr)> {
        let live = || self.followed.iter().filter(|f| now - f.seen <= 1.0 && f.crop.w > 0);
        let pick = live().find(|f| f.made.is_none()).or_else(|| {
            live()
                .filter(|f| f.made.is_some_and(|m| now - m >= refresh))
                .min_by(|a, b| a.made.unwrap_or(0.0).total_cmp(&b.made.unwrap_or(0.0)))
        });
        pick.map(|f| (f.id, f.crop.clone()))
    }

    fn print_made(&mut self, id: u64, emb: Vec<f32>, now: f64) {
        if let Some(f) = self.followed.iter_mut().find(|f| f.id == id) {
            (f.emb, f.new, f.made) = (Some(emb), true, Some(now));
        }
        self.made += 1;
    }

    fn name(&mut self, k: usize, name: &str, sim: f32) {
        if let Some(id) = self.frame.get(k).copied() {
            if let Some(f) = self.followed.iter_mut().find(|f| f.id == id) {
                (f.name, f.sim) = (name.to_string(), sim);
            }
        }
    }

    fn named(&self, k: usize) -> (String, f32) {
        self.frame
            .get(k)
            .and_then(|id| self.followed.iter().find(|f| f.id == *id))
            .map_or_else(|| ("Unknown".to_string(), 0.0), |f| (f.name.clone(), f.sim))
    }
}

/// The faces of a camera's frames, with their prints made behind the frames.
pub struct Prints {
    faces: Arc<Faces>,
    follow: Mutex<Follow>,
    wake: Condvar,
    start: Instant,
}

impl Prints {
    /// Starts the thread that makes the prints.
    pub fn start(faces: Arc<Faces>) -> Arc<Prints> {
        let prints = Arc::new(Prints {
            faces,
            follow: Mutex::new(Follow::default()),
            wake: Condvar::new(),
            start: Instant::now(),
        });
        let behind = prints.clone();
        let _ = std::thread::Builder::new().name("prints behind the frames".into()).spawn(move || behind.run());
        prints
    }

    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// A frame's faces, as `Faces::detect(img, &[640])` finds them, except that a face's print is None in a
    /// frame that brought no new one: `named(k)` is then that face's name from its last print.
    pub fn find(&self, img: &Bgr) -> Result<Vec<Seen>> {
        let mut rows = self.faces.detect_at(img, 640, DET_SCORE, self.faces.pad())?; // one pass, as the live tracker asks
        rows.sort_by(|a, b| b[14].total_cmp(&a[14]));
        let mut kept: Vec<Row> = Vec::new();
        for r in rows {
            if kept.iter().all(|k| iou(&r, k) < 0.4) {
                kept.push(r);
            }
        }
        let mut found = Vec::new();
        for row in kept {
            if row[2] < MIN_FACE || row[3] < MIN_FACE {
                continue;
            }
            let crop = align(img, &row)?;
            let quality = self.faces.quality(&crop)?;
            found.push((row, crop, quality));
        }
        let seen = found.iter().map(|(row, crop, _)| ([row[0], row[1], row[2], row[3]], crop.clone())).collect();
        let carried = self.follow.lock().unwrap_or_else(|e| e.into_inner()).see(self.now(), seen);
        if !found.is_empty() {
            self.wake.notify_one();
        }
        Ok(found.into_iter().zip(carried).map(|((row, crop, quality), emb)| Seen { row, emb, crop, quality }).collect())
    }

    /// What the print of the last frame's k-th face was called: kept for the frames until its next print.
    pub fn name(&self, k: usize, name: &str, sim: f32) {
        self.follow.lock().unwrap_or_else(|e| e.into_inner()).name(k, name, sim);
    }

    pub fn named(&self, k: usize) -> (String, f32) {
        self.follow.lock().unwrap_or_else(|e| e.into_inner()).named(k)
    }

    /// Prints made so far.
    pub fn made(&self) -> u64 {
        self.follow.lock().unwrap_or_else(|e| e.into_inner()).made
    }

    fn run(&self) {
        loop {
            let due = self.follow.lock().unwrap_or_else(|e| e.into_inner()).due(self.now(), REFRESH);
            match due {
                Some((id, crop)) => match self.faces.embed(&crop) {
                    Ok(emb) => self.follow.lock().unwrap_or_else(|e| e.into_inner()).print_made(id, emb, self.now()),
                    Err(e) => {
                        // a print that failed: the face keeps its last one
                        eprintln!("Prints behind the frames: {e}");
                        std::thread::sleep(Duration::from_secs(1));
                    }
                },
                None => {
                    let guard = self.follow.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = self.wake.wait_timeout(guard, Duration::from_millis(50));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(x: f32) -> ([f32; 4], Bgr) {
        ([x, 100.0, 80.0, 80.0], Bgr::new(112, 112))
    }

    #[test]
    fn a_frame_never_waits_and_carries_each_new_print_once() {
        let mut f = Follow::default();
        assert_eq!(f.see(0.0, vec![face(100.0)]), [None], "a face new in view has no print yet");
        assert_eq!(f.named(0), ("Unknown".to_string(), 0.0));
        let (id, _) = f.due(0.1, REFRESH).expect("its print is due at once");
        f.print_made(id, vec![1.0, 0.0], 0.6);
        assert_eq!(f.see(0.7, vec![face(104.0)]), [Some(vec![1.0, 0.0])], "the frame after the print carries it");
        f.name(0, "Ann", 0.71);
        assert_eq!(f.see(0.8, vec![face(108.0)]), [None], "and the next frame doesn't carry it again");
        assert_eq!(f.named(0), ("Ann".to_string(), 0.71), "but the face keeps the name its print gave");
        assert_eq!(f.made, 1);
    }

    #[test]
    fn a_face_new_in_view_comes_first_then_the_oldest_print() {
        let mut f = Follow::default();
        f.see(0.0, vec![face(100.0)]);
        let (first, _) = f.due(0.0, REFRESH).unwrap();
        f.print_made(first, vec![1.0], 0.5);
        assert!(f.due(0.6, REFRESH).is_none(), "its print is new: nothing is due");
        f.see(1.0, vec![face(100.0), face(400.0)]);
        let (second, _) = f.due(1.0, REFRESH).unwrap();
        assert_ne!(second, first, "the face that just came into view is first");
        f.print_made(second, vec![2.0], 1.5);
        f.see(2.1, vec![face(100.0), face(400.0)]);
        assert_eq!(
            f.due(2.1, REFRESH).unwrap().0,
            first,
            "then the face whose print is oldest, once it is REFRESH old"
        );
    }

    #[test]
    fn faces_are_followed_by_where_they_are() {
        let mut f = Follow::default();
        f.see(0.0, vec![face(100.0), face(400.0)]);
        f.name(0, "Ann", 0.7);
        f.name(1, "Bo", 0.6);
        // the next frame finds them in the other order: each keeps its own name
        f.see(0.1, vec![face(398.0), face(103.0)]);
        assert_eq!(f.named(0).0, "Bo");
        assert_eq!(f.named(1).0, "Ann");
        // a face somewhere else entirely is somebody new
        f.see(0.2, vec![face(700.0)]);
        assert_eq!(f.named(0).0, "Unknown");
        assert_eq!(f.followed.len(), 3);
    }

    #[test]
    fn a_face_that_left_is_forgotten() {
        let mut f = Follow::default();
        f.see(0.0, vec![face(100.0)]);
        f.name(0, "Ann", 0.7);
        f.see(1.0, vec![]);
        assert_eq!(f.followed.len(), 1, "gone for a second: still followed");
        assert!(f.due(1.5, REFRESH).is_none(), "but no print is made of a face that isn't in view");
        f.see(FORGET + 0.1, vec![]);
        assert!(f.followed.is_empty());
        f.see(3.0, vec![face(100.0)]);
        assert_eq!(f.named(0).0, "Unknown", "back after that, it is new in view");
        assert_eq!(f.named(5), ("Unknown".to_string(), 0.0));
    }
}
