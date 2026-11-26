//! The live tracker on its own:
//!
//!     face-server [--port 8766] [--pages <folder with the page files>]
//!
//! Or, on a computer that has a shared database, join codes for new devices (a page on 127.0.0.1 shows
//! them; `--home` names another private folder than this user's):
//!
//!     face-server pair [--devices 1] [--minutes 10] [--home <folder>]
//!
//! Or the labeling tool on its own (the page, its lists
//! and every change):
//!
//!     face-server label [--port 8765] [--pages <folder with the page files>]
//!
//! Or the app without its window: the tab page with the labeling tool in this process and the tracker
//! in a process of its own (`FACE_PORTS=<n>` moves its ports; `--browser`, or `FACE_UI=browser`, opens the
//! tab page in the default browser; the window itself is the program `FaceRecognizer`'s):
//!
//!     face-server app [--browser] [--sync] [--pages <folder with the page files>]
//!
//! `FACE_ROOT` is the folder with `models/` and `data/`, `FACE_HOME` the private folder, `FACE_DEVICE` where
//! the models run (auto | gpu | cpu), `FACE_ORT_LIB` the model runtime's library when it isn't beside the
//! program or in `<root>/runtime`. It never opens a browser.

use std::path::PathBuf;

use face_server::{app, paths, start_labeler, start_tracker};

fn main() {
    face_server::no_telemetry();
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// `face-server pair`: join codes, one device after another.
fn pair(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let (mut devices, mut minutes, mut home) = (1u32, 10u64, paths::home());
    while let Some(a) = args.next() {
        match a.as_str() {
            "--devices" => {
                devices = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|n| (1..=10).contains(n))
                    .ok_or("--devices needs 1 to 10")?
            }
            "--minutes" => {
                minutes = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|n| (1..=60).contains(n))
                    .ok_or("--minutes needs 1 to 60")?
            }
            "--home" => home = PathBuf::from(args.next().ok_or("--home needs a folder")?),
            other => return Err(format!("unknown option {other}")),
        }
    }
    face_server::join::pair(&home, devices, minutes)
}

/// Before anything is served: what this program needs and doesn't find is said, and fetched when the user
/// says so (`--get`: without asking, for a start that nobody watches; `--no-get`: never). Nothing is assumed
/// to be installed, and nothing fails later at the photo or video that needed it without having been said
/// here first.
#[cfg(feature = "fetch")]
fn first_run(root: &std::path::Path, label: bool, get: Option<bool>) -> Result<(), String> {
    use std::io::{IsTerminal, Write};

    use face_tools::first_run::{fetch, missing};
    let m = missing(root, label);
    if m.is_empty() {
        return Ok(());
    }
    println!("Not on this computer yet ({}):", root.display());
    m.lines().iter().for_each(|line| println!("  - {line}"));
    let fetchable = !(m.models.is_empty() && m.parts.is_empty());
    let yes = fetchable
        && match get {
            Some(answer) => answer,
            None if std::io::stdin().is_terminal() => {
                print!("Download them now ({:.0} MB)? [Y/n] ", m.megabytes());
                let _ = std::io::stdout().flush();
                let mut answer = String::new();
                let _ = std::io::stdin().read_line(&mut answer);
                !answer.trim().to_lowercase().starts_with('n')
            }
            None => false,
        };
    if yes {
        fetch(root, &m, &mut |line| println!("  getting {line} ..."))?;
        println!("Done.");
    } else if fetchable {
        println!(
            "Not downloaded. Start with --get to download them, or run: face-tools get --root \"{}\"",
            root.display()
        );
    }
    Ok(())
}

#[cfg(not(feature = "fetch"))]
fn first_run(_: &std::path::Path, _: bool, _: Option<bool>) -> Result<(), String> {
    Ok(())
}

/// `face-server app`: the tab page, the labeling tool in this process and the tracker in its own.
fn the_app(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let (mut browser, mut sync, mut pages, mut get) = (false, false, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--browser" => browser = true,
            "--sync" => sync = true,
            "--get" => get = Some(true),
            "--no-get" => get = Some(false),
            "--pages" => pages = Some(PathBuf::from(args.next().ok_or("--pages needs a folder")?)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let (root, home) = (paths::root(), paths::home());
    let pages = pages.unwrap_or_default(); // (none named: the program's own)
    first_run(&root, true, get)?;
    let started = app::start(&root, &home, &pages, sync)?;
    let ui = std::env::var("FACE_UI").unwrap_or_default();
    let open = ui != "none" && (browser || ui == "browser");
    let how = if open { " (in your web browser)" } else { "" };
    println!("{} is running at {}{how}. Stop it with Ctrl+C.", app::TITLE, started.shell);
    if open {
        face_server::desktop::open_url(&started.shell);
    } else if ui != "none" {
        println!(
            "This program has no window: open that address in a browser, start with --browser, or start the app itself (FaceRecognizer)."
        );
    }
    loop {
        std::thread::park();
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("pair") {
        return pair(args.skip(1));
    }
    if args.peek().map(String::as_str) == Some("tracker-child") {
        return app::tracker_child(args.skip(1));
    }
    // at the owner's sign-in: the shared database held on this computer started, nothing else
    if args.peek().map(String::as_str) == Some("start-database") {
        let named: Vec<String> = args.skip(1).collect();
        let at = |flag: &str| named.iter().position(|a| a == flag).and_then(|i| named.get(i + 1)).cloned();
        let root = at("--root").map_or_else(paths::root, PathBuf::from);
        let home = at("--home").map_or_else(paths::home, PathBuf::from);
        face_server::dbhost::at_sign_in(&root, &home);
        return Ok(());
    }
    if args.peek().map(String::as_str) == Some("app") {
        return the_app(args.skip(1));
    }
    let label = args.next_if(|a| a == "label").is_some();
    let (mut port, mut pages) = (if label { 8765u16 } else { 8766 }, None);
    // what is missing at a start: asked about (None), fetched without asking, or left alone
    let mut get: Option<bool> = None;
    // the labeling tool's sync with the shared database: only when asked for, since the app runs its own
    // (two programs syncing one device's library would each send what the other took)
    let mut sync = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--get" => get = Some(true),
            "--no-get" => get = Some(false),
            "--port" => port = args.next().and_then(|v| v.parse().ok()).ok_or("--port needs a number")?,
            "--pages" => pages = Some(PathBuf::from(args.next().ok_or("--pages needs a folder")?)),
            "--no-browser" => {}
            "--sync" if label => sync = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let (root, home) = (paths::root(), paths::home());
    let pages = pages.unwrap_or_default(); // (none named: the program's own)
    first_run(&root, label, get)?;
    if label {
        start_labeler(&root, &home, &pages, port, sync)
            .map_err(|e| format!("{e} (is the labeling tool already running?)"))?;
    } else {
        start_tracker(&root, &home, &pages, port).map_err(|e| format!("{e} (is the tracker already running?)"))?;
    }
    loop {
        std::thread::park();
    }
}
