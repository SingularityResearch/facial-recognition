//! PostgreSQL with pgvector, for a computer that holds the shared database itself (the owner, 2026-11-25: "We
//! should try to install PostgreSQL ourselves", "Make sure to install pgvector as well"). Fetched only at the
//! owner's click (Settings or the Setup page: "Set up a new one on this computer"), never at a start.
//!
//! Where it comes from: conda-forge's own builds, the one publisher that builds PostgreSQL and pgvector together
//! for every system the app runs on. Each package is pinned by its size and SHA-256 as conda-forge publishes it
//! (taken from the file downloaded once), and only conda-forge's channel is ever asked (never Anaconda's own
//! channels, whose terms bind commercial use). A package is a zip of two zstd-packed tar archives; from the one
//! with the files, only what the server needs is taken: the three programs that make, run and stop it, the
//! libraries they and the extensions load, and the server's shared files. Nothing of a package is used unless it
//! is that very file. What is taken is as it came, but for the folder the package was built in where a file names it
//! (conda's "placeholder", in the package's own list `info/paths.json`): it is put in as the folder the files are in
//! now, as conda itself does when it installs a package (`put_folder`). On a Mac that is the three programs, which
//! look for the time zones there; a program so changed is signed again with the system's own `codesign`, ad hoc as
//! it came (a Mac runs no program whose signature doesn't fit it). Windows' packages are taken as they are: their
//! programs find everything by where they are.
//!
//! What each system gets is fixed with the data folders made with it: PostgreSQL's data folder is of one major
//! version, so a pin of another major version needs the folder upgraded (`pg_upgrade`) and is a change of its own.
//! conda-forge builds pgvector 0.8 for PostgreSQL 16 on Windows and for 18 on a Mac.
//!
//! Left out on purpose: `psql` and what only it needs (readline, which is GPL; ncurses; libedit), the client
//! tools, headers, documents and debug symbols.

use std::{
    collections::HashMap,
    io::Read,
    path::{Component, Path, PathBuf},
};

use crate::{
    fetch::{Failure, Fetched, Net, is_file, save},
    zip,
};

/// Where conda-forge publishes its packages. Another address only for a check, through `FACE_TEST_CONDA`
/// (`http://127.0.0.1:<port>`).
const CONDA_FORGE: &str = "https://conda.anaconda.org/conda-forge/";

/// A package: (its file under conda-forge, size, SHA-256) and what is taken out of it: each a path's beginning,
/// with an end after a `*` (`Library/bin/*.dll`: every file under `Library/bin/` whose name ends in `.dll`).
pub struct Package {
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub take: &'static [&'static str],
}

/// The server for one kind of computer.
pub struct Server {
    /// PostgreSQL's version and pgvector's, as the Settings tab says them
    pub postgres: &'static str,
    pub pgvector: &'static str,
    /// where the programs are in the folder
    pub bin: &'static str,
    pub packages: &'static [Package],
}

const fn p(file: &'static str, size: u64, sha256: &'static str, take: &'static [&'static str]) -> Package {
    Package { file, size, sha256, take }
}

const WIN_DLLS: &[&str] = &["Library/bin/*.dll"];
const MAC_LIBS: &[&str] = &["lib/*.dylib"];
const LINUX_LIBS: &[&str] = &["lib/*.so*"];

/// Windows (x64): PostgreSQL 16.15, pgvector 0.8.6, with Microsoft's C++ runtime beside the programs (a fresh
/// Windows doesn't have it, and Windows looks for it beside the program that needs it).
const WINDOWS: Server = Server {
    postgres: "16.15",
    pgvector: "0.8.6",
    bin: "Library/bin",
    packages: &[
        p(
            "win-64/postgresql-16.15-he837cf3_0.conda",
            20_911_464,
            "4196f32d39459af329869c376fc616f6505af090cc63f713adb2a182033be14a",
            &["Library/bin/postgres.exe", "Library/bin/initdb.exe", "Library/bin/pg_ctl.exe"],
        ),
        // (on Windows the server's modules and shared files are in libpq's package)
        p(
            "win-64/libpq-16.15-h43e12c5_0.conda",
            3_705_899,
            "888a85aae50559ed390d8f1dd78ec708ca8f8a854327c795e2656a1de6ad850d",
            &["Library/bin/libpq.dll", "Library/lib/*.dll", "Library/share/"],
        ),
        p(
            "win-64/pgvector-0.8.6-h2466b09_0.conda",
            151_484,
            "5730998991a2b7349c4253ed7881f339bd43d005949b655edc616cca8caf21dd",
            &["Library/lib/vector.dll", "Library/share/extension/vector"],
        ),
        p(
            "win-64/openssl-3.6.5-hf411b9b_0.conda",
            9_402_647,
            "bc44625e0c6d83f3e02b5f6adc01e0e3ca7001d3d94e24959591a57f5a011b9a",
            WIN_DLLS,
        ),
        p(
            "win-64/icu-78.3-h5112557_2.conda",
            16_835_644,
            "75c549b55b673e15de8785a8e5dd85bca7eb612eee0ff4dc8d7bdaa15eacbdbb",
            WIN_DLLS,
        ),
        p(
            "win-64/krb5-1.22.2-h719d79b_2.conda",
            753_425,
            "63ff03324e903eb01a715ccf357df56d66224e61952fd6615d86490ebefb3285",
            WIN_DLLS,
        ),
        p(
            "win-64/libiconv-1.18-hc1393d2_3.conda",
            694_899,
            "35e04e3ddac7720fc7c550a1c9f998604299d6a7bd1f3ec9b2825d825061daa2",
            WIN_DLLS,
        ),
        p(
            "win-64/libxml2-16-2.15.4-h3cfd58e_1.conda",
            515_690,
            "819c4edd7b7bcfa9461aeccb56c0a7e9fa7c251163333e64222c142499822958",
            WIN_DLLS,
        ),
        p(
            "win-64/libzlib-1.3.2-hfd05255_3.conda",
            58_529,
            "0629c2cc0404d3bb29d6baa7b4ba62da80797015e86de050db81ea5a07050527",
            WIN_DLLS,
        ),
        p(
            "win-64/liblzma-5.8.3-hfd05255_1.conda",
            105_809,
            "d36c4a1e1f80fd08e18a407e03622ff2f34dfdd022da6488ad19603dea19e6d5",
            WIN_DLLS,
        ),
        p(
            "win-64/vc14_runtime-14.52.37029-hfcedb63_45.conda",
            836_810,
            "fc56758530ae98573f0caf9a1270a4e2c77f3c53a2662bac5a6ccd0998f5c94b",
            WIN_DLLS,
        ),
        p(
            "win-64/vcomp14-14.52.37029-h54d7a3b_45.conda",
            166_858,
            "8307d1893da5f6b56bac3c8068e775c08875f3569a97ab9219d33eb3c4015d36",
            WIN_DLLS,
        ),
    ],
};

/// A Mac (Apple silicon): PostgreSQL 18.7, pgvector 0.8.6. The programs find their libraries in `lib/` beside
/// `bin/` by themselves (`@rpath`), and the time zones in `share/zoneinfo` (tzdata).
const MAC: Server = Server {
    postgres: "18.7",
    pgvector: "0.8.6",
    bin: "bin",
    packages: &[
        p(
            "osx-arm64/postgresql-18.7-hbf27387_1.conda",
            4_728_567,
            "ffe513423b674813ffdb481cbaf18fea2ead38234ade3b901eb19c605302bef9",
            &["bin/postgres", "bin/initdb", "bin/pg_ctl", "lib/", "share/"],
        ),
        p(
            "osx-arm64/libpq-18.7-h0a198e3_1.conda",
            2_702_884,
            "331b1ee162b301934bb8bdf5aa32d338bc18a6a312abba19255125fd6fea85b6",
            &["lib/", "share/"],
        ),
        p(
            "osx-arm64/pgvector-0.8.6-h24b1a1f_0.conda",
            84_849,
            "e29a1cd9a6e609e081333112115e8b14dff922d42c74eea87fd2148f3f7accfb",
            &["lib/", "share/extension/vector"],
        ),
        p(
            "noarch/tzdata-2026e-h5d301ff_0.conda",
            119_208,
            "2b7ca858879f183ce2dcc65b300f48a0133d5430550f35b23d80d405a0788dbf",
            &["share/zoneinfo/"],
        ),
        p(
            "osx-arm64/openssl-3.6.5-h55eecbc_0.conda",
            3_111_573,
            "44fbc526456571a9fb8fa18157f6f269d4d4bd40eef20183a93b9c9296f94497",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/icu-78.3-py310h579977c_2.conda",
            14_070_242,
            "6cdb5dee54c72e56ab189fb3ad33cb28533553d42590e7e831160248f4416a43",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/libcxx-23.1.6-hc645fd0_0.conda",
            566_632,
            "822b2a88313977594230b7be745324e4c50293a35cf9244bb19ecc9c2c385288",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/krb5-1.22.2-h34f8a20_2.conda",
            1_165_740,
            "aaea1d42b07769920db2af7471ece9399d2613448863da64ad8272998db5db34",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/openldap-2.7.2-hdd0405a_0.conda",
            861_342,
            "dc54510c191833ef5a020d35be82b2c0b95acf2dead5b406698f9305c3240ce7",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/cyrus-sasl-2.1.28-hb961e35_1.conda",
            194_397,
            "2bb1a8cfc2534b05718c21ffacd806c5c3d5289c9e8be12270d9fc5606c859bf",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/libntlm-1.8-h602ebc4_3.conda",
            33_636,
            "1b3cfed992a37294e57aba932743a445d5fe2d939b3c508e88177331b9c11ff2",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/libxml2-16-2.15.4-heb56d2d_1.conda",
            469_239,
            "1acd4dd49c6f54440dc172ab469d0e2c7c3c02de5b1f21158a233eb585ec3c38",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/libxslt-1.1.45-haaf9281_1.conda",
            221_755,
            "4b40785bdc7af110db10c9b951ce7f82c515bb371046367f0e397428d2db44f4",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/libiconv-1.18-he4c29f2_3.conda",
            750_816,
            "689a14968267f2f97c07112fca5636e7d756036b9aee969911267c20bd3eea1f",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/libzlib-1.3.2-h8088a28_3.conda",
            47_822,
            "a18fa5d5bac452401459f966cf0d872224e8080c4ff93c77e168d43ab42ef9d7",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/liblzma-5.8.3-h8088a28_1.conda",
            91_720,
            "23d0630046a3e8b164d8f80f2b74ed2605af2e7050ab9913018056402fae4311",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/lz4-c-1.11.1-ha89b795_0.conda",
            186_300,
            "9f37a2d03a872312ff1d183fa3d9c1652906e0f9f7391ab90130e49b14976d3f",
            MAC_LIBS,
        ),
        p(
            "osx-arm64/zstd-1.5.7-hf451053_7.conda",
            433_687,
            "da867f5092eb0cb746d353694f0098031fd9817a4ce7d5743121209ae0f406ca",
            MAC_LIBS,
        ),
    ],
};

/// The server for this kind of computer (`parts::system`), where there is one.
/// Linux (x86-64): PostgreSQL 18.7, pgvector 0.8.6, as on a Mac (`lib/` beside `bin/`, `$ORIGIN`; the time zones
/// in `share/zoneinfo`, tzdata, which conda-forge's Linux package doesn't name but its programs look for), with the
/// C++ and GCC runtime libraries it was built with (under GCC's runtime library exception, which is for shipping
/// them with any program). Not the Raspberry Pi (the owner, 2026-11-25: "Rasberry Pi is unecessary (it wont work
/// well.)").
const LINUX: Server = Server {
    postgres: "18.7",
    pgvector: "0.8.6",
    bin: "bin",
    packages: &[
        p(
            "linux-64/postgresql-18.7-haf2d9ce_1.conda",
            5_859_580,
            "d55b78e656de1d3e624dc2758a7634bdf14b2305d960b6e232567764830153e9",
            &["bin/postgres", "bin/initdb", "bin/pg_ctl", "lib/", "share/"],
        ),
        p(
            "linux-64/libpq-18.7-h3883810_1.conda",
            2_719_060,
            "cd7328bbe54b68b31df5094e9712d67c7e2cafe2813de5c410ec2bb3cd2e2242",
            &["lib/", "share/"],
        ),
        p(
            "linux-64/pgvector-0.8.6-h1b92c21_0.conda",
            127_147,
            "c822d543da828d00f48fd67973a8661edcc67342ea7bda8aa51d7d05ee6f4395",
            &["lib/", "share/extension/vector"],
        ),
        p(
            "noarch/tzdata-2026e-h5d301ff_0.conda",
            119_208,
            "2b7ca858879f183ce2dcc65b300f48a0133d5430550f35b23d80d405a0788dbf",
            &["share/zoneinfo/"],
        ),
        p(
            "linux-64/libstdcxx-16.2.0-h934c35e_7.conda",
            6_615_436,
            "fa7018298629fc429971fffd6ecba3d27a7b02e4e890d6a93ea01ffebe8cb745",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libgcc-16.2.0-ha9f2e26_7.conda",
            1_058_444,
            "e031634c3a928f9594eba868bc46ef5230d6571a364a0c63215b06c4563bfcea",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libgomp-16.2.0-he0feb66_7.conda",
            640_566,
            "92a0bb350c7be6a4eedcb4e111c60aefd176018e6b1c009a6e025dbe9bbcd758",
            LINUX_LIBS,
        ),
        p(
            "linux-64/openssl-3.6.5-h781a0a9_0.conda",
            3_195_299,
            "9c1533f1cf784f0e250919c0c9762d0b0e6bc727a79290ffb7cea1ceca1d038c",
            LINUX_LIBS,
        ),
        p(
            "linux-64/icu-78.3-py310h44b86e0_2.conda",
            14_459_115,
            "9f07834f0c546ab14d885ce0366285f61f44e326c0edd1fc63b8294e113ae432",
            LINUX_LIBS,
        ),
        p(
            "linux-64/krb5-1.22.2-hbc21106_2.conda",
            1_394_333,
            "2a5c38c85e63df84c4e69ee71439841ce570d259ae3060627bb9a49a938d66f4",
            LINUX_LIBS,
        ),
        p(
            "linux-64/keyutils-1.6.3-h7cc23a3_1.conda",
            135_295,
            "dd053c96dcb0dcfd59422aefea9d2fe937a190167f34fba7893ec1e10a7e8963",
            LINUX_LIBS,
        ),
        p(
            "linux-64/openldap-2.7.2-hbc21106_0.conda",
            774_970,
            "afdcd6003d8c20c4a62cac31c1f32a7751cfb02e369c019a56b7701026d998f9",
            LINUX_LIBS,
        ),
        p(
            "linux-64/cyrus-sasl-2.1.28-hac629b4_1.conda",
            210_103,
            "7684da83306bb69686c0506fb09aa7074e1a55ade50c3a879e4e5df6eebb1009",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libntlm-1.8-hebe6cf0_3.conda",
            34_046,
            "e2abd4814b562672e7321e15bd0cf3450a18ab35d4e671eea169082125a278ab",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libxcrypt-4.4.38-h280c20c_0.conda",
            101_957,
            "f7e9292dd219a6435bbb1223da9586c3e70d66d169c5a92f08db3f2127df04e9",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libxml2-16-2.15.4-hf3af7cc_1.conda",
            569_062,
            "d1906463070c21f3215cd5de93164f31c44d5bac6ae9b5d8f5adf42b1e304039",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libxslt-1.1.45-h8e12856_1.conda",
            250_314,
            "3beb51b5ce8d2e0690b8517d3d955972d502cfbe2899279cbb9cbb6ff134d360",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libiconv-1.18-h0cb94f2_3.conda",
            789_471,
            "f943117edb9cd4d9c61cc972eee5a34291dc55ea7a6e9e38da104995841cbcb6",
            LINUX_LIBS,
        ),
        p(
            "linux-64/libzlib-1.3.2-h25fd6f3_3.conda",
            63_713,
            "eb8a0db0aa570124f7d2a93d7c7f596e3390df5e047818d873baad32985fc736",
            LINUX_LIBS,
        ),
        p(
            "linux-64/liblzma-5.8.3-hb03c661_1.conda",
            112_995,
            "9787df8c22a59c9a70d3e5a10db9ad663485e75e9ccc3f09bd092cb7b95e0dab",
            LINUX_LIBS,
        ),
        p(
            "linux-64/lz4-c-1.11.1-hee9eb32_0.conda",
            237_352,
            "e66dc54f3f3547f1988dc3ef606f2790d7fde24037ffd0a19b4f58e2895f4183",
            LINUX_LIBS,
        ),
        p(
            "linux-64/zstd-1.5.7-hb78ec9c_7.conda",
            601_301,
            "47d682b9f6d6ec9eb1a6e6c3e75ea6273e899e78fb7fc59f81d39745009fbc60",
            LINUX_LIBS,
        ),
        p(
            "linux-64/liburing-2.15-h4dbf13b_0.conda",
            166_831,
            "03191302c0c8bda5225647dce884fb4a3c88eca704884ff255c558d8d64c8bf8",
            LINUX_LIBS,
        ),
    ],
};

pub fn server(system: &str) -> Option<&'static Server> {
    match system {
        "windows-x64" => Some(&WINDOWS),
        "macos-arm64" => Some(&MAC),
        "linux-x64" => Some(&LINUX),
        _ => None,
    }
}

/// The folder the server's programs and files are kept in: `<root>/database/server`, beside its data
/// (`<root>/database/data`), where no installer and no update ever writes or deletes.
pub fn folder(root: &Path) -> PathBuf {
    root.join("database").join("server")
}

/// A program of the server (`postgres`, `initdb`, `pg_ctl`) in `folder`.
pub fn program(folder: &Path, server: &Server, name: &str) -> PathBuf {
    let mut path = folder.join(server.bin).join(name);
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

/// The note of what was taken, by the packages' SHA-256: written last, so a folder with it is whole.
const NOTE: &str = "pinned.txt";

fn note(server: &Server) -> String {
    let mut out = format!("PostgreSQL {} with pgvector {}, from conda-forge\n", server.postgres, server.pgvector);
    for p in server.packages {
        out.push_str(&format!("{} {}\n", p.sha256, p.file));
    }
    out
}

/// Whether `folder` holds the server as these pins have it, whole.
pub fn have(folder: &Path, server: &Server) -> bool {
    std::fs::read_to_string(folder.join(NOTE)).is_ok_and(|n| n == note(server))
        && ["postgres", "initdb", "pg_ctl"].iter().all(|name| program(folder, server, name).is_file())
}

/// Whether a path in a package is one `take` names.
/// Whether a path in a package is one `take` names: each a path's beginning, or with `*` standing for any
/// characters (`lib/*.so*`: under `lib/`, with `.so` in the name after it).
fn taken(path: &str, take: &[&str]) -> bool {
    take.iter().any(|t| {
        let parts: Vec<&str> = t.split('*').collect();
        if parts.len() == 1 {
            return path.starts_with(t);
        }
        let (first, last) = (parts[0], parts[parts.len() - 1]);
        if !path.starts_with(first) || path.len() < first.len() + last.len() || !path.ends_with(last) {
            return false;
        }
        let mut rest = &path[first.len()..path.len() - last.len()];
        parts[1..parts.len() - 1].iter().all(|middle| match rest.find(middle) {
            Some(i) => {
                rest = &rest[i + middle.len()..];
                true
            }
            None => false,
        })
    })
}

/// A path of a package as a path under the folder: its own parts only, never a way out of the folder.
fn inside(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    p.components().all(|c| matches!(c, Component::Normal(_))).then(|| p.to_path_buf())
}

/// A tar archive packed with zstd, out of the package (a zip).
fn packed_tar<'a>(src: &'a zip::OnDisk, entry: &zip::Entry) -> Fetched<tar::Archive<impl Read + 'a>> {
    let unpacked = ruzstd::decoding::StreamingDecoder::new(zip::open(src, entry)?)
        .map_err(|e| Failure::new(format!("the package's files can't be unpacked: {e}")))?;
    Ok(tar::Archive::new(unpacked))
}

/// What the package says of its files that name the folder it was built in: path -> (that folder as written, whether
/// the file is a program or library rather than text).
fn placeholders(src: &zip::OnDisk, listed: &[zip::Entry]) -> Fetched<HashMap<String, (String, bool)>> {
    let mut out = HashMap::new();
    let Some(info) = listed.iter().find(|e| e.name.starts_with("info-") && e.name.ends_with(".tar.zst")) else {
        return Ok(out);
    };
    for entry in packed_tar(src, info)?.entries()? {
        let mut entry = entry?;
        if entry.path()?.to_string_lossy().replace('\\', "/") != "info/paths.json" {
            continue;
        }
        let mut text = String::new();
        entry.by_ref().take(16 << 20).read_to_string(&mut text)?;
        let paths: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| Failure::new(format!("the package's list of files: {e}")))?;
        for p in paths["paths"].as_array().into_iter().flatten() {
            if let (Some(path), Some(placeholder)) = (p["_path"].as_str(), p["prefix_placeholder"].as_str()) {
                out.insert(path.to_string(), (placeholder.to_string(), p["file_mode"] == "binary"));
            }
        }
    }
    Ok(out)
}

/// `placeholder` (the folder a package was built in) in `bytes` made `folder`, as conda does it: in a program or a
/// library each string of characters that holds it gets the folder in its place and is filled up with zero bytes
/// to its old length, so that nothing in the file moves; in text it is simply replaced. How many places.
pub fn put_folder(bytes: &mut Vec<u8>, placeholder: &[u8], folder: &[u8], binary: bool) -> Fetched<usize> {
    let find = |hay: &[u8], from: usize| {
        hay.get(from..).and_then(|h| h.windows(placeholder.len()).position(|w| w == placeholder)).map(|i| i + from)
    };
    if placeholder.is_empty() {
        return Ok(0);
    }
    if !binary {
        let (mut out, mut at, mut n) = (Vec::with_capacity(bytes.len()), 0, 0);
        while let Some(i) = find(bytes, at) {
            out.extend_from_slice(&bytes[at..i]);
            out.extend_from_slice(folder);
            at = i + placeholder.len();
            n += 1;
        }
        out.extend_from_slice(&bytes[at..]);
        *bytes = out;
        return Ok(n);
    }
    if folder.len() > placeholder.len() {
        return Err(Failure::new("the server's folder has a longer path than its programs can take"));
    }
    let (mut at, mut n) = (0, 0);
    while let Some(start) = find(bytes, at) {
        let end = bytes[start..].iter().position(|&b| b == 0).map_or(bytes.len(), |p| start + p);
        let mut string = bytes[start..end].to_vec();
        let count = put_folder(&mut string, placeholder, folder, false)?;
        string.resize(end - start, 0);
        bytes[start..end].copy_from_slice(&string);
        n += count;
        at = end;
    }
    Ok(n)
}

/// The files `take` names out of a conda package (a zip with `pkg-*.tar.zst`), into `folder`. How many.
pub fn unpack(package: &Path, take: &[&str], folder: &Path) -> Fetched<usize> {
    let src = zip::OnDisk::open(package)?;
    let listed = zip::entries(&src)?;
    // (Windows' packages are taken as they are)
    let named = if cfg!(windows) { HashMap::new() } else { placeholders(&src, &listed)? };
    let here = folder.to_string_lossy().into_owned();
    let inner = listed
        .iter()
        .find(|e| e.name.starts_with("pkg-") && e.name.ends_with(".tar.zst"))
        .ok_or_else(|| Failure::new("this isn't a conda package (no pkg-*.tar.zst in it)"))?;
    let mut tar = packed_tar(&src, inner)?;
    let mut n = 0;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().replace('\\', "/");
        if !taken(&name, take) || entry.header().entry_type().is_dir() {
            continue;
        }
        let Some(relative) = inside(&name) else {
            return Err(Failure::new(format!("the package names a path outside its folder: {name}")));
        };
        let dest = folder.join(&relative);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if entry.header().entry_type().is_symlink() {
            // (a Mac's libraries are named by a link to the file with the full version; Windows packages have none)
            #[cfg(unix)]
            {
                let target = entry.link_name()?.ok_or_else(|| Failure::new(format!("{name}: a link to nothing")))?;
                if target.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
                    return Err(Failure::new(format!("{name}: a link out of its folder")));
                }
                let _ = std::fs::remove_file(&dest);
                std::os::unix::fs::symlink(target, &dest)?;
                n += 1;
            }
            continue;
        }
        if !entry.header().entry_type().is_file() {
            continue;
        }
        match named.get(&name) {
            Some((placeholder, binary)) => {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                put_folder(&mut bytes, placeholder.as_bytes(), here.as_bytes(), *binary)?;
                std::fs::write(&dest, &bytes)?;
                // a program changed is signed again, ad hoc as it came: a Mac runs none whose signature doesn't fit
                #[cfg(target_os = "macos")]
                if *binary {
                    let signed = std::process::Command::new("/usr/bin/codesign")
                        .args(["--force", "--sign", "-"])
                        .arg(&dest)
                        .output()?;
                    if !signed.status.success() {
                        return Err(Failure::new(format!(
                            "{name} couldn't be signed again: {}",
                            String::from_utf8_lossy(&signed.stderr).trim()
                        )));
                    }
                }
            }
            None => {
                let mut out = std::fs::File::create(&dest)?;
                std::io::copy(&mut entry, &mut out)?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = entry.header().mode().unwrap_or(0o644) & 0o755;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode))?;
        }
        n += 1;
    }
    if n == 0 {
        return Err(Failure::new(format!("{}: nothing to take out of it", package.display())));
    }
    Ok(n)
}

/// Where the packages are fetched from, with the connection for it: conda-forge over HTTPS, or a stand-in on
/// this computer for a check.
fn channel() -> (String, Net) {
    match std::env::var("FACE_TEST_CONDA") {
        Ok(v) if v.starts_with("http://127.0.0.1:") => (format!("{}/", v.trim_end_matches('/')), Net::plain()),
        _ => (CONDA_FORGE.to_string(), Net::new()),
    }
}

/// The whole download, in bytes.
pub fn size(server: &Server) -> u64 {
    server.packages.iter().map(|p| p.size).sum()
}

/// Fetch the server into `folder`, unless it is there. `progress(bytes so far, of)` as it goes. False: nothing to
/// do. Each package is kept only until its files are out; the folder is made whole or left without its note.
pub fn get(server: &Server, folder: &Path, progress: &mut dyn FnMut(u64, u64)) -> Fetched<bool> {
    if have(folder, server) {
        return Ok(false);
    }
    // what an earlier try left is never mixed with this one
    let _ = std::fs::remove_dir_all(folder);
    std::fs::create_dir_all(folder)?;
    let (total, mut before) = (size(server), 0u64);
    let (channel, net) = channel();
    for package in server.packages {
        let file = folder.join(".package.conda");
        let got = (|| -> Fetched<usize> {
            if !is_file(&file, package.size, package.sha256) {
                let from = net.open(&format!("{channel}{}", package.file), None, None)?;
                save(from, &file, package.size, package.sha256, &mut |got| progress(before + got, total))?;
            }
            unpack(&file, package.take, folder)
        })();
        let _ = std::fs::remove_file(&file);
        got.map_err(|e| Failure::new(format!("{}: {}", package.file, e.why)))?;
        before += package.size;
    }
    for name in ["postgres", "initdb", "pg_ctl"] {
        if !program(folder, server, name).is_file() {
            return Err(Failure::new(format!("the packages gave no {name}")));
        }
    }
    std::fs::write(folder.join(NOTE), note(server))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder a package was built in becomes the folder its files are in, as conda puts it: in a program, in the
    /// same place with zero bytes after it (nothing moves); in text, simply.
    #[test]
    fn the_folder_a_package_was_built_in_becomes_this_one() {
        let ph = b"/opt/placehold_placehold_placehold";
        let mut prog = [&b"\x7fELF.."[..], ph, b"/share/zoneinfo\0next\0", ph, b"\0end"].concat();
        let len = prog.len();
        assert_eq!(put_folder(&mut prog, ph, b"/Users/u/db", true).unwrap(), 2);
        assert_eq!(prog.len(), len);
        let first =
            prog.windows(b"/Users/u/db/share/zoneinfo".len()).position(|w| w == b"/Users/u/db/share/zoneinfo").unwrap();
        assert_eq!(first, 6);
        // the rest of that string is zero bytes, and what came after it is where it was
        let zeros = ph.len() - b"/Users/u/db".len();
        let after = 6 + b"/Users/u/db/share/zoneinfo".len();
        assert!(prog[after..after + zeros].iter().all(|b| *b == 0));
        assert_eq!(&prog[after + zeros..after + zeros + 6], b"\0next\0");
        assert!(prog.ends_with(b"\0end"));
        assert!(put_folder(&mut prog.clone(), ph, &[b'x'; 64], true).is_err());
        let mut text = [&b"prefix="[..], ph, b"\nlib=", ph, b"/lib\n"].concat();
        assert_eq!(put_folder(&mut text, ph, b"/a", false).unwrap(), 2);
        assert_eq!(text, b"prefix=/a\nlib=/a/lib\n");
    }

    #[test]
    fn only_what_is_named_is_taken() {
        let win = WIN_DLLS;
        assert!(taken("Library/bin/libssl-3-x64.dll", win));
        assert!(taken("Library/bin/plugins/preauth/spake64.dll", win));
        assert!(!taken("Library/bin/openssl.exe", win));
        assert!(!taken("Library/include/openssl/ssl.h", win));
        let pgvector = &["Library/lib/vector.dll", "Library/share/extension/vector"];
        assert!(taken("Library/share/extension/vector--0.8.6.sql", pgvector));
        assert!(taken("Library/share/extension/vector.control", pgvector));
        assert!(!taken("Library/include/server/extension/vector/vector.h", pgvector));
        // a Linux library under its versioned names, and nothing else of the folder
        for so in ["lib/libssl.so", "lib/libssl.so.3", "lib/libstdc++.so.6.0.34"] {
            assert!(taken(so, LINUX_LIBS), "{so}");
        }
        for not in ["lib/libssl.a", "lib/pkgconfig/openssl.pc", "share/lib/x.so", "lib/cmake/x"] {
            assert!(!taken(not, LINUX_LIBS), "{not}");
        }
        assert!(inside("Library/bin/x.dll").is_some());
        for out in ["../x", "/etc/passwd", "a/../../b"] {
            assert!(inside(out).is_none(), "{out}");
        }
        // (a drive is a way out on Windows only: elsewhere "C:" is a folder's name)
        assert_eq!(inside("C:/x").is_none(), cfg!(windows));
    }

    /// Every system's list has the server, pgvector, and nothing of what only psql needs; each pin is whole.
    #[test]
    fn each_server_is_postgres_with_pgvector_from_conda_forge_alone() {
        for (system, s) in [("windows-x64", &WINDOWS), ("macos-arm64", &MAC), ("linux-x64", &LINUX)] {
            assert!(std::ptr::eq(server(system).unwrap(), s));
            let names: Vec<&str> = s.packages.iter().map(|p| p.file).collect();
            assert!(names.iter().any(|f| f.contains("/postgresql-") && f.contains(s.postgres)), "{system}");
            assert!(names.iter().any(|f| f.contains("/pgvector-") && f.contains(s.pgvector)), "{system}");
            for gpl in ["readline", "ncurses", "libedit"] {
                assert!(!names.iter().any(|f| f.contains(&format!("/{gpl}-"))), "{system}: {gpl}");
            }
            for p in s.packages {
                assert!(p.file.ends_with(".conda") && !p.take.is_empty());
                assert!(p.sha256.len() == 64 && p.sha256.bytes().all(|b| b.is_ascii_hexdigit()) && p.size > 0);
            }
        }
        assert!(CONDA_FORGE.starts_with("https://conda.anaconda.org/conda-forge/"));
        // not the Raspberry Pi
        assert!(server("linux-arm64").is_none());
    }

    /// A package as conda-forge packs one: the named files come out where they belong, nothing else does.
    #[test]
    fn a_package_s_named_files_come_out_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("face-conda-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // the files, as a tar packed with zstd, in a zip with the package's info beside it (its list of files says
        // which one names the folder the package was built in)
        let packed_tar = |files: &[(&str, &str)]| {
            let mut tar = tar::Builder::new(Vec::new());
            for (name, body) in files {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_mode(0o755);
                h.set_cksum();
                tar.append_data(&mut h, name, body.as_bytes()).unwrap();
            }
            ruzstd::encoding::compress_to_vec(
                &tar.into_inner().unwrap()[..],
                ruzstd::encoding::CompressionLevel::Fastest,
            )
        };
        let packed = packed_tar(&[
            ("Library/bin/postgres.exe", "MZ server"),
            ("Library/bin/psql.exe", "MZ client"),
            ("Library/share/postgres.bki", "bki /built/here/share"),
        ]);
        let info = packed_tar(&[(
            "info/paths.json",
            r#"{"paths": [{"_path": "Library/share/postgres.bki", "prefix_placeholder": "/built/here", "file_mode": "text"}]}"#,
        )]);
        let conda = dir.join("x.conda");
        std::fs::write(
            &conda,
            crate::zip::tests::archive(&[
                ("metadata.json", b"{}", false),
                ("info-x.tar.zst", &info, false),
                ("pkg-x.tar.zst", &packed, false),
            ]),
        )
        .unwrap();
        let out = dir.join("out");
        let n = unpack(&conda, &["Library/bin/postgres.exe", "Library/share/"], &out).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read_to_string(out.join("Library/bin/postgres.exe")).unwrap(), "MZ server");
        // the folder it was built in becomes this one (Windows' packages are taken as they are)
        let bki = std::fs::read_to_string(out.join("Library/share/postgres.bki")).unwrap();
        if cfg!(windows) {
            assert_eq!(bki, "bki /built/here/share");
        } else {
            assert_eq!(bki, format!("bki {}/share", out.display()));
        }
        assert!(!out.join("Library/bin/psql.exe").exists());
        // a zip without the package's files is no package
        std::fs::write(&conda, crate::zip::tests::archive(&[("other.txt", b"x", true)])).unwrap();
        assert!(unpack(&conda, &["Library/"], &out).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
