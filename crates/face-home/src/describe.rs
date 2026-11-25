//! Describe photo's settings:
//! which describer to use, which kind of server it asks (LM Studio, Ollama, llama.cpp or vLLM: the owner,
//! 2026-11-25, "I want to open up support for llama.cpp, vLLM and Ollama, When you select one - it should
//! show the relevant fields that are required for that platform"), the server's address, a model (or
//! automatic), an optional API key
//! kept protected for this user, whether Describe all is shared with the owner's other devices, and an
//! unfinished Describe everything again. In `HOME/describe.json`.

use std::{net::IpAddr, path::Path};

use face_files::{
    filejson,
    paths::utc_now,
    protect::{protect, reveal, write_private},
};
use serde_json::{Map, Value, json};

const FILE: &str = "describe.json";
/// server if it answers, else the built-in model / the server only / the built-in model only
pub const USES: [&str; 3] = ["auto", "server", "builtin"];
/// The kinds of server the describer asks: (its word, its name, the port it listens on as it comes). Each
/// answers the same chat requests (`/v1/chat/completions`); they differ in how they list their models and
/// say which of them read pictures, and in whether they take an API key.
pub const PLATFORMS: [(&str, &str, u16); 4] = [
    ("lmstudio", "LM Studio", 1234),
    ("ollama", "Ollama", 11434),
    ("llamacpp", "llama.cpp", 8080),
    ("vllm", "vLLM", 8000),
];

/// The settings, with what goes for one that isn't set.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// "auto", "server" or "builtin"
    pub choice: String,
    /// the kind of server: one of `PLATFORMS`
    pub platform: String,
    pub url: String,
    /// "" for automatic
    pub model: String,
    /// the server's API key ("" for none): it belongs to `url` and goes nowhere else
    pub key: String,
    /// Describe all also uses the LM Studio servers of the owner's other devices
    pub spread: bool,
    /// an unfinished Describe everything again: (when it was started, UTC; its model)
    pub again: Option<(String, String)>,
}

/// The describer's server when none is set (`FACE_LLM_URL` names another).
pub fn default_url() -> String {
    std::env::var("FACE_LLM_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:1234".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// The address a kind of server has on this computer as it comes (LM Studio's: `default_url`).
pub fn default_url_for(platform: &str) -> String {
    match PLATFORMS.iter().find(|(p, _, _)| *p == platform) {
        Some((p, _, port)) if *p != "lmstudio" => format!("http://127.0.0.1:{port}"),
        _ => default_url(),
    }
}

fn kept(home: &Path) -> Map<String, Value> {
    let read = std::fs::read(home.join(FILE)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    read.and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// A stored time is one: "2026-11-12T10:20:30Z", or an older "2026-11-12 10:20:30".
fn is_time(text: &str) -> bool {
    let b = text.as_bytes();
    let digits = |at: usize, n: usize| b.get(at..at + n).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    b.len() >= 16
        && digits(0, 4)
        && b[4] == b'-'
        && digits(5, 2)
        && b[7] == b'-'
        && digits(8, 2)
        && matches!(b[10], b'T' | b' ')
        && digits(11, 2)
        && digits(14, 2)
}

pub fn config(home: &Path) -> Config {
    let c = kept(home);
    let text = |k: &str| c.get(k).and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
    let again = c.get("again").and_then(|a| {
        let (since, model) = (a.get("since")?.as_str()?, a.get("model")?.as_str()?);
        (is_time(since) && !model.is_empty()).then(|| (since.to_string(), model.to_string()))
    });
    let platform = text("platform")
        .filter(|p| PLATFORMS.iter().any(|(known, _, _)| known == p))
        .unwrap_or_else(|| "lmstudio".to_string());
    Config {
        choice: text("use").filter(|u| USES.contains(&u.as_str())).unwrap_or_else(|| "auto".to_string()),
        url: text("url").unwrap_or_else(|| default_url_for(&platform)),
        platform,
        model: text("model").unwrap_or_default(),
        key: text("key").and_then(|k| reveal(&k).ok()).unwrap_or_default(),
        spread: c.get("spread") == Some(&Value::Bool(true)),
        again,
    }
}

/// describe.json, with the API key protected for this user.
fn write(home: &Path, c: &Config) -> Result<(), String> {
    let key = if c.key.is_empty() { String::new() } else { protect(&c.key)? };
    let again = c.again.as_ref().map(|(since, model)| json!({"since": since, "model": model}));
    let kept = json!({"use": c.choice, "url": c.url, "model": c.model, "key": key, "spread": c.spread, "again": again,
                      "platform": c.platform});
    write_private(&home.join(FILE), filejson::dumps_lines(&kept, true, 1).as_bytes())
}

/// "host:port", "http://host:port/v1/" ... as "http://host:port"; why not, for what isn't a server's address.
pub fn clean_url(url: &str) -> Result<String, String> {
    let not_one =
        || "That isn't a server address: use one like http://127.0.0.1:1234 or http://192.168.1.20:1234".to_string();
    let mut url = url.trim().to_string();
    if !url.is_empty() && !url.contains("://") {
        url = format!("http://{url}");
    }
    let mut url = url.trim_end_matches('/').to_string();
    if url.ends_with("/v1") {
        url.truncate(url.len() - 3);
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(not_one)?;
    if !["http", "https"].contains(&scheme.to_lowercase().as_str()) {
        return Err(not_one());
    }
    let place = rest.split(['/', '?', '#']).next().unwrap_or("");
    let place = place.rsplit_once('@').map_or(place, |(_, host)| host);
    // (an address in brackets is an IPv6 one: its colons are not the port's)
    let (host, port) = match place.strip_prefix('[') {
        Some(inside) => {
            let (host, after) = inside.split_once(']').ok_or_else(not_one)?;
            (host, after.strip_prefix(':'))
        }
        None => place.rsplit_once(':').map_or((place, None), |(host, port)| (host, Some(port))),
    };
    if host.is_empty() {
        return Err(not_one());
    }
    if port.is_some_and(|p| !p.is_empty() && p.parse::<u16>().is_err()) {
        return Err("The port in that address isn't a number".to_string());
    }
    Ok(url)
}

/// The host of a server's address, as the address names it (lower case, an IPv6 one without its brackets).
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let place = rest.split(['/', '?', '#']).next().unwrap_or("");
    let place = place.rsplit_once('@').map_or(place, |(_, host)| host);
    let host = match place.strip_prefix('[') {
        Some(inside) => inside.split(']').next().unwrap_or(""),
        None => place.rsplit_once(':').map_or(place, |(host, _)| host),
    };
    host.to_lowercase()
}

/// "this computer", "your network" or "the internet" for a server's address, and whether pictures sent
/// there would travel unencrypted over the internet.
pub fn where_is(url: &str) -> (&'static str, bool) {
    let host = host_of(url);
    if ["localhost", "127.0.0.1", "::1"].contains(&host.as_str()) {
        return ("this computer", false);
    }
    match host.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() => return ("this computer", false),
        Ok(IpAddr::V4(ip)) => {
            let [a, b, ..] = ip.octets();
            // (what is not an address of the internet: private, link-local, and the ranges kept for other uses)
            let kept_aside = a == 0 || a >= 240 || (a == 198 && (b == 18 || b == 19)) || ip.is_documentation();
            if ip.is_private() || ip.is_link_local() || kept_aside {
                return ("your network", false);
            }
        }
        Ok(IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            if first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80 {
                return ("your network", false);
            }
        }
        Err(_) => {
            if [".local", ".lan", ".home", ".internal"].iter().any(|end| host.ends_with(end)) || !host.contains('.') {
                return ("your network", false);
            }
        }
    }
    ("the internet", !url.to_lowercase().starts_with("https://"))
}

/// Change some of the settings ("use", "platform", "url", "model", "key", "spread", each only when it is among
/// the changes); an empty url means the platform's usual one. A key belongs to its server: a changed address
/// clears it, unless a key comes with the change. Another platform takes its own usual address where the old
/// one's was set, and its model is picked anew.
pub fn set_config(home: &Path, changes: &Map<String, Value>) -> Result<Config, String> {
    let mut c = config(home);
    let text = |k: &str| changes.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    if changes.contains_key("use") {
        let choice = text("use");
        if !USES.contains(&choice.as_str()) {
            return Err("Unknown choice".to_string());
        }
        c.choice = choice;
    }
    if changes.contains_key("platform") {
        let platform = text("platform");
        if !PLATFORMS.iter().any(|(p, _, _)| *p == platform) {
            return Err("Unknown kind of server".to_string());
        }
        if platform != c.platform {
            if c.url == default_url_for(&c.platform) {
                let url = default_url_for(&platform);
                if !changes.contains_key("key") {
                    c.key = String::new();
                }
                c.url = url;
            }
            c.model = String::new();
            c.platform = platform;
        }
    }
    if changes.contains_key("url") {
        let url = if text("url").is_empty() { default_url_for(&c.platform) } else { clean_url(&text("url"))? };
        if url != c.url && !changes.contains_key("key") {
            c.key = String::new();
        }
        c.url = url;
    }
    if changes.contains_key("model") {
        c.model = text("model").chars().take(200).collect();
    }
    if changes.contains_key("key") {
        c.key = text("key").chars().take(500).collect();
    }
    if changes.contains_key("spread") {
        c.spread = changes["spread"] == Value::Bool(true);
    }
    write(home, &c)?;
    Ok(c)
}

/// Start a Describe everything again with a model (from now), or with none note that it is finished.
pub fn set_again(home: &Path, model: Option<&str>) -> Result<Option<(String, String)>, String> {
    if model.is_some() {
        // descriptions are timed to the second: start in the next one, so that one made just before the
        // start is never taken for one made since
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        std::thread::sleep(std::time::Duration::from_secs_f64(1.01 - now.fract()));
    }
    let mut c = config(home);
    c.again = model.map(|m| (utc_now(), m.chars().take(200).collect()));
    write(home, &c)?;
    Ok(c.again)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_servers_address_is_tidied_or_refused() {
        for (given, kept) in [
            ("127.0.0.1:1234", "http://127.0.0.1:1234"),
            (" http://192.168.1.20:1234/v1/ ", "http://192.168.1.20:1234"),
            ("https://models.example/", "https://models.example"),
            ("http://[::1]:1234", "http://[::1]:1234"),
            ("studio.local", "http://studio.local"),
        ] {
            assert_eq!(clean_url(given).as_deref(), Ok(kept), "{given}");
        }
        for bad in ["", "ftp://host", "http://", "http://:1234"] {
            assert!(clean_url(bad).is_err_and(|e| e.starts_with("That isn't a server address")), "{bad}");
        }
        assert_eq!(clean_url("http://host:port").unwrap_err(), "The port in that address isn't a number");
        assert_eq!(clean_url("host:99999").unwrap_err(), "The port in that address isn't a number");
    }

    #[test]
    fn where_a_server_is_says_whether_photos_leave_the_computer() {
        for here in ["http://127.0.0.1:1234", "http://localhost:1234", "http://[::1]:1234", "http://127.5.5.5"] {
            assert_eq!(where_is(here), ("this computer", false), "{here}");
        }
        for near in [
            "http://192.168.1.20:1234",
            "http://10.1.2.3",
            "http://172.20.0.5",
            "http://169.254.1.1",
            "http://studio.local:1234",
            "http://studio",
            "http://[fe80::1]:1234",
            "http://[fd00::5]",
        ] {
            assert_eq!(where_is(near), ("your network", false), "{near}");
        }
        assert_eq!(where_is("http://models.example:1234"), ("the internet", true));
        assert_eq!(where_is("https://models.example"), ("the internet", false));
        assert_eq!(where_is("http://8.8.8.8:1234"), ("the internet", true));
    }

    #[test]
    fn the_settings_are_kept_and_a_key_goes_with_its_server() {
        let home = std::env::temp_dir().join(format!("face-home-describe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let change = |v: Value| set_config(&home, v.as_object().unwrap());
        let first = config(&home);
        assert_eq!(
            (
                first.choice.as_str(),
                first.url.clone(),
                first.model.as_str(),
                first.key.as_str(),
                first.spread,
                first.again.clone()
            ),
            ("auto", default_url(), "", "", false, None)
        );
        let c = change(json!({"use": "server", "url": "192.168.1.20:1234", "model": "  a model ", "key": " a key ", "spread": true})).unwrap();
        assert_eq!(
            (c.choice.as_str(), c.url.as_str(), c.model.as_str(), c.key.as_str(), c.spread),
            ("server", "http://192.168.1.20:1234", "a model", "a key", true)
        );
        assert_eq!(config(&home), c);
        let text = std::fs::read_to_string(home.join(FILE)).unwrap();
        assert!(!text.contains("a key") && text.starts_with("{\n \"use\": \"server\",\n \"url\": "), "{text}");
        // another address: the key stays behind; the same address, or a key given with it: kept
        assert_eq!(change(json!({"url": "http://192.168.1.20:1234/"})).unwrap().key, "a key");
        assert_eq!(change(json!({"url": "http://192.168.1.21:1234"})).unwrap().key, "");
        assert_eq!(change(json!({"url": "http://192.168.1.22:1234", "key": "another"})).unwrap().key, "another");
        assert_eq!(change(json!({"url": ""})).unwrap().url, default_url());
        assert_eq!(change(json!({"use": "the cloud"})).unwrap_err(), "Unknown choice");
        assert!(change(json!({"url": "ftp://x"})).is_err());
        assert!(!change(json!({"spread": "yes"})).unwrap().spread);
        // another kind of server: its own usual address where the old one's was set, its model picked anew
        change(json!({"url": "", "model": "m"})).unwrap();
        let o = change(json!({"platform": "ollama"})).unwrap();
        assert_eq!((o.platform.as_str(), o.url.as_str(), o.model.as_str()), ("ollama", "http://127.0.0.1:11434", ""));
        assert_eq!(config(&home).platform, "ollama");
        assert_eq!(change(json!({"url": ""})).unwrap().url, "http://127.0.0.1:11434", "empty: the platform's own");
        // an address of the owner's own stays when the kind changes
        change(json!({"url": "http://192.168.1.30:9000", "key": "k"})).unwrap();
        let v = change(json!({"platform": "vllm"})).unwrap();
        assert_eq!((v.url.as_str(), v.key.as_str()), ("http://192.168.1.30:9000", "k"));
        assert_eq!(change(json!({"platform": "the cloud"})).unwrap_err(), "Unknown kind of server");
        assert_eq!(default_url_for("llamacpp"), "http://127.0.0.1:8080");
        assert_eq!(default_url_for("lmstudio"), default_url());
        change(json!({"platform": "lmstudio", "url": ""})).unwrap();
        // an unfinished Describe everything again is kept until it is done
        let began = set_again(&home, Some("a model")).unwrap().unwrap();
        assert_eq!((began.0.len(), began.1.as_str(), config(&home).again), (20, "a model", Some(began.clone())));
        assert_eq!(change(json!({"model": "other"})).unwrap().again, Some(began));
        assert_eq!(set_again(&home, None).unwrap(), None);
        std::fs::write(home.join(FILE), r#"{"again": {"since": "some day", "model": "m"}, "use": 3}"#).unwrap();
        assert_eq!((config(&home).again, config(&home).choice), (None, "auto".to_string()));
        // a settings file from before the kinds of server: LM Studio
        assert_eq!(config(&home).platform, "lmstudio");
        let _ = std::fs::remove_dir_all(home);
    }
}
