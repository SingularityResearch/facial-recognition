//! Photo files as the labeling engine sees them: content hashes,
//! picture fingerprints (for copies), online-only cloud files (Windows: made local while read, then let go
//! of), camera dates, this computer's local time, and the photo folders that were added
//! (`data/settings.json`).

use std::path::Path;

use face_core::image::Bgr;
use face_files::{filejson, paths::atomic_write};
use serde_json::{Map, Value, json};

use crate::exif;

/// SHA-1 of the file's bytes: the same photo copied or renamed gets the same key.
pub fn file_hash(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hash = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    let (mut buf, mut n) = (vec![0u8; 1 << 20], 0u64);
    loop {
        match file.read(&mut buf)? {
            0 => break,
            k => {
                hash.update(&buf[..k]);
                n += k as u64;
            }
        }
    }
    let size = std::fs::metadata(path)?.len();
    if n != size {
        // e.g. an online-only cloud file that wasn't all downloaded
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        return Err(std::io::Error::other(format!(
            "read only {} of {} bytes of {name}",
            thousands(n),
            thousands(size)
        )));
    }
    Ok(hash.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// SHA-1 of some bytes, as text.
pub fn sha1_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 1234567 as "1,234,567".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// True if a photo's record hasn't been looked at for possible faces yet (videos aren't: too slow).
pub fn needs_possible(p: &Value) -> bool {
    let has = |k: &str| p.get(k).is_some_and(crate::labels::truthy);
    has("path") && !has("video") && !has("dup_of") && p.get("maybe").is_none()
}

/// True if the file's data is still in the cloud (iCloud Photos, OneDrive: "online-only"). Windows only.
pub fn online_only(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const CLOUD: u32 = 0x40_0000 | 0x1000;
        std::fs::metadata(path).is_ok_and(|m| m.file_attributes() & CLOUD != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn attrib(args: &[&str], path: &Path) {
    let mut run = std::process::Command::new("attrib");
    run.args(args).arg(path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        run.creation_flags(0x0800_0000); // no console window
    }
    let _ = run.status();
}

/// Download an online-only file all the way. Read straight through, such a file can come back cut short, so
/// it is pinned ("Always keep on this device") and waited for, allowing at least 0.5 MB/s. True if this
/// pinned it (`unpin` it when done), false if it was here already; an error if it didn't arrive in time.
pub fn make_local(path: &Path) -> std::io::Result<bool> {
    if !online_only(path) {
        return Ok(false);
    }
    attrib(&["+P", "-U"], path);
    let size = std::fs::metadata(path)?.len();
    let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(300.0 + size as f64 / 5e5);
    while online_only(path) {
        if std::time::Instant::now() > end {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            return Err(std::io::Error::other(format!("{name} didn't finish downloading from the cloud")));
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(true)
}

/// Undo `make_local`'s pin: the files stay here, and the cloud app may free their space again later.
pub fn unpin(paths: &[&Path]) {
    for p in paths {
        attrib(&["-P"], p);
    }
}

/// A picture as shades of grey, as OpenCV makes them (`cvtColor`, BGR to grey).
fn grey(bgr: &Bgr) -> Vec<u8> {
    bgr.px
        .chunks_exact(3)
        .map(|p| ((p[0] as u32 * 3735 + p[1] as u32 * 19235 + p[2] as u32 * 9798 + (1 << 14)) >> 15) as u8)
        .collect()
}

/// 256-bit picture fingerprint (a difference hash), 64 hex digits: re-saved, resized or re-compressed copies
/// of a photo differ in a few bits, different photos in roughly half of them.
pub fn fingerprint(bgr: &Bgr) -> String {
    // grey, shrunk to 17 x 16 by averaging; a bit for each place that is lighter than the one left of it
    let g = grey(bgr);
    let three = Bgr { w: bgr.w, h: bgr.h, px: g.iter().flat_map(|v| [*v, *v, *v]).collect() };
    let small = three.resize_area(17, 16);
    let at = |x: usize, y: usize| small.px[(y * 17 + x) * 3];
    let mut bytes = [0u8; 32];
    for y in 0..16 {
        for x in 0..16 {
            if at(x + 1, y) > at(x, y) {
                bytes[(y * 16 + x) / 8] |= 0x80 >> ((y * 16 + x) % 8);
            }
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The parts of a local time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// A time in seconds since 1970 in this computer's own time (the C library's localtime). None where the
/// system can't say.
pub fn local(t: f64) -> Option<Local> {
    if !t.is_finite() {
        return None;
    }
    let secs = t.floor() as libc::time_t;
    // SAFETY: a zeroed tm is a valid one to write into, and both calls only write into it
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    #[cfg(windows)]
    let ok = unsafe { libc::localtime_s(&mut tm, &secs) } == 0;
    #[cfg(not(windows))]
    let ok = !unsafe { libc::localtime_r(&secs, &mut tm) }.is_null();
    ok.then_some(Local {
        year: tm.tm_year + 1900,
        month: tm.tm_mon as u32 + 1,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
    })
}

/// "2024-05-17" in this computer's own time (`time.strftime("%Y-%m-%d", time.localtime(t))`), or nothing.
pub fn local_day(t: f64) -> String {
    local(t).map_or_else(String::new, |l| format!("{:04}-{:02}-{:02}", l.year, l.month, l.day))
}

/// "2024-05-17 14:03" in this computer's own time.
pub fn local_minute(t: f64) -> String {
    local(t).map_or_else(String::new, |l| {
        format!("{:04}-{:02}-{:02} {:02}:{:02}", l.year, l.month, l.day, l.hour, l.minute)
    })
}

/// A camera's date and time ("%Y:%m:%d %H:%M:%S") as its six numbers,
/// or none when the text isn't one (a field may be one digit, a day may have a space for its first digit, any
/// run of blanks may stand between date and time, and the day must be one that month has).
fn camera_fields(text: &str) -> Option<[i32; 6]> {
    // each field: its ways of being written, in the order they are tried
    fn field(kind: usize, s: &[u8]) -> Vec<(usize, i32)> {
        let digit = |k: usize| s.get(k).filter(|b| b.is_ascii_digit()).map(|b| (*b - b'0') as i32);
        let two = digit(0).zip(digit(1)).map(|(a, b)| a * 10 + b);
        let one = digit(0);
        let mut out = Vec::new();
        match kind {
            0 => out.extend(two.zip(digit(2).zip(digit(3))).map(|(a, (b, c))| (4, a * 100 + b * 10 + c))),
            1 => {
                out.extend(two.filter(|v| (1..=12).contains(v)).map(|v| (2, v)));
                out.extend(one.filter(|v| *v >= 1).map(|v| (1, v)));
            }
            2 => {
                out.extend(two.filter(|v| (1..=31).contains(v)).map(|v| (2, v)));
                out.extend(one.filter(|v| *v >= 1).map(|v| (1, v)));
                out.extend(digit(1).filter(|v| s.first() == Some(&b' ') && *v >= 1).map(|v| (2, v)));
            }
            _ => {
                let most = [23, 59, 61][kind - 3];
                out.extend(two.filter(|v| *v <= most).map(|v| (2, v)));
                out.extend(one.map(|v| (1, v)));
            }
        }
        out
    }
    fn rest(kind: usize, s: &[u8], got: &mut [i32; 6]) -> bool {
        for (n, v) in field(kind, s) {
            got[kind] = v;
            let after = &s[n..];
            let done = match kind {
                5 => after.is_empty(),
                2 => {
                    // one blank or more, as many as leave the rest readable
                    let blanks = after.iter().take_while(|b| b.is_ascii_whitespace() || **b == 0x0b).count();
                    (1..=blanks).rev().any(|k| rest(3, &after[k..], got))
                }
                _ => after.first() == Some(&b':') && rest(kind + 1, &after[1..], got),
            };
            if done {
                return true;
            }
        }
        false
    }
    let mut got = [0; 6];
    if !text.is_ascii() || !rest(0, text.as_bytes(), &mut got) {
        return None;
    }
    let [year, month, day, ..] = got;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month as usize - 1];
    (year >= 1 && day <= days).then_some(got)
}

#[cfg(windows)]
unsafe extern "C" {
    /// the C library's mktime (the libc crate leaves it out on Windows)
    #[link_name = "_mktime64"]
    fn mktime64(tm: *mut libc::tm) -> i64;
}

/// A camera's date and time ("2024:05:17 14:03:22", EXIF's way of writing it, in the camera's own local time)
/// as seconds since 1970 (the C library's mktime of its six numbers), by this
/// computer's own time zone. `Ok(None)`: the text isn't a date and time. `Err`: it is one that the system
/// can't place (on Windows, any before 1970).
pub fn camera_time(text: &str) -> Result<Option<f64>, String> {
    let first: String = text.chars().take(19).collect();
    let Some([year, month, day, hour, minute, second]) = camera_fields(&first) else { return Ok(None) };
    // SAFETY: a zeroed tm is a valid one, and mktime only reads and rewrites it
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    (tm.tm_year, tm.tm_mon, tm.tm_mday) = (year - 1900, month - 1, day);
    (tm.tm_hour, tm.tm_min, tm.tm_sec) = (hour, minute, second);
    (tm.tm_isdst, tm.tm_wday) = (-1, -1);
    #[cfg(windows)]
    let t = unsafe { mktime64(&mut tm) };
    #[cfg(not(windows))]
    let t = unsafe { libc::mktime(&mut tm) } as i64;
    if t == -1 && tm.tm_wday == -1 {
        return Err("mktime argument out of range".to_string());
    }
    Ok(Some(t as f64))
}

/// When the photo was taken (EXIF), else the file's date, as "2024-05-17 14:03". Nothing if unreadable.
pub fn photo_date(path: &Path) -> String {
    if let Some(d) = exif::header_of(path).ok().and_then(|h| h.taken) {
        let c: Vec<char> = d.chars().collect();
        let part = |a: usize, b: usize| c.get(a.min(c.len())..b.min(c.len())).unwrap_or(&[]).iter().collect::<String>();
        return format!("{}-{}-{} {}", part(0, 4), part(5, 7), part(8, 10), part(11, 16));
    }
    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(at) => local_minute(seconds(at)),
        Err(_) => String::new(),
    }
}

/// A file's time as seconds since 1970, in one number.
pub fn seconds(at: std::time::SystemTime) -> f64 {
    match at.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as f64 + 1e-9 * d.subsec_nanos() as f64,
        Err(e) => -(e.duration().as_secs() as f64 + 1e-9 * e.duration().subsec_nanos() as f64),
    }
}

/// `data/settings.json`: {"folders": [the photo folders that were added]}.
pub fn settings(data: &Path) -> Map<String, Value> {
    let read = std::fs::read(data.join("settings.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    match read {
        Some(Value::Object(s)) => s,
        _ => json!({"folders": []}).as_object().cloned().unwrap_or_default(),
    }
}

/// The photo folders that were added.
pub fn folders(data: &Path) -> Vec<String> {
    let s = settings(data);
    s.get("folders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_str().map(str::to_string))
        .collect()
}

/// A folder's path in its one spelling: this system's separators, none doubled, none at
/// the end, no "." steps.
pub fn path_text(folder: &str) -> String {
    let win = cfg!(windows);
    let seps: &[char] = if win { &['/', '\\'] } else { &['/'] };
    let sep = std::path::MAIN_SEPARATOR_STR;
    let parts: Vec<&str> = folder.split(seps).filter(|p| !p.is_empty() && *p != ".").collect();
    if win {
        let unc = folder.starts_with(['/', '\\']) && folder[1..].starts_with(['/', '\\']) && parts.len() >= 2;
        if unc {
            // \\computer\share is one root
            return format!("\\\\{}\\{}\\{}", parts[0], parts[1], parts[2..].join(sep));
        }
        if let Some(drive) = parts.first().filter(|p| p.len() == 2 && p.ends_with(':')) {
            let rooted = folder[2..].starts_with(seps);
            return format!("{drive}{}{}", if rooted { sep } else { "" }, parts[1..].join(sep));
        }
    }
    let joined = parts.join(sep);
    match (folder.starts_with(seps), joined.is_empty()) {
        (true, _) => format!("{sep}{joined}"),
        (false, true) => ".".to_string(),
        (false, false) => joined,
    }
}

/// Add a photo folder to the ones that were added (once).
pub fn remember_folder(data: &Path, folder: &str) -> std::io::Result<()> {
    let mut s = settings(data);
    let folder = path_text(folder);
    let list = s.entry("folders").or_insert_with(|| json!([]));
    if let Some(list) = list.as_array_mut().filter(|l| !l.iter().any(|f| f.as_str() == Some(folder.as_str()))) {
        list.push(json!(folder));
        atomic_write(&data.join("settings.json"), filejson::rows_of(&s, true).as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_known_by_what_is_in_it() {
        let dir = std::env::temp_dir().join(format!("face-photofiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"abc").unwrap();
        assert_eq!(file_hash(&dir.join("a.bin")).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert!(file_hash(&dir.join("not there")).is_err() && !online_only(&dir.join("a.bin")));
        assert_eq!(make_local(&dir.join("a.bin")).unwrap(), false);
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!((thousands(999), thousands(1000), thousands(0)), ("999".into(), "1,000".into(), "0".into()));
        // the folders that were added: each once, in the file's fixed form
        assert!(folders(&dir).is_empty());
        let (a, b) =
            if cfg!(windows) { ("C:/Photos//2024/", "C:\\Photos\\2024") } else { ("/photos//2024/", "/photos/2024") };
        remember_folder(&dir, a).unwrap();
        remember_folder(&dir, b).unwrap();
        assert_eq!(folders(&dir), [b]);
        let written = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert_eq!(written, format!("{{\n\"folders\": [{}]\n}}", serde_json::to_string(b).unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_photo_record_says_whether_it_was_looked_at_for_possible_faces() {
        assert!(needs_possible(&json!({"path": "p.jpg"})));
        for done in [
            json!({"path": "p.jpg", "maybe": {}}),
            json!({"path": "p.mp4", "video": true}),
            json!({"path": "p.jpg", "dup_of": "h"}),
            json!({"path": ""}),
            json!({}),
        ] {
            assert!(!needs_possible(&done), "{done}");
        }
    }

    #[test]
    fn a_cameras_date_and_time_is_read_field_by_field() {
        assert_eq!(camera_fields("2024:05:17 14:03:22"), Some([2024, 5, 17, 14, 3, 22]));
        assert_eq!(camera_fields("2024:5:7 4:3:2"), Some([2024, 5, 7, 4, 3, 2]), "one digit will do");
        assert_eq!(
            camera_fields("2024:05: 7  14:03:22"),
            Some([2024, 5, 7, 14, 3, 22]),
            "a blank for a digit, two blanks between"
        );
        assert_eq!(camera_fields("2024:02:29 00:00:60"), Some([2024, 2, 29, 0, 0, 60]));
        for not in [
            "",
            "2024:05:17",
            "2024:05:17 14:03",
            "2024:13:01 00:00:00",
            "2023:02:29 00:00:00",
            "0000:00:00 00:00:00",
            "2024:05:17 24:00:00",
            "2024:05:17 14:03:22x",
            "2024-05-17 14:03:22",
            "    :  :     :  :  ",
            "?",
        ] {
            assert_eq!(camera_fields(not), None, "{not:?}");
        }
        // placed by this computer's own time zone, and the way back gives the same clock time
        let t = camera_time("2024:05:17 14:03:22 and more").unwrap().unwrap();
        assert_eq!(local(t), Some(Local { year: 2024, month: 5, day: 17, hour: 14, minute: 3, second: 22 }));
        assert_eq!((local_day(t), local_minute(t)), ("2024-05-17".to_string(), "2024-05-17 14:03".to_string()));
        assert_eq!(camera_time("no date").unwrap(), None);
        if cfg!(windows) {
            assert!(camera_time("1960:01:01 00:00:00").is_err(), "Windows places no time before 1970");
        }
    }

    #[test]
    fn a_fingerprint_changes_little_when_the_picture_changes_little() {
        // a picture that gets lighter to the right, then darker: the first eight places of a row rise
        let mut a = Bgr::new(170, 160);
        for y in 0..160 {
            for x in 0..170 {
                let v = if x < 85 { 40 + x } else { 210 - x } as u8;
                a.px[(y * 170 + x) * 3..(y * 170 + x) * 3 + 3].copy_from_slice(&[v, v, v]);
            }
        }
        let fp = fingerprint(&a);
        assert_eq!(fp.len(), 64);
        assert_eq!(&fp[..4], "ff00", "lighter than its left neighbour eight times, then not");
        assert!(fp.as_bytes().chunks(4).all(|row| row == b"ff00"));
        // one corner changed: a few bits
        let mut b = a.clone();
        for y in 0..10 {
            for x in 0..10 {
                b.px[(y * 170 + x) * 3..(y * 170 + x) * 3 + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let apart: u32 = (0..32)
            .map(|k| {
                (u8::from_str_radix(&fp[2 * k..2 * k + 2], 16).unwrap()
                    ^ u8::from_str_radix(&fingerprint(&b)[2 * k..2 * k + 2], 16).unwrap())
                .count_ones()
            })
            .sum();
        assert!((1..=4).contains(&apart), "{apart} bits");
        // colours weigh as OpenCV weighs them
        assert_eq!(grey(&Bgr { w: 3, h: 1, px: vec![255, 0, 0, 0, 255, 0, 0, 0, 255] }), [29, 150, 76]);
    }

    #[test]
    fn a_folder_is_written_in_its_one_spelling() {
        if cfg!(windows) {
            assert_eq!(path_text("C:/Users/x/Pictures/"), "C:\\Users\\x\\Pictures");
            assert_eq!(path_text("C:\\"), "C:\\");
            assert_eq!(path_text("//nas/photos/2024/./a"), "\\\\nas\\photos\\2024\\a");
            assert_eq!(path_text("relative\\\\folder"), "relative\\folder");
        } else {
            assert_eq!(path_text("/home/x//Pictures/"), "/home/x/Pictures");
            assert_eq!(path_text("relative/./folder"), "relative/folder");
            assert_eq!(path_text("/"), "/");
        }
    }
}
