# FR-L1: IME for mod screens (sandboxed `client-full`)

Branch `yoake-free/l1-ime`, base `yoake-free/merge` (862d10c89). Nothing here was run in a real game window.

## What was added

- **Hooks** (fragment `tools/native-hook-fragments/ime.json`, client side, non-cancellable listen hooks):
  - `ime_preedit(text: string, cursor: option<tuple<u32, u32>>)`: live composition; the cursor is a UTF-8 byte range.
    An empty text clears the composition. `Ime::Disabled` sends `("", None)`.
  - `ime_commit(text: string)`: committed text, delivered once, before the existing per-character
    `screen_char_typed_pre` hooks (which still fire).
- **Client services** (`crates/horizon-mod-api-gen/abi-subset-client.json`, capability `service.ime`):
  - 201 `ime_set_allowed(allowed: bool)`
  - 202 `ime_set_cursor_area(x, y, w, h)` in GUI-scaled coordinates; finite values, nonnegative size, else `Err`.
  - Both return `Err` unless the calling mod owns the currently open mod screen. Ids 203-206 are unused.
- **Catalog version**: client catalog `interface_version` 1.2.0 -> 1.3.0 (additive minor, same as ids 199/200).
- Not added to the frozen legacy DLL subset (`abi-subset.json`). No `[wasm] permissions` entry is needed for `service.ime`.

## Routing

- `WindowEvent::Ime::Preedit` -> `Game::ime_preedit`: on a mod screen, `native_ime_preedit` notifies subscribed mods;
  the engine's own preedit update stays.
- `Ime::Commit` -> `Game::ime_commit`: on a mod screen, `native_ime_commit` notifies subscribed mods once, then the
  existing `screen_text_typed` dispatch runs.
- `Ime::Disabled` -> `Game::ime_disabled`: clears the engine preedit and sends an empty preedit to mods on a mod screen.
- `ime_set_allowed` / `ime_set_cursor_area` store state on `Game` (`mod_ime_allowed: Option<bool>`,
  `mod_ime_area: Option<[f32; 4]>`). `flush_ime` converts the area to physical pixels with the HUD scale and only
  re-sends it when it changed. State is reset when a mod screen opens and closes.
- The hooks only enter mods that subscribed (catalog subscription masks). With no mod installed no extra work or
  allocation happens: the notifications are skipped when no mod screen is open.

## Behaviour-preservation rule (fix round)

A mod screen whose mod never called `ime_set_allowed` / `ime_set_cursor_area` behaves exactly as at 862d10c89:

- `text_field_focused`: title-menu mod screen -> `true`; in-game mod screen -> falls through to the ordinary checks.
- `flush_ime`: title-menu mod screen -> overlay hidden, no area; in-game mod screen -> vanilla overlay/area path.
- `ime_commit`: stops after the character hook only for a title-menu mod screen (as before).

Once the mod calls either service (`mod_ime_allowed.is_some() || mod_ime_area.is_some()`), the screen follows the mod:
`mod_ime_allowed` decides IME, the engine overlay is hidden, the mod's area (if any) positions the candidate window, and
`ime_commit` ends after the character hooks. Implemented as pure functions in `game/ime.rs`
(`mod_ime_used`, `mod_text_field_decision`, `flush_mode`, `commit_stops_at_mod_screen`). Small deviation: opening and
closing a mod screen now also clears a stale engine preedit (`set_preedit(None)`).

## Files

- `tools/native-hook-fragments/ime.json`; `crates/horizon-client/src/game/ime.rs`, `game.rs`, `game/native_hooks.rs`,
  `game/native_title_screen.rs`; `crates/horizon-client/src/wasm_mods/client_services.rs`.
- `crates/horizon-mod-api-gen/abi-subset-client.json`; `crates/yoake-wasm/src/client_services.rs`,
  `crates/yoake-wasm/tests/client_services.rs`; `crates/yoake-inject-rt/src/catalog.rs`.
- Generated (do not hand-edit): `yoake-full.wit`, `coverage.json`, `full_adapters.rs`, `full_adapters_report.txt`,
  `full_hook_names.rs`, `crates/yoake-wasm-sdk/src/full_*_generated.rs`, the inject WIT files.
- SDK: `crates/yoake-wasm-sdk/src/full.rs` (wrappers), examples `ime/` and `client-services-demo/`.
- Docs: `docs/guide-ja/10-ime.md`, `docs/guide-ja/04-examples.md`, `docs/guide-en/04-examples.md`, README links,
  `docs/yoake2/MOD-GUIDE.md` section 14, `docs/yoake-wasm-dev.md`.

## Tests

- `horizon-client` unit tests in `game/ime.rs` (no window): owner check, UTF-8 cursor conversion, area validation,
  physical area scaling, and the old-behaviour decision functions listed above.
- `crates/yoake-wasm/tests/client_services.rs`: the services reach the host with the right argument order.
- Generator tests (`horizon-mod-api-gen`) cover the catalog, WIT and adapters.
- SDK guest example `examples/ime` (compile test: `cargo build --release --target wasm32-wasip2 ...`).
- Last coordinator results before this fix round: mod-api-gen 45/45, yoake-wasm lib+tests pass, horizon-client --lib 1200.
  The fix round's code is not yet built.

## Argument-order note

The generator sorts service arguments alphabetically, so the raw WIT import is `ime-set-cursor-area(h, w, x, y)`.
Mod authors must use the wrapper `yoake_wasm_sdk::full::client::ime_set_cursor_area(x, y, w, h)` (it reorders). The
docs (10-ime.md, MOD-GUIDE.md) say so. The service calls need write host access: they fail in `screen_init_post`
(read-only), so the example calls them from `frame` and retries until they succeed.

## Needs checking (real window, on Yoake's PC)

1. Japanese composition on a mod screen shows the live preedit through `ime_preedit` (text, byte cursor range).
2. The candidate window appears at the given area, at several GUI scales and window sizes.
3. Commit text arrives once through `ime_commit` and also as `screen_char_typed_pre` characters, in that order.
4. An old mod screen that never calls the services still gets IME exactly as before (title-menu one on, in-game one off).
5. Vanilla screens (chat, sign, book, title fields, options search) are unchanged.
6. `Ime::Disabled` clears the mod's composition (empty preedit) and the engine overlay.
7. Opening, closing and reopening a mod screen resets IME state; the example re-applies it via `frame`.

## Open questions

- `frame` retry from `screen_init_post` marking works only if `frame` is delivered while the mod screen is open; not verified.
- Guide numbering: another branch adds `docs/guide-ja/09-sprites.md`, so `10-ime.md` (and links to it) may need
  renumbering at merge.
- Whether a mod that calls only `ime_set_cursor_area` (never `ime_set_allowed`) should count as "used" was decided
  yes (hides the engine overlay, applies the area, keeps the old allow rule); confirm with Yoake.

## Yoake 2.0 full-access route: not done

The fn hooks of full-access core modules come from the separate `hook-ledger` / codemod modules, not from this catalog.
Adding IME there needs, and none of it was half-wired:

1. Ledger entries (new hook ids) for `ime_preedit` / `ime_commit` and for the two services.
2. Generated bridge coverage for them (`crates/yoake-bindgen`, ENGINE_CALL_ABI.md).
3. Engine-call bindings on the host side for `ime_set_allowed` / `ime_set_cursor_area`.
4. Guest wire types for a string and an `option<(u32, u32)>` hook argument, which existing fn hooks do not carry.
5. `MOD-GUIDE.ja.md` plus tests for the new ids.
