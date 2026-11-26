//! What each reading means, inside the app (`/readings` on the tracker's server): `docs/READINGS.md` as a page,
//! carried in the program, so that whoever looks at a reading can read what it is, what its values mean and why
//! it matters where the reading is shown, without the internet (the owner, 2026-11-25: "ALL currently supported
//! sensor readings should have a name and summary", "most people dont know what valence means, etc. so it ALL needs
//! to be explained"). The tracker page and the Cameras page show it in a sheet over themselves, at the part a
//! reading's name or a section names. The file in the repository stays the one text; `markdown.rs` turns it into a
//! page that leads nowhere outside the app.

/// The text as it was when the program was built.
const READINGS: &str = include_str!("../../../docs/READINGS.md");
/// Where the same text is on the internet.
const ONLINE: &str = "https://github.com/SingularityResearch/facial-recognition/blob/main/docs/READINGS.md";

/// The text as a page.
pub fn page() -> String {
    crate::markdown::page(READINGS, "What each reading means", &format!("This page on the internet: {ONLINE}"))
}

/// The parts of the page, as the links to them name them.
pub fn parts() -> Vec<String> {
    READINGS
        .lines()
        .filter_map(|l| l.strip_prefix("## ").or_else(|| l.strip_prefix("### ")).or_else(|| l.strip_prefix("#### ")))
        .map(crate::markdown::anchor)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_readings_are_a_whole_page_that_leads_nowhere_outside_the_app() {
        let page = page();
        assert!(page.contains("<h1 id=\"what-each-reading-means\">What each reading means</h1>"));
        assert!(page.contains(ONLINE));
        assert!(page.contains("not a medical device"));
        assert!(crate::markdown::tests::leads_nowhere(&page) >= 3);
        // each reading is a part of its own, and a part's name is never two parts'
        let parts = parts();
        for part in [
            "valence-face",
            "voice-valence",
            "share-of-votes",
            "lead",
            "why-it-isnt-clear",
            "spreads-away",
            "beat-clarity",
        ] {
            assert!(parts.iter().any(|p| p == part), "no part {part}");
            assert!(page.contains(&format!(" id=\"{part}\"")), "the page has no part {part}");
        }
        let mut seen = std::collections::HashSet::new();
        for p in &parts {
            assert!(seen.insert(p), "two parts are called {p}");
        }
    }

    /// Every part the pages open the readings at (a section of the Signals chart, one of its readings, a group of
    /// the Cameras page's readout) is a part of the page.
    #[test]
    fn the_pages_name_parts_the_readings_have() {
        let parts = parts();
        let mut named = 0;
        for script in [include_str!("../../../web/signals_chart.js"), include_str!("../../../web/cameras_ui.js")] {
            let at = script.find("// the parts of docs/READINGS.md").expect("the parts the page names");
            let list = &script[at..at + script[at..].find("\n};").unwrap()];
            for entry in list.split(": \"").skip(1) {
                let part = &entry[..entry.find('"').unwrap()];
                assert!(parts.iter().any(|p| p == part), "the readings have no part {part}");
                named += 1;
            }
        }
        assert!(named >= 60, "the pages name {named} parts of the readings");
    }
}
