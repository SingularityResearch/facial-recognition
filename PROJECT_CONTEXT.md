# Facial-Recognition — Project Context Export

Exported 2026-10-29 from the Claude Project **"Facial-Recognition"**
(*Detect known faces and provide for the management and labeling of new or unknown faces.*).
Revised the same day to match the code and data as they now stand, and kept up to date since
(last: 2026-11-10, v1.9 released for Windows, macOS and Linux: review fixes, one person's export and forget, the
alert inbox; see §2 "macOS and Linux", §7 "Releases" and §10).

This file collects everything established across the Project's chats so a coding assistant or a human
opening the repo in VS Code has the full background: what was built, why, where it lives, the measured
tuning values, the owner's stated preferences, and the environment gotchas that cost time before.

**Related docs:** [README](README.md) (goals, decisions, research value) · [AGENTS.md](AGENTS.md) (layout, conventions, security rules, checks; [CLAUDE.md](CLAUDE.md) reads it) ·
[Face Recognizer README](face-recognizer/README.md) (user docs) ·
[Face Tracker README](face-tracker/README.md) ·
[Why expression reading is layered](docs/WHY_LAYERED_EXPRESSION.md) (the science) ·
[Real-Time Facial Emotion Recognition](Real-Time%20Facial%20Emotion%20Recognition.md) (design review, see §
"Expression refactor").

---

## 1. What this is

A fully local face-recognition system for a USB webcam, built for Chris:

1. **Face tracker** — the original standalone browser page (MediaPipe face detector).
2. **Face recognizer** — a Python pipeline that learned specific people from Chris's personal photo
   library and names them live on the webcam.
3. **Labeling tool** — a local browser app for adding photos, confirming suggested names, naming new
   people, ignoring false positives and fixing mistakes.
4. **Live tracker** — a local browser app that overlays each person's name, match confidence, and
   (optionally) a facial-expression reading directly on their face.

Nothing leaves the machine. No photos or face data are sent anywhere, except one face or photo when the
owner clicks **Web search** in the labeling tool: the face goes to Amazon Rekognition and the photo to Google Cloud Vision (never automatic),
and photos to be described when **Describe photo** is pointed at a model server on another device (Settings tab, since v1.8).

Since 2026-11-08 the tracker and the labeling tool also run together as **FaceRecognizer.exe** (`app.py`): one window
with Face tracking, Labeling and (since v1.8) Settings tabs, each program still on its own server and page.

---

## 2. Where it lives (Windows; macOS and Linux below)

```
X:\source\facial-recognition\
├── AGENTS.md (the guide), CLAUDE.md (reads AGENTS.md), PROJECT_CONTEXT.md, README.md
├── .github\workflows\checks.yml   CI: node tests, ruff, pytest on Windows, macOS, Linux; then builds for all three
├── Real-Time Facial Emotion Recognition.md   expression design review (LM Studio export)
├── docs\
│   └── WHY_LAYERED_EXPRESSION.md            why expression reading is layered, with references
├── face-tracker\                 original standalone HTML tracker (MediaPipe), double-click index.html
│   └── index.html
└── face-recognizer\              the Python project (everything below shares one data\ folder)
    ├── face_id.py                core library + CLI: scan / cluster / name / people / recognize / reembed / migrate
    ├── paths.py                  ROOT/HOME (FACE_ROOT/FACE_HOME overrides), atomic_write
    ├── labels.py                 labeling engine: LabelDB (per-face labels, undo); re-exports the four below
    ├── photofiles.py, photofacts.py, relocate.py, importer.py   hashes/cloud files; dates/tags; upgrade/moved
    │                             photos; background import (split out of labels.py 2026-11-09)
    ├── label_tool.py             labeling web app  → http://127.0.0.1:8765   (launch: label.bat, label.sh)
    ├── label_ui.html, label_ui.js   labeling page and its script (served by label_tool.py)
    ├── tracker.py                live tracker server → http://127.0.0.1:8766 (launch: recognize.bat, recognize.sh)
    ├── tracker_ui.html, tracker_ui.js   live tracker page and its script (served by tracker.py)
    ├── expression.js             expression vote, state model, usual looks, evidence (browser; tests\)
    ├── signals.js                motion, talking, blinks, gaze, timing, pulse, voice, VA fit (browser; tests\)
    ├── settings_bridge.js        the app's Settings tab <-> each page's settings (postMessage; served by both servers)
    ├── app.py, app_ui.html, app_ui.js   FaceRecognizer.exe: one window, three tabs (Face tracking, Labeling,
    │                             Settings); the tab page on 8764
    ├── build_app.bat             Windows: runs build_exe.py → FaceRecognizer.exe
    ├── build_exe.py              the app build for the system it runs on (.exe, .app, Linux folder); CI runs it too
    ├── desktop.py                open a folder, draw the eye to the window (Windows, macOS, Linux)
    ├── alerts.py                 the tracker's alert inbox (%APPDATA%\FaceLabels\alerts.json)
    ├── setup.sh, start.sh, FaceRecognizer.command   macOS and Linux: .venv setup, start the app
    ├── recognize.sh, label.sh    macOS and Linux: start the tracker or the labeling tool on its own
    ├── packaging\macos\          entitlements.plist, sign_and_notarize.sh (CI, with the Apple secrets)
    ├── websearch.py              Web search: Amazon Rekognition + Google Cloud Vision (on click; own keys in %APPDATA%)
    ├── describe.py               Describe photo: LM Studio or any OpenAI-style server (Settings), else florence.py
    ├── florence.py               built-in Florence-2 large describer (ONNX int8, CPU; 0.8 GB on request)
    ├── fetch_models.py           first-start model download from the publishers, models-v1 as fallback; SHA-256
    ├── localguard.py             request checks, page headers (CSP, framing, CORP) and body limits shared by the servers
    ├── persondata.py             one person's data: export (folder) and forget (every place it lives)
    ├── tests\                    Node tests (*.test.js) and Python tests (test_*.py, pytest); ruff.toml
    ├── setup.bat                 installs Python 3.12 via winget, then pip installs requirements.txt
    ├── recognize.bat             starts tracker.py (checks cv2/numpy are installed first)
    ├── label.bat                 starts label_tool.py
    ├── requirements.txt          opencv-python, numpy, scikit-learn, pillow, pillow-heif, onnxruntime-directml, mediapipe,
    │                             boto3 (mostly unpinned; pywebview and pyinstaller come from build_app.bat)
    ├── README.md                 user-facing docs; kept in sync with code changes
    ├── tools\
    │   ├── eval_models.py        ArcFace vs AdaFace vs both, on clean/blurred/tiny/motion faces → data\model_eval.json
    │   ├── hw_bench.py           CPU vs GPU vs NPU timing; compiles the NPU model → data\hw_bench.json
    │   ├── expr_check.py         sanity check for the expression model
    │   ├── expr_eval.py          posed tests + feeling reports → accuracy, calibration, talking, VA fit
    │   ├── get_adaface.py        downloads the official AdaFace IR-101 weights and converts them to ONNX
    │   ├── get_wav2small.py      downloads audEERING Wav2Small weights and converts them to ONNX
    │   ├── import_bench.py       photo-import timing
    │   ├── websearch_check.py    the Web search on picture files
    │   └── webcam_eval.py        live naming on confirmed webcam snapshots (/api/who)
    ├── models\
    │   ├── face_detection_yunet_2023mar.onnx           YuNet detector (OpenCV Zoo)
    │   ├── arcface_w600k_r50.onnx                      ArcFace recognizer (174 MB)
    │   ├── face_recognition_sface_2021dec.onnx         SFace, fallback recognizer (38.7 MB)
    │   ├── ediffiqa_tiny_jun2024.onnx                  eDifFIQA face quality (optional)
    │   ├── facial_expression_recognition_mobilefacenet_2022july.onnx   7-class expression model (optional)
    │   ├── adaface_ir101_webface12m.onnx, adaface_src\  AdaFace, evaluated and not used
    │   ├── face_landmarker.task                        MediaPipe blendshapes (optional)
    │   ├── wav2small.onnx, wav2small_src\              voice arousal/dominance/valence, 69 KB (optional; CC BY-NC-SA)
    │   ├── pose_landmarker_lite.task                   MediaPipe body posture, 5.8 MB (optional; Apache 2.0)
    │   ├── florence2\                                  built-in describer, downloaded on request (not in git)
    │   └── npu\                                        static batch-1 ArcFace, vitisai_config.json, compile cache
    └── data\                     produced by the tools; shared by all programs. LOCAL ONLY since 2026-11-09:
                                  the whole folder is in .gitignore, purged from git history and releases; a
                                  new copy starts with it empty (created on first start)
        ├── faces.npz             every face found: permanent ID (fid), photo, box, face print, quality. Ignored
        │                         faces (since v1.9) keep ID, photo and box only: print zeroed, no picture
        ├── thumbs.npz            each face's picture (112x112 aligned crop, JPEG) by face ID. Until 2026-11-09
        │                         inside faces.npz; moved on the next save.
        │                         Missing: label_tool shows a placeholder or remakes it from the original photo
        │                         (face_id.rebuild_crop: detect again, box overlap >= 0.6; on 200 faces 70%
        │                         remade, median print similarity to the saved crop 0.93)
        ├── photos.json           every photo read, keyed by content hash: path, size, mtime, picture fingerprint,
        │                         face count, and which photo a skipped copy duplicates
        ├── labels.json           label of each face, keyed by face ID (see §3)
        ├── history.jsonl         every labeling change with its time (powers Undo)
        ├── backups\              labels-<time>.json from each labeling-tool start (last 20);
        │                         legacy-<time>\ = the old groups.json/names.json/scanned.json
        │                         (local only since 2026-11-08: in .gitignore, untracked; tracked Oct 31-Nov 8)
        ├── people.npz            face prints of named people — what the tracker and `recognize` use
        ├── settings.json         photo folders that have been added
        ├── sheets\               contact sheets written by `cluster` (group_###.jpg, overview_*.jpg): local only
        ├── photo_facts.json, web_suggestions.json   camera dates/people tags; saved Web search answers
        ├── expr_eval\            opt-in posed test sessions and feeling reports (numbers only)
        └── model_eval.json, hw_bench.json   results from the tools above
```

Outside the project, `%APPDATA%\FaceLabels\` holds what is private to this computer's user: Web search keys and
monthly counts, `describe.json` (describer settings, optional API key), `descriptions.json`, `webcam\` (Collect faces:
each face alone with some room around it, others greyed out; whole frames until v1.9),
`presence\` (opt-in log), `alerts.json` (the alert inbox), `webview\` (the .exe window's browser storage: page options,
baselines, usual looks), `app.log`.

### macOS and Linux (since 2026-11-10, v1.9)

The same code runs on macOS 14+ on Apple Silicon (the onnxruntime and MediaPipe wheels have no Intel Mac builds)
and on Linux (built on Ubuntu 22.04, tested on Ubuntu 26.04 on a real laptop and 24.04 in WSL). From the code:
`./setup.sh`, `./start.sh`. Built apps keep models and `data` in `~/Library/Application Support/FaceRecognizer` or
`~/.local/share/FaceRecognizer` (the app itself is read-only) and the private folder in `.../FaceLabels` beside it.
Recognition runs on the CPU (OpenCV) there, as measured; CUDA (`setup.sh` installs `onnxruntime-gpu[cuda,cudnn]`
when `nvidia-smi` works) and CoreML are used per model only after the start-up check (`face_id._gpu_check`: same
results within 1% on a test input, and faster); `tools/hw_bench.py` measures them on real faces. The Linux window is Qt's web view: without working OpenGL it can't draw, so the app tries it in a child
process first (`--probe-window`) and otherwise opens the browser; the Linux build leaves the graphics libraries to
the system (bundled ones broke OpenGL on Ubuntu 24.04). Build steps for all three: face-recognizer/README.md,
"Building the app".

`face-recognizer\data_backup_20261029\` is a full copy of `data\` from before the per-face migration
(old format, 3.8 MB faces.npz).

Earlier in the project the two halves lived at `X:\source\face-tracker` and `X:\source\face-recognizer`;
they were later consolidated under `X:\source\facial-recognition\` as the two subfolders above.

**Vocabulary Chris uses** (recorded so the right thing gets launched):
- "the recognizer" = the **labeling tool** (`label.bat`, port 8765), *not* `recognize.bat`.
- "the tracker" = the live tracker (`recognize.bat` → `tracker.py`, port 8766).

Hardware: a simple USB webcam; AMD Ryzen AI Max+ 395 PC (16-core CPU, Radeon 8060S GPU, XDNA 2 NPU).
Photo source for training: the owner's iCloud Photos folder (local path not recorded here).

---

## 3. Data model and pipeline

### Per-face labels (labels.py)

Every face has a **permanent ID**: its photo's content hash + where the face is in the photo
(`face_key()`, e.g. `6edd140b4d6dc4a9-94-848`). Labels are stored on the face, so regrouping, rescanning,
or moving/renaming photos never loses them:

```
{"<face id>": {"n": "Chris", "by": "me"}, ...}
  n    the person's name, or "#ignore" (stranger, artwork, not a face: never recognized)
  by   "me" (named by Chris) or "auto" (named automatically because the model was very sure)
  not  names this face is NOT, so they are never suggested again
  ok   a "might be mislabeled" warning was checked and the name kept
  s    the model's score when the face was named automatically
```

Groups are no longer stored. For unnamed faces the engine works out on the fly:
- **suggested** — the model thinks it knows who it is (score ≥ `MATCH`): Chris confirms or rejects.
- **unknown** — nobody known is close: shown in groups of look-alikes to name or ignore. Groups that come
  within `HINT` (`MATCH − 0.12`) of a known person get a "looks like …" hint.

Every change is logged to `history.jsonl` and can be undone. `people.npz` is rebuilt from the labels.

### Labeling tool (label_tool.py + label_ui.html)

- **Add photos** runs the `Importer` in the background on the GPU, subfolders included. It skips photos
  already read (by content hash, so renamed/moved photos are recognized) and copies of photos (re-saved or
  resized; picture fingerprints within `DUP_BITS` = 12 bits **and** matching faces). New faces that clearly
  match a named person are **named automatically**.
- Views: **Suggestions** (per person, least certain first), **New faces** (look-alike groups, then
  **Seen once**), **From photo tags**,
  **Named automatically** (review, least certain first), **Checks** (named faces that look more like someone
  else, people with fewer than `FEW` = 5 clear faces, skipped duplicates), **People** (all faces of a person,
  least typical first; **Rename or merge**).
- Double-click / Space shows the whole photo with everyone boxed and named. Keys: A accept, N name,
  R not this person, X ignore, Z undo, Esc clear selection, Shift-click range.
- Faces too blurry to learn from are hidden unless **Show blurry faces** is on.
- The name picker and toast sit above the photo viewer (z-index 60/61 vs 50). Before 2026-11-01 the picker
  opened hidden behind the viewer, so naming a face from the photo view seemed to do nothing.
- **Clues for unnamed faces** (photo view, `LabelDB.hints`): closest 3 people with scores (`CLOSEST`, display
  only, no threshold), the photo's own people tags, and the **same occasion**: named people in photos taken
  within `TAKEN_NEAR_H` = 12 h (EXIF camera date), else the `FOLDER_NEAR` = 6 photos either side in its folder
  by file name. These constants only choose what is shown; they never label anything.
- **From photo tags** (`PhotoFacts`, `LabelDB.tagged`): at start and after each import a background pass
  (8 threads, ~1,100 photos in a few seconds) reads each photo-with-faces' camera date and people tags: MWG
  face regions (Lightroom/Picasa/digiKam; iPhones write unnamed ones), Microsoft regions (Windows Photo
  Gallery), IPTC PersonInImage, `.xmp` sidecars, and Google Takeout `.json` (`people`). Rectangles are matched
  to found faces by overlap (`match_regions`); names without a box apply when exactly one face is still
  unnamed. Measured on 190 iPhone photos with face regions: MWG rectangles apply to the photo **as stored**
  (rotated photos: 71 of 76 paired vs 32 the other way). Saved in `data/photo_facts.json` with a key per photo (file size + date, sidecar files, face boxes); a start re-reads only photos whose key changed (all 1,127: 1.9 s; none changed: 0.4 s). On 2026-11-01 only one photo
  in `data/` carried a named tag.
- **Web search** (photo view, one click, `websearch.suggest`): POST `/api/websearch {face}`;
  `label_tool.search_picture` makes the face JPEG (50% margin, enlarged to ≥ 400 px) for **Amazon
  Rekognition** `RecognizeCelebrities` and the whole-photo JPEG for **Google Cloud Vision** web detection.
  Owner's rule (2026-11-01): US services only, English results only, nothing from non-US sites. Combined
  into suggestions: Amazon match ≥ 95% high, ≥ 85% medium, else low (display bands, not measured here);
  Google names only count when the *exact* picture is on pages (person-like names, medium); both naming
  the same person → very high. Otherwise "No suggestions found". Tested: a phone photo of a TV still →
  Javier Bardem 99.4%; a Bernie Sanders DJ meme panel → Bernie Sanders 100%. Google alone gave
  nothing useful for either ("no expression", "rock concert", "David Penn" at relevance weight 10.72 for an
  unrelated DJ): Google does not identify faces, its "partial" pages are usually just similar scenes
  (9–12, none with the photo), and its entity scores are unbounded weights, so neither is used.
  Setup 2026-11-01 (the owner's accounts; details kept out of the repo). Amazon: an IAM user with an inline
  policy allowing `rekognition:RecognizeCelebrities` only, key in `%APPDATA%\FaceLabels\aws_rekognition.json`
  (also enterable in the dialog since 2026-11-09), budget alerts at $10/month. Google: a project with the
  Vision API, key restricted to `vision.googleapis.com`, budget $10/month;
  key in `%APPDATA%\FaceLabels\google_vision_key.txt` or `GOOGLE_VISION_API_KEY`. Budgets only warn, so
  `CAPS` stops each service in the tool: 9,000 Amazon ($9) and 3,700 Google ($9.45) per calendar month,
  counted in `%APPDATA%\FaceLabels\<service>_usage.json` before each call. Keys are never in the repo (public).
  Answers are saved per face ID in `data/web_suggestions.json` (`label_tool.WEB_SAVED`; complete answers only)
  and served again with no new calls (a repeat took 4 ms instead of 1.4 s); **Search again** sends `refresh`.
  `tools/websearch_check.py` runs the same search on files. Google Lens is still offered under
  Settings as copy + open `images.google.com` + paste: a form POST to `lens.google.com/v3/upload` gets 403
  even from a normal Edge window, and a server-side upload's results link is tied to that upload's `NID`
  cookie, so it opened without the picture in the owner's browser. Tried and dropped: Yandex (named Bernie
  Sanders, but non-US and mixed-language), Bing (only describes), TinEye (robot check). This is the only
  path by which anything leaves the machine.
- API: GET `/api/summary`, `/api/fids`, `/api/suggested`, `/api/unknown`, `/api/tagged`, `/api/auto`, `/api/person`,
  `/api/checks`, `/api/import`, `/api/photoinfo/…` (includes `hints`); POST `/api/act`, `/api/rename`, `/api/undo`,
  `/api/import`, `/api/import/stop`, `/api/autoname`.

### CLI (face_id.py)

```
scan       find every face in a photo folder (only new photos are read)  — same as Add photos
cluster    group unnamed look-alike faces, write contact sheets to data\sheets (--min 4)
name       name one or more cluster groups, or --ignore them
recognize  name people live on a webcam, or in photos / videos on disk
reembed    recompute face prints from the stored thumbnails after switching model (no photo rescan; a face
           without one is remade from its photo, and it stops if any can't be)
migrate    upgrade an old groups/names database to permanent face IDs + per-face labels
people     list who it knows
```

```
python face_id.py scan "D:\Photos"
python face_id.py cluster
python face_id.py name 4 9 15 Chris          # name groups 4, 9 and 15
python face_id.py name 7 --ignore            # not someone to recognize
python face_id.py people
python face_id.py recognize --camera 0
python face_id.py recognize "D:\some\photos" --out tagged
python face_id.py reembed
python face_id.py migrate "<photo folder>"   # also runs automatically the first time Face Labels starts
```

`cluster` group numbers are only valid until the next `cluster`; since labels are stored per face, naming a
group labels its faces permanently. The labeling tool is the preferred way to name faces.

Image handling: `load_image()` applies EXIF rotation, decodes HEIC/HEIF (pillow-heif), shrinks to
`MAX_SIDE`. Supported: jpg/jpeg/png/webp/bmp/heic/heif/gif/tif/tiff; videos mov/mp4/m4v/avi/mkv.

---

## 4. Models and measured tuning values

Detection: **OpenCV YuNet** (`cv2.FaceDetectorYN`, CPU). Quality: **eDifFIQA** tiny.
Recognition: **ArcFace** (ResNet-50, WebFace600K, from InsightFace buffalo_l) when installed, else **SFace**.
`FACE_MODEL=sface_2021dec` forces the old model. InsightFace pretrained models are non-commercial;
commercial use needs a license. Faces are aligned to the 5-point 112×112 template (`align()`).

```python
MAX_SIDE  = 1280   # photos shrunk to this before detection
DET_SCORE = 0.85   # detector confidence needed to keep a face
MIN_FACE  = 40     # px; smaller faces too blurry to recognize
Q_ENROLL  = 0.30   # faces below this eDifFIQA quality are not used to learn a person
Q_EXPR    = 0.25   # faces below this quality aren't scored for expression
MAX_TURN  = 0.35   # nose offset / eye distance (~35°); more turned faces aren't scored for expression

REC_MODELS = {
    "arcface_w600k_r50": dict(file="arcface_w600k_r50.onnx",              dim=512, match=0.35, cluster=0.70),
    "sface_2021dec":     dict(file="face_recognition_sface_2021dec.onnx", dim=128, match=0.45, cluster=0.58),
}
# match   = cosine similarity needed to call a face a known person
# cluster = cosine distance for grouping faces as one person

# labels.py
AUTO   = 0.45   # auto-name a new face at or above this score ...
MARGIN = 0.10   # ... if the next most likely person scores at least this much lower
TOPK   = 5      # a person's score = mean similarity of their 5 closest face prints
```

Measured on Chris's labeled faces:
- **ArcFace @ 0.35**: 100% of labeled faces correct, 0% of strangers named; on simulated blurry webcam
  frames 76% correct vs 44% for SFace. Clustering at 0.70 put Chris in 2 groups instead of 4.
- **SFace @ 0.45**: 99.3% correct, 0% strangers named (0.40 named 4.4% of strangers).
- **Auto-naming (AUTO 0.45 + MARGIN 0.10)**: named 89% of labeled faces with no mistakes; no stranger
  scored above 0.35.
- **AdaFace IR-101** (WebFace12M, `tools/get_adaface.py`): 8.5 ms/face on GPU, but 95.5% vs ArcFace 99.4%
  on clean labeled faces, about the same on blurry; ArcFace+AdaFace combined 97.7%. Kept ArcFace
  (`tools/eval_models.py`, results in `data/model_eval.json`).
- **Webcam faces, 2026-11-08** (`tools/webcam_eval.py`, `data/webcam_eval.json`): 75 confirmed webcam snapshots
  (all Chris) against 950 photo faces of 93 people, at 0.35. Each snapshot became a simulated track of 7 spoiled
  frames (blur, motion, low resolution, noise, JPEG, light); 3 seeds: one frame 82.9% correct; vote over 7 frame
  names (old tracker rule) 93.3%; **average of the 7 face prints, quality-weighted, 98.2%**; plus mirror image
  98.2% (no gain, not used). 0% wrong and 0% stranger named for all. Real runs of 3 unspoiled snapshots: 100%
  both ways. The tracker now names by the average (`/api/who`). Not measured yet: head-turn weighting and
  per-person settings (need turned-head test faces and webcam faces of more people).
- Matching rule: a face is labeled with the known person whose 5 closest face prints are most similar,
  if similarity ≥ the threshold; otherwise "Unknown".

### Compute (FACE_DEVICE = auto | gpu | npu | cpu)

| Processor | ArcFace, one face | Notes |
|---|---|---|
| GPU (Radeon 8060S, onnxruntime DirectML) | 2.6–3.6 ms | `auto` default; identical results to CPU |
| CPU (OpenCV DNN) | ~12 ms | fallback |
| NPU (Ryzen AI 1.8 VitisAI, BF16) | ~25 ms | 99.97% identical; first start compiles ~6 min (342 s) |

The recognizer, quality and expression models use ONNX Runtime when available; YuNet stays on the CPU.
`auto` decides from the hardware: DirectML (measured) when present; else CUDA or CoreML for each model whose
start-up check passes (`_gpu_check`: GPU and OpenCV CPU on a fixed test input, within `GPU_TOLERANCE` 1%, and
faster by median of 5 runs; decisions printed and in `face_id.GPU_DECISIONS`); else the CPU. Tried on this PC with
ONNX Runtime's CPU standing in for the GPU (2026-11-10): ArcFace and eDifFIQA 0.00% apart from OpenCV, so real
models pass the agreement part. Not yet run on an actual CUDA or CoreML machine.
The NPU needs the Ryzen AI env's Python (`X:\ProgramData\miniconda3\envs\ryzen-ai-1.8.0\python.exe`) and
the static model in `models\npu\`; `tools/hw_bench.py` compiles it and re-measures all three
(`data/hw_bench.json`). `tracker.py` prints `Recognition model: <name> on <device>` at startup.

---

## 5. Live tracker (tracker.py + tracker_ui.html)

- Browser page served at `http://127.0.0.1:8766`; the page grabs webcam frames in the browser and POSTs
  JPEGs to `/api/recognize` (GET `/api/people` lists names). The server detects at 640 px in one pass and
  returns box, keypoints, name, similarity, quality and expression for each face.
- `Engine` reloads `people.npz` whenever its mtime changes, so names saved in the labeling tool appear
  automatically. Each request takes a snapshot of the identifier and passes its own threshold, so two tabs
  with different strictness don't interfere; detection runs under a lock (OpenCV isn't thread-safe).
- Each face gets a box; **name + match %** are drawn on the face near the **top of the head** (Chris's
  explicit preference — not above the bounding box). Unnamed people show as **Unknown**.
- **Match strictness** slider 0.20–0.60, default **0.35** (the measured ArcFace setting; the earlier 0.45
  was the SFace-era value). Per-track name voting over the last **7** frames, each vote weighted by face
  quality (similarity breaks ties), so sharp frames outvote blurry ones.
- **Display**: mirror view, keypoints, motion trail, show or blur unknown faces, smoothing.
- **Camera**: pick webcam by name; auto-starts the last-used camera on page load (localStorage) and
  stops auto-starting if the user explicitly clicks **Stop camera**. **Open video** tracks a file.
- **Known people** card is a collapsible `<details>`, collapsed by default, remembers open/closed state,
  shows a count like "2 of 26" (on-camera of known). Live readout still lists detected names.
- Hardening (`localguard.refuse_reason()`, used by both servers → 403 otherwise): Host must be
  `127.0.0.1:<port>` or `localhost:<port>` (defeats DNS rebinding); POSTs must carry `X-Face-Tool: 1` and,
  if an Origin is sent, come from the tool's own page. `BrokenPipeError`/`ConnectionResetError` on write are
  swallowed (page closed/reloaded mid-request); a Windows port fix sits at the top of `main()`.

---

## 6. Expression (added 2026-10-29)

Asked "how hard would it be to detect emotion?" — answer: easy to get *something*, hard to get it
*good*. Implemented as expression-on-the-face (not felt emotion); README wording is deliberately honest
("this face currently looks …"). 7 classes: angry, disgust, fearful, happy, neutral, sad, surprised
(OpenCV Zoo MobileFaceNet, Apache 2.0, 88% on RAF-DB).

Implementation (`face_id.py` → `Faces.expression()`, `Faces._optional()`, `head_turn()`;
`tracker.py` → gating in `Engine.recognize()`; `tracker_ui.html` → section `// ---- expression`):

- **Only readable faces are scored**: skip frames with quality < `Q_EXPR` (0.25) or head turned more
  than `MAX_TURN` (~35°). Blur/profile makes the model *wrong* (usually "sad"/"neutral"), not unsure.
- **Score twice** (as-is + mirrored), average. Mirrored views agree 93% of the time on stored faces;
  both runs ≈ 0.7 ms per face on GPU.
- **Frames vote** (tracker_ui.html): last 10 scored frames (~1 s), weighted by quality; report a label
  only if there are at least 3 votes (`X_MIN_VOTES`), it has ≥ 45% of the vote (`X_SURE`) and leads the
  runner-up by 12 points (`X_MARGIN`); otherwise shown dimmed with "?". After 10 unscored frames in a row
  the reading is dropped rather than left stale. 95% of stored faces give a clear winner.
- **Calibrate to each person** (on by default): slow running average of each named person's readings,
  kept in the browser (localStorage key `ft-usual`); trusted after ~100 frames. Readings that match the
  usual look are tagged **usual** (dim); departures show bright. **Forget learned usual looks** clears it.
  Functions: `learnUsual`, `readExpression`.
- Reliable labels: happy, surprised, neutral. Fearful/disgust are rare in training data and most
  confused; angry/sad are often just a resting face. Photo-library distribution: ~40–45% neutral,
  ~40–45% happy, rest thin.
- Expression is never stored; display only. The model is optional (the tracker says so at startup if
  missing). UI toggles: **Show expression**, **Calibrate to each person**.
- `python tools\expr_check.py` scores a sample of stored faces (ms/face, label spread, mirror agreement)
  and runs one tracker request on a stored photo — confirms the model is installed and working.
- Status at end of session: Chris confirmed "It is working now!" and the collapsible Known-people card
  "worked!!".

---

## 7. Training data and history

Current data (2026-10-29):
- **5,702 photos** read from the owner's iCloud Photos folder, of which 112 were skipped as copies.
- **1,551 faces** in `faces.npz`; **786 labeled**: 456 by Chris, 328 named automatically, 231 of them
  `#ignore`.
- **26 named people**; `people.npz` holds 544 face prints.

History:
- The first scan read **1,936 photos** → **638 faces** after fixing HEIC decoding and adding
  close-up/selfie handling. That listing truncated at 2,000 entries, so ~60 photos were missed at first.
- Faces were clustered; Chris named **14 people** in the first pass, and more later in the labeling tool.
- Groups that were artwork, video-game characters, tattoos, sculptures, or ambiguous images were ignored
  at Chris's request.
- The database was then migrated from group-based labels (`groups.json`/`names.json`/`scanned.json`) to
  permanent face IDs with per-face labels. (The pre-migration files were removed in the Oct 30 cleanup.
  They were in the old git history, which was replaced on 2026-11-09; a local backup bundle of that history exists.)

Timeline of the Project's chats:

| Date (2026) | Chat | What happened |
|---|---|---|
| ~Oct 26 | *Simple face tracking app* | HTML MediaPipe tracker → camera selector → CDN fix → Python recognizer (YuNet+SFace) trained on iCloud photos → clustering/naming → separate labeling tool → separate live tracker with on-face overlay → setup/launch .bat files → ArcFace + eDifFIQA + DirectML → auto-start camera → tracker hardening. |
| Oct 29 08:04 | — (not in the export; dated by `data\backups\`) | Migration to per-face labels. By the end of the day the labeling tool also had import, suggestions, auto-naming, duplicate detection, checks and undo, and NPU/AdaFace had been evaluated; when each was added isn't recorded. |
| Oct 29 13:37 | *Labeling tool launch* | Launched tracker and labeling tool on the PC; clarified "the recognizer" = labeling tool. |
| Oct 29 14:06 | *Emotion detection from face tracking* | Expression feature (gating, mirrored scoring, voting, per-person calibration), README update, collapsible Known-people card, opened repo in VS Code. |
| Oct 29 16:37 | *Export command* | `/export` attempt; led to this document. |

Releases (FaceRecognizer.exe, GitHub Releases; `models-v1` holds the model files for first-start downloads):

| Version | Date (2026) | What changed |
|---|---|---|
| v1.0-v1.1 | Nov 8 | First releases of FaceRecognizer.exe: both apps in one window (`app.py`, pywebview) |
| v1.2 | Nov 8 | Video import (people followed, 3 clearest faces), moved photos found again (`relocate`) |
| v1.3 | Nov 8 | Import progress always visible, face mesh drawn in shade groups |
| v1.4 | Nov 9 | Possible faces (low detector cut-off), Find more faces, Draw a face |
| v1.5 | Nov 9 | Describe photo (LM Studio; built-in Florence-2 fallback), stacks of the same picture |
| v1.5.1 | Nov 9 | Models fetched from their publishers (OpenCV Zoo, MediaPipe, InsightFace), `models-v1` as fallback |
| v1.6 | Nov 9 | All of `data/` taken out of the repository and its history (repo restarted as one commit); Amazon keys enterable in the app; "No photos yet" welcome |
| v1.9 | Nov 10 | Released 2026-11-10 for Windows, macOS (Apple Silicon, macOS 14+, not yet Apple-signed) and Linux (Ubuntu 22.04+, tested on 26.04), from CI's builds of 587b515: second round of review fixes. Pages load nothing from the internet (system fonts instead of Google Fonts) under a strict Content-Security-Policy; CORP, cross-site requests refused; tools trust the tab page only when the app started them; camera only for the app's tracker; request size limits; Collect faces saves each face alone; Ignore deletes the print and picture; one person's export and forget (**Their data…**); model files checked by SHA-256 (cached); pinned downloads in the tools; CI actions pinned to SHAs; Describe all no longer stops at a failing photo after the first four; alerts: an inbox (`alerts.py`, New / Acknowledged / Archived, the newest 500) with a bell on the tracker page and in the app's tab bar, a banner over any tab, the count in the window title and a taskbar flash; event types (arrive, leave, unusual for them, unknown face) and every timing as settings; each alert keeps what was measured then and again later, including the evidence lines (numbers and text only) |
| v1.8 | Nov 9 | Settings tab (every option in sections, `settings_bridge.js`), all tracker options remembered, alert list with Unknown, Describe photo server/model/API key settings (any OpenAI-style server, other devices with a warning); security fixes from the review; Python tests, CI with ruff and pytest, `labels.py` split, **Why?** for unreadable files. v1.6 release removed (v1.8 is the only app release; `models-v1` stays for first-start downloads) |

---

## 8. Chris's stated preferences and decisions (keep honoring these)

- The labeling/exclusion tool must be a **separate program** from the tracker.
- Name overlay goes **on the face / top of head**, not above the bounding box ("this is MUCH better").
- Ignore faces that are tattoos, art, sculptures, game characters.
- When something needs running on the PC, **run it** rather than giving manual instructions.
- Keep the README truthful about what expression detection can and can't do.
- Known-people list collapsed by default; live readout keeps names visible.
- Thresholds (match strictness default 0.35, `AUTO`/`MARGIN`, expression gates and votes) are measured
  values — don't change them without re-measuring.
- The repository is public for the code only: none of `data/`, no personal details; releases hold the .exe and a
  README.txt for new users. Web search: US services and English results only.
- Settings belong in one **Settings** tab as well as on each page (2026-11-09); alerts pick people from a list,
  Unknown included. Describe photo may use a server on any device, inside or outside the home network.

---

## 9. Environment gotchas (learned the hard way)

Python on this machine:
- The `python` on PATH is 3.15 (`C:\Python315`) and has **no** OpenCV/onnxruntime. The project's libraries
  are in Python 3.12 (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`), which the .bat files prefer
  because other apps add newer Pythons that `py -3` would pick. Use that interpreter for CLI/tools.

Webcam / OpenCV on this Windows machine:
- `cv2.CAP_DSHOW` alone didn't reliably open the USB webcam; iterate `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`.
- Retry `cap.read()` up to ~10 times with short sleeps; some webcams return empty frames while starting.
- Camera index 1 = the Pixel phone's camera via Phone Link, not the USB webcam. Pick cameras by name in the browser UI.

- **One GPU run at a time per process** (`face_id.ACCEL_LOCK`). FaceRecognizer.exe hosts the tracker and the
  labeling tool's webcam import in one process; with Collect faces on, both ran DirectML at once and the AMD
  driver crashed the app (access violation in `amdxc64.dll`, Windows Application log event 1000, 2026-11-08).
  Reproduced with two threads embedding at once (segfault, exit 139); with the lock: 8,192 runs, no crash, and
  the app ran 4+ min while importing 32 snapshots during live tracking. Loading models at the same moment
  (both programs start together) crashed onnxruntime too; loading is serialized by the same lock.

Claude ↔ PC tooling (relevant if continuing with Claude's device tools):
- `device_commit_files` fails above ~30 MB → large models were once shipped as 20 MB `.partN` files
  rejoined by `_join_parts()`. Removed 2026-11-08: the models are in Git LFS, and `fetch_models.py`
  downloads any that are missing: since 2026-11-09 six (seven since 2026-11-10: SFace) from their publishers (OpenCV Zoo on Hugging Face,
  MediaPipe storage, ArcFace's 160 MB out of InsightFace's buffalo_l.zip by HTTP range requests; byte-identical,
  SHA-256 checked), Wav2Small and fallbacks from the Release `models-v1`.
- `device_list_dir` truncates at 2,000 entries.
- Claude's browser tools can't open `file://` URLs — hence both apps are tiny local HTTP servers.
- Staging files in batches of ~50 lands under `/mnt/user-data/uploads/<lastfolder>/` with ~1 s lag per 50 MB.
- Opening a `cv2.imshow` window via computer-use needed full-tier access; the browser-based UI avoids this.

Code-editing convention used throughout: edits were applied with small Python scripts using a
`rep(old, new)` helper that asserts the target string occurs exactly once — safe for surgical changes.

---

### Expression refactor (2026-10-30)

Following the review in `Real-Time Facial Emotion Recognition.md`, with Chris's decisions:
- Voting, confidence flags and usual looks moved **browser-side into `expression.js`** (Node tests in
  `tests/`). Same measured constants. Fix: the usual look now learns each scored frame once (it used to
  re-learn the last vote on every unscored frame).
- **Geometry channel = MediaPipe Face Landmarker in Python** (`mediapipe` pip, `models/face_landmarker.task`,
  ~10 ms/face CPU, only when the page asks with `c=1`). **Evidence only**: shown, never changes the label.
- Each reading carries evidence (votes, share, lead, skip reason, learning progress, rough valence/arousal,
  label-typical cues); "Show expression evidence" toggle. Per-model device report + startup warmup.
- **Measurement = local posed sessions** (tracker page → Expression test → `data/expr_eval/*.jsonl`,
  numbers only; `tools/expr_eval.py` reports). Needed before any fusion weights or threshold change.
- Caveat: `mediapipe` installs `opencv-contrib-python` alongside `opencv-python` (both 5.0.0.93; works,
  but keep their versions identical).

### Other signals, voice and feeling reports (2026-10-31)

Background and research: [docs/WHY_LAYERED_EXPRESSION.md](docs/WHY_LAYERED_EXPRESSION.md). Chris asked to
implement every suggestion from it, including speech (camera + microphone + a voice-emotion model), and to
fit valence/arousal from his own data rather than download a face valence/arousal model.
- **`signals.js`** (browser, Node tests): head motion and approach, talking (jaw-opening spread over 1.5 s,
  confirmed by mic voice activity), blink rate, gaze-away share, smile rise speed / asymmetry / how long a
  reading held, **webcam pulse** (POS rPPG on forehead + cheeks, with SNR; off by default), **microphone**
  features (noise-floor VAD, autocorrelation pitch, pitch SD in semitones, loudness, syllable-rate proxy;
  off by default; audio never stored), feeling-report summaries, `applyFit`.
- **Voice model:** audEERING **Wav2Small** (17K params, arousal/dominance/valence, CC BY-NC-SA 4.0,
  non-commercial). `tools/get_wav2small.py` reads the safetensors as plain arrays, writes the network out
  locally (no downloaded code runs) and exports `models/wav2small.onnx` (matches PyTorch to 1e-4). Runs on
  CPU via ONNX Runtime in `face_id.Voice`; `POST /api/voice` takes 0.5–10 s of float32 16 kHz audio.
  Validated only against PyTorch and a synthetic tone, not yet on real speech.
- **`expression.js`:** state model (HMM filter, `H_STAY` 0.9, unmeasured) shown when it disagrees with the
  vote; temperature-calibrated confidence ("cal NN%") from `data/expr_calibration.json`.
- Two label-changing options, **off by default until measured**: skip expression while talking, use the
  state model.
- **Feeling check:** opt-in self-report (pleasant 1–7, energized 1–7, who) saved with a 28-number summary
  of the previous 30 s to `data/expr_eval/feel-<day>.jsonl`; `POST /api/expr-eval/report` validates it.
- **Expression test** now ends with two talking steps (neutral, smiling) recorded with `talk`, `tv` (jaw
  score) and `vad`; frames also record face brightness.
- **`tools/expr_eval.py`** sections: accuracy (vote vs state model), calibration (writes
  `data/expr_calibration.json`), accuracy by person/quality/size/turn/brightness, talking (misreads, best
  `TALK_JAW_SD`, mic agreement), cues, feeling reports (correlations; ridge fit per person ≥ 15 reports and
  shared ≥ 20, kept only when leave-one-out beats guessing the average; writes `data/va_fit.json`).
- First real session (2026-10-30, 549 frames): happy and neutral 98% voted+clear; posed angry read as
  disgust 84%, fearful as happy 85%, sad as disgust 71%; fitted temperature ≈ 6 (strongly overconfident).

### Body posture (2026-10-31)

Chris asked whether skeletal posture indicates facial emotion. Answer: not the *facial* expression, but
emotion in its own right; strongest for arousal and intense emotions, weak for valence, and not specific.
Added as a third channel, evidence only:
- `face_id.Faces.poses()` (MediaPipe Pose Landmarker lite, up to 4 bodies, ~15 ms/frame CPU),
  `pose_for_face()` (nose inside the face box), `hand_on_face()` (wrist/index over the face box + 15%),
  `POSE_POINTS` (13 points sent to the page). Tracker sends them with `p=1`; posed tests always record them.
- `signals.js`: `postureOf` (head height and neck in shoulder widths, tilt, arms crossed), lean against the
  last 30 s, body movement, breathing (shoulder rise/fall, 6–30/min, still body only), hand on face; 7 new
  feeling-report features (35 in all). Body outline drawn on the video.
- Option "Skip expression when a hand is on the face" (off until measured). Expression test gains a
  "chin on your hand" step (`handAsked`); `expr_eval.py` section 5 measures the detector and its effect.
- Checked on real photos (13/13 points, correct face matching, 45 ms per frame with everything on) and in
  headless Edge; breathing and posture thresholds (`LEAN_ON`, `CROSS_FRAC`, `BREATH_*`) are first guesses.

### Each person's normal (2026-10-31)

Chris: the posture answers are obvious one by one; what matters is reading them in the context of the
individual, and of interactions between identified people. Step 1 (done): per-person baselines.
- `signals.js` `Baselines`: for each named person, a running mean and spread of 17 signals (`B_KEYS`:
  head, neck, tilt, body/head motion, smile, jaw, blinks, gaze, pitch, pitch SD, loudness, speech rate,
  voice arousal/valence, pulse, breathing), one sample a second, trusted after 120 samples (2 min), then
  a slow running average (`B_RATE` 1/600, ~10 min). Kept per context: `alone` (only face in view) and
  `company`, falling back to the overall normal until the context's own is trusted. Spread floors per
  signal so near-constant signals don't produce huge z values.
- `FaceSignals.snapshot()` gives the current values; `unusualEvidence()` shows |z| >= 2 ("unusual for
  Chris (alone): head -2.3 SD"). Controlled by "Calibrate to each person"; stored in localStorage
  `ft-baselines` (never in `data/`). "Forget what's learned" works for everyone or
  one person (usual looks too). Feeling reports gain `withOthers`.
- Step 2 (to discuss): interaction layer between identified people (turn-taking, attention, movement
  synchrony, mimicry, approach), framed by Kenny's Social Relations Model (actor, partner, relationship).

## 10. Open items / ideas not yet done

### Review of 2026-11-09 (code, security, tests): to do, by priority

Done the same day (in v1.8, before release), all checked in the running app:
- **Framing:** every answer of the two programs carries `frame-ancestors` (only themselves and the app's tab page);
  the tab page can't be framed. A page on another local port framing the labeling tool was blocked.
  `settings_bridge.js` also checks the parent's real origin (`location.ancestorOrigins`).
- **Describe API key:** cleared when the server address changes; Test sends it only to its own address; redirects
  aren't followed (the key no longer reached a second server in a redirect test); answers capped at 4 MB, text at
  4,000 characters, non-text answers refused.
- **Shell GETs** `/update`, `/open-release`, `/open` need `X-Face-Tool` (any website got 403); the shell uses
  `localguard.refuse_reason` (Host and port).
- **Bugs:** one `localguard.bind` without address reuse for all three servers (a second bind of 8765 now fails with
  10048); `Importer.begin` starts imports, scans and Describe all under one lock; locks for `photo_cache`, the
  Web search monthly counter and the Florence download start; negative face indexes refused; uncaught handler
  errors answered with a JSON 500 (`localguard.answer_errors`).
- **Requirements:** pinned to the versions in use; only `opencv-contrib-python` (setup.bat removes a plain
  `opencv-python`); pywebview added; `requirements-build.txt` for the build.

Still open (security, low): keys in plain files (DPAPI possible); a committed PyInstaller spec file. (Refusing
`Sec-Fetch-Site: cross-site` was done in v1.9, see below.)

### Review of 2026-11-09, second round: done the same night (v1.9, code; tests, not yet a release)
- **Nothing from the internet in the pages:** the tracker page used Google Fonts; now system fonts (`system-ui`,
  `ui-monospace`). Every answer has a Content-Security-Policy, `default-src 'self'` plus only what each page uses
  (checked in headless Edge on a test copy: tracker with microphone worklet and blob: video, labeling page with
  blob: pictures, the tab page's frames and Settings round trip, no violations); inline `onclick` attributes
  moved into the scripts so `script-src` needs no `'unsafe-inline'`. `Cross-Origin-Resource-Policy: same-origin`;
  `Sec-Fetch-Site: cross-site` refused.
- **Framing:** `FRAME_PARENTS` starts empty; `app.py` adds its tab page before starting the programs, so a .bat copy
  trusts nobody (the app's tab then says to close that copy). Camera and microphone pre-allowed only for the
  app's own tracker port. Request bodies: `Handler.body(limit)` everywhere (negative length used to read to close).
- **Collect faces** saves each face alone (0.6 face widths of room, as `rebuild_crop` uses; enlarged to 160 px;
  other faces greyed out), never the whole frame. **Ignore** deletes the face's print and picture (Undo restores
  them in the same session); ID, photo and box stay, so nothing is read again (a copy of a photo with ignored
  faces is matched on its other faces, or on the fingerprint alone). Consequence: look-alikes of ignored faces
  are no longer suggested as "ignore", and ignored possible faces no longer teach the faceness model.
- **One person's data** (`persondata.py`, **Their data…**): export to a folder, or forget everywhere; the tracker
  page's storage via the tab page (`forgetPerson`). The tracker no longer writes names to `app.log`.
- **Bugs:** Describe all lost the photo of a failed load after the first four (AttributeError ended the job);
  `Baselines.progress` gave -Infinity with nothing learned.
- **Downloads:** model files are checked by SHA-256 also when already there (cached by size and time);
  `get_wav2small.py` and `get_adaface.py` download at a pinned Hugging Face commit with size and SHA-256 (the
  API's LFS ids, equal to the local copies). The face-tracker demo pins its MediaPipe file with an import-map
  `integrity`; its engine files and model can't be pinned that way (README says it's the internet demo).
- **CI:** actions pinned to commit SHAs (checkout v4.4.0, setup-node v4.4.0, setup-python v5.6.0).
- Not done: moving `*.task` and `wav2small_src/model.safetensors` to Git LFS (would rewrite history).

Structure and tests: done the same evening (in v1.8):
- **Tests:** `face_id` no longer stops on import without models (programs call `require_models()`); `paths.py`
  gives one `ROOT`/`HOME` with `FACE_ROOT`/`FACE_HOME` overrides and one `atomic_write`. 64 pytest tests (request
  checks, describe settings, key never sent elsewhere, redirects, Web search keys and caps under 20 parallel
  searches, model download from a local range-request server and fallback, labeling/undo/rename, face-picture
  migration and matching by ID, empty new-user start, error records, threshold tripwire) and 41 Node tests (now
  also the Settings bridge's origin checks, and that every page script parses and is served and packed). Writing
  them found two bugs, fixed: saving Amazon keys broke in the refactor (caught before any commit) and
  `clean_url("http://")` was accepted.
- **CI:** Node tests, ruff (`ruff.toml`: E, F, W, B, which include the pyflakes rules; long lines not
  enforced) and pytest, no LFS;
  pinned Linux packages checked to exist.
- **Structure:** `labels.py` 1,750 → 775 lines (`photofiles.py`, `photofacts.py`, `relocate.py`, `importer.py`,
  re-exported); one `face_id.iou`; the servers share `localguard.Handler`; the tab page left the Python string
  (`app_ui.html`/`.js`) and the page scripts left the HTML (`tracker_ui.js`, `label_ui.js`).
- **Error records:** imports and Describe all keep the last 20 unreadable files with the reason; **Why?** shows them.

Still open (structure): `label_tool.make_handler` is still one long function (its helpers and routes share
the database through a closure); splitting it into a class with route methods is the next step if it keeps growing.

- Record more posed sessions (now with the talking and "chin on hand" steps) and feeling reports; then
  decide on `TALK_JAW_SD`, the talking and hand gates, the state model and any cue fusion from
  `tools/expr_eval.py`.
- Check the voice model, pulse, posture and breathing live (real speech, a still face, a seated body);
  none has been validated on the webcam yet.
- If continuing, re-measure thresholds after any
model change (`python face_id.py reembed`, then `tools\eval_models.py` and `people`).
