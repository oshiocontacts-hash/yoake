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

## F4M-FIX2 in-game

Merged `yoake-free/f4m-fix2` (a8901861d) into `yoake-free/merge` by fast-forward from c316ac509 (no merge commit). Debug binaries rebuilt (`horizon-client`, `horizon-ctl` from `horizon-client-lib`); `fox_ai` built `--release --target wasm32-wasip2` into `target/fox-ai`. Benchmark gate was clear before every build and game run; disk stayed above 27 GB.
Resources: `HORIZON_TEST_RESOURCES=<repo>/horizon/.local/horizon-client/26.3`, `PYTHONUTF8=1`. Screenshots went to `target/` (not the committed `img/`).

### Unit tests

`cargo test -p horizon-client-lib --lib custom`: 250 passed, 0 failed (raw: `logs/f4m-fix2-cargo-test-custom.log`).

### Data-goal variant (`tools/custom_mob_render_e2e.py`, no `--wasm`)

Checks: idle pass, glow pass, pounce pass, lod pass, name pass, strike_mob pass (chicken health samples 4.0, 2.0, 2.0), death pass. **strike FAIL** (the check wants exactly one server frame and exactly 2.0 host damage; the run had five blows and the host lost more than 2).

Strike lines (mob 322, host log, raw in `logs/f4m-fix2-data-goal-lines.txt`):
```
Horizon custom-mob target: mob 322 picked Player
Horizon custom-mob strike: mob 322 begins on Player tick 3879
Horizon custom-mob strike: mob 322 lands on Player tick 3887
Horizon custom-mob strike: mob 322 hit the player for 2: hurt true tick 3887
Horizon custom-mob goal: mob 322 CustomMelee stopped: can_continue false tick 3888
Horizon custom-mob strike: mob 322 begins on Player tick 3904
Horizon custom-mob strike: mob 322 lands on Player tick 3912
... (begins/lands pairs at ticks 3922/3930, 3941/3949, 3960/3968; each goal stop follows)
Horizon custom-mob target: mob 322 target_continue: no target   (tick 4020)
```
Damage landed: **yes**. Health (min over the window, from `state` polls): host 20.0 -> 13.166667938232422, guest 20.0 -> 20.0. Server `hit` frames: 5 (ticks 3887, 3913, 3930, 3951, 3968); host and guest events in the same ticks. The fixture's 600 tick interval is not respected by the chase strike (begins repeat about every 16-20 ticks): a finding for the next fix.

### Wasm variant (`--wasm target/fox-ai/wasm32-wasip2/release/yoake_wasm_fox_ai.wasm`)

Checks: idle, glow, pounce, lod, name, death pass. **strike FAIL**; strike_mob not run in this variant.

Key lines (host, raw in `logs/f4m-fix2-wasm-lines.txt`):
```
Horizon custom-mob target: mob 331 picked Player
Horizon custom-mob script goal: mob 331 pounce can_use Some(true) (on_ground Some(true), target Some((None, true)), visible Some(true), has target_position Some(true)) target distance Some(3.1622776601683795)
Horizon custom-mob server frame: mob 331 "hit" tick 4877
Horizon custom-mob events: mob 331 tick 4877 count 3
Horizon custom-mob goal: mob 331 ScriptGoal stopped: can_continue false tick 4887
Horizon custom-mob target: mob 331 target_continue: no target   (tick 6659)
```
Target acquired: **yes** (previously never, now a player at 3.16 blocks with `can_use` true). Damage landed: **no**. The `hit` frame fires once (host and guest events in tick 4877), but no `strike` line is logged and health stays host 20.0 -> 20.0, guest 20.0 -> 20.0. The script goal's pounce does not apply damage on its `hit` frame; that is the open item for the next fix.

Death checks passed in both variants (`death_time 0`, removed at `death_time 20`).

Results: `logs/f4m-fix2-data-results.json`, `logs/f4m-fix2-wasm-results.json`.

## F4M-E2E seats follow-up (`yoake-free/f4m-e2e` @13ef01e02)

Merged into `yoake-free/merge` as merge commit beac372af (fetch 13ef01e02). Built debug `horizon-ctl` (`horizon-client-lib`) and `horizon-client`, and `seat_fox` for `wasm32-wasip2` (`target/seat-fox`). Benchmark gate clear before every build and game run.

### Run 1: with `--seat-wasm` (aborted)

Check 7 (`seat_set`) failed and the run aborted at it (the harness `AssertionError` is not caught per check, so nothing after it ran). The packaged WASM guest seated the chicken on seat 0 (centre), not seat 1:
```
AssertionError('host: state did not arrive within 40 ticks: [(0.0, 0.8)]')
```
Expected `[(0.5, 0.8)]` (seat 1). Raw: `logs/f4m-e2e-seats-wasm-aborted.log`.

### Run 2: without `--seat-wasm` (all other checks)

| Check | Result | Key values |
|---|---|---|
| 1 ride3 | pass | offsets [[0.0,0.8],[0.5,0.8],[0.5,0.8]]; fourth `/ride` refused |
| 5 guest | pass | guest sees [[0.0,0.8],[0.5,0.8],[0.5,0.8]] |
| 3 stay | pass | 3 riders -> 2 after one killed, seats kept |
| 3 vehicle | pass | vehicle removed, 3 chickens left |
| 2 use_entity | **FAIL** | host `use-entity --type sample_mobs:cinder_fox` -> `"ok": true, "riding": null, "target": 325`; host offset 2.2, steer distance 0.012 (no boarding, no steering) |
| 8, 9 multi_player | **FAIL** | guest `use-entity 325` -> `"ok": true, "riding": null`; guest offset 2.198; only two chickens on the vehicle |
| 4, 10 reload | **FAIL** | post-reload riders [[0.0,0.8],[0.5,0.8]]; host position 2.205 before and after (player seat 1 claim not kept) |
| 12 locator | **FAIL** | rest [0.75, 0.31] on host and guest; pounce delta host 0.04, guest 0.0; baby flag false |
| 7 seat_set | not run in this variant (run 1 above: FAIL) |

Guest-side `use-entity` did not stop with "no entity": it returned `ok: true` with target 325, but it did not board (`riding: null`). Raw: `logs/f4m-e2e-seats-nowasm-stdout.log`, `logs/f4m-e2e-seats-nowasm-host.log`, `logs/f4m-e2e-seats-nowasm-guest.log`, `logs/f4m-e2e-seats-nowasm-results.json`.

## F4M-FIX3 / SEATS2 rerun (GALLERIA, 2026-11-21)

Merged vps/yoake-free/f4m-fix3 (770009182) and vps/yoake-free/f4m-seats2 (377496b29) without conflicts (HEAD 4e593958d). horizon-client and horizon-ctl rebuilt (-j 4); `cargo test -p horizon-client-lib --lib custom`: 252 passed. Guest .wasm files (fox_ai, seat_fox) are the earlier builds (fix3/seats2 change no guest source). Logs: `logs/f4m-fix3-{data,wasm}.log`, `logs/f4m-seats2-{wasm,nowasm}.log`.

- custom_mob_render_e2e.py, data goal: exit 0.
- custom_mob_render_e2e.py --wasm: strike FAIL (0 server frames; host and guest health 20 -> 20). After "target: mob 361 picked Player" the guest still logs `pounce can_use None (... target Some((None, true)), visible Some(true), has target_position Some(true)) target distance Some(3.16)`: the target uuid is still None in-game, so no "script attack" line. Other checks pass.
- custom_mob_seats_e2e.py: ride3, guest, stay, vehicle pass in both runs. use_entity now boards (`ok: true, riding: 316`, steer distance 11.2) but FAILs on the offset check (host offset (0.0, 0.2)). The run then aborts: the guest's `use-entity` gets "that entity is out of reach or hidden; get closer" (the host steered the vehicle ~11 blocks away first), so multi_player, reload and locator did not run.
- --seat-wasm seat_set: FAIL, offset (0.0, 0.8) (seat 0) while the server log says `seat-set vehicle 19 seat 1 rider Some(13): mob 13 now holds seat Some(1)`: the server holds seat 1 but clients place the rider at seat 0.

## WASM-CPU-LIMIT + SEATS2 (afc240763) rerun (GALLERIA, 2026-11-21)

Merged vps/yoake-free/wasm-cpu-limit (c3672808d) and vps/yoake-free/f4m-seats2 (afc240763), no conflicts (HEAD 513ab83ec). Rebuilt horizon-client/ctl, and fox_ai / seat_fox guests fresh in separate targets (target/fox-ai-e2e, target/seat-fox-e2e). `cargo test -p horizon-client-lib --lib custom`: 254 passed. Logs: `logs/e3-{data,wasm,seats-wasm,seats-nowasm}.log`.

- render, data goal: all checks pass (strike included).
- render --wasm: the mod is no longer disabled (no "disabled" line; can_use returns Some(..)). The pounce strike lands once (server frame tick 5890) but on the guest (guest 20 -> 17, host 20 -> 20); the check expects the host, so strike FAILs. All other checks pass.
- seats --seat-wasm: seat_set, ride3, guest, stay, vehicle, use_entity pass. multi_player and reload FAIL: the guest's use-entity still gets "that entity is out of reach or hidden; get closer" after the retries. locator FAIL: pounce delta host 0.29 / guest 0.10, baby_flag false.
- seats without --seat-wasm: same as above minus seat_set (locator pounce delta host 0.25 / guest 0.11).
- Earlier "wasm trap: interrupt" on the first mob_goal_can_use (wall-clock limit) did not recur with the per-call CPU-time limit.
