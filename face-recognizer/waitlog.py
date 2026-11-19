"""
waitlog.py - a log whose rows wait on this device until the shared database's sync has sent them. What the
expression log (exprlog.py) and the music log (musiclog.py) do alike is here once, so that the next log of this
kind isn't a third copy of it.

Such a log is a table of the shared database with one row per (device, name, at): something about a named person
at a moment. The database is where the rows are kept; a device holds them only until they are there.

Rows wait in HOME/<folder>/<UTC day>.jsonl, one JSON object a line with "t" (epoch ms), "name" and the log's own
fields, in the file of the UTC day of the row's own time. The sync (dbsync_home.HomePart.after_push -> the log's
send) puts the lines it hasn't sent yet into the table and keeps in its state (HOME/sync.sqlite) how many bytes of
each file it has sent, as "<key>:<file name>". Sending a line twice changes nothing (one row per device, name and
time), so a file that was rewritten (a person forgotten) is simply sent again from its start: forget() leaves a
mark beside it (<file>.again) that says so, because the file's size alone doesn't (through 1.24.4 only a file
shorter than what was sent counted as rewritten: with rows added after the forget it was as long again, the send
began in the middle of them, and they never reached the database). A past day's file goes once all of it is in
the database.

Files written by earlier versions wait on the devices: the folder, the file names, the lines and the state's keys
stay as they are (tests/test_waitlog.py holds them).

Each log's own module says what a row is and checks it, decides whether anything is kept at all (its switch, a
shared database to keep it in) and leaves out forgotten people (forgotten.names) before it calls add():
none of that is here. A new log also needs its table (a new db/NNN_*.sql), its send() called from
HomePart.after_push, its rows deleted in dbsync_home._forget_on_server, and its place in persondata.py (the list at
its top, export, forget) with a test.
"""
import json
import time
from datetime import datetime, timezone

import paths


class WaitLog:
    """One log's waiting rows: kept (add), read (waiting, days), dropped for a person (forget), sent (send) and
    written out for a person (export_to).

      folder   the folder's name in HOME
      table    the log's table in the shared database
      columns  the table's columns after device, name and at; a waiting line has them under the same names
      values   values(line) -> what goes into those columns for a waiting line, in their order; a line it raises
               KeyError, ValueError or TypeError for (a field it must have isn't there) is not sent
      key      names this log's files in the sync's state; with another one every waiting file is sent again
      needs    fields a line must have, besides "name" and "t", to count as a waiting row
    """

    def __init__(self, folder, table, columns, values, key, needs=()):
        self.folder_name, self.columns, self.values, self.key = folder, tuple(columns), values, key
        self.needs = tuple(needs)
        self.lock = paths.SharedLock(f"waitlog-{folder}")     # appended to by the tracker, sent and rewritten by the sync
        # table and columns are the log's own constants, never anything read from a file or a request
        cols = ", ".join(("device", "name", "at") + self.columns)
        self.insert = (f"INSERT INTO {table} ({cols}) VALUES ({', '.join(['%s'] * (3 + len(self.columns)))}) "
                       "ON CONFLICT (device, name, at) DO NOTHING")
        self.select = f"SELECT device::text, at, {', '.join(self.columns)} FROM {table} WHERE name = %s ORDER BY at"

    def folder(self):
        return paths.HOME / self.folder_name       # read each time, not kept: each test has a HOME of its own

    def add(self, rows):
        """Keep rows until the sync sends them: each a line at the end of its UTC day's file. The log has checked
        them already (every one has its "t" and "name"). Returns how many were kept."""
        by_day = {}
        for r in rows:
            day = datetime.fromtimestamp(r["t"] / 1000, timezone.utc).strftime("%Y-%m-%d")
            by_day.setdefault(day, []).append(json.dumps(r, ensure_ascii=False))
        with self.lock:
            self.folder().mkdir(parents=True, exist_ok=True)
            for day, lines in by_day.items():
                # (opened as text, as these files always were: on Windows a line ends \r\n. send() takes both.)
                with open(self.folder() / f"{day}.jsonl", "a", encoding="utf-8") as f:
                    f.write("".join(ln + "\n" for ln in lines))
        return sum(len(v) for v in by_day.values())

    def _files(self):
        return sorted(self.folder().glob("*.jsonl")) if self.folder().is_dir() else []

    def _rows(self, path):
        out = []
        for ln in path.read_text("utf-8").splitlines():
            try:
                o = json.loads(ln)
            except ValueError:
                continue
            if isinstance(o, dict) and o.get("name") and o.get("t") is not None and all(o.get(k) for k in self.needs):
                out.append(o)
        return out

    def waiting(self, name=None):
        """Rows not sent yet (of one person, or everyone's). Today's file holds today's rows, sent or not."""
        with self.lock:
            return [o for f in self._files() for o in self._rows(f) if name is None or o["name"] == name]

    def days(self):
        """How many rows each day's file holds: {"2026-11-17": 3, ...}. A past day's file is still here only while
        some of it isn't sent."""
        with self.lock:
            return {f.stem: len(self._rows(f)) for f in self._files()}

    def forget(self, name):
        """Drop name's waiting rows (persondata.forget; the table's go with the sync's forget). Returns how many."""
        n = 0
        with self.lock:
            for f in self._files():
                rows = self._rows(f)
                keep = [o for o in rows if o["name"] != name]
                if len(keep) < len(rows):
                    n += len(rows) - len(keep)
                    paths.atomic_write(f, "".join(json.dumps(o, ensure_ascii=False) + "\n" for o in keep))
                    self._again(f).write_text(paths.utc_now(), "utf-8")      # send(): from this file's start
        return n

    @staticmethod
    def _again(f):
        """The mark beside a waiting file that forget() rewrote: what the sync's state says was sent of it no
        longer says where the unsent lines begin."""
        return f.with_name(f.name + ".again")

    def send(self, conn, state, device):
        """The sync: put waiting rows into the table. A row is one per (device, name, time), so sending a line
        twice changes nothing: a file that was rewritten (a person forgotten) is simply sent again. Past days'
        files go once all of them is in the database. Returns how many rows were sent. When the database can't
        take them its error goes on to the sync, and they wait as they were for its next try."""
        today, sent = datetime.now(timezone.utc).strftime("%Y-%m-%d"), 0
        for f in self._files():
            key = f"{self.key}:{f.name}"
            with self.lock:
                try:
                    size = f.stat().st_size
                except OSError:
                    continue
                done = int(state.get(key, 0))
                try:
                    again = self._again(f).stat().st_mtime_ns     # rewritten by a forget since it was last sent
                except OSError:
                    again = None
                if size < done or again is not None:
                    done = 0
                chunk = b""
                if size > done:
                    with open(f, "rb") as fh:
                        fh.seek(done)
                        chunk = fh.read(size - done)
            end = chunk.rfind(b"\n") + 1                # whole lines only; the rest goes next time
            rows = []
            for ln in chunk[:end].decode("utf-8", "replace").splitlines():
                try:
                    o = json.loads(ln)
                    rows.append((device, o["name"], datetime.fromtimestamp(o["t"] / 1000, timezone.utc),
                                 *self.values(o)))
                except (ValueError, KeyError, TypeError, OverflowError, OSError):
                    continue
            if rows:
                with conn.transaction():
                    conn.cursor().executemany(self.insert, rows)
            if end or again is not None:
                state.put(key, done + end)
                sent += len(rows)
            with self.lock:
                try:                                     # the forget this send began after is dealt with; one
                    if again is not None and self._again(f).stat().st_mtime_ns == again:     # since then is not
                        self._again(f).unlink()
                except OSError:
                    pass
                try:                                     # a past day, all of it sent, nothing added for a minute: done
                    st = f.stat()
                    if f.stem < today and st.st_size == done + end and time.time() - st.st_mtime > 60 \
                            and not self._again(f).exists():
                        f.unlink()
                        state.drop(key)
                except OSError:
                    pass
        return sent

    def export_to(self, path, name):
        """Write name's rows to path, one JSON object a line: those in the shared database (when it can be reached)
        and those still waiting here. Returns (how many, why some may be missing or None)."""
        n, note, seen = 0, None, set()
        with open(path, "w", encoding="utf-8") as out:
            try:
                import facedb
                if facedb.configured():
                    with facedb.connect() as conn:
                        for dev, at, *rest in conn.execute(self.select, (name,)):
                            t = int(at.timestamp() * 1000)
                            seen.add((dev, t))
                            out.write(json.dumps({"t": t, "when": at.strftime("%Y-%m-%dT%H:%M:%SZ"), "device": dev,
                                                  **dict(zip(self.columns, rest))}, ensure_ascii=False) + "\n")
                            n += 1
            except Exception as e:                       # can't be reached now: say so in the export
                note = f"the shared database couldn't be read ({str(e).strip().splitlines()[0][:200] if str(e).strip() else type(e).__name__})"
            try:
                import facedb
                me = facedb.device()["id"]
            except Exception:
                me = None
            for o in self.waiting(name):
                if (me, o["t"]) not in seen:
                    out.write(json.dumps({**{k: o.get(k) for k in ("t", *self.columns)},
                                          "device": me, "waitingToBeSent": True}, ensure_ascii=False) + "\n")
                    n += 1
        return n, note
