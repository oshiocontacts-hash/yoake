use std::{
    sync::{Arc, Mutex, MutexGuard, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureToken {
    pub session: u64,
    pub generation: u64,
    pub index: u64,
}

pub struct CapturePixels {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}

pub(super) fn rebase(time: &mut Instant, from: Instant, to: Instant) {
    *time = to
        .checked_sub(from.saturating_duration_since(*time))
        .unwrap_or(to);
}

pub struct CaptureReadback {
    token: CaptureToken,
    epoch: u64,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    device: wgpu::Device,
    buffer: wgpu::Buffer,
}

impl CaptureReadback {
    pub fn read(self) -> Result<CapturePixels, String> {
        let (sender, receiver) = mpsc::channel();
        self.buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = sender.send(result);
            });
        // CHECK: Exercise device loss during a worker-owned wgpu 30 mapping.
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(5)),
            })
            .map_err(|error| error.to_string())?;
        receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let result = (|| {
            let bytes = self
                .buffer
                .get_mapped_range(..)
                .map_err(|error| error.to_string())?;
            let [width, height] = self.size;
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for row in bytes.chunks_exact(super::padded_row(width) as usize) {
                rgba.extend_from_slice(&row[..width as usize * 4]);
            }
            let bgra = matches!(
                self.format,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            );
            for pixel in rgba.chunks_exact_mut(4) {
                if bgra {
                    pixel.swap(0, 2);
                }
                pixel[3] = 255;
            }
            Ok(CapturePixels {
                size: self.size,
                rgba,
            })
        })();
        self.buffer.unmap();
        result
    }
}

struct State {
    size: [u32; 2],
    fixed_size: Option<[u32; 2]>,
    epoch: u64,
    token: Option<CaptureToken>,
    armed: bool,
    ready: Option<Result<CaptureReadback, String>>,
    failure: Option<String>,
}

#[derive(Clone)]
pub struct CaptureHandle(Arc<Mutex<State>>);

impl CaptureHandle {
    pub(super) fn new(size: [u32; 2]) -> Self {
        Self(Arc::new(Mutex::new(State {
            size,
            fixed_size: None,
            epoch: 0,
            token: None,
            armed: false,
            ready: None,
            failure: None,
        })))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn begin(&self) -> Result<[u32; 2], String> {
        let mut state = self.state();
        if state.fixed_size.is_some() {
            return Err("capture output is already active".into());
        }
        if state.size.contains(&0) {
            return Err("capture requires a nonzero drawable size".into());
        }
        state.epoch = state.epoch.wrapping_add(1);
        state.fixed_size = Some(state.size);
        state.failure = None;
        Ok(state.size)
    }

    pub fn request(&self, token: CaptureToken) -> Result<(), String> {
        let mut state = self.state();
        if let Some(reason) = &state.failure {
            return Err(reason.clone());
        }
        if state.fixed_size != Some(state.size) {
            return Err("capture output is inactive or resized".into());
        }
        if state.token.is_some() {
            return Err("capture readback is still outstanding".into());
        }
        state.token = Some(token);
        state.armed = true;
        Ok(())
    }

    pub fn poll(&self) -> Option<(CaptureToken, Result<CaptureReadback, String>)> {
        let mut state = self.state();
        let result = state.ready.take()?;
        Some((state.token.take()?, result))
    }

    pub fn check(&self) -> Result<(), String> {
        self.state().failure.clone().map_or(Ok(()), Err)
    }

    pub fn abort(&self) {
        let mut state = self.state();
        state.epoch = state.epoch.wrapping_add(1);
        state.fixed_size = None;
        state.token = None;
        state.armed = false;
        state.ready = None;
        state.failure = None;
    }

    pub(super) fn resize(&self, size: [u32; 2]) {
        let mut state = self.state();
        state.size = size;
        if state.fixed_size.is_some_and(|fixed| fixed != size) {
            let reason = "drawable resized during capture".to_string();
            state.failure = Some(reason.clone());
            state.armed = false;
            if state.token.is_some() {
                state.ready = Some(Err(reason));
            }
        }
    }

    #[cfg(test)]
    fn frame_index(&self) -> Option<u64> {
        self.state().token.map(|token| token.index)
    }

    pub(super) fn pending(&self) -> bool {
        self.state().armed
    }

    pub(super) fn copy(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Option<CaptureReadback> {
        let mut state = self.state();
        if !state.armed {
            return None;
        }
        state.armed = false;
        let token = state.token?;
        let size = [texture.width(), texture.height()];
        let format = texture.format();
        let sdr = matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        if state.fixed_size != Some(size) || !sdr {
            let reason = "capture requires the session-sized 8-bit SDR final image".to_string();
            state.failure = Some(reason.clone());
            state.ready = Some(Err(reason));
            return None;
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Horizon capture readback"),
            size: u64::from(super::padded_row(size[0])) * u64::from(size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(super::padded_row(size[0])),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        Some(CaptureReadback {
            token,
            epoch: state.epoch,
            size,
            format,
            device: device.clone(),
            buffer,
        })
    }

    pub(super) fn submitted(&self, readback: CaptureReadback) {
        let mut state = self.state();
        if state.epoch == readback.epoch
            && state.token == Some(readback.token)
            && state.failure.is_none()
        {
            state.ready = Some(Ok(readback));
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct PresentationTime {
    origin: Duration,
    current: Duration,
    anchor: Instant,
    delta: Duration,
    frame: u32,
}

impl PresentationTime {
    pub(super) fn set(slot: &mut Option<Self>, time: Option<Duration>) {
        match (slot.as_mut(), time) {
            (Some(clock), Some(time)) => {
                if time != clock.current {
                    clock.delta = time.saturating_sub(clock.current);
                    clock.current = time;
                    clock.frame = clock.frame.wrapping_add(1);
                }
            }
            (None, Some(time)) => {
                *slot = Some(Self {
                    origin: time,
                    current: time,
                    anchor: Instant::now(),
                    delta: Duration::ZERO,
                    frame: 0,
                });
            }
            (_, None) => *slot = None,
        }
    }

    pub(super) fn now(self) -> Instant {
        self.anchor + self.current.saturating_sub(self.origin)
    }

    pub(super) fn elapsed(self) -> Duration {
        self.current
    }

    pub(super) fn delta(self) -> Duration {
        self.delta
    }

    pub(super) fn frame(self) -> u32 {
        self.frame.saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(generation: u64) -> CaptureToken {
        CaptureToken {
            session: generation,
            generation,
            index: 1,
        }
    }

    #[test]
    fn resize_is_latched_even_if_original_size_returns() {
        let handle = CaptureHandle::new([16, 16]);
        handle.begin().unwrap();
        handle.request(token(1)).unwrap();
        handle.resize([32, 16]);
        handle.resize([16, 16]);
        assert!(handle.poll().unwrap().1.is_err());
        assert!(handle.check().is_err());
    }

    #[test]
    fn abort_clears_old_generation() {
        let handle = CaptureHandle::new([16, 16]);
        handle.begin().unwrap();
        handle.request(token(1)).unwrap();
        handle.resize([32, 16]);
        handle.abort();
        handle.begin().unwrap();
        handle.request(token(2)).unwrap();
        assert!(handle.poll().is_none());
        assert_eq!(handle.frame_index(), Some(1));
    }

    #[test]
    fn presentation_advances_only_when_set() {
        let mut clock = None;
        PresentationTime::set(&mut clock, Some(Duration::from_secs(4)));
        let first = clock.unwrap().now();
        PresentationTime::set(&mut clock, Some(Duration::from_millis(4050)));
        assert_eq!(clock.unwrap().now() - first, Duration::from_millis(50));
        PresentationTime::set(&mut clock, None);
        assert!(clock.is_none());
    }
}
