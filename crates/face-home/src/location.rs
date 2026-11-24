//! Where a device is, as far as the private folder keeps it (the owner's design, 2026-11-24: "add
//! geolocation for the devices and show the location of each device", "keeps its location history in the
//! database", "an opt-in setting", and of the history: "only keep the On a move for last 90 days").
//!
//! A location is two numbers and how sure they are (`Fix`): from the system's own location service, or a
//! place the owner set by hand on that device. Nothing is asked, shown or kept while the tracker page's
//! switch is off, which is how a new copy starts.
//!
//! The history is rows in the shared database, which is where it is kept. A row is written when the switch
//! is turned on and then only when the device has moved (`moved`): a device that stays where it is adds
//! nothing. Rows wait in `HOME/location.json` until the tracker's own connection has put them into the
//! database (`face_db::location`); nothing waits on a device without a shared database.

use std::path::{Path, PathBuf};

use face_files::{
    filejson,
    paths::{SharedLock, atomic_write},
};
use serde_json::{Value, json};

/// the tracker page's "Show and keep this device's location" (off unless switched on)
pub const OPTION: &str = "ft-o-optLocation";
/// the tracker page's place set by hand: coordinates or a town's name; empty: the system is asked
pub const PLACE: &str = "ft-o-optPlace";

const FILE: &str = "location.json";
/// metres: a device that is this far from where its last row was written has moved (a choice: far enough
/// that a reading's own wandering inside a building writes nothing)
pub const MOVED: f64 = 100.0;
/// milliseconds between two rows at the least (a device on a journey writes a row a minute, not one for
/// every hundred metres)
pub const APART_MS: i64 = 60_000;
/// rows that wait at the most (a phone away from its database for days): beyond it the oldest go
const MOST_WAITING: usize = 5000;
/// metres: a reading less sure than this says nothing of where the device is
const LEAST_SURE: f64 = 1_000_000.0;

/// Who said where the device is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// the system's own location service
    System,
    /// the owner, on this device
    Hand,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::System => "system",
            Source::Hand => "hand",
        }
    }

    pub fn of(word: &str) -> Option<Source> {
        match word {
            "system" => Some(Source::System),
            "hand" => Some(Source::Hand),
            _ => None,
        }
    }
}

/// Where a device is: degrees north and east, how many metres around that the device may be (nothing for
/// a place set by hand), and when this was said (milliseconds since 1970, UTC).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: Option<f64>,
    pub at: i64,
    pub source: Source,
}

impl Fix {
    /// A location out of what a system or a row said, or nothing when it isn't one: numbers outside the
    /// globe, a reading with no idea how sure it is, or the point 0, 0 that a failed reading gives.
    pub fn of(lat: f64, lon: f64, accuracy: Option<f64>, at: i64, source: Source) -> Option<Fix> {
        let on_globe =
            lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon);
        let sure = match accuracy {
            None => source == Source::Hand,
            Some(m) => m.is_finite() && (0.0..=LEAST_SURE).contains(&m),
        };
        if !on_globe || !sure || (lat == 0.0 && lon == 0.0) || at <= 0 {
            return None;
        }
        Some(Fix { lat: round(lat, 6), lon: round(lon, 6), accuracy: accuracy.map(|m| round(m, 1)), at, source })
    }

    /// As the pages, the bus and a waiting row have it: numbers and one word.
    pub fn json(&self) -> Value {
        json!({"lat": self.lat, "lon": self.lon, "acc": self.accuracy, "at": self.at, "source": self.source.word()})
    }

    /// Out of such an object, whoever wrote it: checked as `of` checks.
    pub fn from_json(v: &Value) -> Option<Fix> {
        let number = |k: &str| v.get(k).and_then(Value::as_f64);
        let accuracy = match v.get("acc") {
            None | Some(Value::Null) => None,
            Some(a) => Some(a.as_f64()?),
        };
        let at = v.get("at").and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)))?;
        Fix::of(number("lat")?, number("lon")?, accuracy, at, Source::of(v.get("source")?.as_str()?)?)
    }
}

fn round(v: f64, places: i32) -> f64 {
    let by = 10f64.powi(places);
    (v * by).round() / by
}

/// Metres between two points of the globe (the great circle; the earth as a ball of its mean radius).
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lat2) = (a.0.to_radians(), b.0.to_radians());
    let (dlat, dlon) = (lat2 - lat1, (b.1 - a.1).to_radians());
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().min(1.0).asin()
}

/// Whether the device has moved since `last`: further than `MOVED`, and further than either reading's own
/// uncertainty (two readings whose circles overlap may be one place).
pub fn moved(last: &Fix, now: &Fix) -> bool {
    let unsure = last.accuracy.unwrap_or(0.0).max(now.accuracy.unwrap_or(0.0));
    distance_m((last.lat, last.lon), (now.lat, now.lon)) > MOVED.max(unsure)
}

/// Two numbers as the owner types a place: "47.6062, -122.3321", degrees north then east, with a comma, a
/// semicolon or a space between. Nothing for anything else (a town's name is looked up by the server).
pub fn coordinates(text: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> =
        text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    let [lat, lon] = parts.as_slice() else { return None };
    let (lat, lon) = (lat.parse::<f64>().ok()?, lon.parse::<f64>().ok()?);
    (lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
        .then_some((lat, lon))
}

/// The rows that wait for the shared database, and where the last one was written.
pub struct Log {
    file: PathBuf,
    /// written by the tracker's step, emptied by its connection to the database
    lock: SharedLock,
}

impl Log {
    pub fn new(home: &Path) -> Log {
        Log { file: home.join(FILE), lock: SharedLock::new(home, "location") }
    }

    fn read(&self) -> (Option<Fix>, Vec<Value>) {
        let kept: Value =
            std::fs::read(&self.file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
        let last = kept.get("last").and_then(Fix::from_json);
        let waiting = kept.get("waiting").and_then(Value::as_array).cloned().unwrap_or_default();
        (last, waiting.into_iter().filter(|r| Fix::from_json(r).is_some()).take(MOST_WAITING).collect())
    }

    fn write(&self, last: Option<&Fix>, waiting: &[Value]) -> Result<(), String> {
        if last.is_none() && waiting.is_empty() {
            return match std::fs::remove_file(&self.file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("can't clear the location file: {e}"))
                }
                _ => Ok(()),
            };
        }
        let kept = json!({"last": last.map(Fix::json), "waiting": waiting});
        atomic_write(&self.file, filejson::dumps(&kept).as_bytes()).map_err(|e| format!("can't keep the location: {e}"))
    }

    /// The device is at `fix` now, `place` being the town there in words. A row waits for the database when
    /// this is the first location since the switch was turned on, or when the device has moved since the
    /// last row and that row is a minute old. Whether a row was added. Nothing is kept unless `keeps` (the
    /// switch is on and the device has a shared database).
    pub fn note(&self, fix: &Fix, place: &str, keeps: bool) -> Result<bool, String> {
        if !keeps {
            return Ok(false);
        }
        let _one = self.lock.hold();
        let (last, mut waiting) = self.read();
        if let Some(last) = &last {
            // (the minute is counted either way: a clock that was set back must not hold the rows up)
            if !moved(last, fix) || fix.at.saturating_sub(last.at).abs() < APART_MS {
                return Ok(false);
            }
        }
        let mut row = fix.json();
        let place: String = place.chars().filter(|c| !c.is_control()).take(200).collect();
        row["place"] = json!(place.trim());
        waiting.push(row);
        if waiting.len() > MOST_WAITING {
            let over = waiting.len() - MOST_WAITING;
            waiting.drain(..over);
        }
        self.write(Some(fix), &waiting)?;
        Ok(true)
    }

    /// The switch was turned off: where the last row was written is forgotten, so that turning it on again
    /// writes a row. Rows that wait were written while it was on, and still go.
    pub fn off(&self) -> Result<(), String> {
        let _one = self.lock.hold();
        let (last, waiting) = self.read();
        if last.is_none() {
            return Ok(());
        }
        self.write(None, &waiting)
    }

    /// Where the last row was written, while the switch has been on since.
    pub fn last(&self) -> Option<Fix> {
        let _one = self.lock.hold();
        self.read().0
    }

    /// The rows that wait, the oldest first: each {lat, lon, acc, at, source, place}.
    pub fn waiting(&self) -> Vec<Value> {
        let _one = self.lock.hold();
        self.read().1
    }

    /// The database has these rows (told by their times): they wait no longer.
    pub fn sent(&self, times: &[i64]) -> Result<(), String> {
        if times.is_empty() {
            return Ok(());
        }
        let _one = self.lock.hold();
        let (last, mut waiting) = self.read();
        waiting.retain(|r| !r.get("at").and_then(Value::as_i64).is_some_and(|at| times.contains(&at)));
        self.write(last.as_ref(), &waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_795_161_900_000; // 2026-11-20 08:05 UTC

    fn at(lat: f64, lon: f64, acc: f64, t: i64) -> Fix {
        Fix::of(lat, lon, Some(acc), t, Source::System).unwrap()
    }

    #[test]
    fn a_location_is_two_numbers_on_the_globe_and_how_sure() {
        let f = Fix::of(47.606_209_123, -122.332_071_9, Some(64.96), T, Source::System).unwrap();
        assert_eq!((f.lat, f.lon, f.accuracy), (47.606_209, -122.332_072, Some(65.0)));
        assert_eq!(f.json(), json!({"lat": 47.606209, "lon": -122.332072, "acc": 65.0, "at": T, "source": "system"}));
        assert_eq!(Fix::from_json(&f.json()), Some(f));
        // a place set by hand has no uncertainty; a reading must say its own
        assert!(Fix::of(1.0, 2.0, None, T, Source::Hand).is_some());
        assert!(Fix::of(1.0, 2.0, None, T, Source::System).is_none());
        for (lat, lon, acc) in [
            (91.0, 0.0, 5.0),
            (-90.1, 0.0, 5.0),
            (10.0, 180.5, 5.0),
            (f64::NAN, 1.0, 5.0),
            (1.0, f64::INFINITY, 5.0),
            (1.0, 1.0, -1.0),
            (1.0, 1.0, f64::NAN),
            (1.0, 1.0, 2_000_000.0),
            (0.0, 0.0, 5.0),
        ] {
            assert!(Fix::of(lat, lon, Some(acc), T, Source::System).is_none(), "{lat} {lon} {acc}");
        }
        assert!(Fix::of(1.0, 1.0, Some(5.0), 0, Source::System).is_none());
        // what a row or another device says is checked the same way
        for bad in [
            json!({"lat": 1, "lon": 2, "acc": 5, "at": T}),
            json!({"lat": 1, "lon": 2, "acc": 5, "at": T, "source": "guess"}),
            json!({"lat": "1", "lon": 2, "acc": 5, "at": T, "source": "system"}),
            json!({"lat": 1, "lon": 2, "acc": "near", "at": T, "source": "system"}),
            json!({"lat": 1, "lon": 200, "acc": 5, "at": T, "source": "system"}),
            json!("here"),
        ] {
            assert_eq!(Fix::from_json(&bad), None, "{bad}");
        }
    }

    #[test]
    fn distances_are_the_globes() {
        // a degree of latitude is 111.2 km everywhere; a degree of longitude at 60 degrees north is half that
        assert!((distance_m((0.0, 0.0), (1.0, 0.0)) - 111_195.0).abs() < 10.0);
        assert!((distance_m((60.0, 0.0), (60.0, 1.0)) - 55_597.0).abs() < 60.0);
        assert_eq!(distance_m((47.6, -122.3), (47.6, -122.3)), 0.0);
        // across the date line the short way round
        assert!(distance_m((0.0, 179.9), (0.0, -179.9)) < 23_000.0);
    }

    #[test]
    fn a_device_has_moved_beyond_a_hundred_metres_and_beyond_its_readings_doubt() {
        let home = at(47.6062, -122.3321, 20.0, T);
        // 0.001 degrees north is 111 m
        assert!(!moved(&home, &at(47.6066, -122.3321, 20.0, T)), "44 m");
        assert!(moved(&home, &at(47.6075, -122.3321, 20.0, T)), "145 m");
        // a reading that is unsure by 500 m has not moved 145 m
        assert!(!moved(&home, &at(47.6075, -122.3321, 500.0, T)));
        assert!(!moved(&at(47.6062, -122.3321, 500.0, T), &at(47.6075, -122.3321, 20.0, T)));
        assert!(moved(&home, &at(47.6162, -122.3321, 500.0, T)), "1.1 km");
    }

    #[test]
    fn a_place_typed_by_hand_is_two_numbers() {
        assert_eq!(coordinates("47.6062, -122.3321"), Some((47.6062, -122.3321)));
        assert_eq!(coordinates("  -33.86 151.21 "), Some((-33.86, 151.21)));
        assert_eq!(coordinates("51.5;-0.12"), Some((51.5, -0.12)));
        for not in ["", "Seattle", "47.6", "47.6, -122.3, 12", "95, 10", "10, 190", "north, east", "nan, 1"] {
            assert_eq!(coordinates(not), None, "{not}");
        }
    }

    #[test]
    fn rows_wait_only_for_a_move_and_until_the_database_has_them() {
        let dir = std::env::temp_dir().join(format!("face-location-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = Log::new(&dir);
        let home = at(47.6062, -122.3321, 20.0, T);
        // not kept here: nothing written
        assert_eq!(log.note(&home, "Seattle", false), Ok(false));
        assert!(!dir.join(FILE).exists());
        // the first location since the switch was turned on is a row; staying adds none
        assert_eq!(log.note(&home, "Seattle,\u{7} Washington", true), Ok(true));
        assert_eq!(log.note(&at(47.6063, -122.3321, 20.0, T + 600_000), "Seattle", true), Ok(false));
        // a move within the same minute waits for the minute; after it, a row
        let away = at(47.7062, -122.3321, 20.0, T + 30_000);
        assert_eq!(log.note(&away, "Shoreline", true), Ok(false));
        let away = at(47.7062, -122.3321, 20.0, T + 90_000);
        assert_eq!(log.note(&away, "Shoreline", true), Ok(true));
        let waiting = log.waiting();
        assert_eq!(waiting.len(), 2);
        assert_eq!(
            waiting[0],
            json!({"lat": 47.6062, "lon": -122.3321, "acc": 20.0, "at": T, "source": "system", "place": "Seattle, Washington"})
        );
        // the database has the first: the second still waits, and where the last row was written stays
        log.sent(&[T]).unwrap();
        assert_eq!(log.waiting().iter().map(|r| r["at"].as_i64().unwrap()).collect::<Vec<_>>(), [T + 90_000]);
        assert_eq!(log.note(&at(47.7063, -122.3321, 20.0, T + 900_000), "Shoreline", true), Ok(false));
        // switched off and on again: a row for where the device is then, whether it moved or not
        log.off().unwrap();
        assert_eq!(log.waiting().len(), 1, "what waited still goes");
        assert_eq!(log.note(&at(47.7063, -122.3321, 20.0, T + 960_000), "Shoreline", true), Ok(true));
        log.sent(&[T + 90_000, T + 960_000]).unwrap();
        log.off().unwrap();
        assert!(!dir.join(FILE).exists(), "nothing waits and nothing is remembered: no file");
        // a damaged file is no rows, and what isn't a location in it is left out
        std::fs::write(dir.join(FILE), b"{\"waiting\": [{\"lat\": 500}, 7], \"last\": \"here\"}").unwrap();
        assert!(log.waiting().is_empty());
        std::fs::write(dir.join(FILE), b"not json").unwrap();
        assert!(log.waiting().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rows_that_wait_have_an_end() {
        let dir = std::env::temp_dir().join(format!("face-location-most-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = Log::new(&dir);
        let rows: Vec<Value> = (0..MOST_WAITING as i64).map(|i| at(10.0, 10.0, 5.0, T + i).json()).collect();
        log.write(None, &rows).unwrap();
        assert_eq!(log.note(&at(20.0, 20.0, 5.0, T + 10_000_000), "", true), Ok(true));
        let waiting = log.waiting();
        assert_eq!(waiting.len(), MOST_WAITING);
        assert_eq!(waiting[0]["at"], T + 1, "the oldest went");
        assert_eq!(waiting[MOST_WAITING - 1]["lat"], 20.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
