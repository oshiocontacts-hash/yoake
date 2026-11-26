# Task F5-GUIDES Report (REM-20, Docs)

Branch: `yoake-free/f5-sdk` (base `a6206a7c50`)

## 1. Files Changed / Added

### Added
- `docs/guide-en/09-sprites.md` — Chapter 9: Sprites & Textures (`full::client::sprites` / `sprite-demo`, HUD/world overlay sprites).
- `docs/guide-en/10-ime.md` — Chapter 10: Japanese Input / IME (`IME_PREEDIT`, `IME_COMMIT`, F-route `f_services`).
- `docs/guide-en/12-files-network.md` — Chapter 12: Files, Network, and Workers (`yoake new worker`, `yoake_wasm_sdk::worker`, `f2-io`, `f2-check`, `f1-f2-demo`).
- `docs/guide-ja/14-custom-mobs.md` — 第14章: カスタムMob (F4M: `content/` layout, `.bbmodel` / `.geo.json` / `.animation.json` / controllers, `ScriptGoal` + `server-full` `service.mobs`, `custom-mob` template, `fox_texture` / `fox_ai` / `seat_fox`, `seats`).
- `docs/guide-en/14-custom-mobs.md` — Chapter 14: Custom Mobs (English counterpart of `docs/guide-ja/14-custom-mobs.md`).
- `docs/guide-ja/15-browser-media.md` — 第15章: ブラウザ・メディア連携 (MEDIA1: `service.browser`, `browser-demo`, safety rules & limits).
- `docs/guide-en/15-browser-media.md` — Chapter 15: Browser & Media Integration (English counterpart of `docs/guide-ja/15-browser-media.md`).
- `docs/yoake2/reports/FR-F5-GUIDES.md` — This report.

### Updated
- `docs/guide-ja/README.md` & `docs/guide-en/README.md` — Updated to the 9 `yoake new` templates, 30 packaged examples, and TOC links covering chapters 9, 10, 12, 14, and 15.
- `docs/guide-ja/02-first-mod.md` & `docs/guide-en/02-first-mod.md` — Updated for the 9 templates (`minigame`, `hud-only`, `custom-mob`, `utility`, `script`, `client-full`, `server-full`, `worker`, `full-access`), `interface = "1.7"`, `[workspace]` exclude list, and WIT worlds.
- `docs/guide-ja/04-examples.md` & `docs/guide-en/04-examples.md` — Updated to all 30 packaged examples (plus test fixtures) with one-line descriptions and the `[wasm] permissions` reference distinguishing active vs legacy backward-compat capabilities.
- `docs/guide-ja/09-sprites.md` — Added `full::client::sprites` wrapper and `sprite-demo` reference.
- `docs/guide-ja/10-ime.md` — Fixed stale `# 9.` / `## 9-*` headings to `# 10.` / `## 10-*` and noted F-route `f_services`.
- `docs/guide-ja/12-files-network.md` — Added `yoake new worker`, `yoake_wasm_sdk::worker`, `f1-f2-demo`, `f2-check`, and updated chapter link.
- `docs/yoake2/MOD-GUIDE.md` — Updated for `yoake new full-access` / `worker`, `yoake build` / `check` / `pack`, `interface = "1.7"`, ABI `1.1` (`Handle::arg` / `arg_value` / `set_arg_value` / `swap_args` / `replace_arg`), `stubs::*`, `hooks::GPU_FRAME` / `IME_PREEDIT` / `IME_COMMIT` + `f_services`, F4M `custom_mobs` (Route F + `fox_ai_full.rs`), and `f1-f2-demo`.
- `docs/yoake2/MOD-GUIDE.ja.md` — Brought to parity with `MOD-GUIDE.md` (added Section 14 IME, renumbered Files/Workers to Section 15, added Section 16 Levels F3-8b, plus ABI 1.1, F1 `f_services`, F4M `custom_mobs`, `yoake new full-access` / `worker`).
- `docs/mods.md` — Rewrote the `Mob types` section for F4M (`"template": "yoake:mob"`) vs legacy template reskins.
- `docs/yoake-wasm-dev.md` — Updated for `yoake:mods@1.7.0` (`SUPPORTED_WASM_INTERFACE = (1, 7)` and `wit-mod-1.0` compat), `yoake:mods/worker@1.7.0`, the 9 templates, `yoake build` / `check` / `pack`, current `PackageServices::require` capability rules, code-verified trust rules (`yak_mods::wasm_preflight`, `YOAKE_WASM_UNSIGNED=1`, `SIGNATURE`, `--yoake-test`, `--remote-mods`, `ApprovedList`), and removed stale force-load / HTTP denial notes.

## 2. Verification & Link Check Results

- Link check (`python .local\check_links.py`): exit `0` — `OK: checked 207 relative links across 21 files; 0 broken.`
- Public tree check (`python -B tools\check_public_tree.py`): exit `0` — `Public tree check passed.`

## 3. What Is Left

- Production-team ZIP packaging script / workflow (`FR-F5-SDK.md` Section 5).
- Live-game client runtime E2E for the documented F1–F5 flows once sufficient disk space is available (not run in this docs-only task).
