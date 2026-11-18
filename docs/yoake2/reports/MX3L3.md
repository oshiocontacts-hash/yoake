# MX3L3: protect the full-access control files; isolate server/friend packages

Branch `yoake2/slice` on top of 8d635e946 (MX3L2), working tree. No git writes, no installs, no game launch. Edits only inside `horizon-integ`. Feature `yoake2-slice`; default builds unchanged. Design notes: `docs/yoake2/ENGINE_CALL_ABI.md`, new section "Control files and remote packages (MX3L3)"; `YOAKE_MIXIN2.md` 2.5 has one more bullet.

## Progress checklist

- [x] 1a control files moved out of the mods folder into a launcher-owned folder outside the game root, passed by `--full-access-dir` / `YOAKE_FULL_ACCESS_DIR`; no path means nothing is full-access
- [x] 1b the folder (and its seal) is protected: `control::guard` refuses storage roots and file paths inside it; wired into `yoake-wasm` `DiskStorage`
- [x] 1c tamper detection: seal (SHA-256 recorded by the launcher side); a mismatch refuses the file. HMAC with a credential-store key documented as the next step
- [x] 2 `install_package` wired: shared code now lives in `horizon-mods` (`yak::control`), which the launcher already depends on; narrow entry points `Control::install_local` and `Control::approve`
- [x] 3 server/friend packages go to `mods/remote/<origin>/`, out of duplicate detection and out of the origin records; `install_package` never overwrites
- [x] 4 nits: `chain_for` early return; atomic writes of the control files
- [x] tests green (table below); docs; both report files

## 1. Control files outside the game root

- Moved the code: `AllowFile`, `OriginFile`, `Origin`, `install_package`, sealing and path protection now live in **`crates/horizon-mods/src/yak/control.rs`** (public as `horizon_mods::yak::control`), so the launcher (which depends on `horizon-mods` with `yak`) and the game share one implementation. `yoake2_mods.rs` re-exports them and keeps the policy and loading.
- **Where.** `<launcher data dir>/yoake-full-access/` (`control_dir_in(backend::data_dir())`, e.g. `%APPDATA%\horizon-launcher\yoake-full-access`); the seal is the sibling `yoake-full-access.seal`. The game takes the path from `--full-access-dir <path>` (or `=`), else `YOAKE_FULL_ACCESS_DIR`. `load_world_mods(mods_dir, control_dir: Option<&Path>)` now takes it; the world-start hook passes `Control::path_from_process()`.
- `Control::open` refuses a relative path and a folder inside the game root (or one that contains it), resolving `..` and symlinks. Without a usable folder the loader runs nothing as full-access and the report says why (`--full-access-dir`, "inside the game folder", seal mismatch).
- **Unreachable by bindings and storage.** `control::protect` registers the folder and seal; `control::guard(path)` resolves `.`/`..` and symlinks through the longest existing prefix (so a not-yet-created child or a `..` route from a missing folder still counts) and refuses. `DiskStorage::open_scoped_with_quota` (the root of every sandboxed mod's storage) calls it. Check of the engine side: the current generated bindings (`ids.lock.json`) are getters, setters and methods of entities, items and the `Game` root and take no paths, and the sandboxed storage takes keys, not paths, with a root chosen by the engine. The first binding that opens a file **must** call `guard` (documented in `ENGINE_CALL_ABI.md`).
- **Tamper detection.** Each save is atomic and then recorded in the seal (SHA-256 of both files). Refused (nothing full-access runs): a file that differs from its seal, an existing file without a seal, a sealed file that is missing. `Control::reseal` is the launcher's explicit repair. This is a hash, **not a MAC**: it catches edits by anything that does not also rewrite the seal. **Next step: HMAC keyed by a secret in the OS credential store** (the launcher's `account.rs` keychain), handed to the game per launch.

## 2. `install_package` wiring

The launcher is its own cargo workspace on GPUI and does not depend on `horizon-client`; I could not build or test it here, so I did not edit its UI code. Instead the narrow, documented entry points are in `horizon-mods`, which the launcher already uses:

- `Control::install_local(mods_dir, source)`: for "install a local .yak": copies the file into the mods folder (never over an existing one) and records origin `local`; returns the SHA-256.
- `Control::approve(bytes)`: the player's approval of exactly that file.
- `install_package(control, mods_dir, file_name, bytes, origin, signer)` for catalog downloads (`Origin::Catalog`) and deliveries.

**To wire in the launcher** (not done): in `yoake_mods_page.rs` `add_local_yak` / after the commit of `plan_local` (`launcher/src/yoake_mods/plan.rs`), when the manifest has `world = "full-access"`, call `install_local` for the instance's `mods` folder and, after the player confirms the warning, `approve`; and set `--full-access-dir <control_dir_in(data_dir())>` (or the env var) where the launcher builds the game command (`launcher/src/launch.rs`, next to `limit_memory`). Until then nothing sets the path, so the full-access loader runs nothing in a launcher-started game. A `.yak` dropped into the mods folder by hand has no record and stays not-full-access (tested).

## 3. Server and friend packages

- `install_package` with `Origin::Server` / `Friend` writes to `<mods>/remote/<origin>/`. The loader scans only the top of the mods folder, so these files take no part in duplicate-id detection, and the control files are not touched at all (tested by comparing the bytes of both files before and after).
- A server package with the same id (other bytes) or the same bytes as an approved local mod neither disables nor demotes it: the local mod loads, its record stays `local`.
- `install_package` creates the file with `create_new`: an existing file is never overwritten (local, catalog, and remote). An update removes the old file with `remove_package` first. If recording the origin fails the new file is removed again.

## 4. Nits

- `chain_for` returns `None` at once when no mod is loaded (before any lookup or allocation); the three dispatchers use `let Some(chain) = ... else`.
- Atomic writes (`control::write_atomic`: temp file next to the target, then rename; no temp file left) for both control files and the seal.

## Tests

| Suite | Result |
|---|---|
| horizon-mods `yak::control` unit tests (new, `--features yak`) | 5 passed |
| horizon-mods default features | 44 passed |
| yoake-slice (`--features wasm`): unit 25, `multi` 6, `policy` 6, `slice` 21, doctest 1 | all passed |
| yoake-wasm storage tests (incl. new protected-root test) | 25 passed |
| horizon-client `yoake2_loader` | **15 passed** (was 12; 3 rewritten, 3 new) |
| lib, feature on: `the_sandboxed_path_skips_full_access_packages` | passed |
| real `Game`, feature on, `HORIZON_TEST_RESOURCES=C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3` (2, ignored), now with the control files in a launcher-owned folder | 2 passed (44 s) |
| horizon-client-lib `yoake2_engine_call` 8, `yoake2_glue` 5, `yoake2_slice` 7 (+1 ignored) | all passed |
| yoake-guest-sdk: unit 3, `guest_runtime` 7 | all passed |
| feature-off `cargo test -p horizon-client --lib` | **1193 passed, 0 failed, 40 ignored** |

Not re-run: the yoake-bindgen `engine` gate (CI, 382 s): no binding changed.

What the new tests cover:
- control folder outside the root: unit test (inside the mods folder, the root, a `..` route, a folder containing the root, a relative path are all refused) and loader test `world_start_needs_a_sealed_launcher_owned_control_folder` (no path, inside-root path);
- reaching the folder: `guard` refuses the folder, its files, the seal, a missing child and a `..` traversal (unit + loader test); `DiskStorage::open` refuses a root that is the folder, a child, or a `..` route and creates nothing inside;
- tampered files: edited allow file, unsealed file, deleted file, deleted seal (unit); at world start an edited allow file and an edited origin file load nothing and unload the previous world's mod; a reseal accepts a deliberate repair;
- server package with the same id or bytes does not disable or demote a local approved mod (`server_and_friend_packages_live_apart_and_cannot_disturb_an_approved_mod`);
- overwrite refused (local update path, remote path), bad file names refused;
- launcher entry point, hand-dropped file stays unknown-origin; option/env path parsing; atomic write.

## Open

- **The launcher is not wired** (see section 2): it must pass `--full-access-dir` and call `install_local` / `approve`. Until it does, a launcher-started game runs no full-access mod.
- The seal is a hash; the HMAC with a credential-store secret is the next step.
- `guard` is only as good as its callers: today only `DiskStorage` calls it, because no other path-taking binding exists.
- Packages in `remote/<origin>/` are stored but nothing in this tree delivers or uses them yet.
