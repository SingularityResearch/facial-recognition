//! The store on its own: things done to a copy of a library, for a check that compares what two builds
//! saved:
//!
//!     store_try <a data folder>      (one JSON object a line on standard input)
//!
//! {"op": "load" | "save" | "save_all" | "forget" | "restore" | "put" | "claim", ...}; one line of JSON is
//! printed for each: {"ok": <the answer>} or {"error": <why>}.

use std::{collections::HashMap, io::BufRead, path::PathBuf};

use face_library::store::{Forgotten, Store};
use serde_json::{Value, json};

fn floats(v: &Value) -> Vec<f32> {
    v.as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).filter_map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()).collect()
}

fn main() {
    let data = PathBuf::from(std::env::args().nth(1).expect("a data folder"));
    let mut store: Option<Store> = None;
    let mut gone: HashMap<usize, Forgotten> = HashMap::new();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let text = |k: &str| ask[k].as_str().unwrap_or("");
        let at = || ask["i"].as_u64().unwrap_or(0) as usize;
        let answer: Result<Value, String> = match (text("op"), store.as_mut()) {
            ("load", _) => Store::load(&data, text("model"), ask["dim"].as_u64().unwrap_or(0) as usize).map(|s| {
                let n = s.len();
                let seen = json!({"n": n, "dim": s.dim, "model": s.model, "needs_ids": s.needs_ids(),
                                  "printless": (0..n).filter(|&i| s.printless(i)).count(),
                                  "missing_thumbs": s.missing_thumbs(), "photos": s.photos.len(),
                                  "with_picture": s.thumbs.iter().filter(|t| !t.is_empty()).count()});
                store = Some(s);
                seen
            }),
            (_, None) => Err("load first".to_string()),
            ("save", Some(s)) => s.save().map(|_| json!(true)),
            ("save_all", Some(s)) => {
                s.thumbs_changed = true;
                s.save().map(|_| json!(true))
            }
            ("forget", Some(s)) => {
                gone.insert(at(), s.forget(at()));
                Ok(json!(s.printless(at())))
            }
            ("restore", Some(s)) => match gone.remove(&at()) {
                Some(saved) => {
                    s.restore(at(), saved);
                    Ok(json!(!s.printless(at())))
                }
                None => Err("that face wasn't forgotten here".to_string()),
            },
            ("put", Some(s)) => {
                let emb = if ask["emb"] == "zeros" { vec![0.0; s.dim] } else { floats(&ask["emb"]) };
                let quality = ask["quality"].as_f64().map_or(f32::NAN, |q| q as f32);
                s.put(
                    text("fid"),
                    text("photo"),
                    text("file"),
                    &floats(&ask["box"]),
                    &emb,
                    &hex(text("picture")),
                    quality,
                )
                .map(|i| json!(i))
            }
            ("claim", Some(s)) => Ok(json!(s.claim_fid(text("photo"), &floats(&ask["row"]), ask["frame"].as_i64()))),
            (other, _) => Err(format!("unknown op {other}")),
        };
        println!("{}", answer.map_or_else(|e| json!({"error": e}), |v| json!({"ok": v})));
    }
}
