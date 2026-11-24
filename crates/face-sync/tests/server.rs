//! The sync's steps against a real server, in a schema of this test's own that is dropped afterwards, also when the test fails: a story is caught where it stops, the schema dropped, and
//! the failure then passed on.
//! `FACE_TEST_DB_HOME` names a private folder with a `database.json` whose role may create schemas; without
//! it the test says so and passes. Nothing here touches the schema the devices share: every statement runs
//! with the test's schema first in view, and only that schema is dropped, by the name this test made up.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use face_core::faces::REC_MODELS;
use face_db::{Device, Settings};
use face_library::labels::LabelDb;
use face_sync::{
    apply,
    home::Home,
    rows::{FaceRow, Picture, Row},
    state::State,
    sync::{self, Did},
};
use futures_util::FutureExt;
use serde_json::{Map, Value, json};
use tokio_postgres::Client;

/// One device: its library, what it remembers of its syncing, and its own connection (the change log says
/// who wrote a row by the connection it came over).
struct Dev {
    device: Device,
    db: LabelDb,
    web: Map<String, Value>,
    state: State,
    client: Client,
    said: Vec<String>,
    /// its folders, and its private folder's part of the sync when that is synced too
    data: PathBuf,
    private: PathBuf,
    home: Option<Home>,
}

impl Dev {
    async fn new(dir: &Path, name: &str, s: &Settings, schema: &str) -> Dev {
        let (data, home) = (dir.join(name).join("data"), dir.join(name).join("home"));
        let device = Device::load(&home, name).unwrap();
        Dev {
            db: LabelDb::open(&data, &home, REC_MODELS[0], false, &mut |_| {}).unwrap(),
            web: Map::new(),
            state: State::open(&home.join("sync.sqlite")).unwrap(),
            client: face_db::connect(s, schema, &device).await.expect("the server"),
            device,
            said: Vec::new(),
            data,
            private: home,
            home: None,
        }
    }

    async fn step(&mut self) -> Result<Did, String> {
        let said = &mut self.said;
        sync::step(
            &mut self.client,
            &self.device.id,
            &self.device.name,
            &sync::Own::new(&mut self.db, &mut self.web),
            &self.state,
            self.home.as_mut(),
            true,
            &mut |l| said.push(l),
        )
        .await
    }

    /// Its camera sees these people now.
    async fn sees(&mut self, names: &[&str]) -> Result<(), String> {
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        self.home.as_mut().expect("its private folder").cameras(&self.client, &self.device.name, &names).await
    }

    async fn count(&self, what: &str) -> i64 {
        self.client.query_one(&format!("SELECT count(*) FROM {what}"), &[]).await.unwrap().get(0)
    }

    fn name_of(&self, fid: &str) -> Option<&str> {
        self.db.labels.get(fid)?.get("n")?.as_str()
    }
}

fn face(print: Option<Vec<f32>>) -> Row {
    Row::Face(Box::new(FaceRow {
        photo: "h1".to_string(),
        file: "a.jpg".to_string(),
        place: vec![10.0, 20.0, 30.0, 40.0, 0.9],
        quality: Some(0.75),
        model: REC_MODELS[0].name.to_string(),
        picture: if print.is_some() { Picture::Jpeg(b"\xff\xd8a picture".to_vec()) } else { Picture::None },
        print,
        possible: None,
        frame: None,
    }))
}

fn did(pushed: usize, pulled: usize) -> Did {
    Did { pushed, pulled, joined: false, dirty: false }
}

#[test]
fn devices_join_a_database_and_take_each_others_changes() {
    let Some(s) = std::env::var_os("FACE_TEST_DB_HOME").and_then(|h| Settings::load(&PathBuf::from(h)).ok().flatten())
    else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let schema = format!("t_{}", &face_db::new_id().replace('-', "")[..12]);
        let dir = std::env::temp_dir().join(format!("face-sync-server-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = Dev::new(&dir, "a", &s, &schema).await;
        a.client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();
        let outcome = AssertUnwindSafe(async {
            face_db::migrate(&mut a.client).await?;
            let mut b = Dev::new(&dir, "b", &s, &schema).await;
            for d in [&a, &b] {
                face_db::register(&d.client, &d.device, "Test OS", "0.0.0-test").await?;
            }

            // A has a library: a photo with two faces, one named, a Web search answer, two lines of history
            let dim = a.db.store.dim;
            let print = |at: usize| (0..dim).map(|i| if i == at { 1.0 } else { 0.0 }).collect::<Vec<f32>>();
            let photo = json!({"name": "a.jpg", "size": 10, "fp": "ab12", "faces": 2, "dup_of": null, "video": null, "fps": null,
                               "maybe_read": false, "extra": {}, "facts": {"t": 1_700_000_000, "tags": ["Ann"], "faces": {}}});
            apply::apply(&mut a.db, &mut a.web, "photos", vec![("h1".to_string(), Some(Row::Plain(photo)))], &mut |_| ())?;
            let faces = vec![("f1".to_string(), Some(face(Some(print(1))))), ("f2".to_string(), Some(face(Some(print(2)))))];
            apply::apply(&mut a.db, &mut a.web, "faces", faces, &mut |_| ())?;
            let label = Row::Plain(json!({"n": "Ann", "by": "me", "s": null, "ok": null, "not": null}));
            apply::apply(&mut a.db, &mut a.web, "labels", vec![("f1".to_string(), Some(label))], &mut |_| ())?;
            let answer = Row::Plain(json!({"when": "2026-11-12T10:20:30Z", "suggestions": []}));
            apply::apply(&mut a.db, &mut a.web, "web_suggestions", vec![("f1".to_string(), Some(answer))], &mut |_| ())?;
            a.db.store.photos["h1"]["path"] = json!("a folder of this device/a.jpg");
            let history = a.db.data().join("history.jsonl");
            std::fs::write(&history, "{\"t\": \"2026-11-12T10:20:31Z\", \"what\": \"named\", \"faces\": {\"f1\": null}}\nnot a line\n").unwrap();

            // A joins: everything it has is in the database, and where its photo is
            assert!(a.step().await?.joined);
            assert!(a.said.iter().any(|l| l == "Joined the shared database: sent 2 faces and 1 labels; this device now has 2 faces."), "{:?}", a.said);
            for (what, n) in [("photos", 1), ("faces WHERE print IS NOT NULL AND picture IS NOT NULL", 2), ("labels", 1), ("web_suggestions", 1)] {
                assert_eq!(a.count(what).await, n, "{what}");
            }
            assert_eq!((a.count("photo_paths").await, a.count("label_history").await), (1, 1));
            // joined, and nothing changed since: nothing is sent, nothing taken
            assert_eq!(a.step().await?, did(0, 0));

            // B joins with nothing and has it all, but for what is A's own (where the photo's file is)
            assert!(b.step().await?.joined);
            assert_eq!((b.db.n(), b.name_of("f1"), b.web.len()), (2, Some("Ann"), 1));
            assert_eq!((b.db.store.print(0), b.db.store.thumbs[0].as_slice()), (&print(1)[..], &b"\xff\xd8a picture"[..]));
            assert!(b.db.store.photos["h1"].get("path").is_none());
            assert_eq!(b.db.facts.facts["h1"].tags, ["Ann"]);
            assert_eq!((b.count("photo_paths").await, b.count("label_history").await), (1, 1), "B has no file and no history of its own");

            // B names the other face; A takes it at its next step, and B is not handed its own change back
            b.db.labels.insert("f2".to_string(), json!({"n": "Bob", "by": "me"}));
            b.db.labels_changed().map_err(|e| e.to_string())?;
            assert_eq!(b.step().await?, did(1, 0));
            assert_eq!(a.step().await?, did(0, 1));
            assert_eq!(a.name_of("f2"), Some("Bob"));
            assert_eq!(b.step().await?, did(0, 0));

            // A takes a name away and adds a line to its history; B loses the name
            a.db.labels.remove("f1");
            a.db.labels_changed().map_err(|e| e.to_string())?;
            std::fs::write(&history, "{\"what\": \"1\"}\nnot a line\n{\"t\": \"2026-11-12T10:20:32Z\", \"what\": \"unnamed\", \"faces\": {}}\n").unwrap();
            assert_eq!(a.step().await?, did(1, 0));
            assert_eq!((a.count("labels").await, a.count("label_history").await), (1, 2));
            assert_eq!(b.step().await?, did(0, 1));
            assert_eq!((b.name_of("f1"), b.name_of("f2")), (None, Some("Bob")));

            // a face's print deleted on A (Ignored there) is deleted on B, and doesn't come back from B
            apply::apply(&mut a.db, &mut a.web, "faces", vec![("f1".to_string(), Some(face(None)))], &mut |_| ())?;
            assert_eq!(a.step().await?, did(1, 0));
            assert_eq!(b.step().await?, did(0, 1));
            assert!(b.db.store.printless(0) && b.db.store.thumbs[0].is_empty() && !b.db.store.printless(1));
            assert_eq!(a.count("faces WHERE print IS NULL AND picture IS NULL").await, 1);

            // a row nobody can use (written with only the database's login) is left out and said, and stops
            // nothing: the change beside it is taken
            a.client.execute("INSERT INTO labels (fid, n, by) VALUES ('f9', '__proto__', 'me'), ('f1', 'Cy', 'me')", &[]).await.map_err(|e| e.to_string())?;
            assert_eq!(b.step().await?, did(0, 2));
            assert_eq!((b.name_of("f9"), b.name_of("f1")), (None, Some("Cy")));
            assert!(b.said.iter().any(|l| l.contains("the labels row f9 can't be used here and was left out")), "{:?}", b.said);

            // read again from their files, both are what was synced; and a device that knows this database
            // doesn't join it again
            let (dim_b, names): (usize, Vec<String>) = (b.db.n(), b.db.labels.keys().cloned().collect());
            drop(b);
            let mut b = Dev::new(&dir, "b", &s, &schema).await;
            b.web = serde_json::from_slice(&std::fs::read(b.db.data().join("web_suggestions.json")).unwrap()).unwrap();
            assert_eq!((b.db.n(), b.db.labels.keys().cloned().collect::<Vec<_>>(), b.web.len()), (dim_b, names, 1));
            assert_eq!(b.step().await?, did(0, 0));
            Ok::<(), String>(())
        })
        .catch_unwind()
        .await;
        let dropped = a.client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await;
        drop(a);
        let _ = std::fs::remove_dir_all(&dir);
        dropped.unwrap();
        match outcome {
            Ok(told) => told.unwrap(),
            Err(failed) => std::panic::resume_unwind(failed),
        }
    });
}

#[test]
fn private_folders_are_synced_and_what_the_server_holds_of_a_forgotten_person_goes() {
    let Some(s) = std::env::var_os("FACE_TEST_DB_HOME").and_then(|h| Settings::load(&PathBuf::from(h)).ok().flatten())
    else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let schema = format!("t_{}", &face_db::new_id().replace('-', "")[..12]);
        let dir = std::env::temp_dir().join(format!("face-sync-server-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = Dev::new(&dir, "a", &s, &schema).await;
        a.client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();
        let outcome = AssertUnwindSafe(async {
            face_db::migrate(&mut a.client).await?;
            let mut b = Dev::new(&dir, "b", &s, &schema).await;
            for d in [&mut a, &mut b] {
                face_db::register(&d.client, &d.device, "Test OS", "0.0.0-test").await?;
                d.home = Some(Home::new(&d.data, &d.private, &d.device.id));
            }
            let text = |file: PathBuf| std::fs::read_to_string(file).unwrap_or_default();

            // A's private folder, and its history, which names someone it has forgotten since
            let gone = "Jos\u{e9} Gone";
            std::fs::write(a.private.join("descriptions.json"), r#"{"h1": {"text": "one", "when": "2026-11-10T08:00:00Z"}}"#).unwrap();
            std::fs::write(a.private.join("alerts.json"), r#"[{"id": "a1", "t": 5000, "kind": "left", "name": "Ann", "text": "Ann left", "state": "new"}]"#).unwrap();
            std::fs::create_dir_all(a.private.join("presence")).unwrap();
            std::fs::write(a.private.join("presence").join("2026-11-10.jsonl"), "{\"name\": \"Ann\", \"start\": 1762765200000, \"end\": 1762765260000}\n").unwrap();
            std::fs::write(a.private.join("settings.json"), r#"{"track": {"ft-strict": "0.35"}}"#).unwrap();
            std::fs::write(a.private.join("person_signals.json"), r#"{"usual": {"Ann": {"n": 3}}}"#).unwrap();
            std::fs::write(a.private.join("forgotten.json"), serde_json::to_string(&json!([gone])).unwrap()).unwrap();
            let lines = [
                json!({"t": "2026-11-10T08:00:00Z", "what": "named Ann", "faces": {"f1": {"n": "Ann", "by": "me"}}}),
                json!({"t": "2026-11-10T08:00:01Z", "what": "named 1 face", "faces": {"f2": {"n": gone, "by": "me"}}}),
                json!({"t": "2026-11-10T08:00:02Z", "what": format!("renamed {gone}"), "faces": {}}),
            ];
            std::fs::write(a.db.data().join("history.jsonl"), lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();

            // A joins: its folder is in the database, with the device it is from
            assert!(a.step().await?.joined);
            for (what, n) in [("descriptions", 1), ("alerts WHERE name = 'Ann'", 1), ("presence", 1), ("label_history", 3), ("forgotten", 0)] {
                assert_eq!(a.count(what).await, n, "{what}");
            }
            // its first sending after: its settings, signals and whom it forgot; what the server holds about
            // them goes, also a line that names them only among its faces (the database writes "é" as "é")
            let sent = a.step().await?;
            assert_eq!((sent.pushed, sent.pulled), (9, 0), "7 settings, a person's signals, a forgotten name");
            assert_eq!((a.count("device_settings").await, a.count("person_signals").await, a.count("forgotten").await), (7, 1, 1));
            assert_eq!(a.count("label_history").await, 1, "the lines about the forgotten person are gone");
            assert_eq!(a.step().await?, did(0, 0));

            // B joins with nothing: it has A's rows, but not A's settings; and it is asked to forget, once
            let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let noted = asked.clone();
            b.home.as_mut().unwrap().forget = Some(Box::new(move |name| {
                noted.lock().unwrap().push(name.to_string());
                Ok(())
            }));
            assert!(b.step().await?.joined);
            assert_eq!(text(b.private.join("descriptions.json")), "{\n\"h1\": {\"text\": \"one\", \"when\": \"2026-11-10T08:00:00Z\"}\n}");
            assert!(text(b.private.join("alerts.json")).contains("\"a1\"") && text(b.private.join("person_signals.json")).contains("\"Ann\""));
            let visits: String = std::fs::read_dir(b.private.join("presence")).unwrap().flatten().map(|f| text(f.path())).collect();
            assert!(visits.contains(&format!("\"device\": \"{}\"", a.device.id)), "A's visit keeps its device: {visits}");
            assert!(!b.private.join("settings.json").exists(), "a device's settings are its own");
            assert_eq!(*asked.lock().unwrap(), [gone]);
            assert!(text(b.private.join("forgotten.json")).contains("Gone"));
            let sent = b.step().await?;
            assert_eq!((sent.pushed, sent.pulled), (6, 0), "B's own settings");
            assert_eq!(b.step().await?, did(0, 0));
            assert_eq!(*asked.lock().unwrap(), [gone], "asked once");

            // someone with the database's password changes one of B's settings and adds one: B's stay as
            // they are, the database gets them again and loses the one B never wrote
            let changed = "UPDATE device_settings SET value = '\"http://elsewhere.example\"' WHERE device = $1::text::uuid AND key = 'url'";
            a.client.execute(changed, &[&b.device.id]).await.map_err(|e| e.to_string())?;
            let added = "INSERT INTO device_settings (device, app, key, value) VALUES ($1::text::uuid, 'track', 'ft-stream', '\"1\"')";
            a.client.execute(added, &[&b.device.id]).await.map_err(|e| e.to_string())?;
            b.said.clear();
            let took = b.step().await?;
            assert_eq!((took.pushed, took.pulled, took.dirty), (0, 2, true));
            assert!(b.said.iter().any(|l| l.contains("2 of this device's settings are different in the database")), "{:?}", b.said);
            assert!(!b.private.join("settings.json").exists() && !b.private.join("describe.json").exists());
            assert_eq!(b.step().await?.pushed, 2, "one sent again, one deleted");
            assert_eq!(b.count("device_settings WHERE key = 'ft-stream' OR value = '\"http://elsewhere.example\"'").await, 0);

            // A deletes its description and adds a visit; B follows
            std::fs::write(a.private.join("descriptions.json"), "{}").unwrap();
            let more = "{\"name\": \"Ann\", \"start\": 1762765300000, \"end\": 1762765360000}\n";
            std::fs::write(a.private.join("presence").join("2026-11-10.jsonl"), format!("{{\"name\": \"Ann\", \"start\": 1762765200000, \"end\": 1762765260000}}\n{more}")).unwrap();
            assert_eq!(a.step().await?.pushed, 2);
            assert_eq!(b.step().await?.pulled, 2);
            assert_eq!(text(b.private.join("descriptions.json")), "{}");
            let visits: String = std::fs::read_dir(b.private.join("presence")).unwrap().flatten().map(|f| text(f.path())).collect();
            assert_eq!(visits.lines().count(), 2);

            // someone moves between the two cameras: seen on A's, then on B's. Nothing is noted until it is
            // switched on, and only people on a device's alert list
            a.sees(&["Ann"]).await?;
            assert_eq!(a.count("sightings").await, 0, "off until switched on");
            for d in [&a, &b] {
                std::fs::write(d.private.join("settings.json"), r#"{"track": {"ft-o-optAlertMove": "1", "ft-alertWho": "ann, Bob"}}"#).unwrap();
            }
            a.sees(&["Ann", "Cy"]).await?;
            assert_eq!(a.count("sightings").await, 1, "Cy is on nobody's list");
            let alerts = |d: &Dev| text(d.private.join("alerts.json")).matches("\"moved\"").count();
            assert_eq!(alerts(&a), 0, "seen on one camera only");
            b.sees(&["Ann"]).await?;
            assert_eq!((b.count("sightings").await, alerts(&b), alerts(&a)), (2, 1, 0), "the device they arrived at raises it");
            assert!(text(b.private.join("alerts.json")).contains("Ann moved from a's camera to b's"));
            b.sees(&["Ann"]).await?;
            assert_eq!(alerts(&b), 1, "still there: not arrived again");
            // the alert reaches A like every other
            assert!(b.step().await?.pushed >= 1);
            a.step().await?;
            assert_eq!(alerts(&a), 1);

            // Copy settings, the owner's own act on B: A's page settings and which describer it uses are B's
            // from then on, what only B had is gone, and where a device sends photos is never copied
            std::fs::write(a.private.join("settings.json"), r#"{"track": {"ft-strict": "0.2"}}"#).unwrap();
            a.step().await?;
            let home_b = b.home.as_ref().unwrap();
            let copied = home_b.copy_settings(&mut b.client, &a.device.id.to_uppercase(), &mut |_| ()).await.map_err(|e| e.text().to_string())?;
            assert_eq!(copied, 3, "a page's setting, which describer and its model");
            let mine: Value = serde_json::from_str(&text(b.private.join("settings.json"))).unwrap();
            assert_eq!(mine["track"], json!({"ft-strict": "0.2"}));
            let own = b.device.id.clone();
            for (from, why) in [(own.as_str(), "That's this device."), ("not an id", "badly formed hexadecimal UUID string"),
                                ("00000000-0000-4000-8000-000000000000", "That device has no settings in the shared database.")] {
                let refused = home_b.copy_settings(&mut b.client, from, &mut |_| ()).await.unwrap_err();
                assert_eq!((refused.text(), refused.refused()), (why, true));
            }
            Ok::<(), String>(())
        })
        .catch_unwind()
        .await;
        let dropped = a.client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await;
        drop(a);
        let _ = std::fs::remove_dir_all(&dir);
        dropped.unwrap();
        match outcome {
            Ok(told) => told.unwrap(),
            Err(failed) => std::panic::resume_unwind(failed),
        }
    });
}

#[test]
fn the_passphrase_for_keys_reaches_only_the_devices_the_owner_lets_in() {
    use face_home::saved;
    use face_sync::keys::Fault;
    let Some(s) = std::env::var_os("FACE_TEST_DB_HOME").and_then(|h| Settings::load(&PathBuf::from(h)).ok().flatten())
    else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let schema = format!("t_{}", &face_db::new_id().replace('-', "")[..12]);
        let dir = std::env::temp_dir().join(format!("face-sync-server-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = Dev::new(&dir, "a", &s, &schema).await;
        a.client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();
        let outcome = AssertUnwindSafe(async {
            face_db::migrate(&mut a.client).await?;
            let (mut b, mut c) = (Dev::new(&dir, "b", &s, &schema).await, Dev::new(&dir, "c", &s, &schema).await);
            for d in [&mut a, &mut b, &mut c] {
                face_db::register(&d.client, &d.device, "Test OS", "0.0.0-test").await?;
                d.home = Some(Home::new(&d.data, &d.private, &d.device.id));
                assert!(d.step().await?.joined);
            }
            let told = |e: Fault| e.text().to_string();
            // (keys made up for the test, put together here so that nothing in this file looks like one)
            let (first, second) = (format!("{}-{}", "made_up_for_the_test", "0123456789"), format!("{}-{}", "another_made_up_one", "9876543210"));

            // A sets the passphrase: too short is refused, typed twice it must be the same
            let keys_a = a.home.as_ref().unwrap().keys();
            assert!(matches!(keys_a.set_passphrase(&mut a.client, "too short", None).await, Err(Fault::Bad(_))));
            assert!(matches!(keys_a.set_passphrase(&mut a.client, "a made-up passphrase", Some("another")).await, Err(Fault::Bad(_))));
            let set = keys_a.set_passphrase(&mut a.client, "a made-up passphrase", Some("a made-up passphrase")).await.map_err(told)?;
            assert_eq!((set["what"].as_str(), set["version"].as_i64()), (Some("set"), Some(1)));
            // A saves a key: after its next steps it is in the database, as ciphertext
            saved::save_google_key(&a.private, &first)?;
            a.step().await?;
            assert_eq!(a.step().await?.pushed, 1, "the key");
            let kept: Vec<u8> = a.client.query_one("SELECT ciphertext FROM secrets WHERE name = 'google_vision'", &[]).await.map_err(|e| e.to_string())?.get(0);
            assert!(!kept.windows(first.len()).any(|w| w == first.as_bytes()));

            // B has the database's password and nothing else: it reads no key, and a wrong passphrase gets it nothing
            b.step().await?;
            b.step().await?;
            assert_eq!(saved::google_key(&b.private), "");
            let keys_b = b.home.as_ref().unwrap().keys();
            let wrong = keys_b.set_passphrase(&mut b.client, "not the passphrase", None).await.unwrap_err();
            assert!(matches!(&wrong, Fault::Wrong(t) if t.starts_with("That isn't the current passphrase.")), "{wrong:?}");
            // typed there by the owner, B has it and reads the key
            let entered = keys_b.set_passphrase(&mut b.client, "a made-up passphrase", None).await.map_err(told)?;
            assert_eq!((entered["what"].as_str(), entered["fingerprint"].clone()), (Some("entered"), set["fingerprint"].clone()));
            b.step().await?;
            assert_eq!(saved::google_key(&b.private), first);

            // B changes the passphrase: every key is encrypted anew, and A, which shows it held the old one,
            // follows without being told anything
            let keys_b = b.home.as_ref().unwrap().keys();
            let changed = keys_b.set_passphrase(&mut b.client, "another made-up passphrase", Some("another made-up passphrase")).await.map_err(told)?;
            assert_eq!((changed["what"].as_str(), changed["version"].as_i64()), (Some("changed"), Some(2)));
            let again: Vec<u8> = b.client.query_one("SELECT ciphertext FROM secrets WHERE name = 'google_vision'", &[]).await.map_err(|e| e.to_string())?.get(0);
            assert_ne!(again, kept);
            a.step().await?;
            let keys_a = a.home.as_ref().unwrap().keys();
            assert_eq!((keys_a.local_version(), keys_a.saved_passphrase().as_str()), (Some(2), "another made-up passphrase"));
            saved::save_google_key(&b.private, &second)?;
            b.step().await?;
            b.step().await?;
            a.step().await?;
            assert_eq!(saved::google_key(&a.private), second, "A reads what B encrypted under the new passphrase");

            // C asks: a code on its own screen; typed on A, C is given the passphrase
            let keys_c = c.home.as_ref().unwrap().keys();
            let asked = keys_c.ask(&c.client).await.map_err(told)?;
            let code = asked["code"].as_str().unwrap().to_string();
            assert_eq!((code.len(), keys_c.request().map(|r| r["code"].clone())), (9, Some(json!(code))));
            let keys_a = a.home.as_ref().unwrap().keys();
            assert_eq!(keys_a.waiting(&a.client).await.map_err(told)?.len(), 1);
            let other = if code.starts_with('A') { "BBBB-BBBB" } else { "AAAA-AAAA" };
            assert!(matches!(keys_a.approve(&mut a.client, other, None).await, Err(Fault::Wrong(_))));
            assert!(matches!(keys_a.approve(&mut a.client, "short", None).await, Err(Fault::Bad(_))));
            c.step().await?;
            assert_eq!(c.home.as_ref().unwrap().keys().local_version(), None, "not given yet");
            let given = keys_a.approve(&mut a.client, &code.to_lowercase(), None).await.map_err(told)?;
            assert_eq!(given["given"], "c");
            c.step().await?;
            let keys_c = c.home.as_ref().unwrap().keys();
            assert_eq!((keys_c.local_version(), keys_c.request()), (Some(2), None));
            assert_eq!(saved::google_key(&c.private), second);
            assert_eq!(c.count("key_requests").await, 0);

            // someone with only the database's password makes a version of their own current and seals it to
            // A: A doesn't follow (nothing ties it to the passphrase A has), and neither reads nor sends keys
            let forged = "INSERT INTO passphrase_versions (salt, n, r, p, nonce, ciphertext, fingerprint, note, prev, link, current)
                          SELECT salt, n, r, p, nonce, ciphertext, 'AAAA-BBBB-CCCC', 'forged', 2, '\\x00'::bytea, false FROM passphrase_versions WHERE id = 1";
            a.client.batch_execute(&format!("{forged}; UPDATE passphrase_versions SET current = (note = 'forged')")).await.map_err(|e| e.to_string())?;
            let third = format!("{}-{}", "a_third_made_up_one", "5555555555");
            saved::save_google_key(&a.private, &third)?;
            a.step().await?;
            a.step().await?;
            let keys_a = a.home.as_ref().unwrap().keys();
            assert_eq!((keys_a.local_version(), keys_a.saved_passphrase().as_str()), (Some(2), "another made-up passphrase"));
            let still: Vec<u8> = a.client.query_one("SELECT ciphertext FROM secrets WHERE name = 'google_vision'", &[]).await.map_err(|e| e.to_string())?.get(0);
            assert_eq!(still, {
                let now: Vec<u8> = b.client.query_one("SELECT ciphertext FROM secrets WHERE name = 'google_vision'", &[]).await.map_err(|e| e.to_string())?.get(0);
                now
            });
            assert_eq!(saved::google_key(&b.private), second, "nothing of A's was sent under a passphrase it can't vouch for");
            let overview = keys_a.overview(&a.client).await.map_err(told)?;
            assert_eq!((overview["here"].clone(), overview["current"]["fingerprint"].clone()), (json!(false), json!("AAAA-BBBB-CCCC")));
            // the owner sets things right (here: the forged row gone, the real one current again)
            a.client.batch_execute("DELETE FROM passphrase_versions WHERE note = 'forged'; UPDATE passphrase_versions SET current = (id = 2)").await.map_err(|e| e.to_string())?;
            a.step().await?;
            // (a device that has the current passphrase again reads the keys again: the database's count,
            // as when the passphrase was typed there; what it saved meanwhile is saved again by the owner)
            assert_eq!(saved::google_key(&a.private), second);
            saved::save_google_key(&a.private, &third)?;
            a.step().await?;
            b.step().await?;
            assert_eq!(saved::google_key(&b.private), third);

            // B restores the first passphrase, which it held itself; C, which never had it, may not
            let keys_c = c.home.as_ref().unwrap().keys();
            assert!(matches!(keys_c.restore(&mut c.client, 1).await, Err(Fault::Wrong(t)) if t.starts_with("This device never had that passphrase")));
            let keys_b = b.home.as_ref().unwrap().keys();
            let restored = keys_b.restore(&mut b.client, 1).await.map_err(told)?;
            assert_eq!((restored["what"].as_str(), restored["fingerprint"].clone()), (Some("restored"), set["fingerprint"].clone()));
            assert!(matches!(keys_b.restore(&mut b.client, 999).await, Err(Fault::Bad(_))));
            for d in [&mut a, &mut c] {
                d.step().await?;
                let keys = d.home.as_ref().unwrap().keys();
                assert_eq!(keys.saved_passphrase(), "a made-up passphrase", "followed the restore");
                assert_eq!(saved::google_key(&d.private), third);
            }
            let overview = b.home.as_ref().unwrap().keys().overview(&b.client).await.map_err(told)?;
            let shown = overview["devices"].as_array().unwrap().iter().filter(|d| d["current"] == json!(true)).count();
            assert_eq!((shown, overview["versions"].as_array().unwrap().len(), overview["here"].clone()), (3, 3, json!(true)), "{overview}");

            // D joins by a code: the device that let it in handed a passphrase over with the login
            // (`face_db::join`). One that isn't the current one is dropped and changes nothing; the current
            // one is taken as if the owner had typed it on D, D reads the keys, and hands it on itself
            let mut d = Dev::new(&dir, "d", &s, &schema).await;
            face_db::register(&d.client, &d.device, "Test OS", "0.0.0-test").await?;
            d.home = Some(Home::new(&d.data, &d.private, &d.device.id));
            assert!(d.step().await?.joined);
            let versions = d.count("passphrase_versions").await;
            face_home::passphrase::receive(&d.private, "another made-up passphrase")?; // (an earlier version's)
            d.step().await?;
            assert!(face_home::passphrase::received(&d.private).is_none(), "looked at once");
            assert_eq!(
                (d.home.as_ref().unwrap().keys().local_version(), saved::google_key(&d.private).as_str()),
                (None, "")
            );
            assert_eq!(face_home::passphrase::to_hand_on(&d.private), None);
            face_home::passphrase::receive(&d.private, "a made-up passphrase")?;
            d.step().await?;
            d.step().await?;
            let keys_d = d.home.as_ref().unwrap().keys();
            let current = b.home.as_ref().unwrap().keys().local_version();
            assert_eq!((keys_d.local_version(), keys_d.saved_passphrase().as_str()), (current, "a made-up passphrase"));
            assert_eq!(saved::google_key(&d.private), third);
            assert!(face_home::passphrase::received(&d.private).is_none());
            assert_eq!(d.count("passphrase_versions").await, versions, "what is handed over never makes a version");
            assert_eq!(face_home::passphrase::to_hand_on(&d.private).as_deref(), Some("a made-up passphrase"));
            let overview = keys_d.overview(&d.client).await.map_err(told)?;
            let shown = overview["devices"].as_array().unwrap().iter().filter(|d| d["current"] == json!(true)).count();
            assert_eq!((shown, overview["here"].clone()), (4, json!(true)), "{overview}");

            // every device hears of a new device, once: A and B each notice for themselves who joined since
            // they first looked (at a step after their own joining, when B and C were there), and raise the
            // same alert about D
            let joined = |private: &std::path::Path| -> Vec<(String, String)> {
                let all = face_home::alerts::Inbox::new(private).all().unwrap();
                let mut of: Vec<(String, String)> = (all.iter().filter(|a| a["kind"] == "joined"))
                    .map(|a| (a["name"].as_str().unwrap().to_string(), a["id"].as_str().unwrap().to_string()))
                    .collect();
                of.sort();
                of
            };
            let mut said = Vec::new();
            let mut log = |line: String| said.push(line);
            assert_eq!(a.home.as_mut().unwrap().new_devices(&a.client, &mut log).await?, 1, "D came after A looked");
            assert_eq!(b.home.as_mut().unwrap().new_devices(&b.client, &mut log).await?, 1, "and after B looked");
            assert_eq!(a.home.as_mut().unwrap().new_devices(&a.client, &mut log).await?, 0, "told once");
            let (at_a, at_b) = (joined(&a.private), joined(&b.private));
            assert_eq!(at_a.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["d"]);
            assert_eq!(at_b, at_a, "the same alert, whoever noticed");
            // D itself looked when it joined: it noted who was there and raised nothing
            assert_eq!(d.home.as_mut().unwrap().new_devices(&d.client, &mut log).await?, 0);
            assert!(joined(&d.private).is_empty());

            // The alert is this device's own knowledge, not a date in the row: a row written with an old date
            // (whoever has the login can write one) is a new device all the same.
            let old = face_db::new_id();
            a.client
                .execute(
                    "INSERT INTO devices (id, name, os, app_version, first_seen, last_seen) \
                     VALUES ($1::text::uuid, 'written by hand', 'x', '0', now() - interval '30 days', now())",
                    &[&old],
                )
                .await
                .map_err(|e| e.to_string())?;
            assert_eq!(a.home.as_mut().unwrap().new_devices(&a.client, &mut log).await?, 1, "an old date hides nothing");
            assert!(joined(&a.private).iter().any(|(name, _)| name == "written by hand"));
            // A device this one knows that shows another certificate than before: told, once. The first
            // certificate a device shows is only noted.
            let cert = |sha: String| {
                let (client, old) = (&a.client, old.clone());
                async move {
                    client
                        .execute("UPDATE devices SET peer_cert_sha256 = $2 WHERE id = $1::text::uuid", &[&old, &sha])
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
            };
            cert("ab".repeat(32)).await?;
            assert_eq!(a.home.as_mut().unwrap().new_devices(&a.client, &mut log).await?, 0, "its first certificate");
            cert("cd".repeat(32)).await?;
            assert_eq!(a.home.as_mut().unwrap().new_devices(&a.client, &mut log).await?, 1, "another certificate");
            assert_eq!(a.home.as_mut().unwrap().new_devices(&a.client, &mut log).await?, 0, "told once");
            // a row whose shape is wrong (no name where a name belongs is fine; nothing here can end the program)
            a.client
                .execute("UPDATE devices SET os = NULL, app_version = NULL WHERE id = $1::text::uuid", &[&old])
                .await
                .map(|_| ())
                .or_else(|_| Ok::<(), String>(()))?; // (where the columns may not be empty, the row stays as it is)
            assert_eq!(a.home.as_mut().unwrap().new_devices(&a.client, &mut log).await?, 0);
            assert!(said.iter().any(|line| line.contains("a new device joined, d")), "{said:?}");
            assert!(said.iter().any(|line| line.contains("written by hand shows a new certificate")), "{said:?}");
            Ok::<(), String>(())
        })
        .catch_unwind()
        .await;
        let dropped = a.client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await;
        drop(a);
        let _ = std::fs::remove_dir_all(&dir);
        dropped.unwrap();
        match outcome {
            Ok(told) => told.unwrap(),
            Err(failed) => std::panic::resume_unwind(failed),
        }
    });
}

#[test]
fn web_searches_count_toward_one_cap_for_every_device() {
    let Some(s) = std::env::var_os("FACE_TEST_DB_HOME").and_then(|h| Settings::load(&PathBuf::from(h)).ok().flatten())
    else {
        eprintln!("skipped: no test database (FACE_TEST_DB_HOME)");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let schema = format!("t_{}", &face_db::new_id().replace('-', "")[..12]);
        let dir = std::env::temp_dir().join(format!("face-sync-server-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = Dev::new(&dir, "a", &s, &schema).await;
        a.client.batch_execute(&format!("CREATE SCHEMA {schema}")).await.unwrap();
        let outcome = AssertUnwindSafe(async {
            face_db::migrate(&mut a.client).await?;
            let b = Dev::new(&dir, "b", &s, &schema).await;
            let count = face_sync::home::count_use;
            // each device's search is counted with the other's
            assert_eq!(count(&a.client, "vision", "2026-11", 4).await?, Some(1));
            assert_eq!(count(&b.client, "vision", "2026-11", 4).await?, Some(2));
            assert_eq!(count(&a.client, "vision", "2026-11", 4).await?, Some(3));
            // the last one before the cap is taken once, whoever asks next is refused, and nothing is counted past it
            assert_eq!(count(&b.client, "vision", "2026-11", 4).await?, Some(4));
            assert_eq!(count(&a.client, "vision", "2026-11", 4).await?, None);
            assert_eq!(count(&b.client, "vision", "2026-11", 4).await?, None);
            let used: i32 = a
                .client
                .query_one("SELECT used FROM usage WHERE service = 'vision' AND month = '2026-11'", &[])
                .await
                .map_err(|e| e.to_string())?
                .get(0);
            assert_eq!(used, 4);
            // another service and another month count for themselves
            assert_eq!(count(&a.client, "rekognition", "2026-11", 4).await?, Some(1));
            assert_eq!(count(&b.client, "vision", "2026-12", 4).await?, Some(1));
            Ok::<(), String>(())
        })
        .catch_unwind()
        .await;
        let dropped = a.client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await;
        drop(a);
        let _ = std::fs::remove_dir_all(&dir);
        dropped.unwrap();
        match outcome {
            Ok(told) => told.unwrap(),
            Err(failed) => std::panic::resume_unwind(failed),
        }
    });
}
