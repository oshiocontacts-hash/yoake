# Yoake code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

## What is signed

Only these files, built by the public GitHub Actions workflow
[`.github/workflows/yoake-windows.yml`](../.github/workflows/yoake-windows.yml)
from the source in this repository:

- `YoakeLauncher.exe` — the Yoake launcher (`launcher/`)
- `yoake.exe` — the Yoake game client (`crates/horizon-client`, no compiled-in mods)

Nothing built on a developer's own computer is signed with this certificate.
Mods are not signed with it: they are distributed as `.yak` packages
(data and WebAssembly), checked by the game against Yoake's own signed catalog.

## Team roles

| Role | Who |
|---|---|
| Authors / committers | Members with write access to this repository |
| Reviewers | [@oshiocontacts-hash](https://github.com/oshiocontacts-hash) |
| Approvers (every signing request) | [@oshiocontacts-hash](https://github.com/oshiocontacts-hash) |

Every member with write access uses two-factor authentication on GitHub.
Each release is approved by hand in SignPath before it is signed.

## Privacy

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or operating it.

What the user asks for, and where it goes:

- **Sign-in** with a Microsoft account (Microsoft / Xbox Live / Minecraft services), to play.
- **Updates**: the launcher reads Yoake's update feed at `portal.yunipop.com`
  (versions and signatures only) and downloads the files the user chooses to install.
- **Yoake portal** sign-in (invite code or Discord) when the user opens it.
- **Multiplayer**: servers, LAN games or friend worlds the user chooses to join.

Yoake builds do not send usage data or crash reports: the upstream Horizon
telemetry is turned off in the Yoake brand (`launcher/src/telemetry.rs`).

## Upstream

Yoake is a fork of [Horizon](https://github.com/horizon-rust/horizon) (MIT).
Yoake's changes are under the same licence ([LICENSE](../LICENSE)).
