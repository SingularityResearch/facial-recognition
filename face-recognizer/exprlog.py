"""
exprlog.py - the expression log: what each named person's face looked like, every few seconds while a tracker sees
them, kept in the shared database (table expression_log, db/010) for reports later. The owner's decision of
2026-11-12; until then expression readings were shown and never stored.

A reading says what a face looked like to the model ("looks happy"), never what the person felt.

The tracker page sends its readings (tracker_ui.js: every EXPR_LOG_EVERY seconds, POST /api/expression-log). They
wait in HOME/expression/<UTC day>.jsonl until the sync has put them in the database (dbsync_home.HomePart.after_push
-> send); a day's waiting file goes once all of it is there, because the database is where the readings are kept.
The waiting and the sending are waitlog.py's, which the music log uses too; what a reading is, is here.
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
import time

import alerts
import forgotten
import waitlog

FOLDER = "expression"            # in HOME: readings waiting to be sent
OPTION = "ft-o-optExprLog"       # the tracker page's "Keep expression readings" (on unless switched off)
MAX_ROWS = 100                   # readings in one request
MAX_DATA = 4000                  # bytes of one reading's data
STATES = ("plain", "change", "usual", "unsure", "")


def _values(o):
    """A waiting reading as expression_log's columns after device, name and time. One without a label isn't sent."""
    return o["label"], o.get("share"), o.get("state") or "", json.dumps(o.get("data") or {})


# The readings waiting here, and how they reach the table. "xlog" names their files in the sync's state.
LOG = waitlog.WaitLog(FOLDER, "expression_log", ("label", "share", "state", "data"), _values, key="xlog")


def folder():
    return LOG.folder()


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
    gone = forgotten.names()
    return LOG.add([r for r in rows if r["name"] not in gone])


def waiting(name=None):
    """Readings not sent yet (of one person, or everyone's)."""
    return LOG.waiting(name)


def forget(name):
    """Drop name's waiting readings (persondata.forget; the table's go with the sync's forget). Returns how many."""
    return LOG.forget(name)


def send(conn, state, device):
    """The sync: put waiting readings into the database's expression_log. A reading is one row per (device, name,
    time), so sending a line twice changes nothing: a file that was rewritten (a person forgotten) is simply sent
    again. Past days' files go once all of them is in the database. Returns how many readings were sent."""
    return LOG.send(conn, state, device)


def export_to(path, name):
    """Write name's readings to path, one JSON object a line: those in the shared database (when it can be reached)
    and those still waiting here. Returns {"readings": n, "note": why some may be missing or None}."""
    n, note = LOG.export_to(path, name)
    return {"readings": n, "note": note}
