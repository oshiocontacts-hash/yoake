# 5. Common Errors and How to Fix Them

This page collects the error messages you are most likely to run into while building a mod, along with their causes and how to fix them.
Compare these against the `"errors"` field in the JSON output from `yoake` commands, compiler messages, or the game log.

---

## 5-1. Hook (`subscriptions`) Errors

### ① `unknown hook subscription: <name>`

- **Cause**: The hook name `<name>` returned by `subscriptions()` does not exist (for example, due to a typo), or you subscribed to a full-catalog hook inside a standard module whose WIT world is `client-mod`.
- **How to fix**:
  - Instead of typing hook names as raw strings (like `"hudd"`), use the constants provided by the SDK: `hook::HUD` or `yoake_wasm_sdk::full::hook::*`.
  - If you are using a full-catalog hook, make sure you set `world = "client-full"` under `[wasm]` in `yak.toml` and implemented `ClientMod` with `export_client_full!`.

```rust
    fn subscriptions() -> &'static [&'static str] {
        &[hook::HUD]
    }
```

### ② `hook hud runs on the client side and cannot be subscribed from a server module`

- **Cause**: You registered a client-only hook (`hud` in this example) in `subscriptions()` on a server module (WIT world `server-full`). Conversely, subscribing to a server-only hook from a client module produces `hook <name> runs on the server side and cannot be subscribed from a client module`.
- **How to fix**:
  - Subscribe to screen-drawing hooks (`hook::HUD`, etc.) on your client module (`ClientMod`).
  - On your server module (`ServerMod`), subscribe only to server-side or shared (`both`) hooks such as `hook::REGISTER_COMMANDS`, `hook::PERFORM_COMMAND`, or `hook::SERVER_TICK`.

---

## 5-2. `yoake` Command and Toolchain Errors

### ③ `... is stale because ... is newer; run yoake build`

- **Cause**: You edited source files such as `src/lib.rs` or `Cargo.toml` and ran `yoake check` without rebuilding first, leaving `bin/wasm/*.wasm` older (`stale`) than your source code.
- **How to fix**: `yoake check` only validates files and never compiles code. Rebuild your `.wasm` binary by running:

```sh
yoake build
```

### ④ `name "con" is a reserved Windows name`

- **Cause**: The name (or `id`) passed to `yoake new` is a reserved Windows device name (`con`, `prn`, `aux`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`).
- **How to fix**: Pick a different name that avoids reserved words, such as `my-hud`:

```sh
yoake new hud-only my-hud
```

### ⑤ `YOAKE_WASM_DEV_DIR is unset; not installed`

- **This is not an error.** The build and check succeeded with `ok:true`, `installed:false`. This note simply tells you that the `.wasm` file was not copied because no development folder was configured.[^dev-cli]
- If you want `yoake dev` to copy a `client-mod` WIT world mod into your development folder, review the [safety warning and setup steps in Chapter 3](./03-play.md), set `$env:YOAKE_WASM_DEV_DIR='<folder-path>'` in that single PowerShell window, then run `yoake dev`.
- For `client-full` and `server-full` mods, you will instead see the note `raw Wasm hot reload only supports client-mod; not installed`. This also finishes with `ok:true`, `installed:false` without copying to the development folder. Build a `.yak` package with `yoake pack`, import it from the launcher, and restart the game.

### ⑥ Cargo Error When the `wasm32-wasip2` Target Is Missing

- **Symptom**: During a build, Cargo / `rustc` reports that the `wasm32-wasip2` target cannot be found (`can't find crate for 'core'` or `target may not be installed`).
- **Cause**: The `wasm32-wasip2` WebAssembly component build target has not been added to your Rust toolchain yet.
- **How to fix**: Run the following command in your terminal to add the target:

```sh
rustup target add wasm32-wasip2
```

---

## 5-3. Common `yak.toml` Mistakes[^validation]

If there is a mistake in `yak.toml`, `yoake check`, `yoake build`, or the game loader at startup will report one of the following errors:

### ⑦ `invalid YAK identifier: <id>`

- **Cause**: The `id` (or a dependency, shader, or service ID) does not follow the length or character rules.
- **How to fix**: An `id` must be **3 to 48 characters long** and contain **only lowercase ASCII letters (`a-z`), digits (`0-9`), underscores (`_`), and hyphens (`-`)**. Uppercase letters, spaces, and 1–2 character names are not allowed.

### ⑧ `wasm.interface must be major.minor, for example 1.0`
(or `unsupported wasm interface major version ...` / `wasm interface ... is newer than the supported 1.0`)

- **Cause**: `interface` under `[wasm]` is not formatted as `major.minor` (like `"1.0"`), or specifies a version other than the `"1.0"` interface supported by the current build.
- **How to fix**: Set `interface = "1.0"` in the `[wasm]` section.

### ⑨ `wasm.client must be a file directly under bin/wasm/ ending in .wasm`
(or for server modules: `wasm.server must be a file directly under bin/wasm/ ending in .wasm`)

- **Cause**: The path set in `client` or `server` under `[wasm]` is not in the form `bin/wasm/<filename>.wasm` (for example, it includes a nested subfolder, points directly into `target/...`, or does not end in `.wasm`).
- **How to fix**: Specify a `.wasm` file directly inside `bin/wasm/`, such as `"bin/wasm/hud-item.wasm"`.

### ⑩ `wasm.world must be "client-mod" or "client-full", not ...`

- **Cause**: `world` under `[wasm]` is set to something other than `"client-mod"` or `"client-full"` (such as `"server-full"`).
- **How to fix**: `world` only configures the client module. Set it to either `"client-mod"` (the default when omitted) or `"client-full"`. Server modules automatically use the `server-full` WIT world whenever `server = "bin/wasm/..."` is declared.

### ⑪ `wasm.world applies to the client module; declare wasm.client`

- **Cause**: Your mod is server-only (no `wasm.client` is declared), but `[wasm]` still includes `world = "..."`.
- **How to fix**: Remove the `world` line from `[wasm]` on server-only mods so it looks like this:

```toml
[wasm]
interface = "1.0"
server = "bin/wasm/server-storage.wasm"
permissions = ["service.storage"]
```

### ⑫ `unknown wasm permission "..." (known: service.commands, service.config, service.http, service.permissions, service.storage)`

- **Cause**: The `permissions` list under `[wasm]` contains an unrecognized permission name.
- **How to fix**: Only `"service.commands"`, `"service.config"`, `"service.http"`, `"service.permissions"`, and `"service.storage"` are valid. Check the spelling.

### ⑬ `wasm permission service.commands applies to the server module; declare wasm.server`
(or `wasm permission service.permissions applies to the server module; declare wasm.server`)

- **Cause**: `"service.commands"` and `"service.permissions"` apply only to server modules, but no `server = "bin/wasm/..."` is declared under `[wasm]`.
- **How to fix**: Either declare a server module (`wasm.server`) or, if your mod is client-only, remove the server-only permission from `permissions`.

### ⑭ `[wasm] needs at least one of client or server` / `at least one side must be enabled`

- **Cause**:
  - `[wasm] needs at least one of client or server`: You included a `[wasm]` section without specifying either `client` or `server`.
  - `at least one side must be enabled`: Both `client = false` and `server = false` are set under `[side]`.
- **How to fix**: Enable at least one side (client, server, or both) and specify the corresponding `.wasm` path.

### ⑮ `[config]` and `[permissions]` Declaration Errors

- `config <name>: value is not a declared enum choice`: On a `type = "enum"` setting, the `default` value is not listed in `values = [...]`.
- `config <name>: integer outside declared range` (or `float outside declared range`): The `default` number falls outside the `min`–`max` bounds.
- `invalid permission declaration: <name>`: Either the permission node name in `[permissions."<name>"]` is invalid, or `default_op_level` is outside `0`–`4`.
- `"permission denied: ..."` (runtime error): Your code called `services::config_get`, `services::storage_get`, `services::has_permission`, or a similar host service without declaring the matching permission (`"service.config"`, `"service.storage"`, `"service.permissions"`) under `[wasm] permissions` in `yak.toml`.

[^dev-cli]: Source: `dev_project` in `.lane/ref/p10-tools-yoake-cli-main.rs`.
[^validation]: Source: `crates/horizon-mods/src/yak/manifest.rs` and `crates/horizon-mods/src/yak/config.rs`.
