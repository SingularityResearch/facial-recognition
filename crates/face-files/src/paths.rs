//! Where the programs keep things: `ROOT` is the folder with
//! `models/` and `data/` (`FACE_ROOT` names another), `HOME` the private per-user folder (`FACE_HOME` names
//! another; otherwise each system's application-data folder, `FaceLabels`).

use std::{
    io,
    path::{Path, PathBuf},
};

pub fn root() -> PathBuf {
    if let Some(p) = std::env::var_os("FACE_ROOT").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

/// The folder with `models/` and `data/` for the installed app:
/// `FACE_ROOT` when set; on Windows beside the program (its installer puts it in a folder of the user's);
/// on macOS and Linux the user's application-data folder `FaceRecognizer` (a Mac's app can't write into
/// itself, and a Linux app's folder is replaced by an update).
pub fn app_root() -> PathBuf {
    if cfg!(windows) || std::env::var_os("FACE_ROOT").is_some_and(|p| !p.is_empty()) {
        return root();
    }
    user_data("FaceRecognizer")
}

pub fn home() -> PathBuf {
    if let Some(p) = std::env::var_os("FACE_HOME").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    user_data("FaceLabels")
}

/// This user's application-data folder for `name`, as each system expects it.
pub fn user_data(name: &str) -> PathBuf {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let user = var("HOME").or_else(|| var("USERPROFILE")).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        var("APPDATA").unwrap_or_else(|| user.join("AppData").join("Roaming")).join(name)
    } else if cfg!(target_os = "macos") {
        user.join("Library").join("Application Support").join(name)
    } else {
        var("XDG_DATA_HOME").unwrap_or_else(|| user.join(".local").join("share")).join(name)
    }
}

/// A time in seconds since 1970 as (year, month, day, hour, minute, second) in UTC.
fn civil(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // the calendar in 400-year eras that begin on a 1 March
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (day, month) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    (yoe + era * 400 + i64::from(month <= 2), month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}

fn seconds(at: std::time::SystemTime) -> i64 {
    match at.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64) - i64::from(e.duration().subsec_nanos() > 0),
    }
}

/// A time in UTC as it is stored: 2026-11-12T01:35:37Z.
pub fn utc_text(at: std::time::SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(seconds(at));
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Now, in UTC, as it is stored.
pub fn utc_now() -> String {
    utc_text(std::time::SystemTime::now())
}

/// A UTC time for file and folder names: 20261112-013537, which sorts in time order (`paths.utc_stamp`).
pub fn utc_stamp(at: std::time::SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(seconds(at));
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// Write a file without ever leaving a half-written one: into `<name>.tmp`, then put in its place.
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    replace_file(&tmp, path)
}

/// Put `tmp` in `dest`'s place, trying again for a second: on Windows that fails while another program (or
/// thread) has `dest` open to read it.
pub fn replace_file(tmp: &Path, dest: &Path) -> io::Result<()> {
    let mut tries = 20;
    loop {
        match std::fs::rename(tmp, dest) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && tries > 1 => {
                tries -= 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            done => return done,
        }
    }
}

/// One thread of one process at a time, for a file that two of the app's processes read, change and write
/// back: `HOME/locks/<name>.lock`, the same file for every program of the app, so that any two of its
/// processes keep each other out. Where the lock file can't be made or locked, it is
/// this process's threads alone that are kept apart, as there.
pub struct SharedLock {
    thread: std::sync::Mutex<()>,
    file: PathBuf,
}

/// The lock, held until this is dropped.
pub struct Held<'a> {
    file: Option<std::fs::File>,
    _thread: std::sync::MutexGuard<'a, ()>,
}

impl SharedLock {
    pub fn new(home: &Path, name: &str) -> SharedLock {
        SharedLock { thread: std::sync::Mutex::new(()), file: home.join("locks").join(format!("{name}.lock")) }
    }

    pub fn hold(&self) -> Held<'_> {
        let thread = self.thread.lock().unwrap_or_else(|e| e.into_inner());
        let file = self.file.parent().and_then(|dir| std::fs::create_dir_all(dir).ok()).and_then(|_| {
            let f = std::fs::OpenOptions::new().read(true).append(true).create(true).open(&self.file).ok()?;
            // waits for whoever holds it: the whole file (a lock on its first byte alone is the same lock to
            // Windows, so a program that takes only that one is kept out too)
            f.lock().ok()?;
            Some(f)
        });
        Held { file, _thread: thread }
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some(f) = &self.file {
            let _ = f.unlock();
        }
    }
}

/// The place of the one running copy of a program for a private folder, held until this is dropped (the
/// program's end, however it ends: the system lets go of the lock with the process).
pub struct OnlyCopy {
    _file: Option<std::fs::File>,
}

/// Take the place of the one running copy (`HOME/locks/<name>.lock`). None when another process holds it
/// and still does after `wait`: that one is the copy that runs, and this one must not start beside it.
/// (`wait`: a copy that starts itself again ends a moment after its successor begins.) Where the lock file
/// can't be made or locked at all, nothing is known and the place counts as taken by this copy, as before
/// there was a lock.
pub fn only_copy(home: &Path, name: &str, wait: std::time::Duration) -> Option<OnlyCopy> {
    let file = home.join("locks").join(format!("{name}.lock"));
    let opened = file
        .parent()
        .and_then(|dir| std::fs::create_dir_all(dir).ok())
        .and_then(|_| std::fs::OpenOptions::new().read(true).append(true).create(true).open(&file).ok());
    let Some(f) = opened else { return Some(OnlyCopy { _file: None }) };
    let end = std::time::Instant::now() + wait;
    loop {
        match f.try_lock() {
            Ok(()) => return Some(OnlyCopy { _file: Some(f) }),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < end => {
                std::thread::sleep(std::time::Duration::from_millis(100))
            }
            Err(std::fs::TryLockError::WouldBlock) => return None,
            Err(std::fs::TryLockError::Error(_)) => return Some(OnlyCopy { _file: None }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn times_are_written_in_utc_in_the_one_form_they_are_stored_in() {
        let at = |secs: u64| std::time::UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(utc_text(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(utc_text(at(1_794_447_337)), "2026-11-12T01:35:37Z");
        assert_eq!(utc_stamp(at(1_794_447_337)), "20261112-013537");
        assert_eq!(utc_text(at(951_782_399)), "2000-02-28T23:59:59Z");
        assert_eq!(utc_text(at(951_782_400)), "2000-02-29T00:00:00Z", "a leap day");
        assert_eq!(utc_text(at(4_107_542_399)), "2100-02-28T23:59:59Z");
        assert_eq!(utc_text(at(4_107_542_400)), "2100-03-01T00:00:00Z", "2100 has none");
        assert_eq!(utc_text(std::time::UNIX_EPOCH - Duration::from_millis(500)), "1969-12-31T23:59:59Z");
        assert_eq!(utc_now().len(), 20);
    }

    #[test]
    fn only_one_copy_of_a_program_runs_for_a_private_folder() {
        let dir = std::env::temp_dir().join(format!("face-only-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let none = Duration::ZERO;
        let first = only_copy(&dir, "running", none).expect("the first copy takes the place");
        assert!(dir.join("locks").join("running.lock").is_file());
        // a second one (another handle on the file: what another process has) is told, at once or after its wait
        assert!(only_copy(&dir, "running", none).is_none());
        let began = Instant::now();
        assert!(only_copy(&dir, "running", Duration::from_millis(300)).is_none());
        assert!(began.elapsed() >= Duration::from_millis(280), "it waited for the first to end");
        // another program's place, and another folder's, are their own
        assert!(only_copy(&dir, "another", none).is_some());
        assert!(only_copy(&dir.join("elsewhere"), "running", none).is_some());
        // a copy that waits takes the place the moment the first is gone (a program starting itself again)
        let waiting = {
            let dir = dir.clone();
            std::thread::spawn(move || only_copy(&dir, "running", Duration::from_secs(20)).map(|_| Instant::now()))
        };
        std::thread::sleep(Duration::from_millis(300));
        let let_go = Instant::now();
        drop(first);
        let taken = waiting.join().unwrap().expect("the waiting copy takes the place");
        assert!(taken.duration_since(let_go) < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_shared_lock_keeps_another_holder_of_the_same_file_out() {
        let dir = std::env::temp_dir().join(format!("face-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // two locks on one file: what two processes have
        let (here, there) = (SharedLock::new(&dir, "alerts"), SharedLock::new(&dir, "alerts"));
        let held = here.hold();
        assert!(dir.join("locks").join("alerts.lock").is_file());
        let waiting = std::thread::spawn(move || {
            let t = Instant::now();
            let _held = there.hold();
            t.elapsed()
        });
        std::thread::sleep(Duration::from_millis(300));
        drop(held);
        let waited = waiting.join().unwrap();
        assert!(waited >= Duration::from_millis(250), "the second holder waited only {waited:?}");
        // another name is another lock
        let other = SharedLock::new(&dir, "settings");
        let _a = here.hold();
        let t = Instant::now();
        let _b = other.hold();
        assert!(t.elapsed() < Duration::from_millis(200));
        drop((_a, _b));
        let _ = std::fs::remove_dir_all(dir);
    }
}
