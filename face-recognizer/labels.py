"""
labels.py - the face-labeling engine shared by label_tool.py (the labeling page) and face_id.py.

Every face has a permanent ID (its photo's content fingerprint + where the face is in the photo),
and its label is stored on the face itself in data/labels.json:

  {"<face id>": {"n": "Ann", "by": "me"}, ...}
    n    the person's name, or "#ignore" (stranger, artwork, not a face: never recognized; its face print and
         picture are deleted, only where it is in its photo is kept: see LabelDB.forget_faces)
    by   "me" (you named it) or "auto" (named automatically because the model was very sure)
    not  names you said this face is NOT, so they are never suggested for it again
    ok   you looked at a "might be mislabeled" warning and kept the name
    s    the model's score when the face was named automatically

Groups are no longer stored. For faces nobody has named, the engine works out:
  suggested  the model thinks it knows who it is (score >= the match setting): you confirm
  unknown    nobody known is close: shown in groups of look-alike faces for you to name
Every change is logged to data/history.jsonl and can be undone; labels.json is backed up at
each start of the labeling page (data/backups/, last 20 kept).

This module holds the labels themselves (LabelDB). The rest of the engine is in photofiles.py (hashes,
fingerprints, cloud files, folders), photofacts.py (camera dates, people tags), relocate.py (upgrade, moved
photos) and importer.py (adding photos and videos); all of it is re-exported here, so `labels.X` still works.
"""
import json
import shutil
import threading
import time
from pathlib import Path

import numpy as np

import face_id as F
import paths

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
SAME = 0.95        # face prints this alike are the same picture again (LabelDB.stacks)
FACE_SHOW = 0.25   # share of possible faces drawn as dashed boxes: the most face-like (LabelDB.face_cut)

# The rest of the labeling engine, in its own modules; re-exported so `labels.X` keeps working.
from photofiles import (CLOUD, file_hash, fingerprint, make_local, needs_possible, online_only,  # noqa: E402,F401
                        photo_date, remember_folder, settings, unpin)
from photofacts import (CLOSEST, FOLDER_NEAR, TAKEN_NEAR_H, PhotoFacts, match_regions,  # noqa: E402,F401
                        xmp_people)
from relocate import migrate, register_faceless, relocate  # noqa: E402,F401


def _named_again(name):
    """A name given by hand is a person again: an earlier forget of that name (persondata.forget) no longer applies
    to them, on this device or, with a shared database, on the others (dbsync_home.unforget)."""
    try:
        import dbsync_home
        dbsync_home.unforget(name)
    except (ImportError, OSError):
        pass


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
        self.version_faces = 0           # bumps when faces or their prints change (the faceness model trains again)
        self._fj_thread = None
        self.faceness_inline = False     # tests: train the faceness model in the calling thread
        if backup:
            self.backup()
        kept = [i for i in range(self.n) if self.name(i) == IGNORE and not self.store.printless(i)]
        if kept:                         # Ignored before v1.9, when their prints and pictures were still kept
            saved = self._backup_before_forgetting()
            self.forget_faces(kept)
            log(f"Deleted the face prints and pictures of {len(kept)} ignored faces (only where they are is kept). "
                f"A copy from before is in {saved}: delete it once you're happy.")

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
        return not q < F.Q_ENROLL and not self.printless(i)     # nan = not scored = usable

    def printless(self, i):
        """True if face i's print and picture were deleted (Ignored, or a forgotten person's): it can't be
        scored, grouped or teach the recognizer."""
        if "gone" not in self._cache:
            E = self.E()
            self._cache["gone"] = set(np.flatnonzero(~E.any(1)).tolist()) if self.n else set()
        return i in self._cache["gone"]

    def forget_faces(self, idx):
        """Delete the prints and pictures of faces idx (Store.forget) and save. Only where each face is stays
        (face ID, photo, box): its photo is already in photos.json, so it isn't read again, and a re-saved copy
        of it still counts as a copy (same_faces). Returns {face ID: (print, picture)} for Undo, this session."""
        with self.lock:
            saved = {self.store.fid[i]: self.store.forget(i) for i in idx}
            self._forgot()
            self.store.save()
        return saved

    def _forgot(self):
        for k in ("E", "E_n", "gone", "st", "st_n"):
            self._cache.pop(k, None)
        self.version_faces += 1          # the faceness model trains again, in the background (kept meanwhile)
        self.groups = None
        self.changed()

    def rec(self, i):
        return self.labels.get(self.store.fid[i], {})

    def name(self, i):
        return self.rec(i).get("n")

    def changed(self):
        self.version += 1
        # kept: face prints, and the faceness model (retrained when faces are added, not on every label)
        self._cache = {k: v for k, v in self._cache.items()
                       if k in ("E", "E_n", "fm", "fs", "fc", "fj", "st", "st_n")}

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
            idx = [i for i in range(self.n) if not self.name(i) and not self.printless(i)]
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
        possible faces you named; junk = possible faces not named (Ignored ones have no print left; most possible faces aren't
        faces: 85% of a hand-sorted sample of 160). Faces drawn by hand count as neither."""
        pos, neg = [], []
        for i in range(self.n):
            nm, mb = self.name(i), self.maybe(i)
            if self.printless(i):                       # Ignored faces: no print left to learn from
                continue
            if mb is None:
                if nm != IGNORE:
                    pos.append(i)
            elif mb > 0:
                (neg if not nm or nm == IGNORE else pos).append(i)
        return pos, neg

    @staticmethod
    def _fit(E, pos, neg):
        """liblinear, not the default L-BFGS: the same L2 logistic regression (scores correlate 1.0 on 7,966 faces,
        99.7% the same possible faces shown), single-threaded C that lets the other threads run. In the built app on
        the Linux laptop, L-BFGS (SciPy and OpenBLAS threads competing with the live tracker's) held Python's lock
        for minutes, and the whole window stopped responding (2026-11-12)."""
        from sklearn.linear_model import LogisticRegression
        return LogisticRegression(C=0.1, max_iter=5000, class_weight="balanced", solver="liblinear").fit(
            E[pos + neg], [1] * len(pos) + [0] * len(neg))

    def _face_model(self):
        """Logistic regression on face prints, real faces against possible faces (_face_sets), for faces it
        hasn't seen (the photo view's on-the-spot boxes). None when there's too little to learn from, or while it's
        being trained (see _faceness_job)."""
        self._faceness_job()
        return self._cache.get("fm")

    def faceness(self, emb):
        """'Looks like a real face' score (0..1) for face prints (rows of emb), or None without a model."""
        m = self._face_model()
        return None if m is None else m.predict_proba(np.atleast_2d(emb))[:, 1]

    def _faceness_job(self):
        """Train the faceness model and the possible faces' scores in the background when the faces changed.
        Training takes seconds on thousands of faces (minutes on a slow computer) and must not hold self.lock: the
        labeling page and the shared database's sync wait on it. Until it's done, the previous model and scores
        are used; with none yet, every possible face is shown. Call with self.lock held."""
        want = (self.n, self.version_faces)
        if self._cache.get("fj") == want or (self._fj_thread is not None and self._fj_thread.is_alive()):
            return
        self._cache["fj"] = want
        E = self.E().copy()
        pos, neg = self._face_sets()
        idx = [i for i in range(self.n) if (self.maybe(i) or 0) > 0]

        def train():
            try:
                fit()
            except Exception as e:                                    # keep the previous model; say why once
                print(f"Couldn't train the faceness model: {e}")

        def fit():
            model, scores = None, {}
            if len(pos) >= 50 and len(neg) >= 50:
                model = self._fit(E, pos, neg)
                if idx:
                    negset = set(neg)
                    folds = [neg[k::5] for k in range(5)]
                    for k, fold in enumerate(folds):
                        m = self._fit(E, pos, [j for kk, f in enumerate(folds) if kk != k for j in f])
                        for j, v in zip(fold, m.predict_proba(E[fold])[:, 1].tolist()):
                            scores[j] = v
                    rest = [j for j in idx if j not in negset]        # possible faces you named: full model
                    if rest:
                        for j, v in zip(rest, model.predict_proba(E[rest])[:, 1].tolist()):
                            scores[j] = v
            with self.lock:
                if self._cache.get("fj") == want:                     # still the faces it was trained on
                    self._cache["fm"] = model
                    self._cache["fs"] = scores
                    self._cache["fc"] = float(np.quantile(list(scores.values()), 1 - FACE_SHOW)) if scores else 0.0
                    self.changed()
        if self.faceness_inline:                                       # tests: train right here
            train()
            return
        self._fj_thread = threading.Thread(target=train, daemon=True, name="faceness")
        self._fj_thread.start()

    def face_cut(self):
        """The faceness a possible face needs for its dashed box: the score of the top FACE_SHOW share of
        possible faces. Each possible face is scored by a model trained without it (5 folds), so it isn't
        simply learned as junk. On 160 hand-sorted possible faces (24 real; AUC 0.84), showing the top 25%
        kept 17 of the 24 real faces and 21 of the other 136, about half of those drawings and game
        characters (to a face print, a drawn face is a face): about 9 in 10 non-face objects get no box.
        Top 35% kept 18 real and 32 others, top 50% 21 and 47. 0 (show all) without enough to learn from, or
        until the first background training is done."""
        self._faceness_job()
        return self._cache.get("fc", 0.0)

    def face_score(self, i):
        """Faceness of possible face i (1.0 for faces drawn by hand and faces that aren't possible faces)."""
        self.face_cut()
        return self._cache.get("fs", {}).get(i, 1.0)

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
        best = max(saved.get("faces", []), key=lambda f: F.iou(f["box"], box), default=None)
        if not best or F.iou(best["box"], box) < 0.3:
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
            for i in idx:                        # all of them first: a bad one mustn't leave the others half done
                if not (0 <= i < self.n):
                    raise ValueError(f"no face {i}")
            for i in idx:
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
            if action == "ignore" and before:    # an ignored face's print and picture aren't kept
                self.undo_stack[-1]["prints"] = self.forget_faces(idx)
        if action == "name" and idx:
            _named_again(name)
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
                    if r.get("n") == new and r.get("not"):      # merged: a face can't be Bob and "not Bob"
                        r["not"] = [x for x in r["not"] if x != new]
                    self.labels[fid] = {k: v for k, v in r.items() if v}
            self._commit(before, f"Merged {old} into {new}" if merge else f"Renamed {old} to {new}")
        if before:
            _named_again(new)
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
            if step.get("prints"):               # Undo of Ignore: the print and picture come back (kept in memory)
                idx_of = {f: i for i, f in enumerate(self.store.fid)}
                for fid, kept in step["prints"].items():
                    if fid in idx_of:
                        self.store.restore(idx_of[fid], kept)
                self._forgot()
                self.store.save()
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
            h.write(json.dumps({"t": paths.utc_now(), "what": text,
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

    @staticmethod
    def _backup_before_forgetting():
        """One copy of faces.npz and thumbs.npz before the v1.9 change deletes ignored faces' prints and pictures
        (labels.json backups don't hold them, so it couldn't be undone otherwise). Returns the folder."""
        d = F.DATA / "backups" / ("before-v1.9-ignore-" + paths.utc_stamp())
        d.mkdir(parents=True, exist_ok=True)
        for name in ("faces.npz", "thumbs.npz"):
            if (F.DATA / name).exists():
                shutil.copy2(F.DATA / name, d / name)
        return d

    def backup(self, keep=20):
        src = F.DATA / "labels.json"
        if src.exists():
            d = F.DATA / "backups"
            d.mkdir(exist_ok=True)
            shutil.copy2(src, d / f"labels-{paths.utc_stamp()}.json")
            for old in sorted(d.glob("labels-*.json"))[:-keep]:
                old.unlink()

    # ----- duplicates
    def same_faces(self, photo, embs, sim=0.90):
        """True if a photo's faces are the same faces as an already-read photo's (a copy of it):
        same number of faces, each with a near-identical face print."""
        with self.lock:
            # the photo's regular faces: embs are the new photo's regular faces too. Its possible and drawn faces
            # (maybe) were counted here before, so a copy of any photo that had one never matched and came in again
            old = [i for i in self.by_photo().get(photo, []) if self.maybe(i) is None]
            if len(old) != len(embs):
                return False
            if not embs:
                return True
            # Ignored faces have no print left: they can't be compared, so the photo's other faces decide (with
            # only Ignored faces, the picture fingerprint alone): else a copy would bring an ignored face back
            keep = [i for i in old if not self.printless(i)]
            if not keep:
                return True
            S = np.asarray(embs, np.float32) @ self.E()[keep].T
            if len(keep) < len(old):             # each remaining old face is among the new ones
                return bool((S.max(0) >= sim).all())
            return bool((S.max(1) >= sim).all())

    def near_duplicate(self, fp):
        """Content hash of an already-read photo that looks the same as fingerprint fp, else None."""
        # copies already skipped (dup_of) aren't compared with: a third version closest to such a copy was matched to
        # it, found "different faces" (a skipped copy has none) and came in whole
        if self._fp is None or len(self._fp_keys) != sum(1 for p in self.store.photos.values()
                                                          if p.get("fp") and not p.get("dup_of")):
            keys = [h for h, p in self.store.photos.items() if p.get("fp") and not p.get("dup_of")]
            self._fp_keys = keys
            self._fp = np.array([np.frombuffer(bytes.fromhex(self.store.photos[h]["fp"]), np.uint8) for h in keys],
                                np.uint8).reshape(len(keys), 32)
        if not len(self._fp_keys):
            return None
        d = POP[self._fp ^ np.frombuffer(bytes.fromhex(fp), np.uint8)].sum(1, dtype=np.int32)
        k = int(np.argmin(d))
        return self._fp_keys[k] if d[k] <= DUP_BITS else None


from importer import PROBLEMS_KEPT, VIDEO, Importer, is_video, note_problem  # noqa: E402,F401
