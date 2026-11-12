use super::{GpuChecks, Renderer, capture, hdr};
use std::sync::Arc;

fn valid_size(size: [u32; 2], maximum: u32) -> bool {
    size[0] > 0
        && size[1] > 0
        && size[0] <= 3840
        && size[1] <= 2160
        && size.iter().all(|side| *side <= maximum)
}

pub(super) struct Output {
    config: wgpu::SurfaceConfiguration,
    surface: Option<Arc<wgpu::Surface<'static>>>,
    hdr: Option<hdr::Hdr>,
    offscreen: Option<wgpu::Texture>,
    depth: wgpu::TextureView,
    scale: f32,
    suspended: bool,
    pub(super) capture: capture::CaptureHandle,
    pub(super) window_size: Option<[u32; 2]>,
}

impl Renderer {
    pub fn begin_native_fixed_output(&mut self, size: [u32; 2]) -> Result<(), String> {
        if self.native_fixed.is_some() || self.capture_time.is_some() {
            return Err("another fixed-frame output is active".into());
        }
        let limits = self.device.limits();
        if !valid_size(size, limits.max_texture_dimension_2d) {
            return Err("fixed-frame size must fit 3840x2160 and the GPU limits".into());
        }
        if !matches!(
            self.config.format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err("fixed frames require an 8-bit SDR output".into());
        }
        let mut config = self.config.clone();
        [config.width, config.height] = size;
        config.usage |= wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let checks = GpuChecks::push(&self.device);
        let target = Self::offscreen_target(&self.device, &config);
        let depth = Self::depth(&self.device, &config);
        if let Some(error) = checks.pop() {
            return Err(format!("cannot allocate fixed-frame output: {error}"));
        }
        let capture = capture::CaptureHandle::new(size);
        capture.begin()?;
        self.frame_gen.stop();
        self.queue.wait();
        self.native_fixed = Some(Output {
            config: std::mem::replace(&mut self.config, config),
            surface: self.surface.take(),
            hdr: self.hdr.take(),
            offscreen: self.offscreen.replace(target),
            depth: std::mem::replace(&mut self.depth, depth),
            scale: self.world_post.scale,
            suspended: self.suspended,
            capture,
            window_size: None,
        });
        self.world_post.scale = 1.0;
        self.suspended = false;
        Ok(())
    }

    pub fn request_native_fixed_pixels(&self, index: u64) -> Result<(), String> {
        self.native_fixed
            .as_ref()
            .ok_or("no fixed-frame output")?
            .capture
            .request(capture::CaptureToken {
                session: 1,
                generation: 1,
                index,
            })
    }

    pub fn read_native_fixed_pixels(&self, index: u64) -> Result<capture::CapturePixels, String> {
        let output = self.native_fixed.as_ref().ok_or("no fixed-frame output")?;
        let (token, readback) = output.capture.poll().ok_or("fixed frame was not submitted")?;
        if token.index != index {
            return Err("fixed-frame readback index changed".into());
        }
        readback?.read()
    }

    pub fn end_native_fixed_output(&mut self) {
        let Some(output) = self.native_fixed.take() else {
            return;
        };
        output.capture.abort();
        self.queue.wait();
        self.set_capture_presentation_time(None);
        self.config = output.config;
        self.surface = output.surface;
        self.hdr = output.hdr;
        self.offscreen = output.offscreen;
        self.depth = output.depth;
        self.world_post.scale = output.scale;
        self.suspended = output.suspended;
        if let Some([width, height]) = output.window_size {
            self.resize(width, height);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_output_validates_without_a_window_or_gpu() {
        assert!(valid_size([3840, 2160], 4096));
        assert!(valid_size([1, 1], 1));
        for size in [[0, 1], [1, 0], [3841, 2160], [3840, 2161]] {
            assert!(!valid_size(size, 8192));
        }
        assert!(!valid_size([3840, 2160], 2048));
    }
}
