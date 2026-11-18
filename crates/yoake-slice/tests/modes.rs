use yoake_slice::bridge::{self, Mode};
use yoake_slice::runtime::{self, ModConfig};

fn resolve(id: u32) -> Option<Mode> {
    match id {
        1 => Some(Mode::Full),
        2 => Some(Mode::ObserveOnly),
        _ => None,
    }
}

const GUEST: &str = r#"(module
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init")
    (call $report (i32.const 1) (i64.extend_i32_s (call $register (i32.const 1) (i32.const 0))))
    (call $report (i32.const 2) (i64.extend_i32_s (call $register (i32.const 2) (i32.const 0))))
    (call $report (i32.const 3) (i64.extend_i32_s (call $register (i32.const 3) (i32.const 0)))))
  (func (export "yoake_hook") (param i32 i64) (result i64) (i64.const 0)))"#;

fn statuses() -> Vec<i64> {
    let mut reports = runtime::take_reports().unwrap();
    reports.sort_unstable_by_key(|(slot, _)| *slot);
    reports.into_iter().map(|(_, status)| status).collect()
}

#[test]
fn registration_reports_the_mode_and_no_resolver_means_full() {
    bridge::set_mode_resolver(Some(resolve));
    runtime::install(GUEST.as_bytes(), ModConfig::new("")).unwrap();
    assert_eq!(statuses(), [0, 1, 2]);
    assert_eq!(runtime::stats().registered, [(1, 0), (2, 0), (3, 0)]);

    bridge::set_mode_resolver(None);
    runtime::install(GUEST.as_bytes(), ModConfig::new("")).unwrap();
    assert_eq!(statuses(), [0, 0, 0]);
    runtime::uninstall().unwrap();
}
