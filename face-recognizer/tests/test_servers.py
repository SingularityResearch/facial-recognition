"""The three servers' answers as a browser sees them (the labeling tool, the tracker and the app's tab page, each on a
local test server with a made-up data folder and no models): the headers every answer carries (what a page may load,
who may frame it, no reading by other sites), requests from other websites refused, request sizes checked, the
tracker's routes (Collect faces, presence log, feeling reports, naming averaged prints), and that no page loads
anything from the internet."""
import base64
import http.client
import json
import re
import types
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import cv2
import numpy as np
import pytest

import app
import face_id as F
import labels as L
import label_tool
import localguard as G
import tracker
from conftest import make_faces

HERE = Path(__file__).resolve().parents[1]


def request(url, path, data=None, headers=None, method=None):
    """(status, headers, body) of one request, with the headers the tool's own page sends for a POST."""
    hdr = dict(headers or {})
    if data is not None:
        hdr.setdefault("X-Face-Tool", "1")
    req = urllib.request.Request(url + path, data=data, headers=hdr, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status, r.headers, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read()


def raw_post(url, path, length, body=b""):
    """A POST with any Content-Length (urllib won't send a wrong one): the status code."""
    u = urllib.parse.urlparse(url)
    c = http.client.HTTPConnection(u.hostname, u.port, timeout=10)
    c.putrequest("POST", path)
    c.putheader("X-Face-Tool", "1")
    c.putheader("Content-Length", length)
    c.endheaders()
    if body:
        c.send(body)
    try:
        return c.getresponse().status
    finally:
        c.close()


def csp(headers):
    """The Content-Security-Policy as {directive: [sources]}."""
    out = {}
    for rule in headers["Content-Security-Policy"].split(";"):
        words = rule.split()
        if words:
            out[words[0]] = words[1:]
    return out


@pytest.fixture
def label_url(data, serve):
    make_faces(data, 6)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    return serve(label_tool.make_handler(db, L.Importer(db)))


class FakeEngine:
    """What the tracker's routes need from Engine, without models: names every averaged print "Ann"."""

    def __init__(self, tmp):
        self.recorder = tracker.Recorder()
        self.recorder.DIR = tmp / "expr_eval"
        self.asked = []
        self.faces = types.SimpleNamespace(devices={}, device="cpu", expression_model=None, cues_available=False,
                                           pose_available=False, hands_available=False, warmup=lambda: {})
        self.voice = types.SimpleNamespace(device=None, model=None)
        self.warmup_ms = {}

    def who(self, prints, threshold):
        self.asked.append((len(prints), threshold))
        return [["Ann", 0.6] for _ in prints]

    def people(self):
        return ["Ann"]


@pytest.fixture
def track(tmp_path, monkeypatch, serve):
    """(tracker URL, engine): Collect faces, presence log and feeling reports go to tmp_path."""
    monkeypatch.setattr(F, "WEBCAM", tmp_path / "webcam")
    monkeypatch.setattr(tracker, "PRESENCE", tmp_path / "presence")
    monkeypatch.setattr(F, "ALERTS", tmp_path / "alerts.json")
    engine = FakeEngine(tmp_path)
    return serve(tracker.make_handler(engine)), engine


@pytest.fixture
def shell_url(serve):
    return serve(app.shell_handler())


# --------------------------------------------------------------------------- headers
def test_every_answer_says_what_the_page_may_load(label_url, track, shell_url):
    for url, path in ((label_url, "/"), (label_url, "/api/summary"), (label_url, "/thumb/0.jpg"),
                      (track[0], "/"), (track[0], "/tracker_ui.js"), (shell_url, "/"), (shell_url, "/status")):
        code, h, _ = request(url, path)
        assert code == 200, (url, path)
        p = csp(h)
        assert p["default-src"] == ["'self'"] and p["object-src"] == ["'none'"] and p["base-uri"] == ["'none'"]
        assert p["connect-src"] == ["'self'"] and "'unsafe-eval'" not in p["script-src"]
        assert "'unsafe-inline'" not in p["script-src"]           # the pages keep no code inline (pages.test.js)
        assert h["Cross-Origin-Resource-Policy"] == "same-origin" and h["X-Content-Type-Options"] == "nosniff"


def test_each_page_gets_only_what_it_uses(label_url, track, shell_url):
    lab, trk, shell = (csp(request(u, "/")[1]) for u in (label_url, track[0], shell_url))
    assert lab["script-src"] == ["'self'"] and lab["img-src"] == ["'self'", "blob:"] and "media-src" not in lab
    assert trk["script-src"] == ["'self'", "blob:"] and trk["media-src"] == ["'self'", "blob:"]   # mic worklet, Open video
    assert shell["img-src"] == ["'self'"] and shell["frame-ancestors"] == ["'none'"]
    assert shell["frame-src"] == [f"http://127.0.0.1:{p}" for p in app.TOOL_PORTS]
    assert "frame-src" not in lab and "frame-src" not in trk


def test_programs_trust_the_tab_page_only_when_the_app_started_them(label_url, monkeypatch):
    assert csp(request(label_url, "/")[1])["frame-ancestors"] == ["'self'"]       # started on its own
    monkeypatch.setattr(G, "FRAME_PARENTS", {"http://127.0.0.1:8764", "http://localhost:8764"})   # as app.py does
    assert csp(request(label_url, "/")[1])["frame-ancestors"] == ["'self'", "http://127.0.0.1:8764",
                                                                  "http://localhost:8764"]


@pytest.mark.parametrize("ancestors, ours", [("'self'", False), ("'self' http://127.0.0.1:8764", True),
                                             ("'self' http://127.0.0.1:9999", False), ("'none'", False)])
def test_the_app_shows_a_running_copy_only_if_it_may_frame_it(serve, monkeypatch, ancestors, ours):
    """A copy started on its own (its .bat) trusts only itself: the app says so instead of showing a refused frame."""
    class Copy(G.Handler):
        frame_ancestors = ancestors

        def _get(self):
            self.send({})
    monkeypatch.setattr(G, "FRAME_PARENTS", {"http://127.0.0.1:8764", "http://localhost:8764"})   # this window
    assert app.lets_us_frame(serve(Copy) + "/") is ours


def test_other_websites_get_nothing(label_url, track, shell_url):
    for url, path in ((label_url, "/thumb/0.jpg"), (label_url, "/api/summary"), (track[0], "/api/people"),
                      (shell_url, "/status")):
        assert request(url, path, headers={"Sec-Fetch-Site": "cross-site"})[0] == 403
        assert request(url, path, headers={"Sec-Fetch-Site": "same-site"})[0] == 200   # the tab page's frames
        assert request(url, path, headers={"Sec-Fetch-Site": "none"})[0] == 200        # typed, bookmark, .bat
    assert request(label_url, "/api/undo", b"{}", {"Sec-Fetch-Site": "cross-site"})[0] == 403


def test_no_page_loads_anything_from_the_internet():
    allowed = {"https://images.google.com/?hl=en&gl=us",          # opened in the browser when you click, not loaded
               "http://127.0.0.1:1234"}                            # LM Studio's address, in a hint
    for f in sorted(HERE.glob("*.html")) + sorted(HERE.glob("*.js")):
        found = set(re.findall(r"""https?://[^\s"'`)<>]+""", f.read_text("utf-8")))
        assert found <= allowed, (f.name, found - allowed)
    assert "fonts.googleapis.com" not in (HERE / "tracker_ui.html").read_text("utf-8")


def test_the_camera_is_only_for_the_tracker_this_window_shows(monkeypatch):
    monkeypatch.setitem(app.STATUS, "track", {})
    assert not app.camera_allowed("http://127.0.0.1:8766/")                  # not started yet
    monkeypatch.setitem(app.STATUS, "track", {"url": "http://127.0.0.1:8766/"})
    assert app.camera_allowed("http://127.0.0.1:8766/") and app.camera_allowed("http://127.0.0.1:8766/x?y")
    for other in ("http://127.0.0.1:8765/", "http://127.0.0.1:9000/", "http://localhost:8766/",
                  "https://127.0.0.1:8766/", "http://evil.example:8766/"):
        assert not app.camera_allowed(other), other


# --------------------------------------------------------------------------- request sizes
@pytest.mark.parametrize("length, code", [("-1", 400), ("abc", 400), ("3000000", 413)])
def test_label_tool_checks_the_request_size(label_url, length, code):
    assert raw_post(label_url, "/api/undo", length) == code


def test_label_tool_takes_an_empty_post(label_url):
    assert raw_post(label_url, "/api/undo", "0") == 200


@pytest.mark.parametrize("length, code", [("-1", 400), ("0", 400), ("30000000", 413)])
def test_tracker_checks_the_request_size(track, length, code):
    assert raw_post(track[0], "/api/recognize", length) == code


def test_label_tool_says_what_this_computer_has(label_url):
    status, _, body = request(label_url, "/api/hardware")
    got = json.loads(body)
    assert status == 200 and got["has"]["threads"] >= 1 and "import_workers" in got["uses"]


# --------------------------------------------------------------------------- the tracker's routes
def _frame():
    img = np.full((480, 640, 3), 200, np.uint8)
    return cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 95])[1].tobytes()


def test_collect_faces_saves_only_each_face(track):
    url, _ = track
    faces = [{"box": [100, 100, 80, 80], "name": "Ann", "sim": 0.61}, {"box": [400, 300, 60, 60], "name": "Unknown", "sim": 0.1}]
    others = [[200, 100, 80, 80]]                                  # someone else beside Ann: not to be saved
    q = urllib.parse.urlencode({"faces": json.dumps(faces), "others": json.dumps(others)})
    code, _, body = request(url, f"/api/capture?{q}", _frame(), {"Content-Type": "image/jpeg"})
    assert code == 200
    stems = json.loads(body)["saved"]
    assert len(stems) == 2
    for stem, f in zip(stems, faces):
        img = cv2.imread(str(F.WEBCAM / f"{stem}.jpg"))
        said = json.loads((F.WEBCAM / f"{stem}.json").read_text("utf-8"))
        assert said["faces"][0]["name"] == f["name"] and len(said["faces"]) == 1
        x, y, w, h = said["faces"][0]["box"]
        assert img.shape[0] < 480 and img.shape[1] < 640                         # never the whole frame
        assert w >= f["box"][2] - 1 and abs(w - h) < 1                           # small ones are enlarged
        assert 0 <= x and x + w <= img.shape[1] and y + h <= img.shape[0] and max(img.shape[:2]) >= tracker.CAPTURE_MIN
        assert abs(int(img[int(y + h / 2), int(x + w / 2)].mean()) - 200) < 8      # the face itself is kept
    ann = cv2.imread(str(F.WEBCAM / f"{stems[0]}.jpg"))
    m = 80 * tracker.CAPTURE_MARGIN                                   # Ann's picture starts at 100 - m
    assert ann.shape[1] == int(np.ceil(100 + 80 + m)) - int(100 - m)
    assert abs(int(ann[140 - int(100 - m), 210 - int(100 - m)].mean()) - 128) < 8   # the other face is greyed out


@pytest.mark.parametrize("faces, others", [("[]", "[]"), ("not json", "[]"), ('[{"box": [1, 2, 3]}]', "[]"),
                                           ('[{"box": [10, 10, 50, 50]}]', '[[1, 2]]'),
                                           ('[{"box": [10, 10, Infinity, 50]}]', "[]"), ('[{"box": [10, 10, 0, 50]}]', "[]"),
                                           (json.dumps([{"box": [1, 1, 9, 9]}] * 21), "[]")])
def test_collect_faces_refuses_bad_requests(track, faces, others):
    q = urllib.parse.urlencode({"faces": faces, "others": others})
    assert request(track[0], f"/api/capture?{q}", _frame())[0] == 400
    assert not F.WEBCAM.exists() or not list(F.WEBCAM.iterdir())


def test_presence_log_keeps_visits_and_refuses_bad_ones(track):
    url, _ = track
    start = 1_762_700_000_000
    ok = json.dumps({"name": "Ann", "start": start, "end": start + 60_000}).encode()
    assert request(url, "/api/presence", ok)[0] == 200
    for bad in ({"name": "Unknown", "start": start, "end": start + 1}, {"name": "Ann", "start": start, "end": start - 1},
                {"name": "", "start": start, "end": start}, {"name": "Ann", "start": 0, "end": 5},
                {"name": "Ann", "start": start, "end": start + 8 * 86_400_000}, ["not", "a", "visit"]):
        assert request(url, "/api/presence", json.dumps(bad).encode())[0] == 400, bad
    day = tracker.time.strftime("%Y-%m-%d", tracker.time.localtime(start / 1000))
    code, _, body = request(url, f"/api/presence?day={day}")
    assert code == 200 and json.loads(body)["visits"] == [{"name": "Ann", "start": start, "end": start + 60_000}]
    assert request(url, "/api/presence?day=../../x")[0] == 400


def test_feeling_reports_keep_only_what_belongs(track):
    url, engine = track
    good = {"valence": 5, "arousal": 3, "name": "Ann", "features": {"smile": 0.4, "pitch_sd": [1.0, 2.0], "gone": None},
            "extra": "dropped"}
    assert request(url, "/api/expr-eval/report", json.dumps(good).encode())[0] == 200
    lines = (engine.recorder.DIR / f"feel-{tracker.time.strftime('%Y%m%d', tracker.time.gmtime())}.jsonl").read_text("utf-8").splitlines()
    saved = json.loads(lines[0])
    assert saved["name"] == "Ann" and saved["valence"] == 5 and "extra" not in saved
    assert saved["features"] == {"smile": 0.4, "pitch_sd": [1.0, 2.0], "gone": None}
    for bad in ({**good, "valence": 8}, {**good, "arousal": True}, {**good, "name": ""}, {**good, "features": {"a b": 1}},
                {**good, "features": {"x": "text"}}, {**good, "features": {"x": [1] * 17}}, [1, 2]):
        assert request(url, "/api/expr-eval/report", json.dumps(bad).encode())[0] == 400, bad
    assert len((engine.recorder.DIR / f"feel-{tracker.time.strftime('%Y%m%d', tracker.time.gmtime())}.jsonl").read_text("utf-8").splitlines()) == 1


def test_who_names_averaged_prints(track):
    url, engine = track
    p = base64.b64encode(np.ones(F.REC["dim"], "<f4").tobytes()).decode()
    code, _, body = request(url, "/api/who", json.dumps({"prints": [p, p], "t": 5}).encode())
    assert code == 200 and json.loads(body)["who"] == [["Ann", 0.6], ["Ann", 0.6]]
    assert engine.asked[-1] == (2, 0.9)                          # strictness kept within 0.1-0.9
    short = base64.b64encode(np.ones(10, "<f4").tobytes()).decode()
    for bad in ({"prints": [short]}, {"prints": ["not base64!"]}, {"prints": [p], "t": "x"}):
        assert request(url, "/api/who", json.dumps(bad).encode())[0] == 400, bad


def test_alert_inbox_keeps_alerts_until_acknowledged_archived_or_deleted(track):
    url, _ = track
    post = lambda path, body: request(url, path, json.dumps(body).encode())
    now = int(__import__("time").time() * 1000)
    code, _, body = post("/api/alerts", {"kind": "arrived", "name": "Ann", "text": "Ann arrived · 09:12", "t": now})
    assert code == 200 and json.loads(body)["counts"] == {"new": 1, "ack": 0, "archived": 0}
    post("/api/alerts", {"kind": "unknown", "name": "Bob", "text": "An unknown face stayed", "t": now + 1})
    listing = json.loads(request(url, "/api/alerts")[2])
    assert [a["kind"] for a in listing["alerts"]] == ["unknown", "arrived"]          # newest first
    assert listing["alerts"][0]["name"] == ""                                        # an unknown face has no name
    ann = listing["alerts"][1]["id"]
    assert json.loads(post("/api/alerts/state", {"ids": [ann], "state": "ack"})[2])["counts"] == {"new": 1, "ack": 1, "archived": 0}
    assert json.loads(post("/api/alerts/state", {"where": "new", "state": "ack"})[2])["counts"]["ack"] == 2
    assert json.loads(post("/api/alerts/state", {"where": "ack", "state": "archived"})[2])["counts"]["archived"] == 2
    assert json.loads(post("/api/alerts/state", {"where": "archived", "state": "delete"})[2])["counts"] ==         {"new": 0, "ack": 0, "archived": 0}
    for bad in ({"kind": "party", "text": "x"}, {"kind": "arrived", "name": "", "text": "x"},
                {"kind": "arrived", "name": "Ann", "text": ""}, {"kind": "arrived", "name": "Ann", "text": "x", "t": 1}):
        assert post("/api/alerts", bad)[0] == 400, bad
    assert post("/api/alerts/state", {"state": "ack"})[0] == 400                      # which ones?
    assert post("/api/alerts/state", {"ids": "all", "state": "ack"})[0] == 400
    assert post("/api/alerts/state", {"ids": [], "state": "gone"})[0] == 400
    assert request(url, "/api/alerts", b"{}", headers={"X-Face-Tool": ""})[0] == 403  # only the tool's own page


def test_alert_inbox_keeps_the_newest(tmp_path, monkeypatch):
    import alerts
    monkeypatch.setattr(F, "ALERTS", tmp_path / "alerts.json")
    monkeypatch.setattr(alerts, "KEEP", 5)
    now = int(__import__("time").time() * 1000)
    first = alerts.add({"kind": "arrived", "name": "Ann", "text": "kept: archived last", "t": now})
    alerts.set_state([first["id"]], "archived")
    for k in range(5):
        alerts.add({"kind": "arrived", "name": "Ann", "text": f"a{k}", "t": now + 1 + k})
    rows = alerts.listing()["alerts"]
    assert len(rows) == 5 and first["id"] not in {r["id"] for r in rows}            # the archived one went first


def test_the_window_says_how_many_alerts_are_new(shell_url, monkeypatch):
    seen = []
    monkeypatch.setattr(app, "attention", lambda unread, flash: seen.append((unread, flash)))
    assert request(shell_url, "/attention?unread=3&flash=1", headers={"X-Face-Tool": "1"})[0] == 200
    assert seen == [(3, True)]
    assert request(shell_url, "/attention?unread=3&flash=1")[0] == 403                # only the tab page itself


def test_an_alert_keeps_what_was_measured_numbers_only(track):
    url, _ = track
    post = lambda path, body: request(url, path, json.dumps(body).encode())
    now = int(__import__("time").time() * 1000)
    first = {"recognition": {"similarity": 0.712345678, "setting": "alone"}, "expression": {"reading": "happy"},
             "vsNormal": {"pulse": {"now": 72, "normal": 68.5, "spread": 4.0, "z": 0.875}, "pitch": {"now": None}},
             "sensors": {"camera": True, "microphone": False}}
    code, _, body = post("/api/alerts", {"kind": "arrived", "name": "Ann", "text": "Ann arrived", "t": now, "details": first})
    assert code == 200
    aid = json.loads(body)["alert"]["id"]
    later = {"vsNormal": {"pulse": {"now": 75, "z": 1.6}}}
    assert json.loads(post("/api/alerts/details", {"id": aid, "which": "later", "details": later})[2]) == {"saved": True}
    got = json.loads(request(url, "/api/alerts")[2])["alerts"][0]["details"]
    assert got["first"]["recognition"]["similarity"] == 0.7123 and got["first"]["vsNormal"]["pitch"] == {"now": None}
    assert got["later"] == later
    assert json.loads(post("/api/alerts/details", {"id": "nope", "which": "later", "details": later})[2]) == {"saved": False}
    for bad in ({"pic": "x" * 600},                        # long text (a picture as base64, say)
                {"a": {"b": {"c": {"d": {"e": 1}}}}},      # too deep
                {"list": [1, 2, 3]},                       # only objects of numbers and short text
                {"big": {f"k{i}": 1.5 for i in range(3000)}}):
        assert post("/api/alerts", {"kind": "arrived", "name": "Ann", "text": "x", "t": now, "details": bad})[0] in (400, 413), bad
    assert post("/api/alerts/details", {"id": aid, "which": "first", "details": later})[0] == 400   # first is fixed


def test_every_event_type_can_be_an_alert(track):
    url, _ = track
    now = int(__import__("time").time() * 1000)
    for kind in ("arrived", "left", "unusual"):
        body = {"kind": kind, "name": "Ann", "text": f"Ann {kind}", "t": now}
        assert request(url, "/api/alerts", json.dumps(body).encode())[0] == 200, kind
        body["name"] = ""                                   # these are about a named person
        assert request(url, "/api/alerts", json.dumps(body).encode())[0] == 400, kind
    kinds = sorted(a["kind"] for a in json.loads(request(url, "/api/alerts")[2])["alerts"])
    assert kinds == ["arrived", "left", "unusual"]


def test_a_second_copy_can_use_other_ports():
    assert app.ports({}) == (8764, 8765, 8766)
    assert app.ports({"FACE_PORTS": "9764"}) == (9764, 9765, 9766)     # a test build beside the usual copy
    assert app.ports({"FACE_PORTS": "nonsense"}) == (8764, 8765, 8766)
    with pytest.raises(SystemExit):
        app.ports({"FACE_PORTS": "80"})


# --------------------------------------------------------------------------- Start again (app.py /restart)
def test_start_again_restarts_what_failed_and_needs_the_pages_header(shell_url, monkeypatch):
    started = []
    monkeypatch.setattr(app, "run_programs", lambda keys=("track", "label"): started.append(keys))
    monkeypatch.setitem(app.STATUS, "track", {"error": "Face tracker couldn't start. (models)"})
    monkeypatch.setitem(app.STATUS, "label", {"error": "Face Labels couldn't start. (models)"})
    assert request(shell_url, "/restart?what=track")[0] == 403                     # not from the window's page
    code, _, body = request(shell_url, "/restart?what=track", headers={"X-Face-Tool": "1"})
    assert code == 200 and json.loads(body) == {"what": "track", "now": "starting"}
    assert started == [("track", "label")]                                          # a failed download: both again
    assert app.STATUS["track"] == {} and app.STATUS["label"] == {}
    assert json.loads(request(shell_url, "/restart?what=track", headers={"X-Face-Tool": "1"})[2])["now"] == "busy"
    assert request(shell_url, "/restart?what=other", headers={"X-Face-Tool": "1"})[0] == 400


def test_start_again_leaves_a_program_that_answers(monkeypatch):
    started = []
    monkeypatch.setattr(app, "run_programs", lambda keys=("track", "label"): started.append(keys))
    monkeypatch.setitem(app.STATUS, "label", {"url": "http://127.0.0.1:8765/"})
    monkeypatch.setitem(app.STATUS, "track", {"url": "http://127.0.0.1:8766/"})
    monkeypatch.setattr(app, "answering", lambda url, which: url.endswith("8765/"))
    assert app.restart("label") == "running" and not started
    assert app.restart("track") == "starting" and started == [("track",)]          # stopped answering: started again


def test_page_settings_and_person_signals_are_kept_by_the_server(track, tmp_path, monkeypatch):
    import devicesettings
    import paths
    monkeypatch.setattr(paths, "HOME", tmp_path / "h")
    url, _ = track
    post = lambda path, body: request(url, path, json.dumps(body).encode())[0]       # noqa: E731
    assert post("/api/settings", {"app": "track", "settings": {"ft-o-optMesh": "1", "ft-camera": "abc"}}) == 200
    state = json.loads(request(url, "/api/settings?app=track")[2])
    assert state["settings"] == {"ft-o-optMesh": "1", "ft-camera": "abc"} and state["forgotten"] == []
    for bad in ({"app": "label", "settings": {"fl-tile": "88"}},           # the tracker keeps only its own page's
                {"app": "track", "settings": {"fl-tile": "88"}}, {"app": "track", "settings": {"ft-usual": "{}"}},
                {"app": "track", "settings": {"ft-x": "y" * 30_000}}, {"app": "track", "settings": {"ft-x": 5}},
                {"app": "track", "settings": ["ft-x"]}):
        assert post("/api/settings", bad) == 400, bad
    assert post("/api/settings", {"app": "track", "settings": {"ft-camera": None}}) == 200      # None removes
    assert devicesettings.settings("track") == {"ft-o-optMesh": "1"}
    assert post("/api/signals", {"key": "ft-baselines", "people": {"Ann": {"all": {"blink": {"n": 3, "m": 1, "v": 0}}}}}) == 200
    assert post("/api/signals", {"key": "ft-other", "people": {}}) == 400
    state = json.loads(request(url, "/api/settings?app=track")[2])
    assert state["signals"]["ft-baselines"]["Ann"]["all"]["blink"]["n"] == 3 and state["signals"]["ft-usual"] == {}
    assert devicesettings.of("Ann") == {"baseline": {"all": {"blink": {"n": 3, "m": 1, "v": 0}}}}
    assert devicesettings.forget("Ann") == 1 and devicesettings.of("Ann") == {}
