"""
lmstudio.py - LM Studio on this computer, kept the way the owner's computers share it (owner's rule, 2026-11-10):
every LM Studio serves on all network interfaces, announces itself on the local network with its real port, and has
the same vision models as the other LM Studio servers there.

- serve_everywhere(): LM Studio's own server settings say "all interfaces" (0.0.0.0); if its server is answering on
  this computer only, it is started again on all interfaces with LM Studio's `lms` command.
- Announcer: while this app runs and LM Studio's server is up, it is announced over mDNS (DNS-SD) as
  `LM Studio on <computer>` (_lmstudio._tcp and _http._tcp, its port): Windows' own service registration
  (DnsServiceRegister, Windows 10 1809+), macOS Bonjour (dns-sd), Linux Avahi (avahi-publish-service). Not when the
  computer announces it already (an Avahi service file, as on shiva-ubuntu).
- missing_models() / get_models(): the vision models other LM Studio servers on the network serve that this one
  hasn't got (same model and quantization), downloaded with `lms get <model>@<quant>` when asked in Settings.

keep() runs all of it every minute on a background thread (app.py). status() is shown in Settings -> This computer.
"""
import hashlib
import json
import os
import platform
import shutil
import socket
import subprocess
import threading
import time
from pathlib import Path

import hardware
import paths

STATE = {"announced": None, "bind": None, "get": None, "share": None}     # what was done, for Settings
_lock = threading.Lock()


def lms_path():
    """LM Studio's `lms` command, or None."""
    exe = "lms.exe" if platform.system() == "Windows" else "lms"
    for p in (Path.home() / ".lmstudio" / "bin" / exe, Path.home() / ".cache" / "lm-studio" / "bin" / exe):
        if p.exists():
            return str(p)
    return shutil.which("lms")


def _lms(*args, timeout=60):
    lms = lms_path()
    if not lms:
        return None
    try:
        return subprocess.run([lms, *args], capture_output=True, text=True, timeout=timeout,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except (OSError, subprocess.SubprocessError):
        return None


def config_path():
    for p in (Path.home() / ".lmstudio" / ".internal" / "http-server-config.json",
              Path.home() / ".cache" / "lm-studio" / ".internal" / "http-server-config.json"):
        if p.exists():
            return p
    return None


def port():
    return hardware.lmstudio_port() or 1234


def running():
    return hardware._port_open(port())


def on_network():
    """LM Studio's server answers on this computer's network addresses (not only 127.0.0.1)."""
    return any(hardware._port_open(port(), ip) for ip in hardware.own_addresses())


def serve_everywhere():
    """Make LM Studio serve on all interfaces: its settings, and its running server. Returns what was done."""
    done = []
    cfg = config_path()
    if cfg:
        try:
            c = json.loads(cfg.read_text(encoding="utf-8"))
            if c.get("networkInterface") != "0.0.0.0":
                c["networkInterface"] = "0.0.0.0"
                cfg.write_text(json.dumps(c, indent=2), encoding="utf-8")
                done.append("its settings now serve on all network interfaces")
        except (OSError, ValueError):
            pass
    if running() and not on_network() and hardware.own_addresses():
        _lms("server", "stop", timeout=30)
        r = _lms("server", "start", "--port", str(port()), "--bind", "0.0.0.0", "--cors", timeout=90)
        if r is not None and r.returncode == 0:
            done.append("its server was started again on all network interfaces")
    if done:
        STATE["bind"] = "; ".join(done)
    return done


# --------------------------------------------------------------------------- announcing (mDNS / DNS-SD)
def _host():
    return socket.gethostname().split(".")[0]


def _txt():
    return {"app": "LM Studio", "api": "openai", "path": "/v1"}


class _WindowsAnnouncer:
    """Windows' DNS-SD registration (dnsapi.dll DnsServiceRegister): the system's own mDNS answers for us."""

    def __init__(self):
        import ctypes
        from ctypes import wintypes as w
        self.c, self.w = ctypes, w
        self.dll = ctypes.WinDLL("dnsapi.dll")
        self.cb_type = ctypes.WINFUNCTYPE(None, w.DWORD, ctypes.c_void_p, ctypes.c_void_p)
        self.cb = self.cb_type(lambda status, ctx, inst: None)

        class Request(ctypes.Structure):
            _fields_ = [("Version", w.ULONG), ("InterfaceIndex", w.ULONG), ("pServiceInstance", ctypes.c_void_p),
                        ("pRegisterCompletionCallback", self.cb_type), ("pQueryContext", ctypes.c_void_p),
                        ("hCredentials", w.HANDLE), ("unicastEnabled", w.BOOL)]
        self.Request = Request
        self.dll.DnsServiceConstructInstance.restype = ctypes.c_void_p
        self.dll.DnsServiceConstructInstance.argtypes = [w.LPCWSTR, w.LPCWSTR, ctypes.c_void_p, ctypes.c_void_p,
                                                         w.WORD, w.WORD, w.WORD, w.DWORD,
                                                         ctypes.POINTER(w.LPCWSTR), ctypes.POINTER(w.LPCWSTR)]
        self.dll.DnsServiceRegister.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        self.dll.DnsServiceDeRegister.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        self.requests = []

    def start(self, name, port_):
        c, w = self.c, self.w
        keys, vals = list(_txt()), list(_txt().values())
        k = (w.LPCWSTR * len(keys))(*keys)
        v = (w.LPCWSTR * len(vals))(*vals)
        for stype in ("_lmstudio._tcp", "_http._tcp"):
            inst = self.dll.DnsServiceConstructInstance(f"{name}.{stype}.local", f"{_host()}.local", None, None,
                                                        port_, 0, 0, len(keys), k, v)
            if not inst:
                raise OSError("DnsServiceConstructInstance failed")
            req = self.Request(1, 0, inst, self.cb, None, None, False)
            rc = self.dll.DnsServiceRegister(c.byref(req), None)
            if rc not in (0, 9506):                          # DNS_REQUEST_PENDING
                raise OSError(f"DnsServiceRegister failed ({rc})")
            self.requests.append((req, k, v))               # kept alive while announced

    def stop(self):
        for req, _, _ in self.requests:
            self.dll.DnsServiceDeRegister(self.c.byref(req), None)
        self.requests = []


class _CommandAnnouncer:
    """macOS dns-sd / Linux avahi-publish-service: a process that keeps the announcement up while it runs."""

    def __init__(self, tool):
        self.tool, self.procs = tool, []

    def start(self, name, port_):
        txt = [f"{a}={b}" for a, b in _txt().items()]
        for stype in ("_lmstudio._tcp", "_http._tcp"):
            cmd = ([self.tool, "-R", name, stype, "local", str(port_), *txt] if self.tool.endswith("dns-sd")
                   else [self.tool, "-s", name, stype, str(port_), *txt])
            self.procs.append(subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))

    def stop(self):
        for p in self.procs:
            p.terminate()
        self.procs = []


def _announcer():
    s = platform.system()
    if s == "Windows":
        return _WindowsAnnouncer()
    tool = shutil.which("dns-sd") if s == "Darwin" else shutil.which("avahi-publish-service")
    if tool:
        return _CommandAnnouncer(tool)
    raise OSError("no way to announce here" + (" (install avahi-utils: sudo apt install avahi-utils)" if s == "Linux" else ""))


_active = {"port": None, "by": None}


def announced_already():
    """This computer's LM Studio is announced already (by the system itself, e.g. an Avahi service file, or by us)."""
    own = set(hardware.own_addresses())
    return any(a["ip"] in own and a["port"] == port() for a in hardware.mdns_services(timeout=1.0))


def announce():
    """Announce this computer's LM Studio while its server is up; stop when it's down; follow a changed port."""
    with _lock:
        p = port() if running() else None
        if _active["by"] and _active["port"] != p:
            _active["by"].stop()
            _active.update(port=None, by=None)
            STATE["announced"] = None
        if p is None or _active["by"]:
            return
        if announced_already():
            STATE["announced"] = f"announced on the network by this computer itself (port {p})"
            return
        try:
            a = _announcer()
            a.start(f"LM Studio on {_host()}", p)
            _active.update(port=p, by=a)
            STATE["announced"] = f"announced on the network by this app as LM Studio on {_host()} (port {p})"
        except Exception as e:
            STATE["announced"] = f"not announced: {e}"


# --------------------------------------------------------------------------- the same models as the network
def local_models():
    """[{key, variant, vision, size}] of the models on disk here (`lms ls --json`), or None without lms."""
    r = _lms("ls", "--json", timeout=60)
    if r is None or r.returncode != 0:
        return None
    try:
        return [{"key": m.get("modelKey"), "variant": m.get("selectedVariant") or m.get("modelKey"),
                 "vision": bool(m.get("vision")), "size": m.get("sizeBytes")} for m in json.loads(r.stdout)]
    except ValueError:
        return None


def missing_models(network=None):
    """Vision models that other LM Studio servers on the network serve and this computer's hasn't got:
    [{variant, from}]; variant is "<model>@<quantization>", as `lms get` takes it."""
    here = local_models()
    if here is None:
        return []
    have = {m["key"] for m in here}
    want = {}
    for srv in network if network is not None else hardware.lmstudio_network():
        if srv["this_computer"]:
            continue
        for m in srv["models"]:
            if m["vision"] and m["id"] not in have:
                variant = f"{m['id']}@{(m.get('quant') or '').lower()}" if m.get("quant") else m["id"]
                want.setdefault(variant, srv["host"])
    s = share()
    for variant in (_read_index(s) if s and s.is_dir() else {}):          # what's in the shared folder too
        if variant.split("@")[0] not in have:
            want.setdefault(variant, "the shared folder")
    return [{"variant": v, "from": h, "in_share": bool(s and s.is_dir() and v in _read_index(s))}
            for v, h in sorted(want.items())]


def _lms_get(variant, st):
    """`lms get <variant> --yes`, following its progress output (percent, and sizes like 1.2 GB / 17.7 GB)."""
    import re
    lms = lms_path()
    try:
        proc = subprocess.Popen([lms, "get", variant, "--yes"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except OSError as e:
        return False, str(e)
    tail, buf = [], b""
    unit = {"B": 1, "KB": 1e3, "MB": 1e6, "GB": 1e9, "KIB": 1024, "MIB": 2**20, "GIB": 2**30}
    while True:
        ch = proc.stdout.read(1)
        if not ch:
            break
        if ch not in (b"\r", b"\n"):
            buf += ch
            continue
        line = buf.decode("utf-8", "replace").strip()
        buf = b""
        if not line:
            continue
        tail = (tail + [line])[-5:]
        if m := re.search(r"(\d+(?:\.\d+)?)\s*%", line):
            st["pct"] = int(float(m.group(1)))
        sizes = re.findall(r"(\d+(?:\.\d+)?)\s*(KiB|MiB|GiB|KB|MB|GB|B)\b", line, re.I)
        if len(sizes) >= 2:
            st["bytes"] = float(sizes[0][0]) * unit[sizes[0][1].upper()]
            st["total"] = float(sizes[1][0]) * unit[sizes[1][1].upper()]
    proc.wait()
    return proc.returncode == 0, " ".join(tail)[-300:]


def get_models(variants):
    """Download these models with `lms get` on a background thread (when asked in Settings)."""
    if not lms_path():
        raise ValueError("LM Studio's lms command isn't on this computer.")
    if STATE.get("get") and STATE["get"].get("running"):
        return

    def run():
        st = STATE["get"] = {"running": True, "done": [], "now": None, "error": None, "how": None, "pct": None,
                             "notes": []}
        for v in variants:
            st.update(now=v, pct=None, how="copying from the shared folder")
            try:
                if from_share(v, st):                    # the shared folder first (fast, the very same files) ...
                    time.sleep(3)                        # LM Studio notices new model files by itself
                    if any(m["key"] == v.split("@")[0] for m in local_models() or []):
                        st["done"].append(f"{v} (from the shared folder)")
                    else:                                # seen once: its own paused download of the model hid it
                        st["error"] = (f"{v}: copied from the shared folder, but LM Studio doesn't list it yet. If LM "
                                       f"Studio has an unfinished download of it, cancel that in LM Studio's Downloads; "
                                       f"else restart LM Studio.")
                    continue
            except OSError as e:                         # any failure there: straight to the internet, no retry
                st["notes"].append(f"{v}: the shared folder's copy failed ({e}); downloaded from the internet instead")
            st.update(how="downloading with LM Studio", pct=None, bytes=None, total=None, started=time.time())
            ok, why = _lms_get(v, st)                           # ... else LM Studio's own sources
            if not ok:
                st["error"] = f"{v}: {why}"
                break
            st["done"].append(f"{v} (downloaded)")
        st.update(running=False, now=None, how=None)
    threading.Thread(target=run, daemon=True, name="lms-get").start()


# --------------------------------------------------------------------------- every minute, while the app runs
def keep(every=60):
    """app.py: keep LM Studio serving on all interfaces and announced, while this app runs (if LM Studio is here)."""
    def loop():
        while True:
            try:
                if lms_path() or config_path():
                    serve_everywhere()
                    announce()
                    if not (STATE.get("get") or {}).get("running"):
                        threading.Thread(target=publish, daemon=True, name="lms-share").start()
            except Exception as e:                      # never stop the app over this
                STATE["announced"] = f"not announced: {type(e).__name__}: {e}"
            time.sleep(every)
    if os.environ.get("FACE_LMSTUDIO", "").lower() not in ("off", "0", "no"):
        threading.Thread(target=loop, daemon=True, name="lmstudio-keeper").start()


def status():
    """For Settings: LM Studio on this computer, in words."""
    if not (lms_path() or config_path()):
        return {"installed": False}
    out = {"installed": True, "running": running(), "port": port(), "on_network": running() and on_network(),
           "announced": STATE.get("announced"), "bind": STATE.get("bind"), "get": STATE.get("get"),
           "shared": share_status()}
    return out


# --------------------------------------------------------------------------- a shared models folder (e.g. on a NAS)
# Every computer with this app and LM Studio puts its vision models in the shared folder the owner set (Settings ->
# This computer -> Shared models folder); a computer missing one copies it from there (checked by SHA-256), and only
# downloads it with `lms get` when it isn't there. Layout: <share>/models/<publisher>/<repo>/<files> (as in LM Studio's
# models folder), <share>/hub/<owner>/<name>/ (LM Studio's description of the model: its name, e.g. qwen/qwen3.8-27b),
# <share>/index.json: {variant: {hub, files: [{path, size, sha256}], from, added}}.
CONF = paths.HOME / "lmstudio.json"              # {"share": folder}: this computer's own setting (private folder)
INDEX = "index.json"
_share_lock = threading.Lock()


def share():
    try:
        s = json.loads(CONF.read_text(encoding="utf-8")).get("share") or ""
    except (OSError, ValueError):
        s = ""
    return Path(s) if s else None


def set_share(folder):
    folder = str(folder or "").strip().strip('"')
    if folder and not Path(folder).is_dir():
        try:
            Path(folder).mkdir(parents=True, exist_ok=True)
        except OSError as e:
            raise ValueError(f"Can't use {folder}: {e}") from e
    CONF.parent.mkdir(parents=True, exist_ok=True)
    paths.atomic_write(CONF, json.dumps({"share": folder}).encode())
    STATE["share"] = None


def models_dir():
    home = Path.home() / ".lmstudio"
    try:
        d = json.loads((home / "settings.json").read_text(encoding="utf-8")).get("downloadsFolder")
        if d:
            return Path(d)
    except (OSError, ValueError, AttributeError):
        pass
    return home / "models"


def hub_dir():
    return Path.home() / ".lmstudio" / "hub" / "models"


def local_variants():
    """{variant: {hub, files: [relative paths in the models folder]}} for this computer's vision models: each model's
    LM Studio description (hub) says which Hugging Face repos hold it; its files are those of the downloaded
    quantization and its picture projector (mmproj)."""
    here = local_models() or []
    out = {}
    for m in here:
        if not m["vision"] or not m["key"] or "@" not in (m["variant"] or ""):
            continue
        quant = m["variant"].split("@", 1)[1].lower()
        try:
            manifest = json.loads((hub_dir() / m["key"] / "manifest.json").read_text(encoding="utf-8"))
            repos = [f"{s['user']}/{s['repo']}" for d in manifest.get("dependencies", []) for s in d.get("sources", [])
                     if s.get("type") == "huggingface"]
        except (OSError, ValueError, KeyError, TypeError):
            repos = []
        files = []
        for repo in repos:
            folder = models_dir() / repo
            if folder.is_dir():
                files += [f"{repo}/{f.name}" for f in sorted(folder.iterdir()) if f.is_file()
                          and (quant.replace("_", "").lower() in f.name.replace("_", "").lower() or "mmproj" in f.name.lower())]
        if files:
            out[m["variant"]] = {"hub": m["key"], "files": files}
    return out


def _sha256(path, chunk=1 << 22):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while b := f.read(chunk):
            h.update(b)
    return h.hexdigest()


def _copy(src, dst, want_sha=None, progress=None):
    """Copy src to dst through a .part file, hashing as it goes; check the hash. Returns the SHA-256."""
    dst.parent.mkdir(parents=True, exist_ok=True)
    part = dst.with_name(dst.name + ".part")
    h = hashlib.sha256()
    with open(src, "rb") as a, open(part, "wb") as b:
        while chunk := a.read(1 << 22):
            b.write(chunk)
            h.update(chunk)
            if progress:
                progress(len(chunk))
    if want_sha and h.hexdigest() != want_sha:
        part.unlink(missing_ok=True)
        raise OSError(f"{src.name}: copied, but its SHA-256 differs from the shared folder's list")
    os.replace(part, dst)
    return h.hexdigest()


def _read_index(s):
    try:
        return json.loads((s / INDEX).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def publish():
    """Put this computer's vision models in the shared folder, those it hasn't got yet (one at a time)."""
    s = share()
    if not s or not s.is_dir() or not _share_lock.acquire(blocking=False):
        return
    try:
        index = _read_index(s)
        for variant, v in local_variants().items():
            entry = index.get(variant)
            if entry and all((s / "models" / f["path"]).exists() and (s / "models" / f["path"]).stat().st_size == f["size"]
                             for f in entry["files"]):
                continue
            STATE["share"] = f"putting {variant} in the shared folder"
            files = []
            for rel in v["files"]:
                src, dst = models_dir() / rel, s / "models" / rel
                sha = _copy(src, dst)
                files.append({"path": rel, "size": src.stat().st_size, "sha256": sha})
            hub_src, hub_dst = hub_dir() / v["hub"], s / "hub" / v["hub"]
            if hub_src.is_dir():
                shutil.copytree(hub_src, hub_dst, dirs_exist_ok=True)
            index = _read_index(s)                            # another computer may have added to it meanwhile
            index[variant] = {"hub": v["hub"], "files": files, "from": _host(), "added": time.strftime("%Y-%m-%d %H:%M")}
            paths.atomic_write(s / INDEX, json.dumps(index, indent=1).encode())
        STATE["share"] = None
    except OSError as e:
        STATE["share"] = f"couldn't put models in the shared folder: {e}"
    finally:
        _share_lock.release()


def from_share(variant, st=None):
    """Copy one model from the shared folder into LM Studio's models folder, checking each file. True if done."""
    s = share()
    entry = _read_index(s).get(variant) if s and s.is_dir() else None
    if not entry:
        return False
    total = sum(f["size"] for f in entry["files"])
    done = [0]
    if st is not None:
        st.update(total=total, bytes=0, started=time.time())

    def progress(n):
        done[0] += n
        if st is not None:
            st.update(pct=int(done[0] * 100 / max(total, 1)), bytes=done[0])
    added = []
    try:
        for f in entry["files"]:
            dst = models_dir() / f["path"]
            if dst.exists() and dst.stat().st_size == f["size"] and _sha256(dst) == f["sha256"]:
                progress(f["size"])
                continue
            _copy(s / "models" / f["path"], dst, f["sha256"], progress)
            added.append(dst)
    except OSError:
        for dst in added:                                 # no half model for LM Studio to find
            dst.unlink(missing_ok=True)
        raise
    hub_src = s / "hub" / entry["hub"]
    if hub_src.is_dir():
        shutil.copytree(hub_src, hub_dir() / entry["hub"], dirs_exist_ok=True)
    return True


def share_status():
    s = share()
    if not s:
        return {"share": None}
    index = _read_index(s) if s.is_dir() else {}
    return {"share": str(s), "reachable": s.is_dir(), "models": sorted(index), "note": STATE.get("share")}
