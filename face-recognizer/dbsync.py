"""Keeps this device's labeling data and the shared database (facedb.py) the same, both ways, in the background.

The files in data/ stay the working copy: the labeling tool and the tracker read and write them as before, and they
are what's used while the database can't be reached. This module compares them with what was last synced and pushes
only what changed, then pulls what other devices changed (the database's `changes` log, by sequence number).

How it decides what changed: for every row (a face, a label, a photo, a Web search answer) it keeps a digest of the
row as last synced, in HOME/sync.sqlite. A row whose digest differs is sent; a row synced and since gone locally is
deleted on the server. Nothing has to be queued by hand: whatever the programs change through their own code is
found at the next check (after each save, `note()`; and every minute anyway). Changes made offline simply wait.

Merging, the same rules as dbimport:
- a label named by hand beats an automatic one from another device; otherwise the last one sent wins;
- a face Ignored on one device stays Ignored (its print and picture don't come back, except by Undo on that device);
- photo paths are each device's own (photo_paths); camera dates and people tags (facts) are read on each device
  from its own copy of the photo, and taken from the database only for photos this device hasn't read;
- the device's own undo history log is appended to label_history, never pulled.

Joining (the first sync with a database, or with a different one): everything this device has is merged in
(dbimport.run), then everything the database has is pulled, so the device ends up with every device's faces, labels
and pictures. Photos from other devices come without a path: they're shown by their faces' pictures, and "The
original photo isn't on this computer" until this device imports the same photo itself."""
import hashlib
import json
import sqlite3
import threading
import time
import traceback
from datetime import datetime, timezone

import numpy as np

import dbimport
import facedb
import paths

STATE = paths.HOME / "sync.sqlite"
EVERY = 3              # seconds between checks for other devices' changes (one cheap query)
LOCAL_EVERY = 60       # seconds between full local checks when nothing called note()
RETRY = (5, 15, 60)    # seconds to wait before connecting again after a failure (then every minute)
BATCH = 300
TABLES = ("photos", "faces", "labels", "web_suggestions")   # in the order they're applied


def _h(*parts):
    h = hashlib.sha1()
    for p in parts:
        if p is None:
            h.update(b"\0N")
        elif isinstance(p, (bytes, bytearray, memoryview)):
            h.update(b"\0B" + bytes(p))
        elif isinstance(p, np.ndarray):
            h.update(b"\0A" + np.ascontiguousarray(p, np.float32).tobytes())
        else:
            h.update(b"\0S" + json.dumps(p, sort_keys=True, default=str).encode())
    return h.hexdigest()


def _f(v):
    """A float or None (NaN means 'not scored')."""
    if v is None:
        return None
    v = float(v)
    return None if v != v else v


# --- local state (sync.sqlite) -----------------------------------------------------------------------------------

class State:
    """Digests of rows as last synced, the database joined and how far its change log was read."""

    def __init__(self, path=STATE):
        path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(str(path), check_same_thread=False, isolation_level=None)
        self.lock = threading.Lock()
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("CREATE TABLE IF NOT EXISTS digest (tbl TEXT, key TEXT, d TEXT, PRIMARY KEY (tbl, key))")
        self.db.execute("CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT)")

    def get(self, k, default=None):
        with self.lock:
            r = self.db.execute("SELECT v FROM meta WHERE k = ?", (k,)).fetchone()
        return r[0] if r else default

    def put(self, k, v):
        with self.lock:
            self.db.execute("INSERT OR REPLACE INTO meta VALUES (?, ?)", (k, str(v)))

    def digests(self, tbl):
        with self.lock:
            return dict(self.db.execute("SELECT key, d FROM digest WHERE tbl = ?", (tbl,)).fetchall())

    def set_digests(self, tbl, items):
        """items: {key: digest or None (forget the key)}."""
        with self.lock:
            self.db.execute("BEGIN")
            self.db.executemany("INSERT OR REPLACE INTO digest VALUES (?, ?, ?)",
                                [(tbl, k, d) for k, d in items.items() if d is not None])
            self.db.executemany("DELETE FROM digest WHERE tbl = ? AND key = ?",
                                [(tbl, k) for k, d in items.items() if d is None])
            self.db.execute("COMMIT")

    def reset(self):
        with self.lock:
            self.db.execute("DELETE FROM digest")
            self.db.execute("DELETE FROM meta")

    def close(self):
        self.db.close()


# --- what each table looks like locally ----------------------------------------------------------------------------

class Local:
    """The labeling tool's data as rows: read under db.lock, applied under db.lock. `db` is a labels.LabelDB;
    `web` its Web search answers ({fid: {...}}, with web_save() to write them). One "part" of the sync: a part has
    TABLES, digests(), rows(), apply(), push(), fetch(), all_keys(), after_push() and on_join()
    (dbsync_home.HomePart is the other)."""
    TABLES = TABLES
    KEYS = {"photos": "hash", "faces": "fid", "labels": "fid", "web_suggestions": "fid"}

    def __init__(self, db, web=None, web_save=None):
        self.db, self.web, self.web_save = db, web if web is not None else {}, web_save

    def push(self, conn, tbl, rows, deletes):
        push(conn, tbl, rows, deletes)

    def fetch(self, conn, tbl, keys):
        return fetch(conn, tbl, keys)

    def all_keys(self, conn, tbl):
        return [r[0] for r in conn.execute(f"SELECT {self.KEYS[tbl]} FROM {tbl}")]

    def after_push(self, conn, state):
        """The undo history log's new lines (this device's own; never pulled)."""
        import face_id as F
        p = F.DATA / "history.jsonl"
        try:
            lines = p.read_text("utf-8").splitlines()
        except OSError:
            return
        done = int(state.get("history_lines", 0))
        if len(lines) < done:                 # the file was rewritten (forget): start counting again
            done = len(lines)
        new = []
        for line in lines[done:]:
            try:
                h = json.loads(line)
            except ValueError:
                continue
            new.append((dbimport.ts(h.get("t")) or datetime.now(timezone.utc), facedb.device()["id"],
                        h.get("what", ""), json.dumps(h.get("faces") or {})))
        if new:
            with conn.transaction():
                conn.cursor().executemany("INSERT INTO label_history (t, device, what, faces) VALUES (%s, %s, %s, %s)",
                                          new)
        state.put("history_lines", len(lines))

    def on_join(self, state):
        import face_id as F
        p = F.DATA / "history.jsonl"
        state.put("history_lines", len(p.read_text("utf-8").splitlines()) if p.exists() else 0)

    def before_join(self):
        """Every file written as it is in memory, so dbimport reads the newest."""
        import face_id as F
        db = self.db
        with db.lock:
            db.store.save()
            F.save_json("labels.json", db.labels)
            db.facts.save()
            if self.web_save:
                self.web_save()

    # digests of every local row, per table
    def digests(self):
        db, s = self.db, self.db.store
        with db.lock:
            facts = {h: dict(f) for h, f in db.facts.facts.items() if f.get("key")}
            out = {"labels": {k: _h(v) for k, v in db.labels.items()},
                   "web_suggestions": {k: _h(v) for k, v in self.web.items()},
                   "photos": {h: self._photo_digest(h, r, facts.get(h)) for h, r in s.photos.items()},
                   "faces": {}}
            for i, fid in enumerate(s.fid):
                out["faces"][fid] = self._face_digest(i)
        return out

    def _face_digest(self, i):
        s = self.db.store
        rec = s.photos.get(s.photo[i], {})
        fid = s.fid[i]
        printless = s.printless(i)
        return _h(s.photo[i], s.file[i], np.asarray(s.box[i], np.float32), _f(s.quality[i]), s.model,
                  None if printless else np.asarray(s.emb[i], np.float32), None if printless else s.thumbs[i] or None,
                  _f((rec.get("maybe") or {}).get(fid)), (rec.get("at") or {}).get(fid))

    def _photo_digest(self, h, r, f):
        shared = {k: v for k, v in r.items() if k not in ("maybe", "at")}
        facts = None if not f else {"t": f.get("t"), "tags": f.get("tags") or [],
                                    "faces": {self.db.store.fid[i]: nm for i, nm in f.get("faces", {}).items()}}
        return _h(shared, "maybe" in r, facts)

    # rows to send
    def rows(self, tbl, keys):
        db, s = self.db, self.db.store
        out = {}
        with db.lock:
            if tbl == "labels":
                return {k: dict(db.labels[k]) for k in keys if k in db.labels}
            if tbl == "web_suggestions":
                return {k: self.web[k] for k in keys if k in self.web}
            if tbl == "photos":
                for h in keys:
                    if h in s.photos:
                        r = s.photos[h]
                        f = db.facts.facts.get(h)
                        out[h] = dict(r, _facts=None if not (f and f.get("key")) else
                                      {"t": f.get("t"), "tags": f.get("tags") or [],
                                       "faces": {s.fid[i]: nm for i, nm in f.get("faces", {}).items()}},
                                      _key=(f or {}).get("key"))
                return out
            idx = {f: i for i, f in enumerate(s.fid)}
            for fid in keys:
                i = idx.get(fid)
                if i is None:
                    continue
                rec = s.photos.get(s.photo[i], {})
                printless = s.printless(i)
                out[fid] = dict(photo=s.photo[i], file=s.file[i], box=[float(v) for v in s.box[i][:5]],
                                quality=_f(s.quality[i]), model=s.model,
                                print=None if printless else np.asarray(s.emb[i], np.float32).copy(),
                                picture=None if printless else (s.thumbs[i] or None),
                                possible=_f((rec.get("maybe") or {}).get(fid)), frame=(rec.get("at") or {}).get(fid))
        return out

    # rows received: {key: row or None (deleted)}. Returns the new local digests of those keys.
    def apply(self, tbl, rows, log=print):
        db, s = self.db, self.db.store
        import face_id as F
        with db.lock:
            if tbl == "labels":
                for k, r in rows.items():
                    if r is None:
                        db.labels.pop(k, None)
                    else:
                        db.labels[k] = {x: r[x] for x in ("n", "by", "s", "ok", "not") if r.get(x) is not None}
                db.changed()
                F.save_json("labels.json", db.labels)
                db.build_people()
                return {k: _h(db.labels[k]) if k in db.labels else None for k in rows}
            if tbl == "web_suggestions":
                for k, r in rows.items():
                    if r is None:
                        self.web.pop(k, None)
                    else:
                        self.web[k] = r
                if self.web_save:
                    self.web_save()
                return {k: _h(self.web[k]) if k in self.web else None for k in rows}
            if tbl == "photos":
                facts_new = []
                for h, r in rows.items():
                    if r is None:
                        continue                # photos are never deleted by another device
                    rec = s.photos.setdefault(h, {})
                    for k in ("name", "size", "fp", "faces", "dup_of", "video", "fps"):
                        if r.get(k) is not None:
                            rec[k] = r[k]
                        elif k in ("dup_of", "video", "fps"):
                            rec.pop(k, None)
                    rec.update(r.get("extra") or {})
                    if r.get("maybe_read"):
                        rec.setdefault("maybe", {})
                    if r.get("facts") and not (db.facts.facts.get(h) or {}).get("key"):
                        facts_new.append((h, r["facts"]))
                if facts_new:
                    idx = {f: i for i, f in enumerate(s.fid)}
                    with db.facts.lock:
                        for h, f in facts_new:
                            db.facts.facts[h] = {"t": f.get("t"), "tags": f.get("tags") or [],
                                                 "faces": {idx[k]: nm for k, nm in (f.get("faces") or {}).items()
                                                           if k in idx},
                                                 "key": "synced"}     # not this device's: read again if the photo is here
                    db.facts.save()
                s.save()
                facts = {h: dict(f) for h, f in db.facts.facts.items() if f.get("key")}
                return {h: self._photo_digest(h, s.photos[h], facts.get(h)) for h in rows if h in s.photos}
            # faces
            dim = F.REC["dim"]
            skipped = 0
            idx = {f: i for i, f in enumerate(s.fid)}
            for fid, r in rows.items():
                if r is None:
                    if fid in idx:              # gone on the server: keep where it is, like Ignore
                        s.forget(idx[fid])
                    continue
                if r["model"] != s.model:
                    skipped += 1
                    continue
                emb = np.zeros(dim, np.float32) if r.get("print") is None else np.asarray(r["print"], np.float32)
                q = float("nan") if r.get("quality") is None else float(r["quality"])
                i = s.put(fid, r["photo"], r.get("file") or "", r["box"], emb, r.get("picture") or b"", q)
                idx[fid] = i
                rec = s.photos.setdefault(r["photo"], {})
                if r.get("possible") is not None:
                    rec.setdefault("maybe", {})[fid] = r["possible"]
                if r.get("frame") is not None:
                    rec.setdefault("at", {})[fid] = r["frame"]
            if skipped:
                log(f"Shared database: {skipped} faces have prints from another recognition model; not taken.")
            db._forgot()
            s.save()
            return {fid: self._face_digest(idx[fid]) for fid in rows if fid in idx}


# --- the server side -----------------------------------------------------------------------------------------------

def push(conn, tbl, rows, deletes):
    """Send rows {key: row} and deletions [key] of one table in one transaction."""
    cur = conn.cursor()
    me = facedb.device()["id"]
    with conn.transaction():
        items = list(rows.items())
        for b in range(0, len(items), BATCH):
            part = items[b:b + BATCH]
            if tbl == "labels":
                cur.executemany("""
                    INSERT INTO labels AS l (fid, n, by, s, ok, nope) VALUES (%s, %s, %s, %s, %s, %s)
                    ON CONFLICT (fid) DO UPDATE SET n = excluded.n, by = excluded.by, s = excluded.s,
                        ok = excluded.ok, nope = excluded.nope
                    WHERE NOT (l.by = 'me' AND excluded.by = 'auto' AND l.n IS DISTINCT FROM excluded.n)""",
                    [(k, r.get("n"), r.get("by"), r.get("s"), r.get("ok"), r.get("not")) for k, r in part])
            elif tbl == "web_suggestions":
                cur.executemany("""INSERT INTO web_suggestions (fid, data) VALUES (%s, %s)
                                   ON CONFLICT (fid) DO UPDATE SET data = excluded.data""",
                                [(k, json.dumps(dbimport.utc_when(r))) for k, r in part])
            elif tbl == "photos":
                known = {"path", "size", "mtime", "fp", "faces", "name", "maybe", "dup_of", "video", "fps", "at",
                         "_facts", "_key"}
                cur.executemany("""
                    INSERT INTO photos (hash, name, size, fp, faces, dup_of, video, fps, maybe_read, extra, facts,
                                        taken, tags, facts_key)
                    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                    ON CONFLICT (hash) DO UPDATE SET name = excluded.name, size = excluded.size, fp = excluded.fp,
                        faces = excluded.faces, dup_of = excluded.dup_of, video = excluded.video, fps = excluded.fps,
                        maybe_read = photos.maybe_read OR excluded.maybe_read, extra = excluded.extra,
                        facts = coalesce(excluded.facts, photos.facts), taken = coalesce(excluded.taken, photos.taken),
                        tags = CASE WHEN excluded.facts IS NULL THEN photos.tags ELSE excluded.tags END,
                        facts_key = coalesce(excluded.facts_key, photos.facts_key)""",
                    [(h, r.get("name") or "", r.get("size"), bytes.fromhex(r["fp"]) if _hexok(r.get("fp")) else None,
                      r.get("faces"), r.get("dup_of"), bool(r.get("video")), r.get("fps"), "maybe" in r,
                      json.dumps({k: v for k, v in r.items() if k not in known}),
                      json.dumps(r["_facts"]) if r.get("_facts") else None,
                      (r.get("_facts") or {}).get("t"), list((r.get("_facts") or {}).get("tags") or []),
                      r.get("_key") if r.get("_key") != "synced" else None) for h, r in part])
                with_path = [(me, h, r["path"], r.get("mtime")) for h, r in part if r.get("path")]
                cur.executemany("""INSERT INTO photo_paths (device, hash, path, mtime) VALUES (%s, %s, %s, %s)
                                   ON CONFLICT (device, hash) DO UPDATE SET path = excluded.path,
                                   mtime = excluded.mtime""", with_path)
                no_path = [h for h, r in part if not r.get("path")]
                if no_path:
                    cur.execute("DELETE FROM photo_paths WHERE device = %s AND hash = ANY(%s)", (me, no_path))
            else:   # faces: an Ignored face's print and picture don't come back from another device
                cur.executemany("""
                    INSERT INTO faces (fid, photo, file, box, quality, model, print, picture, possible, frame)
                    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                    ON CONFLICT (fid) DO UPDATE SET photo = excluded.photo, file = excluded.file, box = excluded.box,
                        quality = excluded.quality, model = excluded.model,
                        print = CASE WHEN faces.print IS NULL AND excluded.print IS NOT NULL
                                     AND faces.updated_by IS DISTINCT FROM %s::uuid THEN NULL ELSE excluded.print END,
                        picture = CASE WHEN faces.print IS NULL AND excluded.print IS NOT NULL
                                       AND faces.updated_by IS DISTINCT FROM %s::uuid THEN NULL ELSE excluded.picture END,
                        possible = excluded.possible, frame = excluded.frame""",
                    [(k, r["photo"], r["file"], r["box"], r["quality"], r["model"], r["print"], r["picture"],
                      r["possible"], r["frame"], me, me) for k, r in part])
        if deletes:
            key = {"labels": "fid", "web_suggestions": "fid", "faces": "fid", "photos": "hash"}[tbl]
            if tbl in ("labels", "web_suggestions"):        # photos and faces are never deleted from here
                cur.execute(f"DELETE FROM {tbl} WHERE {key} = ANY(%s)", (list(deletes),))


def _hexok(v):
    return isinstance(v, str) and len(v) % 2 == 0 and all(c in "0123456789abcdef" for c in v)


def fetch(conn, tbl, keys):
    """Current server rows {key: row or None} in the shape Local.apply takes."""
    out = {k: None for k in keys}
    keys = list(keys)
    for b in range(0, len(keys), BATCH * 3):
        part = keys[b:b + BATCH * 3]
        if tbl == "labels":
            for fid, n, by, s, ok, nope in conn.execute(
                    "SELECT fid, n, by, s, ok, nope FROM labels WHERE fid = ANY(%s)", (part,)):
                out[fid] = {"n": n, "by": by, "s": s, "ok": ok, "not": nope}
        elif tbl == "web_suggestions":
            for fid, data in conn.execute("SELECT fid, data FROM web_suggestions WHERE fid = ANY(%s)", (part,)):
                out[fid] = data
        elif tbl == "photos":
            for row in conn.execute("""SELECT p.hash, p.name, p.size, p.fp, p.faces, p.dup_of, p.video, p.fps,
                                              p.maybe_read, p.extra, p.facts
                                       FROM photos p WHERE p.hash = ANY(%s)""", (part,)):
                h, name, size, fp, faces, dup_of, video, fps, maybe_read, extra, facts = row
                out[h] = {"name": name, "size": size, "fp": bytes(fp).hex() if fp else None, "faces": faces,
                          "dup_of": dup_of, "video": True if video else None, "fps": fps, "maybe_read": maybe_read,
                          "extra": extra or {}, "facts": facts}
        else:
            for row in conn.execute("""SELECT fid, photo, file, box, quality, model, print, picture, possible, frame
                                       FROM faces WHERE fid = ANY(%s)""", (part,)):
                fid, photo, file, box, q, model, pr, pic, possible, frame = row
                if pr is not None and hasattr(pr, "to_numpy"):
                    pr = pr.to_numpy()
                out[fid] = {"photo": photo, "file": file, "box": list(box), "quality": q, "model": model,
                            "print": pr, "picture": bytes(pic) if pic else None, "possible": possible,
                            "frame": frame}
    return out


# --- the background job ------------------------------------------------------------------------------------------

class Sync:
    """One per labeling tool. start() runs it in the background; note() after the labeling data was saved;
    status() for the Settings tab."""

    def __init__(self, local, app_version="", log=print, state_path=STATE, home=None):
        self.local, self.app_version, self.log = local, app_version, log
        self.parts = [local] + ([home] if home is not None else [])
        self.state = State(state_path)
        self.wake = threading.Event()
        self.stopping = threading.Event()
        self.thread = None
        self.conn = None
        self.lock = threading.Lock()          # one sync step at a time (the thread, or join() from a request)
        self.info = {"state": "off", "text": "", "waiting": 0, "last": None, "error": None}
        self._last_local = 0.0
        self._dirty = True

    # -- outside interface
    def start(self):
        if self.thread is None:
            self.thread = threading.Thread(target=self._run, daemon=True, name="shared database")
            self.thread.start()
        return self

    def stop(self):
        self.stopping.set()
        self.wake.set()

    def note(self):
        """Something in the labeling data was saved: check soon."""
        self._dirty = True
        self.wake.set()

    def status(self):
        """{state: off|joining|online|offline, text, waiting, last, error}; waiting: rows changed here and not sent."""
        out = dict(self.info)
        if out["state"] in ("online", "offline"):
            try:
                n = 0
                if out["state"] == "offline":
                    for part in self.parts:
                        local = part.digests()
                        for t in part.TABLES:
                            if local[t] is not None:
                                synced = self.state.digests(t)
                                n += sum(1 for k, d in local[t].items() if synced.get(k) != d)
                out["waiting"] = n
            except Exception:
                pass
        return out

    # -- one step
    def step(self):
        """Connect if needed, join if needed, push local changes, pull others'. Returns what it did."""
        with self.lock:
            if not facedb.configured():
                self._close()
                self.info.update(state="off", text="No shared database: data is kept on this device only.")
                return {}
            if self.conn is None or self.conn.closed:
                self.conn = facedb.open_ready(self.app_version)
            conn = self.conn
            inst = str(conn.execute("SELECT id FROM instance").fetchone()[0])
            if self.state.get("instance") != inst:
                return self._join(conn, inst)
            did = {}
            if self._dirty or time.time() - self._last_local > LOCAL_EVERY:
                self._dirty = False
                self._last_local = time.time()
                did["pushed"] = self._push(conn)
            did["pulled"] = self._pull(conn)
            conn.execute("UPDATE devices SET last_seen = now() WHERE id = %s", (facedb.device()["id"],))
            return did

    def _push(self, conn):
        sent = 0
        for part in self.parts:
            local = part.digests()
            for tbl in part.TABLES:
                mine = local[tbl]
                if mine is None:                  # not available here now (keys without the passphrase)
                    continue
                synced = self.state.digests(tbl)
                changed = [k for k, d in mine.items() if synced.get(k) != d]
                gone = [k for k in synced if k not in mine]
                if not changed and not gone:
                    continue
                rows = part.rows(tbl, changed)
                part.push(conn, tbl, rows, gone)
                self.state.set_digests(tbl, {**{k: mine[k] for k in rows}, **{k: None for k in gone}})
                sent += len(rows) + len(gone)
            part.after_push(conn, self.state)
        return sent

    def _pull(self, conn):
        cursor = int(self.state.get("cursor", 0))
        top = conn.execute("SELECT coalesce(max(seq), 0) FROM changes").fetchone()[0]
        if top <= cursor:
            return 0
        me = facedb.device()["id"]
        owner = {t: part for part in self.parts for t in part.TABLES}
        rows = conn.execute("""SELECT tbl, key, max(seq), (array_agg(device ORDER BY seq DESC))[1]::text
                               FROM changes WHERE seq > %s AND seq <= %s AND tbl = ANY(%s)
                               GROUP BY tbl, key""", (cursor, top, list(owner))).fetchall()
        want = {t: [] for t in owner}
        for tbl, key, _seq, dev in rows:
            if dev != me:                       # the newest change of this row is another device's
                want[tbl].append(key)
        got = 0
        for tbl, part in owner.items():         # in each part's order (photos before faces before labels)
            if want[tbl]:
                server = part.fetch(conn, tbl, want[tbl])
                self.state.set_digests(tbl, part.apply(tbl, server, self.log))
                got += len(want[tbl])
        self.state.put("cursor", top)
        return got

    def _join(self, conn, inst):
        """First sync with this database: merge everything in, then take everything."""
        import face_id as F
        self.info.update(state="joining", text="Joining the shared database: sending this device's data ...")
        self.local.before_join()
        self.state.reset()
        counts = dbimport.run(conn, F.DATA, paths.HOME, facedb.device())
        self.info.update(text="Joining the shared database: taking every device's data ...")
        top = conn.execute("SELECT coalesce(max(seq), 0) FROM changes").fetchone()[0]
        for part in self.parts:
            for tbl in part.TABLES:
                keys = part.all_keys(conn, tbl)
                for b in range(0, len(keys), 2000):
                    got = part.fetch(conn, tbl, keys[b:b + 2000])
                    self.state.set_digests(tbl, part.apply(tbl, got, self.log))
                # a row only this device has (added while joining) differs from the digests: it's sent next time
            part.on_join(self.state)
        self.state.put("cursor", top)
        self.state.put("instance", inst)
        self._dirty = True
        self.log(f"Joined the shared database: sent {counts.get('faces', 0)} faces and {counts.get('labels', 0)} "
                 f"labels; this device now has {self.local.db.n} faces.")
        return {"joined": counts}

    def _close(self):
        if self.conn is not None:
            try:
                self.conn.close()
            except Exception:
                pass
        self.conn = None

    def _run(self):
        fails = 0
        while not self.stopping.is_set():
            try:
                did = self.step()
                fails = 0
                if facedb.configured():
                    self.info.update(state="online", error=None, last=time.strftime("%H:%M:%S"),
                                     text=f"Connected; last checked {time.strftime('%H:%M:%S')}.")
                    if did.get("pushed") or did.get("pulled"):
                        self.info["text"] += f" Sent {did.get('pushed', 0)}, received {did.get('pulled', 0)}."
                wait = EVERY if facedb.configured() else 30
            except Exception as e:
                self._close()
                wait = RETRY[min(fails, len(RETRY) - 1)]
                fails += 1
                msg = str(e).strip().splitlines()[0][:200] if str(e).strip() else type(e).__name__
                self.info.update(state="offline", error=msg,
                                 text=f"Can't reach the shared database ({msg}); working from this device's copy. "
                                      "Changes are sent when it's back.")
                if fails == 1:
                    self.log(f"Shared database: {msg}")
                    if not isinstance(e, OSError) and "connection" not in msg.lower():
                        traceback.print_exc()
            self.wake.wait(wait)
            self.wake.clear()
        self._close()
