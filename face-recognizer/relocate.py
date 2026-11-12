"""
relocate.py - finding photos again: the one-time upgrade of databases from before per-face labels (migrate),
and photos that moved since they were added, e.g. a changed drive letter (relocate, at each start).
Split out of labels.py; `labels` re-exports it.
"""
import hashlib
import os
import shutil
import string
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


import face_id as F

from photofiles import file_hash, fingerprint, remember_folder, settings
import paths

IGNORE = "#ignore"       # = labels.IGNORE (repeated here: labels imports this module)


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

    old = F.DATA / "backups" / ("legacy-" + paths.utc_stamp())
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
