//! Music as a signal (the parts that need no sound): the song a music app
//! here shows as playing (nothing is asked of the internet), a person's usual music made from their liked
//! songs, and one song against it.
//!
//! How a sound is listened to and what it is are `listen.rs` and `sound.rs`; the log of songs is `songlog.rs`.

use std::collections::BTreeMap;

use face_files::{filejson, paths::utc_now};
use serde_json::{Map, Value, json};

/// Spotify's window title when nothing plays.
const PAUSED: [&str; 4] = ["spotify", "spotify free", "spotify premium", ""];

/// ("artist", "song") from a player's "Artist - Song" (the song may have dashes of its own).
pub fn split_title(title: &str) -> Option<(String, String)> {
    let title = title.trim();
    if PAUSED.contains(&title.to_lowercase().as_str()) {
        return None;
    }
    let (artist, song) = title.split_once(" - ")?;
    let cut = |text: &str| text.trim().chars().take(200).collect::<String>();
    let (artist, song) = (cut(artist), cut(song));
    (!artist.is_empty() && !song.is_empty()).then_some((artist, song))
}

/// The titles of the Spotify app's windows on Windows ("Artist - Song" while a song plays).
#[cfg(windows)]
fn spotify_window_titles() -> Vec<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        },
    };
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> i32 {
        // SAFETY: `found` is the list handed to EnumWindows below, which outlives the call; every buffer is as
        // long as the system is told, and the process handle is closed here
        unsafe {
            let n = GetWindowTextLengthW(window);
            if n <= 0 || (IsWindowVisible(window) == 0 && IsIconic(window) == 0) {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, &mut pid);
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return 1;
            }
            let (mut path, mut size) = ([0u16; 520], 520u32);
            let named = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut size) != 0;
            CloseHandle(process);
            if named && String::from_utf16_lossy(&path[..size as usize]).to_lowercase().ends_with("\\spotify.exe") {
                let mut text = vec![0u16; n as usize + 1];
                let got = GetWindowTextW(window, text.as_mut_ptr(), n + 1).max(0) as usize;
                (*(found as *mut Vec<String>)).push(String::from_utf16_lossy(&text[..got]));
            }
            1
        }
    }
    let mut found: Vec<String> = Vec::new();
    // SAFETY: the callback gets the list's address and is done when EnumWindows returns
    unsafe { EnumWindows(Some(each), &raw mut found as LPARAM) };
    found
}

/// What a system tool says within three seconds, or "".
#[cfg(not(windows))]
fn run(program: &str, args: &[&str]) -> String {
    use std::process::{Command, Stdio};
    let Ok(mut child) =
        Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
    else {
        return String::new();
    };
    let began = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if began.elapsed() < std::time::Duration::from_secs(3) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    }
    let mut said = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut out, &mut said);
    }
    said.trim().to_string()
}

/// {artist, song, app} of the song a music app here shows as playing, if one does.
pub fn now_playing() -> Option<Value> {
    #[cfg(windows)]
    let got = spotify_window_titles().iter().find_map(|title| split_title(title));
    // only a Spotify that is open is asked: asking would start it
    #[cfg(target_os = "macos")]
    let got = if run("pgrep", &["-x", "Spotify"]).is_empty() {
        None
    } else {
        split_title(&run(
            "osascript",
            &[
                "-e",
                "tell application \"Spotify\" to if player state is playing then (artist of current track) & \" - \" & (name of current track)",
            ],
        ))
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let got = {
        let said = run("playerctl", &["-p", "spotify", "metadata", "--format", "{{status}}\t{{artist}} - {{title}}"]);
        said.split_once('\t').filter(|(status, _)| *status == "Playing").and_then(|(_, title)| split_title(title))
    };
    got.map(|(artist, song)| json!({"artist": artist, "song": song, "app": "Spotify"}))
}

// ----------------------------------------------------------------------------- a person's usual music

fn year(v: Option<&Value>) -> Option<i64> {
    let text: String = v?.as_str()?.chars().take(4).collect();
    let year: i64 = (text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok())??;
    (1900..=2100).contains(&year).then_some(year)
}

/// Things counted in the order they first came: the most counted first, those counted alike as they came.
#[derive(Default)]
struct Counter(Vec<(String, u64)>);

impl Counter {
    fn add(&mut self, name: &str) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some((_, count)) => *count += 1,
            None => self.0.push((name.to_string(), 1)),
        }
    }

    fn most(&self, n: usize) -> Vec<(String, u64)> {
        let mut all = self.0.clone();
        all.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        all.truncate(n);
        all
    }

    fn top(&self, n: usize, total: usize) -> Vec<Value> {
        self.most(n).into_iter().map(|(name, songs)| json!({"name": name, "songs": songs, "share": filejson::round(songs as f64 / total as f64, 4)})).collect()
    }
}

/// The middle of some numbers (of an even count: the mean of the middle two).
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 }
}

/// The three cut points of four equal parts (between neighbours, counted over one place more than there are numbers).
fn quartiles(values: &[f64]) -> [f64; 3] {
    let mut data = values.to_vec();
    data.sort_by(f64::total_cmp);
    let (ld, n) = (data.len() as i64, 4i64);
    let m = ld + 1;
    [1, 2, 3].map(|i| {
        let j = (i * m / n).clamp(1, ld - 1);
        let delta = i * m - j * n;
        (data[j as usize - 1] * (n - delta) as f64 + data[j as usize] * delta as f64) / n as f64
    })
}

/// A person's usual music from their liked songs (`spotify::liked`) and their artists' genres
/// (`spotify::genres`): counts, shares and short names only.
pub fn picture(songs: &[Value], genres: &Map<String, Value>) -> Value {
    let truthy = |v: Option<&Value>| {
        !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
            && v != Some(&json!(""))
            && v != Some(&json!(0))
    };
    let songs: Vec<&Value> = songs.iter().filter(|s| s.is_object() && truthy(s.get("name"))).collect();
    let n = songs.len();
    if n == 0 {
        return json!({"songs": 0});
    }
    let (mut artists, mut genre_count, mut decades, mut added) =
        (Counter::default(), Counter::default(), BTreeMap::<String, u64>::new(), BTreeMap::<String, u64>::new());
    let (mut years, mut with_genre) = (Vec::new(), 0usize);
    for s in &songs {
        let listed = s.get("artists").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        if let Some(first) =
            listed.first().and_then(|a| a.get("name")).and_then(Value::as_str).filter(|name| !name.is_empty())
        {
            artists.add(first);
        }
        // (each of a song's genres once, however many of its artists have it)
        let mut seen: Vec<&str> = Vec::new();
        for a in listed {
            let theirs = a.get("id").and_then(Value::as_str).and_then(|id| genres.get(id)).and_then(Value::as_array);
            for g in theirs.into_iter().flatten().filter_map(Value::as_str) {
                if !seen.contains(&g) {
                    seen.push(g);
                }
            }
        }
        with_genre += usize::from(!seen.is_empty());
        for g in seen {
            genre_count.add(g);
        }
        if let Some(y) = year(s.get("released")) {
            years.push(y as f64);
            *decades.entry(format!("{}s", y / 10 * 10)).or_default() += 1;
        }
        if let Some(a) = year(s.get("added")) {
            *added.entry(a.to_string()).or_default() += 1;
        }
    }
    let pop: Vec<f64> = songs
        .iter()
        .filter_map(|s| s.get("popularity").filter(|p| p.is_i64() || p.is_u64()).and_then(Value::as_f64))
        .collect();
    let mins: Vec<f64> = songs
        .iter()
        .filter_map(|s| s.get("ms").and_then(Value::as_f64).filter(|ms| *ms != 0.0))
        .map(|ms| ms / 60000.0)
        .collect();
    let shares: Vec<f64> = artists.0.iter().map(|(_, v)| *v as f64 / n as f64).collect();
    let of = |count: usize| filejson::round(count as f64 / n as f64, 4);
    // every artist with more than one liked song (`place` ranks a song's artist among them)
    let mut several: Vec<&(String, u64)> = artists.0.iter().filter(|(_, v)| *v > 1).collect();
    several.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let by_artist: Map<String, Value> = several.into_iter().take(3000).map(|(k, v)| (k.clone(), json!(v))).collect();
    // how spread out the liking is: 1 = every song by another artist, near 0 = nearly all by one
    let spread = if shares.len() > 1 {
        filejson::round(-shares.iter().map(|p| p * p.ln()).sum::<f64>() / (shares.len() as f64).ln(), 4)
    } else {
        0.0
    };
    let year_part = if years.is_empty() {
        Value::Null
    } else {
        let (oldest, newest) =
            (years.iter().copied().fold(f64::MAX, f64::min), years.iter().copied().fold(f64::MIN, f64::max));
        let half =
            (years.len() >= 4).then(|| quartiles(&years)).map(|q| json!([q[0].trunc() as i64, q[2].trunc() as i64]));
        json!({"median": median(&years).trunc() as i64, "oldest": oldest as i64, "newest": newest as i64, "middle_half": half})
    };
    json!({
        "songs": n, "artists": artists.0.len(), "made": utc_now(),
        "top_artists": artists.top(25, n),
        "by_artist": by_artist,
        "top_10_artists_share": of(artists.most(10).iter().map(|(_, v)| *v as usize).sum()),
        "spread": spread,
        "genres": genre_count.top(25, n), "songs_with_genres": of(with_genre),
        "decades": decades.iter().map(|(d, c)| json!({"name": d, "songs": c, "share": of(*c as usize)})).collect::<Vec<_>>(),
        "year": year_part,
        "added": added.iter().map(|(y, c)| json!({"year": y, "songs": c})).collect::<Vec<_>>(),
        "explicit_share": of(songs.iter().filter(|s| truthy(s.get("explicit"))).count()),
        "minutes": if mins.is_empty() { Value::Null } else { json!({"median": filejson::round(median(&mins), 2), "total_hours": filejson::round(mins.iter().sum::<f64>() / 60.0, 1)}) },
        // Spotify's 0..100 for how much a song is played by everyone now: low = little known
        "popularity": if pop.is_empty() { Value::Null } else {
            let share = |count: usize| filejson::round(count as f64 / pop.len() as f64, 4);
            json!({"median": median(&pop).trunc() as i64, "little_known_share": share(pop.iter().filter(|p| **p < 25.0).count()),
                   "widely_played_share": share(pop.iter().filter(|p| **p >= 60.0).count())})
        },
    })
}

/// One song against a person's picture: {artist_rank, artist_songs, genre_overlap, usual}. usual is "one of
/// their artists", "their kind of music" or "not their usual", from the artist's place in the liked songs and,
/// when the artist's genres are known, how much of the liked collection shares them.
pub fn place(artist: &str, song: &str, pic: &Value, artist_genres: Option<&[String]>) -> Value {
    let liked: Vec<(String, Value)> = match pic.get("by_artist").and_then(Value::as_object).filter(|by| !by.is_empty())
    {
        Some(by) => by.iter().map(|(a, n)| (a.clone(), n.clone())).collect(),
        None => (pic.get("top_artists").and_then(Value::as_array).into_iter().flatten())
            .filter_map(|a| Some((a.get("name")?.as_str()?.to_string(), a.get("songs")?.clone())))
            .collect(),
    };
    // most liked first
    let at = liked.iter().position(|(a, _)| a.to_lowercase() == artist.to_lowercase());
    let count = at.map_or(json!(0), |at| liked[at].1.clone());
    let share = |genre: &str| {
        (pic.get("genres").and_then(Value::as_array).into_iter().flatten())
            .find(|g| g["name"] == genre)
            .and_then(|g| g["share"].as_f64())
            .unwrap_or(0.0)
    };
    let overlap = artist_genres
        .filter(|g| !g.is_empty())
        .map(|genres| filejson::round(genres.iter().map(|g| share(g)).fold(0.0, f64::max), 4));
    let songs = pic.get("songs").and_then(Value::as_f64).unwrap_or(0.0);
    let usual = if at.is_some() {
        Some("one of their artists")
    } else if overlap.is_some_and(|o| o >= 0.1) {
        Some("their kind of music")
    } else if overlap.is_some() || songs != 0.0 {
        Some("not their usual")
    } else {
        None
    };
    let cut = |text: &str| text.chars().take(200).collect::<String>();
    json!({"artist": cut(artist), "song": cut(song), "artist_rank": at.map(|at| at + 1), "artist_songs": count, "genre_overlap": overlap, "usual": usual})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_players_title_is_an_artist_and_a_song() {
        assert_eq!(
            split_title(" An Artist - A Song - Live "),
            Some(("An Artist".to_string(), "A Song - Live".to_string()))
        );
        for none in ["Spotify", "spotify premium", "", "  ", "No dash here", "Artist -Song", " - Song", "Artist - "] {
            assert_eq!(split_title(none), None, "{none}");
        }
        assert_eq!(split_title(&format!("{} - s", "a".repeat(300))).unwrap().0.len(), 200);
    }

    #[test]
    fn middles_and_quarters_have_their_fixed_values() {
        // (statistics.median and statistics.quantiles(n=4) of the same numbers)
        assert_eq!((median(&[3.0, 1.0, 2.0]), median(&[4.0, 1.0, 3.0, 2.0])), (2.0, 2.5));
        assert_eq!(quartiles(&[1.0, 2.0, 3.0, 4.0]), [1.25, 2.5, 3.75]);
        assert_eq!(quartiles(&[2019.0, 1999.0, 2005.0, 2021.0, 2010.0]), [2002.0, 2010.0, 2020.0]);
        assert_eq!(quartiles(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]), [2.5, 5.0, 7.5]);
    }

    #[test]
    fn a_persons_usual_music_is_counted_from_their_liked_songs() {
        let liked = |id: &str,
                     name: &str,
                     artists: &[(&str, &str)],
                     released: &str,
                     added: &str,
                     ms: u64,
                     explicit: bool,
                     popularity: Value| {
            json!({"id": id, "name": name, "artists": artists.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(), "album": "An album",
                   "released": released, "added": added, "ms": ms, "explicit": explicit, "popularity": popularity})
        };
        let songs = vec![
            liked(
                "1",
                "One",
                &[("a", "Ann"), ("b", "Bob")],
                "2019-05-17",
                "2024-01-02T10:00:00Z",
                180_000,
                false,
                json!(70),
            ),
            liked("2", "Two", &[("a", "Ann")], "2021", "2025-03-04T10:00:00Z", 240_000, true, json!(10)),
            liked("3", "Three", &[("b", "Bob")], "1999-01-01", "2025-06-07T10:00:00Z", 200_000, false, json!(40)),
            liked("4", "Four", &[("c", "Cy")], "2005-12", "2025-08-09T10:00:00Z", 0, false, Value::Null),
            liked("5", "Five", &[("a", "Ann")], "not a date", "", 300_000, false, json!(60)),
            liked("6", "", &[("a", "Ann")], "2020", "2025", 1, false, json!(99)),
            json!("not a song"),
        ];
        let genres: Map<String, Value> =
            json!({"a": ["pop", "indie"], "b": ["pop", "rock"], "x": ["unused"]}).as_object().unwrap().clone();
        let pic = picture(&songs, &genres);
        assert_eq!((&pic["songs"], &pic["artists"], pic["made"].as_str().unwrap().len()), (&json!(5), &json!(3), 20));
        assert_eq!(
            pic["top_artists"],
            json!([{"name": "Ann", "songs": 3, "share": 0.6}, {"name": "Bob", "songs": 1, "share": 0.2}, {"name": "Cy", "songs": 1, "share": 0.2}])
        );
        assert_eq!((&pic["by_artist"], &pic["top_10_artists_share"]), (&json!({"Ann": 3}), &json!(1.0)));
        // song One counts pop once though both its artists have it
        assert_eq!(
            pic["genres"],
            json!([{"name": "pop", "songs": 4, "share": 0.8}, {"name": "indie", "songs": 3, "share": 0.6}, {"name": "rock", "songs": 2, "share": 0.4}])
        );
        assert_eq!(pic["songs_with_genres"], json!(0.8));
        assert_eq!(
            pic["decades"],
            json!([{"name": "1990s", "songs": 1, "share": 0.2}, {"name": "2000s", "songs": 1, "share": 0.2}, {"name": "2010s", "songs": 1, "share": 0.2},
                                           {"name": "2020s", "songs": 1, "share": 0.2}])
        );
        assert_eq!(pic["year"], json!({"median": 2012, "oldest": 1999, "newest": 2021, "middle_half": [2000, 2020]}));
        assert_eq!(pic["added"], json!([{"year": "2024", "songs": 1}, {"year": "2025", "songs": 3}]));
        assert_eq!(
            (&pic["explicit_share"], &pic["minutes"]),
            (&json!(0.2), &json!({"median": 3.67, "total_hours": 0.3}))
        );
        assert_eq!(pic["popularity"], json!({"median": 50, "little_known_share": 0.25, "widely_played_share": 0.5}));
        assert_eq!(pic["spread"], json!(0.865));
        assert_eq!(picture(&[], &genres), json!({"songs": 0}));
        // one song against it
        assert_eq!(
            place("ANN", "A song", &pic, None),
            json!({"artist": "ANN", "song": "A song", "artist_rank": 1, "artist_songs": 3, "genre_overlap": null, "usual": "one of their artists"})
        );
        let other = place("Someone", "S", &pic, Some(&["rock".to_string(), "jazz".to_string()]));
        assert_eq!(
            (&other["artist_rank"], &other["artist_songs"], &other["genre_overlap"], &other["usual"]),
            (&Value::Null, &json!(0), &json!(0.4), &json!("their kind of music"))
        );
        assert_eq!(place("Someone", "S", &pic, Some(&["jazz".to_string()]))["usual"], json!("not their usual"));
        assert_eq!(place("Someone", "S", &pic, None)["usual"], json!("not their usual"));
        assert_eq!(place("Someone", "S", &json!({"songs": 0}), None)["usual"], Value::Null);
        // (Bob has one song: ranked from the top artists when no artist has several)
        let few = json!({"songs": 2, "top_artists": [{"name": "Ann", "songs": 1}, {"name": "Bob", "songs": 1}]});
        assert_eq!(place("bob", "S", &few, None)["artist_rank"], json!(2));
    }
}
