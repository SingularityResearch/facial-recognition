# Facial-Recognition — Project Context Export

Exported 2026-10-29 from the Claude Project **"Facial-Recognition"**
(*Detect known faces and provide for the management and labeling of new or unknown faces.*).
Revised the same day to match the code and data as they now stand, and kept up to date since
(last: 2026-11-13, v1.22 released for Windows, macOS and Linux after a full review of the code and docs (§7 "Releases", §10); before it, v1.20: the shared database and sync (v1.16), keys under a
passphrase kept in step on every device (v1.17), original photos from other devices (v1.18), every device's live
camera in a Cameras tab (v1.19-v1.20); see §4b, §5b, §7 "Releases", §9 and §10).

This file collects everything established across the Project's chats so a coding assistant or a human
opening the repo in VS Code has the full background: what was built, why, where it lives, the measured
tuning values, the owner's stated preferences, and the environment gotchas that cost time before.

**Related docs:** [README](README.md) (goals, decisions, research value) · [AGENTS.md](AGENTS.md) (layout, conventions, security rules, checks; [CLAUDE.md](CLAUDE.md) reads it) ·
[Face Recognizer README](face-recognizer/README.md) (user docs) ·
[Face Tracker README](face-tracker/README.md) · [Settings](docs/SETTINGS.md) (every setting, environment
variables, what reaches the network) ·
[Why expression reading is layered](docs/WHY_LAYERED_EXPRESSION.md) (the science) ·
[Real-Time Facial Emotion Recognition](Real-Time%20Facial%20Emotion%20Recognition.md) (design review, see §
"Expression refactor").

---

## 1. What this is

A fully local face-recognition system for a USB webcam, built for the owner:

1. **Face tracker** — the original standalone browser page (MediaPipe face detector).
2. **Face recognizer** — a Python pipeline that learned specific people from the owner's personal photo
   library and names them live on the webcam.
3. **Labeling tool** — a local browser app for adding photos, confirming suggested names, naming new
   people, ignoring false positives and fixing mistakes.
4. **Live tracker** — a local browser app that overlays each person's name, match confidence, and
   (optionally) a facial-expression reading directly on their face.

Nothing leaves the machine. No photos or face data are sent anywhere, except one face or photo when the
owner clicks **Web search** in the labeling tool: the face goes to Amazon Rekognition and the photo to Google Cloud Vision (never automatic),
and photos to be described when **Describe photo** is pointed at a model server on another device (Settings tab, since v1.8).
Since v1.16 the owner's devices can also share everything private through their own **shared database** (PostgreSQL +
pgvector on a server they own, TLS checked against its own CA; optional, off for a new copy, §4b); with it, the devices
show each other original photos (v1.18) and their live cameras (v1.19) directly over the LAN through each device's
**peer service** (TLS with a pinned certificate, signed requests; §5b). Nothing goes to anyone else's server.

Since 2026-11-08 the tracker and the labeling tool also run together as **FaceRecognizer.exe** (`app.py`): one window
with Face tracking, Labeling, (since v1.8) Settings and (since v1.19) Cameras tabs, each program still on its own
server and page. The same app is built for macOS and Linux (since v1.9).

---

## 2. Where it lives (Windows; macOS and Linux below)

```
facial-recognition\
├── AGENTS.md (the guide), CLAUDE.md (reads AGENTS.md), PROJECT_CONTEXT.md, README.md
├── .github\workflows\checks.yml   CI: node tests, ruff, pytest on Windows, macOS, Linux; then builds for all three
├── Real-Time Facial Emotion Recognition.md   expression design review (LM Studio export)
├── docs\
│   ├── SETTINGS.md                          every setting, its default and purpose; environment variables;
│   │                                        what reaches the network (kept in step with each SettingsBridge list)
│   └── WHY_LAYERED_EXPRESSION.md            why expression reading is layered, with references
├── face-tracker\                 original standalone HTML tracker (MediaPipe), double-click index.html
│   └── index.html
└── face-recognizer\              the Python project (everything below shares one data\ folder)
    ├── face_id.py                core library + CLI: scan / cluster / name / people / recognize / reembed / migrate
    ├── paths.py                  ROOT/HOME (FACE_ROOT/FACE_HOME overrides), atomic_write
    ├── labels.py                 labeling engine: LabelDB (per-face labels, undo); imports what it uses of the four below
    ├── photofiles.py, photofacts.py, relocate.py, importer.py   hashes/cloud files; dates/tags; upgrade/moved
    │                             photos; background import (split out of labels.py 2026-11-09)
    ├── label_tool.py             labeling web app  → http://127.0.0.1:8765   (launch: label.bat, label.sh)
    ├── label_ui.html, label_ui.js   labeling page and its script (served by label_tool.py)
    ├── tracker.py                live tracker server → http://127.0.0.1:8766 (launch: recognize.bat, recognize.sh)
    ├── tracker_ui.html, tracker_ui.js   live tracker page and its script (served by tracker.py)
    ├── expression.js             expression vote, state model, usual looks, evidence (browser; tests\)
    ├── signals.js                motion, talking, blinks, gaze, timing, pulse, voice, VA fit (browser; tests\)
    ├── settings_bridge.js        the app's Settings tab <-> each page's settings (postMessage; served by both servers)
    ├── page_store.js, devicesettings.py   each page's settings and each person's learned signals kept by the page's
    │                             own server (HOME/settings.json, HOME/person_signals.json); localStorage is the quick copy
    ├── app.py, app_ui.html, app_ui.js   FaceRecognizer.exe: one window, four tabs (Face tracking, Labeling,
    │                             Cameras, Settings); the tab page on 8764; window watchdog (/alive)
    ├── facedb.py, db\NNN_*.sql   optional shared database (PostgreSQL + pgvector): connection, migrations, device id
    ├── dbsync.py, dbsync_home.py   the sync: data\ (Local) and the private folder (HomePart) <-> the database
    ├── dbimport.py               a device's data\ and private folder merged into the database (Join; tools\db_import.py)
    ├── secretbox.py              keys encrypted on the device (scrypt + AES-256-GCM), passphrase versions per device
    ├── peer.py                   peer service on the LAN (port 8767): original photos and live camera streams
    ├── camstream.py, camrelay.py   this device's camera frames and readout in memory; one relay per watched device
    ├── cameras_ui.html, cameras_ui.js   the Cameras tab: every streaming device in a grid (served by tracker.py)
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
    ├── hardware.py               what the computer has (CPU, memory, GPUs, NPU, AI apps, LM Studio on the network)
    ├── cudasupport.py            NVIDIA CUDA for the downloaded Linux app, fetched when asked in Settings
    ├── lmstudio.py               LM Studio here: all interfaces, mDNS announcement, models via the shared folder
    ├── tests\                    Node tests (*.test.js) and Python tests (test_*.py, pytest); ruff.toml
    ├── setup.bat                 installs Python 3.12 via winget, then pip installs requirements.txt
    ├── recognize.bat             starts tracker.py (checks cv2/numpy are installed first)
    ├── label.bat                 starts label_tool.py
    ├── requirements.txt          pinned: opencv-contrib-python (only; mediapipe needs it), numpy, scikit-learn, pillow,
    │                             pillow-heif, onnxruntime-directml (Windows) / onnxruntime, mediapipe, boto3, pywebview,
    │                             truststore, psycopg, pgvector, cryptography; requirements-build.txt adds PyInstaller,
    │                             requirements-dev.txt pytest and ruff
    ├── README.md                 user-facing docs; kept in sync with code changes
    ├── tools\
    │   ├── eval_models.py        ArcFace vs AdaFace vs both, on clean/blurred/tiny/motion faces → data\model_eval.json
    │   ├── hw_bench.py           CPU vs GPU vs NPU timing; compiles the NPU model → data\hw_bench.json
    │   ├── expr_check.py         sanity check for the expression model
    │   ├── expr_eval.py          posed tests + feeling reports → accuracy, calibration, talking, VA fit
    │   ├── get_adaface.py        downloads the official AdaFace IR-101 weights and converts them to ONNX
    │   ├── get_wav2small.py      downloads audEERING Wav2Small weights and converts them to ONNX
    │   ├── get_yunet_dynamic.py  YuNet with free input sizes, for GPU face finding (published in models-v1)
    │   ├── import_bench.py       photo-import timing
    │   ├── import_profile.py     each step of an import timed on a folder
    │   ├── db_certs.py           the shared database server's own CA and server certificate
    │   ├── db_import.py          copies a device's data\ and private folder into the shared database (reads only)
    │   ├── websearch_check.py    the Web search on picture files
    │   └── webcam_eval.py        live naming on confirmed webcam snapshots (/api/who)
    ├── models\
    │   ├── face_detection_yunet_2023mar.onnx           YuNet detector (OpenCV Zoo)
    │   ├── face_detection_yunet_2023mar_dynamic.onnx   the same with free input sizes (GPU face finding)
    │   ├── arcface_w600k_r50.onnx                      ArcFace recognizer (174 MB)
    │   ├── face_recognition_sface_2021dec.onnx         SFace, fallback recognizer (38.7 MB)
    │   ├── ediffiqa_tiny_jun2024.onnx                  eDifFIQA face quality (optional)
    │   ├── facial_expression_recognition_mobilefacenet_2022july.onnx   7-class expression model (optional)
    │   ├── (adaface_ir101_webface12m.onnx, adaface_src\: AdaFace, evaluated and not used; made by
    │   │    tools\get_adaface.py when wanted, not kept since 2026-11-14)
    │   ├── face_landmarker.task                        MediaPipe blendshapes (optional)
    │   ├── wav2small.onnx, wav2small_src\              voice arousal/dominance/valence, 69 KB (optional; CC BY-NC-SA)
    │   ├── pose_landmarker_lite.task                   MediaPipe body posture, 5.8 MB (optional; Apache 2.0)
    │   ├── hand_landmarker.task                        MediaPipe hands, 21 points each (optional, drawing only)
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
`presence\` (opt-in log), `alerts.json` (the alert inbox), `lmstudio.json` (the shared models folder),
`settings.json` and `person_signals.json` (each page's settings, each person's usual look and signal normals, since
v1.16; the pages keep a quick copy in their browser storage), `webview\` (the .exe window's browser storage),
`app.log` (each line stamped with the time in UTC since v1.19.6), `restarted.json` (the last restarts the app made itself: for a stuck
camera, and for a frozen window with how many in a row). With a shared database also: `database.json` + `database-ca.pem` (connection; password DPAPI-protected on
Windows, file mode 600 elsewhere), `device.json` (this device's id), `sync.sqlite` (each row's digest as last synced),
`secret.key` and `device-key.json` (the passphrase for keys and this device's X25519 key pair, protected like the
password), `peer-cert.pem`/`peer-key.pem`/`peer-key.json`/`peer.json` (the peer service's own certificate, its
key encrypted with a protected password, and settings),
`forgotten.json` (people forgotten here or on another device).

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

`face-recognizer\data_backup_20261029\` (a full copy of `data\` from before the per-face migration) is gone from
the checkout; `data\backups\` keeps the labels from each start.

Earlier in the project the two halves lived at two separate folders, `face-tracker` and `face-recognizer`;
they were later consolidated under `facial-recognition\` as the two subfolders above.

**Vocabulary the owner uses** (recorded so the right thing gets launched):
- "the recognizer" = the **labeling tool** (`label.bat`, port 8765), *not* `recognize.bat`.
- "the tracker" = the live tracker (`recognize.bat` → `tracker.py`, port 8766).

Hardware: a simple USB webcam; AMD Ryzen AI Max+ 395 PC (16-core CPU, Radeon 8060S GPU, XDNA 2 NPU). Also in use
since 2026-11-10: a Linux laptop (Ryzen 7 7840HS + RTX 4060) and a Windows NAS that hosts the shared database and the
LM Studio models folder; each runs the app.
Photo source for training: the owner's iCloud Photos folder (local path not recorded here).

---

## 3. Data model and pipeline

### Per-face labels (labels.py)

Every face has a **permanent ID**: its photo's content hash + where the face is in the photo
(`face_key()`, e.g. `6edd140b4d6dc4a9-94-848`). Labels are stored on the face, so regrouping, rescanning,
or moving/renaming photos never loses them:

```
{"<face id>": {"n": "Alex", "by": "me"}, ...}
  n    the person's name, or "#ignore" (stranger, artwork, not a face: never recognized)
  by   "me" (named by the owner) or "auto" (named automatically because the model was very sure)
  not  names this face is NOT, so they are never suggested again
  ok   a "might be mislabeled" warning was checked and the name kept
  s    the model's score when the face was named automatically
```

Groups are no longer stored. For unnamed faces the engine works out on the fly:
- **suggested** — the model thinks it knows who it is (score ≥ `MATCH`): the owner confirms or rejects.
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
  path by which a face or photo goes to an outside company without the owner setting up a server (see docs/SETTINGS.md,
  "What reaches the network").
- API: GET `/api/summary`, `/api/fids`, `/api/suggested`, `/api/unknown`, `/api/tagged`, `/api/auto`, `/api/person`,
  `/api/checks`, `/api/import`, `/api/photoinfo/…` (includes `hints`); POST `/api/act`, `/api/rename`, `/api/undo`,
  `/api/import`, `/api/import/stop` (`/api/autoname` had no caller and went on 2026-11-18). Added since: possible faces (`/api/possiblefaces`,
  `/api/possible/…`, `/api/morefaces`, `/api/drawface`), `/api/stacks`, Web search keys, Describe photo
  (`/api/describe/…`), one person's data (`/api/person/export`, `/api/person/forget`), This computer and LM Studio
  (`/api/hardware`, `/api/hardware/network`, `/api/lmstudio…`, `/api/cuda/download`, `/api/downloads`), the page's
  settings (`/api/settings`), the shared database (`/api/db`, `/api/db/setup`, `/api/db/copy`, `/api/db/passphrase…`,
  `/api/peer`) and `/api/secrets` (saved keys for the Settings tab only).

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
python face_id.py name 4 9 15 Alex           # name groups 4, 9 and 15
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

Detection: **OpenCV YuNet** (`cv2.FaceDetectorYN` on the CPU, or the same model through ONNX Runtime on a graphics processor when the start-up check passes). Quality: **eDifFIQA** tiny.
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

Measured on the owner's labeled faces:
- **ArcFace @ 0.35**: 100% of labeled faces correct, 0% of strangers named; on simulated blurry webcam
  frames 76% correct vs 44% for SFace. Clustering at 0.70 put the owner in 2 groups instead of 4.
- **SFace @ 0.45**: 99.3% correct, 0% strangers named (0.40 named 4.4% of strangers).
- **Auto-naming (AUTO 0.45 + MARGIN 0.10)**: named 89% of labeled faces with no mistakes; no stranger
  scored above 0.35.
- **AdaFace IR-101** (WebFace12M, `tools/get_adaface.py`): 8.5 ms/face on GPU, but 95.5% vs ArcFace 99.4%
  on clean labeled faces, about the same on blurry; ArcFace+AdaFace combined 97.7%. Kept ArcFace
  (`tools/eval_models.py`, results in `data/model_eval.json`).
- **Webcam faces, 2026-11-08** (`tools/webcam_eval.py`, `data/webcam_eval.json`): 75 confirmed webcam snapshots
  (all of the owner) against 950 photo faces of 93 people, at 0.35. Each snapshot became a simulated track of 7 spoiled
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

Every model runs through ONNX Runtime when it can, face finding (YuNet) included since 2026-11-10.
`auto` decides from the hardware (`hardware.py` lists it; Settings → This computer shows it): DirectML when present
(measured; YuNet on DirectML also passes its check here: 0.00% apart, 12.9 vs 26.4 ms with OpenCV); else CUDA or
CoreML for each model whose start-up check passes (`_gpu_check`: GPU and CPU on a fixed test input, within
`GPU_TOLERANCE` 1%, and faster by median of 10 runs after 3 warm-up runs; decisions printed and in
`face_id.GPU_DECISIONS`); else the CPU. Linux + RTX 4060 laptop (2026-11-10): every model passes:
ArcFace 0.10% apart, 4.7 vs 22 ms; YuNet 0.05%, 18 vs 24 ms per 1280x960 pass; quality and expression 0.03%.

GPU face finding (`_yunet_runner`, `_yunet_rows`): `face_detection_yunet_2023mar_dynamic.onnx` (input sizes marked
free by `tools/get_yunet_dynamic.py`; the original is fixed at 640x640 and OpenCV resizes it itself), decoded like
OpenCV's FaceDetectorYN. Measured on CUDA: a run takes 3-6 ms while the input size stays the same but ~30 ms when it
differs from the run before, so each size gets its own session (`_per_size`), and pictures are padded to multiples of
`DET_PAD` 320 (11 sizes for 400 photos; padding to 32 needed 82 and ran the 8 GB card out of memory; at most
`DET_SESSIONS` 12, then the first session takes the rest). With that: 17 ms per photo for all 6 detection passes vs
46-55 ms with OpenCV. Same faces: on 400 photos x 6 passes, regular faces (0.85) identical; 30 borderline possible
faces (0.45 cut-off, scores 0.46-0.56) differ. A GPU error switches face finding to OpenCV for the rest of the run.

Imports (`importer.py`): each file's work runs on `hardware.import_workers()` threads (threads - 2, 2..12), a photo is
read once (hash and picture from the same bytes) and a JPEG decoded straight at reduced size (`draft`: same faces on
120 JPEGs, prints >= 0.987 alike). 412 files from a network drive (180 HEIC, 156 JPEG, 64 PNG, 12 videos), fresh data, through
the app window on the Linux laptop: v1.9 89.5 s; workers + draft, CPU only 73.3 s; + GPU face finding 43.5 s (6, 10, 14,
20 workers: 47, 46, 44, 44 s). Left: decoding (HEIC ~300 ms, JPEG 66, PNG 90 ms on the CPU) and videos (~2 s each,
every frame decoded; OpenCV's pip build has no hardware video decoding and seeking was slower; key frames only with
PyAV would help but adds a dependency). `tools/import_profile.py` times each step; py-spy on the running app showed
the rest. First full import of a 6,189-file folder on the network drive with v1.9: 1,217 s, GPU 2% busy, ~10 cores.
v1.10 (released 2026-11-10, tag at 35b14cb): the downloaded Linux app ships plain ONNX Runtime; with an NVIDIA card,
Settings → This computer checks driver/card (`cudasupport.check`) and offers Download CUDA support (onnxruntime-gpu +
NVIDIA cu12/cuDNN 9 wheels from PyPI, 1.95 GB, 3.5 GB unpacked in `ROOT/cuda`); from the next start `activate()`
imports it ahead of the bundled copy. Tested in the built app on the RTX 4060 laptop: download ~1 min, all models
pass their checks onto CUDA; the 412-file import took 45.2 / 42.8 / 43.8 s with CUDA vs 62.6 / 62.7 s with
FACE_CUDA=off (4 vs 8-10 cores busy). The 1.6 GB sample alone takes 16 s to read over Wi-Fi 6 (~100 MB/s).

v1.11 (released 2026-11-11, tag at a947e45) carries the LM Studio work below, the download bar and docs/SETTINGS.md.
Known issue: for about the first minute after start the built app may say LM Studio here isn't running or announced
(its port check times out while the models load); the next pass, a minute later, corrects it.
v1.12 (released 2026-11-11, tag at ef5ae1a): Describe all shared with the other LM Studio computers, Settings
regrouped, drawn faces fixed (Importer._more_lock). CI's Ubuntu 22.04 build twice failed to resolve the Azure apt
mirror; its package step now retries from archive.ubuntu.com. Same known issue as v1.11.
After v1.12: on the NAS (a fresh Windows, 33 root certificates against 48 on the PC) the first start failed to download
the models from GitHub with CERTIFICATE_VERIFY_FAILED: Python reads only the roots already in the Windows store, and
Windows adds missing ones on demand for its own (schannel) connections only. paths.py now injects `truststore`
(pinned 0.10.5), so every HTTPS request is checked by the operating system.
v1.13 (released 2026-11-11, tag at 286b67f) carries that fix; CI's checks job installs truststore too.
v1.14 (2026-11-11): Start again button; the Live readout as fixed labeled cells (Expression, Face, Voice, Body, Their
normal; groups shown by setting, a dash for no reading, so blocks keep their height); Show expression evidence on by
default; Full body skeleton (pose points below the face) and Hand skeleton (MediaPipe hand_landmarker.task, 21 points,
~8 ms a frame on the 7840HS; optional in fetch_models.py, not in models-v1). First expr_eval calibration on the one posed
session (549 readings): temperature 6.03 (very overconfident), ECE 32.5 -> 10.5 points; angry/fearful 0%, sad 9%.
After v1.14 on the Linux laptop: a white app window (renderer never drew; CDP evaluate got no answer) after updating from a
2026-11-09 build; setting aside webview/GPUCache, DawnGraphiteCache, DawnWebGPUCache fixed it: app.clear_graphics_caches
now does that once per version. Its lid was shut, so the integrated camera sent one value (8/255) everywhere: the tracker
now warns when the picture's brightest and darkest points differ by under 12 of 255 for 6 s, offering the next camera.
v1.15 (released 2026-11-11, tag at e11b9ca) carries both; older releases deleted. v1.16-v1.20 (2026-11-11/12): the
shared database, keys, peer photos and the Cameras tab (§4b, §5b, §7 "Releases"); since v1.20 (tag at 6586b77) only
v1.20.0 and models-v1 remain. v1.20.1 and v1.22 to v1.22.2 (2026-11-12/13) followed (§7 "Releases"); since 2026-11-13
only the latest release and models-v1 are kept on GitHub.

LM Studio across the owner's computers (2026-11-10/11, after v1.10; `lmstudio.py`, `hardware.lmstudio_network`):
both computers serve LM Studio on 0.0.0.0:1234 (the Linux laptop: headless llmster 0.0.27 + CUDA 12 llama.cpp 2.72.0 as
a systemd user service with linger; the Windows PC: the LM Studio app). Announced on the LAN as `_lmstudio._tcp` +
`_http._tcp` (Linux: an Avahi service file; Windows: the app's `DnsServiceRegister`); discovery reads the port
from announcements (tested with a stand-in server on port 18234) and otherwise tries 1234 on each /24 of each real
interface (~3 s), naming computers by mDNS reverse lookup or NetBIOS (Windows answers only the latter); a computer's
two 10G addresses merge into one entry. Same models: Qwen3.6-35B-A3B and Qwen3.8-27B Q4_K_M + mmproj (hub manifests
copied too, so ids match `qwen/...`). Shared folder on the NAS: the Windows PC published 37 GB
in 143 s over 10G; the Linux laptop copied the 17.7 GB 27B from it over Wi-Fi 6 at ~85 MB/s (~3.5 min), byte-identical,
listed by LM Studio at once; `lms get` from Hugging Face ran at ~2 MB/s (~2 h). Gotcha: stopping `lms get` leaves
LM Studio's own download as a paused job that hides the model (cleared from `download-jobs-info.json` and
`single-downloads-info.json`, backups kept); the app now says so if a copied model isn't listed. Describe photo on 
the Linux laptop's 35B: 9 s for the first photo (model load, split between the RTX 4060 and RAM), 3 s after.
Describe all shared between the two (`describe.helpers`, Settings "Share Describe all with other computers",
2026-11-11, 2 sets of 40 library photos each way, test copy): the PC alone 134.6 / 138.4 s (3.4 s a photo); with the
laptop helping over Wi-Fi 99.4 / 86.7 s, the laptop doing 13 of 40 each time (~7 s a photo from the PC, against 3 s
asked on the laptop itself: Wi-Fi and the PC's 1024 px JPEGs), the PC's own rate unchanged: 1.5x overall.
The NPU needs the Ryzen AI environment's Python (its conda environment `ryzen-ai-1.8.0`) and
the static model in `models\npu\`; `tools/hw_bench.py` compiles it and re-measures all three
(`data/hw_bench.json`). `tracker.py` prints `Recognition model: <name> on <device>` at startup.

---

## 4b. Shared database and sync (v1.16-v1.18, 2026-11-11/12)

The owner's decision (2026-11-11): every device with Face Recognizer keeps its data and settings in one PostgreSQL 18.6
+ pgvector 0.8.7 database on a Windows NAS at home (schema `facerec`, role `facerec`: not a superuser, owns only its
schema): private data shared by all devices, settings per device. Choices: work from a local copy and sync; files stay
the default for a new copy (public repo: without `HOME/database.json` or `FACE_DB` nothing changes); keys encrypted
with a passphrase held on each device; per-person baselines shared. Set up from Settings → Shared database → Connect
(`facedb.setup`). Built in four phases (schema and one-time import; labeling data; private folder and keys; each
device's settings and person signals), released together as v1.16.
- Design: local-first. The existing stores and files stay the working copy and the offline cache. `dbsync.py` runs in
  the labeling tool's process: rows whose digest differs from the one recorded at the last sync (`HOME/sync.sqlite`)
  are pushed after each save (`face_id.SAVED_HOOKS` → `Sync.note`) and every minute; other devices' changes are pulled
  from a trigger-written `changes` log every 3 s (deletes as tombstones; new changes told apart by their
  transaction's visibility in the last pull's snapshot, db/009, since `seq` order isn't commit order) and applied
  through the same stores, so
  `people.npz` is rebuilt and the tracker reloads it by mtime. Two parts: `Local` (photos, faces, labels, Web search
  answers; the undo log only sent) and `dbsync_home.HomePart` (descriptions, Collect faces pictures, alerts, presence,
  shared fitted documents, keys, forgotten people, device settings, person signals; the app log, posed tests and the expression log only
  sent). Alerts, presence visits and descriptions nudge the sync when saved (since v1.19), so they reach the other
  devices within seconds. A new place that keeps data needs a table (`db/NNN_*.sql`, run under an advisory lock) and
  a part.
- Join (a device's first sync with a database, by its `instance` id): `dbimport` merges the device's own data in (by
  face ID and photo hash; a hand label beats an automatic one; Ignored stays Ignored), then everything is pulled.
- Recognition stays numpy in memory; pgvector (HNSW cosine on `print::vector(512)`) serves searches across devices
  (3 ms for a face's nearest neighbours over 8k faces).
- Face IDs and photo keys come from the photo's content, so the devices' data merge by key. The NAS found 4,015 of
  the same faces itself: half its prints identical to the PC's, 95% at cosine >= 0.985, 10 under 0.9 (blurry faces;
  its own detector placed them slightly differently); the copy first in the database (the PC's) is kept.
- First import (2026-11-11): 8,242 faces, 9,533 photos, 2,337 labels (119 people), 954 webcam snapshots, 8,337
  descriptions, 216 MB. Sync measured on a test copy: app -> database 0.5 s, database -> app 3 s.
- Server: SSL on (TLS 1.3 negotiated), pg_hba `hostssl` for the home LAN only, with scram-sha-256 (LAN connections
  without TLS refused); CA and server certificate from `tools/db_certs.py` (the CA key stays on the server, admin
  only). Connect logs in with sslmode=require, reads the CA from `server_ca` (db/003; the SCRAM login proves the
  server knows the password) and from then on connects with verify-ca. The password is DPAPI-protected on Windows
  (file mode 600 elsewhere).
- Each device's page settings, describer choices and LM Studio share folder are its own rows (`device_settings`;
  Settings can copy another device's); usual looks and signal baselines are shared (`person_signals`, per person: that
  person's last save wins). Web search use is counted in the database before each call (`count_use`), so each service's monthly cap
  holds across devices. Forgetting a person reaches every device (db/004 `forgotten`) and the tracker pages there;
  the name stops being a forget instruction once it is given to someone again (`forgotten.unforget`). The tracker
  page sends person signals per person and takes in other devices' every half minute (`page_store.js`).
- Keys (`secretbox.py`; passphrase versions since v1.17): encrypted on the device (scrypt + AES-256-GCM, the key's
  name as associated data) with a key from the owner's passphrase. Every passphrase is a version (db/006
  `passphrase_versions`: check value, fingerprint, who and when; one current). Until v1.22.1 each version was sealed
  to every row in `devices` with an X25519 public key (`passphrase_shares`), so the database password alone was
  enough to be sent it. Since v1.22.2 (db/011) a device gets it only from the owner: typed there, or asked for with
  a one-time code typed on a device that has it (`ask`/`approve`, `key_requests`; 8 characters from 32, scrypt
  2^16, 15 minutes: the code makes check values over the request and the answer, so no short code can be tried
  offline in time and neither side accepts the other without it). Holders show it (`devices.key_proof`); a change
  (typed twice; every key re-encrypted in one transaction) or restore is sealed only to devices showing the one it
  replaces and carries `passphrase_versions.link`, made with that one's key, so no device follows a version made by
  someone without its passphrase. Restore only versions the device held itself (`secret.key` `known`). A
  device not on the current version neither sends nor reads keys (`push` returns False), so it can't overwrite them
  with an old one. Settings lists each device's fingerprints of the passphrase and the database password.
- Saved keys shown masked (v1.16.3): the Web search keys, the describe server's key, the database password and (since
  v1.16.5, when it began to be kept in `HOME/secret.key`) the passphrase load into their Settings fields from
  `/api/secrets` (the labeling tool's own page only) as `*`; an eye button shows one only while held (mouse, touch,
  Space or Enter). The owner's choice, 2026-11-11.
- Original photos from other devices (v1.18, `peer.py`, db/007): a device that lacks an original asks one that has it
  (`photo_paths`) and shows the JPEG it answers with, never storing it; see §5b for the peer service. The photo view
  says which device a photo came from, or which device has it when that one can't be reached (and why: refused,
  unreachable, its error answer).
- Gotchas found while building it:
  - psycopg 3 without autocommit makes `conn.transaction()` a savepoint inside a transaction nobody commits (the
    first import ran and rolled back silently); `facedb.connect` is autocommit, so group writes in
    `with conn.transaction():`.
  - Floats must be hashed as float32 (v1.16.2): a face's quality came back from the database's `real` column as 0.9
    and from faces.npz as 0.8999999762, so its digest never matched after a restart; every device sent every face
    again on each start, the others pulled them, and the labeling lock was held long enough to stall the page (18
    minutes on the Linux laptop). The server now also keeps each face picture's SHA-256 (db/005), and a picture is
    sent or fetched only when the other side's differs.
  - The tracker noted `people.npz`'s mtime before loading it, so a read that failed while the file was being
    replaced (Windows locks it; the sync rewrites it often) left it knowing nobody until the file changed again (seen
    on the NAS after joining: 0 people). A failed read is now tried again (v1.16.1).
  - The "looks like a face" model for possible faces (six logistic regressions on face prints) was trained under the
    labeling lock and thrown away by every sync of faces: with 8,242 faces after joining that held the lock for
    seconds (minutes on the NAS and the laptop), and the labeling page and tracker stopped answering at start. It now
    trains in a background thread on a copy, keeping the previous model meanwhile (v1.16.5). Then, in the built Linux
    app, L-BFGS (SciPy and OpenBLAS threads competing with the live tracker's) held Python's lock for minutes and the
    whole window stopped responding: it uses liblinear now, the same L2 logistic regression (scores correlate 1.0 on
    7,966 faces, 99.7% the same possible faces shown), single-threaded C that lets the other threads run (v1.18.1,
    `LabelDB._fit`).
  - truststore replaces `ssl.SSLContext` app-wide and verifies every handshake against the system's roots, which
    refused the peer service's own pinned certificates: `peer.py` keeps the original class (`_context`).
  - A device's "streaming" time was written by the sync, and one slow sync step (22 s while many rows came in) made
    a running camera look offline to the others: the heartbeat (`camstream.start_heartbeat`, every 3 s) has its own
    thread and connection since v1.19.5, and a sync step over 2 s is logged with what it did.
- Times in UTC everywhere since this change (the owner's rule; `paths.utc_now`, `paths.utc_stamp`, timestamptz with
  the session in UTC; shown in local time only on the pages; `paths.parse_time` still reads older local-time text);
  9,345 local-time texts already imported converted.
- Tests: `test_facedb.py` and `test_sync.py` run against a real server when `FACE_TEST_PG` is set (CI job `database`,
  pgvector service; each test makes and drops its own schema).

---

## 5. Live tracker (tracker.py + tracker_ui.html)

- Browser page served at `http://127.0.0.1:8766`; the page grabs webcam frames in the browser and POSTs
  JPEGs to `/api/recognize` (GET `/api/people` lists names). The server detects at 640 px in one pass and
  returns box, keypoints, name, similarity, quality and expression for each face.
- `Engine` reloads `people.npz` whenever its mtime changes, so names saved in the labeling tool appear
  automatically. Each request takes a snapshot of the identifier and passes its own threshold, so two tabs
  with different strictness don't interfere; detection runs under a lock (OpenCV isn't thread-safe).
- Each face gets a box; **name + match %** are drawn on the face near the **top of the head** (the owner's
  explicit preference — not above the bounding box). Unnamed people show as **Unknown**.
- **Match strictness** slider 0.20–0.60, default **0.35** (the measured ArcFace setting; the earlier 0.45
  was the SFace-era value). Since 2026-11-08 each face is named from the quality-weighted average of its last **7**
  face prints (`/api/who`, one request for all faces in view; 98% against 93% for the vote, §4); the per-track vote
  over the last 7 frame names, weighted by face quality (similarity breaks ties), is kept as the fallback.
- **Display**: mirror view, keypoints, motion trail, show or blur unknown faces, smoothing; Full body and Hand
  skeletons (v1.14, drawing only, off by default).
- **Camera**: pick webcam by name; auto-starts the last-used camera on page load (localStorage). **Keep this camera
  running** (v1.19.2, on by default) starts it with the app and again within 10 s when it stops; a start that hangs
  for 30 s counts as failed, after two the page loads again, and if it still hangs the app starts again (v1.19.9, §9).
  Clicking **Stop camera** turns that option off. A flat picture (brightest and darkest points under 12 of 255 apart
  for 6 s: lid shut, shutter closed) gets a warning offering the next camera (v1.15). **Open video** tracks a file.
  **Stream this camera to my other devices** (with a shared database, §5b).
- **Known people** card is a collapsible `<details>`, collapsed by default, remembers open/closed state,
  shows a count like "2 of 26" (on-camera of known). Live readout still lists detected names.
- Hardening (`localguard.refuse_reason()`, used by every server → 403 otherwise): Host must be
  `127.0.0.1:<port>` or `localhost:<port>` (defeats DNS rebinding); POSTs must carry `X-Face-Tool: 1` and,
  if an Origin is sent, come from the tool's own page. `BrokenPipeError`/`ConnectionResetError` on write are
  swallowed (page closed/reloaded mid-request); servers bind with `localguard.bind` (no address reuse on Windows, so
  a second copy fails instead of sharing the port).

---

## 5b. Cameras tab: every device's live camera (v1.19-v1.20, 2026-11-12)

With a shared database each device runs a **peer service** (`peer.py`, db/007-008), the only server that listens on
the LAN: port = the tab page's port + 3 (8767; `FACE_PEER_PORT`, `FACE_PEER=off`; Settings → Shared database → Share my
photos with my other devices, on by default there). TLS with the device's own certificate (`HOME/peer-cert.pem`), whose
SHA-256 is in `devices.peer_cert_sha256` and checked by the asking device; every request HMAC-signed with the cluster
key (HKDF of the database password and its instance id), valid `CLOCK_SKEW` 120 s; photos by content hash only
(`PHOTO_RE`), from the device's own records. On Windows the app must be allowed through the firewall for private
networks. Started by `HomePart._peer`.

- **Streaming** (`camstream.py`): the tracker page composes the video with its overlay canvas (every marker, mirrored
  as shown) into JPEG frames (`streamFrame`, all the time since v1.19.2, 20 a second since v1.21.2, the owner's choices; only while
  Stream this camera is on and the peer service runs) and sends who's in view and its readout as data every second
  (`/api/inview`). Frames stay in memory. The peer service serves `/stream` (multipart: JPEG parts with a JSON readout
  part every `READOUT_EVERY` 1 s between them, since v1.20; a stream lasts until the watcher closes it) and `/readout`.
- **Watching** (`cameras_ui.html`/`.js`, the tracker's `/cameras` page, the app's Cameras tab): every streaming device
  in a grid, each with its readout drawn with text only; double-click for one alone; one events list from every
  camera (a fly-out that can be closed, remembered, its button counting new events, v1.19.8). `camrelay.py` keeps one
  connection per watched device (`peer.open_stream`, never ending on a timer, reconnecting at once) with each device's
  newest frame and readout. The page makes one `/api/cameras/frames?after=id:n,...` request at a time, answered with
  every camera's new frames at once (`tracker.camera_frames`: waits on `camstream.ANY` up to 3 s for the first, then
  gathers 40 ms more; a length-prefixed JSON head, then the JPEGs), and one `/api/cameras/readouts?ids=...` a second,
  answered from memory. Live or offline is judged by frames actually arriving; an offline camera stays in the grid
  marked offline (since when) and comes back by itself; a camera whose newest frame is over 5 s old shows "paused"
  instead of a frozen picture.
- **"Moved" alerts** (`HomePart._cameras`, db/008 `sightings`): someone on this camera who was on another device's
  camera within `MOVE_WINDOW` 600 s raises "moved" (only the device that sees them now raises it); the alert reaches
  every device. Since 1.20.1 sightings are written only for people on the device's alert list while the alert is on,
  and deleted after `KEEP_SIGHTINGS` (20 min); before, every named face was noted whenever a database was set up.
- **Keep this camera running** (v1.19.2, on by default): the camera starts with the app and again within 10 s when it
  stops or its track ends (unplugged, taken by another program); Stop camera turns the option off.
- Measured: 9.7 frames a second at the source for a minute with no gap over half a second (v1.19.4); 40 cameras (a
  stub) at 9-10 fps each in headless Edge with never more than 3 requests open (v1.20). A 1.19 device's readout doesn't
  show on a 1.20 Cameras tab (the readout moved into the stream): update every device.

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
- **Frames vote** (then tracker_ui.html; `expression.js` since 2026-10-30): last 10 scored frames (~1 s), weighted by quality; report a label
  only if there are at least 3 votes (`X_MIN_VOTES`), it has ≥ 45% of the vote (`X_SURE`) and leads the
  runner-up by 12 points (`X_MARGIN`); otherwise shown dimmed with "?". After 10 unscored frames in a row
  the reading is dropped rather than left stale. 95% of stored faces give a clear winner.
- **Calibrate to each person** (on by default): slow running average of each named person's readings,
  kept in the browser (localStorage key `ft-usual`; since v1.16 also `HOME/person_signals.json`); trusted after ~100 frames. Readings that match the
  usual look are tagged **usual** (dim); departures show bright. **Forget learned usual looks** clears it.
  Functions: `learnUsual`, `readExpression`.
- Reliable labels: happy and neutral (surprised was right about half the time on the one posed session). Fearful/disgust are rare in training data and most
  confused; angry/sad are often just a resting face. Photo-library distribution: ~40–45% neutral,
  ~40–45% happy, rest thin.
- Expression readings are stored since 2026-11-12 (the owner's decision, for analytical reports later): with a
  shared database, every 5 s per named person in view, in `expression_log` (`exprlog.py`, db/010; a switch turns
  it off, nothing without a database); and in each alert's details. Before that: never stored. The model is optional (the tracker says so at startup if
  missing). UI toggles: **Show expression**, **Calibrate to each person**.
- `python tools\expr_check.py` scores a sample of stored faces (ms/face, label spread, mirror agreement)
  and runs one tracker request on a stored photo — confirms the model is installed and working.
- Status at end of session: the owner confirmed "It is working now!" and the collapsible Known-people card
  "worked!!".

---

## 7. Training data and history

Current data (2026-10-29):
- **5,702 photos** read from the owner's iCloud Photos folder, of which 112 were skipped as copies.
- **1,551 faces** in `faces.npz`; **786 labeled**: 456 by the owner, 328 named automatically, 231 of them
  `#ignore`.
- **26 named people**; `people.npz` holds 544 face prints.

History:
- The first scan read **1,936 photos** → **638 faces** after fixing HEIC decoding and adding
  close-up/selfie handling. That listing truncated at 2,000 entries, so ~60 photos were missed at first.
- Faces were clustered; the owner named **14 people** in the first pass, and more later in the labeling tool.
- Groups that were artwork, video-game characters, tattoos, sculptures, or ambiguous images were ignored
  at the owner's request.
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

Releases (FaceRecognizer.exe, then also the macOS and Linux apps; GitHub Releases; `models-v1` holds the model files for
first-start downloads; only the latest app release is kept):

| Version | Date (2026) | What changed |
|---|---|---|
| v1.0-v1.1 | Nov 8 | First releases of FaceRecognizer.exe: both apps in one window (`app.py`, pywebview) |
| v1.2 | Nov 8 | Video import (people followed, 3 clearest faces), moved photos found again (`relocate`) |
| v1.3 | Nov 8 | Import progress always visible, face mesh drawn in shade groups |
| v1.4 | Nov 9 | Possible faces (low detector cut-off), Find more faces, Draw a face |
| v1.5 | Nov 9 | Describe photo (LM Studio; built-in Florence-2 fallback), stacks of the same picture |
| v1.5.1 | Nov 9 | Models fetched from their publishers (OpenCV Zoo, MediaPipe, InsightFace), `models-v1` as fallback |
| v1.6 | Nov 9 | All of `data/` taken out of the repository and its history (repo restarted as one commit); Amazon keys enterable in the app; "No photos yet" welcome |
| v1.8 | Nov 9 | Settings tab (every option in sections, `settings_bridge.js`), all tracker options remembered, alert list with Unknown, Describe photo server/model/API key settings (any OpenAI-style server, other devices with a warning); security fixes from the review; Python tests, CI with ruff and pytest, `labels.py` split, **Why?** for unreadable files. v1.6 release removed |
| v1.9 | Nov 10 | Released 2026-11-10 for Windows, macOS (Apple Silicon, macOS 14+, not yet Apple-signed) and Linux (Ubuntu 22.04+, tested on 26.04), from CI's builds of 587b515: second round of review fixes. Pages load nothing from the internet (system fonts instead of Google Fonts) under a strict Content-Security-Policy; CORP, cross-site requests refused; tools trust the tab page only when the app started them; camera only for the app's tracker; request size limits; Collect faces saves each face alone; Ignore deletes the print and picture; one person's export and forget (**Their data…**); model files checked by SHA-256 (cached); pinned downloads in the tools; CI actions pinned to SHAs; Describe all no longer stops at a failing photo after the first four; alerts: an inbox (`alerts.py`, New / Acknowledged / Archived, the newest 500) with a bell on the tracker page and in the app's tab bar, a banner over any tab, the count in the window title and a taskbar flash; event types (arrive, leave, unusual for them, unknown face) and every timing as settings; each alert keeps what was measured then and again later, including the evidence lines (numbers and text only) |
| v1.10 | Nov 10 | Downloaded Linux app ships plain ONNX Runtime; Download CUDA support in Settings (`cudasupport.py`); GPU face finding (YuNet with free input sizes) and parallel import workers (§4) |
| v1.11-v1.13 | Nov 11 | LM Studio on every computer (all interfaces, announced, models from a shared folder), the download bar, `docs/SETTINGS.md` (1.11); Describe all shared with the other LM Studio computers, Settings regrouped (1.12); HTTPS checked by the operating system, `truststore` (1.13) |
| v1.14-v1.15 | Nov 11 | Start again button; Live readout as fixed labeled cells; Full body and Hand skeletons (1.14); window graphics caches cleared once per version, warning for a flat camera picture (1.15) |
| v1.16 | Nov 11 | Optional shared database (PostgreSQL + pgvector) and sync between devices, keys encrypted under a passphrase, each device's settings and every person's signals kept by the page's server, every stored time in UTC (§4b). 1.16.1-1.16.5: failed `people.npz` read retried; faces no longer re-sent after each restart (float32 digests, picture SHA-256); saved keys shown masked with an eye button; Settings forms with captions; faceness model trained in the background |
| v1.17 | Nov 11 | Passphrase versions sealed to every device (X25519): set, change or restore once, every device follows |
| v1.18 | Nov 12 | Original photos from another device over the peer service (port 8767, TLS, signed). 1.18.1: faceness model with liblinear (L-BFGS held the GIL for minutes on Linux) |
| v1.19 | Nov 12 | Cameras tab: every device's live camera with its readout, "moved between cameras" alerts (§5b). 1.19.1-1.19.9: streaming while minimized or covered, Keep this camera running, renderer-exit recovery, streams that never end on a timer (`camrelay.py`), heartbeat on its own thread, no 30 s pauses, UTC-stamped log, window watchdog (`/alive`), events fly-out, no false "offline", a hung camera start retried, then the page reloaded, then the app restarted (§9) |
| v1.20 | Nov 12 | Cameras tab: one request for every camera's frames and one for every readout (the browser's six connections per address); the readout rides along each peer stream. Released 2026-11-12 for Windows, macOS and Linux from CI's builds of 6586b77; older app releases deleted (only v1.20.0 and `models-v1` remain) |
| v1.21 | Nov 12 | From the full review: the sync takes changes by transaction (db/009), a forgotten name given again is no longer forgotten, person signals are sent per person, the LM Studio shared folder can't write outside LM Studio's folders; the expression log in the shared database (`exprlog.py`, db/010); "They move between cameras" off by default; Connect needs channel binding; peer and relay timeouts. Built and installed on the three devices for testing on 2026-11-12; not released yet. |
| v1.21.1 | Nov 12 | The Linux window's freeze found and fixed: answers the page scripts never read (`/alive` every 2 s, `/api/inview` every second) each kept a 2 MiB shared-memory descriptor open in the page's renderer, which froze at its limit (1024 after a fresh start: ten minutes; 8192 after the watchdog's restart in place: every 90 minutes). Every fetch now reads or drops its answer (a test checks), and the app raises its open-files limit to 8192. On Windows the same handles grew without a freeze. Not released yet. |
| v1.21.2 | Nov 12 | Cameras: 20 frames a second instead of 10 (`STREAM_FPS`; the owner's choice), paced by when each frame is due, and drawn on a timer whenever the display hasn't drawn for two frames (a window in a disconnected desktop session sent only 4 a second). 1.21.3: the names and boxes are drawn at most 60 times a second (`DRAW_EVERY`): on a 240 Hz display drawing at its rate left no time to encode stream frames (93 ms a frame instead of 12; 13 frames a second instead of 20). 1.21.4: on Windows camera frames stay in ordinary memory (`MediaFoundationD3D11VideoCapture` off in `KEEP_RUNNING_FLAGS`): in the NAS's disconnected desktop session one read-back from the graphics processor in ten took 165 ms (13 frames a second; 20 without). Not released yet. |
| v1.22 | Nov 13 | Released for Windows, macOS and Linux: everything in 1.21 to 1.21.4 (built and run on the three devices first), and more from the review: Start again never loads a second copy of a program; the sync follows a change of the connection settings; the window watchdog stops when there is no window; a person left in no label can still be exported and forgotten; the server-side forget is repeated daily and clears photo-tag names; LM Studio model names and model lists from the network are checked; overlapping camera and microphone starts; a merge leaves no face that is and isn't the same person; copies of copies; client disconnects and OpenCV's per-detector warning no longer fill the logs. |
| v1.22.1 | Nov 13 | **Fully distributed system** (Settings → LM Studio, off for a new copy; the owner's wish): on, the LM Studio search covers every network adapter and Describe all may use servers beyond the local network. Off, only real adapters count, now on Windows (asked from PowerShell on a thread of its own) and macOS too, not just Linux. Linux: numpy's maths (OpenBLAS) kept to one thread, set in `app.py` before numpy loads: in the packed app a matrix product in the labeling tool got stuck for good in 2 starts of 4, freezing the Labeling tab and the sync (6 of 6 clean with one thread). |
| v1.22.2 | Nov 13 | Released for Windows, macOS and Linux (run on the three devices first). The passphrase for keys is no longer sealed to every row in `devices`: a device gets it typed, or with a one-time code typed on a device that has it (db/011: `key_proof`, version `link`, `key_requests`); a device that typed it reads the shared keys at once; restore only of versions the device held. The LM Studio restart passes `--cors` only when LM Studio's own settings have it on. The three servers keep connections open (HTTP/1.1): on Windows one request in a thousand had failed on a new connection each time (§10). |
| v1.22.3 | Nov 14 | Released for Windows, macOS and Linux, with installers for the first time (the Windows and Linux ones run on the three devices first; the macOS .dmg only built, as there is no Mac to try it on). Five bugs from §10's list: an Ignored face that is named, or taken off the Ignored list, gets its print and picture made again from its photo (and faces left without them by older versions are mended at start); a moved video is found again like a photo; a Cameras page left open in a browser shows frames again after its tracker restarts; Ctrl+1 to Ctrl+4 work from inside a tab; questions are asked in the page (`ask.js`) instead of with `confirm()`, which stopped the camera and made the watchdog restart the app. Also (the owner's wish, 2026-11-14): the app's own icon (`icons/`, drawn by `tools/make_icon.py`: a faceted face, half solid and half mesh, in a camera's corner marks) on the .exe, the .app, the Linux window and each page's browser tab, and installers that put the app in place with icons, none needing administrator rights (`build_exe.py --installer`): `FaceRecognizer-Setup-x64.exe` (Inno Setup; Start menu and desktop icons; into `%LOCALAPPDATA%\Programs\FaceRecognizer` or a folder the user picks, as the app writes `models\` and `data\` beside itself), `FaceRecognizer-macos-arm64.dmg` (drag to Applications) and `FaceRecognizer-linux-x64.run` (installs itself for the user, with a menu entry and a desktop icon). No installer ever deletes `models` or `data`. The portable .zip and .tar.gz stay beside them. And from the NAS's camera gone for six hours that day (§10): **Keep this camera running** counts a camera as running only when its picture arrives, escalates starts that are refused as well as ones that hang, and says in `app.log` what it did (`/api/page-note`); `app.log` gets the memory of the window's processes every half hour (`note_memory`). |
| v1.22.4 | Nov 14 | Released for Windows, macOS and Linux; the first release whose macOS app and .dmg are signed with a Developer ID and notarized by Apple (`tools/apple_signing.py`, CI's signing steps and the built app's `--self-test` came in after 1.22.3). For the Mac, found by reading the code before there was one to try it on: the app window never answered WebKit's question about the camera, so macOS would have asked "Allow 127.0.0.1 to use your camera?" at every page load, with nobody there to answer after an unattended restart (`app.allow_camera_macos`: the window's delegate grants the tracker this window started, `camera_answer`; any other page is still asked); and the app's `Info.plist` says why it reaches the local network (macOS 15 and later ask first; until allowed, the shared database and the other devices can't be reached). Still not run on a real Mac: the window, the camera, a hidden window's frame rate and CoreML are to be measured on the first one. |
| v1.22.5 | Nov 14 | From the network review of that day (§10), built and run on the three devices first. The service the devices reach each other through stands many requests at one moment on Windows (accept queue 128: 16 at once took 1.04 s, now about 20 ms). A request to another device, and the Cameras page's list of devices, no longer open a database connection each (`facedb.ask`: 130 ms down to 13 to 32 ms, and 109 ms to 3 ms). A device's addresses are tried best first, each for 3 s instead of 8, and a device no longer tells the others its container bridges. The log says when another device's clock is more than a minute from this one's, before requests are refused at two minutes, and a refusal for the time says how far apart the clocks are (one device's clock was found 81 s slow). Also from that day, without a release of their own: the library versions (psycopg 3.3.9, mediapipe 1.2.3, boto3 1.44.3), CI's actions at v7. |
| v1.23.0 | Nov 15 | **Music and room sound as one more signal** (the owner's wish; Settings → Music; off until switched on, for a named person, with a shared database). `music.py`: the song the Spotify app shows as playing (its window title, nothing asked of the internet); every audio interface of the computer listened to at once for 8 s every half minute, each by itself: outputs (what the computer plays) and microphones (the room), through `soundcard`; a song's numbers (level, dynamics, tempo, brightness, bass, key and mode); what a room sounds like, from YAMNet's 521 kinds of sound gathered into plain families (quiet, music, speech, television or radio, steady background noise, activity, alerts, animals), and a steady background's own numbers and word (floor, events, steadiness, rumble, hiss, hum). `musiclog.py`: each song is a row of `music_log` (db/012) when it ends, with what each interface heard; music heard with no player naming it is "music in the room"; only numbers and words, never sound. A person's usual music (`music.picture`, from their Spotify liked songs: `spotify.py`, their own key, read-only, PKCE) is a person signal of kind `music`, shared, exported and forgotten with them. Built and run on the three devices first; the first row reached the database from the PC (a song of 234 s, heard on a display's speakers as music, 0.98). The words and cut-offs are not measured against anything: evidence. Found on the devices before the release build: about 20 s after a Windows screen is locked the camera sends no new picture, and its last one was streamed as live (for an hour that night, and since 1.22 after every Remote Desktop disconnect, which locks the session too: once for 21 hours); such a camera now counts as paused (`camPaused`: no faces read, the other devices told, the log). Under a lock (`desktop.screen_locked`) the page stops and starts it at once, and when that start fails or doesn't finish the app starts itself again, once per lock (`app.restart_soon`): in a session locked by a disconnect the running app never got the camera back, and the app started again had it within seconds; locked at the computer's own screen nothing helps until the unlock, and nothing more is loaded or started. Also found and fixed before the release build: why the Windows app's page grew by 1.5 to 2 GB an hour (pywebview's request handlers switched on the browser's network inspection, which kept a record of every request; `app.drop_request_hooks`, section 10). |
| v1.24.0 | Nov 16 | Released for Windows, macOS and Linux (installed on four devices first). **Describe everything again** (the owner's wish; Settings → Describe photo): the whole library described anew with the describer set, every photo and every video frame a face was found in, also what only another device has (shown by that device, `peer.fetch`); asked about first (how many, which model, which computers); each description replaced as its new one arrives; shared out like Describe all (Share Describe all, and beyond the local network with Fully distributed system); a stopped or incomplete run stays under Unfinished to carry on with (`describe.again()` in `describe.json`); Stop and progress in Settings too. Settings buttons can ask a question first (`confirm`). Two bugs a newly installed fourth device showed: a device with an empty library could not join the shared database (`Store.save` with no faces), and on a new Windows profile the window never answered the tracker's camera request (WebView2 names the tab page as the one asking; `camera_allowed`, `Permissions-Policy`, the answer in place before any page loads). §10 has the first run's numbers and what is open. |
| v1.24.1 | Nov 17 | Released for Windows, macOS and Linux (installed on four devices first; the Windows build's window opened on all five of its first starts as a new copy). The two things left open by 1.24.0's first run (§10): a describing helper that failed twice in a row is tried again every five minutes with one photo and rejoins when its server works (`label_tool.HELPER_RETRY`; it used to stay out for the whole run); and the built app's `--self-test` loads the Windows window's own parts (pywebview's Windows platform, WebView2's library, the types the camera answer uses), with `pythonnet`, `clr_loader` and `pyinstaller-hooks-contrib` pinned like the rest. |
| v1.24.2 | Nov 17 | Released for Windows, macOS and Linux (installed on four devices first: data and models untouched, every camera live on every device; the Windows build's window opened as a new copy and answered the camera request). The security fixes from the code review of 2026-11-16 (§10): a row in the shared database can no longer give a photo a file path on a device (`photos.extra` was copied into the photo's record whole, and the peer service showed the file at a photo's path to every device that asked); a device's settings are its own and never taken from the database (a changed row switched on a device's microphone listening or camera stream, or pointed its describer at another server), only the owner's Copy settings takes another device's, and never the settings that say where a device sends photos or fetches files; a row that can't be used is left out and named in the log instead of stopping every device's pull for good (`Sync._apply`, each part's `check`); Share Describe all sends photos only to the owner's own devices (computers in the shared database, at the addresses they gave), not to whatever answers like LM Studio on the network; no answer is kept by a browser any more (face pictures and photos were cacheable for a year: the labeling page keeps what it showed in memory instead). Smaller: at most 64 camera relays, a download is stopped once it is larger than its file, two inputs that raised. |
| v1.24.3 | Nov 17 | Released (installed on four devices first; before publishing, the repository, every file inside the three builds, the build's logs and the notes were searched for the real keys, passwords and tokens of the computers it was made on: none). The smaller security items of the code review (§10): the app shows and gives the camera only to a program it started itself; data and anything a GET starts go only to a tool's own page, not to a page of another program on the computer; a passphrase link says which version it replaces (an old one fitted again after a restore); a database login without the server's authority is bound to its connection; the built-in describer's files are checked by SHA-256 when found already there; the peer service's private key is kept encrypted; nobody can be named `constructor` or `__proto__`. |
| v1.24.4 | Nov 18 | Released (installed on four devices first, the three Windows ones with the signed installer over the unsigned 1.24.3; the repository, every file inside the three builds, the build's logs and the notes were searched for the real keys, passwords and tokens as for 1.24.3: none). The first release with code-signed Windows files (the .exe, the installer and its uninstaller, signed by CI through Azure Artifact Signing; §10). The labeling tool answers about ten times faster after a change on a large library (16,000 faces: 1.3 s to 0.13 s after a name): scores and stacks are kept and brought up to date where a change touches them, with the same answers; the sync hashes outside the labeling lock; the large JSON files are written one line per entry. |
| v1.24.5 | Nov 18 | Released (installed on four devices first; the repository, every file inside the three builds, the build's logs and the notes searched for the real keys, passwords and tokens: none). Structure and the speed leftovers from the code review (§10), and two faults found on the way. `label_tool.py` split (the Describe all job and the Settings routes in modules of their own), the forgotten names in `forgotten.py`, one class for the logs that wait to be sent (`waitlog.py`), one class per synced table in `dbsync_home.py`, two tripwire tests (page routes, synced tables). Fixed: after a person was forgotten, the expression and music readings of everyone else from later that day could miss the database; a synced table without its own branch would have had its rows' keys forgotten as people. Faster: Settings open no longer starts `lms ls` and a database connection every 5 s, people tags and the regrouping after an Ignore. |
| v1.24.6 | Nov 19 | Released for Windows, macOS and Linux (00:51 UTC; installed with its installers on five devices first, a Mac among them for the first time: models untouched, every device online with the same 19,683 faces, every camera live on every device at 17 to 20 frames a second; the Windows build started twice as a new copy, its window opening and answering the camera request, none of 2,488 requests refused; the repository, the three builds, the build's log and the notes searched for keys and passwords). The Linux app's own process no longer grows while people are in view (§10): it went to 6 to 8 GB over a day with 1.6 GB in use, because glibc kept freed memory in up to 128 arenas, one per working thread, and every label change made blocks of 9 to 70 MB for a moment in whichever arena was at hand. `paths.give_memory_back` sets glibc at start (8 arenas, a block of 8 MB or more straight from the system and back when freed, at most 16 MB kept free at an arena's top): test copies side by side for 20 minutes 0.67 to 2.18 GB before, 0.71 to 1.25 GB and level after, the same frames at the same 124 ms each; the real app 1.86 to 1.94 GB over 209 collected faces where 1.24.5 had gone from 3.1 to 4.2 GB over 244. Nothing changes on Windows and macOS. Still to read: a full day of its memory lines (its first 5.5 hours: 1.75 to 2.0 GB, level). Added after the commit, found in the four devices' logs of 1.24.5's first day (the sync through the new table classes: nothing left out, no errors): a face's picture asked for while an import or the sync added a face failed with numpy's "Content of sequences changed" (13 times in two days on two devices, since 1.24.1 at least: `LabelDB.E` read the list of prints without the lock; it and `printless` now make their tables under it, with a test that fails without); and a device never asks another at its own address and port (`peer._order`, `peer.own`: a VPN client gave two devices the same address, so each reached its own service there and, whenever the other was off, the log blamed that device's certificate). The first real Mac (a Mac Studio, the fifth device, 2026-11-18, with the released 1.24.5 .dmg): macOS takes the .dmg and the app as "Notarized Developer ID", the self-test passes, the window opens, the camera runs (20 frames a second, every picture different), it joined the shared database in 19 s (19,259 faces) and every device watches every other's camera; ArcFace on CoreML differs from the CPU by 5.7% there, so recognition runs on the CPU (quality on CoreML). What it showed, fixed here: a device's name is the computer's without its domain (macOS says "name.local", `facedb.device`), and a usual AI port counts as that app only when a model server answers there (macOS's AirPlay Receiver has 5000 and answers 403: Settings showed text-generation-webui as running; `hardware.ai_apps`, `describe.Unavailable.code`); and every start listed the system's fonts again, over 5 s on the Mac ("Matplotlib is building the font cache" in the log: MediaPipe imports matplotlib, and PyInstaller's start-up hook gives the built app a new temporary folder for it each time; `app.keep_font_list` keeps it in the private folder; on the Mac the second start of 1.24.6 served after 6 s where 1.24.5 took about 14). Also there since that day: LM Studio with the two Qwen vision models (2.5 s a picture with Qwen 3.8 27B, the fastest of the five), announced by the app and found by the others. Still open from it: the stream's rate with the window hidden or minimized, a locked screen and display sleep; an app copied into /Applications with its download mark runs from a temporary copy (App Translocation) until the mark is gone, which a drag in Finder does. |
| v1.24.7 | Nov 19 | Released for Windows, macOS and Linux (installed with its installers on five devices first; the Windows build started twice as a new copy; the repository, the three builds, the build's log and the notes searched for keys and passwords). macOS: with another window over the app's, the camera reached the other devices at 1 frame a second (the owner saw it on the Mac the day it joined, working in another app): WebKit slows a covered window's page (timers once a second, nothing drawn), and the Chromium flags that stop this on Windows and Linux mean nothing to it. `app.keep_running_macos` switches WebKit's own slowing off on the window's web view. Measured on that Mac in a window with a counting page (a 25 ms timer, drawn frames a second): covered 1.0 and 0 before, 35.8 and 60 with the occlusion check off, as when shown; hidden or minimized 1.3 and 0 before, 13.5 to 15.7 and 0 with the three page switches off (36 when shown), so a minimized or hidden window on a Mac still streams slower. In the app itself on that Mac, with the same other app in front of it: 1.1 frames a second with 1.24.6, 19.9 with 1.24.7 (at its own tracker), and its log says the four switches were set. |

---

## 8. The owner's stated preferences and decisions (keep honoring these)

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
  Every setting is described in `docs/SETTINGS.md` (2026-11-11).
- The hardware decides (2026-11-10): the app identifies CPU/GPU/NPU and picks the fastest way that gives the same
  answers, per job; the docs say so throughout.
- Big downloads only when asked (2026-11-10): CUDA for the Linux app and LM Studio models are offered in Settings,
  never fetched by themselves; every download shows a progress bar (size, speed, time left).
- LM Studio on every computer with this app (2026-11-10): serves on all network interfaces, port 1234; announced on
  the local network with its real port (discovery reads the port from announcements); the same vision models
  everywhere, copied from a shared folder on the NAS and, if that copy fails in any way, downloaded from the internet. Settings shows a summary of every LM Studio
  server found: names, IPs, ports, models.
- One shared database for all the owner's devices (2026-11-11, §4b): private data shared, settings per device, keys
  only encrypted under a passphrase that stays on each device; optional, and a new copy keeps working from files.
- Every time stored in UTC, shown in local time only on the pages (2026-11-11).
- Saved keys and the database password shown masked in Settings, with an eye button that shows one only while held
  (2026-11-11).
- Every camera runs and streams all the time at its full rate (20 frames a second since v1.21.2), watched or not, minimized or not
  (2026-11-12); the app window must never stay frozen (a watchdog brings it back, §9).

---

## 9. Environment gotchas (learned the hard way)

Python on this machine:
- The `python` on PATH is 3.15 (`C:\Python315`) and has **no** OpenCV/onnxruntime. The project's libraries
  are in Python 3.12 (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`), which the .bat files prefer
  because other apps add newer Pythons that `py -3` would pick. Use that interpreter for CLI/tools.

Webcam / OpenCV on this Windows machine:
- `cv2.CAP_DSHOW` alone didn't reliably open the USB webcam; iterate `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`.
- Retry `cap.read()` up to ~10 times with short sleeps; some webcams return empty frames while starting.
- Camera index 1 = a phone's camera (through Phone Link), not the USB webcam. Pick cameras by name in the browser UI.

- **One GPU run at a time per process** (`face_id.ACCEL_LOCK`). FaceRecognizer.exe hosts the tracker and the
  labeling tool's webcam import in one process; with Collect faces on, both ran DirectML at once and the AMD
  driver crashed the app (access violation in `amdxc64.dll`, Windows Application log event 1000, 2026-11-08).
  Reproduced with two threads embedding at once (segfault, exit 139); with the lock: 8,192 runs, no crash, and
  the app ran 4+ min while importing 32 snapshots during live tracking. Loading models at the same moment
  (both programs start together) crashed onnxruntime too; loading is serialized by the same lock.

The app window (WebView2 on Windows, Qt WebEngine on Linux, WebKit on macOS), streaming and sync (v1.15-v1.20):
- **Stale graphics caches** from an older build froze a newer Qt WebEngine (white window, the renderer never drew):
  `app.clear_graphics_caches` removes the window's compiled graphics caches once per new version, never Local Storage
  (v1.15).
- **Background throttling:** minimized or covered, Chromium slows timers to once a second and stops drawing, so the
  tracker's boxes froze and other devices saw a still picture. The window's browser starts with
  `app.KEEP_RUNNING_FLAGS` (`--disable-background-timer-throttling`, `--disable-renderer-backgrounding`,
  `--disable-backgrounding-occluded-windows`, `CalculateNativeWinOcclusion` and `IntensiveWakeUpThrottling` off,
  through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` / `QTWEBENGINE_CHROMIUM_FLAGS`), and while its page is hidden the
  tracker draws and streams on a timer (v1.19.1). Test copies in a normal browser are still throttled.
- **A dead page renderer** left the window black with nothing in the log (Linux): `app.recover_renderer` catches Qt's
  `renderProcessTerminated` and WebView2's `ProcessFailed`, logs the reason and reloads the page (v1.19.3). Only a
  page's own renderer triggers a reload (v1.19.6): the GPU process and utility processes (video capture, audio,
  network) restart by themselves, and a reload would restart every camera.
- **A live renderer that stops running scripts:** on the Linux laptop the page renderer went idle for five hours
  (camera, stream and picture stopped) while every process lived on, so nothing above fired. The tab page calls
  `/alive` every 2 s; `app.watch_window` checks every 5 s: after `HUNG_AFTER` 30 s of silence it kills the page's
  `QtWebEngineProcess --type=renderer` children (Linux, macOS: Qt then reloads the page), and if the page is still
  silent `RESTART_AFTER` 30 s later (on Windows, where nothing is killed, after the same 60 s) the app starts itself
  again (`restart_app`). A gap of over 20 s between checks means the computer slept and resets the count (v1.19.7).
  Since 1.20.1: restarts in a row wait 2, 4, 8 ... min (at most an hour), so a page that always hangs can't restart
  the app every minute; and on Windows, where `os.execve` only starts a new process and ends the old one (the new
  copy could find the ports still taken and move to others, losing the tab page's storage), the new copy is started
  with `FACE_RESTART_AFTER=<old pid>` and waits for that process to end (`wait_for_previous`; measured: ports taken
  again 0.5 s after).
- **Restarting a PyInstaller build in place:** `restart_app` sets `PYINSTALLER_RESET_ENVIRONMENT=1`, so the re-executed
  bootloader starts as a fresh top-level instance instead of taking itself for the old process's child.
- **The camera start that never returns:** once after a restart on Linux the window's camera (video capture) service
  didn't start and `getUserMedia` hung for good, which Keep this camera running waited on forever. A start over 30 s
  now counts as failed; after two the tracker page reloads, and if it still hangs the page asks the window
  (`fs: "camstuck"` → `/restart-app`) to restart the app, only in the app window and at most once an hour
  (`RESTART_EVERY`, `HOME/restarted.json`) (v1.19.9).
- **Six connections per address:** a browser opens at most six HTTP/1.1 connections to one host:port. Each camera
  was a never-ending multipart `<img>` that ended after 120 s and reconnected, and a reconnecting stream waited behind
  the others (the picture froze for seconds every ~2 minutes, v1.19.4); later one request per camera for frames plus
  one per camera for readouts queued with more than a few cameras, and the tracker page's own requests on the same
  address waited behind them. Since v1.20 the Cameras page's requests don't grow with the number of cameras (§5b).
- **A frame that couldn't be handed over** paused that camera's stream for 30 s; it's retried in half a second now,
  each POST times out after 5 s, and why it failed is logged with the next frame (v1.19.5-v1.19.6). The app log stamps
  every line with the time in UTC and notes any frame gap over 2 s and why a relay dropped.
- **CSS `display` beats `[hidden]`:** the Cameras page's overlay (`display: flex`) and tiles (`display: grid`) stayed
  visible with the `hidden` attribute set, so "offline" stayed over a live picture; `cameras_ui.html` has
  `[hidden] { display: none !important; }` (v1.19.9).
- **Python's lock (GIL) and native libraries:** a long SciPy L-BFGS fit with OpenBLAS threads held the GIL for
  minutes and froze every server thread and the window; liblinear lets the other threads run (§4b). Long work that
  shares a lock with request threads goes to a background thread on a copy.

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

Following the review in `Real-Time Facial Emotion Recognition.md`, with the owner's decisions:
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

Background and research: [docs/WHY_LAYERED_EXPRESSION.md](docs/WHY_LAYERED_EXPRESSION.md). The owner asked to
implement every suggestion from it, including speech (camera + microphone + a voice-emotion model), and to
fit valence/arousal from their own data rather than download a face valence/arousal model.
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

The owner asked whether skeletal posture indicates facial emotion. Answer: not the *facial* expression, but
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

The owner: the posture answers are obvious one by one; what matters is reading them in the context of the
individual, and of interactions between identified people. Step 1 (done): per-person baselines.
- `signals.js` `Baselines`: for each named person, a running mean and spread of 17 signals (`B_KEYS`:
  head, neck, tilt, body/head motion, smile, jaw, blinks, gaze, pitch, pitch SD, loudness, speech rate,
  voice arousal/valence, pulse, breathing), one sample a second, trusted after 120 samples (2 min), then
  a slow running average (`B_RATE` 1/600, ~10 min). Kept per context: `alone` (only face in view) and
  `company`, falling back to the overall normal until the context's own is trusted. Spread floors per
  signal so near-constant signals don't produce huge z values.
- `FaceSignals.snapshot()` gives the current values; `unusualEvidence()` shows |z| >= 2 ("unusual for
  Alex (alone): head -2.3 SD"). Controlled by "Calibrate to each person"; stored in localStorage
  `ft-baselines`, since v1.16 also by the tracker in `HOME/person_signals.json` and, with a shared database, its `person_signals`
  table (never in `data/`). "Forget what's learned" works for everyone or
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

Done in 1.20.1: the Web search keys and the describe server's API key are protected like the database password
(`paths.protect`: DPAPI on Windows, mode 600 elsewhere; `google_vision_key.txt`, the secret in
`aws_rekognition.json`, the key in `describe.json`); plain ones from older versions are read and re-saved
protected (`paths.reveal`). In the shared database the keys exist only encrypted (`secretbox.py`).
Done: the PyInstaller spec file is no longer committed (`build_exe.py` writes it under `build/`, `--specpath`).
(Refusing `Sec-Fetch-Site: cross-site` was done in v1.9, see below.)

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
the database through a closure) and has kept growing (about 570 lines by v1.20, with the peer-photo and Describe all
helpers); splitting it into a class with route methods is the next step.

Open after the 2026-11-12 review and v1.22 (not fixed yet):
- With Share Describe all on, the search for LM Studio servers also runs when a photo is opened, not only while
  Settings is open. (Which adapters it covers is the Fully distributed system setting since v1.22.1.)
- Passphrase for keys (fixed in v1.22.2, see Releases): still open there: a device on an app before 1.22.2 seals
  the passphrase to any row in `devices`; someone with the database password can make another version "current",
  which stops keys syncing until the owner sets it right (no button for that yet); a device waiting for the
  passphrase is shown only in Settings, not as an alert.
- Fixed in v1.22.2 (measured 2026-11-13): on Windows about one request in a thousand from a page to its own server
  failed (app.log "Camera: a frame from the tracker page failed (TypeError: Failed to fetch)", 1,145 in a 12-hour
  run of 1.22.1; about five a minute counting `/api/recognize` and `/api/cameras/frames`). Each one was seen: the
  stream to other devices paused half a second, the Cameras tab one second, and the tracker flashed "Lost the
  recognizer". Cause: the servers spoke HTTP/1.0, so every request (about 90 a second) was a new connection; the
  browser and the server close a finished connection at the same moment, Windows keeps the closed pair of ports
  (6,000 to 7,400 in TimeWait for port 8766), hands new connections random local ports out of 16,384, and a new
  connection that gets such a port again is not answered: the browser's own log shows error 10060 after 1.0 s.
  Reproduced away from the app (headless Edge against a plain Python server: 2 of 2,069; a Python client that
  closes as a browser does: 30 time-outs and 587 "address in use" of 14,433; never when the server closes first,
  nor with connections kept open). Not the accept queue (128 instead of 5: still 15 of 16,075). With HTTP/1.1 and
  connections kept open (`localguard.Handler`): 26 connections for 26,436 requests in 5 minutes, none failed,
  nothing in TimeWait. Linux was not affected by the failures, but kept-open connections showed a second thing
  there: an answer written as headers and then body waited about 40 ms for the page's acknowledgement (41 ms a
  request, the stream at 15 frames a second); answers now leave in one piece with TCP_NODELAY (0.1 ms).
- Fixed in v1.22.3 (2026-11-14; see Releases), five from this list:
  - Naming an Ignored face, or taking it off the Ignored list, left it without print or picture. `LabelDB.act` now
    makes both again from the photo (`remake`: `rebuild_crop`, then the models; checked with the real models: the
    new print's cosine to the deleted one was 0.977, 0.07 s). A face whose photo isn't here, or isn't found in it
    again, stays as it is and the page says so; Undo deletes both again. Faces already left like that are listed at
    the labeling tool's start, before the sync starts (while it runs, a face another device just ignored is for a
    moment printless and not yet Ignored), and mended after the search for moved photos (`lost_prints`, `mend`).
    Still open: one whose photo is gone can only be Ignored again, and without a name it shows in no list.
  - A moved video was never found again: the search by name and size in the photo folders listed pictures only.
  - A Cameras tile stayed frozen after its tracker started again while the page stayed open in a browser: frame
    numbers start again with the program (and a relay's when it is made again after `IDLE`), and the page kept
    asking for frames after its old, higher number. `peek` now takes a number above its own as none.
  - Ctrl+1 to Ctrl+4 pressed inside a tab never reached the tab page: the pages in the tabs pass them on
    (`settings_bridge.js`, `fs: "key"`; the Cameras page loads the bridge for this). Opening Settings that way
    also takes the keyboard out of the now hidden tab.
  - `confirm()` stopped every script of the window (the tabs share one process): the camera, its stream, and the
    tab page's `/alive`, so a question left open 30 s looked like a frozen page to the watchdog (a restart on
    Windows after 60 s). The six questions are asked in the page now (`ask.js`, `askUser`), and a test keeps
    `confirm`, `alert` and `prompt` out of the page scripts.
- 2026-11-14, the NAS's camera gone for six hours (v1.22.2, started 20:50 UTC the day before; its desktop session
  disconnected as usual). At 11:31:40 UTC the window's page renderer ran out of memory (WebView2 `ProcessFailed`,
  reason OutOfMemory; Windows Error Reporting: msedgewebview2.exe, renderer, 0xe0000008), after frame gaps of 5 to
  8 s in the minute before. The app loaded the page again and logged that; after it, no frame came from the page
  until the app was restarted at 18:09 (the other devices were answered "not streaming"), and nothing more was
  logged. The window watchdog saw nothing wrong, so the tab page ran. Ending the renderer by hand recovers within
  3 s on the PC and on the NAS (tried on both), so the out-of-memory case took a path the tests don't: the camera
  check trusted any opened camera, and never escalated a start that is refused at once. Fixed in 1.22.3: running
  means the picture arrives, refused starts escalate too, and each step is logged (`/api/page-note`).
  **Why the page grows (open until 2026-11-15; found, see the end of this item).** `app.log` now has a memory line every half hour (`note_memory`); read the
  slope there after a day. First numbers: the renderer holds about 1 GB of working set within ten minutes of a
  start on the PC and the NAS.
  The slope, from those lines on 2026-11-14 (v1.22.3 and 1.22.4): on both Windows devices the window's memory
  grows by 1.5 to 2 GB an hour, all of it in the one page renderer (WebView2 157): 0.9 GB two minutes after a
  start, 3.6 GB after two hours on one, 0.8 to 5.0 GB in two and a half hours on the other (renderer 4.6 GB
  private). On Linux (Qt WebEngine) the window stays at 0.2 to 0.3 GB over the same hours, while the app's own
  process grows there from 1.7 to 2.1 GB in two hours (on Windows it stays at 3.1 GB). So the growth is in the
  page as WebView2 runs it, not in the page's script as such: look at the heap and at camera frames next (on
  Windows the app keeps them in ordinary memory, `KEEP_RUNNING_FLAGS`).
  **Found 2026-11-15, fixed in 1.23.0: pywebview's request handlers.** pywebview 6.2.1 hooks every request of a
  WebView2 window (`AddWebResourceRequestedFilter('*')`, `WebResourceRequested`, `WebResourceResponseReceived`,
  for its `request_sent`/`response_received` events, which the app doesn't use). The response handler makes
  WebView2 switch on the browser's network inspection in the page's process, and that keeps a record of every
  request for as long as the page lives; the pages make tens of requests a second (camera frames out and in).
  How it was found: Chromium's own memory dump of the real page over the DevTools protocol
  (`Tracing.requestMemoryDump`, `disabled-by-default-memory-infra`), twice, 30 minutes apart: the renderer had
  grown 1.3 GB, and the one kind of object that grew was `blink::NetworkResourcesData::ResourceData` (88 MB of
  them, with their table), which only network inspection makes. The Cameras page alone in headless Edge 157 (no
  such handlers) never grew: visible, hidden, 2 or 8 cameras. Then the same page in a pywebview window that is
  never shown, 5.5 minutes each: with the handlers the renderer went from 323 to 431 MB (24 MB a minute) with
  those records growing; with them taken off when the page had loaded, 41 to 50 MB and no such record, and a
  fifth more camera frames got through. `app.drop_request_hooks` takes them off; a test reads pywebview's
  source and fails when it hooks requests another way. What misled first: the page's picture cache showed
  1.3 GB within two minutes, but that is its cap of 2,000 decoded frames, counted in the page and held by the
  graphics process, and it doesn't grow.
- The Windows installer run while the app was open (first try, on the NAS): Restart Manager closed the app's
  second process, could not close the first, and Setup stopped ("unable to automatically close all applications"),
  leaving the app dead. `CloseApplications=no` and the script's own `CloseApp` since.
- macOS (v1.22.4): the app has been built, signed, notarized and self-tested on GitHub's Macs only, never used on
  a Mac with a camera. To check on the first one: the window (WebKit) opens; the camera starts with no question
  after a reload and after a restart (`allow_camera_macos`); the stream's frame rate with the window hidden or
  minimized (`KEEP_RUNNING_FLAGS` are Chromium's and do nothing in WebKit, which slows hidden pages, and macOS
  naps hidden apps: cameras are never to be slowed, section 8); the CoreML check at start; the watchdog's restart
  in place; photos and cameras to and from the other devices. `window_memory` gives nothing on macOS yet.
- Network review, 2026-11-14 (HTTPS to the internet, mDNS, device to device; measured on the three devices with
  read-only requests). What was found and fixed in 1.22.5:
  - One device's clock was 81 s slow (its time service stopped); requests are refused at `CLOCK_SKEW` (120) s
    apart, and nothing said so beforehand. Its clock is set; the app now writes to the log when another device's
    clock is more than 60 s away (`peer._clock_note`), and a refusal for the time says how far apart they are.
  - The peer service's accept queue was Python's 5: on Windows 16 requests at one moment took 1.04 s (those that
    found it full were sent again after a second); 128 now: 17 to 24 ms.
  - Every request to another device opened a database connection first (95 to 125 ms of a 126 to 135 ms request),
    and so did the Cameras page's list of devices every 10 s (109 ms). `facedb.ask` keeps one connection for such
    questions: 12 to 32 ms a request, 3 ms the list.
  - A device with **Fully distributed system** on told the others its Docker bridge address too; an address that
    doesn't answer was waited for 8 s. Bridges are no longer told (`hardware.reachable_addresses`), addresses are
    tried best first, each for 3 s (`peer._order`, `CONNECT`).
  - The service's answers are sent without waiting (`disable_nagle_algorithm`), as the local servers' are. This
    did not change what was measured: a photo of 0.1 to 0.3 MB takes 6 to 28 ms to arrive after its first byte,
    before and after. That time is a new connection getting up to speed, not held-back pieces.
  Measured and fine: TLS 1.3 everywhere, each device's certificate the pinned one, a handshake 3 to 4 ms; a camera
  stream is 960x540, about 56 kB a frame, 20.0 frames a second (9 Mbit/s for each watcher of each camera), the
  time between frames 50 to 54 ms at the median and 59 to 65 ms at the 95th percentile; three cameras reach the
  Cameras page at 20 a second each. mDNS found all three LM Studio servers in 20 of 20 tries from two devices,
  the answers in 2 to 16 ms. Check for updates takes 0.2 s; models download at 112 (Hugging Face), 336 (Google)
  and 736 to 894 Mbit/s (GitHub); the eight pinned CUDA wheels are all there (1.95 GB at 186 Mbit/s).
  Still open from it:
  - requests to another device each make a new connection (on Windows, Python's connect with a time limit alone
    takes about 15 ms); keeping connections open would save that and the start-up time of each photo;
  - a photo is read, scaled and encoded for each request (up to 0.7 s for a large one): no cache;
  - the mDNS search always listens its full 1.5 s though the answers are in within 20 ms; the search for LM
    Studio servers also tries every address of container and VPN networks (8.4 s on a device with three
    adapters, 3.3 s with one); Windows computers never answer the reverse mDNS question (1 s waited each);
  - one device's stream stopped for about 0.4 s every 11 to 17 s (19.6 frames a second instead of 20.0): in that
    device's page, not in the network. Cause found 2026-11-15: its app ran in a disconnected Remote Desktop
    session. With the session on the computer's own console the stream is 20.0 a second with no gap over
    84 ms, and the app sees the computer's sound devices, which a Remote Desktop session doesn't have at all
    (`tools/keep_session_on_console.ps1` puts a session back on the console at every disconnect). The console
    must stay unlocked: the first version of that script locked it, and the picture froze 20 s later
    (2026-11-15: 240 frames in 12 s, all the same picture, shown as live at 20 a second on every device for an
    hour, with the last face still "in view"; a camera started behind the lock screen never started).
    What Windows does, from that computer's own logs (Winlogon, the camera's driver, 2026-11-11 to 11-15):
    a Remote Desktop disconnect locks the session in the same second (four of four), and 20 s after a lock
    the running camera is powered down (the console lock, and three of three disconnects with the camera on).
    In a session locked by a disconnect, the app that was running doesn't get the camera back: a start from
    its page failed at once, every 10 s for two minutes, and the camera's driver logged no try. The app started
    again has it within seconds (the camera was powered up about twenty times in such sessions over three
    days, at the app's starts, with hours of changing pictures after them; asked to start again 2.5 minutes
    after a disconnect, its camera ran 16 s later). In the session
    locked at the console no start finished, in the running app or a new one, not even after that session was
    disconnected, until the owner signed in. Windows reports both as locked (`WTSSessionInfoEx`, flags 0).
    So before 1.23.0 every disconnect left that device's last picture streamed as live until the app was next
    started: 21 hours on 2026-11-13/14 (so the six hours "gone" in the entry above followed fourteen of a still
    picture), and nothing reported it, because frames kept arriving. Since 1.23.0 a camera
    that sends no new picture counts as paused (`camPaused` in `tracker_ui.js`): no faces are read from the
    still picture, the other devices are told, and it goes to the log. Under a lock the page stops and starts
    it at once; when that fails or doesn't finish, the app starts itself again, once per lock
    (`app.restart_soon`, `unlock_seen`); after that one start is left open and nothing more is loaded or
    started against the lock (the whole chain run on the NAS, 2026-11-15: disconnect 19:11:34 UTC, paused
    noticed 19:11:56, the page's start refused 19:12:14, the app started again, camera powered up 19:12:30,
    56 s after the disconnect);
  - 9 Mbit/s for each watcher of each camera is fine on a local network, too much for a slow link;
  - Describe photo sends a photo to an LM Studio server on another computer over plain HTTP (LM Studio has no TLS).
- **A minimized app window slows its camera's stream to about 1 frame a second** (found 2026-11-14 on Windows,
  v1.22.4 and 1.22.5; against section 8's rule that a camera is never slowed). For about 10 s after minimizing
  the rate holds at 20, then it is 1.2 a second for as long as the window stays minimized (watched 7 minutes);
  restored, it is 20 again at once. A window that is only covered by others, or in a desktop session nobody is
  connected to, keeps 20. The keep-running flags are in force (read from the window's processes). Likely cause,
  not shown yet: `streamFrame` makes each JPEG with `canvas.toBlob`, which the browser encodes in the page's idle
  time; a hidden page gets none, and the encoder then starts only on its fallback of about a second. To try:
  encoding that doesn't wait for idle time (`OffscreenCanvas.convertToBlob`, or in a worker), measured minimized
  for minutes, not seconds (the earlier check of "minimized" must have been shorter than the 10 s). Until then:
  don't minimize the window.
- Music and room sound (v1.23.0, 2026-11-15), what was seen on the three devices and what is open:
  - The Spotify app shows the song even when its sound comes out of another device: on the first evening both of
    the PC's outputs were silent while it showed songs (checked with a quiet test tone, which was heard back), so
    only a microphone could have heard them, and none did. Later the same app played through a display, and the
    output heard it as music (0.95 to 0.98).
  - Raw model answers for a fan are "Waterfall", "Vehicle", "Pink noise": hence the families, and "steady
    background noise" for anything that hardly moves, with its kind from the numbers (hiss, low rumble, mains
    hum). An input at nearly full strength that never changes is said to be, probably, one with nothing plugged in
    (seen on the laptop's analog input). A microphone's constant offset is taken off before anything is measured.
  - One step failing took every interface with it: a device with a display output and a webcam microphone listed
    none (`interfaces` now lists outputs and inputs each by itself, and says why one couldn't be).
  - Open: none of the words or cut-offs (`FAMILIES`, `what`, `background`, `SILENT`) is measured; the key and mode
    of a song are rough (a song in a minor key came out major); a Mac hears its microphones only; players other
    than Spotify aren't read; the tracker page doesn't show any of it yet. Whether what a person plays goes with
    how they say they feel is for the feeling reports to show.
  - The owner asked what happens with several cameras on one device: one runs, the one chosen in the tracker; the
    stream, the Cameras tab, sightings and the expression log all count one camera a device. Not built.
- Library versions, reviewed 2026-11-14: every pin in `requirements*.txt` is the newest release (psycopg 3.3.9,
  mediapipe 1.2.3, boto3 1.44.3 and ruff 0.17.5 moved that day) except ONNX Runtime, which stays at 1.24.4 on
  purpose (1.32.0 was the newest). 1.24.4 is the last release with a DirectML package (Windows). On Linux
  `onnxruntime-gpu` 1.25.0 to 1.26.0 still use CUDA 12; from 1.27.0 it needs CUDA 13 (`nvidia-*` 13.x,
  `nvidia-cudnn-cu13`: read from each release's own list on PyPI), and CUDA 13 needs NVIDIA driver 580 or later
  and drops cards older than the RTX 20 series (Maxwell, Pascal, Volta), where CUDA 12 runs from driver 525.60
  and the GTX 900 series (`cudasupport.check`). One version on every system keeps the answers comparable; moving
  Linux (to 1.26.0 at most without losing cards) means new pins in `setup.sh` and `cudasupport.py` and the
  comparison with the CPU again, for no known gain. Open: measure 1.24.4 against the newest `onnxruntime` with
  CoreML on the first Mac, and move macOS alone if it is better (the two pins are per system already).
  CI's actions moved to their v7 releases (Node 24; the v4 and v5 ones ran with GitHub's "Node.js 20 is
  deprecated" warning), and CI's Node tests run on Node 24.
- The changes log is never pruned; `photo_paths` has no index on `hash`; devices on different recognition models
  overwrite each other's prints; a failed local forget is recorded as done.
- Linux: after the watchdog's restart in place the window's DevTools port stays dead (testing only), and one camera
  start hung after a stop and start a few seconds apart (the app restarted itself and recovered).
- Git history before 9c76e0f still holds the old Connect defaults (a host name and a database name).
- For later, asked for by the owner: reports on the expression log; a per-picture quality menu on the Cameras tab.

### A fourth device and the first Describe everything again (2026-11-16)

A fourth device (a Windows laptop with an 8 GB NVIDIA card) was installed from the 1.23.0 installer and joined; and
**Describe everything again** (Settings → Describe photo) ran for the first time over the whole library. Found and
fixed that day (in 1.24.0):
- **A device with an empty library couldn't join the shared database.** The join first saves the device's own
  files, and saving zero faces failed (`Store.save`: reshape of an empty array); the device stayed "offline" with
  that error for good. The three earlier devices each had faces when they joined. Tests: an empty library saved
  and read again; a new device joining and getting everything.
- **On a new copy the camera never started on Windows.** WebView2 names the page around a frame as the one asking
  for the camera, so the tracker's request arrived as the tab page's; the app only answered the tracker's address,
  WebView2 showed its own question, nobody answered, and no later request was raised while it stood (page reloads
  and the app's restart changed nothing). The three earlier devices had "allowed" remembered in their profiles.
  Reproduced on the PC in a test copy with a new profile and a made-up camera (`--use-fake-device-for-media-stream`
  in `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`: the real camera is never opened), old code against new. Now the tab
  page's address is answered too (`app.camera_allowed`), the answer is in place before any page loads, the tab
  page's `Permissions-Policy` hands camera and microphone to the programs' ports only, and the log says each
  request and its answer. (A header without `self` refused the tracker outright: a page can't hand on what it may
  not use.)
- **Describe everything again**, as it ran: 13,095 photos and video frames with Qwen 3.8 27B, three computers,
  about 12 s a photo on each Ryzen AI Max+ 395 and 22 s on the 8 GB laptop (the 35B-A3B model had taken 3.5 s,
  which the question's time estimate still assumed that day; it now uses what the model in use really took here:
  `describe.typical_secs`). The laptop's LM Studio died on its first picture (20 layers
  on an 8 GB card: "CUDA error: the resource allocation failed") and the run left it out for good after two
  failures in 13 s; loaded by hand with a quarter on the card it works, and Stop then Carry on brought it back.
Open after it:
- (Done in 1.24.1: a helper left out is tried again every five minutes, `label_tool.HELPER_RETRY`.)
- A new device can't read the shared models folder on the NAS without credentials for it: its model was copied
  from the PC by hand. Settings → LM Studio should say so instead of only offering the internet.
- The run ended on 2026-11-17 with its whole list described, yet Settings still says "Unfinished": a run counts
  as done only when nothing in the library is older than its start (`again_due(describe_items(), since)`), and
  Collect faces pictures arrive all the time (about 900 in the seven hours after it ended, none of them on the
  run's list). A run should end when its own list is done.
- LM Studio's per-model load settings on the 8 GB devices are set by hand (`lms load … --gpu 0.25`), and are
  gone after LM Studio restarts.
- The fourth device (32 GB of memory, 8 GB card) can run the 27B model, but not beside the app: 45 to 78 s a
  picture, and its free memory fell to 0.3 GB. Its LM Studio server was stopped again, so it takes no part in a
  shared run; a smaller vision model there, or leaving it out, is the owner's choice.
- Seen twice while testing the release build, on the first two starts of the just-downloaded Windows .exe as a new
  copy: no window (`clr.AddReference('System.Threading')` in pywebview's `winforms.py` raised "Could not load file
  or assembly"), and the app fell back to the web browser. The next ten starts of the same file, five of the
  1.23.0 build and sixteen runs of that call from source all worked, and none of 103 starts in one device's log
  shows it. The window's libraries are the same as in 1.23.0 (pywebview 6.2.1, pythonnet 3.2.1); the cause isn't
  known. `--self-test` didn't load the window's platform on Windows, so it wouldn't have caught a build where this
  always fails; since 1.24.1 it loads pywebview's Windows platform, WebView2's library and the types the camera
  answer uses. `pyinstaller-hooks-contrib`, `pythonnet` and `clr_loader` weren't pinned (CI took hooks-contrib
  2026.10 where the PC had 2026.9; both builds pack the same .NET files); they are now (2026.10, 3.2.1, 0.3.1).

Released as 1.24.0 (2026-11-17 UTC) from CI's builds of 0417201, each installed with its installer on four devices
first (three Windows, one Linux): data and models untouched, every camera live on every device, and the running
describe run stopped, the app updated under it, and carried on. The Windows build was also started as a new copy
with a made-up camera: the window answered the camera request by itself.

### Code review of 2026-11-16 (security, speed, structure)

Four read-only reviews of the whole of `face-recognizer/` (the local servers and pages; the network, keys and
downloads; structure; speed), each finding then checked against the source. Nothing was found that lets a website
read or change anything, and no way to run a script in a page; the request checks, the page policies, the keys'
encryption, the pinned certificates between devices and the download checks held. What was found starts from
someone who has the shared database's password (or one device), or sits on the local network.

Fixed in 1.24.2 (tests: `test_sync_rows.py`, `test_sync.py`, `test_describe.py`, `test_servers.py`, `pages.test.js`). On the four devices after the update: no row left out and no setting put back in any log (the database held nothing of the kind), and the two computers that help describe are still helpers under the new rule:
- **A row could name a file on every device.** `Local.apply("photos")` copied the row's `extra` column into the
  photo's record, `path` included, and `peer.own_photo` shows the file at a photo's path to any device that asks
  with the cluster key (which the database's password gives). Now `dbsync.PHOTO_FIELDS` are never taken from
  `extra`, and `own_photo` shows only a record with a path and a size.
- **A row could change a device's settings.** Rows of `device_settings` for this device were applied when anyone
  changed them: the music listener (every microphone), the camera stream, Collect faces, the describe server's
  address, Share Describe all, the shared models folder (a path, looked at as soon as it is set). Now a device
  applies nothing from that table and puts its own values back; Copy settings (`copy_settings`, the owner's click)
  is the only way in, without the settings in `dbsync_home.STAY_HERE`.
- **One row no device could use stopped every device's pull for good**: an alert that is no object, a face with a
  box of three numbers (saved with the rest, and then no save worked), a visit at a time no calendar has. The
  pull raised at the row and read it again at every step. Now `Sync._apply`: checked first, halved when applying
  still fails, left out and named in the log.
- **Share Describe all sent photos to any computer on the network that answered like LM Studio** with the model's
  name, also one using the name of a computer of the owner's. Helpers are now devices of the shared database at
  the addresses they gave there (`describe.device_addresses`), or this computer.
- **The window's cache kept pictures for a year**: `/thumb` and `/photo` were answered `max-age=31536000,
  immutable`, so a face Ignored or a person forgotten since stayed in the cache on disk, and so did originals
  another device had only shown. Every answer is `no-store` now; the labeling page keeps what it showed as
  blob: URLs in memory (`PICS`), so drawing a list again asks for nothing (checked in a browser: no request on
  the second draw, the photo fetched once, an Ignored face's picture let go at once).
- Smaller: `tracker.camera_ids` raised on a digit that is no number; a relay thread was started for any device id
  a request named (now at most `camrelay.MAX_RELAYS`); the three pinned downloads were only checked once they
  had ended (one without an end filled the disk); `Recorder.SESSION` took a name ending in a line end.

Fixed in 1.24.3 (released 2026-11-17; tests in `test_servers.py`, `test_localguard.py`, `test_peer.py`,
`test_sync.py`, `test_sync_rows.py`, `test_facedb.py`, `test_describe.py`, `test_labels.py`). As released: the
built Windows window, started as a new copy, had none of its pages' 2,421 requests refused (the 1.24.2 build
beside it: the same); on all four devices the peer key was encrypted where it was with the certificate unchanged,
and each device's camera reached the others as before. What a release publishes was searched for the first time
inside the builds too (each PyInstaller program unpacked down to its modules, the .dmg's blocks uncompressed):
no key, password, token or private key of the computers it was made on, in text or in their other encodings.
What the macOS files do carry is their Developer ID signature, which names the developer (every signed Mac app
does; the owner's choice of an individual account, 2026-11-14).
- **The app showed whatever answered on its programs' ports.** A program holding the tracker's port, answering like
  it and naming the window's tab page in its frame-ancestors was framed as the tracker and had its camera and
  microphone request answered. Now `app.start` shows only what this process started (`OWN`), and
  `camera_allowed` answers only for that.
- **A GET needed nothing but the right Host.** A page of another program on this computer (`Sec-Fetch-Site:
  same-site`) could start a search of the network, a camera relay or a look for faces in a photo, and no route
  told its own page from any other. Now data and what a GET starts are for the tool's own page (`same-origin`)
  or a program sending the header (`localguard.OWN_ONLY`). Checked in a browser against a test copy: 1,689
  requests of the app's own pages, none refused, no page error; another program's page was answered 403 on
  every data route of the three servers and 200 on a page file.
- **An old passphrase link fitted again after a restore** (`secretbox._link` didn't say which version it
  replaced; a restore gives a new version an earlier key): whoever can write rows could send every device back
  to the passphrase the owner had undone. The new test does exactly that against the old code and fails there.
  Links made before 1.24.3 no longer fit: a device that was one change behind when it is updated needs the
  passphrase typed or approved once (none of the four was behind).
- A `database.json` without the server's CA now logs in with channel binding; `florence.py` takes a file already
  there only with its SHA-256; `peer-key.pem` is encrypted, its password protected (a key from before is
  encrypted where it is, the certificate stays); nobody can be named `constructor`, `__proto__` or another word
  of `labels.NOT_NAMES`, in the tool or by a row.

Looked at and left as it is:
- A signed request between devices isn't bound to the device it is sent to (`peer.sign`). Binding it would
  change nothing today: a request reaches only a device whose certificate is in the database, writing that row
  takes the database's password, and the password gives the cluster key, so whoever could pass a request on
  can sign their own. It becomes worth doing the day devices get keys of their own.

Open from it, security (not reachable from a website):
- Another account on the same computer can call every route of the three servers, `/api/secrets` included: the
  header the routes ask for is a constant, and a program that isn't a browser sends what it likes. A token made
  at each start would close it; whether a computer with several accounts is in scope is the owner's decision.

From it, speed. Measured 2026-11-17 on a copy of the owner's library (16,479 faces, 20,207 photos, 133 people of
whom one has 6,137 usable faces), the labeling tool from source on a port of its own; released in 1.24.4:
- **Every change scored every face against every person again, twice** (the summary's suggestions and its
  checks), and compared every face with every face for the stacks when a print changed or a face was added: the
  page's questions after one label took 1.3 s, after an Ignore or imported photos 2.4 s, all under the lock an
  import needs. Now the scores and the stacks are kept and brought up to date where a change touches them
  (`labels._columns`, `_closest_kept`, `stacks`): 0.13 s after a name, 0.3 s after an undo, an Ignore or imported
  faces; the first question after a start takes 1 s longer than before (3.4 s: every face is scored, also the
  named ones). In the tool itself a summary after a name went from 0.8 s to 0.17 s.
  - The same answers: the old and the new engine side by side on the copy through names, a new person, a "not
    this person", an Ignore and its undo, 40 faces imported 8 at a time: every best name, every decision a score
    leads to (suggested, automatic name, hint, margin) and every list identical; the largest difference between
    two scores 9e-8 (the products are made in other batches).
  - What it took that reading the code hadn't shown: one person with 6,137 faces cost 0.57 s for each face named
    as them (hence `BIG`); a blurred face is among the five closest of 4,800 other faces, so when it leaves that
    many are looked at again; faces without a print are all "as close" as the fifth closest (0) and were looked
    at again by the thousand until left out.
- **The sync hashed every row under the labeling lock** after each save and every minute (0.5 s): another request
  waited up to 0.52 s for the lock, now 0.03 s (the hashing happens after the lock; the digests are the same).
- **`indent=` in the large files** takes Python's slow encoder: photos.json 0.12 s at every save (now 0.03 s),
  labels.json 0.02 s at every label (0.005 s), descriptions.json 0.08 s at every description (0.04 s). One line
  per entry instead (`paths.rows_json`); they read back as the same data.
- The list of webcam photos was made again after every change with a `Path` per photo (0.1 s): as text now.
- Measured and left: the face store written every 50 imported photos (0.26 s per 50 photos); Describe photo's
  status (4 ms); the alert inbox (185 kB, 2 ms to read).
- The leftovers, measured and changed on 2026-11-18 (1.24.5):
  - with Settings open, every 5 s: LM Studio's probe started `lms ls` (0.5 s; the list is kept a minute now, and
    asked anew after a download), and the shared database's state opened a new connection under the labeling
    lock (0.1 to 0.8 s; now outside the lock, on the kept connection: 2 ms);
  - the summary's people tags went through every photo's facts after every change (0.1 s; 19 of 10,979 photos
    have tags: those are found once);
  - after an Ignore the unknown faces were grouped anew (0.5 s): the Ignored face now only leaves its group, and
    comes back into it on undo.
- Seen and left: the background model of what a face looks like trains 3.5 s after a start and after an Ignore
  (off the lock, on another core); the first summary after a start takes 3.4 s; the tracker and the importer each
  load a full set of models (memory, and the tracker must not wait for an import's use of them).
- **The Linux app's own process grew by 1 to 2.7 GB an hour while people were in view** (found and fixed
  2026-11-18, 1.24.6; the window's processes were never part of it). In `app.log`'s memory lines: level all night
  at about 2 GB, then to 6 to 8.6 GB over a day, where it levels; the PC's stays at 3.3 to 4.3 GB. No leak: glibc
  keeping memory the app had freed.
  - What the running app said. Asked for glibc's own account (`malloc_info`, called through gdb in a thread that
    was waiting: a stop of 0.4 s), its 128 arenas had taken 3.0 GB from the system with 1.6 GB in use.
    `malloc_trim(0)` gave back 0.26 GB: it frees pages inside free pieces and leaves the free tops of thread
    arenas alone, where most of it was. Then someone was in view for a quarter of an hour: the process went from
    3.1 to 4.2 GB, in use from 1,583 to 1,629 MB (250 collected faces), freed and kept from 1,408 to 2,341 MB.
  - Why. 236 threads share 128 arenas (8 for each of 16 processor threads). Once a large block has been freed,
    glibc takes every block up to that size (at most 32 MB) from the arenas instead of straight from the system,
    and keeps up to twice that size free at each arena's top. Every label change makes blocks of 9 to 70 MB for
    a moment (the prints as one table for faces.npz, the pictures joined for thumbs.npz, the score table,
    people.npz read again by the tracker), each time in the arena of whichever thread is at work, so each arena
    ends up with tens of MB of its own. It takes label changes: with Collect faces on, a snapshot is imported
    every 20 s while someone is in view. Frames alone don't do it (6,200 frames through a test copy: level).
  - The fix (`paths.give_memory_back`, when `paths` is loaded, before any thread): 8 arenas, a block of 8 MB or
    more straight from the system and back to it when freed, at most 16 MB kept free at an arena's top
    (`mallopt`; setting the two sizes also stops glibc from raising them). Not when `GLIBC_TUNABLES` already
    sets a `glibc.malloc.` value, which is how the old way is measured beside the new.
  - Measured on test copies on the laptop (a copy of the library, CPU only, no shared database; a script sends
    frames as the page does and a snapshot of each new face, which the labeling tool imports):
    - 8 minutes, the installed 1.24.5 with glibc set through the environment: left alone 0.67 to 2.09 GB (70
      arenas, 1.70 GB taken, 1.17 GB of it free); 1 MB and 4 MB: 1.09 GB; 8 MB and 16 MB: 1.25 GB and level. In
      use the same 0.80 GB in all three.
    - A frame's time, one copy at a time: left alone 127 and 139 ms, 1 MB 135 and 142 ms, 8 MB 126 ms. A frame's
      own buffers are 1.5 to 7.4 MB: under 8 MB they stay in the arenas, with 1 MB each one came new from the
      system. Hence 8 MB.
    - 20 minutes, the installed 1.24.5 beside the built 1.24.6 (the setting made by the code): 0.67 to 2.18 GB
      against 0.71 to 1.25 GB, level from the second minute; 9,345 and 9,362 frames, 124 ms each in both; in use
      802 MB in both. (The script's snapshots ended after six minutes in every run: both copies got the same 77.)
  - What misled first: the kernel's list of the process's memory, with only maps that begin on a 64 MB boundary
    counted as arenas, said "the arenas are level, other blocks grow": adjoining arena heaps are one map there.
    glibc's own account was the thing to ask.
  - The real app (GPU path, shared database), the built 1.24.6 on the laptop since 2026-11-18 16:12 UTC: with
    people in view for 25 minutes (209 snapshots imported, 114 reloads of the known people) its process went
    from 1,855 to 1,939 MB, where 1.24.5 had gone from 3.1 to 4.2 GB over 244 snapshots an hour before. Its
    account then: 8 arenas, 1.57 GB taken, 1.37 GB in use, 0.20 GB free and kept. Still to read: its memory
    lines after a full day (about 2 GB is level; whether what is in use creeps up by more than the new faces).
  - The tools (glibc's account through gdb, what kind of data a process's memory holds, the test-copy scripts)
    are in the backup: `speed-tools/linux-memory`.

From it, structure, done on 2026-11-18 (1.24.5):
- `label_tool.py` went from 981 to 561 lines: the Describe all job is `describejob.DescribeJob` (261 lines moved
  as they were, the closure's variables reached through `self`), and what the Settings tab asks is
  `label_settings.py` (its routes and `db_state`, `secrets`, `app_version`), asked first by the handler. The
  labeling page's own routes are still one `_get` and one `_post` in `make_handler` (about 120 lines each).
- The Web search answers have one lock: the labeling data's own (`db.web_lock` is `db.lock`).
- `waitlog.WaitLog` is what `exprlog.py` and `musiclog.py` shared (old and new compared on the same rows, a
  previous version's file, a failing send, a forget and exports: 122 comparisons the same, files byte for byte).
  It showed a loss that was there before: after a forget, the rows of everyone else that day could miss the
  database (the file, rewritten shorter, had grown past what the sync's state said was sent, and the send began
  in the middle of the new lines; the file was later deleted as all sent). Fixed in the one place it now has.
- The forgotten names are `forgotten.py` (six lazy imports are plain ones now); nine re-exports of `labels.py`
  that nobody used and the `/api/autoname` route are gone.
- Two tripwires: `tests/test_routes.py` (every address a page asks for is answered by the server that sent the
  page; every route has a caller; every GET a page sends is really sent, with anything beyond the test servers
  stopped: 70 routes of the labeling tool, 36 of the tracker, 13 of the tab page; it passed unchanged across the
  split but for the one line naming the new file) and `tests/test_sync_tables.py` (each of the 13 synced tables
  has all its places, from `KEYS` to the migration's trigger and the person's export and forget). Each was shown
  to trip, sixteen ways.
- `dbsync_home.py` has a class per table (`Table` and nine after it: where a table's rows are on this device
  and in the database, what a row must look like, how it is applied), and `HomePart` hands each question to the
  table's class. Before, each table was a branch in seven long functions, so a new table meant about fourteen
  edits, and a table without its branch in one of them fell into that function's last: in `apply` the forgotten
  people's, where every row's key would have been forgotten as a person on every device. Now a table neither part
  knows is refused everywhere (`HomePart._table`; `dbsync._ours` for the labeling data's four tables, whose
  functions still branch and end with the faces).
  - The same answers: the file before and after side by side on one made-up private folder, every table through
    `check` (good rows and one that can't be used), `apply` (rows and a deleted one), `digests`, `rows`, and
    `push`, `fetch` and `all_keys` against a stand-in that takes down every statement: 150 comparisons, none
    different. The two-device tests against a real PostgreSQL pass as before.
Still open, structure: the labeling data's part (`dbsync.Local`, four tables) still branches per function;
`device_settings` and `forgotten` rows are checked by key only.

1.24.5 as released (2026-11-18; CI run 44242836287, commit 5acb8a3):
- Its first CI run failed in the `checks` job on all three systems and built nothing: a new test grouped unknown
  faces for real, which needs scikit-learn, and that job doesn't install it (nor Pillow, ONNX Runtime, MediaPipe,
  boto3, soundcard). The test now stands in for the grouping. Before a push, the suite is run with those
  libraries blocked from import as well (a script in the backup beside the release scan tools).
- On each of the four devices the first sync step under the new code compared every row with what 1.24.4 had
  recorded and found nothing to send or take (the new table classes give the same digests as the old branches,
  on the real data of four devices), and no row was refused; `/api/db` answered from its new module on the kept
  connection. Not yet seen on the devices: a row moving through the new code. Nothing changed on any of them in
  the hour after the installs (nobody in view of a camera since 05:01 UTC, no labeling); the two-device tests
  against the same PostgreSQL server did it in schemas of their own. Look at the logs after the next day's use.
- The built Windows .exe as a new copy: its window after 4 s, none of its pages' 2,505 requests refused, the
  camera request answered by the window. One `/api/recognize` request of the tracker page ended before its
  answer (not seen in the 1.24.4 run; the page lets a request go when a newer frame is ready).

Other open items (2026-11-12):
- The macOS app is signed and notarized by CI since 1.22.4 (2026-11-14): `tools/apple_signing.py` made the
  certificate request and the secrets without a Mac, and the built app's `--self-test` runs under the signature.
  Still to see: the app used on a Mac with a camera.
- **Windows code signing** (asked for 2026-11-17; built, set up in Azure and tried from the owner's computer the
  same day, and CI signs since then: its first signed build is run 44207548400). The .exe, the
  installer and its uninstaller are signed when the build is told with what (`build_exe.py`, "Windows code
  signing"); CI does it on pushes to `main` once five repository secrets are set. Chosen: Azure Artifact Signing
  (about 10 USD a month; the key stays in Microsoft's service, and CI signs in as the repository's workflow, so
  nothing secret is stored). Looked at and not chosen: a certificate authority's OV or EV certificate (150 to
  400+ USD a year, and its key must live on a hardware token or a paid cloud HSM, which CI can't use without
  storing a secret; EV no longer skips SmartScreen since 2024), and SignPath Foundation (free for open source,
  but the publisher shown is theirs).
  - Tried with a self-signed certificate on the 1.24.3 build: the signed one-file .exe passes its self-test (a
    signature is added at the end of the file, where the .exe looks for what it carries); Inno Setup signed its
    uninstaller and Setup through `--sign-file`; a copy under another app id installed, `unins000.exe` and the
    installed .exe read as signed, and it uninstalled clean.
  - Set up the same day with the owner signed in to Azure: the signing account (Basic), their identity check as
    an individual (under an hour from the request to Completed), a Public Trust certificate profile, an app
    registration with no password that Azure trusts for this repository's `main` branch alone and that may sign
    with that one profile, and the five repository secrets. What it took that the guides don't say plainly:
    - the owner's subscription was billed to a Business billing account, which can't take an individual's
      check: a second subscription on their Individual billing account holds the signing account;
    - the check's form takes its email address from the billing account and can't be edited; an alias of the
      owner's own mailbox worked;
    - the check's id, which the certificate profile needs, can be read in the portal only;
    - the Azure CLI was unpacked from Microsoft's signed installer without administrator rights
      (`msiexec /a`), and is called as its `python.exe -IBm azure.cli` from PowerShell (its `az.cmd` breaks
      on brackets in an argument).
  - Tried against the real service from the owner's computer (their own account given the Signer role for the
    trial and taken off it again): a copy of the released 1.24.3 .exe was signed in 6 s, and Windows reads it as
    Valid, dated, signed by the owner's legal name with city, state and country (what an individual's
    certificate carries). The copy was deleted.
  - CI's first signed run failed at the sign-in, then passed: GitHub names a repository to Azure with numbers
    added (`repo:<owner>@<owner number>/<repository>@<repository number>:ref:refs/heads/main`), so the trust
    written with the names alone matched nothing (AADSTS700213; the error prints the subject it wanted). With
    the trust corrected, the job signed the .exe and, after signing in again, the uninstaller and Setup; both
    files downloaded from the run read as Valid and dated on the owner's computer, fit the SHA-256 the build
    wrote down, and the signed .exe passes its self-test. GitHub's Windows machines have the .NET 10 runtime the
    plug-in needs. The log shows none of the Azure ids or names (they are secrets, so masked); the check after
    signing first printed whom the certificate names, and now prints its thumbprint and issuer instead.
  - 1.24.4 is the first signed release (CI run 44222896544). The signed installer went over the unsigned 1.24.3
    on three Windows devices as any update does (the open app closed by it, data and models left alone); on
    each, Windows reads the installed program and `unins000.exe` as signed and valid. The .exe taken out of the
    release's zip is the built one, signature intact. The built .exe as a new copy: its window after 6 s, none of
    its pages' 2,222 requests refused, the camera request answered by the window.
  - Not yet looked at: what SmartScreen says for a download of a signed file (it should name the publisher and
    still warn at first).
  - What signing does not do: stop SmartScreen's warning at once. Windows warns about a new publisher until it
    has seen enough installs, for any kind of certificate. The warning then names the publisher: the owner's
    legal name with an individual account, as on the Mac.
- `hand_landmarker.task` is fetched from MediaPipe storage only (optional in `fetch_models.py`); not yet in
  `models-v1`.
- Asked for on 2026-11-12, for later: a setting on each picture of the Cameras tab (a gear on the tile, as on
  YouTube) for its quality and frame rate. Today the rate is one constant for every camera (`STREAM_FPS`).
- Devices on different versions: the Cameras tab of 1.20 shows no readout for a 1.19 device; there is no version check
  between devices yet, so every device has to be updated together.
- Step 2 of "each person's normal" ("Each person's normal", the interaction layer between identified people) is still to discuss.
- Record more posed sessions (now with the talking and "chin on hand" steps) and feeling reports; then
  decide on `TALK_JAW_SD`, the talking and hand gates, the state model and any cue fusion from
  `tools/expr_eval.py`.
- Check the voice model, pulse, posture and breathing live (real speech, a still face, a seated body);
  none has been validated on the webcam yet.
- If continuing, re-measure thresholds after any
model change (`python face_id.py reembed`, then `tools\eval_models.py` and `people`).
