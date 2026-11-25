//! The signal history in the shared database (`signal_log`, migration 16): every signal a device reads, as
//! one row a stretch of time for the device itself and for each named person in view, for the devices whose
//! owner switched that on. The devices only ever send rows, many at a time in ONE statement (a row a
//! statement was a round trip a row, on a connection that other work would then wait for); each device
//! deletes its own rows that are older than the days its owner set there.
//!
//! The library's rows (`send_library`): what a photo or a video's frame says of each named person in it,
//! under the device `LIBRARY` (they are the library's, whichever device worked them out) and the picture's
//! own time. They follow the library's labels: the rows of a picture are made again whenever they are worked
//! out again, and a row whose face is named otherwise since goes.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use tokio_postgres::Client;

use crate::Result;

/// rows in one statement at the most
pub const MOST: usize = 500;
/// the "device" of the library's rows
pub const LIBRARY: &str = "00000000-0000-0000-0000-000000000000";

/// What went wrong, in the server's own words where it said any (the library's own text for an error of
/// the server's is two words that name nothing).
fn why(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map_or_else(|| e.to_string(), |d| d.message().to_string())
}

fn an_id(device: &str) -> Result<()> {
    uuid::Uuid::parse_str(device).map(|_| ()).map_err(|_| "that is not a device's id".to_string())
}

/// One row as it waits on a device: {t, name, s, n, data}. Nothing for what is no row.
fn parts(row: &Value) -> Option<(String, i64, f32, i32, String)> {
    let name = row.get("name")?.as_str()?.to_string();
    let t = row.get("t")?.as_i64().filter(|t| *t > 0)?;
    let seconds = row.get("s")?.as_f64().filter(|s| s.is_finite() && *s > 0.0)? as f32;
    let readings = i32::try_from(row.get("n")?.as_i64()?).ok().filter(|n| *n > 0)?;
    let data = row.get("data").filter(|d| d.is_object())?;
    // (PostgreSQL takes no text with a zero byte, and one such row would hold every later one up)
    let data = data.to_string();
    (!name.contains('\0') && !data.contains("\\u0000")).then_some((name, t, seconds, readings, data))
}

/// Put a device's rows into the history, all in one statement. A row that is there already changes nothing,
/// and what is no row is left out. How many rows the database took as new.
pub async fn send(client: &Client, device: &str, rows: &[Value]) -> Result<u64> {
    an_id(device)?;
    let (mut names, mut times, mut seconds, mut readings, mut data) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (name, t, s, n, d) in rows.iter().take(MOST).filter_map(parts) {
        names.push(name);
        times.push(t);
        seconds.push(s);
        readings.push(n);
        data.push(d);
    }
    if names.is_empty() {
        return Ok(0);
    }
    client
        .execute(
            "INSERT INTO signal_log (device, name, at, seconds, readings, data)
             SELECT $1::text::uuid, r.name, 'epoch'::timestamptz + r.t * interval '1 millisecond', r.s, r.n, r.data::jsonb
             FROM unnest($2::text[], $3::bigint[], $4::real[], $5::int[], $6::text[]) AS r(name, t, s, n, data)
             ON CONFLICT (device, name, at, ref) DO NOTHING",
            &[&device, &names, &times, &seconds, &readings, &data],
        )
        .await
        .map_err(|e| format!("The signal history couldn't be written: {}", why(&e)))
}

/// One row worked out from a picture of the library: {t, name, source, ref, data}. Nothing for what is no
/// row: a row without a name (the library has no row of its own), another source, a ref that is no
/// picture's content key.
fn library_parts(row: &Value) -> Option<(String, i64, String, String, String)> {
    let name = row.get("name")?.as_str().filter(|n| !n.is_empty() && !n.contains('\0'))?.to_string();
    let t = row.get("t")?.as_i64()?;
    let source = row.get("source")?.as_str().filter(|s| matches!(*s, "photo" | "video"))?.to_string();
    let picture = row.get("ref")?.as_str().filter(|r| a_picture(r))?.to_string();
    let data = row.get("data").filter(|d| d.is_object())?.to_string();
    (!data.contains("\\u0000")).then_some((name, t, source, picture, data))
}

/// A picture's key as the library has them: letters and digits, and a hyphen in some.
fn a_picture(key: &str) -> bool {
    !key.is_empty() && key.len() <= 128 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Put what these pictures of the library say into the history, all in ONE statement: their rows are written
/// (a row that is there already is changed only where it says something else), and a row one of these
/// pictures had that is no longer among them goes (its face is named otherwise now, or not at all).
/// `pictures`: every picture looked at, also one that has no row now. Of two rows for one person at one
/// moment of one picture the first counts. How many rows were written or changed.
pub async fn send_library(client: &Client, pictures: &[String], rows: &[Value]) -> Result<u64> {
    let pictures: Vec<&str> = pictures.iter().map(String::as_str).filter(|p| a_picture(p)).collect();
    let mut seen = HashSet::new();
    let (mut names, mut times, mut sources, mut refs, mut data) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (name, t, source, picture, d) in rows.iter().filter_map(library_parts) {
        if !pictures.contains(&picture.as_str()) || !seen.insert((name.clone(), t, picture.clone())) {
            continue;
        }
        names.push(name);
        times.push(t);
        sources.push(source);
        refs.push(picture);
        data.push(d);
    }
    if pictures.is_empty() {
        return Ok(0);
    }
    client
        .execute(
            "WITH new AS (
                 SELECT r.name, 'epoch'::timestamptz + r.t * interval '1 millisecond' AS at, r.source, r.ref,
                        r.data::jsonb AS data
                 FROM unnest($2::text[], $3::bigint[], $4::text[], $5::text[], $6::text[])
                      AS r(name, t, source, ref, data)
             ), gone AS (
                 DELETE FROM signal_log s
                 WHERE s.device = $1::text::uuid AND s.ref = ANY($7::text[])
                   AND NOT EXISTS (SELECT 1 FROM new n WHERE n.ref = s.ref AND n.name = s.name AND n.at = s.at)
             )
             INSERT INTO signal_log (device, name, at, source, ref, seconds, readings, data)
             SELECT $1::text::uuid, name, at, source, ref, 0, 1, data FROM new
             ON CONFLICT (device, name, at, ref) DO UPDATE SET source = excluded.source, data = excluded.data
             WHERE signal_log.data IS DISTINCT FROM excluded.data OR signal_log.source <> excluded.source",
            &[&LIBRARY, &names, &times, &sources, &refs, &data, &pictures],
        )
        .await
        .map_err(|e| format!("The library's part of the signal history couldn't be written: {}", why(&e)))
}

/// What the history has of the library's faces already: each face's looks by its permanent id, so that a
/// picture is not read for them twice. A row is read without trusting its shape.
pub async fn library_looks(client: &Client) -> Result<HashMap<String, Value>> {
    let said = |e: tokio_postgres::Error| format!("The signal history couldn't be read: {}", why(&e));
    let rows = client
        .query(
            "SELECT data->>'face', (data->'looks')::text FROM signal_log
             WHERE device = $1::text::uuid AND ref <> '' AND data ? 'face' AND data ? 'looks'",
            &[&LIBRARY],
        )
        .await
        .map_err(said)?;
    let mut out = HashMap::new();
    for row in rows {
        let (Ok(Some(face)), Ok(Some(looks))) =
            (row.try_get::<_, Option<String>>(0), row.try_get::<_, Option<String>>(1))
        else {
            continue;
        };
        if let Ok(looks) = serde_json::from_str::<Value>(&looks)
            && looks.is_object()
            && face.len() <= 200
        {
            out.insert(face, looks);
        }
    }
    Ok(out)
}

/// How many rows the library has in the history, and of how many pictures.
pub async fn count_library(client: &Client) -> Result<(i64, i64)> {
    let said = |e: tokio_postgres::Error| format!("The signal history couldn't be read: {}", why(&e));
    let row = client
        .query_one(
            "SELECT count(*), count(DISTINCT ref) FROM signal_log WHERE device = $1::text::uuid AND ref <> ''",
            &[&LIBRARY],
        )
        .await
        .map_err(said)?;
    Ok((row.try_get(0).map_err(said)?, row.try_get(1).map_err(said)?))
}

/// Delete this device's camera rows that are older than `days` (a library's rows are as old as its pictures:
/// they are not a device's, and stay). How many went.
pub async fn prune(client: &Client, device: &str, days: i32) -> Result<u64> {
    an_id(device)?;
    client
        .execute(
            "DELETE FROM signal_log WHERE device = $1::text::uuid AND source = 'camera' AND at < now() - make_interval(days => $2)",
            &[&device, &days.max(1)],
        )
        .await
        .map_err(|e| format!("The signal history's old rows couldn't be deleted: {}", why(&e)))
}

/// Delete every row of a device's history (`LIBRARY`: the rows worked out from the library's pictures). How
/// many went.
pub async fn forget(client: &Client, device: &str) -> Result<u64> {
    an_id(device)?;
    client
        .execute("DELETE FROM signal_log WHERE device = $1::text::uuid", &[&device])
        .await
        .map_err(|e| format!("The signal history couldn't be deleted: {}", why(&e)))
}

/// The same from Settings, on a connection made for it (the one a program keeps for a page's short
/// questions only reads).
pub fn forget_device(home: &std::path::Path, me: &crate::Device, device: &str) -> Result<u64> {
    an_id(device)?;
    let settings = crate::Settings::load(home)?.ok_or("This device has no shared database.")?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = crate::connect(&settings, &settings.schema, me).await?;
        forget(&client, device).await
    })
}

/// Delete this device's rows of the expression log and the music log that are older than `days`: the same days
/// as its signal history (the owner, 2026-11-25: "Expression and music logs should follow the same days
/// settings"). How many went of each.
pub async fn prune_logs(client: &Client, device: &str, days: i32) -> Result<(u64, u64)> {
    an_id(device)?;
    let mut gone = [0u64; 2];
    for (k, table) in ["expression_log", "music_log"].iter().enumerate() {
        gone[k] = client
            .execute(
                &format!(
                    "DELETE FROM {table} WHERE device = $1::text::uuid AND at < now() - make_interval(days => $2)"
                ),
                &[&device, &days.max(1)],
            )
            .await
            .map_err(|e| {
                format!("The {} log's old rows couldn't be deleted: {}", &table[..table.len() - 4], why(&e))
            })?;
    }
    Ok((gone[0], gone[1]))
}

/// How many rows a device has in the history, and how many of them are a named person's.
pub async fn count(client: &Client, device: &str) -> Result<(i64, i64)> {
    an_id(device)?;
    let said = |e: tokio_postgres::Error| format!("The signal history couldn't be read: {}", why(&e));
    let row = client
        .query_one(
            "SELECT count(*), count(*) FILTER (WHERE name <> '') FROM signal_log WHERE device = $1::text::uuid",
            &[&device],
        )
        .await
        .map_err(said)?;
    Ok((row.try_get(0).map_err(said)?, row.try_get(1).map_err(said)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_waiting_row_is_a_row_or_nothing() {
        let good =
            json!({"t": 1_795_161_900_000i64, "name": "Ann", "s": 10, "n": 42, "data": {"pulse": [70, 60, 80, 3]}});
        let (name, t, seconds, readings, data) = parts(&good).unwrap();
        assert_eq!((name.as_str(), t, seconds, readings), ("Ann", 1_795_161_900_000, 10.0, 42));
        assert_eq!(data, "{\"pulse\":[70,60,80,3]}");
        // the device's own row has no name
        assert!(parts(&json!({"t": 5, "name": "", "s": 10, "n": 1, "data": {}})).is_some());
        for (key, value) in [
            ("t", json!(0)),
            ("t", json!("now")),
            ("name", json!(7)),
            ("name", json!("An\u{0}n")),
            ("s", json!(0)),
            ("s", json!(-3)),
            ("n", json!(0)),
            ("n", json!(1.5)),
            ("n", json!(5_000_000_000i64)),
            ("data", json!([1])),
            ("data", json!({"word": "a\u{0}b"})),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(parts(&bad).is_none(), "{key}: {bad}");
        }
        assert!(parts(&json!("a row")).is_none());
    }

    #[test]
    fn a_row_of_the_library_names_a_person_a_picture_and_where_it_is_from() {
        let good = json!({"t": 1_500_000_000_000i64, "name": "Ann", "source": "photo", "ref": "ab12cd34",
                          "data": {"face": "ab12cd34-10-20", "quality": 0.7}});
        let (name, t, source, picture, _) = library_parts(&good).unwrap();
        assert_eq!(
            (name.as_str(), t, source.as_str(), picture.as_str()),
            ("Ann", 1_500_000_000_000, "photo", "ab12cd34")
        );
        // some of the library's keys have a hyphen
        let mut hyphen = good.clone();
        hyphen["ref"] = json!("ab12-cd34");
        assert!(library_parts(&hyphen).is_some());
        // a picture from before 1970 has a time too
        let mut old = good.clone();
        old["t"] = json!(-86_400_000i64);
        assert!(library_parts(&old).is_some());
        for (key, value) in [
            ("name", json!("")),
            ("name", json!(7)),
            ("source", json!("camera")),
            ("source", json!("drawing")),
            ("ref", json!("")),
            ("ref", json!("../photo.jpg")),
            ("ref", json!("a b")),
            ("t", json!("then")),
            ("data", json!("words")),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(library_parts(&bad).is_none(), "{key}: {bad}");
        }
    }
}
