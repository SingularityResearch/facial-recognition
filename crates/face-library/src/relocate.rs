//! Finding photos again (run at each start of the labeling tool):
//! photos whose recorded path is gone, because a drive letter changed or a library was moved, are pointed at
//! where they are now: first the same folder on another drive, then a file with the same name and size in
//! the photo folders that were added. Those folders follow the move too. Photos that aren't found keep their
//! old path.
//!
//! (A library from before faces had permanent IDs is not upgraded here:
//! every library in use was upgraded long ago, and this engine refuses one that wasn't.)

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use face_files::{filejson, paths::atomic_write};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::{
    labels::truthy,
    photofacts::{split, suffix},
    photofiles::{path_text, seconds, settings},
    store::Store,
};

/// the endings of the pictures and videos the programs read
pub const IMAGE_EXT: [&str; 10] = [".jpg", ".jpeg", ".png", ".webp", ".bmp", ".heic", ".heif", ".gif", ".tif", ".tiff"];
pub const VIDEO_EXT: [&str; 5] = [".mov", ".mp4", ".m4v", ".avi", ".mkv"];

const SEPS: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };

/// Every picture and video under a folder (or the one file), in the fixed order of paths: part by part,
/// and on Windows without regard to upper and lower case.
pub fn list_media(folder: &Path, exts: &[&str]) -> Vec<PathBuf> {
    if folder.is_file() {
        return vec![folder.to_path_buf()];
    }
    let mut found = Vec::new();
    let mut todo = vec![folder.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => todo.push(path),
                _ => {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if exts.contains(&suffix(&name).1) && path.is_file() {
                        found.push(path);
                    }
                }
            }
        }
    }
    let parts = |p: &PathBuf| -> Vec<String> {
        let text = p.to_string_lossy();
        text.split(SEPS).map(|part| if cfg!(windows) { part.to_lowercase() } else { part.to_string() }).collect()
    };
    found.sort_by_cached_key(parts);
    found
}

/// The file's size and time if it could be the photo a record describes: it is a file, of the recorded size
/// (when one was recorded).
fn same_file(path: &Path, record: &Map<String, Value>) -> Option<std::fs::Metadata> {
    let st = std::fs::metadata(path).ok()?;
    let size = record.get("size").filter(|s| truthy(s));
    (st.is_file() && size.is_none_or(|s| s.as_u64() == Some(st.len()))).then_some(st)
}

/// A path's parts: the root (with a drive) first.
fn parts(path: &str) -> Vec<String> {
    let text = path_text(path);
    let root = match text.as_bytes() {
        // \\computer\share\ is one root
        [b'\\', b'\\', ..] if cfg!(windows) => text[2..].match_indices('\\').nth(1).map_or(text.len(), |(k, _)| k + 3),
        [_, b':', b'\\', ..] if cfg!(windows) => 3,
        [_, b':', ..] if cfg!(windows) => 2,
        [b'/', ..] | [b'\\', ..] => 1,
        _ => 0,
    };
    let mut out: Vec<String> = if root > 0 { vec![text[..root].to_string()] } else { Vec::new() };
    out.extend(text[root..].split(SEPS).filter(|p| !p.is_empty()).map(str::to_string));
    out
}

fn joined(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        if !out.is_empty() && !out.ends_with(SEPS) {
            out.push(std::path::MAIN_SEPARATOR);
        }
        out.push_str(part);
    }
    out
}

/// (old folder, new folder) that a move swapped: the two paths with their shared ending taken off.
fn moved_root(old: &str, new: &str) -> (String, String) {
    let (mut o, mut n) = (parts(old), parts(new));
    while o.len() > 1 && n.len() > 1 && o[o.len() - 1].to_lowercase() == n[n.len() - 1].to_lowercase() {
        o.pop();
        n.pop();
    }
    (joined(&o), joined(&n))
}

/// The drives this computer has (Windows).
fn drives() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    ('A'..='Z').map(|c| format!("{c}:\\")).filter(|d| Path::new(d).exists()).collect()
}

/// Point the photos whose path is gone at where they are now, and save. `folders`: the photo folders that
/// were added; `data`: the library's folder (its settings follow the move); `log`: told what is looked
/// through. {"missing", "found", "folders": {old: new}}.
pub fn relocate(
    store: &mut Store,
    data: &Path,
    folders: &[String],
    log: &mut dyn FnMut(String),
) -> Result<Value, String> {
    let with_path: Vec<(String, String)> = (store.photos.iter())
        .filter_map(|(h, p)| Some((h.clone(), p.get("path")?.as_str().filter(|p| !p.is_empty())?.to_string())))
        .collect();
    let mut is_dir: HashMap<String, bool> = HashMap::new();
    let missing: IndexMap<String, String> = (with_path.into_iter())
        .filter(|(_, path)| {
            let folder = path_text(split(path).0);
            let there = *is_dir.entry(folder.clone()).or_insert_with(|| Path::new(&folder).is_dir());
            !there || !Path::new(path).exists()
        })
        .collect();
    let mut out = json!({"missing": missing.len(), "found": 0, "folders": {}});
    if missing.is_empty() {
        return Ok(out);
    }
    let record = |h: &str| store.photos.get(h).and_then(Value::as_object).cloned().unwrap_or_default();
    let mut found: IndexMap<String, (PathBuf, std::fs::Metadata)> = IndexMap::new();

    // 1. the same folder on another drive letter
    let drives = drives();
    let mut by_dir: IndexMap<String, Vec<&String>> = IndexMap::new();
    for (h, path) in &missing {
        by_dir.entry(path_text(split(path).0)).or_default().push(h);
    }
    for (dir, hashes) in &by_dir {
        let whole = parts(dir);
        if whole.len() < 2 || !whole[0].ends_with(SEPS) {
            continue;
        }
        for drive in &drives {
            let there = Path::new(drive).join(joined(&whole[1..]));
            if there.to_string_lossy().to_lowercase() == dir.to_lowercase() || !there.is_dir() {
                continue;
            }
            for h in hashes {
                let file = there.join(split(&missing[*h]).1);
                if let (false, Some(st)) = (found.contains_key(*h), same_file(&file, &record(h))) {
                    found.insert((*h).clone(), (file, st));
                }
            }
        }
    }

    // 2. by name and size anywhere in the photo folders that were added
    let left: Vec<&String> = missing.keys().filter(|h| !found.contains_key(*h)).collect();
    if !left.is_empty() {
        let mut index: HashMap<String, Vec<PathBuf>> = HashMap::new();
        let both: Vec<&str> = IMAGE_EXT.iter().chain(&VIDEO_EXT).copied().collect();
        for folder in folders.iter().filter(|f| Path::new(f).is_dir()) {
            log(format!("Looking for {} moved photos in {folder} ...", left.len()));
            for p in list_media(Path::new(folder), &both) {
                index.entry(p.file_name().unwrap_or_default().to_string_lossy().to_lowercase()).or_default().push(p);
            }
        }
        for h in left {
            let name = split(&missing[h]).1.to_lowercase();
            let hit = index.get(&name).into_iter().flatten().find_map(|p| Some((p.clone(), same_file(p, &record(h))?)));
            if let Some(hit) = hit {
                found.insert(h.clone(), hit);
            }
        }
    }
    if found.is_empty() {
        return Ok(out);
    }
    let mut roots: IndexMap<String, String> = IndexMap::new();
    for (h, (path, st)) in &found {
        let Some(rec) = store.photos.get_mut(h).and_then(Value::as_object_mut) else { continue };
        let old = rec.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        let new = path_text(&path.to_string_lossy());
        let mtime = st.modified().map_or(0.0, seconds).trunc() as i64;
        rec.insert("path".to_string(), json!(new));
        rec.insert("mtime".to_string(), json!(mtime));
        let (o, n) = moved_root(&old, &new);
        roots.insert(o, n);
    }
    store.save_photos()?;
    out["found"] = json!(found.len());

    // the folders that were added follow the move (the longest old folder that fits wins)
    let mut s = settings(data);
    let was: Vec<String> = s
        .get("folders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_str().map(str::to_string))
        .collect();
    let mut olds: Vec<&String> = roots.keys().collect();
    olds.sort_by_key(|o| std::cmp::Reverse(o.chars().count()));
    let mut now: Vec<String> = Vec::new();
    for mut f in was.clone() {
        if !Path::new(&f).exists() {
            let lower = f.to_lowercase();
            let fits = |o: &&&String| {
                let old = o.to_lowercase();
                lower == old
                    || lower.starts_with(&format!("{}{}", old.trim_end_matches(['\\', '/']), std::path::MAIN_SEPARATOR))
            };
            if let Some(o) = olds.iter().find(fits) {
                let moved = format!("{}{}", roots[*o], f.chars().skip(o.chars().count()).collect::<String>());
                if Path::new(&moved).is_dir() {
                    out["folders"][&f] = json!(moved);
                    f = moved;
                }
            }
        }
        if !now.contains(&f) {
            now.push(f);
        }
    }
    if now != was {
        s.insert("folders".to_string(), json!(now));
        atomic_write(&data.join("settings.json"), filejson::rows_of(&s, true).as_bytes())
            .map_err(|e| format!("can't write settings.json: {e}"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_that_moved_are_found_by_name_and_size_and_the_folders_follow() {
        let dir = std::env::temp_dir().join(format!("face-relocate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (data, now) = (dir.join("data"), dir.join("now").join("album"));
        std::fs::create_dir_all(now.join("trip")).unwrap();
        std::fs::write(now.join("a.jpg"), b"aaaa").unwrap();
        std::fs::write(now.join("trip").join("B.JPG"), b"bbbbbb").unwrap();
        std::fs::write(now.join("trip").join("c.mp4"), b"cc").unwrap();
        std::fs::write(now.join("d.jpg"), b"another size").unwrap();
        std::fs::write(now.join("notes.txt"), b"no picture").unwrap();
        let was = dir.join("before").join("album");
        let at = |p: &Path| p.to_string_lossy().to_string();
        let mut store = Store::load(&data, "opsis_1", 4).unwrap();
        store.photos = json!({
            "h-a": {"path": at(&was.join("a.jpg")), "size": 4, "mtime": 1},
            "h-b": {"path": at(&was.join("trip").join("b.jpg")), "size": 6, "mtime": 1},
            "h-c": {"path": at(&was.join("trip").join("c.mp4")), "mtime": 1, "video": true},
            "h-d": {"path": at(&was.join("d.jpg")), "size": 4, "mtime": 1},
            "h-e": {"path": at(&now.join("a.jpg")), "size": 4, "mtime": 7},
            "h-f": {"size": 3},
        })
        .as_object()
        .unwrap()
        .clone();
        std::fs::write(
            data.join("settings.json"),
            serde_json::to_string(&json!({"folders": [at(&was), at(&was.join("trip")), at(&dir)]})).unwrap(),
        )
        .unwrap();
        let mut said = Vec::new();
        let out =
            relocate(&mut store, &data, &[at(&dir.join("nowhere")), at(&now)], &mut |line| said.push(line)).unwrap();
        assert_eq!(
            (out["missing"].as_u64(), out["found"].as_u64(), said.len()),
            (Some(4), Some(3), 1),
            "{out} {said:?}"
        );
        assert!(said[0].starts_with("Looking for 4 moved photos in "));
        assert_eq!(store.photos["h-a"]["path"], json!(at(&now.join("a.jpg"))));
        assert_eq!(
            store.photos["h-b"]["path"],
            json!(at(&now.join("trip").join("B.JPG"))),
            "whatever case its name has now"
        );
        assert_eq!(
            store.photos["h-c"]["path"],
            json!(at(&now.join("trip").join("c.mp4"))),
            "videos too, and without a recorded size"
        );
        assert_eq!(store.photos["h-d"]["path"], json!(at(&was.join("d.jpg"))), "another size: not that photo");
        assert_eq!(
            (&store.photos["h-e"]["mtime"], store.photos["h-a"]["mtime"].as_i64().unwrap() > 1_600_000_000),
            (&json!(7), true)
        );
        // the folders that were added follow the move; what is still there stays
        assert_eq!(out["folders"], json!({at(&was): at(&now), at(&was.join("trip")): at(&now.join("trip"))}));
        assert_eq!(crate::photofiles::folders(&data), [at(&now), at(&now.join("trip")), at(&dir)]);
        // saved, and nothing more to find
        let again = Store::load(&data, "opsis_1", 4).unwrap();
        assert_eq!(again.photos, store.photos);
        let out = relocate(&mut store, &data, &[at(&now)], &mut |_| {}).unwrap();
        assert_eq!(out, json!({"missing": 1, "found": 0, "folders": {}}));
        // the files of a folder, in the order of their names whatever their case
        let listed: Vec<String> =
            list_media(&now, &IMAGE_EXT).iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(listed, ["a.jpg", "d.jpg", "B.JPG"]);
        assert_eq!(
            moved_root(&at(&was.join("trip").join("b.jpg")), &at(&now.join("trip").join("B.JPG"))).1,
            at(&dir.join("now"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
