//! The pages' files (`web/`: the tab page, the labeling tool's, the tracker's and Cameras pages', what they
//! share) and the icon (`assets/icons`). A program carries them inside itself, so that an installed app is the program and nothing
//! beside it; `--pages <folder>` reads them from a folder instead, at every request, so that an edit shows
//! on refresh (development, and the checks).

use std::path::Path;

macro_rules! built {
    ($($name:literal),* $(,)?) => {
        [$(($name, include_bytes!(concat!("../../../web/", $name)).as_slice())),* ,
         ("icons/icon.ico", include_bytes!("../../../assets/icons/icon.ico").as_slice())]
    };
}

/// (name below the pages' folder, the file as it was when the program was built)
const BUILT: [(&str, &[u8]); 17] = built![
    "app_ui.html",
    "app_ui.js",
    "ask.js",
    "cameras_ui.html",
    "cameras_ui.js",
    "expression.js",
    "jpeg_worker.js",
    "label_ui.html",
    "label_ui.js",
    "page_store.js",
    "settings_bridge.js",
    "signals.js",
    "signals_chart.css",
    "signals_chart.js",
    "tracker_ui.html",
    "tracker_ui.js",
];

/// The program's own copy of a page file.
pub fn built(name: &str) -> Option<&'static [u8]> {
    BUILT.iter().find(|(n, _)| *n == name).map(|(_, bytes)| *bytes)
}

/// A page file: from `folder` when one was named (and only from there: a file it lacks is not found, so
/// that a wrong folder shows), else the program's own copy. Only the names a server lists reach this.
pub fn read(folder: &Path, name: &str) -> Option<Vec<u8>> {
    if folder.as_os_str().is_empty() {
        return built(name).map(<[u8]>::to_vec);
    }
    std::fs::read(folder.join(name)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_carries_its_pages_and_a_named_folder_is_read_instead() {
        // the program's own: every file, and nothing that isn't listed
        for (name, bytes) in BUILT {
            assert!(!bytes.is_empty(), "{name}");
            assert_eq!(read(Path::new(""), name).as_deref(), Some(bytes), "{name}");
        }
        assert!(built("app_ui.html").is_some_and(|page| page.starts_with(b"<!doctype html>")));
        assert_eq!(read(Path::new(""), "../Cargo.toml"), None);
        assert_eq!(read(Path::new(""), "nothing.js"), None);
        // a folder: its files, and only its files
        let dir = std::env::temp_dir().join(format!("face-pagefiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ask.js"), b"// another").unwrap();
        assert_eq!(read(&dir, "ask.js").as_deref(), Some(b"// another".as_slice()));
        assert_eq!(read(&dir, "app_ui.html"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_file_a_server_names_is_in_the_program() {
        let named = crate::app::PAGES.iter().chain(&crate::label::PAGES).chain(&crate::tracker::PAGES);
        for (path, file, _) in named {
            assert!(built(file).is_some(), "{path}: {file}");
        }
    }
}
