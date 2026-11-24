//! Videos read, a line of numbers a frame, to compare two builds or two devices:
//!
//!     video_try      ({"path": ..., "dump": <a folder to write the frames' bytes into, if wanted>} a line)
//!
//! For each a line of JSON: {"ok": {"fps", "frames": [[frame number, width, height, sha1 of its bytes]], "ms"}}
//! or {"error": <why>}. With {"probe": true}: what the video says about itself only; with {"frame": k}: that
//! frame as the photo view gets it; with {"jump": k}: that frame by a jump alone.

use std::{io::BufRead, path::Path, sync::atomic::AtomicBool, time::Instant};

use face_import::video;
use face_library::photofiles::sha1_hex;
use serde_json::{Value, json};

fn main() {
    let stop = AtomicBool::new(false);
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let path = Path::new(ask["path"].as_str().unwrap_or(""));
        let started = Instant::now();
        let answer = if ask["probe"] == true {
            video::probe(path).map(|i| json!({"fps": i.fps, "count": i.frames, "turn": i.turn}))
        } else if let Some(k) = ask["jump"].as_i64() {
            // the frame by a jump alone: null where the video doesn't say when its pictures are shown
            Ok(video::frame_by_jump(path, k)
                .map_or(Value::Null, |f| json!({"w": f.w, "h": f.h, "sha1": sha1_hex(&f.px)})))
        } else if let Some(k) = ask["frame"].as_i64() {
            video::frame(path, k).map(|f| json!({"w": f.w, "h": f.h, "sha1": sha1_hex(&f.px)}))
        } else {
            let mut frames = Vec::new();
            video::frames(path, &stop, |k, _, frame| {
                if let Some(dir) = ask["dump"].as_str() {
                    let _ = std::fs::write(Path::new(dir).join(format!("{k}.raw")), &frame.px);
                }
                frames.push(json!([k, frame.w, frame.h, sha1_hex(&frame.px)]));
                Ok(true)
            })
            .map(|fps| json!({"fps": fps, "frames": frames, "ms": started.elapsed().as_secs_f64() * 1000.0}))
        };
        println!("{}", answer.map_or_else(|e| json!({"error": e}), |v| json!({"ok": v})));
    }
}
