//! A photo that only another device has, shown to this one (the asking side of the peer service
//! for photos).
//!
//! The shared database says which devices have a photo and where their peer services are. One is asked over
//! TLS, its certificate checked inside the handshake against the SHA-256 the database has for it, the
//! request signed with the cluster's key, and it answers with the photo view's JPEG (a video's frame by its
//! number). A photo is asked for by its content hash only, is at most `MAX_PHOTO` bytes, and stays in
//! memory: shown, or described, never stored.

use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use face_db::{
    Device,
    kept::{Kept, Peer},
    peer::sign,
};

use crate::{
    camrelay::{clock_note, order, pinned, unix_now, why},
    hardware,
};

/// bytes accepted from another device
pub const MAX_PHOTO: usize = 60_000_000;
/// in `fetch`'s reason when no device that has the photo answered at all (a device that answered without
/// the photo was reached: Describe everything again stops asking only after this kind)
pub const UNREACHED: &str = "can't be reached now";
/// how long this computer's own addresses are taken as looked up
const ADDRESSES_FOR: Duration = Duration::from_secs(60);

/// What asking the other devices remembers between photos.
pub struct Asker {
    home: PathBuf,
    /// the connection kept for short questions to the shared database
    kept: Kept,
    /// a device's certificate -> its address that answered last
    good: Mutex<HashMap<String, String>>,
    /// address -> when the log last said that device's clock is far off
    clock_said: Mutex<HashMap<String, Instant>>,
    addresses: Mutex<Option<Addresses>>,
}

/// This computer's addresses as last looked up: those other devices reach it at, and every one it has.
struct Addresses {
    at: Instant,
    reach: Vec<String>,
    own: Vec<String>,
}

/// Why one device at one address gave no photo.
enum Fault {
    /// it was reached and refused (the key, the clocks): its other addresses would say the same
    Refused(String),
    /// it wasn't reached there, or answered something else
    Failed(String),
}

/// This device's own peer service's port, running or not: `FACE_PEER_PORT`, or three after the app's first.
pub fn own_port() -> u16 {
    let said = |name: &str| std::env::var(name).ok();
    port_from(said("FACE_PEER_PORT").as_deref(), said("FACE_PORTS").as_deref())
}

fn port_from(peer_port: Option<&str>, ports: Option<&str>) -> u16 {
    let number = |text: Option<&str>| {
        text.filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))?.parse::<u32>().ok()
    };
    let port = number(peer_port).unwrap_or_else(|| number(ports).unwrap_or(8764) + 3);
    u16::try_from(port).unwrap_or(0)
}

impl Asker {
    pub fn new(home: &Path) -> Asker {
        Asker {
            home: home.to_path_buf(),
            kept: Kept::new(home),
            good: Mutex::new(HashMap::new()),
            clock_said: Mutex::new(HashMap::new()),
            addresses: Mutex::new(None),
        }
    }

    /// (the addresses other devices can reach this computer at, every address of this computer)
    fn addresses(&self) -> (Vec<String>, Vec<String>) {
        let mut kept = self.addresses.lock().unwrap_or_else(|e| e.into_inner());
        let known = kept.take().filter(|a| a.at.elapsed() <= ADDRESSES_FOR).unwrap_or_else(|| {
            let text = |list: Vec<std::net::Ipv4Addr>| list.iter().map(ToString::to_string).collect::<Vec<_>>();
            Addresses {
                at: Instant::now(),
                reach: text(hardware::reachable_addresses(&self.home, None)),
                own: text(hardware::own_addresses(&self.home, Some(true))),
            }
        });
        let found = (known.reach.clone(), known.own.clone());
        *kept = Some(known);
        found
    }

    /// The names of the other sharing devices that have a photo, most recently seen first; none when the
    /// shared database can't be asked.
    pub fn holders(&self, photo: &str) -> Vec<String> {
        let Ok(me) = Device::load(&self.home, &crate::device().0).map(|d| d.id) else { return Vec::new() };
        let removed = face_home::blocked::ids(&self.home);
        let found = self.kept.ask(async |client, _| face_db::kept::holders(client, photo, &me, &removed).await);
        found.map(|list| list.into_iter().map(|p| p.name).collect()).unwrap_or_default()
    }

    /// GET `path` from one device at one address: bytes, or None when it has no such photo.
    fn get(&self, addr: &str, peer: &Peer, key: &[u8; 32], me: &str, path: &str) -> Result<Option<Vec<u8>>, Fault> {
        let failed = |e: std::io::Error| Fault::Failed(e.to_string());
        let mut tls = pinned(addr, peer).map_err(Fault::Failed)?;
        let when = unix_now().to_string();
        let ask = format!(
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nX-Peer-Device: {me}\r\nX-Peer-Time: {when}\r\nX-Peer-Sig: {}\r\n\r\n",
            sign(key, "GET", path, &when, me)
        );
        tls.write_all(ask.as_bytes()).map_err(failed)?;
        let (mut data, mut chunk) = (Vec::new(), vec![0u8; 65_536]);
        while data.len() < MAX_PHOTO + 65_536 {
            match tls.read(&mut chunk) {
                Ok(0) => break,
                Ok(got) => data.extend_from_slice(&chunk[..got]),
                // a device that closes without TLS's goodbye has still said all it had: the length is checked below
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(failed(e)),
            }
        }
        let (head, body) = match data.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(at) => (&data[..at], &data[at + 4..]),
            None => (&data[..], &data[..0]),
        };
        let head = String::from_utf8_lossy(head);
        match head.split("\r\n").next().and_then(|line| line.split_whitespace().nth(1)) {
            Some("200") => {}
            Some("403") => return Err(Fault::Refused(format!("refused: {}", why(body, unix_now())))),
            Some("404") | None => return Ok(None),
            Some(code) => {
                let said: String = String::from_utf8_lossy(body).chars().take(100).collect();
                return Err(Fault::Failed(format!("answered {code}: {said}")));
            }
        }
        clock_note(&self.clock_said, addr, &head);
        let length = head.split("\r\n").skip(1).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
        });
        if length.is_some_and(|n| n != body.len()) {
            return Ok(None);
        }
        Ok((!body.is_empty()).then(|| body.to_vec()))
    }

    /// The photo view's JPEG of a photo (a frame of a video) from another device that has it, with that
    /// device's name; or why not.
    pub fn fetch(&self, photo: &str, frame: Option<i64>) -> Result<(Vec<u8>, String), String> {
        let removed = face_home::blocked::ids(&self.home);
        let asked = Device::load(&self.home, &crate::device().0).and_then(|me| {
            let me = me.id;
            let found = self.kept.ask(async |client, settings| {
                let holders = face_db::kept::holders(client, photo, &me, &removed).await?;
                Ok((holders, face_db::peer::cluster_key(client, settings).await?))
            })?;
            Ok((me, found))
        });
        let (me, (found, key)) = asked.map_err(|e| format!("the shared database can't be reached ({e})"))?;
        if found.is_empty() {
            return Err("no other device that shares its photos has it".to_string());
        }
        let path = match frame {
            Some(k) => format!("/photo/{photo}?frame={k}"),
            None => format!("/photo/{photo}"),
        };
        let (reach, own) = self.addresses();
        let good = |cert: &str| self.good.lock().unwrap_or_else(|e| e.into_inner()).get(cert).cloned();
        let (mut tried, mut last, mut answered) = (Vec::new(), String::new(), false);
        for peer in &found {
            for addr in order(&peer.addresses, peer.port, good(&peer.cert_sha256).as_deref(), &reach, &own, own_port())
            {
                match self.get(&addr, peer, &key, &me, &path) {
                    Ok(jpeg) => {
                        // it answered, with the photo or without
                        self.good
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(peer.cert_sha256.clone(), addr.clone());
                        if let Some(jpeg) = jpeg {
                            return Ok((jpeg, peer.name.clone()));
                        }
                        (last, answered) = (format!("{addr}:{} didn't send it", peer.port), true);
                        break; // the same device has no other answer at its other addresses
                    }
                    Err(Fault::Refused(e)) => {
                        last = format!("{addr}:{}: {e}", peer.port);
                        break;
                    }
                    Err(Fault::Failed(e)) => {
                        let mut good = self.good.lock().unwrap_or_else(|e| e.into_inner());
                        if good.get(&peer.cert_sha256) == Some(&addr) {
                            good.remove(&peer.cert_sha256);
                        }
                        last = format!("{addr}:{}: {e}", peer.port);
                    }
                }
            }
            tried.push(peer.name.clone());
        }
        let how = if answered { "did not send it" } else { UNREACHED };
        Err(format!("it's on {}, which {how} ({last})", tried.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::Arc,
    };

    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    use super::*;

    /// A stand-in for another device's peer service on this computer: TLS with a certificate made here, one
    /// answer to whatever is asked, then the connection is closed. (its port, its certificate's SHA-256,
    /// what it was asked)
    fn device(answers: Vec<Vec<u8>>) -> (u16, String, Arc<Mutex<Vec<String>>>) {
        let made = rcgen::generate_simple_self_signed(vec!["peer".to_string()]).unwrap();
        let der = made.cert.der().to_vec();
        let sha: String =
            ring::digest::digest(&ring::digest::SHA256, &der).as_ref().iter().map(|b| format!("{b:02x}")).collect();
        let key = PrivateKeyDer::try_from(made.signing_key.serialize_der()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(der)], key)
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (config, heard) = (Arc::new(config), asked.clone());
        std::thread::spawn(move || {
            for answer in answers {
                let Ok((tcp, _)) = listener.accept() else { return };
                let Ok(conn) = rustls::ServerConnection::new(config.clone()) else { return };
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let (mut head, mut chunk) = (Vec::new(), [0u8; 2048]);
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut chunk) {
                        Ok(got) if got > 0 => head.extend_from_slice(&chunk[..got]),
                        _ => break,
                    }
                }
                heard.lock().unwrap().push(String::from_utf8_lossy(&head).into_owned());
                let _ = tls.write_all(&answer);
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
        });
        (port, sha, asked)
    }

    fn answer(code: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {code}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", body.len())
            .into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn a_photo_is_asked_for_signed_and_taken_only_whole_and_from_the_right_device() {
        let home = std::env::temp_dir().join(format!("face-peerphoto-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let asker = Asker::new(&home);
        let key = [7u8; 32];
        let photo = "ab".repeat(20);
        let cut = {
            let mut whole = answer("200 OK", b"a picture that is cut off");
            whole.truncate(whole.len() - 5);
            whole
        };
        let (port, sha, asked) = device(vec![
            answer("200 OK", b"a picture"),
            answer("404 Not Found", b"{\"error\": \"not found\"}"),
            answer("403 Forbidden", b"{\"error\": \"bad signature\"}"),
            answer("500 Oops", b"something else"),
            cut,
            answer("200 OK", b""),
        ]);
        let peer =
            |sha: &str| Peer { name: "other".to_string(), addresses: Vec::new(), port, cert_sha256: sha.to_string() };
        let get = |path: &str| asker.get("127.0.0.1", &peer(&sha), &key, "me", path);

        assert_eq!(get(&format!("/photo/{photo}?frame=12")).ok().flatten().as_deref(), Some(&b"a picture"[..]));
        let head = asked.lock().unwrap()[0].clone();
        assert!(head.starts_with(&format!("GET /photo/{photo}?frame=12 HTTP/1.1\r\n")));
        let said = |name: &str| head.lines().find_map(|l| l.strip_prefix(name)).unwrap_or("").to_string();
        // the request is signed over its method, path, time and this device, as the service checks it
        assert!(face_db::peer::signed(
            &key,
            "GET",
            &format!("/photo/{photo}?frame=12"),
            &said("X-Peer-Time: "),
            "me",
            &said("X-Peer-Sig: ")
        ));
        assert_eq!(said("X-Peer-Device: "), "me");

        assert!(matches!(get("/photo/x"), Ok(None)), "a device without the photo is no error");
        assert!(matches!(get("/photo/x"), Err(Fault::Refused(why)) if why == "refused: bad signature"));
        assert!(matches!(get("/photo/x"), Err(Fault::Failed(why)) if why == "answered 500: something else"));
        assert!(matches!(get("/photo/x"), Ok(None)), "a picture that is cut off is not taken");
        assert!(matches!(get("/photo/x"), Ok(None)), "nor an empty one");

        // another certificate than the database has for the device: nothing is asked of it
        let (port, _, asked) = device(vec![answer("200 OK", b"a picture")]);
        let other = Peer { name: "other".to_string(), addresses: Vec::new(), port, cert_sha256: "00".repeat(32) };
        assert!(matches!(asker.get("127.0.0.1", &other, &key, "me", "/photo/x"), Err(Fault::Failed(_))));
        assert!(asked.lock().unwrap().iter().all(|head| !head.contains("X-Peer-Sig")), "no signed request went to it");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn this_devices_own_port_is_the_fixed_one() {
        assert_eq!(port_from(None, None), 8767);
        assert_eq!(port_from(None, Some("9100")), 9103);
        assert_eq!(port_from(Some("9555"), Some("9100")), 9555);
        assert_eq!(port_from(Some("no"), Some("9100")), 9103);
        assert_eq!(port_from(Some(""), Some("")), 8767);
    }
}
