# FR-F4M-SEATS2: custom-vehicle click boarding, seat indices and locator checks

Branch `yoake-free/f4m-seats2`. Written by Kimi from source reading; reviewed, corrected and tested by Claude on LENOVO (horizon-client-lib --lib 1921 passed, horizon-client --lib 1252 passed). Not run in the real game.

## 1. Right-click boarding

### Causes

The control action is not losing the click. `use-entity` presses and releases through `agent_mouse` (`crates/horizon-client/src/game/agent.rs:1520`), while `play_mouse` queues a use on the press (`crates/horizon-client/src/game.rs:17206`). The existing `interact` and `use-item` actions use the same immediate press/release when `--hold` is zero (`agent.rs:1467`). Holding for a tick is not required.

The server reaches the custom interaction through `interact_entity` and `interact_mob` (`crates/horizon-client/src/game.rs:6644`, `crates/horizon-client/src/game/creatures.rs:5441`). A guest's `Interact` packet reaches the same code in the guest's swapped-in turn (`crates/horizon-client/src/game/lan_actions.rs:2285`). The cause is the fixture, not the engine: the seats harness patched the Cinder Fox into a vehicle but kept its
`tameable` block, and `riding::rideable` lets only the owner ride a tameable type (FR-F4M-4c: "a horse is not
ridden wild"). An untamed tameable vehicle takes no player at all, so `use-entity` was a plain click that did nothing.
FR-F4M-4d (seat claims for the host and guests, sections 7.1 and 7.5) says nothing about owners, so the 4c rule stands.

### Fix (Claude's review)

* `tools/custom_mob_seats_e2e.py` `vehicle_fixture`: the vehicle drops `tameable`, so both players may board (the
  checks 2, 8, 9 and 4/10 seat the host and the guest).
* Kimi's first draft removed the owner rule from `riding::rideable` and let the ride shortcut run before the owner's
  sit order in `interact.rs`. Both were reverted: they change the F4M-4c design. Open question for Yoake: should a
  *tamed* tameable vehicle whose type also `sits_on_command` board on the owner's click (as a horse does) instead of
  toggling sit? Today it sits; nothing was changed.

## 2. `seat_set` index

The API is zero-based: seat 1 means the second row. The catalog defines `(entity, rider, seat)` (`crates/horizon-mod-api-gen/abi-subset-server.json:213`) and the generated WIT has the same order (`crates/yoake-wasm/wit/yoake-full.wit:2192`). The SDK's friendlier `(entity, seat, rider)` wrapper maps back to `(entity, rider, seat)` (`crates/yoake-wasm-sdk/src/full.rs:444`). The host forwarding and validation preserve that order (`crates/horizon-client/src/wasm_mods/server_services.rs:312`, `wasm_mods/mob_services.rs:278`). The command event then calls the riding code with the same index.

The seat chooser does not discard an explicit index: `plan_board` checks and returns `Some(seat)`, settlement keeps a valid claim, and `mob_rider_attachment` reads that claim. Existing host tests also call the raw-order service directly.

No defect in the current checked-in path was established by source inspection, so no working code was changed merely to speculate about the observed seat 0. The new `seat_set_keeps_the_requested_nonzero_seat` test (`mobs/custom/ai/tests_4d.rs:405`) pins the exact important behavior: seat 1 remains seat 1 after settlement and uses its `[0.5, 0.8, 0.0]` attachment. The original failure must be rechecked with a freshly rebuilt `seat_fox` guest. If it still returns seat 0 after the unit/service tests pass, the remaining unknown is in the built guest-to-host boundary or runtime glue and needs new runtime evidence, not an argument-order guess.

## 3. Locator animation and baby NBT

`/horizon custommobs trigger` calls `anim_sync::request_trigger` and starts the server rig on the next tick (`game/commands/horizon_cmd.rs:39`, `mobs/custom/anim_sync.rs:428`). The trigger is also synchronized to clients. Server attachments intentionally use the locator's rest position; the client uses the latest animated matrices through `posed_locators` (`game/custom_mob_draw.rs:32`) and the posed riding helpers. The old harness wrongly required host/server movement as well as guest/client movement.

`tools/custom_mob_seats_e2e.py` now asserts animated displacement only on the guest, retains host movement as diagnostic detail, and samples each tick for the full 16-tick clip. The existing resource-free locator tests already pin rest and animated offsets, including scale.

The baby flag in the harness comes from `entities`, which returns `Mob::is_baby()` (`game/agent.rs:749`); that reads `extra.age < 0` (`mobs/living.rs:569`). Custom NBT loading previously skipped vanilla top-level `Age` because `saves_age(Species::Custom)` is false (`entity_nbt.rs:1624`), and custom persistence read only `yoake.age`. `custom::persist::read` now accepts top-level `Age` even when no `yoake` compound exists, while a saved `yoake.age` retains precedence on later loads (`mobs/custom/persist.rs:162`). `vanilla_age_nbt_makes_a_custom_mob_a_baby` covers the summon-shaped input and save/load.

The mouth fixture had no `breedable` block, whose default scale is 1, so half scale was not a valid expectation. The harness now gives that type valid `breedable` data with `baby_scale: 0.5`. This is a harness correction in addition to the engine's `Age` fix.

## Files changed

* `crates/horizon-client/src/game/player_seats.rs` (Claude: `Horizon custom-mob seat:` info lines for a script's `seat-set` and the AI's boarding)
* `crates/horizon-client/src/mobs/custom/ai/tests_4d.rs`
* `crates/horizon-client/src/mobs/custom/persist.rs`
* `tools/custom_mob_seats_e2e.py`
* `.lane/TASK.md` (findings and verification commands)

No generated file was edited.

## Verification still needed

Run the formatting, focused unit/service tests and quick PR checks listed for Claude. Then rebuild the game, ctl and `seat_fox` WASM and run the full seats e2e with `--seat-wasm`. In particular, confirm `use_entity`, `multi_player`, `reload`, `seat_set` and the guest's animated locator. The original seat-0 observation and guest zero-displacement observation are not claimed fixed without that run.

## Seat 0 instead of seat 1 (still open)

The engine path keeps seat 1 in-process (`seat_set_keeps_the_requested_nonzero_seat`). For the next run the game logs
`Horizon custom-mob seat:` lines: a script's `seat-set` (nothing to do / vetoed / the seat the mob now holds) and any
AI boarding (`AI boarding: mob R takes seat S of V`). The seats harness records them in `seat_set.detail.seat_lines`
and no longer aborts the run when the check fails.

## SEATS3

### Causes

The `horizon:mob_sync` client store was not losing the server's claim. It already applied `Seats` after a tracking snapshot,
accepted later claim changes, and ordered them by frame time. The missing step was in the game loop: the claims were copied
from `Remote.custom_mobs` into the client's `Mob` state only by `sync_riders`, the server-side path. A remote client tick went
straight to `position_riders`, so placement still derived a sole rider's seat from its order and chose seat 0. This also
explains why the server could log seat 1 while the host's client view showed seat 0.

The player-height failure was a bad expectation: the seat attachment is 0.8 above the vehicle and the player's default vehicle
attachment is 0.6, so the player's feet are correctly 0.2 above it. The later abort was harness state: the host had steered the
vehicle about 11 blocks away before the guest tried to click it.

### Source changes

* `crates/horizon-client/src/game/remote.rs`: copy remote seat claims before every remote client rider-placement pass.
* `crates/horizon-client/src/game/remote_custom_mob.rs`: factor the claim copy into a testable helper and add
  `client_uses_seat_claims_from_snapshot_and_changes`. It proves the old local derivation gives seat 0, a join snapshot plus
  `Seats` selects the seat-1 attachment `[0.5, 0.8, 0.0]`, and a later claim change selects seat 0.
* `tools/custom_mob_seats_e2e.py`: expect player height 0.2 (with tolerance); stop host steering; teleport the guest from the
  host's current vehicle position; wait until the guest's own view agrees; retry `use-entity` four times. Failures in
  `use_entity`, `multi_player`, and `reload` retain replies as `pass: false` details and allow subsequent checks to run.

No generated file or vanilla behavior was changed.

### Tests

Not run in this worktree by Kimi (shell execution was prohibited). Formatting, focused unit tests, Python syntax, quick PR
checks, rebuilt binaries/WASM, and the full seats e2e with `--seat-wasm` are listed under **Commands for Claude** in
`.lane/TASK.md`. Real-game confirmation is still required, especially `seat_set`, `multi_player`, `reload`, and `locator`.
