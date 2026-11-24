//! This device's sound for the tracker page's Signals chart: every audio interface (each output: what the
//! computer itself plays; each microphone: what sounds in the room) listened to a few seconds at a time, one
//! stretch after another, as the music log listens (`listen::listen_all`), and of each listening only numbers
//! and one word kept: the newest alone, in memory. The sound itself is measured and dropped; nothing is
//! written anywhere.
//!
//! The owner's word for the chart (2026-11-22): "the fastest signal polling we can get from the devices we
//! have". So the stretches follow each other without a wait, and each is as short as its measures allow
//! (`LISTEN_FOR`): the music log's 8 seconds every half minute stay as they are, for a song's tempo and key.
//!
//! It listens only while the page asks. The page asks every few seconds while its switch is on (off until
//! the owner turns it on), each question keeps the listening going for `LEASE` more, and without one it
//! stops by itself: a switch turned off or a page that is closed leaves no microphone listened to.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime},
};

use serde_json::{Value, json};

use crate::{listen, sound::Sounds, soundheard::numbers};

/// Seconds of one listening. The background's numbers and the sound-recognition model need two; a tempo
/// wants a few beats more. A choice, not a measured value.
pub const LISTEN_FOR: f64 = 4.0;

/// how long one question keeps the listening going (the page asks every 2 s)
pub const LEASE: Duration = Duration::from_secs(12);

/// What listens once: made anew in the listening's own thread each time it starts, so that what it holds
/// (the sound-recognition model) never changes threads.
type Hear = Box<dyn FnMut() -> Vec<Value>>;
type Make = Box<dyn Fn() -> Hear + Send + Sync>;

struct State {
    /// asked for until then
    until: Option<Instant>,
    running: bool,
    /// the number of the newest listening (0: none since the listening last began)
    seq: u64,
    listened: u64,
    /// when it ended, ms since 1970
    at: u64,
    interfaces: Vec<Value>,
}

pub struct Live {
    make: Make,
    every: Duration,
    lease: Duration,
    state: Mutex<State>,
}

impl Live {
    /// `make` gives what listens once; a listening every `every` at most (one that takes as long is followed
    /// by the next at once), for as long as one was asked for within `lease`.
    pub fn new(make: Make, every: Duration, lease: Duration) -> Arc<Live> {
        let state = State { until: None, running: false, seq: 0, listened: 0, at: 0, interfaces: Vec::new() };
        Arc::new(Live { make, every, lease, state: Mutex::new(state) })
    }

    /// For this computer: every output and every microphone, one stretch of `LISTEN_FOR` after another, with
    /// the sound-recognition model where it is among the models (without it only the background's numbers).
    pub fn of(root: &Path) -> Arc<Live> {
        let root = root.to_path_buf();
        let make: Make = Box::new(move || {
            let (root, mut model) = (root.clone(), None::<Sounds>);
            Box::new(move || {
                if model.is_none() && Sounds::file(&root).is_some() {
                    model = Sounds::load(&root);
                }
                listen::listen_all(LISTEN_FOR, &["output", "input"], model.as_ref())
            })
        });
        Live::new(make, Duration::from_secs_f64(LISTEN_FOR), LEASE)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The page asks: the listening goes on for `lease` more (and begins when it wasn't going), and the
    /// answer is the newest listening: {available, listening, seq, at, interfaces}. seq 0: none yet.
    pub fn ask(self: &Arc<Self>) -> Value {
        let mut s = self.state();
        s.until = Some(Instant::now() + self.lease);
        if !s.running {
            s.running = true;
            let me = self.clone();
            if std::thread::Builder::new().name("sound-live".into()).spawn(move || me.run()).is_err() {
                s.running = false;
                return json!({"available": false, "why": "This device couldn't start listening."});
            }
        }
        json!({"available": true, "listening": true, "seq": s.seq, "at": s.at, "interfaces": s.interfaces})
    }

    /// Whether anyone listens now (for the tests, and for whoever wants to say so).
    pub fn listening(&self) -> bool {
        self.state().running
    }

    /// Still asked for? When not, the listening is over: said under the same lock a question takes, so a
    /// question that comes at that moment starts it again instead of finding it "running".
    fn wanted(&self) -> bool {
        let mut s = self.state();
        if s.until.is_some_and(|until| Instant::now() < until) {
            return true;
        }
        s.running = false;
        s.seq = 0;
        s.interfaces.clear();
        false
    }

    fn run(self: Arc<Self>) {
        let mut hear = (self.make)();
        while self.wanted() {
            let began = Instant::now();
            let interfaces: Vec<Value> = hear().iter().map(numbers).collect();
            {
                let mut s = self.state();
                s.listened += 1;
                s.seq = s.listened;
                s.at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                s.interfaces = interfaces;
            }
            // until the next listening, giving up as soon as nobody asks any more
            while began.elapsed() < self.every {
                if !self.wanted() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100).min(self.every));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn it_listens_only_while_it_is_asked() {
        let heard = Arc::new(AtomicUsize::new(0));
        let count = heard.clone();
        let make: Make = Box::new(move || {
            let count = count.clone();
            Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                vec![
                    json!({"name": "Microphone", "kind": "input", "what": "speech", "sure": 0.8, "ambient": {"level": -30.0}}),
                ]
            })
        });
        let live = Live::new(make, Duration::from_millis(40), Duration::from_millis(250));
        assert!(!live.listening() && heard.load(Ordering::SeqCst) == 0, "nobody asked: nothing is listened to");
        let first = live.ask();
        assert_eq!((first["available"].as_bool(), first["listening"].as_bool()), (Some(true), Some(true)));
        // asked again and again: it goes on, and the newest listening is the answer
        let mut newest = json!(null);
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(30));
            newest = live.ask();
        }
        assert!(newest["seq"].as_u64().unwrap() >= 2, "{newest}");
        assert!(newest["at"].as_u64().unwrap() > 1_600_000_000_000);
        assert_eq!(newest["interfaces"][0]["what"], "speech");
        assert_eq!(newest["interfaces"][0]["level"], json!(-30.0));
        // nobody asks any more: it stops by itself, and what it heard is gone
        std::thread::sleep(Duration::from_millis(600));
        assert!(!live.listening(), "the listening ended with the asking");
        let after = heard.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(heard.load(Ordering::SeqCst), after, "and nothing listens after it");
        // a new question begins it again, with nothing of the earlier listening
        let again = live.ask();
        assert_eq!(again["seq"], json!(0));
        assert!(again["interfaces"].as_array().unwrap().is_empty());
        std::thread::sleep(Duration::from_millis(120));
        assert!(live.ask()["seq"].as_u64().unwrap() > newest["seq"].as_u64().unwrap(), "its numbers go on, never back");
    }
}
