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
  - `build`: the app on Windows, macOS and Linux (`build_exe.py --no-copy`, macOS signing when its secrets are set;
    Windows signing when its secrets are set and the push is to `main`: `azure/login` as this repository's workflow,
    with no stored password, right before each signing, since that sign-in lasts minutes: `build_exe.py --sign`,
    then after the installer `--signed`, which stops the run unless Windows reads both files as signed, trusted
    and dated),
    the built app's own check (`--self-test`, `app.self_test`: every library loads and works once; on macOS under
    its signature; on Windows also the window's own parts: pywebview's Windows platform with its .NET lines,
    WebView2's library and the types `allow_camera` uses), then its installer (`--installer-only`; the Linux one is tried there: install, icons,
    uninstall; the .dmg signed and notarized too);
    keeps the `FaceRecognizer-windows-x64`, `-macos-arm64` and `-linux-x64` artifacts 14 days, each with the app,
    its installer and `BUILD-INFO.json` (commit, run, both files' SHA-256, signed).
  - Actions are pinned to full commit SHAs, the release in a comment; look new ones up with
    `git ls-remote https://github.com/actions/<name> refs/tags/<tag>`.

### The app and its servers

- `app.py` / `build_app.bat` → `FaceRecognizer.exe` — one window (pywebview: WebView2 on Windows, Qt WebEngine on
  Linux) with the Face tracking, Labeling, Cameras and Settings tabs. `VERSION` in `app.py` is the release number.
  - Starts the labeling tool in its own process (`label_tool.attach`) and, since 1.25, the tracker as a second
    process: this program again with `--tracker-child <port> <this pid> <tab page port> <Cameras page port>`
    (`start_tracker`, `tracker_child`, `tracker.serve_child`). Each on its own port and page. Ports 8764-8766 (tab
    page, labeling, tracker) and 8769 (the Cameras page, below); `FACE_PORTS=<n>` uses n to n+2 and n+5 (a second
    copy). `FACE_TRACKER=inline` keeps the tracker in the app's process, as before (`tracker.bind`/`attach`).
    - Why: one Python process runs one thread at a time. With the labeling tool kept busy (its summary asked for
      by 4 threads) the tracker answered a tiny question in 83 to 97 ms instead of 3, and 204 of 221 answers
      came later than 50 ms; in its own process 3 ms and none late, the camera at the other devices 20 frames a
      second throughout (measured on a copy of a real library and in the PC's app, 2026-11-19). It costs 0.85 GB
      more committed memory (2.8 to 3.7 GB), 50 MB more in use.
    - The app checks that what answers on the tracker's port is the process it started (`trackerlink.hello`: an
      answer only a holder of this run's secret can give), puts it in `OWN` (a `Child`), and watches it
      (`watch_tracker`): ended, or running and silent for `CHILD_SILENT` (30 s), it is ended and started again on
      the same port, at most `CHILD_TRIES` (5 times in 10 minutes). The tracker page keeps its camera through
      that (measured in the app: answering again after 7 s, the camera at another device again after 19 s; a
      process suspended whole: replaced after 35 s, on Linux 36). Start again for a tracker that doesn't answer
      does the same.
    - It is ended with the app (`stop_tracker` before every `os._exit` and before `restart_app`), and ends itself
      when the app's process is gone (`tracker.watch_parent`). It writes the same `app.log` (or console), has its
      own `watch_interpreter`, and is counted apart in the memory line.
    - While it is being started again, the tracker page's camera request is still answered (`TRACKER["again"]`
      in `camera_allowed`).
    - The Cameras tab's page has a port of its own in the tracker's process (`CAMS_PORT`: the tab page's port + 5,
      8769; `tracker.also_serve`: the same handler and the same memory on a second address of 127.0.0.1). The
      window gives each address a process of its own, and the Cameras page was the tracker's address: kept busy,
      it took the tracker's page from 57 frames read a second to 1; on its own port 55.7 (WebView2) and 57.5 (Qt
      WebEngine; test copies, 2026-11-19). The app asks for the port over the link (`trackerlink.cameras_port`)
      and gives the tab page the address in `/status` (`cameras`; `app_ui.js` `camsUrl`). A port that is taken,
      `FACE_TRACKER=inline` and the tracker run by itself leave the page on the tracker's own port, as before.
      The port must stay one of `TOOL_PORTS` (the tab page's `frame-src`; a test), and its frame is handed
      neither camera nor microphone.
    - The tab page looks at `/status` every 5 s and follows a program (and the Cameras page) to a changed
      address: a tracker started again on another port was otherwise left behind in its frame.
    - Tried on Windows and Linux, from source and as the built app; not yet on a Mac.
    A program already answering on a port that this process didn't start (`OWN`) is never shown in the window:
    `start` says it is already running (its .bat, another window, or anything answering like it).
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
  - Each tab's page has a process of its own on Windows and Linux (`localguard.page_headers`), so a hung tab no
    longer stops the tab page. The tab page asks every loaded tab every 2 s whether it runs (`{ fs: "ping" }`,
    answered by `settings_bridge.js`) and names those silent for `ANSWER_IN` (10 s) in `/alive?silent=track,cams`
    (`app_ui.js` `silentTabs`): `alive_said` then doesn't count the window as running, and the watchdog does what
    it does for a window that says nothing; the log names the tab (`not_answering`).
  - A camera that won't start even after a page reload: the tracker page tells the tab page (`fs: "camstuck"`),
    which asks `/restart-app`; `restart_soon` restarts the app at most once every `RESTART_EVERY` (an hour,
    `HOME/restarted.json`), and only in the app window. Under a locked screen the rule is once per lock
    instead (see the tracker's locked-screen bullet).
  - `watch_interpreter`: what a stuck process was doing. The watchdog is a thread of the process, so when the
    interpreter itself stops (a library call that never returns with the interpreter's lock held) it stops too.
    `faulthandler`'s timer needs no interpreter: set again every quarter of `STUCK_AFTER` (60 s), it goes off only
    when the interpreter hasn't run that long, and writes every thread's stack to the log (a block beginning
    "Timeout (0:01:00)!"). It must never go off while the interpreter runs: writing the stacks every 20 s crashed
    a busy test copy (it reads them without the lock). Don't shorten it, don't make it repeat.
  - `note_memory`: every `MEMORY_EVERY` (30 min) the log gets what the window's processes hold (`window_memory`:
    this process's descendants; Windows and Linux), so a page that keeps growing shows before it runs out of memory.
  - `preload_window_parts`: Windows: loads the window's four .NET parts by their full names (then by their
    files) before pywebview asks for them by short name. That short-name load failed at some first starts of a
    new build ("Could not load file or assembly 'System.Threading'" in 1.24.0's tests, "'System.Reflection'"
    in 1.24.10's and 1.25.0's): no window, and the app in the web browser. Why it fails is not known; a failed
    load is remembered for the process's life, so the same name can't be asked again. If the window still fails
    that way it is tried once more. Each failure and what then worked is logged.
  - `drop_request_hooks` (`unhook_requests`): Windows: takes pywebview's two request handlers off the window once
    its page has loaded. They passed every request of the pages through Python, and the second switched on the
    browser's network inspection, which keeps a record of every request while the page lives: the page grew 1.5
    to 2 GB an hour until 1.23.0. A test trips when a new pywebview hooks requests another way.
  - `clear_graphics_caches`: before the window opens, removes its compiled graphics caches once per new version (a
    stale one froze a newer Qt WebEngine: white window); never Local Storage.
  - `allow_camera` (`camera_hook`): Windows: the window answers the tracker's camera and microphone request itself.
    WebView2 names the page around a frame as the one asking, so the request arrives as the tab page's
    (`camera_allowed`: the tracker's address or the tab page's own, `SHELL`). Through 1.23.0 only the tracker's
    address was answered: on a new copy WebView2 showed its own question, nobody answered it, and the camera never
    started (found on a fourth device, 2026-11-16; the others had the answer remembered in their profiles). The
    answer is put in place before the window is shown (`before_show`, then `CoreWebView2InitializationCompleted`),
    and the log says each request and its answer.
  - `KEEP_RUNNING_FLAGS`: the window's browser runs pages unthrottled in the background (the camera stream needs it),
    and on Windows keeps camera frames in ordinary memory (`MediaFoundationD3D11VideoCapture` off: reading them back
    from the graphics processor stalled the page in a disconnected desktop session).
  - `keep_running_macos` (`mac_keep_running`): the same for the macOS window, which is WebKit and takes no such
    flags: WebKit's own switches, set on the window's web view before any page runs, each only where this WebKit
    has it (private names: `MAC_OCCLUSION`, `MAC_HIDDEN_PAGES`), and said in the log. With another window over the
    app's, a page's timers ran once a second and nothing was drawn (the camera reached the other devices at 1
    frame a second, the first real Mac, 2026-11-19); with the occlusion check off a covered window counts as
    shown. A hidden or minimized window is only helped part of the way (measured in a counting page: timers 1.3
    a second before, 13.5 to 15.7 after, 36 when shown). The self-test says how many of the switches this macOS has.
  - `allow_more_open_files`: Linux, macOS: the app and its window's processes may have 8192 files open (systemd
    starts programs with 1024).
  - Linux: `OPENBLAS_NUM_THREADS=1` is set at the top of `app.py`, before numpy loads (several OpenBLAS thread
    pools in the packed app got a matrix product in the labeling tool stuck for good).
  - `keep_font_list`: the built app keeps matplotlib's folder in `HOME/matplotlib` (`MPLCONFIGDIR`), set at the top
    of `app.py` before MediaPipe (which imports matplotlib) loads. PyInstaller's start-up hook gives it a new
    temporary folder at every start, so the system's fonts were listed again each time: over 5 s of every start
    on a Mac.
  - Without a console (the built app) messages go to `HOME/app.log`, each line stamped with the UTC time (`Stamped`;
    `tests/test_app_log.py`).
  - On Linux the window is first tried in a child process (`--probe-window`); without a working window the app uses
    the web browser. `FACE_UI=browser` (or `--browser`) opens the browser instead; `FACE_UI=none` opens nothing.
- `build_exe.py` — the one description of the app build (PyInstaller options, page files packed in `PAGES`), for the
  system it runs on: `FaceRecognizer.exe` (Windows, one file), `FaceRecognizer.app` (macOS: Info.plist
  camera, microphone and local-network text, macOS 14 minimum), the `FaceRecognizer` folder (Linux: leaves the system's graphics
  libraries out, `LINUX_SYSTEM_LIBS`). Run by `build_app.bat` on Windows, by hand on macOS and Linux, and by CI's
  `build` job. Uses `models/` and `data/` beside the app (`face_id.ROOT`). Release steps: face-recognizer/README.md,
  "Building the app".
  - `--installer` (after the build) and `--installer-only` (from the app in `build/dist`; CI, after signing) make
    the system's installer (`INSTALLERS`), which puts the app in place and gives it icons, without administrator
    rights:
    - Windows `FaceRecognizer-Setup-x64.exe`: Inno Setup 6 (`packaging/windows/FaceRecognizer.iss`; `inno_setup`
      finds ISCC). Into `%LOCALAPPDATA%\Programs\FaceRecognizer` or a folder the user picks (the app writes
      `models\` and `data\` beside itself, so never Program Files); Start menu and desktop icons. It installs only
      the .exe and never deletes anything of the user's: no `[UninstallDelete]`, no deleting code (a test). It
      closes an open app itself (`CloseApp`: as closing the window does, then by ending it): Windows' Restart
      Manager can't (the .exe's first process has no window), and closed half of it instead.
    - macOS `FaceRecognizer-macos-arm64.dmg`: the app beside a link to Applications (`hdiutil`).
    - Linux `FaceRecognizer-linux-x64.run`: `packaging/linux/install.sh`, the line `__APP__`, then the app's
      folder as .tar.gz (`linux_installer`). `sh` it: the app in `~/.local/lib/FaceRecognizer`, a menu entry, icons
      and a desktop icon; `uninstall.sh` there removes them and leaves `~/.local/share/FaceRecognizer`.
  - Windows code signing (its section in the file; unsigned unless the environment says with what, `signing()`):
    - `FACE_SIGN_ACCOUNT`, `FACE_SIGN_PROFILE`, `FACE_SIGN_ENDPOINT`: Azure Artifact Signing (once Trusted Signing).
      The key stays in Microsoft's service; `signtool` (the Windows SDK's, `signtool()`) asks it through Microsoft's
      plug-in (`sign_dlib`: one version from nuget.org, by size and SHA-256, `SIGN_DLIB`; needs the .NET runtime
      `SIGN_DLIB_NET`), as whoever is signed in to Azure (`NOT_THESE_LOGINS` leaves the environment and the Azure
      CLI). Or `FACE_SIGN_CERT`: a certificate in the user's Windows store, by thumbprint (a certificate
      authority's on its token; a self-signed one to try the build).
    - `--sign` signs the .exe in `build/dist` (a build with signing set signs it itself); the installer's build
      passes Inno Setup `/DSign=1` and a sign tool that is this file's `--sign-file` (`iscc_args`), so Setup signs
      the uninstaller it packs and itself (`#ifdef Sign` in the .iss); `--signed` (`check_signed`, `signature`)
      reads both files' signatures as Windows does. Every signature is dated (`TIMESTAMP`): Artifact Signing's
      certificates last three days.
    - Tried 2026-11-17 with a self-signed certificate (the signed one-file .exe passes its self-test, and the
      installer, the .exe it installs and `unins000.exe` are all signed); CI signs with the real Artifact Signing
      profile since that day (Windows reads both files as signed, trusted and dated). The owner's steps: README,
      "Signing the Windows app". `check_signed` prints the certificate's thumbprint and issuer, never whom it
      names: an individual's certificate carries their name and town, and the log is public.
- `icons/` — the app's icon, drawn by `tools/make_icon.py` (a faceted face, half solid and half mesh, in a camera's
  corner marks; small sizes drawn again with fewer marks) and kept in git: `icon.ico` (the .exe, the installer, each
  server's `/favicon.ico`: `localguard.Handler.favicon`), `icon.icns` (the .app), `icon.png` and two smaller (the
  Linux window, `app.WINDOW_ICON`, and launcher). Packed into every build (`ICONS`). Change the drawing, run the
  script, commit the files.
- `localguard.py` — what the three servers share:
  - request checks (`refuse_reason`: Host, `Sec-Fetch-Site`, `X-Face-Tool: 1`, Origin). A GET for data or one
    that starts work (`OWN_ONLY`: `/api/`, `/thumb/`, `/photo/`; on the tab page everything but its page files)
    is answered only for the tool's own page (`Sec-Fetch-Site: same-origin`) or with the header; a browser's
    `same-site` (a page of another program on this computer) or `none` (an address typed or opened) gets the
    page files only. A new route under another path that gives data needs `own=True` too;
  - the `Handler` base class (quiet, `send`/`send_json`, `body(limit)`, `refused`; subclasses define `_get`/`_post`;
    uncaught errors answered with a JSON 500). It speaks HTTP/1.1 and keeps connections open (since 1.22.2: with
    a new connection per request, about one in a thousand failed on Windows, PROJECT_CONTEXT.md section 10). So
    every answer must say its length: answer through `send`, or call `keep_or_close()` before `end_headers()`
    (`tracker._frames`); a request body that `body()` didn't read closes the connection after the answer. An answer
    leaves in one piece and at once (`wbufsize`, `disable_nagle_algorithm`: as headers then body it waited 40 ms
    for an acknowledgement on Linux). Every answer says `Cache-Control: no-store`, and `send` has no way to say
    otherwise: through 1.24.1 face pictures and photos were cacheable for a year, so the window's cache on disk
    kept the pictures of faces Ignored or forgotten since;
  - the page headers (`page_headers`: Content-Security-Policy `POLICY` plus each server's `policy`,
    frame-ancestors, CORP, nosniff; and a handler's `permissions` as `Permissions-Policy`: the tab page's, for
    camera and microphone). Also `Origin-Agent-Cluster: ?1`: in Chromium's windows (WebView2, Qt WebEngine) each
    server's pages then run in a process of their own, with their own main thread. Without it the tab page and
    all its tabs shared one: a Labeling page kept busy took the tracker from 46 frames read a second to 0.5
    (measured on a test copy, 2026-11-19; 57 with the header). The Cameras page is the tracker's server's too,
    so in the app it is served on a second port (`app.CAMS_PORT`, see `app.py`); WebKit (the macOS window) has
    one process whatever is sent;
  - `bind` (no port sharing on Windows; an accept queue of 128).
- `settings_bridge.js` — the app window's **Settings** tab reads and changes each page's settings over postMessage.
  - Each page lists its own (`SettingsBridge.init` at the end of its script). The tab shows sections in list order,
    tracker first: keep related rows in one section, and the order in `docs/SETTINGS.md` the same.
  - Changes go through the page's own controls (`SettingsBridge.control(id, section)`) or its own get/set and actions.
  - Setting types: toggle, range, select, text, multi, info, button, form. The tab owns no settings itself. A
    button with `confirm` (text; `confirmYes`) is asked about in the tab first (`askUser`).
  - Pages also send the tab page notices this way (`SettingsBridge.notify`: alerts, `camstuck`). Served by both
    servers.
  - It also passes Ctrl+1 to Ctrl+4 pressed inside a tab on to the tab page (`fs: "key"`; a key pressed in a tab
    never reaches the page around it), and answers the tab page's "does this page run?" (`fs: "ping"` → `"pong"`,
    nothing else sent with it: see the watchdog in `app.py`). The Cameras page loads it for these two alone.
- `ask.js` — `askUser(text, yes)`: a yes-or-no question asked in the page itself (a `<dialog>`), served by all
  three servers. Pages never use `confirm()`, `alert()` or `prompt()`: those stop every script of the window (the
  tab page and its tabs share one process), so the camera and its stream stopped and the watchdog took the window
  for frozen (a test keeps them out).
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
  to the window for an alert, whether the screen is locked (`screen_locked`; Windows only, None elsewhere).
  `paths.user_data` gives each system's application-data folder.
- `trackerlink.py` — what the tracker's process and the rest of the app ask of each other, over 127.0.0.1 to the
  other one's own server, under `/api/local/` (never a page's request), with the secret the app makes for each run
  (`FACE_LINK` in the child's environment; `allowed`, `hello`). With the tracker in the app's process (`REMOTE`
  False: `FACE_TRACKER=inline`, the tests, the .bat files) the same functions look into memory as before.
  - The label side asks the tracker: who the camera sees (`in_view`, for the sightings), where its models run
    (`devices`, for Settings → This computer), and the peer service on or off (`peer_service`): the service and
    the camera's heartbeat (`camstream.start_heartbeat`) run where the camera's frames are, in the tracker's
    process, at the sync's word (`HomePart._peer`, `_cameras`). A tracker that doesn't answer (it is starting
    again) means "not shared for this step", never an error: the sync stops sharing for good on an OSError.
  - The tracker asks the label side: a photo another device asked its peer service for (`photo` →
    `/api/local/photo/<hash>`, by content hash only), and "something was saved" (`note_saved` →
    `/api/local/saved`: an alert or a visit goes out at once, not at the sync's next look a minute later).
  - A new piece of the tracker's memory that the other side needs goes through here, with its route in both
    servers and in `OTHER_CALLERS` (`tests/test_routes.py`).
- `paths.py` — `ROOT` (`FACE_ROOT` overrides), `HOME` (`FACE_HOME` overrides), `atomic_write`/`replace_file`,
  `rows_json` (the large files that are rewritten whole, photos, labels and descriptions: one line per entry,
  through the fast encoder; `indent=` takes Python's slow one),
  `read_json` (a file that is there but can't be read raises instead of counting as empty: for files that are
  rewritten whole), `utc_now`/`utc_stamp`/`parse_time`, `SharedLock` (one thread of one process at a time: a
  threading lock and a lock file in `HOME/locks/`; for the files the tracker's process and the rest of the app both
  read, change and write back: the alert inbox, the settings and person signals, the waiting logs, the device id,
  the peer certificate. A new file of that kind takes one, not a `threading.Lock`). Every module takes its folders from here. Importing it also makes HTTPS check
  certificates the operating system's way (`truststore`): a fresh Windows has few root certificates for Python, and
  the first-start model download from GitHub failed there (CERTIFICATE_VERIFY_FAILED; `tests/test_certificates.py`).
  On Linux it also sets how glibc hands out memory (`give_memory_back`, `MALLOC`: 8 arenas, a block of 8 MB or
  more straight from the system and back to it when freed, at most 16 MB kept free at an arena's top), before any
  thread starts. The values are measured (memory level, no slower a frame): change them only with the measurement. Left alone, glibc kept freed memory in up to 128 arenas, and the app's own process grew to 6 to
  8 GB with 1.6 GB in use (PROJECT_CONTEXT.md section 10). Not when `GLIBC_TUNABLES` already sets `glibc.malloc.`.

### Recognition and labeling

- `face_id.py` — core library + CLI (`scan`, `cluster`, `name`, `people`, `recognize`, `reembed`, `migrate`):
  models, face finding, prints, `Store` (faces, pictures), `require_models`/`check_downloaded`, and where each model
  runs (`auto`, `_gpu_check`, `_yunet_runner`; see Running). `SAVED_HOOKS` are called after each save.
  - A graphics device that goes while the program runs (a driver installed or reset under it): every model's GPU
    runner hands over to the CPU (OpenCV) at its first failure and stays there for the rest of the process
    (`_or_cpu` for recognition, quality and expression; `Faces._detect_at` for face finding), the log and
    Settings → This computer say so ("CPU (OpenCV, after a GPU error)"), and the failed runner goes into `_LOST`
    and is never let go of. Two apps hung on 2026-11-19 right after letting go of face finding's sessions, and
    until 1.24.9 the other models had no way to the CPU: every frame was answered with an error for good. Never
    drop or replace a GPU session that failed; a new GPU runner needs the same hand-over.
- `labels.py` — labeling engine: per-face labels (`LabelDB`), auto-naming, duplicate-photo detection, undo history and
  clues for unnamed faces (`hints`: closest people, photo tags, same occasion). Nobody can be named a word in
  `NOT_NAMES` (`constructor`, `__proto__`, ...: the pages keep people's data in objects by name).
  - What it keeps between changes, so a change costs what it touches and not the whole library (measured on
    16,000 faces: 1.3 to 2.5 s after every label, Ignore and imported photo before, 0.13 to 0.4 s now):
    - every face's score for every person (`_columns`: one table, a row a face, a column a person, while faces x
      people stays under `SCORES_KEPT`). Scored again: a person whose usable faces changed, for every face; a face
      that is new or whose print changed, for every person. What changed is told from the prints themselves
      (`E` compared with the `E` the table was made from), so no change has to remember to say so;
    - for a person with more than `BIG` usable faces, each face's `TOPK` closest of them too (`_closest_kept`):
      a face that comes is compared with every face, a face that goes is looked for among the closest;
    - the stacks (`stacks`, `_stacked`): new faces, and faces whose print changed with the stacks they were in.
    A test asks after each kind of change that all of it is what working everything out again gives
    (`SCORES_KEPT = -1` scores the old way). Change any of it only with that test, and with the same answers on
    a real library (the side-by-side comparison: PROJECT_CONTEXT.md section 10).
    Also kept: the photos that have people tags at all (`tagged`, `_tag_photos`: 19 of 11,000 in one library, and
    every change went through all of them), and the groups of unknown faces through an Ignore and its undo
    (`_forgot(regroup=False)`: the face leaves its group and comes back into it).
  - The rest of the engine is in the modules split from it; `labels` imports only what it uses of them and what
    other code asks for as `labels.X` (a name nobody uses that way is taken from its own module):
  - `photofiles.py` — content hashes, picture fingerprints, online-only cloud files (Windows only), camera dates,
    photo folders.
  - `photofacts.py` — `PhotoFacts`: camera dates + people tags (XMP, Google Takeout), in `data/photo_facts.json`.
    At every start each photo's saved facts are checked against the photo and the files beside it (`key`); those
    are found in the folder's sorted names (`_beside`), never by going through the folder's list for each photo:
    that was a minute of pure Python at every start once the Collect faces folder held 10,000 snapshots (each
    has its verdict .json beside it), growing with the square of the folder, and the sync's first step and the
    pages waited for the interpreter meanwhile. Work on every photo at start must stay linear in the library.
  - `relocate.py` — `migrate` (one-time upgrade to per-face labels) and `relocate` (finds photos and videos that
    moved, e.g. a changed drive letter, at start).
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
  on `http://127.0.0.1:8765`. The owner calls this "the recognizer". Its page keeps the face pictures and photos
  it has shown in memory (`PICS`: blob: URLs; `thumb()`, `photoSrc()`), since no answer may be cached, and lets go
  of them on an action, Undo and a forget (`dropPic`, `forgetPics`). It also runs the shared-database sync
  (`start_sync`) and serves `/api/downloads` and the person routes. The header's second bar (`#dlprogress`,
  `/api/downloads`) shows model and CUDA downloads.
  - Its request handler (`make_handler`) answers the labeling page's own routes and hands the rest on:
    - `label_settings.py` — what the Settings tab asks of the labeling tool (`get`, `post`: LM Studio, this
      computer, the shared database and its passphrase, the keys, Music, the downloads it can start;
      `/api/secrets`, `db_state`). The handler asks it first, and nothing in it is answered under the labeling
      lock: it asks the network, the database (on the kept connection, `facedb.ask`) and other programs.
    - `describejob.py` — `DescribeJob`: Describe all and Describe everything again (`start`, `items`, `due`,
      `again_state`, `HELPER_RETRY`, `PEER_MISSES`), run as the Importer's one background job.
  - The Web search answers (`db.web_saved`) are guarded by the labeling data's own lock (`db.web_lock` is
    `db.lock`): the sync reads and writes them under that one.
- `forgotten.py` — the names of forgotten people (`HOME/forgotten.json`): `names` (`strict=True` for the sync: a
  file that can't be read raises instead of counting as no names), `note`, `unforget`, under one lock. It imports
  only `paths`, so `labels`, `devicesettings`, `exprlog`, `musiclog`, `persondata` and `tracker` import it at
  their top; the `forgotten` table's sync stays in `dbsync_home.HomePart`.
- `persondata.py` — one person's data: `export` (a folder with `data.json` and their pictures) and `forget` (every
  place a person's data lives, listed at its top; their faces stay as Ignored places). In the person view (**Their
  data…**, `/api/person/export`, `/api/person/forget`); the tracker page forgets its browser storage itself
  (`forgetPerson`, sent by the tab page after a forget); with a shared database every device forgets (db/004).
  The forgotten name is kept (`HOME/forgotten.json`, the `forgotten` table) as the instruction for devices that
  haven't forgotten yet; giving the name to someone again removes it everywhere (`LabelDB.act`/`rename` →
  `forgotten.unforget`), so a device joining later doesn't forget that new person.
- `websearch.py` — the labeling tool's **Web search**: Amazon Rekognition celebrity recognition + Google Cloud Vision
  web detection, combined into suggestions with a confidence level. On click only; US services, English results,
  each capped under $10/month (`dbsync_home.count_use` holds the caps across devices); keys in `HOME`, never in the
  repo.
- `describe.py` — the labeling tool's **Describe photo**: a vision model describes the photo and reads its text; told
  not to identify anyone.
  - Default: LM Studio on this computer (`127.0.0.1:1234`, `FACE_LLM_URL`; `reasoning_effort` none, ~3 s a photo).
  - Settings: the describer (automatic / server only / built-in only), the server address (any OpenAI-style server,
    also on another device or the internet, with a warning: `where`), the model, an API key, and `spread` (off by
    default: Describe all also uses the LM Studio servers of the owner's other devices that have the same model,
    `helpers()`: only a computer in the shared database, at an address it gave there (`device_addresses()`), or
    this computer; never whatever else answers like LM Studio on the network, and nobody without a shared
    database; one photo at a time each, a helper failing twice is left out and tried again every `describejob.HELPER_RETRY`
    (300) s; no API key goes to a helper).
  - Settings in `HOME/describe.json`, descriptions per photo in `HOME/descriptions.json`; never `data/`.
  - **Describe everything again** (Settings; `/api/describe/again`, `describejob.DescribeJob.start("fresh" | "on")`): the
    same job over its `items()`: every photo and every video frame a face was found in (`h:frame`), of the whole
    library; what isn't on this computer is asked of the device that has it (`peer.fetch`; after `PEER_MISSES`
    unreached in a row, no more in that run). Each description is replaced as its new one arrives. The run's start
    and model are kept in `describe.json` (`describe.again()`): done means described since then, and it stays until
    nothing is left, so a stopped run is carried on with (`carry_on`).
  - Without LM Studio: `florence.py`, the built-in Florence-2 large (ONNX int8, CPU only: DirectML gave garbage; ~3 s
    a photo; plainer and less accurate), 0.8 GB downloaded on request from Hugging Face (pinned revision, SHA-256)
    into `models/florence2/` (not in git, not in `fetch_models.py`). A file already there counts only with its
    SHA-256 too (`florence.good`, read once for the file as it is: `.verified.json` there).

### Live tracker and cameras

- `tracker.py` + `tracker_ui.html` + `tracker_ui.js` / `recognize.bat`, `recognize.sh` — the live tracker on
  `http://127.0.0.1:8766`. Also serves the Cameras page and the alert inbox.
  - The page's JPEGs (the frame sent to be read, the camera's picture for the other devices) are made by `jpegOf`:
    in Chromium's windows in a worker (`jpeg_worker.js`, served by the tracker, started by the page), elsewhere and
    when the worker fails with `canvas.toBlob`. Chromium makes toBlob's picture when the page has idle time, and
    with none only after a second: a page 96% busy read faces and sent its camera once a second (13 ms a picture
    idle, 1,100 ms then; the worker 10 and 25 ms; the same bytes; WebView2 157 and Qt WebEngine, 2026-11-19).
    WebKit doesn't wait (4 ms on the Mac however busy), so the macOS window keeps toBlob. Never call `toBlob` for
    a picture of the camera outside `jpegOf` (a test). A worker that fails three times in a row (or whose script
    doesn't load) is left for a minute and then tried again with a new one, twice as long later after each further
    failure, at most half an hour: a graphics reset takes its canvas for a moment, and "off until the next start"
    would bring toBlob's wait back.
  - **Collect faces** saves each collected face + the tracker's verdict to `HOME/webcam/` (`face_id.WEBCAM`); never
    the whole frame: the face with `CAPTURE_MARGIN` (0.6 face widths) around it, the room `face_id.rebuild_crop`
    re-detects in, other faces greyed out. `label_tool.py` imports them by itself and shows the verdict as a clue.
  - "Keep this camera running" starts the camera within 10 s whenever it isn't running. Running means its picture
    arrives (`stream && running && video.readyState >= 2`), never just that it was opened: a camera opened but not
    playing was once left like that for six hours. Failed tries: a start that hangs 30 s, a camera on with no
    picture for 30 s (two of these), or starts refused at once (six: a minute). Then the page reloads, and if the
    camera still isn't running the app is asked to start again (`camstuck`, see `app.py`; asked at most every
    `STUCK_EVERY`). Not for a camera that isn't there or isn't allowed (`camAbsent`: "not found" counts only when
    no camera is listed); a busy one gets the reload only. Each step goes to the log through
    `/api/page-note` (`tracker.page_note`: printable text, 200 characters, `NOTES_AN_HOUR`).
  - A camera that is on but sends no new picture is paused (`camPaused`; Windows does this to every program
    about 20 s after the screen is locked, and the video keeps its last picture). Told from the browser's count of frames
    (`getVideoPlaybackQuality().totalVideoFrames`) standing still while a 32x18 sample of the picture is the very
    same three times in a row (`stillCheck`, 6 s; a flat picture is left to the flat check), or the track being
    muted. While it lasts no face is read from the still picture, the page and the log say so, and the other
    devices are told with the readout (`paused` seconds in `/api/inview`, `camstream.readout`; the Cameras page
    shows "paused · no new picture for …" though the frames keep arriving). After 30 s the camera is started
    again, and no more than that.
  - A locked screen on Windows (`/api/screen`, `desktop.screen_locked`: the session's flags). Every Remote
    Desktop disconnect locks the session too. Locked by a disconnect, the running app doesn't get the camera
    back (a start from the page fails at once) but the app started again has it within seconds; locked at the
    computer's own screen no start finishes until the unlock. Windows calls both "locked", so: a camera
    paused under a lock is stopped and started at once (no 30 s wait); when that fails or doesn't finish the
    page asks the app to start again, once (`lockAsked`, `fs: "camstuck"`), and `app.restart_soon` does it
    once per lock, whatever its hourly rule says (`restarted.json` `camera.locked`, cleared by `unlock_seen`
    when the watchdog sees the screen unlocked); after that one start is left open (`startOut`) and the page
    is never loaded again against a lock. Before 1.23.0 a disconnect left the last picture streamed as live
    until the app was next started (once for 21 hours).
- `expression.js` — expression voting, state model, confidence flags, evidence, per-person usual look (browser; served
  by `tracker.py`).
- `signals.js` — motion, talking, blinks, gaze, smile timing, webcam pulse, body posture, microphone voice features,
  each person's normal for every signal (`Baselines`, alone and in company), feeling-report summaries, fitted
  valence/arousal (browser; served by `tracker.py`).
- `waitlog.py` — `WaitLog`: what the expression log and the music log share. Rows wait in
  `HOME/<folder>/<UTC day>.jsonl` until the sync has sent them (`send`, from `HomePart.after_push`: one row per
  device, name and time, so sending twice changes nothing; a past day's file is deleted once all of it is sent),
  and are read, dropped for a forgotten person and exported there. Each log's own module keeps what a row is
  (`check`), its switch, the forgotten-names check before `add`, and its table. Files wait on the devices: the
  folder, file names, lines and the sync state's keys (`xlog:`, `mlog:`) stay as they are.
  - A forget rewrites a waiting file and leaves a mark beside it (`<file>.again`): the next send starts at the
    file's beginning. Through 1.24.4 only a file shorter than what was sent counted as rewritten; with rows added
    after the forget it was as long again, and everyone's rows since the forget were lost.
  - A new log of this kind: a `WaitLog` in its own module, a table (`db/NNN_*.sql`), its `send` in `after_push`,
    its rows in `dbsync_home._forget_on_server`, and `persondata.py` with its test.
- `exprlog.py` — the expression log (the owner's decision, 2026-11-12): what each named person's face looks like,
  every `EXPR_LOG_EVERY` (5 s, `tracker_ui.js`) while a tracker sees them, kept in the shared database for reports
  later (`expression_log`, db/010: device, name, time, label, share, state, and the rest as numbers in `data`).
  - The page posts readings to `/api/expression-log`; they wait in `HOME/expression/<UTC day>.jsonl` until the
    sync has sent them (`exprlog.send` from `HomePart.after_push`; one row per device, name and time, so sending
    twice changes nothing), then a past day's waiting file is deleted: the database is the store.
  - Nothing is written without a shared database, or with **Keep expression readings** off (`ft-o-optExprLog`,
    on by default). Named people only, never Unknown; a forgotten name is refused.
  - `persondata.py` exports (`expression_log.jsonl`) and forgets them.
- Music as a signal (since 1.23; off until the owner switches it on, Settings → Music):
  - `music.py` — `now_playing` (the song the Spotify app shows: its window title on Windows, AppleScript on macOS,
    `playerctl` on Linux; nothing asked of the internet); `interfaces` and `listen_all` (every audio interface
    listened to at once through the optional `soundcard` library: each output by loopback, each microphone;
    a Mac has inputs only); `measure` (the sound as numbers: level, dynamics, tempo, brightness, bass, key and
    mode), `combine`, `best` (an output before a microphone); `picture` (a person's usual music from liked songs)
    and `place` (one song against it).
    What a room sounds like: `sounds` (YAMNet through MediaPipe's audio classifier, `models/yamnet.tflite`:
    521 kinds of sound with scores), `families` (those gathered into quiet, music, speech, television or radio,
    steady background noise, activity, alert sounds, animal sounds: `FAMILIES`, by words in the kind's name),
    `ambient` (the background as numbers: floor, events over it, steadiness, flatness, hum, rumble and hiss
    shares, speech-like modulation), `background` and `what` (those in a word, with how sure). The words and
    their cut-offs are not measured against anything: evidence, shown with the score.
  - `musiclog.py` — the music log (`music_log`, db/012): the `Listener` thread (started by the labeling tool) looks
    every 5 s at what plays and listens 8 s every half minute; a song is one row when it ends (artist, song, when,
    seconds, and in `data` what each interface heard). Rows wait in `HOME/music/<UTC day>.jsonl` until
    `HomePart.after_push` sends them. Settings `fl-music` (off), `fl-musicWho` (the person), `fl-musicListen`
    (all | outputs | none). A person's usual music is their person signal of kind `music`
    (`devicesettings.OWN_KINDS`: kept by the programs, never handed to a page). Routes `/api/music`,
    `/api/music/spotify`, `/api/music/liked`, `/api/music/disconnect` in `label_tool.py`.
  - `spotify.py` — the owner's Spotify library, read with their permission: authorization code with PKCE (their
    own client id, no client secret), the one-time answer taken by `wait_for_code` on 127.0.0.1 (port: tab page
    port + 4, 8768), read-only scope, tokens protected in `HOME/spotify.json`. Only on the owner's click.
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
    At most `MAX_RELAYS` (64) at a time: a device id is whatever a request says.
  - Watching (`cameras_ui.html` + `cameras_ui.js`, the tracker's `/cameras` page, the app's Cameras tab, there
    on a port of its own: `app.CAMS_PORT`): every
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
    `verify-ca` against the server's own CA (made by `tools/db_certs.py`). This device's id and name in
    `HOME/device.json` (`device()`: made once; the name is the computer's without its domain, macOS says
    "name.local").
    Settings without a CA (older, or written by hand) connect with channel binding instead, as `setup` does.
  - `facedb.setup` (Settings → Shared database → Connect) logs in with sslmode=require and
    channel_binding=require, takes the server's CA from the `server_ca` table (the SCRAM login, bound to this TLS
    connection, proves the server at its other end knows the password) and keeps it for verify-ca.
  - Migrations run under an advisory lock; a `changes` log is kept by trigger (deletes as tombstones). Connections
    are autocommit: group writes in `with conn.transaction():`.
  - A new connection costs about 0.1 s (TLS and the login). Short questions that only read (who has a photo, where
    a device is, the Cameras page's list of devices) go through `facedb.ask(fn)`: one connection kept for them,
    made again when it has gone or went unused for `KEEP` (60) s.
  - Migrations: `001_schema` (tables, change log), `002_sync` (database `instance` id, photo facts),
    `003_server_ca`, `004_forgotten` (people forgotten on any device), `005_picture_sha` (send a face picture only
    when it differs), `006_passphrase_versions` (`secretbox.py`), `007_peers` (`peer.py`: addresses, port,
    certificate hash), `008_cameras` (`devices.streaming_at`, `sightings`), `009_change_order` (`changes.txid`: the
    writing transaction), `010_expression_log` (`exprlog.py`), `011_key_trust` (`devices.key_proof`,
    `passphrase_versions.prev`/`link`, `key_requests`: who is given the passphrase for keys), `012_music_log`
    (`musiclog.py`).
- `dbimport.py` + `tools/db_import.py` — copy a device's `data/` and HOME into the database (reads only; merges by
  face ID and photo hash; a hand label beats an automatic one; Ignored stays Ignored). Join uses it.
- `dbsync.py` — the sync, in the labeling tool's process (`label_tool.start_sync`). Files stay the working copy.
  - Each row's digest as last synced is in `HOME/sync.sqlite`. Changed rows are pushed after each save
    (`face_id.SAVED_HOOKS` → `Sync.note`) and every minute; other devices' changes are pulled from `changes` every
    3 s and applied through the stores (so `people.npz` is rebuilt and the tracker reloads by mtime).
    `Local.digests` takes only a copy of what the rows are made of under the labeling lock and hashes after it
    (0.5 s for 16,000 faces and 20,000 photos, which the labeling page waited for after every save).
  - Which changes are new is decided by transaction, not by `seq` (`Sync._pull`, db/009): the pull keeps the
    snapshot it read the log with and next time takes every change whose transaction wasn't visible in it
    (`pg_visible_in_snapshot`). By `seq` alone, rows of a long transaction (a join) that committed after a device
    had read past their numbers were never pulled.
  - Join (first sync with a database, by its `instance` id): dbimport, then everything pulled.
  - Parts: `Local` (photos, faces, labels, Web search answers; the undo log only sent) and `dbsync_home.HomePart`
    (descriptions, Collect faces pictures, alerts, presence, shared documents, keys, forgotten people, device
    settings, person signals; the app log, posed tests and the expression log only sent). `HomePart.every_step` also runs the peer
    service (`_peer`), camera sightings (`_cameras`) and the passphrase catch-up.
  - A new place that keeps data must get a table (new `db/NNN_*.sql`) and its class in `dbsync_home.py`: each
    table of the private folder is a `Table` there (`Descriptions`, `Snapshots`, `Alerts`, `Presence`,
    `Documents`, `Secrets`, `Forgotten`, `DeviceSettings`, `PersonSignals`, listed in `TABLE_CLASSES`) with its
    `local`, `push`, `fetch`, `check` and `apply`; `HomePart`, which the sync talks to, only hands each question
    to the table's class. A table neither part has a place for is refused in every function (`HomePart._table`,
    `dbsync._ours`): before, its rows went to each function's last branch, which for `HomePart.apply` was the
    forgotten people (every key a name to forget, on every device) and for `Local` the faces.
  - Rows from the database are never used as they come (`Sync._apply`, since 1.24.2): each part's
    `check(tbl, key, row)` refuses a row this device can't use (a json or array column holds whatever was
    written: by a faulty version, or by whoever has the database's password); a row that still fails while it is
    applied is found by halving its batch (`BAD_ROW`; never `OSError`, which is this device's own trouble and is
    tried again); a row left out is named in the log and stops nothing. Before, one such row stopped every
    device's pull for good. `check` refuses only what would break this device, so a newer version's rows pass.
    A new table's part needs its `check`.
  - What is a device's own never comes from a row. A photo's `path` and the rest of `PHOTO_FIELDS` are never
    taken from `photos.extra`. A device's settings: `HomePart.apply("device_settings")` applies nothing, has the
    device's own values sent again and a row it never wrote deleted. Another device's settings are taken only by
    the owner's Copy settings on this device (`copy_settings` → `take_settings`), and never `STAY_HERE` (the
    settings that say where a device sends photos or fetches files).
- `secretbox.py` — keys in the database, encrypted on the device (scrypt + AES-256-GCM, name as associated data).
  - Every passphrase is a version (`passphrase_versions`, db/006: check value, fingerprint, who/when; one current).
  - The database password alone gets nobody the passphrase (since 1.22.2, db/011; before, it was sealed to every
    row in `devices`). A device gets it only from the owner: typed there (`set_passphrase`, "entered"), or asked
    for (`ask`: a one-time code shown on that device, 8 characters, 15 minutes, never in the database) and the
    code typed on a device that has it (`approve`). The code's check values prove each side to the other
    (`key_requests`).
  - A holder shows it (`devices.key_proof`: a keyed check value over its id and X25519 public key; private half in
    `HOME/device-key.json`, protected). A change (re-encrypts every key in one transaction) or restore is sealed
    (`passphrase_shares`) only to devices showing the passphrase it replaces, and carries a check value made with
    that one's key over which version it replaces, its own id and its fingerprint (`passphrase_versions.link`,
    `_link`; since 1.24.3 with the replaced version in it: a restore reuses an earlier key, and an old link fitted
    again): `catch_up` (`HomePart.every_step`) follows a new version only then
    (`_follow`), or takes the answer to its own request (`_take_answer`). A device that missed two changes needs
    one of the owner's two ways again.
  - Restore only on a device that held that version itself (`HOME/secret.key` `known`).
  - Whoever has only the database password can still delete or damage these rows (keys then stop syncing), not
    read a key or the passphrase. Don't add a path that seals the passphrase to a device on the strength of a row.
  - A device not on the current version neither sends nor reads keys (`push` returns False: not marked sent).
  - The passphrase and its key are kept in `HOME/secret.key` (protected) so Settings shows it masked.
- `peer.py` — the peer service: devices show each other their original photos (db/007) and live cameras (db/008).
  - A device that lacks an original asks one that has it (the `photo_paths` table), which answers with the photo
    view's JPEG; shown, never stored.
  - Listens on the LAN: port = tab page port + 3 (8767; `FACE_PEER_PORT`; `FACE_PEER=off` or the Settings switch
    turn it off). TLS with the device's own certificate (`HOME/peer-cert.pem`), whose SHA-256 is in
    `devices.peer_cert_sha256` and checked by the asker. Its private key (`peer-key.pem`) is kept encrypted, the
    password protected in `peer-key.json` (`_write_key`; a key from before 1.24.3 is encrypted where it is, one
    that can't be opened is replaced with a new certificate: `renew`).
  - Requests signed (HMAC) with the cluster key (HKDF of the database password and its instance id), valid
    `CLOCK_SKEW` (120) s; photos by hash only (`PHOTO_RE`), from the device's own records.
  - truststore replaces `ssl.SSLContext` app-wide, so `peer.py` uses the original class (`_context`). Started by
    `HomePart._peer`.
  - Asking: a device's addresses are tried best first (`_order`: the one that answered last, then those on a
    network both are on), each for `CONNECT` (3) s; never this computer's own address at its own port (`own`: a
    VPN client can give every computer the same address, and a device asking there reached itself). A device tells the others only addresses they can reach
    (`hardware.reachable_addresses`: never its container bridges, even with **Fully distributed system**).
    Who has a photo and where a device is are asked on the kept database connection (`facedb.ask`).
    `fetch`'s reason tells a device that didn't answer at all (`UNREACHED`) from one that answered without the
    photo (Describe everything again stops asking only after the first kind).
  - Serving: an accept queue of 128 and answers sent at once (`request_queue_size`, `disable_nagle_algorithm`),
    as the three local servers have.
  - Clocks: a request signed more than `CLOCK_SKEW` from the service's time is refused with that time, so the
    asker's message says how far apart the two clocks are; `_clock_note` writes to the log, at most once an hour
    for a device, when its clock is more than half of that away (from the `Date` of its answers).

### Hardware, models and LM Studio

- `hardware.py` — what the computer has and how the programs use it; the labeling tool's `/api/hardware` and
  `/api/hardware/network`, shown in Settings → This computer and → LM Studio.
  - `describe`: processor, memory, GPUs, NPU, ONNX Runtime providers (from the registry, /proc and /sys, sysctl).
  - `uses`: `face_id.LAST_DEVICES`, `GPU_DECISIONS`, `import_workers`.
  - `ai_apps`: AI model apps for Describe photo (the usual local ports, the configured describe server, installed
    apps, LM Studio's downloads, the LAN address when served to the network; cached a minute). A usual port
    counts only when a model server answers there (its models, or 401 for a key): other programs use the same
    ports (macOS's AirPlay Receiver has 5000).
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
    restarted with `lms server start --bind 0.0.0.0`, with `--cors` only when LM Studio's own settings have CORS on
    (`_cors`: LM Studio decides, the owner's choice 2026-11-13; if that start fails, it is started again as it was,
    never left off).
  - `announce`: mDNS `_lmstudio._tcp` + `_http._tcp` with its port (Windows `DnsServiceRegister`, macOS
    `dns-sd -R`, Linux `avahi-publish-service`); skipped when the computer announces it already.
  - `local_models`: the models on disk (`lms ls`), kept `MODELS_FOR` (60) s: starting `lms` takes half a second
    and Settings asks every 5 s; `models_changed()` after a download or before publishing asks anew.
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
  - `yamnet.tflite` (YAMNet, Apache 2.0: 521 kinds of sound, for what a room sounds like, `music.sounds`; not
    in git; in `fetch_models.py` from MediaPipe storage only, OPTIONAL until added to models-v1);
  - local only, not in git: AdaFace (evaluated, unused; made by `tools/get_adaface.py`) and `florence2/`
    (`florence.py`). The Linux app's downloaded CUDA is in `ROOT/cuda`, beside `models/`.

### Setup, tools and tests

- `setup.bat` — installs Python 3.12 (winget) and `requirements.txt` (pinned versions; OpenCV as
  `opencv-contrib-python` only, which mediapipe needs; setup removes a plain `opencv-python` if one is installed).
  `requirements-build.txt` adds PyInstaller and its hooks package for `build_app.bat`; `requirements-dev.txt` has
  pytest and ruff. A library the build packs is pinned even when another library brings it in (`pythonnet` and
  `clr_loader` for pywebview on Windows, `pyinstaller-hooks-contrib` for PyInstaller): unpinned, CI and a
  developer's computer built with different ones.
- `setup.sh`, `start.sh`, `label.sh`, `recognize.sh`, `FaceRecognizer.command` — macOS and Linux: a `.venv` with the
  pinned libraries (Linux adds `pywebview[qt]` for the app window), the models, and starting the app (`--browser`
  for the web browser).
- `packaging/windows/FaceRecognizer.iss`, `packaging/linux/install.sh` — the Windows and Linux installers (see
  `build_exe.py`).
- `tools/apple_signing.py` — signing the macOS builds without a Mac: `request` makes the private key (in
  `HOME/apple-signing`, protected, never in git) and the certificate request to upload to Apple; `secrets` checks
  Apple's Developer ID Application certificate against that key and sets the five repository secrets with `gh`
  (values on standard input; the .p12 in the older encryption macOS's `security import` reads). Steps:
  face-recognizer/README.md, "Signing the macOS app".
- `packaging/macos/` — the app's entitlements (camera, microphone) and `sign_and_notarize.sh` (the .app, then the
  .dmg made from it; prints Apple's log when it refuses), which CI runs when the
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
  - `make_icon.py` (draws the app's icon and writes `icons/`; needs Pillow);
  - `keep_session_on_console.ps1` (Windows computers used through Remote Desktop: `-Install` makes a scheduled
    task that puts the session back on the computer's console at every disconnect, unlocked: locked at the
    computer's own screen, a session gets no camera. A Remote Desktop session has no sound devices of the
    computer's own, and its page is drawn only now and then);
  - shared database: `db_certs.py` (the server's CA and certificate), `db_setup.py` (once per new server: the
    tables and `server_ca`, over verify-ca with that CA), `db_import.py`.
- `tests/` — `conftest.py` points `FACE_ROOT`/`FACE_HOME` at a temp folder before anything is imported: no models, no
  real data.
  - Node (`*.test.js`): `expression.js`, `signals.js`, `settings_bridge.js`, `page_store.js` (person signals sent
    per person, `refresh`); `pages.test.js`: every page script
    parses and is served and packed, no inline handlers, no internet fonts, no `confirm()`/`alert()`/`prompt()`,
    `askUser` (`ask.js`), the tab page's silent tabs, and the tracker's JPEGs (worker, its fallbacks, no `toBlob`
    outside `jpegOf`; `jpeg_worker.js` itself).
  - Servers and security: `test_servers.py` (the three servers on test servers: CSP, CORP, cross-site refusals,
    request sizes, Collect faces, presence log, feeling reports, `/api/who`, no page loading from the internet,
    data only for a page's own requests, no program shown that the app didn't start),
    `test_localguard.py`, `test_certificates.py`.
  - Labeling and people: `test_labels.py` (naming, Ignore and undo, naming an Ignored face, rename/merge, the empty
    new-user start, an empty library saved and read again, the face-picture migration, a moved video), `test_persondata.py` (one person's export and forget; Describe all past an unreadable
    photo and spread among LM Studio servers; Describe everything again: video frames, other devices' photos, carry on), `test_self_attributes.py`.
  - Signing: `test_apple_signing.py` (the request, the secrets from a made-up certificate, what is refused; on
    macOS the .p12 goes through the system's own `security import`).
  - Services and downloads: `test_describe.py`, `test_websearch.py` (keys, caps), `test_fetch_models.py` (against a
    local server), `test_cudasupport.py`, `test_lmstudio.py`, `test_hardware.py`, `test_gpu_choice.py` (also a GPU
    that fails while the program runs: the CPU takes over once, the failed session is never destroyed).
  - App window and tracker: `test_window_watchdog.py` (also the window's answer to the camera request, in place
    before any page loads), `test_window_caches.py`, `test_app_log.py` (also a stuck process: its stacks in the log),
    `test_tracker_reload.py`, `test_skeleton.py`, `test_platforms.py` (also the icon files, the build's icon
    options, the Windows installer's rules, Windows code signing: only when told with what, the plug-in only as
    pinned; and the Linux installer run for real on Linux).
  - Shared database: `test_facedb.py`, `test_sync.py` (also a newly installed device, with no faces, joining;
    rows written with only the database's password), with `FACE_TEST_PG` (CI job `database`; each test makes and
    drops its own schema). `test_sync_rows.py` needs no database: rows that can't be used, a photo row that names
    a file path, a row that changes a device's settings, each handed to the code the sync hands them to.
  - Device to device: `test_peer.py` (no database needed: which address is tried first, what a refusal says about
    the clocks, a running service on this computer, a long computer name, the private key kept encrypted).
  - Two processes: `test_trackerlink.py` (no models needed: the local routes answer only with the run's secret;
    hello proves without being handed it; the app takes only the process it started, starts it again when it
    ends, ends it; a tracker that is down is "not shared now", not an error; a file both processes rewrite is
    locked across them, tried with a second process; the Cameras page on its second port: the page and its
    answers there, only under that port's own Host, the port told over the link, a taken port left alone).
  - Tripwires: `test_routes.py` (every address a page asks for is answered by the server that sent the page: the
    page files as the test servers send them against the route strings in the files its `ROUTE_FILES` names; every
    route has a page that asks for it or an entry in `OTHER_CALLERS`; every GET a page sends is sent for real, but
    those in `SKIP`, with any connection or name look-up beyond the test servers stopped) and `test_sync_tables.py`
    (every table in a sync part's `TABLES` has all its places: `KEYS`, `digests`, `rows`, `check`, `apply`, `push`,
    `fetch`, `all_keys`, its migration with the change-log trigger, and for a table about a person the export and
    forget; every table a migration makes is a part's or listed in `NOT_A_PARTS`).
  - Waiting logs: `test_waitlog.py` (no database needed: where a row waits, sent once, a failed send leaves it
    waiting, when a past day's file goes, a file left by the version before, rows added after a forget).
  - Music: `test_music.py` (a sound with a known beat and key measured, usual music from liked songs, what is
    logged when and for whom, Spotify's permission against a stand-in on this computer).
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
- `app.py` (the tracker's camera without a question: WebView2's permission hook on Windows, `allow_camera_macos`
  on macOS, where WebKit otherwise asks at every page load; on Linux the window is first tried in a child process,
  `--probe-window`, falling back to the browser; the watchdog stops a hung renderer on Linux and macOS only; the
  window's pages kept running in the background by Chromium flags on Windows and Linux, `KEEP_RUNNING_FLAGS`, and
  by WebKit's own switches on macOS, `keep_running_macos`);
- `face_id.py`: `auto` decides from the hardware: DirectML on Windows; CUDA or CoreML per model after a start-up check
  against the CPU (`_gpu_check`); face finding too (`_yunet_runner`, with `DET_DYNAMIC` and a session per input
  size). `setup.sh` installs `onnxruntime-gpu[cuda,cudnn]` instead of `onnxruntime` when `nvidia-smi` works; the
  built Linux app gets CUDA from `cudasupport.py`;
- online-only cloud files (`photofiles.py`) are handled on Windows only.

Ports: `FACE_PORTS=<n>` moves the app window's 8764-8766 (and so the peer port, n+3, Spotify's answer, n+4, and
the Cameras page, n+5).

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
A page asks a question with `askUser` (`ask.js`), never `confirm()`, `alert()` or `prompt()`: those stop the camera
and every other script of the window until answered (a test fails on them).
A route and the page's request for it go in together. When a handler's routes move to another module, add the file
to `ROUTE_FILES` in `tests/test_routes.py`; a GET that reaches the network or another program, or starts work, goes
in its `SKIP` with the reason. A new synced table: first give it a row in `ROW` in `tests/test_sync_tables.py`; the
test then names each place it is still missing.
Keep page code in the `.js` files (no inline `<script>` blocks: a test fails on them), and add any new page file to
its server and to `PAGES` in `build_exe.py` (a test checks both). Change a measured threshold only together with
`tests/test_thresholds.py`, after re-measuring.
Test UI changes in the real app or a test copy, never against the owner's running app:
- The owner's app holds ports 8764-8767. Start a test copy from source on spare ports (tracker/labeling `--port`,
  or the app with `FACE_PORTS`), with `FACE_HOME` (or `APPDATA`) pointed at a temp folder and `FACE_ROOT` (or
  `face_id.DATA`) at an empty or copied folder, so it can't touch the real `data/` or `HOME`.
- Start a built test copy with `FACE_UI=none` (servers only), and `tracker.py` or `label_tool.py` from source
  with `--no-browser`: otherwise the program opens its page in the owner's default browser. A tracker tab left
  there keeps running after the test copy is stopped and, with "Keep this camera running", takes the camera the
  moment the owner's app lets go of it: the app then gets "camera busy" at every start (2026-11-15).
- Headless Edge over CDP works for page checks: a browser of its own each run (its own port and profile), ended
  with its whole process tree (`taskkill /PID <pid> /T /F`; ending only the process that was started leaves the
  rest running, with its tabs). Check that none is left over before and after.
- Importing `face_id` works without models; each program calls `face_id.require_models()` when it starts, which
  stops with `NotDownloaded` (what to run) if they're missing.

## Security rules

- The programs' servers listen on 127.0.0.1 only. The one exception is the peer service (`peer.py`, with a shared
  database, on by default there, Settings switch): LAN, TLS with the device's pinned certificate, HMAC-signed
  requests from the cluster key, original photos by content hash only, never a path or anything else; and
  the device's live camera as its tracker page shows it (`/stream`) with its readout (`/readout`, and as JSON parts
  in `/stream`), only while "Stream this camera" is on. The Cameras page's own port (`app.CAMS_PORT`) is the
  tracker's server a second time, on 127.0.0.1 with the same checks: each port answers only requests that name it
  (Host and Origin), so a page of one is another origin to the other.
- The app's two processes ask each other only under `/api/local/` (`trackerlink.py`), and those routes answer
  only with this run's secret (`X-Face-Link`; made by the app, in the environment of the process it starts, in no
  file, page or log). `hello` is the one exception: it takes no secret and proves the answerer has it, so the app
  never sends the secret to a server it hasn't checked. A photo is given to the tracker's process by content hash
  only, as the peer service takes them.
- Every endpoint goes through `localguard.refuse_reason` (exact Host and port; POST needs `X-Face-Tool: 1` and its
  own Origin). The `app.py` shell uses it too: its GETs that do something (`/update`, `/open-release`, `/open`,
  `/attention`, `/restart`, `/restart-app`: `acts` in `shell_handler`) need the header.
- Data, and anything a GET starts, goes only to the tool's own page (`refuse_reason(own=True)`: the browser's
  `Sec-Fetch-Site: same-origin`, or the header from a program that isn't a browser). A page of another program
  on this computer (`same-site`) and an address typed or opened (`none`) get a tool's page files and nothing
  else; a test asks every kind of route both ways.
- Every answer carries `localguard.page_headers`: a Content-Security-Policy with `default-src 'self'` and only what
  each page uses (`localguard.POLICY`; the tracker adds `blob:` scripts and media for its microphone worklet and Open
  video, the tab page the programs' ports as `frame-src`; no inline script anywhere, so no `onclick=` attributes);
  `frame-ancestors` lets only the page itself frame it, plus the app's tab page when `app.py` started the program
  (`localguard.FRAME_PARENTS` is empty until `app.shell_server` adds its real port; a .bat copy trusts nobody, and
  the app says so instead of showing it); the shell page can't be framed at all; `Cross-Origin-Resource-Policy:
  same-origin`, `nosniff` and `Origin-Agent-Cluster: ?1` (each server's pages in a process of their own: one page
  can't slow or read into another's). The tab page also sends `Permissions-Policy`: camera and microphone for itself and the
  programs' ports only (itself, because a page can't hand on what it may not use). Pages load nothing from the internet (a test checks every page file).
- Requests with `Sec-Fetch-Site: cross-site` are refused (another website's page, even as an `<img>`); same-site is
  how the tab page loads a program's page into its frame (the page's own requests after that are same-origin).
  Request bodies go through `Handler.body(limit)` (negative or non-numeric `Content-Length` 400, too large 413).
- In the app window only the tracker this process started (`app.OWN`) gets the camera and microphone without a
  prompt (`app.camera_allowed`; Windows `allow_camera`, macOS `allow_camera_macos` → `camera_answer`), and only
  a program this process started is shown in the window at all (`app.start`). Any other page
  is left to the window's own question. The tracker's request reaches the window in the tab page's name, so the
  tab page's address is answered too: the tab page hands camera and microphone to the tracker's frame alone
  (`allow=` in `app_ui.html`; its `Permissions-Policy` header names itself and the programs' ports only) and its
  own scripts never use them; a test checks all three.
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
- Spotify's answer to **Connect** arrives on a listener of its own (`spotify.wait_for_code`), not on the three
  servers (their rule refuses requests from another site): 127.0.0.1 only, open for five minutes at most after the
  owner's click, and it takes exactly one request, the one carrying the state that click gave out. Spotify's tokens
  are kept with `paths.protect` and reach no page.
- The music log listens to audio interfaces (outputs and microphones) only while its switch is on, and keeps
  numbers only: the sound of any interface is analysed in memory and never stored (`music.record`), like the
  tracker's microphone.
- Downloads are pinned and checked by size and SHA-256 (`fetch_models.py`, `florence.py`, `cudasupport.py`, the
  build's signing plug-in: `build_exe.sign_dlib`), and
  so is a model file found already there (`fetch_models.is_good`, `florence.good`); the
  CUDA download starts only from its Settings button, and only `onnxruntime/` and `nvidia/` are unpacked from the
  wheels. `lmstudio.from_share` checks each file against the shared folder's own list (a damaged copy, not a
  rewritten list: the folder is as trusted as the computers that write to it) and writes only inside LM Studio's
  models and hub folders, whatever paths the list holds.
- `settings_bridge.js` answers only its parent window, on `127.0.0.1`/`localhost`, and only when that is the origin
  the browser reports for the frame's parent (`location.ancestorOrigins`).
- `lmstudio.py` changes another program's settings (LM Studio's `networkInterface` to 0.0.0.0, restarting its server
  on all interfaces, CORS left as LM Studio's own settings have it) and announces it on the local network: the owner's rule for every computer with this app and LM
  Studio (2026-11-10); `FACE_LMSTUDIO=off` turns it off. Model and CUDA downloads start only from a Settings button.
- `hardware.ai_apps` asks 127.0.0.1's usual AI ports and the configured describe server (with that server's own key,
  through `describe.server_models`). `hardware.lmstudio_network` (Settings → LM Studio on the network, only while the
  tab is open, cached a minute):
  - takes announced servers from mDNS (`mdns_services`: `_lmstudio._tcp`, or `_http._tcp` with `app=LM Studio`;
    their own port);
  - tries port 1234, this computer's LM Studio port and announced ports on every address of the /24 of each real
    interface (`own_addresses`: no loopback, and no containers, virtual switches or VPN tunnels unless the owner switched on
    **Fully distributed system**, `hardware.distributed()`, off for a new copy: then every adapter counts, the peer
    service publishes those addresses too, and `describe.helpers` may use servers beyond the local network);
  - reads each server's model list (`/api/v0/models`) and names computers by mDNS reverse lookup or NetBIOS name
    status; nothing is sent but those questions.
- Describe photo: the API key belongs to its server address (changing the address clears it; Test sends it only to
  that address); redirects aren't followed; answers are capped (`MAX_ANSWER`, `MAX_TEXT`).
- What other computers on the network say is checked before use: an LM Studio server's model list by shape
  (`hardware._lms_models`), a model's name against `lmstudio.VARIANT` before it reaches `lms get`, paths in the
  shared models folder's list (`lmstudio._inside`).
- What the shared database says is checked before use too: whoever has its password can write any row, and so
  can a faulty version. Rows go through `dbsync.Sync._apply` and each part's `check`. A row never gives this
  device a file path (`dbsync.PHOTO_FIELDS`; `peer.own_photo` shows only a photo whose record has a path and a
  size) and never changes its settings (the sync's rules above). A row can still change or delete what the
  devices share (a label, a description, a forgotten name): that is what the database is for.
- Photos leave a device for describing only to the configured describe server and, with Share Describe all on,
  to the owner's other devices (`describe.helpers`: an address a device of the shared database gave there, or
  this computer's own), never to another computer that answers like LM Studio.
- No answer of the three servers may be kept by a browser (`localguard.Handler.send`: always `no-store`; a test
  checks every kind of answer). A page that wants a picture twice keeps it in memory.
- Background jobs (imports, possible-face scans, Describe all) start through `Importer.begin` (one at a time, under
  a lock). Shared caches and counters used by request threads need a lock. The labeling data changes only under
  `db.lock`; what a request reads without it (a face's picture: `label_tool.face_thumb`) must not walk a list
  that grows meanwhile (`LabelDB.E` and `printless` make their tables under the lock).
- Every person's data can be exported and forgotten (`persondata.py`): a new place that keeps anything about a person
  must be added to its list, `export` and `forget`, and to `tests/test_persondata.py`.
- Windows code signing keeps no key and no password anywhere: the key is in Azure Artifact Signing, and CI signs
  in as this repository's workflow on `main` (GitHub's own token for the run, which Azure is told to trust for that
  branch alone, with the Signer role on the one certificate profile). The five repository secrets are names and
  ids. Don't add a client secret or a certificate file to sign with, and don't sign other branches or pull requests.
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
  ("Paused" on a page means the system stopped the camera's picture, a locked screen on Windows: the app says
  so, it never does it.) Never lock a screen to tidy up: on Windows that stops the camera.
- **They move between cameras** (and so camera sightings) is off until switched on.
- Times are stored in UTC everywhere (files, file names, log lines, the database: `paths.utc_now`, `paths.utc_stamp`,
  timestamptz with the session in UTC) and shown in local time only on the pages; `paths.parse_time` also reads
  the older local-time text.
- Describe photo is local by default. Pointing it at another device (Settings) sends each described photo there; the
  Settings tab must say so (`describe.where`). Descriptions and `describe.json` live in `HOME`.
- Releases: the three installers (`FaceRecognizer-Setup-x64.exe`, `FaceRecognizer-macos-arm64.dmg`,
  `FaceRecognizer-linux-x64.run`), `FaceRecognizer-win64.zip` (`FaceRecognizer.exe` and a new-user `README.txt`),
  `FaceRecognizer-macos-arm64.zip`, `FaceRecognizer-linux-x64.tar.gz` and `SHA256SUMS.txt`, from one CI run's
  builds of the tagged commit; never `data\`. An installer never deletes `models` or `data`, on install, update
  or uninstall. The macOS app and its .dmg are signed and notarized by CI (since 1.22.4, with the
  repository's Apple secrets): release only a run whose macOS build says so (`BUILD-INFO.json` `signed`). The
  Windows .exe and installer are code-signed by CI on pushes to `main` (the Azure signing secrets are set since
  2026-11-17): release only a run whose Windows build says `signed` too, and say in the notes what SmartScreen
  still does (it warns about a new publisher until it has seen enough installs).
- The presence log (who was in view, when) is opt-in, names and times only, in `HOME/presence/`, never `data/`.
- Webcam faces named automatically don't enter the recognition gallery until confirmed (`LabelDB.gallery`).
- Ignore deletes the face's print and picture (`LabelDB.forget_faces`, `Store.forget`; Undo restores them from memory
  in the same session); only its ID, photo and box stay, which keep the photo from being read again. Faces ignored
  before v1.9 are converted at the labeling tool's start. Code that uses prints must skip `printless` faces.
  An Ignored face that is given a name or taken off the Ignored list gets both made again from its photo
  (`LabelDB.act` → `remake`; Undo deletes them again); when the photo isn't on the computer the face stays as it is
  and the page says so. Only an Ignored face may be without a print (`lost_prints`, `mend`: faces older versions
  left that way, mended at the labeling tool's start).
- Collect faces never saves whole frames, only each face with room around it and the others greyed out.
- InsightFace ArcFace and audEERING Wav2Small weights are non-commercial; note this if the project is ever redistributed.
