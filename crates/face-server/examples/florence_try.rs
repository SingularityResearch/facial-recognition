//! What the built-in describe model says of a picture, as JSON, for a check of it.
//!
//!     florence_try status <root>                 whether the model's files are in <root>/models/florence2
//!     florence_try get <root>                    the files that are missing fetched, as the labeling tool's
//!                                                route starts it; the status when it has ended, and how
//!                                                often the status was seen with the download running
//!     florence_try tokens <root> <text>...       each text cut into pieces and tokens, and read back
//!     florence_try describe <root> <jpeg>...     each JPEG file described: one line of JSON each
//!     florence_try seen <root> -                 JPEGs on standard input as the model is given them, in
//!                                                numbers (the picture at the model's size, its features)
//!     florence_try describe <root> -             the same for JPEGs on standard input, each after its
//!                                                length as four bytes (low byte first): no picture
//!                                                is written anywhere
//!
//! Nothing is written but the list of the files already checked, beside the model's files.

use std::path::Path;

use face_server::florence;
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match &words[..] {
        ["status", root] => {
            florence::set_root(Path::new(root));
            println!("{}", florence::status());
        }
        ["get", root] => {
            florence::set_root(Path::new(root));
            florence::download_in_background();
            let mut running = 0;
            loop {
                let status = florence::status();
                if status["running"] != true {
                    println!("{}", json!({"status": status, "seen_running": running}));
                    break;
                }
                running += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        ["tokens", root, texts @ ..] => {
            florence::set_root(Path::new(root));
            for text in texts {
                println!("{}", florence::tokens_of(text).unwrap_or_else(|why| json!({"error": why})));
            }
        }
        [what @ ("describe" | "seen"), root, "-"] => {
            use std::io::Read;
            florence::set_root(Path::new(root));
            let mut input = std::io::stdin().lock();
            let mut length = [0u8; 4];
            while input.read_exact(&mut length).is_ok() {
                let mut jpeg = vec![0u8; u32::from_le_bytes(length) as usize];
                if input.read_exact(&mut jpeg).is_err() {
                    break;
                }
                let said = if *what == "seen" { florence::seen_of(&jpeg) } else { florence::describe(&jpeg) };
                println!("{}", said.unwrap_or_else(|why| json!({"error": why})));
            }
        }
        ["describe", root, files @ ..] => {
            florence::set_root(Path::new(root));
            for file in files {
                let said = std::fs::read(file).map_err(|e| e.to_string()).and_then(|jpeg| florence::describe(&jpeg));
                println!("{}", said.unwrap_or_else(|why| json!({"error": why})));
            }
        }
        _ => {
            eprintln!("florence_try status <root> | get <root> | tokens <root> <text>... | describe <root> <jpeg>...");
            std::process::exit(2);
        }
    }
}
