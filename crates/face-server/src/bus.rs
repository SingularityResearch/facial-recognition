//! The event bus of one node (the owner's design, 2026-11-23): "all datapoints should be broadcast to secured
//! subscribers", "all nodes that currently authenticate should share a common API", "everything that a device
//! publishes (its settings) should be able to be securely published and subscribed to", and "the app should
//! not be dependent on a centralized master".
//!
//! What a node publishes is one JSON tree: what it is, every source and signal it has (the Signals chart's
//! sections), what is enabled and what is not, its settings, and its readings. A topic is a path in that
//! tree ("catalog", "state/off", "readings"). `set` puts a value at a path; each change gets the next number
//! and is kept for a while, so a subscriber asks for everything after the last number it saw (`since`): the
//! whole tree the first time, or when it fell too far behind, and only the changes after that. A subscriber
//! with nothing new to hear waits here until there is something: nothing is polled.
//!
//! The same struct is the copy a node keeps of another node's tree (`apply`: what that node's own bus
//! answered, taken in with its numbers), so this node's pages subscribe to any node the same way, through
//! their own server.
//!
//! Nothing is stored: the tree and the last changes are in memory, and gone with the program. Every node is
//! the only authority for its own tree; no node holds another's but as such a copy.

use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime},
};

use serde_json::{Map, Value, json};

/// bytes of one published value, as JSON: more is refused
pub const MAX_VALUE: usize = 400_000;
/// bytes of a whole tree, as JSON
pub const MAX_TREE: usize = 2_000_000;
/// changes kept for subscribers that are a little behind
const MAX_LOG: usize = 256;
const MAX_LOG_BYTES: usize = 4_000_000;
const MAX_PATH: usize = 200;
const MAX_DEPTH: usize = 8;
/// paths a subscriber may name
pub const MAX_PREFIXES: usize = 32;

struct Event {
    seq: u64,
    /// when it was published, by the publisher's clock (milliseconds since 1970, UTC)
    t: u64,
    path: String,
    value: Value,
    size: usize,
    /// said and not kept: a subscriber hears it, the tree never holds it (a reading, who is in view now)
    once: bool,
}

struct Inner {
    /// one run of the publishing program: numbers of another run mean nothing here
    epoch: String,
    seq: u64,
    tree: Map<String, Value>,
    tree_bytes: usize,
    log: VecDeque<Event>,
    log_bytes: usize,
}

pub struct Bus {
    inner: Mutex<Inner>,
    news: Condvar,
}

/// Where a subscriber is: the publisher's run and the last change it saw of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub epoch: String,
    pub seq: u64,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn new_epoch() -> String {
    let mut bytes = [0u8; 8];
    // (if the system gives no random bytes, the time tells two runs apart well enough)
    if ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).is_err() {
        bytes = now_ms().to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A path's parts, or None when it isn't one: a few names with `/` between, no empty one, nothing unprintable.
pub fn parts(path: &str) -> Option<Vec<&str>> {
    if path.is_empty() || path.len() > MAX_PATH {
        return None;
    }
    let parts: Vec<&str> = path.split('/').collect();
    let fine = |p: &&str| !p.is_empty() && !p.chars().any(char::is_control);
    (parts.len() <= MAX_DEPTH && parts.iter().all(fine)).then_some(parts)
}

/// The paths a request names ("a,b/c"), checked: at most `MAX_PREFIXES`, each a path. None when one isn't.
pub fn prefixes(text: &str) -> Option<Vec<String>> {
    let named: Vec<&str> = text.split(',').filter(|p| !p.is_empty()).collect();
    if named.len() > MAX_PREFIXES {
        return None;
    }
    let mut all: Vec<String> = named.iter().map(|p| parts(p).map(|_| p.to_string())).collect::<Option<_>>()?;
    // each path once, and none that lies under another: a change is then heard once, not once for every way
    // a subscriber named it
    all.sort();
    all.dedup();
    let under = |p: &str| all.iter().any(|q| p.strip_prefix(q.as_str()).is_some_and(|rest| rest.starts_with('/')));
    Some(all.iter().filter(|p| !under(p)).cloned().collect())
}

/// How long a tree is as text, counted without writing it anywhere.
fn written(tree: &Map<String, Value>) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    match serde_json::to_writer(&mut count, tree) {
        Ok(()) => count.0,
        Err(_) => usize::MAX, // (what can't be written can't be published either)
    }
}

fn lookup<'a>(tree: &'a Map<String, Value>, parts: &[&str]) -> Option<&'a Value> {
    let (first, rest) = parts.split_first()?;
    let mut at = tree.get(*first)?;
    for p in rest {
        at = at.as_object()?.get(*p)?;
    }
    Some(at)
}

/// Put a value at a path (null: take the branch away); the branch that was there before.
fn place(tree: &mut Map<String, Value>, parts: &[&str], value: Value) -> Option<Value> {
    let (last, above) = parts.split_last()?;
    let mut at = tree;
    for p in above {
        let next = at.entry(p.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new()); // a value where a branch is now: the branch takes its place
        }
        at = next.as_object_mut()?;
    }
    if value.is_null() { at.remove(*last) } else { at.insert(last.to_string(), value) }
}

/// What of a change at `path` a subscriber to `prefix` hears: the change itself when it lies at or under the
/// prefix, the part of it under the prefix when the change is of a branch above, else nothing.
fn narrowed(path: &str, value: &Value, prefix: &str) -> Option<(String, Value)> {
    if path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/')) {
        return Some((path.to_string(), value.clone()));
    }
    let under = prefix.strip_prefix(path).and_then(|rest| rest.strip_prefix('/'))?;
    let mut at = value;
    for p in under.split('/') {
        match at.as_object().and_then(|o| o.get(p)) {
            Some(next) => at = next,
            None => return Some((prefix.to_string(), Value::Null)), // the branch above was replaced by one without it
        }
    }
    Some((prefix.to_string(), at.clone()))
}

impl Inner {
    fn first(&self) -> u64 {
        self.log.front().map_or(self.seq + 1, |e| e.seq)
    }

    fn whole(&self, prefixes: &[String]) -> Value {
        if prefixes.is_empty() {
            return Value::Object(self.tree.clone());
        }
        let mut out = Map::new();
        for prefix in prefixes {
            let Some(parts) = parts(prefix) else { continue };
            if let Some(v) = lookup(&self.tree, &parts) {
                place(&mut out, &parts, v.clone());
            }
        }
        Value::Object(out)
    }

    fn log_push(&mut self, e: Event) {
        self.log_bytes += e.size;
        self.log.push_back(e);
        while self.log.len() > MAX_LOG || self.log_bytes > MAX_LOG_BYTES {
            match self.log.pop_front() {
                Some(old) => self.log_bytes -= old.size,
                None => break,
            }
        }
    }

    /// Put a checked value in the tree; None, and the tree as it was, when the tree would then be larger than
    /// one may be. A sum kept by adding and taking away drifted both ways (a branch taking a value's place, the
    /// names of a path, a branch taken away), so the sum kept here only ever adds, and by no less than a
    /// change can make the tree grow (its value and every name of its path with what stands round a name): it
    /// can say "too large" too early and never too late. When it does, the tree's real size as text is
    /// counted, the sum set to it, and the change refused only if the tree is really too large then.
    fn put(&mut self, parts: &[&str], value: &Value, size: usize) -> Option<()> {
        let top = *parts.first()?;
        let grown = self.tree_bytes.saturating_add(size + parts.iter().map(|p| p.len() + 6).sum::<usize>());
        if grown <= MAX_TREE {
            place(&mut self.tree, parts, value.clone());
            self.tree_bytes = grown;
            return Some(());
        }
        let before = self.tree.get(top).cloned();
        place(&mut self.tree, parts, value.clone());
        let real = written(&self.tree);
        if real > MAX_TREE {
            match before {
                Some(branch) => self.tree.insert(top.to_string(), branch),
                None => self.tree.remove(top),
            };
            self.tree_bytes = MAX_TREE; // (as full as a tree gets: the next change is counted again)
            return None;
        }
        self.tree_bytes = real;
        Some(())
    }
}

impl Default for Bus {
    fn default() -> Self {
        Bus::new()
    }
}

impl Bus {
    pub fn new() -> Bus {
        Bus {
            inner: Mutex::new(Inner {
                epoch: new_epoch(),
                seq: 0,
                tree: Map::new(),
                tree_bytes: 0,
                log: VecDeque::new(),
                log_bytes: 0,
            }),
            news: Condvar::new(),
        }
    }

    fn held(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Publish: put a value at a path of this node's tree (null takes the branch away). Whether anything
    /// changed: a value that is there already is no news.
    pub fn set(&self, path: &str, value: Value) -> Result<bool, String> {
        let parts = parts(path).ok_or("not a path")?;
        let size = value.to_string().len();
        if size > MAX_VALUE {
            return Err("too large to publish".to_string());
        }
        let mut inner = self.held();
        if lookup(&inner.tree, &parts) == Some(&value) || (value.is_null() && lookup(&inner.tree, &parts).is_none()) {
            return Ok(false);
        }
        inner.put(&parts, &value, size).ok_or("this node publishes too much")?;
        inner.seq += 1;
        let seq = inner.seq;
        inner.log_push(Event { seq, t: now_ms(), path: path.to_string(), value, size, once: false });
        drop(inner);
        self.news.notify_all();
        Ok(true)
    }

    /// Say something once: every subscriber that is there hears it under `path` (one that fell a little
    /// behind too), and the tree never holds it. For what is true only for a moment: a batch of readings,
    /// who is in view now. A subscriber that comes later is not given the last one as if it were now.
    pub fn say(&self, path: &str, value: Value) -> Result<(), String> {
        parts(path).ok_or("not a path")?;
        let size = value.to_string().len();
        if size > MAX_VALUE {
            return Err("too large to publish".to_string());
        }
        let mut inner = self.held();
        inner.seq += 1;
        let seq = inner.seq;
        inner.log_push(Event { seq, t: now_ms(), path: path.to_string(), value, size, once: true });
        drop(inner);
        self.news.notify_all();
        Ok(())
    }

    /// The value at a path now.
    pub fn get(&self, path: &str) -> Option<Value> {
        lookup(&self.held().tree, &parts(path)?).cloned()
    }

    pub fn cursor(&self) -> Cursor {
        let inner = self.held();
        Cursor { epoch: inner.epoch.clone(), seq: inner.seq }
    }

    /// Subscribe, one answer at a time: everything after `after` under `prefixes` (none: the whole tree).
    /// {epoch, seq, whole} the first time, after another run of the publisher, or when the subscriber fell
    /// further behind than the changes kept; else {epoch, seq, events: [{seq, t, path, value}]}, waiting up
    /// to `wait` for the first when there is none yet. The subscriber asks again with the answer's epoch and
    /// seq.
    pub fn since(&self, after: Option<&Cursor>, prefixes: &[String], wait: Duration) -> Value {
        let end = Instant::now() + wait;
        let mut inner = self.held();
        // (changes that were none of the subscriber's are passed over while it waits: its place moves on)
        let mut passed: Option<u64> = None;
        loop {
            let behind = match after {
                Some(c) if c.epoch == inner.epoch && c.seq <= inner.seq && c.seq + 1 >= inner.first() => {
                    Some(passed.map_or(c.seq, |p| p.max(c.seq)))
                }
                _ => None,
            };
            let Some(seen) = behind else {
                return json!({"epoch": inner.epoch, "seq": inner.seq, "whole": inner.whole(prefixes)});
            };
            if seen < inner.seq {
                let mut events = Vec::new();
                for e in inner.log.iter().filter(|e| e.seq > seen) {
                    let heard: Vec<(String, Value)> = if prefixes.is_empty() {
                        vec![(e.path.clone(), e.value.clone())]
                    } else {
                        prefixes.iter().filter_map(|p| narrowed(&e.path, &e.value, p)).collect()
                    };
                    for (path, value) in heard {
                        let mut event = json!({"seq": e.seq, "t": e.t, "path": path, "value": value});
                        if e.once {
                            event["once"] = json!(true);
                        }
                        events.push(event);
                    }
                }
                // Changes there were, but none under these paths: the subscriber's place moves on all the same,
                // and it is answered only when there is something for it, or when its wait is over.
                if !events.is_empty() || Instant::now() >= end {
                    return json!({"epoch": inner.epoch, "seq": inner.seq, "events": events});
                }
                passed = Some(inner.seq);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return json!({"epoch": inner.epoch, "seq": inner.seq, "events": []});
            }
            inner = self.news.wait_timeout(inner, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Take in what another node's bus answered (`since`'s answer, as it came over the network), so that this
    /// struct is a copy of that node's tree with that node's own numbers. Everything is checked as it is taken:
    /// a path is a path, a value and the whole tree have their sizes, numbers only go up.
    pub fn apply(&self, answer: &Value) -> Result<(), String> {
        // (an epoch is what this program makes one of: a few letters and digits, as a request may carry it)
        let epoch = (answer.get("epoch").and_then(Value::as_str))
            .filter(|e| !e.is_empty() && e.len() <= 64 && e.bytes().all(|b| b.is_ascii_alphanumeric()));
        let (Some(epoch), Some(seq)) = (epoch, answer.get("seq").and_then(Value::as_u64)) else {
            return Err("not an answer of a bus".to_string());
        };
        let mut inner = self.held();
        if let Some(whole) = answer.get("whole") {
            let tree = whole.as_object().ok_or("not a tree")?;
            let size = whole.to_string().len();
            if size > MAX_TREE {
                return Err("that node's tree is too large".to_string());
            }
            if tree.keys().any(|k| parts(k).is_none_or(|p| p.len() != 1)) {
                return Err("that node's tree has a name that is no path".to_string());
            }
            *inner = Inner {
                epoch: epoch.to_string(),
                seq,
                tree: tree.clone(),
                tree_bytes: written(tree),
                log: VecDeque::new(),
                log_bytes: 0,
            };
        } else {
            let events = answer.get("events").and_then(Value::as_array).ok_or("not an answer of a bus")?;
            if epoch != inner.epoch || seq < inner.seq {
                return Err("that node began again: its whole tree is asked for".to_string());
            }
            for e in events {
                let (Some(n), Some(path)) =
                    (e.get("seq").and_then(Value::as_u64), e.get("path").and_then(Value::as_str))
                else {
                    return Err("not a change".to_string());
                };
                let parts = parts(path).ok_or("a change's path is no path")?;
                let value = e.get("value").cloned().unwrap_or(Value::Null);
                let size = value.to_string().len();
                if size > MAX_VALUE {
                    return Err("a change that can't be taken".to_string());
                }
                if n <= inner.seq {
                    continue; // taken already (a connection made again repeats a little)
                }
                let once = e.get("once").and_then(Value::as_bool) == Some(true);
                // (said once: passed on, never kept; a branch taken away that this copy never had: nothing to do)
                let absent = value.is_null() && lookup(&inner.tree, &parts).is_none();
                if !once && !absent {
                    inner.put(&parts, &value, size).ok_or("that node's tree is too large")?;
                }
                let t = e.get("t").and_then(Value::as_u64).unwrap_or(0);
                inner.log_push(Event { seq: n.min(seq), t, path: path.to_string(), value, size, once });
            }
            inner.seq = seq;
        }
        drop(inner);
        self.news.notify_all();
        Ok(())
    }

    /// Forget everything (a copy whose node can't be reached any more keeps nothing of it).
    pub fn clear(&self) {
        let mut inner = self.held();
        *inner =
            Inner { epoch: new_epoch(), seq: 0, tree: Map::new(), tree_bytes: 0, log: VecDeque::new(), log_bytes: 0 };
        drop(inner);
        self.news.notify_all();
    }
}

/// A cursor from what a request says (`after`, `epoch`), or None for a subscriber that begins.
pub fn cursor_of(after: Option<&str>, epoch: Option<&str>) -> Option<Cursor> {
    let seq = after?.parse::<u64>().ok()?;
    let epoch = epoch.filter(|e| !e.is_empty() && e.len() <= 64 && e.bytes().all(|b| b.is_ascii_alphanumeric()))?;
    Some(Cursor { epoch: epoch.to_string(), seq })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Duration = Duration::ZERO;

    fn cursor(answer: &Value) -> Cursor {
        Cursor { epoch: answer["epoch"].as_str().unwrap().to_string(), seq: answer["seq"].as_u64().unwrap() }
    }

    #[test]
    fn a_subscriber_gets_the_whole_tree_first_and_then_only_what_changes() {
        let bus = Bus::new();
        assert!(bus.set("node", json!({"device": "a computer"})).unwrap());
        assert!(bus.set("state/off", json!(["voice"])).unwrap());
        let first = bus.since(None, &[], NOW);
        assert_eq!(first["whole"], json!({"node": {"device": "a computer"}, "state": {"off": ["voice"]}}));
        assert_eq!(first["seq"], 2);
        let at = cursor(&first);
        // nothing new: an empty answer, at once when it isn't to wait
        assert_eq!(bus.since(Some(&at), &[], NOW)["events"], json!([]));
        // the same value again is no news; another one is
        assert!(!bus.set("state/off", json!(["voice"])).unwrap());
        assert!(bus.set("state/off", json!([])).unwrap());
        assert!(bus.set("readings", json!({"rows": [[1, 2, 3]]})).unwrap());
        let next = bus.since(Some(&at), &[], NOW);
        let heard: Vec<(u64, &str)> = next["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["seq"].as_u64().unwrap(), e["path"].as_str().unwrap()))
            .collect();
        assert_eq!(heard, [(3, "state/off"), (4, "readings")]);
        assert_eq!(next["events"][0]["value"], json!([]));
        assert!(next["events"][0]["t"].as_u64().unwrap() > 1_700_000_000_000);
        assert_eq!(next["seq"], 4);
        // a branch taken away is a change to nothing
        assert!(bus.set("readings", Value::Null).unwrap());
        assert!(!bus.set("readings", Value::Null).unwrap());
        assert_eq!(bus.get("readings"), None);
        assert_eq!(bus.since(Some(&cursor(&next)), &[], NOW)["events"][0]["value"], Value::Null);
    }

    #[test]
    fn a_subscriber_to_a_path_hears_what_lies_under_it_and_what_replaces_it_from_above() {
        let bus = Bus::new();
        bus.set("sources/camera/signals/checks", json!({"unit": "/s", "value": 12})).unwrap();
        bus.set("sources/host/signals/load", json!({"unit": "%", "value": 40})).unwrap();
        bus.set("settings", json!({"optVoice": {"value": false}})).unwrap();
        let only = vec!["sources/camera".to_string()];
        let first = bus.since(None, &only, NOW);
        assert_eq!(
            first["whole"],
            json!({"sources": {"camera": {"signals": {"checks": {"unit": "/s", "value": 12}}}}})
        );
        let at = cursor(&first);
        // elsewhere: no event, but the subscriber's place moves on
        bus.set("sources/host/signals/load/value", json!(41)).unwrap();
        let none = bus.since(Some(&at), &only, NOW);
        assert_eq!((none["events"].clone(), none["seq"].clone()), (json!([]), json!(4)));
        // under the path: as it is
        bus.set("sources/camera/signals/checks/value", json!(13)).unwrap();
        let under = bus.since(Some(&cursor(&none)), &only, NOW);
        assert_eq!(under["events"][0]["path"], "sources/camera/signals/checks/value");
        // above the path: the part of the new branch that lies under it; gone when the new branch has none
        bus.set("sources", json!({"camera": {"enabled": false}, "host": {}})).unwrap();
        let above = bus.since(Some(&cursor(&under)), &only, NOW);
        assert_eq!(
            above["events"],
            json!([{"seq": 6, "t": above["events"][0]["t"], "path": "sources/camera", "value": {"enabled": false}}])
        );
        bus.set("sources", json!({"host": {}})).unwrap();
        assert_eq!(bus.since(Some(&cursor(&above)), &only, NOW)["events"][0]["value"], Value::Null);
        // two paths: each hears its own
        let two = vec!["settings".to_string(), "sources/host".to_string()];
        assert_eq!(
            bus.since(None, &two, NOW)["whole"],
            json!({"settings": {"optVoice": {"value": false}}, "sources": {"host": {}}})
        );
    }

    #[test]
    fn a_subscriber_that_fell_behind_or_knew_another_run_gets_the_whole_tree_again() {
        let bus = Bus::new();
        bus.set("a", json!(0)).unwrap();
        let at = cursor(&bus.since(None, &[], NOW));
        for i in 1..=(MAX_LOG as u64 + 10) {
            bus.set("a", json!(i)).unwrap();
        }
        let again = bus.since(Some(&at), &[], NOW);
        assert_eq!(again["whole"], json!({"a": MAX_LOG as u64 + 10}), "the changes between are no longer kept");
        // a little behind: the changes
        let near = Cursor { epoch: at.epoch.clone(), seq: again["seq"].as_u64().unwrap() - 3 };
        assert_eq!(bus.since(Some(&near), &[], NOW)["events"].as_array().unwrap().len(), 3);
        // another run's number, or one this run never gave
        for odd in [Cursor { epoch: "another".into(), seq: 2 }, Cursor { epoch: at.epoch.clone(), seq: 1_000_000 }] {
            assert!(bus.since(Some(&odd), &[], NOW).get("whole").is_some(), "{odd:?}");
        }
        assert_ne!(Bus::new().cursor().epoch, bus.cursor().epoch);
    }

    #[test]
    fn a_waiting_subscriber_is_told_the_moment_something_is_published() {
        let bus = std::sync::Arc::new(Bus::new());
        bus.set("a", json!(1)).unwrap();
        let at = cursor(&bus.since(None, &[], NOW));
        let (b, began) = (bus.clone(), Instant::now());
        let waiting = std::thread::spawn(move || b.since(Some(&at), &[], Duration::from_secs(20)));
        std::thread::sleep(Duration::from_millis(150));
        bus.set("a", json!(2)).unwrap();
        let heard = waiting.join().unwrap();
        assert_eq!(heard["events"][0]["value"], 2);
        assert!(began.elapsed() < Duration::from_secs(10), "it waited for the news, not for the time to pass");
        // and with no news, it waits as long as it was told and no longer
        let (at, began) = (cursor(&heard), Instant::now());
        assert_eq!(bus.since(Some(&at), &[], Duration::from_millis(200))["events"], json!([]));
        assert!(began.elapsed() >= Duration::from_millis(190) && began.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_subscriber_to_a_path_is_not_answered_for_what_changes_elsewhere() {
        // (a node says its readings four times a second: a subscriber to its state must not be answered, with
        // nothing, as often)
        let bus = std::sync::Arc::new(Bus::new());
        bus.set("state/running", json!(true)).unwrap();
        let at = cursor(&bus.since(None, &[], NOW));
        let (b, began) = (bus.clone(), Instant::now());
        let state = ["state".to_string()];
        let waiting = std::thread::spawn(move || b.since(Some(&at), &state, Duration::from_secs(20)));
        for n in 0..4 {
            std::thread::sleep(Duration::from_millis(80));
            bus.say("readings", json!({"n": n})).unwrap();
        }
        assert!(!waiting.is_finished(), "what was said elsewhere answered it");
        bus.set("state/running", json!(false)).unwrap();
        let heard = waiting.join().unwrap();
        assert!(began.elapsed() < Duration::from_secs(10));
        assert_eq!(
            heard["events"],
            json!([{"seq": heard["seq"], "t": heard["events"][0]["t"], "path": "state/running", "value": false}])
        );
        // its place moved past all of it: nothing is told twice, and with nothing of its own it waits its time
        // out and is given the place the tree is at now
        bus.say("readings", json!({"n": 9})).unwrap();
        let (at, began) = (cursor(&heard), Instant::now());
        let quiet = bus.since(Some(&at), &["state".to_string()], Duration::from_millis(200));
        assert!(began.elapsed() >= Duration::from_millis(190), "{:?}", began.elapsed());
        assert_eq!((&quiet["events"], cursor(&quiet)), (&json!([]), bus.cursor()));
    }

    #[test]
    fn only_paths_are_published_and_nothing_beyond_the_sizes() {
        let bus = Bus::new();
        for bad in ["", "/a", "a/", "a//b", "a/\n", &"a/".repeat(MAX_DEPTH + 1), &"x".repeat(MAX_PATH + 1)] {
            assert!(bus.set(bad, json!(1)).is_err(), "{bad:?}");
        }
        assert!(bus.set("big", json!("x".repeat(MAX_VALUE))).is_err());
        // a tree holds what it may and no more, whatever is put where
        for i in 0..10 {
            let _ = bus.set(&format!("part{i}"), json!("x".repeat(MAX_VALUE - 10)));
        }
        let whole = bus.since(None, &[], NOW)["whole"].to_string().len();
        assert!(whole <= MAX_TREE + 1000 && whole > MAX_TREE / 2, "{whole}");
        // replacing a branch frees what it held
        assert!(bus.set("part0", json!(1)).unwrap());
        assert!(bus.set("part9", json!("x".repeat(1000))).is_ok());
        // a value where a branch is asked for gives way to the branch
        bus.set("part0/deeper", json!(2)).unwrap();
        assert_eq!(bus.get("part0"), Some(json!({"deeper": 2})));
        assert!(prefixes("a,b/c").is_some_and(|p| p == ["a", "b/c"]) && prefixes("").is_some_and(|p| p.is_empty()));
        // a path named twice, or under another one named too, is heard once
        assert_eq!(prefixes("readings,readings,state/off,state,stated").unwrap(), ["readings", "state", "stated"]);
        // the size is the tree's real one, whatever was put where and in which order: a value that gives way
        // to a branch, a branch replaced whole after its parts were set one by one, a branch taken away
        let drift = Bus::new();
        for _ in 0..50 {
            drift.set("a", json!("x".repeat(MAX_VALUE - 10))).unwrap();
            drift.set("a/b", json!(1)).unwrap();
            drift.set("a/c", json!(2)).unwrap();
            drift.set("a", json!({"b": 1})).unwrap();
            drift.set("a", Value::Null).unwrap();
        }
        assert_eq!(written(&drift.held().tree), 2, "an empty tree is two characters long");
        assert!(drift.held().tree_bytes <= MAX_TREE, "and the sum kept of it never ran away");
        assert!(drift.set("a", json!("x".repeat(MAX_VALUE - 10))).unwrap(), "what fits is still taken");
        // a long path with a small value costs what it is long, not what the value is
        let long = Bus::new();
        let mut kept = 0;
        for i in 0..40_000 {
            if long.set(&format!("{}/{i}", "n".repeat(150)), json!(1)).is_ok() {
                kept += 1;
            }
        }
        assert!(kept > 1000 && long.since(None, &[], NOW)["whole"].to_string().len() <= MAX_TREE, "{kept}");
        assert!(prefixes("a,,b").is_some_and(|p| p.len() == 2) && prefixes("a,/b").is_none());
        assert!(prefixes(&"a,".repeat(MAX_PREFIXES + 1)).is_none());
        assert_eq!(cursor_of(Some("7"), Some("abc123")), Some(Cursor { epoch: "abc123".into(), seq: 7 }));
        assert!(cursor_of(Some("x"), Some("abc")).is_none() && cursor_of(Some("7"), Some("a b")).is_none());
        assert!(cursor_of(None, Some("abc")).is_none() && cursor_of(Some("7"), None).is_none());
    }

    #[test]
    fn what_is_said_once_is_heard_by_who_is_there_and_never_kept() {
        let (bus, copy) = (Bus::new(), Bus::new());
        bus.set("state", json!({"running": true})).unwrap();
        let first = bus.since(None, &[], NOW);
        copy.apply(&first).unwrap();
        let at = cursor(&first);
        bus.say("readings", json!({"rows": [[1, 2, 3]]})).unwrap();
        bus.say("readings", json!({"rows": [[4, 5, 6]]})).unwrap();
        // a subscriber that is there hears both, in order, marked as said once
        let heard = bus.since(Some(&at), &[], NOW);
        let rows: Vec<&Value> = heard["events"].as_array().unwrap().iter().map(|e| &e["value"]["rows"][0][0]).collect();
        assert_eq!(rows, [&json!(1), &json!(4)]);
        assert!(heard["events"].as_array().unwrap().iter().all(|e| e["once"] == true && e["path"] == "readings"));
        // one that comes later is given the tree, and the tree never held them
        assert_eq!(bus.since(None, &[], NOW)["whole"], json!({"state": {"running": true}}));
        assert_eq!(bus.get("readings"), None);
        // a copy passes them on to its own subscribers and keeps none either
        copy.apply(&heard).unwrap();
        assert_eq!(copy.since(Some(&at), &[], NOW)["events"], heard["events"]);
        assert_eq!(copy.since(None, &[], NOW)["whole"], json!({"state": {"running": true}}));
        // a branch taken away that a copy never had leaves no trace in it
        let gone = json!({"epoch": at.epoch, "seq": 9, "events": [{"seq": 9, "path": "never/there", "value": null}]});
        copy.apply(&gone).unwrap();
        assert_eq!(copy.since(None, &[], NOW)["whole"], json!({"state": {"running": true}}));
        // an epoch that is not letters and digits is not one of this program's
        assert!(copy.apply(&json!({"epoch": "a b\r\nX: y", "seq": 1, "whole": {}})).is_err());
        assert!(bus.say("a//b", json!(1)).is_err() && bus.say("big", json!("x".repeat(MAX_VALUE))).is_err());
    }

    #[test]
    fn a_copy_of_another_nodes_tree_is_kept_with_that_nodes_own_numbers() {
        let (theirs, copy) = (Bus::new(), Bus::new());
        theirs.set("node", json!({"device": "a phone"})).unwrap();
        theirs.set("state/off", json!(["voice"])).unwrap();
        // the first answer is the whole tree; a subscriber of the copy gets it with the publisher's numbers
        let first = theirs.since(None, &[], NOW);
        copy.apply(&first).unwrap();
        let mine = copy.since(None, &[], NOW);
        assert_eq!((&mine["whole"], &mine["epoch"], &mine["seq"]), (&first["whole"], &first["epoch"], &first["seq"]));
        let at = cursor(&mine);
        // changes follow, and reach the copy's subscribers as changes
        theirs.set("state/off", json!([])).unwrap();
        theirs.set("readings", json!({"rows": []})).unwrap();
        let next = theirs.since(Some(&at), &[], NOW);
        copy.apply(&next).unwrap();
        let heard = copy.since(Some(&at), &[], NOW);
        assert_eq!(heard["events"], next["events"]);
        assert_eq!(copy.get("state/off"), Some(json!([])));
        // an answer with no changes only moves the copy's place
        copy.apply(&json!({"epoch": at.epoch, "seq": 4, "events": []})).unwrap();
        assert_eq!(copy.cursor().seq, 4);
        // the publisher began again: its changes can't be taken, its whole tree can, and the copy's subscribers
        // then get the whole tree too
        let again = Bus::new();
        again.set("node", json!({"device": "a phone", "app": "newer"})).unwrap();
        let other = again.since(Some(&copy.cursor()), &[], NOW);
        assert!(other.get("whole").is_some());
        assert!(copy.apply(&json!({"epoch": other["epoch"], "seq": 9, "events": []})).is_err());
        copy.apply(&other).unwrap();
        assert_eq!(
            copy.since(Some(&cursor(&heard)), &[], NOW)["whole"],
            json!({"node": {"device": "a phone", "app": "newer"}})
        );
        // what isn't an answer, or holds what a tree may not, is refused and changes nothing
        let before = copy.since(None, &[], NOW);
        for bad in [
            json!("a tree"),
            json!({"seq": 3, "whole": {}}),
            json!({"epoch": "e", "seq": 3, "whole": [1]}),
            json!({"epoch": "e", "seq": 3, "whole": {"a/b": 1}}),
            json!({"epoch": "e", "seq": 3, "whole": {"big": "x".repeat(MAX_TREE)}}),
            json!({"epoch": other["epoch"], "seq": 5, "events": [{"seq": 5, "path": "a//b", "value": 1}]}),
            json!({"epoch": other["epoch"], "seq": 5, "events": [{"seq": 5, "path": "a", "value": "x".repeat(MAX_VALUE + 1)}]}),
            json!({"epoch": other["epoch"], "seq": 5, "events": [{"path": "a", "value": 1}]}),
        ] {
            assert!(copy.apply(&bad).is_err(), "{}", bad.to_string().chars().take(80).collect::<String>());
        }
        assert_eq!(copy.since(None, &[], NOW)["whole"], before["whole"]);
        copy.clear();
        assert_eq!(copy.since(None, &[], NOW)["whole"], json!({}));
    }
}
