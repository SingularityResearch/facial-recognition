//! Speech into words for Analyze, on this computer (the owner's decision, 2026-11-25: a local speech model, so
//! that what is said never leaves the computer).
//!
//! OpenAI's Whisper, English (MIT licence; trained on 680,000 hours of audio with transcripts from the internet,
//! and its card names no other terms for them), as exported to ONNX by onnx-community, 8-bit, run on the
//! processor with the model runtime. Its files are downloaded only when the owner asks for them, from Hugging Face
//! at one pinned revision, each kept only with its pinned size and SHA-256 (as Florence's: `florence.rs`).
//!
//! The microphone (this computer's default one, as `listen.rs` lists them) is listened to from the owner's start
//! to their stop, `MAX_SECONDS` at most; what it heard is kept in memory, made into words and let go: it is never
//! written and never sent. The features (`log_mel`) are Whisper's own: 25 ms windows every 10 ms over 30 seconds,
//! 80 mel bands of Slaney's scale, in decibels with their top 80 dB kept; the words are the model's greedy choice
//! with its own rules (tokens it never begins with, tokens it never says, no times).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use face_core::runtime::{Choice, Parts, Runtime, Tensor, find_library};
use face_files::{filejson, paths::atomic_write};
use serde_json::{Map, Value, json};

pub const NAME: &str = "Whisper base.en";
const REPO: &str = "onnx-community/whisper-base.en";
const REV: &str = "51eefc0af78b103839eda9e7e4f4186acc6517fe";
/// the model's files: (name, size in bytes, SHA-256)
pub const FILES: [(&str, u64, &str); 6] = [
    ("config.json", 2197, "c8a0de5ed8a083565a4319db29d0c210fda35b4d6076c2d711cae53ae00f3cb1"),
    ("generation_config.json", 1556, "3479b1f44a07e41db799e22599222fee5816738036def94a39841cb9cdbb4120"),
    ("vocab.json", 999_186, "f6bd25a65e4e63ca31360e9fb11c7e4f9a391a78385d640acd814092dd6eee4f"),
    ("added_tokens.json", 34_604, "560be47bea388757f8d4cc185c5d82067426cbb6361e38016dd90ddc01ab203a"),
    (
        "onnx/encoder_model_quantized.onnx",
        23_201_320,
        "6e8001198c490bbae018c0044f630c2915efb826bad957006ce36152d0ab2a10",
    ),
    (
        "onnx/decoder_model_merged_quantized.onnx",
        53_692_803,
        "dd4761a3f7add26afda3512abff4706920404c2517e85a9f2ff090b0c0987909",
    ),
];
/// bytes of all of them
pub const SIZE: u64 = 2197 + 1556 + 999_186 + 34_604 + 23_201_320 + 53_692_803;
/// the longest a question is listened to
pub const MAX_SECONDS: f64 = 30.0;
/// samples a second the model takes
pub const RATE: usize = 16_000;
const N_FFT: usize = 400;
const HOP: usize = 160;
const MELS: usize = 80;
/// 30 seconds: what the model always takes (shorter is filled with silence)
const SAMPLES: usize = 30 * RATE;
const FRAMES: usize = SAMPLES / HOP;
/// words of an answer at the most (the model's own limit is 448)
const MAX_TOKENS: usize = 224;
/// the file that says which files were checked as they are now
const VERIFIED: &str = ".verified.json";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Say where the programs' folder is: the model's files are in `<root>/models/whisper-base.en`.
pub fn set_root(root: &Path) {
    let _ = ROOT.set(root.to_path_buf());
}

fn folder() -> Option<PathBuf> {
    ROOT.get().map(|root| root.join("models").join("whisper-base.en"))
}

fn held<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|e| e.into_inner())
}

// --------------------------------------------------------------------------- the files

fn checked(dir: &Path) -> Map<String, Value> {
    std::fs::read(dir.join(VERIFIED))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn stamp(path: &Path) -> Option<(u64, Value)> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let written = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((meta.len(), json!({"size": meta.len(), "mtime_ns": written})))
}

fn note(dir: &Path, name: &str) {
    if let Some((_, now)) = stamp(&dir.join(name)) {
        let mut known = checked(dir);
        known.insert(name.to_string(), now);
        let _ = atomic_write(&dir.join(VERIFIED), filejson::dumps_indent1(&Value::Object(known)).as_bytes());
    }
}

fn sha256_is(path: &Path, sha: &str) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut sum = ring::digest::Context::new(&ring::digest::SHA256);
    let mut block = vec![0u8; 1 << 20];
    loop {
        match file.read(&mut block) {
            Ok(0) => break,
            Ok(n) => sum.update(&block[..n]),
            Err(_) => return false,
        }
    }
    sum.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>() == sha
}

/// Whether `dir/name` is the pinned file: its size, and its SHA-256 (read only when the file changed since).
fn good(dir: &Path, name: &str, known: &Map<String, Value>) -> bool {
    let Some(&(_, size, sha)) = FILES.iter().find(|f| f.0 == name) else { return false };
    let path = dir.join(name);
    let Some((now_size, now)) = stamp(&path) else { return false };
    if now_size != size {
        return false;
    }
    if known.get(name) == Some(&now) {
        return true;
    }
    if !sha256_is(&path, sha) {
        return false;
    }
    note(dir, name);
    true
}

fn ready_in(dir: &Path) -> bool {
    let known = checked(dir);
    FILES.iter().all(|f| good(dir, f.0, &known))
}

pub fn ready() -> bool {
    folder().is_some_and(|dir| ready_in(&dir))
}

#[derive(Default)]
struct Fetching {
    running: bool,
    done: u64,
    error: Option<String>,
}

static FETCHING: Mutex<Fetching> = Mutex::new(Fetching { running: false, done: 0, error: None });

/// For the page: whether speech can be made into words here, and the model's state.
pub fn status() -> Value {
    let f = held(&FETCHING);
    let microphone = microphone().is_some();
    json!({"ready": ready(), "can": microphone, "name": NAME, "bytes": SIZE, "getting": f.running, "done": f.done,
           "error": f.error, "listening": held(&LISTENING).is_some(),
           "why": if microphone { Value::Null } else { json!("This computer has no microphone the program can listen to.") }})
}

/// Where the files are asked for: Hugging Face, or a stand-in on this computer in a check (`FACE_TEST_HUGGINGFACE`,
/// which counts only as `http://127.0.0.1:<port>`). In this crate's own tests it is never Hugging Face.
fn publisher() -> (String, bool) {
    if cfg!(test) {
        return ("http://127.0.0.1:9".to_string(), true);
    }
    let said = std::env::var("FACE_TEST_HUGGINGFACE").ok();
    let port = said.as_deref().and_then(|s| s.strip_prefix("http://127.0.0.1:"));
    match port.filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        Some(_) => (said.unwrap_or_default(), true),
        None => ("https://huggingface.co".to_string(), false),
    }
}

#[cfg(feature = "fetch")]
fn download(dir: &Path) -> Result<(), String> {
    use face_tools::fetch::{Net, save};
    let (base, local) = publisher();
    let net = if local { Net::plain() } else { Net::new() };
    std::fs::create_dir_all(dir.join("onnx")).map_err(|e| e.to_string())?;
    for (name, size, sha) in FILES {
        if good(dir, name, &checked(dir)) {
            held(&FETCHING).done += size;
            continue;
        }
        let start = held(&FETCHING).done;
        net.open(&format!("{base}/{REPO}/resolve/{REV}/{name}"), None, None)
            .and_then(|from| {
                save(from, &dir.join(name), size, sha, &mut |so_far| held(&FETCHING).done = start + so_far)
            })
            .map_err(|e| {
                held(&FETCHING).done = start;
                e.why
            })?;
        note(dir, name);
    }
    Ok(())
}

#[cfg(not(feature = "fetch"))]
fn download(_: &Path) -> Result<(), String> {
    Err("this program was built without its downloads".to_string())
}

/// Download the model on a thread of its own, at the owner's click (two clicks start one download).
pub fn get() -> Result<(), String> {
    let dir = folder().ok_or("The programs' folder isn't known")?;
    {
        let mut f = held(&FETCHING);
        if f.running {
            return Ok(());
        }
        (f.running, f.done, f.error) = (true, 0, None);
    }
    std::thread::Builder::new()
        .name("speech download".into())
        .spawn(move || {
            let got = download(&dir);
            let mut f = held(&FETCHING);
            f.running = false;
            f.error = got.err().map(|e| format!("Couldn't download the speech model: {e}"));
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// --------------------------------------------------------------------------- the features

/// The mel filters of Slaney's scale (as Whisper's are made): MELS rows of N_FFT/2 + 1 weights.
fn filters() -> &'static Vec<Vec<f32>> {
    static MADE: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
    MADE.get_or_init(|| {
        let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
        let min_log_mel = min_log_hz / f_sp;
        let log_step = (6.4f64).ln() / 27.0;
        let to_mel =
            |hz: f64| if hz < min_log_hz { hz / f_sp } else { min_log_mel + (hz / min_log_hz).ln() / log_step };
        let to_hz =
            |mel: f64| if mel < min_log_mel { mel * f_sp } else { min_log_hz * (log_step * (mel - min_log_mel)).exp() };
        let (lo, hi) = (to_mel(0.0), to_mel(RATE as f64 / 2.0));
        let hz: Vec<f64> = (0..MELS + 2).map(|i| to_hz(lo + (hi - lo) * i as f64 / (MELS + 1) as f64)).collect();
        let bins = N_FFT / 2 + 1;
        (0..MELS)
            .map(|m| {
                let norm = 2.0 / (hz[m + 2] - hz[m]);
                (0..bins)
                    .map(|k| {
                        let f = k as f64 * RATE as f64 / N_FFT as f64;
                        let lower = (f - hz[m]) / (hz[m + 1] - hz[m]);
                        let upper = (hz[m + 2] - f) / (hz[m + 2] - hz[m + 1]);
                        (lower.min(upper).max(0.0) * norm) as f32
                    })
                    .collect()
            })
            .collect()
    })
}

/// Whisper's features of up to 30 seconds of sound at `RATE`: MELS × FRAMES numbers, band by band.
pub fn log_mel(audio: &[f32]) -> Vec<f32> {
    use rustfft::{FftPlanner, num_complex::Complex};
    let mut x: Vec<f32> = audio.iter().take(SAMPLES).copied().collect();
    x.resize(SAMPLES, 0.0);
    // centred windows: the sound mirrored at both ends by half a window
    let pad = N_FFT / 2;
    let mut padded = Vec::with_capacity(SAMPLES + 2 * pad);
    padded.extend((1..=pad).rev().map(|i| x[i]));
    padded.extend_from_slice(&x);
    padded.extend((1..=pad).map(|i| x[SAMPLES - 1 - i]));
    let window: Vec<f32> =
        (0..N_FFT).map(|n| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N_FFT as f64).cos()) as f32).collect();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    let bins = N_FFT / 2 + 1;
    let f = filters();
    let mut mel = vec![0f32; MELS * FRAMES];
    let mut buf = vec![Complex::new(0f32, 0f32); N_FFT];
    let mut power = vec![0f32; bins];
    for frame in 0..FRAMES {
        let at = frame * HOP;
        for n in 0..N_FFT {
            buf[n] = Complex::new(padded[at + n] * window[n], 0.0);
        }
        fft.process(&mut buf);
        for k in 0..bins {
            power[k] = buf[k].norm_sqr();
        }
        for (m, row) in f.iter().enumerate() {
            let e: f32 = row.iter().zip(&power).map(|(w, p)| w * p).sum();
            mel[m * FRAMES + frame] = e.max(1e-10).log10();
        }
    }
    let top = mel.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    for v in &mut mel {
        *v = (v.max(top - 8.0) + 4.0) / 4.0;
    }
    mel
}

// --------------------------------------------------------------------------- the model

struct Whisper {
    encoder: Parts,
    decoder: Parts,
    /// a token's text in the vocabulary's characters
    text: HashMap<i64, String>,
    heads: usize,
    head_dim: usize,
    start: i64,
    no_times: i64,
    end: i64,
    /// the tokens it never says, and those it never begins with
    never: Vec<i64>,
    never_first: Vec<i64>,
}

fn first(part: &Parts, inputs: &[(&str, &Tensor)]) -> Result<Tensor, String> {
    let wanted = part.outputs.first().cloned().unwrap_or_default();
    let mut out = part.run(inputs, &|name| name == wanted).map_err(|e| e.0)?;
    let (_, shape, data) = out.pop().ok_or("the model gave no answer")?;
    Ok(Tensor::F32(shape, data))
}

impl Whisper {
    fn load(root: &Path, dir: &Path) -> Result<Whisper, String> {
        let json = |name: &str| -> Result<Value, String> {
            serde_json::from_slice(&std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?)
                .map_err(|e| format!("{name}: {e}"))
        };
        let (config, generation, vocab, added) =
            (json("config.json")?, json("generation_config.json")?, json("vocab.json")?, json("added_tokens.json")?);
        let mut text: HashMap<i64, String> = HashMap::new();
        for source in [&vocab, &added] {
            for (k, v) in source.as_object().into_iter().flatten() {
                if let Some(id) = v.as_i64() {
                    text.insert(id, k.clone());
                }
            }
        }
        let number = |v: &Value, k: &str| v[k].as_i64().ok_or_else(|| format!("the model's settings lack {k}"));
        let ids = |v: &Value, k: &str| -> Vec<i64> {
            v[k].as_array().into_iter().flatten().filter_map(Value::as_i64).collect()
        };
        let heads = number(&config, "decoder_attention_heads")? as usize;
        let library = find_library(root).ok_or("The model runtime isn't here")?;
        let runtime = Runtime::new(&library, Choice::Cpu).map_err(|e| e.0)?;
        let part = |name: &str| {
            runtime.parts_on_cpu(&dir.join("onnx").join(format!("{name}_quantized.onnx"))).map_err(|e| e.0)
        };
        Ok(Whisper {
            encoder: part("encoder_model")?,
            decoder: part("decoder_model_merged")?,
            text,
            heads,
            head_dim: number(&config, "d_model")? as usize / heads.max(1),
            start: number(&generation, "decoder_start_token_id")?,
            no_times: number(&generation, "no_timestamps_token_id")?,
            end: number(&generation, "eos_token_id")?,
            never: ids(&generation, "suppress_tokens"),
            never_first: ids(&generation, "begin_suppress_tokens"),
        })
    }

    /// The words of up to 30 seconds of sound at `RATE`.
    fn words(&self, audio: &[f32]) -> Result<String, String> {
        let features = Tensor::F32(vec![1, MELS, FRAMES], log_mel(audio));
        let hidden = first(&self.encoder, &[("input_features", &features)])?;
        let past_names: Vec<String> =
            self.decoder.inputs.iter().filter(|n| n.starts_with("past_key_values")).cloned().collect();
        let mut past: HashMap<String, Tensor> = past_names
            .iter()
            .map(|n| (n.clone(), Tensor::F32(vec![1, self.heads, 0, self.head_dim], Vec::new())))
            .collect();
        let logits_name = self.decoder.outputs.first().cloned().unwrap_or_default();
        let mut said: Vec<i64> = Vec::new();
        let mut asking = vec![self.start, self.no_times];
        for step in 0..MAX_TOKENS {
            let ids = Tensor::I64(vec![1, asking.len()], asking.clone());
            let cached = Tensor::Bool(vec![1], vec![step > 0]);
            let mut inputs: Vec<(&str, &Tensor)> =
                vec![("input_ids", &ids), ("encoder_hidden_states", &hidden), ("use_cache_branch", &cached)];
            inputs.extend(past.iter().map(|(name, t)| (name.as_str(), t)));
            // the encoder's part of the past is made once, on the first step
            let wanted = |name: &str| name == logits_name || step == 0 || name.contains(".decoder.");
            let ran = self.decoder.run(&inputs, &wanted).map_err(|e| e.0)?;
            drop(inputs);
            let mut logits = Vec::new();
            let mut shape = Vec::new();
            for (name, s, data) in ran {
                if name == logits_name {
                    (logits, shape) = (data, s);
                } else {
                    past.insert(name.replace("present", "past_key_values"), Tensor::F32(s, data));
                }
            }
            let vocab = *shape.last().unwrap_or(&0);
            if vocab == 0 || logits.len() < vocab {
                return Err("the model's answer had no words".to_string());
            }
            let from = logits.len() - vocab;
            let scores = &mut logits[from..];
            // what it never says: the listed tokens, every time mark, and the marks of its own beyond the end
            for &t in &self.never {
                if let Some(s) = scores.get_mut(t as usize) {
                    *s = f32::NEG_INFINITY;
                }
            }
            for s in scores.iter_mut().skip(self.end as usize + 1) {
                *s = f32::NEG_INFINITY;
            }
            if said.is_empty() {
                for &t in &self.never_first {
                    if let Some(s) = scores.get_mut(t as usize) {
                        *s = f32::NEG_INFINITY;
                    }
                }
            }
            let mut best = 0;
            for (k, s) in scores.iter().enumerate() {
                if *s > scores[best] {
                    best = k;
                }
            }
            let next = best as i64;
            if next == self.end {
                break;
            }
            said.push(next);
            asking = vec![next];
        }
        Ok(self.decode(&said))
    }

    fn decode(&self, ids: &[i64]) -> String {
        decode_with(&self.text, ids, self.end)
    }
}

/// The text of tokens: each token's characters back into bytes (the byte-level vocabulary), the model's own
/// marks (from `end` on) left out.
fn decode_with(text: &HashMap<i64, String>, ids: &[i64], end: i64) -> String {
    let as_char = crate::florence::byte_chars();
    let as_byte: HashMap<char, u8> = as_char.iter().enumerate().map(|(b, ch)| (*ch, b as u8)).collect();
    let bytes: Vec<u8> = ids
        .iter()
        .filter(|i| **i < end)
        .filter_map(|i| text.get(i))
        .flat_map(|t| t.chars())
        .filter_map(|ch| as_byte.get(&ch).copied())
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

static MODEL: Mutex<Option<Arc<Whisper>>> = Mutex::new(None);

fn model() -> Result<Arc<Whisper>, String> {
    let mut m = held(&MODEL);
    if let Some(w) = m.as_ref() {
        return Ok(w.clone());
    }
    let (Some(root), Some(dir)) = (ROOT.get(), folder()) else {
        return Err("The speech model isn't downloaded".into());
    };
    if !ready_in(&dir) {
        return Err("The speech model isn't downloaded yet".into());
    }
    let w = Arc::new(Whisper::load(root, &dir)?);
    *m = Some(w.clone());
    Ok(w)
}

/// The words of sound at `RATE` (a few seconds of a spoken question): (text, seconds it took).
pub fn transcribe(audio: &[f32]) -> Result<(String, f64), String> {
    let began = Instant::now();
    let w = model()?;
    let text = w.words(audio)?;
    Ok((text, began.elapsed().as_secs_f64()))
}

// --------------------------------------------------------------------------- listening

/// This computer's microphone to listen to: its default one, else the first.
fn microphone() -> Option<crate::listen::Interface> {
    let (found, _) = crate::listen::interfaces();
    let mut inputs: Vec<crate::listen::Interface> = found.into_iter().filter(|i| i.kind == "input").collect();
    let at = inputs.iter().position(|i| i.default).unwrap_or(0);
    (!inputs.is_empty()).then(|| inputs.swap_remove(at))
}

struct Listening {
    stop: Arc<AtomicBool>,
    began: Instant,
    heard: std::thread::JoinHandle<Option<Vec<f32>>>,
}

static LISTENING: Mutex<Option<Listening>> = Mutex::new(None);

/// One channel of what the microphone hears until `stop` (or `MAX_SECONDS`), at `RATE`.
fn hear(mic: crate::listen::Interface, stop: Arc<AtomicBool>) -> Option<Vec<f32>> {
    crate::listen::record_while(&mic, MAX_SECONDS, &|| !stop.load(Ordering::SeqCst), RATE, false)
}

/// A spoken question: `start` listening, `stop` and have it made into words ({text, secs}), or `cancel`.
pub fn listen(what: &str) -> Result<Value, String> {
    match what {
        "start" => {
            if !ready() {
                return Err("The speech model isn't downloaded yet.".to_string());
            }
            let mut l = held(&LISTENING);
            if l.is_some() {
                return Ok(json!({"listening": true}));
            }
            let mic = microphone().ok_or("This computer has no microphone the program can listen to.")?;
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let heard = std::thread::Builder::new()
                .name("speech listen".into())
                .spawn(move || hear(mic, flag))
                .map_err(|e| e.to_string())?;
            *l = Some(Listening { stop, began: Instant::now(), heard });
            // the model is loaded meanwhile, so that the words come at once when the owner stops
            std::thread::spawn(|| {
                let _ = model();
            });
            Ok(json!({"listening": true}))
        }
        "stop" | "cancel" => {
            let Some(l) = held(&LISTENING).take() else { return Err("Nothing is being listened to.".to_string()) };
            l.stop.store(true, Ordering::SeqCst);
            let heard = l.heard.join().ok().flatten().ok_or("The microphone couldn't be listened to.")?;
            if what == "cancel" {
                return Ok(json!({"text": ""}));
            }
            let seconds = l.began.elapsed().as_secs_f64().min(MAX_SECONDS);
            let loudest = heard.iter().fold(0f32, |m, v| m.max(v.abs()));
            if heard.len() < RATE / 4 || loudest < 1e-4 {
                return Ok(json!({"text": "", "said": "Nothing was heard."}));
            }
            let (text, secs) = transcribe(&heard)?;
            Ok(json!({"text": text, "secs": filejson::round(secs, 2), "heard": filejson::round(seconds, 1)}))
        }
        _ => Err("listen: start, stop or cancel".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_pins_are_whole() {
        assert_eq!(FILES.iter().map(|f| f.1).sum::<u64>(), SIZE);
        assert!(FILES.iter().all(|f| f.2.len() == 64 && f.2.bytes().all(|b| b.is_ascii_hexdigit())));
        assert_eq!(publisher(), ("http://127.0.0.1:9".to_string(), true), "a test never reaches Hugging Face");
    }

    #[test]
    fn the_mel_filters_cover_the_bands_as_slaney_s_scale_does() {
        let f = filters();
        assert_eq!((f.len(), f[0].len()), (MELS, N_FFT / 2 + 1));
        // each band a triangle: nonzero over a run of bins, the bands in order of frequency
        let peak = |row: &Vec<f32>| row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(k, _)| k).unwrap();
        let peaks: Vec<usize> = f.iter().map(peak).collect();
        assert!(peaks.windows(2).all(|w| w[0] <= w[1]), "{peaks:?}");
        assert!(f.iter().all(|row| row.iter().all(|w| *w >= 0.0) && row.iter().any(|w| *w > 0.0)));
        // below 1 kHz the scale is linear: the low bands are as wide as each other
        let width = |row: &Vec<f32>| row.iter().filter(|w| **w > 0.0).count();
        assert!((width(&f[5]) as i64 - width(&f[6]) as i64).abs() <= 1);
        assert!(width(&f[75]) > 3 * width(&f[5]), "the high bands are wider");
    }

    #[test]
    fn a_tone_s_features_are_highest_in_its_own_band() {
        let tone: Vec<f32> =
            (0..RATE).map(|n| (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / RATE as f32).sin() * 0.5).collect();
        let mel = log_mel(&tone);
        assert_eq!(mel.len(), MELS * FRAMES);
        // the middle of the first second: the loudest band is the one around 1 kHz
        let frame = 50;
        let band = (0..MELS).max_by(|a, b| mel[a * FRAMES + frame].total_cmp(&mel[b * FRAMES + frame])).unwrap();
        let f = filters();
        let centre = |m: usize| {
            f[m].iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0 as f64 * RATE as f64 / N_FFT as f64
        };
        assert!((centre(band) - 1000.0).abs() < 120.0, "band {band} at {} Hz", centre(band));
        // the silence after the tone is the floor: 8 bels under the loudest, scaled
        let top = mel.iter().copied().fold(f32::MIN, f32::max);
        assert!((mel[10 * FRAMES + 2500] - (top - 2.0)).abs() < 1e-4);
    }

    #[test]
    fn tokens_are_read_back_into_words() {
        let mut text = HashMap::new();
        text.insert(1, "Hello".to_string());
        text.insert(2, "\u{120}world".to_string()); // a space is written as Ġ
        text.insert(3, "!".to_string());
        text.insert(50257, "<|startoftranscript|>".to_string());
        assert_eq!(decode_with(&text, &[50257, 1, 2, 3], 50256), "Hello world!");
    }

    #[test]
    fn nothing_is_listened_to_without_the_model() {
        if !ready() {
            assert!(listen("start").is_err());
        }
        assert!(listen("stop").is_err(), "nothing to stop");
        assert!(listen("sing").is_err());
    }
}
