# FR-REM17d: custom command argument types, yak dependency resolution, villager trade-list event

Branch `yoake-free/rem17d` (from `yoake-free/merge` f28e4e511). Three lanes merged: `r17d-cmd` (agy), `r17d-yak` (Kimi), `r17d-trade` (agy); review, regeneration and tests by Claude on LENOVO (see "Tests (LENOVO)"). Nothing here was run in a game.

## 1. Custom command argument types (lane CMD)

### What

Added custom command argument types for `server-full` WASM mods:

1. **Registration (`server-services`, service ID 301)**: A server mod calls `register-command-argument-types(list<{ name: string, greedy: bool }>)` during `init` (enforced via `initializing_server(&self.id)`). Each type is validated (`1..=64` ASCII `[a-z0-9_-]`, capped at 64 types/mod and 512 total) and namespaced as `<modid>:<name>`.
2. **Usage Grammar**: Commands registered in `register_commands` may reference registered custom types as `<arg:modid:name>` (required) or `[arg:modid:name]` (optional).
   - Non-greedy (`greedy: false`) maps on the wire to `brigadier:string` (`{"type": "word"}`) and consumes one whitespace-delimited token.
   - Greedy (`greedy: true`) maps on the wire to `brigadier:string` (`{"type": "greedy"}`), must appear last (like built-in `text`), and consumes the remainder of the line.
   - All custom argument nodes emit `"suggestions": "minecraft:ask_server"` in the command tree JSON, and [`lan/commands.rs`](file:///C:/Users/suke0/YoakeBuild/wt-r17d-cmd/crates/horizon-client/src/lan/commands.rs) preserves `"minecraft:ask_server"` into `CommandNode.suggestions` so vanilla clients send `CommandSuggestion` packets with zero client-side protocol changes.
   - Existing usages without `<arg:modid:name>` remain untouched; vanilla/no-mod execution takes one relaxed atomic load fast-path (`ANY` / `HAS_ACTIVE_CUSTOM_COMMANDS`).
3. **Server-side Parse Before `perform-command`**: Before invoking `native_perform_command` / `native_perform_console_command`, if a registered usage with custom arguments matches the command line (respecting permission level and ranking literal overloads ahead of argument captures), the host calls the owning mod's `handle-call` export (`caller = "yoake:engine"`, `method = "yoake:command-argument/parse"`).
   - On `Ok(normalized)`: replaces the raw argument span with the normalized value in the command line passed to `perform-command` / `perform-console-command`.
   - On `Err(message)` (or unavailable/trapped parser): sends `Text::literal(message)` in red via `native_feedback` and skips `perform-command`.
4. **Tab-Complete Suggestions**: When `guest_suggestions` handles a `CommandSuggestion` packet matching a custom argument site at the cursor, the host calls the owning mod's `handle-call` (`method = "yoake:command-argument/suggest"`) with `(type_id, partial)`, bounds/sanitizes returned suggestions (max 64 entries, max 128 bytes each, single-line, deduplicated), merges them via `horizon_client::command::merge_suggestions`, and sends the normal `Clientbound::CommandSuggestions` packet.
5. **Unknown Types / Missing Mods**: Usages referencing unregistered or malformed `<modid>:<name>` types fail `usage_nodes_with` and are omitted from the command tree with the standard `"Horizon: a native mod's command was left out: ..."` warning log.

### IDs & Versions

- `crates/horizon-mod-api-gen/abi-subset-server.json`:
  - `interface_version`: `"1.8.0"` -> `"1.9.0"`
  - New service ID: `301` (`register_command_argument_types`, `side: "server"`, `capability: null`)

### WIT Items

Generated from `abi-subset-server.json` via `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen`:
- `server-types`: `record command-argument-type { name: string, greedy: bool }`
- `server-services`: `register-command-argument-types: func(types: list<command-argument-type>) -> result<_, string>;`
- Parse/Suggest callbacks use `server-full`'s existing `handle-call(caller: "yoake:engine", method, payload)` channel (`yoake:command-argument/parse`, `yoake:command-argument/suggest`), wrapped by `yoake_wasm_sdk::full::server::{register_command_argument_types, command_arguments::handle}` (compatible with `#![no_std]`).

### Files Changed / Added

- `crates/horizon-mod-api-gen/abi-subset-server.json` (version `1.9.0`, `command-argument-type` record, service ID `301`)
- `crates/yoake-wasm/src/full.rs` (`register_command_argument_types` in `server_services!` macro)
- `crates/horizon-client/src/wasm_mods/command_arguments.rs` (new: registry, validation, bounding, `handle-call` wire codecs & owner invocation, host unit tests)
- `crates/horizon-client/src/wasm_mods.rs` (`mod command_arguments;` and wrappers for `game/`)
- `crates/horizon-client/src/wasm_mods/server_services.rs` (`GameServerHost::register_command_argument_types`)
- `crates/horizon-client/src/wasm_mods/full.rs` (init-scope unit test for `register_command_argument_types`)
- `crates/yoake-wasm-sdk/src/full.rs` (`server::register_command_argument_types` and `server::command_arguments`)
- `crates/horizon-client/src/lan/commands.rs` (preserve `"minecraft:ask_server"` from mod tree node JSON)
- `crates/horizon-client/src/game/native_server.rs` (grammar support, `CustomCommandIndex`, pre-`perform_command` parse/rewrite/error, suggestion planner, unit tests)
- `crates/horizon-client/src/game/lan_actions.rs` (`guest_suggestions` calls `self.add_custom_command_suggestions`)

### Host Tests Added (No Minecraft Resources)

- `wasm_mods::command_arguments::tests::registration_namespaces_validates_and_enforces_limits`
- `wasm_mods::command_arguments::tests::wire_parse_and_suggest_responses_validate_and_bound`
- `wasm_mods::command_arguments::tests::parse_argument_fails_cleanly_for_unknown_or_missing_owner`
- `wasm_mods::full::tests::command_argument_type_registration_requires_owning_server_initialization`
- `game::native_server::tests::custom_command_argument_types_register_in_tree_and_reject_unknown_or_greedy_middle`
- `game::native_server::tests::custom_command_argument_parse_accepts_normalizes_and_refuses`
- `game::native_server::tests::custom_command_argument_suggestions_match_partial_and_greedy`

### What Is Left

- Regenerate committed WIT/adapter/SDK files (`YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen`) and run builds/tests in Claude's shell round.
- If dedicated `server-hooks` WIT methods (`parse-command-argument` / `suggest-command-argument`) are preferred over `handle-call` dispatch, `horizon-mod-api-gen`'s `wit_full.rs`/`wasm_adapters.rs` would need single-owner `result<string, string>` hook support added; using `handle-call` matches `behaviors.rs` (F4M-10) and `level_services.rs` (F3-8) without altering `mod_api::Hooks` or `horizon-mod-api-gen`.

## 2. `yak::resolve()` in package loading (lane YAK)

### What changed

- `yak::resolve()` now uses manifest discovery order to break ties between packages that are ready to load. A set with no dependencies therefore keeps its previous order instead of being sorted by id.
- Sandboxed YAK startup discovery resolves the combined local and host-synced package set after manifest/build checks. Compatible packages are reordered into dependency-first resolution order.
- Missing dependencies, version mismatches, conflicts, duplicates and cycles produce the existing startup warning and consumable mod toast. The packages directly involved are skipped; required dependents of a skipped package are skipped transitively. Healthy packages still load. Both members of a conflict are refused.
- Empty discovery returns before resolution, and an independent set retains the same packages and order as before.
- Install needed no new code: `launcher/src/yoake_mods/plan/resolver.rs` already calls `yak::resolve()` for candidate install sets, and `plan/transaction.rs` uses it when enabling, disabling and removing packages. Invalid installs are already refused through `Error::Resolution`.

### Ids, versions and WIT

No ABI ids, interface versions, WIT items, generated adapters or stubs changed. No regeneration is needed for this lane.

### Files

- `crates/horizon-mods/src/yak/compatibility.rs`: stable discovery-order topological resolution.
- `crates/horizon-client/src/yak_mods.rs`: startup resolution, selective refusal, notices and resolved load order.
- `crates/horizon-client/src/yak_mods/tests.rs`: host-side dependency fixtures and tests.
- `docs/yoake2/reports/FR-REM17d.md`: this report.
- `.lane/TASK.md`: findings and verification commands.

### Tests

Added host tests for a missing dependency with a message, an incompatible dependency version, dependency-before-dependent ordering, and healthy-package survival. The existing `list_comments_and_unsigned_content_preserve_load_order` test pins unchanged no-dependency order. Existing resolver tests cover conflicts, duplicates, cycles and absent optional dependencies.

Kimi did not run builds or tests because this lane forbids shell commands. Commands for Claude are in `.lane/TASK.md`.

### Left / boundaries

- Full-access packages remain on the separate approved-loader path in `yoake2_mods.rs`; the launcher intentionally splits them out before sandboxed resolution. This lane does not resolve dependencies among full-access packages or between full-access and sandboxed packages. Supporting that requires a follow-up that resolves the approved local and host-scoped full-access sets before any module executes.
- `GAP3*` was not present under `docs/` in this worktree; `GAP2.md` records dependency enforcement as unverified.

## 3. Villager trade-list event (lane TRADE)

### What changed

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

### Design notes (engine behaviour observed)

* **Wandering trader spawn**: `WanderingTraderSpawner::spawn_trader` (`crates/horizon-client/src/game/wandering_trader.rs`) spawns the mob with `extra.trade = None` (`unset = true`); offers are generated lazily in `Game::ensure_offers` on first interaction (`interact_trader`), matching vanilla `AbstractVillager.getOffers`.
* **Restock**: `Trader::restock` (`crates/horizon-client/src/merchant.rs`) only updates `demand`, resets `uses = 0`, and records `last_restock` / `restocks_today`; it does not regenerate offers, so it does not fire `villager_trades`.
* **WASM WIT record**: `merchant-offer` and `item-cost` already exist in `engine-types` (`crates/yoake-wasm/wit/yoake-full.wit`), and `horizon-mod-api-gen` maps `"contract": "collect"` on `&mut Vec<MerchantOffer>` cleanly without editing generated files or adding a duplicate `trade-offer` type or new server-service ABI id.

### IDs and versions

* No server-service ABI IDs added (`crates/horizon-mod-api-gen/abi-subset-server.json` and `crates/yoake-wasm/wit/abi-1.0-baseline.wit` untouched; `abi_1_0_is_additive` stays additive).
* One new fragment hook `villager_trades` in `server-hooks` (`tools/native-hook-fragments/rem17d_trade.json`); generated WIT / adapter / SDK outputs (`yoake-full.wit`, `coverage.json`, `full_adapters.rs`, `full_server_generated.rs`, `full_hooks_generated.rs`, `full_hook_names.rs`) await regeneration by `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen`.

### WIT items (to be emitted on regeneration)

* `interface server-hooks`:
  * `villager-trades: func(entity: u32, kind: string, profession: string, level: s32, offers: list<merchant-offer>) -> list<merchant-offer>;` (reuses existing `engine-types` `merchant-offer`, `item-cost`, and `item-stack`).

### Files changed

* `tools/native-hook-fragments/rem17d_trade.json` (new)
* `crates/horizon-client/src/merchant.rs` (`MAX_MOD_OFFERS`, `MAX_OFFER_STACK`, `valid_offer_item_id`, `item_stack_limit`, `validate_offers_with`, `validate_offers`, unit test)
* `crates/horizon-client/src/mod_api.rs` (re-export `ItemCost`, host unit tests for no-op, edit/add/remove/reorder, and invalid-offer fallback)
* `crates/horizon-client/src/game/native_hooks.rs` (`Game::native_villager_trades`)
* `crates/horizon-client/src/game/trading.rs` (`Game::dispatch_villager_trades`, wired into `ensure_offers` and `merchant_apply` `Event::LevelUp`)

### Tests added (no Minecraft resources required)

* `crates/horizon-client/src/merchant.rs`:
  * `offer_validation_checks_items_counts_bounds_and_list_limit`
* `crates/horizon-client/src/mod_api.rs`:
  * `villager_trades_empty_and_noop_listeners_keep_offers`
  * `villager_trades_mod_may_edit_add_remove_and_reorder_offers`
  * `villager_trades_invalid_edits_fall_back_to_original_offers`

### What is left

* Run `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen` to regenerate committed WIT, coverage JSON, WASM adapters, and guest SDK files.
* Build & run tests (`horizon-mod-api-gen`, `yoake-wasm`, `yoake-wasm-sdk`, `horizon-client-lib`, `horizon-client --lib --bins --tests`) and run hook-API stamping if desired for `@API:...@` doc comments.
