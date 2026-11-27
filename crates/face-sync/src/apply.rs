//! Rows from the database taken into this device's library (`check` and
//! `apply`). Each row is looked at first: the database's columns have types, but its JSON and its
//! lists hold whatever was written, by a faulty version or by whoever has the database's password, and one
//! row that can't be used must never stop the others. Only what would break this device is refused.

// What another device or the network sends is read here, and a panic ends the whole process (`panic = "abort"`):
// clippy shows each unwrap, index and plain sum of this module, to be looked at (not in its tests).
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects))]

use std::collections::HashMap;

use face_files::{filejson, paths::atomic_write};
use face_library::{
    labels::{LabelDb, NOT_NAMES},
    photofacts::Fact,
};
use serde_json::{Map, Value, json};

use crate::{
    digest::{Part, of},
    local,
    rows::{MAX_PICTURE, PHOTO_FIELDS, Picture, Row, short},
};

type Result<T> = std::result::Result<T, String>;

fn nothing(v: Option<&Value>) -> bool {
    matches!(v, None | Some(Value::Null))
}

/// Why a row from the database can't be used on this device as it is, if it can't.
pub fn check(db: &LabelDb, tbl: &str, key: &str, row: &Row) -> Result<()> {
    crate::server::key_of(tbl)?;
    let need = |ok: bool, what: &str| if ok { Ok(()) } else { Err(what.to_string()) };
    need(!key.is_empty(), "no key")?;
    match (tbl, row) {
        ("labels", Row::Plain(r)) => {
            let not = r.get("not");
            need(
                nothing(not) || not.and_then(Value::as_array).is_some_and(|a| a.iter().all(Value::is_string)),
                "names it is not: not a list of names",
            )?;
            need(!r.get("n").and_then(Value::as_str).is_some_and(|n| NOT_NAMES.contains(&n)), "a name no page can keep")
        }
        ("web_suggestions", Row::Plain(r)) => need(r.is_object(), "not an object"),
        ("photos", Row::Plain(r)) => {
            need(nothing(r.get("extra")) || r["extra"].is_object(), "extra: not an object")?;
            let f = r.get("facts");
            let list_or_nothing = |v: Option<&Value>, list: bool| {
                nothing(v) || v.is_some_and(|v| if list { v.is_array() } else { v.is_object() })
            };
            need(
                nothing(f)
                    || (f.is_some_and(Value::is_object)
                        && list_or_nothing(f.and_then(|f| f.get("tags")), true)
                        && list_or_nothing(f.and_then(|f| f.get("faces")), false)),
                "facts: not as photo facts are",
            )?;
            need(nothing(r.get("size")) || r["size"].as_f64().is_some_and(|v| v.is_finite() && v >= 0.0), "size")
        }
        ("faces", Row::Face(f)) => {
            need(!f.photo.is_empty(), "no photo")?;
            need(f.place.len() >= 5 && f.place[..5].iter().all(|v| v.is_finite()), "box: not five numbers")?;
            need(f.frame.is_none_or(|k| k >= 0), "frame")?;
            need(
                match &f.picture {
                    Picture::Jpeg(b) => b.starts_with(b"\xff\xd8") && b.len() <= MAX_PICTURE,
                    _ => true,
                },
                "picture: not a small JPEG",
            )?;
            need(
                f.model != db.store.model || f.print.as_ref().is_none_or(|p| p.len() == db.store.dim),
                "print: wrong length",
            )
        }
        _ => Err("not a row of that table".to_string()),
    }
}

/// A row's value as a whole number or one with decimals, as it came.
fn as_given(v: &Value) -> Option<(f64, bool)> {
    v.as_f64().map(|t| (t, v.is_i64() || v.is_u64()))
}

/// Take rows into the library: none for a key means the database has no such row any more. The digest each
/// of those rows has here afterwards (none: it isn't here), which is what the device then remembers of them.
/// `web`: the Web search answers, written to their file when they change. `log` gets a line worth saying.
pub fn apply(
    db: &mut LabelDb,
    web: &mut Map<String, Value>,
    tbl: &str,
    rows: Vec<(String, Option<Row>)>,
    log: &mut dyn FnMut(String),
) -> Result<HashMap<String, Option<String>>> {
    crate::server::key_of(tbl)?;
    let files = |e: std::io::Error| format!("the library couldn't be saved: {e}");
    let keys: Vec<String> = rows.iter().map(|(k, _)| k.clone()).collect();
    match tbl {
        "labels" => {
            for (fid, row) in rows {
                match row {
                    Some(Row::Plain(r)) => {
                        let kept: Map<String, Value> = (["n", "by", "s", "ok", "not"].iter())
                            .filter_map(|k| r.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone())))
                            .collect();
                        db.labels.insert(fid, Value::Object(kept));
                    }
                    _ => {
                        db.labels.shift_remove(&fid);
                    }
                }
            }
            db.labels_changed().map_err(|e| e.to_string())?;
            Ok(keys.into_iter().map(|k| (k.clone(), db.labels.get(&k).map(|v| of(&[Part::Json(v.clone())])))).collect())
        }
        "web_suggestions" => {
            for (fid, row) in rows {
                match row {
                    Some(Row::Plain(r)) => {
                        web.insert(fid, r);
                    }
                    _ => {
                        web.shift_remove(&fid);
                    }
                }
            }
            atomic_write(&db.data().join("web_suggestions.json"), filejson::rows_of(web, true).as_bytes())
                .map_err(files)?;
            Ok(keys.into_iter().map(|k| (k.clone(), web.get(&k).map(|v| of(&[Part::Json(v.clone())])))).collect())
        }
        "photos" => {
            let mut facts_new = Vec::new();
            for (hash, row) in rows {
                // photos are never deleted by another device
                let Some(Row::Plain(r)) = row else { continue };
                let rec = db.store.photos.entry(hash.clone()).or_insert_with(|| json!({}));
                let Some(rec) = rec.as_object_mut() else { continue };
                for k in ["name", "size", "fp", "faces", "dup_of", "video", "fps"] {
                    match r.get(k).filter(|v| !v.is_null()) {
                        Some(v) => {
                            rec.insert(k.to_string(), v.clone());
                        }
                        None if matches!(k, "dup_of" | "video" | "fps") => {
                            rec.shift_remove(k);
                        }
                        None => {}
                    }
                }
                // never this device's own fields (where the file is)
                for (k, v) in r.get("extra").and_then(Value::as_object).into_iter().flatten() {
                    if !PHOTO_FIELDS.contains(&k.as_str()) {
                        rec.insert(k.clone(), v.clone());
                    }
                }
                if r.get("maybe_read").and_then(Value::as_bool) == Some(true) && !rec.contains_key("maybe") {
                    rec.insert("maybe".to_string(), json!({}));
                }
                // what the photo says about itself is taken only for a photo this device hasn't read
                let read_here = db.facts.facts.get(&hash).is_some_and(|f| f.key.is_some());
                if let Some(f) = r.get("facts").filter(|f| f.as_object().is_some_and(|o| !o.is_empty()) && !read_here) {
                    facts_new.push((hash, f.clone()));
                }
            }
            if !facts_new.is_empty() {
                for (hash, f) in facts_new {
                    let (t, t_whole) =
                        f.get("t").and_then(as_given).map_or((None, false), |(t, whole)| (Some(t), whole));
                    let tags = (f.get("tags").and_then(Value::as_array).into_iter().flatten())
                        .filter_map(|t| t.as_str().map(str::to_string))
                        .collect();
                    let faces = (f.get("faces").and_then(Value::as_object).into_iter().flatten())
                        .filter_map(|(fid, name)| Some((db.store.index_of(fid)?, name.as_str()?.to_string())))
                        .collect();
                    // not this device's own reading: the photo is read again if it is here
                    db.facts.facts.insert(hash, Fact { t, t_whole, tags, faces, key: Some("synced".to_string()) });
                }
                db.facts.save(&db.store).map_err(files)?;
            }
            db.save_store().map_err(|e| e.to_string())?;
            let now = local::digests_of(db, web, "photos", &keys);
            Ok(keys.into_iter().map(|k| (k.clone(), now.get(&k).cloned())).collect())
        }
        _ => {
            let (dim, model) = (db.store.dim, db.store.model.clone());
            let mut skipped = 0;
            for (fid, row) in rows {
                let Some(Row::Face(f)) = row else {
                    // gone in the database: it stays where it is, like an Ignored face
                    if let Some(i) = db.store.index_of(&fid) {
                        db.store.forget(i);
                    }
                    continue;
                };
                if f.model != model {
                    skipped += 1;
                    continue;
                }
                let print = f.print.unwrap_or_else(|| vec![0.0; dim]);
                let picture = match f.picture {
                    Picture::Jpeg(b) => b,
                    // the database has this device's own picture
                    Picture::Same => db.store.index_of(&fid).map(|i| db.store.thumbs[i].clone()).unwrap_or_default(),
                    Picture::None => Vec::new(),
                };
                db.store.put(&fid, &f.photo, &f.file, &f.place, &print, &picture, f.quality.unwrap_or(f32::NAN))?;
                let rec = db.store.photos.entry(f.photo.clone()).or_insert_with(|| json!({}));
                let Some(rec) = rec.as_object_mut() else { continue };
                let mut of_face = |what: &str, v: Value| {
                    if let Some(m) = rec.entry(what).or_insert_with(|| json!({})).as_object_mut() {
                        m.insert(fid.clone(), v);
                    }
                };
                if let Some(score) = f.possible {
                    of_face("maybe", json!(short(score)));
                }
                if let Some(frame) = f.frame {
                    of_face("at", json!(frame));
                }
            }
            if skipped > 0 {
                log(format!("Shared database: {skipped} faces have prints from another recognition model; not taken."));
            }
            db.prints_changed();
            db.save_store().map_err(|e| e.to_string())?;
            let now = local::digests_of(db, web, "faces", &keys);
            Ok(keys.into_iter().map(|k| (k.clone(), now.get(&k).cloned())).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use face_core::faces::REC_MODELS;

    use super::*;
    use crate::rows::FaceRow;

    fn face(photo: &str, print: Option<Vec<f32>>, picture: Picture, possible: Option<f32>) -> Row {
        Row::Face(Box::new(FaceRow {
            photo: photo.to_string(),
            file: "a.jpg".to_string(),
            place: vec![10.0, 20.0, 30.0, 40.0, 0.9],
            quality: Some(0.75),
            model: REC_MODELS[0].name.to_string(),
            print,
            picture,
            possible,
            frame: None,
        }))
    }

    #[test]
    fn rows_from_the_database_are_taken_into_an_empty_library() {
        let dir = std::env::temp_dir().join(format!("face-sync-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut db = LabelDb::open(&dir.join("data"), &dir.join("home"), REC_MODELS[0], false, &mut |_| {}).unwrap();
        let mut web = Map::new();
        let mut said = Vec::new();
        let dim = db.store.dim;
        // a photo: what is another device's own (where its file is) is never taken, wherever the row has it
        let photo = json!({"name": "a.jpg", "size": 10, "fp": "ab12", "faces": 2, "dup_of": null, "video": null, "fps": null,
                           "maybe_read": true, "extra": {"note": "x", "path": "C:/another device's file", "mtime": 5},
                           "facts": {"t": 1_700_000_000, "tags": ["Ann"], "faces": {}}});
        let kept = apply(&mut db, &mut web, "photos", vec![("h1".to_string(), Some(Row::Plain(photo)))], &mut |l| {
            said.push(l)
        })
        .unwrap();
        assert_eq!(
            db.store.photos["h1"],
            json!({"name": "a.jpg", "size": 10, "fp": "ab12", "faces": 2, "note": "x", "maybe": {}})
        );
        let facts = &db.facts.facts["h1"];
        assert_eq!(
            (facts.t, facts.t_whole, facts.tags.as_slice(), facts.key.as_deref()),
            (Some(1.7e9), true, &["Ann".to_string()][..], Some("synced"))
        );
        assert_eq!(kept["h1"], local::digests_of(&db, &web, "photos", &["h1".to_string()]).get("h1").cloned());
        // two faces: one with a print and a picture, one whose print was deleted somewhere; and one from
        // another recognition model, which is passed over and said
        let mut print = vec![0.0f32; dim];
        print[3] = 1.0;
        let mut other = face("h1", Some(vec![1.0; 128]), Picture::None, None);
        if let Row::Face(f) = &mut other {
            f.model = "another model".to_string();
        }
        let rows = vec![
            (
                "f1".to_string(),
                Some(face("h1", Some(print.clone()), Picture::Jpeg(b"\xff\xd8picture".to_vec()), Some(0.512))),
            ),
            ("f2".to_string(), Some(face("h1", None, Picture::None, None))),
            ("f3".to_string(), Some(other)),
        ];
        let kept = apply(&mut db, &mut web, "faces", rows, &mut |l| said.push(l)).unwrap();
        assert_eq!((db.store.len(), db.store.printless(0), db.store.printless(1)), (2, false, true));
        assert_eq!(
            (db.store.print(0), db.store.thumbs[0].as_slice(), db.store.boxes[0]),
            (&print[..], &b"\xff\xd8picture"[..], [10.0, 20.0, 30.0, 40.0, 0.9])
        );
        // a possible face's score is the number its short spelling says, as every device has read it
        assert_eq!(db.store.photos["h1"]["maybe"], json!({"f1": 0.512}));
        assert_eq!((kept["f1"].is_some(), kept["f2"].is_some(), &kept["f3"]), (true, true, &None));
        assert!(said.iter().any(|l| l.contains("1 faces have prints from another recognition model")));
        // a label: only what it says is kept; the people are made anew from it
        let label = json!({"n": "Ann", "by": "me", "s": null, "ok": null, "not": null});
        let kept =
            apply(&mut db, &mut web, "labels", vec![("f1".to_string(), Some(Row::Plain(label)))], &mut |_| ()).unwrap();
        assert_eq!(db.labels["f1"], json!({"n": "Ann", "by": "me"}));
        assert_eq!(kept["f1"], Some(of(&[Part::Json(json!({"n": "Ann", "by": "me"}))])));
        assert!(dir.join("data").join("labels.json").is_file() && dir.join("data").join("people.npz").is_file());
        // a Web search answer
        apply(
            &mut db,
            &mut web,
            "web_suggestions",
            vec![("f1".to_string(), Some(Row::Plain(json!({"when": "2026-11-12T10:20:30Z"}))))],
            &mut |_| (),
        )
        .unwrap();
        assert!(web.contains_key("f1") && dir.join("data").join("web_suggestions.json").is_file());
        // gone in the database: a label and an answer go, a face stays where it is without its print, a photo stays
        for tbl in ["labels", "web_suggestions", "faces"] {
            let kept = apply(&mut db, &mut web, tbl, vec![("f1".to_string(), None)], &mut |_| ()).unwrap();
            assert_eq!(kept["f1"].is_some(), tbl == "faces", "{tbl}");
        }
        assert!(!db.labels.contains_key("f1") && web.is_empty() && db.store.len() == 2 && db.store.printless(0));
        apply(&mut db, &mut web, "photos", vec![("h1".to_string(), None)], &mut |_| ()).unwrap();
        assert!(db.store.photos.contains_key("h1"));
        // read again from its files, the library is what was taken
        drop(db);
        let db = LabelDb::open(&dir.join("data"), &dir.join("home"), REC_MODELS[0], false, &mut |_| {}).unwrap();
        assert_eq!(
            (db.store.len(), db.store.fid.clone(), db.labels.len()),
            (2, vec!["f1".to_string(), "f2".to_string()], 0)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_row_this_device_cannot_use_is_refused_with_the_reason() {
        let dir = std::env::temp_dir().join(format!("face-sync-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = LabelDb::open(&dir.join("data"), &dir.join("home"), REC_MODELS[0], false, &mut |_| {}).unwrap();
        let plain = |v: Value| Row::Plain(v);
        let why = |tbl: &str, key: &str, row: Row| check(&db, tbl, key, &row).err();
        assert_eq!(why("labels", "f1", plain(json!({"n": "Ann", "not": ["Bob"]}))), None);
        assert_eq!(why("labels", "", plain(json!({"n": "Ann"}))).as_deref(), Some("no key"));
        assert_eq!(
            why("labels", "f1", plain(json!({"not": "Bob"}))).as_deref(),
            Some("names it is not: not a list of names")
        );
        assert_eq!(why("labels", "f1", plain(json!({"n": "__proto__"}))).as_deref(), Some("a name no page can keep"));
        assert_eq!(why("web_suggestions", "f1", plain(json!([1]))).as_deref(), Some("not an object"));
        assert_eq!(why("photos", "h1", plain(json!({"extra": {}, "facts": null, "size": 5}))), None);
        assert_eq!(why("photos", "h1", plain(json!({"extra": [1]}))).as_deref(), Some("extra: not an object"));
        assert_eq!(
            why("photos", "h1", plain(json!({"facts": {"tags": "Ann"}}))).as_deref(),
            Some("facts: not as photo facts are")
        );
        assert_eq!(why("photos", "h1", plain(json!({"size": -1}))).as_deref(), Some("size"));
        let good = || face("h1", Some(vec![0.5; db.store.dim]), Picture::Jpeg(b"\xff\xd8".to_vec()), None);
        assert_eq!(why("faces", "f1", good()), None);
        let changed = |change: &dyn Fn(&mut FaceRow)| {
            let mut row = good();
            if let Row::Face(f) = &mut row {
                change(f);
            }
            why("faces", "f1", row)
        };
        // a box of three numbers was once saved with the rest and then stopped every later save
        assert_eq!(changed(&|f| f.place.truncate(3)).as_deref(), Some("box: not five numbers"));
        assert_eq!(changed(&|f| f.photo.clear()).as_deref(), Some("no photo"));
        assert_eq!(changed(&|f| f.frame = Some(-1)).as_deref(), Some("frame"));
        assert_eq!(
            changed(&|f| f.picture = Picture::Jpeg(b"GIF89a".to_vec())).as_deref(),
            Some("picture: not a small JPEG")
        );
        assert_eq!(changed(&|f| f.print = Some(vec![0.5; 7])).as_deref(), Some("print: wrong length"));
        // a print of another model's length is that model's affair: the face is passed over when taken
        assert_eq!(changed(&|f| (f.model, f.print) = ("another model".to_string(), Some(vec![0.5; 7]))), None);
        assert!(why("faces", "f1", plain(json!({}))).is_some() && why("somewhere", "k", plain(json!({}))).is_some());
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
