//! A stand-in for LM Studio's `lms` command, for the checks of LM Studio's upkeep: put as
//! `<a made-up home>/.lmstudio/bin/lms` it answers the two things the programs ask of `lms`, from files of that
//! made-up `.lmstudio` folder, and downloads nothing.
//!
//!     lms ls --json            the models of ls.json, and one for every model folder under hub/models that
//!                              ls.json doesn't name (its variant from variant.txt there): what LM Studio lists
//!                              once a model's files have arrived
//!     lms get <variant> --yes  progress as `lms get` prints it; then the model "is there" (its hub folder is
//!                              made), unless get-fails.txt names the variant: then an error and exit code 1
//!
//!     lms server stop | start ...   nothing but the note of the call; `start` with `--bind` ends with exit
//!                              code 1 when the file bind-fails is there (a server that can't be started on
//!                              all interfaces)
//!
//! Every call's words are added to calls.jsonl.

use std::{io::Write, path::Path};

use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().expect("this program's own path");
    let root = exe.parent().and_then(Path::parent).expect("<home>/.lmstudio/bin/lms").to_path_buf();
    if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(root.join("calls.jsonl")) {
        let _ = writeln!(log, "{}", json!(args));
    }
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words[..] {
        ["ls", "--json"] => {
            if root.join("ls-fails").exists() {
                std::process::exit(1);
            }
            let listed: Value = std::fs::read(root.join("ls.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(json!([]));
            let mut models = listed.as_array().cloned().unwrap_or_default();
            let hub = root.join("hub").join("models");
            let folders = |of: &Path| {
                let mut found: Vec<_> = std::fs::read_dir(of)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect();
                found.sort();
                found
            };
            for owner in folders(&hub) {
                for model in folders(&owner) {
                    let name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let key = format!("{}/{}", name(&owner), name(&model));
                    if !models.iter().any(|m| m["modelKey"] == key.as_str()) {
                        let variant = std::fs::read_to_string(model.join("variant.txt"))
                            .map_or_else(|_| key.clone(), |v| v.trim().to_string());
                        models
                            .push(json!({"modelKey": key, "selectedVariant": variant, "vision": true, "sizeBytes": 0}));
                    }
                }
            }
            println!("{}", Value::Array(models));
        }
        ["get", variant, "--yes"] => {
            let out = std::io::stdout();
            let mut out = out.lock();
            let _ = write!(out, "Resolving {variant}\n\n");
            for step in 1..=4u32 {
                let _ = write!(out, "\rDownloading  {:.1}%  {:.1} MB / 10.0 MB", step as f64 * 25.0, step as f64 * 2.5);
                let _ = out.flush();
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            let _ = writeln!(out);
            let fails = std::fs::read_to_string(root.join("get-fails.txt")).unwrap_or_default();
            if fails.lines().any(|line| line.trim() == variant) {
                let _ = writeln!(out, "Verifying");
                let _ = out.flush();
                eprintln!("Error: the made-up download of this model failed");
                std::process::exit(1);
            }
            let key = variant.split('@').next().unwrap_or(variant);
            let folder = key.split('/').fold(root.join("hub").join("models"), |p, part| p.join(part));
            let _ = std::fs::create_dir_all(&folder);
            let _ = std::fs::write(folder.join("variant.txt"), variant);
            let _ = writeln!(out, "Download completed.");
        }
        ["server", "start", ..] if words.contains(&"--bind") && root.join("bind-fails").exists() => {
            std::process::exit(1)
        }
        _ => {}
    }
}
