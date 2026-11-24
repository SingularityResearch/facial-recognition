//! What this computer has, and how the programs use it (what the
//! Settings tab shows under This computer): the processor, memory, graphics processors and NPU, read from the
//! system itself (the Windows registry, Linux's /proc and /sys, macOS's sysctl), so it takes milliseconds;
//! where each model runs; this computer's own network addresses; and the AI model apps that could describe
//! photos (the usual ports on this computer and the server set in Settings: nothing else on the network).
//!
//! Not here yet: the search for LM Studio servers on the local network.

// (each system's branch returns what it found: only one of them is built, so none is "the last expression")
#![allow(clippy::needless_return)]

// (some of these are for one system's branch, or for the parts built with the importer)
#[allow(unused_imports)]
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[cfg(feature = "import")]
use face_home::describe::config;
#[allow(unused_imports)]
use serde_json::{Map, Value, json};

#[cfg(feature = "import")]
use crate::describe::server_models;

/// Processor threads this program may use.
pub fn cpu_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// What a system tool says, or "".
#[cfg_attr(windows, allow(dead_code))]
fn run(program: &str, args: &[&str]) -> String {
    let mut ask = Command::new(program);
    ask.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        ask.creation_flags(0x0800_0000); // no console window
    }
    ask.output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(String::new, |o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

#[cfg(windows)]
mod registry {
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// A text value under a key of HKEY_LOCAL_MACHINE (`sub`: a subkey's name as the system gave it, or none).
    fn text(key: HKEY, sub: *const u16, name: &str) -> Option<String> {
        let name = wide(name);
        let mut size = 0u32;
        // SAFETY: the first call asks for the value's size in bytes, the second writes at most that many into a
        // buffer of that size; `key` is open and `sub` is null or ends with a zero
        unsafe {
            if RegGetValueW(key, sub, name.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut size) != 0 {
                return None;
            }
            let mut buf = vec![0u16; size as usize / 2 + 1];
            let mut size = (buf.len() * 2) as u32;
            if RegGetValueW(key, sub, name.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr().cast(), &mut size) != 0
            {
                return None;
            }
            let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end]))
        }
    }

    fn open(key: &str) -> Option<HKEY> {
        let mut handle: HKEY = null_mut();
        // SAFETY: the key's name ends with a zero, and the handle is written only on success
        (unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wide(key).as_ptr(), 0, KEY_READ, &mut handle) } == 0)
            .then_some(handle)
    }

    /// `name` under each subkey of HKLM\key (every display adapter's DriverDesc, say), in the system's order.
    pub fn values(key: &str, name: &str) -> Vec<String> {
        let Some(handle) = open(key) else { return Vec::new() };
        let mut out = Vec::new();
        for i in 0.. {
            let mut sub = [0u16; 256];
            let mut len = sub.len() as u32;
            // SAFETY: the system writes at most `len` units of the subkey's name, with its zero, into `sub`
            if unsafe {
                RegEnumKeyExW(handle, i, sub.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut())
            } != 0
            {
                break;
            }
            out.extend(text(handle, sub.as_ptr(), name));
        }
        // SAFETY: the handle was opened above and is not used again
        unsafe { RegCloseKey(handle) };
        out
    }

    /// One text value of HKLM\key.
    pub fn value(key: &str, name: &str) -> Option<String> {
        let handle = open(key)?;
        let said = text(handle, null(), name);
        // SAFETY: as above
        unsafe { RegCloseKey(handle) };
        said
    }
}

/// The kinds of core in Arm's own designs, by the "CPU part" of /proc/cpuinfo.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const ARM_CORES: [(&str, &str); 16] = [
    ("0xd03", "Cortex-A53"),
    ("0xd04", "Cortex-A35"),
    ("0xd05", "Cortex-A55"),
    ("0xd07", "Cortex-A57"),
    ("0xd08", "Cortex-A72"),
    ("0xd09", "Cortex-A73"),
    ("0xd0a", "Cortex-A75"),
    ("0xd0b", "Cortex-A76"),
    ("0xd0d", "Cortex-A77"),
    ("0xd41", "Cortex-A78"),
    ("0xd44", "Cortex-X1"),
    ("0xd46", "Cortex-A510"),
    ("0xd47", "Cortex-A710"),
    ("0xd48", "Cortex-X2"),
    ("0xd4d", "Cortex-A715"),
    ("0xd4e", "Cortex-X3"),
];

/// A name for an ARM Linux computer's processor, whose /proc/cpuinfo has no "model name" line: the board's
/// own name when it has one (a Raspberry Pi says "Raspberry Pi 4 Model B Rev 1.4") with the kind of core.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn arm_cpu_name(cpuinfo: &str, board: &str) -> Option<String> {
    let (mut part, mut maker) = (None, None);
    for line in cpuinfo.lines() {
        let (key, value) = line.split_once(':').unwrap_or((line, ""));
        let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
        if key == "cpu part" && part.is_none() {
            part = Some(value);
        } else if key == "cpu implementer" && maker.is_none() {
            maker = Some(value);
        }
    }
    let core = part.map(|part| match maker.as_deref() {
        Some("0x41") | None => ARM_CORES
            .iter()
            .find(|(p, _)| *p == part)
            .map_or_else(|| format!("ARM core {part}"), |(_, name)| name.to_string()),
        Some(maker) => format!("ARM core {part} (maker {maker})"),
    });
    let board = board.replace('\0', "").trim().to_string();
    match (board.is_empty(), core) {
        (false, Some(core)) => Some(format!("{board} ({core})")),
        (false, None) => Some(board),
        (true, core) => core,
    }
}

pub fn cpu_name() -> String {
    #[cfg(windows)]
    if let Some(name) = registry::values(r"HARDWARE\DESCRIPTION\System\CentralProcessor", "ProcessorNameString").first()
    {
        return name.trim().to_string();
    }
    #[cfg(target_os = "linux")]
    if let Ok(text) = std::fs::read(Path::new("/proc/cpuinfo")).map(|b| String::from_utf8_lossy(&b).into_owned()) {
        if let Some(line) = text.lines().find(|l| l.starts_with("model name")) {
            return line.split_once(':').map_or("", |(_, name)| name).trim().to_string();
        }
        let board = std::fs::read("/proc/device-tree/model")
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        if let Some(name) = arm_cpu_name(&text, &board) {
            return name;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let name = run("sysctl", &["-n", "machdep.cpu.brand_string"]);
        if !name.is_empty() {
            return name;
        }
    }
    "unknown processor".to_string()
}

/// This computer's memory in GB, rounded.
pub fn memory_gb() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: the structure says its own size, and the system fills it
        let mut st: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        st.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut st) } != 0 {
            return Some((st.ullTotalPhys as f64 / (1u64 << 30) as f64).round_ties_even() as u64);
        }
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: f64 = text.lines().find(|l| l.starts_with("MemTotal"))?.split_whitespace().nth(1)?.parse().ok()?;
        return Some((kb / (1u64 << 20) as f64).round_ties_even() as u64);
    }
    #[cfg(target_os = "macos")]
    {
        let bytes: f64 = run("sysctl", &["-n", "hw.memsize"]).parse().unwrap_or(0.0);
        return Some((bytes / (1u64 << 30) as f64).round_ties_even() as u64);
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    None
}

/// The graphics processors' names, the separate ones first where the system says.
pub fn gpus() -> Vec<String> {
    #[cfg(windows)]
    {
        let mut out: Vec<String> = Vec::new();
        for name in registry::values(
            r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}",
            "DriverDesc",
        ) {
            if !out.contains(&name) && !name.contains("Basic") && !name.contains("Remote") {
                out.push(name);
            }
        }
        return out;
    }
    #[cfg(target_os = "macos")]
    {
        let chip = run("sysctl", &["-n", "machdep.cpu.brand_string"]);
        return vec![if chip.starts_with("Apple") {
            format!("{chip} GPU")
        } else {
            "(see System Information)".to_string()
        }];
    }
    #[cfg(target_os = "linux")]
    {
        let said = run("nvidia-smi", &["--query-gpu=name", "--format=csv,noheader"]);
        let mut nvidia: std::collections::VecDeque<&str> = said.lines().filter(|n| !n.is_empty()).collect();
        let mut devices: Vec<PathBuf> =
            std::fs::read_dir("/sys/bus/pci/devices").into_iter().flatten().flatten().map(|d| d.path()).collect();
        devices.sort();
        let mut out = Vec::new();
        for dev in devices {
            let read = |name: &str| std::fs::read_to_string(dev.join(name)).ok();
            // display controllers
            let (Some(class), Some(vendor)) = (read("class"), read("vendor")) else { continue };
            if !class.starts_with("0x03") {
                continue;
            }
            let vendor = match vendor.trim() {
                "0x10de" => "NVIDIA",
                "0x1002" => "AMD",
                "0x8086" => "Intel",
                "0x5143" => "Qualcomm",
                _ => "Other",
            };
            match nvidia.pop_front().filter(|_| vendor == "NVIDIA") {
                Some(name) => out.push(format!("NVIDIA {}", name.strip_prefix("NVIDIA ").unwrap_or(name))),
                None => {
                    out.push(format!("{vendor} graphics ({})", dev.file_name().unwrap_or_default().to_string_lossy()))
                }
            }
        }
        return out;
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    Vec::new()
}

/// The neural processing unit, if the system lists one.
pub fn npu() -> Option<String> {
    #[cfg(windows)]
    {
        // the "Neural processors" (compute accelerator) class
        return registry::values(
            r"SYSTEM\CurrentControlSet\Control\Class\{f01a9d53-3ff6-48d2-9f97-c8a7004be10c}",
            "DriverDesc",
        )
        .into_iter()
        .next();
    }
    #[cfg(target_os = "linux")]
    {
        let mut found: Vec<PathBuf> = (std::fs::read_dir("/sys/class/accel").into_iter().flatten().flatten())
            .map(|d| d.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("accel")))
            .collect();
        found.sort();
        let dev = found.first()?;
        let name = dev.file_name().unwrap_or_default().to_string_lossy().into_owned();
        return Some(match std::fs::read_link(dev.join("device").join("driver")) {
            Ok(driver) => format!("{} ({name})", driver.file_name().unwrap_or_default().to_string_lossy()),
            Err(_) => name,
        });
    }
    #[cfg(target_os = "macos")]
    {
        return run("sysctl", &["-n", "machdep.cpu.brand_string"])
            .starts_with("Apple")
            .then(|| "Apple Neural Engine (reached through CoreML)".to_string());
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    None
}

/// The system, its release and the kind of processor ("Windows 11 (AMD64)").
pub fn system() -> String {
    #[cfg(windows)]
    {
        let build = registry::value(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
            .and_then(|b| b.trim().parse::<u32>().ok());
        let release = match build {
            Some(b) if b >= 22000 => "11",
            _ => "10",
        };
        let machine = std::env::var("PROCESSOR_ARCHITECTURE").unwrap_or_else(|_| std::env::consts::ARCH.to_string());
        return format!("Windows {release} ({machine})");
    }
    #[cfg(target_os = "linux")]
    {
        let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        return format!("Linux {} ({})", release.trim(), std::env::consts::ARCH);
    }
    #[cfg(target_os = "macos")]
    {
        let machine = if cfg!(target_arch = "aarch64") { "arm64" } else { std::env::consts::ARCH };
        return format!("Darwin {} ({machine})", run("uname", &["-r"]));
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

/// The ways this program's model runtime can run a model on this kind of system, as ONNX Runtime names them.
pub fn providers() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["Dml", "CPU"]
    } else if cfg!(any(target_os = "macos", target_os = "ios")) {
        vec!["CoreML", "CPU"]
    } else if cfg!(target_os = "android") {
        vec!["Nnapi", "XNNPACK", "CPU"]
    } else {
        vec!["CPU"]
    }
}

/// What this computer has (kept: it doesn't change while the program runs).
pub fn describe() -> Value {
    static KEPT: OnceLock<Value> = OnceLock::new();
    (KEPT.get_or_init(|| {
        json!({"system": system(), "cpu": cpu_name(),
               "threads": cpu_threads(), "memory_gb": memory_gb(), "gpus": gpus(), "npu": npu(), "onnxruntime": providers()})
    }))
    .clone()
}

#[cfg(feature = "import")]
/// Why a graphics card that's here isn't used, when this program can't run models on it (else "").
fn gpu_note() -> &'static str {
    let has = describe();
    let gpus: Vec<&str> = has["gpus"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if providers().len() > 1 || gpus.is_empty() {
        return "";
    }
    if gpus.iter().any(|g| g.starts_with("NVIDIA")) {
        return "An NVIDIA card is here, but this program has no way to run models on it yet (no CUDA), so everything runs on the processor.";
    }
    "No way to run models on this computer's graphics card was found, so everything runs on the processor."
}

#[cfg(feature = "import")]
/// What the programs chose on this computer, for each job. `devices`: where each loaded model runs.
///
/// `theirs`: the same of the tracker's models, when it runs in a process of its own (`trackerlink::devices`).
pub fn uses(devices: &[(&str, Option<String>)], theirs: &(Map<String, Value>, Map<String, Value>)) -> Value {
    let mut devices: Map<String, Value> =
        devices.iter().filter_map(|(job, at)| Some((job.to_string(), json!(at.as_ref()?)))).collect();
    let mut decisions: Map<String, Value> =
        face_core::runtime::decisions().into_iter().map(|(model, why)| (model, json!(why))).collect();
    devices.extend(theirs.0.clone());
    decisions.extend(theirs.1.clone());
    let setting = match std::env::var("FACE_DEVICE").unwrap_or_default().to_lowercase().as_str() {
        "cpu" => "cpu",
        "gpu" => "gpu",
        _ => "auto",
    };
    json!({"device_setting": setting, "devices": devices, "decisions": decisions, "import_workers": face_import::importer::import_workers(),
           "photos": "decoded on the processor (JPEG straight at reduced size), read once each",
           "videos": "decoded on the processor, about one frame a second looked at",
           "npu_note": "not used: measured slower than the GPU and the processor for one face at a time",
           "gpu_note": gpu_note(), "cuda": "", "cuda_offer": {"offer": false}})
}

// ----------------------------------------------------------------------------- this computer's addresses

/// This device's "Fully distributed system" setting (Settings, LM Studio; `HOME/lmstudio.json`, off for a
/// new copy): the owner's computers may be anywhere, so every network adapter counts as the device's own.
pub fn distributed(home: &Path) -> bool {
    let kept: Option<Value> =
        std::fs::read(home.join("lmstudio.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    kept.is_some_and(|k| k.get("distributed") == Some(&Value::Bool(true)))
}

/// What kind of adapter an address is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Adapter {
    Real,
    /// a VPN tunnel, a virtual switch (Hyper-V, WSL, VirtualBox)
    Virtual,
    /// a bridge the computer keeps for its own containers and virtual machines: no other machine reaches it
    Container,
}

/// Linux names of adapters that aren't the computer's real network ports, and of those the container bridges.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const VIRTUAL: [&str; 12] =
    ["lo", "docker", "br-", "veth", "virbr", "tun", "tap", "wg", "zt", "tailscale", "CloudflareWARP", "vEthernet"];
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const CONTAINER: [&str; 8] = ["docker", "br-", "veth", "virbr", "cni", "podman", "lxc", "lxd"];
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MAC_VIRTUAL: [&str; 13] =
    ["lo", "utun", "bridge", "vmnet", "awdl", "llw", "tap", "tun", "feth", "ipsec", "ppp", "gif", "stf"];

/// What an adapter of this name and this address is, by the system's rules of naming.
#[cfg_attr(windows, allow(dead_code))]
fn kind_by_name(name: &str, whole_mask: bool) -> Adapter {
    if cfg!(target_os = "macos") {
        return if MAC_VIRTUAL.iter().any(|v| name.starts_with(v)) { Adapter::Virtual } else { Adapter::Real };
    }
    if CONTAINER.iter().any(|c| name.starts_with(c)) {
        Adapter::Container
    } else if VIRTUAL.iter().any(|v| name.starts_with(v)) || whole_mask {
        Adapter::Virtual
    } else {
        Adapter::Real
    }
}

/// Every IPv4 address of this computer with the kind of its adapter, or None when the system doesn't say.
#[cfg(windows)]
fn adapters() -> Option<Vec<(Ipv4Addr, Adapter)>> {
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::{
            GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
            GetIfEntry2, IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2,
        },
        Networking::WinSock::{AF_INET, SOCKADDR_IN},
    };
    const LOOPBACK: u32 = 24; // IF_TYPE_SOFTWARE_LOOPBACK
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size = 16_384u32;
    let mut buf: Vec<u64> = Vec::new();
    for _ in 0..3 {
        buf = vec![0u64; size as usize / 8 + 1];
        // SAFETY: the system writes at most `size` bytes of adapters into the buffer, or says how many it needs
        match unsafe {
            GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size)
        } {
            0 => break,
            111 => continue, // ERROR_BUFFER_OVERFLOW: `size` is what it needs
            _ => return None,
        }
    }
    let mut out = Vec::new();
    let mut at = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    // SAFETY: the list the system wrote: each entry and each of its addresses points at the next or at nothing,
    // all inside `buf`, which lives to the end of this function
    unsafe {
        while !at.is_null() {
            let adapter = &*at;
            at = adapter.Next;
            if adapter.IfType == LOOPBACK {
                continue;
            }
            // (a real network port is hardware: a VPN's, a virtual switch's and a container's adapter is not)
            let mut row: MIB_IF_ROW2 = std::mem::zeroed();
            row.InterfaceLuid = adapter.Luid;
            let hardware = GetIfEntry2(&mut row) == 0 && row.InterfaceAndOperStatusFlags._bitfield & 1 == 1;
            let mut address = adapter.FirstUnicastAddress;
            while !address.is_null() {
                let sock = (*address).Address.lpSockaddr;
                if !sock.is_null() && (*sock).sa_family == AF_INET {
                    let ip = (*sock.cast::<SOCKADDR_IN>()).sin_addr.S_un.S_addr.to_ne_bytes();
                    out.push((Ipv4Addr::from(ip), if hardware { Adapter::Real } else { Adapter::Virtual }));
                }
                address = (*address).Next;
            }
        }
    }
    Some(out)
}

#[cfg(unix)]
fn adapters() -> Option<Vec<(Ipv4Addr, Adapter)>> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: the system hands out a list that is read below and given back with freeifaddrs
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut at = list;
    // SAFETY: each entry points at the next or at nothing; names end with a zero; an address of the IPv4
    // family is a sockaddr_in
    unsafe {
        while !at.is_null() {
            let entry = &*at;
            at = entry.ifa_next;
            if entry.ifa_addr.is_null() || (*entry.ifa_addr).sa_family as i32 != libc::AF_INET {
                continue;
            }
            let ip = Ipv4Addr::from((*entry.ifa_addr.cast::<libc::sockaddr_in>()).sin_addr.s_addr.to_ne_bytes());
            let whole = !entry.ifa_netmask.is_null()
                && (*entry.ifa_netmask.cast::<libc::sockaddr_in>()).sin_addr.s_addr == u32::MAX;
            let name = std::ffi::CStr::from_ptr(entry.ifa_name).to_string_lossy().into_owned();
            // (Linux: only addresses other machines can be asked at, as `ip addr show scope global` lists them)
            if cfg!(target_os = "linux") && (ip.is_loopback() || ip.is_link_local()) {
                continue;
            }
            out.push((ip, kind_by_name(&name, whole && cfg!(target_os = "linux"))));
        }
        libc::freeifaddrs(list);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(not(any(windows, unix)))]
fn adapters() -> Option<Vec<(Ipv4Addr, Adapter)>> {
    None
}

/// This computer's own addresses on the local network, by the route it takes toward a private address
/// (opening a UDP socket toward one sends nothing; it only picks the route).
fn lan_addresses() -> Vec<Ipv4Addr> {
    let mut found = Vec::new();
    for probe in ["10.255.255.255:1", "192.168.255.255:1", "172.31.255.255:1"] {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else { continue };
        if socket.connect(probe).is_err() {
            continue;
        }
        if let Ok(SocketAddr::V4(here)) = socket.local_addr()
            && !here.ip().is_loopback()
            && !found.contains(here.ip())
        {
            found.push(*here.ip());
        }
    }
    found
}

/// Every IPv4 address of this computer's real network adapters (each network port, Wi-Fi...): not loopback,
/// and not VPN tunnels, virtual switches or containers unless `every` (default: the "Fully distributed
/// system" setting). When the system doesn't say which adapter an address is on, the addresses of the
/// computer's name are taken as they are.
pub fn own_addresses(home: &Path, every: Option<bool>) -> Vec<Ipv4Addr> {
    let every = every.unwrap_or_else(|| distributed(home));
    let mut found: Vec<Ipv4Addr> = match adapters() {
        Some(known) => {
            known.into_iter().filter(|(_, kind)| every || *kind == Adapter::Real).map(|(ip, _)| ip).collect()
        }
        None => {
            let mut found = lan_addresses();
            let named = crate::host_name().and_then(|name| (name.as_str(), 0).to_socket_addrs().ok());
            found.extend(named.into_iter().flatten().filter_map(|a| match a.ip() {
                IpAddr::V4(ip) => Some(ip),
                IpAddr::V6(_) => None,
            }));
            found
        }
    };
    found.retain(|ip| !ip.is_loopback() && !ip.is_link_local());
    found.sort();
    found.dedup();
    found
}

/// `own_addresses` without those no other machine can reach: this computer's container and virtual-machine
/// bridges, which "Fully distributed system" otherwise counts. What a device tells the others (the peer
/// service's part of the app, which isn't here yet).
#[allow(dead_code)]
pub fn reachable_addresses(home: &Path, every: Option<bool>) -> Vec<Ipv4Addr> {
    let local: Vec<Ipv4Addr> =
        adapters().into_iter().flatten().filter(|(_, kind)| *kind == Adapter::Container).map(|(ip, _)| ip).collect();
    own_addresses(home, every).into_iter().filter(|ip| !local.contains(ip)).collect()
}

// ----------------------------------------------------------------------------- AI model apps (for Describe photo)

#[cfg(feature = "import")]
const AI_PORTS: [(u16, &str); 7] = [
    (1234, "LM Studio"),
    (11434, "Ollama"),
    (1337, "Jan"),
    (8080, "llama.cpp / LocalAI"),
    (8000, "vLLM"),
    (5000, "text-generation-webui"),
    (4891, "GPT4All"),
];

#[cfg(feature = "import")]
fn port_open(ip: Ipv4Addr, port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from((ip, port)), Duration::from_millis(250)).is_ok()
}

#[cfg(feature = "import")]
fn user_home() -> PathBuf {
    PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
}

#[cfg(feature = "import")]
/// AI model apps installed on this computer, running or not (where each normally installs itself).
fn installed_ai_apps() -> Vec<&'static str> {
    let home = user_home();
    let local = std::env::var_os("LOCALAPPDATA").map_or_else(|| home.join("AppData").join("Local"), PathBuf::from);
    let roaming = std::env::var_os("APPDATA").map_or_else(|| home.clone(), PathBuf::from);
    let places: [(&str, Vec<PathBuf>); 3] = [
        (
            "LM Studio",
            vec![
                home.join(".lmstudio"),
                home.join(".cache").join("lm-studio"),
                local.join("Programs").join("LM Studio"),
                local.join("Programs").join("lm-studio"),
                PathBuf::from("/Applications/LM Studio.app"),
            ],
        ),
        ("Jan", vec![home.join("jan"), roaming.join("Jan"), PathBuf::from("/Applications/Jan.app")]),
        ("GPT4All", vec![local.join("nomic.ai").join("GPT4All"), PathBuf::from("/Applications/GPT4All.app")]),
    ];
    let mut found: Vec<&str> =
        places.iter().filter(|(_, at)| at.iter().any(|p| p.exists())).map(|(name, _)| *name).collect();
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| ["ollama", "ollama.exe"].iter().any(|name| dir.join(name).is_file()))
    });
    if on_path
        || (cfg!(windows) && local.join("Programs").join("Ollama").exists())
        || Path::new("/Applications/Ollama.app").exists()
    {
        found.push("Ollama");
    }
    found
}

#[cfg(feature = "import")]
/// LM Studio's downloaded models (its models folder, from its settings.json): (name, reads pictures), a
/// vision model being one with an mmproj (picture projector) file beside it. None when LM Studio isn't there.
fn lmstudio_downloads() -> Vec<(String, bool)> {
    let home = user_home().join(".lmstudio");
    let settings: Option<Value> =
        std::fs::read(home.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let named =
        settings.as_ref().and_then(|s| s.get("downloadsFolder")).and_then(Value::as_str).filter(|f| !f.is_empty());
    let folder = named.map_or_else(|| home.join("models"), PathBuf::from);
    let folders = |of: &Path| {
        let mut found: Vec<PathBuf> =
            std::fs::read_dir(of).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        found.sort();
        found
    };
    let name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut found = Vec::new();
    for publisher in folders(&folder) {
        for model in folders(&publisher) {
            let sees = std::fs::read_dir(&model)
                .into_iter()
                .flatten()
                .flatten()
                .any(|f| f.file_name().to_string_lossy().to_lowercase().contains("mmproj"));
            found.push((format!("{}/{}", name(&publisher), name(&model)), sees));
        }
    }
    found
}

#[cfg(feature = "import")]
/// The AI model apps that could describe photos: [{app, url, running, models, vision, note}]. Asks the usual
/// ports on this computer and the Describe photo server set in Settings (which may be another device), each
/// with a short time limit; nothing else on the network. Kept for a minute.
pub fn ai_apps(home: &Path) -> Value {
    static KEPT: Mutex<Option<(Instant, PathBuf, Value)>> = Mutex::new(None);
    if let Some((at, of, found)) = &*KEPT.lock().unwrap_or_else(|e| e.into_inner())
        && at.elapsed() < Duration::from_secs(60)
        && of == home
    {
        return found.clone();
    }
    let c = config(home);
    let (mut out, mut seen): (Vec<Value>, Vec<String>) = (Vec::new(), Vec::new());
    // Lists what answers at an address as an app. A usual port (not the server set in Settings) is listed only
    // when a model server answers there, with its models or asking for a key (401): the same ports are other
    // programs' too (macOS's own AirPlay Receiver has 5000 and answers 403).
    let probe = |app: &str, url: &str, from_settings: bool, out: &mut Vec<Value>| -> bool {
        let mut entry = json!({"app": app, "url": url, "running": true, "models": 0, "vision": [], "note": ""});
        match server_models(&c, (!from_settings).then_some((url, ""))) {
            Ok((found, loaded)) => {
                entry["models"] = json!(found.len());
                entry["vision"] = json!(
                    found.iter().filter(|(_, sees)| *sees == Some(true)).map(|(m, _)| m).take(6).collect::<Vec<_>>()
                );
                if !loaded.is_empty() {
                    entry["note"] = json!(format!("loaded: {}", loaded[..loaded.len().min(3)].join(", ")));
                }
            }
            Err(e) => {
                if !from_settings && e.code != Some(401) {
                    return false;
                }
                let local =
                    url.strip_prefix("http://127.0.0.1:").and_then(|rest| rest.split('/').next()?.parse::<u16>().ok());
                entry["running"] = json!(local.is_some_and(|port| port_open(Ipv4Addr::LOCALHOST, port)));
                entry["note"] = json!(e.why.chars().take(200).collect::<String>());
            }
        }
        out.push(entry);
        true
    };
    for (port, app) in AI_PORTS {
        let url = format!("http://127.0.0.1:{port}");
        if port_open(Ipv4Addr::LOCALHOST, port) && probe(app, &url, false, &mut out) {
            seen.push(url);
            // served to the network too? then other devices (Describe photo on them) can use this address
            let lan: Vec<String> = own_addresses(home, None)
                .into_iter()
                .filter(|ip| port_open(*ip, port))
                .map(|ip| format!("http://{ip}:{port}"))
                .collect();
            if let Some(last) = out.last_mut() {
                last["lan"] = json!(lan);
            }
        }
    }
    if !seen.contains(&c.url) {
        let app = AI_PORTS
            .iter()
            .find(|(port, _)| c.url.ends_with(&format!(":{port}")))
            .map_or("Describe photo server", |(_, app)| app);
        probe(app, &c.url, true, &mut out);
        if let Some(last) = out.last_mut() {
            let note = last["note"].as_str().unwrap_or("").to_string();
            last["note"] =
                json!(if note.is_empty() { "set in Settings".to_string() } else { format!("set in Settings; {note}") });
        }
    }
    for app in installed_ai_apps() {
        let same: Vec<usize> =
            (0..out.len()).filter(|k| out[*k]["app"].as_str().is_some_and(|a| a.contains(app))).collect();
        if same.iter().any(|k| out[*k]["running"] == true) {
            continue;
        }
        let at = same.first().copied().unwrap_or_else(|| {
            out.push(json!({"app": app, "url": null, "running": false, "models": 0, "vision": [], "note": ""}));
            out.len() - 1
        });
        let mut note = "installed, not running (start it, and its local server)".to_string();
        if app == "LM Studio" {
            let disk = lmstudio_downloads();
            let vision: Vec<&String> = disk.iter().filter(|(_, sees)| *sees).map(|(m, _)| m).take(6).collect();
            note += &format!(
                "; {} model{} downloaded{}",
                disk.len(),
                if disk.len() == 1 { "" } else { "s" },
                if vision.is_empty() { ", none of them reads pictures" } else { "" }
            );
            out[at]["models"] = json!(disk.len());
            out[at]["vision"] = json!(vision);
        }
        out[at]["note"] = json!(note);
    }
    let found = Value::Array(out);
    *KEPT.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), home.to_path_buf(), found.clone()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arm_processor_is_named_by_its_board_and_core() {
        let pi = "processor\t: 0\nBogoMIPS\t: 108.00\nCPU implementer\t: 0x41\nCPU part\t: 0xd08\n\nprocessor\t: 1\nCPU part\t: 0xd0b\n";
        assert_eq!(
            arm_cpu_name(pi, "Raspberry Pi 4 Model B Rev 1.4\0").as_deref(),
            Some("Raspberry Pi 4 Model B Rev 1.4 (Cortex-A72)")
        );
        assert_eq!(arm_cpu_name(pi, "").as_deref(), Some("Cortex-A72"));
        assert_eq!(arm_cpu_name("CPU implementer : 0x41\nCPU part : 0xfff\n", "").as_deref(), Some("ARM core 0xfff"));
        assert_eq!(
            arm_cpu_name("CPU implementer : 0x51\nCPU part : 0x804\n", "A board").as_deref(),
            Some("A board (ARM core 0x804 (maker 0x51))")
        );
        assert_eq!(arm_cpu_name("model name : something\n", " A board \n").as_deref(), Some("A board"));
        assert_eq!(arm_cpu_name("", ""), None);
    }

    #[test]
    fn this_computer_says_what_it_has() {
        let has = describe();
        assert!(
            has["threads"].as_u64().unwrap() >= 1
                && !has["cpu"].as_str().unwrap().is_empty()
                && has["onnxruntime"].as_array().unwrap().contains(&json!("CPU"))
        );
        if cfg!(any(windows, target_os = "linux", target_os = "macos")) {
            assert!(has["memory_gb"].as_u64().unwrap() >= 1 && has["cpu"] != "unknown processor", "{has}");
            assert!(has["system"].as_str().unwrap().split(' ').count() >= 3);
        }
    }

    #[cfg(feature = "import")]
    #[test]
    fn where_the_work_runs_is_said_for_the_models_that_are_loaded() {
        let mine = [("detection", Some("CPU".to_string())), ("quality", None)];
        let none = (Map::new(), Map::new());
        let told = uses(&mine, &none);
        assert_eq!((&told["devices"], &told["cuda_offer"]["offer"]), (&json!({"detection": "CPU"}), &json!(false)));
        assert!(told["import_workers"].as_u64().unwrap() >= 2);
        // the tracker's models, when it runs in a process of its own: its word for its jobs beside this one's
        let theirs = json!({"devices": {"detection": "GPU (DirectML)", "expression": "CPU"}, "decisions": {"a.onnx": "checked"}});
        let theirs = (theirs["devices"].as_object().unwrap().clone(), theirs["decisions"].as_object().unwrap().clone());
        let told = uses(&mine, &theirs);
        assert_eq!(told["devices"], json!({"detection": "GPU (DirectML)", "expression": "CPU"}));
        assert_eq!(told["decisions"]["a.onnx"], "checked");
    }

    #[test]
    fn adapters_are_sorted_out_by_their_names_and_the_setting_is_read() {
        if cfg!(target_os = "macos") {
            assert_eq!(
                (kind_by_name("en0", false), kind_by_name("utun3", false), kind_by_name("bridge100", false)),
                (Adapter::Real, Adapter::Virtual, Adapter::Virtual)
            );
        } else {
            assert_eq!(
                (kind_by_name("eth0", false), kind_by_name("wlan0", false), kind_by_name("enp5s0", true)),
                (Adapter::Real, Adapter::Real, Adapter::Virtual)
            );
            assert_eq!(
                (kind_by_name("docker0", false), kind_by_name("br-1a2b", false), kind_by_name("veth9", false)),
                (Adapter::Container, Adapter::Container, Adapter::Container)
            );
            assert_eq!(
                (kind_by_name("tailscale0", false), kind_by_name("wg0", false), kind_by_name("tun0", false)),
                (Adapter::Virtual, Adapter::Virtual, Adapter::Virtual)
            );
        }
        let home = std::env::temp_dir().join(format!("face-hardware-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(!distributed(&home));
        std::fs::write(home.join("lmstudio.json"), r#"{"distributed": "yes"}"#).unwrap();
        assert!(!distributed(&home));
        std::fs::write(home.join("lmstudio.json"), r#"{"share": "", "distributed": true}"#).unwrap();
        assert!(distributed(&home));
        // this computer's own addresses: never loopback, the real adapters' among every adapter's, in order
        let (real, every) = (own_addresses(&home, Some(false)), own_addresses(&home, Some(true)));
        assert!(
            real.iter().all(|ip| every.contains(ip) && !ip.is_loopback() && !ip.is_link_local()),
            "{real:?} {every:?}"
        );
        assert!(every.windows(2).all(|w| w[0] < w[1]));
        assert!(reachable_addresses(&home, Some(true)).iter().all(|ip| every.contains(ip)));
        let _ = std::fs::remove_dir_all(&home);
    }
}
