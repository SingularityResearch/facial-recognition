//! What a device remembers of its syncing, in `HOME/sync.sqlite`: the digest of every row as it was last
//! sent or taken, which database it has joined, and how far it has read that database's log of changes. The
//! file is the one every device has kept so far (SQLite, two tables),
//! so a device carries on from where it was.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use indexmap::IndexMap;
use rusqlite::{Connection, OptionalExtension, params};

pub struct State {
    db: Mutex<Connection>,
}

fn said(e: rusqlite::Error) -> String {
    format!("the sync's own memory (sync.sqlite) couldn't be used: {e}")
}

impl State {
    /// The state kept in the file at `path`, made if it isn't there.
    pub fn open(path: &Path) -> Result<State, String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(path).map_err(said)?;
        // (the log beside the file, so a reader never waits for a writer)
        db.pragma_update(None, "journal_mode", "WAL").map_err(said)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS digest (tbl TEXT, key TEXT, d TEXT, PRIMARY KEY (tbl, key));
             CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT);",
        )
        .map_err(said)?;
        Ok(State { db: Mutex::new(db) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One thing that is remembered by name (the database joined, how far its log was read, ...).
    pub fn get(&self, k: &str) -> Result<Option<String>, String> {
        self.db().query_row("SELECT v FROM meta WHERE k = ?1", [k], |r| r.get(0)).optional().map_err(said)
    }

    pub fn put(&self, k: &str, v: &str) -> Result<(), String> {
        self.db().execute("INSERT OR REPLACE INTO meta VALUES (?1, ?2)", params![k, v]).map(|_| ()).map_err(said)
    }

    pub fn drop(&self, k: &str) -> Result<(), String> {
        self.db().execute("DELETE FROM meta WHERE k = ?1", [k]).map(|_| ()).map_err(said)
    }

    /// {a row's key: its digest as last synced} for a table.
    pub fn digests(&self, tbl: &str) -> Result<HashMap<String, String>, String> {
        let db = self.db();
        let mut ask = db.prepare("SELECT key, d FROM digest WHERE tbl = ?1").map_err(said)?;
        let rows = ask.query_map([tbl], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(said)?;
        rows.collect::<Result<_, _>>().map_err(said)
    }

    /// Keep these digests for a table; a key with none is forgotten. All of them or none.
    pub fn set_digests(&self, tbl: &str, items: &HashMap<String, Option<String>>) -> Result<(), String> {
        let mut db = self.db();
        let tx = db.transaction().map_err(said)?;
        {
            let mut keep = tx.prepare("INSERT OR REPLACE INTO digest VALUES (?1, ?2, ?3)").map_err(said)?;
            let mut forget = tx.prepare("DELETE FROM digest WHERE tbl = ?1 AND key = ?2").map_err(said)?;
            for (key, digest) in items {
                match digest {
                    Some(d) => keep.execute(params![tbl, key, d]),
                    None => forget.execute(params![tbl, key]),
                }
                .map_err(said)?;
            }
        }
        tx.commit().map_err(said)
    }

    /// Forget everything (before joining another database).
    pub fn reset(&self) -> Result<(), String> {
        self.db().execute_batch("DELETE FROM digest; DELETE FROM meta;").map_err(said)
    }
}

/// For one table: the rows that changed here since they were last synced (new ones too), in the order they
/// are kept here, and the rows that were synced and are gone here. What a push sends.
pub fn pending(now: &IndexMap<String, String>, synced: &HashMap<String, String>) -> (Vec<String>, Vec<String>) {
    let changed: Vec<String> = now.iter().filter(|(k, d)| synced.get(*k) != Some(*d)).map(|(k, _)| k.clone()).collect();
    let mut gone: Vec<String> = synced.keys().filter(|k| !now.contains_key(*k)).cloned().collect();
    gone.sort();
    (changed, gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_synced_is_remembered_across_a_restart() {
        let dir = std::env::temp_dir().join(format!("face-sync-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("home").join("sync.sqlite");
        {
            let state = State::open(&path).unwrap();
            assert_eq!((state.get("instance").unwrap(), state.digests("faces").unwrap().len()), (None, 0));
            state.put("instance", "abc").unwrap();
            state.put("snapshot", "10:20:").unwrap();
            let rows =
                HashMap::from([("f1".to_string(), Some("d1".to_string())), ("f2".to_string(), Some("d2".to_string()))]);
            state.set_digests("faces", &rows).unwrap();
            state.set_digests("labels", &HashMap::from([("f1".to_string(), Some("x".to_string()))])).unwrap();
            // a digest replaced, one forgotten, one that was never there forgotten too
            let rows = HashMap::from([
                ("f1".to_string(), Some("d1b".to_string())),
                ("f2".to_string(), None),
                ("f9".to_string(), None),
            ]);
            state.set_digests("faces", &rows).unwrap();
            state.drop("snapshot").unwrap();
        }
        let state = State::open(&path).unwrap();
        assert_eq!(state.get("instance").unwrap().as_deref(), Some("abc"));
        assert_eq!(state.get("snapshot").unwrap(), None);
        assert_eq!(state.digests("faces").unwrap(), HashMap::from([("f1".to_string(), "d1b".to_string())]));
        assert_eq!(state.digests("labels").unwrap().len(), 1);
        // what a push would send: what is new or changed here, and what is gone here
        let now = IndexMap::from([("f3".to_string(), "d3".to_string()), ("f1".to_string(), "d1c".to_string())]);
        let synced = HashMap::from([("f1".to_string(), "d1b".to_string()), ("f2".to_string(), "d2".to_string())]);
        assert_eq!(pending(&now, &synced), (vec!["f3".to_string(), "f1".to_string()], vec!["f2".to_string()]));
        state.reset().unwrap();
        assert_eq!((state.get("instance").unwrap(), state.digests("faces").unwrap().len()), (None, 0));
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }
}
