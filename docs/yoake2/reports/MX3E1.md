# MX3E1: `engine_call` with hardening

Branch `yoake2/slice` on top of 7f7f100d0 (MX3G1), working tree. No git writes, no installs, no game launch. Everything is behind the `yoake2-slice` feature or in `yoake-slice` / `yoake-bindgen`; feature-off builds are unchanged.

## 1. The `engine_call` import (`crates/yoake-slice/src/runtime.rs`)

`env.engine_call(handle: i64, id: i32, ptr: i32, len: i32, out_ptr: i32, out_cap: i32) -> i32`, registered with `engine_import!`. (`handle` is the hook-argument handle, needed for methods; free functions ignore it.)

Core diff, for re-review:

- **`engine_import!` gains a `prepare:` arm.** It runs before `native()` with the caller, and its value is borrowed by `engine:`, which still runs only inside `native()`. The existing arm is untouched.
- **Bounded copy before `native()`**: `copy_args` refuses `len < 0` or `len > MAX_CALL_ARG_BYTES` (1 MiB) without reading guest memory (status `ARGS_TOO_LARGE`), otherwise copies into a host `Vec`. A range outside the guest memory traps (same rule as an out-of-bounds `out` before).
- **Dispatch by id** through `runtime::set_engine_dispatch(fn(handle, id, &[u8], &mut Vec<u8>) -> Result<(), CallError>)`. The engine registers one function that tries `entities::yoake_glue::call` then `items::yoake_glue::call` (ids are disjoint through the one lock).
- **`catch_unwind`** around the dispatcher, inside `native()`. A panic caused by guest arguments does not unwind through wasmtime or the host: `finish` turns it into an `Err`, which traps the guest, so the MX3V1 round 3 rule applies unchanged: the mod is disabled with the text `engine panic inside engine_call <id>: <message>`, the original runs once if the guest had not run it, and the instance is never entered again. The activation lock is released by the unwind of `with_arg`.
- **After leaving `native()`**: the result is written at `out_ptr` only if it fits `out_cap`. Otherwise the needed size is written as a little-endian `u32` (when `out_cap >= 4`) and the status is `OUT_TOO_SMALL`; nothing else is written. A bigger buffer and a retry work, and no trap happens.
- **Return value**: `>= 0` = result bytes written. Negative: `-1` args too large, `-2` out too small, `-3` bad arguments (any decode error), `-4` read-only, `-5` unknown binding, `-6` no dispatcher, `-16 - status` for handle errors (Stale -17, Locked -18, BadField -19, NoSuchArg -20, WrongType -21). Constants are in `runtime::call_status`.

## 2. Codec hardening (`codec.rs`)

- `Vec` (and `[T; N]`, and the generated vector decoders) reserve `len.min(4096)` (`MAX_PREALLOC`) and grow as elements really arrive. A length is also checked against the bytes that remain, so 1 MiB of arguments claiming a billion `u64`s is `UnexpectedEnd` with no reservation.
- A decode depth counter (`MAX_DECODE_DEPTH` = 64): `Decoder::enter`/`leave`. `Box<T>` decode and every generated named-type decoder (`dec_X` wraps `dec_X_body`) enter a level. Past 64 it is `CallError::TooDeep`. Tested on a recursive `Node { next: Option<Box<Node>> }` through generated glue (10 levels decode, 1000 are refused) and on a hand-written `Box<Self>` chain in the codec.
- New `CallError::ReadOnly(id)` and `CallError::TooDeep`.

## 3. Frozen fields (setters are opt-in)

- `bindgen.json`, per target: `writable: { "Type": ["field", ..] }` and `frozen_types`; top level `frozen_files`.
- Getters exist for every data field. A setter is generated **only** for a listed field; for every other data field the id is reserved in the lock with status `ReadOnly`, `BINDINGS` lists it, and the dispatcher answers `CallError::ReadOnly(id)` (`READ_ONLY` through `engine_call`). The scalar `FieldTable::set` also only writes writable fields (a frozen scalar gets `BadField`).
- The generator refuses to run when a writable list names `id`, `*_id`, anything containing `uuid`, `len`, `index`, `idx`, `seq`, the field `join`, or a tuple index, or a field that is not a data field of the type.
- Types defined in a file that contains `unsafe` are frozen entirely (no accessors and no bound methods; receivers are excluded). The grep over the engine found two such files in the targeted crates: `horizon-client/src/dll_mods.rs` and `horizon-client/src/wasm_mods.rs` (the other hits are examples and a test, not engine types). They are the `frozen_files` list; none of the four generated modules defines types there, so this changes nothing today and guards the survey and the next modules.
- Conservative writable list (the rest is read-only):
  - `ArmorStand`: `fire_ticks`, `health`, `position`, `previous`, `velocity`, `on_ground`, `yaw`, `previous_yaw`, `no_gravity`, `invisible`, `small`, `show_arms`, `no_base_plate`, `name_visible`, `pose`, `last_hit`, `disabled_slots`.
  - `ItemEntity`: `health`, `fire_ticks`, `position`, `previous`, `velocity`, `on_ground`, `age`, `pickup_delay`, `no_gravity`, `bob_offset`, `needs_sync`, `stuck`, `ticks`.
  - `ItemStack`: `count`, `damage`.
  - Left read-only on purpose: `id`, `join`, `thrower`, `target` (entity references), `stack`, `item` (the item id), `equipment`, `vehicle`, `custom_name`, `support`, every `enchantment_*` field.
- `docs/yoake2/bindgen/coverage.md` lists the writable fields per module and the count of read-only accessors.

## 4. `#![forbid(unsafe_code)]`

Every generated module starts with `#![forbid(unsafe_code)]`. The substring grep test is gone; `glue_module_forbids_unsafe_code` compiles the real glue (passes), then inserts an `unsafe` block into the module and asserts that rustc rejects it.

## 5. CI gate

- `.github/workflows/yoake-bindgen.yml` (there is no `yoake-hooks.yml` on this branch): pinned toolchain `dtolnay/rust-toolchain@1.99.1`; fixture and slice tests; the regeneration gate (`--ignored` engine test); the engine tests with the feature.
- The gate test now: (0) requires `rustc` = `crates/yoake-bindgen/toolchain.txt` (1.99.1), and runs rustdoc with `RUSTDOCFLAGS=""`, default features, never the glue's own feature; (1) regenerates and fails on any difference in glue, lock or reports; (2b) append-only: with `YOAKE_BINDGEN_BASE_LOCK` set (CI writes it from `git show origin/<base>:...ids.lock.json`) no id may be removed or renumbered and no path may change id; (2c) **stale glue**: any `*.rs` file next to a configured glue file that no target writes is a failure; (3) `cargo check -p horizon-client --features yoake2-slice`.
- The same checks exist in the binary: `yoake-bindgen --check [--lock-base FILE]` prints `DIFF:`, `LOCK:`, `STALE:` lines and exits 1.

## 6. `spikes/c1-bindgen` deleted

Nothing referenced it (checked in manifests, workflows, scripts, sources).

## Tests

| Suite | Result |
|---|---|
| yoake-slice (`--features wasm`): unit 19 (activation 15, codec 4: round trips, malformed input, huge length, deep nesting), runtime 21, doctest 1 | all passed |
| yoake-bindgen `mini_engine` | 8 passed (generated glue compiles and runs; classifier; determinism; ids stable; `forbid(unsafe_code)` enforced by the compiler; writable opt-in and forbidden names; append-only lock; stale glue) |
| horizon-client-lib `yoake2_engine_call` (new, WASM guest on a real `ArmorStand` in the real S2 hook of `tick_settle`) | 4 passed |
| horizon-client-lib `yoake2_glue` | 5 passed (setters changed: `thrower` and the item id are now `ReadOnly`) |
| horizon-client-lib `yoake2_slice` | 7 passed, 1 ignored (bench) |
| yoake-bindgen `engine` gate (ignored, CI; toolchain pin, regeneration diff, append-only lock against the MX3G1 lock, stale files, `cargo check -p horizon-client --features yoake2-slice`) | 1 passed (420 s cold rustdoc) |
| feature-off `cargo test -p horizon-client --lib` | **1193 passed, 0 failed, 40 ignored** (unchanged) |

What the `engine_call` tests prove: a WAT guest calls `render_yaw` and `id#get` and `fire_ticks#set` inside the hook and the result/effect is the engine's; `ArmorStand.id#set` returns `READ_ONLY` and the id is unchanged; an unknown id and malformed bytes are statuses and leave the mod enabled; 1 MiB + 1 and a negative length are `ARGS_TOO_LARGE`; `position#get` with an 8-byte buffer is `OUT_TOO_SMALL` with `24` written at `out_ptr`, with a 2-byte buffer nothing is written, with 24 bytes it succeeds; `is_disabled(99)` panics inside the engine (`[0, 5, 1, 2, 3, 4][99]`), the mod is disabled with an `engine panic inside engine_call` message, the original runs exactly once, no activation is left open, later ticks run the original without entering the guest (`entries` stays 1), and a new mod installed on the same thread works afterwards.

## Open

- The default panic hook still prints the engine panic message to stderr before `catch_unwind` returns; a quiet hook around guest-driven calls is not installed (a global hook change is not this task's call).
- `Root::Game` is still not available through `engine_call`: `call_game` exists but a guest has no handle for the root, so Game-root bindings are unreachable from WASM. Accessor-path handles and the non-receiver handle table (`&World`, ...) remain the next step.
- `engine_call` allocates the argument and result `Vec`s per call (the 1 MiB cap bounds them); a per-thread scratch buffer is a performance follow-up, nothing measured here.
- The writable list is judgement, not proof of invariant-safety: for example `ArmorStand.health` and `ItemEntity.age` can be set to values the engine would never produce. Review the list.
- `frozen_files` is a hand-kept list of the two files that contain `unsafe`; the CI gate does not grep for new ones.
- Miri is still not available (`with_arg` unsafe unchanged since MX3G1).
