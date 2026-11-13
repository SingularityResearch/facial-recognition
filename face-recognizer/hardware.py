"""
hardware.py - what this computer has, and how the programs use it.

describe() lists the processor, memory, graphics processors (GPUs), NPU and the ONNX Runtime ways of running models
(providers) on this computer; uses() says what the programs chose for each job, from what face_id decided at start
(face_id.DEVICE: each model is run on the GPU only after a check that it gives the CPU's answers, faster). The
Settings tab shows both (the labeling tool's /api/hardware). import_workers() is how many photos an import works on
at once.

Read from the system itself (Windows registry, Linux /proc and /sys, macOS sysctl), so it takes milliseconds;
nvidia-smi is asked for NVIDIA card names when it's there.
"""
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import threading
import time
from functools import lru_cache
from pathlib import Path

import cudasupport


def cpu_threads():
    """Processor threads this program may use (the process's affinity where the system reports it)."""
    try:
        return len(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        return os.cpu_count() or 4


def import_workers():
    """Photos an import works on at once: reading, decoding and finding faces each run on a worker thread
    (OpenCV, the decoders and ONNX Runtime release Python's lock, so threads use several cores). Two threads are left
    for the page, the live tracker and the system; at most 12. Measured on a 16-thread Ryzen 7 7840HS with an RTX 4060
    (2026-11-10): 6, 10, 14 and 20 workers took 47, 46, 44 and 44 s for the same 412 files. FACE_IMPORT_WORKERS
    overrides."""
    raw = os.environ.get("FACE_IMPORT_WORKERS", "").strip()
    if raw.isdigit() and int(raw) > 0:
        return int(raw)
    return max(2, min(cpu_threads() - 2, 12))


def _run(cmd, timeout=3):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0)).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def _reg_values(key, name):
    """Windows: `name` under each subkey of HKLM\\key (e.g. every display adapter's DriverDesc)."""
    import winreg
    out = []
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key) as k:
            for i in range(winreg.QueryInfoKey(k)[0]):
                try:
                    with winreg.OpenKey(k, winreg.EnumKey(k, i)) as sub:
                        out.append(str(winreg.QueryValueEx(sub, name)[0]))
                except OSError:
                    continue
    except OSError:
        pass
    return out


def cpu_name():
    s = platform.system()
    if s == "Windows":
        names = _reg_values(r"HARDWARE\DESCRIPTION\System\CentralProcessor", "ProcessorNameString")
        if names:
            return names[0].strip()
    elif s == "Linux":
        try:
            for line in Path("/proc/cpuinfo").read_text(errors="replace").splitlines():
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
        except OSError:
            pass
    elif s == "Darwin":
        name = _run(["sysctl", "-n", "machdep.cpu.brand_string"])
        if name:
            return name
    return platform.processor() or "unknown processor"


def memory_gb():
    s = platform.system()
    try:
        if s == "Windows":
            import ctypes

            class Status(ctypes.Structure):
                _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong), ("total", ctypes.c_ulonglong),
                            ("avail", ctypes.c_ulonglong), ("pt", ctypes.c_ulonglong), ("pa", ctypes.c_ulonglong),
                            ("vt", ctypes.c_ulonglong), ("va", ctypes.c_ulonglong), ("ve", ctypes.c_ulonglong)]
            st = Status()
            st.length = ctypes.sizeof(Status)
            ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st))
            return round(st.total / 2**30)
        if s == "Linux":
            for line in Path("/proc/meminfo").read_text().splitlines():
                if line.startswith("MemTotal"):
                    return round(int(line.split()[1]) / 2**20)
        if s == "Darwin":
            return round(int(_run(["sysctl", "-n", "hw.memsize"]) or 0) / 2**30)
    except (OSError, ValueError, AttributeError):
        pass
    return None


VENDORS = {"0x10de": "NVIDIA", "0x1002": "AMD", "0x8086": "Intel", "0x5143": "Qualcomm"}


def gpus():
    """The graphics processors' names, the separate (discrete) ones first where the system says."""
    s = platform.system()
    if s == "Windows":
        found = _reg_values(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}",
                            "DriverDesc")
        return [g for g in dict.fromkeys(found) if "Basic" not in g and "Remote" not in g]
    if s == "Darwin":
        chip = _run(["sysctl", "-n", "machdep.cpu.brand_string"])
        return [f"{chip} GPU"] if chip.startswith("Apple") else ["(see System Information)"]
    out = []
    nvidia = [n for n in _run(["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"]).splitlines() if n]
    for dev in sorted(Path("/sys/bus/pci/devices").glob("*")):
        try:
            if not (dev / "class").read_text().startswith("0x03"):           # display controllers
                continue
            vendor = VENDORS.get((dev / "vendor").read_text().strip(), "Other")
        except OSError:
            continue
        if vendor == "NVIDIA" and nvidia:
            out.append(f"NVIDIA {nvidia.pop(0).removeprefix('NVIDIA ')}")
        else:
            out.append(f"{vendor} graphics ({dev.name})")
    return out


def npu():
    """The neural processing unit, if the system lists one, else None."""
    s = platform.system()
    if s == "Windows":
        found = _reg_values(r"SYSTEM\CurrentControlSet\Control\Class\{f01a9d53-3ff6-48d2-9f97-c8a7004be10c}",
                            "DriverDesc")                        # the "Neural processors" (compute accelerator) class
        return found[0] if found else None
    if s == "Linux":
        for dev in sorted(Path("/sys/class/accel").glob("accel*")):
            try:
                return f"{Path(os.readlink(dev / 'device' / 'driver')).name} ({dev.name})"
            except OSError:
                return dev.name
        return None
    if s == "Darwin" and _run(["sysctl", "-n", "machdep.cpu.brand_string"]).startswith("Apple"):
        return "Apple Neural Engine (reached through CoreML)"
    return None


def providers():
    try:
        import onnxruntime as ort
        return [p.removesuffix("ExecutionProvider") for p in ort.get_available_providers()]
    except ImportError:
        return []


@lru_cache(maxsize=1)
def describe():
    """What this computer has (cached: it doesn't change while the program runs)."""
    return {"system": f"{platform.system()} {platform.release()} ({platform.machine()})",
            "python": sys.version.split()[0], "cpu": cpu_name(), "threads": cpu_threads(), "memory_gb": memory_gb(),
            "gpus": gpus(), "npu": npu(), "onnxruntime": providers()}


def uses():
    """What the programs chose on this computer, for each job (filled in as the models load)."""
    import face_id as F
    devices = dict(F.LAST_DEVICES)
    return {"device_setting": F.DEVICE, "devices": devices, "decisions": dict(F.GPU_DECISIONS),
            "import_workers": import_workers(),
            "photos": "decoded on the processor (JPEG straight at reduced size), read once each",
            "videos": "decoded on the processor, about one frame a second looked at",
            "npu_note": ("not used by itself: measured slower than the GPU and the processor for one face at a time "
                         "(FACE_DEVICE=npu tries it)"),
            "gpu_note": gpu_note(), "cuda": cudasupport.status(), "cuda_offer": cudasupport.offer()}


def gpu_note():
    """Why a graphics card that's here isn't used, when this copy of ONNX Runtime can't reach it (else "")."""
    h, have = describe(), set(describe()["onnxruntime"])
    if have & {"Dml", "CUDA", "CoreML", "ROCm", "MIGraphX", "OpenVINO"} or cudasupport.needed():
        return ""                             # in use, or the Linux app is fetching CUDA (cudasupport.status says)
    if any(g.startswith("NVIDIA") for g in h["gpus"]):
        return ("An NVIDIA card is here, but this copy of the app has no CUDA (its libraries are too big to ship), so "
                "everything runs on the processor. To use the card, run the app from the code: ./setup.sh installs "
                "ONNX Runtime with CUDA, then ./start.sh.")
    if h["gpus"]:
        return "No way to run models on this computer's graphics card was found, so everything runs on the processor."
    return ""


# --------------------------------------------------------------------------- AI model apps (for Describe photo)
AI_PORTS = [(1234, "LM Studio"), (11434, "Ollama"), (1337, "Jan"), (8080, "llama.cpp / LocalAI"), (8000, "vLLM"),
            (5000, "text-generation-webui"), (4891, "GPT4All")]
_ai_cache = {"at": 0.0, "found": None}
_ai_lock = threading.Lock()


def _port_open(port, host="127.0.0.1", timeout=0.25):
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except OSError:
        return False


def lan_addresses():
    """This computer's own addresses on the local network (IPv4, not loopback): the one it reaches the network
    through first. Opening a UDP socket toward a private address sends nothing; it only picks the route."""
    found = []
    for probe in ("10.255.255.255", "192.168.255.255", "172.31.255.255"):
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
                s.connect((probe, 1))
                ip = s.getsockname()[0]
            if ip and not ip.startswith("127.") and ip not in found:
                found.append(ip)
        except OSError:
            continue
    return found


VIRTUAL = ("lo", "docker", "br-", "veth", "virbr", "tun", "tap", "wg", "zt", "tailscale", "CloudflareWARP", "vEthernet")


def distributed():
    """This device's "Fully distributed system" setting (Settings -> LM Studio; kept in HOME/lmstudio.json by
    lmstudio.set_distributed, off for a new copy): the owner's computers may be anywhere, not on one local network.
    On, every network adapter counts as the device's own (VPN tunnels and virtual networks too), and LM Studio
    servers beyond the local network may be used (describe.helpers)."""
    import paths
    try:
        return json.loads((paths.HOME / "lmstudio.json").read_text("utf-8")).get("distributed") is True
    except (OSError, ValueError, AttributeError):
        return False


_adapter_cache = {"at": 0.0, "found": None}
_adapter_lock = threading.Lock()
MAC_VIRTUAL = ("lo", "utun", "bridge", "vmnet", "awdl", "llw", "tap", "tun", "feth", "ipsec", "ppp", "gif", "stf")


def _adapters(max_age=300):
    """[(IPv4 address, is the adapter virtual?)] for every address of this computer, or None when the system doesn't
    say (then own_addresses falls back to the addresses of the computer's name). Virtual: a VPN tunnel, a virtual
    switch (Hyper-V, WSL, VirtualBox), a container bridge. Linux: `ip`; macOS: `ifconfig`; Windows: PowerShell's
    Get-NetAdapter, which takes seconds to start: it is asked on a thread of its own and the last answer is given
    meanwhile (None until the first one is in, a few seconds after the program starts), so nothing waits for it."""
    system = platform.system()
    if system == "Linux":
        out = []
        for line in _run(["ip", "-4", "-o", "addr", "show", "scope", "global"]).splitlines():
            parts = line.split()
            if len(parts) > 3:
                out.append((parts[3].split("/")[0], parts[1].startswith(VIRTUAL) or "/32" in parts[3]))
        return out or None
    if system == "Darwin":
        out, name = [], ""
        for line in _run(["ifconfig"]).splitlines():
            if line and not line[0].isspace():
                name = line.split(":")[0]
            elif line.strip().startswith("inet ") and len(line.split()) > 1:
                out.append((line.split()[1], name.startswith(MAC_VIRTUAL)))
        return out or None
    if system != "Windows":
        return None
    with _adapter_lock:
        if time.time() - _adapter_cache["at"] >= max_age and not _adapter_cache.get("asking"):
            _adapter_cache["asking"] = True
            threading.Thread(target=_ask_windows_adapters, daemon=True, name="network adapters").start()
        return _adapter_cache["found"]


def _ask_windows_adapters():
    ask = ("Get-NetIPAddress -AddressFamily IPv4 | ForEach-Object { $a = Get-NetAdapter -InterfaceIndex "
           "$_.InterfaceIndex -ErrorAction SilentlyContinue; if ($a) { '{0} {1}' -f $_.IPAddress, [int]$a.Virtual } }")
    out = []
    for line in _run(["powershell", "-NoProfile", "-NonInteractive", "-Command", ask], timeout=30).splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1] in ("0", "1") and parts[0].count(".") == 3:
            out.append((parts[0], parts[1] == "1"))
    with _adapter_lock:                          # no answer (PowerShell missing or slow): the last one stays
        _adapter_cache.update(at=time.time(), asking=False, found=out or _adapter_cache["found"])


def own_addresses(every=None):
    """Every IPv4 address of this computer's real network adapters (each 10G port, Wi-Fi...): not loopback, and not
    VPN tunnels, virtual switches or containers unless every is true. every defaults to the "Fully distributed
    system" setting (distributed()). When the system doesn't say which adapter an address belongs to, the addresses
    of the computer's name are taken as they are."""
    every = distributed() if every is None else every
    known = _adapters()
    if known is None:
        found = list(lan_addresses())
        try:
            found += socket.gethostbyname_ex(socket.gethostname())[2]
        except OSError:
            pass
    else:
        found = [ip for ip, virtual in known if every or not virtual]
    return sorted({ip for ip in found if not ip.startswith(("127.", "169.254."))},
                  key=lambda a: tuple(int(x) for x in a.split(".")))


def _installed_ai_apps():
    """AI model apps installed on this computer, running or not (where each normally installs itself)."""
    home, s = Path.home(), platform.system()
    local = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
    places = {"LM Studio": [home / ".lmstudio", home / ".cache" / "lm-studio", local / "Programs" / "LM Studio",
                            local / "Programs" / "lm-studio", Path("/Applications/LM Studio.app")],
              "Jan": [home / "jan", Path(os.environ.get("APPDATA", home)) / "Jan", Path("/Applications/Jan.app")],
              "GPT4All": [local / "nomic.ai" / "GPT4All", Path("/Applications/GPT4All.app")]}
    found = [name for name, paths in places.items() if any(p.exists() for p in paths)]
    if shutil.which("ollama") or (s == "Windows" and (local / "Programs" / "Ollama").exists()) or \
            Path("/Applications/Ollama.app").exists():
        found.append("Ollama")
    return found


def lmstudio_downloads():
    """LM Studio's downloaded models (its models folder, from its settings.json): [(name, reads pictures)], a vision
    model being one with an mmproj (picture projector) file beside it. [] when LM Studio isn't installed."""
    home = Path.home() / ".lmstudio"
    folder = home / "models"
    try:
        folder = Path(json.loads((home / "settings.json").read_text(encoding="utf-8")).get("downloadsFolder") or folder)
    except (OSError, ValueError, AttributeError):
        pass
    found = []
    try:
        for pub in sorted(p for p in folder.iterdir() if p.is_dir()):
            for model in sorted(m for m in pub.iterdir() if m.is_dir()):
                sees = any("mmproj" in f.name.lower() for f in model.iterdir())
                found.append((f"{pub.name}/{model.name}", sees))
    except OSError:
        pass
    return found


def ai_apps(max_age=60):
    """The AI model apps that could describe photos: [{app, url, running, models, vision, note}]. Asks the usual
    ports on this computer and the Describe photo server set in Settings (which may be another device), each with
    a short time limit; nothing else on the network. Cached for max_age seconds."""
    import describe
    with _ai_lock:
        if _ai_cache["found"] is not None and time.time() - _ai_cache["at"] < max_age:
            return _ai_cache["found"]
    out, seen = [], set()

    def probe(app, url, key_from_settings=False):
        entry = {"app": app, "url": url, "running": True, "models": 0, "vision": [], "note": ""}
        try:
            found, loaded = describe.server_models(None if key_from_settings else url)
            entry["models"] = len(found)
            entry["vision"] = [m for m, sees in found if sees][:6]
            if loaded:
                entry["note"] = "loaded: " + ", ".join(loaded[:3])
        except describe.Unavailable as e:
            entry["running"], entry["note"] = _port_open(int(url.rsplit(":", 1)[1].split("/")[0])) \
                if url.startswith("http://127.0.0.1:") else False, str(e)[:200]
        except (ValueError, IndexError) as e:
            entry["running"], entry["note"] = False, str(e)[:200]
        out.append(entry)

    for port, app in AI_PORTS:
        if _port_open(port):
            url = f"http://127.0.0.1:{port}"
            seen.add(url)
            probe(app, url)
            # served to the network too? then other devices (Describe photo on them) can use this address
            out[-1]["lan"] = [f"http://{ip}:{port}" for ip in own_addresses() if _port_open(port, ip)]
    configured = describe.config()["url"]
    if configured not in seen:
        app = next((a for p, a in AI_PORTS if configured.endswith(f":{p}")), "Describe photo server")
        probe(app, configured, key_from_settings=True)
        out[-1]["note"] = ("set in Settings" + (f"; {out[-1]['note']}" if out[-1]["note"] else ""))
    for app in _installed_ai_apps():
        same = [e for e in out if app in e["app"]]
        if any(e["running"] for e in same):
            continue
        entry = same[0] if same else {"app": app, "url": None, "running": False, "models": 0, "vision": [], "note": ""}
        if not same:
            out.append(entry)
        entry["note"] = "installed, not running (start it, and its local server)"
        if app == "LM Studio":
            disk = lmstudio_downloads()
            entry["models"], entry["vision"] = len(disk), [m for m, sees in disk if sees][:6]
            entry["note"] += f"; {len(disk)} model{'' if len(disk) == 1 else 's'} downloaded" + \
                ("" if entry["vision"] else ", none of them reads pictures")
    with _ai_lock:
        _ai_cache.update(at=time.time(), found=out)
    return out


# --------------------------------------------------------------------------- LM Studio on the local network
LMS_PORTS = (1234,)           # LM Studio's server port (its default; this project's devices all use it)
_net_cache = {"at": 0.0, "found": None}


def _net_forget():
    """Forget the last search for LM Studio servers (the setting that says where to look has changed)."""
    with _ai_lock:
        _net_cache.update(at=0.0, found=None)


def _lms_models(url, timeout=3):
    """[{id, vision, loaded, quant}] from an LM Studio server's own model list, or None if it isn't LM Studio."""
    import urllib.request
    try:
        with urllib.request.urlopen(url + "/api/v0/models", timeout=timeout) as r:
            data = json.loads(r.read(2_000_000)).get("data", [])
        if not isinstance(data, list):               # any computer on the network can answer here: nothing is assumed
            return None
        text = lambda v: v if isinstance(v, str) and len(v) <= 200 else None   # noqa: E731
        return [{"id": m["id"], "vision": m.get("type") == "vlm", "loaded": m.get("state") == "loaded",
                 "quant": text(m.get("quantization")), "kind": text(m.get("type"))}
                for m in data[:500] if isinstance(m, dict) and text(m.get("id"))]
    except (OSError, ValueError, AttributeError, TypeError):
        return None


def _dns_name(data, i, depth=0):
    """(name, next offset) of a DNS name in a packet, following compression pointers."""
    labels = []
    while i < len(data) and depth < 10:
        n = data[i]
        if n == 0:
            return ".".join(labels), i + 1
        if n >= 0xC0:
            rest, _ = _dns_name(data, ((n & 0x3F) << 8) | data[i + 1], depth + 1)
            return ".".join(labels + ([rest] if rest else [])), i + 2
        labels.append(data[i + 1:i + 1 + n].decode("utf-8", "replace"))
        i += 1 + n
    return ".".join(labels), i


MDNS_TYPES = ("_lmstudio._tcp.local", "_http._tcp.local")


def mdns_services(timeout=1.5):
    """LM Studio servers that announce themselves on the local network (mDNS / DNS-SD, as Avahi does): [{host, ip,
    port, txt}]. Asks for each MDNS_TYPES service, collects every answer for `timeout` seconds, and keeps the
    services that say they are LM Studio (service type _lmstudio._tcp, or app=LM Studio in their text)."""
    import struct
    q = struct.pack(">HHHHHH", 0, 0, len(MDNS_TYPES), 0, 0, 0)
    for name in MDNS_TYPES:
        q += b"".join(bytes([len(p)]) + p.encode() for p in name.split(".")) + b"\0" + struct.pack(">HH", 12, 0x8001)
    records = {"PTR": [], "SRV": {}, "TXT": {}, "A": {}}
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.settimeout(0.3)
            s.sendto(q, ("224.0.0.251", 5353))
            end = time.time() + timeout
            while time.time() < end:
                try:
                    data, _ = s.recvfrom(9000)
                except socket.timeout:
                    continue
                try:
                    qd, an, ns, ar = struct.unpack(">HHHH", data[4:12])
                    i = 12
                    for _ in range(qd):
                        i = _dns_name(data, i)[1] + 4
                    for _ in range(an + ns + ar):
                        name, i = _dns_name(data, i)
                        rtype, _, _, rdlen = struct.unpack(">HHIH", data[i:i + 10])
                        rd = i + 10
                        if rtype == 12:
                            records["PTR"].append((name, _dns_name(data, rd)[0]))
                        elif rtype == 33:
                            port = struct.unpack(">H", data[rd + 4:rd + 6])[0]
                            records["SRV"][name] = (_dns_name(data, rd + 6)[0], port)
                        elif rtype == 16:
                            txt, j = [], rd
                            while j < rd + rdlen:
                                txt.append(data[j + 1:j + 1 + data[j]].decode("utf-8", "replace"))
                                j += 1 + data[j]
                            records["TXT"][name] = txt
                        elif rtype == 1 and rdlen == 4:
                            records["A"].setdefault(name, set()).add(socket.inet_ntoa(data[rd:rd + 4]))
                        i = rd + rdlen
                except (struct.error, IndexError):
                    continue
    except OSError:
        return []
    out = []
    for stype, inst in records["PTR"]:
        txt = records["TXT"].get(inst, [])
        if inst not in records["SRV"] or not (stype.startswith("_lmstudio") or "app=LM Studio" in txt):
            continue
        target, port = records["SRV"][inst]
        for ip in sorted(records["A"].get(target, set())):
            if not any(o["ip"] == ip and o["port"] == port for o in out):      # announced as both types: once
                out.append({"host": target.removesuffix(".local"), "ip": ip, "port": port, "txt": txt})
    return out


def lmstudio_port():
    """The port this computer's LM Studio serves on (its own settings), or None."""
    for f in (Path.home() / ".lmstudio" / ".internal" / "http-server-config.json",
              Path.home() / ".cache" / "lm-studio" / ".internal" / "http-server-config.json"):
        try:
            return int(json.loads(f.read_text(encoding="utf-8")).get("port"))
        except (OSError, ValueError, TypeError):
            continue
    return None


def _mdns_name(ip, timeout=1.0):
    """The name a computer announces for itself on the local network (mDNS, as Avahi on Linux and macOS do): a
    reverse-lookup question to 224.0.0.251:5353 asking for a direct answer. None if nothing answers."""
    import struct
    qname = b"".join(bytes([len(p)]) + p.encode() for p in (ip.split(".")[::-1] + ["in-addr", "arpa"])) + b"\0"
    query = struct.pack(">HHHHHH", 0, 0, 1, 0, 0, 0) + qname + struct.pack(">HH", 12, 0x8001)   # PTR, IN, answer me
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.settimeout(timeout)
            s.sendto(query, ("224.0.0.251", 5353))
            data, _ = s.recvfrom(4096)
    except OSError:
        return None

    def name_at(i):
        return _dns_name(data, i)
    try:
        _, qd, an = struct.unpack(">HHH", data[2:8])[0], *struct.unpack(">HH", data[4:8])
        i = 12
        for _ in range(qd):
            i = name_at(i)[1] + 4
        for _ in range(an):
            _, i = name_at(i)
            rtype, _, _, rdlen = struct.unpack(">HHIH", data[i:i + 10])
            if rtype == 12:
                return name_at(i + 10)[0].removesuffix(".local") or None
            i += 10 + rdlen
    except (struct.error, IndexError):
        pass
    return None


def _netbios_name(ip, timeout=1.0):
    """A Windows computer's name, from its answer to a NetBIOS name-status question (UDP 137). None if it doesn't
    answer."""
    import struct
    query = (struct.pack(">HHHHHH", 0x4652, 0, 1, 0, 0, 0) + b"\x20" + b"CK" + b"AA" * 15 + b"\x00"
             + struct.pack(">HH", 0x21, 1))           # "*", NBSTAT, IN
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.settimeout(timeout)
            s.sendto(query, (ip, 137))
            data, _ = s.recvfrom(2048)
        i = 12 + 34 + 10                                  # header, the name asked about, type/class/ttl/length
        for k in range(data[i]):
            entry = data[i + 1 + 18 * k: i + 19 + 18 * k]
            if entry[15] == 0x00 and not entry[16] & 0x80:    # the computer's own (unique, workstation) name
                return entry[:15].decode("ascii", "replace").strip() or None
    except (OSError, IndexError):
        pass
    return None


def _name_of(ip):
    """A computer's name: its own mDNS announcement (Linux, macOS), its NetBIOS name (Windows), else the network's
    name service."""
    name = _mdns_name(ip) or _netbios_name(ip)
    if name:
        return name
    try:
        return socket.gethostbyaddr(ip)[0].split(".")[0]
    except OSError:
        return None


def lmstudio_network(max_age=60):
    """LM Studio servers on the local network: [{host, ip, url, this_computer, models}]. Asks LM Studio's port on
    every address of this computer's /24 network(s) (a quick connection attempt each), then each server that
    answers for its model list. Only when Settings asks (cached max_age seconds)."""
    with _ai_lock:
        if _net_cache["found"] is not None and time.time() - _net_cache["at"] < max_age:
            return _net_cache["found"]
    from concurrent.futures import ThreadPoolExecutor
    own = set(own_addresses())                    # every interface's network is looked at (two 10G ports: both)
    announced = mdns_services()                   # servers that say where they are: their port, whatever it is
    ports = set(LMS_PORTS) | {a["port"] for a in announced} | ({lmstudio_port()} - {None})
    targets = {(f"{'.'.join(ip.split('.')[:3])}.{n}", port) for ip in own for n in range(1, 255) for port in ports}
    targets = sorted(targets | {(a["ip"], a["port"]) for a in announced})       # announced on another network too
    with ThreadPoolExecutor(64) as ex:
        up = [t for t, ok in zip(targets, ex.map(lambda t: _port_open(t[1], t[0], 0.4), targets)) if ok]
        named = {a["ip"]: a["host"] for a in announced}
        rest = sorted({ip for ip, _ in up} - set(named))
        names = {**dict(zip(rest, ex.map(_name_of, rest))), **named}
        models = list(ex.map(lambda t: _lms_models(f"http://{t[0]}:{t[1]}"), up))
    found = []
    for (ip, port), m in zip(up, models):
        if m is None:
            continue
        host = names.get(ip) or ("this computer" if ip in own else ip)
        same = next((e for e in found if e["port"] == port and (e["host"] == host or (ip in own and e["this_computer"]))
                     and [x["id"] for x in e["models"]] == [x["id"] for x in m]), None)
        if same:                                  # the same server reached by another address of its computer
            same["ips"].append(ip)
            continue
        found.append({"host": host, "ips": [ip], "port": port, "this_computer": ip in own, "models": m,
                      "announced": any(a["ip"] == ip and a["port"] == port for a in announced)})
    for e in found:
        e["ips"].sort(key=lambda a: tuple(int(x) for x in a.split(".")))
        e["urls"] = [f"http://{a}:{e['port']}" for a in e["ips"]]
    found.sort(key=lambda e: (not e["this_computer"], e["host"]))
    with _ai_lock:
        _net_cache.update(at=time.time(), found=found)
    return found
