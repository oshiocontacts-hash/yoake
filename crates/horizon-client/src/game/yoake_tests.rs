//! The server loop's side of `--yoake-test` (`crate::yoake_test`): in a worker it serves the test driver's
//! operations between two ticks. In every other process the bridge does not exist and nothing here does anything.
use super::Game;

impl Game<'_> {
    /// Test mode: executes the driver operations that arrived since the last tick, with the game as the host
    /// (the server services of a `server-full` mod, `wasm_mods/test_driver.rs`).
    pub(super) fn pump_yoake_test(&mut self) {
        let Some(bridge) = crate::yoake_test::bridge() else {
            return;
        };
        bridge.pump(&mut |op| crate::entry::run_test_op(self, op));
    }

    /// Test mode: one server tick ran (an `advance-ticks` counts them).
    pub(super) fn yoake_test_tick_done(&self) {
        if let Some(bridge) = crate::yoake_test::bridge() {
            bridge.tick_done();
        }
    }
}
