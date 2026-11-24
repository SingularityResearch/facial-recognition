# Face Recognizer — guide for coding assistants and contributors

This is the one guide for anyone changing the code (Claude Code reads it through `CLAUDE.md`; Codex and others
read it directly). Keep it current when the layout, conventions or checks change.

Local face recognition for any camera stream: learn specific people from a photo library, name or ignore faces in
a labeling tool, and show names with match confidence (and optional expression) on faces live. Everything runs on
the computer itself; no data leaves it, except a face or photo the owner sends on purpose with the labeling
tool's **Web search**, photos sent to be described when the owner points **Describe photo** at a server on
another device, and, once the owner sets one up, everything private going to their own **shared database**
(PostgreSQL + pgvector over TLS), through which several devices share names, labels and private data and show
each other original photos and live cameras over the local network.

The project is written in Rust. It is a commercial, proprietary product whose source is public (`LICENSE`:
the last section of this guide says what that asks of anyone who writes here).

Names used below: `ROOT` is the folder with `models/` and `data/` (`paths::root`, `paths::app_root`); `HOME` is
the private per-user folder (`paths::home`: `%APPDATA%\FaceLabels` on Windows, each system's application-data
folder elsewhere). `data/` holds the library; `HOME` holds keys, settings and everything private that isn't the
photo library.

## Layout

- `Cargo.toml`, `Cargo.lock`, `rustfmt.toml`, `.cargo/` — the workspace (edition 2024). On Windows the programs
  are built with the C runtime linked in (`.cargo/config.toml`).
- `crates/` — the libraries and the two command-line programs:
  - `face-core` (the recognition core: pictures, detection, prints, the model runtime), `face-files` (folders,
    locks, JSON as the files have always been written), `face-home` (the private folder's stores), `face-library`
    (the store, the labeling engine, what photos say about themselves), `face-import` (photos and videos decoded,
    the importer), `face-db` (the shared database), `face-sync` (the sync with it), `face-tools` (`face-tools get`
    and `have`: every pinned download the programs need; the first-run list), `face-server` (the three local
    servers: the tab page, the labeling tool, the tracker; everything their pages ask for), `face-app` (the
    tracker as a library for the phones' apps).
  - `crates/face-server/pages/` — the pages that are not in `web/` (the first-run question, joining by a
    code, the MediaPipe module and its worker), with their Node test.
- `apps/` — `desktop` (the program `FaceRecognizer`: the app with its window, and as `tracker-child` the
  tracker's own process), `android` (the Java shell, `build.ps1`: the package without Gradle, for trying or
  with `-Release` as it is published, and with `-Release -Bundle` also the app bundle Google Play takes: the
  app as the module `base` and the models as an install-time asset pack, which the app reads as the assets it
  always had; the libraries stay extracted at install, since the model runtime is opened by its path; its tools
  are named by version, never "the newest"; `make_key.ps1`: its release key, once), `ios` (the Swift shell; `runtime.sh` fetches the model runtime it is
  linked with, pinned; `project.sh` writes the Xcode project with the
  app's permanent id and the workspace's version, `build.sh` the build for a connected iPhone, `store.sh` the
  one the App Store takes, `carry.sh` what both carry: the page files and the models; `PrivacyInfo.xcprivacy`
  is Apple's privacy manifest: read the built program's system calls again when a library is added). The window is the system's web view on Windows (`main.rs`) and
  macOS (`mac.rs`), and on Linux and the Raspberry Pi a Chromium window of the app's own (`linux.rs`: the
  owner's word, 2026-11-21, that Linux must have a window too; WebKitGTK reads a camera poorly). Its rules:
  a profile that holds nothing but this window (per private folder, so a test copy never shares the app's),
  and no key in the desktop's keyring (`--password-store=basic`: where a desktop signs in by itself, a
  Raspberry Pi's, the keyring is locked and the browser showed an empty window behind the desktop's question);
  the camera and microphone allowed in that profile for the app's own two addresses and nothing else,
  written anew at every start; the Windows window's keep-running switches; the browser is ended before the
  app starts itself again in place, and a browser an earlier copy left running on the app's profile is ended
  before a new one is started (`linux.rs` `end_leftover`: a second browser on one profile only hands its
  page to the first and ends at once, which read as "no window"); a browser that ends before its page ever ran counts as no window, and
  the app then opens the default browser; `FACE_UI=hidden` runs it headless.
- `web/` — the pages the servers send (the tab page, the labeling page, the tracker and Cameras pages, what they
  share), carried inside the programs (`crates/face-server` `pagefiles.rs`), and `web/tests/` (Node).
- `migrations/` — the shared database's `NNN_*.sql`, carried inside `face-db`.
- `assets/icons/` — the app's icon in every form the systems want (one drawing; the
  phones' icons in `apps/android/res` and `apps/ios/Assets.xcassets` are the same drawing, and Android's is in
  two layers, which a phone cuts to its own shape). `assets/store/` — the same drawing as the stores' pictures
  (Google Play's icon and feature graphic, the Microsoft Store's box art and poster) and the listings'
  screenshots, whose people are made up: generated faces with invented names, never a real person's picture.
- `packaging/` — `windows/` (Inno Setup: `FaceRecognizer.iss`, `build.ps1`, `sign.ps1`), `macos/` (`bundle.sh`,
  `build.sh`, `sign_and_notarize.sh`, `entitlements.plist`, `Info.plist`), `linux/` (`install.sh`, `build.sh`: also
  the Raspberry Pi's), `decoders/` (the project's own small FFmpeg).
- Rust only: no script in another language in the tree, the pages' JavaScript and the build scripts apart. The checks that need more than `cargo test` (a
  desktop, a camera's page, an installer run for real, the shared database) are not all Rust tests yet: those
  that aren't are kept outside this repository. New checks are written in Rust.
- `docs/` — `SETTINGS.md` (every setting, environment variable and what reaches the network),
  `WHY_LAYERED_EXPRESSION.md`, `REFERENCES.md` (the research articles the work builds on, each with what the
  project takes from it: a citation goes in only when it was checked against its source, and a model the app
  does not have is not named there), and `store/` (one file a store: its listing's text and the answers to its
  forms, so that every release says the same).
- `.github/workflows/` — `ci.yml` (tests on four kinds of computer; the installers; signing on `main`; what cargo
  compiled of the libraries is kept between runs, by system, compiler and `Cargo.lock`, with GitHub's own cache
  action: the project's own code is compiled from the checkout every time, and only pushes to `main` write it),
  `decoders.yml` (FFmpeg and the HEIC libraries built for each).

On the owner's computers a folder `face-recognizer/` may lie inside this checkout, untracked and ignored: it
holds an installed app, its models and a library. Never delete it or anything in it.

## Building and checking

```
cargo build --release                       # everything; the app is target/release/FaceRecognizer
cargo run -p face-tools --release -- get --root target/root     # models, model runtime, MediaPipe, FFmpeg, HEIC
cargo fmt --check && cargo test --workspace --release           # before every commit (no models needed)
node --test web/tests/*.test.js crates/face-server/pages/tests/*.test.js
```

The installers: `packaging/windows/build.ps1`, `packaging/macos/build.sh`, `packaging/linux/build.sh`. CI runs the tests and builds the installers on every push to `main`.

Test UI changes in the real app or a test copy, never against the owner's running app:
- A test copy gets its own `FACE_ROOT` and `FACE_HOME` in a temporary folder and its own ports (`FACE_PORTS`), and
  is started with `FACE_UI=none` (servers only) or `FACE_UI=hidden` (the window made, never shown). On macOS and
  Linux the app's own folder for models and data is the user's real one (`paths::app_root`): a test copy of
  `FaceRecognizer` always names both folders.
- Nothing a test copy starts may open a page in the owner's browser: a tab left there keeps running and takes
  the camera. Read every path to `desktop::open_url` before starting new window or page code on a device.
- Every check that starts the app sets `FACE_LMSTUDIO=off`: its upkeep would act on the owner's own LM Studio.
- A check or a test never asks a real outside service (Amazon, Google, Spotify, Hugging Face, the model
  publishers, Microsoft's installer): each has a stand-in address taken only as `http://127.0.0.1:<port>`.

## The parts and their rules

Many rules below say that something is fixed: the bytes of a file, a number, what a request answers. They are
fixed because libraries and devices in use depend on them. Change one only with a test that holds what it must
still give, and say in the test why.

The app (`FaceRecognizer`) runs the sync by default (`--no-sync` turns it off; it does nothing until a shared
database is set up); `face-server label` syncs only with `--sync`. Never run two programs that sync on one
device's library at the same time: each would send what the other took.

- **The library** (`crates/face-library`: the store, the labeling engine, what photos say about themselves;
  `crates/face-files`: folders, locks, JSON). The library's files (`data/labels.json`, `faces.npz`, the photo
  facts) are read by every version and every device: their bytes, their key order and the lock each is written
  under are fixed (`filejson.rs`, `paths.rs` `SharedLock`). In the engine a print changes only through the store's
  functions (they keep the log the kept scores and stacks read), and two prints are compared only by
  `scores::dot`: a second way to the same number is how kept scores go stale. What a change does to a label, to
  the lists and to the files is `store.rs` and `labels.rs`; a photo's key, its tags and its file are
  `photofacts.rs`, `photofiles.rs` and `exif.rs`.
- **The private folder's stores** (`crates/face-home`: the alert inbox, the presence log, settings and person
  signals, the expression log, descriptions), used by the servers and by the sync, each under its own lock.
- **The sync with the shared database** (`crates/face-sync`, `crates/face-db`):
  - the rows' digests and `HOME/sync.sqlite` (`digest.rs`, `local.rs`): a digest that changes makes every
    device send its library again;
  - sending and taking rows (`server.rs`, `apply.rs`) and the steps (`sync.rs`: joining, sending, taking other
    devices' changes by their transactions): tried on a real server only in schemas named `t_...` that a test
    makes and drops, never the shared one;
  - the private folder's tables are `home.rs`: a device's settings never come from a row, a row is checked
    before it is taken, and a new table there needs its place in every function of `home.rs`;
  - what is only ever sent (the waiting logs, the app's log, the posed tests) is `sent.rs`: the files' names,
    their lines and the sync state's keys stay as they are, since rows wait on the devices;
  - the passphrase for keys and the encrypted keys are `keys.rs`, fixed byte for byte: never add a path that
    seals the passphrase to a device on the strength of a row;
  - the sync holds the library only while this device's own data is read or written (`sync::Library`), so that
    no page waits for the network (`crates/face-server` `syncer.rs`).
- **The labeling tool's server** (`crates/face-server` `label.rs`):
  - what the Settings tab asks about the shared database is `label_settings.rs`; `/api/secrets` answers only
    with the page's own header, and nothing there is answered under the labeling lock;
  - one person's export and forget is `persondata.rs` (with `face-library` `person.rs`): a new place that
    keeps anything about a person goes into the list at its top, its `export` and `forget`, and its test;
  - Describe photo is `describe.rs` (a describe server asked) and `describejob.rs` (Describe all and Describe
    everything again, in the importer's one slot), with the describer's settings and the kept descriptions in
    `crates/face-home`: a redirect is never followed, an answer is read only up to a size, the API key goes to
    its own server only, and a helper is only a computer of the shared database at an address it gave there,
    or this computer (`describe::helpers`). Its tests ask stand-in servers on this computer, never LM Studio
    or the internet;
  - the built-in describe model is `florence.rs` (the pinned files, `good`, the tokenizer, the generation); the
    model's picture is made by `Bgr::resize_cubic_float` (the 8-bit model writes other words for one level in a
    few bytes: change it only with numbers that show the words stay). The programs take another address for
    its files only from `FACE_TEST_HUGGINGFACE` and only `http://127.0.0.1:<port>`: never widen that, never ask
    for the download in a test without such a stand-in (a unit test did, 2026-11-21, and fetched 260 MB), and
    keep `florence::publisher` a place where nothing listens in the crate's own tests;
  - Web search is `websearch.rs` (what is sent where, what is kept of an answer, the caps, the count before a
    search is sent). A test never asks the real services (a face and a photo would leave the computer, and be
    billed): another address for a service comes only from `FACE_TEST_VISION` / `FACE_TEST_REKOGNITION` and
    only as `http://127.0.0.1:<port>`: never widen that. Amazon's request is signed by the program itself
    (`aws_authorization`, held to a value from Amazon's library);
  - what Settings shows of this computer is `hardware.rs`: it asks the system itself (registry,
    `GetAdaptersAddresses`, `getifaddrs`, /proc, /sys) and never a program that may not be installed; try a
    change on each kind of system it is for;
  - the search for LM Studio servers on the local network is `lmsnet.rs`: what another computer answers is
    used by its shape only, no answer is followed to another address, and it searches only when Settings asks;
  - LM Studio on this computer is `lmstudio.rs` (its state, its models, the shared folder, and the app's
    upkeep: `keep`, `serve_everywhere`, `announce`). A test gives it a made-up user folder and a stand-in `lms`
    (`examples/lms_stand_in.rs`) and never runs the real `lms`, reads or writes the owner's LM Studio, or
    downloads a model; only what `is_variant` passes reaches `lms get`, and a path of the shared folder's list
    goes through `inside`; in a test nothing is announced on the network (`FACE_TEST_ANNOUNCE` names a file
    that takes it), and everything that starts the app for a test sets `FACE_LMSTUDIO=off`;
  - Music's settings are `spotify.rs`, `music.rs` and `musiclog.rs`: a test never asks Spotify and never opens
    a browser (another address for Spotify only from `FACE_TEST_SPOTIFY` and only `http://127.0.0.1:<port>`:
    never widen that); Spotify's tokens stay protected in the private folder and reach no page, and its answer
    is taken only by `spotify::wait_for_code`;
  - Music's listening is `listen.rs` (every output and every microphone, through `cpal`: the owner's decision,
    2026-11-21, with "every device listens to its microphones as well as its outputs": never drop a kind on a
    system that can give it, nor one microphone of a system. The one exception, the owner's decision of
    2026-11-22 after the first was tried: a Mac never listens to an iPhone or iPad that macOS offers as a
    microphone, "I want the iOS app to listen to sounds separately from the macOS device even when they are
    connected". Opening it takes the phone over (it shows "Pause or Disconnect" and its own app stops), so
    `listen::interfaces` leaves it out, told by the transport the system names for the device and by its name,
    and the tracker page's voice does the same by the name (`ownMicrophone`); a phone listens for itself),
    `sound.rs` and `songlog.rs`: a sound stays in memory while it is measured and is never written. The
    sound-recognition model there is `yamnet.onnx`, one of `face-tools`' own models (`models::OWN`, published
    in the project's models release).
- **What a computer needs beside the programs** (`crates/face-tools`, `face-tools get`): the models, the model
  runtime, MediaPipe's web runtime, FFmpeg, the HEIC libraries (built from source), into `<root>/models` and
  `<root>/runtime`. Every download is pinned by size and SHA-256 and kept only when it is that file (`fetch.rs`:
  `save`); only named files are taken out of an archive; the model list is `models.rs`. A new pin goes in with
  the file downloaded once and its size and SHA-256 taken from that. A Mac's runtime is 1.31.1, not the others'
  1.24.4: on Apple's M4 and later (Arm SME2) 1.24.4 keeps memory for every new picture a model is given, 200 GB
  in twenty minutes of tracking (2026-11-22); a test that runs one picture again and again never shows it. The
  phones' apps carry 1.31.1 as well (an iPhone 17 has such a processor). So before a Mac's (or a phone's)
  runtime is changed, `crates/face-core/examples/grows.rs` is run with the new library on such a Mac, and a
  memory fault is looked for with pictures that differ. Its tests use a server on this computer and never the
  internet. On Windows the programs are built with the C runtime linked in (`.cargo/config.toml`), the HEIC
  libraries likewise, and ONNX Runtime's library gets the four Visual C++ runtime libraries it names beside it
  (`parts::get_vc_runtime`); `face-tools have` then says whether anything in `<root>/runtime` still needs
  something installed. A program that finds something missing at its start says so before it serves anything
  and fetches it when told to (`face-server --get`, or a yes at the prompt; `face_tools::first_run`). The two
  model tests want `FACE_ORT_LIB`, the full path of ONNX Runtime's library.
- **Photos and videos** (`crates/face-import`): a face's permanent ID is made from where its box lies in the
  decoded picture, so every device must decode a photo to the same bytes, now and for every library already
  made (the owner's decision, 2026-11-20). JPEG is libjpeg-turbo built into the program (`decode.rs`: shrunk
  while decoding, but a file of several pictures, an "MPO", decoded whole: `several_pictures`); PNG, GIF and
  WebP are Rust's own decoders; the shrinking is `resample.rs`; HEIC is libheif with libde265, a separate
  library loaded when a HEIC is read (`heif.rs`: `FACE_HEIF_LIB`, beside the program, `<root>/runtime`); videos
  are FFmpeg as separate programs (`video.rs`: `ffmpeg` and `ffprobe` by `FACE_FFMPEG`, beside the program,
  `<root>/runtime`). The versions of these libraries are fixed with the pixels they give. FFmpeg only decodes:
  the decoded picture is taken as it is (`-flags:v +bitexact`, raw) and its colours are converted here
  (`yuv.rs`, the whole-number arithmetic of FFmpeg's x86 code, which is what the computers every library was
  made on gave). FFmpeg's own conversion differs from one processor and build to the next (measured), its
  decoders don't: never let FFmpeg convert colours or scale a frame, on any system. The app's own FFmpeg is
  built by `packaging/decoders/build_ffmpeg.sh` (pinned source, LGPL options only;
  `.github/workflows/decoders.yml` for the four systems): change its options only with a comparison, frame by
  frame, of what it builds. Both are LGPL: keep them separate files, never linked into the program. FFmpeg is
  told a video is a file and may open files only (`as_file`, `ONLY_FILES`): a video that names an address
  inside must never be fetched. A change to how a photo, a video frame or a face's picture is read needs every
  photo of a library read again, byte for byte. A face looked for again in a part of its photo (a face's
  picture made again, a drawn face's points) is padded as the detector's own model pads (`yunet::MODEL_PAD`,
  32), never as whole photos are for the graphics processor: the other padding moves the eyes and mouth
  (measured). The phones' app is built without all of it (`face-server`'s `import` feature off in
  `crates/face-app`): a phone keeps no photo library.
- **The picture steps** (`crates/face-core`) must give OpenCV's bytes (`crates/face-core/tests/parity.rs`,
  against numbers OpenCV gave): change one only with that test.
- **The tracker page** (`web/`) links to the page for joining a shared database (`crates/face-server/pages`:
  `/join`) only where the server names it (`join` in `/api/people`, `#joinLink`).
- **MediaPipe's parts** (cues, mesh, posture, fingers) are the tracker page's, with MediaPipe's own web
  build (`crates/face-server/pages/parts.js`; its version is fixed with the readings it gives; the runtime is one pinned
  download, `face-tools get mediapipe`, served by the tracker itself under `/mp/`, never from the
  internet). The tracker page calls it only where the server names it (`parts` in `/api/people`, `loadParts`).
  They run BESIDE the page, in workers (`parts.js` `loadBeside`, `pages/parts_worker.js`, which loads the
  same module and runs `load` there): in the page they held its one thread 100 to 150 ms a
  check with a face in view, on the fastest computers too, and that was what a check then cost. A computer
  gets a worker for each model and the three run at once; the body and the hands, which need no face's box,
  begin with the frame while the server still reads it for faces (`begin`, only while a face is in view).
  A phone and a Raspberry Pi get one worker for all three (`beside` in what the server says: each worker
  is the whole runtime again in memory; the Pi's by its own word, `face_core::behind::raspberry_pi`).
  Measured with a face in view and all five on (2026-11-24; checks a second): 4.4 in the page and 10.1
  beside it in Chromium's window on Windows, 9.2 and 19.8 in WebKit's on a Mac; one worker on an iPhone 12
  Pro Max left the checks where they were and lifted the stream. The answers must stay the same to the last
  digit as the page's own run on the same frames (compared in both engines before it went in: do that
  again when the way a frame reaches a worker changes). Where the workers can't be had or one stops, the
  page runs the parts itself from then on (`loadAny`), with a turn of its own loop between two runs
  (`breathe`). `FACE_PARTS=page` and `=one` name the way, to compare. The worker is of the old kind, not
  a module: MediaPipe's runtime loads its script with `importScripts`. It sends nothing and keeps nothing.
  For it the tracker's page policy has `'wasm-unsafe-eval'` (WebAssembly may be compiled; `eval` may
  not). It runs on the processor: the graphics processor gave other answers (measured), so don't switch
  it. What it makes of MediaPipe's output (`faceCues`, `poseForFace`, `handOnFace`, `handsForFaces`, the
  landmarker's region) is held to fixed numbers by the Node test beside the module: change one only with
  that test. On a phone
  the cues cost a tenth of a second a face, so the evidence that asks for them starts off there.
- **Joining** (`face_db::join`, the pages' side in `crates/face-server` `join.rs`; the owner's design,
  2026-11-22: "as long as 1 device holds the secrets ... any device that wants to join has to auth with it -
  once the new device is joined - it has the same features - it can allow other devices to join with the
  same methods"). Either device shows the code and the other reads it: a device of the database shows one
  for a new device (`Offer`, `ask`), or the new device shows one and a device of the database reads it
  (`Want`, `give`). The code holds no secret, only a key for one showing, two minutes and one device,
  where the showing device listens, which way it is for and the showing device's name (shown to the owner
  as text, never trusted). What is handed over (the login, and the passphrase for keys when the handing
  device holds it) crosses the local network encrypted for that one exchange, and a device of the database
  sends nothing before the other proved it holds the code's key. A device listens beyond 127.0.0.1 only
  while it shows a code (port 8770). Don't put the login, or anything that stays valid, into a code.
  A device of the database hands over only at its owner's word: the click that shows a code, or the yes
  to the new device whose code it read (reading one hands nothing over), and before either the system's
  own check of who is at the device where it has one (`join::OwnerCheck`: never the app's own face
  recognition, which has no test for a live face). A handed passphrase is kept apart
  (`face_home::passphrase`) until the device's sync found it to be the current one (`Keys::enter`): what
  another device hands over never sets or changes the passphrase. Codes of the first kind (the
  login alone, a device of the database shows) are still read and never shown. A device that gets in is in
  at once, and every device says so (the owner: "an alert is generated"): each device's sync notices a
  device it has not seen in the database before (`face-sync` `home.rs` `new_devices`, the ids it knows in
  `HOME/known-devices.json`) and raises an alert of the kind `joined` whose id is made from the new device's,
  so it is one alert on every device, also for a device that got in with the login and nobody's help. A
  device's first look raises nothing, and neither does its first look after more than a week away. Whether
  a device is new is this device's own knowledge (it has not seen that id): never a date in the row, which
  whoever writes the row sets. The file keeps the certificate each device showed as well, and a device this
  one knows that shows another certificate raises the same kind of alert, once: the same device set up
  again, or another machine under its id. A row is read without trusting its shape (`try_get`).
- **A device's name** (`devices.alias`, migration 14; `face_db::set_alias`, `name_device`, `device_names`). The
  owner's words (2026-11-23): "We need to be able to create device aliases (for a device id)", "in the UI",
  "unique constraint on device name - similar to an email account", "if 2 devices have the same name on first
  join - add an incrementer to the device name and then ask the user of the device", and "the device alias
  has nothing to do with its DNS resolution". So: the alias is what the owner calls a device, kept in the
  shared database by the device's id, and every statement that shows a device's name reads it
  (`face_db::DEVICE_NAME`: the alias, else what the device calls itself). It is shown as text and used for
  nothing else: never looked up, never part of an address; devices reach each other by the addresses they
  note in the database and by nothing they are called. A name is one device's alone, whatever its capitals:
  the table's own index holds that for aliases, and `set_alias` refuses a name another device goes by. A
  device that calls itself what another goes by already is given a number when it is noted
  (`tell_apart` in `register`: "iPhone 2"), kept as its alias, and its own join page then asks whoever is at
  it for a name (`/api/join/name`, the page's `namePart`). Any device may name any device (Settings →
  Shared database, `/api/db/alias`): a phone has no Settings tab. A name is written on a connection made
  for it; the one a program keeps for a page's short questions only reads. A new statement that shows a
  device's name reads `DEVICE_NAME` too.
- **A device's location** (`crates/face-server` `location.rs`, `towns.rs`; `face_home::location`;
  `face_db::location`, migration 15). The owner's words (2026-11-24): "add geolocation for the devices and show
  the location of each device on the" Face tracking "and cameras tab, and keeps its location history in the
  database", "an opt-in setting in the Settings tab", "Also show state, and country if possible", and of the
  history: "only keep the On a move for last 90 days". Their choices the same day: the system's own location
  service with a place typed by hand where there is none, the town's name with the coordinates as text (no
  map), as exact as the system gives. So:
  - It is off for a new copy, switched on device by device (the tracker page's `optLocation`; `face_home::location::OPTION`),
    and while it is off nothing is asked, shown, published or written (`Location::step`: a test says so).
    Never ask a location service, or keep a location, from anything but that switch. The switch and the typed
    place never come from another device: both are in the sync's `STAY_HERE`.
  - Where it comes from: a place typed on the device (`PLACE`: coordinates, or a town of the carried list),
    which asks nobody; else the system's own service, where the system has one the app can count on: Windows
    (`Windows.Devices.Geolocation`), a Mac (Core Location, `location/mac.rs`: its manager, delegate and run
    loop on one thread of their own; the signed app has the location entitlement and the usage words), a
    phone's shell (`face_app_location_wanted`, `face_app_location`, `face_app_location_refused`: the shell asks
    its system, and the system its owner, only while the answer to the first is yes; while the app is in use,
    never in the background). Linux and the Raspberry Pi have none: there a place is typed. Never an address
    lookup at an outside service, and never a map from the internet (the pages load nothing from there).
    What the system's own service does with the networks it sees is said in `PRIVACY.md` and in
    `docs/SETTINGS.md`'s "What reaches the network": keep both true.
  - The towns (`towns.rs`, `crates/face-server/data/towns.tsv`, made by `examples/towns.rs` from GeoNames'
    files: CC BY 4.0, named in `NOTICE`): a point is named from this device's own list, also another
    device's point (`location::town_at`), so no words about a place come from another device. A new list is
    made with the example and goes in with its test.
  - What is made of it: the page's line (`/api/location`, `web/place.js`: checked there, put in as text
    only); the node's `place` on the bus (numbers and one word, the server's own branch, not a page's; the
    typed place is marked secret in the page's settings list, so it is never among the published settings);
    the Cameras list's `place` for each device (`tracker.rs` `places`: this device's own, another's as its
    node says it while a page follows it, else the newest row of its history); and the history.
  - The history is only ever sent: a row when the switch is turned on, then when the device has moved
    (`face_home::location::moved`: `MOVED` and both readings' uncertainty) and the last row is a minute old
    (`APART_MS`). Both are choices, not measured values. Rows wait in `HOME/location.json` (at most
    `MOST_WAITING`) and go on the tracker's own connection to the database, with its "this device is here"
    (`location::send`, in `watch_device` and `start_heartbeat`): a phone has no sync, so the sync is not
    their way. A database that can't take them never ends that connection's other work. Rows older than
    `face_db::location::KEEP_DAYS` are deleted once a day, every device's. Nothing waits on a device without a
    shared database. The rows are about a device, not a named person: they are in no person's export, and
    Settings deletes a device's history (`/api/db/locations/forget`).
  - A test device's location is switched on by its app's environment (`FACE_LOCATION=on`: the server says so
    in `/api/people`, and the page clicks its own switch once after it loaded), for a device whose page can't
    be reached from outside. It is no part of `FACE_SIGNALS`: a location has a switch of its own.
  - Its tests ask no location service: a test says what a system would (`location::told`), and the one
    example that does ask (`examples/where.rs`) is for trying a kind of system by hand and prints no place
    unless told to. Never print a device's coordinates in a log line or a check's output.
- **Removing a device** (`face_home::blocked`, `HOME/blocked-devices.json`; Settings → Shared database, the route
  `/api/db/device`). The owner's words (2026-11-22): "the settings let you remove devices that you dont want to
  see the data ONLY on your device. You should also be able to add devices back as well - per device", and "I
  dont want the removing devices to change database passwords or keys". So it is this device's own list, in
  no row and never synced, and it changes nothing in the database: never add a removal that changes the
  password, the passphrase or a key, or that takes a device's row away. On the device where it is set, the
  removed device's requests get no answer from the peer service (`peer.rs` `judge`, read for each request),
  its camera is neither listed nor asked for (`tracker.rs` `cameras`, `cameras_asked`), it is not asked for a
  photo (`face_db::kept::holders`'s `without`) and its LM Studio is sent no photo to describe
  (`other_devices_addresses`). A new place where this device asks another one, or answers one, reads the
  list too. What it can't do is said where it is set: that device still has the database's login, and the
  devices' requests are signed with a key they all have, so the list binds a device that says who it is.
  A device is removed together with the certificate it had then (`blocked::set_with`, `certs`): a machine
  that shows a removed certificate is no member here under any id (`peer.rs` `is_member`), since whoever has
  the login can write a row with a new id. Added back, its certificate is forgotten with it. Its tests use listeners on
  this computer; the two against a real server (`crates/face-db/tests/server.rs`,
  `crates/face-sync/tests/server.rs`, with `FACE_TEST_DB_HOME`) work in a schema named `t_...` that
  they make and drop. A check that runs whole programs (two test copies, a headless browser whose camera
  shows the other's code) gets such a schema from `crates/face-db/examples/test_schema.rs`, which makes and
  drops only names of that kind.
- **The app** (`crates/face-server` `app.rs`: the tab page's server, the ports, starting and watching the
  tracker's process, starting again). Its shell never asks `/update` or `/open-release` in a test (GitHub, a
  browser) and never `/open` for one of the two folders (a window).
  - What the app's two processes ask each other is `trackerlink.rs`: only under `/api/local/`, only with this
    run's secret (made by the app, in its child's environment, in no file, page or log), `hello` the one
    question without it, a photo by content hash only.
  - The window is `apps/desktop` (the program `FaceRecognizer`: the app with its window, and as
    `tracker-child` the tracker's own process): only the tracker this process started and the tab page around
    it get the camera and microphone unasked (`App::camera_allowed`), the window shows the app's own tab page
    and nothing else, and its browser's switches are the measured ones (`BROWSER_FLAGS`: change them only with
    the measurement). A check of it never shows a window (`FACE_UI=hidden`: no browser is opened then either),
    uses the browser's debugging port only in a test copy (`FACE_WINDOW_DEBUG`: never set in the app as it is
    run), and ends only its own copies of the program, told by the program's file. The window's watchdog and
    the two ways the app is started again (`watch_step`, `window_restart_due`, `restart_soon`, `restart_app`,
    `wait_for_previous`, with `HUNG_AFTER`, `RESTART_AFTER`, `RESTART_EVERY`, `WINDOW_BACKOFF` and
    `HOME/restarted.json`) are measured: change them only with a measurement. The window is shown only once
    its browser is there and destroyed when the browser can't be made (dropped, it would stay: its loop never
    runs then); only a page's own process ending loads the page again (a reload starts every camera again).
  - The Mac's window is `apps/desktop/src/mac.rs`: the web view library's own answer to the camera's request
    is yes to every page, so it is replaced by `camera_asked` (`App::camera_allowed`, by the page's address)
    and the library is told "ask" for when that fails: never leave a path where the answer is yes without the
    address; WebKit asks the window's navigation handler about every frame too, so what stays in the window is
    `App::own_page`, and a window that isn't shown never opens a browser, on any system (a hidden test copy
    handed its three frames to the Mac's browser, 2026-11-21); `KEEP_RUNNING` holds the measured switches. A
    check of it runs on the Mac with an app of its own bundle id, shows nothing unless told to (the camera's
    answer: someone has to be at the screen for macOS's own question), ends only that copy, and never steers
    another app.
  - The first-run question is `firstrun.rs` with its page (`pages/first_run.html`, `first_run.js`): what the
    app is missing is asked about at its own address before the tab page is shown and before either program
    starts, and nothing is fetched without a yes there (or `--get`); a "not now" is remembered only for what
    one feature needs, never for what the programs can't work without (`firstrun::asks`). In a test the
    downloads come from a stand-in (`FACE_TEST_RELEASE`, only `http://127.0.0.1:<port>`, only the models: never
    widen that); a test never fetches the models or the runtime from the internet through the question.
  - The programs carry the page files (`pagefiles.rs`: a new page file goes into its list, and a test names
    every file a server serves that isn't there); `--pages` reads a folder instead. The app program's folder
    for models and data is `paths::app_root` (macOS and Linux: the user's own `FaceRecognizer` folder, where
    the installed app keeps the real library): a test copy of `FaceRecognizer` always gets `FACE_ROOT` and
    `FACE_HOME`.
- **The installers.** The Windows one is `packaging/windows` (`FaceRecognizer.iss`, `build.ps1`): it never
  deletes or touches `models` or `data` (no `[UninstallDelete]`, no deleting code; its one `[InstallDelete]`
  takes away the FFmpeg an earlier Setup left in `runtime\`). It puts FFmpeg beside the program, not into
  `runtime\`: signed, it is no longer the pinned file, in `runtime\` only the pinned file counts as there, and
  the app would ask to download FFmpeg after every install. It installs without administrator rights, and
  closes an open app itself; it keeps the AppId the app has always had (it replaces an installed copy in
  place), so on a computer with the app installed only its `-Trial` form is ever run for a test, installed
  and removed again in its own folder; a real first download (about 110 MB) is for a person, once, never for
  CI. Setup carries Microsoft's WebView2 bootstrapper (the owner's yes, 2026-11-21) and runs it only where the
  runtime is missing, after asking, never into the app's folder; Microsoft has one address for the newest and
  none for a given one, so when the pin in `build.ps1` no longer fits, a person looks at what came (its
  signer) and sets the new size and SHA-256: never drop the pin or the signature test to make a build pass. A
  test never runs Microsoft's bootstrapper (it would install Microsoft's runtime on the computer): it builds
  its installer with a stand-in (`-WebView2`, only with `-Trial`), and the switch that takes the runtime for
  missing exists only in the trial's installer. The Linux and Raspberry Pi installer is `packaging/linux`
  (`install.sh`: never `models` or `data`, no sudo, autostart only when asked, a build for another processor
  refused; `build.sh`), tried under a made-up home folder only; the Mac's is `packaging/macos` (`bundle.sh`,
  `build.sh`), tried with a bundle id of its own, never the app's, and never copied into Applications. What an
  installer brings lies beside the program and counts as there by its names (a signed Mac app's libraries are
  no longer the fetched bytes); what the app fetches counts only as the pinned file
  (`first_run::missing_with`): keep the two apart.
- **A device's camera for the other devices** (`crates/face-server` `camstream.rs`, `peer.rs`): the peer
  service's serving side (TLS with the device's pinned certificate, requests signed with the cluster key,
  `/stream`, `/readout` and a photo by its content hash only, frames and photos in memory only). Its bytes on
  the wire are fixed: devices of every version watch each other. In the app the service runs in the tracker's
  process at the sync's word (`tracker.rs` `local_peer`, `syncer.rs` `peer_step`), on the device's own peer
  port only, and a photo comes from the labeling tool's process over the link, never from a path a request
  names. Watching is `camrelay.rs` (a device's certificate checked against the database's SHA-256 inside the
  handshake; at most 64 relays: a device id is whatever a request says).
- **A photo that only another device has** (`crates/face-server` `peerphoto.rs`, used by `label.rs`'s photo
  view and `describejob.rs`), with the connection `camrelay.rs` makes (`pinned`). A photo is asked for by
  content hash only, taken only whole and up to `MAX_PHOTO`, kept in memory only, and never under the labeling
  lock; nothing is sent to a device whose certificate isn't the database's; and only while the sync runs and
  is online (`Inner::peer_ready`). Its test has three devices in a schema named `t_...` that it makes and
  drops, never the shared one.

## Security rules

(The modules named are in `crates/face-server` unless another crate is named: `guard.rs` and `http.rs` are the
servers' checks, `app.rs` with `apps/desktop` the app, `tracker.rs` the tracker, `label.rs` the labeling tool.)

- The programs' servers listen on 127.0.0.1 only. The one exception is the peer service (`peer.rs`, with a shared
  database, on by default there, Settings switch): LAN, TLS with the device's pinned certificate, HMAC-signed
  requests from the cluster key, original photos by content hash only, never a path or anything else; and
  the device's live camera as its tracker page shows it (`/stream`) with its readout (`/readout`, and as JSON parts
  in `/stream`), only while "Stream this camera" is on. The Cameras page's own port is the tracker's server a
  second time, on 127.0.0.1 with the same checks: each port answers only requests that name it (Host and
  Origin), so a page of one is another origin to the other.
- The app's two processes ask each other only under `/api/local/` (`trackerlink.rs`), and those routes answer
  only with this run's secret (`X-Face-Link`; made by the app, in the environment of the process it starts, in no
  file, page or log). `hello` is the one exception: it takes no secret and proves the answerer has it, so the app
  never sends the secret to a server it hasn't checked. A photo is given to the tracker's process by content hash
  only, as the peer service takes them.
- Every endpoint goes through `guard::refuse_reason` (exact Host and port; POST needs `X-Face-Tool: 1` and its
  own Origin). The app's shell uses it too: its GETs that do something (`/update`, `/open-release`, `/open`,
  `/attention`, `/restart`, `/restart-app`) need the header.
- Data, and anything a GET starts, goes only to the tool's own page (`refuse_reason` with `own`: the browser's
  `Sec-Fetch-Site: same-origin`, or the header from a program that isn't a browser). A page of another program
  on this computer (`same-site`) and an address typed or opened (`none`) get a tool's page files and nothing
  else; a test asks every kind of route both ways.
- Every answer carries the page headers (`guard::policy`): a Content-Security-Policy with `default-src 'self'`
  and only what each page uses (`guard.rs` `POLICY`; the tracker adds `blob:` scripts and media for its
  microphone worklet and Open video, and `'wasm-unsafe-eval'` for MediaPipe's runtime: WebAssembly may be
  compiled, `eval` may not; the tab page adds the programs' ports as `frame-src`; no inline script anywhere, so
  no `onclick=` attributes); `frame-ancestors` lets only the page itself frame it, plus the app's tab page when
  the app started the program (`guard.rs` `FRAME_PARENTS` is empty until the app adds its real port,
  `allow_frame_parents`: a program started on its own trusts nobody, and the app says so instead of showing
  it); the shell page can't be framed at all (`policy_never_framed`); `Cross-Origin-Resource-Policy:
  same-origin`, `nosniff` and `Origin-Agent-Cluster: ?1` (each server's pages in a process of their own: one
  page can't slow or read into another's). The tab page also sends `Permissions-Policy`: camera and microphone
  for itself and the programs' ports only (itself, because a page can't hand on what it may not use). Pages
  load nothing from the internet (a test checks every page file).
- Requests with `Sec-Fetch-Site: cross-site` are refused (another website's page, even as an `<img>`); same-site is
  how the tab page loads a program's page into its frame (the page's own requests after that are same-origin).
  Request bodies have a limit by their route (`guard::body_size`: negative or non-numeric `Content-Length` 400,
  too large 413).
- In the app window only the tracker this process started gets the camera and microphone without a prompt
  (`App::camera_allowed`), and only a program this process started is shown in the window at all
  (`App::own_page`). Any other page is left to the window's own question. The tracker's request reaches the
  window in the tab page's name, so the tab page's address is answered too: the tab page hands camera and
  microphone to the tracker's frame alone (`allow=` in `app_ui.html`; its `Permissions-Policy` header names
  itself and the programs' ports only) and its own scripts never use them; a test checks all three.
- Servers bind with `http::bind` (no address reuse on Windows, so a second copy fails instead of sharing the
  port), and an uncaught error in a handler still gets a JSON 500.
- One copy of the app runs for a private folder (`app.rs` `start`, `face_files::paths::only_copy`:
  `HOME/locks/running.lock`, taken before anything else and held for the run). A port that answers is not
  how a copy tells that another runs: a copy still loading holds its port and answers nothing, and a second
  copy then took the next ports and ran beside it on one library (a Raspberry Pi, 2026-11-23). Never start a
  program of the app before that place is taken, and never let a copy that couldn't take it go on. A copy
  that starts itself again is waited for (`ONLY_WAIT`); a test copy has its own private folder, so its own
  place.
- Anything put into `innerHTML` goes through `esc()` (names, paths, descriptions, web results, tags, errors).
  The Settings tab and the Cameras page's readouts are built with `textContent`/DOM properties only.
- On disk, keys and passwords are kept with `face_files::protect` (DPAPI on Windows, mode 600 elsewhere:
  `write_private`); read them through their module (`face_home::saved`: `google_key`, `aws`, `describe_key`;
  `face_db::saved_login`), never the file.
- Keys (Web search, describe API key) and the shared database's password reach a page only through `/api/secrets`
  (labeling tool; needs `X-Face-Tool: 1`, refused cross-site), for the Settings tab to show them masked, each with an
  eye button that shows it only while held (the owner's choice, 2026-11-11). Every other endpoint returns only
  whether one is saved. The Settings tab never stores them; nothing is written into page HTML or storage. In the
  shared database they exist only encrypted (`face-sync` `keys.rs`); the passphrase stays on each device
  (protected).
- The shared database is reached only over TLS checked against its own CA (`verify-ca`); LAN connections without
  TLS are refused by its pg_hba. The app's role (`facerec`) is not a superuser and owns only its schema.
- Only the two fixed folders may be opened (`/open?what=data|private`); photos are addressed by face index, never by path.
- Spotify's answer to **Connect** arrives on a listener of its own (`spotify::wait_for_code`), not on the three
  servers (their rule refuses requests from another site): 127.0.0.1 only, open for five minutes at most after the
  owner's click, and it takes exactly one request, the one carrying the state that click gave out. Spotify's tokens
  are kept with `protect` and reach no page.
- The music log listens to audio interfaces (outputs and microphones) only while its switch is on, and keeps
  numbers only: the sound of any interface is analysed in memory and never stored (`listen.rs`), like the
  tracker's microphone. The Signals chart's sound (`soundlive.rs`) is the same kind with a switch of its own
  (**Chart this device's sound**, off for a new copy): the tracker's server listens only while its page asks
  (`/api/sound/live`, every 2 s while the switch is on; the listening ends `LEASE` after the last question, so
  a closed page or a switch turned off leaves no microphone listened to), keeps the newest listening alone,
  as numbers and one word, in memory, and nothing of it is written. Never start that listening from anything
  but the page's question, and never keep a second listening or the list of sounds heard. A phone's app has
  no sound library, so there the page hears with the device's own microphone while the same switch is on and
  sends a few seconds at a time to its own server on the phone (`/api/sound/heard`, `soundheard.rs`: one
  channel, 2 to 8 s, measured and dropped; numbers and one word are the answer); the server says so itself
  (`page` in `/api/sound/live`'s answer), a computer's page never does this. The microphone running for
  that gives no voice readings: on the page `voice` is `ear` only while Listen to voice is on.
- Nothing the app carries may send anything anywhere by itself, a library's own telemetry included. The model
  runtime's is switched off before the runtime is loaded, in every program (`face_core::runtime::no_telemetry`
  first thing in each program's start, again where the library is loaded, and the builder's
  `with_telemetry(false)`: the `ort` crate's own default switches it ON). ONNX Runtime from 1.30 on carries
  Microsoft's telemetry client on every system but Windows, on unless `ORT_DISABLE_TELEMETRY=1`: it makes a
  device id and a store in the user's folders and sends Microsoft what it runs on. A library
  is looked into before it goes in, not after (this one showed itself only when the iPhone's 1.31.1 would not
  link without SQLite and a network monitor, 2026-11-23). So: before a
  runtime's pin is changed, or any library is added, `crates/face-core/examples/quiet.rs` is run with it (an
  empty user's folder, the network closed) and the library is looked through for addresses it didn't have
  (`strings`, `grep -a http`); a library that wants system parts it didn't want before (a database, a
  network monitor) is looked into before it is linked. Never remove the switch or the test that holds it.
- Downloads are pinned and checked by size and SHA-256 (`face-tools` `models.rs` and `parts.rs`, `florence.rs`;
  the Windows installer's WebView2 bootstrapper:
  `packaging/windows/build.ps1`, which also wants Microsoft's valid signature on it; the Android build's model
  runtime and Google's bundletool: `apps/android/build.ps1` and CI's `android` job; the iPhone app's model
  runtime: `apps/ios/runtime.sh`, out of Microsoft's own package on nuget.org, whose signatures are checked
  when a pin is made), and
  so is a model file found already there (`models.rs` `is_good`, `florence::good`).
  `lmstudio::from_share` checks each file against the shared folder's own list (a damaged copy, not a
  rewritten list: the folder is as trusted as the computers that write to it) and writes only inside LM Studio's
  models and hub folders, whatever paths the list holds.
- `settings_bridge.js` answers only its parent window, on `127.0.0.1`/`localhost`, and only when that is the origin
  the browser reports for the frame's parent (`location.ancestorOrigins`).
- `lmstudio.rs` changes another program's settings (LM Studio's `networkInterface` to 0.0.0.0, restarting its server
  on all interfaces, CORS left as LM Studio's own settings have it) and announces it on the local network: the owner's rule for every computer with this app and LM
  Studio (2026-11-10); `FACE_LMSTUDIO=off` turns it off. Model downloads start only from a Settings button.
- `hardware::ai_apps` asks 127.0.0.1's usual AI ports and the configured describe server (with that server's own key,
  through `describe::server_models`). `lmsnet::lmstudio_network` (Settings → LM Studio on the network, only while the
  tab is open, cached a minute):
  - takes announced servers from mDNS (`mdns_services`: `_lmstudio._tcp`, or `_http._tcp` with `app=LM Studio`;
    their own port);
  - tries port 1234, this computer's LM Studio port and announced ports on every address of the /24 of each real
    interface (`own_addresses`: no loopback, and no containers, virtual switches or VPN tunnels unless the owner switched on
    **Fully distributed system**, `hardware::distributed`, off for a new copy: then every adapter counts, the peer
    service publishes those addresses too, and `describe::helpers` may use servers beyond the local network);
  - reads each server's model list (`/api/v0/models`) and names computers by mDNS reverse lookup or NetBIOS name
    status; nothing is sent but those questions.
- Describe photo: the API key belongs to its server address (changing the address clears it; Test sends it only to
  that address); redirects aren't followed; answers are capped (`MAX_ANSWER`, `MAX_TEXT`).
- What other computers on the network say is checked before use: an LM Studio server's model list by shape
  (`lmsnet.rs` `lms_models`), a model's name by `lmstudio::is_variant` before it reaches `lms get`, paths in the
  shared models folder's list (`lmstudio.rs` `inside`).
- What the shared database says is checked before use too: whoever has its password can write any row, and so
  can a faulty version. Rows go through `face-sync` `apply.rs` and each part's `check`. A row never gives this
  device a file path (`PHOTO_FIELDS`; `label.rs` `own_photo` shows only a photo whose record has a path and a
  size) and never changes its settings (the sync's rules above). A row can still change or delete what the
  devices share (a label, a description, a forgotten name): that is what the database is for.
- Photos leave a device for describing only to the configured describe server and, with Share Describe all on,
  to the owner's other devices (`describe::helpers`: an address a device of the shared database gave there, or
  this computer's own), never to another computer that answers like LM Studio.
- No answer of the three servers may be kept by a browser (`http.rs`: always `no-store`; a test
  checks every kind of answer). A page that wants a picture twice keeps it in memory.
- Background jobs (imports, possible-face scans, Describe all) start through the importer's `begin` (`face-import` `importer.rs`: one at a time, under
  a lock). Shared caches and counters used by request threads need a lock. The labeling data changes only under
  the labeling lock; what a request reads without it (a face's picture) must not walk a list
  that grows meanwhile (the labeling engine makes its tables, `printless` among them, under the lock).
- Every person's data can be exported and forgotten (`persondata.rs`): a new place that keeps anything about a person
  must be added to its list, `export` and `forget`, and to its test.
- The Windows installer for the Microsoft Store is the same installer (`docs/store/microsoft-store.md`): the
  Store runs it silently with `/OFFLINE=1`, which keeps Setup from downloading anything (Microsoft's WebView2
  runtime, where it is missing), and takes it only when every program and library in it is signed: `build.ps1
  -Sign` signs copies of the project's own builds in `runtime\` (FFmpeg, the HEIC libraries; the fetched files
  stay the pinned bytes) and stops if anything there carries no valid signature. A release whose installer the
  Store points to is not deleted from GitHub until a later one is published there. The Store's package address
  is not GitHub's (it redirects, which the Store refuses) but `downloads.singularityresearch.net`, a Cloudflare
  Worker (`packaging/windows/store-address`: `worker.js`, `wrangler.toml`) that fetches the named release's
  installer from GitHub and hands it on unchanged. It answers only for this repository's releases and the files
  it names, only GET and HEAD, and keeps nothing: never widen what it passes on, and never have it change a byte.
- Windows code signing keeps no key and no password anywhere: the key is in Azure Artifact Signing, and CI signs
  in as this repository's workflow on `main` (GitHub's own token for the run, which Azure is told to trust for that
  branch alone, with the Signer role on the one certificate profile). The five repository secrets are names and
  ids. Don't add a client secret or a certificate file to sign with, and don't sign other branches or pull requests.
- The iPhone app is signed for the App Store by Xcode's automatic signing on the Mac, with the developer's own
  account there and Apple's cloud-managed distribution certificate: no certificate, key or profile for it is kept
  in the repository or in CI. Never print the team id, a device's id or the account.
- The Android app's release key (`apps/android/make_key.ps1`) is the owner's and is kept like a password: on
  the PC under `%LOCALAPPDATA%\Android\release` (its password protected for that Windows user), in the
  owner-only backup, and as two repository secrets (`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`)
  with which CI signs the package on pushes to `main` and nowhere else. Never print the key or its password,
  never pass the password on a command line (the signer takes it from the environment), and never commit
  either. The certificate's SHA-256 is public. A package for trying is signed with a debug key made on the
  spot; it and a release are different apps to a phone.
- Open items from the 2026-11-09 review are in a document the owner keeps.

## Conventions and constraints

- Keep `README.md` and `docs/` in sync with any user-visible change, and this file with layout or rule changes.
- `PRIVACY.md` is the privacy policy the stores' listings point to: a change to what the app keeps, to what
  leaves a device or to a phone's permissions goes into it too (with a new date at its top), as into
  `docs/SETTINGS.md`'s "What reaches the network". The app shows it itself (the stores ask for the policy
  inside the app): the tracker's `/privacy` (`crates/face-server` `privacy.rs`) is that file, carried in the
  program and turned into a page that leads nowhere outside the app (a link stays one only to another part of
  the policy; a test says so), and the tracker page opens it in a sheet over itself, never by leaving the page
  (the camera goes on behind it).
- The body readings (the pulse estimate, the breathing rate) are health features to the stores (the owner's
  decision, 2026-11-22: the phones' apps keep every sensor, so they are declared, not left out). The words "not
  a medical device and does not diagnose, treat, cure or prevent any medical condition", with the reminder to
  ask a healthcare professional, stay on the tracker page (`#medicalHint`), in `PRIVACY.md` and in each store's
  listing (`docs/store/`); a page test holds them to each other. Never word a reading as a diagnosis.
- Nothing may be assumed to be installed on a user's computer (the owner's rule, 2026-11-20): every library or
  program an app needs is included in it, or its installer asks and brings it, or the app asks at its first
  run, before the feature is needed. Never "found if it happens to be there", never a failure later at the
  photo or video that needed it. That covers what a library itself needs: on Windows the Visual C++ runtime
  (VCRUNTIME140, MSVCP140) is not part of Windows. Read a shipped file's imports before calling it
  self-contained (`face-tools have` does, `crates/face-tools` `needs.rs`). A build that needs CMake, a compiler
  or an assembler is for developers and CI: what is built from source reaches users already built.
- A new setting goes in `docs/SETTINGS.md` too (its section, default, what it is for; and in "What reaches the
  network" if it sends anything anywhere). A new environment variable goes in its table there.
- A new option on the tracker or labeling page should also be listed in that page's `SettingsBridge.init` list,
  so it shows in the Settings tab. Options the user changes should be remembered (`remember`/`store.set`).
- Thresholds are **measured values** — don't change them without re-measuring on `data/`:
  `face-core` `faces.rs` (`REC_MODELS`: each recognition model's `match_at`, `auto` and `cluster`, which belong
  to the model and are measured with it by `rec_eval` on a copy of a library), `Q_ENROLL`, `Q_EXPR`, `MAX_TURN`,
  `labels.rs` (`MARGIN`, `TOPK`, `DUP_BITS`), the tracker page's strictness (its default is the model's `match_at`),
  and `expression.js` (`X_SURE`, `X_MARGIN`, `X_MIN_VOTES`, vote windows).
  Also measured, for speed (2026-11-10, Ryzen 7 7840HS + RTX 4060 and Ryzen AI Max+ 395 + Radeon 8060S): `face-core`'s
  `GPU_TOLERANCE`, `DET_PAD` and `DET_SESSIONS`, and `face-import`'s `import_workers`. A GPU path
  must give the same faces as the CPU path: re-run the comparison (regular faces identical) when changing them.
- Not yet measured, so evidence only: `expression.js` `H_STAY` (state model) and every `signals.js` constant
  (`TALK_JAW_SD`, pulse, voice). The options that let them change the label (skip while talking, state model)
  stay off by default until a measurement on posed sessions shows they help. Same for the hand-on-face skip.
- `data/` is local only: the whole folder is in .gitignore and was purged from git history and releases
  (2026-11-09). Never commit anything from it, or personal details (names, paths, accounts, host names, addresses) in
  docs or code. A fresh copy starts with an empty `data/` and every feature must work from there; a face's picture
  (`Store::thumb`) may be missing.
- Labels live per face in `data/labels.json` (keyed by permanent face ID); change them through the
  labeling tool or the labeling engine (`labels.rs`), not by hand-editing JSON. `cluster` group numbers are temporary.
- A new recognition model gets a new name in `REC_MODELS`, its own measured settings and its own index in a new
  migration. A library whose prints are of another model has every print made again when the labeling tool opens
  it (`LabelDb::open_making`: from the picture the library keeps of each face, all or nothing, the old prints kept
  beside the library), and its device then sends them. Prints of two models can't be compared, and a device takes
  only the shared database's rows of its own program's model: all devices of one database are updated together.
  A face left without a print would reach the other devices as an Ignored one: never switch a library part of
  the way.
- Name overlay belongs on the face near the top of the head (owner preference).
- The labeling tool stays a separate program from the tracker (owner preference): separate code, server and page.
  They share one window as tabs (`app.rs`), but each still runs on its own.
- Face-shape cues are evidence only until fusion is measured on posed sessions.
- Expression readings are stored, as numbers and short labels only (the owner's decision, 2026-11-12, for reports
  later; before that they were never stored): in the shared database's `expression_log` (`face-home` `exprlog.rs`; nothing
  without a shared database), in each alert's measured details (`alerts.rs`), and in opt-in posed test sessions
  and feeling reports (`data/expr_eval/`). Never a picture or sound with them. Microphone audio is analysed live
  and never stored; per-person baselines and usual looks are kept by
  the tracker in `HOME/person_signals.json` (never `data/`) with a copy in the page's storage, and in the shared
  database when one is set up; keep README wording honest ("looks", not "feels").
- Camera frames (streamed or relayed) are kept in memory only, never on disk or in the database.
- The event bus (`crates/face-server` `bus.rs`, `busrelay.rs`, the peer service's `/events`, the tracker's
  `/api/bus`; `face_db::members`). The owner's design (2026-11-23): "all datapoints should be broadcast to
  secured subscribers", "all nodes that currently authenticate should share a common API and you can subscribe
  as the base for your messages about state changes in general", "everything that a device publishes (its
  settings) should be able to be securely published and subscribed to - and the UI itself should also use this
  approach", "like all data produced from settings - if it was a complete schema in JSON - and what is enabled
  and what is not and what its readings are", "same as we see on the signals charts", and, more than once, "it
  should NOT be dependent on a centralized master" ("like the NAS"). PostgreSQL's place at the end of that road,
  in their words: "a separate element in the solution, an option that you can enable for trend analysis - but
  it's still a subscriber - the apps should also get their data through this peer subscriber method".
  What is built (the first part): each node publishes ONE JSON tree: `node` (what it is: the server says it),
  `place` (where it is, while its owner has switched its location on: the server says that too),
  `catalog` (every section and signal of the Signals chart as data, `SignalsChart.catalog`), `state` (what is
  on, what is off and why), `settings` (every setting of the tracker page with its value), `inview` and
  `readings` (every reading of the last seconds, each with its own time). The first four and `place` are kept in the tree
  (`Bus::set`); who is in view and the readings are only said (`Bus::say`: heard by who is subscribed at that
  moment, never part of the tree a later subscriber is given), and only while **Stream this camera** is on:
  they say who is in view and how they look, as the readout does, so they go out when the readout does. A topic is a path in that tree; a
  subscriber gets the whole tree first and then each change with the publisher's own number (`Bus::since`),
  waiting on the connection when there is nothing new: nothing is polled, and a subscriber to some paths is
  not answered for what changes elsewhere in the tree. A node keeps a copy of another node's
  tree only while one of its own pages asks (`Followed`, `Bus::apply`), and pages subscribe to any node through
  their own server (`/api/bus?node=`). The tracker page publishes its readings as it takes them
  (`busReadings`: every 250 ms what is new since the last time, packed as for a readout), and the Cameras page
  follows every camera's device for them in ONE waiting question (`followAll`, `/api/bus/many`,
  `Followed::since_many`: a browser gives a page a handful of connections to one address, and a question for
  each camera took them from the pictures); the readout still carries the last seconds for a device
  that publishes none, and a chart keeps each reading once, by its own time, whichever way it came. Its rules:
  - No master. Nothing of the bus may ask the shared database, or any one machine, when a node publishes or
    subscribes. Nodes are found and checked from this device's own member list (`HOME/members.json`: filled
    from the database whenever it answers; never the other way round), and the cluster's key is made from what
    the device keeps (`members::kept_key`), so the peer service starts and answers while the server is away.
    A new way a node learns of members goes through `members::keep`, which checks every field. What the
    database gives is the whole list: a device that is no longer there is no member from then on. A member is
    asked only at an address of a local network (`camrelay::askable`: private, link-local, a provider's
    shared range), unless the owner switched on **Fully distributed system**: the list is filled from rows
    anybody with the login can write, and a row must not send every device's requests out to the internet.
    A login the database refuses (`face_db::login_refused`) takes the cluster's key from the running
    service: a device whose login was withdrawn stops answering as a member.
  - `/events` answers only a request signed with the cluster's key AND made over a connection in which the
    asking device showed its own certificate, the one the member list has for the id it names (`is_member`).
    Never relax either. This is what makes a subscriber a known node, and what makes a device removed on this
    one stay removed whatever it calls itself; the stream ends when its device is removed, is no longer a
    member, or when the service is stopped (`goes_on`). The service's own limits stay: a request's head within
    `HEAD_WITHIN`, `MAX_OPEN` connections of which one address has `MAX_FROM_ONE`, one people file given at a
    time (`PEOPLE_BUSY`, read once and kept by the file's size and time, `people_from`).
  - The people for a device without a library (`peer.rs` `/people`, `busrelay::people`, the tracker's
    `watch_shared`): a computer gives its people file as it is, to a member that showed its certificate, as
    for `/events`; a device that has no people file takes it from the first member that gives one and asks
    the shared database only when none did. The file is taken whole or not at all, up to `MAX_PEOPLE`, read
    as what another device wrote (`Identifier::from_foreign_npz`, `npy::read_npz_foreign`: stored entries
    only, a few arrays, every size checked before anything is allocated; a file that can't be used is asked
    of the next member and not fetched again, `UNUSABLE`; another recognition model's is not used), and kept in memory only: never write
    what another device gave of people to a phone's storage. A device that took its people from another gives
    them to nobody (it has no file): a node gives only what is its own.
  - The cameras too are found from the member list (`camrelay.rs` `from_members`): where a device is and the
    certificate it must show are read from this device's own file, and the shared database is asked only for
    a device the list doesn't know yet or whose place in the list no longer answers. When the database can't be asked, the Cameras page's list is the
    member list (`tracker.rs` `cameras`): every member that shares is listed as one whose camera may stream,
    since only the device itself says whether it does. A new place that asks where a device is reads the
    member list first.
  - A node is the only authority for its own tree. The copy of another node's tree is never written from a
    page, and a page publishes only its own node's `PAGE_BRANCHES`; `node` is the server's to say.
  - Settings are published, never set from the bus: a device's settings change only on that device (the
    standing rule that nothing from outside changes a device's settings).
  - No secret is ever published: the page leaves out actions and anything marked secret (`busSettings`). A new
    setting that holds a key, a password or a token must not be given a place in that list.
  - Nothing is stored: the tree and its last changes are in memory, here and in every copy.
  - What another node sends is checked where it is taken (`Bus::apply`: a path is a path, a value and a tree
    have their sizes, a run's name is letters and digits) and again where it is shown (the chart's `clean`); a name or a word from it is only ever
    put into a page as text.
  - The transport is to become WebSockets (the owner, 2026-11-23: "websockets might be a great way to share the
    ledger and the messages"), pages first: the answers of `since` are the messages either way. A WebSocket
    library is a new library: the rule for new libraries applies before it goes in.
  - On the owner's own devices every sensor option is on and everything is published ("by default - have all
    physical devices - broadcast all sensors - just for our testing"): that is their test setting, not what a
    new copy starts with.
- The Signals chart's **Device** section (the owner, 2026-11-23: the sending device's type, capabilities,
  temperature and performance belong with the sensors, "per camera", "or any sensor source"): what a device says
  of itself. Its readings are asked of the system itself when its page asks (`crates/face-server`
  `devicelive.rs`, `/api/device/live`, every few seconds; nothing runs between and nothing is kept): processor
  and memory in use, the warmest thermal zone where the system has them (Linux, the Raspberry Pi), a battery
  where there is one. Only what the system says: never a guess, never a program that may not be installed, and
  no temperature where a system gives a program none (Windows, a Mac). A phone's heat state (the system's own
  four words; a phone gives no degrees) and its battery come from the app's shell, which asks the system and
  hands them over as numbers (`face_app_health`, `Native.health`, `devicelive::told`). What the device is, is
  text (`facts`, and the page's own line for its camera and for where the models run; a phone's model, cores
  and memory are its system's own answers, `devicelive::phone`): the chart keeps only the lines it has names
  for, each a line long (`cleanFacts`, `FACTS`), and puts them in as text. A reading a device doesn't give has
  no chart, and a device that says nothing of itself no Device section (`Panel.itself`). A new reading
  of the device goes into `HOST`, into the tracker page's sample and into the test that holds the two to each
  other; it travels to the other devices with the readout like the rest (`pack`, `unpack`: the newest once).
- The Signals chart (`web/signals_chart.js` and `.css`, on the tracker page and under each camera of the Cameras
  page; the owner's design, 2026-11-22: the mock-up they chose, "exactly like" it, and "the fastest signal polling
  we can get from the devices we have") keeps its readings in the page's memory only, 15 minutes: never store
  them, in the browser or anywhere. A reading is taken with every check of the picture (`chartSample`), and goes
  on while the drawer is closed or the tab hidden: only the drawing may be skipped. Another device's readings come
  with its readout (`signals` in `/api/inview`, passed on as they are up to `camstream::MAX_DEVICE_SIGNALS`; a
  device that sends none answers as it always did), every one with its own time (`pack`, `unpack`: the last 2.5 s
  with each readout, so one lost on the way loses nothing), and every value is checked where it is kept (`clean`);
  a name or a word from another device is only ever put into the page as text. A new signal goes into the
  module's section list, into `Signals.numbers` (or the tracker page's `faceNumbers`), and into
  `web/tests/signals_chart.test.js`, which holds the three to each other. Its thresholds for "unusual signals"
  (`ODD_SIGNALS`, `ODD_FOR_MS`) and the sound's 4-second stretches (`soundlive::LISTEN_FOR`) are choices, not
  measured values: evidence only, like the page's other signals.
- Cameras are never paused (the owner's rule, 2026-11-12): the app never stops or slows a camera or its stream by
  itself, whether or not a face is in view, the window is shown, or anyone is watching. Only the owner's own
  Stop camera does. The Cameras page keeps fetching every stream while its tab is hidden; don't add a pause.
  ("Paused" on a page means the system stopped the camera's picture, a locked screen on Windows: the app says
  so, it never does it.) Never lock a screen to tidy up: on Windows that stops the camera.
  A busy page counts as the app slowing a stream: this camera's frames for the other devices keep their 20 a
  second whatever the page is reading. The page's worker makes and sends each frame (`tracker_ui.js`
  `streamByWorker`, `jpeg_worker.js` `send`), and the page only hands the picture over: sent from the page,
  a frame waited four times for the page's thread, and with a face in view and MediaPipe's five parts in the
  page the stream fell to 11.5 frames a second in Chromium's window (measured 2026-11-23; 20 since) and to
  15.3 in WebKit's on a Mac (2026-11-24; 19.3 by the worker). An iPhone keeps the page's way, at its
  server's word (`stream` in `/api/people`, `tracker.rs` `stream_way`): on an iPhone 12 Pro Max the
  worker's way gave about a tenth more frames and cost checks, up to half of them with a face in view.
  `FACE_STREAM` names either way, to compare them on a device. The worker has one address, the page's own
  `/api/stream/frame`, and a test holds it to that: never give it another. Change any of this only with the
  measurement: a test copy with a made-up face as its camera, the browser named as the app's window is (a
  headless browser's own name switches the worker off); for WebKit a test copy of the Mac's program with page
  files whose camera is a canvas of a generated face, since no WebKit window has a made-up camera (the
  scripts are kept with the project's backup). A phone can only be measured by what its camera happens to
  see, so say with each number whether a face was in view.
- **They move between cameras** (and so camera sightings) is off until switched on.
- Times are stored in UTC everywhere (files, file names, log lines, the database: `face_files::paths` `utc_now`, `utc_stamp`,
  timestamptz with the session in UTC) and shown in local time only on the pages; a time written as local text
  by an old file is still read.
- Describe photo is local by default. Pointing it at another device (Settings) sends each described photo there; the
  Settings tab must say so (`face_home::describe::where_is`). Descriptions and `describe.json` live in `HOME`.
- Releases: the four installers (`FaceRecognizer-Setup-x64.exe`, `FaceRecognizer-macos-arm64.dmg`,
  `FaceRecognizer-linux-x64.run`, `FaceRecognizer-raspberrypi-arm64.run`), `FaceRecognizer-win64.zip`
  (`FaceRecognizer.exe` and a new-user `README.txt`), `FaceRecognizer-macos-arm64.zip`,
  `FaceRecognizer-linux-x64.tar.gz`, `FaceRecognizer-raspberrypi-arm64.tar.gz` and `SHA256SUMS.txt`, from one CI
  run's builds of the tagged commit; never `data\`. An installer never deletes `models` or `data`, on install, update
  or uninstall. The macOS app and its .dmg are signed and notarized by CI (with the
  repository's Apple secrets): release only a run whose macOS build says so (`BUILD-INFO.json` `signed`). The
  Windows .exe and installer are code-signed by CI on pushes to `main` (the Azure signing secrets are set since
  2026-11-17): release only a run whose Windows build says `signed` too, and say in the notes what SmartScreen
  still does (it warns about a new publisher until it has seen enough installs).
- The presence log (who was in view, when) is opt-in, names and times only, in `HOME/presence/`, never `data/`.
- Webcam faces named automatically don't enter the recognition gallery until confirmed (`labels.rs` `gallery`).
- Ignore deletes the face's print and picture (`labels.rs` `forget_faces`; Undo restores them from memory
  in the same session); only its ID, photo and box stay, which keep the photo from being read again. Code that
  uses prints must skip `printless` faces.
  An Ignored face that is given a name or taken off the Ignored list gets both made again from its photo
  (`labels.rs` `act`, the importer's `remake`; Undo deletes them again); when the photo isn't on the computer the face stays as it is
  and the page says so. Only an Ignored face may be without a print (`lost_prints`, `mend`: a face found
  otherwise is mended at the labeling tool's start).
- Collect faces never saves whole frames, only each face with room around it and the others greyed out.
- The Raspberry Pi has its own code flow (the owner's rule, 2026-11-20): nothing changes how the other hardware
  works to make the Pi easier. What is done for the Pi is done only there (64-bit ARM Linux), in functions of its
  own that the other systems never enter, and a test says so for each. Its release files are its own artifact
  (`FaceRecognizer-raspberrypi-arm64.*`), like each other system's. Measure on the Pi before choosing anything
  for it: a Pi 4 took 0.55 s to recognize one face with the recognizer of the time (2026-11-20). The Pi's own way is
  asked for by `face_core::behind::raspberry_pi` (prints made behind the frames, one worker for MediaPipe's parts).
- The Linux and Raspberry Pi installers start the app at sign-in only when asked (`--autostart`: an entry in the
  desktop's autostart folder, which a later update keeps in step and `uninstall.sh` removes). A Raspberry Pi's
  desktop signs in by itself at boot, so there that is "start at boot".
- Every model the app comes with is licensed for selling it, and `NOTICE` names each. Two are the project's
  own (the owner's names for them, 2026-11-22). **Opsis 1** (`opsis_1`) is the recognizer: trained by the
  project from OpenCV Zoo's SFace, Apache 2.0, to give a poor picture of a face the print of a good one.
  **Melos 1** (`melos_1.onnx`) is the voice model (arousal, dominance, valence): trained by the project on what
  listeners heard in CREMA-D's clips (Open Database License 1.0, named in `NOTICE` as that licence asks), and
  measured on acted speech only: evidence, as every voice reading is. The training tools and their data are
  kept outside this tree, and a library's faces never leave their owner's computers: only a model's weights do. Before a model goes in, its
  licence AND the terms of the data it was trained on are read: a permissive tag on weights whose card says "follow
  the training data's licence" is not enough. The phones' apps carry both models.
- The project is a commercial, proprietary product with public source (the owner's decisions: 2026-11-22, "I
  want to charge for the app in all the app stores"; 2026-11-23, "this is now all commercial"). It is under the project's own licence (`LICENSE`, the Singularity
  Research Commercial License, which covers version 2.4.0 and every later one): the source may be read and the
  published apps used; everything else, building included, needs a written licence, and nothing becomes open
  source later. The licence is a legal document of the owner's: change a word of it only at their word, and
  have it say only what is true. Each version of its text is dated the day it was made (Version 1.1, 24
  November 2026) and is never given an earlier date. The documents and the source's own comments speak of the
  product as it is (the owner's rule, 2026-11-24): they name no version before 2.4.0 and no terms but the
  licence's own, and a comment says what the code does and why in its own terms (a file's bytes are "fixed", a
  number is "measured"). Commit messages keep to the same, and say what changed in general words. Never call the project "open source" ("the source is public" is what is true), never copy
  code in that is under a licence which would bind the whole (GPL, AGPL), and take no outside contribution
  without its author's agreement (`CONTRIBUTING.md`). `licenses/Apache-2.0.txt` is the licence text of other
  people's parts named in `NOTICE`, not a licence of this project. What the app may be sold with has to be
  licensed for that: a model's licence is checked before the model goes in (`NOTICE`). The README's "What is
  new here" names what the owner regards as the project's inventions and reserves the patent rights: never
  write "patent pending" or name a patent unless the owner says one is filed.
