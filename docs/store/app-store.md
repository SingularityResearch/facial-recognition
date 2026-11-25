# Apple's App Store: the listing and App Store Connect's forms

What goes into App Store Connect for the iPhone app, so that every release says the same. The app's bundle id is
`io.github.singularityresearch.facerecognizer` (the Android app's and the Mac app's id too); its version and
build number are the workspace's (`apps/ios/project.sh`: 2.7.0 is build 20700). It is an iPhone app; an iPad runs
it as one.

## Building and sending it (on the Mac, in the desktop session)

```
sh apps/ios/project.sh          # the Xcode project: id, name, version, privacy manifest
sh apps/ios/store.sh            # archive, carry the pages and models, sign for the store: FaceApp.ipa
sh apps/ios/store.sh upload     # the same, sent to App Store Connect (it appears under TestFlight)
```

Signing for the store is Xcode's automatic signing, which needs the developer's Apple Account in Xcode on that Mac
(Xcode → Settings → Accounts), once: Xcode then registers the bundle id and uses Apple's cloud-managed
distribution certificate. No certificate or key is kept in the repository or in CI for it. An upload needs the
app's record in App Store Connect to exist.

What the package holds that Apple checks at upload:

- **The privacy manifest** (`apps/ios/PrivacyInfo.xcprivacy`): nothing collected, nothing tracked; one reason
  API, file sizes and times of the app's own files (`stat`, `fstat`, `lstat`: C617.1). It was read off the built
  program (`nm -u`): look again when a library is added.
- **Why it wants the camera, the microphone, the local network and the location**: the four sentences in
  `project.sh`. The location is asked for only when the owner switches **Show and keep this device's location**
  on, and only while the app is in use (never in the background). In the App Privacy answers it is not
  "collected": it goes to the owner's own devices and database, never to the developer or a third party.
- **Built with the current Xcode and iOS SDK** (Apple names a minimum each year).

## The app's record (App Store Connect → Apps → +)

| Field | Value |
|---|---|
| Platform | iOS |
| Name | Singularity Face Recognizer (the listing's name in every store; a name is given once in the whole store, so have a second one ready: Face Recognizer Local) |
| Primary language | English (U.S.) |
| Bundle ID | `io.github.singularityresearch.facerecognizer` |
| SKU | `facerecognizer-ios` |
| Access | Full access |

## The listing

**Subtitle** (30 characters at most): Names the faces a camera sees

**Promotional text** (170 at most):

> Names the people your camera sees, on your own devices. No account, no cloud: the camera's picture never leaves
> the phone.

**Description** (4000 at most): the Google Play description (`google-play.md`), with "Android 8 or later and a
64-bit ARM processor" replaced by "an iPhone with iOS 16 or later". It carries the words Apple wants of an app
that reads a pulse: not a medical device, and to ask a healthcare professional.

**Keywords** (100 characters at most, commas, no spaces): `face,recognition,camera,privacy,local,offline,people,names,signals,posture`

**Support URL**: `https://github.com/SingularityResearch/facial-recognition/issues` (contact: chris@singularityresearch.net)
**Marketing URL**: `https://github.com/SingularityResearch/facial-recognition`
**Privacy Policy URL**: `https://github.com/SingularityResearch/facial-recognition/blob/main/PRIVACY.md`
**Category**: Utilities. **Price**: free. **Copyright**: the year and the developer's name.

**Screenshots**: the 6.9-inch iPhone size (1320 × 2868), in `assets/store/screenshots/iphone/`. The people in
them are made up: generated faces with invented names, nobody's likeness. The App Store's own picture is the
app's icon (`apps/ios/Assets.xcassets`).

## App Privacy, age rating, export, review

| Form | Answer | Why |
|---|---|---|
| App Privacy | Data Not Collected | The app sends its developer nothing. What goes to the owner's own database and devices is not collected by the developer: that holds for the signal history too (**Keep a history of every signal**, off for a new copy), which keeps readings, the pulse estimate and breathing rate among them, in the owner's own database only. |
| Age rating | No to every content question; "Medical/Treatment Information": none | A utility. |
| Health | The pulse and breathing readings are estimates, the app says it is not a medical device (on its page, in the policy and in the description) | Guideline 1.4.1 looks hard at apps that measure the body with the phone's own sensors. |
| Export compliance | **The owner's answer** | The app brings its own encryption beside the system's: TLS for the database and between devices, and AES-256-GCM with scrypt for keys. All of it is standard, published algorithms, and the source is public. Apple asks at each upload unless `ITSAppUsesNonExemptEncryption` is in the Info.plist; put it there once the answer is settled. |
| Sign-in for the reviewer | None needed | No account. |

**Notes for the reviewer** (App Review Information):

> Face Recognizer needs no account. Open it and allow the camera: it finds the faces in view at once and shows
> its readings. Naming people is done on the desktop app (Windows, macOS, Linux; free from the project's page),
> where the user adds their own photos; a phone takes the names from a database the user hosts themselves, joined
> by scanning a code. A reviewer does not need that to see the app work: without it every face is shown as
> "Unknown". Nothing is sent to us or to any service: the app has no server of ours. The privacy policy is in the
> app (Controls → Privacy policy) and at the address given.
> The optional pulse and breathing readings are estimates from the camera's picture; the app, its description and
> its policy say that it is not a medical device.

## Open

- **The owner's Apple Account in Xcode on the Mac** (or an App Store Connect API key with the Admin role): without
  one of them Xcode can't sign for the store ("No Accounts").
- **Export compliance**: the answer above.
- **The iPhone that runs the trial app**: the store app has another id than the trial one, so to the phone it is
  another app. Its place in the shared database is moved once, from the trial app to a development build with the
  permanent id; a TestFlight build then installs over that and keeps it.
- **The Mac App Store** is a different job (a sandboxed app that downloads nothing that runs): the Mac app stays
  a signed and notarized download.
- **The models' licences** (NOTICE): every model the app comes with is licensed for selling it (the
  face recognizer is the project's own), so the app may be priced.
