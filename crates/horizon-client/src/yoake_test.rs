//! `--yoake-test`: the engine side of `yoake test` for Yoake mods (`docs/yoake-wasm-dev.md`, "Testing a mod
//! in the engine"). The logic is in `crates/yoake-test-runner`; this is what only the game can supply.
//!
//! * The coordinator ([`coordinate`]) is a process that never starts the game: `entry` hands it the command
//!   line before anything else runs. It starts one worker per test.
//! * A worker is this executable with `--yoake-test-run <test>` (parsed with the other options, `game.rs`).
//!   Its integrated server calls [`start_worker`] when it is ready and [`bridge`]'s `pump` once per loop
//!   iteration (`game/yoake_tests.rs`), and the driver's operations run with the game's own server
//!   services (`wasm_mods/test_driver.rs`).
//!
//! Without a `--yoake-test*` flag none of this runs: [`bridge`] is `None`, so the server loop does one
//! cheap check per tick and changes nothing.
use std::path::Path;
use std::sync::OnceLock;

use yoake_test_runner::coordinator::{self, GameCommand};
use yoake_test_runner::{Bridge, Plan, WorkerArgs};

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// The server loop's end of the test bridge: `Some` only in a worker, once its server is ready.
pub(crate) fn bridge() -> Option<&'static Bridge> {
    BRIDGE.get()
}

/// Whether the server loop should run its next tick at once (a test is waiting for ticks).
pub(crate) fn ticking_fast() -> bool {
    BRIDGE.get().is_some_and(Bridge::ticking_fast)
}

/// Starts the worker's test thread. Called once by the integrated server, after its world is ready; a
/// second call (a world opened again) does nothing.
pub(crate) fn start_worker(args: WorkerArgs) {
    let (bridge, client) = yoake_test_runner::bridge::channel(args.timeout);
    if BRIDGE.set(bridge).is_ok() {
        log::info!("yoake test: running {:?} (offscreen, temporary world)", args.test.as_deref().unwrap_or_default());
        yoake_test_runner::worker::start(args, client);
    }
}

/// A fresh superflat world with the fixed seed 0: the generator settings of a `Flat` world, so the game opens
/// the (otherwise empty) folder as a world of that kind and creates the rest as for any new world.
pub(crate) fn prepare_world(world: &Path) -> Result<(), String> {
    crate::save::write_world_gen_settings(
        world,
        0,
        crate::save::WorldGen { preset: crate::save::WorldPreset::Flat, structures: false, bonus_chest: false },
    )
    .map_err(|error| format!("{}: {error}", world.display()))
}

/// Runs the coordinator for `plan`: prints the report as one JSON line on stdout, writes it to
/// `--yoake-test-json` too, and returns the process exit code (0 passed, 1 a test failed, 2 not run).
pub(crate) fn coordinate(plan: &Plan) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("yoake test: cannot find the game executable: {error}");
            return 2;
        }
    };
    let outcome = coordinator::run(plan, &GameCommand::new(exe), &prepare_world);
    let text = outcome.report.to_string();
    if let Some(path) = &plan.json {
        let written = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, &text));
        if let Err(error) = written {
            eprintln!("yoake test: cannot write {}: {error}", path.display());
        }
    }
    println!("{text}");
    outcome.code
}

/// Called first thing by `entry`: turns the process into the coordinator when the command line asks
/// for it (never returns then); otherwise does nothing.
pub(crate) fn coordinator_requested() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match yoake_test_runner::plan::parse(&args) {
        Ok(None) => {}
        Ok(Some(plan)) => std::process::exit(coordinate(&plan)),
        Err(error) => {
            eprintln!("Horizon: {error}");
            std::process::exit(crate::game::exit_code::USAGE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("yoake-test-glue-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_prepared_world_is_a_superflat_one_with_no_structures() {
        let world = scratch("world");
        prepare_world(&world).unwrap();
        // What the game reads when it opens the world: a flat overworld generator.
        let settings = crate::save::world_gen_settings(&world).expect("the world has generator settings");
        let preset = crate::save::WorldGen::read(&world).expect("the options read back");
        assert_eq!(preset.preset, crate::save::WorldPreset::Flat);
        assert!(!preset.structures && !preset.bonus_chest);
        let generator = settings.get("dimensions").and_then(|dimensions| dimensions.get("minecraft:overworld")).and_then(|overworld| overworld.get("generator"));
        assert!(generator.is_some(), "an overworld generator is written");
        let _ = std::fs::remove_dir_all(world);
    }

    #[test]
    fn nothing_is_active_in_a_normal_run() {
        assert!(bridge().is_none());
        assert!(!ticking_fast());
        // Without a --yoake-test flag the coordinator hook does nothing (the test harness's own arguments
        // are not ours).
        coordinator_requested();
    }
}
