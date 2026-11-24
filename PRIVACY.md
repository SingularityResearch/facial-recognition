# Privacy policy

**Face Recognizer**, by Singularity Research. In effect since 24 November 2026. It covers the app on Windows,
macOS, Linux, the Raspberry Pi, Android and iPhone.

## The short version

- Face Recognizer runs on your own devices. **We, its developer, receive nothing from it**: there is no account,
  no sign-in with us, no analytics, no advertising, no crash reports, and no server of ours that the app talks to.
- What the app reads and learns (camera pictures, faces, names, sound measurements) stays on the device, or goes
  to a database and to devices that **you** set up and control.
- Something leaves your devices for somebody else's computer only when you send it on purpose, with a click, and
  only to the services listed [below](#what-leaves-a-device-and-only-when-you-say-so).
- You can export or delete everything the app keeps about any one person.

## What the app handles

| What | What the app does with it | Where it is kept |
|---|---|---|
| **Camera** | Finds faces in the live picture and names the ones you have named. Optionally reads what a face looks like (expression), head and body posture, and a pulse estimate from the picture. | The live picture is kept in memory only, never written to disk or to a database. Only if you start **Collect faces** is a picture of each face it sees saved for you to name (the face with some room around it, never the whole picture). |
| **Photos and videos you add** (computers) | Finds the faces in them so you can name or ignore them. | For each face: a face print (numbers that describe the face, a biometric identifier), a small picture of the face, where it is in the photo, and the name or label you give it. Your photos themselves stay where they are. |
| **Names and labels** | Shown on faces, in lists and in alerts. | On the device, in the app's data folder. |
| **Microphone and audio outputs**, only while you switch the option on | Measures the voice (pitch, loudness, pace) and the room's sound, and says in one word what it hears (speech, music, quiet). | The sound is measured in memory and never recorded or stored. Only numbers and that one word are the result. |
| **What a face looks like over time** (expression readings, each person's usual look) | Shows when a reading is unusual for that person. These are readings of how a face looks, not of what a person feels. | Each person's usual values on the device. Readings over time only if you set up a shared database and leave **Keep expression readings** on. If you switch on the feeling check, your answers to it (numbers) are kept on the device. |
| **Who was in view, and when** | A log of names and times, and alerts you ask for (someone arrives, leaves, an unknown face stays). | The presence log is off until you switch it on. Alerts are kept on the device. |
| **Music that plays** (computers), only while you switch the log on | Notes the song's name and numbers that describe the sound. | On the device. |
| **Photo descriptions** | A vision model on your own computer describes a photo and reads its text. | On the device. |
| **Keys and passwords you enter** (for the services below, for your database) | Used to reach that service. | Protected on the device for your user account. |
| **Settings** | Remembered. | On the device. |

The phones' apps keep no photo library: they read the camera and, if you switch it on, the microphone. The
people a phone can name (their names and face prints) come from one of your own computers, or from your shared
database, and are held on the phone in memory only.

### Permissions on a phone

- **Camera**: to find and name the faces in its picture.
- **Microphone**: for the voice readings and the sound chart, only while you switch those options on.
- **Network**: for the app's own page on the phone itself, for your shared database, and for your other devices
  on your local network.

## Pulse, breathing and other body readings

Some of the optional readings are about the body: a pulse estimate from the camera's picture, a breathing rate
from posture, the voice, and what a face looks like. They are estimates, for your own information. **Face
Recognizer is not a medical device and does not diagnose, treat, cure or prevent any medical condition.** For
medical advice, diagnosis or treatment, ask a healthcare professional. These readings are handled like every
other reading: on the device, or in the places you set up yourself.

## Where your data is kept

On the device, in two folders: the app's data folder (face prints, face pictures, labels) and a private folder
for your user account (keys, settings, descriptions, logs). On a phone both are inside the app's own storage.

**Your own shared database (optional).** If you set one up, your devices share their faces, names, labels and the
other records above through a PostgreSQL database on a server of your own. The connection is encrypted (TLS,
checked against your server's own certificate authority). Keys you enter are stored there only encrypted with a
passphrase that stays on your devices. A device joins the database when you let it in from a device that is
already in it (one shows a code, the other scans it): the database's login and that passphrase then go from the
one device to the other over your local network, encrypted. We have no access to that database: it is yours, and you are the one who
runs and secures it.

**Your own devices on your local network (optional).** With a shared database, your devices can show each other
their original photos and their live cameras with the readings that belong to them, the device's own among them
(how hard it works, how warm it is, its battery, and what kind of device it is). A device also tells your other
devices what it has and how it is set: which sensors it has, which are switched on, and the settings of its
Face tracking tab (never a key or a password). Who is in view of a camera, and the readings taken there, go to
your other devices only while that camera is streamed to them. This goes directly from device to device, is checked on arrival,
and is kept in memory only. This goes from device to
device over your local network, encrypted, and only between devices of your database. Live pictures are kept in
memory only. On each of your devices you can remove another one for that device alone: it then shows nothing of
it and gives it nothing directly, until you add it back.

## What leaves a device, and only when you say so

Nothing is sent by itself except the first two rows. Each service gets only what its row says, and what it does
with that is under its own privacy policy.

| When | What is sent | To whom |
|---|---|---|
| The first start on a computer | A download of the face models. No data of yours; as with any download, the server sees the device's internet address. The phones' apps carry their models and download nothing. | GitHub and the models' publishers |
| You click **Check for updates** | A question for the latest version. No data of yours. | GitHub |
| You click **Web search** on a face (off until you enter your own keys) | That one face, and the photo it is in. | Amazon (Rekognition) and Google (Cloud Vision), under your own accounts with them |
| You point **Describe photo** at a server on another computer | Each photo that is described. | That server, which you chose |
| You connect **Spotify** | A question for your liked songs. | Spotify, under your own account |
| You click a download in Settings (a describe model) | A download. No data of yours. | Hugging Face, LM Studio's sources |
| You set up a **shared database** | The records described above. | Your own server |

[docs/SETTINGS.md](docs/SETTINGS.md#what-reaches-the-network) lists every setting that reaches the network in
full detail.

We do not sell data, share it with advertisers or data brokers, or use it to train models: we never have it.

## Other people in your pictures

The app recognizes people other than you: the people in your photos and in front of your cameras. A face print
is biometric data, and many places have laws about collecting it (in the United States, for example, Illinois,
Texas and Washington; in Europe, the GDPR). **You decide whose faces the app learns, and you are responsible for
having the right to do so**: tell the people concerned, get their consent where the law asks for it, and do not
use the app to identify people who have not agreed to it, to watch people where they expect privacy, or for
anything the law forbids.

The app helps with this:

- **Ignore** deletes a face's print and its picture.
- **Export** gives you everything the app keeps about one person, as files.
- **Forget** deletes everything the app keeps about one person: on the device, and with a shared database on
  every device of yours.

## How long data is kept, and deleting it

Data stays until you delete it: nothing expires by itself, except that camera pictures and sound are never kept
at all, and that the Signals chart's readings are held in memory for 15 minutes and then dropped.

- **One person**: Forget (above).
- **Everything on a computer**: delete the app's data folder and its private folder. Uninstalling the app does
  **not** delete them, on purpose, so an update or a reinstall never loses your library: the Settings tab opens
  both folders.
- **Everything on a phone**: uninstall the app, or clear its storage in the phone's settings.
- **The shared database**: it is on your own server. Forget removes a person from it; removing the database
  removes everything.

Because we hold none of your data, there is nothing we could delete for you, and nothing we could hand to anyone
else.

## Security

The app's own pages are served to the device itself only (127.0.0.1) and answer no other program or website.
Keys and passwords are stored protected for your user account. Everything the app downloads is checked against a
known size and SHA-256 before it is used. [SECURITY.md](SECURITY.md) says more, and how to report a problem.

## Children

Face Recognizer is not directed at children under 13, and we collect no data from anyone, children included.

## Changes

A change to this policy is made in this file, with a new date at the top. Its
[history](../../commits/main/PRIVACY.md) shows every earlier version.

## Contact

Questions about privacy: write to chris@singularityresearch.net, or [open an issue](../../issues) in the project's repository
(please put no personal data into a public issue). Security problems: privately, as [SECURITY.md](SECURITY.md)
describes.
