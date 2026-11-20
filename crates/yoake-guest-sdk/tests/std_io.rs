//! FR-F2: full-access core modules get files, log lines, clocks, random (WASI preview 1) and a worker.
//!
//! * `guests_std/std_fs.rs` is built with Rust `std` for `wasm32-wasip1` and uses `std::fs`.
//! * `guests/worker_sdk.rs` is the usual `no_std` SDK guest and talks to a worker component through
//!   `yoake_guest_sdk::worker`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yoake_mod_io::{FileAccess, LogSink, ModIo};
use yoake_slice::activation::{Accessor, PathRoot, Status};
use yoake_slice::runtime::{self, ModConfig};

#[path = "support/build.rs"]
mod build;

fn tmp() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("std_io");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scratch(name: &str) -> PathBuf {
    let dir = tmp().join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Root;

impl PathRoot for Root {
    fn resolve(&mut self, _: &[Accessor], _: &mut dyn FnMut(&mut dyn std::any::Any)) -> Result<(), Status> {
        Err(Status::Stale)
    }
}

fn tick(hook: u32) {
    runtime::dispatch_root_id(hook, &mut Root, &mut |_| 0);
}

fn build_std_guest() -> Vec<u8> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/guests_std/std_fs.rs");
    let wasm = tmp().join("std_fs.wasm");
    let result = Command::new("rustc")
        .args(["--edition", "2024", "--target", "wasm32-wasip1", "--crate-type", "cdylib", "-C", "opt-level=1", "-C", "panic=abort"])
        .arg("-o")
        .arg(&wasm)
        .arg(&source)
        .output()
        .expect("run rustc");
    assert!(result.status.success(), "std guest did not compile:\n{}", String::from_utf8_lossy(&result.stderr));
    std::fs::read(&wasm).unwrap()
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn sink(&self) -> LogSink {
        let lines = Arc::clone(&self.0);
        LogSink::new(move |line| lines.lock().unwrap().push(line.to_owned()))
    }
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

fn slot(reports: &[(i32, i64)], n: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

#[test]
fn std_guest_uses_std_fs_log_clocks_and_random_through_wasip1() {
    let wasm = build_std_guest();
    let game = scratch("std");
    let log = Log::default();
    let mut io = ModIo::new(FileAccess::new(&game, "stdmod", false));
    io.log = log.sink();
    let cfg = ModConfig { io: Some(io), ..ModConfig::full_access() };
    runtime::uninstall().unwrap();
    runtime::load(&wasm, cfg, "stdmod").expect("a std module loads");
    tick(runtime::HOOK_SERVER_TICK);
    let reports = runtime::take_reports().unwrap();
    assert_eq!(slot(&reports, 1), [1], "std::fs::write to /data: {reports:?}");
    assert_eq!(slot(&reports, 2), [8], "std::fs::read_to_string: {reports:?}");
    assert_eq!(slot(&reports, 3), [1], "/game is writable: {reports:?}");
    assert_eq!(slot(&reports, 4), [1], "wall clock");
    assert_eq!(slot(&reports, 5), [1], "monotonic clock");
    assert_eq!(slot(&reports, 6), [1], "random");
    assert_eq!(std::fs::read_to_string(game.join("mod-data/stdmod/std.txt")).unwrap(), "from std");
    assert_eq!(std::fs::read_to_string(game.join("std-probe.txt")).unwrap(), "g");
    let lines = log.lines();
    assert!(lines.contains(&"wasm mod stdmod [stdout]: std guest says hi".to_owned()), "{lines:?}");
    assert!(lines.contains(&"wasm mod stdmod [stderr]: std guest stderr line".to_owned()), "{lines:?}");
    runtime::uninstall().unwrap();
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn a_wait_in_a_full_access_module_traps_it() {
    let wasm = build_std_guest();
    let game = scratch("sleep");
    let cfg = ModConfig { io: Some(ModIo::new(FileAccess::new(&game, "sleeper", false))), ..ModConfig::full_access() };
    runtime::uninstall().unwrap();
    let id = runtime::load(&wasm, cfg, "sleeper").unwrap();
    tick(runtime::HOOK_CLIENT_TICK);
    let stats = runtime::stats_of(id).unwrap();
    assert!(stats.disabled, "{stats:?}");
    assert!(slot(&runtime::take_reports().unwrap(), 100).is_empty());
    runtime::uninstall().unwrap();
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn a_module_without_wasi_imports_loads_as_before() {
    let wat = r#"(module
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const -2147483647) (i32.const 0))))
  (func (export "yoake_hook") (param i32 i64) (result i64) (i64.const 0)))"#;
    runtime::uninstall().unwrap();
    runtime::load(wat.as_bytes(), ModConfig::full_access(), "plain").expect("loads without any io config");
    runtime::uninstall().unwrap();
}

fn build_sdk_guest(name: &str) -> Vec<u8> {
    let sdk = build::build_sdk(Path::new(env!("CARGO_MANIFEST_DIR")), &tmp());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/guests").join(format!("{name}.rs"));
    build::build_guest(&sdk, &source, &tmp(), name)
}

fn build_worker_component() -> Vec<u8> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-wasm-sdk/examples/f2-io");
    let target = tmp().join("worker-target");
    let result = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["build", "--offline", "--release", "--target", "wasm32-wasip2", "--features", "worker"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", &target)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .output()
        .expect("run cargo");
    assert!(result.status.success(), "worker did not build:\n{}", String::from_utf8_lossy(&result.stderr));
    std::fs::read(target.join("wasm32-wasip2/release/yoake_wasm_f2_io.wasm")).unwrap()
}

#[test]
fn sdk_guest_talks_to_its_worker_and_a_missing_worker_is_an_error_code() {
    let guest = build_sdk_guest("worker_sdk");
    // no worker declared
    runtime::uninstall().unwrap();
    runtime::load(&guest, ModConfig::full_access(), "noworker").unwrap();
    tick(runtime::HOOK_SERVER_TICK);
    let reports = runtime::take_reports().unwrap();
    assert_eq!(slot(&reports, 1), [2], "send: no worker: {reports:?}");
    assert_eq!(slot(&reports, 2), [-1], "recv: no worker: {reports:?}");
    runtime::uninstall().unwrap();

    // with a worker: the echo comes back through the channel
    let game = scratch("sdk-worker");
    let log = Log::default();
    let mut io = ModIo::new(FileAccess::new(&game, "withworker", false));
    io.worker = Some(build_worker_component());
    io.log = log.sink();
    let cfg = ModConfig { io: Some(io), ..ModConfig::full_access() };
    runtime::load(&guest, cfg, "withworker").unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    let mut echoed = false;
    while Instant::now() < until && !echoed {
        tick(runtime::HOOK_SERVER_TICK);
        let reports = runtime::take_reports().unwrap();
        assert!(slot(&reports, 1).iter().all(|status| *status == 0), "{reports:?}");
        echoed = slot(&reports, 2).contains(&4); // "ping"
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(echoed, "no echo from the worker; log: {:?}", log.lines());
    runtime::uninstall().unwrap(); // unloading stops the worker
    let _ = std::fs::remove_dir_all(game);
}
