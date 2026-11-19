//! The Yoake 2.0 Rust guest SDK (MX3K1).
//!
//! A full-access mod is a wasm32 **core module**: it exports `yoake_init` (registers the hooks it wants) and
//! `yoake_hook(hook_id, handle)` (one entry for every hook), and calls the engine through the imports of
//! module `env`. This crate wraps those imports in safe Rust, carries the wire codec, a small allocator and
//! a trapping panic handler, and (in [`stubs`]) the typed per-binding functions yoake-bindgen generates.
//!
//! ```ignore
//! #![no_std]
//! extern crate alloc;
//! use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks};
//!
//! fn init(r: &mut Registrar) { r.hook(hooks::SERVER_TICK, 0).unwrap(); }
//! fn hook(id: u32, root: Handle) -> i64 { /* find entities, read, write */ 0 }
//! export_guest!(init, hook);
//! ```
//!
//! See `docs/yoake2/ENGINE_CALL_ABI.md` for the ABI, including "never call again to retry".
//!
//! # Re-entry: never hold a borrow across `original`
//!
//! `original` runs the engine (and the next mod), and the engine may call a hook that enters **this same guest**
//! again (a nested activation on the same instance). Anything the outer hook keeps borrowed across that call is
//! borrowed when the nested hook runs: a `RefCell` borrow, a `static mut` reference, a lock-like flag. The
//! nested hook then panics (a `RefCell` already borrowed) and a panic is a trap: the mod is disabled. So: take
//! borrows of guest statics only around code that does not call `original` or any engine binding that can run
//! hooks, copy what you need out before calling, and do not keep a `RefCell` guard alive over the call.

#![no_std]
// the SDK is where guest-side `unsafe` lives (imports, allocator); the generated stubs contain none
#![allow(unsafe_code)]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod heap;
pub mod stubs;
pub mod sys;
pub mod wire;

use alloc::vec;
use alloc::vec::Vec;
use wire::{CallError, Decode, Decoder, Encode};

/// Hook ids a guest can register (S2 slot ids are small; engine-synthetic hooks are at and above
/// `0x8000_0000`).
pub mod hooks {
    pub const SYNTHETIC_BASE: u32 = 0x8000_0000;
    /// `Game::yoake2_server_tick`: argument 0 is the `Game` root.
    pub const SERVER_TICK: u32 = SYNTHETIC_BASE + 1;
    /// MX3CL1: once per client tick, on the window's thread; argument 0 is the `Game` root of the client.
    pub const CLIENT_TICK: u32 = SYNTHETIC_BASE + 2;
    /// MX3CL1: once per frame after the vanilla HUD, on the client thread. Observe-only; no handle arguments yet.
    pub const HUD_RENDER: u32 = SYNTHETIC_BASE + 3;
}

/// A handle the engine gave this activation: a hook argument, the `Game` root, or a path handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(pub u64);

/// Statuses of `engine_call` and friends (negative return values).
pub mod status {
    pub const ARGS_TOO_LARGE: i32 = -1;
    pub const OUT_TOO_SMALL: i32 = -2;
    pub const BAD_ARGS: i32 = -3;
    pub const READ_ONLY: i32 = -4;
    pub const UNKNOWN_BINDING: i32 = -5;
    pub const NO_DISPATCHER: i32 = -6;
    pub const NO_SUCH_TICKET: i32 = -7;
    pub const TICKET_LIMIT: i32 = -8;
    pub const HANDLE_BASE: i32 = -16;
    pub const OUT_TOO_SMALL_BASE: i32 = 256;
}

/// What can go wrong with an engine call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The call was refused or failed: the raw negative status (see [`status`]).
    Status(i32),
    /// The handle is not usable: 1 Stale, 2 Locked, 3 BadField, 4 NoSuchArg, 5 WrongType, 6 LimitReached, 7 ReadOnly.
    Handle(i32),
    /// The result bytes did not decode as the stub's type.
    Decode(CallError),
}

impl Error {
    pub fn from_status(status: i32) -> Error {
        if (status::HANDLE_BASE - 7..=status::HANDLE_BASE - 1).contains(&status) {
            Error::Handle(status::HANDLE_BASE - status)
        } else {
            Error::Status(status)
        }
    }

    pub fn is_stale(&self) -> bool {
        matches!(self, Error::Handle(1))
    }
}

impl From<CallError> for Error {
    fn from(e: CallError) -> Self {
        Error::Decode(e)
    }
}

/// Runs binding `id` on `handle` with the encoded `args`. If the result does not fit the first buffer the
/// host has already run the call and kept the result under a ticket: it is fetched with `engine_take`.
/// **Never call again to retry.**
pub fn call(handle: Handle, id: u32, args: &[u8]) -> Result<Vec<u8>, Error> {
    let mut first = [0u8; 256];
    // SAFETY: pointers and lengths describe live buffers of this guest's memory.
    let r = unsafe { sys::engine_call(handle.0 as i64, id as i32, args.as_ptr() as i32, args.len() as i32, first.as_mut_ptr() as i32, first.len() as i32) };
    if r >= 0 {
        return Ok(first[..r as usize].to_vec());
    }
    if r <= -(status::OUT_TOO_SMALL_BASE + 1) {
        let ticket = -r - status::OUT_TOO_SMALL_BASE;
        // the host wrote the needed size at the start of `first`
        let need = u32::from_le_bytes([first[0], first[1], first[2], first[3]]) as usize;
        let mut bytes = vec![0u8; need];
        // SAFETY: as above.
        let taken = unsafe { sys::engine_take(ticket, bytes.as_mut_ptr() as i32, need as i32) };
        return if taken >= 0 { Ok(bytes) } else { Err(Error::from_status(taken)) };
    }
    Err(Error::from_status(r))
}

/// Typed call: encodes the arguments with `enc`, decodes the result with `dec`.
pub fn call_with<R>(handle: Handle, id: u32, enc: impl FnOnce(&mut Vec<u8>), dec: impl FnOnce(&mut Decoder<'_>) -> Result<R, CallError>) -> Result<R, Error> {
    let mut args = Vec::new();
    enc(&mut args);
    let out = call(handle, id, &args)?;
    let mut decoder = Decoder::new(&out);
    let value = dec(&mut decoder)?;
    decoder.finish()?;
    Ok(value)
}

/// Runs the hooked function's original body (or the next hook in the chain); returns its `i64` result.
/// Every handle of this activation is `Locked` while it runs.
pub fn original(handle: Handle) -> Result<i64, Error> {
    let mut out = [0u8; 8];
    // SAFETY: `out` is 8 live bytes.
    let status = unsafe { sys::original(handle.0 as i64, out.as_mut_ptr() as i32) };
    if status == 0 { Ok(i64::from_le_bytes(out)) } else { Err(Error::Handle(status)) }
}

/// A scalar field of a hook argument by the generator's field index, as `i64` bits.
pub fn get(handle: Handle, field: u32) -> Result<i64, Error> {
    let mut out = [0u8; 8];
    // SAFETY: `out` is 8 live bytes.
    let status = unsafe { sys::get(handle.0 as i64, field as i32, out.as_mut_ptr() as i32) };
    if status == 0 { Ok(i64::from_le_bytes(out)) } else { Err(Error::Handle(status)) }
}

pub fn set(handle: Handle, field: u32, bits: i64) -> Result<(), Error> {
    // SAFETY: plain values.
    let status = unsafe { sys::set(handle.0 as i64, field as i32, bits) };
    if status == 0 { Ok(()) } else { Err(Error::Handle(status)) }
}

/// A test aid: appends `(slot, value)` to the host's report list.
pub fn report(slot: i32, value: i64) {
    // SAFETY: plain values.
    unsafe { sys::report(slot, value) }
}

/// How the engine can call a registered hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookMode {
    /// The guest may replace the result or cancel the call by not calling `original`.
    Full,
    /// The guest cannot replace the result or cancel the call.
    ObserveOnly,
    /// Registration was recorded, but this engine build cannot call the id.
    NotCallable,
}

/// Why a hook registration was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterError {
    NotInInit,
    TooMany,
    ReservedId,
    Unknown(i32),
}

fn registration_outcome(status: i32) -> Result<HookMode, RegisterError> {
    match status {
        0 => Ok(HookMode::Full),
        1 => Ok(HookMode::ObserveOnly),
        2 => Ok(HookMode::NotCallable),
        -1 => Err(RegisterError::NotInInit),
        -2 => Err(RegisterError::TooMany),
        -3 => Err(RegisterError::ReservedId),
        other => Err(RegisterError::Unknown(other)),
    }
}

/// Handed to `init`: says which hooks this guest wants. A hook it does not register never enters the guest.
pub struct Registrar(());

impl Registrar {
    #[doc(hidden)]
    pub fn new() -> Self {
        Registrar(())
    }

    pub fn hook(&mut self, id: u32, priority: i32) -> Result<HookMode, RegisterError> {
        // SAFETY: plain values.
        registration_outcome(unsafe { sys::hook_register(id as i32, priority) })
    }
}

impl Default for Registrar {
    fn default() -> Self {
        Self::new()
    }
}

/// The guest ABI major version emitted by [`export_guest!`].
pub const ABI_MAJOR: u16 = 1;
/// The guest ABI minor version emitted by [`export_guest!`].
pub const ABI_MINOR: u16 = 0;

/// Exports the ABI version and the two entry points of a multiplexed guest: `yoake_init` calls
/// `init(&mut Registrar)`, and `yoake_hook` calls `hook(hook_id, handle)` and returns its `i64`.
#[macro_export]
macro_rules! export_guest {
    ($init:path, $hook:path) => {
        #[used]
        #[unsafe(link_section = "yoake_abi")]
        static YOAKE_ABI: [u8; 4] = [
            $crate::ABI_MAJOR as u8,
            ($crate::ABI_MAJOR >> 8) as u8,
            $crate::ABI_MINOR as u8,
            ($crate::ABI_MINOR >> 8) as u8,
        ];
        #[unsafe(no_mangle)]
        pub extern "C" fn yoake_init() {
            $init(&mut $crate::Registrar::new());
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn yoake_hook(hook_id: i32, handle: i64) -> i64 {
            $hook(hook_id as u32, $crate::Handle(handle as u64))
        }
    };
}

/// Encodes one value to bytes (for hand-written calls; stubs do it inline).
pub fn encode<T: Encode + ?Sized>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    value.encode(&mut out);
    out
}

/// Decodes one value from all of `bytes`.
pub fn decode<T: Decode>(bytes: &[u8]) -> Result<T, CallError> {
    let mut d = Decoder::new(bytes);
    let v = T::decode(&mut d)?;
    d.finish()?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_abi_version_is_1_0() {
        assert_eq!((ABI_MAJOR, ABI_MINOR), (1, 0));
    }

    #[test]
    fn registration_statuses_map_to_modes_and_errors() {
        assert_eq!(registration_outcome(0), Ok(HookMode::Full));
        assert_eq!(registration_outcome(1), Ok(HookMode::ObserveOnly));
        assert_eq!(registration_outcome(2), Ok(HookMode::NotCallable));
        assert_eq!(registration_outcome(-1), Err(RegisterError::NotInInit));
        assert_eq!(registration_outcome(-2), Err(RegisterError::TooMany));
        assert_eq!(registration_outcome(-3), Err(RegisterError::ReservedId));
        assert_eq!(registration_outcome(7), Err(RegisterError::Unknown(7)));
    }

    #[test]
    fn statuses_map_to_handle_errors() {
        assert_eq!(Error::from_status(status::HANDLE_BASE - 1), Error::Handle(1));
        assert!(Error::from_status(status::HANDLE_BASE - 1).is_stale());
        assert_eq!(Error::from_status(status::HANDLE_BASE - 5), Error::Handle(5));
        assert_eq!(Error::from_status(status::HANDLE_BASE - 7), Error::Handle(7));
        assert_eq!(Error::from_status(status::READ_ONLY), Error::Status(-4));
        assert_eq!(Error::from_status(-99), Error::Status(-99));
    }
}

/// The mod's worker (FR-F2): a component declared by `[wasm] worker` that runs on its own thread with full
/// WASI p2 (sockets, http, blocking waits). The two calls never block the game thread.
pub mod worker {
    use super::sys;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Why [`send`] failed.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SendError {
        /// 1024 messages or 16 MiB are already waiting; nothing was queued.
        QueueFull,
        /// The mod declares no worker, or it stopped.
        NoWorker,
        /// The message alone is over 16 MiB.
        TooLarge,
        BadPointer,
    }

    /// Why [`recv`] failed.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum RecvError {
        NoWorker,
        BadPointer,
    }

    /// Queues a message for the worker.
    pub fn send(message: &[u8]) -> Result<(), SendError> {
        // SAFETY: the pointer and length describe `message`, which is alive for the call.
        match unsafe { sys::yoake_worker_send(message.as_ptr() as i32, message.len() as i32) } {
            0 => Ok(()),
            -1 => Err(SendError::QueueFull),
            -3 => Err(SendError::TooLarge),
            -4 => Err(SendError::BadPointer),
            _ => Err(SendError::NoWorker),
        }
    }

    /// The next message from the worker, or `None`.
    pub fn recv() -> Result<Option<Vec<u8>>, RecvError> {
        let mut buffer = vec![0u8; 1024];
        loop {
            // SAFETY: the pointer and capacity describe `buffer`.
            let result = unsafe { sys::yoake_worker_recv(buffer.as_mut_ptr() as i32, buffer.len() as i32) };
            match result {
                0 => return Ok(None),
                n if n > 0 => {
                    buffer.truncate(n as usize);
                    return Ok(Some(buffer));
                }
                i32::MIN => return Err(RecvError::NoWorker),
                n if n == i32::MIN + 1 => return Err(RecvError::BadPointer),
                n => buffer.resize(n.unsigned_abs() as usize, 0),
            }
        }
    }
}
