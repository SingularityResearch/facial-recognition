//! The privacy policy inside the app (`/privacy` on the tracker's server): `PRIVACY.md` as a page, carried in
//! the program, so that it can be read where the app runs and without the internet (the stores want the
//! policy inside the app, not only at an address). The file in the repository stays the one text: this only
//! turns the little Markdown it is written in into a page.
//!
//! The page leads nowhere outside the app: a link to another part of the policy stays a link, a link to
//! anything else becomes its words alone, and the policy's own address is written out at the end as text.

/// The policy as it was when the program was built.
const POLICY: &str = include_str!("../../../PRIVACY.md");
/// Where the same text is on the internet.
const ONLINE: &str = "https://github.com/SingularityResearch/facial-recognition/blob/main/PRIVACY.md";

const STYLE: &str = "html { color-scheme: dark light; } \
body { margin: 0; background: #14161a; color: #e6e8ec; font: 15px/1.55 system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif; } \
main { max-width: 760px; margin: 0 auto; padding: 16px 18px 40px; } \
h1 { font-size: 24px; margin: 8px 0 12px; } h2 { font-size: 18px; margin: 28px 0 8px; } h3 { font-size: 15px; margin: 20px 0 6px; } \
p, ul { margin: 8px 0; } ul { padding-left: 22px; } li { margin: 4px 0; } a { color: #7fb4ff; } \
code { font: 13px ui-monospace, Consolas, monospace; background: #22262d; padding: 1px 4px; border-radius: 3px; } \
.table { overflow-x: auto; margin: 10px 0; } table { border-collapse: collapse; width: 100%; font-size: 14px; } \
th, td { text-align: left; vertical-align: top; padding: 6px 8px; border: 1px solid #333944; } th { background: #1c2026; } \
.where { color: #9aa3af; font-size: 13px; margin-top: 28px; overflow-wrap: anywhere; } \
@media (prefers-color-scheme: light) { body { background: #fff; color: #1b1e23; } code { background: #eef0f3; } \
th, td { border-color: #d5d9e0; } th { background: #f4f5f7; } a { color: #1a5fd0; } .where { color: #5c6470; } }";

/// The policy as a page.
pub fn page() -> String {
    render(POLICY)
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A heading's name as a link inside the page names it: small letters and digits, a hyphen for each space.
fn anchor(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|c| match c {
            c if c.is_alphanumeric() => Some(c.to_ascii_lowercase()),
            ' ' | '-' => Some('-'),
            _ => None,
        })
        .collect()
}

/// A line's text: `**strong**`, `` `code` `` and `[words](where)`. Everything else is written as it is,
/// escaped. A link stays one only to another part of this page.
fn inline(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            out.push_str(&format!("<strong>{}</strong>", inline(&after[..end])));
            rest = &after[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            out.push_str(&format!("<code>{}</code>", esc(&after[..end])));
            rest = &after[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && let Some(end) = after[close + 2..].find(')')
        {
            let (words, target) = (&after[..close], &after[close + 2..close + 2 + end]);
            match target.strip_prefix('#') {
                Some(part) => out.push_str(&format!("<a href=\"#{}\">{}</a>", esc(part), inline(words))),
                None => out.push_str(&inline(words)),
            }
            rest = &after[close + 2 + end + 1..];
            continue;
        }
        out.push_str(&esc(&rest[..c.len_utf8()]));
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A table row's cells: `| a | b |`.
fn cells(line: &str) -> Vec<&str> {
    line.trim().trim_start_matches('|').trim_end_matches('|').split('|').map(str::trim).collect()
}

/// What is being gathered from the lines read so far.
enum Open {
    Nothing,
    Paragraph(String),
    /// the items so far; a line that goes on belongs to the last
    List(Vec<String>),
    /// the rows so far; the first is the head
    Table(Vec<Vec<String>>),
}

fn close(open: &mut Open, out: &mut String) {
    match std::mem::replace(open, Open::Nothing) {
        Open::Nothing => {}
        Open::Paragraph(text) => out.push_str(&format!("<p>{}</p>\n", inline(&text))),
        Open::List(items) => {
            out.push_str("<ul>\n");
            for item in items {
                out.push_str(&format!("<li>{}</li>\n", inline(&item)));
            }
            out.push_str("</ul>\n");
        }
        Open::Table(rows) => {
            out.push_str("<div class=\"table\"><table>\n");
            for (i, row) in rows.iter().enumerate() {
                let cell = if i == 0 { "th" } else { "td" };
                out.push_str("<tr>");
                for text in row {
                    out.push_str(&format!("<{cell}>{}</{cell}>", inline(text)));
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</table></div>\n");
        }
    }
}

/// The Markdown the policy is written in, as a page: headings, paragraphs, lists (an item may go on in lines
/// set in by two spaces), tables, and what `inline` knows. Nothing else is Markdown to it.
fn render(markdown: &str) -> String {
    let mut body = String::new();
    let mut open = Open::Nothing;
    for line in markdown.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            close(&mut open, &mut body);
            continue;
        }
        let heading = [("### ", 3), ("## ", 2), ("# ", 1)]
            .into_iter()
            .find_map(|(mark, level)| Some((line.strip_prefix(mark)?, level)));
        if let Some((text, level)) = heading {
            close(&mut open, &mut body);
            body.push_str(&format!("<h{level} id=\"{}\">{}</h{level}>\n", esc(&anchor(text)), inline(text)));
            continue;
        }
        if line.starts_with('|') {
            // the line of dashes under the head is only Markdown's way to say "a table"
            let rule = cells(line).iter().all(|c| !c.is_empty() && c.chars().all(|c| c == '-' || c == ':'));
            let row: Vec<String> = cells(line).into_iter().map(str::to_string).collect();
            match &mut open {
                Open::Table(rows) if rule => debug_assert_eq!(rows.len(), 1),
                Open::Table(rows) => rows.push(row),
                _ => {
                    close(&mut open, &mut body);
                    open = Open::Table(vec![row]);
                }
            }
            continue;
        }
        if let Some(item) = line.strip_prefix("- ") {
            match &mut open {
                Open::List(items) => items.push(item.to_string()),
                _ => {
                    close(&mut open, &mut body);
                    open = Open::List(vec![item.to_string()]);
                }
            }
            continue;
        }
        match &mut open {
            Open::List(items) if line.starts_with("  ") => {
                let last = items.last_mut().expect("a list has an item");
                last.push(' ');
                last.push_str(line.trim_start());
            }
            Open::Paragraph(text) => {
                text.push(' ');
                text.push_str(line.trim_start());
            }
            _ => {
                close(&mut open, &mut body);
                open = Open::Paragraph(line.trim_start().to_string());
            }
        }
    }
    close(&mut open, &mut body);
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>Privacy policy</title><style>{STYLE}</style></head>\n<body><main>\n{body}\
<p class=\"where\">This policy on the internet: {}</p>\n</main></body></html>\n",
        esc(ONLINE)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_markdown_the_policy_uses_becomes_a_page() {
        let page = render(
            "# A <title>\n\nOne line\nand its second, **strong** with `a<b` & [there](#the-part) and [out](../x.md).\n\n\
## The part, again\n\n- first\n  goes on\n- **second**: more\n\n| What | Where |\n|---|---|\n| a | `b` |\n| c | d |\n\nLast.\n",
        );
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<h1 id=\"a-title\">A &lt;title&gt;</h1>"));
        assert!(page.contains(
            "<p>One line and its second, <strong>strong</strong> with <code>a&lt;b</code> &amp; \
<a href=\"#the-part\">there</a> and out.</p>"
        ));
        assert!(page.contains("<h2 id=\"the-part-again\">The part, again</h2>"));
        assert!(page.contains("<ul>\n<li>first goes on</li>\n<li><strong>second</strong>: more</li>\n</ul>"));
        assert!(page.contains("<tr><th>What</th><th>Where</th></tr>\n<tr><td>a</td><td><code>b</code></td></tr>\n<tr><td>c</td><td>d</td></tr>"));
        assert!(page.contains("<p>Last.</p>"));
        // Windows line ends are the same text
        assert_eq!(render("# A\r\n\r\ntext\r\n"), render("# A\n\ntext\n"));
    }

    #[test]
    fn the_policy_is_a_whole_page_that_leads_nowhere_outside_the_app() {
        let page = page();
        assert!(page.contains("<h1 id=\"privacy-policy\">Privacy policy</h1>"));
        assert!(page.contains("not a medical device"));
        assert!(page.contains(ONLINE));
        // nothing runs and nothing is loaded
        for no in ["<script", " src=", "<img", "<link", "<iframe", "javascript:", "onclick"] {
            assert!(!page.contains(no), "{no}");
        }
        // every link is to a part of this page that is there
        let mut links = 0;
        for link in page.split("href=\"").skip(1) {
            let target = &link[..link.find('"').unwrap()];
            let part = target.strip_prefix('#').unwrap_or_else(|| panic!("a link that leaves the page: {target}"));
            assert!(page.contains(&format!(" id=\"{part}\"")), "no part {part}");
            links += 1;
        }
        assert!(links >= 1);
        // no Markdown is left over in what is shown
        let shown = &page[page.find("<main>").unwrap()..];
        for left in ["**", "](", "\n| ", "\n- ", "\n#"] {
            assert!(!shown.contains(left), "left over: {left:?}");
        }
        assert!(shown.matches("<table>").count() >= 2 && shown.matches("<ul>").count() >= 3);
    }
}
