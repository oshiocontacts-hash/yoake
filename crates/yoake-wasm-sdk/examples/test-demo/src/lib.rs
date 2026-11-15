//! A test module (world `test-mod`) for the engine's tests of the test runner: it passes, fails, traps
//! and spins, and exercises every call of the test driver.

use yoake_wasm_sdk::testing::{BlockPos, TestCase, TestResult, TestWorld};

const ORIGIN: BlockPos = BlockPos { x: 1, y: 64, z: 2 };

fn passes(world: &mut TestWorld<'_>) -> TestResult {
    world.place_block(ORIGIN, "minecraft:stone")?;
    world.run_ticks(3)?;
    world.assert_block(ORIGIN, "minecraft:stone")
}

fn fails(world: &mut TestWorld<'_>) -> TestResult {
    world.place_block(ORIGIN, "minecraft:dirt")?;
    world.assert_block(ORIGIN, "minecraft:stone")
}

fn driver_roundtrip(world: &mut TestWorld<'_>) -> TestResult {
    world.break_block(ORIGIN)?;
    let pig = world.spawn_entity("minecraft:pig", ORIGIN)?;
    let player = world.spawn_fake_player("Tester", ORIGIN)?;
    if pig == player {
        return Err("a pig and a player must not share an id".into());
    }
    world.assert_entity_count("minecraft:pig", ORIGIN, 4.0, 1)?;
    world.assert_command("say hi")?;
    let echoed = world.mod_call("demo", "echo", b"ping")?;
    if echoed != b"echo:ping" {
        return Err(format!("unexpected mod-call answer {echoed:?}"));
    }
    if world.player_names()?.first().map(String::as_str) != Some("Owner") {
        return Err("the owner must be the first player".into());
    }
    if yoake_wasm_sdk::testing::engine::client_count() != 1 {
        return Err("one client expected".into());
    }
    println!("printed from the test module");
    world.log("logged");
    if world.tick() != 7 {
        return Err("tick should be 7".into());
    }
    Ok(())
}

fn traps(_: &mut TestWorld<'_>) -> TestResult {
    panic!("the test module panics on purpose");
}

fn spins(_: &mut TestWorld<'_>) -> TestResult {
    let mut counter = 0u64;
    loop {
        counter = std::hint::black_box(counter.wrapping_add(1));
    }
}

static YOAKE_TESTS: &[TestCase] = &[
    TestCase { name: "passes", run: passes },
    TestCase { name: "fails", run: fails },
    TestCase { name: "driver_roundtrip", run: driver_roundtrip },
    TestCase { name: "traps", run: traps },
    TestCase { name: "spins", run: spins },
];

yoake_wasm_sdk::export_tests!(YOAKE_TESTS);
