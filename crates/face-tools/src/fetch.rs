//! Downloads that are pinned: a file is named by its address, its size and its SHA-256, and nothing of a
//! download is used, or left on the disk, unless it is that very file. For everything the programs fetch.

use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use ring::digest::{Context, SHA256};

/// What a request is sent as: a generic name, nothing of the user or the computer.
const AGENT: &str = "model-fetch";

/// A file as its publisher has it.
#[derive(Debug, Clone, Copy)]
pub struct Pin {
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// Why something couldn't be fetched. `lasting`: the server said the file isn't there (a 4xx answer), so
/// asking again won't help.
#[derive(Debug)]
pub struct Failure {
    pub why: String,
    pub lasting: bool,
}

impl Failure {
    pub fn new(why: impl Into<String>) -> Failure {
        Failure { why: why.into(), lasting: false }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Failure {
        Failure::new(e.to_string())
    }
}

pub type Fetched<T> = Result<T, Failure>;

/// The connections of one run.
pub struct Net {
    agent: ureq::Agent,
}

impl Default for Net {
    fn default() -> Net {
        Net::new()
    }
}

impl Net {
    pub fn new() -> Net {
        Net::with(!cfg!(test))
    }

    /// For a stand-in on this computer in a check: plain HTTP is taken. Never for a publisher's address.
    pub fn plain() -> Net {
        Net::with(false)
    }

    fn with(https_only: bool) -> Net {
        let config = ureq::Agent::config_builder()
            .user_agent(AGENT)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            // encrypted connections only, also where a publisher sends on to another address (the tests ask
            // a server on this computer, which has none)
            .https_only(https_only)
            .build();
        Net { agent: config.into() }
    }

    fn failed(url: &str, e: ureq::Error) -> Failure {
        let lasting = matches!(e, ureq::Error::StatusCode(code) if (400..500).contains(&code));
        Failure { why: format!("{}: {e}", shown(url)), lasting }
    }

    /// The file at `url`, or `part` of it: (the first byte, how many). `accept`: the kind of answer asked for,
    /// where a server needs to be told. A part must come back as a part, never as the whole file.
    pub fn open(&self, url: &str, part: Option<(u64, u64)>, accept: Option<&str>) -> Fetched<Box<dyn Read>> {
        let mut ask = self.agent.get(url);
        if let Some((from, n)) = part {
            ask = ask.header("Range", format!("bytes={from}-{}", from + n.max(1) - 1));
        }
        if let Some(kind) = accept {
            ask = ask.header("Accept", kind);
        }
        let answer = ask.call().map_err(|e| Net::failed(url, e))?;
        if part.is_some() && answer.status().as_u16() != 206 {
            return Err(Failure::new(format!("{} doesn't serve parts of the file", shown(url))));
        }
        Ok(Box::new(answer.into_body().into_reader()))
    }

    /// How large the file at `url` is, by the server's word.
    pub fn size(&self, url: &str) -> Fetched<u64> {
        let answer = self.agent.head(url).call().map_err(|e| Net::failed(url, e))?;
        let said = answer.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
        said.ok_or_else(|| Failure::new(format!("{} doesn't say how large it is", shown(url))))
    }

    /// A short answer as text (a listing), at most `limit` bytes of it.
    pub fn text(&self, url: &str, limit: u64) -> Fetched<String> {
        let mut out = String::new();
        self.open(url, None, None)?.take(limit).read_to_string(&mut out)?;
        Ok(out)
    }

    /// Download `pin` to `dest`, unless `dest` is that file already. Nothing is left behind otherwise.
    pub fn download(&self, pin: &Pin, dest: &Path, progress: &mut dyn FnMut(u64)) -> Fetched<()> {
        if is_file(dest, pin.size, pin.sha256) {
            return Ok(());
        }
        let from = self.open(pin.url, None, None)?;
        save(from, dest, pin.size, pin.sha256, progress)
    }
}

/// An address for a message: without what follows a "?" (a download link can carry a signature there).
fn shown(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of a file, as text.
pub fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut sum = Context::new(&SHA256);
    let mut block = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut block)?;
        if n == 0 {
            return Ok(hex(sum.finish().as_ref()));
        }
        sum.update(&block[..n]);
    }
}

/// Whether `path` is the file of this size and SHA-256.
pub fn is_file(path: &Path, size: u64, sha256: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == size) && sha256_of(path).is_ok_and(|s| s == sha256)
}

/// The name a download has until it is checked.
pub fn partial(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".download");
    dest.with_file_name(name)
}

/// What `from` gives, written to `dest` if and only if it is `size` bytes with this SHA-256: into a file
/// beside it first, never more on the disk than the file is known to be, and that file removed when it
/// isn't the one. `progress(bytes so far)` as it goes.
pub fn save(mut from: impl Read, dest: &Path, size: u64, sha256: &str, progress: &mut dyn FnMut(u64)) -> Fetched<()> {
    if let Some(folder) = dest.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let tmp = partial(dest);
    let name = dest.file_name().unwrap_or_default().to_string_lossy().to_string();
    let written = (|| -> Fetched<()> {
        let mut out = File::create(&tmp)?;
        let mut sum = Context::new(&SHA256);
        let mut block = vec![0u8; 1 << 20];
        let mut got = 0u64;
        loop {
            let n = from.read(&mut block)?;
            if n == 0 {
                break;
            }
            got += n as u64;
            if got > size {
                return Err(Failure::new(format!("{name}: the download is larger than the expected file")));
            }
            out.write_all(&block[..n])?;
            sum.update(&block[..n]);
            progress(got);
        }
        out.flush()?;
        drop(out);
        if got != size || hex(sum.finish().as_ref()) != sha256 {
            return Err(Failure::new(format!(
                "{name} downloaded, but it isn't the expected file (size or checksum differs)"
            )));
        }
        face_files::paths::replace_file(&tmp, dest)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Server, folder, sha};

    #[test]
    fn a_download_is_kept_only_when_it_is_the_pinned_file() {
        let (server, dir) = (Server::start(), folder("pinned"));
        let data: Vec<u8> = (0..300_000u32).map(|v| (v % 251) as u8).collect();
        let pin = Pin { url: server.put("/model.bin", &data), size: data.len() as u64, sha256: sha(&data) };
        let (net, dest) = (Net::new(), dir.join("sub").join("model.bin"));
        let mut seen = 0;
        net.download(&pin, &dest, &mut |got| seen = got).unwrap();
        assert_eq!((std::fs::read(&dest).unwrap(), seen), (data.clone(), data.len() as u64));
        // there and right: not asked for again
        net.download(&pin, &dest, &mut |_| ()).unwrap();
        assert_eq!(server.asked("/model.bin").len(), 1);
        // another file under the same address: refused, and nothing of it left
        for (what, served) in [
            ("other bytes", vec![7u8; data.len()]),
            ("longer", [data.clone(), vec![1]].concat()),
            ("shorter", data[..9].to_vec()),
        ] {
            server.put("/model.bin", &served);
            let other = dir.join(format!("{what}.bin"));
            let refused = net.download(&pin, &other, &mut |_| ()).unwrap_err();
            assert!(refused.why.contains("expected file") && !refused.lasting, "{what}: {}", refused.why);
            assert!(!other.exists() && !partial(&other).exists(), "{what}");
        }
        // a file that isn't there is a lasting failure; a server's own trouble isn't
        assert!(
            net.download(&Pin { url: server.url("/nothing"), ..pin }, &dir.join("n"), &mut |_| ()).unwrap_err().lasting
        );
        server.fail("/model.bin", 503, 1);
        assert!(!net.download(&pin, &dir.join("m"), &mut |_| ()).unwrap_err().lasting);
        // a part is a part, and the server's word on the size
        let mut part = Vec::new();
        server.put("/model.bin", &data);
        net.open(pin.url, Some((10, 5)), None).unwrap().read_to_end(&mut part).unwrap();
        assert_eq!((part.as_slice(), net.size(pin.url).unwrap()), (&data[10..15], data.len() as u64));
        let _ = std::fs::remove_dir_all(dir);
    }
}
