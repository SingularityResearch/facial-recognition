"""This device's live camera, as the tracker page shows it (video with every marker drawn on it), for the other
devices to watch (peer.py's /stream), and who the camera sees now (for "moved between cameras" alerts,
dbsync_home.HomePart).

The tracker page sends a JPEG frame now and then: about every 2 s while nobody watches, ~10 a second while someone
does (put() answers how many watch). Frames are kept in memory only, never on disk: the newest one, for the watchers.
Shared by the tracker's server (which takes the page's frames) and the peer service (which serves them); both run in
the app window's one process."""
import threading
import time

WATCH_WINDOW = 5.0          # a watcher counts this long after its last frame
FRESH = 10.0                # a frame older than this means the camera isn't streaming
MAX_FRAME = 3_000_000       # bytes in one frame from the page

_cv = threading.Condition()
_frame = {"seq": 0, "jpeg": None, "at": 0.0, "size": None}
_watchers = {}              # watcher id -> last time it got a frame
_in_view = {}               # name -> last time the page said the camera sees them
_readout = {"at": 0.0, "faces": []}   # the tracker page's live readout as data (readoutData in tracker_ui.js)
MAX_READOUT = 200_000       # bytes of JSON


def put(jpeg, size=None):
    """A new frame from the tracker page. Returns how many watch now."""
    if not isinstance(jpeg, (bytes, bytearray)) or not jpeg.startswith(b"\xff\xd8") or len(jpeg) > MAX_FRAME:
        raise ValueError("expected a JPEG frame")
    with _cv:
        _frame.update(seq=_frame["seq"] + 1, jpeg=bytes(jpeg), at=time.time(), size=size)
        _cv.notify_all()
    return watching()


def latest():
    """(seq, jpeg) of the newest fresh frame, or (seq, None)."""
    with _cv:
        fresh = _frame["jpeg"] is not None and time.time() - _frame["at"] < FRESH
        return _frame["seq"], _frame["jpeg"] if fresh else None


def wait_next(after, watcher, timeout=5.0):
    """The next frame after seq `after` (waits up to timeout), as (seq, jpeg); jpeg None when none came. Notes the
    watcher, so the page sends frames quickly while anyone watches."""
    end = time.time() + timeout
    with _cv:
        _watchers[watcher] = time.time()
        while _frame["seq"] <= after or _frame["jpeg"] is None:
            left = end - time.time()
            if left <= 0:
                return after, None
            _cv.wait(left)
        _watchers[watcher] = time.time()
        return _frame["seq"], _frame["jpeg"]


def watching():
    now = time.time()
    with _cv:
        for k in [k for k, t in _watchers.items() if now - t > WATCH_WINDOW]:
            del _watchers[k]
        return len(_watchers)


def streaming():
    """True when the page sent a frame recently (the camera is on and the page streams)."""
    with _cv:
        return _frame["jpeg"] is not None and time.time() - _frame["at"] < FRESH


def set_in_view(names):
    """The names the camera sees now (the tracker page, every few seconds)."""
    now = time.time()
    with _cv:
        for n in names:
            if isinstance(n, str) and n and n != "Unknown" and not n.startswith("#") and len(n) <= 100:
                _in_view[n] = now


def set_readout(faces):
    """The live readout of the faces in view (from the tracker page), checked: a list of plain JSON values."""
    import json
    if not isinstance(faces, list) or len(faces) > 50 or len(json.dumps(faces)) > MAX_READOUT:
        raise ValueError("bad readout")
    with _cv:
        _readout.update(at=time.time(), faces=faces)


def readout():
    """{faces, age, frameAge}: the newest readout (no faces when it's older than FRESH seconds), and how old the
    newest camera frame is (the watching page shows "paused" when it's old)."""
    with _cv:
        age = time.time() - _readout["at"]
        frame_age = round(time.time() - _frame["at"], 1) if _frame["jpeg"] is not None else None
        return {"faces": _readout["faces"] if age < FRESH else [], "age": round(age, 1), "frameAge": frame_age}


def in_view(within=6.0):
    """{name: last seen} of the people the camera saw within the last `within` seconds."""
    now = time.time()
    with _cv:
        for k in [k for k, t in _in_view.items() if now - t > 3600]:
            del _in_view[k]
        return {k: t for k, t in _in_view.items() if now - t <= within}


def reset():
    """Forget everything (tests)."""
    with _cv:
        _frame.update(seq=0, jpeg=None, at=0.0, size=None)
        _readout.update(at=0.0, faces=[])
        _watchers.clear()
        _in_view.clear()
