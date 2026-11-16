# 3. Playing and Quick-Testing in the Game

Once your mod is built, it's time to see it in action inside Yoake.
There are two ways to run your mod: **quick-testing a `client-mod` WIT world mod from a development folder**, or **installing any mod as a `.yak` package**.

---

## 3-1. Quick-Testing a `client-mod` Mod from a Development Folder

If your mod's WIT world is `client-mod`, you don't need to repack and reinstall a `.yak` file after every edit. Instead, the game can load raw `.wasm` files directly from a development folder and hot-reload them on the fly. The development folder does not read manifests, and every module in it is always treated as `client-mod`.[^dev-dir]

Modules loaded from the development folder get a `dev-` prefix on their `id`, and a module is skipped if an installed mod already has the same `id` (their settings and storage stay separate). Because the bytes of a hot-reloaded module can change at any time, it does not pin a `required_on_join` digest.

> [!WARNING]
> Developer mode has **two separate switches**. Use the Wasm one for everything in this guide.[^unsigned]
>
> - **Wasm developer mode** (the environment variable `YOAKE_WASM_UNSIGNED=1`, or the launcher's "Test your own WASM mods"): lets the game load unsigned `.yak` packages that contain only Wasm, enables loose `<id>.wasm` files from `YOAKE_WASM_DEV_DIR`, and turns on hot reload. It does **not** allow native DLL code. Wasm still runs in the sandbox, but the service permissions that your mod declares (such as `service.commands` on a server module) still apply. Never use it for mods you got from someone else.
> - **DLL developer mode** (the environment variable `YOAKE_DLL_MODS_UNSIGNED=1`, or the launcher's "Test your own DLL mods (developer mode)"): this is a superset of the Wasm switch. It enables the Wasm development features **and also allows unsigned native DLL mods, which have full, unrestricted access to everything on your PC**. Use it only when you are working on a native DLL mod; you do not need it for this guide.
>
> Either way:
>
> - **Never set it permanently.** Never use `setx`, and never add it to your Windows system or user environment variables.
> - **Recommended**: Use the launcher's Settings → "Developer options" (revealed by clicking "Launcher x.y.z" in the sidebar 7 times) → "Test your own WASM mods", then confirm with "Turn on". That toggle applies only to games started by the launcher.[^launcher]
> - If you use the environment variable directly, set `$env:YOAKE_WASM_UNSIGNED='1'` in **that single PowerShell window only**, and **launch the game executable directly from that same window**. When a launcher toggle is off, the launcher strips the matching variable (`YOAKE_WASM_UNSIGNED` or `YOAKE_DLL_MODS_UNSIGNED`) from the game's startup environment.[^launcher]
> - While developer mode is active, install **only `.yak` files that you built yourself**. Close the PowerShell window as soon as you finish testing.

### Setting the Development Folder and Running `yoake dev`

When launching the game executable directly without going through the launcher, follow the safety warning above to set `YOAKE_WASM_UNSIGNED` in PowerShell, then point `YOAKE_WASM_DEV_DIR` at your development folder in the same window:

```powershell
$env:YOAKE_WASM_DEV_DIR='<folder-path>'
```

When you **launch the game directly from that same PowerShell window**, it will load `<id>.wasm` from the specified folder without needing a `.yak` package. This PowerShell setup cannot be used when starting the game through the launcher while the launcher's Wasm developer-mode toggle is off, because the launcher removes `YOAKE_WASM_UNSIGNED` (and `YOAKE_DLL_MODS_UNSIGNED`). In your mod's project folder, run:

```sh
yoake dev
```

`yoake dev` builds and checks your mod, and if its WIT world is `client-mod` and `YOAKE_WASM_DEV_DIR` is set, copies the built `.wasm` binary into that folder.[^cli]

For `client-full` and `server-full` mods, `yoake dev` does not copy the file; instead, it succeeds with `ok:true`, `installed:false` and prints the following note. This is not an error—use the `.yak` installation steps in the next section for those mods.

```text
raw Wasm hot reload only supports client-mod; not installed
```

### Hot Reload (Updating While the Game Is Running)

Once per second, the game checks whether any `.wasm` files discovered at startup have been updated.

- **Once a file stops changing for 1 full second after being written**, the game automatically swaps in a fresh instance on the next frame or tick.[^watch]
- If the new `.wasm` fails to load, the previous working instance keeps running and a warning is written to the log.
- **Files in the development folder are discovered only when the game starts.** If you add a brand-new `.wasm` file after launching the game, restart the game once so it starts watching that file.
- Hot reload only supports modules whose WIT world is `client-mod`. `client-full` and `server-full` modules are not hot-reloaded.

---

## 3-2. Installing a `.yak` Package

Running `yoake pack` outputs your package to `dist/<id>-<version>.yak` inside your project folder:

```sh
yoake pack
```

### Steps for the New Examples and Full-Catalog (`client-full` / `server-full`) Mods

Because `hud-item`, `server-storage`, `config-demo`, `permission-demo`, `mod-call-pair` (`caller` and `responder`), `chest-count`, `item-trade`, and `player-vitals` are all full-catalog (`client-full` / `server-full`) mods, they cannot be loaded from the raw development folder. Always install them using these steps:

1. Run `yoake pack` in the mod's project folder.
2. Review the safety warning above, then enable Settings → "Developer options" (revealed by clicking "Launcher x.y.z" in the sidebar 7 times) → "Test your own WASM mods" in the launcher, and confirm with "Turn on".
3. On the launcher's Mods page, choose and import your local `.yak` file.[^local-yak]
4. Restart the game.

### How the Game Loads Installed Packages

Package extraction and the active mod list work as follows:[^package]

- The **launcher** extracts each `.yak` into `<game dir>/yoake-mods/<id>/<version>/`, keeps the package file in `yoake-mods/.cache/`, and writes `<game dir>/yoake-mods.txt` (one `<id> <version>` entry per line). The game itself loads the extracted folders.
- A dedicated server (`--dedicated`) also loads server modules (WIT world `server-full`) from the packages listed in `yoake-mods.txt` inside its `--game-dir`.
- Unsigned `.yak` packages that you built yourself require Wasm developer mode. While Wasm developer mode is enabled, unsigned `client-mod` modules inside installed `.yak` packages are also watched for hot reload. A `.yak` that carries a `SIGNATURE` must still pass signature verification, even in developer mode.

---

## 3-3. Verifying That Your Mod Loaded and Reading the Logs

When the game starts, it writes one line to the startup log indicating whether the WebAssembly engine is running:

- When no mods are present: `wasm: no mods (engine not started)`
- When mods are loaded: `wasm: N mod(s), engine started` (where `N` is the number of loaded mods)

Log messages printed by your mod (via `yoake_wasm_sdk::log(...)`, or `println!` / `eprintln!` when the standard library is enabled) also appear in the game log. If your HUD doesn't show up or a command doesn't respond, check the startup log first.

---

## Next Steps

Now that you know how to run mods in the game, head to [4. Exploring the Example Mods (`04-examples.md`)](./04-examples.md) to see concrete code examples for drawing item icons, adding slash commands, saving data, building settings screens, and checking permission nodes.

[^dev-dir]: Source: `dev_sources_with` in `crates/horizon-client/src/wasm_mods.rs`.
[^unsigned]: Source: `UnsignedSwitches` in `crates/horizon-client/src/yak_mods.rs`, `docs/api/yoake-mods.md`, and "Developer mode: hot reload" in `docs/yoake-wasm-dev.md`.
[^launcher]: Source: `set_unsigned_dll_env` / `set_unsigned_wasm_env` / `client_command_for_program` in `launcher/src/backend.rs`, and `launcher/src/dev_options.rs`.
[^cli]: Source: `dev_project` / `install_for_reload` in `crates/yoake-cli/src/main.rs`.
[^watch]: Source: `Watch::poll` / `spawn_watcher` in `crates/horizon-client/src/wasm_mods.rs`.
[^local-yak]: Source: `add_local_yak` / `choose_local_yak` in `launcher/src/yoake_mods_page.rs`.
[^package]: Source: `docs/api/yoake-mods.md` and `docs/yoake-wasm-dev.md`.
