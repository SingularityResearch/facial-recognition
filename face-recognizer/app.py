"""
app.py - Face Recognizer: the live tracker and the labeling tool in one window, as two tabs.

The two stay separate programs inside: each keeps its own web server (tracker.py on 8766,
label_tool.py on 8765) and page, exactly as when started from recognize.bat / label.bat, and this
window shows both pages, one per tab. The tracker keeps running while the Labeling tab is shown.
If either is already running from its .bat, that copy is used instead of starting a second one.

Run:  python app.py            (or FaceRecognizer.exe, built with build_app.bat)
Needs pywebview (pip install pywebview); the window is Edge WebView2, part of Windows 11.
"""
import http.server
import json
import os
import re
import sys
import threading
import time
import traceback
import urllib.request
from pathlib import Path
from urllib.parse import urlparse

TITLE = "Face Recognizer"
VERSION = "1.1"                                    # this build; GitHub release tags are "v" + VERSION
REPO = "SingularityResearch/facial-recognition"
HOME = Path(os.environ.get("APPDATA", Path.home())) / "FaceLabels"   # private: outside the published project

SHELL = """<!doctype html><html><head><meta charset="utf-8"><title>Face Recognizer</title><style>
:root { color-scheme: dark; }
html, body { margin: 0; height: 100%; background: #0d1512; color: #dfe8e4; font: 14px "Segoe UI", system-ui, sans-serif; }
body { display: flex; flex-direction: column; }
nav { display: flex; gap: 4px; padding: 6px 10px 0; background: #111c18; border-bottom: 1px solid #24342e; }
nav button { font: inherit; color: #9fb2ab; background: none; border: 1px solid transparent; border-bottom: none;
  border-radius: 8px 8px 0 0; padding: 8px 18px; cursor: pointer; }
nav button:hover { color: #dfe8e4; }
nav button[aria-selected="true"] { color: #dfe8e4; background: #0d1512; border-color: #24342e; margin-bottom: -1px; }
nav small { margin-left: 8px; color: #6f827b; }
nav .ver { margin-left: auto; align-self: center; padding-bottom: 6px; color: #6f827b; font-size: 12px; }
nav .ver button { font: inherit; color: #9fb2ab; background: none; border: 0; padding: 0 0 0 8px; cursor: pointer; text-decoration: underline; }
main { flex: 1; position: relative; }
iframe { position: absolute; inset: 0; width: 100%; height: 100%; border: 0; visibility: hidden; }
iframe.on { visibility: visible; }
.wait { position: absolute; inset: 0; display: grid; place-content: center; color: #9fb2ab; }
.wait.err { color: #f0a08f; white-space: pre-wrap; padding: 24px; }
</style></head><body>
<nav role="tablist">
  <button role="tab" id="tab-track" aria-selected="true" onclick="show('track')">Face tracking<small id="st-track">starting…</small></button>
  <button role="tab" id="tab-label" aria-selected="false" onclick="show('label')">Labeling<small id="st-label">starting…</small></button>
  <span class="ver"><span id="ver">vVERSION</span><button id="upd" onclick="checkUpdate()" title="Asks GitHub for the latest release (only when you click)">Check for updates</button></span>
</nav>
<main>
  <div class="wait" id="wait">Loading the face models…</div>
  <!-- visibility, not display:none, so the hidden tab keeps tracking and collecting faces -->
  <iframe id="f-track" title="Face tracking" allow="camera; microphone; autoplay; fullscreen"></iframe>
  <iframe id="f-label" title="Labeling" allow="clipboard-read; clipboard-write; fullscreen"></iframe>
</main>
<script>
let current = "track";
function show(k) {
  current = k;
  for (const t of ["track", "label"]) {
    document.getElementById("tab-" + t).setAttribute("aria-selected", t === k);
    document.getElementById("f-" + t).classList.toggle("on", t === k && !!document.getElementById("f-" + t).src);
  }
  const f = document.getElementById("f-" + k);
  document.getElementById("wait").hidden = !!f.src;
  if (f.src) f.focus();
  try { localStorage.setItem("tab", k); } catch {}
}
try { if (localStorage.getItem("tab") === "label") current = "label"; } catch {}
const errors = {};
async function poll() {                          // each program's URL once it's serving, or why it failed
  let st = {};
  try { st = await (await fetch("/status", { cache: "no-store" })).json(); } catch {}
  for (const k of ["track", "label"]) {
    const f = document.getElementById("f-" + k), s = st[k] || {};
    if (s.url && !f.src) { f.src = s.url; document.getElementById("st-" + k).textContent = ""; }
    if (s.error) { errors[k] = s.error; document.getElementById("st-" + k).textContent = "failed"; }
  }
  show(current);
  const w = document.getElementById("wait");
  if (errors[current]) { w.className = "wait err"; w.textContent = errors[current]; }
  else {
    w.className = "wait";
    w.textContent = st.fetch   // first start of a downloaded copy: the models come from the project's GitHub Release
      ? `Downloading the face models (first start only): ${Math.round(st.fetch.done / 1e6)} of ${Math.round(st.fetch.total / 1e6)} MB`
      : "Loading the face models…";
  }
  if (!["track", "label"].every(k => document.getElementById("f-" + k).src || errors[k])) setTimeout(poll, 400);
}
poll();
addEventListener("keydown", e => {               // Ctrl+1 / Ctrl+2 switch tabs
  if (e.ctrlKey && (e.key === "1" || e.key === "2")) { show(e.key === "1" ? "track" : "label"); e.preventDefault(); }
});
async function checkUpdate() {
  const b = document.getElementById("upd");
  b.textContent = "Checking…";
  try {
    const r = await (await fetch("/update", { cache: "no-store" })).json();
    if (r.error) b.textContent = "Couldn't check";
    else if (r.newer) { b.textContent = `${r.latest} is available: open the download page`; b.onclick = () => fetch("/open-release"); }
    else b.textContent = "Up to date";
  } catch { b.textContent = "Couldn't check"; }
}
</script></body></html>""".replace("VERSION", VERSION)


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
    """Start one program's server in this process on the first free port from `port`, or use the copy
    already running there. Returns its URL once it answers (models loaded)."""
    for p in range(port, port + 10):
        url = f"http://127.0.0.1:{p}/"
        try:
            srv = module.bind(p) if hasattr(module, "bind") else bind(p)
        except OSError:
            if answering(url, app):
                print(f"{name}: using the copy already running at {url}")
                return url
            continue
        print(f"{name}: loading ...")
        module.attach(srv)
        threading.Thread(target=srv.serve_forever, daemon=True, name=name).start()
        print(f"{name}: serving at {url}")
        return url
    raise OSError(f"No free port between {port} and {port + 9}")


def bind(port):
    http.server.ThreadingHTTPServer.allow_reuse_address = False      # see tracker.bind
    return http.server.ThreadingHTTPServer(("127.0.0.1", port), http.server.BaseHTTPRequestHandler)


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


def shell_server():
    """The two-tab page itself, on 127.0.0.1 (an http://127.0.0.1 page counts as secure, which the
    camera needs; so must the page around the tracker's frame). Serves only "/" and "/status"."""
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            path = self.path.split("?", 1)[0]
            if self.headers.get("Host", "").split(":")[0] not in ("127.0.0.1", "localhost") or path not in (
                    "/", "/status", "/update", "/open-release"):
                self.send_response(404)
                self.end_headers()
                return
            if path == "/open-release":                 # in the normal browser, not inside this window
                import webbrowser
                webbrowser.open(f"https://github.com/{REPO}/releases/latest")
                body = b"{}"
            elif path == "/update":
                body = json.dumps(latest_release()).encode()
            else:
                body = (SHELL if path == "/" else json.dumps(STATUS)).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8" if path == "/" else "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

    try:
        srv = bind(8764)                            # fixed, so the page's remembered tab survives restarts
    except OSError:
        srv = bind(0)                               # taken (a second copy?): any free port
    srv.RequestHandlerClass = Handler
    threading.Thread(target=srv.serve_forever, daemon=True, name="shell").start()
    return f"http://127.0.0.1:{srv.server_address[1]}/"


def allow_camera(window):
    """Let this window's own local pages use the camera and microphone without asking each time
    (WebView2 asks by default). Pages from anywhere else still get WebView2's prompt."""
    if getattr(window, "_camera_ok", False):
        return
    window._camera_ok = True
    try:
        from System import Action
        from Microsoft.Web.WebView2.Core import CoreWebView2PermissionKind as Kind, CoreWebView2PermissionState as State

        def on_request(_, e):
            if urlparse(e.Uri).hostname == "127.0.0.1" and e.PermissionKind in (Kind.Camera, Kind.Microphone):
                e.State = State.Allow

        def hook():
            window.native.webview.CoreWebView2.PermissionRequested += on_request
        window.native.Invoke(Action(hook))
    except Exception:
        print("Camera permission: WebView2 will ask (couldn't pre-allow):")
        traceback.print_exc()


def run_programs():
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
            for key, name in (("track", "Face tracker"), ("label", "Face Labels")):
                fail(key, name, OSError(f"The face models couldn't be downloaded: {e}\n"
                                        f"Check the internet connection and start the app again."))
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
            for key, name in (("track", "Face tracker"), ("label", "Face Labels")):
                fail(key, name, e)
            return
        jobs = [("track", "Face tracker", tracker, 8766, "tracker"),
                ("label", "Face Labels", label_tool, 8765, "face-labels")]
        for job in jobs:                            # both load at once: tracker models and the label database
            threading.Thread(target=one, args=job, daemon=True).start()

    threading.Thread(target=go, daemon=True).start()


def main():
    HOME.mkdir(parents=True, exist_ok=True)
    if getattr(sys, "frozen", False) or sys.stdout is None:      # no console: keep the messages in a log
        log = open(HOME / "app.log", "a", encoding="utf-8", buffering=1)
        sys.stdout = sys.stderr = log
        print(f"\n--- {time.strftime('%Y-%m-%d %H:%M:%S')} {TITLE} starting")
    import webview
    run_programs()
    window = webview.create_window(TITLE, shell_server(), width=1440, height=920, min_size=(900, 600),
                                   background_color="#0d1512")
    window.events.loaded += lambda: allow_camera(window)
    # private_mode=False keeps each page's remembered options and per-person baselines between runs
    webview.start(private_mode=False, storage_path=str(HOME / "webview"))
    os._exit(0)                                     # window closed: stop both servers and background readers


if __name__ == "__main__":
    main()
