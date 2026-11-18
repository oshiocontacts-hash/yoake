//! Yoake 2.0 vertical slice (MX3V1).
//!
//! * [`activation`]: the per-activation arena of handles (`Root::HookArg`), the lending lock and
//!   the original-call record. No dependencies. This is where the slice's `unsafe` lives.
//! * [`codec`]: the wire codec generated glue (MX3G1) reads arguments and writes results with.
//! * [`runtime`] (feature `wasm`): a core-module WASM runtime on the C0 ACTIVE-store pattern, with
//!   fuel and epoch budgets per activation and the trap rule from MX3C0.

pub mod activation;
pub mod codec;
#[cfg(feature = "wasm")]
pub mod runtime;

/// The hand-written WAT guest used by the tests and the benchmark.
#[cfg(feature = "wasm")]
pub const GUEST_WAT: &str = include_str!("../guest/slice_guest.wat");
