# FR-F5-SDK: Modder Kit, WIT/ABI Bump, SDK Refresh & Examples (F5 Code Scope)

Branch: `yoake-free/f5-sdk` (branched from `b7969698fa` = `yoake-free/merge`, F1–F4 complete).  
Scope: Code side of Stage F5 (`REM-19`, SDK wrappers, `yoake` CLI templates/build/check, and example manifests/demos). Guides (`REM-20`) and production-team ZIP packaging stay for the dedicated guides task.

## 1. Audit of GAP3 Stage F5 (at base `b7969698fa`)

| GAP3 Stage F5 Item | Status at Base `b7969698fa` | Outcome in This Task |
|---|---|---|
| **Item 1 — WIT & manifest version bump** (`yoake-mod.wit`, `yoake-full.wit`, `yoake-test.wit` at `1.0.0`; `SUPPORTED_WASM_INTERFACE` `(1, 2)`; `yoake-guest-sdk` ABI default `1.0` vs engine `1.1`) | **Open** | **Done** (`yoake:mods@1.7.0`, `wit-mod-1.0` compat bridge, `SUPPORTED_WASM_INTERFACE = (1, 7)`, `yoake-guest-sdk` `ABI_MINOR = 1`, `1.0` component + archive tests) |
| **Item 3 — Kit (`yoake-cli`) updates** (`custom-mob` placeholder; missing `client-full`/`server-full`/`worker`/`full-access` templates; missing `worker`/`full-access` check & build; bundled SDK missing `wit-worker` and `yoake-guest-sdk`) | **Partial** (REM-18 had added basic `worker` file-size/freshness check & `--features worker` copy in `main.rs`; rest open) | **Done** (all 9 templates updated/added; `yoake build`/`check` handle `worker-mod`, `full-access`, `inject-mod` shared/split paths; bundled SDK includes `wit-worker` & standalone `yoake-guest-sdk`; `yoake dev` hot-reload for full worlds remains out of scope/explicitly rejected by `dev`) |
| **Item 4 — Examples** (`client-services-demo`, `f2-io`, `fox_ai`, `fox_texture`, `full-services`, `gpu-demo`, `hello-std`, `mixin-config`, `seat_fox` lacked `yak.toml`; missing L2 sprite, MEDIA1 browser, F1+F2 guest-sdk examples; full kit sweep not run) | **Open** | **Done** (9 missing `yak.toml` manifests added, 3 new example packages created, 9/9 templates + 30/30 packaged examples pass `yoake build` + `yoake check`) |

---

## 2. What Changed

### 2.1 WIT & ABI Bump (REM-19)

- **WIT package bump (`1.0.0` -> `1.7.0`)**:
  - Updated [`crates/yoake-wasm/wit/yoake-mod.wit`](../../crates/yoake-wasm/wit/yoake-mod.wit), [`crates/yoake-wasm/wit/yoake-test.wit`](../../crates/yoake-wasm/wit/yoake-test.wit), and [`crates/yoake-wasm/wit/yoake-full.wit`](../../crates/yoake-wasm/wit/yoake-full.wit) package declarations to `package yoake:mods@1.7.0;`, alongside generator emitters in [`crates/horizon-mod-api-gen/src/abi/wit.rs`](../../crates/horizon-mod-api-gen/src/abi/wit.rs) and [`crates/horizon-mod-api-gen/src/abi/wit_full.rs`](../../crates/horizon-mod-api-gen/src/abi/wit_full.rs).
  - Kept [`crates/horizon-mod-api-gen/src/abi/baseline.rs`](../../crates/horizon-mod-api-gen/src/abi/baseline.rs) additive compatibility check normalizing `CURRENT_PACKAGE` (`package yoake:mods@1.7.0;`) to `1.0.0` for comparison against `abi-1.0-baseline.wit`.
  - Updated `WORKER_FULL_INTERFACE = "yoake:mods/worker@1.7.0"` in [`crates/yoake-mod-io/src/lib.rs`](../../crates/yoake-mod-io/src/lib.rs) and [`crates/yoake-wasm/src/full_bindings.rs`](../../crates/yoake-wasm/src/full_bindings.rs), and linked `"yoake:mods/worker@1.7.0"`, `"yoake:mods/worker@1.6.0"`, and `"yoake:mods/worker@1.0.0"` in [`crates/yoake-wasm/src/lib.rs`](../../crates/yoake-wasm/src/lib.rs).
- **Backwards compatibility for frozen `1.0.0` `client-mod` `.yak` packages**:
  - Preserved exact `1.0.0` `client-mod` WIT in [`crates/yoake-wasm/wit-mod-1.0/yoake-mod.wit`](../../crates/yoake-wasm/wit-mod-1.0/yoake-mod.wit).
  - Added `bindings_1_0` (sharing `yoake:mods/types@1.0.0` with `bindings`) in [`crates/yoake-wasm/src/lib.rs`](../../crates/yoake-wasm/src/lib.rs) and trait delegation impls in [`crates/yoake-wasm/src/host_1_0.rs`](../../crates/yoake-wasm/src/host_1_0.rs), dispatched transparently via `ClientExports::{Current, Legacy10}` in [`crates/yoake-wasm/src/client.rs`](../../crates/yoake-wasm/src/client.rs).
  - Updated `LEGACY_INTERFACE_VERSION = "1.0.0"` and `INTERFACE_VERSION = "1.7.0"` in [`crates/yoake-wasm/src/lib.rs`](../../crates/yoake-wasm/src/lib.rs).
  - Extended integration test `legacy_1_0_client_mod_component_loads_and_runs` in [`crates/yoake-wasm/tests/client_mod.rs`](../../crates/yoake-wasm/tests/client_mod.rs) to instantiate and tick a real component compiled against `wit-mod-1.0/yoake-mod.wit`.
- **Manifest interface range**:
  - Bumped `SUPPORTED_WASM_INTERFACE` in [`crates/horizon-mods/src/yak/manifest.rs`](../../crates/horizon-mods/src/yak/manifest.rs) to `(1, 7)`, preserving `1.0..=1.7` on hook worlds (`1.1`/`1.2` reserved for `inject-mod`) and proving both `1.0` and `1.7` in `wasm_interface_versions_are_checked` and `archive::wasm_module_packs_and_round_trips`.
- **Engine-call guest ABI `1.1`**:
  - Bumped `ABI_MINOR: u16 = 1` in [`crates/yoake-guest-sdk/src/lib.rs`](../../crates/yoake-guest-sdk/src/lib.rs) to match `ABI_VERSION = 1.1` in `crates/yoake-slice/src/runtime.rs` and updated [`docs/yoake2/ENGINE_CALL_ABI.md`](../ENGINE_CALL_ABI.md).
  - Updated `guest_abi_version_is_1_1` and `rustc_and_wasm_ld_retain_the_sdk_abi_section` (plus `AbiCheck::OkOlderMinor` verification for `1.0`) in [`crates/yoake-guest-sdk/tests/guest_runtime.rs`](../../crates/yoake-guest-sdk/tests/guest_runtime.rs).

### 2.2 SDK Refresh (`yoake-wasm-sdk` and `yoake-guest-sdk`)

- **`crates/yoake-wasm-sdk`**:
  - Added [`crates/yoake-wasm-sdk/src/worker.rs`](../../crates/yoake-wasm-sdk/src/worker.rs) behind feature `worker` in [`crates/yoake-wasm-sdk/Cargo.toml`](../../crates/yoake-wasm-sdk/Cargo.toml): `Worker` trait, `export_worker!`, `send`, `recv`, and `try_recv` targeting `yoake:mods/worker-mod@1.6.0`.
  - Added `hud::{batch_len, batch_scale_since, batch_clip_since, batch_tint_since}` wrappers in [`crates/yoake-wasm-sdk/src/lib.rs`](../../crates/yoake-wasm-sdk/src/lib.rs).
  - Expanded [`crates/yoake-wasm-sdk/src/full.rs`](../../crates/yoake-wasm-sdk/src/full.rs):
    - `full::client::worker::{send, recv}`
    - `full::client::sprites::{set_item, reset_item, item_pixels, set_block, reset_block, block_pixels}`
    - `full::client::media::{Browser, jpeg_to_texture}` (+ re-exported `media_types::*`) with RAII close-on-drop and `open`, `set_visible`, `navigate`, `history`, `back`, `forward`, `reload`, `input`, `state`, `frame_to_texture`, `frame_jpeg`, `audio_pcm`, `eval`, `eval_result`, `post`, `messages`
    - `full::client::ext`: `register_key_binding`, `key_binding_state`, `hud_place`, `gui_texture_write`, `draw_gui_item`, `draw_world_line`, `draw_world_quad`, `draw_world_textured_quad`, `draw_world_billboard`, `draw_post_tint`, `draw_post_vignette`, `post_shader_register`, `post_shader_unregister`, `post_shader_set_uniforms`, `post_shader_enable`, `post_shader_state`, `map_columns`, `config_get`, `config_set`, `storage_get`, `storage_put`, `storage_delete`, `storage_blob_begin`, `storage_blob_write`, `storage_blob_commit`, `storage_blob_abort`, `storage_blob_read`, and RAII `AudioStream` (`open`, `push_f32`, `push_i16`, `queued`, `dropped`, `move_to`)
    - `full::server::mobs::vec3_value`
    - `full::server::ext`: `player_remove_slot`, `config_get`, `config_set`, `has_permission`, `storage_get`, `storage_put`, `storage_delete`, `storage_blob_begin`, `storage_blob_write`, `storage_blob_commit`, `storage_blob_abort`, `storage_blob_read`, `world_border`, `set_world_border`, `play_sound_at`, `set_player_velocity`, `set_player_attack_hitbox`, `clear_player_attack_hitbox`
- **`crates/yoake-guest-sdk`**:
  - Added [`crates/yoake-guest-sdk/src/f_services.rs`](../../crates/yoake-guest-sdk/src/f_services.rs) wrapping REM-12 F-route client host services (`CallError`, `hud_draw`, `hud_gui_size`, `hud_gui_scale`, `ime_event`, `ime_set_allowed`, `ime_set_cursor_area`, `play_sound`, `play_sound_at`, and RAII `AudioStream`).

### 2.3 Kit (`crates/yoake-cli`) Updates

- **Bundled SDK ([`crates/yoake-cli/build.rs`](../../crates/yoake-cli/build.rs))**:
  - Bundles `crates/yoake-wasm-sdk`, `crates/yoake-wasm/wit`, `crates/yoake-wasm/wit-worker`, and `crates/yoake-guest-sdk` (`src/` + rewritten standalone `Cargo.toml` with workspace inheritance resolved and `[workspace]` appended so out-of-tree `full-access` builds succeed).
- **Templates ([`crates/yoake-cli/src/scaffold.rs`](../../crates/yoake-cli/src/scaffold.rs))**:
  - Replaced obsolete `custom-mob` placeholder with real F4M `content/` layout (`horizon-mod.toml`, `entity_type/starter_mob.json` with `ScriptGoal`, geo/animation/controller JSON, 64×32 PNG) + `server-full` `service.mobs` implementation.
  - Added `client-full`, `server-full`, `worker`, and `full-access` templates (`TEMPLATES` now 9 total, `interface = "1.7"`).
- **Build & Check ([`crates/yoake-cli/src/main.rs`](../../crates/yoake-cli/src/main.rs), [`crates/yoake-mod-io/src/worker.rs`](../../crates/yoake-mod-io/src/worker.rs), [`crates/yoake-slice/src/runtime.rs`](../../crates/yoake-slice/src/runtime.rs))**:
  - Added `WorkerHandle::validate` to validate `worker-mod` components during `yoake check`.
  - Added `slice_runtime::validate_module` to validate `full-access` core modules (`yoake_abi`, imports, `yoake_hook` export) during `yoake check`.
  - Added `CORE_MODULE_RUSTFLAGS` for `yoake build` on `world = "full-access"`, split-dir builds when `Cargo.toml` lives in `client/`/`server/`/`worker/` subfolders, and destination deduplication so single-binary `inject-mod` packages sharing `client` and `server` paths (like `mixin-demo`) build cleanly without requiring split Cargo features.
  - Wired `"inject-mod"` validation via `Injection::instantiate_with_permissions` in `yoake check`.

### 2.4 Examples Added / Updated

- **Missing `yak.toml` manifests added**: `client-services-demo`, `f2-io` (also added optional `server` feature + `ServerMod` entry in `Cargo.toml`/`src/lib.rs`), `fox_ai`, `fox_texture`, `full-services`, `gpu-demo`, `hello-std`, `mixin-config`, `seat_fox`.
- **New examples added**:
  - [`crates/yoake-wasm-sdk/examples/sprite-demo`](../../crates/yoake-wasm-sdk/examples/sprite-demo): standalone L2 dynamic item/block sprite override (`client-full`, `sprites::*`).
  - [`crates/yoake-wasm-sdk/examples/browser-demo`](../../crates/yoake-wasm-sdk/examples/browser-demo): MEDIA1 embedded browser overlay into `GuiTexture` (`client-full`, `service.browser`).
  - [`crates/yoake-guest-sdk/examples/f1-f2-demo`](../../crates/yoake-guest-sdk/examples/f1-f2-demo): F1+F2 `yoake-guest-sdk` core server module (`worker::{send, recv}`, `hooks::SERVER_TICK`) paired with a `worker-mod` echo component.
- **Note on non-package fixtures**: `crates/yoake-wasm-sdk/examples/test-demo` is a `test-mod` fixture (`yoake test` / `yoake_test`), not a mod package, and intentionally has no `yak.toml`. `crates/yoake-guest-sdk/examples/` also holds 4 standalone single-file `.rs` fixtures (`engine_call_game_path.rs`, `f_client_services.rs`, `fox_ai_full.rs`, `tick_and_item_use.rs`) compiled directly with `rustc` by `guest_runtime`/`yoake2_examples` tests.
- **Verification harness added**: [`tools/verify_f5_kit.py`](../../tools/verify_f5_kit.py).

---

## 3. Kit Sweep Results (9 Templates + 30 Examples)

Run via `tools/verify_f5_kit.py` using built `yoake.exe` (`--cargo-target-dir C:\Users\USER\NestgateHorizon\horizon-free\target`, `CARGO_NET_OFFLINE=true`). Initial full sweep ran 9 templates + 30 examples (35/39 passed; 4 failed before fixes); after fixing `custom-mob`, `build.rs` standalone `yoake-guest-sdk/Cargo.toml`, `browser-demo`, and `main.rs` destination deduplication and rebuilding `yoake.exe`, `--filter custom-mob full-access browser-demo mixin-demo` passed 4/4.

### 3.1 Per-Template Results (`yoake new` -> `yoake build` -> `yoake check`)

| Template | `new` | `build` | `check` | Time (s) | Run |
|---|---|---|---|---:|---|
| `minigame` | PASS | PASS | PASS | 41.58 | Initial sweep |
| `hud-only` | PASS | PASS | PASS | 31.84 | Initial sweep |
| `custom-mob` | PASS | PASS | PASS | 43.70 | Re-run after `MobInfo::type_id` fix |
| `utility` | PASS | PASS | PASS | 6.61 | Initial sweep |
| `script` | PASS | PASS | PASS | 6.78 | Initial sweep |
| `client-full` | PASS | PASS | PASS | 36.75 | Initial sweep |
| `server-full` | PASS | PASS | PASS | 49.81 | Initial sweep |
| `worker` | PASS | PASS | PASS | 51.00 | Initial sweep |
| `full-access` | PASS | PASS | PASS | 10.67 | Re-run after `build.rs` standalone manifest fix |

### 3.2 Per-Example Results (`yoake build` -> `yoake check`)

| Example | `build` | `check` | Time (s) | Notes |
|---|---|---|---|---|
| `browser-demo` | PASS | PASS | 13.05 | New MEDIA1 demo (re-run after `key: String` & `HudTexture` fix) |
| `chest-count` | PASS | PASS | 29.25 | |
| `client-services-demo` | PASS | PASS | 14.26 | Added `yak.toml` |
| `config-demo` | PASS | PASS | 14.22 | |
| `content-demo` | PASS | PASS | 24.69 | |
| `f2-check` | PASS | PASS | 92.06 | Multi-module (`server` + `worker`) |
| `f2-io` | PASS | PASS | 99.98 | Added `yak.toml` + `server` feature |
| `fox_ai` | PASS | PASS | 21.41 | Added `yak.toml` |
| `fox_texture` | PASS | PASS | 15.28 | Added `yak.toml` |
| `full-demo` | PASS | PASS | 44.55 | Legacy `interface = "1.0"` |
| `full-services` | PASS | PASS | 34.91 | Added `yak.toml` |
| `gpu-demo` | PASS | PASS | 14.80 | Added `yak.toml` |
| `hello` | PASS | PASS | 1.17 | |
| `hello-std` | PASS | PASS | 7.52 | Added `yak.toml` |
| `hud-item` | PASS | PASS | 15.27 | |
| `ime` | PASS | PASS | 16.31 | |
| `item-trade` | PASS | PASS | 22.55 | |
| `level-generator` | PASS | PASS | 24.23 | |
| `mixin-config` | PASS | PASS | 7.56 | Added `yak.toml` (`1.2`) |
| `mixin-demo` | PASS | PASS | 11.12 | Re-run after `main.rs` shared `client == server` dedup |
| `mod-call-pair/caller` | PASS | PASS | 22.75 | |
| `mod-call-pair/responder` | PASS | PASS | 19.06 | |
| `permission-demo` | PASS | PASS | 21.00 | |
| `player-vitals` | PASS | PASS | 21.55 | |
| `ruby-behaviors` | PASS | PASS | 21.89 | |
| `seat_fox` | PASS | PASS | 19.17 | Added `yak.toml` |
| `server-storage` | PASS | PASS | 21.77 | |
| `sprite-demo` | PASS | PASS | 13.78 | New L2 sprite override demo |
| `tested-counter` | PASS | PASS | 6.67 | |
| `yoake-guest-sdk/f1-f2-demo` | PASS | PASS | 13.78 | New F1+F2 `yoake-guest-sdk` + `worker` demo |

---

## 4. Test Results (Observed After Final Edits)

- `.\hz-cargo.cmd test --offline -j 6 -p yoake -p horizon-mods -p yoake-wasm -p yoake-guest-sdk` -> **Exit 0**:
  - `horizon-mods` lib: 108 passed, 0 failed; `tests/yak.rs`: 43 passed, 0 failed (including `wasm_interface_versions_are_checked` and `wasm_module_packs_and_round_trips` for `1.0` and `1.7`).
  - `yoake` (`yoake-cli`) unit (`src/main.rs`): 36 passed, 0 failed (including `all_templates_create_valid_projects` + `Content::read` on `custom-mob` and `a_declared_worker_is_validated_beside_the_client`); `tests/cli_output.rs`: 3 passed, 0 failed.
  - `yoake-guest-sdk` unit (`src/lib.rs`): 5 passed, 0 failed (`guest_abi_version_is_1_1`); `tests/guest_runtime.rs`: 8 passed, 0 failed (1 skipped doc-test); `tests/std_io.rs`: 4 passed, 0 failed (with added `lock_runtime` + `OnceLock` eliminating parallel Windows `rustc` `std_fs` file races).
  - `yoake-wasm` unit (`src/lib.rs`): 73 passed, 0 failed (`worker_link_tests::the_worker_interface_links_under_both_names`); integration tests: `client_mod` 7/7 (including `legacy_1_0_client_mod_component_loads_and_runs`), `client_services` 2/2, `f2_io` 11/11, `full_mod` 5/5, `hello_mod` 1/1, `inject_mod` 9/9, `package_services` 7/7, `sandbox` 16/16, `std_mod` 2/2, `subscriptions` 3/3, `test_mod` 5/5.
- `.\hz-cargo.cmd test --offline -j 6 -p horizon-mod-api-gen -p yoake-mod-io` -> **Exit 0**:
  - `horizon-mod-api-gen`: 46 passed, 0 failed (`abi::baseline::abi_1_0_is_additive`, `abi::wit_full::tests::full_wit_matches_catalog_and_parses`, `abi::tests::guest_sdk_full_bindings_match_the_catalog`).
  - `yoake-mod-io`: 5 passed, 0 failed.
- `.\hz-cargo.cmd test --offline -j 6 -p yoake-slice --features wasm` -> **Exit 0**:
  - `yoake-slice`: 34 unit + 6 `abi` + 1 `modes` + 6 `multi` + 3 `nary` + 6 `policy` + 7 `readonly` + 21 `slice` + 1 compile-fail doc-test passed, 0 failed.
- `python -B tools\check_public_tree.py` -> **Exit 0** (`Public tree check passed.`).

*Disk note*: After all builds and test suites above, `cmd /c dir C:\` reported `7,931,613,184 bytes free` (~7.93 GB), so full `horizon-client` game builds were not kicked off on C: in this run.

---

## 5. Remaining for Guides & Production ZIP Task (REM-20)

- **JA + EN beginner guides parity (`docs/guide-en/`, `docs/guide-ja/`)**:
  - Add missing EN chapters [`docs/guide-en/`](../guide-en/) `09-sprites.md`, `10-ime.md`, `12-files-network.md` matching existing Japanese chapters in [`docs/guide-ja/`](../guide-ja/).
  - Add Custom Mobs (F4M) and Browser/Media (MEDIA1) chapters to both `docs/guide-en/` and `docs/guide-ja/`.
  - Update `docs/guide-{en,ja}/{README,02-first-mod,04-examples}.md` from the old 5-template / 19-example counts to the 9 templates and 30+ examples.
- **Full-access & reference docs (`docs/yoake2/`, `docs/`)**:
  - Bring [`docs/yoake2/MOD-GUIDE.ja.md`](../MOD-GUIDE.ja.md) up to parity with [`docs/yoake2/MOD-GUIDE.md`](../MOD-GUIDE.md) (IME, Levels) and document ABI `1.1` / F1 bridges / F4M `custom_mobs` / `yoake new full-access` in both.
  - Rewrite [`docs/mods.md`](../../mods.md) custom-mob section for F4M format and update [`docs/yoake-wasm-dev.md`](../../yoake-wasm-dev.md) for `1.7.0`, new templates, and current capability/trust rules.
- **Production-team ZIP & live-game e2e**:
  - Add packaging script / workflow step for the production-team ZIP (`yoake` CLI + docs + SDK templates/examples) and run full-client runtime verification once sufficient C: disk space (> 12 GB) is available.
