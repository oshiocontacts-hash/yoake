# FR-MERGE7: F4M-fix (real fox_ai guest in the render e2e) and F4M-4d (seats)

Base `40bbb34be` (MERGE6; f4m-10 and MERGE5 already in). No push. All game runs are finished (see "Games").

## 1. f4m-fix @4e9b41cae and the real `fox_ai` guest

Merged without conflicts (fixture moved to `tools/fixtures/cinder_fox/{yak.toml,content/}`, strike/death/data-merge diagnostics). Cold build of `horizon-client`/`horizon-ctl` in a fresh `target/`, plus the guest:
`cd crates/yoake-wasm-sdk/examples/fox_ai && CARGO_TARGET_DIR=<repo>/target/fox-ai cargo build --release --target wasm32-wasip2` (`yoake_wasm_fox_ai.wasm`).
`tools/custom_mob_render_e2e.py --wasm <that file>` installs the package (`yak.toml` + patched `content/` + `bin/wasm/fox_ai.wasm`) as `yoake-mods/sample_mobs/0.1.0/` on host and guest with `YOAKE_WASM_UNSIGNED=1`;
the type keeps the real `script` goal beside `swim` and targets chickens and players. Without `--wasm` the old data `melee_attack` variant runs.

Results (last real-guest run): idle, glow, pounce (trigger: server frame once in tick 2725, host and guest events in tick 2725), LOD, death pass. **Name tag passes now**: `data merge entity @e[type=sample_mobs:cinder_fox,limit=1] {CustomName:"Foxy",CustomNameVisible:1b}`
is accepted and `docs/yoake2/reports/img/f4m6-nametag.png` shows "Foxy" above the fox. **The strike does not happen with the real guest** and the data goal still does not hurt:

### Real guest: why the script goal never starts (new `info` line in `wasm_mods/mob_runtime.rs`, printed when the gate's inputs change)

The WASM module is loaded (`wasm: mod sample_mobs 0.1.0 loaded (server-full)`) and the engine polls `mob_goal_can_use` for `pounce`; the fox **never has a target** (`target (None, false)`, `has target_position false`),
so `can_use` is always `false` (the guest needs a target 3 to 6 blocks away). Both players were in survival, the host 4.0 blocks from the fox, the guest 3.2, difficulty normal. Health before/after: host 20.0 -> 20.0, guest 20.0 -> 20.0 (no strike). The lines (all of them, `tools/custom_mob_render_e2e.py`, file `docs/yoake2/reports/logs/merge7-real-guest-lines.txt`):

```
[host] Horizon custom-mob script goal: mob 273 pounce can_use Some(false) (on_ground Some(false), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob script goal: mob 273 pounce can_use Some(false) (on_ground Some(true), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob death: mob 273 dies, death_time 1 (limit 20) tick 2996
[host] Horizon custom-mob death: mob 273 removed at death_time 20 (limit 20) tick 3014
[host] Horizon custom-mob script goal: mob 318 pounce can_use Some(false) (on_ground Some(false), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob script goal: mob 318 pounce can_use Some(false) (on_ground Some(true), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob death: mob 318 dies, death_time 1 (limit 20) tick 3719
[host] Horizon custom-mob death: mob 318 removed at death_time 20 (limit 20) tick 3737
[host] Horizon custom-mob script goal: mob 322 pounce can_use Some(false) (on_ground Some(false), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob script goal: mob 322 pounce can_use Some(false) (on_ground Some(true), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob death: mob 322 dies, death_time 1 (limit 20) tick 5717
[host] Horizon custom-mob death: mob 322 removed at death_time 20 (limit 20) tick 5735
[host] Horizon custom-mob script goal: mob 332 pounce can_use Some(false) (on_ground Some(false), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob script goal: mob 332 pounce can_use Some(false) (on_ground Some(true), target Some((None, false)), visible Some(false), has target_position Some(false)) target distance None
[host] Horizon custom-mob death: mob 332 dies, death_time 0 (limit 20) tick 5798
[host] Horizon custom-mob death: mob 332 removed at death_time 20 (limit 20) tick 5817
```

Open for F4M-10/4c: `nearest_target` with `types: [minecraft:chicken, minecraft:player]` does not set a target in this variant (goals `swim` + `script`), while the data-goal variant (below) does begin an attack on `Player`.

### Data-goal variant: strike and death lines (file `docs/yoake2/reports/logs/merge7-data-goal-lines.txt`)

```
[host] Horizon custom-mob death: mob 331 dies, death_time 1 (limit 20) tick 2781
[host] Horizon custom-mob death: mob 331 removed at death_time 20 (limit 20) tick 2799
[host] Horizon custom-mob death: mob 354 dies, death_time 1 (limit 20) tick 3312
[host] Horizon custom-mob death: mob 354 removed at death_time 20 (limit 20) tick 3330
[host] Horizon custom-mob death: mob 356 dies, death_time 1 (limit 20) tick 3978
[host] Horizon custom-mob death: mob 356 removed at death_time 20 (limit 20) tick 3996
[host] Horizon custom-mob strike: mob 361 begins on Player tick 4067
[host] Horizon custom-mob strike: mob 361 dropped, the goal stopped tick 4069
[host] Horizon custom-mob strike: mob 361 begins on Player tick 4107
[host] Horizon custom-mob strike: mob 361 dropped, the goal stopped tick 4111
[host] Horizon custom-mob death: mob 361 dies, death_time 1 (limit 20) tick 4158
[host] Horizon custom-mob death: mob 361 removed at death_time 20 (limit 20) tick 4176
[host] Horizon custom-mob death: mob 364 dies, death_time 0 (limit 20) tick 4243
[host] Horizon custom-mob death: mob 364 removed at death_time 20 (limit 20) tick 4262
```

The strike **begins on Player (tick 4067) and is dropped two ticks later ("the goal stopped")**, before the server `hit` frame arrives (tick 4075), so the blow never lands: no `lands` and no `hit ... hurt` line. Host 20 -> 20, guest 20 -> 20 (fox 1.0 block from the host). The goal is stopped between
`begins` and the frame: `ChaseStrike::can_continue` with a pending blow returns `target_info(..).alive`; something ends the goal (its `stop` logs the drop) within 2 ticks. The same happens again at tick 4107 (begin) / 4111 (dropped). A chicken target (`strike_mob`) is not hurt either.

### Death lines (both variants)

`Horizon custom-mob death: mob N dies, death_time 1 (limit 20) tick G` then `... removed at death_time 20 (limit 20) tick G+18`: the body stays 18 to 19 ticks and the limit is 20 (the fixture's `death` clip is 12 ticks, so the engine's 20 applies: the type's death clip length is not used here;
`death_time 0` appears when the fox was killed in the tick it spawned its first frame). With the real guest the removal came 20 ticks after the damage (death_time 20, limit 20) in every run.

## 2. f4m-4d @0168b930c (final: LAN/host seat reservations via player_claims, `native_mob_may_ride` for seat-set and AI boarding, locator seats)

Merged without conflicts. Goldens `f4m_parity -- --ignored`: A `golden_a_saves` pass, B `golden_b_network` pass, C `golden_c_render_types_and_identity` pass.
Suites: `horizon-client-lib --lib` 1913 passed (48 ignored); `horizon-client --lib` 1251 passed (52 ignored); `level_registry_tests --ignored` 11 passed; `horizon-command` 37; `horizon-world` 385 + integration; `horizon-mods --features yak` 97 + 43;
`horizon-mod-api-gen` 46 (WIT/glue regenerated: no content change); `yoake-wasm` all binaries; `yoake-slice` 32 + 1; `horizon-mobmodel` all (61 unit tests + goldens); hook crates and `yoake-bindgen` all pass; `cargo check --features yoake2-slice` ok; `regen.sh --check` ok.

In-game checks (FR-F4M-4d.md section 5), new `tools/custom_mob_seats_e2e.py` (real host + Horizon guest, offscreen, no focus; the Cinder Fox fixture patched into a vehicle with three seats: seat 0 `controls: player` + `yaw_lock` at the centre, seats 1 and 2 `controls: none` half a block to the sides, 0.8 up):

| Item | Result | Evidence |
|---|---|---|
| 1 `/ride <chicken> mount <vehicle>` three times | pass | the chickens sit at horizontal distance 0.0, 0.5, 0.5 and 0.8 up from the vehicle; the fourth `/ride` replies with the failure message (`...に失敗しました`) and the fourth chicken stays 3.0 away |
| 3 a rider killed | pass | the other two keep distances (0.0, 0.8) and (0.5, 0.8): no shuffling |
| 3 the vehicle killed | pass | the vehicle is removed; the 3 remaining chickens are left in the world |
| 5 LAN client (guest joins after the riders are aboard) | pass | the guest's client sees the riders at 0.0, 0.5, 0.5 / 0.8 up (the snapshot) |
| 2 right click, 4 save and reload, 6 `cooldown_persists`, 7 `seat-set` from a guest, 8 guest seats and claims on disconnect, 9 guest on a `none` seat, 10 save with a player on seat 1, 11 native mod cancelling the mount, 12 locator seat during the pounce | **not run** | horizon-ctl has no entity right click, the harness has no save/reload or a second joining/leaving player for the claims, no packaged guest with `seat-set`, no native mod, and no locator-seat fixture; the first four rows above are what the harness reaches |

## Games

All game runs are finished: `custom_mob_render_e2e.py` (real guest and data goal) and `custom_mob_seats_e2e.py`. Nothing runs now.

## Disk

45 GB free at the start (fresh `target/`), 34 GB at the lowest, never near 12 GB.
