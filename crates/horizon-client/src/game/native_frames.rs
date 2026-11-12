use std::time::{Duration, Instant};

use super::Game;
use horizon_client::clock::FixedStep;

/// Client ticks to run before drawing, followed by interpolation in `[0, 1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameAdvance {
    pub ticks: u32,
    pub partial_tick: f32,
}

/// A zero-based offline frame and its default, rational 20 Hz cadence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedFrame {
    pub index: u64,
    pub fps: u32,
    pub size: (u32, u32),
    pub hud: bool,
    pub elapsed: Duration,
    pub advance: FrameAdvance,
}

struct Run {
    generation: u64,
    fps: u32,
    size: (u32, u32),
    hud: bool,
    index: u64,
    ticks: u64,
    elapsed: Duration,
    origin: Duration,
    saved_clock: FixedStep,
    saved_pause: bool,
}

impl Run {
    fn frame(&self) -> Result<FixedFrame, &'static str> {
        let frame_ticks = u128::from(self.index) * 20;
        let previous = u128::from(self.index.saturating_sub(1)) * 20;
        Ok(FixedFrame {
            index: self.index,
            fps: self.fps,
            size: self.size,
            hud: self.hud,
            elapsed: duration_at(self.index, self.fps)?,
            advance: FrameAdvance {
                ticks: (frame_ticks / u128::from(self.fps) - previous / u128::from(self.fps))
                    as u32,
                partial_tick: (frame_ticks % u128::from(self.fps)) as f32 / self.fps as f32,
            },
        })
    }

    fn advanced(&self, advance: FrameAdvance) -> Result<(u64, Duration), &'static str> {
        if !advance.partial_tick.is_finite() || !(0.0..1.0).contains(&advance.partial_tick) {
            return Err("fixed-frame partial tick must be finite and in [0, 1)");
        }
        let ticks = self
            .ticks
            .checked_add(u64::from(advance.ticks))
            .ok_or("tick overflow")?;
        let elapsed = duration_at(ticks, 20)?
            .checked_add(Duration::from_secs_f64(
                f64::from(advance.partial_tick) / 20.0,
            ))
            .ok_or("frame time overflow")?;
        if elapsed < self.elapsed {
            return Err("fixed frames cannot run backwards");
        }
        self.origin
            .checked_add(elapsed)
            .ok_or("presentation time overflow")?;
        Ok((ticks, elapsed))
    }
}

fn duration_at(index: u64, fps: u32) -> Result<Duration, &'static str> {
    let nanos = u128::from(index) * 1_000_000_000 / u128::from(fps);
    if nanos > i64::MAX as u128 {
        return Err("fixed-frame time exceeds the client clock");
    }
    Ok(Duration::from_nanos(nanos as u64))
}

#[derive(Default)]
pub(super) struct NativeFrames {
    run: Option<Run>,
    resumed: Option<(Duration, Instant)>,
    generation: u64,
}

impl NativeFrames {
    pub(super) fn active(&self) -> bool {
        self.run.is_some()
    }

    pub(super) fn hud_visible(&self) -> bool {
        self.run.as_ref().is_none_or(|run| run.hud)
    }

    pub(super) fn clear_resumed(&mut self) {
        self.resumed = None;
    }

    fn current(&self, generation: u64) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.generation == generation)
    }

    fn presentation_time(&self) -> Option<Duration> {
        self.run
            .as_ref()
            .map(|run| run.origin + run.elapsed)
            .or_else(|| {
                self.resumed
                    .map(|(elapsed, instant)| elapsed + instant.elapsed())
            })
    }
}

impl Game<'_> {
    pub(super) fn native_presentation_time(&self) -> Duration {
        if let Some(elapsed) = self.native_frames.presentation_time() {
            return elapsed;
        }
        #[cfg(feature = "dev-capture")]
        {
            self.capture_presentation_time()
        }
        #[cfg(not(feature = "dev-capture"))]
        {
            self.started.elapsed()
        }
    }

    pub(super) fn begin_native_fixed_frames(
        &mut self,
        fps: u32,
        size: (u32, u32),
        hud: bool,
    ) -> bool {
        if horizon_client::mod_api::mods().is_empty()
            || !self.options.execution_role.is_client()
            || self.native_frames.active()
            || !self.native_session.is_replay()
            || self.player.is_none()
            || self.title.is_some()
            || fps == 0
            || fps > 1000
        {
            return false;
        }
        #[cfg(feature = "dev-capture")]
        if self.frame_capture.active() {
            return false;
        }
        let Some(generation) = self.native_frames.generation.checked_add(1) else {
            return false;
        };
        let origin = self.native_presentation_time();
        let Some(renderer) = &mut self.renderer else {
            return false;
        };
        if let Err(error) = renderer.begin_native_fixed_output([size.0, size.1]) {
            log::warn!("Horizon: fixed-frame request rejected: {error}");
            return false;
        }
        if let Err(error) = self.audio.begin_fixed_audio_mix() {
            renderer.end_native_fixed_output();
            log::warn!("Horizon: fixed-frame audio rejected: {error}");
            return false;
        }
        renderer.set_capture_presentation_time(Some(origin));
        self.native_frames.run = Some(Run {
            generation,
            fps,
            size,
            hud,
            index: 0,
            ticks: 0,
            elapsed: Duration::ZERO,
            origin,
            saved_clock: self.clock.clone(),
            saved_pause: self.pause,
        });
        self.native_frames.generation = generation;
        true
    }

    pub(super) fn finish_native_fixed_frames(&mut self) {
        let Some(run) = self.native_frames.run.take() else {
            return;
        };
        self.audio.end_fixed_audio_mix();
        self.clock = run.saved_clock;
        self.clock.reset();
        self.pause = run.saved_pause;
        let now = Instant::now();
        self.previous = now;
        self.frame_end = now;
        self.native_frames.resumed = Some((run.origin + run.elapsed, now));
        if let Some(renderer) = &mut self.renderer {
            renderer.end_native_fixed_output();
        }
    }

    pub(super) fn native_fixed_redraw(&mut self) -> horizon_client::Result<()> {
        let result = self.native_fixed_redraw_inner();
        if result.is_err() {
            self.finish_native_fixed_frames();
        }
        result
    }

    fn native_fixed_redraw_inner(&mut self) -> horizon_client::Result<()> {
        if !self.native_session.is_replay() || self.player.is_none() || self.title.is_some() {
            self.finish_native_fixed_frames();
            return Ok(());
        }
        let run = self.native_frames.run.as_ref().unwrap();
        let generation = run.generation;
        let frame = run.frame()?;
        let mut advance = frame.advance;
        self.native_fixed_frame(&frame, &mut advance);
        if !self.native_frames.current(generation) {
            return Ok(());
        }
        let Some(run) = &self.native_frames.run else {
            return Ok(());
        };
        let (ticks, elapsed) = run.advanced(advance)?;
        let previous_ticks = run.ticks;
        let delta = elapsed - run.elapsed;
        let saved_clock = run.saved_clock.clone();
        self.pause = false;
        for tick in previous_ticks..ticks {
            let tick_time = duration_at(tick + 1, 20)?;
            self.audio.fixed_audio_until(tick_time, frame.index);
            self.clock
                .native_frame_time(&saved_clock, tick_time, 0.0, 1.0)?;
            let run = self.native_frames.run.as_mut().unwrap();
            run.elapsed = tick_time;
            if let Some(renderer) = &mut self.renderer {
                renderer.set_capture_presentation_time(Some(run.origin + tick_time));
            }
            self.update_world();
            if !self.native_frames.current(generation) {
                return Ok(());
            }
            self.spawn_player()?;
            self.tick()?;
            if !self.native_frames.current(generation) {
                return Ok(());
            }
            self.tick_ambience();
            self.tick_fov();
            self.tick_chat_queue();
            self.tick_music();
            self.tick_sound_engine();
        }
        self.audio.fixed_audio_until(elapsed, frame.index);
        self.clock.native_frame_time(
            &saved_clock,
            elapsed,
            advance.partial_tick,
            delta.as_secs_f32() * 20.0,
        )?;
        let run = self.native_frames.run.as_mut().unwrap();
        run.ticks = ticks;
        run.elapsed = elapsed;
        let presentation = run.origin + elapsed;
        if let Some(renderer) = &mut self.renderer {
            renderer.set_capture_presentation_time(Some(presentation));
        }
        self.frame_ticks = delta.as_secs_f32() * 20.0;
        self.extras_frame(delta);
        self.extras_retry()?;
        if !self.native_frames.current(generation) {
            return Ok(());
        }
        let renderer = self.renderer.as_mut().ok_or_else(|| "renderer removed")?;
        renderer.request_native_fixed_pixels(frame.index)?;
        let _native_crosshair_frame = super::native_crosshair::enter_frame();
        if !self.render()? {
            return Err("fixed-frame render did not produce a frame".into());
        }
        if !self.native_frames.current(generation) {
            return Ok(());
        }
        let pixels = self
            .renderer
            .as_ref()
            .unwrap()
            .read_native_fixed_pixels(frame.index)?;
        self.native_fixed_frame_pixels(frame.index, &pixels.rgba, (pixels.size[0], pixels.size[1]));
        self.notify_native_audio_mix();
        if self.native_frames.current(generation) {
            let run = self.native_frames.run.as_mut().unwrap();
            run.index = run
                .index
                .checked_add(1)
                .ok_or_else(|| "frame index overflow")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(fps: u32) -> Run {
        Run {
            generation: 1,
            fps,
            size: (3840, 2160),
            hud: false,
            index: 0,
            ticks: 0,
            elapsed: Duration::ZERO,
            origin: Duration::ZERO,
            saved_clock: FixedStep::default(),
            saved_pause: false,
        }
    }

    #[test]
    fn no_mod_has_no_run_or_presentation_override() {
        let state = NativeFrames::default();
        assert!(!state.active());
        assert!(state.hud_visible());
        assert!(state.presentation_time().is_none());
    }

    #[test]
    fn ending_and_restarting_cannot_finish_an_old_frame() {
        let mut state = NativeFrames::default();
        state.run = Some(run(60));
        assert!(state.current(1));
        assert!(!state.hud_visible());
        state.run = None;
        assert!(!state.current(1));
        let mut next = run(30);
        next.generation = 2;
        state.run = Some(next);
        assert!(!state.current(1));
        assert!(state.current(2));
    }

    #[test]
    fn fixed_cadence_has_no_accumulated_tick_drift() {
        for fps in [24, 30, 60, 144] {
            let mut run = run(fps);
            for index in 0..=u64::from(fps) * 60 {
                run.index = index;
                let frame = run.frame().unwrap();
                let (ticks, elapsed) = run.advanced(frame.advance).unwrap();
                run.ticks = ticks;
                run.elapsed = elapsed;
            }
            assert_eq!(run.ticks, 1200);
            assert_eq!(run.elapsed, Duration::from_secs(60));
        }
    }

    #[test]
    fn mod_advance_controls_time_not_wall_time() {
        let run = run(60);
        let (ticks, elapsed) = run
            .advanced(FrameAdvance {
                ticks: 3,
                partial_tick: 0.5,
            })
            .unwrap();
        assert_eq!(ticks, 3);
        assert_eq!(elapsed, Duration::from_millis(175));
        let mut clock = run.saved_clock.clone();
        clock
            .native_frame_time(&run.saved_clock, elapsed, 0.5, 3.5)
            .unwrap();
        assert_eq!(clock.fraction(), 0.5);
        for partial_tick in [f32::NAN, f32::INFINITY, -0.1, 1.0] {
            assert!(
                run.advanced(FrameAdvance {
                    ticks: 0,
                    partial_tick,
                })
                .is_err()
            );
        }
        let mut backwards = run;
        backwards.elapsed = Duration::from_millis(25);
        assert!(backwards.advanced(FrameAdvance::default()).is_err());
    }
}
