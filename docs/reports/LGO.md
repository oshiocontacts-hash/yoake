# LGO Report — Launcher Game Options (`options.txt` pre-launch configuration)

## Summary
Added pre-launch Minecraft `options.txt` configuration to the launcher with a global **"Usual settings" (`いつもの設定`)** profile and per-instance **"Game settings" (`ゲーム設定`)** overrides, supporting **Every launch (`Always`)** and **First time only (`Once`)** modes across both Horizon/Yoake and Minecraft: Java Edition instances.

## Files changed / added

### Added
- `launcher/src/game_options.rs` — Data model (`ApplyMode`, `OptionEntry`, `GlobalProfile`, `InstanceConfig`), storage (`<data_dir>/game-options.json`), resolution (`resolve_entries`), `options.txt` patching (`plan_and_apply`, `apply_entries_to_text`, `apply_for_instance`), vanilla codecs/validation (`double_to_string`, sound categories, `fov`, `renderDistance`, `simulationDistance`, `guiScale`, `gamma`, `maxFps`, `mouseSensitivity`, `bobView`, `lang`), and `options.txt` reader (`read_options_txt`, `load_profile_from_options_txt`).
- `launcher/src/game_options/tests.rs` — Unit tests for vanilla `options.txt` string encodings/validation, `Always` vs `Once` apply behavior, key-by-key override, `use_usual` switch, missing `options.txt` creation, unknown key & line-order preservation, storage round-trip, and loading from `options.txt`.
- `launcher/src/game_options_page.rs` — Glass UI editor shared between the instance page **"Game settings" (`ゲーム設定`)** tab and the launcher Settings **"Usual settings" (`いつもの設定`)** section (sliders, steppers, toggles, language picker, mode chips, remove buttons, inherited preview with Override button, "Load this instance's current settings", "Copy to usual settings", and "Other options" key search + raw entry form).
- `docs/yoake-game-options.md` — Player guide in Japanese and English.
- `docs/reports/LGO.md` — This report.

### Modified
- `launcher/src/lib.rs` — Exported `pub mod game_options;`.
- `launcher/src/instances.rs` — Added `game_options: InstanceConfig` to `Meta` and `Instance` (serialized in `instances.json`, skipped when default).
- `launcher/src/backend.rs` — Applied `game_options::apply_for_instance` in `java_game` before launching Java Edition instances (errors logged, never fatal).
- `launcher/src/cli.rs` — Applied `game_options::apply_for_instance` in the Horizon/Yoake CLI launch path (errors logged, never fatal).
- `launcher/src/mcp.rs` — Applied `game_options::apply_for_instance` in the MCP `play` tool launch path (errors logged, never fatal).
- `launcher/src/main.rs` — Wired `game_options_page` into `Launcher`, applied `game_options::apply_for_instance` in GUI Horizon/Yoake launch (`Msg::Session`), reloaded instances on `Msg::JavaGame`, refreshed `options.txt` scan on client exit, added `usual_settings_section` under Settings → Game, and added `HORIZON_LAUNCHER_PAGE=usual-settings` scroll support for screenshots.
- `launcher/src/home_tabs.rs` & `launcher/src/glass_home_tabs.rs` — Added `HomeTab::GameSettings` (`ゲーム設定 / Game settings`, `HORIZON_LAUNCHER_PAGE=game-settings`).
- `launcher/src/import/tests.rs` — Initialized `game_options: Default::default()` in test `Instance` literal.
- `launcher/lang/*.json` (all 16 translation files) — Added translations in `ja.json` and fallback entries in the remaining 15 tables for all 53 new UI strings.

## Tests and results

- `.\hz-cargo.cmd test --manifest-path launcher/Cargo.toml -j 6 -p horizon-launcher game_options` (after final code edit)
  - **Result**: 5 passed, 0 failed (`horizon_launcher` lib tests).
- `.\hz-cargo.cmd test --manifest-path launcher/Cargo.toml -j 6 --bin horizon-launcher` (after final code edit)
  - **Result**: 107 passed, 0 failed, 1 ignored (`cull::desktop_tests::desktop_list_renders_the_viewport_after_scrollbar_jumps`).
- `.\hz-cargo.cmd test --manifest-path launcher/Cargo.toml -j 6 --lib instances` (after final code edit)
  - **Result**: 20 passed, 0 failed.
- `.\hz-cargo.cmd test --manifest-path launcher/Cargo.toml -j 6` (full launcher lib test suite prior to the slider-thumb / scroll-offset UI polish)
  - **Result**: 578 passed, 6 ignored, 3 pre-existing failures unrelated to this task (`backend::tests::platform_assets_are_published`, `i18n::tests::every_language_has_every_string_the_code_asks_for` reporting 2 pre-existing missing keys in `lang/fr.json`, and `yoake_feed::tests::accepts_a_python_signed_presentation_bound_to_its_index`). All 53 new keys added by this task passed `i18n::tests`.
- `.\hz-cargo.cmd build --manifest-path launcher/Cargo.toml -j 6`
  - **Result**: Succeeded (exit 0).

## Screenshots

Captured off-screen via `HORIZON_LAUNCHER_SCREENSHOT` (no focus stolen, isolated `HORIZON_HOME` / `HORIZON_LAUNCHER_HOME` under scratch):
- `target/screenshots/game-settings-ja.png` (1618×1102)
- `target/screenshots/game-settings-en.png` (1618×1102)
- `target/screenshots/usual-settings-ja.png` (1618×1102)
- `target/screenshots/usual-settings-en.png` (1618×1102)

## What is left / Notes
- End-to-end launch with a real Minecraft / Yoake binary was not executed (per task rules, do not start the game or launcher window interactively); pre-launch `options.txt` application is covered by unit tests and wired into all GUI, CLI, MCP, and Java launch paths.
- A final full test verification (`game_options`, `i18n`, `--bin horizon-launcher`, and `cargo check` / `build`) should be recorded after the final report & docs edits if any code changed, or confirmed since docs-only edits do not alter compiled targets.
