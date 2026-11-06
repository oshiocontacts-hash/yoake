use std::{num::NonZeroU32, path::Path, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameTime {
    pub index: u64,
    pub fps: NonZeroU32,
}

impl FrameTime {
    pub fn seconds(self) -> f64 {
        self.index as f64 / f64::from(self.fps.get())
    }

    pub fn elapsed(self) -> Duration {
        let denominator = u64::from(self.fps.get());
        Duration::new(
            self.index / denominator,
            ((self.index % denominator) * 1_000_000_000 / denominator) as u32,
        )
    }

    pub fn delta(self) -> Duration {
        let denominator = u64::from(self.fps.get());
        Duration::from_nanos((1_000_000_000 + denominator / 2) / denominator)
    }

    pub fn total_ticks(self) -> u64 {
        u64::try_from(u128::from(self.index) * 20 / u128::from(self.fps.get()))
            .expect("validated capture tick count")
    }

    pub fn residual(self) -> u32 {
        (u128::from(self.index) * 20 % u128::from(self.fps.get())) as u32
    }

    pub fn partial_tick(self) -> f32 {
        self.residual() as f32 / self.fps.get() as f32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureFrame {
    pub session: u64,
    pub generation: u64,
    pub time: FrameTime,
}

impl CaptureFrame {
    pub fn filename(self) -> String {
        format!("{:06}.png", self.time.index - 1)
    }
}

#[derive(Debug)]
pub struct CaptureWrite {
    pub frame: CaptureFrame,
    pub result: Result<(), String>,
}

// CHECK: T28 must install a generation-aware, single-readback PNG output before capture starts.
pub trait CaptureOutput {
    fn begin(&mut self, directory: &Path, frames: u64) -> Result<(), String>;
    fn request_frame(&mut self, frame: CaptureFrame) -> Result<(), String>;
    fn poll_complete(&mut self) -> Option<CaptureWrite>;
    fn finish(&mut self) -> Result<(), String>;
    fn abort(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSchedule {
    pub time: FrameTime,
    pub ticks_due: u32,
}

#[derive(Clone, Debug)]
pub struct CaptureClock {
    fps: NonZeroU32,
    frames: u64,
    completed: u64,
    presentation_origin: Duration,
}

impl CaptureClock {
    pub fn new(fps: u32, frames: u64, origin: Duration) -> Result<Self, &'static str> {
        let fps = NonZeroU32::new(fps).ok_or("capture fps must be positive")?;
        if frames == 0 {
            return Err("capture frames must be positive");
        }
        if u128::from(frames) * 20 / u128::from(fps.get()) > u128::from(u64::MAX) {
            return Err("capture tick count overflows");
        }
        if origin
            .checked_add(FrameTime { index: frames, fps }.elapsed())
            .is_none()
        {
            return Err("capture presentation time overflows");
        }
        Ok(Self {
            fps,
            frames,
            completed: 0,
            presentation_origin: origin,
        })
    }

    pub fn frame(&self) -> Option<FrameSchedule> {
        if self.completed == self.frames {
            return None;
        }
        let time = FrameTime {
            index: self.completed + 1,
            fps: self.fps,
        };
        Some(FrameSchedule {
            time,
            ticks_due: (time.total_ticks() - self.time().total_ticks()) as u32,
        })
    }

    pub fn acknowledge(&mut self, index: u64) -> Result<(), &'static str> {
        if self.frame().is_none_or(|frame| frame.time.index != index) {
            return Err("unexpected capture frame acknowledgement");
        }
        self.completed = index;
        Ok(())
    }

    pub fn time(&self) -> FrameTime {
        FrameTime {
            index: self.completed,
            fps: self.fps,
        }
    }

    pub fn end_time(&self) -> FrameTime {
        FrameTime {
            index: self.frames,
            fps: self.fps,
        }
    }

    pub fn presentation_time(&self) -> Duration {
        self.presentation_origin
            + self
                .frame()
                .map_or(self.time(), |frame| frame.time)
                .elapsed()
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_totals_and_residuals() {
        for fps in [20, 24, 30, 60, 29, 7] {
            let frames = u64::from(fps) * 17 + 3;
            let mut clock = CaptureClock::new(fps, frames, Duration::ZERO).unwrap();
            let mut ticks = 0;
            while let Some(frame) = clock.frame() {
                ticks += u64::from(frame.ticks_due);
                assert_eq!(ticks, frame.time.total_ticks());
                assert_eq!(
                    u128::from(ticks) * u128::from(fps) + u128::from(frame.time.residual()),
                    u128::from(frame.time.index) * 20
                );
                clock.acknowledge(frame.time.index).unwrap();
            }
            assert_eq!(ticks, frames * 20 / u64::from(fps));
        }
    }

    #[test]
    fn thirty_fps_cycle() {
        let mut clock = CaptureClock::new(30, 3, Duration::ZERO).unwrap();
        for (ticks, residual) in [(0, 20), (1, 10), (1, 0)] {
            let frame = clock.frame().unwrap();
            assert_eq!(frame.ticks_due, ticks);
            assert_eq!(frame.time.residual(), residual);
            clock.acknowledge(frame.time.index).unwrap();
        }
        assert_eq!(clock.time().elapsed(), Duration::from_millis(100));
    }

    #[test]
    fn no_duration_accumulation_or_duplicate_acknowledgement() {
        let mut clock = CaptureClock::new(29, 29, Duration::ZERO).unwrap();
        for index in 1..=29 {
            clock.acknowledge(index).unwrap();
        }
        assert_eq!(clock.time().elapsed(), Duration::from_secs(1));
        assert!(clock.acknowledge(29).is_err());
        assert!(clock.frame().is_none());
    }

    #[test]
    fn invalid_schedules() {
        assert!(CaptureClock::new(0, 1, Duration::ZERO).is_err());
        assert!(CaptureClock::new(30, 0, Duration::ZERO).is_err());
        assert!(CaptureClock::new(1, u64::MAX, Duration::ZERO).is_err());
    }
}
