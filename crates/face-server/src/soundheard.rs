//! A stretch of sound, measured: what `listen::listen_all` makes of one audio interface's listening, and the
//! same of what a device's own page heard with its microphone.
//!
//! A computer listens to its audio interfaces itself (`listen.rs`, `soundlive.rs`). A phone's app has no way
//! to (the sound library isn't in it), but its page has the microphone: for the Signals chart the page sends
//! a few seconds of what it heard to its own server, on the phone itself, which measures them here and
//! answers with numbers and one word. So a phone listens for itself (the owner, 2026-11-22: "I want the iOS
//! app to listen to sounds separately from the macOS device even when they are connected"). The sound is in
//! memory while it is measured and is kept nowhere.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde_json::{Value, json};

use crate::sound::{self, Sounds};

/// seconds of sound the page may send to be measured (the measures want two; more than a few is not a reading
/// of "now")
pub const MIN_SECONDS: f64 = 2.0;
pub const MAX_SECONDS: f64 = 8.0;
/// samples a second it may be sent at
pub const RATES: std::ops::RangeInclusive<usize> = 8_000..=48_000;
/// the most bytes of such a stretch: `MAX_SECONDS` of 32-bit numbers at the highest rate
pub const MAX_BYTES: usize = 8 * 48_000 * 4;

/// One stretch of sound, measured: {name, kind, default, heard, sounds, families, ambient, what, sure,
/// background}. heard: `measure`'s numbers as for a song (none when silent); sounds: the kinds of sound the
/// model hears; ambient: the background's numbers; what and sure: those in a word ("music", "speech",
/// "quiet"...) and how sure. `x` is None when nothing could be listened to.
pub fn entry(name: &str, kind: &str, default: bool, x: Option<&[f32]>, rate: usize, model: Option<&Sounds>) -> Value {
    let amb = x.and_then(|x| sound::ambient(x, rate));
    let snd = match (&amb, model, x) {
        (Some(_), Some(model), Some(x)) => model.hear(x, rate, 6),
        _ => None,
    };
    let (word, sure) = sound::what(snd.as_ref(), amb.as_ref());
    // a song's numbers (tempo, key) only where music is what sounds: a fan has no key
    let song =
        x.filter(|_| word == "music" || (kind == "output" && snd.is_none())).and_then(|x| sound::measure(x, rate));
    let families = sound::families(snd.as_ref());
    json!({"name": name, "kind": kind, "default": default, "heard": song, "sounds": snd,
           "families": if families.is_empty() { Value::Null } else { Value::Object(families) }, "ambient": amb, "what": word, "sure": sure,
           "background": if word == "steady background noise" { sound::background(amb.as_ref()) } else { String::new() }})
}

/// One measured stretch as the Signals chart takes it: its name and kind, what was heard in a word and how
/// sure, and the numbers of the background and, where music sounded, of the music. No sound, no list of sounds.
pub fn numbers(heard: &Value) -> Value {
    let n = |v: &Value| v.as_f64().filter(|x| x.is_finite()).map(|x| (x * 1000.0).round() / 1000.0);
    let (room, song) = (&heard["ambient"], &heard["heard"]);
    json!({
        "name": heard["name"], "kind": heard["kind"], "what": heard["what"], "sure": n(&heard["sure"]),
        "level": n(&room["level"]), "floor": n(&room["floor"]), "events": n(&room["events"]),
        "steady": n(&room["steady"]), "flat": n(&room["flat"]), "hum": n(&room["hum"]),
        "speechlike": n(&room["speechlike"]),
        "tempo": n(&song["tempo"]), "pulse": n(&song["pulse"]), "bright": n(&song["bright"]), "bass": n(&song["bass"]),
    })
}

/// What the page heard with the device's own microphone, measured.
pub struct Heard {
    root: PathBuf,
    /// the sound-recognition model: looked for at the first stretch, and then known to be there or not
    model: Mutex<Option<Option<Sounds>>>,
}

impl Heard {
    pub fn new(root: &Path) -> Heard {
        Heard { root: root.to_path_buf(), model: Mutex::new(None) }
    }

    /// `sound`: one channel of 32-bit numbers, low byte first, at `rate` a second: `MIN_SECONDS` to
    /// `MAX_SECONDS` of it. The answer is one reading as the chart takes it (`numbers`).
    pub fn measure(&self, sound: &[u8], rate: usize) -> Result<Value, String> {
        if !RATES.contains(&rate) || sound.len() % 4 != 0 {
            return Err("expected one channel of float32 audio at 8 to 48 kHz".to_string());
        }
        let x: Vec<f32> = sound.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let seconds = x.len() as f64 / rate as f64;
        if !(MIN_SECONDS..=MAX_SECONDS).contains(&seconds) || !x.iter().all(|v| v.is_finite()) {
            return Err("expected 2 to 8 s of audio".to_string());
        }
        let mut model = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let model = model.get_or_insert_with(|| Sounds::file(&self.root).and_then(|_| Sounds::load(&self.root)));
        Ok(numbers(&entry("This device's microphone", "input", true, Some(&x), rate, model.as_ref())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listening_becomes_numbers_and_one_word() {
        let heard = json!({
            "name": "Speakers", "kind": "output", "default": true, "what": "music", "sure": 0.91234,
            "heard": {"level": -20.0, "tempo": 121.96, "pulse": 0.61, "bright": 1800.0, "bass": 0.2, "key": "A", "mode": "minor"},
            "sounds": {"Music": 0.9, "Singing": 0.4}, "families": {"music": 0.9},
            "ambient": {"level": -21.5, "floor": -40.0, "events": 12.0, "steady": 0.3, "flat": 0.1, "hum": 2.0,
                        "low": 0.2, "high": 0.1, "speechlike": 0.2},
            "background": "",
        });
        let n = numbers(&heard);
        assert_eq!(n["name"], "Speakers");
        assert_eq!((n["kind"].as_str(), n["what"].as_str()), (Some("output"), Some("music")));
        assert_eq!(
            (n["sure"].as_f64(), n["level"].as_f64(), n["tempo"].as_f64()),
            (Some(0.912), Some(-21.5), Some(121.96))
        );
        // nothing but the listed numbers: no list of the sounds heard, nothing of the sound itself
        assert!(n.get("sounds").is_none() && n.get("families").is_none() && n.get("heard").is_none());
        // silence: a word, and no numbers
        let quiet = numbers(
            &json!({"name": "Microphone", "kind": "input", "what": "quiet", "sure": 1.0, "heard": null, "ambient": null}),
        );
        assert_eq!(quiet["what"], "quiet");
        assert!(quiet["level"].is_null() && quiet["tempo"].is_null());
    }

    /// seconds of a tone, as the page sends them
    fn tone(seconds: f64, rate: usize, hz: f64, loud: f32) -> Vec<u8> {
        (0..(seconds * rate as f64) as usize)
            .flat_map(|i| {
                (loud * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32).to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn what_the_page_heard_is_measured_and_only_a_sound_of_the_right_shape_is_taken() {
        // (a folder with no models: the background's numbers alone, as on a device without the sound model)
        let heard = Heard::new(&std::env::temp_dir().join(format!("face-heard-{}", std::process::id())));
        let reading = heard.measure(&tone(4.0, 16_000, 440.0, 0.2), 16_000).unwrap();
        assert_eq!(
            (reading["kind"].as_str(), reading["name"].as_str()),
            (Some("input"), Some("This device's microphone"))
        );
        assert!(reading["level"].as_f64().is_some_and(|db| (-30.0..0.0).contains(&db)), "{reading}");
        assert!(reading["what"].is_string() && reading["steady"].is_number());
        assert!(reading.get("sounds").is_none(), "numbers and one word, nothing else");
        // silence is a word, not an error
        let quiet = heard.measure(&vec![0u8; 4 * 16_000 * 3], 16_000).unwrap();
        assert_eq!(quiet["what"], "quiet");
        // too short, too long, a rate no microphone has, half a number, numbers that aren't
        for (sound, rate) in [
            (tone(1.0, 16_000, 440.0, 0.2), 16_000),
            (tone(9.0, 16_000, 440.0, 0.2), 16_000),
            (tone(4.0, 16_000, 440.0, 0.2), 1_000),
            (tone(4.0, 16_000, 440.0, 0.2), 96_000),
            (tone(4.0, 16_000, 440.0, 0.2)[..4 * 16_000 * 4 - 1].to_vec(), 16_000),
            (f32::NAN.to_le_bytes().repeat(16_000 * 4), 16_000),
        ] {
            assert!(heard.measure(&sound, rate).is_err(), "{} bytes at {rate}", sound.len());
        }
    }
}
