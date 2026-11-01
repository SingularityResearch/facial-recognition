# Facial Recognition and Expression Research

A fully local system that recognizes specific people on a webcam and reads facial expression, voice and
other signals live. The expression side is also a small, honest research testbed: it measures what each
signal is actually worth on your own face before letting it influence anything.

Everything runs on one Windows PC. No photos, video, audio or face data leave the machine.

## Goals

1. **Recognize the people who matter, live.** Learn specific people from a personal photo library, then
   show their name and match confidence on their face in the browser.
2. **Read expression honestly.** Report what a face *looks like* ("this face currently looks happy"),
   never claim to know what someone *feels*, and say how sure the reading is.
3. **Find out what actually works.** Treat every new signal (face shape, motion, talking, voice, pulse) as
   a hypothesis. Measure it on the person's own data against ground truth before it may change a reading.

## What's in the repo

| Part | What it does | Start it |
|---|---|---|
| [face-recognizer/](face-recognizer/README.md) | The Python project: photo import, face recognition, labeling, live tracker, measurement tools | `face-recognizer\setup.bat` once, then the two apps below |
| Labeling tool (`label_tool.py`) | Add photos, confirm suggested names, name new people, fix mistakes | `face-recognizer\label.bat` → http://127.0.0.1:8765 |
| Live tracker (`tracker.py`) | Names, match confidence, expression and other signals on each face | `face-recognizer\recognize.bat` → http://127.0.0.1:8766 |
| Measurement (`tools/expr_eval.py`, `tools/eval_models.py`, `tools/hw_bench.py`) | Accuracy, calibration, talking, per-condition breakdowns, valence/arousal fit, model and hardware comparisons | `python tools\expr_eval.py` from `face-recognizer\` |
| [face-tracker/](face-tracker/README.md) | The original standalone browser face tracker (MediaPipe), no install | Double-click `index.html` |

## Key decisions, and why

| Decision | Why |
|---|---|
| **Local only**: two tiny servers on 127.0.0.1, with request checks (`localguard.py`) | Faces and voices are personal data. Nothing is uploaded, and other websites can't reach the servers. |
| **Thresholds are measured values**, never guesses | Each matching, quality and voting threshold was measured on the owner's labeled photos, for example ArcFace match 0.35: 100% of labeled faces correct, 0% of strangers named. Changing one means re-measuring. |
| **ArcFace for recognition**, with SFace as a fallback; AdaFace evaluated and rejected | ArcFace named 76% of blurry webcam-like faces correctly vs 44% for SFace. AdaFace was slower and less accurate on these photos (95.5% vs 99.4%). |
| **Labels per face, not per cluster** | Cluster numbers change on every regrouping. A permanent ID per face keeps every decision, with undo history. |
| **"Looks", not "feels"** | Research shows facial expression is a weak, context-dependent indicator of felt emotion. The wording follows the evidence. |
| **Layered expression reading** | A quality gate, then voting over time, then a per-person "usual look", then evidence. Each layer fixes a specific failure of single-frame emotion detection. |
| **New signals are evidence only until measured** | Face-shape cues, talking, motion, blinks, gaze, pulse, voice and the state model are shown but don't change the label. The options that would change it are off by default. |
| **Self-report is the only ground truth for feelings** | An opt-in "feeling check" asks how you feel on two 1–7 scales. Without it, accuracy can only be measured against posed expressions, never against feelings. |
| **Only numbers are stored for research** | Posed tests and feeling reports save scores and signals: no pictures, no audio. Live expression, voice and pulse are never written down. |
| **Two separate apps** (labeling and tracking) | Owner preference. Each stays simple, and both can run at once. |

The reasoning behind the layered design, with references: [docs/WHY_LAYERED_EXPRESSION.md](docs/WHY_LAYERED_EXPRESSION.md).

## Why this is useful research material

Most emotion-recognition demos run a classifier on single frames and print a label. This project is built
the other way round. Each claim has to survive measurement on the person it's about. That makes the repo
useful beyond its own use:

- **A worked example of honest affect sensing.** It shows how to scope claims to what the data supports.
  Every reading carries its evidence (votes, share, lead, why frames were skipped, calibrated confidence)
  instead of a bare verdict.
- **Personal ground truth, not dataset ground truth.** Posed sessions (including talking) and self-reports
  are recorded on the person's own face, webcam and voice. Public datasets are mostly posed, Western and
  recorded under other conditions. Measuring on the actual user exposes failures they hide. On the first
  session the model read posed "angry" as disgust 84% of the time and was strongly overconfident (fitted
  temperature about 6), while "happy" and "neutral" were right 98% of the time.
- **Several channels that fail in different ways.** Appearance (expression model and 52 blendshapes),
  motion, talking, blinks, gaze, smile timing and symmetry, webcam pulse (rPPG) and voice prosody plus a
  voice arousal/valence model (see [Voice](#voice)). Combining signals only helps when their errors are independent, so
  `expr_eval.py` measures before anything is combined.
- **A complete measurement harness.** In one script: vote vs state-model accuracy, confidence calibration,
  accuracy by person, face quality, size, head turn and brightness, talking-detector thresholds and
  microphone agreement, and correlations between signals and self-reported feelings. It also produces a
  per-person valence/arousal fit, kept only if it beats guessing the person's average.
- **Reproducible engineering numbers.** Model comparisons (`tools/eval_models.py`) and CPU/GPU/NPU timing on
  an AMD Ryzen AI machine (`tools/hw_bench.py`): GPU 2.6–3.6 ms, CPU 12 ms and NPU 25 ms per face for ArcFace.
- **A documented trail of decisions.** [PROJECT_CONTEXT.md](PROJECT_CONTEXT.md) records what was built, in
  what order, with which measured values and why. The design review that shaped the expression work is kept
  as [Real-Time Facial Emotion Recognition.md](Real-Time%20Facial%20Emotion%20Recognition.md).

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

- **Talking steps in the Expression test.** The test ends with two talking steps (a neutral face, then
  smiling). `tools/expr_eval.py` reports how often the microphone's voice activity was on while talking
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
- The newer signals (talking gate, state model, pulse, voice model) work in the page but haven't been
  validated on enough real sessions. Their thresholds are first guesses, marked as such in the code.
- The valence/arousal fit needs about 15–20 feeling reports per person before it exists at all.
- This is a single-user personal system, not a product. Expression and feeling readings must not be used
  to make decisions about people. The EU AI Act bans emotion recognition in workplaces and schools.

## Privacy note about this repository

`face-recognizer/data/` is tracked in git and holds personal data derived from the owner's photo library:
face prints, face thumbnails, contact sheets, labels and test sessions. **Don't push this repository to a
public or shared remote with `data/` included.** To share the code as research material, remove `data/`
from the history first, or publish a copy without it.

## Licenses of third-party models

| Model | Used for | License |
|---|---|---|
| InsightFace ArcFace R50 (WebFace600K) | Recognition | Non-commercial research only |
| audEERING Wav2Small | Voice arousal/dominance/valence | CC BY-NC-SA 4.0 (non-commercial) |
| OpenCV Zoo YuNet, SFace, expression MobileFaceNet | Detection, fallback recognition, expression | Apache 2.0 / MIT (see OpenCV Zoo) |
| eDifFIQA (tiny) | Face quality | See its upstream repository |
| MediaPipe Face Landmarker | Blendshape cues | Apache 2.0 |

Because of the ArcFace and Wav2Small terms, the project as a whole is for **non-commercial use** unless
those models are replaced or licensed.

## Documentation map

- [face-recognizer/README.md](face-recognizer/README.md): user guide for setup, labeling, the live tracker, expression, other signals, files and tuning
- [docs/WHY_LAYERED_EXPRESSION.md](docs/WHY_LAYERED_EXPRESSION.md): the science behind the layered expression design
- [PROJECT_CONTEXT.md](PROJECT_CONTEXT.md): full history, measured values, environment details and open items
- [Real-Time Facial Emotion Recognition.md](Real-Time%20Facial%20Emotion%20Recognition.md): the external design review (chat export)
- [face-tracker/README.md](face-tracker/README.md): the original browser tracker
- [CLAUDE.md](CLAUDE.md) / [AGENTS.md](AGENTS.md): conventions for coding assistants working in this repo
