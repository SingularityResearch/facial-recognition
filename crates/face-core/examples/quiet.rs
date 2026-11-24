//! Does the model runtime keep or send anything of its own?
//!
//!     HOME=<an empty folder made for this> quiet <models folder>
//!
//! ONNX Runtime from 1.30 on carries Microsoft's telemetry client on every system but Windows, switched on
//! unless told otherwise. At its start it then makes a device id and a store for its events in the user's
//! folders (on a Mac: `Library/Application Support/Microsoft/DeveloperTools/.onnxruntime`) and sends
//! Microsoft what it runs on. The programs switch it off before the runtime is loaded
//! (`runtime::no_telemetry`), and this says whether that still holds with the library it is given.
//!
//! So before a runtime's pin is changed: this, with `FACE_ORT_LIB` naming the new library, the user's folder
//! (`HOME`; on Windows `USERPROFILE`, `APPDATA` and `LOCALAPPDATA` as well) at an empty folder made for it,
//! and the network closed where the system can do that for one program, so that nothing leaves even if the
//! switch no longer works (macOS: `sandbox-exec -p '(version 1)(allow default)(deny network*)' env HOME=...`).
//! It loads the runtime as the programs do, runs the face-finding model once, and lists what is in that
//! folder afterwards. There must be nothing: 1.31.1 with its telemetry left on made four files there.
//! And look through the new library for addresses it didn't have before (`strings`, `grep -a http`).

use std::path::{Path, PathBuf};

use face_core::runtime::{Choice, NO_TELEMETRY, Runtime, find_library};

fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, found);
        } else {
            found.push(path);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        return Err("usage: HOME=<an empty folder> quiet <models folder>".into());
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let home = home.ok_or("name the user's folder for this run (HOME): an empty folder made for it")?;
    let mut before = Vec::new();
    files_under(&home, &mut before);
    if !before.is_empty() {
        return Err(format!("{} is not empty: name a folder made for this run", home.display()).into());
    }
    let models = Path::new(&args[1]);
    let library = find_library(&models.join("..")).ok_or("no model runtime library (FACE_ORT_LIB)")?;
    let rt = Runtime::new(&library, Choice::Cpu)?;
    let det = rt.model_on_cpu(&models.join("face_detection_yunet_2023mar_dynamic.onnx"), "Detection")?;
    det.run(&[1, 3, 576, 640], &vec![0f32; 3 * 576 * 640])?;
    drop(det);
    // (what a client that works behind the runs would write, it has written by now)
    std::thread::sleep(std::time::Duration::from_secs(3));
    let mut made = Vec::new();
    files_under(&home, &mut made);
    println!("{NO_TELEMETRY} in this process: {:?}", std::env::var(NO_TELEMETRY).unwrap_or_default());
    if made.is_empty() {
        println!("quiet: the runtime made nothing in the user's folder");
        return Ok(());
    }
    for file in &made {
        println!("made: {}", file.strip_prefix(&home).unwrap_or(file).display());
    }
    Err(format!("the runtime made {} file(s) of its own: its telemetry is not off", made.len()).into())
}
