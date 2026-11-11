#!/bin/sh
# Starts the labeling tool on its own (label_tool.py, http://127.0.0.1:8765), on macOS or Linux.
# Run ./setup.sh once first. ./start.sh opens both programs in one window instead.
cd "$(dirname "$0")"
if [ ! -x .venv/bin/python ]; then
  echo "Run ./setup.sh first (it installs the libraries and models)."
  exit 1
fi
exec .venv/bin/python label_tool.py "$@"
