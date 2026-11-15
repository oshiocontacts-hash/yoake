use alloc::{format, string::String, vec::Vec};

/// Test modules run by the engine (`yoake test`): the host-backed [`TestDriver`] and `export_tests!`.
/// Needs the SDK feature `test`.
#[cfg(feature = "test")]
pub mod engine;

pub type TestResult = Result<(), String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// What a test may do to its world. [`engine::HostDriver`] (feature `test`) runs it in the game; a
/// mock implements it for `cargo test`. The methods after `run_ticks` have defaults that answer
/// "unsupported", so a mock implements only what its tests use.
pub trait TestDriver {
    fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult;
    fn block_at(&mut self, position: BlockPos) -> Result<String, String>;
    fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String>;
    fn run_ticks(&mut self, ticks: u32) -> TestResult;

    /// Breaks the block as a player would (drops included).
    fn break_block(&mut self, position: BlockPos) -> TestResult {
        let _ = position;
        Err(String::from("unsupported: break_block"))
    }
    /// Spawns a mannequin (not a player: it is not in `player_names`) for `name` and returns its entity id.
    fn spawn_mannequin(&mut self, name: &str, position: BlockPos) -> Result<u64, String> {
        let _ = (name, position);
        Err(String::from("unsupported: spawn_mannequin"))
    }
    /// Runs a server command as the console; whether it succeeded.
    fn run_command(&mut self, command: &str) -> Result<bool, String> {
        let _ = command;
        Err(String::from("unsupported: run_command"))
    }
    /// Calls another loaded server module's `handle-call` (the mod under test, usually).
    fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        let _ = (target, method, payload);
        Err(String::from("unsupported: mod_call"))
    }
    /// The kinds of the entities within `radius` blocks of `position`.
    fn entities_near(&mut self, position: BlockPos, radius: f64) -> Result<Vec<String>, String> {
        let _ = (position, radius);
        Err(String::from("unsupported: entities_near"))
    }
    /// The names of the players on the server, the world's owner first.
    fn player_names(&mut self) -> Result<Vec<String>, String> {
        Err(String::from("unsupported: player_names"))
    }
    /// The server ticks the world has run since the test began.
    fn tick(&mut self) -> u64 {
        0
    }
    /// A line for the test log.
    fn log(&mut self, message: &str) {
        let _ = message;
    }
}

pub struct TestWorld<'driver> {
    driver: &'driver mut dyn TestDriver,
}

impl<'driver> TestWorld<'driver> {
    pub fn new(driver: &'driver mut dyn TestDriver) -> Self {
        Self { driver }
    }

    pub fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
        self.driver.place_block(position, block)
    }

    pub fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        self.driver.block_at(position)
    }

    pub fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String> {
        self.driver.spawn_entity(kind, position)
    }

    pub fn run_ticks(&mut self, ticks: u32) -> TestResult {
        self.driver.run_ticks(ticks)
    }

    pub fn break_block(&mut self, position: BlockPos) -> TestResult {
        self.driver.break_block(position)
    }

    pub fn spawn_mannequin(&mut self, name: &str, position: BlockPos) -> Result<u64, String> {
        self.driver.spawn_mannequin(name, position)
    }

    pub fn run_command(&mut self, command: &str) -> Result<bool, String> {
        self.driver.run_command(command)
    }

    /// Runs `command` and fails the test when the server says it did not succeed.
    pub fn assert_command(&mut self, command: &str) -> TestResult {
        if self.driver.run_command(command)? {
            Ok(())
        } else {
            Err(format!("command did not succeed: {command}"))
        }
    }

    pub fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.driver.mod_call(target, method, payload)
    }

    pub fn entities_near(&mut self, position: BlockPos, radius: f64) -> Result<Vec<String>, String> {
        self.driver.entities_near(position, radius)
    }

    pub fn player_names(&mut self) -> Result<Vec<String>, String> {
        self.driver.player_names()
    }

    pub fn tick(&mut self) -> u64 {
        self.driver.tick()
    }

    pub fn log(&mut self, message: &str) {
        self.driver.log(message)
    }

    /// Fails unless exactly `count` entities of `kind` are within `radius` blocks of `position`.
    pub fn assert_entity_count(&mut self, kind: &str, position: BlockPos, radius: f64, count: usize) -> TestResult {
        let actual = self.driver.entities_near(position, radius)?.iter().filter(|found| *found == kind).count();
        if actual == count {
            Ok(())
        } else {
            Err(format!("entities {kind:?} near {position:?}: expected {count}, got {actual}"))
        }
    }

    pub fn assert_block(&mut self, position: BlockPos, expected: &str) -> TestResult {
        let actual = self.block_at(position)?;
        if actual == expected {
            Ok(())
        } else {
            Err(format!("block at {position:?}: expected {expected:?}, got {actual:?}"))
        }
    }
}

pub struct TestCase {
    pub name: &'static str,
    pub run: for<'driver> fn(&mut TestWorld<'driver>) -> TestResult,
}

impl TestCase {
    pub fn run(&self, driver: &mut dyn TestDriver) -> TestResult {
        (self.run)(&mut TestWorld::new(driver))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[derive(Default)]
    struct MockDriver {
        block: Option<(BlockPos, String)>,
        spawned: Option<(String, BlockPos)>,
        ticks: u32,
    }

    impl TestDriver for MockDriver {
        fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
            self.block = Some((position, block.to_string()));
            Ok(())
        }

        fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
            self.block
                .as_ref()
                .filter(|(at, _)| *at == position)
                .map(|(_, block)| block.clone())
                .ok_or_else(|| "missing block".to_string())
        }

        fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String> {
            self.spawned = Some((kind.to_string(), position));
            Ok(17)
        }

        fn run_ticks(&mut self, ticks: u32) -> TestResult {
            self.ticks += ticks;
            Ok(())
        }
    }

    fn scenario(world: &mut TestWorld<'_>) -> TestResult {
        let position = BlockPos { x: 1, y: 2, z: 3 };
        world.place_block(position, "minecraft:stone")?;
        if world.spawn_entity("minecraft:pig", position)? != 17 {
            return Err("unexpected entity id".to_string());
        }
        world.run_ticks(2)?;
        world.assert_block(position, "minecraft:stone")
    }

    #[test]
    fn executes_registered_test_against_driver() {
        let mut driver = MockDriver::default();
        let case = TestCase { name: "scenario", run: scenario };
        assert_eq!(case.run(&mut driver), Ok(()));
        assert_eq!(driver.spawned, Some(("minecraft:pig".to_string(), BlockPos { x: 1, y: 2, z: 3 })));
        assert_eq!(driver.ticks, 2);
    }

    #[test]
    fn block_assertion_reports_expected_and_actual() {
        let position = BlockPos { x: 1, y: 2, z: 3 };
        let mut driver = MockDriver {
            block: Some((position, "minecraft:dirt".to_string())),
            ..Default::default()
        };
        assert_eq!(
            TestWorld::new(&mut driver).assert_block(position, "minecraft:stone"),
            Err("block at BlockPos { x: 1, y: 2, z: 3 }: expected \"minecraft:stone\", got \"minecraft:dirt\"".to_string())
        );
    }

    #[test]
    fn block_assertion_propagates_driver_errors() {
        let mut driver = MockDriver::default();
        assert_eq!(
            TestWorld::new(&mut driver).assert_block(BlockPos { x: 1, y: 2, z: 3 }, "minecraft:stone"),
            Err("missing block".to_string())
        );
    }
}
