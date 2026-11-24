//! For the tests: a web server on this computer that serves what a test gives it (whole, by parts, or with an
//! error a number of times first), and notes what it was asked. No test reaches the internet.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

#[derive(Default)]
struct Served {
    /// path -> its bytes
    files: HashMap<String, Vec<u8>>,
    /// path -> how many askings are still answered with this status first
    failing: HashMap<String, (u16, u32)>,
    /// every asking: (method, path, the part asked for, the kind of answer asked for)
    asked: Vec<(String, String, Option<String>, Option<String>)>,
}

pub struct Server {
    pub base: String,
    served: Arc<Mutex<Served>>,
}

impl Server {
    pub fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = Arc::new(Mutex::new(Served::default()));
        let shared = served.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
                let first = lines.next().and_then(Result::ok).unwrap_or_default();
                let mut words = first.split_whitespace();
                let (method, path) = (words.next().unwrap_or("").to_string(), words.next().unwrap_or("").to_string());
                let (mut part, mut accept) = (None, None);
                for line in lines.map_while(Result::ok).take_while(|l| !l.is_empty()) {
                    let (name, value) = line.split_once(':').unwrap_or(("", ""));
                    match name.to_ascii_lowercase().as_str() {
                        "range" => part = Some(value.trim().to_string()),
                        "accept" => accept = Some(value.trim().to_string()),
                        _ => {}
                    }
                }
                let mut s = shared.lock().unwrap();
                s.asked.push((method.clone(), path.clone(), part.clone(), accept));
                let failing = s.failing.get_mut(&path).filter(|f| f.1 > 0).map(|f| {
                    f.1 -= 1;
                    f.0
                });
                let (status, body, more) = match (failing, s.files.get(&path)) {
                    (Some(status), _) => (status, Vec::new(), String::new()),
                    (None, None) => (404, Vec::new(), String::new()),
                    (None, Some(data)) => {
                        match part.as_deref().and_then(|p| p.strip_prefix("bytes=")).and_then(|p| p.split_once('-')) {
                            Some((from, to)) => {
                                let from: usize = from.parse().unwrap_or(0);
                                let to = to.parse::<usize>().unwrap_or(usize::MAX).min(data.len().saturating_sub(1));
                                let more = format!("Content-Range: bytes {from}-{to}/{}\r\n", data.len());
                                (206, data.get(from..=to).unwrap_or(&[]).to_vec(), more)
                            }
                            None => (200, data.clone(), String::new()),
                        }
                    }
                };
                drop(s);
                let head =
                    format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{more}Connection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                if method != "HEAD" {
                    let _ = stream.write_all(&body);
                }
            }
        });
        Server { base, served }
    }

    /// Serve `data` at `path`; its whole address, for a pin (which keeps addresses for the program's life).
    pub fn put(&self, path: &str, data: &[u8]) -> &'static str {
        self.served.lock().unwrap().files.insert(path.to_string(), data.to_vec());
        self.url(path)
    }

    pub fn url(&self, path: &str) -> &'static str {
        Box::leak(format!("{}{path}", self.base).into_boxed_str())
    }

    /// Answer the next `times` askings for `path` with `status`.
    pub fn fail(&self, path: &str, status: u16, times: u32) {
        self.served.lock().unwrap().failing.insert(path.to_string(), (status, times));
    }

    /// The askings for `path` so far: (method, the part asked for, the kind of answer asked for).
    pub fn asked(&self, path: &str) -> Vec<(String, Option<String>, Option<String>)> {
        let s = self.served.lock().unwrap();
        s.asked.iter().filter(|a| a.1 == path).map(|a| (a.0.clone(), a.2.clone(), a.3.clone())).collect()
    }
}

/// A folder of the test's own, empty.
pub fn folder(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let at = std::env::temp_dir().join(format!(
        "face-tools-test-{}-{}-{name}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).unwrap();
    at
}

/// The SHA-256 of some bytes, as text that lives as long as a pin wants it to.
pub fn sha(data: &[u8]) -> &'static str {
    Box::leak(crate::fetch::hex(ring::digest::digest(&ring::digest::SHA256, data).as_ref()).into_boxed_str())
}
