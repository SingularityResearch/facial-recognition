r"""
expr_eval.py - how well expression reading works on your own face, from posed test sessions.

Record a session first: tracker page -> Expression test -> Start test (you're asked to make each
expression for a few seconds; only numbers are saved, in data\expr_eval\).

  python tools\expr_eval.py                 (all saved sessions)
  python tools\expr_eval.py 20261030-213000 (one session)

Prints, for each expression you were asked to make:
  - how often the model's top label was that expression, frame by frame and after the tracker's
    vote over 10 frames (same weighting and X_SURE / X_MARGIN gates as expression.js)
  - which labels it was mistaken for
  - the average face-shape cues, to see which cues separate the expressions on your face
Use it before changing any expression threshold, and to measure whether cues would help.
Posed expressions are exaggerated compared with everyday ones, so treat results as an upper bound.
"""
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import face_id as F                     # noqa: E402

X_SURE, X_MARGIN, X_MIN_VOTES, X_WINDOW = 0.45, 0.12, 3, 10   # as in expression.js


def load(sessions):
    folder = F.DATA / "expr_eval"
    files = sorted(folder.glob("*.jsonl")) if folder.exists() else []
    if sessions:
        files = [f for f in files if f.stem in sessions]
    rows = []
    for f in files:
        for line in f.read_text(encoding="utf-8").splitlines():
            if line.strip():
                r = json.loads(line)
                r["session"] = f.stem
                rows.append(r)
    return files, rows


def voted(segment):
    """The tracker's reading after each frame of one posed segment: (label or None, clear?)."""
    out = []
    for i in range(len(segment)):
        win = segment[max(0, i + 1 - X_WINDOW):i + 1]
        w = np.array([0.5 if r["quality"] is None else max(0.05, r["quality"]) for r in win])
        p = (np.array([r["expr"] for r in win]) * w[:, None]).sum(0) / w.sum()
        top, second = np.argsort(p)[::-1][:2]
        clear = len(win) >= X_MIN_VOTES and p[top] >= X_SURE and p[top] - p[second] >= X_MARGIN
        out.append((F.EXPRESSIONS[top], clear))
    return out


def main():
    files, rows = load(set(sys.argv[1:]))
    if not rows:
        sys.exit("No test sessions yet. On the tracker page open 'Expression test' and press Start test.")
    print(f"{len(rows)} readable frames from {len(files)} session(s): {', '.join(f.stem for f in files)}\n")

    segments = defaultdict(list)                 # (session, posed) -> frames in order
    for r in rows:
        segments[(r["session"], r["posed"])].append(r)

    print(f"{'asked for':<10} {'frames':>6} {'frame top-1':>11} {'voted':>6} {'voted+clear':>11} {'p(asked)':>8}  mistaken for")
    for label in F.EXPRESSIONS:
        rs = [r for r in rows if r["posed"] == label]
        if not rs:
            print(f"{label:<10} {0:>6}")
            continue
        idx = F.EXPRESSIONS.index(label)
        tops = [F.EXPRESSIONS[int(np.argmax(r["expr"]))] for r in rs]
        votes = [v for (s, posed), seg in segments.items() if posed == label for v in voted(seg)]
        wrong = Counter(t for t in tops if t != label)
        print(f"{label:<10} {len(rs):>6} {tops.count(label) * 100 / len(rs):>10.0f}% "
              f"{sum(v == label for v, _ in votes) * 100 / len(votes):>5.0f}% "
              f"{sum(v == label and c for v, c in votes) * 100 / len(votes):>10.0f}% "
              f"{np.mean([r['expr'][idx] for r in rs]):>8.2f}  "
              + ", ".join(f"{k} {c * 100 / len(rs):.0f}%" for k, c in wrong.most_common(3)))

    with_cues = [r for r in rows if r.get("blendshapes")]
    if not with_cues:
        print("\nNo face-shape cues recorded (MediaPipe wasn't installed during the test).")
        return
    print(f"\nAverage face-shape cues by expression asked for ({len(with_cues)} frames; 0 = none, 1 = strongest):")
    print(f"{'':<10} " + " ".join(f"{c[:8]:>8}" for c in F.CUES))
    for label in F.EXPRESSIONS:
        cs = [F.face_cues(r["blendshapes"]) for r in with_cues if r["posed"] == label]
        if cs:
            print(f"{label:<10} " + " ".join(f"{np.mean([c[k] for c in cs]):>8.2f}" for k in F.CUES))
    print("\nCues that rise clearly above the 'neutral' row for one expression are the ones worth fusing.")


if __name__ == "__main__":
    main()
