"""import_profile.py - where an import's time goes, step by step, on a sample of photos and videos. Saves nothing.

Run:  python tools/import_profile.py <folder> [photos=150] [videos=6]
Each step is timed one photo at a time, on one thread (the importer overlaps reading with the rest on 4 threads):
reading the file, hashing, decoding, fingerprint, face finding at each size, face prints + quality, the possible-face
scan, and for videos the frame decoding apart from the face finding. Prints milliseconds per photo for each.
"""
import random
import sys
import time
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import cv2  # noqa: E402

import face_id as F  # noqa: E402
import labels as L  # noqa: E402
from importer import Importer  # noqa: E402

folder = sys.argv[1]
n_photos = int(sys.argv[2]) if len(sys.argv) > 2 else 150
n_videos = int(sys.argv[3]) if len(sys.argv) > 3 else 6
media = F.list_media(folder, F.IMAGE_EXT | F.VIDEO_EXT)
rng = random.Random(0)
photos = [f for f in media if f.suffix.lower() in F.IMAGE_EXT]
videos = [f for f in media if f.suffix.lower() in F.VIDEO_EXT]
photos = rng.sample(photos, min(n_photos, len(photos)))
videos = rng.sample(videos, min(n_videos, len(videos)))
faces = F.Faces()
print(f"{len(photos)} photos, {len(videos)} videos from {folder}; models: {faces.devices}")

T = defaultdict(float)
n_faces = mb = 0


def timed(name, fn, *a):
    t = time.perf_counter()
    out = fn(*a)
    T[name] += time.perf_counter() - t
    return out


imp = Importer.__new__(Importer)          # only for its possible-face scan (_low_rows)
imp._more_lock, imp._more_det = __import__("threading").Lock(), None
for f in photos:
    data = timed("1 read file", f.read_bytes)
    mb += len(data) / 1e6
    timed("2 hash", lambda d: __import__("hashlib").sha1(d).hexdigest(), data)
    img = timed("3 decode", F.load_image, f)       # the file is in the system's cache now: decoding only
    if img is None:
        continue
    timed("4 fingerprint", L.fingerprint, img)
    rows = []
    for side in (10_000, 640):
        rows += timed(f"5 find faces at {'full' if side > 2000 else side}", faces._detect_at, img, side)
    kept = []
    for r in sorted(rows, key=lambda r: -r[14]):
        if all(F.iou(r, k) < 0.4 for k in kept) and r[2] >= F.MIN_FACE and r[3] >= F.MIN_FACE:
            kept.append(r)
    for r in kept:
        crop = timed("6 align", F.align, img, r)
        timed("7 face print", faces.embed, crop)
        timed("8 quality", faces.quality, crop)
    n_faces += len(kept)
    timed("9 possible faces (4 sizes)", imp._low_rows, img)

np_ = max(1, len(photos))
total = sum(T.values())
print(f"\nPHOTOS: {total / np_ * 1000:.0f} ms each on one thread ({np_ / total:.1f}/s), {n_faces} faces, "
      f"{mb / np_:.1f} MB each")
for k in sorted(T):
    print(f"  {k:28s} {T[k] / np_ * 1000:7.1f} ms  {T[k] / total:5.1%}")

V = defaultdict(float)
frames = 0
for f in videos:
    t = time.perf_counter()
    got = []
    for _k, _fps, frame in F.video_frames(f):
        got.append(frame)
    V["decode frames"] += time.perf_counter() - t
    frames += len(got)
    for frame in got:
        timed_t = time.perf_counter()
        faces.detect(frame)
        V["find faces + prints"] += time.perf_counter() - timed_t
    cap = cv2.VideoCapture(str(f))
    V["seconds of video"] += (cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0) / max(cap.get(cv2.CAP_PROP_FPS) or 30, 1)
    cap.release()
if videos:
    nv = len(videos)
    print(f"\nVIDEOS: {nv}, {V['seconds of video'] / nv:.0f} s long on average, {frames / nv:.0f} frames looked at each")
    for k in ("decode frames", "find faces + prints"):
        print(f"  {k:28s} {V[k] / nv:7.1f} s per video  ({V[k] / max(frames, 1) * 1000:.0f} ms per frame looked at)")
