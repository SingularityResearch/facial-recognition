"""
webcam_eval.py - how well the live tracker names people on real webcam faces, and which way of
combining frames works best.

Test faces: webcam snapshots saved by the tracker's "Collect faces for labeling" and confirmed by you in
the labeling tool (named "by me"). People are recognized from their photo faces only (no webcam faces),
so a webcam face is never matched against itself or its neighbours.

Each test face becomes a simulated track of FRAMES webcam frames, each spoiled at random the way live
frames are (blur, motion blur, low resolution, noise, JPEG, light changes). Then, per track:
  vote      the tracker today: each frame names someone; names are tallied, weighted by face quality
            and similarity (as tracker_ui.html updateTracks does)
  average   the frames' face prints are averaged, weighted by face quality, and matched once
  avg+flip  as average, with each frame's print the mean of the face and its mirror image
  frame     one frame on its own (the per-frame rate, for reference)
For each: correct, Unknown, wrong person, and "stranger named": the same tracks with that person
left out of the known people, where any name is a mistake. Also "real 3", on the real snapshots: three
consecutive snapshots of one visit, unspoiled.

Run:  python tools/webcam_eval.py [--frames 7] [--threshold 0.35]
Writes data/webcam_eval.json (numbers only).
"""
import argparse
import json
import sys
from pathlib import Path

import cv2
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import face_id as F  # noqa: E402
import labels as L  # noqa: E402


def spoil(img, rng):
    """A random live-webcam version of an aligned 112x112 face crop."""
    out = img.astype(np.float32)
    kind = rng.integers(4)
    if kind == 0:                                     # out of focus
        out = cv2.GaussianBlur(out, (0, 0), rng.uniform(1.0, 3.0))
    elif kind == 1:                                   # moving
        n = int(rng.integers(5, 13))
        k = np.zeros((n, n), np.float32)
        k[n // 2, :] = 1 / n
        rot = cv2.getRotationMatrix2D((n / 2 - 0.5, n / 2 - 0.5), rng.uniform(0, 180), 1)
        k = cv2.warpAffine(k, rot, (n, n))
        out = cv2.filter2D(out, -1, k / max(k.sum(), 1e-6))
    elif kind == 2:                                   # far away: few pixels on the face
        s = int(rng.integers(28, 56))
        out = cv2.resize(cv2.resize(out, (s, s), interpolation=cv2.INTER_AREA), (112, 112))
    out = out * rng.uniform(0.6, 1.3) + rng.uniform(-25, 25)        # light changes
    out = out + rng.normal(0, rng.uniform(3, 12), out.shape)        # sensor noise
    out = np.clip(out, 0, 255).astype(np.uint8)
    ok, jpg = cv2.imencode(".jpg", out, [cv2.IMWRITE_JPEG_QUALITY, int(rng.integers(35, 85))])
    return cv2.imdecode(jpg, cv2.IMREAD_COLOR)


def unit(v):
    return v / (np.linalg.norm(v) + 1e-9)


def vote(ident, prints, qs, thr):
    """The tracker's rule: per-frame names, tallied by quality * (1 + similarity)."""
    tally = {}
    for e, q in zip(prints, qs):
        n, s = ident.who(e, thr)
        tally[n] = tally.get(n, 0) + max(0.05, q) * (1 + s)
    return max(tally, key=tally.get)


def average(ident, prints, qs, thr):
    w = np.maximum(0.05, np.asarray(qs))[:, None]
    return ident.who(unit((np.asarray(prints) * w).sum(0)), thr)[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--frames", type=int, default=7, help="frames per simulated track (the tracker votes over 7)")
    ap.add_argument("--threshold", type=float, default=F.MATCH)
    ap.add_argument("--seed", type=int, default=0)
    a = ap.parse_args()
    rng = np.random.default_rng(a.seed)

    db = L.LabelDB(backup=False, log=lambda *_: None)
    store = db.store
    webcam = lambda i: Path(store.photos.get(store.photo[i], {}).get("path") or "").parent == F.WEBCAM
    tests = [i for i in range(db.n) if webcam(i) and db.rec(i).get("by") == "me" and db.name(i) not in (None, L.IGNORE)]
    gal = [i for i in range(db.n) if not webcam(i) and db.name(i) not in (None, L.IGNORE) and db.usable(i)]
    if not tests:
        sys.exit("No confirmed webcam faces yet. Turn on \"Collect faces for labeling\" in the tracker, then "
                 "confirm the snapshots' names in the labeling tool (Suggestions / Named automatically).")
    E = db.E()
    names = np.array([db.name(i) for i in gal])
    known = F.Identifier(E[gal], names.tolist(), a.threshold)
    without = {p: F.Identifier(E[[g for g, n in zip(gal, names) if n != p]], [n for n in names if n != p], a.threshold)
               for p in {db.name(i) for i in tests}}
    people = sorted({db.name(i) for i in tests})
    print(f"{len(tests)} confirmed webcam faces of {', '.join(people)}; recognized from {len(gal)} photo faces "
          f"of {len(set(names))} people; match setting {a.threshold:.2f}; {a.frames} frames per track")

    faces = F.Faces()
    methods = ("frame", "vote", "average", "avg+flip")
    res = {m: {"known": [], "stranger": []} for m in methods}
    truths = []
    for i in tests:
        truth, crop = db.name(i), store.thumb(i)
        frames = [spoil(crop, rng) for _ in range(a.frames)]
        prints = [faces.embed(f) for f in frames]
        flips = [unit(p + faces.embed(cv2.flip(f, 1))) for p, f in zip(prints, frames)]
        qs = [faces.quality(f) for f in frames]
        qs = [0.5 if np.isnan(q) else q for q in qs]
        for key, ident in (("known", known), ("stranger", without[truth])):
            res["frame"][key] += [ident.who(p, a.threshold)[0] for p in prints]
            res["vote"][key].append(vote(ident, prints, qs, a.threshold))
            res["average"][key].append(average(ident, prints, qs, a.threshold))
            res["avg+flip"][key].append(average(ident, flips, qs, a.threshold))
        truths.append(truth)

    def rates(got, key):
        t = truths if len(got) == len(truths) else [x for x in truths for _ in range(a.frames)]
        n = len(got) / 100
        if key == "stranger":
            return {"stranger named": round(sum(g != "Unknown" for g in got) / n, 1)}
        return {"correct": round(sum(g == x for g, x in zip(got, t)) / n, 1),
                "unknown": round(sum(g == "Unknown" for g in got) / n, 1),
                "wrong": round(sum(g not in (x, "Unknown") for g, x in zip(got, t)) / n, 1)}

    out = {"faces": len(tests), "people": people, "frames": a.frames, "threshold": a.threshold, "simulated": {}}
    print(f"\n{'':10} {'correct':>8} {'unknown':>8} {'wrong':>6} {'stranger named':>15}")
    for m in methods:
        r = {**rates(res[m]["known"], "known"), **rates(res[m]["stranger"], "stranger")}
        out["simulated"][m] = r
        print(f"{m:10} {r['correct']:7.1f}% {r['unknown']:7.1f}% {r['wrong']:5.1f}% {r['stranger named']:14.1f}%")

    # Real snapshots: three consecutive ones from the same visit (saved <= 30 s apart), unspoiled
    when = {i: store.photos.get(store.photo[i], {}).get("mtime", 0) for i in tests}
    seq = sorted(tests, key=lambda i: when[i])
    trios = [seq[k:k + 3] for k in range(len(seq) - 2) if when[seq[k + 2]] - when[seq[k]] <= 30
             and len({db.name(j) for j in seq[k:k + 3]}) == 1]
    if trios:
        real = {"vote": [], "average": []}
        for t in trios:
            prints, qs = [E[j] for j in t], [db.q(j) or 0.5 for j in t]
            real["vote"].append(vote(known, prints, qs, a.threshold) == db.name(t[0]))
            real["average"].append(average(known, prints, qs, a.threshold) == db.name(t[0]))
        out["real3"] = {k: round(100 * float(np.mean(v)), 1) for k, v in real.items()}
        out["real3"]["tracks"] = len(trios)
        print(f"\nreal 3 ({len(trios)} runs of three real snapshots): vote {out['real3']['vote']}% correct, "
              f"average {out['real3']['average']}% correct")
    (F.DATA / "webcam_eval.json").write_text(json.dumps(out, indent=1))
    print("\nSaved data/webcam_eval.json")


if __name__ == "__main__":
    main()
