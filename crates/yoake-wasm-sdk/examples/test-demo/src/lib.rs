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
    let player = world.spawn_mannequin("Tester", ORIGIN)?;
    if pig == player {
        return Err("a pig and a player must not share an id".into());
    }
    world.assert_entity_count("minecraft:pig", ORIGIN, 4.0, 1)?;
    world.assert_command("say hi")?;
    // The peer module answers "<caller> said: <payload>"; a test host may answer "echo:<payload>".
    let echoed = world.mod_call("demo", "echo", b"ping")?;
    if !echoed.ends_with(b"ping") {
        return Err(format!("unexpected mod-call answer {echoed:?}"));
    }
    if world.player_names()?.is_empty() {
        return Err("the world's owner must be listed".into());
    }
    if yoake_wasm_sdk::testing::engine::client_count() != 1 {
        return Err("one client expected".into());
    }
    println!("printed from the test module");
    world.log("logged");
    let before = world.tick();
    world.run_ticks(2)?;
    if world.tick() - before != 2 {
        return Err("two ticks should have run".into());
    }
    Ok(())
}

/// Needs `--yoake-test-clients=2`: the second client sees what the server does, and the two talk.
fn two_clients(world: &mut TestWorld<'_>) -> TestResult {
    if world.client_count() != 2 {
        return Err("this test needs --yoake-test-clients=2".into());
    }
    // A server change reaches the second client.
    let at = BlockPos { x: 3, y: -60, z: 3 };
    world.place_block(at, "minecraft:stone")?;
    world.run_ticks(2)?;
    world.client(1).wait_block(at, "minecraft:stone", 20_000)?;
    world.break_block(at)?;
    world.run_ticks(2)?;
    world.client(1).wait_block(at, "minecraft:air", 20_000)?;
    // Client to server to client: a chat line the second client sends comes back through the server.
    world.client(1).chat("hello-from-b")?;
    world.run_ticks(2)?;
    world.client(1).wait_chat("hello-from-b", 20_000)?;
    // Server to client.
    // `say` from the console does not reach a client's chat log; `tellraw` does.
    world.assert_command("tellraw @a \"hello-from-the-server\"")?;
    world.run_ticks(2)?;
    world.client(1).wait_chat("hello-from-the-server", 20_000)?;
    // The server module answers while both are connected.
    let answer = world.mod_call("demo", "echo", b"two")?;
    if !answer.ends_with(b"two") {
        return Err(format!("unexpected mod-call answer {answer:?}"));
    }
    let names = world.client(1).player_names()?;
    if !names.iter().any(|name| name == "TestB") || names.len() != 2 {
        return Err(format!("the server should list the owner and TestB: {names:?}"));
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
    TestCase { name: "two_clients", run: two_clients },
    TestCase { name: "traps", run: traps },
    TestCase { name: "spins", run: spins },
];

yoake_wasm_sdk::export_tests!(YOAKE_TESTS);
