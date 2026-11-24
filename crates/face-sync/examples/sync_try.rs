//! The sync's view of a library, as lines of JSON, for a check of its rows and digests:
//!
//!     sync_try <a data folder> <a private folder> <a file to write to> [<a sync.sqlite> | <a face's ID>]
//!
//! Writes {"digests": {table: {key: digest}}, "ms": how long the digests took} as JSON; with a device's
//! `sync.sqlite` also "pending": {table: {"changed": [keys], "gone": [keys]}}, what a push would send, and
//! "joined": whether it has joined a database. With a face's ID it prints the digest of each part of that
//! face's row instead. The library is only read.

use std::{path::PathBuf, time::Instant};

use face_core::faces::REC_MODELS;
use face_library::labels::LabelDb;
use serde_json::{Map, Value, json};

fn main() {
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let (data, home, out) =
        (args.next().expect("a data folder"), args.next().expect("a private folder"), args.next().expect("a file"));
    let db = match LabelDb::open(&data, &home, REC_MODELS[0], false, &mut |_| {}) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let web: Map<String, Value> = std::fs::read(data.join("web_suggestions.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    // a face's digest taken apart, for finding where two programs differ: the digest of each of its parts
    let more = args.next();
    let state = more.clone().filter(|m| m.extension().is_some_and(|e| e == "sqlite"));
    if let Some(fid) = more.filter(|_| state.is_none()) {
        let fid = fid.to_string_lossy().to_string();
        let i = db.store.fid.iter().position(|f| *f == fid).expect("the face");
        let parts: Vec<String> =
            face_sync::local::face_parts(&db.store, i).into_iter().map(|p| face_sync::digest::of(&[p])).collect();
        println!("{}", json!(parts));
        return;
    }
    let began = Instant::now();
    let digests = face_sync::local::digests(&db, &web);
    let ms = began.elapsed().as_secs_f64() * 1000.0;
    let mut said = json!({"digests": digests, "ms": ms});
    if let Some(path) = state {
        let state = face_sync::state::State::open(&path).expect("the state");
        let mut pending = Map::new();
        for tbl in face_sync::local::TABLES {
            let (changed, gone) = face_sync::state::pending(&digests[tbl], &state.digests(tbl).expect("its digests"));
            pending.insert(tbl.to_string(), json!({"changed": changed, "gone": gone}));
        }
        said["pending"] = Value::Object(pending);
        said["joined"] = json!(state.get("instance").expect("its memory").is_some());
    }
    std::fs::write(out, said.to_string()).expect("the file written");
}
