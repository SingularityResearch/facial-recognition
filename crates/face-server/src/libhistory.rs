//! The library's part of the signal history (the owner, 2026-11-24: "We should also be able to run this on
//! imported images / videos in the existing library and add the feature to the image / video import - not
//! just on the web cameras. We can only assume certain things from the image backfill - and that is OK.").
//!
//! What a picture of the library says of each named person in it, as rows of the table the cameras write
//! (`face_db::signals::send_library`): who, when (the camera's own time where the photo has one, else the
//! file's; a video's frame that many seconds later), how good the face's picture is and how large the face,
//! how many faces the picture has and how many of them are named, and how the face looks (the expression
//! model, on the picture the library keeps of the face). Nothing a still can't say is assumed: no voice, no
//! pulse, no movement, nothing of a person's usual. A row says where its time is from, and a face that was
//! named automatically says so.
//!
//! It runs as a job in the importer's one slot (so never beside an import or Describe all): started from
//! Settings, and by itself after a folder's import where this device's history is switched on. The job's own
//! thread does all of it, with a connection to the shared database that is its own: no request waits for it.
//! The library's lock is held only to copy what the library says, never while a picture is read or the
//! database asked. What the history has of a face's looks already is read back first, so a face's picture is
//! read for them once, whoever the face is named for later.

use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};

use face_core::{
    faces::{EXPRESSIONS, Faces, Q_EXPR},
    image::Bgr,
};
use face_import::importer::Importer;
use face_library::labels::LabelDb;
use serde_json::{Map, Value, json};

use crate::{
    http::Response,
    label::{Answer, Inner, Labeler, asked},
};

/// what the job is called in the import's status
pub(crate) const JOB: &str = "the library's history";
/// pictures in one statement
const PICTURES_AT_ONCE: usize = 100;
/// a statement that has no answer after this long ends the job
const GIVE_UP: Duration = Duration::from_secs(30);
/// an import's result stays on the page this long before the history's job takes its place (seconds)
const AFTER_IMPORT: f64 = 300.0;
/// a time before 1900 or after 2200 is no picture's time
const FIRST_MS: i64 = -2_208_988_800_000;
const LAST_MS: i64 = 7_258_118_400_000;

/// One named face of a picture, as the library says it.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    /// where the face is in the library now, and its permanent id
    i: usize,
    face: String,
    name: String,
    /// named automatically, and not confirmed
    auto: bool,
    quality: f32,
    /// the face's height in the picture, in its pixels
    px: f32,
    /// a video's frame the face is in, and how many seconds into the video that is
    frame: Option<i64>,
    after: f64,
}

/// One picture of the library (a photo, or a video with every frame a face was found in).
#[derive(Clone, Debug, PartialEq)]
struct Picture {
    /// its content key
    key: String,
    video: bool,
    /// when it was taken (milliseconds since 1970), and where that is from: the camera's own time in the
    /// photo, or only the file's time
    t: Option<i64>,
    time: &'static str,
    /// every face found in it, named or not, by the frame it is in (a photo has one "frame")
    all: Vec<Option<i64>>,
    named: Vec<Seen>,
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// A time in seconds since 1970 as milliseconds, where it is a picture's time at all.
fn a_time(seconds: Option<f64>) -> Option<i64> {
    let ms = seconds.filter(|s| s.is_finite()).map(|s| (s * 1000.0).round())?;
    (ms >= FIRST_MS as f64 && ms <= LAST_MS as f64).then_some(ms as i64)
}

/// Every picture of the library with its faces, in the library's order. Only what the library has in memory:
/// no file is read (a photo whose own facts were not read yet has its file's time).
fn pictures(db: &LabelDb) -> Vec<Picture> {
    let mut place: HashMap<&str, usize> = HashMap::new();
    let mut out: Vec<Picture> = Vec::new();
    for i in 0..db.n() {
        // (a library whose lists are not all one length is not read past the shortest: the programs end as a
        // whole when a thread fails)
        let store = &db.store;
        let (Some(key), Some(fid), Some(quality), Some(placed)) =
            (store.photo.get(i), store.fid.get(i), store.quality.get(i), store.boxes.get(i))
        else {
            continue;
        };
        let Some(p) = store.photos.get(key).and_then(Value::as_object) else { continue };
        let video = truthy(p.get("video"));
        let at = *place.entry(key.as_str()).or_insert_with(|| {
            let taken = if video { None } else { db.facts.facts.get(key).and_then(|f| a_time(f.t)) };
            let (t, time) = match taken {
                Some(t) => (Some(t), "camera"),
                None => (a_time(p.get("mtime").and_then(Value::as_f64)), "file"),
            };
            out.push(Picture { key: key.clone(), video, t, time, all: Vec::new(), named: Vec::new() });
            out.len() - 1
        });
        let frame = p.get("at").filter(|_| video).and_then(|a| a.get(fid)).and_then(Value::as_i64);
        let Some(picture) = out.get_mut(at) else { continue };
        picture.all.push(frame);
        // (a face that is Ignored, or of nobody yet, is counted and has no row)
        let Some(name) = db.name(i).filter(|n| !n.starts_with('#') && *n != "Unknown") else { continue };
        picture.named.push(Seen {
            i,
            face: fid.clone(),
            name: name.to_string(),
            auto: db.by(i) == "auto",
            quality: *quality,
            px: placed[3],
            frame,
            after: db.video_at(i).unwrap_or(0.0),
        });
    }
    out
}

/// The look with the largest share of a face's looks.
fn top_look(looks: &Value) -> Option<&str> {
    let looks = looks.as_object()?;
    let mut best: Option<(&str, f64)> = None;
    for (label, share) in looks {
        let share = share.as_f64()?;
        if best.is_none_or(|(_, b)| share > b) {
            best = Some((label, share));
        }
    }
    best.map(|(label, _)| label)
}

/// The faces of a picture that get a row, each with its moment (milliseconds since 1970): one for each named
/// person at each moment of the picture. A picture without a time has none (it has no place in a history);
/// of two faces named for one person at one moment the better picture counts.
fn chosen(p: &Picture) -> Vec<(i64, &Seen)> {
    let Some(t) = p.t else { return Vec::new() };
    let mut named: Vec<&Seen> = p.named.iter().collect();
    // (a quality that is not a number was never scored: last)
    named.sort_by(|a, b| b.quality.total_cmp(&a.quality).then(a.i.cmp(&b.i)));
    named.sort_by_key(|s| s.quality.is_nan());
    let mut had: Vec<(&str, i64)> = Vec::new();
    let mut out = Vec::new();
    for s in named {
        let at = t + (s.after * 1000.0).round() as i64;
        if !had.contains(&(s.name.as_str(), at)) {
            had.push((s.name.as_str(), at));
            out.push((at, s));
        }
    }
    out
}

/// The rows of one picture: one for each face `chosen`. `looks`: each face's looks, where its picture was
/// read.
fn rows_of(p: &Picture, looks: &HashMap<String, Value>) -> Vec<Value> {
    let mut rows = Vec::new();
    for (at, s) in chosen(p) {
        let mut data = Map::new();
        data.insert("face".to_string(), json!(s.face));
        data.insert("faces".to_string(), json!(p.all.iter().filter(|f| **f == s.frame).count()));
        data.insert("named".to_string(), json!(p.named.iter().filter(|o| o.frame == s.frame).count()));
        data.insert("time".to_string(), json!(p.time));
        if s.px.is_finite() && s.px > 0.0 {
            data.insert("px".to_string(), json!(s.px.round() as i64));
        }
        if !s.quality.is_nan() {
            data.insert("quality".to_string(), json!((f64::from(s.quality) * 1000.0).round() / 1000.0));
        }
        if s.auto {
            data.insert("by".to_string(), json!("auto"));
        }
        if let Some(l) = looks.get(&s.face) {
            data.insert("looks".to_string(), l.clone());
            if let Some(top) = top_look(l) {
                // (as a camera's row has it: how often each look won, and a picture is one reading)
                let won: Map<String, Value> = [(top.to_string(), json!(1))].into_iter().collect();
                data.insert("look".to_string(), Value::Object(won));
            }
        }
        rows.push(json!({"t": at, "name": s.name, "source": if p.video { "video" } else { "photo" }, "ref": p.key,
                         "data": Value::Object(data)}));
    }
    rows
}

/// How a face looks, from the picture the library keeps of it: each look's share. Nothing where the picture
/// can't be read or is not the models' size.
fn read_looks(models: &Faces, jpeg: &[u8]) -> Option<Value> {
    let crop = Bgr::from_jpeg(jpeg).ok().filter(|c| c.w == 112 && c.h == 112)?;
    let shares = models.expression(&crop).ok()??;
    let looks: Map<String, Value> = (EXPRESSIONS.iter().zip(shares))
        .filter(|(_, share)| share.is_finite())
        .map(|(label, share)| (label.to_string(), json!((share * 1e4).round() / 1e4)))
        .collect();
    (looks.len() == EXPRESSIONS.len()).then_some(Value::Object(looks))
}

/// Whether this device's own history is switched on (its Face tracking tab's switch).
pub(crate) fn switched_on(home: &Path) -> bool {
    let setting = face_home::store::Store::new(home).setting("track", face_home::siglog::OPTION);
    setting.as_deref() == Some("1")
}

/// Start the job, unless another runs in the import's slot.
pub(crate) fn start(inner: &Arc<Inner>, home: &Path) -> Result<(), String> {
    if inner.importer.running() {
        return Err("An import is already running.".to_string());
    }
    let Some(settings) = face_db::Settings::load(home)? else {
        return Err("The history is kept in your shared database, and none is set up on this device.".to_string());
    };
    let (job_inner, home) = (inner.clone(), home.to_path_buf());
    inner
        .importer
        .run_other(JOB, "Working out the history from the library", move |job| run(job, &job_inner, &home, &settings))
}

/// The import this status is of, by when it started, if it is one the library's history should follow now:
/// an import of photos (not Describe all, the possible faces' scan or this job itself) that ended well,
/// added something, and has been on the page for `AFTER_IMPORT`.
fn import_to_follow(st: &Value, now: f64) -> Option<f64> {
    let started = st["started"].as_f64()?;
    let other = matches!(st["folder"].as_str(), None | Some("" | JOB | "photo descriptions" | "possible faces"));
    let added = st["new"].as_u64().unwrap_or(0) > 0;
    let ended = started + st["elapsed"].as_f64()?;
    (st["running"] == false && st["stage"] == "Done" && !other && added && now - ended >= AFTER_IMPORT)
        .then_some(started)
}

/// After a folder's import the library's history is worked out again by itself, where this device's history
/// is switched on (the owner: "add the feature to the image / video import"). It looks every 20 s, as the
/// import of the tracker's snapshots does; an import is followed once.
pub(crate) fn follow_imports(inner: Arc<Inner>, home: std::path::PathBuf) {
    let _ = std::thread::Builder::new().name("library history".into()).spawn(move || {
        let mut followed = None;
        loop {
            std::thread::sleep(Duration::from_secs(20));
            let now =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
            let Some(import) = import_to_follow(&inner.importer.status(), now) else { continue };
            if followed == Some(import) {
                continue;
            }
            // (another job began meanwhile: looked at again the next time round. Switched off, or no shared
            // database: the import is not followed later either)
            if inner.importer.running() {
                continue;
            }
            if switched_on(&home) {
                let _ = start(&inner, &home);
            }
            followed = Some(import);
        }
    });
}

fn run(job: &Importer, inner: &Inner, home: &Path, settings: &face_db::Settings) -> Result<(), String> {
    // (a person forgotten on this device has no rows made again)
    let forgotten = face_home::store::Store::new(home).forgotten();
    let mut pictures = pictures(&inner.db());
    for p in &mut pictures {
        p.named.retain(|s| !forgotten.contains(&s.name));
    }
    job.set("total", json!(pictures.len()));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let away = || "The shared database didn't answer.".to_string();
    let device = face_db::Device::load(home, &crate::device().0)?;
    let client = rt
        .block_on(async { tokio::time::timeout(GIVE_UP, face_db::connect(settings, &settings.schema, &device)).await })
        .map_err(|_| away())??;
    let mut looks = rt
        .block_on(async { tokio::time::timeout(GIVE_UP * 4, face_db::signals::library_looks(&client)).await })
        .map_err(|_| away())??;
    // (without the expression model the rows say who, when and how good, and no looks)
    let models = job.models().ok().filter(|m| m.has_expression());
    let (mut written, mut read, mut timeless) = (0u64, 0u64, 0usize);
    for some in pictures.chunks(PICTURES_AT_ONCE) {
        if job.stopped() {
            break;
        }
        if let Some(models) = &models {
            // the faces whose looks nobody has read yet: their pictures are copied under the library's
            // lock and read without it
            let jpegs: Vec<(String, Vec<u8>)> = {
                let db = inner.db();
                // (only the faces that get a row are read: a picture without a time has none)
                (some.iter().flat_map(chosen).map(|(_, s)| s))
                    .filter(|s| !looks.contains_key(&s.face))
                    // (a blurry face is not read: the model is not unsure about it, it is wrong; a face that
                    // was never scored is read)
                    .filter(|s| s.quality.is_nan() || s.quality >= Q_EXPR)
                    .filter(|s| db.store.fid.get(s.i) == Some(&s.face))
                    .filter_map(|s| {
                        let jpeg = db.store.thumbs.get(s.i).filter(|j| !j.is_empty())?;
                        Some((s.face.clone(), jpeg.clone()))
                    })
                    .collect()
            };
            for (face, jpeg) in jpegs {
                if let Some(l) = read_looks(models, &jpeg) {
                    looks.insert(face, l);
                    read += 1;
                }
            }
        }
        let rows: Vec<Value> = some.iter().flat_map(|p| rows_of(p, &looks)).collect();
        timeless += some.iter().filter(|p| p.t.is_none() && !p.named.is_empty()).count();
        let keys: Vec<String> = some.iter().map(|p| p.key.clone()).collect();
        written += rt
            .block_on(async {
                tokio::time::timeout(GIVE_UP, face_db::signals::send_library(&client, &keys, &rows)).await
            })
            .map_err(|_| away())??;
        job.add("done", some.len());
        job.with_status(|st| {
            st.insert("new".to_string(), json!(written));
            st.insert("faces".to_string(), json!(read));
            st.insert("timeless".to_string(), json!(timeless));
        });
    }
    Ok(())
}

impl Labeler {
    /// What the library's history answers to a question; None for a route that isn't its own.
    pub(crate) fn libhistory_get(&self, path: &str) -> Option<Answer> {
        if path != "/api/history/library" {
            return None;
        }
        // (the library's lock only while what it says is copied: the counting is done without it)
        let found = pictures(&self.inner.db());
        let named = found.iter().filter(|p| !p.named.is_empty()).count();
        let rows: usize = found.iter().map(|p| rows_of(p, &HashMap::new()).len()).sum();
        let joined = face_db::Settings::load(&self.home).ok().flatten().is_some();
        Some(Ok(Response::json(json!({"pictures": found.len(), "named": named, "rows": rows, "joined": joined,
                                      "on": switched_on(&self.home)}))))
    }

    /// What it answers to something sent; None for a route that isn't its own.
    pub(crate) fn libhistory_post(&self, path: &str) -> Option<Answer> {
        if path != "/api/history/library" {
            return None;
        }
        Some(start(&self.inner, &self.home).map_err(asked).map(|()| Response::json(json!({"ok": true}))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(i: usize, name: &str, quality: f32, frame: Option<i64>, after: f64) -> Seen {
        Seen { i, face: format!("face{i}"), name: name.to_string(), auto: false, quality, px: 120.4, frame, after }
    }

    #[test]
    fn a_photo_gives_a_row_for_each_named_person_in_it_at_the_photos_own_time() {
        let photo = Picture {
            key: "aa11".to_string(),
            video: false,
            t: Some(1_500_000_000_000),
            time: "camera",
            all: vec![None, None, None],
            named: vec![seen(0, "Ann", 0.8, None, 0.0), Seen { auto: true, ..seen(2, "Bea", f32::NAN, None, 0.0) }],
        };
        let looks: HashMap<String, Value> =
            [("face0".to_string(), json!({"happy": 0.7, "neutral": 0.2, "sad": 0.1}))].into_iter().collect();
        let rows = rows_of(&photo, &looks);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            json!({"t": 1_500_000_000_000i64, "name": "Ann", "source": "photo", "ref": "aa11",
                   "data": {"face": "face0", "faces": 3, "named": 2, "time": "camera", "px": 120, "quality": 0.8,
                            "looks": {"happy": 0.7, "neutral": 0.2, "sad": 0.1}, "look": {"happy": 1}}})
        );
        // a face nobody read the looks of, never scored, and named automatically: the row says only that
        assert_eq!(
            rows[1]["data"],
            json!({"face": "face2", "faces": 3, "named": 2, "time": "camera", "px": 120, "by": "auto"})
        );
        // no row names another person: who was there together is read from the rows of one picture
        assert!(!rows[0].to_string().contains("Bea") && !rows[1].to_string().contains("Ann"));
        // a picture without a time has no place in a history
        assert!(rows_of(&Picture { t: None, ..photo.clone() }, &looks).is_empty());
        // two faces named for one person in one photo: the better picture's row
        let twice =
            Picture { named: vec![seen(0, "Ann", 0.3, None, 0.0), seen(1, "Ann", 0.9, None, 0.0)], ..photo.clone() };
        let rows = rows_of(&twice, &looks);
        assert_eq!((rows.len(), rows[0]["data"]["face"].clone()), (1, json!("face1")));
    }

    #[test]
    fn a_videos_faces_are_rows_at_their_frames_moments_and_its_time_is_only_the_files() {
        let video = Picture {
            key: "bb22".to_string(),
            video: true,
            t: Some(1_600_000_000_000),
            time: "file",
            all: vec![Some(0), Some(0), Some(90), Some(90)],
            named: vec![
                seen(0, "Ann", 0.6, Some(0), 0.0),
                seen(2, "Ann", 0.7, Some(90), 3.0),
                seen(3, "Bea", 0.5, Some(90), 3.0),
            ],
        };
        let rows = rows_of(&video, &HashMap::new());
        let said: Vec<(i64, &str, i64, i64)> = (rows.iter())
            .map(|r| {
                let d = &r["data"];
                (
                    r["t"].as_i64().unwrap(),
                    r["name"].as_str().unwrap(),
                    d["faces"].as_i64().unwrap(),
                    d["named"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [(1_600_000_003_000, "Ann", 2, 2), (1_600_000_000_000, "Ann", 2, 1), (1_600_000_003_000, "Bea", 2, 2)]
        );
        assert!(rows.iter().all(|r| r["source"] == "video" && r["data"]["time"] == "file"));
        // two frames so close that they are one moment (a tenth of a second) are one row for a person: the
        // better picture's
        let close = Picture {
            named: vec![seen(0, "Ann", 0.6, Some(90), 3.0), seen(1, "Ann", 0.9, Some(91), 3.0)],
            ..video.clone()
        };
        let rows = rows_of(&close, &HashMap::new());
        assert_eq!((rows.len(), rows[0]["data"]["face"].clone()), (1, json!("face1")));
        assert_eq!(chosen(&close).len(), 1);
    }

    #[test]
    fn only_an_import_of_photos_that_added_something_is_followed_and_only_after_its_result_was_shown() {
        let import = json!({"running": false, "stage": "Done", "folder": "D:\\photos\\2024", "new": 12,
                            "started": 1000.0, "elapsed": 50.0});
        assert_eq!(import_to_follow(&import, 1050.0 + AFTER_IMPORT), Some(1000.0));
        assert_eq!(import_to_follow(&import, 1050.0 + AFTER_IMPORT - 1.0), None, "its result is still on the page");
        for (key, value) in [
            ("running", json!(true)),
            ("stage", json!("Stopped")),
            ("stage", json!("Failed")),
            ("new", json!(0)),
            ("folder", json!(JOB)),
            ("folder", json!("photo descriptions")),
            ("folder", json!("possible faces")),
            ("folder", json!("")),
        ] {
            let mut not = import.clone();
            not[key] = value;
            assert_eq!(import_to_follow(&not, 5000.0), None, "{key}: {not}");
        }
        assert_eq!(import_to_follow(&json!({"running": false}), 5000.0), None, "nothing ran yet");
    }

    #[test]
    fn what_is_no_pictures_time_is_no_time() {
        assert_eq!(a_time(Some(1_500_000_000.5)), Some(1_500_000_000_500));
        assert_eq!(a_time(Some(-86_400.0)), Some(-86_400_000), "a photo from before 1970");
        for not in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-1e13), Some(1e13)] {
            assert_eq!(a_time(not), None, "{not:?}");
        }
        assert_eq!(top_look(&json!({"happy": 0.2, "sad": 0.5, "neutral": 0.3})), Some("sad"));
        assert_eq!(top_look(&json!({})), None);
        assert_eq!(top_look(&json!("happy")), None);
    }
}
