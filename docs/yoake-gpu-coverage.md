# W2G GPU coverage

This file tracks the host foundation, not a shipped WASM GPU API. The WASM
runtime/catalog/SDK cited by the W2G task are absent from this checkout. There was
no existing WASM coverage file to extend; this is the GPU-specific coverage index.

| Requirement | Status | Implementation / tests |
| --- | --- | --- |
| `gpu` WIT in `client-full` only | Not implemented; required runtime/catalog absent | No WIT or frozen-world changes |
| Owner-checked opaque handles; destroy; stale handles | Host foundation | `gfx/mod_gpu.rs`; `handles_enforce_owner_kind_and_lifetime`, `handles_and_numeric_exhaustion_fail_closed` |
| Failed creation/unwind and retained dependencies | Host foundation | `reservation_failure_and_unwind_release_budgets`, `retained_commands_and_bindings_keep_memory_charged` |
| Per-mod/global memory and handle caps | Host accounting only; not actual driver VRAM | `memory_caps_apply_per_mod_and_across_mods`, `handles_and_numeric_exhaustion_fail_closed` |
| Buffer/texture/shader descriptor size caps | Host foundation | `allocation_sizes_include_mips_layers_and_format`, `texture_dimensions_and_mip_counts_are_bounded`, shader source tests |
| Concrete GPU buffers/textures/samplers/pipelines/bind groups | Not implemented | Generic registry does not create wgpu resources |
| Upload/copy/layout/binding validation | Not implemented beyond byte accounting | Concrete resource descriptors and execution required |
| Frame-only command recording and all frame caps | Host foundation | `frame_limits_are_transactional_and_include_instances`, `draw_and_command_counts_are_bounded_independently_per_mod`, `record_only_in_frame_and_reject_before_constructing_commands` |
| Stage/load/submission order | Host policy only | `commands_follow_stage_then_mod_load_then_submission_order` |
| Production render stages, draw-to and compute execution | Not implemented | No new passes or encoder submission |
| Read-only engine resource/stage restrictions | Host declaration/policy validation only | `engine_resources_are_read_only_and_screen_inputs_are_stage_restricted`, `rejects_engine_writes_and_unguarded_collectives` |
| Stable versioned `yoake_frame` | Not implemented | Must agree layout before enabling guest access |
| Naga IR loop guard and cost validation reuse | Implemented, independently callable | `gfx/mod_gpu_shader.rs`, shared `native_post_shader::validate_cost` |
| Too-big/invalid WGSL and source locations | Implemented | `bounds_source_and_reports_locations` |
| Infinite loop, continue, helper activation reset, normal continuing | CPU and real headless GPU tests | `guards_infinite_loops_and_continues_in_helpers`, `guarded_compute_dispatch_reads_back_exact_iteration_bound` |
| Bounded loop call-graph cost limits | Implemented | `bounded_loops_still_have_call_graph_cost_limits` |
| Workgroup size override bypass / unsupported collectives | Fail closed | `rejects_engine_writes_and_unguarded_collectives` |
| Async readback byte/storage bounds | Host policy/accounting only | `pending_readback_storage_is_bounded_and_retained`; no guest tickets/polling |
| Per-mod GPU timestamps and time-budget suspension | Not implemented | CPU caps do not substitute for GPU-time enforcement |
| Device-loss culprit persistence and next-start quarantine | Not implemented | No new submission path or persistence |
| No-mod idle behavior | Policy unit test; production wiring unchanged | `idle_recorder_does_not_allocate_or_emit_commands`, existing native no-registration test |
| Shader file hot reload | Not implemented | No WASM development folder/runtime |
| Safe WASM SDK and textured-mesh/particle/depth example | Not implemented | SDK/runtime absent |

## Focused verification

All tests listed above live in `crates/horizon-client-render/src/gfx/mod_gpu_tests.rs`
or `mod_gpu_shader.rs`. The headless test reuses the existing native GPU test setup
and `native_services::checked`; it dispatches rewritten Naga IR, copies to a
mapped staging buffer, and verifies exact results for three shaders. It is not
a WASM-to-renderer end-to-end test. Like the existing setup, it returns early if
there is no usable adapter; successful execution on an adapter is needed to
validate the GPU assertions.

Run offline, without starting the game or launcher:

```sh
cargo check --offline -j 4 -p horizon-client-render
cargo test --offline -j 4 -p horizon-client-render --lib gfx::mod_gpu -- --nocapture
cargo test --offline -j 4 -p horizon-client-render --lib gfx::native_post_shader::tests:: -- --nocapture
cargo test --offline -j 4 -p horizon-client-render --lib gfx::native_services_tests:: -- --nocapture
```

These filtered tests need no prepared Minecraft content. They do not establish
bit-identical full-game output, actual VRAM use, timestamp enforcement, or device
loss recovery. Those remain integration and platform-test obligations.
