# Facial-Recognition

Local face recognition for a USB webcam: learn specific people from a photo library, name/ignore faces
in a labeling tool, and show names + match confidence (+ optional expression) on faces live in the
browser. Everything runs on this machine; no data leaves it.

Full background, history, measured tuning values and gotchas: **see `PROJECT_CONTEXT.md`.**
User-facing docs: `face-recognizer/README.md`.

## Layout

- `face-tracker/index.html` — original standalone MediaPipe tracker; double-click to open.
- `face-recognizer/` — the Python project. All programs share `face-recognizer/data/`:
  - `face_id.py` — core library + CLI (`scan`, `cluster`, `name`, `people`, `recognize`, `reembed`, `migrate`).
  - `labels.py` — labeling engine: per-face labels (`LabelDB`), background photo `Importer`,
    auto-naming, duplicate-photo detection, undo history.
  - `label_tool.py` + `label_ui.html` / `label.bat` — labeling web app on `http://127.0.0.1:8765`.
    Chris calls this "the recognizer".
  - `tracker.py` + `tracker_ui.html` / `recognize.bat` — live tracker on `http://127.0.0.1:8766`.
  - `expression.js` — expression voting, confidence flags, evidence, per-person usual look (browser;
    served by `tracker.py`). Tests: `node --test tests` from `face-recognizer\`.
  - `localguard.py` — request checks for both local servers (Host, `X-Face-Tool: 1`, Origin).
  - `setup.bat` — installs Python 3.12 (winget) and `requirements.txt`.
  - `models/` — YuNet (detect), ArcFace R50 (recognize; also shipped split as `.part0`–`.part8`, rejoined
    by `_join_parts()`), SFace (fallback), eDifFIQA (quality), expression model, AdaFace (evaluated, unused),
    `npu/` (static ArcFace + VitisAI config/cache), `face_landmarker.task` (MediaPipe blendshape cues, optional).
  - `tools/` — `eval_models.py`, `hw_bench.py`, `expr_check.py`, `expr_eval.py`, `get_adaface.py`, `import_bench.py`.

## Running

```
face-recognizer\setup.bat        # once
face-recognizer\recognize.bat    # live tracker  (port 8766)
face-recognizer\label.bat        # labeling tool (port 8765)
python face_id.py --help         # CLI pipeline (run from face-recognizer\)
```

Use the Python 3.12 the .bat files pick (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`). The
`python` on PATH (3.15) does not have OpenCV/onnxruntime installed.

## Conventions and constraints

- Keep `face-recognizer/README.md` in sync with any user-visible change.
- Thresholds are **measured values** — don't change them without re-measuring on `data/`:
  `face_id.py` (`REC_MODELS[...]["match"/"cluster"]`, `Q_ENROLL`, `Q_EXPR`, `MAX_TURN`),
  `labels.py` (`AUTO`, `MARGIN`, `TOPK`, `DUP_BITS`), `tracker_ui.html` (strictness default 0.35),
  and `expression.js` (`X_SURE`, `X_MARGIN`, `X_MIN_VOTES`, vote windows).
- Labels live per face in `data/labels.json` (keyed by permanent face ID); change them through the
  labeling tool or `LabelDB`, not by hand-editing JSON. `cluster` group numbers are temporary.
- After switching recognition model: `python face_id.py reembed`.
- Name overlay belongs on the face near the top of the head (owner preference).
- The labeling tool stays a separate program from the tracker (owner preference).
- Camera opening on Windows: try `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`, retry `cap.read()` while the webcam warms up.
- Face-shape cues are evidence only until fusion is measured with `tools/expr_eval.py` on posed sessions.
- Expression is display-only and never stored (except opt-in posed test sessions in `data/expr_eval/`, numbers only); keep README wording honest ("looks", not "feels").
- InsightFace ArcFace weights are non-commercial; note this if the project is ever redistributed.
