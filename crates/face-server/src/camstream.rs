//! This device's live camera as the tracker page shows it, for the other devices to watch:
//! the page hands over a JPEG frame many times a second, the peer service
//! (`peer.rs`) serves the newest to whoever watches. Frames are kept in memory only, never on disk.

use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime},
};

use serde_json::{Value, json};

/// a watcher counts this long after its last frame
const WATCH_WINDOW: Duration = Duration::from_secs(5);
/// a frame older than this means the camera isn't streaming
const FRESH: Duration = Duration::from_secs(10);
/// bytes in one frame from the page
pub const MAX_FRAME: usize = 3_000_000;
/// bytes of a readout's JSON
pub const MAX_READOUT: usize = 200_000;
/// bytes of what goes with a readout for the other devices' Signals chart: the device's readings of the last
/// seconds, every one with its own time (at 30 checks a second and three faces about 60,000)
pub const MAX_DEVICE_SIGNALS: usize = 150_000;

struct State {
    seq: u64,
    jpeg: Option<Arc<Vec<u8>>>,
    at: Option<Instant>,
    /// watcher id -> the last time it got a frame
    watchers: HashMap<String, Instant>,
    /// the tracker page's live readout of the faces in view, as data
    faces: Value,
    faces_at: Option<Instant>,
    paused: f64,
    /// what the device itself measures (its camera's checks, what its audio interfaces heard), or null
    signals: Value,
}

pub struct Stream {
    state: Mutex<State>,
    news: Condvar,
    /// told of this camera's frames too: the Cameras page waits for any camera's next one
    pub any: Arc<Any>,
}

/// who this device's camera saw, and when (one camera a process)
static IN_VIEW: Mutex<Vec<(String, std::time::SystemTime)>> = Mutex::new(Vec::new());

/// The names the camera sees now (the tracker page says, every second): named people only.
pub fn set_in_view(names: &[Value]) {
    let now = std::time::SystemTime::now();
    let mut seen = IN_VIEW.lock().unwrap_or_else(|e| e.into_inner());
    for name in names.iter().filter_map(Value::as_str) {
        if name.is_empty() || name == "Unknown" || name.starts_with('#') || name.chars().count() > 100 {
            continue;
        }
        match seen.iter_mut().find(|(n, _)| n == name) {
            Some(known) => known.1 = now,
            None => seen.push((name.to_string(), now)),
        }
    }
}

/// {name: when last seen, in seconds since 1970} of the people the camera saw within the last `within`
/// seconds. Who wasn't seen for an hour is forgotten.
pub fn in_view(within: f64) -> serde_json::Map<String, Value> {
    let age = |at: &std::time::SystemTime| at.elapsed().map_or(0.0, |d| d.as_secs_f64());
    let mut seen = IN_VIEW.lock().unwrap_or_else(|e| e.into_inner());
    seen.retain(|(_, at)| age(at) <= 3600.0);
    (seen.iter().filter(|(_, at)| age(at) <= within))
        .map(|(name, at)| {
            let when = at.duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
            (name.clone(), json!(when))
        })
        .collect()
}

/// Any camera's new frame, this device's or a relayed one (`camrelay.rs`): the Cameras page asks for every
/// camera's next frame in one request, which waits on this.
pub struct Any {
    lock: Mutex<()>,
    came: Condvar,
}

impl Any {
    pub fn new() -> Arc<Any> {
        Arc::new(Any { lock: Mutex::new(()), came: Condvar::new() })
    }

    /// Wake whoever waits for any camera's next frame. Called without another camera's lock held.
    pub fn news(&self) {
        let _held = self.hold();
        self.came.notify_all();
    }

    /// Held while looking at the cameras, so that a frame arriving meanwhile still wakes the wait.
    pub fn hold(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn wait<'a>(&self, held: MutexGuard<'a, ()>, at_most: Duration) -> MutexGuard<'a, ()> {
        self.came.wait_timeout(held, at_most).unwrap_or_else(|e| e.into_inner()).0
    }
}

fn tenths(seconds: f64) -> f64 {
    (seconds * 10.0).round() / 10.0
}

impl Stream {
    pub fn new() -> Arc<Stream> {
        Arc::new(Stream {
            state: Mutex::new(State {
                seq: 0,
                jpeg: None,
                at: None,
                watchers: HashMap::new(),
                faces: json!([]),
                faces_at: None,
                paused: 0.0,
                signals: Value::Null,
            }),
            news: Condvar::new(),
            any: Any::new(),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new frame from the tracker page. How many watch now.
    pub fn put(&self, jpeg: &[u8]) -> Result<usize, String> {
        if !jpeg.starts_with(&[0xff, 0xd8]) || jpeg.len() > MAX_FRAME {
            return Err("expected a JPEG frame".to_string());
        }
        {
            let mut s = self.state();
            if let Some(gap) = s.at.map(|t| t.elapsed().as_secs_f64()) {
                if gap > 2.0 && gap < 600.0 {
                    println!("Camera: no frame from the tracker page for {gap:.1} s");
                }
            }
            s.seq += 1;
            s.jpeg = Some(Arc::new(jpeg.to_vec()));
            s.at = Some(Instant::now());
        }
        self.news.notify_all();
        self.any.news();
        Ok(self.watching())
    }

    /// (number, frame) of the newest fresh frame when it is newer than `after`, else (after, None); never
    /// waits. Notes the watcher.
    pub fn peek(&self, after: u64, watcher: &str) -> (u64, Option<Arc<Vec<u8>>>) {
        let mut s = self.state();
        s.watchers.insert(watcher.to_string(), Instant::now());
        // a number from before this program started again (a Cameras page left open): any frame is new to it
        let after = if after > s.seq { 0 } else { after };
        if s.seq > after && s.jpeg.is_some() && s.at.is_some_and(|t| t.elapsed() < FRESH) {
            (s.seq, s.jpeg.clone())
        } else {
            (after, None)
        }
    }

    /// (number, frame) of the newest fresh frame, or (number, None).
    pub fn latest(&self) -> (u64, Option<Arc<Vec<u8>>>) {
        let s = self.state();
        let fresh = s.at.is_some_and(|t| t.elapsed() < FRESH);
        (s.seq, if fresh { s.jpeg.clone() } else { None })
    }

    /// The next frame after number `after`, waited for up to `timeout`: (number, frame), the frame None when
    /// none came. Notes the watcher.
    pub fn wait_next(&self, after: u64, watcher: &str, timeout: Duration) -> (u64, Option<Arc<Vec<u8>>>) {
        let end = Instant::now() + timeout;
        let mut s = self.state();
        s.watchers.insert(watcher.to_string(), Instant::now());
        while s.seq <= after || s.jpeg.is_none() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return (after, None);
            }
            s = self.news.wait_timeout(s, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        s.watchers.insert(watcher.to_string(), Instant::now());
        (s.seq, s.jpeg.clone())
    }

    /// How many devices watch now.
    pub fn watching(&self) -> usize {
        let mut s = self.state();
        s.watchers.retain(|_, t| t.elapsed() <= WATCH_WINDOW);
        s.watchers.len()
    }

    /// True when the page sent a frame recently: the camera is on and the page streams.
    pub fn streaming(&self) -> bool {
        let s = self.state();
        s.jpeg.is_some() && s.at.is_some_and(|t| t.elapsed() < FRESH)
    }

    /// The live readout of the faces in view, from the tracker page: a list of plain values, checked for
    /// its size. `paused`: for how many seconds the camera has sent no new picture though it is on. `signals`:
    /// what the device itself measures, for the other devices' Signals chart: an object, passed on as it is
    /// (the page that draws it checks every value), or nothing.
    pub fn set_readout(&self, faces: &Value, paused: Option<&Value>, signals: Option<&Value>) -> Result<(), String> {
        let bad = || "bad readout".to_string();
        let list = faces.as_array().ok_or_else(bad)?;
        if list.len() > 50 || faces.to_string().len() > MAX_READOUT {
            return Err(bad());
        }
        let paused = match paused {
            None | Some(Value::Null) => 0.0,
            Some(Value::Number(n)) => n.as_f64().filter(|p| (0.0..1e9).contains(p)).ok_or_else(bad)?,
            Some(_) => return Err(bad()),
        };
        let signals = match signals {
            None | Some(Value::Null) => Value::Null,
            Some(s) if s.is_object() && s.to_string().len() <= MAX_DEVICE_SIGNALS => s.clone(),
            Some(_) => return Err(bad()),
        };
        let mut s = self.state();
        s.signals = signals;
        s.faces = faces.clone();
        s.faces_at = Some(Instant::now());
        s.paused = tenths(paused);
        Ok(())
    }

    /// {faces, age, frameAge}: the newest readout (no faces when it is older than `FRESH`), and how old the
    /// newest frame is. Also `paused` (seconds) while the camera is on but sends no new picture, and `signals`
    /// when the page gave the device's own with a readout that is still new.
    pub fn readout(&self) -> Value {
        let s = self.state();
        // a readout never given is as old as can be: since 1970
        let age = s.faces_at.map_or_else(
            || SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()),
            |t| t.elapsed().as_secs_f64(),
        );
        let fresh = age < FRESH.as_secs_f64();
        let frame_age = match (&s.jpeg, s.at) {
            (Some(_), Some(t)) => json!(tenths(t.elapsed().as_secs_f64())),
            _ => Value::Null,
        };
        let mut out = json!({"faces": if fresh { s.faces.clone() } else { json!([]) }, "age": tenths(age), "frameAge": frame_age});
        if s.paused > 0.0 && fresh {
            out["paused"] = json!(s.paused);
        }
        if fresh && !s.signals.is_null() {
            out["signals"] = s.signals.clone();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 0xff, 0xd9];

    #[test]
    fn the_newest_frame_is_kept_for_whoever_watches() {
        let s = Stream::new();
        assert_eq!(s.latest(), (0, None));
        assert!(!s.streaming());
        assert_eq!(s.wait_next(0, "a", Duration::from_millis(20)), (0, None), "none came");
        assert_eq!(s.put(JPEG), Ok(1), "the one that just waited counts as watching");
        assert!(s.streaming());
        let (n, frame) = s.latest();
        assert_eq!((n, frame.as_deref().map(Vec::as_slice)), (1, Some(JPEG)));
        // a watcher that has frame 1 waits for the next, which another thread hands over
        let waiting = std::thread::spawn({
            let s = s.clone();
            move || s.wait_next(1, "a", Duration::from_secs(5))
        });
        std::thread::sleep(Duration::from_millis(30));
        s.put(&[JPEG, &[9]].concat()).unwrap();
        let (n, frame) = waiting.join().unwrap();
        assert_eq!((n, frame.map(|f| f.len())), (2, Some(JPEG.len() + 1)));
        assert_eq!(s.watching(), 1);
        // the Cameras page's way of asking: never waits, and a number from another run counts as none
        assert_eq!(s.peek(2, "page"), (2, None));
        assert_eq!(s.peek(1, "page").0, 2);
        assert_eq!(s.peek(99, "page").0, 2);
        assert_eq!(s.watching(), 2);
    }

    #[test]
    fn what_is_not_a_frame_is_refused() {
        let s = Stream::new();
        assert!(s.put(b"<html>").is_err());
        assert!(s.put(&[]).is_err());
        assert!(s.put(&[&[0xff, 0xd8][..], &vec![0; MAX_FRAME]].concat()).is_err(), "too large");
        assert_eq!(s.latest(), (0, None));
    }

    #[test]
    fn the_readout_is_checked_and_goes_out_with_its_age() {
        let s = Stream::new();
        let none = s.readout();
        assert_eq!((none["faces"].clone(), none["frameAge"].clone()), (json!([]), Value::Null));
        assert!(none["age"].as_f64().unwrap() > 1e9, "never given: as old as can be");
        s.set_readout(&json!([{"name": "A", "sim": 0.6}]), Some(&json!(2.34)), None).unwrap();
        assert!(s.readout().get("signals").is_none(), "none given: the answer is as it always was");
        s.put(JPEG).unwrap();
        let r = s.readout();
        assert_eq!(r["faces"], json!([{"name": "A", "sim": 0.6}]));
        assert!(r["age"].as_f64().unwrap() < 1.0 && r["frameAge"].as_f64().unwrap() < 1.0);
        assert_eq!(r["paused"], json!(2.3));
        s.set_readout(&json!([]), None, Some(&json!({"device": {"checks": 9.5}}))).unwrap();
        assert!(s.readout().get("paused").is_none(), "its picture arrives again");
        assert_eq!(s.readout()["signals"], json!({"device": {"checks": 9.5}}), "the device's own signals go with it");
        s.set_readout(&json!([]), None, Some(&Value::Null)).unwrap();
        assert!(s.readout().get("signals").is_none(), "and are gone when the page sends none");
        for signals in [json!([1]), json!("x"), json!({"x": "y".repeat(MAX_DEVICE_SIGNALS)})] {
            assert!(
                s.set_readout(&json!([]), None, Some(&signals)).is_err(),
                "signals that are no object, or too large"
            );
        }
        for (faces, paused) in [
            (json!({"not": "a list"}), None),
            (json!(vec![1; 51]), None),
            (json!(["x".repeat(MAX_READOUT)]), None),
            (json!([]), Some(json!(-1))),
            (json!([]), Some(json!(true))),
            (json!([]), Some(json!("3"))),
        ] {
            assert!(s.set_readout(&faces, paused.as_ref(), None).is_err(), "{paused:?}");
        }
    }
}
