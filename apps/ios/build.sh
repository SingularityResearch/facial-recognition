#!/bin/sh
# On the Mac, inside the desktop session: build the Rust library for the
# iPhone, then the app around it with Xcode's automatic signing, the connected iPhone named as the destination;
# then put the page files and the models into the app and sign it again.
#   ~/fr-test/pages    the page files (face-recognizer's tracker page and scripts, icons/icon.ico)
#   the models         from the Mac's own installed app
#   FACE_IPHONE        which phone, when there are several: the name of its two files in ~/fr-test,
#                      <name>.udid (the build's destination) and <name>.id (devicectl's); "iphone" without it
export PATH="$HOME/.cargo/bin:$PATH" DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export ORT_IOS_XCFWK_PATH="$HOME/fr-test/ort-ios/x/onnxruntime.xcframework" IPHONEOS_DEPLOYMENT_TARGET=16.0
RUST=$HOME/fr-test/rust-src
P=$HOME/fr-test/iosapp
cd "$RUST" && cargo build -p face-app --release --target aarch64-apple-ios > "$P/cargo.log" 2>&1 || { echo "the Rust library didn't build"; tail -5 "$P/cargo.log"; exit 1; }
cd "$P" || exit 1
rm -rf build dd
xcodebuild -project FaceApp.xcodeproj -scheme FaceApp -configuration Release \
  -destination "platform=iOS,id=$(cat ~/fr-test/"${FACE_IPHONE:-iphone}".udid)" -derivedDataPath dd \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration build > build.log 2>&1
echo "xcodebuild status $?"
APP=dd/Build/Products/Release-iphoneos/FaceApp.app
[ -d "$APP" ] || { grep -E "error:|ld: |Undefined" build.log | head -12; exit 1; }
codesign -d --entitlements ent.plist --xml "$APP" 2>/dev/null
IDENT=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=\(Apple Development.*\)/\1/p' | head -1)
sh "$RUST/apps/ios/carry.sh" "$APP" || exit 1
codesign -f -s "$IDENT" --entitlements ent.plist "$APP" > /dev/null 2>&1
codesign --verify --deep --strict "$APP" && echo "signed; app $(du -sh "$APP" | cut -f1)"
