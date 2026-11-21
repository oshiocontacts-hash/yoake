# FR-F4M-FIX3: script pounce damage

## Cause

The animation and goal timing were not the failure. `attacks::deliver_frame` queues the server `hit` event (`crates/horizon-client/src/mobs/custom/ai/attacks.rs:99`), and `script::pre_tick` delivers it before goal selection (`crates/horizon-client/src/mobs/custom/script.rs:695`). Thus the goal stopping ten ticks later cannot discard the callback.

The fox callback reaches `mobs::attack` with `i.target == None` for a player target (`crates/yoake-wasm-sdk/examples/fox_ai/src/lib.rs:63`). The host's `mob_info` conversion hardcoded `target_player: None`, despite the internal view recording that the target was the player. Consequently `mob_attack` rejected the request with `name a target mob or a player` (`crates/horizon-client/src/wasm_mods/mob_services.rs:205`). The guest intentionally ignores the returned result, so no command, `MobEvent::AttackPlayer`, or strike diagnostic was produced.

The downstream path was already functional: `script_apply::apply_in` turns an attack command into player damage and knockback events (`crates/horizon-client/src/mobs/custom/script_apply.rs:80`), and the game applies `AttackPlayer` through `mob_hits_player` (`crates/horizon-client/src/game/creatures.rs:2066`).

## Fix

- `MobView` now carries the target player's UUID when its world view is available (`crates/horizon-client/src/mobs/custom/script.rs:112`, `:163`).
- `mob_info` returns that UUID instead of always returning `None` (`crates/horizon-client/src/wasm_mods/mob_services.rs:107`). No ABI or generated file changed.
- `mob_attack` logs exactly one `Horizon custom-mob strike: mob N script attack ...` line for every accepted or refused request. Refusals include the host error; acceptances include target, damage, and knockback (`crates/horizon-client/src/wasm_mods/server_services.rs:268`). The existing `hit the player` line remains the landing diagnostic.

The guest was correct once given the documented player UUID, so it is unchanged. These changes only affect custom-mob script views and services; vanilla mob behavior is untouched.

## Files

- `crates/horizon-client/src/mobs/custom/script.rs`: publish the target player UUID.
- `crates/horizon-client/src/wasm_mods/mob_services.rs`: return the UUID and add the regression test.
- `crates/horizon-client/src/wasm_mods/server_services.rs`: accepted/refused attack diagnostics.
- `.lane/TASK.md`: findings and verification commands.
- `docs/yoake2/reports/FR-F4M-FIX3.md`: this report.

## Tests

`a_hit_frame_from_the_mock_guest_hurts_its_target_player` uses a mock `Runtime` and the real `GameServerHost` services. Its animation callback reads `mob_info`, passes both target fields to `mob_attack`, and drives `script::pre_tick` after `deliver_frame("hit")`. The test requires the nil UUID, an `AttackPlayer(3.0)` event, player knockback, and an empty command queue afterward. It uses only the synthetic bench world and needs no game resources (`crates/horizon-client/src/wasm_mods/mob_services.rs:427`).

No commands were run in this no-shell task. Formatting, focused and regression tests, and the standard quick self-check are listed under **Commands for Claude** in `.lane/TASK.md` and remain unverified here.

## Real game still needed

Run the real `fox_ai` WASM harness after the unit tests. Verify one server `hit` frame produces a `script attack accepted` line followed by `hit the player ... hurt true`, that player health decreases, and that there is no per-tick attack logging. Actual guest timing, health synchronization, and the end-to-end harness result cannot be established by the mock test.

## Results on LENOVO (Claude)

horizon-client-lib --lib 1919 passed; horizon-client --lib 1252 passed; horizon-client --lib --features yoake-hook-bench `wasm_mods::mob_services` 1 passed (the new test). The fixture in `tools/custom_mob_render_e2e.py` now sets `cooldown_persists: true` (one strike per run; the engine default stays vanilla). Not run in the real game.
