use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Allocation, Error, Gpu, ModId, Owned, Value};

pub(super) struct Ticket {
    pub queries: wgpu::QuerySet,
    pub resolve: wgpu::Buffer,
    pub read: wgpu::Buffer,
    pub count: u32,
    status: Arc<Mutex<Option<Result<(), String>>>>,
    period: f64,
    pub frame: u64,
    workload: Workload,
    allowance: Workload,
}

pub(super) struct Completion {
    frame: u64,
    submitted_at: Instant,
    finished: Arc<Mutex<Option<Instant>>>,
    fallback: bool,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Workload {
    pixels: u64,
    vertices: u64,
    compute: u64,
}

impl Workload {
    pub fn from_commands(
        commands: impl IntoIterator<Item = super::super::mod_gpu::CommandKind>,
    ) -> Self {
        use super::super::mod_gpu::CommandKind;

        let mut workload = Self::default();
        for command in commands {
            match command {
                CommandKind::Render {
                    count,
                    instances,
                    fragment_work,
                    ..
                } => {
                    workload.pixels = workload.pixels.saturating_add(fragment_work);
                    workload.vertices = workload
                        .vertices
                        .saturating_add(u64::from(count) * u64::from(instances));
                }
                CommandKind::Compute { groups, cost, .. } => {
                    let work = groups.into_iter().fold(cost.max(1), |work, group| {
                        work.saturating_mul(u64::from(group))
                    });
                    workload.compute = workload.compute.saturating_add(work);
                }
                _ => {}
            }
        }
        workload
    }

    fn add(&mut self, other: Self) {
        self.pixels = self.pixels.saturating_add(other.pixels);
        self.vertices = self.vertices.saturating_add(other.vertices);
        self.compute = self.compute.saturating_add(other.compute);
    }

    fn uses_quarter(self, allowance: Self) -> bool {
        [
            (self.pixels, allowance.pixels),
            (self.vertices, allowance.vertices),
            (self.compute, allowance.compute),
        ]
        .into_iter()
        .any(|(used, limit)| limit > 0 && u128::from(used) * 4 >= u128::from(limit))
    }
}

pub(super) struct Budget {
    started: Instant,
    throttle_until: Instant,
    frames: std::collections::VecDeque<(u64, Instant)>,
    samples: std::collections::BTreeMap<u64, Sample>,
    peak_strikes: std::collections::BTreeMap<u64, Instant>,
    growth: u32,
    good_frames: u32,
}

struct Sample {
    at: Instant,
    milliseconds: f64,
    complete: bool,
    workload: Workload,
    allowance: Workload,
}

impl Budget {
    fn new(now: Instant, frame: u64) -> Self {
        Self {
            started: now,
            throttle_until: now,
            frames: [(frame, now)].into_iter().collect(),
            samples: Default::default(),
            peak_strikes: Default::default(),
            growth: 0,
            good_frames: 0,
        }
    }

    pub fn divisor(&self) -> u64 {
        16 >> self.growth
    }

    fn advance(&mut self, now: Instant, frame: u64) {
        if self
            .frames
            .back()
            .is_none_or(|(identity, _)| *identity != frame)
        {
            self.frames.push_back((frame, now));
        }
        while self
            .frames
            .front()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= Duration::from_secs(2))
        {
            self.frames.pop_front();
        }
        self.peak_strikes
            .retain(|_, at| now.saturating_duration_since(*at) < Duration::from_secs(60));
        self.samples
            .retain(|_, sample| now.saturating_duration_since(sample.at) < Duration::from_secs(2));
    }

    fn sample(&mut self, now: Instant, frame: u64, sample_frame: u64, milliseconds: f64) -> bool {
        self.advance(now, frame);
        let sample = self.samples.entry(sample_frame).or_insert(Sample {
            at: now,
            milliseconds: 0.0,
            complete: false,
            workload: Workload::default(),
            allowance: Workload::default(),
        });
        sample.milliseconds += milliseconds;
        if sample.milliseconds > 4.0 {
            self.growth = 0;
            self.good_frames = 0;
        }
        if sample.milliseconds > 50.0 {
            self.throttle_until = now + Duration::from_millis(50);
            self.peak_strikes.entry(sample_frame).or_insert(now);
            if self.peak_strikes.len() >= 3 {
                return true;
            }
        }
        now.saturating_duration_since(self.started) >= Duration::from_secs(2)
            && !self.frames.is_empty()
            && self
                .samples
                .values()
                .map(|sample| sample.milliseconds)
                .sum::<f64>()
                / self.frames.len() as f64
                > 4.0
    }

    fn finish_frames(&mut self, frame: u64, pending: &std::collections::BTreeSet<u64>) {
        for (identity, sample) in &mut self.samples {
            if *identity >= frame || sample.complete || pending.contains(identity) {
                continue;
            }
            sample.complete = true;
            if (0.25..=4.0).contains(&sample.milliseconds)
                && sample.workload.uses_quarter(sample.allowance)
            {
                self.good_frames += 1;
                if self.good_frames >= 8 {
                    self.growth = (self.growth + 1).min(4);
                    self.good_frames = 0;
                }
            } else if sample.milliseconds > 4.0 {
                self.growth = 0;
                self.good_frames = 0;
            }
        }
    }
}

impl Ticket {
    pub fn map(&self) {
        let status = self.status.clone();
        self.read
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if let Ok(mut status) = status.lock() {
                    *status = Some(result.map_err(|error| error.to_string()));
                }
            });
    }
}

impl Gpu {
    pub(super) fn submission_ready(&self, owner: ModId) -> bool {
        self.submission_ready_at(owner, Instant::now())
    }

    fn submission_ready_at(&self, owner: ModId, now: Instant) -> bool {
        self.budgets
            .get(&owner)
            .is_none_or(|budget| now >= budget.throttle_until)
        && !self.completions.iter().any(|(identity, completion)| {
            *identity == owner && completion.frame != self.frame_index
        }) && !self.timings.iter().any(|(identity, resource)| {
            *identity == owner
                && matches!(resource.value(), Value::Timing(ticket) if ticket.frame != self.frame_index)
        })
    }

    pub(super) fn submission_started(
        &mut self,
        owner: ModId,
        fallback: bool,
    ) -> Arc<Mutex<Option<Instant>>> {
        self.submission_started_at(owner, fallback, Instant::now())
    }

    fn submission_started_at(
        &mut self,
        owner: ModId,
        fallback: bool,
        now: Instant,
    ) -> Arc<Mutex<Option<Instant>>> {
        self.budgets
            .entry(owner)
            .or_insert_with(|| Budget::new(now, self.frame_index));
        let finished = Arc::new(Mutex::new(None));
        self.completions.push((
            owner,
            Completion {
                frame: self.frame_index,
                submitted_at: now,
                finished: finished.clone(),
                fallback,
            },
        ));
        finished
    }

    fn record_gpu_sample(
        &mut self,
        owner: ModId,
        now: Instant,
        frame: u64,
        milliseconds: f64,
        workload: Option<(Workload, Workload)>,
    ) {
        if self.disabled_reason(owner).is_some() {
            return;
        }
        let budget = self
            .budgets
            .entry(owner)
            .or_insert_with(|| Budget::new(now, self.frame_index));
        let exceeded = budget.sample(now, self.frame_index, frame, milliseconds);
        if let Some((workload, allowance)) = workload {
            if let Some(sample) = budget.samples.get_mut(&frame) {
                sample.workload.add(workload);
                sample.allowance = allowance;
            }
        }
        if exceeded {
            if let Some(hooks) = &self.submission_hooks {
                (hooks.fault)(owner);
            }
            self.disable(
                owner,
                "GPU work exceeded 4 ms/frame over two seconds or the 50 ms peak cap".to_owned(),
            );
        }
    }

    fn collect_completions(&mut self, now: Instant) {
        for (owner, completion) in std::mem::take(&mut self.completions) {
            if self.disabled_reason(owner).is_some() {
                continue;
            }
            let finished = completion
                .finished
                .lock()
                .ok()
                .and_then(|finished| *finished);
            if let Some(finished) = finished {
                if completion.fallback {
                    self.record_gpu_sample(
                        owner,
                        now,
                        completion.frame,
                        finished
                            .saturating_duration_since(completion.submitted_at)
                            .as_secs_f64()
                            * 1000.0,
                        None,
                    );
                }
            } else {
                self.completions.push((owner, completion));
            }
        }
    }

    pub fn charge_cpu(&mut self, owner: ModId, elapsed: Duration) -> Result<(), Error> {
        let now = Instant::now();
        let frame = self.frame_index;
        if self
            .cpu_budgets
            .entry(owner)
            .or_insert_with(|| Budget::new(now, frame))
            .sample(now, frame, frame, elapsed.as_secs_f64() * 1000.0)
        {
            self.disable(
                owner,
                "GPU host work exceeded the frame time budget".to_owned(),
            );
            return Err(Error::Disabled);
        }
        Ok(())
    }

    pub fn collect_timings(&mut self, device: &wgpu::Device) {
        self.frame_index = self.frame_index.saturating_add(1);
        let now = Instant::now();
        for budget in self
            .budgets
            .values_mut()
            .chain(self.cpu_budgets.values_mut())
        {
            budget.advance(now, self.frame_index);
        }
        let _ = device.poll(wgpu::PollType::Poll);
        self.collect_completions(Instant::now());
        let mut disabled = Vec::new();
        for (owner, resource) in std::mem::take(&mut self.timings) {
            if self.disabled_reason(owner).is_some() {
                continue;
            }
            let Value::Timing(ticket) = resource.value() else {
                continue;
            };
            let status = ticket
                .status
                .lock()
                .ok()
                .and_then(|mut status| status.take());
            match status {
                None => self.timings.push((owner, resource)),
                Some(Err(error)) => {
                    disabled.push((owner, format!("GPU timestamp read failed: {error}")))
                }
                Some(Ok(())) => {
                    let bytes = match ticket.read.slice(..).get_mapped_range() {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            ticket.read.unmap();
                            disabled
                                .push((owner, format!("GPU timestamp mapping failed: {error}")));
                            continue;
                        }
                    };
                    let ticks: Vec<_> = bytes[..ticket.count as usize * 8]
                        .chunks_exact(8)
                        .map(|bytes| {
                            u64::from_ne_bytes([
                                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
                                bytes[6], bytes[7],
                            ])
                        })
                        .collect();
                    let milliseconds: f64 = ticks
                        .chunks_exact(2)
                        .map(|pair| {
                            pair[1].saturating_sub(pair[0]) as f64 * ticket.period / 1_000_000.0
                        })
                        .sum();
                    drop(bytes);
                    ticket.read.unmap();
                    self.record_gpu_sample(
                        owner,
                        now,
                        ticket.frame,
                        milliseconds,
                        Some((ticket.workload, ticket.allowance)),
                    );
                }
            }
        }
        for (owner, budget) in &mut self.budgets {
            if !self.timestamps {
                continue;
            }
            let pending = self
                .timings
                .iter()
                .filter_map(|(identity, resource)| {
                    if identity == owner {
                        if let Value::Timing(ticket) = resource.value() {
                            return Some(ticket.frame);
                        }
                    }
                    None
                })
                .chain(
                    self.completions
                        .iter()
                        .filter_map(|(identity, completion)| {
                            (identity == owner).then_some(completion.frame)
                        }),
                )
                .collect();
            budget.finish_frames(self.frame_index, &pending);
        }
        for (owner, reason) in disabled {
            log::warn!("WASM GPU mod {} disabled: {reason}", owner.0);
            self.disable(owner, reason);
        }
    }

    pub(super) fn timing_ticket(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        owner: ModId,
        passes: u32,
        workload: Workload,
    ) -> Result<Option<Owned>, Error> {
        if passes == 0 || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Ok(None);
        }
        self.budgets
            .entry(owner)
            .or_insert_with(|| Budget::new(Instant::now(), self.frame_index));
        let count = query_count(
            owner,
            passes,
            self.timings.iter().map(|(identity, _)| *identity),
        )?;
        let size = (u64::from(count) * 8).div_ceil(256) * 256;
        let allowance = Workload {
            pixels: self.pixel_limit(owner, true).saturating_mul(2048),
            vertices: self.vertex_limit(owner, true),
            compute: self.compute_limit(owner, true),
        };
        let handle = self
            .resources
            .create(owner, Allocation::Buffer(size * 3), || {
                Ok(Value::Timing(Ticket {
                    workload,
                    allowance,
                    queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("Yoake mod GPU timestamps"),
                        ty: wgpu::QueryType::Timestamp,
                        count,
                    }),
                    resolve: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Yoake mod timestamp resolve"),
                        size,
                        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    }),
                    read: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Yoake mod timestamp readback"),
                        size,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    count,
                    status: Arc::new(Mutex::new(None)),
                    period: f64::from(queue.get_timestamp_period()),
                    frame: self.frame_index,
                }))
            })?;
        let resource = self.resources.get(owner, handle)?;
        self.resources.destroy(owner, handle)?;
        Ok(Some(resource))
    }
}

fn query_count(
    owner: ModId,
    passes: u32,
    pending: impl Iterator<Item = ModId>,
) -> Result<u32, Error> {
    if pending.filter(|identity| *identity == owner).count() >= 32 {
        return Err(Error::Host("GPU timestamp backlog exceeded".to_owned()));
    }
    let count = passes.checked_mul(2).ok_or(Error::Commands)?;
    if count > 4096 {
        return Err(Error::Passes);
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_latency_disables_a_slow_mod_without_timestamps() {
        let start = Instant::now();
        let owner = ModId(1);
        let faults = Arc::new(Mutex::new(Vec::new()));
        let fault_log = faults.clone();
        let mut gpu = Gpu::default();
        gpu.set_submission_hooks(super::super::SubmissionHooks {
            before: Arc::new(|_| Ok(())),
            complete: Arc::new(|_| {}),
            fault: Arc::new(move |owner| fault_log.lock().unwrap().push(owner)),
        });
        assert!(!gpu.timestamps);
        for frame in 1..=3 {
            let submitted_at = start + Duration::from_millis((frame - 1) * 400);
            gpu.frame_index = frame;
            assert!(gpu.submission_ready_at(owner, submitted_at));
            let finished = gpu.submission_started_at(owner, true, submitted_at);
            gpu.frame_index += 1;
            assert!(!gpu.submission_ready_at(owner, submitted_at));
            assert!(gpu.submission_ready_at(ModId(2), submitted_at));
            let completed_at = submitted_at + Duration::from_millis(200);
            *finished.lock().unwrap() = Some(completed_at);
            gpu.collect_completions(completed_at);
            assert_eq!(gpu.budget_divisor(owner, false), 16);
            assert_eq!(gpu.disabled_reason(owner).is_some(), frame == 3);
            if frame < 3 {
                assert!(!gpu.submission_ready_at(owner, completed_at));
                assert!(gpu.submission_ready_at(owner, completed_at + Duration::from_millis(50)));
            }
        }
        assert_eq!(*faults.lock().unwrap(), vec![owner]);
        assert!(gpu.disabled_reason(ModId(2)).is_none());
        assert!(gpu.completions.is_empty());
    }

    #[test]
    fn completion_latency_enforces_the_rolling_average_without_timestamps() {
        let start = Instant::now();
        let owner = ModId(1);
        let mut gpu = Gpu::default();
        for frame in 1..=120 {
            gpu.frame_index = frame;
            let submitted_at = start + Duration::from_millis(frame * 17);
            let finished = gpu.submission_started_at(owner, true, submitted_at);
            let completed_at = submitted_at + Duration::from_millis(5);
            *finished.lock().unwrap() = Some(completed_at);
            gpu.collect_completions(completed_at);
            if gpu.disabled_reason(owner).is_some() {
                assert_eq!(frame, 119);
                return;
            }
        }
        panic!("unmeasured adapters must enforce the rolling GPU budget");
    }

    #[test]
    fn completion_markers_do_not_double_charge_timestamp_samples() {
        let start = Instant::now();
        let owner = ModId(1);
        let mut gpu = Gpu::default();
        gpu.frame_index = 1;
        let finished = gpu.submission_started_at(owner, false, start);
        gpu.record_gpu_sample(owner, start, 1, 1.0, None);
        *finished.lock().unwrap() = Some(start + Duration::from_millis(200));
        gpu.collect_completions(start + Duration::from_millis(200));
        assert_eq!(gpu.budgets[&owner].samples[&1].milliseconds, 1.0);
        assert!(gpu.completions.is_empty());
    }

    #[test]
    fn one_triangle_zero_cost_frames_never_ramp() {
        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        let covered_pixels = 1280 * 720;
        let workload =
            Workload::from_commands([super::super::super::mod_gpu::CommandKind::Render {
                count: 3,
                instances: 1,
                fragment_work: covered_pixels,
                pixel_limit: covered_pixels * 64,
                vertex_limit: 1 << 20,
                new_pass: true,
            }]);
        for frame in 1..=1000 {
            let now = start + Duration::from_millis(frame * 17);
            assert!(!budget.sample(now, frame, frame, 0.0));
            let sample = budget.samples.get_mut(&frame).unwrap();
            sample.workload = workload;
            sample.allowance = Workload {
                pixels: covered_pixels * 64 * 2048,
                vertices: 1 << 20,
                ..Default::default()
            };
            budget.finish_frames(frame + 1, &Default::default());
            assert_eq!(budget.divisor(), 16);
        }
    }

    #[test]
    fn allowance_use_cannot_replace_nontrivial_gpu_measurements() {
        let start = Instant::now();
        for (milliseconds, expected) in [(0.0, 16), (0.249, 16), (0.25, 8)] {
            let mut budget = Budget::new(start, 0);
            for frame in 1..=8 {
                budget.sample(start, frame, frame, milliseconds);
                let sample = budget.samples.get_mut(&frame).unwrap();
                sample.workload.pixels = 100;
                sample.allowance.pixels = 400;
                budget.finish_frames(frame + 1, &Default::default());
            }
            assert_eq!(budget.divisor(), expected);
        }
        let mut unmeasured = Budget::new(start, 0);
        for frame in 1..=1000 {
            unmeasured.advance(start, frame);
            unmeasured.finish_frames(frame + 1, &Default::default());
        }
        assert_eq!(unmeasured.divisor(), 16);
    }

    #[test]
    fn query_limits_and_backlog_are_isolated_per_owner() {
        let owner = ModId(1);
        assert_eq!(query_count(owner, 2048, std::iter::empty()), Ok(4096));
        assert_eq!(
            query_count(owner, 2049, std::iter::empty()),
            Err(Error::Passes)
        );
        assert_eq!(
            query_count(owner, u32::MAX, std::iter::empty()),
            Err(Error::Commands)
        );
        assert_eq!(
            query_count(owner, 1, std::iter::repeat_n(ModId(2), 32)),
            Ok(2)
        );
        assert!(query_count(owner, 1, std::iter::repeat_n(owner, 32)).is_err());
        assert_eq!(query_count(owner, 1, std::iter::repeat_n(owner, 31)), Ok(2));
    }

    #[test]
    fn timestamp_budget_checks_aggregate_peaks_and_average() {
        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        assert!(!budget.sample(start, 1, 1, 25.0));
        assert!(!budget.sample(start, 1, 1, 25.0));
        assert!(!budget.sample(start, 1, 1, 0.1));
        assert!(!budget.sample(start, 1, 1, 100.0));
        assert!(!budget.sample(start, 2, 2, 50.1));
        assert!(budget.sample(start, 3, 3, 50.1));
        let mut budget = Budget::new(start, 0);
        for frame in 1..=120 {
            assert_eq!(
                budget.sample(start + Duration::from_millis(frame * 17), frame, frame, 5.0),
                frame >= 118
            );
        }
    }

    #[test]
    fn mods_ramp_only_after_complete_used_measured_frames() {
        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        let workload = Workload {
            pixels: 100,
            ..Default::default()
        };
        let allowance = Workload {
            pixels: 400,
            ..Default::default()
        };
        budget.finish_frames(1000, &Default::default());
        assert_eq!(budget.divisor(), 16);
        for frame in 1..=8 {
            budget.sample(start, frame, frame, 1.0);
            let sample = budget.samples.get_mut(&frame).unwrap();
            sample.workload = workload;
            sample.allowance = allowance;
            budget.finish_frames(frame + 1, &[frame].into_iter().collect());
        }
        assert_eq!(budget.divisor(), 16);
        budget.sample(start, 8, 8, 3.1);
        budget.finish_frames(9, &Default::default());
        assert_eq!(budget.divisor(), 16);
        for frame in 9..=16 {
            budget.sample(start, frame, frame, 1.0);
            let sample = budget.samples.get_mut(&frame).unwrap();
            sample.workload = workload;
            sample.allowance = allowance;
            budget.finish_frames(frame + 1, &Default::default());
        }
        assert_eq!(budget.divisor(), 8);
        assert_eq!(Budget::new(start, 0).divisor(), 16);
        budget.sample(start, 17, 17, 4.1);
        assert_eq!(budget.divisor(), 16);
    }

    #[test]
    fn ramp_requires_a_quarter_of_the_current_allowance() {
        let start = Instant::now();
        for category in 0..3 {
            let mut budget = Budget::new(start, 0);
            for frame in 1..=48 {
                let (used, limit, expected) = match frame {
                    1..=16 => (99, 400, 16),
                    17..=23 => (100, 400, 16),
                    24 => (100, 400, 8),
                    25..=40 => (100, 800, 8),
                    41..=47 => (200, 800, 8),
                    _ => (200, 800, 4),
                };
                budget.sample(start, frame, frame, 1.0);
                let sample = budget.samples.get_mut(&frame).unwrap();
                match category {
                    0 => {
                        sample.workload.pixels = used;
                        sample.allowance.pixels = limit;
                    }
                    1 => {
                        sample.workload.vertices = used;
                        sample.allowance.vertices = limit;
                    }
                    _ => {
                        sample.workload.compute = used;
                        sample.allowance.compute = limit;
                    }
                }
                budget.finish_frames(frame + 1, &Default::default());
                assert_eq!(
                    budget.divisor(),
                    expected,
                    "category {category}, frame {frame}"
                );
            }
        }
    }

    #[test]
    fn ramp_aggregates_work_across_measured_stages_but_not_idle_frames() {
        use super::super::super::mod_gpu::CommandKind;

        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        for frame in 1..=64 {
            budget.sample(start, frame, frame, 0.1);
            let sample = budget.samples.get_mut(&frame).unwrap();
            sample.allowance.pixels = 400;
            budget.finish_frames(frame + 1, &Default::default());
        }
        assert_eq!(budget.divisor(), 16);
        for frame in 65..=72 {
            for _ in 0..2 {
                budget.sample(start, frame, frame, 0.5);
                let sample = budget.samples.get_mut(&frame).unwrap();
                sample
                    .workload
                    .add(Workload::from_commands([CommandKind::Render {
                        count: 6,
                        instances: 1,
                        fragment_work: 50,
                        pixel_limit: 400,
                        vertex_limit: 400,
                        new_pass: true,
                    }]));
                sample.allowance.pixels = 400;
            }
            budget.finish_frames(frame + 1, &Default::default());
        }
        assert_eq!(budget.divisor(), 8);
    }

    #[test]
    fn peak_strikes_expire_and_windows_roll() {
        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        assert!(!budget.sample(start, 1, 1, 51.0));
        assert!(!budget.sample(start, 2, 2, 51.0));
        for frame in 3..4000 {
            budget.advance(start + Duration::from_millis(frame * 16), frame);
        }
        assert!(!budget.sample(start + Duration::from_secs(64), 4000, 4000, 51.0));
        assert_eq!(budget.peak_strikes.len(), 1);
        assert!(!budget.sample(start + Duration::from_secs(65), 4060, 4060, 1.0));
        assert_eq!(budget.samples.len(), 2);
    }

    #[test]
    fn idle_history_does_not_dilute_a_later_expensive_window() {
        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        for frame in 1..=1000 {
            budget.advance(start + Duration::from_millis(frame * 17), frame);
        }
        for frame in 1001..=1120 {
            let exceeded =
                budget.sample(start + Duration::from_millis(frame * 17), frame, frame, 5.0);
            if frame == 1120 {
                assert!(exceeded);
            }
        }
        assert!(budget.frames.len() <= 118);
    }
}
