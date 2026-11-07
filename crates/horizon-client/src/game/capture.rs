use super::{
    Game, Screen,
    integrated::{CaptureControl, CaptureOperation, CaptureReply, CaptureRequest, CaptureSession},
};
use crate::{
    capture_clock::{CaptureClock, CaptureFrame, CaptureOutput, FrameSchedule, FrameTime},
    clock::FixedStep,
};
use horizon_protocol::connection::CaptureFenceId;
use serde_json::{Value, json};
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, Sender, TryRecvError},
    },
    time::{Duration, Instant},
};

static GENERATION: AtomicU64 = AtomicU64::new(1);
const SERVER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
enum Outcome {
    Complete,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Hold,
    WaitingHold,
    Barrier {
        initial: bool,
        step: bool,
    },
    WaitingBarrier {
        marker: CaptureFenceId,
        initial: bool,
        step: bool,
        server_done: bool,
    },
    ClientTick,
    Present,
    WaitingWrite,
    Release,
    WaitingRelease,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Request(CaptureOperation),
    ClientTick(u64),
    Present(FrameSchedule),
    None,
}

struct Machine {
    id: CaptureSession,
    clock: CaptureClock,
    phase: Phase,
    sequence: u64,
    server_ticks: u64,
    client_ticks: u64,
    outcome: Outcome,
}

impl Machine {
    fn new(id: CaptureSession, clock: CaptureClock) -> Self {
        Self {
            id,
            clock,
            phase: Phase::Hold,
            sequence: 0,
            server_ticks: 0,
            client_ticks: 0,
            outcome: Outcome::Complete,
        }
    }

    fn next_frame(&mut self) {
        self.phase = match self.clock.frame() {
            Some(frame) => Phase::Barrier {
                initial: false,
                step: self.server_ticks < frame.time.total_ticks(),
            },
            None => Phase::Release,
        };
    }

    fn action(&mut self) -> Result<Action, String> {
        Ok(match self.phase {
            Phase::Hold => {
                self.phase = Phase::WaitingHold;
                Action::Request(CaptureOperation::Hold)
            }
            Phase::Barrier { initial, step } => {
                self.sequence = self
                    .sequence
                    .checked_add(1)
                    .ok_or("capture fence overflow")?;
                let marker = CaptureFenceId {
                    session: self.id.session,
                    generation: self.id.generation,
                    sequence: self.sequence,
                };
                self.phase = Phase::WaitingBarrier {
                    marker,
                    initial,
                    step,
                    server_done: false,
                };
                Action::Request(if step {
                    CaptureOperation::Step(marker)
                } else {
                    CaptureOperation::Fence(marker)
                })
            }
            Phase::ClientTick => {
                self.client_ticks += 1;
                let frame = self.clock.frame().ok_or("missing capture frame")?;
                self.phase = if self.client_ticks < frame.time.total_ticks() {
                    Phase::ClientTick
                } else {
                    Phase::Present
                };
                Action::ClientTick(self.client_ticks)
            }
            Phase::Present => {
                let frame = self.clock.frame().ok_or("missing capture frame")?;
                self.phase = Phase::WaitingWrite;
                Action::Present(frame)
            }
            Phase::Release => {
                self.phase = Phase::WaitingRelease;
                Action::Request(CaptureOperation::Release)
            }
            _ => Action::None,
        })
    }

    fn reply(&mut self, reply: CaptureReply) -> Result<(), String> {
        if reply.request.id != self.id {
            return Err("stale capture server reply".into());
        }
        match (self.phase, reply.request.operation) {
            (Phase::WaitingHold, CaptureOperation::Hold) => {
                reply.result?;
                self.phase = Phase::Barrier {
                    initial: true,
                    step: false,
                };
            }
            (
                Phase::WaitingBarrier {
                    marker,
                    initial,
                    step,
                    server_done: false,
                },
                operation,
            ) if operation
                == if step {
                    CaptureOperation::Step(marker)
                } else {
                    CaptureOperation::Fence(marker)
                } =>
            {
                reply.result?;
                if step {
                    self.server_ticks += 1;
                }
                self.phase = Phase::WaitingBarrier {
                    marker,
                    initial,
                    step,
                    server_done: true,
                };
            }
            (Phase::WaitingRelease, CaptureOperation::Release) => match reply.result {
                Ok(()) => self.phase = Phase::Done,
                Err(reason) if self.outcome == Outcome::Complete => {
                    self.outcome = Outcome::Failed(reason);
                }
                Err(_) => {}
            },
            _ => return Err("unexpected capture server reply".into()),
        }
        Ok(())
    }

    fn applied(&mut self, applied: CaptureFenceId) {
        if let Phase::WaitingBarrier {
            marker,
            initial,
            server_done: true,
            ..
        } = self.phase
            && marker == applied
        {
            if initial {
                self.next_frame();
            } else {
                let target = self
                    .clock
                    .frame()
                    .expect("active capture frame")
                    .time
                    .total_ticks();
                self.phase = if self.server_ticks < target {
                    Phase::Barrier {
                        initial: false,
                        step: true,
                    }
                } else if self.client_ticks < target {
                    Phase::ClientTick
                } else {
                    Phase::Present
                };
            }
        }
    }

    fn written(&mut self, frame: CaptureFrame) -> Result<(), String> {
        if self.phase != Phase::WaitingWrite
            || frame.session != self.id.session
            || frame.generation != self.id.generation
            || self
                .clock
                .frame()
                .is_none_or(|next| next.time != frame.time)
        {
            return Err("unexpected capture PNG acknowledgement".into());
        }
        self.clock.acknowledge(frame.time.index)?;
        self.next_frame();
        Ok(())
    }

    fn stop(&mut self, outcome: Outcome) {
        if self.phase == Phase::Done {
            return;
        }
        self.outcome = outcome;
        if !matches!(self.phase, Phase::Release | Phase::WaitingRelease) {
            self.phase = Phase::Release;
        }
    }
}

struct Pending {
    receiver: Receiver<CaptureReply>,
    started: Instant,
}

struct Run {
    machine: Machine,
    saved_clock: FixedStep,
    control: CaptureControl,
    pending: Option<Pending>,
    barrier_started: Option<Instant>,
    reply: Sender<String>,
    directory: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CaptureAudioPolicy {
    Normal,
    Muted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CapturePump {
    Inactive,
    Waiting,
    Render(CaptureFrame),
    Restored,
}

#[derive(Default)]
pub(super) struct CaptureState {
    run: Option<Run>,
    stop: Option<Outcome>,
    output: Option<Box<dyn CaptureOutput>>,
    active: bool,
    presentation: Option<Duration>,
    resumed: Option<(Duration, Instant)>,
    status: Option<Value>,
}

impl CaptureState {
    pub(super) fn active(&self) -> bool {
        self.active
    }

    pub(super) fn presentation_time(&self, real: Duration) -> Duration {
        self.presentation.unwrap_or_else(|| {
            self.resumed
                .map_or(real, |(origin, started)| origin + started.elapsed())
        })
    }

    pub(super) fn audio_policy(&self) -> CaptureAudioPolicy {
        if self.active {
            CaptureAudioPolicy::Muted
        } else {
            CaptureAudioPolicy::Normal
        }
    }

    fn status_for(run: &Run) -> Value {
        let machine = &run.machine;
        let time = machine.clock.time();
        json!({
            "state": match machine.phase {
                Phase::Hold | Phase::WaitingHold => "starting",
                Phase::Release | Phase::WaitingRelease => "restoring",
                Phase::Done => match &machine.outcome {
                    Outcome::Complete => "complete",
                    Outcome::Cancelled => "cancelled",
                    Outcome::Failed(_) => "failed",
                },
                Phase::WaitingWrite => "writing",
                _ => "capturing",
            },
            "session": machine.id.session,
            "generation": machine.id.generation,
            "fps": time.fps.get(),
            "frames": machine.clock.frames(),
            "directory": run.directory,
            "frames_written": time.index,
            "server_ticks": machine.server_ticks,
            "client_ticks": machine.client_ticks,
            "simulated_seconds": time.seconds(),
            "audio": if machine.phase == Phase::Done { "normal" } else { "muted" },
            "frame": machine.clock.frame().map(|frame| frame.time.index - 1),
            "error": match &machine.outcome {
                Outcome::Failed(reason) => Some(reason.as_str()),
                _ => None,
            },
        })
    }

    fn final_reply(run: &Run) -> Value {
        let mut reply = Self::status_for(run);
        reply["ok"] = json!(run.machine.outcome == Outcome::Complete);
        if run.machine.clock.time().index > 0 {
            reply["first"] = json!("000000.png");
            reply["last"] = json!(format!("{:06}.png", run.machine.clock.time().index - 1));
        }
        if run.machine.outcome == Outcome::Cancelled {
            reply["error"] = json!("capture cancelled");
        }
        reply
    }
}

impl Game<'_> {
    pub(super) fn capture_presentation_time(&self) -> Duration {
        self.frame_capture.presentation_time(self.started.elapsed())
    }

    pub(super) fn capture_audio_policy(&self) -> CaptureAudioPolicy {
        self.frame_capture.audio_policy()
    }

    pub(super) fn capture_set_output(
        &mut self,
        output: Box<dyn CaptureOutput>,
    ) -> Result<(), String> {
        if self.frame_capture.active {
            return Err("cannot replace an active capture output".into());
        }
        self.frame_capture.output = Some(output);
        Ok(())
    }

    pub(super) fn capture_install_output(&mut self) -> Result<(), String> {
        let renderer = self
            .renderer
            .as_ref()
            .ok_or("capture requires a renderer")?;
        let output = super::capture_output::PngCaptureOutput::new(renderer.capture_handle());
        self.capture_set_output(Box::new(output))
    }

    pub(super) fn capture_status(&self) -> Value {
        self.frame_capture.run.as_ref().map_or_else(
            || {
                self.frame_capture
                    .status
                    .clone()
                    .unwrap_or_else(|| json!({"state": "idle"}))
            },
            CaptureState::status_for,
        )
    }

    pub(super) fn capture_start(
        &mut self,
        directory: &Path,
        fps: u32,
        frames: u64,
        reply: Sender<String>,
    ) -> Result<(), String> {
        if self.frame_capture.active {
            return Err("capture already active".into());
        }
        if self.options.control.is_none() {
            return Err("capture requires explicit --control or --control-socket".into());
        }
        if self.integrated.is_none()
            || self.remote.is_none()
            || self
                .integrated
                .as_ref()
                .is_some_and(|server| server.published().is_some())
            || self.title.is_some()
            || self.player.is_none()
            || self.screen != Screen::Playing
            || self.pending_screenshot.is_some()
            || self.pending_save.is_some()
            || self.agent_busy()
            || self.repro.recording_active()
            || self.repro.replay_active()
        {
            return Err("capture requires a ready unpublished single-player world".into());
        }
        if self.clock.rate() != 20.0
            || self.clock.frozen()
            || self.clock.sprinting()
            || self.clock.stepping()
            || self.agent.hold_server
        {
            return Err(
                "capture requires an unheld, unfrozen 20 TPS clock without tick sprint".into(),
            );
        }
        let origin = self.frame_capture.presentation_time(self.started.elapsed());
        let clock = CaptureClock::new(fps, frames, origin)?;
        self.clock
            .clone()
            .capture_set_time(&self.clock, clock.end_time())?;
        let generation = GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| "capture generation exhausted")?;
        let output = self
            .frame_capture
            .output
            .as_mut()
            .ok_or("capture readback/PNG output is not installed")?;
        if let Err(reason) = output.begin(directory, frames) {
            output.abort();
            return Err(reason);
        }
        self.capture_flight_begin();
        self.frame_capture.presentation = Some(origin);
        self.frame_capture.active = true;
        self.frame_capture.run = Some(Run {
            machine: Machine::new(
                CaptureSession {
                    session: generation,
                    generation,
                },
                clock,
            ),
            saved_clock: self.clock.clone(),
            control: self.integrated.as_ref().unwrap().capture_control(),
            pending: None,
            barrier_started: None,
            reply,
            directory: directory.to_path_buf(),
        });
        self.audio
            .set_capture_muted(self.capture_audio_policy() == CaptureAudioPolicy::Muted);
        let presentation = self.capture_presentation_time();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_capture_presentation_time(Some(presentation));
        }
        Ok(())
    }

    pub(super) fn capture_cancel(&mut self) -> Value {
        if let Some(run) = &mut self.frame_capture.run {
            run.machine.stop(Outcome::Cancelled);
            run.control.cancel(run.machine.id);
        } else if self.frame_capture.active {
            self.frame_capture.stop = Some(Outcome::Cancelled);
        }
        if self.frame_capture.active
            && let Some(output) = &mut self.frame_capture.output
        {
            output.abort();
        }
        self.capture_status()
    }

    pub(super) fn capture_fail(&mut self, reason: String) {
        if let Some(run) = &mut self.frame_capture.run {
            run.machine.stop(Outcome::Failed(reason));
            run.control.cancel(run.machine.id);
        } else if self.frame_capture.active {
            self.frame_capture.stop = Some(Outcome::Failed(reason));
        }
        if self.frame_capture.active
            && let Some(output) = &mut self.frame_capture.output
        {
            output.abort();
        }
    }

    pub(super) fn capture_rendered(&mut self, rendered: bool) {
        if !rendered {
            self.capture_fail("capture frame was not rendered".into());
        }
    }

    pub(super) fn capture_redraw(&mut self) -> bool {
        let frame = match self.capture_pump() {
            CapturePump::Inactive => return false,
            CapturePump::Waiting | CapturePump::Restored => return true,
            CapturePump::Render(frame) => frame,
        };
        if let Err(error) = self.extras_retry() {
            self.capture_fail(error.to_string());
            return true;
        }
        let presentation = self.capture_presentation_time();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_capture_presentation_time(Some(presentation));
        }
        self.extras_frame(frame.time.delta());
        self.advance_flight();
        match self.render() {
            Ok(rendered) => self.capture_rendered(rendered),
            Err(error) => self.capture_fail(error.to_string()),
        }
        self.apply_cursor_icon();
        true
    }

    fn capture_finish_run(&mut self, mut run: Run) {
        if let Some(output) = &mut self.frame_capture.output {
            if run.machine.outcome == Outcome::Complete {
                if let Err(reason) = output.finish() {
                    output.abort();
                    run.machine.outcome = Outcome::Failed(reason);
                }
            } else {
                output.abort();
            }
        }
        self.clock.capture_restore(run.saved_clock.clone());
        let now = Instant::now();
        self.previous = now;
        self.frame_end = now;
        self.capture_flight_end();
        self.frame_capture.resumed = self.frame_capture.presentation.map(|time| (time, now));
        self.frame_capture.presentation = None;
        self.frame_capture.stop = None;
        self.frame_capture.active = false;
        self.audio
            .set_capture_muted(self.capture_audio_policy() == CaptureAudioPolicy::Muted);
        if let Some(renderer) = &mut self.renderer {
            renderer.set_capture_presentation_time(None);
        }
        self.frame_capture.status = Some(CaptureState::status_for(&run));
        let _ = run.reply.send(CaptureState::final_reply(&run).to_string());
    }

    pub(super) fn capture_shutdown(&mut self) {
        let Some(mut run) = self.frame_capture.run.take() else {
            return;
        };
        run.machine.stop(Outcome::Cancelled);
        run.control.cancel(run.machine.id);
        if let Some(server) = self.integrated.take()
            && let Err(error) = server.stop()
        {
            log::warn!("Horizon: failed to stop capture server on exit: {error}");
        }
        run.machine.phase = Phase::Done;
        self.capture_finish_run(run);
    }

    pub(super) fn capture_pump(&mut self) -> CapturePump {
        let Some(mut run) = self.frame_capture.run.take() else {
            return CapturePump::Inactive;
        };
        if !matches!(
            run.machine.phase,
            Phase::Release | Phase::WaitingRelease | Phase::Done
        ) {
            if self
                .renderer
                .as_ref()
                .is_none_or(|renderer| renderer.device_lost())
            {
                run.machine
                    .stop(Outcome::Failed("capture renderer lost its device".into()));
            }
            if let Err(error) = self.poll_remote() {
                run.machine.stop(Outcome::Failed(error.to_string()));
            }
            if let Some(reason) = self
                .integrated
                .as_ref()
                .and_then(|server| server.capture_failure(run.machine.id))
            {
                run.machine.stop(Outcome::Failed(reason));
            }
            if self.remote.is_none()
                || self.integrated.is_none()
                || self.title.is_some()
                || self.pending_save.is_some()
            {
                run.machine
                    .stop(Outcome::Failed("capture world disconnected".into()));
            }
        }
        if let Some(outcome) = self.frame_capture.stop.take() {
            run.machine.stop(outcome);
            run.control.cancel(run.machine.id);
        }
        if matches!(run.machine.phase, Phase::Release) {
            run.control.cancel(run.machine.id);
            run.pending = None;
            run.barrier_started = None;
            if run.machine.outcome != Outcome::Complete
                && let Some(output) = &mut self.frame_capture.output
            {
                output.abort();
            }
        } else if let Some(pending) = &run.pending {
            let reply = pending.receiver.try_recv();
            let timed_out = pending.started.elapsed() >= SERVER_TIMEOUT;
            match reply {
                Ok(reply) => {
                    run.pending = None;
                    if let Err(reason) = run.machine.reply(reply) {
                        run.machine.stop(Outcome::Failed(reason));
                    }
                }
                Err(TryRecvError::Empty) if !timed_out => {}
                Err(_) => {
                    run.pending = None;
                    let reason = "capture server stopped responding".to_string();
                    if run.machine.phase == Phase::WaitingRelease {
                        run.machine.outcome = Outcome::Failed(reason);
                    } else {
                        run.machine.stop(Outcome::Failed(reason));
                    }
                }
            }
        }
        if run.machine.phase == Phase::WaitingRelease && run.control.released(run.machine.id) {
            run.machine.phase = Phase::Done;
        }
        if let Phase::WaitingBarrier {
            marker,
            server_done: true,
            ..
        } = run.machine.phase
        {
            if self
                .remote
                .as_ref()
                .is_some_and(|remote| remote.session.capture_output_applied(marker))
            {
                run.machine.applied(marker);
                run.barrier_started = None;
            } else if run
                .barrier_started
                .is_some_and(|started| started.elapsed() >= SERVER_TIMEOUT)
            {
                run.machine.stop(Outcome::Failed(
                    "capture packet application fence stopped responding".into(),
                ));
            }
        }
        if run.machine.phase == Phase::WaitingWrite
            && let Some(write) = self
                .frame_capture
                .output
                .as_mut()
                .and_then(|output| output.poll_complete())
        {
            let result = write.result.and_then(|()| run.machine.written(write.frame));
            if let Err(reason) = result {
                run.machine.stop(Outcome::Failed(reason));
            }
        }
        let mut result = CapturePump::Waiting;
        match run.machine.action() {
            Ok(Action::Request(operation)) => {
                if operation == CaptureOperation::Release {
                    run.control.cancel(run.machine.id);
                }
                let request = CaptureRequest {
                    id: run.machine.id,
                    operation,
                };
                let sent = match operation {
                    CaptureOperation::Fence(marker) | CaptureOperation::Step(marker) => self
                        .remote
                        .as_ref()
                        .ok_or_else(|| "capture transport closed".to_string())
                        .and_then(|remote| remote.session.capture_fence(marker)),
                    _ => Ok(()),
                };
                match (sent, self.integrated.as_ref()) {
                    (Ok(()), Some(server)) => {
                        if matches!(
                            operation,
                            CaptureOperation::Fence(_) | CaptureOperation::Step(_)
                        ) {
                            run.barrier_started = Some(Instant::now());
                        }
                        run.pending = Some(Pending {
                            receiver: server.capture_request(request),
                            started: Instant::now(),
                        });
                    }
                    (sent, _) => {
                        let reason = sent
                            .err()
                            .unwrap_or_else(|| "integrated server closed".into());
                        if operation == CaptureOperation::Release {
                            run.machine.outcome = Outcome::Failed(reason);
                        } else {
                            run.machine.stop(Outcome::Failed(reason));
                        }
                    }
                }
            }
            Ok(Action::ClientTick(index)) => {
                let ticked = self
                    .clock
                    .capture_set_time(
                        &run.saved_clock,
                        FrameTime {
                            index,
                            fps: NonZeroU32::new(20).unwrap(),
                        },
                    )
                    .map_err(str::to_string)
                    .and_then(|()| self.tick().map_err(|error| error.to_string()));
                if let Err(reason) = ticked {
                    run.machine.stop(Outcome::Failed(reason));
                } else {
                    self.tick_ambience();
                    self.tick_fov();
                    self.tick_chat_queue();
                }
            }
            Ok(Action::Present(schedule)) => {
                if let Err(reason) = self.clock.capture_set_time(&run.saved_clock, schedule.time) {
                    run.machine.stop(Outcome::Failed(reason.into()));
                } else {
                    self.frame_ticks = 20.0 / schedule.time.fps.get() as f32;
                    self.frame_capture.presentation = Some(run.machine.clock.presentation_time());
                    let frame = CaptureFrame {
                        session: run.machine.id.session,
                        generation: run.machine.id.generation,
                        time: schedule.time,
                    };
                    let requested = self
                        .frame_capture
                        .output
                        .as_mut()
                        .ok_or_else(|| "capture output removed".to_string())
                        .and_then(|output| output.request_frame(frame));
                    if let Err(reason) = requested {
                        run.machine.stop(Outcome::Failed(reason));
                    } else {
                        result = CapturePump::Render(frame);
                    }
                }
            }
            Ok(Action::None) => {}
            Err(reason) => run.machine.stop(Outcome::Failed(reason)),
        }
        if let Some(outcome) = self.frame_capture.stop.take() {
            run.machine.stop(outcome);
            run.control.cancel(run.machine.id);
            result = CapturePump::Waiting;
        }
        if run.machine.phase == Phase::Done {
            self.capture_finish_run(run);
            CapturePump::Restored
        } else {
            self.frame_capture.run = Some(run);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_clock::CaptureWrite;

    #[derive(Default)]
    struct SlowOutput {
        requested: Vec<CaptureFrame>,
        pending: Option<(CaptureFrame, u32)>,
        started: bool,
        finished: bool,
        aborted: bool,
    }

    impl CaptureOutput for SlowOutput {
        fn begin(&mut self, _directory: &Path, _frames: u64) -> Result<(), String> {
            self.started = true;
            Ok(())
        }

        fn request_frame(&mut self, frame: CaptureFrame) -> Result<(), String> {
            if !self.started || self.pending.is_some() {
                return Err("output is not ready".into());
            }
            self.requested.push(frame);
            self.pending = Some((frame, 100));
            Ok(())
        }

        fn poll_complete(&mut self) -> Option<CaptureWrite> {
            let (frame, remaining) = self.pending.as_mut()?;
            if *remaining > 0 {
                *remaining -= 1;
                return None;
            }
            let frame = *frame;
            self.pending = None;
            Some(CaptureWrite {
                frame,
                result: Ok(()),
            })
        }

        fn finish(&mut self) -> Result<(), String> {
            if self.pending.is_some() {
                return Err("write is still pending".into());
            }
            self.finished = true;
            Ok(())
        }

        fn abort(&mut self) {
            self.pending = None;
            self.aborted = true;
        }
    }

    fn machine(fps: u32, frames: u64) -> Machine {
        Machine::new(
            CaptureSession {
                session: 3,
                generation: 7,
            },
            CaptureClock::new(fps, frames, Duration::ZERO).unwrap(),
        )
    }

    fn reply(machine: &mut Machine, operation: CaptureOperation) {
        machine
            .reply(CaptureReply {
                request: CaptureRequest {
                    id: machine.id,
                    operation,
                },
                result: Ok(()),
            })
            .unwrap();
    }

    #[test]
    fn slow_renderer_never_repeats_a_frame_or_frame_event() {
        for fps in [7, 20, 24, 30, 60] {
            let mut machine = machine(fps, 13);
            let mut output = SlowOutput::default();
            output.begin(Path::new("frames"), 13).unwrap();
            let mut frame_events = Vec::new();
            let mut ticks = 0;
            loop {
                match machine.action().unwrap() {
                    Action::Request(operation) => {
                        reply(&mut machine, operation);
                        if let CaptureOperation::Fence(marker) | CaptureOperation::Step(marker) =
                            operation
                        {
                            assert_eq!(machine.action().unwrap(), Action::None);
                            machine.applied(marker);
                        }
                    }
                    Action::ClientTick(index) => {
                        assert_eq!(
                            machine.server_ticks,
                            machine.clock.frame().unwrap().time.total_ticks()
                        );
                        ticks += 1;
                        assert_eq!(index, ticks);
                    }
                    Action::Present(frame) => {
                        frame_events.push(frame.time.index);
                        output
                            .request_frame(CaptureFrame {
                                session: machine.id.session,
                                generation: machine.id.generation,
                                time: frame.time,
                            })
                            .unwrap();
                    }
                    Action::None if machine.phase == Phase::Done => break,
                    Action::None if machine.phase == Phase::WaitingWrite => {
                        if let Some(written) = output.poll_complete() {
                            written.result.unwrap();
                            machine.written(written.frame).unwrap();
                            assert!(machine.written(written.frame).is_err());
                        }
                    }
                    Action::None => panic!("capture stalled"),
                }
            }
            output.finish().unwrap();
            assert!(output.finished);
            assert!(!output.aborted);
            assert_eq!(frame_events, (1..=13).collect::<Vec<_>>());
            assert_eq!(
                output
                    .requested
                    .iter()
                    .map(|frame| frame.time.index)
                    .collect::<Vec<_>>(),
                frame_events
            );
            assert_eq!(ticks, 13 * 20 / u64::from(fps));
            assert_eq!(machine.server_ticks, ticks);
        }
    }

    #[test]
    fn cancel_and_failure_release_before_restoring() {
        for outcome in [Outcome::Cancelled, Outcome::Failed("write failed".into())] {
            let mut machine = machine(30, 9);
            assert_eq!(
                machine.action().unwrap(),
                Action::Request(CaptureOperation::Hold)
            );
            machine.stop(outcome.clone());
            assert_eq!(
                machine.action().unwrap(),
                Action::Request(CaptureOperation::Release)
            );
            assert_eq!(machine.action().unwrap(), Action::None);
            reply(&mut machine, CaptureOperation::Release);
            assert_eq!(machine.phase, Phase::Done);
            assert_eq!(machine.outcome, outcome);
            assert_eq!(machine.clock.time().index, 0);
        }
    }

    #[test]
    fn stale_or_early_application_cannot_open_a_barrier() {
        let mut machine = machine(30, 1);
        machine.action().unwrap();
        reply(&mut machine, CaptureOperation::Hold);
        let Action::Request(CaptureOperation::Fence(marker)) = machine.action().unwrap() else {
            panic!("expected initial fence");
        };
        machine.applied(marker);
        assert_eq!(machine.action().unwrap(), Action::None);
        reply(&mut machine, CaptureOperation::Fence(marker));
        machine.applied(CaptureFenceId {
            generation: 6,
            ..marker
        });
        assert_eq!(machine.action().unwrap(), Action::None);
        machine.applied(marker);
        assert!(matches!(
            machine.action().unwrap(),
            Action::Request(CaptureOperation::Fence(_))
        ));
    }

    #[test]
    fn cancellation_and_failure_discard_a_pending_write() {
        for outcome in [Outcome::Cancelled, Outcome::Failed("encoder failed".into())] {
            let mut machine = machine(30, 2);
            let mut output = SlowOutput::default();
            output.begin(Path::new("frames"), 2).unwrap();
            let frame = loop {
                match machine.action().unwrap() {
                    Action::Request(operation) => {
                        reply(&mut machine, operation);
                        if let CaptureOperation::Fence(marker) = operation {
                            machine.applied(marker);
                        }
                    }
                    Action::Present(schedule) => {
                        break CaptureFrame {
                            session: machine.id.session,
                            generation: machine.id.generation,
                            time: schedule.time,
                        };
                    }
                    action => panic!("unexpected capture action: {action:?}"),
                }
            };
            output.request_frame(frame).unwrap();
            assert!(output.poll_complete().is_none());
            machine.stop(outcome.clone());
            assert!(machine.written(frame).is_err());
            assert_eq!(machine.clock.time().index, 0);
            assert_eq!(
                machine.action().unwrap(),
                Action::Request(CaptureOperation::Release)
            );
            reply(&mut machine, CaptureOperation::Release);
            output.abort();
            assert_eq!(machine.phase, Phase::Done);
            assert_eq!(machine.outcome, outcome);
            assert!(output.aborted);
            assert!(output.poll_complete().is_none());
        }
    }
}
