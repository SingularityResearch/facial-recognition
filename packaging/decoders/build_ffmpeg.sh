#!/bin/sh
# FFmpeg 7.1.5 as the two programs the app ships (ffmpeg, ffprobe), built from its publisher's source with only
# what reading a photo library's videos needs: the same on Windows (MSYS2), macOS and Linux, a few MB each.
#
#     build_ffmpeg.sh <a folder to put ffmpeg and ffprobe into> [more options for ./configure]
#
# Why our own: every device has to decode a video to the same frames (a face's permanent ID is made from where
# its box lies in the frame), so every installer carries the same FFmpeg, version 7.1 (8.1 gives other frames
# for some videos); nothing may be assumed to be installed; and ready-made builds are 109 MB a program, exist
# for some systems only, and aren't kept for ever by their publishers.
#
# Licence: LGPL 2.1 or later. Nothing that would make it GPL or unredistributable is switched on (no
# --enable-gpl, no --enable-nonfree), and the programs are shipped as files of their own, never linked into
# the app. The source is the pinned archive below, unchanged; these options are the whole build.
#
# What is built in: the containers of the app's video file types (.mov .mp4 .m4v: mov; .mkv: matroska; .avi),
# the decoders for what cameras and phones write (H.264, HEVC, H.263, MPEG-4, Motion JPEG, VP8, VP9, MPEG-1/2,
# the older Windows kinds, ProRes, DV), what picks frames (select), the decoded picture written as it is to a
# pipe, and reading files only: no network at all. The colours are converted by the app itself, not by FFmpeg:
# FFmpeg's conversion differs from one processor and build to the next, its decoders don't (measured: the app's
# yuv.rs). scale and format are there for the rare kind of picture the app doesn't convert itself.
set -eu

VERSION=7.1.5
SIZE=11050340
SHA256=de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f

out=$1
shift
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL --retry 3 -A model-fetch -o "$work/ffmpeg.tar.xz" "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz"
size=$(wc -c < "$work/ffmpeg.tar.xz" | tr -d ' ')
if command -v sha256sum > /dev/null; then sum=$(sha256sum "$work/ffmpeg.tar.xz"); else sum=$(shasum -a 256 "$work/ffmpeg.tar.xz"); fi
sum=${sum%% *}
if [ "$size" != "$SIZE" ] || [ "$sum" != "$SHA256" ]; then
    echo "the download is not the pinned FFmpeg source (size or SHA-256 differs): nothing was built" >&2
    exit 1
fi
tar -xf "$work/ffmpeg.tar.xz" -C "$work"
cd "$work/ffmpeg-$VERSION"

./configure \
    --disable-everything --disable-autodetect --disable-doc --disable-debug --disable-network \
    --disable-ffplay --enable-ffmpeg --enable-ffprobe --disable-shared --enable-static \
    --enable-protocol=file,pipe \
    --enable-demuxer=mov,matroska,avi \
    --enable-decoder=h264,hevc,h263,mpeg4,mjpeg,vp8,vp9,mpeg1video,mpeg2video,msmpeg4v2,msmpeg4v3,wmv1,wmv2,prores,dvvideo,rawvideo \
    --enable-parser=h264,hevc,h263,mpeg4video,mjpeg,vp8,vp9,mpegvideo \
    --enable-bsf=h264_mp4toannexb,hevc_mp4toannexb,extract_extradata \
    --enable-filter=select,scale,format \
    --enable-encoder=rawvideo --enable-muxer=rawvideo \
    "$@"
jobs=$(getconf _NPROCESSORS_ONLN 2> /dev/null || echo 4)
make -j"$jobs"

for program in ffmpeg ffprobe; do
    if [ -f "$program.exe" ]; then program="$program.exe"; fi
    cp "$program" "$out/"
    strip "$out/$program" 2> /dev/null || true
done
ls -l "$out"
