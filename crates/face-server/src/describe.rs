//! Describe photo: a short summary of a photo from a vision-language model, to help work out the occasion
//! and who might be there (the part that asks a server).
//!
//! The model runs in LM Studio (http://127.0.0.1:1234, OpenAI-style API), by default on this computer.
//! Settings can point it at another address, also on another device or the internet (then each photo
//! described is sent there, and the Settings tab says so: `where`), and at any server with the same API.
//! Automatic: with LM Studio a loaded vision model is preferred, else the first one it lists; other servers
//! don't say which of their models read pictures, so their first model is used unless one is picked.
//! Reasoning is switched off (it spent most of the answer's tokens thinking, twice as long a photo).
//!
//! The model is told not to guess who anyone is: names come from the face recognizer and from the owner.
//! A server's answer is taken as it is: a redirect isn't followed (it would carry the API key and the photo
//! to another address), and an answer is read only up to a size. The API key goes to its own server only.
//!
//! Without a server the built-in model describes (`florence.rs`), once the owner has had it downloaded.

use std::{
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use face_files::filejson;
use face_home::describe::{Config, clean_url, config, default_url, where_is};
use serde_json::{Value, json};

use crate::florence;

/// the first call may wait for LM Studio to load the model
const TIMEOUT: Duration = Duration::from_secs(300);
/// bytes read from a describe server's answer
const MAX_ANSWER: u64 = 4_000_000;
/// characters of a description kept
const MAX_TEXT: usize = 4000;
const PROMPT: &str = "Describe this photo for someone sorting a family photo library, in 2-3 short sentences: the setting \
                      and occasion, how many people there are and what they're doing. Then, on a new line starting 'Text:', \
                      any readable text (signs, shirts, cakes, captions, dates), or 'Text: none'. \
                      Don't guess who anyone is or name anyone.";
/// What the built-in model is called.
pub const BUILTIN: &str = florence::NAME;

/// No model to ask (no server answers, or none that reads pictures), and why. `code`: what a server that
/// did answer refused with (401: it wants a key).
#[derive(Debug, Clone, PartialEq)]
pub struct Unavailable {
    pub why: String,
    pub code: Option<u16>,
}

fn unavailable(why: impl Into<String>) -> Unavailable {
    Unavailable { why: why.into(), code: None }
}

/// What a server said to one request.
enum Said {
    /// a 2xx answer that is JSON
    Json(Value),
    /// it answered with this status (a redirect too: it is never followed), and the start of what it said
    Status(u16, String),
    /// it answered, but not with JSON
    Unreadable,
    /// it answered with more than is read
    TooLarge,
    /// nothing there: refused, timed out, an unknown name
    Nothing(String),
}

/// A server is asked with this long for each step (connecting, sending, its answer's start, its answer),
/// not for all of them together: "localhost" is two addresses, and the first can take two seconds to refuse.
fn agent(timeout: Duration) -> ureq::Agent {
    let t = Some(timeout);
    let config =
        ureq::Agent::config_builder().max_redirects(0).max_redirects_will_error(false).http_status_as_error(false);
    let config = config
        .timeout_connect(t)
        .timeout_send_request(t)
        .timeout_send_body(t)
        .timeout_recv_response(t)
        .timeout_recv_body(t);
    config.build().into()
}

fn read(answer: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Said {
    let mut answer = match answer {
        Ok(a) => a,
        Err(e) => return Said::Nothing(e.to_string()),
    };
    let status = answer.status().as_u16();
    let mut bytes = Vec::new();
    if answer.body_mut().as_reader().take(MAX_ANSWER + 1).read_to_end(&mut bytes).is_err() {
        return Said::Nothing("the answer stopped".to_string());
    }
    if !(200..300).contains(&status) {
        return Said::Status(status, String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).into_owned());
    }
    if bytes.len() as u64 > MAX_ANSWER {
        return Said::TooLarge;
    }
    serde_json::from_slice(&bytes).map_or(Said::Unreadable, Said::Json)
}

fn get(url: &str, path: &str, key: &str, timeout: Duration) -> Said {
    let mut ask = agent(timeout).get(format!("{url}{path}"));
    if !key.is_empty() {
        ask = ask.header("Authorization", format!("Bearer {key}"));
    }
    read(ask.call())
}

/// A server's models as the program uses them: (id, whether it reads pictures: yes, no, or not said).
type Models = Vec<(String, Option<bool>)>;

/// The models a server offers, and those that are loaded. LM Studio says which are vision models and
/// which are loaded; other OpenAI-style servers only list ids. `at`: (address, key) of another server than
/// the saved one (the Test button).
pub fn server_models(c: &Config, at: Option<(&str, &str)>) -> Result<(Models, Vec<String>), Unavailable> {
    let (url, key) = at.unwrap_or((&c.url, &c.key));
    let no_answer = || {
        unavailable(format!(
            "No describe server answers at {url} (LM Studio: start it and turn on its local server; on another device, also 'Serve on local network')"
        ))
    };
    let listed = |v: &Value| v.get("data").and_then(Value::as_array).cloned();
    match get(url, "/api/v0/models", key, Duration::from_secs(3)) {
        // LM Studio's own list
        Said::Json(v) => {
            let rows = listed(&v)
                .filter(|rows| rows.iter().all(|m| m.is_object() && m.get("id").is_some_and(Value::is_string)));
            if let Some(rows) = rows.filter(|_| v.is_object()) {
                let kind = |m: &Value| m.get("type").and_then(Value::as_str).unwrap_or("").to_string();
                let id = |m: &Value| m["id"].as_str().unwrap_or("").to_string();
                let models = rows
                    .iter()
                    .filter(|m| ["vlm", "llm"].contains(&kind(m).as_str()))
                    .map(|m| (id(m), Some(kind(m) == "vlm")))
                    .collect();
                let loaded =
                    rows.iter().filter(|m| m.get("state").and_then(Value::as_str) == Some("loaded")).map(id).collect();
                return Ok((models, loaded));
            }
        }
        // nothing there: don't wait twice
        Said::Nothing(_) => return Err(no_answer()),
        // it answered, but isn't LM Studio: ask the standard way
        Said::Status(..) | Said::Unreadable | Said::TooLarge => {}
    }
    match get(url, "/v1/models", key, Duration::from_secs(3)) {
        Said::Json(v) if v.is_object() => {
            let rows = listed(&v).unwrap_or_default();
            Ok((
                rows.iter()
                    .filter_map(|m| Some((m.get("id")?.as_str().filter(|id| !id.is_empty())?.to_string(), None)))
                    .collect(),
                Vec::new(),
            ))
        }
        Said::Status(code, _) => {
            let hint = if code == 401 || code == 403 { ": check the API key" } else { "" };
            Err(Unavailable { why: format!("The describe server at {url} refused ({code}{hint})"), code: Some(code) })
        }
        _ => Err(no_answer()),
    }
}

type Found = Result<(Models, Vec<String>), Unavailable>;

/// The server's model to use: the one picked in Settings, else a loaded vision model, else the first vision
/// model (LM Studio), else the server's first model (other servers).
fn server_model(c: &Config, found: Option<Found>) -> Result<String, Unavailable> {
    let (models, loaded) = found.unwrap_or_else(|| server_models(c, None))?;
    if !c.model.is_empty() {
        if !models.is_empty() && !models.iter().any(|(m, _)| *m == c.model) {
            return Err(unavailable(format!(
                "The describe server has no model called {}: pick another in Settings",
                c.model
            )));
        }
        return Ok(c.model.clone());
    }
    let says = models.iter().any(|(_, vision)| vision.is_some());
    let vision: Vec<&String> = models.iter().filter(|(_, v)| !says || *v == Some(true)).map(|(m, _)| m).collect();
    let Some(first) = vision.first() else {
        return Err(unavailable(
            "The describe server has no vision model: download one (for example Qwen 3.6) and try again",
        ));
    };
    Ok((*vision.iter().find(|m| loaded.contains(m)).unwrap_or(first)).clone())
}

/// The vision model to use, as chosen in Settings.
///
/// The server's (the one picked, or automatic), else the built-in one if it is downloaded.
pub fn model(c: &Config, found: Option<Found>) -> Result<String, Unavailable> {
    let gb = florence::SIZE as f64 / 1e9;
    if c.choice == "builtin" {
        return match florence::ready() {
            true => Ok(BUILTIN.to_string()),
            false => Err(unavailable(format!("The built-in model isn't downloaded yet ({gb:.1} GB)"))),
        };
    }
    server_model(c, found).or_else(|e| {
        if c.choice == "auto" && florence::ready() {
            return Ok(BUILTIN.to_string());
        }
        if c.choice == "server" {
            return Err(e);
        }
        let why = format!("{}, or download the built-in model ({gb:.1} GB, plainer descriptions)", e.why);
        Err(Unavailable { why, code: e.code })
    })
}

/// {ok, model or why, builtin, config, models, helpers} for the page (never the API key).
/// `devices`: asked for the addresses of the owner's other devices, when the work may be shared out.
pub fn status(home: &Path, devices: &dyn Fn() -> Vec<String>) -> Value {
    let c = config(home);
    let (place, open) = where_is(&c.url);
    let mut out = json!({"builtin": florence::status(),
                         "config": {"use": c.choice, "url": c.url, "model": c.model, "key": !c.key.is_empty(), "default_url": default_url(),
                                    "where": place, "unencrypted": open},
                         "models": []});
    // ask the server once, for the list and for the model to use
    let found = (c.choice != "builtin").then(|| server_models(&c, None));
    if let Some(Ok((models, _))) = &found {
        out["models"] =
            json!(models.iter().filter(|(_, vision)| *vision != Some(false)).map(|(m, _)| m).collect::<Vec<_>>());
    }
    match model(&c, found) {
        Ok(name) => {
            out["ok"] = json!(true);
            out["model"] = json!(name);
        }
        Err(e) => {
            out["ok"] = json!(false);
            out["why"] = json!(e.why);
        }
    }
    out["config"]["spread"] = json!(c.spread);
    let helping = match out["model"].as_str().filter(|_| out["ok"] == true) {
        Some(name) => helpers(home, name, devices),
        None => Vec::new(),
    };
    out["helpers"] = json!(helping.iter().map(|(host, url)| json!({"host": host, "url": url})).collect::<Vec<_>>());
    out
}

/// Seconds to open a connection to an address's host and port (9 when it doesn't open).
fn connect_secs(url: &str) -> f64 {
    use std::net::ToSocketAddrs;
    let place = url.split_once("//").map_or(url, |(_, rest)| rest).split('/').next().unwrap_or("");
    let place = if place.rsplit_once(':').is_some_and(|(_, port)| port.parse::<u16>().is_ok()) {
        place.to_string()
    } else {
        format!("{place}:80")
    };
    let began = Instant::now();
    let opened = place
        .to_socket_addrs()
        .ok()
        .and_then(|mut found| found.next())
        .is_some_and(|to| std::net::TcpStream::connect_timeout(&to, Duration::from_millis(500)).is_ok());
    if opened { began.elapsed().as_secs_f64() } else { 9.0 }
}

/// The other LM Studio servers Describe all may also send photos to: (the computer's name, the address),
/// only with "spread" on and a server's model. Each is one of the owner's own devices: a computer in the
/// shared database, at an address it gave there (`devices`), or this computer; never whatever else on the
/// network answers like LM Studio with the model's name. Each must list the same model as a vision model,
/// so every description comes from the same model. The configured server itself isn't one of them. No API
/// key goes to them.
pub fn helpers(home: &Path, name: &str, devices: &dyn Fn() -> Vec<String>) -> Vec<(String, String)> {
    let c = config(home);
    if !c.spread || c.choice == "builtin" || name.is_empty() || name == BUILTIN {
        return Vec::new();
    }
    let main_here = where_is(&c.url).0 == "this computer";
    let mut ours = devices();
    ours.extend(crate::hardware::own_addresses(home, None).iter().map(ToString::to_string));
    let mut out = Vec::new();
    for e in crate::lmsnet::lmstudio_network(home).as_array().into_iter().flatten() {
        let urls: Vec<&str> = e["urls"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if urls.contains(&c.url.as_str()) || (main_here && e["this_computer"] == true) {
            continue;
        }
        // beyond the local network only as a "Fully distributed system"
        if urls.first().is_some_and(|first| where_is(first).0 == "the internet") && !crate::hardware::distributed(home)
        {
            continue;
        }
        if !e["models"].as_array().into_iter().flatten().any(|m| m["id"] == name && m["vision"] == true) {
            continue;
        }
        // not a device of the owner's (or not at an address it gave): left out
        let mut known: Vec<(f64, &str)> = urls
            .into_iter()
            .filter(|u| ours.contains(&face_home::describe::host_of(u)))
            .map(|u| (connect_secs(u), u))
            .collect();
        // the address that connects first (a computer on Wi-Fi and wired answers at both)
        known.sort_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, best)) = known.first() {
            out.push((e["host"].as_str().unwrap_or("").to_string(), best.to_string()));
        }
    }
    out
}

/// For Settings' Test button: what the server at an address offers, or why it doesn't answer.
pub fn test(home: &Path, url: &str, key: &str) -> Result<String, Unavailable> {
    let url = clean_url(url).map_err(unavailable)?;
    let c = config(home);
    // the saved key: its own server only
    let key = if !key.is_empty() {
        key
    } else if url == c.url {
        c.key.as_str()
    } else {
        ""
    };
    let (models, _) = server_models(&c, Some((&url, key)))?;
    let vision: Vec<&str> = models.iter().filter(|(_, v)| *v == Some(true)).map(|(m, _)| m.as_str()).collect();
    if models.iter().any(|(_, v)| v.is_some()) {
        // LM Studio
        let which = if vision.is_empty() {
            ". Download one (for example Qwen 3.6) in LM Studio".to_string()
        } else {
            format!(": {}", vision[..vision.len().min(5)].join(", "))
        };
        return Ok(format!("LM Studio answers: {} vision model(s){which}", vision.len()));
    }
    let first: Vec<&str> = models.iter().take(5).map(|(m, _)| m.as_str()).collect();
    Ok(format!(
        "The server answers: {} model(s){}",
        models.len(),
        if first.is_empty() { String::new() } else { format!(": {}", first.join(", ")) }
    ))
}

/// Describe one JPEG with the model `name` at the saved server. {text, model, secs}
/// `at`: one of `helpers` instead of the saved server (no API key is sent there).
pub fn ask(home: &Path, jpeg: &[u8], name: &str, at: Option<&str>) -> Result<Value, Unavailable> {
    if name == BUILTIN {
        return florence::describe(jpeg).map_err(unavailable);
    }
    let c = config(home);
    let url = at.unwrap_or(&c.url);
    // a key belongs to its own server only
    let key = if at.is_none_or(|at| at == c.url) { c.key.as_str() } else { "" };
    let picture = format!("data:image/jpeg;base64,{}", B64.encode(jpeg));
    let mut body = json!({"model": name, "temperature": 0.2, "max_tokens": 400, "reasoning_effort": "none",
                          "messages": [{"role": "user", "content": [{"type": "text", "text": PROMPT},
                                                                     {"type": "image_url", "image_url": {"url": picture}}]}]});
    let began = Instant::now();
    let mut said = Said::Unreadable;
    for attempt in 0..2 {
        let mut ask =
            agent(TIMEOUT).post(format!("{url}/v1/chat/completions")).header("Content-Type", "application/json");
        if !key.is_empty() {
            ask = ask.header("Authorization", format!("Bearer {key}"));
        }
        said = read(ask.send(filejson::dumps(&body).as_bytes()));
        match &said {
            // a server that doesn't know the setting: asked once more without it
            Said::Status(400 | 422, why) if attempt == 0 && why.contains("reasoning") => {
                if let Some(o) = body.as_object_mut() {
                    o.shift_remove("reasoning_effort");
                }
            }
            _ => break,
        }
    }
    let answer = match said {
        Said::Json(v) => v,
        Said::Status(code, why) => {
            return Err(unavailable(format!("The describe server couldn't describe it ({code}: {why})")));
        }
        Said::Nothing(why) => return Err(unavailable(format!("The describe server didn't answer: {why}"))),
        Said::Unreadable => return Err(unavailable("The describe server's answer wasn't understood")),
        Said::TooLarge => return Err(unavailable("The describe server's answer was too large")),
    };
    let message = answer.get("choices").and_then(|c| c.get(0)).and_then(|c| c.get("message")).filter(|m| m.is_object());
    let Some(message) = message else { return Err(unavailable("The describe server's answer wasn't understood")) };
    let text = match message.get("content") {
        None | Some(Value::Null) => "",
        Some(Value::String(text)) => text.as_str(),
        Some(other) if other == &json!(false) || other == &json!(0) || other == &json!([]) || other == &json!({}) => "",
        Some(_) => return Err(unavailable("The describe server's answer wasn't text")),
    };
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return Err(unavailable("The model gave an empty answer"));
    }
    Ok(json!({"text": text, "model": name, "secs": filejson::round(began.elapsed().as_secs_f64(), 1)}))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    use face_home::describe::set_config;
    use serde_json::Map;

    use super::*;

    /// What a stand-in server was asked: (path, headers in lower case, body).
    type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

    /// A stand-in describe server on this computer: answers each request with what `answer` makes of
    /// (method, path, headers in lower case, body), and keeps what it was asked.
    fn server(
        answer: impl Fn(&str, &str, &str, &Value) -> (u16, Vec<(String, String)>, String) + Send + 'static,
    ) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut first, mut headers, mut length) = (String::new(), String::new(), 0usize);
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    let line = line.to_lowercase();
                    if let Some(n) = line.strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap_or(0);
                    }
                    headers.push_str(&line);
                }
                let mut body = vec![0u8; length];
                if std::io::Read::read_exact(&mut reader, &mut body).is_err() {
                    continue;
                }
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let mut words = first.split_whitespace();
                let (method, path) = (words.next().unwrap_or("").to_string(), words.next().unwrap_or("").to_string());
                let (code, more, text) = answer(&method, &path, &headers, &body);
                kept.lock().unwrap().push((path, headers, body));
                let more: String = more.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{more}\r\n{text}",
                    text.len()
                );
            }
        });
        (url, seen)
    }

    fn home(name: &str, url: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("face-describe-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let mut c = Map::new();
        c.insert("url".to_string(), json!(url));
        c.insert("use".to_string(), json!("server"));
        set_config(&home, &c).unwrap();
        home
    }

    fn said(text: &str) -> String {
        json!({"choices": [{"message": {"role": "assistant", "content": text}}]}).to_string()
    }

    #[test]
    fn the_model_is_the_one_picked_else_a_loaded_vision_model_else_the_first() {
        let c = |model: &str| Config {
            choice: "server".into(),
            url: "http://127.0.0.1:9".into(),
            model: model.into(),
            key: String::new(),
            spread: false,
            again: None,
        };
        let lm = || {
            Some(Ok((
                vec![("a".to_string(), Some(true)), ("b".to_string(), Some(true)), ("t".to_string(), Some(false))],
                vec!["t".to_string(), "b".to_string()],
            )))
        };
        assert_eq!(model(&c(""), lm()).unwrap(), "b", "a loaded vision model before the first");
        assert_eq!(model(&c("a"), lm()).unwrap(), "a");
        assert!(model(&c("z"), lm()).unwrap_err().why.contains("no model called z"));
        let none_loaded = Some(Ok((vec![("t".to_string(), Some(false)), ("a".to_string(), Some(true))], vec![])));
        assert_eq!(model(&c(""), none_loaded).unwrap(), "a");
        let no_vision = Some(Ok((vec![("t".to_string(), Some(false))], vec![])));
        assert!(model(&c(""), no_vision).unwrap_err().why.contains("no vision model"));
        // a server that doesn't say which models read pictures: its first
        let other = Some(Ok((vec![("x".to_string(), None), ("y".to_string(), None)], vec![])));
        assert_eq!(model(&c(""), other).unwrap(), "x");
        assert!(model(&c(""), Some(Err(unavailable("nothing there")))).unwrap_err().why.contains("nothing there"));
        let mut builtin = c("");
        builtin.choice = "builtin".into();
        assert!(model(&builtin, lm()).is_err(), "the built-in model isn't here");
    }

    #[test]
    fn a_server_is_asked_for_its_models_and_about_a_picture() {
        let (url, seen) = server(|method, path, headers, body| match (method, path) {
            ("GET", "/api/v0/models") => (
                200,
                vec![],
                json!({"data": [{"id": "vision", "type": "vlm", "state": "loaded"}, {"id": "words", "type": "llm"},
                                                                      {"id": "numbers", "type": "embeddings"}]})
                .to_string(),
            ),
            ("POST", "/v1/chat/completions") if !headers.contains("authorization: bearer the-key") => {
                (401, vec![], "no key".to_string())
            }
            // a server that doesn't know the reasoning setting: asked once more without it
            ("POST", "/v1/chat/completions") if body.get("reasoning_effort").is_some() => {
                (400, vec![], "unknown: reasoning_effort".to_string())
            }
            ("POST", "/v1/chat/completions") => (200, vec![], said("  Two people at a table.\nText: none \n")),
            _ => (404, vec![], "{}".to_string()),
        });
        let home = home("asked", &url);
        let c = config(&home);
        assert_eq!(
            server_models(&c, None).unwrap(),
            (vec![("vision".to_string(), Some(true)), ("words".to_string(), Some(false))], vec!["vision".to_string()])
        );
        assert_eq!(test(&home, &url, "").unwrap(), "LM Studio answers: 1 vision model(s): vision");
        let st = status(&home, &Vec::new);
        assert_eq!(
            (&st["ok"], &st["model"], &st["models"], &st["config"]["key"], &st["config"]["where"]),
            (&json!(true), &json!("vision"), &json!(["vision"]), &json!(false), &json!("this computer"))
        );
        // without its key the server refuses, and that is what is said
        assert!(ask(&home, b"a picture", "vision", None).unwrap_err().why.contains("401: no key"));
        let mut key = Map::new();
        key.insert("key".to_string(), json!("the-key"));
        set_config(&home, &key).unwrap();
        seen.lock().unwrap().clear();
        let d = ask(&home, b"a picture", "vision", None).unwrap();
        assert_eq!((&d["text"], &d["model"]), (&json!("Two people at a table.\nText: none"), &json!("vision")));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "with the reasoning setting, then without");
        let first = &seen[0].2;
        assert_eq!(
            (&first["model"], &first["temperature"], &first["max_tokens"], &first["reasoning_effort"]),
            (&json!("vision"), &json!(0.2), &json!(400), &json!("none"))
        );
        assert_eq!(
            first["messages"][0]["content"][1]["image_url"]["url"],
            json!(format!("data:image/jpeg;base64,{}", B64.encode(b"a picture")))
        );
        assert!(first["messages"][0]["content"][0]["text"].as_str().unwrap().contains("Don't guess who anyone is"));
        assert!(seen[1].2.get("reasoning_effort").is_none() && seen[1].2["model"] == "vision");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_redirect_is_never_followed_and_odd_answers_are_refused() {
        let (elsewhere, followed) = server(|_, _, _, _| (200, vec![], said("from somewhere else")));
        let to = format!("{elsewhere}/v1/chat/completions");
        let (url, _) = server(move |_, path, _, body| match (path, body["model"].as_str().unwrap_or("")) {
            ("/v1/models", _) => (200, vec![], json!({"data": [{"id": "any"}, {"id": ""}, "not a row"]}).to_string()),
            (_, "moved") => (302, vec![("Location".to_string(), to.clone())], "moved".to_string()),
            (_, "large") => (200, vec![], said(&"x".repeat(MAX_ANSWER as usize))),
            (_, "long") => (200, vec![], said(&"w".repeat(MAX_TEXT + 500))),
            (_, "empty") => (200, vec![], said("   ")),
            (_, "nothing") => (200, vec![], json!({"choices": [{"message": {"content": null}}]}).to_string()),
            (_, "parts") => {
                (200, vec![], json!({"choices": [{"message": {"content": [{"type": "text"}]}}]}).to_string())
            }
            (_, "none") => (200, vec![], json!({"choices": []}).to_string()),
            (_, "words") => (200, vec![], "hello".to_string()),
            _ => (404, vec![], "{}".to_string()),
        });
        let home = home("odd", &url);
        // not LM Studio: asked the standard way, and its first model is the one
        assert_eq!(model(&config(&home), None).unwrap(), "any");
        assert_eq!(test(&home, &url, "").unwrap(), "The server answers: 1 model(s): any");
        let why = |name: &str| ask(&home, b"p", name, None).unwrap_err().why;
        assert_eq!(why("moved"), "The describe server couldn't describe it (302: moved)");
        assert!(followed.lock().unwrap().is_empty(), "the photo went nowhere else");
        assert_eq!(why("large"), "The describe server's answer was too large");
        assert_eq!(why("empty"), "The model gave an empty answer");
        assert_eq!(why("nothing"), "The model gave an empty answer");
        assert_eq!(why("parts"), "The describe server's answer wasn't text");
        assert_eq!(why("none"), "The describe server's answer wasn't understood");
        assert_eq!(why("words"), "The describe server's answer wasn't understood");
        assert_eq!(ask(&home, b"p", "long", None).unwrap()["text"].as_str().unwrap().len(), MAX_TEXT);
        // nothing there
        let gone = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port())
        };
        assert!(test(&home, &gone, "").unwrap_err().why.starts_with(&format!("No describe server answers at {gone}")));
        assert!(test(&home, "ftp://somewhere", "").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn nothing_is_shared_out_unless_the_owner_switched_it_on() {
        let home = home("spread", "http://127.0.0.1:9");
        // (neither the other devices nor the network are asked)
        let never = || -> Vec<String> { panic!("the other devices are asked only when the work is shared out") };
        assert!(helpers(&home, "vision", &never).is_empty());
        let mut on = Map::new();
        on.insert("spread".to_string(), json!(true));
        on.insert("use".to_string(), json!("builtin"));
        set_config(&home, &on).unwrap();
        assert!(helpers(&home, "vision", &never).is_empty(), "the built-in model is this computer's own");
        on.insert("use".to_string(), json!("server"));
        set_config(&home, &on).unwrap();
        assert!(helpers(&home, "", &never).is_empty() && helpers(&home, BUILTIN, &never).is_empty());
        // which of a computer's addresses connects first
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let open = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let closed = {
            let gone = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://127.0.0.1:{}/v1", gone.local_addr().unwrap().port())
        };
        assert!(connect_secs(&open) < 0.5 && connect_secs(&closed) == 9.0);
        let _ = std::fs::remove_dir_all(&home);
    }
}
