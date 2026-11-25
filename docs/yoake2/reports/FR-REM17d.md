# FR-REM17d: Brigadier-style custom command argument types (Lane CMD)

## What

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

## IDs & Versions

- `crates/horizon-mod-api-gen/abi-subset-server.json`:
  - `interface_version`: `"1.8.0"` -> `"1.9.0"`
  - New service ID: `301` (`register_command_argument_types`, `side: "server"`, `capability: null`)

## WIT Items

Generated from `abi-subset-server.json` via `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen`:
- `server-types`: `record command-argument-type { name: string, greedy: bool }`
- `server-services`: `register-command-argument-types: func(types: list<command-argument-type>) -> result<_, string>;`
- Parse/Suggest callbacks use `server-full`'s existing `handle-call(caller: "yoake:engine", method, payload)` channel (`yoake:command-argument/parse`, `yoake:command-argument/suggest`), wrapped by `yoake_wasm_sdk::full::server::{register_command_argument_types, command_arguments::handle}` (compatible with `#![no_std]`).

## Files Changed / Added

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

## Host Tests Added (No Minecraft Resources)

- `wasm_mods::command_arguments::tests::registration_namespaces_validates_and_enforces_limits`
- `wasm_mods::command_arguments::tests::wire_parse_and_suggest_responses_validate_and_bound`
- `wasm_mods::command_arguments::tests::parse_argument_fails_cleanly_for_unknown_or_missing_owner`
- `wasm_mods::full::tests::command_argument_type_registration_requires_owning_server_initialization`
- `game::native_server::tests::custom_command_argument_types_register_in_tree_and_reject_unknown_or_greedy_middle`
- `game::native_server::tests::custom_command_argument_parse_accepts_normalizes_and_refuses`
- `game::native_server::tests::custom_command_argument_suggestions_match_partial_and_greedy`

## What Is Left

- Regenerate committed WIT/adapter/SDK files (`YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen`) and run builds/tests in Claude's shell round.
- If dedicated `server-hooks` WIT methods (`parse-command-argument` / `suggest-command-argument`) are preferred over `handle-call` dispatch, `horizon-mod-api-gen`'s `wit_full.rs`/`wasm_adapters.rs` would need single-owner `result<string, string>` hook support added; using `handle-call` matches `behaviors.rs` (F4M-10) and `level_services.rs` (F3-8) without altering `mod_api::Hooks` or `horizon-mod-api-gen`.
