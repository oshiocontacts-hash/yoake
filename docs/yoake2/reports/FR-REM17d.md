# FR-REM17d lane TRADE: mutable villager trade-list event

## What changed

* Added server catalog hook `villager_trades` (`tools/native-hook-fragments/rem17d_trade.json`), targeting `horizon_client::game::native_hooks::Game<'_>::native_villager_trades`:
  ```rust
  pub(super) fn native_villager_trades(
      &mut self,
      entity: u32,
      kind: &str,
      profession: &str,
      level: i32,
      offers: &mut Vec<MerchantOffer>,
  )
  ```
  with `"mod_api": { "contract": "collect", "out": "offers" }`. Mods may add, remove, reorder, and edit `MerchantOffer` entries (including `base_cost_a`, optional `cost_b`, `result` with components/NBT, `uses`, `max_uses`, `reward_exp`, `special_price_diff`, `demand`, `price_multiplier`, `xp`).
* Wired offer-generation sites in `crates/horizon-client/src/game/trading.rs`:
  * `Game::ensure_offers`: fires `villager_trades` when initial offers are generated for a villager (first interaction or after `villager::set_profession` marks `unset = true`) or a wandering trader (lazy offer generation on first interaction after spawn).
  * `Game::merchant_apply`: fires `villager_trades` on `merchant::Event::LevelUp(level)` when `mob` is `Some(id)` and `self.remote.is_none()` (authoritative server, before `lan_send_offers` syncs the updated offer list to the client and before `merchant_closed` stores it back to `m.extra.trade`).
  * No-listener fast path: `Game::dispatch_villager_trades` checks `horizon_client::mod_api::mods().is_empty()` first and returns immediately without cloning or touching `offers`.
* Added host-side offer validation in `crates/horizon-client/src/merchant.rs`:
  * `MAX_MOD_OFFERS = 64`, `MAX_OFFER_STACK = 99`, `valid_offer_item_id`, `item_stack_limit`, `validate_offers_with`, `validate_offers`.
  * Enforces non-air valid `namespace:path` item IDs, known items against `Items` (or `published_stack_limit`), stack count bounds (`1..=item.max_stack`, or `1..=99` when no registry is loaded), list length `<= 64`, `max_uses >= 1`, `uses >= 0`, `xp >= 0`, and finite non-negative `price_multiplier`. Invalid edits fall back to the pre-dispatch original offer list.
* Re-exported `ItemCost` alongside `MerchantOffer` in `crates/horizon-client/src/mod_api.rs` (`mod_api::{ItemCost, MerchantOffer}`).

## Design notes (engine behaviour observed)

* **Wandering trader spawn**: `WanderingTraderSpawner::spawn_trader` (`crates/horizon-client/src/game/wandering_trader.rs`) spawns the mob with `extra.trade = None` (`unset = true`); offers are generated lazily in `Game::ensure_offers` on first interaction (`interact_trader`), matching vanilla `AbstractVillager.getOffers`.
* **Restock**: `Trader::restock` (`crates/horizon-client/src/merchant.rs`) only updates `demand`, resets `uses = 0`, and records `last_restock` / `restocks_today`; it does not regenerate offers, so it does not fire `villager_trades`.
* **WASM WIT record**: `merchant-offer` and `item-cost` already exist in `engine-types` (`crates/yoake-wasm/wit/yoake-full.wit`), and `horizon-mod-api-gen` maps `"contract": "collect"` on `&mut Vec<MerchantOffer>` cleanly without editing generated files or adding a duplicate `trade-offer` type or new server-service ABI id.

## IDs and versions

* No server-service ABI IDs added (`crates/horizon-mod-api-gen/abi-subset-server.json` and `crates/yoake-wasm/wit/abi-1.0-baseline.wit` untouched; `abi_1_0_is_additive` stays additive).
* One new fragment hook `villager_trades` in `server-hooks` (`tools/native-hook-fragments/rem17d_trade.json`); generated WIT / adapter / SDK outputs (`yoake-full.wit`, `coverage.json`, `full_adapters.rs`, `full_server_generated.rs`, `full_hooks_generated.rs`, `full_hook_names.rs`) await regeneration by `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen`.

## WIT items (to be emitted on regeneration)

* `interface server-hooks`:
  * `villager-trades: func(entity: u32, kind: string, profession: string, level: s32, offers: list<merchant-offer>) -> list<merchant-offer>;` (reuses existing `engine-types` `merchant-offer`, `item-cost`, and `item-stack`).

## Files changed

* `tools/native-hook-fragments/rem17d_trade.json` (new)
* `crates/horizon-client/src/merchant.rs` (`MAX_MOD_OFFERS`, `MAX_OFFER_STACK`, `valid_offer_item_id`, `item_stack_limit`, `validate_offers_with`, `validate_offers`, unit test)
* `crates/horizon-client/src/mod_api.rs` (re-export `ItemCost`, host unit tests for no-op, edit/add/remove/reorder, and invalid-offer fallback)
* `crates/horizon-client/src/game/native_hooks.rs` (`Game::native_villager_trades`)
* `crates/horizon-client/src/game/trading.rs` (`Game::dispatch_villager_trades`, wired into `ensure_offers` and `merchant_apply` `Event::LevelUp`)

## Tests added (no Minecraft resources required)

* `crates/horizon-client/src/merchant.rs`:
  * `offer_validation_checks_items_counts_bounds_and_list_limit`
* `crates/horizon-client/src/mod_api.rs`:
  * `villager_trades_empty_and_noop_listeners_keep_offers`
  * `villager_trades_mod_may_edit_add_remove_and_reorder_offers`
  * `villager_trades_invalid_edits_fall_back_to_original_offers`

## What is left

* Run `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen` to regenerate committed WIT, coverage JSON, WASM adapters, and guest SDK files.
* Build & run tests (`horizon-mod-api-gen`, `yoake-wasm`, `yoake-wasm-sdk`, `horizon-client-lib`, `horizon-client --lib --bins --tests`) and run hook-API stamping if desired for `@API:...@` doc comments.
