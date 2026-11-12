"""
tracker.py - live face tracker with name recognition, in your browser.

The browser page (tracker_ui.html) picks the camera, shows the video and draws the boxes.
This program does the recognition: the page sends it frames, it answers with faces, names and
(when models/facial_expression_recognition_mobilefacenet_2022july.onnx is installed) expressions,
plus face-shape cues when MediaPipe is installed; expression.js, served to the page, votes on them.
With body posture on, each face also gets its body points (MediaPipe pose) and whether a hand is on it.
When the page listens to the microphone it also sends short clips of speech, which the voice model
(models/wav2small.onnx) scores for arousal, dominance and valence; audio is never stored.
signals.js (motion, talking, blinks, pulse, posture, breathing, voice features) runs in the page.
It uses the same models and people as face_id.py, and picks up new names saved in
label_tool.py automatically.

Run:  python tracker.py        (or double-click recognize.bat)
Opens http://127.0.0.1:8766/ in your browser. Close this window to stop.
"""
import argparse
import base64
import json
import math
import re
import sys
import threading
import time
import webbrowser
from pathlib import Path
from urllib.parse import parse_qsl

import cv2
import numpy as np

import face_id as F
import alerts
import localguard
import paths

HERE = Path(__file__).resolve().parent
PAGE = HERE / "tracker_ui.html"
SCRIPTS = {"/expression.js": HERE / "expression.js", "/signals.js": HERE / "signals.js",
           "/settings_bridge.js": HERE / "settings_bridge.js", "/page_store.js": HERE / "page_store.js", "/tracker_ui.js": HERE / "tracker_ui.js",
           "/cameras_ui.js": HERE / "cameras_ui.js"}
CAMERAS_PAGE = HERE / "cameras_ui.html"            # the other devices' live cameras, in a grid (camstream.py, peer.py)
PEOPLE = F.DATA / "people.npz"
CALIBRATION = F.DATA / "expr_calibration.json"   # written by tools/expr_eval.py
VA_FIT = F.DATA / "va_fit.json"                   # written by tools/expr_eval.py from your feeling reports
MESH_TRIANGLES = F.mesh_triangles()                # face-mesh triangles for the page's "Face mesh" option


CAPTURE_MARGIN = 0.6     # room kept around a collected face, in face sizes: what face_id.rebuild_crop looks in to
                         # find a face again, so the labeling tool's import finds it in the saved picture too
CAPTURE_MIN = 160        # smaller pictures are enlarged to this (longer side), as rebuild_crop does to find small faces
OTHER_PAD = 0.25         # another face is greyed out over its box and this much more on each side


def _boxes(items, what, most=20):
    """[[x, y, w, h], ...] from the page's list ({box: [...]} or plain boxes), checked. Raises ValueError."""
    if not (isinstance(items, list) and len(items) <= most):
        raise ValueError(f"expected at most {most} {what}")
    out = []
    for f in items:
        box = f.get("box") if isinstance(f, dict) else f
        if not (isinstance(box, list) and len(box) == 4
                and all(isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v) for v in box)
                and box[2] > 0 and box[3] > 0):
            raise ValueError(f"each of the {what} needs a box [x, y, w, h]")
        out.append([float(v) for v in box])
    return out


def save_capture(jpeg, faces, others=()):
    """Collect faces: save each face the page asks for, for the labeling tool, as its own small picture: the face
    with room around it (CAPTURE_MARGIN), never the whole frame, and with every other face in that room greyed
    out (the other faces asked for, and others: the boxes of everyone else in view), so nobody else is saved.
    Each is <time>-<k>.jpg in F.WEBCAM plus <time>-<k>.json with the tracker's verdict ({when, faces: [{box,
    name, sim}]}, the box in that picture's pixels). Faces are boxes [x, y, w, h] in the frame's pixels with the
    tracker's name and match. Returns the saved names (stems)."""
    img = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_COLOR)
    if img is None:
        raise ValueError("not a JPEG image")
    boxes = _boxes(faces, "faces")
    if not boxes:
        raise ValueError("expected 1-20 faces")
    rest = _boxes(list(others), "other faces")
    H, W = img.shape[:2]
    when = paths.utc_now()
    now = time.time()
    base = f"webcam-{paths.utc_stamp(now)}-{int(now * 1000) % 1000:03d}"
    F.WEBCAM.mkdir(parents=True, exist_ok=True)
    stems = []
    for k, (f, (x, y, w, h)) in enumerate(zip(faces, boxes)):
        m = max(w, h) * CAPTURE_MARGIN
        x0, y0 = max(0, int(x - m)), max(0, int(y - m))
        x1, y1 = min(W, int(np.ceil(x + w + m))), min(H, int(np.ceil(y + h + m)))
        if x1 - x0 < 8 or y1 - y0 < 8:
            continue                              # outside the frame
        part = img[y0:y1, x0:x1].copy()
        for j, (ox, oy, ow, oh) in enumerate(boxes + rest):
            if j == k:
                continue
            px, py = ow * OTHER_PAD, oh * OTHER_PAD
            ax, ay = max(0, int(ox - px) - x0), max(0, int(oy - py) - y0)
            bx, by = min(x1, int(np.ceil(ox + ow + px))) - x0, min(y1, int(np.ceil(oy + oh + py))) - y0
            if bx > ax and by > ay:
                part[ay:by, ax:bx] = 128
        fx0, fy0 = max(0, int(x) - x0), max(0, int(y) - y0)    # this face itself stays, even where boxes overlap
        fx1, fy1 = min(x1, int(np.ceil(x + w))) - x0, min(y1, int(np.ceil(y + h))) - y0
        part[fy0:fy1, fx0:fx1] = img[y0 + fy0:y0 + fy1, x0 + fx0:x0 + fx1]
        s = max(1.0, CAPTURE_MIN / max(part.shape[:2]))
        if s > 1:
            part = cv2.resize(part, None, fx=s, fy=s, interpolation=cv2.INTER_CUBIC)
        verdict = {"box": [round((x - x0) * s, 1), round((y - y0) * s, 1), round(w * s, 1), round(h * s, 1)],
                   "name": str(f.get("name", "Unknown") if isinstance(f, dict) else "Unknown")[:100],
                   "sim": round(min(1.0, max(-1.0, float(f.get("sim", 0) if isinstance(f, dict) else 0))), 3)}
        stem = f"{base}-{k}"
        (F.WEBCAM / f"{stem}.json").write_text(json.dumps({"when": when, "faces": [verdict]}), encoding="utf-8")
        jpg = cv2.imencode(".jpg", part, [cv2.IMWRITE_JPEG_QUALITY, 92])[1].tobytes()
        (F.WEBCAM / f"{stem}.jpg").write_bytes(jpg)   # last: the labeling tool never sees a jpg without its json
        stems.append(stem)
    return stems


PRESENCE = F.PRESENCE                              # opt-in presence log: private, outside data/


def cameras():
    """{devices: [{id, name, me, streaming, share}], why}: every device of the shared database, and whether its
    camera streams now (a frame within the last half minute)."""
    try:
        import facedb
        with facedb.connect() as conn:
            me = facedb.device()["id"]
            rows = conn.execute("""SELECT id::text, name, peer_share, streaming_at > now() - interval '45 seconds',
                                          streaming_at FROM devices ORDER BY name""").fetchall()
        return {"devices": [{"id": i, "name": n, "me": i == me, "share": bool(sh), "streaming": bool(st),
                             "streamingAt": at.isoformat().replace("+00:00", "Z") if at else None}
                            for i, n, sh, st, at in rows], "why": ""}
    except Exception as e:
        return {"devices": [], "why": f"No shared database to find the other devices ({str(e)[:120]})"}


MAX_CAMERAS = 64                                   # cameras one Cameras page request may ask about
_DEVICE_ID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


def camera_ids(text):
    """{device id: number} from "id:n,id:n" (or bare ids), checked, at most MAX_CAMERAS."""
    out = {}
    for part in (text or "").split(",")[:MAX_CAMERAS]:
        d, _, n = part.partition(":")
        if _DEVICE_ID.fullmatch(d):
            out[d] = int(n) if n.isdigit() else 0
    return out


def camera_readouts(ids):
    """{device id: {faces, age, frameAge} or {faces: [], error}}: this device's from memory, the others' as their
    relays last got them (camrelay.py: the readout rides along each device's stream). Never waits on the network."""
    import camrelay
    import camstream
    import facedb
    me = facedb.device()["id"]
    return {d: camstream.readout() if d == me else camrelay.get(d).live_readout() for d in ids}


def camera_frames(after, watcher, wait=3.0, gather=0.04):
    """([(device id, seq, jpeg)], {device id: error}): each asked-for camera's newest frame after its number. Waits
    up to `wait` s for the first, then `gather` s more so frames of several cameras go in one answer."""
    import camrelay
    import camstream
    import facedb
    me = facedb.device()["id"]
    relays = {d: camrelay.get(d) for d in after if d != me}

    def collect():
        got = []
        for d, n in after.items():
            seq, jpg = camstream.peek(n, watcher) if d == me else relays[d].peek(n)
            if jpg is not None:
                got.append((d, seq, jpg))
        return got
    end = time.time() + wait
    with camstream.ANY:                    # held while looking, so a frame arriving meanwhile still wakes the wait
        got = collect()
        while not got and time.time() < end:
            camstream.ANY.wait(min(0.5, max(0.0, end - time.time())))
            got = collect()
        if got and gather > 0 and len(got) < len(after):
            camstream.ANY.wait(gather)
            got = collect()
    errors = {d: r.error for d, r in relays.items() if r.error and not any(g[0] == d for g in got)}
    return got, errors


def _forgotten():
    """People forgotten on any device (with a shared database): the page forgets what it learned about them."""
    try:
        import dbsync_home
        return sorted(dbsync_home.forgotten_names())
    except ImportError:
        return []


def log_visit(v):
    """Append one visit {name, start, end} (epoch ms) to PRESENCE/<day it started>.jsonl."""
    if not isinstance(v, dict):
        raise ValueError("expected a visit")
    name = str(v.get("name", ""))[:100]
    start, end = int(v["start"]), int(v["end"])
    if not name or name == "Unknown" or not 0 < start <= end or end - start > 7 * 86400_000:
        raise ValueError("bad visit")
    PRESENCE.mkdir(parents=True, exist_ok=True)
    day = time.strftime("%Y-%m-%d", time.localtime(start / 1000))
    with open(PRESENCE / f"{day}.jsonl", "a", encoding="utf-8") as f:
        f.write(json.dumps({"name": name, "start": start, "end": end}) + "\n")
    for hook in F.SAVED_HOOKS:                   # the shared database's sync sends it within seconds
        hook()


def visits(day):
    """The visits logged for day (YYYY-MM-DD), earliest first."""
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", day):
        raise ValueError("day must be YYYY-MM-DD")
    path = PRESENCE / f"{day}.jsonl"
    rows = []
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                rows.append(json.loads(line))
            except ValueError:
                pass
    return sorted(rows, key=lambda r: r.get("start", 0))


def _read_json(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


class Engine:
    """Detector + identifier shared by all requests.

    OpenCV's models are not thread-safe, so detection runs under a lock. The Identifier is
    read-only and replaced as a whole when people.npz changes, so each request takes one
    snapshot of it and passes its own threshold: two tabs with different strictness can't
    interfere with each other."""

    def __init__(self):
        self.faces = F.Faces()
        self.lock = threading.Lock()
        self.ident, self.people_mtime = None, None
        self.recorder = Recorder()
        self.voice = F.Voice()
        with self.lock:
            self._reload()
            self.warmup_ms = self.faces.warmup()

    def _reload(self):                         # call with self.lock held
        mtime = PEOPLE.stat().st_mtime if PEOPLE.exists() else None
        if mtime == self.people_mtime:
            return
        try:
            self.ident = F.Identifier.load() if mtime is not None else None
        except (OSError, ValueError, KeyError) as e:   # being replaced right now (Windows locks it): next request
            print(f"Couldn't read people.npz yet ({e}); trying again.")
            return
        self.people_mtime = mtime                  # only once it's loaded, so a failed read is tried again
        names = self.ident.names if self.ident else []
        # names on a console only: in FaceRecognizer.exe this goes to app.log, which would then hold them
        shown = ", ".join(names) if sys.stdout and sys.stdout.isatty() else f"{len(names)} named"
        print(f"Known people: {shown if names else 'none yet'}")

    def people(self):
        with self.lock:
            self._reload()
            return self.ident.names if self.ident else []

    def who(self, prints, threshold):
        """[name, similarity] for each face print: the page sends each face's recent prints averaged
        (weighted by face quality). Measured with tools/webcam_eval.py on webcam faces: 98% named
        correctly against 93% for voting on each frame's name, with no stranger named either way."""
        with self.lock:
            self._reload()
            ident = self.ident
        out = []
        for p in prints:
            e = np.asarray(p, np.float32)
            e = e / (np.linalg.norm(e) + 1e-9)
            name, sim = ident.who(e, threshold) if ident else ("Unknown", 0.0)
            out.append([name, round(float(sim), 3)])
        return out

    def recognize(self, jpeg, threshold, cues=False, pose=False, mesh=False, record=None, body=False, hands=False):
        """Faces in one webcam frame. cues=True adds the face-shape cues (about 10 ms a face on the
        CPU, so only when the page shows or uses them). mesh=True adds each face's 478 face-mesh
        points (same landmarker run as the cues; for drawing only). pose=True adds each face's body points and
        whether a hand is on the face (10-20 ms a frame). body=True adds all 33 body points (the full skeleton;
        the same pose run). hands=True adds each face's hands, 21 points each (fingers; another 10-20 ms; for
        drawing only). record=(session, posed label, extra) also appends the largest readable face's numbers to a
        posed-expression session file (see Recorder)."""
        img = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_COLOR)
        if img is None:
            raise ValueError("not a JPEG image")
        t0 = time.perf_counter()
        with self.lock:
            self._reload()
            ident = self.ident                 # snapshot: a reload mid-request can't swap it out
            found = self.faces.detect(img, sizes=(640,))   # one pass: webcam faces are large
            # Expression and cues only from faces the model can read: sharp enough, and facing the camera.
            skips = [F.expression_skip(row, quality) for row, _, _, quality in found]
            exprs = [self.faces.expression(crop) if skip is None else None
                     for (_, _, crop, _), skip in zip(found, skips)]
            # One landmarker run gives both: blendshapes from readable faces only, the mesh from any face
            meshes = [self.faces.face_mesh(img, row) if mesh or ((cues or record) and skip is None) else (None, None)
                      for (row, _, _, _), skip in zip(found, skips)]
            shapes = [bs if skip is None else None for (bs, _), skip in zip(meshes, skips)]
            poses = self.faces.poses(img) if (pose or body or record) and found else []
            hand_pts = self.faces.hands(img) if hands and found else []
        bodies = [F.pose_for_face(poses, row) for row, _, _, _ in found]
        their_hands = F.hands_for_faces(hand_pts, [row for row, _, _, _ in found], poses, bodies)
        out = []
        for (row, emb, crop, quality), skip, expr, bs, (_, pts), bi, hs in zip(found, skips, exprs, shapes, meshes, bodies,
                                                                              their_hands):
            name, sim = ident.who(emb, threshold) if ident else ("Unknown", 0.0)
            out.append({
                "box": [round(float(v), 1) for v in row[:4]],
                "kps": [[round(float(row[4 + 2 * k]), 1), round(float(row[5 + 2 * k]), 1)] for k in range(5)],
                "score": round(float(row[14]), 3),
                "name": name,
                "sim": round(float(sim), 3),
                # the face print (float32, base64): the page averages a face's recent prints and asks /api/who
                "emb": base64.b64encode(np.asarray(emb, "<f4").tobytes()).decode("ascii"),
                "quality": None if np.isnan(quality) else round(float(quality), 3),
                "turn": round(float(F.head_turn(row)), 2),
                "light": round(float(crop.mean()) / 255, 2),   # face brightness, for per-condition accuracy
                "expr": None if expr is None else [round(float(v), 3) for v in expr],   # EXPRESSIONS order
                "xskip": skip if self.faces.expression_model else None,   # why there's no reading
                "cues": F.face_cues(bs) if bs and cues else None,
                # flat [x, y, depth, x, y, depth, ...] in frame pixels; triangles from /api/people "meshTriangles"
                "mesh": np.round(pts, 1).ravel().tolist() if mesh and pts is not None else None,
                "pose": F.pose_points(poses[bi]) if bi is not None else None,   # F.POSE_POINTS order
                "hand": F.hand_on_face(poses[bi], row) if bi is not None else None,
                "body": F.pose_all(poses[bi]) if body and bi is not None else None,   # all 33, MediaPipe numbering
                "hands": hs if hands else None,
            })
        if record:
            readable = [(f, bs) for f, bs in zip(out, shapes) if f["expr"] is not None]
            if readable:
                f, bs = max(readable, key=lambda fb: fb[0]["box"][2] * fb[0]["box"][3])
                self.recorder.add(*record, f, bs)
        return {"faces": out, "ms": round((time.perf_counter() - t0) * 1000, 1),
                "width": img.shape[1], "height": img.shape[0]}


class Recorder:
    """Expression test data: the only expression data ever written to disk, and only when you start
    it on the tracker page. All of it is numbers (no images, no audio) in data/expr_eval/:

    <session>.jsonl  a posed test: you're asked to make each expression in turn (then to talk, and to
                     rest your chin on your hand); every readable frame appends the expression asked
                     for, whether you were asked to talk or to put a hand on your chin, the model's 7
                     probabilities, the 52 blendshapes, the body points and whether a hand is on the
                     face, quality, head turn, face size and brightness, and the page's talking signals.
    feel-<day>.jsonl feeling reports: when you answer "how do you feel?" (pleasant and energized,
                     1-7), the answer plus the page's summary of the 30 seconds before the question
                     (expression shares, cues, motion, posture, voice and pulse numbers).

    tools/expr_eval.py measures the model, the talking detector and the signals against them.
    "Delete test data" on the page (or deleting the folder) removes them."""

    DIR = F.DATA / "expr_eval"
    SESSION = re.compile(r"^\d{8}-\d{6}$")

    def __init__(self):
        self.lock = threading.Lock()

    def _append(self, stem, line):
        with self.lock:
            self.DIR.mkdir(exist_ok=True)
            with open(self.DIR / f"{stem}.jsonl", "a", encoding="utf-8") as fh:
                fh.write(json.dumps(line) + "\n")

    def add(self, session, posed, extra, face, blendshapes):
        line = {"t": round(time.time(), 2), "posed": posed, "name": face["name"], "quality": face["quality"],
                "turn": face["turn"], "size": round(face["box"][2]), "light": face["light"], "expr": face["expr"],
                "blendshapes": None if blendshapes is None else {k: round(v, 3) for k, v in blendshapes.items()},
                "pose": face["pose"], "hand": face["hand"], **extra}
        self._append(session, line)

    def report(self, rep):
        """One feeling report (already checked by check_report)."""
        self._append("feel-" + time.strftime("%Y%m%d", time.gmtime()), {"t": round(time.time(), 2), "kind": "report", **rep})

    @staticmethod
    def check_report(body):
        """The report the page sent, keeping only what belongs in it: valence and arousal 1-7, the
        name, and a flat dict of named numbers (or short lists of numbers). Raises ValueError."""
        rep = json.loads(body)
        if not isinstance(rep, dict):
            raise ValueError("bad report")
        out = {}
        for k in ("valence", "arousal"):
            v = rep.get(k)
            if not isinstance(v, int) or isinstance(v, bool) or not 1 <= v <= 7:
                raise ValueError(f"{k} must be 1-7")
            out[k] = v
        name = rep.get("name")
        if not isinstance(name, str) or not 0 < len(name) <= 80:
            raise ValueError("bad name")
        out["name"] = name
        feats = rep.get("features") or {}
        if not isinstance(feats, dict) or len(feats) > 120:
            raise ValueError("bad features")
        num = lambda v: isinstance(v, (int, float)) and not isinstance(v, bool) and abs(v) < 1e6   # noqa: E731
        clean = {}
        for k, v in feats.items():
            if not (isinstance(k, str) and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,39}", k)):
                raise ValueError("bad feature name")
            if v is None or num(v):
                clean[k] = v
            elif isinstance(v, list) and len(v) <= 16 and all(num(x) for x in v):
                clean[k] = v
            else:
                raise ValueError(f"feature {k} must be a number")
        out["features"] = clean
        return out

    def sessions(self):
        return sorted(p.stem for p in self.DIR.glob("*.jsonl") if self.SESSION.match(p.stem)) if self.DIR.exists() else []

    def reports(self):
        if not self.DIR.exists():
            return 0
        return sum(1 for p in self.DIR.glob("feel-*.jsonl") for line in p.read_text(encoding="utf-8").splitlines() if line.strip())

    def delete_all(self):
        with self.lock:
            for p in self.DIR.glob("*.jsonl") if self.DIR.exists() else []:
                p.unlink()
        return True


def make_handler(engine):

    class Handler(localguard.Handler):       # quiet, checked, errors answered: see localguard.Handler
        # beyond localguard.POLICY: the microphone's sound tap (an AudioWorklet module the page makes as a blob:
        # URL) and Open video (a file from this computer played through a blob: URL)
        policy = {"script-src": "'self' blob:", "media-src": "'self' blob:"}

        def _frames(self):
            """Every asked-for camera's next frame in one answer (?after=id:n,id:n): as soon as any has one newer
            than n (or after 3 s with none). The body: a 4-byte big-endian length, that many bytes of JSON
            {frames: [{id, seq, size}], errors: {id: why}}, then the frames' JPEGs one after another. One short
            request for all cameras: the page's connections don't grow with the number of cameras."""
            after = camera_ids(dict(parse_qsl(self.path.partition("?")[2])).get("after"))
            got, errors = camera_frames(after, f"page:{self.client_address[0]}")
            head = json.dumps({"frames": [{"id": d, "seq": s, "size": len(j)} for d, s, j in got],
                               "errors": errors}).encode()
            body = len(head).to_bytes(4, "big") + head + b"".join(j for _, _, j in got)
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            localguard.page_headers(self, self.frame_ancestors, self.policy)
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def _get(self):
            if self.refused(write=False):
                return
            path = self.path.split("?", 1)[0]
            if path in ("/", "/index.html"):
                return self.send(PAGE.read_bytes(), "text/html; charset=utf-8")   # re-read so edits show on refresh
            if path in SCRIPTS:
                return self.send(SCRIPTS[path].read_bytes(), "text/javascript; charset=utf-8")
            if path == "/cameras":
                return self.send(CAMERAS_PAGE.read_bytes(), "text/html; charset=utf-8")
            if path == "/api/cameras":                  # the devices whose cameras can be watched
                return self.send_json(cameras())
            if path == "/api/cameras/frames":           # every watched camera's next frame, in one request
                return self._frames()
            if path == "/api/cameras/readouts":         # their live readouts (faces, names, readings), as data
                ids = camera_ids(dict(parse_qsl(self.path.partition("?")[2])).get("ids"))
                return self.send_json(camera_readouts(ids))
            if path == "/api/presence":
                try:
                    day = dict(parse_qsl(self.path.partition("?")[2])).get("day") or time.strftime("%Y-%m-%d")
                    return self.send_json({"day": day, "visits": visits(day)})
                except ValueError as e:
                    return self.send_json({"error": str(e)}, 400)
            if path == "/api/alerts":                   # the alert inbox (alerts.py)
                return self.send_json(alerts.listing())
            if path == "/api/settings":                 # this page's settings and person signals (page_store.js)
                import devicesettings
                return self.send_json(devicesettings.page_state("track"))
            if path == "/api/people":
                devices = {**engine.faces.devices, "voice": engine.voice.device}
                return self.send_json({"people": engine.people(), "threshold": F.MATCH,
                                       "model": F.REC_NAME, "device": engine.faces.device,
                                       "devices": devices, "warmup_ms": engine.warmup_ms,
                                       "expressions": F.EXPRESSIONS if engine.faces.expression_model else None,
                                       "cues": engine.faces.cues_available, "voice": bool(engine.voice.model),
                                       "meshTriangles": MESH_TRIANGLES if engine.faces.cues_available else None,
                                       "qEnroll": F.Q_ENROLL, "captureDir": str(F.WEBCAM),
                                       "pose": engine.faces.pose_available, "posePoints": list(F.POSE_POINTS),
                                       "hands": engine.faces.hands_available,
                                       "calibration": _read_json(CALIBRATION), "vaFit": _read_json(VA_FIT),
                                       "sessions": len(engine.recorder.sessions()),
                                       "reports": engine.recorder.reports(), "forgotten": _forgotten()})
            self.send(b"not found", "text/plain", 404)

        def _post(self):
            if self.refused(write=True):
                return
            path, _, query = self.path.partition("?")
            try:
                if path == "/api/stream/frame":         # this camera as the page shows it, for the other devices
                    import camstream
                    import peer
                    if not peer.RUNNING or not peer.sharing():
                        return self.send_json({"stream": False})
                    why = self.headers.get("X-Stream-Failed")
                    if why:                             # the page's last frame didn't get here: why, in the log
                        print(f"Camera: a frame from the tracker page failed ({why[:200]})")
                    return self.send_json({"stream": True, "watchers": camstream.put(self.body(camstream.MAX_FRAME))})
                if path == "/api/inview":               # who this camera sees now (moved-between-cameras alerts)
                    import camstream
                    b = json.loads(self.body(camstream.MAX_READOUT + 30_000) or b"{}")
                    camstream.set_in_view(b.get("names") or [])
                    if b.get("readout") is not None:
                        camstream.set_readout(b["readout"])
                    return self.send_json({"ok": True})
                if path == "/api/settings":             # page_store.js: changed settings of this page
                    import devicesettings
                    body = json.loads(self.body(200_000) or b"{}")
                    if body.get("app") != "track":
                        raise ValueError("bad settings")
                    return self.send_json({"settings": devicesettings.update("track", body.get("settings") or {})})
                if path == "/api/signals":              # what was learned about each person (usual look, baselines)
                    import devicesettings
                    body = json.loads(self.body(devicesettings.MAX_SIGNALS + 1000) or b"{}")
                    kind = {v: k for k, v in devicesettings.KINDS.items()}.get(body.get("key"))
                    if kind is None:
                        raise ValueError("bad signals")
                    devicesettings.set_signals(kind, body.get("people") or {})
                    return self.send_json({"ok": True})
                if path == "/api/expr-eval/delete":
                    return self.send_json({"deleted": engine.recorder.delete_all()})
                if path == "/api/expr-eval/report":     # feeling report: see Recorder
                    engine.recorder.report(Recorder.check_report(self.body(200_000)))
                    return self.send_json({"saved": True, "reports": engine.recorder.reports()})
                if path == "/api/voice":                 # float32 samples at 16 kHz, 0.5-10 s; scored, not kept
                    raw = self.body(4 * F.VOICE_RATE * 10 + 1)
                    if len(raw) % 4 or len(raw) < 2 * F.VOICE_RATE:
                        raise ValueError("expected 0.5-10 s of float32 audio")
                    samples = np.frombuffer(raw, "<f4")
                    if not np.isfinite(samples).all():
                        raise ValueError("bad audio")
                    t0 = time.perf_counter()
                    avd = engine.voice.avd(samples)
                    return self.send_json({"avd": None if avd is None else [round(v, 3) for v in avd],
                                           "ms": round((time.perf_counter() - t0) * 1000, 1)})
                if path == "/api/alerts":                # one new alert the page raised, with what it measured
                    rec = alerts.add(json.loads(self.body(alerts.DETAILS_MAX + 5_000)))
                    return self.send_json({"alert": rec, "counts": alerts.counts()})
                if path == "/api/alerts/details":        # the second set of measurements, 15 s later
                    b = json.loads(self.body(alerts.DETAILS_MAX + 5_000))
                    if not isinstance(b, dict):
                        raise ValueError("expected an object")
                    return self.send_json({"saved": alerts.add_details(b.get("id"), b.get("which"), b.get("details"))})
                if path == "/api/alerts/state":          # acknowledge, archive, back to new, or delete
                    b = json.loads(self.body(100_000))
                    if not isinstance(b, dict):
                        raise ValueError("expected an object")
                    return self.send_json({"counts": alerts.set_state(b.get("ids"), b.get("state"), b.get("where"))})
                if path == "/api/presence":              # opt-in presence log: one finished visit
                    log_visit(json.loads(self.body(10_000)))
                    return self.send_json({"saved": True})
                if path == "/api/who":                   # averaged face prints -> names (see Engine.who)
                    body = json.loads(self.body(200_000))
                    prints = [np.frombuffer(base64.b64decode(p), "<f4") for p in body.get("prints", [])][:20]
                    if any(len(p) != F.REC["dim"] for p in prints):
                        raise ValueError(f"each print must have {F.REC['dim']} numbers")
                    threshold = min(0.9, max(0.1, float(body.get("t", F.MATCH))))
                    return self.send_json({"who": engine.who(prints, threshold)})
                if path == "/api/capture":               # "Collect faces": each face's picture for the labeling tool
                    q = dict(parse_qsl(query))
                    faces, others = json.loads(q.get("faces", "[]")), json.loads(q.get("others", "[]"))
                    return self.send_json({"saved": save_capture(self.body(20_000_000), faces, others)})
                if path != "/api/recognize":
                    return self.send(b"not found", "text/plain", 404)
                jpeg = self.body(20_000_000)
                params = dict(parse_qsl(query))
                threshold = min(0.9, max(0.1, float(params.get("t", F.MATCH))))
                record = None
                if "rec" in params:   # posed test: rec=<label>&session=<id>[&talk=1][&hand=1][&tv=<talk score>][&vad=0|1]
                    if params["rec"] not in F.EXPRESSIONS or not Recorder.SESSION.match(params.get("session", "")):
                        raise ValueError("bad test-session parameters")
                    extra = {"talk": params.get("talk") == "1", "handAsked": params.get("hand") == "1"}
                    if "tv" in params:
                        extra["tv"] = round(min(1.0, max(0.0, float(params["tv"]))), 4)
                    if params.get("vad") in ("0", "1"):
                        extra["vad"] = params["vad"] == "1"
                    record = (params["session"], params["rec"], extra)
                return self.send_json(engine.recognize(jpeg, threshold, cues=params.get("c") == "1",
                                                                       pose=params.get("p") == "1", mesh=params.get("m") == "1",
                                                       record=record, body=params.get("b") == "1",
                                                       hands=params.get("f") == "1"))
            except (ValueError, TypeError) as e:       # incl. localguard.RequestError: 400, or 413 when too large
                return self.send_json({"error": str(e)}, getattr(e, "code", 400))

    return Handler


def bind(port):
    """The tracker's web server socket on 127.0.0.1:port, not serving yet: attach() loads the models.
    OSError when the port is taken, usually by a tracker that's already running."""
    return localguard.bind(port)          # no address reuse on Windows: a second copy gets OSError


def attach(srv):
    """Load the models and let srv answer the tracker page. Returns the Engine."""
    F.require_models()
    engine = Engine()
    srv.RequestHandlerClass = make_handler(engine)
    return engine


def main():
    ap = argparse.ArgumentParser(description="Live face tracker with name recognition.")
    ap.add_argument("--port", type=int, default=8766)
    ap.add_argument("--no-browser", action="store_true")
    a = ap.parse_args()

    try:
        srv = bind(a.port)
    except OSError:
        url = f"http://127.0.0.1:{a.port}/"
        print(f"The tracker already seems to be running at {url}. Opening it.")
        if not a.no_browser:
            webbrowser.open(url)
        return
    engine = attach(srv)
    url = f"http://127.0.0.1:{a.port}/"
    print(f"Recognition model: {F.REC_NAME} on {engine.faces.device}")
    print(f"Expression model: {engine.faces.expression_model or 'not installed (optional: models/' + F.EXPR_MODEL.name + ')'}")
    if not engine.faces.cues_available:
        print(f"Face-shape cues: not installed (optional: pip install mediapipe, and models/{F.CUE_MODEL.name})")
    print(f"Voice model: {engine.voice.model or 'not installed (optional: python tools/get_wav2small.py)'}")
    if not engine.faces.pose_available:
        print(f"Body posture: not installed (optional: pip install mediapipe, and models/{F.POSE_MODEL.name})")
    print("Models: " + ", ".join(f"{k} on {v} ({engine.warmup_ms.get(k, '-')} ms first run)"
                                 for k, v in engine.faces.devices.items() if v))
    print(f"Face Tracker is open at {url}")
    print("Leave this window open while tracking. Close it (or press Ctrl+C) to stop.")
    if not a.no_browser:
        threading.Timer(0.5, lambda: webbrowser.open(url)).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
