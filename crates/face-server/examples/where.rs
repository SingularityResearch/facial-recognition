//! Asks this system's own location service once, as the app does while its owner has switched the location
//! on, and says how that went: for trying the service on each kind of system.
//!
//!     cargo run -p face-server --example where            (whether it answered, and how sure)
//!     cargo run -p face-server --example where -- --show  (also where it says the device is)

fn main() {
    let show = std::env::args().any(|a| a == "--show");
    println!("{}", face_server::location::try_system(show));
}
