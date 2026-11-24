#!/bin/sh
# Signs FaceRecognizer.app, or the .dmg made from it, with a Developer ID certificate and has Apple notarize it, so
# macOS opens it without warnings. Run by CI's macOS build job, only when the repository's signing secrets are set
# (made once, by the owner):
#   MACOS_CERT_P12_BASE64   the "Developer ID Application" certificate with its key, as .p12, base64
#   MACOS_CERT_PASSWORD     that .p12's password
#   APPLE_API_KEY_ID, APPLE_API_ISSUER_ID, APPLE_API_KEY_P8_BASE64   an App Store Connect API key (a team key)
# Usage: sign_and_notarize.sh <FaceRecognizer.app | FaceRecognizer-macos-arm64.dmg>
#   the app first: its signature and Apple's ticket are then inside every package made from it;
#   then the .dmg made from that app: a disk image downloaded from the internet is checked as a whole.
# When Apple refuses, its log is printed: it names each file inside the app it objects to, and why.
set -eu
TARGET="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
KEYCHAIN="$WORK/signing.keychain-db"
KC_PASS="$(uuidgen)"
trap 'security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

echo "$MACOS_CERT_P12_BASE64" | base64 --decode > "$WORK/cert.p12"
security create-keychain -p "$KC_PASS" "$KEYCHAIN"
security set-keychain-settings -lut 3600 "$KEYCHAIN"
security unlock-keychain -p "$KC_PASS" "$KEYCHAIN"
security import "$WORK/cert.p12" -k "$KEYCHAIN" -P "$MACOS_CERT_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$KC_PASS" "$KEYCHAIN" >/dev/null
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" | sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1)"
[ -n "$IDENTITY" ] || { echo "No Developer ID Application certificate in MACOS_CERT_P12_BASE64"; exit 1; }

case "$TARGET" in
  *.dmg)
    codesign --force --timestamp --sign "$IDENTITY" --keychain "$KEYCHAIN" "$TARGET"
    codesign --verify --verbose=2 "$TARGET"
    SUBMIT="$TARGET" ;;
  *)
    # what the program runs with lies beside it inside the app (the model runtime, the HEIC libraries, FFmpeg's
    # two programs): each is signed by itself first, then the app around them
    for inside in "$TARGET"/Contents/MacOS/*; do
      [ "$(basename "$inside")" = FaceRecognizer ] && continue
      codesign --force --timestamp --options runtime --sign "$IDENTITY" --keychain "$KEYCHAIN" "$inside"
    done
    codesign --force --deep --timestamp --options runtime --entitlements "$HERE/entitlements.plist" \
      --sign "$IDENTITY" --keychain "$KEYCHAIN" "$TARGET"
    codesign --verify --deep --strict --verbose=2 "$TARGET"
    ditto -c -k --keepParent "$TARGET" "$WORK/notarize.zip"
    SUBMIT="$WORK/notarize.zip" ;;
esac

echo "$APPLE_API_KEY_P8_BASE64" | base64 --decode > "$WORK/key.p8"
KEY="--key $WORK/key.p8 --key-id $APPLE_API_KEY_ID --issuer $APPLE_API_ISSUER_ID"
xcrun notarytool submit "$SUBMIT" $KEY --wait --output-format json > "$WORK/answer.json" || true
cat "$WORK/answer.json"; echo
ID="$(sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' "$WORK/answer.json" | head -1)"
if ! grep -q '"status" *: *"Accepted"' "$WORK/answer.json"; then
  echo "Apple did not accept it. Its log:"
  [ -z "$ID" ] || xcrun notarytool log "$ID" $KEY || true
  exit 1
fi
xcrun stapler staple "$TARGET"
case "$TARGET" in
  *.dmg) spctl --assess --type open --context context:primary-signature --verbose "$TARGET" ;;
  *)     spctl --assess --type execute --verbose "$TARGET" ;;
esac
echo "Signed and notarized: $TARGET"
