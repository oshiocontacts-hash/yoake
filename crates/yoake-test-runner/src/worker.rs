//! A worker's test thread: loads the test module, waits for the world's owner to join, runs one test and
//! reports it. The game starts it once its integrated server is ready ([`start`]); the process exits with
//! the test's verdict.
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use yoake_wasm::{Sandbox, TestMod, TestOutcome};

use crate::bridge::{ChannelHost, Client, Op, Reply};
use crate::plan::WorkerArgs;
use crate::report::TestRecord;

/// Longest a worker waits for the world's owner to join before the test starts.
pub const OWNER_WAIT: Duration = Duration::from_secs(120);
/// Extra time the watchdog gives after the test's own limit before it ends the process.
const GRACE: Duration = Duration::from_secs(15);

fn read_module(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let limit = horizon_mods::yak::MAX_WASM_SIZE;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} is larger than 32 MiB", path.display()));
    }
    Ok(bytes)
}

/// Waits until the server has a player (the owner joins through the window thread, after the server
/// thread is up), or `limit` passes.
fn wait_for_owner(client: &Client, limit: Duration) -> Result<(), String> {
    let deadline = Instant::now() + limit;
    loop {
        match client.request_within(Op::Players, Duration::from_secs(5)) {
            Ok(Reply::Players(players)) if !players.is_empty() => return Ok(()),
            Ok(_) | Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(_) => return Err("no player joined the test world".into()),
            Err(error) => return Err(format!("the test world never answered: {error}")),
        }
    }
}

/// Runs the worker's test to its end and says how it went. Blocks; `client` talks to the server thread.
pub fn run(args: &WorkerArgs, client: Client, owner_wait: Duration) -> TestRecord {
    let name = args.test.clone().unwrap_or_default();
    let Some(module) = &args.module else {
        return TestRecord::fail(&name, 0, "no test module given");
    };
    let setup = || -> Result<TestMod, String> {
        let bytes = read_module(module)?;
        TestMod::instantiate(&mut Sandbox::new(), "yoake-test", &bytes, Box::new(ChannelHost::new(client.clone())), args.timeout)
            .map_err(|error| format!("the test module did not load: {error}"))
    };
    let mut module = match setup() {
        Ok(module) => module,
        Err(error) => return TestRecord::fail(&name, 0, error),
    };
    if let Err(error) = wait_for_owner(&client, owner_wait) {
        return TestRecord::fail(&name, 0, error);
    }
    let began = Instant::now();
    let outcome = module.run_test(&name);
    let ms = began.elapsed().as_millis() as u64;
    match outcome {
        TestOutcome::Pass => TestRecord::pass(&name, ms),
        TestOutcome::Fail(message) => TestRecord::fail(&name, ms, message),
        TestOutcome::Trap(message) => TestRecord::fail(&name, ms, format!("the test module trapped or ran out of time: {message}")),
    }
}

/// Writes the result file whole (a temporary file renamed over the path), so the coordinator never reads half.
pub fn write_result(path: &Path, record: &TestRecord) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, record.to_json().to_string()).map_err(|error| format!("{}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|error| format!("{}: {error}", path.display()))
}

static FINISHED: AtomicBool = AtomicBool::new(false);

/// Reports `record` once (the first caller wins) and ends the process: 0 for a pass, 1 for a failure.
fn finish(args: &WorkerArgs, record: &TestRecord, code: i32) {
    if FINISHED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(path) = &args.result
        && let Err(error) = write_result(path, record)
    {
        eprintln!("yoake-test: could not write the result: {error}");
    }
    std::process::exit(code);
}

/// Starts the test thread and its watchdog. The game calls this once, when its integrated server is ready.
/// The process exits when the test is over (the world is a temporary one that the coordinator deletes).
pub fn start(args: WorkerArgs, client: Client) {
    let name = args.test.clone().unwrap_or_default();
    let watch_args = args.clone();
    let watch_name = name.clone();
    let watchdog = std::thread::Builder::new().name("yoake-test-watchdog".into()).spawn(move || {
        std::thread::sleep(OWNER_WAIT + watch_args.timeout + GRACE);
        let record = TestRecord::fail(&watch_name, 0, "the test did not finish in time");
        finish(&watch_args, &record, 3);
    });
    if let Err(error) = watchdog {
        eprintln!("yoake-test: no watchdog: {error}");
    }
    let spawned = std::thread::Builder::new().name("yoake-test".into()).spawn(move || {
        let record = run(&args, client, OWNER_WAIT);
        finish(&args, &record, if record.ok { 0 } else { 1 });
    });
    if let Err(error) = spawned {
        eprintln!("yoake-test: could not start the test thread: {error}");
        std::process::exit(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{Answer, Bridge, PlayerRow, channel};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    /// A world that answers every driver call with canned values.
    fn serve(bridge: Arc<Bridge>, stop: Arc<AtomicBool>, owner_joins_after: u32) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut polls = 0;
            let mut block = String::from("minecraft:air");
            while !stop.load(Ordering::Relaxed) {
                bridge.pump(&mut |op| -> Answer {
                    Ok(match op {
                        Op::Players => {
                            polls += 1;
                            Reply::Players(if polls > owner_joins_after {
                                vec![PlayerRow { uuid: "u".into(), name: "Owner".into(), position: [0.0; 3], health: 20.0, game_mode: 1 }]
                            } else {
                                Vec::new()
                            })
                        }
                        Op::PlaceBlock { block: placed, .. } => {
                            block = placed;
                            Reply::Done
                        }
                        Op::BlockAt(_) => Reply::Text(block.clone()),
                        Op::RunCommand(_) => Reply::Bool(true),
                        Op::SpawnEntity { .. } | Op::SpawnFakePlayer { .. } => Reply::Id(10),
                        Op::EntitiesNear { center, .. } => Reply::Entities(vec![crate::bridge::EntityRow {
                            id: 10,
                            kind: "minecraft:pig".into(),
                            position: center,
                        }]),
                        Op::ModCall { payload, .. } => {
                            let mut answer = b"echo:".to_vec();
                            answer.extend(payload);
                            Reply::Bytes(answer)
                        }
                        Op::BreakBlock(_) => Reply::Done,
                        Op::Advance(_) => unreachable!("the bridge handles it"),
                    })
                });
                bridge.tick_done();
                if !bridge.ticking_fast() {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        })
    }

    fn args(test: &str, module: &Path, timeout: Duration) -> WorkerArgs {
        WorkerArgs { test: Some(test.into()), module: Some(module.into()), result: None, timeout }
    }

    fn demo_module() -> Option<std::path::PathBuf> {
        let bytes = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::TEST_DEMO)?;
        let path = std::env::temp_dir().join(format!("yoake-test-runner-demo-{}.wasm", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        Some(path)
    }

    #[test]
    fn a_worker_waits_for_the_owner_then_runs_its_test_in_the_world() {
        let Some(module) = demo_module() else { return };
        let (bridge, client) = channel(Duration::from_secs(20));
        let (bridge, stop) = (Arc::new(bridge), Arc::new(AtomicBool::new(false)));
        let world = serve(Arc::clone(&bridge), Arc::clone(&stop), 3);
        let record = run(&args("passes", &module, Duration::from_secs(20)), client.clone(), Duration::from_secs(20));
        assert!(record.ok, "{record:?}");
        assert!(client.ticks() >= 3, "the test advanced the world by 3 ticks, and the owner poll ran the loop");
        let record = run(&args("fails", &module, Duration::from_secs(20)), client.clone(), Duration::from_secs(20));
        assert!(!record.ok && record.error.as_deref().unwrap_or_default().contains("expected"), "{record:?}");
        stop.store(true, Ordering::Relaxed);
        world.join().unwrap();
        let _ = std::fs::remove_file(module);
    }

    #[test]
    fn a_test_that_runs_out_of_time_or_traps_is_a_failure_with_a_reason() {
        let Some(module) = demo_module() else { return };
        let (bridge, client) = channel(Duration::from_secs(20));
        let (bridge, stop) = (Arc::new(bridge), Arc::new(AtomicBool::new(false)));
        let world = serve(Arc::clone(&bridge), Arc::clone(&stop), 0);
        let began = Instant::now();
        let record = run(&args("spins", &module, Duration::from_millis(300)), client.clone(), Duration::from_secs(20));
        assert!(!record.ok && record.error.as_deref().unwrap().contains("trapped or ran out of time"), "{record:?}");
        assert!(began.elapsed() < Duration::from_secs(15));
        let record = run(&args("traps", &module, Duration::from_secs(20)), client, Duration::from_secs(20));
        assert!(!record.ok && record.error.as_deref().unwrap().contains("trapped"), "{record:?}");
        stop.store(true, Ordering::Relaxed);
        world.join().unwrap();
        let _ = std::fs::remove_file(module);
    }

    #[test]
    fn no_owner_or_no_world_or_no_module_fails_instead_of_hanging() {
        let Some(module) = demo_module() else { return };
        // The owner never joins.
        let (bridge, client) = channel(Duration::from_secs(5));
        let (bridge, stop) = (Arc::new(bridge), Arc::new(AtomicBool::new(false)));
        let world = serve(Arc::clone(&bridge), Arc::clone(&stop), u32::MAX);
        let record = run(&args("passes", &module, Duration::from_secs(5)), client, Duration::from_millis(300));
        assert!(!record.ok && record.error.as_deref().unwrap().contains("no player joined"), "{record:?}");
        stop.store(true, Ordering::Relaxed);
        world.join().unwrap();
        // No server thread at all.
        let (bridge, client) = channel(Duration::from_millis(50));
        drop(bridge);
        let record = run(&args("passes", &module, Duration::from_secs(5)), client.clone(), Duration::from_millis(300));
        assert!(!record.ok && record.error.as_deref().unwrap().contains("never answered"), "{record:?}");
        // A module that is not there.
        let record = run(&args("passes", Path::new("no/such/module.wasm"), Duration::from_secs(5)), client.clone(), Duration::from_millis(300));
        assert!(!record.ok && record.error.as_deref().unwrap().contains("no/such/module.wasm"), "{record:?}");
        let _ = std::fs::remove_file(module);
    }

    #[test]
    fn the_result_file_is_written_whole() {
        let path = std::env::temp_dir().join(format!("yoake-test-runner-result-{}.json", std::process::id()));
        write_result(&path, &TestRecord::fail("t", 5, "boom")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(TestRecord::from_json(&value).unwrap(), TestRecord::fail("t", 5, "boom"));
        assert!(!path.with_extension("tmp").exists());
        let _ = std::fs::remove_file(path);
    }
}
