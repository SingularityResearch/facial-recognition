//! A zip archive read just far enough to take named files out of it: its table of contents and one file's
//! bytes, from a file on the disk or from a web server by parts (so one file of 160 MB can be had out of an
//! archive of 289 MB without the rest). A wheel
//! and a NuGet package are zips too. Only what is asked for by name is ever written anywhere.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::Mutex,
};

use flate2::read::DeflateDecoder;

use crate::fetch::{Failure, Fetched, Net};

/// Something whose bytes can be read from any place.
pub trait Source {
    /// How many bytes it has.
    fn size(&self) -> Fetched<u64>;
    /// `n` bytes from `at` on, as a stream.
    fn part(&self, at: u64, n: u64) -> Fetched<Box<dyn Read + '_>>;
}

/// A zip on the disk.
pub struct OnDisk(Mutex<File>);

impl OnDisk {
    pub fn open(path: &std::path::Path) -> Fetched<OnDisk> {
        Ok(OnDisk(Mutex::new(File::open(path)?)))
    }
}

impl Source for OnDisk {
    fn size(&self) -> Fetched<u64> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).metadata()?.len())
    }

    fn part(&self, at: u64, n: u64) -> Fetched<Box<dyn Read + '_>> {
        let mut f = self.0.lock().unwrap_or_else(|e| e.into_inner()).try_clone()?;
        f.seek(SeekFrom::Start(at))?;
        Ok(Box::new(f.take(n)))
    }
}

/// A zip on a web server that serves parts of a file.
pub struct OnServer<'a> {
    pub net: &'a Net,
    pub url: &'a str,
}

impl Source for OnServer<'_> {
    fn size(&self) -> Fetched<u64> {
        self.net.size(self.url)
    }

    fn part(&self, at: u64, n: u64) -> Fetched<Box<dyn Read + '_>> {
        Ok(Box::new(self.net.open(self.url, Some((at, n)), None)?.take(n)))
    }
}

/// A file in the archive, as its table of contents has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    /// 0: stored as it is; 8: deflated
    pub method: u16,
    pub packed: u64,
    pub size: u64,
    /// where its own header begins
    pub header: u64,
}

fn u16_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u64)
}

fn u32_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as u64)
}

/// The most an archive's table of contents may be: far more than any pinned archive's (the largest has
/// 0.1 MB), and little enough to hold in memory whatever a server claims.
const TABLE_AT_MOST: u64 = 64 << 20;

fn all(from: impl Read, n: u64) -> Fetched<Vec<u8>> {
    let mut out = Vec::new();
    from.take(n).read_to_end(&mut out)?;
    if out.len() as u64 != n {
        return Err(Failure::new("the archive ended early"));
    }
    Ok(out)
}

/// The archive's table of contents. An archive of the large kind (over 4 GB, or more than 65,535 files) is
/// refused: none of the pinned ones is.
pub fn entries(src: &dyn Source) -> Fetched<Vec<Entry>> {
    let not_one = || Failure::new("this isn't a zip archive this can read");
    let len = src.size()?;
    // the end record: 22 bytes and a comment of at most 65,535 after the table
    let tail_len = len.min(22 + 65_535);
    let tail = all(src.part(len - tail_len, tail_len)?, tail_len)?;
    let end =
        (0..tail.len().saturating_sub(21)).rev().find(|&i| tail[i..].starts_with(b"PK\x05\x06")).ok_or_else(not_one)?;
    let (count, size, at) = (
        u16_at(&tail, end + 10).ok_or_else(not_one)?,
        u32_at(&tail, end + 12).ok_or_else(not_one)?,
        u32_at(&tail, end + 16).ok_or_else(not_one)?,
    );
    if count == 0xFFFF || size == 0xFFFF_FFFF || at == 0xFFFF_FFFF || at + size > len || size > TABLE_AT_MOST {
        return Err(not_one());
    }
    let table = all(src.part(at, size)?, size)?;
    let mut out = Vec::with_capacity(count as usize);
    let mut p = 0usize;
    for _ in 0..count {
        if !table.get(p..).is_some_and(|t| t.starts_with(b"PK\x01\x02")) {
            return Err(not_one());
        }
        let field16 = |o: usize| u16_at(&table, p + o).ok_or_else(not_one);
        let field32 = |o: usize| u32_at(&table, p + o).ok_or_else(not_one);
        let (name_len, extra, comment) = (field16(28)? as usize, field16(30)? as usize, field16(32)? as usize);
        let name = table.get(p + 46..p + 46 + name_len).ok_or_else(not_one)?;
        out.push(Entry {
            name: String::from_utf8_lossy(name).into_owned(),
            method: field16(10)? as u16,
            packed: field32(20)?,
            size: field32(24)?,
            header: field32(42)?,
        });
        p += 46 + name_len + extra + comment;
    }
    Ok(out)
}

/// One file of the archive, unpacked as it is read. Its length is the caller's to check (`fetch::save` does,
/// with the SHA-256).
pub fn open<'a>(src: &'a dyn Source, entry: &Entry) -> Fetched<Box<dyn Read + 'a>> {
    if !matches!(entry.method, 0 | 8) {
        return Err(Failure::new(format!("{} is packed in a way this can't unpack", entry.name)));
    }
    let head = all(src.part(entry.header, 30)?, 30)?;
    if !head.starts_with(b"PK\x03\x04") {
        return Err(Failure::new("this isn't the expected zip archive"));
    }
    let start = entry.header + 30 + u16_at(&head, 26).unwrap_or(0) + u16_at(&head, 28).unwrap_or(0);
    let packed = src.part(start, entry.packed)?;
    Ok(if entry.method == 0 { packed } else { Box::new(DeflateDecoder::new(packed)) })
}

/// The entry called `name`, or the one entry whose name ends with `name` after a "/".
pub fn find<'e>(entries: &'e [Entry], name: &str) -> Fetched<&'e Entry> {
    let exact = entries.iter().find(|e| e.name == name);
    let ending: Vec<&Entry> = entries.iter().filter(|e| e.name.ends_with(&format!("/{name}"))).collect();
    match (exact, ending.as_slice()) {
        (Some(e), _) => Ok(e),
        (None, [e]) => Ok(e),
        _ => Err(Failure::new(format!("the archive doesn't hold {name} (or holds it more than once)"))),
    }
}

#[cfg(test)]
pub mod tests {
    use std::io::Write;

    use super::*;

    /// A zip archive of these files, each stored or deflated, with a comment at its end.
    pub fn archive(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let (mut out, mut table) = (Vec::new(), Vec::new());
        for (name, data, deflate) in files {
            let packed = if *deflate {
                let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                z.write_all(data).unwrap();
                z.finish().unwrap()
            } else {
                data.to_vec()
            };
            let at = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            let sizes = [(packed.len() as u32).to_le_bytes(), (data.len() as u32).to_le_bytes()].concat();
            let lens = [(name.len() as u16).to_le_bytes(), 4u16.to_le_bytes()].concat();
            out.extend(b"PK\x03\x04\x14\0\0\0");
            out.extend(method.to_le_bytes());
            out.extend([0u8; 8]); // time, date, checksum: not looked at
            out.extend(&sizes);
            out.extend(&lens);
            out.extend(name.as_bytes());
            out.extend([1, 2, 3, 4]); // an extra field of its own, as real archives have
            out.extend(&packed);
            table.extend(b"PK\x01\x02\x14\0\x14\0\0\0");
            table.extend(method.to_le_bytes());
            table.extend([0u8; 8]);
            table.extend(&sizes);
            table.extend((name.len() as u16).to_le_bytes());
            table.extend([0u8; 12]); // extra, comment, disk, attributes
            table.extend(at.to_le_bytes());
            table.extend(name.as_bytes());
        }
        let at = out.len() as u32;
        out.extend(&table);
        out.extend(b"PK\x05\x06\0\0\0\0");
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((table.len() as u32).to_le_bytes());
        out.extend(at.to_le_bytes());
        out.extend(7u16.to_le_bytes());
        out.extend(b"comment");
        out
    }

    struct InMemory(Vec<u8>);

    impl Source for InMemory {
        fn size(&self) -> Fetched<u64> {
            Ok(self.0.len() as u64)
        }

        fn part(&self, at: u64, n: u64) -> Fetched<Box<dyn Read + '_>> {
            Ok(Box::new(&self.0[at as usize..(at + n) as usize]))
        }
    }

    #[test]
    fn a_named_file_comes_out_of_an_archive_as_it_went_in() {
        let long: Vec<u8> = (0..200_000u32).flat_map(|v| (v.wrapping_mul(v) as u16).to_le_bytes()).collect();
        let zip = InMemory(archive(&[
            ("a/first.txt", b"stored as it is", false),
            ("a/b/second.bin", &long, true),
            ("third", b"", true),
        ]));
        let list = entries(&zip).unwrap();
        assert_eq!(
            list.iter().map(|e| (e.name.as_str(), e.size)).collect::<Vec<_>>(),
            [("a/first.txt", 15), ("a/b/second.bin", 400_000), ("third", 0)]
        );
        let read = |name: &str| {
            let mut out = Vec::new();
            open(&zip, find(&list, name).unwrap()).unwrap().read_to_end(&mut out).unwrap();
            out
        };
        assert_eq!(read("a/first.txt"), b"stored as it is");
        assert_eq!(read("second.bin"), long); // by the end of its name
        assert_eq!(read("third"), b"");
        assert!(find(&list, "fourth").is_err() && find(&list, "b").is_err());
        // what is no archive, and one cut short, are refused
        assert!(entries(&InMemory(b"not an archive at all, whatever its name says".to_vec())).is_err());
        let mut cut = archive(&[("x", b"1234567890", false)]);
        cut.truncate(cut.len() - 30);
        assert!(entries(&InMemory(cut)).is_err());
    }
}
