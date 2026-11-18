# MX3L2: loader hardening before any distribution

Branch `yoake2/slice` on top of b6bd2bec3 (MX3L1), working tree. No git writes, no installs, no game launch. Edits only inside `horizon-integ`. Everything is behind `yoake2-slice` or in `yoake-slice`; feature-off builds are unchanged. Design notes: `docs/yoake2/ENGINE_CALL_ABI.md`, new section "Loader hardening (MX3L2)"; `YOAKE_MIXIN2.md` 2.5 got one bullet.

## Progress checklist

- [x] 1a approval bound to content: `full-access.toml` entries `[[allow]] id / version / sha256`; a changed package is not loaded (`ContentChanged`, "approve it again")
- [x] 1b origin as arrival metadata: `yak-origins.toml`, `OriginFile`, `install_package`; unknown/missing origin is never full-access; server/friend never full-access
- [x] 1c duplicate ids: both refuse (`DuplicateId`)
- [x] 2 sandboxed path skips full-access packages (+ preflight refuses), with a log line and a test
- [x] 3 fuel policy: `ModConfig::full_access()` (no fuel limit, 50 ms logging watchdog, finite init), documented
- [x] 4 nits: `chain_for` cache with invalidation; `load_world_mods` handles `unload` errors; depth cap documented as per mod
- [x] tests written and green (table below)
- [x] docs and both report files

## 1. Permission bound to content, origin as metadata

**Where packages are written today (grep result).** Nothing in `horizon-integ` writes a `.yak` into `<game root>/mods`: the launcher (outside this tree) extracts sandboxed packages into `yoake-mods/<id>/<version>/` (listed in `yoake-mods.txt`, read by `yak_mods.rs`), and nothing delivers full-access packages from a server or a friend at all yet. So the full-access folder is a hand-filled folder today, and MX3L1 treated every file in it as local. Now:

- **`yak-origins.toml`** (sidecar next to `full-access.toml`) records, keyed by the package's SHA-256: `origin` = `local` / `catalog` / `server` / `friend`, id, version, and the catalog `signer`. `yoake2_mods::install_package(dir, file_name, bytes, origin, signer)` is the one way to store a package with its origin; a future delivery path must call it (the doc says so). The player's own install uses `Local`, the catalog download `Catalog` (the loader still verifies the signature itself against the trust list), server and friend deliveries `Server` / `Friend`.
- Records are **sticky towards distrust**: bytes that arrived from a server or friend stay `server`/`friend` when the same bytes are installed locally later (delete the record to start over). Bytes recorded local that a server also sends become `server`.
- **No record, an unknown origin word, or a hash with no entry = unknown origin = never full-access** (`Refusal::UnknownOrigin`). A corrupt record file refuses everything (error reported), it never turns into trust. A `server` / `friend` package is refused even with its hash approved (checked before signature and approval).
- **`full-access.toml`** is now `[[allow]]` entries with `id`, `version`, `sha256` of the `.yak` file. Approval = id + exact bytes. An update or replaced file no longer matches: not loaded, and the report says "the package of 'x' changed since it was approved (approved ..., this file is ...); approve it again". Approving again replaces the old entry. MX3L1's `[allow] id = true` is still read, listed in `LoadReport::legacy_notes`, and approves nothing (message: "without a package hash (an old entry)"). `AllowFile::allow_package(bytes)` reads id and version from the manifest.
- **Duplicates**: two `.yak` files with the same manifest id (any file names, same bytes too) make every full-access one of them refuse with `DuplicateId` naming the other file; other mods still load.
- `load_dir` now takes the `OriginFile` instead of one origin for the whole folder; `load_package` takes `Option<Origin>`. `Origin::LocalFolder` is now `Origin::Local`. `LoadReport` gained `problems` and `legacy_notes`, and `messages()` gives the player-facing lines (a plain sandboxed package gets no line).
- **Honest limit**: the origin file is only as good as whoever writes it; today nothing but `install_package` (and the test helpers) does, and a mod cannot write it. Where full-access packages are delivered later, that code is the thing to review.

## 2. Sandboxed path skips full-access packages

`yak_mods::wasm_mods_from` skips any package whose manifest has `world = "full-access"` (both sides) with a `log::info!` line, and `wasm_preflight` returns an error if asked directly. Test `yak_mods::tests::the_sandboxed_path_skips_full_access_packages` (an ordinary server module next to it still loads). It is gated on `yoake2-slice` so the feature-off test count stays 1193; the skip code itself is not feature-gated.

## 3. Fuel policy

`ModConfig::full_access()` is what the loader uses (`load_world_mods`):

| | `ModConfig::new` (tests, unchanged) | `ModConfig::full_access()` |
|---|---|---|
| fuel per activation | 1,000,000 | none (full tank after every top-level activation) |
| trapping epoch | off, configurable | off |
| watchdog (`watchdog_ms`) | off, configurable | 50 ms of guest time, logs, never traps |
| `yoake_init` | finite (50 M fuel, 2000 epoch ticks) | the same, still finite |

The watchdog is an epoch deadline with a callback that logs one stderr line per slow activation, bumps `Stats::slow_activations` / `slowest_ms`, and continues with a fresh deadline. Time inside `original` is not counted (the existing pause logic now covers the watchdog). The ticker thread wakes every 5 ms for a watchdog-only mod (1 ms for a trapping `epoch_ticks`, which still wins and traps). A runaway guest can therefore still hang the server thread: that is the stated price of full access. Documented in `ENGINE_CALL_ABI.md`.

## 4. Nits

- **`chain_for` cache** per hook id (`Rc<Vec<..>>`), dropped by a generation counter that `load`, `unload`, `uninstall` and `Host::disable` (mod death) bump. Hot path is one hash lookup plus an `Rc` clone; an empty chain is cached too.
- **`load_world_mods` and unload errors**: errors are collected, the failing mods stay in the list, nothing new is loaded (two generations on one hook would be worse), and the report's `problems` says why.
- **Depth cap** (`MAX_DEPTH` 16) is documented as per mod (runtime doc and `ENGINE_CALL_ABI.md`).

## Tests

| Suite | Result |
|---|---|
| yoake-slice (`--features wasm`): unit 25, `multi` 6, **`policy` 6 (new)**, `slice` 21, doctest 1 | all passed |
| **horizon-client `yoake2_loader` 12** (was 7; rewritten for the new API, new cases below) | all passed |
| horizon-client lib, feature on: `the_sandboxed_path_skips_full_access_packages` (new) | passed |
| real `Game`, feature on, `HORIZON_TEST_RESOURCES=C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3` (2, ignored): two-mod test and the single-mod test | 2 passed (44 s) |
| horizon-client-lib `yoake2_engine_call` 8, `yoake2_glue` 5, `yoake2_slice` 7 (+1 ignored bench) | all passed |
| yoake-guest-sdk: unit 3, `guest_runtime` 7 | all passed |
| horizon-mods | 44 passed |
| feature-off `cargo test -p horizon-client --lib` | **1193 passed, 0 failed, 40 ignored** (unchanged) |

Not re-run: the yoake-bindgen `engine` gate (382 s, ignored, CI). This change adds no engine method or generated binding.

New and changed tests:
- `yoake2_loader.rs`: content change not loaded until re-approved (`ContentChanged`, message text, then loads after approving); duplicate id disables both with report lines while an unrelated mod loads; server and friend origin never full-access even with the hash allowed (direct and through the folder scan, plus the whole `decide` table); missing / unknown-word / corrupt origin; `install_package` records origin, sticky distrust, signer round trip, bad file names refused; old-style `[allow] id = true` approves nothing and the report says so; catalog, several-mods and sandboxed cases kept.
- `policy.rs` (WAT guests): full-access config values; a 20-million-iteration computation is killed by the tests' fuel limit but runs under `full_access()` twice with the tank refilled; a 200 ms spin is logged once and not trapped while a 150 ms engine call under a quick guest is not logged; the trapping `epoch_ticks` still traps; a spinning `yoake_init` still fails under `full_access()`; chain cache follows load, unload, death and uninstall.
- Real `Game` two-mod test now installs packages through `install_package` (origin local) and approves them by hash.

## Open

- Nothing delivers full-access packages yet and there is no launcher UI for approvals; `install_package` / `AllowFile::allow_package` are the API such code should use.
- The watchdog only logs; there is no kill switch for a runaway mod other than closing the game.
- Client-side loading is still not done (server thread only); `yoake2_world_start_hook` still has no test of its own.
