//! LM Studio servers on the local network, for Settings:
//! each computer's name, the addresses and port to reach it at, and the models it serves.
//!
//! Found two ways, only when Settings asks (kept a minute): servers that announce themselves (mDNS,
//! `_lmstudio._tcp`, or `_http._tcp` with `app=LM Studio`) say their own port; and LM Studio's port is tried
//! on every address of the /24 network of each of this computer's own adapters (`hardware::own_addresses`:
//! the real ones, unless the owner switched on "Fully distributed system"). Each server that answers is asked
//! for its model list, and a computer's name comes from its own mDNS answer, its NetBIOS name, or the
//! network's name service. Nothing is sent but those questions, and what other computers answer is checked
//! by its shape before it is used; no answer is followed to another address.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::hardware::own_addresses;

/// LM Studio's server port (its default; this project's devices all use it).
const LMS_PORTS: [u16; 1] = [1234];
const MDNS: (Ipv4Addr, u16) = (Ipv4Addr::new(224, 0, 0, 251), 5353);
const MDNS_TYPES: [&str; 2] = ["_lmstudio._tcp.local", "_http._tcp.local"];

/// (name, next offset) of a DNS name in a packet, following compression pointers.
fn dns_name(data: &[u8], mut i: usize, depth: usize) -> (String, usize) {
    let mut labels: Vec<String> = Vec::new();
    while i < data.len() && depth < 10 {
        let n = data[i] as usize;
        if n == 0 {
            return (labels.join("."), i + 1);
        }
        if n >= 0xC0 {
            let Some(low) = data.get(i + 1) else { break };
            let (rest, _) = dns_name(data, ((n & 0x3F) << 8) | *low as usize, depth + 1);
            if !rest.is_empty() {
                labels.push(rest);
            }
            return (labels.join("."), i + 2);
        }
        // (a name cut short is read as far as it goes)
        labels.push(String::from_utf8_lossy(&data[(i + 1).min(data.len())..(i + 1 + n).min(data.len())]).into_owned());
        i += 1 + n;
    }
    (labels.join("."), i)
}

/// A DNS name as a question holds it.
fn question_name(name: &str) -> Vec<u8> {
    let mut out: Vec<u8> =
        name.split('.').flat_map(|part| [&[part.len() as u8][..], part.as_bytes()].concat()).collect();
    out.push(0);
    out
}

/// A question for PTR records, answered straight to the asker.
fn ptr_question(names: &[&str]) -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, names.len() as u8, 0, 0, 0, 0, 0, 0];
    for name in names {
        q.extend(question_name(name));
        q.extend([0, 12, 0x80, 0x01]);
    }
    q
}

fn be16(data: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]) as usize)
}

/// A server that announces itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Announced {
    pub host: String,
    pub ip: Ipv4Addr,
    pub port: u16,
    pub txt: Vec<String>,
}

#[derive(Default)]
struct Records {
    ptr: Vec<(String, String)>,
    srv: BTreeMap<String, (String, u16)>,
    txt: BTreeMap<String, Vec<String>>,
    a: BTreeMap<String, BTreeSet<Ipv4Addr>>,
}

/// The records of one mDNS answer, added to what was found. None: the packet stops short.
fn read_answer(data: &[u8], found: &mut Records) -> Option<()> {
    let (qd, records) = (be16(data, 4)?, be16(data, 6)? + be16(data, 8)? + be16(data, 10)?);
    let mut i = 12;
    for _ in 0..qd {
        i = dns_name(data, i, 0).1 + 4;
    }
    for _ in 0..records {
        let (name, next) = dns_name(data, i, 0);
        let (kind, len, rd) = (be16(data, next)?, be16(data, next + 8)?, next + 10);
        data.get(rd..rd + len)?;
        match kind {
            12 => found.ptr.push((name, dns_name(data, rd, 0).0)),
            33 => {
                found.srv.insert(name, (dns_name(data, rd + 6, 0).0, be16(data, rd + 4)? as u16));
            }
            16 => {
                let (mut txt, mut j) = (Vec::new(), rd);
                while j < rd + len {
                    let n = data[j] as usize;
                    txt.push(String::from_utf8_lossy(data.get(j + 1..j + 1 + n)?).into_owned());
                    j += 1 + n;
                }
                found.txt.insert(name, txt);
            }
            1 if len == 4 => {
                found.a.entry(name).or_default().insert(Ipv4Addr::new(
                    data[rd],
                    data[rd + 1],
                    data[rd + 2],
                    data[rd + 3],
                ));
            }
            _ => {}
        }
        i = rd + len;
    }
    Some(())
}

/// Those of the records that are LM Studio's: the service type `_lmstudio._tcp`, or `app=LM Studio` in
/// their text; one announced as both types counts once.
fn announced_in(found: &Records) -> Vec<Announced> {
    let mut out: Vec<Announced> = Vec::new();
    for (kind, instance) in &found.ptr {
        let txt = found.txt.get(instance).cloned().unwrap_or_default();
        let Some((target, port)) = found.srv.get(instance) else { continue };
        if !(kind.starts_with("_lmstudio") || txt.iter().any(|t| t == "app=LM Studio")) {
            continue;
        }
        for ip in found.a.get(target).into_iter().flatten() {
            if !out.iter().any(|o| o.ip == *ip && o.port == *port) {
                out.push(Announced {
                    host: target.strip_suffix(".local").unwrap_or(target).to_string(),
                    ip: *ip,
                    port: *port,
                    txt: txt.clone(),
                });
            }
        }
    }
    out
}

/// LM Studio servers that announce themselves on the local network (mDNS / DNS-SD): every answer that comes
/// within `wait` is taken.
pub fn mdns_services(wait: Duration) -> Vec<Announced> {
    let mut found = Records::default();
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return Vec::new() };
    if socket.set_read_timeout(Some(Duration::from_millis(300))).is_err()
        || socket.send_to(&ptr_question(&MDNS_TYPES), MDNS).is_err()
    {
        return Vec::new();
    }
    let (end, mut buf) = (Instant::now() + wait, vec![0u8; 9000]);
    while Instant::now() < end {
        if let Ok((n, _)) = socket.recv_from(&mut buf) {
            let _ = read_answer(&buf[..n], &mut found);
        }
    }
    announced_in(&found)
}

/// The port this computer's LM Studio serves on (its own settings), if it says.
pub fn lmstudio_port() -> Option<u16> {
    crate::lmstudio::homes().iter().find_map(|folder| {
        let config: Value =
            serde_json::from_slice(&std::fs::read(folder.join(".internal").join("http-server-config.json")).ok()?)
                .ok()?;
        let port = config.get("port")?;
        port.as_u64().or_else(|| port.as_str()?.trim().parse().ok()).and_then(|p| u16::try_from(p).ok())
    })
}

/// One question to one place over UDP, and the first answer within a second.
fn asked(question: &[u8], to: (Ipv4Addr, u16), room: usize) -> Option<Vec<u8>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    socket.send_to(question, to).ok()?;
    let mut buf = vec![0u8; room];
    let (n, _) = socket.recv_from(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

/// The name in the first PTR record of an answer, without ".local".
fn ptr_in(data: &[u8]) -> Option<String> {
    let (qd, an) = (be16(data, 4)?, be16(data, 6)?);
    let mut i = 12;
    for _ in 0..qd {
        i = dns_name(data, i, 0).1 + 4;
    }
    for _ in 0..an {
        let next = dns_name(data, i, 0).1;
        let (kind, len) = (be16(data, next)?, be16(data, next + 8)?);
        if kind == 12 {
            let name = dns_name(data, next + 10, 0).0;
            let name = name.strip_suffix(".local").unwrap_or(&name);
            return (!name.is_empty()).then(|| name.to_string());
        }
        i = next + 10 + len;
    }
    None
}

/// The name a computer announces for itself on the local network (mDNS, as Linux and macOS do): a reverse
/// question to the mDNS group, asking for a direct answer.
fn mdns_name(ip: Ipv4Addr) -> Option<String> {
    let [a, b, c, d] = ip.octets();
    ptr_in(&asked(&ptr_question(&[&format!("{d}.{c}.{b}.{a}.in-addr.arpa")]), MDNS, 4096)?)
}

/// The computer's own name in an answer to a NetBIOS name-status question (unique, workstation).
fn netbios_in(data: &[u8]) -> Option<String> {
    let at = 12 + 34 + 10; // header, the name asked about, type/class/ttl/length
    for k in 0..*data.get(at)? as usize {
        let entry = data.get(at + 1 + 18 * k..at + 19 + 18 * k)?;
        if entry[15] == 0 && entry[16] & 0x80 == 0 {
            let name: String = entry[..15].iter().map(|b| if b.is_ascii() { *b as char } else { '\u{fffd}' }).collect();
            return Some(name.trim().to_string()).filter(|n| !n.is_empty());
        }
    }
    None
}

/// A Windows computer's name, from its answer to a NetBIOS name-status question (UDP 137).
fn netbios_name(ip: Ipv4Addr) -> Option<String> {
    let mut question = vec![0x46, 0x52, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0x20, b'C', b'K'];
    question.extend(b"AA".repeat(15));
    question.extend([0, 0, 0x21, 0, 1]); // "*", NBSTAT, IN
    netbios_in(&asked(&question, (ip, 137), 2048)?)
}

/// The network's name service's name for an address, without its domain.
fn reverse_name(ip: Ipv4Addr) -> Option<String> {
    let mut host = [0u8; 1025];
    #[cfg(windows)]
    let ok = {
        use windows_sys::Win32::Networking::WinSock::{
            AF_INET, IN_ADDR, IN_ADDR_0, NI_NAMEREQD, SOCKADDR, SOCKADDR_IN, getnameinfo,
        };
        let address = SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: 0,
            sin_addr: IN_ADDR { S_un: IN_ADDR_0 { S_addr: u32::from_ne_bytes(ip.octets()) } },
            sin_zero: [0; 8],
        };
        // SAFETY: the address is a whole sockaddr_in of the size given, and the name is written into `host` with
        // its zero, at most `host.len()` bytes (the sockets were started by the UDP questions before this one)
        unsafe {
            getnameinfo(
                (&raw const address).cast::<SOCKADDR>(),
                std::mem::size_of::<SOCKADDR_IN>() as i32,
                host.as_mut_ptr(),
                host.len() as u32,
                std::ptr::null_mut(),
                0,
                NI_NAMEREQD as i32,
            ) == 0
        }
    };
    #[cfg(unix)]
    let ok = {
        // SAFETY: as above; a zeroed sockaddr_in is a valid one on every unix
        unsafe {
            let mut address: libc::sockaddr_in = std::mem::zeroed();
            address.sin_family = libc::AF_INET as libc::sa_family_t;
            address.sin_addr.s_addr = u32::from_ne_bytes(ip.octets());
            libc::getnameinfo(
                (&raw const address).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                host.as_mut_ptr().cast(),
                host.len() as libc::socklen_t,
                std::ptr::null_mut(),
                0,
                libc::NI_NAMEREQD,
            ) == 0
        }
    };
    #[cfg(not(any(windows, unix)))]
    let ok = false;
    let end = host.iter().position(|b| *b == 0).unwrap_or(0);
    let name = String::from_utf8_lossy(&host[..end]);
    (ok && end > 0).then(|| name.split('.').next().unwrap_or("").to_string()).filter(|n| !n.is_empty())
}

/// A computer's name: its own mDNS announcement (Linux, macOS), its NetBIOS name (Windows), else the
/// network's name service.
fn name_of(ip: Ipv4Addr) -> Option<String> {
    mdns_name(ip).or_else(|| netbios_name(ip)).or_else(|| reverse_name(ip))
}

/// [{id, vision, loaded, quant, kind}] from what an LM Studio server's own model list says, or None if this
/// isn't one. Any computer on the network can answer here: nothing is assumed about the answer.
fn models_in(said: &Value) -> Option<Vec<Value>> {
    let rows = match said.as_object()?.get("data") {
        None => return Some(Vec::new()),
        Some(rows) => rows.as_array()?,
    };
    let text = |v: Option<&Value>| v.and_then(Value::as_str).filter(|t| t.chars().count() <= 200).map(str::to_string);
    let mut out = Vec::new();
    for m in rows.iter().take(500) {
        let Some(id) = m.as_object().and_then(|m| text(m.get("id"))).filter(|id| !id.is_empty()) else { continue };
        out.push(json!({"id": id, "vision": m.get("type") == Some(&json!("vlm")), "loaded": m.get("state") == Some(&json!("loaded")),
                        "quant": text(m.get("quantization")), "kind": text(m.get("type"))}));
    }
    Some(out)
}

/// The model list of the LM Studio server at an address (three seconds, 2 MB at most, no redirect followed).
fn lms_models(ip: Ipv4Addr, port: u16) -> Option<Vec<Value>> {
    let t = Some(Duration::from_secs(3));
    let config =
        ureq::Agent::config_builder().max_redirects(0).max_redirects_will_error(false).http_status_as_error(false);
    let agent: ureq::Agent =
        config.timeout_connect(t).timeout_send_request(t).timeout_recv_response(t).timeout_recv_body(t).build().into();
    let mut answer = agent.get(format!("http://{ip}:{port}/api/v0/models")).call().ok()?;
    if !answer.status().is_success() {
        return None;
    }
    let mut bytes = Vec::new();
    answer.body_mut().as_reader().take(2_000_000).read_to_end(&mut bytes).ok()?;
    models_in(&serde_json::from_slice(&bytes).ok()?)
}

/// `work` done for every item on up to 64 threads; the answers in the items' order.
fn each<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let (next, done) = (AtomicUsize::new(0), Mutex::new(Vec::with_capacity(items.len())));
    std::thread::scope(|scope| {
        for _ in 0..items.len().min(64) {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(k) else { return };
                    let made = work(item);
                    done.lock().unwrap_or_else(|e| e.into_inner()).push((k, made));
                }
            });
        }
    });
    let mut done = done.into_inner().unwrap_or_else(|e| e.into_inner());
    done.sort_by_key(|(k, _)| *k);
    done.into_iter().map(|(_, made)| made).collect()
}

/// The servers found, put together: one entry a server, a server reached at several addresses of its
/// computer once, this computer's first and then by name. `up`: (address, port, its models or None when it
/// isn't LM Studio); `names`: a computer's name by its address.
fn put_together(
    up: Vec<(Ipv4Addr, u16, Option<Vec<Value>>)>,
    names: &BTreeMap<Ipv4Addr, String>,
    own: &[Ipv4Addr],
    announced: &[Announced],
) -> Vec<Value> {
    struct Found {
        host: String,
        ips: Vec<Ipv4Addr>,
        port: u16,
        here: bool,
        models: Vec<Value>,
        announced: bool,
    }
    let ids = |models: &[Value]| models.iter().map(|m| m["id"].clone()).collect::<Vec<_>>();
    let mut found: Vec<Found> = Vec::new();
    for (ip, port, models) in up {
        let Some(models) = models else { continue };
        let here = own.contains(&ip);
        let host =
            names.get(&ip).cloned().unwrap_or_else(|| if here { "this computer".to_string() } else { ip.to_string() });
        // the same server reached by another address of its computer
        let same = found
            .iter_mut()
            .find(|e| e.port == port && (e.host == host || (here && e.here)) && ids(&e.models) == ids(&models));
        match same {
            Some(same) => same.ips.push(ip),
            None => found.push(Found {
                host,
                ips: vec![ip],
                port,
                here,
                models,
                announced: announced.iter().any(|a| a.ip == ip && a.port == port),
            }),
        }
    }
    for e in &mut found {
        e.ips.sort();
    }
    found.sort_by(|a, b| (!a.here, &a.host).cmp(&(!b.here, &b.host)));
    (found.into_iter())
        .map(|e| {
            let urls: Vec<String> = e.ips.iter().map(|ip| format!("http://{ip}:{}", e.port)).collect();
            let ips: Vec<String> = e.ips.iter().map(ToString::to_string).collect();
            json!({"host": e.host, "ips": ips, "port": e.port, "this_computer": e.here, "models": e.models, "announced": e.announced, "urls": urls})
        })
        .collect()
}

static KEPT: Mutex<Option<(Instant, std::path::PathBuf, Value)>> = Mutex::new(None);

/// Forget the last search (the setting that says where to look has changed).
pub fn forget() {
    *KEPT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// LM Studio servers on the local network: [{host, ips, port, this_computer, models, announced, urls}].
/// Only when Settings asks; kept a minute.
pub fn lmstudio_network(home: &Path) -> Value {
    if let Some((at, of, found)) = &*KEPT.lock().unwrap_or_else(|e| e.into_inner())
        && at.elapsed() < Duration::from_secs(60)
        && of == home
    {
        return found.clone();
    }
    // every adapter's network is looked at (two network ports: both)
    let own = own_addresses(home, None);
    // servers that say where they are: their port, whatever it is
    let announced = mdns_services(Duration::from_millis(1500));
    let mut ports: BTreeSet<u16> = LMS_PORTS.into_iter().chain(announced.iter().map(|a| a.port)).collect();
    ports.extend(lmstudio_port());
    let mut targets: BTreeSet<(Ipv4Addr, u16)> = BTreeSet::new();
    for ip in &own {
        let [a, b, c, _] = ip.octets();
        targets.extend((1..255u8).flat_map(|n| ports.iter().map(move |port| (Ipv4Addr::new(a, b, c, n), *port))));
    }
    // (announced on another network too)
    targets.extend(announced.iter().map(|a| (a.ip, a.port)));
    let targets: Vec<(Ipv4Addr, u16)> = targets.into_iter().collect();
    let open = each(&targets, |(ip, port)| {
        TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(*ip), *port), Duration::from_millis(400)).is_ok()
    });
    let up: Vec<(Ipv4Addr, u16)> = targets.into_iter().zip(open).filter(|(_, open)| *open).map(|(t, _)| t).collect();
    let mut names: BTreeMap<Ipv4Addr, String> = announced.iter().map(|a| (a.ip, a.host.clone())).collect();
    let rest: Vec<Ipv4Addr> = up
        .iter()
        .map(|(ip, _)| *ip)
        .filter(|ip| !names.contains_key(ip))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    for (ip, name) in rest.iter().zip(each(&rest, |ip| name_of(*ip))) {
        names.extend(name.map(|name| (*ip, name)));
    }
    let models = each(&up, |(ip, port)| lms_models(*ip, *port));
    let up = up.into_iter().zip(models).map(|((ip, port), models)| (ip, port, models)).collect();
    let found = Value::Array(put_together(up, &names, &own, &announced));
    *KEPT.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), home.to_path_buf(), found.clone()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DNS record: its name (as a question holds it), type, and data.
    fn record(name: &str, kind: u16, data: &[u8]) -> Vec<u8> {
        let mut out = question_name(name);
        out.extend(kind.to_be_bytes());
        out.extend([0, 1, 0, 0, 0, 120]);
        out.extend((data.len() as u16).to_be_bytes());
        out.extend(data);
        out
    }

    fn answer(records: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0, 0, 0x84, 0, 0, 0, 0, records.len() as u8, 0, 0, 0, 0];
        out.extend(records.concat());
        out
    }

    #[test]
    fn a_question_has_its_fixed_bytes() {
        // (hardware.mdns_services' packet: two questions for PTR records, answered to the asker)
        let q = ptr_question(&MDNS_TYPES);
        assert_eq!(&q[..12], [0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            &q[12..],
            b"\x09_lmstudio\x04_tcp\x05local\x00\x00\x0c\x80\x01\x05_http\x04_tcp\x05local\x00\x00\x0c\x80\x01"
        );
        let reverse = ptr_question(&["20.1.168.192.in-addr.arpa"]);
        assert_eq!(&reverse[12..], b"\x0220\x011\x03168\x03192\x07in-addr\x04arpa\x00\x00\x0c\x80\x01");
    }

    #[test]
    fn names_are_read_with_their_pointers_and_a_short_packet_is_no_name() {
        // "pc.local" at 12, then a pointer to it behind "srv"
        let mut data = vec![0u8; 12];
        data.extend(b"\x02pc\x05local\x00");
        let at = data.len();
        data.extend(b"\x03srv\xc0\x0c");
        assert_eq!(dns_name(&data, 12, 0), ("pc.local".to_string(), 22));
        assert_eq!(dns_name(&data, at, 0), ("srv.pc.local".to_string(), at + 6));
        // a pointer to itself ends, and so does a name cut short
        assert_eq!(dns_name(b"\xc0\x00", 0, 0).0, "");
        assert_eq!(dns_name(b"\x05ab", 0, 0), ("ab".to_string(), 6));
        assert_eq!(dns_name(b"\xc0", 0, 0).0, "");
    }

    #[test]
    fn only_what_says_it_is_lm_studio_is_taken_from_the_announcements() {
        let srv = |target: &str, port: u16| [&[0, 0, 0, 0][..], &port.to_be_bytes(), &question_name(target)].concat();
        let txt = |parts: &[&str]| {
            parts.iter().flat_map(|p| [&[p.len() as u8][..], p.as_bytes()].concat()).collect::<Vec<u8>>()
        };
        let packet = answer(&[
            record("_lmstudio._tcp.local", 12, &question_name("LM Studio on pc._lmstudio._tcp.local")),
            record("LM Studio on pc._lmstudio._tcp.local", 33, &srv("pc.local", 1234)),
            record("LM Studio on pc._lmstudio._tcp.local", 16, &txt(&["app=LM Studio", "v=1"])),
            // the same server as a web service too: once
            record("_http._tcp.local", 12, &question_name("LM Studio on pc._http._tcp.local")),
            record("LM Studio on pc._http._tcp.local", 33, &srv("pc.local", 1234)),
            record("LM Studio on pc._http._tcp.local", 16, &txt(&["app=LM Studio"])),
            // a printer's web page: not LM Studio
            record("_http._tcp.local", 12, &question_name("Printer._http._tcp.local")),
            record("Printer._http._tcp.local", 33, &srv("printer.local", 80)),
            record("Printer._http._tcp.local", 16, &txt(&["app=Something else"])),
            // one that says so on another port, at two addresses
            record("_http._tcp.local", 12, &question_name("Other._http._tcp.local")),
            record("Other._http._tcp.local", 33, &srv("mac.local", 4321)),
            record("Other._http._tcp.local", 16, &txt(&["app=LM Studio"])),
            record("pc.local", 1, &[192, 168, 1, 20]),
            record("printer.local", 1, &[192, 168, 1, 30]),
            record("mac.local", 1, &[192, 168, 1, 41]),
            record("mac.local", 1, &[192, 168, 1, 40]),
            record("mac.local", 28, &[0; 16]),
        ]);
        let mut found = Records::default();
        assert!(read_answer(&packet, &mut found).is_some());
        let said: Vec<(String, String, u16)> =
            announced_in(&found).into_iter().map(|a| (a.host, a.ip.to_string(), a.port)).collect();
        assert_eq!(
            said,
            [
                ("pc".to_string(), "192.168.1.20".to_string(), 1234),
                ("mac".to_string(), "192.168.1.40".to_string(), 4321),
                ("mac".to_string(), "192.168.1.41".to_string(), 4321)
            ]
        );
        // a packet that stops short adds nothing and breaks nothing
        for cut in 0..packet.len() {
            let _ = read_answer(&packet[..cut], &mut Records::default());
        }
        assert!(read_answer(&packet[..40], &mut Records::default()).is_none());
        // the name a computer gives for its own address
        let reverse = answer(&[record("20.1.168.192.in-addr.arpa", 12, &question_name("pc.local"))]);
        assert_eq!(ptr_in(&reverse).as_deref(), Some("pc"));
        assert_eq!(ptr_in(&reverse[..20]), None);
    }

    #[test]
    fn a_windows_computers_name_is_read_from_its_name_status() {
        let mut data = vec![0u8; 12 + 34 + 10];
        let entry = |name: &str, kind: u8, flags: u8| {
            let mut e = format!("{name:<15}").into_bytes();
            e.extend([kind, flags, 0]);
            e
        };
        data.push(3);
        data.extend(entry("WORKGROUP", 0x00, 0x80)); // a group's name
        data.extend(entry("MYPC", 0x20, 0x04)); // the file server's
        data.extend(entry("MYPC", 0x00, 0x04)); // the computer's own
        assert_eq!(netbios_in(&data).as_deref(), Some("MYPC"));
        assert_eq!(netbios_in(&data[..80]), None);
        assert_eq!(netbios_in(&[]), None);
    }

    #[test]
    fn a_model_list_is_taken_by_its_shape_only() {
        let said = json!({"data": [{"id": "a/vision", "type": "vlm", "state": "loaded", "quantization": "Q4_K_M"}, {"id": "b/words", "type": "llm", "state": "not-loaded"},
                                   {"id": 5}, "not a row", {"type": "vlm"}, {"id": "x".repeat(201)}, {"id": "c", "type": 7, "quantization": "q".repeat(300)}]});
        assert_eq!(
            models_in(&said).unwrap(),
            [
                json!({"id": "a/vision", "vision": true, "loaded": true, "quant": "Q4_K_M", "kind": "vlm"}),
                json!({"id": "b/words", "vision": false, "loaded": false, "quant": null, "kind": "llm"}),
                json!({"id": "c", "vision": false, "loaded": false, "quant": null, "kind": null})
            ]
        );
        assert_eq!(models_in(&json!({})), Some(Vec::new()));
        for other in [json!([]), json!("text"), json!({"data": {"id": "a"}}), json!({"data": "many"})] {
            assert_eq!(models_in(&other), None, "{other}");
        }
        let many: Vec<Value> = (0..600).map(|k| json!({"id": format!("m{k}")})).collect();
        assert_eq!(models_in(&json!({"data": many})).unwrap().len(), 500);
    }

    #[test]
    fn servers_are_put_together_one_a_computer_this_one_first() {
        let ip = |last: u8| Ipv4Addr::new(192, 168, 1, last);
        let models = |ids: &[&str]| Some(ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>());
        let names: BTreeMap<Ipv4Addr, String> =
            [(ip(20), "zeta".to_string()), (ip(21), "zeta".to_string()), (ip(30), "alpha".to_string())].into();
        let announced = [Announced { host: "alpha".into(), ip: ip(30), port: 4321, txt: vec![] }];
        let up = vec![
            (ip(5), 1234, models(&["m1"])),        // this computer, by its first address
            (ip(6), 1234, models(&["m1"])),        // and by its second
            (ip(9), 1234, None),                   // something else on that port
            (ip(21), 1234, models(&["m1", "m2"])), // zeta, at two addresses
            (ip(20), 1234, models(&["m1", "m2"])),
            (ip(30), 4321, models(&[])),     // alpha, which announces itself
            (ip(44), 1234, models(&["m9"])), // a computer nothing names
        ];
        let found = put_together(up, &names, &[ip(5), ip(6)], &announced);
        let said: Vec<(&str, usize, u64, bool, bool)> = (found.iter())
            .map(|e| {
                (
                    e["host"].as_str().unwrap(),
                    e["ips"].as_array().unwrap().len(),
                    e["port"].as_u64().unwrap(),
                    e["this_computer"] == true,
                    e["announced"] == true,
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                ("this computer", 2, 1234, true, false),
                ("192.168.1.44", 1, 1234, false, false),
                ("alpha", 1, 4321, false, true),
                ("zeta", 2, 1234, false, false)
            ]
        );
        assert_eq!(found[3]["urls"], json!(["http://192.168.1.20:1234", "http://192.168.1.21:1234"]));
        assert_eq!(found[0]["models"], json!([{"id": "m1"}]));
    }

    #[test]
    fn work_is_done_for_every_item_and_answered_in_order() {
        let items: Vec<usize> = (0..300).collect();
        assert_eq!(each(&items, |k| k * 2), items.iter().map(|k| k * 2).collect::<Vec<_>>());
        assert!(each(&Vec::<usize>::new(), |k| *k).is_empty());
    }
}
