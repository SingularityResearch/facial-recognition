//! The first-run question (the owner's rule, 2026-11-20: nothing may be assumed to be installed on a user's
//! computer; what an app needs is brought by its installer, or the app asks at its first run, before the
//! feature is needed). What the app is missing is asked about in the app's own page, before the tab page is
//! shown and before either program starts, and fetched on a yes: each file pinned by size and SHA-256
//! (`face_tools`), into the folder beside the app.
//!
//! A "not now" is remembered for what only one feature needs (videos, HEIC photos, the tracker page's
//! cues), so that the question isn't asked again at every start; what the programs can't work without (the
//! model runtime, the models that aren't optional) is asked about every time it is missing.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use face_tools::first_run::{Missing, fetch_with, missing};
use serde_json::{Value, json};

/// the page and its script (they are part of the program, not of `web/`)
pub const PAGE: &str = include_str!("../pages/first_run.html");
pub const SCRIPT: &str = include_str!("../pages/first_run.js");
/// in the private folder: what was answered with "not now"
const KEPT: &str = "first_run.json";

/// "asking": the question waits; "getting": being fetched; "failed": the fetch stopped, with why; "done":
/// answered, the app starts.
struct State {
    stage: &'static str,
    lines: Vec<String>,
    megabytes: f64,
    needed: bool,
    step: String,
    done: u64,
    total: u64,
    error: String,
}

/// The question of one start of the app.
pub struct FirstRun {
    root: PathBuf,
    home: PathBuf,
    state: Mutex<State>,
}

fn held<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The names answered with "not now" before (`Missing::names`).
fn declined(home: &Path) -> Vec<String> {
    let kept: Option<Value> = std::fs::read(home.join(KEPT)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let names = kept.as_ref().and_then(|k| k["declined"].as_array()).cloned().unwrap_or_default();
    names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()
}

/// Whether there is something to ask at this start: something that can be fetched is missing, and it is
/// needed or wasn't all declined before.
pub fn asks(m: &Missing, declined: &[String]) -> bool {
    m.fetchable() && (m.needed() || !m.names().iter().all(|name| declined.contains(name)))
}

impl FirstRun {
    /// The question for this start, or None when there is nothing to ask.
    pub fn due(root: &Path, home: &Path) -> Option<Arc<FirstRun>> {
        let m = missing(root, true);
        if !asks(&m, &declined(home)) {
            return None;
        }
        let state = State {
            stage: "asking",
            lines: m.lines(),
            megabytes: m.megabytes(),
            needed: m.needed(),
            step: String::new(),
            done: 0,
            total: m.bytes(),
            error: String::new(),
        };
        Some(Arc::new(FirstRun { root: root.to_path_buf(), home: home.to_path_buf(), state: Mutex::new(state) }))
    }

    /// The question hasn't been answered yet, or what it asked for is still being fetched.
    pub fn waiting(&self) -> bool {
        held(&self.state).stage != "done"
    }

    /// What the page shows.
    pub fn said(&self) -> Value {
        let st = held(&self.state);
        json!({"stage": st.stage, "lines": st.lines, "megabytes": st.megabytes, "needed": st.needed, "step": st.step,
               "done": st.done, "total": st.total, "error": st.error, "folder": self.root.display().to_string()})
    }

    /// The answer: fetch it all (`get`), or start without. `then` runs once the app may start: at once for a
    /// "not now", after the last file for a yes, and not at all when the fetch fails (the page then asks
    /// again). An answer while something is being fetched, or after the question is done, changes nothing.
    pub fn answer(self: &Arc<Self>, get: bool, then: Box<dyn FnOnce() + Send>) {
        {
            let mut st = held(&self.state);
            if st.stage == "getting" || st.stage == "done" {
                return;
            }
            (st.stage, st.error) = (if get { "getting" } else { "done" }, String::new());
        }
        if !get {
            // remembered, so that what only one feature needs isn't asked about at every start
            let names = missing(&self.root, true).names();
            let text = face_files::filejson::dumps(&json!({"declined": names}));
            let _ = face_files::paths::atomic_write(&self.home.join(KEPT), text.as_bytes());
            println!("First run: started without what is missing.");
            return then();
        }
        let first = self.clone();
        let _ = std::thread::Builder::new().name("first run".into()).spawn(move || {
            let m = missing(&first.root, true);
            {
                let mut st = held(&first.state);
                (st.done, st.total) = (0, m.bytes());
            }
            let fetched = fetch_with(
                &first.root,
                &m,
                &mut |line| {
                    println!("First run: getting {line} ...");
                    held(&first.state).step = line.to_string();
                },
                &mut |done, total| {
                    let mut st = held(&first.state);
                    (st.done, st.total) = (done, total);
                },
            );
            match fetched {
                Ok(()) => {
                    println!("First run: everything is here.");
                    held(&first.state).stage = "done";
                    then();
                }
                Err(why) => {
                    println!("First run: not everything could be fetched: {why}");
                    let mut st = held(&first.state);
                    (st.stage, st.error) = ("failed", why);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use face_tools::models::MODELS;

    use super::*;

    #[test]
    fn the_question_is_asked_for_what_is_needed_every_time_and_for_the_rest_until_declined() {
        let needed = MODELS.iter().find(|m| !m.optional).copied().unwrap();
        let nothing: Vec<String> = Vec::new();
        // nothing missing, or only what can't be had here: nothing to ask
        assert!(!asks(&Missing::default(), &nothing));
        assert!(!asks(&Missing { not_here: vec!["FFmpeg".into()], ..Missing::default() }, &nothing));
        // something for one feature: asked, until that very thing was declined
        let feature = Missing { parts: vec!["ffmpeg", "mediapipe"], ..Missing::default() };
        assert!(asks(&feature, &nothing));
        assert!(asks(&feature, &["ffmpeg".to_string()]));
        assert!(!asks(&feature, &["mediapipe".to_string(), "ffmpeg".to_string(), "heif".to_string()]));
        // what the programs can't work without: every time, whatever was declined
        let runtime = Missing { parts: vec!["runtime"], ..Missing::default() };
        assert!(asks(&runtime, &["runtime".to_string()]));
        let model = Missing { models: vec![needed], ..Missing::default() };
        assert!(asks(&model, &model.names()));
    }

    #[test]
    fn a_not_now_is_kept_in_the_private_folder_and_read_again() {
        let home = std::env::temp_dir().join(format!("face-firstrun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(declined(&home).is_empty());
        std::fs::write(home.join(KEPT), br#"{"declined": ["ffmpeg", "model:x.onnx", 7]}"#).unwrap();
        assert_eq!(declined(&home), ["ffmpeg", "model:x.onnx"]);
        // a file that isn't what it should be counts as nothing declined
        std::fs::write(home.join(KEPT), b"[1, 2").unwrap();
        assert!(declined(&home).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
