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


def test_a_library_with_no_faces_yet_is_saved_and_read_again(data):
    """A new device: nothing in data/. Saving it (the first thing joining a shared database does) once failed."""
    s = F.Store()
    s.save()
    again = F.Store()
    assert again.emb == [] and again.fid == [] and again.thumbs == [] and again.model == F.REC_NAME
    with np.load(data / "faces.npz") as z:
        assert z["emb"].shape == (0, F.REC["dim"]) and z["box"].shape == (0, 5)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    assert db.n == 0 and db.summary()["faces"] == 0


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


class Models:                                    # stands in for face_id.Faces: a print and a quality from a picture
    def embed(self, crop):
        v = np.zeros(512, np.float32)
        v[int(crop[0, 0, 0])] = 1.0
        return v

    def quality(self, crop):
        return 0.8


def photos_here(db, monkeypatch, here=None):
    """The photos of faces `here` (every face, when None) are on this computer: their faces are found in them again."""
    monkeypatch.setattr(F, "rebuild_crop", lambda store, i, faces=None:
                        None if here is not None and i not in here else np.full((112, 112, 3), 40 + i, np.uint8))
    db.models = Models


def test_ignore_and_clear(data, monkeypatch):
    db = db_with_faces(data)
    photos_here(db, monkeypatch)
    db.act("ignore", [3], None)
    assert db.name(3) == L.IGNORE
    db.act("clear", [3], None)
    assert db.rec(3) == {} and not db.printless(3) and db.store.thumbs[3]     # a new face again, with its picture


def test_naming_an_ignored_face_makes_its_print_and_picture_again(data, monkeypatch):
    """Ignore deletes both. A face named afterwards stayed a grey placeholder that never taught the recognizer."""
    emb = make_faces(data, 6)
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    photos_here(db, monkeypatch)
    db.act("ignore", [2, 3], None)
    assert db.act("name", [2], "Ann") == 1
    assert db.name(2) == "Ann" and not db.printless(2) and db.usable(2) and db.gallery() == {"Ann": [2]}
    assert db.store.emb[2][42] == 1 and db.store.thumb(2) is not None            # from the photo, by the models
    s = F.Store()
    assert s.emb[2][42] == 1 and s.thumbs[2] and s.printless(3)                   # saved; the other stays Ignored
    assert list(_npz(data / "people.npz")["name"]) == ["Ann"]
    assert db.undo() == "Named 1 face Ann"                       # Undo: Ignored again, so both are deleted again
    assert db.name(2) == L.IGNORE and db.printless(2) and db.store.thumbs[2] == b"" and F.Store().printless(2)
    assert db.undo() == "Ignored 2 faces" and np.allclose(db.store.emb[2], emb[2])    # and the Ignore undone


def test_an_ignored_face_whose_photo_is_gone_stays_ignored(data, monkeypatch):
    db = db_with_faces(data)
    photos_here(db, monkeypatch, here={3})
    db.act("ignore", [2, 3], None)
    assert db.act("name", [2, 3], "Ann") == 1                    # the one that can be made again is named
    assert db.name(3) == "Ann" and db.name(2) == L.IGNORE and db.printless(2)
    for action, name in (("name", "Ann"), ("clear", None)):
        with pytest.raises(ValueError, match="can't be made again"):
            db.act(action, [2], name)
    assert db.name(2) == L.IGNORE and db.act("ignore", [2], None) == 1           # Ignoring it is always possible
    with pytest.raises(ValueError):
        db.rename(L.IGNORE, "Ann")                               # Ignored isn't a person to rename or merge
    assert db.act("name", [0, 0], "Bob") == 1 and db.undo() and db.rec(0) == {}   # a face given twice: once


def test_faces_left_without_prints_by_older_versions_are_mended(data, monkeypatch):
    make_faces(data, 6)
    z = _npz(data / "faces.npz")
    z["emb"][[2, 3, 4]] = 0                                      # ignored once; then 2 was named, 3 un-ignored
    np.savez(data / "faces.npz", **z)
    (data / "labels.json").write_text(json.dumps({"f2": {"n": "Ann", "by": "me"}, "f4": {"n": L.IGNORE, "by": "me"}}))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    assert db.lost_prints() == [2, 3] and db.gallery() == {}
    photos_here(db, monkeypatch, here={2, 4})
    assert db.mend(db.lost_prints()) == 1                        # 3's photo isn't here
    assert not db.printless(2) and db.gallery() == {"Ann": [2]} and list(_npz(data / "people.npz")["name"]) == ["Ann"]
    assert db.lost_prints() == [3] and db.printless(4) and F.Store().thumbs[2]


def test_a_moved_video_is_found_again_like_a_photo(data, tmp_path, monkeypatch):
    """Only pictures were looked for by name in the photo folders, so a moved video kept its old path for good."""
    import relocate
    new = tmp_path / "library" / "2024"
    new.mkdir(parents=True)
    for name in ("clip.mp4", "photo.jpg", "other.mp4"):
        (new / name).write_bytes(b"x" * (100 if name != "other.mp4" else 7))
    old = tmp_path / "gone"
    store = F.Store()
    store.photos = {"hv": {"path": str(old / "clip.mp4"), "size": 100, "video": True},
                    "hp": {"path": str(old / "photo.jpg"), "size": 100},
                    "ho": {"path": str(old / "other.mp4"), "size": 100}}        # same name, another size: not it
    monkeypatch.setattr(relocate, "_drives", lambda: [])
    r = relocate.relocate(store, [str(tmp_path / "library")], log=lambda *a: None)
    assert (r["missing"], r["found"]) == (3, 2)
    assert store.photos["hv"]["path"] == str(new / "clip.mp4") and store.photos["hp"]["path"] == str(new / "photo.jpg")
    assert store.photos["ho"]["path"] == str(old / "other.mp4")


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


def test_a_name_given_by_hand_is_no_longer_a_forgotten_name(data, tmp_path, monkeypatch):
    import dbsync_home
    import paths
    monkeypatch.setattr(paths, "HOME", tmp_path / "home")
    db = db_with_faces(data)
    for n in ("Ann", "Cy", "Dee"):
        dbsync_home.note_forgotten(n)
    db.act("reject", [0], "Ann")                             # saying who it isn't names nobody
    assert dbsync_home.forgotten_names() == {"Ann", "Cy", "Dee"}
    db.act("name", [0], "Ann")
    db.act("name", [1], "Bob")
    assert dbsync_home.forgotten_names() == {"Cy", "Dee"}
    db.rename("Bob", "Cy")
    assert dbsync_home.forgotten_names() == {"Dee"}
    # a list that can't be read is "no names" for the pages, but the sync must not send that on as every forget withdrawn
    (tmp_path / "home" / dbsync_home.FORGOTTEN).write_text("{not json")
    assert dbsync_home.forgotten_names() == set()
    with pytest.raises(OSError):
        dbsync_home.forgotten_names(strict=True)


def test_a_copy_of_a_photo_with_possible_faces_is_still_a_copy(data):
    """Possible faces (kept with the low detector cut-off) aren't among a new photo's regular faces: counting them
    made every copy of such a photo look like a different photo."""
    emb = make_faces(data, 6)                                # photo h0 holds f0 and f1
    photos = json.loads((data / "photos.json").read_text())
    photos["h0"]["maybe"] = {"f1": 0.5}                      # f1 is a possible face
    (data / "photos.json").write_text(json.dumps(photos))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    assert db.same_faces("h0", [emb[0]])                     # the copy's one regular face is f0
    assert not db.same_faces("h0", [emb[4]])                 # another face: not a copy
    assert not db.same_faces("h0", [emb[0], emb[1]])
    assert db.same_faces("h1", [emb[2], emb[3]])             # a photo without possible faces, as before


def test_a_merge_leaves_no_face_that_is_and_is_not_the_same_person(data):
    db = db_with_faces(data)
    db.act("name", [0], "Ann")
    db.act("name", [1], "Bob")
    db.act("reject", [1], "Ann")                             # Bob's face: "not Ann"
    db.rename("Ann", "Bob")                                  # Ann was Bob all along
    assert db.rec(1) == {"n": "Bob", "by": "me"} and db.name(0) == "Bob"


def test_a_bad_face_number_changes_nothing(data):
    db = db_with_faces(data)
    with pytest.raises(ValueError):
        db.act("name", [0, 1, 99], "Ann")                    # the last one doesn't exist: the first two stay as they were
    assert db.labels == {} and db.name(0) is None


def test_a_skipped_copy_is_not_what_later_copies_are_compared_with(data):
    make_faces(data, 6)
    photos = json.loads((data / "photos.json").read_text())
    photos["h0"]["fp"] = "00" * 32                           # the original
    photos["h1"].update(fp="00" * 31 + "01", dup_of="h0", faces=0)   # a copy of it, skipped at import
    (data / "photos.json").write_text(json.dumps(photos))
    db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
    assert db.near_duplicate("00" * 31 + "01") == "h0"       # a third version, closest to the copy: still the original
