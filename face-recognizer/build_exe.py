"""
build_exe.py - builds the face recognizer app (app.py: the tracker and the labeling tool in one window) with
PyInstaller, for the system it runs on (PyInstaller can't build for another one):

    Windows  build\\dist\\FaceRecognizer.exe     one file; uses the models\\ and data\\ folders beside it
    macOS    build/dist/FaceRecognizer.app       Apple Silicon, macOS 14+; asks once for the camera and microphone
                                                (the text below); signed and notarized by CI when the keys are there
    Linux    build/dist/FaceRecognizer/          a folder (FaceRecognizer inside starts it); CI packs it as .tar.gz

On macOS and Linux the models and data live in the user's application data (paths.py), as the app is read-only.
The one place the build is described: build_app.bat runs it on Windows, and CI's build jobs run it on GitHub's
Windows, macOS and Linux machines, so all make the same app.

    python build_exe.py            -> build, and on Windows copy FaceRecognizer.exe beside this file
    python build_exe.py --no-copy  -> build only (CI)

Needs requirements-build.txt installed (on Linux also pywebview[qt], for the app window).
"""
import argparse
import plistlib
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Every page file the three servers send: packed into the app (a test checks each page's scripts are listed here)
PAGES = ["tracker_ui.html", "label_ui.html", "app_ui.html", "tracker_ui.js", "label_ui.js", "app_ui.js",
         "expression.js", "signals.js", "settings_bridge.js", "page_store.js"]
EXCLUDE = ["torch", "torchvision", "torchaudio", "torchgen", "tensorflow", "pandas", "IPython"]
BUNDLE_ID = "io.github.singularityresearch.facerecognizer"
MACOS_MIN = "14.0"                     # onnxruntime's macOS wheels need macOS 14 (Apple Silicon only)
CAMERA_WHY = ("The live tracker recognizes the people you have named, from the camera. Pictures stay on this Mac: "
              "only numbers are kept, unless you turn on Collect faces.")
MIC_WHY = ("The live tracker can measure voice signals (loudness, pitch, speaking rate) from the microphone. "
           "Nothing is recorded.")


def version():
    return re.search(r'^VERSION = "([^"]+)"', (HERE / "app.py").read_text("utf-8"), re.M).group(1)


def command(platform=sys.platform):
    sep = ";" if platform == "win32" else ":"
    cmd = [sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", "--windowed",
           "--name", "FaceRecognizer", "--workpath", "build", "--distpath", str(Path("build") / "dist"),
           "--specpath", "build"]
    if platform == "win32":
        cmd.append("--onefile")
    elif platform == "darwin":
        cmd += ["--osx-bundle-identifier", BUNDLE_ID]
    for f in PAGES:
        cmd += ["--add-data", f"{HERE / f}{sep}."]
    cmd += ["--add-data", f"{HERE / 'db'}{sep}db"]          # the shared database's schema (facedb.migrate)
    cmd += ["--collect-all", "mediapipe", "--collect-all", "onnxruntime", "--collect-submodules", "webview",
            "--hidden-import", "sklearn.cluster", "--collect-all", "psycopg_binary", "--hidden-import", "pgvector.psycopg"]
    if platform.startswith("linux"):
        cmd += ["--hidden-import", "qtpy", "--collect-submodules", "qtpy"]
    for m in EXCLUDE:
        cmd += ["--exclude-module", m]
    return cmd + ["app.py"]


# Linux: graphics libraries that must be the user's own, matching their Mesa and GPU drivers. PyInstaller copies the
# build machine's (seen: libgbm.so.1, libX11.so.6), and then Qt's web view can't start OpenGL ("Could not initialize
# GLX", 2026-11-09 on Ubuntu 24.04). Every desktop has them (README: what a minimal install needs). Only the system's
# own names: a copy renamed with a hash (libdrm-b0291a67.so...) is private to the Python package it came with, which
# loads it by that name.
LINUX_SYSTEM_LIBS = ("libgbm.so", "libdrm.so", "libX11.so", "libX11-xcb.so", "libxcb.so", "libGL.so", "libGLX.so",
                     "libGLX_", "libEGL.so", "libEGL_", "libGLdispatch.so", "libOpenGL.so", "libwayland-client.so",
                     "libwayland-server.so", "libwayland-egl.so", "libwayland-cursor.so", "libvulkan.so")


def drop_system_libs(internal):
    """Remove the copies of LINUX_SYSTEM_LIBS from a built Linux app's _internal folder. Returns their names."""
    gone = []
    for f in sorted(Path(internal).iterdir()):
        if f.is_file() and f.name.startswith(LINUX_SYSTEM_LIBS):
            f.unlink()
            gone.append(f.name)
    return gone


def finish_macos_app(app, ver):
    """What macOS needs in the app's Info.plist: why it asks for the camera and microphone (without these the
    system refuses them), the oldest macOS it runs on, and its version. Signing comes after (CI)."""
    plist = app / "Contents" / "Info.plist"
    info = plistlib.loads(plist.read_bytes())
    info.update({"NSCameraUsageDescription": CAMERA_WHY, "NSMicrophoneUsageDescription": MIC_WHY,
                 "LSMinimumSystemVersion": MACOS_MIN, "CFBundleShortVersionString": ver, "CFBundleVersion": ver,
                 "CFBundleDisplayName": "Face Recognizer", "NSHighResolutionCapable": True})
    plist.write_bytes(plistlib.dumps(info))


def main():
    ap = argparse.ArgumentParser(description="Build the face recognizer app for this system")
    ap.add_argument("--no-copy", action="store_true", help="leave it in build/dist (don't copy the .exe beside this file)")
    args = ap.parse_args()
    missing = [f for f in PAGES if not (HERE / f).exists()]
    if missing:
        sys.exit(f"Missing page files: {', '.join(missing)}")
    subprocess.run(command(), cwd=HERE, check=True)
    dist = HERE / "build" / "dist"
    if sys.platform == "win32":
        out = dist / "FaceRecognizer.exe"
    elif sys.platform == "darwin":
        out = dist / "FaceRecognizer.app"
        if out.exists():
            finish_macos_app(out, version())
    else:
        out = dist / "FaceRecognizer"
        if (out / "_internal").is_dir():
            print("Left to the system:", ", ".join(drop_system_libs(out / "_internal")) or "nothing")
    if not out.exists():
        sys.exit(f"PyInstaller finished but {out} isn't there")
    if sys.platform == "win32" and not args.no_copy:
        shutil.copy2(out, HERE / out.name)
    print(f"Built {out}")


if __name__ == "__main__":
    main()
