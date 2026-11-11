# Face Recognizer

Three programs share the `data` folder:

- **tracker.py**: the live face tracker in your browser (double-click **recognize.bat**; macOS, Linux: `./recognize.sh`)
- **label_tool.py**: add photos, confirm suggested names, name new people, fix mistakes (double-click **label.bat**; macOS, Linux: `./label.sh`)
- **face_id.py**: command-line scanning, grouping and recognition

Recognizes specific people (by name) on a webcam or in photos and videos, trained on your own photo library.

- **Detection:** OpenCV YuNet (`models/face_detection_yunet_2023mar.onnx`)
- **Recognition:** ArcFace, ResNet-50 trained on WebFace600K (`models/arcface_w600k_r50.onnx`, from InsightFace's buffalo_l pack). It turns each face into a 512-number face print. InsightFace's pretrained models are licensed for non-commercial use; commercial use needs a license from InsightFace. The older, lighter SFace model is kept as a fallback.
- **Matching:** a face is labeled with the known person whose 5 closest face prints are most similar, if the similarity is at least 0.35. Otherwise it's labeled "Unknown". Measured on your labels: 100% named correctly, no strangers named, and on simulated blurry webcam frames 76% named correctly (SFace managed 44%).
- **Face quality:** eDifFIQA (`models/ediffiqa_tiny_jun2024.onnx`) scores each face. Faces below 0.30 (blurry, tiny, turned away) aren't used to learn a person, and in the live tracker sharp frames outweigh blurry ones when deciding a name.
- **Expression:** OpenCV Zoo's facial expression model (`models/facial_expression_recognition_mobilefacenet_2022july.onnx`, MobileFaceNet, Apache 2.0; 88% on the RAF-DB benchmark) reads each face as angry, disgust, fearful, happy, neutral, sad or surprised. The live tracker shows it under the name; see **Expression** below for how it's kept honest.
- **Face-shape cues (optional):** MediaPipe Face Landmarker (`models/face_landmarker.task`, Apache 2.0, `pip install mediapipe`) measures 52 facial movements such as smile, brow raise and jaw drop. The tracker shows them as evidence next to the expression; they don't change the label.
- **Body posture (optional):** MediaPipe Pose Landmarker lite (`models/pose_landmarker_lite.task`, Apache 2.0) finds 33 body points; the tracker uses the head, shoulders, arms and hips. About 15 ms a frame on the CPU.
- **Voice (optional):** audEERING's Wav2Small (`models/wav2small.onnx`, 17K parameters, made by `python tools\get_wav2small.py` from the official weights) reads arousal, dominance and valence from speech when you turn on the microphone. Licensed CC BY-NC-SA 4.0: non-commercial use only, like the ArcFace weights.
- **Security:** the labeling tool and tracker only accept requests from their own pages (see `localguard.py`), so other websites can't change your labels or load your face pictures. Their pages load nothing from the internet: no fonts, scripts or pictures from elsewhere (a Content-Security-Policy allows only their own server).

Everything runs locally. No photos leave your computer, with one exception you start yourself: **Web search** in the photo view (see below) sends that face to Amazon and that photo to Google to suggest who it is.

## Getting it

- **Just to run it:** download it from the project's GitHub **Releases**: `FaceRecognizer-win64.zip` (Windows: unzip it anywhere and double-click **FaceRecognizer.exe**), `FaceRecognizer-macos-arm64.zip` (macOS 14+ on Apple Silicon: **FaceRecognizer.app**) or `FaceRecognizer-linux-x64.tar.gz` (Ubuntu 22.04+: unpack it and run `FaceRecognizer/FaceRecognizer`). No Python needed. It starts with no faces or names: add your own photos in the Labeling tab (**Add photos**). On first start it downloads the face models (about 196 MB): on Windows into the `models\` folder beside it, on macOS and Linux into your application-data folder (see **macOS and Linux**). After that it works offline.
- **The code:** install [Git LFS](https://git-lfs.com) first, then `git clone` the repo. The face models are stored with Git LFS. Without it you get small placeholder files instead, and the programs stop with *"Model files weren't downloaded: install Git LFS and run git lfs pull"*. Run `git lfs install` and `git lfs pull` to fix that. `python fetch_models.py` (which setup.bat also runs) downloads just the models.
- **Download ZIP** on GitHub includes the models: the repository's *Include Git LFS objects in archives* setting is on (since 2026-11-08). Unzip it and run `setup.bat` as above.

## macOS and Linux

The app also runs on **macOS 14 or later on Apple Silicon** (M1 and later; the recognition libraries have no Intel
Mac builds) and on **Linux** (built and tested on Ubuntu; x86_64, and ARM from source).

- **Ready-made:** the release's `FaceRecognizer-macos-arm64.zip` (`FaceRecognizer.app`; not yet signed by Apple:
  right-click it and choose **Open** the first time) and `FaceRecognizer-linux-x64.tar.gz` (unpack it, then run
  `FaceRecognizer/FaceRecognizer`). To build them yourself, see **Building the app** below.
- **From the code:** `./setup.sh` once (a Python 3.12 environment in `.venv` with the pinned libraries and, on Linux,
  the Qt app window; then the models), then `./start.sh`, or double-click `FaceRecognizer.command` on a Mac.
  Python 3.12 first: `brew install python@3.12` (macOS) or `sudo apt install python3.12 python3.12-venv` (Ubuntu).
- **Where things are kept:** the built app keeps its models and `data` in `~/Library/Application Support/FaceRecognizer`
  (macOS) or `~/.local/share/FaceRecognizer` (Linux), and what's private to you (keys, alerts, presence log, the
  window's storage, app.log) in `.../FaceLabels` beside it. Run from the code, models and `data` stay in the project
  folder, as on Windows.
- **Linux system libraries:** a desktop Ubuntu has them all. A minimal install (a server, WSL, a container) needs
  the graphics and window libraries the app window uses:
  `sudo apt install libgl1-mesa-dri libglx-mesa0 libegl1 libegl-mesa0 libgbm1 libwayland-client0 libwayland-server0
  libwayland-egl1 libxkbcommon-x11-0 libxcb-cursor0 libxcb-icccm4 libxcb-keysyms1 libxcb-shape0 libnss3
  libxcomposite1 libxdamage1 libxrandr2 libxtst6 libasound2t64 fonts-dejavu-core` (`libasound2` before Ubuntu 24.04).
  Without working OpenGL the window can't draw; the app notices (it tries the window first) and uses the browser.
- **In the browser instead:** `./start.sh --browser` (or `FaceRecognizer --browser`) opens the same tabs in your web
  browser. The app does this by itself where no app window can open.
- **What's different:** the app picks the processor for each model itself (see **Your hardware: what the app uses**):
  an NVIDIA card on Linux (`./setup.sh` installs ONNX Runtime with CUDA when `nvidia-smi` works) or Apple Silicon's
  CoreML, when a check at start shows it gives the same results and is faster; otherwise the CPU (OpenCV). The labeling tool's handling of
  online-only cloud files (iCloud, OneDrive) is Windows-only for now. On first use, macOS asks once whether the app
  may use the camera and microphone.

## Setup (once)

**macOS and Linux:** run `./setup.sh` (see **macOS and Linux** above); then use `.venv/bin/python` wherever this guide says `python`, and `./start.sh` for the app.

**Windows:** double-click **setup.bat**. It installs Python 3.12 if needed (with winget), installs the libraries in `requirements.txt`, downloads any face models that are missing (`fetch_models.py`), and checks that they load. The .bat files all use that Python 3.12 (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`); to run the `.py` files yourself, use the same one, because a different Python on your PATH may not have OpenCV and ONNX Runtime installed.

## One window: FaceRecognizer.exe (FaceRecognizer.app, FaceRecognizer on Linux)

**FaceRecognizer.exe** opens the face tracker and the labeling tool in one window, with a **Face tracking** tab, a **Labeling** tab and a **Settings** tab at the top (**Ctrl+1** / **Ctrl+2** / **Ctrl+3** also switch). Inside, the two are still separate programs on their usual ports (8766 and 8765) with the same pages as `recognize.bat` and `label.bat`. The tracker keeps running while you're on the Labeling tab, so **Collect faces for labeling** goes on collecting. If either is already open from its .bat, the tab says so and asks you to close that copy (a copy started on its own doesn't let other pages show it, so the window can't), rather than starting a second one on the same data. Both tabs are ready about 10 seconds after starting; closing the window stops everything.

- **Build it** with **build_app.bat**; see **Building the app** below for the full steps on Windows, macOS and Linux. Or download a ready-made one from GitHub **Releases**.
- **Keep it in this folder.** The .exe carries only the code and pages. It uses the `models\` and `data\` folders beside it, so it shares labels with the .bat launchers and git. Rebuild it after changing any `.py`, `.html` or `.js` file.
- **The window** is Edge WebView2 (part of Windows 11), WebKit on macOS, and Qt's web view on Linux. On Windows the camera and microphone are allowed for the app's own pages without a prompt; macOS asks once for the app. The window remembers each page's options and the per-person baselines in `%APPDATA%\FaceLabels\webview\`, separately from your browser's. Messages and errors go to `%APPDATA%\FaceLabels\app.log`.
- **Settings** gathers every option in sections: App (version, Check for updates, and buttons that open the `data` folder and `%APPDATA%\FaceLabels`), Face tracking (camera, display, expression and signals, recognition, alerts and presence log: tick who to be alerted about in a list of everyone known, plus Unknown), Labeling, This computer (the processor, memory, graphics processors and NPU the app found, and where each job runs: see **Your hardware: what the app uses**), Web search (enter or remove your own keys) and Describe photo (which model describes photos; download the built-in one). Changes apply at once and are remembered between runs; sections have **Reset to defaults**. The same options stay on the Face tracking and Labeling tabs too, and the two always agree. Every Face tracking option is now remembered between runs (before, only some were).
- **Check for updates** (top right of the window) asks GitHub for the latest release, only when you click it, and offers to open its download page if it's newer.
- **If the camera is "busy" or missing**, another app has it, often the tracker still open in a browser tab, which starts the camera by itself. Close that tab (or the other app) and press **Start camera** again.

### Building the app (Windows, macOS, Linux)

`build_exe.py` builds the app with PyInstaller for the system it runs on: it can't build for another one, so build each
on its own system (or let GitHub do all three, below). Every build packs in the page files (`tracker_ui.html`/`.js`,
`label_ui.html`/`.js`, `app_ui.html`/`.js`, `expression.js`, `signals.js`, `settings_bridge.js`) and MediaPipe's and
ONNX Runtime's own files, and leaves out PyTorch, TensorFlow, pandas and IPython, which nothing in the app uses (with
PyTorch the .exe was 646 MB). The app carries code and pages only, never models or `data`: it downloads the models on
first start. Its work files go in `face-recognizer/build/`, which git ignores; delete it any time. A build takes
2–6 minutes and needs about 2 GB of free disk space and an internet connection the first time.

**First, on every system: get the code with its files.** Install [Git](https://git-scm.com) and
[Git LFS](https://git-lfs.com), then:
```
git lfs install
git clone https://github.com/SingularityResearch/facial-recognition.git
cd facial-recognition/face-recognizer
```
Without Git LFS the models arrive as small placeholder files; the setup step below replaces them by downloading the
real ones (`fetch_models.py`). `data` isn't in the repository: it's created empty on first start.

#### Windows 10/11 (64-bit) → `FaceRecognizer.exe`

1. **Run `setup.bat` once.** It installs Python 3.12 (with winget) and the libraries in `requirements.txt`, downloads
   any missing face models, and checks that they load. All the .bat files use that Python 3.12
   (`%LOCALAPPDATA%\Programs\Python\Python312\python.exe`).
2. **Close FaceRecognizer.exe if it's running.** Windows locks a running .exe, so the new one can't be copied over it.
3. **Double-click `build_app.bat`.** It installs `requirements-build.txt` (`requirements.txt` plus PyInstaller), runs
   `build_exe.py`, and copies the result to `face-recognizer\FaceRecognizer.exe` (about 210 MB).
4. **Try it.** Double-click `FaceRecognizer.exe`. Both tabs should be ready in about 10 seconds. If a tab says it
   couldn't start, the reason is in `%APPDATA%\FaceLabels\app.log`.

The .exe isn't code-signed, so Windows SmartScreen may warn the first time it runs (**More info → Run anyway**).

#### macOS 14+ on Apple Silicon → `FaceRecognizer.app`

1. **Python 3.12:** `brew install python@3.12` ([Homebrew](https://brew.sh)). Intel Macs can't build or run it: the
   recognition libraries have no Intel Mac builds.
2. **Run `./setup.sh` once** (a `.venv` with the pinned libraries, then the models).
3. **Build:**
   ```
   .venv/bin/python -m pip install -r requirements-build.txt
   .venv/bin/python build_exe.py
   ```
   The app is `build/dist/FaceRecognizer.app`. `build_exe.py` writes into its `Info.plist` why it asks for the camera
   and microphone (without that, macOS refuses them), that it needs macOS 14, and its version.
4. **Try it:** `open build/dist/FaceRecognizer.app`. macOS asks once for the camera and microphone. A build made
   on your own Mac runs there as it is; to give it to others, sign and notarize it (next step), or they must
   right-click it and choose **Open** the first time.
5. **Sign and notarize (optional):** with an Apple Developer account, `packaging/macos/sign_and_notarize.sh
   build/dist/FaceRecognizer.app` signs it with your *Developer ID Application* certificate (hardened runtime, the
   camera and microphone entitlements in `packaging/macos/entitlements.plist`) and has Apple notarize it. It reads the
   certificate and an App Store Connect API key from the environment variables listed at its top; CI does this by
   itself once they're set as repository secrets.

The built app keeps its models and `data` in `~/Library/Application Support/FaceRecognizer` and what's private to you
in `~/Library/Application Support/FaceLabels` (a signed app can't change itself).

#### Linux (Ubuntu 22.04 or later, x86_64) → the `FaceRecognizer` folder

1. **Python 3.12 and the build's system libraries:**
   ```
   sudo apt install python3.12 python3.12-venv libegl1 libgl1 libxkbcommon-x11-0 libxcb-cursor0 libxcb-icccm4 \
     libxcb-keysyms1 libxcb-shape0 libnss3 libxcomposite1 libxdamage1 libxrandr2 libxtst6 libasound2t64
   ```
   (`libasound2` before Ubuntu 24.04.)
2. **Run `./setup.sh` once** (a `.venv` with the pinned libraries and, on Linux, `pywebview[qt]` for the app window;
   then the models).
3. **Build:**
   ```
   .venv/bin/python -m pip install -r requirements-build.txt
   .venv/bin/python build_exe.py
   ```
   The app is the folder `build/dist/FaceRecognizer/`; `FaceRecognizer` inside it starts it. `build_exe.py` leaves the
   graphics libraries (Mesa, OpenGL/EGL, Wayland, X11) to the system: they must match its drivers.
4. **Try it:** `build/dist/FaceRecognizer/FaceRecognizer`. To share it, pack the folder:
   `tar czf FaceRecognizer-linux-x64.tar.gz -C build/dist FaceRecognizer`. A build runs on the Ubuntu release it was
   built on and later ones, so build on the oldest you want to support (CI uses 22.04).

The built app keeps its models and `data` in `~/.local/share/FaceRecognizer` and what's private to you in
`~/.local/share/FaceLabels`. If its window can't draw (no working OpenGL), it opens in your web browser instead.

#### Builds on GitHub, and making a release

Every push builds the app on GitHub's Windows, macOS and Linux machines once the tests pass on all three
(`.github/workflows/checks.yml`). Each run keeps, for 14 days, `FaceRecognizer-windows-x64` (the .exe),
`FaceRecognizer-macos-arm64` (the zipped .app; signed and notarized when the repository's signing secrets are set) and
`FaceRecognizer-linux-x64` (the .tar.gz), each with `BUILD-INFO.json` (commit, run, SHA-256). Download one with
`gh run download <run id> -R SingularityResearch/facial-recognition -n FaceRecognizer-linux-x64`, or, with an older
`gh` that refuses that ("would result in path traversal"), with
`gh api repos/SingularityResearch/facial-recognition/actions/artifacts/<artifact id>/zip > build.zip`.

To make a release:
1. Raise `VERSION` in `app.py` (for example `"1.10"`), commit, push, and wait for the run to pass. Test its builds.
2. From that run's artifacts: zip `FaceRecognizer.exe` with a short `README.txt` for new users as
   `FaceRecognizer-win64.zip`; take `FaceRecognizer-macos-arm64.zip` and `FaceRecognizer-linux-x64.tar.gz` as they
   are; list their SHA-256 in `SHA256SUMS.txt`. **Never include `data`**: it holds your faces, names and labels. A new
   user's copy starts empty.
3. Attach the four files to a release tagged `v1.10` at that commit, with notes saying how to start it on each system.

Leave out the models: the app downloads them on first start (`fetch_models.py`): YuNet, eDifFIQA and the expression
model from OpenCV Zoo on Hugging Face, the face and pose landmarkers from Google's MediaPipe storage, ArcFace from
InsightFace's own release (just that file out of their `buffalo_l.zip`; each the very same file, checked by SHA-256; a
model already there is checked by SHA-256 too, once, and then only again when its size or date changes:
`models/.verified.json`), and Wav2Small, plus any of the others whose source fails or has changed, from the release
`models-v1`. So that release must stay, and the repository must stay public. If a model changes, publish a new models
release (for example `models-v2`) and update `TAG` and the file list in `fetch_models.py`.

**Rebuild after any change** to a `.py`, `.html` or `.js` file: the app carries its own copy of the code and pages.
The models and `data` aren't inside it, so new labels and models need no rebuild.

**Run the window without building:** `python app.py` from `face-recognizer` with the Python above (Windows), or
`./start.sh` (macOS, Linux). This gives the same window and tabs, straight from the source files.

### Running it as a plain web app (no .exe)

The tracker and the labeling tool are ordinary local web apps; the .exe only puts them in one window. To use them in your browser instead (Edge or Chrome):

| | Start | Open |
|---|---|---|
| Face tracker | double-click `recognize.bat` (macOS, Linux: `./recognize.sh`), or `python tracker.py` | http://127.0.0.1:8766 |
| Labeling tool | double-click `label.bat` (macOS, Linux: `./label.sh`), or `python label_tool.py` | http://127.0.0.1:8765 |

- Each .bat (or .sh) opens its page in your browser. Leave its console or terminal window open while you use it, and close the window to stop it. Run both at once to have both pages; they're separate programs.
- Options: `--port 9000` uses another port, and `--no-browser` doesn't open the page (for example to bookmark it, or to start it from a script).
- The browser asks once for the camera and microphone. (In FaceRecognizer.exe's window only the Face tracking tab gets them without asking.) It remembers each page's options and per-person baselines itself, separately from the .exe window's.
- Both servers listen only on this computer (`127.0.0.1`). Other devices on your network can't reach them, and pages from other websites can't use them.
- If FaceRecognizer.exe starts while one is already running from its .bat, that tab says so: close the .bat copy and start the app again. A page started from its .bat can be shown only by itself, never inside another page (only the app window's own tab page may show the programs it started).
- The **Settings** tab exists only in FaceRecognizer.exe's window (or `python app.py`). In the browser, the same options are on each page: the tracker's **Controls** panel, the labeling tool's toolbar and photo view, and **Settings and other ways to search** in the Web search dialog. The Describe photo server, model and API key can be set only in the Settings tab (or the default address with `FACE_LLM_URL`).

## Step by step

### 1. Add new photos

1. Put the new photos in a folder, for example `E:\Users\you\Pictures\New`. Subfolders are included. Supported: JPG, PNG, HEIC/HEIF, WebP, BMP, GIF, TIFF.
2. Double-click **label.bat** (macOS, Linux: `./label.sh`). **Face Labels** opens in your browser at http://127.0.0.1:8765/.
3. Click **Add photos** (top right), type or paste the folder path, and click **Start adding**. Folders you've added before are offered as you type.
4. Exports from photo apps work well here: a Google Takeout of Google Photos, or a folder tagged in Lightroom, Picasa, digiKam or Windows Photo Gallery. Their people tags show up under **From photo tags**. (Photos straight from an iPhone carry face boxes but no names, so they add nothing here.)
5. A progress bar at the top shows the folder, photos read, faces found, faces named automatically and photos already added. You can keep labeling while it runs, and **Stop** ends it early. Photos already read are kept, and the rest are read the next time you add the folder. When it's done, the result stays up for 5 minutes, even when nothing was new. An import started some other way (for example the webcam snapshots) shows here too.

Adding the same folder again is safe: photos already read are skipped, even if they've been renamed or moved, and so are re-saved or resized copies of them. Faces that clearly match someone you've already named are named automatically.

Videos (.mov, .mp4, .m4v, .avi, .mkv) in the folder are read too. About one frame a second is looked at, at most 300 frames per video, so a long video is sampled across its whole length. The same person is followed from frame to frame, and only their 3 clearest faces, at least 3 seconds apart, are kept. A long clip of one person adds a few faces, not hundreds. Video faces have a **video** tag, and the photo view shows the frame each one was found in and its time in the video. Skipped:

- the short clip of an iPhone Live Photo (same name as the photo beside it), because the photo is already read;
- a video with the same name and size as one already read, so an online-only copy (for example in iCloud Photos) isn't downloaded just to find it's the same.

Other online-only photos and videos are downloaded by Windows as they're read.

From the command line, the same thing is: `python face_id.py scan "E:\Users\you\Pictures\New"`.

### 2. Label the faces

In Face Labels, work down the **To review** list on the left. Every change is saved at once; **Undo** (or **Z**) takes back the last one.

1. **Suggestions**: faces the model thinks it knows, grouped by person, least certain first. Click the faces that are wrong (or drag a box across them), press **R** ("not this person"), then click **Accept all** for the rest.
2. **New faces**: faces that look like nobody you've named, in groups of look-alikes. Click **Name…** (or press **N**), pick a name or type a new one, and press Enter. To name only some faces in a group, select them first. Use **Ignore** (**X**) for strangers, artwork and tattoos so they aren't asked about again. If a group shows "Looks like ...?", click it to add the group to that person.
3. **Named automatically** (optional): glance over the automatic names. Select any wrong ones and press **R**; **Looks right** marks the rest as checked.
4. **Checks**: fix faces that look more like someone else, and see who needs more photos. Each person needs about 5 clear faces to be recognized reliably.
5. **From photo tags**: faces that the photos' own people tags name (from photo apps or a Google Takeout export). Check them like suggestions.

Double-click a face (or press **Space**) to see the whole photo. For a face nobody has named, the photo view also shows clues: the closest people (even weak matches), the photo's own tags, and who is named in other photos from the same occasion. Click a name to use it. **Web search** asks Amazon and Google and suggests who it is, with a confidence level. To fix a person's name or merge two people, click them under **People** and use **Rename or merge**. More detail is in **Label people** below.

### 3. Recompute the data (only when needed)

Normal labeling needs no recompute: every change rebuilds `data/people.npz` (what the tracker recognizes from), and a running tracker reloads it by itself. Recompute only in these cases:

| When | What to run (from this folder, tracker and Face Labels closed) |
|---|---|
| You switched recognition model (`FACE_MODEL`, or the ArcFace file was added or removed), or a program says "made with ..., run reembed" | `python face_id.py reembed` |
| The tracker doesn't know someone it should, or `people.npz` is missing or out of date | `python face_id.py reembed` (it also rebuilds `people.npz`) |
| You restored `labels.json` from `data\backups\` | Copy the backup over `data\labels.json`, then run `python face_id.py reembed` |
| Photos moved to a new drive or folder (the photo view says "The original photo isn't on this computer any more") | Usually nothing: each time Face Labels starts, it looks for missing photos in the same folder on other drive letters, then by name and size in your photo folders, and updates their paths and the folder list (the console says "Found N of M photos that had moved"). If they're somewhere else, add the new folder with **Add photos**. Photos are matched by content, so they're re-linked and nothing is read twice. |

`reembed` takes about a minute. It recomputes every face print from the saved face crops, with no photo rescan, and keeps all your labels. Check the result with `python face_id.py people`.

### 4. Start the face tracker

1. Double-click **recognize.bat** (macOS, Linux: `./recognize.sh`). A black **Face Tracker** window (a terminal on macOS and Linux) opens, loads the models (it lists which processor runs each one), and opens http://127.0.0.1:8766/ in your browser.
2. Pick your webcam in **Camera** and click **Start camera**. The first time, allow camera access in the browser. Next time the same camera starts automatically, unless you last stopped it with **Stop camera**.
3. Named people show their name and match percentage on their face; everyone else shows **Unknown**. Names you add in Face Labels appear without restarting.
4. To stop, close the black window (or press Ctrl+C in it). To keep it running but turn the camera off, click **Stop camera**.

If the camera won't start, close other apps that use it (Teams, Zoom, another browser tab) or pick another camera from the list. If the page says "Recognizer isn't running", start recognize.bat (or recognize.sh) again. Running it while the tracker is already open just opens the existing page. The tracker and Face Labels are separate programs and can run at the same time.

## Command-line recognition

```
python face_id.py people                     # who it knows
python face_id.py recognize --camera 0       # live webcam; try 1 or 2 if 0 is the wrong camera
python face_id.py recognize "D:\some\photos" --out tagged   # tag photos, save annotated copies
```

## Live tracker: tracker.py

Double-click **recognize.bat** (macOS, Linux: `./recognize.sh`). Your browser opens the Face Tracker:

- **Layout**: the video fills the window. The controls are a panel on the right. **Hide controls** (top right, or **Ctrl+.**) slides it away and the video grows to the full width, and **Controls** brings it back. The page remembers which you chose. On narrow windows the panel slides over the video instead and starts closed; **Esc** closes it.
- **Camera**: pick your webcam from the list by name, then **Start camera**. **Open video** tracks a video file instead.
- Each face gets a box, and the person's name and match percentage appear on their face. People you haven't named show as **Unknown**. The name comes from the face's last 7 checks together: their face prints are averaged, sharper frames counting more, and the average is matched once. On real webcam faces made blurry, noisy or small the way live frames are, this named 98% correctly, against 93% for the earlier way (each check names someone, and the names are voted on), with no stranger named either way (`tools/webcam_eval.py`).
- **Display**: mirror view, keypoints, face mesh, motion trail, show or blur unknown faces, smoothing.
- **Face mesh (triangles)** (off by default; remembered): MediaPipe's 478 face points joined into 852 triangles over each face, in the person's colour. Each triangle is shaded by the angle it faces, from the points' depth: clear where it faces the camera, darker where the surface turns away, so the mesh follows the shape and turn of the head. It's drawing only and doesn't change any reading. While it's on, the picture shows the frame that was just analysed instead of the live video, so the mesh stays on the face as you move. The picture runs a few hundredths of a second behind and updates at the recognition rate, and the smoothing setting is skipped. It costs about 7–10 ms per face on the CPU, and the page draws each face's triangles in 8 shade groups so several faces stay smooth (the same run that gives the face-shape cues) and needs MediaPipe and `models/face_landmarker.task`; the option is greyed out without them.
- **Match strictness**: raise it if wrong names appear, lower it if known people show as Unknown.
- **Collect faces for labeling** (off by default; remembered): while a face stays in view, the tracker saves a picture of that face every 5 seconds, up to 12 per face per visit. It only saves a face after 3 checks in a row and only when the face is sharp enough to learn from (the same quality setting as photos). Known and Unknown faces are both saved: collecting Unknown faces is how new people get named. **Only the face is saved, never the whole frame**: the face with room around it of 0.6 face widths on each side (what the labeling tool's import needs to find the face again; enlarged to 160 px when smaller), with every other face in that room greyed out, so people in the background and the room itself aren't kept. Each picture goes to `%APPDATA%\FaceLabels\webcam\` with a `.json` beside it saying who the tracker thought the face was and its match percentage. The labeling tool imports new pictures by itself (see **Label people**). They stay outside the project; once imported, their faces (prints, pictures, labels) are in `data/` like faces from photos. Ignore a face you don't want kept: its print and picture are then deleted (see **Label people**).
- **Show expression** / **Calibrate to each person** / **Show expression evidence** / **Expression test**: see below.
- **Listen to voice** / **Webcam pulse** / **Body posture** / **Skip expression while talking** / **Skip expression when a hand is on the face** / **Use state model** / **Feeling check**: see **Other signals** below.
- **Alerts and presence log** (closed panel; everything off until you set it, all remembered):
  - **Alert me about these people**: type names, separated by commas (or tick them in the Settings tab).
  - **Events that raise an alert**: **They arrive** (recognized after being out of view at least the set time, 10 minutes by default), **They leave** (out of view for the set time), **Their signals are unusual for them** (any signal at least the set number of spreads from their own normal for the set time, at most one alert a person per set period; needs **Learn each person's normal**; these thresholds are guesses, not yet measured), and **An unknown face stays** (the set time).
  - **Timing**: each of those times, how long after an alert it measures again (15 s by default), and how long the banner stays (or until dismissed).
  - An alert shows as a banner on the video (in the app window, over whichever tab is open), rings the **Alerts** bell (with how many are new, on the tracker page and in the app's tab bar, Ctrl+4), puts the count in the window's title, flashes its taskbar button when the window is behind others, plays a short sound (**Play a sound with alerts**) and shows a Windows notification if allowed.
  - **The Alerts panel** keeps every alert (the newest 500) until you act on it: **New**, **Acknowledged** and **Archived** lists, with Acknowledge, Archive, Mark new, Restore and Delete, one at a time or all at once. **Details** shows what was measured when the alert was raised and again after the set time: the evidence lines exactly as **Show expression evidence** shows them (expression, signals, body, unusual for them, what it runs on), the expression reading (share, lead, valence/arousal, the latest frame's scores beside their usual look, face-shape cues), recognition, each signal against their normal, 30-second averages and which sensors were on. Numbers and text only, never a picture or sound. Kept in `%APPDATA%\FaceLabels\alerts.json`; a person's **Their data…** exports and forgets their alerts.
  - **Keep a presence log**: notes each visit, meaning who was in view, from when to when, once they've been out of view for a minute. Only names and times are kept, no pictures. A visit counts once a person has been named over 3 checks, so a passing misread isn't logged. The log is kept in `%APPDATA%\FaceLabels\presence\` (one file per day), on this computer only and never in the published `data\`. **Show today** lists today's visits, including people in view now.
- Names you save in the labeling tool are picked up automatically.

### Expression

The model reads the *expression on the face*, not what the person feels: a polite smile and a happy one both read "happy", and a frown of concentration reads "angry" or "sad". Treat it as "this face currently looks ...". Several things are done to make that reading as accurate as it can be:

- **Only readable faces are scored.** The recognizer skips frames whose quality is below 0.25 or whose head is turned more than about 35 degrees (`Q_EXPR` and `MAX_TURN` in `face_id.py`). Blur and profile views don't make the model unsure, they make it wrong (usually "sad" or "neutral"), so those frames cast no vote rather than a weak one.
- **Each face is scored twice**, as is and mirrored, and the two are averaged. This is a standard accuracy gain for expression models; measured on your stored faces, the two views give the same label 93% of the time. Both runs together take 0.7 ms per face on the GPU.
- **Frames vote.** The page averages the last ten scored frames (about a second), weighted by face quality, and only reports a label that has at least 45% of the vote and leads the runner-up by 12 points. Anything less is shown dimmed with a question mark. On your stored faces 95% give a clear winner.
- **Calibrate to each person** (on by default) learns each named person's usual look: a slow running average of their scored frames, kept in this browser (**Forget what's learned** clears it, for everyone or for one person). It's trusted after about 100 frames of that person. Then a reading that departs from their usual look is shown bright, and one that matches it is tagged **usual**. This is what stops someone whose resting face reads a bit "sad", or who smiles most of the time, from being reported as news every second. Turn it off to see the raw reading. While a person's usual look is still being learned, the evidence line says how far along it is ("usual look 34% learned").
- **Show expression evidence** (off by default) adds a line under each face in the live readout explaining the reading, for example `8 votes · 62% · lead 21 · V+0.5 A+0.4 · smile .82 cheek .61`: how many frames voted, the winner's share and its lead over the runner-up, a rough valence (unpleasant to pleasant) and arousal (calm to excited), and the face-shape cues that usually go with that label. When the reading is unclear it says why: `few-votes`, `low-share`, `low-lead`, or `blurry` / `turned` when the latest frame couldn't be read. Valence/arousal are rough textbook placements of the 7 labels, not measurements. The cues cost about 10 ms per face on the CPU, so they're only computed while this is on. A line under the readout shows which processor runs each model, so a silent fall-back to the CPU is visible.

What to expect: "happy", "surprised" and "neutral" are the reliable labels. "Fearful" and "disgust" are the rarest in the model's training data and the ones it confuses most; "angry" and "sad" are often a resting face. On your photo library the readings are 40-45% neutral, 40-45% happy and the rest spread thinly, which matches what the photos are.

`python tools\expr_check.py` scores a sample of your stored faces (time per face, label spread, how often the mirrored views agree, broken down by label, face quality and face size) and runs one tracker request on a stored photo: a quick check that the models are installed and working. On your photos, blurry faces get a "clear" reading just as often as sharp ones, which is why blurry frames get no reading at all.

Expression isn't stored anywhere; it's only shown live. The one exception is the **Expression test**, which you start yourself, to measure how well the reading works on your own face and webcam. The page asks you to make each of the 7 expressions for a few seconds, then to talk for a few seconds (neutral, then smiling), then to rest your chin on your hand, and for each readable frame it saves only numbers (the expression asked for, whether you were asked to talk, the model's scores, the face-shape cues, the talking signals, the body points and whether a hand was on the face, quality, head turn, face size and brightness; no pictures, no audio) to `data\expr_eval\`. `python tools\expr_eval.py` then shows, per expression, how often the model got it right frame by frame and after the vote, what it was mistaken for, and which cues separate the expressions on your face. That's the measurement needed before any expression setting is changed or the cues are allowed to affect the label. Posed faces are exaggerated, so treat the results as a best case. **Delete test data** (or deleting the folder) removes them, together with feeling reports.

### Other signals

A face's expression is only one channel, and a weak one on its own (see [Why expression reading is layered](../docs/WHY_LAYERED_EXPRESSION.md)). With **Show expression evidence** on, a second line under each face shows signals measured separately, in `signals.js`:

- **Motion:** how fast the head moves (face widths per second), and whether it's coming closer or moving away.
- **Talking:** how much the jaw moves over the last 1.5 seconds ("jaw moving" / "jaw still"). With the microphone on, the jaw has to move *and* there has to be voice before the face counts as **talking**. Talking matters because speech makes mouth shapes the expression model reads as "surprised" or "happy".
- **Blinks per minute** (rough: about 10 checks a second can miss short blinks), **gaze away** (how often the eyes look well off-centre), **smile rise** (how fast a smile came on), **smile asym** (left/right difference while smiling) and **held** (how long the current reading has lasted). Spontaneous smiles tend to rise smoothly and be symmetric; posed ones less so.
- **Webcam pulse** (off by default): heart rate from tiny skin-colour changes in the forehead and cheeks (the POS method of remote photoplethysmography). It needs a still face and steady light, and shows its signal-to-noise ratio; below 0 dB it's marked "weak". Treat it as experimental.
- **Body posture** (off by default): a third evidence line, starting "body:", and the body's outline on the video (with Keypoints on). It shows:
    - **head**: how high the nose is above the shoulders, in shoulder widths. Lower means head down or slumped.
    - **neck**: how high the ears are above the shoulders. Smaller means raised shoulders, as in a shrug or tension.
    - **tilt**: the shoulder line's angle, and **leaning in** / **leaning back** compared with the last 30 seconds.
    - **body**: how much the shoulders and hands move. More movement goes with higher arousal.
    - **breathing**: breaths per minute from the slow rise and fall of the shoulders, only while you sit still, with its signal-to-noise ratio.
    - **hand on face** and **arms crossed**.

  For intense emotions the body often tells more than the face, and movement tracks arousal well. A posture still doesn't map to one emotion: a slump can be sadness or tiredness. That is why these too are evidence, and part of the feeling reports.
- **Listen to voice** (off by default): the browser analyses your microphone live, never recording it. It shows how much of the time someone is speaking, loudness above the room's background noise, pitch and its variability (in semitones), and speaking rate (loudness peaks per second, about syllables). Every second that contains speech, the last 2 seconds of audio go to the tracker program on this computer, where the voice model scores arousal and valence ("voice A.62 V.48"). The audio is thrown away after scoring. The voice is attributed to whoever is talking, or to the only face in view.

Microphone tips:
- Use Chrome or Edge. The page records at 16 kHz, which these browsers support.
- The first time, allow microphone access when the browser asks. If you blocked it, use the lock icon in the address bar.
- A headset works best. Without one, sound from your speakers (a video, a call) can count as voice.
- If the voice line never appears, check that the black Face Tracker window says "Voice model: wav2small.onnx". Without the model you still get voice activity, loudness, pitch and rate, but not "voice A.. V..". To install it, run `python tools\get_wav2small.py`.

More on the voice measures, the model and its limits: [Voice](../README.md#voice) in the repository README.

**Each person's normal** (with **Calibrate to each person** on): what counts as a low head, fast speech or a high pulse differs a lot between people. A naturally soft-spoken person isn't sad, and a habitual slouch isn't dejection. So the tracker also learns, for each named person, the usual level and spread of every signal: head height, neck, shoulder tilt, body and head movement, smile, jaw, blinks, gaze, pitch and its variability, loudness, speaking rate, voice arousal and valence, pulse and breathing. It takes one sample a second, and a person's normal is trusted after about 2 minutes of them in view; after that it keeps adjusting slowly (about 10 minutes' memory). It learns separately for when the person is **alone** in view and **in company**, because people sit, move and speak differently with others, and uses their overall normal until the one for the situation is learned. With evidence on, a line under each face then shows what stands out: "unusual for Chris (alone): head -2.3 SD, pitch +2.1 SD" (SD: spreads away from their normal; 2 or more is shown), "usual for Chris", or "learning Chris's normal 40%" at first. All of it stays in this browser. **Forget what's learned** clears it for everyone, or for the person picked next to it. Feeling reports also note whether others were in view.

These signals are evidence only. They never change the expression label, with three exceptions that are off by default because they aren't measured yet:

- **Skip expression while talking:** frames where the face is talking cast no expression vote ("no reading: talking"), the same way blurry frames don't.
- **Skip expression when a hand is on the face:** a hand on the chin or mouth hides part of the face, so those frames cast no vote ("no reading: hand"). Needs Body posture installed.
- **Use state model for expression:** instead of averaging the last ten frames, a small hidden-Markov "state model" keeps a running belief about each face's expression: it assumes an expression usually continues, and blurry or turned-away frames simply hold the belief. The evidence line shows the state model's answer ("state happy 78%") whenever it disagrees with the vote.

Run the Expression test and `tools\expr_eval.py` before turning either on: it reports how both would have done on your own face.

**Feeling check** (in the Expression test card): the expression on a face isn't the same as how someone feels, and the only way to find out whether any of these signals track your feelings is to ask. Press **Report how I feel**, or choose **Ask me every 2 / 5 / 10 min**. You answer two 1-7 scales (unpleasant to pleasant, calm to energized), saying who is answering. The answer is saved together with numbers summarizing the 30 seconds *before* the question: average expression shares, face-shape cues, motion, talking, blinks, gaze, voice and pulse. Pictures and audio are never saved. Watching something that moves you while the feeling check asks every few minutes gives spontaneous data, which is more honest than posed expressions.

`python tools\expr_eval.py` now also:
- compares the state model with the vote on your test sessions;
- calibrates confidence: it measures how often a reading at a given share is actually right and fits a correction, saved to `data\expr_calibration.json`. From then on the evidence shows "cal 64%", meaning readings like this are right about 64% of the time;
- breaks accuracy down by person, face quality, face size, head turn and brightness;
- measures the body channel: how well the hand-on-face detector works and what a hand on the chin does to the reading, and posture by expression;
- measures talking: how often talking faces are misread, how well the jaw-movement detector and the microphone tell talking from silence, and the best talking threshold on your data;
- from your feeling reports, shows which signals go with how you said you felt. Once there are 15 reports for a person (or 20 in all), it fits valence/arousal to that person and keeps the fit only if it predicts your answers better than always guessing your average. The fit is saved to `data\va_fit.json`, and the evidence then shows "V+0.3 A+0.5 fit" instead of the textbook placement.

On the first test session (Oct 30), "happy" and "neutral" were read right 98% of the time after the vote. Your posed "angry" read as disgust (84%) and "fearful" as happy (85%), and the model was strongly overconfident (fitted temperature about 6). That is exactly what calibration and feeling reports are for.

Keep the black **Face Tracker** window open while you use it.

## Label people: label_tool.py (separate app)

Double-click **label.bat** (macOS, Linux: `./label.sh`), or run `python label_tool.py`. Your browser opens **Face Labels**. Every change is saved at once, and **Undo** (or Z) takes back the last one.

**Add photos** reads a folder (subfolders too, photos and videos) in the background, using the GPU. It skips photos it has already read, even renamed or moved ones, and copies of photos (re-saved or resized) whose faces match. When a new face clearly matches someone you've named, it's named automatically. The rest wait for you in two lists:

- **Suggestions**: faces the model thinks it knows, per person, least certain first. Click the wrong ones (or drag a box across them), press **R** ("not this person"), then **Accept** the rest.
- **New faces**: faces that look like nobody you've named, in groups of look-alikes. Name a group, or select some of its faces to name only those. **Ignore** strangers, artwork and tattoos so they aren't asked about again. An ignored face's print and picture are deleted (**Undo** brings them back until the labeling tool is closed); only where it is in its photo is kept, so the photo, or a copy of it, isn't read again and the face doesn't come back. So look-alikes of ignored faces aren't suggested as "ignore" any more, and the ignored list shows grey placeholders. Faces ignored before v1.9 lose their prints and pictures the first time the labeling tool starts. One copy of the face files from just before is kept in `data\backups\before-v1.9-ignore-<date>\` (delete it once you are happy; forgetting a person deletes it too, since it holds everyone's prints).

**Webcam faces**: while it's open, the labeling tool checks `%APPDATA%\FaceLabels\webcam\` every 20 seconds and imports any new face pictures the tracker saved with **Collect faces for labeling**. It does this the same way as **Add photos**: faces it's sure of are named automatically, the rest appear in **Suggestions** or **New faces**, and near-identical pictures are skipped. A webcam face nobody has named shows a **Live tracker** clue with who the tracker thought it was and its match percentage. **Webcam faces named automatically don't teach the recognizer until you confirm them** (in **Named automatically**). Many snapshots of one visit arrive at once, so one wrong automatic name would otherwise make the next snapshots of that person look like the wrong person too.

Also:
- **Named automatically**: check the automatic names, if you like, least certain first.
- **Checks**: named faces that look more like someone else, people with fewer than 5 clear faces, and the duplicate photos that were skipped.
- **People**: click a person to see all their faces, least typical first, so mistakes are at the top. **Rename or merge** fixes a name or combines two people. **Photos…** lists every photo the person is named in, newest first. Type more names under **Together with** to find photos where they're all named. Click a photo to open it, or type a folder and **Copy photos** to copy them there, keeping their file dates. Photos already in that folder are skipped, and photos whose originals aren't on this computer are counted but can't be copied.
  **Their data…** shows what's held about the person and does two things. **Export** writes all of it into a new folder inside the one you type: `data.json` (their faces with photo, box, label and face print; faces marked "not them"; the labeling history about them; saved Web search answers; photo tags; descriptions that name them; their presence log visits, Expression test frames and feeling reports; per-person fitted numbers), their face pictures (`faces\`) and their Collect faces pictures (`webcam\`). **Forget** (type their name, then confirm) deletes all of it and removes their name everywhere: labels and the "not this person" lists, face prints and pictures (`faces.npz`, `thumbs.npz`, `people.npz`), `history.jsonl`, the `labels.json` copies in `data\backups\`, `web_suggestions.json`, `photo_facts.json`, `expr_eval\`, `expr_calibration.json`/`va_fit.json`/`webcam_eval.json`, the contact sheets in `data\sheets\` (made again by `cluster`), and in `%APPDATA%\FaceLabels\` the presence log, Collect faces pictures, descriptions that name them and `app.log`. Their faces stay as Ignored faces without print or picture, so their photos aren't read again. Undo is cleared and it can't be undone. What the tracker page learned about them (usual look, normal signals) is in that page's browser storage: in FaceRecognizer.exe's window the Face tracking tab forgets it at the same time; with the tracker open in a browser, use **Forget what's learned** there.
- Double-click a face (or press Space) to see the whole photo with everyone in it boxed and named; ← → step through faces. Click a box to switch to that face.
- **Same picture, one tile**: a face that's the same picture as faces in other photos (an avatar in many screenshots, burst shots, re-saved copies; face prints at least 0.95 alike) shows as one tile with a **×N** badge, in every list. Selecting, naming, ignoring, accepting or rejecting it does the same to all N. Different photos of one real person are rarely that alike (0.06% of pairs), so different photos still get their own tiles. In this library 1,722 faces fall into 783 such stacks.
- **Possible faces**: every photo is looked at twice. Faces the detector is at least 85% sure of are kept as before; faces it's 45% to 85% sure of, or finds only at other sizes, are kept too, as **possible faces**: blurry, turned, small or partly hidden faces, and some things that only look like faces. They have their own list under **To review**, so **New faces** isn't buried in them. Most of them aren't faces (in a hand-sorted sample of 160, only 24 were real faces), so the list is sorted by how much each looks like a real face: a small model learns that from your library each time faces are added (faces the normal detector found count as real, possible faces as mostly not; possible faces you name teach it too; ignored ones can't any more, since an ignored face's print is deleted). Only the most face-like quarter, under **Look like faces**, get **dashed yellow boxes** in the photo view, labelled with how sure the detector is ("Face? 73%"); the rest are under **Less likely to be faces**, without boxes. On the hand-sorted sample this kept boxes on 17 of the 24 real faces and took them off about 9 in 10 non-face objects (food, tyres, hands, screens). Drawings, paintings and game characters mostly keep their boxes: to the recognition model a drawn face is a face. Name them now or later; once one looks like someone you've named it moves to **Suggestions**, and **Ignore** anything that isn't a face. Possible faces named automatically don't teach the recognizer until you confirm them (like webcam snapshots). Photos added before possible faces were kept: **Look in N photos added before** in the Possible faces list (about 6 photos a second, so roughly 25 minutes for 8,000; progress at the top; videos and online-only cloud photos are skipped). In a sample of 1,378 photos this found 732 possible faces. In an earlier sample of 400, about half of those scored 0.55-0.85 were real faces and half food, tyres, drawings and pets. The photo view also looks at the photo you're viewing on the spot (dashed boxes not saved yet; click one to add it, **Add all possible faces** adds every one; **Show possible faces** turns them off). A face hidden too much (behind someone else, or a hand over most of it) may still not be found: the detector scores those like junk.
- **Draw a face** (photo view, or **B**): for a face the detector misses even as a possible face, for example one mostly hidden behind someone (the detector scored one 0.14, the same as junk). Drag a box around it; when you let go it's added as a face and opens, ready to name (**N**), or name it later under **Possible faces**, where it's marked **Drawn**. **Esc** cancels. The eyes, nose and mouth that line the face up for recognition come from the detector looking just inside your box; if it sees nothing there, they're placed where they sit in an average face. A box mostly on top of a face that's already there is refused. Photos only, not video frames. Like possible faces, a drawn face named automatically doesn't teach the recognizer until you confirm it.
- **The photo, described** (photo view): **Describe photo** (or **D**) asks a vision model running in **LM Studio** on this computer to describe the photo in 2-3 sentences (setting, occasion, how many people and what they're doing) and to read any text in it (signs, shirts, cakes, captions, dates), which often tells you the occasion or who someone is. With the default settings nothing leaves the computer. **Every photo** describes each photo as you open it. **Describe all photos** describes every photo not described yet in the background (progress at the top; **Stop** pauses it and it carries on from there next time; videos, duplicates and online-only cloud photos are skipped): about 3.2 seconds a photo, so roughly 7 hours for 8,000. It sends one photo at a time on purpose: with three at once, LM Studio slowed to a crawl (1.3 instead of 48 tokens a second each, about 45 s a photo). Other programs using the graphics chip at the same time (another local AI app, for example) slow it down too. The model is told not to guess who anyone is. Each description is saved once per photo in `%APPDATA%\FaceLabels\descriptions.json` (not in `data\`); **Describe again** asks anew. Needs LM Studio running with its local server on (port 1234) and a vision model downloaded, for example Qwen 3.6 35B-A3B: about 3 seconds a photo, and about 40 seconds the first time while LM Studio loads the model.
  **Settings → Describe photo** chooses what describes: **Use** (Automatic: the server, or the built-in model when it doesn't answer; the server only; the built-in model only), the **Server address** (empty for `http://127.0.0.1:1234`, or `FACE_LLM_URL` if set), the **Model** (Automatic picks LM Studio's loaded vision model; or pick one) and an optional **API key**; **Test** shows which models the server has. Any server with the same OpenAI-style API works: LM Studio, Ollama (port 11434), llama.cpp, vLLM, Jan. Other servers don't say which of their models read pictures, so pick a vision model there. The server can be on another device, on your network or on the internet: then **each photo you describe is sent to that device** (the Settings tab says so, and warns when it's an unencrypted `http://` address on the internet: use `https://`, and only a server you control). For LM Studio on another computer, turn on **Serve on local network** there. The choices are kept in `%APPDATA%\FaceLabels\describe.json`.
  **Without LM Studio**, the panel offers **Download the built-in model** (0.8 GB, once, from Hugging Face; your photos aren't sent anywhere): Microsoft's Florence-2 large (MIT licence), which runs inside the app on the processor, about 2-3 seconds a photo. Its descriptions are plainer and less accurate: it says what it sees (a man in a cap, a jeep in a parking lot) but not what the picture is (a meme, a game screenshot, a scanned letter), and sometimes gets things wrong; it reads large, clear text well and small or curved text badly. Descriptions show which model wrote them; when LM Studio is running it is always used, and **Describe again** replaces a built-in description with LM Studio's.
- **Layout:**
  - The sidebar (the lists and **People**) slides away with the **☰** button or **Ctrl+.**, so the faces get the whole width. The page remembers whether it was open. On narrow windows it slides over the faces instead, and closes when you pick something.
  - **Find a person** filters the people list; **Enter** opens the first match.
  - **Size** (top bar) makes the face pictures bigger or smaller, which helps when judging blurry faces. It's remembered.
  - **People order** (under **Find a person**): A–Z, **Most faces first**, or **Needs more faces first**. The last puts first the people with fewer than 5 clear faces, too few to be recognized reliably, with the fewest at the top. The **People** heading says how many need more. It's remembered.
  - **webcam** on a face picture marks a snapshot from the live tracker. Hover over it to see whether it was named automatically and still needs your confirmation before it teaches the recognizer.
  - **?** (top bar, or the **?** key) lists every keyboard shortcut.
  - Each list and person has its own address, so the browser's **Back** and **Forward** and reloading the page keep your place.
  - The progress bar for webcam snapshots only shows while new ones are being added, and for a minute after.
- **From photo tags**: faces named by the photo itself. Face tags saved inside the photo or in a `.xmp` file beside it (Windows Photo Gallery, Lightroom, Picasa, digiKam; the face boxes an iPhone saves carry no names) are matched to the face they sit on. Names without a box (Google Takeout `.json` files, IPTC "person in image") are used when only one face in the photo is still unnamed. The tags and camera dates are read in the background and saved in `data/photo_facts.json`; after that a photo is read again only if the file, a `.xmp`/`.json` beside it, or the faces found in it change. Nothing is named until you accept it.

**Clues for a face nobody has named** (photo view, under the buttons). Clicking a name names the face; **Z** undoes it.
- **Live tracker** (webcam snapshots only): who the live tracker thought the face was when it saved it, and its match percentage, or that it didn't recognize the face.
- **Closest people (weak guesses)**: the three people the face scores highest against, with the score, even far below the suggestion setting. A hint only: below 0.35 most of these are wrong.
- **The photo's own tags**, when it has any.
- **Same occasion**: who is named in photos taken within 12 hours of this one (camera date), or, for photos without a camera date (downloads, scans), the 6 photos either side of it in its folder. **⤢** opens that person's face in that photo.

**Web search** (photo view, one click): sends this face to **Amazon** and this photo to **Google**, both US companies, and answers in one line. Either it gives a **suggestion with a confidence level** (click **Name this face …** to open the name picker with it filled in, then press Enter), or it says **No suggestions found**. Results are in English, and only US sites are used.

| What was found | Confidence |
|---|---|
| Amazon's celebrity recognition knows the face, and Google finds this exact picture on pages naming the same person | very high |
| Amazon knows the face with a match of 95% or more | high |
| Amazon, 85–95%, or Google finds this exact picture on web pages naming someone (no face match) | medium |
| Amazon, below 85% | low |

The bands are for reading, not measured on this project's faces. Tested 2026-11-01: a phone photo of a TV still came back "Javier Bardem, high confidence (99.4% match)", and a Bernie Sanders DJ meme "Bernie Sanders, high (100%)". Amazon only knows well-known people and never names private people, so most family photos get **No suggestions found**. That is the honest answer, not a failure. Google never names anyone from the face itself; it only helps when the same picture was posted online with a caption. Pages that share only *part* of the picture (usually just a similar scene) and Google's relevance weights are not used for suggestions.

**Setup (optional; your own accounts).** Each service is optional and uses your own key. Paste the keys under **Settings and other ways to search** in the Web search dialog; they're kept in `%APPDATA%\FaceLabels\` on your computer only, never in the project. Without keys, **Copy face, open Google Lens** still works.
- Amazon: in the AWS console, create an IAM user whose only permission is `rekognition:RecognizeCelebrities` (an inline policy), create an access key for it, and paste the key ID, the secret and the region (for example `us-east-1`). Kept in `aws_rekognition.json`. An AWS budget with e-mail alerts is a good idea. About $1 per 1,000 searches.
- Google: in a Google Cloud project with billing, enable the Cloud Vision API, create an API key restricted to that API, and paste it (or set the `GOOGLE_VISION_API_KEY` environment variable). Kept in `google_vision_key.txt`. 1,000 free searches a month, then $3.50 per 1,000.
- Budgets only warn, so the tool itself stops each service before $10 a month: 9,000 Amazon searches and 3,700 Google searches per calendar month (`CAPS` in `websearch.py`). The counts are shown under each search.

**Settings and other ways to search** (in the dialog): **Copy face, open Google Lens** copies the face and opens Google Images; click the camera icon and press Ctrl+V. (Google only accepts pictures from its own page.) **Save face picture** keeps a copy.

**Each face is searched once.** The answer (suggestions or none, with the date) is saved in `data/web_suggestions.json` by face ID, shown under the face's clues from then on, and returned again without sending anything when you click **Web search** on that face. **Search again** asks anew (and counts toward the limits). Only complete answers are saved: if a service failed, the next click tries again. Like the rest of `data/`, it stays on this computer.

To try it without the page: `python tools\websearch_check.py picture.jpg [--face x0,y0,x1,y1]`.

**This is the only feature that sends anything off your computer**: the face goes to Amazon and the photo to Google, which may keep them, and only when you click **Web search**. Tried and dropped: Yandex (named people well, but it's non-US and returns mixed languages), Bing (only describes a picture), and TinEye (robot check).
- Keys: **A** accept, **N** name, **R** not this person, **X** ignore, **Z** undo, **Esc** clear the selection. Shift-click selects a range.
- Faces too blurry or small to learn from are hidden from the lists; **Show blurry faces** brings them back.

How sure counts as sure: a face is named automatically when it scores 0.45 or more with its best match and at least 0.10 more than the next person. On your labeled faces that named 89% with no mistakes, and no stranger scored above 0.35. It's suggested from 0.35 (the recognizer's match setting). These are `AUTO` and `MARGIN` in `labels.py`.

The live tracker picks up new names by itself. Close the black window when you're done.

## From the command line

```
python face_id.py scan "E:\Users\you\Pictures\New"    # same as Add photos
python face_id.py cluster                                 # groups of faces nobody has named, as contact sheets in data\sheets (local only)
python face_id.py name 4 9 15 Chris                       # name groups 4, 9 and 15
python face_id.py name 7 --ignore                         # group 7 isn't anyone to recognize
```

## Your hardware: what the app uses

The app looks at the computer when it starts and picks, job by job, the fastest way it has that gives the same answers. **Settings → This computer** shows what it found (processor, memory, graphics processors, NPU) and where each job runs, with the start-up check behind each choice.

| Job | Where it runs |
|---|---|
| Face finding (YuNet), recognition (ArcFace), photo quality, expression | a graphics processor (GPU) when there is one and it passes the check below, else the processor (OpenCV) |
| Reading and decoding photos | the processor, on several threads at once (Settings shows how many); JPEGs are decoded straight at reduced size, each file is read once |
| Videos | the processor decodes them; about one frame a second is looked at, and its faces go through the jobs above |
| Face-shape cues, body posture (MediaPipe), voice | the processor (about 10 ms a face, 15 ms a frame, a few ms per 2 s of sound) |
| NPU | not used by itself: measured slower than the GPU and the processor for one face at a time |

Which GPU route, by system:

| Hardware | Route |
|---|---|
| Windows with a DirectX 12 GPU (AMD, NVIDIA, Intel) | DirectML (`onnxruntime-directml`, installed by setup.bat) |
| Linux with an NVIDIA card | CUDA: `./setup.sh` installs `onnxruntime-gpu` with its own CUDA and cuDNN libraries when `nvidia-smi` works (no CUDA toolkit needed) |
| Mac with Apple Silicon | CoreML |
| anything else | the processor |

**The check:** each model runs once on the GPU and once on the processor with the same test input. The GPU is used only if its answer is within 1% of the processor's and it is faster (median of 10 runs, after warming up). Face finding is compared with OpenCV's own detector on a photo-sized picture. The decisions are printed at start, for example `arcface_w600k_r50.onnx: CUDA: same results (0.10% apart), 4.7 ms vs 22.1 ms on the CPU: GPU`. If the GPU fails later (for example it runs out of memory), face finding switches to the processor and says so. Face finding on a GPU needs one more model file, `face_detection_yunet_2023mar_dynamic.onnx`, which `fetch_models.py` downloads; without it faces are found on the processor.

To choose yourself, set `FACE_DEVICE` before starting: `gpu` (the GPU without the check), `cpu`, or `npu` (needs the Ryzen AI environment's Python, `X:\ProgramData\miniconda3\envs\ryzen-ai-1.8.0\python.exe`; `tools/hw_bench.py` compiles the NPU model and re-measures). `FACE_IMPORT_WORKERS` sets how many photos an import works on at once.

**Measured** (ArcFace, one face at a time):

| Computer | GPU | Processor | NPU |
|---|---|---|---|
| Ryzen AI Max+ 395, Radeon 8060S (Windows, DirectML) | 2.6-3.6 ms | ~12 ms | ~25 ms (BF16, 99.97% identical; first start compiles ~6 min) |
| Ryzen 7 7840HS, RTX 4060 laptop (Linux, CUDA) | 4.7 ms | 22 ms | not available under Linux there |

**Adding photos**, 412 files from a network drive (180 HEIC, 156 JPEG, 64 PNG, 12 videos) on the Ryzen 7 7840HS + RTX 4060, through the app window:

| Version | Time |
|---|---|
| v1.9: 4 files read at a time, faces found one photo at a time on the processor | 89.5 s |
| photos worked on 12 at once, JPEGs decoded at reduced size, processor only (`FACE_DEVICE=cpu`) | 73.3 s |
| the same, with face finding on the GPU (the default there) | **43.5 s** |

What's left is mostly decoding: a 12-megapixel HEIC takes about 300 ms on the processor, a JPEG 66 ms, a PNG 90 ms; a video about 2 s. `tools/import_profile.py <folder>` times each step of an import on your own photos.

**Bigger models tried:** AdaFace IR-101 (WebFace12M, `tools/get_adaface.py` converts the official weights) ran at 8.5 ms per face on the GPU but didn't beat ArcFace on your photos: 95.5% vs 99.4% of labeled faces correct, about the same on blurry frames. Combining both models didn't help either (`tools/eval_models.py` reproduces the comparison).

## Switching recognition models

Face prints from different models can't be mixed. After changing models, run `python face_id.py reembed` (about a minute: it re-reads the saved face crops, no photo rescan needed). Your labels are stored per face, so they all carry over. To go back to SFace, set `FACE_MODEL=sface_2021dec` before running the commands.

## Tuning

- **Wrong names:** raise the match setting (the tracker's Match strictness slider, or `--threshold 0.4`).
- **Known people shown as Unknown:** lower it to about 0.3, or label more photos of them.
- **Measuring on your webcam:** `python tools/webcam_eval.py` scores the live tracker on your confirmed webcam snapshots: correct, Unknown, wrong person, and stranger named (that person left out of the known people). It also compares ways of combining frames. Results go to `data/webcam_eval.json` (numbers only). It needs webcam faces collected with **Collect faces for labeling** and confirmed in the labeling tool. The more people they cover, the more the stranger figure means (on 2026-11-08 they were all of one person).
- **New-face groups mixing two people, or one person split across groups:** select faces within a group to name just those; the grouping distance is `cluster` in `REC_MODELS` in `face_id.py` (0.70 with ArcFace).

## Files

- `data/faces.npz`: every face found, with its permanent ID, face print and quality (an ignored face keeps only its ID, photo and box: no print)
- `data/thumbs.npz`: a small picture of each face (if it's missing, the labeling tool makes each picture again from the original photo, or shows a grey placeholder)
- `data/photos.json`: every photo read: where it is, its fingerprint, and which photo a skipped copy duplicates
- `data/labels.json`: the label of each face (name, ignored, named by you or automatically, names it is not)
- `data/history.jsonl`: every labeling change, with the time; `data/backups/`: a copy of labels.json from each start of Face Labels (last 20)
- `data/people.npz`: face prints of the named people (what the tracker and `recognize` use)
- `data/settings.json`: photo folders you've added
- `data/photo_facts.json`: each photo's camera date and people tags (read again only when the photo, a file beside it, or its faces change)
- `data/web_suggestions.json`: the saved Web search answer for each face searched (suggestions or none, with the date)
- Outside the project, in `%APPDATA%\FaceLabels\` (the App section of **Settings** opens it):
  - the Web search keys (`google_vision_key.txt`, `aws_rekognition.json`) and this month's search counts (`vision_usage.json`, `rekognition_usage.json`). Never put the keys in the repository.
  - `describe.json`: the Describe photo settings (which describer, server address, model, optional API key); `descriptions.json`: the saved photo descriptions
  - `webcam\`: face pictures from **Collect faces for labeling** (each face alone, never the whole frame); `presence\`: the presence log, if you turned it on
  - `webview\`: the app window's browser storage (each page's options, per-person baselines and usual looks); `app.log`: the app's messages and errors
- The keys and `describe.json` are plain files readable by your Windows account; anyone who can sign in as you can read them.
- `websearch.py`: the Web search (Amazon + Google, combined); `tools\websearch_check.py`: the same search on picture files
- If an import or **Describe all** couldn't read some files, **Why?** beside the progress bar lists the last 20 and the reason for each (nothing is saved).
- `data/expr_eval/`: expression test sessions (`<date>-<time>.jsonl`) and feeling reports (`feel-<date>.jsonl`), only if you've run the Expression test or reported how you feel (numbers only: no pictures, no audio). **Delete test data** on the tracker page removes them.
- `data/expr_calibration.json`, `data/va_fit.json`: fitted by `tools\expr_eval.py` from those (a few numbers each; delete them to go back to the defaults)
- Live expression readings, voice and pulse aren't written to `data`. The expression model, MediaPipe and the voice model are optional (the tracker says so at startup if one is missing). The usual looks live in the browser's storage for the tracker page.

All of `data/` stays on this computer: it's in `.gitignore` and not in the releases, and a new copy of the project starts with an empty one.

**Where each setting is kept:** the Settings tab only shows them; each lives with the program it belongs to.

| Setting | Kept in |
|---|---|
| Face tracking options, alert names, Labeling view options (picture size, order, blurry, possible faces, describe every photo) | the page's browser storage: `%APPDATA%\FaceLabels\webview\` for the .exe window, the browser's own for the .bat pages |
| Photo folders added | `data/settings.json` |
| Web search keys | `%APPDATA%\FaceLabels\google_vision_key.txt`, `aws_rekognition.json` |
| Describe photo: describer, server, model, API key | `%APPDATA%\FaceLabels\describe.json` |
| Environment variables (optional) | `FACE_DEVICE` (auto, gpu, npu, cpu), `FACE_MODEL` (recognition model), `FACE_LLM_URL` (default describe server), `GOOGLE_VISION_API_KEY`, `FACE_ROOT` (another folder with `models\` and `data\`), `FACE_HOME` (another folder instead of `%APPDATA%\FaceLabels`), `FACE_PORTS` (the app window's ports, n to n+2 instead of 8764-8766, so a second copy with other folders runs beside the usual one) |

Databases from before per-face labels are upgraded automatically the first time Face Labels starts (or with `python face_id.py migrate "<photo folder>"`). The old group files are moved to `data/backups/legacy-<date>/`.

`data/backups/` holds the last 20 copies of `labels.json`; like the rest of `data/`, it's only on your computer, so back up `data/` yourself (it isn't in git).

## Checks and tests (for changing the code)

From `face-recognizer\`, with the Python 3.12 above and `python -m pip install -r requirements-dev.txt` once:

```
node --test tests/*.test.js           # the browser code, and that every page script parses and is served
python -m ruff check .         # Python checker (ruff.toml)
python -m pytest tests -q      # Python tests: no models, camera or network needed; never touch your data
```

On macOS and Linux use `.venv/bin/python` (after `./setup.sh`). GitHub runs the same on every push, on Windows, macOS and Linux, and then builds the app for all three (`.github/workflows/checks.yml`; see **Building the app**). The tests use a made-up `data` folder and
local test servers; `tests/test_thresholds.py` fails if a measured threshold changes, as a reminder to re-measure.

## More documentation

- [Repository README](../README.md): goals, key decisions and research value
- [Project context](../PROJECT_CONTEXT.md): background, history and measured tuning values
- [Why expression reading is layered](../docs/WHY_LAYERED_EXPRESSION.md): the science behind the expression design
- [Real-Time Facial Emotion Recognition](../Real-Time%20Facial%20Emotion%20Recognition.md): the expression design review
- [Face Tracker](../face-tracker/README.md): the original standalone browser tracker
- [AGENTS.md](../AGENTS.md) (read by [CLAUDE.md](../CLAUDE.md)): layout, conventions, security rules and tests, for coding assistants and contributors
