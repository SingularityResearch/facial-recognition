//! Where this device is (the owner's design, 2026-11-24: "add geolocation for the devices and show the
//! location of each device", "keeps its location history in the database", "an opt-in setting"; shown with
//! the town, "state, and country if possible").
//!
//! Nothing here runs until the tracker page's switch is on (`face_home::location::OPTION`, off for a new
//! copy). While it is on, the location is one of two things:
//!
//! - a place the owner typed on this device (`PLACE`: coordinates, or a town of the list the program
//!   carries). Nothing is asked of anyone then;
//! - else what the system's own location service says, where the system has one the app can count on:
//!   Windows (`Windows.Devices.Geolocation`), a Mac (Core Location), and a phone, whose shell asks its
//!   system and tells it here (`told`). The system asks its owner once, and works a computer's place out
//!   with its maker's service: that is the system's doing, at the owner's word, and the privacy policy says
//!   so. Linux and the Raspberry Pi have no such service, so there a place is typed.
//!
//! What is made of it: the page's own line (`Location::answer`), the node's `place` on the bus for the
//! owner's other devices (numbers and one word; each device puts the town's name to them itself, from its
//! own list), and the history's rows, which wait in the private folder until this program's own connection
//! to the shared database has sent them (`send`). Rows older than 90 days are deleted there once a day.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use face_home::location::{APART_MS, Fix, Log, OPTION, PLACE, Source, coordinates, distance_m, moved};
use serde_json::{Value, json};

use crate::towns;

/// What the system's location service said last.
static SYSTEM: Mutex<Option<Fix>> = Mutex::new(None);
/// The system's service is wanted now: the switch is on and no place is typed.
static WANTED: AtomicBool = AtomicBool::new(false);
/// Why the system gives no location (`ASKING` and the words after it); 0 while nothing was said.
static WHY: AtomicU8 = AtomicU8::new(0);

/// the system is being asked
pub const ASKING: u8 = 1;
/// the owner, or a rule of the device, doesn't let the app see the location
pub const NOT_ALLOWED: u8 = 2;
/// the system's location service is switched off
pub const SWITCHED_OFF: u8 = 3;
/// the system was asked and couldn't tell
pub const NOT_FOUND: u8 = 4;

/// Whether this system has a location service the app asks.
pub const SYSTEM_HAS: bool = cfg!(any(windows, target_os = "macos", target_os = "ios", target_os = "android"));

/// seconds between two askings of a computer's location service (a computer moves slowly, if at all)
const ASK_EVERY: u64 = 60;
/// seconds after the program's start before a computer's location service is first asked. Asked while the
/// page was opening the camera, Windows held both up: the camera's start did not finish in 30 s, twice of
/// two starts, where it starts within seconds without the asking (a Windows computer, 2026-11-24).
const FIRST_ASK_AFTER: u64 = 60;
/// seconds between two steps of the tracker's own watch
const STEP_EVERY: u64 = 5;
/// seconds between two tries to send the rows that wait
const SEND_EVERY: u64 = 30;
/// metres: the town's name is worked out again when the device is this far from where it last was
const NAME_AGAIN: f64 = 200.0;

fn now_ms() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn system() -> MutexGuard<'static, Option<Fix>> {
    SYSTEM.lock().unwrap_or_else(|e| e.into_inner())
}

/// The system's location service says where the device is: degrees north and east, and the metres around
/// that point the device may be. A phone's shell says it; a computer's service is asked here. Taken only
/// while the location is wanted, and only when it is a location (`Fix::of`).
pub fn told(lat: f64, lon: f64, accuracy: f64) {
    if !wanted() {
        return;
    }
    if let Some(fix) = Fix::of(lat, lon, Some(accuracy), now_ms(), Source::System) {
        *system() = Some(fix);
        WHY.store(0, Ordering::SeqCst);
    }
}

/// The system gives no location now, and why (`ASKING`, `NOT_ALLOWED`, `SWITCHED_OFF`, `NOT_FOUND`). What
/// it said before stays while it is only asking or couldn't tell (a device that lost its signal is still
/// where it last was), and goes when the owner or the system took the location away.
pub fn refused(why: i32) {
    if let Ok(why @ ASKING..=NOT_FOUND) = u8::try_from(why) {
        WHY.store(why, Ordering::SeqCst);
        if why == NOT_ALLOWED || why == SWITCHED_OFF {
            *system() = None; // (no longer the app's to know)
        }
    }
}

/// Whether the system's location service is to be asked now: a phone's shell asks this every few seconds,
/// and asks its system for the location (and its owner for the permission) only while the answer is yes.
pub fn wanted() -> bool {
    WANTED.load(Ordering::SeqCst)
}

/// What the page says when the system doesn't let the app see the location: where the owner allows it.
const ALLOW: &str = if cfg!(windows) {
    "Windows doesn't let the app see this device's location. Allow it in Windows' Settings, under Privacy & security, Location: Location services, and Let desktop apps access your location. Or type this device's place."
} else if cfg!(target_os = "macos") {
    "This Mac doesn't let the app see its location. Allow it in System Settings, under Privacy & Security, Location Services, for Face Recognizer. Or type this device's place."
} else if cfg!(target_os = "ios") {
    "This device doesn't let the app see its location. Allow it in Settings, under Privacy & Security, Location Services, for Face Recognizer. Or type this device's place."
} else {
    "This device doesn't let the app see its location. Allow it in Settings, under Apps, Face Recognizer, Permissions, Location. Or type this device's place."
};

/// Why there is no location, in words for the page.
fn why_words(why: u8) -> &'static str {
    if !SYSTEM_HAS {
        return "This system has no location service the app can ask. Type this device's place.";
    }
    match why {
        NOT_ALLOWED => ALLOW,
        SWITCHED_OFF => {
            "This device's location service is switched off. Turn it on in the system's settings (Location, or Location Services), or type this device's place."
        }
        NOT_FOUND => {
            "The system's location service couldn't tell where this device is. It is asked again; or type this device's place."
        }
        _ => "Asking this device's location service…",
    }
}

/// What the page is shown and the node publishes, as the last step left it.
#[derive(Default)]
struct Shown {
    on: bool,
    fix: Option<Fix>,
    /// the town at `named_at`, in words
    town: String,
    named_at: Option<(f64, f64)>,
    /// the place typed by hand as it was last read, where that is, and since when
    typed: String,
    hand: Option<(f64, f64)>,
    hand_since: i64,
    why: &'static str,
    /// where the history's last row was written, as far as this run knows
    last_row: Option<Fix>,
    /// the history's file was looked at since the switch last changed
    looked: bool,
    said: String,
}

/// This device's location: what its page shows, what its node publishes, and the history's waiting rows.
pub struct Location {
    home: PathBuf,
    log: Log,
    shown: Mutex<Shown>,
    watching: AtomicBool,
}

impl Location {
    pub fn new(home: &Path) -> Arc<Location> {
        Arc::new(Location {
            home: home.to_path_buf(),
            log: Log::new(home),
            shown: Mutex::new(Shown::default()),
            watching: AtomicBool::new(false),
        })
    }

    fn shown(&self) -> MutexGuard<'_, Shown> {
        self.shown.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One step: `on` is the page's switch, `typed` the place set by hand, `keeps` whether this device has
    /// a shared database for the history. The node's `place` after it: the location as numbers and one
    /// word, or null while there is none to say.
    pub fn step(&self, on: bool, typed: &str, keeps: bool) -> Value {
        let typed = typed.trim();
        let mut s = self.shown();
        if !on {
            WANTED.store(false, Ordering::SeqCst);
            *system() = None;
            WHY.store(0, Ordering::SeqCst);
            // (where the last row was written is forgotten: switched on again, a row says where it is then)
            if s.on || !s.looked {
                self.tell(&mut s, self.log.off());
            }
            *s = Shown { looked: true, said: std::mem::take(&mut s.said), ..Shown::default() };
            return Value::Null;
        }
        if !s.on {
            *s = Shown {
                on: true,
                last_row: self.log.last(),
                looked: true,
                said: std::mem::take(&mut s.said),
                ..Shown::default()
            };
        }
        if typed != s.typed {
            s.typed = typed.to_string();
            s.hand = coordinates(typed).or_else(|| towns::named(typed).map(|t| (t.lat, t.lon)));
            s.hand_since = now_ms();
        }
        let by_hand = !typed.is_empty();
        let ask = !by_hand && SYSTEM_HAS;
        if ask && !WANTED.swap(true, Ordering::SeqCst) {
            WHY.store(ASKING, Ordering::SeqCst);
        } else if !ask {
            WANTED.store(false, Ordering::SeqCst);
            *system() = None;
        }
        let fix = match (by_hand, s.hand) {
            (true, Some((lat, lon))) => Fix::of(lat, lon, None, s.hand_since, Source::Hand),
            (true, None) => None,
            (false, _) => *system(),
        };
        s.why = match (&fix, by_hand) {
            (Some(_), _) => "",
            (None, true) => {
                "That place is not two numbers (latitude, longitude) and not a town the app knows. Try a town with its state or country, as in Springfield, Illinois."
            }
            (None, false) => why_words(WHY.load(Ordering::SeqCst)),
        };
        if let Some(fix) = &fix {
            if s.named_at.is_none_or(|at| distance_m(at, (fix.lat, fix.lon)) > NAME_AGAIN) {
                s.town = towns::said(fix.lat, fix.lon);
                s.named_at = Some((fix.lat, fix.lon));
            }
            // a row of the history: the first since the switch was turned on, and then for a move
            let due = s.last_row.as_ref().is_none_or(|last| moved(last, fix) && (fix.at - last.at).abs() >= APART_MS);
            if keeps && due {
                match self.log.note(fix, &s.town, true) {
                    Ok(true) => s.last_row = Some(*fix),
                    Ok(false) => s.last_row = self.log.last().or(s.last_row),
                    Err(e) => self.tell(&mut s, Err(e)),
                }
            }
        }
        s.fix = fix;
        fix.map_or(Value::Null, |f| f.json())
    }

    /// A trouble with the history's file, said once.
    fn tell(&self, s: &mut Shown, done: Result<(), String>) {
        if let Err(e) = done
            && e != s.said
        {
            eprintln!("Location: {e}");
            s.said = e;
        }
    }

    /// What the page asks (`/api/location`): whether the switch is on, where the device is with the town
    /// in words, or why there is no location; and whether this system has a location service to ask.
    pub fn answer(&self) -> Value {
        let s = self.shown();
        let place = s.fix.map(|f| {
            let mut place = f.json();
            place["town"] = json!(s.town);
            place
        });
        json!({"on": s.on, "place": place, "why": s.why, "system": SYSTEM_HAS, "typed": !s.typed.is_empty()})
    }

    /// The settings as the tracker page keeps them, read and a step taken; the node's `place` after it.
    pub fn look(&self) -> Value {
        let store = face_home::store::Store::new(&self.home);
        let on = store.setting("track", OPTION).as_deref() == Some("1");
        let typed = store.setting("track", PLACE).unwrap_or_default();
        let keeps = matches!(face_db::Settings::load(&self.home), Ok(Some(_)));
        self.step(on, &typed, keeps)
    }

    /// On a thread of its own, started once: a step every few seconds, the node's `place` kept as the step
    /// leaves it, and a computer's location service asked while it is wanted.
    pub fn watch(self: &Arc<Self>, bus: Arc<crate::bus::Bus>) {
        if self.watching.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        let started = std::thread::Builder::new().name("this device's location".into()).spawn(move || {
            let (began, mut asked) = (Instant::now(), None::<Instant>);
            loop {
                let _ = bus.set("place", me.look());
                let settled = began.elapsed().as_secs() >= FIRST_ASK_AFTER;
                if wanted() && settled && asked.is_none_or(|at| at.elapsed().as_secs() >= ASK_EVERY) {
                    asked = Some(Instant::now());
                    ask_system();
                    let _ = bus.set("place", me.look());
                } else if !wanted() {
                    asked = None;
                }
                std::thread::sleep(Duration::from_secs(STEP_EVERY));
            }
        });
        if started.is_err() {
            self.watching.store(false, Ordering::SeqCst);
        }
    }
}

/// The town at a point in words, for another device's location: each device names a point from its own
/// list, so no words come from another device. The last few points are remembered.
pub fn town_at(lat: f64, lon: f64) -> String {
    static NAMED: Mutex<Vec<((i64, i64), String)>> = Mutex::new(Vec::new());
    if !(lat.is_finite() && lon.is_finite()) {
        return String::new();
    }
    // (to about a hundred metres: the town's name doesn't change within that)
    let key = ((lat * 1000.0).round() as i64, (lon * 1000.0).round() as i64);
    let mut named = NAMED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, town)) = named.iter().find(|(k, _)| *k == key) {
        return town.clone();
    }
    let town = towns::said(lat, lon);
    if named.len() >= 64 {
        named.remove(0);
    }
    named.push((key, town.clone()));
    town
}

/// What sending the history's rows remembers between two tries.
#[derive(Default)]
pub struct Sending {
    tried: Option<Instant>,
    /// the UTC day old rows were last deleted on
    pruned: String,
    said: String,
}

/// Put the rows that wait into the shared database, and once a day delete the rows that are older than
/// they are kept. Called with every "this device is here"; it does something every `SEND_EVERY` seconds.
/// A database that can't take them (one that isn't up to date yet) leaves them waiting, and that never
/// ends the connection's other work.
pub async fn send(client: &tokio_postgres::Client, device: &str, home: &Path, sending: &mut Sending) {
    if sending.tried.is_some_and(|at| at.elapsed().as_secs() < SEND_EVERY) {
        return;
    }
    sending.tried = Some(Instant::now());
    let log = Log::new(home);
    let rows = log.waiting();
    let mut done: Result<(), String> = Ok(());
    if !rows.is_empty() {
        done = match face_db::location::send(client, device, &rows).await {
            Ok(times) => log.sent(&times),
            Err(e) => Err(e),
        };
    }
    let today = crate::paths::utc_now()[..10].to_string();
    if done.is_ok() && sending.pruned != today {
        done = face_db::location::prune(client).await.map(|_| ());
        if done.is_ok() {
            sending.pruned = today;
        }
    }
    match done {
        Ok(()) => sending.said.clear(),
        Err(e) if e != sending.said => {
            eprintln!("{e}");
            sending.said = e;
        }
        Err(_) => {}
    }
}

/// Ask this system's location service once, outside the app, and say in words how that went: for trying
/// the service on a kind of system (`examples/where.rs`). With `show`, where it says the device is.
pub fn try_system(show: bool) -> String {
    if !SYSTEM_HAS {
        return why_words(0).to_string();
    }
    WANTED.store(true, Ordering::SeqCst);
    WHY.store(ASKING, Ordering::SeqCst);
    ask_system();
    // (a Mac's service answers on a thread of its own, a little later)
    for _ in 0..40 {
        if system().is_some() || WHY.load(Ordering::SeqCst) != ASKING {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let (fix, why) = (system().take(), WHY.swap(0, Ordering::SeqCst));
    WANTED.store(false, Ordering::SeqCst);
    match fix {
        Some(fix) if show => {
            format!(
                "{}: {:.5}, {:.5}, within {} m",
                towns::said(fix.lat, fix.lon),
                fix.lat,
                fix.lon,
                fix.accuracy.unwrap_or(0.0)
            )
        }
        Some(fix) => format!("The system said where this device is, within {} m.", fix.accuracy.unwrap_or(0.0)),
        None => why_words(why).to_string(),
    }
}

// ---- a computer's own location service ---------------------------------------------------------------------

/// Ask this computer's location service once and take what it says (`told`, `refused`). A phone's shell
/// asks its system itself; a system without such a service is never asked.
#[cfg(windows)]
fn ask_system() {
    use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator, PositionAccuracy};
    let began = Instant::now();
    let mut allowed_after = Duration::ZERO;
    let mut asked = || -> windows::core::Result<Option<(f64, f64, f64)>> {
        // (Windows' own answer for this app: its location switches, and its question to the owner where
        // it asks one)
        let access = Geolocator::RequestAccessAsync()?.join()?;
        allowed_after = began.elapsed();
        if access != GeolocationAccessStatus::Allowed {
            return Ok(None);
        }
        let locator = Geolocator::new()?;
        locator.SetDesiredAccuracy(PositionAccuracy::High)?;
        let at = locator.GetGeopositionAsync()?.join()?.Coordinate()?;
        let point = at.Point()?.Position()?;
        Ok(Some((point.Latitude, point.Longitude, at.Accuracy()?)))
    };
    let answer = asked();
    // (how long Windows took, when it took long: never where the device is)
    if began.elapsed() > Duration::from_secs(3) || std::env::var_os("FACE_LOCATION_TIMES").is_some() {
        println!(
            "Location: Windows' location service took {:.1} s to answer ({:.1} s of it to say whether the app may ask).",
            began.elapsed().as_secs_f64(),
            allowed_after.as_secs_f64()
        );
    }
    match answer {
        Ok(Some((lat, lon, accuracy))) => told(lat, lon, accuracy),
        Ok(None) => refused(i32::from(NOT_ALLOWED)),
        Err(_) => refused(i32::from(NOT_FOUND)),
    }
}

#[cfg(target_os = "macos")]
fn ask_system() {
    mac::ask();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn ask_system() {}

#[cfg(target_os = "macos")]
mod mac;

#[cfg(test)]
mod tests {
    use super::*;

    /// (the tests share what the system said: one at a time)
    static ONE: Mutex<()> = Mutex::new(());

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("face-server-location-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nothing_is_asked_shown_or_kept_while_the_switch_is_off() {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = folder("off");
        let here = Location::new(&dir);
        assert_eq!(here.step(false, "47.6062, -122.3321", true), Value::Null);
        assert!(!wanted());
        // what a shell says while nothing is wanted is not taken
        told(47.6062, -122.3321, 20.0);
        assert_eq!(*system(), None);
        let a = here.answer();
        assert_eq!((a["on"].clone(), a["place"].clone()), (json!(false), Value::Null));
        assert!(!dir.join("location.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_place_typed_by_hand_is_the_location_and_the_system_is_not_asked() {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = folder("hand");
        let here = Location::new(&dir);
        let place = here.step(true, " 39.8017, -89.6437 ", true);
        assert_eq!(
            (place["lat"].clone(), place["lon"].clone(), place["acc"].clone()),
            (json!(39.8017), json!(-89.6437), Value::Null)
        );
        assert_eq!(place["source"], "hand");
        assert!(!wanted(), "a typed place asks nobody");
        let a = here.answer();
        assert_eq!(a["place"]["town"], "Springfield, Illinois, United States");
        assert_eq!((a["on"].clone(), a["why"].clone(), a["typed"].clone()), (json!(true), json!(""), json!(true)));
        // the history: a row for where it is when switched on, none for staying, one for a new place
        assert_eq!(Log::new(&dir).waiting().len(), 1);
        here.step(true, "39.8017, -89.6437", true);
        assert_eq!(Log::new(&dir).waiting().len(), 1);
        // a town by its name; the same minute, so the row waits for the minute
        let place = here.step(true, "Paris, France", true);
        assert_eq!(place["source"], "hand");
        assert_eq!(here.answer()["place"]["town"], "Paris, Île-de-France, France");
        assert_eq!(Log::new(&dir).waiting().len(), 1);
        // what is no place is said, and nothing is shown for it
        assert_eq!(here.step(true, "somewhere nice", true), Value::Null);
        let a = here.answer();
        assert!(a["place"].is_null() && a["why"].as_str().unwrap().contains("not a town the app knows"), "{a}");
        // switched off: nothing more is shown; what waited still goes
        assert_eq!(here.step(false, "39.8017, -89.6437", true), Value::Null);
        assert_eq!(here.answer()["on"], false);
        assert_eq!(Log::new(&dir).waiting().len(), 1);
        assert_eq!(Log::new(&dir).last(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_shared_database_nothing_of_the_history_is_written() {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = folder("alone");
        let here = Location::new(&dir);
        assert_eq!(here.step(true, "39.8017, -89.6437", false)["source"], "hand");
        assert!(!dir.join("location.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_systems_service_is_wanted_only_without_a_typed_place_and_what_it_says_is_checked() {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = folder("system");
        let here = Location::new(&dir);
        assert_eq!(here.step(true, "", true), Value::Null);
        assert_eq!(wanted(), SYSTEM_HAS);
        let a = here.answer();
        assert_eq!(a["system"], SYSTEM_HAS);
        assert!(!a["why"].as_str().unwrap().is_empty(), "no location yet: the page is told why");
        if SYSTEM_HAS {
            // what is no location is not taken for one
            told(91.0, 0.0, 10.0);
            told(47.0, -122.0, -1.0);
            told(0.0, 0.0, 10.0);
            assert_eq!(here.step(true, "", true), Value::Null);
            told(47.6062, -122.3321, 35.0);
            let place = here.step(true, "", true);
            assert_eq!(
                (place["lat"].clone(), place["source"].clone(), place["acc"].clone()),
                (json!(47.6062), json!("system"), json!(35.0))
            );
            assert_eq!(here.answer()["place"]["town"], "Seattle, Washington, United States");
            assert_eq!(Log::new(&dir).waiting().len(), 1);
            // the owner said no: the app no longer knows, and says why
            refused(i32::from(NOT_ALLOWED));
            assert_eq!(here.step(true, "", true), Value::Null);
            assert!(here.answer()["why"].as_str().unwrap().contains("Allow it"));
            // a typed place takes over, and the system is no longer asked
            told(47.6062, -122.3321, 35.0);
            assert_eq!(here.step(true, "Springfield, IL", true)["source"], "hand");
            assert!(!wanted());
            assert_eq!(*system(), None);
        } else {
            assert!(a["why"].as_str().unwrap().contains("Type this device's place"));
        }
        here.step(false, "", true);
        assert!(!wanted());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn another_devices_point_is_named_from_this_devices_own_list() {
        assert_eq!(town_at(39.8017, -89.6437), "Springfield, Illinois, United States");
        assert_eq!(town_at(39.8017, -89.6437), "Springfield, Illinois, United States");
        assert_eq!(town_at(f64::NAN, 0.0), "");
        assert_eq!(town_at(95.0, 0.0), "");
    }

    #[test]
    fn why_there_is_no_location_is_said_in_words_for_every_reason() {
        for why in [0, ASKING, NOT_ALLOWED, SWITCHED_OFF, NOT_FOUND, 99] {
            let words = why_words(why);
            assert!(words.len() > 20 && words.len() < 400, "{why}: {words}");
        }
        // what a shell says that is no reason changes nothing
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        WHY.store(ASKING, Ordering::SeqCst);
        for not in [0, -1, 5, 300] {
            refused(not);
            assert_eq!(WHY.load(Ordering::SeqCst), ASKING, "{not}");
        }
        WHY.store(0, Ordering::SeqCst);
    }
}
