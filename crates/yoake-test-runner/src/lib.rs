//! The engine side of `yoake test` (design: `docs/yoake-wasm-dev.md`, "Testing a mod in the engine").
//!
//! `horizon-client --yoake-test <project|.yak>` is the *coordinator*: it loads the project's test module
//! once to list its tests, then runs every test in its own *worker*, a fresh offscreen game process
//! (`--offscreen`, never a visible window) with a fresh temporary superflat world in a temporary game
//! folder, and reports `{ok, tests: [{name, ok, ms, error?}]}`. The worker is the same executable with
//! `--yoake-test-run <name>`: its integrated server thread serves the test module's driver calls through a
//! [`Bridge`] (between two server ticks), and it exits with the test's result.
//!
//! This crate holds everything that does not need the game: parsing the command line ([`plan`]), the
//! package installer for the worker's game folder ([`install`]), the coordinator and its process handling
//! ([`coordinator`]), the report ([`report`]), the worker's test thread ([`worker`]) and the bridge
//! between the test thread and the server thread ([`bridge`]). The game supplies the world
//! preparation, the pump in its server loop and the executor of the driver's operations.
//!
//! Test mode is reachable only from the command line of the process. Nothing here runs without one of
//! the `--yoake-test*` flags, and no mod, server or save can set them.

pub mod bridge;
pub mod coordinator;
pub mod install;
pub mod plan;
pub mod report;
pub mod worker;

pub use bridge::{Bridge, Client, Op, Reply};
pub use plan::{Plan, WorkerArgs};
pub use report::TestRecord;
