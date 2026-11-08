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
import http.server
import json
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
from localguard import refuse_reason

HERE = Path(__file__).resolve().parent
PAGE = HERE / "tracker_ui.html"
SCRIPTS = {"/expression.js": HERE / "expression.js", "/signals.js": HERE / "signals.js"}
PEOPLE = F.DATA / "people.npz"
CALIBRATION = F.DATA / "expr_calibration.json"   # written by tools/expr_eval.py
VA_FIT = F.DATA / "va_fit.json"                   # written by tools/expr_eval.py from your feeling reports
MESH_TRIANGLES = F.mesh_triangles()                # face-mesh triangles for the page's "Face mesh" option


def save_capture(jpeg, faces):
    """Save a webcam frame for the labeling tool: <time>.jpg in F.WEBCAM plus <time>.json with the
    tracker's verdict for each face it was saved for ([{box: [x, y, w, h], name, sim}], frame pixels)."""
    img = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_COLOR)
    if img is None:
        raise ValueError("not a JPEG image")
    if not (isinstance(faces, list) and 0 < len(faces) <= 20):
        raise ValueError("expected 1-20 faces")
    clean = []
    for f in faces:
        box = f.get("box") if isinstance(f, dict) else None
        if not (isinstance(box, list) and len(box) == 4 and all(isinstance(v, (int, float)) for v in box)):
            raise ValueError("each face needs a box [x, y, w, h]")
        clean.append({"box": [round(float(v), 1) for v in box], "name": str(f.get("name", "Unknown"))[:100],
                      "sim": round(min(1.0, max(-1.0, float(f.get("sim", 0)))), 3)})
    F.WEBCAM.mkdir(parents=True, exist_ok=True)
    stem = time.strftime("webcam-%Y%m%d-%H%M%S-") + f"{int(time.time() * 1000) % 1000:03d}"
    (F.WEBCAM / f"{stem}.json").write_text(json.dumps({"when": time.strftime("%Y-%m-%d %H:%M:%S"), "faces": clean}),
                                           encoding="utf-8")
    (F.WEBCAM / f"{stem}.jpg").write_bytes(jpeg)     # last, so the labeling tool never sees a jpg without its json
    return stem


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
        self.people_mtime = mtime
        self.ident = F.Identifier.load() if mtime is not None else None
        names = self.ident.names if self.ident else []
        print(f"Known people: {', '.join(names) if names else 'none yet'}")

    def people(self):
        with self.lock:
            self._reload()
            return self.ident.names if self.ident else []

    def recognize(self, jpeg, threshold, cues=False, pose=False, mesh=False, record=None):
        """Faces in one webcam frame. cues=True adds the face-shape cues (about 10 ms a face on the
        CPU, so only when the page shows or uses them). mesh=True adds each face's 478 face-mesh
        points (same landmarker run as the cues; for drawing only). pose=True adds each face's body points and
        whether a hand is on the face (10-20 ms a frame). record=(session, posed label, extra) also
        appends the largest readable face's numbers to a posed-expression session file (see Recorder)."""
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
            poses = self.faces.poses(img) if (pose or record) and found else []
        bodies = [F.pose_for_face(poses, row) for row, _, _, _ in found]
        out = []
        for (row, emb, crop, quality), skip, expr, bs, (_, pts), bi in zip(found, skips, exprs, shapes, meshes, bodies):
            name, sim = ident.who(emb, threshold) if ident else ("Unknown", 0.0)
            out.append({
                "box": [round(float(v), 1) for v in row[:4]],
                "kps": [[round(float(row[4 + 2 * k]), 1), round(float(row[5 + 2 * k]), 1)] for k in range(5)],
                "score": round(float(row[14]), 3),
                "name": name,
                "sim": round(float(sim), 3),
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
        self._append("feel-" + time.strftime("%Y%m%d"), {"t": round(time.time(), 2), "kind": "report", **rep})

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

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def send(self, body, ctype, code=200):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass                              # page was closed or reloaded mid-request

        def send_json(self, obj, code=200):
            self.send(json.dumps(obj).encode(), "application/json", code)

        def refused(self, write):
            reason = refuse_reason(self.headers, self.server.server_address[1], write)
            if reason:
                self.send_json({"error": reason}, 403)
            return reason

        def do_GET(self):
            if self.refused(write=False):
                return
            path = self.path.split("?", 1)[0]
            if path in ("/", "/index.html"):
                return self.send(PAGE.read_bytes(), "text/html; charset=utf-8")   # re-read so edits show on refresh
            if path in SCRIPTS:
                return self.send(SCRIPTS[path].read_bytes(), "text/javascript; charset=utf-8")
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
                                       "calibration": _read_json(CALIBRATION), "vaFit": _read_json(VA_FIT),
                                       "sessions": len(engine.recorder.sessions()),
                                       "reports": engine.recorder.reports()})
            self.send(b"not found", "text/plain", 404)

        def body(self, limit):
            n = int(self.headers.get("Content-Length", 0) or 0)
            if not 0 < n < limit:
                raise ValueError("empty or oversized request")
            return self.rfile.read(n)

        def do_POST(self):
            if self.refused(write=True):
                return
            path, _, query = self.path.partition("?")
            try:
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
                if path == "/api/capture":               # "Collect faces": frame for the labeling tool
                    faces = json.loads(dict(parse_qsl(query)).get("faces", "[]"))
                    return self.send_json({"saved": save_capture(self.body(20_000_000), faces)})
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
                                                       record=record))
            except (ValueError, TypeError) as e:
                return self.send_json({"error": str(e)}, 400)

    return Handler


def bind(port):
    """The tracker's web server socket on 127.0.0.1:port, not serving yet: attach() loads the models.
    OSError when the port is taken, usually by a tracker that's already running."""
    if sys.platform == "win32":
        # Windows lets a second program bind a port that's already in use when SO_REUSEADDR is set
        # (Python's HTTP server sets it), and the old program keeps getting the connections. Without
        # it the bind fails as it should, and the "already running" branch is taken.
        http.server.ThreadingHTTPServer.allow_reuse_address = False
    return http.server.ThreadingHTTPServer(("127.0.0.1", port), http.server.BaseHTTPRequestHandler)


def attach(srv):
    """Load the models and let srv answer the tracker page. Returns the Engine."""
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
