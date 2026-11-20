# Yoake WebAssembly Mod Beginner's Guide

This guide is a beginner's introduction to building mods for **Yoake**.
If you have even a little programming experience, you can follow along step by step—even if this is your very first time using **Rust** or **WebAssembly (Wasm)**.

---

## What Is a WebAssembly Mod?

A Yoake mod is written in Rust, compiled to a flavor of WebAssembly called **`wasm32-wasip2`**, and bundled together with a manifest file (`yak.toml`) into a `.yak` package for distribution and installation.

Wasm mods run inside a safe sandbox within the game. Even if a mod hits an infinite loop or runs out of time, an error or a timeout stops only that mod.

This guide builds every mod as a `.yak` package that contains Wasm. Native DLL mods still exist too, but they are an advanced path for the owner and developers and are not covered here.

---

## What You'll Build in This Guide

Through this guide and the included examples, you'll learn how to build mods that do the following:

1. **Customize the Screen (HUD)**
   - Draw translucent panels, text, and item icons (such as a diamond) anywhere on the game screen
   - Hide built-in HUD elements like the hotbar, add extra lines to item tooltips, and change the field of view (FOV)
2. **Add Custom Slash Commands and Save Data**
   - Register custom commands like `/visits`, persist per-world and per-player records, and reply in chat
3. **Hook Into the In-Game Settings Screen (Configure)**
   - Define configuration options (strings, dropdown choices, and more) under `[config]` in `yak.toml`, and immediately apply changes made from the **Configure** button in the mod list
4. **Check Permission Nodes**
   - Declare permission nodes under `[permissions]` in `yak.toml` and allow or deny commands like `/secret` based on operator (OP) level or a world's `permissions.toml`
5. **Test, Publish, and Intercept Engine Functions**
   - Run automated tests inside the real game engine, learn how signing and the launcher's catalog work, and hook into engine functions with Mixin

---

## Table of Contents and Reading Order

Read the chapters in order from top to bottom for a complete walkthrough—from setting up your tools to building, testing, publishing, and expanding your own mod.

### 1. [Setting Up Your Environment (`01-setup.md`)](./01-setup.md)
- Installing the Rust toolchain (`rustup`)
- Adding the WebAssembly build target (`rustup target add wasm32-wasip2`)
- Installing the `yoake` CLI
- Choosing an editor and preparing a workspace folder (focused on Windows, with notes for macOS and Linux)

### 2. [Creating Your First Mod (`02-first-mod.md`)](./02-first-mod.md)
- Scaffolding a project with `yoake new hud-only my-hud` (including a list of all 5 templates)
- What each generated file does (`yak.toml`, `Cargo.toml`, `src/lib.rs`, `README.md`, `.yoake/`, etc.), including the `[features] tests` setting that the template includes
- The `yoake build` → `yoake check` → `yoake pack` workflow and checking the output at `dist/<id>-<version>.yak`

### 3. [Playing and Quick-Testing in the Game (`03-play.md`)](./03-play.md)
- How importing `.yak` packages and game loading work
- Hot-reloading mods whose WIT world is `client-mod` using `YOAKE_WASM_DEV_DIR` and `yoake dev`
- The two developer-mode switches (one for Wasm, one for DLL) and the safety precautions that go with them

### 4. [Exploring the Example Mods (`04-examples.md`)](./04-examples.md)
- Walkthrough of the 19 examples in the repository, including `hello-std` (`hello`, `hello-std`, `hud-item`, `server-storage`, `config-demo`, `permission-demo`, `full-demo`, `full-services`, `mod-call-pair`, `chest-count`, `item-trade`, `player-vitals`, `content-demo`, `mixin-demo`, `test-demo`, `tested-counter`, `client-services-demo`, `gpu-demo`, and `ime`)
- Overview of the hooks (`hook`) and host services (`services`) each example uses
- How to write `[config]`, `[wasm] permissions`, and `[permissions."<node>"]` in `yak.toml`

### 5. [Common Errors and How to Fix Them (`05-errors.md`)](./05-errors.md)
- Typos in hook names and mixing up client vs. server hooks
- Forgetting to rebuild `.wasm` (stale errors) and Windows reserved-name errors
- Errors when `YOAKE_WASM_DEV_DIR` is unset or `wasm32-wasip2` hasn't been installed
- Common `yak.toml` validation errors and how to fix them

### 6. [Testing Your Mod Inside the Game Engine (`06-testing.md`)](./06-testing.md)
- Running automated tests inside the real game engine with `yoake test` and `horizon-client --yoake-test`
- Complete `TestWorld` and `TestDriver` reference, plus superflat world rules
- Two-client multiplayer tests (`--clients 2` and `ClientView`)
- Common test failures and unimplemented features

### 7. [Publishing Your Mod (`07-publishing.md`)](./07-publishing.md)
- Versioning, what goes inside a `.yak` package, and its size limits
- How signing works, why players need a package signed with Yoake's key, and what "unsigned" means
- Getting into the launcher's catalog, publishing updates, and what you must never do

### 8. [Getting Started with Yoake Mixin (`08-mixin.md`)](./08-mixin.md)
- Intercepting engine functions with the `inject-mod` WIT world, injection points (HEAD, RETURN, and overwrite), and the site catalog
- Value ranges, command and chat interception, ordering, resource limits, and traps

### 11. [Screen, inventory, sound and food services (`11-small-services.md`)](./11-small-services.md)
- Sending from screen input hooks, turned textures, reading inventories
- Client-only sound, setting food, block entity write-back, consuming a key

### 13. [Dimensions, levels and chunk generators (`13-dimensions.md`)](./13-dimensions.md)
- Adding dimensions with data-pack JSON (`dimension` and `dimension_type`), save folders, and validation errors
- Generators (`noise`, `flat`, `debug`, `yoake:mod`), biome sources, and per-level seeds
- Travel (`/execute in`, `/tp`, portal coordinate scale) and the reconnect rule for runtime-created levels
- WASM mod level registration, runtime lifecycle, and chunk generators (`level-generator` example)

---

## Recommended Ways to Read

- **If you want to get something running right away**: Follow [`01-setup.md`](./01-setup.md) → [`02-first-mod.md`](./02-first-mod.md) → [`03-play.md`](./03-play.md) in order to draw your first text on screen, and continue to [`06-testing.md`](./06-testing.md) to test world scenarios in the game engine.
- **If you already know what feature you want to build**: After trying the basics once, jump to [`04-examples.md`](./04-examples.md) to adapt the closest example, and use [`06-testing.md`](./06-testing.md) to verify server-side behavior automatically.
- **If you want to publish your mod**: Read [`07-publishing.md`](./07-publishing.md) for versioning, signing, and the catalog.
- **If you want to hook into engine functions themselves**: After the basics, read [`08-mixin.md`](./08-mixin.md).
- **When you run into an error**: Check the message printed by the command or in the game log, and look it up in [`05-errors.md`](./05-errors.md) (or Section 6-6 of [`06-testing.md`](./06-testing.md) for test runner errors) like a reference dictionary.
