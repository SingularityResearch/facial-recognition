//! The licence, accepted at the app's first start where no installer asked for it: a Mac, Linux and the Raspberry
//! Pi (the owner, 2026-11-25: "Add accept / decline page at Mac first start on Mac / Linux / Pi"). On Windows Setup
//! shows it on a page of its own and goes on only once it is accepted, so the app doesn't ask again there.
//!
//! It is the first stage of the first-run question (`firstrun.rs`, its page `pages/first_run.html`): before anything
//! else is shown or started, the app's own page shows the licence with Accept and Decline. Accepted, that is kept in
//! the private folder with the licence's SHA-256, and the start goes on; declined, the app ends. A licence whose text
//! has changed is asked about again. A start that nobody watches (no window: a check, a test copy) isn't asked.

use std::path::Path;

use ring::digest::{SHA256, digest};
use serde_json::{Value, json};

/// The licence as it was when the program was built.
pub const TEXT: &str = include_str!("../../../LICENSE");
/// in the private folder: which licence was accepted, and when
const KEPT: &str = "licence.json";

fn sha256() -> String {
    digest(&SHA256, TEXT.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Its version line ("Version 1.1, 24 November 2026").
pub fn version() -> &'static str {
    TEXT.lines().find(|l| l.starts_with("Version ")).unwrap_or("")
}

/// Whether this licence was accepted on this computer, for this private folder.
pub fn accepted(home: &Path) -> bool {
    let kept: Option<Value> = std::fs::read(home.join(KEPT)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    kept.is_some_and(|k| k["sha256"] == sha256())
}

/// Whether the first start asks for it here: not on Windows (Setup asked), not once accepted.
pub fn due(home: &Path) -> bool {
    !cfg!(windows) && !accepted(home)
}

/// Accepted: kept, with when and which.
pub fn accept(home: &Path) -> Result<(), String> {
    let text = face_files::filejson::dumps(
        &json!({"sha256": sha256(), "version": version(), "at": face_files::paths::utc_now()}),
    );
    face_files::paths::atomic_write(&home.join(KEPT), text.as_bytes()).map_err(|e| e.to_string())
}

/// The licence for a page: a line that was broken only for the file's width (90 characters or more) goes on into the
/// next, so that the page wraps the words itself; short lines (the heading) and list items stay lines. No word
/// changes (a test holds it), as Windows' Setup shows it (`packaging/windows/build.ps1`).
pub fn for_page() -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in TEXT.lines() {
        let item = {
            let t = line.trim();
            t.starts_with("- ")
                || (t.starts_with('(') && t.find(") ").is_some_and(|i| i <= 3))
                || t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        };
        match lines.last_mut() {
            Some(previous) if previous.chars().count() >= 90 && !line.trim().is_empty() && !item => {
                previous.push(' ');
                previous.push_str(line.trim());
            }
            _ => lines.push(line.to_string()),
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_shows_every_word_of_the_licence_in_its_order() {
        let words = |t: &str| t.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(words(&for_page()), words(TEXT));
        assert!(for_page().lines().count() < TEXT.lines().count());
        // the heading and the list items keep their own lines
        let page = for_page();
        assert!(page.starts_with("Face Recognizer\nCopyright"));
        assert!(page.lines().any(|l| l.trim_start().starts_with("- build the Work")));
        assert!(page.lines().any(|l| l.trim_start().starts_with("(b) Install and use")));
        assert!(version().starts_with("Version "));
    }

    #[test]
    fn accepted_once_it_is_kept_and_asked_again_for_another_licence() {
        let home = std::env::temp_dir().join(format!("face-licence-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(!accepted(&home));
        assert_eq!(due(&home), !cfg!(windows));
        accept(&home).unwrap();
        assert!(accepted(&home) && !due(&home));
        // another licence's acceptance is no acceptance of this one
        std::fs::write(home.join(KEPT), r#"{"sha256": "00"}"#).unwrap();
        assert!(!accepted(&home));
        let _ = std::fs::remove_dir_all(&home);
    }
}
