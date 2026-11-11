"""
cudasupport.py - NVIDIA CUDA for the downloaded Linux app, fetched when it's needed.

The Linux app is built with the plain ONNX Runtime (the processor only): CUDA's libraries are about 1.9 GB, more than
a release file may hold. So when the app finds an NVIDIA card with a working driver and no CUDA in its own ONNX
Runtime, Settings -> This computer offers to download it (offer()), after checking the card and driver can run it
(check(): CUDA 12 needs driver 525.60 or newer, and the card compute capability MIN_CC or higher). Asked, it downloads
NVIDIA's own CUDA 12 and cuDNN 9 packages and Microsoft's ONNX Runtime with CUDA from PyPI, for the app's Python
(3.12, x86_64), the versions setup.sh installs, each checked by size and SHA-256, into `cuda/` in its data folder,
in the background. From the next start, activate() makes the app import that ONNX Runtime instead of its own, and
face_id's start-up check then decides, model by model, whether the card is used (same answers as the processor,
faster).

Run from the code, setup.sh installs the same with pip, so this is only for the frozen Linux app (needed()).
FACE_CUDA=off turns it off. status() is shown in Settings -> This computer. If the downloaded copy doesn't load, the
app goes on with its own, says why, and doesn't try it again until the next version.
"""
import hashlib
import importlib.abc
import importlib.machinery
import json
import os
import shutil
import sys
import threading
import urllib.request
import zipfile
from pathlib import Path

import paths

DIR = paths.ROOT / "cuda"                     # ~/.local/share/FaceRecognizer/cuda for the built app
READY = DIR / "ready.json"                    # written last, when every wheel is unpacked
TOP = ("onnxruntime", "nvidia")               # the packages taken from DIR instead of the app's own

PY = "https://files.pythonhosted.org/packages/"
# (file, bytes, SHA-256, path on files.pythonhosted.org): onnxruntime-gpu 1.24.4 for Python 3.12 (the app's), and the
# CUDA 12 / cuDNN 9 wheels its [cuda,cudnn] extras need (cuBLAS for cuDNN, nvJitLink for cuFFT). Change together.
WHEELS = [
    ("onnxruntime_gpu-1.24.4-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl", 252765531,
     "eb0e38f0c1ef3b76ae0081c8e51eed20dd8925aa916f0fc6f9b8b17d05610e99", "d0/2c/5b3fd4748cf7ed291eae541a37e426efc20ea04cb6e6a05768304ab0aa41"),
    ("nvidia_cublas_cu12-12.9.3.13-py3-none-manylinux_2_27_x86_64.whl", 581217458,
     "d7d51055f161b317c7105ddaae90e71eda059026074ffef93cf7cde2886bfe72", "7b/ea/9b6c36aef90766eebb4763aded157039e797ae9789c9c216e960d1d3cd47"),
    ("nvidia_cuda_nvrtc_cu12-12.9.86-py3-none-manylinux2010_x86_64.manylinux_2_12_x86_64.whl", 89568129,
     "210cf05005a447e29214e9ce50851e83fc5f4358df8b453155d5e1918094dcb4", "b8/85/e4af82cc9202023862090bfca4ea827d533329e925c758f0cde964cb54b7"),
    ("nvidia_cuda_runtime_cu12-12.9.79-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl", 3493179,
     "25bba2dfb01d48a9b59ca474a1ac43c6ebf7011f1b0b8cc44f54eb6ac48a96c3", "bc/46/a92db19b8309581092a3add7e6fceb4c301a3fd233969856a8cbf042cd3c"),
    ("nvidia_cudnn_cu12-9.28.1.4-py3-none-manylinux_2_27_x86_64.whl", 716579350,
     "cbcf6f33c77fef3aa61d3fec36f2e89b07eb27d65b9f369c1adf3cfee08c2c21", "2a/16/be895437624ae0400bda2b4bb69336cdb43346545e54edb15389d193a3b3"),
    ("nvidia_cufft_cu12-11.4.1.4-py3-none-manylinux2014_x86_64.manylinux_2_17_x86_64.whl", 200877592,
     "c67884f2a7d276b4b80eb56a79322a95df592ae5e765cf1243693365ccab4e28", "95/f4/61e6996dd20481ee834f57a8e9dca28b1869366a135e0d42e2aa8493bdd4"),
    ("nvidia_curand_cu12-10.3.10.19-py3-none-manylinux_2_27_x86_64.whl", 68295626,
     "49b274db4780d421bd2ccd362e1415c13887c53c214f0d4b761752b8f9f6aa1e", "31/44/193a0e171750ca9f8320626e8a1f2381e4077a65e69e2fb9708bd479e34a"),
    ("nvidia_nvjitlink_cu12-12.9.86-py3-none-manylinux2010_x86_64.manylinux_2_12_x86_64.whl", 39748338,
     "e3f1171dbdc83c5932a45f0f4c99180a70de9bd2718c1ab77d14104f6d7147f9", "46/0c/c75bbfb967457a0b7670b8ad267bfc4fffdf341c074e0a80db06c24ccfd4"),
]
SET = hashlib.sha256("".join(w[2] for w in WHEELS).encode()).hexdigest()[:16]   # which set is unpacked

STATE = {"state": "idle"}                     # idle | downloading | ready | active | failed | off | not needed
_lock = threading.Lock()


def nvidia_present():
    """An NVIDIA card with its driver loaded (the kernel module reports a version)."""
    return Path("/proc/driver/nvidia/version").exists() or bool(shutil.which("nvidia-smi"))


def needed():
    """True for the frozen Linux x86_64 app on a computer with an NVIDIA driver, unless turned off."""
    return (sys.platform.startswith("linux") and getattr(sys, "frozen", False) and os.uname().machine == "x86_64"
            and os.environ.get("FACE_CUDA", "").lower() not in ("off", "0", "no") and nvidia_present())


MIN_DRIVER = (525, 60)        # CUDA 12's minimum Linux driver (minor version compatibility)
MIN_CC = 5.2                  # the oldest cards ONNX Runtime's CUDA build and cuDNN 9 still run on (Maxwell 2nd gen)


def device():
    """{name, compute capability, driver} of the first NVIDIA card, from the driver itself and nvidia-smi."""
    out = {"name": None, "cc": None, "driver": None}
    try:
        line = Path("/proc/driver/nvidia/version").read_text().splitlines()[0]
        out["driver"] = next((w for w in line.split() if w.count(".") >= 1 and w.split(".")[0].isdigit()), None)
    except (OSError, IndexError):
        pass
    if shutil.which("nvidia-smi"):
        import subprocess
        try:
            r = subprocess.run(["nvidia-smi", "--query-gpu=name,compute_cap,driver_version", "--format=csv,noheader"],
                               capture_output=True, text=True, timeout=5).stdout.strip().splitlines()
            name, cc, drv = (v.strip() for v in r[0].split(","))
            out.update(name=name, cc=float(cc), driver=drv)
        except (OSError, ValueError, IndexError, subprocess.SubprocessError):
            pass
    return out


def check(dev=None):
    """(can CUDA 12 run here, why not / what was found)."""
    d = dev or device()
    if not d["driver"]:
        return False, "No NVIDIA driver is running (install the NVIDIA driver for the card first)."
    try:
        drv = tuple(int(x) for x in d["driver"].split(".")[:2])
    except ValueError:
        drv = (0, 0)
    if drv < MIN_DRIVER:
        return False, (f"The NVIDIA driver is {d['driver']}; CUDA 12 needs {MIN_DRIVER[0]}.{MIN_DRIVER[1]} or newer "
                       f"(update it with your system's driver tool, then start the app again).")
    if d["cc"] is not None and d["cc"] < MIN_CC:
        return False, f"{d['name']} (compute capability {d['cc']}) is older than CUDA 12 and cuDNN 9 support."
    return True, f"{d['name'] or 'NVIDIA card'}, driver {d['driver']}" + (f", compute capability {d['cc']}" if d["cc"] else "")


def offer():
    """For Settings: whether to offer the download, and what the card is (or why it can't be used)."""
    if not needed() or ready() or STATE.get("state") in ("downloading", "active"):
        return {"offer": False}
    ok, why = check()
    size = sum(w[1] for w in WHEELS) / 1e9
    return {"offer": ok, "card": why if ok else None, "why": None if ok else why, "gb": round(size, 2)}


def ready():
    try:
        return json.loads(READY.read_text()).get("set") == SET
    except (OSError, ValueError, AttributeError):
        return False


class _FromDir(importlib.abc.MetaPathFinder):
    """Finds onnxruntime and nvidia (and everything under them) in DIR, ahead of the app's own bundled modules."""

    def find_spec(self, name, path, target=None):
        if name.split(".")[0] not in TOP:
            return None
        return importlib.machinery.PathFinder.find_spec(name, [str(DIR)] if path is None else path)


_finder = None


def activate():
    """At the very start, before anything imports onnxruntime: use the downloaded ONNX Runtime with CUDA, if it's
    there and loads. Returns True when it's in use."""
    global _finder
    if os.environ.get("FACE_CUDA", "").lower() in ("off", "0", "no"):
        STATE.update(state="off")
        return False
    if not (needed() and ready()):
        return False
    _finder = _FromDir()
    sys.meta_path.insert(0, _finder)
    try:
        import onnxruntime as ort
        if "CUDAExecutionProvider" not in ort.get_available_providers():
            raise ImportError(f"no CUDA in the downloaded ONNX Runtime ({ort.get_available_providers()})")
        STATE.update(state="active", version=ort.__version__, where=str(DIR))
        return True
    except Exception as e:                    # go on with the app's own ONNX Runtime
        sys.meta_path.remove(_finder)
        _finder = None
        for mod in [m for m in sys.modules if m.split(".")[0] in TOP]:
            del sys.modules[mod]
        STATE.update(state="failed", error=f"the downloaded CUDA didn't load: {type(e).__name__}: {str(e)[:300]}")
        try:
            READY.write_text(json.dumps({"set": "broken", "why": STATE["error"]}))      # not again for this set
        except OSError:
            pass
        return False


def start_download():
    """Fetch and unpack the wheels on a background thread (only when asked, in Settings). Raises ValueError with the
    reason when it doesn't apply here."""
    if not needed():
        raise ValueError("CUDA is downloaded only by the Linux app on a computer with an NVIDIA card.")
    if ready() or STATE.get("state") in ("downloading", "active"):
        return
    ok, why = check()
    if not ok:
        raise ValueError(why)
    try:
        if READY.exists() and json.loads(READY.read_text()).get("set") == "broken":
            READY.unlink()                    # asked again: try once more
    except (OSError, ValueError):
        pass
    threading.Thread(target=_download, daemon=True, name="cuda-download").start()


def _download():
    with _lock:
        total = sum(w[1] for w in WHEELS)
        STATE.update(state="downloading", done=0, total=total, error=None)
        part = DIR.with_name("cuda.download")
        try:
            shutil.rmtree(part, ignore_errors=True)
            part.mkdir(parents=True)
            done = 0
            for name, size, sha, where in WHEELS:
                tmp = part / name
                h = hashlib.sha256()
                req = urllib.request.Request(PY + where + "/" + name, headers={"User-Agent": "FaceRecognizer-cuda"})
                with urllib.request.urlopen(req, timeout=60) as r, open(tmp, "wb") as out:
                    while chunk := r.read(1 << 20):
                        out.write(chunk)
                        h.update(chunk)
                        done += len(chunk)
                        STATE["done"] = done
                if tmp.stat().st_size != size or h.hexdigest() != sha:
                    raise OSError(f"{name} isn't the expected file (size or SHA-256 differs)")
                with zipfile.ZipFile(tmp) as z:
                    for m in z.namelist():            # only the packages, nowhere outside the folder
                        if m.split("/")[0] in TOP and ".." not in m.split("/"):
                            z.extract(m, part / "unpacked")
                tmp.unlink()
            shutil.rmtree(DIR, ignore_errors=True)
            (part / "unpacked").rename(DIR)
            shutil.rmtree(part, ignore_errors=True)
            READY.write_text(json.dumps({"set": SET, "wheels": [w[0] for w in WHEELS]}))
            STATE.update(state="ready")
        except Exception as e:
            shutil.rmtree(part, ignore_errors=True)
            STATE.update(state="failed", error=f"{type(e).__name__}: {str(e)[:300]}")


def status():
    """For Settings -> This computer: what happened with CUDA for this app, in words ("" when it doesn't apply)."""
    s = STATE.get("state")
    if s == "active":
        return f"CUDA: downloaded ONNX Runtime {STATE.get('version')} with CUDA in use ({STATE.get('where')})"
    if s == "downloading":
        return (f"CUDA: downloading for the NVIDIA card, {STATE.get('done', 0) / 1e9:.2f} of "
                f"{STATE.get('total', 0) / 1e9:.2f} GB (from PyPI; used from the next start)")
    if s == "ready":
        return "CUDA: downloaded; start the app again to use the NVIDIA card"
    if s == "failed":
        return f"CUDA: {STATE.get('error')}"
    o = offer()
    if o.get("offer"):
        return (f"CUDA: an NVIDIA card is here ({o['card']}) but this app can't use it yet: Download CUDA support "
                f"below ({o['gb']:.1f} GB) to run the models on it.")
    if o.get("why"):
        return f"CUDA: an NVIDIA card is here, but {o['why'][0].lower()}{o['why'][1:]}"
    if s == "off":
        return "CUDA: turned off (FACE_CUDA=off)"
    return ""
