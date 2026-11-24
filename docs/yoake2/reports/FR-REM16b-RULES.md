# REM-16b RULES: dedicated command tree and game-rule reload

## What

- The `ClientboundCommandsPacket` tree now adds every registered mod game rule below vanilla's `/gamerule` node. A boolean rule has a `brigadier:bool` `value` argument; an integer rule has `brigadier:integer` with its declared minimum and maximum. The nodes copy the vanilla rule nodes' permission requirement. With no mod rules, the report and emitted packet are unchanged.
- Mod rule registration is replaceable rather than write-once. Rebuilding the content root installs added and removed rules and changed defaults, bounds, and categories. Existing integer values are clamped on reads and serialization when bounds narrow. Vanilla rules remain in their original order and keep their behavior.
- Removed mod rules disappear from gameplay, the rules screen, command dispatch, and command completion. Their explicit namespaced values remain in `game_rules.dat`, including across save/load, so re-adding a rule restores its value.
- `/reload` rebuilds the mod content overlay before reloading world tables and resends command trees. The resource/mod reload path also resends them, including through the integrated-server thread when the reload originated in the client window.
- Newly constructed rules screens use the new registry and clamped values. Rule metadata is leaked as before so existing `GameRules` values and open screens never hold dangling references.

## Files

- `crates/horizon-client-core/src/gamerules.rs`: replaceable synchronized registry, bound clamping, removed-rule value persistence, and reload tests.
- `crates/horizon-client/src/lan/commands.rs`: report-tree injection and packet tests.
- `crates/horizon-client/src/game/mods.rs`: registration on every content-root build and `/reload` content-root rebuilding.
- `crates/horizon-client/src/game/functions.rs`: `/reload` integration and command resend.
- `crates/horizon-client/src/game/packs.rs`: command resend after resource/mod reload, including integrated-server handoff.

## Tests added (not run in this lane)

- `horizon-client-core::gamerules`: no-mod/vanilla list stability, registration, added rules, surviving values, narrowed-bound clamping, changed metadata, removed-rule hiding, save/load retention, and restoration on re-add.
- `horizon-client-lib::lan::commands`: identical no-mod packet, namespaced literals, bool argument, bounded integer argument bytes, permission restriction, and filtering below the required permission level.

## What is left / limits

- Changing an existing rule's value type is not value-preserving: while its new type differs from the saved value, the effective value is its new default. Defaults, bounds, and categories reload safely; a type migration still needs an explicit policy if mods require one.
- An already-open rules screen keeps its snapshot until reopened; it does not mutate rows underneath the player during reload.
- Reloads intentionally retain old leaked rule metadata. This is small but grows when declarations change repeatedly.
- Other write-once content registries retain their existing restart warnings; this work changes only game rules.
- No commands were run because this lane forbids shell use. Compilation, formatting, and tests remain for Claude.
