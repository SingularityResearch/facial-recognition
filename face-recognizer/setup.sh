#!/bin/sh
# Sets up the face recognizer on macOS or Linux (setup.bat does this on Windows): a Python 3.12 environment in
# .venv with the pinned libraries (requirements.txt), then the face models (fetch_models.py). Run it once, and again
# after updating:   ./setup.sh
# macOS: Apple Silicon, macOS 14 or later (the recognition libraries have no Intel Mac builds).
# Linux: the app window uses Qt (installed here into .venv); without it the app opens in the web browser instead.
set -e
cd "$(dirname "$0")"
PY="${PYTHON:-python3.12}"
if ! command -v "$PY" >/dev/null 2>&1; then
  echo "Python 3.12 is needed first."
  echo "  macOS:  brew install python@3.12        (https://brew.sh)"
  echo "  Ubuntu: sudo apt install python3.12 python3.12-venv"
  exit 1
fi
echo "Using $("$PY" --version)"
[ -x .venv/bin/python ] || "$PY" -m venv .venv
.venv/bin/python -m pip install --upgrade pip
ORT_GPU="onnxruntime-gpu[cuda,cudnn]==1.24.4"     # same version as requirements.txt's onnxruntime
if [ "$(uname)" = "Linux" ] && nvidia-smi >/dev/null 2>&1; then
  # NVIDIA card with a working driver: ONNX Runtime with CUDA (the CUDA and cuDNN libraries come from pip, no
  # CUDA toolkit needed). It replaces plain onnxruntime: both install the same files, like the two OpenCVs.
  echo "NVIDIA GPU found: installing ONNX Runtime with CUDA."
  grep -v '^onnxruntime==' requirements.txt > .venv/requirements-nvidia.txt
  .venv/bin/python -m pip install -r .venv/requirements-nvidia.txt
  if .venv/bin/python -m pip show onnxruntime >/dev/null 2>&1; then
    .venv/bin/python -m pip uninstall -y onnxruntime
    .venv/bin/python -m pip install --force-reinstall --no-deps "onnxruntime-gpu==1.24.4"
  fi
  .venv/bin/python -m pip install "$ORT_GPU"
else
  .venv/bin/python -m pip install -r requirements.txt
fi
if [ "$(uname)" = "Linux" ]; then
  # the app window: Qt's web view, which brings its own browser engine (no system packages needed)
  .venv/bin/python -m pip install "pywebview[qt]==6.2.1" || echo "(No app window libraries: the app will open in your web browser.)"
fi
.venv/bin/python fetch_models.py
echo
echo "Ready. Start the app with ./start.sh (or double-click FaceRecognizer.command on a Mac)."
