"""
eval_models.py - compare recognition models on your own labeled faces (runs on the GPU).

Models: ArcFace R50, AdaFace IR-101, and "ArcFace+AdaFace" (both face prints joined, so the score is
the average of the two models' similarities). For each it reports, per match setting:
  correct / unknown / stranger named   on your labeled faces (leave-one-out, same check as before)
and on simulated bad webcam frames (blur, tiny, motion+noise).
Run:  python tools/eval_models.py
"""
import json
import sys
import time
from pathlib import Path

import cv2
import numpy as np
import onnxruntime as ort

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import face_id as F  # noqa: E402

MODELS = F.MODELS
rng = np.random.default_rng(0)


def session(name):
    return ort.InferenceSession(str(MODELS / name), providers=["DmlExecutionProvider", "CPUExecutionProvider"])


def embedder(sess, bgr_order=False):
    iname = sess.get_inputs()[0].name

    def run(crops):
        x = np.stack([c if bgr_order else c[:, :, ::-1] for c in crops]).astype(np.float32)
        x = ((x - 127.5) / 127.5).transpose(0, 3, 1, 2).copy()
        # one face per call: DirectML mishandles ArcFace's BatchNorm with batches > 1 (and the
        # live tracker sends one face at a time anyway)
        out = np.vstack([sess.run(None, {iname: x[i:i + 1]})[0] for i in range(len(x))])
        return out / np.linalg.norm(out, axis=1, keepdims=True)
    return run


def degrade(img, kind):
    if kind == "blur":
        return cv2.GaussianBlur(img, (0, 0), 3.0)
    if kind == "tiny":
        return cv2.resize(cv2.resize(img, (20, 20), interpolation=cv2.INTER_AREA), (112, 112))
    k = np.zeros((15, 15), np.float32)
    k[7, :] = 1 / 15
    return np.clip(cv2.filter2D(img, -1, k) + rng.normal(0, 18, img.shape), 0, 255).astype(np.uint8)


def check(G, D, lab, named, use, thr):
    """Leave-one-out: G = gallery prints, D = query prints for the named faces (row k = named[k])."""
    ok = unk = wrong = stranger = 0
    for k, i in enumerate(named):
        keep = [j for j in use if j != i]
        p, _ = F.Identifier(G[keep], lab[keep].tolist(), thr).who(D[k])
        ok += p == lab[i]
        unk += p == "Unknown"
        wrong += p not in (lab[i], "Unknown")
        k2 = [j for j in keep if lab[j] != lab[i]]
        stranger += F.Identifier(G[k2], lab[k2].tolist(), thr).who(D[k])[0] != "Unknown"
    n = len(named) / 100
    return ok / n, unk / n, wrong / n, stranger / n


def main():
    store = F.Store()
    crops = [store.thumb(i) for i in range(len(store.emb))]
    import labels as L
    db = L.LabelDB(backup=False)
    lab = np.array([n if n and n != L.IGNORE else "" for n in map(db.name, range(db.n))])
    named = list(np.where(lab != "")[0])
    q = np.asarray(store.quality)
    use = [j for j in named if not q[j] < F.Q_ENROLL]
    print(f"{len(named)} labeled faces, {len(set(lab[named]))} people; providers {ort.get_available_providers()}")

    arc = embedder(session("arcface_w600k_r50.onnx"))
    ada_s = session("adaface_ir101_webface12m.onnx")
    ada_rgb, ada_bgr = embedder(ada_s), embedder(ada_s, bgr_order=True)

    # which colour order does this AdaFace export expect? pick the one that separates people best
    sep = {}
    for nm, fn in (("RGB", ada_rgb), ("BGR", ada_bgr)):
        E = fn([crops[i] for i in named])
        M = E @ E.T
        same = lab[named][:, None] == lab[named][None, :]
        iu = np.triu_indices(len(named), 1)
        g, b = M[iu][same[iu]], M[iu][~same[iu]]
        sep[nm] = (g.mean() - b.mean()) / np.sqrt((g.var() + b.var()) / 2)
    ada = ada_rgb if sep["RGB"] >= sep["BGR"] else ada_bgr
    print(f"AdaFace colour order: RGB d'={sep['RGB']:.2f}  BGR d'={sep['BGR']:.2f}")

    models = {"ArcFace R50": arc, "AdaFace IR-101": ada}
    G = {nm: fn(crops) for nm, fn in models.items()}
    G["ArcFace+AdaFace"] = np.hstack([G["ArcFace R50"], G["AdaFace IR-101"]]) / np.sqrt(2)

    # speed on the GPU, one face at a time (live tracker case)
    for nm, fn in models.items():
        fn(crops[:1])
        t = time.perf_counter()
        for i in range(50):
            fn(crops[i:i + 1])
        print(f"{nm}: {(time.perf_counter() - t) / 50 * 1000:.1f} ms/face on GPU")

    results = {}
    for nm, E in G.items():
        M = E[named] @ E[named].T
        same = lab[named][:, None] == lab[named][None, :]
        iu = np.triu_indices(len(named), 1)
        g, b = M[iu][same[iu]], M[iu][~same[iu]]
        dprime = (g.mean() - b.mean()) / np.sqrt((g.var() + b.var()) / 2)
        # lowest setting that names no stranger and no wrong person on clean faces
        best = None
        for thr in np.arange(0.15, 0.61, 0.01):
            r = check(E, E[named], lab, named, use, thr)
            if r[3] == 0 and r[2] == 0:
                best = (round(float(thr), 2), r)
                break
        thr = best[0] + 0.02            # small safety margin above the edge
        clean = check(E, E[named], lab, named, use, thr)
        rows = {"clean": clean}
        for kind in ("blur", "tiny", "motion"):
            bad = [degrade(crops[i], kind) for i in named]
            if nm == "ArcFace+AdaFace":
                D = np.hstack([arc(bad), ada(bad)]) / np.sqrt(2)
            else:
                D = models[nm](bad)
            rows[kind] = check(E, D, lab, named, use, thr)
        results[nm] = dict(dprime=round(float(dprime), 2), threshold=round(thr, 2), **{k: [round(v, 1) for v in r] for k, r in rows.items()})
        print(f"\n{nm}: separation d'={dprime:.2f}, match setting {thr:.2f}")
        for k, r in rows.items():
            print(f"  {k:6s} correct {r[0]:5.1f}%  unknown {r[1]:5.1f}%  wrong {r[2]:4.1f}%  stranger named {r[3]:4.1f}%")
    (F.DATA / "model_eval.json").write_text(json.dumps(results, indent=1))


if __name__ == "__main__":
    main()
