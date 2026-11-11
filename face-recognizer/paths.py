r"""
paths.py - where the programs keep things, and the one way they write files.

  ROOT  the folder with models\ and data\: beside the source files; when built, beside FaceRecognizer.exe on
        Windows (the .exe carries only the code and pages and shares this folder's models and data with the .bat
        launchers), and in the user's application data on macOS and Linux, where the app itself is read-only
        (a signed .app, an AppImage): ~/Library/Application Support/FaceRecognizer, ~/.local/share/FaceRecognizer.
        FACE_ROOT=<folder> uses another one (tests, a second copy).
  HOME  what is private to this computer's user and never belongs in the project (Web search keys, describe
        settings and descriptions, webcam snapshots, presence log, alerts, the app window's storage, app.log):
        %APPDATA%\FaceLabels on Windows, ~/Library/Application Support/FaceLabels on macOS,
        $XDG_DATA_HOME/FaceLabels (~/.local/share/FaceLabels) on Linux. FACE_HOME=<folder> uses another one.

Nothing here imports the rest of the project, so every module (and the tests) can use it.
"""
import os
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _system_certificates():
    """HTTPS checks certificates the way the operating system does (truststore): on Windows, Python reads only the
    root certificates already in the store, and a fresh install has few (Windows adds the rest when it first needs
    them, for its own connections only), so the model download from GitHub failed with CERTIFICATE_VERIFY_FAILED on
    a new computer (2026-11-11). Every module imports this one, so every download, update check and Web search gets it."""
    try:
        import truststore
        truststore.inject_into_ssl()
    except Exception:                          # not installed (an old setup): Python's own checks, as before
        pass


_system_certificates()


def user_data(name, platform=sys.platform, env=os.environ, home=None):
    """This user's application-data folder for name, as each system expects it."""
    home = Path(home or Path.home())
    if platform == "win32":
        return Path(env.get("APPDATA") or home / "AppData" / "Roaming") / name
    if platform == "darwin":
        return home / "Library" / "Application Support" / name
    return Path(env.get("XDG_DATA_HOME") or home / ".local" / "share") / name


def _root():
    if os.environ.get("FACE_ROOT"):
        return Path(os.environ["FACE_ROOT"]).resolve()
    if not getattr(sys, "frozen", False):
        return HERE
    if sys.platform == "win32":
        return Path(sys.executable).resolve().parent
    return user_data("FaceRecognizer")


ROOT = _root()
HOME = Path(os.environ["FACE_HOME"]).resolve() if os.environ.get("FACE_HOME") else user_data("FaceLabels")


def replace_file(tmp, dest, tries=20):
    """os.replace, retrying briefly: on Windows it fails while another program (or thread) is reading dest."""
    for k in range(tries):
        try:
            return os.replace(tmp, dest)
        except PermissionError:
            if k == tries - 1:
                raise
            time.sleep(0.05)


def atomic_write(path, data):
    """Write text or bytes to path without ever leaving a half-written file: into path.tmp, then replaced."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    if isinstance(data, str):
        tmp.write_text(data, "utf-8")
    else:
        tmp.write_bytes(data)
    replace_file(tmp, path)
