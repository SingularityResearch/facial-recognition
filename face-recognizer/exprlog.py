"""
exprlog.py - the expression log: what each named person's face looked like, every few seconds while a tracker sees
them, kept in the shared database (table expression_log, db/010) for reports later. The owner's decision of
2026-11-12; until then expression readings were shown and never stored.

A reading says what a face looked like to the model ("looks happy"), never what the person felt.

The tracker page sends its readings (tracker_ui.js: every EXPR_LOG_EVERY seconds, POST /api/expression-log). They
wait in HOME/expression/<UTC day>.jsonl until the sync has put them in the database (dbsync_home.HomePart.after_push
-> send); a day's waiting file goes once all of it is there, because the database is where the readings are kept.
Without a shared database, or with "Keep expression readings" switched off (ft-o-optExprLog), nothing is written.

One reading: {"t": epoch ms, "name", "label", "share", "state", "data"}
  label  the expression shown           share  its share of the last second's votes, 0..1
  state  plain | change | usual | unsure (expression.js)
  data   numbers and short text only (alerts.check_details): the lead over the next expression, votes, the latest
         frame's score for every expression, valence and arousal, flags, alone or in company, how well the face
         matched, its quality

persondata.py exports and forgets a person's readings: the waiting files here, and the table (export_to; the sync's
forget, dbsync_home._forget_on_server).
"""
import json
import threading
import time
from datetime import datetime, timezone

import alerts
import paths

FOLDER = "expression"            # in HOME: readings waiting to be sent
OPTION = "ft-o-optExprLog"       # the tracker page's "Keep expression readings" (on unless switched off)
MAX_ROWS = 100                   # readings in one request
MAX_DATA = 4000                  # bytes of one reading's data
STATES = ("plain", "change", "usual", "unsure", "")
_lock = threading.Lock()


def folder():
    return paths.HOME / FOLDER


def enabled():
    """Readings are kept only with a shared database to keep them in, and while the switch is on."""
    import devicesettings
    import facedb
    return facedb.configured() and devicesettings.settings("track").get(OPTION, "1") != "0"


def check(r, now_ms=None):
    """One reading as the page sends it, checked; ValueError if it isn't one."""
    if not isinstance(r, dict):
        raise ValueError("a reading is an object")
    name = str(r.get("name") or "").strip()[:100]
    if not name or name == "Unknown" or name.startswith("#"):
        raise ValueError("a reading needs a named person")
    try:
        t = int(r.get("t"))
        share = None if r.get("share") is None else float(r["share"])
    except (TypeError, ValueError):
        raise ValueError("bad reading") from None
    now_ms = time.time() * 1000 if now_ms is None else now_ms
    if abs(t - now_ms) > 86_400_000:
        raise ValueError("reading time out of range")
    label, state = str(r.get("label") or "")[:40], str(r.get("state") or "")[:20]
    if not label or state not in STATES or (share is not None and not 0 <= share <= 1):
        raise ValueError("bad reading")
    data = r.get("data") or {}
    if not isinstance(data, dict) or len(json.dumps(data)) > MAX_DATA:
        raise ValueError("bad reading data")
    return {"t": t, "name": name, "label": label, "share": None if share is None else round(share, 4), "state": state,
            "data": alerts.check_details(data)}


def add(rows):
    """Keep readings until the sync sends them. Returns how many were kept (0 when readings aren't kept here).
    A forgotten person's are left out: a page that hasn't heard of the forget yet can't write them back."""
    if not isinstance(rows, list) or len(rows) > MAX_ROWS:
        raise ValueError("expected a short list of readings")
    rows = [check(r) for r in rows]
    if not rows or not enabled():
        return 0
    try:
        import dbsync_home
        gone = dbsync_home.forgotten_names()
    except ImportError:
        gone = set()
    by_day = {}
    for r in rows:
        if r["name"] not in gone:
            day = datetime.fromtimestamp(r["t"] / 1000, timezone.utc).strftime("%Y-%m-%d")
            by_day.setdefault(day, []).append(json.dumps(r, ensure_ascii=False))
    with _lock:
        folder().mkdir(parents=True, exist_ok=True)
        for day, lines in by_day.items():
            with open(folder() / f"{day}.jsonl", "a", encoding="utf-8") as f:
                f.write("".join(ln + "\n" for ln in lines))
    return sum(len(v) for v in by_day.values())


def _files():
    return sorted(folder().glob("*.jsonl")) if folder().is_dir() else []


def _rows(path):
    out = []
    for ln in path.read_text("utf-8").splitlines():
        try:
            o = json.loads(ln)
        except ValueError:
            continue
        if isinstance(o, dict) and o.get("name") and o.get("t") is not None:
            out.append(o)
    return out


def waiting(name=None):
    """Readings not sent yet (of one person, or everyone's)."""
    with _lock:
        return [o for f in _files() for o in _rows(f) if name is None or o["name"] == name]


def forget(name):
    """Drop name's waiting readings (persondata.forget; the table's go with the sync's forget). Returns how many."""
    n = 0
    with _lock:
        for f in _files():
            rows = _rows(f)
            keep = [o for o in rows if o["name"] != name]
            if len(keep) < len(rows):
                n += len(rows) - len(keep)
                paths.atomic_write(f, "".join(json.dumps(o, ensure_ascii=False) + "\n" for o in keep))
    return n


def send(conn, state, device):
    """The sync: put waiting readings into the database's expression_log. A reading is one row per (device, name,
    time), so sending a line twice changes nothing: a file that was rewritten (a person forgotten) is simply sent
    again. Past days' files go once all of them is in the database. Returns how many readings were sent."""
    today, sent = datetime.now(timezone.utc).strftime("%Y-%m-%d"), 0
    for f in _files():
        key = f"xlog:{f.name}"
        with _lock:
            try:
                size = f.stat().st_size
            except OSError:
                continue
            done = int(state.get(key, 0))
            if size < done:
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
                rows.append((device, o["name"], datetime.fromtimestamp(o["t"] / 1000, timezone.utc), o["label"],
                             o.get("share"), o.get("state") or "", json.dumps(o.get("data") or {})))
            except (ValueError, KeyError, TypeError, OverflowError, OSError):
                continue
        if rows:
            with conn.transaction():
                conn.cursor().executemany(
                    """INSERT INTO expression_log (device, name, at, label, share, state, data)
                       VALUES (%s, %s, %s, %s, %s, %s, %s) ON CONFLICT (device, name, at) DO NOTHING""", rows)
        if end:
            state.put(key, done + end)
            sent += len(rows)
        with _lock:                                  # a past day, all of it sent, nothing added for a minute: done
            try:
                st = f.stat()
                if f.stem < today and st.st_size == done + end and time.time() - st.st_mtime > 60:
                    f.unlink()
                    state.drop(key)
            except OSError:
                pass
    return sent


def export_to(path, name):
    """Write name's readings to path, one JSON object a line: those in the shared database (when it can be reached)
    and those still waiting here. Returns {"readings": n, "note": why some may be missing or None}."""
    n, note, seen = 0, None, set()
    with open(path, "w", encoding="utf-8") as out:
        try:
            import facedb
            if facedb.configured():
                with facedb.connect() as conn:
                    for dev, at, label, share, state, data in conn.execute(
                            """SELECT device::text, at, label, share, state, data FROM expression_log
                               WHERE name = %s ORDER BY at""", (name,)):
                        t = int(at.timestamp() * 1000)
                        seen.add((dev, t))
                        out.write(json.dumps({"t": t, "when": at.strftime("%Y-%m-%dT%H:%M:%SZ"), "device": dev,
                                              "label": label, "share": share, "state": state, "data": data},
                                             ensure_ascii=False) + "\n")
                        n += 1
        except Exception as e:                       # can't be reached now: say so in the export
            note = f"the shared database couldn't be read ({str(e).strip().splitlines()[0][:200] if str(e).strip() else type(e).__name__})"
        try:
            import facedb
            me = facedb.device()["id"]
        except Exception:
            me = None
        for o in waiting(name):
            if (me, o["t"]) not in seen:
                out.write(json.dumps({**{k: o.get(k) for k in ("t", "label", "share", "state", "data")},
                                      "device": me, "waitingToBeSent": True}, ensure_ascii=False) + "\n")
                n += 1
    return {"readings": n, "note": note}
