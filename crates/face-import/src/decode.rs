//! A photo file as the picture faces are looked for in: decoded,
//! turned upright as its EXIF says, without its transparency, and shrunk to `MAX_SIDE`.
//!
//! Where a face's box lies in that picture makes the face's permanent ID, and the pixels make its print, so
//! every device has to come to the same picture, the one the libraries that exist were made
//! from:
//!
//! - JPEG: libjpeg-turbo, at a reduced scale where the photo is large (the
//!   smallest of 1/1, 1/2, 1/4, 1/8 that leaves both sides at least `MAX_SIDE`), its accurate method: whole
//!   numbers throughout, the same on every processor;
//! - PNG, GIF: decoded by Rust's own decoders (their pixels are the same from any decoder), then made RGB by
//!   a fixed rule for each kind: a palette looked up, grey repeated, transparency dropped, of 16 bits
//!   the upper 8;
//! - WebP: decoded here in Rust;
//! - HEIC: libheif with libde265, as a library beside the program (`heif.rs`);
//! - the shrinking is a bilinear resize in whole numbers (`resample.rs`).

use std::path::Path;

use face_core::image::Bgr;
use face_library::exif;

use crate::resample;

/// photos are shrunk to this before faces are looked for
pub const MAX_SIDE: usize = 1280;

type Result<T> = std::result::Result<T, String>;

/// A decoded picture: red, green, blue for each place, row after row.
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// What kind of picture file this is, by how it begins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Jpeg,
    Png,
    Gif,
    WebP,
    Heif,
}

pub fn kind(data: &[u8]) -> Option<Kind> {
    match data {
        [0xff, 0xd8, ..] => Some(Kind::Jpeg),
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some(Kind::Png),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some(Kind::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(Kind::WebP),
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => Some(Kind::Heif),
        _ => None,
    }
}

/// Whether a JPEG is a file of several pictures (an "MPO": a phone's photo with its depth map or a
/// second lens's picture after it). A JPEG is shrunk while it is decoded, and only a plain JPEG: a
/// file of several pictures is decoded whole and shrunk afterwards, which gives other values (up to 15 levels in
/// a real library's three such photos that are large enough to be shrunk). So the decision is fixed,
/// step for step: the last "MPF" part
/// before the picture holds a small directory, which must say in whole numbers that there are two pictures or
/// more and list each as a JPEG; and the file must not be an Ultra HDR photo, which stays a plain JPEG.
pub fn several_pictures(data: &[u8]) -> bool {
    let (mut mp, mut ultra_hdr) = (None, false);
    // the parts before the picture, in the file's order
    let mut at = 3;
    let mut s = Some(0xFFu8);
    if !data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return false;
    }
    let next = |at: &mut usize| {
        let b = data.get(*at).copied();
        *at += 1;
        b
    };
    while let Some(first) = s {
        if first != 0xFF {
            s = next(&mut at);
            continue;
        }
        let Some(second) = next(&mut at) else { break };
        match second {
            // a part without a length
            0xC8 | 0xD0..=0xD9 | 0xF0..=0xFD => s = next(&mut at),
            0xC0..=0xFE => {
                let (Some(hi), Some(lo)) = (next(&mut at), next(&mut at)) else { return false };
                let Some(n) = ((hi as usize) << 8 | lo as usize).checked_sub(2) else { return false };
                let Some(part) = data.get(at..at + n) else { return false };
                at += n;
                if second == 0xE1 && part.windows(16).any(|w| w == b" hdrgm:Version=\"") {
                    ultra_hdr = true;
                }
                if second == 0xE2 && part.starts_with(b"MPF\0") {
                    mp = Some(&part[4..]);
                }
                if second == 0xDA {
                    break; // the picture begins
                }
                s = next(&mut at);
            }
            0xFF => s = Some(0xFF), // padding
            0x00 => s = next(&mut at),
            _ => return false, // not a JPEG
        }
    }
    mp.is_some_and(|mp| !ultra_hdr && mp_pictures(mp).is_some_and(|n| n > 1))
}

/// How many pictures an "MPF" part's directory says the file holds, where the file counts as one
/// of several pictures at all: None where the directory is damaged or something else.
fn mp_pictures(mp: &[u8]) -> Option<i128> {
    let head = mp.get(..8)?;
    let known: [&[u8]; 6] = [b"MM\0\x2a", b"II\x2a\0", b"MM\x2a\0", b"II\0\x2a", b"MM\0\x2b", b"II\x2b\0"];
    if !known.iter().any(|k| head.starts_with(k)) || head[2] == 43 {
        return None; // (a directory of the large kind is not read)
    }
    let big = head.starts_with(b"MM");
    // the list of pictures is read by the first four bytes, the directory by the first two (fixed)
    let entries_big = head.starts_with(b"MM\0\x2a");
    let u16_at = |b: &[u8], at: usize, big: bool| -> Option<u16> {
        let v: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if big { u16::from_be_bytes(v) } else { u16::from_le_bytes(v) })
    };
    let u32_at = |b: &[u8], at: usize, big: bool| -> Option<u32> {
        let v: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if big { u32::from_be_bytes(v) } else { u32::from_le_bytes(v) })
    };
    let mut at = u32_at(head, 4, big)? as usize;
    // each entry's kind and bytes, the first of a tag kept; a directory cut short keeps what came before
    let mut tags: Vec<(u16, u16, &[u8])> = Vec::new();
    let mut total = 0usize;
    if let Some(count) = u16_at(mp, at, big) {
        at += 2;
        for _ in 0..count {
            let Some(entry) = mp.get(at..at + 12) else { break };
            at += 12;
            let (tag, kind, n) = (u16_at(entry, 0, big)?, u16_at(entry, 2, big)?, u32_at(entry, 4, big)? as usize);
            if tags.iter().any(|t| t.0 == tag) {
                continue;
            }
            let unit = match kind {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 | 16 => 8,
                _ => continue,
            };
            let size = n.saturating_mul(unit);
            let value = if size > 4 {
                total = total.saturating_add(size);
                if total > mp.len() {
                    continue;
                }
                let from = u32_at(entry, 8, big)? as usize;
                let Some(value) = from.checked_add(size).and_then(|end| mp.get(from..end)) else { break };
                value
            } else {
                &entry[8..8 + size]
            };
            if !value.is_empty() {
                tags.push((tag, kind, value));
            }
        }
    }
    let of = |tag: u16| tags.iter().find(|t| t.0 == tag).map(|t| (t.1, t.2));
    // the number of pictures: a whole number of any width, its first value
    let (kind, v) = of(0xB001)?;
    let pictures: i128 = match kind {
        3 => u16_at(v, 0, big)? as i128,
        4 | 13 => u32_at(v, 0, big)? as i128,
        6 => v[0] as i8 as i128,
        8 => u16_at(v, 0, big)? as i16 as i128,
        9 => u32_at(v, 0, big)? as i32 as i128,
        16 => {
            let b: [u8; 8] = v.get(..8)?.try_into().ok()?;
            (if big { u64::from_be_bytes(b) } else { u64::from_le_bytes(b) }) as i128
        }
        _ => return None,
    };
    // the list of them: plain bytes, 16 for each, and each a JPEG
    let (kind, list) = of(0xB002)?;
    if !matches!(kind, 1 | 7) {
        return None;
    }
    for k in 0..pictures.max(0) as usize {
        let attribute = u32_at(list, k.checked_mul(16)?, entries_big).filter(|_| list.len() >= k * 16 + 16)?;
        if (attribute >> 24) & 7 != 0 {
            return None;
        }
    }
    Some(pictures)
}

thread_local! {
    /// what libjpeg warned about in the JPEG this thread read last, if anything
    static WARNED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// What libjpeg warned about in the last JPEG this thread loaded (it was read all the same): a file that
/// is damaged somewhere can come out differently from one reader to the next.
pub fn jpeg_warning() -> Option<String> {
    WARNED.with(|w| w.borrow().clone())
}

/// A JPEG as it is read. A file that only lacks its last two bytes (the mark that says "the end") is a
/// whole picture, and is read; one that stops inside the picture is refused as cut short. libjpeg
/// says "premature end" to both, so such a file is read once more with the mark put after it: read then
/// without a word, it was whole.
fn jpeg(data: &[u8]) -> Result<Rgb> {
    WARNED.with(|w| *w.borrow_mut() = None);
    match jpeg_once(data) {
        Err(why) if why.to_lowercase().contains("premature end") => {
            let ended = [data, &[0xff, 0xd9]].concat();
            jpeg_once(&ended).map_err(|_| why)
        }
        done => done,
    }
}

/// A JPEG at the reduced scale that suits a picture of at most `MAX_SIDE`: by libjpeg-turbo, asked
/// through its own interface (TurboJPEG) so that a file it only warns about is still read.
fn jpeg_once(data: &[u8]) -> Result<Rgb> {
    use std::ffi::{CStr, c_int};

    use turbojpeg::raw as tj;
    struct Handle(tj::tjhandle);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: made by tj3Init, destroyed once
            unsafe { tj::tj3Destroy(self.0) }
        }
    }
    // SAFETY: the calls are TurboJPEG's own with a handle it made, the file's bytes, and a buffer of the size
    // it is told (rows of `pitch` bytes, `h` of them)
    unsafe {
        let handle = Handle(tj::tj3Init(tj::TJINIT_TJINIT_DECOMPRESS as c_int));
        if handle.0.is_null() {
            return Err("libjpeg-turbo couldn't begin".to_string());
        }
        // A warning leaves the picture readable (a marker out of place, bytes it doesn't know), and such a
        // file is read; a file that ends early is not one of those: it is refused as cut short
        let done = |result: c_int| -> Result<()> {
            if result == 0 {
                return Ok(());
            }
            let why = CStr::from_ptr(tj::tj3GetErrorStr(handle.0)).to_string_lossy().into_owned();
            let warning = tj::tj3GetErrorCode(handle.0) == tj::TJERR_TJERR_WARNING as c_int;
            if warning && !why.to_lowercase().contains("premature end") {
                WARNED.with(|w| *w.borrow_mut() = Some(why));
                Ok(())
            } else {
                Err(why)
            }
        };
        done(tj::tj3DecompressHeader(handle.0, data.as_ptr(), data.len() as _))?;
        let get = |what: tj::TJPARAM| tj::tj3Get(handle.0, what as c_int);
        let (width, height) = (get(tj::TJPARAM_TJPARAM_JPEGWIDTH), get(tj::TJPARAM_TJPARAM_JPEGHEIGHT));
        if width <= 0 || height <= 0 {
            return Err("a JPEG without a size".to_string());
        }
        let (width, height) = (width as usize, height as usize);
        // the largest of 8, 4, 2 that leaves both sides at least MAX_SIDE; a file of several pictures is
        // decoded whole (see `several_pictures`)
        let by = match (width / MAX_SIDE).min(height / MAX_SIDE) {
            _ if several_pictures(data) => 1,
            8.. => 8,
            4.. => 4,
            2.. => 2,
            _ => 1,
        };
        done(tj::tj3SetScalingFactor(handle.0, tj::tjscalingfactor { num: 1, denom: by as c_int }))?;
        let (w, h) = (width.div_ceil(by), height.div_ceil(by));
        let space = get(tj::TJPARAM_TJPARAM_COLORSPACE);
        if space == tj::TJCS_TJCS_CMYK as c_int || space == tj::TJCS_TJCS_YCCK as c_int {
            // four inks, stored the other way round (Adobe's way, which every such JPEG is taken for)
            let mut ink = vec![0u8; w * h * 4];
            done(tj::tj3Decompress8(
                handle.0,
                data.as_ptr(),
                data.len() as _,
                ink.as_mut_ptr(),
                (w * 4) as c_int,
                tj::TJPF_TJPF_CMYK,
            ))?;
            let times = |a: u8, b: i32| {
                let t = a as i32 * b + 128;
                ((t >> 8) + t) >> 8
            };
            let px = (ink.chunks_exact(4))
                .flat_map(|p| [0, 1, 2].map(|c| (p[3] as i32 - times(255 - p[c], p[3] as i32)).clamp(0, 255) as u8))
                .collect();
            return Ok(Rgb { w, h, px });
        }
        let mut px = vec![0u8; w * h * 3];
        done(tj::tj3Decompress8(
            handle.0,
            data.as_ptr(),
            data.len() as _,
            px.as_mut_ptr(),
            (w * 3) as c_int,
            tj::TJPF_TJPF_RGB,
        ))?;
        Ok(Rgb { w, h, px })
    }
}

/// A PNG as red, green and blue, by the fixed rule for its kind.
fn png(data: &[u8]) -> Result<Rgb> {
    use png::{BitDepth, ColorType, Transformations};
    let said = |e: png::DecodingError| e.to_string();
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(Transformations::IDENTITY);
    let mut reader = dec.read_info().map_err(said)?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("a PNG too large to read")?];
    let frame = reader.next_frame(&mut buf).map_err(said)?;
    let (w, h, line) = (frame.width as usize, frame.height as usize, frame.line_size);
    let palette: Vec<u8> = reader.info().palette.as_deref().map(<[u8]>::to_vec).unwrap_or_default();
    let (kind, depth) = (frame.color_type, frame.bit_depth);
    let bits = depth as usize;
    let mut px = Vec::with_capacity(w * h * 3);
    for row in buf.chunks(line).take(h) {
        // a value of fewer than 8 bits out of the row
        let small = |x: usize| (row[x * bits / 8] >> (8 - bits - x * bits % 8)) & ((1 << bits) - 1);
        for x in 0..w {
            let rgb = match (kind, depth) {
                (ColorType::Rgb, BitDepth::Eight) => [row[x * 3], row[x * 3 + 1], row[x * 3 + 2]],
                (ColorType::Rgba, BitDepth::Eight) => [row[x * 4], row[x * 4 + 1], row[x * 4 + 2]],
                // of 16 bits the upper 8
                (ColorType::Rgb, BitDepth::Sixteen) => [row[x * 6], row[x * 6 + 2], row[x * 6 + 4]],
                (ColorType::Rgba, BitDepth::Sixteen) => [row[x * 8], row[x * 8 + 2], row[x * 8 + 4]],
                (ColorType::Grayscale, BitDepth::Eight) => [row[x]; 3],
                (ColorType::GrayscaleAlpha, BitDepth::Eight) => [row[x * 2]; 3],
                (ColorType::GrayscaleAlpha, BitDepth::Sixteen) => [row[x * 4]; 3],
                // grey of 16 bits: the whole number is kept and cut off at white
                (ColorType::Grayscale, BitDepth::Sixteen) => [if row[x * 2] > 0 { 255 } else { row[x * 2 + 1] }; 3],
                (ColorType::Grayscale, BitDepth::One) => [small(x) * 255; 3],
                (ColorType::Grayscale, BitDepth::Two) => [small(x) * 85; 3],
                (ColorType::Grayscale, BitDepth::Four) => [small(x) * 17; 3],
                (ColorType::Indexed, _) => {
                    let i = if bits == 8 { row[x] } else { small(x) } as usize * 3;
                    [0, 1, 2].map(|c| palette.get(i + c).copied().unwrap_or(0))
                }
                _ => return Err("a PNG of a kind that isn't read".to_string()),
            };
            px.extend_from_slice(&rgb);
        }
    }
    Ok(Rgb { w, h, px })
}

/// A GIF's first picture as red, green and blue: its colours looked up (the picture's own,
/// else the file's), and what the picture doesn't cover of the whole in its see-through colour, else the first.
fn gif(data: &[u8]) -> Result<Rgb> {
    let said = |e: gif::DecodingError| e.to_string();
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = options.read_info(std::io::Cursor::new(data)).map_err(said)?;
    let (w, h) = (dec.width() as usize, dec.height() as usize);
    let shared: Option<Vec<u8>> = dec.global_palette().map(<[u8]>::to_vec);
    let frame = dec.read_next_frame().map_err(said)?.ok_or("a GIF without a picture")?;
    // a list of colours that is just the greys in order doesn't count as one: the numbers are the greys
    let greys = |p: &Vec<u8>| p.len() == 768 && p.iter().enumerate().all(|(i, v)| *v as usize == i / 3);
    let palette = frame.palette.clone().or(shared.filter(|p| !greys(p)));
    let mut canvas = vec![frame.transparent.unwrap_or(0); w * h];
    let (left, top, fw, fh) = (frame.left as usize, frame.top as usize, frame.width as usize, frame.height as usize);
    for y in 0..fh.min(h.saturating_sub(top)) {
        let n = fw.min(w.saturating_sub(left));
        canvas[(top + y) * w + left..(top + y) * w + left + n].copy_from_slice(&frame.buffer[y * fw..y * fw + n]);
    }
    let px = (canvas.iter())
        .flat_map(|i| match &palette {
            Some(p) => [0, 1, 2].map(|c| p.get(*i as usize * 3 + c).copied().unwrap_or(0)),
            None => [*i; 3],
        })
        .collect();
    Ok(Rgb { w, h, px })
}

fn webp(data: &[u8]) -> Result<Rgb> {
    let said = |e: image_webp::DecodingError| e.to_string();
    let mut dec = image_webp::WebPDecoder::new(std::io::Cursor::new(data)).map_err(said)?;
    let (w, h) = dec.dimensions();
    let mut buf = vec![0u8; dec.output_buffer_size().ok_or("a WebP too large to read")?];
    dec.read_image(&mut buf).map_err(said)?;
    let (w, h) = (w as usize, h as usize);
    let px = if dec.has_alpha() { buf.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect() } else { buf };
    Ok(Rgb { w, h, px })
}

/// The picture turned as EXIF's orientation says.
fn upright(p: Rgb, orientation: u32) -> Rgb {
    let (w, h) = (p.w, p.h);
    if !(2..=8).contains(&orientation) {
        return p;
    }
    let turned = orientation >= 5;
    let (ow, oh) = if turned { (h, w) } else { (w, h) };
    let mut px = vec![0u8; ow * oh * 3];
    for y in 0..oh {
        for x in 0..ow {
            // where this place of the upright picture is in the stored one
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x),
            };
            px[(y * ow + x) * 3..(y * ow + x) * 3 + 3].copy_from_slice(&p.px[(sy * w + sx) * 3..(sy * w + sx) * 3 + 3]);
        }
    }
    Rgb { w: ow, h: oh, px }
}

/// To the nearest whole number, a half to the even one.
fn rounded(v: f64) -> usize {
    v.round_ties_even() as usize
}

/// A photo's bytes as the picture faces are looked for in: upright, at most `MAX_SIDE` on its longer side,
/// blue first as the models take it.
pub fn load(data: &[u8]) -> Result<Bgr> {
    let kind = kind(data).ok_or("not a picture this can read")?;
    let orientation = match kind {
        // already upright as libheif hands it over
        Kind::Heif | Kind::Gif => 1,
        _ => exif::header(std::io::Cursor::new(data)).map_or(1, |h| h.orientation),
    };
    let picture = match kind {
        Kind::Jpeg => jpeg(data)?,
        Kind::Png => png(data)?,
        Kind::Gif => gif(data)?,
        Kind::WebP => webp(data)?,
        Kind::Heif => crate::heif::decode(data)?,
    };
    let p = upright(picture, orientation);
    let scale = MAX_SIDE as f64 / p.w.max(p.h).max(1) as f64;
    let (w, h, mut px) = if scale < 1.0 {
        let (w, h) = (rounded(p.w as f64 * scale), rounded(p.h as f64 * scale));
        (w, h, resample::resize(&p.px, p.w, p.h, 3, w, h))
    } else {
        (p.w, p.h, p.px)
    };
    px.chunks_exact_mut(3).for_each(|c| c.swap(0, 2));
    Bgr::from_px(w, h, px).map_err(|e| e.0)
}

/// The photo file at `path`, loaded as `load` loads its bytes.
pub fn load_file(path: &Path) -> Result<Bgr> {
    load(&std::fs::read(path).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod several {
    use super::several_pictures;

    /// A JPEG's beginning with these parts before its picture.
    fn file(parts: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        for (marker, body) in parts {
            out.extend([0xFF, *marker]);
            out.extend(((body.len() + 2) as u16).to_be_bytes());
            out.extend(body);
        }
        out.extend([0xFF, 0xDA, 0, 2, 1, 2, 3]);
        out
    }

    /// An "MPF" part that says `pictures` pictures and lists `listed` of them, the first of kind `first`.
    fn mpf(big: bool, pictures: u32, listed: usize, first: u32) -> Vec<u8> {
        let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut d: Vec<u8> = b"MPF\0".to_vec();
        d.extend(if big { *b"MM\0\x2a" } else { *b"II\x2a\0" });
        d.extend(u32b(8));
        d.extend(u16b(3));
        for (tag, kind, n, value) in [
            (0xB000u16, 7u16, 4u32, *b"0100"),
            (0xB001, 4, 1, u32b(pictures)),
            (0xB002, 7, 16 * listed as u32, u32b(50)),
        ] {
            d.extend(u16b(tag));
            d.extend(u16b(kind));
            d.extend(u32b(n));
            d.extend(value);
        }
        d.extend(u32b(0));
        for k in 0..listed {
            d.extend(u32b(if k == 0 { first } else { 0 }));
            d.extend([0u8; 12]);
        }
        d
    }

    #[test]
    fn a_file_of_several_pictures_is_told_by_its_fixed_rule() {
        let exif = (0xE1, b"Exif\0\0II".to_vec());
        for big in [false, true] {
            assert!(several_pictures(&file(&[exif.clone(), (0xE2, mpf(big, 2, 2, 0x2003_0000))])));
            assert!(several_pictures(&file(&[(0xE2, mpf(big, 3, 3, 0))])));
            // one picture; fewer listed than said; a picture that is no JPEG
            assert!(!several_pictures(&file(&[(0xE2, mpf(big, 1, 1, 0))])));
            assert!(!several_pictures(&file(&[(0xE2, mpf(big, 2, 1, 0))])));
            assert!(!several_pictures(&file(&[(0xE2, mpf(big, 2, 2, 0x0100_0000))])));
        }
        // no such part, a part of another kind, an Ultra HDR photo, and what is no JPEG
        assert!(!several_pictures(&file(&[exif.clone()])));
        assert!(!several_pictures(&file(&[(0xE2, b"ICC_PROFILE\0\x01\x01".to_vec())])));
        let hdr = (0xE1, b"http://ns.adobe.com/xap/1.0/\0<x hdrgm:Version=\"1.0\"/>".to_vec());
        assert!(!several_pictures(&file(&[hdr, (0xE2, mpf(false, 2, 2, 0))])));
        assert!(!several_pictures(b"\x89PNG") && !several_pictures(&[0xFF, 0xD8, 0xFF]));
        // the part after the picture has begun doesn't count
        let mut late = file(&[]);
        late.extend(file(&[(0xE2, mpf(false, 2, 2, 0))]));
        assert!(!several_pictures(&late));
        // a directory cut short keeps what came before it: here the number alone, so no list
        let mut cut = mpf(false, 2, 2, 0);
        cut.truncate(4 + 8 + 2 + 24);
        assert!(!several_pictures(&file(&[(0xE2, cut)])));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: usize, h: usize) -> Rgb {
        Rgb { w, h, px: (0..w * h).flat_map(|i| [i as u8, 0, 0]).collect() }
    }

    fn reds(p: &Rgb) -> Vec<u8> {
        p.px.chunks_exact(3).map(|c| c[0]).collect()
    }

    #[test]
    fn a_picture_is_turned_upright_as_its_exif_says() {
        // 3 wide, 2 high:  0 1 2 / 3 4 5
        let turn = |o: u32| {
            let p = upright(picture(3, 2), o);
            (p.w, p.h, reds(&p))
        };
        assert_eq!(turn(1), (3, 2, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(turn(2), (3, 2, vec![2, 1, 0, 5, 4, 3]), "mirrored");
        assert_eq!(turn(3), (3, 2, vec![5, 4, 3, 2, 1, 0]), "upside down");
        assert_eq!(turn(4), (3, 2, vec![3, 4, 5, 0, 1, 2]), "top to bottom");
        assert_eq!(turn(5), (2, 3, vec![0, 3, 1, 4, 2, 5]), "across the diagonal");
        assert_eq!(turn(6), (2, 3, vec![3, 0, 4, 1, 5, 2]), "a quarter turn clockwise");
        assert_eq!(turn(7), (2, 3, vec![5, 2, 4, 1, 3, 0]), "across the other diagonal");
        assert_eq!(turn(8), (2, 3, vec![2, 5, 1, 4, 0, 3]), "a quarter turn the other way");
        assert_eq!(turn(0), turn(1));
        assert_eq!((rounded(0.5), rounded(1.5), rounded(2.5), rounded(959.6)), (0, 2, 2, 960));
    }

    #[test]
    fn the_kind_of_file_is_told_by_how_it_begins() {
        assert_eq!(kind(&[0xff, 0xd8, 0xff, 0xe0]), Some(Kind::Jpeg));
        assert_eq!(kind(b"\x89PNG\r\n\x1a\n...."), Some(Kind::Png));
        assert_eq!(kind(b"GIF89a...."), Some(Kind::Gif));
        assert_eq!(kind(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::WebP));
        assert_eq!(kind(b"\0\0\0\x18ftypheic"), Some(Kind::Heif));
        assert_eq!(kind(b"hello"), None);
        assert!(load(b"hello").is_err());
    }
}
