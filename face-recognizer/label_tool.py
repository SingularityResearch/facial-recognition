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

import describe
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

    def photo_jpeg(i):
        """The whole photo face i was found in (for a video: the frame it was found in), as JPEG."""
        h = db.store.photo[i]
        rec = db.store.photos.get(h, {})
        frame = rec.get("at", {}).get(db.store.fid[i]) if rec.get("video") else None
        key = h if frame is None else (h, frame)
        if key in photo_cache:
            photo_cache.move_to_end(key)
            return photo_cache[key]
        p = rec.get("path")
        if not (p and Path(p).exists()):
            return None
        # same scale as the stored face boxes
        img = F.load_image(p) if frame is None else F.video_frame(p, frame)
        if img is None:
            return None
        jpg = cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()
        photo_cache[key] = jpg
        while len(photo_cache) > 12:
            photo_cache.popitem(last=False)
        return jpg

    def describe_key(i):
        """Descriptions are per photo (per frame for a video): every face in it shares one."""
        h = db.store.photo[i]
        rec = db.store.photos.get(h, {})
        frame = rec.get("at", {}).get(db.store.fid[i]) if rec.get("video") else None
        return h if frame is None else f"{h}:{frame}"

    def describe_picture(i):
        """The photo of face i at most 1024 px on its longer side, as JPEG, for the local model."""
        jpg = photo_jpeg(i)
        if jpg is None:
            raise ValueError("The original photo isn't on this computer any more.")
        img = cv2.imdecode(np.frombuffer(jpg, np.uint8), cv2.IMREAD_COLOR)
        s = min(1.0, 1024 / max(img.shape[:2]))
        if s < 1:
            img = cv2.resize(img, None, fx=s, fy=s, interpolation=cv2.INTER_AREA)
        return cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()

    def describe_all():
        """Describe every photo not described yet (not videos, duplicates or online-only cloud files) in the
        background; progress in importer.status so the page's progress bar shows it, and the page's Stop button
        stops it. One request to LM Studio at a time, the next photos loading meanwhile: with 3 at once, LM Studio's
        log showed generation falling from 48 to 1.3 tokens/s per request (Qwen 3.6 35B-A3B), 45 s a photo
        instead of about 3.5 s."""
        if importer.thread and importer.thread.is_alive():
            raise ValueError("An import is already running.")
        name = describe.model()                  # raises Unavailable with what's missing
        importer.stop.clear()

        def run():
            st = importer.status = dict(running=True, folder="photo descriptions", stage="Describing photos",
                                        total=0, done=0, new=0, known=0, duplicates=0, faces=0, auto=0, errors=0,
                                        started=time.time(), error=None)
            try:
                with db.lock:
                    todo = [(h, p["path"]) for h, p in db.store.photos.items()
                            if p.get("path") and not p.get("video") and not p.get("dup_of")]
                todo = [(h, p) for h, p in todo if not describe.saved(h)]
                st["total"] = len(todo)

                def load(item):                     # on 2 loader threads, a few photos ahead
                    h, path = item
                    if not Path(path).exists() or L.online_only(path):
                        return h, None
                    img = F.load_image(path)
                    if img is None:
                        raise OSError("unreadable")
                    sc = min(1.0, 1024 / max(img.shape[:2]))
                    if sc < 1:
                        img = cv2.resize(img, None, fx=sc, fy=sc, interpolation=cv2.INTER_AREA)
                    return h, cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()
                from concurrent.futures import ThreadPoolExecutor
                with ThreadPoolExecutor(2) as ex:
                    window = [ex.submit(load, it) for it in todo[:4]]
                    nxt = 4
                    while window and not importer.stop.is_set():
                        fu = window.pop(0)
                        if nxt < len(todo):
                            window.append(ex.submit(load, todo[nxt]))
                            nxt += 1
                        try:
                            h, jpg = fu.result()
                            if jpg is None:
                                st["known"] += 1
                            else:
                                describe.save(h, describe.ask(jpg, name), write=False)
                                st["new"] += 1
                        except describe.Unavailable as e:
                            st["errors"] += 1
                            if "isn't running" in str(e):
                                raise
                        except Exception:
                            st["errors"] += 1
                        st["done"] += 1
                        if st["done"] % 25 == 0:
                            describe.flush()
                    for fu in window:
                        fu.cancel()
                st["stage"] = "Stopped" if importer.stop.is_set() else "Done"
            except Exception as e:
                st["error"] = f"{type(e).__name__}: {e}"
                st["stage"] = "Failed"
            finally:
                describe.flush()
                st["running"] = False
                st["elapsed"] = round(time.time() - st["started"], 1)
        importer.thread = threading.Thread(target=run, daemon=True)
        importer.thread.start()

    def search_picture(i, face):
        """JPEG to search the web with: the whole photo, or the face with hair, chin and some background
        around it (enlarged to at least 400 px, as search sites find small pictures poorly)."""
        jpg = photo_jpeg(i)
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
                    jpg = photo_jpeg(i) if 0 <= i < db.n else None
                    if jpg is None:
                        return self.send({"error": "photo not found"}, code=404)
                    return self.send(jpg, "image/jpeg", cache=True)
                # "Describe photo" (describe.py): outside the database lock, as it asks LM Studio
                if path == "/api/describe/status":
                    with db.lock:
                        n = sum(1 for p in db.store.photos.values()
                                if p.get("path") and not p.get("video") and not p.get("dup_of"))
                    return self.send(dict(describe.status(), described=describe.count(), photos=n))
                if path.startswith("/api/describe/"):
                    i = int(path[14:])
                    if not 0 <= i < db.n:
                        return self.send({"error": "no such face"}, code=404)
                    return self.send({"saved": describe.saved(describe_key(i))})
                with db.lock:
                    if path == "/api/summary":
                        s = db.summary()
                        s["app"] = "face-labels"
                        s["import"] = importer.status
                        return self.send(s)
                    if path == "/api/stacks":                # same picture again: {face: first face of its stack}
                        return self.send({"n": db.n, "of": db.stacks()})
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
                    if path == "/api/possiblefaces":         # the Possible faces list
                        return self.send({"faces": db.possible(), "left": db.possible_left()})
                    if path.startswith("/api/possible/"):   # possible faces the import didn't keep (shown only)
                        return self.send({"faces": importer.possible_faces(int(path[14:]))})
                    if path.startswith("/api/photoinfo/"):
                        i = int(path[15:])
                        h = db.store.photo[i]
                        p = db.store.photos.get(h, {})
                        others = [{"i": j, "box": [round(float(v)) for v in db.store.box[j][:4]], "name": db.name(j),
                                   "q": db.q(j), "maybe": db.maybe(j), "shown": db.shown(j)} for j in db.by_photo().get(h, [])]
                        path_ = p.get("path")
                        others = [f for f in others if db.video_at(f["i"]) == db.video_at(i)]   # same video frame
                        return self.send({"i": i, "path": path_ or p.get("name") or db.store.file[i],
                                          "video": db.video_at(i),
                                          "exists": bool(path_ and Path(path_).exists()),
                                          "date": L.photo_date(path_) if path_ else "", "faces": others,
                                          "hints": dict(db.hints(i), web=web_saved.get(db.store.fid[i]))})
                    if path == "/api/websearch/key":
                        return self.send({"set": bool(websearch.key()), "services": websearch.services()})
                    if path == "/api/photos":                # photos with all of these people: ?with=A&with=B
                        names = parse_qs(u.query).get("with", [])
                        return self.send({"photos": db.photos_with(names)})
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
                if path == "/api/morefaces":              # look again for faces in one photo (photo view)
                    new = importer.find_more(int(body.get("i", -1)), body.get("box"))
                    return self.send({"ok": True, "new": new})
                if path == "/api/rename":
                    n = db.rename(str(body.get("old", "")), str(body.get("new", "")))
                    return self.send({"ok": True, "changed": n})
                if path == "/api/undo":
                    return self.send({"ok": True, "undone": db.undo()})
                if path == "/api/import":
                    importer.start(body.get("folder", ""))
                    return self.send({"ok": True})
                if path == "/api/drawface":               # a face drawn by hand in the photo view
                    j = importer.add_drawn(int(body.get("i", -1)), body.get("box") or [])
                    return self.send({"ok": True, "i": j})
                if path == "/api/possible/scan":            # look for possible faces in photos added before
                    importer.scan_possible()
                    return self.send({"ok": True})
                if path == "/api/import/stop":
                    importer.stop.set()
                    return self.send({"ok": True})
                if path == "/api/export":                  # copy the photos with these people into a folder
                    names = body.get("names")
                    if not (isinstance(names, list) and names and all(isinstance(n, str) for n in names)):
                        raise ValueError("names must be a list of names")
                    with db.lock:
                        rows = db.photos_with(names)
                    try:
                        n = db.export(rows, body.get("folder", ""))
                    except OSError as e:
                        raise ValueError(f"Couldn't copy into that folder: {e}")
                    return self.send({"ok": True, **n})
                if path == "/api/autoname":
                    return self.send({"ok": True, "named": db.autoname()})
                if path == "/api/describe/all":          # describe every photo not described yet (background)
                    try:
                        describe_all()
                    except describe.Unavailable as e:
                        raise ValueError(str(e))
                    return self.send({"ok": True})
                if path == "/api/describe":              # describe the photo with the local model (LM Studio)
                    i = body.get("i")
                    if not (isinstance(i, int) and 0 <= i < db.n):
                        raise ValueError("no such face")
                    k = describe_key(i)
                    d = describe.saved(k)
                    if d and not body.get("refresh"):
                        return self.send(dict(d, cached=True))
                    try:
                        d = describe.ask(describe_picture(i))
                    except describe.Unavailable as e:
                        raise ValueError(str(e))
                    return self.send(describe.save(k, d))
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
            st = importer.status
            # leave a folder import's result on screen for 5 minutes before this replaces it
            folder_done = st.get("folder") and Path(st["folder"]) != F.WEBCAM and \
                time.time() - st.get("started", 0) - (st.get("elapsed") or 0) < 300
            if newest > seen and not st.get("running") and not folder_done:
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
    def find_then_read():
        try:                               # photos that moved (e.g. a drive letter changed): find them first
            r = L.relocate(db.store, L.settings()["folders"], db.lock)
            if r["found"]:
                db.version += 1
                moved = "".join(f"\n  {o} -> {n}" for o, n in r["folders"].items())
                print(f"Found {r['found']} of {r['missing']} photos that had moved.{moved}")
            elif r["missing"]:
                print(f"{r['missing']} photos aren't where they were and weren't found in your photo folders.")
        except Exception as e:             # never stop the tool over this
            print(f"Couldn't look for moved photos: {e}")
        db.facts.start()                   # camera dates and people tags, read in the background
    threading.Thread(target=find_then_read, daemon=True).start()
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
