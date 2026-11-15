# Yoake WASM development: GPU host foundation

## Availability

This checkout does not contain `crates/yoake-wasm`, `crates/yoake-wasm-sdk`,
`crates/yoake-wasm/wit`, or a WASM host/catalog generator. Its existing mod API is
the native API described in [native-mods.md](native-mods.md). Consequently, **WASM
mods cannot call a GPU API in this checkout**. There is no new `gpu` WIT import,
SDK, or working `gpu-demo`; no frozen world has been modified.

The W2G changes introduce an independently tested host-policy foundation in
`horizon-client-render::gfx::{mod_gpu, mod_gpu_shader}`. These modules are not
connected to mod frame hooks, renderer stages, or production GPU submission.
They must not be advertised as a completed sandbox or a driver-hang guarantee.
See [yoake-gpu-coverage.md](yoake-gpu-coverage.md) for the implementation boundary.

## Resource ownership and accounting

`mod_gpu::Resources<Value>` is a host-only registry. The host supplies the calling
mod's `ModId`; this identity must never come from guest arguments. Handles are
nonzero `u32` values, monotonically allocated without reuse. Lookup, typed lookup,
and destruction reject foreign, unknown, wrong-kind, and destroyed handles with
errors. Removing a mod's handles does not invalidate an already retained resource.
Allocation failure or an unwinding resource factory releases its reservation.

Use **one registry for all mods and resource kinds**, with a host-owned resource
enum as `Value`. Separate registries have independent budgets and handle spaces;
they do not enforce a combined budget. Preserve the handle sequence when the
renderer/device is recreated, or replace the guest handle namespace before
accepting calls again. Constructing a fresh registry while guests retain old
handles could alias those handles.

Each resource carries a reference-counted accounting lease. Recorded commands,
bind groups, and pending readbacks must retain their dependencies, including until
GPU completion. Destroying a public handle does not refund those retained leases.
Dropping the last reference refunds the allocation and handle charges. The
foundation does not implement submission-completion tracking for its caller.

| Limit | Host policy |
| --- | --- |
| Per-mod accounted memory | 256 MiB |
| Combined accounted memory | 1 GiB within the shared registry |
| Live allocations per mod | 4,096, including retained destroyed handles |
| Buffer size | 1 byte through 256 MiB |
| 2D texture dimensions | 1 through 8,192 on each axis |
| 2D array layers | 1 through 256 |
| 3D texture dimensions | 1 through 512 on all three axes |
| Shader source | 1 byte through 256 KiB |
| One readback allocation | 1 byte through 1 MiB, charged twice for staging and result |

Texture charges include every mip, array layer, and format's bytes per texel.
Supported format descriptors are RGBA8 unorm/sRGB, RGBA16F, RGBA32F, R32F, RG16F,
and Depth32F. The accounting rounds every allocation up to 64 KiB and charges at
least 64 KiB for samplers, pipelines, and bind groups. These are conservative
bookkeeping charges, **not a measurement or proof of actual driver VRAM usage**.
Adapter limits, format support, usage flags, buffer offsets/alignment, texture
copy regions/row layout, pipeline layouts, and bind group compatibility still need
validation in the concrete GPU service before allocation or recording.

## Frame recording and stages

The host-only `Recorder<Command>` accepts records only between `begin_frame` and
`finish_frame`. It checks a proposed command before constructing its payload;
failed construction does not spend the frame budget. Every successful record
costs one command, even if it performs zero GPU work.

| Per-mod, per-frame limit | Value |
| --- | --- |
| Commands of all kinds combined | 4,096 |
| Draw calls | 4,096 |
| Draw vertices/indices multiplied by instances | 2^24 |
| Compute workgroups, product of dispatch dimensions | 65,536 |
| Upload bytes | 16 MiB |
| Readback bytes | 1 MiB |

The combined command limit is intentionally stricter than the separate draw
limit. Counters use checked arithmetic and reject a whole over-budget record.
The empty recorder does not allocate backing storage or emit commands. Its stage
order is `world-opaque`, `world-translucent`, `after-world`, `hud`; within each stage
records sort by host-assigned mod load order, then retain submission order.
These are **ordering rules only**, not actual render passes. There is no draw,
render-to-texture, dispatch, write, or asynchronous ticket API exposed to guests.

`engine_binding` rejects writable engine bindings. It reserves frame data, block
atlas, atlas sampler, world depth, scene color, and lightmap identities. In this
foundation, both world depth and scene color are restricted to `after-world`;
depth sampling for world-translucent soft particles is not implemented.

The guarded shader validator provisionally reserves bind group 3: binding 0 for
a uniform, 1 for the block atlas, 2 for its sampler, 3 for world depth, 4 for scene
color, and 5 for the lightmap. It rejects writable or incompatible engine resource
declarations. This is **not a published ABI**: no actual engine views are bound,
no exact uniform layout is checked, and no versioned `yoake_frame` struct exists
yet. Camera matrices, previous frame/TAA jitter, clocks, celestial direction, and
fog must be specified together with that ABI before guest access is enabled.

## Guarded WGSL

`mod_gpu_shader::validate` parses WGSL with Naga, rewrites every loop in every
helper and entry point, revalidates the resulting IR, and applies W2-C's existing
call-graph cost analysis. A successful result owns the rewritten Naga module.
The future GPU service must create shaders from **that module**, never from the
original WGSL string, and must retain wgpu's bounds-check robustness.

Every loop activation gets its own zeroed counter. A check at the beginning of
the body breaks before iteration 257: at most **256 body iterations** run per
activation. The counter increments before guest code, so `continue` cannot bypass
it. Counters reset on repeated helper calls/loop activations, and the original
continuing block and `break_if` remain attached to the loop. This bound is stricter
than 65,536 so useful small loops can fit the existing cost budget. A nested loop
or expensive loop may be rejected even though it has been guarded.

The shared static limits are 8,192 expression-equivalents, 64 image fetches, and
call depth 16 per invocation. Costs conservatively include loop repetition and
repeated calls. Further limits are 64 helper functions, 16 entry points, 4,096
expressions per function before rewriting, 256 invocations per compute workgroup,
16 KiB per type, and 16 KiB total private global storage. Pipeline overrides,
workgroup memory, barriers, subgroup operations, and ray queries are rejected.
This avoids override-based workgroup-limit bypass and divergent guarded-loop
collective synchronization. Workgroup-based compute algorithms are therefore not
supported by this foundation.

Parse/IR validation failures include source line and column where Naga provides
them. Policy-only errors use line/column zero, meaning no single source location.
There is no file watcher or development-folder shader hot reload yet.

## Integration still required

Restore or identify the intended WASM runtime, generated catalog and SDK before
adding a `gpu` interface exclusively to `client-full`. Then implement concrete
resources and command descriptors, validated bindings/copies, frame-hook routing,
stage-specific isolated encoders, and asynchronous readback tickets. Reuse W2-C's
`native_services::checked` error scopes and mod-encoder machinery rather than
introducing an independent submission path.

Per-mod timestamp measurement and the 4 ms/2 s enforcement window, persisted
pre-submit culprit recording/device-loss quarantine, user notices, renderer
recreation, hot reload, a stable frame uniform ABI, and the SDK/demo remain
unimplemented. CPU caps and bounded shader loops alone cannot guarantee the
absence of a driver hang, GPU denial of service, or a crash loop. No production
renderer pass, target allocation, or timestamp query is added by this foundation.
