"""Copy one device's existing data into the shared database (see facedb.py); dbsync.join runs it when a device
joins, tools/db_import.py from the command line. Reads only: nothing in DATA or HOME is
changed. Data already in the database is merged, not replaced: faces and photos are the same on every device (their
keys come from the photo), so a second device adds only what the first didn't have.

    python tools/db_import.py --data <data folder> --home <FaceLabels folder> [--device-id UUID] [--device-name NAME]

Defaults: this program's data/ and HOME, and this device (HOME/device.json). For another device's copied folders,
give that device's id (from its device.json), or a new one is made and printed: put it in that device's device.json
before it joins, so it keeps its own settings and paths.

Merging rules: a label named by hand beats an automatic one; between two of the same kind the one already in the
database stays (import the main computer first). A face Ignored anywhere stays Ignored (no print, no picture).
Keys (Web search, describe) aren't copied here: they're encrypted with the owner's passphrase when it's set up."""
import argparse
import json
import re
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

import numpy as np

import facedb
import paths

BATCH = 500
SHARED_DOCS = ("expr_calibration", "va_fit")               # about people, shared
DEVICE_DOCS = ("model_eval", "hw_bench", "webcam_eval")    # measured on that device's hardware and camera


def load_json(p, default):
    try:
        return json.loads(Path(p).read_text("utf-8"))
    except (OSError, ValueError):
        return default


def ts(text):
    """A stored time (UTC ISO, or older local-time text) -> aware datetime in UTC (None when unreadable)."""
    return paths.parse_time(text)


def utc_when(obj, key="when"):
    """A copy of obj with its time field as UTC ISO text (older files kept local time)."""
    if isinstance(obj, dict) and isinstance(obj.get(key), str):
        return dict(obj, **{key: paths.utc_text(obj[key])})
    return obj


def batches(rows, n=BATCH):
    for i in range(0, len(rows), n):
        yield rows[i:i + n]


def load_faces(data):
    """[(fid, photo, file, box, quality, model, print or None, picture or None)] from faces.npz + thumbs.npz."""
    p = data / "faces.npz"
    if not p.exists():
        return []
    z = np.load(p, allow_pickle=False)
    model = str(z["model"])
    emb = z["emb"].astype(np.float32)
    pics = {}
    t = data / "thumbs.npz"
    if t.exists():
        tz = np.load(t, allow_pickle=False)
        blob, off = tz["thumbs"].tobytes(), tz["thumb_off"]
        for k, fid in enumerate(tz["fid"]):
            b = blob[off[k]:off[k + 1]]
            if b:
                pics[str(fid)] = b
    out = []
    for i in range(len(z["fid"])):
        fid = str(z["fid"][i])
        e = emb[i]
        printless = not np.any(e)
        q = float(z["quality"][i])
        out.append((fid, str(z["photo"][i]), str(z["files"][i]), [float(v) for v in z["box"][i]],
                    None if np.isnan(q) else q, model, None if printless else e,
                    None if printless else pics.get(fid)))
    return out


def run(conn, data, home, dev):
    t0 = time.time()
    did = dev["id"]
    counts = {}
    conn.execute("SELECT set_config('facerec.device', %s, false)", (did,))
    with conn.transaction():
        conn.execute("""INSERT INTO devices (id, name, os) VALUES (%s, %s, 'imported')
                        ON CONFLICT (id) DO UPDATE SET name = excluded.name""", (did, dev["name"]))

    # photos + where this device has them + photo facts
    photos = load_json(data / "photos.json", {})
    facts = load_json(data / "photo_facts.json", {})
    rows, prows, possible, frames = [], [], {}, {}
    known = {"path", "size", "mtime", "fp", "faces", "name", "maybe", "dup_of", "video", "fps", "at"}
    for h, r in photos.items():
        f = facts.get(h) or {}
        fp = r.get("fp")
        rows.append((h, r.get("name") or Path(r.get("path") or "").name, r.get("size"),
                     bytes.fromhex(fp) if isinstance(fp, str) and re.fullmatch(r"[0-9a-f]+", fp) else None,
                     r.get("faces"), r.get("dup_of"), bool(r.get("video")), r.get("fps"), "maybe" in r,
                     f.get("t"), list(f.get("tags") or []), f.get("key"),
                     json.dumps({k: v for k, v in r.items() if k not in known}),
                     json.dumps({"t": f.get("t"), "tags": f.get("tags") or [], "faces": f.get("faces") or {}})
                     if f.get("key") else None))
        if r.get("path"):
            prows.append((did, h, r["path"], r.get("mtime")))
        for fid, s in (r.get("maybe") or {}).items():
            possible[fid] = float(s)
        for fid, fr in (r.get("at") or {}).items():
            frames[fid] = int(fr)
    fact_names = {fid: name for f in facts.values() for fid, name in (f.get("faces") or {}).items()}
    with conn.transaction():
        for b in batches(rows):
            conn.cursor().executemany("""
                INSERT INTO photos (hash, name, size, fp, faces, dup_of, video, fps, maybe_read, taken, tags,
                                    facts_key, extra, facts)
                VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                ON CONFLICT (hash) DO UPDATE SET
                    size = coalesce(photos.size, excluded.size), fp = coalesce(photos.fp, excluded.fp),
                    faces = greatest(photos.faces, excluded.faces), dup_of = coalesce(photos.dup_of, excluded.dup_of),
                    fps = coalesce(photos.fps, excluded.fps), maybe_read = photos.maybe_read OR excluded.maybe_read,
                    taken = coalesce(photos.taken, excluded.taken),
                    tags = ARRAY(SELECT DISTINCT unnest(photos.tags || excluded.tags)),
                    facts_key = coalesce(photos.facts_key, excluded.facts_key),
                    extra = excluded.extra || photos.extra,
                    facts = coalesce(photos.facts, excluded.facts)""", b)
        for b in batches(prows):
            conn.cursor().executemany("""
                INSERT INTO photo_paths (device, hash, path, mtime) VALUES (%s, %s, %s, %s)
                ON CONFLICT (device, hash) DO UPDATE SET path = excluded.path, mtime = excluded.mtime""", b)
    counts["photos"], counts["photo paths"] = len(rows), len(prows)

    # faces (prints as vectors, pictures)
    faces = load_faces(data)
    with conn.transaction():
        for b in batches(faces):
            conn.cursor().executemany("""
                INSERT INTO faces (fid, photo, file, box, quality, model, print, picture, possible, frame, fact_name)
                VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                ON CONFLICT (fid) DO UPDATE SET
                    quality = coalesce(faces.quality, excluded.quality),
                    picture = CASE WHEN faces.print IS NULL THEN NULL ELSE coalesce(faces.picture, excluded.picture) END,
                    possible = coalesce(faces.possible, excluded.possible),
                    frame = coalesce(faces.frame, excluded.frame),
                    fact_name = coalesce(faces.fact_name, excluded.fact_name)""",
                [(fid, ph, fl, box, q, m, e, pic, possible.get(fid), frames.get(fid), fact_names.get(fid))
                 for fid, ph, fl, box, q, m, e, pic in b])
        # a face Ignored (printless) on this device is Ignored everywhere
        gone = [fid for fid, *_rest, e, _pic in faces if e is None]
        for b in batches(gone):
            conn.execute("UPDATE faces SET print = NULL, picture = NULL WHERE fid = ANY(%s) AND print IS NOT NULL",
                         (b,))
    counts["faces"], counts["printless"] = len(faces), len(gone)

    # labels: by me beats auto; else the one already there stays; the names a face is not are joined
    labels = load_json(data / "labels.json", {})
    lrows = [(fid, v.get("n"), v.get("by"), v.get("s"), v.get("ok"), v.get("not")) for fid, v in labels.items()]
    with conn.transaction():
        for b in batches(lrows):
            conn.cursor().executemany("""
                INSERT INTO labels AS l (fid, n, by, s, ok, nope) VALUES (%s, %s, %s, %s, %s, %s)
                ON CONFLICT (fid) DO UPDATE SET
                    n = CASE WHEN l.n IS NULL OR (l.by = 'auto' AND excluded.by = 'me') THEN excluded.n ELSE l.n END,
                    by = CASE WHEN l.n IS NULL OR (l.by = 'auto' AND excluded.by = 'me') THEN excluded.by ELSE l.by END,
                    s = CASE WHEN l.n IS NULL OR (l.by = 'auto' AND excluded.by = 'me') THEN excluded.s ELSE l.s END,
                    ok = coalesce(l.ok, excluded.ok),
                    nope = (SELECT array_agg(DISTINCT x) FROM unnest(coalesce(l.nope, '{}') ||
                                                                      coalesce(excluded.nope, '{}')) x)""", b)
        conn.execute("""UPDATE faces SET print = NULL, picture = NULL WHERE print IS NOT NULL
                        AND fid IN (SELECT fid FROM labels WHERE n = '#ignore')""")
    counts["labels"] = len(lrows)

    # undo history log (each line once: the device's lines are only added when it has none yet)
    hist = []
    hp = data / "history.jsonl"
    if hp.exists():
        for line in hp.read_text("utf-8").splitlines():
            try:
                h = json.loads(line)
            except ValueError:
                continue
            hist.append((ts(h.get("t")) or datetime.now(timezone.utc), did, h.get("what", ""),
                         json.dumps(h.get("faces") or {})))
    with conn.transaction():
        if hist and not conn.execute("SELECT 1 FROM label_history WHERE device = %s LIMIT 1", (did,)).fetchone():
            for b in batches(hist):
                conn.cursor().executemany(
                    "INSERT INTO label_history (t, device, what, faces) VALUES (%s, %s, %s, %s)", b)
        else:
            hist = []
    counts["history lines"] = len(hist)

    # Web search results, descriptions (the first stays)
    sugg = load_json(data / "web_suggestions.json", {})
    desc = load_json(home / "descriptions.json", {})
    with conn.transaction():
        for b in batches(list(sugg.items())):
            conn.cursor().executemany("INSERT INTO web_suggestions (fid, data) VALUES (%s, %s) "
                                      "ON CONFLICT DO NOTHING", [(k, json.dumps(utc_when(v))) for k, v in b])
        for b in batches(list(desc.items())):
            conn.cursor().executemany("INSERT INTO descriptions (key, data) VALUES (%s, %s) "
                                      "ON CONFLICT DO NOTHING", [(k, json.dumps(utc_when(v))) for k, v in b])
    counts["web suggestions"], counts["descriptions"] = len(sugg), len(desc)

    # webcam snapshots: picture + the tracker's verdict
    snaps = []
    wc = home / "webcam"
    if wc.is_dir():
        for jp in sorted(wc.glob("webcam-*.jpg")):
            v = load_json(jp.with_suffix(".json"), None)
            snaps.append((jp.stem, did, ts((v or {}).get("when")), jp.read_bytes(),
                          json.dumps(utc_when(v)) if v is not None else None))
    with conn.transaction():
        for b in batches(snaps, 100):
            conn.cursor().executemany("INSERT INTO webcam_snapshots (name, device, taken, jpeg, verdict) "
                                      "VALUES (%s, %s, %s, %s, %s) ON CONFLICT DO NOTHING", b)
    counts["webcam snapshots"] = len(snaps)

    # alerts, presence log
    alerts = load_json(home / "alerts.json", [])
    pres = []
    pdir = home / "presence"
    if pdir.is_dir():
        for f in sorted(pdir.glob("*.jsonl")):
            for line in f.read_text("utf-8").splitlines():
                try:
                    v = json.loads(line)
                    pres.append((f"{did}:{int(v['start'])}:{v['name']}", did, v["name"], int(v["start"]),
                                 int(v["end"])))
                except (ValueError, KeyError, TypeError):
                    continue
    with conn.transaction():
        for b in batches([a for a in alerts if isinstance(a, dict) and a.get("id")]):
            conn.cursor().executemany("INSERT INTO alerts (id, device, data) VALUES (%s, %s, %s) "
                                      "ON CONFLICT DO NOTHING", [(str(a["id"]), did, json.dumps(a)) for a in b])
        for b in batches(pres):
            conn.cursor().executemany("INSERT INTO presence (id, device, name, start_ms, end_ms) "
                                      "VALUES (%s, %s, %s, %s, %s) ON CONFLICT DO NOTHING", b)
    counts["alerts"], counts["presence visits"] = len(alerts), len(pres)

    # posed expression tests and feeling reports
    ev = []
    edir = data / "expr_eval"
    if edir.is_dir():
        for f in sorted(edir.glob("*.jsonl")):
            for k, line in enumerate(f.read_text("utf-8").splitlines()):
                if line.strip():
                    try:
                        ev.append((f"{f.name}:{k}", did, f.name, json.dumps(json.loads(line))))
                    except ValueError:
                        continue
    with conn.transaction():
        for b in batches(ev):
            conn.cursor().executemany("INSERT INTO expr_eval (id, device, file, data) VALUES (%s, %s, %s, %s) "
                                      "ON CONFLICT DO NOTHING", b)
    counts["expression test lines"] = len(ev)

    # small documents
    docs = []
    for name in SHARED_DOCS:
        v = load_json(data / f"{name}.json", None)
        if v is not None:
            docs.append((name, None, name, json.dumps(v)))
    for name in DEVICE_DOCS:
        v = load_json(data / f"{name}.json", None)
        if v is not None:
            docs.append((f"{did}:{name}", did, name, json.dumps(v)))
    folders = load_json(data / "settings.json", {}).get("folders")
    if folders is not None:
        docs.append((f"{did}:photo_folders", did, "photo_folders", json.dumps(folders)))
    with conn.transaction():
        conn.cursor().executemany("INSERT INTO documents (id, device, name, body) VALUES (%s, %s, %s, %s) "
                                  "ON CONFLICT DO NOTHING", docs)
    counts["documents"] = len(docs)

    # Web search use this month (the larger count stays, so a cap is never undercounted)
    use = []
    for svc, fn in (("vision", "vision_usage.json"), ("rekognition", "rekognition_usage.json")):
        u = load_json(home / fn, None)
        if u and u.get("month"):
            use.append((svc, u["month"], int(u.get("used") or 0)))
    with conn.transaction():
        conn.cursor().executemany("""INSERT INTO usage (service, month, used) VALUES (%s, %s, %s)
                                     ON CONFLICT (service, month) DO UPDATE SET
                                     used = greatest(usage.used, excluded.used)""", use)

    # app.log: each line with the time of the start it belongs to (the device's lines only once)
    log, at = [], None
    lp = home / "app.log"
    if lp.exists():
        for line in lp.read_text("utf-8", errors="replace").splitlines():
            m = re.match(r"--- (\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\dZ?) ", line)
            if m:
                at = ts(m.group(1))
            if line.strip():
                log.append((did, at or datetime.now(timezone.utc), line))
    with conn.transaction():
        if log and not conn.execute("SELECT 1 FROM app_log WHERE device = %s LIMIT 1", (did,)).fetchone():
            for b in batches(log, 2000):
                conn.cursor().executemany("INSERT INTO app_log (device, at, line) VALUES (%s, %s, %s)", b)
        else:
            log = []
    counts["log lines"] = len(log)
    counts["seconds"] = round(time.time() - t0, 1)
    return counts


def check(conn, data):
    """Each local print against its database copy: the smallest cosine similarity (1.0 when copied exactly)."""
    faces = [f for f in load_faces(data) if f[6] is not None]
    if not faces:
        return None
    worst = 1.0
    for b in batches(faces, 1000):
        got = dict(conn.execute("SELECT fid, print FROM faces WHERE fid = ANY(%s) AND print IS NOT NULL",
                                ([f[0] for f in b],)).fetchall())
        for fid, *_r in b:
            if fid in got:
                a, d = _r[5], np.asarray(got[fid].to_numpy() if hasattr(got[fid], "to_numpy") else got[fid], dtype=np.float32)
                worst = min(worst, float(a @ d / (np.linalg.norm(a) * np.linalg.norm(d))))
    return worst


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--data", type=Path, default=paths.ROOT / "data")
    ap.add_argument("--home", type=Path, default=paths.HOME)
    ap.add_argument("--device-id")
    ap.add_argument("--device-name")
    a = ap.parse_args(argv)
    dev = load_json(a.home / "device.json", None) if a.device_id is None else None
    if a.device_id or not dev:
        dev = {"id": a.device_id or str(uuid.uuid4()), "name": a.device_name or "imported device"}
        if not a.device_id:
            print(f"New device id {dev['id']}: put it in that device's {facedb.DEVICE.name} before it joins.")
    if a.device_name:
        dev["name"] = a.device_name
    uuid.UUID(dev["id"])
    conn = facedb.connect()
    try:
        print("Schema migrations applied:", facedb.migrate(conn) or "none")
        counts = run(conn, a.data, a.home, dev)
        for k, v in counts.items():
            print(f"  {k}: {v}")
        w = check(conn, a.data)
        print(f"Prints checked: the smallest similarity to the database copy is {w} (below 1.0 where another"
              " device found the same face first: its own detector gives slightly different landmarks)")
    finally:
        conn.close()


if __name__ == "__main__":
    main()
