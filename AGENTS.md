# Facial-Recognition — guide for coding assistants and contributors

This is the one guide for anyone changing the code (Claude Code reads it through `CLAUDE.md`; Codex and others read it
directly). Keep it current when the layout, conventions or checks change.

Local face recognition for a USB webcam: learn specific people from a photo library, name/ignore faces
in a labeling tool, and show names + match confidence (+ optional expression) on faces live in the
browser. Everything runs on this machine; no data leaves it, except a face or photo the owner sends
on purpose with the labeling tool's **Web search** (face to Amazon, photo to Google; never automatic), photos sent
to be described when the owner points **Describe photo** at a server on another device (Settings), and, once the
owner sets one up, everything private going to their own **shared database** (PostgreSQL + pgvector over TLS, so
several devices share names, labels and private data; optional, off for a new copy). With a shared database the
devices also show each other original photos and, when switched on, their live cameras, directly over the local
network (`peer.py`).

Goals, key decisions and research value: [`README.md`](README.md).
Full background, history, measured tuning values and gotchas: **see [`PROJECT_CONTEXT.md`](PROJECT_CONTEXT.md).**
User-facing docs: [`face-recognizer/README.md`](face-recognizer/README.md).
Every setting, environment variable and what reaches the network: [`docs/SETTINGS.md`](docs/SETTINGS.md).
Why expression reading is layered (the science): [`docs/WHY_LAYERED_EXPRESSION.md`](docs/WHY_LAYERED_EXPRESSION.md).
Expression design review (LM Studio export): [`Real-Time Facial Emotion Recognition.md`](Real-Time%20Facial%20Emotion%20Recognition.md).

Names used below: `ROOT` is the folder with `models/` and `data/` (`paths.ROOT`); `HOME` is the private per-user
folder (`paths.HOME`: `%APPDATA%\FaceLabels` on Windows, each system's application-data folder elsewhere). `data/`
is shared by all programs; `HOME` holds keys, settings and everything private that isn't the photo library.

## Layout

### Top level

- `docs/` — `SETTINGS.md` (every setting, default, purpose, environment variables, what reaches the network; keep it
  in step with each page's `SettingsBridge.init` list) and background write-ups (`WHY_LAYERED_EXPRESSION.md`).
- `face-tracker/` — `index.html`, the original standalone MediaPipe tracker (double-click to open), and its README. It
  uses the internet (jsDelivr, Google model storage and fonts) and isn't part of the offline tool; its MediaPipe main
  file is pinned with an import-map `integrity` (SHA-384).
- `face-recognizer/` — the Python project, described below.
- `.github/workflows/checks.yml` — CI on every push/PR, three jobs:
  - `checks`: Node tests, ruff and pytest (no LFS, no models).
  - `database`: the shared-database tests (`pytest -k "facedb or sync"`) against a pgvector service, with
    `FACE_TEST_PG`.
  - `build`: the app on Windows, macOS and Linux (`build_exe.py --no-copy`, macOS signing when its secrets are set);
    keeps the `FaceRecognizer-windows-x64`, `-macos-arm64` and `-linux-x64` artifacts 14 days, each with
    `BUILD-INFO.json` (commit, run, SHA-256, signed).
  - Actions are pinned to full commit SHAs, the release in a comment; look new ones up with
    `git ls-remote https://github.com/actions/<name> refs/tags/<tag>`.

### The app and its servers

- `app.py` / `build_app.bat` → `FaceRecognizer.exe` — one window (pywebview: WebView2 on Windows, Qt WebEngine on
  Linux) with the Face tracking, Labeling, Cameras and Settings tabs. `VERSION` in `app.py` is the release number.
  - Starts both programs in one process (`tracker.bind`/`attach`, `label_tool.attach`), each still on its own port
    and page. Ports 8764-8766 (tab page, labeling, tracker); `FACE_PORTS=<n>` uses n to n+2 (a second copy).
  - The tab page itself (`app_ui.html` + `app_ui.js`, `{{VERSION}}` filled in) is served on `127.0.0.1:8764`:
    `/status`, `/paths`, `/update`, `/open-release`, `/open`, `/attention` (alert count in the title, taskbar flash),
    `/restart` (Start again for a program that failed or stopped; one loaded in this process and only slow is never
    loaded a second time: `OWN`), `/alive`, `/restart-app`.
  - Window watchdog: the tab page asks `/alive` every 2 s; `watch_window` (`watch_step`) stops the page renderer
    after `HUNG_AFTER` (30) s of silence (Linux with Qt: Qt loads the page again; the macOS window is WebKit, with no such renderer to stop), and starts the whole app again in
    (`restart_app`) if it stays silent `RESTART_AFTER` (30) s more, or on Windows. Linux, macOS: `os.execve` in place
    (same pid, ports freed). Windows has no real exec: a new copy is started with `FACE_RESTART_AFTER=<pid>` and
    waits for the old one to end before taking the ports (`wait_for_previous`). Restarts in a row back off
    (`window_restart_due`: `WINDOW_BACKOFF` 2 min, doubling, at most an hour; `HOME/restarted.json` `window`).
    A gap of more than 20 s between its own checks means the computer slept: it starts counting again.
    `recover_renderer` reloads a page whose renderer died.
  - A camera that won't start even after a page reload: the tracker page tells the tab page (`fs: "camstuck"`),
    which asks `/restart-app`; `restart_soon` restarts the app at most once every `RESTART_EVERY` (an hour,
    `HOME/restarted.json`), and only in the app window.
  - `clear_graphics_caches`: before the window opens, removes its compiled graphics caches once per new version (a
    stale one froze a newer Qt WebEngine: white window); never Local Storage.
  - `KEEP_RUNNING_FLAGS`: the window's browser runs pages unthrottled in the background (the camera stream needs it),
    and on Windows keeps camera frames in ordinary memory (`MediaFoundationD3D11VideoCapture` off: reading them back
    from the graphics processor stalled the page in a disconnected desktop session).
  - `allow_more_open_files`: Linux, macOS: the app and its window's processes may have 8192 files open (systemd
    starts programs with 1024).
  - Without a console (the built app) messages go to `HOME/app.log`, each line stamped with the UTC time (`Stamped`;
    `tests/test_app_log.py`).
  - On Linux the window is first tried in a child process (`--probe-window`); without a working window the app uses
    the web browser. `FACE_UI=browser` (or `--browser`) opens the browser instead; `FACE_UI=none` opens nothing.
- `build_exe.py` — the one description of the app build (PyInstaller options, page files packed in `PAGES`), for the
  system it runs on: `FaceRecognizer.exe` (Windows, one file), `FaceRecognizer.app` (macOS: Info.plist
  camera/microphone text, macOS 14 minimum), the `FaceRecognizer` folder (Linux: leaves the system's graphics
  libraries out, `LINUX_SYSTEM_LIBS`). Run by `build_app.bat` on Windows, by hand on macOS and Linux, and by CI's
  `build` job. Uses `models/` and `data/` beside the app (`face_id.ROOT`). Release steps: face-recognizer/README.md,
  "Building the app".
- `localguard.py` — what the three servers share:
  - request checks (`refuse_reason`: Host, `Sec-Fetch-Site`, `X-Face-Tool: 1`, Origin);
  - the `Handler` base class (quiet, `send`/`send_json`, `body(limit)`, `refused`; subclasses define `_get`/`_post`;
    uncaught errors answered with a JSON 500);
  - the page headers (`page_headers`: Content-Security-Policy `POLICY` plus each server's `policy`,
    frame-ancestors, CORP, nosniff);
  - `bind` (no port sharing on Windows).
- `settings_bridge.js` — the app window's **Settings** tab reads and changes each page's settings over postMessage.
  - Each page lists its own (`SettingsBridge.init` at the end of its script). The tab shows sections in list order,
    tracker first: keep related rows in one section, and the order in `docs/SETTINGS.md` the same.
  - Changes go through the page's own controls (`SettingsBridge.control(id, section)`) or its own get/set and actions.
  - Setting types: toggle, range, select, text, multi, info, button, form. The tab owns no settings itself.
  - Pages also send the tab page notices this way (`SettingsBridge.notify`: alerts, `camstuck`). Served by both
    servers.
- `devicesettings.py` + `page_store.js` — what each page remembers, kept by the page's own server
  (`/api/settings`, `/api/signals`): its settings in `HOME/settings.json`, and each person's usual look and signal
  normals in `HOME/person_signals.json`. `page_store.js` restores them into localStorage first thing (one synchronous
  request) and sends each change; localStorage is the page's quick copy. With a shared database both are synced
  (settings per device, person signals shared).
  - Person signals go per person: the page sends only the people it changed and the names it removed
    (`merge_signals`; never its whole map, which dropped everyone another device had learned since the page loaded),
    and every half minute takes in what other devices learned or forgot (`PageStore.signals.refresh`,
    `GET /api/signals?v=`). A forgotten name is refused until it is given to someone again.
- `desktop.py` — what the app asks of the operating system on Windows, macOS and Linux: show a folder, draw the eye
  to the window for an alert. `paths.user_data` gives each system's application-data folder.
- `paths.py` — `ROOT` (`FACE_ROOT` overrides), `HOME` (`FACE_HOME` overrides), `atomic_write`/`replace_file`,
  `read_json` (a file that is there but can't be read raises instead of counting as empty: for files that are
  rewritten whole), `utc_now`/`utc_stamp`/`parse_time`. Every module takes its folders from here. Importing it also makes HTTPS check
  certificates the operating system's way (`truststore`): a fresh Windows has few root certificates for Python, and
  the first-start model download from GitHub failed there (CERTIFICATE_VERIFY_FAILED; `tests/test_certificates.py`).

### Recognition and labeling

- `face_id.py` — core library + CLI (`scan`, `cluster`, `name`, `people`, `recognize`, `reembed`, `migrate`):
  models, face finding, prints, `Store` (faces, pictures), `require_models`/`check_downloaded`, and where each model
  runs (`auto`, `_gpu_check`, `_yunet_runner`; see Running). `SAVED_HOOKS` are called after each save.
- `labels.py` — labeling engine: per-face labels (`LabelDB`), auto-naming, duplicate-photo detection, undo history and
  clues for unnamed faces (`hints`: closest people, photo tags, same occasion). It re-exports the modules split from it
  (`labels.X` still works):
  - `photofiles.py` — content hashes, picture fingerprints, online-only cloud files (Windows only), camera dates,
    photo folders.
  - `photofacts.py` — `PhotoFacts`: camera dates + people tags (XMP, Google Takeout), in `data/photo_facts.json`.
  - `relocate.py` — `migrate` (one-time upgrade to per-face labels) and `relocate` (finds photos that moved, e.g. a
    changed drive letter, at start).
  - `importer.py` — the background photo/video `Importer`:
    - one job at a time (`Importer.begin`); unreadable files kept with the reason (`note_problem`, the page's
      **Why?**);
    - each file's work that doesn't touch the database (read once: hash and picture from the same bytes, decode,
      faces, possible faces, a whole video) runs on `hardware.import_workers()` threads; the import thread only adds
      the results. Each thread has its own OpenCV detector (`Faces._detector`); GPU runs go through `ACCEL_LOCK`;
    - videos: `face_id.video_faces`, ~1 frame/s, each person followed and their 3 clearest faces kept (frame numbers
      in `photos.json` `at`);
    - possible faces: each photo is also read with a low detector cut-off (`Importer.MORE_SCORE` 0.45, 4 sizes;
      sampled, not measured for accuracy), marked in `photos.json` `maybe` and listed apart from New faces.
- `label_tool.py` + `label_ui.html` (layout) + `label_ui.js` (script) / `label.bat`, `label.sh` — the labeling web app
  on `http://127.0.0.1:8765`. The owner calls this "the recognizer". It also runs the shared-database sync
  (`start_sync`) and serves `/api/hardware`, `/api/downloads`, `/api/secrets`, the person routes and Settings'
  database, LM Studio and CUDA actions. The header's second bar (`#dlprogress`, `/api/downloads`) shows model and
  CUDA downloads.
- `persondata.py` — one person's data: `export` (a folder with `data.json` and their pictures) and `forget` (every
  place a person's data lives, listed at its top; their faces stay as Ignored places). In the person view (**Their
  data…**, `/api/person/export`, `/api/person/forget`); the tracker page forgets its browser storage itself
  (`forgetPerson`, sent by the tab page after a forget); with a shared database every device forgets (db/004).
  The forgotten name is kept (`HOME/forgotten.json`, the `forgotten` table) as the instruction for devices that
  haven't forgotten yet; giving the name to someone again removes it everywhere (`LabelDB.act`/`rename` →
  `dbsync_home.unforget`), so a device joining later doesn't forget that new person.
- `websearch.py` — the labeling tool's **Web search**: Amazon Rekognition celebrity recognition + Google Cloud Vision
  web detection, combined into suggestions with a confidence level. On click only; US services, English results,
  each capped under $10/month (`dbsync_home.count_use` holds the caps across devices); keys in `HOME`, never in the
  repo.
- `describe.py` — the labeling tool's **Describe photo**: a vision model describes the photo and reads its text; told
  not to identify anyone.
  - Default: LM Studio on this computer (`127.0.0.1:1234`, `FACE_LLM_URL`; `reasoning_effort` none, ~3 s a photo).
  - Settings: the describer (automatic / server only / built-in only), the server address (any OpenAI-style server,
    also on another device or the internet, with a warning: `where`), the model, an API key, and `spread` (off by
    default: Describe all also uses the other LM Studio servers on the LAN with the same model, `helpers()`; one
    photo at a time each, a helper failing twice is left out; no API key goes to a helper).
  - Settings in `HOME/describe.json`, descriptions per photo in `HOME/descriptions.json`; never `data/`.
  - Without LM Studio: `florence.py`, the built-in Florence-2 large (ONNX int8, CPU only: DirectML gave garbage; ~3 s
    a photo; plainer and less accurate), 0.8 GB downloaded on request from Hugging Face (pinned revision, SHA-256)
    into `models/florence2/` (not in git, not in `fetch_models.py`).

### Live tracker and cameras

- `tracker.py` + `tracker_ui.html` + `tracker_ui.js` / `recognize.bat`, `recognize.sh` — the live tracker on
  `http://127.0.0.1:8766`. Also serves the Cameras page and the alert inbox.
  - **Collect faces** saves each collected face + the tracker's verdict to `HOME/webcam/` (`face_id.WEBCAM`); never
    the whole frame: the face with `CAPTURE_MARGIN` (0.6 face widths) around it, the room `face_id.rebuild_crop`
    re-detects in, other faces greyed out. `label_tool.py` imports them by itself and shows the verdict as a clue.
  - "Keep this camera running" starts the camera within 10 s whenever it isn't running. A start that hangs 30 s
    counts as failed; after two the page reloads, and if it still hangs the app starts again (`camstuck`, see
    `app.py`).
- `expression.js` — expression voting, state model, confidence flags, evidence, per-person usual look (browser; served
  by `tracker.py`).
- `signals.js` — motion, talking, blinks, gaze, smile timing, webcam pulse, body posture, microphone voice features,
  each person's normal for every signal (`Baselines`, alone and in company), feeling-report summaries, fitted
  valence/arousal (browser; served by `tracker.py`).
- `exprlog.py` — the expression log (the owner's decision, 2026-11-12): what each named person's face looks like,
  every `EXPR_LOG_EVERY` (5 s, `tracker_ui.js`) while a tracker sees them, kept in the shared database for reports
  later (`expression_log`, db/010: device, name, time, label, share, state, and the rest as numbers in `data`).
  - The page posts readings to `/api/expression-log`; they wait in `HOME/expression/<UTC day>.jsonl` until the
    sync has sent them (`exprlog.send` from `HomePart.after_push`; one row per device, name and time, so sending
    twice changes nothing), then a past day's waiting file is deleted: the database is the store.
  - Nothing is written without a shared database, or with **Keep expression readings** off (`ft-o-optExprLog`,
    on by default). Named people only, never Unknown; a forgotten name is refused.
  - `persondata.py` exports (`expression_log.jsonl`) and forgets them.
- `alerts.py` — the tracker's alert inbox (`HOME/alerts.json`): add, list, acknowledge / archive / delete, and each
  alert's measured details (numbers and short text only, checked by `check_details`); routes `/api/alerts`,
  `/api/alerts/state`, `/api/alerts/details` in `tracker.py`. The bell and banner in the app window come from the
  tracker page over the Settings bridge (`fs: "alerts"`); `app.py`'s `/attention` sets the title and flashes the
  taskbar button.
- Live cameras across devices (with a shared database; db/008). Frames stay in memory, never on disk.
  - Sending (`camstream.py`): while "Stream this camera" is on and the peer service runs, the tracker page composes
    the video with its overlay canvas (every marker, mirrored as shown) into JPEG frames (`streamFrame`, `STREAM_FPS` 20 a second, the overlay drawn at most 60 times a second: `DRAW_EVERY`; all the
    time, watched or not: the owner's choice) and posts them to `/api/stream/frame`. Every second it also posts who's
    in view and its readout as data (`/api/inview`, `readoutData`). While the page is hidden it draws on a timer; the
    app window's browser runs unthrottled (`app.KEEP_RUNNING_FLAGS`).
  - Serving (`peer.py`): `/stream` is multipart JPEG with a JSON readout part every `READOUT_EVERY` (1) s between the
    frames, so no device needs a request per camera for readouts; `/readout` answers one readout on its own.
  - Relaying (`camrelay.py`): one connection per watched device (`peer.open_stream`, never ending on a timer,
    reconnecting at once), keeping that device's newest frame and readout; closed after `IDLE` (60) s unwatched.
  - Watching (`cameras_ui.html` + `cameras_ui.js`, the tracker's `/cameras` page, the app's Cameras tab): every
    streaming device in a grid with its readout, drawn with text only, and an events fly-out (alerts from every
    device; closed state remembered, its button counting new events). A browser allows six connections per address,
    so the page's requests never grow with the number of cameras:
    - one `/api/cameras/frames?after=id:n,...` at a time answers every camera's new frames at once
      (`camera_frames`: waits on `camstream.ANY` for the first, gathers 40 ms more; body: 4-byte length, a JSON head,
      then the JPEGs);
    - one `/api/cameras/readouts?ids=...` a second answers every readout from memory (`camera_readouts`);
    - at most `MAX_CAMERAS` (64) cameras per request. Measured with a stub, at the earlier 10 a second: 40 cameras at 9-10 fps each, at most 3
      requests open.
  - "Moved" alerts: `HomePart._cameras` keeps `sightings` and raises "moved" when someone is on this camera within
    `MOVE_WINDOW` (10 min) of being seen on another's. Sightings are opt-in like the presence log: only while that alert
    is on, only people on the device's alert list (`ft-alertWho`); rows older than `KEEP_SIGHTINGS` (20 min) are deleted.

### Shared database and devices

- `facedb.py` + `db/NNN_*.sql` — the optional shared database (PostgreSQL + pgvector on a server the devices share;
  since v1.16). Without `HOME/database.json` (or `FACE_DB`) nothing changes: files only.
  - Connection settings in `HOME/database.json` (password DPAPI-protected on Windows, mode 600 elsewhere); TLS
    `verify-ca` against the server's own CA (made by `tools/db_certs.py`). This device's id in `HOME/device.json`.
  - `facedb.setup` (Settings → Shared database → Connect) logs in with sslmode=require and
    channel_binding=require, takes the server's CA from the `server_ca` table (the SCRAM login, bound to this TLS
    connection, proves the server at its other end knows the password) and keeps it for verify-ca.
  - Migrations run under an advisory lock; a `changes` log is kept by trigger (deletes as tombstones). Connections
    are autocommit: group writes in `with conn.transaction():`.
  - Migrations: `001_schema` (tables, change log), `002_sync` (database `instance` id, photo facts),
    `003_server_ca`, `004_forgotten` (people forgotten on any device), `005_picture_sha` (send a face picture only
    when it differs), `006_passphrase_versions` (`secretbox.py`), `007_peers` (`peer.py`: addresses, port,
    certificate hash), `008_cameras` (`devices.streaming_at`, `sightings`), `009_change_order` (`changes.txid`: the
    writing transaction), `010_expression_log` (`exprlog.py`).
- `dbimport.py` + `tools/db_import.py` — copy a device's `data/` and HOME into the database (reads only; merges by
  face ID and photo hash; a hand label beats an automatic one; Ignored stays Ignored). Join uses it.
- `dbsync.py` — the sync, in the labeling tool's process (`label_tool.start_sync`). Files stay the working copy.
  - Each row's digest as last synced is in `HOME/sync.sqlite`. Changed rows are pushed after each save
    (`face_id.SAVED_HOOKS` → `Sync.note`) and every minute; other devices' changes are pulled from `changes` every
    3 s and applied through the stores (so `people.npz` is rebuilt and the tracker reloads by mtime).
  - Which changes are new is decided by transaction, not by `seq` (`Sync._pull`, db/009): the pull keeps the
    snapshot it read the log with and next time takes every change whose transaction wasn't visible in it
    (`pg_visible_in_snapshot`). By `seq` alone, rows of a long transaction (a join) that committed after a device
    had read past their numbers were never pulled.
  - Join (first sync with a database, by its `instance` id): dbimport, then everything pulled.
  - Parts: `Local` (photos, faces, labels, Web search answers; the undo log only sent) and `dbsync_home.HomePart`
    (descriptions, Collect faces pictures, alerts, presence, shared documents, keys, forgotten people, device
    settings, person signals; the app log, posed tests and the expression log only sent). `HomePart.every_step` also runs the peer
    service (`_peer`), camera sightings (`_cameras`) and the passphrase catch-up.
  - A new place that keeps data must get a table (new `db/NNN_*.sql`) and a part.
- `secretbox.py` — keys in the database, encrypted on the device (scrypt + AES-256-GCM, name as associated data).
  - Every passphrase is a version (`passphrase_versions`, db/006: check value, fingerprint, who/when; one current).
  - Each version is sealed to every device's X25519 public key (`passphrase_shares`; private half in
    `HOME/device-key.json`, protected), so setting, changing (re-encrypts every key in one transaction) or restoring
    on one device reaches all. `HomePart.every_step` takes the current one (`catch_up`) and registers fingerprints.
  - A device not on the current version neither sends nor reads keys (`push` returns False: not marked sent).
  - The passphrase and its key are kept in `HOME/secret.key` (protected) so Settings shows it masked.
- `peer.py` — the peer service: devices show each other their original photos (db/007) and live cameras (db/008).
  - A device that lacks an original asks one that has it (the `photo_paths` table), which answers with the photo
    view's JPEG; shown, never stored.
  - Listens on the LAN: port = tab page port + 3 (8767; `FACE_PEER_PORT`; `FACE_PEER=off` or the Settings switch
    turn it off). TLS with the device's own certificate (`HOME/peer-cert.pem`), whose SHA-256 is in
    `devices.peer_cert_sha256` and checked by the asker.
  - Requests signed (HMAC) with the cluster key (HKDF of the database password and its instance id), valid
    `CLOCK_SKEW` (120) s; photos by hash only (`PHOTO_RE`), from the device's own records.
  - truststore replaces `ssl.SSLContext` app-wide, so `peer.py` uses the original class (`_context`). Started by
    `HomePart._peer`.

### Hardware, models and LM Studio

- `hardware.py` — what the computer has and how the programs use it; the labeling tool's `/api/hardware` and
  `/api/hardware/network`, shown in Settings → This computer and → LM Studio.
  - `describe`: processor, memory, GPUs, NPU, ONNX Runtime providers (from the registry, /proc and /sys, sysctl).
  - `uses`: `face_id.LAST_DEVICES`, `GPU_DECISIONS`, `import_workers`.
  - `ai_apps`: AI model apps for Describe photo (the usual local ports, the configured describe server, installed
    apps, LM Studio's downloads, the LAN address when served to the network; cached a minute).
  - `lmstudio_network`: every LM Studio server on the local network (name, addresses, port, models).
- `cudasupport.py` — NVIDIA CUDA for the built Linux app (built with plain ONNX Runtime). `FACE_CUDA=off` turns it off.
  - `check`: driver 525.60+, compute capability 5.2+.
  - `offer`: Settings → This computer's **Download CUDA support**, never automatic.
  - `start_download` (`/api/cuda/download`): onnxruntime-gpu and NVIDIA's cu12 wheels from PyPI, pinned by size and
    SHA-256, unpacked into `ROOT/cuda`.
  - `activate` (at app start): a meta-path finder imports `onnxruntime`/`nvidia` from there ahead of the bundled
    copy; falls back to it if that fails.
- `lmstudio.py` — LM Studio on this computer, kept as the owner wants on every computer with this app and LM Studio
  (`keep()`, every minute from `app.py`; `FACE_LMSTUDIO=off` turns it off).
  - `serve_everywhere`: its `http-server-config.json` `networkInterface` 0.0.0.0; a localhost-only server is
    restarted with `lms server start --bind 0.0.0.0 --cors` (CORS on, as the owner's LM Studio servers run; if that
    start fails, it is started again as it was, never left off).
  - `announce`: mDNS `_lmstudio._tcp` + `_http._tcp` with its port (Windows `DnsServiceRegister`, macOS
    `dns-sd -R`, Linux `avahi-publish-service`); skipped when the computer announces it already.
  - `missing_models` / `get_models`: vision models other servers on the network have, from Settings → LM Studio's
    button: first from the shared models folder (`from_share`, checked by SHA-256, removed again if a file fails;
    paths in the folder's `index.json` that would leave LM Studio's models or hub folder are never used: `_inside`),
    else `lms get <model>@<quant>`. Stopping `lms get` doesn't stop LM Studio's own download: it keeps a paused job
    that hides the model until cancelled in LM Studio.
  - `publish`: puts this computer's vision models in the shared folder (`<share>/models/...`, `<share>/hub/...`,
    `<share>/index.json` with each file's size and SHA-256). The folder is this computer's setting in
    `HOME/lmstudio.json`.
- `fetch_models.py` — downloads missing models (or Git LFS placeholders), with SHA-256 checks (also of files already
  there, remembered by size and time in `models/.verified.json`, not in git); run by `setup.bat`/`setup.sh` and by
  the app on first start.
  - Eight from their publishers (OpenCV Zoo on Hugging Face, pinned; MediaPipe storage v1; ArcFace's bytes only, by
    range requests, from InsightFace's `buffalo_l.zip`); Wav2Small, the dynamic YuNet (and any fallback) from the GitHub Release
    `models-v1`.
  - `download_checked` is the pinned download (URL at a commit, size, SHA-256) that `tools/get_wav2small.py` and
    `tools/get_adaface.py` use.
  - `face_id.check_downloaded()` stops with "Model files weren't downloaded: install Git LFS and run git lfs pull"
    when it finds placeholders.
- `models/` (Git LFS, except the `.task` files and `wav2small_src/model.safetensors`, which are plain git files:
  moving them would rewrite history):
  - YuNet (detect), ArcFace R50 (recognize), SFace (fallback), eDifFIQA (quality), the expression model;
  - `npu/` (static ArcFace + VitisAI config/cache);
  - `face_landmarker.task` (MediaPipe blendshape cues, optional);
  - `wav2small.onnx` (voice arousal/dominance/valence, optional; made by `tools/get_wav2small.py`);
  - `pose_landmarker_lite.task` (MediaPipe body posture, optional; all 33 points drawn with Full body skeleton);
  - `hand_landmarker.task` (MediaPipe hands, 21 points each, optional, display only: Hand skeleton; in
    `fetch_models.py` from MediaPipe storage only, OPTIONAL until added to models-v1);
  - local only, not in git: AdaFace (evaluated, unused; made by `tools/get_adaface.py`) and `florence2/`
    (`florence.py`). The Linux app's downloaded CUDA is in `ROOT/cuda`, beside `models/`.

### Setup, tools and tests

- `setup.bat` — installs Python 3.12 (winget) and `requirements.txt` (pinned versions; OpenCV as
  `opencv-contrib-python` only, which mediapipe needs; setup removes a plain `opencv-python` if one is installed).
  `requirements-build.txt` adds PyInstaller for `build_app.bat`; `requirements-dev.txt` has pytest and ruff.
- `setup.sh`, `start.sh`, `label.sh`, `recognize.sh`, `FaceRecognizer.command` — macOS and Linux: a `.venv` with the
  pinned libraries (Linux adds `pywebview[qt]` for the app window), the models, and starting the app (`--browser`
  for the web browser).
- `packaging/macos/` — the app's entitlements (camera, microphone) and `sign_and_notarize.sh`, which CI runs when the
  signing secrets are set (MACOS_CERT_P12_BASE64, MACOS_CERT_PASSWORD, APPLE_API_KEY_ID, APPLE_API_ISSUER_ID,
  APPLE_API_KEY_P8_BASE64).
- `tools/` — measuring and one-off scripts:
  - models and speed: `eval_models.py`, `hw_bench.py`, `import_bench.py`, `import_profile.py` (each step of an
    import timed on a folder);
  - expression: `expr_check.py`, `expr_eval.py` (posed sessions);
  - `webcam_eval.py` (live naming on confirmed webcam snapshots; measured the averaged-print rule the tracker uses
    via `/api/who`), `websearch_check.py`;
  - making models: `get_adaface.py`, `get_wav2small.py`, `get_yunet_dynamic.py` (YuNet with free input sizes, for
    GPU face finding; published in models-v1);
  - shared database: `db_certs.py` (the server's CA and certificate), `db_setup.py` (once per new server: the
    tables and `server_ca`, over verify-ca with that CA), `db_import.py`.
- `tests/` — `conftest.py` points `FACE_ROOT`/`FACE_HOME` at a temp folder before anything is imported: no models, no
  real data.
  - Node (`*.test.js`): `expression.js`, `signals.js`, `settings_bridge.js`, `page_store.js` (person signals sent
    per person, `refresh`); `pages.test.js`: every page script
    parses and is served and packed, no inline handlers, no internet fonts.
  - Servers and security: `test_servers.py` (the three servers on test servers: CSP, CORP, cross-site refusals,
    request sizes, Collect faces, presence log, feeling reports, `/api/who`, no page loading from the internet),
    `test_localguard.py`, `test_certificates.py`.
  - Labeling and people: `test_labels.py` (naming, Ignore and undo, rename/merge, the empty new-user start, the
    face-picture migration), `test_persondata.py` (one person's export and forget; Describe all past an unreadable
    photo and spread among LM Studio servers), `test_self_attributes.py`.
  - Services and downloads: `test_describe.py`, `test_websearch.py` (keys, caps), `test_fetch_models.py` (against a
    local server), `test_cudasupport.py`, `test_lmstudio.py`, `test_hardware.py`, `test_gpu_choice.py`.
  - App window and tracker: `test_window_watchdog.py`, `test_window_caches.py`, `test_app_log.py`,
    `test_tracker_reload.py`, `test_skeleton.py`, `test_platforms.py`.
  - Shared database: `test_facedb.py`, `test_sync.py`, with `FACE_TEST_PG` (CI job `database`; each test makes and
    drops its own schema).
  - `test_thresholds.py`: a tripwire on the measured thresholds.

## Running

```
face-recognizer\setup.bat        # once
face-recognizer\recognize.bat    # live tracker  (port 8766)
face-recognizer\label.bat        # labeling tool (port 8765)
face-recognizer\build_app.bat    # builds FaceRecognizer.exe: both programs in one window, with tabs
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
Platform differences live in:
- `paths.py` (folders) and `desktop.py` (open a folder, attention);
- `app.py` (WebView2 camera hook on Windows only; on Linux the window is first tried in a child process,
  `--probe-window`, falling back to the browser; the watchdog stops a hung renderer on Linux and macOS only);
- `face_id.py`: `auto` decides from the hardware: DirectML on Windows; CUDA or CoreML per model after a start-up check
  against the CPU (`_gpu_check`); face finding too (`_yunet_runner`, with `DET_DYNAMIC` and a session per input
  size). `setup.sh` installs `onnxruntime-gpu[cuda,cudnn]` instead of `onnxruntime` when `nvidia-smi` works; the
  built Linux app gets CUDA from `cudasupport.py`;
- online-only cloud files (`photofiles.py`) are handled on Windows only.

Ports: `FACE_PORTS=<n>` moves the app window's 8764-8766 (and so the peer port, n+3).

## Checks before committing

```
cd face-recognizer
node --test tests/*.test.js                                # browser code and page scripts
set PY=%LOCALAPPDATA%\Programs\Python\Python312\python.exe
%PY% -m ruff check .                                 # ruff.toml; pyflakes rules plus likely bugs
%PY% -m pytest tests -q                              # Python tests (requirements-dev.txt)
```

CI runs all of these (the database tests only with its PostgreSQL service; locally set `FACE_TEST_PG` to run them).
Every `fetch` in a page script must read its answer (`json()`, `text()`, `arrayBuffer()`) or let it go (`drop(r)`,
`r.body?.cancel()`): an unread answer keeps a file descriptor open in the page's process, and on Linux the window
froze when they ran out (ten minutes after a start, found 2026-11-12; `tests/pages.test.js` checks every fetch).
Keep page code in the `.js` files (no inline `<script>` blocks: a test fails on them), and add any new page file to
its server and to `PAGES` in `build_exe.py` (a test checks both). Change a measured threshold only together with
`tests/test_thresholds.py`, after re-measuring.
Test UI changes in the real app or a test copy, never against the owner's running app:
- The owner's app holds ports 8764-8767. Start a test copy from source on spare ports (tracker/labeling `--port`,
  or the app with `FACE_PORTS`), with `FACE_HOME` (or `APPDATA`) pointed at a temp folder and `FACE_ROOT` (or
  `face_id.DATA`) at an empty or copied folder, so it can't touch the real `data/` or `HOME`.
- Start a built test copy with `FACE_UI=none` (servers only): `FACE_UI=browser` opens the owner's default browser,
  and that tab says the program stopped once the test copy is killed.
- Headless Edge over CDP works for page checks. Kill leftover `msedge --remote-debugging-port` processes first:
  an old one keeps old tabs (and their errors) alive.
- Importing `face_id` works without models; each program calls `face_id.require_models()` when it starts, which
  stops with `NotDownloaded` (what to run) if they're missing.

## Security rules

- The programs' servers listen on 127.0.0.1 only. The one exception is the peer service (`peer.py`, with a shared
  database, on by default there, Settings switch): LAN, TLS with the device's pinned certificate, HMAC-signed
  requests from the cluster key, original photos by content hash only, never a path or anything else; and
  the device's live camera as its tracker page shows it (`/stream`) with its readout (`/readout`, and as JSON parts
  in `/stream`), only while "Stream this camera" is on.
- Every endpoint goes through `localguard.refuse_reason` (exact Host and port; POST needs `X-Face-Tool: 1` and its
  own Origin). The `app.py` shell uses it too: its GETs that do something (`/update`, `/open-release`, `/open`,
  `/attention`, `/restart`, `/restart-app`: `acts` in `shell_handler`) need the header.
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
  The Settings tab and the Cameras page's readouts are built with `textContent`/DOM properties only.
- On disk, keys and passwords are kept with `paths.protect` (DPAPI on Windows, mode 600 elsewhere: `write_private`);
  read them through their module (`websearch.saved_key`/`aws`, `describe.config`, `facedb.saved_login`), never
  the file.
- Keys (Web search, describe API key) and the shared database's password reach a page only through `/api/secrets`
  (labeling tool; needs `X-Face-Tool: 1`, refused cross-site), for the Settings tab to show them masked, each with an
  eye button that shows it only while held (the owner's choice, 2026-11-11). Every other endpoint returns only
  whether one is saved. The Settings tab never stores them; nothing is written into page HTML or storage. In the
  shared database they exist only encrypted (`secretbox.py`); the passphrase stays on each device (protected).
- The shared database is reached only over TLS checked against its own CA (`verify-ca`); LAN connections without
  TLS are refused by its pg_hba. The app's role (`facerec`) is not a superuser and owns only its schema.
- Only the two fixed folders may be opened (`/open?what=data|private`); photos are addressed by face index, never by path.
- Downloads are pinned and checked by size and SHA-256 (`fetch_models.py`, `florence.py`, `cudasupport.py`); the
  CUDA download starts only from its Settings button, and only `onnxruntime/` and `nvidia/` are unpacked from the
  wheels. `lmstudio.from_share` checks each file against the shared folder's own list (a damaged copy, not a
  rewritten list: the folder is as trusted as the computers that write to it) and writes only inside LM Studio's
  models and hub folders, whatever paths the list holds.
- `settings_bridge.js` answers only its parent window, on `127.0.0.1`/`localhost`, and only when that is the origin
  the browser reports for the frame's parent (`location.ancestorOrigins`).
- `lmstudio.py` changes another program's settings (LM Studio's `networkInterface` to 0.0.0.0, restarting its server
  on all interfaces with CORS on) and announces it on the local network: the owner's rule for every computer with this app and LM
  Studio (2026-11-10); `FACE_LMSTUDIO=off` turns it off. Model and CUDA downloads start only from a Settings button.
- `hardware.ai_apps` asks 127.0.0.1's usual AI ports and the configured describe server (with that server's own key,
  through `describe.server_models`). `hardware.lmstudio_network` (Settings → LM Studio on the network, only while the
  tab is open, cached a minute):
  - takes announced servers from mDNS (`mdns_services`: `_lmstudio._tcp`, or `_http._tcp` with `app=LM Studio`;
    their own port);
  - tries port 1234, this computer's LM Studio port and announced ports on every address of the /24 of each real
    interface (`own_addresses`: no loopback, containers or VPN tunnels);
  - reads each server's model list (`/api/v0/models`) and names computers by mDNS reverse lookup or NetBIOS name
    status; nothing is sent but those questions.
- Describe photo: the API key belongs to its server address (changing the address clears it; Test sends it only to
  that address); redirects aren't followed; answers are capped (`MAX_ANSWER`, `MAX_TEXT`).
- What other computers on the network say is checked before use: an LM Studio server's model list by shape
  (`hardware._lms_models`), a model's name against `lmstudio.VARIANT` before it reaches `lms get`, paths in the
  shared models folder's list (`lmstudio._inside`).
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
  (2026-11-09). Never commit anything from it, or personal details (names, paths, accounts, host names, addresses) in
  docs or code. A fresh copy starts with an empty `data/` and every feature must work from there; `Store.thumb(i)`
  may be None.
- Labels live per face in `data/labels.json` (keyed by permanent face ID); change them through the
  labeling tool or `LabelDB`, not by hand-editing JSON. `cluster` group numbers are temporary.
- After switching recognition model: `python face_id.py reembed`.
- Name overlay belongs on the face near the top of the head (owner preference).
- The labeling tool stays a separate program from the tracker (owner preference): separate code, server and page.
  Since 2026-11-08 they can share one window as tabs (`app.py`), but each still runs on its own.
- Camera opening on Windows: try `CAP_DSHOW` → `CAP_MSMF` → `CAP_ANY`, retry `cap.read()` while the webcam warms up.
- Face-shape cues are evidence only until fusion is measured with `tools/expr_eval.py` on posed sessions.
- Expression readings are stored, as numbers and short labels only (the owner's decision, 2026-11-12, for reports
  later; before that they were never stored): in the shared database's `expression_log` (`exprlog.py`; nothing
  without a shared database), in each alert's measured details (`alerts.py`), and in opt-in posed test sessions
  and feeling reports (`data/expr_eval/`). Never a picture or sound with them. Microphone audio is analysed live
  and never stored; per-person baselines and usual looks are kept by
  the tracker in `HOME/person_signals.json` (never `data/`) with a copy in the page's storage, and in the shared
  database when one is set up; keep README wording honest ("looks", not "feels").
- Camera frames (streamed or relayed) are kept in memory only, never on disk or in the database.
- Cameras are never paused (the owner's rule, 2026-11-12): the app never stops or slows a camera or its stream by
  itself, whether or not a face is in view, the window is shown, or anyone is watching. Only the owner's own
  Stop camera does. The Cameras page keeps fetching every stream while its tab is hidden; don't add a pause.
- **They move between cameras** (and so camera sightings) is off until switched on.
- Times are stored in UTC everywhere (files, file names, log lines, the database: `paths.utc_now`, `paths.utc_stamp`,
  timestamptz with the session in UTC) and shown in local time only on the pages; `paths.parse_time` also reads
  the older local-time text.
- Describe photo is local by default. Pointing it at another device (Settings) sends each described photo there; the
  Settings tab must say so (`describe.where`). Descriptions and `describe.json` live in `HOME`.
- Releases: `FaceRecognizer-win64.zip` (`FaceRecognizer.exe` and a new-user `README.txt`),
  `FaceRecognizer-macos-arm64.zip`, `FaceRecognizer-linux-x64.tar.gz` and `SHA256SUMS.txt`, from one CI run's
  builds of the tagged commit; never `data\`. The macOS app is signed and notarized only once the Apple secrets
  are set; say so in the notes until then.
- The presence log (who was in view, when) is opt-in, names and times only, in `HOME/presence/`, never `data/`.
- Webcam faces named automatically don't enter the recognition gallery until confirmed (`LabelDB.gallery`).
- Ignore deletes the face's print and picture (`LabelDB.forget_faces`, `Store.forget`; Undo restores them from memory
  in the same session); only its ID, photo and box stay, which keep the photo from being read again. Faces ignored
  before v1.9 are converted at the labeling tool's start. Code that uses prints must skip `printless` faces.
- Collect faces never saves whole frames, only each face with room around it and the others greyed out.
- InsightFace ArcFace and audEERING Wav2Small weights are non-commercial; note this if the project is ever redistributed.
