//! What a Windows program or library needs from outside itself: the libraries it names (its imports, the
//! delayed ones too), read from the file. Nothing may be assumed to be installed on a user's computer (the
//! owner's rule, 2026-11-20), and the usual way to break that without noticing is a library that asks for the
//! Visual C++ runtime, which is on a developer's computer and not part of Windows. So what is shipped is
//! read, not believed: every name must be Windows' own or a file shipped beside it.

use std::path::Path;

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn u32_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// The libraries a Windows program or library (a PE file) names, as it writes them. None for a file that
/// isn't one.
pub fn imports(file: &[u8]) -> Option<Vec<String>> {
    let pe = u32_at(file, 0x3c).filter(|_| file.starts_with(b"MZ"))?;
    if file.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let (sections, optional) = (u16_at(file, pe + 6)?, pe + 24);
    // where the tables of directories begin: after 96 bytes of an optional header for 32 bits, 112 for 64
    let dirs = optional + if u16_at(file, optional)? == 0x20b { 112 } else { 96 };
    let table = optional + u16_at(file, pe + 20)?;
    // an address as the loaded program has it -> its place in the file
    let place = |rva: usize| -> Option<usize> {
        (0..sections).find_map(|k| {
            let s = table + 40 * k;
            let (size, at, raw) =
                (u32_at(file, s + 8)?.max(u32_at(file, s + 16)?), u32_at(file, s + 12)?, u32_at(file, s + 20)?);
            (rva >= at && rva < at + size).then(|| rva - at + raw)
        })
    };
    let name = |rva: usize| -> Option<String> {
        let at = place(rva)?;
        let end = file.get(at..)?.iter().position(|&c| c == 0)?;
        Some(String::from_utf8_lossy(&file[at..at + end]).into_owned())
    };
    let mut out = Vec::new();
    // the imports (directory 1: entries of 20 bytes, the name's address at 12) and the delayed ones
    // (directory 13: entries of 32 bytes, the name's address at 4), each list ended by an empty entry
    for (dir, size, at_name) in [(1usize, 20usize, 12usize), (13, 32, 4)] {
        let Some(mut at) = u32_at(file, dirs + 8 * dir).filter(|rva| *rva != 0).and_then(place) else { continue };
        while let Some(rva) = u32_at(file, at + at_name).filter(|rva| *rva != 0) {
            out.extend(name(rva));
            at += size;
        }
    }
    Some(out)
}

/// Whether a library of this name is Windows' own: one of its sets of functions ("api-ms-win-...", which
/// includes the C runtime every Windows since 10 has), or a file in its system folder that is not one of the
/// runtimes programs bring with them (those can be in the system folder too, when some program's installer
/// put them there: that is the case this is for).
pub fn of_windows(name: &str, system: &Path) -> bool {
    let n = name.to_ascii_lowercase();
    let brought = ["vcruntime", "msvcp", "concrt", "vccorlib", "vcomp", "vcamp", "mfc", "atl1", "msvcr"];
    if n.starts_with("api-ms-win-") || n.starts_with("ext-ms-") {
        return true;
    }
    !brought.iter().any(|b| n.starts_with(b)) && system.join(&n).is_file()
}

/// For every program and library in `folders` (`.exe`, `.dll`): the libraries it names that are neither
/// Windows' own nor a file in one of the folders: (the file's name, what it needs). Empty: what is in the
/// folders needs nothing installed.
pub fn from_outside(folders: &[&Path], system: &Path) -> Vec<(String, String)> {
    let files: Vec<std::path::PathBuf> = (folders.iter())
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|f| f.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll") || e.eq_ignore_ascii_case("exe")))
        .collect();
    let here: Vec<String> =
        files.iter().filter_map(|p| Some(p.file_name()?.to_string_lossy().to_ascii_lowercase())).collect();
    let mut out = Vec::new();
    for path in &files {
        let Some(named) = std::fs::read(path).ok().and_then(|b| imports(&b)) else { continue };
        for lib in named {
            if !here.contains(&lib.to_ascii_lowercase()) && !of_windows(&lib, system) {
                out.push((path.file_name().unwrap_or_default().to_string_lossy().into_owned(), lib));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Windows' system folder on this computer.
pub fn system_folder() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::folder;

    /// A file laid out like a Windows library for 64 bits that names these libraries, the last `delayed` of
    /// them as delayed imports.
    fn library(names: &[&str], delayed: usize) -> Vec<u8> {
        let mut f = vec![0u8; 0x400];
        f[..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        f[0x80..0x84].copy_from_slice(b"PE\0\0");
        f[0x86..0x88].copy_from_slice(&1u16.to_le_bytes()); // one section
        f[0x94..0x96].copy_from_slice(&240u16.to_le_bytes()); // the optional header's size
        f[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes()); // for 64 bits
        let (dirs, section, base) = (0x98 + 112, 0x98 + 240, 0x1000usize);
        // the one section: 0x1000 bytes at address 0x1000, kept in the file from 0x400 on
        for (at, v) in [(8, 0x1000u32), (12, base as u32), (16, 0x1000), (20, 0x400)] {
            f[section + at..section + at + 4].copy_from_slice(&v.to_le_bytes());
        }
        let mut body = vec![0u8; 0x1000];
        let (now, later) = names.split_at(names.len() - delayed);
        let mut text = 0x800usize;
        let mut put = |body: &mut Vec<u8>, name: &str| {
            let at = text;
            body[at..at + name.len()].copy_from_slice(name.as_bytes());
            text += name.len() + 1;
            (base + at) as u32
        };
        for (k, n) in now.iter().enumerate() {
            let rva = put(&mut body, n);
            body[k * 20 + 12..k * 20 + 16].copy_from_slice(&rva.to_le_bytes());
        }
        for (k, n) in later.iter().enumerate() {
            let rva = put(&mut body, n);
            body[0x400 + k * 32 + 4..0x400 + k * 32 + 8].copy_from_slice(&rva.to_le_bytes());
        }
        f[dirs + 8..dirs + 12].copy_from_slice(&(base as u32).to_le_bytes());
        if delayed > 0 {
            f[dirs + 8 * 13..dirs + 8 * 13 + 4].copy_from_slice(&((base + 0x400) as u32).to_le_bytes());
        }
        f.extend(body);
        f
    }

    #[test]
    fn what_a_library_needs_from_outside_is_read_from_the_file() {
        let made = library(
            &["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-heap-l1-1-0.dll", "beside.dll", "MSVCP140.dll"],
            1,
        );
        assert_eq!(
            imports(&made).unwrap(),
            ["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-heap-l1-1-0.dll", "beside.dll", "MSVCP140.dll"]
        );
        assert!(imports(b"no program at all").is_none() && imports(&library(&[], 0)).unwrap().is_empty());
        // a made-up system folder that has Windows' own library, and the C++ runtime some installer put there
        let (system, shipped) = (folder("system"), folder("shipped"));
        for name in ["kernel32.dll", "vcruntime140.dll", "msvcp140.dll"] {
            std::fs::write(system.join(name), b"x").unwrap();
        }
        std::fs::write(shipped.join("main.dll"), &made).unwrap();
        std::fs::write(shipped.join("beside.dll"), library(&["kernel32.dll"], 0)).unwrap();
        std::fs::write(shipped.join("notes.txt"), b"not looked at").unwrap();
        // the runtimes are needed from outside although the system folder has them; Windows' own are not
        let need = |d: &Path| from_outside(&[d], &system);
        assert_eq!(
            need(&shipped),
            [
                ("main.dll".to_string(), "MSVCP140.dll".to_string()),
                ("main.dll".to_string(), "VCRUNTIME140.dll".to_string())
            ]
        );
        // shipped beside it: nothing is needed from outside any more
        for name in ["VCRUNTIME140.dll", "msvcp140.dll"] {
            std::fs::write(shipped.join(name), library(&["KERNEL32.dll"], 0)).unwrap();
        }
        assert!(need(&shipped).is_empty());
        for d in [system, shipped] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// The programs this was built into, on Windows: with the C runtime linked in they name nothing that
    /// isn't Windows' own.
    #[cfg(windows)]
    #[test]
    fn this_program_needs_nothing_installed() {
        let me = std::env::current_exe().unwrap();
        let named = imports(&std::fs::read(&me).unwrap()).unwrap();
        let outside: Vec<&String> = named.iter().filter(|n| !of_windows(n, &system_folder())).collect();
        assert!(!named.is_empty() && outside.is_empty(), "{outside:?}");
    }
}
