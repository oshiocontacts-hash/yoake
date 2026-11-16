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
    pub pipelines: Mutex<Vec<Arc<Mutex<PipelineBudget>>>>,
}

pub(super) struct Budget {
    started: Instant,
    first_frame: u64,
    milliseconds: f64,
    peaks: std::collections::BTreeMap<u64, f64>,
    peak_strikes: std::collections::BTreeSet<u64>,
}

#[derive(Default)]
pub(super) struct PipelineBudget {
    frame: u64,
    used: u64,
    growth: u32,
    good_frames: u32,
    measured_frame: Option<u64>,
    measured_milliseconds: f64,
}

impl PipelineBudget {
    pub fn check(&self, frame: u64, amount: u64, maximum: u64) -> bool {
        let used = if self.frame == frame { self.used } else { 0 };
        let limit = (maximum / 16)
            .max(1)
            .saturating_mul(1 << self.growth)
            .min(maximum);
        used.saturating_add(amount) <= limit
    }

    pub fn charge(&mut self, frame: u64, amount: u64) {
        if self.frame != frame {
            self.frame = frame;
            self.used = 0;
        }
        self.used = self.used.saturating_add(amount);
    }

    fn measured(&mut self, frame: u64, milliseconds: f64) {
        if self.measured_frame.is_some_and(|latest| frame < latest) {
            if milliseconds > 4.0 {
                self.growth = 0;
                self.good_frames = 0;
            }
            return;
        }
        let new_frame = self.measured_frame != Some(frame);
        if new_frame {
            self.measured_frame = Some(frame);
            self.measured_milliseconds = 0.0;
        }
        self.measured_milliseconds += milliseconds;
        if self.measured_milliseconds > 4.0 {
            self.growth = 0;
            self.good_frames = 0;
        } else if new_frame {
            self.good_frames += 1;
            if self.good_frames >= 8 {
                self.growth = (self.growth + 1).min(4);
                self.good_frames = 0;
            }
        }
    }
}

impl Budget {
    fn new(now: Instant, frame: u64) -> Self {
        Self {
            started: now,
            first_frame: frame,
            milliseconds: 0.0,
            peaks: Default::default(),
            peak_strikes: Default::default(),
        }
    }

    fn sample(&mut self, now: Instant, frame: u64, sample_frame: u64, milliseconds: f64) -> bool {
        let peak = self.peaks.entry(sample_frame).or_default();
        *peak += milliseconds;
        if *peak > 50.0 {
            self.peak_strikes.insert(sample_frame);
            if self.peak_strikes.len() >= 3 {
                return true;
            }
        }
        self.peaks
            .retain(|identity, _| identity.saturating_add(256) >= frame);
        self.milliseconds += milliseconds;
        let frames = frame.saturating_sub(self.first_frame);
        if now.duration_since(self.started) < Duration::from_secs(2) || frames == 0 {
            return false;
        }
        let over_budget = self.milliseconds / frames as f64 > 4.0;
        self.started = now;
        self.first_frame = frame;
        self.milliseconds = 0.0;
        over_budget
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
    pub fn charge_cpu(&mut self, owner: ModId, elapsed: Duration) -> Result<(), Error> {
        let now = Instant::now();
        let frame = self.frame_index;
        if self
            .budgets
            .entry(owner)
            .or_insert_with(|| Budget::new(now, frame))
            .sample(now, frame, frame, elapsed.as_secs_f64() * 1000.0)
        {
            self.disable(
                owner,
                "GPU host and device work exceeded the frame time budget".to_owned(),
            );
            return Err(Error::Disabled);
        }
        Ok(())
    }

    pub fn collect_timings(&mut self, device: &wgpu::Device) {
        self.frame_index = self.frame_index.saturating_add(1);
        if self.timings.is_empty() {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        let now = Instant::now();
        let mut disabled = Vec::new();
        for (owner, resource) in std::mem::take(&mut self.timings) {
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
                    if let Ok(pipelines) = ticket.pipelines.lock() {
                        for pipeline in pipelines.iter() {
                            if let Ok(mut pipeline) = pipeline.lock() {
                                pipeline.measured(ticket.frame, milliseconds);
                            }
                        }
                    }
                    let budget = self
                        .budgets
                        .entry(owner)
                        .or_insert_with(|| Budget::new(now, self.frame_index));
                    if budget.sample(now, self.frame_index, ticket.frame, milliseconds) {
                        disabled.push((
                            owner,
                            "GPU work exceeded 4 ms/frame over two seconds or the 50 ms peak cap"
                                .to_owned(),
                        ));
                    }
                }
            }
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
    ) -> Result<Option<Owned>, Error> {
        if passes == 0 || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Ok(None);
        }
        let count = query_count(
            owner,
            passes,
            self.timings.iter().map(|(identity, _)| *identity),
        )?;
        let size = (u64::from(count) * 8).div_ceil(256) * 256;
        let handle = self
            .resources
            .create(owner, Allocation::Buffer(size * 3), || {
                Ok(Value::Timing(Ticket {
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
                    pipelines: Default::default(),
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
                frame == 118
            );
        }
    }

    #[test]
    fn pipelines_ramp_only_after_safe_timestamp_samples() {
        let mut pipeline = PipelineBudget::default();
        assert!(pipeline.check(1, 4, 64));
        assert!(!pipeline.check(1, 5, 64));
        pipeline.charge(1, 4);
        assert!(!pipeline.check(1, 1, 64));
        for frame in 2..10 {
            pipeline.measured(frame, 1.0);
        }
        assert!(pipeline.check(10, 8, 64));
        pipeline.measured(10, 4.1);
        assert!(!pipeline.check(11, 5, 64));
        for frame in 11..19 {
            pipeline.measured(frame, 3.0);
        }
        assert!(pipeline.check(19, 8, 64));
        pipeline.measured(18, 2.0);
        assert!(!pipeline.check(19, 5, 64));
        for _ in 0..8 {
            pipeline.measured(17, 1.0);
        }
        assert!(!pipeline.check(19, 5, 64));
        let unmeasured = PipelineBudget::default();
        assert!(!unmeasured.check(10_000, 5, 64));
    }
}
