//! Bounded, copy-only final output observation (@API:replay_audio_mix@).

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

const SLOTS: usize = 64;
const SAMPLES: usize = 4096;

#[derive(Default)]
pub(crate) struct PendingDecodes {
    count: Mutex<usize>,
    finished: Condvar,
}

pub(crate) struct DecodeGuard(Arc<PendingDecodes>);

impl PendingDecodes {
    pub(crate) fn begin(self: &Arc<Self>) -> DecodeGuard {
        *self.count.lock().unwrap_or_else(|error| error.into_inner()) += 1;
        DecodeGuard(self.clone())
    }

    pub(crate) fn wait(&self) {
        let mut count = self.count.lock().unwrap_or_else(|error| error.into_inner());
        while *count != 0 {
            count = self
                .finished
                .wait(count)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

impl Drop for DecodeGuard {
    fn drop(&mut self) {
        let mut count = self.0.count.lock().unwrap_or_else(|error| error.into_inner());
        *count -= 1;
        if *count == 0 {
            self.0.finished.notify_all();
        }
    }
}

/// Interleaved, post-limiter floating-point PCM copied from the final mixer.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeAudioMixChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    /// Monotonic chunk sequence, including chunks dropped by the producer.
    pub sequence: u64,
    /// Monotonic sample-frame offset; interpret each chunk at its own rate.
    pub first_frame: u64,
    /// The RH-6 frame being rendered, or `None` for real-time output.
    pub frame_index: Option<u64>,
}

/// One consumer's independent drain. Gaps include overwritten and contended chunks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeAudioMixBatch {
    pub chunks: Vec<NativeAudioMixChunk>,
    pub dropped_chunks: u64,
}

struct Slot {
    chunk: NativeAudioMixChunk,
    length: usize,
}

struct Ring {
    slots: Vec<Slot>,
    next: usize,
    count: usize,
}

pub(crate) struct MixTap {
    ring: Mutex<Ring>,
    sequence: AtomicU64,
    frames: AtomicU64,
    pub(crate) offline: AtomicBool,
    pub(crate) decodes: Arc<PendingDecodes>,
}

impl MixTap {
    pub(crate) fn new() -> Self {
        Self {
            ring: Mutex::new(Ring {
                slots: (0..SLOTS)
                    .map(|_| Slot {
                        chunk: NativeAudioMixChunk {
                            samples: vec![0.0; SAMPLES],
                            sample_rate: 0,
                            channels: 0,
                            sequence: 0,
                            first_frame: 0,
                            frame_index: None,
                        },
                        length: 0,
                    })
                    .collect(),
                next: 0,
                count: 0,
            }),
            sequence: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            offline: AtomicBool::new(false),
            decodes: Arc::default(),
        }
    }

    pub(crate) fn record(
        &self,
        samples: &[f32],
        channels: usize,
        sample_rate: u32,
        frame_index: Option<u64>,
    ) {
        if channels == 0 || channels > SAMPLES || channels > u16::MAX as usize {
            return;
        }
        let chunk_size = SAMPLES / channels * channels;
        for samples in samples.chunks(chunk_size) {
            let length = samples.len() / channels * channels;
            if length == 0 {
                continue;
            }
            let sequence = self.sequence.fetch_add(1, Ordering::AcqRel);
            let first_frame = self
                .frames
                .fetch_add((length / channels) as u64, Ordering::Relaxed);
            let Ok(mut ring) = self.ring.try_lock() else {
                continue;
            };
            let next = ring.next;
            let slot = &mut ring.slots[next];
            slot.chunk.samples[..length].copy_from_slice(&samples[..length]);
            slot.chunk.sample_rate = sample_rate;
            slot.chunk.channels = channels as u16;
            slot.chunk.sequence = sequence;
            slot.chunk.first_frame = first_frame;
            slot.chunk.frame_index = frame_index;
            slot.length = length;
            ring.next = (next + 1) % SLOTS;
            ring.count = (ring.count + 1).min(SLOTS);
        }
    }

    pub(crate) fn drain(&self, cursor: &mut u64) -> NativeAudioMixBatch {
        let ring = self.ring.lock().unwrap_or_else(|error| error.into_inner());
        let through = self.sequence.load(Ordering::Acquire);
        let mut chunks = Vec::new();
        let oldest = (ring.next + SLOTS - ring.count) % SLOTS;
        for offset in 0..ring.count {
            let slot = &ring.slots[(oldest + offset) % SLOTS];
            if slot.chunk.sequence >= *cursor && slot.chunk.sequence < through {
                chunks.push(NativeAudioMixChunk {
                    samples: slot.chunk.samples[..slot.length].to_vec(),
                    sample_rate: slot.chunk.sample_rate,
                    channels: slot.chunk.channels,
                    sequence: slot.chunk.sequence,
                    first_frame: slot.chunk.first_frame,
                    frame_index: slot.chunk.frame_index,
                });
            }
        }
        let dropped_chunks = through
            .saturating_sub(*cursor)
            .saturating_sub(chunks.len() as u64);
        *cursor = through;
        NativeAudioMixBatch {
            chunks,
            dropped_chunks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_preparation_waits_for_in_flight_decodes_on_the_caller() {
        let pending = Arc::new(PendingDecodes::default());
        let guard = pending.begin();
        assert_eq!(*pending.count.lock().unwrap(), 1);
        let worker = std::thread::spawn(move || drop(guard));
        pending.wait();
        assert_eq!(*pending.count.lock().unwrap(), 0);
        worker.join().unwrap();
    }

    #[test]
    fn bounded_ring_overwrites_old_chunks_and_consumers_are_independent() {
        let tap = MixTap::new();
        for index in 0..SLOTS + 3 {
            tap.record(&[index as f32; 4], 2, 48_000, Some(7));
        }
        let mut first = 0;
        let mut second = 0;
        let batch = tap.drain(&mut first);
        assert_eq!(batch.chunks.len(), SLOTS);
        assert_eq!(batch.dropped_chunks, 3);
        assert_eq!(batch.chunks[0].first_frame, 6);
        assert_eq!(batch.chunks[0].samples, vec![3.0; 4]);
        assert_eq!(batch.chunks[0].frame_index, Some(7));
        assert_eq!(tap.drain(&mut second), batch);
        assert!(tap.drain(&mut first).chunks.is_empty());
    }

    #[test]
    fn a_busy_consumer_drops_instead_of_waiting_or_allocating() {
        let tap = MixTap::new();
        let storage = tap.ring.lock().unwrap();
        tap.record(&[0.5; 16], 2, 48_000, None);
        assert_eq!(storage.count, 0);
        assert!(
            storage
                .slots
                .iter()
                .all(|slot| slot.chunk.samples.len() == SAMPLES)
        );
        drop(storage);
        let mut cursor = 0;
        let batch = tap.drain(&mut cursor);
        assert_eq!(batch.dropped_chunks, 1);
        assert!(batch.chunks.is_empty());
    }
}
