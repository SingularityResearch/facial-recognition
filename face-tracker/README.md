# Face Tracker

Real-time face tracking in the browser with a webcam (USB or built-in) or a video file.
Uses Google's MediaPipe face detector (BlazeFace short-range, made for faces within about 2 m of the camera); all
processing runs in the browser. It finds faces only: it doesn't recognize who they are or read expression.

## Run

Double-click `index.html` to open it in Chrome or Edge, click **Start camera**, and allow camera access.
Pick your USB webcam from the **Camera** dropdown if the wrong camera shows, or click **Open video** to track faces
in a video file.

This is the project's original demo, and **it uses the internet**: each time it opens it loads the detector from
`cdn.jsdelivr.net`, the face model from `storage.googleapis.com` and its fonts from Google Fonts. The video stays in
the browser; nothing is uploaded. It is not part of the offline Face Recognizer (`../face-recognizer`), which loads
nothing from the internet. The detector's main file is pinned to MediaPipe 0.10.14 with its SHA-384 (an import map
`integrity`, so the browser refuses a changed file); the engine files it loads next and the model can't be pinned
that way.

## Features

- Tracks multiple faces with stable IDs, boxes, keypoints and motion trails
- Live face count, FPS and per-face position
- Mirror view, keypoints, motion trail, face blur, smoothing and minimum confidence controls

## See also

- [Repository README](../README.md): goals and key decisions
- [Face Recognizer](../face-recognizer/README.md): recognizes specific people and shows their names and expression live
- [Project context](../PROJECT_CONTEXT.md): background and history of both programs
- [Why expression reading is layered](../docs/WHY_LAYERED_EXPRESSION.md)
