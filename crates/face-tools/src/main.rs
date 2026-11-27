//! Everything a computer needs beside the programs, fetched by a program of the project's own: nothing has to
//! be installed first, and nothing is used that isn't the pinned file (its size and SHA-256).
//!
//!     face-tools get [--root <the folder with models/ and data/>] [models] [runtime] [mediapipe] [ffmpeg] [heif] [postgres]
//!     face-tools have [--root <folder>]
//!     face-tools list
//!
//! `get` with nothing named fetches all of them; what is there and right is left alone. `have` only says
//! what is there, and `list` what would be fetched, as JSON. The models go to `<root>/models`, MediaPipe's web runtime to `<root>/models/mediapipe-web`,
//! the model runtime, FFmpeg and the HEIC libraries to `<root>/runtime`, where the programs look for them.
//! `heif` builds libheif and libde265 from their source, which needs CMake and a C++ compiler. `postgres`
//! (PostgreSQL with pgvector, into `<root>/database/server`, for a computer that holds the shared database
//! itself) only when named: the app fetches it at its owner's click.
//!
//! What is sent: a request for each file, to its publisher (Hugging Face, Google's model storage, GitHub,
//! the host of Microsoft's runtime packages, the npm registry, conda-forge for `postgres`), as "model-fetch". Nothing about the user or the computer.

// (this crate needs no unsafe code: the compiler holds it to that)
#![forbid(unsafe_code)]

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use face_tools::{
    fetch::Net,
    models::{self, Folder, MODELS, Release},
    parts,
};

const PARTS: [&str; 6] = ["models", "runtime", "mediapipe", "ffmpeg", "heif", "postgres"];

/// A line that is written over as a download goes on, a whole percent at a time.
struct Shown {
    last: i64,
}

impl Shown {
    fn at(&mut self, done: u64, total: u64, name: &str) {
        let percent = done.saturating_mul(100).checked_div(total).unwrap_or(0) as i64;
        if percent != self.last {
            self.last = percent;
            print!("\r  {percent:3}%  {name:<60}");
            let _ = std::io::stdout().flush();
        }
    }

    fn end(&mut self) {
        if self.last >= 0 {
            println!();
        }
        self.last = -1;
    }
}

fn have(root: &Path) {
    let folder = Folder::new(root);
    let runtime = root.join("runtime");
    let missing = folder.needed(&MODELS);
    let word = |there: bool| if there { "here" } else { "missing" };
    println!(
        "models: {} of {} here{}",
        MODELS.len() - missing.len(),
        MODELS.len(),
        if missing.is_empty() { "" } else { ", missing:" }
    );
    for m in &missing {
        println!("  {}{}", m.name, if m.optional { " (optional)" } else { "" });
    }
    let system = parts::system();
    let packed = |p: Option<parts::Package>| match p {
        Some(p) => word(parts::have(p.files, &runtime)),
        None => "no pinned package for this kind of computer",
    };
    println!("the model runtime: {}", packed(parts::runtime(system)));
    println!(
        "MediaPipe's web runtime: {}",
        word(parts::have(&parts::MEDIAPIPE_FILES, &folder.at.join("mediapipe-web")))
    );
    println!("FFmpeg: {}", packed(parts::ffmpeg(system)));
    // what is in the folder must need nothing that isn't Windows' own or in the folder too
    if cfg!(windows) {
        let outside = face_tools::needs::from_outside(&[&runtime], &face_tools::needs::system_folder());
        match outside.as_slice() {
            [] => println!(
                "what the programs load needs nothing installed: every library it names is Windows' own or beside it"
            ),
            more => {
                println!("NEEDS SOMETHING INSTALLED (not part of Windows, and not in {}):", runtime.display());
                more.iter().for_each(|(file, lib)| println!("  {file} needs {lib}"));
            }
        }
    }
    println!("the HEIC libraries: {}", word(parts::heif_names().iter().all(|(_, name)| runtime.join(name).is_file())));
}

/// Everything that would be fetched, as JSON: for a check of the list.
fn list() -> serde_json::Value {
    use models::Origin;
    use serde_json::json;
    let packed = |p: parts::Package| {
        let files: Vec<_> =
            p.files.iter().map(|(member, name, size, sha256)| json!([member, name, size, sha256])).collect();
        json!({"what": p.what, "url": p.pin.url, "size": p.pin.size, "sha256": p.pin.sha256, "files": files})
    };
    let systems = ["windows-x64", "windows-arm64", "macos-arm64", "linux-x64", "linux-arm64"];
    let per_system = |of: fn(&str) -> Option<parts::Package>| {
        systems.iter().filter_map(|s| Some((s.to_string(), packed(of(s)?)))).collect::<serde_json::Map<_, _>>()
    };
    let origin = |o: Origin| match o {
        Origin::Ours => json!(null),
        Origin::At(url) => json!(url),
        Origin::InZip(url, member) => json!([url, member]),
    };
    let models: Vec<_> = (MODELS.iter())
        .map(|m| json!({"name": m.name, "size": m.size, "sha256": m.sha256, "origin": origin(m.origin), "optional": m.optional, "own": models::OWN.contains(&m.name)}))
        .collect();
    let mediapipe: Vec<_> =
        parts::MEDIAPIPE_FILES.iter().map(|(member, name, size, sha256)| json!([member, name, size, sha256])).collect();
    json!({
        "models": models,
        "release": models::RELEASE,
        "release_list": models::RELEASE_LIST,
        "tries": models::TRIES,
        "wait": models::WAIT,
        "runtime": per_system(parts::runtime),
        "ffmpeg": per_system(parts::ffmpeg),
        "mediapipe": {"url": parts::MEDIAPIPE.url, "size": parts::MEDIAPIPE.size, "sha256": parts::MEDIAPIPE.sha256, "files": mediapipe},
        "heif": [
            {"url": parts::DE265.url, "size": parts::DE265.size, "sha256": parts::DE265.sha256},
            {"url": parts::HEIF.url, "size": parts::HEIF.size, "sha256": parts::HEIF.sha256},
        ],
    })
}

fn get(root: &Path, wanted: &[String]) -> Result<(), String> {
    let all = wanted.is_empty();
    let want = |part: &str| all || wanted.iter().any(|w| w == part);
    let (net, runtime, system) = (Net::new(), root.join("runtime"), parts::system());
    let mut shown = Shown { last: -1 };
    if want("models") {
        let folder = Folder::new(root);
        let todo = folder.needed(&MODELS);
        if todo.is_empty() {
            println!("All face models are here.");
        } else {
            let mb = todo.iter().map(|m| m.size).sum::<u64>() as f64 / 1e6;
            println!("Downloading {} face models ({mb:.0} MB) into {} ...", todo.len(), folder.at.display());
            let mut notes = Vec::new();
            let got = models::fetch(
                &net,
                &mut Release::default(),
                &folder,
                &MODELS,
                &mut |d, t, n| shown.at(d, t, n),
                &mut |n| notes.push(n),
            );
            shown.end();
            notes.iter().for_each(|n| println!("  {n}"));
            got?;
        }
    }
    let mut packed = |part: &str, package: Option<parts::Package>, folder: &Path| -> Result<(), String> {
        let Some(package) = package else {
            // asked for by name: said as a failure; fetched with everything else: said and passed over
            let why = format!("There is no pinned package of {part} for this kind of computer ({system}) yet.");
            return if all {
                println!("{why}");
                Ok(())
            } else {
                Err(why)
            };
        };
        let got = parts::get(&net, &package, folder, &mut |d, t| shown.at(d, t, package.what));
        shown.end();
        match got {
            Ok(true) => println!("{}: put into {}", package.what, folder.display()),
            Ok(false) => println!("{}: here", package.what),
            Err(e) => return Err(format!("{}: {}", package.what, e.why)),
        }
        Ok(())
    };
    if want("runtime") {
        packed("the model runtime", parts::runtime(system), &runtime)?;
        match parts::get_vc_runtime(&runtime) {
            Ok(true) => println!("the Visual C++ runtime's libraries, which the model runtime needs: put beside it"),
            Ok(false) => {}
            Err(e) => return Err(e.why),
        }
    }
    if want("ffmpeg") {
        packed("FFmpeg", parts::ffmpeg(system), &runtime)?;
    }
    if want("mediapipe") {
        let got =
            parts::get_mediapipe(&net, &root.join("models"), &mut |d, t| shown.at(d, t, "MediaPipe's web runtime"));
        shown.end();
        match got {
            Ok(true) => {
                println!("MediaPipe's web runtime: put into {}", root.join("models").join("mediapipe-web").display())
            }
            Ok(false) => println!("MediaPipe's web runtime: here"),
            Err(e) => return Err(format!("MediaPipe's web runtime: {}", e.why)),
        }
    }
    if want("heif") {
        match parts::get_heif(&net, &runtime, &mut |line| println!("  {line}")) {
            Ok(true) => println!("The HEIC libraries: built and put into {}", runtime.display()),
            Ok(false) => println!("The HEIC libraries: here"),
            // with everything else: a computer without CMake still gets the rest, and is told
            Err(e) if all => {
                println!("The HEIC libraries weren't built (HEIC photos can't be read without them): {}", e.why)
            }
            Err(e) => return Err(format!("The HEIC libraries: {}", e.why)),
        }
    }
    // (named only: the server is for the computer that holds the shared database)
    if wanted.iter().any(|w| w == "postgres") {
        let server = face_tools::database::server(system)
            .ok_or(format!("There is no pinned PostgreSQL with pgvector for this kind of computer ({system}) yet."))?;
        let folder = face_tools::database::folder(root);
        let what = format!("PostgreSQL {} with pgvector {}", server.postgres, server.pgvector);
        let got = face_tools::database::get(server, &folder, &mut |d, t| shown.at(d, t, &what));
        shown.end();
        match got {
            Ok(true) => println!("{what}: put into {}", folder.display()),
            Ok(false) => println!("{what}: here"),
            Err(e) => return Err(format!("{what}: {}", e.why)),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut root: PathBuf = face_files::paths::root();
    if let Some(at) = args.iter().position(|a| a == "--root") {
        if at + 1 >= args.len() {
            eprintln!("--root needs a folder");
            return ExitCode::from(2);
        }
        root = PathBuf::from(args.remove(at + 1));
        args.remove(at);
    }
    let unknown: Vec<&String> = args.iter().skip(1).filter(|a| !PARTS.contains(&a.as_str())).collect();
    match args.first().map(String::as_str) {
        Some("have") if args.len() == 1 => {
            have(&root);
            ExitCode::SUCCESS
        }
        Some("list") if args.len() == 1 => {
            println!("{}", list());
            ExitCode::SUCCESS
        }
        Some("get") if unknown.is_empty() => match get(&root, &args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(why) => {
                eprintln!("\n{why}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!(
                "face-tools get [--root <folder>] [{}]\nface-tools have [--root <folder>]\nface-tools list",
                PARTS.join("] [")
            );
            ExitCode::from(2)
        }
    }
}
