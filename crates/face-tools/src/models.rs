//! The face models, downloaded when they are missing: the list, its sources and its rules.
//! Each model comes from its original publisher when that offers the very same file
//! (OpenCV Zoo on Hugging Face at pinned revisions, Google's MediaPipe model storage), else, or when that fails
//! or the file there has changed, from the project's own release "models-v2". A file is used only with its size and
//! SHA-256; one already there is checked the same way, once, and then remembered by its size and time
//! (`models/.verified.json`).
//!
//! Every model here is licensed for selling the app with it (`NOTICE` names each). The recognizer and the
//! voice model are the project's own.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use face_files::{filejson, paths};
use serde_json::{Map, Value, json};

use crate::{
    fetch::{Failure, Fetched, Net, save, sha256_of},
    zip,
};

/// Where a model's publisher has the same file.
#[derive(Debug, Clone, Copy)]
pub enum Origin {
    /// only the project's own release has it (made by the project's tools)
    Ours,
    At(&'static str),
    /// (an archive's address, the file in it): only that file's bytes are downloaded
    InZip(&'static str, &'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub origin: Origin,
    /// the programs work without it (another way is used instead): when it can't be had it is passed over
    /// with a note, and tried again the next time
    pub optional: bool,
}

const fn model(name: &'static str, size: u64, sha256: &'static str, origin: Origin, optional: bool) -> Model {
    Model { name, size, sha256, origin, optional }
}

/// The project's own release of the same files. Change the tag and the list together when a model changes.
pub const RELEASE: &str = "https://github.com/SingularityResearch/facial-recognition/releases/download/models-v2/";
/// The same release through GitHub's API: other servers, used when the download links fail.
pub const RELEASE_LIST: &str =
    "https://api.github.com/repos/SingularityResearch/facial-recognition/releases/tags/models-v2";
/// attempts at the project's release per file; seconds before attempt n is `WAIT * n`
pub const TRIES: u64 = 4;
pub const WAIT: u64 = 3;

/// The models the project publishes itself, in its models release: its own two, and YAMNet as ONNX.
pub const OWN: [&str; 3] = ["yamnet.onnx", "opsis_1.onnx", "melos_1.onnx"];

pub const MODELS: [Model; 11] = [
    model(
        "face_detection_yunet_2023mar.onnx",
        232_589,
        "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
        Origin::At(
            "https://huggingface.co/opencv/face_detection_yunet/resolve/3cc26e7f1014a5ee5d74a42acee58bafc9d0a310/face_detection_yunet_2023mar.onnx",
        ),
        false,
    ),
    model(
        "ediffiqa_tiny_jun2024.onnx",
        7_272_678,
        "9426c899cc0f01665240cb7d9e7f98e18e24e456c178326c771a43da289bfc6a",
        Origin::At(
            "https://huggingface.co/opencv/face_image_quality_assessment_ediffiqa/resolve/4fce5f6a5438b55a9f996a4e02164b63b849c587/ediffiqa_tiny_jun2024.onnx",
        ),
        false,
    ),
    model(
        "facial_expression_recognition_mobilefacenet_2022july.onnx",
        4_791_892,
        "4f61307602fc089ce20488a31d4e4614e3c9753a7d6c41578c854858b183e1a9",
        Origin::At(
            "https://huggingface.co/opencv/facial_expression_recognition/resolve/b473270576eabddaacffd08adb4bed27b3fc54e6/facial_expression_recognition_mobilefacenet_2022july.onnx",
        ),
        false,
    ),
    model(
        "face_landmarker.task",
        3_758_596,
        "64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff",
        Origin::At(
            "https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task",
        ),
        false,
    ),
    model(
        "pose_landmarker_lite.task",
        5_777_746,
        "59929e1d1ee95287735ddd833b19cf4ac46d29bc7afddbbf6753c459690d574a",
        Origin::At(
            "https://storage.googleapis.com/mediapipe-models/pose_landmarker/pose_landmarker_lite/float16/1/pose_landmarker_lite.task",
        ),
        false,
    ),
    // fingers, for drawing only: without it the Hand skeleton option is greyed out
    model(
        "hand_landmarker.task",
        7_819_105,
        "fbc2a30080c3c557093b5ddfc334698132eb341044ccee322ccf8bcf3607cde1",
        Origin::At(
            "https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task",
        ),
        true,
    ),
    // YAMNet (Google, Apache 2.0): 521 kinds of sound; without it a room's sound isn't named
    model(
        "yamnet.tflite",
        4_126_810,
        "4d8b4a53282dc83ef04e3e7dbc4fbc98082e34e44ed798e16c3a0cdd4c584faf",
        Origin::At("https://storage.googleapis.com/mediapipe-models/audio_classifier/yamnet/float32/1/yamnet.tflite"),
        true,
    ),
    // YuNet with any input size, for face finding on a graphics processor; without it faces are found on
    // the processor
    model(
        "face_detection_yunet_2023mar_dynamic.onnx",
        229_738,
        "ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0",
        Origin::Ours,
        true,
    ),
    // YAMNet as ONNX, for these programs' model runtime, which has no MediaPipe: made from yamnet.tflite as
    // it is and kept in the project's release; without it a
    // room's sound isn't named
    model(
        "yamnet.onnx",
        5_077_452,
        "3aea11cc647810eba0e3fcc5e543d1ce877b68b3458b07a7a49bce102447af41",
        Origin::Ours,
        true,
    ),
    // Opsis 1, the recognizer: the project's own (`face-core` `REC_MODELS`), trained by the project from OpenCV
    // Zoo's SFace (Apache 2.0) and kept in the project's release. A new recognizer is a new file with a new name.
    model(
        "opsis_1.onnx",
        38_555_416,
        "5359382c3cf006fa70345946cff930fa6baa11c6ccdd85caf633230f4adefff2",
        Origin::Ours,
        false,
    ),
    // Melos 1, the voice model (arousal, dominance, valence): the project's own (`face-core` `voice.rs`),
    // trained by the project on what listeners heard in CREMA-D's clips (Open Database License 1.0)
    model(
        "melos_1.onnx",
        1_270_687,
        "107eadefc77f2c922dabf7a81eba33009c6bb364dfdce42509d90165bdb641ed",
        Origin::Ours,
        false,
    ),
];

const LFS_SIGN: &[u8] = b"version https://git-lfs";
/// in `models/`: {file: {size, mtime_ns, sha256}} of files whose SHA-256 was checked
const VERIFIED: &str = ".verified.json";

/// True for a Git LFS placeholder: a small text file standing in for a file that wasn't downloaded.
pub fn is_placeholder(path: &Path) -> bool {
    let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= 1024);
    small && std::fs::read(path).is_ok_and(|b| b.starts_with(LFS_SIGN))
}

/// A folder of models and what is known of its files.
pub struct Folder {
    pub at: PathBuf,
}

impl Folder {
    pub fn new(root: &Path) -> Folder {
        Folder { at: root.join("models") }
    }

    fn verified(&self) -> Map<String, Value> {
        let read = std::fs::read(self.at.join(VERIFIED)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        read.and_then(|v| v.as_object().cloned()).unwrap_or_default()
    }

    /// (size, time of the last change in nanoseconds) of a file, as `.verified.json` notes them.
    fn state(&self, name: &str) -> Option<(u64, u128)> {
        let m = std::fs::metadata(self.at.join(name)).ok()?;
        Some((m.len(), m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos()))
    }

    /// Note that `name`, as it is now, has this SHA-256. (A folder that can't be written: checked again next time.)
    fn remember(&self, name: &str, sha256: &str) {
        let Some((size, mtime)) = self.state(name) else { return };
        let mut all = self.verified();
        all.insert(name.to_string(), json!({"size": size, "mtime_ns": mtime as u64, "sha256": sha256}));
        let _ = paths::atomic_write(&self.at.join(VERIFIED), filejson::dumps_indent1(&Value::Object(all)).as_bytes());
    }

    /// True if the file is the expected one: its size and SHA-256. The SHA-256 is worked out only when the
    /// file's size or time differ from the last check.
    pub fn is_good(&self, m: &Model) -> bool {
        let path = self.at.join(m.name);
        let Some((size, mtime)) = self.state(m.name) else { return false };
        if size != m.size || is_placeholder(&path) {
            return false;
        }
        let seen = self.verified();
        let seen = seen.get(m.name);
        let said = |key: &str| seen.and_then(|s| s.get(key));
        if said("size").and_then(Value::as_u64) == Some(size)
            && said("mtime_ns").and_then(Value::as_u64) == Some(mtime as u64)
            && said("sha256").and_then(Value::as_str) == Some(m.sha256)
        {
            return true;
        }
        let good = sha256_of(&path).is_ok_and(|got| got == m.sha256);
        if good {
            self.remember(m.name, m.sha256);
        }
        good
    }

    /// The models that are missing, placeholders, or not the expected file.
    pub fn needed(&self, of: &[Model]) -> Vec<Model> {
        of.iter().filter(|m| !self.is_good(m)).copied().collect()
    }
}

/// Where the project's own release has its files, for the run.
pub struct Release {
    pub base: String,
    pub list: String,
    /// seconds before attempt n is this times n (tests: none)
    pub wait: u64,
    by_api: Option<HashMap<String, String>>,
}

impl Default for Release {
    fn default() -> Release {
        Release { base: RELEASE.to_string(), list: RELEASE_LIST.to_string(), wait: WAIT, by_api: None }
    }
}

impl Release {
    pub fn at(base: &str, list: &str) -> Release {
        Release { base: base.to_string(), list: list.to_string(), wait: 0, by_api: None }
    }

    /// (address, the kind of answer to ask for) for an attempt at the release: odd attempts use the download
    /// link, even ones GitHub's API for the same file (the link again if its listing can't be read).
    fn source(&mut self, net: &Net, name: &str, attempt: u64) -> (String, Option<&'static str>) {
        if attempt.is_multiple_of(2) {
            if self.by_api.is_none() {
                let listed = net.text(&self.list, 1 << 20).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
                let urls = listed.as_ref().and_then(|v| v["assets"].as_array()).map(|assets| {
                    let pair = |a: &Value| Some((a["name"].as_str()?.to_string(), a["url"].as_str()?.to_string()));
                    assets.iter().filter_map(pair).collect::<HashMap<_, _>>()
                });
                if urls.is_some() {
                    self.by_api = urls;
                }
            }
            if let Some(url) = self.by_api.as_ref().and_then(|u| u.get(name)) {
                return (url.clone(), Some("application/octet-stream"));
            }
        }
        (format!("{}{name}", self.base), None)
    }
}

/// One attempt at one model: 0 is its publisher's copy, the others the project's release.
fn attempt(
    net: &Net,
    release: &mut Release,
    folder: &Folder,
    m: &Model,
    n: u64,
    progress: &mut dyn FnMut(u64),
) -> Fetched<()> {
    let dest = folder.at.join(m.name);
    match (n, m.origin) {
        (0, Origin::InZip(url, member)) => {
            let archive = zip::OnServer { net, url };
            let listed = zip::entries(&archive)?;
            save(zip::open(&archive, zip::find(&listed, member)?)?, &dest, m.size, m.sha256, progress)
        }
        (0, Origin::At(url)) => save(net.open(url, None, None)?, &dest, m.size, m.sha256, progress),
        _ => {
            let (url, accept) = release.source(net, m.name, n);
            save(net.open(&url, None, accept)?, &dest, m.size, m.sha256, progress)
        }
    }
}

/// Download every model of `of` that is needed. `progress(bytes done, bytes in all, the file's name)` as it
/// goes; `note` gets a line for an optional file that was passed over. The names downloaded; an error (with
/// the reason) when one that isn't optional can't be had.
pub fn fetch(
    net: &Net,
    release: &mut Release,
    folder: &Folder,
    of: &[Model],
    progress: &mut dyn FnMut(u64, u64, &str),
    note: &mut dyn FnMut(String),
) -> Result<Vec<&'static str>, String> {
    let todo = folder.needed(of);
    let total: u64 = todo.iter().map(|m| m.size).sum();
    let (mut done, mut got) = (0u64, Vec::new());
    std::fs::create_dir_all(&folder.at).map_err(|e| e.to_string())?;
    for m in &todo {
        let first = if matches!(m.origin, Origin::Ours) { 1 } else { 0 };
        for n in first..=TRIES {
            let tried = attempt(net, release, folder, m, n, &mut |bytes| progress(done + bytes, total, m.name));
            match tried {
                Ok(()) => {
                    folder.remember(m.name, m.sha256);
                    done += m.size;
                    got.push(m.name);
                    break;
                }
                // the publisher's copy: any failure, a changed file too, falls through to the release
                Err(_) if n == 0 => progress(done, total, m.name),
                // the release's servers sometimes answer with an error or drop a large download midway:
                // tried again after a pause. A file that isn't there won't appear.
                Err(Failure { why, lasting }) => {
                    progress(done, total, m.name);
                    if lasting || n == TRIES {
                        if m.optional {
                            note(format!("(Skipped {}, which is optional: {why})", m.name));
                            break;
                        }
                        return Err(format!("Couldn't download {} from {}: {why}", m.name, release.base));
                    }
                    std::thread::sleep(Duration::from_secs(release.wait * n));
                }
            }
        }
    }
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::{Server, folder, sha},
        zip::tests::archive,
    };

    fn bytes(seed: u32, n: usize) -> Vec<u8> {
        (0..n as u32).map(|v| (v.wrapping_mul(2_654_435_761).wrapping_add(seed) >> 13) as u8).collect()
    }

    fn a_model(name: &'static str, data: &[u8], origin: Origin, optional: bool) -> Model {
        Model { name, size: data.len() as u64, sha256: sha(data), origin, optional }
    }

    #[test]
    fn models_come_from_their_publishers_or_the_projects_release() {
        let (server, root) = (Server::start(), folder("models"));
        let (net, dir) = (Net::new(), Folder::new(&root));
        let (a, b, c, d, e) = (bytes(1, 5_000), bytes(2, 70_000), bytes(3, 900), bytes(4, 2_000), bytes(5, 100));
        // a: its publisher has it. b: inside an archive at its publisher's. c: its publisher's copy has
        // changed, the release has the right one. d: only the release has it, which fails twice first.
        // e: optional, and nowhere.
        let zip =
            archive(&[("other.onnx", &bytes(9, 30_000), true), ("b_inside.onnx", &b, true), ("last", b"x", false)]);
        let of = [
            a_model("a.onnx", &a, Origin::At(server.put("/origin/a.onnx", &a)), false),
            a_model("b.onnx", &b, Origin::InZip(server.put("/origin/pack.zip", &zip), "b_inside.onnx"), false),
            a_model("c.onnx", &c, Origin::At(server.put("/origin/c.onnx", b"another file now")), false),
            a_model("d.onnx", &d, Origin::Ours, false),
            a_model("e.onnx", &e, Origin::At(server.url("/origin/e.onnx")), true),
        ];
        server.put("/release/c.onnx", &c);
        server.put("/release/d.onnx", &d);
        server.put("/asset/d", &d);
        server.fail("/release/d.onnx", 502, 1);
        server.fail("/asset/d", 500, 1);
        let listing = json!({"assets": [{"name": "d.onnx", "url": server.url("/asset/d")}]}).to_string();
        server.put("/list", listing.as_bytes());
        let mut release = Release::at(server.url("/release/"), server.url("/list"));
        let (mut notes, mut last) = (Vec::new(), (0, 0));
        let got =
            fetch(&net, &mut release, &dir, &of, &mut |done, total, _| last = (done, total), &mut |n| notes.push(n))
                .unwrap();
        assert_eq!(got, ["a.onnx", "b.onnx", "c.onnx", "d.onnx"]);
        for (name, data) in [("a.onnx", &a), ("b.onnx", &b), ("c.onnx", &c), ("d.onnx", &d)] {
            assert_eq!(&std::fs::read(dir.at.join(name)).unwrap(), data, "{name}");
        }
        assert!(
            notes.len() == 1
                && notes[0].starts_with("(Skipped e.onnx, which is optional:")
                && !dir.at.join("e.onnx").exists()
        );
        assert_eq!(last.1, (a.len() + b.len() + c.len() + d.len() + e.len()) as u64);
        // the archive was never fetched whole: its size asked, then parts
        let pack = server.asked("/origin/pack.zip");
        assert!(pack.iter().all(|(method, part, _)| method == "HEAD" || part.is_some()) && pack.len() >= 3, "{pack:?}");
        // d: the link (502), the API's address with the kind of answer it wants (500), the link again
        assert_eq!(server.asked("/release/d.onnx").len(), 2);
        assert_eq!(server.asked("/asset/d"), [("GET".to_string(), None, Some("application/octet-stream".to_string()))]);
        // nothing half-done is left, and what was checked is remembered in `.verified.json`
        let names: Vec<String> = std::fs::read_dir(&dir.at)
            .unwrap()
            .map(|f| f.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".download")), "{names:?}");
        let kept: Value = serde_json::from_slice(&std::fs::read(dir.at.join(VERIFIED)).unwrap()).unwrap();
        assert_eq!(
            (kept["a.onnx"]["size"].as_u64(), kept["a.onnx"]["sha256"].as_str()),
            (Some(5_000), Some(of[0].sha256))
        );
        assert!(kept["a.onnx"]["mtime_ns"].as_u64().unwrap() > 1_600_000_000_000_000_000);
        // all there now: nothing is needed but the optional one, and nothing is asked for again
        let before = server.asked("/origin/a.onnx").len();
        assert_eq!(dir.needed(&of).iter().map(|m| m.name).collect::<Vec<_>>(), ["e.onnx"]);
        assert_eq!(server.asked("/origin/a.onnx").len(), before);
        // a file changed behind the note's back (the same size, another time): found out
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(dir.at.join("a.onnx"), vec![0u8; a.len()]).unwrap();
        assert!(!dir.is_good(&of[0]) && dir.is_good(&of[1]));
        // a placeholder of Git LFS stands for a file that isn't there
        let lfs = b"version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 5\n";
        std::fs::write(dir.at.join("c.onnx"), lfs).unwrap();
        assert!(
            is_placeholder(&dir.at.join("c.onnx")) && !dir.is_good(&of[2]) && !is_placeholder(&dir.at.join("b.onnx"))
        );
        // one that isn't optional and is nowhere: said, with where it was looked for
        let gone = [a_model("f.onnx", &e, Origin::Ours, false)];
        let why = fetch(&net, &mut release, &dir, &gone, &mut |_, _, _| (), &mut |_| ()).unwrap_err();
        assert!(
            why.starts_with("Couldn't download f.onnx from ") && server.asked("/release/f.onnx").len() == 1,
            "{why}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_list_holds_what_must_hold_of_any_list() {
        // (each name once, and for each a size and a sum that can be one)
        let mut names: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MODELS.len());
        assert!(MODELS.iter().all(|m| m.sha256.len() == 64 && m.size > 0));
        assert_eq!(
            MODELS.iter().filter(|m| m.optional).map(|m| m.name).collect::<Vec<_>>(),
            ["hand_landmarker.task", "yamnet.tflite", "face_detection_yunet_2023mar_dynamic.onnx", "yamnet.onnx"]
        );
        // a model of these programs' own is one of the list, comes from the project's release, and is last
        assert!(OWN.iter().all(|own| MODELS.iter().any(|m| m.name == *own && matches!(m.origin, Origin::Ours))));
        assert!(MODELS.iter().rev().take(OWN.len()).all(|m| OWN.contains(&m.name)));
        assert!(MODELS.iter().all(|m| match m.origin {
            Origin::At(url) | Origin::InZip(url, _) => url.starts_with("https://"),
            Origin::Ours => true,
        }));
    }
}
