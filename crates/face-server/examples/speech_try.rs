//! Analyze's speech model tried on recordings, for measuring it on a computer: each WAV file's words and how long
//! making them took. The model's files are read from `<root>/models/whisper-base.en` as the app keeps them.
//!
//!     speech_try <root> <file.wav>...      (16-bit PCM, any rate, mono or stereo)
//!
//! Prints the words and the seconds only; nothing is written.

use std::path::Path;

/// A WAV file's sound as one channel of numbers, and its rate.
fn wav(path: &Path) -> Result<(Vec<f32>, usize), String> {
    let b = std::fs::read(path).map_err(|e| e.to_string())?;
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut at, mut channels, mut rate, mut bits) = (12usize, 1usize, 16_000usize, 16u16);
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = u32::from_le_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]) as usize;
        let body = &b[at + 8..(at + 8 + size).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]).max(1) as usize;
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
            bits = u16::from_le_bytes([body[14], body[15]]);
        }
        if id == b"data" {
            if bits != 16 {
                return Err("only 16-bit PCM".into());
            }
            let samples: Vec<f32> = body
                .chunks_exact(2 * channels)
                .map(|frame| {
                    frame.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0).sum::<f32>()
                        / channels as f32
                })
                .collect();
            return Ok((samples, rate));
        }
        at += 8 + size + (size & 1);
    }
    Err("no sound in the file".into())
}

fn main() -> Result<(), String> {
    face_core::runtime::no_telemetry();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, files) = args.split_first().ok_or("usage: speech_try <root> <file.wav>...")?;
    face_server::speech::set_root(Path::new(root));
    if !face_server::speech::ready() {
        return Err("the speech model isn't in <root>/models/whisper-base.en".into());
    }
    for f in files {
        let (sound, rate) = wav(Path::new(f))?;
        let at_rate = face_server::sound::resample(&sound, rate, face_server::speech::RATE);
        let (words, secs) = face_server::speech::transcribe(&at_rate)?;
        println!(
            "{:.2} s for {:.1} s of sound: {words}",
            secs,
            at_rate.len() as f64 / face_server::speech::RATE as f64
        );
    }
    Ok(())
}
