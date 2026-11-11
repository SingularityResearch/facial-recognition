"""
face_id.py - recognize specific people from your own photos.

Pipeline
  1. scan      find every face in a photo folder and store a "face print" (512 numbers with ArcFace);
               faces the model is sure about are named automatically
  2. label     label_tool.py: confirm suggestions, name new people (or `cluster` + `name` here)
  3. recognize name people live on a webcam (tracker.py), or in photos / videos on disk

Models, in ./models:
  YuNet    face detection (OpenCV Zoo)
  ArcFace  face recognition, ResNet-50 trained on WebFace600K (InsightFace buffalo_l pack).
           InsightFace's pretrained models are for non-commercial use; commercial use needs a license.
  SFace    the previous, lighter recognition model; used only if ArcFace isn't installed
  eDifFIQA face-quality scoring (OpenCV Zoo)
  FER      facial expression (OpenCV Zoo, MobileFaceNet): 7 expressions, shown by the live tracker
  MediaPipe Face Landmarker (optional, pip mediapipe): 52 face-shape "blendshapes" (smile, brow
           raise, jaw open...), shown by the live tracker as evidence next to the expression
  MediaPipe Pose Landmarker lite (optional): body points (head, shoulders, arms, hips) for posture
  Wav2Small (optional, tools/get_wav2small.py): voice arousal/dominance/valence from the microphone.
           CC BY-NC-SA 4.0, non-commercial use only.
After switching recognition models, run:  python face_id.py reembed

Examples
  python face_id.py scan "D:\\Photos"
  python face_id.py cluster                    (groups of faces nobody has named yet)
  python face_id.py name 3 7 12 Chris          (name groups 3, 7 and 12 Chris)
  python face_id.py name 9 --ignore            (group 9 is not someone to recognize)
  python face_id.py people
  python face_id.py recognize --camera 0
  python face_id.py recognize "D:\\some\\photos" --out tagged
"""
import argparse
import json
import os
import sys
import threading
import time
from pathlib import Path

import cv2
import numpy as np

import paths

HERE = Path(__file__).resolve().parent
# models/ and data/ sit beside the code, or beside FaceRecognizer.exe when built (app.py): the .exe
# carries only the code and pages, and shares this folder's models and data with the .bat launchers
ROOT = paths.ROOT                # FACE_ROOT=<folder> uses another (tests)
MODELS = ROOT / "models"
DATA = ROOT / "data"
DET_MODEL = MODELS / "face_detection_yunet_2023mar.onnx"
QUAL_MODEL = MODELS / "ediffiqa_tiny_jun2024.onnx"            # eDifFIQA face-quality model (optional)
EXPR_MODEL = MODELS / "facial_expression_recognition_mobilefacenet_2022july.onnx"   # expression model (optional)
EXPRESSIONS = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"]   # its outputs, in order
CUE_MODEL = MODELS / "face_landmarker.task"   # MediaPipe Face Landmarker, blendshapes (optional)
# Face-shape cues sent to the tracker page: each is the mean of these MediaPipe blendshapes (left + right).
CUES = {
    "smile": ["mouthSmileLeft", "mouthSmileRight"], "cheek": ["cheekSquintLeft", "cheekSquintRight"],
    "frown": ["mouthFrownLeft", "mouthFrownRight"], "press": ["mouthPressLeft", "mouthPressRight"],
    "stretch": ["mouthStretchLeft", "mouthStretchRight"], "jaw": ["jawOpen"],
    "browDown": ["browDownLeft", "browDownRight"], "browInnerUp": ["browInnerUp"],
    "browOuterUp": ["browOuterUpLeft", "browOuterUpRight"], "eyeWide": ["eyeWideLeft", "eyeWideRight"],
    "eyeSquint": ["eyeSquintLeft", "eyeSquintRight"], "sneer": ["noseSneerLeft", "noseSneerRight"],
    "upperLip": ["mouthUpperUpLeft", "mouthUpperUpRight"], "blink": ["eyeBlinkLeft", "eyeBlinkRight"],
}
# Eye directions for the "gaze" cue: (left eye, right eye) blendshapes that move together.
GAZE = [("eyeLookOutLeft", "eyeLookInRight"), ("eyeLookInLeft", "eyeLookOutRight"),
        ("eyeLookUpLeft", "eyeLookUpRight"), ("eyeLookDownLeft", "eyeLookDownRight")]
POSE_MODEL = MODELS / "pose_landmarker_lite.task"   # MediaPipe Pose Landmarker lite, 33 body points (optional)
# Body points sent to the tracker page, in this order (MediaPipe numbering): nose, ears, shoulders,
# elbows, wrists, index fingers, hips. Each as [x, y, visibility] in frame pixels.
POSE_POINTS = {"nose": 0, "earL": 7, "earR": 8, "shoulderL": 11, "shoulderR": 12, "elbowL": 13, "elbowR": 14,
               "wristL": 15, "wristR": 16, "indexL": 19, "indexR": 20, "hipL": 23, "hipR": 24}
POSE_VIS = 0.5           # a body point counts as seen at this visibility
VOICE_MODEL = MODELS / "wav2small.onnx"       # Wav2Small voice arousal/dominance/valence (optional; tools/get_wav2small.py)
VOICE_RATE = 16000                            # samples per second it expects

IMAGE_EXT = {".jpg", ".jpeg", ".png", ".webp", ".bmp", ".heic", ".heif", ".gif", ".tif", ".tiff"}
VIDEO_EXT = {".mov", ".mp4", ".m4v", ".avi", ".mkv"}

MAX_SIDE = 1280          # photos are shrunk to this before detection (speed)
# Pictures of faces the tracker saves for labeling (its "Collect faces" option: each face with some room around
# it, never the whole frame), with a .json beside each saying who the tracker thought it was. Outside the
# project; the labeling tool imports them.
WEBCAM = paths.HOME / "webcam"
PRESENCE = paths.HOME / "presence"       # the tracker's opt-in presence log (names and times), one file a day
ALERTS = paths.HOME / "alerts.json"      # the tracker's alert inbox (alerts.py): alerts kept to look at later
DET_SCORE = 0.85         # detector confidence needed to keep a face
MIN_FACE = 40            # px, smaller faces are too blurry to recognise reliably
Q_ENROLL = 0.30          # faces below this eDifFIQA quality are not used to learn a person
Q_EXPR = 0.25            # faces below this quality aren't scored for expression (blur reads as "sad"/"neutral")
MAX_TURN = 0.35          # head turned more than this (nose offset / eye distance, about 35 degrees) isn't either

# Recognition models. Each has its own score scale, so its own settings (all measured on your labels):
#   match    cosine similarity needed to call a face a known person
#   cluster  cosine distance for grouping faces as one person
REC_MODELS = {
    "arcface_w600k_r50": dict(file="arcface_w600k_r50.onnx", dim=512, match=0.35, cluster=0.70),
    # ArcFace at 0.35: 100% of labeled faces correct, 0% of strangers named; on simulated blurry
    # webcam frames 76% correct vs 44% for SFace. Grouping at 0.70 put Chris in 2 groups instead of 4.
    "sface_2021dec": dict(file="face_recognition_sface_2021dec.onnx", dim=128, match=0.45, cluster=0.58),
    # SFace at 0.45: 99.3% correct, 0% of strangers named (0.40 named 4.4%).
}


LFS_MESSAGE = "Model files weren't downloaded: install Git LFS and run git lfs pull"


class NotDownloaded(SystemExit):
    """Files this needs are Git LFS placeholders (the project was copied without Git LFS)."""


def check_downloaded():
    """Stop with LFS_MESSAGE when a model or data file this program reads is a Git LFS placeholder:
    a small text file left where the real file should be when the project was copied without Git LFS.
    (Reading one would fail later with an error that doesn't say why.)"""
    from fetch_models import is_placeholder
    files = [DET_MODEL, QUAL_MODEL, EXPR_MODEL, CUE_MODEL, POSE_MODEL, VOICE_MODEL,
             *(MODELS / cfg["file"] for cfg in REC_MODELS.values()),
             DATA / "faces.npz", DATA / "people.npz", DATA / "photos.json"]
    missing = [p for p in files if is_placeholder(p)]
    if missing:
        models = any(p.parent == MODELS for p in missing)
        raise NotDownloaded(
            f"{LFS_MESSAGE}\n  Placeholders instead of: {', '.join(p.name for p in missing[:6])}"
            f"{' ...' if len(missing) > 6 else ''}"
            + ("\n  (For the models alone you can also run: python fetch_models.py)" if models else ""))


def active_model():
    """ArcFace when installed, else SFace. FACE_MODEL=sface_2021dec forces the old model."""
    forced = os.environ.get("FACE_MODEL")
    if forced:
        if forced not in REC_MODELS:
            sys.exit(f"Unknown FACE_MODEL {forced}; choose from {', '.join(REC_MODELS)}")
        return forced
    for name, cfg in REC_MODELS.items():
        if (MODELS / cfg["file"]).exists():
            return name
    raise NotDownloaded(f"No face recognition model found in {MODELS}. Run: python fetch_models.py")


# Checked when a program starts (require_models), not here: importing this module must work without the models
# (tests, CI), and a missing model should stop a program with a message that says what to do.
try:
    check_downloaded()
    REC_NAME = active_model()
    _not_ready = None
except NotDownloaded as e:
    REC_NAME, _not_ready = next(iter(REC_MODELS)), e


def require_models():
    """Stop with NotDownloaded (what to run) when the models or data files are missing or Git LFS placeholders.
    Every program calls this before loading anything; library code that only reads data doesn't need to."""
    if _not_ready is not None:
        raise _not_ready


REC = REC_MODELS[REC_NAME]
MATCH = REC["match"]
CLUSTER_DIST = REC["cluster"]

# 5-point template for 112x112 aligned faces (eyes, nose, mouth corners), shared by ArcFace and SFace
ARC_TEMPLATE = np.array([[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366],
                         [41.5493, 92.3655], [70.7299, 92.2041]], np.float32)


# Where the neural networks run. FACE_DEVICE = auto (default) | gpu | npu | cpu
#   auto Looks at the hardware and decides, per model, when the program starts:
#        - Windows, any DirectX 12 GPU: ONNX Runtime + DirectML (measured: same results, 3-4x faster than the CPU).
#        - NVIDIA card (CUDA, onnxruntime-gpu: setup.sh installs it when nvidia-smi works) or Apple Silicon
#          (CoreML): used only after a check at start (_gpu_check) shows the GPU gives the same results as the
#          CPU path (OpenCV) on a test input and is faster; otherwise the model stays on the CPU. The decision
#          is printed and shows in each program's list of which processor runs each model.
#        - otherwise the CPU (OpenCV).
#        The NPU is never chosen by itself: measured slower than the GPU and the CPU for one face at a time.
#   gpu  the GPU when there is one, without the check (CUDA and CoreML even when slower or different)
#   npu  ONNX Runtime + VitisAI (Ryzen AI NPU; run from the ryzen-ai conda env): low power, but slower; first
#        start compiles ~6 min
#   cpu  OpenCV DNN
# Measured on this PC (ArcFace, one face): GPU 2.6-3.6 ms, CPU 12 ms, NPU 25 ms.
DEVICE = os.environ.get("FACE_DEVICE", "auto").lower()


# One model run at a time on the GPU/NPU in this process. FaceRecognizer.exe runs the tracker and the
# labeling tool's photo/webcam import in one process; two threads running DirectML at once crashed the
# AMD driver (access violation in amdxc64.dll, 2026-11-08), and loading a model while another thread
# loaded or ran one crashed onnxruntime itself at start-up. Both are serialized here. A run takes
# ~3 ms, so waiting costs little.
ACCEL_LOCK = threading.Lock()

# GPUs that are checked at start before auto uses them (DirectML is measured, see DEVICE)
CHECKED_GPU = [("CUDAExecutionProvider", "CUDA"), ("CoreMLExecutionProvider", "CoreML")]
GPU_TOLERANCE = 0.01        # largest relative difference from the CPU's answer (|gpu - cpu| / |cpu|) to count as same
GPU_DECISIONS = {}          # model file -> what the check found, for the startup log


def _ort_runner(path, npu_static=None):
    """Return (run, device_name) for an ONNX model on the chosen device, or None to use OpenCV.
    Runs are serialized process-wide with ACCEL_LOCK."""
    if DEVICE == "cpu":
        return None
    try:
        import onnxruntime as ort
    except ImportError:
        return None
    avail = ort.get_available_providers()
    so = ort.SessionOptions()
    so.log_severity_level = 3
    if DEVICE == "npu" and "VitisAIExecutionProvider" in avail and npu_static:
        npu_dir = MODELS / "npu"
        cfg = npu_dir / "vitisai_config.json"
        if npu_static.exists() and cfg.exists():
            opts = {"config_file": str(cfg), "cache_dir": str(npu_dir / "cache"), "cache_key": npu_static.stem,
                    "enable_cache_file_io_in_mem": 0}
            with ACCEL_LOCK:                     # loading too: the app's two programs start at the same moment
                sess = ort.InferenceSession(str(npu_static), so, providers=["VitisAIExecutionProvider"],
                                            provider_options=[opts])
            return _locked(sess), "NPU (Ryzen AI)"
    if DEVICE not in ("auto", "gpu"):
        return None
    if "DmlExecutionProvider" in avail:
        with ACCEL_LOCK:                         # loading a model while another runs crashed onnxruntime too
            sess = ort.InferenceSession(str(path), so, providers=["DmlExecutionProvider", "CPUExecutionProvider"])
        return _locked(sess), "GPU (DirectML)"
    for provider, name in CHECKED_GPU:
        if provider not in avail:
            continue
        if provider == "CUDAExecutionProvider":
            _preload_cuda(ort)
        try:
            with ACCEL_LOCK:
                sess = ort.InferenceSession(str(path), so, providers=[provider, "CPUExecutionProvider"])
        except Exception as e:
            _decided(path, f"{name} couldn't load it ({str(e)[:120]}): CPU")
            continue
        if sess.get_providers()[0] != provider:              # the driver or libraries were missing
            _decided(path, f"{name} isn't working on this computer: CPU")
            continue
        if DEVICE == "gpu":
            _decided(path, f"{name}, asked for with FACE_DEVICE=gpu (not checked)")
            return _locked(sess), f"GPU ({name}, not checked)"
        x = _test_input(sess)
        if x is None:
            _decided(path, f"{name}: can't make a test input for this model: CPU")
            continue
        gpu = _locked(sess)
        ok, why = _gpu_check(gpu, _cv_runner(path), x)
        _decided(path, f"{name}: {why}: {'GPU' if ok else 'CPU'}")
        if ok:
            return gpu, f"GPU ({name}, checked)"
    return None


def _decided(path, text):
    GPU_DECISIONS[Path(path).name] = text
    print(f"{Path(path).name}: {text}", file=sys.stderr, flush=True)


def _test_input(sess):
    """A fixed test input for the model's first input (unknown sizes taken as 1), or None if it isn't float."""
    inp = sess.get_inputs()[0]
    if inp.type != "tensor(float)":
        return None
    shape = [d if isinstance(d, int) and d > 0 else 1 for d in inp.shape]
    return np.random.default_rng(0).uniform(-1, 1, shape).astype(np.float32)


def _gpu_check(gpu, cpu, x, runs=5):
    """Decide whether to use the GPU for one model: (use it, why). run(x) callables for each. Same results
    (within GPU_TOLERANCE of the CPU's) and faster (median of `runs`, after one run each to warm up)."""
    try:
        a = np.asarray(gpu(x), np.float64).ravel()
        b = np.asarray(cpu(x), np.float64).ravel()
    except Exception as e:
        return False, f"test run failed ({str(e)[:120]})"
    if a.shape != b.shape or not np.all(np.isfinite(a)):
        return False, "different results from the CPU"
    diff = float(np.linalg.norm(a - b) / max(np.linalg.norm(b), 1e-6))
    if diff > GPU_TOLERANCE:
        return False, f"different results from the CPU ({diff:.1%} apart)"

    def median_ms(run):
        times = []
        for _ in range(runs):
            t = time.perf_counter()
            run(x)
            times.append((time.perf_counter() - t) * 1000)
        return float(np.median(times))
    g, c = median_ms(gpu), median_ms(cpu)
    if g >= c:
        return False, f"same results but not faster ({g:.1f} ms vs {c:.1f} ms on the CPU)"
    return True, f"same results ({diff:.2%} apart), {g:.1f} ms vs {c:.1f} ms on the CPU"


_cuda_loaded = False


def _preload_cuda(ort):
    """onnxruntime-gpu[cuda,cudnn] (setup.sh, NVIDIA on Linux) brings CUDA and cuDNN as pip packages: load them
    before the first CUDA session, or it looks only in the system's library folders."""
    global _cuda_loaded
    if not _cuda_loaded and hasattr(ort, "preload_dlls"):
        _cuda_loaded = True
        try:
            ort.preload_dlls()
        except Exception:                         # a system-wide CUDA install still works without it
            pass


def _locked(sess):
    name = sess.get_inputs()[0].name

    def run(x):
        with ACCEL_LOCK:
            return sess.run(None, {name: x})[0]
    return run


def _cv_runner(path):
    """run(x) for an ONNX model on the CPU with OpenCV DNN."""
    net = cv2.dnn.readNetFromONNX(str(path))

    def run(x):
        net.setInput(x)
        return net.forward()
    return run


def align(bgr, row):
    """Rotate, scale and crop a detected face to the 112x112 template the recognizers expect."""
    src = np.asarray(row[4:14], np.float32).reshape(5, 2)
    m, _ = cv2.estimateAffinePartial2D(src, ARC_TEMPLATE, method=cv2.LMEDS)
    return cv2.warpAffine(bgr, m, (112, 112), borderValue=0)


# --------------------------------------------------------------------------- models
class Faces:
    def __init__(self):
        if not DET_MODEL.exists():
            sys.exit(f"Missing model file: {DET_MODEL}")
        self.det = cv2.FaceDetectorYN.create(str(DET_MODEL), "", (320, 320), DET_SCORE, 0.3, 5000)
        path = MODELS / REC["file"]
        if REC_NAME.startswith("sface"):
            sface = cv2.FaceRecognizerSF.create(str(path), "")
            self._feature = lambda crop: sface.feature(crop)
            self.device = "CPU (OpenCV)"
        else:
            run, self.device = (_ort_runner(path, MODELS / "npu" / (path.stem + "_b1.onnx"))
                                or (_cv_runner(path), "CPU (OpenCV)"))

            def arcface(crop):
                x = (cv2.cvtColor(crop, cv2.COLOR_BGR2RGB).astype(np.float32) - 127.5) / 127.5
                return run(np.ascontiguousarray(x.transpose(2, 0, 1)[None]))   # one face per call
            self._feature = arcface
        self._qrun, qdev = self._optional(QUAL_MODEL)    # quality and expression models: GPU or CPU
        self._xrun, xdev = self._optional(EXPR_MODEL)
        self.expression_model = EXPR_MODEL.name if self._xrun else None
        self._landmarker = None                          # MediaPipe, loaded on first use of cues()
        self.cues_available = CUE_MODEL.exists() and _has_module("mediapipe")
        self._poser = None                               # MediaPipe pose, loaded on first use of poses()
        self.pose_available = POSE_MODEL.exists() and _has_module("mediapipe")
        # Which processor runs each model, so a silent fall back to the CPU shows up in the startup log
        self.devices = {"detection": "CPU (OpenCV)", "recognition": self.device, "quality": qdev,
                        "expression": xdev, "cues": "CPU (MediaPipe)" if self.cues_available else None,
                        "pose": "CPU (MediaPipe)" if self.pose_available else None}

    @staticmethod
    def _optional(path):
        """(runner, device name) for an optional small ONNX model: ONNX Runtime on the GPU when
        available, else OpenCV on the CPU. (None, None) when the file isn't installed."""
        if not path.exists():
            return None, None
        return (_ort_runner(path) if DEVICE != "npu" else None) or (_cv_runner(path), "CPU (OpenCV)")

    def warmup(self):
        """Run each model once on a blank face so the first live frame isn't the slow one (GPU
        sessions compile shaders on first use). Returns {model: milliseconds} for that first run."""
        crop = np.zeros((112, 112, 3), np.uint8)
        steps = {"detection": lambda: self._detect_at(np.zeros((360, 640, 3), np.uint8), 640),
                 "recognition": lambda: self.embed(crop)}
        if self._qrun:
            steps["quality"] = lambda: self.quality(crop)
        if self._xrun:
            steps["expression"] = lambda: self.expression(crop)
        if self.cues_available:
            steps["cues"] = lambda: self.cues(np.zeros((360, 640, 3), np.uint8), [270, 130, 100, 100])
        if self.pose_available:
            steps["pose"] = lambda: self.poses(np.zeros((360, 640, 3), np.uint8))
        ms = {}
        for name, step in steps.items():
            t0 = time.perf_counter()
            step()
            ms[name] = round((time.perf_counter() - t0) * 1000, 1)
        return ms

    def embed(self, crop):
        """Face print of an aligned 112x112 crop, scaled to length 1 (so dot product = cosine similarity)."""
        v = np.asarray(self._feature(crop), np.float32).ravel()
        return v / (np.linalg.norm(v) + 1e-9)

    def quality(self, crop):
        """eDifFIQA quality of an aligned 112x112 face crop, roughly 0 (unusable) to 0.8 (excellent).
        Returns nan when the quality model isn't installed."""
        if self._qrun is None:
            return float("nan")
        x = cv2.cvtColor(cv2.resize(crop, (112, 112)), cv2.COLOR_BGR2RGB).astype(np.float32) / 255.0
        x = np.ascontiguousarray(np.moveaxis(((x - 0.5) / 0.5)[None], -1, 1))
        return float(np.asarray(self._qrun(x)).ravel()[0])

    def expression(self, crop):
        """Probabilities of the EXPRESSIONS (in that order) for an aligned 112x112 face crop, or None
        when the expression model isn't installed. The crop and its mirror image are both scored and
        the two averaged (test-time augmentation): a cheap, standard accuracy gain that also steadies
        the answer from frame to frame. Same alignment template and input scaling as ArcFace."""
        if self._xrun is None:
            return None
        x = (cv2.cvtColor(cv2.resize(crop, (112, 112)), cv2.COLOR_BGR2RGB).astype(np.float32) - 127.5) / 127.5
        x = x.transpose(2, 0, 1)[None]
        probs = np.zeros(len(EXPRESSIONS), np.float64)
        for view in (x, x[..., ::-1]):                      # the model takes one face per call
            logits = np.asarray(self._xrun(np.ascontiguousarray(view)), np.float64).ravel()
            e = np.exp(logits - logits.max())
            probs += e / e.sum()
        return probs / 2

    def blendshapes(self, bgr, box):
        """MediaPipe's 52 blendshape scores (0..1, by name) for the face in `box` (x, y, w, h), or
        None when MediaPipe or its model isn't installed or it finds no face there."""
        return self.face_mesh(bgr, box)[0]

    def face_mesh(self, bgr, box):
        """(blendshapes, points) for the face in `box` (x, y, w, h): MediaPipe's 52 blendshape
        scores (0..1, by name) and its 478 face landmarks as a 478x3 array of [x px, y px, depth px]
        in frame pixels (depth: smaller is nearer the camera). (None, None) when MediaPipe or its
        model isn't installed or it finds no face there. The landmarker gets a square region 1.6x
        the box, so the whole head is in view, resized to 256 px. MESH_TRIANGLES joins the points."""
        if not self.cues_available:
            return None, None
        if self._landmarker is None:
            from mediapipe.tasks.python import BaseOptions, vision
            self._landmarker = vision.FaceLandmarker.create_from_options(vision.FaceLandmarkerOptions(
                base_options=BaseOptions(model_asset_path=str(CUE_MODEL)),
                running_mode=vision.RunningMode.IMAGE, output_face_blendshapes=True, num_faces=1))
        import mediapipe as mp
        x, y, w, h = (float(v) for v in box[:4])
        side = max(w, h) * 1.6
        x0, y0 = int(max(0, x + w / 2 - side / 2)), int(max(0, y + h / 2 - side / 2))
        region = bgr[y0:int(y + h / 2 + side / 2), x0:int(x + w / 2 + side / 2)]
        if region.size == 0:
            return None, None
        rgb = cv2.cvtColor(cv2.resize(region, (256, 256), interpolation=cv2.INTER_AREA), cv2.COLOR_BGR2RGB)
        found = self._landmarker.detect(mp.Image(image_format=mp.ImageFormat.SRGB, data=np.ascontiguousarray(rgb)))
        if not found.face_blendshapes:
            return None, None
        rh, rw = region.shape[:2]
        points = np.array([[x0 + p.x * rw, y0 + p.y * rh, p.z * rw] for p in found.face_landmarks[0]], np.float32)
        return {c.category_name: float(c.score) for c in found.face_blendshapes[0]}, points

    def cues(self, bgr, box):
        """The face-shape CUES (smile, browDown, jaw...) for the face in `box`, or None."""
        bs = self.blendshapes(bgr, box)
        return None if bs is None else face_cues(bs)

    def poses(self, bgr):
        """Body poses in a frame (up to 4 people): a list of 33x3 arrays of MediaPipe pose landmarks
        as [x px, y px, visibility]. Empty when MediaPipe or its pose model isn't installed. About
        10-20 ms on the CPU for the whole frame."""
        if not self.pose_available:
            return []
        if self._poser is None:
            from mediapipe.tasks.python import BaseOptions, vision
            self._poser = vision.PoseLandmarker.create_from_options(vision.PoseLandmarkerOptions(
                base_options=BaseOptions(model_asset_path=str(POSE_MODEL)),
                running_mode=vision.RunningMode.IMAGE, num_poses=4))
        import mediapipe as mp
        h, w = bgr.shape[:2]
        rgb = np.ascontiguousarray(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB))
        found = self._poser.detect(mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb))
        return [np.array([[p.x * w, p.y * h, p.visibility if p.visibility is not None else 0.0] for p in lm], np.float32)
                for lm in found.pose_landmarks]

    def _detect_at(self, bgr, side):
        h, w = bgr.shape[:2]
        s = min(1.0, side / max(h, w))
        img = cv2.resize(bgr, (round(w * s), round(h * s)), interpolation=cv2.INTER_AREA) if s < 1 else bgr
        self.det.setInputSize((img.shape[1], img.shape[0]))
        _, rows = self.det.detect(img)
        if rows is None:
            return []
        rows = rows.copy()
        rows[:, :14] /= s                     # back to full-image coordinates
        return list(rows)

    def detect(self, bgr, sizes=(10_000, 640)):
        """Return list of (row, embedding, aligned_crop, quality). row = YuNet 15 values + score.

        By default runs the detector twice: at full size for small faces, and at 640 px for
        close-ups (selfies), which YuNet misses when a face fills most of a big frame.
        The live tracker passes sizes=(640,): webcam faces are large, so one pass is enough."""
        rows = [r for size in sizes for r in self._detect_at(bgr, size)]
        rows.sort(key=lambda r: -r[14])
        kept = []
        for r in rows:                         # drop duplicates found at both sizes
            if all(iou(r, k) < 0.4 for k in kept):
                kept.append(r)
        out = []
        for row in kept:
            if row[2] < MIN_FACE or row[3] < MIN_FACE:
                continue
            crop = align(bgr, row)
            out.append((row, self.embed(crop), crop, self.quality(crop)))
        return out


def head_turn(row):
    """How far a detected face is turned left or right, from its YuNet landmarks: the nose tip's
    sideways offset from the line through the eye midpoint and mouth midpoint, as a fraction of the
    distance between the eyes. About 0 facing the camera, about 0.3 at a 30 degree turn, 0.5 at 45."""
    k = np.asarray(row[4:14], np.float64).reshape(5, 2)    # eye, eye, nose, mouth corner, mouth corner
    eyes, nose, mouth = (k[0] + k[1]) / 2, k[2], (k[3] + k[4]) / 2
    eye_dist = np.hypot(*(k[1] - k[0])) + 1e-6
    return abs(nose[0] - (eyes[0] + mouth[0]) / 2) / eye_dist


def expression_skip(row, quality):
    """Why a detected face can't be read for expression: "blurry", "turned", or None if it can.
    Blur and profile views don't make the expression model unsure, they make it wrong (mostly
    "sad"/"neutral"), so such frames get no reading at all rather than a weak one."""
    if quality < Q_EXPR:                       # nan (no quality model) compares False: not skipped
        return "blurry"
    if head_turn(row) > MAX_TURN:
        return "turned"
    return None


def mesh_triangles():
    """MediaPipe's face-mesh tessellation as a flat list of landmark indices, three per triangle
    (852 triangles over the 468 face points; the 10 iris points aren't in it), or None without
    MediaPipe. For drawing the points Faces.face_mesh returns."""
    try:
        from mediapipe.tasks.python.vision import FaceLandmarksConnections
    except ImportError:
        return None
    # The edges come in threes, a->b, b->c, c->a, one triangle each, so the starts list the corners
    return [c.start for c in FaceLandmarksConnections.FACE_LANDMARKS_TESSELATION]


def face_cues(blendshapes):
    """Reduce MediaPipe's 52 blendshapes to the CUES, plus smileAsym (left/right smile difference:
    posed smiles tend to be less symmetric than spontaneous ones) and gaze (how far both eyes look
    away from straight ahead, in the strongest direction), each rounded to 2 decimals."""
    g = lambda n: blendshapes.get(n, 0.0)          # noqa: E731
    out = {cue: round(sum(g(n) for n in names) / len(names), 2) for cue, names in CUES.items()}
    out["smileAsym"] = round(abs(g("mouthSmileLeft") - g("mouthSmileRight")), 2)
    out["gaze"] = round(max((g(a) + g(b)) / 2 for a, b in GAZE), 2)
    return out


def pose_for_face(poses, box):
    """Index of the pose whose nose lies in the face box (x, y, w, h), nearest the box centre, or None."""
    x, y, w, h = (float(v) for v in box[:4])
    best, dist = None, None
    for i, p in enumerate(poses):
        nx, ny, vis = p[0]
        if vis < POSE_VIS or not (x - 0.25 * w <= nx <= x + 1.25 * w and y - 0.25 * h <= ny <= y + 1.25 * h):
            continue
        d = np.hypot(nx - (x + w / 2), ny - (y + h / 2))
        if dist is None or d < dist:
            best, dist = i, d
    return best


def hand_on_face(pose, box):
    """True when a seen wrist or index fingertip is over the face (box enlarged by 15% each side):
    a hand on the chin, cheek or mouth. It hides part of the face from the expression model.
    None when no hand point is seen."""
    x, y, w, h = (float(v) for v in box[:4])
    pts = [pose[i] for i in (15, 16, 19, 20) if pose[i][2] >= POSE_VIS]
    if not pts:
        return None
    return any(x - 0.15 * w <= px <= x + 1.15 * w and y - 0.15 * h <= py <= y + 1.15 * h for px, py, _ in pts)


def pose_points(pose):
    """The POSE_POINTS of one pose, as [[x, y, visibility], ...] rounded for sending."""
    return [[round(float(pose[i][0]), 1), round(float(pose[i][1]), 1), round(float(pose[i][2]), 2)]
            for i in POSE_POINTS.values()]


class Voice:
    """Wav2Small (audEERING; CC BY-NC-SA 4.0, non-commercial): arousal, dominance and valence of a
    voice, each about 0..1, from 16 kHz mono speech. 17K parameters, so it runs on the CPU in about a
    millisecond per second of audio. Optional: install it with tools/get_wav2small.py.
    Audio is only scored here, never written anywhere."""

    def __init__(self):
        self._run = None
        if VOICE_MODEL.exists() and _has_module("onnxruntime"):
            import onnxruntime as ort
            so = ort.SessionOptions()
            so.log_severity_level = 3
            sess = ort.InferenceSession(str(VOICE_MODEL), so, providers=["CPUExecutionProvider"])
            self._run = lambda x: sess.run(None, {"audio": x})[0]
        self.model = VOICE_MODEL.name if self._run else None
        self.device = "CPU (ONNX Runtime)" if self._run else None

    def avd(self, samples):
        """[arousal, dominance, valence] for float32 samples at VOICE_RATE, or None without the model."""
        if self._run is None:
            return None
        x = np.ascontiguousarray(np.asarray(samples, np.float32)[None])
        return [float(v) for v in np.asarray(self._run(x)).ravel()[:3]]


def _has_module(name):
    import importlib.util
    return importlib.util.find_spec(name) is not None


def iou(a, b):
    """Overlap of two face boxes (x, y, w, h): intersection over union."""
    x1, y1 = max(a[0], b[0]), max(a[1], b[1])
    x2, y2 = min(a[0] + a[2], b[0] + b[2]), min(a[1] + a[3], b[1] + b[3])
    inter = max(0, x2 - x1) * max(0, y2 - y1)
    return inter / (a[2] * a[3] + b[2] * b[3] - inter + 1e-9)


# --------------------------------------------------------------------------- images
_heif_ready = None


def load_image(path):
    """Load any photo as BGR, upright (EXIF rotation applied), shrunk to MAX_SIDE."""
    global _heif_ready
    from PIL import Image, ImageOps
    ext = Path(path).suffix.lower()
    if ext in (".heic", ".heif") and _heif_ready is None:
        try:
            import pillow_heif
            pillow_heif.register_heif_opener()
            _heif_ready = True
        except ImportError:
            _heif_ready = False
            print("  note: install pillow-heif to read .heic photos (pip install pillow-heif)")
    if ext in (".heic", ".heif") and not _heif_ready:
        return None
    try:
        with Image.open(path) as im:
            im = ImageOps.exif_transpose(im)
            im = im.convert("RGB")
            scale = MAX_SIDE / max(im.size)
            if scale < 1:
                im = im.resize((round(im.width * scale), round(im.height * scale)), Image.BILINEAR)
            return cv2.cvtColor(np.asarray(im), cv2.COLOR_RGB2BGR)
    except Exception as e:
        print(f"  skip {Path(path).name}: {e}")
        return None


# --------------------------------------------------------------------------- videos
VIDEO_EVERY = 1.0        # seconds between the video frames looked at
VIDEO_FRAMES = 300       # most frames looked at in one video (a long video's are spread over all of it)
VIDEO_KEEP = 3           # faces kept per person per video: the clearest ones ...
VIDEO_GAP = 3.0          # ... at least this many seconds apart


def _fit(frame):
    """Shrink a video frame to MAX_SIDE, as photos are (face boxes are stored at this scale)."""
    h, w = frame.shape[:2]
    s = MAX_SIDE / max(h, w)
    return cv2.resize(frame, (round(w * s), round(h * s)), interpolation=cv2.INTER_AREA) if s < 1 else frame


def video_frames(path, stop=None):
    """Yield (frame number, frames per second, BGR frame shrunk to MAX_SIDE) about every VIDEO_EVERY
    seconds, at most VIDEO_FRAMES of them. Raises OSError if the video can't be opened."""
    cap = cv2.VideoCapture(str(path))
    if not cap.isOpened():
        raise OSError(f"can't open video {Path(path).name}")
    try:
        fps = cap.get(cv2.CAP_PROP_FPS)
        fps = fps if 0 < fps < 1000 else 30.0
        n = int(cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0)
        step = max(1, round(fps * VIDEO_EVERY), n // VIDEO_FRAMES)
        k = 0
        while not (stop and stop.is_set()):
            if k % step == 0:
                ok, frame = cap.read()
                if not ok or frame is None:
                    break
                yield k, fps, _fit(frame)
            elif not cap.grab():               # skipped frames are decoded but not converted: fast
                break
            k += 1
    finally:
        cap.release()


def video_frame(path, k):
    """Frame number k of a video, shrunk to MAX_SIDE like video_frames' (None if it can't be read)."""
    cap = cv2.VideoCapture(str(path))
    try:
        if not cap.isOpened():
            return None
        cap.set(cv2.CAP_PROP_POS_FRAMES, int(k))
        ok, frame = cap.read()
        return _fit(frame) if ok and frame is not None else None
    finally:
        cap.release()


def video_faces(faces, path, stop=None):
    """The faces in a video worth keeping. Frames are looked at about once a second; the same person's
    faces across the video are grouped and only that person's VIDEO_KEEP clearest faces, VIDEO_GAP
    seconds apart, are kept, so a long clip of one person adds a few faces, not hundreds.
    Same person: a face where theirs was in the frame looked at just before (head turned down or
    away, the face print alone often doesn't match), else face prints at least MATCH alike (the
    recognizer's own setting). Returns (fps, [(frame number, row, emb, crop, q)])."""
    people, fps, prev = [], 30.0, None           # [their face prints, [(k, row, emb, crop, q)]]
    for k, fps, frame in video_frames(path, stop):     # noqa: B007 (fps is returned after the loop)
        taken = set()                             # one face per person per frame
        for row, emb, crop, q in faces.detect(frame):
            best, best_s = None, 0.3              # 1. followed: same place as in the previous frame
            for n, p in enumerate(people):
                last_k, last_row = p[1][-1][0], p[1][-1][1]
                if n not in taken and last_k == prev and (s := iou(row, last_row)) >= best_s:
                    best, best_s = n, s
            if best is None:                      # 2. recognized: alike any face of theirs seen so far
                best_s = MATCH
                for n, p in enumerate(people):
                    if n not in taken and (s := float((p[0] @ emb).max())) >= best_s:
                        best, best_s = n, s
            if best is None:
                people.append([emb[None], []])
                best = len(people) - 1
            else:
                people[best][0] = np.vstack([people[best][0], emb])
            taken.add(best)
            people[best][1].append((k, row, emb, crop, q))
        prev = k
    out = []
    for _, seen in people:
        seen.sort(key=lambda r: -(r[4] if r[4] == r[4] else 0.0))      # clearest first (nan: unknown)
        kept = []
        for r in seen:
            if all(abs(r[0] - o[0]) >= VIDEO_GAP * fps for o in kept):
                kept.append(r)
                if len(kept) == VIDEO_KEEP:
                    break
        out += sorted(kept, key=lambda r: r[0])
    return fps, out


def list_media(folder, exts):
    folder = Path(folder)
    if folder.is_file():
        return [folder]
    return sorted(p for p in folder.rglob("*") if p.suffix.lower() in exts and p.is_file())


# --------------------------------------------------------------------------- store
def face_key(photo, row, frame=None):
    """Permanent ID of a face: its photo's content fingerprint + where the face's centre is
    (+ the frame number, for a face from a video)."""
    key = f"{photo[:16]}-{round(float(row[0]) + float(row[2]) / 2)}-{round(float(row[1]) + float(row[3]) / 2)}"
    return key if frame is None else f"{key}-f{int(frame)}"


class Store:
    """data/faces.npz holds every face found by a scan: face print, permanent face ID, source photo,
    box and quality. data/thumbs.npz holds each face's picture (the 112x112 aligned crop, JPEG, all packed
    into one byte array, with the face IDs they belong to). The pictures are kept apart because they stay on
    this computer (all of data/ is in .gitignore); when thumbs.npz is missing or lacks a face, thumb(i) is
    None, and rebuild_crop() can make one again from the original photo when it's on the computer. Databases from before 2026-11-09 kept the pictures in
    faces.npz; they're moved to thumbs.npz on the next save.
    data/photos.json describes each photo read: {content hash: {path, size, mtime, fp, faces, dup_of}}."""

    def __init__(self):
        DATA.mkdir(exist_ok=True)
        self.path = DATA / "faces.npz"
        self.thumbs_path = DATA / "thumbs.npz"
        self.photos_path = DATA / "photos.json"
        self.fid, self.photo = [], []
        self.thumbs_changed = False             # thumbs.npz needs writing on the next save
        if self.path.exists():
            z = np.load(self.path, allow_pickle=False)
            self.emb = list(z["emb"])
            self.file = [str(f) for f in z["files"]]
            self.box = list(z["box"])
            self.quality = list(z["quality"]) if "quality" in z.files else [float("nan")] * len(self.emb)
            # databases from before model tracking were all made with SFace
            self.model = str(z["model"]) if "model" in z.files else "sface_2021dec"
            if "fid" in z.files:                 # older databases get IDs from labels.migrate()
                self.fid = [str(f) for f in z["fid"]]
                self.photo = [str(p) for p in z["photo"]]
            if "thumbs" in z.files:              # before 2026-11-09: pictures inside faces.npz
                jpg, off = z["thumbs"], z["thumb_off"]
                self.thumbs = [jpg[off[i]:off[i + 1]].tobytes() for i in range(len(off) - 1)]
                self.thumbs_changed = True
            else:
                self.thumbs = self._load_thumbs()
        else:
            self.emb, self.file, self.box, self.thumbs, self.quality = [], [], [], [], []
            self.model = REC_NAME
        self.photos = load_json("photos.json", {})
        self._fids = set(self.fid)

    def _load_thumbs(self):
        """Face pictures from thumbs.npz, matched to the faces by face ID (b"" where there's none)."""
        n = len(self.emb)
        try:
            z = np.load(self.thumbs_path, allow_pickle=False)
            jpg, off = z["thumbs"], z["thumb_off"]
            got = [jpg[off[i]:off[i + 1]].tobytes() for i in range(len(off) - 1)]
            if "fid" in z.files and self.fid:
                by = dict(zip((str(f) for f in z["fid"]), got))
                return [by.get(f, b"") for f in self.fid]
            return (got + [b""] * n)[:n]
        except (OSError, ValueError, KeyError):
            return [b""] * n

    @property
    def needs_ids(self):
        return bool(self.emb) and not self.fid

    def claim_fid(self, photo, row, frame=None):
        """Record the permanent ID of a new face in `photo` (or in a video's frame) and return it."""
        fid = face_key(photo, row, frame)
        while fid in self._fids:                 # practically never: two faces centred on the same pixel
            fid += "x"
        self._fids.add(fid)
        self.fid.append(fid)
        self.photo.append(photo)
        return fid

    def add(self, photo, file, row, emb, crop, quality=float("nan"), frame=None):
        """Append one face (frame: its frame number, for a face from a video); returns its index."""
        self.claim_fid(photo, row, frame)
        self.quality.append(quality)
        self.emb.append(emb)
        self.file.append(file)
        self.box.append(np.asarray(row[:5], np.float32))
        self.thumbs.append(cv2.imencode(".jpg", crop, [cv2.IMWRITE_JPEG_QUALITY, 88])[1].tobytes())
        self.thumbs_changed = True
        return len(self.emb) - 1

    def thumb(self, i):
        """Face i's picture (112x112 BGR), or None when there's none on this computer."""
        t = self.thumbs[i]
        return cv2.imdecode(np.frombuffer(t, np.uint8), cv2.IMREAD_COLOR) if t else None

    def set_thumb(self, i, crop):
        self.thumbs[i] = cv2.imencode(".jpg", crop, [cv2.IMWRITE_JPEG_QUALITY, 88])[1].tobytes()
        self.thumbs_changed = True

    def missing_thumbs(self):
        return sum(1 for i, t in enumerate(self.thumbs) if not t and not self.printless(i))

    def printless(self, i):
        """True if face i's print and picture were deleted (forget): an Ignored face, or a forgotten person's."""
        return not np.any(self.emb[i])

    def forget(self, i):
        """Delete face i's print and picture; keep only where it is (face ID, photo, box, quality), so the photo
        isn't read again and the face doesn't come back as a new face. Returns (print, picture) for an undo."""
        old = (np.asarray(self.emb[i], np.float32).copy(), self.thumbs[i])
        self.emb[i] = np.zeros_like(old[0])
        if self.thumbs[i]:
            self.thumbs[i] = b""
            self.thumbs_changed = True
        return old

    def restore(self, i, saved):
        """Put back a print and picture that forget() returned (Undo in the same session)."""
        self.emb[i], self.thumbs[i] = saved[0], saved[1]
        self.thumbs_changed = True

    def save(self):
        if self.thumbs_changed or not self.thumbs_path.exists():
            # the pictures first: faces.npz without them is only written once they're safe in thumbs.npz
            off = np.cumsum([0] + [len(t) for t in self.thumbs]).astype(np.int64)
            tmp = DATA / "thumbs.tmp.npz"
            np.savez(tmp, fid=np.asarray(self.fid, dtype=str),
                     thumbs=np.frombuffer(b"".join(self.thumbs), np.uint8), thumb_off=off)
            replace_file(tmp, self.thumbs_path)
            self.thumbs_changed = False
        tmp = DATA / "faces.tmp.npz"
        np.savez(tmp,
                 emb=np.asarray(self.emb, np.float32).reshape(len(self.emb), -1),
                 model=np.asarray(self.model),
                 files=np.asarray(self.file, dtype=str),
                 fid=np.asarray(self.fid, dtype=str),
                 photo=np.asarray(self.photo, dtype=str),
                 box=np.asarray(self.box, np.float32).reshape(-1, 5),
                 quality=np.asarray(self.quality, np.float32))
        replace_file(tmp, self.path)       # never leave a half-written database
        save_json("photos.json", self.photos)


replace_file = paths.replace_file      # os.replace with retries (Windows: dest briefly open elsewhere)


def load_json(name, default):
    p = DATA / name
    return json.loads(p.read_text("utf-8")) if p.exists() else default


def save_json(name, obj):
    paths.atomic_write(DATA / name, json.dumps(obj, indent=1))     # never leave a half-written file


REBUILD_IOU = 0.6                 # a face found again must cover its saved box this well


def rebuild_crop(store, i, faces=None):
    """Face i's aligned 112x112 crop made again from its original photo (or video frame), for a face whose
    picture isn't on this computer: the face is detected again near its saved box (the box alone has no eye
    and mouth points to align with). None if the photo isn't here or the face isn't found again."""
    if not store.photo:
        return None
    rec = store.photos.get(store.photo[i], {})
    path = rec.get("path")
    if not (path and Path(path).exists()):
        return None
    frame = rec.get("at", {}).get(store.fid[i]) if rec.get("video") else None
    img = load_image(path) if frame is None else video_frame(path, frame)
    if img is None:
        return None
    x, y, w, h = (float(v) for v in store.box[i][:4])
    m = max(w, h) * 0.6                          # look in the box with some room around it
    x0, y0 = max(0, int(x - m)), max(0, int(y - m))
    x1, y1 = min(img.shape[1], int(x + w + m)), min(img.shape[0], int(y + h + m))
    if x1 - x0 < 8 or y1 - y0 < 8:
        return None
    part = img[y0:y1, x0:x1]
    s = 1.0 if max(part.shape[:2]) >= 160 else 160 / max(part.shape[:2])   # small faces: enlarge to find them
    if s != 1.0:
        part = cv2.resize(part, None, fx=s, fy=s, interpolation=cv2.INTER_CUBIC)
    det = cv2.FaceDetectorYN.create(str(DET_MODEL), "", (part.shape[1], part.shape[0]), 0.3, 0.3, 50)
    _, rows = det.detect(part)
    if rows is None or not len(rows):
        return None
    bx, by, bw, bh = (x - x0) * s, (y - y0) * s, w * s, h * s

    def overlap(r):                              # intersection over union with the saved box
        iw = max(0.0, min(bx + bw, r[0] + r[2]) - max(bx, r[0]))
        ih = max(0.0, min(by + bh, r[1] + r[3]) - max(by, r[1]))
        return iw * ih / (bw * bh + r[2] * r[3] - iw * ih)
    row = max(rows, key=overlap).copy()
    if overlap(row) < REBUILD_IOU:
        return None                              # the face found isn't this one
    row[:14] /= s
    row[[0, 4, 6, 8, 10, 12]] += x0               # x of the box and of the 5 points (2, 3: width, height)
    row[[1, 5, 7, 9, 11, 13]] += y0
    return align(img, row)


# --------------------------------------------------------------------------- scan
def require_current(store):
    """Face prints from different models can't be compared; refuse to mix them."""
    if store.emb and store.model != REC_NAME:
        sys.exit(f"The face database was made with {store.model}, but the active model is {REC_NAME}.\n"
                 f"Run:  python face_id.py reembed")


def cmd_reembed(a):
    """Recompute every stored face print with the active model, from the saved face crops."""
    faces, store = Faces(), Store()
    if not store.emb:
        sys.exit("No faces yet. Run scan first.")
    t0 = time.time()
    lost = []
    for i in range(len(store.emb)):
        if store.printless(i):                   # deleted on purpose (Ignored or forgotten): stays deleted
            continue
        crop = store.thumb(i)
        if crop is None:                         # no picture on this computer: make it again from the photo
            crop = rebuild_crop(store, i)
            if crop is None:
                lost.append(i)
                continue
            store.set_thumb(i, crop)
        store.emb[i] = faces.embed(crop)
        if np.isnan(store.quality[i]):
            store.quality[i] = faces.quality(crop)
        if (i + 1) % 100 == 0:
            print(f"  {i + 1}/{len(store.emb)} faces")
    if lost:
        sys.exit(f"{len(lost)} faces have no saved picture and their photos aren't on this computer (or the face\n"
                 f"wasn't found again), so their face prints can't be remade with {REC_NAME}. Nothing was saved.\n"
                 f"Copy data\\thumbs.npz from the computer the faces were found on, or keep the current model.")
    old, store.model = store.model, REC_NAME
    store.save()
    build_people()
    print(f"Re-embedded {len(store.emb)} faces with {REC_NAME} (was {old}) in {time.time() - t0:.0f}s. Your labels are kept.")


def cmd_migrate(a):
    """Upgrade an older database: permanent face IDs, per-face labels, photo paths and fingerprints."""
    import labels
    store = Store()
    if not store.needs_ids:
        print("The face database is already up to date.")
        return
    labels.migrate(store, a.folders or labels.settings()["folders"])


def cmd_scan(a):
    """Read new photos (by content, so renamed or copied photos aren't read twice), find faces,
    and name the ones the model is sure about. Same as "Add photos" on the labeling page."""
    import labels
    db = labels.LabelDB(backup=False)
    imp = labels.Importer(db)
    t = threading.Thread(target=imp.run, args=(Path(a.folder),), daemon=True)
    t.start()
    while t.is_alive():
        t.join(2)
        st = imp.status
        if st.get("total"):
            print(f"  {st['stage']}: {st['done']}/{st['total']} photos, {st['faces']} faces, "
                  f"{st['duplicates']} duplicates skipped", flush=True)
    st = imp.status
    if st.get("error"):
        sys.exit("Scan failed: " + st["error"])
    print(f"Done. {st['new']} new photos, {st['faces']} faces found, {st['auto']} named automatically, "
          f"{st['known']} already read, {st['duplicates']} duplicates skipped, {st['errors']} unreadable.")
    print("Next: python label_tool.py   (confirm suggestions and name new people)")


# --------------------------------------------------------------------------- cluster
def cmd_cluster(a):
    """Group the faces nobody has named into look-alike groups and write a contact sheet for each."""
    import labels
    db = labels.LabelDB(backup=False)
    groups = db.unknown(show_low=True, regroup=True)["groups"]
    save_json("unknown_groups.json", [[db.store.fid[i] for i, _ in g["faces"]] for g in groups])
    sheets = DATA / "sheets"
    sheets.mkdir(exist_ok=True)
    for old in sheets.glob("group_*.jpg"):
        old.unlink()
    shown = 0
    for g in groups:
        if len(g["faces"]) < a.min:
            break
        contact_sheet(g["id"], [i for i, _ in g["faces"]], db.store, sheets / f"group_{g['id']:03d}.jpg")
        shown += 1
    print(f"{len(groups)} groups of unnamed faces; {shown} with {a.min}+ faces have contact sheets in {sheets}")
    print("Next: python face_id.py name <group> <Name>   (or use label_tool.py)")


def contact_sheet(gid, members, store, out, cols=8, size=112, max_faces=40):
    # spread the sample across the group so the sheet shows variety
    pick = members if len(members) <= max_faces else [members[int(j)] for j in np.linspace(0, len(members) - 1, max_faces)]
    rows = (len(pick) + cols - 1) // cols
    head = 40
    sheet = np.full((head + rows * size, cols * size, 3), 245, np.uint8)
    cv2.putText(sheet, f"Group {gid}  ({len(members)} faces)", (8, 28), cv2.FONT_HERSHEY_SIMPLEX, 0.8, (30, 30, 30), 2, cv2.LINE_AA)
    for k, idx in enumerate(pick):
        crop = store.thumb(idx)
        if crop is None:
            continue
        r, c = divmod(k, cols)
        sheet[head + r * size: head + (r + 1) * size, c * size:(c + 1) * size] = cv2.resize(crop, (size, size))
    cv2.imwrite(str(out), sheet, [cv2.IMWRITE_JPEG_QUALITY, 85])


# --------------------------------------------------------------------------- name
def cmd_name(a):
    """Name groups from the last `cluster` run."""
    import labels
    groups = load_json("unknown_groups.json", None)
    if groups is None:
        sys.exit("Run cluster first.")
    if not a.ignore and not a.name:
        sys.exit("Give a name, or --ignore.")
    db = labels.LabelDB(backup=False)
    idx_of = {f: i for i, f in enumerate(db.store.fid)}
    idx = [idx_of[f] for g in a.groups if 1 <= g <= len(groups) for f in groups[g - 1] if f in idx_of]
    db.act("ignore" if a.ignore else "name", idx, a.name)
    cmd_people(a)


def build_people():
    """people.npz = face prints of every named, usable face, used for recognition."""
    import labels
    return labels.LabelDB(backup=False).build_people()


def cmd_people(a):
    p = DATA / "people.npz"
    if not p.exists():
        print("Nobody named yet.")
        return
    z = np.load(p)
    names, counts = np.unique(z["name"], return_counts=True)
    print("Known people:")
    for n, c in sorted(zip(names, counts), key=lambda x: -x[1]):
        print(f"  {n:<20} {c} face prints")


# --------------------------------------------------------------------------- recognize
class Identifier:
    """Read-only after construction, so one instance can be shared safely between threads."""

    def __init__(self, emb, names, threshold=MATCH):
        self.names = sorted(set(names))
        names = np.asarray(names)
        self.gallery = {n: emb[names == n] for n in self.names}     # per person: all their face prints
        self.threshold = threshold

    @classmethod
    def load(cls, threshold=MATCH, path=None):
        """Load people.npz; returns None if nobody has been named yet."""
        p = path or DATA / "people.npz"
        if not p.exists():
            return None
        z = np.load(p)
        if not len(z["name"]):
            return None
        made_with = str(z["model"]) if "model" in z.files else "sface_2021dec"
        if made_with != REC_NAME:
            print(f"people.npz was made with {made_with}, not {REC_NAME}. Run: python face_id.py reembed")
            return None
        return cls(z["emb"], z["name"].tolist(), threshold)

    def who(self, emb, threshold=None):
        threshold = self.threshold if threshold is None else threshold
        best, best_s = "Unknown", -1.0
        for n, g in self.gallery.items():
            sims = g @ emb
            k = min(5, len(sims))
            s = float(np.sort(sims)[-k:].mean())   # mean of the 5 closest prints: robust to one odd photo
            if s > best_s:
                best, best_s = n, s
        return (best if best_s >= threshold else "Unknown"), best_s


def draw(img, row, name, score):
    x, y, w, h = [int(v) for v in row[:4]]
    color = (60, 200, 90) if name != "Unknown" else (80, 80, 230)
    cv2.rectangle(img, (x, y), (x + w, y + h), color, 2)
    label = f"{name} {score:.2f}"
    (tw, th), _ = cv2.getTextSize(label, cv2.FONT_HERSHEY_SIMPLEX, 0.6, 2)
    cv2.rectangle(img, (x, y - th - 10), (x + tw + 8, y), color, -1)
    cv2.putText(img, label, (x + 4, y - 6), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 0, 0), 2, cv2.LINE_AA)


def cmd_recognize(a):
    faces, ident = Faces(), Identifier.load(a.threshold)
    if ident is None:
        sys.exit("Nobody named yet. Run scan, cluster and name (or label_tool.py) first.")
    print("Known:", ", ".join(ident.names))
    if a.camera is not None:
        return run_camera(faces, ident, a.camera)
    if not a.path:
        sys.exit("Give --camera N or a photo/folder path.")
    out = Path(a.out) if a.out else None
    if out:
        out.mkdir(parents=True, exist_ok=True)
    for f in list_media(a.path, IMAGE_EXT | VIDEO_EXT):
        if f.suffix.lower() in VIDEO_EXT:
            run_video(faces, ident, f)
            continue
        img = load_image(f)
        if img is None:
            continue
        found = []
        for row, emb, *_ in faces.detect(img):
            name, s = ident.who(emb)
            found.append(f"{name} ({s:.2f})")
            draw(img, row, name, s)
        print(f"{f.name}: {', '.join(found) if found else 'no faces'}")
        if out and found:
            cv2.imwrite(str(out / (f.stem + ".jpg")), img)


def run_video(faces, ident, f):
    """Count who appears in a video file (every 10th frame); annotated copies are photos only."""
    cap = cv2.VideoCapture(str(f))
    seen, n = {}, 0
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        n += 1
        if n % 10:
            continue                      # every 10th frame is plenty
        for _row, emb, *_ in faces.detect(frame):
            name, _ = ident.who(emb)
            seen[name] = seen.get(name, 0) + 1
    cap.release()
    print(f"{f.name}: " + (", ".join(f"{k} ({v} frames)" for k, v in seen.items()) or "no faces"))


def open_camera(index):
    """Open a webcam, trying each Windows camera system in turn (some USB webcams only work with one)."""
    backends = [cv2.CAP_DSHOW, cv2.CAP_MSMF, cv2.CAP_ANY] if os.name == "nt" else [cv2.CAP_ANY]
    for be in backends:
        cap = cv2.VideoCapture(index, be)
        if not cap.isOpened():
            cap.release()
            continue
        cap.set(cv2.CAP_PROP_FRAME_WIDTH, 1280)
        cap.set(cv2.CAP_PROP_FRAME_HEIGHT, 720)
        for _ in range(10):                   # some webcams return a few empty frames while starting
            ok, frame = cap.read()
            if ok and frame is not None and frame.size:
                return cap
            time.sleep(0.05)
        cap.release()
    return None


def run_camera(faces, ident, index):
    """Live webcam view. Keys: C = next camera, M = mirror on/off, Q or Esc = quit."""
    cap = open_camera(index)
    if cap is None:
        for i in range(5):                     # requested camera missing: use the first one that works
            if i != index and (cap := open_camera(i)) is not None:
                print(f"Camera {index} didn't open; using camera {i}.")
                index = i
                break
    if cap is None:
        sys.exit("No camera opened. Check the webcam is plugged in and not in use by another app (Teams, Zoom, a browser tab).")
    print("Keys: C = switch camera, M = mirror, Q or Esc = quit.")
    win, mirror = "Face ID", True
    cv2.namedWindow(win, cv2.WINDOW_NORMAL)
    cv2.resizeWindow(win, 1280, 720)
    t, fps = time.time(), 0.0
    while True:
        ok, frame = cap.read()
        if not ok:
            print(f"Camera {index} stopped sending video.")
            break
        if mirror:
            frame = cv2.flip(frame, 1)
        for row, emb, *_ in faces.detect(frame, sizes=(640,)):
            name, s = ident.who(emb)
            draw(frame, row, name, s)
        now = time.time()
        fps = 0.9 * fps + 0.1 / max(now - t, 1e-6)
        t = now
        info = f"Camera {index}  |  {fps:.0f} fps  |  C: switch camera   M: mirror   Q: quit"
        cv2.putText(frame, info, (10, 28), cv2.FONT_HERSHEY_SIMPLEX, 0.65, (0, 0, 0), 4, cv2.LINE_AA)
        cv2.putText(frame, info, (10, 28), cv2.FONT_HERSHEY_SIMPLEX, 0.65, (255, 255, 255), 1, cv2.LINE_AA)
        cv2.imshow(win, frame)
        key = cv2.waitKey(1) & 0xFF
        if key in (27, ord("q")) or cv2.getWindowProperty(win, cv2.WND_PROP_VISIBLE) < 1:
            break
        if key == ord("m"):
            mirror = not mirror
        if key == ord("c"):
            cap.release()
            for step in range(1, 6):          # next camera that works, wrapping around
                nxt = (index + step) % 5
                if (new := open_camera(nxt)) is not None:
                    cap, index = new, nxt
                    break
            else:
                cap = open_camera(index)
            print(f"Using camera {index}.")
    cap.release()
    cv2.destroyAllWindows()


# --------------------------------------------------------------------------- cli
def main():
    require_models()
    p = argparse.ArgumentParser(description="Recognize specific people from your photos.")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("reembed", help="recompute all face prints after switching recognition model")
    s.set_defaults(fn=cmd_reembed)

    s = sub.add_parser("migrate", help="upgrade an older face database (permanent face IDs, per-face labels)")
    s.add_argument("folders", nargs="*", help="photo folders scanned before, to find each photo again")
    s.set_defaults(fn=cmd_migrate)

    s = sub.add_parser("scan", help="find faces in a photo folder (repeatable; only new photos are read)")
    s.add_argument("folder")
    s.set_defaults(fn=cmd_scan)

    s = sub.add_parser("cluster", help="group unnamed look-alike faces and write contact sheets")
    s.add_argument("--min", type=int, default=4, help="smallest group to make a sheet for")
    s.set_defaults(fn=cmd_cluster)

    s = sub.add_parser("name", help="name one or more groups")
    s.add_argument("args", nargs="+", metavar="GROUP... NAME")
    s.add_argument("--ignore", action="store_true", help="mark groups as not someone to recognize")
    s.set_defaults(fn=cmd_name)

    s = sub.add_parser("people", help="list known people")
    s.set_defaults(fn=cmd_people)

    s = sub.add_parser("recognize", help="name people on a webcam or in photos/videos")
    s.add_argument("path", nargs="?")
    s.add_argument("--camera", type=int, help="webcam index (0, 1, 2 ...)")
    s.add_argument("--out", help="folder for annotated copies of photos")
    s.add_argument("--threshold", type=float, default=MATCH, help="higher = fewer false matches (default %(default)s)")
    s.set_defaults(fn=cmd_recognize)

    a = p.parse_args()
    if a.cmd == "name":   # `name 3 7 Chris Smith` -> groups [3, 7], name "Chris Smith"
        nums = [x for x in a.args if x.isdigit()]
        a.groups = [int(x) for x in nums]
        a.name = " ".join(x for x in a.args if not x.isdigit()) or None
        if not a.groups:
            sys.exit("Give at least one group number.")
    a.fn(a)


if __name__ == "__main__":
    main()
