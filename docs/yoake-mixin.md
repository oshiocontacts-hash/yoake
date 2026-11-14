# Yoake injection: prototype status

This is an experimental, disconnected foundation, not an available mod feature.
No gameplay functions are annotated, and there is no Wasmtime adapter or
tick-boundary publication. The draft site catalog is empty. Manifests with a
`[mixin]` section are validated and then rejected until injection is supported. The existing
1.0 WASM interface and the unmodified game behavior are unchanged. Mod authors
cannot yet install or target these prototype sites.

## Prototype site definitions

The `yoake-inject-macro` crate currently accepts free functions with named,
fixed-width scalar, owned string, or byte-vector arguments and returns:

```rust
#[yoake_inject(id = "example::calculate", side = "server", allow = "head,return")]
fn calculate(value: i32) -> i32 {
    value + 1
}
```

The owning crate must depend on both `yoake-inject-macro` and
`yoake-inject-rt`. `id` defaults to the module path and function name; `allow`
defaults to all three injection points. The prototype additionally accepts
`side = "client"` or `side = "server"`. An omitted side is unspecified and
the runtime refuses to register the site: choose a side explicitly until a
catalog can assign and validate one. There is no automatic site discovery.

The site ID is the path followed by `#` and the lowercase SHA-256 digest of
`yoake-inject-v1:(<comma-separated argument WIT shapes>)-><return WIT shape>`.
For example, the shape for an `i32` argument and return is `s32` in both
positions. This is a prototype contract: a signature change produces a new ID,
and the registry distinguishes a known path with a different hash from an
unknown path. Engine-version compatibility checks are not implemented.

The source catalog generator scans `horizon-world/src`, `horizon-items/src`,
`horizon-client/src/game.rs`, and `horizon-client/src/game/`. For every
cataloged site it requires an explicit `id` and `side`, rejects duplicate
paths and unsupported signatures, and emits the ID, signature hash, WIT
shapes, allowed points, side, and source path. Regenerate with
`cargo run -p horizon-mod-api-gen --example inject_catalog`; its committed-file
test checks `crates/yoake-wasm/wit/inject-sites.json`. It is only a draft:
it rejects explicitly conditional sites and cannot yet reliably account for
conditional out-of-line modules, build features, or a compiled module graph.
It must not be treated as a definitive list of loadable sites.

Supported native shapes are `()`, `bool`, fixed-width signed and unsigned
integers through 64 bits, `f32`/`f64`, `String`, and `Vec<u8>`. The macro rejects async, const,
unsafe, extern, generic and variadic functions, raw pointers, references,
methods with a receiver, complex patterns, and all other types. Scoped engine
handles, other collections, `Result`/`?`, and method support remain
to be designed and implemented. Associated functions are not supported. For
the supported subset the expansion keeps an early `return` inside the original
body; the integration test is not a substitute for the missing control-flow,
drop and borrowing coverage.

## Shared value model and WIT draft

`yoake-value` defines a tagged representation shared in principle with the
state-accessor work: unit, boolean, signed and unsigned integers, float, text,
bytes, list and record. Site metadata keeps the narrower numeric shape and
checks replacement values against it. The separate M3A worktree still has its
own value type and has not yet been migrated to this crate.

`crates/yoake-wasm/wit-inject/yoake-inject.wit` is a draft 1.1 injection
interface. WIT has no recursive value variant in this design, so list and
record values are represented as a node table with indices. The standalone
`yoake-value::wire` encoder and decoder apply a caller-supplied cumulative
node, depth and byte budget per callback, with a hard depth limit of 64. They
reject invalid indices, repeated or unreachable nodes, mismatched root kinds,
and duplicate record keys. Create a new encoder or decoder for each callback
and discard it after any error. The code has not been verified against real
component bindings; allocation performed by those bindings would also need
limits before this validation runs. The draft is not wired into the current
host or guest bindings and does not make 1.1 mods loadable.

## Standalone dispatch behavior

The registry validates a candidate snapshot before publication. It rejects
duplicate subscriptions, unsupported points, wrong sides and multiple
overwrites at one site. A policy callback is retained for a future
authorization layer; there is no user-facing permission prompt. Callbacks
are currently ordered lexicographically by mod ID. HEAD can replace all
arguments or cancel with a result; overwrite can replace the original; RETURN
can replace the result. Disabled mods are skipped.

A reported callback error or a type-invalid response disables the mod. If the
original has not run, the dispatcher calls it with the original arguments.
If it has run, a RETURN failure preserves its native result rather than
executing it again. Neither path rolls back guest or engine side effects, and
the behavior after modified arguments and a RETURN failure needs a documented
integration policy.

The generated zero-mask branch performs one relaxed atomic load before
entering the untouched original body; it does not construct value snapshots or
acquire runtime locks on that path. No overhead measurement has been made. On
the active path the site snapshots handlers before calling guests.
`Registry::publish` requires a caller-arranged quiescent point across all
readers, including the integrated-server and client threads. That coordination
is not implemented.

## Remaining work

Implement scoped handles and safe methods and compound types, migrate M3A to
the common value model, connect and validate the WIT conversion, make the
catalog reflect the compiled module graph, annotate gameplay, enable manifest
and version checks, wire the WASM host and trap handling, coordinate
publication, run the tests, and measure the no-mod fast path before exposing
the feature.
