# Settings

Every setting in Face Recognizer: what it is for, its default, and where it is kept. They are all in the app
window's **Settings** tab (Ctrl+3), in the sections below, in the order the tab shows them: App; Face tracking
(camera, display, expression, voice/pulse/posture, recognition, alerts, presence log); Labeling (the face lists, Web
search, Describe photo); LM Studio; This computer; Shared database. Changes apply at once and are remembered; sections with
switches and choices have **Reset to defaults**. The same options also stay where they are used: the Face tracking
tab's **Controls** panel and the Labeling tab's toolbar. Run in a web browser instead of the app window
(`--browser`, or `recognize.bat` / `label.bat`), there is no Settings tab: use those controls on each page.

How settings are kept:

| Kind | Where | Examples |
|---|---|---|
| Page options | the page's own storage, and its program in `%APPDATA%\FaceLabels\settings.json` (per computer); with a shared database also this device's rows there | everything under Face tracking, Labeling's display options |
| Program settings | `%APPDATA%\FaceLabels\` (macOS `~/Library/Application Support/FaceLabels`, Linux `~/.local/share/FaceLabels`) | Describe photo (`describe.json`), Web search keys, the shared models folder (`lmstudio.json`), alerts |
| Start-up choices | environment variables (end of this page) | `FACE_DEVICE`, `FACE_PORTS` |

Keys and the shared database's password are shown in their fields masked (`*`); hold the eye button beside a field to see it. The same goes for the passphrase for keys.

---

## App

| Setting | What it is for |
|---|---|
| **Version**, **Check for updates** | The app's version. Check for updates asks GitHub for the latest release, only when clicked, and offers its download page. |
| **Faces, names and labels** (Open folder) | Opens the `data` folder: face prints, face pictures, labels, photo dates and tags. It stays on this computer; keep it out of any public repository. |
| **Keys, descriptions, snapshots, log** (Open folder) | Opens the private folder above: Web search keys, photo descriptions, webcam snapshots (Collect faces), alerts, the presence log, and `app.log` (messages and errors). |

The **Alerts** bell in the tab bar (Ctrl+4) lists alerts as New, Acknowledged and Archived; what raises them is set
under *Face tracking: alerts*.

---

## Face tracking: camera

| Setting | Default | What it is for |
|---|---|---|
| **Camera** | the system's default | Which camera the tracker uses: every camera the computer sees, by name (a phone used as a webcam included). The same camera starts by itself next time unless you last stopped it. |
| **Mirror view** | on | Shows the picture mirrored, as a mirror would. Switched on for a webcam and off for a video file when the source starts. |

## Face tracking: display

| Setting | Default | What it is for |
|---|---|---|
| **Keypoints (eyes, nose, mouth)** | on | The five points the detector finds on each face. |
| **Face mesh (triangles)** | off | MediaPipe's 478-point face mesh drawn over each face (needs the face landmarker model). |
| **Mesh strength** | Normal | How visible the mesh is: Light, Normal, Strong. |
| **Full body skeleton** | off | The body points the body posture model finds below the face, joined: shoulders, arms with rough hands, hips, legs and feet. Drawn only, never kept. Needs the pose model. |
| **Hand skeleton (fingers)** | off | The joints of every finger (21 points a hand) from MediaPipe's hand model, in the person's colour; about 10-20 ms more a frame on the CPU. Drawn only, never kept. Needs `models/hand_landmarker.task`. |
| **Motion trail** | on | A short trail behind each face showing where it moved. |
| **Show unknown faces** | on | Draw faces the tracker can't name, labeled Unknown. |
| **Blur unknown faces** | off | Blur the faces of people it doesn't know (for screen sharing or recording). |
| **Smoothing** | 0.5 (0 to 0.9) | How much the boxes are smoothed from frame to frame: higher is steadier but lags more. |

## Face tracking: expression

Expression is display only and never stored. The settings marked *evidence only* aren't measured yet: they're off by
default and don't change the expression label until `tools/expr_eval.py` shows they help.

| Setting | Default | What it is for |
|---|---|---|
| **Show expression** | on | The face's expression under its name ("looks happy"), read from sharp, camera-facing frames and averaged over the last second. |
| **Calibrate to each person** | on | Learns each named person's usual look (about a minute) and their normal for every signal; a reading that departs from it is highlighted, one that matches is marked "usual". Kept in the window's storage only. |
| **Show expression evidence** | on | What the expression is based on: the model's votes, face-shape cues, and why a frame was skipped. |
| **Skip expression while talking** | off | *Evidence only.* No expression reading while the mouth moves with speech. |
| **Use state model for expression** | off | *Evidence only.* Smooths the label with a model of how expressions change over time. |
| **Skip expression when a hand is on the face** | off | *Evidence only.* No reading while a hand covers part of the face. |
| **Ask how I feel (feeling check)** | Don't ask | Every 2, 5 or 10 minutes, asks how you feel on two 1–7 scales; the answers (numbers only) are the ground truth for measuring the signals. |

## Face tracking: voice, pulse and posture

Signals besides the face's expression, each with its own normal per person when **Calibrate to each person** is on.

| Setting | Default | What it is for |
|---|---|---|
| **Listen to voice (microphone)** | off | Voice features live (loudness, pitch, speaking rate) and the voice model's arousal and valence. Analysed as it arrives, never recorded. |
| **Webcam pulse (experimental)** | off | Heart rate from tiny skin-colour changes (rPPG); needs a still face in steady light; shows its signal quality. |
| **Body posture** | off | Head height, shoulders, lean, body movement, breathing, hand on face and crossed arms from MediaPipe's pose model. |

## Face tracking: recognition

| Setting | Default | What it is for |
|---|---|---|
| **Match strictness** | 0.35 (0.20 to 0.60) | How alike a face must be to a known person to be named. Higher: fewer wrong names, more Unknown. 0.35 is the measured best for ArcFace. |
| **Collect faces for labeling** | off | While a face stays in view, saves a sharp picture of that face every few seconds (up to 12 per face) with who the tracker thinks it is; never the whole frame, other faces greyed out. The Labeling tab adds them by itself, for you to confirm. |

## Face tracking: alerts

| Setting | Default | What it is for |
|---|---|---|
| **Alert me about these people** | none | Whom to be told about; Unknown means a face nobody is named for. |
| **They arrive** | on | Alert when one of them is first seen after being out of view at least the time below. |
| **They leave** | off | Alert when one of them has been out of view the time below. |
| **Their signals are unusual for them** | off | Alert when one of their signals is far from their own normal (needs Calibrate to each person; the thresholds are guesses, not yet measured). |
| **An unknown face stays** | off | Alert when a face nobody is named for stays in view the time below. |
| **"Arrive": out of view at least** | 10 min | How long someone must have been away for their return to count as arriving. |
| **"Leave": out of view for** | 5 min | How long someone must be out of view to count as gone. |
| **Unknown face: stays** | 30 s | How long an unknown face must stay. |
| **Unusual: at least** | 3 spreads | How far from their normal a signal must be (in spreads, their own standard deviations). |
| **Unusual: for at least** | 30 s | How long it must stay that far. |
| **Unusual: at most one alert a person every** | 30 min | Limits repeated unusual-signal alerts. |
| **Alert details: measure again after** | 15 s | Each alert keeps what was measured when raised and again this long after. |
| **Banner stays** | 30 s | How long the banner over the tabs stays (or until dismissed). |
| **Play a sound with alerts** | on | A short sound with each alert. |

## Face tracking: presence log

| Setting | Default | What it is for |
|---|---|---|
| **Keep a presence log** | off | Who was in view and when: names and times only, in the private folder's `presence\`. |

---

## Labeling

| Setting | Default | What it is for |
|---|---|---|
| **Face picture size** | 88 px (64 to 176) | Size of the face pictures in the lists. |
| **Order of the people list** | A–Z | Or most faces first, or those that need more faces first. |
| **Show blurry faces** | off | Faces too blurry or small to learn from are hidden from the review lists unless this is on. |
| **Show possible faces in photos** | on | Dashed boxes on things the detector thinks may be faces (blurry, turned, partly hidden); nothing is saved until you click one. |
| **Photo folders added** | (shown) | The folders already added; Add photos offers them as you type. |

---

## Labeling: Web search

Sends one face to Amazon (celebrity recognition) and its photo to Google (pages with the same picture), only when
you click **Web search** on a face. US services, English results, each capped under $10 a month.

| Setting | What it is for |
|---|---|
| **Keys** | Whether an Amazon and a Google key are saved. The keys are your own and stay in the private folder. |
| **Google Cloud Vision API key** | Your Google Cloud Vision key (web detection). Save an empty key to remove it. |
| **Amazon Rekognition key** | Your AWS access key ID, secret and region (default us-east-1). |

## Labeling: Describe photo

Describes a photo and reads its text with a vision model, without identifying anyone. Descriptions are kept in the
private folder (`descriptions.json`).

| Setting | Default | What it is for |
|---|---|---|
| **Describing with** | (shown) | Which model answers now, how many photos are described, and the built-in model's state. |
| **Describe every photo as it's opened** | off | Runs Describe photo on each photo you open in the photo viewer, with the describer set here. |
| **Describer** | Automatic | Automatic: the server, or the built-in model when the server doesn't answer. Or the server only, or the built-in model only. |
| **Server address** | `http://127.0.0.1:1234` | LM Studio, or another server with the same API (Ollama on 11434, llama.cpp, vLLM, Jan). Any server listed under LM Studio on the network can go here. A server on another computer receives each photo described; the section says so. Changing the address clears its API key. |
| **Model** | Automatic | Automatic: LM Studio's loaded vision model, else its first one. Other servers don't say which models read pictures: pick one. |
| **Check the server** (Test) | | Asks the server which models it has. |
| **API key** | none | Only if the server asks for one (for example behind a proxy). Belongs to the server address; sent only there. |
| **Share Describe all with other computers** | off | **Describe all** also sends photos to the other LM Studio servers on the local network that have the same vision model (so every description comes from one model): each takes the next photo as it finishes, one at a time, so a faster computer does more. **Each photo sent there is seen by that computer.** One that fails twice in a row is left out and its photo goes back to the others; this computer's server keeps going to the end. A single photo is described by the server above only. Measured: 40 photos in 136 s on one PC, 93 s with a second computer (RTX 4060 laptop on Wi-Fi) helping. |
| **Shared with** | (shown, when on) | The computers Describe all would share with now, and their addresses. |
| **Built-in model** (Download) | | Florence-2 large (0.8 GB from Hugging Face, checked by SHA-256): runs inside the app on the processor, so photos stay on this computer; plainer descriptions than LM Studio's. |

---

## LM Studio

LM Studio on this computer and on the local network: the servers Describe photo can use, and the vision models
kept the same on every computer.

| Setting | What it is for |
|---|---|
| **LM Studio on this computer** | Whether LM Studio's server is running, and on which port. While the app runs it keeps LM Studio serving on all network interfaces and announces it on the network with its port, unless the computer already does. Lists the vision models the other LM Studio servers (or the shared folder) have and this one hasn't, and where each would come from. |
| **Get the same models** (button) | Gets those models: copied from the **Shared models folder** when they're there (the same files, each checked by SHA-256); if that copy fails in any way or the model isn't there, downloaded from the internet with LM Studio (the same model and quantization). A bar at the top of the Labeling tab shows which, how far, and how long is left. |
| **Shared models folder** | A folder every computer can reach, for example on a NAS: `\\NAS\share\lmstudio-models` on Windows, `/mnt/nas/share/lmstudio-models` on Linux. Each computer with the app and LM Studio puts its vision models there (with a list of every file's size and SHA-256) and gets the ones it lacks from there first. Empty: not used. This computer's own setting (`lmstudio.json`). |
| **LM Studio on the network** | Every LM Studio server on the local network: each computer's name, its addresses (every network port) and port, the models it serves, which read pictures and which are loaded. Servers that announce themselves give their own port; the rest are found by trying LM Studio's port on each address of this computer's networks, only while Settings is open. |
| **AI model apps found** | Apps that can run a vision model for Describe photo: LM Studio, Ollama, Jan, llama.cpp, vLLM and others on their usual ports here, the server set under Describe photo, apps installed but not running (for LM Studio, its downloaded models), and the address other computers can use when the server listens on all interfaces. |

---

## This computer

What the app found on this computer and what it does with it. See the user guide's
[Your hardware: what the app uses](../face-recognizer/README.md#your-hardware-what-the-app-uses).

| Setting | What it is for |
|---|---|
| **Hardware** | The processor (and threads), memory, graphics processors, NPU, and the ways ONNX Runtime can run models here (DirectML, CUDA, CoreML, CPU). |
| **Where the work runs** | Which processor runs each job (face finding, recognition, photo quality, expression, face-shape cues, body posture, hands), with the start-up check behind each choice: a model runs on the graphics processor only if it gives the processor's answers (within 1%), faster. Also how many photos an import works on at once. To choose yourself, set `FACE_DEVICE`. |
| **NVIDIA CUDA** (button) | Shown on the downloaded Linux app when it finds an NVIDIA card it can't use yet (after checking the driver, 525.60 or newer, and the card). Downloads NVIDIA's CUDA 12 and cuDNN 9 and Microsoft's ONNX Runtime with CUDA from PyPI (about 1.9 GB, each file checked by SHA-256) into the app's data folder; used from the next start. Nothing is downloaded until clicked. |

---

## Shared database

Optional: keeps every device's faces, names, labels and private data in one PostgreSQL database (with pgvector) that
all your devices with Face Recognizer share, and each device's own settings there too. Without it, everything stays
in each device's own files, as before. See the user guide's
[Sharing between your devices](../face-recognizer/README.md#sharing-between-your-devices-shared-database).

| Setting | Default | What it is for |
|---|---|---|
| **Shared database** (status) | Not set up | The server and database, whether it's reachable, when it last checked, how many changes made here wait to be sent (while the server can't be reached), and this device's name. |
| **Connect** (server, port, database, user, password) | empty, 5432, `sentient` | Logs in over an encrypted connection, takes the server's own certificate authority from its `server_ca` table and from then on checks the server with it on every connection. The first time, this device's data is merged in and every other device's is taken (Joining). Leave the server empty and Connect to stop using it: this device keeps its copy. Kept in `%APPDATA%\FaceLabels\database.json`; the password is protected by Windows for your user (DPAPI; on macOS and Linux the file is readable by you only). |
| **Stream this camera to my other devices** (Face tracking: recognition) | on | Your other devices can watch this camera live in their Cameras tab, as this page shows it (names, boxes and every marker that's on), with its live readout. Only while one watches is it sent at ~10 frames a second (otherwise a frame every 2 s, to say it's on); over your local network, encrypted, to your devices only; never stored. Needs a shared database and photo sharing on. |
| **They move between cameras** (Face tracking: alerts) | on | An alert when someone this camera sees was on another device's camera in the last 10 minutes. Every device shows it. |
| **Share my photos with my other devices** | on (with a shared database) | Lets your other devices show the original photos and videos this device has, fetched when opened and never kept there. They ask this device over your local network on port 8767 (the app's ports + 3), encrypted (TLS, this device's own certificate, checked against the database), and only your devices can ask (requests signed with a key made from the database password). Photos by their content only, nothing else. Also lists the devices sharing now and their addresses. |
| **Copy settings from** | — | Takes another device's options for the Face tracking and Labeling pages, its describer choices and LM Studio shared folder (not its describe server address), in place of this device's. |
| **Passphrase for keys** (typed twice) | not set | The Web search keys and the describe server's API key are kept in the database encrypted with it (AES-256-GCM, key from the passphrase by scrypt), on the device before they're sent. Set it once, on any device: it reaches every other device by itself, sealed to each device's own key pair so only that device can open it. Typing a new one (twice) changes it: every key is encrypted again and every device takes the new one. Each device keeps it (`secret.key`, protected like the password), so the field shows it masked with the eye button. |
| **Passphrase on each device** | — | Each device's fingerprint of the passphrase it has, and whether it's the current one (one-way: a fingerprint can't be turned back into it); also each device's database password fingerprint. |
| **Restore an earlier passphrase** | — | Every passphrase ever set is kept (who set it, when). Choosing one makes it current again: every key is encrypted again with it and every device takes it. For undoing a wrong change. |

What is shared, and what stays each device's own:

| Shared by every device | Each device's own (in the database under its id) | Only ever sent, never taken by other devices |
|---|---|---|
| faces (prints, pictures, boxes), labels, photos (by content), photo tags and dates, Web search answers, descriptions, Collect faces pictures, alerts, presence visits, each person's usual look and signal normals, fitted expression numbers, keys (encrypted), forgotten people, Web search use per month | page options, describer choices, LM Studio shared folder, where each photo is on it (`photo_paths`), measured hardware and model numbers | the app log (kept 90 days), the undo history log, posed expression tests |

---

## Environment variables

Set before starting the app (or a program) to change how it starts.

| Variable | Values | What it does |
|---|---|---|
| `FACE_DEVICE` | `auto` (default), `gpu`, `cpu`, `npu` | Where the models run. auto: the graphics processor for each model that passes its start-up check, else the processor. gpu: the graphics processor without the check. npu: the Ryzen AI NPU (needs the Ryzen AI Python). |
| `FACE_MODEL` | `sface_2021dec` | Use the older SFace recognizer instead of ArcFace (then run `face_id.py reembed`). |
| `FACE_IMPORT_WORKERS` | a number | How many photos an import works on at once (default: processor threads − 2, 2 to 12). |
| `FACE_CUDA` | `off` | Don't use or offer the downloaded CUDA (Linux app). |
| `FACE_LMSTUDIO` | `off` | Don't keep LM Studio on all interfaces or announce it. |
| `FACE_LLM_URL` | an address | The default Describe photo server (before one is set in Settings). |
| `GOOGLE_VISION_API_KEY` | a key | Google key for Web search, instead of saving one. |
| `FACE_PORTS` | a port n | The app window's ports: n to n+2 instead of 8764 to 8766 (to run a second copy). |
| `FACE_UI` | `browser`, `none` | browser: open the app in the web browser instead of its window (same as `--browser`). none: start the servers only, with no window and no browser (for test copies; open `http://127.0.0.1:8764/` yourself). |
| `FACE_ROOT` | a folder | Another folder with `models` and `data`. |
| `FACE_HOME` | a folder | Another folder instead of the private `FaceLabels` folder. |
| `FACE_DB` | a PostgreSQL connection string | Use this shared database instead of the one set in Settings (tests and test copies). |
| `FACE_DB_SCHEMA` | a schema name | The schema in it (default `facerec`). |
| `FACE_PEER` | `off` | Don't share this device's photos with the other devices (no peer service). |
| `FACE_PEER_PORT` | a port | The peer service's port instead of the app's ports + 3 (8767). |

---

## What reaches the network

Nothing is sent anywhere by itself except as listed here.

| Setting or action | What is sent, and where |
|---|---|
| First start | Downloads the face models from their publishers or the project's `models-v1` release (checked by SHA-256). |
| **Check for updates** | Asks GitHub for the latest release, when clicked. |
| **Web search** | One face to Amazon and its photo to Google, when clicked, with your keys. |
| **Describe photo** on another computer's server | Each photo described goes to that server. |
| **Share Describe all with other computers** | During Describe all, photos go to the other LM Studio servers on the local network that have the same model. |
| **Built-in model**, **NVIDIA CUDA**, **Get the same models** (from the internet) | Downloads, when clicked: Hugging Face, PyPI, LM Studio's sources. |
| **LM Studio → LM Studio on the network** | While Settings is open: a connection attempt to LM Studio's port on each address of this computer's local networks, each server's model list, each computer's name (mDNS or NetBIOS), and an mDNS question for announced servers. Local network only. |
| **LM Studio on this computer** | While the app runs: announces this computer's LM Studio on the local network (mDNS). |
| **Shared models folder** | Copies vision models to and from that folder (your own NAS or share). |
| **Stream this camera to my other devices** | While on and the camera runs: this device's camera, as its tracker shows it, and its live readout go to your other devices that watch (Cameras tab), over the peer service. Who the camera sees (names and times) goes to the shared database for "moved between cameras". |
| **Share my photos with my other devices** | While on: this device answers your other devices' requests for its original photos (by content hash), on the local network, TLS. Opening a photo another device has asks that device for it. |
| **Shared database** | Once connected: every face, name, label, photo record, description, Collect faces picture, alert, presence visit, per-person signal and this device's settings go to that PostgreSQL server, and come from it, over TLS checked against the server's own certificate authority. Keys go only encrypted, and only once the passphrase is entered. |
