"""persondata.py and the labeling tool's person routes, on a made-up data folder and private folder: everything held
about one person exported, then forgotten everywhere (their faces kept only as Ignored places); plus the labeling
tool's Describe all going on past an unreadable photo, and shared out among LM Studio servers."""
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
