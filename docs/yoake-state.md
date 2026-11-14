# Yoake state access prototype

## Status

This is native Rust groundwork, **not a WASM mod interface**. It does not generate
WIT records or services, provide guest SDK wrappers, expose handles to mods, or
connect snapshots, writes, or constant overrides to the game or the WASM host.
Mods cannot read or change engine state with it yet. No gameplay constant has
been annotated or routed through the override registry.

The initial catalog describes ten existing structs and 75 explicitly annotated
fields: food data, the block-behavior game-rules snapshot, portal-travel border
bounds, moving piston state, sculk charge cursors, effect details, player
experience, motion and health flags, and basic mob fields. All are read-only.
These structs are not necessarily the authoritative owner of the corresponding
complete player or world state. Inventory, equipment, most type-specific mob
components, most block entities, server settings, and much other requested state
are not covered.

## Native snapshots

`#[yoake_state::yoake_state]` processes named, non-generic structs. Fields are
excluded unless explicitly annotated. `#[yoake_state]` and
`#[yoake_state(read_only)]` expose a field for reading;
`#[yoake_state(skip)]` excludes it. A read-only struct cannot contain writable
fields. Supported field types are `bool`, `i32`, `f32`, and `f64`.

```rust
#[yoake_state::yoake_state]
struct Example {
    #[yoake_state(read_only)]
    ready: bool,
    #[yoake_state(write, min = 0, max = 20)]
    count: i32,
    #[yoake_state(skip)]
    internal: u64,
}
```

The attribute implements `yoake_state::State` and creates native typed getters
such as `yoake_state_ready`. Numeric writable fields require both bounds; boolean
fields cannot have them. These checks are not a substitute for gameplay-specific
cross-field, registry, or lifecycle validation. The generated `set` method
changes the Rust value immediately; callers must not expose it directly to an
untrusted mod or treat it as a scheduled engine mutation. Ordinary `#[cfg]`
attributes are propagated to generated code; `#[cfg_attr]` on a state struct or
an exposed field is not supported.

`State::snapshot` creates a tagged `Value::Record`. `State::get` resolves a path
from that snapshot: an empty path names the root, record components name fields,
and list components are numeric indexes. Path length and component sizes are
bounded. Invalid paths, wrong value types, out-of-range values, and non-finite
floating-point values return errors. Snapshots also return an error if a float
field is non-finite. The tagged `Value` type is a native Rust prototype, not a
published or compatibility-checked WIT type.

## Deferred writes and constants

`PendingWrites` is a bounded native queue for one state type and one resolved
handle. Creating, staging, and applying a queue require a `ResolvedHandle` from
`HandleTable`. Staging validates the target, path, value size, and the
state's `validate_set` method. `apply` rechecks the target and validates each
write against the current state before calling `set`. Writes are applied in
order and report individual results; the batch is not atomic. `discard` clears
the queue.

`HandleTable` can issue native handles for player UUIDs, entity IDs, block and
chunk positions, levels, and the server. It associates handles with an instance,
world generation, and object generation, and supports explicit lifecycle
invalidation. It only checks basic dimension-string size and control characters;
the host must verify registry membership and object existence. Its identifiers
are predictable and are not secrets. Only trusted host code may provide the
instance and generation values, and the host must invalidate entries whenever
the corresponding instance, world, or object changes.

The host must resolve the handle again when staging and applying, compare the
current object generation with `ResolvedHandle::object_generation()`, and
authorize the operation. A resolved handle is a
borrow of the table, so it cannot outlive a table change that invalidates it.
The queue is not connected to a hook or tick safe point,
does not identify an actual game object, and does not enforce mod capabilities.
It must not be used as an authorization boundary. A host integration must resolve
opaque handles in the correct mod, world, dimension, and object generation,
reject stale or unauthorized handles, and commit through gameplay-aware mutation
methods at an agreed safe point.

`#[yoake_state::yoake_const]` can describe a native `bool`, `i32`, `f32`, or
`f64` constant. Numeric constants require finite typed bounds. A
`ConstantOverrides` registry validates definitions and overrides and can create
immutable snapshots or reset overrides. It is independent of the game loop; no
gameplay reads currently consult it, and it has no WASM import, permission model,
or per-world lifecycle. Once integrated, overrides must affect gameplay only
when explicitly requested by a mod, and that intentional change breaks vanilla
parity for the overridden behavior. Without an override, vanilla must remain
unchanged.

## Catalog

`crates/horizon-world/src/yoake_state_catalog.rs` and
`crates/horizon-client/src/yoake_state_catalog.rs` explicitly register the
initial types and build the JSON catalog from their generated descriptors.
`crates/yoake-wasm/wit/state-catalog.json` is the committed catalog; the
`generated_catalog_matches_committed_file` unit test compares it with the
generated value. Adding an annotated struct does not automatically register it.
The catalog is a prototype, not proof of WASM availability or a stable ABI.

In an environment permitted to run Rust tools, regenerate it from the repository
root with `cargo run -q -p horizon-client-lib --bin yoake_state_catalog` and save the
output as UTF-8 to `crates/yoake-wasm/wit/state-catalog.json`. Run the catalog
comparison and the `yoake-state` tests there before relying on the output.

## Remaining integration work

- Agree a versioned WIT value and snapshot contract compatible with existing mods,
  then generate records, services, typed guest wrappers, and the complete catalog.
- Integrate capability-checked, generation-scoped handles and host dispatch
  without exposing host pointers or allowing cross-mod access.
- Identify authoritative state and gameplay mutators, including semantic and
  cross-field checks, before enabling each write at the chosen safe point.
- Expand the opted-in state set and introduce explicitly selected gameplay
  constants with mod-scoped override ownership, reset, and tick snapshot rules.
- Verify compilation, generated files, compatibility, and vanilla behavior in a
  Rust- and Minecraft-equipped environment.
