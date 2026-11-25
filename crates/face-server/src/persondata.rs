//! Everything the programs hold about one person: export it, or forget it.
//! The labeling tool's person view (**Their data...**) uses it through
//! `/api/person/export` and `/api/person/forget`, and the sync when another device forgot someone.
//!
//! Where a person's data can be (`data` is the library's folder, `home` the private one):
//!
//! - `data/labels.json`: the labels of their faces, and their name in other faces' "not this person" lists
//! - `data/faces.npz`, `thumbs.npz`: their faces' prints and pictures; `data/people.npz`: their prints as the
//!   tracker loads them (made again from the labels)
//! - `data/history.jsonl`: labeling changes that name them; `data/backups/`: earlier copies of the labels
//! - `data/web_suggestions.json`: saved Web search answers for their faces, or naming them
//! - `data/photo_facts.json`: their name among a photo's people tags
//! - `data/expr_eval/*.jsonl`: posed test frames and feeling reports with their name
//! - `data/expr_calibration.json`, `va_fit.json`, `webcam_eval.json`: numbers fitted or measured per person
//! - `data/sheets/`: contact sheets of unnamed faces, which may show them
//! - `home/presence/*.jsonl`: their visits; `home/alerts.json`: the tracker's alerts about them
//! - `home/expression/*.jsonl`, `home/music/*.jsonl`: their expression readings and the songs that played
//!   for them, waiting to be sent to the shared database, and there the `expression_log` and `music_log`
//!   tables; their name as this device's music setting
//! - `home/signals/*.jsonl`: rows of their signal history that the shared database couldn't take yet, and
//!   there the `signal_log` table (every signal read of them, a row a stretch of time, kept only where a
//!   device's owner switched the history on; and what the library's pictures say of them, a row a picture,
//!   where its owner had that worked out: `libhistory.rs`)
//! - `home/webcam/`: Collect faces pictures of them, and the tracker's verdicts naming them
//! - `home/descriptions.json`: photo descriptions whose text names them; `home/app.log`
//! - `home/person_signals.json`: their usual look and their normal for each signal; `home/settings.json`:
//!   their name in the tracker's alert list
//! - the shared database, when one is set up: a forget here is sent to it and to every other device, and
//!   clears what only the server holds. The forgotten name itself is the one thing kept
//!   (`home/forgotten.json`), as the instruction for devices that haven't forgotten yet.
//!
//! Forgetting keeps their faces as Ignored faces without print or picture, removes their name everywhere
//! above, and clears Undo. It can't be undone. A new place that keeps anything about a person goes into
//! this list, `export` and `forget`.

use std::{
    collections::BTreeSet,
    io::{Seek, Write},
    path::{Path, PathBuf},
};

use face_db::kept::Kept;
use face_files::{
    filejson, forgotten,
    paths::{atomic_write, utc_now, utc_stamp},
};
use face_home::{
    alerts::Inbox,
    descriptions,
    store::Store,
    waiting::{EXPRESSION, MUSIC, SIGNALS, WaitLog},
};
use face_library::{
    labels::{Fault, LabelDb},
    person::{drop_lines, files, history_names, jsonl, mentions, only, scrub},
};
use serde_json::{Map, Value, json};

use crate::label::Inner;

/// in `data`: numbers kept per person
const FITTED: [&str; 3] = ["expr_calibration.json", "va_fit.json", "webcam_eval.json"];
/// in `data`: the Web search answers by face
const WEB_SAVED: &str = "web_suggestions.json";
/// how a line of a file written as text ends on this system
const END: &str = if cfg!(windows) { "\r\n" } else { "\n" };

type Result<T> = std::result::Result<T, Fault>;

fn failed(e: impl std::fmt::Display) -> Fault {
    Fault::Files(e.to_string())
}

fn named(o: &Value, name: &str) -> bool {
    o.get("name").and_then(Value::as_str) == Some(name)
}

/// The person whose music this device's is (the owner's setting), or "".
fn music_who(store: &Store) -> String {
    let who: String = store.setting("label", "fl-musicWho").unwrap_or_default().trim().chars().take(100).collect();
    if who == "Unknown" || who.starts_with('#') { String::new() } else { who }
}

fn alert_list(store: &Store) -> Vec<String> {
    store.setting("track", "ft-alertWho").unwrap_or_default().split(',').map(|n| n.trim().to_string()).collect()
}

/// True if something outside the labels is still kept under a name: after a rename, or once their last
/// face was cleared, the person is in no label, and their alerts, visits, learned signals and readings
/// could otherwise neither be exported nor forgotten.
fn kept_elsewhere(home: &Path, name: &str) -> bool {
    let store = Store::new(home);
    store.of(name).is_ok_and(|s| !s.is_empty())
        || Inbox::new(home).about(name).is_ok_and(|a| !a.is_empty())
        || !EXPRESSION.waiting(home, Some(name)).is_empty()
        || !MUSIC.waiting(home, Some(name)).is_empty()
        || !SIGNALS.waiting(home, Some(name)).is_empty()
        || music_who(&store) == name
        || alert_list(&store).iter().any(|n| n == name)
        || files(&home.join("presence"), ".jsonl", false)
            .iter()
            .any(|f| jsonl(f).iter().any(|(_, o)| o.as_ref().is_some_and(|o| named(o, name))))
}

/// The name, if it is a person: in the labels (named on a face, or in a "not this person" list), or kept
/// elsewhere.
fn check_name(db: &LabelDb, home: &Path, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('#') {
        return Err(Fault::Asked("Give the name of a person.".to_string()));
    }
    if !db.is_person(name) && !kept_elsewhere(home, name) {
        return Err(Fault::Asked(format!("Nobody is named {name}.")));
    }
    Ok(name.to_string())
}

/// A Web search answer is about a person: it is for one of their faces, or one of its suggestions names them.
fn web_about(fids: &[String], name: &str, fid: &str, answer: &Value) -> bool {
    fids.iter().any(|f| f == fid)
        || answer.get("suggestions").and_then(Value::as_array).is_some_and(|s| s.iter().any(|s| named(s, name)))
}

/// Collect faces pictures (the .jpg, with its .json beside it) of someone's faces, or whose verdict names them.
fn snapshots(paths: &[String], home: &Path, name: &str) -> Vec<PathBuf> {
    let webcam = home.join("webcam");
    let folder = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
    let mut pictures: BTreeSet<PathBuf> = paths
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.parent().is_some_and(|dir| folder(dir) == folder(&webcam)))
        .collect();
    for verdict in files(&webcam, ".json", false) {
        let said: Option<Value> = std::fs::read(&verdict).ok().and_then(|b| serde_json::from_slice(&b).ok());
        if said
            .and_then(|s| s.get("faces").cloned())
            .and_then(|f| f.as_array().cloned())
            .is_some_and(|f| f.iter().any(|f| named(f, name)))
        {
            pictures.insert(verdict.with_extension("jpg"));
        }
    }
    pictures.into_iter().collect()
}

/// A column of a log's table, as it is read for the export.
#[derive(Clone, Copy)]
enum Col {
    Text(&'static str),
    Real(&'static str),
    Json(&'static str),
}

/// Write a person's rows of a log to a file, one JSON object a line: those in the shared database (when it
/// can be reached) and those still waiting here. (how many, why some may be missing)
fn export_log(
    home: &Path,
    log: &WaitLog,
    table: &str,
    cols: [Col; 4],
    path: &Path,
    name: &str,
) -> std::io::Result<(usize, Option<String>)> {
    let names = cols.map(|c| match c {
        Col::Text(n) | Col::Real(n) | Col::Json(n) => n,
    });
    let (mut lines, mut note, mut seen) = (Vec::new(), None, BTreeSet::new());
    if face_db::config(home).is_some() {
        // (a 32-bit number as the number its short spelling says, as every device reads these columns)
        let select: Vec<String> = cols
            .iter()
            .map(|c| match c {
                Col::Text(n) => n.to_string(),
                Col::Real(n) => format!("({n}::text)::float8"),
                Col::Json(n) => format!("{n}::text"),
            })
            .collect();
        let ask = format!(
            "SELECT device::text, floor(extract(epoch FROM at) * 1000)::bigint, to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), {}
             FROM {table} WHERE name = $1 ORDER BY at",
            select.join(", ")
        );
        let rows = Kept::new(home).ask(async |client, _| {
            let rows = client.query(&ask, &[&name]).await.map_err(|e| e.to_string())?;
            Ok(rows
                .iter()
                .map(|r| {
                    let (device, t, when): (Option<String>, i64, String) = (r.get(0), r.get(1), r.get(2));
                    let mut line = Map::new();
                    line.insert("t".to_string(), json!(t));
                    line.insert("when".to_string(), json!(when));
                    line.insert("device".to_string(), json!(device));
                    for (k, col) in cols.iter().enumerate() {
                        let value = match col {
                            Col::Text(_) => json!(r.get::<_, Option<String>>(3 + k)),
                            Col::Real(_) => json!(r.get::<_, Option<f64>>(3 + k)),
                            Col::Json(_) => r
                                .get::<_, Option<String>>(3 + k)
                                .and_then(|t| serde_json::from_str(&t).ok())
                                .unwrap_or(Value::Null),
                        };
                        line.insert(names[k].to_string(), value);
                    }
                    (device, t, Value::Object(line))
                })
                .collect::<Vec<_>>())
        });
        match rows {
            Ok(rows) => {
                for (device, t, line) in rows {
                    seen.insert((device, t));
                    lines.push(line);
                }
            }
            // can't be reached now: say so in the export
            Err(e) => {
                note = Some(format!(
                    "the shared database couldn't be read ({})",
                    e.trim().lines().next().unwrap_or("").chars().take(200).collect::<String>()
                ))
            }
        }
    }
    let me = face_db::Device::load(home, &crate::device().0).ok().map(|d| d.id);
    for row in log.waiting(home, Some(name)) {
        if !seen.contains(&(me.clone(), row.get("t").and_then(Value::as_i64).unwrap_or(i64::MIN))) {
            let mut line = Map::new();
            for k in std::iter::once("t").chain(names) {
                // (a waiting row of the signal history spells two of its columns shortly)
                let waiting = match k {
                    "seconds" if row.get("seconds").is_none() => "s",
                    "readings" => "n",
                    other => other,
                };
                line.insert(k.to_string(), row.get(waiting).cloned().unwrap_or(Value::Null));
            }
            line.insert("device".to_string(), json!(me));
            line.insert("waitingToBeSent".to_string(), json!(true));
            lines.push(Value::Object(line));
        }
    }
    std::fs::write(path, lines.iter().map(|l| filejson::dumps_utf8(l) + END).collect::<String>())?;
    Ok((lines.len(), note))
}

/// Write everything held about a person into a new folder inside `folder`: `data.json` (every place in this
/// module's list, with the face prints as base64 of 32-bit numbers), `faces/<face ID>.jpg` (their face
/// pictures), `webcam/` (their Collect faces pictures) and the logs' rows as files of their own.
/// {folder, faces, pictures}
pub(crate) fn export(inner: &Inner, data: &Path, home: &Path, name: &str, folder: &str) -> Result<Value> {
    let dest = PathBuf::from(folder.trim().trim_matches('"'));
    if !dest.is_absolute() {
        return Err(Fault::Asked("Give the full path of the folder, e.g. E:\\Documents\\Export".to_string()));
    }
    let (name, held) = {
        let db = inner.db();
        let name = check_name(&db, home, name)?;
        let held = db.held_about(&name);
        (name, held)
    };
    let name = name.as_str();
    let snaps = snapshots(&held.paths, home, name);
    let web = inner.web_saved.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let store = Store::new(home);
    let about = |file: &Path, is: &dyn Fn(&Value) -> bool| -> Vec<Value> {
        jsonl(file).into_iter().filter_map(|(_, o)| o.filter(|o| o.is_object() && is(o))).collect()
    };
    // each file's lines about them, by the file's name; a file with none is left out
    let by_file = |folder: PathBuf| -> Map<String, Value> {
        (files(&folder, ".jsonl", false).iter())
            .filter_map(|f| {
                let rows = about(f, &|o| named(o, name));
                (!rows.is_empty())
                    .then(|| (f.file_name().unwrap_or_default().to_string_lossy().into_owned(), Value::Array(rows)))
            })
            .collect()
    };
    let fitted: Map<String, Value> = (FITTED.iter())
        .filter(|f| data.join(f).exists())
        .map(|f| {
            let kept: Option<Value> = std::fs::read(data.join(f)).ok().and_then(|b| serde_json::from_slice(&b).ok());
            (f.to_string(), kept.map_or(Value::Null, |v| only(&v, name)))
        })
        .collect();
    let described = descriptions::find(home, |d| mentions(d.get("text").and_then(Value::as_str).unwrap_or(""), name))
        .map_err(failed)?;
    let mut out = Map::new();
    let mut put = |k: &str, v: Value| out.insert(k.to_string(), v);
    put("person", json!(name));
    put("exported", json!(utc_now()));
    put(
        "about",
        json!(
            "Everything the face recognizer holds about this person on this computer. Face prints are float32 numbers, base64. Their usual look and normal signals are only in the tracker page's browser storage, which this can't read."
        ),
    );
    put("faces", json!(held.faces));
    put("markedNotThem", json!(held.not_them));
    put("history", json!(about(&data.join("history.jsonl"), &|o| history_names(o, name))));
    put(
        "webSearch",
        Value::Object(
            web.iter()
                .filter(|(fid, a)| web_about(&held.fids, name, fid, a))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
    );
    put("photoTags", json!(held.tags));
    put("expressionTests", Value::Object(by_file(data.join("expr_eval"))));
    put("fitted", Value::Object(fitted));
    put("presence", Value::Object(by_file(home.join("presence"))));
    put("alerts", json!(Inbox::new(home).about(name).map_err(failed)?));
    put("descriptions", Value::Object(described));
    put(
        "webcam",
        json!(
            snaps.iter().map(|s| s.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect::<Vec<_>>()
        ),
    );
    put("personSignals", Value::Object(store.of(name).map_err(failed)?));
    put("inAlertList", json!(alert_list(&store).iter().any(|n| n == name)));

    let safe: String =
        name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 0x20 { '_' } else { c }).collect();
    let safe = safe.trim_matches([' ', '.']);
    let label = format!(
        "{} - face recognizer data {}",
        if safe.is_empty() { "person" } else { safe },
        utc_stamp(std::time::SystemTime::now())
    );
    let folder = dest.join(label);
    if folder.exists() {
        return Err(failed(format!("{} is there already", folder.display())));
    }
    std::fs::create_dir_all(folder.join("faces")).map_err(failed)?;
    for (fid, jpeg) in &held.pictures {
        let file: String =
            fid.chars().map(|c| if c.is_ascii_alphanumeric() || "_.-".contains(c) { c } else { '_' }).collect();
        std::fs::write(folder.join("faces").join(format!("{file}.jpg")), jpeg).map_err(failed)?;
    }
    if !snaps.is_empty() {
        std::fs::create_dir(folder.join("webcam")).map_err(failed)?;
        for picture in &snaps {
            for file in [picture.clone(), picture.with_extension("json")] {
                if let (true, Some(name)) = (file.exists(), file.file_name()) {
                    std::fs::copy(&file, folder.join("webcam").join(name)).map_err(failed)?;
                }
            }
        }
    }
    // the logs may be long: files of their own
    let logs = [
        (
            "expressionLog",
            "expression_log.jsonl",
            "readings",
            &EXPRESSION,
            "expression_log",
            [Col::Text("label"), Col::Real("share"), Col::Text("state"), Col::Json("data")],
        ),
        (
            "musicLog",
            "music_log.jsonl",
            "songs",
            &MUSIC,
            "music_log",
            [Col::Text("artist"), Col::Text("song"), Col::Real("seconds"), Col::Json("data")],
        ),
        (
            "signalLog",
            "signal_log.jsonl",
            "rows",
            &SIGNALS,
            "signal_log",
            [Col::Text("source"), Col::Real("seconds"), Col::Real("readings"), Col::Json("data")],
        ),
    ];
    for (key, file, counted, log, table, cols) in logs {
        let (n, note) = export_log(home, log, table, cols, &folder.join(file), name).map_err(failed)?;
        if n > 0 {
            out.insert(key.to_string(), json!({"file": file, counted: n, "note": note}));
        } else {
            let _ = std::fs::remove_file(folder.join(file));
            if note.is_some() {
                out.insert(key.to_string(), json!({counted: 0, "note": note}));
            }
        }
    }
    atomic_write(&folder.join("data.json"), filejson::dumps_indent1(&Value::Object(out)).as_bytes()).map_err(failed)?;
    Ok(json!({"folder": folder.to_string_lossy(), "faces": held.faces.len(), "pictures": held.pictures.len()}))
}

/// Drop the lines of a log that name someone. Written in place: the app keeps its log open.
fn forget_log(path: &Path, name: &str) -> usize {
    let Ok(bytes) = std::fs::read(path) else { return 0 };
    let lines: Vec<&[u8]> = bytes.split_inclusive(|b| *b == b'\n').collect();
    let keep: Vec<&[u8]> = lines.iter().filter(|l| !mentions(&String::from_utf8_lossy(l), name)).copied().collect();
    if keep.len() < lines.len() {
        let kept = keep.concat();
        let written = std::fs::OpenOptions::new().write(true).open(path).and_then(|mut f| {
            f.rewind()?;
            f.write_all(&kept)?;
            f.set_len(kept.len() as u64)
        });
        if written.is_err() {
            return 0;
        }
    }
    lines.len() - keep.len()
}

/// Forget a person everywhere in this module's list (but a page's browser storage, which the page clears
/// itself): what was removed, by place. Their faces stay as Ignored faces without print or picture. Clears
/// Undo; can't be undone. With a shared database the forget also goes to every other device, and `known` is
/// how it arrives there: the name may already be gone from this device's labels.
pub(crate) fn forget(inner: &Inner, data: &Path, home: &Path, name: &str, known: bool) -> Result<Value> {
    let mut done = Map::new();
    let (name, forgot, snaps) = {
        let mut db = inner.db();
        let name = if known { name.trim().to_string() } else { check_name(&db, home, name)? };
        if name.is_empty() || name.starts_with('#') {
            return Err(Fault::Asked("Give the name of a person.".to_string()));
        }
        let paths = db.held_about(&name).paths;
        let snaps = snapshots(&paths, home, &name);
        let forgot = db.forget_person(&name)?;
        (name, forgot, snaps)
    };
    let name = name.as_str();
    let mut put = |k: &str, n: usize| done.insert(k.to_string(), json!(n));
    put("faces", forgot.faces);
    put("labels", forgot.labels);
    put("history", forgot.history);
    put("photoTags", forgot.photo_tags);
    put("backups", forgot.backups);
    {
        let mut web = inner.web_saved.lock().unwrap_or_else(|e| e.into_inner());
        let before = web.len();
        web.retain(|fid, answer| !web_about(&forgot.fids, name, fid, answer));
        if web.len() < before {
            atomic_write(&data.join(WEB_SAVED), filejson::rows_of(&web, true).as_bytes()).map_err(failed)?;
        }
        put("webSearch", before - web.len());
    }
    let dropped = |folder: PathBuf| -> Result<usize> {
        files(&folder, ".jsonl", false).iter().map(|f| drop_lines(f, |o| named(o, name)).map_err(failed)).sum()
    };
    put("expressionTests", dropped(data.join("expr_eval"))?);
    let mut fitted = 0;
    for file in FITTED {
        let kept: Option<Value> = std::fs::read(data.join(file)).ok().and_then(|b| serde_json::from_slice(&b).ok());
        if let Some(old) = kept {
            let new = scrub(&old, name);
            if new != old {
                atomic_write(&data.join(file), filejson::rows_json(&new, true).as_bytes()).map_err(failed)?;
                fitted += 1;
            }
        }
    }
    put("fitted", fitted);
    // (the sheets can't be told apart by who they show; they are made again when asked for)
    let sheets = files(&data.join("sheets"), ".jpg", false);
    for sheet in &sheets {
        let _ = std::fs::remove_file(sheet);
    }
    put("sheets", sheets.len());
    put("presence", dropped(home.join("presence"))?);
    put("alerts", Inbox::new(home).forget(name).map_err(failed)?);
    put("expressionLog", EXPRESSION.forget(home, name).map_err(failed)?);
    put("musicLog", MUSIC.forget(home, name).map_err(failed)?);
    put("signalLog", SIGNALS.forget(home, name).map_err(failed)?);
    let store = Store::new(home);
    if music_who(&store) == name {
        // this device's music was theirs: it is nobody's now (nothing logged)
        store.update("label", &json!({"fl-musicWho": null})).map_err(failed)?;
        put("musicWho", 1);
    }
    for picture in &snaps {
        let _ = std::fs::remove_file(picture);
        let _ = std::fs::remove_file(picture.with_extension("json"));
    }
    put("webcam", snaps.len());
    put(
        "descriptions",
        descriptions::drop(home, |d| mentions(d.get("text").and_then(Value::as_str).unwrap_or(""), name))
            .map_err(failed)?,
    );
    put("appLog", forget_log(&home.join("app.log"), name));
    put("personSignals", store.forget_signals(name).map_err(failed)?);
    let listed: Vec<String> = alert_list(&store).into_iter().filter(|n| !n.is_empty()).collect();
    let left: Vec<&String> = listed.iter().filter(|n| *n != name).collect();
    if left.len() < listed.len() {
        store
            .update("track", &json!({"ft-alertWho": left.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")}))
            .map_err(failed)?;
        put("alertList", 1);
    }
    // the other devices forget them too (only with a shared database)
    if face_db::config(home).is_some() {
        forgotten::note(home, name).map_err(failed)?;
        done.insert("otherDevices".to_string(), json!(true));
    }
    Ok(Value::Object(done))
}
