# FR-E2E-FIX3: stabilize `behaviors_e2e.py` and `custom_mob_seats_e2e.py`

Branch `yoake-free/e2e-fix3` from `yoake-free/merge`. Only the e2e scripts were modified; no engine code was changed.

## 1. `tools/behaviors_e2e.py`

### What was flaky and why

1. **`registered`**: `log_text(game)` was read immediately after `setblock ruby:lamp[lit=false]`. Because Horizon writes logs asynchronously on a background `Writer` thread (`crates/horizon-client-render/src/logging.rs`) and block updates travel from the integrated server to the client, `'ruby'` could be missing from the log (or `lit=false` not yet reflected on the client) when checked immediately.
2. **`interact` (`use` and `vanilla`)**:
   - The script filled only the `y - 1` stone floor without clearing `y .. y + 4` to air, so worldgen terrain/foliage could stand between the player and the target block (or at the stand spot).
   - `aim_at` issued `tp @s ... facing ...` without waiting for the teleport to reach the client, and `plain` called `interact` on `minecraft:lever` directly without clearing terrain, placing a support block, choosing a known `face`/`facing`, or aiming from a clear stand point. Because `minecraft:lever` has a thin, facing-dependent outline box, `aim_block` (`crates/horizon-client/src/game/agent.rs`) could miss it or report `"no face of block ... is visible from here"`.

### What changed

- Cleared `y .. y + 4` to `minecraft:air` above the stone platform in both `behaviors` and `plain`.
- Added `wait_registered(game, pos, seconds=30)` polling every 5 ticks until `lit_of(game, lamp) == 'false'` and `'ruby'` (or `'registering'`) appears in `log_text(game)`, and recorded `log_has_ruby` / `log_has_registering` in `registered.detail`.
- Added `parse_state_props`, `is_air_block`, and `aim_candidates` to inspect the target block's `face`/`facing` properties and surrounding air blocks, pick an exposed face and clear stand point, wait via `game.until` for `pos_of(game)` to reach the teleport destination in `aim_at`, and pass the chosen face to `game.raw('interact', *pos, face)` in `interact_when_ready` (retrying on transient visibility/reach errors).
- In `plain` (`vanilla`), placed a stone support block at `(px + 2, py, pz + 1)` and `minecraft:lever[face=wall,facing=north]` at `(px + 2, py, pz)`, and interacted through `interact_when_ready`.

## 2. `tools/custom_mob_seats_e2e.py` (`--seat-wasm`)

### What was flaky and why

1. **Fixed tick waits and `multi_player`**: `guest.until(..., ticks=60)` was too short under build/system load when syncing the host's moved fox to the guest client, and fixed `wait(...)` calls around `/ride`, `use-entity`, `dismount`, and `reopen_world` could observe state before updates settled.
2. **Suffocating riders / fox displacement into uncleared terrain**: In `use_entity`, `move forward 0.9` in `--offscreen` mode drove the fox up to ~11 blocks (past the `[-12, +12]` cleared arena into natural terrain), causing seated chickens (`m3`, `m4`) in `multi_player` to suffocate in blocks at the edge (`health` dropping to `0.0` before `reload`), leaving fewer riders and differing seat offsets on reload.
3. **`--seat-wasm` auto-boarding range & `save-all` command**: Chickens (`m3`, `m4`, `m0`) were summoned at fixed `(hx, hy, hz + 3.0)` without checking distance from the driven fox (`seat_fox` auto-seats chickens within 4.0 blocks onto seat 1), and `reload` sent `/save-all` as a chat command (which is dedicated-only and ignored on an integrated server) instead of `horizon-ctl save-all`.

### What changed

- Expanded the cleared stone/air arena from `±12` to `±20` blocks (`hy .. hy + 5`) and shortened `use_entity`'s forward drive from `0.9s` to `0.5s` so the vehicle stays comfortably inside the cleared area.
- Added `safe_chicken_pos` to spawn chickens `>5` blocks away from the fox's current position so `seat_fox` (`MAX_BOARD_DISTANCE = 4.0`) never hijacks `/ride` test chickens.
- Replaced fixed tick waits across `seat_set`, `ride3`, `guest`, `stay`, `vehicle`, `use_entity`, `multi_player`, `reload`, `clear_mobs`, and `locator` with condition-based `until` waits (`ticks=100..240`), including waiting for `'Guest'` to leave `lan status`'s `guests` list and retrying `/ride m4` briefly after guest disconnect.
- Switched `reload` to `host.raw('save-all')`, added `host.until` checks after `reopen_world` for both `post_chickens` and `post_host`, and reconnected `guest` on a `reload` failure so `locator` never runs with `guest` on the title screen.

## 3. `tools/levels_e2e.py`

Left untouched (already fixed by `ChunkBatchReceived` acknowledgement in FR-E2E-FIX2).

## Verification results

Built binaries and WASM guests:
- `.\hz-cargo.cmd build -j 6 -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl`
- `.\hz-cargo.cmd build --manifest-path crates\yoake-wasm-sdk\examples\ruby-behaviors\Cargo.toml --release --target wasm32-wasip2`
- `.\hz-cargo.cmd build --manifest-path crates\yoake-wasm-sdk\examples\seat_fox\Cargo.toml --release --target wasm32-wasip2`

| Script | Pass count | Checks |
| --- | --- | --- |
| `.\hz-py.cmd tools\behaviors_e2e.py --bin-dir target\debug --wasm target\wasm32-wasip2\release\yoake_wasm_ruby_behaviors.wasm` | **3/3** | `registered`, `use`, `wand`, `vanilla` all PASS in all 3 runs |
| `.\hz-py.cmd tools\custom_mob_seats_e2e.py --bin-dir target\debug --seat-wasm target\wasm32-wasip2\release\yoake_wasm_seat_fox.wasm --out target\seats-e2e` | **3/3** | `seat_set`, `ride3`, `guest`, `stay`, `vehicle`, `use_entity`, `multi_player`, `reload`, `locator` all PASS in all 3 runs |
