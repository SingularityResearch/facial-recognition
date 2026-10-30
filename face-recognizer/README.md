# Face Recognizer

Three programs share the `data` folder:

- **tracker.py**: the live face tracker in your browser (double-click **recognize.bat**)
- **label_tool.py**: add photos, confirm suggested names, name new people, fix mistakes (double-click **label.bat**)
- **face_id.py**: command-line scanning, grouping and recognition

Recognizes specific people (by name) on a webcam or in photos and videos, trained on your own photo library.

- **Detection:** OpenCV YuNet (`models/face_detection_yunet_2023mar.onnx`)
- **Recognition:** ArcFace, ResNet-50 trained on WebFace600K (`models/arcface_w600k_r50.onnx`, from InsightFace's buffalo_l pack). It turns each face into a 512-number face print. InsightFace's pretrained models are licensed for non-commercial use; commercial use needs a license from InsightFace. The older, lighter SFace model is kept as a fallback.
- **Matching:** a face is labeled with the known person whose 5 closest face prints are most similar, if the similarity is at least 0.35. Otherwise it's labeled "Unknown". Measured on your labels: 100% named correctly, no strangers named, and on simulated blurry webcam frames 76% named correctly (SFace managed 44%).
- **Face quality:** eDifFIQA (`models/ediffiqa_tiny_jun2024.onnx`) scores each face. Faces below 0.30 (blurry, tiny, turned away) aren't used to learn a person, and in the live tracker sharp frames outweigh blurry ones when deciding a name.
- **Expression:** OpenCV Zoo's facial expression model (`models/facial_expression_recognition_mobilefacenet_2022july.onnx`, MobileFaceNet, Apache 2.0; 88% on the RAF-DB benchmark) reads each face as angry, disgust, fearful, happy, neutral, sad or surprised. The live tracker shows it under the name; see **Expression** below for how it's kept honest.
- **Security:** the labeling tool and tracker only accept requests from their own pages (see `localguard.py`), so other websites can't change your labels.

Everything runs locally. No photos leave your computer.

## Setup (once)

1. Install Python 3.10+ from python.org (tick "Add to PATH").
2. In this folder:

   ```
   pip install -r requirements.txt
   ```

## Use

```
python face_id.py people                     # who it knows
python face_id.py recognize --camera 0       # live webcam; try 1 or 2 if 0 is the wrong camera
python face_id.py recognize "D:\some\photos" --out tagged   # tag photos, save annotated copies
```

## Live tracker: tracker.py

Double-click **recognize.bat**. Your browser opens the Face Tracker:

- **Camera**: pick your webcam from the list by name, then **Start camera**. **Open video** tracks a video file instead.
- Each face gets a box, and the person's name and match percentage appear on their face. People you haven't named show as **Unknown**.
- **Display**: mirror view, keypoints, motion trail, show or blur unknown faces, smoothing.
- **Match strictness**: raise it if wrong names appear, lower it if known people show as Unknown.
- **Show expression** / **Calibrate to each person**: see below.
- Names you save in the labeling tool are picked up automatically.

### Expression

The model reads the *expression on the face*, not what the person feels: a polite smile and a happy one both read "happy", and a frown of concentration reads "angry" or "sad". Treat it as "this face currently looks ...". Several things are done to make that reading as accurate as it can be:

- **Only readable faces are scored.** The recognizer skips frames whose quality is below 0.25 or whose head is turned more than about 35 degrees (`Q_EXPR` and `MAX_TURN` in `face_id.py`). Blur and profile views don't make the model unsure, they make it wrong (usually "sad" or "neutral"), so those frames cast no vote rather than a weak one.
- **Each face is scored twice**, as is and mirrored, and the two are averaged. This is a standard accuracy gain for expression models; measured on your stored faces, the two views give the same label 93% of the time. Both runs together take 0.7 ms per face on the GPU.
- **Frames vote.** The page averages the last ten scored frames (about a second), weighted by face quality, and only reports a label that has at least 45% of the vote and leads the runner-up by 12 points. Anything less is shown dimmed with a question mark. On your stored faces 95% give a clear winner.
- **Calibrate to each person** (on by default) learns each named person's usual look: a slow running average of their scored frames, kept in this browser (**Forget learned usual looks** clears it). It's trusted after about 100 frames of that person. Then a reading that departs from their usual look is shown bright, and one that matches it is tagged **usual**. This is what stops someone whose resting face reads a bit "sad", or who smiles most of the time, from being reported as news every second. Turn it off to see the raw reading.

What to expect: "happy", "surprised" and "neutral" are the reliable labels. "Fearful" and "disgust" are the rarest in the model's training data and the ones it confuses most; "angry" and "sad" are often a resting face. On your photo library the readings are 40-45% neutral, 40-45% happy and the rest spread thinly, which matches what the photos are.

`python tools\expr_check.py` scores a sample of your stored faces (time per face, label spread, how often the mirrored views agree) and runs one tracker request on a stored photo: a quick check that the model is installed and working. Expression isn't stored anywhere; it's only shown live.

Keep the black **Face Tracker** window open while you use it.

## Label people: label_tool.py (separate app)

Double-click **label.bat**, or run `python label_tool.py`. Your browser opens **Face Labels**. Every change is saved at once, and **Undo** (or Z) takes back the last one.

**Add photos** reads a folder (subfolders too) in the background, using the GPU. It skips photos it has already read, even renamed or moved ones, and copies of photos (re-saved or resized) whose faces match. When a new face clearly matches someone you've named, it's named automatically. The rest wait for you in two lists:

- **Suggestions**: faces the model thinks it knows, per person, least certain first. Click the wrong ones (or drag a box across them), press **R** ("not this person"), then **Accept** the rest.
- **New faces**: faces that look like nobody you've named, in groups of look-alikes. Name a group, or select some of its faces to name only those. **Ignore** strangers, artwork and tattoos so they aren't asked about again.

Also:
- **Named automatically**: check the automatic names, if you like, least certain first.
- **Checks**: named faces that look more like someone else, people with fewer than 5 clear faces, and the duplicate photos that were skipped.
- **People**: click a person to see all their faces, least typical first, so mistakes are at the top. **Rename or merge** fixes a name or combines two people.
- Double-click a face (or press Space) to see the whole photo with everyone in it boxed and named; ← → step through faces.
- Keys: **A** accept, **N** name, **R** not this person, **X** ignore, **Z** undo, **Esc** clear the selection. Shift-click selects a range.
- Faces too blurry or small to learn from are hidden from the lists; **Show blurry faces** brings them back.

How sure counts as sure: a face is named automatically when it scores 0.45 or more with its best match and at least 0.10 more than the next person. On your labeled faces that named 89% with no mistakes, and no stranger scored above 0.35. It's suggested from 0.35 (the recognizer's match setting). These are `AUTO` and `MARGIN` in `labels.py`.

The live tracker picks up new names by itself. Close the black window when you're done.

## From the command line

```
python face_id.py scan "E:\Users\Chris\Pictures\New"    # same as Add photos
python face_id.py cluster                                 # groups of faces nobody has named, as contact sheets in data\sheets
python face_id.py name 4 9 15 Chris                       # name groups 4, 9 and 15
python face_id.py name 7 --ignore                         # group 7 isn't anyone to recognize
```

## GPU and NPU

Measured on this PC (AMD Ryzen AI Max+ 395: 16-core CPU, Radeon 8060S GPU, XDNA 2 NPU), ArcFace, one face at a time:

| Processor | Time per face | Notes |
|---|---|---|
| **GPU** (Radeon 8060S, DirectML) | **2.6-3.6 ms** | default; identical results to CPU |
| CPU (OpenCV) | ~12 ms | fallback |
| NPU (Ryzen AI 1.8, BF16) | ~25 ms | low power; results 99.97% identical; first start compiles for ~6 minutes |

The recognizer and quality models use the GPU automatically (`onnxruntime-directml`, installed by setup.bat). Face detection (YuNet) stays on the CPU, where it takes a few milliseconds. To choose explicitly, set `FACE_DEVICE` to `gpu`, `cpu` or `npu` before starting. `npu` needs the Ryzen AI environment's Python (`X:\ProgramData\miniconda3\envs\ryzen-ai-1.8.0\python.exe`); `tools/hw_bench.py` compiles the NPU model and re-measures all three.

**Bigger models tried:** AdaFace IR-101 (WebFace12M, `tools/get_adaface.py` converts the official weights) ran at 8.5 ms per face on the GPU but didn't beat ArcFace on your photos: 95.5% vs 99.4% of labeled faces correct, about the same on blurry frames. Combining both models didn't help either (`tools/eval_models.py` reproduces the comparison).

## Switching recognition models

Face prints from different models can't be mixed. After changing models, run `python face_id.py reembed` (about a minute: it re-reads the saved face crops, no photo rescan needed). Your labels are stored per face, so they all carry over. To go back to SFace, set `FACE_MODEL=sface_2021dec` before running the commands.

## Tuning

- **Wrong names:** raise the match setting (the tracker's Match strictness slider, or `--threshold 0.4`).
- **Known people shown as Unknown:** lower it to about 0.3, or label more photos of them.
- **New-face groups mixing two people, or one person split across groups:** select faces within a group to name just those; the grouping distance is `cluster` in `REC_MODELS` in `face_id.py` (0.70 with ArcFace).

## Files

- `data/faces.npz`: every face found, with its permanent ID, face print, quality and a thumbnail
- `data/photos.json`: every photo read: where it is, its fingerprint, and which photo a skipped copy duplicates
- `data/labels.json`: the label of each face (name, ignored, named by you or automatically, names it is not)
- `data/history.jsonl`: every labeling change, with the time; `data/backups/`: a copy of labels.json from each start of Face Labels (last 20)
- `data/people.npz`: face prints of the named people (what the tracker and `recognize` use)
- `data/settings.json`: photo folders you've added
- The expression model and each person's learned usual look aren't written to `data`: the model is optional (the tracker says so at startup if it's missing) and the usual looks live in the browser's storage for the tracker page.

Databases from before per-face labels are upgraded automatically the first time Face Labels starts (or with `python face_id.py migrate "<photo folder>"`). The old group files are kept in `data/backups/legacy-<date>/`.
