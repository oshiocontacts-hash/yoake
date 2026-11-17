//! The test-mod world with the `test-demo` guest (built by `yoake_wasm::test_guests`): a fake test
//! host stands in for the engine.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yoake_wasm::test_driver::{BlockPos, EntityInfo, PlayerInfo, Vec3};
use yoake_wasm::{Sandbox, TestHost, TestMod, TestOutcome};

#[derive(Default, Clone)]
struct Calls(Arc<Mutex<Vec<String>>>);

struct World {
    calls: Calls,
    block: Option<(BlockPos, String)>,
    ticks: u64,
}

impl World {
    fn new(calls: &Calls) -> Self {
        Self { calls: calls.clone(), block: None, ticks: 0 }
    }
    fn note(&self, line: String) {
        self.calls.0.lock().unwrap().push(line);
    }
}

impl TestHost for World {
    fn advance_ticks(&mut self, ticks: u32) -> Result<(), String> {
        self.ticks += u64::from(ticks);
        self.note(format!("advance {ticks}"));
        Ok(())
    }
    fn tick(&mut self) -> u64 {
        self.ticks
    }
    fn place_block(&mut self, position: BlockPos, block: &str) -> Result<(), String> {
        self.note(format!("place {},{},{} {block}", position.x, position.y, position.z));
        self.block = Some((position, block.to_owned()));
        Ok(())
    }
    fn break_block(&mut self, position: BlockPos) -> Result<(), String> {
        self.note(format!("break {},{},{}", position.x, position.y, position.z));
        self.block = None;
        Ok(())
    }
    fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        match &self.block {
            Some((at, block)) if (at.x, at.y, at.z) == (position.x, position.y, position.z) => Ok(block.clone()),
            _ => Ok("minecraft:air".to_owned()),
        }
    }
    fn spawn_entity(&mut self, kind: &str, position: Vec3) -> Result<u32, String> {
        self.note(format!("spawn {kind} at {},{},{}", position.x, position.y, position.z));
        Ok(10)
    }
    fn spawn_mannequin(&mut self, name: &str, _: Vec3) -> Result<u32, String> {
        self.note(format!("mannequin {name}"));
        Ok(11)
    }
    fn entities_near(&mut self, center: Vec3, _: f64) -> Result<Vec<EntityInfo>, String> {
        Ok(vec![EntityInfo { id: 10, kind: "minecraft:pig".into(), position: center }])
    }
    fn players(&mut self) -> Result<Vec<PlayerInfo>, String> {
        Ok(vec![PlayerInfo {
            uuid: "00000000-0000-0000-0000-000000000001".into(),
            name: "Owner".into(),
            position: Vec3 { x: 0.0, y: 64.0, z: 0.0 },
            health: 20.0,
            game_mode: 1,
        }])
    }
    fn run_command(&mut self, command: &str) -> Result<bool, String> {
        self.note(format!("command {command}"));
        Ok(true)
    }
    fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.note(format!("mod-call {target} {method}"));
        let mut answer = b"echo:".to_vec();
        answer.extend_from_slice(payload);
        Ok(answer)
    }
    fn log(&mut self, message: &str) {
        self.note(format!("log {message}"));
    }
}

fn load(calls: &Calls, per_test: Duration) -> Option<TestMod> {
    let bytes = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::TEST_DEMO)?;
    Some(TestMod::instantiate(&mut Sandbox::new(), "demo-tests", &bytes, Box::new(World::new(calls)), per_test).expect("the test module loads"))
}

#[test]
fn the_module_lists_its_tests_and_runs_them_against_the_host() {
    let calls = Calls::default();
    let Some(mut module) = load(&calls, Duration::from_secs(20)) else { return };
    assert_eq!(module.tests().unwrap(), ["passes", "fails", "driver_roundtrip", "two_clients", "traps", "spins"]);

    assert_eq!(module.run_test("passes"), TestOutcome::Pass);
    assert_eq!(*calls.0.lock().unwrap(), ["place 1,64,2 minecraft:stone", "advance 3"]);

    // An assertion failure is the test's own message, and the module keeps running.
    match module.run_test("fails") {
        TestOutcome::Fail(message) => assert!(message.contains("expected \"minecraft:stone\""), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(!module.is_disabled());
    match module.run_test("no-such-test") {
        TestOutcome::Fail(message) => assert!(message.contains("no-such-test"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_driver_call_reaches_the_host() {
    let calls = Calls::default();
    let Some(mut module) = load(&calls, Duration::from_secs(20)) else { return };
    assert_eq!(module.run_test("driver_roundtrip"), TestOutcome::Pass);
    // Without a second client the two-client test says what it needs.
    match module.run_test("two_clients") {
        TestOutcome::Fail(message) => assert!(message.contains("--yoake-test-clients=2"), "{message}"),
        other => panic!("{other:?}"),
    }
    let calls = calls.0.lock().unwrap();
    for expected in [
        "break 1,64,2",
        "spawn minecraft:pig at 1.5,64,2.5",
        "mannequin Tester",
        "command say hi",
        "mod-call demo echo",
        "log logged",
        "log printed from the test module",
    ] {
        assert!(calls.iter().any(|line| line == expected), "{expected} missing from {calls:?}");
    }
}

#[test]
fn a_trap_ends_the_module_but_not_the_host() {
    let calls = Calls::default();
    let Some(mut module) = load(&calls, Duration::from_secs(20)) else { return };
    match module.run_test("traps") {
        TestOutcome::Trap(message) => assert!(!message.is_empty()),
        other => panic!("{other:?}"),
    }
    assert!(module.is_disabled());
    // Later tests of a disabled module do not run: they report the trap.
    assert!(matches!(module.run_test("passes"), TestOutcome::Trap(_)));
}

#[test]
fn a_test_that_never_returns_is_stopped_by_the_clock() {
    let calls = Calls::default();
    let Some(mut module) = load(&calls, Duration::from_millis(300)) else { return };
    let began = Instant::now();
    match module.run_test("spins") {
        TestOutcome::Trap(message) => assert!(!message.is_empty()),
        other => panic!("{other:?}"),
    }
    assert!(began.elapsed() < Duration::from_secs(10), "stopped after {:?}", began.elapsed());
    assert!(module.is_disabled());
}

#[test]
fn a_mod_component_cannot_import_the_test_driver() {
    // The test module is a valid component, but `Sandbox::compile` (the path every mod takes) refuses
    // its `test-driver` import: a mod cannot reach the driver.
    let Some(bytes) = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::TEST_DEMO) else { return };
    let error = Sandbox::new().compile(&bytes).err().expect("a normal compile must refuse the test driver");
    assert!(error.contains("test-driver"), "{error}");
}
