//! Does the detector find the same faces whether a picture is padded to 320 or only to the model's own 32?
//! And how long does each take?
//!
//!     pads <models folder> <list of .jpg files>
//!
//! Prints numbers only. See `Faces::pad` for what this found and what follows from it.

use std::{path::Path, time::Instant};

use face_core::{
    faces::{DET_SCORE, Faces},
    geometry::iou,
    image::Bgr,
    runtime::{Choice, Runtime, find_library},
    yunet::{DET_PAD, MODEL_PAD},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: pads <models folder> <list of .jpg files>".into());
    }
    let models = Path::new(&args[1]);
    let library = find_library(&models.join("..")).ok_or("no model runtime library (FACE_ORT_LIB)")?;
    let faces = Faces::load(&Runtime::new(&library, Choice::from_env())?, models)?;
    faces.warmup();
    let (mut pictures, mut same_count, mut compared) = (0usize, 0usize, 0usize);
    let (mut worst, mut score_worst, mut offs) = (0f32, 0f32, Vec::new());
    let (mut ms_wide, mut ms_tight) = (Vec::new(), Vec::new());
    for file in std::fs::read_to_string(&args[2])?.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(img) = Bgr::from_jpeg(&std::fs::read(file)?) else { continue };
        pictures += 1;
        let timed = |pad: usize, ms: &mut Vec<f64>| {
            let _ = faces.detect_at(&img, 640, DET_SCORE, pad); // a size's first run sets its session up
            let t = Instant::now();
            let rows = faces.detect_at(&img, 640, DET_SCORE, pad);
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
            rows
        };
        let (wide, tight) = (timed(DET_PAD, &mut ms_wide)?, timed(MODEL_PAD, &mut ms_tight)?);
        if wide.len() == tight.len() {
            same_count += 1;
        }
        // each face with the one it overlaps most in the other pass (two faces of almost one score can swap places)
        for a in &wide {
            let Some(b) = tight.iter().max_by(|x, y| iou(a, &x[..]).total_cmp(&iou(a, &y[..]))) else { continue };
            compared += 1;
            let off = a[..14].iter().zip(&b[..14]).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
            offs.push(off / a[2].max(1.0));
            worst = worst.max(off);
            score_worst = score_worst.max((a[14] - b[14]).abs());
        }
    }
    offs.sort_by(|a, b| a.total_cmp(b));
    let share = |limit: f32| offs.iter().filter(|o| **o > limit).count();
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    println!("devices: {:?}", faces.devices().first());
    println!("{pictures} pictures: the same number of faces in {same_count}; {compared} faces compared");
    println!("  boxes and points apart: {worst:.4} px at most; scores apart: {score_worst:.5} at most");
    println!(
        "  as a share of the face's width: median {:.5}; over 1%: {} faces; over 5%: {} faces",
        offs.get(offs.len() / 2).copied().unwrap_or(0.0),
        share(0.01),
        share(0.05)
    );
    println!(
        "  finding faces: {:.2} ms padded to {DET_PAD}, {:.2} ms padded to {MODEL_PAD}",
        median(&mut ms_wide),
        median(&mut ms_tight)
    );
    Ok(())
}
