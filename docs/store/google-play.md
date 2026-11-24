# Google Play: the listing and the console's forms

What goes into Play Console for the Android app, so that every release says the same. The package is
`io.github.singularityresearch.facerecognizer`; it is uploaded as the app bundle CI builds
(`FaceRecognizer-android-bundle`, or `apps/android/build.ps1 -Release -Bundle`).

## Store listing

**App name** (30 characters at most): Singularity Face Recognizer (the listing's name in every store; the installed
app is called Face Recognizer)

**Short description** (80 at most):

> Names the faces your camera sees. Runs on your own devices, with no cloud.

**Full description** (4000 at most):

> Face Recognizer names the people your camera sees, live, and shows how sure each match is. Everything is
> worked out on the phone itself: the camera's picture is never stored and never sent to us or to any cloud
> service.
>
> This app is the phone's part of Face Recognizer, which also runs on Windows, macOS, Linux and the Raspberry Pi
> (free, from the project's page). On a computer you add your photos and name the people in them; your devices
> then share those names through a database you host yourself, and the phone joins it by scanning a code that a
> computer shows. Without that, the phone still finds every face in view and shows its readings, but has nobody
> to name.
>
> WHAT IT DOES
> • Names on faces, live, with match confidence, near the top of each head.
> • What a face looks like: expression, read from sharp, camera-facing frames. It says "looks happy", never
>   "feels".
> • Optional readings, each behind its own switch: head and body posture, hands near the face, voice (pitch,
>   loudness, pace) and the room's sound in one word (speech, music, quiet).
> • Signals: every reading of the last minutes as charts, one expander a section, with each person's own usual
>   range.
> • Your other devices can show this phone's camera and its readings in their Cameras tab, over your local
>   network, encrypted, and only between your own devices.
>
> PRIVATE BY DESIGN
> • No account, no sign-in, no ads, no analytics, no tracking.
> • The models are inside the app: nothing is downloaded when it starts.
> • The camera's picture and the microphone's sound are measured in memory and never recorded.
> • The only places data goes are ones you set up yourself: your own database, and your own devices.
> • Every person's data can be exported or forgotten, from any computer of yours.
>
> PLEASE USE IT FAIRLY
> A face print is biometric data. Add only people who know and agree, and follow the law where you live.
>
> NOT A MEDICAL DEVICE
> The optional pulse and breathing readings are estimates from the camera's picture, for your own information.
> Face Recognizer is not a medical device and does not diagnose, treat, cure or prevent any medical condition.
> For medical advice, diagnosis or treatment, ask a healthcare professional. The readings need a phone with a
> camera, Android 8 or later and a 64-bit ARM processor; the pulse estimate needs a still face in steady light.
>
> The source code is public, so what the app does can be checked. Source code, the desktop apps and the full
> privacy policy are on the project's page.

**Category**: Tools. **Tags**: none needed.

**Contact details**: chris@singularityresearch.net (the developer email, public) and, as website, the repository's address.

**Privacy policy**: `https://github.com/SingularityResearch/facial-recognition/blob/main/PRIVACY.md`

**Graphics**: the icon (`assets/store/google-play-icon-512.png`) and the feature graphic
(`assets/store/google-play-feature-1024x500.png`), both the app's one drawing; the phone screenshots in
`assets/store/screenshots/android/`. The people in the screenshots are made up: generated faces with invented
names, nobody's likeness.

## App content (the console's forms)

| Form | Answer | Why |
|---|---|---|
| Privacy policy | the address above | |
| App access | All functionality is available without special access | No sign-in. Note for the reviewer: "The app opens on the camera and finds faces at once. Naming people needs the desktop app and a database of your own, which a reviewer does not need to see the app work." |
| Ads | No ads | |
| Content rating | Utility / productivity; no to every content question | |
| Target audience | 18 and over | Not directed at children; keeps the app out of the Families rules. |
| News app | No | |
| Data safety: data collected | None | "Collected" means sent off the device to the developer or a third party. The app sends us nothing. What goes to the owner's own database and devices goes to places they chose and control, not to the developer. |
| Data safety: data shared | None | Nothing goes to a third party. |
| Data safety: encrypted in transit | Yes | TLS to the database, TLS between devices. |
| Data safety: deletion | Not applicable (no account, nothing held by the developer) | PRIVACY.md says how a user deletes their own data. |
| Government app | No | |
| Financial features | None | |
| Health | The app has health features: Activity and Fitness (body vitals: a pulse estimate and a breathing rate from the camera). Not a regulated medical device. | See below. |

Permissions the package asks for: `CAMERA`, `RECORD_AUDIO`, `MODIFY_AUDIO_SETTINGS`, `INTERNET`. None of them is
one of Play's restricted permissions, so there is no permission declaration form to fill in.

### The body readings and Play's Health rules

The owner's decision (2026-11-22): the phone app keeps every sensor reading, so the app is declared as one with
health features. **Webcam pulse** (off until switched on) and the breathing rate that **Body posture** gives are
body vitals in Play's sense. What that asks of the app, and where each is:

- The words "not a medical device and does not diagnose, treat, cure or prevent any medical condition", with
  the reminder to ask a healthcare professional: in the listing's description (above), on the tracker page under
  the options (`#medicalHint`), and in `PRIVACY.md`. A page test holds the three to the same words.
- A privacy policy inside the app: the tracker page's **Privacy policy** link shows `PRIVACY.md` as the app's own
  page (`/privacy`), without the internet.
- Which devices the sensor-based readings work on: in the description (above).

Pick the category with the form in front of you: its list has changed more than once, and "Activity and Fitness"
is the one that names heart rate today.

## Releases

1. Internal testing: upload the bundle, add the testers' Google accounts (up to 100; works at once).
2. Closed testing: at least 12 testers opted in for 14 days in a row (a personal account's requirement), then
   apply for production access.
3. Production.

The version code is the workspace's version (2.5.2 is 20502): Play takes each code once, so every upload needs a
new version in `Cargo.toml`.
