# W2G GPU coverage

The generated `yoake:mods@1.0.0` `client-full` world exposes the host-owned GPU
API behind `service.gpu`. The frozen `client-mod` 1.0 world is unchanged. The
API, frame ABI, stage rules, limits and development workflow are documented in
[yoake-wasm-dev.md](yoake-wasm-dev.md#client-gpu-api).

| Requirement | Implementation / observable coverage |
| --- | --- |
| W2-C services forwarded through generated imports | `abi-subset-client.json`, `wasm_mods/client_services.rs`, `client-services-demo`; real guest imports exercise key binding, world line, shader registration/rejection and GUI texture write bounds |
| Client-only GPU WIT and safe SDK | `abi-subset-gpu.json` IDs 146–162, generated `yoake-full.wit` / adapters, `yoake-wasm-sdk/src/gpu.rs`; generator snapshots and real guest imports |
| Owner, kind and lifetime validation | `gfx/mod_gpu.rs`; `handles_enforce_owner_kind_and_lifetime`, `handles_and_numeric_exhaustion_fail_closed` |
| Failed creation, retained dependencies and memory caps | `reservation_failure_and_unwind_release_budgets`, `retained_commands_and_bindings_keep_memory_charged`, `memory_caps_apply_per_mod_and_across_mods` |
| Concrete buffers, textures, samplers, shaders, pipelines and bind groups | `gfx/mod_gpu_resources.rs` and its `commands.rs` / `pipelines.rs`; descriptor, usage, range, layout and ownership validation before wgpu execution |
| Texture dimensions, mips, layers and format accounting | `allocation_sizes_include_mips_layers_and_format`, `texture_dimensions_and_mip_counts_are_bounded`; real texture upload and sampled mesh in the GPU guest |
| Frame-only recording and transactional caps | `frame_limits_are_transactional_and_include_instances`, `draw_and_command_counts_are_bounded_independently_per_mod`, `record_only_in_frame_and_reject_before_constructing_commands`; guest rejects oversized buffer, out-of-range write and excessive dispatch |
| Stage, load and submission order | `commands_follow_stage_then_mod_load_then_submission_order`; production `gfx.rs` submits at opaque, translucent, after-world and HUD boundaries |
| Rendering, compute and offscreen targets | `gfx/mod_gpu_runtime.rs`, concrete command execution; GPU guest renders a textured mesh, compute-driven particles and a depth-based effect into headless targets and checks readback pixels |
| Read-only engine resources and stage restrictions | `gfx/mod_gpu_resources/engine.rs`; `engine_resources_are_read_only_and_screen_inputs_are_stage_restricted`, `rejects_engine_writes_and_unguarded_collectives` |
| Versioned 576-byte frame ABI | `gfx/mod_gpu_runtime.rs`, SDK `ENGINE_WGSL`; layout and frame-input tests cover version, size, matrices, camera, time, fog, celestial direction and jitter |
| Bounded validated shaders | Shared W2-C call-graph cost validator and `gfx/mod_gpu_shader.rs` Naga rewrite; `bounds_source_and_reports_locations`, `guards_infinite_loops_and_continues_in_helpers`, `bounded_loops_still_have_call_graph_cost_limits` |
| Real GPU loop-guard execution | `guarded_compute_dispatch_reads_back_exact_iteration_bound` runs rewritten Naga shaders and checks the exact bounded result |
| Bounded asynchronous readback | Retained, charged staging buffers and polling tickets; `pending_readback_storage_is_bounded_and_retained` plus guest compute result readback |
| Per-mod timestamp suspension | `gfx/mod_gpu_resources/timing.rs`; feature-gated queries and bounded pending samples, `timestamp_budget_uses_gpu_samples_not_cpu_elapsed_time` |
| In-flight persistence and next-start quarantine | `gfx/mod_gpu_runtime/recovery.rs`; `incomplete_work_blocks_stable_identity_on_restart`, `completion_waits_for_every_submission_and_preserves_other_owners`, `malformed_marker_fails_closed` |
| Renderer recreation, traps and lost permission | `wasm_mods/full/gpu_tests.rs`; `gpu_guests_render_compute_reload_trap_and_device_loss` checks resource teardown/recreation, trapped guest isolation and disabled GPU access |
| No-mod idle path | `idle_recorder_does_not_allocate_or_emit_commands`, existing native no-registration baseline; renderer avoids GPU-stage allocations and submissions until a mod uses the API |
| Confined development shader overrides | `wasm_mods/dev_shaders.rs`; `paths_are_confined_and_only_valid_stable_edits_reload`, real guest reload selects a new component while keeping invalid edits out |
| SDK example | `yoake-wasm-sdk/examples/gpu-demo`: world-opaque textured mesh, world-translucent compute particles, after-world depth tint; verification variants test reload and traps |

## Verification

Run offline without starting the game or launcher. Set the indicated variables
only for their commands; preserve the existing target directory and build
configuration.

```sh
YOAKE_UPDATE_WIT=1 cargo test --offline -j 4 -p horizon-mod-api-gen
cargo test --offline -j 4 -p horizon-client-render --lib gfx:: -- --test-threads=1
YOAKE_REQUIRE_GUESTS=1 cargo test --offline -j 4 -p horizon-client --lib wasm_mods:: -- --test-threads=1
YOAKE_REQUIRE_GUESTS=1 cargo test --offline -j 4 -p yoake-wasm
cargo check --offline -j 4 -p horizon-client
```

`YOAKE_REQUIRE_GUESTS=1` makes missing guest builds and unavailable headless GPU
adapters fail the relevant integration tests instead of silently skipping their
assertions. The guest builder uses offline release builds for `wasm32-wasip2`,
including the GPU verification, reload and trap variants.

## Boundaries / platform confirmation

- Memory budgets account for rounded logical resource sizes, retained bindings,
  staging buffers and engine copies, not driver-reported physical VRAM.
- Timestamp enforcement needs adapter support. Its mean uses GPU samples in a
  fixed two-second window, not a rolling window or CPU elapsed time. Unsupported
  adapters retain the CPU-side command, memory and shader caps only.
- Recovery tests exercise persistence, quarantine, callbacks and renderer
  recreation. They do not force a physical device/driver loss. An interrupted
  process can leave an in-flight marker conservatively; it is not proof that a
  mod caused a driver fault.
- Invalid WGSL overrides keep the previous shader/component. A valid shader that
  violates a pipeline interface can still fail resource creation and disable the
  mod; this is not a transactional live pipeline editor.
- The filtered renderer suite retains its pre-existing ignored GPU benchmarks,
  timing probes and content/adapter-dependent checks. These are not replaced by
  the new headless guest tests.
- Headless checks need no prepared Minecraft content. Full-game visual parity,
  platform-specific driver behavior and long-running GPU timing/recovery still
  need owner verification; no game or launcher run is part of this verification.
