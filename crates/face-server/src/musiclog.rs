//! The music log as Settings shows and changes it (without the listening):
//! the switch, whose music this device's is, what a music app shows as playing now, Spotify's connection, and a
//! person's usual music made from their liked songs. The usual music is kept as the person's signal of kind
//! "music", so it is shared, exported and forgotten like their other normals.
//!
//! The thread that listens and logs each song when it ends is started here (`start_music`): it does nothing
//! until the owner switches the log on, with a shared database to keep it in and a person named whose music
//! this device's is. What it does at each look is `songlog.rs`.

use std::sync::{Arc, Mutex};

use face_home::waiting::MUSIC;
use serde_json::{Map, Value, json};

use crate::{
    http::Response,
    label::{Answer, Labeler, asked},
    listen, music,
    songlog::{self, Listener},
    sound::Sounds,
    spotify::{self, Places},
};

const OPTION: &str = "fl-music";
const WHO: &str = "fl-musicWho";
const LISTEN: &str = "fl-musicListen";

/// The routes of Music's settings that are sent something.
pub(crate) const POSTS: [&str; 3] = ["/api/music/spotify", "/api/music/liked", "/api/music/disconnect"];

/// What the listening thread last knew, for Settings: the song that plays ({artist, song, heard}), and the
/// newest listening (when, and what every interface heard).
type Known = (Option<Value>, Option<(f64, Vec<Value>)>);
static KNOWN: Mutex<Known> = Mutex::new((None, None));

fn seconds_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// How the one Spotify job goes: (idle | connecting | reading | done | failed, what to say).
static JOB: Mutex<(&str, String)> = Mutex::new(("idle", String::new()));

fn job_says(text: String) {
    JOB.lock().unwrap_or_else(|e| e.into_inner()).1 = text;
}

/// Run one Spotify job at a time on a thread of its own; `JOB` says how it goes.
fn job(
    state: &'static str,
    text: &str,
    target: impl FnOnce() -> Result<String, String> + Send + 'static,
) -> Result<(), String> {
    {
        let mut now = JOB.lock().unwrap_or_else(|e| e.into_inner());
        if now.0 == "connecting" || now.0 == "reading" {
            return Err("Spotify is being asked already".to_string());
        }
        *now = (state, text.to_string());
    }
    let run = move || {
        let ended = match target() {
            Ok(text) => ("done", if text.is_empty() { "Done.".to_string() } else { text }),
            Err(why) => ("failed", why.chars().take(300).collect()),
        };
        *JOB.lock().unwrap_or_else(|e| e.into_inner()) = ended;
    };
    std::thread::Builder::new().name("spotify".into()).spawn(run).map(|_| ()).map_err(|e| e.to_string())
}

/// A number with its thousands apart ("12,345").
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Labeler {
    /// The person whose music this device's is (the owner's setting), or "".
    fn music_who(&self) -> String {
        let name: String = self.store.setting("label", WHO).unwrap_or_default().trim().chars().take(100).collect();
        if name == "Unknown" || name.starts_with('#') { String::new() } else { name }
    }

    /// Songs are logged only with a shared database to keep them in, the switch on, and a person named.
    fn music_logging(&self) -> bool {
        matches!(face_db::Settings::load(&self.home), Ok(Some(_)))
            && self.store.setting("label", OPTION).as_deref() == Some("1")
            && !self.music_who().is_empty()
    }

    /// Which interfaces are listened to: every one (the default), the outputs only, or none.
    fn listen_kinds(&self) -> Vec<&'static str> {
        match self.store.setting("label", LISTEN).as_deref() {
            Some("outputs") => vec!["output"],
            Some("none") => Vec::new(),
            _ => vec!["output", "input"],
        }
    }

    /// Start the listening thread (the labeling tool does, at its start). Every few seconds it looks at what
    /// a music app shows as playing and, when a listening is due, listens to every interface at once; a
    /// song that ended is kept until the sync sends it. It does nothing while the log is switched off.
    pub fn start_music(self: &Arc<Self>) {
        let me = self.clone();
        let root = self.data.parent().unwrap_or(&self.data).to_path_buf();
        let run = move || {
            let (mut listener, mut model) = (Listener::default(), None::<Sounds>);
            loop {
                std::thread::sleep(std::time::Duration::from_secs_f64(songlog::LOOK_EVERY));
                if !me.music_logging() {
                    listener.off();
                    *KNOWN.lock().unwrap_or_else(|e| e.into_inner()) = (None, None);
                    continue;
                }
                let (now, kinds) = (seconds_now(), me.listen_kinds());
                let ended = listener.step(now, music::now_playing().as_ref(), &kinds, &mut |kinds| {
                    // (the sound-recognition model is loaded at the first listening that finds it there)
                    if model.is_none() && Sounds::file(&root).is_some() {
                        model = Sounds::load(&root);
                    }
                    listen::listen_all(songlog::LISTEN_FOR, kinds, model.as_ref())
                });
                if let Some(song) = ended {
                    let who = me.music_who();
                    let picture = me.store.of(&who).ok().and_then(|mut kinds| kinds.remove("music"));
                    let kept = songlog::row(&song, &who, picture.as_ref())
                        .map(|row| songlog::check(&row, (now * 1000.0) as i64));
                    match kept {
                        // (a forgotten person's songs are not kept)
                        Some(Ok(row)) if !me.store.forgotten().contains(&who) => {
                            if let Err(why) = MUSIC.add(&me.home, &[row]) {
                                eprintln!("Music log: {why}");
                            }
                        }
                        Some(Err(why)) => eprintln!("Music log: {why}"),
                        _ => {}
                    }
                }
                let current =
                    listener.song.as_ref().map(|s| json!({"artist": s.artist, "song": s.song, "heard": s.heard.len()}));
                *KNOWN.lock().unwrap_or_else(|e| e.into_inner()) = (current, listener.last.clone());
            }
        };
        if let Err(why) = std::thread::Builder::new().name("music log".into()).spawn(run) {
            eprintln!("The music log's listening didn't start: {why}");
        }
    }

    /// `/api/music`: what Settings shows: the switch, whose music, what plays now, Spotify, the picture.
    pub(crate) fn music_status(&self) -> Value {
        // (how the job goes first: a job that is said to be done has then written what is read after it)
        let job = JOB.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let name = self.music_who();
        let pic = if name.is_empty() {
            None
        } else {
            self.store.of(&name).ok().and_then(|mut kinds| kinds.remove("music")).filter(Value::is_object)
        };
        let sp = match spotify::status(&self.home) {
            Ok(mut sp) => {
                sp["redirect"] = json!(spotify::redirect_uri());
                sp
            }
            Err(why) => {
                json!({"connected": false, "client_id": "", "error": why.chars().take(200).collect::<String>(), "redirect": ""})
            }
        };
        // (today's file holds today's songs, sent or not; an older one only while some of it isn't sent)
        let today: String = face_files::paths::utc_now().chars().take(10).collect();
        let counts = MUSIC.days(&self.home);
        let now = counts.iter().find(|(day, _)| *day == today).map_or(0, |(_, n)| *n);
        let earlier: usize = counts.iter().filter(|(day, _)| *day < today).map(|(_, n)| *n).sum();
        let (current, last) = KNOWN.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let room = last.map(|(at, heard)| {
            let said: Vec<Value> = (heard.iter())
                .map(|h| {
                    Value::Object(
                        ["name", "kind", "what", "sure", "background"]
                            .iter()
                            .map(|k| (k.to_string(), h[*k].clone()))
                            .collect::<Map<String, Value>>(),
                    )
                })
                .collect();
            json!({"ago": (seconds_now() - at).round() as i64, "interfaces": said})
        });
        let model = Sounds::file(self.data.parent().unwrap_or(&self.data)).is_some();
        let (found, why) = listen::interfaces();
        let interfaces: Vec<Value> =
            found.iter().map(|i| json!({"name": i.name, "kind": i.kind, "default": i.default})).collect();
        let shown = pic.map(|pic| {
            let parts = [
                "songs",
                "artists",
                "made",
                "source",
                "top_artists",
                "genres",
                "decades",
                "year",
                "popularity",
                "spread",
            ];
            Value::Object(
                parts
                    .iter()
                    .map(|k| (k.to_string(), pic.get(*k).cloned().unwrap_or(Value::Null)))
                    .collect::<Map<String, Value>>(),
            )
        });
        json!({"on": self.store.setting("label", OPTION).as_deref() == Some("1"), "who": name,
               "listen": self.store.setting("label", LISTEN).filter(|l| !l.is_empty()).unwrap_or_else(|| "all".to_string()),
               "today": now, "earlier": earlier, "logging": self.music_logging(), "playing": music::now_playing(),
               "current": current,
               // what each interface heard at the last listening, in a word (the numbers stay here)
               "room": room, "model": model, "interfaces": interfaces, "interfaces_why": why.join("; "),
               "spotify": sp, "job": {"state": job.0, "text": job.1}, "picture": shown})
    }

    /// What Music's settings answer to something sent; None for a route that isn't one of them.
    pub(crate) fn music_post(&self, path: &str, body: &Map<String, Value>) -> Option<Answer> {
        let ok = |done: Result<(), String>| done.map(|()| Response::json(json!({"ok": true}))).map_err(asked);
        let home = self.home.clone();
        Some(match path {
            // connect Spotify: its page opens in the browser, and the owner is waited for (five minutes at most)
            "/api/music/spotify" => ok(spotify::check_client_id(
                body.get("client_id").and_then(Value::as_str).unwrap_or(""),
            )
            .and_then(|client_id| {
                job("connecting", "Waiting for you to agree on Spotify's page in your browser.", move || {
                    spotify::connect(&home, &Places::from_env(), &client_id)
                        .map(|_| "Spotify is connected.".to_string())
                })
            })),
            // read the liked songs, and make the picture of the person this device's music belongs to
            "/api/music/liked" => ok((|| {
                let name = self.music_who();
                if name.is_empty() {
                    return Err("Say first whose music this device's is.".to_string());
                }
                if spotify::status(&self.home)?["connected"] != true {
                    return Err("Connect Spotify first.".to_string());
                }
                let store = face_home::store::Store::new(&self.home);
                job("reading", "Reading liked songs", move || {
                    let places = Places::from_env();
                    let said = |e: spotify::Fault| e.text().to_string();
                    let songs = spotify::liked(&home, &places, &mut |done, total| {
                        job_says(format!("Reading liked songs: {} of {}", commas(done as u64), commas(total as u64)))
                    })
                    .map_err(said)?;
                    let ids: Vec<String> =
                        (songs.iter().flat_map(|s| s["artists"].as_array().into_iter().flatten().take(2)))
                            .filter_map(|a| a["id"].as_str().map(str::to_string))
                            .collect();
                    job_says(format!("{} liked songs; reading their artists' genres", commas(songs.len() as u64)));
                    let genres = spotify::genres(&home, &places, &ids, &mut |done, total| {
                        job_says(format!("Genres: {} of {} artists", commas(done as u64), commas(total as u64)))
                    })
                    .map_err(said)?;
                    let mut pic = music::picture(&songs, &genres);
                    pic["source"] = json!("Spotify liked songs");
                    store.put_signal("music", &name, Some(&pic))?;
                    let count = |k: &str| commas(pic[k].as_u64().unwrap_or(0));
                    Ok(format!(
                        "{name}'s usual music is made from {} liked songs by {} artists.",
                        count("songs"),
                        count("artists")
                    ))
                })
            })()),
            "/api/music/disconnect" => ok(spotify::disconnect(&self.home)),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_set_apart() {
        assert_eq!(
            [commas(0), commas(999), commas(1000), commas(12_345), commas(1_234_567)],
            ["0", "999", "1,000", "12,345", "1,234,567"]
        );
    }
}
