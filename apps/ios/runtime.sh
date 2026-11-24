#!/bin/sh
# On the Mac: fetch the model runtime the iPhone app is linked with (ONNX Runtime's static xcframework) into
# ~/fr-test/ort-ios/x, where project.sh, build.sh and store.sh expect it.
#
# It comes out of Microsoft's own package for .NET (Microsoft.ML.OnnxRuntime on nuget.org, which carries the
# iPhone's xcframework). The archive that CocoaPods names for the same thing was not to be had (its server
# answered with another name's certificate, 2026-11-22) and has no version after 1.30. The package is pinned
# by size and SHA-256, as every download is. A new pin goes in with the package downloaded once, its
# signatures checked (`dotnet nuget verify --all`: the author's, Microsoft Corporation, and nuget.org's), and
# its size and SHA-256 taken from that.
#
# 1.31.1, as a Mac and the Android app have: 1.24 kept memory for every camera frame on Apple's processors
# with SME2, an iPhone 17's among them. Before this pin is changed: crates/face-core/examples/grows.rs on a
# Mac of that kind, with the same version's library for the Mac.
set -e
VERSION=1.31.1
SIZE=159626394
SHA256=6871ccbc97f3d5bdb4095a1dcec33aa78d56f6ed8de5980ef5fa1d9f2e7c679a
INSIDE=runtimes/ios/native/onnxruntime.xcframework.zip
INSIDE_SHA256=d871ff5f3fa0beef014d5baff965ac6ba5c2724d37b2de2d738d8bc78fcca3a4
TO=$HOME/fr-test/ort-ios
URL=https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/$VERSION/microsoft.ml.onnxruntime.$VERSION.nupkg

if [ -e "$TO" ]; then
  echo "$TO is there already: move it away first (nothing is deleted here)"
  exit 1
fi
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
curl -sSL --fail -o "$WORK/runtime.nupkg" "$URL"
sum() { shasum -a 256 "$1" | cut -d' ' -f1; }
if [ "$(stat -f %z "$WORK/runtime.nupkg")" != "$SIZE" ] || [ "$(sum "$WORK/runtime.nupkg")" != "$SHA256" ]; then
  echo "what came is not the pinned package: nothing was unpacked"
  exit 1
fi
unzip -q "$WORK/runtime.nupkg" "$INSIDE" -d "$WORK"
if [ "$(sum "$WORK/$INSIDE")" != "$INSIDE_SHA256" ]; then
  echo "the iPhone's runtime inside the package is not the pinned file: nothing was unpacked"
  exit 1
fi
mkdir -p "$TO/x"
unzip -q "$WORK/$INSIDE" -d "$TO/x"
echo "ONNX Runtime $VERSION for the iPhone is in $TO/x"
