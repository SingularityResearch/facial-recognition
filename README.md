# Facial Recognition and Emotional Response Research

<p align="center">
  <img src="face-recognizer/icons/icon.png" width="320" alt="The Face Recognizer icon: a faceted face, half solid and half wire mesh, inside the corner marks of a camera's viewfinder">
</p>

A fully local system that recognizes specific people on webcams (any camera the computer sees, including a
phone used as a webcam) and reads facial expression, voice and other signals live. Imported photos and videos
are recognized the same way: the same face prints and matching, with faces named in the labeling tool.

The research is about **emotional response**: how a recognized person responds to the people, music and events
around them. It is original research: it reads each recognized person against their own normal, across face,
voice, body and several cameras, and measures what each signal is actually worth on that person before letting
it influence anything. So far every one of those signals is taken from outside the person. The next channel is
taken from inside: brain activity from an EEG headset, recorded on the same clock as everything the cameras and
microphones measure, so that what a face shows can be compared with how the person's brain responded at that
moment (see [EEG](#eeg-the-response-from-inside-next)).

Everything runs on your own computers: Windows, macOS (14+, Apple Silicon), Linux (Ubuntu 22.04+) or, since
1.26, a Raspberry Pi (a Pi 4 works, slowly; 64-bit Raspberry Pi OS 13). One computer
is enough; it identifies its processor, graphics processors and NPU at start and runs each job the fastest way that
gives the same answers (Settings, **This computer**). No photos, video, audio or face data leave the machine unless
you send them on purpose:
- to a **shared database** of your own, if you set one up so several of your devices share names, labels and the
  rest (PostgreSQL with pgvector on a computer of yours, over TLS; optional, off for a new copy). With one, your
  devices also show each other original photos and their live cameras (the **Cameras** tab) directly over your
  local network: encrypted, signed so only your devices can ask, and never stored;
- the labeling tool's **Web search** (one face or photo, when you click it);
- **Describe photo**, if you point it at a model server on another device.

## Goals

1. **Recognize the people who matter, live, on every device.** A cluster of your own devices learns specific
   people together, from the photos, videos and camera pictures each of them holds, and shows each person's
   name and match confidence on their face on any camera stream in the cluster.
2. **Measure the emotional response, not only its display.** An emotional response has parts that can each be
   measured: what shows (face, voice, body), what the body does (pulse, breathing), what the brain does (EEG,
   next) and what the person reports. Record them for the same person at the same moment, and find out how they
   move together for that person.
3. **Read expression honestly.** Report what a face *looks like* ("this face currently looks happy"),
   never claim to know what someone *feels*, and say how sure the reading is.
4. **Find out what actually works.** Treat every new signal (face shape, motion, talking, voice, pulse, and
   EEG when it arrives) as a hypothesis. Measure it on the person's own data against ground truth before it
   may change a reading.

## What's in the repo

| Part | What it does | Start it |
|---|---|---|
| [face-recognizer/](face-recognizer/README.md) | The Python project: photo import, face recognition, labeling, live tracker, measurement tools | `face-recognizer\setup.bat` once (macOS, Linux: `./setup.sh`), then the two apps below |
| FaceRecognizer app (`app.py`) | Both apps in one window, with **Face tracking**, **Labeling**, **Cameras** (every device's live camera, with a shared database) and **Settings** tabs and the **Alerts** bell; downloads the models on first start. `FaceRecognizer.exe` (Windows), `FaceRecognizer.app` (macOS), `FaceRecognizer` (Linux) | Download from Releases, or build it: [Building the app](face-recognizer/README.md#building-the-app-windows-macos-linux) (`build_app.bat` on Windows, `build_exe.py` on macOS and Linux); from the code, `./start.sh` on macOS and Linux |
| Labeling tool (`label_tool.py`) | Add photos and videos, confirm suggested names, name new people, fix mistakes; clues for unknown faces (closest people, photo tags, same occasion), one-click **Web search** and **Describe photo** (a local vision model; **Describe all** and, from Settings, **Describe everything again** across every device's photos and video frames, shared out among your computers' LM Studio servers) | `face-recognizer\label.bat` (macOS, Linux: `./label.sh`) → http://127.0.0.1:8765 |
| Live tracker (`tracker.py`) | Names, match confidence, expression and other signals on each face | `face-recognizer\recognize.bat` (macOS, Linux: `./recognize.sh`) → http://127.0.0.1:8766 |
| Shared database (`facedb.py`, `dbsync.py`, `peer.py`; optional) | Several devices share names, labels, faces and private data through your own PostgreSQL + pgvector server; original photos and live cameras go device to device | Set up the server once ([Sharing between your devices](face-recognizer/README.md#sharing-between-your-devices-shared-database)), then Settings → **Shared database** → **Connect** on each device |
| Measurement (`tools/expr_eval.py`, `tools/eval_models.py`, `tools/hw_bench.py`) | Accuracy, calibration, talking, per-condition breakdowns, valence/arousal fit, model and hardware comparisons | `python tools\expr_eval.py` from `face-recognizer\` |
| [face-tracker/](face-tracker/README.md) | The original standalone browser face tracker (MediaPipe), no install | Double-click `index.html` |

## Key decisions, and why

| Decision | Why |
|---|---|
| **Local only**: small servers on 127.0.0.1 (the tracker, the labeling tool and the app window's tab page), with request checks (`localguard.py`) | Faces and voices are personal data. Nothing is uploaded, the pages load nothing from the internet (a Content-Security-Policy allows only their own server), and other websites can't reach the servers. The one exception to the internet rule is opt-in per click: the labeling tool's **Web search** sends that face to Amazon and that photo to Google (US services, English results) to suggest who it is. The one server that listens beyond the computer is the peer service below, and only with a shared database. |
| **Several devices, one shared database (optional)** | The owner uses more than one computer, and names given on one should reach the others within seconds. Each device keeps working from its own files (fast, and fine while the server is away); changes go both ways through PostgreSQL with pgvector (face prints are vectors) on a computer of the owner's. The connection is TLS checked against the server's own certificate authority, the app's database role is not a superuser, and keys are stored there only encrypted with a passphrase that stays on the devices. A name given by hand beats an automatic one, Ignored stays Ignored, and forgetting a person is carried out on every device. |
| **Devices talk to each other directly, for photos and cameras only** | Original photos stay on the device that has them, and live video doesn't belong in a database. So, with a shared database, each device runs a peer service on the local network: TLS with the device's own certificate (its fingerprint is in the database, so no other machine can pose as it), every request signed with a key only your devices can derive, photos asked for by content hash only, never by path. What it sends is shown and never stored: a photo, or the camera as its tracker page shows it (names and markers drawn on), with its readout. That also allows a "moved between cameras" alert when someone seen on one device's camera appears on another's. Off with one Settings switch. |
| **Times stored in UTC** | Devices sync with each other and can be in different time zones or cross a daylight-saving change. Every stored time (files, file names, the database) is UTC; pages show local time. |
| **Thresholds are measured values**, never guesses | Each matching, quality and voting threshold was measured on the owner's labeled photos, for example ArcFace match 0.35: 100% of labeled faces correct, 0% of strangers named. Changing one means re-measuring. |
| **ArcFace for recognition**, with SFace as a fallback; AdaFace evaluated and rejected | ArcFace named 76% of blurry webcam-like faces correctly vs 44% for SFace. AdaFace was slower and less accurate on these photos (95.5% vs 99.4%). |
| **The hardware decides, after a check** | Each model runs on the graphics processor (DirectML on Windows, CUDA on NVIDIA under Linux, CoreML on a Mac) only after a start-up check shows it gives the processor's answers (within 1%), faster (CUDA, CoreML and face finding everywhere; DirectML on Windows runs the other models as measured, without the check at each start); photos are read and decoded on several processor threads at once. Measured on a Ryzen 7 7840HS + RTX 4060: importing 412 files went from 89.5 s to 43.5 s, with the same faces found. The NPU isn't picked by itself: it was slower for one face at a time. |
| **LM Studio shared across your computers** | Describe photo needs a vision model; the owner runs LM Studio on more than one computer. On every computer with this app and LM Studio, the app keeps LM Studio serving on all network interfaces, announces it on the local network (mDNS, with its port), lists every LM Studio server it finds (names, addresses, ports, models), and gets missing vision models from a shared folder (a NAS: 17.7 GB in about 3.5 min over Wi-Fi) before falling back to the internet (about 2 MB/s from Hugging Face here). Downloads start only when asked, with a progress bar. |
| **Labels per face, not per cluster** | Cluster numbers change on every regrouping. A permanent ID per face keeps every decision, with undo history. |
| **"Looks", not "feels"** | Research shows facial expression is a weak, context-dependent indicator of felt emotion. The wording follows the evidence. |
| **Layered expression reading** | A quality gate, then voting over time, then a per-person "usual look", then evidence. Each layer fixes a specific failure of single-frame emotion detection. |
| **New signals are evidence only until measured** | Face-shape cues, talking, motion, blinks, gaze, pulse, body posture, voice and the state model are shown but don't change the label. The options that would change it (skip while talking, skip when a hand is on the face, state model) are off by default. |
| **Every signal is read against the person's own normal** | People differ more from each other than from their own good and bad days, so a population norm says little about one person (the idiographic view, Molenaar 2004). Each named person's usual level of every signal is learned, separately alone and in company, and the tracker shows departures ("head -2.3 SD for Sam"). Kept on this computer (by the tracker, with a copy in the page's browser storage) and in your shared database if you set one up; forgettable per person. |
| **Self-report is the only ground truth for feelings** | An opt-in "feeling check" asks how you feel on two 1–7 scales. Without it, accuracy can only be measured against posed expressions, never against feelings. |
| **EEG as the next channel: a second measurement, not a verdict** (planned) | Self-reports are few, and a face can be composed. EEG measures the response continuously and can't be posed the same way, so it gives every outward signal something to be tested against between reports. It is treated like every other signal here: per person, measured before it is trusted, and never called a feeling. |
| **Only numbers are stored for research** | Posed tests and feeling reports save scores and signals (including body points): no pictures, no audio. Live voice, pulse and posture are never written down. Expression readings are: with a shared database, every few seconds for each named person (the owner's choice, for reports later; a switch turns it off), and with each alert. Always as numbers, never a picture or sound. |
| **Two separate apps** (labeling and tracking) | Owner preference. Each stays simple, and both can run at once. Since 2026-11-08 FaceRecognizer.exe shows both in one window as tabs, but each keeps its own server and page. |
| **One Settings tab that owns no settings** | Each setting stays with the program it belongs to (the page's controls and storage, or its server). The Settings tab asks each page for its settings and sends changes back (`settings_bridge.js`), so the tab and the pages can't disagree and each page still works on its own. |
| **Describe photo: local by default, your choice of server** | A vision model in LM Studio on this computer, or a built-in one (Florence-2) when there's none. The server can be moved to another device; the Settings tab then says each described photo is sent there, and warns about unencrypted internet addresses. |
| **Clues before guesses for unknown faces** | A face seen once can't be learned from, so the labeling tool shows what else is known: the closest people (with their scores), the photo's own people tags (photo apps, Google Takeout), and who was photographed at the same occasion. None of these ever labels a face by itself. |
| **Web search: US services, English, capped, saved** | Owner's rules. Amazon Rekognition names well-known people from the face with a match percentage; Google Cloud Vision only helps when the same picture is online, since it doesn't identify faces. The tool combines them into one suggestion with a confidence level, or says it found none. Each service stops itself before $10 a month, keys stay outside the (public) repository, and each face's answer is saved so it is never paid for twice. |

The reasoning behind the layered design, with references: [docs/WHY_LAYERED_EXPRESSION.md](docs/WHY_LAYERED_EXPRESSION.md).

## Why this is useful research material

Most emotion-recognition demos run a classifier on single frames and print a label. This project is built
the other way round. Each claim has to survive measurement on the person it's about. That makes the repo
useful beyond its own use:

- **The emotional response as the subject, with its parts kept apart.** What a person reports, what shows on
  them and what their body does are tied together less tightly than is usually assumed, and how tightly differs
  between people and situations (Mauss et al. 2005; Mauss & Robinson 2009). So no single channel is read as "the emotion". Each is
  recorded as its own measurement of the response, on one clock, for an identified person, which makes the
  ties between them something to measure per person instead of something to assume. EEG adds the part that
  has been missing: the response as the brain produces it (see [EEG](#eeg-the-response-from-inside-next)).
- **A worked example of honest affect sensing.** It shows how to scope claims to what the data supports.
  Every reading carries its evidence (votes, share, lead, why frames were skipped, calibrated confidence)
  instead of a bare verdict.
- **Personal ground truth, not dataset ground truth.** Posed sessions (including talking) and self-reports
  are recorded on the person's own face, webcam and voice. Public datasets are mostly posed, Western and
  recorded under other conditions. Measuring on the actual user exposes failures they hide. On the first
  session the model read posed "angry" as disgust 84% of the time and was strongly overconfident (fitted
  temperature about 6), while "happy" and "neutral" were right 98% of the time.
- **Several channels that fail in different ways.** Appearance (expression model and 52 blendshapes),
  motion, talking, blinks, gaze, smile timing and symmetry, webcam pulse (rPPG), body posture (head height,
  shoulders, lean, movement, breathing, hand on face) and voice prosody plus a voice arousal/valence model
  (see [Body](#body) and [Voice](#voice)). Combining signals only helps when their errors are independent, so
  `expr_eval.py` measures before anything is combined.
- **Identity makes context possible.** Because people are recognized, every signal can be read against that
  person's own normal, alone or in company, instead of a population average. The next step is the
  interaction between identified people (Kenny's Social Relations Model: what is the person's own style,
  what a partner brings out, and what is unique to the pair).
- **A complete measurement harness.** In one script: vote vs state-model accuracy, confidence calibration,
  accuracy by person, face quality, size, head turn and brightness, talking-detector thresholds and
  microphone agreement, and correlations between signals and self-reported feelings. It also produces a
  per-person valence/arousal fit, kept only if it beats guessing the person's average.
- **Reproducible engineering numbers.** Model comparisons (`tools/eval_models.py`) and CPU/GPU/NPU timing on
  an AMD Ryzen AI machine (`tools/hw_bench.py`): GPU 2.6–3.6 ms, CPU 12 ms and NPU 25 ms per face for ArcFace.
- **A documented trail of decisions.** [PROJECT_CONTEXT.md](PROJECT_CONTEXT.md) records what was built, in
  what order, with which measured values and why. The design review that shaped the expression work is kept
  as [Real-Time Facial Emotion Recognition.md](Real-Time%20Facial%20Emotion%20Recognition.md).

## EEG: the response from inside (next)

Not built yet: the headset hasn't arrived and nothing in the code reads it. This section says why it is the next
channel and what it is expected to add.

### What is missing without it

Face, voice and body are displays. A display can be composed (a polite smile), held back (a still face during
a strong response) or absent (nobody shows much alone at a desk), and the link from a face to a feeling is weak
and depends on context (Barrett et al. 2019). The only measure of the feeling itself is the person's own report,
and reports are few: the per-person fit needs 15 to 20 of them before it exists at all, and each one interrupts
what it asks about. Between two reports the project can only compare one display with another.

EEG measures electrical activity of the brain at the scalp, many times a second, whether or not anything shows.
It is not a reading of feelings either. It is a second, independent measurement of the same response, taken
from the other side.

### What it adds to the research

- **A reference between self-reports.** Each expression reading (one every 5 seconds per named person) gets
  an EEG measurement from the same seconds. Every outward signal can then be tested against the brain's
  response thousands of times a day instead of at a handful of reports.
- **Display and response told apart.** Moments when the face shows something and the EEG doesn't change, and
  moments when the EEG changes and the face stays still, become visible and countable. Which outward signals
  follow the inner response for a given person, and which are mostly manner, is the question this project
  exists to answer.
- **Responses to real events, by name.** Emotion research with EEG mostly shows pictures or film clips in a
  laboratory, because the brain's response has to be lined up with the moment something happened (the late
  positive potential to emotional pictures: Hajcak, MacNamara & Olvet 2010). This system already knows those
  moments in daily life, on one UTC clock: which recognized person came into view, on which camera, which song
  started, what the room sounded like. The response in the seconds after a specific person arrives can be
  measured, per wearer and per person arriving.
- **How tightly one person's response hangs together.** With report, face, voice, body and brain recorded
  together, the ties between them can be measured for each person, alone and in company, instead of taken
  from a population average (the same idiographic rule as the rest of the project).
- **Each channel checks the other's errors.** Muscle and eye activity are far stronger at the scalp than brain
  activity, and above about 20 Hz much of a scalp recording is muscle (Whitham et al. 2007): a smile, a blink,
  talking or a clenched jaw can pass for a change in the brain. The cameras measure exactly those movements at
  the same moment (52 face shapes, blinks, jaw, head motion), so affected seconds can be marked instead of
  guessed. The headset's own detection of facial actions can in turn be compared with what the camera saw.
- **The vendor's numbers get measured too.** The headset's software reports its own estimates ("stress",
  "engagement", "relaxation" and others). Here they are hypotheses like any other signal: compared with the
  wearer's reports and with the other channels before they count for anything.

### What the research says

- Frontal alpha asymmetry is the most studied EEG measure in emotion research. It follows approach and
  withdrawal motivation better than pleasant and unpleasant feeling (Davidson 1992; Coan & Allen 2004;
  Harmon-Jones & Gable 2018), so it is not a "valence meter".
- Telling emotional states apart from EEG works above chance, modestly, and within one person much better than
  across people (the DEAP recordings: Koelstra et al. 2012). That fits this project, which fits every model
  per person.
- An earlier headset of the same family gave auditory evoked responses close to a laboratory system's
  (Badcock et al. 2013). The 16-channel model planned here (Emotiv EPOC X PRO) hasn't been checked by this
  project.

### Limits, known before it arrives

- One headset measures one wearer. Everyone else in view is still read from outside only.
- Wearing a headset changes the situation, and dry sensors lose contact when the wearer moves. The contact
  quality of every sensor has to be recorded with the readings.
- Sixteen scalp sensors can't locate activity deep in the brain, where much of emotion is processed.
- The headset can only be read through its maker's own program, which needs an account with them and the
  internet to sign in (as its maker documents it today). That program is outside this project's control. What
  the project would keep of the readings stays in your own database, like the expression log.
- What is stored (band powers and the maker's estimates, or the raw signal as well) isn't decided. Whatever is
  stored belongs to the wearer by name and goes into their export and their forget, like everything else held
  about a person.
- An EEG reading is "what the recording showed", not what the person feels. The "looks, not feels" rule holds
  here too.

References for this section: Badcock et al. (2013), *PeerJ* 1:e38. Barrett et al. (2019), *Psychological
Science in the Public Interest* 20(1). Coan & Allen (2004), *Biological Psychology* 67. Davidson (1992),
*Brain and Cognition* 20. Hajcak, MacNamara & Olvet (2010), *Developmental Neuropsychology* 35(2).
Harmon-Jones & Gable (2018), *Psychophysiology* 55(1). Koelstra et al. (2012), *IEEE Transactions on Affective
Computing* 3(1). Mauss et al. (2005), *Emotion* 5(2). Mauss & Robinson (2009), *Cognition and Emotion* 23(2).
Whitham et al. (2007), *Clinical Neurophysiology* 118(8).

## Body

The body is a third channel beside face and voice. For intense emotions it is often the more telling
one: in Aviezer, Trope & Todorov (2012) people couldn't tell winning from losing tennis players by the
face alone, but could from the body. Movement tracks arousal well (Wallbott 1998; Dael, Mortillaro &
Scherer 2012). Some states have typical postures: pride is upright and expanded (Tracy & Robins 2004),
sadness and shame are slumped with the head lowered. As with the voice, valence is hard to read from the
body (Kleinsmith & Bianchi-Berthouze 2013), and no posture maps to a single emotion: a slump can be
sadness or tiredness.

A desk webcam mostly sees head and shoulders, so the tracker measures what that view supports. It uses
MediaPipe Pose Landmarker lite (Apache 2.0, about 15 ms a frame on the CPU):

| Measure | Meaning |
|---|---|
| Head height (nose above the shoulders, in shoulder widths) | Lower: head down or slumped |
| Neck (ears above the shoulders) | Smaller: shoulders raised (shrug, tension) |
| Shoulder tilt; leaning in or back against the last 30 s | Engagement or withdrawal |
| Body movement (shoulders and hands) | Arousal |
| Breathing rate from the shoulders' rise and fall (still body only) | Arousal and stress; a second body-based check beside the webcam pulse |
| Hand on face; arms crossed | Self-touch, thinking or anxiety. A hand also hides the face, so the expression reading can skip those frames (option, off by default) |

The Expression test now includes a "chin on your hand" step. `tools/expr_eval.py` reports how well the
hand detector works and what a hand does to the reading. All body measures go into the feeling reports, so
the per-person fit decides whether they predict how you feel.

## Voice

The face is only one channel, and a weak one on its own. The voice is a second channel that fails in
different ways. A blurry or turned-away face doesn't affect it, and a polite smile can't fake it the
same way. That makes it the most useful independent check on the expression reading.

### What the research says

- **Voice carries arousal much better than valence.** Vocal-expression research (for example the
  Juslin & Laukka 2003 meta-analysis) finds that how activated someone is shows clearly in the voice:
  higher pitch, wider pitch range, louder, faster. Whether the feeling is pleasant or unpleasant is much
  harder to hear. Speech-emotion models show the same pattern, predicting arousal more accurately than
  valence. So treat the voice's arousal number as the stronger signal and its valence as weak.
- **Talking corrupts the face reading.** Speech moves the mouth in shapes the expression model misreads,
  mostly as "surprised" or "happy". Knowing when someone is talking is useful even if nothing else about
  the voice is used.
- **Voice and face are complementary, not redundant.** Combining channels only helps when their errors
  are independent. Face and voice rarely fail at the same moment, which is why they are worth combining,
  but only after measurement shows it helps.

### What the tracker measures

Turn on **Listen to voice (microphone)** on the tracker page. It's off by default.

| Measure | How | Shown as |
|---|---|---|
| Voice activity | Each 20 ms of audio is compared with a running estimate of the room's background noise; 9 dB above it counts as voice | `voice 48%` (share of time speaking) |
| Loudness | dB above the room's noise while speaking | `+18 dB` |
| Pitch and its variability | Autocorrelation over 40 ms, 75–400 Hz; variability in semitones | `180 Hz ±2.1 st` |
| Speaking rate | Loudness peaks per second of speech, about syllables per second | `4.1/s` |
| Arousal, dominance, valence | The Wav2Small voice model, on the last 2 seconds of speech | `voice A.62 V.48` (0–1) |

The voice is attributed to whichever face is talking (jaw moving), or to the only face in view. With the
microphone on, a face counts as **talking** only when its jaw moves *and* there is voice. That makes the
talking detector, and the optional "skip expression while talking", more reliable.

### How it works, and what is never kept

1. The browser captures the microphone at 16 kHz. Echo cancellation is on. Noise suppression and
   automatic gain are off, because they would distort the loudness and pitch being measured.
2. All the measures above, except the model, are computed in the browser (`signals.js`).
3. Every second that contains enough speech (30% of the last 2 seconds), the last 2 seconds of audio go
   to the tracker program on this PC (`POST /api/voice`, accepted only from the tracker's own page).
   The voice model scores the clip on the CPU in a few milliseconds, and the clip is discarded.
4. Readings are smoothed over consecutive clips and expire after 5 seconds without speech.

**Audio is never written to disk or sent anywhere off the PC.** Only the derived numbers appear: on
screen live, and, if you use the feeling check, as seven numbers in the report (the latest voice
arousal, dominance and valence; and, over the last 3 seconds, share of time speaking, loudness, pitch
variability and speaking rate).

### The voice model

**Wav2Small** by audEERING ([model card](https://huggingface.co/audeering/wav2small),
[paper](https://arxiv.org/abs/2408.13920)) has 17,000 parameters (the ONNX file is 69 KB). It turns 16 kHz
speech into arousal, dominance and valence, each about 0–1. It was chosen because it's small enough to run
continuously on the CPU, outputs dimensions rather than emotion categories (matching the valence/arousal
approach used elsewhere), and has a known license.

Models considered and not used:
- **audEERING's large wav2vec2 model** (about 600 MB): too heavy to run continuously.
- **NVIDIA Audio2Emotion:** its license forbids use for emotion recognition.
- **Several wav2vec2-base classifiers:** unclear licenses, about 95M parameters, and categories instead of dimensions.

`tools/get_wav2small.py` downloads only the weight file. It reads that file as plain numbers (a
safetensors file can't contain code), rebuilds the network from the published description, exports it to
ONNX and checks the result against PyTorch (they agree to 4 decimal places). No downloaded code runs.

**License: CC BY-NC-SA 4.0**, research and non-commercial use only. A commercial license is available
from audEERING.

### How the voice is measured

- **Talking steps in the Expression test.** After the seven expressions, the test has two talking steps
  (a neutral face, then smiling) and a "chin on your hand" step. `tools/expr_eval.py` reports how often the microphone's voice activity was on while talking
  versus silent, alongside the jaw-movement detector.
- **Feeling reports.** `expr_eval.py` correlates the voice measures with how you said you felt, and
  includes them in the per-person valence/arousal fit. A voice measure earns weight only if it helps
  predict your answers.

### Limits

- The voice model has only been checked against the original PyTorch model and synthetic tones, not yet
  on real speech in this room. The voice-activity margin (9 dB) is a first guess.
- Wav2Small was trained on podcast speech, mostly English. Accents, languages, microphones and rooms
  unlike that will shift its numbers. That is one reason the fit is made per person.
- With several people talking, the voice is a mix. Attribution is only as good as the jaw-movement
  detector.
- Without a headset, sound from the PC's speakers (a video, a call) can count as voice despite echo
  cancellation.
- Voice arousal is "how activated the voice sounds", not how the person feels. The same "looks, not feels"
  rule applies.

## Current status and limits

- Recognition is mature and measured. On the first posed session, expression reading was right 98% of
  the time for "happy" and "neutral", about half the time for "surprised", and rarely for posed "angry",
  "fearful", "sad" and "disgust".
- The newer signals (talking gate, hand-on-face gate, state model, pulse, body posture and breathing,
  voice model) work in the page but haven't been validated on enough real sessions. Their thresholds are first guesses, marked as such in the code.
- The valence/arousal fit needs about 15–20 feeling reports per person before it exists at all.
- EEG is planned, not built: no code reads a headset yet, and nothing above about EEG has been measured here.
- The shared database, photos between devices and the Cameras tab are new (versions 1.16 to 1.20) and run on a
  few devices so far; the Cameras page was measured with a stand-in for 40 cameras (9–10 frames a second each).
- Web search named both well-known people it was tried on (Amazon's own match confidence 99.4% and 100%: two pictures, not an accuracy measurement) but never private people, and its confidence bands haven't been measured on this project's faces.
- This is a single-user personal system, not a product. Expression, feeling and EEG readings must not be used
  to make decisions about people. The EU AI Act bans emotion recognition in workplaces and schools.

## Your data stays on your own devices

The repository and the releases contain the programs only, no photos, faces, names or labels. Everything
the tools learn from your photos lives in `face-recognizer/data/` (beside `FaceRecognizer.exe` for the
.exe; in `~/Library/Application Support/FaceRecognizer/data` or `~/.local/share/FaceRecognizer/data` for the macOS
and Linux apps), which is created on first start, starts empty, and is in `.gitignore`: face prints, face pictures,
labels, photo dates and people tags, saved Web search answers and benchmark results. Web search keys,
photo descriptions and their settings, Collect faces pictures, alerts, the presence log, each page's settings, what
the tracker learned about each person (usual look, signal normals) and, with a shared database, its connection
settings and this device's keys are in `%APPDATA%\FaceLabels\` (macOS: `~/Library/Application Support/FaceLabels`,
Linux: `~/.local/share/FaceLabels`), also only on your computer.

With a **shared database** (optional; see [Sharing between your devices](face-recognizer/README.md#sharing-between-your-devices-shared-database)),
the same data also goes to your own PostgreSQL server, over TLS, so your other devices get it: faces, prints and
pictures, labels, photos by content, tags, descriptions, alerts, presence visits, each person's learned signals, each
device's settings, and the Web search and describe keys encrypted with your passphrase (the server never sees them
in plain text). Where each photo file is on a device is kept there under that device, so the others know whom to
ask for the original; they never open it by that path. Devices then also answer each other directly on
the local network (port 8767): an original photo another device asks for, and this device's live camera while
**Stream this camera to my other devices** is on (on by default once a shared database is set up). Both are
encrypted, need a request signed with a key only your devices have, and are only shown, never stored. **Share my
photos with my other devices** (Settings → Shared database) turns both off; the database sync goes on.

Each person can be exported or forgotten from the labeling tool (a person's page, **Their data…**): the export
writes everything held about them into a folder (`data.json` and their face pictures); forgetting removes their
name, face prints and pictures from every place above and keeps only where their faces are, so the same photos
aren't read again. An ignored face (a stranger, a bystander) keeps no face print or picture either.

The local network: when **Settings** is open, the app looks for LM Studio servers on this computer's local
networks (a connection attempt to LM Studio's port, each server's model list, each computer's name); while it runs, it
announces this computer's LM Studio there; and with a **Shared models folder** set, it copies vision models to and from
that folder. Large downloads (models, CUDA support for NVIDIA cards on Linux) happen only when you click. The full list
of what reaches the network, setting by setting: [docs/SETTINGS.md](docs/SETTINGS.md#what-reaches-the-network).

Until 2026-11-09 the repository included the author's `data/` folder; it was removed from the repository's
whole history and from the releases then. Keep your own `data/` out of any public repository: it holds
personal data about the people in your photos.

## Licenses of third-party models

The project's own code is under the Apache License 2.0 (`LICENSE`). The models it downloads keep their publishers' terms, listed here; two of them are for non-commercial use only.

| Model | Used for | License |
|---|---|---|
| InsightFace ArcFace R50 (WebFace600K) | Recognition | Non-commercial research only |
| audEERING Wav2Small | Voice arousal/dominance/valence | CC BY-NC-SA 4.0 (non-commercial) |
| OpenCV Zoo YuNet, SFace, expression MobileFaceNet | Detection, fallback recognition, expression | Apache 2.0 / MIT (see OpenCV Zoo) |
| eDifFIQA (tiny) | Face quality | See its upstream repository |
| MediaPipe Face Landmarker | Blendshape cues | Apache 2.0 |
| MediaPipe Pose Landmarker lite | Body posture | Apache 2.0 |
| MediaPipe Hand Landmarker | Hand skeleton (display only) | Apache 2.0 |
| Microsoft Florence-2 large (ONNX, optional download) | Built-in photo describer | MIT |
| Amazon Rekognition, Google Cloud Vision (online services, optional) | Web search in the labeling tool | Paid per use under each provider's terms; your own keys |

Because of the ArcFace and Wav2Small terms, the project as a whole is for **non-commercial use** unless
those models are replaced or licensed.

## Documentation map

- [face-recognizer/README.md](face-recognizer/README.md): user guide for setup, labeling, the live tracker, expression, other signals, files and tuning
- [docs/SETTINGS.md](docs/SETTINGS.md): every setting, what it is for, its default, the environment variables, and what reaches the network
- [docs/WHY_LAYERED_EXPRESSION.md](docs/WHY_LAYERED_EXPRESSION.md): the science behind the layered expression design
- [PROJECT_CONTEXT.md](PROJECT_CONTEXT.md): full history, measured values, environment details and open items
- [Real-Time Facial Emotion Recognition.md](Real-Time%20Facial%20Emotion%20Recognition.md): the external design review (chat export)
- [face-tracker/README.md](face-tracker/README.md): the original browser tracker
- [AGENTS.md](AGENTS.md): layout, conventions, security rules and how to test, for coding assistants and contributors ([CLAUDE.md](CLAUDE.md) reads it)
