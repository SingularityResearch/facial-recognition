//! The peer service on its own, with a made-up camera, for a check that asks it as another device
//! would:
//!
//!     peer_try <an empty folder> <a password> <a database id>
//!
//! It prints the port and the certificate's SHA-256, then hands the service a new "frame" twenty times a
//! second and a readout every second, until it is stopped. No database, no camera, no model.

use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use face_server::{camstream::Stream, peer};
use serde_json::json;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(home), Some(password), Some(instance)) = (args.next(), args.next(), args.next()) else {
        eprintln!("peer_try <folder> <password> <database id>");
        std::process::exit(2);
    };
    let stream = Stream::new();
    let key: peer::Key = Arc::new(RwLock::new(face_db::peer::derive_key(&password, &instance)));
    let service = peer::start(
        &PathBuf::from(home),
        "a test device",
        stream.clone(),
        key,
        None,
        peer::Gives::default(),
        std::sync::Arc::new(face_server::bus::Bus::new()),
    )
    .expect("the service");
    println!("{} {}", service.port, service.cert_sha256);
    for n in 0u32.. {
        // what a frame is to the service: bytes that begin as a JPEG does
        let mut frame = vec![0xff, 0xd8, 0xff, 0xe0];
        frame.extend_from_slice(format!("frame {n}").as_bytes());
        frame.extend_from_slice(&[0xff, 0xd9]);
        stream.put(&frame).expect("a frame");
        if n % 20 == 0 {
            stream.set_readout(&json!([{"name": "Someone", "n": n}]), None, None).expect("a readout");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
