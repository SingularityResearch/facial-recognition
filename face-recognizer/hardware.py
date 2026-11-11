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
                         "(FACE_DEVICE=npu tries it)")}


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
