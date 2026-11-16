# Writing a Yoake WebAssembly mod

A Yoake wasm mod is a WebAssembly **component** (`wasm32-wasip2`) for the WIT world
`yoake:mods/client-mod@1.0.0` (`crates/yoake-wasm/wit/yoake-mod.wit`). The guest SDK is
`crates/yoake-wasm-sdk`; a complete example is `crates/yoake-wasm-sdk/examples/hello`.
The frozen world stays unchanged. The extended `client-full` and `server-full` worlds use
`yoake:mods@1.0.0` in `yoake-full.wit`; the GPU interface belongs to `client-full` only.

## Toolchain

```sh
rustup target add wasm32-wasip2
```

## Build

```sh
cd crates/yoake-wasm-sdk/examples/hello
cargo build --release --target wasm32-wasip2
# -> target/wasm32-wasip2/release/yoake_wasm_hello.wasm  (about 18 KB)
```

Use `crate-type = ["cdylib"]` and, for small output, `opt-level = "s"`, `lto = true`,
`strip = true`, `panic = "abort"` in `[profile.release]`. The SDK crates are excluded from the
host workspace; each has its own `[workspace]` table.

## Mod code

```rust
#![no_std]
use yoake_wasm_sdk::prelude::*;
use yoake_wasm_sdk::{Mod, export_mod, hook, hud};

#[derive(Default)]
struct Hello;

impl Mod for Hello {
    fn subscriptions() -> &'static [&'static str] { &[hook::HUD] }
    fn hud(&mut self) { let _ = hud::text("hi", (4.0, 4.0), 0xffffff, true); }
}
export_mod!(Hello);
```

- Implement `Mod` (all hooks default to no-ops) and list every hook you override in
  `subscriptions()` using the `hook::*` constants. The host only calls subscribed hooks.
- **Normal `std` works** (SDK feature `std`, on by default): `println!`/`eprintln!` go to the mod's
  log (one line per newline, long lines cut, bounded per call), `std::time::{Instant, SystemTime}`,
  `HashMap`, threads-free code, random (`HashMap`'s hasher) all work. The host links exactly this
  WASI preview 2 subset: `wasi:cli/{stdin,stdout,stderr,environment,exit,terminal-*}`,
  `wasi:clocks/{monotonic-clock,wall-clock}`, `wasi:random/*`, `wasi:io/*` and
  `wasi:filesystem/{types,preopens}`. stdin is empty, the environment and the arguments are empty,
  `std::process::exit` traps the mod (only that mod is disabled), the clock never blocks the host:
  `thread::sleep` and `pollable.block()` on a clock wait of more than 2 ms trap the mod (shorter waits
  return at once), the filesystem has **no
  preopened directory** (every `std::fs` call fails; the interfaces are linked only because `std`
  imports them) and there are **no sockets** (a component that imports `wasi:sockets/*`, or any
  other interface outside `yoake:mods/*` and the list above, is rejected at load). The example is
  `crates/yoake-wasm-sdk/examples/hello-std`.
- For the smallest modules turn `std` off (`default-features = false, features = ["runtime"]`) and
  write `#![no_std]`: the SDK ships its own allocator and panic handler (feature `runtime`) and
  `yoake_wasm_sdk::prelude` re-exports `String`, `Vec`, `Box`, `format!`, `vec!`. A panic traps
  the instance. See `examples/hello`.
- Host calls: `log`, `feedback`, `hud::{text, rect, text_width}`, `settings::{get, set}`,
  `net_send`, `local_player`, `playing`, `world_key`, `snapshot_world`, `instance_dir`,
  `account`, `screen::*`. All return `Result<_, HostError>` (`net_send`: `SendError`).
- The generated bindings come from `build.rs` (wit-bindgen 0.57.1's proc macro does not escape the
  package name `mod`).

Check the imports of a build (`yoake:mods/*` and, for `std` mods, the WASI subset):
`wasm-tools component wit target/wasm32-wasip2/release/yoake_wasm_hello.wasm`.

## Packaging in a `.yak`

Put the component at `bin/wasm/<name>.wasm` and declare it in `yak.toml`:

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

`interface = "1.0"` is the package's WASM compatibility declaration, checked separately from
the extended WIT package version. Keep it at `1.0` for this host, including full-world mods
built against WIT `1.0.0`. The host rejects imports outside `yoake:mods/*` and the WASI subset above.

## Developer mode: hot reload

With `YOAKE_DLL_MODS_UNSIGNED=1` the game polls every unsigned wasm module of an installed `.yak`
once a second and, when the file changed (and stayed unchanged for a whole poll), builds a fresh
instance and swaps it in at the next frame or tick. A new build that fails to load keeps the old
instance running (one warning in the log). `YOAKE_WASM_DEV_DIR=<folder>` additionally loads every
`<id>.wasm` in the folder as a mod (no `.yak` needed) and reloads it the same way. Both are off
without the flag. The startup log has one line saying `wasm: no mods (engine not started)` or
`wasm: N mod(s), engine started`.

## Full-catalog mods (`client-full`, `server-full`)

The frozen `client-mod` world has 19 hooks. Two more worlds of `crates/yoake-wasm/wit/yoake-full.wit` cover
every hook of the native catalog (`coverage.json` says how typed each one is):

| world | exports | runs |
|---|---|---|
| `client-full` | every client-side and shared hook | in the client (the render thread) |
| `server-full` | every server-side and shared hook + `server-services` imports | in the integrated server (host) and in the dedicated server |

A hook of the game's shared code (`both` in the catalog) goes to the client module when it runs on the
client thread and to the server module when it runs on the server thread.

### Choosing the world in `yak.toml`

```toml
[wasm]
interface = "1.0"
client = "bin/wasm/demo-client.wasm"   # world client-mod (default) ...
world  = "client-full"                  # ... or the full client world
server = "bin/wasm/demo-server.wasm"   # always server-full
```

Both modules of a package share one identity (id, version, sha256 of the client module, else of the
server module). A package with `required_on_join = true` must match on host and guest.
A dedicated server (`--dedicated`) loads only server modules, from the packages listed in the
`yoake-mods.txt` of its `--game-dir` (a server-only package is fine there). Development hot reload covers
both client worlds, swapping a fresh instance at the next frame; server modules are not hot-reloaded.
The server runs one world at a time
and a guest keeps its state in its instance, so each world's server starts with a fresh instance of the
server module (it runs `init` again when `server_about_to_start` fires for a later world). A package whose
client module fails to load does not run its server module either.

#### Limits every module runs under

* Memory 64 MiB, 1 memory, 8 tables of at most 65,536 elements, 16 instances; no threads.
* A hook call may run 20 ms (client) or 25 ms (server) of wall-clock time, checked every millisecond
  (a trap, fuel or time, disables the module; the limit is not tighter because a busy machine can pause
  a thread for a scheduler quantum, and a tight wall-clock limit disabled healthy mods); a call also has 10 million units of fuel, which bounds its CPU time first. All the calls of
  one module in a 50 ms window may use 10 ms (client) or 25 ms (server) of it;
  further calls in the window are skipped (a skipped hook answers its fallback) and counted, and a module
  that overruns in 20 windows in a row is disabled. `init` has 250 ms.
* Time spent inside a server service (the game breaking a block, reading a container, another package's
  `handle-call`) is the game's, not the guest's: it extends the call's deadline (by at most 400 ms in all)
  and does not count against the aggregate budget; the guest's own work stays bounded by fuel and the
  clock, and the native helpers allow 512 to 1024 calls per mod and server tick.
* A hook that finds its module busy (re-entered from the module's own service call) answers its fallback
  and is counted; the first drop is logged.
* A guest's answers are checked before they touch the game: floats must be finite, strings at most
  64 KiB, lists at most 65,536 elements, item stacks need a registered item id and a count between 1 and
  the item's stack limit, block states must exist, an NBT argument that cannot be encoded skips the hook,
  and the strings the engine must keep for the whole run (command names) are limited to 4096 strings of
  256 bytes and 256 KiB in total.

#### Permissions

`[wasm] permissions = ["service.commands"]` lets the server module call `run_command` (the package
installer/launcher should show it; the load log also says it). Without it `run_command` answers an
error. Even with it a command line goes to the game's own commands only (never to another mod's command
hook) and `transfer`, `fetchprofile`, `publish`, `unpublish`, `debug`, `jfr`, `perf`, `stop`, `save-off`,
`save-on`, `op`, `deop`, `whitelist`, `ban`, `ban-ip`, `pardon`, `pardon-ip`, `setidletimeout`, `test`,
`compute` and `pregen` are refused, also after `run` in `execute` and `return`.

The client-only `gpu` imports additionally require `service.gpu`. They are not imported by
`server-full` or the frozen `client-mod` world. A GPU error does not grant another capability.

### Writing one (SDK feature `full`)

```toml
yoake-wasm-sdk = { path = "...", features = ["full"] }
```

```rust
use yoake_wasm_sdk::full::{hook, server::{ServerMod, et, services}};
#[derive(Default)] struct M;
impl ServerMod for M {
    fn subscriptions() -> &'static [&'static str] { &[hook::PERFORM_COMMAND] }
    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool { /* true = take it */ false }
}
yoake_wasm_sdk::export_server_full!(M);
```

`yoake_wasm_sdk::full::client::ClientMod` / `export_client_full!` are the client twins. Contracts: cancel returns
`bool` (or `cancel` in an outcome record); replace returns an option; collect gets the aggregate and returns
it; `&mut` draw objects (`HudBatch`, `BlockEntityDraw`, `Corners`) become command lists you return; other
`&mut` values come back as `option` fields of the outcome. Engine objects that cannot be copied are `u64`
handles, valid only during the call. Example: `crates/yoake-wasm-sdk/examples/full-demo` (build once per
feature, `client` and `server`).

GPU resources are different from borrowed engine-object handles: the safe `full::client::gpu`
wrapper owns persistent `u32` resources and destroys them on drop. They belong to one client
instance, not to the server or another package, and are invalidated when that instance is replaced.

### What the host applies / does not

* HUD commands: text (plain), rect, gradient (strips), flat quads, sprites, items, clip/scale/blend runs.
  Skipped: `texture`, `highlight`, textured quads.
* Model commands: push/pop/translate/rotate/scale/light/keep, quad, block. Skipped: cube, model, item.
* Corner reservations and world-stage draws: applied. Client imports forward key bindings, view hits,
  display names and namespace ownership, GUI items, screen kind and slot rectangles, world quads,
  lines and billboards (including `world-pre-post`), post shaders, map tiles and colors, players and
  entities, camera overrides, writable GUI textures, PCM streams, mob-as-player and HUD placement.
  The catalog is `crates/horizon-mod-api-gen/abi-subset-client.json` (ABI IDs 111–145).
  Native services run under the calling WASM package's stable identity, so namespace validation and
  native per-mod limits also apply to WASM. Texture resources are instance-owned and capped at 64;
  failed creation does not consume a slot. Unsupported hook context returns an error, not a dummy
  success. The `client-services-demo` guest exercises real key, line, post-shader and bounded
  texture-write imports, including error propagation, in the runtime integration tests.
* Server services: all 84 of `server-services` (`crates/horizon-mod-api-gen/abi-subset-server.json`) answer,
  except four (below). Each validates its arguments (finite numbers, sizes, registry ids, UUIDs) and fails
  closed (an `Err` changes nothing). Mods are trusted inside the game: they may change other players'
  inventories, break blocks and move items. The one hard line is the PC: storage is a per-package folder, commands
  are gated and denylisted, paths are relative to the game folder. `status` is `forwarded` (a native host
  service existed), `new` (the native host service was added for this; native mods may use it too: fragment
  `tools/native-hook-fragments/wasm_services.json`, `docs/native-mods.md` "Server helpers for WebAssembly services")
  or `deferred`. The SDK's `full::server::ext` has plain-argument wrappers for the common ones.

  | Services | Status |
  | --- | --- |
  | `acting-player`, `server-players` (now with `sneaking`), `integrated-owner`, `held` (whole stack), `player-stacks`, `player-teleport`, `player-set-health`, `player-set-gamemode`, `player-clear-inventory`, `player-effect-give/clear`, `player-attribute-modifier-remove`, `tell`, `action-bar`, `player-title`, `player-clear-title`, `player-play-sound`, `bossbar-*` (7), `block`, `block-property`, `set-block-property`, `block-collision`, `edit-blocks`, `block-entity-sync`, `spawn-mannequin`, `remove-spawned-entity`, `net-send-to-player`, `world-dir`, `config-dir`, `entities-in`, `clip`, `run-command` (`service.commands`, denylist) | forwarded |
  | `player-attribute-modifier-add`, `hurt-entity`, `server-player-motion`, `set-server-player-velocity`, `set-player-attack-hitbox`, `clear-player-attack-hitbox`, `world-border`, `set-world-border`, `open-menu`, `menu-set-items`, `menu-close` | forwarded (native service of the engine) |
  | `world-control` (time, game rule, force-load), `team-apply`, `particle-spawn` | forwarded as typed changes written as the game's own commands (every word validated; no `service.commands` needed) |
  | `world-storage-read/write`, `config-read/write` (per package folder `yoake-wasm/<id>`, keys of letters, digits `_ . -`, 1 MiB a value, 256 keys, 32 MiB), `asset-read` (`data/<key>` of the game's resources), `qa-mode-enabled` (false), `qa-record` (debug log) | forwarded (WASM layer) |
  | `play-sound-at` | new (WASM layer: every player in earshot) |
  | `break-block-as` (the game's own player break: drops with Fortune/Silk Touch, tool damage, XP, stats, mod events), `player-sneaking` (guests, the LAN host and single player's owner), `container-slots`, `container-set-slots` (same items, totals verified; plain containers, both halves of a double chest), `containers-in`, `player-set-slots` (36 main slots, totals verified), `block-tags`, `item-tags`, `block-tool-requirement`, `can-harvest`, `attribute-get`, `item-entities-in`, `item-entity-move`, `item-entity-pickup`, `give-items` (remainder returned), `inventory-apply` (exact: id + count + components, all or nothing; `give` needs room for all), `advancement-progress`, `advancement-grant`, `entity-vitals-of` | new |
  | `mod-call` | new: `target-mod`, `method`, `payload` (1 MiB); the callee exports `handle-call(caller, method, payload)` of the `server-full` world; no call into a caller that is waiting (it is busy), at most 4 nested, never to itself |
  | `block-entity-read`, `block-entity-patch-call` | still missing: a block-entity hook returns its new state directly (no native service to forward) |
  | `range-override-set/clear` | deferred (Nestgate voice, W5) |

  `handle-call` is a new export of the `server-full` world: a module built before it cannot be loaded by
  this host (rebuild with the SDK). A service called from a read-only hook (the hook has no `&mut` host)
  answers `Err("unsupported: ...")`; the services that change the game need a hook that may.
* A guest that subscribes to a hook with no adapter is refused at load (`UNWIRED` in the generated code;
  `crates/horizon-client/src/wasm_mods/full_adapters_report.txt` lists them and the partial ones).
* The adapters (`full_adapters.rs`) are generated by `horizon-mod-api-gen` and **committed**, like the WIT.
  After a catalog or generator change run `YOAKE_UPDATE_WIT=1 cargo test --offline -j 4 -p horizon-mod-api-gen`
  (a test fails when the committed file is out of date). The game's build does not generate them.
* Legacy draw-command adapters require a render-stage subscription. GPU commands recorded during a
  frame are queued for their specified stage; a subscribed frame hook can prepare them before that stage.
* The guest examples used by the tests are built by `yoake_wasm::test_guests` (feature `test-guests`) into
  `CARGO_TARGET_DIR`, then `YOAKE_WASM_TARGET_DIR`, else a `target-wasm` folder next to the repository,
  else `target/wasm-guests`. Guest builds use offline Cargo and four jobs.
  With `YOAKE_REQUIRE_GUESTS=1`, `YOAKE_CI=1` or `CI`, a guest that cannot be built fails its test
  instead of skipping. `YOAKE_REQUIRE_GUESTS=1` also requires a usable adapter for the GPU guest test.

## Client GPU API

The GPU catalog is `crates/horizon-mod-api-gen/abi-subset-gpu.json` (ABI IDs 146–163).
WIT types and generated host adapters are committed. Use `yoake_wasm_sdk::full::client::gpu`
for safe ownership; the generated raw imports are available when a lower-level interface is needed.
Creation is rejected during `init`: create lazily in a subscribed client hook. Recording uploads,
copies, dispatches, draws and reads is allowed only while the host has an active frame.

| SDK operation | Contract |
| --- | --- |
| `Buffer::create`, `write`, `copy_to`, `range`, `read` | Explicit size and usage; checked ranges and copy alignment; reads return asynchronous tickets |
| `Texture::create`, `write` | Typed format, dimensions, mip count, usage and 2D-array/3D choice; tightly packed uploads with checked origins and extents |
| `Sampler::create` | Nearest/linear filtering and clamp/repeat addressing |
| `Shader::create`, `Shader::from_file` | Bounded WGSL validated and rewritten through Naga; structured error with line/column when available |
| `ComputePipeline::create`, `dispatch` | Validated entry point, bind groups and workgroup counts |
| `RenderPipeline::create`, `draw` | Typed vertex layout, color/depth format, depth write, blend, cull and line mode; optional vertex/index ranges and instancing |
| `BindGroup::create` | Pipeline-derived layout and explicit borrowed bindings, with retained host dependencies |
| `Readback::poll` | `Ok(None)` while pending, bytes once ready, error on failure; consumed after completion/error, or destroyed on drop |
| `frame` | Version, monotonic GPU frame index, tick, delta and viewport size |

Buffer usage flags are `COPY_SRC`, `COPY_DST`, `INDEX`, `VERTEX`, `UNIFORM` and `STORAGE`.
Texture flags are `COPY_SRC`, `COPY_DST`, `SAMPLED`, `STORAGE` and `RENDER`. Supported formats are
RGBA8 unorm/sRGB, RGBA16/32 float, R32 float, RG16 float, Depth32 float, BGRA8 unorm and RGB10A2 unorm.
Usage, format, binding and attachment mismatches return errors rather than bypassing wgpu validation.
`Draw` supports indexed/non-indexed and instanced draws. `Target::default()` selects the engine
stage target; explicit color/depth textures select an offscreen target, with optional color clear.

### Stages and read-only engine bindings

Commands execute in stage order, then mod load order, then each mod's recording order.
Engine-target draws preserve the vanilla attachment contents; offscreen targets do not replace them.
Engine depth uses reverse Z (`GreaterEqual`). No-mod operation records no GPU commands and creates
none of the mod-only frame resources or screen copies.

| `Stage` | Placement | Screen inputs |
| --- | --- | --- |
| `WorldOpaque` | After vanilla opaque drawing | Neither world depth nor scene color |
| `WorldTranslucent` | After vanilla translucent drawing | Read-only copied world depth |
| `AfterWorld` | After world rendering and screen-input copies | Read-only copied world depth and scene color |
| `Hud` | After the HUD, before menus | Neither world depth nor scene color |

Bind groups 0–2 are mod-owned. Group 3 is reserved for the engine and cannot be supplied by a mod:

| Binding | Read-only resource |
| --- | --- |
| 0 | Versioned `EngineFrame` uniform |
| 1 | Block/item atlas texture |
| 2 | Atlas sampler |
| 3 | World depth texture, restricted to `WorldTranslucent` and `AfterWorld` |
| 4 | Scene color texture, restricted to `AfterWorld` |
| 5 | Lightmap texture |

`ENGINE_WGSL` declares the `EngineFrame` struct and `@group(3) @binding(0)` uniform named `engine`.
Append the texture/sampler declarations your shader needs. The host checks their binding types and
stage availability, rejects engine writes, and supplies copies instead of sampling a live attachment.

### Frame ABI version 1

The uniform is 576 bytes, using column-major matrices and 16-byte-aligned fields. Check
`engine.header.x` for version 1 and `engine.header.y` for byte size. `header.zw` carry the low/high
tick words. The hook's `Frame.index` is the tick; `gpu::frame().index` is the GPU frame counter.

| Offset | WGSL field / type | Meaning |
| --- | --- | --- |
| 0 | `header: vec4<u32>` | Version, size, tick low, tick high |
| 16 | `view: mat4x4<f32>` | Current view |
| 80 | `projection: mat4x4<f32>` | Current unjittered projection |
| 144 | `view_projection: mat4x4<f32>` | Current unjittered view-projection |
| 208 | `inverse_view_projection: mat4x4<f32>` | Its inverse |
| 272 | `previous_view_projection: mat4x4<f32>` | Previous unjittered view-projection; current on the first frame |
| 336 | `jittered_view_projection: mat4x4<f32>` | Current jittered view-projection |
| 400 | `inverse_jittered_view_projection: mat4x4<f32>` | Its inverse |
| 464 | `camera_time: vec4<f32>` | Camera XYZ, elapsed seconds |
| 480 | `viewport_delta: vec4<f32>` | Width, height, delta seconds, tick as float |
| 496 | `sun: vec4<f32>` | Sun direction XYZ, W = 1 |
| 512 | `moon: vec4<f32>` | Opposite direction XYZ, W = 1 |
| 528 | `fog_color: vec4<f32>` | Fog color |
| 544 | `fog_range: vec4<f32>` | Fog range parameters |
| 560 | `jitter: vec4<f32>` | Jitter parameters |

### Limits and failure isolation

Handles are nonzero, owner-checked and kind-checked; on counter wrap the registry skips live IDs.
Destroying a handle
does not release memory still retained by a command, bind group, pipeline or readback. Accounting is
logical allocation accounting rounded to 64 KiB, **not measured driver VRAM**: 256 MiB per mod,
1 GiB shared, and 4,096 handles per instance. Allocation failure unwinds its reservation.
Buffers are 1 byte–256 MiB; textures are at most 8,192 pixels on either 2D axis, 256 array layers,
or 512 on a 3D axis, with validated mip counts. WGSL source is at most 256 KiB.

Each mod/frame permits 4,096 commands/draws, 65,536 instances per draw, 16 MiB uploads,
256 MiB buffer copies and 1 MiB requested readback bytes. A read is 1 byte–1 MiB; its retained
source and staging storage remain charged while pending. Upload budgets are checked before
allocating staging buffers or padded texture data.

Static safety caps reject clearly excessive work rather than charging every triangle as a
full-screen draw. The full per-mod/frame caps are 1,024 screens of estimated fragment work and
2^24 vertices multiplied by instances. Fragment work is the viewport/scissor intersection's
covered pixels multiplied by the primitive count, accumulated across all draws of a mod/frame,
then divided by 2,048 once, rounding up at the end. Topology determines the primitive count,
including all instances. This assumes meshes have an overdraw depth of 2,048 primitives,
rather than charging a whole viewport or a separate rounding penalty per draw or instance.
`draw_region` narrows that rectangle, while `draw` uses the whole target. Compute charges total
invocations multiplied by the shader's static instruction, texture and loop cost, with a
64 Mi-unit cap. Its allowance always admits at
least one maximum-size workgroup (256 invocations at the maximum static cost of 8,192).
Consecutive draws to the same target share a render pass, with at most 64 passes per mod/frame.
Shader and pipeline creation attempts remain limited to two per frame and eight per minute,
including failed attempts and reloads. Depth and scene snapshots are shared once per stage
across mods; engine bind groups are cached per pipeline within each owner's batch.

At `after-world`, scene colour may intentionally be copied into a mod-owned texture and read back
to the CPU; the same 1 MiB per-mod/frame readback limit applies. HUD-stage GPU commands run only on
the real frame when frame generation is enabled, never on interpolated frames. Mods that never
read scene colour do not force a scene-read target. Scene-read targets stay allocated until
120 consecutive frames without a read. If a read is requested after that target has lapsed,
or the source lacks `COPY_SRC`, scene-reading commands are skipped for that frame without an
error or suspension; other commands still run, and the request makes the target sticky again
for subsequent frames. The frame descriptor reports the actual render target's dimensions and format.

The Naga IR guard caps loops at 256 iterations per function activation, including helper functions
and continuing blocks. Static limits include 8,192 instructions, 64 texture fetches, call depth 16,
64 helpers, 16 entry points, 4,096 expressions, 256 workgroup invocations and 16 KiB combined private
storage and function locals.
Workgroup-size overrides, barriers, subgroup/ray operations and workgroup storage are rejected.
Only the validated, rewritten Naga module is compiled; robustness checks are not relaxed.

Measured GPU time is the main per-mod budget on every adapter. With `TIMESTAMP_QUERY`, the host
uses pass timestamps. Without it, each mod's command batch is submitted separately and timed
from its submission to `on_submitted_work_done`. This fallback is a conservative completion
latency, not isolated GPU execution time: earlier queued engine work and delayed callback
delivery can increase the charge. Both measurements enforce the same rules:
at most 4 ms per frame averaged over a rolling two-second window. Three frames over 50 ms
within 60 seconds suspend that owner's GPU API without suspending its other services; older
strikes expire. Each peak immediately pauses new submissions for at least 50 ms. A mod cannot
submit a newer frame until its outstanding older frame completes and its measurements are
collected, so a slow mod cannot queue an unbounded backlog ahead of enforcement.
New mods start at one sixteenth of the static caps, then eight distinct safe measured frames
allow each doubling up to the full caps. Each qualifying frame must measure at least 0.25 ms
and use at least one quarter of its current fragment, vertex-instance or compute allowance, aggregated
across all measured stages. The ramp is per mod, not per pipeline, and slow frames reset it;
idle, zero-cost, skipped, unmeasured and underused frames do not grow it. The initial one-sixteenth
allowance is 64 screens, 2^20 vertex-instances and 4 Mi compute-cost units, with the same minimum workgroup
allowance. Without timestamps, each mod permanently receives that initial allowance.
There is no CPU-timing-based GPU ramp on those adapters.

Host recording/encoding/submission CPU time has a separate rolling two-second 4 ms/frame
budget and expiring peak strikes. Shader, pipeline and other resource creation calls are
excluded from that per-frame CPU budget, so compilation latency does not suspend legitimate
mods. They still count against the WASM aggregate host/guest budget and creation rate limits.
CPU timing is **not a substitute for GPU timing**. Pending timing samples are limited per
owner (32); each query set stays within wgpu's 4,096-query ceiling, and timing storage is charged.
Suspension survives renderer recreation. The engine's default uncaptured-error handler is
unchanged; mod resource creation errors are captured in mod scopes. These limits reduce risk,
but actual driver/TDR behavior still needs platform tests.

Accepted residual risk: a static estimate cannot both admit large meshes and reject every
pathological shader workload. On adapters with or without timestamps, a hostile mod can cause
one very slow submitted frame before its completion is observed; it is then throttled, not
allowed an unlimited stream of unchecked frames. The third peak strike, or a sustained average
violation, disables GPU work and quarantines the package fingerprint. A repeated fault for the
same fingerprint requires explicit owner re-enable, even after a package change. Already
submitted work cannot be preempted: one frame may include several stage batches, and its
duration has no hard upper bound. These controls do not guarantee immunity to a driver hang
or OS TDR before completion. A device loss or abnormal exit uses the recovery marker below.

Before GPU submission, the host persists registered owner identities in
`<HORIZON_HOME>/state/wasm-gpu-inflight.json` only when the identity set changes, without fsync on
the frame path. An identity remains while work is outstanding, then for 120 idle frames (or until
clean exit); unchanged active sets are not rewritten. Failed writes are retried after 120 frames,
with warnings limited to once per minute, and are not attributed to the mod. Device loss blocks all in-flight identities in memory and in
the recovery file, including across renderer recreation. The next start quarantines those
identities and logs the marker path. Malformed recovery state fails closed. Abnormal
termination can also leave a marker: it is **not proof** that a listed mod caused a driver fault.
Quarantine does not expire through clean launches in which the blocked mod cannot run GPU work.
Measured-budget suspension also immediately quarantines its registered owner, even when its
submission has already completed. Fault counts persist per package-version/module-SHA-256
fingerprint, including across package changes and explicit re-enables. After a fingerprint's first
fault, changing the package can
automatically clear quarantine. After its second fault, only an explicit owner action can clear
quarantine, even if the package changes. Repeated observations of the same outstanding fault
do not increment its count.
`mod_gpu_runtime::reenable(identity)` explicitly clears quarantine and the live GPU suspension;
it rejects identities with outstanding work and persists the change before enabling them,
without erasing fault history.
This is the API for a future launcher/developer-screen re-enable control; no UI is added here.

### Development shaders and demo

With **both** `YOAKE_DLL_MODS_UNSIGNED=1` and `YOAKE_WASM_DEV_DIR=<folder>`, `Shader::from_file`
can override its embedded source from `<folder>/<mod-id>/<relative-path>.wgsl`. A missing file
initially uses the embedded source. Paths are relative WGSL files, canonicalized inside that mod's
directory; traversal, absolute paths, alternate streams and escapes are rejected. Limits are
64 tracked files per instance, 512 bytes per path and 256 KiB per file.

After a valid edit stays unchanged across two one-second polls, the host loads a fresh client
instance and GPU resources at the next frame. Invalid source or a failed component load leaves the
old instance running with a diagnostic; deleting an override also keeps the old instance. A valid
WGSL edit with a pipeline-interface mismatch can still fail when the new instance creates its
pipeline, disabling that mod. Override files must include `ENGINE_WGSL` when they use `engine`.

`crates/yoake-wasm-sdk/examples/gpu-demo` contains a 2,048-triangle textured mesh,
64 compute-updated particles drawn in a bounded region, and a depth-based after-world tint.
Its mesh, compute and particle workload is shared with a headless renderer regression; the
effect embeds `effect.wgsl`. Build from that directory, without running the game:

```sh
cargo build --offline -j 4 --release --target wasm32-wasip2
cargo build --offline -j 4 --release --target wasm32-wasip2 --features verify
```

The `verify` feature additionally checks GPU readback and rejected bounds/workgroup/allocation
requests; `reload` and `trap` variants exercise replacement and trap cleanup in host tests.
`docs/yoake-gpu-coverage.md` maps implementation and tests, including the strict guest test commands
and the remaining platform-validation obligations.
