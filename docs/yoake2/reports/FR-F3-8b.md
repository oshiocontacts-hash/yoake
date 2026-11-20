# FR-F3-8b: data route, current-level, generator safety, hooks, bindings, e2e

Branch `yoake-free/f3-8b` (f3-8, then `vps/yoake-free/merge@fed161c18` merged; two textual conflicts only, `game.rs` mod list and
`ENGINE_CALL_ABI.md`, both kept as both sides; the regenerated catalog did not change).

## Done
* F3-8a: `horizon-mods` checks `data/<ns>/dimension`, `dimension_type`, `worldgen/**` JSON (errors start with the file path);
  `docs/mods.md` section.
* `current_level(player) -> result<string,string>` (service 235, server-read) with SDK `ext::current_level`; WIT, adapters and SDK
  regenerated.
* Generator safety: a failed generator call is retried once after 50 ms; a second failure marks the chunk `not_generated`
  (`ModChunk::mark_failed` -> `Chunk::not_generated`); `save` and `save_later` skip it, the first block written clears it.
  Test `save::tests::a_chunk_filled_by_a_failed_generator_is_not_saved` (needs `HORIZON_TEST_RESOURCES`) and
  `an_edit_clears_the_not_generated_mark`.
* Hook points `level::before_open`, `level::after_open`, `level::before_close` and `worldgen::generate_chunk` (runs before a mod
  generator fills a chunk; mod-generator levels only, vanilla terrain untouched); catalog 56 to 60, `site_policy` for the chunk
  coordinates, notes in `MOD-GUIDE.md` section 16 and `ENGINE_CALL_ABI.md`, count in `yoake-mixin.md`.
* Bindings: `ModChunk` no longer carries a lifetime (it owns its chunk and holds the biome lookup in an `Arc`), so `yoake-bindgen`
  can bind it. New target `mod_generator` in `bindgen.json`; `regen.sh` run with 15 GB free (rustdoc JSON deleted right after):
  `block`, `fill_layer`, `mark_failed`, `set_biome`, `set_block` and the fields are bound, `ids.lock.json` only gained 150 lines
  (ids appended, none changed). Glue `yoake_glue/mod_generator.rs`, guest stubs `stubs/mod_generator.rs`, dispatch chain in
  `native_client_items.rs` (5 modules); `cargo check -p horizon-client --features yoake2-slice` is clean.
* Sample fix: `level-generator/yak.toml` had `world = "server-full"` under `[wasm]`, which the game refuses
  (known worlds are client-mod, client-full, inject-mod; a server module is always server-full). Removed.
* F3-8d e2e `tools/levelgen_wasm_e2e.py` (two processes, host plus Horizon guest, no other horizon-client running): the host loads
  the sample as an installed package (`yoake-mods.txt`, `YOAKE_WASM_UNSIGNED=1`), lists `levelgen:terraces`; the guest enters it
  and sees y 0 height 64 facts, bedrock at y 0, stone at y 1 and in 7 sampled columns the grass top at
  `8 + 4 * ((x div 8 + z div 8) mod 4) + b` (b = 0 here), not a flat floor, air above; back in the Overworld the vanilla facts
  return. All three checks PASS.

## Test results (after the merge)
* `horizon-client --lib`: 1231 passed, 0 failed, 51 ignored. Ignored `level_registry_tests` with `HORIZON_TEST_RESOURCES`
  (vanilla chunk hashes, hub/registry parity, the WASM generator level that saves): 11 passed.
* `horizon-client-lib` 1797, `horizon-world` 1456, `yoake-wasm` (`YOAKE_REQUIRE_GUESTS=1`) 117, `horizon-mods` 385,
  `yoake-value` 17, `horizon-client-render` 68 passed, 0 failed.
* `horizon-mod-api-gen --lib`: 46 passed; `yoake-bindgen`: all passed (engine gate included).

## Notes
* Before the final regeneration the hook catalog was stale (the macro refuses to compile with a stale catalog), so after any
  change to a hooked signature run `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib inject_catalog` first, then
  `regen.sh`.
* The `worldgen::generate_chunk` hook runs on a chunk-generating worker thread; a hook that takes long loses the chunk to the
  safety rule above.
* A `hook_bridge_mod_generator.rs` (the ledger-id bridge of the F1 tooling) was not generated for the new hook sites.
