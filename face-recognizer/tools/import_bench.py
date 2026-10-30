"""import_bench.py - time "Add photos" on a sample of photos not read yet, without saving anything.
Run:  python tools/import_bench.py "E:\\Users\\Chris\\iCloud\\Photos" 60"""
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import face_id as F  # noqa: E402
import labels as L  # noqa: E402

folder, n = sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 60
store = F.Store()
known = {p.get("path") for p in store.photos.values()}
todo = [p for p in F.list_media(folder, F.IMAGE_EXT) if str(p) not in known]
print(f"{len(todo)} photos in the folder not read yet; timing {min(n, len(todo))}")
todo = todo[:n]
faces = F.Faces()
print("device:", faces.device)


def load(f):
    t = time.perf_counter()
    h = L.file_hash(f)
    img = F.load_image(f)
    return f, img, time.perf_counter() - t


t0 = time.perf_counter()
det_t, found, heic = 0.0, 0, sum(1 for f in todo if f.suffix.lower() in (".heic", ".heif"))
with ThreadPoolExecutor(4) as ex:
    for f, img, lt in ex.map(load, todo):
        if img is None:
            continue
        L.fingerprint(img)
        t = time.perf_counter()
        found += len(faces.detect(img))
        det_t += time.perf_counter() - t
total = time.perf_counter() - t0
print(f"{len(todo)} photos ({heic} HEIC) in {total:.1f}s = {len(todo) / total:.1f} photos/s; {found} faces")
print(f"face finding + face prints on the main thread: {det_t / len(todo) * 1000:.0f} ms per photo")
print(f"estimate for {len(L.F.list_media(folder, F.IMAGE_EXT)) - len(known)} unread photos: "
      f"{(len(F.list_media(folder, F.IMAGE_EXT)) - len(known)) / (len(todo) / total) / 60:.1f} min")
