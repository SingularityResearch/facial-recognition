"""labels.py and face_id.Store on a tiny made-up data\\ folder: naming, undo, rename/merge, the new-user (empty)
start, and moving face pictures out of faces.npz."""
import json

import cv2
import numpy as np
import pytest

import face_id as F
import labels as L
from conftest import make_faces


def db_with_faces(data, n=6):
    make_faces(data, n)
    return L.LabelDB(folders=[], backup=False, log=lambda *a: None)


def test_a_new_copy_starts_empty(data):
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    assert db.n == 0 and db.labels == {} and db.people_faces() == {}


def test_without_models_programs_stop_with_a_message():
    with pytest.raises(SystemExit) as e:                    # the tests run with no models at all
        F.require_models()
    assert "fetch_models" in str(e.value) or "Git LFS" in str(e.value)


def test_name_reject_rename_and_undo(data):
    db = db_with_faces(data)
    db.act("name", [0, 1], "Ann")
    assert db.name(0) == "Ann" and db.rec(0)["by"] == "me"
    db.act("reject", [2], "Ann")
    assert db.rec(2)["not"] == ["Ann"]
    db.act("name", [2], "Bob")
    assert db.rename("Ann", "Bob") == 3                    # merges Ann into Bob: 2 named, 1 "not Ann"
    assert {db.name(i) for i in (0, 1, 2)} == {"Bob"} and "Ann" not in json.dumps(db.labels)
    assert db.undo() == "Merged Ann into Bob"
    assert db.name(0) == "Ann" and db.name(2) == "Bob"
    db.undo()
    db.undo()
    assert db.name(2) is None and db.rec(2) == {} and db.name(0) == "Ann"
    assert json.loads((data / "labels.json").read_text("utf-8")) == db.labels       # saved as it goes
    assert len((data / "history.jsonl").read_text("utf-8").splitlines()) == 7       # 4 changes + 3 undos


@pytest.mark.parametrize("action, idx, name", [
    ("name", [0], ""), ("name", [0], "#secret"), ("reject", [0], None), ("name", [99], "Ann"), ("name", [-1], "Ann"),
    ("explode", [0], "Ann"),
])
def test_bad_changes_are_refused(data, action, idx, name):
    db = db_with_faces(data)
    with pytest.raises(ValueError):
        db.act(action, idx, name)
    assert db.labels == {}


def test_ignore_and_clear(data):
    db = db_with_faces(data)
    db.act("ignore", [3], None)
    assert db.name(3) == L.IGNORE
    db.act("clear", [3], None)
    assert db.rec(3) == {}


def _npz(path):
    with np.load(path) as z:                     # closed again: Windows can't replace a file that's open
        return dict(z)


def test_ignore_deletes_the_print_and_picture_but_not_the_place(data):
    pics = [_jpeg(10 * i) for i in range(6)]
    make_faces(data, 6, thumbs=pics)
    F.Store().save()
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    emb2 = np.asarray(db.store.emb[2]).copy()
    db.act("ignore", [2, 3], None)
    z, s = _npz(data / "faces.npz"), F.Store()
    assert not z["emb"][2].any() and not z["emb"][3].any() and z["emb"][1].any()      # prints gone from disk
    assert s.thumbs[2] == b"" and s.thumbs[3] == b"" and s.thumbs[1] == pics[1]       # pictures too
    assert list(z["fid"]) == [f"f{i}" for i in range(6)] and "h1" in s.photos         # where they are is kept
    assert db.printless(2) and not db.usable(2) and L.IGNORE not in db.gallery()
    assert db.undo() == "Ignored 2 faces"                       # Undo, in the same session, brings them back
    z = _npz(data / "faces.npz")
    assert np.allclose(z["emb"][2], emb2) and F.Store().thumbs[2] == pics[2] and not db.printless(2)


def test_faces_ignored_before_lose_their_prints_at_start(data):
    make_faces(data, 6, thumbs=[_jpeg(5)] * 6)
    F.Store().save()
    (data / "labels.json").write_text(json.dumps({"f4": {"n": L.IGNORE, "by": "me"}, "f0": {"n": "Ann", "by": "me"}}))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    assert db.printless(4) and not db.printless(0) and not _npz(data / "faces.npz")["emb"][4].any()
    assert F.Store().thumbs[4] == b"" and db.name(4) == L.IGNORE


def test_a_copy_of_a_photo_with_an_ignored_face_is_still_a_copy(data):
    emb = make_faces(data, 6)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    db.act("ignore", [2], None)                                 # photo h1 holds faces 2 (ignored) and 3
    assert db.same_faces("h1", [emb[2], emb[3]])                 # its copy: the same faces again
    assert db.same_faces("h1", [emb[5], emb[3]])                 # the ignored one can't be compared any more
    assert not db.same_faces("h1", [emb[0], emb[5]])             # face 3 isn't in it: another photo
    db.act("ignore", [3], None)
    assert db.same_faces("h1", [emb[0], emb[1]])                 # only ignored faces: the fingerprint decides
    assert not db.same_faces("h1", [emb[0]])


def test_ignored_faces_never_teach_the_recognizer_or_get_suggested(data):
    make_faces(data, 6)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    db.act("ignore", [0, 1, 2], None)
    db.act("name", [3], "Ann")
    assert set(db.gallery()) == {"Ann"} and all(nm != L.IGNORE for _, nm, _, _ in db.triage())
    assert list(_npz(data / "people.npz")["name"]) == ["Ann"]


def _jpeg(v):
    return cv2.imencode(".jpg", np.full((112, 112, 3), v, np.uint8))[1].tobytes()


def test_old_database_pictures_move_to_thumbs_npz(data):
    pics = [_jpeg(10 * i) for i in range(4)]
    make_faces(data, 4, thumbs=pics)
    s = F.Store()
    assert s.thumbs_changed and s.thumb(2) is not None
    s.save()
    assert "thumbs" not in np.load(data / "faces.npz").files and (data / "thumbs.npz").exists()
    again = F.Store()
    assert not again.thumbs_changed and again.thumbs == pics


def test_pictures_are_matched_by_face_id(data):
    pics = [_jpeg(10 * i) for i in range(4)]
    make_faces(data, 4, thumbs=pics)
    F.Store().save()
    z = dict(np.load(data / "faces.npz"))
    order = [3, 2, 1, 0]                                   # the database reordered; thumbs.npz isn't
    for k in ("emb", "files", "fid", "photo", "box", "quality"):
        z[k] = z[k][order]
    np.savez(data / "faces.npz", **z)
    assert F.Store().thumbs == [pics[i] for i in order]


def test_no_pictures_on_this_computer(data):
    make_faces(data, 3)
    s = F.Store()
    assert s.thumb(0) is None and len(s.thumbs) == 3


def test_problems_keep_the_last_files_and_why():
    st = {"errors": 0}
    for k in range(L.PROBLEMS_KEPT + 5):
        L.note_problem(st, f"D:/Photos/{k}.jpg", OSError(f"broken {k}"))
    L.note_problem(st, "D:/Photos/video.mov", "couldn't read the video")
    L.note_problem(st, "D:/Photos/x.jpg", ValueError())
    assert st["errors"] == L.PROBLEMS_KEPT + 7 and len(st["problems"]) == L.PROBLEMS_KEPT
    assert st["problems"][-3] == {"file": "D:/Photos/24.jpg", "why": "OSError: broken 24"}
    assert st["problems"][-2]["why"] == "couldn't read the video" and st["problems"][-1]["why"] == "ValueError"


def test_the_faceness_model_trains_in_the_background_without_holding_the_lock(data, monkeypatch):
    """Training on thousands of faces takes seconds (minutes on a slow computer): the labeling page and the shared
    database's sync must not wait for it. Until it's done, every possible face is shown."""
    import threading
    pytest.importorskip("sklearn")                       # the app's own requirement; not installed in CI's checks
    make_faces(data, 160)
    photos = json.loads((data / "photos.json").read_text("utf-8"))
    for k in range(40, 80):                              # half the photos: their faces are possible faces
        photos[f"h{k}"]["maybe"] = {f"f{2 * k}": 0.5, f"f{2 * k + 1}": 0.5}
    (data / "photos.json").write_text(json.dumps(photos), "utf-8")
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    started, release = threading.Event(), threading.Event()
    real = L.LabelDB._fit

    def slow_fit(E, pos, neg):
        started.set()
        release.wait(10)
        return real(E, pos, neg)
    monkeypatch.setattr(L.LabelDB, "_fit", staticmethod(slow_fit))
    with db.lock:
        assert db.face_cut() == 0.0                      # nothing yet: show every possible face
    assert started.wait(5)
    assert db.lock.acquire(timeout=2)                    # training runs without the lock
    db.lock.release()
    release.set()
    db._fj_thread.join(20)
    with db.lock:
        assert db.face_cut() > 0.0 and db.faceness(np.asarray(db.store.emb[:2])) is not None
