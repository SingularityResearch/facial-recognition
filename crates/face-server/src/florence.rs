//! The built-in photo describer, used by Describe photo when no describe server is there.
//!
//! Microsoft's Florence-2 large (MIT licence), as exported to ONNX by onnx-community, 8-bit, run on the
//! processor with the model runtime: about 0.8 GB in seven files, downloaded only when the owner asks for
//! it, from Hugging Face at one pinned revision, each file kept only when it has the pinned size and
//! SHA-256. A file already there counts only with that SHA-256 too (read once for the file as it is:
//! `.verified.json` beside the files). It writes a caption and
//! reads text; the answer has a describe server's shape: a few sentences, then "Text: ...".
//!
//! The tokenizer (BART's byte-level BPE) and the generation (greedy, with the model's rule that no three
//! tokens repeat) are written out here: nothing beyond the model runtime is
//! needed.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Instant,
};

use face_core::{
    image::Bgr,
    runtime::{Choice, Parts, Runtime, Tensor, find_library},
};
use face_files::{filejson, paths::atomic_write};
use serde_json::{Map, Value, json};

pub const NAME: &str = "Florence-2 large (built-in)";
const REPO: &str = "onnx-community/Florence-2-large-ft";
const REV: &str = "04ace74913c28d7ec94af32cee5c111a10126b6f";
/// the model's files: (name, size in bytes, SHA-256)
pub const FILES: [(&str, u64, &str); 7] = [
    ("vocab.json", 798_293, "ed19656ea1707df69134c4af35c8ceda2cc9860bf2c3495026153a133670ab5e"),
    ("merges.txt", 456_318, "1ce1664773c50f3e0cc8842619a93edc4624525b728b188a9e0be33b7726adc5"),
    ("added_tokens.json", 22_410, "ba67457c577895525996e7a42d7da60a05856506ea4fd25ff9a023a64662f74b"),
    (
        "onnx/vision_encoder_quantized.onnx",
        366_371_802,
        "0dbf2172ef1dc6fbb4da0f02498ef9e3a009bf84e8f2e0d971f39cc5b7e053ce",
    ),
    (
        "onnx/embed_tokens_quantized.onnx",
        52_520_480,
        "dab1d09ab449f1302df6e439b35ec9e93fb037134bf6bed9ffeaa93a5b35d43c",
    ),
    (
        "onnx/encoder_model_quantized.onnx",
        152_985_855,
        "9fe30b02cc8c2247631f8f0b9bc4d6613df1c33da0afe4087abff2f940bc2fe0",
    ),
    (
        "onnx/decoder_model_merged_quantized.onnx",
        257_619_824,
        "167e7807fb1e5b8d693138f84bc81e1368ea11833264131c1ea96eafc9d13222",
    ),
];
/// all of it, in bytes
pub const SIZE: u64 = {
    let (mut sum, mut k) = (0, 0);
    while k < FILES.len() {
        sum += FILES[k].1;
        k += 1;
    }
    sum
};
const CAPTION: &str = "Describe with a paragraph what is shown in the image.";
const OCR: &str = "What is the text in the image?";
/// tokens per answer
const MAX_NEW: usize = 256;
/// no three tokens repeat, as in the model's own generation settings
const NGRAM: usize = 3;
/// the decoder's attention heads and the width of each (the files are pinned: this is that model)
const HEADS: usize = 16;
const HEAD: usize = 64;
/// the picture's side for the model, and how its colours are brought to the model's range
const SIDE: usize = 768;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
/// in the model's folder: {file: {size, mtime_ns}} of the files whose SHA-256 was checked
const VERIFIED: &str = ".verified.json";

/// where the programs' `models` folder is (set once, by the labeling tool)
static ROOT: OnceLock<PathBuf> = OnceLock::new();
/// one reading of a file at a time (the page asks for the status again meanwhile)
static VERIFY: Mutex<()> = Mutex::new(());
/// the download, for the page
static STATE: Mutex<State> = Mutex::new(State { running: false, done: 0, error: None });
/// the loaded model: one description at a time (the model uses every core)
static MODEL: Mutex<Option<Florence>> = Mutex::new(None);

struct State {
    running: bool,
    done: u64,
    error: Option<String>,
}

fn held<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|e| e.into_inner())
}

/// Say where the programs' folder is: the model's files are in `<root>/models/florence2`.
pub fn set_root(root: &Path) {
    let _ = ROOT.set(root.to_path_buf());
}

fn folder() -> Option<PathBuf> {
    ROOT.get().map(|root| root.join("models").join("florence2"))
}

fn checked(dir: &Path) -> Map<String, Value> {
    let read = std::fs::read(dir.join(VERIFIED)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    match read {
        Some(Value::Object(known)) => known,
        _ => Map::new(),
    }
}

/// A file as it is now: its size and when it was last written, to the nanosecond.
fn stamp(path: &Path) -> Option<(u64, Value)> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let written = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((meta.len(), json!({"size": meta.len(), "mtime_ns": written})))
}

/// Remember that `dir/name`, as it is now, has the expected SHA-256.
fn note(dir: &Path, name: &str) {
    if let Some((_, now)) = stamp(&dir.join(name)) {
        let mut known = checked(dir);
        known.insert(name.to_string(), now);
        // (a folder that can't be written: checked again next time)
        let _ = atomic_write(&dir.join(VERIFIED), filejson::dumps_indent1(&Value::Object(known)).as_bytes());
    }
}

/// Whether `dir/name` is the expected file: its size and its SHA-256. The file is read only when its size
/// or time differ from the last check.
fn good(dir: &Path, name: &str, known: Option<&Map<String, Value>>) -> bool {
    let Some(&(_, size, sha)) = FILES.iter().find(|f| f.0 == name) else { return false };
    let path = dir.join(name);
    let Some((now_size, now)) = stamp(&path) else { return false };
    if now_size != size {
        return false;
    }
    if known.map_or_else(|| checked(dir).get(name) == Some(&now), |k| k.get(name) == Some(&now)) {
        return true;
    }
    let _one = held(&VERIFY);
    if checked(dir).get(name) == Some(&now) {
        return true; // read by another request while this one waited
    }
    if !sha256_is(&path, sha) {
        return false;
    }
    note(dir, name);
    true
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

fn ready_in(dir: &Path) -> bool {
    let known = checked(dir);
    FILES.iter().all(|f| good(dir, f.0, Some(&known)))
}

/// Whether the built-in model is here, every file the pinned one.
pub fn ready() -> bool {
    folder().is_some_and(|dir| ready_in(&dir))
}

/// For the page: is the built-in model here, or being downloaded.
pub fn status() -> Value {
    let ready = ready();
    let st = held(&STATE);
    json!({"name": NAME, "ready": ready, "size_mb": (SIZE as f64 / 1e6).round() as u64, "running": st.running, "done": st.done,
           "error": st.error})
}

/// Where the files are asked for: Hugging Face, or a stand-in on this computer in a check
/// (`FACE_TEST_HUGGINGFACE`, which counts only as `http://127.0.0.1:<port>`). In this crate's own tests it
/// is never Hugging Face: a test that starts the download reaches nothing.
fn publisher() -> (String, bool) {
    if cfg!(test) {
        return ("http://127.0.0.1:9".to_string(), true);
    }
    match stand_in(std::env::var("FACE_TEST_HUGGINGFACE").ok().as_deref()) {
        Some(at) => (at, true),
        None => ("https://huggingface.co".to_string(), false),
    }
}

/// An address that may stand in for the publisher: this computer's own, by number, with a port.
fn stand_in(said: Option<&str>) -> Option<String> {
    let port = said?.strip_prefix("http://127.0.0.1:")?;
    (!port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())).then(|| said.unwrap_or_default().to_string())
}

/// Fetch the model's files that are missing, into `dir` (progress in the state the page reads).
#[cfg(feature = "fetch")]
fn download(dir: &Path) -> Result<(), String> {
    use face_tools::fetch::{Net, save};
    {
        let mut st = held(&STATE);
        (st.running, st.done, st.error) = (true, 0, None);
    }
    let (base, local) = publisher();
    let net = if local { Net::plain() } else { Net::new() };
    let fetched = (|| -> Result<(), String> {
        for (name, size, sha) in FILES {
            if good(dir, name, None) {
                held(&STATE).done += size; // there already, and the expected file (not just as large)
                continue;
            }
            let start = held(&STATE).done;
            let got = net.open(&format!("{base}/{REPO}/resolve/{REV}/{name}"), None, None).and_then(|from| {
                save(from, &dir.join(name), size, sha, &mut |so_far| held(&STATE).done = start + so_far)
            });
            if let Err(e) = got {
                held(&STATE).done = start;
                return Err(e.why);
            }
            note(dir, name);
        }
        Ok(())
    })();
    let mut st = held(&STATE);
    st.running = false;
    fetched.map_err(|e| {
        let why = format!("Couldn't download the built-in model: {e}");
        st.error = Some(why.clone());
        why
    })
}

#[cfg(not(feature = "fetch"))]
fn download(_: &Path) -> Result<(), String> {
    let why = "Couldn't download the built-in model: this program was built without its downloads".to_string();
    let mut st = held(&STATE);
    (st.running, st.error) = (false, Some(why.clone()));
    Err(why)
}

/// Start the download on a thread of its own; two quick clicks start one, not two into the same files.
pub fn download_in_background() {
    let Some(dir) = folder() else { return };
    {
        let mut st = held(&STATE);
        if st.running {
            return;
        }
        st.running = true;
    }
    std::thread::spawn(move || {
        let _ = download(&dir); // the reason is in the state the page reads
    });
}

// --------------------------------------------------------------------------- the tokenizer (byte-level BPE)

/// Each byte as the character the vocabulary writes it with: the printable ones as themselves, the rest
/// as the characters from 256 on.
fn byte_chars() -> [char; 256] {
    let own = |b: u32| (33..=126).contains(&b) || (161..=172).contains(&b) || (174..=255).contains(&b);
    let mut out = ['\0'; 256];
    let mut n = 0;
    for b in 0..256u32 {
        out[b as usize] = if own(b) {
            char::from_u32(b).unwrap_or('\0')
        } else {
            n += 1;
            char::from_u32(255 + n).unwrap_or('\0')
        };
    }
    out
}

/// A text cut into the pieces the tokenizer takes one at a time, by the tokenizer's own pattern:
/// `'s|'t|'re|'ve|'m|'ll|'d| ?letters| ?digits| ?other signs|spaces before more spaces or the end|spaces`.
fn words(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let letter = |ch: char| ch.is_alphabetic();
    let digit = |ch: char| ch.is_numeric();
    let sign = |ch: char| !ch.is_whitespace() && !ch.is_alphanumeric() && ch != '_';
    let run = |from: usize, is: &dyn Fn(char) -> bool| c[from..].iter().take_while(|ch| is(**ch)).count();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < c.len() {
        let rest: String = c[i..].iter().take(3).collect();
        let mut len =
            ["'s", "'t", "'re", "'ve", "'m", "'ll", "'d"].iter().find(|s| rest.starts_with(**s)).map_or(0, |s| s.len());
        if len == 0 {
            let lead = usize::from(c[i] == ' ');
            for is in [&letter as &dyn Fn(char) -> bool, &digit, &sign] {
                // (with the space taken first, then without it, as the pattern tries them)
                for lead in [lead, 0] {
                    let n = if i + lead < c.len() { run(i + lead, is) } else { 0 };
                    if len == 0 && n > 0 {
                        len = lead + n;
                    }
                }
            }
        }
        if len == 0 {
            let spaces = run(i, &|ch: char| ch.is_whitespace());
            // spaces that more text follows leave their last one to it, when they are more than one
            len = if spaces > 1 && i + spaces < c.len() { spaces - 1 } else { spaces };
        }
        if len == 0 {
            i += 1; // (an underscore alone: no piece)
            continue;
        }
        out.push(c[i..i + len].iter().collect());
        i += len;
    }
    out
}

struct Tokenizer {
    vocab: HashMap<String, i64>,
    /// a token's text by its number
    text: HashMap<i64, String>,
    /// the tokens that are no text: the start, the end, the padding, and the place marks
    skip: HashSet<i64>,
    /// the merges, best first
    ranks: HashMap<(String, String), usize>,
    as_char: [char; 256],
    as_byte: HashMap<char, u8>,
}

impl Tokenizer {
    fn load(dir: &Path) -> Result<Tokenizer, String> {
        let numbers = |name: &str| -> Result<HashMap<String, i64>, String> {
            let raw = std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
            serde_json::from_slice(&raw).map_err(|e| format!("{name}: {e}"))
        };
        let (vocab, added) = (numbers("vocab.json")?, numbers("added_tokens.json")?);
        let mut skip: HashSet<i64> = added.values().copied().collect();
        skip.extend([0, 1, 2, 3]);
        let mut text: HashMap<i64, String> = vocab.iter().map(|(k, v)| (*v, k.clone())).collect();
        text.extend(added.iter().map(|(k, v)| (*v, k.clone())));
        let merges = std::fs::read_to_string(dir.join("merges.txt")).map_err(|e| format!("merges.txt: {e}"))?;
        let mut ranks = HashMap::new();
        for (k, line) in merges.lines().skip(1).enumerate() {
            let mut pair = line.split_whitespace();
            if let (Some(a), Some(b)) = (pair.next(), pair.next()) {
                ranks.insert((a.to_string(), b.to_string()), k);
            }
        }
        let as_char = byte_chars();
        let as_byte = as_char.iter().enumerate().map(|(b, ch)| (*ch, b as u8)).collect();
        Ok(Tokenizer { vocab, text, skip, ranks, as_char, as_byte })
    }

    /// One piece's characters merged into the vocabulary's parts: the best-ranked pair first, one at a time.
    fn bpe(&self, word: &str) -> Vec<String> {
        let mut parts: Vec<String> = word.chars().map(String::from).collect();
        while parts.len() > 1 {
            let best = (0..parts.len() - 1)
                .filter_map(|i| self.ranks.get(&(parts[i].clone(), parts[i + 1].clone())).map(|rank| (*rank, i)))
                .min();
            let Some((_, i)) = best else { break };
            let joined = format!("{}{}", parts[i], parts[i + 1]);
            parts.splice(i..i + 2, [joined]);
        }
        parts
    }

    fn encode(&self, text: &str) -> Result<Vec<i64>, String> {
        let mut ids = vec![0];
        for word in words(text) {
            let written: String = word.bytes().map(|b| self.as_char[b as usize]).collect();
            for part in self.bpe(&written) {
                ids.push(*self.vocab.get(&part).ok_or("the model's vocabulary lacks a piece of the question")?);
            }
        }
        ids.push(2);
        Ok(ids)
    }

    fn decode(&self, ids: &[i64]) -> String {
        let bytes: Vec<u8> = (ids.iter().filter(|i| !self.skip.contains(i)))
            .filter_map(|i| self.text.get(i))
            .flat_map(|t| t.chars())
            .filter_map(|ch| self.as_byte.get(&ch).copied())
            .collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }
}

// --------------------------------------------------------------------------- the model

struct Florence {
    vision: Parts,
    embed: Parts,
    encoder: Parts,
    decoder: Parts,
    tokens: Tokenizer,
}

/// The first output of a run, as a tensor.
fn first(part: &Parts, inputs: &[(&str, &Tensor)]) -> Result<Tensor, String> {
    let wanted = part.outputs.first().cloned().unwrap_or_default();
    let mut out = part.run(inputs, &|name| name == wanted).map_err(|e| e.0)?;
    let (_, shape, data) = out.pop().ok_or("the model gave no answer")?;
    Ok(Tensor::F32(shape, data))
}

impl Florence {
    fn load(root: &Path, dir: &Path) -> Result<Florence, String> {
        let library = find_library(root).ok_or("The model runtime isn't here")?;
        let runtime = Runtime::new(&library, Choice::Cpu).map_err(|e| e.0)?;
        let part = |name: &str| {
            runtime.parts_on_cpu(&dir.join("onnx").join(format!("{name}_quantized.onnx"))).map_err(|e| e.0)
        };
        Ok(Florence {
            vision: part("vision_encoder")?,
            embed: part("embed_tokens")?,
            encoder: part("encoder_model")?,
            decoder: part("decoder_model_merged")?,
            tokens: Tokenizer::load(dir)?,
        })
    }

    /// What the model sees in a picture.
    fn features(&self, picture: &Bgr) -> Result<Tensor, String> {
        let small = picture.resize_cubic_float(SIDE, SIDE);
        let mut x = vec![0f32; 3 * SIDE * SIDE];
        for (at, px) in small.px.as_chunks::<3>().0.iter().enumerate() {
            // (red first for the model)
            for c in 0..3 {
                x[c * SIDE * SIDE + at] = (px[2 - c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
        first(&self.vision, &[("pixel_values", &Tensor::F32(vec![1, 3, SIDE, SIDE], x))])
    }

    /// The model's answer to one question about the picture: greedy, with its no-repeat rule (its own
    /// settings ask for 3 beams; greedy was close when the two were compared, and 3 times cheaper).
    fn answer(&self, features: &Tensor, question: &str) -> Result<String, String> {
        let Tensor::F32(seen_shape, seen) = features else {
            return Err("the picture's features aren't numbers".into());
        };
        let ids = self.tokens.encode(question)?;
        let asked = first(&self.embed, &[("input_ids", &Tensor::I64(vec![1, ids.len()], ids.clone()))])?;
        let Tensor::F32(asked_shape, asked) = asked else { unreachable!() };
        let (long, wide) = (seen_shape[1] + asked_shape[1], seen_shape[2]);
        let both = Tensor::F32(vec![1, long, wide], [seen.as_slice(), asked.as_slice()].concat());
        let mask = Tensor::I64(vec![1, long], vec![1; long]);
        let hidden = first(&self.encoder, &[("inputs_embeds", &both), ("attention_mask", &mask)])?;
        let past_names: Vec<&String> =
            self.decoder.inputs.iter().filter(|n| n.starts_with("past_key_values")).collect();
        // (before the first step there is no past: nothing of each, which the model's first branch never reads)
        let mut past: HashMap<String, Tensor> =
            past_names.iter().map(|n| ((*n).clone(), Tensor::F32(vec![1, HEADS, 0, HEAD], Vec::new()))).collect();
        let logits_name = self.decoder.outputs.first().cloned().unwrap_or_default();
        let mut out: Vec<i64> = vec![2];
        for step in 0..MAX_NEW {
            let last = Tensor::I64(vec![1, 1], vec![*out.last().unwrap_or(&2)]);
            let x = first(&self.embed, &[("input_ids", &last)])?;
            let cached = Tensor::Bool(vec![1], vec![step > 0]);
            let mut inputs: Vec<(&str, &Tensor)> = vec![
                ("inputs_embeds", &x),
                ("encoder_hidden_states", &hidden),
                ("encoder_attention_mask", &mask),
                ("use_cache_branch", &cached),
            ];
            inputs.extend(past.iter().map(|(name, t)| (name.as_str(), t)));
            // the encoder's part of the past is computed once, on the first step
            let wanted = |name: &str| name == logits_name || step == 0 || name.contains(".decoder.");
            let ran = self.decoder.run(&inputs, &wanted).map_err(|e| e.0)?;
            drop(inputs);
            let mut logits = Vec::new();
            for (name, shape, data) in ran {
                if name == logits_name {
                    logits = data;
                } else {
                    past.insert(name.replace("present", "past_key_values"), Tensor::F32(shape, data));
                }
            }
            let next = if step == 0 {
                0 // the start is given
            } else {
                let scores = &mut logits[..];
                if out.len() >= NGRAM {
                    let before = &out[out.len() - (NGRAM - 1)..];
                    for j in 0..=out.len() - NGRAM {
                        if &out[j..j + NGRAM - 1] == before
                            && let Some(score) = scores.get_mut(out[j + NGRAM - 1] as usize)
                        {
                            *score = f32::NEG_INFINITY;
                        }
                    }
                }
                // (the first of the highest)
                let mut best = 0;
                for (k, score) in scores.iter().enumerate() {
                    if *score > scores[best] {
                        best = k;
                    }
                }
                best as i64
            };
            out.push(next);
            if next == 2 && step > 0 {
                break;
            }
        }
        Ok(self.tokens.decode(&out))
    }
}

/// Describe one JPEG: {text, model, secs}, as a describe server's answer is kept.
pub fn describe(jpeg: &[u8]) -> Result<Value, String> {
    let (Some(root), Some(dir)) = (ROOT.get(), folder()) else {
        return Err("The built-in model isn't downloaded".into());
    };
    if !ready_in(&dir) {
        return Err("The built-in model isn't downloaded".to_string());
    }
    let picture = face_import::decode::load(jpeg).map_err(|_| "unreadable picture".to_string())?;
    let mut model = held(&MODEL);
    let began = Instant::now();
    if model.is_none() {
        *model = Some(Florence::load(root, &dir)?);
    }
    let florence = model.as_ref().ok_or("the model isn't loaded")?;
    let seen = florence.features(&picture)?;
    let caption = florence.answer(&seen, CAPTION)?;
    let read = florence.answer(&seen, OCR)?;
    let mut read = read.split_whitespace().collect::<Vec<_>>().join(" ");
    if read.chars().filter(|c| *c != ' ').count() < 3 {
        read = "none".to_string(); // a stray letter or digit: there's no text
    }
    Ok(
        json!({"text": format!("{caption}\nText: {read}"), "model": NAME, "secs": filejson::round(began.elapsed().as_secs_f64(), 1)}),
    )
}

/// For a check: a JPEG as the model is given it, in numbers: the decoded picture, the picture at the
/// model's size, and what the model's eye makes of it (each by its sum and its first values).
pub fn seen_of(jpeg: &[u8]) -> Result<Value, String> {
    let (Some(root), Some(dir)) = (ROOT.get(), folder()) else { return Err("no folder".into()) };
    let picture = face_import::decode::load(jpeg).map_err(|_| "unreadable picture".to_string())?;
    let small = picture.resize_cubic_float(SIDE, SIDE);
    let sum = |px: &[u8]| px.iter().map(|v| *v as u64).sum::<u64>();
    let mut model = held(&MODEL);
    if model.is_none() {
        *model = Some(Florence::load(root, &dir)?);
    }
    let Tensor::F32(shape, values) = model.as_ref().ok_or("the model isn't loaded")?.features(&picture)? else {
        return Err("no features".into());
    };
    Ok(json!({"decoded": {"w": picture.w, "h": picture.h, "sum": sum(&picture.px)},
              "small": {"sum": sum(&small.px), "bytes": small.px},
              "features": {"shape": shape, "values": values}}))
}

/// For a check: the pieces a text is cut into and its tokens, and those tokens as text again.
pub fn tokens_of(text: &str) -> Result<Value, String> {
    let dir = folder().ok_or("no folder")?;
    let tokens = Tokenizer::load(&dir)?;
    let ids = tokens.encode(text)?;
    Ok(json!({"words": words(text), "ids": ids, "text": tokens.decode(&ids)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_cut_as_the_tokenizers_pattern_cuts_it() {
        // (fixed: the model was trained on text cut this way)
        let cut = |t: &str| words(t);
        assert_eq!(
            cut(CAPTION),
            ["Describe", " with", " a", " paragraph", " what", " is", " shown", " in", " the", " image", "."]
        );
        assert_eq!(cut(OCR), ["What", " is", " the", " text", " in", " the", " image", "?"]);
        assert_eq!(cut("it's 42 x_y  ok!! "), ["it", "'s", " 42", " x", "y", " ", " ok", "!!", " "]);
        assert_eq!(cut("a   b"), ["a", "  ", " b"]);
        assert_eq!(cut("they'll we've I'm"), ["they", "'ll", " we", "'ve", " I", "'m"]);
        assert_eq!(cut(""), Vec::<String>::new());
    }

    #[test]
    fn bytes_are_written_as_the_vocabulary_writes_them() {
        let as_char = byte_chars();
        assert_eq!(as_char[b'a' as usize], 'a');
        assert_eq!(as_char[b' ' as usize], '\u{120}', "a space is the vocabulary's G with a dot");
        assert_eq!(as_char[0], '\u{100}');
        assert_eq!(as_char[127], '\u{121}');
        assert_eq!(as_char[173], '\u{143}');
        assert_eq!(as_char[255], '\u{ff}');
        let distinct: HashSet<char> = as_char.iter().copied().collect();
        assert_eq!(distinct.len(), 256);
    }

    #[test]
    fn the_files_pins_are_whole() {
        assert_eq!(SIZE, 830_774_982);
        assert_eq!((SIZE as f64 / 1e6).round() as u64, 831);
        assert!(FILES.iter().all(|f| f.2.len() == 64));
    }

    #[test]
    fn a_file_counts_only_with_its_size_and_sha256_and_is_read_once() {
        let dir = std::env::temp_dir().join(format!("face-florence-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // no file, a file of another size: not it, and nothing is remembered
        assert!(!good(&dir, "vocab.json", None));
        std::fs::write(dir.join("vocab.json"), b"short").unwrap();
        assert!(!good(&dir, "vocab.json", None));
        // the right size and other bytes: read, and not it
        std::fs::write(dir.join("vocab.json"), vec![b'x'; 798_293]).unwrap();
        assert!(!good(&dir, "vocab.json", None));
        assert!(checked(&dir).is_empty());
        // a file the list has as checked, at this size and time, is not read again
        let (_, now) = stamp(&dir.join("vocab.json")).unwrap();
        let mut known = Map::new();
        known.insert("vocab.json".to_string(), now);
        std::fs::write(dir.join(VERIFIED), filejson::dumps_indent1(&Value::Object(known))).unwrap();
        assert!(good(&dir, "vocab.json", None));
        assert!(!good(&dir, "not one of them", None));
        assert!(!ready_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_address_for_the_files_counts_only_on_this_computer() {
        assert_eq!(stand_in(Some("http://127.0.0.1:8123")).as_deref(), Some("http://127.0.0.1:8123"));
        for other in [
            "https://127.0.0.1:8123",
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://127.0.0.1:80/x",
            "http://localhost:8123",
            "http://127.0.0.1.example.org:80",
            "http://192.168.1.5:8123",
            "",
        ] {
            assert_eq!(stand_in(Some(other)), None, "{other}");
        }
        assert_eq!(stand_in(None), None);
        // and a test of this crate never reaches the publisher
        assert!(publisher().0.starts_with("http://127.0.0.1:"));
    }
}
