"""
app.py - Face Recognizer: the live tracker and the labeling tool in one window, as two tabs.

The two stay separate programs inside: each keeps its own web server (tracker.py on 8766,
label_tool.py on 8765) and page, exactly as when started from recognize.bat / label.bat, and this
window shows both pages, one per tab. The tracker keeps running while the Labeling tab is shown.
If either is already running from its .bat, that copy is used instead of starting a second one.

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

TITLE = "Face Recognizer"
VERSION = "1.19.9"                                    # this build; GitHub release tags are "v" + VERSION
REPO = "SingularityResearch/facial-recognition"
HOME = paths.HOME                                  # %APPDATA%\FaceLabels: private, outside the project

HERE = paths.HERE                                  # the page files (inside the .exe when built)


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
         "/app_ui.js": (HERE / "app_ui.js", "text/javascript; charset=utf-8")}


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


def lets_us_frame(url):
    """True if the page at url may be shown in this window: its frame-ancestors name this window's tab page
    (localguard.FRAME_PARENTS, set by shell_server). A program started on its own (its .bat) names only itself."""
    try:
        req = urllib.request.Request(url, headers={"X-Face-Tool": "1"})
        with urllib.request.urlopen(req, timeout=3) as r:
            csp = r.headers.get("Content-Security-Policy", "")
    except Exception:
        return False
    for rule in csp.split(";"):
        words = rule.split()
        if words[:1] == ["frame-ancestors"]:
            return any(o in words[1:] for o in localguard.FRAME_PARENTS)
    return False


def start(name, module, port, app):
    """Start one program's server in this process on the first free port from `port`, or use the copy
    already running there. Returns its URL once it answers (models loaded)."""
    for p in range(port, port + PORT_TRIES):
        url = f"http://127.0.0.1:{p}/"
        try:
            srv = module.bind(p) if hasattr(module, "bind") else bind(p)
        except OSError:
            if answering(url, app):
                if not lets_us_frame(url):
                    # Started on its own, so it trusts only its own page. Starting a second copy instead would
                    # have two programs writing the same data files, so ask for that copy to be closed.
                    raise OSError(f"It is already running at {url}, started on its own (its .bat) or by another Face "
                                  f"Recognizer window, and a copy started that way can't be shown in this window. "
                                  f"\nClose that copy (its black "
                                  f"window) and start the app again, or keep using it in your browser at {url}") from None
                print(f"{name}: using the copy already running at {url}")
                return url
            continue
        print(f"{name}: loading ...")
        try:
            module.attach(srv)
        except BaseException:
            srv.server_close()                       # free the port for a later Start again
            raise
        threading.Thread(target=srv.serve_forever, daemon=True, name=name).start()
        print(f"{name}: serving at {url}")
        return url
    raise OSError(f"No free port between {port} and {port + PORT_TRIES - 1}")


def bind(port):
    return localguard.bind(port)                     # no address reuse on Windows: a second copy gets OSError


STATUS = {"track": {}, "label": {}}         # {"url": ...} once serving, {"error": ...} if it failed


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

        def _get(self):
            path = self.path.split("?", 1)[0]
            if self.refused(write=path in acts):
                return
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
    # the programs this window starts may be framed by this page (and only then: see localguard.FRAME_PARENTS)
    localguard.FRAME_PARENTS.update({f"http://127.0.0.1:{port}", f"http://localhost:{port}"})
    threading.Thread(target=srv.serve_forever, daemon=True, name="shell").start()
    return f"http://127.0.0.1:{srv.server_address[1]}/"


WINDOW = None                               # the app window, once made (main)

# ---- the window's page must never stay frozen ----------------------------------------------------------------------
# The tab page asks /alive every 2 s. Seen on the Linux laptop (Qt WebEngine): the page's renderer stopped running
# its scripts for hours while the process lived on, idle (no camera, no stream, nothing drawn). After HUNG_AFTER s
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
    """Start this app again in place (same process id on Linux and macOS): its ports are freed by the exec."""
    exe = sys.executable
    args = [exe] + ([] if getattr(sys, "frozen", False) else [str(Path(__file__).resolve())]) + sys.argv[1:]
    env = dict(os.environ, PYINSTALLER_RESET_ENVIRONMENT="1")       # a fresh start for a PyInstaller build
    try:
        sys.stdout.flush()
    except Exception:
        pass
    os.execve(exe, args, env)


RESTART_EVERY = 3600                        # restart_soon: at most once an hour (HOME/restarted.json)


def restart_soon(why):
    """Start the app again in a moment (the answer goes out first): only in the app window, and at most once every
    RESTART_EVERY s, so a camera that's really broken can't keep the app restarting. True when it will."""
    if WINDOW is None:
        return False
    f = HOME / "restarted.json"
    try:
        last = float(json.loads(f.read_text("utf-8")).get("at", 0))
    except (OSError, ValueError, AttributeError):
        last = 0.0
    if time.time() - last < RESTART_EVERY:
        print(f"Not starting the app again ({why}): it did so less than an hour ago.")
        return False
    try:
        paths.atomic_write(f, json.dumps({"at": time.time(), "why": why}).encode())
    except OSError:
        return False
    print(f"Starting the app again: {why}.")
    threading.Timer(1.0, restart_app).start()
    return True


def watch_window():
    """Watch the window's page (ALIVE) and bring it back when it stops (watch_step)."""
    import signal

    def run():
        st = {}
        while True:
            time.sleep(5)
            now = time.monotonic()
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
                print(f"The window's page hasn't answered for {now - ALIVE['at']:.0f} s: starting the app again.")
                try:
                    restart_app()
                except Exception:
                    traceback.print_exc()
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
    """True if the page at uri may use the camera and microphone without a prompt: only the tracker this window
    started or found (its address in STATUS), not any other program on 127.0.0.1."""
    track = STATUS["track"].get("url")
    if not track:
        return False
    u, t = urlparse(uri), urlparse(track)
    return (u.scheme, u.hostname, u.port) == (t.scheme, t.hostname, t.port)


def allow_camera(window):
    """Let this window's tracker page use the camera and microphone without asking each time (WebView2 asks by
    default). Any other page, including other programs on 127.0.0.1, still gets WebView2's prompt. Windows only: on
    macOS the system asks once for the app (the .app's NSCameraUsageDescription), and on Linux the window toolkit."""
    if sys.platform != "win32" or getattr(window, "_camera_ok", False):
        return
    window._camera_ok = True
    try:
        from System import Action
        from Microsoft.Web.WebView2.Core import CoreWebView2PermissionKind as Kind, CoreWebView2PermissionState as State

        def on_request(_, e):
            # the tracker's own alerts may show as Windows notifications too
            if e.PermissionKind in (Kind.Camera, Kind.Microphone, Kind.Notifications) and camera_allowed(e.Uri):
                e.State = State.Allow

        def hook():
            window.native.webview.CoreWebView2.PermissionRequested += on_request
        window.native.Invoke(Action(hook))
    except Exception:
        print("Camera permission: WebView2 will ask (couldn't pre-allow):")
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
                      "--disable-features=CalculateNativeWinOcclusion,IntensiveWakeUpThrottling")


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


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if "--probe-window" in args:
        probe_window()
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
        window = webview.create_window(TITLE, shell, width=1440, height=920, min_size=(900, 600),
                                       background_color="#0d1512")
        global WINDOW
        WINDOW = window
        window.events.loaded += lambda: allow_camera(window)
        window.events.loaded += lambda: recover_renderer(window)
        # private_mode=False keeps each page's remembered options and per-person baselines between runs
        clear_graphics_caches(HOME / "webview")
        keep_running_in_background()
        watch_window()                              # a page that stops answering is brought back
        webview.start(private_mode=False, storage_path=str(HOME / "webview"))
    except Exception:                               # no window toolkit here (e.g. Linux without GTK or Qt)
        traceback.print_exc()
        print("No app window could open here: using the web browser instead.")
        in_browser(shell)
    os._exit(0)                                     # window closed: stop both servers and background readers


if __name__ == "__main__":
    main()
