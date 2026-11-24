#!/bin/sh
# store.sh [upload]: on the Mac, inside the desktop session: the iPhone app as the App Store takes it.
# Builds the Rust library, archives the app with Xcode, puts what the app carries into the archived app (carry.sh,
# every part), and has Xcode sign it for the App Store (the developer's own account in Xcode; its distribution
# certificate is Apple's cloud-managed one). Without "upload" the signed package is left in
# ~/fr-test/iosapp/store/out (FaceApp.ipa): for looking at. With "upload" Xcode sends it to App Store Connect,
# where the app's record (this bundle id) must exist: it then appears there under TestFlight after Apple's
# processing. The project is the one project.sh wrote (run that first: the version and build number are in it).
# Prints statuses and sizes only.
export PATH="$HOME/.cargo/bin:$PATH" DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export ORT_IOS_XCFWK_PATH="$HOME/fr-test/ort-ios/x/onnxruntime.xcframework" IPHONEOS_DEPLOYMENT_TARGET=16.0
MODE=${1:-export}
RUST=$HOME/fr-test/rust-src
P=$HOME/fr-test/iosapp
TEAM=$(sed -n 1p ~/fr-test/team.txt)
cd "$RUST" && cargo build -p face-app --release --target aarch64-apple-ios > "$P/cargo.log" 2>&1 || { echo "the Rust library didn't build"; tail -5 "$P/cargo.log"; exit 1; }
cd "$P" || exit 1
rm -rf store dd-store; mkdir store
xcodebuild -project FaceApp.xcodeproj -scheme FaceApp -configuration Release -destination "generic/platform=iOS" \
  -archivePath store/FaceApp.xcarchive -derivedDataPath dd-store -allowProvisioningUpdates archive > store/archive.log 2>&1
echo "archive status $?"
APP=store/FaceApp.xcarchive/Products/Applications/FaceApp.app
[ -d "$APP" ] || { grep -E "error:|ld: |Undefined" store/archive.log | sed -E "s#/Users/[a-z]+#~#g" | head -12; exit 1; }
sh "$RUST/apps/ios/carry.sh" "$APP" all || exit 1
echo "version $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Info.plist"), build $(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Info.plist"); privacy manifest in the app: $([ -f "$APP/PrivacyInfo.xcprivacy" ] && echo yes || echo no)"
[ "$MODE" = upload ] && DEST=upload || DEST=export
cat > store/export.plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key><string>app-store-connect</string>
	<key>destination</key><string>$DEST</string>
	<key>teamID</key><string>$TEAM</string>
	<key>signingStyle</key><string>automatic</string>
	<key>manageAppVersionAndBuildNumber</key><false/>
	<key>uploadSymbols</key><true/>
</dict>
</plist>
EOF
# (Xcode signs the app anew for the store here, with everything that was put into it)
xcodebuild -exportArchive -archivePath store/FaceApp.xcarchive -exportOptionsPlist store/export.plist -exportPath store/out \
  -allowProvisioningUpdates > store/export.log 2>&1
STATUS=$?
echo "$DEST status $STATUS"
if [ $STATUS -ne 0 ]; then
  grep -E "error:|Error Domain|No profiles|requires|not found|invalid" store/export.log | sed -E -e "s#/Users/[a-z]+#~#g" -e "s/[A-Z0-9]{10}/<id>/g" | cut -c1-220 | head -10
  exit 1
fi
[ -f store/out/FaceApp.ipa ] && echo "package: $(du -h store/out/FaceApp.ipa | cut -f1)"
[ "$DEST" = upload ] && echo "sent to App Store Connect: it appears under TestFlight when Apple has processed it"
exit 0
