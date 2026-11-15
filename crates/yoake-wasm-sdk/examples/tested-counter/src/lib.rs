//! A small beginner example pairing a client Wasm mod (`TestedCounter`) with engine tests
//! (`YOAKE_TESTS`) in the same crate.
//! - The `#[yoake_test]` tests compile into a separate `test-mod` component (`--features tests`)
//!   that exercises the engine's server-side world through `TestWorld`; they do not load or drive
//!   the `TestedCounter` client mod's hooks (`tick` / `hud`).
//! - Running `cargo test` on the host runs `YOAKE_TESTS` against a local `MockWorld` (`TestDriver`
//!   mock), not the game engine.

use yoake_wasm_sdk::testing::{BlockPos, TestCase, TestResult, TestWorld};
use yoake_wasm_sdk::{Mod, hook, hud, yoake_test};

/// Target position right above the superflat grass layer (`y = -61` is assumed; checked by `ground_is_grass`).
const ORIGIN: BlockPos = BlockPos { x: 0, y: -60, z: 0 };

/// A tiny client mod that counts ticks and displays the counter on the HUD.
#[derive(Default)]
pub struct TestedCounter {
    /// Number of client ticks observed since `init`.
    pub ticks: u64,
}

impl Mod for TestedCounter {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::TICK, hook::HUD]
    }

    fn tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    fn hud(&mut self) {
        let _ = hud::text(&format!("tested-counter: {}", self.ticks), (10.0, 10.0), 0xffffff, true);
    }
}

/// Calibration test: pins the superflat grass height (`y = -61`) that the docs leave open.
#[yoake_test]
fn ground_is_grass(world: &mut TestWorld<'_>) -> TestResult {
    world.assert_block(BlockPos { x: 0, y: -61, z: 0 }, "minecraft:grass_block")
}

/// Places a stone block in the frozen superflat world, advances 2 ticks, and checks its ID.
#[yoake_test]
fn places_and_asserts_block(world: &mut TestWorld<'_>) -> TestResult {
    world.place_block(ORIGIN, "minecraft:stone")?;
    world.run_ticks(2)?;
    world.assert_block(ORIGIN, "minecraft:stone")
}

/// Advances server ticks, verifies the world tick counter, and spawns/counts an entity in the engine.
#[yoake_test]
fn advances_ticks_and_spawns_entity(world: &mut TestWorld<'_>) -> TestResult {
    let t0 = world.tick();
    world.run_ticks(5)?;
    if world.tick() < t0 + 5 {
        return Err(format!("expected world tick >= {}, got {}", t0 + 5, world.tick()));
    }
    world.spawn_entity("minecraft:pig", ORIGIN)?;
    world.assert_entity_count("minecraft:pig", ORIGIN, 4.0, 1)?;
    world.log("advanced 5 ticks and spawned a pig");
    Ok(())
}

/// Places planks, verifies the block ID, breaks the block, and confirms it becomes air.
#[yoake_test]
fn places_and_breaks_block(world: &mut TestWorld<'_>) -> TestResult {
    world.place_block(ORIGIN, "minecraft:oak_planks")?;
    world.run_ticks(1)?;
    world.assert_block(ORIGIN, "minecraft:oak_planks")?;
    world.break_block(ORIGIN)?;
    world.run_ticks(1)?;
    world.assert_block(ORIGIN, "minecraft:air")
}

/// Registry of all tests exported when built with `--features tests`.
pub const YOAKE_TESTS: &[TestCase] = &[
    TestCase { name: "ground_is_grass", run: ground_is_grass },
    TestCase { name: "places_and_asserts_block", run: places_and_asserts_block },
    TestCase { name: "advances_ticks_and_spawns_entity", run: advances_ticks_and_spawns_entity },
    TestCase { name: "places_and_breaks_block", run: places_and_breaks_block },
];

// Export the client mod component (`client-mod` world) on normal builds.
#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_mod!(TestedCounter);

// Export the test module component (`test-mod` world) when `--features tests` is enabled.
#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);

#[cfg(test)]
mod tests {
    use super::*;
    use yoake_wasm_sdk::testing::TestDriver;

    #[derive(Default)]
    struct MockWorld {
        block: Option<(BlockPos, String)>,
        entities: Vec<(String, BlockPos)>,
        ticks: u64,
    }

    impl TestDriver for MockWorld {
        fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
            self.block = Some((position, block.to_owned()));
            Ok(())
        }
        fn break_block(&mut self, position: BlockPos) -> TestResult {
            self.block = Some((position, String::from("minecraft:air")));
            Ok(())
        }
        fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
            if let Some((_, block)) = self.block.as_ref().filter(|(at, _)| *at == position) {
                return Ok(block.clone());
            }
            if position.y == -61 {
                return Ok(String::from("minecraft:grass_block"));
            }
            Err(String::from("missing block"))
        }
        fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String> {
            self.entities.push((kind.to_owned(), position));
            Ok(self.entities.len() as u64)
        }
        fn entities_near(&mut self, position: BlockPos, _radius: f64) -> Result<Vec<String>, String> {
            Ok(self.entities.iter().filter(|(_, at)| *at == position).map(|(kind, _)| kind.clone()).collect())
        }
        fn run_ticks(&mut self, ticks: u32) -> TestResult {
            self.ticks += u64::from(ticks);
            Ok(())
        }
        fn tick(&mut self) -> u64 {
            self.ticks
        }
    }

    #[test]
    fn runs_all_registered_tests_against_mock() {
        for case in YOAKE_TESTS {
            let mut mock = MockWorld::default();
            assert_eq!(case.run(&mut mock), Ok(()), "test {} failed", case.name);
        }
    }
}
