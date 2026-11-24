//! The labeling engine on its own: a list of things to do to a library, each answered with a line of JSON,
//! for a check that compares answers and files between two builds or two copies of a library:
//!
//!     labels_try <a data folder> <a private folder>      (one JSON object a line on standard input)
//!
//! {"op": "open" | "gallery" | "triage" | "suggested" | "unknown" | "person" | "auto_review" | "checks" |
//! "summary" | "stacks" | "act" | "rename" | "undo" | "autoname" | "near_duplicate" | "same_faces", ...}; one
//! line of JSON is printed for each: {"ok": <the answer>, "ms": <how long the engine took>} or {"error": <why>}.

use std::{io::BufRead, path::PathBuf, time::Instant};

use face_core::faces::REC_MODELS;
use face_library::labels::{Fault, LabelDb};
use serde_json::{Value, json};

fn faces(v: &Value) -> Vec<usize> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_u64().map(|i| i as usize)).collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let data = PathBuf::from(args.next().expect("a data folder"));
    let home = PathBuf::from(args.next().expect("a private folder"));
    let mut db: Option<LabelDb> = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let text = |k: &str| ask[k].as_str();
        let flag = |k: &str| ask[k].as_bool().unwrap_or(false);
        let started = Instant::now();
        let said = |r: Result<Value, Fault>| r.map_err(|e| e.to_string());
        let answer: Result<Value, String> = match (text("op").unwrap_or(""), db.as_mut()) {
            ("open", _) => {
                let mut notes = Vec::new();
                said(LabelDb::open(&data, &home, REC_MODELS[0], flag("backup"), &mut |n| notes.push(n)).map(
                    |mut opened| {
                        if let Some(dir) = text("webcam") {
                            opened.webcam = PathBuf::from(dir);
                            opened.changed();
                        }
                        let seen = json!({"n": opened.n(), "labels": opened.labels.len(), "notes": notes.len()});
                        db = Some(opened);
                        seen
                    },
                ))
            }
            (_, None) => Err("open first".to_string()),
            ("gallery", Some(db)) => {
                let g = db.gallery();
                let people: Vec<Value> = g.people.iter().map(|(name, faces)| json!([name, faces])).collect();
                let seen: Vec<&str> = g.seen.iter().map(|j| g.people[*j].0.as_str()).collect();
                Ok(json!({"people": people, "seen": seen}))
            }
            ("triage", Some(db)) => {
                let (tri, g) = (db.triage(), db.gallery());
                Ok(Value::Array(
                    tri.iter()
                        .map(|t| json!([t.i, t.name.map(|j| g.people[j].0.as_str()), t.s as f64, t.s2 as f64]))
                        .collect(),
                ))
            }
            ("score", Some(db)) => {
                let (names, rows) = db.score(&faces(&ask["idx"]));
                let rows: Vec<Vec<f64>> = rows.iter().map(|r| r.iter().map(|v| *v as f64).collect()).collect();
                Ok(json!({"names": names, "rows": rows}))
            }
            ("suggested", Some(db)) => Ok(db.suggested(flag("show_low"))),
            ("unknown", Some(db)) => Ok(db.unknown(flag("show_low"), flag("regroup"))),
            ("person", Some(db)) => Ok(db.person(text("name").unwrap_or(""))),
            ("auto_review", Some(db)) => Ok(db.auto_review()),
            ("checks", Some(db)) => Ok((*db.checks()).clone()),
            ("summary", Some(db)) => {
                db.learn_faceness(); // (learned by the time a summary counts them)
                Ok(db.summary())
            }
            ("learn", Some(db)) => {
                db.learn_faceness();
                let (pos, neg) = db.face_sets();
                let mut scores: Vec<(usize, f64)> =
                    (0..db.n()).map(|i| (i, db.face_score(i))).filter(|(_, s)| *s != 1.0).collect();
                scores.sort_by_key(|(i, _)| *i);
                Ok(json!({"cut": db.face_cut(), "scores": scores, "real": pos.len(), "not": neg.len()}))
            }
            ("possible", Some(db)) => {
                db.learn_faceness();
                Ok(db.possible())
            }
            ("stacks", Some(db)) => {
                let mut pairs: Vec<(usize, usize)> = db.stacks().iter().map(|(a, b)| (*a, *b)).collect();
                pairs.sort_unstable();
                Ok(json!(pairs))
            }
            ("act", Some(db)) => said(
                db.act(text("action").unwrap_or(""), &faces(&ask["idx"]), text("name"), text("text")).map(|n| json!(n)),
            ),
            ("rename", Some(db)) => {
                said(db.rename(text("old").unwrap_or(""), text("new").unwrap_or("")).map(|n| json!(n)))
            }
            ("undo", Some(db)) => said(db.undo().map(|t| json!(t))),
            ("autoname", Some(db)) => {
                let only = ask["idx"].is_array().then(|| faces(&ask["idx"]));
                said(db.autoname(only.as_deref()).map(|n| json!(n)))
            }
            ("near_duplicate", Some(db)) => Ok(json!(db.near_duplicate(text("fp").unwrap_or("")))),
            ("same_faces", Some(db)) => {
                let embs: Vec<f32> =
                    ask["embs"].as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
                let sim = ask["sim"].as_f64().unwrap_or(0.90) as f32;
                Ok(json!(db.same_faces(text("photo").unwrap_or(""), &embs, sim)))
            }
            ("prints", Some(db)) => {
                // the prints of some faces, for a question the other program is asked with the same numbers
                let all: Vec<f64> =
                    faces(&ask["idx"]).iter().flat_map(|i| db.store.print(*i).iter().map(|v| *v as f64)).collect();
                Ok(json!(all))
            }
            (other, _) => Err(format!("unknown op {other}")),
        };
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        println!("{}", answer.map_or_else(|e| json!({"error": e}), |v| json!({"ok": v, "ms": ms})));
    }
}
