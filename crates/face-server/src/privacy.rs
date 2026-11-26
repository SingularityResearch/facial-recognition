//! The privacy policy inside the app (`/privacy` on the tracker's server): `PRIVACY.md` as a page, carried in
//! the program, so that it can be read where the app runs and without the internet (the stores want the
//! policy inside the app, not only at an address). The file in the repository stays the one text; `markdown.rs`
//! turns it into a page that leads nowhere outside the app.

/// The policy as it was when the program was built.
const POLICY: &str = include_str!("../../../PRIVACY.md");
/// Where the same text is on the internet.
const ONLINE: &str = "https://github.com/SingularityResearch/facial-recognition/blob/main/PRIVACY.md";

/// The policy as a page.
pub fn page() -> String {
    crate::markdown::page(POLICY, "Privacy policy", &format!("This policy on the internet: {ONLINE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_is_a_whole_page_that_leads_nowhere_outside_the_app() {
        let page = page();
        assert!(page.contains("<h1 id=\"privacy-policy\">Privacy policy</h1>"));
        assert!(page.contains("not a medical device"));
        assert!(page.contains(ONLINE));
        assert!(crate::markdown::tests::leads_nowhere(&page) >= 1);
        let shown = &page[page.find("<main>").unwrap()..];
        assert!(shown.matches("<table>").count() >= 2 && shown.matches("<ul>").count() >= 3);
    }
}
