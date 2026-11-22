# FR-F4M-FIX4: last three custom-mob e2e failures (2026-11-22, GALLERIA)

Branch `yoake-free/f4m-fix4` from `yoake-free/merge` c01031597. Implementation: agy (Gemini 4 Hydrogen); final build, unit tests and the four e2e runs: coordinator.

## Causes and changes
1. **render --wasm strike.** The strike did land on exactly one player; the check failed because it expected 2.0 damage. `fox_ai` (the wasm example) deals a fixed 3.0 (`mobs::attack`), the data-goal fixture sets `attack_damage = 2`. `tools/custom_mob_render_e2e.py` now expects 3.0 with `--wasm`, 2.0 without. (The earlier "hits the guest" note in FR-MERGE7 was a misreading: the check does not require a specific player.)
2. **seats multi_player / reload, "out of reach or hidden".** The test agent aimed `use-entity` at one low interior point; with a rider seated on top the ray hit the rider first. `game/agent.rs`: `use_entity_aim_points` tries the low interior point, then points on the near face toward the eye, then the centre (unit test `use_entity_aim_points_clear_a_rider_on_top`). The e2e's dismount now holds Shift across ticks (`key lshift tap` was released before a tick sent PlayerInput).
3. **locator: guest pounce delta 0.10, baby_flag false.** Custom mobs were not treated as ageable when building the baby flag, and their `minecraft:scale` was never synced (custom types are not in `attributes.json`'s `entities` table). `game/lan.rs`, `lan_attributes.rs` (`custom_scale_attr`), `remote_mob_data.rs`, `mobs/custom/persist.rs`; unit test `custom_mob_receives_server_baby_flag` and a scale-attribute test. The locator check now freezes ticks and steps one at a time so the 16-tick pounce peak is sampled on both clients.

## Results (shared target `horizon-free/target`)
- `cargo test -p horizon-client-lib --lib custom`: 255 passed.
- render (data goal): 9/9 checks pass. render --wasm: 8/8 pass. seats --seat-wasm: 9/9 pass (locator host/guest pounce delta 0.31/0.31, baby_flag true). seats without wasm: 8/8 pass.
- Logs: `logs/f4m-fix4-{data,wasm,seats-wasm,seats-nowasm}.log`.

## Open
- None for these checks. agy's run hit its print timeout while the e2e ran; the coordinator reran everything above on the final tree.
