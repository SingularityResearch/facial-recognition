//! What the programs make of a sound, and what they hear, as JSON, for a check of it.
//!
//!     music_try measure <file> [root]    a sound's numbers: the file ("-": standard input) holds one channel
//!                                        of 32-bit floats at 22,050 a second; with a root that has
//!                                        models/yamnet.onnx and the model runtime, also the kinds of sound
//!     music_try sounds <rate> <file> <root>  the kinds of sound alone, of a sound at any number of samples a
//!                                        second (at 16,000 the model hears it as it is)
//!     music_try tone <seconds> <dBFS>    play an A (440 Hz) that quiet or loud on the default output, so that
//!                                        a check can tell whether what the computer plays is heard
//!     music_try log <file>               the song log's looks, played from a script: {who, picture, kinds,
//!                                        now_ms, steps: [{now, playing, heard}]}; the rows it would keep
//!     music_try interfaces               this computer's audio interfaces that can be listened to
//!     music_try listen <seconds> [root]  every interface listened to at once, as the music log does
//!
//! Nothing is written anywhere: a sound that is listened to stays in memory and only its numbers are printed.

use std::path::Path;

use face_server::{
    listen,
    songlog::{self, Listener},
    sound::{self, RATE, Sounds},
};
use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let model = |root: Option<&&str>| root.and_then(|root| Sounds::load(Path::new(root)));
    let said = match &words[..] {
        ["measure", file, rest @ ..] => {
            let raw = if *file == "-" {
                let mut raw = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin().lock(), &mut raw)
                    .expect("the sound on standard input");
                raw
            } else {
                std::fs::read(file).expect("the sound's file")
            };
            let x: Vec<f32> = raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            let amb = sound::ambient(&x, RATE);
            let model = model(rest.first());
            let snd = model.as_ref().filter(|_| amb.is_some()).and_then(|m| m.hear(&x, RATE, 6));
            let (what, sure) = sound::what(snd.as_ref(), amb.as_ref());
            let families = sound::families(snd.as_ref());
            json!({"measure": sound::measure(&x, RATE), "ambient": amb, "model": model.is_some(), "sounds": snd,
                   "families": if families.is_empty() { Value::Null } else { Value::Object(families) }, "what": what, "sure": sure,
                   "background": sound::background(amb.as_ref())})
        }
        ["sounds", rate, file, root] => {
            let raw = if *file == "-" {
                let mut raw = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin().lock(), &mut raw)
                    .expect("the sound on standard input");
                raw
            } else {
                std::fs::read(file).expect("the sound's file")
            };
            let x: Vec<f32> = raw.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            let model = Sounds::load(Path::new(root)).expect("the sound-recognition model and the model runtime");
            json!({"sounds": model.hear(&x, rate.parse().unwrap_or(RATE), 12)})
        }
        ["tone", seconds, level] => {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
            let (seconds, level): (f64, f64) = (seconds.parse().unwrap_or(3.0), level.parse().unwrap_or(-50.0));
            let device = cpal::default_host().default_output_device().expect("an output to play on");
            let config = device.default_output_config().expect("how the output wants its sound");
            let (rate, channels, loud) =
                (config.sample_rate() as f32, config.channels() as usize, 10f32.powf(level as f32 / 20.0));
            let mut k = 0u64;
            let play = move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for moment in out.chunks_mut(channels) {
                    moment.fill(loud * (std::f32::consts::TAU * 440.0 * k as f32 / rate).sin());
                    k += 1;
                }
            };
            let stream = device.build_output_stream(config.into(), play, |_| {}, None).expect("a stream to the output");
            stream.play().expect("the tone playing");
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
            json!({"played": seconds, "level": level, "format": "32-bit floats"})
        }
        ["log", file] => {
            let script: Value = serde_json::from_slice(&std::fs::read(file).expect("the script")).expect("a script");
            let kinds: Vec<&str> = script["kinds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let (who, picture, now_ms) = (
                script["who"].as_str().unwrap_or(""),
                script.get("picture").filter(|p| !p.is_null()),
                script["now_ms"].as_i64().unwrap_or(0),
            );
            let (mut listener, mut rows, mut refused) = (Listener::default(), Vec::new(), Vec::new());
            for step in script["steps"].as_array().into_iter().flatten() {
                let heard = step["heard"].as_array().cloned().unwrap_or_default();
                let ended = listener.step(
                    step["now"].as_f64().unwrap_or(0.0),
                    step.get("playing").filter(|p| !p.is_null()),
                    &kinds,
                    &mut |_| heard.clone(),
                );
                if let Some(row) = ended.and_then(|song| songlog::row(&song, who, picture)) {
                    match songlog::check(&row, now_ms) {
                        Ok(row) => rows.push(row),
                        Err(why) => refused.push(why),
                    }
                }
            }
            let current =
                listener.song.as_ref().map(|s| json!({"artist": s.artist, "song": s.song, "heard": s.heard.len()}));
            json!({"rows": rows, "refused": refused, "current": current, "listened": listener.last.is_some()})
        }
        ["interfaces"] => {
            let (found, why) = listen::interfaces();
            json!({"interfaces": found.iter().map(|i| json!({"id": i.id, "name": i.name, "kind": i.kind, "default": i.default})).collect::<Vec<_>>(), "why": why})
        }
        ["listen", seconds, rest @ ..] => {
            let model = model(rest.first());
            json!({"model": model.is_some(), "heard": listen::listen_all(seconds.parse().unwrap_or(4.0), &["output", "input"], model.as_ref())})
        }
        _ => {
            eprintln!("music_try measure <file> [root] | interfaces | listen <seconds> [root] | tone <seconds> <dBFS>");
            std::process::exit(2);
        }
    };
    println!("{said}");
}
