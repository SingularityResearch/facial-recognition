r"""
persondata.py - everything the programs hold about one person: export it, or forget it.

The labeling tool's person view (**Their data…**) uses it through label_tool.py (/api/person/export and
/api/person/forget). Where a person's data can be (data\ is face_id.DATA; the private folder is
%APPDATA%\FaceLabels, paths.HOME):

  data\labels.json             the labels of their faces, and their name in other faces' "not this person" lists
  data\faces.npz, thumbs.npz   their faces' prints and pictures
  data\people.npz              their prints as the tracker loads them (made again from the labels)
  data\history.jsonl           labeling changes that name them
  data\backups\                earlier copies of labels.json, and the group files from before per-face labels
  data\web_suggestions.json    saved Web search answers for their faces, or naming them
  data\photo_facts.json        their name among a photo's people tags
  data\expr_eval\*.jsonl       Expression test frames and feeling reports with their name
  data\expr_calibration.json, va_fit.json, webcam_eval.json   numbers fitted or measured per person
  data\sheets\                 contact sheets of unnamed faces (`face_id.py cluster`), which may show them
  private\presence\*.jsonl     their visits (presence log)
  private\alerts.json          the tracker's alerts about them arriving (alerts.py)
  private\webcam\              Collect faces pictures of them, and the tracker's verdicts naming them
  private\descriptions.json    photo descriptions whose text names them
  private\app.log              FaceRecognizer.exe's messages (the tracker's "Known people" line before v1.9)
  private\person_signals.json  their usual look and their normal for each signal (devicesettings.py; the tracker
           page keeps a copy in its browser storage and forgets it itself: forgetPerson in tracker_ui.js, sent by
           the app window's tab after a forget, and at the page's next load or people check for a forget that came
           from another device)
  private\settings.json        their name in the tracker's alert list (ft-alertWho; the page clears it itself too)
  the shared database (facedb.py), when one is set up: every table above's copy, and the sightings (who each
           device's camera saw: names and times). A forget here is sent to it and
           to every other device (dbsync_home: the forgotten table), and clears what only the server holds.

Forgetting keeps their faces as Ignored faces without print or picture (only where each face is in its photo,
so the photos aren't read again and the faces don't come back as new faces), removes their name everywhere
above, and clears Undo. It can't be undone.
"""
import base64
import json
import re
import shutil
from pathlib import Path

import numpy as np

import alerts
import describe
import devicesettings
import face_id as F
import labels as L
import paths

WEB_SAVED = "web_suggestions.json"     # in data\: Web search answers by face ID (label_tool.WEB_SAVED)
FITTED = ("expr_calibration.json", "va_fit.json", "webcam_eval.json")   # in data\: numbers kept per person


def check_name(db, name):
    """name, if it is a person in the labels (named on a face, or in a "not this person" list); else ValueError."""
    name = str(name or "").strip()
    if not name or name.startswith("#"):
        raise ValueError("Give the name of a person.")
    if not any(r.get("n") == name or name in (r.get("not") or ()) for r in db.labels.values()):
        raise ValueError(f"Nobody is named {name}.")
    return name


def mentions(text, name):
    """True if text has name as a word (any case): "Ann" in "Ann and Bo", not in "Anna"."""
    return bool(re.search(r"(?<!\w)" + re.escape(name) + r"(?!\w)", str(text or ""), re.I))


def scrub(obj, name):
    """obj (anything JSON) without name: dict keys and values equal to it, and list items equal to it, removed."""
    if isinstance(obj, dict):
        return {k: scrub(v, name) for k, v in obj.items() if k != name and v != name}
    if isinstance(obj, list):
        return [scrub(v, name) for v in obj if v != name]
    return obj


def forget_label(rec, name):
    """A face's label with name forgotten: their face becomes Ignored, a "not" list loses it. None: no label."""
    r = dict(rec or {})
    if r.get("n") == name:
        r = {"n": L.IGNORE, "by": "me", "not": r.get("not")}
    if name in (r.get("not") or ()):
        r["not"] = [x for x in r["not"] if x != name]
    return {k: v for k, v in r.items() if v} or None


def _history_names(line, name):
    """True if a history.jsonl entry is about name (its text, or any face's label in it)."""
    if mentions(line.get("what", ""), name):
        return True
    return any(isinstance(r, dict) and (r.get("n") == name or name in (r.get("not") or ()))
               for r in (line.get("faces") or {}).values())


def _jsonl(path):
    """[(line text, parsed or None)] of a .jsonl file."""
    try:
        text = Path(path).read_text("utf-8")
    except OSError:
        return []
    out = []
    for s in text.splitlines():
        try:
            out.append((s, json.loads(s)))
        except ValueError:
            out.append((s, None))
    return out


def _drop_lines(path, about):
    """Rewrite a .jsonl file without the lines about(parsed line) is true for; returns how many went."""
    rows = _jsonl(path)
    keep = [s for s, o in rows if not (isinstance(o, dict) and about(o))]
    if len(keep) < len(rows):
        paths.atomic_write(path, "".join(s + "\n" for s in keep))
    return len(rows) - len(keep)


def _named(o, name):
    return o.get("name") == name


def _web_about(fids, name):
    return lambda fid, a: fid in fids or any(isinstance(s, dict) and s.get("name") == name
                                             for s in (a or {}).get("suggestions") or [])


def _their_faces(db, name):
    return [i for i in range(db.n) if db.name(i) == name]


def _snapshots(db, idx, name):
    """Collect faces pictures (.jpg, with their .json) of faces idx, or whose verdict names name."""
    pics = {Path(p) for p in (db.store.photos.get(db.store.photo[i], {}).get("path") for i in idx)
            if p and Path(p).parent == F.WEBCAM}
    if F.WEBCAM.is_dir():
        for js in F.WEBCAM.glob("*.json"):
            try:
                said = json.loads(js.read_text("utf-8"))
            except (OSError, ValueError):
                continue
            if any(isinstance(f, dict) and f.get("name") == name for f in said.get("faces") or []):
                pics.add(js.with_suffix(".jpg"))
    return sorted(pics)


# --------------------------------------------------------------------------- export
def export(db, name, folder, web_saved=None):
    """Write everything held about name into a new folder inside folder: data.json (every place in this module's
    list, with the face prints as base64 float32) and faces\\<face ID>.jpg (their face pictures) and webcam\\ (their
    Collect faces pictures). Returns {folder, faces, pictures}."""
    dest = Path(str(folder).strip().strip('"'))
    if not dest.is_absolute():
        raise ValueError("Give the full path of the folder, e.g. E:\\Documents\\Export")
    with db.lock:
        name = check_name(db, name)
        idx = _their_faces(db, name)
        fids = {db.store.fid[i] for i in idx}
        faces = []
        for i in idx:
            h = db.store.photo[i]
            p = db.store.photos.get(h, {})
            faces.append({"faceId": db.store.fid[i], "photo": p.get("path") or p.get("name") or db.store.file[i],
                          "photoHash": h, "videoSeconds": db.video_at(i),
                          "box": [round(float(v), 1) for v in db.store.box[i][:4]], "quality": db.q(i),
                          "label": db.rec(i), "picture": f"faces/{db.store.fid[i]}.jpg" if db.store.thumbs[i] else None,
                          "print": None if db.store.printless(i) else
                          base64.b64encode(np.asarray(db.store.emb[i], "<f4").tobytes()).decode("ascii")})
        pictures = {db.store.fid[i]: db.store.thumbs[i] for i in idx if db.store.thumbs[i]}
        not_them = [{"faceId": f, "label": r} for f, r in db.labels.items() if name in (r.get("not") or ())]
        with db.facts.lock:
            tags = [{"photo": db.store.photos.get(h, {}).get("path"), "tags": f.get("tags")}
                    for h, f in db.facts.facts.items() if name in (f.get("tags") or ()) or name in f.get("faces", {}).values()]
        snaps = _snapshots(db, idx, name)
    web = web_saved if web_saved is not None else F.load_json(WEB_SAVED, {})
    about_web = _web_about(fids, name)
    expr = F.DATA / "expr_eval"
    out = {
        "person": name,
        "exported": paths.utc_now(),
        "about": "Everything the face recognizer holds about this person on this computer (persondata.py). Face prints"
                 " are 512 float32 numbers, base64. Their usual look and normal signals are only in the tracker"
                 " page's browser storage, which this can't read.",
        "faces": faces,
        "markedNotThem": not_them,
        "history": [o for _, o in _jsonl(F.DATA / "history.jsonl") if isinstance(o, dict) and _history_names(o, name)],
        "webSearch": {fid: a for fid, a in dict(web).items() if about_web(fid, a)},
        "photoTags": tags,
        "expressionTests": {p.name: [o for _, o in _jsonl(p) if isinstance(o, dict) and _named(o, name)]
                            for p in sorted(expr.glob("*.jsonl"))},
        "fitted": {fn: _only(F.load_json(fn, None), name) for fn in FITTED if (F.DATA / fn).exists()},
        "presence": {p.name: [o for _, o in _jsonl(p) if isinstance(o, dict) and _named(o, name)]
                     for p in sorted(F.PRESENCE.glob("*.jsonl"))},
        "alerts": alerts.about(name),
        "descriptions": describe.find(lambda d: mentions(d.get("text"), name)),
        "webcam": [s.name for s in snaps],
        "personSignals": devicesettings.of(name),
        "inAlertList": name in [x.strip() for x in (devicesettings.settings("track").get("ft-alertWho") or "").split(",")],
    }
    out["expressionTests"] = {k: v for k, v in out["expressionTests"].items() if v}
    out["presence"] = {k: v for k, v in out["presence"].items() if v}
    safe = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", name).strip(" .") or "person"
    where = dest / f"{safe} - face recognizer data {paths.utc_stamp()}"
    (where / "faces").mkdir(parents=True, exist_ok=False)
    for fid, jpg in pictures.items():
        (where / "faces" / f"{re.sub(r'[^A-Za-z0-9_.-]', '_', fid)}.jpg").write_bytes(jpg)
    if snaps:
        (where / "webcam").mkdir()
        for s in snaps:
            for f in (s, s.with_suffix(".json")):
                if f.exists():
                    shutil.copy2(f, where / "webcam" / f.name)
    paths.atomic_write(where / "data.json", json.dumps(out, indent=1, ensure_ascii=False))
    return {"folder": str(where), "faces": len(faces), "pictures": len(pictures)}


def _only(obj, name):
    """The parts of a fitted-numbers file that are about name (a key named name, or a list holding it)."""
    if isinstance(obj, dict):
        got = {k: (v if k == name else _only(v, name)) for k, v in obj.items()}
        return {k: v for k, v in got.items() if k == name or v not in (None, {}, [])}
    if isinstance(obj, list):
        return [name] if name in obj else None
    return None


# --------------------------------------------------------------------------- forget
def forget(db, name, web_saved=None, web_lock=None, known=False):
    """Forget name everywhere in this module's list (but the browser storage) and return what was removed, by
    place. Their faces stay as Ignored faces without print or picture. Clears Undo; can't be undone.
    With a shared database the forget also goes to every other device (dbsync_home: the forgotten table), and
    known=True is how it arrives there: the name may already be gone from this device's labels."""
    done = {}
    with db.lock:
        name = str(name or "").strip() if known else check_name(db, name)
        if not name or name.startswith("#"):
            raise ValueError("Give the name of a person.")
        idx = _their_faces(db, name)
        fids = {db.store.fid[i] for i in idx}
        snaps = _snapshots(db, idx, name)
        changed = []
        for fid, r in list(db.labels.items()):
            new = forget_label(r, name)
            if new != r:
                changed.append(fid)
                if new:
                    db.labels[fid] = new
                else:
                    db.labels.pop(fid)
        db.undo_stack.clear()                    # an Undo could bring the name back
        if idx:
            db.forget_faces(idx)                 # prints and pictures; saves faces.npz and thumbs.npz
        done["faces"] = len(idx)
        done["labels"] = len(changed)
        done["history"] = _drop_lines(F.DATA / "history.jsonl", lambda o: _history_names(o, name))
        db._save(f"Forgot a person ({len(idx)} faces)", sorted(fids))   # labels.json, history line, people.npz
        with db.facts.lock:
            n = 0
            for f in db.facts.facts.values():
                if name in (f.get("tags") or ()) or name in f.get("faces", {}).values():
                    f["tags"] = [t for t in f.get("tags") or () if t != name]
                    f["faces"] = {i: nm for i, nm in f.get("faces", {}).items() if nm != name}
                    n += 1
        if n:
            db.facts.save()
        done["photoTags"] = n
    done["backups"] = _forget_backups(name)
    about_web = _web_about(fids, name)
    if web_saved is None:
        saved = F.load_json(WEB_SAVED, {})
        gone = [k for k, a in saved.items() if about_web(k, a)]
        if gone:
            F.save_json(WEB_SAVED, {k: a for k, a in saved.items() if k not in gone})
    else:
        lock = web_lock or _NoLock()
        with lock:
            gone = [k for k, a in web_saved.items() if about_web(k, a)]
            for k in gone:
                del web_saved[k]
            if gone:
                F.save_json(WEB_SAVED, web_saved)
    done["webSearch"] = len(gone)
    expr = F.DATA / "expr_eval"
    done["expressionTests"] = sum(_drop_lines(p, lambda o: _named(o, name)) for p in expr.glob("*.jsonl")) if expr.is_dir() else 0
    n = 0
    for fn in FITTED:
        old = F.load_json(fn, None) if (F.DATA / fn).exists() else None
        if old is not None and scrub(old, name) != old:
            F.save_json(fn, scrub(old, name))
            n += 1
    done["fitted"] = n
    sheets = list((F.DATA / "sheets").glob("*.jpg")) if (F.DATA / "sheets").is_dir() else []
    for s in sheets:                             # can't tell which show them; `face_id.py cluster` makes them again
        s.unlink(missing_ok=True)
    done["sheets"] = len(sheets)
    done["presence"] = sum(_drop_lines(p, lambda o: _named(o, name)) for p in F.PRESENCE.glob("*.jsonl")) \
        if F.PRESENCE.is_dir() else 0
    done["alerts"] = alerts.forget(name)
    for s in snaps:
        s.unlink(missing_ok=True)
        s.with_suffix(".json").unlink(missing_ok=True)
    done["webcam"] = len(snaps)
    done["descriptions"] = describe.drop(lambda d: mentions(d.get("text"), name))
    done["appLog"] = _forget_log(paths.HOME / "app.log", name)
    done["personSignals"] = devicesettings.forget(name)
    who = devicesettings.settings("track").get("ft-alertWho") or ""
    left = ", ".join(x.strip() for x in who.split(",") if x.strip() and x.strip() != name)
    if left != ", ".join(x.strip() for x in who.split(",") if x.strip()):
        devicesettings.update("track", {"ft-alertWho": left})
        done["alertList"] = 1
    try:                                     # the other devices forget them too (only with a shared database)
        import facedb
        if facedb.configured():
            import dbsync_home
            dbsync_home.note_forgotten(name)
            done["otherDevices"] = True
    except ImportError:
        pass
    return done


class _NoLock:
    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def _forget_backups(name):
    """data\\backups: labels-*.json copies get the same change as labels.json; other JSON files there (the old
    group files in legacy-*\\) lose the name wherever it appears. Returns how many files changed."""
    root = F.DATA / "backups"
    n = 0
    for p in sorted(root.rglob("*.json")) if root.is_dir() else []:
        try:
            old = json.loads(p.read_text("utf-8"))
        except (OSError, ValueError):
            continue
        if p.name.startswith("labels-") and isinstance(old, dict):
            new = {}
            for fid, r in old.items():
                r2 = forget_label(r, name) if isinstance(r, dict) else r
                if r2:
                    new[fid] = r2
        else:
            new = scrub(old, name)
        if new != old:
            paths.atomic_write(p, json.dumps(new, indent=1))
            n += 1
    # the one copy of faces.npz/thumbs.npz kept from before v1.9 deleted ignored faces' prints (labels.py) holds
    # this person's prints and pictures too: it goes
    for d in sorted(root.glob("before-v1.9-ignore-*")) if root.is_dir() else []:
        shutil.rmtree(d, ignore_errors=True)
        n += 1
    return n


def _forget_log(path, name):
    """Drop the lines of a log that name name. Rewritten in place: FaceRecognizer.exe keeps app.log open."""
    try:
        with open(path, "r+", encoding="utf-8", errors="replace") as f:
            lines = f.readlines()
            keep = [s for s in lines if not mentions(s, name)]
            if len(keep) < len(lines):
                f.seek(0)
                f.writelines(keep)
                f.truncate()
            return len(lines) - len(keep)
    except OSError:
        return 0
