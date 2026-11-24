//! Recognition models measured on a library's own labeled faces, which is how a recognizer's settings are
//! chosen:
//!
//!     rec_eval <a COPY of a data folder> <an empty private folder> <onnxruntime library> [--cap N] <model.onnx>...
//!
//! Every labeled face's picture (the aligned 112x112 crop the library keeps) is given to each model, and each
//! face is then looked up among the others (leave-one-out, with the app's own rule: a person's score is the mean
//! of their five closest prints): correct, unknown, wrong name, and "stranger named" (the same face looked up
//! with its own person taken out of the gallery: whoever is named then is a stranger to it). The same on
//! pictures made worse as a webcam makes them: blurred, tiny, moved and noisy. From it the match setting is
//! read: the lowest one that names no stranger and no wrong person on the clean pictures, plus 0.02.
//!
//! `--cap N` takes at most N faces of each person, spread evenly over theirs (a library where one person has
//! thousands of faces would otherwise measure that person). Prints numbers only: no name and no file.
//! Run it on a copy: the library is opened as the app opens it.
//!
//! For a model that was trained on a library's own faces (the owner's permission, 2026-11-22), people are split
//! by a number made from the name: `--fold K/N` measures only the people of that fold, and
//!
//!     rec_eval <a COPY of a data folder> <an empty private folder> --export <folder> --fold K/N
//!
//! writes everybody else's face pictures for the training, so that a model is never measured on a person it
//! learned. Those are pictures of real people: they stay on the owner's computers, and only a model's weights
//! leave them.

use std::{path::PathBuf, time::Instant};

use face_core::{
    faces::REC_MODELS,
    gallery::dot,
    image::Bgr,
    runtime::{Choice, Model, Probe, Runtime},
};
use face_library::labels::{IGNORE, LabelDb};

const Q_ENROLL: f32 = 0.30;
const TOP: usize = 5;

/// How a model takes a face: its colours' order, and its values as -1 to 1 (most recognizers' way) or as 0 to 255 (the
/// way of OpenCV's own recognizer, SFace).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Form {
    rgb: bool,
    raw: bool,
}
const FORMS: [Form; 4] = [
    Form { rgb: true, raw: false },
    Form { rgb: false, raw: false },
    Form { rgb: true, raw: true },
    Form { rgb: false, raw: true },
];

fn planes(crop: &Bgr, form: Form) -> Vec<f32> {
    let (w, h) = (crop.w, crop.h);
    let value = |v: u8| if form.raw { v as f32 } else { (v as f32 - 127.5) / 127.5 };
    let (first, last) = if form.rgb { (2, 0) } else { (0, 2) };
    let mut x = vec![0f32; 3 * w * h];
    for y in 0..h {
        let row = crop.row(y);
        for xx in 0..w {
            let at = y * w + xx;
            x[at] = value(row[xx * 3 + first]);
            x[w * h + at] = value(row[xx * 3 + 1]);
            x[2 * w * h + at] = value(row[xx * 3 + last]);
        }
    }
    x
}

fn embed(model: &Model, crop: &Bgr, form: Form) -> Vec<f32> {
    let mut v = model.run(&[1, 3, 112, 112], &planes(crop, form)).expect("the model runs").swap_remove(0);
    let norm = v.iter().map(|a| a * a).sum::<f32>().sqrt() + 1e-9;
    v.iter_mut().for_each(|a| *a /= norm);
    v
}

/// OpenCV's border for filters: mirrored without repeating the edge.
fn mirror(i: isize, n: usize) -> usize {
    let n = n as isize;
    let mut i = i;
    if i < 0 {
        i = -i;
    }
    if i >= n {
        i = 2 * (n - 1) - i;
    }
    i.clamp(0, n - 1) as usize
}

/// One row filter and one column filter over a picture (each a list of weights around its middle).
fn filter(img: &Bgr, across: &[f32], down: &[f32]) -> Vec<f32> {
    let (w, h) = (img.w, img.h);
    let mut mid = vec![0f32; w * h * 3];
    let half = (across.len() / 2) as isize;
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut s = 0.0;
                for (k, wt) in across.iter().enumerate() {
                    s += wt * img.px[(y * w + mirror(x as isize + k as isize - half, w)) * 3 + c] as f32;
                }
                mid[(y * w + x) * 3 + c] = s;
            }
        }
    }
    let mut out = vec![0f32; w * h * 3];
    let half = (down.len() / 2) as isize;
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut s = 0.0;
                for (k, wt) in down.iter().enumerate() {
                    s += wt * mid[(mirror(y as isize + k as isize - half, h) * w + x) * 3 + c];
                }
                out[(y * w + x) * 3 + c] = s;
            }
        }
    }
    out
}

fn to_picture(w: usize, h: usize, v: &[f32]) -> Bgr {
    Bgr::from_px(w, h, v.iter().map(|a| a.round().clamp(0.0, 255.0) as u8).collect()).expect("a picture")
}

/// A small generator of numbers for the noise: the same pictures at every run.
struct Noise(u64);
impl Noise {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    fn normal(&mut self) -> f64 {
        (-2.0 * self.unit().ln()).sqrt() * (std::f64::consts::TAU * self.unit()).cos()
    }
}

/// A picture as a poor webcam frame: "blur" (out of focus), "tiny" (a face 20 pixels wide, enlarged), "motion"
/// (moved sideways while the picture was taken, with noise).
fn worse(img: &Bgr, kind: &str, noise: &mut Noise) -> Bgr {
    match kind {
        "blur" => {
            let k: Vec<f32> = (-9..=9).map(|i| (-(i * i) as f32 / 18.0).exp()).collect();
            let sum: f32 = k.iter().sum();
            let k: Vec<f32> = k.iter().map(|a| a / sum).collect();
            to_picture(img.w, img.h, &filter(img, &k, &k))
        }
        "tiny" => {
            let small = img.resize_area(20, 20);
            let mut out = vec![0f32; img.w * img.h * 3];
            for y in 0..img.h {
                let fy = ((y as f32 + 0.5) * 20.0 / img.h as f32 - 0.5).clamp(0.0, 19.0);
                let (y0, ty) = (fy.floor() as usize, fy.fract());
                let y1 = (y0 + 1).min(19);
                for x in 0..img.w {
                    let fx = ((x as f32 + 0.5) * 20.0 / img.w as f32 - 0.5).clamp(0.0, 19.0);
                    let (x0, tx) = (fx.floor() as usize, fx.fract());
                    let x1 = (x0 + 1).min(19);
                    for c in 0..3 {
                        let p = |yy: usize, xx: usize| small.px[(yy * 20 + xx) * 3 + c] as f32;
                        let top = p(y0, x0) * (1.0 - tx) + p(y0, x1) * tx;
                        let bottom = p(y1, x0) * (1.0 - tx) + p(y1, x1) * tx;
                        out[(y * img.w + x) * 3 + c] = top * (1.0 - ty) + bottom * ty;
                    }
                }
            }
            to_picture(img.w, img.h, &out)
        }
        _ => {
            let mut v = filter(img, &[1.0 / 15.0; 15], &[1.0]);
            v.iter_mut().for_each(|a| *a += 18.0 * noise.normal() as f32);
            to_picture(img.w, img.h, &v)
        }
    }
}

/// What a face's look-up among the others gave: was the best person its own, with what score, and the best
/// score of anybody else (its score as a stranger).
#[derive(Clone, Copy)]
struct Found {
    own: bool,
    best: f32,
    other: f32,
}

/// Each query (print k of `queries` is face `faces[k]`) looked up in the gallery of `gallery` prints (the same
/// faces' clean prints), leaving the face itself out.
fn look_up(
    queries: &[f32],
    gallery: &[f32],
    dim: usize,
    person: &[usize],
    usable: &[bool],
    people: usize,
) -> Vec<Found> {
    let n = person.len();
    let mut of: Vec<Vec<usize>> = vec![Vec::new(); people];
    for j in 0..n {
        if usable[j] {
            of[person[j]].push(j);
        }
    }
    let threads = std::thread::available_parallelism().map_or(4, |t| t.get());
    let step = n.div_ceil(threads).max(1);
    let mut out = vec![Found { own: false, best: -1.0, other: -1.0 }; n];
    std::thread::scope(|scope| {
        for (t, part) in out.chunks_mut(step).enumerate() {
            let of = &of;
            scope.spawn(move || {
                for (off, found) in part.iter_mut().enumerate() {
                    let k = t * step + off;
                    let q = &queries[k * dim..(k + 1) * dim];
                    let (mut best, mut best_p, mut other) = (-1.0f32, usize::MAX, -1.0f32);
                    for (p, faces) in of.iter().enumerate() {
                        let mut top = [f32::MIN; TOP];
                        let mut count = 0;
                        for &j in faces {
                            if j == k {
                                continue;
                            }
                            let s = dot(&gallery[j * dim..(j + 1) * dim], q);
                            count += 1;
                            if s > top[0] {
                                top[0] = s;
                                let mut i = 0;
                                while i + 1 < TOP && top[i] > top[i + 1] {
                                    top.swap(i, i + 1);
                                    i += 1;
                                }
                            }
                        }
                        let take = count.min(TOP);
                        if take == 0 {
                            continue;
                        }
                        let s = top[TOP - take..].iter().sum::<f32>() / take as f32;
                        if s > best {
                            best = s;
                            best_p = p;
                        }
                        if p != person[k] && s > other {
                            other = s;
                        }
                    }
                    *found = Found { own: best_p == person[k], best, other };
                }
            });
        }
    });
    out
}

/// (correct, unknown, wrong name, stranger named), each in percent, at a match setting.
fn rates(found: &[Found], at: f32) -> [f64; 4] {
    let mut r = [0usize; 4];
    for f in found {
        if f.best < at {
            r[1] += 1;
        } else if f.own {
            r[0] += 1;
        } else {
            r[2] += 1;
        }
        if f.other >= at {
            r[3] += 1;
        }
    }
    r.map(|c| 100.0 * c as f64 / found.len().max(1) as f64)
}

/// A number for a person that is the same in every run and says nothing about the name (FNV-1a).
fn person_number(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// `--export`: the library's face pictures as files to train a model on, one folder a person, named by the
/// person's number and never by the name; faces nobody named go into `_`. With `--fold K/N` the people of
/// that fold are left out: they are the ones a trained model is measured on. `faces.tsv` lists each file with
/// the face's quality, its photo as a number (two faces of one photo are two people; of one video they may
/// be the same person twice), whether that is a video, and the face's width there. These are pictures of
/// real people: the folder belongs on the owner's own computers and nowhere else, and only a model's
/// weights ever leave them.
///
/// `--export-held` writes the other side: only the people of the fold, and no unnamed face. It is not for
/// training: it is how the unnamed faces that are one of those people are found and taken out of a training.
fn write_faces(db: &LabelDb, out: &std::path::Path, fold: Option<(u64, u64)>, held: bool) {
    use std::fmt::Write;
    let (mut people, mut named, mut unnamed, mut kept_back) = (std::collections::BTreeSet::new(), 0, 0, 0);
    let mut list = String::new();
    for i in 0..db.n() {
        let Some(picture) = db.store.thumbs.get(i).filter(|t| !t.is_empty()) else { continue };
        let folder = match db.name(i) {
            Some(n) if n == IGNORE => continue,
            Some(n) if !n.is_empty() => {
                let number = person_number(n);
                if fold.is_some_and(|(k, of)| number % of == k) != held {
                    kept_back += 1;
                    continue;
                }
                people.insert(number);
                named += 1;
                format!("{number:016x}")
            }
            _ if held => continue,
            _ => {
                unnamed += 1;
                "_".to_string()
            }
        };
        std::fs::create_dir_all(out.join(&folder)).expect("the folder for a person");
        std::fs::write(out.join(&folder).join(format!("{i}.jpg")), picture).expect("a face's picture written");
        // which photo (as a number, not its hash or its name), whether that is a video, the face's width there
        let file = db.store.file[i].to_lowercase();
        let video = face_library::relocate::VIDEO_EXT.iter().any(|e| file.ends_with(e));
        writeln!(
            list,
            "{folder}/{i}.jpg\t{:.3}\t{:016x}\t{}\t{:.0}",
            db.store.quality[i],
            person_number(&db.store.photo[i]),
            video as u8,
            db.store.boxes[i][2]
        )
        .unwrap();
    }
    std::fs::write(out.join("faces.tsv"), list).expect("the list written");
    println!(
        "written: {named} faces of {} people, {unnamed} faces nobody named; kept back for measuring: {kept_back} faces",
        people.len()
    );
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut cap = usize::MAX;
    if let Some(at) = args.iter().position(|a| a == "--cap") {
        cap = args[at + 1].parse().expect("--cap takes a number");
        args.drain(at..at + 2);
    }
    // --curve: also what each setting below the strictest gives on the clean pictures, to choose a default by
    let curve = args.iter().position(|a| a == "--curve").map(|at| args.remove(at)).is_some();
    // --fold K/N: the people a model in training never sees. Measuring takes only them; --export writes the others.
    let mut fold: Option<(u64, u64)> = None;
    if let Some(at) = args.iter().position(|a| a == "--fold") {
        let (k, n) = args[at + 1].split_once('/').expect("--fold takes K/N");
        fold = Some((k.parse().expect("--fold takes K/N"), n.parse().expect("--fold takes K/N")));
        args.drain(at..at + 2);
    }
    let held_out = |name: &str| fold.is_none_or(|(k, n)| person_number(name) % n == k);
    let mut export: Option<PathBuf> = None;
    let mut held = false;
    if let Some(at) = args.iter().position(|a| a == "--export" || a == "--export-held") {
        held = args[at] == "--export-held";
        export = Some(PathBuf::from(&args[at + 1]));
        args.drain(at..at + 2);
    }
    if args.len() < if export.is_some() { 2 } else { 4 } {
        eprintln!(
            "usage: rec_eval <a copy of a data folder> <an empty private folder> <onnxruntime library> [--cap N] [--fold K/N] <model.onnx>...\n       rec_eval <a copy of a data folder> <an empty private folder> --export <folder> [--fold K/N]"
        );
        std::process::exit(2);
    }
    let (data, home) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    // (opened as a library of whatever model made its prints: this tool reads the faces' pictures and names,
    // never the prints, and must not have them made again)
    let made_by = face_library::store::Store::load(&data, REC_MODELS[0].name, REC_MODELS[0].dim).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let its_own = face_core::faces::RecModel {
        name: Box::leak(made_by.model.clone().into_boxed_str()),
        dim: made_by.dim,
        ..REC_MODELS[0]
    };
    drop(made_by);
    let db = LabelDb::open(&data, &home, its_own, false, &mut |_| {}).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    if let Some(out) = export {
        write_faces(&db, &out, fold, held);
        return;
    }
    let library = PathBuf::from(&args[2]);

    // the labeled faces that have a picture, at most `cap` a person
    let mut names: Vec<&str> = Vec::new();
    let mut by_person: Vec<Vec<usize>> = Vec::new();
    for i in 0..db.n() {
        let Some(name) = db.name(i).filter(|n| !n.is_empty() && *n != IGNORE) else { continue };
        if !held_out(name) {
            continue;
        }
        if db.store.thumbs.get(i).is_none_or(|t| t.is_empty()) {
            continue;
        }
        let p = names.iter().position(|n| *n == name).unwrap_or_else(|| {
            names.push(name);
            by_person.push(Vec::new());
            names.len() - 1
        });
        by_person[p].push(i);
    }
    let (mut faces, mut person) = (Vec::new(), Vec::new());
    for (p, all) in by_person.iter().enumerate() {
        let take = all.len().min(cap);
        for t in 0..take {
            faces.push(all[t * all.len() / take]);
            person.push(p);
        }
    }
    let crops: Vec<Bgr> = faces.iter().map(|&i| db.store.thumb(i).expect("a face's picture")).collect();
    let fits = crops.iter().filter(|c| c.w == 112 && c.h == 112).count();
    let usable: Vec<bool> = faces.iter().map(|&i| !(db.store.quality[i] < Q_ENROLL)).collect();
    println!(
        "labeled faces with a picture: {} of {} people; measured: {} (at most {} a person), {} of them good enough to learn from; pictures of 112x112: {}",
        by_person.iter().map(Vec::len).sum::<usize>(),
        names.len(),
        faces.len(),
        if cap == usize::MAX { "all".to_string() } else { cap.to_string() },
        usable.iter().filter(|u| **u).count(),
        fits
    );
    if fits != crops.len() {
        eprintln!("some pictures are not 112x112: stopping");
        std::process::exit(1);
    }

    // the same poor pictures for every model
    let kinds = ["blur", "tiny", "motion"];
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let poor: Vec<Vec<Bgr>> =
        kinds.iter().map(|kind| crops.iter().map(|c| worse(c, kind, &mut noise)).collect()).collect();

    let rt = Runtime::new(&library, Choice::from_env()).expect("the model runtime");
    for file in &args[3..] {
        let path = PathBuf::from(file);
        let model = match rt.model(&path, "Recognition", &Probe::face()) {
            Ok(m) => m,
            Err(e) => {
                println!("\n{}: {e}", path.file_name().unwrap().to_string_lossy());
                continue;
            }
        };
        let dim = embed(&model, &crops[0], FORMS[0]).len();
        // how far apart the same person's pairs and different people's pairs lie (d'), over every `stride`th face
        let apart = |prints: &[f32], stride: usize| {
            let (mut same, mut diff) = (Vec::new(), Vec::new());
            let n = prints.len() / dim;
            for a in 0..n {
                for b in (a + 1)..n {
                    let s = dot(&prints[a * dim..(a + 1) * dim], &prints[b * dim..(b + 1) * dim]) as f64;
                    if person[a * stride] == person[b * stride] { same.push(s) } else { diff.push(s) }
                }
            }
            let stat = |v: &[f64]| {
                let m = v.iter().sum::<f64>() / v.len().max(1) as f64;
                (m, v.iter().map(|a| (a - m) * (a - m)).sum::<f64>() / v.len().max(1) as f64)
            };
            let ((ms, vs), (md, vd)) = (stat(&same), stat(&diff));
            (ms, md, (ms - md) / ((vs + vd) / 2.0).sqrt())
        };
        // the form the model takes its faces in: the one that tells people apart best on a sample
        let sample = (faces.len() / 300).max(1);
        let tried: Vec<(Form, f64)> = FORMS
            .iter()
            .map(|&form| {
                let prints: Vec<f32> = crops.iter().step_by(sample).flat_map(|c| embed(&model, c, form)).collect();
                (form, apart(&prints, sample).2)
            })
            .collect();
        let form = tried.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        println!(
            "
{}: takes its faces as {} in {} (d' on a sample: {})",
            path.file_name().unwrap().to_string_lossy(),
            if form.rgb { "red-green-blue" } else { "blue-green-red" },
            if form.raw { "0 to 255" } else { "-1 to 1" },
            tried
                .iter()
                .map(|(f, d)| format!(
                    "{}{} {d:.2}",
                    if f.rgb { "RGB" } else { "BGR" },
                    if f.raw { " raw" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let began = Instant::now();
        let all = |pictures: &[Bgr]| -> Vec<f32> { pictures.iter().flat_map(|c| embed(&model, c, form)).collect() };
        let clean = all(&crops);
        let ms = began.elapsed().as_secs_f64() * 1000.0 / crops.len() as f64;
        let (ms_, md, dprime) = apart(&clean, 1);

        let found = look_up(&clean, &clean, dim, &person, &usable, names.len());
        // up to 0.95: a model's scores have their own scale (one trained here names nobody wrongly from 0.6 up)
        let edge = (15..=95).map(|t| t as f32 / 100.0).find(|&t| {
            let r = rates(&found, t);
            r[2] == 0.0 && r[3] == 0.0
        });
        println!(
            "\n{} ({} MB, prints of {} numbers, {:.1} ms a face on the {}): same person {:.3}, different people {:.3}, d' {:.2}",
            path.file_name().unwrap().to_string_lossy(),
            std::fs::metadata(&path).map_or(0, |m| m.len() / 1_000_000),
            dim,
            ms,
            model.device(),
            ms_,
            md,
            dprime
        );
        let Some(edge) = edge else {
            println!("  no setting from 0.15 to 0.95 names no stranger and no wrong person");
            for t in [0.30f32, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90] {
                let r = rates(&found, t);
                println!(
                    "  at {t:.2}: correct {:.1}%, unknown {:.1}%, wrong {:.2}%, stranger named {:.2}%",
                    r[0], r[1], r[2], r[3]
                );
            }
            continue;
        };
        let at = edge + 0.02;
        println!(
            "  lowest setting with no stranger and no wrong name: {edge:.2}; match setting (that + 0.02): {at:.2}"
        );
        let line = |what: &str, found: &[Found]| {
            let r = rates(found, at);
            println!(
                "  {what:<7} correct {:5.1}%  unknown {:5.1}%  wrong {:5.2}%  stranger named {:5.2}%",
                r[0], r[1], r[2], r[3]
            );
        };
        line("clean", &found);
        for (kind, pictures) in kinds.iter().zip(&poor) {
            line(kind, &look_up(&all(pictures), &clean, dim, &person, &usable, names.len()));
        }
        if curve {
            for step in (0..=6).rev() {
                let t = edge - 0.05 * step as f32;
                let r = rates(&found, t);
                println!(
                    "  at {t:.2}: correct {:5.1}%  unknown {:5.1}%  wrong {:5.2}%  stranger named {:5.2}%",
                    r[0], r[1], r[2], r[3]
                );
            }
        }
    }
}
