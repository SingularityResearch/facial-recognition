//! The member list every device keeps for itself (`HOME/members.json`): which devices belong to the cluster,
//! where each was last reached and the certificate its peer service must show. The owner's rule (2026-11-23):
//! "the app should not be dependent on a centralized master". The devices find and check each other from
//! this list, so what they say to each other directly (the event bus, the cameras) goes on when the shared
//! database can't be reached. The database is one place the list is filled from, whenever it answers; it is
//! not asked when a device connects to another.
//!
//! The list holds no secret: ids, names as text, addresses, a port, a certificate's SHA-256. What makes a
//! device a member is still the cluster's key, which every device derives from the database's password and
//! the database's own id: the id is kept here beside the list, so the key can be made without the server.
//! A name is only ever shown; a device is reached by its addresses and known by its certificate.

use std::path::Path;

use serde_json::{Value, json};
use tokio_postgres::Client;

use crate::{Result, Settings};

const FILE: &str = "members.json";
/// more devices than anybody has: a list beyond it is a damaged file
const MOST: usize = 256;
const MOST_ADDRESSES: usize = 16;

/// One device of the cluster, as this device last learned of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: String,
    /// what the device is called, to be shown (the owner's name for it, else its own)
    pub name: String,
    pub addresses: Vec<String>,
    pub port: u16,
    /// the SHA-256 of the certificate its peer service shows, and that it shows when it asks
    pub cert_sha256: String,
    /// it answers other devices
    pub share: bool,
}

fn an_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok() && id.len() == 36 && !id.bytes().any(|b| b.is_ascii_uppercase())
}

fn a_sha(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A member from what a file or another program says of it, or None when it isn't one: every field checked,
/// a name cut to a line of text, an address only when it is one.
fn checked(v: &Value) -> Option<Member> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
    let (id, cert) = (text("id"), text("cert"));
    if !an_id(id) || !(cert.is_empty() || a_sha(cert)) {
        return None;
    }
    let name: String = text("name").chars().filter(|c| !c.is_control()).take(100).collect();
    let addresses = (v.get("addresses").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default().iter())
        .filter_map(Value::as_str)
        .filter(|a| a.parse::<std::net::IpAddr>().is_ok())
        .take(MOST_ADDRESSES)
        .map(str::to_string)
        .collect();
    let port = v.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(0);
    Some(Member {
        id: id.to_string(),
        name,
        addresses,
        port,
        cert_sha256: cert.to_string(),
        share: v.get("share").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn as_json(m: &Member) -> Value {
    json!({"id": m.id, "name": m.name, "addresses": m.addresses, "port": m.port, "cert": m.cert_sha256, "share": m.share})
}

fn file(home: &Path) -> Value {
    std::fs::read(home.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

/// The members this device knows of.
pub fn kept(home: &Path) -> Vec<Member> {
    let v = file(home);
    let listed = v.get("members").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    listed.iter().filter_map(checked).take(MOST).collect()
}

/// One member by its id.
pub fn member(home: &Path, id: &str) -> Option<Member> {
    kept(home).into_iter().find(|m| m.id == id)
}

/// The shared database's own id, as it was last read: what the cluster's key is salted with.
pub fn instance(home: &Path) -> Option<String> {
    let v = file(home);
    v.get("instance").and_then(Value::as_str).filter(|i| an_id(i)).map(str::to_string)
}

/// Keep what was learned of the members. From the shared database (`instance_id` is its id) it is the whole
/// list: a member the database no longer names is a member no longer, so a list can't be filled for good with
/// devices that were only ever rows. From elsewhere (no id) each member named replaces what was kept of it
/// and the others stay.
pub fn keep(home: &Path, instance_id: Option<&str>, learned: &[Member]) -> Result<()> {
    let known = kept(home);
    let before = (known.clone(), instance(home));
    let whole = instance_id.is_some_and(an_id);
    let mut all = if whole { Vec::new() } else { known };
    for m in learned.iter().filter(|m| an_id(&m.id)).take(MOST) {
        match all.iter().position(|a| a.id == m.id) {
            Some(at) => all[at] = m.clone(),
            None if all.len() < MOST => all.push(m.clone()),
            None => {}
        }
    }
    let instance_id = instance_id.filter(|i| an_id(i)).map(str::to_string).or_else(|| before.1.clone());
    if (all.clone(), instance_id.clone()) == before {
        return Ok(()); // nothing new: the file is left as it is
    }
    std::fs::create_dir_all(home).map_err(|e| format!("can't keep the member list: {e}"))?;
    let out = json!({"instance": instance_id, "members": all.iter().map(as_json).collect::<Vec<Value>>()});
    face_files::paths::atomic_write(&home.join(FILE), face_files::filejson::dumps(&out).as_bytes())
        .map_err(|e| format!("can't keep the member list: {e}"))
}

/// The cluster's key from what this device keeps: the database's password (protected, in its settings) and
/// the database's id as last read. None before the device has ever reached the database.
pub fn kept_key(home: &Path) -> Option<[u8; 32]> {
    let settings = Settings::load(home).ok()??;
    crate::peer::derive_key(&settings.password, &instance(home)?)
}

/// The members as the shared database has them now, and the database's own id.
pub async fn read(client: &Client) -> Result<(String, Vec<Member>)> {
    let wrong = |e| format!("The devices couldn't be read from the shared database: {e}");
    let instance: String = client.query_one("SELECT id::text FROM instance", &[]).await.map_err(wrong)?.get(0);
    let rows = client
        .query(
            &format!(
                "SELECT id::text, {name}, peer_addresses, peer_port, peer_cert_sha256, peer_share FROM devices \
                 ORDER BY first_seen, id LIMIT {most}",
                name = crate::DEVICE_NAME,
                most = MOST + 1
            ),
            &[],
        )
        .await
        .map_err(wrong)?;
    // Whoever has the database's password can write any row, and so can a faulty version: a row is read
    // without trusting its shape (a list with an empty place in it, a number where a text belongs), and one
    // that can't be read is left out. Never `get` here: it ends the program on a row it can't read, and
    // this is read by every device every minute.
    let members = rows
        .iter()
        .filter_map(|r| {
            let addresses: Vec<String> = r
                .try_get::<_, Option<Vec<Option<String>>>>(2)
                .ok()?
                .unwrap_or_default()
                .into_iter()
                .flatten()
                .collect();
            checked(&json!({
                "id": r.try_get::<_, String>(0).ok()?,
                "name": r.try_get::<_, Option<String>>(1).ok()?.unwrap_or_default(),
                "addresses": addresses,
                "port": r.try_get::<_, Option<i32>>(3).ok()?.unwrap_or(0),
                "cert": r.try_get::<_, Option<String>>(4).ok()?.unwrap_or_default(),
                "share": r.try_get::<_, Option<bool>>(5).ok()?.unwrap_or(false),
            }))
        })
        .collect();
    Ok((instance, members))
}

/// Read the members from the shared database and keep them on this device; the cluster's key with it.
pub async fn refresh(client: &Client, settings: &Settings, home: &Path) -> Result<[u8; 32]> {
    let (instance, members) = read(client).await?;
    keep(home, Some(&instance), &members)?;
    crate::peer::derive_key(&settings.password, &instance)
        .ok_or_else(|| "The devices' key couldn't be made.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "11111111-2222-4333-8444-555555555555";
    const B: &str = "99999999-2222-4333-8444-555555555555";
    const DB: &str = "0b0e7a52-6f0c-4b6e-9d53-1c2f3a4b5c6d";

    fn one(id: &str, name: &str, port: u16) -> Member {
        Member {
            id: id.to_string(),
            name: name.to_string(),
            addresses: vec!["192.168.1.20".to_string()],
            port,
            cert_sha256: "ab".repeat(32),
            share: true,
        }
    }

    #[test]
    fn the_list_is_kept_and_what_is_learned_later_replaces_what_was_known() {
        let home = std::env::temp_dir().join(format!("face-db-members-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert!(kept(&home).is_empty() && instance(&home).is_none() && member(&home, A).is_none());
        keep(&home, Some(DB), &[one(A, "a computer", 8767), one(B, "a phone", 40001)]).unwrap();
        assert_eq!(kept(&home), vec![one(A, "a computer", 8767), one(B, "a phone", 40001)]);
        assert_eq!(instance(&home).as_deref(), Some(DB));
        // one member learned again, from somewhere that doesn't know the database's id: the other stays, and so does the id
        keep(&home, None, &[one(B, "a phone, renamed", 40002)]).unwrap();
        assert_eq!(member(&home, B), Some(one(B, "a phone, renamed", 40002)));
        assert_eq!(member(&home, A), Some(one(A, "a computer", 8767)));
        assert_eq!(instance(&home).as_deref(), Some(DB));
        // what the database says is the whole list: a member it no longer names is one no longer
        keep(&home, Some(DB), &[one(A, "a computer", 8767)]).unwrap();
        assert_eq!(kept(&home), vec![one(A, "a computer", 8767)]);
        // nothing new: the file isn't written again
        let written = std::fs::metadata(home.join(FILE)).unwrap().modified().unwrap();
        keep(&home, Some(DB), &[one(A, "a computer", 8767)]).unwrap();
        assert_eq!(std::fs::metadata(home.join(FILE)).unwrap().modified().unwrap(), written);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn what_a_file_says_is_checked_before_it_is_believed() {
        let home = std::env::temp_dir().join(format!("face-db-members-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let long = "n".repeat(500);
        let text = json!({"instance": "not an id", "members": [
            {"id": A, "name": format!("a\nname{long}"), "addresses": ["192.168.1.20", "a.host.name", 7, "::1"], "port": 70000,
             "cert": "ab".repeat(32), "share": "yes"},
            {"id": "../../etc/passwd", "name": "x", "cert": "ab".repeat(32)},
            {"id": B, "name": "x", "cert": "not a certificate's number"},
            "a member", 3
        ]});
        std::fs::write(home.join(FILE), text.to_string()).unwrap();
        let all = kept(&home);
        assert_eq!(all.len(), 1, "{all:?}");
        let m = &all[0];
        assert_eq!((m.id.as_str(), m.port, m.share), (A, 0, false));
        assert!(m.name.len() == 100 && !m.name.contains('\n'), "{}", m.name);
        assert_eq!(m.addresses, ["192.168.1.20", "::1"], "a name is never an address");
        assert!(instance(&home).is_none());
        std::fs::write(home.join(FILE), "not a list").unwrap();
        assert!(kept(&home).is_empty());
        let _ = std::fs::remove_dir_all(home);
    }
}
