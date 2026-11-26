//! The model servers of the other kinds on this computer kept open to the owner's other devices, as LM Studio is
//! (lmstudio.rs). The owner's words (2026-11-25): "Yes, keep Ollama open to the network like LM Studio" and "same
//! with llama.cpp and vLLM if they are installed".
//!
//! None of them has a setting of its own that the app could change, as LM Studio has: Ollama listens where its
//! environment's OLLAMA_HOST says (this computer only, unless it is set), llama.cpp's server and vLLM where their
//! `--host` says (llama.cpp: this computer only, unless it is given). So a server of this user's that listens on
//! this computer only is started again as it was started: the same program with the same arguments, in the same
//! folder, with the same environment (its models' folder among them), but listening on every interface. Ollama
//! that its own app runs is started again through its app, which hands the setting on. A server that doesn't
//! answer on the network in its time is started again as it was: the owner's server is never left off, and that
//! one isn't tried again while the app runs. A server whose start this user can't read (another user's, a system
//! service such as Linux's ollama.service) is left alone, and Settings says why and how.
//!
//! Every minute while the app runs, with LM Studio's upkeep and under the same switch (Settings, Keep the model
//! servers here open to my other devices: off for a new copy, `lmstudio::keep_open`). `FACE_LMSTUDIO=off` (which
//! every check sets) or `FACE_KEEP_SERVERS=off` turns it off whatever the switch says.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// What this upkeep last did, for Settings (the labeling tool's process reads it): `HOME/<this>`.
const SAID: &str = "model-servers.json";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Ollama,
    LlamaCpp,
    Vllm,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Ollama => "Ollama",
            Kind::LlamaCpp => "llama.cpp",
            Kind::Vllm => "vLLM",
        }
    }

    /// How long a server of this kind may take to answer once started again: a model may load first (a
    /// choice, not a measurement: vLLM takes minutes for a large model).
    fn within(self) -> Duration {
        Duration::from_secs(match self {
            Kind::Ollama => 90,
            Kind::LlamaCpp => 300,
            Kind::Vllm => 1200,
        })
    }
}

/// Where a server listens, as its start says it.
#[derive(Debug, PartialEq)]
struct Listen {
    host: String,
    port: u16,
}

impl Listen {
    /// On this computer only: a loopback host. (An empty host, 0.0.0.0 or ::, or this computer's address on a
    /// network, is reachable from there.)
    fn here_only(&self) -> bool {
        let h = self.host.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        h == "localhost" || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    }
}

/// OLLAMA_HOST as Ollama reads it: `[scheme://]host[:port]`; unset or empty, 127.0.0.1:11434; an empty host
/// (":11434") every interface.
fn ollama_listen(value: Option<&str>) -> Listen {
    let v = value.unwrap_or("").trim();
    let v = v.split_once("://").map_or(v, |(_, rest)| rest).trim_end_matches('/');
    if v.is_empty() {
        return Listen { host: "127.0.0.1".into(), port: 11434 };
    }
    let (host, port) = if let Some(rest) = v.strip_prefix('[') {
        let (h, after) = rest.split_once(']').unwrap_or((rest, ""));
        (h.to_string(), after.strip_prefix(':').and_then(|p| p.parse().ok()))
    } else if v.matches(':').count() == 1 {
        let (h, p) = v.split_once(':').unwrap_or((v, ""));
        (h.to_string(), p.parse().ok())
    } else {
        (v.to_string(), None)
    };
    Listen { host, port: port.unwrap_or(11434) }
}

/// The value of an option, `--name V` or `--name=V`; the last one given counts, as for the servers themselves.
fn option(args: &[String], name: &str) -> Option<String> {
    let mut found = None;
    for (i, a) in args.iter().enumerate() {
        if a == name {
            found = args.get(i + 1).cloned();
        } else if let Some(v) = a.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')) {
            found = Some(v.to_string());
        }
    }
    found
}

/// llama.cpp's server: `--host` (else LLAMA_ARG_HOST, else 127.0.0.1) and `--port` (else LLAMA_ARG_PORT, else 8080).
fn llama_listen(args: &[String], env: &HashMap<String, String>) -> Listen {
    let host = option(args, "--host").or_else(|| env.get("LLAMA_ARG_HOST").cloned()).unwrap_or("127.0.0.1".into());
    let port = option(args, "--port").or_else(|| env.get("LLAMA_ARG_PORT").cloned());
    Listen { host, port: port.and_then(|p| p.parse().ok()).unwrap_or(8080) }
}

/// vLLM: `--host` (every interface unless given) and `--port` (8000).
fn vllm_listen(args: &[String]) -> Listen {
    Listen {
        host: option(args, "--host").unwrap_or_default(),
        port: option(args, "--port").and_then(|p| p.parse().ok()).unwrap_or(8000),
    }
}

/// The arguments with every `--host` given as 0.0.0.0, or `--host 0.0.0.0` added (which also stands over
/// llama.cpp's LLAMA_ARG_HOST).
fn open_args(args: &[String]) -> Vec<String> {
    let (mut out, mut given, mut i) = (Vec::new(), false, 0);
    while i < args.len() {
        if args[i] == "--host" {
            out.extend(["--host".to_string(), "0.0.0.0".to_string()]);
            (given, i) = (true, i + 2);
            continue;
        }
        if args[i].starts_with("--host=") {
            out.push("--host=0.0.0.0".to_string());
            given = true;
        } else {
            out.push(args[i].clone());
        }
        i += 1;
    }
    if !given {
        out.extend(["--host".to_string(), "0.0.0.0".to_string()]);
    }
    out
}

/// Which kind of server a process is, from its program's name and its whole command line (argv[0] first).
fn kind_of(stem: &str, cmd: &[String]) -> Option<Kind> {
    let stem = stem.to_ascii_lowercase();
    if stem == "ollama" && cmd.get(1).is_some_and(|a| a == "serve") {
        return Some(Kind::Ollama);
    }
    if stem == "llama-server" {
        return Some(Kind::LlamaCpp);
    }
    // `vllm serve ...` (as the script, or its interpreter running it) or the server's module
    let vllm = cmd.windows(2).any(|w| Path::new(&w[0]).file_stem().is_some_and(|s| s == "vllm") && w[1] == "serve")
        || cmd.iter().any(|a| a == "vllm.entrypoints.openai.api_server");
    vllm.then_some(Kind::Vllm)
}

/// Ollama's own app, which runs its server: Windows' "ollama app.exe", macOS' Ollama.app.
fn ollama_app(stem: &str, exe: Option<&Path>) -> bool {
    stem.eq_ignore_ascii_case("ollama app")
        || exe.is_some_and(|p| p.to_string_lossy().to_ascii_lowercase().contains("ollama.app/contents/macos/"))
}

/// A process as it was started: what is needed to start it again exactly so.
#[derive(Clone)]
struct Start {
    pid: Pid,
    exe: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
}

/// A server found listening on this computer only, and how to start it again open.
struct Found {
    kind: Kind,
    listen: Listen,
    /// what is started again: Ollama's app where it runs the server, else the server itself
    start: Start,
    /// (key, value) set in its environment, or None for `--host 0.0.0.0` in its arguments
    set: Option<(String, String)>,
}

/// (servers to open, servers that can't be: (kind, port, why))
fn look(sys: &System) -> (Vec<Found>, Vec<(Kind, u16, String)>) {
    let text = |v: &[OsString]| v.iter().map(|s| s.to_string_lossy().into_owned()).collect::<Vec<_>>();
    let stem = |p: &sysinfo::Process| {
        p.exe()
            .and_then(|e| e.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.name().to_string_lossy().trim_end_matches(".exe").to_string())
    };
    let start = |p: &sysinfo::Process| -> Option<Start> {
        let cmd = text(p.cmd());
        let exe = p.exe()?.to_path_buf();
        // (unreadable: another user's, or a system service; an empty environment is never a real start)
        (!cmd.is_empty() && !p.environ().is_empty()).then(|| Start {
            pid: p.pid(),
            exe,
            args: cmd[1..].to_vec(),
            cwd: p.cwd().map(Path::to_path_buf),
            env: p
                .environ()
                .iter()
                .filter_map(|kv| {
                    let s = kv.to_string_lossy();
                    let (k, v) = s.split_once('=').filter(|(k, _)| !k.is_empty())?;
                    Some((OsString::from(k), OsString::from(v)))
                })
                .collect(),
        })
    };
    let kinds: HashMap<Pid, Kind> =
        sys.processes().iter().filter_map(|(pid, p)| Some((*pid, kind_of(&stem(p), &text(p.cmd()))?))).collect();
    let (mut open, mut cannot) = (Vec::new(), Vec::new());
    for (pid, kind) in &kinds {
        let Some(p) = sys.process(*pid) else { continue };
        // a server's own workers (vLLM's) are started with it
        if p.parent().and_then(|parent| kinds.get(&parent)) == Some(kind) {
            continue;
        }
        let cmd = text(p.cmd());
        let env: HashMap<String, String> = p
            .environ()
            .iter()
            .filter_map(|kv| kv.to_string_lossy().split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let readable = !cmd.is_empty() && !env.is_empty();
        let listen = match kind {
            Kind::Ollama => ollama_listen(env.get("OLLAMA_HOST").map(String::as_str)),
            Kind::LlamaCpp => llama_listen(cmd.get(1..).unwrap_or_default(), &env),
            Kind::Vllm => vllm_listen(cmd.get(1..).unwrap_or_default()),
        };
        if readable && !listen.here_only() {
            continue;
        }
        if !readable {
            let how = match kind {
                Kind::Ollama => {
                    " To open it: give it OLLAMA_HOST=0.0.0.0 (on Linux, sudo systemctl edit ollama.service, \
                     [Service] Environment=\"OLLAMA_HOST=0.0.0.0\", then sudo systemctl restart ollama)."
                }
                _ => " To open it: start it with --host 0.0.0.0.",
            };
            cannot.push((
                *kind,
                listen.port,
                format!("runs as another user or a system service, so this app can't start it again.{how}"),
            ));
            continue;
        }
        let app = p
            .parent()
            .and_then(|parent| sys.process(parent))
            .filter(|a| *kind == Kind::Ollama && ollama_app(&stem(a), a.exe()));
        let Some(to_start) = (match app {
            Some(a) => start(a),
            None => start(p),
        }) else {
            continue;
        };
        let set = match kind {
            Kind::Ollama => Some(("OLLAMA_HOST".to_string(), format!("0.0.0.0:{}", listen.port))),
            _ => None,
        };
        open.push(Found { kind: *kind, listen, start: to_start, set });
    }
    (open, cannot)
}

fn processes() -> System {
    let mut sys = System::new();
    let what = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
        .with_environ(UpdateKind::Always)
        .with_cwd(UpdateKind::Always);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, what);
    sys
}

/// A process and every process it started, children first.
fn tree(sys: &System, root: Pid) -> Vec<Pid> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        out.extend(sys.processes().iter().filter(|(_, p)| p.parent() == Some(parent)).map(|(pid, _)| *pid));
        i += 1;
    }
    out.reverse();
    out
}

/// Ends the processes, gently first where the system has a way (a server then lets go of its graphics memory),
/// and waits until they are gone (at most a minute).
fn end(pids: &[Pid]) {
    let alive = |pid: &Pid| {
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[*pid]), true, ProcessRefreshKind::nothing());
        sys.process(*pid).is_some()
    };
    for pid in pids {
        stop(*pid, false);
    }
    let t0 = Instant::now();
    while pids.iter().any(alive) && t0.elapsed() < Duration::from_secs(45) {
        std::thread::sleep(Duration::from_millis(500));
    }
    for pid in pids.iter().filter(|p| alive(p)) {
        stop(*pid, true);
    }
    let t0 = Instant::now();
    while pids.iter().any(alive) && t0.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(unix)]
fn stop(pid: Pid, hard: bool) {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
    if let Some(p) = sys.process(pid) {
        let _ = p.kill_with(if hard { sysinfo::Signal::Kill } else { sysinfo::Signal::Term });
    }
}

#[cfg(windows)]
fn stop(pid: Pid, _hard: bool) {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    };
    // SAFETY: a handle opened here for this one process is used once and closed
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid.as_u32());
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// Starts a program as `start` says, with `args` and `env` in place of its own, apart from this app: in a
/// group of its own, no window, nothing of it read here, and it goes on when this app ends.
fn launch(start: &Start, args: &[String], env: &[(OsString, OsString)]) -> std::io::Result<u32> {
    let mut run = Command::new(&start.exe);
    run.args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = start.cwd.as_ref().filter(|c| c.is_dir()) {
        run.current_dir(cwd);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        run.creation_flags(0x0800_0000 | 0x0000_0200); // no console window; a process group of its own
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        run.process_group(0);
    }
    let mut child = run.spawn()?;
    let id = child.id();
    // (reaped when it ends, so it never lingers as a finished child of this app)
    let _ = std::thread::Builder::new().name("model-server".into()).spawn(move || {
        let _ = child.wait();
    });
    Ok(id)
}

/// The server answers at one of this computer's network addresses.
fn answers_on_network(home: &Path, port: u16) -> bool {
    crate::hardware::own_addresses(home, None).into_iter().any(|ip: Ipv4Addr| {
        TcpStream::connect_timeout(&SocketAddr::from((ip, port)), Duration::from_millis(400)).is_ok()
    })
}

/// The original start with the change made: (arguments, environment).
fn changed(f: &Found) -> (Vec<String>, Vec<(OsString, OsString)>) {
    match &f.set {
        Some((key, value)) => {
            let mut env: Vec<(OsString, OsString)> =
                f.start.env.iter().filter(|(k, _)| !k.to_string_lossy().eq_ignore_ascii_case(key)).cloned().collect();
            env.push((key.into(), value.into()));
            (f.start.args.clone(), env)
        }
        None => (open_args(&f.start.args), f.start.env.clone()),
    }
}

/// Starts one server again listening on every interface; as it was when that fails. What happened, in words.
fn open_one(home: &Path, f: &Found) -> Result<String, String> {
    let sys = processes();
    end(&tree(&sys, f.start.pid));
    let (args, env) = changed(f);
    let started = launch(&f.start, &args, &env).map_err(|e| format!("couldn't be started again: {e}"))?;
    let t0 = Instant::now();
    while t0.elapsed() < f.kind.within() {
        std::thread::sleep(Duration::from_secs(2));
        if answers_on_network(home, f.listen.port) {
            return Ok(format!(
                "started again listening on every interface (it listened on this computer only), in {} s",
                t0.elapsed().as_secs()
            ));
        }
    }
    // never leave the owner's server off: back as it was
    let sys = processes();
    end(&tree(&sys, Pid::from_u32(started)));
    let back = launch(&f.start, &f.start.args, &f.start.env);
    Err(format!(
        "didn't answer on the network within {} s when started again, so it was started as before{}",
        f.kind.within().as_secs(),
        if back.is_err() { " (and that failed too: start it yourself)" } else { "" }
    ))
}

/// Starts that were tried and failed, not tried again while the app runs: kind, program and arguments.
static TRIED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn off() -> bool {
    ["FACE_LMSTUDIO", "FACE_KEEP_SERVERS"]
        .iter()
        .any(|v| std::env::var(v).is_ok_and(|v| matches!(v.to_lowercase().as_str(), "off" | "0" | "no")))
}

/// One round: every server of this user's that listens on this computer only is started again open.
pub(crate) fn keep_once(home: &Path) {
    if off() || crate::hardware::own_addresses(home, None).is_empty() {
        return;
    }
    let (open, cannot) = look(&processes());
    let mut said = read(home);
    // (one that answers on the network already, through its own service's settings or anything else, is left be)
    for (kind, port, why) in cannot.into_iter().filter(|(_, port, _)| !answers_on_network(home, *port)) {
        said.insert(format!("{}:{port}", kind.name()), json!({"said": why, "at": face_files::paths::utc_now()}));
    }
    for f in open.into_iter().filter(|f| !answers_on_network(home, f.listen.port)) {
        let key = format!("{:?} {} {}", f.kind, f.start.exe.display(), f.start.args.join(" "));
        if TRIED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).contains(&key) {
            continue;
        }
        let what = open_one(home, &f);
        if what.is_err() {
            TRIED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(key);
        }
        let text = what.unwrap_or_else(|e| e);
        println!("{} on port {}: {text}", f.kind.name(), f.listen.port);
        said.insert(
            format!("{}:{}", f.kind.name(), f.listen.port),
            json!({"said": text, "at": face_files::paths::utc_now()}),
        );
    }
    let _ = face_files::paths::atomic_write(
        &home.join(SAID),
        serde_json::to_string_pretty(&Value::Object(said)).unwrap_or_default().as_bytes(),
    );
}

fn read(home: &Path) -> serde_json::Map<String, Value> {
    std::fs::read(home.join(SAID))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// What the upkeep last said of the server on this port, for Settings.
pub(crate) fn said(home: &Path, port: u16) -> Option<String> {
    let all = read(home);
    let (_, v) = all.iter().find(|(k, _)| k.rsplit_once(':').is_some_and(|(_, p)| p == port.to_string()))?;
    v["said"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn ollama_host_is_read_as_ollama_reads_it() {
        let here = |v: Option<&str>| ollama_listen(v).here_only();
        assert_eq!(ollama_listen(None), Listen { host: "127.0.0.1".into(), port: 11434 });
        assert!(here(None) && here(Some("")) && here(Some("127.0.0.1:11434")) && here(Some("http://localhost:11500")));
        assert!(here(Some("[::1]:11434")));
        assert_eq!(ollama_listen(Some("127.0.0.1:11500")).port, 11500);
        assert!(
            !here(Some("0.0.0.0"))
                && !here(Some(":11434"))
                && !here(Some("10.1.2.3:11434"))
                && !here(Some("[::]:11434"))
        );
        assert_eq!(ollama_listen(Some("0.0.0.0")).port, 11434);
    }

    #[test]
    fn llama_cpp_and_vllm_listen_where_their_arguments_say() {
        let none = HashMap::new();
        assert!(llama_listen(&s(&["-m", "m.gguf"]), &none).here_only(), "llama.cpp: this computer unless given");
        assert_eq!(llama_listen(&s(&["--port", "9000"]), &none).port, 9000);
        assert!(!llama_listen(&s(&["--host", "0.0.0.0"]), &none).here_only());
        assert!(
            !llama_listen(&s(&[]), &HashMap::from([("LLAMA_ARG_HOST".to_string(), "0.0.0.0".to_string())])).here_only()
        );
        assert!(!vllm_listen(&s(&["serve", "m"])).here_only(), "vLLM: every interface unless given");
        assert!(vllm_listen(&s(&["serve", "m", "--host=127.0.0.1"])).here_only());
        assert_eq!(vllm_listen(&s(&["serve", "m"])).port, 8000);
    }

    #[test]
    fn a_server_is_started_again_with_only_its_host_changed() {
        assert_eq!(
            open_args(&s(&["-m", "a.gguf", "--host", "127.0.0.1", "--port", "8080"])),
            s(&["-m", "a.gguf", "--host", "0.0.0.0", "--port", "8080"])
        );
        assert_eq!(open_args(&s(&["serve", "m", "--host=localhost"])), s(&["serve", "m", "--host=0.0.0.0"]));
        assert_eq!(open_args(&s(&["-m", "a.gguf"])), s(&["-m", "a.gguf", "--host", "0.0.0.0"]));
        // Ollama: its environment, everything kept but OLLAMA_HOST (its models' folder too)
        let f = Found {
            kind: Kind::Ollama,
            listen: Listen { host: "127.0.0.1".into(), port: 11434 },
            start: Start {
                pid: Pid::from_u32(1),
                exe: PathBuf::from("ollama"),
                args: s(&["serve"]),
                cwd: None,
                env: vec![("OLLAMA_HOST".into(), "127.0.0.1:11434".into()), ("OLLAMA_MODELS".into(), "/models".into())],
            },
            set: Some(("OLLAMA_HOST".into(), "0.0.0.0:11434".into())),
        };
        let (args, env) = changed(&f);
        assert_eq!(args, s(&["serve"]));
        assert_eq!(
            env,
            vec![("OLLAMA_MODELS".into(), "/models".into()), ("OLLAMA_HOST".into(), "0.0.0.0:11434".into())]
        );
    }

    #[test]
    fn servers_are_told_by_their_program_and_command() {
        assert_eq!(kind_of("ollama", &s(&["ollama", "serve"])), Some(Kind::Ollama));
        assert_eq!(
            kind_of("ollama", &s(&["ollama", "runner", "--model", "x"])),
            None,
            "a model's runner is the server's"
        );
        assert_eq!(kind_of("llama-server", &s(&["llama-server", "-m", "x"])), Some(Kind::LlamaCpp));
        assert_eq!(kind_of("interp", &s(&["interp", "/opt/venv/bin/vllm", "serve", "m"])), Some(Kind::Vllm));
        assert_eq!(kind_of("interp", &s(&["interp", "-m", "vllm.entrypoints.openai.api_server"])), Some(Kind::Vllm));
        assert_eq!(kind_of("interp", &s(&["interp", "train.x"])), None);
        assert!(ollama_app("ollama app", None));
        assert!(ollama_app("Ollama", Some(Path::new("/Applications/Ollama.app/Contents/MacOS/Ollama"))));
        assert!(!ollama_app("ollama", Some(Path::new("/usr/local/bin/ollama"))));
    }

    #[test]
    fn the_upkeep_is_off_where_every_check_switches_lm_studio_s_off() {
        let src = include_str!("servekeep.rs");
        assert!(src.contains(r#"["FACE_LMSTUDIO", "FACE_KEEP_SERVERS"]"#));
    }
}
