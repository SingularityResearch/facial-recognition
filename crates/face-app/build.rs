//! The iPhone's model runtime (ONNX Runtime 1.31.1's static library) wants system parts the one before it
//! didn't: SQLite, zlib, the network-path monitor and UIKit. All of them are for the telemetry client that is
//! built into it. The app switches that client off before the runtime is loaded
//! (`face_core::runtime::no_telemetry`), and nothing of it is then set up; the linker still has to find what
//! its code names. The app's own link line (`apps/ios/project.sh`) names the same.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        for library in ["sqlite3", "z"] {
            println!("cargo:rustc-link-lib={library}");
        }
        for framework in ["Network", "UIKit"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
