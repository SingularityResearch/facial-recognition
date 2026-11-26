//! The setup guide inside the app (`/guide` on the labeling tool's server): `docs/SETUP.md` as a page, carried in
//! the program, so that someone who installed the app has the guide where the Setup page names it, without the
//! internet and without the repository (the owner, 2026-11-25: "We need a wizard guide"). The Setup page shows it
//! over itself, at the part it names. The file in the repository stays the one text; `markdown.rs` turns it into a
//! page that leads nowhere outside the app.

/// The guide as it was when the program was built.
const GUIDE: &str = include_str!("../../../docs/SETUP.md");
/// Where the same text is on the internet.
const ONLINE: &str = "https://github.com/SingularityResearch/facial-recognition/blob/main/docs/SETUP.md";

/// The guide as a page.
pub fn page() -> String {
    crate::markdown::page(GUIDE, "Setup guide", &format!("This guide on the internet: {ONLINE}"))
}

/// The parts of the guide, as the links to them name them.
pub fn parts() -> Vec<String> {
    GUIDE
        .lines()
        .filter_map(|l| l.strip_prefix("## ").or_else(|| l.strip_prefix("### ")))
        .map(crate::markdown::anchor)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_is_a_whole_page_that_leads_nowhere_outside_the_app() {
        let page = page();
        assert!(page.contains("<h1 id=\"setting-up-face-recognizer-step-by-step\">"));
        assert!(page.contains(ONLINE));
        crate::markdown::tests::leads_nowhere(&page);
        // its steps and commands are there as steps and commands
        assert!(
            page.contains("<ol>") && page.contains("<pre><code>") && page.contains("listen_addresses = &#39;*&#39;")
        );
    }

    /// Every part the Setup page opens the guide at is a part of the guide.
    #[test]
    fn the_setup_page_names_parts_the_guide_has() {
        let script = include_str!("../../../web/setup.js");
        let parts = parts();
        let mut named = 0;
        for at in script.split("guideAt(\"").skip(1) {
            let part = &at[..at.find('"').unwrap()];
            assert!(parts.iter().any(|p| p == part), "the guide has no part {part}");
            named += 1;
        }
        assert!(named >= 3, "the Setup page names {named} parts of the guide");
    }
}
