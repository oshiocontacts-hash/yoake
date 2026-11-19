# MX3MOB: mob and player glue, fn hooks and bridges

Date: 2026-11-19, on `yoake2/merge`, rustc 1.100.0, LENOVO. Glue by Claude; codemod `--shallow` by agy (Gemini 4
Hydrogen); the mob bench by Kimi (K3.1); review, fixes, builds and the gate by Claude.

## 1. Glue (commit 86b0455c6)

- bindgen targets `mobs` (`horizon_client_lib::mobs`, mobs.rs: 90 functions, 359 fields bound) and `motion`
  (`PlayerMotion` and friends, motion.rs: 41 functions, 89 fields). Writable: Mob position, velocity, rotations,
  on_ground, health, fire, air, frozen ticks, absorption, fall distance, no_gravity, no_ai, silent, invisible, glowing,
  persistent, name_visible; PlayerMotion position, velocity, yaw, head_yaw, fire_ticks, air, fall_distance, flying.
- Game paths `mob` (`Game.mobs` by id) and `player_motion` (`Game.player`, an `Option`: `None` is `Stale`; new
  `optional` in `game_paths`). `engine_call` dispatch tries the mobs and motion glue; SDK stubs `mobs`, `motion`.
- Generator: `&'static` arguments are excluded (a value decoded for the call cannot be kept: `Mob::add_effect`);
  refused writable names match whole snake_case segments, not substrings (`silent` is a field; `len`, `item_len`,
  `entity_id`, `uuid` are not), with a test.

## 2. fn hooks on mobs and motion

- New codemod flag `--shallow` (agy): a `--module` then means only that module's own items, not its inline or file
  child modules. `hook-ledger/modules.txt` lines `mobs shallow=true`, `motion shallow=true`; CI passes the flag.
  (mobs has 37 child modules in about 100 files; they are not instrumented.)
- `codemod apply --shallow`: **110 hooks in mobs.rs, 197 in motion.rs** (one more was excluded, below).
- Two fixes the build showed:
  - **macro hygiene**: the generated registration fn returned a bare `Result<(), &'static str>`; motion.rs imports a
    one-parameter `Result` alias, so 198 functions failed. `yoake-hook-macro` and the bridge generator now write
    `::core::result::Result` / `::core::option::Option`.
  - **`yoake_inject`**: `PlayerMotion::set_sprinting` also carries the 1.x `#[yoake_inject]`, which wraps the function
    so the hook's claim symbol was emitted twice (a release-build error). The codemod now refuses functions wrapped by
    `yoake_inject` ("wrapped by yoake_inject", with a test); its ledger entry (535) is a tombstone.

### Bench gate (`examples/yoake_mob_hook_bench.rs`, `tools/yoake-hook-bench-gate.py --metrics mob_tick_ns_per_mob`)

1024 cows on a flat stone floor without game resources (Kimi's `mobs::mob_hook_bench`), `Mob::tick`, 12 samples of
200 ticks, median ns per mob per tick; baseline = the same build with `yoake-hook-baseline` (every fn hook off),
7 alternating runs:

| | median of runs (ns per mob per tick) |
| --- | --- |
| hooks off | 4519.8 |
| hooks on, empty | 4700.5 |
| **overhead** | **+4.0 %** (limit 5 %): passes |

All hooks stay. The laptop is noisy (runs vary by about ±10 %), so the margin is small; if a later change pushes it
over, the first candidates for `skip` are the per-mob helpers called several times per tick, or a synthetic per-tick
"mobs ticked" hook instead of per-mob hooks. (A first gate run showed +3.0 %; it was invalid, both exes were the
baseline because the instrumented build had failed, see the fixes above. The gate is now run only after a successful
build, and the bench prints which build it is.)

## 3. Bridges

| module | bridged | unbridged (main reasons) |
| --- | --- | --- |
| mobs | 68 | 33: arg0 not a reference 10, `&T`/`&self` without a table 17, no FieldTable 5 |
| motion | 23 | 174: arg0 type has a non-`'static` lifetime 167 (`MotionWorld<'_, '_, '_>` and similar contexts) |
| entities (unchanged count) | 81 | 59 |
| game::item_use | 29 | 35 |

Generator change: a by-value argument that is not known to be `Copy` (known: primitives, shared references, types the
bridged module derives `Copy` for, tuples, arrays and `Option`s of those) is moved into the first `original` call: the
dispatcher holds it in an `Option` and takes it, so the real original runs at most once and a later `original`
returns `0`. Before, such hooks did not compile (`Option<Arc<str>>`, `Option<ItemStack>`, `Option<&mut dyn FnMut>`).
This also applies to some existing entities and item_use hooks whose by-value types come from other crates (ids 16,
61, 62, 130, 168, 172, 175); hook 71 keeps calling `original` twice (its arguments derive `Copy` in entities.rs).

Loader: `yoake2_mods::BRIDGE_MODULES` lists the four bridges (name, `BRIDGED`, `mode`, `install`); `hook_mode` and
`install_bridges` go through it, and the disjoint-ids test covers every pair.

## 4. Tests

- `mobs::yoake_glue_tests`: the glue on a real `Mob` through a hook handle (get and set health, the id setter refused).
- `mobs::yoake_bridge_tests`: a guest on 445 `Mob::extinguish_fire` (Full; not calling `original` cancels: the fire
  stays) and 446 `Mob::is_fully_frozen(&self)` (read-only argument; the guest's `1` replaces the result).
- codemod: `--shallow` (4 tests), the `yoake_inject` rule.

## 5. Results (all suites on this commit, rustc 1.100.0)

| suite | result |
| --- | --- |
| hook crates (incl. codemod shallow and yoake_inject) | 134 passed |
| `horizon-client --lib` off / `yoake2-slice` | 1196 / 40 ignored; 1199 / 44 ignored |
| `horizon-client-lib --lib` off / on | 1639 / 48 ignored; 1641 / 48 ignored |
| horizon-mods; with `yak` | 44; 119 |
| yoake-wasm, horizon-protocol, launcher | 124, 220, 797 / 8 ignored |
| codemod `check` and `bridge --check`: entities, item_use, mobs + motion (`--shallow`) | clean |
| yoake2_glue / engine_call / slice / bridge | 31 / 1 ignored |
| yoake2_loader, yoake2_examples, yoake2_template | 43, 2, 2 |
| yoake-slice (`wasm`), yoake-guest-sdk (serial: the guest builds share a temp folder) | 77; 13 / 1 ignored |
| yoake-bindgen mini engine; regeneration gate | 8; passed (after regenerating the coverage reports: the new hook machinery shows as excluded) |
| mob bench gate | +4.0 % (limit 5 %) |
