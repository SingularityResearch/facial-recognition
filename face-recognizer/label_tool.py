"""
label_tool.py - the labeling page: confirm suggested names, name new people, fix mistakes, add photos.

Separate from the live tracker (tracker.py). Both share the data/ folder:
  reads/writes  data/faces.npz, data/photos.json, data/labels.json   (see labels.py)
  writes        data/people.npz   (what the tracker recognizes people from; it reloads it by itself)

Run:  python label_tool.py        (or double-click label.bat)
A page opens in your browser at http://127.0.0.1:8765/. Close this window when done.
Every change is saved immediately and can be undone.
"""
import argparse
import http.server
import json
import sys
import threading
import time
import webbrowser
from collections import OrderedDict
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import cv2
import numpy as np

import websearch
from localguard import refuse_reason

HERE = Path(__file__).resolve().parent
PAGE = HERE / "label_ui.html"
WEB_SAVED = "web_suggestions.json"     # in data/: Web search answers per face, so a face is searched (and paid for) once


def make_handler(db, importer):
    import face_id as F
    import labels as L
    photo_cache = OrderedDict()            # last few whole photos, already JPEG-encoded
    web_saved = F.load_json(WEB_SAVED, {})   # face ID -> its last complete Web search answer
    web_lock = threading.Lock()

    def photo_jpeg(h):
        if h in photo_cache:
            photo_cache.move_to_end(h)
            return photo_cache[h]
        p = db.store.photos.get(h, {}).get("path")
        img = F.load_image(p) if p and Path(p).exists() else None   # same scale as the stored face boxes
        if img is None:
            return None
        jpg = cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()
        photo_cache[h] = jpg
        while len(photo_cache) > 12:
            photo_cache.popitem(last=False)
        return jpg

    def search_picture(i, face):
        """JPEG to search the web with: the whole photo, or the face with hair, chin and some background
        around it (enlarged to at least 400 px, as search sites find small pictures poorly)."""
        jpg = photo_jpeg(db.store.photo[i])
        if jpg is None:
            return bytes(db.store.thumbs[i])     # original photo gone: the stored face is all there is
        if not face:
            return jpg
        img = cv2.imdecode(np.frombuffer(jpg, np.uint8), cv2.IMREAD_COLOR)
        x, y, w, h = (float(v) for v in db.store.box[i][:4])
        x0, y0 = max(0, int(x - w / 2)), max(0, int(y - h / 2))
        crop = img[y0:min(img.shape[0], int(y + h * 1.5)), x0:min(img.shape[1], int(x + w * 1.5))]
        k = max(1.0, 400 / max(crop.shape[:2]))
        if k > 1:
            crop = cv2.resize(crop, None, fx=k, fy=k, interpolation=cv2.INTER_CUBIC)
        return cv2.imencode(".jpg", crop, [cv2.IMWRITE_JPEG_QUALITY, 92])[1].tobytes()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def send(self, body, ctype="application/json", code=200, cache=False):
            if not isinstance(body, (bytes, bytearray)):
                body = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "max-age=31536000, immutable" if cache else "no-store")
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def refused(self, write):
            reason = refuse_reason(self.headers, self.server.server_address[1], write)
            if reason:
                self.send({"error": reason}, code=403)
            return reason

        def do_GET(self):
            if self.refused(write=False):
                return
            u = urlparse(self.path)
            q = {k: v[0] for k, v in parse_qs(u.query).items()}
            path, low = u.path, q.get("low") == "1"
            try:
                if path in ("/", "/index.html"):
                    return self.send(PAGE.read_bytes(), "text/html; charset=utf-8")
                if path.startswith("/thumb/"):
                    i = int(path[7:].split(".")[0])
                    if 0 <= i < db.n:
                        return self.send(db.store.thumbs[i], "image/jpeg", cache=True)   # URL carries the face ID
                    return self.send({"error": "no such face"}, code=404)
                if path.startswith("/photo/"):
                    i = int(path[7:].split(".")[0])
                    jpg = photo_jpeg(db.store.photo[i]) if 0 <= i < db.n else None
                    if jpg is None:
                        return self.send({"error": "photo not found"}, code=404)
                    return self.send(jpg, "image/jpeg", cache=True)
                with db.lock:
                    if path == "/api/summary":
                        s = db.summary()
                        s["app"] = "face-labels"
                        s["import"] = importer.status
                        return self.send(s)
                    if path == "/api/fids":                  # face index -> permanent ID (for image URLs)
                        return self.send({"fids": db.store.fid})
                    if path == "/api/suggested":
                        return self.send({"groups": db.suggested(low)})
                    if path == "/api/unknown":
                        return self.send(db.unknown(low, regroup=q.get("regroup") == "1"))
                    if path == "/api/auto":
                        return self.send({"groups": db.auto_review()})
                    if path == "/api/person":
                        return self.send({"name": q.get("name", ""), "faces": db.person(q.get("name", ""))})
                    if path == "/api/checks":
                        return self.send(db.checks())
                    if path == "/api/import":
                        return self.send(importer.status)
                    if path.startswith("/api/photoinfo/"):
                        i = int(path[15:])
                        h = db.store.photo[i]
                        p = db.store.photos.get(h, {})
                        others = [{"i": j, "box": [round(float(v)) for v in db.store.box[j][:4]], "name": db.name(j),
                                   "q": db.q(j)} for j in db.by_photo().get(h, [])]
                        path_ = p.get("path")
                        return self.send({"i": i, "path": path_ or p.get("name") or db.store.file[i],
                                          "exists": bool(path_ and Path(path_).exists()),
                                          "date": L.photo_date(path_) if path_ else "", "faces": others,
                                          "hints": dict(db.hints(i), web=web_saved.get(db.store.fid[i]))})
                    if path == "/api/websearch/key":
                        return self.send({"set": bool(websearch.key()), "services": websearch.services()})
                    if path == "/api/tagged":
                        return self.send({"groups": db.tagged(low)})
            except (ValueError, IndexError) as e:
                return self.send({"error": str(e)}, code=400)
            self.send({"error": "not found"}, code=404)

        def do_POST(self):
            if self.refused(write=True):
                return
            try:
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0) or 0)) or b"{}")
                if not isinstance(body, dict):
                    raise ValueError("expected an object")
                path = self.path.split("?", 1)[0]
                if path == "/api/act":
                    faces = body.get("faces")
                    if not (isinstance(faces, list) and all(isinstance(i, int) for i in faces)):
                        raise ValueError("faces must be a list of numbers")
                    n = db.act(str(body.get("action")), faces, body.get("name"))
                    return self.send({"ok": True, "changed": n, "undo": db.undo_stack[-1]["text"] if db.undo_stack else ""})
                if path == "/api/rename":
                    n = db.rename(str(body.get("old", "")), str(body.get("new", "")))
                    return self.send({"ok": True, "changed": n})
                if path == "/api/undo":
                    return self.send({"ok": True, "undone": db.undo()})
                if path == "/api/import":
                    importer.start(body.get("folder", ""))
                    return self.send({"ok": True})
                if path == "/api/import/stop":
                    importer.stop.set()
                    return self.send({"ok": True})
                if path == "/api/autoname":
                    return self.send({"ok": True, "named": db.autoname()})
                if path == "/api/websearch/key":        # optional Cloud Vision key, kept outside the project
                    websearch.save_key(body.get("key"))
                    return self.send({"ok": True, "set": bool(websearch.key()), "services": websearch.services()})
                if path == "/api/websearch":            # face to Amazon, photo to Google: only when you click
                    i = body.get("face")
                    if not (isinstance(i, int) and 0 <= i < db.n):
                        raise ValueError("no such face")
                    try:
                        fid = db.store.fid[i]
                        with web_lock:
                            saved = web_saved.get(fid)
                        if saved and not body.get("refresh"):   # asked before: no new searches, no cost
                            return self.send(dict(saved, cached=True, errors=[], usage={}))
                        r = websearch.suggest(search_picture(i, True), search_picture(i, False))
                        r["when"] = time.strftime("%Y-%m-%d %H:%M")
                        if not r["errors"]:                       # keep only complete answers
                            with web_lock:
                                web_saved[fid] = {k: r[k] for k in ("when", "suggestions", "checked", "details")}
                                F.save_json(WEB_SAVED, web_saved)
                        return self.send(r)
                    except (OSError, websearch.http.client.HTTPException) as e:
                        return self.send({"error": f"Couldn't reach the search site: {e}"}, code=502)
            except (ValueError, KeyError, TypeError) as e:
                return self.send({"error": str(e)}, code=400)
            self.send({"error": "not found"}, code=404)

    return Handler


def watch_webcam(importer, every=20):
    """Import the tracker's webcam snapshots (face_id.WEBCAM) by themselves: whenever new ones have
    arrived and no other import is running. The import names the faces it's sure of; the rest wait in
    Suggested and Unknown with the tracker's own verdict among the clues."""
    import face_id as F
    seen = 0.0
    while True:
        try:
            newest = max((p.stat().st_mtime for p in F.WEBCAM.glob("*.jpg")), default=0.0)
            if newest > seen and not importer.status.get("running"):
                importer.start(F.WEBCAM)
                seen = newest
        except (OSError, ValueError):
            pass                          # folder busy or another import started first: try next time
        time.sleep(every)


def attach(srv):
    """Load the faces and let srv (an HTTP server on 127.0.0.1) answer the labeling page; starts the
    background readers (photo dates and tags, webcam snapshots). Returns the LabelDB."""
    import labels as L
    db = L.LabelDB()
    importer = L.Importer(db)
    srv.RequestHandlerClass = make_handler(db, importer)
    db.facts.start()                       # camera dates and people tags, read in the background
    threading.Thread(target=watch_webcam, args=(importer,), daemon=True).start()
    return db


def already_running(port):
    """True if another copy of this tool is answering on the port."""
    import urllib.request
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/summary", timeout=3) as r:
            return json.loads(r.read()).get("app") == "face-labels"
    except Exception:
        return False


def main():
    ap = argparse.ArgumentParser(description="Label faces: confirm suggestions, name new people, add photos.")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--no-browser", action="store_true", help="don't open the browser automatically")
    a = ap.parse_args()

    srv, port = None, a.port
    for port in range(a.port, a.port + 10):
        try:
            srv = http.server.ThreadingHTTPServer(("127.0.0.1", port), http.server.BaseHTTPRequestHandler)
            break
        except OSError:
            if already_running(port):
                url = f"http://127.0.0.1:{port}/"
                print(f"Face Labels is already open at {url}. Opening it in your browser.")
                if not a.no_browser:
                    webbrowser.open(url)
                return
    if srv is None:
        sys.exit(f"Couldn't open a port between {a.port} and {a.port + 9}. Try: python label_tool.py --port 9000")

    print("Loading faces ...")
    db = attach(srv)
    url = f"http://127.0.0.1:{port}/"
    print(f"Face Labels is open at {url}  ({db.n} faces, model {db.store.model})")
    print("Leave this window open while labeling. Close it (or press Ctrl+C) when done.")
    if not a.no_browser:
        threading.Timer(0.5, lambda: webbrowser.open(url)).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
