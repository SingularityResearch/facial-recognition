//! Photos decoded, a line of numbers each, to compare two builds or two devices:
//!
//!     decode_try      ({"path": ..., "dump": <a file to write the picture's bytes to, if wanted>} a line)
//!
//! For each a line of JSON: {"ok": {"w", "h", "sha1" of the picture's bytes (blue, green, red), "ms"}} or
//! {"error": <why>}.

use std::{io::BufRead, path::Path, time::Instant};

use face_import::decode;
use face_library::photofiles::sha1_hex;
use serde_json::{Value, json};

fn main() {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let started = Instant::now();
        if let Some(size) = ask["resize"].as_array() {
            // a picture's bytes (three a place) from a file, shrunk, into another file: the shrinking on its own
            let n: Vec<usize> = size.iter().map(|v| v.as_u64().unwrap_or(0) as usize).collect();
            let px = std::fs::read(ask["path"].as_str().unwrap_or("")).unwrap_or_default();
            let out = face_import::resample::resize(&px, n[0], n[1], 3, n[2], n[3]);
            let _ = std::fs::write(ask["dump"].as_str().unwrap_or(""), &out);
            println!("{}", json!({"ok": {"w": n[2], "h": n[3]}}));
            continue;
        }
        let answer = decode::load_file(Path::new(ask["path"].as_str().unwrap_or(""))).map(|p| {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if let Some(dump) = ask["dump"].as_str() {
                let _ = std::fs::write(dump, &p.px);
            }
            json!({"w": p.w, "h": p.h, "sha1": sha1_hex(&p.px), "ms": ms, "warned": decode::jpeg_warning()})
        });
        println!("{}", answer.map_or_else(|e| json!({"error": e}), |v| json!({"ok": v})));
    }
}
