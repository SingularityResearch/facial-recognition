//! What a device says of itself for the Signals chart's "Device" section (the owner, 2026-11-23: the sending
//! device's type, capabilities, temperature and performance belong with the sensors, for every source).
//!
//! Readings, asked of the system itself each time the page asks (every few seconds; nothing runs between):
//! how much of the processor is in use since the last asking, how much of the memory, how warm the device is,
//! its heat state, its battery and whether it is on power. Each is a number, or nothing where this system
//! doesn't say: never a guess, and never from a program that may not be installed.
//!
//! - Windows: processor and memory; the battery of a computer that has one. Windows gives a program no
//!   temperature without special rights.
//! - Linux, the Raspberry Pi: processor, memory, the warmest of the system's own thermal zones, a battery.
//! - Mac: processor and memory. macOS gives a program no temperature.
//! - A phone: memory, and the processor where the system lets an app read it. Its heat state (the system's
//!   own word for it: nominal, fair, serious, critical; a phone gives no degrees) and its battery come from
//!   the app's shell, which asks the system for them (`told`).
//!
//! Like every reading of the chart they are kept in the page's memory only and never stored; another device of
//! the shared database gets them with this device's readout, as numbers.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

/// The processor's time when it was last asked for: (busy, all), in the system's own unit.
static BEFORE: Mutex<Option<(u64, u64)>> = Mutex::new(None);
/// What a phone's shell said last, and when: (heat state 0 to 3, battery in percent, on power).
static TOLD: Mutex<Option<(Instant, Option<u8>, Option<f64>, Option<bool>)>> = Mutex::new(None);
/// How long what the shell said stands (it says it every few seconds).
const TOLD_FOR: Duration = Duration::from_secs(60);

/// The heat states, as the chart words them (Apple's four; Android's seven are told as these).
pub const HEAT: [&str; 4] = ["nominal", "fair", "serious", "critical"];

/// A phone's shell says what only the system's own interfaces give it: the heat state (0 nominal to 3
/// critical; anything else: not known), the battery in percent (outside 0 to 100: not known), and whether the
/// device is on power (0 no, 1 yes; anything else: not known).
pub fn told(heat: i32, battery: f32, on_power: i32) {
    let heat = u8::try_from(heat).ok().filter(|h| usize::from(*h) < HEAT.len());
    let battery = Some(f64::from(battery)).filter(|b| b.is_finite() && (0.0..=100.0).contains(b));
    let on_power = match on_power {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    *TOLD.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), heat, battery, on_power));
}

fn one_place(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The share of `all` that `busy` is, in percent; nothing when no time passed.
fn share(busy: u64, all: u64) -> Option<f64> {
    (all > 0 && busy <= all).then(|| busy as f64 * 100.0 / all as f64)
}

/// This device now: {load, memory, temp, heat, battery, charging}, each null where this system doesn't say.
/// `load` is the processor's use since this was last asked (null the first time).
pub fn reading() -> Value {
    let load = cpu_times().and_then(|now| {
        let was = BEFORE.lock().unwrap_or_else(|e| e.into_inner()).replace(now);
        let (busy0, all0) = was?;
        // (a counter that started again: nothing to say this time)
        share(now.0.checked_sub(busy0)?, now.1.checked_sub(all0)?)
    });
    let (mut battery, mut charging) = battery();
    let mut heat = None;
    if let Some((at, h, b, c)) = *TOLD.lock().unwrap_or_else(|e| e.into_inner()) {
        if at.elapsed() < TOLD_FOR {
            heat = h;
            battery = battery.or(b);
            charging = charging.or(c);
        }
    }
    json!({"load": load.map(one_place), "memory": memory_used().map(one_place), "temp": temperature().map(one_place),
           "heat": heat, "battery": battery.map(|b| b.round()), "charging": charging})
}

/// What this device is, as short lines of text for the chart: its system, the app's version, its processor
/// and memory. `system`: what a phone's shell said the system is (a computer's is asked of the system itself).
pub fn facts(system: &str) -> Value {
    let has = crate::hardware::describe();
    let said = |k: &str| has.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let on_phone = cfg!(any(target_os = "android", target_os = "ios"));
    let processor = match (said("cpu"), has.get("threads").and_then(Value::as_u64)) {
        (cpu, _) if cpu.is_empty() || cpu == "unknown processor" => String::new(),
        (cpu, Some(threads)) if threads > 0 => format!("{cpu}, {threads} threads"),
        (cpu, _) => cpu,
    };
    let memory =
        has.get("memory_gb").and_then(Value::as_u64).filter(|gb| *gb > 0).map(|gb| format!("{gb} GB of memory"));
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let (device, processor, memory) = {
        let _ = (processor, memory); // (a computer's: a phone's system is asked in its own way)
        let (model, cores, bytes) = phone();
        phone_says(&model, cores, bytes)
    };
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let (device, memory) = ("a computer".to_string(), memory.unwrap_or_default());
    json!({"device": device,
           "system": if on_phone && !system.trim().is_empty() { system.trim().to_string() } else { said("system") },
           "app": format!("Face Recognizer {}", env!("CARGO_PKG_VERSION")),
           "processor": processor, "memory": memory})
}

/// A phone in words: what its maker calls the model, how many cores its processor has, its memory. Each only
/// when the system said it; a model's name is kept as text of a line's length and nothing else is made of it.
#[cfg(any(target_os = "android", target_os = "ios", test))]
fn phone_says(model: &str, cores: Option<u64>, bytes: Option<u64>) -> (String, String, String) {
    let model: String = model.chars().filter(|c| !c.is_control()).take(60).collect::<String>().trim().to_string();
    let kind = if model.starts_with("iPad") { "a tablet" } else { "a phone" };
    let device = if model.is_empty() { kind.to_string() } else { format!("{kind}, {model}") };
    let processor = cores.filter(|n| *n > 0 && *n < 4096).map(|n| format!("{n} processor cores")).unwrap_or_default();
    let gb = bytes.map(|b| (b + (1 << 29)) >> 30).filter(|gb| *gb > 0);
    (device, processor, gb.map(|gb| format!("{gb} GB of memory")).unwrap_or_default())
}

/// An iPhone's own answers: its model as Apple's systems name it ("iPhone18,2"), its cores, its memory in bytes.
#[cfg(target_os = "ios")]
fn phone() -> (String, Option<u64>, Option<u64>) {
    // SAFETY: the system writes at most `size` bytes into each place and says how many it wrote
    unsafe {
        let mut text = [0u8; 64];
        let mut size = text.len() - 1;
        let model =
            if libc::sysctlbyname(c"hw.machine".as_ptr(), text.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0)
                == 0
            {
                String::from_utf8_lossy(&text[..size.min(text.len())]).trim_end_matches('\0').to_string()
            } else {
                String::new()
            };
        let number = |name: &std::ffi::CStr| {
            let (mut n, mut size) = (0u64, std::mem::size_of::<u64>());
            (libc::sysctlbyname(name.as_ptr(), (&raw mut n).cast(), &mut size, std::ptr::null_mut(), 0) == 0)
                .then_some(n)
        };
        (model, number(c"hw.ncpu"), number(c"hw.memsize"))
    }
}

/// An Android phone's own answers: its maker and model as the system's properties name them, its cores, its
/// memory in bytes.
#[cfg(target_os = "android")]
fn phone() -> (String, Option<u64>, Option<u64>) {
    let said = |name: &std::ffi::CStr| {
        let mut value = [0 as libc::c_char; 128]; // (a property's value is 92 bytes at most, its end included)
        // SAFETY: the system writes a text of at most 92 bytes with its end into the place given
        let n = unsafe { libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) };
        let bytes: Vec<u8> = value.iter().take(usize::try_from(n).unwrap_or(0).min(92)).map(|c| *c as u8).collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    };
    let (maker, model) = (said(c"ro.product.manufacturer"), said(c"ro.product.model"));
    let named =
        if model.to_lowercase().starts_with(&maker.to_lowercase()) { model } else { format!("{maker} {model}") };
    let cores = std::thread::available_parallelism().ok().and_then(|n| u64::try_from(n.get()).ok());
    let bytes = std::fs::read_to_string("/proc/meminfo").ok().and_then(|text| {
        let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
        line.split_ascii_whitespace().nth(1)?.parse::<u64>().ok()?.checked_mul(1024)
    });
    (named.trim().to_string(), cores, bytes)
}

// ---- Linux, the Raspberry Pi, Android ----------------------------------------------------------------------

#[cfg(any(target_os = "linux", target_os = "android"))]
fn cpu_times() -> Option<(u64, u64)> {
    // "cpu  user nice system idle iowait irq softirq steal ...": the whole machine's, since it started
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    cpu_line(text.lines().next()?)
}

#[cfg(any(target_os = "linux", target_os = "android", test))]
fn cpu_line(line: &str) -> Option<(u64, u64)> {
    let mut parts = line.split_ascii_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let times: Vec<u64> = parts.take(8).map(|n| n.parse().ok()).collect::<Option<_>>()?;
    if times.len() < 4 {
        return None;
    }
    let all: u64 = times.iter().sum();
    let idle = times[3] + times.get(4).copied().unwrap_or(0);
    Some((all.checked_sub(idle)?, all))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn memory_used() -> Option<f64> {
    memory_from(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(any(target_os = "linux", target_os = "android", test))]
fn memory_from(meminfo: &str) -> Option<f64> {
    let kb = |name: &str| -> Option<u64> {
        let line = meminfo.lines().find(|l| l.starts_with(name))?;
        line[name.len()..].split_ascii_whitespace().next()?.parse().ok()
    };
    let (total, free) = (kb("MemTotal:")?, kb("MemAvailable:")?);
    share(total.checked_sub(free)?, total)
}

/// The warmest of the system's own thermal zones, in degrees Celsius.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn temperature() -> Option<f64> {
    let zones = std::fs::read_dir("/sys/class/thermal").ok()?;
    let read = |zone: std::path::PathBuf| degrees(&std::fs::read_to_string(zone.join("temp")).ok()?);
    zones
        .flatten()
        .filter(|z| z.file_name().to_string_lossy().starts_with("thermal_zone"))
        .take(64)
        .filter_map(|z| read(z.path()))
        .fold(None, |warmest: Option<f64>, t| Some(warmest.map_or(t, |w| w.max(t))))
}

/// A thermal zone's reading (thousandths of a degree) as degrees; nothing for what can't be a temperature.
#[cfg(any(target_os = "linux", target_os = "android", test))]
fn degrees(text: &str) -> Option<f64> {
    let milli: i64 = text.trim().parse().ok()?;
    (1_000..150_000).contains(&milli).then(|| milli as f64 / 1000.0)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn battery() -> (Option<f64>, Option<bool>) {
    let Ok(supplies) = std::fs::read_dir("/sys/class/power_supply") else { return (None, None) };
    for supply in supplies.flatten().take(32) {
        let said = |file: &str| std::fs::read_to_string(supply.path().join(file)).ok().map(|t| t.trim().to_string());
        // (the computer's own battery: a wireless mouse or keyboard has one too, and says "Device" of it)
        if said("type").as_deref() != Some("Battery") || said("scope").as_deref() == Some("Device") {
            continue;
        }
        let Some(level) = said("capacity").and_then(|c| c.parse::<f64>().ok()).filter(|c| (0.0..=100.0).contains(c))
        else {
            continue;
        };
        let on_power = said("status").map(|s| s != "Discharging");
        return (Some(level), on_power);
    }
    (None, None)
}

// ---- Windows -----------------------------------------------------------------------------------------------

#[cfg(windows)]
fn cpu_times() -> Option<(u64, u64)> {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetSystemTimes};
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut idle, mut kernel, mut user) = (zero, zero, zero);
    // SAFETY: three places for the three times it writes
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
        return None;
    }
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    // (the kernel's time has the idle time in it)
    let all = ticks(kernel) + ticks(user);
    Some((all.checked_sub(ticks(idle))?, all))
}

#[cfg(windows)]
fn memory_used() -> Option<f64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: a structure of plain numbers, its size said in it as the call wants
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut m) } != 0)
        .then(|| f64::from(m.dwMemoryLoad))
        .filter(|p| (0.0..=100.0).contains(p))
}

#[cfg(windows)]
fn temperature() -> Option<f64> {
    None // Windows gives a program none without special rights
}

#[cfg(windows)]
fn battery() -> (Option<f64>, Option<bool>) {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: a structure of plain numbers that the call fills
    let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return (None, None);
    }
    // (128: this computer has no battery; 255: not known)
    if s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
        return (None, None);
    }
    let on_power = match s.ACLineStatus {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    (Some(f64::from(s.BatteryLifePercent)), on_power)
}

// ---- a Mac, an iPhone --------------------------------------------------------------------------------------

#[cfg(target_vendor = "apple")]
fn cpu_times() -> Option<(u64, u64)> {
    // SAFETY: the kernel fills a structure of four counters, whose size in numbers is said
    unsafe {
        let mut info: libc::host_cpu_load_info = std::mem::zeroed();
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        let host = libc::mach_host_self();
        let got = libc::host_statistics(host, libc::HOST_CPU_LOAD_INFO, (&raw mut info).cast(), &mut count);
        if got != libc::KERN_SUCCESS {
            return None;
        }
        // user, system, idle, nice
        let all: u64 = info.cpu_ticks.iter().map(|t| u64::from(*t)).sum();
        Some((all.checked_sub(u64::from(info.cpu_ticks[2]))?, all))
    }
}

#[cfg(target_vendor = "apple")]
fn memory_used() -> Option<f64> {
    // SAFETY: the kernel fills a structure of counters, whose size in numbers is said; sysctl a number of 8 bytes
    unsafe {
        let mut vm: libc::vm_statistics64 = std::mem::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let host = libc::mach_host_self();
        if libc::host_statistics64(host, libc::HOST_VM_INFO64, (&raw mut vm).cast(), &mut count) != libc::KERN_SUCCESS {
            return None;
        }
        let (mut total, mut size) = (0u64, std::mem::size_of::<u64>());
        if libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut total).cast(), &mut size, std::ptr::null_mut(), 0) != 0
        {
            return None;
        }
        let page = u64::try_from(libc::sysconf(libc::_SC_PAGESIZE)).ok()?;
        // in use: what programs hold, what the system itself can't give back, and what it keeps compressed
        let used = (u64::from(vm.active_count) + u64::from(vm.wire_count) + u64::from(vm.compressor_page_count)) * page;
        share(used.min(total), total)
    }
}

#[cfg(target_vendor = "apple")]
fn temperature() -> Option<f64> {
    None // Apple's systems give a program none
}

#[cfg(target_vendor = "apple")]
fn battery() -> (Option<f64>, Option<bool>) {
    (None, None) // a phone's comes from its shell (`told`)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_systems_own_lines_are_read_as_what_they_say() {
        // the processor: busy is everything but idle and waiting for a disk
        assert_eq!(cpu_line("cpu  100 0 50 800 50 0 0 0 0 0"), Some((150, 1000)));
        assert_eq!(cpu_line("cpu 1 2 3 4"), Some((6, 10)));
        for bad in ["", "cpu0 1 2 3 4", "cpu 1 2 3", "cpu a b c d", "intr 5 6 7 8"] {
            assert_eq!(cpu_line(bad), None, "{bad}");
        }
        assert_eq!(share(150, 1000), Some(15.0));
        assert_eq!((share(0, 0), share(5, 4)), (None, None));
        // the memory: what isn't available, of all there is
        let meminfo = "MemTotal:        8000000 kB\nMemFree:          100000 kB\nMemAvailable:    6000000 kB\n";
        assert_eq!(memory_from(meminfo), Some(25.0));
        assert_eq!(memory_from("MemTotal: 10 kB\n"), None);
        assert_eq!(memory_from("MemTotal: 10 kB\nMemAvailable: 20 kB\n"), None);
        // a thermal zone: thousandths of a degree, and only what can be a temperature
        assert_eq!(degrees("48312\n"), Some(48.312));
        for not in ["0", "-5000", "999999", "warm", ""] {
            assert_eq!(degrees(not), None, "{not}");
        }
    }

    #[test]
    fn a_reading_has_every_name_and_only_numbers_this_system_gave() {
        let first = reading();
        std::thread::sleep(Duration::from_millis(120));
        let r = reading();
        for name in ["load", "memory", "temp", "heat", "battery", "charging"] {
            assert!(r.get(name).is_some() && first.get(name).is_some(), "{name} is missing: {r}");
        }
        let within = |name: &str, lo: f64, hi: f64| {
            r[name].is_null() || r[name].as_f64().is_some_and(|v| (lo..=hi).contains(&v))
        };
        assert!(within("load", 0.0, 100.0) && within("memory", 0.0, 100.0), "{r}");
        assert!(within("temp", 1.0, 150.0) && within("battery", 0.0, 100.0), "{r}");
        assert!(r["charging"].is_null() || r["charging"].is_boolean(), "{r}");
        // the systems the tests run on all say how much of their memory is in use; the second asking has a load
        assert!(r["memory"].is_number(), "{r}");
        // (the load is what passed since the last asking by anyone: another test asking at the same moment
        // leaves nothing in between, so it is asked for a few times)
        if cfg!(any(windows, target_os = "linux", target_os = "macos")) {
            let got = (0..10).any(|_| {
                std::thread::sleep(Duration::from_millis(150));
                reading()["load"].as_f64().is_some_and(|load| (0.0..=100.0).contains(&load))
            });
            assert!(got, "this system gave no load");
        }
    }

    #[test]
    fn what_a_phones_shell_says_is_checked_and_stands_for_a_while() {
        told(2, 81.4, 1);
        let r = reading();
        assert_eq!(r["heat"], 2);
        // (a computer's own battery, where it has one, comes first)
        if battery() == (None, None) {
            assert_eq!((r["battery"].as_f64(), r["charging"].as_bool()), (Some(81.0), Some(true)));
        }
        // what can't be a heat state, a battery level or a yes or no is not taken for one
        told(9, 250.0, 7);
        let r = reading();
        assert!(r["heat"].is_null(), "{r}");
        if battery() == (None, None) {
            assert!(r["battery"].is_null() && r["charging"].is_null(), "{r}");
        }
        told(-1, f32::NAN, -3);
        assert!(reading()["heat"].is_null());
        *TOLD.lock().unwrap() = None;
    }

    #[test]
    fn the_facts_are_short_lines_of_text() {
        let f = facts("");
        for name in ["device", "system", "app", "processor", "memory"] {
            assert!(f[name].as_str().is_some_and(|t| t.len() <= 200), "{name}: {f}");
        }
        assert!(f["app"].as_str().unwrap().starts_with("Face Recognizer "));
        assert_eq!(f["device"], "a computer");
    }

    #[test]
    fn a_phone_says_what_it_is_only_as_far_as_its_system_said() {
        let all = phone_says("iPhone18,2", Some(6), Some(12 << 30));
        assert_eq!(
            all,
            ("a phone, iPhone18,2".to_string(), "6 processor cores".to_string(), "12 GB of memory".to_string())
        );
        // a system reports a little less memory than the phone is sold with: said as the nearest whole number
        assert_eq!(phone_says("Google Pixel 10", Some(8), Some(15_900_000_000)).2, "15 GB of memory");
        assert_eq!(phone_says("iPad16,3", None, None).0, "a tablet, iPad16,3");
        // nothing said: nothing made up
        assert_eq!(phone_says("", None, None), ("a phone".to_string(), String::new(), String::new()));
        assert_eq!(phone_says("x", Some(0), Some(0)), ("a phone, x".to_string(), String::new(), String::new()));
        // a model's name is a line of text, whatever a system puts there
        let odd = phone_says(&format!("Model\n\u{7}{}", "y".repeat(300)), None, None).0;
        assert!(odd.len() <= 9 + 60 && !odd.contains('\n') && !odd.contains('\u{7}'), "{odd}");
    }
}
