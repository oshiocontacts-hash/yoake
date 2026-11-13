# Yoake on macOS: where to start

Yoake is this fork of Horizon. Upstream Horizon already builds and ships on macOS (Apple Silicon):
see `packaging/macos/README.md` (the launcher as `Horizon.app` in a `.dmg`) and
`.github/workflows/release.yml` (the game). What is Windows-only today is what Yoake added.
Work on a branch (e.g. `mac-support`) and open a pull request; keep each step small.

## 1. Build and run the game and the launcher as they are

- `cargo build -p horizon-client` and the launcher (`--manifest-path launcher/Cargo.toml`), on the Mac.
- Prepare resources from your own Minecraft as upstream describes (never commit `.local/`).
- Note what breaks. Upstream's macOS code paths (`launcher/src/game_app.rs`, `packaging/macos/`) are the
  reference; Yoake's own crates are the likely gaps.

## 2. DLL mods on macOS (`.dylib`)

- `crates/horizon-client/src/dll_mods.rs` loads mods with `libloading` (works on macOS), but its path
  checks accept only `.dll` (around the two `"DLL path must be absolute with .dll extension"` checks).
  Accept the platform's library extension (`.dylib` on macOS, `.dll` on Windows) and keep everything
  else identical: identity, engine fingerprint, ABI and signature checks must not loosen.
- `crates/horizon-mods/src/yak/manifest.rs`: the `.yak` target enum has only `windows-x64`. Add
  `macos-arm64` (and its `library_filename`: `lib<name>.dylib`), with tests. Update `docs/yak-format.md`
  in the workspace when the format grows (tell the owner).
- `cargo-horizon-mod pack --bin macos-arm64=<path>` and `yak verify` must accept it.
- The game picks `bin/<target>/` for the platform it runs on (`crates/horizon-client/src/yak_mods.rs`).
- The mod kit (github.com/oshiocontacts-hash/yoake-modkit) then gets `targets = ["windows-x64", "macos-arm64"]`
  examples; the owner regenerates it.

## 3. The Yoake launcher on macOS

- The Yoake build sets `YOAKE_*` environment variables at build time (feed URL, portal URL, public keys,
  Microsoft client id). Ask the owner for the values to use; never commit them.
- The Yoake brand (`launcher/src/brand.rs`) and the mods page must work; packaging can reuse
  `packaging/macos/` with Yoake names and icons.

## 4. Distribution (owner's decision)

macOS blocks unsigned apps for other players. Giving builds to others needs an Apple Developer ID
(paid) and notarization, like upstream's `packaging/macos/notarize.sh`. Testing on your own Mac works
without it (ad-hoc signing).

## Checks before a pull request

- Windows must keep working: the owner runs the Windows tests and a game start before merging.
- With no mods installed the game behaves exactly like vanilla.
- No Mojang files, no keys, no `YOAKE_*` values in the diff.
