//! Mod-owned GUI textures. Register before preparing the HUD batch, redraw in
//! `WorldPrePost`, then sample in the HUD or a mod screen. API @API:gui_textures@.

use super::{Blend, HudBatch, Rect};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

const MAX_REGISTRATIONS: usize = 64;
const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_UPLOAD_BYTES: usize = 16 * 1024 * 1024;
const MAX_RETAINED_RUNS: usize = 256;
const RUN_POOL_SIZE: usize = 4;
const BUNDLE_CACHE_SIZE: usize = 256;
pub(super) const MAX_CACHED_DRAWS: usize = 256;
static DEVICE_EPOCH: AtomicU64 = AtomicU64::new(0);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_REGISTRATION: AtomicU64 = AtomicU64::new(1);
static REGISTRATIONS: OnceLock<Mutex<Vec<Weak<Registration>>>> = OnceLock::new();
static RUN_POOL: Mutex<RunPool<NativeTextureRun>> = Mutex::new(RunPool::new());
static BLEND_POOL: Mutex<RunPool<(Range<u32>, Blend)>> = Mutex::new(RunPool::new());

#[cfg(test)]
thread_local! {
    static RUN_POOL_TAKES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BLEND_POOL_TAKES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct RunPool<Run> {
    slots: [Option<Vec<Run>>; RUN_POOL_SIZE],
}

impl<Run> RunPool<Run> {
    const fn new() -> Self {
        Self {
            slots: [const { None }; RUN_POOL_SIZE],
        }
    }

    fn take(&mut self) -> Vec<Run> {
        self.slots
            .iter_mut()
            .find_map(Option::take)
            .unwrap_or_default()
    }

    fn recycle(&mut self, runs: Vec<Run>) {
        if runs.capacity() != 0
            && runs.capacity() <= MAX_RETAINED_RUNS
            && let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none())
        {
            *slot = Some(runs);
        }
    }
}

fn take_runs() -> Vec<NativeTextureRun> {
    #[cfg(test)]
    RUN_POOL_TAKES.with(|takes| takes.set(takes.get() + 1));
    RUN_POOL
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
}

pub(super) fn recycle_runs(mut runs: Vec<NativeTextureRun>) {
    runs.clear();
    if runs.capacity() == 0 || runs.capacity() > MAX_RETAINED_RUNS {
        return;
    }
    RUN_POOL
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .recycle(runs);
}

fn take_blends() -> Vec<(Range<u32>, Blend)> {
    #[cfg(test)]
    BLEND_POOL_TAKES.with(|takes| takes.set(takes.get() + 1));
    BLEND_POOL
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
}

fn recycle_blends(mut blends: Vec<(Range<u32>, Blend)>) {
    blends.clear();
    if blends.capacity() == 0 || blends.capacity() > MAX_RETAINED_RUNS {
        return;
    }
    BLEND_POOL
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .recycle(blends);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DrawKey {
    pub(super) vertices: [u32; 2],
    pub(super) registration: u64,
    pub(super) blend: Blend,
    pub(super) text: bool,
}

impl DrawKey {
    pub(super) const EMPTY: Self = Self {
        vertices: [0; 2],
        registration: 0,
        blend: Blend::Alpha,
        text: false,
    };
}

#[derive(Clone, Debug)]
pub(super) struct CachedBundle {
    pub(super) bundle: wgpu::RenderBundle,
    pub(super) groups: usize,
}

#[derive(Debug)]
struct BundleEntry {
    commands: Box<[DrawKey]>,
    depth: bool,
    cached: CachedBundle,
}

#[derive(Debug)]
struct BundleCache {
    entries: [Option<BundleEntry>; BUNDLE_CACHE_SIZE],
    next: usize,
}

impl BundleCache {
    const fn new() -> Self {
        Self {
            entries: [const { None }; BUNDLE_CACHE_SIZE],
            next: 0,
        }
    }

    fn clear(&mut self) {
        self.entries.fill_with(|| None);
        self.next = 0;
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct DrawCounts {
    pub(super) passes: AtomicU64,
    pub(super) groups: AtomicU64,
}

pub(crate) fn device_epoch() -> u64 {
    DEVICE_EPOCH.load(Ordering::Acquire)
}

pub(crate) fn device_lost() {
    DEVICE_EPOCH.fetch_add(1, Ordering::AcqRel);
    if let Some(registrations) = REGISTRATIONS.get() {
        let registrations = registrations
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for registration in registrations.iter().filter_map(Weak::upgrade) {
            registration.generation.clear_bundles();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeGuiTextureFilter {
    Nearest,
    Linear,
}

/// Source dimensions and left, top, right, bottom borders in source pixels / GUI units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeGuiNineSlice {
    pub source_size: [u32; 2],
    pub border: [u32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeGuiTextureError {
    RendererUnavailable,
    InvalidDimensions,
    UnsupportedTexture,
    UnsupportedFormat,
    MissingSampledUsage,
    MissingUploadUsage,
    InvalidUpload,
    UploadLimit,
    StaleGeneration,
    RegistrationLimit,
    MemoryLimit,
}

impl std::fmt::Display for NativeGuiTextureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "native GUI texture: {self:?}")
    }
}

impl std::error::Error for NativeGuiTextureError {}

#[derive(Debug)]
struct Generation {
    id: u64,
    epoch: u64,
    valid: AtomicBool,
    bundles: Mutex<BundleCache>,
}

impl Generation {
    fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire) && self.epoch == device_epoch()
    }

    fn clear_bundles(&self) {
        self.bundles
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
    }
}

#[derive(Debug)]
struct Registration {
    id: u64,
    generation: Arc<Generation>,
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
    bytes: u64,
    filter: NativeGuiTextureFilter,
    pixels: Mutex<Option<Vec<u8>>>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.generation.clear_bundles();
    }
}

/// Opaque registration. Cloning retains the texture and cached bind group, not
/// another slot. Resource reload and device loss invalidate every old handle.
#[derive(Clone, Debug)]
pub struct NativeGuiTexture(Arc<Registration>);

impl NativeGuiTexture {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.0.texture
    }

    pub fn generation(&self) -> u64 {
        self.0.generation.id
    }

    pub fn is_valid(&self) -> bool {
        self.0.generation.is_valid()
    }

    pub(super) fn group(&self) -> &wgpu::BindGroup {
        &self.0.group
    }

    pub fn registration(&self) -> u64 {
        self.0.id
    }

    pub(super) fn filter(&self) -> NativeGuiTextureFilter {
        self.0.filter
    }

    pub(super) fn restore_source(&self) -> Result<Source<'_>, NativeGuiTextureError> {
        if self.0.generation.epoch != device_epoch()
            && self
                .0
                .pixels
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_none()
        {
            return Err(NativeGuiTextureError::StaleGeneration);
        }
        Ok(Source::Restore(self))
    }

    pub(super) fn needs_restore_upload(&self) -> bool {
        self.0.generation.epoch != device_epoch()
    }

    pub(super) fn restore_upload(&self, queue: &wgpu::Queue) {
        let pixels = self
            .0
            .pixels
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(pixels) = pixels.as_ref() {
            let row_bytes = pixels.len() / self.texture().height() as usize;
            let rows = (MAX_UPLOAD_BYTES as usize / row_bytes).max(1);
            for (index, chunk) in pixels.chunks(row_bytes * rows).enumerate() {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: (index * rows) as u32,
                            z: 0,
                        },
                        ..self.texture().as_image_copy()
                    },
                    chunk,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes as u32),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: self.texture().width(),
                        height: (chunk.len() / row_bytes) as u32,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}

/// Sparse texture selections; gaps select the ordinary HUD atlas.
#[derive(Clone, Debug)]
pub struct NativeTextureRun {
    pub(super) range: Range<u32>,
    pub(super) texture: NativeGuiTexture,
}

pub(super) struct TextureParts<'a> {
    runs: &'a [NativeTextureRun],
    range: Range<u32>,
}

impl<'a> TextureParts<'a> {
    pub(super) fn new(runs: &'a [NativeTextureRun], range: Range<u32>) -> Self {
        Self { runs, range }
    }
}

impl<'a> Iterator for TextureParts<'a> {
    type Item = (Range<u32>, Option<&'a NativeGuiTexture>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.range.is_empty() {
            return None;
        }
        while let Some(run) = self.runs.first() {
            if run.range.end <= self.range.start {
                self.runs = &self.runs[1..];
                continue;
            }
            let start = self.range.start;
            let (end, texture) = if run.range.start > start {
                (run.range.start.min(self.range.end), None)
            } else {
                (run.range.end.min(self.range.end), Some(&run.texture))
            };
            self.range.start = end;
            return Some((start..end, texture));
        }
        Some((std::mem::replace(&mut self.range, 0..0), None))
    }
}

fn upload_layout(
    dimensions: [u32; 2],
    origin: [u32; 2],
    size: [u32; 2],
    format: wgpu::TextureFormat,
    length: usize,
) -> Result<u32, NativeGuiTextureError> {
    let bytes_per_pixel = match format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 4,
        wgpu::TextureFormat::Rgba16Float => 8,
        _ => return Err(NativeGuiTextureError::UnsupportedFormat),
    };
    if size.contains(&0)
        || origin
            .iter()
            .zip(size)
            .zip(dimensions)
            .any(|((&offset, extent), bound)| {
                offset.checked_add(extent).is_none_or(|end| end > bound)
            })
    {
        return Err(NativeGuiTextureError::InvalidUpload);
    }
    let row_bytes = size[0]
        .checked_mul(bytes_per_pixel)
        .ok_or(NativeGuiTextureError::InvalidUpload)?;
    if u64::from(row_bytes) * u64::from(size[1]) != length as u64 {
        return Err(NativeGuiTextureError::InvalidUpload);
    }
    if length > MAX_UPLOAD_BYTES {
        return Err(NativeGuiTextureError::UploadLimit);
    }
    Ok(row_bytes)
}

fn check_limits(
    size: [u32; 2],
    max_side: u32,
    registrations: usize,
    retained_bytes: u64,
    bytes: u64,
) -> Result<(), NativeGuiTextureError> {
    if size
        .iter()
        .any(|&side| side == 0 || side > max_side.min(4096))
    {
        return Err(NativeGuiTextureError::InvalidDimensions);
    }
    if registrations >= MAX_REGISTRATIONS {
        return Err(NativeGuiTextureError::RegistrationLimit);
    }
    if retained_bytes.saturating_add(bytes) > MAX_BYTES {
        return Err(NativeGuiTextureError::MemoryLimit);
    }
    Ok(())
}

pub(super) enum Source<'a> {
    Register(&'a wgpu::Texture),
    Create([u32; 2], wgpu::TextureFormat),
    Restore(&'a NativeGuiTexture),
}

pub(super) struct NativeGuiTextures {
    generation: Arc<Generation>,
    device: wgpu::Device,
}

impl NativeGuiTextures {
    pub(super) fn new(epoch: u64, device: &wgpu::Device) -> Self {
        Self {
            generation: Arc::new(Generation {
                id: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
                epoch,
                valid: AtomicBool::new(true),
                bundles: Mutex::new(BundleCache::new()),
            }),
            device: device.clone(),
        }
    }

    pub(super) fn owns(&self, texture: &NativeGuiTexture) -> bool {
        texture.is_valid() && Arc::ptr_eq(&self.generation, &texture.0.generation)
    }

    pub(super) fn write(
        &self,
        queue: &wgpu::Queue,
        texture: &NativeGuiTexture,
        origin: [u32; 2],
        size: [u32; 2],
        bytes: &[u8],
    ) -> Result<(), NativeGuiTextureError> {
        if !self.owns(texture) {
            return Err(NativeGuiTextureError::StaleGeneration);
        }
        let target = texture.texture();
        if !target.usage().contains(wgpu::TextureUsages::COPY_DST) {
            return Err(NativeGuiTextureError::MissingUploadUsage);
        }
        let row_bytes = upload_layout(
            [target.width(), target.height()],
            origin,
            size,
            target.format(),
            bytes.len(),
        )?;
        if let Some(pixels) = texture
            .0
            .pixels
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_mut()
        {
            let pixel_bytes = row_bytes as usize / size[0] as usize;
            let target_row = target.width() as usize * pixel_bytes;
            for (row, source) in bytes.chunks_exact(row_bytes as usize).enumerate() {
                let offset =
                    (origin[1] as usize + row) * target_row + origin[0] as usize * pixel_bytes;
                pixels[offset..offset + source.len()].copy_from_slice(source);
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                origin: wgpu::Origin3d {
                    x: origin[0],
                    y: origin[1],
                    z: 0,
                },
                ..target.as_image_copy()
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    pub(super) fn clear_bundles(&self) {
        self.generation.clear_bundles();
    }

    pub(super) fn bundle(
        &self,
        commands: &[DrawKey],
        depth: bool,
        create: impl FnOnce(&wgpu::Device) -> CachedBundle,
    ) -> Option<CachedBundle> {
        if commands.is_empty() || commands.len() > MAX_CACHED_DRAWS {
            return None;
        }
        let mut bundles = self
            .generation
            .bundles
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !self.generation.is_valid() {
            return None;
        }
        if let Some(entry) = bundles
            .entries
            .iter()
            .flatten()
            .find(|entry| entry.depth == depth && entry.commands.as_ref() == commands)
        {
            return Some(entry.cached.clone());
        }
        let cached = create(&self.device);
        let next = bundles.next;
        bundles.entries[next] = Some(BundleEntry {
            commands: commands.into(),
            depth,
            cached: cached.clone(),
        });
        bundles.next = (next + 1) % BUNDLE_CACHE_SIZE;
        Some(cached)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn insert(
        &self,
        device: &wgpu::Device,
        source: Source<'_>,
        filter: NativeGuiTextureFilter,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        glint_view: &wgpu::TextureView,
        glint_sampler: &wgpu::Sampler,
        nearest: &wgpu::Sampler,
        linear: &wgpu::Sampler,
    ) -> Result<NativeGuiTexture, NativeGuiTextureError> {
        if !self.generation.is_valid() {
            return Err(NativeGuiTextureError::StaleGeneration);
        }
        let (size, format, mip_levels) = match &source {
            Source::Register(texture) => {
                if texture.dimension() != wgpu::TextureDimension::D2
                    || texture.depth_or_array_layers() != 1
                    || texture.sample_count() != 1
                {
                    return Err(NativeGuiTextureError::UnsupportedTexture);
                }
                if !texture
                    .usage()
                    .contains(wgpu::TextureUsages::TEXTURE_BINDING)
                {
                    return Err(NativeGuiTextureError::MissingSampledUsage);
                }
                (
                    [texture.width(), texture.height()],
                    texture.format(),
                    texture.mip_level_count(),
                )
            }
            Source::Create(size, format) => (*size, *format, 1),
            Source::Restore(previous) => (
                [previous.texture().width(), previous.texture().height()],
                previous.texture().format(),
                previous.texture().mip_level_count(),
            ),
        };
        let max_side = device.limits().max_texture_dimension_2d;
        check_limits(size, max_side, 0, 0, 0)?;
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 4,
            wgpu::TextureFormat::Rgba16Float => 8,
            _ => return Err(NativeGuiTextureError::UnsupportedFormat),
        };
        if !format
            .guaranteed_format_features(device.features())
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        {
            return Err(NativeGuiTextureError::UnsupportedFormat);
        }
        let bytes = (0..mip_levels)
            .map(|level| {
                u64::from(size[0].checked_shr(level).unwrap_or(0).max(1))
                    * u64::from(size[1].checked_shr(level).unwrap_or(0).max(1))
                    * bytes_per_pixel
            })
            .sum();
        let mut registrations = REGISTRATIONS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registrations.retain(|registration| registration.strong_count() != 0);
        let previous_id = match &source {
            Source::Restore(previous) => Some(previous.registration()),
            _ => None,
        };
        let retained: BTreeMap<_, _> = registrations
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|registration| Some(registration.id) != previous_id)
            .map(|registration| (registration.id, registration.bytes))
            .collect();
        check_limits(
            size,
            max_side,
            retained.len(),
            retained.values().sum(),
            bytes,
        )?;
        let pixels = match &source {
            Source::Register(_) => None,
            Source::Create(_, _) => Some(vec![0; bytes as usize]),
            Source::Restore(previous) => previous
                .0
                .pixels
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone(),
        };
        // CHECK: external textures must use this device; wgpu exposes no device identity query.
        let texture = match source {
            Source::Register(texture) => texture.clone(),
            Source::Restore(previous) if previous.0.generation.epoch == device_epoch() => {
                previous.texture().clone()
            }
            Source::Create(_, _) | Source::Restore(_) => {
                let [width, height] = size;
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Native GUI texture"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            }
        };
        let view = texture.create_view(&Default::default());
        let sampler = match filter {
            NativeGuiTextureFilter::Nearest => nearest,
            NativeGuiTextureFilter::Linear => linear,
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Native GUI texture"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(glint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(glint_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(linear),
                },
            ],
        });
        let registration = Arc::new(Registration {
            id: previous_id.unwrap_or_else(|| NEXT_REGISTRATION.fetch_add(1, Ordering::Relaxed)),
            generation: self.generation.clone(),
            texture,
            group,
            bytes,
            filter,
            pixels: Mutex::new(pixels),
        });
        registrations.push(Arc::downgrade(&registration));
        Ok(NativeGuiTexture(registration))
    }
}

impl Drop for NativeGuiTextures {
    fn drop(&mut self) {
        self.generation.valid.store(false, Ordering::Release);
        self.generation.clear_bundles();
    }
}

impl Drop for HudBatch {
    fn drop(&mut self) {
        if let Some(runs) = self.native_textures.take() {
            recycle_runs(runs);
        }
        if self.native_texture_storage {
            recycle_blends(std::mem::take(&mut self.blends));
        }
    }
}

impl HudBatch {
    /// Append without reordering translucent commands. Invalid geometry or stale
    /// handles return false without changing the batch. ARGB uses straight alpha.
    pub fn native_gui_texture(
        &mut self,
        texture: &NativeGuiTexture,
        rect: [f32; 4],
        uv: [f32; 4],
        argb: u32,
        nine_slice: Option<NativeGuiNineSlice>,
    ) -> bool {
        if !texture.is_valid()
            || !rect.iter().chain(uv.iter()).all(|value| value.is_finite())
            || uv.iter().any(|value| !(0.0..=1.0).contains(value))
            || !(rect[0] + rect[2]).is_finite()
            || !(rect[1] + rect[3]).is_finite()
            || rect[2] <= 0.0
            || rect[3] <= 0.0
            || nine_slice.is_some_and(|slice| {
                slice.source_size.contains(&0)
                    || slice.border[0]
                        .checked_add(slice.border[2])
                        .is_none_or(|sum| sum > slice.source_size[0])
                    || slice.border[1]
                        .checked_add(slice.border[3])
                        .is_none_or(|sum| sum > slice.source_size[1])
            })
        {
            return false;
        }
        let start = self.vertices.len() as u32;
        let sprite = Rect {
            u0: uv[0],
            v0: uv[1],
            u1: uv[2],
            v1: uv[3],
            w: texture.texture().width(),
            h: texture.texture().height(),
        };
        let color = (argb & 0xff00_ff00) | ((argb & 0xff) << 16) | ((argb >> 16) & 0xff);
        if let Some(slice) = nine_slice {
            crate::ui::draw_native_nine_slice(self, sprite, slice, rect, color);
        } else {
            self.rect_uv(rect[0], rect[1], rect[2], rect[3], sprite, color);
        }
        let end = self.vertices.len() as u32;
        if start == end {
            return true;
        }
        if !self.native_texture_storage {
            let mut blends = take_blends();
            if blends.capacity() > self.blends.capacity() {
                blends.append(&mut self.blends);
                self.blends = blends;
            }
            self.native_texture_storage = true;
        }
        self.blend_since(start as usize, Blend::Alpha);
        let runs = self.native_textures.get_or_insert_with(take_runs);
        if let Some(last) = runs.last_mut()
            && last.range.end == start
            && Arc::ptr_eq(&last.texture.0, &texture.0)
        {
            last.range.end = end;
        } else {
            runs.push(NativeTextureRun {
                range: start..end,
                texture: texture.clone(),
            });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Blend, HudRenderer, rgba};
    use super::*;

    #[test]
    fn run_pool_reuses_storage_and_bounds_retention() {
        let mut pool = RunPool::<NativeTextureRun>::new();
        assert_eq!(pool.take().capacity(), 0);
        pool.recycle(Vec::new());
        assert!(pool.slots.iter().all(Option::is_none));
        pool.recycle(Vec::with_capacity(MAX_RETAINED_RUNS + 1));
        assert!(pool.slots.iter().all(Option::is_none));

        let runs = Vec::with_capacity(8);
        let pointer = runs.as_ptr();
        let capacity = runs.capacity();
        pool.recycle(runs);
        let runs = pool.take();
        assert!(runs.is_empty());
        assert_eq!(runs.as_ptr(), pointer);
        assert_eq!(runs.capacity(), capacity);
        assert!(pool.slots.iter().all(Option::is_none));

        for _ in 0..RUN_POOL_SIZE + 1 {
            pool.recycle(Vec::with_capacity(MAX_RETAINED_RUNS));
        }
        assert_eq!(pool.slots.iter().flatten().count(), RUN_POOL_SIZE);
        assert!(
            pool.slots
                .iter()
                .flatten()
                .all(|runs| runs.is_empty() && runs.capacity() <= MAX_RETAINED_RUNS)
        );
    }

    #[test]
    fn atlas_only_batches_never_take_run_storage() {
        let takes = RUN_POOL_TAKES.with(std::cell::Cell::get);
        let blend_takes = BLEND_POOL_TAKES.with(std::cell::Cell::get);
        drop(HudBatch::new([16, 16]));
        let mut batch = HudBatch::new([16, 16]);
        atlas_rect(&mut batch, [0.0, 0.0, 8.0, 8.0], u32::MAX);
        batch.clip_since(0, 1.0, 1.0, 7.0, 7.0);
        assert!(batch.native_textures.is_none());
        drop(batch);
        assert_eq!(RUN_POOL_TAKES.with(std::cell::Cell::get), takes);
        assert_eq!(BLEND_POOL_TAKES.with(std::cell::Cell::get), blend_takes);
    }

    fn live_registrations() -> usize {
        REGISTRATIONS.get().map_or(0, |registrations| {
            registrations
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .iter()
                .filter(|registration| registration.strong_count() != 0)
                .count()
        })
    }

    #[test]
    fn dimensions_slots_and_retained_memory_are_bounded() {
        for size in [[0, 1], [1, 0], [4097, 1], [1, 4097]] {
            assert_eq!(
                check_limits(size, 8192, 0, 0, 4),
                Err(NativeGuiTextureError::InvalidDimensions)
            );
        }
        assert_eq!(check_limits([4096, 4096], 8192, 63, 0, MAX_BYTES), Ok(()));
        assert_eq!(
            check_limits([1025, 1], 1024, 0, 0, 4),
            Err(NativeGuiTextureError::InvalidDimensions)
        );
        assert_eq!(
            check_limits([1, 1], 4096, MAX_REGISTRATIONS, 0, 4),
            Err(NativeGuiTextureError::RegistrationLimit)
        );
        assert_eq!(check_limits([1, 1], 4096, 0, MAX_BYTES - 4, 4), Ok(()));
        for (retained, bytes) in [(MAX_BYTES, 1), (MAX_BYTES - 3, 4), (u64::MAX, 4)] {
            assert_eq!(
                check_limits([1, 1], 4096, 0, retained, bytes),
                Err(NativeGuiTextureError::MemoryLimit)
            );
        }
    }

    #[test]
    fn uploads_validate_extent_format_length_and_budget() {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        assert_eq!(upload_layout([4, 4], [1, 1], [2, 2], format, 16), Ok(8));
        assert_eq!(
            upload_layout([4, 4], [0, 0], [2, 2], wgpu::TextureFormat::Rgba16Float, 32),
            Ok(16)
        );
        for (origin, size, length) in [
            ([0, 0], [0, 1], 0),
            ([3, 0], [2, 1], 8),
            ([u32::MAX, 0], [1, 1], 4),
            ([0, 0], [2, 2], 15),
        ] {
            assert_eq!(
                upload_layout([4, 4], origin, size, format, length),
                Err(NativeGuiTextureError::InvalidUpload)
            );
        }
        assert_eq!(
            upload_layout([4096; 2], [0; 2], [4096; 2], format, 4096 * 4096 * 4),
            Err(NativeGuiTextureError::UploadLimit)
        );
        assert_eq!(
            upload_layout(
                [1; 2],
                [0; 2],
                [1; 2],
                wgpu::TextureFormat::Rgba8UnormSrgb,
                4
            ),
            Err(NativeGuiTextureError::UnsupportedFormat)
        );
    }

    #[test]
    fn atlas_parts_need_no_texture_selection() {
        let parts: Vec<_> = TextureParts::new(&[], 6..24).collect();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].0, 6..24);
        assert!(parts[0].1.is_none());
        assert!(TextureParts::new(&[], 6..6).next().is_none());
    }

    fn sampled_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texels: &[[u8; 4]],
    ) -> wgpu::Texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: texels.len() as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            texels.as_flattened(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(texture.width() * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
        texture
    }

    fn render(
        renderer: &mut HudRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batch: &HudBatch,
        size: [u32; 2],
        split: bool,
    ) -> (Vec<u8>, (u64, u64)) {
        let descriptor = wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let target = device.create_texture(&descriptor);
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            ..descriptor
        });
        let stride = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(stride) * u64::from(size[1]),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = target.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut uploads = crate::gfx::upload::Uploads::new(device);
        renderer.prepare(device, &mut uploads, &mut encoder, batch, size);
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        if split {
            for range in [0..batch.blur_at, batch.blur_at..batch.vertices.len() as u32] {
                renderer.draw_range(&mut encoder, &view, &depth_view, batch, size, range, None);
            }
        } else {
            renderer.draw(&mut encoder, &view, &depth_view, batch, size);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &output,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        output
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = output.slice(..).get_mapped_range().unwrap();
        let pixels = mapped
            .chunks_exact(stride as usize)
            .flat_map(|row| row[..size[0] as usize * 4].iter().copied())
            .collect();
        let counts = &renderer.native_texture_draw_counts;
        (
            pixels,
            (
                counts.passes.swap(0, Ordering::Relaxed),
                counts.groups.swap(0, Ordering::Relaxed),
            ),
        )
    }

    fn atlas_rect(batch: &mut HudBatch, rect: [f32; 4], color: u32) {
        batch.rect_uv(
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 1,
                h: 1,
            },
            color,
        );
    }

    #[test]
    fn texture_order_tint_clipping_limits_and_generations() {
        let instance = crate::gfx::test_gpu::instance();
        let Some((device, queue)) =
            pollster::block_on(instance.request_adapter(&Default::default()))
                .ok()
                .and_then(|adapter| {
                    pollster::block_on(adapter.request_device(&Default::default())).ok()
                })
        else {
            return;
        };
        let atlas = sampled_texture(&device, &queue, &[[255; 4]]);
        let view = atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let mut renderer = HudRenderer::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            &view,
            &view,
            &sampler,
        )
        .unwrap();
        let mut plain = HudBatch::new([1, 1]);
        atlas_rect(&mut plain, [0.0, 0.0, 1.0, 1.0], u32::MAX);
        let plain_vertices = bytemuck::cast_slice::<_, u8>(&plain.vertices).to_vec();
        let baseline = render(&mut renderer, &device, &queue, &plain, [1, 1], false);
        assert!(plain.native_textures.is_none());
        assert!(renderer.native_textures.get().is_none());
        assert_eq!(baseline, (vec![255; 4], (1, 1)));
        plain.native_textures = Some(Vec::new());
        assert_eq!(
            render(&mut renderer, &device, &queue, &plain, [1, 1], false),
            baseline
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&plain.vertices),
            plain_vertices
        );
        assert!(renderer.native_textures.get().is_none());

        let upload = renderer
            .native_gui_texture_create(
                &device,
                [1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
                NativeGuiTextureFilter::Nearest,
            )
            .unwrap();
        assert_eq!(
            renderer.native_gui_texture_write(&queue, &upload, [0, 0], [1, 1], &[255, 0, 0, 255]),
            Ok(())
        );
        let mut uploaded = HudBatch::new([1, 1]);
        assert!(uploaded.native_gui_texture(
            &upload,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None
        ));
        assert_eq!(
            render(&mut renderer, &device, &queue, &uploaded, [1, 1], false).0,
            vec![255, 0, 0, 255]
        );
        assert_eq!(
            renderer.native_gui_texture_write(&queue, &upload, [1, 0], [1, 1], &[0; 4]),
            Err(NativeGuiTextureError::InvalidUpload)
        );
        drop(uploaded);
        drop(upload);

        assert!(matches!(
            renderer.native_gui_texture_create(
                &device,
                [u32::MAX; 2],
                wgpu::TextureFormat::Rgba16Float,
                NativeGuiTextureFilter::Nearest,
            ),
            Err(NativeGuiTextureError::InvalidDimensions)
        ));
        let registration_count = live_registrations();
        let temporary = renderer
            .native_gui_texture_register(&device, &atlas, NativeGuiTextureFilter::Nearest)
            .unwrap();
        let registration = Arc::downgrade(&temporary.0);
        let generation = Arc::clone(&temporary.0.generation);
        assert_eq!(live_registrations(), registration_count + 1);
        let mut recycled = HudBatch::new([1, 1]);
        assert!(recycled.native_gui_texture(
            &temporary,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        assert_eq!(
            render(&mut renderer, &device, &queue, &recycled, [1, 1], false),
            baseline
        );
        assert_eq!(
            render(&mut renderer, &device, &queue, &recycled, [1, 1], false),
            baseline
        );
        let commands = [DrawKey {
            vertices: [0, 6],
            registration: temporary.registration(),
            blend: Blend::Alpha,
            text: false,
        }];
        let manager = renderer.native_textures.get().unwrap();
        let cached = manager
            .bundle(&commands, false, |_| panic!("bundle was not reused"))
            .unwrap();
        assert!(
            manager
                .bundle(&[DrawKey::EMPTY; MAX_CACHED_DRAWS + 1], false, |_| {
                    panic!("oversized bundle was retained")
                })
                .is_none()
        );
        for offset in 1..=BUNDLE_CACHE_SIZE {
            let mut commands = commands;
            commands[0].vertices = [offset as u32, offset as u32 + 6];
            assert!(
                manager
                    .bundle(&commands, false, |_| cached.clone())
                    .is_some()
            );
        }
        assert_eq!(
            generation
                .bundles
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .entries
                .iter()
                .flatten()
                .count(),
            BUNDLE_CACHE_SIZE
        );
        drop(cached);
        drop(temporary);
        assert_eq!(registration.strong_count(), 1);
        drop(recycled);
        assert_eq!(registration.strong_count(), 0);
        assert_eq!(live_registrations(), registration_count);
        assert!(
            generation
                .bundles
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .entries
                .iter()
                .all(Option::is_none)
        );
        assert!(
            RUN_POOL
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .slots
                .iter()
                .flatten()
                .all(Vec::is_empty)
        );

        let temporary = renderer
            .native_gui_texture_register(&device, &atlas, NativeGuiTextureFilter::Nearest)
            .unwrap();
        let registration = Arc::downgrade(&temporary.0);
        let mut clipped = HudBatch::new([1, 1]);
        assert!(clipped.native_gui_texture(
            &temporary,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        drop(temporary);
        clipped.clip_since(0, 2.0, 2.0, 3.0, 3.0);
        assert!(clipped.native_textures.is_none());
        assert_eq!(registration.strong_count(), 0);
        assert_eq!(live_registrations(), registration_count);

        let red = sampled_texture(&device, &queue, &[[255, 0, 0, 255]]);
        let green = sampled_texture(&device, &queue, &[[0, 255, 0, 255]]);
        let first = renderer
            .native_gui_texture_register(&device, &red, NativeGuiTextureFilter::Nearest)
            .unwrap();
        let second = renderer
            .native_gui_texture_register(&device, &green, NativeGuiTextureFilter::Linear)
            .unwrap();
        assert!(Arc::ptr_eq(&first.0, &first.clone().0));
        let mut native_only = HudBatch::new([1, 1]);
        for _ in 0..2 {
            assert!(native_only.native_gui_texture(
                &first,
                [0.0, 0.0, 1.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
                u32::MAX,
                None,
            ));
        }
        assert_eq!(native_only.native_textures.as_ref().unwrap().len(), 1);
        assert_eq!(
            render(&mut renderer, &device, &queue, &native_only, [1, 1], false).1,
            (1, 1)
        );
        let mut ordered = HudBatch::new([1, 1]);
        atlas_rect(&mut ordered, [0.0, 0.0, 1.0, 1.0], u32::MAX);
        for texture in [&first, &second] {
            assert!(ordered.native_gui_texture(
                texture,
                [0.0, 0.0, 1.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
                0x80ff_ffff,
                None,
            ));
        }
        atlas_rect(&mut ordered, [0.0, 0.0, 1.0, 1.0], rgba(0, 0, 255, 128));
        let runs = ordered.native_textures.as_ref().unwrap();
        let parts: Vec<_> = TextureParts::new(runs, 0..24).collect();
        assert_eq!(
            parts.iter().map(|part| part.0.clone()).collect::<Vec<_>>(),
            [0..6, 6..12, 12..18, 18..24]
        );
        assert!(parts[0].1.is_none() && parts[3].1.is_none());
        assert!(Arc::ptr_eq(&parts[1].1.unwrap().0, &first.0));
        assert!(Arc::ptr_eq(&parts[2].1.unwrap().0, &second.0));
        let (pixels, counts) = render(&mut renderer, &device, &queue, &ordered, [1, 1], false);
        assert_eq!(counts, (1, 4));
        for (actual, expected) in pixels.iter().zip([63_u8, 95, 159, 255]) {
            assert!(actual.abs_diff(expected) <= 1);
        }
        ordered.blur_at = 12;
        assert_eq!(
            render(&mut renderer, &device, &queue, &ordered, [1, 1], true).0,
            pixels
        );

        let colors = sampled_texture(&device, &queue, &[[255, 0, 0, 255], [0, 255, 0, 255]]);
        let selection = renderer
            .native_gui_texture_register(&device, &colors, NativeGuiTextureFilter::Nearest)
            .unwrap();
        let mut tinted = HudBatch::new([2, 1]);
        assert!(tinted.native_gui_texture(
            &selection,
            [0.0, 0.0, 2.0, 1.0],
            [0.5, 0.0, 1.0, 1.0],
            0x8080_4020,
            None,
        ));
        assert!(
            tinted
                .vertices
                .iter()
                .all(|vertex| vertex.color == 0x8020_4080)
        );
        let tinted_pixels = render(&mut renderer, &device, &queue, &tinted, [2, 1], false).0;
        for pixel in tinted_pixels.chunks_exact(4) {
            for (actual, expected) in pixel.iter().zip([0_u8, 32, 0, 128]) {
                assert!(actual.abs_diff(expected) <= 1);
            }
        }
        let mut geometry = HudBatch::new([2, 1]);
        geometry.rect_uv(
            0.0,
            0.0,
            2.0,
            1.0,
            Rect {
                u0: 0.5,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 2,
                h: 1,
            },
            0x8020_4080,
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&tinted.vertices),
            bytemuck::cast_slice::<_, u8>(&geometry.vertices)
        );

        let mut clipped = HudBatch::new([10, 10]);
        atlas_rect(&mut clipped, [0.0, 0.0, 1.0, 1.0], u32::MAX);
        assert!(clipped.native_gui_texture(
            &first,
            [0.0, 0.0, 4.0, 4.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        assert!(clipped.native_gui_texture(
            &second,
            [8.0, 8.0, 2.0, 2.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        atlas_rect(&mut clipped, [2.0, 1.0, 2.0, 2.0], u32::MAX);
        clipped.blur_at = 18;
        clipped.blend_since(0, Blend::Additive);
        clipped.clip_since(6, 2.0, 1.0, 4.0, 3.0);
        assert_eq!(clipped.vertices.len(), 18);
        assert_eq!(clipped.blur_at, 12);
        assert_eq!(
            clipped.blends,
            [(6..12, Blend::Alpha), (0..18, Blend::Additive)]
        );
        let runs = clipped.native_textures.as_ref().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].range, 6..12);
        for vertex in &clipped.vertices[6..12] {
            assert!((2.0..=4.0).contains(&vertex.pos[0]));
            assert!((1.0..=3.0).contains(&vertex.pos[1]));
            assert!((0.5..=1.0).contains(&vertex.uv[0]));
            assert!((0.25..=0.75).contains(&vertex.uv[1]));
        }
        clipped.clip_since(0, 20.0, 20.0, 21.0, 21.0);
        assert!(clipped.vertices.is_empty());
        assert!(clipped.native_textures.is_none());

        let mut sliced = HudBatch::new([5, 5]);
        assert!(sliced.native_gui_texture(
            &first,
            [0.0, 0.0, 5.0, 5.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            Some(NativeGuiNineSlice {
                source_size: [3, 3],
                border: [1; 4],
            }),
        ));
        assert_eq!(sliced.vertices.len(), 54);
        assert_eq!(sliced.native_textures.as_ref().unwrap()[0].range, 0..54);
        let before = sliced.vertices.len();
        assert!(!sliced.native_gui_texture(
            &first,
            [0.0, 0.0, f32::NAN, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        assert!(!sliced.native_gui_texture(
            &first,
            [0.0, 0.0, 5.0, 5.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            Some(NativeGuiNineSlice {
                source_size: [u32::MAX; 2],
                border: [u32::MAX; 4],
            }),
        ));
        assert_eq!(sliced.vertices.len(), before);

        let created = renderer
            .native_gui_texture_create(
                &device,
                [2, 2],
                wgpu::TextureFormat::Bgra8Unorm,
                NativeGuiTextureFilter::Nearest,
            )
            .unwrap();
        assert!(created.texture().usage().contains(
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
        ));
        drop(created);
        let float_supported = wgpu::TextureFormat::Rgba16Float
            .guaranteed_format_features(device.features())
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE);
        assert_eq!(
            renderer
                .native_gui_texture_create(
                    &device,
                    [1, 1],
                    wgpu::TextureFormat::Rgba16Float,
                    NativeGuiTextureFilter::Linear,
                )
                .is_ok(),
            float_supported
        );
        assert_eq!(
            renderer
                .native_gui_texture_create(
                    &device,
                    [1, 1],
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    NativeGuiTextureFilter::Nearest,
                )
                .unwrap_err(),
            NativeGuiTextureError::UnsupportedFormat
        );
        let unsampled = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        assert_eq!(
            renderer
                .native_gui_texture_register(&device, &unsampled, NativeGuiTextureFilter::Nearest)
                .unwrap_err(),
            NativeGuiTextureError::MissingSampledUsage
        );
        let mut retained = Vec::new();
        for _ in 3..MAX_REGISTRATIONS {
            retained.push(
                renderer
                    .native_gui_texture_create(
                        &device,
                        [1, 1],
                        wgpu::TextureFormat::Rgba8Unorm,
                        NativeGuiTextureFilter::Nearest,
                    )
                    .unwrap(),
            );
        }
        assert_eq!(
            renderer
                .native_gui_texture_create(
                    &device,
                    [1, 1],
                    wgpu::TextureFormat::Rgba8Unorm,
                    NativeGuiTextureFilter::Nearest,
                )
                .unwrap_err(),
            NativeGuiTextureError::RegistrationLimit
        );
        retained.pop();
        let replacement = renderer
            .native_gui_texture_create(
                &device,
                [1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
                NativeGuiTextureFilter::Nearest,
            )
            .unwrap();
        drop(replacement);
        drop(retained);

        let saved = renderer
            .native_gui_texture_create(
                &device,
                [1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
                NativeGuiTextureFilter::Nearest,
            )
            .unwrap();
        renderer
            .native_gui_texture_write(&queue, &saved, [0, 0], [1, 1], &[255, 0, 0, 255])
            .unwrap();

        device_lost();
        assert!(
            first
                .0
                .generation
                .bundles
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .entries
                .iter()
                .all(Option::is_none)
        );
        assert!(!first.is_valid());
        assert!(!renderer.native_gui_texture_owns(&first));
        assert!(!plain.native_gui_texture(
            &first,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        assert_eq!(
            renderer
                .native_gui_texture_create(
                    &device,
                    [1, 1],
                    wgpu::TextureFormat::Rgba8Unorm,
                    NativeGuiTextureFilter::Nearest,
                )
                .unwrap_err(),
            NativeGuiTextureError::StaleGeneration
        );
        let mut reloaded = HudRenderer::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            &view,
            &view,
            &sampler,
        )
        .unwrap();
        assert_eq!(
            reloaded
                .native_world_texture_restore(&device, &queue, &first)
                .unwrap_err(),
            NativeGuiTextureError::StaleGeneration
        );
        let restored = reloaded
            .native_world_texture_restore(&device, &queue, &saved)
            .unwrap();
        assert_eq!(restored.registration(), saved.registration());
        assert_eq!(restored.filter(), NativeGuiTextureFilter::Nearest);
        let mut restored_batch = HudBatch::new([1, 1]);
        assert!(restored_batch.native_gui_texture(
            &restored,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        assert_eq!(
            render(
                &mut reloaded,
                &device,
                &queue,
                &restored_batch,
                [1, 1],
                false
            )
            .0,
            [255, 0, 0, 255]
        );
        let fresh = reloaded
            .native_gui_texture_create(
                &device,
                [1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
                NativeGuiTextureFilter::Nearest,
            )
            .unwrap();
        assert_ne!(fresh.generation(), first.generation());
        assert!(fresh.is_valid());
        let mut fresh_batch = HudBatch::new([1, 1]);
        assert!(fresh_batch.native_gui_texture(
            &fresh,
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            u32::MAX,
            None,
        ));
        render(&mut reloaded, &device, &queue, &fresh_batch, [1, 1], false);
        assert!(
            fresh
                .0
                .generation
                .bundles
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .entries
                .iter()
                .any(Option::is_some)
        );
        drop(reloaded);
        assert!(!fresh.is_valid());
        assert!(
            fresh
                .0
                .generation
                .bundles
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .entries
                .iter()
                .all(Option::is_none)
        );
    }
}
