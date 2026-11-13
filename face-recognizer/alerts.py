"""
alerts.py - the tracker's alert inbox: every alert it raised (a chosen person arrived, left or had unusual signals; an unknown face stayed),
kept until you delete it, so an alert you didn't see when it happened is still there to look at.

Each alert is {id, t (epoch ms), kind ("arrived" | "left" | "unusual" | "unknown"), name, text, state, details}; state is "new" (not
looked at yet), "ack" (acknowledged: seen) or "archived" (put away). details holds what the tracker measured about the
face when the alert was raised ("first") and again 15 seconds later ("later"): recognition, expression, face cues,
voice, body, pulse and breathing, and how far each was from the person's normal. Numbers and short labels only, never
a picture or sound (check_details). The newest KEEP are kept: beyond that the oldest archived
go first, then the oldest of the rest. Stored in the private folder (%APPDATA%\\FaceLabels\\alerts.json, face_id.ALERTS),
outside the project like the presence log; it names people, so persondata.py exports and forgets it with the rest.
"""
import json
import re
import threading
import time
import uuid

import face_id as F
import paths

KINDS = ("arrived", "left", "unusual", "unknown", "moved")   # moved: from one device's camera to another's
STATES = ("new", "ack", "archived")
KEEP = 500
DETAIL_SETS = ("first", "later")
DETAILS_MAX = 20_000                             # bytes of JSON per set of details
_lock = threading.Lock()


def check_details(d, depth=0):
    """A set of details as the page sends it: objects of numbers, short strings (500 characters at most: an evidence
    line, never a picture), booleans and null, at most 4 deep
    and DETAILS_MAX bytes. Returns it (numbers rounded), or raises ValueError."""
    if depth == 0 and len(json.dumps(d)) > DETAILS_MAX:
        raise ValueError("details too large")
    if isinstance(d, bool) or d is None:
        return d
    if isinstance(d, (int, float)):
        if d != d or d in (float("inf"), float("-inf")):
            return None
        return round(d, 4) if isinstance(d, float) else d
    if isinstance(d, str):
        if len(d) > 500:
            raise ValueError("detail text too long")
        return d
    if isinstance(d, dict) and depth < 4:
        if not all(isinstance(k, str) and len(k) <= 40 for k in d):
            raise ValueError("bad detail name")
        return {k: check_details(v, depth + 1) for k, v in d.items()}
    raise ValueError("details must be objects of numbers and short text")


def _load():
    rows = paths.read_json(F.ALERTS, [])           # OSError if it's there and can't be read: never taken as empty
    return [r for r in rows if isinstance(r, dict) and r.get("id")] if isinstance(rows, list) else []


def _save(rows):
    F.ALERTS.parent.mkdir(parents=True, exist_ok=True)
    paths.atomic_write(F.ALERTS, json.dumps(rows, indent=1, ensure_ascii=False))
    for hook in F.SAVED_HOOKS:                   # the shared database's sync sends it within seconds
        hook()


def counts(rows=None):
    rows = _load() if rows is None else rows
    return {s: sum(r.get("state") == s for r in rows) for s in STATES}


def listing():
    """{"alerts": newest first, "counts": {state: n}}."""
    with _lock:
        rows = _load()
    return {"alerts": sorted(rows, key=lambda r: -r.get("t", 0)), "counts": counts(rows)}


def add(a):
    """Store one new alert from the tracker page ({kind, name, text, t}); checked. Returns the stored alert."""
    if not isinstance(a, dict) or a.get("kind") not in KINDS:
        raise ValueError("expected an alert of kind " + " or ".join(KINDS))
    name = str(a.get("name") or "")[:100]
    text = re.sub(r"\s+", " ", str(a.get("text") or ""))[:200].strip()
    t = int(a.get("t") or time.time() * 1000)
    if not text or (a["kind"] != "unknown" and (not name or name == "Unknown")):
        raise ValueError("bad alert")
    if abs(t - time.time() * 1000) > 7 * 86400_000:
        raise ValueError("alert time out of range")
    rec = {"id": uuid.uuid4().hex[:12], "t": t, "kind": a["kind"], "name": name if a["kind"] != "unknown" else "",
           "text": text, "state": "new"}
    if a.get("details") is not None:
        rec["details"] = {"first": check_details(a["details"])}
    with _lock:
        rows = _load()
        rows.append(rec)
        if len(rows) > KEEP:                     # oldest archived first, then oldest of the rest
            rows.sort(key=lambda r: (r.get("state") != "archived", r.get("t", 0)))
            rows = rows[len(rows) - KEEP:]
        _save(rows)
    return rec


def set_state(ids=None, state="ack", where=None):
    """Move alerts to state ("new" | "ack" | "archived"), or delete them (state "delete"): the ones listed in ids,
    or every alert now in state `where`. Returns the counts."""
    if state not in STATES + ("delete",):
        raise ValueError("unknown state")
    if where is not None and where not in STATES:
        raise ValueError("unknown state")
    if ids is not None and (not isinstance(ids, list) or not all(isinstance(i, str) for i in ids)):
        raise ValueError("ids must be a list")
    if ids is None and where is None:
        raise ValueError("say which alerts")
    pick = (lambda r: r["id"] in set(ids)) if ids is not None else (lambda r: r.get("state") == where)
    with _lock:
        rows = _load()
        if state == "delete":
            rows = [r for r in rows if not pick(r)]
        else:
            for r in rows:
                if pick(r):
                    r["state"] = state
        _save(rows)
    return counts(rows)


def add_details(alert_id, which, details):
    """Add a later set of details to an alert (the page measures again 15 s after raising it). Returns True if the
    alert is still there."""
    if which not in DETAIL_SETS or which == "first":
        raise ValueError("which must be 'later'")
    if not isinstance(alert_id, str):
        raise ValueError("bad alert id")
    details = check_details(details)
    with _lock:
        rows = _load()
        for r in rows:
            if r["id"] == alert_id:
                r.setdefault("details", {})[which] = details
                _save(rows)
                return True
    return False


def about(name):
    """The alerts naming this person (export)."""
    with _lock:
        return [r for r in _load() if r.get("name") == name]


def forget(name):
    """Delete the alerts naming this person; returns how many."""
    with _lock:
        rows = _load()
        keep = [r for r in rows if r.get("name") != name]
        if len(keep) < len(rows):
            _save(keep)
    return len(rows) - len(keep)
