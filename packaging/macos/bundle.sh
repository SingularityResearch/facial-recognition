#!/bin/sh
# Makes the app as macOS knows one (FaceRecognizer.app) from the built program:
#
#     sh packaging/macos/bundle.sh <the built FaceRecognizer> <folder to make the app in> [bundle id] [name shown] [runtime folder]
#
# The app gets the program, the icon and Info.plist (the texts macOS shows when it asks about the camera, the
# microphone and the local network), and, when a runtime folder is named, what the program runs with beside the
# program inside the app (the model runtime, the HEIC libraries: the first place the program looks in, and the
# only one a signed app may load a library from). It is signed for this computer only (ad hoc: no key, no
# account). A copy for a check takes a bundle id and a name of its own, so that macOS keeps what it is allowed
# apart from the app's. Signing for other computers and notarizing are the release's steps, not this one's.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
program=$1
into=$2
id=${3:-io.github.singularityresearch.facerecognizer}
name=${4:-Face Recognizer}
runtime=${5:-}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/../../Cargo.toml" | head -1)
app="$into/FaceRecognizer.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$program" "$app/Contents/MacOS/FaceRecognizer"
if [ -n "$runtime" ] && [ -d "$runtime" ]; then
  for f in "$runtime"/*; do
    [ -f "$f" ] || continue
    cp "$f" "$app/Contents/MacOS/"
    codesign --force --sign - "$app/Contents/MacOS/$(basename "$f")" 2>/dev/null
  done
fi
cp "$here/../../assets/icons/icon.icns" "$app/Contents/Resources/icon.icns"
sed -e "s/__ID__/$id/" -e "s/__NAME__/$name/" -e "s/__VERSION__/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
codesign --force --sign - "$app" 2>/dev/null
echo "$app"
