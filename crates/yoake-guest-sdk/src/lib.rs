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
//! fn init(r: &mut Registrar) { r.hook(hooks::SERVER_TICK, 0); }
//! fn hook(id: u32, root: Handle) -> i64 { /* find entities, read, write */ 0 }
//! export_guest!(init, hook);
//! ```
//!
//! See `docs/yoake2/ENGINE_CALL_ABI.md` for the ABI, including "never call again to retry".

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
    /// The handle is not usable: 1 Stale, 2 Locked, 3 BadField, 4 NoSuchArg, 5 WrongType, 6 LimitReached.
    Handle(i32),
    /// The result bytes did not decode as the stub's type.
    Decode(CallError),
}

impl Error {
    pub fn from_status(status: i32) -> Error {
        if (status::HANDLE_BASE - 6..=status::HANDLE_BASE - 1).contains(&status) {
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

/// Handed to `init`: says which hooks this guest wants. A hook it does not register never enters the guest.
pub struct Registrar(());

impl Registrar {
    #[doc(hidden)]
    pub fn new() -> Self {
        Registrar(())
    }

    pub fn hook(&mut self, id: u32, priority: i32) {
        // SAFETY: plain values.
        unsafe { sys::hook_register(id as i32, priority) };
    }
}

impl Default for Registrar {
    fn default() -> Self {
        Self::new()
    }
}

/// Exports the two entry points of a multiplexed guest: `yoake_init` calls `init(&mut Registrar)`,
/// `yoake_hook` calls `hook(hook_id, handle)` and returns its `i64`.
#[macro_export]
macro_rules! export_guest {
    ($init:path, $hook:path) => {
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
    fn statuses_map_to_handle_errors() {
        assert_eq!(Error::from_status(status::HANDLE_BASE - 1), Error::Handle(1));
        assert!(Error::from_status(status::HANDLE_BASE - 1).is_stale());
        assert_eq!(Error::from_status(status::HANDLE_BASE - 5), Error::Handle(5));
        assert_eq!(Error::from_status(status::READ_ONLY), Error::Status(-4));
        assert_eq!(Error::from_status(-99), Error::Status(-99));
    }
}
