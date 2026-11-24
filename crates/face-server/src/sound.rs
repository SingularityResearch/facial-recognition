//! What a sound is, as numbers and in a word (the part about how it sounds):
//! a song's numbers (`measure`: level, dynamics, tempo, brightness, bass, key and mode), a background's
//! (`ambient`: its floor, the events over it, steadiness, flatness, mains hum, rumble and hiss, how
//! speech-like it comes and goes), the kinds of sound the sound-recognition model hears (`Sounds`: YAMNet,
//! 521 kinds, as ONNX), those gathered into a few families, and all of it in a word with how sure (`what`).
//!
//! Measured values and opinions of the sound, nothing about a listener. The words and their cut-offs are
//! not measured against anything: evidence, shown with its score. The sound itself is never kept.

use std::{f64::consts::PI, path::Path};

use face_core::runtime::{Choice, Model, Runtime, find_library};
use face_files::filejson::round;
use rustfft::{FftPlanner, num_complex::Complex};
use serde_json::{Map, Value, json};

/// samples a second the sound is measured at
pub const RATE: usize = 22050;
/// dBFS: quieter than this, nothing is playing here
const SILENT: f64 = -65.0;
/// Krumhansl's key profiles
const MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const NOTES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const N: usize = 2048;
const HOP: usize = 512;

/// A sum of 32-bit numbers added pairwise (by eights, in halves): a fixed order, so that a mean made of it is the same
/// to the last bit on every device. It matters where a sound has gaps of digital silence: what is left of them after the
/// mean is taken away is that last bit, and a level is made of it.
fn pairwise(a: &[f32]) -> f32 {
    let n = a.len();
    if n < 8 {
        return a.iter().fold(0.0, |sum, v| sum + v);
    }
    if n <= 128 {
        let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
        let whole = n - n % 8;
        for eight in a[8..whole].as_chunks::<8>().0 {
            for (sum, v) in r.iter_mut().zip(eight) {
                *sum += v;
            }
        }
        return a[whole..]
            .iter()
            .fold(((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7])), |sum, v| sum + v);
    }
    let half = n / 2 - (n / 2) % 8;
    pairwise(&a[..half]) + pairwise(&a[half..])
}

/// The mean of some 32-bit numbers' squares.
fn mean_square(x: &[f32]) -> f32 {
    let squares: Vec<f32> = x.iter().map(|v| v * v).collect();
    pairwise(&squares) / x.len().max(1) as f32
}

/// The sound without its constant offset (an input's offset is no sound), and how loud it is (dBFS).
fn centred(x: &[f32]) -> (Vec<f32>, f64) {
    let mean = pairwise(x) / x.len().max(1) as f32;
    let x: Vec<f32> = x.iter().map(|v| v - mean).collect();
    let level = 20.0 * (mean_square(&x).sqrt() as f64 + 1e-12).log10();
    (x, level)
}

/// The Hann window whose two ends are zero.
fn hanning(n: usize) -> Vec<f64> {
    (0..n).map(|k| if n < 2 { 1.0 } else { 0.5 - 0.5 * (2.0 * PI * k as f64 / (n - 1) as f64).cos() }).collect()
}

/// The sizes of the first half of some numbers' spectrum.
fn spectrum(x: &[f64], planner: &mut FftPlanner<f64>) -> Vec<f64> {
    let mut buf: Vec<Complex<f64>> = x.iter().map(|v| Complex::new(*v, 0.0)).collect();
    planner.plan_fft_forward(buf.len()).process(&mut buf);
    buf[..x.len() / 2 + 1].iter().map(|c| c.norm()).collect()
}

/// Every moment's spectrum: frames of 2,048 samples, 512 apart, each under a hanning window.
fn frames(x: &[f32]) -> Vec<Vec<f64>> {
    let (window, mut planner) = (hanning(N), FftPlanner::new());
    (0..1 + (x.len() - N) / HOP)
        .map(|f| {
            let frame: Vec<f64> = x[f * HOP..f * HOP + N].iter().zip(&window).map(|(v, w)| *v as f64 * w).collect();
            spectrum(&frame, &mut planner)
        })
        .collect()
}

/// A percentile, by straight lines between neighbours.
fn percentile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = (sorted.len() - 1) as f64 * q / 100.0;
    let (low, t) = (at.floor() as usize, at - at.floor());
    let (a, b) = (sorted[low], sorted[(low + 1).min(sorted.len() - 1)]);
    if t >= 0.5 { b - (b - a) * (1.0 - t) } else { a + (b - a) * t }
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 }
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut n) = (0.0, 0usize);
    for v in values {
        sum += v;
        n += 1;
    }
    sum / n.max(1) as f64
}

/// How alike two rows of numbers move (their correlation).
fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let (ma, mb) = (mean(a.iter().copied()), mean(b.iter().copied()));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        ab += (x - ma) * (y - mb);
        aa += (x - ma) * (x - ma);
        bb += (y - mb) * (y - mb);
    }
    ab / (aa.sqrt() * bb.sqrt())
}

/// The sound as numbers: {level, dynamics, tempo, pulse, bright, bass, change, key, mode, mode_sure}, or None
/// when it is silence (or shorter than two seconds). level dBFS; dynamics dB between its quiet and loud
/// moments; tempo beats a minute and pulse 0..1 how clear that beat is; bright the centre of its spectrum
/// in Hz; bass the share of its energy under 150 Hz; change how much the spectrum moves from moment to
/// moment; key and mode (major or minor) with mode_sure 0..1.
pub fn measure(x: &[f32], rate: usize) -> Option<Value> {
    if x.len() < rate * 2 {
        return None;
    }
    let (x, level) = centred(x);
    if level < SILENT {
        return None;
    }
    let spec = frames(&x);
    let freq: Vec<f64> = (0..N / 2 + 1).map(|k| k as f64 * rate as f64 / N as f64).collect();
    let total: Vec<f64> = spec.iter().map(|f| f.iter().map(|s| s * s).sum::<f64>() + 1e-12).collect();
    let rms: Vec<f64> = total.iter().map(|t| 10.0 * (t / N as f64 + 1e-12).log10()).collect();
    let loudest = rms.iter().copied().fold(f64::MIN, f64::max);
    // (the gaps are left out)
    let heard: Vec<usize> = (0..spec.len()).filter(|f| rms[*f] > loudest - 40.0).collect();
    let centroid =
        mean(heard.iter().map(|f| spec[*f].iter().zip(&freq).map(|(s, hz)| s * s * hz).sum::<f64>() / total[*f]));
    let bass = mean(heard.iter().map(|f| {
        spec[*f].iter().zip(&freq).filter(|(_, hz)| **hz < 150.0).map(|(s, _)| s * s).sum::<f64>() / total[*f]
    }));
    let loud: Vec<f64> = heard.iter().map(|f| rms[*f]).collect();
    let dynamics = percentile(&loud, 95.0) - percentile(&loud, 10.0);
    let logs: Vec<Vec<f64>> = spec.iter().map(|f| f.iter().map(|s| (100.0 * s).ln_1p()).collect()).collect();
    // how much is new in each frame
    let flux: Vec<f64> = logs.windows(2).map(|w| w[1].iter().zip(&w[0]).map(|(b, a)| (b - a).max(0.0)).sum()).collect();
    let all = mean(logs.iter().flat_map(|f| f.iter().copied()));
    let change = mean(flux.iter().copied()) / (all * (N / 2 + 1) as f64 + 1e-9);
    // tempo: the lag at which the pattern of new sound repeats best, between 50 and 200 beats a minute
    let centre = mean(flux.iter().copied());
    let env: Vec<f64> = flux.iter().map(|f| f - centre).collect();
    let acf = |lag: usize| env[lag..].iter().zip(&env).map(|(a, b)| a * b).sum::<f64>();
    let per_frame = HOP as f64 / rate as f64;
    let (lo, hi) =
        ((60.0 / 200.0 / per_frame) as usize, (env.len().saturating_sub(1)).min((60.0 / 50.0 / per_frame) as usize));
    let (mut tempo, mut pulse) = (None, None);
    if hi > lo + 2 && acf(0) > 0.0 {
        // (people hear the beat near 110)
        let weighed = |lag: usize| {
            let bpm = 60.0 / (lag as f64 * per_frame);
            acf(lag) * (-0.5 * ((bpm / 110.0).log2() / 0.9).powi(2)).exp()
        };
        let best = (lo..hi).fold(lo, |best, lag| if weighed(lag) > weighed(best) { lag } else { best });
        (tempo, pulse) = (Some(60.0 / (best as f64 * per_frame)), Some((acf(best) / acf(0)).max(0.0)));
    }
    // key and mode: how much of each of the 12 notes sounds, against the major and minor profiles
    let mut chroma = [0.0f64; 12];
    for (k, hz) in freq.iter().enumerate().filter(|(_, hz)| (55.0..=2000.0).contains(*hz)) {
        let note = (12.0 * (hz / 440.0).log2() + 69.0).round_ties_even() as i64;
        chroma[note.rem_euclid(12) as usize] += heard.iter().map(|f| spec[*f][k] * spec[*f][k]).sum::<f64>();
    }
    let (mut key, mut mode, mut sure) = (None, None, None);
    let sum: f64 = chroma.iter().sum();
    if sum > 0.0 {
        let chroma: Vec<f64> = chroma.iter().map(|c| c / sum).collect();
        let mut fits: Vec<(f64, &str, &str)> = Vec::new();
        for (name, profile) in [("major", MAJOR), ("minor", MINOR)] {
            for k in 0..12 {
                let turned: Vec<f64> = (0..12).map(|i| profile[(i + 12 - k) % 12]).collect();
                fits.push((correlation(&chroma, &turned), NOTES[k], name));
            }
        }
        fits.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(a.1)).then(b.2.cmp(a.2)));
        let other = fits.iter().find(|f| f.2 != fits[0].2).map_or(fits[0].0, |f| f.0);
        (key, mode, sure) = (Some(fits[0].1), Some(fits[0].2), Some(((fits[0].0 - other) * 4.0).clamp(0.0, 1.0)));
    }
    let r = |v: Option<f64>, d: usize| v.map(|v| round(v, d));
    Some(
        json!({"level": round(level, 1), "dynamics": round(dynamics, 1), "tempo": r(tempo, 0), "pulse": r(pulse, 2), "bright": round(centroid, 0),
                "bass": round(bass, 3), "change": round(change, 4), "key": key, "mode": mode, "mode_sure": r(sure, 2)}),
    )
}

/// The background of a sound as numbers, for what isn't a song: {level, floor, events, steady, flat, hum,
/// low, high, speechlike}, or None when it is silence (or shorter than two seconds).
///   level   how loud overall (dBFS)             floor  the level it rarely goes under: the steady background
///   events  dB between that floor and its loud moments (things happening over the background)
///   steady  0..1, 1 = the loudness hardly moves (a fan, a hum), 0 = it comes and goes (talk, music, activity)
///   flat    0..1, 1 = hiss-like noise with no tone, 0 = pure tones
///   hum     dB by which mains hum (50 or 60 Hz and its multiples) stands out of its surroundings
///   low, high  the shares of the energy under 150 Hz (rumble) and over 4 kHz (hiss, clatter)
///   speechlike  0..1, how much of the loudness of the voice range comes and goes at the rate of syllables
pub fn ambient(x: &[f32], rate: usize) -> Option<Value> {
    if x.len() < rate * 2 {
        return None;
    }
    let (x, level) = centred(x);
    if level < SILENT - 15.0 {
        return None;
    }
    let spec = frames(&x);
    let power: Vec<Vec<f64>> = spec.iter().map(|f| f.iter().map(|s| s * s).collect()).collect();
    let freq: Vec<f64> = (0..N / 2 + 1).map(|k| k as f64 * rate as f64 / N as f64).collect();
    let total: f64 = power.iter().map(|f| f.iter().sum::<f64>() + 1e-20).sum();
    // each moment's level, in dBFS like `level`
    let db: Vec<f64> =
        (0..spec.len()).map(|f| (10.0 * (mean_square(&x[f * HOP..f * HOP + N]) + 1e-24).log10()) as f64).collect();
    let (floor, loud) = (percentile(&db, 10.0), percentile(&db, 95.0));
    let spread = {
        let m = mean(db.iter().copied());
        mean(db.iter().map(|d| (d - m) * (d - m))).sqrt()
    };
    let bins = N / 2 + 1;
    let avg: Vec<f64> = (0..bins).map(|k| mean(power.iter().map(|f| f[k])) + 1e-20).collect();
    let band: Vec<f64> = (0..bins).filter(|k| freq[*k] > 100.0 && freq[*k] < 8000.0).map(|k| avg[k]).collect();
    let flat = mean(band.iter().map(|v| v.ln())).exp() / mean(band.iter().copied());
    // the mains tone and its first multiples against their surroundings
    let mut hum = 0.0f64;
    for base in [50.0, 60.0] {
        let lines: Vec<f64> = (1..=3)
            .map(|k| {
                let i = (base * k as f64 * N as f64 / rate as f64).round_ties_even() as usize;
                let near: Vec<f64> =
                    avg[1.max(i.saturating_sub(6))..i - 1].iter().chain(&avg[i + 2..i + 7]).copied().collect();
                let peak = avg[i - 1..i + 2].iter().copied().fold(f64::MIN, f64::max);
                10.0 * (peak / (median(&near) + 1e-20)).log10()
            })
            .collect();
        hum = hum.max(median(&lines));
    }
    let voice: Vec<f64> =
        power.iter().map(|f| (0..bins).filter(|k| freq[*k] > 300.0 && freq[*k] < 3400.0).map(|k| f[k]).sum()).collect();
    let centre = mean(voice.iter().copied()) + 1e-20;
    let window = hanning(voice.len());
    let env: Vec<f64> = voice.iter().zip(&window).map(|(v, w)| (v / centre - 1.0) * w).collect();
    let moves: Vec<f64> = spectrum(&env, &mut FftPlanner::new()).iter().map(|m| m * m).collect();
    let at = |k: usize| k as f64 * rate as f64 / (HOP as f64 * env.len() as f64);
    let syllables: f64 = moves.iter().enumerate().filter(|(k, _)| (2.0..=8.0).contains(&at(*k))).map(|(_, m)| m).sum();
    let speechlike =
        syllables / (moves.iter().enumerate().filter(|(k, _)| at(*k) > 0.3).map(|(_, m)| m).sum::<f64>() + 1e-20);
    let share = |keep: &dyn Fn(f64) -> bool| {
        power.iter().map(|f| (0..bins).filter(|k| keep(freq[*k])).map(|k| f[k]).sum::<f64>()).sum::<f64>() / total
    };
    Some(
        json!({"level": round(level, 1), "floor": round(floor, 1), "events": round(loud - floor, 1), "steady": round((1.0 - spread / 6.0).max(0.0), 2),
                "flat": round(flat, 3), "hum": round(hum.max(0.0), 1), "low": round(share(&|hz| hz < 150.0), 3), "high": round(share(&|hz| hz > 4000.0), 3),
                "speechlike": round(speechlike, 2)}),
    )
}

/// The model's 521 kinds of sound, gathered into the few families a background is described by. A kind
/// belongs to the first family one of whose words is in its name (lower case). Not measured against
/// anything: a way to say the model's answer in plain words, shown with its score. Animal sounds come
/// before music: "bird song" is a bird.
const FAMILIES: [(&str, &[&str]); 8] = [
    ("quiet", &["silence"]),
    ("animal sounds", &["animal", "dog", "bark", "cat", "meow", "bird", "chirp", "purr", "insect", "cricket"]),
    (
        "music",
        &[
            "music",
            "singing",
            "song",
            "guitar",
            "piano",
            "drum",
            "bass",
            "synth",
            "violin",
            "choir",
            "orchestra",
            "instrument",
            "trumpet",
            "saxophone",
            "flute",
            "keyboard (musical)",
            "organ",
            "harmonica",
            "rapping",
            "humming",
            "chant",
            "beatboxing",
            "strum",
            "percussion",
            "cymbal",
            "jingle",
            "soundtrack",
        ],
    ),
    ("television or radio", &["television", "radio"]),
    (
        "speech",
        &[
            "speech",
            "conversation",
            "narration",
            "monologue",
            "babbling",
            "whispering",
            "shout",
            "yell",
            "laughter",
            "chatter",
            "crowd",
            "hubbub",
        ],
    ),
    (
        "steady background noise",
        &[
            "noise",
            "static",
            "hum",
            "air conditioning",
            "fan",
            "engine",
            "idling",
            "vehicle",
            "truck",
            "car",
            "wind",
            "water",
            "stream",
            "rain",
            "ocean",
            "waves",
            "rumble",
            "inside,",
            "outside,",
            "white",
            "pink",
            "drone",
            "buzz",
            "motor",
            "aircraft",
            "train",
            "traffic",
            "hiss",
            "vibration",
            "throbbing",
            "ventilation",
        ],
    ),
    (
        "alert sounds",
        &[
            "alarm",
            "telephone",
            "ringtone",
            "beep",
            "doorbell",
            "siren",
            "buzzer",
            "smoke detector",
            "bell",
            "chime",
            "ding",
        ],
    ),
    (
        "activity",
        &[
            "typing",
            "keyboard",
            "click",
            "mouse",
            "door",
            "footsteps",
            "walk",
            "dishes",
            "cutlery",
            "tick",
            "knock",
            "writing",
            "rustle",
            "chair",
            "drawer",
            "tap",
            "thump",
            "clatter",
            "scissors",
            "zipper",
            "cupboard",
            "printer",
            "breathing",
            "cough",
            "sneeze",
            "chewing",
            "sniff",
            "clapping",
            "bang",
            "squeak",
            "crumpling",
            "tearing",
            "paper",
            "sound effect",
        ],
    ),
];

/// The family a kind of sound belongs to.
pub fn family_of(kind: &str) -> &'static str {
    let low = kind.to_lowercase();
    FAMILIES
        .iter()
        .find(|(_, words)| words.iter().any(|w| low.contains(w)))
        .map_or("other sounds", |(family, _)| family)
}

/// The model's kinds of sound added up by family: {family: share}, most first.
pub fn families(sounds: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut out: Vec<(&str, f64)> = Vec::new();
    for (kind, score) in sounds.into_iter().flatten() {
        let (family, score) = (family_of(kind), score.as_f64().unwrap_or(0.0));
        match out.iter_mut().find(|(f, _)| *f == family) {
            Some((_, sum)) => *sum += score,
            None => out.push((family, score)),
        }
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.into_iter().map(|(family, sum)| (family.to_string(), json!(round(sum.min(1.0), 3)))).collect()
}

/// A steady background in plain words from its numbers: what kind of noise it is.
pub fn background(amb: Option<&Value>) -> String {
    let Some(amb) = amb else { return String::new() };
    let of = |k: &str| amb[k].as_f64().unwrap_or(0.0);
    // near full strength and never changing: no room sounds so
    if of("level") >= -12.0 && of("steady") >= 0.8 {
        return "an input with nothing plugged in, probably".to_string();
    }
    let mut parts = Vec::new();
    if of("hum") >= 10.0 {
        parts.push("mains hum");
    }
    if of("low") >= 0.5 {
        parts.push("low rumble");
    } else if of("high") >= 0.3 {
        parts.push("hiss");
    } else if of("flat") >= 0.3 {
        parts.push("broadband, fan-like");
    }
    parts.join(", ")
}

/// In a few words what an interface heard, and how sure (0..1): "quiet", "music", "speech", "television or
/// radio", "steady background noise", "activity", "alert sounds", "animal sounds" or "other sounds". From the
/// model's kinds of sound when it is here; without it only from how the background behaves (then never
/// music or speech). An opinion with its score, not a fact about the room.
pub fn what(sounds: Option<&Map<String, Value>>, amb: Option<&Value>) -> (String, f64) {
    let Some(amb) = amb.filter(|a| a["level"].as_f64().unwrap_or(f64::MIN) >= SILENT) else {
        return ("quiet".to_string(), if amb.is_none() { 1.0 } else { 0.8 });
    };
    let steady = amb["steady"].as_f64().unwrap_or(0.0);
    let fam = families(sounds);
    let of = |family: &str| fam.get(family).and_then(Value::as_f64).unwrap_or(0.0);
    let Some((first, score)) = fam.iter().next().map(|(f, s)| (f.clone(), s.as_f64().unwrap_or(0.0))) else {
        return ((if steady > 0.7 { "steady background noise" } else { "sound that comes and goes" }).to_string(), 0.5);
    };
    if of("music") >= 0.25 && of("music") >= of("speech") {
        return ("music".to_string(), of("music"));
    }
    if of("speech") >= 0.25 {
        return ("speech".to_string(), of("speech"));
    }
    // a background that hardly moves is a steady noise, whatever single thing the model likens it to
    if steady >= 0.7 && !["quiet", "music", "speech"].contains(&first.as_str()) {
        return ("steady background noise".to_string(), round(of("steady background noise").max(steady * 0.6), 2));
    }
    (first, score)
}

/// Of the interfaces listened to, the one to take a song's numbers from: an output that sounds (the song
/// exactly as played) before any microphone (the song through a room), the loudest of its kind.
pub fn best(heard: &[Value]) -> Option<&Value> {
    let rank = |h: &Value| (h["kind"] == "output", h["heard"]["level"].as_f64().unwrap_or(f64::MIN));
    (heard.iter().filter(|h| h["heard"].is_object())).fold(None, |best: Option<&Value>, h| match best {
        Some(b) if (rank(b).0 && !rank(h).0) || (rank(b).0 == rank(h).0 && rank(b).1 >= rank(h).1) => Some(b),
        _ => Some(h),
    })
}

/// One set of numbers for a song from the several times it was listened to: the median of each, the key and
/// mode heard most.
pub fn combine(measures: &[&Value]) -> Option<Value> {
    let measures: Vec<&&Value> = measures.iter().filter(|m| m.as_object().is_some_and(|o| !o.is_empty())).collect();
    if measures.is_empty() {
        return None;
    }
    let mut out = Map::new();
    out.insert("heard".to_string(), json!(measures.len()));
    for k in ["level", "dynamics", "tempo", "pulse", "bright", "bass", "change", "mode_sure"] {
        let values: Vec<f64> = measures.iter().filter_map(|m| m[k].as_f64()).collect();
        out.insert(k.to_string(), if values.is_empty() { Value::Null } else { json!(round(median(&values), 4)) });
    }
    for k in ["key", "mode"] {
        let mut counted: Vec<(&str, usize)> = Vec::new();
        for v in measures.iter().filter_map(|m| m[k].as_str()).filter(|v| !v.is_empty()) {
            match counted.iter_mut().find(|(name, _)| *name == v) {
                Some((_, n)) => *n += 1,
                None => counted.push((v, 1)),
            }
        }
        // (the one heard most; of those heard alike, the first)
        let most = counted
            .iter()
            .fold(None::<&(&str, usize)>, |best, c| if best.is_none_or(|b| c.1 > b.1) { Some(c) } else { best });
        out.insert(k.to_string(), most.map_or(Value::Null, |(name, _)| json!(name)));
    }
    Some(Value::Object(out))
}

// ----------------------------------------------------------------------------- the sound-recognition model

/// samples a second the model was taught at, and how many it takes at a time (0.975 s)
const MODEL_RATE: usize = 16000;
const MODEL_SAMPLES: usize = 15600;
const LABELS: &str = include_str!("yamnet_labels.txt");

/// The zeroth modified Bessel function of the first kind (for the Kaiser window).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, half) = (1.0, 1.0, x / 2.0);
    for k in 1..60 {
        term *= (half / k as f64) * (half / k as f64);
        sum += term;
        if term < 1e-17 * sum {
            break;
        }
    }
    sum
}

/// A sound at another number of samples a second: each new sample from its neighbours in the old sound,
/// weighed by a sinc under a Kaiser window, the way MediaPipe's resampler brings a sound to its model's
/// rate (cut off at nine tenths of the lower rate's half, five periods of that rate to each side, the
/// window's shape 6): so the model hears a sound as MediaPipe's own resampler gives it, as nearly as can be.
pub fn resample(x: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    const BETA: f64 = 6.0;
    let step = from as f64 / to as f64;
    let cutoff = 0.9 * from.min(to) as f64 / from as f64; // in turns of half the old rate
    let radius = 5.0 * step.max(1.0);
    let window = |d: f64| bessel_i0(BETA * (1.0 - (d / radius).powi(2)).max(0.0).sqrt()) / bessel_i0(BETA);
    (0..(x.len() as f64 / step).ceil() as usize)
        .map(|k| {
            let at = k as f64 * step;
            let (first, last) = ((at - radius).ceil() as i64, (at + radius).floor() as i64);
            let (mut sum, mut weights) = (0.0, 0.0);
            for i in first..=last {
                let d = i as f64 - at;
                let turn = PI * cutoff * d;
                let weight = if turn.abs() < 1e-12 { 1.0 } else { turn.sin() / turn } * window(d);
                weights += weight;
                // (beyond the sound's ends is silence)
                sum += weight * usize::try_from(i).ok().and_then(|i| x.get(i)).copied().unwrap_or(0.0) as f64;
            }
            (sum / weights) as f32
        })
        .collect()
}

/// The sound-recognition model (YAMNet, Apache 2.0: 521 kinds of sound), as ONNX on the processor.
pub struct Sounds {
    model: Model,
    labels: Vec<&'static str>,
}

impl Sounds {
    /// The model's file among the models (`yamnet.onnx`), when it is there.
    pub fn file(root: &Path) -> Option<std::path::PathBuf> {
        let file = root.join("models").join("yamnet.onnx");
        std::fs::metadata(&file).is_ok_and(|m| m.len() > 1_000_000).then_some(file)
    }

    /// The model, loaded; None when its file or the model runtime isn't there.
    pub fn load(root: &Path) -> Option<Sounds> {
        let runtime = Runtime::new(&find_library(root)?, Choice::Cpu).ok()?;
        let model = runtime.model_on_cpu(&Self::file(root)?, "sounds").ok()?;
        Some(Sounds { model, labels: LABELS.lines().collect() })
    }

    /// What kinds of sound these seconds hold: {kind: share 0..1} of the `top` kinds, each the model's
    /// average score over the seconds ("Music", "Speech", "Silence", "Typing", "Air conditioning"...). None
    /// for less than a second. A judgement of the sound, with its score: a microphone in a room hears
    /// whatever is there.
    pub fn hear(&self, x: &[f32], rate: usize, top: usize) -> Option<Map<String, Value>> {
        if x.len() < rate {
            return None;
        }
        // the model was taught on sound at ordinary loudness: a quiet microphone is brought up to it first
        let mean = (x.iter().map(|v| *v as f64).sum::<f64>() / x.len() as f64) as f32;
        let peak = x.iter().map(|v| (v - mean).abs()).fold(0.0f32, f32::max);
        let gain = if peak > 1e-5 { 0.5 / peak } else { 1.0 };
        let x: Vec<f32> = x.iter().map(|v| (v - mean) * gain).collect();
        let x = resample(&x, rate, MODEL_RATE);
        // a second at a time, the last, shorter one filled up with silence
        let (mut total, mut n): (Vec<(usize, f64)>, usize) = (Vec::new(), 0);
        for piece in x.chunks(MODEL_SAMPLES) {
            let mut one = piece.to_vec();
            one.resize(MODEL_SAMPLES, 0.0);
            let scores = self.model.run(&[MODEL_SAMPLES], &one).ok()?.into_iter().next()?;
            // (each second's twelve highest kinds count)
            let mut order: Vec<usize> = (0..scores.len()).collect();
            order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]).then(a.cmp(b)));
            for k in order.into_iter().take(12) {
                match total.iter_mut().find(|(kind, _)| *kind == k) {
                    Some((_, sum)) => *sum += scores[k] as f64,
                    None => total.push((k, scores[k] as f64)),
                }
            }
            n += 1;
        }
        if n == 0 {
            return None;
        }
        total.sort_by(|a, b| b.1.total_cmp(&a.1));
        Some(
            (total.into_iter().take(top))
                .filter(|(_, sum)| sum / n as f64 >= 0.02)
                .map(|(k, sum)| {
                    (self.labels.get(k).copied().unwrap_or("?").to_string(), json!(round(sum / n as f64, 3)))
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, seconds: f64, rate: usize, loud: f64) -> Vec<f32> {
        (0..(seconds * rate as f64) as usize)
            .map(|k| (loud * (2.0 * PI * hz * k as f64 / rate as f64).sin()) as f32)
            .collect()
    }

    #[test]
    fn silence_and_what_is_too_short_have_no_numbers() {
        assert_eq!(measure(&vec![0.0; RATE * 3], RATE), None);
        assert_eq!(ambient(&vec![0.0; RATE * 3], RATE), None);
        assert_eq!(measure(&tone(440.0, 1.5, RATE, 0.5), RATE), None);
        // an offset alone is no sound
        assert_eq!(measure(&vec![0.25; RATE * 3], RATE), None);
        assert_eq!(what(None, None), ("quiet".to_string(), 1.0));
        assert_eq!(what(None, Some(&json!({"level": -70.0, "steady": 1.0}))), ("quiet".to_string(), 0.8));
    }

    #[test]
    fn a_tone_has_its_level_its_note_and_no_bass() {
        // an A at half strength: -9 dBFS, bright at 440 Hz, the note A
        let m = measure(&tone(440.0, 4.0, RATE, 0.5), RATE).unwrap();
        assert_eq!((&m["level"], &m["key"], &m["bass"]), (&json!(-9.0), &json!("A"), &json!(0.0)));
        assert!((m["bright"].as_f64().unwrap() - 440.0).abs() <= 2.0 && m["dynamics"].as_f64().unwrap() < 0.5, "{m}");
        // a beat twice a second is heard as 120 a minute
        let rate = RATE as f64;
        let beat: Vec<f32> = (0..RATE * 8)
            .map(|k| {
                if (k as f64 / rate * 2.0).fract() < 0.1 {
                    (0.6 * (2.0 * PI * 220.0 * k as f64 / rate).sin()) as f32
                } else {
                    0.0
                }
            })
            .collect();
        let m = measure(&beat, RATE).unwrap();
        assert!((m["tempo"].as_f64().unwrap() - 120.0).abs() <= 3.0 && m["pulse"].as_f64().unwrap() > 0.5, "{m}");
        // a hum at mains pitch stands out of its surroundings, steadily
        let hum: Vec<f32> = tone(50.0, 4.0, RATE, 0.2)
            .iter()
            .zip(tone(100.0, 4.0, RATE, 0.1))
            .zip(tone(150.0, 4.0, RATE, 0.05))
            .map(|((a, b), c)| a + b + c)
            .collect();
        let a = ambient(&hum, RATE).unwrap();
        assert!(
            a["hum"].as_f64().unwrap() > 20.0
                && a["steady"].as_f64().unwrap() > 0.9
                && a["low"].as_f64().unwrap() > 0.9,
            "{a}"
        );
        assert_eq!(background(Some(&a)), "mains hum, low rumble");
    }

    #[test]
    fn kinds_of_sound_are_gathered_into_families_and_said_in_a_word() {
        for (kind, family) in [
            ("Silence", "quiet"),
            ("Bird vocalization, bird call, bird song", "animal sounds"),
            ("Music", "music"),
            ("Guitar", "music"),
            ("Piano", "music"),
            ("Drum kit", "music"),
            ("Television", "television or radio"),
            ("Speech", "speech"),
            ("Air conditioning", "steady background noise"),
            ("Telephone bell ringing", "alert sounds"),
            ("Typing", "activity"),
            ("Computer keyboard", "activity"),
            ("Keyboard (musical)", "music"),
            ("Gunshot, gunfire", "other sounds"),
        ] {
            assert_eq!(family_of(kind), family, "{kind}");
        }
        let sounds = json!({"Music": 0.4, "Guitar": 0.2, "Speech": 0.3, "Typing": 0.1, "Gunshot, gunfire": 0.05})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            Value::Object(families(Some(&sounds))),
            json!({"music": 0.6, "speech": 0.3, "activity": 0.1, "other sounds": 0.05})
        );
        let amb = json!({"level": -30.0, "steady": 0.2});
        assert_eq!(what(Some(&sounds), Some(&amb)), ("music".to_string(), 0.6));
        let talk = json!({"Speech": 0.5, "Music": 0.3}).as_object().unwrap().clone();
        assert_eq!(what(Some(&talk), Some(&amb)), ("speech".to_string(), 0.5));
        let fan = json!({"Mechanical fan": 0.15, "Typing": 0.2}).as_object().unwrap().clone();
        assert_eq!(
            what(Some(&fan), Some(&json!({"level": -40.0, "steady": 0.9}))),
            ("steady background noise".to_string(), 0.54)
        );
        assert_eq!(what(Some(&fan), Some(&amb)), ("activity".to_string(), 0.2));
        assert_eq!(
            what(None, Some(&json!({"level": -40.0, "steady": 0.9}))),
            ("steady background noise".to_string(), 0.5)
        );
        assert_eq!(what(None, Some(&amb)), ("sound that comes and goes".to_string(), 0.5));
        assert_eq!(
            background(Some(&json!({"level": -5.0, "steady": 0.95, "hum": 0.0, "low": 0.0, "high": 0.0, "flat": 0.0}))),
            "an input with nothing plugged in, probably"
        );
        assert_eq!(
            background(Some(&json!({"level": -40.0, "steady": 0.9, "hum": 0.0, "low": 0.1, "high": 0.4, "flat": 0.5}))),
            "hiss"
        );
        assert_eq!(background(None), "");
    }

    #[test]
    fn several_listenings_give_one_set_of_numbers_and_an_output_comes_first() {
        let (a, b, c) = (
            json!({"level": -20.0, "tempo": 120.0, "key": "C", "mode": "major"}),
            json!({"level": -22.0, "tempo": 121.0, "key": "G", "mode": "major"}),
            json!({"level": -30.0, "tempo": null, "key": "G", "mode": "minor"}),
        );
        let one = combine(&[&a, &b, &c, &Value::Null]).unwrap();
        assert_eq!(
            (&one["heard"], &one["level"], &one["tempo"], &one["key"], &one["mode"], &one["bass"]),
            (&json!(3), &json!(-22.0), &json!(120.5), &json!("G"), &json!("major"), &Value::Null)
        );
        assert_eq!(combine(&[&Value::Null]), None);
        let heard = [
            json!({"name": "mic", "kind": "input", "heard": {"level": -10.0}}),
            json!({"name": "quiet", "kind": "output", "heard": null}),
            json!({"name": "speakers", "kind": "output", "heard": {"level": -40.0}}),
            json!({"name": "headset", "kind": "output", "heard": {"level": -25.0}}),
        ];
        assert_eq!(best(&heard).unwrap()["name"], "headset");
        assert_eq!(best(&heard[..2]).unwrap()["name"], "mic");
        assert!(best(&heard[1..2]).is_none());
    }

    #[test]
    fn a_sound_keeps_its_pitch_at_another_rate() {
        let slower = resample(&tone(1000.0, 1.0, RATE, 0.5), RATE, MODEL_RATE);
        assert!((slower.len() as i64 - 16000).abs() <= 1, "{}", slower.len());
        // still a thousand turns a second, as loud as before
        let crossings = slower.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let peak = slower[2000..14000].iter().fold(0.0f32, |p, v| p.max(v.abs()));
        assert!((crossings as i64 - 1000).abs() <= 2 && (peak - 0.5).abs() < 0.01, "{crossings} {peak}");
        // what is too high for the new rate is gone, not turned into a lower tone
        let high = resample(&tone(10_500.0, 1.0, RATE, 0.5), RATE, MODEL_RATE);
        assert!(high[2000..14000].iter().fold(0.0f32, |p, v| p.max(v.abs())) < 0.01);
        assert!((bessel_i0(0.0) - 1.0).abs() < 1e-15 && (bessel_i0(6.0) - 67.234_406_976_478).abs() < 1e-9);
        assert_eq!(LABELS.lines().count(), 521);
    }
}
