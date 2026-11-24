# Contributing

[AGENTS.md](AGENTS.md) is the full guide to the code: the layout, the rules each part keeps, the security rules
and the conventions. This page is the short version.

## The licence, and contributions

The project is Singularity Research's commercial product, published under the
[Singularity Research Commercial License](LICENSE): the source may be read, and building or changing it needs
a licence. So a contribution starts with an issue: Singularity Research gives a contributor the licence they
need to build and work on the code, and a contribution can only be taken together with its author's agreement
that it becomes Singularity Research's to license under these and any other terms.

## Before you commit

```
cargo fmt --check
cargo test --workspace --release
node --test web/tests/*.test.js crates/face-server/pages/tests/*.test.js
```

None of these needs a model, a camera or the network. CI runs them on Windows, a Mac, Linux and 64-bit ARM
Linux, and then builds each system's installer.

To run the programs you need the models and what the programs run with:

```
cargo run -p face-tools --release -- get --root target/root
```

Every download is pinned by size and SHA-256 and kept only when it is that file.

## Rules that matter most

- **Nothing personal in the repository.** No photos, names, paths, accounts, host names or addresses, in code,
  docs, tests or commit messages. The library (`data/`) is never in git.
- **Nothing leaves the computer by itself.** The servers listen on 127.0.0.1 only; what reaches the network is
  listed in [docs/SETTINGS.md](docs/SETTINGS.md) and a new setting or request goes there too.
- **Nothing may be assumed to be installed** on a user's computer: what the app needs is in the installer, or
  the app asks for it at its first start.
- **Downloads are pinned** by size and SHA-256.
- **Measured values stay measured.** Thresholds and tuning numbers were measured; change one only with a new
  measurement.
- **A test never asks a real outside service** and never touches a real library: test copies get their own
  folders (`FACE_ROOT`, `FACE_HOME`) and ports (`FACE_PORTS`), and show nothing (`FACE_UI=none` or `hidden`).
- **An installer never deletes** the models or the data.

## Checks beyond the tests

What needs a desktop, a camera's page, an installer run for real or the shared database is checked by scripts
that are not part of this repository. They are being made into Rust tests, which is where a new check goes.

## Security

See [SECURITY.md](SECURITY.md) for how to report a problem.
