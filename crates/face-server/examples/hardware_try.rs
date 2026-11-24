//! This computer's own network addresses as the programs find them,
//! as one line of JSON (the real adapters', every adapter's, and those other machines can reach).
//!
//!     hardware_try <private folder>

use std::path::PathBuf;

use face_server::hardware::{own_addresses, reachable_addresses};

fn main() {
    let home = PathBuf::from(std::env::args().nth(1).unwrap_or_default());
    let text = |ips: Vec<std::net::Ipv4Addr>| ips.iter().map(ToString::to_string).collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::json!({
            "real": text(own_addresses(&home, Some(false))),
            "every": text(own_addresses(&home, Some(true))),
            "set": text(own_addresses(&home, None)),
            "reachable": text(reachable_addresses(&home, Some(true))),
        })
    );
}
