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
                               (secretbox, "DEVICE_KEY", h / "device-key.json"),
                               (secretbox, "REQUEST", h / "key-request.json"),
                               (__import__("peer"), "CERT", h / "peer-cert.pem"), (__import__("peer"), "KEY", h / "peer-key.pem"),
                               (__import__("peer"), "CONF", h / "peer.json")):
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

    # b isn't sent it for being in the database: the owner asks on b and types b's code on a; then b has the key
    b.step()
    b.switch()
    assert secretbox.saved_passphrase() == ""
    with shared.connect() as c:
        code = secretbox.ask(c)["code"]
    a.switch()
    with shared.connect() as c:
        assert secretbox.approve(c, code)["what"] == "approved"
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
    # (a as a device coming from 1.22.1, whose file doesn't list the passphrases it has had: it notes them once)
    kept = json.loads((a.home / "secret.key").read_text())
    (a.home / "secret.key").write_text(json.dumps({k: v for k, v in kept.items() if k != "known"}))
    a.step()
    a.step()
    a.switch()
    assert secretbox.saved_passphrase() == "a brand new passphrase" and websearch.key() == "A" * 39
    assert len(secretbox._known()) == 2
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


def test_the_database_password_alone_gets_nobody_the_passphrase(shared, tmp_path, monkeypatch):
    """Whoever holds the database password can write any row. That must not get them the passphrase for keys or a
    key: a device is given the passphrase only when the owner types it there, or types that device's one-time code
    on a device that has it; and no device takes a passphrase that wasn't made by someone holding the one it has."""
    import secretbox
    import websearch
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
    a = Device(tmp_path / "a", monkeypatch, 2)
    a.step()
    a.switch()
    websearch.save_key("A" * 39)
    with shared.connect() as c:
        secretbox.set_passphrase(c, "correct horse battery", "correct horse battery")
        first = secretbox.current(c)
    a.sync.note()
    a.step()
    a.step()

    # an intruder with the database password adds a "device" with their own key pair, saying it has the passphrase
    thief = X25519PrivateKey.generate()
    thief_pub = thief.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    thief_id = str(uuid.uuid4())

    def shares_for_thief(c):
        return c.execute("SELECT count(*) FROM passphrase_shares WHERE device = %s", (thief_id,)).fetchone()[0]
    with shared.connect() as c:
        c.execute("""INSERT INTO devices (id, name, os, app_version, public_key, key_version, key_fingerprint, key_proof)
                     VALUES (%s, 'thief', 'x', 'x', %s, %s, %s, %s)""",
                  (thief_id, thief_pub, first["id"], first["fingerprint"], os.urandom(32)))
    a.step()
    a.step()
    with shared.connect() as c:
        assert shares_for_thief(c) == 0                                   # a row in devices is given nothing
        # nor does a copy of a real device's proof help: it is bound to that device's id and public key
        c.execute("UPDATE devices SET key_proof = (SELECT key_proof FROM devices WHERE id = %s) WHERE id = %s",
                  (a.id, thief_id))
    a.step()
    with shared.connect() as c:
        assert shares_for_thief(c) == 0

    # a second device of the owner's: it isn't sent the passphrase either, until the owner lets it in
    b = Device(tmp_path / "b", monkeypatch, 2)
    b.step()
    a.step()
    b.step()
    b.switch()
    assert secretbox.saved_passphrase() == "" and not websearch.key()
    with shared.connect() as c:
        with pytest.raises(secretbox.WrongPassphrase):
            secretbox.set_passphrase(c, "not the passphrase", "not the passphrase")    # a wrong one changes nothing
        assert secretbox.current(c)["id"] == first["id"]
        assert secretbox.set_passphrase(c, "correct horse battery", "correct horse battery")["what"] == "entered"
    b.step()
    b.step()
    b.switch()
    assert websearch.key() == "A" * 39                                    # typed there: b reads the keys

    # a third asks with a code. The code checks the request: with the intruder's public key put in its place, the
    # right code gives nothing; neither does a wrong code, nor an answer that wasn't made with the code
    d = Device(tmp_path / "d", monkeypatch, 2)
    d.step()
    d.switch()
    with shared.connect() as c:
        code = secretbox.ask(c)["code"]
        assert len(code) == 9 and secretbox.request()["code"] == code
        assert code.replace("-", "") not in json.dumps(
            [list(map(str, r)) for r in c.execute("SELECT * FROM key_requests").fetchall()])   # never in the database
        real = c.execute("SELECT public_key FROM key_requests WHERE device = %s", (d.id,)).fetchone()[0]
        c.execute("UPDATE key_requests SET public_key = %s WHERE device = %s", (thief_pub, d.id))
    a.switch()
    with shared.connect() as c:
        assert [w["name"] for w in secretbox.overview(c)["waiting"]] == ["d"]
        with pytest.raises(secretbox.WrongPassphrase):
            secretbox.approve(c, code)
        c.execute("UPDATE key_requests SET public_key = %s WHERE device = %s", (real, d.id))
        with pytest.raises(secretbox.WrongPassphrase):
            secretbox.approve(c, "AAAA-AAAA")
        with pytest.raises(ValueError):
            secretbox.approve(c, "AAAA")
        assert c.execute("SELECT sealed FROM key_requests WHERE device = %s", (d.id,)).fetchone()[0] is None
        # the intruder answers the request themselves, with a passphrase of their own sealed to d
        c.execute("UPDATE key_requests SET version = %s, sealed = %s, answer_mac = %s WHERE device = %s",
                  (first["id"], secretbox.seal(real, "the intruder's passphrase"), os.urandom(32), d.id))
    d.step()
    d.switch()
    assert secretbox.saved_passphrase() == ""                             # not taken: it lacks the code's check value
    with shared.connect() as c:                                           # a request older than 15 minutes is over
        c.execute("UPDATE key_requests SET sealed = NULL, answer_mac = NULL, version = NULL, "
                  "asked_at = now() - interval '16 minutes' WHERE device = %s", (d.id,))
    a.switch()
    with shared.connect() as c:
        with pytest.raises(ValueError, match="No device is waiting"):
            secretbox.approve(c, code)
    d.switch()
    with shared.connect() as c:
        code2 = secretbox.ask(c)["code"]
    assert code2 != code
    a.switch()
    with shared.connect() as c:
        assert secretbox.approve(c, code2.lower().replace("-", " "))["given"] == "d"   # typed loosely
    d.step()
    d.step()
    d.switch()
    assert secretbox.saved_passphrase() == "correct horse battery" and websearch.key() == "A" * 39
    assert secretbox.request() is None
    with shared.connect() as c:
        assert c.execute("SELECT count(*) FROM key_requests").fetchone()[0] == 0
        assert shares_for_thief(c) == 0

    # a change goes to the three devices that showed the passphrase, not to the intruder's row
    a.step()
    b.step()
    b.switch()
    with shared.connect() as c:
        assert secretbox.set_passphrase(c, "a brand new passphrase", "a brand new passphrase")["what"] == "changed"
        assert shares_for_thief(c) == 0
        assert c.execute("SELECT count(*) FROM passphrase_shares s JOIN passphrase_versions v ON v.id = s.version "
                         "WHERE v.current").fetchone()[0] == 3
    for dev in (a, d):
        dev.step()
        dev.switch()
        assert secretbox.saved_passphrase() == "a brand new passphrase" and websearch.key() == "A" * 39
    # the intruder puts a passphrase of their own in the list, sealed to a: a can't be made to "restore" it (it
    # restores only passphrases it has had itself)
    a.switch()
    with shared.connect() as c:
        real_id = secretbox.current(c)["id"]
        salt = os.urandom(16)
        key = secretbox._derive("the intruder's passphrase", salt, **secretbox.SCRYPT)
        nonce, ct = secretbox.encrypt(key, "check", secretbox.CHECK)
        fake = c.execute("""INSERT INTO passphrase_versions (salt, n, r, p, nonce, ciphertext, fingerprint, note,
                                                             prev, link)
                            VALUES (%s, %s, 8, 1, %s, %s, %s, 'set', %s, %s) RETURNING id""",
                         (salt, secretbox.SCRYPT["n"], nonce, ct, secretbox.fingerprint(key), real_id,
                          os.urandom(32))).fetchone()[0]
        c.execute("INSERT INTO passphrase_shares (version, device, sealed) VALUES (%s, %s, %s)",
                  (fake, a.id, secretbox.seal(secretbox.device_public(), "the intruder's passphrase")))
        ov = secretbox.overview(c)
        assert {v["id"]: v["known"] for v in ov["versions"]} == {first["id"]: True, real_id: True, fake: False}
        with pytest.raises(secretbox.WrongPassphrase, match="never had"):
            secretbox.restore(c, fake)
        assert secretbox.current(c)["id"] == real_id

        # then they make it "current": a doesn't take it, and sends no key under it; it keeps the one it has and
        # stops syncing keys until the owner sets things right
        with c.transaction():
            c.execute("UPDATE passphrase_versions SET current = false")
            c.execute("UPDATE passphrase_versions SET current = true WHERE id = %s", (fake,))
        before = c.execute("SELECT ciphertext FROM secrets WHERE name = 'google_vision'").fetchone()[0]
    websearch.save_key("B" * 39)
    a.sync.note()
    a.step()
    a.step()
    a.switch()
    assert secretbox.saved_passphrase() == "a brand new passphrase" and secretbox.local_version() == real_id
    assert a.sync.parts[-1].key_current is False
    with shared.connect() as c:
        assert bytes(c.execute("SELECT ciphertext FROM secrets WHERE name = 'google_vision'").fetchone()[0]) == bytes(before)
        with pytest.raises(secretbox.WrongPassphrase):
            secretbox.approve(c, "AAAA-AAAA")                             # and it gives nobody anything meanwhile


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

    # b's tracker page saves what it learned about Bo. It says only that (it may never have heard of Ann): Ann stays,
    # on b, in the database and on a
    devicesettings.merge_signals("usual", {"Bo": {"p": [0.2] * 7, "n": 9}})
    b.sync.note()
    b.step()
    a.step()
    for dev in (a, b):
        dev.switch()
        assert sorted(devicesettings.signals()["usual"]) == ["Ann", "Bo"]
    with shared.connect() as c:
        assert c.execute("SELECT count(*) FROM person_signals WHERE kind = 'usual'").fetchone()[0] == 2


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


def test_a_device_shows_another_its_original_photos(shared, tmp_path, monkeypatch):
    """b doesn't have a photo; a does: b gets its JPEG from a over TLS with a signed request, checked against a's
    certificate in the database. Unsigned requests, wrong certificates and photos a doesn't have get nothing."""
    import socket
    import ssl

    import cv2

    import peer
    H = "ab" * 20                                                 # a photo's content hash (SHA-1)
    a = Device(tmp_path / "a", monkeypatch, 2)
    pic = tmp_path / "a" / "photo.jpg"
    img = np.full((60, 80, 3), 120, np.uint8)
    cv2.rectangle(img, (10, 10), (40, 40), (0, 0, 255), -1)
    cv2.imwrite(str(pic), img)
    a.switch()
    a.db.store.photos[H] = {"path": str(pic), "size": pic.stat().st_size, "name": "photo.jpg"}
    a.db.store.save()
    a.step()
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port_a = s.getsockname()[1]
    monkeypatch.setattr(peer, "port", lambda: port_a)
    monkeypatch.setattr(peer, "addresses", lambda: ["127.0.0.1"])
    svc = peer.Service(lambda h, f: peer.own_photo(a.db.store, h, f), lambda: peer.CURRENT_KEY,
                       bind="127.0.0.1", listen_port=port_a).start()
    try:
        with shared.connect() as c:
            peer.CURRENT_KEY = peer.cluster_key(c)
            peer.register(c, True)
        b = Device(tmp_path / "b", monkeypatch, 2)
        b.step()                                                  # joins: knows h0 is on a
        b.switch()
        import socket
        silent = socket.create_connection(("127.0.0.1", port_a))   # connects and says nothing: must hold up nobody
        jpg, who = peer.fetch(H)
        silent.close()
        assert who == "a", who
        assert jpg[:2] == bytes([0xFF, 0xD8])
        got = cv2.imdecode(np.frombuffer(jpg, np.uint8), cv2.IMREAD_COLOR)
        assert got.shape == img.shape and abs(int(got[25, 25, 2]) - 255) < 40      # the red square came through
        assert not list((tmp_path / "b").rglob("photo*.jpg"))                      # shown, never kept on b
        with shared.connect() as c:
            fp = c.execute("SELECT peer_cert_sha256 FROM devices WHERE name = 'a'").fetchone()[0]
        with pytest.raises(PermissionError):
            peer._get("127.0.0.1", port_a, fp, "/photo/" + H, b"x" * 32)              # wrong key: refused (403)
        with pytest.raises(ssl.SSLError):
            peer._get("127.0.0.1", port_a, "0" * 64, "/photo/" + H, peer.CURRENT_KEY)  # not a's certificate
        assert peer.fetch("cd" * 20)[0] is None                 # a's records don't have that photo
    finally:
        svc.stop()


def test_watching_another_devices_camera_and_moving_between_cameras(shared, tmp_path, monkeypatch):
    """a streams its camera and readout; b watches through the peer service (TLS, signed). Ann seen on a, then on b:
    b raises "moved", which reaches a too."""
    import socket
    import time

    import alerts
    import camstream
    import peer
    camstream.reset()
    import devicesettings
    a = Device(tmp_path / "a", monkeypatch, 2)
    devicesettings.update("track", {"ft-alertWho": "Ann", "ft-o-optAlertMove": "1"})   # sightings: off until switched
                                                                    # on, and people on the alert list only
    a.step()
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port_a = s.getsockname()[1]
    monkeypatch.setattr(peer, "port", lambda: port_a)
    monkeypatch.setattr(peer, "addresses", lambda: ["127.0.0.1"])
    a.switch()
    svc = peer.Service(lambda h, f: None, lambda: peer.CURRENT_KEY, bind="127.0.0.1", listen_port=port_a).start()
    try:
        with shared.connect() as c:
            peer.CURRENT_KEY = peer.cluster_key(c)
            peer.register(c, True)
        frame = bytes([0xFF, 0xD8]) + b"a's camera" + bytes([0xFF, 0xD9])
        camstream.put(frame)
        camstream.set_in_view(["Ann", "Zed"])
        camstream.set_readout([{"name": "Ann", "sim": 70}])
        a.step()                                                   # a: Ann on its camera (sightings)
        with shared.connect() as c:                                # (a's own step marks it not sharing: the
            peer.register(c, True)                                 #  test runs its service apart)
            seen = [r[0] for r in c.execute("SELECT name FROM sightings")]
        assert seen == ["Ann"]                                     # Zed isn't on a's alert list: not written
        camstream.set_in_view(["Ann"])
        b = Device(tmp_path / "b", monkeypatch, 2)
        devicesettings.update("track", {"ft-alertWho": "ann, Bob", "ft-o-optAlertMove": "1"})
        b.step()
        b.switch()
        with shared.connect() as c:
            a_id = c.execute("SELECT id::text FROM devices WHERE name = 'a'").fetchone()[0]
        assert peer.fetch_readout(a_id)["faces"] == [{"name": "Ann", "sim": 70}]
        import camrelay                                            # the way the Cameras page gets it
        camstream.put(frame)                                       # (the page sends ~10 a second: a fresh one)
        camstream.set_readout([{"name": "Ann", "sim": 70}])
        relay = camrelay.get(a_id)
        assert relay.wait_next(0, timeout=10)[1] == frame, relay.error
        for _ in range(50):                                        # the readout rides along the same stream
            if relay.live_readout().get("faces"):
                break
            time.sleep(0.1)
        assert relay.live_readout()["faces"] == [{"name": "Ann", "sim": 70}]
        camrelay.stop_all()
        sock, ctype, first = peer.open_stream(a_id)
        try:
            assert ctype.startswith("multipart/x-mixed-replace")
            data = first
            while frame not in data:
                data += sock.recv(65536)
        finally:
            sock.close()
        # Ann now on b's camera (same process here: b's step sees the names; the page sends them every second)
        camstream.set_in_view(["Ann"])
        b.step()
        assert any(x["kind"] == "moved" and x["text"] == "Ann moved from a's camera to b's"
                   for x in alerts.listing()["alerts"])
        b.step()
        a.step()
        a.switch()
        assert any(x["kind"] == "moved" for x in alerts.listing()["alerts"])          # the alert reached a too
    finally:
        svc.stop()
        camstream.reset()


def test_a_change_committed_late_still_reaches_devices_that_pulled_meanwhile(shared, tmp_path, monkeypatch):
    """A long transaction (a device joining, a big import) takes its change-log numbers early and commits late. A
    device that pulled in between had already passed those numbers; it must still get the rows (db/009)."""
    import describe
    a = Device(tmp_path / "a", monkeypatch, 6)
    a.step()
    b = Device(tmp_path / "b", monkeypatch, 6)
    b.step()
    a.step()
    b.switch()
    slow = shared.connect()                                  # b's long transaction: written, not committed yet
    slow.execute("BEGIN")
    slow.execute("INSERT INTO descriptions (key, data) VALUES ('h1', '{\"text\": \"written early, committed late\"}')")
    b.db.act("name", [b.db.store.fid.index("f2")], "Cy")     # meanwhile b commits a later change ...
    b.step()
    a.step()
    assert a.name("f2") == "Cy"                              # ... which a pulls, moving past the open one's number
    a.switch()
    assert describe.saved("h1") is None
    slow.execute("COMMIT")
    slow.close()
    a.step()
    a.switch()
    assert describe.saved("h1")["text"] == "written early, committed late"
    assert a.step().get("pulled") == 0                       # and nothing is taken twice


def test_a_forgotten_name_given_again_is_not_forgotten_by_a_later_device(shared, tmp_path, monkeypatch):
    """The forgotten name is an instruction to devices that haven't forgotten yet. Once the name is given to someone
    again it must stop being one, or a device joining later would forget that new person on every device."""
    import dbsync_home
    import persondata
    a = Device(tmp_path / "a", monkeypatch, 6, labels={"f0": {"n": "Ann", "by": "me"}})
    a.step()
    b = Device(tmp_path / "b", monkeypatch, 6)
    b.step()
    a.switch()
    persondata.forget(a.db, "Ann", a.web, None)
    a.sync.note()
    a.step()
    b.step()
    b.switch()
    assert b.forgot == ["Ann"] and b.name("f0") == "#ignore" and dbsync_home.forgotten_names() == {"Ann"}

    a.switch()
    a.db.act("name", [a.db.store.fid.index("f4")], "Ann")    # another Ann, or the same one added again
    assert dbsync_home.forgotten_names() == set()
    a.step()
    with shared.connect() as c:
        assert c.execute("SELECT count(*) FROM forgotten").fetchone()[0] == 0
    b.step()
    b.switch()
    assert dbsync_home.forgotten_names() == set() and b.name("f4") == "Ann"

    late = Device(tmp_path / "late", monkeypatch, 6)         # joins after all of it
    late.step()
    late.step()
    a.step()
    b.step()
    assert late.forgot == [] and late.name("f4") == "Ann" and late.name("f0") == "#ignore"
    assert a.name("f4") == "Ann" and b.name("f4") == "Ann" and b.forgot == ["Ann"]
    i4 = a.db.store.fid.index("f4")
    assert np.any(a.db.store.emb[i4]) and not a.db.printless(i4)             # her print is still there


def test_expression_readings_go_to_the_database_and_go_with_a_forget(shared, tmp_path, monkeypatch):
    """The expression log (exprlog.py, db/010): a device's waiting readings are put in expression_log once each, a
    past day's waiting file goes when all of it is there, and forgetting a person removes their rows."""
    import os
    import time

    import exprlog
    import persondata
    a = Device(tmp_path / "a", monkeypatch, 6, labels={"f0": {"n": "Ann", "by": "me"}, "f2": {"n": "Bo", "by": "me"}})
    a.step()
    a.switch()
    now = int(time.time() * 1000)
    row = lambda name, t: {"t": t, "name": name, "label": "happy", "share": 0.8, "state": "plain",   # noqa: E731
                           "data": {"scores": {"happy": 0.8}, "setting": "alone"}}
    assert exprlog.add([row("Ann", now), row("Bo", now), row("Ann", now + 5000)]) == 3
    # yesterday's waiting file (UTC), written as the tracker would have yesterday: add() takes only readings of the
    # last day, and how far back "yesterday noon" is depends on the time of day the test runs
    from datetime import datetime, timedelta, timezone
    yday = (datetime.now(timezone.utc) - timedelta(days=1)).replace(hour=12, minute=0, second=0, microsecond=0)
    old_file = exprlog.folder() / f"{yday:%Y-%m-%d}.jsonl"
    old_file.write_text(json.dumps(row("Ann", int(yday.timestamp() * 1000))) + "\n", "utf-8")
    os.utime(old_file, (time.time() - 300, time.time() - 300))
    a.sync.note()
    a.step()
    with shared.connect() as c:
        rows = c.execute("SELECT name, label, share, state, data, device::text FROM expression_log ORDER BY at, name").fetchall()
        assert [r[0] for r in rows] == ["Ann", "Ann", "Bo", "Ann"] and rows[0][4]["scores"] == {"happy": 0.8}
        assert {r[5] for r in rows} == {a.id} and rows[1][1:4] == ("happy", 0.8, "plain")
    a.switch()
    assert not old_file.exists() and len(exprlog.waiting()) == 3          # yesterday's is in the database: gone here
    assert exprlog.add([row("Ann", now + 10_000)]) == 1                    # more of today: only the new line is sent
    a.sync.note()
    a.step()
    with shared.connect() as c:
        assert c.execute("SELECT count(*) FROM expression_log").fetchone()[0] == 5
    b = Device(tmp_path / "b", monkeypatch, 6)
    b.step()
    a.switch()
    persondata.forget(a.db, "Ann", a.web, None)                          # rewrites the waiting file: sent again, once each
    a.sync.note()
    a.step()
    a.step()
    with shared.connect() as c:
        assert c.execute("SELECT name FROM expression_log").fetchall() == [("Bo",)]
    a.switch()
    assert [r["name"] for r in exprlog.waiting()] == ["Bo"]


def test_the_sync_follows_a_change_of_the_connection_settings(shared, tmp_path, monkeypatch):
    """Settings -> Connect to another server or schema: the sync used to go on with the connection it had."""
    a = Device(tmp_path / "a", monkeypatch, 2)
    a.step()
    first = a.sync.conn
    a.step()
    assert a.sync.conn is first and not first.closed
    real = a.facedb._conninfo
    monkeypatch.setattr(a.facedb, "_conninfo", lambda: (real()[0] + " application_name=changed", real()[1]))
    a.step()
    assert a.sync.conn is not first and first.closed and not a.sync.conn.closed
