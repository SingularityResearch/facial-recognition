#!/bin/sh
# Builds the app's Linux installer, or on 64-bit ARM Linux the Raspberry Pi's (an artifact of its own):
#
#     sh packaging/linux/build.sh [--no-build]
#
# The program is built, what it runs with is fetched where it isn't there yet (face-tools get: each download
# pinned by size and SHA-256; what there is no package of for this kind of computer is said and left out), and
# both are packed behind install.sh as target/dist/FaceRecognizer-<system>.run: the script, the line __APP__,
# then the app's folder as a .tar.gz. --no-build packs what is built already.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
rust=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$rust/target}
machine=$(uname -m)
case "$machine" in
  x86_64) system=linux-x64 ;;
  aarch64) system=raspberrypi-arm64 ;;
  *) echo "There is no installer for $machine computers."; exit 1 ;;
esac
if [ "${1:-}" != --no-build ]; then
  (cd "$rust" && cargo build -p face-desktop -p face-tools --release)
  # (each by itself: one that there is no package of yet doesn't keep the others out)
  for part in runtime heif ffmpeg; do
    "$target/release/face-tools" get --root "$target/root" "$part" || true
  done
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
app="$work/FaceRecognizer"
mkdir -p "$app/icons"
cp "$target/release/FaceRecognizer" "$app/"
# what the program runs with, beside it: the first place it looks in
if [ -d "$target/root/runtime" ]; then cp -a "$target/root/runtime/." "$app/"; fi
icons="$rust/assets/icons"
cp "$icons/icon.png" "$icons/icon-64.png" "$icons/icon-256.png" "$app/icons/"
mkdir -p "$target/dist"
out="$target/dist/FaceRecognizer-$system.run"
sed "s/__MACHINE__/$machine/" "$here/install.sh" > "$out"
echo "__APP__" >> "$out"
tar -C "$work" -czf - FaceRecognizer >> "$out"
chmod +x "$out"
# the same folder by itself, to unpack anywhere
tar -C "$work" -czf "$target/dist/FaceRecognizer-$system.tar.gz" FaceRecognizer
echo "$(basename "$out"): $(( $(wc -c < "$out") / 1000000 )) MB"
