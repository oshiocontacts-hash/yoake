//! Test modules: components of the WIT world `test-mod` (`wit/yoake-test.wit`) that the engine's test
//! runner (`horizon-client --yoake-test`) loads next to the mod under test.
//!
//! A test module is *not* a mod. It exports `tests` and `run-test` and imports `test-driver`, an
//! interface no other world links ([`crate::Sandbox::compile`] rejects a component that imports it), so
//! a mod can neither use the driver nor be started as a test module: the only way to get a [`TestMod`]
//! is [`TestMod::instantiate`], which the engine calls only in its test process. It runs under the
//! same memory, table and WASI limits as every module, with a longer clock limit per test (the test
//! waits for the world to tick) and no aggregate budget.
use std::time::Duration;

use wasmtime::component::HasSelf;

use crate::{HostState, LoadError, ModInstance, OutputLine, Sandbox, TimeLimits};

mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "test-mod",
    });
}

pub use bindings::yoake::mods::test_driver;
use test_driver::{BlockPos, EntityInfo, PlayerInfo, Vec3};

/// Most tests one module may declare.
pub const MAX_TESTS: usize = 1024;
/// Longest test name, in bytes.
pub const MAX_TEST_NAME: usize = 128;
/// The most ticks one `advance-ticks` call may ask for (one hour of game time).
pub const MAX_ADVANCE_TICKS: u32 = 72_000;

const UNSUPPORTED: &str = "unsupported: this test host does not provide it";

fn unsupported<T>() -> Result<T, String> {
    Err(UNSUPPORTED.to_owned())
}

/// The host side of `test-driver`. Every method answers "unsupported" unless a host overrides it, so a
/// test double implements only what its test calls. The engine's host runs each call on its server
/// thread (`horizon-client/src/yoake_test.rs`).
pub trait TestHost: Send {
    fn advance_ticks(&mut self, ticks: u32) -> Result<(), String> {
        let _ = ticks;
        unsupported()
    }
    fn tick(&mut self) -> u64 {
        0
    }
    fn place_block(&mut self, position: BlockPos, block: &str) -> Result<(), String> {
        let _ = (position, block);
        unsupported()
    }
    fn break_block(&mut self, position: BlockPos) -> Result<(), String> {
        let _ = position;
        unsupported()
    }
    fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        let _ = position;
        unsupported()
    }
    fn spawn_entity(&mut self, kind: &str, position: Vec3) -> Result<u32, String> {
        let _ = (kind, position);
        unsupported()
    }
    fn spawn_mannequin(&mut self, name: &str, position: Vec3) -> Result<u32, String> {
        let _ = (name, position);
        unsupported()
    }
    fn entities_near(&mut self, center: Vec3, radius: f64) -> Result<Vec<EntityInfo>, String> {
        let _ = (center, radius);
        unsupported()
    }
    fn players(&mut self) -> Result<Vec<PlayerInfo>, String> {
        unsupported()
    }
    fn run_command(&mut self, command: &str) -> Result<bool, String> {
        let _ = command;
        unsupported()
    }
    fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        let _ = (target, method, payload);
        unsupported()
    }
    fn log(&mut self, message: &str) {
        let _ = message;
    }
    fn client_count(&mut self) -> u32 {
        1
    }
    fn client_call(&mut self, index: u32, command: &str) -> Result<String, String> {
        let _ = (index, command);
        unsupported()
    }
    fn client_wait(
        &mut self,
        index: u32,
        command: &str,
        expect: &str,
        timeout: Duration,
    ) -> Result<String, String> {
        let _ = (index, command, expect, timeout);
        unsupported()
    }
    fn client_player_names(&mut self, index: u32) -> Result<Vec<String>, String> {
        let _ = index;
        unsupported()
    }
}

/// The longest a `client-wait` may wait.
pub const MAX_CLIENT_WAIT: Duration = Duration::from_secs(120);

struct TestContext {
    host: Box<dyn TestHost>,
}

impl test_driver::Host for TestContext {
    fn advance_ticks(&mut self, ticks: u32) -> Result<(), String> {
        if ticks == 0 || ticks > MAX_ADVANCE_TICKS {
            return Err(format!(
                "advance-ticks takes 1 to {MAX_ADVANCE_TICKS} ticks"
            ));
        }
        self.host.advance_ticks(ticks)
    }
    fn tick(&mut self) -> u64 {
        self.host.tick()
    }
    fn place_block(&mut self, position: BlockPos, block: String) -> Result<(), String> {
        self.host.place_block(position, &block)
    }
    fn break_block(&mut self, position: BlockPos) -> Result<(), String> {
        self.host.break_block(position)
    }
    fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        self.host.block_at(position)
    }
    fn spawn_entity(&mut self, kind: String, position: Vec3) -> Result<u32, String> {
        finite(&position)?;
        self.host.spawn_entity(&kind, position)
    }
    fn spawn_mannequin(&mut self, name: String, position: Vec3) -> Result<u32, String> {
        finite(&position)?;
        self.host.spawn_mannequin(&name, position)
    }
    fn entities_near(&mut self, center: Vec3, radius: f64) -> Result<Vec<EntityInfo>, String> {
        finite(&center)?;
        if !radius.is_finite() || radius <= 0.0 {
            return Err("the radius must be a positive finite number".into());
        }
        self.host.entities_near(center, radius)
    }
    fn players(&mut self) -> Result<Vec<PlayerInfo>, String> {
        self.host.players()
    }
    fn run_command(&mut self, command: String) -> Result<bool, String> {
        if command.chars().any(char::is_control) {
            return Err("a command is one line of plain text".into());
        }
        self.host.run_command(&command)
    }
    fn mod_call(
        &mut self,
        target: String,
        method: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        self.host.mod_call(&target, &method, &payload)
    }
    fn log(&mut self, message: String) {
        self.host.log(&message);
    }
    fn client_count(&mut self) -> u32 {
        self.host.client_count()
    }
    fn client_call(&mut self, index: u32, command: String) -> Result<String, String> {
        self.host.client_call(index, &command)
    }
    fn client_wait(
        &mut self,
        index: u32,
        command: String,
        expect: String,
        timeout_ms: u32,
    ) -> Result<String, String> {
        let timeout = Duration::from_millis(u64::from(timeout_ms));
        if expect.is_empty() || timeout.is_zero() || timeout > MAX_CLIENT_WAIT {
            return Err(format!(
                "client-wait needs a text to wait for and 1 to {} ms",
                MAX_CLIENT_WAIT.as_millis()
            ));
        }
        self.host.client_wait(index, &command, &expect, timeout)
    }
    fn client_player_names(&mut self, index: u32) -> Result<Vec<String>, String> {
        self.host.client_player_names(index)
    }
}

fn finite(position: &Vec3) -> Result<(), String> {
    if position.x.is_finite() && position.y.is_finite() && position.z.is_finite() {
        Ok(())
    } else {
        Err("positions must be finite numbers".into())
    }
}

/// How one test ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TestOutcome {
    Pass,
    /// The test returned `err(message)`: an assertion failed.
    Fail(String),
    /// The module trapped, ran out of fuel or time, or exited: the module is disabled and runs no more tests.
    Trap(String),
}

impl TestOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// Checks the names a test module declares: 1 to [`MAX_TESTS`] names, each 1 to [`MAX_TEST_NAME`] bytes of
/// plain text without control characters and not starting with `-`, no duplicates. Returns them unchanged.
pub fn validate_test_names(names: Vec<String>) -> Result<Vec<String>, String> {
    if names.len() > MAX_TESTS {
        return Err(format!("a test module declares at most {MAX_TESTS} tests"));
    }
    let mut seen = std::collections::HashSet::new();
    for name in &names {
        // A name never starts with '-': it travels on a command line, where it would read as an option.
        if name.is_empty()
            || name.len() > MAX_TEST_NAME
            || name.starts_with('-')
            || name.chars().any(char::is_control)
        {
            return Err(format!(
                "not a valid test name: {:?}",
                name.chars().take(40).collect::<String>()
            ));
        }
        if !seen.insert(name.as_str()) {
            return Err(format!("duplicate test name: {name}"));
        }
    }
    Ok(names)
}

/// A loaded test module.
pub struct TestMod {
    raw: ModInstance<TestContext>,
    exports: bindings::TestMod,
}

impl TestMod {
    /// Instantiates the component with `host` as the test driver. `per_test` is how long one test may
    /// run (wall-clock, waiting for the world included); a test over it traps the module.
    pub fn instantiate(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        host: Box<dyn TestHost>,
        per_test: Duration,
    ) -> Result<Self, LoadError> {
        let id = id.into();
        let mut raw = runtime
            .compile_test(component_bytes)
            .map_err(LoadError::new)?
            .instantiate(id, TestContext { host }, |linker| {
                bindings::TestMod::add_to_linker::<HostState<TestContext>, HasSelf<TestContext>>(
                    linker,
                    |state| &mut state.host,
                )
                .map_err(|error| format!("{error:#}"))
            })
            .map_err(LoadError::new)?;
        let exports = bindings::TestMod::new(&mut raw.store, &raw.instance)
            .map_err(|error| LoadError::new(format!("invalid test-mod exports: {error:#}")))?;
        raw.set_limits(TimeLimits {
            call: per_test,
            aggregate: Duration::MAX,
        });
        // Fuel is the CPU bound; the clock limit above is the wall-clock one.
        raw.set_fuel_per_call(i64::MAX as u64 / 2);
        Ok(Self { raw, exports })
    }

    /// The test names the module declares, validated ([`validate_test_names`]).
    pub fn tests(&mut self) -> Result<Vec<String>, String> {
        let exports = &self.exports;
        let mut trap = None;
        let names = self.raw.call("tests", |store, _| {
            exports.call_tests(store).map_err(|error| {
                let text = format!("{error:#}");
                trap = Some(text.clone());
                text
            })
        });
        self.flush();
        match names {
            Some(names) => validate_test_names(names),
            None => Err(format!(
                "the test module trapped listing its tests: {}",
                trap.unwrap_or_default()
            )),
        }
    }

    /// Runs one test. A trap disables the module (every later call answers `Trap`).
    pub fn run_test(&mut self, name: &str) -> TestOutcome {
        if self.raw.is_disabled() {
            return TestOutcome::Trap("the test module was disabled by an earlier trap".into());
        }
        let exports = &self.exports;
        let mut trap = None;
        let result = self.raw.call("run_test", |store, _| {
            exports.call_run_test(store, name).map_err(|error| {
                let text = format!("{error:#}");
                trap = Some(text.clone());
                text
            })
        });
        self.flush();
        match result {
            Some(Ok(())) => TestOutcome::Pass,
            Some(Err(message)) => TestOutcome::Fail(message),
            None => TestOutcome::Trap(trap.unwrap_or_else(|| "the test module is disabled".into())),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.raw.is_disabled()
    }

    /// What the module printed (WASI stdout and stderr) since the last call, by line.
    pub fn drain_output(&mut self) -> Vec<OutputLine> {
        self.raw.drain_output()
    }

    fn flush(&mut self) {
        for line in self.raw.drain_output() {
            self.raw.store.data_mut().host.host.log(&line.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn test_names_are_validated() {
        assert_eq!(
            validate_test_names(names(&["a", "b c"])).unwrap(),
            names(&["a", "b c"])
        );
        assert_eq!(
            validate_test_names(Vec::new()).unwrap(),
            Vec::<String>::new()
        );
        assert!(validate_test_names(names(&[""])).is_err());
        assert!(validate_test_names(names(&["a\nb"])).is_err());
        assert!(
            validate_test_names(names(&["a", "a"]))
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(validate_test_names(vec!["x".repeat(MAX_TEST_NAME)]).is_ok());
        assert!(validate_test_names(vec!["x".repeat(MAX_TEST_NAME + 1)]).is_err());
        assert!(
            validate_test_names((0..=MAX_TESTS).map(|index| index.to_string()).collect()).is_err()
        );
    }

    #[test]
    fn a_plain_mod_cannot_import_the_driver() {
        // `yoake:mods/test-driver` is allowed only by `compile_test`.
        assert!(
            crate::check_import("yoake:mods/test-driver@1.0.0", false, false)
                .unwrap_err()
                .contains("test runner")
        );
        assert!(crate::check_import("yoake:mods/test-driver@1.0.0", true, false).is_ok());
        assert!(crate::check_import("yoake:mods/test-driver@1.7.0", true, false).is_ok());
        assert!(crate::check_import("yoake:mods/logging@1.0.0", false, false).is_ok());
        assert!(crate::check_import("yoake:mods/logging@1.7.0", false, false).is_ok());
    }

    #[test]
    fn the_driver_checks_what_it_forwards() {
        struct Spy(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
        impl TestHost for Spy {
            fn advance_ticks(&mut self, ticks: u32) -> Result<(), String> {
                self.0.lock().unwrap().push(format!("advance {ticks}"));
                Ok(())
            }
            fn run_command(&mut self, command: &str) -> Result<bool, String> {
                self.0.lock().unwrap().push(format!("command {command}"));
                Ok(true)
            }
            fn spawn_entity(&mut self, kind: &str, _: Vec3) -> Result<u32, String> {
                self.0.lock().unwrap().push(format!("spawn {kind}"));
                Ok(3)
            }
        }
        use test_driver::Host as _;
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut context = TestContext {
            host: Box::new(Spy(calls.clone())),
        };
        assert!(context.advance_ticks(0).is_err());
        assert!(context.advance_ticks(MAX_ADVANCE_TICKS + 1).is_err());
        assert_eq!(context.advance_ticks(5), Ok(()));
        assert!(context.run_command("say hi\nstop".into()).is_err());
        assert_eq!(context.run_command("say hi".into()), Ok(true));
        let here = Vec3 {
            x: 0.0,
            y: 64.0,
            z: 0.0,
        };
        assert!(
            context
                .spawn_entity(
                    "minecraft:pig".into(),
                    Vec3 {
                        x: f64::NAN,
                        ..here
                    }
                )
                .is_err()
        );
        assert_eq!(context.spawn_entity("minecraft:pig".into(), here), Ok(3));
        assert!(context.entities_near(here, -1.0).is_err());
        assert!(context.entities_near(here, f64::INFINITY).is_err());
        assert!(
            context
                .client_wait(1, "status".into(), String::new(), 100)
                .is_err()
        );
        assert!(
            context
                .client_wait(1, "status".into(), "x".into(), 0)
                .is_err()
        );
        assert!(
            context
                .client_wait(1, "status".into(), "x".into(), 1_000_000)
                .is_err()
        );
        assert!(
            context
                .client_call(1, "status".into())
                .unwrap_err()
                .starts_with("unsupported")
        );
        // What a host does not implement answers "unsupported".
        assert!(
            context
                .block_at(BlockPos { x: 0, y: 0, z: 0 })
                .unwrap_err()
                .starts_with("unsupported")
        );
        assert_eq!(
            *calls.lock().unwrap(),
            ["advance 5", "command say hi", "spawn minecraft:pig"]
        );
    }
}
