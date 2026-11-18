"""
describejob.py - Describe all and Describe everything again: the background job that has every photo described.

The labeling tool starts it (label_tool.py: its routes /api/describe/all and /api/describe/again, and the single
photo's Describe) and shows its progress; describe.py asks a model about one picture and keeps the descriptions.
One job at a time, shared with the imports: it runs as the Importer's thread and reports in importer.status, so
the page's progress bar and Stop button are the import's own. (This was 260 lines inside label_tool.make_handler,
the function that answers every request of the labeling page.)
"""
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

import cv2
import numpy as np

import describe
import face_id as F
import labels as L
import paths

HELPER_RETRY = 300                     # s a describing helper that failed twice in a row sits out before it is tried
                                       # again (its LM Studio may have been loading a model, or restarting)
PEER_MISSES = 10                       # Describe everything again: other devices not reached this often in a row
                                       # are asked for no more photos in that run


class DescribeJob:
    """db: the labels.LabelDB; importer: its Importer (the job lock, the thread, stop and status are its own);
    photo_jpeg(i): the whole photo of face i as JPEG, or None (the labeling tool's, with its small cache and the
    other devices behind it); peer_ready(): whether the other devices of a shared database can be asked."""

    def __init__(self, db, importer, photo_jpeg, peer_ready):
        self.db, self.importer, self.photo_jpeg, self.peer_ready = db, importer, photo_jpeg, peer_ready

    def key(self, i):
        """Descriptions are per photo (per frame for a video): every face in it shares one."""
        h = self.db.store.photo[i]
        rec = self.db.store.photos.get(h, {})
        frame = rec.get("at", {}).get(self.db.store.fid[i]) if rec.get("video") else None
        return h if frame is None else f"{h}:{frame}"

    def picture(self, i):
        """The photo of face i at most 1024 px on its longer side, as JPEG, for the local model."""
        jpg = self.photo_jpeg(i)
        if jpg is None:
            raise ValueError("The original photo isn't on this computer any more.")
        img = cv2.imdecode(np.frombuffer(jpg, np.uint8), cv2.IMREAD_COLOR)
        s = min(1.0, 1024 / max(img.shape[:2]))
        if s < 1:
            img = cv2.resize(img, None, fx=s, fy=s, interpolation=cv2.INTER_AREA)
        return cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()

    def items(self):
        """What Describe everything again covers: every photo, and every video frame a face was found in, of the
        whole library, whether its file is on this computer or only on another device of the shared database
        (a duplicate is left out: its first copy is described). [(description key, photo hash, path or None,
        frame number or None, what to call it)], those with a file here first."""
        with self.db.lock:
            out = []
            for h, p in self.db.store.photos.items():
                if p.get("dup_of"):
                    continue
                path = p.get("path")
                shown = path or p.get("name") or h
                if p.get("video"):
                    frames = sorted({k for k in (p.get("at") or {}).values() if isinstance(k, int)})
                    out += [(f"{h}:{k}", h, path, k, f"{shown} (frame {k})") for k in frames]
                else:
                    out.append((h, h, path, None, shown))
        return sorted(out, key=lambda it: not it[2])

    def due(self, items, since):
        """Those of items whose description wasn't made since the Describe everything again was started."""
        since = paths.utc_text(since)
        t, when = paths.parse_time(since), describe.whens()

        def done(key):
            w = when.get(key)
            if isinstance(w, str) and len(w) == 20 and w.endswith("Z"):
                return w >= since               # both as paths.utc_now writes them: that text sorts in time order
            d = paths.parse_time(w)             # an older one, in local time
            return d is not None and d >= t
        return [it for it in items if not done(it[0])]

    def again_state(self):
        """For Settings: how much Describe everything again covers, and an unfinished one's start and what is left."""
        a, items = describe.again(), self.items()
        return {"total": len(items), "frames": sum(1 for it in items if it[3] is not None),
                "elsewhere": sum(1 for it in items if not it[2]), "since": a["since"] if a else None,
                "model": a["model"] if a else None, "left": len(self.due(items, a["since"])) if a else 0}

    def start(self, again=None):
        """Describe every photo not described yet (not videos, duplicates or online-only cloud files) in the
        background; progress in importer.status so the page's progress bar shows it, and the page's Stop button
        stops it. One request to each server at a time, the next photos loading meanwhile: with 3 at once, LM
        Studio's log showed generation falling from 48 to 1.3 tokens/s per request (Qwen 3.6 35B-A3B), 45 s a photo
        instead of about 3.5 s. With "spread" on (Settings), the other LM Studio servers on the network with the
        same model each take the next photo as they finish (describe.helpers); a helper that fails twice in a row
        is left out and its photo goes back to the others, and it is tried again every HELPER_RETRY seconds with one
        photo (the first run left a helper out for good 13 s in, and 17 hours of work went to two computers).
        again ("fresh" or "on"): Describe everything again instead (Settings): everything in items(),
        described or not, each description replaced as its new one arrives; what only another device has is shown
        by that device (peer.py). "fresh" starts one from now; "on" carries on with the unfinished one, which
        describe.again() keeps until nothing is left."""
        with self.importer.job_lock:                  # one background job at a time (imports, scans, this)
            self._start(again)

    def _start(self, again):
        if self.importer.thread and self.importer.thread.is_alive():
            raise ValueError("An import is already running.")
        name = describe.model()                  # raises Unavailable with what's missing
        mark = describe.again() if again else None
        if again == "on" and not mark:
            raise ValueError("Nothing is unfinished: start Describe everything again instead.")
        if again == "fresh":
            mark = describe.set_again(name)
        self.importer.stop.clear()

        def run():
            st = self.importer.status = dict(running=True, folder="photo descriptions", stage="Describing photos",
                                        total=0, done=0, new=0, known=0, duplicates=0, faces=0, auto=0, errors=0,
                                        started=time.time(), error=None, again=bool(again))
            try:
                if again:
                    todo = self.due(self.items(), mark["since"])
                else:
                    with self.db.lock:
                        todo = [(h, h, p["path"], None, p["path"]) for h, p in self.db.store.photos.items()
                                if p.get("path") and not p.get("video") and not p.get("dup_of")]
                    todo = [it for it in todo if not describe.saved(it[0])]
                st["total"] = len(todo)
                if todo:
                    st["stage"] = "Finding LM Studio servers"
                    servers = [{"host": "this computer" if describe.where(describe.config()["url"])[0] == "this computer"
                                else urlparse(describe.config()["url"]).hostname, "url": None}]
                    try:
                        servers += describe.helpers(name)
                    except Exception as e:              # the network search failed: describe here alone
                        st["left_out"] = [f"the other computers (couldn't search the network: {e})"]
                    st["by"] = {s["host"]: 0 for s in servers}
                    st["stage"] = (("Describing everything again" if again else "Describing photos")
                                   + (f" on {len(servers)} computers" if len(servers) > 1 else ""))
                    self._on(servers, todo, name, st, bool(again))
                if again and not self.importer.stop.is_set() and not self.due(self.items(), mark["since"]):
                    describe.set_again(None)            # all of it done; else it stays, to carry on with
                st["stage"] = "Stopped" if self.importer.stop.is_set() else "Done"
            except Exception as e:
                st["error"] = f"{type(e).__name__}: {e}"
                st["stage"] = "Failed"
            finally:
                describe.flush()
                st["running"] = False
                st["elapsed"] = round(time.time() - st["started"], 1)
        self.importer.thread = threading.Thread(target=run, daemon=True)
        self.importer.thread.start()

    def _on(self, servers, todo, name, st, elsewhere=False):
        """Describe the photos in todo (items as items() gives them) with one worker per server in
        servers [{"host", "url"}] (url None: the configured server, which keeps going to the end). Loader threads
        read and shrink the photos a few ahead; each worker takes the next one as it finishes. Counts go in st
        (importer.status). elsewhere: a photo that isn't on this computer is asked of the device that has it."""
        import queue
        work = queue.Queue()
        for it in todo:
            work.put(it)
        ready = queue.Queue(maxsize=2 * len(servers) + 2)    # loaded photos waiting for a free server
        back = queue.Queue()                                # photos a failing helper gave back: taken first
        lock = threading.Lock()
        left = [len(todo)]                                  # photos not finished yet (described, skipped, failed)
        stop = self.importer.stop
        away = {"misses": 0, "off": not (elsewhere and self.peer_ready())}

        def from_device(h, frame):
            """The photo from another device, or None. After PEER_MISSES in a row that no device answered for, no
            more are asked in this run (each would wait for every address of a device that is off)."""
            with lock:
                if away["off"]:
                    return None
            import peer
            jpg, why = peer.fetch(h, frame)
            with lock:
                if jpg is not None:
                    away["misses"] = 0
                elif peer.UNREACHED in why:
                    away["misses"] += 1
                    if away["misses"] >= PEER_MISSES and not away["off"]:
                        away["off"] = True
                        st.setdefault("left_out", []).append(f"what is only on other devices ({why[:200]})")
            return jpg

        def finish(n=1):
            with lock:
                left[0] -= n
                st["done"] += n
                if st["done"] % 25 == 0:
                    describe.flush()

        def load(item):
            _key, h, path, frame, _shown = item
            if path and Path(path).exists() and not L.online_only(path):
                img = F.load_image(path) if frame is None else F.video_frame(path, frame)
            else:
                jpg = from_device(h, frame)
                if jpg is None:
                    return None
                img = cv2.imdecode(np.frombuffer(jpg, np.uint8), cv2.IMREAD_COLOR)
            if img is None:
                raise OSError("unreadable")
            sc = min(1.0, 1024 / max(img.shape[:2]))
            if sc < 1:
                img = cv2.resize(img, None, fx=sc, fy=sc, interpolation=cv2.INTER_AREA)
            return cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 85])[1].tobytes()

        def loader():
            while not stop.is_set():
                try:
                    item = work.get_nowait()
                except queue.Empty:
                    return
                try:
                    jpg = load(item)
                except Exception as e:
                    L.note_problem(st, item[4], e)
                    finish()
                    continue
                if jpg is None:                             # not on this computer now (moved, online-only),
                                                            # nor shown by another device
                    with lock:
                        st["known"] += 1
                    finish()
                    continue
                while not stop.is_set():
                    try:
                        ready.put((item, jpg), timeout=0.5)
                        break
                    except queue.Full:
                        pass

        def sit_out():
            """A helper left out waits HELPER_RETRY s; False when the run ended or was stopped meanwhile."""
            until = time.monotonic() + HELPER_RETRY
            while time.monotonic() < until:
                if stop.wait(min(1.0, max(0.05, until - time.monotonic()))):
                    return False
                with lock:
                    if left[0] <= 0:
                        return False
            return True

        def worker(srv, main):
            fails = 0
            while not stop.is_set():
                with lock:
                    if left[0] <= 0:
                        return
                try:
                    item, jpg = back.get_nowait()
                except queue.Empty:
                    try:
                        item, jpg = ready.get(timeout=0.5)
                    except queue.Empty:
                        continue
                try:
                    d = describe.ask(jpg, name, srv["url"])
                except Exception as e:
                    if not main:                            # another computer: its photo goes back to the others
                        back.put((item, jpg))
                        fails += 1
                        if fails >= 2:
                            note = (f"{srv['host']} (failed twice: {str(e)[:200]}; tried again every "
                                    f"{max(1, round(HELPER_RETRY / 60))} min)")
                            with lock:                      # shown on the progress bar, not as an unreadable photo
                                st.setdefault("left_out", []).append(note)
                            if not sit_out():
                                return
                            with lock:                      # back in: one more failure and it sits out again
                                st["left_out"].remove(note)
                            fails = 1
                        continue
                    L.note_problem(st, item[4], str(e))
                    finish()
                    continue
                fails = 0
                describe.save(item[0], d, write=False)
                with lock:
                    st["new"] += 1
                    st["by"][srv["host"]] = st["by"].get(srv["host"], 0) + 1
                finish()

        threads = [threading.Thread(target=loader, daemon=True) for _ in range(2 + len(servers))]
        threads += [threading.Thread(target=worker, args=(s, k == 0), daemon=True) for k, s in enumerate(servers)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
