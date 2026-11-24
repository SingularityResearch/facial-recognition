//! Keeping other websites out of the programs' local servers: the request checks and page headers every
//! server shares.
//!
//! The servers answer on 127.0.0.1. Without a check, any website open in a browser could send them requests,
//! or read their answers through DNS rebinding. Every request must therefore:
//! - name this computer in its Host header (127.0.0.1 or localhost, with this server's port);
//! - not come from another website's page (the browser's `Sec-Fetch-Site: cross-site`);
//! - for data and anything a GET starts (`OWN_ONLY`): come from the tool's own page (`same-origin`), or carry
//!   the header a program that isn't a browser sends;
//! - for anything that changes or computes something (POST): carry `X-Face-Tool: 1` and, if the browser
//!   sends an Origin, come from this server's own page.

use std::collections::HashMap;

pub const HEADER: &str = "x-face-tool";
/// GET paths only the tool's own page may ask for
pub const OWN_ONLY: [&str; 3] = ["/api/", "/thumb/", "/photo/"];

/// What a page may load: only from its own server, with exactly the exceptions the pages use. No inline or
/// eval'd script, so a name or text that slipped past escaping still couldn't run.
const POLICY: [(&str, &str); 8] = [
    ("default-src", "'self'"),
    ("script-src", "'self'"),
    ("style-src", "'self' 'unsafe-inline'"),
    ("img-src", "'self' blob:"),
    ("connect-src", "'self'"),
    ("object-src", "'none'"),
    ("base-uri", "'none'"),
    ("form-action", "'self'"),
];

/// Pages that may show this process's pages in a frame: the app's tab page, once the app has started the
/// program (nobody before: a program started on its own trusts no page but its own).
static FRAME_PARENTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Let the app's tab page on this port frame this process's pages.
pub fn allow_frame_parents(shell_port: u16) {
    let mut parents = FRAME_PARENTS.lock().unwrap_or_else(|e| e.into_inner());
    for origin in [format!("http://127.0.0.1:{shell_port}"), format!("http://localhost:{shell_port}")] {
        if !parents.contains(&origin) {
            parents.push(origin);
        }
    }
}

/// The pages that may frame this process's pages.
pub fn frame_parents() -> Vec<String> {
    FRAME_PARENTS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The shared rules with a server's own, which replace the shared ones of the same name.
fn rules(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut rules: Vec<(String, String)> = POLICY.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    for (k, v) in extra {
        match rules.iter_mut().find(|(name, _)| name == k) {
            Some(rule) => rule.1 = v.to_string(),
            None => rules.push((k.to_string(), v.to_string())),
        }
    }
    rules
}

fn written(rules: &[(String, String)]) -> String {
    rules.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join("; ")
}

/// The Content-Security-Policy value: the shared rules with a server's own (which replace the shared ones
/// of the same name), then who may frame its pages (itself and `frame_parents`).
pub fn policy(extra: &[(&str, &str)], frame_parents: &[String]) -> String {
    let mut rules = rules(extra);
    let mut parents: Vec<&str> = frame_parents.iter().map(String::as_str).collect();
    parents.sort_unstable();
    parents.insert(0, "'self'");
    rules.push(("frame-ancestors".to_string(), parents.join(" ")));
    written(&rules)
}

/// The same for a page that no page may frame: the app's own tab page.
pub fn policy_never_framed(extra: &[(&str, &str)]) -> String {
    let mut rules = rules(extra);
    rules.push(("frame-ancestors".to_string(), "'none'".to_string()));
    written(&rules)
}

/// Why a request must be refused, or None when it may go on. `headers` by lower-case name. `write`: the
/// request changes or starts something (a POST). `own`: a GET for data, or one that starts work.
pub fn refuse_reason(headers: &HashMap<String, String>, port: u16, write: bool, own: bool) -> Option<String> {
    let local = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let get = |name: &str| headers.get(name).map(String::as_str);
    if !local.iter().any(|h| Some(h.as_str()) == get("host")) {
        return Some("This tool only answers requests addressed to 127.0.0.1.".to_string());
    }
    let site = get("sec-fetch-site");
    if site == Some("cross-site") {
        return Some("Requests from other websites are not allowed.".to_string());
    }
    let tool = get(HEADER) == Some("1");
    if own && !matches!(site, None | Some("same-origin")) && !tool {
        return Some("Only this tool's own page may ask for that.".to_string());
    }
    if write {
        if !tool {
            return Some("Missing the X-Face-Tool header; requests must come from the tool's own page.".to_string());
        }
        if let Some(origin) = get("origin") {
            if !local.iter().any(|h| origin == format!("http://{h}")) {
                return Some("Requests from other websites are not allowed.".to_string());
            }
        }
    }
    None
}

/// The request body's length from Content-Length, checked: a number, not negative, below `limit` and not 0.
/// Err((status, why)) otherwise.
pub fn body_size(content_length: Option<&str>, limit: usize) -> Result<usize, (u16, String)> {
    let raw = content_length.unwrap_or("0").trim();
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err((400, "bad Content-Length".to_string()));
    }
    let n: usize = raw.parse().map_err(|_| (413, format!("request too large (limit {} bytes)", grouped(limit))))?;
    if n >= limit {
        return Err((413, format!("request too large (limit {} bytes)", grouped(limit))));
    }
    if n == 0 {
        return Err((400, "empty request".to_string()));
    }
    Ok(n)
}

/// 20000000 as 20,000,000.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn only_requests_addressed_to_this_computer_and_port() {
        assert!(refuse_reason(&headers(&[("host", "127.0.0.1:8766")]), 8766, false, false).is_none());
        assert!(refuse_reason(&headers(&[("host", "localhost:8766")]), 8766, false, false).is_none());
        for host in ["127.0.0.1:8765", "example.com", "127.0.0.1", "192.0.2.5:8766"] {
            assert!(refuse_reason(&headers(&[("host", host)]), 8766, false, false).is_some(), "{host}");
        }
        assert!(refuse_reason(&headers(&[]), 8766, false, false).is_some());
    }

    #[test]
    fn another_websites_page_gets_nothing() {
        let h = headers(&[("host", "127.0.0.1:8766"), ("sec-fetch-site", "cross-site")]);
        assert!(refuse_reason(&h, 8766, false, false).is_some());
    }

    #[test]
    fn data_only_for_the_tools_own_page() {
        let with = |site: &str| headers(&[("host", "127.0.0.1:8766"), ("sec-fetch-site", site)]);
        assert!(refuse_reason(&with("same-origin"), 8766, false, true).is_none());
        assert!(refuse_reason(&with("same-site"), 8766, false, true).is_some()); // another program's page
        assert!(refuse_reason(&with("none"), 8766, false, true).is_some()); // an address typed or opened
        assert!(refuse_reason(&with("same-site"), 8766, false, false).is_none()); // the page files themselves
        // a program that isn't a browser sends the header instead
        let tool = headers(&[("host", "127.0.0.1:8766"), ("sec-fetch-site", "none"), (HEADER, "1")]);
        assert!(refuse_reason(&tool, 8766, false, true).is_none());
        assert!(refuse_reason(&headers(&[("host", "127.0.0.1:8766")]), 8766, false, true).is_none());
    }

    #[test]
    fn a_change_needs_the_header_and_its_own_origin() {
        let base = [("host", "127.0.0.1:8766")];
        assert!(refuse_reason(&headers(&base), 8766, true, false).is_some());
        let ok = headers(&[("host", "127.0.0.1:8766"), (HEADER, "1"), ("origin", "http://127.0.0.1:8766")]);
        assert!(refuse_reason(&ok, 8766, true, false).is_none());
        for origin in ["http://127.0.0.1:8765", "https://example.com", "null"] {
            let h = headers(&[("host", "127.0.0.1:8766"), (HEADER, "1"), ("origin", origin)]);
            assert!(refuse_reason(&h, 8766, true, false).is_some(), "{origin}");
        }
    }

    #[test]
    fn the_policy_is_the_programs() {
        let p = policy(&[("script-src", "'self' blob:"), ("media-src", "'self' blob:")], &[]);
        assert_eq!(
            p,
            "default-src 'self'; script-src 'self' blob:; style-src 'self' 'unsafe-inline'; img-src 'self' blob:; \
             connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; media-src 'self' blob:; \
             frame-ancestors 'self'"
        );
        assert!(
            policy(&[], &["http://127.0.0.1:8764".to_string()])
                .ends_with("frame-ancestors 'self' http://127.0.0.1:8764")
        );
    }

    #[test]
    fn a_bodys_length_is_checked() {
        assert_eq!(body_size(Some("12"), 100), Ok(12));
        assert_eq!(body_size(Some("0"), 100).unwrap_err().0, 400);
        assert_eq!(body_size(None, 100).unwrap_err().0, 400);
        assert_eq!(body_size(Some("-1"), 100).unwrap_err().0, 400);
        assert_eq!(body_size(Some("+5"), 100).unwrap_err().0, 400);
        assert_eq!(body_size(Some("100"), 100).unwrap_err(), (413, "request too large (limit 100 bytes)".to_string()));
        assert_eq!(
            body_size(Some("99999999999999999999999"), 20_000_000).unwrap_err().1,
            "request too large (limit 20,000,000 bytes)"
        );
    }
}
