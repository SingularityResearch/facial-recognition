#!/bin/sh
# carry.sh <FaceApp.app> [all]: put what the iPhone app carries into the built app: the page files and the models.
# The trial build (build.sh) and the App Store build (store.sh) both use it, so the two carry the same.
#   ~/fr-test/pages            the page files (web/*.js *.html *.css of this repository)
#   the models                 from the Mac's own installed app, which fetched each as the pinned file
#   ~/fr-test/mediapipe-web    MediaPipe's web runtime, as `face-tools get mediapipe` fetches it
# With "all" (the App Store build) nothing may be missing: a published app has every part. Without it, the sound
# model and MediaPipe's parts are taken when they are there, and the app says what it lacks.
APP=${1:?usage: carry.sh <FaceApp.app> [all]}
ALL=${2:-}
M="$HOME/Library/Application Support/FaceRecognizer/models"
[ -d "$APP" ] || { echo "no app at $APP"; exit 1; }
[ -d ~/fr-test/pages ] || { echo "no page files in ~/fr-test/pages"; exit 1; }
rm -rf "$APP/pages" "$APP/models"; mkdir "$APP/models"
cp -R ~/fr-test/pages "$APP/pages"
for f in opsis_1.onnx melos_1.onnx ediffiqa_tiny_jun2024.onnx face_detection_yunet_2023mar.onnx face_detection_yunet_2023mar_dynamic.onnx facial_expression_recognition_mobilefacenet_2022july.onnx; do
  cp "$M/$f" "$APP/models/" || { echo "the model $f isn't in the Mac's installed app"; exit 1; }
done
missing=""
# the sound model for the Signals chart's sound (what the phone's own microphone heard, in a word); without it the
# chart has the background's numbers and no word for music or speech
if [ -f "$M/yamnet.onnx" ]; then cp "$M/yamnet.onnx" "$APP/models/"; else missing="$missing yamnet.onnx"; fi
# MediaPipe's parts run in the page: its three models and its web runtime; without them the app has no cues,
# posture or fingers and says so
for f in face_landmarker.task pose_landmarker_lite.task hand_landmarker.task; do
  if [ -f "$M/$f" ]; then cp "$M/$f" "$APP/models/"; else missing="$missing $f"; fi
done
if [ -d ~/fr-test/mediapipe-web ]; then cp -R ~/fr-test/mediapipe-web "$APP/models/mediapipe-web"; else missing="$missing mediapipe-web"; fi
if [ -n "$missing" ]; then
  echo "not carried (not on this Mac):$missing"
  [ "$ALL" = all ] && exit 1
fi
echo "carried: $(ls "$APP/pages" | wc -l | tr -d ' ') page files, $(find "$APP/models" -type f | wc -l | tr -d ' ') model files"
