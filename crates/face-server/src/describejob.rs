//! Describe all and Describe everything again: the background job that has every photo described,
//! and the one photo's picture for the model.
//!
//! One job at a time, shared with the imports: it runs in the importer's slot and reports in its status, so
//! the page's progress bar and Stop button are the import's own. One request to the server at a time, the
//! next photos loading meanwhile (three at once made a server forty times slower a request).
//!
//! Describe everything again also asks the other devices of a shared database for the photos that only
//! they have (`peerphoto.rs`), until `PEER_MISSES` in a row were asked of devices that didn't answer.

use std::{
    collections::VecDeque,
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

use face_core::image::{Bgr, round_even};
use face_home::{
    describe::{config, host_of, set_again, set_config, where_is},
    descriptions,
};
use face_import::{decode, importer::Importer, video};
use face_library::{labels::LabelDb, photofiles::online_only};
use serde_json::{Map, Value, json};

use crate::{
    describe,
    http::{Request, Response},
    label::{Answer, Labeler, asked, number},
    peerphoto::{Asker, UNREACHED},
};

/// Describe everything again: other devices not reached this often in a row are asked no more in that run
/// (each photo would wait for every address of a device that is off)
const PEER_MISSES: usize = 10;

/// One thing to describe: a photo, or a frame of a video a face was found in.
#[derive(Clone)]
struct Item {
    /// what its description is kept under: the photo's hash, or `hash:frame`
    key: String,
    path: Option<String>,
    frame: Option<i64>,
    /// what to call it when it can't be read
    shown: String,
}

/// A picture at most 1024 px on its longer side, as JPEG, for the model.
fn for_model(img: &Bgr) -> Result<Vec<u8>, String> {
    let longer = img.w.max(img.h).max(1);
    let scale = (1024.0 / longer as f64).min(1.0);
    if scale < 1.0 {
        let (w, h) =
            (round_even(img.w as f64 * scale).max(1) as usize, round_even(img.h as f64 * scale).max(1) as usize);
        return img.resize_area(w, h).to_jpeg(85).map_err(|e| e.0);
    }
    img.to_jpeg(85).map_err(|e| e.0)
}

/// Descriptions are per photo (per frame for a video): every face in it shares one.
pub(crate) fn key(db: &LabelDb, i: usize) -> String {
    let hash = &db.store.photo[i];
    let record = db.store.photos.get(hash);
    let video = record.and_then(|r| r.get("video")).is_some_and(|v| v.as_bool() == Some(true));
    let frame = record
        .filter(|_| video)
        .and_then(|r| r.get("at"))
        .and_then(|at| at.get(&db.store.fid[i]))
        .filter(|f| !f.is_null());
    match frame {
        Some(frame) => format!("{hash}:{}", frame.as_i64().map_or_else(|| frame.to_string(), |f| f.to_string())),
        None => hash.clone(),
    }
}

/// What Describe everything again covers: every photo, and every video frame a face was found in, of the
/// whole library, whether its file is on this computer or only on another device (a duplicate is left out:
/// its first copy is described). Those with a file here first.
fn items(db: &LabelDb) -> Vec<Item> {
    let mut out = Vec::new();
    for (hash, p) in &db.store.photos {
        let said = |k: &str| p.get(k).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
        if said("dup_of").is_some() {
            continue;
        }
        let path = said("path");
        let shown = path.clone().or_else(|| said("name")).unwrap_or_else(|| hash.clone());
        if p.get("video").is_some_and(|v| v.as_bool() == Some(true)) {
            let mut frames: Vec<i64> =
                p.get("at").and_then(Value::as_object).into_iter().flatten().filter_map(|(_, k)| k.as_i64()).collect();
            frames.sort_unstable();
            frames.dedup();
            out.extend(frames.into_iter().map(|k| Item {
                key: format!("{hash}:{k}"),
                path: path.clone(),
                frame: Some(k),
                shown: format!("{shown} (frame {k})"),
            }));
        } else {
            out.push(Item { key: hash.clone(), path, frame: None, shown });
        }
    }
    out.sort_by_key(|it| it.path.is_none());
    out
}

/// Those of `items` whose description wasn't made since a Describe everything again was started.
fn due(home: &Path, items: Vec<Item>, since: &str) -> Result<Vec<Item>, String> {
    let since = face_sync_time(since);
    let when = descriptions::whens(home)?;
    Ok((items.into_iter())
        .filter(|it| {
            // (both as the programs write a time: that text sorts in time order; an older one, in local time, the same once read)
            !when
                .get(&it.key)
                .and_then(Value::as_str)
                .map(face_sync_time)
                .is_some_and(|w| w.len() == 20 && w.ends_with('Z') && w >= since)
        })
        .collect())
}

/// A stored time as UTC text ("2026-11-12T10:20:30Z"); what isn't one stays as it is.
fn face_sync_time(text: &str) -> String {
    #[cfg(feature = "sync")]
    {
        face_sync::local::utc_text(text)
    }
    #[cfg(not(feature = "sync"))]
    {
        text.to_string()
    }
}

impl Labeler {
    /// The photo of face i at most 1024 px on its longer side, as JPEG, for the model.
    pub(crate) fn picture_for_model(&self, i: usize) -> Result<Vec<u8>, String> {
        let whole = self.photo_jpeg(i).ok_or("The original photo isn't on this computer any more.")?;
        for_model(&Bgr::from_jpeg(&whole).map_err(|e| e.0)?)
    }

    /// For Settings: how much Describe everything again covers, and an unfinished one's start and what is left.
    pub(crate) fn again_state(&self) -> Result<Value, String> {
        let all = items(&self.inner.db());
        let again = config(&self.home).again;
        let (total, frames, elsewhere) = (
            all.len(),
            all.iter().filter(|it| it.frame.is_some()).count(),
            all.iter().filter(|it| it.path.is_none()).count(),
        );
        let left = match &again {
            Some((since, _)) => due(&self.home, all, since)?.len(),
            None => 0,
        };
        Ok(json!({"total": total, "frames": frames, "elsewhere": elsewhere, "since": again.as_ref().map(|a| &a.0),
                  "model": again.as_ref().map(|a| &a.1), "left": left}))
    }

    /// Describe every photo not described yet (not videos, duplicates or online-only cloud files) in the
    /// background. `again` ("fresh" or "on"): Describe everything again instead, everything in `items`,
    /// described or not, each description replaced as its new one arrives; "fresh" starts one from now, "on"
    /// carries on with the unfinished one, which is kept until nothing is left.
    pub(crate) fn describe_all(&self, again: Option<&str>) -> Result<(), String> {
        let importer = &self.inner.importer;
        if importer.running() {
            return Err("An import is already running.".to_string());
        }
        let home = self.home.clone();
        let c = config(&home);
        let name = describe::model(&c, None).map_err(|e| e.why)?; // says what is missing
        let mut mark = c.again.clone().filter(|_| again.is_some());
        if again == Some("on") && mark.is_none() {
            return Err("Nothing is unfinished: start Describe everything again instead.".to_string());
        }
        if again == Some("fresh") {
            mark = set_again(&home, Some(&name))?;
        }
        let (inner, anew) = (self.inner.clone(), again.is_some());
        // (asked of the shared database here, not on the job's thread: where the other devices are)
        let devices = if c.spread { self.device_addresses() } else { Vec::new() };
        importer.run_other("photo descriptions", "Describing photos", move |job| {
            job.set("again", json!(anew));
            let done = (|| -> Result<(), String> {
                let todo = match &mark {
                    Some((since, _)) => due(&home, items(&inner.db()), since)?,
                    None => {
                        let all: Vec<Item> = items(&inner.db())
                            .into_iter()
                            .filter(|it| it.path.is_some() && it.frame.is_none())
                            .collect();
                        let mut todo = Vec::new();
                        for it in all {
                            let described = descriptions::saved(&home, &it.key)?
                                .is_some_and(|d| d.as_object().is_some_and(|o| !o.is_empty()));
                            if !described {
                                todo.push(Item { shown: it.path.clone().unwrap_or_default(), ..it });
                            }
                        }
                        todo
                    }
                };
                job.set("total", json!(todo.len()));
                if !todo.is_empty() {
                    job.set("stage", json!("Finding LM Studio servers"));
                    let c = config(&home);
                    let host = if where_is(&c.url).0 == "this computer" {
                        "this computer".to_string()
                    } else {
                        host_of(&c.url)
                    };
                    let mut servers = vec![Server { host, url: None }];
                    // with "spread" on, the owner's other LM Studio servers with the same model take photos too
                    let helping = describe::helpers(&home, &name, &|| devices.clone());
                    servers.extend(helping.into_iter().map(|(host, url)| Server { host, url: Some(url) }));
                    job.set("by", Value::Object(servers.iter().map(|s| (s.host.clone(), json!(0))).collect()));
                    let what = if anew { "Describing everything again" } else { "Describing photos" };
                    let on = if servers.len() > 1 { format!(" on {} computers", servers.len()) } else { String::new() };
                    job.set("stage", json!(format!("{what}{on}")));
                    // what isn't on this computer is asked of the device that has it, in Describe everything again
                    let elsewhere = (anew && inner.peer_ready()).then_some(&inner.asker);
                    run(job, &home, todo, &name, &servers, elsewhere);
                }
                if let (Some((since, _)), false) = (&mark, job.stopped())
                    && due(&home, items(&inner.db()), since)?.is_empty()
                {
                    set_again(&home, None)?; // all of it done; else it stays, to carry on with
                }
                Ok(())
            })();
            descriptions::flush(&home)?;
            done
        })
    }
}

/// A server that describes: the configured one (`url` None: it keeps going to the end) or one of the owner's
/// other LM Studio servers.
struct Server {
    host: String,
    url: Option<String>,
}

/// how long a describing helper that failed twice in a row sits out before it is tried again (its LM Studio
/// may have been loading a model, or starting again)
const HELPER_RETRY: Duration = Duration::from_secs(300);

/// Describe the photos in `todo` with one worker a server (the first is the configured one). Loader threads
/// read and shrink the photos a few ahead; each worker takes the next one as it finishes: one request to a
/// server at a time. A helper that fails gives its photo back to the others; failing twice in a row it is
/// left out, said on the progress bar, and tried again every `HELPER_RETRY` with one photo. Counts go into
/// the job's status. `elsewhere`: a photo that isn't on this computer is asked of the device that has it.
fn run(job: &Importer, home: &Path, todo: Vec<Item>, name: &str, servers: &[Server], elsewhere: Option<&Asker>) {
    type Loaded = (Item, Vec<u8>);
    fn held<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        lock.lock().unwrap_or_else(|e| e.into_inner())
    }
    let left = Mutex::new(todo.len());
    let work = Mutex::new(VecDeque::from(todo));
    // loaded photos waiting for a free server, and photos a failing helper gave back (taken first)
    let (ready, back): (Mutex<VecDeque<Loaded>>, Mutex<VecDeque<Loaded>>) =
        (Mutex::new(VecDeque::new()), Mutex::new(VecDeque::new()));
    let room = 2 * servers.len() + 2;
    let finish = |n: usize| {
        let mut left = held(&left);
        *left = left.saturating_sub(n);
        job.add("done", n);
        if job.count("done").is_multiple_of(25) {
            let _ = descriptions::flush(home);
        }
    };
    // (other devices not reached in a row, no more asked in this run)
    let away = Mutex::new((0usize, elsewhere.is_none()));
    // The photo from another device, or None. After `PEER_MISSES` in a row that no device answered for, no
    // more are asked in this run.
    let from_device = |item: &Item| -> Option<Vec<u8>> {
        let asker = elsewhere.filter(|_| !held(&away).1)?;
        let got = asker.fetch(item.key.split(':').next().unwrap_or(&item.key), item.frame);
        let mut away = held(&away);
        match got {
            Ok((jpeg, _)) => {
                away.0 = 0;
                Some(jpeg)
            }
            Err(why) => {
                if why.contains(UNREACHED) {
                    away.0 += 1;
                    if away.0 >= PEER_MISSES && !away.1 {
                        away.1 = true;
                        let why: String = why.chars().take(200).collect();
                        let note = json!(format!("what is only on other devices ({why})"));
                        job.with_status(|st| {
                            if let Some(list) = st.entry("left_out").or_insert_with(|| json!([])).as_array_mut() {
                                list.push(note);
                            }
                        });
                    }
                }
                None
            }
        }
    };
    let load = |item: &Item| -> Result<Option<Vec<u8>>, String> {
        let unreadable = || "couldn't read the picture (damaged, or a format it can't open)".to_string();
        let Some(path) = item.path.as_deref().map(Path::new).filter(|p| p.exists() && !online_only(p)) else {
            // not on this computer now (moved, online-only): another device that has it shows it, or nobody
            let Some(jpeg) = from_device(item) else { return Ok(None) };
            return for_model(&Bgr::from_jpeg(&jpeg).map_err(|_| unreadable())?).map(Some);
        };
        let img = match item.frame {
            None => decode::load_file(path),
            Some(k) => video::frame(path, k),
        };
        for_model(&img.map_err(|_| unreadable())?).map(Some)
    };
    // A helper left out waits; false when the run ended or was stopped meanwhile.
    let sit_out = || {
        let until = Instant::now() + HELPER_RETRY;
        while Instant::now() < until {
            if job.stopped() || *held(&left) == 0 {
                return false;
            }
            std::thread::sleep(
                until
                    .saturating_duration_since(Instant::now())
                    .clamp(Duration::from_millis(50), Duration::from_secs(1)),
            );
        }
        !job.stopped()
    };
    let (left, work, ready, back, finish, load, sit_out) = (&left, &work, &ready, &back, &finish, &load, &sit_out);
    std::thread::scope(|scope| {
        for _ in 0..2 + servers.len() {
            scope.spawn(move || {
                while !job.stopped() {
                    let Some(item) = held(work).pop_front() else { return };
                    match load(&item) {
                        Err(why) => {
                            job.problem(Path::new(&item.shown), &why);
                            finish(1);
                        }
                        Ok(None) => {
                            job.add("known", 1);
                            finish(1);
                        }
                        Ok(Some(jpeg)) => {
                            let mut loaded = Some((item, jpeg));
                            while !job.stopped() {
                                let mut waiting = held(ready);
                                if waiting.len() < room {
                                    waiting.extend(loaded.take());
                                    break;
                                }
                                drop(waiting);
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        }
                    }
                }
            });
        }
        for (k, server) in servers.iter().enumerate() {
            let main = k == 0;
            scope.spawn(move || {
                let mut fails = 0;
                while !job.stopped() {
                    if *held(left) == 0 {
                        return;
                    }
                    let next = held(back).pop_front().or_else(|| held(ready).pop_front());
                    let Some((item, jpeg)) = next else {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    };
                    match describe::ask(home, &jpeg, name, server.url.as_deref()) {
                        // another computer: its photo goes back to the others
                        Err(e) if !main => {
                            held(back).push_back((item, jpeg));
                            fails += 1;
                            if fails >= 2 {
                                let why: String = e.why.chars().take(200).collect();
                                let minutes = (HELPER_RETRY.as_secs_f64() / 60.0).round().max(1.0);
                                let note = json!(format!(
                                    "{} (failed twice: {why}; tried again every {minutes} min)",
                                    server.host
                                ));
                                // shown on the progress bar, not as an unreadable photo
                                job.with_status(|st| {
                                    if let Some(list) = st.entry("left_out").or_insert_with(|| json!([])).as_array_mut()
                                    {
                                        list.push(note.clone());
                                    }
                                });
                                if !sit_out() {
                                    return;
                                }
                                // back in: one more failure and it sits out again
                                job.with_status(|st| {
                                    if let Some(list) = st.get_mut("left_out").and_then(Value::as_array_mut) {
                                        list.retain(|n| *n != note);
                                    }
                                });
                                fails = 1;
                            }
                        }
                        Err(e) => {
                            job.problem(Path::new(&item.shown), &e.why);
                            finish(1);
                        }
                        Ok(described) => {
                            fails = 0;
                            match descriptions::save(home, &item.key, &described, false) {
                                Err(why) => job.problem(Path::new(&item.shown), &why),
                                Ok(_) => job.with_status(|st| {
                                    let count = |v: Option<&Value>| v.and_then(Value::as_u64).unwrap_or(0) + 1;
                                    st.insert("new".to_string(), json!(count(st.get("new"))));
                                    if let Some(by) = st.get_mut("by").and_then(Value::as_object_mut) {
                                        by.insert(server.host.clone(), json!(count(by.get(&server.host))));
                                    }
                                }),
                            }
                            finish(1);
                        }
                    }
                }
            });
        }
    });
}

/// The routes of Describe photo that are sent something.
pub(crate) const POSTS: [&str; 7] = [
    "/api/describe",
    "/api/describe/load",
    "/api/describe/all",
    "/api/describe/again",
    "/api/describe/config",
    "/api/describe/test",
    "/api/describe/getmodel",
];

/// What counts as nothing: null, false, zero, an empty text, list or object.
pub(crate) fn falsy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !b,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
    }
}

impl Labeler {
    /// The addresses of the owner's other devices, as each gave them in the shared database, each device's
    /// together (none without one).
    pub(crate) fn device_addresses(&self) -> Vec<Vec<String>> {
        #[cfg(feature = "sync")]
        {
            self.other_devices()
        }
        #[cfg(not(feature = "sync"))]
        {
            Vec::new()
        }
    }

    /// What Describe photo's routes answer to a question; None for a route that isn't one of them.
    pub(crate) fn describe_get(&self, req: &Request) -> Option<Answer> {
        let failed = |why: String| Response::error(500, why);
        if req.path == "/api/describe/status" {
            let photos = {
                let db = self.inner.db();
                let has = |p: &Value, k: &str| !falsy(p.get(k));
                db.store.photos.values().filter(|p| has(p, "path") && !has(p, "video") && !has(p, "dup_of")).count()
            };
            return Some(
                (|| {
                    let mut st = describe::status(&self.home, &|| self.device_addresses());
                    st["described"] = json!(descriptions::count(&self.home)?);
                    st["photos"] = json!(photos);
                    st["again"] = self.again_state()?;
                    st["secs"] = match (st["ok"] == true, st["model"].as_str()) {
                        (true, Some(model)) => json!(descriptions::typical_secs(&self.home, model)?),
                        _ => Value::Null,
                    };
                    Ok(Response::json(st))
                })()
                .map_err(failed),
            );
        }
        let rest = req.path.strip_prefix("/api/describe/")?;
        Some((|| {
            let i = number(rest)?;
            let key = {
                let db = self.inner.db();
                let Some(i) = usize::try_from(i).ok().filter(|i| *i < db.n()) else {
                    return Err(Response::error(404, "no such face"));
                };
                key(&db, i)
            };
            Ok(Response::json(json!({"saved": descriptions::saved(&self.home, &key).map_err(failed)?})))
        })())
    }

    /// What Describe photo's routes answer to something sent; None for a route that isn't one of them.
    pub(crate) fn describe_post(&self, path: &str, body: &Map<String, Value>) -> Option<Answer> {
        let ok = || Ok(Response::json(json!({"ok": true})));
        Some(match path {
            // describe every photo not described yet (background)
            "/api/describe/all" => self.describe_all(None).map_err(asked).and_then(|()| ok()),
            // Settings: describe everything again, or carry on with it
            "/api/describe/again" => {
                let how = if body.get("carry_on") == Some(&Value::Bool(true)) { "on" } else { "fresh" };
                self.describe_all(Some(how)).map_err(asked).and_then(|()| ok())
            }
            // describe the photo with the model (the answer is kept: asked once a photo)
            "/api/describe" => (|| {
                let key = {
                    let db = self.inner.db();
                    let i = body.get("i").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < db.n());
                    let Some(i) = i else { return Err(asked("no such face")) };
                    (i, key(&db, i))
                };
                let (i, key) = key;
                let failed = |why: String| Response::error(500, why);
                let saved = descriptions::saved(&self.home, &key).map_err(failed)?;
                if let Some(Value::Object(mut d)) = saved.filter(|d| !falsy(Some(d)) && falsy(body.get("refresh"))) {
                    d.insert("cached".to_string(), json!(true));
                    return Ok(Response::json(Value::Object(d)));
                }
                let jpeg = self.picture_for_model(i).map_err(asked)?;
                let name = describe::model(&config(&self.home), None).map_err(|e| asked(e.why))?;
                let described = describe::ask(&self.home, &jpeg, &name, None).map_err(|e| asked(e.why))?;
                Ok(Response::json(descriptions::save(&self.home, &key, &described, true).map_err(failed)?))
            })(),
            // Settings: which describer, server, model, API key. Choosing the kind of server or the model loads
            // the model that suits this computer with its settings (the owner: "decided when the settings are
            // interacted with"), on a thread of its own
            "/api/describe/config" => {
                let changes: Map<String, Value> = (["use", "platform", "url", "model", "key", "spread"].iter())
                    .filter_map(|k| Some((k.to_string(), body.get(*k)?.clone())))
                    .collect();
                let chose = changes.contains_key("platform") || changes.contains_key("model");
                set_config(&self.home, &changes).map_err(asked).and_then(|_| {
                    if chose {
                        let _ = describe::load_suggested(&self.home);
                    }
                    ok()
                })
            }
            // Settings' Load button: the suggested model with its settings
            "/api/describe/load" => describe::load_suggested(&self.home).map_err(asked).and_then(|()| ok()),
            // Settings' Test button
            "/api/describe/test" => {
                let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("");
                describe::test(&self.home, text("url"), text("key"))
                    .map(|message| Response::json(json!({"ok": true, "message": message})))
                    .map_err(|e| asked(e.why))
            }
            // the built-in describer's download, in the background
            "/api/describe/getmodel" => {
                crate::florence::download_in_background();
                ok()
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_for_the_model_is_1024_px_at_most_on_its_longer_side() {
        let size = |w: usize, h: usize| {
            let jpeg = for_model(&Bgr { w, h, px: vec![120; w * h * 3] }).unwrap();
            let back = Bgr::from_jpeg(&jpeg).unwrap();
            (back.w, back.h)
        };
        assert_eq!(size(2000, 1000), (1024, 512));
        assert_eq!(size(600, 3000), (205, 1024));
        assert_eq!(size(1024, 768), (1024, 768), "one that fits is sent as it is");
        assert_eq!(size(40, 30), (40, 30));
    }

    #[test]
    fn what_counts_as_nothing() {
        for v in [json!(null), json!(false), json!(0), json!(0.0), json!(""), json!([]), json!({})] {
            assert!(falsy(Some(&v)), "{v}");
        }
        for v in [json!(true), json!(1), json!("no"), json!([0]), json!({"a": 0})] {
            assert!(!falsy(Some(&v)), "{v}");
        }
        assert!(falsy(None));
    }

    #[test]
    fn what_is_due_is_what_was_not_described_since_the_start() {
        let home = std::env::temp_dir().join(format!("face-describejob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let mut all = Map::new();
        all.insert("before".to_string(), json!({"text": "t", "when": "2026-11-12T10:20:29Z"}));
        all.insert("at".to_string(), json!({"text": "t", "when": "2026-11-12T10:20:30Z"}));
        all.insert("after:30".to_string(), json!({"text": "t", "when": "2026-11-13T00:00:00Z"}));
        all.insert("undated".to_string(), json!({"text": "t"}));
        all.insert("odd".to_string(), json!({"text": "t", "when": "some day"}));
        descriptions::write(&home, &all).unwrap();
        let items = ["before", "at", "after:30", "undated", "odd", "never"].map(|key| Item {
            key: key.to_string(),
            path: None,
            frame: None,
            shown: String::new(),
        });
        let left: Vec<String> =
            due(&home, items.to_vec(), "2026-11-12T10:20:30Z").unwrap().into_iter().map(|it| it.key).collect();
        assert_eq!(left, ["before", "undated", "odd", "never"]);
        let _ = std::fs::remove_dir_all(&home);
    }
}
