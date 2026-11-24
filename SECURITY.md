# Security

## Reporting a problem

Please report a security problem privately, through GitHub's
[private vulnerability reporting](../../security/advisories/new) for this repository, not in a public issue.
Say what you found, how to reproduce it and which version it was. You will get an answer there.

Only the latest release is supported.

## What the app is built to keep

- **Local by default.** The app's servers listen on 127.0.0.1 only and answer only their own pages. The one
  service that listens on the local network (with a shared database set up) is the device-to-device service:
  TLS with a pinned certificate, signed requests, photos by content hash only.
- **Nothing sent by itself.** A face or photo leaves the computer only when you send it (Web search), when you
  point Describe photo at a server on another device, or to the shared database you set up yourself.
  [docs/SETTINGS.md](docs/SETTINGS.md) lists everything that reaches the network.
- **Keys stay protected.** Keys and passwords are kept protected on disk (DPAPI on Windows, owner-only files
  elsewhere), reach a page only to be shown masked in Settings, and exist in the shared database only encrypted
  on the device.
- **Downloads are pinned** by size and SHA-256, and the Windows and macOS builds are signed by CI.
- **Your data is yours.** Every person's data can be exported and forgotten; an installer never deletes models
  or data.

[AGENTS.md](AGENTS.md), "Security rules", has the rules in full.
