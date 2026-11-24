//! What photos say about themselves, as lines of JSON, for a check of the facts a library keeps:
//!
//!     facts_try [<a data folder> <a private folder>]      (one JSON object a line on standard input)
//!
//! Without a library: {"op": "header" | "photo_date" | "file_hash", "path": ...}, {"op": "fingerprint", "w",
//! "h", "px": <the picture's bytes as hex>}. With one ("open" first): "facts_all" (every photo's key and
//! facts, read now), "save_facts", "read_facts", "tagged", "hints", "occasion", "photos_with", "live", "relocate". One line
//! of JSON is printed for each: {"ok": <the answer>, "ms": <how long it took>} or {"error": <why>}.

use std::{
    io::BufRead,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use face_core::{faces::REC_MODELS, image::Bgr};
use face_library::{exif, labels::LabelDb, photofacts::Fact, photofiles, relocate};
use serde_json::{Value, json};

fn fact(f: &Fact) -> Value {
    json!({"t": f.t_value(), "tags": f.tags, "faces": f.faces})
}

fn main() {
    let mut args = std::env::args().skip(1);
    let folders = args.next().zip(args.next()).map(|(data, home)| (PathBuf::from(data), PathBuf::from(home)));
    let mut db: Option<LabelDb> = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let text = |k: &str| ask[k].as_str().unwrap_or("");
        let started = Instant::now();
        let answer: Result<Value, String> = match (text("op"), db.as_mut()) {
            ("header", _) => exif::header_of(Path::new(text("path")))
                .map(|h| json!({"size": [h.width, h.height], "orientation": h.orientation, "taken": h.taken})),
            ("photo_date", _) => Ok(json!(photofiles::photo_date(Path::new(text("path"))))),
            ("file_hash", _) => {
                photofiles::file_hash(Path::new(text("path"))).map(|h| json!(h)).map_err(|e| e.to_string())
            }
            ("fingerprint", _) => {
                let hex = text("px");
                let px: Vec<u8> =
                    (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect();
                let (w, h) = (ask["w"].as_u64().unwrap_or(0) as usize, ask["h"].as_u64().unwrap_or(0) as usize);
                Bgr::from_px(w, h, px).map(|b| json!(photofiles::fingerprint(&b))).map_err(|e| e.0)
            }
            ("open", _) => match &folders {
                None => Err("no library was named".to_string()),
                Some((data, home)) => LabelDb::open(data, home, REC_MODELS[0], false, &mut |_| {})
                    .map_err(|e| e.to_string())
                    .map(|mut opened| {
                        if !text("webcam").is_empty() {
                            opened.webcam = PathBuf::from(text("webcam"));
                            opened.changed();
                        }
                        let seen = json!({"n": opened.n(), "facts": opened.facts.facts.len()});
                        db = Some(opened);
                        seen
                    }),
            },
            (_, None) => Err("open first".to_string()),
            ("facts_all", Some(db)) => {
                // every photo's key and facts read now, whatever is kept: eight at a time
                let jobs = db.facts_jobs(None);
                let listings = db.facts.listings.clone();
                let (next, out) = (AtomicUsize::new(0), Mutex::new(vec![Value::Null; jobs.len()]));
                std::thread::scope(|s| {
                    for _ in 0..8 {
                        s.spawn(|| {
                            loop {
                                let k = next.fetch_add(1, Ordering::Relaxed);
                                let Some(job) = jobs.get(k) else { break };
                                let row =
                                    json!([job.h, listings.key(job).ok(), listings.read(job).ok().as_ref().map(fact)]);
                                out.lock().unwrap()[k] = row;
                            }
                        });
                    }
                });
                Ok(Value::Array(out.into_inner().unwrap()))
            }
            ("save_facts", Some(db)) => db.facts.save(&db.store).map(|_| json!(true)).map_err(|e| e.to_string()),
            ("read_facts", Some(db)) => db
                .read_facts(None)
                .map_err(|e| e.to_string())
                .map(|read| json!({"read": read, "total": db.facts.status.total})),
            ("tagged", Some(db)) => Ok(db.tagged(ask["show_low"].as_bool().unwrap_or(false))),
            ("hints", Some(db)) => Ok(db.hints(ask["i"].as_u64().unwrap_or(0) as usize)),
            ("occasion", Some(db)) => Ok(db.occasion(text("h"))),
            ("photos_with", Some(db)) => {
                let names: Vec<&str> =
                    ask["names"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                Ok(db.photos_with(&names))
            }
            ("live", Some(db)) => Ok(db.live_verdict(ask["i"].as_u64().unwrap_or(0) as usize)),
            ("relocate", Some(db)) => {
                let data = folders.as_ref().map(|f| f.0.clone()).unwrap_or_default();
                let added: Vec<String> = ask["folders"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect();
                let mut lines = 0;
                relocate::relocate(&mut db.store, &data, &added, &mut |_| lines += 1).map(|mut out| {
                    out["lines"] = json!(lines);
                    out
                })
            }
            (other, _) => Err(format!("unknown op {other}")),
        };
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        println!("{}", answer.map_or_else(|e| json!({"error": e}), |v| json!({"ok": v, "ms": ms})));
    }
}
