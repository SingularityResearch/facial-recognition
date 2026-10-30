# Face Tracker

Real-time face tracking in the browser with a webcam (USB or built-in) or a video file.
Uses Google's MediaPipe face detector; all processing runs locally.

## Run

Double-click `index.html` to open it in Chrome or Edge, click **Start camera**, and allow camera access.
Pick your USB webcam from the **Camera** dropdown if the wrong camera shows.

Needs an internet connection on first load to fetch the detector from
`cdn.jsdelivr.net` and the model from `storage.googleapis.com`.

## Features

- Tracks multiple faces with stable IDs, boxes, keypoints and motion trails
- Live face count, FPS and per-face position
- Mirror view, smoothing, minimum confidence and face blur controls
