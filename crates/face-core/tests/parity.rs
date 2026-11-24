//! The core against fixed numbers: every step here must give what OpenCV gave for the
//! same made-up input (`tests/vectors/*.json`). A face cut out a
//! little differently is a different print, and prints are shared between devices and compared with measured
//! thresholds, so "close" is not the bar for the picture steps: the same bytes are.
//!
//! The model tests need the model files and the model runtime's library (`FACE_ORT_LIB`); without them they
//! say so and pass.

use std::path::PathBuf;

use face_core::{
    faces::Faces,
    geometry::estimate_similarity,
    image::Bgr,
    runtime::{Choice, Runtime},
    yunet,
};
use serde_json::Value;

/// The numbers to hold the core to. `FACE_VECTORS` names their folder where this test runs on another device
/// than it was built on (a phone, a Raspberry Pi): the folder it was built beside isn't there.
fn vectors(name: &str) -> Value {
    vectors_dir_checked(name)
}

/// A folder beside this program, when there is one: on an iPhone the test is an app's own program, and what
/// it reads (the expected answers, the models) is in the app beside it.
fn beside_program(name: &str) -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join(name);
    p.is_dir().then_some(p)
}

fn vectors_dir_checked(name: &str) -> Value {
    let dir = std::env::var_os("FACE_VECTORS")
        .map(PathBuf::from)
        .or_else(|| beside_program("vectors"))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("vectors"));
    let p = dir.join(name);
    serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

fn unpacked(text: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' => 62,
            _ => 63,
        }
    };
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let n = chunk.iter().fold(0u32, |a, &c| (a << 6) | val(c)) << (6 * (4 - chunk.len()));
        let b = n.to_be_bytes();
        out.extend_from_slice(&b[1..chunk.len()]);
    }
    out
}

/// The made-up picture the vectors were made from, byte for byte.
fn picture(seed: u64, w: usize, h: usize) -> Bgr {
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let base = [x * 255 / (w - 1).max(1), y * 255 / (h - 1).max(1), (x + y) * 255 / (w + h - 2).max(1)];
            for (c, b) in base.iter().enumerate() {
                let i = ((y * w + x) * 3 + c) as u64;
                let mut v = ((i + seed) as u32).wrapping_mul(0x9E37_79B1);
                v ^= v >> 15;
                v = v.wrapping_mul(0x85EB_CA77);
                v ^= v >> 13;
                v = v.wrapping_mul(0xC2B2_AE3D);
                v ^= v >> 16;
                px.push((*b as i64 + (v % 121) as i64 - 60).clamp(0, 255) as u8);
            }
        }
    }
    Bgr::from_px(w, h, px).unwrap()
}

fn num(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}

fn differ(a: &[u8], b: &[u8]) -> (usize, i32) {
    assert_eq!(a.len(), b.len(), "sizes differ");
    let diffs = a.iter().zip(b).filter(|(x, y)| x != y).count();
    let most = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap_or(0);
    (diffs, most)
}

#[test]
fn shrinking_a_picture_gives_opencvs_bytes() {
    for c in vectors("resize.json").as_array().unwrap() {
        let (w, h, dw, dh) = (num(&c["w"]), num(&c["h"]), num(&c["dw"]), num(&c["dh"]));
        let got = picture(c["seed"].as_u64().unwrap(), w, h).resize_area(dw, dh);
        let want = unpacked(c["dst"].as_str().unwrap());
        let (diffs, most) = differ(&got.px, &want);
        assert_eq!(diffs, 0, "{w}x{h} to {dw}x{dh}: {diffs} bytes differ, by up to {most}");
    }
}

#[test]
fn a_faces_points_give_opencvs_matrix() {
    let cases = vectors("estimate.json");
    let mut worst = 0f64;
    for c in cases.as_array().unwrap() {
        let p: Vec<f32> = c["pts"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let pts: Vec<[f32; 2]> = p.chunks(2).map(|q| [q[0], q[1]]).collect();
        let got = estimate_similarity(&pts, &face_core::geometry::ARC_TEMPLATE);
        match c["m"].as_array() {
            None => assert!(got.is_none()),
            Some(want) => {
                let got = got.expect("a matrix");
                for (g, w) in got.iter().zip(want) {
                    let w = w.as_f64().unwrap();
                    worst = worst.max((g - w).abs() / (1.0 + w.abs()));
                }
            }
        }
    }
    // not 1e-12: OpenCV's refinement stops short of the least-squares answer by its own rounding noise (see
    // geometry.rs). A different choice of the points that count would be off by a hundred times this bar.
    assert!(worst < 1e-4, "the matrix is off by up to {worst:e}");
}

#[test]
fn cutting_a_face_out_gives_opencvs_bytes() {
    for c in vectors("warp.json").as_array().unwrap() {
        let (w, h) = (num(&c["w"]), num(&c["h"]));
        let m: Vec<f64> = c["m"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let got = picture(c["seed"].as_u64().unwrap(), w, h).warp_affine(&m.try_into().unwrap(), 112, 112);
        let want = unpacked(c["dst"].as_str().unwrap());
        let (diffs, most) = differ(&got.px, &want);
        assert_eq!(diffs, 0, "{w}x{h}: {diffs} of {} bytes differ, by up to {most}", want.len());
    }
}

#[test]
fn the_detectors_outputs_give_the_programs_faces() {
    for c in vectors("yunet.json").as_array().unwrap() {
        let (pw, ph) = (num(&c["pw"]), num(&c["ph"]));
        let mut out: Vec<Vec<f32>> = Vec::new();
        for width in [1usize, 1, 4, 10] {
            for stride in [8usize, 16, 32] {
                out.push(vec![0f32; (pw / stride) * (ph / stride) * width]);
            }
        }
        for (k, cells) in c["cells"].as_array().unwrap().iter().enumerate() {
            for cell in cells.as_array().unwrap() {
                let v: Vec<f64> = cell.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
                let i = v[0] as usize;
                out[k][i] = v[1] as f32;
                out[3 + k][i] = v[2] as f32;
                for j in 0..4 {
                    out[6 + k][i * 4 + j] = v[3 + j] as f32;
                }
                for j in 0..10 {
                    out[9 + k][i * 10 + j] = v[7 + j] as f32;
                }
            }
        }
        let got = yunet::rows(&out, pw, c["score"].as_f64().unwrap(), 0.3, 5000);
        let want = c["rows"].as_array().unwrap();
        assert_eq!(got.len(), want.len(), "{pw}x{ph}: a different number of faces");
        for (g, w) in got.iter().zip(want) {
            for (a, b) in g.iter().zip(w.as_array().unwrap()) {
                let b = b.as_f64().unwrap();
                assert!((*a as f64 - b).abs() <= 1e-4 * (1.0 + b.abs()), "{pw}x{ph}: {a} against {b}");
            }
        }
    }
}

#[test]
fn a_jpeg_decodes_as_opencv_decodes_it() {
    for c in vectors("jpeg.json").as_array().unwrap() {
        let got = Bgr::from_jpeg(&unpacked(c["jpeg"].as_str().unwrap())).unwrap();
        assert_eq!((got.w, got.h), (num(&c["w"]), num(&c["h"])));
        let want = unpacked(c["bgr"].as_str().unwrap());
        let (_, most) = differ(&got.px, &want);
        let mean =
            got.px.iter().zip(&want).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / want.len() as f64;
        // two decoders round the same JPEG's arithmetic differently: a level here and there, never a picture
        assert!(mean < 0.6 && most <= 6, "decoded {mean:.3} levels apart on average, {most} at most");
    }
}

/// (the models' folder, the runtime's library) when both are on this computer.
fn models() -> Option<(PathBuf, PathBuf)> {
    let dir = std::env::var_os("FACE_MODELS")
        .map(PathBuf::from)
        .or_else(|| beside_program("models"))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/root/models"));
    let lib = face_core::runtime::find_library(&dir.join(".."));
    match lib {
        Some(lib) if dir.join(face_core::faces::REC_MODELS[0].file).is_file() => Some((dir, lib)),
        _ => {
            eprintln!("skipped: no model files or no model runtime (FACE_ORT_LIB) on this computer");
            None
        }
    }
}

fn cosine(a: &[f32], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * y).sum();
    let (na, nb) =
        (a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt(), b.iter().map(|y| y * y).sum::<f64>().sqrt());
    dot / (na * nb)
}

fn models_agree(choice: Choice, tolerance: f64) {
    let Some((dir, lib)) = models() else { return };
    let v = vectors("models.json");
    let faces = Faces::load(&Runtime::new(&lib, choice).unwrap(), &dir).unwrap();
    assert_eq!(v["rec"].as_str().unwrap(), faces.rec_model.name);
    for c in v["crops"].as_array().unwrap() {
        let crop = picture(c["seed"].as_u64().unwrap(), 112, 112);
        let want: Vec<f64> = c["emb"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let cos = cosine(&faces.embed(&crop).unwrap(), &want);
        assert!(1.0 - cos < tolerance * tolerance, "a print {cos} from the programs' ({:?})", faces.devices());
        let q = faces.quality(&crop).unwrap() as f64;
        assert!((q - c["quality"].as_f64().unwrap()).abs() < tolerance.max(1e-4), "quality {q}");
        let e = faces.expression(&crop).unwrap().unwrap();
        for (g, w) in e.iter().zip(c["expr"].as_array().unwrap()) {
            assert!((g - w.as_f64().unwrap()).abs() < tolerance.max(1e-4), "expression {g} against {w}");
        }
    }
}

#[test]
fn the_models_answer_as_in_the_programs_on_the_processor() {
    models_agree(Choice::Cpu, 1e-3);
}

/// For a new recognition model: its prints for the test's pictures, as the processor gives them on the computer
/// this is run on, written into `models.json` in place of the model's before (quality and expression stay: their
/// models are the same). The tests above then hold every system and every faster path to these.
///
///     cargo test -p face-core --release --test parity -- --ignored the_recognizers_prints_are_written
#[test]
#[ignore = "writes tests/vectors/models.json: only for a new recognition model"]
fn the_recognizers_prints_are_written() {
    let Some((dir, lib)) = models() else { return };
    let faces = Faces::load(&Runtime::new(&lib, Choice::Cpu).unwrap(), &dir).unwrap();
    let mut v = vectors("models.json");
    v["rec"] = serde_json::json!(faces.rec_model.name);
    for c in v["crops"].as_array_mut().unwrap() {
        let crop = picture(c["seed"].as_u64().unwrap(), 112, 112);
        c["emb"] = serde_json::json!(faces.embed(&crop).unwrap());
    }
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("vectors").join("models.json");
    std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
}

#[test]
fn the_models_answer_the_same_on_the_graphics_processor() {
    models_agree(Choice::Auto, 0.01); // GPU_TOLERANCE
}
