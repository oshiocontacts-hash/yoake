# FR-F1 W10 (wave 4): more crates stamped, `Game.world`/`Game.settings`, W12 signatures, fewer warnings

Branch `yoake-free/f1-w10` (from `f1-w9b`). Everything below was verified with a stamped `cargo check -p horizon-client --features yoake2-slice` through the overlay (no full stamped build, no binary; the release gate and W6 stay for later). Last stamped check: clean, 0 errors, 10m22s from a half-warm target.

## 1. W10: five more crates stamped, with bridges

`horizon-client-render`, `horizon-core`, `horizon-protocol`, `horizon-items`, `horizon-command` now go through `stamp-config.txt` (`min_stmts=3 private_types=true bridges=true`), each with the usual infra: hook macro/rt deps, optional `yoake-slice` + `yoake2-slice` feature (wired through `horizon-client-lib`), `build.rs` `emit_crate_table`, `__yoake_hook_keys`, a committed ledger shard, and an entry in the loader's `STAMPED_BRIDGES` (now 8 modules). Plain `cargo build` stays unstamped.

| crate | hooks in ledger | bridged | unbridged |
|---|---|---|---|
| horizon-client-render | 1,546 | 1,450 | 62 (4.1%) |
| horizon-core | 233 | 209 | 15 (6.7%) |
| horizon-protocol | 292 | 282 | 2 (0.7%) |
| horizon-items | 62 | 54 | 0 |
| horizon-command | 87 | 87 | 0 |
| (existing) horizon-world / client-lib / client | 878 / 3,839 / 3,817 | 843 / 3,183 / 3,707 | 35 / 30 / 25 |

Overall about 9,800 bridged and 169 unbridged in the stamped bridge files, **1.7% unbridged** (counts are from the bridge generator's per-file output; client-lib and client share a source tree so their split is approximate). No field tables were generated for the new crates (that needs rustdoc JSON per crate); their bridges treat object arguments as held opaque handles, which compiled and is why core and render have the highest unbridged shares. Existing ids did not change (checked entry by entry against the previous ledger).

Needs on this machine: the GNU toolchain needs `C:\Users\USER\NestgateHorizon\toolchain\mingw64\bin` on PATH (ring's build script wants gcc).

## 2. W9c: `Game.world` and `Game.settings`

* `Game.options` is the launch-argument struct (`Options` in game.rs, command line, not settings); the player's settings are `Game.settings: Settings`. I exposed `Game.world` (`World`, the level) and `Game.settings` (`Settings`), both from `horizon_client_render`: bindgen got the render crate in its rustdoc set, two targets (`render_world`, `render_settings`, glue in `crates/horizon-client-render/src/yoake_glue/`, guest stubs `render_world.rs`/`render_settings.rs`) and two `game_paths` (`Game.world#handle`, `Game.settings#handle`, ids 1515/1516). Getters for every scalar field, setters reserved and refused (`ReadOnly`) since `writable` is empty. Re-running bindgen with the render crate in the set also bound more of `mobs` and `motion` (their methods take `World`), so those glue files grew.
* `Game.options` itself (the launch struct) is not exposed: it would need `Options` moved out of the 22k-line game.rs into its own module for a bindgen target. Say if you want it.
* Test: `crates/horizon-client-render/tests/yoake_game_ref.rs` (`--features yoake2-slice`): a `&Game`-shaped root with the two paths opens both handles through `activation::scope_n` + `root_shared`, calls the generated `World.day_time#get` and `Settings.fov#get`, and checks that setters are refused, a handle of the wrong module is `WrongType`, and a `&mut` borrow through the shared root is `ReadOnly`. **Limits:** the `Game` is a stand-in with the same two paths (the real one needs a window and GPU; the real `impl PathRootRef for Game` is generated and only compile-checked), and the guest side is native Rust calling the generated binding table, not a wasm guest.

## 3. W12: generics, `impl Fn`, by-value self

* By-value `self` / `mut self` of a type that derives `Copy` in the crate: the macro takes it as an ordinary by-value argument; the codemod passes a per-impl `FnContext{self_copy}`.
* `impl Fn(..)` / `impl FnMut(..)` arguments (single bound, no borrows in the closure's signature): the hook sees `&dyn Fn` / `&mut dyn FnMut`; with no mod the original gets the real closure. `impl FnOnce`, extra bounds and closures with `&` in their signature stay skipped (the macro refuses nested borrows).
* Type-parameter generics where every parameter already says `'static` (inline or in `where`): hookable natively with the existing typed registration, one instantiation at a time. I did not add the "list instantiations for the wasm bridge" part: the bridge refuses generics, a guest cannot name `T`.
* Bridge: all three kinds stay unbridged (it refuses by-value self, generics and now any `impl Trait` argument).
* Macro tests (`crates/yoake-hook-rt/tests/mechanics.rs`, 18 pass) cover all three; codemod eligibility test added. **Yield is small:** about 150 new hookable functions across the eight crates (ledger +22 world, +26 client-lib, +12 client, +36 render, +15 core, +2 protocol), nowhere near the 871/802/766 skipped by those reasons, because most of those functions are also small-bodied, generic over non-`'static` params, take `impl Trait` other than closures, or sit in generic impls.

## 4. Warnings

The 1,901 warnings of the stamped lib were `non_snake_case` on the trait-register functions (2,920 over the build) and `private_interfaces` on the field-table statics (635). Fixed at the source: `non_snake_case` on the macro's register fn, `private_interfaces, private_bounds` in the generator's allow line (render.rs) and in the 513 committed generated files. Stamped check now: client-lib 4, client 13, render 1, no warnings from world/core/protocol/items/command. The rest are unrelated (manifest, mod-api-gen, slice, callsite-match, test-runner).

## 5. Disk and cleanup

Targets (`target`, `target-stamped`, `target-doc`, rustdoc JSON, overlays) were deleted along the way and at the end; sizes in the hand-off message.

## 6. Still open

Release gate and W6 (need >= 30 GB, quiet machine); bridging by-value Copy self; tables for the five new crates; a wasm guest test over the real generated `Game` bridge (needs the full stamped build).
