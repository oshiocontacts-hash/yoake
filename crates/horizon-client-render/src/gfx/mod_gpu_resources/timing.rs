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
}

pub(super) struct Budget {
    started: Instant,
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
}

impl Budget {
    fn new(now: Instant, frame: u64) -> Self {
        Self {
            started: now,
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
        });
        sample.milliseconds += milliseconds;
        if sample.milliseconds > 4.0 {
            self.growth = 0;
            self.good_frames = 0;
        }
        if sample.milliseconds > 50.0 {
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
            if sample.milliseconds <= 4.0 {
                self.good_frames += 1;
                if self.good_frames >= 8 {
                    self.growth = (self.growth + 1).min(4);
                    self.good_frames = 0;
                }
            } else {
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
        if self.timings.is_empty() {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
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
        for (owner, budget) in &mut self.budgets {
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
                frame >= 118
            );
        }
    }

    #[test]
    fn mods_ramp_only_after_complete_used_measured_frames() {
        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        budget.finish_frames(1000, &Default::default());
        assert_eq!(budget.divisor(), 16);
        for frame in 1..=8 {
            budget.sample(start, frame, frame, 1.0);
            budget.finish_frames(frame + 1, &[frame].into_iter().collect());
        }
        assert_eq!(budget.divisor(), 16);
        budget.sample(start, 8, 8, 3.1);
        budget.finish_frames(9, &Default::default());
        assert_eq!(budget.divisor(), 16);
        for frame in 9..=16 {
            budget.sample(start, frame, frame, 1.0);
            budget.finish_frames(frame + 1, &Default::default());
        }
        assert_eq!(budget.divisor(), 8);
        assert_eq!(Budget::new(start, 0).divisor(), 16);
        budget.sample(start, 17, 17, 4.1);
        assert_eq!(budget.divisor(), 16);
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
