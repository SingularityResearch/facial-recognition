# Microsoft Store: the listing and Partner Center's forms

What goes into Partner Center for the Windows app, so that every release says the same. The app is listed as an
**EXE app** (Microsoft's "MSI or EXE app": the Store downloads the project's own signed installer from an address
and runs it silently). An MSIX package is not used: it could not keep the models and the library beside the
program, where every installed copy has them.

## The package (Partner Center → Packages)

| Field | Value |
|---|---|
| Package URL | `https://downloads.singularityresearch.net/facerecognizer/v<version>/FaceRecognizer-Setup-x64.exe` |
| Architecture | x64 |
| App type | EXE |
| Installer parameters | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /OFFLINE=1` |
| Languages | English (United States) |

The Store takes only an address that answers with the file itself, and GitHub answers a release's download address
with a redirect. So the package address is a small Cloudflare Worker on the owner's domain
(`packaging/windows/store-address`): asked for a version's installer, it fetches that release's file from GitHub and
hands it on unchanged. Nothing is uploaded at a release: the address for a new version works as soon as the
release is published.

Installer handling (Inno Setup's exit codes; the page that documents them:
`https://jrsoftware.org/ishelp/index.php?topic=setupexitcodes`):

| Outcome | Code |
|---|---|
| Installation successful | 0 |
| Installation cancelled by user | 2, 5 |
| Reboot required | 8 |
| Any other failure | 1, 3, 4, 6, 7 |

What the Store asks of the installer, and how it is met:

- **Every program and library in it is signed** with a certificate that leads to a root of Microsoft's Trusted
  Root Program: CI signs the program, the uninstaller, Setup, and the project's own builds in `runtime\` (FFmpeg,
  the HEIC libraries); Microsoft's own files there are signed by Microsoft. `packaging/windows/build.ps1 -Sign`
  stops if one of them carries no valid signature. Only a build from `main` is signed: submit only a release.
- **Silent, and no download while it runs**: `/OFFLINE=1` keeps Setup from fetching Microsoft's WebView2 runtime
  on a computer without it (Windows 11 has it; without it the app opens in the web browser and says so).
- **A versioned address whose file never changes**: the Worker's address for a release, which gives that
  release's file from GitHub. **A release the Store points to must stay on GitHub** until a later submission is
  published: the release routine deletes the release before the newest, so check Partner Center's package URL
  before deleting one.
- **An update** is a new submission with the new release's address. The Store does not update an EXE app by
  itself; the app's own **Check for updates** tells the user.

After the install the app asks before it downloads the face models (79 MB, each file checked by size and
SHA-256): say so in the description, so that nobody is surprised by it.

## Store listing

**Product name**: Singularity Face Recognizer (reserved in Partner Center, 2026-11-22; "Face Recognizer" alone was
taken there). It is the listing's name in every store; the installed app is still called Face Recognizer.

**Short description**:

> Names the people your cameras see and the faces in your photo library, on your own computers. No cloud, no
> account.

**Description** (10,000 characters at most):

> Face Recognizer learns specific people from your own photo library and names them, live, on any camera, with how
> sure each match is. Everything is worked out on your own computer: no account, no cloud service, no analytics.
>
> WHAT IT DOES
> • Face tracking: names on the faces a camera sees, with match confidence; alerts when someone arrives or leaves;
>   optional readings of expression, posture, voice and the room's sound, as numbers only. Signals charts every
>   reading of the last minutes.
> • Labeling: add photos and videos, then name the faces found in them or ignore them, with clues (closest
>   people, photo tags, same occasion) and Undo.
> • Cameras: every one of your devices' live cameras in one grid, with what each sees.
> • Describe photo: a vision model on your own computer describes a photo and reads its text.
> • Several devices as one (optional): your devices share names and labels through a database you host yourself,
>   and show each other their cameras and original photos over your local network, encrypted.
>
> PRIVATE BY DESIGN
> • The camera's picture and the microphone's sound are measured in memory and never recorded.
> • Nothing leaves your computer unless you send it on purpose; the privacy policy lists every case.
> • Every person's data can be exported or forgotten.
>
> GOOD TO KNOW
> • At its first start the app asks before it downloads its face models (79 MB), each file checked before use.
> • The app also runs on macOS, Linux, the Raspberry Pi, Android and iPhone.
> • A face print is biometric data: add only people who know and agree, and follow the law where you live.
>
> NOT A MEDICAL DEVICE
> The optional pulse and breathing readings are estimates from the camera's picture, for your own information.
> Face Recognizer is not a medical device and does not diagnose, treat, cure or prevent any medical condition.
> For medical advice, diagnosis or treatment, ask a healthcare professional.
>
> The source code is public, so what the app does can be checked.

**Product features** (each 200 characters at most):

- Names the people a camera sees, live, with match confidence
- Learns people from your own photos and videos; name or ignore every face found
- Runs on your own computer: no account, no cloud, no analytics
- Signals: every sensor reading of the last minutes as charts
- All your devices' cameras in one grid, over your own network
- Export or forget everything about any one person

**Keywords** (7 terms, 21 words at most): face recognition, photo library, camera, privacy, local, offline,
people

**Applicable license terms**: Licensed by Singularity Research under the Singularity Research Commercial
License (`https://github.com/SingularityResearch/facial-recognition/blob/main/LICENSE`).
The models the app downloads that are other people's have licences of their own:
`https://github.com/SingularityResearch/facial-recognition/blob/main/NOTICE`. (This field and the description's
last line are set in Partner Center with the package they belong to: check both whenever a package is
submitted.)

**Privacy policy URL**: `https://github.com/SingularityResearch/facial-recognition/blob/main/PRIVACY.md`

**Website**: `https://github.com/SingularityResearch/facial-recognition`
**Support contact**: chris@singularityresearch.net

**Graphics**: the 1:1 box art (`assets/store/microsoft-store-boxart-1080.png`) and the 2:3 poster
(`assets/store/microsoft-store-poster-720x1080.png`), both the app's one drawing; the screenshots in
`assets/store/screenshots/windows/`. The people in the screenshots are made up: generated faces with invented
names, nobody's likeness.

## Properties, age rating, pricing

| Form | Answer |
|---|---|
| Category | Photo & video; second category Health & fitness (the owner's choice in Partner Center) |
| Privacy policy | given as text: `PRIVACY.md` (paste it anew when it changes) |
| System requirements | Camera: recommended. Microphone: recommended. Memory: 8 GB recommended. Windows 10 or 11, 64-bit. |
| Age rating (the questionnaire) | A utility: no to every content question |
| Pricing | Free |
| Markets | All the Store offers |

## Open

- **Individual or company account.** Microsoft's individual accounts are for apps that are not part of a
  business; a commercial product belongs under a company account, which can't be made from an individual one.
- **The models' licences** (NOTICE): every model the app comes with is licensed for selling it (the
  face recognizer is the project's own), so the app may be priced.
