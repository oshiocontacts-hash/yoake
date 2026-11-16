# 8. Getting Started with Yoake Mixin

Yoake Mixin lets a WebAssembly (Wasm) component in a signed `.yak` package intercept designated points in engine functions. Your mod can inspect or replace arguments, modify return values, or supply a result instead of running the original function body.[^overview]

This chapter walks through the `mixin-demo` example to show how to write mixin handlers and the rules they must follow.

---

## 8-1. What Mixin Can and Cannot Do

A mixin can only intercept functions that the engine exposes in its catalog. The current catalog has **51** injection sites, and every site allows HEAD, RETURN, and overwrite.[^catalog]

- **What you can do**: Read exposed arguments, replace arguments or cancel execution at HEAD, replace the return value at RETURN, or return a custom result instead of running the native body with overwrite.
- **What you cannot do**: Call arbitrary Rust functions, dereference memory addresses, or intercept functions that are not in the catalog.
- **Scope**: Although the catalog includes sites for entity ticking and death, animal targetability, and projectile or item spawning, there is no catch-all hook for "all entities" or "all AI decisions." Each site covers only its specific function.

Adding a new injection site requires an engine update. As a mod author, you choose from the sites already in the catalog. Package signature, identity, fingerprint (engine identity), ABI (the binary compatibility contract between engine and mod), and engine-version checks apply in full to mixins.[^overview]

## 8-2. Injection Points

In the SDK, a single `Handler` entry can implement one or more injection points. In the generated registration data, HEAD uses bit 1, RETURN uses bit 2, and overwrite uses bit 4.[^sdk]

| Point | SDK callback | Role |
|---|---|---|
| HEAD | `fn(&[Value]) -> HeadOutcome` | Inspect or replace arguments, or cancel before the native body runs |
| RETURN | `fn(Value) -> Option<Value>` | Receive the current result and keep it or replace it |
| overwrite | `fn(&[Value]) -> Value` | Supply a return value without running the native body |

### HEAD: Intercept Before the Body

Here is the chat logger from `mixin-demo`. If argument 1 is a string, it prints the message to the log and returns `Proceed` so normal processing continues.[^demo]

```rust
fn log_chat(arguments: &[Value]) -> HeadOutcome {
    if let Some(Value::Text(message)) = arguments.get(1) {
        println!("[mixin-demo] chat: {message}");
    }
    HeadOutcome::Proceed
}
```

A HEAD callback returns `HeadOutcome::Proceed`, `HeadOutcome::ReplaceArguments(Vec<Value>)`, or `HeadOutcome::Cancel(Value)`. The argument slice includes the receiver at index 0. `Cancel` skips any remaining HEAD handlers, overwrite, and the native body, but RETURN handlers still run on the cancelled result.

### RETURN: Modify Only the Return Value

This example from `docs/yoake-mixin.md` halves fall damage. Returning `None` keeps the current value; returning `Some(value)` replaces it.[^overview]

```rust
use yoake_wasm_sdk::mixin::{Handler, Value};

fn halve_fall_damage(result: Value) -> Option<Value> {
    match result {
        Value::Signed(damage) => Some(Value::Signed(damage / 2)),
        _ => None,
    }
}

static HANDLERS: &[Handler] = &[Handler {
    site: "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94",
    head: None,
    returned: Some(halve_fall_damage),
    overwrite: None,
}];

yoake_wasm_sdk::export_mixin!(HANDLERS);
```

In the SDK `Handler` struct, the field for a RETURN callback is named **`returned`**. Set it to `Some(halve_fall_damage)` and export the table with `export_mixin!`.

### overwrite: Replace the Body

The SDK field and callback type look like this:[^sdk]

```rust
    pub overwrite: Option<fn(&[Value]) -> Value>,
```

For example, the runtime test suite verifies that an overwrite returning `Value::Signed(55)` leaves the native body's call count at 0. Only one mod can register an overwrite for the same site and side; see section 8-7 for multi-mod conflicts.[^runtime]

## 8-3. Configuring the SDK and `yak.toml`

In the example's `Cargo.toml`, disable `default-features` on the SDK and enable `std` and `mixin`:[^demo]

```toml
yoake-wasm-sdk = { path = "../..", default-features = false, features = ["std", "mixin"] }
```

In `yak.toml`, set `interface = "1.1"` and the WIT world (mod kind) `world = "inject-mod"`, and provide the client module and/or server module required by your targets. List the **full site IDs, including their hashes**, under `[mixin] targets`:

```toml
[wasm]
interface = "1.1"
world = "inject-mod"
client = "bin/wasm/mixin-demo.wasm"
server = "bin/wasm/mixin-demo.wasm"
permissions = ["service.commands", "service.chat"]

[mixin]
targets = [
    "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94",
    "world::level::bonemeal::crop_bonemeal_increase#b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1",
    "game::chat::handle_chat_input#2d6b616020cec7006f8b8c6aeb2d8f9faf0caa7a8dbeb7f38b5a4d839b0d2e62",
]
```

`targets` must contain 1 to 1,024 unique entries. Server-only mixins are also supported. A `side = "both"` site can run in the client module, the server module, or both, depending on what the package provides. Each module registers all declared targets, and the host activates only the registrations that match that module's side. The example uses the same component for both sides, so server-only targets are filtered out on the client side.[^manifest]

Build `mixin-demo` with the command below, then stage the output `yoake_mixin_demo.wasm` into a separate package folder as `bin/wasm/mixin-demo.wasm`. Official package signing and distribution follow the standard owner workflow.[^demo]

```sh
cargo build --offline -j 4 --release --target wasm32-wasip2
```

## 8-4. Reading the Site Catalog

For a human-readable table, see "Committed sites and signatures" in [`docs/yoake-mixin.md`](../yoake-mixin.md); for the machine-readable catalog referenced by tooling, see [`crates/yoake-wasm/wit/inject-sites.json`](../../crates/yoake-wasm/wit/inject-sites.json). Copy the `Full target ID` directly into both `targets` and `Handler.site`.

Here is the JSON entry for the bonemeal crop-growth increment site:[^catalog]

```json
  {
    "id": "world::level::bonemeal::crop_bonemeal_increase#b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1",
    "path": "world::level::bonemeal::crop_bonemeal_increase",
    "signature_hash": "b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1",
    "arguments": [
      "scoped<&mutSelf>",
      "block-state-id"
    ],
    "result": "s32",
    "side": "server",
    "allow": [
      "head",
      "return",
      "overwrite"
    ],
    "source": "crates/horizon-world/src/level/bonemeal.rs",
    "argument_rules": [
      {
        "kind": "finite"
      },
      {
        "kind": "block-state",
        "registry": "live"
      }
    ],
    "result_rule": {
      "kind": "number",
      "maximum": 25.0,
      "minimum": 0.0
    },
    "required_capabilities": [],
    "client_required_capabilities": []
  },
```

- `id`: The full `path#signature_hash` ID. `signature_hash` is derived from the argument and result shapes; stale hashes are rejected at load time.
- `arguments` / `result`: The shapes of the arguments and return value passed to callbacks. `scoped<...>` is a scoped handle, described below.
- `side`: `client`, `server`, or `both`.
- `allow`: Allowed injection points—all three in this entry.
- `argument_rules` / `result_rule`: Validation rules for replacement values. Here, the growth increment must be between 0 and 25.
- `required_capabilities` / `client_required_capabilities`: Service permissions (`[wasm] permissions`) required to register this site (none in this example).

Doubling the bonemeal increment affects only this site; it does not double ordinary random-tick crop growth. In addition to bonemeal growth, `mixin-demo` halves fall damage and logs chat messages.[^demo]

## 8-5. Value Ranges and Rejection

Replaced numbers must fit both the exact integer or float width in the catalog and the site's allowed range. `usize` and `isize` use the host's pointer width. NaN and infinity are rejected by default, including inside vectors. Out-of-range values are **rejected, not clamped**.[^overview]

The table below lists common bounds. For other domains—such as crafting, experience, enchanting, fall and jump inputs, or attack speed—check each site's `argument_rules` and `result_rule`.

| Replacement domain | Inclusive bounds |
|---|---|
| Explosion radius | 0 to 128 |
| Sleep timer | 0 to 110 |
| Crop growth speed | 0.01 to 1024 |
| Bonemeal increment | 0 to 25 |
| Fire ignition odds | 0 to 10,000 |
| Damage amount | 0 to 1,000,000 |
| Exhaustion / saturation modifier | 0 to 40 |
| Food nutrition | 0 to 20 |
| Positions / item velocity (per component) | -30,000,000 to 30,000,000 / -1024 to 1024 |

`block-pos` is a 3-element list of signed 32-bit integers; `vec3` and `vec3-f32` are 3-element lists of floats. `block-state-id` is an unsigned 32-bit integer that must be smaller than the current world's state count. This live registry bound is checked for the two state arguments of `on_place` and the state arguments of `crop_random_tick`, `crop_bonemeal_increase`, and `drop_falling_block`. If the live bound is unavailable, the replacement is also rejected.

A `scoped<...>` handle is not a raw memory address; it is a read-only value valid only during the current call. When replacing arguments in HEAD, you must echo every original handle back unchanged, including its `scope`, `type`, and `snapshot` (the identity metadata carried by the handle). Caching a handle for later, reusing one across calls, or tampering with its contents is rejected.

If a handler traps or returns an invalid response, that site, side, and injection point is disabled (for how long is undecided), the mod, target, and failure are logged, and the call uses the native fallback described in section 8-8. By contrast, if an engine argument or native result exceeds the conversion budget, the host simply skips the callback and falls back to native execution without disabling the handler.[^runtime]

## 8-6. Intercepting Commands and Chat

The sensitive command and chat sites are `game::commands::execute::execute_line`, `game::commands::execute_command`, and `game::chat::handle_chat_input`. Registering any injection point on these sites requires the service permission ([wasm] permissions) `["service.commands"]`, and client-side chat interception also requires `"service.chat"`. Even a read-only logger like the example must declare them.[^catalog] Note that declaring `service.chat` requires `wasm.client`, while `service.commands` requires `wasm.server` in WIT worlds other than `inject-mod` (`inject-mod` is an exception).[^manifest]

Because command-side denylist checking is currently being reworked, details of the post-rewrite inspection procedure are **(being revised)**.

## 8-7. Ordering and Conflicts

Handlers are ordered by **`(bare mod ID, side)`**—using the package ID (`id` in yak.toml) directly rather than a formatted ID/side display string, so `foo` runs before `foo-extras`.[^runtime]

- **HEAD**: Later mods receive the arguments modified by earlier mods. Returning `Cancel` skips the remaining HEAD handlers, overwrite, and the native body.
- **overwrite**: Registering two overwrites for the same site and side causes a load-time conflict. Client and server overwrites are tracked independently.
- **RETURN**: Runs in mod-ID order, with each mod receiving the result replaced by the previous mod. RETURN also runs on results produced by a HEAD `Cancel` or an overwrite.

In short, multiple mods can stack changes on the same site with HEAD and RETURN, whereas only one mod per side can replace the body with overwrite.

## 8-8. Resource Limits and Traps

The main time and memory limits are listed below. Reaching 20 consecutive aggregate-window strikes (windows where the time budget is exhausted) disables the instance.[^overview]

| Limit | Client | Server |
|---|---:|---:|
| Per-call deadline | 20 ms | 25 ms |
| Aggregate execution per 50 ms window | 10 ms | 25 ms |
| Fuel per call | 10,000,000 | 10,000,000 |
| Linear memory | 64 MiB | 64 MiB |

In addition, each callback value conversion is capped at 4,096 total nodes, depth 32, and 256 KiB of payload.

**A trap or invalid response does not crash the game.** The host disables only that site, side, and injection point (for how long is undecided), logs the failure, and falls back to native execution. The exact fallback depends on where the failure occurred:[^runtime]

1. **HEAD or overwrite fails**: Runs the native body once with the original arguments.
2. **RETURN fails after the native body already ran with HEAD-modified arguments**: Uses the cached native result from that call. It does not re-run the native body and does not keep replacements from earlier RETURN handlers.
3. **RETURN fails after HEAD cancelled or overwrite succeeded**: Runs the previously uncalled native body once with the original arguments.

Side effects that have already occurred in the guest or native code are not rolled back. After a real trap, the next surviving callback recreates the mod component and resets any in-memory state the mod held. Aggregate accounting is preserved, and recreation time is charged to the budget; if recreation fails, that instance is disabled.

## 8-9. Speed When No Mod Is Installed

When no handler is registered for a site, the function performs a single relaxed atomic load and branch before entering the original body—before any thread-local side check, lock acquisition, snapshot creation, argument construction, or memory allocation.[^macro]

In Windows release microbenchmarks from development logs (median of 11 runs), the gate added **0.6194 ns/call** on a minimal function and **0.7934 ns/call** on a function matching `calculate_fall_damage`. That represents 40.53% and 9.84% respectively, showing a measurable relative cost on very short functions. However, these microbenchmarks do not measure full-game performance; whether whole-engine overhead with no mod installed stays below 1% is **(undecided)**.[^overview]

## 8-10. Common Pitfalls

| Symptom | What to check |
|---|---|
| Rejected for unknown site or mismatched signature | Copy the full `path#signature_hash` from the current catalog—not just `path`—into both `targets` and `Handler.site` |
| Does not run on the expected side | Check the site's `side` and `wasm.client` / `wasm.server`. You can point both to the same Wasm file, but server-only targets are filtered out on the client |
| Registration itself is rejected | Check for an empty registration set, duplicate entries, undeclared sites, disallowed injection points, an `interface` other than `1.1`, or a WIT world other than `inject-mod` |
| Adding an overwrite causes a conflict | Check whether another mod already overwrites the same site and side, and consider whether HEAD or RETURN can achieve the same goal |
| Disabled after returning a value | Check the `Value` variant, integer width, per-site numeric bounds, NaN/infinity, and live registry bounds for block-state IDs |
| Replacing arguments with handles fails | Keep every original handle unchanged; handles cannot be cached, reused across calls, or modified |
| Crop growth does not change as expected | `crop_bonemeal_increase` only changes the bonemeal increment; ordinary random-tick growth uses `crop_random_tick` |
| Cannot register command or chat sites | Declare `service.commands` (plus `service.chat` for client chat), and check `wasm.client` for `service.chat` and `wasm.server` for `service.commands` outside `inject-mod`. Detailed rewrite checks are (being revised) |

---

[^overview]: Source: `docs/yoake-mixin.md` (scope, values, failure behavior, limits, benchmarks).
[^sdk]: Source: `crates/yoake-wasm-sdk/src/mixin.rs` (`Handler`, callback types, registration bits, `export_mixin!`).
[^demo]: Source: `src/lib.rs`, `Cargo.toml`, `yak.toml`, and `README.md` in `crates/yoake-wasm-sdk/examples/mixin-demo/`.
[^manifest]: Source: `Mixin`, `Wasm`, and `Manifest::validate` in `crates/horizon-mods/src/yak/manifest.rs`, and "Package and registration" in `docs/yoake-mixin.md`.
[^catalog]: Source: `crates/yoake-wasm/wit/inject-sites.json` and `crates/horizon-mod-api-gen/src/inject_catalog.rs`.
[^runtime]: Source: `crates/yoake-inject-rt/src/lib.rs`, `src/tests.rs`, `src/security_tests.rs`, `tests/macro_dispatch.rs`, and "Ordering, conflicts, and failure" in `docs/yoake-mixin.md`.
[^macro]: Source: `crates/yoake-inject-macro/src/lib.rs`, `crates/yoake-inject-rt/src/lib.rs`, and "No-injection cost and measurement" in `docs/yoake-mixin.md`.
