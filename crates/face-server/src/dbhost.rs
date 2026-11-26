//! The shared database held on this computer: PostgreSQL with pgvector, fetched, made, started and connected by
//! the app itself at its owner's click (the owner, 2026-11-25: "We should try to install PostgreSQL ourselves",
//! "Make sure to install pgvector as well"; "anything we did here in our tests - the app or app installer should
//! be able to do for the user"). The Setup page's "On this computer" and Settings start it; nothing does by itself.
//!
//! What it does, in order (`set_up`): fetches the pinned server (`face_tools::database`, about 60 to 90 MB from
//! conda-forge) into `<root>/database/server`; makes the data folder `<root>/database/data` with a password for
//! the administrator that is made here, used once and never kept; makes the certificates (`face_db::servercert`,
//! the authority kept on this device) for this computer's addresses; writes the lines PostgreSQL needs (TLS only,
//! a password, this computer's own networks only); starts it; and connects as every new server is connected
//! (`face_db::setup` with the administrator's login: the app's own login, the database, pgvector, the tables, the
//! authority registered). Then the administrator's way in is shut (`pg_hba.conf`): from then on only the app's
//! own login reaches the server, over TLS, from this computer's networks.
//!
//! It runs whether the app runs or not (the owner, 2026-11-25: "what if they close the app - does Postgres stop? It
//! should not"): closing the app leaves it running, and it starts again at the owner's sign-in to this computer
//! (`autostart`: on Windows a start at sign-in of this user, which runs this program as `start-database`, without
//! a window; on a Mac a launch agent of this user, with which macOS also starts it again should it end). While the
//! app runs, it starts the server again should it have ended (`keep`). Nothing needs administrator rights, and
//! nothing is installed into the system: no service, no account. (It runs while the owner is signed in: a computer
//! that holds the database is one its owner stays signed in to.)
//!
//! What it keeps: `HOME/database-host.json` (where the data is, the port, the versions; no password) and the
//! folders under `<root>/database`, which no installer and no update ever touches.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::Duration,
};

use face_tools::database::{self, Server};
use serde_json::{Value, json};

/// What this computer's server is (no password in it).
const NOTE: &str = "database-host.json";
/// The ports tried, the usual one first: another PostgreSQL on this computer keeps its own.
const PORTS: std::ops::RangeInclusive<u16> = 5432..=5442;
/// The administrator's line in `pg_hba.conf`, there only while the app prepares the server.
const ADMIN_LINE: &str = "hostssl all postgres samehost scram-sha-256";

/// The data folder: `<root>/database/data`.
pub fn data(root: &Path) -> PathBuf {
    root.join("database").join("data")
}

fn server() -> Option<&'static Server> {
    database::server(face_tools::parts::system())
}

fn noted(home: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(home.join(NOTE)).ok()?).ok()
}

/// Whether this computer holds the shared database (made whole, not only begun).
pub fn held(home: &Path) -> bool {
    noted(home).is_some_and(|n| n["prepared"] == true)
}

/// Whether `root` is the installed app's own folder, as the app finds it without `FACE_ROOT` (a test copy always
/// names another): its start at sign-in then has the plain name an uninstaller takes away, a test copy's one of its own.
fn installed(root: &Path) -> bool {
    let own = if cfg!(windows) {
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
    } else {
        Some(face_files::paths::user_data("FaceRecognizer"))
    };
    own.is_some_and(|own| own == root)
}

/// The folder whose `database` holds this computer's server: where it was made, as its note says, wherever the app
/// is installed now (installed again into another folder, the app still finds it); else the app's own folder.
pub fn place(root: &Path, home: &Path) -> PathBuf {
    noted(home)
        .and_then(|n| n["data"].as_str().map(PathBuf::from))
        .and_then(|data| data.parent()?.parent().map(Path::to_path_buf))
        .filter(|at| data(at).join("PG_VERSION").is_file())
        .unwrap_or_else(|| root.to_path_buf())
}

/// A program of the server, started without a window of its own and with nothing of this process's to hold on to.
fn program(root: &Path, name: &str) -> Result<Command, String> {
    let server = server().ok_or("There is no PostgreSQL for this kind of computer in the app yet.")?;
    let path = database::program(&database::folder(root), server, name);
    if !path.is_file() {
        return Err(format!("{} isn't there: set the database up on this computer again (Setup).", path.display()));
    }
    let mut c = Command::new(path);
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    Ok(c)
}

fn run(mut c: Command, what: &str) -> Result<(), String> {
    let done = c.status().map_err(|e| format!("{what}: {e}"))?;
    if done.success() { Ok(()) } else { Err(format!("{what} stopped with {done}")) }
}

/// The last lines of the server's log, for a failure's message.
fn log_tail(root: &Path) -> String {
    let text = std::fs::read_to_string(data(root).join("server.log")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(4).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" / ")
}

/// Whether the server runs (its own way of telling: `pg_ctl status`).
pub fn running(root: &Path) -> bool {
    let Ok(mut c) = program(root, "pg_ctl") else { return false };
    c.arg("status").arg("-D").arg(data(root));
    c.status().is_ok_and(|s| s.success())
}

/// Start the server, unless it runs. Waits until it takes connections (a minute at most). True: it was started.
pub fn start(root: &Path) -> Result<bool, String> {
    if running(root) {
        return Ok(false);
    }
    // On a Mac only by macOS, through the agent: it is what starts the server at sign-in and again should it end,
    // and a server started otherwise beside it has the agent's own start fail again and again on the taken folder
    // (seen on the Mac, 2026-11-25). The agent is made here where it isn't there yet (its load starts the server).
    #[cfg(target_os = "macos")]
    {
        if mac_start::loaded(root) {
            mac_start::kick(root);
        } else {
            mac_start::keep(root)?;
        }
        for _ in 0..60 {
            if running(root) {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Err(format!("PostgreSQL didn't start: {}", log_tail(root)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut c = program(root, "pg_ctl")?;
        c.args(["start", "-w", "-t", "60", "-s", "-D"]).arg(data(root)).arg("-l").arg(data(root).join("server.log"));
        run(c, "PostgreSQL's start").map_err(|e| format!("{e}: {}", log_tail(root)))?;
        Ok(true)
    }
}

/// Stop the server (its fast way: connections are ended, nothing is lost).
pub fn stop(root: &Path) {
    if !running(root) {
        return;
    }
    if let Ok(mut c) = program(root, "pg_ctl") {
        c.args(["stop", "-w", "-t", "30", "-s", "-m", "fast", "-D"]).arg(data(root));
        let _ = c.status();
    }
}

/// How often the app looks whether the server still runs.
const LOOK_EVERY: Duration = Duration::from_secs(60);

/// While the app runs, where this computer holds the database: its start at sign-in kept as it should be, and the
/// server started now and again should it have ended (on a thread of its own).
pub fn keep(root: &Path, home: &Path) {
    if !held(home) {
        return;
    }
    // the first look before the programs start (the sync then finds the server running): at once where the
    // sign-in started it, seconds where it is started here
    look(&place(root, home), true);
    watch(root, home);
}

fn look(at: &Path, first: bool) {
    match start(at) {
        Ok(true) => println!("Shared database: this computer's server was started"),
        Ok(false) if first => println!("Shared database: this computer's server runs"),
        Ok(false) => {}
        Err(e) => println!("Shared database: this computer's server didn't start: {e}"),
    }
}

/// One watch for the run, begun at the app's start or once a setting up or a taking on is done.
static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The start at sign-in kept, and the server looked at every `LOOK_EVERY` (started again should it have ended), on
/// a thread of its own, for as long as the app runs (a server set up during a run is watched from then on: before,
/// one killed then stayed down until the app was started again, seen on Linux, 2026-11-25).
fn watch(root: &Path, home: &Path) {
    if WATCHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let (root, home) = (root.to_path_buf(), home.to_path_buf());
    let _ = std::thread::Builder::new().name("database server".into()).spawn(move || {
        if let Err(e) = autostart(&root, &home) {
            println!("Shared database: its start at sign-in couldn't be kept: {e}");
        }
        loop {
            std::thread::sleep(LOOK_EVERY);
            if held(&home) {
                look(&place(&root, &home), false);
            }
        }
    });
}

/// The app is uninstalled (Setup's uninstaller runs this program as `stop-database`): the server stopped and its
/// start at sign-in taken away (Setup removes the value itself too). Its data stays, as the library does: an app
/// installed again takes it on.
pub fn uninstalled(root: &Path, home: &Path) {
    if !held(home) {
        return;
    }
    let at = place(root, home);
    #[cfg(target_os = "macos")]
    mac_start::remove(&at);
    #[cfg(all(unix, not(target_os = "macos")))]
    linux_start::remove(root);
    stop(&at);
    println!("Shared database: this computer's server was stopped, as the app is uninstalled (its data is kept)");
}

/// This program at the owner's sign-in (`start-database`): the server started, where this computer holds it.
pub fn at_sign_in(root: &Path, home: &Path) {
    if !held(home) {
        return;
    }
    match start(&place(root, home)) {
        Ok(true) => println!("Shared database: this computer's server was started at sign-in"),
        Ok(false) => {}
        Err(e) => println!("Shared database: this computer's server didn't start at sign-in: {e}"),
    }
}

/// The server started at the owner's sign-in from now on (see the top of this file). Written again whenever it
/// isn't as it should be (the program moved, an update): looked at at every start of the app.
pub fn autostart(root: &Path, home: &Path) -> Result<(), String> {
    #[cfg(windows)]
    return windows_start::keep(root, home);
    #[cfg(target_os = "macos")]
    return mac_start::keep(&place(root, home));
    #[cfg(all(unix, not(target_os = "macos")))]
    return linux_start::keep(root, home);
    #[allow(unreachable_code)]
    {
        let _ = (root, home);
        Ok(())
    }
}

/// Linux: an entry in this user's autostart folder (the desktop's own way, as the app's `--autostart` is), which runs
/// this program as `start-database` at sign-in. The server is started by `pg_ctl` and runs on by itself.
#[cfg(all(unix, not(target_os = "macos")))]
mod linux_start {
    use std::path::{Path, PathBuf};

    /// The entry's file: one for each folder of the app, so that a test copy never takes the installed app's.
    pub fn file(root: &Path) -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?;
        let name = if super::installed(root) {
            "face-recognizer-database.desktop".to_string()
        } else {
            let digest = ring::digest::digest(&ring::digest::SHA256, root.display().to_string().as_bytes());
            let tag: String = digest.as_ref()[..4].iter().map(|b| format!("{b:02x}")).collect();
            format!("face-recognizer-database-{tag}.desktop")
        };
        Some(config.join("autostart").join(name))
    }

    /// An argument of the entry's command, quoted as the desktop entry specification asks.
    fn quoted(arg: &str) -> String {
        let mut out = String::from("\"");
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    }

    pub fn entry(me: &Path, root: &Path, home: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Face Recognizer database\n\
Comment=Starts the shared database this computer holds for Face Recognizer\n\
Exec={} start-database --root {} --home {}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
            quoted(&me.display().to_string()),
            quoted(&root.display().to_string()),
            quoted(&home.display().to_string())
        )
    }

    pub fn keep(root: &Path, home: &Path) -> Result<(), String> {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let file = file(root).ok_or("this user's folder isn't known")?;
        let text = entry(&me, root, home);
        if std::fs::read_to_string(&file).is_ok_and(|had| had == text) {
            return Ok(());
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, text).map_err(|e| e.to_string())
    }

    pub fn remove(root: &Path) {
        if let Some(file) = file(root) {
            let _ = std::fs::remove_file(file);
        }
    }
}

#[cfg(windows)]
mod windows_start {
    use std::{path::Path, ptr::null_mut};

    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW,
    };

    /// This user's programs started at sign-in.
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// The start's name: one for each folder of the app, so that a test copy never takes the installed app's.
    pub fn name(root: &Path) -> String {
        if super::installed(root) {
            "Face Recognizer database".to_string()
        } else {
            format!("Face Recognizer database ({})", root.display())
        }
    }

    /// What is started: this program as `start-database`, for these two folders.
    pub fn command(me: &Path, root: &Path, home: &Path) -> String {
        format!("\"{}\" start-database --root \"{}\" --home \"{}\"", me.display(), root.display(), home.display())
    }

    pub fn keep(root: &Path, home: &Path) -> Result<(), String> {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = command(&me, root, home);
        let (key, value) = (wide(RUN), wide(&name(root)));
        // as it is already: nothing written
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: a value of this user's own key read into a buffer of the size given
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if read == 0 && String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]) == command {
            return Ok(());
        }
        let data = wide(&command);
        // SAFETY: a text value of this user's own key, written whole
        let wrote = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if wrote == 0 { Ok(()) } else { Err(format!("Windows refused it ({wrote})")) }
    }
}

#[cfg(target_os = "macos")]
mod mac_start {
    use std::{
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    /// The agent's name: one for each folder of the app, so that a test copy never takes the installed app's.
    pub fn label(root: &Path) -> String {
        let digest = ring::digest::digest(&ring::digest::SHA256, root.display().to_string().as_bytes());
        let tag: String = digest.as_ref()[..4].iter().map(|b| format!("{b:02x}")).collect();
        format!("net.singularityresearch.facerecognizer.database.{tag}")
    }

    fn plist(root: &Path) -> Option<PathBuf> {
        Some(
            PathBuf::from(std::env::var_os("HOME")?)
                .join("Library/LaunchAgents")
                .join(format!("{}.plist", label(root))),
        )
    }

    fn esc(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    /// The agent: PostgreSQL itself, started at sign-in and started again should it end by failing (a stop of its
    /// own, `pg_ctl stop`, stays a stop).
    pub fn agent(root: &Path) -> Result<String, String> {
        let server = super::server().ok_or("no server for this computer")?;
        let postgres = face_tools::database::program(&face_tools::database::folder(root), server, "postgres");
        let data = super::data(root);
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n\
<key>Label</key><string>{}</string>\n\
<key>ProgramArguments</key><array><string>{}</string><string>-D</string><string>{}</string></array>\n\
<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n\
<key>StandardErrorPath</key><string>{}</string>\n<key>ProcessType</key><string>Background</string>\n</dict></plist>\n",
            esc(&label(root)),
            esc(&postgres.display().to_string()),
            esc(&data.display().to_string()),
            esc(&data.join("server.log").display().to_string())
        ))
    }

    fn domain() -> String {
        // SAFETY: asks for this process's own user id
        format!("gui/{}", unsafe { libc::getuid() })
    }

    fn launchctl(args: &[&str]) -> bool {
        Command::new("/bin/launchctl")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    pub fn keep(root: &Path) -> Result<(), String> {
        let file = plist(root).ok_or("this user's folder isn't known")?;
        let text = agent(root)?;
        let service = format!("{}/{}", domain(), label(root));
        if std::fs::read_to_string(&file).is_ok_and(|had| had == text) && loaded(root) {
            return Ok(());
        }
        if loaded(root) {
            launchctl(&["bootout", &service]);
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        let file = file.display().to_string();
        if launchctl(&["bootstrap", &domain(), &file]) {
            Ok(())
        } else {
            Err("macOS didn't take the launch agent".to_string())
        }
    }

    /// Whether macOS has the agent.
    pub fn loaded(root: &Path) -> bool {
        launchctl(&["print", &format!("{}/{}", domain(), label(root))])
    }

    /// The agent taken away (the app is no longer there to start it).
    pub fn remove(root: &Path) {
        launchctl(&["bootout", &format!("{}/{}", domain(), label(root))]);
        if let Some(file) = plist(root) {
            let _ = std::fs::remove_file(file);
        }
    }

    /// The server started by macOS, where the agent is there.
    pub fn kick(root: &Path) -> bool {
        loaded(root) && launchctl(&["kickstart", &format!("{}/{}", domain(), label(root))])
    }
}

/// What the Settings tab and the Setup page say of it.
pub fn state(root: &Path, home: &Path) -> Value {
    let note = noted(home);
    let root = &place(root, home);
    let server = server();
    json!({
        "can": server.is_some(),
        "postgres": server.map(|s| s.postgres),
        "pgvector": server.map(|s| s.pgvector),
        "mb": server.map(|s| (database::size(s) as f64 / 1e6).round()),
        "held": held(home),
        "running": held(home) && running(root),
        // a database in the app's folder that this private folder doesn't know (installed again after the private
        // folder was lost): it can be taken on
        "found": !held(home) && data(root).join("PG_VERSION").is_file(),
        // the database this computer held isn't where it was any more: it can be forgotten
        "missing": held(home) && !data(root).join("PG_VERSION").is_file(),
        "port": note.as_ref().and_then(|n| n["port"].as_u64()),
        "folder": root.join("database").display().to_string(),
        "work": held_work().clone(),
    })
}

/// The setting up as it goes, for the page that asked: {state: working | done | failed, text, percent}.
static WORK: Mutex<Value> = Mutex::new(Value::Null);

fn held_work() -> std::sync::MutexGuard<'static, Value> {
    WORK.lock().unwrap_or_else(|e| e.into_inner())
}

fn say(state: &str, text: &str, percent: Option<u64>) {
    *held_work() = json!({"state": state, "text": text, "percent": percent});
}

/// Begin setting the database up on this computer, on a thread of its own: false when it is under way already.
/// `done` is told once the database is set up (the sync then looks at once).
/// What `begin` does: set a new database up, or take on one that is here.
#[derive(Clone, Copy, PartialEq)]
pub enum Job {
    SetUp,
    TakeOn,
}

pub fn begin(root: &Path, home: &Path, job: Job, done: impl FnOnce() + Send + 'static) -> bool {
    {
        let mut work = held_work();
        if work["state"] == "working" {
            return false;
        }
        *work = json!({"state": "working", "text": "Starting.", "percent": null});
    }
    let (root, home) = (root.to_path_buf(), home.to_path_buf());
    let (root_, home_) = (root.clone(), home.clone());
    let run = move || if job == Job::TakeOn { take_on(&root, &home) } else { set_up(&root, &home) };
    let started = std::thread::Builder::new().name("database setup".into()).spawn(move || match run() {
        Ok(text) => {
            println!("Shared database: {text}");
            say("done", &text, None);
            watch(&root_, &home_);
            done();
        }
        Err(e) => {
            println!("Shared database: setting it up on this computer failed: {e}");
            say("failed", &e, None);
        }
    });
    if started.is_err() {
        say("failed", "The setup couldn't start.", None);
    }
    true
}

/// The database this computer held, taken on again where this private folder doesn't know it: the app was installed
/// again after the private folder was lost (the app's own folder, and with it `database`, was kept). Nothing in it
/// changes but the app's own login's password: the server stopped, the new password is given by PostgreSQL's own way
/// for the owner of its folder (single-user mode, which needs no login), the server started, and this device connects
/// as any device does to a server that is set up (the server proves itself by the login's channel binding, and its
/// certificate authority is taken from it). The text to show.
pub fn take_on(root: &Path, home: &Path) -> Result<String, String> {
    if face_db::config(home).is_some() {
        return Err("This device has a shared database already.".to_string());
    }
    let server = server().ok_or("There is no PostgreSQL for this kind of computer in the app yet.")?;
    let data = data(root);
    let version = std::fs::read_to_string(data.join("PG_VERSION"))
        .map_err(|_| format!("There is no database in {}.", data.display()))?;
    let major = server.postgres.split('.').next().unwrap_or_default();
    if version.trim() != major {
        return Err(format!(
            "The database in {} is PostgreSQL {}'s, and this app brings PostgreSQL {}: it isn't taken on.",
            data.display(),
            version.trim(),
            server.postgres
        ));
    }
    let what = format!("PostgreSQL {} with pgvector {}", server.postgres, server.pgvector);
    say("working", &format!("Getting {what} where it isn't here ..."), None);
    database::get(server, &database::folder(root), &mut |_, _| ())
        .map_err(|e| format!("The download failed: {}", e.why))?;
    // its port, as its own settings say (the app's lines are the last)
    let conf = std::fs::read_to_string(data.join("postgresql.conf")).map_err(|e| e.to_string())?;
    let port = conf
        .lines()
        .filter_map(|l| l.trim().strip_prefix("port = ")?.split_whitespace().next()?.parse::<u16>().ok())
        .last()
        .unwrap_or(5432);
    say("working", "Giving the app's own login a new password ...", None);
    #[cfg(target_os = "macos")]
    mac_start::remove(root);
    stop(root);
    let password = made_password();
    let mut single = program(root, "postgres")?;
    single.args(["--single", "-D"]).arg(&data).arg("postgres").stdin(Stdio::piped());
    let mut child = single.spawn().map_err(|e| format!("PostgreSQL's single-user mode: {e}"))?;
    {
        use std::io::Write;
        let mut input = child.stdin.take().ok_or("PostgreSQL's single-user mode took no input")?;
        input
            .write_all(
                format!(
                    "ALTER ROLE facerec WITH LOGIN PASSWORD '{password}';
"
                )
                .as_bytes(),
            )
            .map_err(|e| e.to_string())?;
    }
    let ended = child.wait().map_err(|e| e.to_string())?;
    if !ended.success() {
        return Err(format!("PostgreSQL's single-user mode stopped with {ended}: {}", log_tail(root)));
    }
    std::fs::write(
        home.join(NOTE),
        json!({"data": data.display().to_string(), "port": port, "postgres": server.postgres, "pgvector": server.pgvector,
               "prepared": true, "made": face_files::paths::utc_now(), "taken_on": true})
        .to_string(),
    )
    .map_err(|e| e.to_string())?;
    say("working", "Starting it and connecting ...", None);
    start(root)?;
    let here = primary(home);
    let login = face_db::join::Login::new(&here, port, "facerec", "facerec", &password, "facerec");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(face_db::setup(&login, home))
        .map_err(|e| format!("Connecting failed: {}", e.lines().next().unwrap_or("")))?;
    let at_sign_in = match autostart(root, home) {
        Ok(()) => " It keeps running when the app is closed, and starts again when you sign in.".to_string(),
        Err(e) => format!(" Its start at sign-in couldn't be set ({e}): the app starts it."),
    };
    Ok(format!("This computer holds the shared database again: {what} at {here}:{port}, as it was.{at_sign_in}"))
}

/// The database this computer held is no longer where it was (its folder deleted): this device stops using it, so
/// that a new one can be set up or another joined. Refused while the database is there.
pub fn forget(root: &Path, home: &Path) -> Result<String, String> {
    let at = place(root, home);
    if data(&at).join("PG_VERSION").is_file() {
        return Err(format!("The database is still in {}: it isn't forgotten while it is there.", data(&at).display()));
    }
    let _ = std::fs::remove_file(home.join(NOTE));
    face_db::forget(home)?;
    Ok("Forgotten: this device has no shared database now. Set a new one up, or join your other devices' one."
        .to_string())
}

/// This computer's address as the other devices reach it: a private address of a real network adapter (the one the
/// wider network is reached from where it is one), not a VPN's or a container's (a Linux computer whose way out was a
/// VPN gave that one's address, 2026-11-25); else the one the wider network is reached from; else itself.
fn primary(home: &Path) -> String {
    let real = crate::hardware::own_addresses(home, Some(false));
    let toward: Vec<Ipv4Addr> = face_db::join::own_addresses(("192.0.2.1", 9))
        .into_iter()
        .filter_map(|a| match a {
            IpAddr::V4(v) if !v.is_loopback() => Some(v),
            _ => None,
        })
        .collect();
    choose(&real, &toward).to_string()
}

/// `primary`'s choice from this computer's real adapters' addresses and those it reaches the wider network from.
fn choose(real: &[Ipv4Addr], toward: &[Ipv4Addr]) -> Ipv4Addr {
    let private = |ip: &&Ipv4Addr| ip.is_private();
    toward
        .iter()
        .find(|ip| real.contains(ip) && ip.is_private())
        .or_else(|| real.iter().find(private))
        .or_else(|| toward.first())
        .or_else(|| real.first())
        .copied()
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

fn free_port() -> Option<u16> {
    PORTS.into_iter().find(|p| TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, *p))).is_ok())
}

/// A password made here, for the administrator, used once.
fn made_password() -> String {
    let id = face_db::new_id() + &face_db::new_id();
    id.chars().filter(char::is_ascii_alphanumeric).take(40).collect()
}

/// `pg_hba.conf`: TLS only, a password, from this computer's own networks; the administrator from this computer
/// alone, and only while `admin`.
fn access(admin: bool) -> String {
    let mut out = String::from(
        "# Face Recognizer's shared database, made by the app on this computer: encrypted connections only (TLS), from\n\
         # this computer's own networks, with a password. The administrator's login is refused from everywhere.\n",
    );
    if admin {
        out.push_str(ADMIN_LINE);
        out.push('\n');
    }
    out.push_str("hostssl all postgres all reject\nhostssl all all samenet scram-sha-256\n");
    out
}

/// The lines `postgresql.conf` gets at its end.
fn settings(port: u16) -> String {
    format!(
        "\n# Face Recognizer (made by the app on this computer)\nlisten_addresses = '*'\nport = {port}\nssl = on\n\
ssl_cert_file = 'server.crt'\nssl_key_file = 'server.key'\npassword_encryption = scram-sha-256\n"
    )
}

/// Set the shared database up on this computer, the whole way (see the top of this file). The text to show.
pub fn set_up(root: &Path, home: &Path) -> Result<String, String> {
    if face_db::config(home).is_some() {
        return Err("This device has a shared database already.".to_string());
    }
    let server = server().ok_or("There is no PostgreSQL for this kind of computer in the app yet: set the database up on another computer (Setup, \"On another computer\").")?;
    let data = data(root);
    // what an earlier try left (its server not prepared, so nothing in it yet) is made again; anything else
    // in the folder is someone's database and is left alone
    if data.join("PG_VERSION").exists() {
        match noted(home) {
            Some(n) if n["prepared"] != true => {
                // (on a Mac its agent first: it would start the server again on a folder that is going)
                #[cfg(target_os = "macos")]
                mac_start::remove(root);
                stop(root);
                std::fs::remove_dir_all(&data).map_err(|e| format!("Can't clear {}: {e}", data.display()))?;
            }
            _ => return Err(format!("{} holds a database already: it is left as it is.", data.display())),
        }
    }
    let what = format!("PostgreSQL {} with pgvector {}", server.postgres, server.pgvector);
    let mb = database::size(server) as f64 / 1e6;
    say("working", &format!("Downloading {what} ({mb:.0} MB from conda-forge, each file checked) ..."), Some(0));
    let mut last = u64::MAX;
    database::get(server, &database::folder(root), &mut |got, total| {
        let percent = got.saturating_mul(100).checked_div(total).unwrap_or(0);
        if percent != last {
            last = percent;
            say(
                "working",
                &format!("Downloading {what} ({mb:.0} MB from conda-forge, each file checked) ..."),
                Some(percent),
            );
        }
    })
    .map_err(|e| format!("The download failed: {}", e.why))?;

    say("working", "Making the database's folder ...", None);
    let port = free_port().ok_or("No free port for the database between 5432 and 5442.")?;
    let admin = made_password();
    std::fs::create_dir_all(data.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
    let secret = home.join(face_db::servercert::FOLDER).join(".once");
    std::fs::create_dir_all(secret.parent().unwrap_or(home)).map_err(|e| e.to_string())?;
    face_files::protect::write_private(&secret, admin.as_bytes())?;
    let mut init = program(root, "initdb")?;
    init.arg("-D").arg(&data).args(["-U", "postgres", "-A", "scram-sha-256", "-E", "UTF8", "--locale=C"]);
    init.arg(format!("--pwfile={}", secret.display()));
    let made = run(init, "initdb");
    let _ = std::fs::remove_file(&secret);
    made?;
    std::fs::write(
        home.join(NOTE),
        json!({"data": data.display().to_string(), "port": port, "postgres": server.postgres, "pgvector": server.pgvector,
               "prepared": false, "made": face_files::paths::utc_now()})
        .to_string(),
    )
    .map_err(|e| e.to_string())?;

    say("working", "Making the certificates and the server's settings ...", None);
    let here = primary(home);
    let mut names = vec![here.clone()];
    names
        .extend(crate::hardware::own_addresses(home, Some(false)).iter().map(|a| a.to_string()).filter(|a| *a != here));
    let made = face_db::servercert::make(home, &names)?;
    for file in ["server.crt", "server.key"] {
        std::fs::copy(made.join(file), data.join(file)).map_err(|e| format!("{file}: {e}"))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data.join("server.key"), std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    let conf = data.join("postgresql.conf");
    let mut text = std::fs::read_to_string(&conf).map_err(|e| e.to_string())?;
    text.push_str(&settings(port));
    std::fs::write(&conf, text).map_err(|e| e.to_string())?;
    std::fs::write(data.join("pg_hba.conf"), access(true)).map_err(|e| e.to_string())?;

    say("working", "Starting it (Windows may ask whether PostgreSQL may be reached on your network) ...", None);
    start(root)?;
    // it takes connections now (pg_ctl waited): the address the other devices use reaches it too
    let reached = format!("{here}:{port}")
        .parse::<SocketAddr>()
        .ok()
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)).is_ok());
    if !reached {
        return Err(format!("The server runs, but isn't reached at {here}:{port}: is a firewall in the way?"));
    }

    say("working", "Preparing it for Face Recognizer (its own login, the database, pgvector, the tables) ...", None);
    let login = face_db::join::Login::new(&here, port, "facerec", "postgres", &admin, "facerec");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(face_db::setup(&login, home))
        .map_err(|e| format!("Preparing it failed: {}", e.lines().next().unwrap_or("")))?;
    drop(admin);

    // the administrator's way in is shut: from now on only the app's own login reaches the server
    std::fs::write(data.join("pg_hba.conf"), access(false)).map_err(|e| e.to_string())?;
    let mut reload = program(root, "pg_ctl")?;
    reload.args(["reload", "-s", "-D"]).arg(&data);
    run(reload, "PostgreSQL's reload")?;
    let mut note = noted(home).unwrap_or_else(|| json!({}));
    note["prepared"] = json!(true);
    std::fs::write(home.join(NOTE), note.to_string()).map_err(|e| e.to_string())?;
    // it runs from now on whether the app does or not, and starts again at sign-in
    let at_sign_in = match autostart(root, home) {
        Ok(()) => {
            " It keeps running when the app is closed, and starts again when you sign in to this computer.".to_string()
        }
        Err(e) => format!(
            " It keeps running when the app is closed; its start at sign-in couldn't be set ({e}): the app starts it."
        ),
    };
    Ok(format!(
        "This computer holds the shared database now: {what} at {here}:{port}.{at_sign_in} Other devices join it with a code."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_takes_only_tls_from_its_own_networks_and_the_administrator_only_while_it_is_prepared() {
        let preparing = access(true);
        let after = access(false);
        assert!(preparing.contains(ADMIN_LINE) && !after.contains(ADMIN_LINE));
        for text in [&preparing, &after] {
            let rules: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
            // TLS only (no "host" or "local" lines), a password everywhere, the administrator refused after its line
            assert!(rules.iter().all(|l| l.starts_with("hostssl ")), "{rules:?}");
            assert!(rules.iter().all(|l| l.ends_with("scram-sha-256") || l.ends_with("reject")));
            let reject = rules.iter().position(|l| *l == "hostssl all postgres all reject").unwrap();
            assert!(rules.iter().position(|l| l.ends_with("samenet scram-sha-256")).unwrap() > reject);
        }
        let conf = settings(5433);
        for line in [
            "listen_addresses = '*'",
            "port = 5433",
            "ssl = on",
            "ssl_key_file = 'server.key'",
            "password_encryption = scram-sha-256",
        ] {
            assert!(conf.contains(line), "{line}");
        }
        let p = made_password();
        assert!(p.len() >= 30 && p.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(p, made_password());
    }

    /// The start at sign-in is this program for these two folders, under a name of its own for each copy of the
    /// app; nothing of it asks for administrator rights (this user's own key).
    #[cfg(windows)]
    #[test]
    fn the_start_at_sign_in_names_the_program_and_its_folders() {
        let (me, root, home) =
            (Path::new(r"C:\A b\FaceRecognizer.exe"), Path::new(r"C:\A b"), Path::new(r"C:\U\FaceLabels"));
        assert_eq!(
            windows_start::command(me, root, home),
            r#""C:\A b\FaceRecognizer.exe" start-database --root "C:\A b" --home "C:\U\FaceLabels""#
        );
        assert_ne!(windows_start::name(Path::new(r"D:\other")), windows_start::name(&face_files::paths::app_root()));
        assert_eq!(windows_start::name(&face_files::paths::app_root()), "Face Recognizer database");
    }

    /// Linux: the autostart entry runs this program for these two folders, each argument quoted as desktop entries
    /// quote them, under a file of its own for each copy of the app.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_linux_start_at_sign_in_is_an_autostart_entry_of_this_program() {
        let e = linux_start::entry(
            Path::new("/home/u/.local/lib/FaceRecognizer/FaceRecognizer"),
            Path::new("/a b/$x"),
            Path::new("/h"),
        );
        assert!(e.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(e.contains(
            "Exec=\"/home/u/.local/lib/FaceRecognizer/FaceRecognizer\" start-database --root \"/a b/\\$x\" --home \"/h\"\n"
        ));
        let (installed, test) =
            (linux_start::file(&face_files::paths::app_root()), linux_start::file(Path::new("/tmp/t")));
        assert!(installed.unwrap().ends_with("autostart/face-recognizer-database.desktop"));
        assert_ne!(linux_start::file(&face_files::paths::app_root()), test);
    }

    /// The address the other devices are given: a real adapter's private one, never a VPN's that the way out goes
    /// through.
    #[test]
    fn the_address_given_is_a_real_adapter_s_private_one() {
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        let (lan, vpn, other) = (ip("192.168.1.20"), ip("100.64.12.34"), ip("172.16.5.4"));
        assert_eq!(choose(&[lan, other], &[vpn]), lan);
        assert_eq!(choose(&[lan, other], &[other]), other);
        assert_eq!(choose(&[], &[vpn]), vpn);
        assert_eq!(choose(&[], &[]), Ipv4Addr::LOCALHOST);
    }

    #[test]
    fn a_computer_without_a_server_holds_nothing_and_says_what_it_could() {
        let home = std::env::temp_dir().join(format!("face-dbhost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(!held(&home));
        let s = state(&home, &home);
        assert_eq!(s["held"], false);
        assert_eq!(s["running"], false);
        assert_eq!(s["can"], server().is_some());
        // begun but not prepared is not held
        std::fs::write(home.join(NOTE), r#"{"prepared": false, "port": 5432}"#).unwrap();
        assert!(!held(&home));
        let _ = std::fs::remove_dir_all(&home);
    }
}
