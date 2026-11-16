# Yoake 1.0 Release Notes (Draft)

Yoake 1.0 is the release of our Rust game client and launcher compatible with Minecraft: Java Edition 26.3.

## 1. WebAssembly (Wasm) mods (.yak packages)

- Mods in Yoake are built primarily as Wasm components (`wasm32-wasip2`) compiled from Rust and packaged as `.yak` files. Java mod JAR files do not run.
- Wasm mods run inside an in-game sandbox bounded by memory limits (64 MiB), per-call and per-50 ms window execution-time and CPU (fuel) limits, and the service permissions declared in `yak.toml`.
- In addition to `client-mod` (which provides 19 core client hooks), Yoake supports `client-full` for the full client catalog and `server-full` for the integrated server and dedicated server.
- Running a Wasm mod in a normal player environment requires a valid signature (`SIGNATURE`) from Yoake's pinned signing key. The contact point and submission workflow for mod authors to request package signing or portal (catalog) listing and updates are (undecided).
- You can save your chosen base pack and mods as a configuration in the launcher and share or import it as a new instance using a YK1 share code.

## 2. SDK and the yoake CLI

- The Rust guest SDK (`yoake-wasm-sdk`) lets mods draw on the HUD, register custom commands, manage settings, read and write scoped per-key storage, call server services, and make mod-to-mod calls (`mod-call`). Both the Rust standard library (`std`, enabled by default) and `#![no_std]` are supported.
- Install the `yoake` CLI from the public repository with `cargo install --path crates/yoake-cli`. Prebuilt executables on GitHub Releases are (undecided).
- Scaffold new projects from templates with `yoake new`, then run `yoake build` (build and validate), `yoake check` (validate `yak.toml` and `.wasm` without rebuilding), and `yoake pack` (output `dist/<id>-<version>.yak`). Each command reports its result as a single line of JSON on standard output.
- When testing your own unsigned Wasm mods is enabled in developer settings, `client-mod` modules can be hot-reloaded while the game is running—polled once per second—using `yoake dev` and a development folder (or an installed unsigned `.yak`).

## 3. Test runner

- Test functions registered with `#[yoake_test]` and `export_tests!` compile into a separate test module component (`bin/wasm/<id>-tests.wasm`) that is never bundled into distributable `.yak` packages.
- Running `yoake test` (or `horizon-client --yoake-test`) launches the game offscreen without opening a window, creates a fresh superflat world for each test, and runs your tests inside the real game engine.
- After warmup, the world is frozen so `TestWorld` actions—placing or breaking blocks, spawning entities or mannequins, running commands, and calling `mod_call`—advance the server by the exact number of ticks requested with `run_ticks`. You can also test scenarios quickly without starting the game using `cargo test` and a local mock (`TestDriver`).
- Passing `--clients 2` (`--yoake-test-clients=2`) connects a second client over loopback so you can test multiplayer block changes and chat synchronization (testing client-only hooks such as HUD rendering and operating on dimensions other than the Overworld are not implemented).

## 4. Mixin

- Yoake Mixin (`interface = "1.1"`, WIT world `inject-mod`) lets Wasm components intercept **51** designated injection sites exposed in the engine catalog. Package signature, identity, fingerprint, ABI, and engine-version checks apply in full to mixins.
- Every site supports **HEAD** (inspect, replace arguments, or cancel before the native body runs), **RETURN** (inspect or replace the return value), and **overwrite** (supply a result without running the native body). Only one mod per side may register an overwrite on a given site, while HEAD and RETURN handlers stack in mod-ID order.
- Replacement numbers and block-state IDs are validated against each site's type width and allowed bounds; out-of-range values and invalid scoped-handle changes are rejected rather than clamped. Command and chat sites require declaring `service.commands` (plus `service.chat` for client-side chat), and rewritten commands are subject to the same safety limits as ordinary commands.
- A trap or invalid response does not crash the game: the host disables the failing handler for that site, side, and injection point and falls back to native execution.

## 5. GPU (planned)

- Additional GPU features are **planned** for a future update and are not yet implemented in Yoake 1.0 (details are undecided).
- Today, the game client runs on GPUs supporting Vulkan or DirectX 12 on Windows, Vulkan or OpenGL 3.3 on Linux, and macOS (Apple Silicon, or Intel from the next Nightly), with support for graphics presets and shader packs.

## 6. Friends and groups

- Open a single-player world to friends to generate an invite code that friends can enter in the game or launcher to join. Connections are encrypted using password-authenticated key exchange (SPAKE2) and ChaCha20-Poly1305, and the invite code expires as soon as you stop sharing or close the world.
- Automatic router port mapping (PCP / NAT-PMP / UPnP) and STUN public-address discovery are built in; when a direct connection is not possible, players can connect using port forwarding, a VPN such as Tailscale, or short codes through a self-hosted relay server (`horizon-relay`).
- The launcher's Friends page organizes social features across four tabs (Friends, Requests, Groups, and Blocked), supporting friend requests by friend code (`YKF-XXXX-XXXX`), group creation and group invite codes (`YKG-XXXX-XXXX`, with configurable use limits, expiration days, and revocation), changing member roles, and blocking.
- Launcher friends and groups require signing in to the Yoake portal and display "Friends are not available yet." until enabled on the portal. This social system is separate from world invite codes—while Friends is unavailable, playing via world invite codes, LAN, or online servers continues to work normally.

## 7. The new glass look of the launcher

- The launcher adopts a new glass-style design, unifying pages such as Home and Friends around shared tabbed panels, list rows, input fields, and popover menus.
- Choose between dark, light, and system appearance modes along with custom accent colors.
- Destructive or sensitive actions—such as removing or blocking a friend, deleting or leaving a group, kicking a member, or replacing your friend code—use a two-step confirmation button (with red danger styling and a cancel button) to prevent accidental clicks.

## 8. Japanese and English

- The launcher interface is available in two languages: Japanese (`ja`) and English (`en`).
- On first launch, the launcher starts in English if the system's preferred language is English, and in Japanese otherwise; you can switch languages at any time in Settings.
- Any string not present in the Japanese translation table (`launcher/lang/ja.json`) falls back to English, and any saved language setting other than `ja` or `en` is displayed in Japanese.

## Sources

※ This section is an internal note to be removed before publishing.

- **Intro (compatible version)**: `README.md`, `docs/store/store-listing-ja.md`.
- **1. WebAssembly (Wasm) mods (.yak packages)**: `docs/store/store-listing-ja.md`, `.lane/ref/yoake-wasm-dev.md` (19 hooks, 64 MiB, 50 ms window), `.lane/ref/guides/guide-publish/docs/guide-en/07-publishing.md`, `.lane/ref/guides/guide-en/docs/guide-en/02-first-mod.md`, `.lane/ref/guides/guide-en/docs/guide-en/03-play.md`.
- **2. SDK and the yoake CLI**: `.lane/ref/yoake-wasm-dev.md`, `.lane/ref/guides/guide-en/docs/guide-en/02-first-mod.md`, `.lane/ref/guides/guide-en/docs/guide-en/03-play.md`, `.lane/ref/guides/sdk-examples-2/docs/guide-ja/03-play.md`.
- **3. Test runner**: `.lane/ref/guides/guide-testing/docs/guide-ja/06-testing.md`, `.lane/ref/guides/guide-en/docs/guide-en/06-testing.md`.
- **4. Mixin**: `.lane/ref/guides/guide-mixin/docs/guide-ja/08-mixin.md`, `.lane/ref/guides/guide-mixin/docs/guide-en/08-mixin.md` (command gate: `docs/yoake-mixin.md` on `yoake/mixin-m1` and 877978546).
- **5. GPU (planned)**: `README.md`, `docs/store/store-listing-ja.md`.
- **6. Friends and groups**: `docs/play-with-friends.md`, `launcher/src/friends_view.rs`, `launcher/src/friends_page.rs` (role changes, invite uses and days), `launcher/src/social.rs`, `docs/store/store-listing-ja.md`.
- **7. The new glass look of the launcher**: `launcher/src/friends_view.rs`, `docs/store/store-listing-ja.md`.
- **8. Japanese and English**: `launcher/src/i18n.rs`, `docs/store/store-listing-ja.md`.
