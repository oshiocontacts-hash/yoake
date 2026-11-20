# FR-F3-8b: data route, current-level, generator safety, level and worldgen hooks

Branch `yoake-free/f3-8b` (f3-8, then `vps/yoake-free/merge@fed161c18` merged; two conflicts only, `game.rs` mod list and
`ENGINE_CALL_ABI.md`, both kept as both sides; no generated file conflicted and the regenerated catalog was unchanged).

## Done and verified
* F3-8a: `horizon-mods` checks `data/<ns>/dimension`, `dimension_type`, `worldgen/**` JSON (errors start with the file path);
  `docs/mods.md` section. `cargo test -p horizon-mods`: 59 passed.
* Item 2: service 235 `current_level(player) -> result<string,string>` (server-read), SDK `ext::current_level`, WIT/adapters/SDK
  regenerated; `horizon-mod-api-gen --lib`: 46 passed after the merge.
* Item 4: a failed generator call is retried once after 50 ms; a second failure marks the chunk `not_generated`
  (`ModChunk::mark_failed` -> `Chunk::not_generated`), `save`/`save_later` skip it, the first block written clears it.
* Item 3 (hooks): `level::before_open`, `level::after_open`, `level::before_close`, `worldgen::generate_chunk` (mod-generator levels
  only); catalog 56 to 60 sites, `site_policy` for the chunk coordinates, notes in `MOD-GUIDE.md` section 16 and `ENGINE_CALL_ABI.md`.
* `cargo test -p horizon-client --lib`: 1231 passed, 0 failed, 51 ignored.
* Ignored `level_registry_tests` with `HORIZON_TEST_RESOURCES` (vanilla chunk hashes, hub/registry parity, the WASM generator level
  that saves): 11 passed.

## Not done
* Suites of horizon-client-lib, horizon-world, yoake-wasm (`YOAKE_REQUIRE_GUESTS=1`), yoake-value, horizon-client-render: the run was
  stopped twice because C: fell under 12 GB (down to 8.9 GB while other agents built), so no result.
* `yoake-bindgen` target for `ModChunk` and `ids.lock.json` (`regen.sh` needs 15 GB free; the free space never held there).
* F3-8d two-process e2e (needs the binaries built; not started).
* No test of its own for the save skip of `not_generated` chunks (covered only by the flag-clearing unit test).
