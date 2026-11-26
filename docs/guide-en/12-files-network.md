# 12. Files, Network, and Workers

This chapter explains how mods read and write files and use the network. Because Yoake mods run inside a small trusted group, mods can use files and the network just like Forge or Fabric mods.

Two rules stay fixed:

- When no mod is installed, the game does not change at all.
- **The game's main thread never waits for a mod.**

---

## 12-1. Files (`/data`, `/game`, `/host`)

Mods see the following folders:

| Path seen by the mod | Real folder on disk | Notes |
|---|---|---|
| `/data` | `<game dir>/mod-data/<mod id>/` | Private to this mod. Created automatically on first start. Read/write. |
| `/game` | The instance's game folder | Read/write. |
| `/host/<drive letter>` | PC drive root, e.g. `/host/c` (`/host/root` on Unix) | Read/write, **only** when `[wasm] host_filesystem = true` in `yak.toml`. |

Save your mod's data as ordinary files under `/data`. Standard Rust `std::fs` works directly:

```rust
std::fs::write("/data/score.txt", "42")?;
let text = std::fs::read_to_string("/data/score.txt")?;
```

> [!NOTE]
> File calls count toward your hook's time budget. When reading or writing large files, use a worker as described below.

---

## 12-2. Network

Network calls can take seconds to finish. Running them on the main mod instance would freeze the game. For that reason, **all networking goes in a worker**. Trying to open a socket or send an HTTP request from the main mod instance returns an access-denied error.

---

## 12-3. Workers (`worker-mod`)

A worker is a second WebAssembly component that runs behind your mod on its own dedicated OS thread. Because it has its own thread, waiting for a server will not freeze the game. Inside a worker, you can use:

- Files (`/data`, `/game`, and `/host` if enabled)
- TCP, UDP, and DNS (`std::net`, `wasi:sockets`)
- Outgoing HTTP (`wasi:http`)
- Blocking waits (`std::thread::sleep`, etc.)

A worker cannot touch the game world or engine handles directly. It talks to the main mod only through byte-slice messages.

### Scaffolding and `yak.toml`

You can create a ready-to-build server + worker project with:

```sh
yoake new worker my-worker
```

In `yak.toml`, declare the worker component under `[wasm]`:

```toml
[wasm]
interface = "1.7"
server = "bin/wasm/my-worker.wasm"
worker = "bin/wasm/my-worker-worker.wasm"
```

### Sending and receiving messages

| Who | Send | Receive |
|---|---|---|
| Main mod (`server-full` / `client-full`) | `worker::send(bytes)` (`worker-send`) | `worker::recv()` (`worker-recv`, non-blocking, `None` if empty) |
| Worker (`yoake_wasm_sdk::worker`) | `worker::send(&bytes)` (`host-send`) | `worker::recv()` (`host-recv`, blocks until message) / `worker::try_recv()` (`host-try-recv`, non-blocking) |

In Rust with `yoake_wasm_sdk::worker` (Cargo feature `worker`), implement `Worker` and export it with `export_worker!`:

```rust
use yoake_wasm_sdk::worker::{self, Worker};

#[derive(Default)]
struct MyWorker;

impl Worker for MyWorker {
    fn run(&mut self) {
        while let Some(message) = worker::recv() {
            // Inspect `message`, do HTTP/TCP/file work, and send back a reply:
            let _ = worker::send(b"done");
        }
    }
}

yoake_wasm_sdk::export_worker!(MyWorker);
```

And on the server mod side (`yoake_wasm_sdk::full::server::worker`):

```rust
fn init(&mut self) {
    let _ = worker::send(b"ping".to_vec());
}

fn tick(&mut self) {
    while let Some(reply) = worker::recv() {
        let _ = yoake_wasm_sdk::log("my-worker", &String::from_utf8_lossy(&reply));
    }
}
```

> [!TIP]
> Full-access core mods (`world = "full-access"`) can also pair with a `worker-mod` component using `yoake_guest_sdk::worker::{send, recv}`. See [`crates/yoake-guest-sdk/examples/f1-f2-demo/`](../../crates/yoake-guest-sdk/examples/f1-f2-demo/src/lib.rs).

---

## 12-4. Rules and Limits

- **Queues**: One queue in each direction, capped at **1,024 messages and 16 MiB**. When full, `send` returns an error (`queue-full`) immediately without blocking. A single message larger than 16 MiB returns `too-large`.
- **Worker crash or exit**: If the worker traps, panics, or exits, only the worker stops. A log line is written, and the main mod keeps running; further sends return `closed`.
- **Unload / shutdown**: When the mod is unloaded (hot reload, leaving a world) or the game exits, the worker stops too (`worker::recv()` returns `None`).
- **Two-sided mods**: A package with both a `client` and a `server` module gets one worker per instance.

---

## 12-5. Note on Legacy `[wasm] permissions`

In sandboxed mods, config, permission-node, and storage services (`service.config`, `service.permissions`, `service.storage`) no longer require listing them in `[wasm] permissions`. Old manifests that still declare them continue to load cleanly.

Note that `service.commands`, `service.chat`, `service.gpu`, `service.mobs`, and `service.browser` still have their own rules—check [Chapter 4](./04-examples.md) and the chapters for each feature.

Examples: [`crates/yoake-wasm-sdk/examples/f2-io`](../../crates/yoake-wasm-sdk/examples/f2-io/src/lib.rs) and [`crates/yoake-wasm-sdk/examples/f2-check`](../../crates/yoake-wasm-sdk/examples/f2-check/server/src/lib.rs). Full specification: [`docs/yoake2/MOD-GUIDE.md` section 15](../yoake2/MOD-GUIDE.md#15-files-network-and-workers-fr-f2).
