r"""
fetch_models.py - download the face models the apps use, when they're missing.

The models live in the repo through Git LFS. A copy made without Git LFS (Download ZIP without LFS
files, or git clone without git-lfs) has small placeholder files instead, and FaceRecognizer.exe from
a GitHub Release comes without models. This downloads the ones the tracker and labeling tool use
(about 196 MB) from the project's GitHub Release "models-v1", checks each file's SHA-256, and puts it
in models\. Files already there and correct are left alone.

Run:  python fetch_models.py           (FaceRecognizer.exe does this by itself on first start)

Not downloaded: SFace (fallback only, when ArcFace is missing), AdaFace (evaluation only:
tools/get_adaface.py) and the NPU copy of ArcFace (Ryzen AI only; it's in the repo).
Licences: ArcFace (InsightFace) and Wav2Small (audEERING, CC BY-NC-SA 4.0) are non-commercial.
"""
import hashlib
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = Path(sys.executable).resolve().parent if getattr(sys, "frozen", False) else HERE   # as face_id.ROOT
MODELS = ROOT / "models"
TAG = "models-v1"
BASE = f"https://github.com/SingularityResearch/facial-recognition/releases/download/{TAG}/"
LFS_SIGN = b"version https://git-lfs"
TRIES, WAIT = 4, 3                    # attempts per file; seconds before retry n is WAIT * n

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


def fetch(progress=None):
    """Download every needed model. progress(done_bytes, total_bytes, file name) is called as it goes.
    Returns the names downloaded. Raises OSError (with the reason) when one can't be fetched."""
    todo = needed()
    total, done = sum(FILES[n][0] for n in todo), 0
    MODELS.mkdir(parents=True, exist_ok=True)
    for name in todo:
        size, sha = FILES[name]
        tmp = MODELS / (name + ".download")
        for attempt in range(1, TRIES + 1):
            h, got = hashlib.sha256(), 0
            req = urllib.request.Request(BASE + name, headers={"User-Agent": "FaceRecognizer-model-fetch"})
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
