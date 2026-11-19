"""
photofacts.py - what a photo says about itself: camera dates and people tags (XMP from photo apps, Google
Takeout .json), matched to the faces found, read in the background (PhotoFacts). Split out of labels.py;
`labels` re-exports it.
"""
import bisect
import hashlib
import json
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


import face_id as F


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


def match_regions(regions, boxes):
    """Pair tagged face regions (pixel boxes) with found faces: greedy by overlap, at least 0.2.
    Tag regions are drawn looser than the detector's boxes, so a face well inside a region also counts."""
    pairs = []
    for a, r in enumerate(regions):
        for b, f in enumerate(boxes):
            s = F.iou(r, f)
            cx, cy = f[0] + f[2] / 2, f[1] + f[3] / 2
            if s < 0.2 and r[0] <= cx <= r[0] + r[2] and r[1] <= cy <= r[1] + r[3] and f[2] * f[3] >= 0.15 * r[2] * r[3]:
                s = 0.2
            if s >= 0.2:
                pairs.append((s, a, b))
    used_a, used_b, out = set(), set(), {}
    for _s, a, b in sorted(pairs, reverse=True):
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
        self._names = {}                 # (folder, ending) -> that listing's file names, sorted (_beside)
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
        side = [(k, v.stat().st_mtime) for ext in (".xmp", ".json")
                for k, v in self._beside(p.parent, ext, (p.stem.lower(), n[:40]))]
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

    def _beside(self, folder, ext, starts):
        """The files of `folder` ending in ext whose names begin with one of `starts`: [(name, path)] in name
        order, found in the folder's sorted names. Before 1.24.8 key() sorted and went through the folder's whole
        list for every photo: in a folder of 10,000 photos with a file beside each (the Collect faces folder:
        every snapshot has its verdict .json) that was 5.6 ms a photo, a minute of pure Python at every start
        and growing with the square of the folder, while the sync and the pages waited for the interpreter."""
        k = (folder, ext)
        listing = self._listing(folder, ext)
        names = self._names.get(k)
        if names is None or len(names) != len(listing):
            names = self._names[k] = sorted(listing)
        found = set()
        for start in starts:
            i = bisect.bisect_left(names, start)
            while i < len(names) and names[i].startswith(start):
                found.add(names[i])
                i += 1
        return [(name, listing[name]) for name in sorted(found)]

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
