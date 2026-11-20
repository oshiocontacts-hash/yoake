# Cinder Fox fixture (F4M)

A Yoake package folder (`yak.toml`) with the Cinder Fox custom mob and the guest that scripts its `pounce` goal.

| Path | What |
|---|---|
| `content/` | The mod's data and assets (`horizon-mod.toml`, `data/`, `assets/`): a **content mod** and the **data-only variant** of the fixture. Load it alone with `--dev-mod tools/fixtures/cinder_fox/content`; most mob tests read it from here. |
| `yak.toml` | The package manifest: `[wasm] interface = "1.0"`, `server = "bin/wasm/fox_ai.wasm"`, `permissions = ["service.mobs"]` (the same keys `yoake new` writes; `yak.toml` is the only manifest with `[wasm]`, a content mod's `horizon-mod.toml` has none). `content/` rides inside the package, its id and version must equal the package's. |
| `bin/wasm/fox_ai.wasm` | Not committed (git-ignored). The guest source is `crates/yoake-wasm-sdk/examples/fox_ai`; the tests build it with the `yoake-wasm` `test-guests` pipeline (`yoake_wasm::test_guests::FOX_AI`, `wasm32-wasip2`, release) and write it here in a scratch copy of the package, never into the repository. |

The in-game script goal test (`wasm_mods::mob_runtime`, features `yoake-hook-bench,yoake-wasm/test-guests`) stages `yak.toml` + `content/` + the built guest as an installed package
(`yoake-mods/sample_mobs/0.1.0/`, listed in `yoake-mods.txt`), reads it through `yak_mods::Runtime` and loads the guest the way the game does.

To try it by hand: build the guest (`cargo build --release --target wasm32-wasip2` in the example folder), copy it to `bin/wasm/fox_ai.wasm`, and install the folder as a package (`yoake-mods.txt` line `sample_mobs 0.1.0`; an unsigned
package needs `YOAKE_WASM_UNSIGNED=1`).
