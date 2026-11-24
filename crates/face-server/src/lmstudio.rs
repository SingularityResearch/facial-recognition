//! LM Studio on this computer, for Settings (the part the labeling tool
//! answers): whether it is here and serving, the vision models other LM Studio servers on the network have
//! and this one lacks, getting those (from the shared models folder first, else with LM Studio's own `lms get`),
//! the shared folder itself (every computer puts its vision models there, each file listed with its size and
//! SHA-256), and the "Fully distributed system" setting.
//!
//! What other computers say is checked before it is used: a model's name before it reaches `lms get`, and
//! every path in the shared folder's list, which is never followed outside LM Studio's models and hub
//! folders. A copy is checked against the list's SHA-256 (that catches a damaged copy, not a list someone
//! rewrote: the folder is trusted like the computers that write to it).
//!
//! Not here: keeping LM Studio serving on all interfaces and announcing it (the app does that every minute:
//! with the app itself).

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, atomic::AtomicBool, atomic::Ordering},
    time::{Duration, Instant},
};

use face_files::{
    filejson,
    paths::{atomic_write, utc_now},
};
use ring::digest;
use serde_json::{Map, Value, json};

use crate::{
    hardware,
    http::Response,
    label::{Answer, Labeler, asked},
    lmsnet,
};

/// The routes of LM Studio's settings that are sent something.
pub(crate) const POSTS: [&str; 3] = ["/api/lmstudio/share", "/api/lmstudio/distributed", "/api/lmstudio/get"];

impl Labeler {
    /// `/api/lmstudio`: LM Studio on this computer, the models it lacks, and the distributed setting. It asks
    /// other programs and the network: never under the labeling data's lock.
    pub(crate) fn lmstudio_state(&self) -> Value {
        let mut st = status(&self.home);
        let missing = if st["installed"] == true {
            missing_models(&self.home, &lmsnet::lmstudio_network(&self.home))
        } else {
            Vec::new()
        };
        st["missing"] = json!(missing);
        st["distributed"] = json!(hardware::distributed(&self.home));
        st
    }

    /// What LM Studio's settings answer to something sent; None for a route that isn't one of them.
    pub(crate) fn lmstudio_post(&self, path: &str, body: &Map<String, Value>) -> Option<Answer> {
        Some(match path {
            // the shared models folder (this computer's setting): its models are put there behind the answer
            "/api/lmstudio/share" => set_share(&self.home, body.get("folder").and_then(Value::as_str).unwrap_or(""))
                .map_err(asked)
                .map(|()| {
                    let home = self.home.clone();
                    std::thread::spawn(move || publish(&home));
                    Response::json(json!({"ok": true, "shared": share_status(&self.home)}))
                }),
            // "Fully distributed system": every adapter, any network
            "/api/lmstudio/distributed" => set_distributed(&self.home, body.get("on") == Some(&Value::Bool(true)))
                .map_err(asked)
                .map(|()| Response::json(json!({"ok": true, "distributed": hardware::distributed(&self.home)}))),
            // download the models other LM Studio servers have
            "/api/lmstudio/get" => {
                let missing = missing_models(&self.home, &lmsnet::lmstudio_network(&self.home));
                let variants = missing.iter().filter_map(|m| m["variant"].as_str().map(str::to_string)).collect();
                get_models(&self.home, variants).map_err(asked).map(|()| Response::json(json!({"ok": true})))
            }
            _ => return None,
        })
    }
}

/// What was done, for Settings: the model download's progress ("get") and the shared folder's note.
static GET: Mutex<Option<Map<String, Value>>> = Mutex::new(None);
static SHARE_NOTE: Mutex<Option<String>> = Mutex::new(None);
/// one computer puts models into the shared folder at a time
static PUBLISHING: AtomicBool = AtomicBool::new(false);

fn held<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|e| e.into_inner())
}

fn user_home() -> PathBuf {
    PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
}

/// LM Studio's `lms` command, if it is on this computer.
pub fn lms_path() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "lms.exe" } else { "lms" };
    let home = user_home();
    let here = [home.join(".lmstudio"), home.join(".cache").join("lm-studio")]
        .into_iter()
        .map(|f| f.join("bin").join(exe))
        .find(|p| p.exists());
    here.or_else(|| std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(exe)).find(|p| p.is_file()))
}

fn command(lms: &Path, args: &[&str]) -> Command {
    let mut ask = Command::new(lms);
    ask.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        ask.creation_flags(0x0800_0000); // no console window
    }
    ask
}

/// What `lms` says to these words: (it ended well, its output), or None without it, or when it takes too long.
fn lms(args: &[&str], wait: Duration) -> Option<(bool, String)> {
    let mut child = command(&lms_path()?, args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut said = Vec::new();
        let _ = out.read_to_end(&mut said);
        said
    });
    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if began.elapsed() < wait => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    Some((status.success(), String::from_utf8_lossy(&reader.join().ok()?).into_owned()))
}

/// LM Studio's own server settings, where it keeps them.
pub fn config_path() -> Option<PathBuf> {
    let home = user_home();
    [home.join(".lmstudio"), home.join(".cache").join("lm-studio")]
        .into_iter()
        .map(|f| f.join(".internal").join("http-server-config.json"))
        .find(|p| p.exists())
}

/// The port this computer's LM Studio serves on.
pub fn port() -> u16 {
    lmsnet::lmstudio_port().unwrap_or(1234)
}

fn port_open(ip: Ipv4Addr, port: u16, wait: Duration) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from((ip, port)), wait).is_ok()
}

/// LM Studio's server answers on this computer. Patient: at a program's start the computer is busy loading
/// models, and a quick check said "not running" while the server was up.
pub fn running() -> bool {
    (0..2).any(|_| port_open(Ipv4Addr::LOCALHOST, port(), Duration::from_secs(1)))
}

/// LM Studio's server answers on this computer's network addresses (not only 127.0.0.1).
pub fn on_network(home: &Path) -> bool {
    hardware::own_addresses(home, None).into_iter().any(|ip| port_open(ip, port(), Duration::from_millis(250)))
}

// ----------------------------------------------------------------------------- the same models as the network

/// A model on this computer's disk, as `lms ls` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    pub key: Option<String>,
    pub variant: Option<String>,
    pub vision: bool,
}

/// the list of models on disk is kept this long: starting `lms` takes half a second, and Settings asks every 5 s
const MODELS_FOR: Duration = Duration::from_secs(60);
static MODELS: Mutex<Option<(Instant, Vec<Local>)>> = Mutex::new(None);

/// The models on disk may have changed (a download ended, they are about to be published): `lms` is asked
/// again the next time.
pub fn models_changed() {
    *held(&MODELS) = None;
}

/// What `lms ls --json` printed, as the models it lists; None when that isn't a list.
fn models_listed(said: &str) -> Option<Vec<Local>> {
    let listed: Value = serde_json::from_str(said).ok()?;
    let text = |m: &Value, k: &str| m.get(k).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
    let truthy = |v: Option<&Value>| {
        !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
            && v != Some(&json!(0))
            && v != Some(&json!(""))
    };
    Some(
        listed
            .as_array()?
            .iter()
            .filter(|m| m.is_object())
            .map(|m| Local {
                key: text(m, "modelKey"),
                variant: text(m, "selectedVariant").or_else(|| text(m, "modelKey")),
                vision: truthy(m.get("vision")),
            })
            .collect(),
    )
}

/// The models on disk here, or None without `lms`.
pub fn local_models() -> Option<Vec<Local>> {
    let mut kept = held(&MODELS);
    if let Some((at, listed)) = &*kept
        && at.elapsed() < MODELS_FOR
    {
        return Some(listed.clone());
    }
    let (ok, said) = lms(&["ls", "--json"], Duration::from_secs(60))?;
    let listed = models_listed(&said).filter(|_| ok)?;
    *kept = Some((Instant::now(), listed.clone()));
    Some(listed)
}

/// A model as `lms get` takes it: `<publisher>/<name>`, maybe `@<quantization>`. Names come from other
/// computers on the network and from the shared folder's list: anything else (an option, an address) is
/// never handed to `lms get --yes`.
pub fn is_variant(text: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '.' || c == '-';
    let name = |part: &str| part.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && part.chars().all(word);
    let (model, quant) = match text.split_once('@') {
        Some((model, quant)) => (model, Some(quant)),
        None => (text, None),
    };
    let parts: Vec<&str> = model.split('/').collect();
    parts.len() <= 2 && parts.iter().all(|p| name(p)) && quant.is_none_or(|q| !q.is_empty() && q.chars().all(word))
}

/// Vision models that other LM Studio servers on the network serve and this computer's hasn't got:
/// [{variant, from, in_share}]; variant is `<model>@<quantization>`, as `lms get` takes it. `network`: what
/// the search of the network found.
pub fn missing_models(home: &Path, network: &Value) -> Vec<Value> {
    let Some(here) = local_models() else { return Vec::new() };
    let have: Vec<&str> = here.iter().filter_map(|m| m.key.as_deref()).collect();
    let mut want: BTreeMap<String, String> = BTreeMap::new();
    for server in network.as_array().into_iter().flatten().filter(|s| s["this_computer"] != true) {
        for m in server["models"].as_array().into_iter().flatten() {
            let Some(id) = m["id"].as_str().filter(|id| m["vision"] == true && !have.contains(id)) else { continue };
            let variant = match m["quant"].as_str().filter(|q| !q.is_empty()) {
                Some(quant) => format!("{id}@{}", quant.to_lowercase()),
                None => id.to_string(),
            };
            want.entry(variant).or_insert_with(|| server["host"].as_str().unwrap_or("").to_string());
        }
    }
    // what's in the shared folder too
    let index = share(home).filter(|s| s.is_dir()).map(|s| read_index(&s)).unwrap_or_default();
    for variant in index.keys() {
        if !have.contains(&variant.split('@').next().unwrap_or("")) {
            want.entry(variant.clone()).or_insert_with(|| "the shared folder".to_string());
        }
    }
    let mut out: Vec<(String, String)> = want.into_iter().filter(|(v, _)| is_variant(v)).collect();
    // (sorted by code point)
    out.sort();
    out.into_iter()
        .map(|(variant, from)| json!({"variant": variant, "from": from, "in_share": index.contains_key(&variant)}))
        .collect()
}

/// The first percentage in a line of `lms get`'s progress ("12.5 %"), as a whole number.
fn percent(line: &str) -> Option<i64> {
    numbers(line).into_iter().find(|(_, rest)| rest.trim_start().starts_with('%')).map(|(n, _)| n as i64)
}

/// Every number in a line with what follows it, as a pattern reading from the left finds them (a number
/// with its decimals first; when what follows doesn't fit, `fits` is asked again without them).
fn numbers(line: &str) -> Vec<(f64, &str)> {
    let bytes = line.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut whole = i;
        while whole < bytes.len() && bytes[whole].is_ascii_digit() {
            whole += 1;
        }
        let mut end = whole;
        if bytes.get(whole) == Some(&b'.') && bytes.get(whole + 1).is_some_and(u8::is_ascii_digit) {
            end = whole + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
        out.push((line[i..end].parse().unwrap_or(0.0), &line[end..]));
        if end != whole {
            out.push((line[i..whole].parse().unwrap_or(0.0), &line[whole..]));
        }
        i = whole;
    }
    out
}

/// The sizes in a line of `lms get`'s progress ("1.2 GB / 17.7 GB"), in bytes, in the line's order.
fn sizes(line: &str) -> Vec<f64> {
    const UNITS: [(&str, f64); 7] = [
        ("kib", 1024.0),
        ("mib", 1_048_576.0),
        ("gib", 1_073_741_824.0),
        ("kb", 1e3),
        ("mb", 1e6),
        ("gb", 1e9),
        ("b", 1.0),
    ];
    let (mut out, mut taken) = (Vec::new(), 0usize);
    for (n, rest) in numbers(line) {
        // (a number inside one already taken with its unit isn't a second one)
        let at = line.len() - rest.len();
        if at < taken {
            continue;
        }
        let after = rest.trim_start();
        let low = after.to_lowercase();
        let unit = UNITS.iter().find(|(unit, _)| {
            low.starts_with(unit)
                && !after[unit.len()..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_')
        });
        if let Some((unit, bytes)) = unit {
            out.push(n * bytes);
            taken = line.len() - after.len() + unit.len();
        }
    }
    out
}

fn set(key: &str, value: Value) {
    if let Some(st) = held(&GET).as_mut() {
        st.insert(key.to_string(), value);
    }
}

fn seconds_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// `lms get <variant> --yes`, following its progress output (a percentage, and sizes like 1.2 GB / 17.7 GB):
/// (it ended well, the end of what it said).
fn lms_get(variant: &str) -> (bool, String) {
    let Some(lms) = lms_path() else { return (false, "LM Studio's lms command isn't on this computer.".to_string()) };
    let mut ask = command(&lms, &["get", variant, "--yes"]);
    // (what it says and what it warns of, in one stream, as it comes)
    let (mut child, out) = match std::io::pipe()
        .and_then(|(read, write)| Ok((ask.stdout(write.try_clone()?).stderr(write).spawn()?, read)))
    {
        Ok(started) => started,
        Err(e) => return (false, e.to_string()),
    };
    drop(ask);
    let (mut tail, mut line): (Vec<String>, Vec<u8>) = (Vec::new(), Vec::new());
    for byte in std::io::BufReader::new(out).bytes() {
        let Ok(byte) = byte else { break };
        if byte != b'\r' && byte != b'\n' {
            line.push(byte);
            continue;
        }
        let said = String::from_utf8_lossy(&line).trim().to_string();
        line.clear();
        if said.is_empty() {
            continue;
        }
        if let Some(pct) = percent(&said) {
            set("pct", json!(pct));
        }
        if let [done, total, ..] = sizes(&said)[..] {
            set("bytes", json!(done));
            set("total", json!(total));
        }
        tail.push(said);
        let extra = tail.len().saturating_sub(5);
        tail.drain(..extra);
    }
    let ok = child.wait().is_ok_and(|status| status.success());
    let said: Vec<char> = tail.join(" ").chars().collect();
    (ok, said[said.len().saturating_sub(300)..].iter().collect())
}

/// Download these models on a thread of its own (when asked in Settings): from the shared folder first (fast,
/// the very same files), else with `lms get` from LM Studio's own sources.
pub fn get_models(home: &Path, variants: Vec<String>) -> Result<(), String> {
    if lms_path().is_none() {
        return Err("LM Studio's lms command isn't on this computer.".to_string());
    }
    let variants: Vec<String> = variants.into_iter().filter(|v| is_variant(v)).collect();
    {
        let mut get = held(&GET);
        if get.as_ref().is_some_and(|st| st.get("running") == Some(&json!(true))) {
            return Ok(());
        }
        *get = json!({"running": true, "done": [], "now": null, "error": null, "how": null, "pct": null, "notes": []})
            .as_object()
            .cloned();
    }
    let home = home.to_path_buf();
    let add = |key: &str, text: String| {
        if let Some(list) = held(&GET).as_mut().and_then(|st| st.get_mut(key)).and_then(Value::as_array_mut) {
            list.push(json!(text));
        }
    };
    let run = move || {
        for v in variants {
            set("now", json!(v));
            set("pct", Value::Null);
            set("how", json!("copying from the shared folder"));
            set("fallback", Value::Null);
            match from_share(&home, &v, true) {
                Ok(true) => {
                    std::thread::sleep(Duration::from_secs(3)); // LM Studio notices new model files by itself
                    models_changed();
                    let key = v.split('@').next().unwrap_or("");
                    if local_models().unwrap_or_default().iter().any(|m| m.key.as_deref() == Some(key)) {
                        add("done", format!("{v} (copied from the shared folder)"));
                    } else {
                        // seen once: its own paused download of the model hid it
                        set(
                            "error",
                            json!(format!(
                                "{v}: copied from the shared folder, but LM Studio doesn't list it yet. If LM Studio has an unfinished download of it, cancel that in LM Studio's Downloads; else restart LM Studio."
                            )),
                        );
                    }
                    continue;
                }
                Ok(false) => {}
                // any failure there: straight to the internet, no retry
                Err(why) => {
                    add(
                        "notes",
                        format!("{v}: the shared folder's copy failed ({why}); downloaded from the internet instead"),
                    );
                    set(
                        "fallback",
                        json!(format!(
                            "the shared folder's copy failed ({})",
                            why.chars().take(120).collect::<String>()
                        )),
                    );
                }
            }
            set("how", json!("downloading from the internet (LM Studio's sources)"));
            for key in ["pct", "bytes", "total"] {
                set(key, Value::Null);
            }
            set("started", json!(seconds_now()));
            let (ok, why) = lms_get(&v);
            if !ok {
                set("error", json!(format!("{v}: {why}")));
                break;
            }
            add("done", format!("{v} (downloaded from the internet)"));
        }
        models_changed(); // what is on disk now is asked of lms again
        set("running", json!(false));
        set("now", Value::Null);
        set("how", Value::Null);
    };
    std::thread::Builder::new().name("lms-get".into()).spawn(run).map(|_| ()).map_err(|e| e.to_string())
}

/// The model download in progress, for the page's progress bar (None when none runs).
pub fn download() -> Option<Value> {
    let get = held(&GET);
    let g = get.as_ref().filter(|g| g.get("running") == Some(&json!(true)))?;
    let part = |k: &str| g.get(k).cloned().unwrap_or(Value::Null);
    Some(
        json!({"what": part("now"), "how": part("how"), "fallback": part("fallback"), "pct": part("pct"), "bytes": part("bytes"), "total": part("total"),
                "started": part("started"), "done": g.get("done").and_then(Value::as_array).map_or(0, Vec::len)}),
    )
}

/// LM Studio on this computer, for Settings.
pub fn status(home: &Path) -> Value {
    if lms_path().is_none() && config_path().is_none() {
        return json!({"installed": false});
    }
    let runs = running();
    // (announced and bind: what the app's upkeep did, `keep`; nothing when only the labeling tool runs)
    json!({"installed": true, "running": runs, "port": port(), "on_network": runs && on_network(home),
           "announced": held(&ANNOUNCED).clone(), "bind": held(&BIND).clone(),
           "get": held(&GET).clone(), "shared": share_status(home)})
}

// ----------------------------------------------------------------------------- kept as the owner wants it
// On every computer with this app and LM Studio (the owner's rule, 2026-11-10): LM Studio serves on all network
// interfaces and is announced on the local network, so that the owner's other devices find it. This changes
// another program's settings and restarts its server; `FACE_LMSTUDIO=off` turns all of it off.

/// what the upkeep did, for Settings
static ANNOUNCED: Mutex<Option<String>> = Mutex::new(None);
static BIND: Mutex<Option<String>> = Mutex::new(None);
/// the announcement this app keeps up, and for which port
static ACTIVE: Mutex<Option<(u16, Announcer)>> = Mutex::new(None);

/// ["--cors"] when LM Studio's own settings have CORS on, else nothing: a restart keeps what the owner chose
/// there. The app passes their choice on and never makes one.
fn cors() -> Vec<&'static str> {
    let kept: Option<Value> =
        config_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok());
    if kept.is_some_and(|c| c["cors"] == true) { vec!["--cors"] } else { Vec::new() }
}

/// Make LM Studio serve on all interfaces: its settings, and its running server. What was done, in words.
pub fn serve_everywhere(home: &Path) -> Vec<String> {
    let mut done = Vec::new();
    if let Some(file) = config_path() {
        let kept: Option<Value> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok());
        if let Some(Value::Object(mut settings)) = kept
            && settings.get("networkInterface") != Some(&json!("0.0.0.0"))
        {
            settings.insert("networkInterface".to_string(), json!("0.0.0.0"));
            // (never a half-written settings file of theirs)
            if atomic_write(&file, filejson::dumps_lines(&Value::Object(settings), true, 2).as_bytes()).is_ok() {
                done.push("its settings now serve on all network interfaces".to_string());
            }
        }
    }
    if running() && !on_network(home) && !hardware::own_addresses(home, None).is_empty() {
        let port = port().to_string();
        let _ = lms(&["server", "stop"], Duration::from_secs(30));
        let mut start = vec!["server", "start", "--port", &port, "--bind", "0.0.0.0"];
        start.extend(cors());
        if lms(&start, Duration::from_secs(90)).is_some_and(|(ok, _)| ok) {
            done.push("its server was started again on all network interfaces".to_string());
        } else {
            // never leave their server off: back as it was
            let _ = lms(&["server", "start", "--port", &port], Duration::from_secs(90));
            done.push("its server couldn't be started on all network interfaces; it runs as before".to_string());
        }
    }
    if !done.is_empty() {
        *held(&BIND) = Some(done.join("; "));
    }
    done
}

/// An announcement of this computer's LM Studio on the local network, kept up until it is stopped.
enum Announcer {
    /// a check's stand-in: what would be announced is written to this file, and nothing is said on the network
    Noted(PathBuf),
    /// a program of the system that announces while it runs (macOS `dns-sd`, Linux `avahi-publish-service`)
    #[cfg(not(windows))]
    Programs(Vec<std::process::Child>),
    #[cfg(windows)]
    System(windows_announce::Registered),
}

impl Announcer {
    fn start(name: &str, port: u16) -> Result<Announcer, String> {
        if let Some(file) = std::env::var_os("FACE_TEST_ANNOUNCE").map(PathBuf::from) {
            let note = filejson::dumps(&json!(["start", name, port, ["app=LM Studio", "api=openai", "path=/v1"]]));
            let mut log =
                std::fs::OpenOptions::new().create(true).append(true).open(&file).map_err(|e| e.to_string())?;
            writeln!(log, "{note}").map_err(|e| e.to_string())?;
            return Ok(Announcer::Noted(file));
        }
        Announcer::system(name, port)
    }

    #[cfg(windows)]
    fn system(name: &str, port: u16) -> Result<Announcer, String> {
        windows_announce::register(name, &host(), port).map(Announcer::System)
    }

    #[cfg(not(windows))]
    fn system(name: &str, port: u16) -> Result<Announcer, String> {
        let tool = if cfg!(target_os = "macos") { "dns-sd" } else { "avahi-publish-service" };
        let found = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|d| d.join(tool))
            .find(|p| p.is_file());
        let Some(tool) = found else {
            let how =
                if cfg!(target_os = "linux") { " (install avahi-utils: sudo apt install avahi-utils)" } else { "" };
            return Err(format!("no way to announce here{how}"));
        };
        let mut running = Vec::new();
        for kind in ["_lmstudio._tcp", "_http._tcp"] {
            let mut ask = Command::new(&tool);
            if cfg!(target_os = "macos") {
                ask.args(["-R", name, kind, "local", &port.to_string()]);
            } else {
                ask.args(["-s", name, kind, &port.to_string()]);
            }
            ask.args(["app=LM Studio", "api=openai", "path=/v1"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            running.push(ask.spawn().map_err(|e| e.to_string())?);
        }
        Ok(Announcer::Programs(running))
    }

    fn stop(self) {
        match self {
            Announcer::Noted(file) => {
                if let Ok(mut log) = std::fs::OpenOptions::new().append(true).open(file) {
                    let _ = writeln!(log, "{}", filejson::dumps(&json!(["stop"])));
                }
            }
            #[cfg(not(windows))]
            Announcer::Programs(running) => {
                for mut program in running {
                    let _ = program.kill();
                    let _ = program.wait();
                }
            }
            #[cfg(windows)]
            Announcer::System(registered) => registered.stop(),
        }
    }
}

/// Windows' own DNS-SD registration: the system's mDNS answers for this app.
#[cfg(windows)]
mod windows_announce {
    use std::ffi::c_void;

    use windows_sys::Win32::NetworkManagement::Dns::{
        DNS_SERVICE_INSTANCE, DNS_SERVICE_REGISTER_REQUEST, DnsServiceConstructInstance, DnsServiceDeRegister,
        DnsServiceFreeInstance, DnsServiceRegister,
    };

    /// the system's "asked, and being done" answer to a registration
    const PENDING: u32 = 9506;

    /// The two registrations (`_lmstudio._tcp`, `_http._tcp`), kept where they were made for as long as the
    /// system knows them: it holds on to the requests' addresses.
    /// (each request in a place of its own on purpose: a list that grows would move them)
    #[allow(clippy::vec_box)]
    pub struct Registered(Vec<Box<DNS_SERVICE_REGISTER_REQUEST>>);

    // SAFETY: the requests are only handed to the system's own calls, which may come from any thread
    unsafe impl Send for Registered {}

    unsafe extern "system" fn done(_: u32, _: *const c_void, _: *const DNS_SERVICE_INSTANCE) {}

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    pub fn register(name: &str, host: &str, port: u16) -> Result<Registered, String> {
        let pairs = [("app", "LM Studio"), ("api", "openai"), ("path", "/v1")];
        let (keys, values): (Vec<Vec<u16>>, Vec<Vec<u16>>) = pairs.iter().map(|(k, v)| (wide(k), wide(v))).unzip();
        let (keys, values): (Vec<*const u16>, Vec<*const u16>) =
            (keys.iter().map(|k| k.as_ptr()).collect(), values.iter().map(|v| v.as_ptr()).collect());
        let host = wide(&format!("{host}.local"));
        let mut made = Registered(Vec::new());
        for kind in ["_lmstudio._tcp", "_http._tcp"] {
            let service = wide(&format!("{name}.{kind}.local"));
            // SAFETY: every pointer is to text or a list that lives until the call returns; the system copies them
            let instance = unsafe {
                DnsServiceConstructInstance(
                    service.as_ptr(),
                    host.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    port,
                    0,
                    0,
                    pairs.len() as u32,
                    keys.as_ptr(),
                    values.as_ptr(),
                )
            };
            if instance.is_null() {
                made.stop();
                return Err("DnsServiceConstructInstance failed".to_string());
            }
            let request = Box::new(DNS_SERVICE_REGISTER_REQUEST {
                Version: 1,
                InterfaceIndex: 0,
                pServiceInstance: instance,
                pRegisterCompletionCallback: Some(done),
                pQueryContext: std::ptr::null_mut(),
                hCredentials: std::ptr::null_mut(),
                unicastEnabled: 0,
            });
            // SAFETY: the request stays where it is (boxed, kept in `made`) for as long as it is registered
            let code = unsafe { DnsServiceRegister(&*request, std::ptr::null_mut()) };
            if code != 0 && code != PENDING {
                // SAFETY: an instance the system made and nobody else holds
                unsafe { DnsServiceFreeInstance(instance) };
                made.stop();
                return Err(format!("DnsServiceRegister failed ({code})"));
            }
            made.0.push(request);
        }
        Ok(made)
    }

    impl Registered {
        pub fn stop(mut self) {
            for request in self.0.drain(..) {
                // SAFETY: the same request that was registered, still where it was
                unsafe {
                    DnsServiceDeRegister(&*request, std::ptr::null_mut());
                    DnsServiceFreeInstance(request.pServiceInstance);
                }
            }
        }
    }
}

/// This computer's name without its domain, as it is announced.
fn host() -> String {
    crate::device().0
}

/// This computer's LM Studio is announced already (by the system itself, an Avahi service file say, or by
/// this app).
fn announced_already(home: &Path) -> bool {
    let (own, port) = (hardware::own_addresses(home, None), port());
    lmsnet::mdns_services(Duration::from_secs(1)).iter().any(|a| a.port == port && own.contains(&a.ip))
}

/// Announce this computer's LM Studio while its server is up; stop when it is down; follow a changed port.
pub fn announce(home: &Path) {
    let mut active = held(&ACTIVE);
    let now = running().then(port);
    if active.as_ref().is_some_and(|(port, _)| Some(*port) != now) {
        if let Some((_, announcer)) = active.take() {
            announcer.stop();
        }
        *held(&ANNOUNCED) = None;
    }
    let Some(port) = now else {
        *held(&ANNOUNCED) = Some("not announced: LM Studio's server isn't running".to_string());
        return;
    };
    if active.is_some() {
        return;
    }
    if announced_already(home) {
        *held(&ANNOUNCED) = Some(format!("announced on the network by this computer itself (port {port})"));
        return;
    }
    let host = host();
    *held(&ANNOUNCED) = Some(match Announcer::start(&format!("LM Studio on {host}"), port) {
        Ok(announcer) => {
            *active = Some((port, announcer));
            format!("announced on the network by this app as LM Studio on {host} (port {port})")
        }
        Err(why) => format!("not announced: {why}"),
    });
}

/// One round of the upkeep: nothing without LM Studio here.
pub fn keep_once(home: &Path) {
    if lms_path().is_none() && config_path().is_none() {
        return;
    }
    serve_everywhere(home);
    announce(home);
    let getting = held(&GET).as_ref().is_some_and(|get| get.get("running") == Some(&json!(true)));
    if !getting {
        let home = home.to_path_buf();
        let _ = std::thread::Builder::new().name("lms-share".into()).spawn(move || publish(&home));
    }
}

/// The app: keep LM Studio serving on all interfaces and announced while this app runs, every minute
/// (`FACE_LMSTUDIO=off` turns it off).
pub fn keep(home: &Path) {
    let off = std::env::var("FACE_LMSTUDIO").is_ok_and(|v| matches!(v.to_lowercase().as_str(), "off" | "0" | "no"));
    if off {
        return;
    }
    let home = home.to_path_buf();
    let _ = std::thread::Builder::new().name("lmstudio-keeper".into()).spawn(move || {
        std::thread::sleep(Duration::from_secs(20)); // after the start: while the models load, checks time out
        loop {
            keep_once(&home);
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}

// ----------------------------------------------------------------------------- a shared models folder (a NAS, say)
// Layout: <share>/models/<publisher>/<repo>/<files> (as in LM Studio's models folder), <share>/hub/<owner>/<name>/
// (LM Studio's description of the model), <share>/index.json: {variant: {hub, files: [{path, size, sha256}], from,
// added}}.

const CONF: &str = "lmstudio.json";
const INDEX: &str = "index.json";

fn conf(home: &Path) -> Map<String, Value> {
    let kept: Option<Value> = std::fs::read(home.join(CONF)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    kept.and_then(|k| k.as_object().cloned()).unwrap_or_default()
}

/// The shared models folder, this computer's own setting.
pub fn share(home: &Path) -> Option<PathBuf> {
    conf(home).get("share").and_then(Value::as_str).filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Change some of this computer's own LM Studio settings (`HOME/lmstudio.json`), keeping the others.
fn save_conf(home: &Path, key: &str, value: Value) -> Result<(), String> {
    let mut c = conf(home);
    c.insert(key.to_string(), value);
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    atomic_write(&home.join(CONF), filejson::dumps(&Value::Object(c)).as_bytes()).map_err(|e| e.to_string())
}

/// Set (or with "" unset) the shared models folder; one that isn't there is made.
pub fn set_share(home: &Path, folder: &str) -> Result<(), String> {
    let folder = folder.trim().trim_matches('"');
    if !folder.is_empty() && !Path::new(folder).is_dir() {
        std::fs::create_dir_all(folder).map_err(|e| format!("Can't use {folder}: {e}"))?;
    }
    save_conf(home, "share", json!(folder))?;
    *held(&SHARE_NOTE) = None;
    Ok(())
}

/// The "Fully distributed system" setting: the search for LM Studio servers covers every network adapter,
/// and servers beyond the local network may be used. Off for a new copy.
pub fn set_distributed(home: &Path, on: bool) -> Result<(), String> {
    save_conf(home, "distributed", json!(on))?;
    lmsnet::forget(); // the next search looks again, at the adapters that count now
    Ok(())
}

/// LM Studio's models folder (from its settings) and where it keeps each model's description.
pub fn models_dir() -> PathBuf {
    let home = user_home().join(".lmstudio");
    let settings: Option<Value> =
        std::fs::read(home.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let named =
        settings.as_ref().and_then(|s| s.get("downloadsFolder")).and_then(Value::as_str).filter(|f| !f.is_empty());
    named.map_or_else(|| home.join("models"), PathBuf::from)
}

fn hub_dir() -> PathBuf {
    user_home().join(".lmstudio").join("hub").join("models")
}

/// A folder's files' names, in the order of paths (without regard to case on Windows).
fn file_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = (std::fs::read_dir(folder).into_iter().flatten().flatten())
        .filter(|f| f.path().is_file())
        .map(|f| f.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by_key(|name| if cfg!(windows) { name.to_lowercase() } else { name.clone() });
    names
}

/// {variant: (hub, files in the models folder)} for this computer's vision models: each model's LM Studio
/// description says which Hugging Face repos hold it; its files are those of the downloaded quantization and
/// its picture projector (mmproj).
fn local_variants() -> Vec<(String, String, Vec<String>)> {
    models_changed();
    let mut out: Vec<(String, String, Vec<String>)> = Vec::new();
    for m in local_models().unwrap_or_default() {
        let (Some(key), Some(variant)) = (m.key.filter(|_| m.vision), m.variant) else { continue };
        let Some((_, quant)) = variant.split_once('@') else { continue };
        let quant = quant.to_lowercase().replace('_', "");
        let manifest: Option<Value> = std::fs::read(hub_dir().join(&key).join("manifest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        // (a description that isn't as LM Studio writes them names no files at all)
        let repos = (|| {
            let mut repos = Vec::new();
            for d in manifest
                .as_ref()?
                .as_object()?
                .get("dependencies")
                .map_or(Some(&[][..]), |d| d.as_array().map(Vec::as_slice))?
            {
                for s in d.as_object()?.get("sources").map_or(Some(&[][..]), |s| s.as_array().map(Vec::as_slice))? {
                    if s.as_object()?.get("type") == Some(&json!("huggingface")) {
                        repos.push(format!("{}/{}", text_of(s.get("user")?), text_of(s.get("repo")?)));
                    }
                }
            }
            Some(repos)
        })()
        .unwrap_or_default();
        let mut files = Vec::new();
        for repo in repos {
            for name in file_names(&models_dir().join(&repo)) {
                let low = name.to_lowercase();
                if low.replace('_', "").contains(&quant) || low.contains("mmproj") {
                    files.push(format!("{repo}/{name}"));
                }
            }
        }
        if !files.is_empty() && !out.iter().any(|(v, _, _)| *v == variant) {
            out.push((variant, key, files));
        }
    }
    out
}

/// A value as text: a text as it is, anything else as JSON.
fn text_of(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(path: &Path) -> std::io::Result<String> {
    let (mut file, mut hash, mut buf) =
        (std::fs::File::open(path)?, digest::Context::new(&digest::SHA256), vec![0u8; 1 << 22]);
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hex(hash.finish().as_ref()));
        }
        hash.update(&buf[..n]);
    }
}

/// Copy a file through a .part file, hashing as it goes; with `want`, its SHA-256 must be that. Its SHA-256.
fn copy(src: &Path, dst: &Path, want: Option<&str>, progress: &mut dyn FnMut(usize)) -> Result<String, String> {
    let name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let said = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dst.parent().unwrap_or(Path::new("."))).map_err(said)?;
    let part = dst.with_file_name(format!("{}.part", name(dst)));
    let mut hash = digest::Context::new(&digest::SHA256);
    {
        let (mut from, mut to, mut buf) =
            (std::fs::File::open(src).map_err(said)?, std::fs::File::create(&part).map_err(said)?, vec![0u8; 1 << 22]);
        loop {
            let n = from.read(&mut buf).map_err(said)?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n]).map_err(said)?;
            hash.update(&buf[..n]);
            progress(n);
        }
    }
    let sha = hex(hash.finish().as_ref());
    if want.is_some_and(|want| want != sha) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: copied, but its SHA-256 differs from the shared folder's list", name(src)));
    }
    std::fs::rename(&part, dst).map_err(said)?;
    Ok(sha)
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// The folder names of a path from the shared folder's list, or None when it isn't a plain relative path:
/// other computers write that list, so a path that could leave the folder it is joined to (.., a drive, a
/// leading slash) is never used.
fn parts(rel: &Value) -> Option<Vec<String>> {
    let parts: Vec<String> = rel.as_str()?.replace('\\', "/").split('/').map(str::to_string).collect();
    let plain = |p: &String| !p.is_empty() && p != "." && p != ".." && !p.contains(':') && p == p.trim();
    parts.iter().all(plain).then_some(parts)
}

/// A path with its links followed, as far as it exists.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path.to_path_buf();
    loop {
        if let Ok(real) = at.canonicalize() {
            return rest.iter().rev().fold(real, |p, name| p.join(name));
        }
        match (at.file_name().map(|n| n.to_os_string()), at.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                at = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// `base` joined with a path from the shared folder's list; refused unless it stays inside `base` (links
/// included).
fn inside(base: &Path, rel: &Value) -> Result<PathBuf, String> {
    let out = parts(rel).map(|parts| parts.iter().fold(base.to_path_buf(), |p, part| p.join(part)));
    match out {
        Some(out) if resolved(&out).starts_with(resolved(base)) => Ok(out),
        _ => {
            let shown: String = text_of(rel).chars().take(200).collect();
            Err(format!("the shared folder's list names a path outside its folder: '{shown}'"))
        }
    }
}

/// One model of the shared folder's list, as `publish` writes it: {hub, files: [{path, size, sha256}]}.
fn entry_ok(e: &Value) -> bool {
    let file = |f: &Value| {
        f.get("path").is_some_and(|p| parts(p).is_some())
            && f.get("size").is_some_and(|s| s.is_u64())
            && f.get("sha256").and_then(Value::as_str).is_some_and(|s| s.chars().count() == 64)
    };
    e.get("hub").is_some_and(|h| parts(h).is_some())
        && e.get("files").and_then(Value::as_array).is_some_and(|files| !files.is_empty() && files.iter().all(file))
}

/// The shared folder's list of models; entries that aren't as `publish` writes them are left out.
fn read_index(share: &Path) -> Map<String, Value> {
    let index: Option<Value> = std::fs::read(share.join(INDEX)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    index.and_then(|i| i.as_object().cloned()).unwrap_or_default().into_iter().filter(|(_, e)| entry_ok(e)).collect()
}

/// Put this computer's vision models in the shared folder, those it hasn't got yet (one at a time).
pub fn publish(home: &Path) {
    let Some(s) = share(home).filter(|s| s.is_dir()) else { return };
    if PUBLISHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let done = (|| -> Result<(), String> {
        let mut index = read_index(&s);
        for (variant, hub, files) in local_variants() {
            let there = |f: &Value| {
                f["path"]
                    .as_str()
                    .and_then(|p| std::fs::metadata(s.join("models").join(p)).ok())
                    .is_some_and(|m| Some(m.len()) == f["size"].as_u64())
            };
            if index.get(&variant).is_some_and(|e| e["files"].as_array().is_some_and(|files| files.iter().all(there))) {
                continue;
            }
            *held(&SHARE_NOTE) = Some(format!("putting {variant} in the shared folder"));
            let mut listed = Vec::new();
            for rel in &files {
                let (src, dst) = (models_dir().join(rel), s.join("models").join(rel));
                let sha = copy(&src, &dst, None, &mut |_| {})?;
                listed.push(json!({"path": rel, "size": std::fs::metadata(&src).map_err(|e| e.to_string())?.len(), "sha256": sha}));
            }
            let hub_src = hub_dir().join(&hub);
            if hub_src.is_dir() {
                copy_tree(&hub_src, &s.join("hub").join(&hub)).map_err(|e| e.to_string())?;
            }
            // another computer may have added to it meanwhile
            index = read_index(&s);
            let host = crate::host_name().map(|n| n.split('.').next().unwrap_or("").to_string()).unwrap_or_default();
            index.insert(variant, json!({"hub": hub, "files": listed, "from": host, "added": utc_now()}));
            atomic_write(&s.join(INDEX), filejson::dumps_lines(&Value::Object(index.clone()), true, 1).as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    *held(&SHARE_NOTE) = done.err().map(|why| format!("couldn't put models in the shared folder: {why}"));
    PUBLISHING.store(false, Ordering::SeqCst);
}

/// Copy one model from the shared folder into LM Studio's models folder: Ok(true) when done, Ok(false) when
/// the folder hasn't got it. Each file is checked against the SHA-256 in the folder's own list, and nothing
/// is written outside LM Studio's models and hub folders whatever the list says. `progress`: say it in the
/// download's state.
pub fn from_share(home: &Path, variant: &str, progress: bool) -> Result<bool, String> {
    let Some(s) = share(home).filter(|s| s.is_dir()) else { return Ok(false) };
    let Some(entry) = read_index(&s).get(variant).cloned() else { return Ok(false) };
    let files = entry["files"].as_array().cloned().unwrap_or_default();
    let total: u64 = files.iter().filter_map(|f| f["size"].as_u64()).sum();
    if progress {
        set("total", json!(total));
        set("bytes", json!(0));
        set("started", json!(seconds_now()));
    }
    let mut done = 0u64;
    let mut moved = |n: usize| {
        done += n as u64;
        if progress {
            set("pct", json!(done * 100 / total.max(1)));
            set("bytes", json!(done));
        }
    };
    let mut added: Vec<PathBuf> = Vec::new();
    let copied = (|| -> Result<(PathBuf, PathBuf), String> {
        let mut todo = Vec::new();
        for f in &files {
            todo.push((f, inside(&s.join("models"), &f["path"])?, inside(&models_dir(), &f["path"])?));
        }
        let hub = (inside(&s.join("hub"), &entry["hub"])?, inside(&hub_dir(), &entry["hub"])?);
        for (f, src, dst) in todo {
            let (size, sha) = (f["size"].as_u64().unwrap_or(0), f["sha256"].as_str().unwrap_or(""));
            if std::fs::metadata(&dst).is_ok_and(|m| m.len() == size) && sha256(&dst).is_ok_and(|have| have == sha) {
                moved(size as usize);
                continue;
            }
            copy(&src, &dst, Some(sha), &mut moved)?;
            added.push(dst);
        }
        Ok(hub)
    })();
    let (hub_src, hub_dst) = match copied {
        Ok(hub) => hub,
        Err(why) => {
            // no half model for LM Studio to find
            for dst in added {
                let _ = std::fs::remove_file(dst);
            }
            return Err(why);
        }
    };
    if hub_src.is_dir() {
        copy_tree(&hub_src, &hub_dst).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// The shared folder for Settings: where it is, whether it can be reached, the models in it.
pub fn share_status(home: &Path) -> Value {
    let Some(s) = share(home) else { return json!({"share": null}) };
    let mut models: Vec<String> = if s.is_dir() { read_index(&s).keys().cloned().collect() } else { Vec::new() };
    models.sort();
    json!({"share": s.to_string_lossy(), "reachable": s.is_dir(), "models": models, "note": held(&SHARE_NOTE).clone()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_models_name_reaches_lms_get() {
        for name in ["qwen/qwen3.8-27b", "qwen/qwen3.8-27b@q4_k_m", "model", "a.b-c_d/e@Q8_0", "m@q"] {
            assert!(is_variant(name), "{name}");
        }
        for name in [
            "",
            "--yes",
            "-x/y",
            "a/b/c",
            "a/b@",
            "a/b@q@r",
            "a b",
            "http://x/y",
            "a/b@q 1",
            "/b",
            "a/",
            "a/.b",
            "@q",
            "a;b",
            "a/b@q;rm",
        ] {
            assert!(!is_variant(name), "{name}");
        }
    }

    #[test]
    fn progress_is_read_from_what_lms_get_prints() {
        assert_eq!(percent("Downloading 12.5 %  1.2 GB / 17.7 GB"), Some(12));
        assert_eq!(percent("99% done, then 100%"), Some(99));
        assert_eq!(percent("version 1.2.3 of 4"), None);
        assert_eq!(sizes("Downloading 12.5 %  1.2 GB / 17.7 GB"), [1.2e9, 17.7e9]);
        assert_eq!(sizes("512 KiB of 2MiB at 3 mb/s"), [524_288.0, 2_097_152.0, 3e6]);
        assert_eq!(sizes("v1.2.3 GB"), [2.3e9], "the number that stands before the unit");
        assert_eq!(sizes("12 Bytes, 5 B, 7 Bs"), [5.0]);
        assert!(sizes("no sizes: 15 files").is_empty());
    }

    #[test]
    fn the_models_on_disk_are_read_from_lms_ls() {
        let said = r#"[{"modelKey": "a/vision", "selectedVariant": "a/vision@q4_k_m", "vision": true, "sizeBytes": 5},
                       {"modelKey": "b/words", "vision": false}, {"modelKey": "c/odd", "selectedVariant": "", "vision": 1}, "not a model"]"#;
        let listed = models_listed(said).unwrap();
        let local = |key: &str, variant: &str, vision: bool| Local {
            key: Some(key.into()),
            variant: Some(variant.into()),
            vision,
        };
        assert_eq!(
            listed,
            [
                local("a/vision", "a/vision@q4_k_m", true),
                local("b/words", "b/words", false),
                local("c/odd", "c/odd", true)
            ]
        );
        assert_eq!(models_listed("{}"), None);
        assert_eq!(models_listed("not json"), None);
    }

    #[test]
    fn a_path_from_the_shared_list_never_leaves_its_folder() {
        let dir = std::env::temp_dir().join(format!("face-lmstudio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let base = dir.join("models");
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(
            inside(&base, &json!("pub/repo/file.gguf")).unwrap(),
            base.join("pub").join("repo").join("file.gguf")
        );
        assert_eq!(
            inside(&base, &json!("pub\\repo\\file.gguf")).unwrap(),
            base.join("pub").join("repo").join("file.gguf")
        );
        for bad in [
            json!("../x"),
            json!("a/../../x"),
            json!("/etc/x"),
            json!("C:/x"),
            json!("a//b"),
            json!("a/./b"),
            json!(" a/b"),
            json!("a/b "),
            json!(""),
            json!(5),
            json!(null),
            json!("\\\\server\\share\\x"),
        ] {
            let refused = inside(&base, &bad).unwrap_err();
            assert!(
                refused.starts_with("the shared folder's list names a path outside its folder"),
                "{bad}: {refused}"
            );
        }
        // an entry as publish writes it, and those that aren't
        let good =
            json!({"hub": "pub/name", "files": [{"path": "pub/repo/f.gguf", "size": 3, "sha256": "0".repeat(64)}]});
        assert!(entry_ok(&good));
        for (part, value) in [
            ("hub", json!("../hub")),
            ("hub", json!(7)),
            ("files", json!([])),
            ("files", json!("many")),
            ("files", json!([{"path": "../f", "size": 3, "sha256": "0".repeat(64)}])),
            ("files", json!([{"path": "f", "size": 3.5, "sha256": "0".repeat(64)}])),
            ("files", json!([{"path": "f", "size": -1, "sha256": "0".repeat(64)}])),
            ("files", json!([{"path": "f", "size": 3, "sha256": "short"}])),
            ("files", json!([{"path": "f", "size": 3}])),
        ] {
            let mut bad = good.clone();
            bad[part] = value;
            assert!(!entry_ok(&bad), "{bad}");
        }
        std::fs::write(
            dir.join(INDEX),
            json!({"ok/model@q4": good, "bad/model": {"hub": "x"}, "worse": 5}).to_string(),
        )
        .unwrap();
        assert_eq!(read_index(&dir).keys().collect::<Vec<_>>(), ["ok/model@q4"]);
        assert!(read_index(&base).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_is_checked_against_its_sha256_and_the_settings_keep_each_other() {
        let dir = std::env::temp_dir().join(format!("face-lmstudio-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("src.bin"), b"a model's file").unwrap();
        let mut moved = 0;
        let sha = copy(&dir.join("src.bin"), &dir.join("to").join("dst.bin"), None, &mut |n| moved += n).unwrap();
        assert_eq!((sha.len(), moved, sha256(&dir.join("to").join("dst.bin")).unwrap() == sha), (64, 14, true));
        assert_eq!(copy(&dir.join("src.bin"), &dir.join("again.bin"), Some(&sha), &mut |_| {}).unwrap(), sha);
        let refused =
            copy(&dir.join("src.bin"), &dir.join("wrong.bin"), Some(&"0".repeat(64)), &mut |_| {}).unwrap_err();
        assert_eq!(refused, "src.bin: copied, but its SHA-256 differs from the shared folder's list");
        assert!(!dir.join("wrong.bin").exists() && !dir.join("wrong.bin.part").exists(), "nothing half-copied stays");
        // this computer's own settings: the shared folder and the distributed switch, each keeping the other
        let home = dir.join("home");
        assert_eq!((share(&home), share_status(&home)), (None, json!({"share": null})));
        let folder = dir.join("shared").join("models");
        set_share(&home, &format!(" \"{}\" ", folder.display())).unwrap();
        set_distributed(&home, true).unwrap();
        assert!(folder.is_dir() && share(&home).as_deref() == Some(folder.as_path()) && hardware::distributed(&home));
        assert_eq!(
            share_status(&home),
            json!({"share": folder.to_string_lossy(), "reachable": true, "models": [], "note": null})
        );
        set_share(&home, "").unwrap();
        assert!(share(&home).is_none() && hardware::distributed(&home));
        assert_eq!(std::fs::read_to_string(home.join(CONF)).unwrap(), r#"{"share": "", "distributed": true}"#);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
