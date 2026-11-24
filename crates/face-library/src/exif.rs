//! What a picture file says about itself without being decoded: its size, which way up it is, and when the
//! camera took it. All of it is read from the file's own headers, so that a photo's facts never
//! depend on a decoder:
//!
//! - JPEG: the size from the frame header, EXIF from the "Exif" segment before the picture's data (and the
//!   way up from the XMP segment when EXIF doesn't say);
//! - PNG, WebP: the size from the header, EXIF from its own chunk;
//! - GIF, BMP: the size; these carry no EXIF;
//! - HEIC, HEIF, AVIF: the main picture's size as it is shown (turned, where the file says to turn it), and
//!   so always "upright": the picture is decoded turned, and its EXIF orientation counts as 1.
//!
//! Anything else (a video, a file that isn't a picture) is an error.

use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};

type Result<T> = std::result::Result<T, String>;

/// What a picture file says about itself.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Header {
    /// as stored (for HEIC: as shown)
    pub width: u32,
    pub height: u32,
    /// EXIF's orientation, 1 (upright) where the file doesn't say
    pub orientation: u32,
    /// EXIF's "taken at" (DateTimeOriginal), else its date and time, as the file has it:
    /// "2024:05:17 14:03:22"
    pub taken: Option<String>,
}

/// A file read a few bytes here and there.
struct Src<R> {
    r: R,
    len: u64,
}

impl<R: Read + Seek> Src<R> {
    /// Up to `n` bytes from `at` (fewer at the file's end).
    fn upto(&mut self, at: u64, n: usize) -> Vec<u8> {
        let n = n.min(self.len.saturating_sub(at) as usize);
        let mut buf = vec![0u8; n];
        if n == 0 || self.r.seek(SeekFrom::Start(at)).is_err() {
            return Vec::new();
        }
        let mut got = 0;
        while got < n {
            match self.r.read(&mut buf[got..]) {
                Ok(0) | Err(_) => break,
                Ok(k) => got += k,
            }
        }
        buf.truncate(got);
        buf
    }

    /// Exactly `n` bytes from `at`.
    fn at(&mut self, at: u64, n: usize) -> Result<Vec<u8>> {
        let got = self.upto(at, n);
        if got.len() == n { Ok(got) } else { Err("the file ends early".to_string()) }
    }
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 2).map(|v| u16::from_be_bytes([v[0], v[1]]) as u32)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]))
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 2).map(|v| u16::from_le_bytes([v[0], v[1]]) as u32)
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}

/// EXIF: a small TIFF file. (the way up, the date) from its first directory and the EXIF
/// directory it points to.
fn exif(data: &[u8]) -> (Option<u32>, Option<String>) {
    let mut d = data;
    for _ in 0..2 {
        d = d.strip_prefix(b"Exif\0\0").unwrap_or(d);
    }
    let le = match d.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return (None, None),
    };
    let u16_at = |at: usize| if le { le16(d, at) } else { be16(d, at) };
    let u32_at = |at: usize| if le { le32(d, at) } else { be32(d, at) };
    // one entry of a directory: (type, how many, where its value is)
    let entry = |dir: usize, tag: u32| -> Option<(u32, usize, usize)> {
        let n = u16_at(dir)? as usize;
        (0..n).map(|k| dir + 2 + 12 * k).find(|at| u16_at(*at) == Some(tag)).and_then(|at| {
            let (kind, count) = (u16_at(at + 2)?, u32_at(at + 4)? as usize);
            let size = count.checked_mul(match kind {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 => 8,
                _ => return None,
            })?;
            let value = if size <= 4 { at + 8 } else { u32_at(at + 8)? as usize };
            d.get(value..value.checked_add(size)?).map(|_| (kind, count, value))
        })
    };
    let whole = |dir: usize, tag: u32| -> Option<u32> {
        match entry(dir, tag)? {
            (_, 0, _) => None,
            (1, _, at) => d.get(at).map(|b| *b as u32),
            (3, _, at) => u16_at(at),
            (4, _, at) => u32_at(at),
            _ => None,
        }
    };
    // text without the zero that ends it, each byte a character. A value that isn't
    // text is something, but no date: "?" stands for it
    let text = |dir: usize, tag: u32| -> Option<String> {
        match entry(dir, tag)? {
            (2, count, at) => {
                let raw = &d[at..at + count];
                Some(raw.strip_suffix(b"\0").unwrap_or(raw).iter().map(|b| *b as char).collect())
            }
            (_, 0, _) => None,
            _ => Some("?".to_string()),
        }
    };
    let Some(first) = u32_at(4).map(|at| at as usize) else { return (None, None) };
    let taken = (whole(first, 0x8769).and_then(|sub| text(sub as usize, 0x9003)).filter(|t| !t.is_empty()))
        .or_else(|| text(first, 0x0132).filter(|t| !t.is_empty()));
    (whole(first, 0x0112), taken)
}

/// The way up that an XMP packet says, where EXIF doesn't: `tiff:Orientation="6"` or `>6<`.
fn xmp_orientation(xmp: &[u8]) -> Option<u32> {
    const KEY: &[u8] = b"tiff:Orientation";
    let mut from = 0;
    while let Some(k) = xmp[from..].windows(KEY.len()).position(|w| w == KEY) {
        let rest = &xmp[from + k + KEY.len()..];
        let digit = match rest {
            [b'=', b'"', d, ..] | [b'>', d, ..] if d.is_ascii_digit() => Some((*d - b'0') as u32),
            _ => None,
        };
        if digit.is_some() {
            return digit;
        }
        from += k + KEY.len();
    }
    None
}

fn jpeg<R: Read + Seek>(src: &mut Src<R>) -> Result<Header> {
    let (mut at, mut found, mut xmp) = (2u64, Vec::new(), None);
    let mut size = None;
    loop {
        let head = src.at(at, 4)?;
        if head[0] != 0xff {
            return Err("not a JPEG's marker".to_string());
        }
        let marker = head[1];
        if marker == 0xff {
            at += 1; // a fill byte
            continue;
        }
        if matches!(marker, 0x00 | 0x01 | 0xd0..=0xd9) {
            at += 2;
            continue;
        }
        let n = be16(&head, 2).unwrap_or(2).max(2) as usize;
        match marker {
            // the frame header: the picture's size
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                let frame = src.at(at + 4, 5)?;
                size = Some((be16(&frame, 3).unwrap_or(0), be16(&frame, 1).unwrap_or(0)));
            }
            0xe1 => {
                let body = src.at(at + 4, n - 2)?;
                if let Some(rest) = body.strip_prefix(b"Exif\0\0") {
                    // a long EXIF goes on in the next segment
                    found.extend_from_slice(if found.is_empty() { &body } else { rest });
                } else if body.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
                    xmp = Some(body[29..].to_vec());
                }
            }
            0xda => break, // the picture's data begins: nothing is read beyond it
            _ => {}
        }
        at += 2 + n as u64;
    }
    let (width, height) = size.ok_or("a JPEG without a frame header")?;
    let (way, taken) = exif(&found);
    let way = way.or_else(|| xmp.as_deref().and_then(xmp_orientation));
    Ok(Header { width, height, orientation: way.unwrap_or(1), taken })
}

fn png<R: Read + Seek>(src: &mut Src<R>) -> Result<Header> {
    let first = src.at(8, 25)?;
    if &first[4..8] != b"IHDR" {
        return Err("a PNG without its header".to_string());
    }
    let (width, height) = (be32(&first, 8).unwrap_or(0), be32(&first, 12).unwrap_or(0));
    let (mut at, mut found, mut xmp) = (8u64, None, None);
    while let Ok(head) = src.at(at, 8) {
        let n = be32(&head, 0).unwrap_or(0) as u64;
        match &head[4..8] {
            b"eXIf" => found = src.at(at + 8, n as usize).ok(),
            b"iTXt" if n < 1 << 20 => {
                // the XMP packet, as text: keyword, zero, two flags, language, zero, translated keyword, zero
                let body = src.at(at + 8, n as usize).unwrap_or_default();
                if let Some(rest) = body.strip_prefix(b"XML:com.adobe.xmp\0\0\0") {
                    let text = rest.splitn(3, |b| *b == 0).nth(2).unwrap_or(&[]);
                    xmp = Some(text.to_vec());
                }
            }
            b"IEND" => break,
            _ => {}
        }
        at += 12 + n;
    }
    let (way, taken) = found.as_deref().map_or((None, None), exif);
    let way = way.or_else(|| xmp.as_deref().and_then(xmp_orientation));
    Ok(Header { width, height, orientation: way.unwrap_or(1), taken })
}

fn webp<R: Read + Seek>(src: &mut Src<R>) -> Result<Header> {
    let (mut at, mut size, mut found) = (12u64, None, None);
    while let Ok(head) = src.at(at, 8) {
        let n = le32(&head, 4).unwrap_or(0) as u64;
        match &head[0..4] {
            b"VP8X" => {
                let b = src.at(at + 8, 10)?;
                let three = |k: usize| (b[k] as u32 | (b[k + 1] as u32) << 8 | (b[k + 2] as u32) << 16) + 1;
                size = Some((three(4), three(7)));
            }
            b"VP8 " if size.is_none() => {
                let b = src.at(at + 8, 10)?;
                size = Some((le16(&b, 6).unwrap_or(0) & 0x3fff, le16(&b, 8).unwrap_or(0) & 0x3fff));
            }
            b"VP8L" if size.is_none() => {
                let b = src.at(at + 8, 5)?;
                let bits = le32(&b, 1).unwrap_or(0);
                size = Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1));
            }
            b"EXIF" => found = src.at(at + 8, n as usize).ok(),
            _ => {}
        }
        at += 8 + n + (n & 1);
    }
    let (width, height) = size.ok_or("a WebP without a picture")?;
    let (way, taken) = found.as_deref().map_or((None, None), exif);
    Ok(Header { width, height, orientation: way.unwrap_or(1), taken })
}

/// One box of an ISO media file: (its type, where its contents begin, where it ends).
fn boxed<R: Read + Seek>(src: &mut Src<R>, at: u64, end: u64) -> Option<([u8; 4], u64, u64)> {
    if at + 8 > end {
        return None;
    }
    let head = src.at(at, 8).ok()?;
    let kind = [head[4], head[5], head[6], head[7]];
    let (body, stop) = match be32(&head, 0)? {
        0 => (at + 8, end),
        1 => {
            let long = src.at(at + 8, 8).ok()?;
            (at + 16, at + u64::from_be_bytes(long.try_into().ok()?))
        }
        n => (at + 8, at + n as u64),
    };
    (stop >= body && stop <= end).then_some((kind, body, stop))
}

/// A whole number of 0, 2, 4 or 8 bytes, as the item lists of a HEIC file have them.
fn sized(b: &[u8], at: &mut usize, size: usize) -> Option<u64> {
    let v = b.get(*at..*at + size)?.iter().fold(0u64, |v, x| v << 8 | *x as u64);
    *at += size;
    Some(v)
}

fn heif<R: Read + Seek>(src: &mut Src<R>) -> Result<Header> {
    let len = src.len;
    let mut at = 0;
    let mut meta = None;
    while let Some((kind, body, stop)) = boxed(src, at, len) {
        if &kind == b"meta" {
            meta = Some((body + 4, stop)); // a version and flags come first
            break;
        }
        at = stop;
    }
    let (start, end) = meta.ok_or("no list of the file's pictures")?;
    if end - start > 16 << 20 {
        return Err("the list of the file's pictures is too large".to_string());
    }
    let (mut main, mut exif_items, mut places, mut props, mut uses) =
        (None, Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut at = start;
    while let Some((kind, body, stop)) = boxed(src, at, end) {
        let b = src.at(body, (stop - body) as usize)?;
        match &kind {
            b"pitm" => main = if b.first() == Some(&0) { be16(&b, 4) } else { be32(&b, 4) },
            b"iinf" => {
                // each entry a box: version, flags, the item's number, a protection number, its type
                let mut p = if b.first() == Some(&0) { 6 } else { 8 };
                while let (Some(n), Some(kind)) = (be32(&b, p), b.get(p + 4..p + 8)) {
                    if kind == b"infe" {
                        let (id, at_type) = match b.get(p + 8) {
                            Some(2) => (be16(&b, p + 12), p + 16),
                            Some(3) => (be32(&b, p + 12), p + 18),
                            _ => (None, 0),
                        };
                        if let (Some(id), Some(b"Exif")) = (id, b.get(at_type..at_type + 4)) {
                            exif_items.push(id);
                        }
                    }
                    if n < 8 {
                        break;
                    }
                    p += n as usize;
                }
            }
            b"iloc" => {
                let version = b.first().copied().unwrap_or(0);
                let (sizes, more) = (b.get(4).copied().unwrap_or(0), b.get(5).copied().unwrap_or(0));
                let (off, length, base, index) =
                    ((sizes >> 4) as usize, (sizes & 15) as usize, (more >> 4) as usize, (more & 15) as usize);
                let mut p = 6;
                let count = sized(&b, &mut p, if version < 2 { 2 } else { 4 }).unwrap_or(0);
                for _ in 0..count {
                    let Some(id) = sized(&b, &mut p, if version < 2 { 2 } else { 4 }) else { break };
                    let method = if version > 0 { sized(&b, &mut p, 2).unwrap_or(0) & 15 } else { 0 };
                    p += 2; // which file: this one
                    let (Some(base), Some(extents)) = (sized(&b, &mut p, base), sized(&b, &mut p, 2)) else { break };
                    for _ in 0..extents {
                        if version > 0 {
                            p += index;
                        }
                        let (Some(o), Some(l)) = (sized(&b, &mut p, off), sized(&b, &mut p, length)) else { break };
                        places.push((id as u32, method, base + o, l));
                    }
                }
            }
            b"iprp" => {
                let mut q = body;
                while let Some((kind, inner, inner_stop)) = boxed(src, q, stop) {
                    let c = src.at(inner, (inner_stop - inner) as usize)?;
                    if &kind == b"ipco" {
                        let mut r = inner;
                        while let Some((kind, pb, ps)) = boxed(src, r, inner_stop) {
                            props.push((kind, src.at(pb, (ps - pb).min(64) as usize)?));
                            r = ps;
                        }
                    } else if &kind == b"ipma" {
                        let (version, flags) = (c.first().copied().unwrap_or(0), c.get(3).copied().unwrap_or(0));
                        let mut p = 4;
                        let entries = sized(&c, &mut p, 4).unwrap_or(0);
                        for _ in 0..entries {
                            let Some(id) = sized(&c, &mut p, if version < 1 { 2 } else { 4 }) else { break };
                            let n = sized(&c, &mut p, 1).unwrap_or(0);
                            for _ in 0..n {
                                let Some(v) = sized(&c, &mut p, if flags & 1 == 1 { 2 } else { 1 }) else { break };
                                uses.push((id as u32, (v & if flags & 1 == 1 { 0x7fff } else { 0x7f }) as usize));
                            }
                        }
                    }
                    q = inner_stop;
                }
            }
            _ => {}
        }
        at = stop;
    }
    let main = main.ok_or("no main picture")?;
    // the main picture's size, then what the file says to do with it before it is shown, in the file's order
    let mut size: Option<(u32, u32)> = None;
    for (_, k) in uses.iter().filter(|(id, _)| *id == main) {
        let Some((kind, b)) = k.checked_sub(1).and_then(|k| props.get(k)) else { continue };
        match (kind, size) {
            (b"ispe", _) => size = be32(b, 4).zip(be32(b, 8)),
            (b"irot", Some((w, h))) if b.first().is_some_and(|a| a & 1 == 1) => size = Some((h, w)),
            (b"clap", Some((w, h))) => {
                // cut to the clean part: its width and height as fractions
                let part = |n: usize, whole: u32| match (be32(b, n), be32(b, n + 4)) {
                    (Some(a), Some(d)) if d > 0 => (a as f64 / d as f64).round().clamp(1.0, whole as f64) as u32,
                    _ => whole,
                };
                size = Some((part(0, w), part(8, h)));
            }
            _ => {}
        }
    }
    let (width, height) = size.ok_or("the main picture has no size")?;
    // EXIF: the first such item with anything in it. Its first four bytes say where the TIFF part begins
    let mut taken = None;
    for id in exif_items {
        let mut data = Vec::new();
        for (_, method, at, n) in places.iter().filter(|(item, ..)| *item == id) {
            if *method == 0 && *n < 16 << 20 {
                data.extend(src.upto(*at, *n as usize));
            }
        }
        let mut skip = be32(&data, 0).map_or(4, |n| n as usize + 4);
        if data.len() <= skip.saturating_add(4) {
            skip = 4;
        }
        let Some(tiff) = data.get(skip..).filter(|t| !t.is_empty()) else { continue };
        taken = exif(tiff).1;
        break;
    }
    // shown turned already, so upright whatever EXIF says
    Ok(Header { width, height, orientation: 1, taken })
}

/// What the picture in this file says about itself.
pub fn header<R: Read + Seek>(mut r: R) -> Result<Header> {
    let len = r.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    let mut src = Src { r, len };
    let first = src.upto(0, 32);
    const STILL: [&[u8; 4]; 12] =
        [b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs", b"mif1", b"msf1", b"avif", b"avis"];
    match first.as_slice() {
        [0xff, 0xd8, ..] => jpeg(&mut src),
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => png(&mut src),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] if first.len() >= 10 => Ok(Header {
            width: le16(&first, 6).unwrap_or(0),
            height: le16(&first, 8).unwrap_or(0),
            orientation: 1,
            taken: None,
        }),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => webp(&mut src),
        [b'B', b'M', ..] if first.len() >= 26 => {
            let side = |at: usize| le32(&first, at).map_or(0, |v| (v as i32).unsigned_abs());
            Ok(Header { width: side(18), height: side(22), orientation: 1, taken: None })
        }
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => {
            // the kinds of file it says it is: a picture's among them, or it is a video
            let n = be32(&first, 0).unwrap_or(0).clamp(16, 512) as usize;
            let kinds = src.upto(8, n - 8);
            let still =
                kinds.chunks_exact(4).enumerate().any(|(k, c)| k != 1 && STILL.iter().any(|s| s.as_slice() == c));
            if still { heif(&mut src) } else { Err("a video, not a picture".to_string()) }
        }
        _ => Err("not a picture this can read".to_string()),
    }
}

/// What the picture file at `path` says about itself.
pub fn header_of(path: &Path) -> Result<Header> {
    header(std::io::BufReader::with_capacity(4096, std::fs::File::open(path).map_err(|e| e.to_string())?))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use super::*;

    /// A small TIFF as EXIF has it: the way up, a date and time, and in the EXIF directory when it was taken.
    pub fn tiff(le: bool, way: Option<u16>, stamp: Option<&str>, taken: Option<&str>) -> Vec<u8> {
        let w16 = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let w32 = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut entries: Vec<(u16, u16, u32, Vec<u8>)> = Vec::new();
        if let Some(way) = way {
            entries.push((0x0112, 3, 1, [w16(way), [0, 0]].concat()));
        }
        if let Some(stamp) = stamp {
            entries.push((0x0132, 2, stamp.len() as u32 + 1, [stamp.as_bytes(), b"\0"].concat()));
        }
        let sub_at = 8 + 2 + 12 * (entries.len() + taken.is_some() as usize) + 4;
        let mut far: Vec<u8> = Vec::new();
        if let Some(taken) = taken {
            // the EXIF directory: one entry, its text right after it
            let text_at = sub_at + 2 + 12 + 4;
            far.extend(w16(1));
            far.extend(
                [w16(0x9003).as_slice(), &w16(2), &w32(taken.len() as u32 + 1), &w32(text_at as u32), &w32(0)].concat(),
            );
            far.extend(taken.as_bytes());
            far.push(0);
            entries.push((0x8769, 4, 1, w32(sub_at as u32).to_vec()));
        }
        let mut out: Vec<u8> = if le { b"II*\0".to_vec() } else { b"MM\0*".to_vec() };
        out.extend(w32(8));
        out.extend(w16(entries.len() as u16));
        let mut tail = far;
        for (tag, kind, count, value) in &entries {
            out.extend([w16(*tag), w16(*kind)].concat());
            out.extend(w32(*count));
            if value.len() <= 4 {
                out.extend(value.iter().chain([0u8; 4].iter()).take(4));
            } else {
                out.extend(w32((sub_at + tail.len()) as u32));
                tail.extend(value);
            }
        }
        out.extend(w32(0));
        out.extend(tail);
        out
    }

    fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
        [&[0xff, marker][..], &(body.len() as u16 + 2).to_be_bytes(), body].concat()
    }

    /// A JPEG's headers (no picture): EXIF, an XMP packet, the frame header, the start of the data.
    pub fn jpeg_file(w: u16, h: u16, exif: Option<&[u8]>, xmp: Option<&str>) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8];
        out.extend(segment(0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
        if let Some(exif) = exif {
            out.extend(segment(0xe1, &[b"Exif\0\0", exif].concat()));
        }
        if let Some(xmp) = xmp {
            out.extend(segment(0xe1, &[b"http://ns.adobe.com/xap/1.0/\0", xmp.as_bytes()].concat()));
        }
        out.extend(segment(
            0xc0,
            &[&[8u8][..], &h.to_be_bytes(), &w.to_be_bytes(), &[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]].concat(),
        ));
        out.extend(segment(0xda, &[1, 1, 0, 0, 63, 0]));
        out.extend([0x12, 0x34, 0xff, 0xd9]);
        out
    }

    fn read(file: &[u8]) -> Result<Header> {
        header(Cursor::new(file.to_vec()))
    }

    #[test]
    fn a_jpeg_says_its_size_its_way_up_and_when_it_was_taken() {
        for le in [true, false] {
            let file = jpeg_file(
                4032,
                3024,
                Some(&tiff(le, Some(6), Some("2024:05:18 09:00:00"), Some("2024:05:17 14:03:22"))),
                None,
            );
            let said = read(&file).unwrap();
            assert_eq!((said.width, said.height, said.orientation), (4032, 3024, 6));
            assert_eq!(said.taken.as_deref(), Some("2024:05:17 14:03:22"), "when it was taken comes first");
        }
        // only a date and time; nothing at all; the way up from the XMP packet when EXIF doesn't say
        let file = jpeg_file(10, 20, Some(&tiff(true, None, Some("2020:01:02 03:04:05"), None)), None);
        assert_eq!(
            read(&file).unwrap(),
            Header { width: 10, height: 20, orientation: 1, taken: Some("2020:01:02 03:04:05".into()) }
        );
        assert_eq!(
            read(&jpeg_file(10, 20, None, None)).unwrap(),
            Header { width: 10, height: 20, orientation: 1, taken: None }
        );
        let file = jpeg_file(
            10,
            20,
            Some(&tiff(true, None, None, None)),
            Some("<x:xmpmeta><a tiff:Orientation=\"8\"/></x:xmpmeta>"),
        );
        assert_eq!(read(&file).unwrap().orientation, 8);
        let file =
            jpeg_file(10, 20, Some(&tiff(true, Some(3), None, None)), Some("<tiff:Orientation>8</tiff:Orientation>"));
        assert_eq!(read(&file).unwrap().orientation, 3, "EXIF says it first");
        assert_eq!(xmp_orientation(b"<tiff:Orientation>8</tiff:Orientation>"), Some(8));
        // cut short, and not a picture
        assert!(read(&file[..40]).is_err() && read(b"hello, this is text").is_err() && read(b"").is_err());
    }

    #[test]
    fn the_other_kinds_of_picture_say_their_size() {
        let chunk =
            |kind: &[u8; 4], body: &[u8]| [&(body.len() as u32).to_be_bytes()[..], kind, body, &[0; 4]].concat();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &[&300u32.to_be_bytes()[..], &200u32.to_be_bytes(), &[8, 2, 0, 0, 0]].concat()));
        png.extend(chunk(b"IDAT", &[1, 2, 3]));
        png.extend(chunk(b"eXIf", &tiff(false, Some(6), None, Some("2019:12:31 23:59:59"))));
        png.extend(chunk(b"IEND", &[]));
        assert_eq!(
            read(&png).unwrap(),
            Header { width: 300, height: 200, orientation: 6, taken: Some("2019:12:31 23:59:59".into()) }
        );
        assert_eq!(
            read(b"GIF89a\x40\x01\xf0\0\0\0\0").unwrap(),
            Header { width: 320, height: 240, orientation: 1, taken: None }
        );
        let riff = |parts: &[u8]| [&b"RIFF"[..], &(parts.len() as u32 + 4).to_le_bytes(), b"WEBP", parts].concat();
        let part = |kind: &[u8; 4], body: &[u8]| {
            [&kind[..], &(body.len() as u32).to_le_bytes(), body, &[0][..body.len() % 2]].concat()
        };
        let wide = part(b"VP8X", &[8, 0, 0, 0, 0x1f, 3, 0, 0x57, 2, 0]);
        let with_exif = [wide, part(b"VP8 ", &[0; 10]), part(b"EXIF", &tiff(true, Some(3), None, None))].concat();
        assert_eq!(read(&riff(&with_exif)).unwrap(), Header { width: 800, height: 600, orientation: 3, taken: None });
        let plain = part(b"VP8 ", &[0, 0, 0, 0x9d, 1, 0x2a, 0x40, 1, 0xf0, 0]);
        assert_eq!(read(&riff(&plain)).unwrap().width, 320);
    }

    #[test]
    fn an_iphones_picture_says_its_size_as_shown_and_is_upright() {
        let bx = |kind: &[u8; 4], body: &[u8]| [&(body.len() as u32 + 8).to_be_bytes()[..], kind, body].concat();
        let full = |kind: &[u8; 4], version: u8, body: &[u8]| bx(kind, &[&[version, 0, 0, 0][..], body].concat());
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let exif_item =
            [&[0u8, 0, 0, 6][..], b"Exif\0\0", &tiff(false, Some(6), None, Some("2023:08:09 10:11:12"))].concat();
        let infe = |id: u16, kind: &[u8; 4]| full(b"infe", 2, &[&id.to_be_bytes()[..], &[0, 0], kind, &[0]].concat());
        let props = bx(
            b"ipco",
            &[
                full(b"ispe", 0, &[&4032u32.to_be_bytes()[..], &3024u32.to_be_bytes()].concat()),
                bx(b"irot", &[3]),
                bx(b"colr", b"nclx"),
            ]
            .concat(),
        );
        // item 1 (the main picture) uses properties 1 and 2
        let uses = full(b"ipma", 0, &[&1u32.to_be_bytes()[..], &[0, 1, 2, 0x81, 2]].concat());
        let build = |exif_at: u32| {
            let iloc = full(
                b"iloc",
                0,
                &[
                    &[0x44u8, 0, 0, 1][..],
                    &[0, 2, 0, 0, 0, 1],
                    &exif_at.to_be_bytes(),
                    &(exif_item.len() as u32).to_be_bytes(),
                ]
                .concat(),
            );
            let meta = full(
                b"meta",
                0,
                &[
                    full(b"hdlr", 0, b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0"),
                    full(b"pitm", 0, &[0, 1]),
                    full(b"iinf", 0, &[&[0u8, 2][..], &infe(1, b"hvc1"), &infe(2, b"Exif")].concat()),
                    iloc,
                    bx(b"iprp", &[props.clone(), uses.clone()].concat()),
                ]
                .concat(),
            );
            [ftyp.clone(), meta].concat()
        };
        let at = build(0).len() as u32 + 8;
        let file = [build(at), bx(b"mdat", &exif_item)].concat();
        let said = read(&file).unwrap();
        assert_eq!((said.width, said.height, said.orientation), (3024, 4032, 1), "turned as the file says, so upright");
        assert_eq!(said.taken.as_deref(), Some("2023:08:09 10:11:12"));
        // a video says so
        assert!(read(&bx(b"ftyp", b"isom\0\0\x02\0isomiso2mp41")).is_err());
    }
}
