# Bridge: ledger hook ids -> yoake-slice dispatch (design, MX3CS2 Phase 1d)

Status: design, 2026-11-19. Not implemented on `yoake2/callsite`: it needs `yoake-slice` (branch `yoake2/slice`
@c852d7ad8) and its generated field tables, which are not on this branch. Written for the merge of the two
branches.

## Goal

A guest that registers hook id N (`hook_register(N, priority)` in `yoake_init`) is entered for exactly the fn hook
or call-site hook whose **ledger id is N**, and for nothing else. One id space: the ledger shards (`hook-ledger/`)
hand out fn and call-site ids from one counter, starting at 1. Id 0 stays the legacy single-export guest
(`ModConfig::hook_export`); ids at or above `SYNTHETIC_HOOK_BASE = 0x8000_0000` are the runtime's own (for example
`HOOK_SERVER_TICK`). The ledger and runtime assert both ranges (Phase 1d): no ledger id is 0, none is at or above
0x8000_0000, and `SLOT_CAPACITY` (2^20) is below it.

## How a hook runs today

- S2 slot: `SLOTS[N]` holds a chain of typed callbacks. A hook site with an empty slot runs the original after one
  Acquire load. The only way in is the generated, crate-private `Type::__yoake_register_<fn>(callback, order)`
  (call-site shims: `__yoake_register___yoake_cs_<id>`), which checks the callback's exact TypeId.
- yoake-slice: `runtime::dispatch_id(N, arg0: &mut T, &FieldTable<T>, orig: &mut dyn FnMut(&mut T) -> i64) -> i64`
  enters a multiplexed guest only if it registered N; otherwise it runs `orig`. The guest sees `arg0` as a handle and
  its fields through the table; the other arguments are not visible to it.

## The bridge

One generated module per instrumented crate, compiled only with the `yoake2-slice` feature, `include!`d at the crate
root next to `__yoake_hook_keys` (so it can name crate-private types and the `pub(crate)` registration fns):

```rust
#[cfg(feature = "yoake2-slice")]
#[doc(hidden)]
pub mod __yoake_slice_bridge {
    include!(concat!(env!("OUT_DIR"), "/yoake_slice_bridge.rs"));
}
```

`yoake-hook-codemod bridge` writes `src/yoake_glue/hook_bridge_<module>.rs` for each scanned module (included
into that module under `#[cfg(feature = "yoake2-slice")]`). For every live id N whose shape is bridgeable,
`pub mod hook_bridge` contains a dispatcher, `BRIDGED`, `OBSERVE_ONLY`, `UNBRIDGED`, `mode`, and `install`:

```rust
// ledger id 71: horizon_client_lib::entities::ArmorStand::tick_settle
fn __bridge_71(
    original: &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep),
    stand: &mut ArmorStand,
    medium: StandMedium,
    step: StandStep,
) {
    let _ = yoake_slice::runtime::dispatch_id(71, stand, &crate::yoake_glue::FIELDS_ArmorStand,
        &mut |stand| { original(stand, medium, step); 0 });
}

pub const BRIDGED: &[u32] = &[
    // sorted list of every id this module can install
    71,
];

pub fn mode(id: u32) -> Option<yoake_slice::bridge::Mode> {
    if BRIDGED.binary_search(&id).is_err() {
        return None;
    }
    if OBSERVE_ONLY.binary_search(&id).is_ok() {
        Some(yoake_slice::bridge::Mode::ObserveOnly)
    } else {
        Some(yoake_slice::bridge::Mode::Full)
    }
}

pub fn install(registered: &[(u32, i32)]) -> yoake_slice::bridge::InstallReport {
    let mut report = yoake_slice::bridge::InstallReport::default();
    for &(id, priority) in registered {
        if id >= yoake_slice::runtime::SYNTHETIC_HOOK_BASE {
            report.not_mine.push(id);
            continue;
        }
        let order = yoake_hook_rt::Order { priority, mod_load_order: 0 };
        let outcome: Option<Result<(), &'static str>> = match id {
            71 => Some(yoake_slice::bridge::install_once(id, || {
                ArmorStand::__yoake_register_tick_settle(__bridge_71, order)
            })),
            // ... one arm per id in BRIDGED
            _ => None,
        };
        match outcome {
            Some(Ok(())) => report.installed.push(id),
            Some(Err(err)) => report.failed.push((id, err)),
            None => {
                if let Some(&entry) = UNBRIDGED.iter().find(|&&(u_id, _)| u_id == id) {
                    report.not_bridgeable.push(entry);
                } else {
                    report.not_mine.push(id);
                }
            }
        }
    }
    report
}
```

### Installing and querying hooks (`install`, `install_once`, `InstallReport`, `mode`)

Registering appends to a slot's chain and nothing removes an entry (S2 hot-reload generation, MX3S2 open problem 3),
so `hook_bridge::install` wraps every typed registration in `yoake_slice::bridge::install_once(id, || ...)`.
`install_once` runs the registration **at most once per process** and caches the first `Result<(), &'static str>`
outcome (including failures, which are not retried): a second world, or a second mod on the same id, reuses the
dispatcher instead of adding another one (which would run guests twice per call). An installed dispatcher whose id
no loaded mod registers any more (the mod was unloaded with its world, or the call comes from a thread without mods)
looks up an empty chain and runs the original. Which guests run, and in what order, is the runtime's chain for that
id (priority, then load order); the dispatcher's own slot order uses the priority of the first mod that registered it.

`install` returns a `yoake_slice::bridge::InstallReport` that keeps the three failure/skip cases distinct:
- `installed`: ids in `BRIDGED` whose `install_once` returned `Ok(())` (installed now or earlier in the process).
- `failed`: `(id, reason)` for ids in `BRIDGED` whose typed registration returned `Err(reason)` (e.g., slot full or
  callback type mismatch).
- `not_bridgeable`: `(id, reason)` for ids in this module's `UNBRIDGED` list.
- `not_mine`: everything else — another module's id, an unknown id, or `id >= yoake_slice::runtime::SYNTHETIC_HOOK_BASE`.

`hook_bridge::mode(id)` returns `Some(Mode::Full)` or `Some(Mode::ObserveOnly)` for ids in `BRIDGED`, and `None`
otherwise (for `UNBRIDGED` or unknown ids). The loader combines the modules' `mode` functions into
`yoake_slice::bridge::set_mode_resolver` so `hook_register` can tell guests what mode a hook runs in.

The production loader does this in `yoake2_mods::install_bridges`, called by `load_world_mods` once the world's
mods have run `yoake_init`: it takes the union of the ids every loaded mod registered (`runtime::stats_of`, id 0 =
legacy guest left out), tries each bridge's `install` in turn (entities, then `game::item_use`; ledger ids are unique
across crates) and reports, per mod, the ids no bridge took (`LoadReport::unbridged`, one line each in
`LoadReport::messages`). Only the slots a guest asked for get a callback; every other slot stays empty and costs one
load. Test: `yoake2_loader` `a_loaded_mod_that_registers_a_bridged_hook_id_is_called_by_the_engine` (approved list,
two world starts).

### Which hooks are bridgeable

The slice passes one argument to the guest, so a hook is bridgeable when:
- its first parameter is `&mut T` where T has a generated `FieldTable` (the yoake-bindgen field tables of
  MX3G1, for example ArmorStand, ItemEntity, ExperienceOrb; `Game` uses `dispatch_root_id` instead);
- the other parameters are captured by the `orig` closure unchanged (the guest cannot see or replace them yet);
- the result is:
  * `()`, an integer, `bool`, or `f32`/`f64` (round-tripped as `i64`; floats as bits): the guest may replace
    the result or cancel the call by not calling `original`;
  * any other non-borrowed result type `R` (no non-`'static` lifetimes, not `impl Trait` or `!`): bridged
    in **observe-only mode** and listed in `hook_bridge::OBSERVE_ONLY`.

Anything else (non-`&mut` arg0, missing `FieldTable`, arg0 or return type with a non-`'static` lifetime,
generic parameters, or `bridge-exclude.txt` entries) cannot be bridged and is listed in
`hook_bridge::UNBRIDGED`.

Call-site shims are bridged the same way: their first parameter is the receiver.

#### Results

Full-mode dispatchers (`Mode::Full`) encode the original's result as `i64` for the guest and decode the guest's `i64`
return value back through `yoake_slice::bridge::{encode, decode}`:

| Result type | Encode (`orig` -> guest) | Decode (guest -> host) | Out of range when |
|---|---|---|---|
| `()` | `0` | ignored (`()`) | never |
| `bool` | `encode::bool(r)` (`0` / `1`) | `decode::bool(value)` | `value` is not `0` or `1` |
| `i64` | `r` (identity) | `value` (identity) | never |
| `u64` | `encode::u64(r)` (bit pattern) | `decode::u64(value)` | never |
| `usize` | `encode::usize(r)` (two's complement) | `decode::int::<usize>(value)` | `value < 0` (or `> usize::MAX`) |
| `isize` | `r as i64` | `decode::int::<isize>(value)` | outside `isize::MIN..=isize::MAX` |
| `i8`, `i16`, `i32`, `u8`, `u16`, `u32` | `encode::int(r)` | `decode::int::<T>(value)` | outside `T::MIN..=T::MAX` |
| `f32` | `encode::f32(r)` (bits in low 32 bits) | `decode::f32(value)` | outside `0..=u32::MAX` |
| `f64` | `encode::f64(r)` (64-bit pattern) | `decode::f64(value)` | never |

When a type can be out of range, the dispatcher tracks `let mut last: Option<T> = None;` inside the `original`
closure. If `decode` returns `None`:
1. The dispatcher calls `yoake_slice::bridge::note_out_of_range(id, value)` (which increments `out_of_range_count()`
   and prints once per hook id to stderr).
2. If `original` ran during the guest's activation, the last `original` result (`last`) is returned; otherwise
   `original` runs now and its result is returned. That direct run skips the rest of the chain: guests after the
   one that returned the bad value do not run for this call.

A value the guest passes through unchanged (`encode(last) == value`, e.g. a `usize` above `i64::MAX` that the
original returned) is not out of range: `last` is returned and nothing is counted.

Types that can never be out of range (`()`, `i64`, `u64`, `f64`) do not track `last`.

#### Observe-only mode (`OBSERVE_ONLY`)

When a bridgeable hook returns a type `R` that does not fit an `i64`, the generated dispatcher keeps `R`
on the host side:

```rust
fn __bridge_N(original: &mut dyn FnMut(&mut T, A) -> R, p0: &mut T, p1: A) -> R {
    let mut result: Option<R> = None;
    let _ = yoake_slice::runtime::dispatch_id(N, p0, &TABLE, &mut |p0| {
        if result.is_none() {
            result = Some(original(p0, p1));
        }
        0
    });
    match result {
        Some(value) => value,
        // The guest did not call `original`: the result cannot be produced by the guest, so the original runs.
        None => original(p0, p1),
    }
}
```

Semantics:
- the guest sees the hook and its `arg0` handle and may call `original`, but cannot replace the result (its `i64`
  return value is ignored);
- the real original runs **exactly once** per call: the first `original` that reaches it (through the chain) runs it
  and its result is kept; a later `original` that reaches it again returns `0` without running it (no duplicate side
  effects, e.g. call site 215 hurts the item once);
- if no `original` reaches it, it runs once after the guests return, so the hooked fn cannot be cancelled in this
  mode.

## Changes needed at merge time

1. `yoake2/slice` tests and the bench example call `yoake_hook_rt::register::<SettleCallback>(ArmorStand::
   __YOAKE_HOOK_ID_tick_settle, ..)`. On yoake2/ids round 2 that API became the hidden `__register_typed`, and the
   typed `__yoake_register_<fn>` is `pub(crate)` (SF6), so these integration tests no longer compile. They should
   register through the bridge's `install` (or a test-only `pub fn` in the crate behind `yoake2-slice`, as
   `entities::hook_bench` does for the bench). The bench example on yoake2/callsite already uses
   `entities::hook_bench::register_noop_tick_settle()`.
2. `tick_settle` now has call-site rewrites in other callers but none inside it (Phase 1d marks it hot-path), so
   MX3V1's numbers for it stay comparable.
3. Textual conflicts to expect (both branches change them):
   - `crates/horizon-client/src/entities.rs`: yoake2/slice appends `include!("yoake_glue/entities.rs")` under
     `#[cfg(feature = "yoake2-slice")]` at the end; yoake2/callsite appends the generated call-site shim block at the
     end. Keep both; the codemod treats the include as an opaque macro and only manages its own block.
   - `Cargo.toml` (workspace members), `Cargo.lock`, `crates/horizon-client-lib/Cargo.toml`,
     `crates/horizon-client/Cargo.toml` (features and dependencies on both sides), `crates/horizon-client/src/game.rs`
     (yoake2/slice adds a test module, yoake2/callsite makes `Game` `pub(crate)` and adds six
     `#[allow(private_interfaces)]`): additive, in different places.
4. The generated glue under `src/yoake_glue/` is compiled only with `yoake2-slice`; THIR runs on default features,
   so call sites inside the glue are never candidates. If the glue is ever compiled by default, its calls are
   macro-expanded (`include!`) and still excluded.

## Open questions

- Several mods on one hook: answered. One dispatcher per id; the runtime's chain orders the guests (see above).
- Arguments 1.. as guest-visible handles need `dispatch2` (exists for two args) or an n-ary dispatch; the generator
  can pick `dispatch2` when the second parameter also has a field table.
