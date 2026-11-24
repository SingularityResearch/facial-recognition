//! What a device only ever sends:
//! the expression log's and the music log's rows waiting in the private
//! folder, the app's own log, and the lines of the posed tests. The database is where these are kept; no
//! device takes them back.

use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use face_files::{
    filejson,
    paths::{SharedLock, utc_now},
};
use serde_json::{Value, json};
use tokio_postgres::Client;

use crate::{
    local::utc_time,
    server::{Param, run_all},
    state::State,
};

type Result<T> = std::result::Result<T, String>;

fn said(e: tokio_postgres::Error) -> String {
    crate::server::words(&e)
}

/// Days the app's log is kept in the database.
const LOG_KEEP_DAYS: i32 = 90;

/// A log whose rows wait on this device until they are in the database: one row for a device, a name and
/// a time, so a line sent twice changes nothing.
pub struct Waiting {
    /// the folder's name in the private folder: `<folder>/<UTC day>.jsonl`, a line a row
    pub folder: &'static str,
    /// names this log's files in what the sync remembers: `<key>:<file name>` is how many bytes of it were sent
    pub key: &'static str,
    insert: &'static str,
    /// a waiting line as the table's columns after device, name and time; None: not a row to send
    values: fn(&Value) -> Option<Vec<Param>>,
}

fn text(o: &Value, k: &str) -> Option<String> {
    o.get(k).and_then(Value::as_str).map(str::to_string)
}

fn real(o: &Value, k: &str) -> Option<f32> {
    o.get(k).and_then(Value::as_f64).map(|v| v as f32)
}

/// A row's data as the database gets it: what the line has, or an empty object.
fn data(o: &Value) -> String {
    let empty = |v: &Value| v.is_null() || v.as_object().is_some_and(|m| m.is_empty());
    filejson::dumps(o.get("data").filter(|d| !empty(d)).unwrap_or(&json!({})))
}

/// The expression log: what each named person's face looked like (a reading without a label isn't sent).
pub const EXPRESSION: Waiting = Waiting {
    folder: "expression",
    key: "xlog",
    insert: "INSERT INTO expression_log (device, name, at, label, share, state, data)
             VALUES ($1::text::uuid, $2, 'epoch'::timestamptz + $3::bigint * interval '1 millisecond', $4, $5::real, $6, $7::text::jsonb)
             ON CONFLICT (device, name, at) DO NOTHING",
    values: |o| {
        let values: Vec<Param> = vec![Box::new(text(o, "label")?), Box::new(real(o, "share")), Box::new(text(o, "state").unwrap_or_default()), Box::new(data(o))];
        Some(values)
    },
};

/// The music log: the songs that played (a line without artist or song isn't sent).
pub const MUSIC: Waiting = Waiting {
    folder: "music",
    key: "mlog",
    insert: "INSERT INTO music_log (device, name, at, artist, song, seconds, data)
             VALUES ($1::text::uuid, $2, 'epoch'::timestamptz + $3::bigint * interval '1 millisecond', $4, $5, $6::real, $7::text::jsonb)
             ON CONFLICT (device, name, at) DO NOTHING",
    values: |o| {
        let values: Vec<Param> = vec![Box::new(text(o, "artist")?), Box::new(text(o, "song")?), Box::new(real(o, "seconds")), Box::new(data(o))];
        Some(values)
    },
};

/// The files of a folder whose names end so, in the order of their names.
fn files(folder: &Path, ends: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = (std::fs::read_dir(folder).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(ends)))
        .collect();
    found.sort();
    found
}

/// A text's lines, ended at every kind of line end (more than "\n").
fn lines(text: &str) -> impl Iterator<Item = &str> {
    let ends = |c: char| {
        matches!(
            c,
            '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    };
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let at = rest.find(ends).unwrap_or(rest.len());
        let line = &rest[..at];
        let after = &rest[at..];
        rest = match after.strip_prefix("\r\n") {
            Some(next) => next,
            None => after.get(after.chars().next().map_or(0, char::len_utf8)..).unwrap_or(""),
        };
        Some(line)
    })
}

fn number(state: &State, key: &str) -> Result<u64> {
    Ok(state.get(key)?.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// The bytes of a file from `from` to `to`, up to its last whole line: (the text, how many bytes it is).
fn whole_lines(file: &Path, from: u64, to: u64) -> (String, u64) {
    let mut chunk = Vec::new();
    if to > from {
        let read = std::fs::File::open(file).and_then(|mut f| {
            f.seek(SeekFrom::Start(from))?;
            f.take(to - from).read_to_end(&mut chunk)
        });
        if read.is_err() {
            chunk.clear();
        }
    }
    let end = chunk.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    (as_text(&chunk[..end]), end as u64)
}

/// A file's bytes as the text that is sent: without the zero bytes a file has where its storage failed
/// while it was written. PostgreSQL takes no text with one, and one such line kept a device's whole sync from
/// ever finishing a step (a Raspberry Pi whose card had dropped out, 2026-11-22).
fn as_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.contains('\0') { text.replace('\0', "") } else { text.into_owned() }
}

fn changed_at(file: &Path) -> Option<SystemTime> {
    std::fs::metadata(file).ok()?.modified().ok()
}

/// The mark beside a waiting file that a forget rewrote: what was remembered as sent of it no longer says
/// where the unsent lines begin.
fn again_mark(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".again");
    file.with_file_name(name)
}

/// The days since the year 1 of today's date in UTC: the number a day has in the sync's state.
pub(crate) fn today_number() -> String {
    let days = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400);
    (days + 719_163).to_string()
}

impl Waiting {
    /// Put the waiting rows into the table. A file that was rewritten (a person forgotten) is sent again
    /// from its start; a past day's file goes once all of it is in the database and nothing was added to it
    /// for a minute. How many rows were sent. When the database can't take them, they wait as they were.
    pub async fn send(&self, client: &mut Client, state: &State, home: &Path, device: &str) -> Result<usize> {
        let lock = SharedLock::new(home, &format!("waitlog-{}", self.folder));
        let today = utc_now()[..10].to_string();
        let mut sent = 0;
        for file in files(&home.join(self.folder), ".jsonl") {
            let Some(name) = file.file_name().and_then(|n| n.to_str()) else { continue };
            let key = format!("{}:{name}", self.key);
            let mark = again_mark(&file);
            let (done, again, text, end) = {
                let _one = lock.hold();
                let Ok(size) = std::fs::metadata(&file).map(|m| m.len()) else { continue };
                let mut done = number(state, &key)?;
                let again = changed_at(&mark); // rewritten by a forget since it was last sent
                if size < done || again.is_some() {
                    done = 0;
                }
                let (text, end) = whole_lines(&file, done, size);
                (done, again, text, end)
            };
            let rows: Vec<Vec<Param>> = (lines(&text))
                .filter_map(|line| {
                    let o: Value = serde_json::from_str(line).ok()?;
                    let at = o.get("t").and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)))?;
                    let mut row: Vec<Param> = vec![Box::new(device.to_string()), Box::new(text_of(&o)?), Box::new(at)];
                    row.extend((self.values)(&o)?);
                    Some(row)
                })
                .collect();
            if !rows.is_empty() {
                let tx = client.transaction().await.map_err(said)?;
                let insert = tx.prepare(self.insert).await.map_err(said)?;
                run_all(&tx, &insert, &rows).await?;
                tx.commit().await.map_err(said)?;
            }
            if end > 0 || again.is_some() {
                state.put(&key, &(done + end).to_string())?;
                sent += rows.len();
            }
            let _one = lock.hold();
            // the forget this sending began after is dealt with; one since then is not
            if again.is_some() && changed_at(&mark) == again {
                let _ = std::fs::remove_file(&mark);
            }
            // a past day, all of it sent, nothing added for a minute: done
            if let Ok(m) = std::fs::metadata(&file) {
                let quiet =
                    m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d > Duration::from_secs(60));
                let day = name.strip_suffix(".jsonl").unwrap_or(name);
                if day < today.as_str() && m.len() == done + end && quiet && !mark.exists() {
                    let _ = std::fs::remove_file(&file);
                    state.drop(&key)?;
                }
            }
        }
        Ok(sent)
    }
}

fn text_of(o: &Value) -> Option<String> {
    text(o, "name")
}

/// The app's own log (`HOME/app.log`): the whole lines added since the last time, and once a day what is
/// older than it is kept for is deleted there.
pub async fn app_log(client: &mut Client, state: &State, home: &Path, device: &str) -> Result<()> {
    let file = home.join("app.log");
    let size = std::fs::metadata(&file).map_or(0, |m| m.len());
    let mut done = number(state, "log_offset")?;
    if size < done {
        done = size; // written anew (a person forgotten) or replaced
    }
    if size > done {
        let (text, end) = whole_lines(&file, done, size);
        let rows: Vec<Vec<Param>> = lines(&text)
            .filter(|l| !l.trim().is_empty())
            .map(|l| -> Vec<Param> { vec![Box::new(device.to_string()), Box::new(l.to_string())] })
            .collect();
        if !rows.is_empty() {
            let tx = client.transaction().await.map_err(said)?;
            let insert = tx
                .prepare("INSERT INTO app_log (device, at, line) VALUES ($1::text::uuid, now(), $2)")
                .await
                .map_err(said)?;
            run_all(&tx, &insert, &rows).await?;
            tx.commit().await.map_err(said)?;
        }
        state.put("log_offset", &(done + end).to_string())?;
    }
    let today = today_number();
    if state.get("log_pruned_day")?.as_deref() != Some(today.as_str()) {
        let old = "DELETE FROM app_log WHERE device = $1::text::uuid AND at < now() - make_interval(days => $2)";
        client.execute(old, &[&device, &LOG_KEEP_DAYS]).await.map_err(said)?;
        state.put("log_pruned_day", &today)?;
    }
    Ok(())
}

/// A line of a posed test as its row: (id, file, the line as JSON).
type TestLine = (String, String, String);

/// The posed tests and feeling reports (`data/expr_eval/*.jsonl`) as rows: (id, file, the line), from each
/// file's line `from(file name)` on; and how many lines each file has now.
fn test_lines(data: &Path, from: &dyn Fn(&str) -> usize) -> (Vec<TestLine>, Vec<(String, usize)>) {
    let (mut rows, mut counts) = (Vec::new(), Vec::new());
    for file in files(&data.join("expr_eval"), ".jsonl") {
        let (Some(name), Ok(text)) = (file.file_name().and_then(|n| n.to_str()), std::fs::read_to_string(&file)) else {
            continue;
        };
        let all: Vec<&str> = lines(&text).collect();
        for (k, line) in all.iter().enumerate().skip(from(name).min(all.len())) {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                rows.push((format!("{name}:{k}"), name.to_string(), filejson::dumps(&v)));
            }
        }
        counts.push((name.to_string(), all.len()));
    }
    (rows, counts)
}

const TEST_LINE: &str = "INSERT INTO expr_eval (id, device, file, data) VALUES ($1, $2::text::uuid, $3, $4::text::jsonb) ON CONFLICT DO NOTHING";

fn test_params(device: &str, rows: Vec<TestLine>) -> Vec<Vec<Param>> {
    (rows.into_iter())
        .map(|(id, file, line)| -> Vec<Param> {
            vec![Box::new(id), Box::new(device.to_string()), Box::new(file), Box::new(line)]
        })
        .collect()
}

/// The posed tests' and feeling reports' new lines.
pub async fn posed_tests(client: &mut Client, state: &State, data: &Path, device: &str) -> Result<()> {
    let sent = |name: &str| state.get(&format!("expr:{name}")).ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0);
    let (rows, counts) = test_lines(data, &sent);
    for (name, n) in counts {
        state.put(&format!("expr:{name}"), &n.to_string())?;
    }
    if !rows.is_empty() {
        let tx = client.transaction().await.map_err(said)?;
        let insert = tx.prepare(TEST_LINE).await.map_err(said)?;
        run_all(&tx, &insert, &test_params(device, rows)).await?;
        tx.commit().await.map_err(said)?;
    }
    Ok(())
}

/// A line of the app's log that begins a run ("--- 2026-11-10 08:00:00Z ..."): when.
fn run_began(line: &str) -> Option<String> {
    let rest = line.strip_prefix("--- ")?;
    let b = rest.as_bytes();
    let shaped = |at: usize, what: &str| {
        what.bytes()
            .enumerate()
            .all(|(i, w)| b.get(at + i).is_some_and(|c| if w == b'd' { c.is_ascii_digit() } else { *c == w }))
    };
    if !(shaped(0, "dddd-dd-dd") && matches!(b.get(10), Some(b' ' | b'T')) && shaped(11, "dd:dd:dd")) {
        return None;
    }
    let len = if b.get(19) == Some(&b'Z') { 20 } else { 19 };
    if b.get(len) != Some(&b' ') {
        return None;
    }
    utc_time(&rest[..len])
}

/// When a device joins a database: every line of the posed tests, and the app's log with each line at the
/// time of the run it belongs to (the device's lines only once). Inside the join's own transaction.
pub async fn merge_in(tx: &tokio_postgres::Transaction<'_>, home: &Path, data: &Path, device: &str) -> Result<()> {
    let (rows, _) = test_lines(data, &|_| 0);
    if !rows.is_empty() {
        let insert = tx.prepare(TEST_LINE).await.map_err(said)?;
        run_all(tx, &insert, &test_params(device, rows)).await?;
    }
    let Ok(bytes) = std::fs::read(home.join("app.log")) else { return Ok(()) };
    let text = as_text(&bytes);
    let mut at: Option<String> = None;
    let mut rows: Vec<Vec<Param>> = Vec::new();
    for line in lines(&text) {
        if let Some(began) = run_began(line) {
            at = Some(began);
        }
        if !line.trim().is_empty() {
            rows.push(vec![Box::new(device.to_string()), Box::new(at.clone()), Box::new(line.to_string())]);
        }
    }
    let has = tx
        .query_opt("SELECT 1 FROM app_log WHERE device = $1::text::uuid LIMIT 1", &[&device])
        .await
        .map_err(said)?
        .is_some();
    if !rows.is_empty() && !has {
        let insert = tx
            .prepare("INSERT INTO app_log (device, at, line) VALUES ($1::text::uuid, coalesce($2::text::timestamptz, now()), $3)")
            .await
            .map_err(said)?;
        run_all(tx, &insert, &rows).await?;
    }
    Ok(())
}

/// After joining: only what is written from now on is sent (the join sent the log and the tests as they are).
pub fn on_join(state: &State, home: &Path, data: &Path) -> Result<()> {
    state.put("log_offset", &std::fs::metadata(home.join("app.log")).map_or(0, |m| m.len()).to_string())?;
    for (name, n) in test_lines(data, &|_| usize::MAX).1 {
        state.put(&format!("expr:{name}"), &n.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_end_at_every_kind_of_line_end() {
        assert_eq!(lines("a\nb\r\nc\rd\u{2028}e\n").collect::<Vec<_>>(), ["a", "b", "c", "d", "e"]);
        assert_eq!(lines("a\n\nb").collect::<Vec<_>>(), ["a", "", "b"]);
        assert_eq!(lines("").count(), 0);
        assert_eq!(lines("\n").collect::<Vec<_>>(), [""]);
    }

    #[test]
    fn a_run_of_the_app_begins_with_its_time() {
        assert_eq!(run_began("--- 2026-11-10 08:00:00Z FaceRecognizer 2.4.0").as_deref(), Some("2026-11-10T08:00:00Z"));
        assert_eq!(run_began("--- 2026-11-10T08:00:00Z x").as_deref(), Some("2026-11-10T08:00:00Z"));
        // (an older log's time, in local time: whatever this computer's zone, it is a time)
        assert!(run_began("--- 2026-11-10 08:00:00 x").is_some());
        for line in
            ["2026-11-10 08:00:00Z x", "--- 2026-11-10 08:00:00Z", "--- 2026-11-10 08:00 x", "--- soon x", "---"]
        {
            assert_eq!(run_began(line), None, "{line}");
        }
    }

    #[test]
    fn only_whole_lines_are_read_from_where_the_last_sending_stopped() {
        let dir = std::env::temp_dir().join(format!("face-sync-sent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("expr_eval")).unwrap();
        let file = dir.join("app.log");
        std::fs::write(&file, "one\ntwo\nthr").unwrap();
        assert_eq!(whole_lines(&file, 0, 11), ("one\ntwo\n".to_string(), 8));
        assert_eq!(whole_lines(&file, 4, 11), ("two\n".to_string(), 4));
        assert_eq!(whole_lines(&file, 8, 11), (String::new(), 0));
        assert_eq!(whole_lines(&dir.join("no such file"), 0, 5), (String::new(), 0));
        // a file whose storage failed: its zero bytes are not sent, and the place in the file counts them
        std::fs::write(&file, b"one\n\0\0\0two\0\nthr").unwrap();
        assert_eq!(whole_lines(&file, 0, 15), ("one\ntwo\n".to_string(), 12));
        assert_eq!(as_text(b"\0\0"), "");
        assert_eq!(again_mark(&dir.join("2026-11-10.jsonl")), dir.join("2026-11-10.jsonl.again"));
        // the posed tests' lines: a row for each that can be read, numbered by its place in the file
        std::fs::write(dir.join("expr_eval").join("s1.jsonl"), "{\"a\": 1}\nnot a line\n\n{\"b\": 2.50}\n").unwrap();
        let (rows, counts) = test_lines(&dir, &|_| 0);
        assert_eq!(
            rows,
            [
                ("s1.jsonl:0".into(), "s1.jsonl".into(), "{\"a\": 1}".into()),
                ("s1.jsonl:3".into(), "s1.jsonl".into(), "{\"b\": 2.5}".into())
            ]
        );
        assert_eq!(counts, [("s1.jsonl".to_string(), 4)]);
        assert_eq!(test_lines(&dir, &|_| 3).0.len(), 1);
        assert!(test_lines(&dir, &|_| 99).0.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_waiting_line_is_a_row_only_with_what_its_table_needs() {
        let reading = json!({"t": 5, "name": "Ann", "label": "happy", "share": 0.5, "data": {"n": 1}});
        assert_eq!((EXPRESSION.values)(&reading).map(|v| v.len()), Some(4));
        assert!((EXPRESSION.values)(&json!({"t": 5, "name": "Ann"})).is_none(), "a reading without a label");
        assert_eq!((MUSIC.values)(&json!({"artist": "A", "song": "S"})).map(|v| v.len()), Some(4));
        assert!((MUSIC.values)(&json!({"artist": "A"})).is_none() && (MUSIC.values)(&json!({"song": "S"})).is_none());
        assert_eq!(
            (data(&json!({})), data(&json!({"data": null})), data(&json!({"data": {"k": [1]}}))),
            ("{}".to_string(), "{}".to_string(), "{\"k\": [1]}".to_string())
        );
    }
}
