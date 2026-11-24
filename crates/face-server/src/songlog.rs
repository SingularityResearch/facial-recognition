//! The music log's listening: every few seconds a look at what a
//! music app shows as playing, and while the log is on, every half minute a listening to every audio
//! interface at once. A song is one row, made when it ends or changes: the song a player names, or music an
//! interface hears twice in a row with no player naming it ("music in the room", with no artist).
//!
//!   {"t": ms it was first seen playing, "name", "artist" ("" for music in the room), "song", "seconds", "data"}
//!   data  app (the player, or "room"); heard (the numbers taken for the song: from an output before a
//!         microphone); via (output | input) and on (that interface's name); interfaces {name: {kind, what,
//!         sure, background, families, ambient, music}} for each one that wasn't quiet; place (the song
//!         against the person's usual music)
//!
//! Only numbers and short text: the sound is never kept. Rows wait in the private folder until the sync has
//! put them in the shared database (`face_home::waiting::MUSIC`).

use face_files::filejson;
use face_home::alerts::check_details;
use serde_json::{Map, Value, json};

use crate::{music, sound};

/// seconds between looks at what is playing
pub const LOOK_EVERY: f64 = 5.0;
/// seconds between listenings while the log is on, and how long one lasts
pub const LISTEN_EVERY: f64 = 30.0;
pub const LISTEN_FOR: f64 = 8.0;
/// a song seen for less was skipped past: not logged
const MIN_SECONDS: f64 = 15.0;
/// bytes of one row's data
const MAX_DATA: usize = 12_000;
/// the song of a row when no player named it: an interface heard music
pub const ROOM_SONG: &str = "music in the room";
/// listenings in a row that must hear music before the room counts as playing some
const ROOM_AFTER: u32 = 2;

/// A song that plays: who by, what, which app says so ("room": an interface heard music), when it was first
/// and last seen, and what each listening heard meanwhile.
#[derive(Clone, Debug)]
pub struct Song {
    pub artist: String,
    pub song: String,
    pub app: String,
    pub t0: f64,
    pub seen: f64,
    pub heard: Vec<Vec<Value>>,
}

/// What the listening remembers between its looks.
#[derive(Default)]
pub struct Listener {
    pub song: Option<Song>,
    listened: f64,
    /// the newest listening: when, and what every interface heard
    pub last: Option<(f64, Vec<Value>)>,
    /// listenings in a row in which an interface heard music with no player naming a song
    room: u32,
}

impl Listener {
    /// The log is off: nothing plays, nothing was heard.
    pub fn off(&mut self) {
        (self.song, self.last, self.room) = (None, None, 0);
    }

    /// One look (and, when one is due, one listening): `playing` is what a music app shows, `listen` hears
    /// every interface of these kinds. The song that ended with this look, if one did.
    pub fn step(
        &mut self,
        now: f64,
        playing: Option<&Value>,
        kinds: &[&str],
        listen: &mut dyn FnMut(&[&str]) -> Vec<Value>,
    ) -> Option<Song> {
        let text = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
        let named = self.song.as_ref().is_some_and(|s| s.app != "room");
        let same = playing.is_some_and(|p| {
            named
                && self
                    .song
                    .as_ref()
                    .is_some_and(|s| (text(p, "artist"), text(p, "song")) == (s.artist.clone(), s.song.clone()))
        });
        let mut ended = None;
        // a player's song ended or changed, or a player now names what the room was playing
        if self.song.is_some() && (playing.is_some() || named) && !same {
            ended = self.song.take();
        }
        if let Some(p) = playing.filter(|_| !same) {
            self.song = Some(Song {
                artist: text(p, "artist"),
                song: text(p, "song"),
                app: text(p, "app"),
                t0: now,
                seen: now,
                heard: Vec::new(),
            });
            (self.listened, self.room) = (0.0, 0);
        }
        if let Some(song) = self.song.as_mut().filter(|s| s.app != "room") {
            song.seen = now;
        }
        if !kinds.is_empty() && now - self.listened >= LISTEN_EVERY {
            self.listened = now;
            let heard = listen(kinds);
            self.last = Some((now, heard.clone()));
            let music_here = heard.iter().any(|h| h["what"] == "music");
            if let Some(song) = self.song.as_mut().filter(|s| s.app != "room") {
                if !heard.is_empty() && song.heard.len() < 40 {
                    song.heard.push(heard);
                }
            } else if music_here {
                // no player names a song, and music sounds
                self.room += 1;
                if self.song.is_none() && self.room >= ROOM_AFTER {
                    let began = now - LISTEN_EVERY * (self.room - 1) as f64;
                    self.song = Some(Song {
                        artist: String::new(),
                        song: ROOM_SONG.to_string(),
                        app: "room".to_string(),
                        t0: began,
                        seen: now,
                        heard: Vec::new(),
                    });
                }
                if let Some(song) = self.song.as_mut() {
                    song.seen = now;
                    if song.heard.len() < 40 {
                        song.heard.push(heard);
                    }
                }
            } else {
                self.room = 0;
                // the room's music stopped
                if self.song.is_some() {
                    ended = self.song.take();
                }
            }
        }
        ended
    }
}

/// One interface's listening as the log keeps it: what it heard in a word, how sure, the families of sound,
/// the background's numbers.
fn summary(h: &Value) -> Value {
    let truthy = |v: &Value| {
        !matches!(v, Value::Null | Value::Bool(false))
            && v != &json!("")
            && v != &json!({})
            && v != &json!([])
            && v != &json!(0)
    };
    let mut out = Map::new();
    out.insert("kind".to_string(), h["kind"].clone());
    out.insert("what".to_string(), json!(h["what"].as_str().unwrap_or("")));
    out.insert("sure".to_string(), h["sure"].clone());
    if truthy(&h["background"]) {
        out.insert("background".to_string(), h["background"].clone());
    }
    if let Some(families) = h["families"].as_object().filter(|f| !f.is_empty()) {
        out.insert(
            "families".to_string(),
            Value::Object(families.iter().take(4).map(|(k, v)| (k.clone(), v.clone())).collect()),
        );
    }
    if truthy(&h["ambient"]) {
        out.insert("ambient".to_string(), h["ambient"].clone());
    }
    Value::Object(out)
}

/// The row of a song that ended, for the person whose music this device's is; None for one that was skipped
/// past. `picture`: that person's usual music, when it was made.
pub fn row(song: &Song, who: &str, picture: Option<&Value>) -> Option<Value> {
    if song.seen - song.t0 < MIN_SECONDS {
        return None;
    }
    // (in the order they were first heard: the newest listening's word for each interface, and every listening's
    // numbers of the music it heard)
    let mut seen: Vec<(String, Value)> = Vec::new();
    let mut per: Vec<(String, Value, Vec<Value>)> = Vec::new();
    for h in song.heard.iter().flatten() {
        let key: String = h["name"].as_str().unwrap_or("").chars().take(40).collect();
        if h["what"].as_str().is_some_and(|w| !w.is_empty() && w != "quiet") {
            match seen.iter_mut().find(|(k, _)| *k == key) {
                Some((_, said)) => *said = summary(h),
                None => seen.push((key.clone(), summary(h))),
            }
        }
        if h["heard"].as_object().is_some_and(|m| !m.is_empty()) {
            match per.iter_mut().find(|(k, _, _)| *k == key) {
                Some((_, _, all)) => all.push(h["heard"].clone()),
                None => per.push((key, h["kind"].clone(), vec![h["heard"].clone()])),
            }
        }
    }
    let combined = |all: &[Value]| sound::combine(&all.iter().collect::<Vec<_>>()).unwrap_or(Value::Null);
    let mut data = Map::new();
    data.insert("app".to_string(), json!(song.app));
    if !seen.is_empty() || !per.is_empty() {
        let mut interfaces = Map::new();
        let names = seen
            .iter()
            .map(|(k, _)| k)
            .chain(per.iter().map(|(k, _, _)| k).filter(|k| !seen.iter().any(|(s, _)| s == *k)));
        for name in names {
            let music = per.iter().find(|(k, _, _)| k == name);
            let mut one = match (seen.iter().find(|(k, _)| k == name), music) {
                (Some((_, said)), _) => said.as_object().cloned().unwrap_or_default(),
                (None, Some((_, kind, _))) => [("kind".to_string(), kind.clone())].into_iter().collect(),
                (None, None) => Map::new(),
            };
            if let Some((_, _, all)) = music {
                one.insert("music".to_string(), combined(all));
            }
            interfaces.insert(name.clone(), Value::Object(one));
        }
        data.insert("interfaces".to_string(), Value::Object(interfaces));
    }
    if !per.is_empty() {
        let each: Vec<Value> =
            per.iter().map(|(name, kind, all)| json!({"name": name, "kind": kind, "heard": combined(all)})).collect();
        if let Some(top) = sound::best(&each) {
            data.insert("heard".to_string(), top["heard"].clone());
            data.insert("via".to_string(), top["kind"].clone());
            data.insert("on".to_string(), top["name"].clone());
        }
    }
    if let Some(pic) = picture.filter(|p| p.as_object().is_some_and(|p| !p.is_empty()) && !song.artist.is_empty()) {
        let mut placed = music::place(&song.artist, &song.song, pic, None).as_object().cloned().unwrap_or_default();
        placed.retain(|k, _| k != "artist" && k != "song");
        data.insert("place".to_string(), Value::Object(placed));
    }
    Some(
        json!({"t": (song.t0 * 1000.0) as i64, "name": who, "artist": song.artist, "song": song.song, "seconds": song.seen - song.t0, "data": data}),
    )
}

/// One row, checked: as it is kept, or why it isn't one.
pub fn check(r: &Value, now_ms: i64) -> Result<Value, String> {
    if !r.is_object() {
        return Err("a row is an object".to_string());
    }
    let text = |k: &str, most: usize| -> String {
        let said = match &r[k] {
            Value::String(s) => s.clone(),
            Value::Null | Value::Bool(false) => String::new(),
            other => other.to_string(),
        };
        said.trim().chars().take(most).collect()
    };
    let (name, artist, song) = (text("name", 100), text("artist", 200), text("song", 200));
    if name.is_empty() || name == "Unknown" || name.starts_with('#') || song.is_empty() {
        return Err(
            "a row needs a named person and a song (the artist may be unknown: music heard in the room)".to_string()
        );
    }
    let bad = || "bad row".to_string();
    let t = r["t"]
        .as_i64()
        .or_else(|| r["t"].as_f64().map(|t| t.trunc() as i64))
        .or_else(|| r["t"].as_str()?.trim().parse().ok())
        .ok_or_else(bad)?;
    let seconds = match &r["seconds"] {
        Value::Null => None,
        v => Some(filejson::round(v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()).ok_or_else(bad)?, 1)),
    };
    if (t - now_ms).abs() > 86_400_000 || seconds.is_some_and(|s| !(0.0..=86_400.0).contains(&s)) {
        return Err("row time out of range".to_string());
    }
    let data = match &r["data"] {
        Value::Null => json!({}),
        Value::Object(data) if filejson::dumps(&Value::Object(data.clone())).chars().count() <= MAX_DATA => {
            Value::Object(data.clone())
        }
        _ => return Err("bad row data".to_string()),
    };
    Ok(json!({"t": t, "name": name, "artist": artist, "song": song, "seconds": seconds, "data": check_details(&data)?}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(what: &str, music: bool) -> Vec<Value> {
        let numbers = json!({"level": -20.0, "dynamics": 6.0, "tempo": 120.0, "pulse": 0.5, "bright": 2000.0, "bass": 0.1, "change": 0.02, "key": "C", "mode": "major", "mode_sure": 0.4});
        vec![
            json!({"name": "Speakers", "kind": "output", "default": true, "heard": if music { numbers.clone() } else { Value::Null }, "sounds": null, "families": {"music": 0.7, "speech": 0.1},
                   "ambient": {"level": -20.0, "steady": 0.3}, "what": what, "sure": 0.7, "background": ""}),
            json!({"name": "Microphone", "kind": "input", "default": true, "heard": null, "sounds": null, "families": null, "ambient": null, "what": "quiet", "sure": 1.0, "background": ""}),
        ]
    }

    #[test]
    fn a_song_is_logged_when_it_ends_with_what_each_interface_heard() {
        let (mut l, t0, mut listened) = (Listener::default(), 1_800_000_000.0, 0);
        let song = json!({"artist": "An Artist", "song": "A Song", "app": "Spotify"});
        let mut step = |l: &mut Listener, now: f64, playing: Option<&Value>, what: &str| {
            l.step(now, playing, &["output", "input"], &mut |_| {
                listened += 1;
                heard(what, what == "music")
            })
        };
        // nothing plays: listened to, nothing kept
        assert!(step(&mut l, t0, None, "quiet").is_none() && l.song.is_none() && l.last.is_some());
        // the song plays for 95 seconds
        for k in (0..=95).step_by(5) {
            assert!(step(&mut l, t0 + 10.0 + k as f64, Some(&song), "music").is_none());
        }
        assert_eq!(
            (l.song.as_ref().unwrap().heard.len(), l.song.as_ref().unwrap().seen - l.song.as_ref().unwrap().t0),
            (4, 95.0)
        );
        // another song begins: the first one ends
        let next = json!({"artist": "Another", "song": "Next", "app": "Spotify"});
        let ended = step(&mut l, t0 + 110.0, Some(&next), "music").unwrap();
        let r = check(
            &row(&ended, "Ann", Some(&json!({"songs": 3, "by_artist": {"An Artist": 2}}))).unwrap(),
            (t0 * 1000.0) as i64,
        )
        .unwrap();
        assert_eq!(
            (&r["name"], &r["artist"], &r["song"], &r["seconds"], &r["t"]),
            (&json!("Ann"), &json!("An Artist"), &json!("A Song"), &json!(95.0), &json!((t0 as i64 + 10) * 1000))
        );
        let d = &r["data"];
        assert_eq!(
            (&d["app"], &d["via"], &d["on"], &d["heard"]["heard"], &d["heard"]["tempo"], &d["heard"]["key"]),
            (&json!("Spotify"), &json!("output"), &json!("Speakers"), &json!(4), &json!(120.0), &json!("C"))
        );
        assert_eq!(
            d["interfaces"],
            json!({"Speakers": {"kind": "output", "what": "music", "sure": 0.7, "families": {"music": 0.7, "speech": 0.1}, "ambient": {"level": -20.0, "steady": 0.3},
                                                         "music": d["heard"]}})
        );
        assert_eq!(
            d["place"],
            json!({"artist_rank": 1, "artist_songs": 2, "genre_overlap": null, "usual": "one of their artists"})
        );
        // seen for five seconds: skipped past, not logged
        let ended = step(&mut l, t0 + 115.0, None, "quiet").unwrap();
        assert!(row(&ended, "Ann", None).is_none() && l.song.is_none());
        assert!(listened >= 5);
        // switched off: nothing is remembered
        l.off();
        assert!(l.song.is_none() && l.last.is_none());
    }

    #[test]
    fn music_heard_in_the_room_is_logged_without_a_player_naming_it() {
        let (mut l, t0) = (Listener::default(), 1_800_000_000.0);
        let step = |l: &mut Listener, now: f64, what: &str| {
            l.step(now, None, &["input"], &mut |_| heard(what, what == "music"))
        };
        assert!(step(&mut l, t0, "speech").is_none());
        // heard once: not yet; twice in a row: the room plays music, since the first of the two
        assert!(step(&mut l, t0 + 30.0, "music").is_none() && l.song.is_none());
        assert!(step(&mut l, t0 + 60.0, "music").is_none());
        assert_eq!((l.song.as_ref().unwrap().app.as_str(), l.song.as_ref().unwrap().t0), ("room", t0 + 30.0));
        assert!(step(&mut l, t0 + 90.0, "music").is_none() && step(&mut l, t0 + 95.0, "music").is_none());
        // the music stopped
        let ended = step(&mut l, t0 + 120.0, "speech").unwrap();
        let r = check(&row(&ended, "Ann", Some(&json!({"songs": 3}))).unwrap(), (t0 * 1000.0) as i64).unwrap();
        assert_eq!(
            (&r["artist"], &r["song"], &r["seconds"], &r["data"]["app"]),
            (&json!(""), &json!(ROOM_SONG), &json!(60.0), &json!("room"))
        );
        assert!(r["data"].get("place").is_none() && r["data"]["heard"]["heard"] == 2, "{r}");
        assert!(l.song.is_none());
    }

    #[test]
    fn only_a_named_persons_songs_are_kept_and_only_as_numbers_and_short_text() {
        let now = 1_800_000_000_000i64;
        let good = json!({"t": now, "name": " Ann ", "artist": "", "song": " A song ", "seconds": 12.3456, "data": {"app": "room", "heard": {"level": -20.123_456}}});
        assert_eq!(
            check(&good, now).unwrap(),
            json!({"t": now, "name": "Ann", "artist": "", "song": "A song", "seconds": 12.3, "data": {"app": "room", "heard": {"level": -20.1235}}})
        );
        for (change, why) in [
            (json!({"name": "Unknown"}), "a row needs"),
            (json!({"name": "#ignore"}), "a row needs"),
            (json!({"song": " "}), "a row needs"),
            (json!({"t": "soon"}), "bad row"),
            (json!({"seconds": "long"}), "bad row"),
            (json!({"t": now + 90_000_000}), "row time out of range"),
            (json!({"seconds": -1}), "row time out of range"),
            (json!({"data": "words"}), "bad row data"),
            (json!({"data": {"text": "x".repeat(13_000)}}), "bad row data"),
            (json!({"data": {"picture": ["no", "lists"]}}), "details"),
        ] {
            let mut bad = good.clone();
            for (k, v) in change.as_object().unwrap() {
                bad[k] = v.clone();
            }
            let refused = check(&bad, now).unwrap_err();
            assert!(refused.contains(why), "{change}: {refused}");
        }
        assert!(check(&json!("a row"), now).is_err());
        assert_eq!(
            check(&json!({"t": now, "name": "Ann", "song": "S", "seconds": null}), now).unwrap()["data"],
            json!({})
        );
    }
}
