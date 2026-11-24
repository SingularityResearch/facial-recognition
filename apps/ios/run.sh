#!/bin/sh
# On the Mac, inside the desktop session: install the built app on the iPhone, start it, and show what it
# prints for a while (the tracker's own lines: where each model runs, frames read a second).
#   sh run.sh [seconds to listen, default 60]
# FACE_IPHONE names the phone as for build.sh.
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
SECONDS_TO_LISTEN=${1:-60}
APP=$HOME/fr-test/iosapp/dd/Build/Products/Release-iphoneos/FaceApp.app
DEV=$(cat ~/fr-test/"${FACE_IPHONE:-iphone}".id)
BUNDLE=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$APP/Info.plist")
xcrun devicectl device install app --device "$DEV" "$APP" 2>&1 | grep -E "App installed|ERROR|error" | sed -E "s/$BUNDLE/<bundle id>/g" | head -3
OUT=$HOME/fr-test/iosapp/console.log
xcrun devicectl device process launch --device "$DEV" --console --terminate-existing "$BUNDLE" > "$OUT" 2>&1 &
PID=$!
sleep "$SECONDS_TO_LISTEN"
kill $PID 2>/dev/null
sed -E "s/$BUNDLE/<bundle id>/g" "$OUT" | grep -vE "Acquired tunnel|Enabling developer disk|Acquired usage assertion|^Launched application|^Waiting for the application" | tail -60
