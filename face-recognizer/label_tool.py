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
import webbrowser
from collections import OrderedDict
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import cv2

from localguard import refuse_reason

HERE = Path(__file__).resolve().parent
PAGE = HERE / "label_ui.html"


def make_handler(db, importer):
    import face_id as F
    import labels as L
    photo_cache = OrderedDict()            # last few whole photos, already JPEG-encoded

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
                                          "date": L.photo_date(path_) if path_ else "", "faces": others})
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
            except (ValueError, KeyError, TypeError) as e:
                return self.send({"error": str(e)}, code=400)
            self.send({"error": "not found"}, code=404)

    return Handler


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

    import labels as L
    print("Loading faces ...")
    db = L.LabelDB()
    srv.RequestHandlerClass = make_handler(db, L.Importer(db))
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
