# engine_call ABI notes (SDK-facing)

For guests (core WASM modules) that call generated engine bindings. Imports are in module `env`. Read this before
writing a retry loop: **there is no retry**.

## `engine_call(handle: i64, id: i32, ptr: i32, len: i32, out_ptr: i32, out_cap: i32) -> i32`

Runs binding `id` (ids come from `crates/yoake-bindgen/ids.lock.json`, append-only, never reused).

- `handle`: the handle of the receiver: argument 0 of the hook (`Root::HookArg`), or a *path handle* (below). Free
  functions ignore it.
- `ptr`, `len`: the encoded arguments (codec: little-endian numbers, 32-bit lengths, tags for `Option`/`Result`,
  `usize` as 64 bits). At most 1 MiB; more is refused without reading guest memory.
- `out_ptr`, `out_cap`: where the encoded result goes.

Return value:

| value | meaning |
|---|---|
| `>= 0` | the call ran; this many result bytes were written at `out_ptr` |
| `<= -257` | the call ran, the result did not fit `out_cap`. The result is kept; the **ticket** is `-ret - 256`. The size was written as a little-endian `u32` at `out_ptr` if `out_cap >= 4` |
| `-1` | arguments over 1 MiB (nothing ran) |
| `-3` | malformed arguments (nothing ran) |
| `-4` | the binding is a setter of a read-only field (nothing ran) |
| `-5` | unknown id |
| `-6` | the host registered no dispatcher |
| `-8` | the result did not fit and this activation already holds 8 kept results or 16 MiB: **the call ran, its result is lost**. Size the buffer first |
| `-17 … -21` | handle errors: Stale, Locked, BadField, NoSuchArg, WrongType (nothing ran) |

An engine panic caused by the arguments is not a status: the guest traps, the mod is disabled (the trap rule), and the
host logs `engine state may be inconsistent` with the binding id.

### Never call again to retry

By the time the buffer turns out too small the engine call has **already run**. Calling again would run it a second
time (a setter, a spawn, a counter). So the result is kept under a ticket and you fetch it:

## `engine_take(ticket: i32, out_ptr: i32, out_cap: i32) -> i32`

Copies the kept result without running anything. Returns the byte count (the ticket is then spent), or `-2` (still too
small: the size is written at `out_ptr` when `out_cap >= 4`, the ticket stays), or `-7` (no such ticket).

A ticket is valid only inside the activation that got it and is dropped when that activation ends. If you do not
want to deal with tickets, size `out_cap` for the biggest result (the getters of one type are at most a few hundred
bytes; lists are bounded by the type).

## Path handles (`Root::Game`)

In a hook whose argument 0 is the `Game` root (the `yoake2_server_tick` hook; no other hook has one) the guest calls a
generated `Game.<name>#handle` binding with the root handle, for example `Game.item_entity#handle(id: u32) -> u64`, and
gets a path handle. Every bound method and field accessor of the type then works on it exactly as on a hook argument.

- The handle is a path (`Game`, field, id), re-borrowed from the root on **every** call, never a reference.
- It is valid only inside this activation (`Stale` after it). It is `Stale` when the object is gone (removed from the
  level), `Locked` while `original` or another call of this activation runs, `WrongType` for a method of another type.
- An activation holds at most 256 path handles.
- Inside any other hook (`ArmorStand::tick_settle`, ...) there is no root: the caller holds an outer `&mut`. Asking for a
  path there is `WrongType`.
