# MX3PIN: one pinned toolchain (rustc 1.100.0)

Date: 2026-11-19, on `yoake2/merge`. The owner approved one pinned compiler for the workspace.

## What changed

- `rust-toolchain.toml` at the workspace root: `channel = "1.100.0"`, components rustfmt, clippy, rust-src, targets
  `wasm32-unknown-unknown` and `wasm32-wasip2` (the SDK guests the tests build use wasip2), profile minimal.
- `crates/yoake-bindgen/toolchain.txt` (1.99.1) is gone. `yoake_bindgen::toolchain` reads `channel` from the file;
  the generator CLI and the regeneration gate (`tests/engine.rs`) run `rustc -V` in the workspace and stop unless it
  is exactly that version (a moving channel such as `stable` is refused: it cannot be checked).
- `yoake-hook-thir dump` does the same check before it runs rustc (THIR text is toolchain-specific); `rustc -vV` now
  runs in the workspace, so rustup picks the toolchain the way cargo does.
- CI: `yoake-bindgen.yml` and both jobs of `yoake-windows.yml` install the file's toolchain with
  `rustup toolchain install` (no argument) instead of `dtolnay/rust-toolchain@1.99.1` / `@stable`; the other Yoake
  workflows have no toolchain step and pick the file up through rustup.
- `crates/yoake-bindgen/regen.sh` sets `RUSTDOCFLAGS=` as the gate does (the JSON's cfg is pinned).
- The generator reads `target/doc/*.json` from `CARGO_TARGET_DIR` when it is set (`yoake_bindgen::doc_path`); before,
  regen.sh failed with a separate target folder.
- docs/yoake2/BUILDING-WORKTREES.md: "The pinned toolchain" (how to move to another compiler, and how a machine
  that has the version under another name uses it without a download).

## What the regeneration on 1.100.0 changed

Regenerated with `crates/yoake-bindgen/regen.sh` (rustdoc JSON of the six engine crates, feature off).
- **Glue, SDK stubs and binding ids: unchanged.** Same bound set (items 20, entities 55, game_items 5, entity_query 7),
  same ids, no id added or removed.
- **`ids.lock.json`: 59 `sig` strings** (text only; ids and paths unchanged): rustdoc 1.100.0 prints a re-exported
  type by its re-export path (`horizon_core::geometry::Vec3` -> `horizon_core::Vec3`, 26 lines) and the result of a
  derived impl as `Self` (`fn clone(&Self) -> Experience` -> `fn clone(&Self) -> Self`, the rest).
- **One generator fix the regeneration showed.** The merge brought the hook machinery into `entities`: codemod
  call-site shims (`__yoake_cs_<id>`) and trait-method hook wrappers (`__yoake_trait_*`). Their first regeneration
  bound 19 of them as engine bindings, which would let a guest call a hook shim directly (bypassing or re-entering
  the hooks). The generator now excludes every `__yoake_*` function ("hook machinery"); with that, nothing new is
  bound and the lock gets no entries for them. `docs/yoake2/bindgen/functions-entities.md` and `coverage.md` list them
  as excluded (entities: 163 functions, 76 excluded; before the merge 101 / 14).

## Results

| check | result |
| --- | --- |
| bindgen regeneration gate (`--ignored`, with the base lock from `yoake2/merge`) | **passed** (rustdoc JSON of six crates, glue current, append-only, unsafe policy, no stale glue, `cargo check -p horizon-client --features yoake2-slice`), 277 s |
| yoake-bindgen tests (mini engine 8, policy 3, toolchain 2) | passed |
| yoake-hook-thir tests (fixtures) | 7 passed |
| `verify-callsites` after THIR dumps on 1.100.0 (dump refuses another rustc) | entities 58 of 58, item_use 41 of 41 |

The gate also showed that the codemod's `hook_bridge_*.rs`, which sit next to the bindgen glue in `yoake_glue/`,
counted as stale glue for bindgen; `stale_glue_files` now leaves them to `yoake-hook-codemod bridge --check`.

## Notes

- LENOVO had 1.100.0 installed as `stable`; its worktrees carry a rustup directory override to `stable`, which takes
  precedence over the file, so nothing was downloaded and the version checks pass. GALLERIA has
  `1.100.0-x86_64-pc-windows-gnu`.
