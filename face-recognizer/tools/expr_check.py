r"""
expr_check.py - try the expression model on faces you've already scanned.

  python tools\expr_check.py            (200 random stored faces)
  python tools\expr_check.py 1000

Prints how long one face takes, how the labels are spread, how often the two mirror-image
views agree, and runs one full tracker request on a stored photo. Use it to check the model
is installed and working, and to get a feel for how it reads your photos.
"""
import json
import random
import sys
import time
from collections import Counter
from pathlib import Path

import cv2
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import face_id as F                     # noqa: E402


def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 200
    faces = F.Faces()
    print(f"Recognition on {faces.device}; expression model: {faces.expression_model or 'NOT INSTALLED'}; "
          f"face-shape cues: {'installed' if faces.cues_available else 'not installed'}")
    print("Models:", ", ".join(f"{k} on {v}" for k, v in faces.devices.items() if v))
    if not faces.expression_model:
        sys.exit(f"Put {F.EXPR_MODEL.name} in {F.MODELS}")
    store = F.Store()
    total = len(store.thumbs)
    idx = random.Random(1).sample(range(total), min(n, total))
    print(f"Scoring {len(idx)} of {total} stored faces...")

    labels, t_sum, flips_agree, sure = Counter(), 0.0, 0, 0
    scored = []                                  # (label, top share, clear?, mirror agrees?, quality, face width)
    for i in idx:
        crop = store.thumb(i)
        t0 = time.perf_counter()
        p = faces.expression(crop)
        t_sum += time.perf_counter() - t0
        lab = F.EXPRESSIONS[int(np.argmax(p))]
        labels[lab] += 1
        clear = p.max() >= 0.45 and np.sort(p)[-1] - np.sort(p)[-2] >= 0.12
        sure += clear
        # does the plain view agree with the mirrored view? (the method averages both)
        x = (cv2.cvtColor(crop, cv2.COLOR_BGR2RGB).astype(np.float32) - 127.5) / 127.5
        x = np.ascontiguousarray(x.transpose(2, 0, 1)[None])
        a = int(np.argmax(faces._xrun(x))); b = int(np.argmax(faces._xrun(np.ascontiguousarray(x[..., ::-1]))))
        flips_agree += a == b
        scored.append((lab, float(p.max()), clear, a == b, float(store.quality[i]), float(store.box[i][2])))
    k = len(idx)
    print(f"{t_sum / k * 1000:.2f} ms per face (both views)")
    print("Labels:", ", ".join(f"{l} {c * 100 / k:.0f}%" for l, c in labels.most_common()))
    print(f"Clear winner (>=45% and 12 points ahead): {sure * 100 / k:.0f}%   mirror views agree: {flips_agree * 100 / k:.0f}%")

    # Where the model is weak: by label, by face quality and by face size in the photo
    def report(title, groups):
        print(f"\n{title:<18} {'faces':>6} {'avg top':>8} {'clear':>6} {'mirror':>7}")
        for name, rows in groups:
            if rows:
                print(f"{name:<18} {len(rows):>6} {np.mean([r[1] for r in rows]):>8.2f} "
                      f"{np.mean([r[2] for r in rows]) * 100:>5.0f}% {np.mean([r[3] for r in rows]) * 100:>6.0f}%")
    report("By label", [(l, [r for r in scored if r[0] == l]) for l in F.EXPRESSIONS])
    report("By quality", [(f"{name} ({lo}-{hi})", [r for r in scored if lo <= r[4] < hi]) for name, lo, hi in
                          (("blurry", 0, F.Q_EXPR), ("fair", F.Q_EXPR, 0.4), ("sharp", 0.4, 9))]
           + [("unknown", [r for r in scored if np.isnan(r[4])])])
    report("By face size (px)", [(f"{lo}-{hi}", [r for r in scored if lo <= r[5] < hi])
                                 for lo, hi in ((0, 80), (80, 160), (160, 320), (320, 99999))])

    # one full tracker request on a stored photo, as the page would send it
    import tracker
    eng = tracker.Engine()
    photos = json.loads((F.DATA / "photos.json").read_text(encoding="utf-8"))
    with_faces = [ph for ph in photos.values() if ph.get("faces")]
    for ph in random.Random(1).sample(with_faces, min(20, len(with_faces))):
        path = Path(ph["path"])
        if path.exists():
            img = F.load_image(path)
            if img is None:
                continue
            res = eng.recognize(cv2.imencode(".jpg", img)[1].tobytes(), F.MATCH, cues=True)
            if not res["faces"]:
                continue
            print(f"\nTracker request on one photo: {len(res['faces'])} faces in {res['ms']} ms (with cues)")
            for f in res["faces"]:
                e = f["expr"]
                print(f"  {f['name']:<14} quality {f['quality']}  turn {f['turn']:.2f}  expression:",
                      f"skipped ({f['xskip']})" if e is None else f"{F.EXPRESSIONS[int(np.argmax(e))]} {max(e) * 100:.0f}%")
                if f["cues"]:
                    print("    cues:", ", ".join(f"{c} {v:.2f}" for c, v in f["cues"].items() if v >= 0.2) or "all low")
            break


if __name__ == "__main__":
    main()
