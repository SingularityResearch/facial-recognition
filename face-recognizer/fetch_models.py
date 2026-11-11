r"""
fetch_models.py - download the face models the apps use, when they're missing.

The models live in the repo through Git LFS. A copy made without Git LFS (Download ZIP without LFS
files, or git clone without git-lfs) has small placeholder files instead, and FaceRecognizer.exe from
a GitHub Release comes without models. This downloads the ones the tracker and labeling tool use
(about 196 MB), checks each file's SHA-256, and puts it in models\. Files already there and correct are
left alone: a file already there is checked by its SHA-256 too, once, and then remembered as checked (with its size
and modification time, in models\.verified.json), so a start doesn't read 200 MB again; it's checked again when
its size or time changes.

Where from: each model from its original publisher when it offers the very same file (checked 2026-11-09:
byte-identical), else, or when that fails or the file there has changed, from the project's GitHub Release
"models-v1":
- YuNet, eDifFIQA, the expression model: OpenCV Zoo on Hugging Face (pinned revisions).
- Face and pose landmarker: Google's MediaPipe model storage (version 1, not "latest").
- ArcFace R50: InsightFace's release v0.7, inside buffalo_l.zip (289 MB, 5 models). Only this file's 160 MB
  of compressed bytes are fetched (HTTP range requests: the zip's table of contents, then the one file),
  and unpacked as they arrive.
- YuNet with any input size (face finding on a GPU): only "models-v1", made by tools/get_yunet_dynamic.py.
- Wav2Small: only "models-v1". audEERING publishes the weights; the ONNX file is made by tools/get_wav2small.py.

Run:  python fetch_models.py           (FaceRecognizer.exe does this by itself on first start)

Not downloaded: SFace (fallback only, when ArcFace is missing), AdaFace (evaluation only:
tools/get_adaface.py) and the NPU copy of ArcFace (Ryzen AI only; it's in the repo).
Licences: ArcFace (InsightFace) and Wav2Small (audEERING, CC BY-NC-SA 4.0) are non-commercial.
"""
import hashlib
import io
import json
import struct
import sys
import time
import urllib.error
import urllib.request
import zipfile
import zlib
from pathlib import Path

import paths

HERE = Path(__file__).resolve().parent
ROOT = paths.ROOT                 # as face_id.ROOT (FACE_ROOT=<folder> uses another)
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
    "face_recognition_sface_2021dec.onnx":         # the fallback recognizer (and FACE_MODEL=sface_2021dec)
        HF.format("face_recognition_sface", "3d7082438a6e4551e840c9b2bb60b71e8da4b524", "face_recognition_sface_2021dec.onnx"),
    "ediffiqa_tiny_jun2024.onnx":
        HF.format("face_image_quality_assessment_ediffiqa", "4fce5f6a5438b55a9f996a4e02164b63b849c587", "ediffiqa_tiny_jun2024.onnx"),
    "facial_expression_recognition_mobilefacenet_2022july.onnx":
        HF.format("facial_expression_recognition", "b473270576eabddaacffd08adb4bed27b3fc54e6",
                  "facial_expression_recognition_mobilefacenet_2022july.onnx"),
    "face_landmarker.task": MP.format("face_landmarker", "face_landmarker", "face_landmarker.task"),
    "pose_landmarker_lite.task": MP.format("pose_landmarker", "pose_landmarker_lite", "pose_landmarker_lite.task"),
    # (zip, file in it): only that file's bytes are downloaded
    "arcface_w600k_r50.onnx":
        ("https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_l.zip", "w600k_r50.onnx"),
}

# file: (size in bytes, sha256). Change TAG and these together when a model changes.
FILES = {
    "face_detection_yunet_2023mar.onnx": (232589, "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"),
    "arcface_w600k_r50.onnx": (174383860, "4c06341c33c2ca1f86781dab0e829f88ad5b64be9fba56e56bc9ebdefc619e43"),
    "face_recognition_sface_2021dec.onnx": (38696353, "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79"),
    "ediffiqa_tiny_jun2024.onnx": (7272678, "9426c899cc0f01665240cb7d9e7f98e18e24e456c178326c771a43da289bfc6a"),
    "facial_expression_recognition_mobilefacenet_2022july.onnx":
        (4791892, "4f61307602fc089ce20488a31d4e4614e3c9753a7d6c41578c854858b183e1a9"),
    "face_landmarker.task": (3758596, "64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff"),
    "pose_landmarker_lite.task": (5777746, "59929e1d1ee95287735ddd833b19cf4ac46d29bc7afddbbf6753c459690d574a"),
    "wav2small.onnx": (71332, "c9df50bb1e7a4f1f5010279fcebe2110717ff94a3cc6fe66e1b151e177a91210"),
    # YuNet with any input size, for face finding on a GPU (tools/get_yunet_dynamic.py; only in models-v1)
    "face_detection_yunet_2023mar_dynamic.onnx": (229738, "ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0"),
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


VERIFIED = ".verified.json"           # in models\: {file: {size, mtime_ns, sha256}} of files whose SHA-256 was checked


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def _verified():
    try:
        v = json.loads((MODELS / VERIFIED).read_text("utf-8"))
        return v if isinstance(v, dict) else {}
    except (OSError, ValueError):
        return {}


def _remember(name, sha):
    """Note that models\name, as it is now (size and time), has SHA-256 sha."""
    st = (MODELS / name).stat()
    v = _verified()
    v[name] = {"size": st.st_size, "mtime_ns": st.st_mtime_ns, "sha256": sha}
    try:
        paths.atomic_write(MODELS / VERIFIED, json.dumps(v, indent=1))
    except OSError:
        pass                          # read-only folder: checked again next time


def is_good(name):
    """True if models\name is the expected file: right size and SHA-256. The hash is computed only when the file's
    size or modification time differ from the last check (VERIFIED)."""
    path = MODELS / name
    size, sha = FILES[name]
    try:
        st = path.stat()
    except OSError:
        return False
    if st.st_size != size or is_placeholder(path):
        return False
    seen = _verified().get(name) or {}
    if seen.get("size") == st.st_size and seen.get("mtime_ns") == st.st_mtime_ns and seen.get("sha256") == sha:
        return True
    try:
        got = sha256_of(path)
    except OSError:
        return False
    if got == sha:
        _remember(name, got)
    return got == sha


# Files the programs work without (another way is used instead): one that can't be downloaded is skipped with a note,
# not an error, and tried again next time.
OPTIONAL = {"face_detection_yunet_2023mar_dynamic.onnx"}     # without it faces are found on the CPU (OpenCV)


def needed():
    """The model files that are missing, placeholders, or not the expected file (size, SHA-256)."""
    return [name for name in FILES if not is_good(name)]


def download_checked(url, dest, size, sha):
    """Download url to dest (Path) unless dest is already that file, and check it: size in bytes and SHA-256. A file
    already there that isn't it, or a download that isn't, raises OSError and leaves nothing behind. For the
    tools that fetch weights to convert (tools/get_wav2small.py, tools/get_adaface.py)."""
    if dest.exists():
        if dest.stat().st_size == size and sha256_of(dest) == sha:
            return dest
        raise OSError(f"{dest} isn't the expected file (size or SHA-256 differs): delete it to download it again")
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_name(dest.name + ".download")
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "FaceRecognizer-model-fetch"})
        with urllib.request.urlopen(req, timeout=60) as r, open(tmp, "wb") as out:
            while chunk := r.read(1 << 20):
                out.write(chunk)
        if tmp.stat().st_size != size or sha256_of(tmp) != sha:
            raise OSError(f"{url} isn't the expected file (size or SHA-256 differs)")
        paths.replace_file(tmp, dest)
    finally:
        tmp.unlink(missing_ok=True)
    return dest


class _RemoteFile(io.RawIOBase):
    """A file on a web server, read with HTTP range requests (so zipfile can read a zip's table of contents
    without downloading the zip)."""

    def __init__(self, url, hdr):
        self.url, self.hdr, self.pos = url, hdr, 0
        with urllib.request.urlopen(urllib.request.Request(url, headers=hdr, method="HEAD"), timeout=60) as r:
            self.size = int(r.headers["Content-Length"])

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, off, whence=0):
        self.pos = off if whence == 0 else self.pos + off if whence == 1 else self.size + off
        return self.pos

    def get(self, start, end, timeout=60):
        """Response for bytes start..end-1; raises OSError if the server doesn't send just that part."""
        req = urllib.request.Request(self.url, headers={**self.hdr, "Range": f"bytes={start}-{end - 1}"})
        r = urllib.request.urlopen(req, timeout=timeout)
        if r.status != 206:
            r.close()
            raise OSError(f"{self.url} doesn't serve parts of the file")
        return r

    def readinto(self, b):
        if self.pos >= self.size:
            return 0
        with self.get(self.pos, min(self.size, self.pos + len(b))) as r:
            d = r.read()
        b[:len(d)] = d
        self.pos += len(d)
        return len(d)


def _zip_member(url, member, hdr):
    """Open one file inside a zip on a web server: (response, unpack) where unpack(chunk) gives the file's
    bytes. Reads the zip's table of contents and that file's header, then streams just its compressed bytes."""
    rf = _RemoteFile(url, hdr)
    info = zipfile.ZipFile(io.BufferedReader(rf, 1 << 16)).getinfo(member)
    if info.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
        raise OSError(f"{member} in {url} is packed in a way this can't unpack")
    rf.seek(info.header_offset)
    head = rf.read(30)
    if head[:4] != b"PK\x03\x04":
        raise OSError(f"{url} isn't the expected zip")
    n, extra = struct.unpack("<HH", head[26:30])
    start = info.header_offset + 30 + n + extra
    r = rf.get(start, start + info.compress_size)
    if info.compress_type == zipfile.ZIP_STORED:
        return r, lambda chunk: chunk
    return r, zlib.decompressobj(-15).decompress


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
    Returns the names downloaded. Raises OSError (with the reason) when one can't be fetched, unless it's OPTIONAL."""
    todo = needed()
    got_all = []
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
            try:
                if isinstance(url, tuple):                  # a file inside a zip
                    r, unpack = _zip_member(*url, hdr)
                else:
                    r, unpack = urllib.request.urlopen(urllib.request.Request(url, headers=hdr), timeout=60), None
                with r, open(tmp, "wb") as out:
                    while chunk := r.read(1 << 20):
                        if unpack:
                            chunk = unpack(chunk)
                        out.write(chunk)
                        h.update(chunk)
                        got += len(chunk)
                        if progress:
                            progress(done + got, total, name)
                if h.hexdigest() != sha:
                    raise OSError(f"{name} downloaded, but it isn't the expected file (checksum differs)")
                paths.replace_file(tmp, MODELS / name)
                _remember(name, sha)
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
                    if name in OPTIONAL:
                        print(f"\n  (Skipped {name}, which is optional: {e})", flush=True)
                        break
                    raise OSError(f"Couldn't download {name} from {BASE}: {e}") from e
                time.sleep(WAIT * attempt)
        else:
            continue
        if (MODELS / name).exists() and not is_placeholder(MODELS / name):
            got_all.append(name)
    return got_all


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
