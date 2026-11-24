//! What a computer needs beside the program and the models, each from its publisher's own package, pinned by
//! size and SHA-256, and only the named files taken out of it:
//!
//! - the model runtime (ONNX Runtime 1.24.4): Microsoft's own packages (wheels, which are zip archives); the
//!   libraries in them are the ones every measurement of this project was made with. A Mac has 1.31.1: on
//!   Apple's M4 and later (Arm SME2) 1.24.4's convolution keeps a table for every picture it is given and never
//!   drops one, 4 to 6 MB for each camera frame: the tracker's process on an M5 passed 200 GB in twenty minutes
//!   (measured 2026-11-22). 1.31.1 keeps nothing and gives the same faces (prints 0.01% apart). Before a Mac's
//!   pin is changed, `crates/face-core/examples/grows.rs` is run with the new library on such a Mac;
//! - MediaPipe's web runtime (`@mediapipe/tasks-vision` 1.2.3 from the npm registry), for the tracker page;
//! - FFmpeg 7.1 as two programs (an LGPL build), for videos;
//! - libheif 1.23.6 with libde265 1.1.3, for HEIC: built here from their publishers' source with CMake, as
//!   two libraries of their own (LGPL).
//!
//! The libraries and programs go to `<root>/runtime`, where the programs look for them by themselves.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use flate2::read::GzDecoder;

use crate::{
    fetch::{Failure, Fetched, Net, Pin, is_file, partial, save},
    zip,
};

/// A file to take out of an archive: (its name in the archive, or the end of it after a "/"; the name it is
/// kept under; its size; its SHA-256).
pub type Kept = (&'static str, &'static str, u64, &'static str);

/// An archive and what is taken out of it.
pub struct Package {
    pub what: &'static str,
    pub pin: Pin,
    pub files: &'static [Kept],
}

/// This computer's kind, as the lists below name it.
pub fn system() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("macos", "aarch64") => "macos-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => "another system",
    }
}

macro_rules! wheel {
    ($path:literal) => {
        concat!("https://files.pythonhosted.org/packages/", $path)
    };
}

/// The model runtime for a kind of computer. On Windows the build with DirectML (any DirectX 12 graphics
/// processor), with the DirectML library it was made for beside it.
pub fn runtime(system: &str) -> Option<Package> {
    Some(match system {
        "windows-x64" => Package {
            what: "the model runtime (ONNX Runtime 1.24.4 with DirectML)",
            pin: Pin {
                url: wheel!(
                    "88/ea/33814eb0ec96775eda4c1d30b0d86e91d7d2cd0d84c66d3915aef0e06fa3/onnxruntime_directml-1.24.4-cp312-cp312-win_amd64.whl"
                ),
                size: 25_111_930,
                sha256: "f2ecb68b7b7b259d2ef3112ae760149f9b5a1e7c0fbb73d539da6250a648a614",
            },
            files: &[
                (
                    "onnxruntime/capi/onnxruntime.dll",
                    "onnxruntime.dll",
                    21_111_832,
                    "302c69f9779d63ef4ab90316e59444c4acbaca7fe3455020d79d10bcfcb00715",
                ),
                (
                    "onnxruntime/capi/DirectML.dll",
                    "DirectML.dll",
                    18_527_776,
                    "b73972115320e906a49602f2027a3266622881b0d325ba685e0f165a9482a8d7",
                ),
                (
                    "onnxruntime/capi/onnxruntime_providers_shared.dll",
                    "onnxruntime_providers_shared.dll",
                    21_576,
                    "f6bbae3af4c0673b0d1e6ca8db900dc85ad959a395b69d361038ec7995cc2652",
                ),
            ],
        },
        "windows-arm64" => Package {
            what: "the model runtime (ONNX Runtime 1.24.4)",
            pin: Pin {
                url: wheel!(
                    "43/06/8b8ec6e9e6a474fcd5d772453f627ad4549dfe3ab8c0bf70af5afcde551b/onnxruntime-1.24.4-cp312-cp312-win_arm64.whl"
                ),
                size: 12_270_275,
                sha256: "e6214096e14b7b52e3bee1903dc12dc7ca09cb65e26664668a4620cc5e6f9a90",
            },
            files: &[
                (
                    "onnxruntime/capi/onnxruntime.dll",
                    "onnxruntime.dll",
                    15_476_760,
                    "8074abb34bee85312d49c39f1cde276d97aa5c27f5fe370727c8a46192086a0f",
                ),
                (
                    "onnxruntime/capi/onnxruntime_providers_shared.dll",
                    "onnxruntime_providers_shared.dll",
                    21_536,
                    "159db864e38997f35ddbdb4f85abde78156d2f3665e2740d54c8128ba39ae096",
                ),
            ],
        },
        "macos-arm64" => Package {
            // (not 1.24.4 as the others: see the top of this file)
            what: "the model runtime (ONNX Runtime 1.31.1)",
            pin: Pin {
                url: wheel!(
                    "0e/f7/cecc0efeeea83bff2f7789ae1b80f27fa0f0210d6d9d66d1f0031a6b5e47/onnxruntime-1.31.1-cp312-cp312-macosx_14_0_arm64.whl"
                ),
                size: 20_882_125,
                sha256: "96dd2564b10ea7153ad9c207e7cd8e5917cb3ba521e0e0a62ac03486fdc8d56c",
            },
            files: &[(
                "onnxruntime/capi/libonnxruntime.1.31.1.dylib",
                "libonnxruntime.dylib",
                33_037_336,
                "d586cbe89effc085a09d3341b3cfbd5c322517f9d397aeb20814af21433f2b36",
            )],
        },
        "linux-x64" => Package {
            what: "the model runtime (ONNX Runtime 1.24.4)",
            pin: Pin {
                url: wheel!(
                    "6d/ab/5b68110e0460d73fad814d5bd11c7b1ddcce5c37b10177eb264d6a36e331/onnxruntime-1.24.4-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl"
                ),
                size: 17_244_912,
                sha256: "0d640eb9f3782689b55cfa715094474cd5662f2f137be6a6f847a594b6e9705c",
            },
            files: &[
                (
                    "onnxruntime/capi/libonnxruntime.so.1.24.4",
                    "libonnxruntime.so",
                    22_159_232,
                    "2d518e11c7767f9fdc4413bdbeb45acf30db504ff76a82fc19d9238a3f9f6c24",
                ),
                (
                    "onnxruntime/capi/libonnxruntime_providers_shared.so",
                    "libonnxruntime_providers_shared.so",
                    14_632,
                    "086ec1d5388f64153d9c63470d126693db9a182c8ce236d3a1119068471b8a0d",
                ),
            ],
        },
        "linux-arm64" => Package {
            what: "the model runtime (ONNX Runtime 1.24.4)",
            pin: Pin {
                url: wheel!(
                    "aa/60/c4d1c8043eb42f8a9aa9e931c8c293d289c48ff463267130eca97d13357f/onnxruntime-1.24.4-cp312-cp312-manylinux_2_27_aarch64.manylinux_2_28_aarch64.whl"
                ),
                size: 15_172_485,
                sha256: "1a5c5a544b22f90859c88617ecb30e161ee3349fcc73878854f43d77f00558b5",
            },
            files: &[
                (
                    "onnxruntime/capi/libonnxruntime.so.1.24.4",
                    "libonnxruntime.so",
                    18_625_376,
                    "f373ae5fd4d878ed17ebea26e5480bd3d0a15e3bd6bc94b878e6c1b73998d22a",
                ),
                (
                    "onnxruntime/capi/libonnxruntime_providers_shared.so",
                    "libonnxruntime_providers_shared.so",
                    198_792,
                    "3b6be288fbfb7dff8770d08a23defdde18e8f7e0f5a2b344a0e5e238c999ea88",
                ),
            ],
        },
        _ => return None,
    })
}

/// FFmpeg 7.1 as programs, for a kind of computer: the version whose frames every library was made
/// with (FFmpeg 8.1 gives other frames for some videos). An LGPL build.
pub fn ffmpeg(system: &str) -> Option<Package> {
    match system {
        "windows-x64" => Some(Package {
            what: "FFmpeg 7.1 (videos)",
            pin: Pin {
                url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-07-31-14-10/ffmpeg-n7.1.5-12-g1fdbca85aa-win64-lgpl-7.1.zip",
                size: 139_452_771,
                sha256: "b7c1c846dacca68ee4ebf5c390742c973b3d5d14a6d44b061f500d8e4ac74fc0",
            },
            files: &[
                (
                    "bin/ffmpeg.exe",
                    "ffmpeg.exe",
                    109_306_880,
                    "3e79ae0855dce679f3911069df641e9da7dc82aec76807205607338218cd956f",
                ),
                (
                    "bin/ffprobe.exe",
                    "ffprobe.exe",
                    109_169_664,
                    "ee8e8a69735563cce488d35f719d0b6e41f22ea8f8fd01271583ea5afe154f58",
                ),
            ],
        }),
        _ => None,
    }
}

/// Whether every file of a package is in `folder`, each with its size and SHA-256.
pub fn have(files: &[Kept], folder: &Path) -> bool {
    files.iter().all(|(_, name, size, sha256)| is_file(&folder.join(name), *size, sha256))
}

/// Download a package and take its files out into `folder`, unless they are there. False: nothing to do.
/// The archive is kept only until its files are out; anything else in it is never written.
pub fn get(net: &Net, package: &Package, folder: &Path, progress: &mut dyn FnMut(u64, u64)) -> Fetched<bool> {
    if have(package.files, folder) {
        return Ok(false);
    }
    std::fs::create_dir_all(folder)?;
    let archive = partial(&folder.join("package"));
    let taken = (|| -> Fetched<()> {
        net.download(&package.pin, &archive, &mut |got| progress(got, package.pin.size))?;
        let src = zip::OnDisk::open(&archive)?;
        let listed = zip::entries(&src)?;
        for (member, name, size, sha256) in package.files {
            save(zip::open(&src, zip::find(&listed, member)?)?, &folder.join(name), *size, sha256, &mut |_| ())?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&archive);
    taken.map(|()| true)
}

/// The Visual C++ runtime's libraries that ONNX Runtime's library names (Microsoft builds it that way, in
/// every package of it): not part of Windows, so they are shipped beside it. An installer packs them with
/// the rest of the folder; here they are copied from this computer's Visual Studio, where the build tools
/// that made the programs have them for exactly this (`VC\Redist\MSVC\<version>\x64\Microsoft.VC*.CRT`).
pub const VC_RUNTIME: [&str; 4] = ["msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll"];

/// Where Visual Studio keeps the runtime's libraries to ship with a program, for this kind of processor.
fn vc_redist() -> Option<PathBuf> {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let crt_in = |versions: &Path| -> Option<PathBuf> {
        let mut found: Vec<PathBuf> = (std::fs::read_dir(versions).ok()?.flatten())
            .flat_map(|v| std::fs::read_dir(v.path().join(arch)).into_iter().flatten().flatten())
            .map(|d| d.path())
            .filter(|d| {
                d.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".CRT")) && d.join(VC_RUNTIME[0]).is_file()
            })
            .collect();
        found.sort();
        found.pop()
    };
    // a build's own environment names it; else Visual Studio's own finder says where the newest one is
    if let Some(dir) = std::env::var_os("VCToolsRedistDir").map(PathBuf::from) {
        if let Some(found) = dir.parent().and_then(crt_in) {
            return Some(found);
        }
    }
    let finder =
        PathBuf::from(std::env::var_os("ProgramFiles(x86)")?).join("Microsoft Visual Studio/Installer/vswhere.exe");
    let said =
        Command::new(finder).args(["-latest", "-products", "*", "-property", "installationPath"]).output().ok()?;
    crt_in(&PathBuf::from(String::from_utf8_lossy(&said.stdout).trim()).join("VC/Redist/MSVC"))
}

/// The Visual C++ runtime's libraries into `folder`, beside ONNX Runtime's, unless they are there. False:
/// nothing to do (also on a system that isn't Windows).
pub fn get_vc_runtime(folder: &Path) -> Fetched<bool> {
    if !cfg!(windows) || VC_RUNTIME.iter().all(|name| folder.join(name).is_file()) {
        return Ok(false);
    }
    let from = vc_redist().ok_or_else(|| {
        Failure::new(format!(
            "the Visual C++ runtime's libraries ({}) weren't found in a Visual Studio on this computer; ONNX Runtime needs them beside it",
            VC_RUNTIME.join(", ")
        ))
    })?;
    std::fs::create_dir_all(folder)?;
    for name in VC_RUNTIME {
        std::fs::copy(from.join(name), folder.join(name))?;
    }
    Ok(true)
}

/// MediaPipe's web runtime (1.2.3, the version the readings were measured with), as its publisher's web
/// package. Three files of it, for the tracker page (which loads them from its own server, never from the
/// internet). Apache 2.0.
pub const MEDIAPIPE: Pin = Pin {
    url: "https://registry.npmjs.org/@mediapipe/tasks-vision/-/tasks-vision-1.2.3.tgz",
    size: 8_798_983,
    sha256: "469602650dab70256df387983ea44e85fa66020335bae85a54fa8fda062ca0a0",
};
pub const MEDIAPIPE_FILES: [Kept; 3] = [
    (
        "package/vision_bundle.mjs",
        "vision_bundle.mjs",
        155_990,
        "fb0c02c296f44ce0cc324ba16bd646d8773eb4a7847db4019cc4d0430db7a9b5",
    ),
    (
        "package/wasm/vision_wasm_internal.js",
        "wasm/vision_wasm_internal.js",
        336_624,
        "2aa5fd3fdd3d8208cd7587df0bb19fb6925c4ea16a76dd025aedc1f64d6a6309",
    ),
    (
        "package/wasm/vision_wasm_internal.wasm",
        "wasm/vision_wasm_internal.wasm",
        14_665_716,
        "225bc06c0f51a9cf31fea0e9e0b0243e9e01fb5ef332cc1a7fa6656dbdfdaabf",
    ),
];

/// The named files of a packed tar archive (`.tar.gz`) into `folder`, each checked. Nothing else of the
/// archive is written anywhere.
pub fn from_tar(archive: impl Read, files: &[Kept], folder: &Path) -> Fetched<()> {
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    let mut left: Vec<&Kept> = files.iter().collect();
    for entry in tar.entries()? {
        let entry = entry?;
        let name = entry.path()?.to_string_lossy().replace('\\', "/");
        if let Some(at) = left.iter().position(|(member, ..)| *member == name) {
            let (_, keep_as, size, sha256) = left.remove(at);
            save(entry, &folder.join(keep_as), *size, sha256, &mut |_| ())?;
        }
    }
    match left.first() {
        None => Ok(()),
        Some((member, ..)) => Err(Failure::new(format!("the archive doesn't hold {member}"))),
    }
}

/// MediaPipe's web runtime into `<models>/mediapipe-web`, unless it is there. False: nothing to do.
pub fn get_mediapipe(net: &Net, models: &Path, progress: &mut dyn FnMut(u64, u64)) -> Fetched<bool> {
    let folder = models.join("mediapipe-web");
    if have(&MEDIAPIPE_FILES, &folder) {
        return Ok(false);
    }
    std::fs::create_dir_all(&folder)?;
    let archive = partial(&folder.join("package"));
    let taken = (|| -> Fetched<()> {
        net.download(&MEDIAPIPE, &archive, &mut |got| progress(got, MEDIAPIPE.size))?;
        from_tar(std::fs::File::open(&archive)?, &MEDIAPIPE_FILES, &folder)
    })();
    let _ = std::fs::remove_file(&archive);
    taken.map(|()| true)
}

/// The HEIC libraries' source, as their publisher releases it: the versions every library's HEIC
/// photos were decoded with.
pub const DE265: Pin = Pin {
    url: "https://github.com/strukturag/libde265/releases/download/v1.1.3/libde265-1.1.3.tar.gz",
    size: 324_452,
    sha256: "554228bd17788c99a7e63b37ab5634722190e6e2bf60c1dcb01cef328e133905",
};
pub const HEIF: Pin = Pin {
    url: "https://github.com/strukturag/libheif/releases/download/v1.23.6/libheif-1.23.6.tar.gz",
    size: 2_373_898,
    sha256: "4484346dc5995319dbc11e3a1c35d0a2ec46511ce370900869337fd2c7033125",
};

/// libheif's and libde265's library files on this system: (as the build names them below its output folder,
/// as the programs look for them).
pub fn heif_names() -> [(&'static str, &'static str); 2] {
    if cfg!(windows) {
        [("bin/heif.dll", "heif.dll"), ("bin/libde265.dll", "libde265.dll")]
    } else if cfg!(target_vendor = "apple") {
        [("lib/libheif.1.dylib", "libheif.1.dylib"), ("lib/libde265.0.dylib", "libde265.0.dylib")]
    } else {
        [("lib/libheif.so.1", "libheif.so.1"), ("lib/libde265.so.0", "libde265.so.0")]
    }
}

/// libde265 as the decoder alone, and libheif with it and nothing else: no other codec, no plugin, no
/// example program. The options that are named are the ones that differ from the projects' own defaults
/// or that must not follow what happens to be installed on the computer.
const DE265_OPTIONS: [&str; 4] =
    ["-DBUILD_SHARED_LIBS=ON", "-DENABLE_SDL=OFF", "-DENABLE_DECODER=OFF", "-DENABLE_ENCODER=OFF"];
const HEIF_OPTIONS: [&str; 32] = [
    "-DBUILD_SHARED_LIBS=ON",
    "-DBUILD_TESTING=OFF",
    // no documentation: where Doxygen happens to be installed libheif builds its own, which then wants
    // Graphviz too and failed the whole build on GitHub's Windows machines (2026-11-21)
    "-DBUILD_DOCUMENTATION=OFF",
    "-DCMAKE_DISABLE_FIND_PACKAGE_Doxygen=ON",
    "-DWITH_EXAMPLES=OFF",
    "-DWITH_GDK_PIXBUF=OFF",
    "-DENABLE_PLUGIN_LOADING=OFF",
    "-DWITH_LIBDE265=ON",
    "-DWITH_LIBDE265_PLUGIN=OFF",
    "-DWITH_X265=OFF",
    "-DWITH_X264=OFF",
    "-DWITH_AOM_DECODER=OFF",
    "-DWITH_AOM_ENCODER=OFF",
    "-DWITH_DAV1D=OFF",
    "-DWITH_RAV1E=OFF",
    "-DWITH_SvtEnc=OFF",
    "-DWITH_KVAZAAR=OFF",
    "-DWITH_UVG266=OFF",
    "-DWITH_VVDEC=OFF",
    "-DWITH_VVENC=OFF",
    "-DWITH_OpenH264_DECODER=OFF",
    "-DWITH_FFMPEG_DECODER=OFF",
    "-DWITH_JPEG_DECODER=OFF",
    "-DWITH_JPEG_ENCODER=OFF",
    "-DWITH_OpenJPEG_DECODER=OFF",
    "-DWITH_OpenJPEG_ENCODER=OFF",
    "-DWITH_OPENJPH_ENCODER=OFF",
    "-DWITH_UNCOMPRESSED_CODEC=OFF",
    "-DWITH_HEADER_COMPRESSION=OFF",
    "-DWITH_LIBSHARPYUV=OFF",
    "-DWITH_WEBCODECS=OFF",
    "-DCMAKE_BUILD_TYPE=Release",
];

fn cmake(args: &[&std::ffi::OsStr], what: &str) -> Fetched<()> {
    let ran = Command::new("cmake").args(args).output().map_err(|e| {
        Failure::new(format!(
            "CMake couldn't be run ({e}): it and a C++ compiler are needed to build the HEIC libraries"
        ))
    })?;
    if ran.status.success() {
        return Ok(());
    }
    // (what went wrong is said on either stream: Visual Studio's build tools write their errors to the first)
    let said = format!("{}\n{}", String::from_utf8_lossy(&ran.stdout), String::from_utf8_lossy(&ran.stderr));
    let lines: Vec<&str> = said.lines().filter(|line| !line.trim().is_empty()).collect();
    let errors: Vec<&str> =
        lines.iter().copied().filter(|line| line.contains("error") || line.contains("Error")).take(8).collect();
    let last = &lines[lines.len().saturating_sub(12)..];
    Err(Failure::new(format!("{what} failed:\n{}\n{}", errors.join("\n"), last.join("\n"))))
}

/// A source archive unpacked below `into`. (An entry that would land outside is refused by the unpacking.)
fn unpack(archive: &Path, into: &Path) -> Fetched<()> {
    std::fs::create_dir_all(into)?;
    let mut tar = tar::Archive::new(GzDecoder::new(std::fs::File::open(archive)?));
    tar.set_preserve_permissions(false);
    tar.set_preserve_mtime(false);
    Ok(tar.unpack(into)?)
}

/// libheif and libde265 built from their pinned source into `folder`, unless they are there (by name: a
/// build isn't the same bytes twice). False: nothing to do. `say` gets a line for each step, which takes
/// minutes. The build's own folder is removed afterwards.
pub fn get_heif(net: &Net, folder: &Path, say: &mut dyn FnMut(&str)) -> Fetched<bool> {
    let names = heif_names();
    if names.iter().all(|(_, name)| folder.join(name).is_file()) {
        return Ok(false);
    }
    let work = folder.join(".build");
    let built = (|| -> Fetched<()> {
        std::fs::create_dir_all(&work)?;
        let out = work.join("out");
        for (pin, name, options) in
            [(&DE265, "libde265-1.1.3", &DE265_OPTIONS[..]), (&HEIF, "libheif-1.23.6", &HEIF_OPTIONS[..])]
        {
            say(&format!("{name}: its source"));
            let archive = work.join(format!("{name}.tar.gz"));
            net.download(pin, &archive, &mut |_| ())?;
            unpack(&archive, &work)?;
            let (source, build) = (work.join(name), work.join(format!("build-{name}")));
            say(&format!("{name}: building (some minutes)"));
            let mut configure: Vec<std::ffi::OsString> = vec![
                "-S".into(),
                source.into(),
                "-B".into(),
                build.clone().into(),
                "-DCMAKE_BUILD_TYPE=Release".into(),
            ];
            // Windows: the C and C++ runtime linked into each library, so that neither needs the Visual C++
            // redistributable, which is not part of Windows (`needs.rs`). Nothing of the runtime's crosses
            // between the two: libheif asks libde265 through its C functions only.
            configure.push("-DCMAKE_POLICY_DEFAULT_CMP0091=NEW".into());
            configure.push("-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded".into());
            // Linux: the same for the C++ library, which a computer needn't have in the version these were
            // built with (not tried on Linux yet: the build there is CI's, see .github/workflows/decoders.yml)
            if cfg!(target_os = "linux") {
                configure.push("-DCMAKE_SHARED_LINKER_FLAGS=-static-libstdc++ -static-libgcc".into());
            }
            configure.push(format!("-DCMAKE_INSTALL_PREFIX={}", out.display()).into());
            configure.push(format!("-DCMAKE_PREFIX_PATH={}", out.display()).into());
            configure.extend(options.iter().map(|o| std::ffi::OsString::from(*o)));
            let args: Vec<&std::ffi::OsStr> = configure.iter().map(|a| a.as_os_str()).collect();
            cmake(&args, &format!("Setting up {name}'s build"))?;
            let build_os = build.as_os_str();
            let step = |more: &[&str]| -> Vec<std::ffi::OsString> {
                let mut a = vec![std::ffi::OsString::from(more[0]), build_os.to_os_string()];
                a.extend(more[1..].iter().map(std::ffi::OsString::from));
                a
            };
            // as many compilers at once as the computer has processors. Left without a number, the Mac's
            // build tool starts every file at once: on a machine with 3 processors that never ended.
            let at_once = std::thread::available_parallelism().map_or(2, |n| n.get()).to_string();
            for (args, what) in [
                (step(&["--build", "--config", "Release", "--parallel", &at_once]), format!("Building {name}")),
                (step(&["--install", "--config", "Release"]), format!("Putting {name} in place")),
            ] {
                let args: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
                cmake(&args, &what)?;
            }
        }
        for (built_as, name) in names {
            // (a library's name can be a link to the file with the whole version: the file itself is copied)
            let from: PathBuf = std::fs::canonicalize(out.join(built_as))?;
            std::fs::copy(from, folder.join(name))?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    built.map(|()| true)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::{
        testing::{Server, folder, sha},
        zip::tests::archive,
    };

    #[test]
    fn a_packages_named_files_are_taken_out_and_nothing_else() {
        let (server, dir) = (Server::start(), folder("package"));
        let (lib, tool) = (vec![3u8; 40_000], b"a program".to_vec());
        let zip = archive(&[
            ("pkg-1.0/capi/lib.dll", &lib, true),
            ("pkg-1.0/bin/tool.exe", &tool, false),
            ("pkg-1.0/other.txt", b"not wanted", true),
        ]);
        let files: &'static [Kept] = Box::leak(Box::new([
            ("pkg-1.0/capi/lib.dll", "lib.dll", lib.len() as u64, sha(&lib)),
            ("bin/tool.exe", "tool.exe", tool.len() as u64, sha(&tool)),
        ]));
        let package = Package {
            what: "a package",
            pin: Pin { url: server.put("/p.whl", &zip), size: zip.len() as u64, sha256: sha(&zip) },
            files,
        };
        let net = Net::new();
        assert!(!have(files, &dir));
        assert!(get(&net, &package, &dir, &mut |_, _| ()).unwrap());
        let mut names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|f| f.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["lib.dll", "tool.exe"]);
        assert_eq!(std::fs::read(dir.join("lib.dll")).unwrap(), lib);
        // there and right: nothing to do, and nothing asked
        assert!(have(files, &dir) && !get(&net, &package, &dir, &mut |_, _| ()).unwrap());
        assert_eq!(server.asked("/p.whl").len(), 1);
        // a file of it changed: fetched again
        std::fs::write(dir.join("tool.exe"), b"something else").unwrap();
        assert!(!have(files, &dir) && get(&net, &package, &dir, &mut |_, _| ()).unwrap() && have(files, &dir));
        // a package that isn't the pinned one: nothing of it is taken out
        let other = folder("package-other");
        server.put(
            "/p.whl",
            &archive(&[("pkg-1.0/capi/lib.dll", b"planted", false), ("pkg-1.0/bin/tool.exe", b"planted", false)]),
        );
        assert!(get(&net, &package, &other, &mut |_, _| ()).is_err());
        assert_eq!(std::fs::read_dir(&other).unwrap().count(), 0);
        for d in [dir, other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn named_files_come_out_of_a_packed_tar_archive() {
        let dir = folder("tar");
        let (a, b) = (b"the bundle".to_vec(), vec![9u8; 5_000]);
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
        for (name, data) in [
            ("package/readme.md", &b"not wanted"[..]),
            ("package/bundle.mjs", &a[..]),
            ("package/wasm/inner.wasm", &b[..]),
        ] {
            let mut head = tar::Header::new_gnu();
            head.set_size(data.len() as u64);
            head.set_mode(0o644);
            head.set_cksum();
            tar.append_data(&mut head, name, data).unwrap();
        }
        let mut packed = tar.into_inner().unwrap();
        packed.flush().unwrap();
        let packed = packed.finish().unwrap();
        let files = [
            ("package/bundle.mjs", "bundle.mjs", a.len() as u64, sha(&a)),
            ("package/wasm/inner.wasm", "wasm/inner.wasm", b.len() as u64, sha(&b)),
        ];
        from_tar(&packed[..], &files, &dir).unwrap();
        assert_eq!(
            (
                std::fs::read(dir.join("bundle.mjs")).unwrap(),
                std::fs::read(dir.join("wasm").join("inner.wasm")).unwrap()
            ),
            (a.clone(), b)
        );
        assert!(!dir.join("readme.md").exists() && !dir.join("package").exists());
        // a file the archive doesn't hold, and one that isn't the pinned file
        assert!(
            from_tar(&packed[..], &[("package/none", "none", 1, sha(b"x"))], &dir)
                .unwrap_err()
                .why
                .contains("doesn't hold")
        );
        assert!(
            from_tar(&packed[..], &[("package/bundle.mjs", "wrong.mjs", a.len() as u64, sha(b"other"))], &dir).is_err()
        );
        assert!(!dir.join("wrong.mjs").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_kind_of_computer_the_project_runs_on_has_its_runtime() {
        for system in ["windows-x64", "windows-arm64", "macos-arm64", "linux-x64", "linux-arm64"] {
            let p = runtime(system).unwrap();
            assert!(
                p.pin.url.starts_with("https://files.pythonhosted.org/packages/") && p.pin.url.ends_with(".whl"),
                "{system}"
            );
            let library = if system.starts_with("windows") {
                "onnxruntime.dll"
            } else if system == "macos-arm64" {
                "libonnxruntime.dylib"
            } else {
                "libonnxruntime.so"
            };
            assert!(p.files.iter().any(|f| f.1 == library) && p.files.iter().all(|f| f.3.len() == 64), "{system}");
        }
        assert!(
            runtime("another system").is_none()
                && ffmpeg("windows-x64").is_some()
                && ffmpeg("another system").is_none()
        );
    }
}
