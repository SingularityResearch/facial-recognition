"""persondata.py and the labeling tool's person routes, on a made-up data folder and private folder: everything held
about one person exported, then forgotten everywhere (their faces kept only as Ignored places); plus the labeling
tool's Describe all going on past an unreadable photo, and shared out among LM Studio servers, and Describe
everything again: photos, video frames and what only another device has."""
import json
import re
import threading
import time
import urllib.error
import urllib.request

import cv2
import numpy as np
import pytest

import alerts
import describe
import face_id as F
import labels as L
import label_tool
import paths
import persondata as P
from conftest import make_faces


def _jpeg(v):
    return cv2.imencode(".jpg", np.full((112, 112, 3), v, np.uint8))[1].tobytes()


@pytest.fixture
def world(data, home, tmp_path, monkeypatch):
    """A library with Ann (faces 0, 1), Bob (face 2, marked "not Ann") and Anna (face 3), and Ann in every other
    place a person can be. Returns the LabelDB."""
    monkeypatch.setattr(F, "WEBCAM", home / "webcam")
    monkeypatch.setattr(F, "PRESENCE", home / "presence")
    monkeypatch.setattr(F, "ALERTS", home / "alerts.json")
    monkeypatch.setattr(paths, "HOME", home)
    make_faces(data, 6, thumbs=[_jpeg(20 * i) for i in range(6)])
    F.Store().save()
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    db.act("name", [0, 1], "Ann")
    db.act("name", [2], "Bob")
    db.act("reject", [2], "Ann")
    db.act("name", [3], "Anna")
    (data / "backups").mkdir()
    (data / "backups" / "labels-20261101-000000.json").write_text(json.dumps(
        {"f0": {"n": "Ann", "by": "me"}, "f2": {"n": "Bob", "by": "me", "not": ["Ann"]}}))
    (data / "backups" / "legacy-20261029").mkdir()
    (data / "backups" / "legacy-20261029" / "names.json").write_text(json.dumps({"3": "Ann", "4": "Bob"}))
    F.save_json("web_suggestions.json", {"f0": {"suggestions": [], "when": "x"},
                                         "f5": {"suggestions": [{"name": "Ann"}], "when": "y"},
                                         "f2": {"suggestions": [{"name": "Bob"}], "when": "z"}})
    db.facts.facts["h0"] = {"t": None, "tags": ["Ann", "Bob"], "faces": {0: "Ann"}, "key": "k"}
    db.facts.save()
    ev = data / "expr_eval"
    ev.mkdir()
    (ev / "20261101-120000.jsonl").write_text(json.dumps({"name": "Ann", "posed": "happy"}) + "\n" +
                                              json.dumps({"name": "Bob", "posed": "sad"}) + "\n")
    (ev / "feel-20261101.jsonl").write_text(json.dumps({"name": "Ann", "valence": 5}) + "\n")
    F.save_json("va_fit.json", {"features": ["smile"], "models": {"Ann": {"w": [1]}, "*": {"w": [2]}}})
    F.save_json("webcam_eval.json", {"people": ["Ann", "Bob"], "faces": 3})
    (data / "sheets").mkdir()
    (data / "sheets" / "group_001.jpg").write_bytes(_jpeg(1))
    F.PRESENCE.mkdir()
    (F.PRESENCE / "2026-11-01.jsonl").write_text(json.dumps({"name": "Ann", "start": 1, "end": 2}) + "\n" +
                                                 json.dumps({"name": "Bob", "start": 3, "end": 4}) + "\n")
    F.WEBCAM.mkdir()
    for stem, who in (("webcam-1-0", "Ann"), ("webcam-2-0", "Bob")):
        (F.WEBCAM / f"{stem}.json").write_text(json.dumps({"when": "x", "faces": [{"box": [1, 2, 3, 4], "name": who}]}))
        (F.WEBCAM / f"{stem}.jpg").write_bytes(_jpeg(9))
    now = int(time.time() * 1000)
    alerts.add({"kind": "arrived", "name": "Ann", "text": "Ann arrived · 09:00", "t": now})
    alerts.add({"kind": "arrived", "name": "Bob", "text": "Bob arrived · 09:05", "t": now})
    describe.save("h0", {"text": "Ann and a dog on a beach"})
    describe.save("h1", {"text": "Anna's birthday cake"})
    describe.save("h2", {"text": "A sign reading ANN'S DINER"})
    (home / "app.log").write_text("starting\nKnown people: Ann, Bob\nok\n")
    return db


def _everything(data, home):
    """All text in the data and private folders, plus the names in people.npz."""
    out = []
    for root in (data, home):
        for f in root.rglob("*"):
            if f.is_file() and f.suffix in (".json", ".jsonl", ".log", ".txt"):
                out.append(f"{f.relative_to(root)}: {f.read_text('utf-8')}")
    with np.load(data / "people.npz") as z:
        out.append("people.npz: " + " ".join(map(str, z["name"])))
    return "\n".join(out)


def test_export_gathers_everything_about_one_person(world, data, home, tmp_path):
    r = P.export(world, "Ann", tmp_path / "out")
    assert r["faces"] == 2 and r["pictures"] == 2
    d = json.loads((tmp_path / r["folder"] / "data.json").read_text("utf-8"))
    assert d["person"] == "Ann" and [f["faceId"] for f in d["faces"]] == ["f0", "f1"]
    assert all(f["print"] and (tmp_path / r["folder"] / f["picture"]).exists() for f in d["faces"])
    assert np.frombuffer(__import__("base64").b64decode(d["faces"][0]["print"]), "<f4").shape == (F.REC["dim"],)
    assert [x["faceId"] for x in d["markedNotThem"]] == ["f2"]
    assert set(d["webSearch"]) == {"f0", "f5"} and d["photoTags"][0]["tags"] == ["Ann", "Bob"]
    assert d["expressionTests"] == {"20261101-120000.jsonl": [{"name": "Ann", "posed": "happy"}],
                                    "feel-20261101.jsonl": [{"name": "Ann", "valence": 5}]}
    assert d["presence"] == {"2026-11-01.jsonl": [{"name": "Ann", "start": 1, "end": 2}]}
    assert [a["text"] for a in d["alerts"]] == ["Ann arrived · 09:00"]
    assert set(d["descriptions"]) == {"h0", "h2"}                 # not "Anna's"
    assert d["fitted"]["va_fit.json"] == {"models": {"Ann": {"w": [1]}}} and d["fitted"]["webcam_eval.json"] == {"people": ["Ann"]}
    assert d["webcam"] == ["webcam-1-0.jpg"] and (tmp_path / r["folder"] / "webcam" / "webcam-1-0.json").exists()
    assert any("Named 2 faces Ann" == h["what"] for h in d["history"])
    assert "Bob" not in json.dumps(d["faces"]) and all(f["label"]["n"] == "Ann" for f in d["faces"])


def test_export_needs_a_known_person_and_a_full_folder_path(world, tmp_path):
    with pytest.raises(ValueError):
        P.export(world, "Zed", tmp_path)
    with pytest.raises(ValueError):
        P.export(world, "Ann", "relative\\folder")
    with pytest.raises(ValueError):
        P.export(world, "#ignore", tmp_path)


def test_faces_ignored_before_v19_lose_their_prints_with_one_copy_kept_until_someone_is_forgotten(data, home):
    make_faces(data, 3, thumbs=[_jpeg(20 * i) for i in range(3)])
    F.Store().save()
    F.save_json("labels.json", {"f1": {"n": L.IGNORE, "by": "me"}, "f0": {"n": "Ann", "by": "me"}})
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)      # the first start of v1.9
    assert db.printless(1) and not db.printless(0)
    copies = list((data / "backups").glob("before-v1.9-ignore-*"))
    assert len(copies) == 1 and (copies[0] / "faces.npz").exists() and (copies[0] / "thumbs.npz").exists()
    L.LabelDB(folders=[], backup=False, log=lambda *a: None)            # nothing left to convert: no second copy
    assert len(list((data / "backups").glob("before-v1.9-ignore-*"))) == 1
    P.forget(db, "Ann")                                                 # the copy holds Ann's prints too
    assert not list((data / "backups").glob("before-v1.9-ignore-*"))


def test_forget_removes_them_everywhere(world, data, home):
    before = _everything(data, home)
    assert re.search(r"\bAnn\b", before)
    done = P.forget(world, "Ann")
    after = _everything(data, home)
    assert not re.search(r"\bAnn\b", after, re.I), [s for s in after.splitlines() if re.search(r"\bAnn\b", s, re.I)]
    assert "Anna" in after and "Bob" in after                    # nobody else is touched
    assert done["faces"] == 2 and done["webcam"] == 1 and done["descriptions"] == 2 and done["sheets"] == 1
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)       # as the next start sees it
    assert db.name(0) == L.IGNORE and db.printless(0) and db.printless(1) and db.store.thumbs[0] == b""
    assert db.rec(2) == {"n": "Bob", "by": "me"} and db.name(3) == "Anna" and not db.printless(3)
    assert "h0" in db.store.photos and (F.WEBCAM / "webcam-2-0.jpg").exists() and not (F.WEBCAM / "webcam-1-0.jpg").exists()
    assert [a["name"] for a in alerts.listing()["alerts"]] == ["Bob"] and done["alerts"] == 1
    assert world.undo_stack == []                                 # Undo can't bring the name back
    assert describe.saved("h1") and not describe.saved("h0")
    assert (home / "app.log").read_text("utf-8") == "starting\nok\n"


def test_forget_keeps_the_in_memory_web_answers_in_step(world, data):
    web = F.load_json("web_suggestions.json", {})
    P.forget(world, "Ann", web, threading.Lock())
    assert set(web) == {"f2"} and F.load_json("web_suggestions.json", {}) == web


# --------------------------------------------------------------------------- the labeling tool's routes
def _post(url, path, body):
    req = urllib.request.Request(url + path, json.dumps(body).encode(), {"X-Face-Tool": "1"})
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


def test_forget_and_export_routes(world, serve, tmp_path):
    url = serve(label_tool.make_handler(world, L.Importer(world)))
    assert _post(url, "/api/person/forget", {"name": "Ann"})[0] == 400                      # not confirmed
    assert _post(url, "/api/person/forget", {"name": "Ann", "confirm": "ann"})[0] == 400
    assert world.name(0) == "Ann"
    code, r = _post(url, "/api/person/export", {"name": "Ann", "folder": str(tmp_path / "x")})
    assert code == 200 and r["faces"] == 2
    code, r = _post(url, "/api/person/forget", {"name": "Ann", "confirm": "Ann"})
    assert code == 200 and r["removed"]["faces"] == 2 and world.name(0) == L.IGNORE
    assert _post(url, "/api/person/forget", {"name": "Ann", "confirm": "Ann"})[0] == 400      # nobody left by that name


def test_describe_all_goes_on_past_an_unreadable_photo(data, home, serve, tmp_path, monkeypatch):
    """A photo that can't be read is noted (with its file) and the rest are described: the photos after the first
    four are loaded by later requests, which once lacked the photo's name and stopped the job."""
    make_faces(data, 16)
    photos = {}
    for k in range(8):
        f = tmp_path / f"p{k}.jpg"
        f.write_bytes(b"x")
        photos[f"h{k}"] = {"path": str(f)}
    (data / "photos.json").write_text(json.dumps(photos))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    imp = L.Importer(db)
    monkeypatch.setattr(describe, "model", lambda: "test-model")
    monkeypatch.setattr(describe, "ask", lambda jpg, name=None, url=None: {"text": "a photo", "model": "test-model"})
    monkeypatch.setattr(L, "online_only", lambda p: False)
    monkeypatch.setattr(F, "load_image", lambda p: None if p.endswith("p5.jpg") else np.zeros((20, 20, 3), np.uint8))
    url = serve(label_tool.make_handler(db, imp))
    assert _post(url, "/api/describe/all", {})[0] == 200
    imp.thread.join(10)
    st = imp.status
    assert st["stage"] == "Done" and st["done"] == 8 and st["new"] == 7 and st["errors"] == 1, st
    assert st["problems"] == [{"file": str(tmp_path / "p5.jpg"), "why": "OSError: unreadable"}]
    assert describe.count() == 7 and time.time() - st["started"] < 10


@pytest.mark.parametrize("helper_works", [True, False])
def test_describe_all_shares_photos_with_other_servers(data, home, serve, tmp_path, monkeypatch, helper_works):
    """With other LM Studio servers (describe.helpers), each takes photos as it finishes; one that fails twice is
    left out, its photos go back to the others, and every photo is still described once."""
    make_faces(data, 4)
    photos = {}
    for k in range(30):
        f = tmp_path / f"p{k}.jpg"
        f.write_bytes(b"x")
        photos[f"h{k}"] = {"path": str(f)}
    (data / "photos.json").write_text(json.dumps(photos))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    imp = L.Importer(db)
    asked = []

    def ask(jpg, name=None, url=None):
        asked.append(url)
        time.sleep(0.01)
        if url and not helper_works:
            raise describe.Unavailable("The describe server didn't answer")
        return {"text": "a photo", "model": name}
    monkeypatch.setattr(describe, "model", lambda: "test-model")
    monkeypatch.setattr(describe, "helpers", lambda name: [{"host": "other", "url": "http://192.168.1.30:1234"}])
    monkeypatch.setattr(describe, "ask", ask)
    monkeypatch.setattr(L, "online_only", lambda p: False)
    monkeypatch.setattr(F, "load_image", lambda p: np.zeros((20, 20, 3), np.uint8))
    url = serve(label_tool.make_handler(db, imp))
    assert _post(url, "/api/describe/all", {})[0] == 200
    imp.thread.join(10)
    st = imp.status
    assert st["stage"] == "Done" and st["done"] == 30 and st["new"] == 30 and st["errors"] == 0, st
    assert describe.count() == 30 and sum(st["by"].values()) == 30
    if helper_works:
        assert st["by"]["other"] > 0 and st["by"]["this computer"] > 0 and not st.get("left_out")
    else:
        assert st["by"]["other"] == 0 and asked.count("http://192.168.1.30:1234") == 2
        assert st["left_out"][0].startswith("other (failed twice")


def test_a_helper_left_out_is_tried_again_and_comes_back(data, home, serve, tmp_path, monkeypatch):
    """A helper that fails twice sits out, then gets one photo again: when its server works by then it rejoins (and
    is no longer listed as left out); it once stayed out for the whole run, 13 seconds into 17 hours."""
    make_faces(data, 4)
    photos = {}
    for k in range(60):
        f = tmp_path / f"p{k}.jpg"
        f.write_bytes(b"x")
        photos[f"h{k}"] = {"path": str(f)}
    (data / "photos.json").write_text(json.dumps(photos))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    imp = L.Importer(db)
    helper, noted = [], []

    def ask(jpg, name=None, url=None):
        time.sleep(0.02)
        noted.extend(imp.status.get("left_out") or [])           # what the progress bar says meanwhile
        if url:
            helper.append(time.monotonic())
            if len(helper) <= 3:                                 # down at first: twice, and once more after the wait
                raise describe.Unavailable("The describe server didn't answer")
        return {"text": "a photo", "model": name}
    monkeypatch.setattr(label_tool, "HELPER_RETRY", 0.15)
    monkeypatch.setattr(describe, "model", lambda: "test-model")
    monkeypatch.setattr(describe, "helpers", lambda name: [{"host": "other", "url": "http://192.168.1.30:1234"}])
    monkeypatch.setattr(describe, "ask", ask)
    monkeypatch.setattr(L, "online_only", lambda p: False)
    monkeypatch.setattr(F, "load_image", lambda p: np.zeros((20, 20, 3), np.uint8))
    url = serve(label_tool.make_handler(db, imp))
    assert _post(url, "/api/describe/all", {})[0] == 200
    imp.thread.join(20)
    st = imp.status
    assert st["stage"] == "Done" and st["done"] == 60 and st["new"] == 60 and st["errors"] == 0, st
    assert describe.count() == 60 and sum(st["by"].values()) == 60     # every photo once, whoever failed on it
    assert st["by"]["other"] > 0 and st["left_out"] == []              # it came back, and is no longer listed
    assert helper[2] - helper[1] >= 0.14 and helper[3] - helper[2] >= 0.14   # it sat out before each new try
    assert noted and all(n.startswith("other (failed twice") and "tried again every" in n for n in noted)


def _library(data, tmp_path, monkeypatch):
    """Four photos here (h0-h3; h3 a duplicate of h0), a video here with faces in frames 30 and 90, and two photos
    only another device has (h5, h6). Returns (LabelDB, Importer, what was asked to be described)."""
    make_faces(data, 4)
    photos = {}
    for k in range(4):
        f = tmp_path / f"p{k}.jpg"
        f.write_bytes(b"x")
        photos[f"h{k}"] = {"path": str(f)}
    photos["h3"]["dup_of"] = "h0"
    (tmp_path / "clip.mp4").write_bytes(b"x")
    photos["h4"] = {"path": str(tmp_path / "clip.mp4"), "video": True, "at": {"f0": 30, "f1": 30, "f2": 90}}
    photos["h5"] = {"name": "away.jpg"}
    photos["h6"] = {"name": "gone.jpg"}
    (data / "photos.json").write_text(json.dumps(photos))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    asked = []

    def ask(jpg, name=None, url=None):
        asked.append(len(jpg))
        return {"text": "described again", "model": name}
    monkeypatch.setattr(describe, "model", lambda: "new-model")
    monkeypatch.setattr(describe, "ask", ask)
    monkeypatch.setattr(L, "online_only", lambda p: False)
    monkeypatch.setattr(F, "load_image", lambda p: np.zeros((20, 20, 3), np.uint8))
    monkeypatch.setattr(F, "video_frame", lambda p, k: np.zeros((20 + k, 20, 3), np.uint8))
    return db, L.Importer(db), asked


def _status(url):
    with urllib.request.urlopen(url + "/api/describe/status", timeout=10) as r:
        return json.loads(r.read())


def test_describe_everything_again_covers_photos_video_frames_and_other_devices(data, home, serve, tmp_path, monkeypatch):
    """Every photo and every video frame a face was found in is described anew, old descriptions replaced; what
    only another device has is asked of it (peer.fetch); what no device sends stays to carry on with."""
    import types

    import peer
    db, imp, asked = _library(data, tmp_path, monkeypatch)
    db.sync = types.SimpleNamespace(info={"state": "online"})          # a shared database, reached
    sent = []

    def fetch(h, frame=None):
        sent.append((h, frame))
        return (_jpeg(7), "nas") if h == "h5" else (None, "no other device that shares its photos has it")
    monkeypatch.setattr(peer, "fetch", fetch)
    monkeypatch.setattr(describe, "status", lambda: {"ok": True, "model": "new-model"})
    old = {"text": "old", "model": "old-model", "when": "2026-01-01T00:00:00Z"}
    with describe._lock:                                               # as described long ago: h0, and frame 30
        describe._load().update({"h0": dict(old), "h4:30": dict(old), "h3": dict(old)})
    url = serve(label_tool.make_handler(db, imp))
    a = _status(url)["again"]
    assert a == {"total": 7, "frames": 2, "elsewhere": 2, "since": None, "model": None, "left": 0}
    assert _post(url, "/api/describe/again", {"carry_on": True})[0] == 400      # nothing unfinished to carry on
    assert _post(url, "/api/describe/again", {})[0] == 200
    imp.thread.join(10)
    st = imp.status
    assert st["stage"] == "Done" and st["again"] and st["total"] == 7 and st["new"] == 6 and st["known"] == 1, st
    assert sorted(sent) == [("h5", None), ("h6", None)] and len(asked) == 6
    for k in ("h0", "h1", "h2", "h4:30", "h4:90", "h5"):
        assert describe.saved(k)["text"] == "described again" and describe.saved(k)["model"] == "new-model", k
    assert describe.saved("h3")["text"] == "old" and not describe.saved("h6") and not describe.saved("h4")
    a = _status(url)["again"]                                          # h6 is left: the run stays, to carry on
    assert a["left"] == 1 and a["model"] == "new-model" and describe.again()["since"] == a["since"]
    monkeypatch.setattr(peer, "fetch", lambda h, frame=None: (_jpeg(9), "nas"))
    assert _post(url, "/api/describe/again", {"carry_on": True})[0] == 200
    imp.thread.join(10)
    assert imp.status["total"] == 1 and imp.status["new"] == 1 and describe.saved("h6")["text"] == "described again"
    assert describe.again() is None and _status(url)["again"]["left"] == 0     # all of it done: nothing unfinished


def test_describe_everything_again_stops_asking_devices_that_do_not_answer(data, home, serve, tmp_path, monkeypatch):
    """Devices that aren't reached PEER_MISSES times in a row are asked for no more photos in that run (each ask
    waits for every address); without a shared database nothing is asked at all. Describe all itself is unchanged:
    only photos here that aren't described yet."""
    import types

    import peer
    db, imp, asked = _library(data, tmp_path, monkeypatch)
    with db.lock:
        for k in range(30):
            db.store.photos[f"away{k}"] = {"name": f"away{k}.jpg"}
    sent = []

    def fetch(h, frame=None):
        sent.append(h)
        return None, f"it's on nas, which {peer.UNREACHED} (192.168.1.9:8767: timed out)"
    monkeypatch.setattr(peer, "fetch", fetch)
    url = serve(label_tool.make_handler(db, imp))
    assert _post(url, "/api/describe/again", {})[0] == 200             # no shared database: other devices not asked
    imp.thread.join(10)
    assert sent == [] and imp.status["new"] == 5 and imp.status["known"] == 32
    db.sync = types.SimpleNamespace(info={"state": "online"})
    monkeypatch.setattr(label_tool, "PEER_MISSES", 3)
    assert _post(url, "/api/describe/again", {})[0] == 200             # starts over: everything again
    imp.thread.join(10)
    st = imp.status
    assert st["stage"] == "Done" and st["total"] == 37 and st["new"] == 5 and st["known"] == 32, st
    assert 3 <= len(sent) <= 3 + 5 and len(st["left_out"]) == 1 and st["left_out"][0].startswith("what is only on other devices")
    assert describe.again()                                            # 32 left: kept, to carry on with
    describe.drop(lambda d: True)
    assert _post(url, "/api/describe/all", {})[0] == 200
    imp.thread.join(10)
    assert imp.status["total"] == 3 and not imp.status["again"] and describe.count() == 3    # h0-h2: no video, no dup


def test_a_persons_expression_readings_are_exported_and_forgotten(data, home, tmp_path, monkeypatch):
    """The expression log (exprlog.py): a person's waiting readings are in their export (a file of its own, with
    those in the shared database when it can be reached) and go when they are forgotten."""
    import time
    from pathlib import Path

    import exprlog
    import facedb
    monkeypatch.setattr(paths, "HOME", home)
    monkeypatch.setattr(F, "PRESENCE", home / "presence")
    monkeypatch.setattr(F, "ALERTS", home / "alerts.json")
    monkeypatch.setattr(F, "WEBCAM", home / "webcam")
    make_faces(data, 6)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    db.act("name", [0, 1], "Ann")
    db.act("name", [2], "Bo")
    monkeypatch.setattr(facedb, "configured", lambda: True)              # readings are kept only for a database
    now = int(time.time() * 1000)
    assert exprlog.add([{"t": now + k, "name": n, "label": "happy", "share": 0.7, "state": "usual", "data": {}}
                        for k, n in enumerate(["Ann", "Bo", "Ann"])]) == 3
    out = P.export(db, "Ann", tmp_path / "out")
    where = Path(out["folder"])
    lines = [json.loads(ln) for ln in (where / "expression_log.jsonl").read_text("utf-8").splitlines()]
    assert len(lines) == 2 and all(ln["label"] == "happy" and ln["waitingToBeSent"] for ln in lines)
    d = json.loads((where / "data.json").read_text("utf-8"))
    assert d["expressionLog"]["readings"] == 2 and "couldn't be read" in d["expressionLog"]["note"]   # no server here
    monkeypatch.setattr(facedb, "configured", lambda: False)
    assert P.forget(db, "Ann")["expressionLog"] == 2
    assert [r["name"] for r in exprlog.waiting()] == ["Bo"]


def test_a_persons_music_is_exported_and_forgotten(data, home, tmp_path, monkeypatch):
    """The music log (musiclog.py): the songs that played for a person are in their export (a file of its own),
    their usual music is among their person signals, and all of it goes when they are forgotten, with their name
    as this device's music setting."""
    import time
    from pathlib import Path

    import devicesettings
    import facedb
    import musiclog
    monkeypatch.setattr(paths, "HOME", home)
    monkeypatch.setattr(F, "PRESENCE", home / "presence")
    monkeypatch.setattr(F, "ALERTS", home / "alerts.json")
    monkeypatch.setattr(F, "WEBCAM", home / "webcam")
    make_faces(data, 6)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    db.act("name", [0, 1], "Ann")
    db.act("name", [2], "Bo")
    devicesettings.update("label", {musiclog.WHO: "Ann", musiclog.OPTION: "1"})
    musiclog.set_picture("Ann", {"songs": 3, "artists": 2, "top_artists": [{"name": "X", "songs": 2, "share": 0.67}]})
    now = int(time.time() * 1000)
    for k, n in enumerate(["Ann", "Bo", "Ann"]):
        assert musiclog.add({"t": now + k, "name": n, "artist": "X", "song": f"song {k}", "seconds": 200,
                             "data": {"app": "Spotify", "heard": {"tempo": 90, "mode": "minor"}}})
    monkeypatch.setattr(facedb, "configured", lambda: True)
    out = P.export(db, "Ann", tmp_path / "out")
    where = Path(out["folder"])
    lines = [json.loads(ln) for ln in (where / "music_log.jsonl").read_text("utf-8").splitlines()]
    assert [ln["song"] for ln in lines] == ["song 0", "song 2"] and all(ln["waitingToBeSent"] for ln in lines)
    d = json.loads((where / "data.json").read_text("utf-8"))
    assert d["musicLog"]["songs"] == 2 and "couldn't be read" in d["musicLog"]["note"]          # no server here
    assert json.dumps(d).count('"top_artists"') == 1                     # their usual music, among their signals
    monkeypatch.setattr(facedb, "configured", lambda: False)
    done = P.forget(db, "Ann")
    assert done["musicLog"] == 2 and done["musicWho"] == 1 and done["personSignals"] == 1
    assert [r["name"] for r in musiclog.waiting()] == ["Bo"] and musiclog.who() == ""
    assert musiclog.picture_of("Ann") is None


def test_someone_left_in_no_label_can_still_be_forgotten(data, home, monkeypatch):
    """After a rename, or once their last face was cleared, a person is in no label. What is still kept under the
    name (alerts, learned signals, the alert list) must stay exportable and forgettable."""
    import devicesettings
    monkeypatch.setattr(paths, "HOME", home)
    monkeypatch.setattr(F, "PRESENCE", home / "presence")
    monkeypatch.setattr(F, "ALERTS", home / "alerts.json")
    monkeypatch.setattr(F, "WEBCAM", home / "webcam")
    make_faces(data, 6)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    db.act("name", [0], "Annie")
    devicesettings.put_signal("usual", "Annie", {"p": [0.1] * 7, "n": 150})
    alerts.add({"kind": "arrived", "name": "Annie", "text": "Annie arrived"})
    db.rename("Annie", "Ann")
    with pytest.raises(ValueError):
        P.check_name(db, "Nobody")
    assert P.check_name(db, "Annie") == "Annie"
    done = P.forget(db, "Annie")
    assert done["personSignals"] == 1 and done["alerts"] == 1 and done["faces"] == 0
    assert db.name(0) == "Ann" and devicesettings.of("Annie") == {} and alerts.about("Annie") == []
    with pytest.raises(ValueError):
        P.check_name(db, "Annie")                            # nothing is kept under the name any more
