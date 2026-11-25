# FR-E2E-FIX4: stabilize `custom_mob_acceptance.py` `s5` (`tick_sync`) and `s6` (`with_proxy`)

Branch `yoake-free/e2e-fix4` (from `yoake-free/merge` `ee02ec8604`). Only `tools/custom_mob_acceptance.py` (commit `0644f5c079`) and this report were changed; no engine code was modified.

## 1. What was flaky and why

### `s5` (`tick_sync`)
- In `run_s5_multiplayer`, the script summoned `sample_mobs:cinder_fox`, waited 20 ticks, fired `horizon custommobs trigger @e[type=sample_mobs:cinder_fox] pounce` once, waited 30 host ticks, and compared `at[0]` and `bt[0]` against `st[0]` (`abs(at[0] - st[0]) <= 1` and `abs(bt[0] - st[0]) <= 1`).
- Remote clients increment `self.world.game_time` locally every client tick and snap to the server's `game_time` on `SetTime` (`crates/horizon-client/src/lan/hub.rs`, broadcast every 20 server ticks; handled in `crates/horizon-client/src/game/remote.rs`). Under heavy load, client clocks can transiently drift a few ticks ahead/behind the server between `SetTime` packets (previously observed as `server_ticks [4258]` vs `client_a_ticks [4261]`, `client_b_ticks [4261]`), causing a single trigger sample to fail `within_one_tick`.

### `s6` (`with_proxy`)
- `check_s6_vanilla_client` summoned the fox after `make_flat_arena(host)` without setting world spawn or verifying that the fox existed on the host before `lan open`, and `stock_client_probe` ran a fixed `want_entities_s=5.0` read loop in `play`.
- In `lan/entity.rs`, `Entities::tick` only pairs entities in chunks already sent to the viewer (`v.chunks`). In observed runs, ~25–38 existing entities (host player and underground mobs) arrived before the proxied `minecraft:fox` (`protocol_id: 55`), so a 5-second fixed window under load could exit after seeing other entities before the proxy fox was paired and received.
- `check_s6_vanilla_client` also still carried a stale `missing_engine_work` string claiming F4M-5a was not implemented (already implemented in `FR-F4M-5a.md`).

## 2. What changed in `tools/custom_mob_acceptance.py`

1. **`s5` (`run_s5_multiplayer`):**
   - Wait via `until` for both `guest_a` and `guest_b` to report `horizon:mob_sync` (`known >= 1`) after summoning the fox.
   - Sample `horizon custommobs trigger ... pounce` up to 6 times within a 60s timeout (waiting at least 20–24 ticks between triggers so the 0.8s animation finishes and a 20-tick `SetTime` sync settles), polling logs in 2-tick steps until server, Client A, and Client B all emit their event ticks.
   - Judge `within_one_tick` on whether any sample produces a server tick `s` with matching client ticks `a` and `b` in that same sample where `max(abs(a - s), abs(b - s)) <= 1`, recording `samples` and `aligned_sample` in `detail.tick_sync`.

2. **`s6` (`stock_client_probe` and `check_s6_vanilla_client`):**
   - Set world spawn to `(hx, hy, hz)` (`setworldspawn`), summon the fox, and wait via `host.until(lambda: render_e2e.fox_list(host), bool, ticks=100)` before opening LAN and connecting the stock probe.
   - Updated `stock_client_probe` with `want_entities_s=30.0` for `with_proxy` and `target_proto_id=fox_proto_id`, returning as soon as `AddEntity` with `protocol_id == 55` arrives (and replying to play `PING` `62` with `PONG` `45` if received).
   - Removed the stale `missing_engine_work` entry from `s6`'s `detail`.

## 3. Verification

Binaries used: `C:\Users\USER\NestgateHorizon\horizon-free\target\debug\{horizon-client,horizon-ctl}.exe` and `C:\Users\USER\NestgateHorizon\horizon-free\target\guests-e2e\wasm32-wasip2\release\{yoake_wasm_fox_ai,yoake_wasm_fox_texture}.wasm`.

### Repeated `--only s5,s6` runs (3/3 PASS)

| Run | Exit | `s5` `tick_sync` (`server` / `client_a` / `client_b`, `max_skew`) | `s6` `with_proxy` (`minecraft:fox` proto `55`) | `s6` `without_proxy` |
|---|---|---|---|---|
| 1/3 | 0 | PASS (`[6578]` / `[6578]` / `[6578]`, `max_skew: 0`) | PASS (`entity_id: 67`, `pos: [6.5, 69.0, -4.5]`) | PASS (refused in `configuration`) |
| 2/3 | 0 | PASS (`[4870]` / `[4870]` / `[4870]`, `max_skew: 0`) | PASS (`entity_id: 70`, `pos: [-0.11, 70.0, -7.6]`) | PASS (refused in `configuration`) |
| 3/3 | 0 | PASS (`[4838]` / `[4838]` / `[4838]`, `max_skew: 0`) | PASS (`entity_id: 84`, `pos: [-4.5, 70.0, -2.5]`) | PASS (refused in `configuration`) |

### Full run (all 10 scenarios)

Note: an initial full run timed out in `s9` at 600s while compiling the `--release` `horizon-client-lib` test binary in `horizon-free\target`; after finishing that build with `.\hz-cargo.cmd test --release --no-run -p horizon-client-lib --lib`, the full `custom_mob_acceptance.py` run without `--only` exited 0 (`10/10` PASS):

| Scenario | Result | Key observed details (`target/custom-mob-acceptance/results.json`) |
|---|---|---|
| `s1` | **PASS** | `render_e2e_no_wasm` 9/9 checks pass (`idle`, `glow`, `pounce` tick `2394`, `lod` 30/70, `name`, `brain`, `strike_mob`, `strike` tick `5183`, `death`); `spawn_egg_ok: true`, `wander_max_displacement: 11.625`, debug shapes/labels `(16, 23)` |
| `s2` | **PASS** | `cargo test -p horizon-mobmodel --test cinder_fox`: `13 passed, 0 failed` |
| `s3` | **PASS** | `render_e2e_wasm` 10/10 checks pass (incl. `runtime_texture`, `model_commands` cube/item=2, `strike` tick `4449`); `chicken_pounce_hit: true`, `tamed: true`, `sit`/`stand` verified, `follow_displacement: 8.728` |
| `s4` | **PASS** | `uuid_preserved: true`, `unmodded_session_fox_count: 0`, `owner`/`sitting: true`/`state: "sit"`/`warmth: 7` preserved after modded reload and unmodded roundtrip |
| `s5` | **PASS** | `join_fetch` exit `75` (`yak_sha_match: true`); `tick_sync` `[3903]`/`[3903]`/`[3903]` (`max_skew: 0`); `late_join_pounce` snapshot `known: 1`, `diff: 152029`; `client_b_warmth_synced_changed` ok |
| `s6` | **PASS** | `with_proxy` reached `play`, saw `protocol_id: 55` (`entity_id: 290` at `[6.07, 66.0, -7.96]`), `disconnected: null`; `without_proxy` refused in `configuration` (`Content registry mismatch...`) |
| `s7` | **PASS** | `geo_malformed_and_command` pass (`ear_box_changed_before_to_ear: 2428`, `broken_warned_once: true`, `command_ok: true`); `anim_reload_ok: true`, `glow_reload_ok: true` (`1439792` px) |
| `s8` | **PASS** | `render_e2e_debug` pass (`9403` px on / `0` off; `F3+M` chord `9388` px on / `0` off); `logged_shapes_and_labels` `[16, 23]` |
| `s9` | **PASS** | `custom_mobs_200` release bench: `spread` `0.199`/`0.915`/`1.117` ms med (update/draw/total), `near` `0.462`/`0.877`/`1.344` ms med — within `1.0`/`1.5`/`3.0` ms budgets |
| `s10` | **PASS** | `f4m_parity` (`goldens_a_b_c`: `3 passed, 0 failed`), `the_custom_species_row_is_neutral` (`1 passed, 0 failed`), `test_f` (`modded_unmodded_modded_ok: true`, `vanilla_open_save` 4 region files, `has_yoake_key: false`); `test_d` not verified (no pre-F4M baseline JSON in `benchmarks/perf/`) |
