//! The sync against a real database server, a step at a time, for a check of what each does:
//!
//!     sync_db_try push <a data folder> <a private folder> <a folder with database.json> <a schema>
//!     sync_db_try take <a data folder> <a private folder> <a folder with database.json> <a schema>
//!     sync_db_try step <a data folder> <a private folder> <a folder with database.json> <a schema> [home [twice [four keys to save between the two]]]
//!     sync_db_try key <a data folder> <a private folder> <a folder with database.json> <a schema> <action> [...]
//!
//! `push` sends every row of the library that isn't as this device last synced it (what it remembers is in
//! the private folder's sync.sqlite) and deletes what is gone here; `take` takes every row the schema has
//! into the library; `step` is one step of the sync itself (`sync::step`: joining the first time, else sending
//! what changed and taking what other devices changed). Each prints a line
//! of JSON with what it did. The schema must be one made for a test
//! (its name begins "t_"): this never touches the schema the devices share.

use std::{collections::HashMap, path::PathBuf, time::Instant};

use face_core::faces::REC_MODELS;
use face_db::{Device, Settings};
use face_home::saved;
use face_library::labels::LabelDb;
use face_sync::{
    apply,
    home::Home,
    keys::{Fault, Keys},
    local, server, state, sync,
};
use ring::digest::{SHA256, digest};
use serde_json::{Map, Value, json};

async fn run() -> Result<Value, String> {
    let mut args = std::env::args().skip(1);
    let what = args.next().ok_or("push, take or step")?;
    let mut folder = || args.next().map(PathBuf::from).ok_or("a folder is missing");
    let (data, home, db_home) = (folder()?, folder()?, folder()?);
    let schema = args.next().filter(|s| s.starts_with("t_")).ok_or("a schema made for a test (t_...) is needed")?;
    let mut db = LabelDb::open(&data, &home, REC_MODELS[0], false, &mut |_| {}).map_err(|e| e.to_string())?;
    let web_file = data.join("web_suggestions.json");
    let mut web: Map<String, Value> =
        std::fs::read(&web_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let settings = Settings::load(&db_home)?.ok_or("no database is set up in that folder")?;
    let device = Device::load(&home, "a check's device")?;
    let mut client = face_db::connect(&settings, &schema, &device).await?;
    face_db::register(&client, &device, "check", "0.0.0-test").await?;
    let state = state::State::open(&home.join("sync.sqlite"))?;
    let began = Instant::now();
    let mut did = Map::new();
    match what.as_str() {
        "push" => {
            let now = local::digests(&db, &web);
            for tbl in local::TABLES {
                let (changed, gone) = state::pending(&now[tbl], &state.digests(tbl)?);
                let rows = local::rows(&db, &web, tbl, &changed);
                server::push(&mut client, &device.id, tbl, &rows, &gone).await?;
                let mut kept: HashMap<String, Option<String>> =
                    rows.iter().map(|(k, _)| (k.clone(), now[tbl].get(k).cloned())).collect();
                kept.extend(gone.iter().map(|k| (k.clone(), None)));
                state.set_digests(tbl, &kept)?;
                did.insert(tbl.to_string(), json!({"sent": rows.len(), "deleted": gone.len()}));
            }
        }
        "take" => {
            for tbl in local::TABLES {
                let keys = server::all_keys(&client, tbl).await?;
                let (mut taken, mut left_out) = (0, 0);
                for part in keys.chunks(2000) {
                    // a face's picture that this device has the same of isn't fetched again
                    let pictures: HashMap<String, Vec<u8>> = if tbl == "faces" {
                        (part.iter())
                            .filter_map(|k| Some((k, &db.store.thumbs[db.store.index_of(k)?])))
                            .filter(|(_, t)| !t.is_empty())
                            .map(|(k, t)| (k.clone(), digest(&SHA256, t).as_ref().to_vec()))
                            .collect()
                    } else {
                        HashMap::new()
                    };
                    let got = server::fetch(&client, tbl, part, &pictures).await?;
                    let good: Vec<_> = (got.into_iter())
                        .filter(|(k, row)| {
                            let ok = row.as_ref().is_none_or(|r| apply::check(&db, tbl, k, r).is_ok());
                            left_out += usize::from(!ok);
                            ok
                        })
                        .collect();
                    taken += good.len();
                    let kept = apply::apply(&mut db, &mut web, tbl, good, &mut |line| eprintln!("{line}"))?;
                    state.set_digests(tbl, &kept)?;
                }
                did.insert(tbl.to_string(), json!({"taken": taken, "left out": left_out}));
            }
        }
        "step" => {
            // with the private folder's tables too, when asked ("home" as a sixth word)
            let mut part = (args.next().as_deref() == Some("home")).then(|| Home::new(&data, &home, &device.id));
            // (forgetting a person on this device is not here yet: it is noted that this device was asked to)
            if let Some(part) = part.as_mut() {
                let asked = home.join("forgot.log");
                part.forget = Some(Box::new(move |name| {
                    use std::io::Write;
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&asked)
                        .map_err(|e| e.to_string())?;
                    file.write_all(format!("{name}\n").as_bytes()).map_err(|e| e.to_string())
                }));
            }
            // "twice": two steps one after the other, as a program that stays up takes them (what a device
            // learns in a step, that it has the current passphrase, it uses in the next)
            let times = if args.next().as_deref() == Some("twice") { 2 } else { 1 };
            // keys the owner saves in Settings while the program is up: saved between the two steps
            let to_save: Vec<String> = args.by_ref().collect();
            let mut said = Vec::new();
            let (mut joined, mut pushed, mut pulled) = (false, 0, 0);
            for k in 0..times {
                if let (1, [google, id, secret, describe]) = (k, to_save.as_slice()) {
                    saved::save_google_key(&home, google)?;
                    saved::save_aws(&home, id, secret, None)?;
                    saved::save_describe_key(&home, describe)?;
                }
                let done = sync::step(
                    &mut client,
                    &device.id,
                    &device.name,
                    &sync::Own::new(&mut db, &mut web),
                    &state,
                    part.as_mut(),
                    true,
                    &mut |l| said.push(l),
                )
                .await?;
                (joined, pushed, pulled) = (joined || done.joined, pushed + done.pushed, pulled + done.pulled);
            }
            did.insert("joined".to_string(), json!(joined));
            did.insert("pushed".to_string(), json!(pushed));
            did.insert("pulled".to_string(), json!(pulled));
            did.insert("log".to_string(), json!(said));
        }
        // the passphrase for keys and the keys saved on this device
        "key" => {
            let keys = Keys::new(&home, &device.id);
            let action = args.next().ok_or("an action is missing")?;
            let rest: Vec<String> = args.collect();
            let word = |i: usize| rest.get(i).map(String::as_str);
            let done = match action.as_str() {
                "set" => keys.set_passphrase(&mut client, word(0).unwrap_or(""), word(1)).await,
                "restore" => keys.restore(&mut client, word(0).and_then(|v| v.parse().ok()).unwrap_or(0)).await,
                "ask" => keys.ask(&client).await,
                "approve" => keys.approve(&mut client, word(0).unwrap_or(""), None).await,
                "overview" => keys.overview(&client).await,
                // what is saved here, as digests: never a key itself
                "saved" => {
                    let sum = |text: &str| {
                        digest(&SHA256, text.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>()
                    };
                    let aws = saved::aws(&home)
                        .map(|(id, secret, region)| format!("{id} {secret} {}", region.unwrap_or_default()));
                    Ok(json!({"google": sum(&saved::google_key(&home)), "aws": sum(&aws.unwrap_or_default()),
                              "describe": sum(&saved::describe_key(&home).1), "version": keys.local_version()}))
                }
                _ => return Err("set, restore, ask, approve, overview or saved".to_string()),
            };
            match done {
                Ok(v) => did.insert("did".to_string(), v),
                Err(e) if e.refused() => {
                    did.insert("refused".to_string(), json!({"text": e.text(), "wrong": matches!(e, Fault::Wrong(_))}))
                }
                Err(e) => return Err(e.text().to_string()),
            };
        }
        _ => return Err("push, take, step or key".to_string()),
    }
    did.insert("ms".to_string(), json!(began.elapsed().as_secs_f64() * 1000.0));
    Ok(Value::Object(did))
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
    match rt.block_on(run()) {
        Ok(did) => println!("{did}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
