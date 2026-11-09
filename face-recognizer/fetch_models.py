r"""
fetch_models.py - download the face models the apps use, when they're missing.

The models live in the repo through Git LFS. A copy made without Git LFS (Download ZIP without LFS
files, or git clone without git-lfs) has small placeholder files instead, and FaceRecognizer.exe from
a GitHub Release comes without models. This downloads the ones the tracker and labeling tool use
(about 196 MB), checks each file's SHA-256, and puts it in models\. Files already there and correct are
left alone.

Where from: each model from its original publisher when it offers the very same file (checked 2026-11-09:
byte-identical), else, or when that fails or the file there has changed, from the project's GitHub Release
"models-v1":
- YuNet, eDifFIQA, the expression model: OpenCV Zoo on Hugging Face (pinned revisions).
- Face and pose landmarker: Google's MediaPipe model storage (version 1, not "latest").
- ArcFace R50: only "models-v1". InsightFace publishes it inside buffalo_l.zip (289 MB for this 174 MB file).
- Wav2Small: only "models-v1". audEERING publishes the weights; the ONNX file is made by tools/get_wav2small.py.

Run:  python fetch_models.py           (FaceRecognizer.exe does this by itself on first start)

Not downloaded: SFace (fallback only, when ArcFace is missing), AdaFace (evaluation only:
tools/get_adaface.py) and the NPU copy of ArcFace (Ryzen AI only; it's in the repo).
Licences: ArcFace (InsightFace) and Wav2Small (audEERING, CC BY-NC-SA 4.0) are non-commercial.
"""
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = Path(sys.executable).resolve().parent if getattr(sys, "frozen", False) else HERE   # as face_id.ROOT
MODELS = ROOT / "models"
REPO = "SingularityResearch/facial-recognition"
TAG = "models-v1"
BASE = f"https://github.com/{REPO}/releases/download/{TAG}/"
# The same files through GitHub's API: different servers, used when the download links fail (on
# 2026-11-08 every release link answered 504 for a while, while the API served the files fine).
API = f"https://api.github.com/repos/{REPO}/releases/tags/{TAG}"
_api_urls = {}
LFS_SIGN = b"version https://git-lfs"
TRIES, WAIT = 4, 3                    # attempts at models-v1 per file; seconds before retry n is WAIT * n
HF = "https://huggingface.co/opencv/{}/resolve/{}/{}"
MP = "https://storage.googleapis.com/mediapipe-models/{}/{}/float16/1/{}"
# The original publishers' copies of the same files (same size and SHA-256 as below), tried first, once each.
ORIGIN = {
    "face_detection_yunet_2023mar.onnx":
        HF.format("face_detection_yunet", "3cc26e7f1014a5ee5d74a42acee58bafc9d0a310", "face_detection_yunet_2023mar.onnx"),
    "ediffiqa_tiny_jun2024.onnx":
        HF.format("face_image_quality_assessment_ediffiqa", "4fce5f6a5438b55a9f996a4e02164b63b849c587", "ediffiqa_tiny_jun2024.onnx"),
    "facial_expression_recognition_mobilefacenet_2022july.onnx":
        HF.format("facial_expression_recognition", "b473270576eabddaacffd08adb4bed27b3fc54e6",
                  "facial_expression_recognition_mobilefacenet_2022july.onnx"),
    "face_landmarker.task": MP.format("face_landmarker", "face_landmarker", "face_landmarker.task"),
    "pose_landmarker_lite.task": MP.format("pose_landmarker", "pose_landmarker_lite", "pose_landmarker_lite.task"),
}

# file: (size in bytes, sha256). Change TAG and these together when a model changes.
FILES = {
    "face_detection_yunet_2023mar.onnx": (232589, "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"),
    "arcface_w600k_r50.onnx": (174383860, "4c06341c33c2ca1f86781dab0e829f88ad5b64be9fba56e56bc9ebdefc619e43"),
    "ediffiqa_tiny_jun2024.onnx": (7272678, "9426c899cc0f01665240cb7d9e7f98e18e24e456c178326c771a43da289bfc6a"),
    "facial_expression_recognition_mobilefacenet_2022july.onnx":
        (4791892, "4f61307602fc089ce20488a31d4e4614e3c9753a7d6c41578c854858b183e1a9"),
    "face_landmarker.task": (3758596, "64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff"),
    "pose_landmarker_lite.task": (5777746, "59929e1d1ee95287735ddd833b19cf4ac46d29bc7afddbbf6753c459690d574a"),
    "wav2small.onnx": (71332, "c9df50bb1e7a4f1f5010279fcebe2110717ff94a3cc6fe66e1b151e177a91210"),
}


def is_placeholder(path):
    """True for a Git LFS placeholder: a small text file standing in for a file that wasn't downloaded."""
    try:
        if path.stat().st_size > 1024:
            return False
        with open(path, "rb") as f:
            return f.read(len(LFS_SIGN)) == LFS_SIGN
    except OSError:
        return False


def needed():
    """The model files that are missing, placeholders, or the wrong size."""
    return [name for name, (size, _) in FILES.items()
            if not (MODELS / name).exists() or is_placeholder(MODELS / name) or (MODELS / name).stat().st_size != size]


def _source(name, attempt):
    """(url, headers) for a download attempt at models-v1: odd attempts use the release download link,
    even ones GitHub's API for the same file (falling back to the link if the API listing can't be read)."""
    hdr = {"User-Agent": "FaceRecognizer-model-fetch"}
    if attempt % 2 == 0:
        try:
            if not _api_urls:
                with urllib.request.urlopen(urllib.request.Request(API, headers=hdr), timeout=30) as r:
                    _api_urls.update({a["name"]: a["url"] for a in json.loads(r.read()).get("assets", [])})
            if name in _api_urls:
                return _api_urls[name], {**hdr, "Accept": "application/octet-stream"}
        except (OSError, ValueError):
            pass
    return BASE + name, hdr


def fetch(progress=None):
    """Download every needed model. progress(done_bytes, total_bytes, file name) is called as it goes.
    Returns the names downloaded. Raises OSError (with the reason) when one can't be fetched."""
    todo = needed()
    total, done = sum(FILES[n][0] for n in todo), 0
    MODELS.mkdir(parents=True, exist_ok=True)
    hdr0 = {"User-Agent": "FaceRecognizer-model-fetch"}
    for name in todo:
        size, sha = FILES[name]
        tmp = MODELS / (name + ".download")
        # attempt 0: the original publisher (any failure, including a changed file, falls through to models-v1)
        for attempt in range(0 if name in ORIGIN else 1, TRIES + 1):
            h, got = hashlib.sha256(), 0
            url, hdr = (ORIGIN[name], hdr0) if attempt == 0 else _source(name, attempt)
            req = urllib.request.Request(url, headers=hdr)
            try:
                with urllib.request.urlopen(req, timeout=60) as r, open(tmp, "wb") as out:
                    while chunk := r.read(1 << 20):
                        out.write(chunk)
                        h.update(chunk)
                        got += len(chunk)
                        if progress:
                            progress(done + got, total, name)
                if h.hexdigest() != sha:
                    raise OSError(f"{name} downloaded, but it isn't the expected file (checksum differs)")
                os.replace(tmp, MODELS / name)
                done += got
                break
            except Exception as e:
                tmp.unlink(missing_ok=True)
                if progress:
                    progress(done, total, name)
                if attempt == 0:
                    continue
                # GitHub's download servers sometimes answer 5xx or drop a large download midway: try
                # again after a pause. A 4xx (e.g. 404: the release is gone) won't fix itself.
                lasting = isinstance(e, urllib.error.HTTPError) and 400 <= e.code < 500
                if lasting or attempt == TRIES:
                    raise OSError(f"Couldn't download {name} from {BASE}: {e}") from e
                time.sleep(WAIT * attempt)
    return todo


def main():
    todo = needed()
    if not todo:
        print("All face models are here.")
        return
    mb = sum(FILES[n][0] for n in todo) / 1e6
    print(f"Downloading {len(todo)} face models ({mb:.0f} MB) into {MODELS} ...")
    last = [-1]

    def show(done, total, name):
        pct = int(done * 100 / total)
        if pct != last[0]:
            last[0] = pct
            print(f"\r  {pct:3d}%  {name:<60}", end="", flush=True)
    try:
        fetch(show)
    except OSError as e:
        sys.exit(f"\n{e}")
    print("\nDone.")


if __name__ == "__main__":
    main()
