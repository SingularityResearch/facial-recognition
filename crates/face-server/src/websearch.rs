//! The labeling tool's Web search (only when the owner clicks it): suggests who a face is.
//!
//! Two US services, English results only (the owner's rule), each optional and each capped under $10 a month:
//! Amazon Rekognition's celebrity recognition on the face (a well-known person's name with Amazon's match
//! confidence; it never names private people), and Google Cloud Vision's web detection on the whole photo
//! (whether this exact picture is on web pages, and the names those pages give; only English text and
//! US-style domains are kept). `suggest` combines them into suggestions with a confidence level, or none.
//!
//! The face goes to Amazon and the photo to Google; both may keep them. Nothing is sent but on the owner's
//! click, a search is counted before it is sent (a failed one may still be billed), and neither service is
//! followed to another address. The keys are the private folder's (`face_home::saved`).

use std::{io::Read, path::Path, sync::Mutex, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use face_core::image::Bgr;
use face_files::{
    filejson,
    paths::{atomic_write, utc_now},
};
use face_home::saved;
use ring::{digest, hmac};
use serde_json::{Map, Value, json};

use crate::{
    describejob::falsy,
    http::Response,
    label::{Answer, Labeler, asked},
};

const UA: &str = "FaceLabels/1.0 (local photo labeling tool)";
const TIMEOUT: Duration = Duration::from_secs(30);
/// bytes read from a service's answer
const MAX_ANSWER: u64 = 4_000_000;
/// Searches per calendar month, so each bill stays under the owner's $10 a month (2026-11-01 prices):
/// Google 1,000 free + 2,700 at $3.50 per 1,000 = $9.45; Amazon $1.00 per 1,000 images = $9.00.
pub const CAPS: [(&str, i64); 2] = [("vision", 3700), ("rekognition", 9000)];
const US_TLDS: [&str; 7] = ["com", "org", "net", "edu", "gov", "mil", "us"];
const LEVELS: [&str; 4] = ["very high", "high", "medium", "low"];

/// Where the two services are asked. A check or a test names stand-ins on this computer; nothing else can.
pub struct Places {
    /// Google Cloud Vision: scheme, host and port
    pub vision: String,
    /// Amazon Rekognition, when it isn't Amazon's own address for the key's region
    pub rekognition: Option<String>,
}

impl Places {
    /// The services' own addresses; `FACE_TEST_VISION` and `FACE_TEST_REKOGNITION` name stand-ins for a
    /// check, and count only when they are on this computer (http://127.0.0.1:port): the owner's keys, a
    /// face and a photo are never sent anywhere else on the environment's word.
    pub fn from_env() -> Places {
        let here = |name: &str| {
            let url = std::env::var(name).ok()?;
            let port = url.strip_prefix("http://127.0.0.1:")?;
            (!port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())).then_some(url)
        };
        Places {
            vision: here("FACE_TEST_VISION").unwrap_or_else(|| "https://vision.googleapis.com".to_string()),
            rekognition: here("FACE_TEST_REKOGNITION"),
        }
    }
}

/// One search counted with every device's, in the shared database: Ok(Some(used this month)), Ok(None) when
/// there is no database or it can't be reached (this device then counts alone), Err(why) past the cap.
pub type Shared<'a> = &'a dyn Fn(&str, &str, i64) -> Result<Option<i64>, String>;

fn cap(service: &str) -> i64 {
    CAPS.iter().find(|(s, _)| *s == service).map_or(0, |(_, cap)| *cap)
}

/// {month, used, cap} of a service for this calendar month (billing months are UTC). A count that is there
/// but can't be read is an error, never "none used yet".
pub fn usage(home: &Path, service: &str) -> Result<Value, String> {
    let month: String = utc_now().chars().take(7).collect();
    let file = home.join(format!("{service}_usage.json"));
    let kept: Value = match std::fs::read(&file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(e) => return Err(format!("can't read {service}_usage.json: {e}")),
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("can't read {service}_usage.json: {e}"))?,
    };
    let used = if kept.get("month").and_then(Value::as_str) == Some(&month) {
        kept.get("used").and_then(|u| u.as_i64().or_else(|| u.as_f64().map(|f| f as i64))).unwrap_or(0)
    } else {
        0
    };
    Ok(json!({"month": month, "used": used, "cap": cap(service)}))
}

static COUNTING: Mutex<()> = Mutex::new(());

/// Count a search before sending it (a failed call may still be billed); refused past the cap. Under a
/// lock, so two searches at once can't both take the last one.
fn count(home: &Path, service: &str, shared: Shared) -> Result<(), String> {
    let _one = COUNTING.lock().unwrap_or_else(|e| e.into_inner());
    let u = usage(home, service)?;
    let (month, used, cap) = (u["month"].as_str().unwrap_or(""), u["used"].as_i64().unwrap_or(0), cap(service));
    let keep = |used: i64| {
        std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
        let text = filejson::dumps(&json!({"month": month, "used": used}));
        atomic_write(&home.join(format!("{service}_usage.json")), text.as_bytes()).map_err(|e| e.to_string())
    };
    if let Some(everyone) = shared(service, month, cap)? {
        return keep(everyone.max(used + 1));
    }
    if used >= cap {
        return Err(format!("{service} limit reached: {cap} searches this month (keeps the bill under $10)"));
    }
    keep(used + 1)
}

/// Letters, digits, spaces and the punctuation English text has: nothing else.
pub fn english(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || c.is_whitespace()
                || ".,:;!?'\"&()/+#%@$*_-|".contains(c)
                || ['\u{2013}', '\u{2014}', '\u{2019}', '\u{201c}', '\u{201d}'].contains(&c)
        })
}

/// The host a web address names, in lower case ("" when it names none).
fn host(url: &str) -> String {
    let rest = url.split_once("//").map_or(url, |(_, rest)| rest);
    let place = rest.split(['/', '?', '#']).next().unwrap_or("");
    let place = place.rsplit_once('@').map_or(place, |(_, host)| host);
    let host = match place.strip_prefix('[') {
        Some(inside) => inside.split(']').next().unwrap_or(""),
        None => place.split(':').next().unwrap_or(""),
    };
    host.to_lowercase()
}

/// Whether an address is of a US-style domain (.com, .org, .net, .edu, .gov, .mil, .us).
pub fn us_site(url: &str) -> bool {
    US_TLDS.contains(&host(url).rsplit('.').next().unwrap_or(""))
}

/// A name as a person's is written: two to four words, each a capital and then small letters ("Javier
/// Bardem", "Mary J. Blige", "Jean-Luc Picard").
fn person(name: &str) -> bool {
    let word = |w: &str| {
        let mut letters = w.chars();
        letters.next().is_some_and(|c| c.is_ascii_uppercase())
            && w.len() > 1
            && letters.all(|c| c.is_ascii_lowercase() || c == '\'' || c == '.')
    };
    let words: Vec<&str> = name.split([' ', '-']).collect();
    (2..=4).contains(&words.len()) && words.iter().all(|w| word(w))
}

fn agent() -> ureq::Agent {
    let t = Some(TIMEOUT);
    let config =
        ureq::Agent::config_builder().max_redirects(0).max_redirects_will_error(false).http_status_as_error(false);
    let config = config.timeout_connect(t).timeout_send_request(t).timeout_send_body(t).timeout_recv_response(t);
    config.timeout_recv_body(t).build().into()
}

/// A service's answer: its status and what it said (at most `MAX_ANSWER` bytes), or why it wasn't reached.
fn answer(sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, Vec<u8>), String> {
    let mut answer = sent.map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    (answer.body_mut().as_reader().take(MAX_ANSWER + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ANSWER {
        return Err("the answer was too large".to_string());
    }
    Ok((answer.status().as_u16(), bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `Authorization` header of a request to an AWS service (Signature Version 4): the request's method is
/// POST, its path "/", and these four headers are the signed ones. `key`: the key's id, its secret, its region.
fn aws_authorization(key: (&str, &str, &str), host: &str, when: &str, kind: &str, target: &str, body: &[u8]) -> String {
    let (id, secret, region) = key;
    const SERVICE: &str = "rekognition";
    const SIGNED: &str = "content-type;host;x-amz-date;x-amz-target";
    let day = &when[..8.min(when.len())];
    let sha = |data: &[u8]| hex(digest::digest(&digest::SHA256, data).as_ref());
    let request = format!(
        "POST\n/\n\ncontent-type:{kind}\nhost:{host}\nx-amz-date:{when}\nx-amz-target:{target}\n\n{SIGNED}\n{}",
        sha(body)
    );
    let scope = format!("{day}/{region}/{SERVICE}/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{when}\n{scope}\n{}", sha(request.as_bytes()));
    let mac =
        |key: &[u8], data: &str| hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data.as_bytes()).as_ref().to_vec();
    let mut key = mac(format!("AWS4{secret}").as_bytes(), day);
    for part in [region, SERVICE, "aws4_request"] {
        key = mac(&key, part);
    }
    format!("AWS4-HMAC-SHA256 Credential={id}/{scope}, SignedHeaders={SIGNED}, Signature={}", hex(&mac(&key, &to_sign)))
}

/// Celebrities in the picture: [{name, confidence %, links}], best first.
fn rekognition(home: &Path, jpeg: &[u8], places: &Places, shared: Shared) -> Result<Value, String> {
    let Some((id, secret, region)) = saved::aws(home) else {
        return Err("No Amazon Rekognition key saved".to_string());
    };
    let region = region.filter(|r| !r.is_empty()).unwrap_or_else(|| "us-east-1".to_string());
    count(home, "rekognition", shared)?;
    let url = (places.rekognition.clone()).unwrap_or_else(|| format!("https://rekognition.{region}.amazonaws.com"));
    let (kind, target) = ("application/x-amz-json-1.1", "RekognitionService.RecognizeCelebrities");
    let body = filejson::dumps(&json!({"Image": {"Bytes": B64.encode(jpeg)}}));
    let when: String = utc_now().chars().filter(|c| *c != '-' && *c != ':').collect();
    let named = url.split_once("//").map_or(url.as_str(), |(_, host)| host);
    let authorization = aws_authorization((&id, &secret, &region), named, &when, kind, target, body.as_bytes());
    let ask = agent()
        .post(format!("{url}/"))
        .header("Content-Type", kind)
        .header("X-Amz-Date", &when)
        .header("X-Amz-Target", target);
    let (status, said) =
        answer(ask.header("Authorization", authorization).header("User-Agent", UA).send(body.as_bytes()))?;
    let said: Value = serde_json::from_slice(&said).unwrap_or(Value::Null);
    if status != 200 {
        // (in the words Amazon's own library says a refusal with)
        let code = said
            .get("__type")
            .and_then(Value::as_str)
            .map_or(status.to_string(), |t| t.rsplit('#').next().unwrap_or(t).to_string());
        let why = said.get("message").or_else(|| said.get("Message")).and_then(Value::as_str).unwrap_or("");
        return Err(format!("An error occurred ({code}) when calling the RecognizeCelebrities operation: {why}"));
    }
    let mut out: Vec<Value> = (said.get("CelebrityFaces").and_then(Value::as_array).into_iter().flatten())
        .filter_map(|f| {
            let name = f.get("Name").and_then(Value::as_str).filter(|n| english(n))?;
            let confidence = filejson::round(f.get("MatchConfidence").and_then(Value::as_f64).unwrap_or(0.0), 1);
            let links: Vec<String> = (f.get("Urls").and_then(Value::as_array).into_iter().flatten())
                .filter_map(Value::as_str)
                .filter(|u| us_site(u))
                .map(|u| format!("https://{u}"))
                .collect();
            Some(json!({"name": name, "confidence": confidence, "links": links}))
        })
        .collect();
    out.sort_by(|a, b| b["confidence"].as_f64().unwrap_or(0.0).total_cmp(&a["confidence"].as_f64().unwrap_or(0.0)));
    Ok(Value::Array(out))
}

/// What may stand in an address as it is; everything else is written as %XX.
fn quoted(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Google Cloud Vision's web detection of a picture, as `describe` tells it.
fn vision(home: &Path, key: &str, jpeg: &[u8], places: &Places, shared: Shared) -> Result<Value, String> {
    if key.is_empty() {
        return Err("No Cloud Vision key saved".to_string());
    }
    count(home, "vision", shared)?;
    let ask = json!({"requests": [{"image": {"content": B64.encode(jpeg)}, "features": [{"type": "WEB_DETECTION", "maxResults": 30}]}]});
    let to = format!("{}/v1/images:annotate?key={}", places.vision, quoted(key));
    let sent = agent()
        .post(to)
        .header("Content-Type", "application/json")
        .header("User-Agent", UA)
        .send(filejson::dumps(&ask).as_bytes());
    let (status, said) = answer(sent)?;
    let said: Value = if said.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&said).map_err(|_| "the answer wasn't understood".to_string())?
    };
    if status != 200 {
        let why = said
            .get("error")
            .and_then(|e| e.get("message"))
            .map_or_else(|| format!("answer {status}"), |m| m.as_str().map_or_else(|| m.to_string(), str::to_string));
        return Err(format!("Cloud Vision: {why}"));
    }
    let first = said
        .get("responses")
        .and_then(|r| r.get(0))
        .and_then(|r| r.get("webDetection"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    Ok(describe(&first))
}

/// Cloud Vision's web detection, English and US sites only.
///   match   "exact"   this same picture is on web pages: the names those pages give are a real clue
///           "partial" pages share part of the picture: usually just a similar scene, so not used for
///                     suggestions
///           "similar" only different pictures that look alike; "none" nothing
///   names   web entities in Google's order. Their scores are unbounded relevance weights, not confidence.
///   pages   [{title, url, domain, match: exact or partial}], exact copies first
pub fn describe(w: &Value) -> Value {
    let list = |k: &str| w.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let pages_all: Vec<Value> =
        list("pagesWithMatchingImages").into_iter().filter(|p| text(p, "url").starts_with("http")).collect();
    let kind = |p: &Value| {
        if p.get("fullMatchingImages").and_then(Value::as_array).is_some_and(|f| !f.is_empty()) {
            "exact"
        } else {
            "partial"
        }
    };
    let title = |p: &Value| {
        // without its tags
        let (mut out, mut tag) = (String::new(), None::<String>);
        for c in text(p, "pageTitle").chars() {
            match (&mut tag, c) {
                (None, '<') => tag = Some(String::new()),
                (None, c) => out.push(c),
                (Some(inside), '>') if !inside.is_empty() => tag = None,
                (Some(inside), '>') => {
                    // "<>" is no tag: kept as it is
                    out.push_str("<>");
                    let _ = inside;
                    tag = None;
                }
                (Some(inside), c) => inside.push(c),
            }
        }
        if let Some(open) = tag {
            out.push('<');
            out.push_str(&open);
        }
        out.trim().chars().take(160).collect::<String>()
    };
    let mut pages: Vec<Value> = (pages_all.iter())
        .filter(|p| us_site(&text(p, "url")))
        .map(|p| json!({"title": title(p), "url": text(p, "url"), "domain": host(&text(p, "url")), "match": kind(p)}))
        .filter(|p| p["title"] == "" || english(p["title"].as_str().unwrap_or("")))
        .collect();
    pages.sort_by_key(|p| p["match"] != "exact");
    let full = list("fullMatchingImages").len();
    let exact_pages = pages_all.iter().filter(|p| kind(p) == "exact").count();
    let matched = if full > 0 || exact_pages > 0 {
        "exact"
    } else if !list("partialMatchingImages").is_empty() || !pages_all.is_empty() {
        "partial"
    } else if !list("visuallySimilarImages").is_empty() {
        "similar"
    } else {
        "none"
    };
    let mut entities = list("webEntities");
    let score = |e: &Value| e.get("score").and_then(Value::as_f64).unwrap_or(0.0);
    entities.sort_by(|a, b| score(b).total_cmp(&score(a)));
    let (mut seen, mut names) = (Vec::new(), Vec::new());
    for e in &entities {
        let name = text(e, "description").trim().to_string();
        if english(&name) && !seen.contains(&name.to_lowercase()) {
            seen.push(name.to_lowercase());
            names.push(name);
        }
    }
    let guess: Vec<String> =
        list("bestGuessLabels").iter().map(|g| text(g, "label")).filter(|l| english(l)).take(1).collect();
    names.truncate(8);
    pages.truncate(8);
    json!({"match": matched, "guess": guess, "names": names, "pages": pages, "exactPages": exact_pages})
}

/// Words for Amazon's match confidence. Display bands, not measured on this project's faces.
fn level(pct: f64) -> &'static str {
    if pct >= 95.0 {
        "high"
    } else if pct >= 85.0 {
        "medium"
    } else {
        "low"
    }
}

/// Which services have a key: (Google, Amazon).
pub fn services(home: &Path, google_key: &str) -> (bool, bool) {
    (!google_key.is_empty(), saved::aws(home).is_some())
}

/// Ask every service that has a key, and combine: {suggestions, checked, details, errors, usage}. A
/// suggestion is {name, level, confidence (Amazon's %, or null), why: [reasons], links}:
///   Amazon names a celebrity          -> its confidence: high from 95%, medium from 85%, else low
///   Google finds this exact picture   -> person-like names from those pages: medium
///   both name the same person         -> very high
/// Err: no service is set up.
pub fn suggest(
    home: &Path,
    google_key: &str,
    face_jpeg: &[u8],
    photo_jpeg: &[u8],
    places: &Places,
    shared: Shared,
) -> Result<Value, String> {
    let (google, amazon) = services(home, google_key);
    if !google && !amazon {
        return Err("No web search service is set up (see Settings in this dialog)".to_string());
    }
    let (mut checked, mut errors, mut details, mut used) = (Vec::new(), Vec::new(), Map::new(), Map::new());
    // (in the order they were found: a suggestion keeps its place when the other service agrees)
    let mut by: Vec<(String, Value)> = Vec::new();
    if amazon {
        checked.push("Amazon celebrity recognition (the face)");
        // one service failing mustn't hide the other's answer
        match rekognition(home, face_jpeg, places, shared) {
            Ok(celebs) => {
                for c in celebs.as_array().into_iter().flatten().take(2) {
                    let (name, confidence) =
                        (c["name"].as_str().unwrap_or(""), c["confidence"].as_f64().unwrap_or(0.0));
                    let found = json!({"name": name, "level": level(confidence), "confidence": c["confidence"],
                                       "why": [format!("Amazon recognizes {name} ({confidence:.1}% match)")], "links": c["links"]});
                    match by.iter_mut().find(|(k, _)| *k == name.to_lowercase()) {
                        Some(kept) => kept.1 = found,
                        None => by.push((name.to_lowercase(), found)),
                    }
                }
                details.insert("amazon".to_string(), celebs);
            }
            Err(why) => errors.push(format!("Amazon: {why}")),
        }
        used.insert("rekognition".to_string(), usage(home, "rekognition")?);
    }
    if google {
        checked.push("Google pages with this picture (the whole photo)");
        match vision(home, google_key, photo_jpeg, places, shared) {
            Ok(v) => {
                if v["match"] == "exact" {
                    let pages = v["exactPages"].as_u64().unwrap_or(0);
                    let reason = |name: &str| {
                        let (n, s) = (
                            if pages == 0 { "some".to_string() } else { pages.to_string() },
                            if pages == 1 { "" } else { "s" },
                        );
                        format!("Google finds this exact picture on {n} web page{s} naming {name}")
                    };
                    let names: Vec<&str> = v["names"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|n| person(n))
                        .take(2)
                        .collect();
                    for name in names {
                        match by.iter_mut().find(|(k, _)| *k == name.to_lowercase()) {
                            Some((_, both)) => {
                                both["level"] = json!("very high");
                                if let Some(why) = both["why"].as_array_mut() {
                                    why.push(json!(reason(name)));
                                }
                            }
                            None => {
                                let links: Vec<&Value> = v["pages"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter(|p| p["match"] == "exact")
                                    .map(|p| &p["url"])
                                    .take(2)
                                    .collect();
                                by.push((name.to_lowercase(), json!({"name": name, "level": "medium", "confidence": null, "why": [reason(name)], "links": links})));
                            }
                        }
                    }
                }
                details.insert("google".to_string(), v);
            }
            Err(why) => errors.push(format!("Google: {why}")),
        }
        used.insert("vision".to_string(), usage(home, "vision")?);
    }
    let mut suggestions: Vec<Value> = by.into_iter().map(|(_, s)| s).collect();
    let place = |s: &Value| LEVELS.iter().position(|l| s["level"] == *l).unwrap_or(LEVELS.len());
    suggestions.sort_by(|a, b| {
        place(a)
            .cmp(&place(b))
            .then(b["confidence"].as_f64().unwrap_or(0.0).total_cmp(&a["confidence"].as_f64().unwrap_or(0.0)))
    });
    Ok(json!({"suggestions": suggestions, "checked": checked, "details": details, "errors": errors, "usage": used}))
}

impl Labeler {
    /// The JPEG to search the web with: the whole photo, or the face with hair, chin and some background
    /// around it (enlarged to at least 400 px, as search sites find small pictures poorly). With the
    /// original photo gone, the stored picture of the face is all there is.
    fn search_picture(&self, i: usize, face: bool) -> Result<Vec<u8>, String> {
        let Some(whole) = self.photo_jpeg(i) else {
            let kept = self.inner.db().store.thumbs[i].clone();
            if kept.is_empty() {
                return Err("Neither the photo nor a picture of this face is on this computer.".to_string());
            }
            return Ok(kept);
        };
        if !face {
            return Ok((*whole).clone());
        }
        let img = Bgr::from_jpeg(&whole).map_err(|e| e.0)?;
        let at = self.inner.db().store.boxes[i];
        let (x, y, w, h) = (at[0] as f64, at[1] as f64, at[2] as f64, at[3] as f64);
        let (x0, y0) = (((x - w / 2.0) as i64).max(0) as usize, ((y - h / 2.0) as i64).max(0) as usize);
        let (x1, y1) = (
            ((x + w * 1.5) as i64).clamp(0, img.w as i64) as usize,
            ((y + h * 1.5) as i64).clamp(0, img.h as i64) as usize,
        );
        if x1 <= x0 || y1 <= y0 {
            return Err("The face isn't in its photo.".to_string());
        }
        let mut crop = Bgr::new(x1 - x0, y1 - y0);
        for row in y0..y1 {
            let from = &img.row(row)[x0 * 3..x1 * 3];
            crop.px[(row - y0) * crop.w * 3..(row - y0 + 1) * crop.w * 3].copy_from_slice(from);
        }
        let larger = (400.0 / crop.w.max(crop.h) as f64).max(1.0);
        let crop = if larger > 1.0 { crop.resize_cubic(larger, larger) } else { crop };
        crop.to_jpeg(92).map_err(|e| e.0)
    }

    /// `/api/websearch`: the face to Amazon, its photo to Google, only when the owner clicks. A face asked
    /// about before is answered from what was kept (no new searches, no cost) unless asked to search anew;
    /// only complete answers are kept.
    pub(crate) fn websearch_post(&self, body: &Map<String, Value>) -> Answer {
        let (i, fid) = {
            let db = self.inner.db();
            let i = body.get("face").and_then(Value::as_u64).map(|i| i as usize).filter(|i| *i < db.n());
            let Some(i) = i else { return Err(asked("no such face")) };
            (i, db.store.fid[i].clone())
        };
        let kept = self.inner.web_saved.lock().unwrap_or_else(|e| e.into_inner()).get(&fid).cloned();
        if let Some(Value::Object(mut kept)) = kept.filter(|k| !falsy(Some(k)) && falsy(body.get("refresh"))) {
            kept.insert("cached".to_string(), json!(true));
            kept.insert("errors".to_string(), json!([]));
            kept.insert("usage".to_string(), json!({}));
            return Ok(Response::json(Value::Object(kept)));
        }
        let (face, photo) =
            (self.search_picture(i, true).map_err(asked)?, self.search_picture(i, false).map_err(asked)?);
        let key = saved::google_key_in_use(&self.home);
        let shared = |service: &str, month: &str, cap: i64| self.count_use(service, month, cap);
        let mut found = suggest(&self.home, &key, &face, &photo, &Places::from_env(), &shared).map_err(asked)?;
        found["when"] = json!(utc_now());
        if found["errors"].as_array().is_some_and(Vec::is_empty) {
            let mut web = self.inner.web_saved.lock().unwrap_or_else(|e| e.into_inner());
            let keep = json!({"when": found["when"], "suggestions": found["suggestions"], "checked": found["checked"], "details": found["details"]});
            web.insert(fid, keep);
            atomic_write(&self.data.join(crate::label::WEB_SAVED), filejson::rows_of(&web, true).as_bytes())
                .map_err(|e| Response::error(500, format!("can't write the Web search answers: {e}")))?;
        }
        Ok(Response::json(found))
    }

    /// One search counted with every device's in the shared database, when there is one (see `Shared`).
    fn count_use(&self, service: &str, month: &str, cap: i64) -> Result<Option<i64>, String> {
        #[cfg(feature = "sync")]
        {
            self.count_in_database(service, month, cap)
        }
        #[cfg(not(feature = "sync"))]
        {
            let _ = (service, month, cap);
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    use super::*;

    /// What a stand-in was asked: (the request's first line, its headers in lower case, its body).
    type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

    /// A stand-in for both services on this computer: `answer` makes (status, body) of a request's path and body.
    fn stand_in(answer: impl Fn(&str, &Value) -> (u16, String) + Send + 'static) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut first, mut headers, mut length) = (String::new(), String::new(), 0usize);
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap_or(0);
                    }
                    headers.push_str(&line);
                }
                let mut body = vec![0u8; length];
                if std::io::Read::read_exact(&mut reader, &mut body).is_err() {
                    continue;
                }
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                let (code, text) = answer(&path, &body);
                kept.lock().unwrap().push((first.trim().to_string(), headers, body));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        (url, seen)
    }

    fn home(name: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("face-websearch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    /// Stand-in keys, put together here so that no file holds anything that looks like a key.
    fn keys() -> (String, String, String) {
        (format!("AKIA{}", "TEST".repeat(4)), "s".repeat(40), format!("AIza{}", "g".repeat(35)))
    }

    #[test]
    fn a_request_to_amazon_is_signed_as_amazons_own_library_signs_it() {
        // (the value is botocore's for the same request: its key, date, region, headers and body)
        let (id, secret, _) = keys();
        let body = filejson::dumps(&json!({"Image": {"Bytes": "AAAA"}}));
        let said = aws_authorization(
            (&id, &secret, "us-east-1"),
            "rekognition.us-east-1.amazonaws.com",
            "20261121T123045Z",
            "application/x-amz-json-1.1",
            "RekognitionService.RecognizeCelebrities",
            body.as_bytes(),
        );
        assert_eq!(
            said,
            format!(
                "AWS4-HMAC-SHA256 Credential={id}/20261121/us-east-1/rekognition/aws4_request, \
                                  SignedHeaders=content-type;host;x-amz-date;x-amz-target, \
                                  Signature=fdedcfd3aeab0376648de5ed6138284fc04fccbef382b795065ec248326b923d"
            )
        );
    }

    #[test]
    fn only_english_text_us_sites_and_names_like_a_persons() {
        assert!(
            english("Mary J. Blige \u{2013} \u{201c}Live\u{201d} (2020) & more!")
                && !english("")
                && !english("Bj\u{f6}rk")
                && !english("\u{6771}\u{4eac}")
        );
        assert!(
            us_site("https://www.example.com/a?b")
                && us_site("example.org")
                && us_site("http://Sub.Example.GOV:8080/x")
                && us_site("//example.us")
        );
        assert!(
            !us_site("https://example.co.uk/")
                && !us_site("example.de")
                && !us_site("")
                && !us_site("https://user@example.fr/")
        );
        for name in ["Javier Bardem", "Mary J. Blige", "Jean-Luc Picard", "Conan O'brien Jr. Smith"] {
            assert!(person(name), "{name}");
        }
        for name in [
            "Cher",
            "javier bardem",
            "The Quick Brown Fox Jumps",
            "Photo 2020",
            "A B",
            "Disc-Jockey",
            "MARY SMITH",
            "Mary  Smith",
            "",
        ] {
            assert_eq!(person(name), name == "Disc-Jockey", "{name}");
        }
        assert_eq!((level(95.0), level(94.9), level(85.0), level(84.9)), ("high", "medium", "medium", "low"));
        assert_eq!(quoted("abc-_.~/9"), "abc-_.~/9");
        assert_eq!(quoted("a b&c=d"), "a%20b%26c%3Dd");
    }

    #[test]
    fn what_google_found_is_told_in_english_and_us_sites_only() {
        let w = json!({
            "webEntities": [{"description": "Second", "score": 0.5}, {"description": " First Person ", "score": 9.5}, {"description": "first person", "score": 0.4},
                            {"description": "Bj\u{f6}rk", "score": 99}, {"score": 3}],
            "bestGuessLabels": [{"label": "\u{6771}\u{4eac}"}, {"label": "a guess"}, {"label": "another"}],
            "pagesWithMatchingImages": [
                {"url": "https://example.com/partial", "pageTitle": "A <b>partial</b> page"},
                {"url": "https://example.de/exact", "pageTitle": "Not a US site", "fullMatchingImages": [{"url": "x"}]},
                {"url": "https://news.example.org/exact", "pageTitle": "  An exact <i>copy</i> ", "fullMatchingImages": [{"url": "x"}]},
                {"url": "https://example.net/odd", "pageTitle": "Caf\u{e9} title"},
                {"url": "ftp://example.com/x", "pageTitle": "not a web page"},
                {"url": "https://example.com/none"},
            ],
            "partialMatchingImages": [{"url": "y"}],
        });
        let d = describe(&w);
        assert_eq!(d["match"], "exact");
        assert_eq!(d["exactPages"], 2, "counted before the sites are sorted out");
        assert_eq!(d["names"], json!(["First Person", "Second"]));
        assert_eq!(d["guess"], json!(["a guess"]));
        assert_eq!(
            d["pages"],
            json!([
                {"title": "An exact copy", "url": "https://news.example.org/exact", "domain": "news.example.org", "match": "exact"},
                {"title": "A partial page", "url": "https://example.com/partial", "domain": "example.com", "match": "partial"},
                {"title": "", "url": "https://example.com/none", "domain": "example.com", "match": "partial"},
            ])
        );
        assert_eq!(describe(&json!({"partialMatchingImages": [1]}))["match"], "partial");
        assert_eq!(describe(&json!({"visuallySimilarImages": [1]}))["match"], "similar");
        assert_eq!(
            describe(&json!({})),
            json!({"match": "none", "guess": [], "names": [], "pages": [], "exactPages": 0})
        );
    }

    #[test]
    fn both_services_are_asked_counted_and_combined() {
        let (url, seen) = stand_in(|path, body| {
            if path.starts_with("/v1/images:annotate") {
                let exact = json!({"webEntities": [{"description": "Ann Example", "score": 2}, {"description": "Some Body", "score": 1}, {"description": "a party", "score": 3}],
                                   "pagesWithMatchingImages": [{"url": "https://example.com/a", "pageTitle": "Ann", "fullMatchingImages": [1]}]});
                return (200, json!({"responses": [{"webDetection": exact}]}).to_string());
            }
            assert!(body["Image"]["Bytes"].is_string());
            (200, json!({"CelebrityFaces": [{"Name": "Bob Sample", "MatchConfidence": 80.04, "Urls": ["www.example.com/bob", "example.de/bob"]},
                                             {"Name": "Ann Example", "MatchConfidence": 97.26, "Urls": []},
                                             {"Name": "Third Person", "MatchConfidence": 60}]}).to_string())
        });
        let home = home("both");
        let (id, secret, google) = keys();
        saved::save_aws(&home, &id, &secret, None).unwrap();
        let places = Places { vision: url.clone(), rekognition: Some(url) };
        let alone = |_: &str, _: &str, _: i64| Ok(None);
        let found = suggest(&home, &google, b"a face", b"a photo", &places, &alone).unwrap();
        assert_eq!(found["errors"], json!([]));
        assert_eq!(
            found["checked"],
            json!(["Amazon celebrity recognition (the face)", "Google pages with this picture (the whole photo)"])
        );
        // both name Ann: very high; Bob from Amazon alone: low; Some Body from Google alone: medium
        let said: Vec<(&str, &str)> = found["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), s["level"].as_str().unwrap()))
            .collect();
        assert_eq!(said, [("Ann Example", "very high"), ("Some Body", "medium"), ("Bob Sample", "low")]);
        assert_eq!(
            found["suggestions"][0]["why"],
            json!([
                "Amazon recognizes Ann Example (97.3% match)",
                "Google finds this exact picture on 1 web page naming Ann Example"
            ])
        );
        assert_eq!(
            found["suggestions"][2],
            json!({"name": "Bob Sample", "level": "low", "confidence": 80.0, "why": ["Amazon recognizes Bob Sample (80.0% match)"],
                                                    "links": ["https://www.example.com/bob"]})
        );
        assert_eq!(found["suggestions"][1]["links"], json!(["https://example.com/a"]));
        assert_eq!(
            (
                &found["usage"]["vision"]["used"],
                &found["usage"]["rekognition"]["used"],
                &found["usage"]["vision"]["cap"]
            ),
            (&json!(1), &json!(1), &json!(3700))
        );
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            // the face to Amazon, signed; the photo to Google, with its key in the address
            assert_eq!(seen[0].2, json!({"Image": {"Bytes": B64.encode(b"a face")}}));
            assert!(
                seen[0].1.to_lowercase().contains("authorization: aws4-hmac-sha256 credential=")
                    && seen[0].1.to_lowercase().contains("x-amz-target: rekognitionservice.recognizecelebrities")
            );
            assert!(seen[1].0.starts_with(&format!("POST /v1/images:annotate?key={google} ")));
            assert_eq!(seen[1].2["requests"][0]["image"]["content"], json!(B64.encode(b"a photo")));
            assert_eq!(seen[1].2["requests"][0]["features"], json!([{"type": "WEB_DETECTION", "maxResults": 30}]));
        }
        // past the cap nothing is sent, and the other service still answers
        let month = usage(&home, "vision").unwrap()["month"].as_str().unwrap().to_string();
        std::fs::write(home.join("vision_usage.json"), filejson::dumps(&json!({"month": month, "used": 3700})))
            .unwrap();
        let found = suggest(&home, &google, b"a face", b"a photo", &places, &alone).unwrap();
        assert_eq!(
            found["errors"],
            json!(["Google: vision limit reached: 3700 searches this month (keeps the bill under $10)"])
        );
        assert_eq!(
            (
                seen.lock().unwrap().len(),
                &found["usage"]["rekognition"]["used"],
                found["suggestions"].as_array().unwrap().len()
            ),
            (3, &json!(2), 2)
        );
        // every device's searches count: the shared count is kept here too, and its cap refuses
        let everyone = |service: &str, _: &str, _: i64| {
            if service == "rekognition" { Ok(Some(41)) } else { Err("vision limit reached, everywhere".to_string()) }
        };
        let found = suggest(&home, &google, b"a face", b"a photo", &places, &everyone).unwrap();
        assert_eq!(
            (&found["usage"]["rekognition"]["used"], &found["errors"]),
            (&json!(41), &json!(["Google: vision limit reached, everywhere"]))
        );
        // a count from last month is none
        std::fs::write(home.join("vision_usage.json"), r#"{"month": "2020-01", "used": 3700}"#).unwrap();
        assert_eq!(usage(&home, "vision").unwrap()["used"], 0);
        std::fs::write(home.join("vision_usage.json"), "{").unwrap();
        assert!(usage(&home, "vision").is_err(), "a count that can't be read is never taken as none");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_service_that_refuses_or_is_gone_is_said_and_no_service_is_refused() {
        let (url, _) = stand_in(|path, _| {
            if path.starts_with("/v1/") {
                return (403, json!({"error": {"code": 403, "message": "API key not valid."}}).to_string());
            }
            (400, json!({"__type": "com.amazon.coral.service#InvalidSignatureException", "message": "The signature does not match."}).to_string())
        });
        let home = home("refused");
        let (id, secret, google) = keys();
        let alone = |_: &str, _: &str, _: i64| Ok(None);
        let places = Places { vision: url.clone(), rekognition: Some(url) };
        assert_eq!(
            suggest(&home, "", b"f", b"p", &places, &alone).unwrap_err(),
            "No web search service is set up (see Settings in this dialog)"
        );
        saved::save_aws(&home, &id, &secret, Some("eu-west-1")).unwrap();
        let found = suggest(&home, &google, b"f", b"p", &places, &alone).unwrap();
        assert_eq!(
            found["errors"],
            json!([
                "Amazon: An error occurred (InvalidSignatureException) when calling the RecognizeCelebrities operation: The signature does not match.",
                "Google: Cloud Vision: API key not valid."
            ])
        );
        assert_eq!((&found["suggestions"], &found["details"]), (&json!([]), &json!({})));
        // nothing listens there
        let gone = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port())
        };
        let found =
            suggest(&home, &google, b"f", b"p", &Places { vision: gone.clone(), rekognition: Some(gone) }, &alone)
                .unwrap();
        let errors: Vec<&str> = found["errors"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(errors.len() == 2 && errors[0].starts_with("Amazon: ") && errors[1].starts_with("Google: "));
        // (counted all the same: a search is counted before it is sent)
        assert_eq!(found["usage"]["vision"]["used"], 2);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn only_a_stand_in_on_this_computer_can_take_a_services_place() {
        // (the names are read when asked, so this test alone sets them)
        unsafe {
            std::env::set_var("FACE_TEST_VISION", "http://192.168.1.20:8000");
            std::env::set_var("FACE_TEST_REKOGNITION", "https://127.0.0.1:1");
        }
        let places = Places::from_env();
        assert_eq!((places.vision.as_str(), places.rekognition), ("https://vision.googleapis.com", None));
        unsafe {
            std::env::set_var("FACE_TEST_VISION", "http://127.0.0.1:8123/elsewhere");
            std::env::set_var("FACE_TEST_REKOGNITION", "http://127.0.0.1:8124");
        }
        let places = Places::from_env();
        assert_eq!(
            (places.vision.as_str(), places.rekognition.as_deref()),
            ("https://vision.googleapis.com", Some("http://127.0.0.1:8124"))
        );
        unsafe {
            std::env::remove_var("FACE_TEST_VISION");
            std::env::remove_var("FACE_TEST_REKOGNITION");
        }
    }
}
