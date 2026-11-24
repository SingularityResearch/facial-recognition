//! The labeling side's data as rows: photos, faces, labels and Web search answers, each with a digest of the
//! row as it is now. Comparing the digests with those of the last sync
//! says which rows to send.

use std::collections::HashMap;

use face_library::{labels::LabelDb, store::Store};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::{
    digest::{Part, of},
    rows::{FaceRow, Picture, Row, Rows},
};

/// The tables of this part, in the order their rows are applied.
pub const TABLES: [&str; 4] = ["photos", "faces", "labels", "web_suggestions"];

/// {row's key: digest} for each table, the rows in the library's own order.
pub type Digests = HashMap<&'static str, IndexMap<String, String>>;

/// What a photo says about itself, as the other devices get it: when it was taken, its people tags, and
/// which of its faces a tag sits on, by the faces' permanent IDs. None for a photo not read on this device.
pub(crate) fn facts(db: &LabelDb, hash: &str) -> Option<Value> {
    let f = db.facts.facts.get(hash).filter(|f| f.key.is_some())?;
    let taken = match f.t {
        // (a time kept as a whole number stays one: it is written as it was read)
        Some(t) if f.t_whole => json!(t as i64),
        Some(t) => json!(t),
        None => Value::Null,
    };
    let faces: Map<String, Value> = f.faces.iter().map(|(i, name)| (db.store.fid[*i].clone(), json!(name))).collect();
    Some(json!({"t": taken, "tags": f.tags, "faces": faces}))
}

/// A photo's digest: its record without what belongs to single faces (which possible faces, which frames),
/// whether possible faces were looked for in it, and its facts.
fn photo_digest(db: &LabelDb, hash: &str, record: &Value) -> String {
    let shared: Map<String, Value> = (record.as_object().into_iter().flatten())
        .filter(|(k, _)| k.as_str() != "maybe" && k.as_str() != "at")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let looked = record.get("maybe").is_some();
    of(&[
        Part::Json(Value::Object(shared)),
        Part::Json(json!(looked)),
        facts(db, hash).map_or(Part::Nothing, Part::Json),
    ])
}

/// A face's digest: where it is, its quality, its print and picture (nothing of either for a face whose
/// print was deleted), its score as a possible face and its frame in a video.
fn face_digest(s: &Store, i: usize) -> String {
    of(&face_parts(s, i))
}

/// What a face's digest is made of, in order.
pub fn face_parts(s: &Store, i: usize) -> Vec<Part<'_>> {
    let (fid, dim) = (&s.fid[i], s.dim);
    let record = s.photos.get(&s.photo[i]);
    let of_face = |what: &str| record.and_then(|r| r.get(what)).and_then(|m| m.get(fid));
    let gone = s.printless(i);
    vec![
        Part::text(&s.photo[i]),
        Part::text(&s.file[i]),
        Part::Floats(&s.boxes[i]),
        Part::real(Some(s.quality[i] as f64)),
        Part::text(&s.model),
        if gone { Part::Nothing } else { Part::Floats(&s.prints()[i * dim..(i + 1) * dim]) },
        if gone || s.thumbs[i].is_empty() { Part::Nothing } else { Part::Bytes(&s.thumbs[i]) },
        Part::real(of_face("maybe").and_then(Value::as_f64)),
        of_face("at").filter(|v| !v.is_null()).map_or(Part::Nothing, |v| Part::Json(v.clone())),
    ]
}

/// Every face's digest, worked out on every core: a face's is over its print and picture, 7 KB or so, and
/// this is done after every save.
fn face_digests(s: &Store) -> IndexMap<String, String> {
    let n = s.len();
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get()).min(n.max(1));
    let share = n.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let work: Vec<_> = (0..n)
            .step_by(share)
            .map(|from| {
                scope.spawn(move || {
                    (from..(from + share).min(n)).map(|i| (s.fid[i].clone(), face_digest(s, i))).collect::<Vec<_>>()
                })
            })
            .collect();
        work.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    })
}

/// The digests of some rows of one table as they are now: a key that isn't here has none.
pub fn digests_of(db: &LabelDb, web: &Map<String, Value>, tbl: &str, keys: &[String]) -> HashMap<String, String> {
    let one = |k: &String| -> Option<String> {
        match tbl {
            "labels" => db.labels.get(k).map(|v| of(&[Part::Json(v.clone())])),
            "web_suggestions" => web.get(k).map(|v| of(&[Part::Json(v.clone())])),
            "photos" => db.store.photos.get(k).map(|r| photo_digest(db, k, r)),
            _ => db.store.index_of(k).map(|i| face_digest(&db.store, i)),
        }
    };
    keys.iter().filter_map(|k| Some((k.clone(), one(k)?))).collect()
}

/// A stored time as UTC text ("2026-11-12T10:20:30Z"): one with "Z" or an offset is moved to UTC, an older
/// one without is read as this computer's local time. Anything that isn't a time stays as it is.
pub fn utc_text(text: &str) -> String {
    utc_time(text).unwrap_or_else(|| text.to_string())
}

/// A stored time as UTC text, or none when it isn't a time.
pub fn utc_time(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let digits = |from: usize, n: usize| -> Option<i64> {
        let part = text.get(from..from + n)?;
        part.bytes().all(|c| c.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let (mut hour, mut minute, mut second, mut at) = (0, 0, 0, 10);
    if b.len() > 10 {
        if !matches!(b[10], b'T' | b' ') {
            return None;
        }
        let (h, m) = (digits(11, 2)?, digits(14, 2)?);
        (hour, minute, at) = (h, m, 16);
        if b.get(16) == Some(&b':') {
            let s = digits(17, 2)?;
            (second, at) = (s, 19);
            if b.get(19) == Some(&b'.') {
                at = 20 + b[20..].iter().take_while(|c| c.is_ascii_digit()).count();
            }
        }
    }
    let zone = &text[at..];
    // seconds since 1970 of that date and time as if it were UTC
    let days = {
        let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * m + 2) / 5 + day - 1 - 719_468
    };
    let as_utc = days * 86_400 + hour * 3600 + minute * 60 + second;
    let seconds = match zone {
        "Z" | "+00:00" => as_utc,
        "" => {
            // written in local time, before times were kept in UTC
            let local = format!("{year:04}:{month:02}:{day:02} {hour:02}:{minute:02}:{second:02}");
            match face_library::photofiles::camera_time(&local) {
                Ok(Some(t)) => t as i64,
                _ => return None,
            }
        }
        z => {
            let sign = match z.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let z = z[1..].replace(':', "");
            let (Some(h), Some(m)) =
                (z.get(0..2).and_then(|v| v.parse::<i64>().ok()), z.get(2..4).and_then(|v| v.parse::<i64>().ok()))
            else {
                return None;
            };
            as_utc - sign * (h * 3600 + m * 60)
        }
    };
    let seconds = u64::try_from(seconds).ok()?;
    Some(face_files::paths::utc_text(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds)))
}

/// The rows to send for these keys of one table, as they are now (a key that isn't here is left out).
pub fn rows(db: &LabelDb, web: &Map<String, Value>, tbl: &str, keys: &[String]) -> Rows {
    let s = &db.store;
    let one = |k: &String| -> Option<Row> {
        match tbl {
            "labels" => db.labels.get(k).map(|v| Row::Plain(v.clone())),
            // (a Web search answer's time goes out as UTC text, whatever an older version kept)
            "web_suggestions" => web.get(k).map(|v| {
                let mut v = v.clone();
                if let Some(when) = v.get("when").and_then(Value::as_str).map(utc_text) {
                    v["when"] = json!(when);
                }
                Row::Plain(v)
            }),
            "photos" => s.photos.get(k).map(|r| {
                let mut r = r.clone();
                let key = db.facts.facts.get(k).and_then(|f| f.key.clone());
                r["_facts"] = facts(db, k).unwrap_or(Value::Null);
                r["_key"] = json!(key);
                Row::Plain(r)
            }),
            _ => s.index_of(k).map(|i| {
                let record = s.photos.get(&s.photo[i]);
                let of_face = |what: &str| record.and_then(|r| r.get(what)).and_then(|m| m.get(k));
                let gone = s.printless(i);
                let real = |v: Option<f64>| v.map(|v| v as f32).filter(|v| !v.is_nan());
                Row::Face(Box::new(FaceRow {
                    photo: s.photo[i].clone(),
                    file: s.file[i].clone(),
                    place: s.boxes[i].to_vec(),
                    quality: real(Some(s.quality[i] as f64)),
                    model: s.model.clone(),
                    print: (!gone).then(|| s.print(i).to_vec()),
                    picture: if gone || s.thumbs[i].is_empty() {
                        Picture::None
                    } else {
                        Picture::Jpeg(s.thumbs[i].clone())
                    },
                    possible: real(of_face("maybe").and_then(Value::as_f64)),
                    frame: of_face("at").and_then(Value::as_i64).map(|v| v as i32),
                }))
            }),
        }
    };
    keys.iter().filter_map(|k| Some((k.clone(), one(k)?))).collect()
}

/// The digest of every row there is now. `web`: the Web search answers, by face.
pub fn digests(db: &LabelDb, web: &Map<String, Value>) -> Digests {
    let plain =
        |rows: &Map<String, Value>| rows.iter().map(|(k, v)| (k.clone(), of(&[Part::Json(v.clone())]))).collect();
    HashMap::from([
        ("labels", plain(&db.labels)),
        ("web_suggestions", plain(web)),
        ("photos", db.store.photos.iter().map(|(h, r)| (h.clone(), photo_digest(db, h, r))).collect()),
        ("faces", face_digests(&db.store)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_time_goes_out_as_utc_text() {
        assert_eq!(utc_text("2026-11-12T10:20:30Z"), "2026-11-12T10:20:30Z");
        assert_eq!(utc_text("2026-11-12T10:20:30.123456+00:00"), "2026-11-12T10:20:30Z");
        assert_eq!(utc_text("2026-11-12T10:20:30+02:00"), "2026-11-12T08:20:30Z");
        assert_eq!(utc_text("2026-01-01T00:30:00-05:30"), "2026-01-01T06:00:00Z");
        assert_eq!(utc_text("2024-02-29 23:59Z"), "2024-02-29T23:59:00Z");
        // an older one without a zone is this computer's local time: whatever that is, it comes out as UTC text
        let local = utc_text("2026-06-01 12:00:00");
        assert!(local.len() == 20 && local.ends_with('Z') && local.starts_with("2026-0"), "{local}");
        // what isn't a time stays as it is
        for text in ["", "yesterday", "2026-13", "2026/11/12 10:20", "2026-11-12T10:20:30 UTC"] {
            assert_eq!(utc_text(text), text);
        }
    }
}
