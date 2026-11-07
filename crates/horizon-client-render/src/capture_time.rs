use std::{
    cell::Cell,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct Clock {
    wall: Instant,
    visual: Instant,
    capture: Option<Duration>,
}

impl Clock {
    fn now(self, wall: Instant) -> Instant {
        if self.capture.is_some() {
            self.visual
        } else {
            self.visual + wall.saturating_duration_since(self.wall)
        }
    }

    fn set(&mut self, time: Option<Duration>, wall: Instant) {
        self.visual = match (self.capture, time) {
            (Some(previous), Some(time)) => self.visual + time.saturating_sub(previous),
            _ => self.now(wall),
        };
        self.wall = wall;
        self.capture = time;
    }
}

thread_local! {
    static CLOCK: Cell<Option<Clock>> = const { Cell::new(None) };
}

pub(crate) fn set(time: Option<Duration>) {
    CLOCK.with(|slot| {
        if time.is_none() && slot.get().is_none() {
            return;
        }
        let wall = Instant::now();
        let mut clock = slot.get().unwrap_or(Clock {
            wall,
            visual: wall,
            capture: None,
        });
        clock.set(time, wall);
        slot.set(Some(clock));
    });
}

fn now(wall: Instant) -> Instant {
    CLOCK.with(|slot| slot.get().map_or(wall, |clock| clock.now(wall)))
}

#[derive(Clone, Copy, Debug)]
pub struct AnimationInstant {
    wall: Instant,
    visual: Instant,
}

impl AnimationInstant {
    pub fn now() -> Self {
        let wall = Instant::now();
        Self {
            wall,
            visual: now(wall),
        }
    }

    pub fn elapsed(self) -> Duration {
        let wall = Instant::now();
        now(wall).saturating_duration_since(self.visual)
    }

    pub(crate) fn wall(self) -> Instant {
        self.wall
    }
}

impl From<Instant> for AnimationInstant {
    fn from(wall: Instant) -> Self {
        let current = Instant::now();
        let visual = now(current);
        let visual = if wall <= current {
            visual.checked_sub(current - wall).unwrap_or(visual)
        } else {
            visual + (wall - current)
        };
        Self { wall, visual }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_clock_ignores_worker_waits_and_resumes_without_a_jump() {
        let start = Instant::now();
        let mut clock = Clock {
            wall: start,
            visual: start,
            capture: None,
        };
        clock.set(Some(Duration::from_secs(4)), start);
        assert_eq!(clock.now(start + Duration::from_secs(10)), start);
        clock.set(
            Some(Duration::from_millis(4050)),
            start + Duration::from_secs(10),
        );
        assert_eq!(clock.visual - start, Duration::from_millis(50));
        clock.set(None, start + Duration::from_secs(20));
        assert_eq!(
            clock.now(start + Duration::from_millis(20025)) - start,
            Duration::from_millis(75)
        );
    }
}
