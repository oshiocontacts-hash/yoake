# FR-F4M-7: runtime entity textures for custom mobs

Branch `yoake-free/f4m-7` (from `yoake-free/merge` b90919bc13). Design: `docs/yoake2/FR-F4-MOBS.md`, "### F4M-7".

## Implemented against the design

| Design item | Done |
| --- | --- |
| Reserved atlas region | A `1024x1024` region `horizon/mob-textures` in the entity atlas, reserved only when the world has custom mobs, a mod holds `service.mobs`, or runtime textures exist (`game/custom_mob_textures.rs::should_reserve_mob_textures`). `horizon-client-render/src/gfx.rs` + `hud.rs` (`alloc_rect`, `mob_texture_region`). |
| Partial uploads | Dirty rectangles per slot are flushed as sprite patches (`textures::flush_patches`, `flush_custom_mob_textures` every frame). Full or sub-rectangle writes, 4 MiB per call. |
| `mob-texture-*` services | `mob-texture-create(width, height) -> u32` (1..=1024 per side, 64 live textures per mod), `mob-texture-write(id, origin, size, rgba)`, `mob-texture-bind(entity, slot, id?)` (`None` restores the pack texture). Capability `service.mobs`. Interface version 1.7.0 -> 1.8.0, services 289/290/291 in `abi-subset-client.json`; WIT, SDK (`full::client::mob_texture_*`), catalog and adapters regenerated with the generator, not by hand. Host side in `wasm_mods/client_services.rs`, state in `mobs/custom/textures.rs`. |
| Bind per mob and per layer | Slots: `""`/`"default"` (also layers that inherit it), a layer name, `"glowmask"`, a bone name or `"bone:<name>"` (that bone's quads). `render_state.rs` (`DrawCtx.texture_overrides`, `emit_bone`), applied in `custom_mob_draw.rs`. |
| Atlas rebuild / shader-pack toggle | Textures and bindings survive: `reapply_custom_mob_textures` marks every slot dirty after an atlas rebuild (`packs.rs`, `game.rs`). |
| Lifetime | Unloading or reloading a mod frees its textures and clears its bindings (`textures::release_instance`). |
| Multiplayer | Everything is local to each client; nothing is sent over the network. |
| Example | New client guest `crates/yoake-wasm-sdk/examples/fox_texture`: on `synced-changed-client` `warmth` it repaints a 64x32 texture (full write, then a partial gold band) and binds it to the Cinder Fox. `fox_ai` now sets `warmth` in `mob-hurt`. |
| Docs | `docs/yoake2/MOD-GUIDE.md` and `MOD-GUIDE.ja.md`, "Runtime entity textures". |

## Two engine bugs found by the e2e and fixed

1. `mob-hurt` ran only inside `Mob::damage_aimed`. A player's swing (`hurt_by_player` -> `damage_with`) and `/damage` skipped it, so `fox_ai` never saw a hit in a real game. The hook now runs in `Mob::damage_with` (after the refusal checks; the aimed part name comes from `attacker_aim`) and was removed from `damage_aimed`, which still scales the blow.
2. The integrated host's local mirror (`LocalMirror::sync`) overwrote synced values without reporting them, so `synced-changed-client` never fired for the host's own mobs. It now queues a `SyncedChanged` for every changed value (`RemoteCustomMobs::note_change`).

Also: a `Horizon custom-mob texture: mob N slot "default" texture Some(id)` info line per successful bind (failures log a warning), used by the e2e.

## Files

Engine: `horizon-client-render/src/{gfx.rs,hud.rs,gfx/native_services_tests.rs}`, `horizon-client/src/{mobs.rs,game.rs,game/custom_mob_draw.rs,game/custom_mob_textures.rs (new),game/packs.rs,mobs/custom/{mod.rs,textures.rs (new),render_state.rs,remote.rs},wasm_mods.rs,wasm_mods/{client_services.rs,mob_services.rs,mob_runtime.rs,resources.rs,full_adapters.rs,full_adapters_report.txt}}`.
Generated: `horizon-mod-api-gen/abi-subset-client.json`, `yoake-inject-rt/src/catalog.rs`, `yoake-wasm/wit*/…`, `yoake-wasm/src/{client_services.rs,full_hook_names.rs,test_guests.rs}`, `yoake-wasm-sdk/src/{full.rs,full_*_generated.rs}`.
Examples/tools/docs: `yoake-wasm-sdk/examples/{fox_ai,fox_texture}`, `tools/custom_mob_render_e2e.py`, `docs/yoake2/MOD-GUIDE{,.ja}.md`.

Commits: 718f13bfa6 (catalog/WIT/SDK), bc1d10ea03 (engine), 40d3f81de3 (example, e2e, docs), then the hurt/mirror fix commit and this report. (The first commit message says 1.7.0; the interface version is 1.8.0.)

## Unit tests

* `horizon-client-lib --lib custom`: 267 passed (262 before; new: slot limits/dimension checks/4 MiB write cap, mirror notes a changed value, and the earlier texture tests).
* `horizon-client-render --lib mob_texture`: 2 passed.
* `horizon-mod-api-gen --lib`: 46 passed.
* `horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests wasm_mods::mob_runtime`: 11 passed (includes `the_fox_texture_guest_creates_writes_and_binds_on_warmth_change_and_cleans_up_on_unload` and permission denial for an ungranted mod; plus `the_fox_ai_guest_sets_warmth_when_the_fox_is_hurt`).

## Render e2e (`tools/custom_mob_render_e2e.py`, --offscreen --no-focus)

* Without `--wasm`: `idle`, `glow`, `pounce`, `lod`, `name`, `brain`, `strike_mob`, `strike`, `death`, `_screenshots` all `pass: true`.
* With `--wasm yoake_wasm_fox_ai.wasm --client-wasm yoake_wasm_fox_texture.wasm`: `idle`, `glow`, `pounce`, `lod`, `name`, `brain`, `strike`, `runtime_texture`, `death`, `_screenshots` all `pass: true`. `runtime_texture`: seven player swings in two groups; host and guest each logged 5 `bind default texture 1` lines (one per warmth change) and both screenshots differ from the first; the fox is visibly repainted blue/gold (`target/e2e-shots-wasm/f4m6-tex-*.png`).

## Left open

* The pixel-difference part of `runtime_texture` compares whole frames, so it is a weak signal on its own (the fox moves when hit); the bind log lines carry the check. A cropped comparison around the fox would be tighter.
* The mod's WASM `println` output is not forwarded to the log in the real game for client modules (not investigated); the e2e therefore relies on the host-side bind line.
* `mob-hurt` now also fires for blows that never had an aim (part name is then empty); a shader-pack-toggle test in a running game was covered by the unit test of dirty re-flush only, not by the e2e.
