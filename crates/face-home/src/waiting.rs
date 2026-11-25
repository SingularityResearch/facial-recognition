//! Rows that wait on this device until the sync has sent them, for what the device itself asks of them
//! (`waiting` and `forget`; keeping a row is each log's own, sending them
//! the sync's): the expression log's readings and the music log's songs, in `HOME/<folder>/<UTC day>.jsonl`.

use std::path::{Path, PathBuf};

use face_files::{
    filejson,
    paths::{SharedLock, atomic_write, utc_now},
};
use serde_json::Value;

pub struct WaitLog {
    /// the folder's name in the private folder
    pub folder: &'static str,
    /// fields a line must have, besides its name and time, to count as a waiting row
    needs: &'static [&'static str],
}

/// What each named person's face looked like.
pub const EXPRESSION: WaitLog = WaitLog { folder: "expression", needs: &[] };
/// The songs that played for a person.
pub const MUSIC: WaitLog = WaitLog { folder: "music", needs: &["song"] };
/// The signal history's rows that the shared database couldn't take yet (`siglog`).
pub const SIGNALS: WaitLog = WaitLog { folder: "signals", needs: &["data"] };

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

impl WaitLog {
    fn files(&self, home: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = (std::fs::read_dir(home.join(self.folder)).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        found.sort();
        found
    }

    fn rows(&self, file: &Path) -> Vec<Value> {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        (text.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()))
            .filter(|o| {
                o.is_object()
                    && truthy(o.get("name"))
                    && o.get("t").is_some_and(|t| !t.is_null())
                    && self.needs.iter().all(|k| truthy(o.get(*k)))
            })
            .collect()
    }

    /// Keep rows until the sync sends them: each a line at the end of its UTC day's file (the log has checked
    /// them already: every one has its "t" and "name"). How many were kept.
    pub fn add(&self, home: &Path, rows: &[Value]) -> std::io::Result<usize> {
        use std::io::Write;
        // (these files are text: on Windows a line ends \r\n)
        const END: &str = if cfg!(windows) { "\r\n" } else { "\n" };
        let day = |r: &Value| {
            let seconds = r["t"].as_i64().unwrap_or(0).div_euclid(1000);
            face_files::paths::utc_text(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds.max(0) as u64))
                .chars()
                .take(10)
                .collect::<String>()
        };
        let lock = SharedLock::new(home, &format!("waitlog-{}", self.folder));
        let _one = lock.hold();
        std::fs::create_dir_all(home.join(self.folder))?;
        for r in rows {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(home.join(self.folder).join(format!("{}.jsonl", day(r))))?;
            file.write_all((face_files::filejson::dumps_utf8(r) + END).as_bytes())?;
        }
        Ok(rows.len())
    }

    /// How many rows each day's file holds: [("2026-11-17", 3), ...], oldest first. A past day's file is
    /// still here only while some of it isn't sent.
    pub fn days(&self, home: &Path) -> Vec<(String, usize)> {
        let lock = SharedLock::new(home, &format!("waitlog-{}", self.folder));
        let _one = lock.hold();
        let day = |f: &PathBuf| f.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        self.files(home).iter().map(|f| (day(f), self.rows(f).len())).collect()
    }

    /// Rows not sent yet, of one person or everyone's. (Today's file holds today's rows, sent or not.)
    pub fn waiting(&self, home: &Path, name: Option<&str>) -> Vec<Value> {
        let lock = SharedLock::new(home, &format!("waitlog-{}", self.folder));
        let _one = lock.hold();
        (self.files(home).iter().flat_map(|f| self.rows(f)))
            .filter(|o| name.is_none_or(|n| o["name"].as_str() == Some(n)))
            .collect()
    }

    /// Drop a person's waiting rows (the table's go with the sync's forget); how many. A file that is
    /// written anew gets a mark beside it: the sync sends it again from its start, since what was sent of
    /// it no longer says where the unsent lines begin.
    pub fn forget(&self, home: &Path, name: &str) -> std::io::Result<usize> {
        let lock = SharedLock::new(home, &format!("waitlog-{}", self.folder));
        let _one = lock.hold();
        let mut n = 0;
        for file in self.files(home) {
            let rows = self.rows(&file);
            let keep: Vec<&Value> = rows.iter().filter(|o| o["name"].as_str() != Some(name)).collect();
            if keep.len() < rows.len() {
                n += rows.len() - keep.len();
                atomic_write(
                    &file,
                    keep.iter().map(|o| filejson::dumps_utf8(o) + "\n").collect::<String>().as_bytes(),
                )?;
                let mut mark = file.clone().into_os_string();
                mark.push(".again");
                std::fs::write(mark, utc_now())?;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_persons_waiting_rows_are_found_and_dropped_and_the_file_marked() {
        let home = std::env::temp_dir().join(format!("face-home-waiting-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("music")).unwrap();
        let file = home.join("music").join("2026-11-10.jsonl");
        let lines = "{\"t\": 1, \"name\": \"Ann\", \"song\": \"S\", \"artist\": \"A\"}\r\nnot a line\n{\"t\": 2, \"name\": \"Bo\", \"song\": \"T\"}\n\
                     {\"t\": 3, \"name\": \"Ann\"}\n";
        std::fs::write(&file, lines).unwrap();
        assert_eq!(MUSIC.waiting(&home, None).len(), 2, "a line without a song is no row");
        assert_eq!(MUSIC.waiting(&home, Some("Ann")).len(), 1);
        assert!(EXPRESSION.waiting(&home, None).is_empty());
        assert_eq!(MUSIC.forget(&home, "Nobody").unwrap(), 0);
        assert!(
            !home.join("music").join("2026-11-10.jsonl.again").exists()
                && std::fs::read_to_string(&file).unwrap() == lines
        );
        assert_eq!(MUSIC.forget(&home, "Ann").unwrap(), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"t\": 2, \"name\": \"Bo\", \"song\": \"T\"}\n");
        assert!(home.join("music").join("2026-11-10.jsonl.again").exists());
        let _ = std::fs::remove_dir_all(home);
    }
}
