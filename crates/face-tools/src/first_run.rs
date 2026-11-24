//! What a program is missing when it starts, said up front: nothing may be assumed to be installed, and a
//! program that isn't brought by an installer (which packs all of it) asks before its first run for what it
//! needs, never fails later at the photo or video that needed it (the owner's rule, 2026-11-20).

use std::path::Path;

use crate::{
    fetch::Net,
    models::{self, Folder, MODELS, Model, Origin, Release},
    parts,
};

/// What isn't there. `parts`: of "runtime", "mediapipe", "ffmpeg", "heif".
#[derive(Default)]
pub struct Missing {
    pub models: Vec<Model>,
    pub parts: Vec<&'static str>,
    /// what can't be had by this program on this computer, each with the reason
    pub not_here: Vec<String>,
}

impl Missing {
    pub fn is_empty(&self) -> bool {
        self.models.is_empty() && self.parts.is_empty() && self.not_here.is_empty()
    }

    /// Whether any of it can be fetched (what is only `not_here` can't).
    pub fn fetchable(&self) -> bool {
        !(self.models.is_empty() && self.parts.is_empty())
    }

    /// Whether the programs can't do their work without some of it: the model runtime, or a model that isn't
    /// optional. The rest is for one feature each (videos, HEIC photos, the tracker page's cues).
    pub fn needed(&self) -> bool {
        self.parts.contains(&"runtime") || self.models.iter().any(|m| !m.optional)
    }

    /// A name for each thing that can be fetched ("runtime", "model:<file>"), for remembering a "not now".
    pub fn names(&self) -> Vec<String> {
        let models = self.models.iter().map(|m| format!("model:{}", m.name));
        self.parts.iter().map(|p| p.to_string()).chain(models).collect()
    }

    /// Bytes to download for all of it.
    pub fn bytes(&self) -> u64 {
        self.models.iter().map(|m| m.size).sum::<u64>() + self.parts.iter().map(|p| part_size(p)).sum::<u64>()
    }

    /// Megabytes to download for all of it.
    pub fn megabytes(&self) -> f64 {
        self.bytes() as f64 / 1e6
    }

    /// What is missing, a line each, in a user's words.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.models.is_empty() {
            let optional = self.models.iter().filter(|m| m.optional).count();
            let more = if optional > 0 { format!(" ({optional} of them optional)") } else { String::new() };
            out.push(format!("{} face models{more}", self.models.len()));
        }
        for part in &self.parts {
            out.push(
                match *part {
                    "runtime" => "the model runtime (ONNX Runtime), without which no face is found",
                    "mediapipe" => "MediaPipe's runtime for the tracker page (face-shape cues, posture, hands)",
                    "ffmpeg" => "FFmpeg, without which videos can't be read",
                    _ => "the HEIC libraries, without which an iPhone's HEIC photos can't be read (built here: some minutes)",
                }
                .to_string(),
            );
        }
        out.extend(self.not_here.iter().cloned());
        out
    }
}

/// Bytes to download for a part.
fn part_size(name: &str) -> u64 {
    let system = parts::system();
    match name {
        "runtime" => parts::runtime(system).map_or(0, |p| p.pin.size),
        "ffmpeg" => parts::ffmpeg(system).map_or(0, |p| p.pin.size),
        "mediapipe" => parts::MEDIAPIPE.size,
        "heif" => parts::DE265.size + parts::HEIF.size,
        _ => 0,
    }
}

/// An address that may stand in for the project's release in a check (`FACE_TEST_RELEASE`): this computer's
/// own, by number, with a port, and nothing else. Every file is still kept only when it is the pinned one.
fn stand_in(said: Option<&str>) -> Option<String> {
    let port = said?.strip_prefix("http://127.0.0.1:")?;
    (!port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())).then(|| said.unwrap_or_default().to_string())
}

/// What a program started with this root is missing. `photos`: it reads photos and videos (the labeling
/// tool), so FFmpeg and the HEIC libraries count. What the program runs with counts as there in
/// `<root>/runtime` (fetched) and beside the program itself (brought by an installer): the two places the
/// programs look in.
pub fn missing(root: &Path, photos: bool) -> Missing {
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    missing_with(root, beside.as_deref(), photos)
}

/// Whether `there` holds for the folder what is fetched goes into, or for the program's own folder.
fn in_either(runtime: &Path, beside: Option<&Path>, there: impl Fn(&Path) -> bool) -> bool {
    there(runtime) || beside.is_some_and(there)
}

/// `missing`, with the program's own folder named.
pub fn missing_with(root: &Path, beside: Option<&Path>, photos: bool) -> Missing {
    let (runtime, system) = (root.join("runtime"), parts::system());
    // fetched: the pinned files. Beside the program: by their names, since an installer's copy is part of
    // the app and no longer the fetched bytes (signing a Mac's app signs each library inside it)
    let here = |files: &[parts::Kept]| {
        parts::have(files, &runtime)
            || beside.is_some_and(|folder| files.iter().all(|(_, name, _, _)| folder.join(name).is_file()))
    };
    let mut m = Missing { models: Folder::new(root).needed(&MODELS), ..Missing::default() };
    let mut packed = |name: &'static str, what: &str, package: Option<parts::Package>| match package {
        Some(p) if !here(p.files) => m.parts.push(name),
        Some(_) => {}
        None => m.not_here.push(format!("{what}: there is no package of it for this kind of computer ({system}) yet")),
    };
    packed("runtime", "the model runtime", parts::runtime(system));
    if photos {
        packed("ffmpeg", "FFmpeg (videos)", parts::ffmpeg(system));
    }
    if !parts::have(&parts::MEDIAPIPE_FILES, &root.join("models").join("mediapipe-web")) {
        m.parts.push("mediapipe");
    }
    let heif = |folder: &Path| parts::heif_names().iter().all(|(_, name)| folder.join(name).is_file());
    if photos && !in_either(&runtime, beside, heif) {
        // built from source: only where there is something to build with (an installer brings them built)
        if std::process::Command::new("cmake").arg("--version").output().is_ok_and(|o| o.status.success()) {
            m.parts.push("heif");
        } else {
            m.not_here.push(
                "the HEIC libraries (an iPhone's HEIC photos): they come with the installer; this computer has no CMake to build them with".to_string(),
            );
        }
    }
    m
}

/// Fetch what is missing. `say` gets a line for each step.
pub fn fetch(root: &Path, m: &Missing, say: &mut dyn FnMut(&str)) -> Result<(), String> {
    fetch_with(root, m, say, &mut |_, _| ())
}

/// Fetch what is missing. `say` gets a line for each step, `progress` the bytes downloaded so far and in
/// all, as it goes.
pub fn fetch_with(
    root: &Path,
    m: &Missing,
    say: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let (runtime, system, total) = (root.join("runtime"), parts::system(), m.bytes());
    // (a check: the models from a stand-in on this computer, over plain HTTP, and nothing else fetched)
    let stand_in = stand_in(std::env::var("FACE_TEST_RELEASE").ok().as_deref());
    let net = if stand_in.is_some() { Net::plain() } else { Net::new() };
    let mut before = 0u64;
    if !m.models.is_empty() {
        let size = m.models.iter().map(|x| x.size).sum::<u64>();
        say(&format!("the face models ({:.0} MB)", size as f64 / 1e6));
        let (mut release, of) = match &stand_in {
            Some(base) => {
                let ours: Vec<Model> = MODELS.iter().map(|m| Model { origin: Origin::Ours, ..*m }).collect();
                (Release::at(&format!("{base}/"), &format!("{base}/list")), ours)
            }
            None => (Release::default(), MODELS.to_vec()),
        };
        let mut notes = Vec::new();
        let got = models::fetch(
            &net,
            &mut release,
            &Folder::new(root),
            &of,
            &mut |done, _, _| progress(before + done, total),
            &mut |n| notes.push(n),
        );
        notes.iter().for_each(|n| say(n));
        got?;
        before += size;
        progress(before, total);
    }
    if stand_in.is_some() && !m.parts.is_empty() {
        return Err("a check brings everything but the models itself: nothing else is fetched from a stand-in".into());
    }
    for part in &m.parts {
        let mut step = |got: u64, _: u64| progress(before + got, total);
        let done = match *part {
            "runtime" => {
                say("the model runtime");
                let package = parts::runtime(system).ok_or("no model runtime for this kind of computer")?;
                parts::get(&net, &package, &runtime, &mut step)
                    .and_then(|_| parts::get_vc_runtime(&runtime))
                    .map(|_| ())
            }
            "ffmpeg" => {
                say("FFmpeg");
                let package = parts::ffmpeg(system).ok_or("no FFmpeg for this kind of computer")?;
                parts::get(&net, &package, &runtime, &mut step).map(|_| ())
            }
            "mediapipe" => {
                say("MediaPipe's runtime for the tracker page");
                parts::get_mediapipe(&net, &root.join("models"), &mut step).map(|_| ())
            }
            _ => parts::get_heif(&net, &runtime, say).map(|_| ()),
        };
        done.map_err(|e| e.why)?;
        before += part_size(part);
        progress(before, total);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_computer_by_number_may_stand_in_for_the_release() {
        assert_eq!(stand_in(Some("http://127.0.0.1:8123")).as_deref(), Some("http://127.0.0.1:8123"));
        for other in [
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://127.0.0.1:81/x",
            "http://127.0.0.1:81@example.org",
            "https://127.0.0.1:81",
            "http://localhost:81",
            "http://example.org:81",
            "",
        ] {
            assert_eq!(stand_in(Some(other)), None, "{other}");
        }
        assert_eq!(stand_in(None), None);
    }

    #[test]
    fn what_an_installer_put_beside_the_program_counts_as_there() {
        let dir = std::env::temp_dir().join(format!("face-first-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (runtime, beside) = (dir.join("root").join("runtime"), dir.join("program"));
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(&beside).unwrap();
        let there = |folder: &Path| folder.join("library").is_file();
        // nowhere
        assert!(!in_either(&runtime, Some(&beside), there) && !in_either(&runtime, None, there));
        // beside the program only: there when the program's folder is known
        std::fs::write(beside.join("library"), b"x").unwrap();
        assert!(in_either(&runtime, Some(&beside), there) && !in_either(&runtime, None, there));
        // where it is fetched to
        std::fs::write(runtime.join("library"), b"x").unwrap();
        assert!(in_either(&runtime, None, there));
        let _ = std::fs::remove_dir_all(&dir);
        // a folder with nothing fetched and nothing beside it lacks the runtime; with files of the runtime's
        // names beside the program (an installer's copy, signed with the app: other bytes than fetched) it
        // doesn't, while the same files where fetched ones go are not the pinned ones and don't count
        let Some(package) = parts::runtime(parts::system()) else { return };
        let (root, program) = (dir.join("root"), dir.join("program"));
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        std::fs::create_dir_all(&program).unwrap();
        assert!(missing_with(&root, Some(&program), false).parts.contains(&"runtime"));
        for (_, name, _, _) in package.files {
            std::fs::write(root.join("runtime").join(name), b"not the pinned file").unwrap();
        }
        assert!(missing_with(&root, Some(&program), false).parts.contains(&"runtime"));
        for (_, name, _, _) in package.files {
            std::fs::write(program.join(name), b"the app's own copy").unwrap();
        }
        assert!(!missing_with(&root, Some(&program), false).parts.contains(&"runtime"));
        assert!(missing_with(&root, None, false).parts.contains(&"runtime"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_missing_says_whether_the_programs_need_it_and_names_each_thing() {
        let optional = MODELS.iter().find(|m| m.optional).copied();
        let needed = MODELS.iter().find(|m| !m.optional).copied().unwrap();
        let nothing = Missing::default();
        assert!(!nothing.fetchable() && !nothing.needed() && nothing.names().is_empty() && nothing.bytes() == 0);
        // only something for one feature: can be fetched, isn't needed
        let feature =
            Missing { parts: vec!["mediapipe"], models: optional.into_iter().collect(), ..Missing::default() };
        assert!(feature.fetchable() && !feature.needed());
        assert_eq!(feature.names()[0], "mediapipe");
        assert!(feature.bytes() >= parts::MEDIAPIPE.size);
        // the runtime, or a model the programs can't do without
        assert!(Missing { parts: vec!["runtime"], ..Missing::default() }.needed());
        let model = Missing { models: vec![needed], ..Missing::default() };
        assert!(model.needed() && model.bytes() == needed.size);
        assert_eq!(model.names(), [format!("model:{}", needed.name)]);
        // what can't be had here at all is nothing to fetch
        let not_here = Missing { not_here: vec!["FFmpeg".into()], ..Missing::default() };
        assert!(!not_here.fetchable() && !not_here.needed() && !not_here.is_empty());
    }
}
