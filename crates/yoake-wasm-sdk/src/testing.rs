use alloc::{format, string::String};

pub type TestResult = Result<(), String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

pub trait TestDriver {
    fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult;
    fn block_at(&mut self, position: BlockPos) -> Result<String, String>;
    fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String>;
    fn run_ticks(&mut self, ticks: u32) -> TestResult;
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
