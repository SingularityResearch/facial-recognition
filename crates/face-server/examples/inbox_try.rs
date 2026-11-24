//! The alert inbox and the presence log on their own: things done to them in a folder, each answered with
//! a line of JSON, for a check that compares the answers and the files:
//!
//!     inbox_try <a private folder>      (one JSON object a line on standard input)
//!
//! {"op": "add" | "details" | "state" | "list" | "visit" | "visits" | "readings" | "capture" | "report" | "posed", ...}; one line of JSON is printed for
//! each: {"ok": <the answer>} or {"error": <why>}, with "ms", how long it took (a lock held elsewhere shows).

use std::{io::BufRead, path::PathBuf, time::Instant};

use face_server::{alerts::Inbox, capture::Capture, exprlog::ExprLog, presence::Presence, recorder};
use serde_json::{Value, json};

fn main() {
    let home = PathBuf::from(std::env::args().nth(1).expect("a folder"));
    let (inbox, presence, readings) = (Inbox::new(&home), Presence::new(&home), ExprLog::new(&home));
    let capture = Capture::new(&home);
    // test data is the library's, not the private folder's: `<folder>/root/data/expr_eval`
    let tests = recorder::Recorder::new(&home.join("root"));
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(ask) = serde_json::from_str::<Value>(&line) else { continue };
        let t = Instant::now();
        let answer: Result<Value, String> = match ask["op"].as_str().unwrap_or("") {
            "add" => inbox.add(&ask["alert"]),
            "details" => inbox.add_details(ask.get("id"), ask.get("which"), ask.get("details")).map(|b| json!(b)),
            "state" => inbox.set_state(ask.get("ids"), ask.get("state"), ask.get("where")),
            "list" => inbox.listing(),
            "visit" => presence.log_visit(&ask["visit"]).map(|_| json!(true)),
            "visits" => presence.visits(ask["day"].as_str().unwrap_or("")).map(|v| json!(v)),
            "readings" => {
                let gone =
                    ask["forgotten"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(str::to_string));
                let now = ask["now"].as_i64().unwrap_or(0);
                readings.add(ask.get("readings"), ask["enabled"] == true, &gone.collect(), now).map(|n| json!(n))
            }
            // posed tests and feeling reports
            "report" => recorder::check_report(ask["body"].as_str().unwrap_or("").as_bytes())
                .and_then(|rep| tests.report(&rep))
                .map(|_| json!(tests.reports())),
            "posed" => {
                let none = serde_json::Map::new();
                let (session, label) = (ask["session"].as_str().unwrap_or(""), ask["posed"].as_str().unwrap_or(""));
                tests
                    .add(session, label, ask["extra"].as_object().unwrap_or(&none), &ask["face"])
                    .map(|_| json!(tests.sessions()))
            }
            // a frame from a file: Collect faces
            "capture" => std::fs::read(ask["jpeg"].as_str().unwrap_or(""))
                .map_err(|e| e.to_string())
                .and_then(|jpeg| capture.save(&jpeg, &ask["faces"], &ask["others"]))
                .map(|stems| json!(stems)),
            // a picture's raw bytes (blue, green, red a pixel) enlarged with the bicubic filter, into another file
            "cubic" => std::fs::read(ask["raw"].as_str().unwrap_or("")).map_err(|e| e.to_string()).and_then(|px| {
                let (w, h) = (ask["w"].as_u64().unwrap_or(0) as usize, ask["h"].as_u64().unwrap_or(0) as usize);
                let big = face_core::image::Bgr::from_px(w, h, px)
                    .map_err(|e| e.0)?
                    .resize_cubic(ask["fx"].as_f64().unwrap_or(1.0), ask["fy"].as_f64().unwrap_or(1.0));
                std::fs::write(ask["out"].as_str().unwrap_or(""), &big.px).map_err(|e| e.to_string())?;
                Ok(json!([big.w, big.h]))
            }),
            // a JPEG read into raw bytes, and raw bytes written as a JPEG: each half of what a saved picture
            // goes through, by itself
            "decode" => std::fs::read(ask["jpeg"].as_str().unwrap_or("")).map_err(|e| e.to_string()).and_then(|jpeg| {
                let img = face_core::image::Bgr::from_jpeg(&jpeg).map_err(|e| e.0)?;
                std::fs::write(ask["out"].as_str().unwrap_or(""), &img.px).map_err(|e| e.to_string())?;
                Ok(json!([img.w, img.h]))
            }),
            "encode" => std::fs::read(ask["raw"].as_str().unwrap_or("")).map_err(|e| e.to_string()).and_then(|px| {
                let (w, h) = (ask["w"].as_u64().unwrap_or(0) as usize, ask["h"].as_u64().unwrap_or(0) as usize);
                let jpeg = face_core::image::Bgr::from_px(w, h, px)
                    .and_then(|img| img.to_jpeg(ask["quality"].as_u64().unwrap_or(92) as u8))
                    .map_err(|e| e.0)?;
                std::fs::write(ask["out"].as_str().unwrap_or(""), &jpeg).map_err(|e| e.to_string())?;
                Ok(json!(jpeg.len()))
            }),
            other => Err(format!("unknown op {other}")),
        };
        let ms = t.elapsed().as_millis() as u64;
        println!(
            "{}",
            match answer {
                Ok(v) => json!({"ok": v, "ms": ms}),
                Err(e) => json!({"error": e, "ms": ms}),
            }
        );
    }
}
