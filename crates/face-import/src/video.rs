//! Videos: a frame about every second is looked at for faces, and of each person the clearest few are kept
//! (`video_frames`, `video_faces`, `video_frame`).
//!
//! The frames are decoded by FFmpeg, as
//! programs of their own beside this one (`ffmpeg` and `ffprobe`), asked for the frames wanted and handing
//! them over a pipe as plain pictures. They are looked for where `FACE_FFMPEG` names ffmpeg's file, else
//! beside the program, else in `<root>/runtime`. A frame is made red, green, blue as OpenCV makes it (the
//! conversion that takes every video for standard-definition colour, by code that is the same on every
//! processor), turned as the video says it is shown, and shrunk as photos are.

use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

use face_core::{
    faces::{Faces, REC_MODELS},
    geometry::iou,
    image::{Bgr, round_even},
    yunet::Row,
};
use serde_json::Value;

use crate::{decode::MAX_SIDE, yuv};

/// seconds between the video frames looked at
pub const VIDEO_EVERY: f64 = 1.0;
/// most frames looked at in one video (a long video's are spread over all of it)
pub const VIDEO_FRAMES: i64 = 300;
/// faces kept per person per video: the clearest ones ...
pub const VIDEO_KEEP: usize = 3;
/// ... at least this many seconds apart
pub const VIDEO_GAP: f64 = 3.0;

type Result<T> = std::result::Result<T, String>;

/// A face of a video worth keeping.
pub struct Found {
    /// the number of the frame it was found in
    pub frame: i64,
    pub row: Row,
    pub emb: Vec<f32>,
    pub crop: Bgr,
    pub quality: f32,
}

fn program(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// Where ffmpeg and ffprobe are, if they are anywhere.
pub fn find_tools() -> Option<(PathBuf, PathBuf)> {
    let pair = |ffmpeg: PathBuf| {
        let probe = ffmpeg.with_file_name(program("ffprobe"));
        (ffmpeg.is_file() && probe.is_file()).then_some((ffmpeg, probe))
    };
    if let Some(p) = std::env::var_os("FACE_FFMPEG").map(PathBuf::from) {
        return pair(p);
    }
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(program("ffmpeg"))));
    [beside, Some(face_files::paths::root().join("runtime").join(program("ffmpeg")))]
        .into_iter()
        .flatten()
        .find_map(pair)
}

fn tools() -> Result<(PathBuf, PathBuf)> {
    find_tools().ok_or_else(|| {
        "Videos need FFmpeg (ffmpeg and ffprobe), which wasn't found beside the program or in its runtime folder."
            .to_string()
    })
}

fn quiet(program: &Path) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // no console window
    }
    c
}

/// A video file as FFmpeg is told of it: a file on this computer and nothing else. FFmpeg takes a name for
/// an address (one that begins "http:" would be fetched, one that begins "-" read as an option), and a file
/// that is a playlist inside can name other addresses: so the name is given as a file's, and FFmpeg may
/// open files only (`ONLY_FILES`), whatever the video says.
fn as_file(path: &Path) -> std::ffi::OsString {
    let mut name = std::ffi::OsString::from("file:");
    name.push(path);
    name
}

/// FFmpeg's and ffprobe's options that keep them to files on this computer (see `as_file`).
const ONLY_FILES: [&str; 2] = ["-protocol_whitelist", "file,pipe"];

/// What a video says about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// frames a second (30 where the video doesn't say anything usable)
    pub fps: f64,
    /// how many frames it has, as far as it says
    pub frames: i64,
    /// how far to turn a frame clockwise to show it: 0, 90, 180 or 270
    pub turn: i32,
    /// its light and dark use the whole range (the old "J" kinds of picture)
    pub full_range: bool,
    /// a decoded picture's width and height (before it is turned)
    pub width: usize,
    pub height: usize,
    /// the kind of decoded picture, where the program converts it itself (`yuv.rs`); None: left to FFmpeg
    pub kind: Option<yuv::Kind>,
}

fn ratio(text: &str) -> f64 {
    match text.split_once('/') {
        Some((a, b)) => match (a.parse::<f64>(), b.parse::<f64>()) {
            (Ok(a), Ok(b)) if b != 0.0 => a / b,
            _ => 0.0,
        },
        None => text.parse().unwrap_or(0.0),
    }
}

/// A video's frame rate, frame count and turn, as OpenCV reports them for it.
pub fn probe(path: &Path) -> Result<Info> {
    let (_, ffprobe) = tools()?;
    let out = quiet(&ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=pix_fmt,width,height,avg_frame_rate,r_frame_rate,nb_frames,duration:stream_side_data=rotation:format=duration")
        .args(["-of", "json"])
        .args(ONLY_FILES)
        .arg(as_file(path))
        .output()
        .map_err(|e| format!("ffprobe couldn't be run: {e}"))?;
    let said: Value = serde_json::from_slice(&out.stdout).map_err(|_| "can't open video".to_string())?;
    let stream = said["streams"].get(0).ok_or("can't open video")?;
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    let mut fps = ratio(&text(&stream["avg_frame_rate"]));
    if fps < 1e-9 {
        fps = ratio(&text(&stream["r_frame_rate"]));
    }
    let seconds =
        [&stream["duration"], &said["format"]["duration"]].iter().find_map(|d| d.as_str()?.parse::<f64>().ok());
    let mut frames = text(&stream["nb_frames"]).parse::<i64>().unwrap_or(0);
    if frames == 0 {
        frames = seconds.map_or(0, |s| (s * fps + 0.5).floor() as i64);
    }
    let rotation =
        (stream["side_data_list"].as_array().into_iter().flatten()).find_map(|d| d["rotation"].as_f64()).unwrap_or(0.0);
    // the video says how far it is turned the other way round; a frame is turned back by as much
    let turn = ((-rotation).round() as i32).rem_euclid(360);
    Ok(Info {
        fps: if fps > 0.0 && fps < 1000.0 { fps } else { 30.0 },
        frames,
        turn,
        full_range: text(&stream["pix_fmt"]).starts_with("yuvj"),
        width: stream["width"].as_u64().unwrap_or(0) as usize,
        height: stream["height"].as_u64().unwrap_or(0) as usize,
        kind: yuv::Kind::of(&text(&stream["pix_fmt"])),
    })
}

/// A frame turned as the video says it is shown.
fn turned(p: Bgr, turn: i32) -> Bgr {
    let (w, h) = (p.w, p.h);
    let (ow, oh) = if turn == 90 || turn == 270 { (h, w) } else { (w, h) };
    if !matches!(turn, 90 | 180 | 270) {
        return p;
    }
    let mut px = vec![0u8; ow * oh * 3];
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = match turn {
                90 => (y, h - 1 - x),
                180 => (w - 1 - x, h - 1 - y),
                _ => (w - 1 - y, x),
            };
            px[(y * ow + x) * 3..(y * ow + x) * 3 + 3].copy_from_slice(&p.px[(sy * w + sx) * 3..(sy * w + sx) * 3 + 3]);
        }
    }
    Bgr { w: ow, h: oh, px }
}

/// A frame shrunk to `MAX_SIDE`, as photos are (the faces' boxes are kept at this size).
fn fit(frame: Bgr) -> Bgr {
    let s = MAX_SIDE as f64 / frame.w.max(frame.h) as f64;
    if s < 1.0 {
        frame.resize_area(round_even(frame.w as f64 * s) as usize, round_even(frame.h as f64 * s) as usize)
    } else {
        frame
    }
}

/// The frames that `select` picks, in order: `take(how many came before, the frame)` until it says to stop.
/// `from`: a time in the video (its own clock, in seconds) to begin reading at; the frames keep their times.
fn read_frames(
    path: &Path,
    info: &Info,
    select: &str,
    limit: Option<usize>,
    from: Option<f64>,
    mut take: impl FnMut(usize, Bgr) -> Result<bool>,
) -> Result<()> {
    let (ffmpeg, _) = tools()?;
    if info.width == 0 || info.height == 0 {
        return Err("the video doesn't say how large its pictures are".to_string());
    }
    // The decoded picture is taken as it is and its colours are converted here (`yuv.rs`): FFmpeg's own
    // conversion gives other values from one processor and build to the next, its decoders the same bytes
    // everywhere (measured), the older kinds once they are asked to decode exactly ("bitexact"). A kind of
    // picture `yuv.rs` doesn't know (rare: no camera or phone writes one) is made into one it knows by
    // FFmpeg first. The frame is neither turned nor cut to the container's "clean" part by FFmpeg: OpenCV
    // turned it afterwards and didn't cut it, and so does this.
    let (kind, filter) = match info.kind {
        Some(kind) => (kind, format!("select='{select}'")),
        None => (
            yuv::Kind { colour: yuv::Colour::Full, bits: 8, full_range: false },
            format!("select='{select}',scale=flags=bicubic+bitexact:out_range=tv,format=yuv444p"),
        ),
    };
    let mut run = quiet(&ffmpeg);
    run.args(["-v", "error", "-nostdin", "-noautorotate", "-apply_cropping", "codec", "-flags:v", "+bitexact"])
        .args(ONLY_FILES);
    if let Some(from) = from {
        run.args(["-seek_timestamp", "1", "-ss", &format!("{from:.6}"), "-copyts"]);
    }
    run.arg("-i").arg(as_file(path)).args(["-map", "0:v:0", "-vf", &filter, "-fps_mode", "passthrough"]);
    if let Some(n) = limit {
        run.args(["-frames:v", &n.to_string()]);
    }
    let mut child = run
        .args(["-f", "rawvideo", "pipe:1"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg couldn't be run: {e}"))?;
    let mut out = BufReader::with_capacity(1 << 20, child.stdout.take().ok_or("ffmpeg gave no pipe")?);
    let mut done = Ok(());
    let mut n = 0;
    let mut raw = vec![0u8; kind.frame_bytes(info.width, info.height)];
    loop {
        // a decoded picture: its planes one after the other, nothing between the pictures
        if out.read_exact(&mut raw).is_err() {
            break; // no more frames (or cut off in the middle of one: the frames before it stand)
        }
        let Some(frame) = yuv::to_bgr(kind, info.width, info.height, &raw) else {
            done = Err("ffmpeg handed over something that is no picture".to_string());
            break;
        };
        match take(n, fit(turned(frame, info.turn))) {
            Ok(true) => n += 1,
            Ok(false) => break,
            Err(e) => {
                done = Err(e);
                break;
            }
        }
    }
    drop(out);
    let _ = child.kill();
    let _ = child.wait();
    done
}

/// `take(frame number, frames a second, the frame)` for a frame about every `VIDEO_EVERY` seconds, at most
/// `VIDEO_FRAMES` of them, until it says to stop. The video's frames a second.
pub fn frames(path: &Path, stop: &AtomicBool, mut take: impl FnMut(i64, f64, Bgr) -> Result<bool>) -> Result<f64> {
    let info = probe(path)
        .map_err(|_| format!("can't open video {}", path.file_name().unwrap_or_default().to_string_lossy()))?;
    let step = 1.max((info.fps * VIDEO_EVERY).round_ties_even() as i64).max(info.frames / VIDEO_FRAMES);
    read_frames(path, &info, &format!("not(mod(n,{step}))"), None, None, |n, frame| {
        if stop.load(Ordering::SeqCst) {
            return Ok(false);
        }
        take(n as i64 * step, info.fps, frame)
    })?;
    Ok(info.fps)
}

/// From which frame number on a single frame is jumped to and not counted to: counting decodes every frame
/// before it (6 s for a frame 16 minutes into a video, measured), a jump asks the video for its list of
/// pictures first (a fifth of a second) and decodes from the last whole picture before the frame.
const JUMP_FROM: i64 = 120;

/// One frame of a video by its number, shrunk as photos are.
pub fn frame(path: &Path, k: i64) -> Result<Bgr> {
    let info = probe(path)?;
    if k >= JUMP_FROM
        && let Some(frame) = jumped(path, &info, k)
    {
        return Ok(frame);
    }
    counted(path, &info, k)
}

/// Frame `k`, read from the video's beginning.
fn counted(path: &Path, info: &Info, k: i64) -> Result<Bgr> {
    let mut got = None;
    read_frames(path, info, &format!("eq(n,{k})"), Some(1), None, |_, frame| {
        got = Some(frame);
        Ok(false)
    })?;
    got.ok_or_else(|| "the video has no such frame".to_string())
}

/// Frame `k`, read from the last whole picture before it: the video's list of pictures says when each is
/// shown, the frame is the `k`th of them by that time, and reading begins at the one before it. None where
/// the video doesn't say (the frame is then counted to), or holds no such frame.
fn jumped(path: &Path, info: &Info, k: i64) -> Option<Bgr> {
    let (before, at) = shown_at(path, k)?;
    let mut got = None;
    // the first frame shown later than halfway between the one before and this one: this one
    let select = format!("gte(t,{:.6})", (before + at) / 2.0);
    read_frames(path, info, &select, Some(1), Some(before), |_, frame| {
        got = Some(frame);
        Ok(false)
    })
    .ok()?;
    got
}

/// When frames `k - 1` and `k` of a video are shown, in seconds of the video's own clock: from its list of
/// pictures (no picture is decoded for it), those left out that the video itself says aren't shown. None
/// where a picture has no time, two share one, or there is no frame `k`.
fn shown_at(path: &Path, k: i64) -> Option<(f64, f64)> {
    let (_, ffprobe) = tools().ok()?;
    let out = quiet(&ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts_time,flags", "-of", "csv=p=0"])
        .args(ONLY_FILES)
        .arg(as_file(path))
        .output()
        .ok()?;
    let mut times = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let (time, flags) = line.split_once(',')?;
        if !flags.contains('D') {
            times.push(time.trim().parse::<f64>().ok()?);
        }
    }
    times.sort_by(f64::total_cmp);
    let k = usize::try_from(k).ok().filter(|k| *k >= 1 && *k < times.len())?;
    (times[k] > times[k - 1]).then_some((times[k - 1], times[k]))
}

/// For the checks: frame `k` by a jump alone (None where `frame` would count to it), to hold the jump to the
/// counting.
pub fn frame_by_jump(path: &Path, k: i64) -> Option<Bgr> {
    jumped(path, &probe(path).ok()?, k)
}

/// (frames a second, the faces worth keeping) of a video. The same person's faces across the video are put
/// together, and only that person's `VIDEO_KEEP` clearest faces, `VIDEO_GAP` seconds apart, are kept, so a long
/// clip of one person adds a few faces, not hundreds. The same person: a face where theirs was in the frame
/// looked at just before (with the head turned down or away the print alone often doesn't match), else a
/// print at least as alike any of theirs as the recognizer asks.
pub fn faces(faces: &Faces, path: &Path, stop: &AtomicBool) -> Result<(f64, Vec<Found>)> {
    struct Person {
        prints: Vec<Vec<f32>>,
        seen: Vec<Found>,
    }
    let match_at = REC_MODELS[0].match_at;
    let mut people: Vec<Person> = Vec::new();
    let mut prev: Option<i64> = None;
    let fps = frames(path, stop, |k, _, frame| {
        let mut taken = vec![false; people.len()]; // one face per person per frame
        for face in faces.detect(&frame, &[10_000, 640]).map_err(|e| e.0)? {
            // 1. followed: in the same place as in the frame looked at before
            let (mut best, mut best_s) = (None, 0.3f32);
            for (n, p) in people.iter().enumerate() {
                let Some(last) = p.seen.last() else { continue };
                if !taken[n] && Some(last.frame) == prev {
                    let s = iou(&face.row, &last.row);
                    if s >= best_s {
                        (best, best_s) = (Some(n), s);
                    }
                }
            }
            // 2. recognized: alike any face of theirs seen so far
            if best.is_none() {
                let mut best_s = match_at;
                for (n, p) in people.iter().enumerate() {
                    if taken[n] {
                        continue;
                    }
                    let most = p
                        .prints
                        .iter()
                        .map(|e| face_library::scores::dot(e, &face.emb))
                        .fold(f32::NEG_INFINITY, f32::max) as f64;
                    if most >= best_s {
                        (best, best_s) = (Some(n), most);
                    }
                }
            }
            let n = match best {
                Some(n) => n,
                None => {
                    people.push(Person { prints: Vec::new(), seen: Vec::new() });
                    taken.push(false);
                    people.len() - 1
                }
            };
            taken[n] = true;
            people[n].prints.push(face.emb.clone());
            people[n].seen.push(Found {
                frame: k,
                row: face.row,
                emb: face.emb,
                crop: face.crop,
                quality: face.quality,
            });
        }
        prev = Some(k);
        Ok(true)
    })?;
    let mut out = Vec::new();
    for mut person in people {
        // the clearest first (a face that wasn't scored counts as the least clear)
        let clear = |f: &Found| if f.quality.is_nan() { 0.0 } else { f.quality };
        person.seen.sort_by(|a, b| clear(b).total_cmp(&clear(a)));
        let mut kept: Vec<Found> = Vec::new();
        for face in person.seen {
            if kept.len() < VIDEO_KEEP && kept.iter().all(|o| (face.frame - o.frame).abs() as f64 >= VIDEO_GAP * fps) {
                kept.push(face);
            }
        }
        kept.sort_by_key(|f| f.frame);
        out.extend(kept);
    }
    Ok((fps, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_turned_as_the_video_says() {
        // 3 wide, 2 high:  0 1 2 / 3 4 5
        let frame = || Bgr { w: 3, h: 2, px: (0..6u8).flat_map(|i| [i, 0, 0]).collect() };
        let blues = |p: Bgr| (p.w, p.h, p.px.chunks_exact(3).map(|c| c[0]).collect::<Vec<_>>());
        assert_eq!(blues(turned(frame(), 0)), (3, 2, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(blues(turned(frame(), 90)), (2, 3, vec![3, 0, 4, 1, 5, 2]), "a quarter turn clockwise");
        assert_eq!(blues(turned(frame(), 180)), (3, 2, vec![5, 4, 3, 2, 1, 0]));
        assert_eq!(blues(turned(frame(), 270)), (2, 3, vec![2, 5, 1, 4, 0, 3]), "a quarter turn the other way");
        assert_eq!((ratio("30000/1001") * 1000.0).round(), 29970.0);
        assert_eq!((ratio("25"), ratio("0/0"), ratio("")), (25.0, 0.0, 0.0));
        let small = fit(Bgr::new(640, 360));
        assert_eq!((small.w, small.h), (640, 360), "a small frame stays as it is");
        let big = fit(Bgr::new(3840, 2160));
        assert_eq!((big.w, big.h), (1280, 720));
    }
}
