# FR-F4M-5a: proxy vanilla mobs for stock clients and brand gate

Branch `yoake-free/f4m5a` (from `yoake-free/merge` @ `3439626d69`), commit `4c0741f546`. Not pushed.

## What was implemented

1. **Proxy mapping (`crates/horizon-client/src/lan/mob_proxy.rs`, `lan/hub.rs`, `lan/entity.rs`):**
   - Added `lan/mob_proxy.rs` with `ProxyTarget { kind, name_tag }`, `proxy_for` / `proxy_in`, `wire_kind` / `wire_kind_in`, `is_living`, `stock_meta`, and `stock_defaults`.
   - `lan/hub.rs`: `entity_type(..., stock: bool)` (and test helper `entity_type_with`) resolves custom mob types through `mob_proxy::wire_kind(kind, stock)` before looking up the protocol id in `SyncData`. `Hub` sets `Viewer.stock = g.brand != "horizon"` and passes `stock` into `Entities::tick`.
   - `lan/entity.rs`: `ServerEntity` stores `stock_kind`, `stock_proxy`, and `stock_meta`. For stock viewers (`v.stock == true`) of a proxied custom mob, `pairing_stock` sends `AddEntity` with the vanilla `proxy` protocol id, filters `SetEntityData` to indices/serializers valid for the vanilla proxy class (`Entity` `0..=7`, `Living` `0..=14`, `Mob` `0..=15`, `BabyAt16Mob` `0..=16`, `AgeableMob` `0..=17`) with optional `proxy_name_tag` applied on indices `2`/`3` if unset, and suppresses `UpdateAttributes` / `SetEquipment` when `!is_living(&proxy.kind)`. Movement, head rotation, leash/passenger links, and `RemoveEntities` continue through the standard tracker path (`encode_stock_changes` replaces only `SetEntityData` and gates `UpdateAttributes`/`SetEquipment` on `living`).
   - `horizon:mob_sync` remains gated by `lan::mob_sync::reads_mob_sync` (`g.placed && g.brand == "horizon"`), so stock clients receive no `horizon:mob_sync` payloads.

2. **Brand gate (`crates/horizon-client/src/lan/conn.rs`):**
   - Extracted `brand_gate_reason(brand, horizon_clients_only, custom_components, unproxied_custom_mobs)` and wired it into `login_and_configure` with `super::mob_proxy::any_without_proxy()`.
   - When a client's `minecraft:brand` is not `"horizon"` and any loaded custom mob type has no `proxy`, configuration disconnects up front with the existing `crate::content_identity::MISMATCH` text (`"Content registry mismatch: install the same content mods as the server."`), or `HORIZON_CLIENTS_ONLY_REASON` if `horizon_clients_only` is set.

3. **No-send rule (`crates/horizon-client/src/lan/mob_proxy.rs`, `lan/entity.rs`):**
   - `wire_kind` returns `None` for any custom mob type without `proxy` when `stock == true`.
   - `ServerEntity::stock_kind` is therefore `None`, so `Entities::tick` never pairs or emits `AddEntity` for an unproxied custom mob to a stock viewer.

4. **Unit tests (`crates/horizon-client/src/lan/mob_proxy.rs`, `lan/conn.rs`):**
   - `lan::mob_proxy::tests::proxy_lookup_returns_vanilla_target_and_name_tag`
   - `lan::mob_proxy::tests::no_proxy_detection_finds_any_unproxied_custom_type`
   - `lan::mob_proxy::tests::stock_metadata_filters_to_valid_vanilla_indices_and_applies_name_tag`
   - `lan::mob_proxy::tests::stock_guest_never_gets_a_custom_protocol_id` (verifies Horizon vs stock `entity_type_with`, `AddEntity`, filtered `SetEntityData`, non-tracking of unproxied mob, movement + health metadata update, and `RemoveEntities` across ticks)
   - `lan::conn::tests::brand_gate_refuses_stock_clients_for_unproxied_custom_mobs`

## Tests and build results

| Command | Result |
|---|---|
| `.\hz-cargo.cmd test -j 6 -p horizon-client-lib --lib mob_proxy` | 4 passed, 0 failed |
| `.\hz-cargo.cmd test -j 6 -p horizon-client-lib --lib custom` | 294 passed, 0 failed, 1 ignored (`custom_mobs_200`) |
| `.\hz-cargo.cmd test -j 6 -p horizon-client-lib --lib lan::` | 234 passed, 0 failed, 1 ignored |
| `.\hz-cargo.cmd build -j 6 -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl` | Exited 0 |
| `.\hz-cargo.cmd build --release --target wasm32-wasip2 --manifest-path crates/yoake-wasm-sdk/examples/fox_ai/Cargo.toml` | Exited 0 |
| `.\hz-cargo.cmd build --release --target wasm32-wasip2 --manifest-path crates/yoake-wasm-sdk/examples/fox_texture/Cargo.toml` | Exited 0 |
| `.\hz-cargo.cmd build -j 6 -p horizon-client --example level_probe` | Exited 0 |
| `.\hz-py.cmd tools\levels_e2e.py --bin-dir C:\Users\USER\NestgateHorizon\horizon-f4m15\target\debug --out C:\Users\USER\NestgateHorizon\horizon-f4m15\target\levels-e2e-f4m5a` | Exited 0 (`setup`, `travel`, `custom_portal`, `stock`, `created`, `lifecycle` all PASS) |

## Acceptance scenario `s6` result

Command:
`.\hz-py.cmd tools\custom_mob_acceptance.py --bin-dir C:\Users\USER\NestgateHorizon\horizon-f4m15\target\debug --wasm C:\Users\USER\NestgateHorizon\horizon-f4m15\target\wasm32-wasip2\release\yoake_wasm_fox_ai.wasm --client-wasm C:\Users\USER\NestgateHorizon\horizon-f4m15\target\wasm32-wasip2\release\yoake_wasm_fox_texture.wasm --only s6`

Result: **`s6` PASS** (exit 0):
- `with_proxy`: host summoned `sample_mobs:cinder_fox` at `(6.5, 69, -2.5)`. The stock-brand client reached `play` (`disconnected: null`, `config_channels: ["minecraft:brand"]`, `play_channels: []`) and received `AddEntity` for `entity_id: 115` with `protocol_id: 55` (`minecraft:fox`) at `[6.5, 69.0, -2.5]`.
- `without_proxy`: stock-brand client was refused in `configuration` with `Content registry mismatch: install the same content mods as the server.` and saw no entities.

## What is left

- Full `tools/pr_ready.py` run (workspace `fmt`, `clippy -D warnings`, Python unittests, parity harnesses such as parity B, and all touched crates).
- Remaining `custom_mob_acceptance.py` scenarios (`s1`–`s5`, `s7`–`s10`) were not run on this branch.
- Live multiplayer e2e combat hurt/death on a proxied mob for a stock client is covered by the `mob_proxy` unit test (`SetEntityData` health update + `RemoveEntities`) and tracker/hub inspection, not a dedicated e2e combat scenario.
- `rustfmt` check on the pre-existing files `lan/conn.rs`, `lan/entity.rs`, `lan/hub.rs`, `lan/mod.rs` showed formatting diffs in untouched sections of those files; only the new `lan/mob_proxy.rs` was formatted with `rustfmt --edition 2024`.
