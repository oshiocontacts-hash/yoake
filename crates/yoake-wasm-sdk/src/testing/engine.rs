//! Tests that run in the engine (feature `test`): the world `test-mod` of `crates/yoake-wasm/wit/yoake-test.wit`.
//!
//! A test module is its own component, built apart from the mod (a Cargo feature of your crate, for
//! example `tests`), and run by `yoake test` or `horizon-client --yoake-test`; it is never part of a
//! released `.yak`.
//!
//! ```ignore
//! use yoake_wasm_sdk::testing::{BlockPos, TestCase, TestResult, TestWorld};
//!
//! fn places_a_block(world: &mut TestWorld<'_>) -> TestResult {
//!     let at = BlockPos { x: 0, y: 64, z: 0 };
//!     world.place_block(at, "minecraft:stone")?;
//!     world.run_ticks(2)?;
//!     world.assert_block(at, "minecraft:stone")
//! }
//!
//! static YOAKE_TESTS: &[TestCase] = &[TestCase { name: "places_a_block", run: places_a_block }];
//! yoake_wasm_sdk::export_tests!(YOAKE_TESTS);
//! ```
use alloc::{format, string::{String, ToString}, vec::Vec};

use super::{BlockPos, TestCase, TestDriver, TestResult};

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../yoake-wasm/wit",
        world: "test-mod",
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "yoake_wasm_sdk::testing::engine::bindings",
    });
}

use bindings::yoake::mods::test_driver as host;

fn center(position: BlockPos) -> host::Vec3 {
    host::Vec3 { x: f64::from(position.x) + 0.5, y: f64::from(position.y), z: f64::from(position.z) + 0.5 }
}

fn pos(position: BlockPos) -> host::BlockPos {
    host::BlockPos { x: position.x, y: position.y, z: position.z }
}

/// The [`TestDriver`] of a test module: every call goes to the engine's test host.
pub struct HostDriver;

impl TestDriver for HostDriver {
    fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
        host::place_block(pos(position), block)
    }

    fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        host::block_at(pos(position))
    }

    fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String> {
        host::spawn_entity(kind, center(position)).map(u64::from)
    }

    fn run_ticks(&mut self, ticks: u32) -> TestResult {
        host::advance_ticks(ticks)
    }

    fn break_block(&mut self, position: BlockPos) -> TestResult {
        host::break_block(pos(position))
    }

    fn spawn_fake_player(&mut self, name: &str, position: BlockPos) -> Result<u64, String> {
        host::spawn_fake_player(name, center(position)).map(u64::from)
    }

    fn run_command(&mut self, command: &str) -> Result<bool, String> {
        host::run_command(command)
    }

    fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        host::mod_call(target, method, payload)
    }

    fn entities_near(&mut self, position: BlockPos, radius: f64) -> Result<Vec<String>, String> {
        host::entities_near(center(position), radius).map(|found| found.into_iter().map(|entity| entity.kind).collect())
    }

    fn player_names(&mut self) -> Result<Vec<String>, String> {
        host::players().map(|players| players.into_iter().map(|player| player.name).collect())
    }

    fn tick(&mut self) -> u64 {
        host::tick()
    }

    fn log(&mut self, message: &str) {
        host::log(message)
    }
}

/// How many clients take part (1 unless the run asked for more).
pub fn client_count() -> u32 {
    host::client_count()
}

#[doc(hidden)]
pub fn names(tests: &[TestCase]) -> Vec<String> {
    tests.iter().map(|case| case.name.to_string()).collect()
}

#[doc(hidden)]
pub fn run(tests: &[TestCase], name: &str) -> Result<(), String> {
    run_with(tests, name, &mut HostDriver)
}

/// Runs the test called `name` against `driver`.
pub fn run_with(tests: &[TestCase], name: &str, driver: &mut dyn TestDriver) -> Result<(), String> {
    match tests.iter().find(|case| case.name == name) {
        Some(case) => case.run(driver),
        None => Err(format!("no test named {name:?}")),
    }
}

/// Exports `tests` (a `&'static [TestCase]`) as the test module's `tests` and `run-test`.
#[macro_export]
macro_rules! export_tests {
    ($tests:expr) => {
        const _: () = {
            $crate::__export_realloc!();

            struct Shim;

            impl $crate::testing::engine::bindings::Guest for Shim {
                fn tests() -> $crate::__private::Vec<$crate::__private::String> {
                    $crate::testing::engine::names($tests)
                }

                fn run_test(name: $crate::__private::String) -> ::core::result::Result<(), $crate::__private::String> {
                    $crate::testing::engine::run($tests, &name)
                }
            }

            $crate::testing::engine::bindings::export!(Shim with_types_in $crate::testing::engine::bindings);
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_test_name_fails_with_its_name() {
        struct Nothing;
        impl TestDriver for Nothing {
            fn place_block(&mut self, _: BlockPos, _: &str) -> TestResult { Ok(()) }
            fn block_at(&mut self, _: BlockPos) -> Result<String, String> { Ok(String::new()) }
            fn spawn_entity(&mut self, _: &str, _: BlockPos) -> Result<u64, String> { Ok(0) }
            fn run_ticks(&mut self, _: u32) -> TestResult { Ok(()) }
        }
        assert_eq!(run_with(&[], "missing", &mut Nothing), Err("no test named \"missing\"".to_string()));
    }
}
