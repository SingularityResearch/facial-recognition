"""
tracker.py - live face tracker with name recognition, in your browser.

The browser page (tracker_ui.html) picks the camera, shows the video and draws the boxes.
This program does the recognition: the page sends it frames, it answers with faces, names and
(when models/facial_expression_recognition_mobilefacenet_2022july.onnx is installed) expressions,
plus face-shape cues when MediaPipe is installed; expression.js, served to the page, votes on them.
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
EXPR_JS = HERE / "expression.js"
PEOPLE = F.DATA / "people.npz"


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

    def recognize(self, jpeg, threshold, cues=False, record=None):
        """Faces in one webcam frame. cues=True adds the face-shape cues (about 10 ms a face on the
        CPU, so only when the page shows them). record=(session, posed label) also appends the
        largest readable face's numbers to a posed-expression session file (see Recorder)."""
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
            shapes = [self.faces.blendshapes(img, row) if (cues or record) and skip is None else None
                      for (row, _, _, _), skip in zip(found, skips)]
        out = []
        for (row, emb, _, quality), skip, expr, bs in zip(found, skips, exprs, shapes):
            name, sim = ident.who(emb, threshold) if ident else ("Unknown", 0.0)
            out.append({
                "box": [round(float(v), 1) for v in row[:4]],
                "kps": [[round(float(row[4 + 2 * k]), 1), round(float(row[5 + 2 * k]), 1)] for k in range(5)],
                "score": round(float(row[14]), 3),
                "name": name,
                "sim": round(float(sim), 3),
                "quality": None if np.isnan(quality) else round(float(quality), 3),
                "turn": round(float(F.head_turn(row)), 2),
                "expr": None if expr is None else [round(float(v), 3) for v in expr],   # EXPRESSIONS order
                "xskip": skip if self.faces.expression_model else None,   # why there's no reading
                "cues": F.face_cues(bs) if bs and cues else None,
            })
        if record:
            readable = [(f, bs) for f, bs in zip(out, shapes) if f["expr"] is not None]
            if readable:
                f, bs = max(readable, key=lambda fb: fb[0]["box"][2] * fb[0]["box"][3])
                self.recorder.add(*record, f, bs)
        return {"faces": out, "ms": round((time.perf_counter() - t0) * 1000, 1),
                "width": img.shape[1], "height": img.shape[0]}


class Recorder:
    """Posed-expression test sessions, the only expression data ever written to disk, and only when
    you start one on the tracker page. While it runs the page asks you to make each expression in
    turn; every readable frame appends one line of numbers (no image) to
    data/expr_eval/<session>.jsonl: the expression you were asked for, the model's 7 probabilities,
    the 52 blendshapes, quality, head turn and face size. tools/expr_eval.py measures the model and
    the cues against them. "Delete test sessions" on the page (or deleting the folder) removes them."""

    DIR = F.DATA / "expr_eval"
    SESSION = re.compile(r"^\d{8}-\d{6}$")

    def __init__(self):
        self.lock = threading.Lock()

    def add(self, session, posed, face, blendshapes):
        line = {"t": round(time.time(), 2), "posed": posed, "name": face["name"], "quality": face["quality"],
                "turn": face["turn"], "size": round(face["box"][2]), "expr": face["expr"],
                "blendshapes": None if blendshapes is None else {k: round(v, 3) for k, v in blendshapes.items()}}
        with self.lock:
            self.DIR.mkdir(exist_ok=True)
            with open(self.DIR / f"{session}.jsonl", "a", encoding="utf-8") as fh:
                fh.write(json.dumps(line) + "\n")

    def sessions(self):
        return sorted(p.stem for p in self.DIR.glob("*.jsonl")) if self.DIR.exists() else []

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
            if path == "/expression.js":
                return self.send(EXPR_JS.read_bytes(), "text/javascript; charset=utf-8")
            if path == "/api/people":
                return self.send_json({"people": engine.people(), "threshold": F.MATCH,
                                       "model": F.REC_NAME, "device": engine.faces.device,
                                       "devices": engine.faces.devices, "warmup_ms": engine.warmup_ms,
                                       "expressions": F.EXPRESSIONS if engine.faces.expression_model else None,
                                       "cues": engine.faces.cues_available,
                                       "sessions": len(engine.recorder.sessions())})
            self.send(b"not found", "text/plain", 404)

        def do_POST(self):
            if self.refused(write=True):
                return
            path, _, query = self.path.partition("?")
            if path == "/api/expr-eval/delete":
                return self.send_json({"deleted": engine.recorder.delete_all()})
            if path != "/api/recognize":
                return self.send(b"not found", "text/plain", 404)
            try:
                n = int(self.headers.get("Content-Length", 0) or 0)
                if not 0 < n < 20_000_000:
                    raise ValueError("empty or oversized frame")
                params = dict(parse_qsl(query))
                threshold = min(0.9, max(0.1, float(params.get("t", F.MATCH))))
                record = None
                if "rec" in params:                  # posed-expression test session: rec=<label>&session=<id>
                    if params["rec"] not in F.EXPRESSIONS or not Recorder.SESSION.match(params.get("session", "")):
                        raise ValueError("bad test-session parameters")
                    record = (params["session"], params["rec"])
                return self.send_json(engine.recognize(self.rfile.read(n), threshold,
                                                       cues=params.get("c") == "1", record=record))
            except (ValueError, TypeError) as e:
                return self.send_json({"error": str(e)}, 400)

    return Handler


def main():
    ap = argparse.ArgumentParser(description="Live face tracker with name recognition.")
    ap.add_argument("--port", type=int, default=8766)
    ap.add_argument("--no-browser", action="store_true")
    a = ap.parse_args()

    engine = Engine()
    if sys.platform == "win32":
        # Windows lets a second program bind a port that's already in use when SO_REUSEADDR is set
        # (Python's HTTP server sets it), and the old program keeps getting the connections. Without
        # it the bind fails as it should, and the "already running" branch below is taken.
        http.server.ThreadingHTTPServer.allow_reuse_address = False
    try:
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", a.port), make_handler(engine))
    except OSError:
        url = f"http://127.0.0.1:{a.port}/"
        print(f"The tracker already seems to be running at {url}. Opening it.")
        if not a.no_browser:
            webbrowser.open(url)
        return
    url = f"http://127.0.0.1:{a.port}/"
    print(f"Recognition model: {F.REC_NAME} on {engine.faces.device}")
    print(f"Expression model: {engine.faces.expression_model or 'not installed (optional: models/' + F.EXPR_MODEL.name + ')'}")
    if not engine.faces.cues_available:
        print(f"Face-shape cues: not installed (optional: pip install mediapipe, and models/{F.CUE_MODEL.name})")
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
