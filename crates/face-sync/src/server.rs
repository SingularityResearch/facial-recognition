//! The database's side of the sync for the labeling data: rows sent (`push`) and taken (`fetch`), with
//! statements that are fixed, so that what devices of different versions write is the same
//! and each takes what the other wrote. The merging rules are in the statements: a label named by hand
//! beats an automatic one from another device; a face whose print was deleted somewhere doesn't get it back
//! from another device; photos and faces are never deleted from here.

use std::collections::HashMap;

use face_files::filejson;
use futures_util::future::try_join_all;
use ring::digest::{SHA256, digest};
use serde_json::{Map, Value, json};
use tokio_postgres::{Client, Statement, Transaction, types::ToSql};

use crate::rows::{FaceRow, PHOTO_FIELDS, Picture, Row, Rows, short};

type Result<T> = std::result::Result<T, String>;

fn said(e: tokio_postgres::Error) -> String {
    words(&e)
}

/// What a database error says: the server's own sentence where it gave one, with its code (the library's
/// word for every such error is "db error", which names nothing: a device whose sync never finished a step
/// said only that, 2026-11-22). The sentence, not the server's detail, which is where a row's values are.
pub(crate) fn words(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => {
            let sentence: String = db.message().chars().take(200).collect();
            format!("The shared database: {sentence} ({})", db.code().code())
        }
        None => format!("The shared database: {e}"),
    }
}

/// The column a table's rows are known by.
pub fn key_of(tbl: &str) -> Result<&'static str> {
    match tbl {
        "photos" => Ok("hash"),
        "faces" | "labels" | "web_suggestions" => Ok("fid"),
        _ => Err(format!("the sync has no place to put rows of {tbl:?}")),
    }
}

pub(crate) fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// A picture fingerprint as the database keeps it: its bytes, when the text is lower-case hex.
pub(crate) fn hex_bytes(v: Option<&Value>) -> Option<Vec<u8>> {
    let s = v?.as_str()?;
    if s.len() % 2 != 0 || !s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// One value of a statement, owned, so that many statements can be on their way at once.
pub(crate) type Param = Box<dyn ToSql + Sync + Send>;

/// Rows sent at a time without waiting for each one's answer: a row a round trip took twice as long
/// (measured).
const BATCH: usize = 300;

/// The same statement for many rows, in their order, a batch on its way at a time.
pub(crate) async fn run_all(tx: &Transaction<'_>, statement: &Statement, rows: &[Vec<Param>]) -> Result<()> {
    for part in rows.chunks(BATCH) {
        let asks = part.iter().map(|row| {
            let args: Vec<&(dyn ToSql + Sync)> = row.iter().map(|p| p.as_ref() as &(dyn ToSql + Sync)).collect();
            async move { tx.execute(statement, &args).await }
        });
        try_join_all(asks).await.map_err(said)?;
    }
    Ok(())
}

/// Send rows and deletions of one table, all of them or none.
pub async fn push(client: &mut Client, device: &str, tbl: &str, rows: &Rows, deletes: &[String]) -> Result<()> {
    let key = key_of(tbl)?;
    let tx = client.transaction().await.map_err(said)?;
    let own = |v: &String| -> Param { Box::new(v.clone()) };
    match tbl {
        "labels" => {
            let send = tx
                .prepare(
                    "INSERT INTO labels AS l (fid, n, by, s, ok, nope) VALUES ($1, $2, $3, $4::real, $5::bool, $6::text[])
                     ON CONFLICT (fid) DO UPDATE SET n = excluded.n, by = excluded.by, s = excluded.s,
                         ok = excluded.ok, nope = excluded.nope
                     WHERE NOT (l.by = 'me' AND excluded.by = 'auto' AND l.n IS DISTINCT FROM excluded.n)",
                )
                .await
                .map_err(said)?;
            let mut all = Vec::with_capacity(rows.len());
            for (fid, row) in rows {
                let Row::Plain(r) = row else { continue };
                let not: Option<Vec<String>> = r
                    .get("not")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect());
                let args: Vec<Param> = vec![
                    own(fid),
                    Box::new(text(r, "n")),
                    Box::new(text(r, "by")),
                    Box::new(r.get("s").and_then(Value::as_f64).map(|v| v as f32)),
                    Box::new(r.get("ok").and_then(Value::as_bool)),
                    Box::new(not),
                ];
                all.push(args);
            }
            run_all(&tx, &send, &all).await?;
        }
        "web_suggestions" => {
            let send = tx
                .prepare(
                    "INSERT INTO web_suggestions (fid, data) VALUES ($1, $2::text::jsonb)
                     ON CONFLICT (fid) DO UPDATE SET data = excluded.data",
                )
                .await
                .map_err(said)?;
            let mut all = Vec::with_capacity(rows.len());
            for (fid, row) in rows {
                let Row::Plain(r) = row else { continue };
                let args: Vec<Param> = vec![own(fid), Box::new(filejson::dumps(r))];
                all.push(args);
            }
            run_all(&tx, &send, &all).await?;
        }
        "photos" => {
            let send = tx
                .prepare(
                    "INSERT INTO photos (hash, name, size, fp, faces, dup_of, video, fps, maybe_read, extra, facts,
                                         taken, tags, facts_key)
                     VALUES ($1, $2, $3::bigint, $4::bytea, $5::int, $6, $7::bool, $8::real, $9::bool, $10::text::jsonb,
                             $11::text::jsonb, $12::float8, $13::text[], $14)
                     ON CONFLICT (hash) DO UPDATE SET name = excluded.name, size = excluded.size, fp = excluded.fp,
                         faces = excluded.faces, dup_of = excluded.dup_of, video = excluded.video, fps = excluded.fps,
                         maybe_read = photos.maybe_read OR excluded.maybe_read, extra = excluded.extra,
                         facts = coalesce(excluded.facts, photos.facts), taken = coalesce(excluded.taken, photos.taken),
                         tags = CASE WHEN excluded.facts IS NULL THEN photos.tags ELSE excluded.tags END,
                         facts_key = coalesce(excluded.facts_key, photos.facts_key)",
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
            let (mut all, mut paths, mut no_path) = (Vec::with_capacity(rows.len()), Vec::new(), Vec::new());
            for (hash, row) in rows {
                let Row::Plain(r) = row else { continue };
                let extra: Map<String, Value> = (r.as_object().into_iter().flatten())
                    .filter(|(k, _)| !PHOTO_FIELDS.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let facts = r.get("_facts").filter(|f| truthy(Some(f)));
                let tags: Vec<String> =
                    (facts.and_then(|f| f.get("tags")).and_then(Value::as_array).into_iter().flatten())
                        .filter_map(|t| t.as_str().map(str::to_string))
                        .collect();
                let args: Vec<Param> = vec![
                    own(hash),
                    Box::new(text(r, "name").unwrap_or_default()),
                    Box::new(r.get("size").and_then(Value::as_i64)),
                    Box::new(hex_bytes(r.get("fp"))),
                    Box::new(r.get("faces").and_then(Value::as_i64).map(|v| v as i32)),
                    Box::new(text(r, "dup_of")),
                    Box::new(truthy(r.get("video"))),
                    Box::new(r.get("fps").and_then(Value::as_f64).map(|v| v as f32)),
                    Box::new(r.get("maybe").is_some()),
                    // (numbers in the files' spelling: the database keeps them as written)
                    Box::new(filejson::dumps(&Value::Object(extra))),
                    Box::new(facts.map(filejson::dumps)),
                    Box::new(facts.and_then(|f| f.get("t")).and_then(Value::as_f64)),
                    Box::new(tags),
                    Box::new(text(r, "_key").filter(|k| k != "synced")),
                ];
                all.push(args);
                match text(r, "path").filter(|p| !p.is_empty()) {
                    Some(p) => {
                        let args: Vec<Param> = vec![
                            Box::new(device.to_string()),
                            own(hash),
                            Box::new(p),
                            Box::new(r.get("mtime").and_then(Value::as_f64)),
                        ];
                        paths.push(args);
                    }
                    None => no_path.push(hash.clone()),
                }
            }
            // the photos first: where a photo is can only be said of a photo the database has
            run_all(&tx, &send, &all).await?;
            run_all(&tx, &path, &paths).await?;
            if !no_path.is_empty() {
                tx.execute(
                    "DELETE FROM photo_paths WHERE device = $1::text::uuid AND hash = ANY($2)",
                    &[&device, &no_path],
                )
                .await
                .map_err(said)?;
            }
        }
        _ => {
            // a picture the server already has (the same SHA-256) isn't sent again
            let fids: Vec<&String> = rows.iter().map(|(k, _)| k).collect();
            let have: HashMap<String, Option<Vec<u8>>> =
                (tx.query("SELECT fid, picture_sha FROM faces WHERE fid = ANY($1)", &[&fids]).await.map_err(said)?)
                    .iter()
                    .map(|r| (r.get(0), r.get(1)))
                    .collect();
            // an Ignored face's print and picture don't come back from another device
            let send = tx
                .prepare(
                    "INSERT INTO faces (fid, photo, file, box, quality, model, print, picture, possible, frame)
                     VALUES ($1, $2, $3, $4::real[], $5::real, $6, $7::real[]::vector, $8::bytea, $9::real, $10::int)
                     ON CONFLICT (fid) DO UPDATE SET photo = excluded.photo, file = excluded.file, box = excluded.box,
                         quality = excluded.quality, model = excluded.model,
                         print = CASE WHEN faces.print IS NULL AND excluded.print IS NOT NULL
                                      AND faces.updated_by IS DISTINCT FROM $11::text::uuid THEN NULL ELSE excluded.print END,
                         picture = CASE WHEN faces.print IS NULL AND excluded.print IS NOT NULL
                                        AND faces.updated_by IS DISTINCT FROM $11::text::uuid THEN NULL
                                        WHEN $12::bool THEN faces.picture ELSE excluded.picture END,
                         possible = excluded.possible, frame = excluded.frame",
                )
                .await
                .map_err(said)?;
            let mut all = Vec::with_capacity(rows.len());
            for (fid, row) in rows {
                let Row::Face(f) = row else { continue };
                let picture = match &f.picture {
                    Picture::Jpeg(b) => Some(b),
                    _ => None,
                };
                let same = picture
                    .is_some_and(|b| have.get(fid).and_then(Option::as_deref) == Some(digest(&SHA256, b).as_ref()));
                let args: Vec<Param> = vec![
                    own(fid),
                    own(&f.photo),
                    own(&f.file),
                    Box::new(f.place.clone()),
                    Box::new(f.quality),
                    own(&f.model),
                    Box::new(f.print.clone()),
                    Box::new(picture.filter(|_| !same).cloned()),
                    Box::new(f.possible),
                    Box::new(f.frame),
                    Box::new(device.to_string()),
                    Box::new(same),
                ];
                all.push(args);
            }
            run_all(&tx, &send, &all).await?;
        }
    }
    // photos and faces are never deleted from here
    if !deletes.is_empty() && matches!(tbl, "labels" | "web_suggestions") {
        tx.execute(&format!("DELETE FROM {tbl} WHERE {key} = ANY($1)"), &[&deletes]).await.map_err(said)?;
    }
    tx.commit().await.map_err(said)
}

/// Every key the database has of a table.
pub async fn all_keys(client: &Client, tbl: &str) -> Result<Vec<String>> {
    let key = key_of(tbl)?;
    Ok(client.query(&format!("SELECT {key} FROM {tbl}"), &[]).await.map_err(said)?.iter().map(|r| r.get(0)).collect())
}

fn parsed(text: Option<String>) -> Value {
    text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn real(v: Option<f32>) -> Value {
    v.map_or(Value::Null, |v| json!(short(v)))
}

/// The database's rows for these keys, in the order asked: none for a key it doesn't have. `pictures`: the
/// SHA-256 of this device's picture of a face; a picture the database has the same of isn't fetched (its row
/// says `Picture::Same`).
pub async fn fetch(
    client: &Client,
    tbl: &str,
    keys: &[String],
    pictures: &HashMap<String, Vec<u8>>,
) -> Result<Vec<(String, Option<Row>)>> {
    key_of(tbl)?;
    let mut got: HashMap<String, Row> = HashMap::new();
    for part in keys.chunks(900) {
        let part: Vec<&String> = part.iter().collect();
        match tbl {
            "labels" => {
                let rows = client
                    .query("SELECT fid, n, by, s, ok, nope FROM labels WHERE fid = ANY($1)", &[&part])
                    .await
                    .map_err(said)?;
                for r in rows {
                    let row = json!({"n": r.get::<_, Option<String>>(1), "by": r.get::<_, Option<String>>(2), "s": real(r.get(3)),
                                     "ok": r.get::<_, Option<bool>>(4), "not": r.get::<_, Option<Vec<String>>>(5)});
                    got.insert(r.get(0), Row::Plain(row));
                }
            }
            "web_suggestions" => {
                let rows = client
                    .query("SELECT fid, data::text FROM web_suggestions WHERE fid = ANY($1)", &[&part])
                    .await
                    .map_err(said)?;
                for r in rows {
                    got.insert(r.get(0), Row::Plain(parsed(r.get(1))));
                }
            }
            "photos" => {
                let rows = client
                    .query(
                        "SELECT hash, name, size, fp, faces, dup_of, video, fps, maybe_read, extra::text, facts::text
                         FROM photos WHERE hash = ANY($1)",
                        &[&part],
                    )
                    .await
                    .map_err(said)?;
                for r in rows {
                    let fp: Option<Vec<u8>> = r.get(3);
                    let fp =
                        fp.filter(|b| !b.is_empty()).map(|b| b.iter().map(|v| format!("{v:02x}")).collect::<String>());
                    let extra = match parsed(r.get(9)) {
                        Value::Null => json!({}),
                        other => other,
                    };
                    let row = json!({"name": r.get::<_, String>(1), "size": r.get::<_, Option<i64>>(2), "fp": fp,
                                     "faces": r.get::<_, Option<i32>>(4), "dup_of": r.get::<_, Option<String>>(5),
                                     "video": if r.get::<_, bool>(6) { json!(true) } else { Value::Null }, "fps": real(r.get(7)),
                                     "maybe_read": r.get::<_, bool>(8), "extra": extra, "facts": parsed(r.get(10))});
                    got.insert(r.get(0), Row::Plain(row));
                }
            }
            _ => {
                let rows = client
                    .query(
                        "SELECT fid, photo, file, box, quality, model, print::real[], picture_sha, possible, frame
                         FROM faces WHERE fid = ANY($1)",
                        &[&part],
                    )
                    .await
                    .map_err(said)?;
                let mut want = Vec::new();
                for r in rows {
                    let fid: String = r.get(0);
                    let sha: Option<Vec<u8>> = r.get(7);
                    let picture = match &sha {
                        Some(sha) if pictures.get(&fid) == Some(sha) => Picture::Same,
                        Some(_) => {
                            want.push(fid.clone());
                            Picture::None
                        }
                        None => Picture::None,
                    };
                    let face = FaceRow {
                        photo: r.get(1),
                        file: r.get(2),
                        place: r.get(3),
                        quality: r.get(4),
                        model: r.get(5),
                        print: r.get(6),
                        picture,
                        possible: r.get(8),
                        frame: r.get(9),
                    };
                    got.insert(fid, Row::Face(Box::new(face)));
                }
                if !want.is_empty() {
                    for r in client
                        .query("SELECT fid, picture FROM faces WHERE fid = ANY($1)", &[&want])
                        .await
                        .map_err(said)?
                    {
                        let picture: Option<Vec<u8>> = r.get(1);
                        if let (Some(Row::Face(face)), Some(b)) =
                            (got.get_mut(r.get::<_, &str>(0)), picture.filter(|b| !b.is_empty()))
                        {
                            face.picture = Picture::Jpeg(b);
                        }
                    }
                }
            }
        }
    }
    Ok(keys.iter().map(|k| (k.clone(), got.remove(k))).collect())
}
