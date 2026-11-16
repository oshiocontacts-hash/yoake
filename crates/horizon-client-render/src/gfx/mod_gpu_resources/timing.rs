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
}

pub(super) struct Budget {
    started: Instant,
    first_frame: u64,
    milliseconds: f64,
}

impl Budget {
    fn new(now: Instant, frame: u64) -> Self {
        Self {
            started: now,
            first_frame: frame,
            milliseconds: 0.0,
        }
    }

    fn sample(&mut self, now: Instant, frame: u64, milliseconds: f64) -> bool {
        self.milliseconds += milliseconds;
        let frames = frame.saturating_sub(self.first_frame);
        if now.duration_since(self.started) < Duration::from_secs(2) || frames == 0 {
            return false;
        }
        let over_budget = self.milliseconds / frames as f64 > 4.0;
        *self = Self::new(now, frame);
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
                        .map(|bytes| u64::from_ne_bytes(bytes.try_into().unwrap()))
                        .collect();
                    let milliseconds = ticks
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
                    if budget.sample(now, self.frame_index, milliseconds) {
                        disabled.push((
                            owner,
                            "GPU work exceeded 4 ms/frame over two seconds".to_owned(),
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
        if self.timings.len() >= 256
            || self
                .timings
                .iter()
                .filter(|(identity, _)| *identity == owner)
                .count()
                >= 32
        {
            return Err(Error::Host("GPU timestamp backlog exceeded".to_owned()));
        }
        let count = passes.checked_mul(2).ok_or(Error::Commands)?;
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
                }))
            })?;
        let resource = self.resources.get(owner, handle)?;
        self.resources.destroy(owner, handle)?;
        Ok(Some(resource))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_budget_uses_gpu_samples_not_cpu_elapsed_time() {
        let start = Instant::now();
        let mut budget = Budget::new(start, 0);
        assert!(!budget.sample(start + Duration::from_secs(1), 60, 480.0));
        assert!(budget.sample(start + Duration::from_secs(2), 120, 1.0));
        assert!(!budget.sample(start + Duration::from_secs(4), 240, 480.0));
        assert!(!budget.sample(start + Duration::from_secs(8), 480, 0.0));
    }
}
