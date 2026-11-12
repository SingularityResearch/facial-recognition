"""The shared database (facedb.py, db/*.sql, tools/db_import.py).

Without FACE_TEST_PG only the parts that need no server run: no database means files only, the password is never
kept in clear on Windows, bad settings are refused. With FACE_TEST_PG (a libpq connection string to a PostgreSQL with
pgvector, as in CI's database job) each test gets its own schema, dropped afterwards."""
import json
import os
import sys
import uuid
from pathlib import Path

import numpy as np
import pytest

import facedb

PG = os.environ.get("FACE_TEST_PG")
needs_pg = pytest.mark.skipif(not PG, reason="FACE_TEST_PG not set (no test database)")
CA = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n"


def test_no_database_means_files_only():
    assert facedb.config() is None and not facedb.configured()
    with pytest.raises(facedb.NotConfigured):
        facedb.connect()
    assert facedb.test()["ok"] is False


def test_the_password_is_protected_and_bad_settings_refused():
    c = facedb.save_config("db.example", 5432, "sentient", "facerec", "s3cret-pw", CA)
    assert c["host"] == "db.example" and c["ca"] is True and "password" not in c
    raw = facedb.CONFIG.read_text("utf-8")
    if sys.platform == "win32":
        assert "s3cret-pw" not in raw and json.loads(raw)["password"].startswith("dpapi:")
    assert facedb.unprotect(json.loads(raw)["password"]) == "s3cret-pw"
    with pytest.raises(ValueError):
        facedb.save_config("db.example", 5432, "sentient", "facerec", "pw", CA, schema="x; drop table")
    with pytest.raises(ValueError):
        facedb.save_config("db.example", 5432, "sentient", "facerec", "pw", "not a certificate")
    assert facedb.save_config("", 0, "", "", "", "") is None and facedb.config() is None


def test_the_device_keeps_its_id():
    a = facedb.device()
    assert uuid.UUID(a["id"]) and facedb.device() == a


def test_migrations_are_numbered_and_packed():
    ms = facedb.migrations()
    assert ms and ms[0][0] == 1 and [n for n, _ in ms] == sorted({n for n, _ in ms})
    build = (Path(__file__).resolve().parents[1] / "build_exe.py").read_text("utf-8")
    assert '"db"' in build or "'db'" in build, "build_exe.py must pack db/*.sql"


# --- with a test database ----------------------------------------------------------------------------------------

@pytest.fixture
def pg(monkeypatch):
    import psycopg
    schema = "t_" + uuid.uuid4().hex[:12]
    admin = psycopg.connect(PG, autocommit=True)
    try:
        admin.execute("CREATE EXTENSION IF NOT EXISTS vector")
    except psycopg.Error:
        pass                                    # created by the administrator already
    admin.execute(f"CREATE SCHEMA {schema}")
    monkeypatch.setenv("FACE_DB", PG)
    monkeypatch.setenv("FACE_DB_SCHEMA", schema)
    conn = facedb.open_ready("test")
    yield conn
    conn.close()
    admin.execute(f"DROP SCHEMA {schema} CASCADE")
    admin.close()


def make_device(folder, faces, labels):
    """A made-up device: data/ with faces.npz, thumbs.npz, photos.json, labels.json; HOME with a webcam snapshot."""
    data, home = folder / "data", folder / "home"
    (home / "webcam").mkdir(parents=True)
    data.mkdir()
    fids = [f[0] for f in faces]
    emb = np.array([f[2] for f in faces], dtype=np.float32)
    np.savez(data / "faces.npz", emb=emb, model=np.array("arcface_w600k_r50"), files=np.array(["a.jpg"] * len(faces)),
             fid=np.array(fids), photo=np.array([f[1] for f in faces]),
             box=np.array([[10, 10, 50, 50, 0.9]] * len(faces), dtype=np.float32),
             quality=np.array([0.5] * len(faces), dtype=np.float32))
    pics = [b"\xff\xd8pic-" + f.encode() for f in fids]
    np.savez(data / "thumbs.npz", fid=np.array(fids), thumbs=np.frombuffer(b"".join(pics), dtype=np.uint8),
             thumb_off=np.cumsum([0] + [len(p) for p in pics]))
    photos = {f[1]: {"path": f"/photos/{f[1]}.jpg", "size": 10, "mtime": 1.0, "fp": "ab" * 32, "faces": 1,
                     "name": "a.jpg"} for f in faces}
    (data / "photos.json").write_text(json.dumps(photos))
    (data / "labels.json").write_text(json.dumps(labels))
    (home / "webcam" / "webcam-20261111-120000-000.jpg").write_bytes(b"\xff\xd8snap")
    (home / "webcam" / "webcam-20261111-120000-000.json").write_text(
        json.dumps({"when": "2026-11-11 12:00:00", "faces": []}))
    return data, home


def unit(seed, dim=512):
    v = np.random.default_rng(seed).standard_normal(dim).astype(np.float32)
    return v / np.linalg.norm(v)


@needs_pg
def test_import_merges_devices_and_keeps_hand_labels(pg, tmp_path):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
    import db_import
    a = make_device(tmp_path / "a", [("f1", "p1", unit(1)), ("f2", "p2", unit(2)), ("f3", "p3", np.zeros(512))],
                    {"f1": {"n": "Ann", "by": "auto", "s": 0.5}, "f2": {"n": "Bob", "by": "me"},
                     "f3": {"n": "#ignore", "by": "me"}})
    b = make_device(tmp_path / "b", [("f1", "p1", unit(1)), ("f3", "p3", unit(3)), ("f4", "p4", unit(4))],
                    {"f1": {"n": "Anna", "by": "me"}, "f2": {"n": "Rob", "by": "auto"}})
    da, db = {"id": str(uuid.uuid4()), "name": "A"}, {"id": str(uuid.uuid4()), "name": "B"}
    db_import.run(pg, *a, da)
    again = db_import.run(pg, *a, da)                        # a second run adds nothing twice
    assert again["history lines"] == 0
    db_import.run(pg, *b, db)
    one = lambda sql, *p: pg.execute(sql, p).fetchone()      # noqa: E731
    assert one("SELECT count(*) FROM faces")[0] == 4
    assert one("SELECT n, by FROM labels WHERE fid = 'f1'") == ("Anna", "me")      # by hand beats automatic
    assert one("SELECT n, by FROM labels WHERE fid = 'f2'") == ("Bob", "me")       # the hand label stays
    assert one("SELECT print IS NULL, picture IS NULL FROM faces WHERE fid = 'f3'") == (True, True)  # Ignored
    assert one("SELECT count(*) FROM photo_paths")[0] == 6                         # each device its own paths
    assert one("SELECT count(*) FROM webcam_snapshots")[0] == 1
    assert db_import.check(pg, a[0]) == pytest.approx(1.0, abs=1e-6)
    near = pg.execute("SELECT fid FROM faces WHERE model = 'arcface_w600k_r50' AND print IS NOT NULL "
                      "ORDER BY print::vector(512) <=> %s::vector(512) LIMIT 1", (unit(4),)).fetchone()
    assert near == ("f4",)


@needs_pg
def test_changes_record_who_changed_what_and_deletes(pg):
    pg.execute("INSERT INTO labels (fid, n, by) VALUES ('x1', 'Ann', 'me')")
    pg.execute("UPDATE labels SET n = 'Anna' WHERE fid = 'x1'")
    pg.execute("DELETE FROM labels WHERE fid = 'x1'")
    rows = pg.execute("SELECT tbl, key, op, device::text FROM changes ORDER BY seq").fetchall()
    me = facedb.device()["id"]
    assert [r[:3] for r in rows if r[0] == "labels"] == [("labels", "x1", "U"), ("labels", "x1", "U"),
                                                          ("labels", "x1", "D")]
    assert {r[3] for r in rows if r[0] == "labels"} == {me}


@needs_pg
def test_migrate_twice_is_a_no_op(pg):
    assert facedb.migrate(pg) == [] and facedb.version(pg) == facedb.migrations()[-1][0]
