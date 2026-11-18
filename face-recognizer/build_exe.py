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

    python build_exe.py                   -> build, and on Windows copy FaceRecognizer.exe beside this file
    python build_exe.py --no-copy         -> build only (CI)
    python build_exe.py --installer       -> build, then make this system's installer too
    python build_exe.py --installer-only  -> the installer from the app already in build/dist (CI: after signing)
    python build_exe.py --sign            -> Windows: sign the .exe in build/dist (see "Windows code signing" below)
    python build_exe.py --signed          -> Windows: check that the .exe and the installer are signed and trusted

The installers put the app where it belongs and give it icons (the app's own: icons/, drawn by tools/make_icon.py):

    Windows  build\\dist\\FaceRecognizer-Setup-x64.exe    Inno Setup (packaging/windows/FaceRecognizer.iss): Start menu
                                                        and desktop icons, no administrator rights
    macOS    build/dist/FaceRecognizer-macos-arm64.dmg  the app beside a link to Applications, to drag across
    Linux    build/dist/FaceRecognizer-linux-x64.run    "sh FaceRecognizer-linux-x64.run": the app for this user, in the
                                                        applications menu and on the desktop (packaging/linux/install.sh)

Needs requirements-build.txt installed (on Linux also pywebview[qt], for the app window); the Windows installer
needs Inno Setup 6 (winget install -e --id JRSoftware.InnoSetup; GitHub's Windows machines have it).
"""
import argparse
import hashlib
import io
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Every page file the three servers send: packed into the app (a test checks each page's scripts are listed here)
PAGES = ["tracker_ui.html", "label_ui.html", "app_ui.html", "tracker_ui.js", "label_ui.js", "app_ui.js",
         "expression.js", "signals.js", "settings_bridge.js", "page_store.js", "cameras_ui.html", "cameras_ui.js",
         "ask.js"]
# The app's icon, made by tools/make_icon.py: packed into the app too (the pages' favicon, the Linux window and
# launcher), in a folder of the same name
ICONS = ["icon.ico", "icon.png", "icon-256.png", "icon-64.png"]
INSTALLERS = {"win32": "FaceRecognizer-Setup-x64.exe", "darwin": "FaceRecognizer-macos-arm64.dmg",
              "linux": "FaceRecognizer-linux-x64.run"}
EXCLUDE = ["torch", "torchvision", "torchaudio", "torchgen", "tensorflow", "pandas", "IPython"]
BUNDLE_ID = "io.github.singularityresearch.facerecognizer"
MACOS_MIN = "14.0"                     # onnxruntime's macOS wheels need macOS 14 (Apple Silicon only)
CAMERA_WHY = ("The live tracker recognizes the people you have named, from the camera. Pictures stay on this Mac: "
              "only numbers are kept, unless you turn on Collect faces.")
MIC_WHY = ("The live tracker can measure voice signals (loudness, pitch, speaking rate) from the microphone. "
           "Nothing is recorded.")
# macOS 15 and later ask before an app reaches anything on the local network; until allowed, the connection fails
NETWORK_WHY = ("Face Recognizer reaches the shared database you set up and your other devices (their photos and live "
               "cameras), and finds LM Studio on your network for Describe photo.")
BONJOUR = ["_lmstudio._tcp", "_http._tcp"]      # the services hardware.mdns_services asks for, lmstudio.py announces


def version():
    return re.search(r'^VERSION = "([^"]+)"', (HERE / "app.py").read_text("utf-8"), re.M).group(1)


def command(platform=sys.platform):
    sep = ";" if platform == "win32" else ":"
    cmd = [sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", "--windowed",
           "--name", "FaceRecognizer", "--workpath", "build", "--distpath", str(Path("build") / "dist"),
           "--specpath", "build"]
    if platform == "win32":
        cmd += ["--onefile", "--icon", str(HERE / "icons" / "icon.ico")]
    elif platform == "darwin":
        cmd += ["--osx-bundle-identifier", BUNDLE_ID, "--icon", str(HERE / "icons" / "icon.icns")]
    for f in PAGES:
        cmd += ["--add-data", f"{HERE / f}{sep}."]
    for f in ICONS:
        cmd += ["--add-data", f"{HERE / 'icons' / f}{sep}icons"]
    cmd += ["--add-data", f"{HERE / 'db'}{sep}db"]          # the shared database's schema (facedb.migrate)
    cmd += ["--collect-all", "soundcard"]                   # its interface descriptions (.h files) are read at run time
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
    system refuses them) and for the local network (the shared database, other devices, LM Studio), the oldest
    macOS it runs on, and its version. Signing comes after (CI)."""
    plist = app / "Contents" / "Info.plist"
    info = plistlib.loads(plist.read_bytes())
    info.update({"NSCameraUsageDescription": CAMERA_WHY, "NSMicrophoneUsageDescription": MIC_WHY,
                 "NSLocalNetworkUsageDescription": NETWORK_WHY, "NSBonjourServices": BONJOUR,
                 "LSMinimumSystemVersion": MACOS_MIN, "CFBundleShortVersionString": ver, "CFBundleVersion": ver,
                 "CFBundleDisplayName": "Face Recognizer", "NSHighResolutionCapable": True})
    plist.write_bytes(plistlib.dumps(info))


# ---- installers -------------------------------------------------------------------------------------------------
def inno_setup(env=os.environ):
    """Inno Setup's compiler (ISCC.exe), or None: on the PATH, or where its own installer puts it."""
    places = [Path(env[v]) / "Inno Setup 6" / "ISCC.exe" for v in ("ProgramFiles(x86)", "ProgramFiles") if env.get(v)]
    if env.get("LOCALAPPDATA"):                      # installed for one user (winget --scope user)
        places.append(Path(env["LOCALAPPDATA"]) / "Programs" / "Inno Setup 6" / "ISCC.exe")
    return shutil.which("iscc") or next((str(p) for p in places if p.is_file()), None)


def iscc_args(ver, dist, signed):
    """What Inno Setup's compiler is given. Signed: the script's Sign block is switched on, and its sign tool is
    this file's --sign-file, which Inno Setup runs for the uninstaller it packs and for the finished installer
    ($q is a quote and $f the quoted file, in Inno Setup's own writing)."""
    args = ["/Qp", f"/DAppVersion={ver}", f"/O{dist}"]
    if signed:
        args += ["/DSign=1", f"/Ssign=$q{sys.executable}$q $q{Path(__file__).resolve()}$q --sign-file $f"]
    return args + [str(HERE / "packaging" / "windows" / "FaceRecognizer.iss")]


def windows_installer(dist, ver):
    """FaceRecognizer-Setup-x64.exe from dist/FaceRecognizer.exe (packaging/windows/FaceRecognizer.iss). Signed,
    with its uninstaller, when the build is told with what (signing())."""
    iscc = inno_setup()
    if not iscc:
        sys.exit("The Windows installer needs Inno Setup 6: winget install -e --id JRSoftware.InnoSetup")
    subprocess.run([iscc] + iscc_args(ver, dist, signing() is not None), cwd=HERE, check=True)
    return dist / INSTALLERS["win32"]


# ---- Windows code signing ---------------------------------------------------------------------------------------
# The .exe, the installer and the uninstaller inside it are signed when the environment says with what:
#
#   FACE_SIGN_ACCOUNT, FACE_SIGN_PROFILE, FACE_SIGN_ENDPOINT
#       Azure Artifact Signing (Microsoft's signing service, once called Trusted Signing): the account, its
#       certificate profile, and the address of the account's region (https://eus.codesigning.azure.net, ...).
#       The certificate's key never leaves Microsoft's service: signtool sends it the file's hash, as whoever is
#       signed in to Azure here (az login; CI: azure/login as this repository's workflow, with no stored password)
#       and has the "Artifact Signing Certificate Profile Signer" role on the profile.
#   FACE_SIGN_CERT
#       instead: the SHA-1 thumbprint of a code signing certificate in this user's Windows certificate store (one
#       from a certificate authority, on its hardware token; or a self-signed one, to try the build).
#
# With neither, nothing is signed, as before. Steps for the owner: README, "Signing the Windows app".
SIGN_DLIB = ("1.0.151", 14740357, "35375a4bc4e1102da729fc3d8d9925f774c3baae6b8b7ff6398d948ca9ab0d8a")
SIGN_DLIB_URL = "https://api.nuget.org/v3-flatcontainer/microsoft.artifactsigning.client/{0}/microsoft.artifactsigning.client.{0}.nupkg"
SIGN_DLIB_NET = 10                     # the .NET runtime that version of the plug-in needs (its runtimeconfig.json)
SIGNTOOL_MIN = (10, 0, 22621, 755)     # the oldest signtool the plug-in works with
# A signature is dated by a time server, so it stays good after its certificate ends (Artifact Signing's last 3 days)
TIMESTAMP = "http://timestamp.acs.microsoft.com"
SIGN_AS = ["/d", "Face Recognizer", "/du", "https://github.com/SingularityResearch/facial-recognition"]
# The plug-in tries these ways of signing in to Azure in turn. Left: the environment (AZURE_CLIENT_ID and the
# rest) and the Azure CLI. The others wait for something that isn't there (a managed identity) or open a window.
NOT_THESE_LOGINS = ["ManagedIdentityCredential", "WorkloadIdentityCredential", "SharedTokenCacheCredential",
                    "VisualStudioCredential", "VisualStudioCodeCredential", "AzurePowerShellCredential",
                    "AzureDeveloperCliCredential", "InteractiveBrowserCredential"]


def signing(env=os.environ):
    """What this build is signed with: ("azure", {account, profile, endpoint}), ("cert", thumbprint) or None.
    Half of it set is a mistake, not an unsigned build: it stops."""
    azure = {k: (env.get(f"FACE_SIGN_{k.upper()}") or "").strip() for k in ("account", "profile", "endpoint")}
    cert = re.sub(r"[\s:]", "", env.get("FACE_SIGN_CERT") or "").upper()
    if any(azure.values()):
        missing = [f"FACE_SIGN_{k.upper()}" for k, v in azure.items() if not v]
        if missing or cert:
            sys.exit("Signing: " + (f"{', '.join(missing)} not set" if missing else
                                    "both an Artifact Signing account and FACE_SIGN_CERT are set: choose one"))
        if not re.fullmatch(r"https://[a-z0-9]+\.codesigning\.azure\.net/?", azure["endpoint"]):
            sys.exit("Signing: FACE_SIGN_ENDPOINT isn't an Artifact Signing address (https://<region>.codesigning.azure.net)")
        return "azure", azure
    if cert:
        if not re.fullmatch(r"[0-9A-F]{40}", cert):
            sys.exit("Signing: FACE_SIGN_CERT isn't a certificate's SHA-1 thumbprint (40 hex digits)")
        return "cert", cert
    return None


def signtool(env=os.environ):
    """The newest 64-bit signtool.exe of the Windows SDK (GitHub's Windows machines have one), or None."""
    found = []
    for v in ("ProgramFiles(x86)", "ProgramFiles"):
        kits = Path(env[v]) / "Windows Kits" / "10" / "bin" if env.get(v) else None
        for exe in (kits.glob("10.*/x64/signtool.exe") if kits and kits.is_dir() else []):
            ver = tuple(int(n) for n in exe.parent.parent.name.split(".") if n.isdigit())
            # the SDK folder's name ends in .0 whatever the tool's own build number: compare the first three parts
            if ver[:3] >= SIGNTOOL_MIN[:3]:
                found.append((ver, str(exe)))
    return max(found)[1] if found else None


def sign_dlib(work):
    """signtool's plug-in for Artifact Signing (Azure.CodeSigning.Dlib.dll), unpacked under `work`: Microsoft's
    package from nuget.org, at one version, taken only with the size and SHA-256 written above."""
    ver, size, sha = SIGN_DLIB
    folder = Path(work) / f"dlib-{ver}"
    dll = folder / "bin" / "x64" / "Azure.CodeSigning.Dlib.dll"
    if dll.is_file():
        return dll
    with urllib.request.urlopen(urllib.request.Request(SIGN_DLIB_URL.format(ver),
                                                       headers={"User-Agent": "FaceRecognizer-build"}), timeout=120) as r:
        raw = r.read(size + 1)
    if len(raw) != size or hashlib.sha256(raw).hexdigest() != sha:
        sys.exit("Signing: the Artifact Signing plug-in from nuget.org isn't the expected file (size or SHA-256 differs)")
    shutil.rmtree(folder, ignore_errors=True)
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        for name in z.namelist():
            if re.fullmatch(r"bin/x64/[A-Za-z0-9_.\-]+", name):          # the 64-bit plug-in's own files only
                (folder / name).parent.mkdir(parents=True, exist_ok=True)
                (folder / name).write_bytes(z.read(name))
    return dll


def sign_command(tool, how, files, work):
    """signtool's command line for `how` (signing()). For Artifact Signing it also writes the plug-in's
    description of the account (no secret in it) into `work`."""
    cmd = [tool, "sign", "/fd", "SHA256", "/tr", TIMESTAMP, "/td", "SHA256"] + SIGN_AS
    kind, what = how
    if kind == "cert":
        return cmd + ["/sha1", what] + [str(f) for f in files]
    meta = Path(work) / "metadata.json"
    meta.parent.mkdir(parents=True, exist_ok=True)
    meta.write_text(json.dumps({"Endpoint": what["endpoint"], "CodeSigningAccountName": what["account"],
                                "CertificateProfileName": what["profile"],
                                "ExcludeCredentials": NOT_THESE_LOGINS}, indent=1), "utf-8")
    return cmd + ["/dlib", str(sign_dlib(work)), "/dmdf", str(meta)] + [str(f) for f in files]


def has_dotnet(major=SIGN_DLIB_NET):
    """Whether the .NET runtime the plug-in needs is installed (without it signtool fails and says nothing)."""
    try:
        out = subprocess.run(["dotnet", "--list-runtimes"], capture_output=True, text=True, timeout=60).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return any(int(m.group(1)) >= major for m in re.finditer(r"^Microsoft\.NETCore\.App (\d+)\.", out, re.M))


def sign(files, env=os.environ):
    """Sign `files` (Windows programs) with what signing() says; stops with the reason when it can't."""
    how = signing(env)
    if not how:
        sys.exit("Signing: not set up (FACE_SIGN_ACCOUNT, FACE_SIGN_PROFILE and FACE_SIGN_ENDPOINT, or FACE_SIGN_CERT)")
    tool = signtool(env)
    if not tool:
        sys.exit("Signing needs signtool.exe from the Windows SDK: winget install -e --id Microsoft.WindowsSDK.10.0.26100")
    if how[0] == "azure" and not has_dotnet():
        sys.exit(f"Signing with Artifact Signing needs the .NET {SIGN_DLIB_NET} runtime: "
                 f"winget install -e --id Microsoft.DotNet.Runtime.{SIGN_DLIB_NET}")
    r = subprocess.run(sign_command(tool, how, files, HERE / "build" / "signing"))
    if r.returncode:
        sys.exit(f"Signing failed (signtool: {r.returncode}). With Artifact Signing: is this computer signed in to "
                 "Azure (az login), with the Signer role on the certificate profile, and is the address the account's region?")
    for f in files:
        print(f"Signed {f}")


def signature(file):
    """A Windows program's signature as Windows itself reads it: {status, signer, thumbprint, timestamped}.
    status is "Valid" only for a signature that fits the file and leads to a certificate authority Windows trusts."""
    ps = ("$s = Get-AuthenticodeSignature -LiteralPath $env:FACE_SIGNED_FILE; "
          "@{status = [string]$s.Status; signer = [string]$s.SignerCertificate.Subject; "
          "thumbprint = [string]$s.SignerCertificate.Thumbprint; "
          "timestamped = $null -ne $s.TimeStamperCertificate} | ConvertTo-Json -Compress")
    # without PSModulePath: started from PowerShell 7, Windows PowerShell inherits its module folders and then
    # can't load its own security module (the command fails, with exit code 0)
    env = {k: v for k, v in os.environ.items() if k.upper() != "PSMODULEPATH"}
    r = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", ps], capture_output=True,
                       text=True, env={**env, "FACE_SIGNED_FILE": str(file)})
    got = json.loads(r.stdout) if r.stdout.strip().startswith("{") else {}
    if not got.get("status"):
        sys.exit(f"Couldn't read the signature of {file}: {r.stderr.strip()[:300] or 'no answer from PowerShell'}")
    return got


def check_signed(files):
    """Stops unless each of `files` is signed, trusted by Windows and dated by a time server."""
    bad = []
    for f in files:
        s = signature(f)
        ok = s["status"] == "Valid" and s["timestamped"]
        print(f"{'Signed' if ok else 'NOT signed and trusted'}: {Path(f).name}: {s['status']}, by {s['signer'] or 'nobody'}"
              f"{'' if s['timestamped'] else ', no time stamp'}")
        bad += [] if ok else [f]
    if bad:
        sys.exit(1)


def macos_installer(dist):
    """FaceRecognizer-macos-arm64.dmg: the app beside a link to Applications, the usual way to install on a Mac.
    Made after signing: the .app inside carries the signature and Apple's ticket."""
    out, stage = dist / INSTALLERS["darwin"], dist / "dmg"
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir(parents=True)
    subprocess.run(["ditto", str(dist / "FaceRecognizer.app"), str(stage / "FaceRecognizer.app")], check=True)
    os.symlink("/Applications", stage / "Applications")
    out.unlink(missing_ok=True)
    subprocess.run(["hdiutil", "create", "-volname", "Face Recognizer", "-srcfolder", str(stage), "-ov",
                    "-format", "UDZO", str(out)], check=True)
    shutil.rmtree(stage, ignore_errors=True)
    return out


def linux_installer(folder, out):
    """A file that installs itself: packaging/linux/install.sh, the line __APP__, then `folder` (the built app) as
    a .tar.gz. The user's computer needs only sh and tar. Plain Python: made the same on any system."""
    script = (HERE / "packaging" / "linux" / "install.sh").read_bytes().replace(b"\r\n", b"\n")
    packed = io.BytesIO()
    with tarfile.open(fileobj=packed, mode="w:gz") as tar:
        tar.add(folder, arcname="FaceRecognizer")
    out = Path(out)
    out.write_bytes(script.rstrip(b"\n") + b"\n__APP__\n" + packed.getvalue())
    os.chmod(out, 0o755)
    return out


def installer(dist, platform=sys.platform):
    """This system's installer, from the app already built in dist."""
    if platform == "win32":
        return windows_installer(dist, version())
    if platform == "darwin":
        return macos_installer(dist)
    return linux_installer(dist / "FaceRecognizer", dist / INSTALLERS["linux"])


def main():
    ap = argparse.ArgumentParser(description="Build the face recognizer app for this system")
    ap.add_argument("--no-copy", action="store_true", help="leave it in build/dist (don't copy the .exe beside this file)")
    ap.add_argument("--installer", action="store_true", help="also make this system's installer (see the top of this file)")
    ap.add_argument("--installer-only", action="store_true", help="only the installer, from the app already in build/dist")
    ap.add_argument("--sign", action="store_true", help="Windows: only sign the .exe already in build/dist")
    ap.add_argument("--signed", action="store_true", help="Windows: check the .exe and the installer in build/dist are signed and trusted")
    ap.add_argument("--sign-file", metavar="FILE", help="Windows: sign this one file (what the installer's build runs)")
    args = ap.parse_args()
    if args.sign_file:
        return sign([args.sign_file])
    if args.sign or args.signed:
        if sys.platform != "win32":
            sys.exit("--sign and --signed are for the Windows build (macOS: packaging/macos/sign_and_notarize.sh)")
        dist = HERE / "build" / "dist"
        if args.sign:
            return sign([dist / "FaceRecognizer.exe"])
        return check_signed([dist / "FaceRecognizer.exe", dist / INSTALLERS["win32"]])
    missing = [f for f in PAGES if not (HERE / f).exists()]
    missing += [f"icons/{f}" for f in ICONS + ["icon.icns"] if not (HERE / "icons" / f).exists()]
    if missing:
        sys.exit(f"Missing files: {', '.join(missing)}")
    dist = HERE / "build" / "dist"
    if args.installer_only:
        print(f"Built {installer(dist)}")
        return
    subprocess.run(command(), cwd=HERE, check=True)
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
    if sys.platform == "win32" and signing():
        sign([out])                    # before it is copied or packed into the installer
    if sys.platform == "win32" and not args.no_copy:
        shutil.copy2(out, HERE / out.name)
    print(f"Built {out}")
    if args.installer:
        print(f"Built {installer(dist)}")


if __name__ == "__main__":
    main()
