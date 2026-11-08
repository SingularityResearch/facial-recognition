# Facial-Recognition

Local face recognition for a USB webcam: learn specific people from a photo library, name/ignore faces
in a labeling tool, and show names + match confidence (+ optional expression) on faces live in the
browser. Everything runs on this machine; no data leaves it, except a face or photo the owner sends
on purpose with the labeling tool's **Web search** (face to Amazon, photo to Google; never automatic).

Goals, key decisions and research value: [`README.md`](README.md).
Full background, history, measured tuning values and gotchas: **see [`PROJECT_CONTEXT.md`](PROJECT_CONTEXT.md).**
User-facing docs: [`face-recognizer/README.md`](face-recognizer/README.md).
Why expression reading is layered (the science): [`docs/WHY_LAYERED_EXPRESSION.md`](docs/WHY_LAYERED_EXPRESSION.md).
Expression design review (LM Studio export): [`Real-Time Facial Emotion Recognition.md`](Real-Time%20Facial%20Emotion%20Recognition.md).

## Layout

- `docs/` — background write-ups (`WHY_LAYERED_EXPRESSION.md`).
- `face-tracker/index.html` — original standalone MediaPipe tracker; double-click to open.
- `face-recognizer/` — the Python project. All programs share `face-recognizer/data/`:
  - `face_id.py` — core library + CLI (`scan`, `cluster`, `name`, `people`, `recognize`, `reembed`, `migrate`).
  - `labels.py` — labeling engine: per-face labels (`LabelDB`), background photo/video `Importer`
    (videos: `face_id.video_faces`, ~1 frame/s, each person followed and their 3 clearest faces kept; frame numbers in
    `photos.json` `at`), `relocate` (finds photos that moved, e.g. a changed drive letter, at start),
    auto-naming, duplicate-photo detection, undo history, `PhotoFacts` (camera dates + people tags, saved in `data/photo_facts.json`)
    and clues for unnamed faces (`hints`: closest people, photo tags, same occasion).
  - `label_tool.py` + `label_ui.html` / `label.bat` — labeling web app on `http://127.0.0.1:8765`.
    Chris calls this "the recognizer".
  - `tracker.py` + `tracker_ui.html` / `recognize.bat` — live tracker on `http://127.0.0.1:8766`.
    Its **Collect faces** option saves webcam snapshots + the tracker's verdict to `%APPDATA%\FaceLabels\webcam\`
    (`face_id.WEBCAM`); `label_tool.py` imports them by itself and shows the verdict as a clue.
  - `expression.js` — expression voting, state model, confidence flags, evidence, per-person usual look (browser;
    served by `tracker.py`). Tests: `node --test tests` from `face-recognizer\`.
  - `signals.js` — motion, talking, blinks, gaze, smile timing, webcam pulse, body posture, microphone voice features,
    each person's normal for every signal (`Baselines`, alone and in company),
    feeling-report summaries, fitted valence/arousal (browser; served by `tracker.py`; tests in `tests/`).
  - `app.py` / `build_app.bat` → `FaceRecognizer.exe` — one window (pywebview, WebView2) with Face tracking and
    Labeling tabs; starts both servers in one process (`tracker.bind/attach`, `label_tool.attach`), each still on its
    own port and page. PyInstaller one-file build; uses `models\` and `data\` beside the .exe (`face_id.ROOT`).
  - `websearch.py` — the labeling tool's **Web search**: Amazon Rekognition celebrity recognition + Google Cloud Vision web detection, combined into suggestions with a confidence level (on click only; US services, English results, each capped under $10/month; keys in `%APPDATA%\FaceLabels`, never in the repo).
  - `localguard.py` — request checks for both local servers (Host, `X-Face-Tool: 1`, Origin).
  - `setup.bat` — installs Python 3.12 (winget) and `requirements.txt`.
  - `fetch_models.py` — downloads missing models (or Git LFS placeholders) from the GitHub Release `models-v1`
    with SHA-256 checks; run by `setup.bat` and by the .exe on first start. `face_id.check_downloaded()` stops
    with "Model files weren't downloaded: install Git LFS and run git lfs pull" when it finds placeholders.
  - `models/` (Git LFS) — YuNet (detect), ArcFace R50 (recognize), SFace (fallback), eDifFIQA (quality),
    expression model, AdaFace (evaluated, unused; local only, made by `tools/get_adaface.py`, not in git),
    `npu/` (static ArcFace + VitisAI config/cache), `face_landmarker.task` (MediaPipe blendshape cues, optional),
    `wav2small.onnx` (voice arousal/dominance/valence, optional; made by `tools/get_wav2small.py`),
    `pose_landmarker_lite.task` (MediaPipe body posture, optional).
  - `tools/` — `eval_models.py`, `hw_bench.py`, `expr_check.py`, `expr_eval.py`, `get_adaface.py`, `get_wav2small.py`, `import_bench.py`, `websearch_check.py`, `webcam_eval.py` (live naming on confirmed webcam snapshots; measured the
    averaged-print rule the tracker uses via `/api/who`).
  - `.github/workflows/checks.yml` — CI on every push/PR: `node --test tests` and pyflakes (no LFS).

## Running

```
face-recognizer\setup.bat        # once
face-recognizer\recognize.bat    # live tracker  (port 8766)
face-recognizer\label.bat        # labeling tool (port 8765)
face-recognizer\build_app.bat    # builds FaceRecognizer.exe: both apps in one window, two tabs
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
- Not yet measured, so evidence only: `expression.js` `H_STAY` (state model) and every `signals.js` constant
  (`TALK_JAW_SD`, pulse, voice). The options that let them change the label (skip while talking, state model)
  stay off by default until `tools/expr_eval.py` shows they help on posed sessions. Same for the hand-on-face skip.
- Labels live per face in `data/labels.json` (keyed by permanent face ID); change them through the
  labeling tool or `LabelDB`, not by hand-editing JSON. `cluster` group numbers are temporary.
- After switching recognition model: `python face_id.py reembed`.
- Name overlay belongs on the face near the top of the head (owner preference).
- The labeling tool stays a separate program from the tracker (owner preference): separate code, server and page.
  Since 2026-11-08 they can share one window as two tabs (`app.py`), but each still runs on its own.
- Camera opening on Windows: try `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`, retry `cap.read()` while the webcam warms up.
- Face-shape cues are evidence only until fusion is measured with `tools/expr_eval.py` on posed sessions.
- Expression is display-only and never stored (except opt-in posed test sessions and feeling reports in `data/expr_eval/`,
  numbers only); microphone audio is analysed live and never stored; per-person baselines and usual looks live in the
  browser's storage only (never `data/`, which is published); keep README wording honest ("looks", not "feels").
- The presence log (who was in view, when) is opt-in, names and times only, in `%APPDATA%\FaceLabels\presence\`, never `data/`.
- Webcam faces named automatically don't enter the recognition gallery until confirmed (`LabelDB.gallery`).
- InsightFace ArcFace and audEERING Wav2Small weights are non-commercial; note this if the project is ever redistributed.
