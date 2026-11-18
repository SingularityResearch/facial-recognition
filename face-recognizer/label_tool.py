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

import cudasupport
import describe
import label_settings
import lmstudio
import websearch
import localguard
import paths

HERE = Path(__file__).resolve().parent
PAGE = HERE / "label_ui.html"
# stands in for a face whose picture isn't on this computer (a copy of the project from GitHub has none)
NO_FACE = (b'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 112 112"><rect width="112" height="112" fill="#8884"/>'
           b'<circle cx="56" cy="44" r="20" fill="#8888"/><path d="M18 104c4-24 20-34 38-34s34 10 38 34z" fill="#8888"/>'
           b'<title>No picture of this face on this computer</title></svg>')
MAX_BODY = 2_000_000                   # bytes in a request (a list of 100,000 face numbers is about 0.7 MB)
WEB_SAVED = "web_suggestions.json"     # in data/: Web search answers per face, so a face is searched (and paid for) once
                                       # (persondata.WEB_SAVED: exported and forgotten with a person)


def make_handler(db, importer):
    import describejob
    import face_id as F
    import labels as L
    import persondata
    photo_cache = OrderedDict()            # last few whole photos, already JPEG-encoded
    cache_lock = threading.Lock()          # requests come in on several threads
    no_thumb = set()                       # faces whose picture couldn't be remade (this run)
    remake = []                            # faces waiting for their picture to be remade (newest asked first)
    remake_cv = threading.Condition()
    # face ID -> its last complete Web search answer (attach() puts it on db, for the shared database's sync)
    web_saved = db.web_saved if hasattr(db, "web_saved") else F.load_json(WEB_SAVED, {})
    web_lock = getattr(db, "web_lock", None) or db.lock

    def photo_jpeg(i):
        """The whole photo face i was found in (for a video: the frame it was found in), as JPEG."""
        h = db.store.photo[i]
        rec = db.store.photos.get(h, {})
        frame = rec.get("at", {}).get(db.store.fid[i]) if rec.get("video") else None
        key = h if frame is None else (h, frame)
        with cache_lock:
            if key in photo_cache:
                photo_cache.move_to_end(key)
                return photo_cache[key]
        p = rec.get("path")
        if not (p and Path(p).exists()):
            jpg = from_peer(h, frame)                # another device that has it shows it to this one (peer.py)
            if jpg is None:
                return None
        else:
            # same scale as the stored face boxes
            img = F.load_image(p) if frame is None else F.video_frame(p, frame)
            if img is None:
                return None
            jpg = cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()
        with cache_lock:
            photo_cache[key] = jpg
            while len(photo_cache) > 12:
                photo_cache.popitem(last=False)
        return jpg

    def from_peer(h, frame):
        """The original from another device of the shared database (shown, kept only in photo_cache), or None."""
        if not peer_ready():
            return None
        import peer
        jpg, _who = peer.fetch(h, frame)
        return jpg

    def peer_ready():
        sync = getattr(db, "sync", None)
        return sync is not None and sync.info.get("state") == "online"

    def remote_holders(h):
        """Names of the other sharing devices that have photo h (for the photo view), [] when it can't be asked."""
        if not peer_ready():
            return []
        try:
            import facedb
            import peer
            with facedb.connect() as conn:
                return [r[0] for r in peer.holders(conn, h)]
        except Exception:
            return []

    job = describejob.DescribeJob(db, importer, photo_jpeg, peer_ready)     # Describe all, Describe everything again

    def face_thumb(i, wait=False):
        """Face i's picture as JPEG, or None. A copy of the project from GitHub has no face pictures
        (data/thumbs.npz stays on the computer it was made on): a missing one is made again from the original
        photo when that's here (about 0.5 s each) - in the background, so a list of faces doesn't hold up the
        page (it shows a placeholder until the list is drawn again); wait=True makes it now."""
        t = db.store.thumbs[i]
        if t or i in no_thumb or db.printless(i):    # Ignored faces' pictures are deleted on purpose: not remade
            return t or None
        if wait:
            return remake_one(i)
        with remake_cv:
            if i not in remake:
                remake.append(i)
                remake_cv.notify()
        return None

    def remake_one(i):
        if db.store.thumbs[i] or i in no_thumb or db.printless(i):
            return db.store.thumbs[i] or None
        try:
            crop = F.rebuild_crop(db.store, i)
        except Exception:
            crop = None
        if crop is None:
            no_thumb.add(i)
            return None
        with db.lock:
            db.store.set_thumb(i, crop)
        return db.store.thumbs[i]

    def remaker():
        while True:
            with remake_cv:
                while not remake:
                    remake_cv.wait()
                i = remake.pop()
            remake_one(i)
    threading.Thread(target=remaker, daemon=True, name="face pictures").start()

    def search_picture(i, face):
        """JPEG to search the web with: the whole photo, or the face with hair, chin and some background
        around it (enlarged to at least 400 px, as search sites find small pictures poorly)."""
        jpg = photo_jpeg(i)
        if jpg is None:
            t = face_thumb(i, wait=True)         # original photo gone: the stored face is all there is
            if t is None:
                raise ValueError("Neither the photo nor a picture of this face is on this computer.")
            return t
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

    class Handler(localguard.Handler):       # quiet, checked, errors answered: see localguard.Handler
        def _get(self):
            u = urlparse(self.path)
            if self.refused(write=False, own=u.path.startswith(localguard.OWN_ONLY)):
                return
            q = {k: v[0] for k, v in parse_qs(u.query).items()}
            path, low = u.path, q.get("low") == "1"
            try:
                if path in ("/", "/index.html"):
                    return self.send(PAGE.read_bytes(), "text/html; charset=utf-8")
                if path in ("/label_ui.js", "/settings_bridge.js", "/page_store.js", "/ask.js"):   # the page's scripts
                    return self.send((HERE / path[1:]).read_bytes(), "text/javascript; charset=utf-8")
                if path == "/favicon.ico":
                    return self.favicon()
                if path.startswith("/thumb/"):
                    i = int(path[7:].split(".")[0])
                    if 0 <= i < db.n:
                        t = face_thumb(i)
                        if t is None:                    # not cached: the picture may turn up (thumbs.npz copied in)
                            return self.send(NO_FACE, "image/svg+xml")
                        return self.send(t, "image/jpeg")
                    return self.send({"error": "no such face"}, code=404)
                if path.startswith("/photo/"):
                    i = int(path[7:].split(".")[0])
                    jpg = photo_jpeg(i) if 0 <= i < db.n else None
                    if jpg is None:
                        return self.send({"error": "photo not found"}, code=404)
                    return self.send(jpg, "image/jpeg")
                # "Describe photo" (describe.py): outside the database lock, as it asks LM Studio
                if path == "/api/describe/status":
                    with db.lock:
                        n = sum(1 for p in db.store.photos.values()
                                if p.get("path") and not p.get("video") and not p.get("dup_of"))
                    st = describe.status()
                    return self.send(dict(st, described=describe.count(), photos=n, again=job.again_state(),
                                          secs=describe.typical_secs(st.get("model")) if st.get("ok") else None))
                if path.startswith("/api/describe/"):
                    i = int(path[14:])
                    if not 0 <= i < db.n:
                        return self.send({"error": "no such face"}, code=404)
                    return self.send({"saved": describe.saved(job.key(i))})
                if label_settings.get(self, path, db) is not False:     # what the Settings tab asks: its own module
                    return
                if path == "/api/settings":              # this page's settings (page_store.js)
                    import devicesettings
                    return self.send(devicesettings.page_state("label"))
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
                    if path == "/api/downloads":               # what is downloading now (the header's bar)
                        return self.send({"items": downloads()})
                    if path == "/api/possiblefaces":         # the Possible faces list
                        return self.send({"faces": db.possible(), "left": db.possible_left()})
                    if path.startswith("/api/possible/"):   # possible faces the import didn't keep (shown only)
                        i = int(path[14:])
                        if not 0 <= i < db.n:
                            raise IndexError("no such face")
                        return self.send({"faces": importer.possible_faces(i)})
                    if path.startswith("/api/photoinfo/"):
                        i = int(path[15:])
                        if not 0 <= i < db.n:
                            raise IndexError("no such face")
                        h = db.store.photo[i]
                        p = db.store.photos.get(h, {})
                        others = [{"i": j, "box": [round(float(v)) for v in db.store.box[j][:4]], "name": db.name(j),
                                   "q": db.q(j), "maybe": db.maybe(j), "shown": db.shown(j)} for j in db.by_photo().get(h, [])]
                        path_ = p.get("path")
                        others = [f for f in others if db.video_at(f["i"]) == db.video_at(i)]   # same video frame
                        here = bool(path_ and Path(path_).exists())
                        return self.send({"i": i, "path": path_ or p.get("name") or db.store.file[i],
                                          "video": db.video_at(i),
                                          "exists": here, "remote": [] if here else remote_holders(h),
                                          "date": L.photo_date(path_) if path_ else "", "faces": others,
                                          "hints": dict(db.hints(i), web=web_saved.get(db.store.fid[i]))})
                    if path == "/api/photos":                # photos with all of these people: ?with=A&with=B
                        names = parse_qs(u.query).get("with", [])
                        return self.send({"photos": db.photos_with(names)})
                    if path == "/api/tagged":
                        return self.send({"groups": db.tagged(low)})
            except (ValueError, IndexError) as e:
                return self.send({"error": str(e)}, code=400)
            self.send({"error": "not found"}, code=404)

        def _post(self):
            if self.refused(write=True):
                return
            try:
                body = json.loads(self.body(MAX_BODY, empty=True) or b"{}")
                if not isinstance(body, dict):
                    raise ValueError("expected an object")
                path = self.path.split("?", 1)[0]
                if label_settings.post(self, path, body, db) is not False:   # what the Settings tab asks
                    return
                if path == "/api/act":
                    faces = body.get("faces")
                    if not (isinstance(faces, list) and all(isinstance(i, int) for i in faces)):
                        raise ValueError("faces must be a list of numbers")
                    n = db.act(str(body.get("action")), faces, body.get("name"))
                    # skipped: Ignored faces whose print and picture couldn't be made again (LabelDB.act)
                    return self.send({"ok": True, "changed": n, "skipped": len(set(faces)) - n,
                                      "undo": db.undo_stack[-1]["text"] if db.undo_stack else ""})
                if path == "/api/morefaces":              # look again for faces in one photo (photo view)
                    new = importer.find_more(face_index(body), body.get("box"))
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
                    j = importer.add_drawn(face_index(body), body.get("box") or [])
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
                        raise ValueError(f"Couldn't copy into that folder: {e}") from e
                    return self.send({"ok": True, **n})
                if path == "/api/person/export":          # everything held about one person, into a folder
                    try:
                        r = persondata.export(db, body.get("name"), body.get("folder", ""), web_saved)
                    except OSError as e:
                        raise ValueError(f"Couldn't write into that folder: {e}") from e
                    return self.send({"ok": True, **r})
                if path == "/api/person/forget":          # ... or forget them everywhere (confirmed on the page)
                    if body.get("confirm") != body.get("name"):
                        raise ValueError("Type the person's name to confirm.")
                    return self.send({"ok": True, "removed": persondata.forget(db, body.get("name"), web_saved, web_lock)})
                if path == "/api/describe/all":          # describe every photo not described yet (background)
                    try:
                        job.start()
                    except describe.Unavailable as e:
                        raise ValueError(str(e)) from e
                    return self.send({"ok": True})
                if path == "/api/describe/again":        # Settings: describe everything again, or carry on with it
                    try:
                        job.start("on" if body.get("carry_on") is True else "fresh")
                    except describe.Unavailable as e:
                        raise ValueError(str(e)) from e
                    return self.send({"ok": True})
                if path == "/api/describe":              # describe the photo with the local model (LM Studio or built-in)
                    i = body.get("i")
                    if not (isinstance(i, int) and 0 <= i < db.n):
                        raise ValueError("no such face")
                    k = job.key(i)
                    d = describe.saved(k)
                    if d and not body.get("refresh"):
                        return self.send(dict(d, cached=True))
                    try:
                        d = describe.ask(job.picture(i))
                    except describe.Unavailable as e:
                        raise ValueError(str(e)) from e
                    return self.send(describe.save(k, d))
                if path == "/api/settings":             # page_store.js: changed settings of this page
                    import devicesettings
                    if body.get("app") != "label":
                        raise ValueError("bad settings")
                    return self.send({"settings": devicesettings.update("label", body.get("settings") or {})})
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
                        r["when"] = paths.utc_now()
                        if not r["errors"]:                       # keep only complete answers
                            with web_lock:
                                web_saved[fid] = {k: r[k] for k in ("when", "suggestions", "checked", "details")}
                                F.save_json(WEB_SAVED, web_saved)
                        return self.send(r)
                    except (OSError, websearch.http.client.HTTPException) as e:
                        return self.send({"error": f"Couldn't reach the search site: {e}"}, code=502)
            except (ValueError, KeyError, TypeError) as e:   # incl. localguard.RequestError (400, or 413)
                return self.send({"error": str(e)}, code=getattr(e, "code", 400))
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


def downloads():
    """Downloads in progress, for the progress bar: models for LM Studio (lmstudio.get_models) and CUDA support
    (cudasupport). [{what, how, pct, bytes, total, started}]"""
    out = []
    g = lmstudio.STATE.get("get") or {}
    if g.get("running"):
        out.append({"what": g.get("now"), "how": g.get("how"), "fallback": g.get("fallback"), "pct": g.get("pct"),
                    "bytes": g.get("bytes"),
                    "total": g.get("total"), "started": g.get("started"), "done": len(g.get("done", []))})
    c = cudasupport.STATE
    if c.get("state") == "downloading":
        out.append({"what": "CUDA support", "how": "downloading from PyPI", "bytes": c.get("done"), "total": c.get("total"),
                    "pct": int(c.get("done", 0) * 100 / max(c.get("total") or 1, 1)), "started": c.get("started")})
    return out


def face_index(body, db=None):
    """The face a request is about (its "i"): a missing or bad one is refused, never taken as the last face (-1)."""
    try:
        i = int(body.get("i"))
    except (TypeError, ValueError):
        raise ValueError("no such face") from None
    if i < 0:
        raise ValueError("no such face")
    return i


def facedb_configured():
    import facedb
    return facedb.configured()


def start_sync(db):
    """The shared database's sync (dbsync.py), when psycopg is installed; without a database configured it only
    reports that. db.sync is None when it can't run here."""
    import face_id as F
    try:
        import dbsync
        import psycopg  # noqa: F401
    except ImportError:
        db.sync = None
        return None
    version = label_settings.app_version()

    import dbsync_home
    import persondata

    def save_web():
        with db.web_lock:
            F.save_json(WEB_SAVED, db.web_saved)

    def forget(name):                      # another device forgot someone: here too
        persondata.forget(db, name, db.web_saved, db.web_lock, known=True)
    import peer
    home = dbsync_home.HomePart(forget=forget, peer_provider=lambda h, frame: peer.own_photo(db.store, h, frame))
    db.sync = dbsync.Sync(dbsync.Local(db, db.web_saved, save_web), version, home=home).start()
    websearch.shared_count = lambda service, month, cap: (
        dbsync_home.count_use(service, month, cap) if facedb_configured() else None)
    F.SAVED_HOOKS.append(db.sync.note)
    return db.sync


def attach(srv):
    """Load the faces and let srv (an HTTP server on 127.0.0.1) answer the labeling page; starts the
    background readers (photo dates and tags, webcam snapshots). Returns the LabelDB."""
    import face_id as F
    import labels as L
    F.require_models()
    db = L.LabelDB()
    # The Web search answers are guarded by the labeling data's own lock: the sync reads and writes them under
    # that one (dbsync.Local), and a lock of their own here let a search's answer be stored while the sync was
    # going through them.
    db.web_saved, db.web_lock = F.load_json(WEB_SAVED, {}), db.lock
    importer = L.Importer(db)
    srv.RequestHandlerClass = make_handler(db, importer)
    # Listed before the sync starts: while it runs, a face another device just ignored is for a moment without print
    # and not yet Ignored here, and must not get its print back.
    lost = db.lost_prints()
    start_sync(db)
    import musiclog
    musiclog.start()                       # the music log: does nothing until the owner switches it on (Settings)
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
        if lost:                           # Ignored faces named or un-ignored before v1.22.3 (LabelDB.lost_prints)
            try:
                n = db.mend(lost)
                print(f"{len(lost)} faces had no face print or picture left (they were ignored once, then named or "
                      f"taken off the Ignored list): made again from the photos for {n}"
                      + ("." if n == len(lost) else f"; the photos of the other {len(lost) - n} aren't on this "
                         "computer, or the face wasn't found in them again."))
            except Exception as e:
                print(f"Couldn't make the missing face prints again: {e}")
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
            srv = localguard.bind(port)        # no address reuse on Windows: a second copy gets OSError
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
