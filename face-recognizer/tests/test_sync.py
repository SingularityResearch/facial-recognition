"""Two devices sharing one database (dbsync.py), run one after the other in this process: each has its own data/,
FaceLabels folder, device id and sync state. Needs FACE_TEST_PG (see test_facedb.py)."""
import json
import os
import uuid

import numpy as np
import psycopg
import pytest

import face_id as F
import labels as L
from conftest import make_faces

PG = os.environ.get("FACE_TEST_PG")
pytestmark = pytest.mark.skipif(not PG, reason="FACE_TEST_PG not set (no test database)")


class Device:
    """One device: switch() makes its folders and id the current ones; then use .db and .sync."""

    def __init__(self, root, monkeypatch, n, seed=0, labels=None):
        import dbsync
        import facedb
        import paths
        self.mp, self.facedb, self.paths = monkeypatch, facedb, paths
        self.data, self.home = root / "data", root / "home"
        self.data.mkdir(parents=True)
        self.home.mkdir()
        self.id = str(uuid.uuid4())
        (self.home / "device.json").write_text(json.dumps({"id": self.id, "name": root.name}))
        self.switch()
        make_faces(self.data, n, thumbs=[b"\xff\xd8pic%d" % i for i in range(n)], seed=seed)
        F.Store().save()
        if labels:
            (self.data / "labels.json").write_text(json.dumps(labels))
        self.db = L.LabelDB(folders=[], backup=False, log=lambda *a: None)
        self.web = {}
        import dbsync_home
        import persondata
        self.forgot = []

        def forget(name):
            self.switch()
            self.forgot.append(name)
            persondata.forget(self.db, name, self.web, None, known=True)
        self.sync = dbsync.Sync(dbsync.Local(self.db, self.web, lambda: None), "test", log=lambda *a: None,
                                state_path=self.home / "sync.sqlite", home=dbsync_home.HomePart(forget=forget))
        F.SAVED_HOOKS.append(self.sync.note)          # as label_tool.start_sync does

    def switch(self):
        import alerts  # noqa: F401
        import describe
        import secretbox
        import websearch
        h = self.home
        self.mp.setattr(F, "DATA", self.data)
        for mod, attr, val in ((self.paths, "HOME", h), (self.facedb, "DEVICE", h / "device.json"),
                               (F, "WEBCAM", h / "webcam"), (F, "PRESENCE", h / "presence"), (F, "ALERTS", h / "alerts.json"),
                               (describe, "STORE", h / "descriptions.json"), (describe, "CONFIG", h / "describe.json"),
                               (describe, "_saved", None), (websearch, "HOME", h),
                               (websearch, "KEY_FILE", h / "google_vision_key.txt"),
                               (websearch, "AWS_FILE", h / "aws_rekognition.json"), (secretbox, "KEY_FILE", h / "secret.key"),
                               (secretbox, "DEVICE_KEY", h / "device-key.json")):
            self.mp.setattr(mod, attr, val)
        import tracker
        self.mp.setattr(tracker, "PRESENCE", h / "presence")
        return self

    def step(self):
        self.switch()
        return self.sync.step()

    def name(self, fid):
        return self.db.labels.get(fid, {}).get("n")


@pytest.fixture
def shared(monkeypatch):
    import psycopg

    import facedb
    schema = "t_" + uuid.uuid4().hex[:12]
    admin = psycopg.connect(PG, autocommit=True)
    try:
        admin.execute("CREATE EXTENSION IF NOT EXISTS vector")
    except psycopg.Error:
        pass
    admin.execute(f"CREATE SCHEMA {schema}")
    monkeypatch.setenv("FACE_DB", PG)
    monkeypatch.setenv("FACE_DB_SCHEMA", schema)
    monkeypatch.setattr(F, "SAVED_HOOKS", [])
    yield facedb
    admin.execute(f"DROP SCHEMA {schema} CASCADE")
    admin.close()


def test_two_devices_join_and_share_labels_ignores_and_hand_labels(shared, tmp_path, monkeypatch):
    a = Device(tmp_path / "a", monkeypatch, 6, labels={"f0": {"n": "Ann", "by": "me"}})
    assert "joined" in a.step()
    b = Device(tmp_path / "b", monkeypatch, 4, labels={"f1": {"n": "Bob", "by": "auto", "s": 0.5}})
    assert "joined" in b.step()
    # b now has every face a had (with prints and pictures) and both devices' labels
    assert b.db.n == 6 and b.name("f0") == "Ann" and b.name("f1") == "Bob"
    i5 = b.db.store.fid.index("f5")
    assert b.db.store.thumbs[i5] == b"\xff\xd8pic5" and np.any(b.db.store.emb[i5])
    a.step()
    assert a.name("f1") == "Bob"                                  # b's label reached a

    # a name given on b reaches a, and a's recognizer list (people.npz) is rebuilt with it
    b.switch()
    b.db.act("name", [b.db.store.fid.index("f2")], "Cy")
    b.step()
    a.step()
    assert a.name("f2") == "Cy" and a.db.labels["f2"]["by"] == "me"
    assert "Cy" in set(np.load(a.data / "people.npz")["name"])

    # Ignore on a: b's copy of the face loses its print and picture too
    a.switch()
    a.db.act("ignore", [a.db.store.fid.index("f3")], None)
    a.step()
    b.step()
    i3 = b.db.store.fid.index("f3")
    assert b.name("f3") == L.IGNORE and b.db.store.printless(i3) and b.db.store.thumbs[i3] == b""

    # a hand label on b beats an automatic one sent later by a
    b.switch()
    b.db.act("name", [b.db.store.fid.index("f4")], "Eve")
    b.step()
    a.switch()
    a.db.labels["f4"] = {"n": "Dee", "by": "auto", "s": 0.6}
    F.save_json("labels.json", a.db.labels)
    a.step()
    a.step()
    assert a.name("f4") == "Eve" and b.name("f4") == "Eve"
    with shared.connect() as c:
        assert c.execute("SELECT n, by FROM labels WHERE fid = 'f4'").fetchone() == ("Eve", "me")
        assert c.execute("SELECT count(*) FROM label_history").fetchone()[0] >= 3   # each device's undo log


def test_offline_changes_wait_and_are_sent_later(shared, tmp_path, monkeypatch):
    a = Device(tmp_path / "a", monkeypatch, 4)
    a.step()
    monkeypatch.setenv("FACE_DB", "host=127.0.0.1 port=1 dbname=x user=x password=x connect_timeout=1")
    a.sync._close()
    a.switch()
    a.db.act("name", [0], "Ann")
    with pytest.raises(psycopg.OperationalError):
        a.step()                                                  # can't reach it: the change waits
    assert a.db.labels["f0"]["n"] == "Ann"                        # still saved on this device
    monkeypatch.setenv("FACE_DB", PG)
    a.step()
    with shared.connect() as c:
        assert c.execute("SELECT n FROM labels WHERE fid = 'f0'").fetchone() == ("Ann",)


def test_nothing_is_sent_twice(shared, tmp_path, monkeypatch):
    a = Device(tmp_path / "a", monkeypatch, 4)
    a.step()                                                      # joins
    a.step()                                                      # sends what joining didn't (this device's settings)
    a.sync.note()
    assert a.step().get("pushed") == 0
    with shared.connect() as c:
        before = c.execute("SELECT max(seq) FROM changes").fetchone()[0]
    a.sync.note()
    a.step()
    with shared.connect() as c:
        assert c.execute("SELECT max(seq) FROM changes").fetchone()[0] == before


def test_private_data_keys_and_forgets_reach_the_other_device(shared, tmp_path, monkeypatch):
    import alerts
    import describe
    import persondata
    import secretbox
    import tracker
    import websearch
    a = Device(tmp_path / "a", monkeypatch, 4, labels={"f0": {"n": "Ann", "by": "me"}})
    a.step()
    b = Device(tmp_path / "b", monkeypatch, 4)
    b.step()

    # on a: a description, an alert, a visit, a Collect faces snapshot and a Web search key
    a.switch()
    describe.save("h0", {"text": "Ann at the beach", "model": "m", "secs": 1})
    alerts.add({"kind": "arrived", "name": "Ann", "text": "Ann arrived", "t": int(tracker.time.time() * 1000)})
    tracker.log_visit({"name": "Ann", "start": 1_700_000_000_000, "end": 1_700_000_060_000})
    (a.home / "webcam").mkdir(exist_ok=True)
    (a.home / "webcam" / "webcam-20261112-010000-000.json").write_text(json.dumps({"when": "2026-11-12T01:00:00Z", "faces": []}))
    (a.home / "webcam" / "webcam-20261112-010000-000.jpg").write_bytes(b"JPGsnap")
    websearch.save_key("A" * 39)
    b.step()                                                              # b's public key reaches the database
    a.switch()
    with shared.connect() as c:
        with pytest.raises(ValueError):
            secretbox.set_passphrase(c, "short", "short")
        with pytest.raises(ValueError):
            secretbox.set_passphrase(c, "correct horse battery", "correct horse batterY")   # typed twice, differently
        r = secretbox.set_passphrase(c, "correct horse battery", "correct horse battery")
        assert r["what"] == "set"
    assert secretbox.saved_passphrase() == "correct horse battery"         # kept on the device, for Settings
    a.sync.note()
    a.step()
    a.step()
    with shared.connect() as c:
        row = c.execute("SELECT ciphertext FROM secrets WHERE name = 'google_vision'").fetchone()
        assert row and b"AAAA" not in bytes(row[0])                       # encrypted on the device

    # b takes the passphrase by itself (sealed to its key), and with it the key
    b.step()
    b.step()
    b.switch()
    assert secretbox.saved_passphrase() == "correct horse battery"
    assert describe.saved("h0")["text"] == "Ann at the beach"
    assert [x["text"] for x in alerts.listing()["alerts"]] == ["Ann arrived"]
    day = tracker.time.strftime("%Y-%m-%d", tracker.time.localtime(1_700_000_000))
    assert [v["name"] for v in tracker.visits(day)] == ["Ann"]
    assert (b.home / "webcam" / "webcam-20261112-010000-000.jpg").read_bytes() == b"JPGsnap"
    assert websearch.key() == "A" * 39                                       # decrypted with b's own passphrase key

    # b changes it; a takes the new one by itself and still reads the key
    with shared.connect() as c:
        assert secretbox.set_passphrase(c, "a brand new passphrase", "a brand new passphrase")["what"] == "changed"
    a.step()
    a.step()
    a.switch()
    assert secretbox.saved_passphrase() == "a brand new passphrase" and websearch.key() == "A" * 39
    with shared.connect() as c:
        ov = secretbox.overview(c)
        assert {d["name"]: d["current"] for d in ov["devices"]} == {"a": True, "b": False} or             [d for d in ov["devices"] if d["name"] == "a"][0]["current"]
        first = [v for v in ov["versions"] if v["note"] == "set"][0]["id"]
        # a wrong change undone from a: the first passphrase is current again, everywhere
        assert secretbox.restore(c, first)["what"] == "restored"
    b.step()
    b.step()
    b.switch()
    assert secretbox.saved_passphrase() == "correct horse battery" and websearch.key() == "A" * 39
    a.step()
    with shared.connect() as c:
        ov = secretbox.overview(c)
        assert all(d["current"] for d in ov["devices"]), ov["devices"]       # every device on the same passphrase
        fps = {d["fingerprint"] for d in ov["devices"]}
        assert fps == {ov["current"]["fingerprint"]}

    # the Web search cap counts every device's searches
    monkeypatch.setattr(websearch, "shared_count", __import__("dbsync_home").count_use)
    monkeypatch.setitem(websearch.CAPS, "vision", 2)
    websearch._count("vision")
    a.switch()
    websearch._count("vision")
    with pytest.raises(ValueError):
        websearch._count("vision")

    # forgetting Ann on a reaches b's files too
    a.switch()
    persondata.forget(a.db, "Ann", a.web, None)
    a.sync.note()
    a.step()
    b.step()
    b.step()
    b.switch()
    assert b.forgot == ["Ann"]
    assert alerts.listing()["alerts"] == [] and tracker.visits(day) == [] and describe.saved("h0") is None
    assert b.name("f0") == "#ignore"
    with shared.connect() as c:
        assert c.execute("SELECT count(*) FROM alerts").fetchone()[0] == 0
        assert c.execute("SELECT count(*) FROM presence").fetchone()[0] == 0


def test_settings_stay_each_devices_own_and_signals_are_shared(shared, tmp_path, monkeypatch):
    import dbsync_home
    import devicesettings
    a = Device(tmp_path / "a", monkeypatch, 2)
    a.step()
    b = Device(tmp_path / "b", monkeypatch, 2)
    b.step()
    a.switch()
    devicesettings.update("track", {"ft-o-optMesh": "1", "ft-camera": "cam-a"})
    devicesettings.set_signals("usual", {"Ann": {"p": [0.1] * 7, "n": 150}})
    a.sync.note()
    a.step()
    b.step()
    b.switch()
    assert devicesettings.settings("track") == {}                         # a's settings stay a's
    assert devicesettings.signals()["usual"]["Ann"]["n"] == 150             # what was learned about Ann is shared
    with shared.connect() as c:
        rows = c.execute("SELECT key, value FROM device_settings WHERE device = %s AND app = 'track'", (a.id,)).fetchall()
        assert dict(rows) == {"ft-o-optMesh": "1", "ft-camera": "cam-a"}
        assert c.execute("SELECT count(*) FROM device_settings WHERE device = %s AND app = 'describe'", (b.id,)).fetchone()[0] == 4
        # b takes a's settings
        assert dbsync_home.copy_settings(c, a.id, b.sync.parts[-1]) >= 2
    b.switch()
    assert devicesettings.settings("track") == {"ft-o-optMesh": "1", "ft-camera": "cam-a"}
    with pytest.raises(ValueError):
        with shared.connect() as c:
            dbsync_home.copy_settings(c, b.id, b.sync.parts[-1])            # not from itself


def test_a_restart_sends_nothing_again(shared, tmp_path, monkeypatch):
    """What a device took from the database hashes the same after the program reads its files again: floats come
    back from the database's real columns as short decimals (0.9) and from the files as float32 (0.8999999762)."""
    import dbsync
    a = Device(tmp_path / "a", monkeypatch, 4)
    a.step()
    b = Device(tmp_path / "b", monkeypatch, 2, seed=7)
    b.step()
    b.step()
    b.switch()
    again = dbsync.Local(L.LabelDB(folders=[], backup=False, log=lambda *a: None)).digests()
    for tbl in ("photos", "faces", "labels"):
        synced = b.sync.state.digests(tbl)
        assert {k: d for k, d in again[tbl].items() if synced.get(k) != d} == {}, tbl
