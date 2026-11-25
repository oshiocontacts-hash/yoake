# FR-E2E-FIX2: REM-16b e2e coverage, `levels_e2e` stock flake, and missing lang keys

Branch `yoake-free/e2e-fix2` from `yoake-free/merge` (`47d227de1a`).

## 1. `tools/content_kinds_e2e.py` (REM-16b coverage + reload engine fixes)

Extended the `ruby` fixture mod and check suite in `tools/content_kinds_e2e.py`:

- **tab** (`pass`): opens chat (`/`), types `gamerule ruby:gem_r`, and triggers Tab completion via `horizon-ctl` (`lan_e2e.complete`), asserting completion to `/gamerule ruby:gem_rate`.
  - Scope note: single-player runs on the integrated server thread (`game/integrated.rs`), so this exercises the integrated server's `ClientboundCommandsPacket` tree (`lan/commands.rs`), not a separate `--dedicated` server process.
- **tag** (`pass`): adds `ruby:crush` (untagged) alongside `ruby:shatter`, puts `ruby:shatter` into `#ruby:shards`, and merges `#ruby:shards` into `#minecraft:bypasses_armor` (`replace: false`). Under `natural_health_regeneration false` and full diamond armor (`instant_health` top-up), `/damage @s 5 ruby:crush` dealt `1.25` damage while `/damage @s 5 ruby:shatter` dealt `5.0`.
- **flow** (`pass`): builds a 7x7 stone platform, sets `ruby:oil` (`viscosity: 4`, `tick_rate: 10`), waits 25 ticks, and verifies `ruby:flowing_oil[level=4]` at `x+1` and `minecraft:air` at `x+2`.
- **bucket** (`pass`): in survival with `minecraft:bucket`, aims (`look-at`) at the `ruby:oil` source and uses (`use-item`), verifying the block clears to `minecraft:air` and `ruby:oil_bucket` enters inventory; then selects `ruby:oil_bucket`, aims (`look-at-block ... up`) at another platform cell and uses it, verifying `ruby:oil[level=0]` is placed and `minecraft:bucket` is returned.
- **reload** (`pass`): adds `ruby:gem_limit` (`default: 4`), updates `gem_limit.json` to `default: 12`, adds `gem_bonus.json` (`default: 9`), sends `/reload` via chat (avoiding `Integrated::call`'s 10 s timeout when unoptimized debug rebuilds of `.horizon-cache/mod-content` take >10 s), waits until `ruby:gem_bonus` appears in Tab completion, and verifies `/gamerule ruby:gem_limit` reports `12`, `/gamerule ruby:gem_bonus` reports `9`, and the log contains neither `content mods not loaded` nor `need a game restart`.

### Engine bugs found and fixed during `reload` e2e

1. **Integrated server lost `vanilla_root` (`crates/horizon-client/src/game/integrated.rs`, `crates/horizon-client/src/game/mods.rs`):** `open_integrated` and `run_component_harness` cloned `options` (where `options.resources` already pointed to `.horizon-cache/mod-content`) into `serve` / `server_game` without setting `server.ui.vanilla_root`. On `/reload`, `Game::vanilla_root()` fell back to `self.options.resources`, so `mods::reload_content_root` layered `ruby` on top of an already-modded root, failed with `ruby: ruby:crop is already a block`, and reset mod game rules and fluids to empty. `serve`, `server_game`, and `run_component_harness` now propagate `self.vanilla_root()` into `server.ui.vanilla_root`, and `mods::content_root_in` / `mods::reload_content_root_in` accept an explicit cache directory for tests. Added `#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]` regression test `game::integrated::tests::server_game_keeps_vanilla_root_for_mod_reload`.
2. **Cached command dispatchers did not refresh on mod rule reload (`crates/horizon-client-core/src/gamerules.rs`, `crates/horizon-command/src/vanilla.rs`, `crates/horizon-command/src/tests.rs`):** `vanilla::{dispatcher, release_dispatcher, game_tree, game_dispatcher_with_features}` cached `OnceLock` command trees built at startup, so after `/reload` registered `ruby:gem_bonus`, client Tab completion (`ClientboundCommandsPacket`) saw `ruby:gem_bonus` while server command parsing still rejected `/gamerule ruby:gem_bonus` as an invalid argument. Added a monotonic `MOD_RULES_EPOCH` (`gamerules::mod_rules_epoch()`) bumped in `gamerules::register_mod_rules` and replaced the `OnceLock` caches in `horizon-command::vanilla` with a single epoch-keyed `RwLock<CachedTrees>` (`cached_tree`). Added non-ignored unit test `horizon_command::tests::mod_game_rule_reload_refreshes_cached_dispatcher`.

### `content_kinds_e2e.py` results (latest run, `--bin-dir target\debug`, exit 0)

| Check | Result | Detail |
| --- | --- | --- |
| `rule` | PASS | `ruby:gem_rate` set to `7`, queried `7`, `99` refused, `keep_inventory` ok |
| `tab` | PASS | `/gamerule ruby:gem_r` -> `/gamerule ruby:gem_rate` |
| `enchant` | PASS | `ruby:keen` level 2 on diamond sword |
| `damage` | PASS | `/damage @s 1 ruby:shatter` accepted |
| `tag` | PASS | `ruby:crush` dealt `1.25`, `ruby:shatter` dealt `5.0` |
| `state` | PASS | `ruby:crop[age=3]` placed, `age=9` refused |
| `fluid` | PASS | `ruby:oil[level=0]` placed, no registration refusal logged |
| `flow` | PASS | `ruby:flowing_oil[level=4]` at +1, `minecraft:air` at +2 |
| `bucket` | PASS | `minecraft:bucket` <-> `ruby:oil_bucket` pickup & place round-trip |
| `reload` | PASS | `gem_limit` `4` -> `12`, `gem_bonus` `9`, tab `/gamerule ruby:gem_bonus` |
| `vanilla` | PASS | Without mod, `gem_rate`/`keen`/`shatter`/`crop` refused, `keep_inventory` ok |

## 2. Flaky `stock` check in `tools/levels_e2e.py`

### Root cause

In `crates/horizon-client/src/lan/guest.rs`, new guests start with `unacknowledged: 0` and `max_unacknowledged: 1`. In `crates/horizon-client/src/lan/hub.rs`, `next_batch` refuses to send chunks whenever `g.unacknowledged >= g.max_unacknowledged`; sending any chunk batch increments `g.unacknowledged += 1`, and only `Serverbound::ChunkBatchReceived` decrements it and raises `max_unacknowledged` to `10`. Across `Respawn` (`change_level` in `lan/hub.rs`), `unacknowledged` is preserved.

`crates/horizon-client/examples/level_probe.rs` previously handled only keep-alives, `Login`/`Respawn` (`PlayerLoaded`), and `PlayerPosition` (to send the chat command), and never replied to `Clientbound::ChunkBatchFinished` or `PlayerPosition` teleports:

- When the Overworld sent 1 initial chunk batch before `level_probe`'s `execute in mypack:moon` ran, `g.unacknowledged` reached `1 == g.max_unacknowledged` and stayed `1` after `Respawn`, so `next_batch` returned `None` forever (`chunks_after_respawn: 0`, `chunk_biomes: []`).
- When the command ran before the Overworld sent a batch, `g.unacknowledged` was still `0`, so `mypack:moon` got one batch (9 chunks) and passed (~50% flake rate).

### Fix and results

Updated `crates/horizon-client/examples/level_probe.rs` to answer `play::Clientbound::ChunkBatchFinished` with `play::Serverbound::ChunkBatchReceived { chunks_per_tick: 64.0 }`, confirm exact teleports (`AcceptTeleportation` on `PlayerPosition` when `relatives == 0`), and wait up to 20 s (or 3 s after first chunk post-respawn) for chunks after `Respawn`.

Rebuilt `level_probe` and ran `.\hz-py.cmd tools\levels_e2e.py --bin-dir target\debug` three consecutive times: **3/3 runs passed**.

| Run | `setup` | `travel` | `custom_portal` | `stock` | `created` | `lifecycle` | Overall |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | PASS | PASS | PASS | PASS | PASS | PASS | PASS (exit 0) |
| 2 | PASS | PASS | PASS | PASS | PASS | PASS | PASS (exit 0) |
| 3 | PASS | PASS | PASS | PASS | PASS | PASS | PASS (exit 0) |

## 3. Missing translations (`horizon.custommobs.*`)

`horizon-client-core::lang::tests::every_horizon_key_the_code_names_is_in_en_us` flagged `horizon.custommobs.stats` (and `horizon.custommobs.trigger`, both referenced in `crates/horizon-client/src/game/commands/horizon_cmd.rs`). Added both keys to:

- `crates/horizon-client/assets/horizon/lang/en_us.json`
- `crates/horizon-client/assets/horizon/lang/ja_jp.json`

## 4. Unit tests run

- `.\hz-cargo.cmd test -j 6 -p horizon-client-core -p horizon-command`:
  - `horizon-client-core`: **62 passed, 0 failed, 0 ignored**
  - `horizon-command`: **39 passed, 0 failed, 0 ignored** (includes new `tests::mod_game_rule_reload_refreshes_cached_dispatcher`)
- `.\hz-cargo.cmd test -j 6 -p horizon-client --lib -- integrated::tests`:
  - **7 passed, 0 failed, 2 ignored** (includes `server_game_keeps_vanilla_root_for_mod_reload` marked `#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]`)
- `.\hz-cargo.cmd test -j 6 -p horizon-core`:
  - **122 passed, 1 failed, 3 ignored** — pre-existing failure `jmath::tests::matches_hotspot_on_aarch64` at `crates/horizon-core/src/jmath.rs:591` (`4001ce4e022387bc`: `13827033635053522175` vs `13827033635053522176`, 1 ULP on this Windows x86_64 host; untouched).

## 5. What is left

- A dedicated `--dedicated` server process was not launched separately for `/gamerule <mod>:<rule>` Tab completion (covered via the integrated server's `ClientboundCommandsPacket` tree; `--dedicated` share of `/gamerule` nodes in `lan/commands.rs` already has unit coverage in `lan::commands`).
- `horizon-client::game::integrated::tests::server_game_keeps_vanilla_root_for_mod_reload` is `#[ignore]` and was compiled (with zero errors/warnings) rather than run standalone with `-- --ignored` under `hz-cargo.cmd` (which does not set `HORIZON_TEST_RESOURCES`); the live `/reload` path was verified end-to-end by `content_kinds_e2e.py`.
- `horizon-core`'s pre-existing `jmath::tests::matches_hotspot_on_aarch64` 1-ULP failure on Windows x86_64 remains untouched.
