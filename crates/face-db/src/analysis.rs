//! What Analyze reads (the owner, 2026-11-25: "The Analyze tab should work like a regular LLM based UI - you
//! should be able to type in or say (voice support) anything related to the historical or current data related
//! to ANY sensor on an device, and and faces we are recording", "I want analyze to include all signals -
//! everything", and of how: "Would it be good to have a pattern ... that gives the LLM the schema of the database
//! and lets it run its own queries? I dont want to hard code a bunch of queries inside the app for specific
//! scenarios - unless they are just for testing").
//!
//! So the model writes its own SQL, against a layer of tables this module defines over the shared database
//! (`layer`, documented for the model by `LAYER_DOC`): the history's rows keep every signal of a stretch packed
//! in one JSON value, which a model rarely reads right, so the layer gives them flat, a row per signal, word,
//! look, place, song, alert, photo and face, with the devices by the names the owner gave them. A question is run
//! (`query`) in a transaction of its own that can only read, with a time limit, in the owner's time zone, with
//! only the layer's tables in view (the schema's own are not on the search path), and its rows are taken as JSON,
//! a number at most. What the rows hold is data: the model's SQL never becomes part of another statement.

use serde_json::{Value, json};
use tokio_postgres::Client;

use crate::{DEVICE_NAME_D, Result};

/// how long one statement may run on the server, in milliseconds
pub const STATEMENT_MS: u32 = 15_000;
/// rows of one question at the most
pub const MOST_ROWS: usize = 5000;

fn why(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map_or_else(|| e.to_string(), |d| d.message().to_string())
}

/// The statement time limit for this connection (asked once, after connecting).
pub async fn begin(client: &Client) -> Result<()> {
    client.batch_execute(&format!("SET statement_timeout = {STATEMENT_MS}")).await.map_err(|e| why(&e))
}

/// The layer's tables as one `WITH` list over the schema `s` (a name `schema_name` allows). Each is inlined into
/// the question that names it (`NOT MATERIALIZED`): a filter on a person or a time reaches the history's own
/// indexes, and a table nobody names costs nothing.
pub fn layer(s: &str) -> String {
    let dev = DEVICE_NAME_D;
    let device = format!(
        "coalesce({dev}, CASE WHEN r.device = '00000000-0000-0000-0000-000000000000' THEN 'photo library' \
         ELSE 'a device no longer listed' END)"
    );
    // a stretch's number: [average, lowest, highest, how many] (a library row: the number alone)
    let num = |v: &str, i: usize| {
        format!("CASE jsonb_typeof({v}) WHEN 'array' THEN ({v}->>{i})::float8 ELSE ({v} #>> '{{}}')::float8 END")
    };
    let is_num = |v: &str| {
        format!(
            "(jsonb_typeof({v}) = 'number' OR (jsonb_typeof({v}) = 'array' AND jsonb_array_length({v}) = 4 \
             AND jsonb_typeof({v}->0) = 'number'))"
        )
    };
    let object = |v: &str| format!("CASE jsonb_typeof({v}) WHEN 'object' THEN {v} ELSE '{{}}'::jsonb END");
    let array = |v: &str| format!("CASE jsonb_typeof({v}) WHEN 'array' THEN {v} ELSE '[]'::jsonb END");
    let base = format!("FROM \"{s}\".signal_log r LEFT JOIN \"{s}\".devices d ON d.id = r.device");
    let cols = format!(
        "{device} AS device, r.name AS person, r.at, r.seconds::float8 AS seconds, r.source, nullif(r.ref, '') AS photo"
    );
    let n = |v: &str| format!("CASE jsonb_typeof({v}) WHEN 'array' THEN coalesce(({v}->>3)::float8, 1) ELSE 1 END");
    format!(
        "devices AS NOT MATERIALIZED (\
           SELECT d.id::text AS id, {dev} AS name, d.os, d.app_version AS version, d.first_seen, d.last_seen \
           FROM \"{s}\".devices d), \
         people AS NOT MATERIALIZED (\
           SELECT l.n AS name, count(*)::int8 AS faces FROM \"{s}\".labels l \
           WHERE l.n IS NOT NULL AND l.n <> '' AND left(l.n, 1) <> '#' GROUP BY l.n), \
         stretches AS NOT MATERIALIZED (\
           SELECT {cols}, r.readings, \
                  CASE WHEN r.source <> 'camera' THEN coalesce(r.data->>'time', 'camera') END AS time_from {base}), \
         readings AS NOT MATERIALIZED (\
           SELECT {cols}, NULL::text AS interface, e.key AS signal, {v0} AS value, {v1} AS low, {v2} AS high, {vn} AS n \
             {base} CROSS JOIN LATERAL jsonb_each(r.data) e WHERE {isn} \
           UNION ALL SELECT {cols}, NULL, 'looks.' || l.key, (l.value #>> '{{}}')::float8, (l.value #>> '{{}}')::float8, \
             (l.value #>> '{{}}')::float8, r.readings::float8 \
             {base} CROSS JOIN LATERAL jsonb_each({looks}) l WHERE jsonb_typeof(l.value) = 'number' \
           UNION ALL SELECT {cols}, NULL, 'z.' || z.key, (z.value #>> '{{}}')::float8, (z.value #>> '{{}}')::float8, \
             (z.value #>> '{{}}')::float8, r.readings::float8 \
             {base} CROSS JOIN LATERAL jsonb_each({zs}) z WHERE jsonb_typeof(z.value) = 'number' \
           UNION ALL SELECT {cols}, x.value->>'name', 'sound.' || e.key, {s0}, {s1}, {s2}, {sn} \
             {base} CROSS JOIN LATERAL jsonb_array_elements({sound}) x CROSS JOIN LATERAL jsonb_each({xo}) e WHERE {sisn}), \
         words AS NOT MATERIALIZED (\
           SELECT {cols}, NULL::text AS interface, e.key AS signal, w.key AS word, (w.value #>> '{{}}')::float8 AS count \
             {base} CROSS JOIN LATERAL jsonb_each(r.data) e CROSS JOIN LATERAL jsonb_each({eo}) w \
             WHERE e.key NOT IN ('looks', 'z') AND jsonb_typeof(w.value) = 'number' \
           UNION ALL SELECT {cols}, x.value->>'name', 'sound.' || e.key, w.key, (w.value #>> '{{}}')::float8 \
             {base} CROSS JOIN LATERAL jsonb_array_elements({sound}) x CROSS JOIN LATERAL jsonb_each({xo}) e \
             CROSS JOIN LATERAL jsonb_each({eo}) w WHERE jsonb_typeof(w.value) = 'number'), \
         expressions AS NOT MATERIALIZED (\
           SELECT {dev} AS device, r.name AS person, r.at, r.label AS look, r.share::float8 AS share, r.state, \
                  (r.data->>'valence')::float8 AS valence, (r.data->>'arousal')::float8 AS arousal \
           FROM \"{s}\".expression_log r LEFT JOIN \"{s}\".devices d ON d.id = r.device), \
         music AS NOT MATERIALIZED (\
           SELECT {dev} AS device, r.name AS person, r.at, r.artist, r.song, r.seconds::float8 AS seconds \
           FROM \"{s}\".music_log r LEFT JOIN \"{s}\".devices d ON d.id = r.device), \
         places AS NOT MATERIALIZED (\
           SELECT {dev} AS device, r.at, r.lat, r.lon, r.accuracy::float8 AS accuracy_m, r.source \
           FROM \"{s}\".location_log r LEFT JOIN \"{s}\".devices d ON d.id = r.device), \
         alerts AS NOT MATERIALIZED (\
           SELECT {dev} AS device, to_timestamp((r.data->>'t')::float8 / 1000) AS at, r.data->>'kind' AS kind, \
                  r.data->>'name' AS person, r.data->>'text' AS text, r.data->>'state' AS state \
           FROM \"{s}\".alerts r LEFT JOIN \"{s}\".devices d ON d.id = r.device WHERE jsonb_typeof(r.data->'t') = 'number'), \
         visits AS NOT MATERIALIZED (\
           SELECT {dev} AS device, r.name AS person, to_timestamp(r.start_ms / 1000.0) AS from_at, \
                  to_timestamp(r.end_ms / 1000.0) AS to_at \
           FROM \"{s}\".presence r LEFT JOIN \"{s}\".devices d ON d.id = r.device), \
         sightings AS NOT MATERIALIZED (\
           SELECT {dev} AS device, r.name AS person, r.first_seen, r.last_seen \
           FROM \"{s}\".sightings r LEFT JOIN \"{s}\".devices d ON d.id = r.device), \
         photos AS NOT MATERIALIZED (\
           SELECT p.hash AS photo, p.name AS file, to_timestamp(p.taken) AS taken, p.video, p.faces, \
                  dsc.data->>'text' AS description \
           FROM \"{s}\".photos p LEFT JOIN \"{s}\".descriptions dsc ON dsc.key = p.hash WHERE p.dup_of IS NULL), \
         photo_people AS NOT MATERIALIZED (\
           SELECT f.photo, l.n AS person, f.frame, f.quality::float8 AS face_quality \
           FROM \"{s}\".faces f JOIN \"{s}\".labels l ON l.fid = f.fid \
           WHERE l.n IS NOT NULL AND l.n <> '' AND left(l.n, 1) <> '#'), \
         usual AS NOT MATERIALIZED (\
           SELECT r.name AS person, r.kind, r.data, r.updated_at FROM \"{s}\".person_signals r)",
        v0 = num("e.value", 0),
        v1 = num("e.value", 1),
        v2 = num("e.value", 2),
        vn = n("e.value"),
        isn = is_num("e.value"),
        looks = object("r.data->'looks'"),
        zs = object("r.data->'z'"),
        sound = array("r.data->'sound'"),
        xo = object("x.value"),
        s0 = num("e.value", 0),
        s1 = num("e.value", 1),
        s2 = num("e.value", 2),
        sn = n("e.value"),
        sisn = is_num("e.value"),
        eo = object("e.value"),
    )
}

/// The layer for the model: its tables, their columns and what they mean.
pub const LAYER_DOC: &str = "\
Tables (PostgreSQL; times are timestamptz and the session is in the owner's time zone, so now(), date_trunc('day', at) \
and to_char(at, ...) are local; a person's name is as the owner named them; a device by its name):
- readings(device, person, at, seconds, source, photo, interface, signal, value, low, high, n): every number read. One \
row per signal per stretch of about 10 seconds: of a device itself (person = '') or of a named person in its camera's \
view. value: the stretch's average; low, high: its lowest and highest; n: how many readings it is (average over \
stretches as sum(value*n)/sum(n)). source 'camera', or 'photo' / 'video' for rows the library's pictures gave (then \
photo is the picture and at its time; device 'photo library'). Sound signals ('sound.level'...) are of one audio \
interface each (interface: its name). Signals 'looks.<look>' (a look's average share), 'z.<signal>' (how far from the \
person's own usual, in spreads: beyond 2 or -2 is unusual) and 'odd' (how many signals were unusual at once).
- words(device, person, at, source, photo, interface, signal, word, count): how often each word was read in a stretch \
(signal 'look': the look that won; 'state'; 'heat'; 'sound.what': what an interface heard).
- stretches(device, person, at, seconds, source, photo, readings, time_from): the stretches themselves: who was in \
which camera's view when. Two people together: the same device and the same at (or the same photo). For a picture's \
row, time_from says where its time came from: 'camera' (the picture's own date) or 'file' (it has none: the file's \
date, often when it was copied or downloaded, not when it was taken).
- expressions(device, person, at, look, share, state, valence, arousal): each expression reading (every few seconds).
- music(device, person, at, artist, song, seconds): songs heard.
- places(device, at, lat, lon, accuracy_m, source): where a device was (a row when it moved).
- alerts(device, at, kind, person, text, state): alerts (kinds: arrived, left, moved, unknown, odd, joined...).
- visits(device, person, from_at, to_at): the presence log's visits.
- sightings(device, person, first_seen, last_seen): which camera saw whom, first and last.
- photos(photo, file, taken, video, faces, description): the library's photos and videos (taken: the picture's time, \
may be null; description: what Describe photo wrote). How a person looks in a picture is in readings, not in the \
description: rows with source 'photo' or 'video', that photo and that person (signals looks.<look>, quality, px).
- photo_people(photo, person, frame, face_quality): who is named in each picture (a video's frame).
- people(name, faces): every named person and how many faces of them the library has.
- devices(id, name, os, version, first_seen, last_seen).
- usual(person, kind, data, updated_at): each person's usual levels as the devices learned them (JSON).";

/// Rows of one question: each row an object of its columns, at most `most` of them; and whether there were more.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub rows: Vec<Value>,
    pub more: bool,
}

/// A question the model wrote, as one statement over the layer (the schema `s`'s own tables out of view), in a
/// transaction that can only read, with the time limit, in the time zone `tz` (an IANA name; UTC when it isn't
/// one). Its rows as JSON objects.
pub async fn query(client: &Client, s: &str, tz: &str, sql: &str, most: usize) -> Result<Found> {
    if !crate::schema_name(s) {
        return Err("bad schema name".to_string());
    }
    let asked = sql.trim().trim_end_matches(';').trim();
    if asked.is_empty() {
        return Err("no question".to_string());
    }
    let zone =
        if !tz.is_empty() && tz.len() <= 64 && tz.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b)) {
            tz
        } else {
            "UTC"
        };
    let most = most.clamp(1, MOST_ROWS);
    // (one statement: the extended protocol takes no more than one, so nothing can follow it)
    let full = format!("WITH {} SELECT row_to_json(q)::text FROM ({asked}\n) q LIMIT {}", layer(s), most + 1);
    client.batch_execute("BEGIN READ ONLY").await.map_err(|e| why(&e))?;
    let set = client
        .batch_execute(&format!(
            "SET LOCAL statement_timeout = {STATEMENT_MS}; SET LOCAL search_path = pg_temp; SET LOCAL TIME ZONE '{zone}'"
        ))
        .await;
    let ran = match set {
        Ok(()) => client.query(&full, &[]).await.map_err(|e| why(&e)),
        Err(e) => Err(why(&e)),
    };
    let _ = client.batch_execute("ROLLBACK").await;
    let got = ran?;
    let mut rows: Vec<Value> = got
        .iter()
        .map(|r| r.get::<_, Option<String>>(0).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null))
        .collect();
    let more = rows.len() > most;
    rows.truncate(most);
    Ok(Found { rows, more })
}

/// The devices of the shared database: [{id, name, os, version, first, last}] (times in ms).
pub async fn devices(client: &Client) -> Result<Vec<Value>> {
    let sql = format!(
        "SELECT d.id::text, {DEVICE_NAME_D}, d.os, d.app_version, (extract(epoch FROM d.first_seen) * 1000)::int8, \
                (extract(epoch FROM d.last_seen) * 1000)::int8 FROM devices d ORDER BY 2"
    );
    Ok(client
        .query(&sql, &[])
        .await
        .map_err(|e| why(&e))?
        .iter()
        .map(|r| {
            let (id, name, os, version, first, last): (String, String, String, String, i64, i64) =
                (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5));
            json!({"id": id, "name": name, "os": os, "version": version, "first": first, "last": last})
        })
        .collect())
}

/// Every named person: [{name, faces (named faces in the library), last (newest history row of a camera, ms)}].
/// An Ignored face's label ("#...") is no person.
pub async fn people(client: &Client) -> Result<Vec<Value>> {
    let sql = "WITH named AS (SELECT n AS name, count(*)::int8 AS faces FROM labels \
                              WHERE n IS NOT NULL AND n <> '' AND left(n, 1) <> '#' GROUP BY n), \
                    seen AS (SELECT name, (extract(epoch FROM max(at)) * 1000)::int8 AS last \
                             FROM signal_log WHERE name <> '' AND source = 'camera' GROUP BY name) \
               SELECT coalesce(named.name, seen.name), coalesce(named.faces, 0), seen.last \
               FROM named FULL JOIN seen ON seen.name = named.name ORDER BY 1";
    Ok(client
        .query(sql, &[])
        .await
        .map_err(|e| why(&e))?
        .iter()
        .map(|r| {
            let (name, faces, last): (String, i64, Option<i64>) = (r.get(0), r.get(1), r.get(2));
            json!({"name": name, "faces": faces, "last": last})
        })
        .collect())
}

/// One picture of the library in full, for the page that shows it: {photo, name, taken, video, text (its
/// description), faces: [{fid, name, frame, quality}], rows: [{name, t, source, data}] (what the history worked out
/// of it for each named person)}. None when the library has no such picture.
pub async fn picture(client: &Client, hash: &str) -> Result<Option<Value>> {
    let rows = |sql: &'static str| async move { client.query(sql, &[&hash]).await.map_err(|e| why(&e)) };
    let sql = "SELECT p.name, (p.taken * 1000)::int8, p.video, dsc.data->>'text' FROM photos p \
               LEFT JOIN descriptions dsc ON dsc.key = p.hash WHERE p.hash = $1";
    let Some(r) = rows(sql).await?.into_iter().next() else { return Ok(None) };
    let (name, taken, video, text): (String, Option<i64>, bool, Option<String>) =
        (r.get(0), r.get(1), r.get(2), r.get(3));
    let sql = "SELECT f.fid, coalesce(l.n, ''), f.frame, f.quality::float8 FROM faces f LEFT JOIN labels l ON l.fid = f.fid \
               WHERE f.photo = $1 AND f.possible IS NULL ORDER BY f.frame NULLS FIRST, f.fid LIMIT 200";
    let faces: Vec<Value> = rows(sql)
        .await?
        .iter()
        .map(|r| {
            let (fid, n, frame, quality): (String, String, Option<i32>, Option<f64>) =
                (r.get(0), r.get(1), r.get(2), r.get(3));
            // (an Ignored face is no one)
            let named = if n.starts_with('#') { String::new() } else { n };
            json!({"fid": fid, "name": named, "frame": frame, "quality": quality})
        })
        .collect();
    let sql = "SELECT name, (extract(epoch FROM at) * 1000)::int8, source, data::text FROM signal_log \
               WHERE ref = $1 ORDER BY at, name LIMIT 200";
    let found: Vec<Value> = rows(sql)
        .await?
        .iter()
        .map(|r| {
            let (name, t, source, data): (String, i64, String, Option<String>) = (r.get(0), r.get(1), r.get(2), r.get(3));
            json!({"name": name, "t": t, "source": source, "data": data.and_then(|d| serde_json::from_str::<Value>(&d).ok())})
        })
        .collect();
    Ok(Some(
        json!({"photo": hash, "name": name, "taken": taken, "video": video, "text": text, "faces": faces, "rows": found}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layer_names_only_its_schema_s_tables_and_every_table_it_documents() {
        let l = layer("facerec");
        for table in [
            "signal_log",
            "devices",
            "labels",
            "expression_log",
            "music_log",
            "location_log",
            "alerts",
            "presence",
            "sightings",
            "photos",
            "descriptions",
            "faces",
            "person_signals",
        ] {
            assert!(l.contains(&format!("\"facerec\".{table}")), "{table}");
        }
        // (no other table of the schema: keys, logins and the change log are not in it)
        for not in ["secrets", "key_requests", "passphrase", "changes", "app_log"] {
            assert!(!l.contains(not), "{not}");
        }
        for table in [
            "readings",
            "words",
            "stretches",
            "expressions",
            "music",
            "places",
            "alerts",
            "visits",
            "sightings",
            "photos",
            "photo_people",
            "people",
            "devices",
            "usual",
        ] {
            assert!(l.contains(&format!("{table} AS NOT MATERIALIZED")), "{table}");
            assert!(LAYER_DOC.contains(&format!("- {table}(")), "{table} is documented");
        }
    }
}
