//! MX3K1: real Rust guests, compiled to wasm32 core modules with the installed `wasm32-wasip2` target, run
//! on the slice runtime.
//!
//! The component wrapper is skipped by linking with `rust-lld` in `wasm-ld` flavor instead of the default
//! `wasm-component-ld`; the guests are `no_std` + the SDK's allocator and panic handler, so the module has
//! no WASI imports at all. Nothing is installed: `rustc`, the target's std/core and `rust-lld` ship with the
//! toolchain.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use yoake_slice::activation::FieldTable;
use yoake_slice::codec::CallError;
use yoake_slice::runtime::{self, ModConfig, set_engine_dispatch};

fn tmp() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guests");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[path = "support/build.rs"]
mod build;

/// The SDK as a wasm32 rlib, built once per test run.
fn sdk_rlib() -> &'static PathBuf {
    static RLIB: OnceLock<PathBuf> = OnceLock::new();
    RLIB.get_or_init(|| build::build_sdk(Path::new(env!("CARGO_MANIFEST_DIR")), &tmp()))
}

/// Compiles `tests/guests/<name>.rs` to a core module (no component wrapper, no WASI imports).
fn build_guest(name: &str) -> Vec<u8> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/guests").join(format!("{name}.rs"));
    build::build_guest(sdk_rlib(), &source, &tmp(), name)
}

fn install(wasm: &[u8]) {
    runtime::install(wasm, ModConfig::new("")).unwrap();
}

fn slot(reports: &[(i32, i64)], n: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

// ---- a small hooked engine: f's body calls g, both hooked ----

struct Thing {
    count: i32,
}

static TABLE: FieldTable<Thing> = FieldTable {
    get: |t, f| (f == 0).then(|| i64::from(t.count)),
    set: |t, f, v| {
        if f == 0 {
            t.count = v as i32;
            true
        } else {
            false
        }
    },
};

fn f(t: &mut Thing) {
    runtime::dispatch_id(1, t, &TABLE, &mut |t| {
        t.count += 1;
        g(t);
        0
    });
}

fn g(t: &mut Thing) {
    runtime::dispatch_id(2, t, &TABLE, &mut |t| {
        t.count += 100;
        0
    });
}

/// The guest's `burn`, on the host: the expected values.
fn burn(depth: u32, seed: u32) -> u32 {
    let mut a = [0u32; 32];
    for (i, slot) in a.iter_mut().enumerate() {
        *slot = seed.wrapping_mul(i as u32 + 1);
    }
    if depth == 0 {
        return a.iter().fold(0u32, |x, y| x.wrapping_add(*y));
    }
    burn(depth - 1, seed + 1).wrapping_add(a[(depth as usize) % 32])
}

#[test]
fn a_rust_guest_compiles_to_a_core_module_and_registers_its_hooks() {
    let wasm = build_guest("reentry");
    assert_eq!(&wasm[..8], [0, b'a', b's', b'm', 1, 0, 0, 0], "a core module (version 1), not a component");
    install(&wasm);
    let stats = runtime::stats();
    assert_eq!(stats.registered, vec![(1, 0), (2, 5)], "yoake_init registered the hooks with their priorities");
    assert!(!stats.disabled);
}

#[test]
fn rustc_and_wasm_ld_retain_the_sdk_abi_section() {
    let wasm = build_guest("reentry");
    assert_eq!(runtime::ABI_VERSION, runtime::AbiVersion { major: 1, minor: 0 });
    assert_eq!(runtime::module_abi_version(&wasm).unwrap(), Some(runtime::ABI_VERSION));
}

#[test]
fn shadow_stack_and_heap_survive_host_guest_original_nested_hook_same_guest() {
    install(&build_guest("reentry"));
    let mut thing = Thing { count: 0 };
    f(&mut thing);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 1), [1], "the outer hook ran");
    assert_eq!(slot(&r, 2), [1], "its heap vector, stack array and recursion are intact after the nested activation");
    assert_eq!(slot(&r, 3), [201], "and it can still allocate");
    assert_eq!(slot(&r, 4), [i64::from(burn(20, 3))]);
    assert_eq!(slot(&r, 10), [i64::from(burn(30, 7))], "the nested hook ran in the same guest instance");
    assert_eq!(slot(&r, 11), [(0..500u32).map(|x| x * 2).sum::<u32>() as i64]);
    assert_eq!(thing.count, 77, "the inner hook replaced g's body and wrote through its handle; f's body ran before");
    let stats = runtime::stats();
    assert_eq!(stats.entries, 2);
    assert!(!stats.disabled);
    assert_eq!(yoake_slice::activation::live_activations(), 0);
    // and again: the allocator reuses freed blocks, nothing has drifted
    f(&mut thing);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 2), [1]);
    assert_eq!(runtime::stats().entries, 4);
}

#[test]
fn a_hook_the_guest_did_not_register_does_not_enter_it() {
    install(&build_guest("only_outer"));
    assert_eq!(runtime::stats().registered, vec![(1, 0)]);
    let mut thing = Thing { count: 0 };
    f(&mut thing);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 101), [1], "hook 1 ran in the guest");
    assert!(slot(&r, 102).is_empty(), "hook 2 did not");
    assert_eq!(thing.count, 101, "g's original body ran un-hooked");
    assert_eq!(runtime::stats().entries, 1);
    // a hook id nobody registered, on a fresh thread's worth of state: still plain
    let mut other = Thing { count: 0 };
    runtime::dispatch_id(77, &mut other, &TABLE, &mut |t| {
        t.count = 5;
        0
    });
    assert_eq!(other.count, 5);
    assert_eq!(runtime::stats().entries, 1);
}

static RUNS: AtomicU32 = AtomicU32::new(0);

fn dispatch(_handle: u64, id: u32, _args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {
    match id {
        1 => {
            out.extend_from_slice(&[1, 2, 3]);
            Ok(())
        }
        2 => {
            RUNS.fetch_add(1, Ordering::SeqCst);
            out.extend((0..300).map(|i| (i % 251) as u8));
            Ok(())
        }
        other => Err(CallError::UnknownBinding(other)),
    }
}

#[test]
fn the_sdk_fetches_a_result_that_did_not_fit_through_a_ticket_without_a_retry() {
    set_engine_dispatch(Some(dispatch));
    install(&build_guest("tickets"));
    let mut thing = Thing { count: 0 };
    runtime::dispatch_id(1, &mut thing, &TABLE, &mut |_| 0);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 1), [3], "a small result");
    assert_eq!(slot(&r, 2), [300], "the 300-byte result came back whole");
    assert_eq!(slot(&r, 3), [0]);
    assert_eq!(slot(&r, 4), [i64::from((299 % 251) as u8)]);
    assert_eq!(slot(&r, 5), [1], "every byte is right");
    assert_eq!(slot(&r, 6), [i64::from(runtime::call_status::UNKNOWN_BINDING)]);
    assert_eq!(RUNS.load(Ordering::SeqCst), 1, "the binding ran once although its result did not fit the first buffer");
    assert!(!runtime::stats().disabled);
}

#[test]
fn the_guest_allocator_honours_alignments_above_16() {
    install(&build_guest("align"));
    let mut thing = Thing { count: 0 };
    runtime::dispatch_id(1, &mut thing, &TABLE, &mut |_| 0);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 1), [0], "no allocation of align 64, 256 or 4096 was misaligned (also after frees and Vec growth)");
    assert_eq!(slot(&r, 2), [150]);
    assert!(!runtime::stats().disabled, "{:?}", runtime::stats().trap);
}

#[test]
fn memory_grows_while_an_outer_hook_waits_in_original_and_its_blocks_survive() {
    // filling 3 MiB byte by byte is a lot of instructions: a bigger fuel budget than the default
    runtime::install(&build_guest("grow_nesting"), ModConfig { fuel: Some(500_000_000), ..ModConfig::new("") }).unwrap();
    let mut thing = Thing { count: 0 };
    f(&mut thing);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 10), [7 + 7 + 3], "the nested hook allocated and touched 3 MiB: {:?}", runtime::stats().trap);
    assert_eq!(slot(&r, 1), [1], "the outer hook's vector is intact");
    assert!(slot(&r, 2)[0] >= 48, "the guest memory grew by at least 3 MiB during the nesting: {:?}", slot(&r, 2));
    assert_eq!(slot(&r, 3), [100_005], "and it can allocate again afterwards");
    assert!(!runtime::stats().disabled, "{:?}", runtime::stats().trap);
}

#[test]
fn the_outer_hook_continues_after_a_nested_trap() {
    install(&build_guest("trap_nested"));
    let mut thing = Thing { count: 0 };
    f(&mut thing);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 10), [1], "the nested hook ran and trapped");
    assert_eq!(slot(&r, 2), [1], "the outer hook continued after `original` and its data is intact");
    let stats = runtime::stats();
    assert!(stats.disabled, "a trap disables the mod");
    assert_eq!(thing.count, 101, "f's body +1 and g's fallback +100: each engine body exactly once");
    // never entered again
    let before = runtime::stats().entries;
    f(&mut thing);
    assert_eq!(runtime::stats().entries, before);
}
