"""The Raspberry Pi's own code flow.

The owner's rule (2026-11-20): nothing changes how the other hardware works to make the Pi easier; what the Pi
needs, it gets here, in code the other systems never enter (`here()` is False on them, and a test says so).

Why the Pi needs its own: recognition. A face's print (ArcFace R50) takes 0.55 s to make on a Raspberry Pi 4's
processor, with OpenCV and with ONNX Runtime alike (the same answers; measured 2026-11-20), where a frame's other
work takes 0.1 s: finding the faces 67 ms, a face's quality 21 ms, its expression 19 ms. The general flow makes
every face's print in every frame (face_id.Faces.detect), which a graphics processor does in a few milliseconds. On
the Pi that was one frame read in 0.7 s with one face in view and one in 2.6 s with four, and with all four cores
kept busy by it everything else waited too: a JPEG took the page 135 ms to make, a tiny answer of the tracker's
server over 100 ms, and the camera reached the other devices at 1.3 to 1.7 frames a second.

So here a frame never waits for a print (`Prints`):
  - each frame finds the faces and reads each one's quality (and the tracker its expression), as everywhere;
  - a face is followed from frame to frame by where it is (its box against the boxes of the frame before);
  - one thread makes prints behind the frames, one face at a time: a face new in view first, then each face
    again every REFRESH seconds, from its newest picture; the print itself is face_id's own (Faces.embed);
  - a frame carries a face's new print once, when one was made since the last frame, and the face's name as the
    last print gave it (unknown until its first). The tracker page takes a frame without a print as it always
    has: it adds none to that face's average.
What that costs: a face new in view has its name about a second later, not in the same frame, and its average
is built from about one print a second instead of one a frame.

Tried for the Pi and not kept: smaller pictures from its page (640 px instead of 960 for the one sent to be read
and the one for the other devices). It gave nothing on a Pi 4 (its camera at the other devices 2.5 to 3.2 frames a
second with them, 3.2 to 3.5 without; a JPEG took the page 150 ms either way: it waits for a free core, not on
pixels) and cost a coarser picture, so it was taken out again before any release (2026-11-20)."""
import os
import platform
import sys
import threading
import time
from pathlib import Path

import face_id as F

REFRESH = 1.5        # s between two prints of the same face (the thread makes one print at a time, 0.55 s each on a Pi 4)
SAME = 0.3           # a face is the one followed when its box overlaps that one's last box at least this much (IoU)
FORGET = 2.0         # s without being seen before a followed face is let go


def board(model="/proc/device-tree/model"):
    """What the board calls itself ("Raspberry Pi 4 Model B Rev 1.4"), or "" where there is no such file."""
    try:
        return Path(model).read_text(errors="replace").replace("\0", "").strip()
    except OSError:
        return ""


def here(env=None, system=None, machine=None, name=None):
    """True on a Raspberry Pi with a 64-bit system: the only place this module's flow runs. FACE_PI=off turns it
    off there (the general flow, for comparing); FACE_PI=on runs it on another computer (to try it, never by
    itself)."""
    said = ((os.environ if env is None else env).get("FACE_PI") or "").strip().lower()
    if said in ("on", "off"):
        return said == "on"
    system = sys.platform if system is None else system
    machine = (platform.machine() if machine is None else machine).lower()
    if not system.startswith("linux") or machine not in ("aarch64", "arm64"):
        return False
    return (board() if name is None else name).startswith("Raspberry Pi")


class Prints:
    """The faces of a camera's frames, with their prints made behind the frames (see the top of this file).
    find(bgr) answers as face_id.Faces.detect(bgr, sizes=(640,)) does, [(row, print, crop, quality)], except that
    a face's print is None in a frame that brought no new one; named(k) is then that face's (name, similarity)
    from its last print, and name(k, name, sim) keeps what a new print was called."""

    def __init__(self, faces, refresh=REFRESH, start=True, clock=time.monotonic):
        self.faces, self.refresh, self.clock = faces, refresh, clock
        self.lock = threading.Lock()
        self.followed = []           # [{box, crop, seen, made, emb, new, name, sim}]: the faces in view
        self.frame = []              # the followed face of each face in the last frame, in its order
        self.made = 0                # prints made so far (for the log and the tests)
        self._wake = threading.Event()
        if start:
            threading.Thread(target=self._run, daemon=True, name="the Pi's prints").start()

    # ---- the frames' side
    def find(self, bgr):
        faces = self.faces
        rows = sorted(faces._detect_at(bgr, 640), key=lambda r: -r[14])      # one pass, as the live tracker asks
        kept = []
        for r in rows:
            if all(F.iou(r, k) < 0.4 for k in kept):
                kept.append(r)
        seen = [(row, F.align(bgr, row)) for row in kept if row[2] >= F.MIN_FACE and row[3] >= F.MIN_FACE]
        found = [(row, crop, faces.quality(crop)) for row, crop in seen]
        now, out = self.clock(), []
        with self.lock:
            self.followed = [f for f in self.followed if now - f["seen"] <= FORGET]
            free, self.frame = list(self.followed), []
            for row, crop, quality in found:
                best = max(free, key=lambda f: F.iou(row, f["box"]), default=None)
                if best is not None and F.iou(row, best["box"]) >= SAME:
                    free.remove(best)
                else:                                    # new in view: no print and no name yet
                    best = {"made": None, "emb": None, "new": False, "name": "Unknown", "sim": 0.0}
                    self.followed.append(best)
                best.update(box=[float(v) for v in row[:4]], crop=crop, seen=now)
                emb = best["emb"] if best["new"] else None
                best["new"] = False
                self.frame.append(best)
                out.append((row, emb, crop, quality))
        if found:
            self._wake.set()
        return out

    def name(self, k, name, sim):
        """What the print of the last frame's k-th face was called: kept for the frames until its next print."""
        with self.lock:
            if k < len(self.frame):
                self.frame[k].update(name=name, sim=float(sim))

    def named(self, k):
        with self.lock:
            return (self.frame[k]["name"], self.frame[k]["sim"]) if k < len(self.frame) else ("Unknown", 0.0)

    # ---- the prints' side
    def due(self, now):
        """The followed face whose print is to be made next: one without a print first (the longest in view),
        else the one whose print is oldest and at least `refresh` old. None when nothing is due."""
        with self.lock:
            live = [f for f in self.followed if now - f["seen"] <= 1.0]
            fresh = [f for f in live if f["made"] is None]
            if fresh:
                return fresh[0]
            old = [f for f in live if now - f["made"] >= self.refresh]
            return min(old, key=lambda f: f["made"]) if old else None

    def step(self):
        """Make the print that is due, if any. True when one was made."""
        f = self.due(self.clock())
        if f is None:
            return False
        # face_id's own print, the same as everywhere. A runner of the Pi's own on two of the four cores (ONNX
        # Runtime: 0.8 s a print, the same answers) was tried so that the frames keep the other two: no gain
        # (4.9 to 5.2 frames a second with one face either way, 2026-11-20), so there is none.
        emb = self.faces.embed(f["crop"])
        with self.lock:
            f.update(emb=emb, new=True, made=self.clock())
            self.made += 1
        return True

    def _run(self):
        while True:
            try:
                if not self.step():
                    self._wake.wait(0.05)
                    self._wake.clear()
            except Exception as e:                       # a print that failed: the face keeps its last one
                print(f"The Pi's prints: {type(e).__name__}: {e}", flush=True)
                time.sleep(1)
