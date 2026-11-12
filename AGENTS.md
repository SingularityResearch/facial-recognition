# Facial-Recognition — guide for coding assistants and contributors

This is the one guide for anyone changing the code (Claude Code reads it through `CLAUDE.md`; Codex and others read it
directly). Keep it current when the layout, conventions or checks change.

Local face recognition for a USB webcam: learn specific people from a photo library, name/ignore faces
in a labeling tool, and show names + match confidence (+ optional expression) on faces live in the
browser. Everything runs on this machine; no data leaves it, except a face or photo the owner sends
on purpose with the labeling tool's **Web search** (face to Amazon, photo to Google; never automatic), photos sent
to be described when the owner points **Describe photo** at a server on another device (Settings), and, once the
owner sets one up, everything private going to their own **shared database** (PostgreSQL + pgvector over TLS, so
several devices share names, labels and private data; optional, off for a new copy).

Goals, key decisions and research value: [`README.md`](README.md).
Full background, history, measured tuning values and gotchas: **see [`PROJECT_CONTEXT.md`](PROJECT_CONTEXT.md).**
User-facing docs: [`face-recognizer/README.md`](face-recognizer/README.md).
Why expression reading is layered (the science): [`docs/WHY_LAYERED_EXPRESSION.md`](docs/WHY_LAYERED_EXPRESSION.md).
Expression design review (LM Studio export): [`Real-Time Facial Emotion Recognition.md`](Real-Time%20Facial%20Emotion%20Recognition.md).

## Layout

- `docs/` — `SETTINGS.md` (every setting, default, purpose, environment variables, what reaches the network; keep it
  in step with each page's `SettingsBridge.init` list) and background write-ups (`WHY_LAYERED_EXPRESSION.md`).
- `face-tracker/index.html` — original standalone MediaPipe tracker; double-click to open. It uses the internet (jsDelivr,
  Google model storage and fonts) and isn't part of the offline tool; its MediaPipe main file is pinned with an
  import-map `integrity` (SHA-384).
- `face-recognizer/` — the Python project. All programs share `face-recognizer/data/`:
  - `face_id.py` — core library + CLI (`scan`, `cluster`, `name`, `people`, `recognize`, `reembed`, `migrate`).
  - `cudasupport.py` — NVIDIA CUDA for the frozen Linux app (built with plain ONNX Runtime): `check` (driver 525.60+,
    compute capability 5.2+), `offer` (Settings → This computer's **Download CUDA support**, never automatic),
    `start_download` (`/api/cuda/download`: onnxruntime-gpu and NVIDIA's cu12 wheels from PyPI, pinned by size and
    SHA-256, unpacked into `ROOT/cuda`), `activate` (at app start: a meta-path finder imports `onnxruntime`/`nvidia`
    from there ahead of the bundled copy; falls back to it if that fails). `FACE_CUDA=off` turns it off.
  - `lmstudio.py` — LM Studio on this computer, kept as the owner wants on every computer with this app and LM Studio
    (`keep()`, every minute from `app.py`; `FACE_LMSTUDIO=off`): `serve_everywhere` (its `http-server-config.json`
    `networkInterface` 0.0.0.0; a localhost-only server restarted with `lms server start --bind 0.0.0.0`), `announce`
    (mDNS `_lmstudio._tcp` + `_http._tcp` with its port: Windows `DnsServiceRegister`, macOS `dns-sd -R`, Linux
    `avahi-publish-service`; skipped when the computer announces it already), `missing_models` / `get_models` (vision
    models other servers on the network have, from Settings → LM Studio's button: first from the shared models
    folder, `from_share`, checked by SHA-256, removed again if a file fails; else `lms get <model>@<quant>`), `publish`
    (puts this computer's vision models in the shared folder: `<share>/models/...`, `<share>/hub/...`,
    `<share>/index.json` with each file's size and SHA-256; the folder is this computer's setting in
    `%APPDATA%\FaceLabels\lmstudio.json`). Stopping `lms get` doesn't stop LM Studio's own download: it keeps a paused
    job that hides the model until cancelled in LM Studio. The Labeling header's second bar (`#dlprogress`,
    `/api/downloads`) shows model and CUDA downloads.
  - `facedb.py` + `db/NNN_*.sql` — the optional shared database (PostgreSQL + pgvector on a server the devices
    share; since v1.16): connection settings in `HOME/database.json` (password DPAPI-protected on
    Windows, mode 600 elsewhere; TLS `verify-ca` against the server's own CA from `tools/db_certs.py`), this device's
    id in `HOME/device.json`, migrations under an advisory lock, `changes` log by trigger (deletes as tombstones).
    Connections are autocommit: group writes in `with conn.transaction():`. Without `database.json` (or `FACE_DB`)
    nothing changes: files only. `tools/db_import.py` copies a device's `data/` and HOME in (reads only; merges by
    face ID and photo hash; a hand label beats an automatic one; Ignored stays Ignored); `dbimport.py` holds the
    work (Join uses it). `facedb.setup` (Settings → Shared database → Connect) logs in with sslmode=require, takes
    the server's CA from `server_ca` (SCRAM proves the server knows the password) and keeps it for verify-ca.
    Tests: `test_facedb.py`, `test_sync.py`, with `FACE_TEST_PG` (CI job `database`, pgvector service; each test
    makes and drops its own schema).
  - `dbsync.py` — the sync, in the labeling tool's process (`label_tool.start_sync`): files stay the working copy;
    each row's digest as last synced is in `HOME/sync.sqlite`; changed rows are pushed (after each save:
    `face_id.SAVED_HOOKS` → `Sync.note`; and every minute), other devices' changes pulled from `changes` every 3 s
    and applied through the stores (so `people.npz` is rebuilt and the tracker reloads by mtime). Parts: `Local`
    (photos, faces, labels, Web search answers; the undo log only sent) and `dbsync_home.HomePart` (descriptions,
    Collect faces pictures, alerts, presence, shared documents, keys, forgotten people, device settings, person
    signals; the app log and posed tests only sent; `count_use` holds the Web search caps across devices). Join (first
    sync with a database, by its `instance` id): dbimport, then everything pulled. A new place that keeps data must
    get a table (new `db/NNN_*.sql`) and a part.
  - `secretbox.py` — keys in the database encrypted on the device (scrypt + AES-256-GCM, name as associated data);
    `secret_check` tells a right passphrase from a wrong one; the derived key in `HOME/secret.key` (protected).
  - `devicesettings.py` + `page_store.js` — each page's settings (`HOME/settings.json`) and each person's usual look
    and signal normals (`HOME/person_signals.json`), kept by the page's own server (`/api/settings`,
    `/api/signals`); `page_store.js` restores them into localStorage first thing (one synchronous request) and sends
    each change; localStorage is the page's quick copy.
  - `hardware.py` — what the computer has (`describe`: processor, memory, GPUs, NPU, ONNX Runtime providers; from
    the registry, /proc and /sys, sysctl) and how the programs use it (`uses`: `face_id.LAST_DEVICES`,
    `GPU_DECISIONS`, `import_workers`), `ai_apps` (AI model apps for Describe photo: the usual local ports, the
    configured describe server, installed apps, LM Studio's downloads, the LAN address when served to the network;
    cached a minute) and `lmstudio_network` (every LM Studio server on the local network: name, addresses, port,
    models); the labeling tool's `/api/hardware` and `/api/hardware/network`, shown in Settings → This computer and → LM Studio.
  - `paths.py` — `ROOT` (models\ and data\; `FACE_ROOT` overrides), `HOME` (`%APPDATA%\FaceLabels`; `FACE_HOME`
    overrides), `atomic_write`/`replace_file`. Every module takes its folders from here. Importing it also makes
    HTTPS check certificates the operating system's way (`truststore`): a fresh Windows has few root certificates for
    Python, and the first-start model download from GitHub failed there (CERTIFICATE_VERIFY_FAILED).
  - `labels.py` — labeling engine: per-face labels (`LabelDB`), auto-naming, duplicate-photo detection, undo history
    and clues for unnamed faces (`hints`: closest people, photo tags, same occasion). The rest of the engine is split
    into modules that `labels` re-exports (`labels.X` still works):
    - `photofiles.py` — content hashes, picture fingerprints, online-only cloud files, camera dates, photo folders.
    - `photofacts.py` — `PhotoFacts`: camera dates + people tags (XMP, Google Takeout), saved in `data/photo_facts.json`.
    - `relocate.py` — `migrate` (one-time upgrade to per-face labels) and `relocate` (finds photos that moved, e.g. a
      changed drive letter, at start).
    - `importer.py` — background photo/video `Importer` (videos: `face_id.video_faces`, ~1 frame/s, each person
      followed and their 3 clearest faces kept; frame numbers in `photos.json` `at`); one job at a time
      (`Importer.begin`); unreadable files kept with the reason (`note_problem`, shown by the page's **Why?**).
      Each file's work that doesn't touch the database (read once: hash and picture from the same bytes, decode,
      faces, possible faces, a whole video) runs on `hardware.import_workers()` threads; the import thread only adds
      the results. Each thread has its own OpenCV detector (`Faces._detector`); GPU runs go through `ACCEL_LOCK`.
    Possible faces: each photo is also read with a low detector cut-off (`Importer.MORE_SCORE` 0.45, 4 sizes; sampled,
    not measured for accuracy); those faces are marked in `photos.json` `maybe` and listed apart from New faces.
  - `label_tool.py` + `label_ui.html` (layout) + `label_ui.js` (script) / `label.bat`, `label.sh` — labeling web app on
    `http://127.0.0.1:8765`.
    The owner calls this "the recognizer".
  - `tracker.py` + `tracker_ui.html` + `tracker_ui.js` / `recognize.bat`, `recognize.sh` — live tracker on `http://127.0.0.1:8766`.
    Its **Collect faces** option saves each collected face (never the whole frame: the face with `CAPTURE_MARGIN`
    0.6 face widths around it, the room `face_id.rebuild_crop` re-detects in, other faces greyed out) + the tracker's
    verdict to `%APPDATA%\FaceLabels\webcam\` (`face_id.WEBCAM`); `label_tool.py` imports them by itself and shows
    the verdict as a clue.
  - `expression.js` — expression voting, state model, confidence flags, evidence, per-person usual look (browser;
    served by `tracker.py`). Tests: `node --test tests/*.test.js` from `face-recognizer\`.
  - `signals.js` — motion, talking, blinks, gaze, smile timing, webcam pulse, body posture, microphone voice features,
    each person's normal for every signal (`Baselines`, alone and in company),
    feeling-report summaries, fitted valence/arousal (browser; served by `tracker.py`; tests in `tests/`).
  - `app.py` / `build_app.bat` → `FaceRecognizer.exe` (ports 8764-8766, or `FACE_PORTS`=n for n to n+2: a second copy) — one window (pywebview, WebView2) with Face tracking,
    Labeling and Settings tabs; starts both servers in one process (`tracker.bind/attach`, `label_tool.attach`), each
    still on its own port and page. The tab page itself (`app_ui.html` + `app_ui.js`, `{{VERSION}}` filled in) is
    served on `127.0.0.1:8764` (`/status`, `/update`, `/open-release`, `/paths`, `/open`, `/restart`: Start again for a program that failed or stopped). Before the window opens, `clear_graphics_caches` removes the window's
    compiled graphics caches once per new version (a stale one froze a newer Qt WebEngine: white window), never Local Storage. PyInstaller one-file build; uses
    `models\` and `data\` beside the .exe (`face_id.ROOT`); `VERSION` in `app.py` is the release number.
  - `websearch.py` — the labeling tool's **Web search**: Amazon Rekognition celebrity recognition + Google Cloud Vision web detection, combined into suggestions with a confidence level (on click only; US services, English results, each capped under $10/month; keys in `%APPDATA%\FaceLabels`, never in the repo).
  - `describe.py` — the labeling tool's **Describe photo**: a vision model in LM Studio on this computer
    (`127.0.0.1:1234`, `FACE_LLM_URL`; `reasoning_effort` none, ~3 s a photo) describes the photo and reads its text.
    The Settings tab can choose the describer (automatic / server only / built-in only), the server address (any
    OpenAI-style server, also on another device or the internet, with a warning), the model, an API key and
    `spread` (off by default: Describe all also uses the other LM Studio servers on the LAN with the same model,
    `helpers()`; one photo at a time each, a helper failing twice is left out; no API key goes to a helper):
    `%APPDATA%\FaceLabels\describe.json`, never `data/`;
    saved per photo in `%APPDATA%\FaceLabels\descriptions.json`, never `data/`. Told not to identify anyone.
    Without LM Studio: `florence.py`, the built-in Florence-2 large (ONNX int8, CPU only: DirectML gave garbage;
    ~3 s a photo; plainer and less accurate), 0.8 GB downloaded on request from Hugging Face (pinned revision, SHA-256)
    into `models/florence2/` (not in git, not in `fetch_models.py`).
  - `settings_bridge.js` — the app window's **Settings** tab (`app.py`) reads and changes each page's settings over
    postMessage; each page lists its own (`SettingsBridge.init` at the end of its script; the tab shows sections in
    list order, tracker first: keep related rows in one section and the order in `docs/SETTINGS.md` the same) and applies changes through
    its own controls (`SettingsBridge.control(id, section)`) or its own get/set and actions. Served by both servers.
    Setting types: toggle, range, select, text, multi, info, button, form. The tab owns no settings itself.
  - `localguard.py` — what the three servers share: request checks (Host, `Sec-Fetch-Site`, `X-Face-Tool: 1`,
    Origin), the `Handler` base class (quiet, `send`/`send_json`, `body(limit)`, `refused`, the page headers:
    Content-Security-Policy `POLICY` plus each server's `policy`, frame-ancestors, CORP, nosniff; uncaught errors
    answered with a JSON 500; subclasses define `_get`/`_post`) and `bind` (no port sharing on Windows).
  - `alerts.py` — the tracker's alert inbox (`%APPDATA%\FaceLabels\alerts.json`): add, list, acknowledge / archive /
    delete, and each alert's measured details (numbers and short text only, checked by `check_details`); routes
    `/api/alerts`, `/api/alerts/state`, `/api/alerts/details` in tracker.py; the bell and banner in the app window
    come from the tracker page over the Settings bridge (`fs: "alerts"`), and app.py's `/attention` sets the title
    and flashes the taskbar button.
  - `persondata.py` — one person's data: `export` (a folder with `data.json` and their pictures) and `forget`
    (every place a person's data lives, listed at its top; their faces stay as Ignored places). The labeling tool's
    person view (**Their data…**, `/api/person/export`, `/api/person/forget`); the tracker page forgets its browser
    storage itself (`forgetPerson`, sent by the app's tab page after a forget).
  - `desktop.py` — what the app asks of the operating system on Windows, macOS and Linux: show a folder, draw the
    eye to the window for an alert. `paths.user_data` gives each system's application-data folder.
  - `setup.sh`, `start.sh`, `label.sh`, `recognize.sh`, `FaceRecognizer.command` — macOS and Linux: a `.venv` with the pinned libraries (Linux
    adds `pywebview[qt]` for the app window), the models, and starting the app (`--browser` for the web browser).
  - `packaging/macos/` — the app's entitlements (camera, microphone) and `sign_and_notarize.sh`, which CI runs when
    the signing secrets are set (MACOS_CERT_P12_BASE64, MACOS_CERT_PASSWORD, APPLE_API_KEY_ID, APPLE_API_ISSUER_ID,
    APPLE_API_KEY_P8_BASE64).
  - `setup.bat` — installs Python 3.12 (winget) and `requirements.txt` (pinned versions; OpenCV as
    `opencv-contrib-python` only, which mediapipe needs; setup removes a plain `opencv-python` if one is installed).
    `requirements-build.txt` adds PyInstaller for `build_app.bat`; `requirements-dev.txt` has pytest and ruff.
  - `build_exe.py` — the one description of the app build (PyInstaller options, page files packed), for the system it
    runs on: `FaceRecognizer.exe` (Windows, one file), `FaceRecognizer.app` (macOS: Info.plist camera/microphone text,
    macOS 14 minimum), the `FaceRecognizer` folder (Linux: leaves the system's graphics libraries out,
    `LINUX_SYSTEM_LIBS`). Run by `build_app.bat` on Windows, by hand on macOS and Linux, and by CI's `build` job on all
    three, which keeps 14 days the `FaceRecognizer-windows-x64`, `-macos-arm64` and `-linux-x64` artifacts (with
    BUILD-INFO.json: commit, run, SHA-256, signed). Release steps: face-recognizer/README.md, "Building the app".
  - `fetch_models.py` — downloads missing models (or Git LFS placeholders): seven from their publishers (OpenCV Zoo on Hugging
    Face, pinned; MediaPipe storage v1; ArcFace's bytes only, by range requests, from InsightFace's `buffalo_l.zip`),
    Wav2Small (and any fallback) from the GitHub Release `models-v1`,
    with SHA-256 checks (also of files already there, remembered by size and time in `models/.verified.json`,
    not in git); run by `setup.bat` and by the .exe on first start. `download_checked` is the pinned download
    (URL at a commit, size, SHA-256) that `tools/get_wav2small.py` and `tools/get_adaface.py` use. `face_id.check_downloaded()` stops
    with "Model files weren't downloaded: install Git LFS and run git lfs pull" when it finds placeholders.
  - `models/` (Git LFS) — YuNet (detect), ArcFace R50 (recognize), SFace (fallback), eDifFIQA (quality),
    expression model, AdaFace (evaluated, unused; local only, made by `tools/get_adaface.py`, not in git),
    `npu/` (static ArcFace + VitisAI config/cache), `face_landmarker.task` (MediaPipe blendshape cues, optional;
    the `.task` files and `wav2small_src/model.safetensors` are plain git files, not LFS: moving them would
    rewrite history),
    `wav2small.onnx` (voice arousal/dominance/valence, optional; made by `tools/get_wav2small.py`),
    `pose_landmarker_lite.task` (MediaPipe body posture, optional; all 33 points drawn with Full body skeleton),
    `hand_landmarker.task` (MediaPipe hands, 21 points each, optional, display only: Hand skeleton; a plain git file
    like the other `.task` files; in `fetch_models.py` from MediaPipe storage only, OPTIONAL until added to models-v1).
  - `tools/` — `eval_models.py`, `hw_bench.py`, `expr_check.py`, `expr_eval.py`, `get_adaface.py`, `get_wav2small.py`, `get_yunet_dynamic.py` (YuNet with free input sizes, for GPU face finding; published in models-v1),
    `import_bench.py`, `import_profile.py` (each step of an import timed on a folder), `websearch_check.py`, `webcam_eval.py` (live naming on confirmed webcam snapshots; measured the
    averaged-print rule the tracker uses via `/api/who`).
  - `tests/` — Node tests (`*.test.js`: `expression.js`, `signals.js`, `settings_bridge.js`, every page script parses
    and is served and packed, no inline handlers, no internet fonts) and Python tests (`test_*.py`, pytest: request
    checks, the three servers' headers and routes on test servers (`test_servers.py`: CSP, CORP, cross-site
    refusals, request sizes, Collect faces, presence log, feeling reports, `/api/who`, no page loading from the
    internet), describe settings and where photos/keys go, Web search keys and caps, the model download against a
    local server, labeling, Ignore and undo, one person's export and forget (`test_persondata.py`, also Describe
    all past an unreadable photo), the face-picture migration, the empty new-user start, and a tripwire on the
    measured thresholds). `conftest.py`
    points `FACE_ROOT`/`FACE_HOME` at a temp folder before anything is imported: no models, no real data.
  - `.github/workflows/checks.yml` — CI on every push/PR: Node tests, ruff and pytest (no LFS, no models). Actions are
    pinned to full commit SHAs, the release in a comment; look new ones up with
    `git ls-remote https://github.com/actions/<name> refs/tags/<tag>`.

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

macOS (14+, Apple Silicon) and Linux (Ubuntu 22.04+), from `face-recognizer/`:
```
./setup.sh                                   # once: .venv, pinned libraries (+ pywebview[qt] on Linux), models
./start.sh                                   # the app window (--browser, or FACE_UI=browser: the web browser)
./recognize.sh                               # live tracker on its own (port 8766)
./label.sh                                   # labeling tool on its own (port 8765)
.venv/bin/python tracker.py                  # or label_tool.py, face_id.py --help: as on Windows
.venv/bin/python -m pip install -r requirements-build.txt && .venv/bin/python build_exe.py   # build the app
```
Platform differences live in `paths.py` (folders), `desktop.py` (open a folder, attention), `app.py` (WebView2
camera hook on Windows only; on Linux the window is first tried in a child process, `--probe-window`, falling back to
the browser) and `face_id.py` (`auto` decides from the hardware: DirectML on Windows; CUDA or CoreML per model after a start-up check against
the CPU, `_gpu_check`; face finding too, `_yunet_runner`, with `DET_DYNAMIC` and a session per input size; `setup.sh`
installs `onnxruntime-gpu[cuda,cudnn]` instead of `onnxruntime` when `nvidia-smi` works). Online-only
cloud files (`photofiles.py`) are handled on Windows only. Ports: `FACE_PORTS=<n>` moves the app window's 8764-8766.

## Checks before committing

```
cd face-recognizer
node --test tests/*.test.js                                # browser code and page scripts
set PY=%LOCALAPPDATA%\Programs\Python\Python312\python.exe
%PY% -m ruff check .                                 # ruff.toml; pyflakes rules plus likely bugs
%PY% -m pytest tests -q                              # Python tests (requirements-dev.txt)
```

CI runs all of these. Keep page code in the `.js` files (no inline `<script>` blocks: a test fails on them), and add
any new page file to its server and to `PAGES` in `build_exe.py` (a test checks both). Change a measured threshold only together
with `tests/test_thresholds.py`, after re-measuring.
Test UI changes in the real app or a test copy, never against the owner's running app:
- The owner's app holds ports 8764-8766. Start a test copy from source on spare ports (tracker/labeling `--port`,
  or `app.start` with a port offset), with `APPDATA` pointed at a temp folder and `face_id.DATA` at an empty or copied
  folder, so it can't touch the real `data/` or `%APPDATA%\FaceLabels`.
- Start a built test copy with `FACE_UI=none` (servers only): `FACE_UI=browser` opens the owner's default browser,
  and that tab says the program stopped once the test copy is killed.
- Headless Edge over CDP works for page checks. Kill leftover `msedge --remote-debugging-port` processes first:
  an old one keeps old tabs (and their errors) alive.
- Importing `face_id` works without models; each program calls `face_id.require_models()` when it starts, which
  stops with `NotDownloaded` (what to run) if they're missing.

## Security rules

- Every endpoint goes through `localguard.refuse_reason` (exact Host and port; POST needs `X-Face-Tool: 1` and its
  own Origin). The `app.py` shell uses it too: its GETs that do something (`/update`, `/open-release`, `/open`, `/restart`) need
  the header.
- Every answer carries `localguard.page_headers`: a Content-Security-Policy with `default-src 'self'` and only what
  each page uses (`localguard.POLICY`; the tracker adds `blob:` scripts and media for its microphone worklet and Open
  video, the tab page the programs' ports as `frame-src`; no inline script anywhere, so no `onclick=` attributes);
  `frame-ancestors` lets only the page itself frame it, plus the app's tab page when `app.py` started the program
  (`localguard.FRAME_PARENTS` is empty until `app.shell_server` adds its real port; a .bat copy trusts nobody, and
  the app says so instead of showing it); the shell page can't be framed at all; `Cross-Origin-Resource-Policy:
  same-origin` and `nosniff`. Pages load nothing from the internet (a test checks every page file).
- Requests with `Sec-Fetch-Site: cross-site` are refused (another website's page, even as an `<img>`); same-site is
  the tab page's frames on other 127.0.0.1 ports. Request bodies go through `Handler.body(limit)` (negative or
  non-numeric `Content-Length` 400, too large 413).
- In the app window only the tracker this window shows gets the camera and microphone without a prompt
  (`app.camera_allowed`).
- Servers bind with `localguard.bind` (no address reuse on Windows, so a second copy fails instead of sharing the
  port) and wrap handlers in `localguard.answer_errors` (an uncaught error still gets a JSON 500).
- Anything put into `innerHTML` goes through `esc()` (names, paths, descriptions, web results, tags, errors).
  The Settings tab builds its rows with `textContent`/DOM properties only.
- Keys (Web search, describe API key) and the shared database's password reach a page only through `/api/secrets`
  (labeling tool; needs `X-Face-Tool: 1`, refused cross-site), for the Settings tab to show them masked, each with an
  eye button that shows it only while held (the owner's choice, 2026-11-11). Every other endpoint returns only
  whether one is saved. The Settings tab never stores them; nothing is written into page HTML or storage. In the
  shared database they exist only encrypted (`secretbox.py`); the passphrase is never stored, only its derived key.
- The shared database is reached only over TLS checked against its own CA (`verify-ca`); LAN connections without
  TLS are refused by its pg_hba. The app's role (`facerec`) is not a superuser and owns only its schema.
- Only the two fixed folders may be opened (`/open?what=data|private`); photos are addressed by face index, never by path.
- Downloads are pinned and checked by size and SHA-256 (`fetch_models.py`, `florence.py`, `cudasupport.py`); the CUDA
  download starts only from its Settings button, and only `onnxruntime/` and `nvidia/` are unpacked from the wheels.
- `settings_bridge.js` answers only its parent window, on `127.0.0.1`/`localhost`, and only when that is the origin
  the browser reports for the frame's parent (`location.ancestorOrigins`).
- `lmstudio.py` changes another program's settings (LM Studio's `networkInterface` to 0.0.0.0, restarting its server
  on all interfaces) and announces it on the local network: the owner's rule for every computer with this app and LM
  Studio (2026-11-10); `FACE_LMSTUDIO=off` turns it off. Model and CUDA downloads start only from a Settings button.
- `hardware.ai_apps` asks 127.0.0.1's usual AI ports and the configured describe server (with that server's own key,
  through `describe.server_models`). `hardware.lmstudio_network` (Settings → LM Studio on the network, only while the
  tab is open, cached a minute) takes announced servers from mDNS (`mdns_services`: `_lmstudio._tcp`, or `_http._tcp`
  with `app=LM Studio`; their own port) and tries port 1234, this computer's LM Studio port and announced ports on every
  address of the /24 of each real interface
  (`own_addresses`: no loopback, containers or VPN tunnels), reads each server's model list (`/api/v0/models`), and
  names computers by mDNS reverse lookup or NetBIOS name status; nothing is sent but those questions.
- Describe photo: the API key belongs to its server address (changing the address clears it; Test sends it only to
  that address); redirects aren't followed; answers are capped (`MAX_ANSWER`, `MAX_TEXT`).
- Background jobs (imports, possible-face scans, Describe all) start through `Importer.begin` (one at a time, under
  a lock). Shared caches and counters used by request threads need a lock.
- Every person's data can be exported and forgotten (`persondata.py`): a new place that keeps anything about a person
  must be added to its list, `export` and `forget`, and to `tests/test_persondata.py`.
- Open items from the 2026-11-09 review: `PROJECT_CONTEXT.md` section 10.

## Conventions and constraints

- Keep `face-recognizer/README.md` in sync with any user-visible change, and this file with layout or rule changes.
- A new setting goes in `docs/SETTINGS.md` too (its section, default, what it is for; and in "What reaches the
  network" if it sends anything anywhere). A new environment variable goes in its table there.
- A new option on the tracker or labeling page should also be listed in that page's `SettingsBridge.init` list,
  so it shows in the Settings tab. Options the user changes should be remembered (`remember`/`store.set`).
- Thresholds are **measured values** — don't change them without re-measuring on `data/`:
  `face_id.py` (`REC_MODELS[...]["match"/"cluster"]`, `Q_ENROLL`, `Q_EXPR`, `MAX_TURN`),
  `labels.py` (`AUTO`, `MARGIN`, `TOPK`, `DUP_BITS`), `tracker_ui.html` (strictness default 0.35),
  and `expression.js` (`X_SURE`, `X_MARGIN`, `X_MIN_VOTES`, vote windows).
  Also measured, for speed (2026-11-10, Ryzen 7 7840HS + RTX 4060 and Ryzen AI Max+ 395 + Radeon 8060S; numbers in
  PROJECT_CONTEXT.md): `face_id.py` `GPU_TOLERANCE`, `DET_PAD`, `DET_SESSIONS`, `hardware.import_workers`. A GPU path
  must give the same faces as the CPU path: re-run the comparison (regular faces identical) when changing them.
- Not yet measured, so evidence only: `expression.js` `H_STAY` (state model) and every `signals.js` constant
  (`TALK_JAW_SD`, pulse, voice). The options that let them change the label (skip while talking, state model)
  stay off by default until `tools/expr_eval.py` shows they help on posed sessions. Same for the hand-on-face skip.
- `data/` is local only: the whole folder is in .gitignore and was purged from git history and releases
  (2026-11-09). Never commit anything from it, or personal details (names, paths, accounts) in docs. A fresh
  copy starts with an empty `data/` and every feature must work from there; `Store.thumb(i)` may be None.
- Labels live per face in `data/labels.json` (keyed by permanent face ID); change them through the
  labeling tool or `LabelDB`, not by hand-editing JSON. `cluster` group numbers are temporary.
- After switching recognition model: `python face_id.py reembed`.
- Name overlay belongs on the face near the top of the head (owner preference).
- The labeling tool stays a separate program from the tracker (owner preference): separate code, server and page.
  Since 2026-11-08 they can share one window as two tabs (`app.py`), but each still runs on its own.
- Camera opening on Windows: try `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`, retry `cap.read()` while the webcam warms up.
- Face-shape cues are evidence only until fusion is measured with `tools/expr_eval.py` on posed sessions.
- Expression is display-only and never stored (except opt-in posed test sessions and feeling reports in `data/expr_eval/`,
  numbers only); microphone audio is analysed live and never stored; per-person baselines and usual looks are kept by
  the tracker in `HOME/person_signals.json` (never `data/`) with a copy in the page's storage, and in the shared
  database when one is set up; keep README wording honest ("looks", not "feels").
- Times are stored in UTC everywhere (files, file names, the database: `paths.utc_now`, `paths.utc_stamp`,
  timestamptz with the session in UTC) and shown in local time only on the pages; `paths.parse_time` also reads
  the older local-time text.
- Describe photo is local by default. Pointing it at another device (Settings) sends each described photo there; the
  Settings tab must say so (`describe.where`). Descriptions and `describe.json` live in `%APPDATA%\FaceLabels`.
- Releases: `FaceRecognizer-win64.zip` (`FaceRecognizer.exe` and a new-user `README.txt`),
  `FaceRecognizer-macos-arm64.zip`, `FaceRecognizer-linux-x64.tar.gz` and `SHA256SUMS.txt`, from one CI run's
  builds of the tagged commit; never `data\`. The macOS app is signed and notarized only once the Apple secrets
  are set; say so in the notes until then.
- The presence log (who was in view, when) is opt-in, names and times only, in `%APPDATA%\FaceLabels\presence\`, never `data/`.
- Webcam faces named automatically don't enter the recognition gallery until confirmed (`LabelDB.gallery`).
- Ignore deletes the face's print and picture (`LabelDB.forget_faces`, `Store.forget`; Undo restores them from memory
  in the same session); only its ID, photo and box stay, which keep the photo from being read again. Faces ignored
  before v1.9 are converted at the labeling tool's start. Code that uses prints must skip `printless` faces.
- Collect faces never saves whole frames, only each face with room around it and the others greyed out.
- InsightFace ArcFace and audEERING Wav2Small weights are non-commercial; note this if the project is ever redistributed.
