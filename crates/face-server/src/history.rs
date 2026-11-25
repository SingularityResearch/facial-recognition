//! The signal history's writer (the owner, 2026-11-24: "Keep a history of ALL signals in the database", and
//! of how: "make sure all database read / writes are non-blocking on all other threads - this should NOT
//! slow down ANY other part of the entire application", "This should NOT slow down the FPS at all - if
//! necessary, batch and only send to the database at the peak idle time on the device. This is an important
//! feature, but its more important that it is both optional and does NOT impact FPS performance"; and, once
//! the first measurements showed that writing every half minute with a face in view cost nothing that could
//! be measured, 2026-11-25: "if the performance is good (almost the same as with none) with faster with more
//! frequent database writes / reads - then we should do faster (more frequent) database writes / reads. It
//! objective is to balance the database operations so that they get to / from the databases as fast as
//! possible while slowing down the FPS the leasts amount").
//!
//! So nothing but this module's own thread ever touches the database or a file for the history, and that
//! thread balances the two by itself:
//!
//! - a request (the page's rows, as each is finished) only puts its checked rows into a list in memory
//!   (`take`: a lock held for the time of a few pushes) and is answered;
//! - one thread, with a connection to the shared database that is its own, sends what is in the list in ONE
//!   statement (`face_db::signals::send`). No other connection waits for it: not the one that says "this
//!   device is here", not the sync's, not the pages' short questions;
//! - with nobody in view (`QUIET_AFTER`) it writes at once, at its next look (`LOOK_EVERY`);
//! - with a face in view (`face_seen`: the tracker is reading faces, its busiest work) it writes as often
//!   as keeps its own database work under one part in `BUSY_SHARE` of the time: after work that took t it
//!   waits `BUSY_SHARE` x t (`busy_wait`: `BUSY_LEAST` at the least, `HOLD_MOST` at the most). A database
//!   that answers in a few thousandths of a second is written to every few seconds; one that is slow or far
//!   away, or doesn't answer, is left alone for that much longer, by itself. Half a full list is written
//!   whatever the pace;
//! - while the database can't be reached the rows go to the private folder instead (`siglog::Spill`), and
//!   from there to the database, oldest first, when it is back. A statement that gets no answer is given up
//!   after `GIVE_UP` and the connection made anew;
//! - the list has an end (`MOST_HELD`): a writer that falls behind loses the oldest rows, never memory, and
//!   says how many (`status`).
//!
//! Once a day the thread deletes this device's rows that are older than the days its owner set
//! (`siglog::keep_days`). Everything it did is counted for the page (`status`): how long its statements
//! took, how many it wrote with a face in view, and how long after a row was finished the database had it.
//! `QUIET_AFTER`, `BUSY_SHARE`, `BUSY_LEAST` and `HOLD_MOST` are choices; what they cost is measured (the
//! guide's "The signal history" has the numbers and how they are taken).

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use face_home::siglog::{self, Spill};
use serde_json::{Value, json};

/// between two looks at the list
const LOOK_EVERY: Duration = Duration::from_secs(1);
/// nobody in view for this long: the device is quiet, and the rows are written at once
const QUIET_AFTER: Duration = Duration::from_secs(4);
/// with a face in view the writer's database work may take one part in this many of the time
const BUSY_SHARE: f64 = 1000.0;
/// with a face in view two writings are this far apart at the least, and at the most
const BUSY_LEAST: Duration = Duration::from_secs(5);
const HOLD_MOST: Duration = Duration::from_secs(300);
/// The seconds `FACE_HISTORY_HOLD` names, if it names any: a fixed wait between two writings with a face in
/// view, in place of the writer's own pace, for measuring what a pace costs.
fn named_wait() -> Option<Duration> {
    let named = std::env::var("FACE_HISTORY_HOLD").ok().and_then(|s| s.trim().parse::<u64>().ok());
    named.filter(|s| (1..=3600).contains(s)).map(Duration::from_secs)
}
/// How long the writer waits after its last database work before the next, while a face is in view: `named`
/// if a wait was named; else `BUSY_SHARE` times what that work took (`cost`), so that the work is one part in
/// `BUSY_SHARE` of the time at the most, within `BUSY_LEAST` and `HOLD_MOST`. Before any work was timed there
/// is nothing to wait for.
fn busy_wait(cost: Option<Duration>, named: Option<Duration>) -> Duration {
    match (named, cost) {
        (Some(wait), _) => wait,
        (None, None) => Duration::ZERO,
        (None, Some(cost)) => {
            Duration::from_secs_f64((cost.as_secs_f64() * BUSY_SHARE).min(HOLD_MOST.as_secs_f64())).max(BUSY_LEAST)
        }
    }
}
/// before the database is tried again after it couldn't be reached
const CONNECT_AGAIN: Duration = Duration::from_secs(30);
/// a statement that has no answer after this long is given up, and the connection with it
const GIVE_UP: Duration = Duration::from_secs(20);
/// rows held in memory at the most (about an hour of a full room)
const MOST_HELD: usize = 20_000;

/// What the writer did since the program started, for the page and for measuring.
#[derive(Clone, Default)]
struct Counts {
    /// rows taken from requests
    taken: u64,
    /// rows the database has (new ones, and ones it had already)
    written: u64,
    /// statements that wrote rows, and how many of them while a face was in view
    batches: u64,
    busy_batches: u64,
    /// milliseconds between a row being finished and the database having it: of the last statement's rows
    /// (the longest), and of all rows together
    last_lag_ms: f64,
    all_lag_ms: f64,
    /// rows lost because the list was full
    lost: u64,
    /// rows put into the private folder to wait
    waited: u64,
    /// old rows deleted: of the history, and of the expression and music logs
    pruned: u64,
    pruned_logs: u64,
    /// milliseconds the last writing statement took, the slowest one, and all of them together
    last_ms: f64,
    slowest_ms: f64,
    all_ms: f64,
    /// why the database couldn't be written to, while it can't
    trouble: String,
}

/// The rows that wait for the writer, and since when the oldest of them does.
#[derive(Default)]
struct Held {
    rows: VecDeque<Value>,
    since: Option<Instant>,
}

/// What the writer's thread keeps from one look at the list to the next: its own runtime and connection, the
/// folder where rows wait, and when it last tried the database, looked, and tidied.
struct Writer {
    rt: tokio::runtime::Runtime,
    spill: Spill,
    client: Option<tokio_postgres::Client>,
    tried: Option<Instant>,
    looked: Option<Instant>,
    pruned_on: String,
    /// when its last database work ended and how long that took: what its pace with a face in view goes by
    worked: Option<(Instant, Duration)>,
    /// a wait named for measuring, in place of that pace
    named: Option<Duration>,
}

impl Writer {
    fn new(home: &Path) -> Option<Writer> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
        Some(Writer {
            rt,
            spill: Spill::new(home),
            client: None,
            tried: None,
            looked: None,
            pruned_on: String::new(),
            worked: None,
            named: named_wait(),
        })
    }
}

pub struct History {
    home: PathBuf,
    held: Mutex<Held>,
    counts: Mutex<Counts>,
    started: AtomicBool,
    /// when the tracker last had a face in its camera's picture (milliseconds since 1970; 0: never)
    face_at: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl History {
    pub fn new(home: &Path) -> Arc<History> {
        Arc::new(History {
            home: home.to_path_buf(),
            held: Mutex::new(Held::default()),
            counts: Mutex::new(Counts::default()),
            started: AtomicBool::new(false),
            face_at: AtomicU64::new(0),
        })
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The tracker has a face in its camera's picture now: its busiest work. One number is set; nothing waits.
    pub fn face_seen(&self) {
        self.face_at.store(now_ms(), Ordering::Relaxed);
    }

    /// Whether nobody has been in view for `QUIET_AFTER`: the moment the writer works in.
    fn quiet(&self) -> bool {
        now_ms().saturating_sub(self.face_at.load(Ordering::Relaxed)) >= QUIET_AFTER.as_millis() as u64
    }

    /// Rows from a request, checked already (`siglog::rows`): put where the writer takes them. Never waits
    /// for a file or for the database. How many were taken.
    pub fn take(&self, rows: Vec<Value>) -> usize {
        let n = rows.len();
        let mut lost = 0;
        {
            let mut held = self.held();
            if n > 0 && held.rows.is_empty() {
                held.since = Some(Instant::now());
            }
            for row in rows {
                if held.rows.len() >= MOST_HELD {
                    held.rows.pop_front();
                    lost += 1;
                }
                held.rows.push_back(row);
            }
        }
        let mut counts = self.counts();
        counts.taken += n as u64;
        counts.lost += lost;
        n
    }

    /// What the writer did, as numbers: for the page's Settings and for measuring.
    pub fn status(&self) -> Value {
        let c = self.counts().clone();
        let (held, held_for) = {
            let held = self.held();
            (held.rows.len(), held.since.map_or(0, |s| s.elapsed().as_secs()))
        };
        let round = |ms: f64| (ms * 10.0).round() / 10.0;
        json!({"taken": c.taken, "written": c.written, "batches": c.batches, "busyBatches": c.busy_batches,
               "held": held, "heldFor": held_for, "lost": c.lost, "waited": c.waited, "pruned": c.pruned, "prunedLogs": c.pruned_logs,
               "lastMs": round(c.last_ms), "slowestMs": round(c.slowest_ms),
               "averageMs": if c.batches > 0 { round(c.all_ms / c.batches as f64) } else { 0.0 },
               "lastLagMs": c.last_lag_ms.round(),
               "averageLagMs": if c.written > 0 { (c.all_lag_ms / c.written as f64).round() } else { 0.0 },
               "quiet": self.quiet(), "trouble": c.trouble})
    }

    /// The writer's thread, started once (with the first rows, or by the tracker when it starts).
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        let started = std::thread::Builder::new().name("signal history".into()).spawn(move || me.run());
        if started.is_err() {
            self.started.store(false, Ordering::SeqCst);
        }
    }

    fn trouble(&self, why: String) {
        let mut counts = self.counts();
        if counts.trouble != why {
            eprintln!("Signal history: {why}");
            counts.trouble = why;
        }
    }

    fn run(&self) {
        let Some(mut writer) = Writer::new(&self.home) else { return };
        loop {
            std::thread::sleep(LOOK_EVERY);
            self.step(&mut writer);
        }
    }

    /// One look at the list: whatever is due is written, put to wait, or left for a little later. It must
    /// never end the thread with an error of its own: the programs end as a whole when a thread does.
    fn step(&self, w: &mut Writer) {
        // With nobody in view: at once. With a face in view: when the last database work is long enough ago
        // for its cost (`busy_wait`), or the list is half full.
        let quiet = self.quiet();
        let (full, some) = {
            let held = self.held();
            (held.rows.len() >= MOST_HELD / 2, !held.rows.is_empty())
        };
        let paced = w.worked.is_none_or(|(at, cost)| at.elapsed() >= busy_wait(Some(cost), w.named));
        if !quiet && !paced && !full {
            return;
        }
        // (nothing waits in memory: whether rows wait in the folder, and the day's tidying, are looked at
        // once a minute, not at every turn: a history that is switched off costs a number's reading)
        if !some && w.looked.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
            return;
        }
        w.looked = Some(Instant::now());
        // (without a shared database nothing is kept: the request says so itself, and takes no rows)
        let Ok(Some(settings)) = face_db::Settings::load(&self.home) else {
            w.client = None;
            return;
        };
        let today: String = crate::paths::utc_now().chars().take(10).collect();
        let rows: Vec<Value> = {
            let mut held = self.held();
            let n = held.rows.len().min(face_db::signals::MOST);
            let rows = held.rows.drain(..n).collect();
            held.since = if held.rows.is_empty() { None } else { Some(Instant::now()) };
            rows
        };
        // (a person forgotten since their rows were taken: those rows go no further)
        let forgotten = face_home::store::Store::new(&self.home).forgotten();
        let rows: Vec<Value> =
            rows.into_iter().filter(|r| !r["name"].as_str().is_some_and(|n| forgotten.contains(n))).collect();
        if rows.is_empty() && w.pruned_on == today && w.spill.size() == 0 {
            return;
        }
        let Ok(device) = face_db::Device::load(&self.home, &crate::device().0) else {
            self.wait(&w.spill, &rows);
            return;
        };
        if w.client.as_ref().is_some_and(tokio_postgres::Client::is_closed) {
            w.client = None;
        }
        if w.client.is_none() && w.tried.is_none_or(|at| at.elapsed() >= CONNECT_AGAIN) {
            w.tried = Some(Instant::now());
            // (the waiting is made inside the runtime: a timer made outside it has no clock to go by)
            let connecting = w.rt.block_on(async {
                tokio::time::timeout(GIVE_UP, face_db::connect(&settings, &settings.schema, &device)).await
            });
            match connecting {
                Ok(Ok(c)) => w.client = Some(c),
                Ok(Err(e)) => self.trouble(e),
                Err(_) => self.trouble("The shared database didn't answer.".to_string()),
            }
        }
        // (from here on the database, or the folder in its place, is worked with: timed as a whole, for the
        // pace with a face in view. The connection's own making is not in it: it is made once, and tried
        // again only every `CONNECT_AGAIN`)
        let began = Instant::now();
        let Some(c) = w.client.as_ref() else {
            // the database is away: the rows wait in the private folder
            self.wait(&w.spill, &rows);
            w.worked = Some((Instant::now(), began.elapsed()));
            return;
        };
        let written = w.rt.block_on(async {
            // what waited in the folder goes first, the oldest first; then what just came
            if let Some((file, old)) = w.spill.oldest(face_db::signals::MOST) {
                self.send(c, &device.id, &old, !quiet).await?;
                w.spill.done(&file, &old)?;
            }
            if !rows.is_empty() {
                self.send(c, &device.id, &rows, !quiet).await?;
            }
            Ok::<(), String>(())
        });
        if let Err(e) = written {
            self.trouble(e);
            self.wait(&w.spill, &rows);
            (w.client, w.tried) = (None, Some(Instant::now()));
            w.worked = Some((Instant::now(), began.elapsed()));
            return;
        }
        w.worked = Some((Instant::now(), began.elapsed()));
        self.counts().trouble.clear();
        // Once a day: this device's rows older than its days, of the history and of the expression and music
        // logs, which follow the same days. Whether or not the history is switched on, and whether or not
        // someone is in view (a camera that always sees someone would never tidy): it is this thread's work,
        // on its own connection, and as paced as the rest of it.
        if w.pruned_on != today {
            let store = face_home::store::Store::new(&self.home);
            let days = siglog::keep_days(store.setting("track", siglog::KEEP).as_deref());
            let logs = w.rt.block_on(async {
                tokio::time::timeout(GIVE_UP, face_db::signals::prune_logs(c, &device.id, days)).await
            });
            match logs {
                Ok(Ok((expression, music))) => self.counts().pruned_logs += expression + music,
                Ok(Err(e)) => self.trouble(e),
                Err(_) => self.trouble("The shared database didn't answer the tidying of the logs.".to_string()),
            }
            let pruning = w
                .rt
                .block_on(async { tokio::time::timeout(GIVE_UP, face_db::signals::prune(c, &device.id, days)).await });
            if let Ok(Ok(gone)) = pruning {
                self.counts().pruned += gone;
                w.pruned_on = today;
            }
        }
    }

    /// One statement for these rows, timed; given up when it has no answer.
    async fn send(
        &self,
        client: &tokio_postgres::Client,
        device: &str,
        rows: &[Value],
        busy: bool,
    ) -> Result<(), String> {
        let began = Instant::now();
        match tokio::time::timeout(GIVE_UP, face_db::signals::send(client, device, rows)).await {
            Ok(Ok(_)) => {
                let ms = began.elapsed().as_secs_f64() * 1000.0;
                // (how long after each row was finished the database has it: a row is finished when its
                // stretch ends)
                let now = now_ms() as f64;
                let lags: Vec<f64> = (rows.iter())
                    .filter_map(|r| Some(now - (r["t"].as_f64()? + r["s"].as_f64()? * 1000.0)))
                    .map(|lag| lag.max(0.0))
                    .collect();
                let mut counts = self.counts();
                counts.last_lag_ms = lags.iter().fold(0.0, |a: f64, b| a.max(*b));
                counts.all_lag_ms += lags.iter().sum::<f64>();
                counts.written += rows.len() as u64;
                counts.batches += 1;
                counts.busy_batches += u64::from(busy);
                counts.last_ms = ms;
                counts.all_ms += ms;
                counts.slowest_ms = counts.slowest_ms.max(ms);
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("The shared database didn't answer a writing of the signal history.".to_string()),
        }
    }

    /// The database can't take these rows now: they wait in the private folder.
    fn wait(&self, spill: &Spill, rows: &[Value]) {
        if rows.is_empty() {
            return;
        }
        match spill.keep(rows) {
            Ok(()) => self.counts().waited += rows.len() as u64,
            Err(e) => {
                self.counts().lost += rows.len() as u64;
                self.trouble(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(t: i64) -> Value {
        json!({"t": t, "name": "A", "s": 10, "n": 1, "data": {}})
    }

    #[test]
    fn a_request_only_puts_its_rows_into_the_list_and_the_list_has_an_end() {
        let dir = std::env::temp_dir().join(format!("face-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let history = History::new(&dir);
        assert_eq!(history.take(vec![row(1), row(2)]), 2);
        let s = history.status();
        assert_eq!(
            (s["taken"].clone(), s["held"].clone(), s["written"].clone(), s["lost"].clone()),
            (json!(2), json!(2), json!(0), json!(0))
        );
        // taking rows made no file and asked no database: nothing is in the private folder
        assert!(!dir.exists());
        // a writer that falls behind loses the oldest rows, and says how many
        let many: Vec<Value> = (0..MOST_HELD as i64 + 5).map(row).collect();
        let began = Instant::now();
        history.take(many);
        assert!(began.elapsed() < Duration::from_millis(500), "taking rows is quick: {:?}", began.elapsed());
        let s = history.status();
        assert_eq!((s["held"].clone(), s["lost"].clone()), (json!(MOST_HELD), json!(7)));
        assert_eq!(history.held().rows.front().unwrap()["t"], 5);
    }

    /// The writer's own step with a shared database that doesn't answer (an address of this computer where
    /// nothing listens): the thread goes on, and the rows wait in the private folder. The step is the
    /// thread's whole work, so an error of its own in it would end the program: this takes it as the thread
    /// does.
    #[test]
    fn with_the_database_away_the_writer_goes_on_and_the_rows_wait_in_the_private_folder() {
        let dir = std::env::temp_dir().join(format!("face-history-away-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        // ("a password", kept as the programs keep one where the system has no protection for it)
        let settings = json!({"host": "127.0.0.1", "port": port, "user": "u", "password": "plain:YSBwYXNzd29yZA=="});
        std::fs::write(dir.join("database.json"), settings.to_string()).unwrap();
        let history = History::new(&dir);
        let mut writer = Writer::new(&dir).unwrap();
        // nothing to write: a step asks nobody
        history.step(&mut writer);
        assert_eq!(history.status()["waited"], 0);
        let now = now_ms() as i64;
        let rows: Vec<Value> =
            (0..3).map(|i| json!({"t": now - i * 10_000, "name": "", "s": 10, "n": 5, "data": {}})).collect();
        assert_eq!(history.take(rows), 3);
        history.step(&mut writer);
        let s = history.status();
        assert_eq!(
            (s["held"].clone(), s["waited"].clone(), s["written"].clone()),
            (json!(0), json!(3), json!(0)),
            "{s}"
        );
        assert!(s["trouble"].as_str().is_some_and(|t| !t.is_empty()), "the writer says why: {s}");
        assert!(writer.spill.size() > 0);
        // more rows while it is away: they wait too, and the database is not tried again at once
        let began = Instant::now();
        assert_eq!(history.take(vec![json!({"t": now - 40_000, "name": "", "s": 10, "n": 5, "data": {}})]), 1);
        history.step(&mut writer);
        assert_eq!(history.status()["waited"], 4);
        assert!(began.elapsed() < Duration::from_secs(1), "no second try so soon: {:?}", began.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The writer's pace with a face in view: its database work is one part in `BUSY_SHARE` of the time at
    /// the most, so a quick database is written to every few seconds and a slow one is left alone longer.
    #[test]
    fn with_a_face_in_view_the_writer_waits_by_what_its_last_work_cost() {
        let ms = Duration::from_millis;
        // nothing was timed yet: nothing to wait for
        assert_eq!(busy_wait(None, None), Duration::ZERO);
        // a statement of 6 ms: 6 s; a quicker one: the least wait
        assert_eq!(busy_wait(Some(ms(6)), None), Duration::from_secs(6));
        assert_eq!(busy_wait(Some(ms(2)), None), BUSY_LEAST);
        // a slow link: left alone that much longer; a database that didn't answer: the longest wait
        assert_eq!(busy_wait(Some(ms(120)), None), Duration::from_secs(120));
        assert_eq!(busy_wait(Some(GIVE_UP), None), HOLD_MOST);
        // a wait named for measuring counts whatever the work cost
        assert_eq!(busy_wait(Some(ms(6)), Some(Duration::from_secs(20))), Duration::from_secs(20));
        assert_eq!(busy_wait(None, Some(Duration::from_secs(20))), Duration::from_secs(20));
    }

    #[test]
    fn the_device_is_quiet_only_when_nobody_has_been_in_view_for_a_while() {
        let history = History::new(Path::new("nowhere"));
        assert!(history.quiet(), "a tracker that never saw a face is quiet");
        history.face_seen();
        assert!(!history.quiet(), "a face is in view: the writer waits");
        assert_eq!(history.status()["quiet"], false);
        // (a face last seen longer ago than the wait: quiet again)
        history.face_at.store(now_ms() - QUIET_AFTER.as_millis() as u64 - 1, Ordering::Relaxed);
        assert!(history.quiet());
    }
}
