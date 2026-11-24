//! The keys saved on this device, and the photo describer's settings they belong with (Web search's
//! key for Google and its keys for Amazon, and the describe server's key): each
//! kept protected for this user (`face_files::protect`), in a file of its own in the private folder.
//! Nothing here reaches a page or a log.

use std::path::Path;

use face_files::{
    filejson,
    protect::{protect, reveal, write_private},
};
use serde_json::{Map, Value, json};

use crate::describe;

const GOOGLE: &str = "google_vision_key.txt";
const AWS: &str = "aws_rekognition.json";
fn all(text: &str, ok: impl Fn(char) -> bool) -> bool {
    text.chars().all(ok)
}

fn remove(file: &Path) -> Result<(), String> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("can't remove a saved key: {e}")),
        _ => Ok(()),
    }
}

/// The Google key saved on this device ("" when none, or when it can't be read here).
pub fn google_key(home: &Path) -> String {
    let stored = std::fs::read_to_string(home.join(GOOGLE)).unwrap_or_default();
    reveal(&stored).unwrap_or_default()
}

/// The Google key in use: `GOOGLE_VISION_API_KEY` when the environment has one, else the one saved here.
pub fn google_key_in_use(home: &Path) -> String {
    let set = std::env::var("GOOGLE_VISION_API_KEY").map(|k| k.trim().to_string()).unwrap_or_default();
    if set.is_empty() { google_key(home) } else { set }
}

/// Keep the Google key, or with an empty one remove it.
pub fn save_google_key(home: &Path, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return remove(&home.join(GOOGLE));
    }
    if !((20..=80).contains(&key.len()) && all(key, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        return Err("That doesn't look like a Google API key".to_string());
    }
    write_private(&home.join(GOOGLE), protect(key)?.as_bytes())
}

/// The Amazon key saved on this device: (access key ID, secret access key, region when one is kept).
pub fn aws(home: &Path) -> Option<(String, String, Option<String>)> {
    let bytes = std::fs::read(home.join(AWS)).ok()?;
    let kept: Value = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes)).ok()?;
    let text = |k: &str| kept.get(k).and_then(Value::as_str).filter(|t| !t.is_empty());
    Some((
        text("access_key_id")?.to_string(),
        reveal(text("secret_access_key")?).ok()?,
        text("region").map(str::to_string),
    ))
}

/// Keep the Amazon key, or with an empty key ID remove it.
pub fn save_aws(home: &Path, id: &str, secret: &str, region: Option<&str>) -> Result<(), String> {
    let (id, secret, region) =
        (id.trim(), secret.trim(), region.map(str::trim).filter(|r| !r.is_empty()).unwrap_or("us-east-1"));
    if id.is_empty() {
        return remove(&home.join(AWS));
    }
    let upper = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit();
    if !(id.len() == 20 && (id.starts_with("AKIA") || id.starts_with("ASIA")) && all(id, upper)) {
        return Err("That doesn't look like an AWS access key ID (20 characters starting with AKIA)".to_string());
    }
    if !(secret.len() == 40 && all(secret, |c| c.is_ascii_alphanumeric() || "/+=".contains(c))) {
        return Err("That doesn't look like an AWS secret access key (40 characters)".to_string());
    }
    let parts: Vec<&str> = region.split('-').collect();
    let named = |p: &&str| !p.is_empty() && all(p, |c| c.is_ascii_lowercase());
    let shaped = parts.len() >= 3
        && parts[0].len() == 2
        && parts[..parts.len() - 1].iter().all(named)
        && parts[parts.len() - 1].len() == 1
        && all(parts[parts.len() - 1], |c| c.is_ascii_digit());
    if !shaped {
        return Err("That doesn't look like an AWS region (for example us-east-1)".to_string());
    }
    let kept = json!({"access_key_id": id, "secret_access_key": protect(secret)?, "region": region});
    write_private(&home.join(AWS), filejson::dumps(&kept).as_bytes())
}

/// The photo describer's settings that every device may know, with what goes for one that isn't set:
/// which describer ("use"), the server's address ("url"), the model, and whether Describe all is shared
/// with the owner's other devices ("spread").
pub fn describer(home: &Path) -> [(&'static str, Value); 4] {
    let c = describe::config(home);
    [("use", json!(c.choice)), ("url", json!(c.url)), ("model", json!(c.model)), ("spread", json!(c.spread))]
}

/// The describe server's key saved on this device: (the server's address it belongs to, the key or "").
pub fn describe_key(home: &Path) -> (String, String) {
    let c = describe::config(home);
    (c.url, c.key)
}

fn one(what: &str, value: Value) -> Map<String, Value> {
    let mut change = Map::new();
    change.insert(what.to_string(), value);
    change
}

/// Set which describer is used ("use": auto, server or builtin) or its model; the other settings stay.
pub fn set_describer(home: &Path, what: &str, value: &Value) -> Result<(), String> {
    if what != "use" && what != "model" {
        return Err("Unknown setting".to_string());
    }
    describe::set_config(home, &one(what, value.clone())).map(|_| ())
}

/// Keep the describe server's key (500 characters at most), or with an empty one remove it; the other
/// settings stay as they are.
pub fn save_describe_key(home: &Path, key: &str) -> Result<(), String> {
    describe::set_config(home, &one("key", json!(key))).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_kept_protected_and_only_when_they_look_like_keys() {
        let home = std::env::temp_dir().join(format!("face-home-saved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let text = |file: &str| std::fs::read_to_string(home.join(file)).unwrap();

        assert_eq!(google_key(&home), "");
        // (made up, and put together here so that nothing in this file looks like a key to a scanner)
        let key = format!("{}-{}", "made_up_for_the_test", "0123456789");
        let key = key.as_str();
        save_google_key(&home, &format!(" {key}\n")).unwrap();
        assert_eq!(google_key(&home), key);
        assert!(!text(GOOGLE).contains(key), "kept protected");
        for bad in ["short", "has a space in it, and is long enough", &"x".repeat(81)] {
            assert_eq!(save_google_key(&home, bad).unwrap_err(), "That doesn't look like a Google API key");
        }
        // an older key, saved as it is, is still read
        std::fs::write(home.join(GOOGLE), format!("{key}\n")).unwrap();
        assert_eq!(google_key(&home), key);
        save_google_key(&home, "").unwrap();
        assert!(!home.join(GOOGLE).exists() && save_google_key(&home, "").is_ok());

        assert_eq!(aws(&home), None);
        let (id, secret) = (format!("AKIA{}", "TEST".repeat(4)), "made/Up+".repeat(5));
        let (id, secret) = (id.as_str(), secret.as_str());
        save_aws(&home, id, secret, None).unwrap();
        assert_eq!(aws(&home), Some((id.to_string(), secret.to_string(), Some("us-east-1".to_string()))));
        assert!(text(AWS).contains(id) && !text(AWS).contains(secret));
        save_aws(&home, id, secret, Some("eu-central-1")).unwrap();
        assert_eq!(aws(&home).unwrap().2.as_deref(), Some("eu-central-1"));
        assert!(save_aws(&home, "AKIAtooshort", secret, None).unwrap_err().contains("access key ID"));
        assert!(save_aws(&home, id, "short", None).unwrap_err().contains("secret access key"));
        for region in ["us", "us-east", "US-east-1", "us--1", "us-east-12"] {
            assert!(save_aws(&home, id, secret, Some(region)).unwrap_err().contains("region"), "{region}");
        }
        // written with a byte order mark by an editor: still read
        std::fs::write(
            home.join(AWS),
            format!("\u{feff}{{\"access_key_id\": \"{id}\", \"secret_access_key\": \"{secret}\"}}"),
        )
        .unwrap();
        assert_eq!(aws(&home), Some((id.to_string(), secret.to_string(), None)));
        save_aws(&home, " ", "", None).unwrap();
        assert!(!home.join(AWS).exists());

        // the describe server's key belongs to its address; the other settings stay when it is kept
        assert_eq!(describer(&home)[0], ("use", json!("auto")));
        assert_eq!((describer(&home)[2].1.clone(), describer(&home)[3].1.clone()), (json!(""), json!(false)));
        std::fs::write(
            home.join("describe.json"),
            r#"{"use": "server", "url": "http://10.1.2.3:1234", "model": "m", "spread": true, "again": null}"#,
        )
        .unwrap();
        assert_eq!(describe_key(&home), ("http://10.1.2.3:1234".to_string(), String::new()));
        save_describe_key(&home, " a key for that server ").unwrap();
        assert_eq!(describe_key(&home), ("http://10.1.2.3:1234".to_string(), "a key for that server".to_string()));
        assert!(!text("describe.json").contains("a key for that server"));
        assert_eq!(
            describer(&home).map(|(_, v)| v),
            [json!("server"), json!("http://10.1.2.3:1234"), json!("m"), json!(true)]
        );
        save_describe_key(&home, "").unwrap();
        assert_eq!(describe_key(&home).1, "");
        // which describer and its model (Settings, and Copy settings)
        set_describer(&home, "use", &json!("builtin")).unwrap();
        set_describer(&home, "model", &json!("  a model's name  ")).unwrap();
        assert_eq!(
            describer(&home).map(|(_, v)| v),
            [json!("builtin"), json!("http://10.1.2.3:1234"), json!("a model's name"), json!(true)]
        );
        assert_eq!(set_describer(&home, "use", &json!("the cloud")).unwrap_err(), "Unknown choice");
        assert!(
            set_describer(&home, "url", &json!("http://elsewhere.example")).is_err(),
            "where photos are sent is not set this way"
        );
        let _ = std::fs::remove_dir_all(home);
    }
}
