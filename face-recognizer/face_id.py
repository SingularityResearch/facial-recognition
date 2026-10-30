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
After switching recognition models, run:  python face_id.py reembed

Examples
  python face_id.py scan "E:\\Users\\Chris\\iCloud\\Photos"
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
import time
from pathlib import Path

import cv2
import numpy as np

HERE = Path(__file__).resolve().parent
MODELS = HERE / "models"
DATA = HERE / "data"
DET_MODEL = MODELS / "face_detection_yunet_2023mar.onnx"
QUAL_MODEL = MODELS / "ediffiqa_tiny_jun2024.onnx"            # eDifFIQA face-quality model (optional)
EXPR_MODEL = MODELS / "facial_expression_recognition_mobilefacenet_2022july.onnx"   # expression model (optional)
EXPRESSIONS = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"]   # its outputs, in order

IMAGE_EXT = {".jpg", ".jpeg", ".png", ".webp", ".bmp", ".heic", ".heif", ".gif", ".tif", ".tiff"}
VIDEO_EXT = {".mov", ".mp4", ".m4v", ".avi", ".mkv"}

MAX_SIDE = 1280          # photos are shrunk to this before detection (speed)
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


def _join_parts(path):
    """Large models ship split into .part0, .part1 ... files; rejoin them once."""
    parts = sorted(path.parent.glob(path.name + ".part*"), key=lambda p: int(p.suffix[5:]))
    if not path.exists() and parts:
        tmp = path.with_suffix(".joining")
        with open(tmp, "wb") as out:
            for p in parts:
                out.write(p.read_bytes())
        os.replace(tmp, path)
    return path.exists()


def active_model():
    """ArcFace when installed, else SFace. FACE_MODEL=sface_2021dec forces the old model."""
    forced = os.environ.get("FACE_MODEL")
    if forced:
        if forced not in REC_MODELS:
            sys.exit(f"Unknown FACE_MODEL {forced}; choose from {', '.join(REC_MODELS)}")
        return forced
    for name, cfg in REC_MODELS.items():
        if _join_parts(MODELS / cfg["file"]):
            return name
    sys.exit("No face recognition model found in the models folder.")


REC_NAME = active_model()
REC = REC_MODELS[REC_NAME]
MATCH = REC["match"]
CLUSTER_DIST = REC["cluster"]

# 5-point template for 112x112 aligned faces (eyes, nose, mouth corners), shared by ArcFace and SFace
ARC_TEMPLATE = np.array([[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366],
                         [41.5493, 92.3655], [70.7299, 92.2041]], np.float32)


# Where the neural networks run. FACE_DEVICE = auto (default) | gpu | npu | cpu
#   gpu  ONNX Runtime + DirectML (any DirectX 12 GPU; on this PC the Radeon 8060S): fastest
#   npu  ONNX Runtime + VitisAI (Ryzen AI NPU; run from the ryzen-ai conda env): low power, but
#        measured slower than the GPU and CPU for one face at a time; first start compiles ~6 min
#   cpu  OpenCV DNN
#   auto GPU if available, else CPU
# Measured on this PC (ArcFace, one face): GPU 2.6-3.6 ms, CPU 12 ms, NPU 25 ms.
DEVICE = os.environ.get("FACE_DEVICE", "auto").lower()


def _ort_runner(path, npu_static=None):
    """Return (run, device_name) for an ONNX model on the chosen device, or None to use OpenCV."""
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
            sess = ort.InferenceSession(str(npu_static), so, providers=["VitisAIExecutionProvider"],
                                        provider_options=[opts])
            name = sess.get_inputs()[0].name
            return (lambda x: sess.run(None, {name: x})[0]), "NPU (Ryzen AI)"
    if DEVICE in ("auto", "gpu") and "DmlExecutionProvider" in avail:
        sess = ort.InferenceSession(str(path), so, providers=["DmlExecutionProvider", "CPUExecutionProvider"])
        name = sess.get_inputs()[0].name
        return (lambda x: sess.run(None, {name: x})[0]), "GPU (DirectML)"
    return None


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
        self.model = REC_NAME
        path = MODELS / REC["file"]
        if REC_NAME.startswith("sface"):
            sface = cv2.FaceRecognizerSF.create(str(path), "")
            self._feature = lambda crop: sface.feature(crop)
            self.device = "CPU (OpenCV)"
        else:
            runner = _ort_runner(path, MODELS / "npu" / (path.stem + "_b1.onnx"))
            if runner:
                run, self.device = runner
            else:
                net = cv2.dnn.readNetFromONNX(str(path))
                self.device = "CPU (OpenCV)"

                def run(x):
                    net.setInput(x)
                    return net.forward()

            def arcface(crop):
                x = (cv2.cvtColor(crop, cv2.COLOR_BGR2RGB).astype(np.float32) - 127.5) / 127.5
                return run(np.ascontiguousarray(x.transpose(2, 0, 1)[None]))   # one face per call
            self._feature = arcface
        self._qrun = self._optional(QUAL_MODEL)    # quality and expression models: GPU or CPU
        self._xrun = self._optional(EXPR_MODEL)
        self.expression_model = EXPR_MODEL.name if self._xrun else None

    @staticmethod
    def _optional(path):
        """Runner for an optional small ONNX model: ONNX Runtime on the GPU when available, else
        OpenCV on the CPU. None when the file isn't installed."""
        if not path.exists():
            return None
        runner = _ort_runner(path) if DEVICE != "npu" else None
        if runner:
            return runner[0]
        net = cv2.dnn.readNetFromONNX(str(path))

        def run(x):
            net.setInput(x)
            return net.forward()
        return run

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
            if all(_iou(r, k) < 0.4 for k in kept):
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


def _iou(a, b):
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


def list_media(folder, exts):
    folder = Path(folder)
    if folder.is_file():
        return [folder]
    return sorted(p for p in folder.rglob("*") if p.suffix.lower() in exts and p.is_file())


# --------------------------------------------------------------------------- store
def face_key(photo, row):
    """Permanent ID of a face: its photo's content fingerprint + where the face's centre is."""
    return f"{photo[:16]}-{round(float(row[0]) + float(row[2]) / 2)}-{round(float(row[1]) + float(row[3]) / 2)}"


class Store:
    """data/faces.npz holds every face found by a scan: face print, permanent face ID, source photo,
    box, quality and a JPEG thumbnail (all thumbnails packed into one byte array).
    data/photos.json describes each photo read: {content hash: {path, size, mtime, fp, faces, dup_of}}."""

    def __init__(self):
        DATA.mkdir(exist_ok=True)
        self.path = DATA / "faces.npz"
        self.photos_path = DATA / "photos.json"
        self.fid, self.photo = [], []
        if self.path.exists():
            z = np.load(self.path, allow_pickle=False)
            self.emb = list(z["emb"])
            self.file = [str(f) for f in z["files"]]
            self.box = list(z["box"])
            jpg, off = z["thumbs"], z["thumb_off"]
            self.thumbs = [jpg[off[i]:off[i + 1]].tobytes() for i in range(len(off) - 1)]
            self.quality = list(z["quality"]) if "quality" in z.files else [float("nan")] * len(self.emb)
            # databases from before model tracking were all made with SFace
            self.model = str(z["model"]) if "model" in z.files else "sface_2021dec"
            if "fid" in z.files:                 # older databases get IDs from labels.migrate()
                self.fid = [str(f) for f in z["fid"]]
                self.photo = [str(p) for p in z["photo"]]
        else:
            self.emb, self.file, self.box, self.thumbs, self.quality = [], [], [], [], []
            self.model = REC_NAME
        self.photos = load_json("photos.json", {})
        self._fids = set(self.fid)

    @property
    def needs_ids(self):
        return bool(self.emb) and not self.fid

    def add(self, photo, file, row, emb, crop, quality=float("nan")):
        """Append one face; returns its index."""
        fid = face_key(photo, row)
        while fid in self._fids:                 # practically never: two faces centred on the same pixel
            fid += "x"
        self._fids.add(fid)
        self.fid.append(fid)
        self.photo.append(photo)
        self.quality.append(quality)
        self.emb.append(emb)
        self.file.append(file)
        self.box.append(np.asarray(row[:5], np.float32))
        self.thumbs.append(cv2.imencode(".jpg", crop, [cv2.IMWRITE_JPEG_QUALITY, 88])[1].tobytes())
        return len(self.emb) - 1

    def fill_quality(self, faces):
        """Score faces scanned before quality scoring existed. Returns how many were scored."""
        todo = [i for i, q in enumerate(self.quality) if np.isnan(q)]
        if faces._qrun is None:
            return 0
        for i in todo:
            self.quality[i] = faces.quality(self.thumb(i))
        return len(todo)

    def thumb(self, i):
        return cv2.imdecode(np.frombuffer(self.thumbs[i], np.uint8), cv2.IMREAD_COLOR)

    def save(self):
        off = np.cumsum([0] + [len(t) for t in self.thumbs]).astype(np.int64)
        tmp = DATA / "faces.tmp.npz"
        np.savez(tmp,
                 emb=np.asarray(self.emb, np.float32).reshape(len(self.emb), -1),
                 model=np.asarray(self.model),
                 files=np.asarray(self.file, dtype=str),
                 fid=np.asarray(self.fid, dtype=str),
                 photo=np.asarray(self.photo, dtype=str),
                 box=np.asarray(self.box, np.float32).reshape(-1, 5),
                 thumbs=np.frombuffer(b"".join(self.thumbs), np.uint8),
                 thumb_off=off,
                 quality=np.asarray(self.quality, np.float32))
        replace_file(tmp, self.path)       # never leave a half-written database
        save_json("photos.json", self.photos)


def replace_file(tmp, dest, tries=20):
    """os.replace, retrying briefly: on Windows it fails while another program is reading dest."""
    for k in range(tries):
        try:
            return os.replace(tmp, dest)
        except PermissionError:
            if k == tries - 1:
                raise
            time.sleep(0.05)


def load_json(name, default):
    p = DATA / name
    return json.loads(p.read_text("utf-8")) if p.exists() else default


def save_json(name, obj):
    tmp = DATA / (name + ".tmp")
    tmp.write_text(json.dumps(obj, indent=1), "utf-8")
    replace_file(tmp, DATA / name)        # never leave a half-written file


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
    for i in range(len(store.emb)):
        crop = store.thumb(i)
        store.emb[i] = faces.embed(crop)
        if np.isnan(store.quality[i]):
            store.quality[i] = faces.quality(crop)
        if (i + 1) % 100 == 0:
            print(f"  {i + 1}/{len(store.emb)} faces")
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
    t = __import__("threading").Thread(target=imp.run, args=(Path(a.folder),), daemon=True)
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
            run_video(faces, ident, f, out)
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


def run_video(faces, ident, f, out):
    cap = cv2.VideoCapture(str(f))
    seen, n = {}, 0
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        n += 1
        if n % 10:
            continue                      # every 10th frame is plenty
        for row, emb, *_ in faces.detect(frame):
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
