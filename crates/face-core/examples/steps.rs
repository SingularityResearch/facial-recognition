//! How long each step of reading one frame takes (median of many runs), on the device `FACE_DEVICE` names:
//!
//!     steps <models folder> <frame.jpg> [runs]
//!
//! For deciding where each model should run: a small model can be quicker on the processor than on a
//! graphics processor that has to be waited for.

use std::{path::Path, time::Instant};

use face_core::{
    faces::{DET_SCORE, Faces, align},
    image::Bgr,
    runtime::{Choice, Runtime, find_library},
};

fn median_ms(runs: usize, mut step: impl FnMut()) -> f64 {
    for _ in 0..5 {
        step();
    }
    let mut ms: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            step();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    ms.sort_by(|a, b| a.total_cmp(b));
    ms[ms.len() / 2]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Without arguments (as an iPhone app's own program, which is started without any): the models and the
    // frame beside the program.
    let args: Vec<String> = std::env::args().collect();
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let (models, frame) = match (args.get(1), args.get(2)) {
        (Some(m), Some(f)) => (Path::new(m).to_path_buf(), Path::new(f).to_path_buf()),
        _ if beside.join("models").is_dir() => (beside.join("models"), beside.join("frame.jpg")),
        _ => return Err("usage: steps <models folder> <frame.jpg> [runs]".into()),
    };
    let models = models.as_path();
    let runs = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(60);
    let library = find_library(&models.join("..")).ok_or("no model runtime library (FACE_ORT_LIB)")?;
    let runtime = Runtime::new(&library, Choice::from_env())?;
    let faces = Faces::load(&runtime, models)?;
    let voice = face_core::voice::Voice::load(&runtime, models);
    faces.warmup();
    let jpeg = std::fs::read(&frame)?;
    let img = Bgr::from_jpeg(&jpeg)?;
    let pad = faces.pad();
    let rows = faces.detect_at(&img, 640, DET_SCORE, pad)?;
    // A frame without a face (a made-up one, on a device no photo should be copied to) still times every step:
    // a model takes as long for any picture. The face is then a made-up one in the middle of the frame.
    let stand_in: face_core::yunet::Row = {
        let (cx, cy, s) = (img.w as f32 / 2.0, img.h as f32 / 2.0, 2.0f32);
        let mut r = [0f32; 15];
        (r[0], r[1], r[2], r[3], r[14]) = (cx - 56.0 * s, cy - 56.0 * s, 112.0 * s, 112.0 * s, 0.9);
        for (i, p) in face_core::geometry::ARC_TEMPLATE.iter().enumerate() {
            r[4 + 2 * i] = cx + (p[0] - 56.0) * s;
            r[5 + 2 * i] = cy + (p[1] - 56.0) * s;
        }
        r
    };
    let row = rows.first().unwrap_or(&stand_in);
    let crop = align(&img, row)?;
    println!("{}x{}, {} face(s); devices: {:?}", img.w, img.h, rows.len(), faces.devices());
    println!("  decode the JPEG      {:6.2} ms", median_ms(runs, || drop(Bgr::from_jpeg(&jpeg))));
    println!("  shrink to 640        {:6.2} ms", median_ms(runs, || drop(img.resize_area(640, 360))));
    println!("  find faces           {:6.2} ms", median_ms(runs, || drop(faces.detect_at(&img, 640, DET_SCORE, pad))));
    println!("  align one face       {:6.2} ms", median_ms(runs, || drop(align(&img, row))));
    println!("  print                {:6.2} ms", median_ms(runs, || drop(faces.embed(&crop))));
    println!("  quality              {:6.2} ms", median_ms(runs, || drop(faces.quality(&crop))));
    println!("  expression (2 runs)  {:6.2} ms", median_ms(runs, || drop(faces.expression(&crop))));
    // four seconds of a sound with a voice's pitch and a little noise (nobody's voice)
    let samples: Vec<f32> = (0..4 * face_core::voice::RATE)
        .map(|k| {
            let t = k as f32 / face_core::voice::RATE as f32;
            0.3 * (2.0 * std::f32::consts::PI * 180.0 * t).sin()
                * (1.0 + 0.5 * (2.0 * std::f32::consts::PI * 3.0 * t).sin())
                + 0.02 * ((k * 7919 % 1000) as f32 / 500.0 - 1.0)
        })
        .collect();
    if voice.loaded() {
        println!("  voice, 4 s (Melos 1) {:6.2} ms", median_ms(runs, || drop(voice.avd(&samples))));
    }
    println!(
        "  the whole frame      {:6.2} ms",
        median_ms(runs, || {
            if let Ok(found) = faces.detect(&img, &[640]) {
                for f in &found {
                    let _ = faces.expression(&f.crop);
                }
            }
        })
    );
    Ok(())
}
