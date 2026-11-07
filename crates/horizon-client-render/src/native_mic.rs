//! Opt-in, mod-owned microphone capture. Merely constructing the manager opens nothing.

use crate::native_audio_stream::{PcmReader, PcmWriter, pcm_ring};
use std::{
    collections::HashMap,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(feature = "audio")]
use {
    cpal::traits::{DeviceTrait, HostTrait, StreamTrait},
    std::sync::{Arc, atomic::AtomicBool},
};

/// Opaque capture handle. It cannot be read or closed by another mod.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeMicId {
    owner: usize,
    session: u64,
    serial: u64,
}

#[derive(Default)]
struct MicBudget {
    owners: HashMap<usize, usize>,
    count: usize,
}

impl MicBudget {
    fn reserve(&mut self, owner: usize) -> Result<(), String> {
        if self.owners.contains_key(&owner) || self.count >= 4 {
            return Err("Microphone limit reached (one per mod, four globally)".into());
        }
        self.owners.insert(owner, 1);
        self.count += 1;
        Ok(())
    }

    fn release(&mut self, owner: usize) {
        if self.owners.remove(&owner).is_some() {
            self.count -= 1;
        }
    }
}

fn global_budget() -> &'static Mutex<MicBudget> {
    static BUDGET: OnceLock<Mutex<MicBudget>> = OnceLock::new();
    BUDGET.get_or_init(Mutex::default)
}

struct MicLease(usize);

impl Drop for MicLease {
    fn drop(&mut self) {
        global_budget()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .release(self.0);
    }
}

struct CaptureConverter {
    writer: PcmWriter,
    previous: Option<f32>,
    frame: u64,
    next_output: f64,
    step: f64,
}

impl CaptureConverter {
    fn new(writer: PcmWriter, input_rate: u32, output_rate: u32) -> Self {
        Self {
            writer,
            previous: None,
            frame: 0,
            next_output: 0.0,
            step: f64::from(input_rate) / f64::from(output_rate),
        }
    }

    fn frame(&mut self, sample: f32) {
        let sample = if sample.is_finite() { sample } else { 0.0 };
        let position = self.frame as f64;
        if let Some(previous) = self.previous {
            while self.next_output <= position {
                let fraction = (self.next_output - (position - 1.0)) as f32;
                self.writer
                    .push(&[previous * (1.0 - fraction) + sample * fraction]);
                self.next_output += self.step;
            }
        } else {
            self.writer.push(&[sample]);
            self.next_output += self.step;
        }
        self.previous = Some(sample);
        self.frame += 1;
    }

    fn input(&mut self, length: usize, channels: usize, sample: impl Fn(usize) -> f32) {
        for frame in 0..length / channels {
            let mut mono = 0.0;
            for channel in 0..channels {
                let value = sample(frame * channels + channel);
                mono += if value.is_finite() { value } else { 0.0 };
            }
            self.frame(mono / channels as f32);
        }
    }
}

struct Microphone {
    #[cfg(feature = "audio")]
    _stream: cpal::Stream,
    reader: PcmReader,
    #[cfg(feature = "audio")]
    failed: Arc<AtomicBool>,
    _lease: MicLease,
}

/// Capture storage is allocated only by an explicit successful open call.
pub struct NativeMicrophones {
    session: u64,
    serial: u64,
    microphones: HashMap<NativeMicId, Microphone>,
}

impl Default for NativeMicrophones {
    fn default() -> Self {
        static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
        Self {
            session: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            serial: 0,
            microphones: HashMap::new(),
        }
    }
}

impl NativeMicrophones {
    /// Enumerating devices does not start capture or request microphone access.
    pub fn devices() -> Vec<String> {
        #[cfg(feature = "audio")]
        {
            let mut names = Vec::new();
            if let Ok(devices) = cpal::default_host().input_devices() {
                for device in devices {
                    if let Ok(name) = device.name()
                        && !names.contains(&name)
                    {
                        names.push(name);
                    }
                }
            }
            names
        }
        #[cfg(not(feature = "audio"))]
        Vec::new()
    }

    /// The calling mod must obtain the player's consent before calling this.
    pub fn open(
        &mut self,
        owner: usize,
        device: Option<&str>,
        sample_rate: u32,
    ) -> Result<NativeMicId, String> {
        if !(8_000..=192_000).contains(&sample_rate) {
            return Err("Microphone sample rate must be 8000..=192000 Hz".into());
        }
        #[cfg(feature = "audio")]
        {
            let serial = self
                .serial
                .checked_add(1)
                .ok_or("Microphone ID exhausted")?;
            let microphone = open_microphone(owner, device, sample_rate)?;
            let id = NativeMicId {
                owner,
                session: self.session,
                serial,
            };
            self.serial = serial;
            self.microphones.insert(id, microphone);
            Ok(id)
        }
        #[cfg(not(feature = "audio"))]
        {
            let _ = (owner, device);
            Err("Microphone capture is unavailable in this build".into())
        }
    }

    /// Returns immediately; the unused suffix of `output` is left unchanged.
    pub fn read(&mut self, owner: usize, id: NativeMicId, output: &mut [f32]) -> usize {
        if id.owner != owner {
            return 0;
        }
        let Some(microphone) = self.microphones.get_mut(&id) else {
            return 0;
        };
        #[cfg(feature = "audio")]
        if microphone.failed.load(Ordering::Relaxed) {
            return 0;
        }
        microphone.reader.read(output)
    }

    pub fn close(&mut self, owner: usize, id: NativeMicId) -> bool {
        id.owner == owner && self.microphones.remove(&id).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.microphones.is_empty()
    }

    pub fn retain_owners(&mut self, mut enabled: impl FnMut(usize) -> bool) {
        self.microphones.retain(|id, _| enabled(id.owner));
    }
}

#[cfg(feature = "audio")]
fn open_microphone(
    owner: usize,
    name: Option<&str>,
    sample_rate: u32,
) -> Result<Microphone, String> {
    global_budget()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .reserve(owner)?;
    let lease = MicLease(owner);
    let host = cpal::default_host();
    let device = match name {
        Some(name) => host
            .input_devices()
            .map_err(|error| error.to_string())?
            .find(|device| device.name().is_ok_and(|candidate| candidate == name))
            .ok_or_else(|| format!("Microphone not found: {name}"))?,
        None => host.default_input_device().ok_or("No default microphone")?,
    };
    let config = device
        .default_input_config()
        .map_err(|error| error.to_string())?;
    if config.channels() == 0 || config.sample_rate().0 == 0 {
        return Err("Microphone returned an invalid configuration".into());
    }
    let (writer, reader) = pcm_ring(sample_rate as usize, 1);
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match config.sample_format() {
        cpal::SampleFormat::I8 => build_input::<i8>(&device, &config, sample_rate, writer, &failed),
        cpal::SampleFormat::I16 => {
            build_input::<i16>(&device, &config, sample_rate, writer, &failed)
        }
        cpal::SampleFormat::I32 => {
            build_input::<i32>(&device, &config, sample_rate, writer, &failed)
        }
        cpal::SampleFormat::I64 => {
            build_input::<i64>(&device, &config, sample_rate, writer, &failed)
        }
        cpal::SampleFormat::U8 => build_input::<u8>(&device, &config, sample_rate, writer, &failed),
        cpal::SampleFormat::U16 => {
            build_input::<u16>(&device, &config, sample_rate, writer, &failed)
        }
        cpal::SampleFormat::U32 => {
            build_input::<u32>(&device, &config, sample_rate, writer, &failed)
        }
        cpal::SampleFormat::U64 => {
            build_input::<u64>(&device, &config, sample_rate, writer, &failed)
        }
        cpal::SampleFormat::F32 => {
            build_input::<f32>(&device, &config, sample_rate, writer, &failed)
        }
        cpal::SampleFormat::F64 => {
            build_input::<f64>(&device, &config, sample_rate, writer, &failed)
        }
        format => return Err(format!("Unsupported microphone sample format: {format:?}")),
    }?;
    stream.play().map_err(|error| error.to_string())?;
    Ok(Microphone {
        _stream: stream,
        reader,
        failed,
        _lease: lease,
    })
}

#[cfg(feature = "audio")]
fn build_input<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    sample_rate: u32,
    writer: PcmWriter,
    failed: &Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    use cpal::Sample;

    let channels = usize::from(config.channels());
    let mut converter = CaptureConverter::new(writer, config.sample_rate().0, sample_rate);
    let failed = Arc::clone(failed);
    device
        .build_input_stream(
            &config.config(),
            move |data: &[T], _| {
                converter.input(data.len(), channels, |index| f32::from_sample(data[index]));
            },
            move |_| failed.store(true, Ordering::Relaxed),
            None,
        )
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_downmixes_resamples_and_never_fills_the_unused_suffix() {
        let (writer, mut reader) = pcm_ring(8, 1);
        let mut converter = CaptureConverter::new(writer, 48_000, 24_000);
        let input = [0.0, 0.0, 0.2, 0.6, 0.4, 0.8, 0.6, 1.0, 0.8, 0.8];
        converter.input(input.len(), 2, |index| input[index]);
        let mut output = [9.0; 8];
        assert_eq!(reader.read(&mut output), 3);
        assert_eq!(&output[..3], &[0.0, 0.6, 0.8]);
        assert_eq!(&output[3..], &[9.0; 5]);
        assert_eq!(reader.read(&mut output), 0);
    }

    #[test]
    fn capture_interpolates_across_callbacks_and_drops_overflow() {
        let (writer, mut reader) = pcm_ring(3, 1);
        let mut converter = CaptureConverter::new(writer, 24_000, 48_000);
        converter.frame(0.0);
        converter.frame(1.0);
        converter.frame(0.0);
        assert_eq!(converter.writer.queued(), 3);
        assert_eq!(converter.writer.dropped(), 2);
        let mut output = [0.0; 3];
        assert_eq!(reader.read(&mut output), 3);
        assert_eq!(output, [0.0, 0.5, 1.0]);
    }

    #[test]
    fn microphone_budget_is_bounded_and_reusable() {
        let mut budget = MicBudget::default();
        for owner in 0..4 {
            assert!(budget.reserve(owner).is_ok());
        }
        assert!(budget.reserve(0).is_err());
        assert!(budget.reserve(4).is_err());
        budget.release(1);
        assert!(budget.reserve(4).is_ok());
        assert!(!budget.owners.contains_key(&1));
    }

    #[test]
    fn invalid_and_foreign_microphones_read_nothing() {
        let mut microphones = NativeMicrophones::default();
        let id = NativeMicId {
            owner: 1,
            session: microphones.session,
            serial: 1,
        };
        let mut output = [7.0; 4];
        assert_eq!(microphones.read(2, id, &mut output), 0);
        assert_eq!(microphones.read(1, id, &mut output), 0);
        assert!(!microphones.close(2, id));
        assert_eq!(output, [7.0; 4]);
    }
}
