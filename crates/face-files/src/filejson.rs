//! JSON in the one spelling the programs' files have: a line of a log, a row of a library, must be the same
//! bytes whichever device and whichever version wrote it, and sizes are measured by it.
//!
//! `dumps`: one line (", " and ": " between things, everything outside ASCII as `\uXXXX`).
//! `dumps_indent1`: a line a thing, indented by one space, text as it is: the files that are rewritten whole.

use std::fmt::Write as _;

use serde_json::Value;

/// A float as the files spell it: the shortest digits that give the number back, in exponent form
/// below 1e-4 and from 1e16 ("1e-05", "1.5e+16"), and never without a decimal point otherwise ("1.0").
///
/// Where the number lies exactly halfway between two spellings of that shortest length, the files have the
/// one whose last digit is even, and the language's own shortest spelling can be the other: 0.57013702392578125 (a
/// 32-bit float widened, as a face's quality is) is "0.5701370239257812" in a file. So the count of digits
/// is taken from the language's shortest spelling, and the digits themselves from rounding the number to that many,
/// which rounds a tie to the even digit; they are used when they still give the number back.
fn float(f: f64, out: &mut String) {
    if f == 0.0 || !f.is_finite() {
        out.push_str(if f == 0.0 && f.is_sign_negative() {
            "-0.0"
        } else if f == 0.0 {
            "0.0"
        } else {
            "NaN"
        });
        return;
    }
    let shortest = format!("{f:e}");
    let count = shortest.split('e').next().unwrap_or("").chars().filter(char::is_ascii_digit).count();
    let rounded = format!("{f:.*e}", count.saturating_sub(1));
    let sci = if rounded.parse::<f64>() == Ok(f) { rounded } else { shortest };
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    if negative {
        out.push('-');
    }
    if !(1e-4..1e16).contains(&f.abs()) {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(out, "e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    } else if exp < 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-exp - 1) as usize));
        out.push_str(digits);
    } else {
        let whole = exp as usize + 1;
        if digits.len() > whole {
            out.push_str(&digits[..whole]);
            out.push('.');
            out.push_str(&digits[whole..]);
        } else {
            out.push_str(digits);
            out.push_str(&"0".repeat(whole - digits.len()));
            out.push_str(".0");
        }
    }
}

/// A float as the files spell it (and so their JSON).
pub fn float_repr(f: f64) -> String {
    let mut out = String::new();
    float(f, &mut out);
    out
}

fn text(s: &str, ascii: bool, out: &mut String) {
    out.push('"');
    // what needs no change is copied a stretch at a time: the large files are mostly such text
    let plain = |b: u8| (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\';
    let mut rest = s;
    while !rest.is_empty() {
        let run = rest.bytes().position(|b| !plain(b)).unwrap_or(rest.len());
        out.push_str(&rest[..run]);
        rest = &rest[run..];
        let Some(c) = rest.chars().next() else { break };
        rest = &rest[c.len_utf8()..];
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            // with ASCII asked for, everything but space to "~" is written this way, the delete character too
            c if (c as u32) < 0x20 || (ascii && (c as u32) > 0x7e) => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number(n: &serde_json::Number, out: &mut String) {
    match (n.as_i64(), n.as_u64(), n.as_f64()) {
        (_, _, Some(f)) if n.is_f64() => float(f, out),
        (Some(i), _, _) => {
            let _ = write!(out, "{i}");
        }
        (_, Some(u), _) => {
            let _ = write!(out, "{u}");
        }
        _ => out.push_str(&n.to_string()),
    }
}

/// `indent`: None for one line; Some(n) for a line a thing, n spaces a level.
fn write(v: &Value, ascii: bool, indent: Option<usize>, depth: usize, out: &mut String) {
    let line = |out: &mut String, depth: usize| {
        if let Some(n) = indent {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', n * depth));
        }
    };
    let between = if indent.is_some() { "," } else { ", " };
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => number(n, out),
        Value::String(s) => text(s, ascii, out),
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Object(items) if items.is_empty() => out.push_str("{}"),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(between);
                }
                line(out, depth + 1);
                write(item, ascii, indent, depth + 1, out);
            }
            line(out, depth);
            out.push(']');
        }
        Value::Object(items) => {
            out.push('{');
            for (i, (key, item)) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(between);
                }
                line(out, depth + 1);
                text(key, ascii, out);
                out.push_str(": ");
                write(item, ascii, indent, depth + 1, out);
            }
            line(out, depth);
            out.push('}');
        }
    }
}

/// What a page can send as a name or a short text, as text: text as it is, nothing for
/// what counts as empty (null, false, 0, ""), a number or `true` as the files spell them. None for a list or an object,
/// which no page sends: every caller refuses those.
pub fn said(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Some(String::new()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Bool(true)) => Some("True".to_string()),
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => Some(String::new()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(_) => None,
    }
}

/// `v` rounded to `places` decimals: the nearest number with that many, and of two equally near the one
/// whose last digit is even.
pub fn round(v: f64, places: usize) -> f64 {
    format!("{v:.places$}").parse().unwrap_or(v)
}

/// `json.dumps(v, ensure_ascii=False)`
pub fn dumps_utf8(v: &Value) -> String {
    let mut out = String::new();
    write(v, false, None, 0, &mut out);
    out
}

/// A JSON object or list with one line for each of its entries, for
/// the large files that are rewritten whole (photos, labels, descriptions), which can then be read and
/// compared line by line. `ascii` as `json.dumps`' `ensure_ascii`.
pub fn rows_json(v: &Value, ascii: bool) -> String {
    let mut out = String::new();
    match v {
        Value::Object(items) => return rows_of(items, ascii),
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                write(item, ascii, None, 0, &mut out);
            }
            out.push_str("\n]");
        }
        other => write(other, ascii, None, 0, &mut out),
    }
    out
}

/// `rows_json` for an object as it is kept, without making a copy of it first.
pub fn rows_of(items: &serde_json::Map<String, Value>, ascii: bool) -> String {
    if items.is_empty() {
        return "{}".to_string();
    }
    let mut out = String::with_capacity(items.len() * 64);
    out.push_str("{\n");
    for (i, (key, item)) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        text(key, ascii, &mut out);
        out.push_str(": ");
        write(item, ascii, None, 0, &mut out);
    }
    out.push_str("\n}");
    out
}

/// `json.dumps(v)`
pub fn dumps(v: &Value) -> String {
    let mut out = String::new();
    write(v, true, None, 0, &mut out);
    out
}

/// `json.dumps(v, ensure_ascii=ascii, indent=indent)`: a line a thing, `indent` spaces a level (none at 0).
pub fn dumps_lines(v: &Value, ascii: bool, indent: usize) -> String {
    let mut out = String::new();
    write(v, ascii, Some(indent), 0, &mut out);
    out
}

/// `json.dumps(v, indent=1, ensure_ascii=False)`
pub fn dumps_indent1(v: &Value) -> String {
    let mut out = String::new();
    write(v, false, Some(1), 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_file_of_lines_is_written_in_the_files_fixed_spelling() {
        // (the expected text is fixed: files in use have it)
        let v = json!({"track": {"ft-a": "1", "ft-b": "caf\u{e9}"}, "label": {}, "n": [1, 2.5, []]});
        // the pages' settings and the person signals: a line a thing, no indent
        let plain = "{\n\"track\": {\n\"ft-a\": \"1\",\n\"ft-b\": \"caf\u{e9}\"\n},\n\"label\": {},\n\"n\": [\n1,\n2.5,\n[]\n]\n}";
        assert_eq!(dumps_lines(&v, false, 0), plain);
        // a fitted document: a space a level, and only ASCII
        let fitted = "{\n \"track\": {\n  \"ft-a\": \"1\",\n  \"ft-b\": \"caf\\u00e9\"\n },\n \"label\": {},\n \"n\": [\n  1,\n  2.5,\n  []\n ]\n}";
        assert_eq!(dumps_lines(&v, true, 1), fitted);
    }

    #[test]
    fn a_float_is_spelled_as_the_files_spell_it() {
        // each text is the fixed spelling of that number
        for (f, text) in [
            (0.1, "0.1"),
            (1.0, "1.0"),
            (100.0, "100.0"),
            (-2.5, "-2.5"),
            (0.812, "0.812"),
            (123456789.123, "123456789.123"),
            (1699999999.25, "1699999999.25"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.25e-7, "1.25e-07"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (0.0, "0.0"),
            // exactly halfway between two spellings of the shortest length: the even one (32-bit floats
            // widened, as a face's quality and a detector's score are kept)
            (0.5701370239257812, "0.5701370239257812"),
            (0.8123999834060669, "0.8123999834060669"),
            (0.30000001192092896, "0.30000001192092896"),
        ] {
            assert_eq!(float_repr(f), text);
        }
    }

    // each expected text is the fixed spelling of the same value

    #[test]
    fn one_line_in_the_fixed_spelling() {
        let v = json!({"name": "Zoë \"Z\"\n", "start": 1, "end": 2.0, "list": [1, 2.5, null, true], "none": {}, "empty": []});
        assert_eq!(
            dumps(&v),
            r#"{"name": "Zo\u00eb \"Z\"\n", "start": 1, "end": 2.0, "list": [1, 2.5, null, true], "none": {}, "empty": []}"#
        );
        assert_eq!(dumps(&json!("😀\u{1}\u{7f}")), r#""\ud83d\ude00\u0001\u007f""#);
    }

    #[test]
    fn a_line_a_thing_in_the_fixed_spelling() {
        let v = json!([{"id": "a", "name": "Zoë", "details": {"first": {"sim": 0.612}}, "none": []}, 5]);
        assert_eq!(
            dumps_indent1(&v),
            "[\n {\n  \"id\": \"a\",\n  \"name\": \"Zoë\",\n  \"details\": {\n   \"first\": {\n    \"sim\": 0.612\n   }\n  },\n  \"none\": []\n },\n 5\n]"
        );
        assert_eq!(dumps_indent1(&json!([])), "[]");
    }

    #[test]
    fn a_line_an_entry_as_the_large_files_are_written() {
        // the fixed text for these values
        let photos =
            json!({"ab12": {"path": "C:\\p\\a.jpg", "size": 5, "mtime": 1.5, "faces": 2}, "cd34": {"dup_of": "ab12"}});
        assert_eq!(
            rows_json(&photos, true),
            "{\n\"ab12\": {\"path\": \"C:\\\\p\\\\a.jpg\", \"size\": 5, \"mtime\": 1.5, \"faces\": 2},\n\"cd34\": {\"dup_of\": \"ab12\"}\n}"
        );
        assert_eq!(rows_json(&json!([1, {"a": [2, 3]}, "x"]), true), "[\n1,\n{\"a\": [2, 3]},\n\"x\"\n]");
        assert_eq!(
            (rows_json(&json!({}), true), rows_json(&json!([]), true), rows_json(&json!(7), true)),
            ("{}".to_string(), "[]".to_string(), "7".to_string())
        );
    }

    #[test]
    fn floats_in_the_fixed_spelling() {
        for (f, fixed) in [
            (1.0, "1.0"),
            (0.1, "0.1"),
            (-2.5, "-2.5"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1.5e-7, "1.5e-07"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (9_999_999_999_999_998.0, "9999999999999998.0"),
            (1e100, "1e+100"),
            (123456.789, "123456.789"),
            (0.0, "0.0"),
        ] {
            let mut said = String::new();
            float(f, &mut said);
            assert_eq!(said, fixed, "{f:e}");
        }
        assert_eq!(dumps(&json!([1, 1.0, 12345678901234567890u64, -3])), "[1, 1.0, 12345678901234567890, -3]");
    }
}
