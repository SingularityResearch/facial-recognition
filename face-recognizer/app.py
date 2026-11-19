"""
app.py - Face Recognizer: the live tracker and the labeling tool in one window, as two tabs.

The two stay separate programs inside: each keeps its own web server (tracker.py on 8766,
label_tool.py on 8765) and page, exactly as when started from recognize.bat / label.bat, and this
window shows both pages, one per tab. The tracker keeps running while the Labeling tab is shown.
If either is already running from its .bat, the window says so instead of starting a second one (start).

Run:  python app.py            (or FaceRecognizer.exe, built with build_app.bat; the app on macOS and Linux)
      python app.py --browser  (the same tab page in the default web browser instead of a window of its own;
                                FACE_UI=browser does the same)
      FACE_UI=none             (the servers only: no window and no browser; for test copies, which then never open
                                anything on the desktop: open http://127.0.0.1:8764/ yourself)
Needs pywebview (pip install pywebview). The window is Edge WebView2 on Windows, WebKit on macOS, and GTK or Qt on
Linux; where none can open or draw (a Linux system without those libraries, or without working OpenGL), the app
falls back to the browser by itself: on Linux it first tries the window in a child process (--probe-window).
"""
import json
import os
import re
import shutil
import sys
import threading
import time
import traceback
import urllib.request
from pathlib import Path
from urllib.parse import parse_qsl, urlparse

import desktop
import localguard
import paths

# One thread for numpy's own maths (OpenBLAS) on Linux, set before anything loads numpy. The packed Linux app carries
# three copies of OpenBLAS (numpy's, SciPy's, OpenCV's); with their thread pools, a matrix product in the labeling tool
# while the tracker's models loaded got stuck for good in 2 starts of 4 (2026-11-13: the Labeling tab and the sync
# waited behind it). With one thread: 6 starts of 6 clean. The matrices here are small, and the models run in ONNX
# Runtime, which this doesn't touch.
if sys.platform.startswith("linux"):
    os.environ.setdefault("OPENBLAS_NUM_THREADS", "1")


def keep_font_list(env=os.environ, built=None):
    """The built app: matplotlib's folder is one that stays. MediaPipe imports matplotlib, and PyInstaller's start-up
    hook points it at a new temporary folder at every start, so it listed the system's fonts again each time: more
    than 5 s of every start on a Mac ("Matplotlib is building the font cache" in the log, found on the first real
    one). In the private folder the list is made once. Before anything imports MediaPipe."""
    if getattr(sys, "frozen", False) if built is None else built:
        env["MPLCONFIGDIR"] = str(paths.HOME / "matplotlib")


keep_font_list()

TITLE = "Face Recognizer"
VERSION = "1.24.6"                             # this build; GitHub release tags are "v" + VERSION
REPO = "SingularityResearch/facial-recognition"
HOME = paths.HOME                                  # %APPDATA%\FaceLabels: private, outside the project

HERE = paths.HERE                                  # the page files (inside the .exe when built)
WINDOW_ICON = HERE / "icons" / "icon.png"          # the app's icon (tools/make_icon.py), for the Linux window


def ports(env=os.environ):
    """(tab page, labeling tool, tracker) ports: 8764, 8765, 8766, or from FACE_PORTS=<n> (n, n+1, n+2), so a second
    copy with other folders (FACE_ROOT, FACE_HOME: a test build, say) runs beside the usual one."""
    raw = (env.get("FACE_PORTS") or "").strip()
    base = int(raw) if raw.isdigit() else 8764
    if not 1024 <= base <= 65000:
        raise SystemExit(f"FACE_PORTS must be between 1024 and 65000, not {raw}")
    return base, base + 1, base + 2


# Each program runs on the first free port from its own (up to PORT_TRIES), or a copy already running there is used
SHELL_PORT, LABEL_PORT, TRACK_PORT = ports()
PORT_TRIES = 10
TOOL_PORTS = range(LABEL_PORT, TRACK_PORT + PORT_TRIES)     # every port the two programs' pages can be on

# The tab page: app_ui.html (layout and style) and app_ui.js (tabs, status, Settings tab)
PAGES = {"/": (HERE / "app_ui.html", "text/html; charset=utf-8"),
         "/app_ui.js": (HERE / "app_ui.js", "text/javascript; charset=utf-8"),
         "/ask.js": (HERE / "ask.js", "text/javascript; charset=utf-8")}      # questions asked in the page (no confirm())


def answering(url, app):
    """True if one of our programs (app = 'face-labels' or 'tracker') is already serving at url."""
    try:
        path = "api/summary" if app == "face-labels" else "api/people"
        req = urllib.request.Request(url + path, headers={"X-Face-Tool": "1"})
        with urllib.request.urlopen(req, timeout=3) as r:
            body = json.loads(r.read())
        return body.get("app") == "face-labels" if app == "face-labels" else "people" in body
    except Exception:
        return False


def start(name, module, port, app):
    """Start one program's server in this process on the first free port from `port`. Returns its URL once it
    answers (models loaded). A copy already answering there that this process didn't start is an error."""
    for p in range(port, port + PORT_TRIES):
        url = f"http://127.0.0.1:{p}/"
        try:
            srv = module.bind(p) if hasattr(module, "bind") else bind(p)
        except OSError:
            if answering(url, app):
                if url in OWN:                       # this process's own, answering again (Start again)
                    print(f"{name}: still serving at {url}")
                    return url
                # Not started here: its .bat, another Face Recognizer window, or some other program answering
                # like one of ours. It is never shown in this window: through 1.24.2 the app took whatever
                # answered there and named this window's tab page in its frame-ancestors as its own, and so
                # answered its camera and microphone request (camera_allowed). Starting a second copy instead
                # would have two programs writing the same data files, so ask for that copy to be closed.
                raise OSError(f"It is already running at {url}, started on its own (its .bat) or by another Face "
                              f"Recognizer window, and a copy started that way can't be shown in this window. "
                              f"\nClose that copy (its black "
                              f"window) and start the app again, or keep using it in your browser at {url}") from None
            continue
        print(f"{name}: loading ...")
        try:
            module.attach(srv)
        except BaseException:
            srv.server_close()                       # free the port for a later Start again
            raise
        OWN[url] = threading.Thread(target=srv.serve_forever, daemon=True, name=name)
        OWN[url].start()
        print(f"{name}: serving at {url}")
        return url
    raise OSError(f"No free port between {port} and {port + PORT_TRIES - 1}")


def bind(port):
    return localguard.bind(port)                     # no address reuse on Windows: a second copy gets OSError


STATUS = {"track": {}, "label": {}}         # {"url": ...} once serving, {"error": ...} if it failed
SHELL = {}                                  # {"url": the tab page's own address} once its server is up


def latest_release():
    """{latest, newer} from GitHub's latest release of this project, or {error}. Only when asked
    (the window's "Check for updates"): the app contacts nothing on its own except the first-start
    model download."""
    try:
        req = urllib.request.Request(f"https://api.github.com/repos/{REPO}/releases/latest",
                                     headers={"Accept": "application/vnd.github+json", "User-Agent": "FaceRecognizer"})
        with urllib.request.urlopen(req, timeout=10) as r:
            tag = json.loads(r.read()).get("tag_name", "")
        num = lambda v: tuple(int(x) for x in re.findall(r"\d+", v)[:3])
        return {"latest": tag, "newer": num(tag) > num(VERSION)}
    except Exception as e:
        return {"error": str(e)}


def data_dir():
    """The programs' data folder (face_id.DATA): data\\ beside the .exe, or beside the source files."""
    return paths.ROOT / "data"


def shell_handler():
    """The tab page's request handler. The same request checks as the two programs (localguard): the right
    Host; and for anything that does something (contacts GitHub, opens a browser or a folder) the X-Face-Tool
    header, which only this page sends."""
    acts = ("/update", "/open-release", "/open", "/attention", "/restart", "/restart-app")

    class Handler(localguard.Handler):
        frame_ancestors = "'none'"                       # the window's own page: never inside another page
        # beyond localguard.POLICY: the two programs' pages in its frames (on 127.0.0.1, any of their ports); no
        # pictures of its own beyond its server's
        policy = {"frame-src": " ".join(f"http://127.0.0.1:{p}" for p in TOOL_PORTS), "img-src": "'self'"}
        # camera and microphone: handed on only to a program's page (their ports), and of those only to a frame
        # that asks for them (allow= in app_ui.html: the tracker's frame alone). "self" has to be there: a page
        # can't hand on what it may not use (without it the tracker was refused outright; this page's own
        # scripts never use either). See camera_allowed.
        permissions = ", ".join(f"{what}=(self {' '.join(f'"http://127.0.0.1:{p}"' for p in TOOL_PORTS)})"
                                for what in ("camera", "microphone"))

        def _get(self):
            path = self.path.split("?", 1)[0]
            # the page's files to anyone on this computer; what the window is doing only to the page itself
            if self.refused(write=path in acts, own=path not in PAGES and path != "/favicon.ico"):
                return
            if path == "/favicon.ico":
                return self.favicon()
            if path in PAGES:
                file, ctype = PAGES[path]
                return self.send(file.read_text("utf-8").replace("{{VERSION}}", VERSION).encode(), ctype)
            if path == "/attention":                     # alerts: the window's title and taskbar button
                q = dict(parse_qsl(self.path.partition("?")[2]))
                attention(int(q.get("unread", "0") or 0), q.get("flash") == "1")
                return self.send({})
            if path == "/alive":                         # the window's page runs (watch_window)
                ALIVE["at"] = time.monotonic()
                return self.send({})
            if path == "/restart-app":                   # the camera won't start even after a page reload
                return self.send({"restarting": restart_soon("the camera wouldn't start")})
            if path == "/restart":                       # a program that failed or stopped: Start again
                what = dict(parse_qsl(self.path.partition("?")[2])).get("what")
                if what not in NAMES:
                    return self.send({"error": "unknown program"}, code=400)
                return self.send({"what": what, "now": restart(what)})
            if path not in ("/status", "/paths") + acts:
                return self.send({"error": "not found"}, code=404)
            if path == "/open-release":                 # in the normal browser, not inside this window
                import webbrowser
                webbrowser.open(f"https://github.com/{REPO}/releases/latest")
                body = {}
            elif path in ("/paths", "/open"):             # the Settings tab's data folders
                where = {"data": data_dir(), "private": HOME}
                if path == "/open":
                    d = where.get(self.path.split("what=", 1)[-1])
                    if d:
                        d.mkdir(parents=True, exist_ok=True)
                        desktop.open_folder(d)
                body = {k: str(v) for k, v in where.items()}
            elif path == "/update":
                body = latest_release()
            else:
                body = STATUS
            self.send(body)

    return Handler


def shell_server():
    """The tab page itself, on 127.0.0.1 (an http://127.0.0.1 page counts as secure, which the camera
    needs; so must the page around the tracker's frame)."""
    try:
        srv = bind(SHELL_PORT)                      # fixed, so the page's remembered tab survives restarts
    except OSError:
        srv = bind(0)                               # taken (a second copy?): any free port
    srv.RequestHandlerClass = shell_handler()
    port = srv.server_address[1]
    SHELL["url"] = f"http://127.0.0.1:{port}/"
    # the programs this window starts may be framed by this page (and only then: see localguard.FRAME_PARENTS)
    localguard.FRAME_PARENTS.update({f"http://127.0.0.1:{port}", f"http://localhost:{port}"})
    threading.Thread(target=srv.serve_forever, daemon=True, name="shell").start()
    return f"http://127.0.0.1:{srv.server_address[1]}/"


WINDOW = None                               # the app window, once made (main)

# ---- the window's page must never stay frozen ----------------------------------------------------------------------
# The tab page asks /alive every 2 s. Seen on the Linux laptop (Qt WebEngine): the page's renderer stopped running
# its scripts for hours while the process lived on, idle (no camera, no stream, nothing drawn). The cause found on
# 2026-11-12: every answer a page script didn't read kept a file descriptor open in the renderer, which froze at its
# limit (1024: ten minutes after a start; 8192 after a restart in place: every 90 minutes). The scripts let unread
# answers go now (drop), and allow_more_open_files gives a margin; this watchdog stays for whatever comes next. After HUNG_AFTER s
# without a word the page's renderer is stopped (Linux, macOS: Qt loads the page again, recover_renderer); if the
# page still says nothing RESTART_AFTER s later, or on Windows, the whole app starts again in place.
ALIVE = {"at": None}
HUNG_AFTER = 30
RESTART_AFTER = 30


def watch_step(st, now, alive_at):
    """The watchdog's decision at monotonic time now: None, "kill" (the renderer) or "restart" (the app). st keeps
    {"tick", "killed"} between calls."""
    slept = st.get("tick") is not None and now - st["tick"] > 20     # the computer slept: start counting again
    st["tick"] = now
    if slept:
        st["killed"] = None
        return "reset"
    if alive_at is None or now - alive_at < HUNG_AFTER:
        st["killed"] = None
        return None
    if st.get("killed") is None:
        st["killed"] = now
        return "kill" if sys.platform != "win32" else None
    if now - st["killed"] >= RESTART_AFTER:
        st["killed"] = None
        return "restart"
    return None


def _parent(pid):
    try:
        if sys.platform.startswith("linux"):
            for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                if line.startswith("PPid:"):
                    return int(line.split()[1])
        else:
            import subprocess
            return int(subprocess.run(["ps", "-o", "ppid=", "-p", str(pid)], capture_output=True, text=True,
                                      timeout=5).stdout.strip() or 0)
    except Exception:
        return 0
    return 0


def renderer_pids():
    """This app's page renderers (QtWebEngineProcess --type=renderer started by this process), Linux and macOS."""
    me, out = os.getpid(), []
    try:
        if sys.platform.startswith("linux"):
            cands = []
            for d in Path("/proc").iterdir():
                if d.name.isdigit():
                    try:
                        cmd = (d / "cmdline").read_bytes()
                    except OSError:
                        continue
                    if b"QtWebEngineProcess" in cmd.split(b"\0")[0] and b"--type=renderer" in cmd:
                        cands.append(int(d.name))
        else:
            import subprocess
            r = subprocess.run(["pgrep", "-f", "QtWebEngineProcess.*--type=renderer"], capture_output=True,
                               text=True, timeout=5)
            cands = [int(p) for p in r.stdout.split()]
    except Exception:
        return []
    for pid in cands:
        p = pid
        for _ in range(6):                       # renderer -> (zygote ->) ... -> this app
            p = _parent(p)
            if p in (0, 1):
                break
            if p == me:
                out.append(pid)
                break
    return out


def restart_app():
    """Start this app again. Linux and macOS: in place (exec, same process id; the exec frees the ports). Windows has
    no real exec (os.execve starts a new process and ends this one, so the new copy could find the ports still
    taken and move to others): the new copy is started with FACE_RESTART_AFTER=<this pid> and waits for this process
    to be gone before it takes the ports (wait_for_previous)."""
    exe = sys.executable
    args = [exe] + ([] if getattr(sys, "frozen", False) else [str(Path(__file__).resolve())]) + sys.argv[1:]
    env = dict(os.environ, PYINSTALLER_RESET_ENVIRONMENT="1")       # a fresh start for a PyInstaller build
    try:
        sys.stdout.flush()
    except Exception:
        pass
    if sys.platform == "win32":
        import subprocess
        env["FACE_RESTART_AFTER"] = str(os.getpid())
        subprocess.Popen(args, env=env, close_fds=True)
        os._exit(0)
    os.execve(exe, args, env)


def wait_for_previous(timeout=30):
    """A copy started by restart_app on Windows: wait (up to timeout s) until the copy that started it has ended, so
    its ports are free."""
    pid = os.environ.pop("FACE_RESTART_AFTER", "")
    if sys.platform != "win32" or not pid.isdigit():
        return
    import ctypes
    k = ctypes.windll.kernel32
    h = k.OpenProcess(0x00100000, False, int(pid))        # SYNCHRONIZE; 0 when it has ended already
    if h:
        try:
            k.WaitForSingleObject(h, int(timeout * 1000))
        finally:
            k.CloseHandle(h)
    time.sleep(0.5)                                       # its window's helper processes end with it


RESTART_EVERY = 3600                        # camera restarts: at most once an hour (HOME/restarted.json)
WINDOW_BACKOFF = 120                        # s: window restarts in a row wait 2, 4, 8 ... min, at most an hour


def _restarts():
    """HOME/restarted.json: {"camera": {at, why}, "window": {at, n}} (older files held only the camera's)."""
    try:
        d = json.loads((HOME / "restarted.json").read_text("utf-8"))
    except (OSError, ValueError):
        return {}
    if not isinstance(d, dict):
        return {}
    return {"camera": d} if "at" in d else d


def _note_restart(kind, rec):
    d = _restarts()
    d[kind] = rec
    paths.atomic_write(HOME / "restarted.json", json.dumps(d).encode())


def window_restart_due(rec, now):
    """(due, n) for the watchdog's restart: the first at once; while they keep coming, each waits twice as long as
    the one before (WINDOW_BACKOFF, 2x, 4x ... at most RESTART_EVERY), so a page that always hangs can't keep the app
    restarting every minute; an hour without one starts over. n: restarts in the current run of them."""
    try:
        at, n = float(rec.get("at", 0)), int(rec.get("n", 0))
    except (AttributeError, TypeError, ValueError):
        at, n = 0.0, 0
    if now - at >= RESTART_EVERY:
        n = 0
    wait = 0 if n == 0 else min(RESTART_EVERY, WINDOW_BACKOFF * 2 ** (n - 1))
    return now - at >= wait, n


def restart_soon(why, locked=None):
    """Start the app again in a moment (the answer goes out first): only in the app window, and at most once every
    RESTART_EVERY s, so a camera that's really broken can't keep the app restarting. True when it will.

    Under a locked screen (Windows; `locked`: asked of the system here when not given) the rule is once per lock
    instead, whatever the hour says. A Remote Desktop disconnect locks the session, Windows stops the camera 20 s
    later, and the running app doesn't get it back: a start from its page fails at once, while the app started
    again has it within seconds (seen 2026-11-15). Locked at the computer's own screen that changes nothing, so
    it isn't done twice: the record keeps `locked` until the screen is seen unlocked (unlock_seen)."""
    if WINDOW is None:
        return False
    rec = _restarts().get("camera", {})
    try:
        last = float(rec.get("at", 0))
    except (AttributeError, TypeError, ValueError):
        rec, last = {}, 0.0
    if locked is None:
        locked = desktop.screen_locked() is True
    if locked:
        if rec.get("locked"):
            print(f"Not starting the app again ({why}): it did so once since the screen was locked.")
            return False
    elif time.time() - last < RESTART_EVERY:
        print(f"Not starting the app again ({why}): it did so less than {RESTART_EVERY // 60} min ago.")
        return False
    try:
        _note_restart("camera", {"at": time.time(), "why": why, "locked": bool(locked)})
    except OSError:
        return False
    print(f"Starting the app again: {why}{' (the screen is locked)' if locked else ''}.")
    threading.Timer(1.0, restart_app).start()
    return True


def unlock_seen(locked=None):
    """The screen isn't locked (any more): the next lock gets its one restart again (restart_soon). Asked by the
    window's watchdog every few seconds; reads the record only, and writes it when there is something to clear."""
    try:
        rec = _restarts().get("camera", {})
        if not isinstance(rec, dict) or not rec.get("locked"):
            return False
        if (desktop.screen_locked() if locked is None else locked) is not False:
            return False
        _note_restart("camera", {**rec, "locked": False})
        return True
    except Exception:
        return False


WATCHING = {"on": True}                     # off once there is no app window to watch (the browser is used instead)
MEMORY_EVERY = 1800                         # s: how often the window's memory goes to the log (note_memory)


def descendants(table, root):
    """The process ids below root in table {pid: parent pid} (root itself not among them)."""
    kids = {}
    for pid, parent in table.items():
        kids.setdefault(parent, []).append(pid)
    out, todo = set(), list(kids.get(root, []))
    while todo:
        p = todo.pop()
        if p != root and p not in out:
            out.add(p)
            todo += kids.get(p, [])
    return sorted(out)


def _processes():
    """({pid: parent pid} of every process, memory(pid) -> the bytes that process has to itself, or None when it
    can't be read). Windows and Linux; ({}, None) elsewhere."""
    if sys.platform == "win32":
        import ctypes
        from ctypes import wintypes
        k = ctypes.WinDLL("kernel32", use_last_error=True)

        class Entry(ctypes.Structure):               # PROCESSENTRY32W
            _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD), ("th32ProcessID", wintypes.DWORD),
                        ("th32DefaultHeapID", ctypes.c_size_t), ("th32ModuleID", wintypes.DWORD),
                        ("cntThreads", wintypes.DWORD), ("th32ParentProcessID", wintypes.DWORD),
                        ("pcPriClassBase", wintypes.LONG), ("dwFlags", wintypes.DWORD),
                        ("szExeFile", wintypes.WCHAR * 260)]

        class Counters(ctypes.Structure):            # PROCESS_MEMORY_COUNTERS_EX
            _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
                (n, ctypes.c_size_t) for n in ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                                               "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                                               "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage",
                                               "PrivateUsage")]
        k.CreateToolhelp32Snapshot.restype = k.OpenProcess.restype = wintypes.HANDLE
        k.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
        k.Process32FirstW.argtypes = k.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Entry)]
        k.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        k.CloseHandle.argtypes = [wintypes.HANDLE]
        k.K32GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
        snap = k.CreateToolhelp32Snapshot(2, 0)      # TH32CS_SNAPPROCESS
        if not snap or snap == wintypes.HANDLE(-1).value:
            return {}, None
        table, e = {}, Entry()
        e.dwSize = ctypes.sizeof(Entry)
        more = k.Process32FirstW(snap, ctypes.byref(e))
        while more:
            table[e.th32ProcessID] = e.th32ParentProcessID
            more = k.Process32NextW(snap, ctypes.byref(e))
        k.CloseHandle(snap)

        def memory(pid):
            h = k.OpenProcess(0x1000, False, pid)    # PROCESS_QUERY_LIMITED_INFORMATION
            if not h:
                return None
            try:
                c = Counters()
                c.cb = ctypes.sizeof(Counters)
                return c.PrivateUsage if k.K32GetProcessMemoryInfo(h, ctypes.byref(c), c.cb) else None
            finally:
                k.CloseHandle(h)
        return table, memory
    if sys.platform.startswith("linux"):
        table = {}
        for d in Path("/proc").iterdir():
            if d.name.isdigit():
                try:                                 # "pid (name) state parent ...": the name may hold spaces and )
                    table[int(d.name)] = int((d / "stat").read_text().rsplit(")", 1)[1].split()[1])
                except (OSError, ValueError, IndexError):
                    pass

        def memory(pid):
            try:
                for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                    if line.startswith("RssAnon:"):
                        return int(line.split()[1]) * 1024
            except (OSError, ValueError, IndexError):
                pass
            return None
        return table, memory
    return {}, None


def window_memory():
    """What the app's window uses: (how many processes it has, the MB they have to themselves in all, the MB of the
    largest, the MB of this process). The window's processes are this one's descendants (the web view's browser,
    page renderer, graphics and camera processes). None where it can't be read (macOS)."""
    try:
        table, memory = _processes()
        if not table:
            return None
        sizes = [m for m in (memory(p) for p in descendants(table, os.getpid())) if m]
        mb = 1024 * 1024
        return len(sizes), sum(sizes) / mb, max(sizes, default=0) / mb, (memory(os.getpid()) or 0) / mb
    except Exception:
        return None


def note_memory():
    """A line in the log, every MEMORY_EVERY s: a page that keeps growing shows here long before it runs out. The
    NAS's page did after 14 hours (2026-11-14, a renderer "OutOfMemory"), and nothing had recorded how it grew."""
    m = window_memory()
    if m:
        print(f"Memory: the window's {m[0]} processes hold {m[1]:,.0f} MB (the largest {m[2]:,.0f} MB), "
              f"the app itself {m[3]:,.0f} MB.")


def watch_window():
    """Watch the window's page (ALIVE) and bring it back when it stops (watch_step)."""
    import signal

    def run():
        st = {}
        noted = time.monotonic() - MEMORY_EVERY + 120    # the first memory line two minutes after the start
        while WATCHING["on"]:
            time.sleep(5)
            now = time.monotonic()
            if now - noted >= MEMORY_EVERY:
                noted = now
                note_memory()
            unlock_seen()
            what = watch_step(st, now, ALIVE["at"])
            if what == "reset" and ALIVE["at"] is not None:
                ALIVE["at"] = now
            elif what == "kill":
                pids = renderer_pids()
                print(f"The window's page hasn't answered for {now - ALIVE['at']:.0f} s: stopping its renderer "
                      f"{pids} so it loads again.")
                for p in pids:
                    try:
                        os.kill(p, signal.SIGKILL)
                    except OSError as e:
                        print(f"Couldn't stop {p}: {e}")
            elif what == "restart":
                due, n = window_restart_due(_restarts().get("window", {}), time.time())
                if not due:
                    if not st.get("held"):
                        print(f"The window's page hasn't answered for {now - ALIVE['at']:.0f} s, but the app was "
                              f"started again for that {n} time(s) lately: waiting longer before the next.")
                    st["held"] = True
                    continue
                print(f"The window's page hasn't answered for {now - ALIVE['at']:.0f} s: starting the app again.")
                try:
                    _note_restart("window", {"at": time.time(), "n": n + 1})
                    restart_app()
                except Exception:
                    traceback.print_exc()
            elif ALIVE["at"] is not None and now - ALIVE["at"] < HUNG_AFTER:
                st["held"] = False
    threading.Thread(target=run, daemon=True, name="window watchdog").start()


def attention(unread, flash):
    """New alerts (the tracker's inbox): say how many in the window's title, which shows on its taskbar button, and
    when one just happened draw the eye to the window (desktop.attention: taskbar flash, Dock bounce, urgent)."""
    w = WINDOW
    if w is None:
        return
    unread = max(0, min(unread, 9999))
    try:
        w.set_title(f"{TITLE} · {unread} new alert{'s' if unread != 1 else ''}" if unread else TITLE)
    except Exception:
        pass
    if flash:
        desktop.attention(w)


def camera_allowed(uri):
    """True if a request for the camera and microphone made in the name of uri is answered without a prompt: the
    tracker this process started (its address in STATUS and in OWN), or this window's own tab page while that
    tracker is up; not any other program on 127.0.0.1, and not a tracker this process didn't start.
    The tab page: WebView2 says a framed page's request comes from the page around it (seen with runtime 157,
    2026-11-16: the tracker's request arrived as the tab page's and was left to WebView2's own question, which on a
    new copy nobody answers; the three older devices had "allowed" remembered in their profiles). The tab page's
    own scripts never use camera or microphone, and it hands them to the tracker's frame alone: allow= on that
    frame in app_ui.html, and its Permissions-Policy header (shell_handler) names only the programs' ports."""
    track = STATUS["track"].get("url")
    if not track or track not in OWN:
        return False
    u = urlparse(uri)
    return any((u.scheme, u.hostname, u.port) == (a.scheme, a.hostname, a.port)
               for a in map(urlparse, filter(None, (track, SHELL.get("url")))))


def camera_hook(core, kinds, allow, say=print):
    """Answer the permission requests of core (the window's CoreWebView2): `allow` for the tracker this window
    shows (camera_allowed) asking for one of `kinds`; anything else is left to WebView2's own question. Each kind
    of request and its answer is said once (the log), so a camera that never starts can be told from one that was
    never asked for. Returns the handler."""
    said = set()

    def on_request(_, e):
        uri, kind = str(e.Uri), e.PermissionKind
        ok = kind in kinds and camera_allowed(uri)
        if ok:
            e.State = allow
        if (uri, str(kind), ok) not in said:
            said.add((uri, str(kind), ok))
            say(f"Permission: {uri} asked for {kind}: {'allowed' if ok else 'left to the window to ask'}")
    core.PermissionRequested += on_request
    return on_request


def allow_camera(window, early=False):
    """Let this window's tracker page use the camera and microphone without asking each time (WebView2 asks by
    default). Any other page, including other programs on 127.0.0.1, still gets WebView2's prompt. Windows only: on
    macOS the system asks once for the app (the .app's NSCameraUsageDescription) and the window's own question is
    answered by allow_camera_macos; on Linux the window toolkit asks.
    early: called before the window is shown (its own thread, WebView2 not made yet): the answer is put in place
    the moment WebView2 is made, before any page loads. Through 1.23.0 it was only put in place once the tab page had
    loaded, which waits for the pages in its tabs when they are already up: the tracker then asked first, WebView2
    showed its own question, and while that stood unanswered it raised no further request, so on a new copy (nothing
    remembered yet) the camera never started, through page reloads and app restarts (found on a fourth device,
    2026-11-16; the three others had the answer remembered from earlier). Called again when the page has loaded,
    in case that came too late."""
    if sys.platform != "win32" or getattr(window, "_camera_ok", False):
        return
    try:
        from System import Action
        from Microsoft.Web.WebView2.Core import CoreWebView2PermissionKind as Kind, CoreWebView2PermissionState as State

        def attach(core):
            if core is not None and not getattr(window, "_camera_ok", False):
                # the tracker's own alerts may show as Windows notifications too
                camera_hook(core, (Kind.Camera, Kind.Microphone, Kind.Notifications), State.Allow)
                window._camera_ok = True

        view = window.native.webview
        if early:
            def ready(sender, _args):
                try:
                    attach(sender.CoreWebView2)
                except Exception:
                    print("Camera permission: WebView2 will ask (couldn't pre-allow):")
                    traceback.print_exc()
            view.CoreWebView2InitializationCompleted += ready
            attach(view.CoreWebView2)                # None until it is made
        else:
            window.native.Invoke(Action(lambda: attach(view.CoreWebView2)))
    except Exception:
        print("Camera permission: WebView2 will ask (couldn't pre-allow):")
        traceback.print_exc()


CAMERA_ASKED = b"webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:"
ASK_USER, GRANT = 0, 1                           # WebKit's WKPermissionDecision: prompt, grant
_MAC_CAMERA = []                                 # the window's delegate class with the answer, once made


def camera_answer(origin):
    """What the macOS window answers a page that asks for the camera or microphone: GRANT to the tracker this
    window started (camera_allowed), ASK_USER for any other page. `origin` is WebKit's WKSecurityOrigin."""
    try:
        uri = f"{origin.protocol()}://{origin.host()}:{int(origin.port())}"
    except Exception:
        return ASK_USER
    return GRANT if camera_allowed(uri) else ASK_USER


def allow_camera_macos():
    """macOS: let this window's tracker page use the camera and microphone without the window asking. WebKit asks
    its window about every page that wants them, and a window that doesn't answer shows "Allow 127.0.0.1 to use
    your camera?" each time the page loads (it never remembers): after a restart or a reload nobody is there to
    answer, and the camera stays off. pywebview's window doesn't answer, so its delegate class is given the answer
    here, before the window is made. The system's own question for the app (once, remembered) stays. True if it is
    in place."""
    if sys.platform != "darwin":
        return False
    if _MAC_CAMERA:
        return True
    try:
        import objc
        from webview.platforms import cocoa
        # the last argument is a block taking the decision (an integer)
        objc.registerMetaDataForSelector(b"NSObject", CAMERA_ASKED, {
            "required": False, "retval": {"type": b"v"},
            "arguments": {2: {"type": b"@"}, 3: {"type": b"@"}, 4: {"type": b"@"}, 5: {"type": b"q"},
                          6: {"type": b"@?", "callable": {"retval": {"type": b"v"},
                                                         "arguments": {0: {"type": b"^v"}, 1: {"type": b"q"}}}}}})

        class FaceRecognizerBrowserDelegate(cocoa.BrowserView.BrowserDelegate):
            @objc.typedSelector(b"v@:@@@q@?")
            def webView_requestMediaCapturePermissionForOrigin_initiatedByFrame_type_decisionHandler_(
                    self, webview, origin, frame, kind, handler):
                handler(camera_answer(origin))

        if not FaceRecognizerBrowserDelegate.instancesRespondToSelector_(CAMERA_ASKED):
            raise RuntimeError("the window's delegate didn't take the answer")
        cocoa.BrowserView.BrowserDelegate = FaceRecognizerBrowserDelegate
        _MAC_CAMERA.append(FaceRecognizerBrowserDelegate)
        return True
    except Exception:
        print("Camera permission: the window will ask each time the page loads (couldn't pre-allow):")
        traceback.print_exc()
        return False


def unhook_requests(core, browser, every):
    """Take pywebview's two request handlers off the window's WebView2 (core: its CoreWebView2; browser:
    pywebview's object holding the handlers; every: WebView2's value for "all kinds of request")."""
    core.WebResourceResponseReceived -= browser.on_web_resource_response
    core.WebResourceRequested -= browser.on_web_resource_request
    core.RemoveWebResourceRequestedFilter("*", every)


def drop_request_hooks(window):
    """Windows: stop pywebview (6.2.1) passing every request of the window's pages through its own two handlers.
    They serve its request_sent and response_received events, which this app doesn't use, and the second
    (WebResourceResponseReceived) switches on the browser's network inspection in the page's process, which keeps
    a record of every request for as long as the page lives. With tens of requests a second (camera frames out
    and in) the page grew by 1.5 to 2 GB an hour and ran out of memory after 14 hours on one device. Found
    2026-11-15 in Chromium's own memory dump of the page (blink::NetworkResourcesData::ResourceData); the same
    page in the same window without the handlers stayed at 50 MB where it had 431 MB after five minutes with
    them, and got a fifth more camera frames through. Once per window, when its page has loaded.
    tests/test_window_watchdog.py checks that pywebview still hooks requests this way."""
    if sys.platform != "win32" or getattr(window, "_requests_unhooked", False):
        return
    window._requests_unhooked = True
    try:
        from System import Action
        from Microsoft.Web.WebView2.Core import CoreWebView2WebResourceContext as Context

        def hook():
            try:
                unhook_requests(window.native.webview.CoreWebView2, window.native.browser, Context.All)
            except Exception:
                print("The window's request handlers stay on (its page's memory will grow):")
                traceback.print_exc()
        window.native.Invoke(Action(hook))
    except Exception:
        print("The window's request handlers stay on (its page's memory will grow):")
        traceback.print_exc()


def recover_renderer(window):
    """When the window's page renderer dies (crashed, killed, out of memory) the window stays black or white with
    nothing drawn (seen on the Linux laptop): log why and load the page again. Qt WebEngine (Linux, macOS):
    renderProcessTerminated; WebView2 (Windows): ProcessFailed. Once per window."""
    if getattr(window, "_recover_on", False):
        return
    window._recover_on = True
    try:
        view = window.native.webview
    except Exception:
        return
    if sys.platform == "win32":
        try:
            from System import Action

            def on_failed(_, e):
                kind = str(e.ProcessFailedKind)
                why = f"exit {getattr(e, 'ExitCode', '?')}, {getattr(e, 'Reason', '?')}, " \
                      f"{getattr(e, 'ProcessDescription', '') or 'no description'}"
                # only a page's own renderer needs the page loaded again; the GPU and utility processes (video
                # capture, audio, network) restart by themselves, and a reload would restart every camera
                again = "Render" in kind or "Frame" in kind
                print(f"The window's {kind} stopped ({why}){': loading the page again' if again else ''}.")
                if again:
                    view.Reload()

            def hook():
                view.CoreWebView2.ProcessFailed += on_failed
            window.native.Invoke(Action(hook))
        except Exception:
            traceback.print_exc()
        return
    try:
        from qtpy.QtCore import QTimer

        def on_terminated(status, code):
            print(f"The window's page renderer stopped ({status}, exit {code}): loading the page again.")
            QTimer.singleShot(1000, view.reload)
        view.page().renderProcessTerminated.connect(on_terminated)
    except Exception:
        traceback.print_exc()


NAMES = {"track": "Face tracker", "label": "Face Labels"}
_restart_lock = threading.Lock()


OWN = {}                                    # url -> the thread serving a program started in this process


def restart(key):
    """Start one program again (the window's Start again button): after it failed (say the model download), or
    when it stopped answering. Returns "running" (it answers: nothing to do), "starting" or "busy" (already
    starting). A new copy may come up on the next port; the tab page takes its URL from /status."""
    with _restart_lock:
        s = STATUS.get(key, {})
        if not s.get("url") and not s.get("error"):
            return "busy"
        if s.get("url") and answering(s["url"], "tracker" if key == "track" else "face-labels"):
            return "running"
        if s.get("url") in OWN and OWN[s["url"]].is_alive():
            # It is loaded in this process and only slow or answering with an error. Starting it "again" found its
            # own port taken, moved to the next and loaded a second copy onto the same data files: two labeling
            # databases, two syncs, two importers. The page is just loaded again instead.
            return "running"
        keys = [key] + [k for k in NAMES if k != key and STATUS.get(k, {}).get("error")]   # a failed download: both
        for k in keys:
            print(f"{NAMES[k]}: starting again")
            STATUS[k] = {}
        run_programs(tuple(keys))
        return "starting"


def run_programs(keys=("track", "label")):
    """Start the programs (both at first; one again from restart): models first, then each server in its own
    thread. STATUS[key] gets {"url"} or {"error"}."""
    def fail(key, name, e):
        traceback.print_exc()
        why = str(e) if isinstance(e, SystemExit) else f"{type(e).__name__}: {e}"   # SystemExit: our own message
        STATUS[key] = {"error": f"{name} couldn't start.\n\n{why}\n\nDetails are in {HOME / 'app.log'}"}

    def get_models():
        """First start of a downloaded FaceRecognizer.exe: fetch the models (fetch_models.py)."""
        import fetch_models
        if not fetch_models.needed():
            return True
        print(f"Downloading models into {fetch_models.MODELS}: {', '.join(fetch_models.needed())}")

        def show(done, total, _):
            STATUS["fetch"] = {"done": done, "total": total}
        try:
            fetch_models.fetch(show)
            STATUS.pop("fetch", None)
            return True
        except OSError as e:
            for key in keys:
                fail(key, NAMES[key], OSError(f"The face models couldn't be downloaded: {e}\n"
                                              f"Check the internet connection, then click Start again."))
            return False

    def one(key, name, module, port, app):
        try:
            STATUS[key] = {"url": start(name, module, port, app)}
        except BaseException as e:               # incl. SystemExit (e.g. face_id.NotDownloaded) from loading
            fail(key, name, e)

    def go():
        if not get_models():
            return
        try:
            import label_tool
            import tracker                          # imports face_id, which checks the model and data files
        except BaseException as e:
            for key in keys:
                fail(key, NAMES[key], e)
            return
        jobs = [("track", "Face tracker", tracker, TRACK_PORT, "tracker"),
                ("label", "Face Labels", label_tool, LABEL_PORT, "face-labels")]
        for job in (j for j in jobs if j[0] in keys):   # both load at once: tracker models and the label database
            threading.Thread(target=one, args=job, daemon=True).start()

    threading.Thread(target=go, daemon=True).start()


def in_browser(shell, open_it=True):
    """The tab page in the default web browser (--browser, or no window toolkit; open_it False: FACE_UI=none, the
    servers only): the servers keep running until this process is stopped (Ctrl+C, or closing its terminal)."""
    import webbrowser
    print(f"{TITLE} is running at {shell}{' (in your web browser)' if open_it else ''}. Stop it with Ctrl+C.")
    if open_it:
        webbrowser.open(shell)
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    os._exit(0)


PROBE_SECONDS = 25


def probe_window():
    """--probe-window: open a tiny window with a page in it and close it again; exit 0 if the page loaded. Run as a
    child process by window_works(): where the window toolkit can't draw (Qt's web view without working OpenGL),
    it aborts the whole process, which Python can't catch."""
    import webview
    loaded = []
    win = webview.create_window(TITLE, html="<p>Starting ...</p>", width=320, height=120)
    win.events.loaded += lambda: loaded.append(1)

    def close_soon():
        for _ in range(PROBE_SECONDS * 5):
            if loaded:
                break
            time.sleep(0.2)
        time.sleep(0.3)
        win.destroy()
    webview.start(close_soon)
    os._exit(0 if loaded else 3)


def window_works():
    """True if the app window can open and show a page here. Only Linux is checked (a child process tries it):
    Windows and macOS always have their system web view."""
    if not sys.platform.startswith("linux"):
        return True
    import subprocess
    cmd = [sys.executable, "--probe-window"] if getattr(sys, "frozen", False) else         [sys.executable, str(Path(__file__).resolve()), "--probe-window"]
    try:
        r = subprocess.run(cmd, timeout=PROBE_SECONDS + 10, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except (OSError, subprocess.TimeoutExpired) as e:
        print(f"The app window didn't start ({e}).")
        return False
    if r.returncode != 0:
        tail = r.stderr.decode("utf-8", "replace").strip().splitlines()[-3:]
        print(f"The app window can't draw here (exit {r.returncode}): " + " | ".join(tail))
    return r.returncode == 0


# The window's compiled graphics and script caches. A newer built-in browser can freeze on ones an older version wrote
# and show only a white window (seen on Linux after updating from a 2026-11-09 build to 1.14: the page never drew until
# these were set aside). Each version starts with fresh ones; they're only caches, rebuilt on their own. What the pages
# remember (Local Storage, Session Storage, cookies) is never touched.
GRAPHICS_CACHES = {"GPUCache", "GPUPersistentCache", "GrShaderCache", "ShaderCache", "DawnGraphiteCache",
                   "DawnWebGPUCache", "Code Cache"}
CACHE_VERSION_FILE = "app-version.txt"


def clear_graphics_caches(folder, version=None):
    """When the app's version differs from the one that last used `folder` (the window's storage), remove the
    GRAPHICS_CACHES folders in it (up to 3 levels down: WebView2 keeps them under EBWebView\\ and EBWebView\\Default\\),
    then note this version. Returns the folders removed. Never raises: a cache left in place is only a cache."""
    version = version or VERSION
    folder = Path(folder)
    mark = folder / CACHE_VERSION_FILE
    try:
        if mark.read_text("utf-8").strip() == version:
            return []
    except OSError:
        pass
    removed = []
    if folder.is_dir():
        level = [folder]
        for _ in range(3):
            below = []
            for d in level:
                try:
                    subs = [s for s in d.iterdir() if s.is_dir() and not s.is_symlink()]
                except OSError:
                    continue
                for s in subs:
                    if s.name in GRAPHICS_CACHES:
                        shutil.rmtree(s, ignore_errors=True)
                        removed.append(s)
                    else:
                        below.append(s)
            level = below
    try:
        folder.mkdir(parents=True, exist_ok=True)
        mark.write_text(version, "utf-8")
    except OSError:
        pass
    if removed:
        print(f"New version {version}: cleared the window's graphics caches ({len(removed)} folders).")
    return removed


# The window keeps tracking and streaming its camera when it's minimized, covered or not in front: Chromium (WebView2,
# Qt WebEngine) otherwise slows timers to once a second and stops drawing in background windows, and the tracker's
# names and boxes froze, on its own screen and in other devices' Cameras tab.
KEEP_RUNNING_FLAGS = ("--disable-background-timer-throttling --disable-renderer-backgrounding "
                      "--disable-backgrounding-occluded-windows "
                      "--disable-features=CalculateNativeWinOcclusion,IntensiveWakeUpThrottling,"
                      "MediaFoundationD3D11VideoCapture")
# MediaFoundationD3D11VideoCapture off (Windows): camera frames stay in ordinary memory. As textures on the graphics
# processor, every frame the page copies out (for recognition, for the stream) has to be read back from it, and in a
# desktop session nobody is connected to (the NAS) one read-back in ten took 165 ms: the page stalled and the stream
# reached 13 frames a second. Off, every step took 4 ms or less and it reached 20 (measured 2026-11-12).


def allow_more_open_files(want=8192):
    """Linux, macOS: let this process, and the window's helper processes that inherit it, have `want` files open
    (systemd starts programs with 1024). The window's page process once used up 1024 in ten minutes (one descriptor
    for every answer its scripts didn't read: fixed there, drop() in the page scripts) and froze. This is the margin
    for the next such thing: hours instead of minutes before the watchdog has to step in. Returns the limit."""
    try:
        import resource
        soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
        if hard != resource.RLIM_INFINITY:
            want = min(want, hard)
        if soft != resource.RLIM_INFINITY and soft < want:
            resource.setrlimit(resource.RLIMIT_NOFILE, (want, hard))
            soft = want
        return soft
    except Exception:                                     # Windows (no such limit), or not allowed: as it was
        return None


def keep_running_in_background(env=os.environ):
    """Add KEEP_RUNNING_FLAGS to the window's browser (before it starts), keeping any flags already set."""
    for var in ("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "QTWEBENGINE_CHROMIUM_FLAGS"):
        have = env.get(var, "")
        if "--disable-background-timer-throttling" not in have:
            env[var] = (have + " " + KEEP_RUNNING_FLAGS).strip()


class Stamped:
    """The log file, each line starting with the time in UTC (HH:MM:SS), so a freeze or a restart can be matched
    with what else happened then."""
    def __init__(self, f):
        self.f, self.fresh, self.lock = f, True, threading.Lock()

    def write(self, s):
        with self.lock:
            out = []
            for part in s.splitlines(keepends=True):
                if self.fresh and part.strip():
                    out.append(time.strftime("%H:%M:%S ", time.gmtime()))
                out.append(part)
                self.fresh = part.endswith("\n")
            return self.f.write("".join(out))

    def __getattr__(self, name):
        return getattr(self.f, name)


def self_test():
    """--self-test: load everything the app is made of and use each part once, with no window, camera, model or
    network. For trying a build where it was made (CI does, for all three systems): a library left out of the
    package, or one a signed macOS app's hardened runtime refuses to load, fails here instead of on someone's
    computer. One line per part in HOME/self-test.txt (and on the console, where there is one). True if all passed."""
    import importlib
    lines, good = [], True

    def part(what, use):
        nonlocal good
        try:
            lines.append(f"ok      {what}: {use()}")
        except BaseException as e:               # a library that can't load may raise anything
            good = False
            lines.append(f"FAILED  {what}: {type(e).__name__}: {str(e)[:300]}")

    def numbers():
        import numpy as np
        return f"numpy {np.__version__}, {float((np.ones((64, 64), np.float32) @ np.ones((64, 64), np.float32)).sum()):.0f}"

    def pictures():
        import cv2
        import numpy as np
        done, jpg = cv2.imencode(".jpg", np.zeros((32, 32, 3), np.uint8))
        return f"OpenCV {cv2.__version__}, a {len(jpg)}-byte JPEG, face detector class {hasattr(cv2, 'FaceDetectorYN') and done}"

    def models():
        import onnxruntime
        return f"ONNX Runtime {onnxruntime.__version__}: {', '.join(onnxruntime.get_available_providers())}"

    def landmarks():
        import mediapipe
        from mediapipe.tasks.python import vision
        return f"MediaPipe {mediapipe.__version__}, {vision.FaceLandmarker.__name__}"

    def secrets_():
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        box = AESGCM(bytes(32))
        return "AES-GCM round trip " + str(box.decrypt(bytes(12), box.encrypt(bytes(12), b"text", None), None) == b"text")

    def callback():                              # a function made at run time: what a hardened runtime may forbid
        import ctypes
        return f"ctypes callback gives {ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int)(lambda n: n + 1)(41)}"

    def sound():                                 # the music log's listening: optional, so its absence isn't a failure
        import music
        try:
            import soundcard
        except Exception as e:                   # e.g. Linux without PulseAudio's library: song names only
            return f"no sound library here ({type(e).__name__}): the music log keeps song names only"
        found = music.interfaces()
        return (f"soundcard {getattr(soundcard, '__version__', '')}: {sum(i['kind'] == 'output' for i in found)} outputs, "
                f"{sum(i['kind'] == 'input' for i in found)} inputs can be listened to")

    def own():
        names = ("face_id", "labels", "label_tool", "tracker", "facedb", "dbsync", "dbsync_home", "peer", "secretbox",
                 "describe", "websearch", "hardware", "lmstudio", "persondata", "exprlog", "camrelay", "cudasupport",
                 "music", "musiclog", "spotify", "describejob", "label_settings", "forgotten", "waitlog")
        for n in names:
            importlib.import_module(n)
        return f"{len(names)} modules"

    def packed():                                # every file the three servers send, and the database's steps
        import facedb
        import label_tool
        import tracker
        files = {f for f, _ in PAGES.values()} | set(tracker.SCRIPTS.values()) | {
            tracker.PAGE, tracker.CAMERAS_PAGE, label_tool.PAGE, HERE / "label_ui.js", localguard.ICON, WINDOW_ICON}
        steps = len(list(facedb.SQL_DIR.glob("*.sql")))
        missing = sorted(f.name for f in files if not f.is_file()) + ([] if steps else ["db/*.sql"])
        if missing:
            raise FileNotFoundError(", ".join(missing))
        return f"{len(files)} page and icon files, {steps} database steps"

    def window():
        import webview
        if sys.platform == "win32":
            # the window's own parts, as it loads them at start: .NET through pythonnet (the lines in pywebview's
            # winforms.py that once failed on a new build's first starts: no window, and the web browser instead),
            # WebView2's library from the package, and the types the camera answer uses (allow_camera)
            forms = importlib.import_module("webview.platforms.winforms")
            importlib.import_module("webview.platforms.edgechromium")
            kind = importlib.import_module("Microsoft.Web.WebView2.Core").CoreWebView2PermissionKind
            if str(kind.Camera) != "Camera":
                raise RuntimeError(f"WebView2's permission kinds aren't as expected: {kind.Camera}")
            return (f"pywebview {getattr(webview, '__version__', '')}".strip() + ", its .NET and WebView2 parts load"
                    + ("" if forms.is_chromium else " (no WebView2 runtime on this computer)"))
        if sys.platform == "darwin":             # the window toolkit itself only where signing may stop it
            importlib.import_module("webview.platforms.cocoa")
            if not allow_camera_macos():         # the tracker's camera without a question at each page load
                raise RuntimeError("the window can't answer a page's camera request (see the lines above)")
            return f"pywebview {getattr(webview, '__version__', '')}, answers the tracker's camera request".strip()
        return f"pywebview {getattr(webview, '__version__', '')}".strip()

    for what, use in (("numbers", numbers), ("pictures", pictures), ("recognition models", models),
                      ("face landmarks", landmarks), ("clustering", lambda: importlib.import_module("sklearn.cluster").__name__),
                      ("database", lambda: f"psycopg {importlib.import_module('psycopg').__version__}, "
                                           f"{importlib.import_module('pgvector.psycopg').__name__}"),
                      ("encryption", secrets_), ("photo formats", lambda: f"Pillow {importlib.import_module('PIL').__version__}, "
                                                                           f"{importlib.import_module('pillow_heif').__name__}"),
                      ("certificates", lambda: importlib.import_module("truststore").__name__),
                      ("run-time functions", callback), ("the app's own code", own), ("pages", packed),
                      ("sound", sound),
                      ("window", window)):
        part(what, use)
    lines.append(f"{TITLE} {VERSION} on {sys.platform}: " + ("everything loads" if good else "SOMETHING FAILED"))
    text = "\n".join(lines) + "\n"
    try:
        HOME.mkdir(parents=True, exist_ok=True)
        (HOME / "self-test.txt").write_text(text, "utf-8")
    except OSError:
        pass
    try:
        print(text, end="", flush=True)
    except Exception:                            # a windowed build has no console
        pass
    return good


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if "--self-test" in args:
        os._exit(0 if self_test() else 1)        # at once: libraries loaded for the test may keep threads running
    if "--probe-window" in args:
        probe_window()
    wait_for_previous()                             # Windows, started again by restart_app: its ports free first
    HOME.mkdir(parents=True, exist_ok=True)
    if getattr(sys, "frozen", False) or sys.stdout is None:      # no console: keep the messages in a log
        log = Stamped(open(HOME / "app.log", "a", encoding="utf-8", buffering=1))
        sys.stdout = sys.stderr = log
        print(f"\n--- {paths.utc_now()} {TITLE} starting")
    import cudasupport                              # Linux app + NVIDIA: its downloaded CUDA, before onnxruntime loads
    cudasupport.activate()
    shell = shell_server()                          # first: the programs started next trust this page to frame them
    run_programs()                                  # (Settings offers to download CUDA when an NVIDIA card is here)
    import lmstudio                                 # LM Studio here: serve on all interfaces, announce it (every minute)
    lmstudio.keep()
    if os.environ.get("FACE_UI") == "none":           # test copies: nothing opens on the desktop
        in_browser(shell, open_it=False)
    if "--browser" in args or os.environ.get("FACE_UI") == "browser":
        in_browser(shell)
    if not window_works():                          # e.g. Linux without working OpenGL for Qt's web view
        print("Using the web browser instead of the app window.")
        in_browser(shell)
    try:
        import webview
        allow_camera_macos()                        # before the window is made: its delegate class answers
        window = webview.create_window(TITLE, shell, width=1440, height=920, min_size=(900, 600),
                                       background_color="#0d1512")
        global WINDOW
        WINDOW = window
        window.events.before_show += lambda: allow_camera(window, early=True)   # before any page can ask
        window.events.loaded += lambda: allow_camera(window)
        window.events.loaded += lambda: recover_renderer(window)
        window.events.loaded += lambda: drop_request_hooks(window)
        # private_mode=False keeps each page's remembered options and per-person baselines between runs
        clear_graphics_caches(HOME / "webview")
        allow_more_open_files()
        keep_running_in_background()
        watch_window()                              # a page that stops answering is brought back
        # private_mode and storage_path: see above. The window's icon: Windows and macOS take the app's own (set
        # when it is built); Linux's window toolkits are given it here.
        start = {"private_mode": False, "storage_path": str(HOME / "webview")}
        if sys.platform.startswith("linux") and WINDOW_ICON.is_file():
            start["icon"] = str(WINDOW_ICON)
        webview.start(**start)
        # in the log, so an app that is gone can be told from one that crashed (2026-11-15: an app that had
        # ended with nothing logged, and no way to say whether its window had been closed)
        print("The app window was closed: the app ends.", flush=True)
    except Exception:                               # no window toolkit here (e.g. Linux without GTK or Qt)
        traceback.print_exc()
        print("No app window could open here: using the web browser instead.")
        WINDOW, WATCHING["on"] = None, False          # no window: nothing to watch or to start again for (a closed
        in_browser(shell)                             # browser tab isn't a frozen window)
    os._exit(0)                                     # window closed: stop both servers and background readers


if __name__ == "__main__":
    main()
