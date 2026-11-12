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
        self.sync = dbsync.Sync(dbsync.Local(self.db, self.web, lambda: None), "test", log=lambda *a: None,
                                state_path=self.home / "sync.sqlite")
        F.SAVED_HOOKS.append(self.sync.note)          # as label_tool.start_sync does

    def switch(self):
        self.mp.setattr(F, "DATA", self.data)
        self.mp.setattr(self.paths, "HOME", self.home)
        self.mp.setattr(self.facedb, "DEVICE", self.home / "device.json")
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
    a.step()
    a.sync.note()
    assert a.step().get("pushed") == 0
    with shared.connect() as c:
        before = c.execute("SELECT max(seq) FROM changes").fetchone()[0]
    a.sync.note()
    a.step()
    with shared.connect() as c:
        assert c.execute("SELECT max(seq) FROM changes").fetchone()[0] == before
