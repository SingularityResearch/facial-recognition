//! The writing side against a real server, in a schema of this test's own that is dropped afterwards.
//! `FACE_TEST_DB_HOME` names a private folder with a `database.json`
//! whose role may create schemas; without it these tests say so and pass. Nothing here touches the schema
//! the devices share: every statement runs with the test's schema first in view, and only that schema is
//! dropped, by the name this test made up.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use face_db::{Device, Settings};
use futures_util::FutureExt;

fn settings() -> Option<Settings> {
    let home = PathBuf::from(std::env::var_os("FACE_TEST_DB_HOME")?);
    Settings::load(&home).ok().flatten()
}

fn made_up_schema() -> String {
    format!("t_{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

#[test]
fn a_new_schema_is_migrated_and_a_device_noted_and_seen() {
    let Some(s) = settings() else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let schema = made_up_schema();
        let home = std::env::temp_dir().join(format!("face-db-server-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let device = Device::load(&home, "a test device").unwrap();
        // the schema is made on a connection that has the shared schema's name nowhere in a statement
        let mut client = face_db::connect(&s, &schema, &device).await.expect("the server");
        client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();
        let outcome = AssertUnwindSafe(async {
            assert_eq!(face_db::version(&client).await?, 0, "an empty schema");
            let applied = face_db::migrate(&mut client).await?;
            assert_eq!(applied, (1..=face_db::MIGRATIONS.len() as i32).collect::<Vec<_>>());
            assert_eq!(face_db::version(&client).await?, face_db::MIGRATIONS.len() as i32);
            assert!(face_db::migrate(&mut client).await?.is_empty(), "nothing is applied twice");
            // the tables are this schema's, not the shared one's
            let at: String = client.query_one("SELECT table_schema::text FROM information_schema.tables WHERE table_name = 'devices' AND table_schema = current_schema()", &[]).await.map_err(|e| e.to_string())?.get(0);
            assert_eq!(at, schema);

            face_db::register(&client, &device, "Test OS 1", "0.0.0-test").await?;
            face_db::register(&client, &device, "Test OS 2", "0.0.0-test").await?;
            let row = client.query_one("SELECT name, os, streaming_at IS NULL, (SELECT count(*) FROM devices) FROM devices WHERE id::text = $1", &[&device.id]).await.map_err(|e| e.to_string())?;
            assert_eq!((row.get::<_, String>(0).as_str(), row.get::<_, String>(1).as_str()), ("a test device", "Test OS 2"));
            assert!(row.get::<_, bool>(2), "a device that never streamed");
            assert_eq!(row.get::<_, i64>(3), 1, "noted twice, it is one device");

            // what the owner calls a device: shown in its place wherever the devices are listed, kept as text a
            // page can show, and the device goes by its own name again when it is taken away
            assert_eq!(face_db::set_alias(&client, &device.id, "  Kitchen\u{7} tablet \n").await?, "Kitchen tablet");
            let listed = face_db::kept::cameras(&client).await?;
            assert_eq!(listed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Kitchen tablet"]);
            let long = face_db::set_alias(&client, &device.id, &"\u{e9}".repeat(90)).await?;
            assert_eq!(long.chars().count(), face_db::join::NAME_MAX);
            assert_eq!(face_db::set_alias(&client, &device.id, " ").await?, "a test device");
            assert_eq!(face_db::kept::cameras(&client).await?[0].name, "a test device");
            // the device itself noting its name again changes nothing of what the owner called it
            face_db::set_alias(&client, &device.id, "Hall").await?;
            face_db::register(&client, &device, "Test OS 2", "0.0.0-test").await?;
            assert_eq!(face_db::kept::cameras(&client).await?[0].name, "Hall");

            face_db::heartbeat(&client, &device, false).await?;
            let quiet: bool = client.query_one("SELECT streaming_at IS NULL FROM devices", &[]).await.map_err(|e| e.to_string())?.get(0);
            assert!(quiet, "here, but its camera doesn't stream");
            face_db::heartbeat(&client, &device, true).await?;
            let fresh: bool = client.query_one("SELECT streaming_at > now() - interval '10 seconds' AND last_seen > now() - interval '10 seconds' FROM devices", &[]).await.map_err(|e| e.to_string())?.get(0);
            assert!(fresh, "its camera streams now");

            // the reading side reads this schema as it reads the shared one: nobody is named in a new one
            let (names, _, _) = face_db::people(&client, "opsis_1").await?;
            assert!(names.is_empty());

            // a name is one device's alone, whatever its capitals: another device takes neither this one's
            // alias nor the name a device without one calls itself
            let other = Device::load(&home.join("other"), "Garage").unwrap();
            face_db::register(&client, &other, "Test OS 1", "0.0.0-test").await?;
            for taken in ["Hall", "  hALL "] {
                let said = face_db::set_alias(&client, &other.id, taken).await;
                assert!(said.unwrap_err().contains("already"), "{taken} was given twice");
            }
            let said = face_db::set_alias(&client, &device.id, "GARAGE").await;
            assert!(said.unwrap_err().contains("already"), "another device's own name was taken");
            assert_eq!(face_db::kept::cameras(&client).await?.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Garage", "Hall"]);
            // a device's own name, written its way, is its to take
            assert_eq!(face_db::set_alias(&client, &other.id, "garage").await?, "garage");
            assert_eq!(face_db::set_alias(&client, &other.id, "Shed").await?, "Shed");
            // the table itself holds it, whoever writes the row
            let forced = client.execute("UPDATE devices SET alias = ' hall' WHERE id = $1::text::uuid", &[&other.id]).await;
            assert!(forced.is_err(), "the index let a second device take the name");
            // a name that was given back is free again
            face_db::set_alias(&client, &device.id, "").await?;
            assert_eq!(face_db::set_alias(&client, &other.id, "Hall").await?, "Hall");

            // A device that calls itself what another goes by already is told apart by a number, for every
            // device: two phones both say "iPhone". The one that was there first keeps the plain name.
            let names = async |client: &tokio_postgres::Client| -> Result<Vec<String>, String> {
                Ok(face_db::kept::cameras(client).await?.into_iter().map(|c| c.name).collect())
            };
            let phone = |n: &str| Device::load(&home.join(n), "iPhone").unwrap();
            let (first, second, third) = (phone("p1"), phone("p2"), phone("p3"));
            face_db::register(&client, &first, "iOS 26", "2.4.0").await?;
            face_db::register(&client, &second, "iOS 27", "2.4.0").await?;
            assert_eq!(names(&client).await?, ["a test device", "Hall", "iPhone", "iPhone 2"]);
            // noted again (every start does): nothing changes, and the first is still the plain one
            face_db::register(&client, &first, "iOS 26", "2.4.0").await?;
            face_db::register(&client, &second, "iOS 27", "2.4.0").await?;
            assert_eq!(names(&client).await?, ["a test device", "Hall", "iPhone", "iPhone 2"]);
            let row = client.query_one("SELECT alias FROM devices WHERE id = $1::text::uuid", &[&first.id]).await;
            assert_eq!(row.map_err(|e| e.to_string())?.get::<_, String>(0), "", "the first has no alias: it is its own name");
            // a third takes the next free number, also past one the owner gave away to something else
            face_db::set_alias(&client, &other.id, "iphone 3").await?;
            face_db::register(&client, &third, "iOS 27", "2.4.0").await?;
            assert_eq!(names(&client).await?, ["a test device", "iPhone", "iPhone 2", "iphone 3", "iPhone 4"]);
            // the owner's name for a device stands, and a number is not given back to one that has a name
            assert_eq!(face_db::set_alias(&client, &second.id, "Kitchen phone").await?, "Kitchen phone");
            face_db::register(&client, &second, "iOS 27", "2.4.0").await?;
            assert_eq!(names(&client).await?, ["a test device", "iPhone", "iphone 3", "iPhone 4", "Kitchen phone"]);
            // a device yields to a name the owner gave another device, even an older one's own
            face_db::set_alias(&client, &other.id, "a test device").await.unwrap_err();
            face_db::set_alias(&client, &third.id, "").await?;
            face_db::set_alias(&client, &second.id, "iPhone 2").await?;
            face_db::register(&client, &third, "iOS 27", "2.4.0").await?;
            assert_eq!(names(&client).await?, ["a test device", "iPhone", "iPhone 2", "iphone 3", "iPhone 4"]);
            assert!(face_db::set_alias(&client, "not-an-id", "x").await.is_err());
            let nobody = face_db::set_alias(&client, &face_db::new_id(), "x").await.unwrap_err();
            assert!(nobody.contains("not a device"), "{nobody}");

            // the member list every device keeps for itself: what the database says of each device, taken as
            // it is said and kept in the device's own folder, with the database's id beside it
            face_db::peer::register(&client, &device, &["192.168.1.20".to_string()], 8767, &"ab".repeat(32)).await?;
            let (instance, members) = face_db::members::read(&client).await?;
            assert_eq!(members.len(), 5);
            let mine = members.iter().find(|m| m.id == device.id).expect("this device is a member");
            assert_eq!((mine.name.as_str(), mine.port, mine.share), ("a test device", 8767, true));
            assert_eq!(mine.addresses, ["192.168.1.20"]);
            assert_eq!(mine.cert_sha256, "ab".repeat(32));
            // a device that never said where it is: a member all the same, reached nowhere
            let quiet = members.iter().find(|m| m.id == third.id).expect("the third device is a member");
            assert_eq!((quiet.port, quiet.cert_sha256.as_str()), (0, ""));
            let key = face_db::members::refresh(&client, &s, &home).await?;
            assert_eq!(face_db::members::kept(&home), members);
            assert_eq!(face_db::members::instance(&home), Some(instance));
            // and the cluster's key made with it is the one the server's own id gives
            assert_eq!(face_db::peer::cluster_key(&client, &s).await?, key);
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

/// Joining, against the real server and reading only: this computer shows a code, the code's reader is
/// handed the login over this computer's own network address, and `setup` logs in with it (channel
/// binding), takes the server's certificate authority from the server and keeps the settings in a folder
/// of this test's own. A connection made from the kept settings then checks the server against it.
#[test]
fn a_device_that_read_the_code_sets_itself_up() {
    use face_db::join::{self, Handover, Kind, Offer, Outcome, Received, Want};
    let Some(s) = settings() else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let never = || std::sync::atomic::AtomicBool::new(false);
    let offer = Offer::open(0, (&s.host, s.port), "a device of the database").expect("an address on the local network");
    let text = offer.code();
    let what = Handover::new(s.login(), Some("a passphrase that goes with it".to_string()), "a device of the database");
    let serving = std::thread::spawn(move || offer.serve(&what, &never()));
    let now = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs();
    let handed = join::ask(&join::read_code(&text, now).unwrap()).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(serving.join().unwrap(), Outcome::Given(_)));
    assert_eq!(handed.passphrase.as_deref(), Some("a passphrase that goes with it"));
    assert_eq!(handed.from, "a device of the database");

    // the other way round: the new device shows the code, a device of the database reads it and hands over
    let want = Want::open(0, "the new device").expect("an address on the local network");
    let text = want.code();
    let receiving = std::thread::spawn(move || want.receive(&never()));
    let code = join::read_code(&text, now).unwrap();
    assert_eq!((code.kind(), code.name()), (Kind::Wants, "the new device"));
    join::give(&code, &Handover::new(s.login(), None, "a device of the database"), (&s.host, s.port))
        .unwrap_or_else(|e| panic!("{e}"));
    let Received::Taken(given, _) = receiving.join().unwrap() else { panic!("the new device took nothing") };
    assert!(given.passphrase.is_none());
    let handed = given.login;

    let home = std::env::temp_dir().join(format!("face-db-join-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        // (what came the second way: the first is the same login)
        let kept = face_db::setup(&handed, &home).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((kept.host.as_str(), kept.port, kept.schema.as_str()), (s.host.as_str(), s.port, s.schema.as_str()));
        let file = std::fs::read_to_string(home.join("database.json")).unwrap();
        assert!(file.contains("BEGIN CERTIFICATE"), "the server's authority is kept");
        assert!(file.contains("\"dpapi:") || file.contains("\"plain:"), "the password as paths.protect keeps it");
        let client = face_db::connect_read_only(&kept).await.unwrap_or_else(|e| panic!("{e}"));
        let one: i32 = client.query_one("SELECT 1", &[]).await.unwrap().get(0);
        assert_eq!(one, 1);
    });
    let _ = std::fs::remove_dir_all(home);
}

/// The location history against the real server: rows go in once, what is no location is left out, each
/// device's newest row and a device's history are read back, a device's history is deleted at its owner's
/// word, and rows older than they are kept go.
#[test]
fn the_location_history_takes_rows_once_and_lets_old_ones_go() {
    use face_db::location::{self, Row};
    use serde_json::json;
    let Some(s) = settings() else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let schema = made_up_schema();
        let home = std::env::temp_dir().join(format!("face-db-location-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let device = Device::load(&home, "a test device").unwrap();
        let other = Device::load(&home.join("other"), "another").unwrap();
        let mut client = face_db::connect(&s, &schema, &device).await.expect("the server");
        client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();
        let outcome = AssertUnwindSafe(async {
            face_db::migrate(&mut client).await?;
            let now: i64 = client
                .query_one("SELECT (extract(epoch FROM now()) * 1000)::bigint", &[])
                .await
                .map_err(|e| e.to_string())?
                .get(0);
            let (first, second) = (now - 7_200_000, now - 60_000);
            let rows = vec![
                json!({"lat": 39.8017, "lon": -89.6437, "acc": 61.5, "at": first, "source": "system",
                       "place": "Springfield, Illinois,\n United States"}),
                json!({"lat": 500, "lon": 0, "acc": 5, "at": first + 1, "source": "system", "place": "nowhere"}),
                json!({"lat": 48.8534, "lon": 2.3488, "acc": null, "at": second, "source": "hand", "place": "Paris"}),
            ];
            // every row is dealt with: the two locations are in, the third never will be
            let mut done = location::send(&client, &device.id, &rows).await?;
            done.sort_unstable();
            assert_eq!(done, [first, first + 1, second]);
            assert_eq!(location::count(&client, &device.id).await?, 2);
            // sent again (a device that didn't hear the answer): nothing is there twice
            location::send(&client, &device.id, &rows).await?;
            assert_eq!(location::count(&client, &device.id).await?, 2);
            let history = location::history(&client, &device.id, 50).await?;
            assert_eq!(history.len(), 2);
            assert_eq!(
                history[0],
                Row { lat: 48.8534, lon: 2.3488, accuracy: None, at: second, source: "hand".into(), place: "Paris".into() }
            );
            assert_eq!((history[1].accuracy, history[1].at), (Some(61.5), first));
            assert_eq!(history[1].place, "Springfield, Illinois, United States", "a place is one line of text");
            assert_eq!(location::history(&client, &device.id, 1).await?.len(), 1);
            // each device's newest row
            location::send(&client, &other.id, &rows[..1]).await?;
            let mut latest = location::latest(&client).await?;
            latest.sort_by_key(|(_, row)| row.at);
            assert_eq!(latest.len(), 2);
            assert_eq!((latest[0].0.as_str(), latest[0].1.at), (other.id.as_str(), first));
            assert_eq!((latest[1].0.as_str(), latest[1].1.at), (device.id.as_str(), second));
            // a row somebody wrote by hand that is no location is not read as one
            let forced = client
                .execute(
                    "INSERT INTO location_log (device, at, lat, lon, source) VALUES ($1::text::uuid, now(), 1, 1, 'guess')",
                    &[&other.id],
                )
                .await;
            assert!(forced.is_err(), "the table took a row whose source is no source");
            // what is no device's id is refused before anything is asked
            assert!(location::send(&client, "not an id", &rows).await.is_err());
            assert!(location::history(&client, "x' OR '1'='1", 5).await.is_err());
            // a row older than it is kept goes; the others stay
            let old = now - (i64::from(location::KEEP_DAYS) + 1) * 86_400_000;
            location::send(&client, &device.id, &[json!({"lat": 1.5, "lon": 2.5, "acc": 9, "at": old, "source": "system"})])
                .await?;
            assert_eq!(location::count(&client, &device.id).await?, 3);
            assert_eq!(location::prune(&client).await?, 1);
            assert_eq!(location::count(&client, &device.id).await?, 2);
            // a device's whole history, at its owner's word: the other device's stays
            assert_eq!(location::forget(&client, &device.id).await?, 2);
            assert_eq!(location::count(&client, &device.id).await?, 0);
            assert_eq!(location::count(&client, &other.id).await?, 1);
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
