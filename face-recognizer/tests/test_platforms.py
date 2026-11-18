"""What differs between Windows, macOS and Linux (paths.user_data, desktop.py), checked for all three on any one:
where each system keeps a user's application data, and how a folder is shown."""
from pathlib import Path

import desktop
import paths


def test_each_system_keeps_application_data_where_it_expects():
    home = Path("/home/ann")
    assert paths.user_data("FaceLabels", "win32", {"APPDATA": r"C:\Users\Ann\AppData\Roaming"}, home) == \
        Path(r"C:\Users\Ann\AppData\Roaming") / "FaceLabels"
    assert paths.user_data("FaceLabels", "darwin", {}, home) == home / "Library" / "Application Support" / "FaceLabels"
    assert paths.user_data("FaceLabels", "linux", {}, home) == home / ".local" / "share" / "FaceLabels"
    assert paths.user_data("FaceLabels", "linux", {"XDG_DATA_HOME": "/data/ann"}, home) == Path("/data/ann/FaceLabels")


def test_folders_open_in_each_systems_file_manager():
    ran = []
    desktop.open_folder("/tmp/x", "darwin", ran.append)
    desktop.open_folder("/tmp/x", "linux", ran.append)
    assert ran == [["open", "/tmp/x"], ["xdg-open", "/tmp/x"]]


def test_attention_never_raises_where_the_window_cant_do_it():
    class NoWindow:
        native = None
    for platform in ("win32", "darwin", "linux"):
        desktop.attention(NoWindow(), platform)          # prints why, doesn't raise


def test_a_locked_screen_is_told_from_what_windows_says_about_the_session():
    """Windows stops running cameras when the screen is locked, so the tracker page asks before it decides what to
    do about a camera that isn't running. The answer is read from the start of WTSINFOEXW: as measured on an
    unlocked console and a connected Remote Desktop session (flags 1), and on a session locked at the console
    and one locked by a Remote Desktop disconnect (flags 0, state 4), 2026-11-15."""
    import struct
    import sys

    def info(flags, level=1, state=0):
        return struct.pack("<IIIIi", level, 0, 1, state, flags)
    assert desktop.screen_locked("win32", lambda: info(0)) is True
    assert desktop.screen_locked("win32", lambda: info(0, state=4)) is True          # locked and disconnected
    assert desktop.screen_locked("win32", lambda: info(1)) is False
    assert desktop.screen_locked("win32", lambda: info(-1)) is None                  # Windows doesn't know
    assert desktop.screen_locked("win32", lambda: info(0, level=2)) is None          # not the layout read here
    assert desktop.screen_locked("win32", lambda: b"") is None                       # the question failed
    assert desktop.screen_locked("win32", lambda: 1 / 0) is None                     # never raises
    assert desktop.screen_locked("darwin") is None and desktop.screen_locked("linux") is None
    assert desktop.screen_locked() in ((True, False, None) if sys.platform == "win32" else (None,))   # the real one


def test_the_window_is_tried_in_a_child_process_on_linux_only(monkeypatch):
    import subprocess
    import types

    import app
    ran = []

    def fake_run(cmd, **kw):
        ran.append(cmd)
        return types.SimpleNamespace(returncode=ran_code[0], stderr=b"Could not initialize GLX")
    monkeypatch.setattr(subprocess, "run", fake_run)
    ran_code = [3]                                   # the child aborted or never showed the page
    monkeypatch.setattr(app.sys, "platform", "linux")
    assert app.window_works() is False and ran[-1][-1] == "--probe-window"
    ran_code[0] = 0
    assert app.window_works() is True
    monkeypatch.setattr(app.sys, "platform", "darwin")
    n = len(ran)
    assert app.window_works() is True and len(ran) == n      # macOS and Windows: not tried, always their web view


def test_the_macos_app_says_why_it_wants_the_camera_and_the_local_network(tmp_path):
    """Info.plist: without the camera and microphone texts macOS refuses them; macOS 15+ asks before the app reaches
    the shared database or another device, with the local-network text."""
    import plistlib

    import build_exe
    app = tmp_path / "FaceRecognizer.app"
    (app / "Contents").mkdir(parents=True)
    (app / "Contents" / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": build_exe.BUNDLE_ID}))
    build_exe.finish_macos_app(app, "1.2.3")
    info = plistlib.loads((app / "Contents" / "Info.plist").read_bytes())
    for key in ("NSCameraUsageDescription", "NSMicrophoneUsageDescription", "NSLocalNetworkUsageDescription"):
        assert len(info[key]) > 40, key
    assert info["NSBonjourServices"] == ["_lmstudio._tcp", "_http._tcp"]
    assert (info["CFBundleShortVersionString"], info["LSMinimumSystemVersion"]) == ("1.2.3", "14.0")
    assert info["CFBundleIdentifier"] == build_exe.BUNDLE_ID                # what PyInstaller wrote stays


def test_the_macos_window_lets_only_this_apps_tracker_use_the_camera_unasked(monkeypatch):
    """app.camera_answer: WebKit asks the window about each page that wants the camera; the tracker this window
    started is granted, anything else is left to the user."""
    import app

    class Origin:                                    # as WebKit's WKSecurityOrigin
        def __init__(self, host, port, scheme="http"):
            self.parts = scheme, host, port

        def protocol(self):
            return self.parts[0]

        def host(self):
            return self.parts[1]

        def port(self):
            return self.parts[2]
    monkeypatch.setitem(app.STATUS, "track", {})
    assert app.camera_answer(Origin("127.0.0.1", 8766)) == app.ASK_USER         # no tracker yet
    monkeypatch.setitem(app.STATUS, "track", {"url": "http://127.0.0.1:8766/"})
    monkeypatch.setattr(app, "OWN", {})
    assert app.camera_answer(Origin("127.0.0.1", 8766)) == app.ASK_USER         # not one this process started
    monkeypatch.setitem(app.OWN, "http://127.0.0.1:8766/", None)
    assert app.camera_answer(Origin("127.0.0.1", 8766)) == app.GRANT
    for other in (Origin("127.0.0.1", 8765), Origin("127.0.0.1", 0), Origin("localhost", 8766),
                  Origin("127.0.0.1", 8766, "https"), Origin("example.com", 8766), object()):
        assert app.camera_answer(other) == app.ASK_USER
    monkeypatch.setattr(app.sys, "platform", "linux")
    assert app.allow_camera_macos() is False                                # the Mac's window only


def test_the_linux_build_leaves_graphics_libraries_to_the_system(tmp_path):
    import build_exe
    names = ["libgbm.so.1", "libX11.so.6", "libGLX_mesa.so.0", "libwayland-client.so.0",      # the system's own
             "libdrm-b0291a67.so.2.4.0", "libX11-xcb-b482bfde.so.1.0.0", "libQt6Core.so.6", "libglib-2.0.so.0"]
    for n in names:
        (tmp_path / n).write_bytes(b"")
    gone = build_exe.drop_system_libs(tmp_path)
    assert sorted(gone) == sorted(names[:4])
    assert sorted(p.name for p in tmp_path.iterdir()) == sorted(names[4:])   # private copies and Qt stay


def test_the_app_is_built_with_its_own_icon_on_every_system():
    """icons/ (tools/make_icon.py): the .exe's and the .app's icon, and packed into every build for the pages'
    favicon, the Linux window and its launcher. Read here without Pillow, as the checks run without it."""
    import struct

    import build_exe
    icons = build_exe.HERE / "icons"
    ico = (icons / "icon.ico").read_bytes()
    reserved, kind, count = struct.unpack("<HHH", ico[:6])
    sizes = {ico[6 + 16 * k] or 256 for k in range(count)}
    assert (reserved, kind) == (0, 1) and sizes == {16, 20, 24, 32, 40, 48, 64, 128, 256}     # small to large: sharp everywhere
    icns = (icons / "icon.icns").read_bytes()
    assert icns[:4] == b"icns" and struct.unpack(">I", icns[4:8])[0] == len(icns)
    for name, side in (("icon.png", 512), ("icon-256.png", 256), ("icon-64.png", 64)):
        png = (icons / name).read_bytes()
        assert png[:8] == b"\x89PNG\r\n\x1a\n" and struct.unpack(">II", png[16:24]) == (side, side)

    def after(cmd, flag):
        return [cmd[i + 1] for i, c in enumerate(cmd) if c == flag]
    win, mac, linux = (build_exe.command(p) for p in ("win32", "darwin", "linux"))
    assert after(win, "--icon") == [str(icons / "icon.ico")] and after(mac, "--icon") == [str(icons / "icon.icns")]
    assert after(linux, "--icon") == []                       # no icon inside a Linux program: the launcher has it
    for cmd, sep in ((win, ";"), (mac, ":"), (linux, ":")):
        assert {f"{icons / f}{sep}icons" for f in build_exe.ICONS} <= set(after(cmd, "--add-data"))


def test_the_windows_installer_needs_no_administrator_and_never_deletes_data(tmp_path):
    import build_exe
    iss = (build_exe.HERE / "packaging" / "windows" / "FaceRecognizer.iss").read_text("utf-8")
    code = "\n".join(line for line in iss.splitlines() if not line.lstrip().startswith((";", "//")))
    assert "PrivilegesRequired=lowest" in code and r"{localappdata}\Programs\FaceRecognizer" in code   # the app writes beside itself
    assert "{autoprograms}" in code and "{autodesktop}" in code and r"SetupIconFile=..\..\icons\icon.ico" in code
    assert f"OutputBaseFilename={build_exe.INSTALLERS['win32'][:-4]}" in code
    for never in ("[UninstallDelete]", "[InstallDelete]", "DelTree", "DeleteFile", "RemoveDir"):
        assert never not in code                               # models\ and data\ are the user's: an uninstall leaves them
    # the app open while installing: Windows' Restart Manager can't close it (the install stopped), so the script does
    assert "CloseApplications=no" in code and "function CloseApp" in code and "PrepareToInstall" in code
    assert [line for line in code.splitlines() if line.startswith("Source:")] == \
        [r'Source: "..\..\build\dist\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion']        # only the program
    made = tmp_path / "Programs" / "Inno Setup 6"
    made.mkdir(parents=True)
    (made / "ISCC.exe").write_bytes(b"")
    assert build_exe.inno_setup({"LOCALAPPDATA": str(tmp_path)}) in (str(made / "ISCC.exe"), build_exe.shutil.which("iscc"))


def test_the_windows_build_is_signed_only_when_told_with_what(tmp_path, monkeypatch):
    """build_exe's Windows code signing: nothing set means an unsigned build, half of it set stops the build, and
    the installer's Sign block (the uninstaller and Setup itself) is switched on only for a signed one."""
    import json
    import re

    import build_exe
    import pytest
    thumb = "42614539A547167F5D35BFC1E9706F9F07D85776"
    azure = {"FACE_SIGN_ACCOUNT": "acct", "FACE_SIGN_PROFILE": "prof", "FACE_SIGN_ENDPOINT": "https://eus.codesigning.azure.net"}
    assert build_exe.signing({}) is None
    assert build_exe.signing({"FACE_SIGN_ACCOUNT": "", "FACE_SIGN_PROFILE": " ", "FACE_SIGN_CERT": ""}) is None   # CI without the secrets
    assert build_exe.signing(azure) == ("azure", {"account": "acct", "profile": "prof", "endpoint": azure["FACE_SIGN_ENDPOINT"]})
    assert build_exe.signing({"FACE_SIGN_CERT": "42 61 45 39 a5 47 16 7f 5d 35 bf c1 e9 70 6f 9f 07 d8 57 76"}) == ("cert", thumb)
    for wrong in ({"FACE_SIGN_ACCOUNT": "acct"},                                      # half of it
                  {**azure, "FACE_SIGN_CERT": thumb},                                  # both
                  {**azure, "FACE_SIGN_ENDPOINT": "https://eus.codesigning.azure.net.example.com/"},   # somewhere else
                  {"FACE_SIGN_CERT": "1234"}):
        with pytest.raises(SystemExit):
            build_exe.signing(wrong)

    cmd = build_exe.sign_command("signtool.exe", ("cert", thumb), ["a.exe"], tmp_path)
    assert cmd[:2] == ["signtool.exe", "sign"] and cmd[cmd.index("/sha1") + 1] == thumb and cmd[-1] == "a.exe"
    assert cmd[cmd.index("/tr") + 1] == build_exe.TIMESTAMP and cmd[cmd.index("/fd") + 1] == "SHA256"   # dated, SHA-256
    monkeypatch.setattr(build_exe, "sign_dlib", lambda work: tmp_path / "plugin.dll")
    cmd = build_exe.sign_command("signtool.exe", build_exe.signing(azure), ["a.exe", "b.exe"], tmp_path)
    assert cmd[cmd.index("/dlib") + 1] == str(tmp_path / "plugin.dll") and cmd[-2:] == ["a.exe", "b.exe"]
    meta = json.loads((tmp_path / "metadata.json").read_text("utf-8"))
    assert cmd[cmd.index("/dmdf") + 1] == str(tmp_path / "metadata.json")
    assert (meta["Endpoint"], meta["CodeSigningAccountName"], meta["CertificateProfileName"]) == \
        (azure["FACE_SIGN_ENDPOINT"], "acct", "prof")
    # signing in is left to the environment and the Azure CLI (CI: azure/login); nothing that opens a window
    assert not {"AzureCliCredential", "EnvironmentCredential"} & set(meta["ExcludeCredentials"])
    assert "InteractiveBrowserCredential" in meta["ExcludeCredentials"]

    plain, signed = build_exe.iscc_args("1.2.3", tmp_path, False), build_exe.iscc_args("1.2.3", tmp_path, True)
    assert not [a for a in plain if a.startswith(("/S", "/DSign"))] and plain[-1].endswith("FaceRecognizer.iss")
    assert "/DSign=1" in signed and [a for a in signed if a.startswith("/Ssign=$q") and a.endswith("--sign-file $f")]
    iss = (build_exe.HERE / "packaging" / "windows" / "FaceRecognizer.iss").read_text("utf-8")
    assert "#ifdef Sign\nSignTool=sign\nSignedUninstaller=yes\n#endif" in iss.replace("\r\n", "\n")
    ver, size, sha = build_exe.SIGN_DLIB
    assert size > 0 and re.fullmatch(r"[0-9a-f]{64}", sha) and f"/{ver}/" in build_exe.SIGN_DLIB_URL.format(ver)


def test_the_signing_plug_in_is_taken_only_as_pinned_and_signtool_only_when_new_enough(tmp_path, monkeypatch):
    import hashlib
    import io
    import zipfile

    import build_exe
    import pytest
    packed = io.BytesIO()
    with zipfile.ZipFile(packed, "w") as z:
        for name in ("bin/x64/Azure.CodeSigning.Dlib.dll", "bin/x64/Azure.Core.dll", "bin/x86/Azure.CodeSigning.Dlib.dll",
                     "docs/README.md", "bin/x64/../../../outside.dll"):
            z.writestr(name, b"x")
    raw = packed.getvalue()

    class Answer(io.BytesIO):
        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False
    monkeypatch.setattr(build_exe.urllib.request, "urlopen", lambda req, timeout=None: Answer(raw))
    monkeypatch.setattr(build_exe, "SIGN_DLIB", ("9.9.9", len(raw), "0" * 64))
    with pytest.raises(SystemExit):                              # not the file written down: nothing unpacked
        build_exe.sign_dlib(tmp_path)
    assert not list(tmp_path.rglob("*.dll"))
    monkeypatch.setattr(build_exe, "SIGN_DLIB", ("9.9.9", len(raw), hashlib.sha256(raw).hexdigest()))
    dll = build_exe.sign_dlib(tmp_path)
    assert dll == tmp_path / "dlib-9.9.9" / "bin" / "x64" / "Azure.CodeSigning.Dlib.dll" and dll.is_file()
    assert sorted(p.name for p in tmp_path.rglob("*") if p.is_file()) == ["Azure.CodeSigning.Dlib.dll", "Azure.Core.dll"]

    kits = tmp_path / "pf" / "Windows Kits" / "10" / "bin"
    for sdk in ("10.0.19041.0", "10.0.20348.0"):                 # older than the plug-in works with
        (kits / sdk / "x64").mkdir(parents=True)
        (kits / sdk / "x64" / "signtool.exe").write_bytes(b"")
    assert build_exe.signtool({"ProgramFiles(x86)": str(tmp_path / "pf")}) is None
    for sdk in ("10.0.22621.0", "10.0.26100.0"):
        (kits / sdk / "x64").mkdir(parents=True)
        (kits / sdk / "x64" / "signtool.exe").write_bytes(b"")
    assert build_exe.signtool({"ProgramFiles(x86)": str(tmp_path / "pf")}) == str(kits / "10.0.26100.0" / "x64" / "signtool.exe")


def test_the_linux_installer_is_one_file_that_installs_for_this_user(tmp_path):
    """build_exe.linux_installer: install.sh, the line __APP__, the app as .tar.gz. Run for real on Linux: the app,
    its menu entry, icons and desktop icon appear in a home folder of its own; an update replaces the app; the
    uninstall removes all of it and leaves the models and data alone."""
    import io
    import os
    import subprocess
    import sys
    import tarfile

    import build_exe
    app = tmp_path / "built" / "FaceRecognizer"
    (app / "_internal" / "icons").mkdir(parents=True)
    (app / "FaceRecognizer").write_bytes(b"#!/bin/sh\necho started\n")
    os.chmod(app / "FaceRecognizer", 0o755)
    for f in build_exe.ICONS:
        (app / "_internal" / "icons" / f).write_bytes((build_exe.HERE / "icons" / f).read_bytes())
    run = build_exe.linux_installer(app, tmp_path / build_exe.INSTALLERS["linux"])
    script, _, packed = run.read_bytes().partition(b"\n__APP__\n")
    assert script.startswith(b"#!/bin/sh\n") and b"\r" not in script and b"sudo" not in script.replace(b"(no sudo)", b"")
    with tarfile.open(fileobj=io.BytesIO(packed), mode="r:gz") as tar:
        assert "FaceRecognizer/FaceRecognizer" in tar.getnames() and "FaceRecognizer/_internal/icons/icon.png" in tar.getnames()
    if not sys.platform.startswith("linux"):
        return
    import platform
    home = tmp_path / "home"
    share = home / ".local" / "share"
    (home / "Desktop").mkdir(parents=True)
    (share / "FaceRecognizer" / "data").mkdir(parents=True)             # the user's models and data, from before
    (share / "FaceRecognizer" / "data" / "labels.json").write_text("{}")
    env = {"HOME": str(home), "PATH": os.environ["PATH"]}

    def sh(*args):
        return subprocess.run(["sh", *args], env=env, capture_output=True, text=True, timeout=60)
    r = sh(str(run))
    if platform.machine() != "x86_64":
        assert r.returncode == 1 and "x86_64" in r.stdout
        return
    assert r.returncode == 0, r.stdout + r.stderr
    installed, entry = home / ".local" / "lib" / "FaceRecognizer", share / "applications" / "FaceRecognizer.desktop"
    icon = share / "icons" / "hicolor" / "512x512" / "apps" / "FaceRecognizer.png"
    text = entry.read_text()
    assert f'Exec="{installed}/FaceRecognizer"' in text and f"Icon={icon}" in text and "Name=Face Recognizer" in text
    assert icon.read_bytes() == (build_exe.HERE / "icons" / "icon.png").read_bytes()
    assert (home / "Desktop" / "FaceRecognizer.desktop").read_text() == text
    assert subprocess.run([str(home / ".local" / "bin" / "facerecognizer")], capture_output=True, text=True).stdout == "started\n"
    (installed / "old-file").write_text("from the version before")
    assert sh(str(run), "--no-desktop").returncode == 0 and not (installed / "old-file").exists()   # an update: all new
    assert sh(str(run), "--what").returncode == 2
    r = sh(str(installed / "uninstall.sh"))
    assert r.returncode == 0 and "left in" in r.stdout
    assert not installed.exists() and not entry.exists() and not icon.exists()
    assert not (home / "Desktop" / "FaceRecognizer.desktop").exists() and not (home / ".local" / "bin" / "facerecognizer").exists()
    assert (share / "FaceRecognizer" / "data" / "labels.json").read_text() == "{}"                 # never touched


def test_private_files_are_this_users_alone(tmp_path):
    """Keys and passwords: mode 600 outside Windows (there the folder is the user's own), made that way rather
    than written first and restricted afterwards."""
    import os
    import sys

    import paths
    f = tmp_path / "key.json"
    f.write_text("old")
    if sys.platform != "win32":
        os.chmod(f, 0o644)
        (tmp_path / "key.json.tmp").write_text("left over")
    paths.write_private(f, "secret")
    assert f.read_text() == "secret" and not (tmp_path / "key.json.tmp").exists()
    if sys.platform != "win32":
        assert f.stat().st_mode & 0o777 == 0o600


def test_a_file_that_cant_be_read_is_not_an_empty_file(tmp_path, monkeypatch):
    """Settings, alerts, descriptions and the Web search counts are read, changed and written back whole. A read that
    fails for a moment (another program has the file open) must never count as "nothing there yet"."""
    import pytest
    from pathlib import Path

    import alerts
    import devicesettings
    import face_id as F
    import paths
    assert paths.read_json(tmp_path / "none.json", {"new": True}) == {"new": True}        # no file: the default
    (tmp_path / "bad.json").write_text("{not json")
    assert paths.read_json(tmp_path / "bad.json", []) == []
    monkeypatch.setattr(paths, "HOME", tmp_path)
    monkeypatch.setattr(F, "ALERTS", tmp_path / "alerts.json")
    devicesettings.update("track", {"ft-camera": "abc"})
    alerts.add({"kind": "arrived", "name": "Ann", "text": "Ann arrived"})
    real, busy = Path.read_text, {"left": 3}

    def read_text(self, *a, **k):
        if busy["left"]:
            busy["left"] -= 1
            raise PermissionError("in use")
        return real(self, *a, **k)
    monkeypatch.setattr(Path, "read_text", read_text)
    assert devicesettings.settings("track") == {"ft-camera": "abc"}                        # busy for a moment: waited
    busy["left"] = 10 ** 6                                                                 # busy for good
    with pytest.raises(OSError):
        devicesettings.update("track", {"ft-o-optMesh": "1"})
    with pytest.raises(OSError):
        alerts.add({"kind": "left", "name": "Ann", "text": "Ann left"})
    busy["left"] = 0
    assert devicesettings.settings("track") == {"ft-camera": "abc"}                        # nothing was written over
    assert [a["text"] for a in alerts.listing()["alerts"]] == ["Ann arrived"]


def test_the_linux_app_keeps_numpys_maths_to_one_thread():
    """Set when app.py is loaded, before numpy can be: several OpenBLAS thread pools at once got a matrix product in the
    packed Linux app stuck for good."""
    import os
    import sys

    import app  # noqa: F401
    if sys.platform.startswith("linux"):
        assert os.environ.get("OPENBLAS_NUM_THREADS") == "1"
