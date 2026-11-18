//! What the generated hook bridges share (`yoake-hook-codemod bridge`, docs/yoake2/HOOK-SLICE-BRIDGE.md).
//!
//! A bridge module (`hook_bridge`, one per scanned engine module) turns a ledger hook id into a typed registration on
//! the engine's hook slot whose callback enters the guests through `runtime::dispatch_id`. This module holds the parts
//! that must be process-wide or identical in every bridge: install-once, the install report, the mode of a hook, and
//! the decoding of the guest's `i64` result. No dependencies; no `unsafe`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// How a bridged hook treats the guest (ENGINE_CALL_ABI.md, "Hook modes").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The result round-trips through `i64`: the guest's return value is the hooked function's result, and a guest
    /// that does not call `original` cancels the call.
    Full,
    /// The result is not `i64`-encodable: the guest may call `original` (the engine keeps that result) but cannot
    /// replace it or cancel the call; if no guest of the chain calls `original`, the original runs once after them.
    ObserveOnly,
}

/// What one bridge module's `install` did with the ids it was given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallReport {
    /// Installed by this call or earlier in this process ([`install_once`]).
    pub installed: Vec<u32>,
    /// Ids this module has no hook for (another module's, or not a ledger id at all). Not an error by itself: the
    /// caller asks every bridge module and an id is unknown only when no module has it.
    pub not_mine: Vec<u32>,
    /// This module's hooks that cannot be bridged, with the reason (`UNBRIDGED`).
    pub not_bridgeable: Vec<(u32, &'static str)>,
    /// Bridgeable, but the typed registration refused it (slot full, callback type mismatch), with the reason.
    pub failed: Vec<(u32, &'static str)>,
}

static INSTALLED: Mutex<BTreeMap<u32, Result<(), &'static str>>> = Mutex::new(BTreeMap::new());

/// Runs `register` for hook `id` at most once per process and returns its outcome (the first outcome on every later
/// call). Registering appends to the hook slot's chain and nothing removes an entry, so a second registration of the
/// same dispatcher would run every guest on that id twice per call. A dispatcher with no guest left for its id (the
/// mod was unloaded, or the call comes from a thread without mods) finds an empty chain and runs the original.
pub fn install_once(id: u32, register: impl FnOnce() -> Result<(), &'static str>) -> Result<(), &'static str> {
    let mut installed = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
    *installed.entry(id).or_insert_with(register)
}

/// Whether the dispatcher of `id` is installed in this process.
pub fn is_installed(id: u32) -> bool {
    INSTALLED.lock().unwrap_or_else(|e| e.into_inner()).get(&id).is_some_and(|r| r.is_ok())
}

/// Answers "what mode would hook `id` have": `Some(mode)` when a bridge can install it, `None` when this engine build
/// cannot call it (no bridge module has it, or it is not bridgeable). Set by the loader from the bridges' `mode`
/// functions before mods run `yoake_init`, so `hook_register` can tell the guest.
pub type ModeResolver = fn(u32) -> Option<Mode>;

static RESOLVER: RwLock<Option<ModeResolver>> = RwLock::new(None);

/// Sets (or clears) the process-wide [`ModeResolver`].
pub fn set_mode_resolver(resolver: Option<ModeResolver>) {
    *RESOLVER.write().unwrap_or_else(|e| e.into_inner()) = resolver;
}

/// The mode of hook `id`: `None` when no resolver is set (a host without bridges, e.g. the runtime's own tests),
/// `Some(None)` when the resolver says this build cannot call it.
pub fn mode_of(id: u32) -> Option<Option<Mode>> {
    RESOLVER.read().unwrap_or_else(|e| e.into_inner()).map(|resolve| resolve(id))
}

static OUT_OF_RANGE: AtomicU64 = AtomicU64::new(0);
static OUT_OF_RANGE_IDS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// A guest returned `value` for hook `id`, and it does not fit the hooked function's result type. The bridge then
/// keeps the original's result (running the original if it has not run). Counted, and printed once per hook id.
pub fn note_out_of_range(id: u32, value: i64) {
    OUT_OF_RANGE.fetch_add(1, Ordering::Relaxed);
    let mut seen = OUT_OF_RANGE_IDS.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.contains(&id) {
        seen.push(id);
        eprintln!("yoake: a mod returned {value} for hook {id}, out of range for its result type; the original result is used");
    }
}

/// How many guest results were out of range since the process started.
pub fn out_of_range_count() -> u64 {
    OUT_OF_RANGE.load(Ordering::Relaxed)
}

/// Decoding the guest's `i64` into the hooked function's result type (ENGINE_CALL_ABI.md, "Hook results").
/// `None`: out of range, see [`note_out_of_range`].
pub mod decode {
    /// `0` is `false`, `1` is `true`, anything else is out of range.
    pub fn bool(value: i64) -> Option<bool> {
        match value {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    /// Integers other than `i64`/`u64`: the value must fit (`i32` takes `i32::MIN..=i32::MAX`, `u8` takes `0..=255`,
    /// `usize` takes `0..=usize::MAX` as far as `i64` reaches).
    pub fn int<T: TryFrom<i64>>(value: i64) -> Option<T> {
        T::try_from(value).ok()
    }

    /// `u64` travels as its two's-complement bit pattern (`u64::MAX` is `-1`): every `i64` is a valid `u64`.
    pub fn u64(value: i64) -> u64 {
        value as u64
    }

    /// `f32` travels as its bits in the low 32 bits: the value must be in `0..=u32::MAX`.
    pub fn f32(value: i64) -> Option<f32> {
        u32::try_from(value).ok().map(f32::from_bits)
    }

    /// `f64` travels as its bits: every `i64` is a valid `f64` bit pattern.
    pub fn f64(value: i64) -> f64 {
        f64::from_bits(value as u64)
    }
}

/// Encoding the original's result for the guest (the inverse of [`decode`]).
pub mod encode {
    pub fn bool(value: bool) -> i64 {
        i64::from(value)
    }

    /// Integers that fit `i64` (all of them except `u64`/`usize`/`u128`/`i128`).
    pub fn int<T: Into<i64>>(value: T) -> i64 {
        value.into()
    }

    pub fn u64(value: u64) -> i64 {
        value as i64
    }

    /// `usize` is at most 64 bits on every target the engine builds for; as `u64`, two's complement.
    pub fn usize(value: usize) -> i64 {
        value as u64 as i64
    }

    pub fn f32(value: f32) -> i64 {
        i64::from(value.to_bits())
    }

    pub fn f64(value: f64) -> i64 {
        value.to_bits() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_once_runs_the_registration_once_and_keeps_its_outcome() {
        let mut calls = 0;
        assert_eq!(install_once(0x7fff_0001, || { calls += 1; Ok(()) }), Ok(()));
        assert_eq!(install_once(0x7fff_0001, || { calls += 1; Err("never") }), Ok(()));
        assert_eq!(calls, 1);
        assert!(is_installed(0x7fff_0001));
        assert_eq!(install_once(0x7fff_0002, || Err("slot full")), Err("slot full"));
        assert_eq!(install_once(0x7fff_0002, || Ok(())), Err("slot full"), "a failure is not retried either");
        assert!(!is_installed(0x7fff_0002));
    }

    #[test]
    fn decoding_checks_the_range() {
        assert_eq!(decode::bool(1), Some(true));
        assert_eq!(decode::bool(2), None);
        assert_eq!(decode::int::<i32>(i64::from(i32::MIN)), Some(i32::MIN));
        assert_eq!(decode::int::<i32>(i64::from(i32::MAX) + 1), None);
        assert_eq!(decode::int::<u8>(-1), None);
        assert_eq!(decode::int::<usize>(-1), None);
        assert_eq!(decode::u64(-1), u64::MAX);
        assert_eq!(decode::f32(encode::f32(1.5)), Some(1.5));
        assert_eq!(decode::f32(-1), None);
        assert_eq!(decode::f32(1 << 32), None);
        assert_eq!(decode::f64(encode::f64(-2.25)), -2.25);
        assert_eq!(decode::u64(encode::u64(u64::MAX)), u64::MAX);
        assert_eq!(encode::usize(usize::MAX), -1);
    }
}
