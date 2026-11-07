//! Bounded, mod-owned PCM queues. No vanilla sources or channels are reserved.

use crate::audio::NativeSoundError;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};

/// Listener streams are non-positional; missing entities stay open but inaudible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeStreamAnchor {
    Entity(u32),
    Position([f64; 3]),
    Listener,
}

/// An opaque, mod-owned handle, invalidated when its world's queues close.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeAudioStreamId {
    owner: usize,
    session: u64,
    serial: u64,
}

/// Independent of vanilla's channel budget and finite local sound quotas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeAudioStreamLimits {
    pub streams_per_mod: usize,
    pub streams_global: usize,
    pub buffered_seconds_per_mod: usize,
    pub buffered_seconds_global: usize,
    pub seconds_per_stream: usize,
    pub minimum_sample_rate: u32,
    pub maximum_sample_rate: u32,
}

impl Default for NativeAudioStreamLimits {
    fn default() -> Self {
        Self {
            streams_per_mod: 8,
            streams_global: 32,
            buffered_seconds_per_mod: 8,
            buffered_seconds_global: 32,
            seconds_per_stream: 1,
            minimum_sample_rate: 8_000,
            maximum_sample_rate: 192_000,
        }
    }
}

struct Ring {
    samples: Box<[AtomicU32]>,
    channels: usize,
    frames: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU64,
}

pub(crate) struct PcmWriter(Arc<Ring>);

pub(crate) struct PcmReader {
    ring: Arc<Ring>,
    phase: f64,
}

/// Single producer / single consumer, with independently published frame cursors.
pub(crate) fn pcm_ring(frames: usize, channels: usize) -> (PcmWriter, PcmReader) {
    assert!(frames > 0 && matches!(channels, 1 | 2));
    let ring = Arc::new(Ring {
        samples: (0..frames * channels).map(|_| AtomicU32::new(0)).collect(),
        channels,
        frames,
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
        dropped: AtomicU64::new(0),
    });
    (PcmWriter(Arc::clone(&ring)), PcmReader { ring, phase: 0.0 })
}

impl PcmWriter {
    fn push_with(&mut self, length: usize, sample: impl Fn(usize) -> f32) -> usize {
        let ring = &self.0;
        let head = ring.head.load(Ordering::Relaxed);
        let tail = ring.tail.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail).min(ring.frames);
        let frames = (length / ring.channels).min(ring.frames - used);
        for frame in 0..frames {
            let offset = head.wrapping_add(frame) % ring.frames * ring.channels;
            for channel in 0..ring.channels {
                let value = sample(frame * ring.channels + channel);
                let value = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                ring.samples[offset + channel].store(value.to_bits(), Ordering::Relaxed);
            }
        }
        let accepted = frames * ring.channels;
        ring.head
            .store(head.wrapping_add(frames), Ordering::Release);
        ring.dropped
            .fetch_add((length - accepted) as u64, Ordering::Relaxed);
        accepted
    }

    pub(crate) fn push(&mut self, samples: &[f32]) -> usize {
        self.push_with(samples.len(), |index| samples[index])
    }

    pub(crate) fn push_i16(&mut self, samples: &[i16]) -> usize {
        self.push_with(samples.len(), |index| f32::from(samples[index]) / 32768.0)
    }

    pub(crate) fn queued(&self) -> usize {
        let tail = self.0.tail.load(Ordering::Acquire);
        let head = self.0.head.load(Ordering::Acquire);
        head.wrapping_sub(tail).min(self.0.frames) * self.0.channels
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.0.dropped.load(Ordering::Relaxed)
    }
}

impl PcmReader {
    /// Linear streaming resampling, retaining a fractional source-frame cursor.
    pub(crate) fn next(&mut self, step: f64) -> [f32; 2] {
        let ring = &self.ring;
        let tail = ring.tail.load(Ordering::Relaxed);
        let head = ring.head.load(Ordering::Acquire);
        let available = head.wrapping_sub(tail).min(ring.frames);
        if available == 0 {
            self.phase = 0.0;
            return [0.0; 2];
        }
        let fraction = self.phase as f32;
        let mut output = [0.0; 2];
        for (channel, value) in output.iter_mut().enumerate() {
            let channel = channel.min(ring.channels - 1);
            let first = tail % ring.frames * ring.channels + channel;
            let first = f32::from_bits(ring.samples[first].load(Ordering::Relaxed));
            let second = if available > 1 {
                let offset = tail.wrapping_add(1) % ring.frames * ring.channels + channel;
                f32::from_bits(ring.samples[offset].load(Ordering::Relaxed))
            } else {
                0.0
            };
            *value = first * (1.0 - fraction) + second * fraction;
        }
        let advance = self.phase + step;
        let consumed = (advance.floor() as usize).min(available);
        self.phase = if consumed == available {
            0.0
        } else {
            advance - consumed as f64
        };
        ring.tail
            .store(tail.wrapping_add(consumed), Ordering::Release);
        output
    }

    /// Non-blocking mono microphone drain. Unwritten output elements are unchanged.
    pub(crate) fn read(&mut self, output: &mut [f32]) -> usize {
        let ring = &self.ring;
        let tail = ring.tail.load(Ordering::Relaxed);
        let head = ring.head.load(Ordering::Acquire);
        let count = head.wrapping_sub(tail).min(ring.frames).min(output.len());
        for (index, value) in output[..count].iter_mut().enumerate() {
            let offset = tail.wrapping_add(index) % ring.frames * ring.channels;
            *value = f32::from_bits(ring.samples[offset].load(Ordering::Relaxed));
        }
        ring.tail.store(tail.wrapping_add(count), Ordering::Release);
        count
    }
}

#[derive(Default)]
struct StreamBudget {
    owners: HashMap<usize, usize>,
    streams: usize,
    seconds: usize,
}

impl StreamBudget {
    fn reserve(
        &mut self,
        owner: usize,
        limits: NativeAudioStreamLimits,
    ) -> Result<(), NativeSoundError> {
        let owned = self.owners.get(&owner).copied().unwrap_or(0);
        if owned >= limits.streams_per_mod || self.streams >= limits.streams_global {
            return Err(NativeSoundError::StreamLimit);
        }
        if (owned + 1) * limits.seconds_per_stream > limits.buffered_seconds_per_mod
            || self.seconds + limits.seconds_per_stream > limits.buffered_seconds_global
        {
            return Err(NativeSoundError::BufferedSecondsLimit);
        }
        self.owners.insert(owner, owned + 1);
        self.streams += 1;
        self.seconds += limits.seconds_per_stream;
        Ok(())
    }
}

fn global_budget() -> &'static Mutex<StreamBudget> {
    static BUDGET: OnceLock<Mutex<StreamBudget>> = OnceLock::new();
    BUDGET.get_or_init(Mutex::default)
}

struct StreamLease {
    owner: usize,
    seconds: usize,
}

impl Drop for StreamLease {
    fn drop(&mut self) {
        let mut budget = global_budget()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(count) = budget.owners.get_mut(&self.owner) {
            *count -= 1;
            if *count == 0 {
                budget.owners.remove(&self.owner);
            }
        }
        budget.streams -= 1;
        budget.seconds -= self.seconds;
    }
}

struct StreamQueue {
    writer: PcmWriter,
    _lease: StreamLease,
}

pub(crate) struct StreamVoice {
    pub(crate) reader: PcmReader,
    pub(crate) sample_rate: u32,
    pub(crate) category: usize,
    pub(crate) gain: f32,
    pub(crate) position: Option<[f32; 3]>,
    pub(crate) relative: bool,
    pub(crate) seat: u8,
    pub(crate) world: u32,
}

pub(crate) struct NativeAudioStreams {
    session: u64,
    serial: u64,
    queues: HashMap<NativeAudioStreamId, StreamQueue>,
}

impl Default for NativeAudioStreams {
    fn default() -> Self {
        static SESSION: AtomicU64 = AtomicU64::new(1);
        Self {
            session: SESSION.fetch_add(1, Ordering::Relaxed),
            serial: 0,
            queues: HashMap::new(),
        }
    }
}

impl NativeAudioStreams {
    pub(crate) fn open(
        &mut self,
        owner: usize,
        sample_rate: u32,
        channels: u16,
        category: usize,
    ) -> Result<(NativeAudioStreamId, StreamVoice), NativeSoundError> {
        let limits = NativeAudioStreamLimits::default();
        if !(limits.minimum_sample_rate..=limits.maximum_sample_rate).contains(&sample_rate) {
            return Err(NativeSoundError::InvalidSampleRate);
        }
        if !matches!(channels, 1 | 2) {
            return Err(NativeSoundError::InvalidChannels);
        }
        global_budget()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reserve(owner, limits)?;
        let lease = StreamLease {
            owner,
            seconds: limits.seconds_per_stream,
        };
        let (writer, reader) = pcm_ring(
            sample_rate as usize * limits.seconds_per_stream,
            usize::from(channels),
        );
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or(NativeSoundError::ChannelExhausted)?;
        let id = NativeAudioStreamId {
            owner,
            session: self.session,
            serial: self.serial,
        };
        self.queues.insert(
            id,
            StreamQueue {
                writer,
                _lease: lease,
            },
        );
        Ok((
            id,
            StreamVoice {
                reader,
                sample_rate,
                category,
                gain: 0.0,
                position: None,
                relative: false,
                seat: 0,
                world: 0,
            },
        ))
    }

    fn queue(
        &mut self,
        owner: usize,
        id: NativeAudioStreamId,
    ) -> Result<&mut StreamQueue, NativeSoundError> {
        if id.owner != owner {
            return Err(NativeSoundError::ForeignHandle);
        }
        self.queues
            .get_mut(&id)
            .ok_or(NativeSoundError::StaleHandle)
    }

    pub(crate) fn push(
        &mut self,
        owner: usize,
        id: NativeAudioStreamId,
        samples: &[f32],
    ) -> Result<usize, NativeSoundError> {
        let queue = self.queue(owner, id)?;
        if !samples.len().is_multiple_of(queue.writer.0.channels) {
            return Err(NativeSoundError::IncompleteFrame);
        }
        Ok(queue.writer.push(samples))
    }

    pub(crate) fn push_i16(
        &mut self,
        owner: usize,
        id: NativeAudioStreamId,
        samples: &[i16],
    ) -> Result<usize, NativeSoundError> {
        let queue = self.queue(owner, id)?;
        if !samples.len().is_multiple_of(queue.writer.0.channels) {
            return Err(NativeSoundError::IncompleteFrame);
        }
        Ok(queue.writer.push_i16(samples))
    }

    pub(crate) fn queued(&mut self, owner: usize, id: NativeAudioStreamId) -> Option<usize> {
        self.queue(owner, id)
            .ok()
            .map(|queue| queue.writer.queued())
    }

    pub(crate) fn dropped(&mut self, owner: usize, id: NativeAudioStreamId) -> Option<u64> {
        self.queue(owner, id)
            .ok()
            .map(|queue| queue.writer.dropped())
    }

    pub(crate) fn check(
        &mut self,
        owner: usize,
        id: NativeAudioStreamId,
    ) -> Result<(), NativeSoundError> {
        self.queue(owner, id).map(|_| ())
    }

    pub(crate) fn close(&mut self, owner: usize, id: NativeAudioStreamId) -> bool {
        id.owner == owner && self.queues.remove(&id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_drops_only_complete_frames_and_wraps() {
        let (mut writer, mut reader) = pcm_ring(2, 2);
        assert_eq!(writer.push(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]), 4);
        assert_eq!(writer.queued(), 4);
        assert_eq!(writer.dropped(), 2);
        assert_eq!(reader.next(1.0), [0.1, 0.2]);
        assert_eq!(writer.push_i16(&[16384, -16384]), 2);
        assert_eq!(reader.next(1.0), [0.3, 0.4]);
        assert_eq!(reader.next(1.0), [0.5, -0.5]);
        assert_eq!(writer.queued(), 0);
    }

    #[test]
    fn underrun_is_silence_and_resets_resampling_phase() {
        let (mut writer, mut reader) = pcm_ring(4, 1);
        assert_eq!(reader.next(0.5), [0.0; 2]);
        assert_eq!(writer.push(&[1.0]), 1);
        assert_eq!(reader.next(0.5), [1.0; 2]);
        assert_eq!(reader.next(0.5), [0.5; 2]);
        assert_eq!(reader.next(0.5), [0.0; 2]);
        assert_eq!(writer.push(&[-0.5]), 1);
        assert_eq!(reader.next(1.0), [-0.5; 2]);
    }

    #[test]
    fn resampling_interpolates_and_retains_phase_across_calls() {
        let (mut writer, mut reader) = pcm_ring(8, 1);
        writer.push(&[0.0, 0.5, 1.0, 0.5, 0.0]);
        let actual: Vec<_> = (0..8).map(|_| reader.next(0.5)[0]).collect();
        assert_eq!(actual, [0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25]);
        assert_eq!(writer.queued(), 1);
        assert_eq!(reader.next(2.0), [0.0; 2]);
        assert_eq!(writer.queued(), 0);
    }

    #[test]
    fn input_is_finite_normalized_pcm() {
        let (mut writer, mut reader) = pcm_ring(4, 1);
        writer.push(&[f32::NAN, f32::INFINITY, f32::MAX, -f32::MAX]);
        assert_eq!(reader.next(1.0), [0.0; 2]);
        assert_eq!(reader.next(1.0), [0.0; 2]);
        assert_eq!(reader.next(1.0), [1.0; 2]);
        assert_eq!(reader.next(1.0), [-1.0; 2]);
    }

    #[test]
    fn stream_count_and_buffered_seconds_have_independent_limits() {
        let limits = NativeAudioStreamLimits {
            streams_per_mod: 2,
            streams_global: 3,
            buffered_seconds_per_mod: 2,
            buffered_seconds_global: 3,
            ..NativeAudioStreamLimits::default()
        };
        let mut budget = StreamBudget::default();
        budget.reserve(7, limits).unwrap();
        budget.reserve(7, limits).unwrap();
        assert_eq!(
            budget.reserve(7, limits),
            Err(NativeSoundError::StreamLimit)
        );
        budget.reserve(8, limits).unwrap();
        assert_eq!(
            budget.reserve(9, limits),
            Err(NativeSoundError::StreamLimit)
        );
        let mut budget = StreamBudget::default();
        let limits = NativeAudioStreamLimits {
            buffered_seconds_per_mod: 1,
            ..limits
        };
        budget.reserve(7, limits).unwrap();
        assert_eq!(
            budget.reserve(7, limits),
            Err(NativeSoundError::BufferedSecondsLimit)
        );
        let limits = NativeAudioStreamLimits {
            buffered_seconds_global: 1,
            ..limits
        };
        assert_eq!(
            budget.reserve(8, limits),
            Err(NativeSoundError::BufferedSecondsLimit)
        );
    }

    #[test]
    fn handles_are_owned_and_close_invalidates_them() {
        let mut streams = NativeAudioStreams::default();
        let (handle, _voice) = streams.open(71, 8_000, 2, 9).unwrap();
        assert_eq!(
            streams.push(72, handle, &[0.0, 0.0]),
            Err(NativeSoundError::ForeignHandle)
        );
        assert_eq!(
            streams.push(71, handle, &[0.0]),
            Err(NativeSoundError::IncompleteFrame)
        );
        assert!(streams.close(71, handle));
        assert_eq!(
            streams.push(71, handle, &[0.0, 0.0]),
            Err(NativeSoundError::StaleHandle)
        );
    }
}
