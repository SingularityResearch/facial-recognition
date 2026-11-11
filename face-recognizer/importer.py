"""
importer.py - adding photos and videos in the background (Importer): reading, finding faces, possible faces,
duplicates, naming the faces it is sure of; one job at a time (Importer.begin). Files it can't read are counted
and the last few kept with the reason (note_problem). Split out of labels.py; `labels` re-exports it.
"""
import hashlib
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import cv2
import numpy as np

import face_id as F
import hardware

from photofiles import (fingerprint, file_hash, make_local, needs_possible, online_only, remember_folder,
                        unpin)

PROBLEMS_KEPT = 20       # files that couldn't be read or described: the last this many are kept to show why


def note_problem(st, path, why):
    """Count a file a background job couldn't read (or describe) in its status, and keep the last few
    {file, why} so the page can say which files and why, not just how many."""
    st["errors"] = st.get("errors", 0) + 1
    why = why if isinstance(why, str) else f"{type(why).__name__}: {why}" if str(why) else type(why).__name__
    st.setdefault("problems", []).append({"file": str(path), "why": why[:300]})
    del st["problems"][:-PROBLEMS_KEPT]


# --------------------------------------------------------------------------- import
VIDEO = object()                 # load() result for a video: its faces, found frame by frame on the worker


def is_video(path):
    return Path(path).suffix.lower() in F.VIDEO_EXT


class Importer:
    """Reads a photo folder in the background: skips photos already read and duplicates, finds the
    faces on the GPU, and names the ones the model is sure about. Progress is in self.status."""

    def __init__(self, db):
        self.db = db
        self.status = {"running": False}
        self.faces = None
        self.thread = None
        self.stop = threading.Event()
        self.job_lock = threading.Lock()          # imports, possible-face scans and Describe all: one at a time
        self._more_cache = {}                     # (photo, frame) -> (image, possible-face rows)

    def start(self, folder):
        folder = Path(str(folder).strip().strip('"'))
        if not folder.is_dir():
            raise ValueError(f"Folder not found: {folder}")
        self.begin(self.run, folder)

    def begin(self, target, *args):
        """Start a background job unless one is running (checked and started under one lock)."""
        with self.job_lock:
            if self.thread and self.thread.is_alive():
                raise ValueError("An import is already running.")
            self.stop.clear()
            self.thread = threading.Thread(target=target, args=args, daemon=True)
            self.thread.start()

    MORE_SCORE = 0.45                 # possible faces: detector confidence, far below F.DET_SCORE ...
    MORE_SIZES = (10_000, 640, 480, 320)   # ... at more scales: you look at what it finds and decide

    def _low_rows(self, img):
        """Every face the detector sees in img with the low cut-off MORE_SCORE, at MORE_SIZES (any thread: each
        has its own detector)."""
        if self.faces is None:
            self.faces = F.Faces()
        rows = [r for side in self.MORE_SIZES for r in self.faces._detect_at(img, side, self.MORE_SCORE)]
        rows.sort(key=lambda r: -r[14])
        kept = []
        for r in rows:
            if min(r[2], r[3]) >= F.MIN_FACE and all(F.iou(r, k) < 0.3 for k in kept):
                kept.append(r)
        return kept

    def _add_possible(self, h, rec, img, rows, frame=None):
        """Add rows as possible faces of photo h (record rec) and mark the photo as looked at.
        Returns their indexes; the caller saves."""
        db, store = self.db, self.db.store
        if rows and self.faces is None:
            self.faces = F.Faces()
        new = []
        for row in rows:
            crop = F.align(img, row)
            emb, q = self.faces.embed(crop), self.faces.quality(crop)
            with db.lock:
                j = store.add(h, rec.get("name") or Path(rec["path"]).name, row, emb, crop, q, frame)
                rec.setdefault("maybe", {})[store.fid[j]] = round(float(row[14]), 3)
                if frame is not None:
                    rec.setdefault("at", {})[store.fid[j]] = int(frame)
                new.append(j)
        with db.lock:
            rec.setdefault("maybe", {})
            rec["faces"] = rec.get("faces", 0) + len(new)
            if new:
                db.groups = None                  # regroup unknown faces with the new ones
            db.changed()
        return new

    def _possible(self, i):
        """Possible faces in the photo (or video frame) that face i is in that aren't faces there yet, for
        the photo view's dashed boxes. Returns (photo hash, photo record, frame, image, candidate rows)."""
        db, store = self.db, self.db.store
        with db.lock:
            h = store.photo[i]
            rec = store.photos.get(h, {})
            path = rec.get("path")
            frame = rec.get("at", {}).get(store.fid[i]) if rec.get("video") else None
            have = [store.box[j] for j in db.by_photo().get(h, [])
                    if not rec.get("video") or rec.get("at", {}).get(store.fid[j]) == frame]
        if not (path and Path(path).exists()):
            raise ValueError("The original photo isn't on this computer any more.")
        key = (h, frame)
        if key in self._more_cache:
            img, rows = self._more_cache[key]
        else:
            img = F.load_image(path) if frame is None else F.video_frame(path, frame)
            if img is None:
                raise ValueError("Couldn't read the photo.")
            rows = self._low_rows(img)
            self._more_cache[key] = (img, rows)
            while len(self._more_cache) > 4:      # a few photos: stepping back and forth stays quick
                self._more_cache.pop(next(iter(self._more_cache)), None)
        return h, rec, frame, img, [r for r in rows if all(F.iou(r, k) < 0.3 for k in have)]

    def possible_faces(self, i):
        """[{box: [x, y, w, h], score}] for the photo view's dashed boxes (nothing is saved): only those
        that look enough like a real face (LabelDB.face_cut), like the saved possible faces."""
        _, _, _, img, rows = self._possible(i)
        if rows:
            if self.faces is None:
                self.faces = F.Faces()
            f = self.db.faceness(np.asarray([self.faces.embed(F.align(img, r)) for r in rows]))
            if f is not None:
                cut = self.db.face_cut()
                rows = [r for r, v in zip(rows, f) if v >= cut]
        return [{"box": [round(float(v), 1) for v in r[:4]], "score": round(float(r[14]), 2)} for r in rows]

    def find_more(self, i, box=None):
        """Add the possible face at `box` (as possible_faces gave it), or all of them, to the photo of face i,
        ready to name. Returns the new faces' indexes."""
        h, rec, frame, img, rows = self._possible(i)
        if box is not None:
            rows = [r for r in rows if F.iou(r, [float(v) for v in box]) > 0.8][:1]
        new = self._add_possible(h, rec, img, rows, frame) if rows else []
        if new:
            with self.db.lock:
                self.db.store.save()
        return new

    def add_drawn(self, i, box):
        """Add a face you drew by hand ([x, y, w, h] in the photo's pixels, as the photo view shows it) to the
        photo (or video frame) of face i, for faces the detector misses even with the low cut-off (one mostly
        hidden behind someone scored 0.14, like junk). Its eyes, nose and mouth (needed to line the face up for
        recognition) come from the detector looking just inside the box with almost no cut-off; failing that,
        from where they sit in an average face. Kept as a possible face (score 0: drawn). Returns its index."""
        db, store = self.db, self.db.store
        x, y, w, h = (float(v) for v in box)
        if w < 16 or h < 16:
            raise ValueError("Draw a bigger box around the face.")
        with db.lock:
            ph = store.photo[i]
            rec = store.photos.get(ph, {})
            path = rec.get("path")
            frame = rec.get("at", {}).get(store.fid[i]) if rec.get("video") else None
            have = [store.box[j] for j in db.by_photo().get(ph, [])
                    if not rec.get("video") or rec.get("at", {}).get(store.fid[j]) == frame]
        def covered(b):                           # share of the drawn box inside face b
            ix = max(0.0, min(x + w, b[0] + b[2]) - max(x, b[0]))
            iy = max(0.0, min(y + h, b[1] + b[3]) - max(y, b[1]))
            return ix * iy / (w * h)
        if any(covered(b) > 0.6 for b in have):
            raise ValueError("There's already a face there.")
        if not (path and Path(path).exists()):
            raise ValueError("The original photo isn't on this computer any more.")
        img = F.load_image(path) if frame is None else F.video_frame(path, frame)
        if img is None:
            raise ValueError("Couldn't read the photo.")
        H, W = img.shape[:2]
        x, y = max(0.0, x), max(0.0, y)
        w, h = min(w, W - x), min(h, H - y)
        row = self._points_in(img, x, y, w, h)
        if row is None:                           # an average face's points, scaled to the box
            side = max(w, h)
            cx, cy = x + w / 2, y + h / 2
            pts = (F.ARC_TEMPLATE / 112.0 - 0.5) * side + [cx, cy]
            row = np.array([x, y, w, h, *pts.ravel(), 0.0], np.float32)
        else:
            row = np.array([x, y, w, h, *row[4:14], 0.0], np.float32)   # your box, the detector's points
        rows = [row]
        new = self._add_possible(ph, rec, img, rows, frame)
        with db.lock:
            store.save()
        return new[0]

    def _points_in(self, img, x, y, w, h):
        """The detector's best face inside the box (looked at enlarged, cut-off 0.02), or None."""
        pad = 0.4 * max(w, h)
        x0, y0 = int(max(0, x - pad)), int(max(0, y - pad))
        x1, y1 = int(min(img.shape[1], x + w + pad)), int(min(img.shape[0], y + h + pad))
        region = img[y0:y1, x0:x1]
        if region.size == 0:
            return None
        z = 320.0 / max(region.shape[:2])
        region = cv2.resize(region, None, fx=z, fy=z, interpolation=cv2.INTER_CUBIC if z > 1 else cv2.INTER_AREA)
        with self._more_lock:
            det = cv2.FaceDetectorYN.create(str(F.DET_MODEL), "", (region.shape[1], region.shape[0]), 0.02, 0.3, 50)
            _, found = det.detect(region)
        if found is None:
            return None
        best, best_iou = None, 0.3
        for r in found:
            r = r.copy()
            r[:14] /= z
            r[[0, 4, 6, 8, 10, 12]] += x0         # x and the 5 points' x (2 and 3 are width and height)
            r[[1, 5, 7, 9, 11, 13]] += y0
            k = F.iou(r, [x, y, w, h])
            if k > best_iou:
                best, best_iou = r, k
        return best

    def scan_possible(self):
        """Look for possible faces in every photo added before possible faces were kept, in the background
        (progress in self.status, like an import). Skips videos, and online-only cloud files (they'd be
        downloaded just for this): those are looked at another time, once they're on this computer."""
        self.begin(self._scan_possible)

    def _scan_possible(self):
        db, store = self.db, self.db.store
        st = self.status = dict(running=True, folder="possible faces", stage="Looking for possible faces", total=0,
                                done=0, new=0, known=0, duplicates=0, faces=0, auto=0, errors=0,
                                started=time.time(), error=None)
        pending = []
        try:
            with db.lock:
                todo = [h for h, p in store.photos.items() if needs_possible(p)]
            st["total"] = len(todo)
            for k, h in enumerate(todo):
                if self.stop.is_set():
                    break
                rec = store.photos[h]
                path = rec.get("path")
                try:
                    if not Path(path).exists() or online_only(path):
                        st["known"] += 1          # not on this computer now: another time
                    else:
                        img = F.load_image(path)
                        if img is None:
                            raise OSError("unreadable")
                        with db.lock:
                            have = [store.box[j] for j in db.by_photo().get(h, [])]
                        rows = [r for r in self._low_rows(img) if all(F.iou(r, b) < 0.3 for b in have)]
                        new = self._add_possible(h, rec, img, rows)
                        pending += new
                        st["faces"] += len(new)
                        st["new"] += bool(new)
                except Exception as e:
                    note_problem(st, path, e)
                st["done"] += 1
                if (k + 1) % 100 == 0:
                    self._checkpoint(pending)
            self._checkpoint(pending)
            st["stage"] = "Stopped" if self.stop.is_set() else "Done"
        except Exception as e:
            st["error"] = f"{type(e).__name__}: {e}"
            st["stage"] = "Failed"
            try:
                self._checkpoint(pending)
            except Exception:
                pass
        finally:
            st["running"] = False
            st["elapsed"] = round(time.time() - st["started"], 1)

    def run(self, folder):
        db, store = self.db, self.db.store
        st = self.status = dict(running=True, folder=str(folder), stage="Loading the face models", total=0, done=0,
                                new=0, known=0, duplicates=0, faces=0, auto=0, errors=0, started=time.time(), error=None)
        pending = []                              # new faces not yet saved and auto-named
        added = []                                # new photos with faces: their dates and people tags are read after
        pinned = []                               # online-only files this downloaded (unpinned at the end)
        try:
            if self.faces is None:
                self.faces = F.Faces()
            st["device"] = self.faces.device
            st["stage"] = "Listing photos"
            files = F.list_media(folder, F.IMAGE_EXT | F.VIDEO_EXT)
            # A Live Photo's short clip sits beside its photo under the same name: the photo is enough.
            stills = {(f.parent, f.stem.lower()) for f in files if f.suffix.lower() in F.IMAGE_EXT}
            files = [f for f in files if not (is_video(f) and (f.parent, f.stem.lower()) in stills)]
            st["total"], st["videos"] = len(files), sum(1 for f in files if is_video(f))
            with db.lock:
                by_path = {p["path"]: p for p in store.photos.values() if p.get("path")}
                # videos already read, by name and size: a copy isn't read again (an online-only copy,
                # e.g. in iCloud Photos, would otherwise be downloaded just to find it's the same)
                seen_videos = {(Path(p["path"]).name.lower(), p.get("size")) for p in store.photos.values()
                               if p.get("video") and p.get("path")}
            todo = []
            for f in files:
                p = by_path.get(str(f))
                try:
                    s = f.stat()
                except OSError as e:
                    note_problem(st, f, e)
                    continue
                if (p and p.get("size") == s.st_size and p.get("mtime") == int(s.st_mtime)) or \
                        (is_video(f) and (f.name.lower(), s.st_size) in seen_videos):
                    st["known"] += 1
                    st["done"] += 1
                else:
                    todo.append(f)
            remember_folder(folder)
            st["stage"] = "Reading photos"

            def load(f):
                """On a worker: everything about one file that doesn't change the database. A photo is read once
                (its hash and its picture from the same bytes), decoded, and its faces and possible faces found."""
                if make_local(f):
                    pinned.append(f)
                s = f.stat()
                if is_video(f):
                    h = file_hash(f)
                    if h in store.photos:
                        return f, s, h, None
                    try:
                        return f, s, h, (VIDEO, F.video_faces(self.faces, f, self.stop), None)
                    except Exception as e:
                        return f, s, h, (VIDEO, (None, None), e)
                data = f.read_bytes()
                if len(data) != s.st_size:             # e.g. an online-only cloud file that wasn't all downloaded
                    raise OSError(f"read only {len(data):,} of {s.st_size:,} bytes of {f.name}")
                h = hashlib.sha1(data).hexdigest()       # the same key as photofiles.file_hash
                if h in store.photos:
                    return f, s, h, None
                img = F.load_image(f, data)
                if img is None:
                    return f, s, h, (None,)
                found = self.faces.detect(img)
                low = [r for r in self._low_rows(img) if all(F.iou(r, k[0]) < 0.3 for k in found)]
                return f, s, h, (img, fingerprint(img), found, low)

            workers = hardware.import_workers()
            st["workers"] = workers
            with ThreadPoolExecutor(workers) as ex:
                def submit(f):
                    fut = ex.submit(load, f)
                    fut.item = f                          # which file, if it fails
                    return fut
                ahead = 2 * workers                       # files in progress at once
                window = [submit(f) for f in todo[:ahead]]
                nxt = ahead
                while window and not self.stop.is_set():
                    fut = window.pop(0)
                    if nxt < len(todo):
                        window.append(submit(todo[nxt]))
                        nxt += 1
                    try:
                        f, s, h, got = fut.result()
                    except Exception as e:
                        note_problem(st, fut.item, e)
                        st["done"] += 1
                        continue
                    meta = dict(path=str(f), size=s.st_size, mtime=int(s.st_mtime))
                    if h in store.photos:
                        old = store.photos[h]
                        with db.lock:
                            if not old.get("path") or not Path(old["path"]).exists():
                                old.update(meta)      # same photo, moved or renamed
                                st["known"] += 1
                            else:
                                st["duplicates"] += 1
                    elif got[0] is VIDEO:
                        (fps, found), why = got[1], got[2] or "couldn't read the video"
                        if self.stop.is_set():
                            break                         # half-read: read again next time
                        if found is None:
                            note_problem(st, f, why)
                        else:
                            with db.lock:
                                at = {}
                                for k, row, emb, crop, q in found:
                                    j = store.add(h, f.name, row, emb, crop, q, frame=k)
                                    pending.append(j)
                                    at[store.fid[j]] = int(k)
                                store.photos[h] = dict(meta, fp=None, faces=len(found), name=f.name,
                                                       video=True, fps=round(float(fps), 3), at=at)
                                db.changed()
                            st["new"] += 1
                            st["faces"] += len(found)
                    elif got[0] is None:
                        note_problem(st, f, "couldn't read the picture (damaged, or a format it can't open)")
                    else:
                        img, fp, found, low = got
                        with db.lock:
                            dup = db.near_duplicate(fp)
                        if dup and db.same_faces(dup, [e for _, e, _, _ in found]):
                            with db.lock:
                                store.photos[h] = dict(meta, fp=fp, faces=0, dup_of=dup, name=f.name)
                            st["duplicates"] += 1
                        else:
                            with db.lock:
                                for row, emb, crop, q in found:
                                    pending.append(store.add(h, f.name, row, emb, crop, q))
                                store.photos[h] = dict(meta, fp=fp, faces=len(found), name=f.name)
                                db._fp = None
                                db.changed()
                            # and the possible faces: what the detector saw with the low cut-off too
                            more = self._add_possible(h, store.photos[h], img, low)
                            pending += more
                            if found or more:
                                added.append(h)
                            st["new"] += 1
                            st["faces"] += len(found) + len(more)
                    st["done"] += 1
                    if st["new"] and st["new"] % 50 == 0 and pending:
                        self._checkpoint(pending)
                for fut in window:
                    fut.cancel()
            self._checkpoint(pending)
            st["stage"] = "Stopped" if self.stop.is_set() else "Done"
        except Exception as e:                    # report, don't crash the page
            st["error"] = f"{type(e).__name__}: {e}"
            st["stage"] = "Failed"
            try:
                self._checkpoint(pending)
            except Exception:
                pass
        finally:
            unpin(pinned)
            with db.lock:
                db.groups = None                  # regroup unknown faces with the new ones
                db.changed()
            if added:
                db.facts.start(added)
            st["running"] = False
            st["elapsed"] = round(time.time() - st["started"], 1)

    def _checkpoint(self, pending):
        """Save the database and auto-name the faces added since the last checkpoint."""
        db = self.db
        with db.lock:
            db.store.save()
            if pending:
                self.status["auto"] += db.autoname(pending)
                pending.clear()
