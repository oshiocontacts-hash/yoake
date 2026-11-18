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

`OUT_DIR/yoake_slice_bridge.rs` is written by the crate's build.rs (the same `yoake_hook_ledger::build` step that
writes KEYS) from the shard plus the codemod's per-hook signature record. For every live id N whose shape is
bridgeable it contains a dispatcher and an installer:

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

pub fn install(registered: &[(u32, i32)]) -> Vec<u32> {      // returns ids it could not bridge
    let mut unbridged = Vec::new();
    for &(id, priority) in registered {
        let order = yoake_hook_rt::Order { priority, mod_load_order: 0 };
        let ok = match id {
            71 => crate::entities::ArmorStand::__yoake_register_tick_settle(__bridge_71, order).is_ok(),
            // ... one arm per bridgeable id
            _ => false,
        };
        if !ok { unbridged.push(id); }
    }
    unbridged
}
```

The loader calls `install(&runtime::stats().registered)` after `runtime::install` (only ids below
`SYNTHETIC_HOOK_BASE`), so only the slots a guest asked for get a callback; every other slot stays empty and costs one
load. Re-installing a mod needs the S2 hot-reload generation (MX3S2 open problem 3); until then a slot keeps the
first bridge.

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

#### Observe-only mode (`OBSERVE_ONLY`)

When a bridgeable hook returns a type `R` that does not fit an `i64`, the generated dispatcher keeps `R`
on the host side:

```rust
fn __bridge_N(original: &mut dyn FnMut(&mut T, A) -> R, p0: &mut T, p1: A) -> R {
    let mut result: Option<R> = None;
    let _ = yoake_slice::runtime::dispatch_id(N, p0, &TABLE, &mut |p0| {
        result = Some(original(p0, p1));
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
- the guest sees the hook and its `arg0` handle and may call `original` zero or more times, but cannot
  replace the result (its `i64` return value is ignored);
- the **last** `original` result is returned;
- if the guest never calls `original`, `original` runs once after the guest returns, so the hooked fn
  cannot be cancelled in this mode.

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

- Several mods on one hook: the bridge installs one callback per registered id; ordering several guests by priority
  is the loader's job (ENGINE_CALL_ABI says so). With one guest per thread today this is moot.
- Arguments 1.. as guest-visible handles need `dispatch2` (exists for two args) or an n-ary dispatch; the generator
  can pick `dispatch2` when the second parameter also has a field table.
