//! The library's array archives (`people.npz`, `faces.npz`, `thumbs.npz`): the files every released version
//! wrote, which a device must still open after it is updated, and which any other reader of the format
//! must open too. An .npz is a zip of .npy arrays; what the library holds are float32 tables, text,
//! whole numbers and bytes.

// What another device or the network sends is read here, and a panic ends the whole process (`panic = "abort"`):
// clippy shows each unwrap, index and plain sum of this module, to be looked at (not in its tests).
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects))]

use std::collections::HashMap;

use crate::{Error, Result};

/// One array of an archive.
#[derive(Debug, Clone)]
pub struct Array {
    pub shape: Vec<usize>,
    pub data: Data,
}

#[derive(Debug, Clone)]
pub enum Data {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I64(Vec<i64>),
    Text(Vec<String>),
    Bytes(Vec<u8>),
}

impl Array {
    pub fn f32(&self) -> Result<&[f32]> {
        match &self.data {
            Data::F32(v) => Ok(v),
            _ => Err(Error::msg("expected float32 values")),
        }
    }
    pub fn text(&self) -> Result<&[String]> {
        match &self.data {
            Data::Text(v) => Ok(v),
            _ => Err(Error::msg("expected text")),
        }
    }
    pub fn wholes_of(&self) -> Result<&[i64]> {
        match &self.data {
            Data::I64(v) => Ok(v),
            _ => Err(Error::msg("expected whole numbers")),
        }
    }
    pub fn bytes_of(&self) -> Result<&[u8]> {
        match &self.data {
            Data::Bytes(v) => Ok(v),
            _ => Err(Error::msg("expected bytes")),
        }
    }
}

fn u16le(b: &[u8], at: usize) -> Result<usize> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize).ok_or_else(|| Error::msg("cut-off archive"))
}

fn u32le(b: &[u8], at: usize) -> Result<usize> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
        .ok_or_else(|| Error::msg("cut-off archive"))
}

fn u64le(b: &[u8], at: usize) -> Result<usize> {
    b.get(at..at + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()) as usize)
        .ok_or_else(|| Error::msg("cut-off archive"))
}

/// arrays in an archive that came from another device, at most: a people file has three
const FOREIGN_ARRAYS: usize = 16;
/// arrays in any archive, at most
const MOST_ARRAYS: usize = 4096;

/// Every array of an .npz, by name (without ".npy").
pub fn read_npz(bytes: &[u8]) -> Result<HashMap<String, Array>> {
    read_archive(bytes, false)
}

/// The same for an archive that came from another device: nothing in it is taken on trust. Only arrays stored
/// as they are (what `write_npz` writes: no unpacking, so everything read lies inside the bytes that
/// arrived), and only a handful of them.
pub fn read_npz_foreign(bytes: &[u8]) -> Result<HashMap<String, Array>> {
    read_archive(bytes, true)
}

fn read_archive(bytes: &[u8], foreign: bool) -> Result<HashMap<String, Array>> {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const EOCD64_LOC: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
    let bad = || Error::msg("not an .npz archive");
    if bytes.len() < 22 {
        return Err(bad());
    }
    let from = bytes.len().saturating_sub(22 + 65_535);
    let end = (from..=bytes.len() - 22).rev().find(|&i| bytes[i..i + 4] == EOCD).ok_or_else(bad)?;
    let (mut count, mut dir) = (u16le(bytes, end + 10)?, u32le(bytes, end + 16)?);
    if end >= 20 && bytes[end - 20..end - 16] == EOCD64_LOC {
        // a large archive keeps both as 64-bit
        let at = u64le(bytes, end - 12)?;
        count = u64le(bytes, at + 32)?;
        dir = u64le(bytes, at + 48)?;
    }
    if count > if foreign { FOREIGN_ARRAYS } else { MOST_ARRAYS } {
        return Err(Error::msg("an archive of more arrays than this reads"));
    }
    let mut out = HashMap::new();
    let mut at = dir;
    for _ in 0..count {
        if bytes.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(bad());
        }
        let method = u16le(bytes, at + 10)?;
        let (mut packed, mut size) = (u32le(bytes, at + 20)?, u32le(bytes, at + 24)?);
        let (name_len, extra_len, comment_len) =
            (u16le(bytes, at + 28)?, u16le(bytes, at + 30)?, u16le(bytes, at + 32)?);
        let mut local = u32le(bytes, at + 42)?;
        // (every place and length is the archive's own word: added so that none can wrap round)
        let name_at = at.saturating_add(46);
        let extra_at = name_at.saturating_add(name_len);
        let name = String::from_utf8_lossy(bytes.get(name_at..extra_at).ok_or_else(bad)?).into_owned();
        let extra = bytes.get(extra_at..extra_at.saturating_add(extra_len)).ok_or_else(bad)?;
        let mut e = 0;
        while e + 4 <= extra.len() {
            // sizes past 4 GB are in a 64-bit extra field
            let (id, len) = (u16le(extra, e)?, u16le(extra, e + 2)?);
            if id == 1 {
                let mut p = e + 4;
                if size == 0xffff_ffff {
                    size = u64le(extra, p)?;
                    p += 8;
                }
                if packed == 0xffff_ffff {
                    packed = u64le(extra, p)?;
                    p += 8;
                }
                if local == 0xffff_ffff {
                    local = u64le(extra, p)?;
                }
            }
            e += 4 + len;
        }
        at = extra_at.saturating_add(extra_len).saturating_add(comment_len);
        if bytes.get(local..local.saturating_add(4)) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
            return Err(bad());
        }
        let start = local
            .saturating_add(30)
            .saturating_add(u16le(bytes, local.saturating_add(26))?)
            .saturating_add(u16le(bytes, local.saturating_add(28))?);
        let raw = bytes.get(start..start.saturating_add(packed)).ok_or_else(bad)?;
        let data = match method {
            0 => raw.to_vec(),
            // (unpacked, an array is as large as the archive says it is: another device's word is not taken
            // for that, and no archive is unpacked to more than a library's prints can be)
            8 if !foreign && size <= MOST_UNPACKED => {
                miniz_oxide::inflate::decompress_to_vec_with_limit(raw, size).map_err(|_| bad())?
            }
            _ => return Err(Error::msg("an archive packed in a way this can't read")),
        };
        if data.len() != size {
            return Err(bad());
        }
        out.insert(name.strip_suffix(".npy").unwrap_or(&name).to_string(), read_npy(&data)?);
    }
    Ok(out)
}

/// bytes one packed array may unpack to: more than any library's prints
const MOST_UNPACKED: usize = 2_000_000_000;

/// One .npy file: its header (a dictionary written as text) and the values after it.
pub fn read_npy(b: &[u8]) -> Result<Array> {
    let bad = || Error::msg("not an .npy array");
    if b.len() < 10 || &b[..6] != b"\x93NUMPY" {
        return Err(bad());
    }
    let (head_len, head_at) = if b[6] == 1 { (u16le(b, 8)?, 10) } else { (u32le(b, 8)?, 12) };
    let head = std::str::from_utf8(b.get(head_at..head_at + head_len).ok_or_else(bad)?).map_err(|_| bad())?;
    let body = &b[head_at + head_len..];
    let field = |key: &str| -> Option<&str> {
        let at = head.find(&format!("'{key}'"))? + key.len() + 2;
        Some(head[at..].trim_start_matches([':', ' ']))
    };
    let descr = field("descr").and_then(|s| s.strip_prefix('\'')).and_then(|s| s.split('\'').next()).ok_or_else(bad)?;
    if field("fortran_order").is_some_and(|s| s.starts_with("True")) {
        return Err(Error::msg("an array stored column by column, which this can't read"));
    }
    let shape_text =
        field("shape").and_then(|s| s.strip_prefix('(')).and_then(|s| s.split(')').next()).ok_or_else(bad)?;
    let shape: Vec<usize> = shape_text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().map_err(|_| bad()))
        .collect::<Result<_>>()?;
    // (the shape is the file's own word: its product, and the bytes it asks for, may not wrap round)
    let n: usize = shape.iter().try_fold(1usize, |all, &d| all.checked_mul(d)).ok_or_else(bad)?;
    let need = |size: usize| match n.checked_mul(size) {
        Some(bytes) if body.len() >= bytes => Ok(()),
        _ => Err(bad()),
    };
    let data = match descr {
        "<f4" => {
            need(4)?;
            Data::F32(body[..n * 4].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
        }
        "<f8" => {
            need(8)?;
            Data::F64(body[..n * 8].chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect())
        }
        "<i8" => {
            need(8)?;
            Data::I64(body[..n * 8].chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect())
        }
        "<i4" => {
            need(4)?;
            Data::I64(body[..n * 4].chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap()) as i64).collect())
        }
        "|u1" | "|b1" => {
            need(1)?;
            Data::Bytes(body[..n].to_vec())
        }
        d if d.starts_with("<U") => {
            // text: fixed width, four bytes a character
            let width: usize = d[2..].parse().map_err(|_| bad())?;
            // (a text of no width holds nothing to count its entries by: only an array of none is that)
            if width == 0 && n != 0 {
                return Err(bad());
            }
            need(width.checked_mul(4).ok_or_else(bad)?)?;
            let one = |c: &[u8]| -> String {
                c.chunks_exact(4)
                    .map(|q| u32::from_le_bytes(q.try_into().unwrap()))
                    .take_while(|&u| u != 0)
                    .filter_map(char::from_u32)
                    .collect()
            };
            Data::Text(if width == 0 {
                vec![String::new(); n]
            } else {
                body[..n * width * 4].chunks_exact(width * 4).map(one).collect()
            })
        }
        _ => return Err(Error::msg(format!("an array of a kind this can't read ({descr})"))),
    };
    Ok(Array { shape, data })
}

// ---- writing: the archives the library's side saves (`faces.npz`, `thumbs.npz`, `people.npz`) ------------------

impl Array {
    /// A table of float32 values, `rows` by `columns`.
    pub fn table(values: Vec<f32>, rows: usize, columns: usize) -> Array {
        Array { shape: vec![rows, columns], data: Data::F32(values) }
    }
    /// A list of float32 values.
    pub fn floats(values: Vec<f32>) -> Array {
        Array { shape: vec![values.len()], data: Data::F32(values) }
    }
    /// A list of 64-bit whole numbers.
    pub fn wholes(values: Vec<i64>) -> Array {
        Array { shape: vec![values.len()], data: Data::I64(values) }
    }
    /// A list of bytes.
    pub fn bytes(values: Vec<u8>) -> Array {
        Array { shape: vec![values.len()], data: Data::Bytes(values) }
    }
    /// A list of texts.
    pub fn texts(values: Vec<String>) -> Array {
        Array { shape: vec![values.len()], data: Data::Text(values) }
    }
    /// One text, as the format keeps a single value: an array without a shape.
    pub fn one_text(value: &str) -> Array {
        Array { shape: Vec::new(), data: Data::Text(vec![value.to_string()]) }
    }
}

/// One array as a .npy file: the header a dictionary written as text, padded so that the values
/// begin at a multiple of 64 bytes; text as fixed-width UTF-32, each as wide as the longest.
pub fn write_npy(a: &Array) -> Vec<u8> {
    let (descr, body): (String, Vec<u8>) = match &a.data {
        Data::F32(v) => {
            // filled in place: a library's prints are ten million numbers, and gathering them a number at a
            // time took longer than writing them
            let mut body = vec![0u8; v.len() * 4];
            for (to, x) in body.chunks_exact_mut(4).zip(v) {
                to.copy_from_slice(&x.to_le_bytes());
            }
            ("<f4".to_string(), body)
        }
        Data::F64(v) => ("<f8".to_string(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Data::I64(v) => ("<i8".to_string(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
        Data::Bytes(v) => ("|u1".to_string(), v.clone()),
        Data::Text(v) => {
            let width = v.iter().map(|s| s.chars().count()).max().unwrap_or(0).max(1);
            let mut body = Vec::with_capacity(v.len() * width * 4);
            for s in v {
                let mut n = 0;
                for c in s.chars() {
                    body.extend_from_slice(&(c as u32).to_le_bytes());
                    n += 1;
                }
                body.resize(body.len() + (width - n) * 4, 0);
            }
            (format!("<U{width}"), body)
        }
    };
    let shape = match a.shape.as_slice() {
        [] => "()".to_string(),
        [n] => format!("({n},)"),
        many => format!("({})", many.iter().map(usize::to_string).collect::<Vec<_>>().join(", ")),
    };
    let mut head = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
    // the magic text, the version and the header's length come before it: 10 bytes
    let padded = (10 + head.len() + 1).div_ceil(64) * 64;
    head.push_str(&" ".repeat(padded - 10 - head.len() - 1));
    head.push('\n');
    let mut out = Vec::with_capacity(padded + body.len());
    out.extend_from_slice(b"\x93NUMPY\x01\x00");
    out.extend_from_slice(&(head.len() as u16).to_le_bytes());
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(&body);
    out
}

/// The checksum a zip keeps of each file (CRC-32), eight bytes at a step: the library's archives are over a
/// hundred megabytes and are written whole at a save, and a byte at a step took longer than writing them.
fn crc32(data: &[u8]) -> u32 {
    static TABLES: std::sync::OnceLock<Box<[[u32; 256]; 8]>> = std::sync::OnceLock::new();
    let t = TABLES.get_or_init(|| {
        let mut t = Box::new([[0u32; 256]; 8]);
        for i in 0..256 {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            t[0][i] = c;
        }
        for i in 0..256 {
            for k in 1..8 {
                t[k][i] = (t[k - 1][i] >> 8) ^ t[0][(t[k - 1][i] & 0xff) as usize];
            }
        }
        t
    });
    let mut c = !0u32;
    let mut steps = data.chunks_exact(8);
    for b in &mut steps {
        let (lo, hi) = (u32::from_le_bytes([b[0], b[1], b[2], b[3]]) ^ c, u32::from_le_bytes([b[4], b[5], b[6], b[7]]));
        c = t[7][(lo & 0xff) as usize]
            ^ t[6][((lo >> 8) & 0xff) as usize]
            ^ t[5][((lo >> 16) & 0xff) as usize]
            ^ t[4][(lo >> 24) as usize]
            ^ t[3][(hi & 0xff) as usize]
            ^ t[2][((hi >> 8) & 0xff) as usize]
            ^ t[1][((hi >> 16) & 0xff) as usize]
            ^ t[0][(hi >> 24) as usize];
    }
    for b in steps.remainder() {
        c = t[0][((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// Arrays as an .npz: a zip of .npy files, not packed, each with the 64-bit
/// fields such an archive always has, so that one past 4 GB is the same kind of file as a small one.
pub fn write_npz(arrays: &[(&str, Array)], out: &mut impl std::io::Write) -> std::io::Result<()> {
    const FAR: u32 = 0xffff_ffff;
    const DATE: u16 = 0x0021; // 1980-01-01: when a file was written says nothing here
    let mut at = 0u64;
    let mut directory = Vec::new();
    for (name, array) in arrays {
        let (name, data) = (format!("{name}.npy"), write_npy(array));
        let (crc, size) = (crc32(&data), data.len() as u64);
        let mut head = Vec::with_capacity(50 + name.len());
        head.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        head.extend_from_slice(&45u16.to_le_bytes()); // needs a reader that knows 64-bit sizes
        head.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // no flags, not packed, no time
        head.extend_from_slice(&DATE.to_le_bytes());
        head.extend_from_slice(&crc.to_le_bytes());
        head.extend_from_slice(&FAR.to_le_bytes());
        head.extend_from_slice(&FAR.to_le_bytes());
        head.extend_from_slice(&(name.len() as u16).to_le_bytes());
        head.extend_from_slice(&20u16.to_le_bytes());
        head.extend_from_slice(name.as_bytes());
        head.extend_from_slice(&1u16.to_le_bytes());
        head.extend_from_slice(&16u16.to_le_bytes());
        head.extend_from_slice(&size.to_le_bytes());
        head.extend_from_slice(&size.to_le_bytes());
        out.write_all(&head)?;
        out.write_all(&data)?;
        // the same again for the directory at the end: sizes and place as 32-bit numbers where they fit
        let mut extra = Vec::new();
        let mut field = |v: u64| {
            if v >= FAR as u64 {
                extra.extend_from_slice(&v.to_le_bytes());
                FAR
            } else {
                v as u32
            }
        };
        let (size32, packed32) = (field(size), field(size));
        let at32 = field(at);
        directory.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
        directory.extend_from_slice(&45u16.to_le_bytes());
        directory.extend_from_slice(&45u16.to_le_bytes());
        directory.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        directory.extend_from_slice(&DATE.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&packed32.to_le_bytes());
        directory.extend_from_slice(&size32.to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&(if extra.is_empty() { 0 } else { extra.len() as u16 + 4 }).to_le_bytes());
        directory.extend_from_slice(&[0; 10]); // no comment, one disk, no attributes
        directory.extend_from_slice(&at32.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
        if !extra.is_empty() {
            directory.extend_from_slice(&1u16.to_le_bytes());
            directory.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            directory.extend_from_slice(&extra);
        }
        at += head.len() as u64 + size;
    }
    let (count, dir_size) = (arrays.len() as u64, directory.len() as u64);
    out.write_all(&directory)?;
    let mut end = Vec::new();
    if at >= FAR as u64 || dir_size >= FAR as u64 || count >= 0xffff {
        end.extend_from_slice(&[0x50, 0x4b, 0x06, 0x06]);
        end.extend_from_slice(&44u64.to_le_bytes());
        end.extend_from_slice(&45u16.to_le_bytes());
        end.extend_from_slice(&45u16.to_le_bytes());
        end.extend_from_slice(&[0; 8]);
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&dir_size.to_le_bytes());
        end.extend_from_slice(&at.to_le_bytes());
        end.extend_from_slice(&[0x50, 0x4b, 0x06, 0x07, 0, 0, 0, 0]);
        end.extend_from_slice(&(at + dir_size).to_le_bytes());
        end.extend_from_slice(&1u32.to_le_bytes());
    }
    end.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    end.extend_from_slice(&(count.min(0xffff) as u16).to_le_bytes());
    end.extend_from_slice(&(count.min(0xffff) as u16).to_le_bytes());
    end.extend_from_slice(&(dir_size.min(FAR as u64) as u32).to_le_bytes());
    end.extend_from_slice(&(at.min(FAR as u64) as u32).to_le_bytes());
    end.extend_from_slice(&[0, 0]);
    out.write_all(&end)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_checksum_is_the_one_a_zip_keeps() {
        assert_eq!(super::crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(super::crc32(b""), 0);
        // eight bytes at a step and a byte at a step give the same, whatever is left over at the end
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 131 + 7) as u8).collect();
        for n in [1, 7, 8, 9, 63, 64, 65, 1000] {
            let mut c = !0u32;
            for b in &data[..n] {
                c ^= *b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            assert_eq!(super::crc32(&data[..n]), !c, "{n} bytes");
        }
    }

    use super::*;

    /// One .npy file with this kind of value, this shape (as its header writes it) and these bytes after it.
    fn an_npy(descr: &str, shape: &str, body: &[u8]) -> Vec<u8> {
        let head = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': ({shape}), }}\n");
        let mut out = b"\x93NUMPY\x01\x00".to_vec();
        out.extend_from_slice(&(head.len() as u16).to_le_bytes());
        out.extend_from_slice(head.as_bytes());
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn a_file_made_to_break_the_reader_is_refused_and_nothing_else_happens() {
        // an honest one, to know the helper writes what the reader reads
        let two = read_npy(&an_npy("<f4", "2,", &[0, 0, 128, 63, 0, 0, 0, 64])).unwrap();
        assert!(matches!(two.data, Data::F32(v) if v == [1.0, 2.0]));
        // texts of no width, a million million of them: nothing to allocate that many of
        assert!(read_npy(&an_npy("<U0", "1099511627776,", b"")).is_err());
        assert!(matches!(read_npy(&an_npy("<U0", "0,", b"")).unwrap().data, Data::Text(t) if t.is_empty()));
        // a width whose bytes wrap round to none
        assert!(read_npy(&an_npy("<U4611686018427387904", "1,", b"")).is_err());
        // a shape whose product wraps round, and one that asks for more bytes than there are
        assert!(read_npy(&an_npy("<f4", "4611686018427387904, 4", b"")).is_err());
        assert!(read_npy(&an_npy("<f4", "4611686018427387904,", &[0; 16])).is_err());
        assert!(read_npy(&an_npy("<f8", "3,", &[0; 16])).is_err());
        // none of nothing, however wide: an array of no values, whose shape the caller checks
        let wide = read_npy(&an_npy("<f4", "0, 9223372036854775808", b"")).unwrap();
        assert!(matches!(&wide.data, Data::F32(v) if v.is_empty()) && wide.shape == [0, 9_223_372_036_854_775_808]);
    }

    #[test]
    fn an_archive_from_another_device_is_read_with_nothing_taken_on_trust() {
        // what this program writes is read either way
        let arrays = [("emb", Array::table(vec![0.5; 6], 2, 3)), ("model", Array::one_text("opsis_1"))];
        let mut zip = Vec::new();
        write_npz(&arrays, &mut zip).unwrap();
        assert_eq!(read_npz_foreign(&zip).unwrap().len(), 2);
        // more arrays than a people file has: read as one's own, refused as another device's
        let many: Vec<(String, Array)> =
            (0..FOREIGN_ARRAYS + 1).map(|i| (format!("a{i}"), Array::one_text("x"))).collect();
        let named: Vec<(&str, Array)> = many.iter().map(|(n, a)| (n.as_str(), a.clone())).collect();
        let mut zip = Vec::new();
        write_npz(&named, &mut zip).unwrap();
        assert_eq!(read_npz(&zip).unwrap().len(), FOREIGN_ARRAYS + 1);
        assert!(read_npz_foreign(&zip).is_err());
        // a packed array says itself how large it unpacks: never unpacked for another device
        let mut zip = Vec::new();
        write_npz(&[("emb", Array::table(vec![0.5; 6], 2, 3))], &mut zip).unwrap();
        // (the method, in the file's own header and in the directory: stored -> deflated)
        let local = zip.windows(4).position(|w| w == [0x50, 0x4b, 0x03, 0x04]).unwrap();
        let dir = zip.windows(4).position(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        zip[local + 8] = 8;
        zip[dir + 10] = 8;
        assert!(read_npz_foreign(&zip).is_err());
        // an archive that claims more arrays than any has, or is cut off anywhere, is refused, not followed
        for cut in [0, 10, 21, 22, 40, zip.len() / 2, zip.len() - 1] {
            assert!(read_npz_foreign(&zip[..cut]).is_err(), "cut at {cut}");
        }
        let mut wild = zip.clone();
        let end = wild.len() - 22;
        wild[end + 16..end + 20].copy_from_slice(&0xffff_fff0u32.to_le_bytes()); // where the directory is said to be
        assert!(read_npz_foreign(&wild).is_err());
    }

    #[test]
    fn what_is_written_is_read_back_value_for_value() {
        let arrays = [
            ("emb", Array::table(vec![0.5, -1.25, f32::NAN, 3.0e-7, 0.0, 1.0], 2, 3)),
            ("none", Array::table(Vec::new(), 0, 512)),
            ("model", Array::one_text("opsis_1")),
            ("files", Array::texts(vec!["a.jpg".to_string(), "Zoë's photo 😀.png".to_string(), String::new()])),
            ("nobody", Array::texts(Vec::new())),
            ("thumbs", Array::bytes(vec![0xff, 0xd8, 0, 7, 0xff, 0xd9])),
            ("thumb_off", Array::wholes(vec![0, 6, 6, i64::MAX])),
            ("quality", Array::floats(vec![0.61, f32::NAN])),
        ];
        let mut zip = Vec::new();
        write_npz(&arrays, &mut zip).unwrap();
        let back = read_npz(&zip).unwrap();
        assert_eq!(back.len(), arrays.len());
        for (name, a) in &arrays {
            let b = &back[*name];
            assert_eq!(b.shape, a.shape, "{name}");
            match (&a.data, &b.data) {
                (Data::F32(x), Data::F32(y)) => {
                    assert_eq!(
                        x.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        y.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                    )
                }
                (Data::I64(x), Data::I64(y)) => assert_eq!(x, y),
                (Data::Bytes(x), Data::Bytes(y)) => assert_eq!(x, y),
                (Data::Text(x), Data::Text(y)) => assert_eq!(x, y),
                _ => panic!("{name} came back as another kind"),
            }
        }
    }

    #[test]
    fn a_header_is_the_formats() {
        let npy = write_npy(&Array::table(vec![0.0; 6], 2, 3));
        assert_eq!(&npy[..8], b"\x93NUMPY\x01\x00");
        let head_len = u16::from_le_bytes([npy[8], npy[9]]) as usize;
        assert_eq!((10 + head_len) % 64, 0, "the values begin at a multiple of 64");
        let head = std::str::from_utf8(&npy[10..10 + head_len]).unwrap();
        assert!(
            head.starts_with("{'descr': '<f4', 'fortran_order': False, 'shape': (2, 3), }") && head.ends_with('\n'),
            "{head:?}"
        );
        assert_eq!(npy.len(), 10 + head_len + 24);
        let one = write_npy(&Array::one_text("ab"));
        assert!(
            std::str::from_utf8(&one[10..70])
                .unwrap()
                .starts_with("{'descr': '<U2', 'fortran_order': False, 'shape': (), }")
        );
        let list = write_npy(&Array::floats(vec![1.0; 5]));
        assert!(std::str::from_utf8(&list[10..70]).unwrap().contains("'shape': (5,), }"));
        assert!(std::str::from_utf8(&write_npy(&Array::texts(Vec::new()))[10..70]).unwrap().contains("'<U1'"));
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
