"""Rows that wait on a device until the shared database's sync has sent them (waitlog.py): what the expression log
and the music log share. Where a row waits, what a send takes and leaves, and when a waiting file goes. No database
is needed: a stand-in takes what send() gives it. The two logs' own tests go through the same class (test_servers.py,
test_music.py, test_persondata.py; with a database test_sync.py and test_facedb.py)."""
import contextlib
import json
import os
import time
from datetime import datetime, timedelta, timezone

import pytest

import waitlog

DEVICE = "5f0c1d1e-0000-4000-8000-000000000001"


class Away(Exception):
    """The shared database can't be reached."""


class Database:
    """A stand-in for the connection to the shared database. `table` is the log's table by its key (device, name,
    at): a row sent again changes nothing there, as ON CONFLICT (device, name, at) DO NOTHING has it. `asked` is
    every statement with its rows. While `away`, it takes nothing."""

    def __init__(self):
        self.table, self.asked, self.away = {}, [], False

    @contextlib.contextmanager
    def transaction(self):
        before = dict(self.table)
        try:
            yield
        except BaseException:
            self.table = before                          # rolled back
            raise

    def cursor(self):
        return self

    def executemany(self, sql, rows):
        if self.away:
            raise Away()
        assert sql.endswith("ON CONFLICT (device, name, at) DO NOTHING")
        self.asked.append((sql, list(rows)))
        for r in rows:
            self.table.setdefault(r[:3], r[3:])

    def counts(self):
        return [len(rows) for _, rows in self.asked]


class State(dict):
    """The sync's state as send() uses it (dbsync.State keeps it in HOME/sync.sqlite, every value as text)."""

    def put(self, k, v):
        self[k] = str(v)

    def drop(self, k):
        self.pop(k, None)


@pytest.fixture
def log(tmp_path, monkeypatch):
    """A made-up third log in an empty private folder, set up as the next log of this kind would be: steps taken
    in a place. A line without a place isn't a row of it."""
    import paths
    monkeypatch.setattr(paths, "HOME", tmp_path)
    return waitlog.WaitLog("steps", "step_log", ("place", "steps", "data"),
                           lambda o: (o["place"], o.get("steps"), json.dumps(o.get("data") or {})),
                           key="slog", needs=("place",))


def row(name, t, **more):
    return {"t": t, "name": name, "place": "garden", "steps": 120, "data": {"pace": 1.5}, **more}


def when(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc)


def noon(days_ago):
    """Noon (UTC) that many days ago, as epoch ms, and the name of that day's waiting file."""
    d = (datetime.now(timezone.utc) - timedelta(days=days_ago)).replace(hour=12, minute=0, second=0, microsecond=0)
    return int(d.timestamp() * 1000), f"{d:%Y-%m-%d}.jsonl"


def quiet(f):
    """As if nothing had been added to the file for five minutes."""
    os.utime(f, (time.time() - 300, time.time() - 300))


def test_a_row_waits_as_one_line_in_the_file_of_its_own_utc_day(log, tmp_path):
    """A row waits in HOME/<folder>/<UTC day of its own time>.jsonl as one line of JSON, names written as they are,
    added at the file's end. Files like this wait on the owner's devices from the versions before: with another
    file name or another line they would not be found or not be read."""
    last, midnight = 1_700_006_399_000, 1_700_006_400_000          # 2023-11-14 23:59:59 and 2023-11-15 00:00:00 UTC
    assert log.add([row("Zoë", last), row("Bo", midnight)]) == 2 and log.add([row("Ann", last + 500)]) == 1
    assert log.add([]) == 0
    assert log.folder() == tmp_path / "steps"
    assert sorted(f.name for f in log.folder().iterdir()) == ["2023-11-14.jsonl", "2023-11-15.jsonl"]
    assert (log.folder() / "2023-11-14.jsonl").read_bytes() == (        # (a text file: each system's own line end)
        '{"t": 1700006399000, "name": "Zoë", "place": "garden", "steps": 120, "data": {"pace": 1.5}}' + os.linesep
        + '{"t": 1700006399500, "name": "Ann", "place": "garden", "steps": 120, "data": {"pace": 1.5}}' + os.linesep
    ).encode("utf-8")
    assert [r["name"] for r in log.waiting()] == ["Zoë", "Ann", "Bo"] and log.waiting("Bo") == [row("Bo", midnight)]
    assert log.waiting("Nobody") == [] and log.days() == {"2023-11-14": 2, "2023-11-15": 1}
    with open(log.folder() / "2023-11-15.jsonl", "ab") as f:        # lines that are no rows of this log don't count
        f.write(b'not json\n["a", "list"]\n{"t": 1700006400001, "name": "Bo"}\n{"t": 1700006400002, "place": "garden"}\n')
    assert len(log.waiting()) == 3 and log.days()["2023-11-15"] == 1


def test_waiting_rows_are_sent_once_and_sending_again_changes_nothing(log):
    """A send puts each waiting line into the table and keeps in the sync's state how far into each file it got,
    so the next one takes only what was added since. And a file sent again from its start (its mark lost, or the
    file rewritten) changes nothing: the table has one row per device, name and time."""
    now = int(time.time() * 1000)
    log.add([row("Ann", now), row("Bo", now + 1, steps=None, data=None)])
    db, state = Database(), State()
    assert log.send(db, state, DEVICE) == 2
    assert db.table == {(DEVICE, "Ann", when(now)): ("garden", 120, '{"pace": 1.5}'),
                        (DEVICE, "Bo", when(now + 1)): ("garden", None, "{}")}
    assert db.asked[0][0] == ("INSERT INTO step_log (device, name, at, place, steps, data) VALUES (%s, %s, %s, %s, %s, %s) "
                              "ON CONFLICT (device, name, at) DO NOTHING")
    (f,) = log.folder().iterdir()
    assert state == {f"slog:{f.name}": str(f.stat().st_size)}
    assert log.send(db, state, DEVICE) == 0 and db.counts() == [2]                 # nothing new: nothing is asked
    log.add([row("Ann", now + 2)])
    assert log.send(db, state, DEVICE) == 1 and db.counts() == [2, 1] and len(db.table) == 3      # only the new line
    state.clear()
    assert log.send(db, state, DEVICE) == 3 and db.counts() == [2, 1, 3] and len(db.table) == 3   # all again: the same
    assert len(log.waiting()) == 3 and f.exists()                    # today's file stays: today's rows, sent or not


def test_rows_the_database_did_not_take_wait_for_the_next_send(log):
    """The database away in the middle of a send: its error goes on to the sync, which tries again later, and
    nothing counts as sent. The files and the state are as they were, and the next send takes every row. Counted
    as sent, the rows would be lost for good: a past day's file is deleted once all of it counts as sent."""
    (y, yfile), now = noon(1), int(time.time() * 1000)
    log.add([row("Ann", y), row("Bo", y + 1), row("Ann", now)])
    quiet(log.folder() / yfile)
    before = {f.name: f.read_bytes() for f in log.folder().iterdir()}
    db, state = Database(), State()
    db.away = True
    with pytest.raises(Away):
        log.send(db, state, DEVICE)
    assert state == {} and db.table == {} and len(log.waiting()) == 3
    assert {f.name: f.read_bytes() for f in log.folder().iterdir()} == before and len(before) == 2
    db.away = False
    assert log.send(db, state, DEVICE) == 3 and len(db.table) == 3
    assert not (log.folder() / yfile).exists()                       # yesterday's, now that the database has it


def test_a_past_days_file_goes_once_all_of_it_is_sent_and_nothing_was_added_for_a_minute(log):
    """The database is where the rows are kept, so a past day's waiting file is deleted, with its mark in the
    state, once the whole of it is sent. Never today's, where rows are still added, and not one written to within
    the last minute: a row of just before midnight may still be on its way."""
    (y, yfile), (b, bfile), now = noon(1), noon(2), int(time.time() * 1000)
    log.add([row("Ann", y), row("Bo", y + 1), row("Ann", b), row("Ann", now)])
    folder = log.folder()
    (today,) = (f for f in folder.iterdir() if f.name not in (yfile, bfile))
    quiet(folder / yfile)
    quiet(today)                                                     # (the day before's was written to this moment)
    db, state = Database(), State()
    assert log.send(db, state, DEVICE) == 4
    assert not (folder / yfile).exists() and f"slog:{yfile}" not in state
    assert (folder / bfile).exists() and state[f"slog:{bfile}"] == str((folder / bfile).stat().st_size)
    assert today.exists()                                            # however long nothing was added to it
    quiet(folder / bfile)
    assert log.send(db, state, DEVICE) == 0 and not (folder / bfile).exists()
    assert list(state) == [f"slog:{today.name}"] and len(db.table) == 4 and len(log.waiting()) == 1


def test_only_whole_lines_are_sent_and_a_line_that_is_no_row_is_passed_over(log):
    """The sync may read a file while a row is being added to it: an unfinished last line waits for its end (the
    mark stands after the last whole line), and its file is not deleted. A line that is no row of this log
    (damaged, without a field the table needs, a time no clock has) is passed over and doesn't stop the rows
    after it."""
    y, yfile = noon(1)
    f = log.folder() / yfile
    f.parent.mkdir()
    whole = json.dumps(row("Ann", y)) + "\n"
    no_rows = "".join(ln + "\n" for ln in ("not json", json.dumps(["a", "list"]), json.dumps({"t": y + 1, "name": "Bo"}),
                                           json.dumps(row("Bo", 10 ** 20)), json.dumps(row("Bo", "soon"))))
    last = json.dumps(row("Bo", y + 2)) + "\n"
    f.write_bytes((whole + no_rows + last[:30]).encode("utf-8"))
    quiet(f)
    db, state = Database(), State()
    assert log.send(db, state, DEVICE) == 1 and list(db.table) == [(DEVICE, "Ann", when(y))]
    assert state == {f"slog:{yfile}": str(len(whole + no_rows))} and f.exists()
    with open(f, "ab") as fh:
        fh.write(last[30:].encode("utf-8"))
    quiet(f)
    assert log.send(db, state, DEVICE) == 1 and list(db.table)[1] == (DEVICE, "Bo", when(y + 2))
    assert not f.exists() and state == {}


def test_a_forgotten_persons_waiting_rows_go_and_what_is_left_is_sent_again(log):
    """Forgetting a person rewrites the waiting files without their rows. A rewritten file is shorter than what the
    state says was sent of it, so it is sent again from its start, which changes nothing in the database, and a row
    added after the forget is sent with it."""
    now = int(time.time() * 1000)
    log.add([row("Ann", now), row("Bo", now + 1), row("Ann", now + 2)])
    db, state = Database(), State()
    assert log.send(db, state, DEVICE) == 3
    assert log.forget("Ann") == 2 and log.forget("Ann") == 0 and log.forget("Nobody") == 0
    assert log.waiting() == [row("Bo", now + 1)]
    log.add([row("Bo", now + 3)])
    assert log.send(db, state, DEVICE) == 2 and db.counts() == [3, 2]
    assert len(db.table) == 4 and (DEVICE, "Bo", when(now + 3)) in db.table


def test_rows_added_after_a_forget_are_sent_even_when_the_file_is_as_long_as_before(log):
    """Through 1.24.4 a rewritten file was told only by being shorter than what had been sent of it. A person with
    few rows forgotten, others in view: the file was as long again by the next send, the send began in the middle
    of the new lines, sent none of them, and the file was later deleted as all sent. The rows of everyone else
    since the forget were lost. Now forget() says the file was rewritten, and it is sent from its start."""
    now = int(time.time() * 1000)
    log.add([row("Bo", now), row("Ann", now + 1), row("Ann", now + 2)])
    db, state = Database(), State()
    assert log.send(db, state, DEVICE) == 3
    assert log.forget("Ann") == 2
    log.add([row("Bo", now + 3), row("Cy", now + 4), row("Bo", now + 5)])     # longer than what was sent before
    assert log.send(db, state, DEVICE) == 4                                    # Bo's first again, and the three new
    assert {k[1:] for k in db.table} >= {("Bo", when(now + 3)), ("Cy", when(now + 4)), ("Bo", when(now + 5))}
    assert not list(log.folder().glob("*.again"))                              # dealt with: the next send goes on
    log.add([row("Cy", now + 6)])
    assert log.send(db, state, DEVICE) == 1 and db.counts() == [3, 4, 1]
    # a send that fails leaves the mark: the file is sent from its start at the next one
    assert log.forget("Cy") == 2
    db.away = True
    with pytest.raises(Away):
        log.send(db, state, DEVICE)
    assert list(log.folder().glob("*.again"))
    db.away = False
    assert log.send(db, state, DEVICE) == 3 and not list(log.folder().glob("*.again"))


def test_files_left_waiting_by_the_version_before_are_found_and_sent_on_from_their_marks(tmp_path, monkeypatch):
    """Readings and songs wait on the owner's devices in files 1.24.4 wrote, and the sync's state there says how
    many bytes of each it has sent, as "xlog:<file>" and "mlog:<file>". Both logs must find them as they are: the
    folder, the file's name, the line, the state's key. With another folder or line they would never be sent, and
    with another key every file would be sent again whole."""
    import exprlog
    import musiclog
    import paths
    monkeypatch.setattr(paths, "HOME", tmp_path)
    at = datetime(2025, 11, 17, 12, 0, 5, tzinfo=timezone.utc)
    left = [(exprlog, "expression", "xlog",
             ['{"t": 1763380800000, "name": "Ann", "label": "happy", "share": 0.8123, "state": "plain", '
              '"data": {"scores": {"happy": 0.8}, "setting": "alone"}}',
              '{"t": 1763380805000, "name": "Zoë", "label": "calm", "share": null, "state": "", "data": {}}'],
             "INSERT INTO expression_log (device, name, at, label, share, state, data) VALUES (%s, %s, %s, %s, %s, %s, %s) "
             "ON CONFLICT (device, name, at) DO NOTHING",
             (DEVICE, "Zoë", at, "calm", None, "", "{}")),
            (musiclog, "music", "mlog",
             ['{"t": 1763380800000, "name": "Ann", "artist": "Mad Season", "song": "Wake Up", "seconds": 240.0, '
              '"data": {"app": "Spotify", "heard": {"tempo": 72.0}}}',
              '{"t": 1763380805000, "name": "Zoë", "artist": "", "song": "music in the room", "seconds": 90.0, '
              '"data": {"app": "room"}}'],
             "INSERT INTO music_log (device, name, at, artist, song, seconds, data) VALUES (%s, %s, %s, %s, %s, %s, %s) "
             "ON CONFLICT (device, name, at) DO NOTHING",
             (DEVICE, "Zoë", at, "", "music in the room", 90.0, '{"app": "room"}'))]
    for mod, folder, key, lines, sql, second in left:
        for end in ("\r\n", "\n"):                       # as written on Windows; as elsewhere, and after a forget
            f = tmp_path / folder / "2025-11-17.jsonl"
            f.parent.mkdir(exist_ok=True)
            f.write_bytes("".join(ln + end for ln in lines).encode("utf-8"))
            quiet(f)
            assert mod.folder() == f.parent and [r["name"] for r in mod.waiting()] == ["Ann", "Zoë"]
            db = Database()
            state = State({f"{key}:2025-11-17.jsonl": str(len((lines[0] + end).encode("utf-8")))})   # the first was sent
            assert mod.send(db, state, DEVICE) == 1 and db.asked == [(sql, [second])], (folder, end)
            assert not f.exists() and state == {}        # a past day, all of it sent: gone, with its mark


def test_an_export_has_the_databases_rows_and_those_still_waiting_each_once(log, tmp_path, monkeypatch):
    """A person's export (persondata.py) holds their rows in the shared database and those still waiting here.
    Today's waiting file has rows that are sent already: a row this device has in both places is written once.
    When the database can't be reached, what waits here is still exported, and a note says what may be missing."""
    import facedb
    now = int(time.time()) * 1000                        # a whole second: read back from the database as the same number
    log.add([row("Ann", now), row("Ann", now + 1000), row("Bo", now)])
    theirs = [(DEVICE, when(now), "garden", 120, {"pace": 1.5}), ("another device", when(now), "street", None, {})]
    asked = []

    class Conn:
        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

        def execute(self, sql, args):
            asked.append((sql, args))
            return iter(theirs)
    monkeypatch.setattr(facedb, "configured", lambda: True)
    monkeypatch.setattr(facedb, "connect", Conn)
    monkeypatch.setattr(facedb, "device", lambda: {"id": DEVICE})
    out = tmp_path / "out.jsonl"
    assert log.export_to(out, "Ann") == (3, None)
    assert asked == [("SELECT device::text, at, place, steps, data FROM step_log WHERE name = %s ORDER BY at", ("Ann",))]
    lines = [json.loads(ln) for ln in out.read_text("utf-8").splitlines()]
    assert lines[0] == {"t": now, "when": f"{when(now):%Y-%m-%dT%H:%M:%SZ}", "device": DEVICE, "place": "garden",
                        "steps": 120, "data": {"pace": 1.5}}
    assert (lines[1]["device"], lines[1]["place"], lines[1]["steps"]) == ("another device", "street", None)
    assert lines[2] == {"t": now + 1000, "place": "garden", "steps": 120, "data": {"pace": 1.5}, "device": DEVICE,
                        "waitingToBeSent": True}

    def away():
        raise Away("no route to the server\nand more that doesn't belong in an export")
    monkeypatch.setattr(facedb, "connect", away)
    n, note = log.export_to(out, "Ann")
    assert n == 2 and note == "the shared database couldn't be read (no route to the server)"
    assert all(json.loads(ln)["waitingToBeSent"] for ln in out.read_text("utf-8").splitlines())
    monkeypatch.setattr(facedb, "configured", lambda: False)        # no shared database: nothing is missing
    assert log.export_to(out, "Bo") == (1, None)
