//! The licences of other people's parts inside the app (`/notices` on the tracker's server, opened from the tracker
//! page's Licences link): `NOTICE` and `licenses/rust-libraries.txt` as they were when the program was built. Their
//! licences ask for these words to go with the app; carried in the program, they do wherever it runs, phones too,
//! and can be read without the internet. The files in the repository stay the one text: this only shows them.

/// What the app carries and downloads of other people's, with their licences.
const NOTICE: &str = include_str!("../../../NOTICE");
/// The Rust libraries built into the programs, each with its licence's text (made by cargo-about).
const LIBRARIES: &str = include_str!("../../../licenses/rust-libraries.txt");

/// Both, as a page.
pub fn page() -> String {
    crate::markdown::text_page("Licences", &[("Notice", NOTICE), ("The Rust libraries in the programs", LIBRARIES)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notices_are_a_whole_page_that_leads_nowhere_outside_the_app() {
        let page = page();
        assert!(page.contains("<h1 id=\"licences\">Licences</h1>"));
        // the Independent JPEG Group's credit, which its licence asks of the app's documentation
        assert!(page.contains("This software is based in part on the work of the Independent JPEG Group."));
        // every library's licence, as cargo-about made the file
        assert!(page.contains("Used by:") && page.contains("Permission is hereby granted"));
        // as text: nothing of either file is taken as the page's own markup
        assert!(!page[page.find("<main>").unwrap()..].contains("<http"));
        assert_eq!(crate::markdown::tests::leads_nowhere(&page), 0, "no link at all");
    }
}
