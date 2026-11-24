//! The shared database (PostgreSQL with pgvector on a server the devices share).
//!
//! It connects with TLS checked against the server's own certificate authority, and with the
//! settings and protected password of `HOME/database.json`. The reading side (`connect_read_only`, `people`)
//! has a connection that is itself set read-only, so nothing there can change what the devices share. The
//! writing side so far: the schema's migrations, and a device's own row with its heartbeat. `join` is how a
//! new device gets the login from one that has it, and `setup` keeps it.

pub mod join;
pub mod kept;
pub mod location;
pub mod members;
pub mod peer;

use std::{path::Path, sync::Arc, time::Duration};

use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
};
use tokio_postgres::{Client, config::ChannelBinding};

/// What went wrong, in words a log line or a page can show. Never holds the password.
pub type Result<T> = std::result::Result<T, String>;

/// A new id nobody else has (a random UUID, as text).
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// seconds: an unreachable server must not hold up the programs
const CONNECT_TIMEOUT: u64 = 5;
/// faces below this quality are not used to learn a person (a measured value)
const Q_ENROLL: f64 = 0.30;
/// the name the labeling tool gives an Ignored face
const IGNORE: &str = "#ignore";

/// This device's database settings (`HOME/database.json`).
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub dbname: String,
    pub user: String,
    password: String,
    /// the server's own certificate authority (PEM), or empty for settings saved without one
    ca: String,
    pub schema: String,
}

impl Settings {
    /// None when this device has no shared database.
    pub fn load(home: &Path) -> Result<Option<Settings>> {
        let bytes = match std::fs::read(home.join("database.json")) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("can't read database.json: {e}")),
        };
        let c: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("can't read database.json: {e}"))?;
        let text = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if text("host").is_empty() {
            return Ok(None);
        }
        let schema = if text("schema").is_empty() { "facerec".to_string() } else { text("schema") };
        if !schema_name(&schema) {
            return Err("bad schema name".to_string());
        }
        Ok(Some(Settings {
            host: text("host"),
            port: c.get("port").and_then(|p| p.as_u64()).unwrap_or(5432) as u16,
            dbname: if text("dbname").is_empty() { "postgres".to_string() } else { text("dbname") },
            user: text("user"),
            password: unprotect(&text("password"))?,
            ca: text("ca"),
            schema,
        }))
    }

    /// The login, to hand to a device that joins (`join::Offer::serve`).
    /// A short one-way fingerprint of the password, keyed with a database's own id (so it means nothing
    /// elsewhere): what a device shows of the password it has, XXXX-XXXX-XXXX.
    pub fn password_fingerprint(&self, instance: &str) -> String {
        use ring::hmac;
        let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, instance.as_bytes()), self.password.as_bytes());
        let h: String = tag.as_ref().iter().take(6).map(|b| format!("{b:02X}")).collect();
        format!("{}-{}-{}", &h[..4], &h[4..8], &h[8..])
    }

    pub fn login(&self) -> join::Login {
        join::Login {
            host: self.host.clone(),
            port: self.port,
            dbname: self.dbname.clone(),
            user: self.user.clone(),
            password: self.password.clone(),
            schema: self.schema.clone(),
        }
    }
}

/// The settings file's content, when a shared database is set up in it.
fn kept(home: &Path) -> Option<serde_json::Value> {
    let c: serde_json::Value = serde_json::from_slice(&std::fs::read(home.join("database.json")).ok()?).ok()?;
    c.get("host").and_then(|h| h.as_str()).is_some_and(|h| !h.is_empty()).then_some(c)
}

/// This device's database settings for the Settings tab: {host, port, dbname, user, schema, ca: whether the
/// server's certificate authority is kept}. None when there is no shared database. Never the password.
pub fn config(home: &Path) -> Option<serde_json::Value> {
    let c = kept(home)?;
    let has_ca = c.get("ca").and_then(|v| v.as_str()).is_some_and(|ca| !ca.is_empty());
    Some(serde_json::json!({"host": c["host"], "port": c["port"], "dbname": c["dbname"], "user": c["user"],
                            "schema": c["schema"], "ca": has_ca}))
}

/// {host, port, dbname, user, password} as saved on this device, for the Settings tab's own page to show
/// masked (and nothing else: it holds the password). None when there is no shared database.
pub fn saved_login(home: &Path) -> Option<serde_json::Value> {
    let c = kept(home)?;
    let text = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let password = c.get("password").and_then(|p| p.as_str()).and_then(|p| face_files::protect::unprotect(p).ok());
    let port = c.get("port").and_then(|p| p.as_u64()).filter(|p| *p > 0).unwrap_or(5432);
    Some(
        serde_json::json!({"host": text("host"), "port": port.to_string(), "dbname": text("dbname"), "user": text("user"),
                            "password": password.unwrap_or_default()}),
    )
}

/// No shared database from now on: back to this device's files only.
pub fn forget(home: &Path) -> Result<()> {
    match std::fs::remove_file(home.join("database.json")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("can't remove database.json: {e}")),
        _ => Ok(()),
    }
}

/// A schema's name may have only lower-case letters, digits and `_`: it is put into a statement as it is.
pub fn schema_name(s: &str) -> bool {
    let mut chars = s.chars();
    s.len() <= 63
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The saved database password, read back.
fn unprotect(stored: &str) -> Result<String> {
    face_files::protect::unprotect(stored)
        .map_err(|_| "The saved database password can't be read on this device.".to_string())
}

/// "verify-ca": the server must show a certificate from this authority, under whichever of its addresses a
/// device reaches it (the authority is this server's own, so the name is not
/// what is checked).
#[derive(Debug)]
struct FromThisAuthority(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for FromThisAuthority {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        use rustls::{CertificateError::*, Error::InvalidCertificate};
        match self.0.verify_server_cert(end_entity, intermediates, server_name, ocsp, now) {
            Err(InvalidCertificate(NotValidForName | NotValidForNameContext { .. })) => {
                Ok(ServerCertVerified::assertion())
            }
            other => other,
        }
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

/// Settings saved without the server's authority: any certificate is taken, and the login itself has to say
/// whose server this is (channel binding: only a server that knows the password and is the other end of
/// this very connection passes).
#[derive(Debug)]
struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// A connection to the server as the settings say, nothing set on it yet.
async fn open(s: &Settings) -> Result<Client> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS didn't start: {e}"))?;
    let mut config = tokio_postgres::Config::new();
    config
        .host(&s.host)
        .port(s.port)
        .dbname(&s.dbname)
        .user(&s.user)
        .password(&s.password)
        .application_name("Face Recognizer")
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
        .ssl_mode(tokio_postgres::config::SslMode::Require);
    let tls = if s.ca.is_empty() {
        config.channel_binding(ChannelBinding::Require);
        builder.dangerous().with_custom_certificate_verifier(Arc::new(AnyCertificate(provider))).with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(s.ca.as_bytes()) {
            let cert = cert.map_err(|_| "The saved certificate authority can't be read.".to_string())?;
            roots.add(cert).map_err(|_| "The saved certificate authority can't be used.".to_string())?;
        }
        let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .map_err(|_| "The saved certificate authority can't be used.".to_string())?;
        builder.dangerous().with_custom_certificate_verifier(Arc::new(FromThisAuthority(inner))).with_no_client_auth()
    };
    let (client, connection) =
        config.connect(tokio_postgres_rustls::MakeRustlsConnect::new(tls)).await.map_err(|e| {
            // (the server answered and said no to this login: not the same as a server that can't be reached)
            use tokio_postgres::error::SqlState;
            let refused = [SqlState::INVALID_PASSWORD, SqlState::INVALID_AUTHORIZATION_SPECIFICATION];
            if e.code().is_some_and(|code| refused.contains(code)) {
                format!("{LOGIN_REFUSED}: {e}")
            } else {
                format!("The shared database can't be reached: {e}")
            }
        })?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

const LOGIN_REFUSED: &str = "The shared database refused this device's login";

/// Whether an error of connecting says that the server answered and refused this device's login (its
/// password was changed, or its role taken away), as opposed to a server that can't be reached. A device
/// that is refused no longer belongs to the cluster on the strength of what it kept from before.
pub fn login_refused(error: &str) -> bool {
    error.starts_with(LOGIN_REFUSED)
}

/// A connection that can only read, with the schema's tables in view and times in UTC.
pub async fn connect_read_only(s: &Settings) -> Result<Client> {
    let client = open(s).await?;
    client
        .batch_execute(&format!(
            "SET default_transaction_read_only = on; SET search_path = {}, public; SET TIME ZONE 'UTC'",
            s.schema
        ))
        .await
        .map_err(|e| format!("The shared database refused the session's settings: {e}"))?;
    Ok(client)
}

/// The people everyone is recognized from, as the labeling engine's `gallery` and `build_people` decide it:
/// every named, usable face's print with its person's name. Usable: it has a print, and its quality is not
/// below `Q_ENROLL` (a face never scored counts as usable). A webcam snapshot's face or a possible face
/// that was named automatically waits for the owner's confirmation first. (names, prints one after another,
/// numbers a print).
pub async fn people(client: &Client, rec_model: &str) -> Result<(Vec<String>, Vec<f32>, usize)> {
    let rows = client
        .query(
            "SELECT l.n, f.print::real[] FROM faces f JOIN labels l ON l.fid = f.fid \
             WHERE l.n IS NOT NULL AND l.n <> '' AND l.n <> $1 AND f.model = $2 AND f.print IS NOT NULL \
               AND (f.quality IS NULL OR f.quality = 'NaN' OR NOT f.quality::float8 < $3) \
               AND NOT (COALESCE(l.by, '') = 'auto' AND (f.possible IS NOT NULL OR f.file LIKE 'webcam-%')) \
             ORDER BY l.n, f.fid",
            &[&IGNORE, &rec_model, &Q_ENROLL],
        )
        .await
        .map_err(|e| format!("The people couldn't be read from the shared database: {e}"))?;
    let (mut names, mut emb, mut dim) = (Vec::new(), Vec::new(), 0usize);
    for row in rows {
        let name: String = row.get(0);
        let print: Vec<f32> = row.get(1);
        if print.iter().all(|v| *v == 0.0) {
            continue; // a face whose print was deleted (Ignored once, or a forgotten person's)
        }
        if dim == 0 {
            dim = print.len();
        }
        if print.len() != dim {
            continue; // a row this device can't use stops nothing
        }
        names.push(name);
        emb.extend_from_slice(&print);
    }
    Ok((names, emb, dim))
}

/// The same, from a place that has no async runtime of its own: connects, reads, lets go.
pub fn people_now(home: &Path, rec_model: &str) -> Result<Option<(Vec<String>, Vec<f32>, usize)>> {
    let Some(settings) = Settings::load(home)? else { return Ok(None) };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = connect_read_only(&settings).await?;
        people(&client, rec_model).await.map(Some)
    })
}

// ---- writing: the schema, and this device's own row ---------------------------------------------------------

/// The schema's migrations, `migrations/NNN_*.sql`. A test
/// holds this list to the folder.
pub const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../../../migrations/001_schema.sql")),
    (2, include_str!("../../../migrations/002_sync.sql")),
    (3, include_str!("../../../migrations/003_server_ca.sql")),
    (4, include_str!("../../../migrations/004_forgotten.sql")),
    (5, include_str!("../../../migrations/005_picture_sha.sql")),
    (6, include_str!("../../../migrations/006_passphrase_versions.sql")),
    (7, include_str!("../../../migrations/007_peers.sql")),
    (8, include_str!("../../../migrations/008_cameras.sql")),
    (9, include_str!("../../../migrations/009_change_order.sql")),
    (10, include_str!("../../../migrations/010_expression_log.sql")),
    (11, include_str!("../../../migrations/011_key_trust.sql")),
    (12, include_str!("../../../migrations/012_music_log.sql")),
    (13, include_str!("../../../migrations/013_own_recognizer.sql")),
    (14, include_str!("../../../migrations/014_device_alias.sql")),
    (15, include_str!("../../../migrations/015_location_log.sql")),
];

/// What a device is called, as every statement reads it that shows a device's name: the name the owner gave
/// it (`devices.alias`, Settings: "Name a device"), else what the device calls itself. For a statement where
/// the devices table has no other name.
pub const DEVICE_NAME: &str = "coalesce(nullif(btrim(alias), ''), name)";
/// The same where the devices table is called `d`.
pub const DEVICE_NAME_D: &str = "coalesce(nullif(btrim(d.alias), ''), d.name)";

/// Give a device the name the owner calls it by, or take that name away (an empty one). The name is kept
/// as text a page can show: without control characters, `join::NAME_MAX` characters at most. What the
/// device is called from now on (its own name again when the alias is taken away), or that there is no such
/// device.
///
/// A name is one device's alone, as an email address is one account's (the owner's rule, 2026-11-23): a name
/// that another device goes by already, whatever its capitals, is refused. Two aliases can't be the same by
/// the table's own index; that an alias is not another device's own name is seen to here.
pub async fn set_alias(client: &Client, device_id: &str, alias: &str) -> Result<String> {
    let no_such = || "That is not a device of the shared database.".to_string();
    if uuid::Uuid::parse_str(device_id).is_err() {
        return Err(no_such());
    }
    let alias = join::clean_name(alias);
    let taken = || format!("Another device is called {alias} already: a name is one device's alone.");
    if !alias.is_empty() {
        let ask =
            format!("SELECT 1 FROM devices WHERE id <> $2::text::uuid AND lower({DEVICE_NAME}) = lower($1) LIMIT 1");
        let other = client.query_opt(&ask, &[&alias, &device_id]).await;
        if other.map_err(|e| format!("The devices' names couldn't be read from the shared database: {e}"))?.is_some() {
            return Err(taken());
        }
    }
    let said = format!("UPDATE devices SET alias = $1 WHERE id = $2::text::uuid RETURNING {DEVICE_NAME}");
    match client.query_opt(&said, &[&alias, &device_id]).await {
        Ok(row) => row.map(|r| r.get(0)).ok_or_else(no_such),
        // (two devices named at the same moment: the index let one of them in)
        Err(e) if e.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) => Err(taken()),
        Err(e) => Err(format!("The device's name couldn't be kept in the shared database: {e}")),
    }
}

/// This device as the others know it (`HOME/device.json`: made once, never changed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
}

impl Device {
    /// This device's id and name, made the first time they are asked for. `name` is what a new device is
    /// called: the computer's name without its domain, or what a phone's shell says it is called.
    pub fn load(home: &Path, name: &str) -> Result<Device> {
        let file = home.join("device.json");
        if let Ok(bytes) = std::fs::read(&file) {
            if let Ok(d) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let (id, name) = (d.get("id").and_then(|v| v.as_str()), d.get("name").and_then(|v| v.as_str()));
                if let (Some(id), Some(name)) = (id, name) {
                    if uuid::Uuid::parse_str(id).is_ok() {
                        return Ok(Device { id: id.to_string(), name: name.to_string() });
                    }
                }
            }
        }
        let name: String = name.trim().chars().filter(|c| !c.is_control()).take(100).collect();
        let d = Device {
            id: uuid::Uuid::new_v4().to_string(),
            name: if name.is_empty() { "this device".to_string() } else { name },
        };
        std::fs::create_dir_all(home).map_err(|e| format!("can't make {}: {e}", home.display()))?;
        let text = serde_json::json!({"id": d.id, "name": d.name}).to_string();
        let tmp = home.join("device.json.tmp");
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, &file))
            .map_err(|e| format!("can't write device.json: {e}"))?;
        Ok(d)
    }
}

/// A connection that may write, for this device: the schema's tables in view, times in UTC, and the device
/// named for the change log (every change says who made it). `schema` is the settings' own unless a test
/// names another.
pub async fn connect(s: &Settings, schema: &str, device: &Device) -> Result<Client> {
    if !schema_name(schema) {
        return Err("bad schema name".to_string());
    }
    let client = open(s).await?;
    client
        .batch_execute(&format!("SET search_path = {schema}, public; SET TIME ZONE 'UTC'"))
        .await
        .map_err(|e| format!("The shared database refused the session's settings: {e}"))?;
    client
        .execute("SELECT set_config('facerec.device', $1, false)", &[&device.id])
        .await
        .map_err(|e| format!("The shared database refused the session's settings: {e}"))?;
    Ok(client)
}

/// The schema's version: the newest migration applied, 0 for an empty schema.
pub async fn version(client: &Client) -> Result<i32> {
    let e = |e: tokio_postgres::Error| format!("The shared database's version couldn't be read: {e}");
    let there: bool =
        client.query_one("SELECT to_regclass('schema_version') IS NOT NULL", &[]).await.map_err(e)?.get(0);
    if !there {
        return Ok(0);
    }
    let v: Option<i32> = client.query_one("SELECT max(version) FROM schema_version", &[]).await.map_err(e)?.get(0);
    Ok(v.unwrap_or(0))
}

/// Bring the schema up to the newest migration, all that are missing in one transaction, under a lock so
/// that two devices starting together don't both run one. The numbers applied.
pub async fn migrate(client: &mut Client) -> Result<Vec<i32>> {
    let e = |e: tokio_postgres::Error| format!("The shared database couldn't be brought up to date: {e}");
    let tx = client.transaction().await.map_err(e)?;
    tx.batch_execute("SELECT pg_advisory_xact_lock(hashtext(current_schema()))").await.map_err(e)?;
    let there: bool = tx.query_one("SELECT to_regclass('schema_version') IS NOT NULL", &[]).await.map_err(e)?.get(0);
    let have: i32 = if there {
        tx.query_one("SELECT max(version) FROM schema_version", &[])
            .await
            .map_err(e)?
            .get::<_, Option<i32>>(0)
            .unwrap_or(0)
    } else {
        0
    };
    let mut done = Vec::new();
    for (n, sql) in MIGRATIONS {
        if *n > have {
            tx.batch_execute(sql).await.map_err(|e| format!("Migration {n} failed: {e}"))?;
            done.push(*n);
        }
    }
    tx.commit().await.map_err(e)?;
    Ok(done)
}

/// Note this device: its name, system and app version, and that it was seen now.
pub async fn register(client: &Client, device: &Device, os: &str, app_version: &str) -> Result<()> {
    uuid::Uuid::parse_str(&device.id).map_err(|_| "this device's id is not an id".to_string())?;
    client
        .execute(
            "INSERT INTO devices (id, name, os, app_version) VALUES ($1::text::uuid, $2, $3, $4) \
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, os = excluded.os, \
             app_version = excluded.app_version, last_seen = now()",
            &[&device.id, &device.name, &os, &app_version],
        )
        .await
        .map_err(|e| format!("This device couldn't be noted in the shared database: {e}"))?;
    // (a nicety that must not keep a device from being noted: a schema that has no aliases yet has none)
    let _ = tell_apart(client, device).await;
    Ok(())
}

/// `set_alias` for a page's request, on a connection made for it as the device `me` and ended with it: the one
/// a program keeps for a page's short questions only reads.
pub fn name_device(home: &Path, me: &Device, device_id: &str, alias: &str) -> Result<String> {
    let settings = Settings::load(home)?.ok_or("This device has no shared database.")?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = connect(&settings, &settings.schema, me).await?;
        set_alias(&client, device_id, alias).await
    })
}

/// What a device is called: (the name every device shows it under, the name it calls itself, whether that
/// first name is only the number the app gave it to tell it apart, `tell_apart`). None for no such device.
pub async fn device_names(client: &Client, device_id: &str) -> Result<Option<(String, String, bool)>> {
    if uuid::Uuid::parse_str(device_id).is_err() {
        return Ok(None);
    }
    let ask = format!("SELECT {DEVICE_NAME}, name, btrim(alias) FROM devices WHERE id = $1::text::uuid");
    let row = client.query_opt(&ask, &[&device_id]).await;
    let row = row.map_err(|e| format!("The device's name couldn't be read from the shared database: {e}"))?;
    Ok(row.map(|r| {
        let (shown, own, alias): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        let numbered = numbered(&own, &alias);
        (join::clean_name(&shown), join::clean_name(&own), numbered)
    }))
}

/// Whether `alias` is `own` with a number after it: what the app gives a device to tell it apart.
fn numbered(own: &str, alias: &str) -> bool {
    let (own, alias) = (own.trim().to_lowercase(), alias.trim().to_lowercase());
    // (a long name is cut to make room for the number: its beginning is what is compared)
    let kept: String = own.chars().take(join::NAME_MAX - 4).collect();
    alias
        .strip_prefix(kept.trim())
        .and_then(|rest| rest.strip_prefix(' '))
        .is_some_and(|n| !n.is_empty() && n.len() <= 2 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// A device that calls itself what another device goes by already is told apart by a number after its name:
/// the second "iPhone" is "iPhone 2", for every device, until the owner names it (Settings: Name a device).
/// The owner left that to the app (2026-11-23: "pseudo device names may be used"). It is kept as the device's
/// alias, like a name the owner gave, so a name stays one device's alone. The device that was there first
/// keeps its plain name, and so does one whose name the owner gave to another device only later than never:
/// a device yields to an alias, and to an older device that calls itself the same.
async fn tell_apart(client: &Client, device: &Device) -> Result<()> {
    let said = |e: tokio_postgres::Error| e.to_string();
    let ask = format!(
        "SELECT EXISTS (SELECT 1 FROM devices me JOIN devices o ON o.id <> me.id \
                        WHERE me.id = $1::text::uuid AND btrim(me.alias) = '' \
                          AND lower(coalesce(nullif(btrim(o.alias), ''), o.name)) = lower(btrim(me.name)) \
                          AND (btrim(o.alias) <> '' OR (o.first_seen, o.id) < (me.first_seen, me.id)))"
    );
    if !client.query_one(&ask, &[&device.id]).await.map_err(said)?.get::<_, bool>(0) {
        return Ok(());
    }
    // (room for the number within what a name may be long)
    let own: String = join::clean_name(&device.name).chars().take(join::NAME_MAX - 4).collect();
    let take = format!(
        "UPDATE devices SET alias = $1 WHERE id = $2::text::uuid AND btrim(alias) = '' \
           AND NOT EXISTS (SELECT 1 FROM devices o WHERE o.id <> $2::text::uuid AND lower({DEVICE_NAME}) = lower($1))"
    )
    .replace(DEVICE_NAME, "coalesce(nullif(btrim(o.alias), ''), o.name)");
    for n in 2..100 {
        let name = format!("{} {n}", own.trim());
        match client.execute(&take, &[&name, &device.id]).await {
            Ok(1) => return Ok(()),
            // taken by another device (seen, or by the index when two took it at once): the next number
            Ok(_) => {}
            Err(e) if e.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) => {}
            Err(e) => return Err(said(e)),
        }
        // (named meanwhile, by the owner: nothing more to do)
        let named = "SELECT btrim(alias) <> '' FROM devices WHERE id = $1::text::uuid";
        if client.query_opt(named, &[&device.id]).await.map_err(said)?.is_none_or(|r| r.get(0)) {
            return Ok(());
        }
    }
    Ok(())
}

/// Say that this device is here, and whether its camera streams now (every few seconds, on a connection of
/// its own: a long sync step must not make the camera look offline to the other devices).
pub async fn heartbeat(client: &Client, device: &Device, streaming: bool) -> Result<()> {
    uuid::Uuid::parse_str(&device.id).map_err(|_| "this device's id is not an id".to_string())?;
    client
        .execute(
            "UPDATE devices SET last_seen = now(), streaming_at = CASE WHEN $1 THEN now() ELSE streaming_at END WHERE id = $2::text::uuid",
            &[&streaming, &device.id],
        )
        .await
        .map_err(|e| format!("The shared database couldn't be told this device is here: {e}"))?;
    Ok(())
}

// ---- setting a device up: the login kept ---------------------------------------------------------------------

use face_files::protect::{protect, write_private};

/// Set this device up for the shared database with a login: log in
/// over an encrypted connection with channel binding, which proves that the server at the other end of this
/// very connection knows the password (without it, something between the two could pass the login along
/// and hand over an authority of its own); take the server's certificate authority from its `server_ca`
/// table; keep the settings with it in `HOME/database.json`, so that every later connection checks the
/// server. The settings as kept.
pub async fn setup(join: &join::Login, home: &Path) -> Result<Settings> {
    let first = Settings {
        host: join.host.clone(),
        port: join.port,
        dbname: join.dbname.clone(),
        user: join.user.clone(),
        password: join.password.clone(),
        ca: String::new(), // none yet: `open` then insists on channel binding
        schema: join.schema.clone(),
    };
    let client = open(&first).await.map_err(|e| format!("Couldn't log in: {e}"))?;
    let e = |e: tokio_postgres::Error| format!("Couldn't read the server's certificate authority: {e}");
    client.batch_execute(&format!("SET search_path = {}, public", first.schema)).await.map_err(e)?;
    let there: bool = client.query_one("SELECT to_regclass('server_ca') IS NOT NULL", &[]).await.map_err(e)?.get(0);
    let pem: Option<String> = if there {
        client.query_opt("SELECT pem FROM server_ca WHERE id = 1", &[]).await.map_err(e)?.map(|r| r.get(0))
    } else {
        None
    };
    let Some(pem) = pem.filter(|p| p.contains("BEGIN CERTIFICATE")) else {
        return Err("The server has no certificate authority in its server_ca table: ask its administrator to add it \
                    so this device can check the server."
            .to_string());
    };
    let kept = serde_json::json!({"host": first.host, "port": first.port, "dbname": first.dbname, "user": first.user,
                                  "schema": first.schema, "password": protect(&first.password)?, "ca": pem});
    write_private(
        &home.join("database.json"),
        serde_json::to_string_pretty(&kept).map_err(|e| e.to_string())?.as_bytes(),
    )?;
    Settings::load(home)?.ok_or_else(|| "The settings were kept but can't be read back.".to_string())
}

/// {ok, text} for the Settings tab: connect as this device does (the tables brought up to date, the device
/// noted) and say what answered.
pub async fn test(home: &Path, device: &Device, os: &str, app_version: &str) -> serde_json::Value {
    let said = |ok: bool, text: String| serde_json::json!({"ok": ok, "text": text});
    let first = |e: String| e.trim().lines().next().unwrap_or("").chars().take(300).collect::<String>();
    let settings = match Settings::load(home) {
        Ok(Some(s)) => s,
        Ok(None) => return said(false, "No shared database is set up.".to_string()),
        Err(e) => return said(false, format!("Couldn't use the database: {}", first(e))),
    };
    let asked = async {
        let mut client = connect(&settings, &settings.schema, device).await?;
        migrate(&mut client).await?;
        register(&client, device, os, app_version).await?;
        let ask =
            "SELECT version(), (SELECT extversion FROM pg_extension WHERE extname = 'vector'), current_schema()::text,
                          (SELECT count(*) FROM devices)";
        let row = client.query_one(ask, &[]).await.map_err(|e| e.to_string())?;
        let (version, vector, schema, devices): (String, Option<String>, String, i64) =
            (row.get(0), row.get(1), row.get(2), row.get(3));
        let version = version.split(" on ").next().unwrap_or("").to_string();
        Ok::<_, String>(format!(
            "Connected: {version}, pgvector {}, schema {schema}, {devices} device(s).",
            vector.unwrap_or_else(|| "None".to_string())
        ))
    };
    match asked.await {
        Ok(text) => said(true, text),
        Err(e) => said(false, format!("Couldn't use the database: {}", first(e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_settings_shows_of_the_database_never_holds_the_password() {
        let home = std::env::temp_dir().join(format!("face-db-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(config(&home).is_none() && saved_login(&home).is_none() && forget(&home).is_ok());
        let password = face_files::protect::protect("a made-up password").unwrap();
        let kept = serde_json::json!({"host": "db.example", "port": 5433, "dbname": "faces", "user": "facerec", "schema": "facerec",
                                      "password": password, "ca": "-----BEGIN CERTIFICATE-----"});
        std::fs::write(home.join("database.json"), kept.to_string()).unwrap();
        let shown = config(&home).unwrap();
        assert_eq!(
            shown,
            serde_json::json!({"host": "db.example", "port": 5433, "dbname": "faces", "user": "facerec", "schema": "facerec", "ca": true})
        );
        assert!(!shown.to_string().contains("password"));
        let login = saved_login(&home).unwrap();
        assert_eq!(
            login,
            serde_json::json!({"host": "db.example", "port": "5433", "dbname": "faces", "user": "facerec", "password": "a made-up password"})
        );
        // no host: not set up
        std::fs::write(home.join("database.json"), r#"{"host": "", "password": "x"}"#).unwrap();
        assert!(config(&home).is_none() && saved_login(&home).is_none());
        forget(&home).unwrap();
        assert!(!home.join("database.json").exists());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_name_that_is_only_a_number_the_app_gave_is_told() {
        for (own, alias) in
            [("iPhone", "iPhone 2"), ("iPhone", "iphone 17"), (" iPhone ", "IPHONE 9"), ("My phone", "my phone 3")]
        {
            assert!(numbered(own, alias), "{own} / {alias}");
        }
        for (own, alias) in [
            ("iPhone", ""),
            ("iPhone", "iPhone"),
            ("iPhone", "iPhone 2a"),
            ("iPhone", "iPhone  2"),
            ("iPhone", "Kitchen 2"),
            ("iPhone", "iPhone 123"),
            ("iPhone", "iPhone2"),
            ("iPhone", "iPhone 17 Pro"),
        ] {
            assert!(!numbered(own, alias), "{own} / {alias}");
        }
        // a name too long for a number after it was cut for one
        let long = "a".repeat(join::NAME_MAX);
        assert!(numbered(&long, &format!("{} 2", "a".repeat(join::NAME_MAX - 4))));
    }

    #[test]
    fn the_migrations_are_the_folders() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|f| f.ok().map(|f| f.file_name().to_string_lossy().into_owned()))
            .filter(|n| n.ends_with(".sql") && n.as_bytes()[..3].iter().all(u8::is_ascii_digit))
            .collect();
        files.sort();
        assert_eq!(files.len(), MIGRATIONS.len(), "a migration file is not in MIGRATIONS: {files:?}");
        for (file, (n, sql)) in files.iter().zip(MIGRATIONS) {
            assert_eq!(file[..3].parse::<i32>().unwrap(), *n);
            assert_eq!(
                std::fs::read_to_string(dir.join(file)).unwrap().replace("\r\n", "\n"),
                sql.replace("\r\n", "\n"),
                "{file}"
            );
            // a migration that doesn't note its own number is run again at every start of every device
            // (the thirteenth was released so: each start made the faces' index again)
            assert!(sql.contains(&format!("INSERT INTO schema_version VALUES ({n});")), "{file} doesn't note itself");
        }
    }

    #[test]
    fn a_device_gets_its_id_once() {
        let dir = std::env::temp_dir().join(format!("face-db-device-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Device::load(&dir, " J. Doe's phone\n").unwrap();
        assert_eq!(first.name, "J. Doe's phone", "the name as the shell said it");
        assert!(uuid::Uuid::parse_str(&first.id).is_ok());
        assert_eq!(Device::load(&dir, "another name").unwrap(), first, "asked again, it is the same device");
        std::fs::write(dir.join("device.json"), b"{ not json").unwrap();
        assert_ne!(Device::load(&dir, "").unwrap().id, first.id, "a file that can't be read is made anew");
        assert_eq!(Device::load(&dir, "").unwrap().name, "this device");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_schema_name_is_checked_before_it_goes_into_a_statement() {
        for good in ["facerec", "t_0a1b", "_x"] {
            assert!(schema_name(good), "{good}");
        }
        for bad in ["", "Facerec", "1abc", "a-b", "a b", "a;drop", "a\"b", &"a".repeat(64)] {
            assert!(!schema_name(bad), "{bad}");
        }
    }

    #[test]
    fn a_saved_password_is_read_back_or_refused() {
        assert_eq!(unprotect("plain:c2VjcmV0").unwrap(), "secret");
        for bad in ["", "secret", "plain:###", "other:c2VjcmV0"] {
            let e = unprotect(bad).unwrap_err();
            assert!(!e.contains("secret"), "the message says nothing of what was saved");
        }
    }

    #[test]
    fn no_settings_means_no_shared_database() {
        let dir = std::env::temp_dir().join(format!("face-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Settings::load(&dir).unwrap().is_none());
        std::fs::write(dir.join("database.json"), br#"{"host": ""}"#).unwrap();
        assert!(Settings::load(&dir).unwrap().is_none());
        std::fs::write(dir.join("database.json"), br#"{"host": "db", "schema": "Bad Name", "password": "plain:eA=="}"#)
            .unwrap();
        assert!(Settings::load(&dir).is_err());
        std::fs::write(
            dir.join("database.json"),
            br#"{"host": "db", "port": 5433, "user": "u", "password": "plain:eA=="}"#,
        )
        .unwrap();
        let s = Settings::load(&dir).unwrap().unwrap();
        assert_eq!(
            (s.host.as_str(), s.port, s.dbname.as_str(), s.schema.as_str()),
            ("db", 5433, "postgres", "facerec")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
