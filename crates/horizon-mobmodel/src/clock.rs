//! `AnimClock`: piecewise-linear animation time for server/client agreement (FR-F4-MOBS.md section 3.5).
//!
//! The server sends an **anchor** `(state id, anchor_tick, anchor_time, speed)` and sends a new one whenever the state
//! or the speed changes. The client computes
//!
//! ```text
//! time = anchor_time + ((now_tick - anchor_tick) + partial) * speed / 20
//! ```
//!
//! Units: animation time is **seconds** (f64 on the wire and in this type; the controller's own clocks are f32 and
//! convert exactly at anchors), ticks are game ticks of 1/20 s (`TICKS_PER_SECOND`), `partial` is the fraction of the
//! current tick in `[0, 1)`. Speed is a multiplier of real time (1 = real time, 0 = frozen, negative = backwards;
//! controllers clamp their speeds to `[0, 100]`, but the clock itself accepts any finite value).
//!
//! The tick difference is taken in `i64` before it becomes a float, so even tick counts around 10^12 lose nothing.
//! The result never goes below 0 (animation time has no negative side); a non-finite speed or partial counts as 0.
//! Between anchors the clock is exactly linear, which is what the server's integrated clock is when the speed is
//! constant; at an anchor both sides hold the same number.

/// Game ticks per second.
pub const TICKS_PER_SECOND: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimClock {
    /// The controller state this clock belongs to (the one-byte synced id).
    pub state: u8,
    /// Server game tick of the anchor.
    pub anchor_tick: i64,
    /// Animation time in seconds at the anchor.
    pub anchor_time: f64,
    pub speed: f32,
}

impl AnimClock {
    pub fn new(state: u8, anchor_tick: i64, anchor_time: f64, speed: f32) -> Self {
        let anchor_time = if anchor_time.is_finite() { anchor_time.max(0.0) } else { 0.0 };
        let speed = if speed.is_finite() { speed } else { 0.0 };
        Self { state, anchor_tick, anchor_time, speed }
    }

    /// Animation time in seconds at `tick` plus `partial` of a tick.
    pub fn time_at(&self, tick: i64, partial: f32) -> f64 {
        let partial = if partial.is_finite() { partial as f64 } else { 0.0 };
        let ticks = tick.wrapping_sub(self.anchor_tick) as f64 + partial;
        (self.anchor_time + ticks * self.speed as f64 / TICKS_PER_SECOND).max(0.0)
    }

    /// A new anchor at `tick` with the same time (continuous) but a new speed and/or state. Use it whenever either
    /// changes, and send the result.
    pub fn reanchor(&self, tick: i64, state: u8, speed: f32) -> AnimClock {
        AnimClock::new(state, tick, self.time_at(tick, 0.0), speed)
    }

    /// Whether `state`/`speed` differ from this anchor, so the server must send a new one.
    pub fn is_stale(&self, state: u8, speed: f32) -> bool {
        self.state != state || self.speed != speed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_between_anchors() {
        let c = AnimClock::new(2, 100, 1.5, 2.0);
        assert_eq!(c.time_at(100, 0.0), 1.5);
        assert_eq!(c.time_at(120, 0.0), 3.5); // one second of real time at speed 2
        assert_eq!(c.time_at(110, 0.5), 1.5 + 10.5 * 2.0 / 20.0);
        assert_eq!(c.time_at(100, 0.5), 1.5 + 0.05);
    }

    #[test]
    fn reanchor_is_continuous() {
        let c = AnimClock::new(1, 0, 0.0, 1.0);
        let d = c.reanchor(30, 1, 3.0);
        assert_eq!(d.anchor_time, 1.5);
        assert_eq!(d.time_at(30, 0.0), c.time_at(30, 0.0));
        // after the change the new speed rules
        assert_eq!(d.time_at(50, 0.0), 1.5 + 3.0);
        // a state change keeps time too (the state's own clock restarts at the controller, so callers pass 0 there)
        let e = d.reanchor(50, 7, 0.0);
        assert_eq!((e.state, e.anchor_time), (7, 4.5));
        assert_eq!(e.time_at(1_000, 0.9), 4.5); // frozen
        assert!(d.is_stale(1, 1.0) && d.is_stale(2, 3.0) && !d.is_stale(1, 3.0));
    }

    #[test]
    fn zero_and_negative_speed() {
        let z = AnimClock::new(0, 10, 2.0, 0.0);
        assert_eq!(z.time_at(10_000, 0.7), 2.0);
        let n = AnimClock::new(0, 0, 2.0, -1.0);
        assert_eq!(n.time_at(20, 0.0), 1.0);
        assert_eq!(n.time_at(40, 0.0), 0.0);
        assert_eq!(n.time_at(400, 0.0), 0.0); // never negative
        // re-anchoring after the floor starts from 0, not from a negative number
        assert_eq!(n.reanchor(400, 0, 1.0).anchor_time, 0.0);
    }

    #[test]
    fn bad_numbers_are_neutral() {
        let c = AnimClock::new(0, 0, f64::NAN, f32::NAN);
        assert_eq!((c.anchor_time, c.speed), (0.0, 0.0));
        let c = AnimClock::new(0, 0, 1.0, 1.0);
        assert_eq!(c.time_at(20, f32::NAN), 2.0);
        assert_eq!(AnimClock::new(0, 0, -5.0, 1.0).anchor_time, 0.0);
    }

    #[test]
    fn large_ticks_keep_precision() {
        let base: i64 = 1_000_000_000_000; // 1e12 ticks: far past where f32 (or a naive tick*0.05) would blur
        let c = AnimClock::new(0, base, 12.5, 1.0);
        assert_eq!(c.time_at(base + 20, 0.0), 13.5);
        assert!((c.time_at(base + 1, 0.0) - 12.55).abs() < 1e-12);
        assert_eq!(c.time_at(base - 20, 0.0), 11.5);
        // Same answer as an anchor taken at tick 0: only the difference matters.
        let d = AnimClock::new(0, 0, 12.5, 1.0);
        assert_eq!(c.time_at(base + 7, 0.25), d.time_at(7, 0.25));
        // i64 extremes do not panic
        let _ = c.time_at(i64::MAX, 0.5);
        let _ = c.time_at(i64::MIN, 0.5);
    }

    #[test]
    fn constant_speed_matches_summed_steps() {
        // The server integrates dt = 0.05 per tick; the client uses one anchor.
        let c = AnimClock::new(0, 0, 0.0, 1.75);
        let mut integrated = 0.0f64;
        for tick in 1..=2000i64 {
            integrated += 0.05 * 1.75;
            assert!((c.time_at(tick, 0.0) - integrated).abs() < 1e-9, "tick {tick}");
        }
    }
}
