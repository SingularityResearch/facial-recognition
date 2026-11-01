r"""
expr_eval.py - how well expression reading and the other signals work on your own face and voice.

Record data first, on the tracker page -> Expression test:
  - Start test: you're asked to make each expression for a few seconds, then to talk for a few
    seconds (neutral, then smiling). Only numbers are saved, in data\expr_eval\<session>.jsonl.
  - Feeling check: now and then you say how you feel (unpleasant..pleasant, calm..energized, 1-7);
    saved with a summary of the signals in the 30 s before the question, in data\expr_eval\feel-*.jsonl.

  python tools\expr_eval.py                 (everything saved)
  python tools\expr_eval.py 20261030-213000 (one posed session; feeling reports are always all used)

Prints:
  1. per expression asked for: how often the model's top label was right, frame by frame and after
     the tracker's vote (same weighting and X_SURE / X_MARGIN gates as expression.js), and after the
     state model (H_STAY as in expression.js), plus what it was mistaken for
  2. confidence calibration: how often a reading at a given share is right, before and after a fitted
     temperature; writes data\expr_calibration.json (the tracker shows it as "cal NN%")
  3. accuracy by condition: person, face quality, size, head turn and brightness
  4. talking: how often talking faces are misread, how well the jaw-movement detector (TALK_JAW_SD in
     signals.js) and the microphone tell talking from not, and what the talking gate would do
  5. body: how well the hand-on-face detector works and what a hand on the chin does to the reading
     (the test's 'chin on hand' step), and posture by expression asked for
  6. face-shape cues by expression
  7. feeling reports: which signals go with how you said you felt, and a valence/arousal fit per
     person (and shared) that's kept only if it beats guessing your average; writes data\va_fit.json
Use it before changing any expression threshold or turning on the experimental options.
Posed expressions are exaggerated compared with everyday ones, so treat 1-5 as an upper bound.
"""
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import face_id as F                     # noqa: E402

X_SURE, X_MARGIN, X_MIN_VOTES, X_WINDOW = 0.45, 0.12, 3, 10   # as in expression.js
H_STAY, H_FULL_Q = 0.9, 0.5                                   # state model, as in expression.js
# Textbook valence/arousal of each label, as in expression.js (the baseline the fit must beat)
VA = {"angry": (-0.6, 0.7), "disgust": (-0.7, 0.3), "fearful": (-0.6, 0.8), "happy": (0.8, 0.5),
      "neutral": (0, 0), "sad": (-0.7, -0.4), "surprised": (0.3, 0.8)}
CALIBRATION = F.DATA / "expr_calibration.json"
VA_FIT = F.DATA / "va_fit.json"
MIN_CAL_FRAMES, MIN_FIT_PERSON, MIN_FIT_SHARED = 100, 15, 20


def talk_threshold():
    """TALK_JAW_SD from signals.js, so this measures what the page actually uses."""
    m = re.search(r"TALK_JAW_SD\s*=\s*([\d.]+)", (F.HERE / "signals.js").read_text(encoding="utf-8"))
    return float(m.group(1)) if m else None


def load(sessions):
    folder = F.DATA / "expr_eval"
    files = sorted(folder.glob("*.jsonl")) if folder.exists() else []
    posed, reports, used = [], [], []
    for f in files:
        is_feel = f.stem.startswith("feel-")
        if sessions and not is_feel and f.stem not in sessions:
            continue
        used.append(f)
        for line in f.read_text(encoding="utf-8").splitlines():
            if line.strip():
                r = json.loads(line)
                r["session"] = f.stem
                (reports if r.get("kind") == "report" else posed).append(r)
    return used, posed, reports


def weight(r):
    return 0.5 if r.get("quality") is None else max(0.05, r["quality"])


def voted(segment):
    """The tracker's vote after each frame of one posed segment: (probabilities, label, clear?)."""
    out = []
    for i in range(len(segment)):
        win = segment[max(0, i + 1 - X_WINDOW):i + 1]
        w = np.array([weight(r) for r in win])
        p = (np.array([r["expr"] for r in win]) * w[:, None]).sum(0) / w.sum()
        top, second = np.argsort(p)[::-1][:2]
        clear = len(win) >= X_MIN_VOTES and p[top] >= X_SURE and p[top] - p[second] >= X_MARGIN
        out.append((p, F.EXPRESSIONS[top], clear))
    return out


def state_model(segment):
    """The state model's belief after each frame (as filterStep in expression.js): (label, clear?)."""
    out, b = [], None
    for i, r in enumerate(segment):
        probs = np.maximum(np.array(r["expr"], np.float64), 1e-6)
        n = len(probs)
        w = 0.5 if r.get("quality") is None else min(1.0, max(0.1, r["quality"] / H_FULL_Q))
        prior = b if b is not None else np.full(n, 1 / n)
        b = (H_STAY * prior + (1 - H_STAY) / n) * probs ** w
        b /= b.sum()
        top, second = np.argsort(b)[::-1][:2]
        clear = i + 1 >= X_MIN_VOTES and b[top] >= X_SURE and b[top] - b[second] >= X_MARGIN
        out.append((F.EXPRESSIONS[top], clear))
    return out


def kind(r):
    """Which kind of posed step a frame is from: "talk", "hand" (chin on hand) or "plain"."""
    return "talk" if r.get("talk") else "hand" if r.get("handAsked") else "plain"


def segments_of(rows):
    seg = defaultdict(list)                     # (session, posed, kind) -> frames in order
    for r in rows:
        seg[(r["session"], r["posed"], kind(r))].append(r)
    return seg


def pct(a, b):
    return f"{a * 100 / b:>5.0f}%" if b else "     -"


# ------------------------------------------------------------------------- 1. per expression
def accuracy_table(rows, seg):
    print(f"{'asked for':<10} {'frames':>6} {'frame top-1':>11} {'voted':>6} {'voted+clear':>11} "
          f"{'state':>6} {'state+clear':>11} {'p(asked)':>8}  mistaken for")
    for label in F.EXPRESSIONS:
        rs = [r for r in rows if r["posed"] == label]
        if not rs:
            print(f"{label:<10} {0:>6}")
            continue
        idx = F.EXPRESSIONS.index(label)
        tops = [F.EXPRESSIONS[int(np.argmax(r["expr"]))] for r in rs]
        votes = [v for (s, posed, k), sg in seg.items() if posed == label and k == "plain" for v in voted(sg)]
        states = [v for (s, posed, k), sg in seg.items() if posed == label and k == "plain" for v in state_model(sg)]
        wrong = Counter(t for t in tops if t != label)
        print(f"{label:<10} {len(rs):>6} {pct(tops.count(label), len(rs)):>11} "
              f"{pct(sum(v == label for _, v, _ in votes), len(votes))} "
              f"{pct(sum(v == label and c for _, v, c in votes), len(votes)):>11} "
              f"{pct(sum(v == label for v, _ in states), len(states))} "
              f"{pct(sum(v == label and c for v, c in states), len(states)):>11} "
              f"{np.mean([r['expr'][idx] for r in rs]):>8.2f}  "
              + ", ".join(f"{k} {c * 100 / len(rs):.0f}%" for k, c in wrong.most_common(3)))
    print("'state' = the state model (the page's 'Use state model' option). Prefer it only if it's at least as\n"
          "accurate as 'voted' here and gives clear readings as often.")


# ------------------------------------------------------------------------- 2. calibration
def soften(p, t):
    z = np.maximum(p, 1e-9) ** (1 / t)
    return z / z.sum(-1, keepdims=True)


def fit_temperature(ps, ys):
    best = (np.inf, 1.0)
    for t in np.exp(np.linspace(np.log(0.1), np.log(30), 200)):
        nll = -np.mean(np.log(soften(ps, t)[np.arange(len(ys)), ys] + 1e-12))
        best = min(best, (nll, round(float(t), 2)))
    return best[1]


def ece(ps, ys, bins=10):
    conf, pred = ps.max(1), ps.argmax(1)
    err = 0.0
    for lo in np.linspace(0, 1, bins, endpoint=False):
        m = (conf >= lo) & (conf < lo + 1 / bins)
        if m.any():
            err += m.mean() * abs(conf[m].mean() - (pred[m] == ys[m]).mean())
    return err


def calibration(seg):
    pairs = [(p, F.EXPRESSIONS.index(posed), sg[0]["name"])
             for (s, posed, k), sg in seg.items() if k == "plain" for p, _, _ in voted(sg)]
    if len(pairs) < MIN_CAL_FRAMES:
        print(f"Needs at least {MIN_CAL_FRAMES} test frames (have {len(pairs)}); run more Expression tests.")
        return
    ps, ys, names = np.array([p for p, _, _ in pairs]), np.array([y for _, y, _ in pairs]), [n for _, _, n in pairs]
    t = fit_temperature(ps, ys)
    cal = soften(ps, t)
    print(f"{len(pairs)} voted readings. Temperature {t} (1 = already calibrated; above 1 = the model is overconfident).")
    print(f"Calibration error (ECE): {ece(ps, ys) * 100:.1f} points before, {ece(cal, ys) * 100:.1f} after.")
    print(f"{'share':<10} {'readings':>8} {'right':>6} {'calibrated share':>17}")
    conf, right = ps.max(1), ps.argmax(1) == ys
    for lo, hi in [(0, .45), (.45, .6), (.6, .75), (.75, .9), (.9, 1.01)]:
        m = (conf >= lo) & (conf < hi)
        if m.any():
            print(f"{f'{lo:.2f}-{min(hi, 1):.2f}':<10} {m.sum():>8} {pct(right[m].sum(), m.sum())} {cal[m].max(1).mean() * 100:>16.0f}%")
    people = {}
    for name in sorted(set(names)):
        m = np.array([n == name for n in names])
        if name != "Unknown" and m.sum() >= MIN_CAL_FRAMES:
            people[name] = fit_temperature(ps[m], ys[m])
    if people:
        print("Per person: " + ", ".join(f"{n} {v}" for n, v in people.items()))
    CALIBRATION.write_text(json.dumps({"global": t, "people": people, "readings": len(pairs),
                                       "ece_before": round(ece(ps, ys), 4), "ece_after": round(ece(cal, ys), 4)}, indent=1))
    print(f"Wrote data\\{CALIBRATION.name} (the tracker shows 'cal NN%' in the evidence from now on).")


# ------------------------------------------------------------------------- 3. by condition
def by_condition(seg):
    rows = [(sg[i], v == posed and c) for (s, posed, k), sg in seg.items() if k == "plain"
            for i, (_, v, c) in enumerate(voted(sg))]
    if not rows:
        return
    def band(field, cuts, names):              # value -> its band's name (bands listed low to high)
        def key(r):
            v = r.get(field)
            return None if v is None else names[sum(v >= c for c in cuts)]
        return key, names
    groups = {
        "person": (lambda r: r["name"], None),
        "quality": band("quality", [.35, .5], ["<0.35", "0.35-0.5", ">=0.5"]),
        "face size": band("size", [150, 250], ["<150 px", "150-250 px", ">=250 px"]),
        "head turn": band("turn", [.1, .2], ["<0.1", "0.1-0.2", ">=0.2"]),
        "brightness": band("light", [.35, .55], ["dark <0.35", "0.35-0.55", "bright >=0.55"]),
    }
    print(f"{'condition':<12} {'group':<14} {'readings':>8} {'voted+clear right':>18}")
    for cond, (key, order) in groups.items():
        tally = defaultdict(lambda: [0, 0])
        for r, ok in rows:
            k = key(r)
            if k is not None:
                tally[k][0] += ok
                tally[k][1] += 1
        for k, (ok, n) in sorted(tally.items(), key=lambda kv: order.index(kv[0]) if order else kv[0]):
            print(f"{cond:<12} {str(k):<14} {n:>8} {pct(ok, n):>18}")
    print("Large gaps between groups of one condition point at where the model fails for a structural reason.")


# ------------------------------------------------------------------------- 4. talking
def talking(rows, seg):
    talk = [r for r in rows if r.get("talk")]
    if not talk:
        print("No talking frames yet: run the Expression test (it ends with two talking steps).")
        return
    quiet = [r for r in rows if kind(r) == "plain"]
    for label in sorted({r["posed"] for r in talk}):
        tv = [v for (s, posed, k), sg in seg.items() if posed == label and k == "talk" for v in voted(sg)]
        qv = [v for (s, posed, k), sg in seg.items() if posed == label and k == "plain" for v in voted(sg)]
        wrong = Counter(v for _, v, c in tv if c and v != label)
        print(f"Asked '{label}': voted+clear right {pct(sum(v == label and c for _, v, c in tv), len(tv)).strip()} while talking, "
              f"{pct(sum(v == label and c for _, v, c in qv), len(qv)).strip()} silent; while talking misread as "
              + (", ".join(f"{k} {n * 100 / len(tv):.0f}%" for k, n in wrong.most_common(3)) or "nothing")
              + f"; unclear {pct(sum(not c for _, _, c in tv), len(tv)).strip()}")
    t_s = np.array([r["tv"] for r in talk if r.get("tv") is not None])
    q_s = np.array([r["tv"] for r in quiet if r.get("tv") is not None])
    thr = talk_threshold()
    if len(t_s) and len(q_s):
        print(f"Jaw-movement score (signals.js talking): talking median {np.median(t_s):.3f} "
              f"(quartiles {np.percentile(t_s, 25):.3f}-{np.percentile(t_s, 75):.3f}), "
              f"silent median {np.median(q_s):.3f} (quartiles {np.percentile(q_s, 25):.3f}-{np.percentile(q_s, 75):.3f})")
        cands = np.unique(np.concatenate([t_s, q_s]))
        bal = [((t_s >= c).mean() + (q_s < c).mean()) / 2 for c in cands]
        best = cands[int(np.argmax(bal))]
        if thr is not None:
            print(f"At TALK_JAW_SD {thr}: catches {pct((t_s >= thr).sum(), len(t_s)).strip()} of talking frames, "
                  f"wrongly flags {pct((q_s >= thr).sum(), len(q_s)).strip()} of silent ones.")
        print(f"Best threshold on this data: {best:.3f} (catches {pct((t_s >= best).sum(), len(t_s)).strip()}, "
              f"wrongly flags {pct((q_s >= best).sum(), len(q_s)).strip()}). Change TALK_JAW_SD in signals.js only with "
              f"enough sessions behind it.")
    else:
        print("No jaw-movement scores recorded (face-shape cues need MediaPipe).")
    tv_ = [r["vad"] for r in talk if "vad" in r]
    qv_ = [r["vad"] for r in quiet if "vad" in r]
    if tv_ and qv_:
        print(f"Microphone voice activity: on in {pct(sum(tv_), len(tv_)).strip()} of talking frames, "
              f"{pct(sum(qv_), len(qv_)).strip()} of silent ones.")


# ------------------------------------------------------------------------- 5. body
def posture(pts):
    """Head height and neck (ears) above the shoulder line, in shoulder widths, as in signals.js."""
    if not pts or pts[3][2] < F.POSE_VIS or pts[4][2] < F.POSE_VIS:
        return None
    sw = float(np.hypot(pts[4][0] - pts[3][0], pts[4][1] - pts[3][1]))
    if sw <= 10:
        return None
    mid = (pts[3][1] + pts[4][1]) / 2
    ears = [pts[i][1] for i in (1, 2) if pts[i][2] >= F.POSE_VIS]
    return {"head": (mid - pts[0][1]) / sw if pts[0][2] >= F.POSE_VIS else None,
            "neck": (mid - np.mean(ears)) / sw if ears else None}


def body(rows, seg):
    with_pose = [r for r in rows if r.get("pose")]
    if not with_pose:
        print("No body points recorded (the pose model wasn't installed, or no shoulders were in view).")
        return
    print(f"{len(with_pose)} test frames with body points ({pct(len(with_pose), len(rows)).strip()} of all).")
    hand = [r for r in rows if kind(r) == "hand"]
    if hand:
        on = [r["hand"] for r in hand if r.get("hand") is not None]
        off = [r["hand"] for r in rows if kind(r) == "plain" and r.get("hand") is not None]
        print(f"Hand-on-face detector: on in {pct(sum(on), len(on)).strip()} of 'chin on hand' frames, "
              f"{pct(sum(off), len(off)).strip()} of the other silent frames.")
        hv = [v for (s, posed, k), sg in seg.items() if k == "hand" for v in voted(sg)]
        nv = [v for (s, posed, k), sg in seg.items() if posed == "neutral" and k == "plain" for v in voted(sg)]
        wrong = Counter(v for _, v, c in hv if c and v != "neutral")
        print(f"'Neutral' voted+clear right {pct(sum(v == 'neutral' and c for _, v, c in hv), len(hv)).strip()} with a hand "
              f"on the chin, {pct(sum(v == 'neutral' and c for _, v, c in nv), len(nv)).strip()} without; with the hand misread as "
              + (", ".join(f"{k} {n * 100 / len(hv):.0f}%" for k, n in wrong.most_common(3)) or "nothing")
              + f"; unclear {pct(sum(not c for _, _, c in hv), len(hv)).strip()}")
        print("If the hand costs accuracy and the detector catches it, the 'skip when a hand is on the face' option is worth turning on.")
    else:
        print("No 'chin on hand' step yet: run the Expression test (it now ends with one).")
    print("\nPosture by expression asked for (shoulder widths; head = nose above the shoulders, neck = ears above them):")
    for label in F.EXPRESSIONS:
        qs = [q for r in with_pose if r["posed"] == label and kind(r) == "plain" and (q := posture(r["pose"]))]
        heads = [q["head"] for q in qs if q["head"] is not None]
        necks = [q["neck"] for q in qs if q["neck"] is not None]
        if qs:
            print(f"  {label:<10} {len(qs):>4} frames  head {np.mean(heads) if heads else float('nan'):.2f}  "
                  f"neck {np.mean(necks) if necks else float('nan'):.2f}")


# ------------------------------------------------------------------------- 6. cues
def cue_table(rows):
    with_cues = [r for r in rows if r.get("blendshapes") and kind(r) == "plain"]
    if not with_cues:
        print("No face-shape cues recorded (MediaPipe wasn't installed during the test).")
        return
    names = list(F.face_cues({}).keys())
    print(f"Average face-shape cues by expression asked for ({len(with_cues)} frames; 0 = none, 1 = strongest):")
    print(f"{'':<10} " + " ".join(f"{c[:8]:>8}" for c in names))
    for label in F.EXPRESSIONS:
        cs = [F.face_cues(r["blendshapes"]) for r in with_cues if r["posed"] == label]
        if cs:
            print(f"{label:<10} " + " ".join(f"{np.mean([c[k] for c in cs]):>8.2f}" for k in names))
    print("Cues that rise clearly above the 'neutral' row for one expression are the ones worth fusing.")


# ------------------------------------------------------------------------- 7. feeling reports
def design(reports, feats):
    X = np.array([[np.nan if r["features"].get(k) is None else float(r["features"][k]) for k in feats] for r in reports])
    y = np.array([[(r["valence"] - 4) / 3, (r["arousal"] - 4) / 3] for r in reports])
    return X, y


def ridge(X, y, lam):
    """Standardized ridge regression; missing values count as the column mean. Returns a model dict."""
    mean = np.nanmean(X, 0)
    mean = np.where(np.isnan(mean), 0, mean)
    Xf = np.where(np.isnan(X), mean, X)
    scale = Xf.std(0)
    scale = np.where(scale > 1e-9, scale, 1.0)
    Z = (Xf - mean) / scale
    w = np.linalg.solve(Z.T @ Z + lam * np.eye(Z.shape[1]), Z.T @ (y - y.mean(0)))
    return {"mean": mean, "scale": scale, "w": w, "b": y.mean(0)}


def predict(m, X):
    Xf = np.where(np.isnan(X), m["mean"], X)
    return np.clip(((Xf - m["mean"]) / m["scale"]) @ m["w"] + m["b"], -1, 1)


def loo(X, y, lam):
    """Leave-one-out mean absolute error of ridge, and of always guessing the average."""
    err, base = [], []
    for i in range(len(y)):
        keep = np.arange(len(y)) != i
        m = ridge(X[keep], y[keep], lam)
        err.append(np.abs(predict(m, X[i:i + 1])[0] - y[i]))
        base.append(np.abs(y[keep].mean(0) - y[i]))
    return np.mean(err, 0), np.mean(base, 0)


def textbook(reports):
    out = []
    for r in reports:
        p = [r["features"].get("p_" + l) for l in F.EXPRESSIONS]
        out.append(None if any(v is None for v in p) else
                   [sum(pv * VA[l][k] for pv, l in zip(p, F.EXPRESSIONS)) for k in (0, 1)])
    return out


def feelings(reports):
    if not reports:
        print("No feeling reports yet: on the tracker page use 'Report how I feel' (or 'Ask me every ...').")
        return
    counts = Counter(r["name"] for r in reports)
    print(f"{len(reports)} reports: " + ", ".join(f"{n} {c}" for n, c in counts.most_common()))
    feats = sorted({k for r in reports for k, v in r["features"].items() if isinstance(v, (int, float))})
    feats = [k for k in feats if sum(r["features"].get(k) is not None for r in reports) >= len(reports) / 2]
    if not feats:
        print("The reports carry no signal numbers.")
        return
    X, y = design(reports, feats)
    print("\nSignals that go with what you reported (correlation, -1..1; |r| above about 0.3 with 20+ reports is worth a look):")
    for k, side in ((0, "pleasant"), (1, "energized")):
        cors = []
        for j, f in enumerate(feats):
            m = ~np.isnan(X[:, j])
            if m.sum() >= 8 and X[m, j].std() > 1e-9 and y[m, k].std() > 1e-9:
                cors.append((np.corrcoef(X[m, j], y[m, k])[0, 1], f))
        cors.sort(key=lambda c: -abs(c[0]))
        print(f"  {side:<10} " + (", ".join(f"{f} {c:+.2f}" for c, f in cors[:6]) or "not enough varied reports yet"))
    tb = textbook(reports)
    have = [i for i, v in enumerate(tb) if v is not None]
    if have:
        tb_err = np.mean([np.abs(np.array(tb[i]) - y[i]) for i in have], 0)
        print(f"\nTextbook valence/arousal from the expression shares: off by {tb_err[0]:.2f} / {tb_err[1]:.2f} on average (scale -1..1)")
    models = {}
    groups = [("*", list(range(len(reports))), MIN_FIT_SHARED)] + \
             [(n, [i for i, r in enumerate(reports) if r["name"] == n], MIN_FIT_PERSON) for n in counts if n != "Unknown"]
    print(f"{'fit for':<12} {'reports':>7} {'fit error V/A':>14} {'guess-average error V/A':>24}  kept")
    for name, idx, need in groups:
        if len(idx) < need:
            print(f"{name:<12} {len(idx):>7}  needs {need}")
            continue
        Xg, yg = X[idx], y[idx]
        lam = min((1.0, 3.0, 10.0, 30.0, 100.0), key=lambda l: loo(Xg, yg, l)[0].sum())
        err, base = loo(Xg, yg, lam)
        better = err < base
        m = ridge(Xg, yg, lam)
        for k in (0, 1):
            if not better[k]:                     # no better than guessing the average: keep just the average
                m["w"][:, k] = 0
        print(f"{name:<12} {len(idx):>7} {err[0]:>7.2f} / {err[1]:.2f} {base[0]:>13.2f} / {base[1]:.2f}        "
              + ("valence " if better[0] else "") + ("arousal" if better[1] else "") + ("" if better.any() else "no"))
        if better.any():
            models[name] = {"mean": m["mean"].round(5).tolist(), "scale": m["scale"].round(5).tolist(),
                            "v": {"w": m["w"][:, 0].round(5).tolist(), "b": round(float(m["b"][0]), 5)},
                            "a": {"w": m["w"][:, 1].round(5).tolist(), "b": round(float(m["b"][1]), 5)}}
    if models:
        VA_FIT.write_text(json.dumps({"features": feats, "models": models, "reports": len(reports)}, indent=1))
        print(f"Wrote data\\{VA_FIT.name}: the tracker's evidence now shows fitted valence/arousal ('fit').")
    elif VA_FIT.exists():
        VA_FIT.unlink()
        print(f"No fit beats guessing the average any more; removed data\\{VA_FIT.name}.")


def main():
    files, rows, reports = load(set(sys.argv[1:]))
    if not rows and not reports:
        sys.exit("No test data yet. On the tracker page open 'Expression test' and press Start test, "
                 "or use 'Report how I feel'.")
    print(f"{len(rows)} readable test frames and {len(reports)} feeling reports from {len(files)} file(s)\n")
    seg = segments_of(rows)
    silent = [r for r in rows if kind(r) == "plain"]
    sections = [("1. Expression asked for vs read", lambda: accuracy_table(silent, seg)),
                ("2. Confidence calibration", lambda: calibration(seg)),
                ("3. Accuracy by condition (voted readings)", lambda: by_condition(seg)),
                ("4. Talking", lambda: talking(rows, seg)),
                ("5. Body: hand on face and posture", lambda: body(rows, seg)),
                ("6. Face-shape cues", lambda: cue_table(rows)),
                ("7. Feeling reports", lambda: feelings(reports))]
    for title, run in sections:
        if not title.startswith("7") and not rows:
            continue
        print(f"== {title}")
        run()
        print()


if __name__ == "__main__":
    main()
