//! Short questions that only read (which devices there are, where one is reached), asked on one connection
//! kept for them: a new connection costs a tenth of a second for TLS and
//! the login, and a page asks every few seconds. The connection can only read, and is made again when it
//! has gone or went unused for `KEEP` seconds.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use tokio_postgres::Client;

use crate::{Result, Settings, connect_read_only};

/// seconds an unused connection is kept
const KEEP: Duration = Duration::from_secs(60);

struct Open {
    runtime: tokio::runtime::Runtime,
    client: Client,
    settings: Settings,
    used: Instant,
}

pub struct Kept {
    home: PathBuf,
    open: Mutex<Option<Open>>,
    /// when the server last couldn't be reached, and what was said then
    failed: Mutex<Option<(Instant, String)>>,
}

/// After the server couldn't be reached, questions are answered with that at once for this long: a page asks
/// every few seconds, each question would wait out a connection attempt, and they wait one behind another,
/// so with the server away a page's list took longer than the page waits (found with the database stopped,
/// 2026-11-23). What a page shows without the server comes from the device's own files meanwhile.
const AGAIN: Duration = Duration::from_secs(10);

/// One device of the shared database, as the Cameras page lists it.
pub struct Camera {
    pub id: String,
    pub name: String,
    /// it answers other devices
    pub share: bool,
    /// its camera sent a frame within the last 45 seconds
    pub streaming: bool,
    /// when it last did, as the pages read times ("2026-11-20T08:05:00.123456Z")
    pub streaming_at: Option<String>,
}

/// Where a sharing device is reached, and the certificate its peer service must show.
pub struct Peer {
    pub name: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub cert_sha256: String,
}

impl Kept {
    pub fn new(home: &Path) -> Kept {
        Kept { home: home.to_path_buf(), open: Mutex::new(None), failed: Mutex::new(None) }
    }

    /// Why the server couldn't be reached, while that was said less than `AGAIN` ago.
    fn away(&self) -> Option<String> {
        let failed = self.failed.lock().unwrap_or_else(|e| e.into_inner());
        failed.as_ref().filter(|(at, _)| at.elapsed() < AGAIN).map(|(_, why)| why.clone())
    }

    /// Ask on the kept connection; one question at a time. A connection that fails is let go of, and the
    /// question asked once more on a new one.
    pub fn ask<T>(&self, question: impl AsyncFn(&Client, &Settings) -> Result<T>) -> Result<T> {
        if let Some(why) = self.away() {
            return Err(why);
        }
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        // (a question that waited behind one that just failed doesn't try again itself)
        if let Some(why) = self.away() {
            return Err(why);
        }
        if open.as_ref().is_some_and(|o| o.used.elapsed() > KEEP || o.client.is_closed()) {
            *open = None;
        }
        let mut last = String::new();
        for _ in 0..2 {
            if open.is_none() {
                let settings =
                    Settings::load(&self.home)?.ok_or_else(|| "this device has no shared database".to_string())?;
                let runtime =
                    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
                let client = match runtime.block_on(connect_read_only(&settings)) {
                    Ok(client) => client,
                    Err(why) => {
                        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), why.clone()));
                        return Err(why);
                    }
                };
                *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = None;
                *open = Some(Open { runtime, client, settings, used: Instant::now() });
            }
            let Some(o) = open.as_mut() else { break };
            match o.runtime.block_on(question(&o.client, &o.settings)) {
                Ok(answer) => {
                    o.used = Instant::now();
                    return Ok(answer);
                }
                Err(e) if o.client.is_closed() => {
                    last = e; // the connection had gone: once more on a new one
                    *open = None;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
}

/// Every device of the shared database, by name, and whether its camera streams now.
pub async fn cameras(client: &Client) -> Result<Vec<Camera>> {
    let rows = client
        .query(
            &format!(
                "SELECT id::text, {name}, peer_share, COALESCE(streaming_at > now() - interval '45 seconds', false), \
                        to_char(streaming_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
                 FROM devices ORDER BY 2",
                name = crate::DEVICE_NAME
            ),
            &[],
        )
        .await
        .map_err(|e| format!("The devices couldn't be read from the shared database: {e}"))?;
    Ok(rows
        .iter()
        .map(|r| Camera {
            id: r.get(0),
            name: r.get(1),
            share: r.get(2),
            streaming: r.get(3),
            // (a time is written without a fraction when it has none)
            streaming_at: r.get::<_, Option<String>>(4).map(|t| t.replace(".000000Z", "Z")),
        })
        .collect())
}

/// Where a device that shares is reached, or None when it doesn't share (or isn't one).
pub async fn device_peer(client: &Client, device_id: &str) -> Result<Option<Peer>> {
    if uuid::Uuid::parse_str(device_id).is_err() {
        return Ok(None);
    }
    let row = client
        .query_opt(
            &format!(
                "SELECT {name}, peer_addresses, peer_port, peer_cert_sha256 FROM devices \
                 WHERE id = $1::text::uuid AND peer_share AND peer_port IS NOT NULL AND peer_cert_sha256 IS NOT NULL",
                name = crate::DEVICE_NAME
            ),
            &[&device_id],
        )
        .await
        .map_err(|e| format!("That device couldn't be looked up in the shared database: {e}"))?;
    Ok(row.as_ref().and_then(peer_of))
}

/// A row of name, addresses, port and certificate as where a device is reached. A row whose shape isn't that
/// (a list with an empty place in it) is no device to reach: it is left out, never the end of the program.
fn peer_of(r: &tokio_postgres::Row) -> Option<Peer> {
    let port = u16::try_from(r.try_get::<_, i32>(2).ok()?).ok()?;
    let addresses: Vec<String> =
        r.try_get::<_, Option<Vec<Option<String>>>>(1).ok()?.unwrap_or_default().into_iter().flatten().collect();
    Some(Peer {
        name: r.try_get::<_, Option<String>>(0).ok()?.unwrap_or_default(),
        addresses,
        port,
        cert_sha256: r.try_get(3).ok()?,
    })
}

/// The other sharing devices that have a photo (by its content hash), most recently seen first.
/// `me`: this device's id. `without`: the devices this one's owner removed on it
/// (their ids), which are asked for nothing.
pub async fn holders(client: &Client, photo: &str, me: &str, without: &[String]) -> Result<Vec<Peer>> {
    let rows = client
        .query(
            &format!(
                "SELECT {name}, d.peer_addresses, d.peer_port, d.peer_cert_sha256 \
                 FROM photo_paths p JOIN devices d ON d.id = p.device \
                 WHERE p.hash = $1 AND d.id::text <> $2 AND d.peer_share AND d.peer_port IS NOT NULL \
                   AND d.peer_cert_sha256 IS NOT NULL AND NOT (d.id::text = ANY($3)) \
                 ORDER BY d.last_seen DESC",
                name = crate::DEVICE_NAME_D
            ),
            &[&photo, &me, &without],
        )
        .await
        .map_err(|e| format!("Who has that photo couldn't be asked of the shared database: {e}"))?;
    Ok(rows.iter().filter_map(peer_of).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_the_server_away_a_question_is_answered_at_once_after_the_first() {
        let home = std::env::temp_dir().join(format!("face-db-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // no shared database on the device: said as that, and not remembered as a server that is away
        let kept = Kept::new(&home);
        assert!(kept.ask(async |_, _| Ok(1)).is_err_and(|e| e.contains("no shared database")));
        assert!(kept.away().is_none());
        // a server where nothing listens (this computer, a port nothing has)
        let settings = r#"{"host": "127.0.0.1", "port": 9, "user": "u", "password": "plain:YSBwYXNzd29yZA=="}"#;
        std::fs::write(home.join("database.json"), settings).unwrap();
        let first = kept.ask(async |_, _| Ok(1)).expect_err("nothing listens there");
        // the next question is told the same at once, without another attempt
        let began = Instant::now();
        assert_eq!(kept.ask(async |_, _| Ok(2)).expect_err("still away"), first);
        assert!(began.elapsed() < Duration::from_millis(200), "{:?}", began.elapsed());
        assert_eq!(kept.away().as_deref(), Some(first.as_str()));
        let _ = std::fs::remove_dir_all(home);
    }
}
