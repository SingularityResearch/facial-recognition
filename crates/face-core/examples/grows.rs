//! Does the model runtime keep memory for every new picture it is given?
//!
//!     grows <models folder> [runs]
//!
//! ONNX Runtime 1.24.4 did on Apple's M4 and later (its convolution for Arm's SME2, the KleidiAI code): a table
//! for each picture, by the picture's content, never dropped. With one picture run again and again nothing grew,
//! which is how every test passed; a camera never sends the same picture twice, and the tracker's process on
//! an M5 passed 200 GB in twenty minutes (2026-11-22). 1.31.1 keeps nothing, and a Mac's runtime is pinned to
//! it (`face-tools` `parts.rs`).
//!
//! So before a runtime's pin is changed: this, on such a Mac, with `FACE_ORT_LIB` naming the new library.
//! The face-finding model is run on the processor with a new picture of noise each time, always from the same
//! buffer, and the process's size is printed every hundred runs. It must stand still after the first hundred;
//! 1.24.4 added 635 MB with every hundred.

use std::path::Path;

use face_core::runtime::{Choice, Runtime, find_library};

/// This process's resident size in MB, as `ps` says it (nothing on a system without it).
fn size_mb() -> String {
    let asked = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
    match asked.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()) {
        Some(kb) => format!("{:.0} MB", kb / 1024.0),
        None => "size unknown".to_string(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        return Err("usage: grows <models folder> [runs]".into());
    }
    let models = Path::new(&args[1]);
    let runs: usize = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(500);
    let library = find_library(&models.join("..")).ok_or("no model runtime library (FACE_ORT_LIB)")?;
    let rt = Runtime::new(&library, Choice::Cpu)?;
    let det = rt.model_on_cpu(&models.join("face_detection_yunet_2023mar_dynamic.onnx"), "Detection")?;
    let (shape, mut seed) = ([1usize, 3, 576, 640], 0x9E37_79B9u32);
    let mut picture = vec![0f32; 3 * 576 * 640];
    for run in 0..=runs {
        for v in picture.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *v = (seed >> 24) as f32;
        }
        det.run(&shape, &picture)?;
        if run % 100 == 0 {
            println!("after {run} new pictures: {}", size_mb());
        }
    }
    Ok(())
}
