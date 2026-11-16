# Yoake recipes

These are starting points, not promises that every feature is implemented. The
[Wasm guide](yoake-wasm-dev.md) describes the client and full worlds, available
services, permissions and sandbox limits; the [API index](yoake-api-index.md)
points to the exact WIT and hook catalog. Handle service errors, and keep
server-authoritative state on the server. SDK test worlds currently require a
caller-provided driver; there is no engine test runner yet.

## Projects and client hooks

### 1. Start a project

Run `yoake new utility my-mod`, edit its author and description, then run
`yoake build` and `yoake check` inside the new directory. The generated project
includes the SDK and WIT definitions; `YOAKE_SDK_PATH` optionally selects a local
SDK checkout instead. `check` runs `init` against stub hosts and validates its
subscriptions, but does not run the mod in a game.

### 2. Run a timer

Subscribe to `hook::TICK`, increment a counter in `Mod::tick`, and trigger an
action when it reaches the desired tick count. Reset it after firing; use
`Mod::frame` only when elapsed frame time, rather than game ticks, is intended.

### 3. Draw HUD text

Subscribe to `hook::HUD` and draw in `Mod::hud`:

```rust
let _ = yoake_wasm_sdk::hud::text("Hello", (4.0, 4.0), 0xffffff, true);
```

### 4. Draw a HUD background

In the same hook, draw a rectangle before its label:

```rust
let _ = yoake_wasm_sdk::hud::rect([4.0, 4.0, 100.0, 20.0], 0x88000000);
```

Check the color and bounds conventions in the SDK before styling a real HUD.

### 5. Log diagnostics

Use `yoake_wasm_sdk::log("my-mod", "initialized")` for diagnostics rather
than storing secrets or player data in logs. Check its `Result` when reporting
an error matters.

### 6. Give client feedback

Use `yoake_wasm_sdk::feedback(message)` for a client-facing message and handle
the result. Real mods should localize user-visible messages.

### 7. Save a client setting

Use `settings::get("show-hud")` and `settings::set("show-hud", "true")`.
Provide a default when the key is absent and handle storage errors.

### 8. Read the local player

Call `local_player()` and handle both an error and `None`; inspect the returned
snapshot only when a player is present. Do not use a client snapshot as
authoritative multiplayer state.

### 9. Detect the current world

Call `world_key()` and handle `None` when no world is active. Reset
world-specific client state when the key changes.

### 10. Read a world snapshot

Call `snapshot_world()` for the snapshot the host exposes, and handle errors
when the context is unavailable. Read the WIT record for its actual fields.

### 11. Respond to a key

Subscribe to `hook::KEY`; in `Mod::key`, act only when `pressed` is true and
the key matches the one your mod expects. Avoid assuming a particular layout.

### 12. Open a mod screen

Call `screen::open(mod_id)` to request the screen for a mod; inspect the
returned `bool` and error. Use `screen::close(mod_id)` to close it. Defining
the actual screen requires the supported screen hooks, not just this call.

### 13. Send a client packet

Call `net_send(channel, payload)` with a small, versioned payload and handle
`SendError`. Define and validate the channel and packet format on both sides.

### 14. Receive server state

Subscribe to `hook::NET_FROM_SERVER` and implement `Mod::net_from_server`.
Match your channel, validate the bytes and reject unknown versions before
updating client display state.

## Server-side patterns

The following require a server/full-world component and the appropriate hooks;
they are not implemented by the client-only project templates. Consult the
[server services](../crates/yoake-wasm/wit/yoake-full.wit#L1924) and the
[implementation notes](yoake-wasm-dev.md) for support.

### 15. Sync data to a player

Encode a versioned payload on the server and use `net-send-to-player`; use
recipe 14 to receive it. Re-send authoritative state on join or when it changes.

### 16. Store world-specific data

Use `world-storage-read` and `world-storage-write` with a mod-owned key.
Version the bytes and handle missing or malformed data during upgrades.

### 17. Keep server configuration

Use `config-read` and `config-write` for a small versioned settings document.
Validate every value and supply defaults when there is no saved configuration.

### 18. Read a game resource

Use `asset-read` with a valid relative key to read `data/<key>` from the game's
resources. Package assets are collected by the packer, but this service does
not expose them. Treat a missing resource or rejected path as an error.

### 19. Change a player's inventory

Construct the WIT `inventory-change` for the desired operation and submit it
with `inventory-apply`. Check both errors and the returned success flag.

### 20. Build a team game

Use `team-apply` for validated team changes and a server tick hook for round
timing. Sync the current round to clients using recipe 15; clean up team state
when the round ends.

### 21. Change time or a game rule

Choose the corresponding typed `world-control-change` and call
`world-control`. Check the outcome and only perform the change while your game
mode is active.

### 22. Spawn particles

Construct a WIT `particle-request` with bounded count and coordinates, then
call `particle-spawn`. Handle a rejected request rather than retrying every tick.

### 23. Spawn a mannequin

Use `spawn-mannequin` with a player, pose, position and yaw. The result may
contain no entity; keep a returned ID and use `remove-spawned-entity` for
cleanup. A mannequin is not a general custom-mob registration API.

### 24. Prototype a custom mob

Run `yoake new custom-mob my-mob` and replace the data and model placeholders.
The template does not register an entity: verify the engine's registration and
rendering support before treating it as a working mob.

### 25. Prototype custom block behavior

Describe the block in the mod's content data and select an appropriate block
hook from the catalog, such as `tick-block`. Use `block`,
`set-block-property` or `edit-blocks` only with supported values and check
their outcomes. Confirm content registration and hook support in the engine.

### 26. Run an approved game command

Declare `permissions = ["service.commands"]` under `[wasm]`, then use
`run-command` for a validated game command. The permission and host denylist
still apply; never interpolate untrusted arguments into a command string.

### 27. Track a score

Keep a score in server-owned state, persist it with recipe 16 and send it with
recipe 15. If integration with the vanilla scoreboard is needed, first verify
the available scoreboard hooks and services instead of assuming a write API.

### 28. Plan an HTTP integration

Do not import WASI sockets: the current sandbox rejects them. Record the
required endpoint, permissions and behavior in `WASM-NEEDS.md`; use only a
future, explicitly approved host service if one becomes available.

## Tests and iteration

### 29. Test a world operation

Mark a function `#[yoake_test]`, accept `&mut TestWorld<'_>`, and return
`TestResult`. Call `place_block`, `run_ticks`, and `assert_block`, then add the
function to the explicit `TestCase` registry. A unit test can run that case
against a mock `TestDriver`; this does not yet run a real game world.

### 30. Iterate with developer reload

Set `YOAKE_WASM_DEV_DIR` to the raw-Wasm watch directory and arrange for
`<id>.wasm` to exist when a developer-mode game starts. `yoake dev` builds and
checks the mod, then atomically updates that raw file; it does not pack. A
`client-full` or server-only mod, or an unset `YOAKE_WASM_DEV_DIR`, succeeds
without installing and reports a note in its JSON result. The offscreen runner
is not implemented, and installation does not prove the game accepted a reload.
