# MX3R1: Game-root accessor paths, `engine_take`, CI paths, compiler-enforced unsafe

Branch `yoake2/slice` on top of f7a6e0dac (MX3E1), working tree. No git writes, no installs, no game launch. Everything is behind `yoake2-slice` or in `yoake-slice` / `yoake-bindgen`; feature-off builds are unchanged. SDK-facing ABI notes: `docs/yoake2/ENGINE_CALL_ABI.md`.

## 1. No double execution on `OUT_TOO_SMALL` (`engine_take`)

Decision: keep the result.

- When the encoded result does not fit `out_cap`, the call has already run. The host keeps the bytes in `Host.tickets` keyed by `(activation entry, ticket)` and `engine_call` returns `-(256 + ticket)` (so `<= -257`). The needed size is still written as a `u32` at `out_ptr` when `out_cap >= 4`.
- New import `engine_take(ticket, out_ptr, out_cap) -> i32` copies the kept result and runs nothing. It returns the byte count (ticket spent), `-2` (still too small: size written, ticket kept) or `-7` (no such ticket). It is a plain host import: it never enters `native()`.
- A ticket is valid only in the activation that got it; `leave()` drops the activation's tickets (also on a panic unwind). Limits: 8 tickets and 16 MiB per activation; past that the call still ran and its result is lost with `-8` (`TICKET_LIMIT`), documented so guests size the buffer first.
- Documented as "never call again to retry" in `runtime.rs`, `call_status`, and `docs/yoake2/ENGINE_CALL_ABI.md`.
- `a_result_that_does_not_fit_is_kept_under_a_ticket_and_never_run_twice` counts engine runs through the dispatcher: one run for a too-small buffer, one take, and the second take of the same ticket is `-7`; also a too-small take buffer (size back, ticket kept) and `cap = 2` (ticket still issued). `a_mutating_call_with_a_too_small_buffer_changes_the_object_once` does a setter (`fire_ticks = 77`) then a getter with a 1-byte buffer: two engine runs in total, the object changed once. `tickets_are_bounded_and_die_with_the_activation`: nine too-small calls give eight tickets and `TICKET_LIMIT`.

## 2. CI paths

`.github/workflows/yoake-bindgen.yml` now triggers on `pull_request` and on `push` (`main`, `yoake/**`, `yoake2/**`) for changes in `crates/yoake-bindgen`, `yoake-slice`, `horizon-items`, `horizon-client`, `horizon-client-lib`, and the rustdoc inputs `horizon-core`, `horizon-world`, `horizon-client-core`, plus `Cargo.lock` and the workflow itself; and on `workflow_dispatch`. (No YAML anchors: Actions does not support them, so the path list is written out twice.) The test step also runs `yoake2_engine_call`.

## 3. Unsafe enforced by the compiler

State found: `horizon-core`, `horizon-world`, `horizon-client-core`, `horizon-items` already carry `#![forbid(unsafe_code)]`; `horizon-client/src/client_lib.rs` (the horizon-client-lib crate root) is `forbid`; `horizon-client/src/lib.rs` is `deny`, and the only `#[allow(unsafe_code)]` are the two `#[path]` module declarations in `entry.rs` (`dll_mods.rs`, `wasm_mods.rs`). Nothing had to change and no unexpected unsafe turned up; all of them build (feature off and on).

New: `yoake_bindgen::policy::check_unsafe_policy`, used by the CI gate and by `tests/policy.rs`. It checks the four `forbid` lines, the `deny` and `forbid` of horizon-client, that every `#[allow(unsafe_code)]` module file is in `frozen_files` (allowlist subset of the frozen files), and that no other file in those five crates opens unsafe code (comments and strings that merely say "unsafe" are ignored). Negative tests build a fake tree: an unlisted allow, a removed `forbid`, a stray `unsafe` block.

## 4. `Root::Game` accessor paths

### Runtime (`yoake-slice/src/activation.rs`, `runtime.rs`)

- `trait PathRoot { fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status> }` and `enum Accessor { Field(u32), Id(u64), Index(u32) }`. The engine side lends the object at a path as `&mut dyn Any`; an object that is gone is `Stale`, an unknown path `BadField`. The downcast is safe code.
- `activation::scope_root(root: &mut G, original, body)`: like `scope`, but argument 0 is a path root. `G` may carry a lifetime (`Game<'a>`): the root is erased to a pointer plus a function pointer whose signature has no lifetime (`resolve_impl::<G>`), the same erasure as `run_cell`, no transmute.
- `activation::open_path(root_handle, path)` verifies the object exists, then makes a **path handle**: top bit set, 31-bit slot generation, 32-bit slot, in a per-thread table; at most 256 per activation (`LimitReached`). All of an activation's path handles are freed when it ends (also on unwind), so they are `Stale` afterwards and a reused slot has a new generation.
- `with_arg::<T>` accepts both kinds of handle: a path handle re-resolves from the root on every call (the handle is a path, never a reference), the activation is locked while `f` runs. `get`/`set`/`original` on a path handle are `WrongType`.
- `runtime::dispatch_root(&mut Game, original)`: the hook entry for a function whose argument 0 is the root, same trap rule as `dispatch`.
- Unsafe: two new deref sites (`resolve_impl`, and the two call sites of the resolver in `open_path` / `with_path`), same discipline as `get`/`set`: the record is found and the activation locked before the pointer is used.

### When the root is available (decision)

Only in a hook whose argument 0 really is the `Game` root. Today that is one function, added behind the feature: `Game::yoake2_server_tick(&mut self)` in `game/native_client_items.rs`. It is meant to run once per tick on the integrated-server thread before the entity ticks. **It is not called from the tick loop yet** (that is a one-line change at the call site; the runtime does nothing until a mod is installed). Inside any other hook (`ArmorStand::tick_settle`, ...) the handle is a hook argument with no resolver, and a guest asking for a path gets `WrongType` (tested), because the caller holds an outer `&mut` (design 4b).

### Generated (`bindgen.json` target `game_items`, `game_paths`)

For each configured path the generator emits an arm of `impl PathRoot for Game<'_>` (inside the module below `game`, so private fields are reachable) and a binding `game_items::Game.<name>#handle` (id in the lock). Configured now: `item_entity` (`dropped`), `armor_stand` (`armor_stands`), `experience_orb` (`orbs`), `painting` (`paintings`), `item_frame` (`item_frames`), `cushion` (`cushions`), all by `id: u32`, and `experience` (direct field). The engine compiles with it (`cargo check -p horizon-client --features yoake2-slice`). Mobs (`Game.mobs`) and the player (`Game.player`) are not bound: their types live in modules (`mobs.rs`, the player code) that have no generated glue yet; adding them is a config entry once those modules are generated.

### Coverage gain

**196 bindings** (bound methods plus field getter/setter ids) of 7 types become reachable from a Game-root hook: ItemEntity 42 (6 methods + 36 accessor ids), ArmorStand 57, ExperienceOrb 27, Painting 18, ItemFrame 21, Cushion 18, Experience 13. Before, each was reachable only inside a hook on that very type. The table is in `docs/yoake2/bindgen/coverage.md`.

### Tests

- `yoake-slice` unit tests with a mock root: path handle reads and writes, two handles to one object, `WrongType`, `get`/`original` refused, handles freed with the activation (`live_path_handles() == 0`) and `Stale` after it, an object removed in `original` makes its handle `Stale`, and nested use inside `original` or inside an access is `Locked` (the overlapping exclusive use is rejected), the 256 bound, resolver panic frees everything.
- Generator fixture (native, real generated code): `impl PathRoot for Game`, `Game.thing#handle` by id and `Game.lone#handle` without an id; methods and field accessors through the path; an unknown id is `Stale`; the root handle calling a `Thing` method is `WrongType`; inside `original` the handle is `Locked`; after `original` removed the object it is `Stale` while the other handle works; after the scope the handle is `Stale`.
- WASM end to end (`yoake2_engine_call`): a hand-written WAT guest in a `dispatch_root` hook gets a handle for item 7 by id (8 bytes), reads its position (24 bytes, the right `f64`), writes `fire_ticks = 55` (the item changed), reads it back, then calls `original` (the engine removes the item) and the same handle is `Stale`. A missing id is `Stale`. The same request in the `ArmorStand::tick_settle` hook is `WrongType`.
  **Caveat:** there is no real `Game` in a unit test here (`Game::new` needs game resources, `HORIZON_TEST_RESOURCES` is unset, and the tests that need it are among the 40 ignored). The WASM test uses a hand-written `World` root that mirrors the generated resolver, and a hand-written "get handle" binding that mirrors the generated one. The generated resolver and binding are executed natively in the fixture and compiled in the engine, but not run against a live `Game`.

## 5. Panic note

When a guest-provoked engine panic is caught, the host logs `yoake: engine state may be inconsistent: binding <id> panicked on guest-supplied arguments (<message>); the mod is disabled` to stderr and the trap text says the same (`runtime::stats().trap`).

## Tests

| Suite | Result |
|---|---|
| yoake-slice (`--features wasm`): unit 23 (activation 19, codec 4), runtime 21, doctest 1 | all passed |
| yoake-bindgen `mini_engine` | 8 passed |
| yoake-bindgen `policy` (new) | 3 passed |
| horizon-client-lib `yoake2_engine_call` | 8 passed (the size test now covers tickets; new: mutating call, ticket bounds, Game-root path, missing id plus non-Game hook) |
| horizon-client-lib `yoake2_glue` | 5 passed |
| horizon-client-lib `yoake2_slice` | 7 passed, 1 ignored |
| yoake-bindgen `engine` gate (ignored, CI: toolchain pin, regeneration diff, append-only against the MX3E1 lock, unsafe policy, stale files, `cargo check -p horizon-client --features yoake2-slice`) | 1 passed (404 s) |
| feature-off `cargo test -p horizon-client --lib` | **1193 passed, 0 failed, 40 ignored** (unchanged) |

## Open

- `yoake2_server_tick` is not called from the integrated-server tick loop; mods cannot use Game paths in a running game until it is.
- Path handles exist for entity lists and direct fields. Mobs and the player need their modules generated; `Game.world_tables.items` (an `Option`) and nested lists need `Option`/nested accessors in the resolver generator.
- The WASM test mirrors the generated resolver instead of using a live `Game` (see the caveat).
- A path resolves a list element with a linear `find` by id per call (fine for hook-sized lists; an index or map would be needed for thousands of calls per tick).
- Ticket limits (8 / 16 MiB) and the 256-handle bound are judgement values.
- Two handles of one activation cannot be used at the same time (the activation lock is per activation), so a binding that would take two objects needs the borrow table; no such binding exists yet.
