"""A tripwire on the shared database's synced tables (dbsync.Local, dbsync_home.HomePart): every table in a part's
TABLES has every place a synced table needs, and every table a migration makes is some part's or listed here.

Why: a synced table is written in many places that nothing ties together. A part has no table of functions per
table: digests(), rows(), check(), apply(), push(), fetch() and all_keys() each have a branch per table, most of
them ending in a branch for "whatever is left" (HomePart: the forgotten names; Local: the faces). A table added to
TABLES and forgotten in one of them doesn't fail: it is quietly handled as that last table, and so is never sent,
never taken from the other devices, or never checked. And a table about a person has to be in persondata.py's
export and forget too.

The places, per table (each run on a made-up device, without a database):
  on the device     KEYS; digests() has the device's row; rows() gives it; check() takes it and refuses a row that
                    can't be used; apply() of the device's own row leaves it in step (the digest apply() answers
                    is the one digests() gives); apply() takes a row away
  in the database   push() writes the table and deletes from it (or NEVER_DELETED says why not); fetch() and
                    all_keys() ask it. Seen with a stand-in connection that keeps the statements (Recorder)
  in db/*.sql       a migration makes the table and its change-log trigger, keyed as the part keys it (KEYS)
  about a person    an export of the person's data shows the row, and forgetting them takes it away or changes
                    it (or NOBODYS says why the table holds nothing about anyone)
The other way round: every table the migrations make is in a part's TABLES or in NOT_A_PARTS with who keeps it, and
those that are only ever sent are sent (after_push).

Adding a table: put a row for it in ROW (how a device comes to have one); the test then says what is missing.
What this can't see: the SQL itself running, the merge on join (dbimport.py) and two devices: tests/test_sync.py,
with a real database."""
import contextlib
import json
import re
import time
from pathlib import Path

import alerts
import describe
import devicesettings
import face_id as F
import paths
import test_sync_rows

device = test_sync_rows.device        # the made-up device of tests/test_sync_rows.py: six faces, its own folders

HERE = Path(__file__).resolve().parents[1]
PERSON = "Zelda Quince"
KEY = bytes(range(32))                # stands in for the key made from the owner's passphrase (secretbox.local_key)


# ---------------------------------------------------------------------------------------------- what this test is told
def _photo(d):
    d.db.facts.facts["h0"] = {"t": None, "tags": [PERSON, "tag-mark"], "faces": {0: PERSON}, "key": "k"}
    d.db.facts.save()
    return "h0"


def _label(d):
    d.db.act("name", [0], PERSON)
    return "f0"


def _web(d):
    d.local.web["f0"] = {"when": "2026-11-01T10:00:00Z", "suggestions": [{"name": PERSON, "links": []}],
                         "checked": ["web-mark"], "details": {}}
    return "f0"


def _description(d):
    describe.save("h0", {"text": f"{PERSON} on a beach (description-mark)"})
    return "h0"


def _snapshot(d):
    stem = "webcam-20261101-100000-000-0"
    F.WEBCAM.mkdir(parents=True, exist_ok=True)
    said = {"when": "2026-11-01T10:00:00Z", "faces": [{"box": [1, 2, 3, 4], "name": PERSON, "sim": 0.6}]}
    (F.WEBCAM / f"{stem}.json").write_text(json.dumps(said), "utf-8")
    (F.WEBCAM / f"{stem}.jpg").write_bytes(b"\xff\xd8 a collected face")
    return stem


def _alert(d):
    return alerts.add({"kind": "arrived", "name": PERSON, "text": f"{PERSON} arrived (alert-mark)",
                       "t": int(time.time() * 1000)})["id"]


def _visit(d):
    F.PRESENCE.mkdir(parents=True, exist_ok=True)
    visit = {"name": PERSON, "start": 1_762_000_000_123, "end": 1_762_000_060_000}
    (F.PRESENCE / "2026-11-01.jsonl").write_text(json.dumps(visit) + "\n", "utf-8")
    return f"{d.me}:1762000000123:{PERSON}"


def _document(d):
    F.save_json("va_fit.json", {"features": ["smile"], "models": {PERSON: {"w": [0.123456]}, "*": {"w": [2]}}})
    return "va_fit"


def _secret(d):
    import websearch
    websearch.save_key("G" * 39)
    return "google_vision"


def _forgotten(d):
    import forgotten
    forgotten.note("Somebody Else")
    return "Somebody Else"


def _setting(d):
    devicesettings.update("track", {"ft-alertWho": PERSON})
    return f"{d.me}|track|ft-alertWho"


def _signal(d):
    devicesettings.put_signal("usual", PERSON, {"mark": "signal-mark"})
    return f"{PERSON}|usual"


# How a device comes to have a row of each synced table, here a row about PERSON: through the programs' own modules
# and files, as they make it. Returns the row's key. (In this order: the label makes face f0 PERSON's.)
ROW = {"labels": _label, "faces": lambda d: "f0", "photos": _photo, "web_suggestions": _web,
       "descriptions": _description, "webcam_snapshots": _snapshot, "alerts": _alert, "presence": _visit,
       "documents": _document, "secrets": _secret, "forgotten": _forgotten, "device_settings": _setting,
       "person_signals": _signal}

# What an export of PERSON's data (persondata.export: data.json and the files beside it) shows of that row.
SHOWN = {"labels": '"faceId": "f0"', "faces": "faces/f0.jpg", "photos": "tag-mark", "web_suggestions": "web-mark",
         "descriptions": "description-mark", "webcam_snapshots": "webcam-20261101-100000-000-0.jpg",
         "alerts": "alert-mark", "presence": "1762000000123", "documents": "0.123456",
         "device_settings": '"inAlertList": true', "person_signals": "signal-mark"}
# ... or why the table holds nothing about anyone.
NOBODYS = {
    "secrets": "the owner's keys for Web search and Describe photo",
    "forgotten": "the forgotten name itself, the one thing kept: the instruction for devices that haven't forgotten yet",
}

# Tables whose rows check() has nothing to refuse in: why.
NOTHING_TO_CHECK = {
    "secrets": "every column is typed (name text, nonce and ciphertext bytea): no row of another shape can come "
               "out of the database, and apply() leaves a key it can't read as it is",
}
# Tables push() never deletes from: why (as the code says, or does).
NEVER_DELETED = {
    "photos": "photos are never deleted from a device (dbsync.push)",
    "faces": "faces are never deleted from a device: an Ignored face keeps its row, without print or picture",
    "documents": "a fitted or measured file that is gone here stays in the database (no delete is written)",
}

# Tables the migrations make that no part syncs: who keeps them.
NOT_A_PARTS = {
    "schema_version": "which migrations have run (facedb.migrate)",
    "instance": "the database's own id, which a device joins by (dbsync.Sync.step)",
    "server_ca": "the server's CA certificate (facedb.setup)",
    "devices": "the devices themselves (facedb.register; peer.py, camstream.py and secretbox.py keep their columns)",
    "changes": "the change log, written by the triggers and read by the pull (dbsync.Sync._pull)",
    "photo_paths": "each device's own file paths: written with its photos (dbsync.push), asked by peer.py",
    "usage": "Web search use counts, counted in the database itself (dbsync_home.count_use)",
    "sightings": "who each camera sees, kept 20 minutes for the moved alert (HomePart._cameras)",
    "passphrase_versions": "the passphrase for keys (secretbox.py reads and writes it itself)",
    "passphrase_shares": "the passphrase for keys, sealed to devices (secretbox.py)",
    "key_requests": "a device asking for the passphrase (secretbox.py)",
    "label_history": "only sent: the undo history's new lines (Local.after_push)",
    "app_log": "only sent: the app log's new lines (HomePart.after_push)",
    "expr_eval": "only sent: posed tests and feeling reports (HomePart.after_push)",
    "expression_log": "only sent: expression readings (exprlog.send, from HomePart.after_push)",
    "music_log": "only sent: songs (musiclog.send, from HomePart.after_push)",
}

JUNK = (7, "text", [1, 2])                         # rows no table can use
BAD_KEYS = ("#x", "k" * 300, "one part")           # keys a table with a rule for its keys refuses (not "": all do)


# ---------------------------------------------------------------------------------------------- helpers
class Recorder:
    """Stands in for the database's connection: keeps every statement it is given (as one line) and answers each
    with no rows. So push(), fetch(), all_keys() and after_push() run without a database, and what they would
    write or ask can be read."""

    def __init__(self):
        self.said = []

    def cursor(self):
        return self

    def transaction(self):
        return contextlib.nullcontext()

    def execute(self, sql, params=None):
        self.said.append(" ".join(str(sql).split()))
        return self

    def executemany(self, sql, rows):
        if list(rows):                             # nothing to write: no statement
            self.said.append(" ".join(str(sql).split()))

    def fetchone(self):
        return None

    def fetchall(self):
        return []

    def __iter__(self):
        return iter(())

    def does(self, what, tbl):
        """True if a statement was `what` ("INSERT INTO", "DELETE FROM", "SELECT") of table tbl."""
        if what == "SELECT":
            return any(re.search(rf"\bSELECT\b.*\bFROM\s+{tbl}\b", s, re.I) for s in self.said)
        return any(re.search(rf"\b{what}\s+{tbl}\b", s, re.I) for s in self.said)


def ready(d, monkeypatch, parts=None):
    """The device with a passphrase for keys (so its keys sync) and a row in every table ROW knows.
    Returns (the parts, {table: its row's key}, [tables without a row in ROW])."""
    import secretbox
    monkeypatch.setattr(secretbox, "local_key", lambda: KEY)
    monkeypatch.setattr(secretbox, "current", lambda conn: {"id": 1})
    monkeypatch.setattr(secretbox, "on_current", lambda cur: True)
    parts = parts or [d.local, d.home]
    for part in parts:
        part.key_current = True
    keys = {tbl: make(d) for tbl, make in ROW.items()}
    return parts, keys, [f"{tbl}: no row for it in ROW in tests/test_sync_tables.py (how a device comes to have one)"
                         for part in parts for tbl in part.TABLES if tbl not in ROW]


def as_the_database_gives(tbl, key, row):
    """The device's own row (rows()) as fetch() would hand it back. They are the same for every table but the
    keys: sent encrypted (HomePart.push). A photo's row is left as rows() makes it, which apply() reads the same
    fields from."""
    if tbl == "secrets":
        import secretbox
        return dict(zip(("nonce", "ciphertext"), secretbox.encrypt(KEY, key, row)))
    return row


def tried(what, do, *args):
    """(do(*args), None), or (None, the sentence that says `what` failed)."""
    try:
        return do(*args), None
    except Exception as e:                         # whatever a missing branch raises is what the test reports
        return None, f"{what} fails ({type(e).__name__}: {str(e)[:100]})"


def refuses(part, tbl, key, row):
    import dbsync
    try:
        part.check(tbl, key, row)
    except dbsync.BAD_ROW:
        return True
    return False


# ---------------------------------------------------------------------------------------------- the places
def on_the_device(d, parts, keys):
    """What is missing of each table's places on the device: [sentence]."""
    missing = []
    said = lambda part, tbl, what: missing.append(f"{tbl}: {type(part).__name__}.{what}")
    for part in parts:
        digests, why = tried("digests()", part.digests)
        if why:
            missing.append(f"{type(part).__name__}.{why}")
        for tbl in (t for t in part.TABLES if t in keys):
            key = keys[tbl]
            if tbl not in getattr(part, "KEYS", {}):
                said(part, tbl, "KEYS has no key column for it")
            if key not in ((digests or {}).get(tbl) or {}):
                said(part, tbl, "digests() doesn't have the device's row")
            got, why = tried("rows()", part.rows, tbl, [key])
            if why or key not in got:
                said(part, tbl, why or "rows() doesn't give the device's row")
            row = as_the_database_gives(tbl, key, got[key]) if got and key in got else None
            if row is not None:
                _, why = tried("check() of a row the device made itself", part.check, tbl, key, row)
                if why:
                    said(part, tbl, why)
            strict = any(refuses(part, tbl, key, junk) for junk in JUNK) or \
                any(refuses(part, tbl, k, {} if row is None else row) for k in BAD_KEYS)
            if not strict and tbl not in NOTHING_TO_CHECK:
                said(part, tbl, "check() refuses no row of it, whatever the row is: say what a row must look like "
                                "(or list the table in NOTHING_TO_CHECK with why)")
            if strict and tbl in NOTHING_TO_CHECK:
                said(part, tbl, "check() does refuse rows of it now: take it out of NOTHING_TO_CHECK")
            # (without a row from rows(), an empty one: the places after it are still looked at)
            out, why = tried("apply() of the device's own row", part.apply, tbl, {key: {} if row is None else row},
                             lambda *a: None)
            now, _ = tried("digests()", part.digests)
            if why:
                said(part, tbl, why)
            elif not out or out.get(key) is None:
                said(part, tbl, "apply() answers no digest for the row it was given (no branch for the table?)")
            elif out[key] != ((now or {}).get(tbl) or {}).get(key):
                said(part, tbl, "apply() of the device's own row answers another digest than digests() gives: "
                                "the row would be sent again at once")
    for part in parts:                             # a row taken away (deleted on another device): last, it changes things
        for tbl in (t for t in part.TABLES if t in keys):
            before = len(d.said)
            _, why = tried("apply() of a row that was deleted", d.sync._apply, part, tbl, {keys[tbl]: None})
            if why or any("left out" in s for s in d.said[before:]):
                said(part, tbl, why or f"apply() can't take a deleted row ({d.said[-1][-90:]})")
    return missing


def in_the_database(parts, keys):
    """What is missing of each table's places in the database: [sentence]."""
    missing = []
    said = lambda part, tbl, what: missing.append(f"{tbl}: {type(part).__name__}.{what}")
    for part in parts:
        for tbl in (t for t in part.TABLES if t in keys):
            key = keys[tbl]
            rows, _ = tried("rows()", part.rows, tbl, [key])
            conn = Recorder()
            _, why = tried("push()", part.push, conn, tbl, rows or {key: {}}, [])
            if why or not conn.does("INSERT INTO", tbl):
                said(part, tbl, why or "push() writes nothing to the table")
            conn = Recorder()
            _, why = tried("push() of a deleted row", part.push, conn, tbl, {}, [key])
            deletes = conn.does("DELETE FROM", tbl)
            if why or (not deletes and tbl not in NEVER_DELETED):
                said(part, tbl, why or "push() doesn't delete a row that is gone here (or list the table in "
                                       "NEVER_DELETED with why)")
            if deletes and tbl in NEVER_DELETED:
                said(part, tbl, "push() does delete from it now: take it out of NEVER_DELETED")
            conn = Recorder()
            got, why = tried("fetch()", part.fetch, conn, tbl, [key])
            if why or not conn.does("SELECT", tbl):
                said(part, tbl, why or "fetch() doesn't ask the table for the row")
            elif got != {key: None}:
                said(part, tbl, f"fetch() of a row the database doesn't have answers {got!r}, not that it is gone")
            conn = Recorder()
            _, why = tried("all_keys()", part.all_keys, conn, tbl)
            if why or not conn.does("SELECT", tbl):
                said(part, tbl, why or "all_keys() doesn't ask the table")
    return missing


def migrations():
    """({table: the file that makes it}, {table: the key columns its change-log trigger is given}) from db/*.sql.
    Read as text, because the SQL can't be run here: there is no PostgreSQL in this test (tests/test_sync.py runs
    the migrations for real)."""
    made, logged = {}, {}
    for f in sorted((HERE / "db").glob("*.sql")):
        sql = re.sub(r"--[^\n]*", "", f.read_text("utf-8"))
        for what, tbl in re.findall(r"\b(CREATE|DROP) TABLE (?:IF (?:NOT )?EXISTS )?(\w+)", sql, re.I):
            if what.upper() == "CREATE":
                made[tbl] = f.name
            else:
                made.pop(tbl, None)
                logged.pop(tbl, None)
        # a trigger written out: CREATE TRIGGER x_change AFTER INSERT OR UPDATE OR DELETE ON x ... note_change('key')
        for tbl, cols in re.findall(r"CREATE TRIGGER \w+ AFTER [\w ]+? ON (\w+)[^;]*?note_change\(([^)]*)\)", sql, re.I | re.S):
            logged[tbl] = ",".join(re.findall(r"'(\w+)'", cols))
        # db/001 makes its triggers in a loop over ('table', 'key,columns') pairs
        for pairs in re.findall(r"VALUES(.*?)AS v\(tbl, keys\)", sql, re.S):
            logged.update({tbl: cols for tbl, cols in re.findall(r"\('(\w+)',\s*'([\w,]+)'\)", pairs) if tbl in made})
    return made, logged


def in_the_migrations(parts):
    """What is missing in db/*.sql for the parts' tables, and tables made there that are nobody's: [sentence]."""
    made, logged = migrations()
    missing = []
    owners = {}
    for part in parts:
        for tbl in part.TABLES:
            owners.setdefault(tbl, []).append(type(part).__name__)
            if tbl not in made:
                missing.append(f"{tbl}: no db/NNN_*.sql makes the table (CREATE TABLE {tbl})")
            if tbl not in logged:
                missing.append(f"{tbl}: no migration gives it the change-log trigger (note_change): the other "
                               f"devices would never hear of a change")
            elif logged[tbl] != getattr(part, "KEYS", {}).get(tbl):
                missing.append(f"{tbl}: its change-log trigger is keyed by {logged[tbl]!r}, {type(part).__name__}.KEYS "
                               f"says {getattr(part, 'KEYS', {}).get(tbl)!r}: the pull would ask for keys that "
                               f"aren't its rows'")
    missing += [f"{tbl}: in the TABLES of {' and '.join(who)}" for tbl, who in owners.items() if len(who) > 1]
    missing += [f"{tbl} ({made[tbl]}): no part syncs this table: add it to a part's TABLES (the test then says what "
                f"else it needs), or to NOT_A_PARTS in tests/test_sync_tables.py with who keeps it"
                for tbl in sorted(set(made) - set(owners) - set(NOT_A_PARTS))]
    missing += [f"{tbl}: listed in NOT_A_PARTS, but {'a part syncs it' if tbl in owners else 'no migration makes it'}"
                for tbl in NOT_A_PARTS if tbl in owners or tbl not in made]
    return missing


def about_a_person(d, parts, keys, folder):
    """What is missing for the tables that hold something about a person: [sentence]. Exports PERSON's data into
    folder, then forgets them."""
    import persondata
    missing = [f"{tbl}: is it about a person? Say what an export shows of its row (SHOWN), or why it holds nothing "
               f"about anyone (NOBODYS), in tests/test_sync_tables.py" for part in parts for tbl in part.TABLES
               if tbl in keys and tbl not in SHOWN and tbl not in NOBODYS]
    out = Path(persondata.export(d.db, PERSON, str(folder), d.local.web)["folder"])
    text = (out / "data.json").read_text("utf-8") + "\n".join(p.relative_to(out).as_posix() for p in out.rglob("*"))
    before = {tbl: part.digests()[tbl] for part in parts for tbl in part.TABLES if tbl in SHOWN and tbl in keys}
    missing += [f"{tbl}: persondata.export doesn't show the person's row ({SHOWN[tbl]!r} isn't in the export)"
                for tbl in before if SHOWN[tbl] not in text]
    persondata.forget(d.db, PERSON, d.local.web)
    after = {tbl: part.digests()[tbl] for part in parts for tbl in part.TABLES if tbl in before}
    missing += [f"{tbl}: persondata.forget leaves the person's row as it was" for tbl in before
                if keys[tbl] in before[tbl] and after[tbl].get(keys[tbl]) == before[tbl][keys[tbl]]]
    return missing


def listed(missing):
    return "a synced table is missing a place:\n  " + "\n  ".join(missing)


# ---------------------------------------------------------------------------------------------- the tripwires
def test_every_synced_table_has_its_places_on_this_device(device, monkeypatch):
    """For every table of both parts: its key column is known, the device's row is in digests() and rows(),
    check() takes it and refuses a row that can't be used, and apply() of the device's own row leaves it in step
    and can take a row away. A table added to TABLES and left out of one of these is handled as another table
    (the last branch), or not at all, and nothing says so."""
    parts, keys, unknown = ready(device, monkeypatch)
    assert not unknown, listed(unknown)
    missing = on_the_device(device, parts, keys)
    assert not missing, listed(missing)
    assert set(keys) == {t for part in parts for t in part.TABLES}, "ROW has rows for tables no part syncs"


def test_every_synced_table_is_written_to_and_asked_from_the_database(device, monkeypatch):
    """push(), fetch() and all_keys() for every table, with a stand-in connection that keeps the statements: the
    table is written, deleted from (or NEVER_DELETED says why not) and asked. Without its branch a table's rows
    are marked as sent and never leave the device, or the forgotten names are asked in its place."""
    parts, keys, unknown = ready(device, monkeypatch)
    assert not unknown, listed(unknown)
    missing = in_the_database(parts, keys)
    assert not missing, listed(missing)


def test_every_synced_table_is_made_by_a_migration_and_every_table_made_is_somebodys(device):
    """db/*.sql: each synced table is made there, with the change-log trigger the pull depends on, keyed as the
    part keys its rows. And the other way round: a table a migration makes belongs to a part, or NOT_A_PARTS says
    who keeps it (a new place that keeps data needs a table and a part)."""
    missing = in_the_migrations([device.local, device.home])
    assert not missing, listed(missing)


def test_what_is_only_sent_is_sent(device, monkeypatch):
    """The tables NOT_A_PARTS calls "only sent" (the undo history, the app log, posed tests, the expression log
    and the music log) are written by after_push() from this device's files."""
    import exprlog
    import musiclog
    d = device
    ready(d, monkeypatch)
    now, today = int(time.time() * 1000), paths.utc_now()[:10]
    # a line waiting to be sent, for each: {table: (its file, the line)}
    waiting = {
        "label_history": (F.DATA / "history.jsonl", {"t": paths.utc_now(), "what": "Named 1 face", "faces": {}}),
        "app_log": (paths.HOME / "app.log", "2026-11-01T10:00:00Z Face tracker: serving"),
        "expr_eval": (F.DATA / "expr_eval" / "20261101-120000.jsonl", {"name": PERSON, "posed": "happy"}),
        "expression_log": (exprlog.folder() / f"{today}.jsonl", {"t": now, "name": PERSON, "label": "happy",
                                                                 "share": 0.8, "state": "plain", "data": {}}),
        "music_log": (musiclog.folder() / f"{today}.jsonl", {"t": now, "name": PERSON, "artist": "An Artist",
                                                             "song": "A Song", "seconds": 200, "data": {}}),
    }
    only_sent = {tbl for tbl, why in NOT_A_PARTS.items() if why.startswith("only sent")}
    assert set(waiting) == only_sent, "give each only-sent table a waiting line here, so this test sees it sent"
    for file, line in waiting.values():
        file.parent.mkdir(parents=True, exist_ok=True)
        with open(file, "a", encoding="utf-8") as f:
            f.write((line if isinstance(line, str) else json.dumps(line)) + "\n")
    conn = Recorder()
    for part in (d.local, d.home):
        part.after_push(conn, d.sync.state)
    unsent = sorted(tbl for tbl in only_sent if not conn.does("INSERT INTO", tbl))
    assert not unsent, f"after_push() writes nothing to {unsent}, which NOT_A_PARTS says are only sent"


def test_a_synced_table_about_a_person_is_exported_and_forgotten_with_them(device, monkeypatch, tmp_path):
    """Every person's data can be exported and forgotten (persondata.py). For each synced table that holds
    something about a person, the export of that person shows their row, and after a forget the row is gone or
    changed (which the sync then sends to the database and the other devices). A new table has to say which it
    is: about a person (SHOWN), or not (NOBODYS)."""
    parts, keys, unknown = ready(device, monkeypatch)
    assert not unknown, listed(unknown)
    missing = about_a_person(device, parts, keys, tmp_path / "export")
    assert not missing, listed(missing)
    assert not set(SHOWN) & set(NOBODYS) and set(SHOWN) | set(NOBODYS) == set(keys)


# ---------------------------------------------------------------------------------------------- the tripwire, tripped
def test_a_new_table_with_none_of_its_places_is_noticed_place_by_place(device, monkeypatch, tmp_path):
    """The tripwire itself: a table added to a part's TABLES and to nothing else (as a subclass here, the code
    isn't touched) is named with every place it is missing, and the real tables beside it with none."""
    import dbsync
    import dbsync_home
    d = device

    class HomeAndPets(dbsync_home.HomePart):
        TABLES = dbsync_home.HomePart.TABLES + ("pets",)

    class LocalAndPets(dbsync.Local):
        TABLES = dbsync.Local.TABLES + ("pets",)
    home, local = HomeAndPets(), LocalAndPets(d.db, d.local.web, lambda: None)

    _, _, unknown = ready(d, monkeypatch, [d.local, home])
    assert unknown == ["pets: no row for it in ROW in tests/test_sync_tables.py (how a device comes to have one)"]
    monkeypatch.setitem(ROW, "pets", lambda d: "rex")           # the developer's next step: a row, and nothing else yet
    parts, keys, unknown = ready(d, monkeypatch, [d.local, home])
    assert not unknown and keys["pets"] == "rex"

    def places(missing):
        """The places said to be missing, which must all be the new table's: [part.place], without the details."""
        assert all(m.startswith("pets: ") for m in missing), missing
        return [re.split(r" \(|: say|\?", m[len("pets: "):])[0] for m in missing]

    # (HomePart refuses a table it has no class for, in every one of its places: none goes on to another table's)
    assert places(in_the_database(parts, keys)) == [
        "HomeAndPets.push() fails", "HomeAndPets.push() of a deleted row fails",
        "HomeAndPets.fetch() fails", "HomeAndPets.all_keys() fails"]
    assert places(in_the_database([local], {"pets": "rex"})) == [
        "LocalAndPets.push() fails", "LocalAndPets.push() of a deleted row fails",
        "LocalAndPets.fetch() fails", "LocalAndPets.all_keys() fails"]
    assert places(in_the_migrations(parts)) == ["no db/NNN_*.sql makes the table",
                                                "no migration gives it the change-log trigger"]
    assert "pets: in the TABLES of LocalAndPets and HomeAndPets" in in_the_migrations([local, home])
    assert places(about_a_person(d, parts, keys, tmp_path / "export")) == ["is it about a person"]
    # (these two last: they take rows away, and the forget above took the person's)
    assert places(on_the_device(d, [local], {"pets": "rex"})) == [
        "LocalAndPets.KEYS has no key column for it", "LocalAndPets.digests() doesn't have the device's row",
        "LocalAndPets.rows() fails", "LocalAndPets.apply() of the device's own row fails",
        "LocalAndPets.apply() can't take a deleted row"]
    assert places(on_the_device(d, [home], {"pets": "rex"})) == [
        "HomeAndPets.KEYS has no key column for it", "HomeAndPets.digests() doesn't have the device's row",
        "HomeAndPets.rows() fails",
        # (apply() refuses a table it has no class for: it once took such a table's keys for people to forget)
        "HomeAndPets.apply() of the device's own row fails", "HomeAndPets.apply() can't take a deleted row"]


def test_one_place_taken_out_of_a_real_table_is_noticed(device, monkeypatch):
    """The tripwire itself, on the real tables: a place that has no branch for one table (here: the table handed
    on under a name the part doesn't know, which is what a forgotten branch comes to) is named, and only that."""
    import dbsync_home
    Home = dbsync_home.HomePart
    parts, keys, _ = ready(device, monkeypatch)
    assert not in_the_database(parts, keys)                     # as the code is: nothing missing

    def without(method, tbl):
        """Home.method as if it had no branch for tbl. Returns the real one."""
        real = getattr(Home, method)
        monkeypatch.setattr(Home, method, lambda self, *a: real(
            self, *("pets" if isinstance(x, str) and x == tbl else x for x in a)))
        return real

    real = without("push", "alerts")
    no_place = "(ValueError: the sync has no place to put rows of 'pets')"
    assert in_the_database(parts, keys) == [f"alerts: HomePart.push() fails {no_place}",
                                            f"alerts: HomePart.push() of a deleted row fails {no_place}"]
    monkeypatch.setattr(Home, "push", real)
    real = without("fetch", "presence")
    assert in_the_database(parts, keys) == [f"presence: HomePart.fetch() fails {no_place}"]
    monkeypatch.setattr(Home, "fetch", real)
    assert not in_the_database(parts, keys)

    without("check", "descriptions")
    without("apply", "alerts")
    without("rows", "presence")
    got = on_the_device(device, parts, keys)
    assert [g for g in got if not g.startswith("alerts: ")] == [
        f"descriptions: HomePart.check() of a row the device made itself fails {no_place}",
        f"presence: HomePart.rows() fails {no_place}"]
    # apply() without a branch for the table refuses its rows (it once took their keys for people to forget):
    # the device's own row and a deleted one are both named
    alerts = [g for g in got if g.startswith("alerts: ")]
    assert len(alerts) == 2 and all("HomePart.apply() " in g and "no place to put rows of 'pets'" in g for g in alerts)
