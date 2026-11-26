//! What Analyze reads (`analysis.rs`): the layer of tables a model writes its SQL against, over every kind of row
//! of the shared database, against a real server, in a schema of this test's own that is dropped afterwards.
//! `FACE_TEST_DB_HOME` names a private folder with a `database.json` whose role may create schemas; without it the
//! test says so and passes. Nothing here touches the schema the devices share. The questions are written as a model
//! writes them; the numbers they must give are worked out by hand from the rows put in.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use face_db::{Device, Settings, analysis as a, signals};
use futures_util::FutureExt;
use serde_json::{Value, json};

/// A question as a model writes it, in the owner's time zone.
async fn q(client: &tokio_postgres::Client, schema: &str, sql: &str) -> Result<a::Found, String> {
    a::query(client, schema, "America/Chicago", sql, 100).await
}

fn settings() -> Option<Settings> {
    let home = PathBuf::from(std::env::var_os("FACE_TEST_DB_HOME")?);
    Settings::load(&home).ok().flatten()
}

#[test]
fn analyze_s_layer_gives_every_kind_of_row_flat_and_reads_only() {
    let Some(s) = settings() else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let schema = format!("t_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let home = std::env::temp_dir().join(format!("face-db-analysis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let device = Device::load(&home, "a test device").unwrap();
        let other = Device::load(&home.join("other"), "another").unwrap();
        let mut client = face_db::connect(&s, &schema, &device).await.expect("the server");
        client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();
        let outcome = AssertUnwindSafe(async {
            face_db::migrate(&mut client).await?;
            a::begin(&client).await?;
            face_db::register(&client, &device, "Test OS", "2.7.0").await?;
            face_db::register(&client, &other, "Test OS", "2.7.0").await?;
            face_db::set_alias(&client, &other.id, "Kitchen").await?;
            let e = |e: tokio_postgres::Error| e.to_string();
            let now: i64 =
                client.query_one("SELECT (extract(epoch FROM now()) * 1000)::bigint", &[]).await.map_err(e)?.get(0);
            let start = (now - 3_600_000) / 60_000 * 60_000; // a whole minute an hour ago
            // 30 stretches of 10 s: the device's own row and Ann's and Zoë's; Ann's pulse rises by one a stretch,
            // with a jump of 20 at the 20th
            let mut rows = Vec::new();
            for k in 0..30i64 {
                let t = start + k * 10_000;
                let pulse = 60.0 + k as f64 + if k >= 20 { 20.0 } else { 0.0 };
                rows.push(json!({"t": t, "name": "", "s": 10, "n": 100, "data": {
                    "load": [40.0, 30.0, 50.0, 100], "heat": {"nominal": 3, "fair": 1},
                    "sound": [{"name": "Speakers", "kind": "output", "level": [-20.0, -30.0, -10.0, 2], "what": {"music": 2}},
                              {"name": "Microphone", "kind": "input", "level": [-50.0, -55.0, -45.0, 2], "what": {"quiet": 2}}]}}));
                rows.push(json!({"t": t, "name": "Ann", "s": 10, "n": 100, "data": {
                    "pulse": [pulse, pulse - 2.0, pulse + 2.0, 100], "smile": [0.5, 0.1, 0.9, 100],
                    "look": {"happy": 2, "neutral": 1}, "looks": {"happy": 0.6, "neutral": 0.4},
                    "z": {"pulse": if k >= 20 { 2.5 } else { 0.5 }}, "odd": if k >= 20 { 3 } else { 0 }}}));
                rows.push(json!({"t": t, "name": "Zoë", "s": 10, "n": 50, "data": {"pulse": [75.0, 70.0, 80.0, 50]}}));
            }
            signals::send(&client, &device.id, &rows).await?;
            // the other device saw Ann later, once
            let later = json!({"t": start + 600_000, "name": "Ann", "s": 10, "n": 10, "data": {"pulse": [90.0, 90.0, 90.0, 10]}});
            signals::send(&client, &other.id, &[later]).await?;
            let one = |found: &a::Found| found.rows.first().cloned().unwrap_or(Value::Null);

            // a series by the minute, the average weighted by readings
            let by_minute = q(&client, &schema,
                "SELECT date_trunc('minute', at) AS at, sum(value * n) / sum(n) AS value, min(low) AS low, max(high) AS high, \
                 sum(n) AS n FROM readings WHERE person = 'Ann' AND signal = 'pulse' AND device = 'a test device' \
                 GROUP BY 1 ORDER BY 1",
            )
            .await?;
            assert_eq!(by_minute.rows.len(), 5, "{by_minute:?}");
            let f = |v: &Value| v.as_f64().unwrap();
            let r0 = &by_minute.rows[0];
            assert_eq!((f(&r0["value"]), f(&r0["low"]), f(&r0["high"])), (62.5, 58.0, 67.0));
            // the time zone is the owner's: times come in local time
            assert!(by_minute.rows[0]["at"].as_str().unwrap().ends_with("-06:00"), "{:?}", by_minute.rows[0]["at"]);
            // every kind of signal a row keeps, flat
            let value = |found: &a::Found| one(found)["v"].as_f64().unwrap();
            let avg = |signal: &str, person: &str| {
                format!("SELECT sum(value * n) / sum(n) AS v FROM readings WHERE signal = '{signal}' AND person = '{person}'")
            };
            assert!((value(&q(&client, &schema, &avg("looks.happy", "Ann")).await?) - 0.6).abs() < 1e-9);
            assert!((value(&q(&client, &schema, &avg("z.pulse", "Ann")).await?) - (20.0 * 0.5 + 10.0 * 2.5) / 30.0).abs() < 1e-9);
            assert_eq!(value(&q(&client, &schema, "SELECT max(value) AS v FROM readings WHERE signal = 'odd'").await?), 3.0);
            assert_eq!(value(&q(&client, &schema, &avg("load", "")).await?), 40.0);
            assert_eq!(
                value(&q(&client, &schema, "SELECT avg(value) AS v FROM readings WHERE signal = 'sound.level' AND interface = 'Speakers'").await?),
                -20.0
            );
            // words: how often each was read
            let happy = q(&client, &schema,
                "SELECT sum(count) FILTER (WHERE word = 'happy') / sum(count) AS v FROM words WHERE person = 'Ann' AND signal = 'look'",
            )
            .await?;
            assert!((value(&happy) - 2.0 / 3.0).abs() < 1e-9);
            assert_eq!(
                value(&q(&client, &schema, "SELECT sum(count) AS v FROM words WHERE signal = 'sound.what' AND word = 'music'").await?),
                60.0
            );
            // a device by the name the owner gave it
            assert_eq!(
                one(&q(&client, &schema, "SELECT device, max(at) AS last FROM stretches WHERE person = 'Ann' GROUP BY device ORDER BY 2 DESC").await?)["device"],
                json!("Kitchen")
            );
            // two people together: the same device and stretch
            let together = q(&client, &schema,
                "SELECT count(*) AS v FROM stretches a JOIN stretches b ON b.device = a.device AND b.at = a.at AND b.person = 'Zoë' \
                 WHERE a.person = 'Ann'",
            )
            .await?;
            assert_eq!(one(&together)["v"], json!(30));
            // unusual: beyond two spreads of its own level, by a window over the rows
            let unusual = q(&client, &schema,
                "SELECT * FROM (SELECT at, value, (value - avg(value) OVER ()) / stddev_samp(value) OVER () AS z FROM readings \
                 WHERE person = 'Ann' AND signal = 'pulse' AND device = 'a test device') x ORDER BY abs(z) DESC LIMIT 3",
            )
            .await?;
            assert!(unusual.rows[0]["z"].as_f64().unwrap().abs() > 1.5, "{unusual:?}");

            // the people, and every log
            client
                .batch_execute(
                    "INSERT INTO labels (fid, n) VALUES ('f1', 'Ann'), ('f2', 'Ann'), ('f3', '#ignore'), ('f4', 'Bea'); \
                     INSERT INTO photos (hash, name, taken) VALUES ('p1', 'beach.jpg', 1700000000), ('p2', 'home.jpg', 1710000000); \
                     INSERT INTO faces (fid, photo, box, model) VALUES ('f1', 'p1', '{0,0,1,1,1}', 'm'), ('f2', 'p2', '{0,0,1,1,1}', 'm'), \
                       ('f4', 'p1', '{0,0,1,1,1}', 'm'); \
                     INSERT INTO descriptions (key, data) VALUES ('p1', '{\"text\": \"Two people on a beach.\"}');",
                )
                .await
                .map_err(e)?;
            for (sql, args) in [
                (
                    "INSERT INTO expression_log (device, name, at, label, share, data) VALUES ($1::text::uuid, 'Ann', now() - interval '2 hours', 'happy', 0.8, '{\"valence\": 0.6}')",
                    &device.id,
                ),
                ("INSERT INTO music_log (device, name, at, artist, song, seconds) VALUES ($1::text::uuid, '', now(), 'An artist', 'A song', 200)", &device.id),
                ("INSERT INTO location_log (device, at, lat, lon, accuracy, source) VALUES ($1::text::uuid, now(), 40.0, -89.0, 50, 'system')", &device.id),
                ("INSERT INTO sightings (device, name) VALUES ($1::text::uuid, 'Bea')", &device.id),
            ] {
                client.execute(sql, &[args]).await.map_err(e)?;
            }
            client
                .execute(
                    "INSERT INTO presence (id, device, name, start_ms, end_ms) VALUES ('v1', $1::text::uuid, 'Ann', $2, $3)",
                    &[&device.id, &(now - 7_200_000), &(now - 3_600_000)],
                )
                .await
                .map_err(e)?;
            let alert = json!({"id": "a1", "t": now - 60_000, "kind": "arrived", "name": "Ann", "text": "Ann arrived", "state": "new"});
            client
                .execute("INSERT INTO alerts (id, device, data) VALUES ('a1', $1::text::uuid, $2::text::jsonb)", &[&device.id, &alert.to_string()])
                .await
                .map_err(e)?;
            client
                .execute("INSERT INTO person_signals (name, kind, data) VALUES ('Ann', 'expr', '{\"n\": 3}')", &[])
                .await
                .map_err(e)?;
            let names: Vec<Value> = q(&client, &schema, "SELECT name, faces FROM people ORDER BY name").await?.rows;
            assert_eq!(names, vec![json!({"name": "Ann", "faces": 2}), json!({"name": "Bea", "faces": 1})], "an Ignored face is nobody");
            for (sql, want) in [
                ("SELECT look AS v FROM expressions WHERE person = 'Ann'", json!("happy")),
                ("SELECT valence::text AS v FROM expressions WHERE person = 'Ann'", json!("0.6")),
                ("SELECT song AS v FROM music", json!("A song")),
                ("SELECT accuracy_m::int AS v FROM places", json!(50)),
                ("SELECT text AS v FROM alerts WHERE kind = 'arrived'", json!("Ann arrived")),
                ("SELECT person AS v FROM visits", json!("Ann")),
                ("SELECT person AS v FROM sightings", json!("Bea")),
                ("SELECT data->>'n' AS v FROM usual WHERE person = 'Ann'", json!("3")),
                ("SELECT count(*) AS v FROM devices", json!(2)),
            ] {
                assert_eq!(one(&q(&client, &schema, sql).await?)["v"], want, "{sql}");
            }
            // photos of two people together, with their description
            let both = q(&client, &schema,
                "SELECT p.photo, p.description FROM photos p WHERE (SELECT count(DISTINCT person) FROM photo_people pp \
                 WHERE pp.photo = p.photo AND pp.person IN ('Ann', 'Bea')) = 2",
            )
            .await?;
            assert_eq!(both.rows, vec![json!({"photo": "p1", "description": "Two people on a beach."})]);
            let p = a::picture(&client, "p1").await?.expect("the picture");
            let faces: Vec<&str> = p["faces"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
            assert_eq!(faces, ["Ann", "Bea"]);

            // what can't be done: write, read the schema's own tables, run past the limit, or run two statements
            for bad in [
                "INSERT INTO people VALUES ('x', 1)",
                "SELECT * FROM secrets",
                "SELECT * FROM labels",
                "SELECT 1; DELETE FROM labels",
                "WITH gone AS (DELETE FROM labels RETURNING 1) SELECT * FROM gone",
            ] {
                let said = q(&client, &schema, bad).await;
                assert!(said.is_err(), "{bad}: {said:?}");
            }
            // a setting a question changes is its own transaction's, undone with it: the next question still
            // can't write, and this connection's own settings are as they were
            q(&client, &schema, "SELECT set_config('default_transaction_read_only', 'off', false)").await?;
            q(&client, &schema, "SELECT set_config('search_path', 'public', false)").await?;
            assert!(q(&client, &schema, "SELECT * FROM labels").await.is_err(), "the schema's own tables stay out of view");
            let still: String = client.query_one("SHOW default_transaction_read_only", &[]).await.map_err(e)?.get(0);
            assert_eq!(still, "off", "this test's own connection is as it was");
            let named: i64 = client.query_one("SELECT count(*) FROM labels", &[]).await.map_err(e)?.get(0);
            assert_eq!(named, 4, "nothing was deleted");
            let many = a::query(&client, &schema, "UTC", "SELECT generate_series(1, 50) AS v", 10).await?;
            assert_eq!((many.rows.len(), many.more), (10, true));
            let slow = a::query(&client, &schema, "UTC", "SELECT pg_sleep(20)", 10).await;
            assert!(slow.is_err(), "past the time limit");
            Ok::<(), String>(())
        })
        .catch_unwind()
        .await;
        let dropped = client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await;
        let _ = std::fs::remove_dir_all(&home);
        dropped.unwrap();
        match outcome {
            Ok(told) => told.unwrap(),
            Err(failed) => std::panic::resume_unwind(failed),
        }
    });
}
