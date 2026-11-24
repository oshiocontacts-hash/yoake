# FR-REM18: data generation and yoake worker build

Branch `yoake-free/rem18-datagen` (worktree `horizon-rem18`), from `yoake-free/merge`.

## Goal and result

| Goal | Result |
|---|---|
| Typed builders for the JSON a content mod's folder holds (block states, block and item models, recipes, loot tables, tags, lang, content kinds block/item/enchantment/damage_type/game_rule/fluid), at the paths `horizon-mods` reads | Done: `crates/yoake-datagen`. Paths come from `crates/horizon-mods/src/content.rs` (`data/<id>/horizon/<kind>/`, `assets/<id>/{blockstates,models,items,lang}/`, `data/<id>/{recipe,loot_table}/`, `data/<ns>/tags/<block|item>/`). |
| Run it: `cargo horizon-mod datagen`, writes into the mod folder, then the existing check validates; `--check` for CI | Done: `crates/cargo-horizon-mod/src/datagen.rs`. Opt-in is a `datagen/` crate beside the manifest (a content mod has no `Cargo.toml`, and `build` refuses one), not `[package.metadata]`. |
| yoake-cli: worker and full-world module build/check | Full-world (`client-full`, `server-full`) already existed in `build`/`check`/`dev`, so nothing was added there. The worker (`[wasm] worker`, world `worker-mod`) was missing: added to `build` and `check`. |
| Tests: builder exact JSON, integration (datagen on a tiny example mod, then the package check) | Done, see below. |
| Docs: `docs/mods.md` "Data generation" section; MOD-GUIDE not touched | Done. |

## Commits

- `220a41a83f` REM-18: yoake-datagen, typed builders (`Cargo.toml`, `Cargo.lock`, `crates/yoake-datagen/`)
- `8bc6162d60` REM-18: cargo horizon-mod datagen [--check]
- `37005c5214` REM-18: yoake build/check a worker module; mods.md Data generation
- `3c158c8a0e` REM-18: datagen integration covers the loader's content checks

## Files

- New: `crates/yoake-datagen/{Cargo.toml,src/lib.rs,src/tests.rs}`, `crates/cargo-horizon-mod/src/datagen.rs`, this report.
- Changed: `Cargo.toml` (workspace member), `Cargo.lock`, `crates/cargo-horizon-mod/src/main.rs` (subcommand dispatch and usage), `crates/yoake-cli/src/main.rs` (worker in `build` and `check`), `crates/yoake-cli/src/tests.rs` (one test), `docs/mods.md`.
- Not committed: `GAP3-ref.md`, `hz-cargo.cmd`, `target/`.

## Tests

- `yoake-datagen`: 14 unit tests pass. Each builder is compared with exact text or with the expected JSON written out in the test. One run test covers write, check, and drift detection. One test checks that writing a path twice is refused.
- `cargo-horizon-mod`: 8 tests pass. The new one runs `datagen` on a template content mod with a `datagen/` crate that depends on this checkout's `yoake-datagen`. It generates a block, item, recipe, loot table, tag, enchantment, damage type, game rule and lang, then builds the `.hmod` and checks it with the registry checks (`Content::read` included), then `--check` passes, a drifted file fails, and a rewrite restores it. The loader rejected two real mistakes in my first generator (a fluid's blocks were undeclared, and the fluid's textures were missing), so the test is checking against the loader, not just against itself.
- `yoake` (yoake-cli): `a_declared_worker_is_checked_as_a_file_beside_the_client` passes on its own. The full package run was not finished: 30 of 35 tests reported ok, and 5 WASM-guest tests (`checks_full_demo_example_manifest`, `rejects_*`) had not reported after roughly 40 minutes in a debug build. I did not touch them. An earlier run also stalled on the same tests, so they may be slow or stuck. Not verified: whether they pass on the base commit.

## Notes and limits

- Generated paths are owned by the generator. `datagen` does not delete files it no longer writes, and it overwrites a hand-written file at the same path.
- Fluids need texture PNGs under `assets/<id>/textures/` that datagen does not make, so the integration example leaves fluids out. The fluid builder is covered by its unit test only.
- The worker is checked as a file (declared, regular, within the size limit, fresh against sources). yoake-wasm has no public host to instantiate a `worker-mod` component, so the WIT world is not validated by `yoake check`.
- Recipe and loot JSON follow vanilla 26.x as I know it, not a loader that checks them. `cookingtime` is confirmed by `crates/horizon-client/src/lan/advancements.rs`. Loot pools use the vanilla `"conditions": [{"condition": ...}]` form. The shipped template `crates/cargo-horizon-mod/template/content/loot.json` uses `"condition": {"type": ...}` on the pool instead. I did not change the template; it may need a look.
- `--check` compares bytes. If a mod repo checks out with CRLF conversion, `--check` will report differences; a `.gitattributes` rule forcing LF on generated paths would avoid that.

## Left

- Run the five stalled yoake-cli tests to completion, and compare against the base commit if they still stall.
- Decide on the loot template shape (see above).
- Optional: a `yoake datagen` for WASM packages. The task allowed either command; only `cargo horizon-mod datagen` was built.
- Fluid example with placeholder textures, if the fluid path should be covered end to end.
