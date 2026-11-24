//! Faces' pictures made again from their photos, to compare with the ones a library keeps:
//!
//!     crop_try <root: the folder with models/ and data/> <a private folder>
//!     ({"i": <a face's place in the library>, "dump": <a file to write the picture's bytes into, if wanted>} a line)
//!
//! For each a line of JSON: {"ok": {"w", "h", "sha1" of its bytes}}, {"ok": null} where the photo isn't here or
//! the face isn't found in it again, or {"error": <why>}. The library is only read.

use std::{
    io::BufRead,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use face_core::faces::REC_MODELS;
use face_import::importer::Importer;
use face_library::{labels::LabelDb, photofiles::sha1_hex};
use serde_json::{Value, json};

fn main() {
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let (root, home) = (args.next().expect("a root folder"), args.next().expect("a private folder"));
    let db = match LabelDb::open(&root.join("data"), &home, REC_MODELS[0], false, &mut |_| {}) {
        Ok(db) => Arc::new(Mutex::new(db)),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let importer = Importer::new(db.clone(), &root, &home);
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let answer = importer.models().map(|faces| {
            let i = ask["i"].as_u64().unwrap_or(u64::MAX) as usize;
            let from = Importer::crop_source(&db.lock().unwrap().store, i);
            let crop = from.and_then(|from| Importer::crop_from(&faces, &from));
            crop.map_or(Value::Null, |c| {
                if let Some(dump) = ask["dump"].as_str() {
                    let _ = std::fs::write(dump, &c.px);
                }
                json!({"w": c.w, "h": c.h, "sha1": sha1_hex(&c.px)})
            })
        });
        println!("{}", answer.map_or_else(|e| json!({"error": e}), |v| json!({"ok": v})));
    }
}
