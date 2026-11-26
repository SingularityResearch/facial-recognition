//! The little Markdown the project's own documents are written in, as a page carried in the program: the privacy
//! policy (`privacy.rs`, `/privacy` on the tracker's server) and the setup guide (`guide.rs`, `/guide` on the
//! labeling tool's). The files in the repository stay the one text: this only turns them into a page.
//!
//! What it knows: headings (`#` to `####`), paragraphs, lists (`- ` and `1. `, an item going on in the lines set in
//! under it, which may hold further lists and code), tables, code between two lines of three backticks, a rule
//! (`---`), and in a line `**strong**`, `` `code` `` and `[words](where)`. Nothing else is Markdown to it.
//!
//! The page leads nowhere outside the app: a link to another part of the same page stays a link, a link to anything
//! else becomes its words alone, and where the document is on the internet is written out at the end as text.
//! Nothing runs on it and it loads nothing.

const STYLE: &str = "html { color-scheme: dark light; } \
body { margin: 0; background: #14161a; color: #e6e8ec; font: 15px/1.55 system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif; } \
main { max-width: 760px; margin: 0 auto; padding: 16px 18px 40px; } \
h1 { font-size: 24px; margin: 8px 0 12px; } h2 { font-size: 18px; margin: 28px 0 8px; } h3 { font-size: 15px; margin: 20px 0 6px; } \
h4 { font-size: 14px; margin: 16px 0 4px; } \
p, ul, ol { margin: 8px 0; } ul, ol { padding-left: 22px; } li { margin: 4px 0; } a { color: #7fb4ff; } \
code { font: 13px ui-monospace, Consolas, monospace; background: #22262d; padding: 1px 4px; border-radius: 3px; } \
pre { background: #22262d; padding: 10px 12px; border-radius: 6px; overflow-x: auto; margin: 8px 0; } \
pre code { padding: 0; background: none; white-space: pre; } \
hr { border: 0; border-top: 1px solid #333944; margin: 24px 0; } \
.table { overflow-x: auto; margin: 10px 0; } table { border-collapse: collapse; width: 100%; font-size: 14px; } \
th, td { text-align: left; vertical-align: top; padding: 6px 8px; border: 1px solid #333944; } th { background: #1c2026; } \
.where { color: #9aa3af; font-size: 13px; margin-top: 28px; overflow-wrap: anywhere; } \
@media (prefers-color-scheme: light) { body { background: #fff; color: #1b1e23; } code, pre { background: #eef0f3; } \
th, td, hr { border-color: #d5d9e0; } th { background: #f4f5f7; } a { color: #1a5fd0; } .where { color: #5c6470; } }";

/// `markdown` as a whole page called `title`, with `note` (where it is on the internet) at its end.
pub fn page(markdown: &str, title: &str, note: &str) -> String {
    let lines: Vec<(usize, String)> = markdown.lines().map(|l| (indent(l), l.trim().to_string())).collect();
    let body = blocks(&lines, false);
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{}</title><style>{STYLE}</style></head>\n<body><main>\n{body}\
<p class=\"where\">{}</p>\n</main></body></html>\n",
        esc(title),
        esc(note)
    )
}

pub fn esc(text: &str) -> String {
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
pub fn anchor(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|c| match c {
            c if c.is_alphanumeric() => Some(c.to_ascii_lowercase()),
            ' ' | '-' => Some('-'),
            _ => None,
        })
        .collect()
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
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

/// A list item's mark: (ordered with its number, or not; how wide the mark is with the space after it).
fn item_mark(text: &str) -> Option<(Option<u32>, usize)> {
    if text.starts_with("- ") {
        return Some((None, 2));
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits < 4 && text[digits..].starts_with(". ") {
        return Some((text[..digits].parse().ok(), digits + 2));
    }
    None
}

fn is_fence(text: &str) -> bool {
    text.starts_with("```")
}

fn heading(text: &str) -> Option<(&str, usize)> {
    [("#### ", 4), ("### ", 3), ("## ", 2), ("# ", 1)]
        .into_iter()
        .find_map(|(mark, level)| Some((text.strip_prefix(mark)?, level)))
}

/// Whether a line begins a block of its own rather than going on with a paragraph.
fn begins_block(text: &str) -> bool {
    text.is_empty()
        || is_fence(text)
        || heading(text).is_some()
        || text == "---"
        || text.starts_with('|')
        || item_mark(text).is_some()
}

/// Lines (their indent, their text) as blocks. `tight`: the lines are a list item's, whose first paragraph is
/// written without a paragraph of its own.
fn blocks(lines: &[(usize, String)], tight: bool) -> String {
    let mut out = String::new();
    let mut first = true;
    let mut i = 0;
    while i < lines.len() {
        let (at, text) = (&lines[i].0, lines[i].1.as_str());
        if text.is_empty() {
            i += 1;
            continue;
        }
        if is_fence(text) {
            // what is between the two fences, as it is, set in as it is under the first
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !is_fence(&lines[i].1) {
                let (n, t) = (&lines[i].0, &lines[i].1);
                code.push(if t.is_empty() {
                    String::new()
                } else {
                    format!("{}{t}", " ".repeat(n.saturating_sub(*at)))
                });
                i += 1;
            }
            i += 1; // the closing fence
            out.push_str(&format!("<pre><code>{}</code></pre>\n", esc(&code.join("\n"))));
        } else if let Some((words, level)) = heading(text) {
            out.push_str(&format!("<h{level} id=\"{}\">{}</h{level}>\n", esc(&anchor(words)), inline(words)));
            i += 1;
        } else if text == "---" {
            out.push_str("<hr>\n");
            i += 1;
        } else if text.starts_with('|') {
            out.push_str("<div class=\"table\"><table>\n");
            let mut row = 0;
            while i < lines.len() && lines[i].1.starts_with('|') {
                let line = &lines[i].1;
                i += 1;
                // the line of dashes under the head is only Markdown's way to say "a table"
                if row == 1 && cells(line).iter().all(|c| !c.is_empty() && c.chars().all(|c| c == '-' || c == ':')) {
                    continue;
                }
                let cell = if row == 0 { "th" } else { "td" };
                out.push_str("<tr>");
                for text in cells(line) {
                    out.push_str(&format!("<{cell}>{}</{cell}>", inline(text)));
                }
                out.push_str("</tr>\n");
                row += 1;
            }
            out.push_str("</table></div>\n");
        } else if let Some((number, _)) = item_mark(text) {
            // one list: the items of its kind at this indent, each with the lines set in under it
            let (kind, base) = (number.is_some(), *at);
            out.push_str(&match number {
                Some(n) if n != 1 => format!("<ol start=\"{n}\">\n"),
                Some(_) => "<ol>\n".to_string(),
                None => "<ul>\n".to_string(),
            });
            loop {
                // empty lines between two items of the list
                let next = (i..lines.len()).find(|&k| !lines[k].1.is_empty());
                let Some(k) = next.filter(|&k| lines[k].0 == base) else { break };
                let Some((number, width)) = item_mark(&lines[k].1) else { break };
                i = k;
                if number.is_some() != kind {
                    break;
                }
                let inner = base + width;
                let mut item = vec![(0, lines[i].1[width..].to_string())];
                i += 1;
                while i < lines.len() {
                    let (n, t) = (lines[i].0, &lines[i].1);
                    // a line set in under the item is the item's; an empty one only when the item goes on after it
                    let goes_on = if t.is_empty() {
                        lines[i..].iter().find(|(_, t)| !t.is_empty()).is_some_and(|(n, _)| *n > base)
                    } else {
                        n > base
                    };
                    if !goes_on {
                        break;
                    }
                    item.push((n.saturating_sub(inner), t.clone()));
                    i += 1;
                }
                out.push_str(&format!("<li>{}</li>\n", blocks(&item, true).trim_end()));
            }
            out.push_str(if kind { "</ol>\n" } else { "</ul>\n" });
        } else {
            let mut text = text.to_string();
            i += 1;
            while i < lines.len() && !begins_block(&lines[i].1) {
                text.push(' ');
                text.push_str(&lines[i].1);
                i += 1;
            }
            if tight && first {
                out.push_str(&inline(&text));
                out.push('\n');
            } else {
                out.push_str(&format!("<p>{}</p>\n", inline(&text)));
            }
        }
        first = false;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A document's page: nothing runs, nothing is loaded, every link is to a part of the page that is there, and
    /// no Markdown is left over in what is shown (code shown as code apart). How many links it has.
    pub(crate) fn leads_nowhere(page: &str) -> usize {
        for no in ["<script", " src=", "<img", "<link", "<iframe", "javascript:", "onclick"] {
            assert!(!page.contains(no), "{no}");
        }
        let mut links = 0;
        for link in page.split("href=\"").skip(1) {
            let target = &link[..link.find('"').unwrap()];
            let part = target.strip_prefix('#').unwrap_or_else(|| panic!("a link that leaves the page: {target}"));
            assert!(page.contains(&format!(" id=\"{part}\"")), "no part {part}");
            links += 1;
        }
        let mut shown = page[page.find("<main>").unwrap()..].to_string();
        while let (Some(a), Some(b)) = (shown.find("<pre>"), shown.find("</pre>")) {
            shown.replace_range(a..b + "</pre>".len(), "");
        }
        for left in ["**", "](", "\n| ", "\n- ", "\n#", "```"] {
            assert!(!shown.contains(left), "left over: {left:?}");
        }
        links
    }

    #[test]
    fn the_markdown_the_documents_use_becomes_a_page() {
        let page = page(
            "# A <title>\n\nOne line\nand its second, **strong** with `a<b` & [there](#the-part) and [out](../x.md).\n\n\
## The part, again\n\n- first\n  goes on\n- **second**: more\n\n| What | Where |\n|---|---|\n| a | `b` |\n| c | d |\n\nLast.\n",
            "T",
            "here",
        );
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<title>T</title>"));
        assert!(page.contains("<h1 id=\"a-title\">A &lt;title&gt;</h1>"));
        assert!(page.contains(
            "<p>One line and its second, <strong>strong</strong> with <code>a&lt;b</code> &amp; \
<a href=\"#the-part\">there</a> and out.</p>"
        ));
        assert!(page.contains("<h2 id=\"the-part-again\">The part, again</h2>"));
        assert!(page.contains("<ul>\n<li>first goes on</li>\n<li><strong>second</strong>: more</li>\n</ul>"));
        assert!(page.contains("<tr><th>What</th><th>Where</th></tr>\n<tr><td>a</td><td><code>b</code></td></tr>\n<tr><td>c</td><td>d</td></tr>"));
        assert!(page.contains("<p>Last.</p>"));
        assert!(page.contains("<p class=\"where\">here</p>"));
        // Windows line ends are the same text
        assert_eq!(super::page("# A\r\n\r\ntext\r\n", "T", "n"), super::page("# A\n\ntext\n", "T", "n"));
    }

    #[test]
    fn steps_hold_lists_and_code_set_in_under_them() {
        let page = page(
            "#### Four\n\n1. Copy it:\n   - Linux: `a`\n     goes on\n   - Windows\n2. Then:\n   ```\n   run <this>\n     set in\n   ```\n   and after.\n\n\
3. Three\n\n---\n\n```\nplain\n```\n\n- outer\n  - inner:\n    ```\n    code\n    ```\n  - second\n",
            "T",
            "n",
        );
        assert!(page.contains("<h4 id=\"four\">Four</h4>"));
        assert!(
            page.contains(
                "<ol>\n<li>Copy it:\n<ul>\n<li>Linux: <code>a</code> goes on</li>\n<li>Windows</li>\n</ul></li>\n\
<li>Then:\n<pre><code>run &lt;this&gt;\n  set in</code></pre>\n<p>and after.</p></li>\n<li>Three</li>\n</ol>"
            ),
            "{page}"
        );
        assert!(page.contains("<hr>\n<pre><code>plain</code></pre>"));
        assert!(page.contains("<ul>\n<li>outer\n<ul>\n<li>inner:\n<pre><code>code</code></pre></li>\n<li>second</li>\n</ul></li>\n</ul>"), "{page}");
        // a numbered list that starts later says so
        assert!(
            super::page("4. four\n5. five\n", "T", "n")
                .contains("<ol start=\"4\">\n<li>four</li>\n<li>five</li>\n</ol>")
        );
    }
}
