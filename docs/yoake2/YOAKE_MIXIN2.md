# Yoake 2.0: jar-like mods with Java-level freedom (design draft, route C)

Status: draft for review, 2026-11-18. 1.0.0 ships as it is (Store submission 2 in certification). This is **Yoake 2.0.0**.

## 0. Owner decisions

- Mods come only from the owner and people the owner trusts.
- Mixin is as free as Java's, so a new mod idea never needs a launcher or engine version bump.
- It works like Forge jars:
  - **drop the file into the mods folder and play**;
  - make a mod and try it at once;
  - **only the one engine binary is signed**.
- Routes considered and rejected:
  - A: native DLL mods. Every DLL needs code signing for Smart App Control.
  - B: packs compiled into the engine. Every pack is a new exe to sign, and building is slow.

Why C works: a jar is bytecode run by the signed `java.exe`, so Windows never inspects it. A Yoake `.yak` is WASM bytecode run by the signed `yoake.exe`, the same position. What jars have that Yoake lacks is *reach*: the JVM shows mods everything (reflection, Mixin). Route C gives WASM mods that reach.

## 1. What a "full-access" mod can do

A full-access mod is still a WASM component, a `.yak`. It can:
1. **Call nearly any engine function** through generated host bindings (the reflection/accessor equivalent): world, entities, items, GUI, rendering helpers, networking, the atlas and textures, the integrated server.
2. **Hook nearly any engine function** with an *around* hook. The callback receives the arguments and an `original` callable. Head, return, cancel and overwrite are sugar on top.
3. **Use host capabilities freely:** HTTP and sockets (WASI sockets / http), files under its own data folder, timers and threads within wasmtime's limits, the GPU through the existing mod-GPU API.
4. Do everything that ordinary sandboxed WASM mods already do.

Who may run full access:
- **Local mods:** a `.yak` the player put into their own `mods` folder. The player's own choice, like dropping a jar. The launcher shows clearly that it has full access.
- **Catalog mods** from authors on the owner-signed trust list, which is delivered by the feed with a high-water mark and expiry (Fable review item 10).
- Mods from anywhere else stay sandboxed, as today.

## 2. Architecture

### 2.1 Generated engine bindings ("reflection")

A build step (xtask) reads the engine crates (rustdoc JSON, or `syn` over the sources) and generates:
- **WIT interfaces** under `yoake:engine/*`, mirroring the module tree, e.g. `yoake:engine/game-entities`.
- **Host glue** in the engine that implements them by calling the real functions.
- **A guest SDK crate** (`yoake-engine-sdk`) with Rust wrappers that look like the engine API.

Type mapping:
- **Plain data** (numbers, strings, enums, small structs, Vec, Option, Result): copied across, as in WIT records.
- **Engine objects that live in the engine** (`Game`, the world, an entity, an item stack in an inventory, GPU resources): **handles**. A handle is a generation-checked id in a per-context table. The guest calls methods on it; nothing is copied. Handles are valid only during the hook or tick that produced them, unless explicitly pinned. This is the safe answer to "`&mut Game` from a hook" (Fable review item 9).
- **Borrowed arguments** (`&T`, `&mut T`): handles for engine objects, copies for plain data, with write-back for `&mut` plain data.
- **Generics, `impl Trait`, closures, const fns:** not bound, or bound per listed instantiation. Document the gaps. Fable counted about 180 generic fns and 286 `impl Trait` in horizon-client.

Coverage target: public and crate-visible functions of the gameplay, world, render-helper and protocol modules. Hot internals stay out: mesher maths, the allocator, per-vertex code.

### 2.2 Around hooks on nearly every engine function

- An attribute added by an idempotent codemod gives each eligible function one slot.
- Slots live in **one global array indexed by a generated const id**, not one static per function. That means no export-count or duplication problems (Fable item 2 and 6).
- The entry check is one Acquire load. When the slot is empty, the original `#[inline(always)]` body runs. There is no exit check.
- When the slot is set, the engine builds the guest arguments, using the same mapping as 2.1, and calls the mod's export. The mod may call `original(args…)` zero or more times and returns the result.
- Chains: priority, then load order. Several mods compose by each one wrapping the next.
- Registration happens at load only, with Release publication. The chain is leaked and never unloaded. This matches Fable's item 9.
- Excluded:
  - generic fns, `impl Trait`, async, const fns;
  - the hot-leaf crates (mesher, light, worldgen inner loops, fast_hash);
  - anything listed `#[yoake::no_hook]` after benchmarks.

  The excluded list ships with the SDK.
- Existing value sites (the 56 validated Mixin sites) stay for sandboxed mods.

### 2.3 Performance

- No mod: one predictable branch per hooked call. Measured in spike C2. Target: under 1 % tick and frame time.
- Hooked: a WASM call costs on the order of 100 ns plus argument marshalling. Acceptable for gameplay logic; mods are told not to hook per-vertex code. wasmtime compiles with Cranelift and caches compiled modules on disk.
- The engine keeps thin LTO, because nothing is split into DLLs.

### 2.4 Install and play

- `mods/` folder in the instance: drop a `.yak` and it loads on start. In dev mode, hot reload uses the existing reload path; a file watcher reloads the mod.
- `yoake new --full` creates a full-access mod template. `yoake run` builds the mod, copies it into a test instance and starts the game.

### 2.5 Trust

- The engine keeps **one owner root key** plus a **spare root**. The author list in the feed is owner-signed and has a sequence high-water mark and an expiry. Release hashes are co-signed in the owner catalog (double signing, Fable item 10). There is a hash denylist.
- A local drop-in needs no signature: the player chose it. It is still verified as a valid component, and the launcher warns that it has full access.

### 2.6 Freezing and engine generations

The engine is meant to stay at 2.0.x. If a rebuild is ever needed (security, driver patches, Minecraft protocol), mods keep working: they talk through the generated WIT interfaces, not machine-level ABI. Bindings are versioned:
- an interface that disappears is kept as a shim;
- generated ids are stable, recorded in a lock file like `abi-1.0-ids.json`.

This is a large advantage over DLL mods.

## 3. Spikes, before any big build

- **C1 bindings:** generate bindings for one engine module, e.g. entities or items. That means handles, one `&mut Game` method, and plain-data structs. Then call them from a test guest. Report the generator approach (rustdoc JSON or syn), what failed to map, and the per-call overhead.
- **C2 hooks:** around-hook slots on one hot module. Measure the empty-slot cost, binary size and compile time, plus the cost of one WASM around hook on a hot function. The earlier MX3S2 started a `yoake-hook-macro` in `horizon-integ`; reuse it.
- **C3 capabilities:** an HTTP POST from a full-access guest through wasmtime WASI http/sockets, plus a runtime atlas sprite registered from a guest through generated bindings. These are cooking-duel's two hardest needs.

## 4. Phases after the spikes

1. Generator and host glue for the gameplay modules, with the lock file of ids.
2. The hook codemod across eligible modules, with the exclusion list and benchmarks.
3. Full-access policy: the mods folder, the trust list in the feed, launcher UI.
4. SDK, docs and examples: HTTP, a runtime item sprite, IME, a big gameplay mixin.
5. 2.0.0 release: CI, feed, then the Store.

Every phase gets a Fable review and full green CI. With no mods loaded, vanilla behaviour stays unchanged.

## 4b. Changes adopted from the Fable review (round 2, 2026-11-18)

- **Full-access mods are core WASM modules, not components.** Components can't be re-entered (may-enter), but around hooks must allow guest → `original` → another hook → the same guest.
  - There is a single dispatch import, `engine_call(id, ptr, len)`, plus an id table on the host. That means no WIT and no bindgen for thousands of functions.
  - The `&mut Store` reaches inner calls through the existing thread-local `ACTIVE` pattern.
  - The 1.0 component ABI stays for sandboxed mods.
- **Time limits:** fuel and wall-clock stop while `original` runs. Full-access mods have no hard per-call limit; a watchdog only logs.
- **Handles are paths from a root** (root plus a list of accessors: field, entity id, slot), re-borrowed from the root on every call.
  - Handles taken from hook arguments are valid only for that activation.
  - A dynamic borrow table (shared or exclusive per handle) locks handles lent to `original`. Functions that return references give child handles that lock their parent.
  - Calls that take two handles reject overlapping paths.
  - All unsafe stays inside the runtime crate.
- **Generator:** rustdoc JSON with `--document-private-items` on the pinned toolchain. `syn` is used only for the attribute codemod. Glue that calls private items is `include!`d inside each crate.
- **Coverage:** expect 40–60 % automatic at first, about 80 % with rules and shims. Ship an "unbound list". Start with the modules that real mods need (items, entities, GUI, world for cooking-duel), not everything at once. Generate getters and setters for private fields.
- **Threads:**
  - Hooks fire only on the main thread and the integrated-server thread, never on workers.
  - One instance per side; side-to-side state goes through host messages.
  - There are no guest threads (wasm threads are off; wasip2 can't spawn). Background work goes into a separate instance plus messages.
- **Networking:** a host async service, not WASI http. `http.request()` returns an id and the result arrives on a later tick; it is built on the existing `ureq` in a host thread. Sockets work the same way.
- **Cost:** with a hook, about 0.3–2 µs per call. The docs forbid hooking functions that are called about 100k times per tick.
- **Generations (2.6 corrected):** compatible while the engine is frozen. A new engine generation means regenerated bindings plus a diff report; ids stay stable, but removed or changed functions break the mods that used them.
- **Full access is opt-in per mod**, with a remembered switch. Mods delivered automatically by servers or friends never get full access.
- **Hot reload:** in dev mode, hook chains carry a generation so a reload can replace them.

Estimate: 16–24 weeks for broad coverage. The **first practical version, limited to the modules real mods need, is about 8–10 weeks.**

Spike order: **C0** re-entrancy (core module; check that a component traps; pausing the time limits), then **C1** generator on one module with path handles and the borrow table, **C2** hook slots, **C3** async HTTP plus runtime atlas sprite.

## 4c. Spike C0 result (2026-11-18, reports/MX3C0.md)

- **Core-module re-entry works.** The chain host F → hook_f → original_f → F → hook_g → original_g ran on one Store and one thread. The guest stack stayed consistent. The only unsafe is the ACTIVE thread-local `StoreContextMut`, about 12 lines.
- Component re-entry did not trap in wasmtime 48.0.7, but components cost 3–4× more per call. A trap poisons a component instance; it does not poison a core instance. **Core modules are confirmed**, for cost and simplicity.
- **Fuel:** save the caller's fuel and give each activation its own budget; native time in `original` is free automatically. **Epoch:** re-set the deadline inside `original` to pause it. Keep epoch as a log-only watchdog for full-access mods.
- **Cost:**
  - a trivial round trip is about 110–120 ns;
  - with 64 B of arguments, about 320–390 ns using alloc and free per call, about 130–150 ns expected with a scratch buffer.
- **Trap rule:** disable the mod. For the activation that trapped, run the original only if it never ran; otherwise return the recorded result. Keep a per-activation record of (calls, last_result) so side effects never run twice. Never call the disabled instance again.

## 4d. Owner goal (2026-11-18): close the remaining gaps with Java Mixin inside 2.0

**Call-site hooks** (`@Redirect` / `@Inject` at an INVOKE). The codemod also rewrites call expressions inside hookable function bodies into call-site slots, using the same ledger, id kind `callsite`, keyed by `caller path + callee path + ordinal`. A mod can wrap, replace or cancel one call inside one function, before or after it. Variables:
- field writes (`self.x = …`) get optional `store` sites;
- arguments and returns of calls are already covered by call-site hooks.

Locals in general are not exposed; that covers most `@ModifyVariable` use. Empty sites cost the same as function slots.

**Unbindable functions**, about 20–35 % today:
- generics: bind the concrete instantiations the engine actually uses, collected at build time;
- closures and `dyn Fn` arguments: a guest-callback handle (re-entry proven in C0);
- reference returns: child handles that lock their parent;
- `impl Trait` and iterators: an iterator handle (`next`) or collect-to-list adapters;
- private fields: generated getters and setters;
- hot tiny functions: no slot of their own, reached through their callers' call-site hooks.

The ledger and codemod being built now (yoake2/ids) must support the `callsite` and `store` id kinds from the start. Estimated extra: +2–4 weeks inside the 2.0 schedule.

## 4e. Generator inside the engine (MX3G1, 2026-11-18)

The C1 generator now lives in `crates/yoake-bindgen` (a build-time tool). It reads rustdoc JSON (stable toolchain, `RUSTC_BOOTSTRAP=1`, `--document-private-items`) and writes one glue file per module, which the engine `include!`s inside that module under the `yoake2-slice` feature: private items stay reachable, default builds are unchanged. Two modules are bound in the real engine (`horizon_items::items`, `horizon_client_lib::entities`), plus the two `Game` modules `native_client_items` and `native_entity_query`.

- **Classifier:** plain data is any non-generic struct or enum whose fields are visible from the module and are data (tuple, unit and data-carrying enums included), plus `Vec`/`Option`/`Result`/`Box`/`Arc`/`Rc`/maps/sets/tuples/arrays. A non-`Game` receiver is a handle, not an exclusion. Bound functions split into basic (a) and mechanical (b); handle-required and excluded functions are listed with reasons. Real numbers: `docs/yoake2/bindgen/coverage.md`.
- **Receivers:** `&self`/`&mut self` on a handle type is reached through `yoake_slice::activation::with_arg` (a `TypeId`-checked, lent argument of the current activation); the `Game` root gets `call_game(&mut Game, id, ..)`. Generated code has no `unsafe` and no raw pointers. Accessor paths from `Game` are reported, not yet resolved.
- **Fields:** getters and setters for every plain-data field of the structs in the module, private ones included, plus a scalar `FieldTable` per type.
- **Ids:** `crates/yoake-bindgen/ids.lock.json`; ids are never reused or renumbered, removed bindings stay as `unbound`, signature changes are reported.
- **CI:** `cargo test -p yoake-bindgen --test engine -- --ignored` regenerates from the real engine, fails on any diff and runs `cargo check` with the feature on. Report: `docs/yoake2/reports/MX3G1.md`.

## 5. Open questions (要確認)

- The first author list besides the owner (for example rumda500).
- Whether local drop-ins get full access by default, or after a one-time "allow full access" switch per mod. Suggested: a switch per mod, remembered.
- Whether 2.0 keeps the 1.0 sandboxed WASM ABI frozen, for compatibility with 1.0 mods. Suggested: yes.
