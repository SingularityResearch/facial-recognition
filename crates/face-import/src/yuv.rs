//! A video's decoded picture made into the colours faces are looked for in, here and not by FFmpeg: FFmpeg's
//! own conversion gives other values on one kind of processor than on another, and from a build without
//! processor-specific code than from one with it (measured: every frame of seven test clips differed between
//! two builds whose decoded pictures were the same bytes). A face's permanent ID is made from where its box
//! lies in the frame, so every device has to make the same frame: the decoded picture is taken from FFmpeg
//! as it is, and converted by the whole-number arithmetic below, which is the same everywhere.
//!
//! The arithmetic is the one the libraries of every device so far were made with: what FFmpeg's conversion
//! does on an x86 processor for the usual kind of video (8 bits, colour at half the size both ways), which
//! is what every library's frames were made with (its `yuv_2_rgb.asm` and the numbers of
//! `ff_yuv2rgb_c_init_tables`, for colours as standard definition has them, as OpenCV takes every video).
//! Each value times 8, the offsets taken away, multiplied keeping the upper 16 bits of the product, added
//! up and cut to 0..255; a picture's colour values used for the places they cover, not smoothed between.
//! The other kinds (10 bits, colour at full size or half the size one way) go through the same arithmetic,
//! with a 10-bit value times 2 where an 8-bit one is times 8.

use face_core::image::Bgr;

/// How a decoded picture's colour is kept: at half the size both ways, half the size across, or full size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Colour {
    Half,
    HalfAcross,
    Full,
}

/// A kind of decoded picture this converts itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kind {
    pub colour: Colour,
    /// 8 or 10 bits a value (10: two bytes, the low one first)
    pub bits: u32,
    /// light and dark use the whole range of values (the old "J" kinds of picture)
    pub full_range: bool,
}

impl Kind {
    /// The kind FFmpeg names so (its "pix_fmt"), or None for one that is left to FFmpeg to convert.
    pub fn of(name: &str) -> Option<Kind> {
        let (colour, bits, full_range) = match name {
            "yuv420p" => (Colour::Half, 8, false),
            "yuvj420p" => (Colour::Half, 8, true),
            "yuv422p" => (Colour::HalfAcross, 8, false),
            "yuvj422p" => (Colour::HalfAcross, 8, true),
            "yuv444p" => (Colour::Full, 8, false),
            "yuvj444p" => (Colour::Full, 8, true),
            "yuv420p10le" => (Colour::Half, 10, false),
            "yuv422p10le" => (Colour::HalfAcross, 10, false),
            "yuv444p10le" => (Colour::Full, 10, false),
            _ => return None,
        };
        Some(Kind { colour, bits, full_range })
    }

    /// (width, height) of each of the two colour planes for a picture of this size.
    fn colour_size(&self, w: usize, h: usize) -> (usize, usize) {
        match self.colour {
            Colour::Half => (w.div_ceil(2), h.div_ceil(2)),
            Colour::HalfAcross => (w.div_ceil(2), h),
            Colour::Full => (w, h),
        }
    }

    /// How many bytes one decoded picture of this size is.
    pub fn frame_bytes(&self, w: usize, h: usize) -> usize {
        let (cw, ch) = self.colour_size(w, h);
        (w * h + 2 * cw * ch) * if self.bits > 8 { 2 } else { 1 }
    }
}

/// The conversion's numbers, as FFmpeg works them out for standard definition's colours.
struct Numbers {
    y: i32,
    y_off: i16,
    vr: i32,
    ub: i32,
    ug: i32,
    vg: i32,
}

impl Numbers {
    fn new(full_range: bool) -> Numbers {
        // (a value in 16.16 fixed point -> the 16 bits FFmpeg keeps of it, rounded)
        let round16 = |f: i64| -> i32 { (((f + (1 << 15)) >> 16) as i32).clamp(-0x7FFF, 0x7FFF) };
        let (mut crv, mut cbu, mut cgu, mut cgv, mut cy, mut oy) =
            (104_597i64, 132_201i64, -25_675i64, -53_279i64, 1i64 << 16, 0i64);
        if full_range {
            (crv, cbu, cgu, cgv) = (crv * 224 / 255, cbu * 224 / 255, cgu * 224 / 255, cgv * 224 / 255);
        } else {
            cy = cy * 255 / 219;
            oy = 16 << 16;
        }
        Numbers {
            y: round16(cy << 13),
            y_off: round16(oy << 3) as i16,
            vr: round16(crv << 13),
            ub: round16(cbu << 13),
            ug: round16(cgu << 13),
            vg: round16(cgv << 13),
        }
    }
}

/// One decoded picture (`data`: the brightness plane, then the two colour planes, as FFmpeg writes them raw)
/// as blue, green, red. None when `data` isn't a picture of this size and kind.
pub fn to_bgr(kind: Kind, w: usize, h: usize, data: &[u8]) -> Option<Bgr> {
    if w == 0 || h == 0 || data.len() != kind.frame_bytes(w, h) {
        return None;
    }
    let n = Numbers::new(kind.full_range);
    let (cw, ch) = kind.colour_size(w, h);
    let wide = kind.bits > 8;
    // a value as the arithmetic takes it: an 8-bit one times 8, a 10-bit one times 2
    let at = |plane: usize, i: usize| -> i16 {
        if wide {
            (u16::from_le_bytes([data[plane + 2 * i], data[plane + 2 * i + 1]]) << 1) as i16
        } else {
            (data[plane + i] as i16) << 3
        }
    };
    let step = if wide { 2 } else { 1 };
    let (u_plane, v_plane) = (w * h * step, (w * h + cw * ch) * step);
    let high = |a: i16, b: i32| -> i16 { ((a as i32 * b) >> 16) as i16 };
    let cut = |v: i16| v.clamp(0, 255) as u8;
    let mut px = vec![0u8; w * h * 3];
    for row in 0..h {
        let c_row = if kind.colour == Colour::Half { row / 2 } else { row };
        for col in 0..w {
            let c = c_row * cw + if kind.colour == Colour::Full { col } else { col / 2 };
            let y = high(at(0, row * w + col).wrapping_sub(n.y_off), n.y);
            let (u, v) = (at(u_plane, c).saturating_sub(1024), at(v_plane, c).saturating_sub(1024));
            let green = high(u, n.ug).saturating_add(high(v, n.vg));
            let out = &mut px[(row * w + col) * 3..][..3];
            out[0] = cut(high(u, n.ub).saturating_add(y));
            out[1] = cut(green.saturating_add(y));
            out[2] = cut(high(v, n.vr).saturating_add(y));
        }
    }
    Some(Bgr { w, h, px })
}

#[cfg(test)]
mod tests {
    use face_library::photofiles::sha1_hex;

    use super::*;

    /// Made-up values, the same every time.
    fn made(n: usize, seed: u64) -> Vec<u8> {
        (0..n as u64).map(|i| ((i * 2_654_435_761 + seed * 40_503) >> 7) as u8).collect()
    }

    #[test]
    fn made_up_pictures_come_out_as_from_ffmpeg_on_x86() {
        // each answer is the SHA-1 of what FFmpeg 7.1.5 (an x86 build with its processor-specific code) wrote
        // for the same 16 x 8 picture, converted to bgr24 with standard definition's colours
        assert_eq!(&made(6, 7), &[167, 154, 141, 129, 116, 103]);
        for (name, n, sum, first) in [
            ("yuv420p", 192, "f0c0287e2bbda8351de9fa3a1f98496b6c5245ef", [94u8, 134, 255, 79, 119, 255]),
            ("yuvj420p", 192, "792cc260cf45ca175f1ea0f57c80886b14e0c25d", [96, 131, 255, 83, 118, 249]),
            ("yuv422p", 256, "1968dbca6a12bb8e84ce8233d0f7a46065a3cfa3", [94, 255, 47, 79, 240, 32]),
            ("yuvj422p", 256, "3f4946db1985f9cbe32d4e622d15fa634adf9863", [96, 237, 54, 83, 224, 41]),
        ] {
            let got = to_bgr(Kind::of(name).unwrap(), 16, 8, &made(n, 7)).unwrap();
            assert_eq!((&got.px[..6], sha1_hex(&got.px).as_str()), (&first[..], sum), "{name}");
        }
    }

    #[test]
    fn the_numbers_are_ffmpegs() {
        // as its x86 code holds them for a video in the usual range and one in the whole range
        let n = Numbers::new(false);
        assert_eq!((n.y, n.y_off, n.vr, n.ub, n.ug, n.vg), (9539, 128, 13075, 16525, -3209, -6660));
        let n = Numbers::new(true);
        assert_eq!((n.y, n.y_off), (8192, 0));
    }

    #[test]
    fn a_picture_is_converted_as_ffmpeg_converts_it_on_x86() {
        // 4 x 2 values of brightness with one colour value for each 2 x 2: black, white and colours, in the
        // usual range; the answers are FFmpeg 7.1's own (the x86 build, yuv420p to bgr24)
        let data = [16u8, 235, 81, 145, 41, 210, 128, 128, /* u */ 128, 90, /* v */ 128, 240];
        let kind = Kind::of("yuv420p").unwrap();
        let got = to_bgr(kind, 4, 2, &data).unwrap();
        assert_eq!(&got.px[..6], &[0, 0, 0, 255, 255, 255]);
        assert_eq!(kind.frame_bytes(4, 2), 12);
        assert!(to_bgr(kind, 4, 2, &data[..11]).is_none() && Kind::of("pal8").is_none());
        // the same values as a 10-bit picture (each times 4) give the same colours
        let wide: Vec<u8> = data.iter().flat_map(|v| ((*v as u16) << 2).to_le_bytes()).collect();
        assert_eq!(to_bgr(Kind::of("yuv420p10le").unwrap(), 4, 2, &wide).unwrap().px, got.px);
        // colour at full size: each place its own
        let full = [16u8, 235, 128, 128, 128, 128];
        assert_eq!(to_bgr(Kind::of("yuv444p").unwrap(), 2, 1, &full).unwrap().px, [0, 0, 0, 255, 255, 255]);
        // the whole range: 0 is black and 255 white
        assert_eq!(
            to_bgr(Kind::of("yuvj444p").unwrap(), 2, 1, &[0, 255, 128, 128, 128, 128]).unwrap().px,
            [0, 0, 0, 255, 255, 255]
        );
    }
}
