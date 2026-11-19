"""The tracker in a process of its own: what it and the rest of the app ask of each other.

Since 1.25 the app starts the live tracker (tracker.py: the camera's frames in, recognition, the camera's stream out)
as a second process. Before, both programs shared one Python process, where only one thread runs at a time: with the
labeling tool kept busy, the tracker answered a tiny question in 83 ms instead of 3.5 (measured on a copy of a real
library, 2026-11-19), and a call that never came back stopped the camera's server with everything else.

What used to be a look into the other program's memory is now a question over 127.0.0.1, to the other one's own
server, under /api/local/: never a page's request. Each carries the secret the app made for this run (LINK in the
environment of the process it starts; nothing else has it) and X-Face-Tool, like every request of the programs.

    the app, the sync (label side)  ->  the tracker:   hello (is it the process this app started?), who is in
                                                       view, where its models run, the peer service on or off
    the tracker                      ->  the label side: a photo for another device (peer.py asks the tracker's
                                                       process, the photos are the labeling tool's), "something was
                                                       saved" (the sync sends it at once instead of a minute later)

With the tracker in the app's own process (FACE_TRACKER=inline, the tests, recognize.bat) REMOTE is False and the
callers look into memory as before."""
import hashlib
import hmac
import json
import os
import secrets
import threading
import time
import urllib.error
import urllib.request

REMOTE = False                  # the tracker has a process of its own (set by app.py before it starts one)
TRACKER = None                  # its address ("http://127.0.0.1:8766/") while it runs; None while it is down
LABEL = None                    # in the tracker's process: the labeling tool's address, once that side has called
TOKEN = os.environ.get("FACE_LINK") or ""       # this run's secret (the app makes it, its child inherits it)
TIMEOUT = 4
_OPEN = urllib.request.build_opener(urllib.request.ProxyHandler({}))       # 127.0.0.1 only: never through a proxy


def new_token():
    """Make this run's secret (the app, once, before it starts the tracker's process)."""
    global TOKEN
    TOKEN = secrets.token_hex(32)
    os.environ["FACE_LINK"] = TOKEN         # inherited by the process the app starts; no page, file or log has it
    return TOKEN


def allowed(headers):
    """Is this request one of the app's own two processes'? (the receiving server's check for /api/local/)"""
    got = headers.get("X-Face-Link") or ""
    return bool(TOKEN) and hmac.compare_digest(got.encode(), TOKEN.encode())


def proof(nonce):
    """What only a process that has this run's secret can answer to `nonce` (hello)."""
    return hmac.new(TOKEN.encode(), str(nonce).encode(), hashlib.sha256).hexdigest()


def _ask(base, path, body=None, timeout=TIMEOUT, raw=False):
    """GET (or POST with a JSON body) base + path as one of the app's processes. The JSON answer; raw: its bytes.
    OSError (URLError) when the other process doesn't answer; a 4xx or 5xx answer raises too."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base.rstrip("/") + path, data=data,
                                 headers={"X-Face-Tool": "1", "X-Face-Link": TOKEN, "Content-Type": "application/json",
                                          "Origin": base.rstrip("/")})
    with _OPEN.open(req, timeout=timeout) as r:
        got = r.read()
    return got if raw else json.loads(got or b"{}")


def hello(url, timeout=TIMEOUT):
    """True when the server at url is the tracker this app started: it can answer what only a holder of this
    run's secret can. The secret itself is sent to nobody who hasn't shown that."""
    if not TOKEN:
        return False
    nonce = secrets.token_hex(16)
    try:
        req = urllib.request.Request(url.rstrip("/") + f"/api/local/hello?n={nonce}", headers={"X-Face-Tool": "1"})
        with _OPEN.open(req, timeout=timeout) as r:
            got = json.loads(r.read() or b"{}")
        return hmac.compare_digest(str(got.get("proof") or ""), proof(nonce))
    except (OSError, ValueError):
        return False


# ---- the label side asks the tracker ---------------------------------------------------------------------------------
def in_view(within=6.0):
    """{name: when} of the people this device's camera sees now (dbsync_home's sightings)."""
    if not REMOTE:
        import camstream
        return camstream.in_view(within)
    if not TRACKER:
        return {}
    try:
        return dict(_ask(TRACKER, f"/api/local/inview?within={within}").get("names") or {})
    except (OSError, ValueError):
        return {}                              # the tracker is starting again: nobody in view for this step


def camera_services():
    """The camera's own part in the shared database (whether it streams, that the device is there): started where
    the camera's frames are."""
    if not REMOTE:
        import camstream
        camstream.start_heartbeat()
    elif TRACKER:
        try:
            _ask(TRACKER, "/api/local/inview")     # its process starts its own heartbeat when it is asked this
        except (OSError, ValueError):
            pass                                   # it is starting again: at the sync's next step


def peer_service(want, provider, key_source, held):
    """The peer service on or off, where the camera's frames are. held: the Service this process runs when the
    tracker is in this process (or None). Returns (running, held)."""
    import peer
    if not REMOTE:
        if want and held is None:
            held = peer.Service(provider, key_source).start()
        elif not want and held is not None:
            held.stop()
            held = None
        return held is not None, held
    if not TRACKER:
        return False, None
    try:
        got = _ask(TRACKER, "/api/local/peer", {"run": bool(want), "label": LABEL_URL.get("url")})
    except (OSError, ValueError):
        # Its process doesn't answer: it is being started again. Not shared for this step, and asked again at the
        # next. Never an error for the caller, who takes an OSError as "the port can't be used" and stops trying:
        # the first time the tracker's process was ended in a test, its camera never reached the other devices again.
        return False, None
    if got.get("error"):
        raise OSError(got["error"])            # the tracker's process couldn't take the port: that is the caller's
    return bool(got.get("running")), None


LABEL_URL = {}                  # in the label side's process: {"url": its own address} (label_tool.attach)


def devices():
    """({job: device}, {model file: decision}) of the tracker's models, for Settings → This computer."""
    if not REMOTE or not TRACKER:
        return {}, {}
    try:
        got = _ask(TRACKER, "/api/local/devices")
        return dict(got.get("devices") or {}), dict(got.get("decisions") or {})
    except (OSError, ValueError):
        return {}, {}


def cameras_port():
    """The port the tracker's process serves the Cameras page on besides its own, or None (it has none, or its
    process doesn't answer)."""
    if not REMOTE or not TRACKER:
        return None
    try:
        port = _ask(TRACKER, "/api/local/cameras").get("port")
        return port if isinstance(port, int) and 1024 <= port <= 65535 else None
    except (OSError, ValueError):
        return None


# ---- the tracker asks the label side ----------------------------------------------------------------------------------
def photo(photo_id, frame=None):
    """A photo of this device for another one (peer.Service's provider in the tracker's process): the labeling
    tool has the photos. JPEG bytes or None."""
    if not LABEL:
        return None
    try:
        return _ask(LABEL, f"/api/local/photo/{photo_id}" + (f"?frame={int(frame)}" if frame is not None else ""),
                    timeout=20, raw=True) or None
    except (OSError, ValueError):
        return None


_saved = threading.Event()


def note_saved():
    """Something the sync sends was saved in the tracker's process (an alert, a visit): tell the label side, so it
    goes out now and not at the sync's next look a minute later. Never waits (called under the saver's lock)."""
    _saved.set()


def _tell_saved():
    while True:
        _saved.wait()
        time.sleep(0.3)                        # a few saves in a row: one word for all of them
        _saved.clear()
        if LABEL:
            try:
                _ask(LABEL, "/api/local/saved", {})
            except (OSError, ValueError):
                pass                           # the sync finds it at its next look


def start_telling():
    threading.Thread(target=_tell_saved, daemon=True, name="tell the sync").start()
