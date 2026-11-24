//! A row's digest: what the sync remembers of each row as it was last sent or taken, to know which rows
//! changed since. Made as every device has always made them, so that a
//! device's memory of what it has synced stays good from one version to the next: a digest that came out
//! differently would have that device send its whole library again, and every other device take it again.
//!
//! A digest is the SHA-1 over its parts in order, each marked with its kind: nothing; bytes; a list of
//! numbers as 32-bit floats; anything else as JSON with its names in order, in the files' one-line
//! spelling.

use face_files::filejson;
use ring::digest::{Context, SHA1_FOR_LEGACY_USE_ONLY};
use serde_json::{Map, Value};

/// One part of a row.
pub enum Part<'a> {
    Nothing,
    Bytes(&'a [u8]),
    Floats(&'a [f32]),
    Json(Value),
}

impl Part<'_> {
    pub fn text(s: &str) -> Part<'static> {
        Part::Json(Value::String(s.to_string()))
    }

    /// A number kept as a 32-bit float (a quality, a detector's score), or nothing where it isn't a number:
    /// the files keep these as 32-bit floats and the database as `real`, and read back from either it must
    /// come out the same.
    pub fn real(v: Option<f64>) -> Part<'static> {
        match v.map(|v| v as f32).filter(|v| !v.is_nan()) {
            Some(v) => Part::Json(serde_json::Number::from_f64(v as f64).map_or(Value::Null, Value::Number)),
            None => Part::Nothing,
        }
    }
}

/// `v` with the names of every object in it in order.
pub fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(items) => {
            let mut names: Vec<&String> = items.keys().collect();
            names.sort();
            Value::Object(names.into_iter().map(|k| (k.clone(), sorted(&items[k]))).collect::<Map<_, _>>())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// The digest of a row made of these parts.
pub fn of(parts: &[Part]) -> String {
    let mut sum = Context::new(&SHA1_FOR_LEGACY_USE_ONLY);
    for part in parts {
        match part {
            Part::Nothing => sum.update(b"\0N"),
            Part::Bytes(b) => {
                sum.update(b"\0B");
                sum.update(b);
            }
            Part::Floats(values) => {
                sum.update(b"\0A");
                for v in *values {
                    sum.update(&v.to_le_bytes());
                }
            }
            Part::Json(v) => {
                sum.update(b"\0S");
                sum.update(filejson::dumps(&sorted(v)).as_bytes());
            }
        }
    }
    sum.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_rows_digest_is_what_the_devices_have_kept_so_far() {
        // each answer is fixed: it is what the devices have kept for these parts
        assert_eq!(of(&[]), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(of(&[Part::Nothing]), "4e95cbe0df6bc8c85e9def17db8cff854861caeb");
        let label = json!({"n": "Zoë", "by": "me", "s": 0.812, "ok": true, "not": ["A", "B"]});
        assert_eq!(of(&[Part::Json(label)]), "c559fde58881fe4d5214db29a03c3f3b0505694c");
        let parts = [
            Part::text("abc"),
            Part::Floats(&[1.5, -2.0, 0.1]),
            Part::real(Some(0.8124)),
            Part::real(Some(f64::NAN)),
            Part::real(None),
            Part::Bytes(b"\xff\xd8pic"),
            Part::Json(json!(12)),
            Part::Json(json!(true)),
        ];
        assert_eq!(of(&parts), "6bba52c5dad768291cc81695d5799eb4338e6840");
        // names in order, whatever order they were kept in, at every depth
        assert_eq!(
            of(&[Part::Json(json!({"b": {"z": 1, "a": 2}, "a": 1.0}))]),
            of(&[Part::Json(json!({"a": 1.0, "b": {"a": 2, "z": 1}}))])
        );
    }
}
