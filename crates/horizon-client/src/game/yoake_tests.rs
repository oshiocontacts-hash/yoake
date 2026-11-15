//! The server loop's side of `--yoake-test` (`crate::yoake_test`): in a worker it serves the test driver's
//! operations between two ticks. In every other process the bridge does not exist and nothing here does anything.
use super::Game;

impl Game<'_> {
    /// Test mode: executes the driver operations that arrived, with the game as the host (the server services
    /// of a `server-full` mod, `wasm_mods/test_driver.rs`). While the test froze the world this waits for the
    /// test's next call (up to 50 ms) instead of ticking, so a call costs what it costs and not a tick.
    pub(super) fn pump_yoake_test(&mut self) {
        crate::yoake_test::serve(&mut |op| {
            self.native_lifecycle_test_read_scope(true);
            let answer = crate::entry::run_test_op(self, op);
            self.native_lifecycle_test_read_scope(false);
            answer
        });
    }

    /// Test mode: one server tick ran (an `advance-ticks` counts them).
    pub(super) fn yoake_test_tick_done(&self) {
        if let Some(bridge) = crate::yoake_test::bridge() {
            bridge.tick_done();
        }
    }
}
