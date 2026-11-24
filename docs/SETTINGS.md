# Settings

Every setting in Face Recognizer: what it is for, its default, and where it is kept. They are all in the app
window's **Settings** tab (Ctrl+3), in the sections below, in the order the tab shows them: App; Face tracking
(camera, display, expression, voice/pulse/posture, recognition and this camera, alerts, presence log); Labeling (the
face lists, Web search, Describe photo); LM Studio; This computer; Shared database. Changes apply at once and are
remembered; sections with switches and choices have **Reset to defaults**. The same options also stay where they are
used: the Face tracking tab's **Controls** panel and the Labeling tab's toolbar. The programs started on their
own (`recognize.bat` / `label.bat`, or the `.sh` ones) have no Settings tab: use those controls on each page. (`--browser` opens the whole app, Settings tab included, in the web browser.)
The **Cameras** tab has no rows in Settings; its two choices are under [Cameras tab](#cameras-tab).

How settings are kept:

| Kind | Where | Examples |
|---|---|---|
| Page options | the page's own storage, and its program in `%APPDATA%\FaceLabels\settings.json` (per computer); with a shared database also this device's rows there | everything under Face tracking, Labeling's display options |
| What the tracker learned about each person | `%APPDATA%\FaceLabels\person_signals.json`, a copy in the page's storage; with a shared database also there | usual look and signal normals (Calibrate to each person) |
| Program settings | `%APPDATA%\FaceLabels\` (macOS `~/Library/Application Support/FaceLabels`, Linux `~/.local/share/FaceLabels`) | Describe photo (`describe.json`), Web search keys (keys protected for your user, like the database password), the shared models folder (`lmstudio.json`), alerts, photo sharing (`peer.json`) |
| Shared database connection | `%APPDATA%\FaceLabels\` | `database.json` (server, user, protected password), `database-ca.pem`, `device.json` (this device's id and name), `secret.key` (the passphrase, protected), `device-key.json`, `peer-cert.pem`, `peer-key.pem` and `peer-key.json` (this device's own keys; the private ones protected), `sync.sqlite` (what was last synced) |
| Window conveniences | the page's storage only, per computer | the tab last shown, the Controls panel and Labeling list open or closed, the Cameras tab's choices |
| Start-up choices | environment variables (end of this page) | `FACE_DEVICE`, `FACE_PORTS` |

Keys and the shared database's password are shown in their fields masked (`*`); hold the eye button beside a field to see it. The same goes for the passphrase for keys.

---

## App

| Setting | What it is for |
|---|---|
| **Version**, **Check for updates** | The app's version. Check for updates asks GitHub for the latest release, only when clicked, and offers its download page. |
| **Faces, names and labels** (Open folder) | Opens the `data` folder: face prints, face pictures, labels, photo dates and tags. It stays on this computer; keep it out of any public repository. |
| **Keys, descriptions, snapshots, log** (Open folder) | Opens the private folder above: Web search keys, photo descriptions, webcam snapshots (Collect faces), alerts, the presence log, and `app.log` (messages and errors). |

**Privacy policy** (a link in the Face tracking tab's Controls, under the camera's choice) shows the
[privacy policy](../PRIVACY.md) inside the app, over the page, without the internet; the camera goes on behind it.

The **Alerts** bell in the tab bar (Ctrl+4) lists alerts as New, Acknowledged and Archived; what raises them is set
under *Face tracking: alerts*.

Not settings, but the app looks after itself: a program that failed or stopped shows **Start again**; a window whose
page stops answering for 30 s is brought back (and the app starts again in place if it stays silent); and when
**Keep this camera running** finds the camera not running (a start hanging over 30 s or a camera on with no picture
for 30 s, twice; or a minute of starts refused at once), the tracker page loads again, and if it still isn't running
the app starts again (at most once an hour, in the app window only; not for a camera that isn't there, isn't allowed
or is busy in another program). What it did is in `app.log` (**Tracker page:** lines), which also gets a line every
half hour with the memory the window's processes hold.

---

## Face tracking: camera

On a phone the camera that faces you is the one it starts with, and a round button on the picture (bottom right)
switches between the front and the back camera. A camera you chose stays chosen, as on a computer.

| Setting | Default | What it is for |
|---|---|---|
| **Camera** | the system's default | Which camera the tracker uses: every camera the computer sees, by name (a phone used as a webcam included). The same camera starts by itself next time unless you last stopped it (with **Keep this camera running** on, it always starts). The camera you picked stays this device's camera: at a start where it isn't there (unplugged, or not ready yet) the default camera is used meanwhile, and the picked one is gone back to as soon as it is there again. |
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

Expression readings are shown live and, with a shared database, kept there (see **Keep expression readings**); each alert also keeps what was measured when it was raised. The settings marked *evidence only* aren't measured yet: they're off by
default and don't change the expression label until a measurement on posed sessions shows they help.

| Setting | Default | What it is for |
|---|---|---|
| **Show expression** | on | The face's expression under its name ("looks happy"), read from sharp, camera-facing frames and averaged over the last second. |
| **Calibrate to each person** | on | Learns each named person's usual look (about a minute) and their normal for every signal; a reading that departs from it is highlighted, one that matches is marked "usual". Kept on this computer (`person_signals.json` in the private folder, and the page's storage); with a shared database every device shares them. **Forget what's learned** (Controls panel) clears it. |
| **Show expression evidence** | on | What the expression is based on: the model's votes, face-shape cues, and why a frame was skipped. |
| **Keep expression readings** | on | With a shared database: every 5 seconds, what each named person's face looks like (the expression shown and its share, the model's score for every expression, valence and arousal, alone or in company, how well the face matched) is kept in the database's `expression_log`, for reports later. Numbers and short labels only, never a picture or sound; never for unknown faces. It records how a face looked, not what the person felt. Nothing is kept without a shared database. A person's readings are in their export and go when they are forgotten. |
| **Skip expression while talking** | off | *Evidence only.* No expression reading while the mouth moves with speech. |
| **Use state model for expression** | off | *Evidence only.* Smooths the label with a model of how expressions change over time. |
| **Skip expression when a hand is on the face** | off | *Evidence only.* No reading while a hand covers part of the face. |
| **Ask how I feel (feeling check)** | Don't ask | Every 2, 5 or 10 minutes, asks how you feel on two 1–7 scales; the answers (numbers only) are the ground truth for measuring the signals. |

## Face tracking: voice, pulse and posture

Signals besides the face's expression, each with its own normal per person when **Calibrate to each person** is on.

| Setting | Default | What it is for |
|---|---|---|
| **Listen to voice (microphone)** | off | Voice features live (loudness, pitch, speaking rate). Analysed as it arrives, never recorded. |
| **Webcam pulse (experimental)** | off | Heart rate from tiny skin-colour changes (rPPG); needs a still face in steady light; shows its signal quality. An estimate for your own information: Face Recognizer is not a medical device (the page says so under these options). |
| **Body posture** | off | Head height, shoulders, lean, body movement, breathing, hand on face and crossed arms from MediaPipe's pose model. |
| **Chart this device's sound (speakers and microphones)** | off | For the Signals chart. While it is on and the Face tracking page is open, every audio interface of this computer (each output: what the computer itself plays; each microphone: what sounds in the room) is listened to, 4 seconds at a time, one stretch after another, and measured in memory: how loud, the background's numbers, a tempo where music sounds, and one word for what was heard (music, speech, quiet...). Only those numbers and the word are kept, for 15 minutes, in the page's memory; the sound itself is never kept. Turned off, or the page closed, the listening stops within seconds. A microphone hears the room, so this is off until you turn it on. On a phone the app can't listen to audio interfaces itself: there the page's own microphone hears, and a few seconds at a time go to the phone's own server, which measures them the same way and keeps nothing; the phone's speaker isn't charted. |

## Face tracking: the Signals chart

The **Signals** button in the Face tracking tab's header opens a drawer under the video: every sensor reading of this
device over the last minutes as charts, one expander a section. **This device**: People in view, Camera (checks a
second, how long one takes; and in words what the camera gives: its picture's size and rate), Device, and a section
for each audio interface while its sound is charted. **Device** is the device itself: in words what it is (a
computer or a phone, a phone's model as its maker names it, its system, the app's version, its processor and
memory, where each model runs), and as
readings how much of its processor and of its memory is in use, how warm it is, its heat state, its battery and
whether it is on power. Each only where the device's own system says it, never a guess: a Linux computer and a
Raspberry Pi give degrees; a phone gives its system's own heat state (nominal, fair, serious, critical) and no
degrees; a Mac and a Windows computer give no temperature at all; a battery is charted where there is one. **One person**
(whoever was in view; pick one): Recognition, Expression, Face, Voice, Body and hands, Their normal. Closed, a section
is one line with its main signal; open, it has a small chart for each signal on one time axis. The green band is the
person's own usual range, orange a reading outside it, a gap a time without a reading; pointing at one chart reads
them all at that moment. A section has readings only while its own option is on (**Listen to voice**, **Webcam
pulse**, **Body posture**, **Show expression**, **Calibrate to each person**): one without readings says which
option is off and has a button that turns it on.

The chart takes a reading with every check of the camera's picture, as fast as the device checks (tens a second on
a computer with a graphics processor), whether the drawer is open or not, and keeps the last 15 minutes in the
page's memory only: nothing of it is stored, and it starts empty when the app starts. Its choices (drawer open, which
sections are open, 1, 5 or 15 minutes shown) are remembered in the window's storage, on this computer only. The marks
on its Events strip (in view, left, unusual signals, unknown face, music heard) are made by the chart from its own
readings; they are not the alerts.

## Face tracking: recognition

| Setting | Default | What it is for |
|---|---|---|
| **Match strictness** | the recognizer's measured setting (0.25 to 0.75) | How alike a face must be to a known person to be named. Higher: fewer wrong names, more Unknown. The default is the measured best for the app's recognition model. |
| **Collect faces for labeling** | off | While a face stays in view, saves a sharp picture of that face every few seconds (up to 12 per face) with who the tracker thinks it is; never the whole frame, other faces greyed out. The Labeling tab adds them by itself, for you to confirm. |
| **Keep this camera running** | on | Starts the camera when the app starts, and again within 10 s if it stops (unplugged, busy, an error), so this device's camera is always on in every device's Cameras tab. A camera that won't run (a start that hangs or is refused, or a camera that is on with no picture) is retried as described under [App](#app). Clicking Stop camera turns this off. |
| **Stream this camera to my other devices** | on | Your other devices can watch this camera live in their Cameras tab, as this page shows it (names, boxes and every marker that's on), with its live readout. Sent all the time at full rate (about 20 frames a second), watched or not, also while the window is behind others (a minimized or hidden window streams slower on a Mac); over your local network, encrypted, to your devices only; never stored. Needs a shared database and **Share my photos with my other devices** on (the peer service). |

## Face tracking: alerts

| Setting | Default | What it is for |
|---|---|---|
| **Alert me about these people** | none | Whom to be told about; Unknown means a face nobody is named for. |
| **They arrive** | on | Alert when one of them is first seen after being out of view at least the time below. |
| **They leave** | off | Alert when one of them has been out of view the time below. |
| **They move between cameras** | off | With a shared database: alert when someone on the alert list who was on another device's camera in the last 10 minutes is now on this one. Raised by the device they arrived at; every device shows it. Only people on the alert list are noted in the database for it (`sightings`, deleted after 20 minutes); with it off, nothing is noted. |
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

## Face tracking: location

Where a device is. Off for a new copy: while it is off, no location service is asked, no location is shown or
told to your other devices, and no row is written. Each device has its own switch (a phone's is in its Controls),
and **Copy settings from** never copies the switch or a typed place to another device.

| Setting | Default | What it is for |
|---|---|---|
| **Show and keep this device's location** | off | This device shows where it is: in the Location card of its Face tracking tab and, on your other devices, over its picture in their Cameras tab: the nearest town with its state and country, the coordinates, how sure the reading is and how old. With a shared database, its moves are kept there as its location history: a row when the switch is turned on, then a row each time the device has moved more than about 100 m (and more than the reading's own uncertainty), at most one a minute; a device that stays where it is adds nothing. Rows are deleted after 90 days. Without a shared database nothing is written. The place comes from the device's own location service: Windows (Settings, Privacy & security, Location must allow desktop apps), a Mac, an iPhone and an Android phone each ask you once. A computer has no GPS: its system places it by the Wi-Fi networks around it, so it needs its Wi-Fi switched on, and it says so beside its place ("Placed by the Wi-Fi networks around this computer"). With Wi-Fi off, or without any, a Windows computer is placed only roughly (to within kilometres, and it says "Placed only roughly") and a Mac not at all: switch Wi-Fi on, or type the place. Linux and a Raspberry Pi have no location service the app can count on, so there a place is typed. The town's name comes from a list of the world's towns inside the app; nothing is looked up on the internet. |
| **This device's place, typed** | empty | Leave it empty and the device's own location service is asked. Type a place and nobody is asked: a town (`Springfield`, or with what tells it apart: `Springfield, Illinois`, `Paris, France`; of several towns of one name the largest is taken), or latitude and longitude (`39.8017, -89.6437`). What the app took it as is shown at once. The typed place is one of this device's settings, kept like them; it is told to your other devices only as the device's location, and only while the switch is on. |
| **This device is** (information) | — | Where the device is now, as its Face tracking tab shows it, or why the app can't say (the system doesn't allow it, its location service is switched off, the typed place is not a place). |

## Cameras tab

Every device's live camera in a grid (devices with **Stream this camera to my other devices** on), each with its
readout, and one list of events from every camera. Over each picture's lower edge: where that device is, for a
device whose **Show and keep this device's location** is on (what the device itself says while this tab follows
it, else the newest row of its location history, marked "last known"). Its two choices aren't in the Settings tab; the tab remembers them
in the window's storage, on this computer only.

| Choice | Default | What it is for |
|---|---|---|
| **Show** (a tick per device, in the tab's header) | every camera | Untick a device to leave its camera out of the grid. |
| **Events** (Hide events) | shown | The events list is a fly-out on the side; closed, the cameras take the whole width and the Events button counts what came since. |
| **Focus** (on each camera's picture; or double-click the picture) | all cameras | That camera alone, laid out as that device's own Face tracking tab: its picture as large as the window allows, its live readout beside it with the three counts, its Signals chart in a drawer under the picture. **All cameras** in the header (or Esc) brings the grid back. The other cameras go on being fetched meanwhile. Not remembered: the tab opens on all cameras. |
| **Signals** (under each camera) | closed | That device's Signals chart, as its own Face tracking tab shows it: every reading it sends with its readout, each with its own time, so the chart is as fast as that device checks its picture. Kept in this page's memory only, from the moment the Cameras tab is opened. A camera whose chart is open takes the whole row, and the grid scrolls. |

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
| **Share Describe all with other computers** | off | **Describe all** also sends photos to your other devices' LM Studio servers that have the same vision model. Your other devices are the computers in your shared database, at the addresses they gave there: no other computer on the network is sent a photo, and without a shared database nothing is shared (the same model, so every description comes from one model): each takes the next photo as it finishes, one at a time, so a faster computer does more. **Each photo sent there is seen by that computer.** One that fails twice in a row is left out, its photo goes back to the others, and it is tried again every five minutes; this computer's server keeps going to the end. A single photo is described by the server above only. Measured: 40 photos in 136 s on one PC, 93 s with a second computer (RTX 4060 laptop on Wi-Fi) helping. |
| **Shared with** | (shown, when on) | The computers Describe all would share with now, and their addresses. |
| **Describe everything again** (button) | | Describes the whole library anew with the describer set above, for example after choosing a better model: every photo, and every video frame a face was found in, on all your devices. With a shared database, what is only on another device is shown by that device (as in the photo view), and the new descriptions reach every device. Asks first, saying how many, with which model and which computers. Each description is replaced when its new one arrives, so stopping loses nothing. The work is shared as Describe all's is: with **Share Describe all with other computers** on, among the LM Studio servers that have the same model; with **Fully distributed system** on, also those beyond the local network. Progress and **Stop** are at the top of the Labeling tab. |
| **Unfinished** (Carry on) | (shown, when one is) | A Describe everything again that was stopped, or that left some out (unreadable, or their device didn't answer): describes what is left, with the describer set now. When it was started and with which model is kept in the private folder (`describe.json`) until nothing is left. |
| **Running now**, **Stop describing** | (shown, while describing) | How far Describe all or Describe everything again is and which computer did how many; Stop keeps what is described so far. |
| **Built-in model** (Download) | | Florence-2 large (0.8 GB from Hugging Face, checked by SHA-256): runs inside the app on the processor, so photos stay on this computer; plainer descriptions than LM Studio's. |

---

## Music

Music as one more signal about a person: which song a music app on this computer shows as playing (Spotify), how
it sounds on each of this computer's audio interfaces, and the person's usual music. The numbers describe the
sound (how loud, tempo, how bright or bassy, major or minor), never what anyone feels. The sound itself is never
kept. Nothing is listened to or logged until the switch is on, a person is chosen, and a shared database is set up.

| Setting | Default | What it is for |
|---|---|---|
| **Music** | (shown) | What plays now, whether songs are being logged and for whom, every audio interface that can be listened to (each output: what the computer itself plays; each microphone: what sounds in the room) with what it heard at the last listening, in a word (quiet, music, speech, steady background noise and its kind, activity...), and the chosen person's usual music. |
| **Keep a log of the music that plays** | off | Each song Spotify shows as playing here is kept in the shared database when it ends: artist, song, when, for how long, what each audio interface heard meanwhile (in a word, with the background's numbers, and a song's numbers where music sounded), and how the song sits against the person's usual music. Music that an interface hears with no player naming a song is kept as "music in the room". A song seen for less than 15 seconds isn't kept. While on, the interfaces chosen under Listen on are listened to for 8 seconds every half minute, song or no song. |
| **This device's music belongs to** | nobody | The named person whose music plays on this computer. Without one, nothing is logged. |
| (on a Mac) | | An iPhone or iPad that macOS offers as a microphone is never listened to, by the music log, the Signals chart's sound or Listen to voice: opening it would take the phone over and stop its own app. A phone listens for itself. |
| **Listen on** | every audio interface | While a song plays, 8 seconds every half minute: every interface (outputs and microphones), outputs only (what this computer itself plays), or none (song names only). A microphone hears the room, so it also hears a song playing on another device; the operating system may show the microphone in use. A Mac can't hear its own outputs: microphones only. |
| **Spotify** (Connect) | not connected | Your own Spotify key's Client ID (made at developer.spotify.com, with the redirect address the row shows). Connect opens Spotify's page in your browser, where you sign in and agree to read-only access to your library. The tokens are kept protected in the private folder. |
| **Usual music** (Read my liked songs) | | Reads the connected account's liked songs and makes the chosen person's usual music: artists, genres, decades, how well known the songs are. Kept with the person's other normals; shared with your other devices through the shared database. |
| **Stop using Spotify** (Disconnect) | | Forgets Spotify's tokens on this computer. The usual music already made stays. |

---

## LM Studio

LM Studio on this computer and on the local network: the servers Describe photo can use, and the vision models
kept the same on every computer.

| Setting | What it is for |
|---|---|
| **LM Studio on this computer** | Whether LM Studio's server is running, and on which port. While the app runs it keeps LM Studio serving on all network interfaces and announces it on the network with its port, unless the computer already does. Lists the vision models the other LM Studio servers (or the shared folder) have and this one hasn't, and where each would come from. |
| **Get the same models** (button) | Gets those models: copied from the **Shared models folder** when they're there (the same files, each checked by SHA-256); if that copy fails in any way or the model isn't there, downloaded from the internet with LM Studio (the same model and quantization). A bar at the top of the Labeling tab shows which, how far, and how long is left. |
| **Shared models folder** | A folder every computer can reach, for example on a NAS: `\\NAS\share\lmstudio-models` on Windows, `/mnt/nas/share/lmstudio-models` on Linux. Each computer with the app and LM Studio puts its vision models there (with a list of every file's size and SHA-256) and gets the ones it lacks from there first. Empty: not used. This computer's own setting (`lmstudio.json`). |
| **Fully distributed system** | off | For computers that aren't all on one local network (some reached over a VPN, or over the internet). On: the search for LM Studio servers covers every network adapter of this computer, VPN tunnels and virtual networks included; this computer tells your other devices those addresses too (for their photos and cameras); and Describe all, with Share Describe all on, may send photos to LM Studio servers beyond your local network. Off: only the real adapters of your local network (Ethernet, Wi-Fi) are searched and told. This computer's own setting. |
| **LM Studio on the network** | Every LM Studio server on the local network: each computer's name, its addresses (every network port) and port, the models it serves, which read pictures and which are loaded. Servers that announce themselves give their own port; the rest are found by trying LM Studio's port on each address of this computer's networks, only while Settings is open. With **Share Describe all with other computers** on, the same search also runs when a photo is opened in the labeling tool and during Describe all. |
| **AI model apps found** | Apps that can run a vision model for Describe photo: LM Studio, Ollama, Jan, llama.cpp, vLLM and others on their usual ports here, the server set under Describe photo, apps installed but not running (for LM Studio, its downloaded models), and the address other computers can use when the server listens on all interfaces. |

---

## This computer

What the app found on this computer and what it does with it. See the user guide's
[Your hardware: what the app uses](../face-recognizer/README.md#your-hardware-what-the-app-uses).

| Setting | What it is for |
|---|---|
| **Hardware** | The processor (and threads), memory, graphics processors, NPU, and the ways ONNX Runtime can run models here (DirectML, CUDA, CoreML, CPU). |
| **Where the work runs** | Which processor runs each job (face finding, recognition, photo quality, expression, face-shape cues, body posture), with the start-up check behind each choice: a model runs on the graphics processor only if it gives the processor's answers (within 1%), faster. On Windows, DirectML runs recognition, photo quality and expression as measured there, without that check at each start; face finding is checked on every system. Also how many photos an import works on at once. To choose yourself, set `FACE_DEVICE`. |

---

## Shared database

Optional: keeps every device's faces, names, labels and private data in one PostgreSQL database (with pgvector) that
all your devices with Face Recognizer share, and each device's own settings there too. Without it, everything stays
in each device's own files, as before. See the user guide's
[Sharing between your devices](../face-recognizer/README.md#sharing-between-your-devices-shared-database).

| Setting | Default | What it is for |
|---|---|---|
| **Shared database** (status) | Not set up | The server and database, whether it's reachable, when it last checked, how many changes made here wait to be sent (while the server can't be reached), and this device's name. |
| **Connect** (server, port, database, user, password) | empty, 5432, `facerec` | A new server is prepared once: the app's role and database, and the server's certificate authority in its `server_ca` table. Logs in over an encrypted connection, takes the server's own certificate authority from its `server_ca` table and from then on checks the server with it on every connection. The first time, this device's data is merged in and every other device's is taken (Joining). Leave the server empty and Connect to stop using it: this device keeps its copy. Kept in `%APPDATA%\FaceLabels\database.json`; the password is protected by Windows for your user (DPAPI; on macOS and Linux the file is readable by you only). |
| **Join a new device** (on a device without a shared database: **Join a shared database**; Face tracking → Controls) | — | Lets a device into your shared database without anything being typed: one device shows a join code (a QR code) and the other scans it with its camera. Either way round. A device of the database shows a code and the new device scans it (a phone's own camera app reads it too); or the new device shows one and a device of the database scans it, names the new device and asks you before anything is handed over, which is how a phone lets in a computer you are sitting at. The new device gets the database's login and, when the device that lets it in has it, the passphrase for keys: it reads everything your other devices read, and can let further devices in the same way. A code works once, for two minutes, and only between two devices on the same local network; it holds no login. A device still on version 2.2 can let a new one in too (the login alone). A device that joined is in at once, and each of your computers says so in its alert inbox within half a minute: **A new device joined your shared database**, with the device's name, its system and its app's version. |
| **Share my photos with my other devices** | on (with a shared database) | Runs this device's peer service: lets your other devices show the original photos and videos this device has, fetched when opened and never kept there, and carries this camera's live stream (**Stream this camera**, under Face tracking: recognition). They ask this device over your local network on port 8767 (the app's ports + 3; `FACE_PEER_PORT`), encrypted (TLS, this device's own certificate, whose fingerprint is in the database and checked by the asker), and only your devices can ask (requests signed with a key made from the database password). Photos by their content only, nothing else. Also lists the devices sharing now and their addresses. Kept in `peer.json`; `FACE_PEER=off` turns it off whatever this says. |
| **Name a device** (a device to choose, its name) | — | What you call a device. Every device of the database then shows it under that name: in the Cameras tab, in alerts and in these lists. A phone calls itself only "iPhone", and a name tells two of them apart. A name is one device's alone, as an email address is one account's: one that another device goes by already is not taken, whatever its capitals. A device that joins calling itself what another goes by already is given a number by itself ("iPhone 2"), and its own **Join a new device** page asks whoever is at it for a name; that page shows and changes a device's own name at any time, which is how a phone is named from the phone. Saving an empty name takes the name away again. The name is only ever shown: it has nothing to do with how the devices find each other on the network. |
| **Location history of** (a device to choose), **Where it was** | — | The chosen device's location history, the newest row first (up to 200 rows): when, the town, the coordinates, how sure. Only devices whose **Show and keep this device's location** is or was on have rows. |
| **Delete a device's location history** (a device to choose) | — | Deletes every row of that device's location history in the shared database, from any device. It can't be undone. The device goes on writing rows while its own switch is on. |
| **Remove a device, on this device only** | — | For a device of your database that this device should have nothing to do with. This device then shows nothing of it (its camera in the Cameras tab, its photos), asks it for nothing (a photo only it has, a description from its LM Studio), and answers none of its requests for this device's camera, readout and photos. It is this device's own list: nothing changes in the shared database, no password or key is changed, and your other devices go on with that device as before unless you remove it there too. The device is removed together with the certificate it has at that time, so the same machine is refused here under another id as well; adding it back forgets the certificate too. What it does not do: the removed device stays in the database and still reads what every device shares through it (names, labels, descriptions). |
| **Add a removed device back** (shown while a device is removed here) | — | Names the devices removed on this device. Choosing one makes it a device like the others again, here. |
| **Copy settings from** | — | Takes another device's options for the Face tracking and Labeling pages and which describer and model it uses, in place of this device's. Not the settings that say where a device sends photos or fetches files (the describe server's address, Share Describe all, the shared models folder, Fully distributed system): those are set on each device. Copy settings is the only way a device's settings come from the database: a device otherwise keeps its own, and puts them back if its rows there are changed by anyone else. |
| **Passphrase for keys** (typed twice, at least 10 characters) | not set | Until it's set, each device's keys stay on it alone. The Web search keys and the describe server's API key are kept in the database encrypted with it (AES-256-GCM, key from the passphrase by scrypt), on the device before they're sent. Set it once, on any device. Another device gets it only from you: type it there too (the button reads **Enter**), or use **Get the passphrase from another device**. The database password alone gets nobody the passphrase or a key. Typing a new one (twice) on a device that has it changes it: every key is encrypted again, and your devices that have the current one take the new one by themselves (a device that missed two changes needs it typed, or a code, again). Each device keeps it (`secret.key`, protected like the password), so the field shows it masked with the eye button. |
| **Get the passphrase from another device** (shown on a device that doesn't have it) | — | **Show a code** shows a one-time code on this device (8 letters and digits, for 15 minutes; **This device's code**). Type it on a device that has the passphrase and this device is given it, sealed so only it can open it. The code is how each device knows the other is yours; it is never sent to the database. |
| **A device is asking for the passphrase** (shown on a device that has it, while another asks) | — | Names the device asking. Type the code that device shows and it is given the passphrase. If you didn't just ask on a device yourself, do nothing: without the code nobody is given anything. |
| **Passphrase on each device** | — | Each device's fingerprint of the passphrase it has, and whether it's the current one (one-way: a fingerprint can't be turned back into it); a device counts as current when it also shows that it holds the passphrase, which nobody without it can do. Also each device's database password fingerprint. |
| **Restore an earlier passphrase** | — | Lists the passphrases this device has had itself (who set each, when). Choosing one makes it current again: every key is encrypted again with it and your devices that have the current one take it. For undoing a wrong change. |

What is shared, and what stays each device's own:

| Shared by every device | Each device's own (in the database under its id) | Only ever sent, never taken by other devices |
|---|---|---|
| faces (prints, pictures, boxes), labels, photos (by content), photo tags and dates, Web search answers, descriptions, Collect faces pictures, alerts (also "moved between cameras"), presence visits, expression readings (sent only, see Keep expression readings), each person's usual look and signal normals, fitted expression numbers, keys (encrypted; the passphrase sealed to each device you let in), forgotten people, Web search use per month | page options, describer choices, LM Studio shared folder, where each photo is on it (`photo_paths`), measured hardware and model numbers; the device list (name, app version, addresses and certificate fingerprint for the peer service, last seen, whether its camera streams) and who on its alert list its camera saw in the last 20 minutes (names and times, for "moved between cameras", only while that alert is on) | the location history of each device whose location is switched on (kept 90 days), the app log (kept 90 days), the undo history log, posed expression tests |

Live camera pictures and readouts never go to the database: they go from device to device over the peer service, and
stay in memory.

---

## Environment variables

Set before starting the app (or a program) to change how it starts.

| Variable | Values | What it does |
|---|---|---|
| `FACE_DEVICE` | `auto` (default), `gpu`, `cpu` | Where the models run. auto: the graphics processor for each model that passes its start-up check, else the processor (DirectML on Windows: without the check, except for face finding). gpu: the graphics processor without the check. |
| `FACE_IMPORT_WORKERS` | a number | How many photos an import works on at once (default: processor threads − 2, 2 to 12). |
| `FACE_CUDA` | `off` (or `0`, `no`) | Don't use or offer the downloaded CUDA (Linux app). |
| `FACE_PARTS` | `page`, `one`, `gpu` | Where the face, body and finger readings of the Face tracking page run. Without it: beside the page, a worker for each of the three models on a computer and one worker for all three on a phone or a Raspberry Pi. `page` keeps them in the page itself (as before 2.5.2), `one` gives a computer the phone's one worker: both to compare the ways on a device. `gpu` tries the graphics processor for them, which is faster and gives other readings than the ones the app was measured with. |
| `FACE_STREAM` | `transfer`, `page` | How this camera's frames for the other devices leave the Face tracking page. Without it: made and sent beside the page (`transfer`), except on an iPhone, where the page makes and sends them itself (`page`). Either names the way, to compare the two on a device. |
| `FACE_SIGNALS` | `all` | For a test device: its Face tracking page switches every option that makes signal readings on and keeps it on (expression, calibration, evidence, streaming, voice, pulse, body posture, the chart's sound), so the device sends every signal it has. Not for everyday use: the microphone is listened to. |
| `FACE_LOCATION` | `on` | For a test device: its Face tracking page switches **Show and keep this device's location** on when the app starts, as if by hand (so it stays on afterwards, until it is switched off there). The system still asks for its own permission. Without it the location is on only where its owner switched it on. |
| `FACE_LMSTUDIO` | `off` (or `0`, `no`) | Don't keep LM Studio on all interfaces, announce it, or put its models in the shared models folder. |
| `FACE_LLM_URL` | an address | The default Describe photo server (before one is set in Settings). |
| `GOOGLE_VISION_API_KEY` | a key | Google key for Web search, instead of saving one. |
| `FACE_PORTS` | a port n | The app window's ports: n to n+2 instead of 8764 to 8766 (to run a second copy). The peer service then uses n+3, Spotify's one-time answer to **Connect** n+4 (8768 by default: the redirect address of your Spotify key must name that port), and the Cameras tab's page n+5 (8769 by default; on 127.0.0.1 like the others). |
| `FACE_PRINTS` | `frame`, `behind` | Whether a face's print is made behind the frames (a face is found in every frame, and named a little later) or with each frame. Without it a phone and a Raspberry Pi make them behind the frames and every other computer with each frame. `frame` turns it off there, `behind` lets another computer try it: both for comparing. |
| `FACE_TRACKER` | `inline` | Keep the live tracker in the app's own process, as before 1.25, instead of a process of its own (then heavy work in the labeling tool can delay the camera's server again). For comparing the two. |
| `FACE_UI` | `browser`, `none` | browser: open the app in the web browser instead of its window (same as `--browser`). none: start the servers only, with no window and no browser (for test copies; open `http://127.0.0.1:8764/` yourself). |
| `FACE_ROOT` | a folder | Another folder with `models` and `data`. |
| `FACE_HOME` | a folder | Another folder instead of the private `FaceLabels` folder. |
| `FACE_DB` | a PostgreSQL connection string (`host=... dbname=... user=... password=...`) | Use this shared database instead of the one set in Settings (tests and test copies). |
| `FACE_DB_SCHEMA` | a schema name | The schema in it (default `facerec`). |
| `FACE_PEER` | `off` | Don't share this device's photos or stream its camera to the other devices (no peer service), whatever Settings says. |
| `FACE_PEER_PORT` | a port | The peer service's port instead of the app's ports + 3 (8767, or `FACE_PORTS` + 3). |
| `XDG_DATA_HOME` | a folder | Linux: the private folder is `FaceLabels` inside it (default `~/.local/share`), unless `FACE_HOME` is set. |
| `GLIBC_TUNABLES` | the system's own, e.g. `glibc.malloc.arena_max=128` | Linux: the programs set how the C library hands out memory themselves (few arenas, large blocks back to the system when freed: `paths.give_memory_back`), so the app's memory stays level. When this variable already sets a `glibc.malloc.` value, they leave all of it as set (for measuring). |
| `FACE_RESTART_AFTER` | a process id | Set by the app itself when it starts again on Windows (the new copy waits for that process to end); not for use by hand. |
| `FACE_SIGN_ACCOUNT`, `FACE_SIGN_PROFILE`, `FACE_SIGN_ENDPOINT` | an Azure Artifact Signing account, its certificate profile, its region's address (`https://<region>.codesigning.azure.net`) | `packaging/windows/sign.ps1` only: code-sign the .exe, the installer and its uninstaller with that profile, as whoever is signed in to Azure (`az login`; CI signs in as the repository's workflow). All three or none. |
| `FACE_SIGN_CERT` | a certificate's SHA-1 thumbprint | `packaging/windows/sign.ps1` only: sign with that code signing certificate from this user's Windows certificate store instead. |
| `FACE_TEST_DB_HOME` | a private folder with a `database.json` | Tests only: the login for a real server that the tests of the shared database and of the sync run against, each in a schema named `t_...` that it makes and drops; without it they're skipped. |

---

## What reaches the network

Nothing is sent anywhere by itself except as listed here.

| Setting or action | What is sent, and where |
|---|---|
| First start | Downloads the face models from their publishers or the project's `models-v2` release (checked by SHA-256). |
| **Check for updates** | Asks GitHub for the latest release, when clicked. |
| **Web search** | One face to Amazon and its photo to Google, when clicked, with your keys. |
| **Describe photo** on another computer's server | Each photo described goes to that server. |
| **Share Describe all with other computers** | During Describe all and Describe everything again, photos go to the LM Studio servers of your other devices (the computers in your shared database, at the addresses they gave there) that have the same model. Never to another computer on the network, and nowhere without a shared database. |
| **Describe everything again** | With a shared database: each photo or video frame only another of your devices has is asked of that device (the peer service: local network, TLS, signed) and described like the rest, so with a server on another computer or Share Describe all on it goes there too. Nothing more than Describe all sends otherwise. |
| **Built-in model**, **Get the same models** (from the internet) | Downloads, when clicked: Hugging Face, LM Studio's sources. |
| **Music → Spotify** (Connect), **Read my liked songs** | When clicked: Spotify's own sign-in page opens in your browser; then the app asks Spotify (accounts.spotify.com, api.spotify.com, HTTPS) for the connected account's liked songs and their artists' genres. Nothing is sent to Spotify but that question and your key. The music log itself asks nothing of the internet: the song's name comes from the Spotify app's window, the numbers from this computer's own audio interfaces. |
| **LM Studio → LM Studio on the network** | While Settings is open (and, with Share Describe all on, when a photo is opened and during Describe all): a connection attempt to LM Studio's port on each address of this computer's local networks, each server's model list, each computer's name (mDNS or NetBIOS), and an mDNS question for announced servers. Local network only, on this computer's real adapters; with **Fully distributed system** on, every adapter's network (VPNs and virtual networks too), and servers found beyond the local network may be sent photos by Describe all and Describe everything again. |
| **LM Studio → AI model apps found** | While Settings is open: asks the usual AI ports on this computer, and the Describe photo server set (which may be another device, with its own API key) for its models. |
| **LM Studio on this computer** | While the app runs (every minute): keeps this computer's LM Studio server listening on all network interfaces (restarting it with `--bind 0.0.0.0` if it listened on this computer only; CORS stays as LM Studio's own **Enable CORS** setting has it: on means a web page open in a browser may call the server too, and the app never switches it on), so other computers on the network can use it, and announces it on the local network (mDNS). `FACE_LMSTUDIO=off` stops both. |
| **Shared models folder** | While the app runs: puts this computer's vision models in that folder (your own NAS or share); **Get the same models** copies from it. |
| **Share my photos with my other devices** | While on (with a shared database): the peer service listens on the local network (port 8767), TLS. It answers your other devices' requests for its original photos (by content hash) and for this camera's stream. Opening a photo another device has asks that device for it. |
| The event bus (no switch of its own: it follows **Share my photos with my other devices**, which runs the peer service) | While this device shares: what it publishes of itself goes to each of your other devices whose page asks for it, over the peer service (your local network, TLS, each side's certificate checked against the member list it keeps, the request signed with the cluster's key). That is one document: what the device is (its system, the app's version, its processor and memory), every section and signal the Signals chart has there, what is switched on and off, and every setting of its Face tracking tab with its value (never a key, a password or a token). Who is in view and the readings of the last seconds go out with it only while **Stream this camera to my other devices** is on, and only to the devices listening at that moment: they are not kept for one that asks later. Nothing of it is stored on either side, and nothing goes to a device that is not a member or that you removed on this one. A member is asked only at an address of a local network, unless **Fully distributed system** is on. |
| The people a device without a library names (no switch of its own: it follows **Share my photos with my other devices** on the computer that gives them) | A device that keeps no library of its own (a phone) asks your computers for the people it can name: their names with their face prints, as that computer's own library has them. They go from the computer to that device directly, over the peer service (your local network, TLS, each side's certificate checked against the member list it keeps, the request signed with the cluster's key), and are held there in memory only. The shared database is asked for them only when no computer gave them. |
| **Stream this camera to my other devices** | While on and the camera runs: this device's camera, as its tracker shows it, and its live readout go to your other devices whose Cameras tab shows it, over the peer service. With the readout go the readings of the last seconds as numbers (for their Signals chart): each person in view by name with every signal's number and their usual range, how fast the camera is checked, the device's own readings (processor and memory in use, warmth, battery) with what it is in words (its system, the app's version, its processor and memory, what its camera gives), and, while **Chart this device's sound** is on, each audio interface's name, numbers and one word. |
| **Cameras** tab | While it watches: this device's app connects to each streaming device's peer service (TLS, signed requests) for its frames and readout. |
| **Join a new device**, **Join a shared database** | While a join code is shown (two minutes at most): the device that shows it listens on the local network (port 8770, or a free one). The device that scanned the code connects there, each shows the other that it holds the code's key, and the database's login and the passphrase for keys cross once, encrypted for that one exchange. A code only ever sends a device to an address of a local network. |
| **Show and keep this device's location** | While on: the app itself asks nothing on the internet. With no place typed, it asks the device's own location service, and that service (the system's, not the app's) works a computer's place out from the networks around it with its maker's servers: Microsoft, Apple or Google. A typed place asks nobody. The location (numbers and one word) goes to your other devices with what this device publishes of itself (the peer service: local network, TLS, signed, members only), and the history's rows go to your shared database on this device's own connection to it. |
| **Shared database** | Once connected: every face, name, label, photo record, description, Collect faces picture, alert, presence visit, expression reading (while Keep expression readings is on), song played (while Keep a log of the music that plays is on), per-person signal (a person's usual music among them) and this device's settings go to that PostgreSQL server, and come from it, over TLS checked against the server's own certificate authority. Also who on this device's alert list its camera sees (names and times, kept 20 minutes, only while "They move between cameras" is on) and this device's entry in the device list. Keys go only encrypted, and only once the passphrase is set. |
