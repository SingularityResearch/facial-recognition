#!/bin/sh
# Builds the Mac's app and its installer, a disk image with the app beside a link to Applications:
#
#     sh packaging/macos/build.sh [--no-build] [--app | --dmg] [bundle id] [name shown]
#
# The program is built, what it runs with is fetched where it isn't there yet (face-tools get: each download
# pinned by size and SHA-256, the HEIC libraries built from their pinned sources where CMake is there; what there
# is no package of for a Mac yet is said and left out), bundle.sh makes FaceRecognizer.app with all of it inside,
# and hdiutil packs it as FaceRecognizer-macos-arm64.dmg, both in target/dist (or the folder DIST names).
# --no-build packs what is built already. --app stops after the app; --dmg packs the app that is in the folder
# already. CI uses the two so that the app is signed and notarized before it goes into the image
# (sign_and_notarize.sh), and the image after. Without them the app is signed for this computer only.
# A bundle id and name of its own are for a check's copy.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$repo/target}
build=yes
part=all
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) build=no ;;
    --app) part=app ;;
    --dmg) part=dmg ;;
    *) break ;;
  esac
  shift
done
dist=${DIST:-$target/dist}
mkdir -p "$dist"
if [ "$part" != dmg ]; then
  if [ "$build" = yes ]; then
    (cd "$repo" && cargo build -p face-desktop -p face-tools --release)
    # (each by itself: one that there is no package of yet doesn't keep the others out)
    for piece in runtime heif ffmpeg; do
      "$target/release/face-tools" get --root "$target/root" "$piece" || true
    done
  fi
  sh "$here/bundle.sh" "$target/release/FaceRecognizer" "$dist" "${1:-io.github.singularityresearch.facerecognizer}" "${2:-Face Recognizer}" "$target/root/runtime" > /dev/null
  echo "FaceRecognizer.app"
fi
if [ "$part" != app ]; then
  stage=$(mktemp -d)
  trap 'rm -rf "$stage"' EXIT
  ditto "$dist/FaceRecognizer.app" "$stage/FaceRecognizer.app"
  ln -s /Applications "$stage/Applications"
  out="$dist/FaceRecognizer-macos-arm64.dmg"
  rm -f "$out"
  hdiutil create -quiet -volname "Face Recognizer" -srcfolder "$stage" -ov -format UDZO "$out"
  echo "$(basename "$out"): $(( $(wc -c < "$out") / 1000000 )) MB"
fi
