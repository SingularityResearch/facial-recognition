//! What the devices of one shared database ask each other directly, as far
//! as it comes from the database: the key their requests are signed with, a request's signature, and where a
//! device says it can be reached. The service itself is the server's (`face-server` `peer.rs`).

use ring::{hkdf, hmac};
use tokio_postgres::Client;

use crate::{Device, Result, Settings};

/// seconds a signed request stays valid
pub const CLOCK_SKEW: u64 = 120;

struct Bytes(usize);

impl hkdf::KeyType for Bytes {
    fn len(&self) -> usize {
        self.0
    }
}

/// The key every device with the shared database's password derives the same: HKDF of the password, salted
/// with the database's own id. Only the owner's devices can sign with it.
pub fn derive_key(password: &str, instance: &str) -> Option<[u8; 32]> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, instance.as_bytes()).extract(password.as_bytes());
    let mut key = [0u8; 32];
    let info = [&b"face-recognizer peer requests"[..]];
    prk.expand(&info, Bytes(32)).and_then(|okm| okm.fill(&mut key)).ok()?;
    Some(key)
}

/// The cluster key of the database this connection is to.
pub async fn cluster_key(client: &Client, s: &Settings) -> Result<[u8; 32]> {
    let instance: String = client
        .query_one("SELECT id::text FROM instance", &[])
        .await
        .map_err(|e| format!("The shared database's id couldn't be read: {e}"))?
        .get(0);
    derive_key(&s.password, &instance).ok_or_else(|| "The devices' key couldn't be made.".to_string())
}

fn message(method: &str, path: &str, when: &str, device: &str) -> Vec<u8> {
    format!("{method}\n{path}\n{when}\n{device}").into_bytes()
}

/// A request's signature, as text: HMAC-SHA256 of the method, the path, the time and the asking device.
pub fn sign(key: &[u8; 32], method: &str, path: &str, when: &str, device: &str) -> String {
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), &message(method, path, when, device));
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `signature` is that request's, compared without telling how much of it was right.
pub fn signed(key: &[u8; 32], method: &str, path: &str, when: &str, device: &str, signature: &str) -> bool {
    if signature.len() != 64 || !signature.is_ascii() {
        return false;
    }
    let tag: Option<Vec<u8>> = (0..32).map(|i| u8::from_str_radix(&signature[2 * i..2 * i + 2], 16).ok()).collect();
    let Some(tag) = tag else { return false };
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, key), &message(method, path, when, device), &tag).is_ok()
}

/// Whether this device shows its photos and camera to the owner's other devices: on with a shared database
/// unless switched off in Settings (`HOME/peer.json`) or by `FACE_PEER=off`.
pub fn sharing(home: &std::path::Path) -> bool {
    if std::env::var("FACE_PEER").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        return false;
    }
    let kept: Option<serde_json::Value> =
        std::fs::read(home.join("peer.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    kept.and_then(|k| k.get("share").cloned()) != Some(serde_json::Value::Bool(false))
}

/// The Settings switch: share with the other devices, or not.
pub fn set_sharing(home: &std::path::Path, on: bool) -> Result<()> {
    face_files::paths::atomic_write(&home.join("peer.json"), serde_json::json!({"share": on}).to_string().as_bytes())
        .map_err(|e| format!("can't write peer.json: {e}"))
}

/// Say where the other devices reach this one: its addresses, its peer service's port and the SHA-256 of
/// that service's certificate, which an asking device checks.
pub async fn register(
    client: &Client,
    device: &Device,
    addresses: &[String],
    port: u16,
    cert_sha256: &str,
) -> Result<()> {
    client
        .execute(
            "UPDATE devices SET peer_addresses = $1, peer_port = $2, peer_cert_sha256 = $3, peer_share = true \
             WHERE id = $4::text::uuid",
            &[&addresses, &(port as i32), &cert_sha256, &device.id],
        )
        .await
        .map_err(|e| format!("The shared database couldn't be told where this device is: {e}"))?;
    Ok(())
}

/// Say that this device answers no other device.
pub async fn unshare(client: &Client, device: &Device) -> Result<()> {
    client
        .execute("UPDATE devices SET peer_share = false WHERE id = $1::text::uuid", &[&device.id])
        .await
        .map_err(|e| format!("The shared database couldn't be told this device doesn't share: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // fixed values for these made-up inputs: every device, of whatever version, must come to the same ones
    const PASSWORD: &str = "a pass word / with + signs";
    const INSTANCE: &str = "0b0e7a52-6f0c-4b6e-9d53-1c2f3a4b5c6d";
    const KEY: &str = "38ad2d3f69e8155636131d65a46c9bf294474f6aa5ebb5c9901adbbbbfeb4337";
    const DEVICE: &str = "11111111-2222-4333-8444-555555555555";
    const SIGNATURE: &str = "2e6e0b253cc791f623672b75de530ac6b3f6e89c2f805b83b1449321fff29152";

    #[test]
    fn sharing_is_on_until_switched_off() {
        if std::env::var("FACE_PEER").is_ok() {
            return; // (the environment decides: nothing to tell from the file)
        }
        let home = std::env::temp_dir().join(format!("face-db-peer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(sharing(&home), "on with a shared database unless switched off");
        set_sharing(&home, false).unwrap();
        assert!(!sharing(&home));
        assert_eq!(std::fs::read_to_string(home.join("peer.json")).unwrap(), r#"{"share":false}"#);
        std::fs::write(home.join("peer.json"), r#"{"share": true}"#).unwrap();
        assert!(sharing(&home));
        std::fs::write(home.join("peer.json"), "not a file of settings").unwrap();
        assert!(sharing(&home));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_cluster_key_is_the_one_every_device_derives() {
        let key = derive_key(PASSWORD, INSTANCE).unwrap();
        assert_eq!(key.iter().map(|b| format!("{b:02x}")).collect::<String>(), KEY);
        assert_ne!(derive_key(PASSWORD, "another database").unwrap(), key);
        assert_ne!(derive_key("another password", INSTANCE).unwrap(), key);
    }

    #[test]
    fn a_request_is_signed_as_every_device_signs_it() {
        let key = derive_key(PASSWORD, INSTANCE).unwrap();
        assert_eq!(sign(&key, "GET", "/stream", "1795000000", DEVICE), SIGNATURE);
        assert!(signed(&key, "GET", "/stream", "1795000000", DEVICE, SIGNATURE));
        for (method, path, when, device) in [
            ("POST", "/stream", "1795000000", DEVICE),
            ("GET", "/readout", "1795000000", DEVICE),
            ("GET", "/stream", "1795000001", DEVICE),
            ("GET", "/stream", "1795000000", "another device"),
        ] {
            assert!(!signed(&key, method, path, when, device, SIGNATURE), "{method} {path} {when} {device}");
        }
        let other = derive_key("another password", INSTANCE).unwrap();
        assert!(!signed(&other, "GET", "/stream", "1795000000", DEVICE, SIGNATURE), "another cluster's key");
        for bad in ["", "2e6e", &SIGNATURE.replace('2', "g"), &format!("{SIGNATURE}00"), &"é".repeat(32)] {
            assert!(!signed(&key, "GET", "/stream", "1795000000", DEVICE, bad), "{bad}");
        }
    }
}
