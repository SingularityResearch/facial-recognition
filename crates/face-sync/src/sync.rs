//! The sync's steps for the labeling data: send what changed here,
//! take what other devices changed, and, the first time with
//! a database, merge this device's data in and take everything.
//!
//! Which of the database's changes are new is decided by their transaction, not by their number in its log:
//! a number is taken when a row is written and seen when its transaction commits, so a long transaction (a
//! device joining) commits rows numbered below what a device looking meanwhile has already passed. A device
//! remembers the snapshot it read the log with, and next time takes every change whose transaction wasn't
//! visible in it.

use std::collections::HashMap;

use face_files::filejson;
use face_library::labels::LabelDb;
use ring::digest::{SHA256, digest};
use serde_json::{Map, Value, json};
use tokio_postgres::{Client, IsolationLevel, types::ToSql};

use crate::{
    apply,
    home::{self, Home},
    local::{self, TABLES},
    rows::{PHOTO_FIELDS, Row},
    server::{self, Param, run_all},
    state::{State, pending},
};

type Result<T> = std::result::Result<T, String>;

fn said(e: tokio_postgres::Error) -> String {
    crate::server::words(&e)
}

/// Rows left out that the log names, for each table and step; the rest are counted.
const BAD_SHOWN: usize = 5;

/// A key or a message from the database for a line of the log: one line, `n` characters at most (they are
/// anyone's text).
fn printable(text: &str, n: usize) -> String {
    text.chars().take(n).map(|c| if c.is_control() { '?' } else { c }).collect()
}

/// What a step did.
#[derive(Debug, Default, PartialEq)]
pub struct Did {
    /// rows sent, and rows deleted there
    pub pushed: usize,
    /// rows of other devices' changes looked at
    pub pulled: usize,
    /// this was the first step with this database: everything was merged in and taken
    pub joined: bool,
    /// something taken has to be sent back soon (a row this device keeps its own way)
    pub dirty: bool,
}

/// The library a device syncs, and its Web search answers: handed out only for as long as this device's own
/// data is read or written, never while the database is asked or answered. In a server that is the labeling
/// data's lock, so the pages don't wait for the network.
pub trait Library {
    fn with<R>(&self, f: impl FnOnce(&mut LabelDb, &mut Map<String, Value>) -> R) -> R;
}

/// A library nobody else uses meanwhile (a tool, a test).
pub struct Own<'a>(std::cell::RefCell<(&'a mut LabelDb, &'a mut Map<String, Value>)>);

impl<'a> Own<'a> {
    pub fn new(db: &'a mut LabelDb, web: &'a mut Map<String, Value>) -> Own<'a> {
        Own(std::cell::RefCell::new((db, web)))
    }
}

impl Library for Own<'_> {
    fn with<R>(&self, f: impl FnOnce(&mut LabelDb, &mut Map<String, Value>) -> R) -> R {
        let mut held = self.0.borrow_mut();
        let (db, web) = &mut *held;
        f(db, web)
    }
}

/// Send the rows that changed here since they were last synced and delete the ones that are gone here, then
/// the undo history's new lines; the labeling data first, then the private folder's (`home`, when that is
/// synced too). How many rows were sent and deleted.
pub async fn push_changed(
    client: &mut Client,
    device: &str,
    lib: &impl Library,
    state: &State,
    home: Option<&Home>,
) -> Result<usize> {
    // (every row's digest: a fifth of a second for a large library, the one long hold of a step)
    let now = lib.with(|db, web| local::digests(db, web));
    let mut sent = 0;
    for tbl in TABLES {
        let (changed, gone) = pending(&now[tbl], &state.digests(tbl)?);
        if changed.is_empty() && gone.is_empty() {
            continue;
        }
        let rows = lib.with(|db, web| local::rows(db, web, tbl, &changed));
        server::push(client, device, tbl, &rows, &gone).await?;
        let mut kept: HashMap<String, Option<String>> =
            rows.iter().map(|(k, _)| (k.clone(), now[tbl].get(k).cloned())).collect();
        kept.extend(gone.iter().map(|k| (k.clone(), None)));
        state.set_digests(tbl, &kept)?;
        sent += rows.len() + gone.len();
    }
    history(client, device, lib, state).await?;
    if let Some(home) = home {
        let now = home.digests()?;
        for tbl in home::TABLES {
            // (not there: not this device's to send now, the keys without the current passphrase)
            let Some(mine) = now.get(tbl) else { continue };
            let (changed, gone) = pending(mine, &state.digests(tbl)?);
            if changed.is_empty() && gone.is_empty() {
                continue;
            }
            let rows = home.rows(tbl, &changed)?;
            if !home.push(client, tbl, &rows, &gone).await? {
                continue; // held back: they stay as not sent
            }
            let mut kept: HashMap<String, Option<String>> =
                rows.iter().map(|(k, _)| (k.clone(), now[tbl].get(k).cloned())).collect();
            kept.extend(gone.iter().map(|k| (k.clone(), None)));
            state.set_digests(tbl, &kept)?;
            sent += rows.len() + gone.len();
        }
        home.after_push(client, state).await?;
    }
    Ok(sent)
}

/// How many rows changed here and are not sent yet.
pub fn waiting(lib: &impl Library, state: &State, home: Option<&Home>) -> Result<usize> {
    let now = lib.with(|db, web| local::digests(db, web));
    let mut n = 0;
    for tbl in TABLES {
        let synced = state.digests(tbl)?;
        n += now[tbl].iter().filter(|(k, d)| synced.get(*k) != Some(*d)).count();
    }
    if let Some(home) = home {
        for (tbl, mine) in home.digests()? {
            let synced = state.digests(tbl)?;
            n += mine.iter().filter(|(k, d)| synced.get(*k) != Some(*d)).count();
        }
    }
    Ok(n)
}

/// The undo history's lines (`data/history.jsonl`), as the database's `label_history` gets them.
fn history_lines(db: &LabelDb) -> Vec<String> {
    std::fs::read_to_string(db.data().join("history.jsonl"))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// One line of the undo history as its row: (when, what, the faces), or none for a line that can't be read.
/// A line without a time that can be read gets the time it is sent at.
fn history_row(line: &str) -> Option<(Option<String>, String, String)> {
    let h: Value = serde_json::from_str(line).ok().filter(Value::is_object)?;
    let when = h.get("t").and_then(Value::as_str).and_then(local::utc_time);
    let faces = h.get("faces").filter(|f| server::truthy(Some(f))).cloned().unwrap_or_else(|| json!({}));
    Some((when, h.get("what").and_then(Value::as_str).unwrap_or("").to_string(), filejson::dumps(&faces)))
}

const HISTORY: &str = "INSERT INTO label_history (t, device, what, faces)
                       VALUES (coalesce($1::text::timestamptz, now()), $2::text::uuid, $3, $4::text::jsonb)";

fn history_params(device: &str, rows: Vec<(Option<String>, String, String)>) -> Vec<Vec<Param>> {
    (rows.into_iter())
        .map(|(when, what, faces)| -> Vec<Param> {
            vec![Box::new(when), Box::new(device.to_string()), Box::new(what), Box::new(faces)]
        })
        .collect()
}

/// The undo history's new lines, added to the database's (this device's own log: sent, never taken).
async fn history(client: &mut Client, device: &str, lib: &impl Library, state: &State) -> Result<()> {
    let lines = lib.with(|db, _| history_lines(db));
    let mut done: usize = state.get("history_lines")?.and_then(|v| v.parse().ok()).unwrap_or(0);
    if lines.len() < done {
        done = lines.len(); // the file was written anew (a person forgotten): counting starts again
    }
    let new: Vec<_> = lines[done..].iter().filter_map(|l| history_row(l)).collect();
    if !new.is_empty() {
        let tx = client.transaction().await.map_err(said)?;
        let add = tx.prepare(HISTORY).await.map_err(said)?;
        run_all(&tx, &add, &history_params(device, new)).await?;
        tx.commit().await.map_err(said)?;
    }
    state.put("history_lines", &lines.len().to_string())
}

/// The SHA-256 of this device's picture of each of these faces that it has one of.
fn pictures(db: &LabelDb, keys: &[String]) -> HashMap<String, Vec<u8>> {
    (keys.iter())
        .filter_map(|k| Some((k, &db.store.thumbs[db.store.index_of(k)?])))
        .filter(|(_, t)| !t.is_empty())
        .map(|(k, t)| (k.clone(), digest(&SHA256, t).as_ref().to_vec()))
        .collect()
}

/// Take these rows of a table from the database into the library. A row this device can't use is left out
/// and said (the first few; `left` counts them for the table); it stops nothing, and nothing is remembered
/// about it, so its next change is looked at again. Whether something has to be sent back.
#[allow(clippy::too_many_arguments)]
async fn take(
    client: &Client,
    lib: &impl Library,
    state: &State,
    tbl: &str,
    keys: &[String],
    left: &mut usize,
    log: &mut dyn FnMut(String),
) -> Result<bool> {
    let known = if tbl == "faces" { lib.with(|db, _| pictures(db, keys)) } else { HashMap::new() };
    let got = server::fetch(client, tbl, keys, &known).await?;
    let mut there: HashMap<String, bool> = HashMap::new();
    // looked at and taken in one hold: nothing changes between the two
    let now_here = lib.with(|db, web| {
        let mut good = Vec::with_capacity(got.len());
        for (key, row) in got {
            match row.as_ref().map(|r| apply::check(db, tbl, &key, r)) {
                Some(Err(why)) => left_out(tbl, &key, &why, left, log),
                _ => {
                    there.insert(key.clone(), row.is_some());
                    good.push((key, row));
                }
            }
        }
        apply::apply(db, web, tbl, good, log)
    })?;
    let dirty = now_here.iter().any(|(k, d)| d.is_none() && there.get(k) == Some(&true));
    state.set_digests(tbl, &now_here)?;
    Ok(dirty)
}

/// A row that can't be used here: counted for its table, and said while they are few.
fn left_out(tbl: &str, key: &str, why: &str, left: &mut usize, log: &mut dyn FnMut(String)) {
    *left += 1;
    if *left <= BAD_SHOWN {
        log(format!(
            "Shared database: the {tbl} row {} can't be used here and was left out ({}).",
            printable(key, 80),
            printable(why, 120)
        ));
    }
}

/// `take` for a table of the private folder.
async fn take_home(
    client: &Client,
    home: &mut Home,
    state: &State,
    tbl: &str,
    keys: &[String],
    left: &mut usize,
    log: &mut dyn FnMut(String),
) -> Result<bool> {
    let got = home.fetch(client, tbl, keys).await?;
    let mut there: HashMap<String, bool> = HashMap::new();
    let mut good = Vec::with_capacity(got.len());
    for (key, row) in got {
        match row.as_ref().map(|r| home.check(tbl, &key, r)) {
            Some(Err(why)) => left_out(tbl, &key, &why, left, log),
            _ => {
                there.insert(key.clone(), row.is_some());
                good.push((key, row));
            }
        }
    }
    let now_here = home.apply(tbl, good, log)?;
    // (a row this device keeps its own way, its settings: sent back soon)
    let dirty = now_here.iter().any(|(k, d)| d.is_none() && there.get(k) == Some(&true));
    state.set_digests(tbl, &now_here)?;
    Ok(dirty)
}

fn say_left_out(left: &HashMap<&str, usize>, log: &mut dyn FnMut(String)) {
    for (tbl, n) in left {
        if *n > BAD_SHOWN {
            log(format!("Shared database: {} more {tbl} rows couldn't be used here and were left out.", n - BAD_SHOWN));
        }
    }
}

/// Every row of every table, taken here. The snapshot taken before the first: the next look takes whatever
/// was committed after it (some of it twice, which changes nothing).
pub async fn take_all(
    client: &Client,
    lib: &impl Library,
    state: &State,
    home: Option<&mut Home>,
    log: &mut dyn FnMut(String),
) -> Result<String> {
    let snapshot: String = client.query_one("SELECT pg_current_snapshot()::text", &[]).await.map_err(said)?.get(0);
    let mut left = HashMap::new();
    for tbl in TABLES {
        let keys = server::all_keys(client, tbl).await?;
        for part in keys.chunks(2000) {
            take(client, lib, state, tbl, part, left.entry(tbl).or_insert(0), log).await?;
        }
        // a row only this device has (added while joining) differs from what is remembered: it is sent next time
    }
    if let Some(home) = home {
        for tbl in home::TABLES {
            let keys = home.all_keys(client, tbl).await?;
            for part in keys.chunks(2000) {
                take_home(client, home, state, tbl, part, left.entry(tbl).or_insert(0), log).await?;
            }
        }
    }
    say_left_out(&left, log);
    Ok(snapshot)
}

const NEWEST: &str = "SELECT tbl, key, (array_agg(device ORDER BY seq DESC))[1]::text FROM changes WHERE ";

/// Take the rows other devices changed since the last look. (How many rows were looked at, whether something
/// has to be sent back soon.)
#[allow(clippy::too_many_arguments)]
pub async fn pull(
    client: &mut Client,
    device: &str,
    lib: &impl Library,
    state: &State,
    home: Option<&mut Home>,
    log: &mut dyn FnMut(String),
) -> Result<(usize, bool)> {
    let seen = state.get("snapshot")?;
    let mut tables: Vec<&str> = TABLES.to_vec();
    if home.is_some() {
        tables.extend(home::TABLES);
    }
    // the snapshot and the rows of the log: one view of the database
    let tx = client.build_transaction().isolation_level(IsolationLevel::RepeatableRead).start().await.map_err(said)?;
    let (now, rows) = match &seen {
        // the first look since the log says which transaction wrote each change: by its numbers this once
        None => {
            let now: String = tx.query_one("SELECT pg_current_snapshot()::text", &[]).await.map_err(said)?.get(0);
            let from: i64 = state.get("cursor")?.and_then(|v| v.parse().ok()).unwrap_or(0);
            let ask = format!("{NEWEST}seq > $1 AND tbl = ANY($2) GROUP BY tbl, key");
            (now, Some(tx.query(&ask, &[&from, &tables]).await.map_err(said)?))
        }
        Some(seen) => {
            let ask = "SELECT s::text, pg_snapshot_xmax(s)::text::bigint < pg_snapshot_xmin($1::text::pg_snapshot)::text::bigint
                       FROM pg_current_snapshot() AS s";
            let row = tx.query_one(ask, &[seen]).await.map_err(said)?;
            let (now, back): (String, bool) = (row.get(0), row.get(1));
            if now == *seen {
                return Ok((0, false));
            }
            let ask = format!(
                "{NEWEST}txid >= pg_snapshot_xmin($1::text::pg_snapshot)::text::bigint
                 AND txid < pg_snapshot_xmax($2::text::pg_snapshot)::text::bigint
                 AND NOT pg_visible_in_snapshot(txid::text::xid8, $1::text::pg_snapshot)
                 AND tbl = ANY($3) GROUP BY tbl, key"
            );
            let rows = if back {
                None
            } else {
                let args: [&(dyn ToSql + Sync); 3] = [seen, &now, &tables];
                Some(tx.query(&ask, &args).await.map_err(said)?)
            };
            (now, rows)
        }
    };
    tx.commit().await.map_err(said)?;
    let Some(rows) = rows else {
        // restored from a backup onto another server: its transaction numbers started again
        log("Shared database: its transaction numbers went back (moved or restored?): taking everything again."
            .to_string());
        push_changed(client, device, lib, state, home.as_deref()).await?;
        let snapshot = take_all(client, lib, state, home, log).await?;
        state.put("snapshot", &snapshot)?;
        return Ok((0, false));
    };
    let mut want: HashMap<String, Vec<String>> = HashMap::new();
    for row in &rows {
        // the newest change of this row is another device's
        if row.get::<_, Option<String>>(2).as_deref() != Some(device) {
            want.entry(row.get(0)).or_default().push(row.get(1));
        }
    }
    let (mut got, mut dirty, mut left) = (0, false, HashMap::new());
    // in the tables' own order: photos before faces before labels
    for tbl in TABLES {
        if let Some(keys) = want.get(tbl) {
            dirty |= take(client, lib, state, tbl, keys, left.entry(tbl).or_insert(0), log).await?;
            got += keys.len();
        }
    }
    if let Some(home) = home {
        for tbl in home::TABLES {
            if let Some(keys) = want.get(tbl) {
                dirty |= take_home(client, home, state, tbl, keys, left.entry(tbl).or_insert(0), log).await?;
                got += keys.len();
            }
        }
    }
    say_left_out(&left, log);
    state.put("snapshot", &now)?;
    Ok((got, dirty))
}

/// What a device merges into a database it joins, made ready: each table's rows as the statements take them.
struct Merge {
    photos: Vec<Vec<Param>>,
    paths: Vec<Vec<Param>>,
    faces: Vec<Vec<Param>>,
    /// faces whose print was deleted here
    gone: Vec<String>,
    labels: Vec<Vec<Param>>,
    history: Vec<(Option<String>, String, String)>,
    answers: Vec<Vec<Param>>,
}

fn merge_rows(db: &LabelDb, web: &Map<String, Value>, device: &str) -> Merge {
    let s = &db.store;
    // photos, where this device has them, and what they say about themselves
    let text = |v: &Value, k: &str| server::text(v, k).filter(|t| !t.is_empty());
    let (mut photos, mut paths, mut tag_names) = (Vec::with_capacity(s.photos.len()), Vec::new(), HashMap::new());
    for (hash, r) in &s.photos {
        let facts = db.facts.facts.get(hash);
        tag_names.extend(facts.into_iter().flat_map(|f| f.faces.iter().map(|(i, n)| (s.fid[*i].clone(), n.clone()))));
        let file_name = text(r, "name").or_else(|| {
            let p = text(r, "path")?;
            Some(p.rsplit(['/', '\\']).next().unwrap_or("").to_string())
        });
        let extra: Map<String, Value> = (r.as_object().into_iter().flatten())
            .filter(|(k, _)| !PHOTO_FIELDS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let args: Vec<Param> = vec![
            Box::new(hash.clone()),
            Box::new(file_name.unwrap_or_default()),
            Box::new(r.get("size").and_then(Value::as_i64)),
            Box::new(server::hex_bytes(r.get("fp"))),
            Box::new(r.get("faces").and_then(Value::as_i64).map(|v| v as i32)),
            Box::new(server::text(r, "dup_of")),
            Box::new(server::truthy(r.get("video"))),
            Box::new(r.get("fps").and_then(Value::as_f64).map(|v| v as f32)),
            Box::new(r.get("maybe").is_some()),
            Box::new(facts.and_then(|f| f.t)),
            Box::new(facts.map(|f| f.tags.clone()).unwrap_or_default()),
            Box::new(facts.and_then(|f| f.key.clone())),
            Box::new(filejson::dumps(&Value::Object(extra))),
            Box::new(local::facts(db, hash).map(|f| filejson::dumps(&f))),
        ];
        photos.push(args);
        if let Some(p) = text(r, "path") {
            let args: Vec<Param> = vec![
                Box::new(device.to_string()),
                Box::new(hash.clone()),
                Box::new(p),
                Box::new(r.get("mtime").and_then(Value::as_f64)),
            ];
            paths.push(args);
        }
    }
    // faces: prints and pictures
    let (mut faces, mut gone) = (Vec::with_capacity(s.len()), Vec::new());
    for i in 0..s.len() {
        let fid = &s.fid[i];
        let record = s.photos.get(&s.photo[i]);
        let of_face = |what: &str| record.and_then(|r| r.get(what)).and_then(|m| m.get(fid));
        let printless = s.printless(i);
        if printless {
            gone.push(fid.clone());
        }
        let args: Vec<Param> = vec![
            Box::new(fid.clone()),
            Box::new(s.photo[i].clone()),
            Box::new(s.file[i].clone()),
            Box::new(s.boxes[i].to_vec()),
            Box::new(Some(s.quality[i]).filter(|q| !q.is_nan())),
            Box::new(s.model.clone()),
            Box::new((!printless).then(|| s.print(i).to_vec())),
            Box::new(Some(s.thumbs[i].clone()).filter(|t| !printless && !t.is_empty())),
            Box::new(of_face("maybe").and_then(Value::as_f64).map(|v| v as f32)),
            Box::new(of_face("at").and_then(Value::as_i64).map(|v| v as i32)),
            Box::new(tag_names.get(fid).cloned()),
        ];
        faces.push(args);
    }
    let mut labels = Vec::with_capacity(db.labels.len());
    for (fid, v) in &db.labels {
        let not: Option<Vec<String>> = v
            .get("not")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect());
        let args: Vec<Param> = vec![
            Box::new(fid.clone()),
            Box::new(server::text(v, "n")),
            Box::new(server::text(v, "by")),
            Box::new(v.get("s").and_then(Value::as_f64).map(|x| x as f32)),
            Box::new(v.get("ok").and_then(Value::as_bool)),
            Box::new(not),
        ];
        labels.push(args);
    }
    let history = history_lines(db).iter().filter_map(|l| history_row(l)).collect();
    let mut answers = Vec::with_capacity(web.len());
    for (fid, row) in local::rows(db, web, "web_suggestions", &web.keys().cloned().collect::<Vec<_>>()) {
        if let Row::Plain(v) = row {
            let args: Vec<Param> = vec![Box::new(fid), Box::new(filejson::dumps(&v))];
            answers.push(args);
        }
    }
    Merge { photos, paths, faces, gone, labels, history, answers }
}

/// Merge this device's labeling data into a database it joins.
/// What is in the database already is merged, not replaced: faces and photos are the same on
/// every device (their keys come from the photo), a label named by hand beats an automatic one and between
/// two of a kind the one already there stays, and a face Ignored anywhere stays Ignored. (table: rows) sent.
pub async fn merge_in(client: &mut Client, device: &str, name: &str, lib: &impl Library) -> Result<Map<String, Value>> {
    let merge = lib.with(|db, web| merge_rows(db, web, device));
    let mut counts = Map::new();
    let tx = client.transaction().await.map_err(said)?;
    tx.execute(
        "INSERT INTO devices (id, name, os) VALUES ($1::text::uuid, $2, 'imported') ON CONFLICT (id) DO UPDATE SET name = excluded.name",
        &[&device, &name],
    )
    .await
    .map_err(said)?;
    let photo = tx
        .prepare(
            "INSERT INTO photos (hash, name, size, fp, faces, dup_of, video, fps, maybe_read, taken, tags, facts_key, extra, facts)
             VALUES ($1, $2, $3::bigint, $4::bytea, $5::int, $6, $7::bool, $8::real, $9::bool, $10::float8, $11::text[], $12,
                     $13::text::jsonb, $14::text::jsonb)
             ON CONFLICT (hash) DO UPDATE SET
                 size = coalesce(photos.size, excluded.size), fp = coalesce(photos.fp, excluded.fp),
                 faces = greatest(photos.faces, excluded.faces), dup_of = coalesce(photos.dup_of, excluded.dup_of),
                 fps = coalesce(photos.fps, excluded.fps), maybe_read = photos.maybe_read OR excluded.maybe_read,
                 taken = coalesce(photos.taken, excluded.taken),
                 tags = ARRAY(SELECT DISTINCT unnest(photos.tags || excluded.tags)),
                 facts_key = coalesce(photos.facts_key, excluded.facts_key),
                 extra = excluded.extra || photos.extra,
                 facts = coalesce(photos.facts, excluded.facts)",
        )
        .await
        .map_err(said)?;
    let path = tx
        .prepare(
            "INSERT INTO photo_paths (device, hash, path, mtime) VALUES ($1::text::uuid, $2, $3, $4::float8)
             ON CONFLICT (device, hash) DO UPDATE SET path = excluded.path, mtime = excluded.mtime",
        )
        .await
        .map_err(said)?;
    // a batch of rows on its way at a time; the photos first: where one is can only be said of a photo there
    run_all(&tx, &photo, &merge.photos).await?;
    run_all(&tx, &path, &merge.paths).await?;
    counts.insert("photos".to_string(), json!(merge.photos.len()));
    counts.insert("photo paths".to_string(), json!(merge.paths.len()));

    let face = tx
        .prepare(
            "INSERT INTO faces (fid, photo, file, box, quality, model, print, picture, possible, frame, fact_name)
             VALUES ($1, $2, $3, $4::real[], $5::real, $6, $7::real[]::vector, $8::bytea, $9::real, $10::int, $11)
             ON CONFLICT (fid) DO UPDATE SET
                 quality = coalesce(faces.quality, excluded.quality),
                 picture = CASE WHEN faces.print IS NULL THEN NULL ELSE coalesce(faces.picture, excluded.picture) END,
                 possible = coalesce(faces.possible, excluded.possible),
                 frame = coalesce(faces.frame, excluded.frame),
                 fact_name = coalesce(faces.fact_name, excluded.fact_name)",
        )
        .await
        .map_err(said)?;
    run_all(&tx, &face, &merge.faces).await?;
    // a face Ignored on this device is Ignored everywhere
    if !merge.gone.is_empty() {
        tx.execute(
            "UPDATE faces SET print = NULL, picture = NULL WHERE fid = ANY($1) AND print IS NOT NULL",
            &[&merge.gone],
        )
        .await
        .map_err(said)?;
    }
    counts.insert("faces".to_string(), json!(merge.faces.len()));
    counts.insert("printless".to_string(), json!(merge.gone.len()));

    // labels: by hand beats automatic; else the one already there stays; the names a face is not are joined
    let label = tx
        .prepare(
            "INSERT INTO labels AS l (fid, n, by, s, ok, nope) VALUES ($1, $2, $3, $4::real, $5::bool, $6::text[])
             ON CONFLICT (fid) DO UPDATE SET
                 n = CASE WHEN l.n IS NULL OR (l.by = 'auto' AND excluded.by = 'me') THEN excluded.n ELSE l.n END,
                 by = CASE WHEN l.n IS NULL OR (l.by = 'auto' AND excluded.by = 'me') THEN excluded.by ELSE l.by END,
                 s = CASE WHEN l.n IS NULL OR (l.by = 'auto' AND excluded.by = 'me') THEN excluded.s ELSE l.s END,
                 ok = coalesce(l.ok, excluded.ok),
                 nope = (SELECT array_agg(DISTINCT x) FROM unnest(coalesce(l.nope, '{}') || coalesce(excluded.nope, '{}')) x)",
        )
        .await
        .map_err(said)?;
    run_all(&tx, &label, &merge.labels).await?;
    tx.execute("UPDATE faces SET print = NULL, picture = NULL WHERE print IS NOT NULL AND fid IN (SELECT fid FROM labels WHERE n = '#ignore')", &[])
        .await
        .map_err(said)?;
    counts.insert("labels".to_string(), json!(merge.labels.len()));

    // the undo history: each line once (the device's lines are only added when it has none there yet)
    let has = tx
        .query_opt("SELECT 1 FROM label_history WHERE device = $1::text::uuid LIMIT 1", &[&device])
        .await
        .map_err(said)?
        .is_some();
    let mut added = 0;
    if !has {
        let add = tx.prepare(HISTORY).await.map_err(said)?;
        added = merge.history.len();
        run_all(&tx, &add, &history_params(device, merge.history)).await?;
    }
    counts.insert("history lines".to_string(), json!(added));

    // Web search answers: the first one stays
    let answer = tx
        .prepare("INSERT INTO web_suggestions (fid, data) VALUES ($1, $2::text::jsonb) ON CONFLICT DO NOTHING")
        .await
        .map_err(said)?;
    run_all(&tx, &answer, &merge.answers).await?;
    counts.insert("web suggestions".to_string(), json!(merge.answers.len()));
    tx.commit().await.map_err(said)?;
    Ok(counts)
}

/// One step: the first time with a database (told by its own ID) merge this device's data in and take
/// everything; else send what changed here (`send`: something was saved, or it is a minute since the last
/// look: making every row's digest is not for every step) and take what other devices changed. `home`: the
/// private folder's part, when that is synced too.
#[allow(clippy::too_many_arguments)]
pub async fn step(
    client: &mut Client,
    device: &str,
    name: &str,
    lib: &impl Library,
    state: &State,
    mut home: Option<&mut Home>,
    send: bool,
    log: &mut dyn FnMut(String),
) -> Result<Did> {
    let database: String = client.query_one("SELECT id::text FROM instance", &[]).await.map_err(said)?.get(0);
    if state.get("instance")?.as_deref() != Some(database.as_str()) {
        // every file written as it is in memory, so that what is merged in is the newest
        lib.with(|db, _| -> Result<()> {
            db.labels_changed().map_err(|e| e.to_string())?;
            db.save_store().map_err(|e| e.to_string())?;
            db.facts.save(&db.store).map_err(|e| format!("the library couldn't be saved: {e}"))
        })?;
        state.reset()?;
        let counts = merge_in(client, device, name, lib).await?;
        if let Some(home) = home.as_deref() {
            home.merge_in(client).await?;
        }
        let snapshot = take_all(client, lib, state, home.as_deref_mut(), log).await?;
        state.put("history_lines", &lib.with(|db, _| history_lines(db)).len().to_string())?;
        if let Some(home) = home.as_deref() {
            home.on_join(state)?;
        }
        state.put("snapshot", &snapshot)?;
        state.put("instance", &database)?;
        log(format!(
            "Joined the shared database: sent {} faces and {} labels; this device now has {} faces.",
            counts["faces"],
            counts["labels"],
            lib.with(|db, _| db.n())
        ));
        return Ok(Did { joined: true, dirty: true, ..Did::default() });
    }
    let pushed = if send { push_changed(client, device, lib, state, home.as_deref()).await? } else { 0 };
    // the passphrase for keys: taken when this device may, shown that it is held
    if let Some(home) = home.as_deref_mut() {
        home.every_step(client, state, &database, log).await?;
        home.new_devices_now_and_then(client, log).await?;
    }
    let (pulled, dirty) = pull(client, device, lib, state, home, log).await?;
    client.execute("UPDATE devices SET last_seen = now() WHERE id = $1::text::uuid", &[&device]).await.map_err(said)?;
    Ok(Did { pushed, pulled, joined: false, dirty })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_the_undo_history_is_its_row() {
        let row = |line: &str| history_row(line);
        let full = r#"{"t": "2026-11-12T10:20:31Z", "what": "named 2 faces", "faces": {"f1": {"n": "Ann", "by": "me"}, "f2": null}}"#;
        assert_eq!(
            row(full),
            Some((
                Some("2026-11-12T10:20:31Z".to_string()),
                "named 2 faces".to_string(),
                r#"{"f1": {"n": "Ann", "by": "me"}, "f2": null}"#.to_string()
            ))
        );
        // a time with an offset goes out as UTC; no faces is an empty object, as the table wants one
        assert_eq!(
            row(r#"{"t": "2026-11-12T12:20:31+02:00", "what": "undo", "faces": null}"#),
            Some((Some("2026-11-12T10:20:31Z".to_string()), "undo".to_string(), "{}".to_string()))
        );
        // a time that can't be read: none, and the database takes the time the line is sent at
        assert_eq!(row(r#"{"t": "some day", "what": "x"}"#), Some((None, "x".to_string(), "{}".to_string())));
        assert_eq!(row(r#"{"faces": {}}"#), Some((None, String::new(), "{}".to_string())));
        // what isn't a line of the history is passed over
        for line in ["", "not a line", "[1, 2]", "3", r#"{"t": "2026"#] {
            assert_eq!(row(line), None, "{line}");
        }
    }

    #[test]
    fn anyones_text_is_one_short_line_in_the_log() {
        assert_eq!(printable("a face's key", 80), "a face's key");
        assert_eq!(printable("two\nlines\tand a tab", 80), "two?lines?and a tab");
        assert_eq!(printable(&"x".repeat(500), 120).len(), 120);
        assert_eq!(printable("né", 2), "né");
    }

    #[test]
    fn the_history_file_is_read_as_lines_and_a_missing_one_as_none() {
        let dir = std::env::temp_dir().join(format!("face-sync-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db =
            LabelDb::open(&dir.join("data"), &dir.join("home"), face_core::faces::REC_MODELS[0], false, &mut |_| {})
                .unwrap();
        assert!(history_lines(&db).is_empty());
        std::fs::write(dir.join("data").join("history.jsonl"), "{\"what\": \"a\"}\nnot a line\n{\"what\": \"b\"}\n")
            .unwrap();
        let lines = history_lines(&db);
        // every line counts for where the sending is, also one that can't be read
        assert_eq!(lines.len(), 3);
        assert_eq!(lines.iter().filter_map(|l| history_row(l)).map(|r| r.1).collect::<Vec<_>>(), ["a", "b"]);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
