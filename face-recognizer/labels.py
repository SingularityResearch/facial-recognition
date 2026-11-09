"""
labels.py - the face-labeling engine shared by label_tool.py (the labeling page) and face_id.py.

Every face has a permanent ID (its photo's content fingerprint + where the face is in the photo),
and its label is stored on the face itself in data/labels.json:

  {"<face id>": {"n": "Chris", "by": "me"}, ...}
    n    the person's name, or "#ignore" (stranger, artwork, not a face: never recognized)
    by   "me" (you named it) or "auto" (named automatically because the model was very sure)
    not  names you said this face is NOT, so they are never suggested for it again
    ok   you looked at a "might be mislabeled" warning and kept the name
    s    the model's score when the face was named automatically

Groups are no longer stored. For faces nobody has named, the engine works out:
  suggested  the model thinks it knows who it is (score >= the match setting): you confirm
  unknown    nobody known is close: shown in groups of look-alike faces for you to name
Every change is logged to data/history.jsonl and can be undone; labels.json is backed up at
each start of the labeling page (data/backups/, last 20 kept).
"""
import hashlib
import json
import os
import shutil
import string
import subprocess
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import cv2
import numpy as np

import face_id as F

IGNORE = "#ignore"
MATCH = F.MATCH          # same setting the live recognizer uses
AUTO = 0.45              # name a new face automatically at or above this score (measured: 89% of
                         # your labeled faces named with no mistakes; no stranger scored above 0.35) ...
MARGIN = 0.10            # ... if the next most likely person scores at least this much lower
TOPK = 5                 # a person's score = mean of the 5 closest of their faces (as the recognizer)
FEW = 5                  # people with fewer usable faces than this are flagged "needs more photos"
DUP_BITS = 12            # photos whose 256-bit picture fingerprints differ in <= this many bits, and whose
                         # faces match, are copies (plain or dark photos can look alike by fingerprint alone)
HINT = MATCH - 0.12      # unknown groups this close to a known person get a "looks like ..." hint
POP = np.array([bin(i).count("1") for i in range(256)], np.uint8)


# --------------------------------------------------------------------------- photo fingerprints
def file_hash(path):
    """SHA-1 of the file's bytes: the same photo copied or renamed gets the same key."""
    h, n = hashlib.sha1(), 0
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
            n += len(chunk)
    if n != os.stat(path).st_size:            # e.g. an online-only cloud file that wasn't all downloaded
        raise OSError(f"read only {n:,} of {os.stat(path).st_size:,} bytes of {Path(path).name}")
    return h.hexdigest()


CLOUD = 0x400000 | 0x1000                     # Windows file attributes: data still in the cloud (online-only)


SAME = 0.95        # face prints this alike are the same picture again (LabelDB.stacks)
FACE_SHOW = 0.25   # share of possible faces drawn as dashed boxes: the most face-like (LabelDB.face_cut)


def needs_possible(p):
    """True if photo record p hasn't been looked at for possible faces yet (videos aren't: too slow)."""
    return bool(p.get("path")) and not p.get("video") and not p.get("dup_of") and "maybe" not in p


def online_only(path):
    try:
        return bool(os.stat(path).st_file_attributes & CLOUD)
    except (OSError, AttributeError):         # AttributeError: not Windows
        return False


def make_local(path):
    """Download an online-only file (iCloud Photos, OneDrive) all the way. Read straight through, such
    a file can come back cut short (seen with iCloud: 190 KB of a 442 KB video, then end of file), so it's
    pinned ("Always keep on this device") and waited for, allowing at least 0.5 MB/s (iCloud for Windows
    managed about 1 video a minute on 2026-11-08).
    Returns True if this pinned it (unpin() it when done), False if it was already here.
    Raises OSError if it didn't arrive in time."""
    if not online_only(path):
        return False
    subprocess.run(["attrib", "+P", "-U", str(path)], capture_output=True,
                   creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    end = time.time() + 300 + os.stat(path).st_size / 5e5
    while online_only(path):
        if time.time() > end:
            raise OSError(f"{Path(path).name} didn't finish downloading from the cloud")
        time.sleep(1)
    return True


def unpin(paths):
    """Undo make_local's pin: the files stay here, and the cloud app may free their space again later."""
    for p in paths:
        subprocess.run(["attrib", "-P", str(p)], capture_output=True,
                       creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))


def fingerprint(bgr):
    """256-bit picture fingerprint (difference hash). Re-saved, resized or re-compressed copies of a
    photo differ in a few bits; different photos differ in roughly half of them."""
    g = cv2.resize(cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY), (17, 16), interpolation=cv2.INTER_AREA)
    return np.packbits((g[:, 1:] > g[:, :-1]).ravel()).tobytes().hex()


def photo_date(path):
    """When the photo was taken (EXIF), else the file's date. '' if unreadable."""
    try:
        from PIL import Image
        if Path(path).suffix.lower() in (".heic", ".heif"):
            import pillow_heif
            pillow_heif.register_heif_opener()
        with Image.open(path) as im:
            ex = im.getexif()
            d = ex.get_ifd(0x8769).get(36867) or ex.get(306)
        if d:
            return f"{d[:4]}-{d[5:7]}-{d[8:10]} {d[11:16]}"
    except Exception:
        pass
    try:
        return time.strftime("%Y-%m-%d %H:%M", time.localtime(Path(path).stat().st_mtime))
    except OSError:
        return ""


def settings():
    return F.load_json("settings.json", {"folders": []})


def remember_folder(folder):
    s = settings()
    folder = str(Path(folder))
    if folder not in s["folders"]:
        s["folders"].append(folder)
        F.save_json("settings.json", s)


# --------------------------------------------------------------------------- one-time upgrade
def migrate(store, folders, log=print):
    """Give every face in an older database a permanent ID and move the group names onto the faces.

    Older scans remembered photos by file name only; the photo folders are searched for those names
    so each photo gets its content hash, path (for the whole-photo view) and fingerprint
    (for duplicate detection). Photos that can't be found still get IDs, just no photo view."""
    by_name = {}
    for folder in folders:
        if Path(folder).exists():
            log(f"Looking for your photos in {folder} ...")
            for p in F.list_media(folder, F.IMAGE_EXT):
                by_name.setdefault(p.name, p)
    names = sorted(set(store.file))

    def info(name):
        p = by_name.get(name)
        if p is None:
            return None
        try:
            st = p.stat()
            img = F.load_image(p)
            return dict(hash=file_hash(p), path=str(p), size=st.st_size, mtime=int(st.st_mtime),
                        fp=fingerprint(img) if img is not None else None)
        except OSError:
            return None

    photo_of, t0 = {}, time.time()
    with ThreadPoolExecutor(8) as ex:
        for k, (name, rec) in enumerate(zip(names, ex.map(info, names)), 1):
            if rec:
                h = rec.pop("hash")
            else:
                h = "nofile-" + hashlib.sha1(name.encode()).hexdigest()[:24]
                rec = dict(path=None, size=None, mtime=None, fp=None)
            rec["name"], rec["faces"] = name, 0
            store.photos[h] = rec
            photo_of[name] = h
            if k % 250 == 0 or k == len(names):
                log(f"  {k}/{len(names)} photos fingerprinted ({time.time() - t0:.0f}s)")
    found = sum(1 for h in photo_of.values() if not h.startswith("nofile-"))

    store.fid, store.photo, store._fids = [], [], set()
    for name, box in zip(store.file, store.box):
        h = photo_of[name]
        store.claim_fid(h, box)
        store.photos[h]["faces"] += 1

    # group names -> per-face labels
    groups = F.load_json("groups.json", [])
    gnames = F.load_json("names.json", {})
    excluded = set(F.load_json("excluded.json", []))
    labels = F.load_json("labels.json", {})
    for i, g in enumerate(groups[:len(store.fid)]):
        n = gnames.get(str(g))
        if n is None:
            continue                              # unnamed group: stays unlabeled
        if i in excluded:                         # "not this group's person"
            if n:
                labels[store.fid[i]] = {"not": [n]}
        else:
            labels[store.fid[i]] = {"n": n or IGNORE, "by": "me"}
    register_faceless(store, F.load_json("scanned.json", []), by_name)
    store.save()
    F.save_json("labels.json", labels)
    for f in folders:
        remember_folder(f)

    old = F.DATA / "backups" / time.strftime("legacy-%Y%m%d-%H%M%S")
    old.mkdir(parents=True, exist_ok=True)
    for name in ("groups.json", "names.json", "excluded.json", "scanned.json"):
        if (F.DATA / name).exists():
            shutil.move(str(F.DATA / name), str(old / name))
    log(f"Upgraded {len(store.fid)} faces: {found} of {len(names)} photos found on disk, "
        f"{sum(1 for v in labels.values() if v.get('n'))} faces carry names. Old files are in {old}")


def register_faceless(store, scanned_names, by_name):
    """Older scans also read photos with no faces; record them (by path, size and date) so adding
    the same folder again doesn't read them a second time."""
    have = {p.get("path") for p in store.photos.values()}
    n = 0
    for name in scanned_names:
        p = by_name.get(name)
        if p is None or str(p) in have:
            continue
        try:
            st = p.stat()
        except OSError:
            continue
        store.photos["path-" + hashlib.sha1(str(p).encode()).hexdigest()[:24]] = dict(
            path=str(p), size=st.st_size, mtime=int(st.st_mtime), fp=None, faces=0, name=name)
        n += 1
    return n


# --------------------------------------------------------------------------- photos that moved
def _drives():
    return [f"{c}:\\" for c in string.ascii_uppercase if os.path.exists(f"{c}:\\")]


def _same_file(p, rec):
    """p could be the photo rec describes: it exists and has the recorded size (when one was recorded)."""
    try:
        st = p.stat()
    except OSError:
        return None
    return st if p.is_file() and (not rec.get("size") or st.st_size == rec["size"]) else None


def _moved_root(old, new):
    """(old folder, new folder) that the move swapped: the paths with their shared ending removed."""
    o, n = list(Path(old).parts), list(Path(new).parts)
    while len(o) > 1 and len(n) > 1 and o[-1].lower() == n[-1].lower():
        o.pop()
        n.pop()
    return str(Path(*o)), str(Path(*n))


def relocate(store, folders, lock=None, log=print):
    """Find photos whose recorded path is gone (a drive letter changed, a library was moved) and point
    them at where they are now: first the same folder on another drive, then a file with the same name
    and size in the remembered photo folders. The remembered folders follow the move too.
    Returns {"missing", "found", "folders": {old: new}}. Photos not found keep their old path."""
    lock = lock or threading.RLock()
    with lock:
        missing = {h: dict(p) for h, p in store.photos.items() if p.get("path")}
    gone_dirs = {d for d in {str(Path(p["path"]).parent) for p in missing.values()} if not Path(d).is_dir()}
    missing = {h: p for h, p in missing.items()
               if str(Path(p["path"]).parent) in gone_dirs or not Path(p["path"]).exists()}
    out = {"missing": len(missing), "found": 0, "folders": {}}
    if not missing:
        return out
    found = {}                                     # hash -> (new path, stat)

    # 1. the same folder on another drive letter
    drives = _drives()
    by_dir = {}
    for h, p in missing.items():
        by_dir.setdefault(Path(p["path"]).parent, []).append(h)
    for d, hs in by_dir.items():
        tail = d.relative_to(d.anchor)
        for drv in drives:
            cand = Path(drv) / tail
            if cand == d or not cand.is_dir():
                continue
            for h in hs:
                if h not in found and (st := _same_file(cand / Path(missing[h]["path"]).name, missing[h])):
                    found[h] = (cand / Path(missing[h]["path"]).name, st)

    # 2. by name and size anywhere in the remembered photo folders
    left = [h for h in missing if h not in found]
    if left:
        index = {}
        for folder in folders:
            if Path(folder).is_dir():
                log(f"Looking for {len(left)} moved photos in {folder} ...")
                for p in F.list_media(folder, F.IMAGE_EXT):
                    index.setdefault(p.name.lower(), []).append(p)
        for h in left:
            for p in index.get(Path(missing[h]["path"]).name.lower(), []):
                if st := _same_file(p, missing[h]):
                    found[h] = (p, st)
                    break

    if not found:
        return out
    roots = {}
    with lock:
        for h, (p, st) in found.items():
            rec = store.photos.get(h)
            if rec is None:
                continue
            old, rec["path"], rec["mtime"] = rec["path"], str(p), int(st.st_mtime)
            o, n = _moved_root(old, p)
            roots[o] = n
        F.save_json("photos.json", store.photos)
    out["found"] = len(found)

    # the remembered folders follow the move (the longest matching old folder wins)
    s = settings()
    new_folders = []
    for f in s["folders"]:
        if not Path(f).exists():
            for o in sorted(roots, key=len, reverse=True):
                if f.lower() == o.lower() or f.lower().startswith(o.lower().rstrip("\\/") + os.sep):
                    nf = roots[o] + f[len(o):]
                    if Path(nf).is_dir():
                        out["folders"][f] = nf
                        f = nf
                    break
        if f not in new_folders:
            new_folders.append(f)
    if new_folders != s["folders"]:
        s["folders"] = new_folders
        F.save_json("settings.json", s)
    return out


# --------------------------------------------------------------------------- what a photo says about itself
TAKEN_NEAR_H = 12        # photos taken within this many hours of each other count as one occasion
FOLDER_NEAR = 6          # photo without a camera date: this many photos with faces either side in its folder
CLOSEST = 3              # closest people listed for a face (a hint to you, never a label)
NS = dict(rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#",
          mwg="http://www.metadataworkinggroup.com/schemas/regions/",
          area="http://ns.adobe.com/xmp/sType/Area#",
          dim="http://ns.adobe.com/xap/1.0/sType/Dimensions#",
          mpri="http://ns.microsoft.com/photo/1.2/t/RegionInfo#",
          mpreg="http://ns.microsoft.com/photo/1.2/t/Region#",
          iptc="http://iptc.org/std/Iptc4xmpExt/2008-02-29/")


def _val(el, ns, name):
    """An XMP property written either as an attribute (here or on a nested rdf:Description) or as an element."""
    tag = f"{{{NS[ns]}}}{name}"
    for e in el.iter():
        v = e.get(tag) if e.tag != tag else e.text
        if isinstance(v, str) and v.strip():
            return v.strip()
    return None


def _num(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def xmp_people(xml):
    """People tags in an XMP packet: face regions (MWG: Lightroom, Picasa, digiKam; iPhones write unnamed ones; Microsoft:
    Windows Photo Gallery) and plain names (IPTC PersonInImage). Rectangles are normalized top-left
    [x, y, w, h]; 'frame' says whether they apply to the photo as stored or as displayed (rotated)."""
    import xml.etree.ElementTree as ET
    try:
        root = ET.fromstring(xml)
    except ET.ParseError:
        return []
    out = []
    for rl in root.iter(f"{{{NS['mwg']}}}Regions"):
        dims = next(rl.iter(f"{{{NS['mwg']}}}AppliedToDimensions"), None)
        applied = (_num(_val(dims, "dim", "w")), _num(_val(dims, "dim", "h"))) if dims is not None else None
        for li in rl.iter(f"{{{NS['rdf']}}}li"):
            area = next(li.iter(f"{{{NS['mwg']}}}Area"), None)
            if area is None or (_val(li, "mwg", "Type") or "Face") != "Face" or (_val(area, "area", "unit") or "normalized") != "normalized":
                continue
            x, y, w, h = (_num(_val(area, "area", k)) for k in "xywh")
            if None in (x, y, w, h):
                continue
            out.append(dict(name=_val(li, "mwg", "Name"), rect=[x - w / 2, y - h / 2, w, h], kind="mwg", applied=applied))
    for rl in root.iter(f"{{{NS['mpri']}}}Regions"):
        for li in rl.iter(f"{{{NS['rdf']}}}li"):
            r = [_num(v) for v in (_val(li, "mpreg", "Rectangle") or "").split(",")]
            out.append(dict(name=_val(li, "mpreg", "PersonDisplayName"), kind="mp", applied=None,
                            rect=r if len(r) == 4 and None not in r else None))
    for p in root.iter(f"{{{NS['iptc']}}}PersonInImage"):
        out += [dict(name=li.text.strip(), rect=None, kind="iptc") for li in p.iter(f"{{{NS['rdf']}}}li") if li.text and li.text.strip()]
    return out


def _xmp_packets(data):
    out, k = [], 0
    while (k := data.find(b"<x:xmpmeta", k)) >= 0:
        e = data.find(b"</x:xmpmeta>", k)
        if e < 0:
            break
        out.append(data[k:e + 12])
        k = e
    return out


def _orient(r, o):
    """Normalized top-left rect on the photo as stored -> on the photo as displayed (EXIF orientation o)."""
    x, y, w, h = r
    return {2: [1 - x - w, y, w, h], 3: [1 - x - w, 1 - y - h, w, h], 4: [x, 1 - y - h, w, h],
            5: [y, x, h, w], 6: [1 - y - h, x, h, w], 7: [1 - y - h, 1 - x - w, h, w],
            8: [y, 1 - x - w, h, w]}.get(o, [x, y, w, h])


def _iou(a, b):
    ix = max(0.0, min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0]))
    iy = max(0.0, min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1]))
    inter = ix * iy
    return inter / (a[2] * a[3] + b[2] * b[3] - inter + 1e-9)


def match_regions(regions, boxes):
    """Pair tagged face regions (pixel boxes) with found faces: greedy by overlap, at least 0.2.
    Tag regions are drawn looser than the detector's boxes, so a face well inside a region also counts."""
    pairs = []
    for a, r in enumerate(regions):
        for b, f in enumerate(boxes):
            s = _iou(r, f)
            cx, cy = f[0] + f[2] / 2, f[1] + f[3] / 2
            if s < 0.2 and r[0] <= cx <= r[0] + r[2] and r[1] <= cy <= r[1] + r[3] and f[2] * f[3] >= 0.15 * r[2] * r[3]:
                s = 0.2
            if s >= 0.2:
                pairs.append((s, a, b))
    used_a, used_b, out = set(), set(), {}
    for s, a, b in sorted(pairs, reverse=True):
        if a not in used_a and b not in used_b:
            used_a.add(a), used_b.add(b)
            out[b] = a
    return out


class PhotoFacts:
    """What each photo says about itself, read in the background: when the camera took it, and who it
    is tagged with (photo-app face tags inside the file or in a .xmp sidecar, or a Google Takeout
    .json). Saved in data/photo_facts.json; a photo is read again only when it, its sidecar files, or
    the faces found in it change. Nothing here is ever a label by itself.
    Tag rectangles are taken as applying to the photo as stored (before EXIF rotation), as MWG says:
    on the rotated iPhone photos here that paired 71 of 76 tag boxes with found faces, against 32 the other way."""
    FILE = "photo_facts.json"

    def __init__(self, db):
        self.db = db
        self.facts = {}                  # photo hash -> {"t": camera time or None, "tags": [...], "faces": {face: name}, "key": ...}
        self.lock = threading.Lock()
        self.status = {"running": False, "done": 0, "total": 0, "read": 0}
        self.version = 0
        self._dirs = {}
        self.thread = None
        idx_of = {f: i for i, f in enumerate(db.store.fid)}
        for h, f in F.load_json(self.FILE, {}).items():
            faces = {idx_of[k]: nm for k, nm in f.get("faces", {}).items() if k in idx_of}
            self.facts[h] = dict(f, faces=faces)

    def save(self):
        fid = self.db.store.fid
        with self.lock:
            out = {h: dict(f, faces={fid[i]: nm for i, nm in f["faces"].items()}) for h, f in self.facts.items() if f.get("key")}
        F.save_json(self.FILE, out)

    def key(self, h):
        """What a photo's facts depend on: the file, its sidecar files, and where its faces are."""
        p = Path(self.db.store.photos[h]["path"])
        st = p.stat()
        n = p.name.lower()
        side = [(k, v.stat().st_mtime) for ext in (".xmp", ".json") for k, v in sorted(self._listing(p.parent, ext).items())
                if k.startswith(p.stem.lower()) or k.startswith(n[:40])]
        with self.db.lock:
            geo = [[round(float(v)) for v in self.db.store.box[i][:4]] for i in self.db.by_photo().get(h, [])]
        return hashlib.sha1(repr((st.st_size, int(st.st_mtime), side, geo)).encode()).hexdigest()[:16]

    def _fresh(self, h):
        """Saved facts if nothing they depend on changed, else read the photo again."""
        try:
            k = self.key(h)
        except OSError:
            return self.facts.get(h) or self._read_safe(h), False
        with self.lock:
            f = self.facts.get(h)
        if f and f.get("key") == k:
            return f, False
        f = self._read_safe(h)
        f["key"] = k
        return f, True

    def start(self, hashes=None):
        """Read every photo with faces (or just these), in the background."""
        with self.db.lock:
            todo = [h for h in (hashes if hashes is not None else self.db.by_photo())
                    if self.db.store.photos.get(h, {}).get("path") and not self.db.store.photos[h].get("video")]
        if self.thread and self.thread.is_alive():
            if hashes is None:
                return
            self.thread.join()
        self.thread = threading.Thread(target=self._run, args=(todo,), daemon=True)
        self.thread.start()

    def _run(self, todo):
        st = self.status = {"running": True, "done": 0, "total": len(todo), "read": 0}
        with ThreadPoolExecutor(8) as ex:
            for h, (f, new) in zip(todo, ex.map(self._fresh, todo)):
                with self.lock:
                    self.facts[h] = f
                st["done"] += 1
                st["read"] += new
        if st["read"]:
            self.save()
        self.version += 1
        st["running"] = False

    def get(self, h):
        """Facts for one photo, reading it now if the background pass hasn't yet."""
        with self.lock:
            f = self.facts.get(h)
        if f is None:
            f = self._read_safe(h)
            with self.lock:
                self.facts[h] = f
        return f

    def items(self):
        with self.lock:
            return list(self.facts.items())

    def _read_safe(self, h):
        try:
            return self.read(h)
        except Exception:                # an unreadable photo just has no facts
            return {"t": None, "tags": [], "faces": {}}

    def _listing(self, folder, ext):
        k = (folder, ext)
        if k not in self._dirs:
            try:
                self._dirs[k] = {p.name.lower(): p for p in Path(folder).iterdir() if p.suffix.lower() == ext}
            except OSError:
                self._dirs[k] = {}
        return self._dirs[k]

    def read(self, h):
        t, tags = self.tags(h)
        named = [r for r in tags if r.get("name")]
        with self.db.lock:
            idx = self.db.by_photo().get(h, [])
            boxes = [[float(v) for v in self.db.store.box[i][:4]] for i in idx]
        regions = [r for r in named if r.get("box")]
        pairs = match_regions([r["box"] for r in regions], boxes)
        return {"t": t, "tags": sorted({r["name"] for r in named}), "faces": {idx[b]: regions[a]["name"] for b, a in pairs.items()}}

    def tags(self, h):
        """(camera time, people tags with 'box' in face-box pixels where the tag has a rectangle)."""
        from PIL import Image
        p = Path(self.db.store.photos[h]["path"])
        if p.suffix.lower() in (".heic", ".heif"):
            import pillow_heif
            pillow_heif.register_heif_opener()
        with Image.open(p) as im:
            size, ex = im.size, im.getexif()
        o = ex.get(0x0112, 1)
        d = ex.get_ifd(0x8769).get(36867) or ex.get(306)
        try:
            t = time.mktime(time.strptime(str(d)[:19], "%Y:%m:%d %H:%M:%S")) if d else None
        except ValueError:
            t = None
        with open(p, "rb") as fh:
            packets = _xmp_packets(fh.read(1 << 20))
        side = self._listing(p.parent, ".xmp")
        for k in (p.stem.lower() + ".xmp", p.name.lower() + ".xmp"):
            if k in side:
                packets += _xmp_packets(side[k].read_bytes())
        tags = [r for x in packets for r in xmp_people(x)]
        tags += self._takeout(p)

        # tag rectangles -> boxes on the photo the faces were found in (upright, shrunk to MAX_SIDE)
        W, H = size
        Wd, Hd = (H, W) if o in (5, 6, 7, 8) else (W, H)
        s = min(1.0, F.MAX_SIDE / max(W, H))
        sw, sh = round(Wd * s), round(Hd * s)
        for r in tags:
            if r.get("rect"):
                a = r.get("applied")
                displayed = r["kind"] == "mp" or (a and o in (5, 6, 7, 8) and (round(a[0] or 0), round(a[1] or 0)) == (H, W))
                x, y, w, hh = r["rect"] if displayed else _orient(r["rect"], o)
                r["box"] = [x * sw, y * sh, w * sw, hh * sh]
        return t, tags

    def _takeout(self, p):
        """People named in a Google Takeout sidecar (photo.jpg.json, photo.jpg.supplemental-metadata.json;
        long file names are cut short in the export, so other .json files starting the same way are checked)."""
        js = self._listing(p.parent, ".json")
        if not js:
            return []
        n = p.name.lower()
        exact = [js[k] for k in (n + ".json", n + ".supplemental-metadata.json") if k in js]
        cands = exact + [v for k, v in js.items() if k.startswith(n[:40]) and v not in exact][:6]
        for c in cands:
            try:
                j = json.loads(c.read_text("utf-8"))
            except (OSError, ValueError):
                continue
            if not isinstance(j, dict) or not (c in exact or (j.get("title") or "").lower() == n):
                continue
            if isinstance(j.get("people"), list):
                return [dict(name=str(x["name"]).strip(), rect=None, kind="takeout")
                        for x in j["people"] if isinstance(x, dict) and str(x.get("name", "")).strip()]
        return []


# --------------------------------------------------------------------------- engine
class LabelDB:
    """All faces + their labels. Thread-safe: the labeling page's requests and a background photo
    import share one instance under self.lock."""

    def __init__(self, folders=None, backup=True, log=print):
        self.lock = threading.RLock()
        self.store = F.Store()
        if self.store.needs_ids:
            migrate(self.store, folders if folders is not None else settings()["folders"], log)
        F.require_current(self.store)
        self.labels = F.load_json("labels.json", {})
        self.undo_stack = []
        self.version = 0                 # bumps on every change; caches key on it
        self._cache = {}
        self.groups = None               # cached groups of unknown faces (lists of face indices)
        self._grouped = set()
        self._fp, self._fp_keys = None, []
        self.facts = PhotoFacts(self)    # camera dates and people tags; start() reads them in the background
        if backup:
            self.backup()

    # ----- basics
    @property
    def n(self):
        return len(self.store.emb)

    def E(self):
        if self._cache.get("E_n") != self.n:
            dim = F.REC["dim"]
            self._cache["E"] = np.asarray(self.store.emb, np.float32).reshape(self.n, dim)
            self._cache["E_n"] = self.n
        return self._cache["E"]

    def usable(self, i):
        q = self.store.quality[i]
        return not q < F.Q_ENROLL                # nan = not scored = usable

    def rec(self, i):
        return self.labels.get(self.store.fid[i], {})

    def name(self, i):
        return self.rec(i).get("n")

    def changed(self):
        self.version += 1
        # kept: face prints, and the faceness model (retrained when faces are added, not on every label)
        self._cache = {k: v for k, v in self._cache.items()
                       if k in ("E", "E_n", "fm", "fm_n", "fs", "fc", "fc_n", "st", "st_n")}

    def by_photo(self):
        if self._cache.get("bp_n") != self.n:
            m = {}
            for i, h in enumerate(self.store.photo):
                m.setdefault(h, []).append(i)
            self._cache["bp"], self._cache["bp_n"] = m, self.n
        return self._cache["bp"]

    # ----- scoring
    def photos_with(self, names):
        """Photos in which every one of `names` is named, newest first:
        [{i: a face of the first name there (for its picture), path, exists, date, people: [names]}]."""
        want = [n for n in dict.fromkeys(names) if n]
        if not want:
            return []
        out = []
        for h, faces in self.by_photo().items():
            here = {self.name(i): i for i in faces if self.name(i) and self.name(i) != IGNORE}
            if all(n in here for n in want):
                meta = self.store.photos.get(h, {})
                p = meta.get("path") or ""
                with self.facts.lock:                 # camera date if already read; else the file's date
                    t = (self.facts.facts.get(h) or {}).get("t") or meta.get("mtime") or 0
                out.append({"i": here[want[0]], "path": p, "exists": bool(p) and Path(p).exists(), "t": t,
                            "date": time.strftime("%Y-%m-%d", time.localtime(t)) if t else "", "people": sorted(here)})
        return sorted(out, key=lambda r: r["t"], reverse=True)

    @staticmethod
    def export(rows, folder):
        """Copy the photos in rows (from photos_with) into folder, keeping their file dates. Needs no
        lock, so the labeling page stays responsive while it copies. Files already there with the same
        name and size are skipped; other name clashes get " (2)" etc.
        Returns {copied, skipped, missing} (missing: the original photo isn't on this computer)."""
        dest = Path(str(folder).strip().strip('"'))
        if not dest.is_absolute():
            raise ValueError("Give the full path of the folder, e.g. E:\\Pictures\\Export")
        dest.mkdir(parents=True, exist_ok=True)
        counts = {"copied": 0, "skipped": 0, "missing": 0}
        for r in rows:
            src = Path(r["path"]) if r["exists"] else None
            if src is None:
                counts["missing"] += 1
                continue
            to, k = dest / src.name, 2
            while to.exists():
                if to.stat().st_size == src.stat().st_size:
                    break
                to, k = dest / f"{src.stem} ({k}){src.suffix}", k + 1
            if to.exists():
                counts["skipped"] += 1
                continue
            shutil.copy2(src, to)
            counts["copied"] += 1
        return counts

    def webcam(self, i):
        """True if face i is from a webcam snapshot the live tracker saved (F.WEBCAM)."""
        if self._cache.get("cam_n") != self.n:
            self._cache["cam"] = {h for h, p in self.store.photos.items()
                                  if p.get("path") and Path(p["path"]).parent == F.WEBCAM}
            self._cache["cam_n"] = self.n
        return self.store.photo[i] in self._cache["cam"]

    def maybe(self, i):
        """Detector score of face i if it's a possible face (kept with the low cut-off Importer.MORE_SCORE,
        not the import's F.DET_SCORE), else None."""
        if self._cache.get("mb_n") != self.n or "mb" not in self._cache:
            self._cache["mb"] = {fid: sc for p in self.store.photos.values() for fid, sc in p.get("maybe", {}).items()}
            self._cache["mb_n"] = self.n
        return self._cache["mb"].get(self.store.fid[i])

    def video_at(self, i):
        """Seconds into its video where face i was found, or None for a face from a photo."""
        p = self.store.photos.get(self.store.photo[i], {})
        k = p.get("at", {}).get(self.store.fid[i]) if p.get("video") else None
        return None if k is None else round(k / (p.get("fps") or 30.0), 1)

    def gallery(self):
        """{name: [face indices]} of named, usable faces: what each person is recognized from.
        Webcam faces named automatically wait for your confirmation first: many snapshots of one visit
        come in at once, so one wrong automatic name would otherwise teach the recognizer the wrong
        face and name the next snapshots wrongly too. Possible faces (maybe()) named automatically wait too."""
        if "gal" not in self._cache:
            idx_of = {f: i for i, f in enumerate(self.store.fid)}
            gal = {}
            for fid, r in self.labels.items():
                i = idx_of.get(fid)
                if i is not None and r.get("n") and self.usable(i) and \
                        not (r.get("by") == "auto" and (self.webcam(i) or self.maybe(i) is not None)):
                    gal.setdefault(r["n"], []).append(i)
            self._cache["gal"] = {k: sorted(v) for k, v in gal.items()}
        return self._cache["gal"]

    def score(self, idx, gal=None):
        """Score faces against every person, the way the recognizer does (mean of the 5 closest of
        that person's faces). A face never counts toward its own score, and names in its 'not'
        list score -1. Returns (names, P) with P[row, person]."""
        gal = self.gallery() if gal is None else gal
        names = sorted(gal)
        idx = np.asarray(idx, int)
        P = np.full((len(idx), len(names)), -1.0, np.float32)
        if not len(idx) or not names:
            return names, P
        E = self.E()
        X = E[idx]
        for j, nm in enumerate(names):
            cols = np.asarray(gal[nm])
            for a in range(0, len(idx), 4096):              # bounded memory for big imports
                S = X[a:a + 4096] @ E[cols].T
                own = idx[a:a + 4096, None] == cols[None, :]
                S[own] = -2.0
                kk = min(TOPK, len(cols))
                T = np.sort(np.partition(S, len(cols) - kk, axis=1)[:, -kk:], axis=1)[:, ::-1]
                krow = np.minimum(kk, len(cols) - own.any(1))
                C = np.cumsum(T, axis=1)
                r = np.arange(len(T))
                P[a:a + 4096, j] = np.where(krow > 0, C[r, np.maximum(krow, 1) - 1] / np.maximum(krow, 1), -1.0)
        pos = {nm: j for j, nm in enumerate(names)}
        for r, i in enumerate(idx):
            for nm in self.rec(int(i)).get("not", ()):
                if nm in pos:
                    P[r, pos[nm]] = -1.0
        return names, P

    @staticmethod
    def best(names, P):
        """Per row: (best name, best score, second-best score)."""
        if not names or not len(P):
            return [None] * len(P), np.full(len(P), -1.0), np.full(len(P), -1.0)
        o = np.argsort(-P, axis=1)
        r = np.arange(len(P))
        b = P[r, o[:, 0]]
        s = P[r, o[:, 1]] if P.shape[1] > 1 else np.full(len(P), -1.0)
        return [names[k] for k in o[:, 0]], b, s

    def triage(self):
        """Every unnamed face: (index, best name, best score, second score). Cached per version."""
        if "tri" not in self._cache:
            idx = [i for i in range(self.n) if not self.name(i)]
            names, P = self.score(idx)
            bn, bs, ss = self.best(names, P)
            self._cache["tri"] = list(zip(idx, bn, bs.tolist(), ss.tolist()))
        return self._cache["tri"]

    # ----- queues for the labeling page
    def hidden(self, i, show_low):
        return not show_low and not self.usable(i)

    def suggested(self, show_low=False):
        out = {}
        for i, nm, s, _ in self.triage():
            if nm and s >= MATCH and not self.hidden(i, show_low):
                out.setdefault(nm, []).append([i, round(s, 3), self.q(i)])
        for v in out.values():
            v.sort(key=lambda f: f[1])           # least sure first: those are the ones to check
        return sorted(({"name": k, "faces": v} for k, v in out.items()), key=lambda g: (g["name"] == IGNORE, -len(g["faces"])))

    def q(self, i):
        q = self.store.quality[i]
        return None if np.isnan(q) else round(float(q), 3)

    def unknown(self, show_low=False, regroup=False):
        tri = [(i, nm, s) for i, nm, s, _ in self.triage() if not (nm and s >= MATCH) and self.maybe(i) is None]
        unk = {i for i, _, _ in tri}
        if regroup or self.groups is None:
            self._regroup(sorted(unk))
        groups = []
        for g in self.groups:
            m = [i for i in g if i in unk and not self.hidden(i, show_low)]
            if m:
                groups.append(m)
        loose = [i for i in sorted(unk - self._grouped) if not self.hidden(i, show_low)]
        singles = [g[0] for g in groups if len(g) == 1] + loose
        groups = [g for g in groups if len(g) > 1]
        groups.sort(key=len, reverse=True)
        out = []
        gal = self.gallery()
        for k, g in enumerate(groups):
            names, P = self.score(g, gal)
            hint = None
            if names:
                m = P.mean(0)
                j = int(np.argmax(m))
                if m[j] >= HINT:
                    hint = {"name": names[j], "s": round(float(m[j]), 3)}
            out.append({"id": k + 1, "faces": [[i, self.q(i)] for i in g], "hint": hint})
        singles.sort(key=lambda i: -(self.q(i) or 0))
        return {"groups": out, "singles": [[i, self.q(i)] for i in singles]}

    def _face_sets(self):
        """(real, junk) face indexes to learn faceness from: real = faces from the normal detector (85% sure;
        named or not, but not Ignored: trained on only the named ones, it ranked real faces worse) and
        possible faces you named; junk = possible faces not named, or Ignored (most possible faces aren't
        faces: 85% of a hand-sorted sample of 160). Faces drawn by hand count as neither."""
        pos, neg = [], []
        for i in range(self.n):
            nm, mb = self.name(i), self.maybe(i)
            if mb is None:
                if nm != IGNORE:
                    pos.append(i)
            elif mb > 0:
                (neg if not nm or nm == IGNORE else pos).append(i)
        return pos, neg

    @staticmethod
    def _fit(E, pos, neg):
        from sklearn.linear_model import LogisticRegression
        return LogisticRegression(C=0.1, max_iter=5000, class_weight="balanced").fit(
            E[pos + neg], [1] * len(pos) + [0] * len(neg))

    def _face_model(self):
        """Logistic regression on face prints, real faces against possible faces (_face_sets), for faces it
        hasn't seen (the photo view's on-the-spot boxes). Retrained when the number of faces changes. None when
        there's too little to learn from."""
        if self._cache.get("fm_n") != self.n:
            self._cache["fm_n"] = self.n
            pos, neg = self._face_sets()
            self._cache["fm"] = self._fit(self.E(), pos, neg) if len(pos) >= 50 and len(neg) >= 50 else None
        return self._cache["fm"]

    def faceness(self, emb):
        """'Looks like a real face' score (0..1) for face prints (rows of emb), or None without a model."""
        m = self._face_model()
        return None if m is None else m.predict_proba(np.atleast_2d(emb))[:, 1]

    def face_cut(self):
        """The faceness a possible face needs for its dashed box: the score of the top FACE_SHOW share of
        possible faces. Each possible face is scored by a model trained without it (5 folds), so it isn't
        simply learned as junk. On 160 hand-sorted possible faces (24 real; AUC 0.84), showing the top 25%
        kept 17 of the 24 real faces and 21 of the other 136, about half of those drawings and game
        characters (to a face print, a drawn face is a face): about 9 in 10 non-face objects get no box.
        Top 35% kept 18 real and 32 others, top 50% 21 and 47. 0 (show all) without enough to learn from."""
        if self._cache.get("fc_n") != self.n:
            self._cache["fc_n"] = self.n
            pos, neg = self._face_sets()
            idx = [i for i in range(self.n) if (self.maybe(i) or 0) > 0]
            scores = {}
            if len(pos) >= 50 and len(neg) >= 50 and idx:
                E = self.E()
                negset = set(neg)
                folds = [neg[k::5] for k in range(5)]
                for k, fold in enumerate(folds):
                    m = self._fit(E, pos, [j for kk, f in enumerate(folds) if kk != k for j in f])
                    for j, v in zip(fold, m.predict_proba(E[fold])[:, 1].tolist()):
                        scores[j] = v
                rest = [j for j in idx if j not in negset]        # possible faces you named: full model
                if rest:
                    for j, v in zip(rest, self.faceness(E[rest]).tolist()):
                        scores[j] = v
            self._cache["fs"] = scores
            self._cache["fc"] = float(np.quantile(list(scores.values()), 1 - FACE_SHOW)) if scores else 0.0
        return self._cache["fc"]

    def face_score(self, i):
        """Faceness of possible face i (1.0 for faces drawn by hand and faces that aren't possible faces)."""
        self.face_cut()
        return self._cache["fs"].get(i, 1.0)

    def shown(self, i):
        """Whether the photo view draws face i: every face except unnamed possible faces below face_cut."""
        return self.maybe(i) is None or bool(self.name(i)) or self.face_score(i) >= self.face_cut()

    def possible(self):
        """Possible faces nobody has named and the model doesn't suggest a name for, most face-like first:
        [[i, quality, detector score, faceness, shown]]. Their own list, so New faces isn't buried in them."""
        out = [[i, self.q(i), self.maybe(i), round(self.face_score(i), 3), self.shown(i)]
               for i, nm, s, _ in self.triage() if self.maybe(i) is not None and not (nm and s >= MATCH)]
        return sorted(out, key=lambda f: -f[3])

    def stacks(self):
        """{face: first face of its stack} for faces that are the same picture as another face: face prints at
        least SAME alike (screenshots of one avatar, burst shots, re-saved copies). The labeling page shows a
        stack as one tile. Measured: 31 copies of one avatar in different screenshots were 0.96 alike (median);
        different photos of one real person 0.44 (median), 0.74 at the 99th percentile, 0.2% at 0.9 or more.
        Recomputed when faces are added."""
        if self._cache.get("st_n") != self.n:
            self._cache["st_n"] = self.n
            E = self.E()
            parent = list(range(self.n))

            def root(a):
                while parent[a] != a:
                    parent[a] = parent[parent[a]]
                    a = parent[a]
                return a
            for a in range(0, self.n, 2048):
                M = E[a:a + 2048] @ E.T
                for r, j in zip(*np.nonzero(M >= SAME)):
                    i = a + int(r)
                    if i < j:
                        ri, rj = root(i), root(int(j))
                        if ri != rj:
                            parent[max(ri, rj)] = min(ri, rj)
            out = {}
            for i in range(self.n):
                r = root(i)
                if r != i:
                    out[i] = r
            for r in set(out.values()):
                out[r] = r
            self._cache["st"] = out
        return self._cache["st"]

    def possible_left(self):
        """Photos not yet looked at with the low cut-off (Importer.scan_possible)."""
        return sum(1 for p in self.store.photos.values() if needs_possible(p))

    def _regroup(self, idx):
        """Cluster faces nobody is suggested for into look-alike groups (same settings as `cluster`)."""
        self._grouped = set(idx)
        if len(idx) < 2:
            self.groups = [[i] for i in idx]
            return
        from sklearn.cluster import AgglomerativeClustering
        lab = AgglomerativeClustering(n_clusters=None, metric="cosine", linkage="average",
                                      distance_threshold=F.CLUSTER_DIST).fit_predict(self.E()[idx])
        groups = {}
        for i, l in zip(idx, lab):
            groups.setdefault(int(l), []).append(i)
        self.groups = sorted(groups.values(), key=len, reverse=True)

    def person(self, name):
        idx = [i for i in range(self.n) if self.name(i) == name]
        names, P = self.score(idx)
        j = names.index(name) if name in names else None
        out = []
        for r, i in enumerate(idx):
            rec = self.rec(i)
            out.append([i, round(float(P[r, j]), 3) if j is not None else None, self.q(i), rec.get("by", "me")])
        out.sort(key=lambda f: (f[1] is None, f[1] if f[1] is not None else 0))
        return out

    def auto_review(self):
        """Faces named automatically, per person, least sure first."""
        out = {}
        for nm, faces in self.people_faces(by="auto").items():
            names, P = self.score(faces)
            j = names.index(nm) if nm in names else None
            out[nm] = sorted([[i, round(float(P[r, j]), 3) if j is not None else None, self.q(i)]
                              for r, i in enumerate(faces)], key=lambda f: f[1] if f[1] is not None else -1)
        return sorted(({"name": k, "faces": v} for k, v in out.items()), key=lambda g: (g["name"] == IGNORE, -len(g["faces"])))

    def people_faces(self, by=None):
        out = {}
        for i in range(self.n):
            r = self.rec(i)
            if r.get("n") and (by is None or r.get("by", "me") == by):
                out.setdefault(r["n"], []).append(i)
        return out

    # ----- hints from the photo itself
    def tag_names(self, h, f=None):
        """{face: name} from the photo's people tags: tag rectangles that sit on a face, plus a tag
        without a rectangle when it is the only name left and only one face is still unnamed."""
        f = f or self.facts.get(h)
        out = dict(f["faces"])
        idx = self.by_photo().get(h, [])
        left = set(f["tags"]) - set(out.values()) - {self.name(i) for i in idx}
        open_ = [i for i in idx if i not in out and not self.name(i)]
        if len(left) == 1 and len(open_) == 1:
            out[open_[0]] = left.pop()
        return out

    def tagged(self, show_low=False):
        """Unnamed faces the photo's own people tags name, grouped by that name."""
        key = ("tagged", show_low, self.facts.version, len(self.facts.facts))
        if key not in self._cache:
            out = {}
            for h, f in self.facts.items():
                for i, nm in self.tag_names(h, f).items():
                    if not self.name(i) and nm not in self.rec(i).get("not", ()) and not self.hidden(i, show_low):
                        out.setdefault(nm, []).append([i, self.q(i)])
            known = set(self.people_faces())
            self._cache[key] = sorted(({"name": k, "faces": v, "known": k in known} for k, v in out.items()),
                                      key=lambda g: -len(g["faces"]))
        return self._cache[key]

    def occasion(self, h):
        """Named people in photos from the same occasion: taken within TAKEN_NEAR_H hours (camera date),
        else the photos next to it in its folder."""
        bp, f = self.by_photo(), self.facts.get(h)
        if f["t"]:
            near = sorted((abs(g["t"] - f["t"]), k) for k, g in self.facts.items()
                          if k != h and k in bp and g.get("t") and abs(g["t"] - f["t"]) <= TAKEN_NEAR_H * 3600)
            near, how = [k for _, k in near[:40]], f"taken within {TAKEN_NEAR_H} hours of it"
        else:
            if self._cache.get("fold_n") != self.n:
                fold = {}
                for k in bp:
                    p = self.store.photos.get(k, {}).get("path")
                    if p:
                        fold.setdefault(str(Path(p).parent), []).append((Path(p).name.lower(), k))
                self._cache["fold"] = {d: [k for _, k in sorted(v)] for d, v in fold.items()}
                self._cache["fold_n"] = self.n
            p = self.store.photos.get(h, {}).get("path")
            row = self._cache["fold"].get(str(Path(p).parent), []) if p else []
            k = row.index(h) if h in row else -1
            near = [x for x in row[max(0, k - FOLDER_NEAR):k + FOLDER_NEAR + 1] if x != h] if k >= 0 else []
            how = "next to it in its folder"
        count, example = {}, {}
        for k in near:
            for j in bp[k]:
                nm = self.name(j)
                if nm and nm != IGNORE:
                    count[nm] = count.get(nm, 0) + 1
                    example.setdefault(nm, j)
        people = sorted(({"name": nm, "n": c, "i": example[nm]} for nm, c in count.items()), key=lambda x: -x["n"])
        return {"how": how, "photos": len(near), "people": people}

    def hints(self, i):
        """Everything that might tell you who face i is: the closest people (however weak), the photo's
        people tags, and who else was photographed at the same occasion."""
        h = self.store.photo[i]
        closest = []
        if not self.name(i):
            names, P = self.score([i])
            closest = [{"name": names[j], "s": round(float(P[0, j]), 3)} for j in np.argsort(-P[0])
                       if names[j] != IGNORE and P[0, j] > 0][:CLOSEST]
        f = self.facts.get(h)
        return {"closest": closest, "tag": self.tag_names(h, f).get(i), "tags": f["tags"], "occasion": self.occasion(h),
                "live": self.live_verdict(i)}

    def live_verdict(self, i):
        """For a webcam snapshot the tracker saved: who the tracker thought face i was, how sure it was
        and when ({name, sim, when}), from the .json saved beside the photo; None for other photos."""
        p = self.store.photos.get(self.store.photo[i], {}).get("path")
        if not p or Path(p).parent != F.WEBCAM:
            return None
        try:
            saved = json.loads(Path(p).with_suffix(".json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None
        box = [float(v) for v in self.store.box[i][:4]]
        best = max(saved.get("faces", []), key=lambda f: _iou(f["box"], box), default=None)
        if not best or _iou(best["box"], box) < 0.3:
            return None
        return {"name": best["name"], "sim": best["sim"], "when": saved.get("when", "")}

    def checks(self):
        """Possible mistakes and gaps:
        mislabeled  named faces that look more like someone else, or not like the rest of their person
        few         people with too few usable faces to be recognized reliably
        duplicates  photos skipped during import because they were copies of photos already read"""
        if "chk" in self._cache:
            return self._cache["chk"]
        gal = self.gallery()
        idx = [i for nm, faces in gal.items() if nm != IGNORE for i in faces if not self.rec(i).get("ok")]
        names, P = self.score(idx, gal)
        bad = []
        for r, i in enumerate(idx):
            nm = self.name(i)
            j = names.index(nm)
            own = float(P[r, j])
            others = P[r].copy()
            others[j] = -1
            k = int(np.argmax(others)) if len(names) > 1 else None
            other, os_ = (names[k], float(others[k])) if k is not None else (None, -1.0)
            if len(gal[nm]) > 1 and (own < MATCH or os_ > own):
                bad.append({"i": i, "name": nm, "s": round(own, 3), "other": other, "os": round(os_, 3),
                            "by": self.rec(i).get("by", "me"), "q": self.q(i)})
        bad.sort(key=lambda b: b["s"] - b["os"])
        everyone = {r.get("n") for r in self.labels.values() if r.get("n") and r.get("n") != IGNORE}
        few = sorted(({"name": nm, "usable": len(gal.get(nm, []))} for nm in everyone if len(gal.get(nm, [])) < FEW),
                     key=lambda f: f["usable"])
        dups = [{"path": p.get("path"), "of": self.store.photos.get(p["dup_of"], {}).get("path")}
                for p in self.store.photos.values() if p.get("dup_of")]
        self._cache["chk"] = {"mislabeled": bad, "few": few, "duplicates": dups}
        return self._cache["chk"]

    def summary(self):
        tri = self.triage()
        sugg = sum(1 for i, nm, s, _ in tri if nm and s >= MATCH and self.usable(i))
        unk = sum(1 for i, nm, s, _ in tri if not (nm and s >= MATCH) and self.usable(i) and self.maybe(i) is None)
        low = sum(1 for i, _, _, _ in tri if not self.usable(i))
        people = []
        gal = self.gallery()
        for nm, faces in sorted(self.people_faces().items(), key=lambda kv: (kv[0] == IGNORE, kv[0].lower())):
            usable = gal.get(nm, [])
            cover = max(usable or faces, key=lambda i: self.store.quality[i] if not np.isnan(self.store.quality[i]) else 0)
            people.append({"name": nm, "faces": len(faces), "usable": len(usable), "cover": cover,
                           "auto": sum(1 for i in faces if self.rec(i).get("by") == "auto")})
        chk = self.checks()
        return {"faces": self.n, "photos": len(self.store.photos), "suggested": sugg, "unknown": unk,
                "hiddenLow": low, "possible": sum(1 for f in self.possible() if f[4]), "possibleLeft": self.possible_left(),
                "auto": sum(1 for r in self.labels.values() if r.get("by") == "auto" and r.get("n")),
                "checks": len(chk["mislabeled"]) + len(chk["few"]), "people": people,
                "tagged": sum(len(g["faces"]) for g in self.tagged()), "facts": self.facts.status,
                "canUndo": bool(self.undo_stack), "undoText": self.undo_stack[-1]["text"] if self.undo_stack else "",
                "webcam": [i for i in range(self.n) if self.webcam(i)],   # faces from the tracker's snapshots
                "video": [i for i in range(self.n) if self.store.photos.get(self.store.photo[i], {}).get("video")],
                "settings": {"match": MATCH, "auto": AUTO, "margin": MARGIN, "qEnroll": F.Q_ENROLL, "few": FEW,
                             "model": self.store.model, "folders": settings()["folders"]}}

    # ----- changes
    def act(self, action, idx, name=None, text=None):
        """Change labels. action: name | ignore | reject (not this person) | clear | confirm | ok."""
        name = (name or "").strip() or None
        if action in ("name", "reject") and not name:
            raise ValueError("a name is needed")
        if action == "name" and name.startswith("#") and name != IGNORE:
            raise ValueError("names can't start with #")
        with self.lock:
            before = []
            for i in idx:
                if not (0 <= i < self.n):
                    raise ValueError(f"no face {i}")
                fid = self.store.fid[i]
                old = self.labels.get(fid)
                before.append((fid, dict(old) if old else None))
                r = dict(old or {})
                if action in ("name", "ignore"):
                    r["n"] = name if action == "name" else IGNORE
                    r["by"] = "me"
                    for k in ("s", "ok"):
                        r.pop(k, None)
                    if r.get("not"):
                        r["not"] = [x for x in r["not"] if x != r["n"]] or None
                elif action == "reject":
                    r["not"] = sorted(set(r.get("not") or []) | {name})
                    if r.get("n") == name:
                        for k in ("n", "by", "s", "ok"):
                            r.pop(k, None)
                elif action == "clear":
                    for k in ("n", "by", "s", "ok"):
                        r.pop(k, None)
                elif action == "confirm":
                    if r.get("n"):
                        r["by"] = "me"
                        r.pop("s", None)
                elif action == "ok":
                    r["ok"] = True
                else:
                    raise ValueError(f"unknown action {action}")
                r = {k: v for k, v in r.items() if v}
                if r:
                    self.labels[fid] = r
                else:
                    self.labels.pop(fid, None)
            self._commit(before, text or self._describe(action, len(idx), name))
        return len(idx)

    @staticmethod
    def _describe(action, n, name):
        faces = f"{n} face{'s' if n != 1 else ''}"
        return {"name": f"Named {faces} {name}", "ignore": f"Ignored {faces}", "reject": f"Marked {faces} as not {name}",
                "clear": f"Removed the name from {faces}", "confirm": f"Confirmed {faces}",
                "ok": f"Kept the name on {faces}"}.get(action, action)

    def rename(self, old, new):
        new = (new or "").strip()
        if not new or new.startswith("#"):
            raise ValueError("give a new name")
        with self.lock:
            merge = new in self.people_faces()
            before = []
            for fid, r in list(self.labels.items()):
                hit_n, hit_not = r.get("n") == old, old in (r.get("not") or ())
                if hit_n or hit_not:
                    before.append((fid, dict(r)))
                    r = dict(r)
                    if hit_n:
                        r["n"] = new
                    if hit_not:
                        r["not"] = sorted({new if x == old else x for x in r["not"]})
                    self.labels[fid] = r
            self._commit(before, f"Merged {old} into {new}" if merge else f"Renamed {old} to {new}")
            return len(before)

    def undo(self):
        with self.lock:
            if not self.undo_stack:
                return None
            step = self.undo_stack.pop()
            for fid, old in step["before"]:
                if old:
                    self.labels[fid] = old
                else:
                    self.labels.pop(fid, None)
            self._save(f"Undo: {step['text']}", [f for f, _ in step["before"]])
            return step["text"]

    def _commit(self, before, text):
        if not before:
            return
        self.undo_stack.append({"before": before, "text": text})
        del self.undo_stack[:-200]
        self._save(text, [f for f, _ in before])

    def _save(self, text, fids):
        self.changed()
        F.save_json("labels.json", self.labels)
        with open(F.DATA / "history.jsonl", "a", encoding="utf-8") as h:
            h.write(json.dumps({"t": time.strftime("%Y-%m-%d %H:%M:%S"), "what": text,
                                "faces": {f: self.labels.get(f) for f in fids}}) + "\n")
        self.build_people()

    def build_people(self):
        """data/people.npz = face prints of every named, usable face: what the live tracker loads."""
        gal = {k: v for k, v in self.gallery().items() if k != IGNORE}
        idx = [i for faces in gal.values() for i in faces]
        who = [nm for nm, faces in gal.items() for _ in faces]
        E = self.E()
        tmp = F.DATA / "people.tmp.npz"
        np.savez(tmp, emb=E[idx] if idx else np.zeros((0, E.shape[1] if self.n else F.REC["dim"]), np.float32),
                 name=np.asarray(who, dtype=str), model=np.asarray(self.store.model))
        F.replace_file(tmp, F.DATA / "people.npz")
        return {nm: len(f) for nm, f in gal.items()}

    def autoname(self, idx=None):
        """Name unnamed faces the model is very sure about (score >= AUTO and clearly ahead of the
        next person). Returns how many. These are marked 'auto' and can be reviewed later."""
        with self.lock:
            want = None if idx is None else set(idx)
            hits = [(i, nm, s) for i, nm, s, s2 in self.triage()
                    if nm and s >= AUTO and s - s2 >= MARGIN and self.usable(i) and (want is None or i in want)]
            for i, nm, s in hits:
                r = dict(self.rec(i))
                r.update(n=nm, by="auto", s=round(s, 3))
                self.labels[self.store.fid[i]] = r
            if hits:
                self.changed()
                F.save_json("labels.json", self.labels)
                self.build_people()
            return len(hits)

    def backup(self, keep=20):
        src = F.DATA / "labels.json"
        if src.exists():
            d = F.DATA / "backups"
            d.mkdir(exist_ok=True)
            shutil.copy2(src, d / time.strftime("labels-%Y%m%d-%H%M%S.json"))
            for old in sorted(d.glob("labels-*.json"))[:-keep]:
                old.unlink()

    # ----- duplicates
    def same_faces(self, photo, embs, sim=0.90):
        """True if a photo's faces are the same faces as an already-read photo's (a copy of it):
        same number of faces, each with a near-identical face print."""
        with self.lock:
            old = self.by_photo().get(photo, [])
            if len(old) != len(embs):
                return False
            if not embs:
                return True
            S = np.asarray(embs, np.float32) @ self.E()[old].T
            return bool((S.max(1) >= sim).all())

    def near_duplicate(self, fp):
        """Content hash of an already-read photo that looks the same as fingerprint fp, else None."""
        if self._fp is None or len(self._fp_keys) != sum(1 for p in self.store.photos.values() if p.get("fp")):
            keys = [h for h, p in self.store.photos.items() if p.get("fp")]
            self._fp_keys = keys
            self._fp = np.array([np.frombuffer(bytes.fromhex(self.store.photos[h]["fp"]), np.uint8) for h in keys],
                                np.uint8).reshape(len(keys), 32)
        if not len(self._fp_keys):
            return None
        d = POP[self._fp ^ np.frombuffer(bytes.fromhex(fp), np.uint8)].sum(1, dtype=np.int32)
        k = int(np.argmin(d))
        return self._fp_keys[k] if d[k] <= DUP_BITS else None


# --------------------------------------------------------------------------- import
VIDEO = object()                 # load() result for a video: its faces are found frame by frame


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
        self._more_lock = threading.Lock()
        self._more_det = None
        self._more_cache = {}                     # (photo, frame) -> (image, possible-face rows)

    def start(self, folder):
        folder = Path(str(folder).strip().strip('"'))
        if not folder.is_dir():
            raise ValueError(f"Folder not found: {folder}")
        if self.thread and self.thread.is_alive():
            raise ValueError("An import is already running.")
        self.stop.clear()
        self.thread = threading.Thread(target=self.run, args=(folder,), daemon=True)
        self.thread.start()

    MORE_SCORE = 0.45                 # possible faces: detector confidence, far below F.DET_SCORE ...
    MORE_SIZES = (10_000, 640, 480, 320)   # ... at more scales: you look at what it finds and decide

    def _low_rows(self, img):
        """Every face the detector sees in img with the low cut-off MORE_SCORE, at MORE_SIZES."""
        with self._more_lock:                     # its own detector (an import may use the other), one at a time
            if self._more_det is None:
                self._more_det = cv2.FaceDetectorYN.create(str(F.DET_MODEL), "", (320, 320), self.MORE_SCORE, 0.3, 5000)
            rows = []
            h_, w_ = img.shape[:2]
            for side in self.MORE_SIZES:
                s = min(1.0, side / max(h_, w_))
                im = cv2.resize(img, (round(w_ * s), round(h_ * s)), interpolation=cv2.INTER_AREA) if s < 1 else img
                self._more_det.setInputSize((im.shape[1], im.shape[0]))
                _, found = self._more_det.detect(im)
                if found is not None:
                    found = found.copy()
                    found[:, :14] /= s
                    rows += list(found)
        rows.sort(key=lambda r: -r[14])
        kept = []
        for r in rows:
            if min(r[2], r[3]) >= F.MIN_FACE and all(F._iou(r, k) < 0.3 for k in kept):
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
        return h, rec, frame, img, [r for r in rows if all(F._iou(r, k) < 0.3 for k in have)]

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
            rows = [r for r in rows if F._iou(r, [float(v) for v in box]) > 0.8][:1]
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
            k = F._iou(r, [x, y, w, h])
            if k > best_iou:
                best, best_iou = r, k
        return best

    def scan_possible(self):
        """Look for possible faces in every photo added before possible faces were kept, in the background
        (progress in self.status, like an import). Skips videos, and online-only cloud files (they'd be
        downloaded just for this): those are looked at another time, once they're on this computer."""
        if self.thread and self.thread.is_alive():
            raise ValueError("An import is already running.")
        self.stop.clear()
        self.thread = threading.Thread(target=self._scan_possible, daemon=True)
        self.thread.start()

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
                        rows = [r for r in self._low_rows(img) if all(F._iou(r, b) < 0.3 for b in have)]
                        new = self._add_possible(h, rec, img, rows)
                        pending += new
                        st["faces"] += len(new)
                        st["new"] += bool(new)
                except Exception:
                    st["errors"] += 1
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
                except OSError:
                    st["errors"] += 1
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
                if make_local(f):
                    pinned.append(f)
                s = f.stat()
                h = file_hash(f)
                if h in store.photos:
                    return f, s, h, None
                if is_video(f):
                    return f, s, h, VIDEO               # read on this thread, frame by frame, below
                return f, s, h, F.load_image(f)

            with ThreadPoolExecutor(4) as ex:
                window = [ex.submit(load, f) for f in todo[:8]]
                nxt = 8
                while window and not self.stop.is_set():
                    fut = window.pop(0)
                    if nxt < len(todo):
                        window.append(ex.submit(load, todo[nxt]))
                        nxt += 1
                    try:
                        f, s, h, img = fut.result()
                    except Exception:
                        st["errors"] += 1
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
                    elif img is VIDEO:
                        st["stage"] = f"Reading video {f.name}"
                        try:
                            fps, found = F.video_faces(self.faces, f, self.stop)
                        except Exception:
                            fps, found = None, None
                        st["stage"] = "Reading photos"
                        if self.stop.is_set():
                            break                         # half-read: read again next time
                        if found is None:
                            st["errors"] += 1
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
                    elif img is None:
                        st["errors"] += 1
                    else:
                        fp = fingerprint(img)
                        with db.lock:
                            dup = db.near_duplicate(fp)
                        found = self.faces.detect(img)
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
                            # and the possible faces: what the detector sees with the low cut-off too
                            low = [r for r in self._low_rows(img) if all(F._iou(r, k[0]) < 0.3 for k in found)]
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
