//! Pictures through the core, one line of numbers each: for comparing two builds or two devices,
//! and for timing.
//!
//!     frames <models folder> <people.npz or -> <out.jsonl> <list of files>
//!
//! The list names one picture a line: a .jpg, or a .bgr dump (width and height as two little-endian 32-bit
//! numbers, then the BGR bytes) when both sides should start from the very same pixels.

use std::{io::Write, path::Path, time::Instant};

use face_core::{
    faces::{Faces, expression_skip},
    gallery::Identifier,
    image::Bgr,
    runtime::{Choice, Runtime, find_library},
};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: frames <models folder> <people.npz or -> <out.jsonl> <list of files>".into());
    }
    let models = Path::new(&args[1]);
    let library = find_library(&models.join("..")).ok_or("no model runtime library (FACE_ORT_LIB)")?;
    let faces = Faces::load(&Runtime::new(&library, Choice::from_env())?, models)?;
    let ident = if args[2] == "-" { None } else { Identifier::load(Path::new(&args[2]), faces.rec_model.name)? };
    eprintln!("devices: {:?}; first runs: {:?}", faces.devices(), faces.warmup());
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3])?);
    for file in std::fs::read_to_string(&args[4])?.lines().filter(|l| !l.trim().is_empty()) {
        let bytes = std::fs::read(file)?;
        let t0 = Instant::now();
        let img = if file.ends_with(".bgr") {
            let (w, h) = (u32::from_le_bytes(bytes[0..4].try_into()?), u32::from_le_bytes(bytes[4..8].try_into()?));
            Bgr::from_px(w as usize, h as usize, bytes[8..].to_vec())?
        } else {
            Bgr::from_jpeg(&bytes)?
        };
        let t1 = Instant::now();
        let found = faces.detect(&img, &[640])?;
        let t2 = Instant::now();
        let mut rows = Vec::new();
        for f in &found {
            let skip = expression_skip(&f.row, f.quality);
            let expr = if skip.is_none() { faces.expression(&f.crop)? } else { None };
            let (name, sim) = match &ident {
                Some(i) => i.who(&f.emb, 0.35),
                None => ("Unknown".to_string(), 0.0),
            };
            rows.push(json!({
                "row": f.row.to_vec(), "emb": f.emb, "quality": f.quality, "skip": skip,
                "expr": expr.map(|e| e.to_vec()), "name": name, "sim": sim, "crop": f.crop.px.iter().map(|&b| b as u32).sum::<u32>(),
            }));
        }
        let t3 = Instant::now();
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
        writeln!(
            out,
            "{}",
            json!({"file": file, "w": img.w, "h": img.h, "decode_ms": ms(t0, t1), "detect_ms": ms(t1, t2),
                                    "expr_ms": ms(t2, t3), "faces": rows})
        )?;
    }
    Ok(())
}
