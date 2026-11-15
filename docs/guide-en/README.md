# Yoake WebAssembly Mod Beginner's Guide

This guide is a beginner's introduction to building mods for **Yoake**.
If you have even a little programming experience, you can follow along step by step—even if this is your very first time using **Rust** or **WebAssembly (WASM)**.

---

## What Is a WebAssembly Mod?

A Yoake mod is written in Rust, compiled to a flavor of WebAssembly called **`wasm32-wasip2`**, and bundled together with a manifest file (`yak.toml`) into a `.yak` package for distribution and installation.

WebAssembly mods run inside a safe sandbox within the game. Even if a mod hits an infinite loop or runs out of time, only that mod is stopped—the game itself keeps running safely.

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
4. **Check Player Permissions**
   - Declare permission nodes under `[permissions]` in `yak.toml` and allow or deny commands like `/secret` based on operator (OP) level or a world's `permissions.toml`

---

## Table of Contents and Reading Order

Read the chapters in order from top to bottom for a complete walkthrough—from setting up your tools to building, testing, and expanding your own mod.

### 1. [Setting Up Your Environment (`01-setup.md`)](./01-setup.md)
- Installing the Rust toolchain (`rustup`)
- Adding the WebAssembly build target (`rustup target add wasm32-wasip2`)
- Choosing an editor and preparing a workspace folder (focused on Windows, with notes for macOS and Linux)

### 2. [Creating Your First Mod (`02-first-mod.md`)](./02-first-mod.md)
- Scaffolding a project with `yoake new hud-only my-hud` (including a list of all 5 templates)
- What each generated file does (`yak.toml`, `Cargo.toml`, `src/lib.rs`, `README.md`, `.yoake/`, etc.)
- The `yoake build` → `yoake check` → `yoake pack` workflow and checking the output at `dist/<id>-<version>.yak`

### 3. [Playing and Quick-Testing in the Game (`03-play.md`)](./03-play.md)
- How importing `.yak` packages and game loading work
- Hot-reloading mods whose WIT world is `client-mod` using `YOAKE_WASM_DEV_DIR` and `yoake dev`, plus safety precautions for developer mode

### 4. [Exploring the Example Mods (`04-examples.md`)](./04-examples.md)
- Walkthrough of the 7 examples in the repository (`hello`, `hud-item`, `server-storage`, `config-demo`, `permission-demo`, `full-demo`, and `full-services`)
- Overview of the hooks (`hook`) and host services (`services`) each example uses
- How to write `[config]`, `[wasm] permissions`, and `[permissions."node"]` in `yak.toml`

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

---

## Recommended Ways to Read

- **If you want to get something running right away**: Follow [`01-setup.md`](./01-setup.md) → [`02-first-mod.md`](./02-first-mod.md) → [`03-play.md`](./03-play.md) in order to draw your first text on screen, and continue to [`06-testing.md`](./06-testing.md) to test world scenarios in the game engine.
- **If you already know what feature you want to build**: After trying the basics once, jump to [`04-examples.md`](./04-examples.md) to adapt the closest example, and use [`06-testing.md`](./06-testing.md) to verify server-side behavior automatically.
- **When you run into an error**: Check the message printed by the command or in the game log, and look it up in [`05-errors.md`](./05-errors.md) (or Section 6 of [`06-testing.md`](./06-testing.md) for test runner errors) like a reference dictionary.
