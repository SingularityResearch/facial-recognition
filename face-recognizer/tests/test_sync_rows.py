"""What a device takes from the shared database, row by row (dbsync.py, dbsync_home.py), without a database: the
rows are handed to the same code the sync hands them to. Whoever has the database's password can write any row,
and so can a faulty version: a row may never give this device a file path, change its settings, or stop the rows
after it. The same with a real database: test_sync.py."""
import json

import numpy as np
import pytest

import face_id as F
import labels as L
from conftest import make_faces


@pytest.fixture
def device(data, home, tmp_path, monkeypatch):
    """A device with six faces in three photos, its own private folder and id; .db, .local, .home, .sync."""
    import dbsync
    import dbsync_home
    import facedb
    import paths
    monkeypatch.setattr(paths, "HOME", home)
    monkeypatch.setattr(facedb, "DEVICE", home / "device.json")
    monkeypatch.setattr(F, "ALERTS", home / "alerts.json")
    monkeypatch.setattr(F, "PRESENCE", home / "presence")
    monkeypatch.setattr(F, "WEBCAM", home / "webcam")
    monkeypatch.setattr(F, "SAVED_HOOKS", [])
    make_faces(data, 6, thumbs=[b"\xff\xd8pic%d" % i for i in range(6)])
    F.Store().save()

    class Device:
        db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
        said = []
    d = Device()
    d.local, d.home = dbsync.Local(d.db, {}, lambda: None), dbsync_home.HomePart()
    d.sync = dbsync.Sync(d.local, "test", log=d.said.append, state_path=home / "sync.sqlite", home=d.home)
    d.me = facedb.device()["id"]
    yield d
    d.sync.state.close()


def face(**changes):
    return dict({"photo": "h9", "file": "new.jpg", "box": [1.0, 2.0, 30.0, 40.0, 0.9], "quality": 0.5,
                 "model": "arcface_w600k_r50", "print": np.full(512, 0.04, np.float32), "picture": b"\xff\xd8new",
                 "possible": None, "frame": None}, **changes)


def test_a_photo_row_never_gives_this_device_a_file_path(device, tmp_path, monkeypatch):
    """The photos table's `extra` column was copied into the photo's record whole, `path` included, and the peer
    service shows the file at a photo's path to every device that asks: a row could name any picture or video
    this user can read."""
    import cv2

    import peer
    secret = tmp_path / "private.jpg"
    cv2.imwrite(str(secret), np.full((40, 40, 3), 90, np.uint8))
    monkeypatch.setattr(F, "load_image", lambda p, data=None: cv2.imread(str(p)))    # (the real one needs Pillow)
    row = {"name": "photo0.jpg", "size": secret.stat().st_size, "fp": None, "faces": 2, "dup_of": None, "video": None,
           "fps": None, "maybe_read": False, "facts": None,
           "extra": {"path": str(secret), "mtime": 5, "at": {"f0": 3}, "note": "kept"}}
    assert device.sync._apply(device.local, "photos", {"h0": row, "h7": dict(row)}).keys() == {"h0", "h7"}
    for h in ("h0", "h7"):                                    # a photo this device has, and one it never read
        rec = device.db.store.photos[h]
        assert not rec.get("path") and "mtime" not in rec and "at" not in rec, rec
        assert rec["note"] == "kept" and rec["size"] == secret.stat().st_size      # other fields as before
        assert peer.own_photo(device.db.store, h, None) is None
    # and a record with a path but no size (none is ever made that way) is not shown either
    device.db.store.photos["h7"] = {"path": str(secret), "name": "private.jpg"}
    assert peer.own_photo(device.db.store, "h7", None) is None
    device.db.store.photos["h7"]["size"] = secret.stat().st_size
    assert peer.own_photo(device.db.store, "h7", None)[:2] == b"\xff\xd8"          # this device's own record: shown
    assert json.loads((F.DATA / "photos.json").read_text("utf-8"))["h0"].get("path") in (None, "")


def test_a_devices_settings_are_never_changed_by_a_row(device):
    """A changed device_settings row was applied on the device it names: the music listener (its microphones), the
    camera stream, where Describe photo sends photos. Now nothing is applied: the row is marked not as synced (sent
    again as it is here), and a row this device never wrote is deleted with the next push."""
    import describe
    import devicesettings
    import dbsync_home
    import hardware
    import lmstudio
    devicesettings.update("track", {"ft-o-optStream": "0"})
    before = (devicesettings.settings(), describe.config(), lmstudio.share(), hardware.distributed())
    me, mine = device.me, device.home._settings()
    rows = {f"{me}|track|ft-o-optStream": {"app": "track", "key": "ft-o-optStream", "value": "1"},
            f"{me}|track|ft-o-optKeepCam": {"app": "track", "key": "ft-o-optKeepCam", "value": "1"},
            f"{me}|label|fl-music": {"app": "label", "key": "fl-music", "value": "1"},
            f"{me}|describe|url": {"app": "describe", "key": "url", "value": "http://203.0.113.9:1234"},
            f"{me}|describe|spread": {"app": "describe", "key": "spread", "value": True},
            f"{me}|lmstudio|share": {"app": "lmstudio", "key": "share", "value": r"\\203.0.113.9\models"},
            f"{me}|lmstudio|distributed": {"app": "lmstudio", "key": "distributed", "value": "1"},
            f"{me}|describe|use": dict(mine[f"{me}|describe|use"])}                # this one as it is here
    out = device.sync._apply(device.home, "device_settings", rows)
    assert (devicesettings.settings(), describe.config(), lmstudio.share(), hardware.distributed()) == before
    assert out[f"{me}|track|ft-o-optStream"] is None and out[f"{me}|describe|url"] is None      # sent again
    assert out[f"{me}|track|ft-o-optKeepCam"] == dbsync_home.NOT_OURS == out[f"{me}|label|fl-music"]   # deleted there
    assert out[f"{me}|describe|use"] == dbsync_home._h(mine[f"{me}|describe|use"])              # in step
    assert "settings are different in the database" in device.said[-1] and "203.0.113.9" not in device.said[-1]

    # Copy settings (the owner's click on this device) takes the pages' settings and the describer chosen, never
    # where this device sends photos or fetches files
    device.home.take_settings({k: v for k, v in rows.items()})
    assert devicesettings.settings("track") == {"ft-o-optStream": "1", "ft-o-optKeepCam": "1"}
    assert (describe.config()["url"], describe.config()["spread"]) == (before[1]["url"], before[1]["spread"])
    assert (lmstudio.share(), hardware.distributed()) == before[2:]


def test_a_row_that_cant_be_used_is_left_out_and_the_others_are_taken(device):
    """One row that raised stopped the pull for good, on every device: the same row was read again at every step."""
    import alerts
    import describe
    me = device.me
    good_alert = {"id": "a1", "t": 1_800_000_000_000, "kind": "arrived", "name": "Ann", "text": "Ann arrived", "state": "new"}
    out = device.sync._apply(device.home, "alerts", {
        "a1": good_alert, "a2": "just text", "a3": {"id": "another", "t": 1}, "a4": {"id": "a4", "t": "soon"},
        "a5": {"id": "a5", "t": 5, "details": [1, 2]}, "a6": None})
    assert set(out) == {"a1", "a6"} and [a["id"] for a in alerts.listing()["alerts"]] == ["a1"]
    out = device.sync._apply(device.home, "descriptions", {"h0": {"text": "a beach"}, "h1": [1, 2], "h2": "text"})
    assert set(out) == {"h0"} and describe.saved("h0")["text"] == "a beach" and describe.saved("h1") is None
    visit = {"name": "Ann", "start": 1_700_000_000_000, "end": 1_700_000_060_000, "device": me}
    out = device.sync._apply(device.home, "presence", {
        f"{me}:1700000000000:Ann": visit,
        f"{me}:100000000000000000:Ann": dict(visit, start=10 ** 17, end=10 ** 17 + 1),      # no such day
        "made-up-id": visit, f"{me}:5:": dict(visit, start=5, name="")})
    assert set(out) == {f"{me}:1700000000000:Ann"} and len(device.home._presence()) == 1
    out = device.sync._apply(device.home, "person_signals", {"Ann|usual": {"name": "Ann", "kind": "usual", "data": []},
                                                              "no-kind": {"name": "x", "kind": "usual", "data": {}}})
    assert out == {} and device.home._signals() == {}
    assert device.sync._apply(device.home, "forgotten", {"#ignore": {}, "x" * 300: {}}) == {}
    assert device.home.digests()["alerts"].keys() == {"a1"}       # and the next look at this device's rows works

    # faces: a box of three numbers was saved with the rest, and no save worked after it
    n = device.db.n
    out = device.sync._apply(device.local, "faces", {
        "new1": face(), "bad-box": face(box=[1.0, 2.0, 3.0]), "bad-box2": face(box=[1.0, None, 3.0, 4.0, 0.5]),
        "bad-print": face(print=np.zeros(128, np.float32)), "bad-picture": face(picture=b"MZ" + b"\0" * 50),
        "huge-picture": face(picture=b"\xff\xd8" + b"\0" * 1_000_001), "no-photo": face(photo="")})
    assert set(out) == {"new1"} and device.db.store.fid[n:] == ["new1"]
    device.db.store.save()                                        # still saves, and reads again
    assert F.Store().fid[-1] == "new1"
    out = device.sync._apply(device.local, "photos", {"h0": {"name": "p", "extra": [1], "facts": None},
                                                      "h1": {"name": "p", "extra": {}, "facts": "tags"}})
    assert out == {}
    assert device.sync._apply(device.local, "web_suggestions", {"f0": 7, "f1": {"suggestions": []}}).keys() == {"f1"}
    assert device.sync._apply(device.local, "labels", {"f0": {"n": "Ann", "by": "me", "not": [None]},
                                                       "f1": {"n": "Bo", "by": "me"},
                                                       "f2": {"n": "__proto__", "by": "me"},      # labels.NOT_NAMES
                                                       "f3": {"n": "constructor", "by": "me"}}).keys() == {"f1"}
    assert device.sync._apply(device.home, "person_signals", {
        "constructor|usual": {"name": "constructor", "kind": "usual", "data": {}}}) == {}
    assert device.db.labels.get("f1", {}).get("n") == "Bo" and "f0" not in device.db.labels

    said = "\n".join(device.said)
    assert "the alerts row a2 can't be used here and was left out" in said and "box: not five numbers" in said
    device.sync._say_left_out()
    assert "more faces rows couldn't be used here" in device.said[-1]       # only the first few are named


def test_a_row_that_fails_while_applied_is_found_by_halving(device):
    """Should a row pass the check and still fail when applied, the rest of its batch is applied without it.
    Trouble of this device's own (a full disk) is no bad row: it is raised, and the step tried again."""
    class Part:
        applied = []

        def apply(self, tbl, rows, log):
            for r in rows.values():
                if r == "disk":
                    raise OSError("no space left")
                r["v"] + 1                                       # a row without v: KeyError; with text: TypeError
            Part.applied.append(sorted(rows))
            return {k: "d" for k in rows}
    rows = {f"k{i}": {"v": i} for i in range(9)}
    rows["k3"], rows["k7"] = {}, {"v": "seven"}
    assert sorted(device.sync._apply(Part(), "things", rows)) == [f"k{i}" for i in range(9) if i not in (3, 7)]
    assert sum(device.said[i].count("left out") for i in range(len(device.said))) == 2
    assert "(KeyError: 'v')" in device.said[0] and "TypeError" in device.said[1]
    with pytest.raises(OSError):
        device.sync._apply(Part(), "things", {"a": {"v": 1}, "b": "disk"})


def test_a_key_or_message_from_the_database_goes_into_the_log_as_one_short_line():
    import dbsync
    assert dbsync.printable("a\nb\x1b[31m" + "x" * 200) == "a?b?[31m" + "x" * 72
