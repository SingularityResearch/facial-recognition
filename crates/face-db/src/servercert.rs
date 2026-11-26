//! Certificates for a new database server, made by the app for its owner (the owner's words, 2026-11-25: "offer to
//! create the CA for the user when the database gets setup and register it", and "anything we did here in our tests -
//! the app or app installer should be able to do for the user").
//!
//! The devices check the server against its own certificate authority (`verify-ca`), so a server needs a certificate
//! signed by an authority, not a self-signed one. This makes both: an authority of the owner's own, kept on this
//! device (its key protected, so that a server's certificate can be made again under it), and the server's
//! certificate and key, put in `HOME/database-server/` with the lines the server's two configuration files need.
//! When this device then connects, the authority is registered in the server's `server_ca` table, from where every
//! device that joins takes it. The keys are ECDSA P-256 with SHA-256: the login's channel binding needs a signature
//! it knows (not Ed25519). Nothing here leaves the device; the owner copies the server's two files to the server.

use std::path::{Path, PathBuf};

use rustls::pki_types::{CertificateDer, pem::PemObject};

use crate::Result;

/// The folder the server's files are put in, in the private folder.
pub const FOLDER: &str = "database-server";
const CA_CERT: &str = "root.crt";
const CA_KEY: &str = "ca.json";
const SERVER_CERT: &str = "server.crt";
const SERVER_KEY: &str = "server.key";
const STEPS: &str = "setup-the-server.txt";
/// How long the certificates hold (a choice: a server of one's own, renewed rarely; the devices check the dates)
const CA_DAYS: i64 = 20 * 365;
const SERVER_DAYS: i64 = 10 * 365;

/// What the folder holds now, for Settings: {folder, ca (an authority is here), made (this app made it), server (a
/// server's certificate is here), names}.
pub fn state(home: &Path) -> serde_json::Value {
    let dir = home.join(FOLDER);
    let names: Vec<String> = std::fs::read(dir.join(CA_KEY))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["server_names"].as_array().cloned())
        .map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    // (the steps, so the setup page can show them where the files are)
    let steps = std::fs::read_to_string(dir.join(STEPS)).unwrap_or_default();
    serde_json::json!({"folder": dir.display().to_string(), "ca": dir.join(CA_CERT).is_file(),
                       "made": dir.join(CA_KEY).is_file(), "server": dir.join(SERVER_CERT).is_file(), "names": names,
                       "steps": steps})
}

/// The authority this device registers in a new server (made here, or one of the owner's own given here).
pub fn authority(home: &Path) -> Option<String> {
    pem_of(&std::fs::read_to_string(home.join(FOLDER).join(CA_CERT)).ok()?)
}

/// The authority's name and key, kept so that a server's certificate can be made again under it.
fn kept_authority(home: &Path) -> Option<(String, rcgen::KeyPair)> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(home.join(FOLDER).join(CA_KEY)).ok()?).ok()?;
    let key = face_files::protect::unprotect(v["key"].as_str()?).ok()?;
    Some((v["name"].as_str()?.to_string(), rcgen::KeyPair::from_pem(&key).ok()?))
}

fn authority_params(name: &str) -> rcgen::CertificateParams {
    let mut p = rcgen::CertificateParams::default();
    p.distinguished_name = rcgen::DistinguishedName::new();
    p.distinguished_name.push(rcgen::DnType::CommonName, name);
    p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    p.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::CrlSign];
    p
}

/// Makes the server's certificate and key for these names (its addresses as the devices reach it: they are put
/// in the certificate, though the devices check the authority and not the name), under this device's authority,
/// made first when there is none. A different authority given here before (`keep_given`) is replaced. The
/// folder's path.
pub fn make(home: &Path, names: &[String]) -> Result<PathBuf> {
    let dir = home.join(FOLDER);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't make {}: {e}", dir.display()))?;
    let now = time::OffsetDateTime::now_utc();
    let (name, ca_key) = match kept_authority(home) {
        Some(kept) => kept,
        None => {
            let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
            // (a name of its own for each authority, so that two never look alike)
            let name = format!("Face Recognizer database authority {}", &crate::new_id()[..8]);
            let mut p = authority_params(&name);
            p.not_before = now - time::Duration::days(1);
            p.not_after = now + time::Duration::days(CA_DAYS);
            let cert = p.self_signed(&key).map_err(|e| e.to_string())?;
            let protected = face_files::protect::protect(&key.serialize_pem())?;
            let kept = serde_json::json!({"name": name, "key": protected, "server_names": names});
            face_files::protect::write_private(&dir.join(CA_KEY), kept.to_string().as_bytes())?;
            std::fs::write(dir.join(CA_CERT), cert.pem()).map_err(|e| e.to_string())?;
            (name, key)
        }
    };
    // the authority's certificate as it is kept (made now, or before)
    let issuer = rcgen::Issuer::new(authority_params(&name), &ca_key);
    let mut names: Vec<String> = names.iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    for local in ["localhost", "127.0.0.1"] {
        if !names.iter().any(|n| n == local) {
            names.push(local.to_string());
        }
    }
    let mut p = rcgen::CertificateParams::new(names.clone()).map_err(|e| format!("Not a server's address: {e}"))?;
    p.distinguished_name = rcgen::DistinguishedName::new();
    p.distinguished_name.push(rcgen::DnType::CommonName, names[0].clone());
    p.is_ca = rcgen::IsCa::ExplicitNoCa;
    p.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature, rcgen::KeyUsagePurpose::KeyEncipherment];
    p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    p.use_authority_key_identifier_extension = true;
    p.not_before = now - time::Duration::days(1);
    p.not_after = now + time::Duration::days(SERVER_DAYS);
    let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = p.signed_by(&key, &issuer).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(SERVER_CERT), cert.pem()).map_err(|e| e.to_string())?;
    // (the server's key: for its owner to copy to the server, readable by them alone)
    face_files::protect::write_private(&dir.join(SERVER_KEY), key.serialize_pem().as_bytes())?;
    if let Ok(b) = std::fs::read(dir.join(CA_KEY))
        && let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&b)
    {
        v["server_names"] = serde_json::json!(names);
        let _ = face_files::protect::write_private(&dir.join(CA_KEY), v.to_string().as_bytes());
    }
    std::fs::write(dir.join(STEPS), steps(&names[0])).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// An authority of the owner's own (they made the server's certificate themselves): kept as the one this device
/// registers. Pasted text is taken as it comes (a field of one line loses the PEM's line breaks).
pub fn keep_given(home: &Path, text: &str) -> Result<()> {
    let pem = pem_of(text).ok_or("That isn't a certificate (PEM: -----BEGIN CERTIFICATE----- ...).")?;
    let dir = home.join(FOLDER);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // (an authority made here before isn't the one any more: its key goes, so nothing is made under it)
    let _ = std::fs::remove_file(dir.join(CA_KEY));
    for f in [SERVER_CERT, SERVER_KEY, STEPS] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    std::fs::write(dir.join(CA_CERT), pem).map_err(|e| e.to_string())
}

/// The certificates in a text, as PEM with its line breaks, None for anything else.
pub fn pem_of(text: &str) -> Option<String> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let end = after.find(END)?;
        let body: String = after[..end].chars().filter(|c| !c.is_whitespace()).collect();
        if body.is_empty() || !body.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)) {
            return None;
        }
        out.push_str(BEGIN);
        out.push('\n');
        for line in body.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(line).ok()?);
            out.push('\n');
        }
        out.push_str(END);
        out.push('\n');
        rest = &after[end + END.len()..];
    }
    (!out.is_empty() && CertificateDer::pem_slice_iter(out.as_bytes()).all(|c| c.is_ok())).then_some(out)
}

/// What the server's owner does with the two files: the lines for postgresql.conf and pg_hba.conf.
fn steps(server: &str) -> String {
    format!(
        "Face Recognizer: certificates for your database server ({server})\n\
         \n\
         1. Copy server.crt and server.key from this folder into the server's data folder (the folder that\n   \
            holds postgresql.conf). On Linux and macOS the key must belong to the server's user and be readable\n   \
            by it alone:  chown postgres:postgres server.key ; chmod 600 server.key\n\
         2. In postgresql.conf, set (remove a # at the start of a line to use it):\n   \
            listen_addresses = '*'\n   \
            ssl = on\n   \
            ssl_cert_file = 'server.crt'\n   \
            ssl_key_file = 'server.key'\n   \
            password_encryption = scram-sha-256\n\
         3. At the end of pg_hba.conf, add a line for your local network (here 192.168.1.0/24: use your own):\n   \
            hostssl  all  all  192.168.1.0/24  scram-sha-256\n\
         4. Restart PostgreSQL. Let its port (5432) through the server's firewall, for your local network only.\n\
         5. In Face Recognizer, Settings, Devices and shared database, Shared database: Connect, the first time\n   \
            with the server's administrator login (user postgres and its password). The app makes its own login,\n   \
            the database, pgvector and its tables, and registers this folder's root.crt in the server. It keeps\n   \
            only its own login, never the administrator's.\n\
         \n\
         root.crt is the certificate authority (public: every device takes it from the server). Keep server.key\n\
         private; once it is on the server you may delete it here (Make them again makes a new one).\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("face-db-servercert-{}", crate::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_server_s_certificate_is_trusted_through_the_authority_made_with_it_and_by_nothing_else() {
        let home = fresh();
        let dir = make(&home, &["192.168.1.20".to_string()]).unwrap();
        let ca = authority(&home).unwrap();
        let server = std::fs::read(dir.join(SERVER_CERT)).unwrap();
        let check = |ca: &str| {
            let mut roots = rustls::RootCertStore::empty();
            for c in CertificateDer::pem_slice_iter(ca.as_bytes()) {
                roots.add(c.unwrap()).unwrap();
            }
            let v = rustls::client::WebPkiServerVerifier::builder_with_provider(
                std::sync::Arc::new(roots),
                std::sync::Arc::new(rustls::crypto::ring::default_provider()),
            )
            .build()
            .unwrap();
            let leaf = CertificateDer::pem_slice_iter(&server).next().unwrap().unwrap();
            use rustls::client::danger::ServerCertVerifier;
            v.verify_server_cert(
                &leaf,
                &[],
                &rustls::pki_types::ServerName::try_from("192.168.1.20").unwrap(),
                &[],
                rustls::pki_types::UnixTime::now(),
            )
        };
        assert!(check(&ca).is_ok(), "the server's certificate comes from the authority made with it");
        // made again: the same authority signs a new certificate, so the devices that have it go on
        make(&home, &["db.local".to_string()]).unwrap();
        assert_eq!(authority(&home).unwrap(), ca, "the authority stays");
        // another authority (one made on another device) doesn't vouch for it
        let other = fresh();
        make(&other, &["x".to_string()]).unwrap();
        assert!(check(&authority(&other).unwrap()).is_err());
        // the key that signs is kept protected; the server's key is written for its owner alone; the steps say
        // what the server's two files need
        let kept = std::fs::read_to_string(dir.join(CA_KEY)).unwrap();
        assert!(!kept.contains("PRIVATE KEY"), "the authority's key is never kept in the clear");
        let steps = std::fs::read_to_string(dir.join(STEPS)).unwrap();
        for line in ["ssl = on", "ssl_cert_file = 'server.crt'", "hostssl", "scram-sha-256", "listen_addresses"] {
            assert!(steps.contains(line), "{line}");
        }
        let st = state(&home);
        assert_eq!(
            (st["ca"].as_bool(), st["made"].as_bool(), st["server"].as_bool()),
            (Some(true), Some(true), Some(true))
        );
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn a_pasted_authority_is_read_whatever_became_of_its_line_breaks() {
        let home = fresh();
        make(&home, &["x".to_string()]).unwrap();
        let pem = authority(&home).unwrap();
        let flat = pem.replace('\n', " ");
        assert_eq!(pem_of(&flat).as_deref(), Some(pem.as_str()));
        assert_eq!(pem_of("not a certificate"), None);
        assert_eq!(pem_of("-----BEGIN CERTIFICATE----- <script> -----END CERTIFICATE-----"), None);
        // an authority of the owner's own takes the place of the one made here, and nothing is made under it again
        let mine = fresh();
        keep_given(&mine, &flat).unwrap();
        assert_eq!(authority(&mine).unwrap(), pem);
        assert_eq!(state(&mine)["made"], false);
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(mine);
    }
}
