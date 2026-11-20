# FR-F1 wave 3: W8 (n-ary activations) and W9 (every hook shape bridged)

Branch `yoake-free/f1-w3` (from `yoake-free/f1-w2` @192aac27a), worktree `C:\Users\USER\NestgateHorizon\horizon-f1`. Not pushed.
`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0`, no stamped builds (the dev loop stays unstamped; stamping is checked through the overlay).
Disk was 13 to 26 GB free all wave (other builds), so I only ran what is small: unit tests, the wasm tests of `yoake-slice`, `cargo check` of
`horizon-client --features yoake2-slice`, and the bridge/glue/slice integration tests of `horizon-client-lib` (about 4 GB of target). Targets deleted at the end.

| Item | State |
|---|---|
| W8 every argument readable and replaceable, `original` with replaced args, borrow table, test guest on a 5-argument fn | done |
| W9 opaque handles for any type | done (held handles; see the limit on field access below) |
| W9 per-argument kinds, zero-argument hooks, by-value arguments | done |
| W9 UNBRIDGED below 15 % | **1.2 %** (98 of 8,165 hooks of the three crates), from 58 % (lib+client) and 100 % (world) |
| W9 `&Game` read root with path traversal | **not done**: `&Game` hooks are bridged as an opaque held reference (guest can pass it on, not traverse). Follow-up W9c. |
| W9 generated field tables for the 637 + 225 receiver types | **not done**: needs a rustdoc-JSON run of six crates and the generator's "all fields" mode (about 6 GB of disk, not available). Follow-up W9b. |
| W9 per-module bridge for W4's private types | partly: the bridge now refuses (reason in `UNBRIDGED`) a type private to another module instead of emitting uncompilable code; a bridge included into the owning module can name it. Wiring the per-module includes is follow-up W9d. |

## 1. W8: n-ary activations (`crates/yoake-slice`)

Commits: `W8: n-ary activations ...`, `W8: dispatch_n_id, arg_get/... (ABI 1.1) ...`.

**Engine side (`activation.rs`).**
* `ArgSet` registers up to **8** arguments (was 4 and `&mut T` only): `mut_ref` (with a field table), `shared` (read-only table), `opaque_mut` / `opaque_shared` (any `T`, lifetimes allowed, no fields), `value` (a by-value scalar as bits). `scope_n(set, original, body)` opens the activation; more than 8 arguments refuse the scope (`LimitReached`), the caller then runs the original.
* `original` receives an `ArgView` built **from the activation's current slots** at call time, so replacements the guest made are what the engine body sees. `ArgView` is also what the next mod of a chain receives.
* **Borrow table:** `ArgView::mut_ref::<T>(i)` hands out `&mut` to one argument at most once per `original` call (a second request, or a shared view of a `&mut` already out, is `None`), so a bridge can take `(&mut A, B, &mut C)` without two live `&mut` to one object. Replacement can not alias: `arg_swap` exchanges the two objects' slots (pointer set unchanged; tables travel with the objects), `arg_replace` lets a **shared** argument take another shared argument's object, and refuses a `&mut` (`Overlap`).
* New statuses `Overlap = 8` and `WrongKind = 9`; existing handle paths (`get`, `set`, `with_arg`, `open_path`) answer `WrongKind` / `WrongType` for a by-value slot.

**Runtime and ABI 1.1 (`runtime.rs`, ENGINE_CALL_ABI.md).** `dispatch_n_id(hook_id, set, orig)` with the same chain semantics as the other dispatchers (priority order, trap rule, fall-through). New imports `arg_get(handle, out)`, `arg_set(handle, bits)`, `arg_swap(a, b)`, `arg_replace(dst, src)`; argument `i` is the argument-0 handle with its low byte set to `i`. `ABI_VERSION` is **1.1** (an additive minor: 1.0 guests, including everything the SDK builds today, still load and get the existing "older minor" info line). Guest SDK: `Handle::arg(i)`, `arg_value`, `set_arg_value`, `swap_args`, `replace_arg`.

**Tests.**
* `activation::tests` (3 new, 31 total, pure arena, no wasm): three arguments of three kinds readable and replaceable and seen by `original`; borrow-table refusals; swap/replace/overlap/kind/limit rules; arena freed.
* `tests/nary.rs` (wasm, WAT guest, 3 tests): a hook on a **five-argument** fn (`&mut Thing`, `i32`, `&mut Thing`, `&Thing`, `&Thing`): the guest reads all five (`get` / `arg_get`), gets `Overlap` / `WrongKind` statuses for the illegal replacements (not traps), sets a field, replaces the value, swaps the two `&mut` objects, replaces one shared argument by the other, calls `original`; the engine body ran on exactly the swapped-in object with the replaced values (`3 + 7 + 2*10 + 5*100`). Two mods compose and the second sees the first one's replaced value. Without a mod the original runs on the arguments as given.
* All earlier `yoake-slice` suites with `--features wasm` pass (abi test updated for 1.1: `check_abi(1.0)` is now "older minor").

## 2. W9: every hook shape in the bridge (`yoake-hook-codemod bridge --nary`)

Commits: `W9: bridge --nary ...`, `W9: bridge refuses types private to another module; in-tree bridges ... regenerated`, `W9: Copy by-value arguments ...`, docs.

**Rule.** With `--nary` every hook whose signature has other than exactly one argument takes the n-ary form (`dispatch_n_id`), and every hook the arg0 forms had to leave in `UNBRIDGED` (no table, `&Game`, by-value first argument, no arguments, non-static lifetime) falls back to it. One-argument hooks with a table keep the arg0 forms (they already show everything). Argument kinds in the generated dispatcher:

| argument | registered as | the guest can |
|---|---|---|
| `&mut T`, table known, no non-static lifetime | `mut_ref` | fields, bindings, swap |
| `&T` with a read-only table | `shared` | read fields, replace |
| `bool`, integers, `f32`, `f64` by value | `value` (same encoding as results) | `arg_get`, `arg_set` (an out-of-range replacement keeps the original argument) |
| `Copy` by value, `&T` without table, `&str`, `&[T]`, `&dyn`, `&Game` | the value or reference is copied and held (`opaque_shared`) | pass on |
| non-`Copy` by value, `&mut T` without a table | moved into an `Option` the bridge owns (`opaque_mut`) | pass on; the first `original` takes it back, a later one returns 0 (same rule as before, so the engine body runs at most once) |

Also: `--module '*'` for the bridge; a hook that names a type private to another module than the one the bridge file is included into is `UNBRIDGED` with the reason (`TypeNameCheck`; found by `cargo check`: it emitted `EntityFluidWorld`, a private struct of a child module).

**Measured on the stamped overlay** (`sync-ledger --frozen` into a scratch root, then `bridge --nary --module '*'` per crate with the existing glue tables; 8,165 hooks):

| Crate | bridged | of which n-ary | unbridged | % |
|---|---|---|---|---|
| horizon-client-lib + horizon-client (share `yoake_glue/`) | 7,227 | 6,547 | 82 | **1.1 %** |
| horizon-world | 840 | 840 | 16 | **1.9 %** |
| total | 8,067 | 7,387 | 98 | **1.2 %** |

Before (wave 2 numbers, as-is generator): lib+client 42 % bridged / 58 % unbridged; world 0 % / 100 %.
What is left in `UNBRIDGED`: 43 borrowed return types (`-> &'r T`; needs a result handle), 21 hooks with more than 8 arguments, 27 whose arg0 form failed first and the n-ary form refused (`&Game`-receiver ones with a borrowed return), a few private-type ones.

**Compile and behaviour checks of the generated code** (the four in-tree bridged modules regenerated with `--nary`: entities 129, `game::item_use` 54, mobs 101, motion 197 hooks):
* `cargo check -p horizon-client --features yoake2-slice`: clean.
* `cargo test -p horizon-client-lib --features yoake2-slice,yoake-hook-bench --test yoake2_bridge --test yoake2_glue --test yoake2_slice`: **12 + 5 + 7 passed**. Two of the old tests encoded the old classification and were updated (hook 1, a five-scalar-argument fn, is bridged now; the "not bridgeable" example is a borrowed-return hook). One real regression was caught and fixed on the way: `Copy` by-value arguments were first treated as moved-once, which made a second `original` a no-op (`tick_settle` guest calling original twice); they are held copies now.
* New test `a_five_argument_fn_is_bridged_n_ary_and_the_guest_reads_and_replaces_scalars`: an SDK-less WAT guest hooks `vertical_collision_velocity(f64, f64, f64, bool, bool)` through the generated bridge, reads argument 0 (f64 bits) and 4 (bool), replaces `slime` by false, and the engine returns 0.0 instead of the bounce velocity.
* Codemod bridge tests: 4 (the arg0 expectations unchanged; one new test covers n-ary: table / scalar / held / zero-argument / by-value-first / `&Game` shapes and that the old forms are unchanged without `--nary`).

### What this does not give a guest yet

* **Field access on tables that do not exist.** Opaque handles make every hook callable and every argument *passable*, swappable and replaceable by identity. Reading or writing a *field* needs a generated table, and `engine_call` needs a generated binding whose receiver type matches (`with_arg::<T>` is a type-id check; opaque slots have none). The 637 (lib+client) and 225 (world) receiver types without tables are exactly the work of W9b: run `regen.sh` with all six crates (rustdoc JSON, about 6 GB and a quiet disk) and add an "every field of types listed by the bridge's `UNBRIDGED`/held report" mode to `yoake-bindgen` (`bindgen.json` targets are explicit today). The generator can already tell which types matter: the held `T` names in the generated bridges.
* **`&Game` traversal** (W9c): a read-only `PathRootRef` (generated `iter()` arms next to the existing `iter_mut()` ones in `render.rs:628`, `scope_root_ref`, read-only `open_path`/`with_path`).
* **Per-module bridge for private types** (W9d): `overlay.rs` would generate one bridge file per module and add the `include!` line in the overlay (the tree stays unstamped), so private types are nameable.
* The generated bridge files for the whole workspace are not committed or wired: only the four modules that already had them (regenerated). For the other 8 k hooks the overlay would generate and include them at build time (same W9d work); measured here by running the generator on the overlay copy.

## 3. Risks

* The ArgSet/ArgView layer adds `unsafe` in `activation.rs` (typed re-borrow from a raw slot pointer). Its soundness argument is the existing one (the scope holds the borrows; the activation is locked while `original` runs) plus the taken-mask; the new tests exercise it, but `miri_activation.rs` was not extended or run (no nightly here). Please run it before release.
* `type_name`-based matching of opaque types (lifetimes erased) is a runtime check, not a type system guarantee; two different types with the same printed name in different crates would be confused by `swap_args` (same kind and the same name). Unlikely, noted.
* ABI 1.1: a guest built against 1.1 imports does not load on a 1.0 engine (minor additions are one-way, as documented).
* Bridges compile with `&mut` (argument) types; `p_h` holders add one `Option` per held argument on the stack for hooked calls with a mod installed only for the slot owner; with no guest the cost is the empty-slot check plus building an `ArgSet` only when a chain exists (the check happens before `ArgSet` use in `dispatch_n_id`, but the holders are still created at the top of the generated fn): **not measured**; it needs the W6 cost gate.

## 4. Needs checking

* Miri on `activation.rs` n-ary paths; the cost of the n-ary bridge prologue with no mod (W6).
* `yoake2_engine_call` (the fourth integration suite of the CI line) was not run (disk).
* A full stamped build with n-ary bridges wired for all modules (W9d first).
