# FR-F3-8b: data route, current-level, safety, hooks (partly done)

Branch `yoake-free/f3-8b` (from f3-8). Stopped early: C: fell under 12 GB (11 GB, another agent building in horizon-f1) and my
target dir had no `deps` left, so any build of horizon-client would need about 15 GB. Per the disk rule I paused builds.

## Done and verified
* F3-8a: `horizon-mods` checks `data/<ns>/dimension`, `dimension_type` and `worldgen/**` JSON in owned namespaces; errors start with
  the file path (`check_level_json`, test `level_json_is_checked_with_its_path`); `cargo test -p horizon-mods`: 59 passed.
  `docs/mods.md` section "Dimensions and world generation in a `.yak`". Committed.

## Written, NOT built or run (needs a build with >= 15 GB free)
* Item 2, `current_level(player) -> result<string,string>` (service 235, server-read): catalog row, `full.rs` trait line, impl in
  `server_services.rs` (finds the player's `dimension` in `server_players()`), SDK `ext::current_level`, unit assertion in
  `level_services.rs`. `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen` ran and passed (46 tests), regenerating the WIT, adapters
  and SDK bindings (these files are in the commit). The horizon-client side has not been compiled.
* Item 4, generator safety: `WasmColumnSource::generate` retries a failed call once after 50 ms (not when the module is not running);
  a final failure calls `ModChunk::mark_failed`, `ModTerrain::proto` sets the new `Chunk::not_generated`, `save`/`save_later` skip such
  chunks, and the first block written clears the mark (test `an_edit_clears_the_not_generated_mark` in horizon-client-render).
* Item 3 (part): injected hook points `level::before_open`, `level::after_open`, `level::before_close` (in `game/levels.rs`, called
  from `enter_level` and the three drop sites in `level_manager.rs`) and `worldgen::generate_chunk(level, cx, cz, &mut ModChunk)` in
  `mod_generator.rs` (runs before a mod generator fills the chunk; mod levels only, vanilla terrain untouched).

## Not done
* `inject-sites.json` / `yoake-inject-rt/src/catalog.rs` are not regenerated for the 4 new sites (run
  `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib`; the test counting 56 sites in `inject_catalog.rs` becomes 60), nor
  `site_policy` / `docs/yoake-mixin.md` counts.
* `yoake-bindgen` target for `ModChunk` (`set_block`, `fill_layer`, `block`, `set_biome`), `ids.lock.json`, `regen.sh` (needs 15 GB),
  `ENGINE_CALL_ABI.md` and `MOD-GUIDE.md` notes.
* Item 5 (two-process e2e with the level-generator sample): needs the binaries built.
