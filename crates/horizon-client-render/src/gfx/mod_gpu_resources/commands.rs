use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{Allocation, Error, Gpu, ModId, Owned, Value, checked, texture_format};
use crate::gfx::mod_gpu::{CommandKind, Stage};

pub struct Draw {
    pub pipeline: u32,
    pub groups: Vec<u32>,
    pub vertex: Option<(u32, u64, u64)>,
    pub index: Option<Index>,
    pub count: u32,
    pub instances: u32,
}

pub struct Index {
    pub buffer: u32,
    pub offset: u64,
    pub size: u64,
    pub u32_indices: bool,
}

pub enum Target {
    Frame,
    Texture {
        color: u32,
        depth: Option<u32>,
        clear: Option<[f64; 4]>,
    },
}

pub struct FrameTarget<'frame> {
    pub color: &'frame wgpu::TextureView,
    pub depth: Option<&'frame wgpu::TextureView>,
    pub format: wgpu::TextureFormat,
}

pub enum Readback {
    Pending,
    Ready(Vec<u8>),
}

pub(super) struct Ticket {
    buffer: wgpu::Buffer,
    size: u64,
    status: Arc<Mutex<Option<Result<(), String>>>>,
}

pub(super) enum Command {
    Write {
        source: Owned,
        destination: Owned,
        offset: u64,
        size: u64,
    },
    Copy {
        source: Owned,
        destination: Owned,
        source_offset: u64,
        destination_offset: u64,
        size: u64,
    },
    TextureWrite {
        source: Owned,
        destination: Owned,
        mip: u32,
        origin: [u32; 3],
        extent: [u32; 3],
        row_bytes: u32,
        rows: u32,
    },
    Dispatch {
        pipeline: Owned,
        groups: Vec<Owned>,
        workgroups: [u32; 3],
    },
    Draw {
        pipeline: Owned,
        groups: Vec<Owned>,
        vertex: Option<(Owned, u64, u64)>,
        index: Option<(Owned, u64, u64, bool)>,
        count: u32,
        instances: u32,
        target: DrawTarget,
    },
    Read {
        source: Owned,
        offset: u64,
        ticket: Owned,
    },
}

pub(super) enum DrawTarget {
    Frame,
    Texture {
        color: Owned,
        depth: Option<Owned>,
        clear: Option<[f64; 4]>,
    },
}

fn validate_stage(mask: u32, stage: Stage) -> Result<(), Error> {
    if mask & (1 << 3) != 0 && !matches!(stage, Stage::AfterWorld | Stage::WorldTranslucent)
        || mask & (1 << 4) != 0 && stage != Stage::AfterWorld
    {
        return Err(Error::EngineBinding);
    }
    Ok(())
}

impl DrawTarget {
    fn pixels(&self, width: u32, height: u32) -> Result<u64, Error> {
        match self {
            Self::Frame => Ok(u64::from(width).saturating_mul(u64::from(height))),
            Self::Texture { color, .. } => match color.value() {
                Value::Texture { descriptor, .. } => {
                    Ok(u64::from(descriptor.width).saturating_mul(u64::from(descriptor.height)))
                }
                _ => Err(Error::Kind),
            },
        }
    }
}

fn buffer(
    resource: &Owned,
    usage: wgpu::BufferUsages,
    offset: u64,
    size: u64,
) -> Result<&wgpu::Buffer, Error> {
    let Value::Buffer {
        buffer,
        size: capacity,
        usage: actual,
    } = resource.value()
    else {
        return Err(Error::Kind);
    };
    if size == 0
        || !actual.contains(usage)
        || offset.checked_add(size).is_none_or(|end| end > *capacity)
    {
        return Err(Error::BufferSize);
    }
    Ok(buffer)
}

impl Gpu {
    pub fn begin_frame(&mut self) -> Result<(), Error> {
        self.pending.clear();
        self.recorder.begin_frame()
    }

    pub fn finish_frame(&mut self) -> Result<(), Error> {
        self.pending = self.recorder.finish_frame()?;
        Ok(())
    }

    pub fn has_stage(&self, stage: Stage) -> bool {
        self.pending.iter().any(|command| command.stage == stage)
    }

    fn groups(&self, owner: ModId, pipeline: &Owned, handles: &[u32]) -> Result<Vec<Owned>, Error> {
        if handles.len() > 3 {
            return Err(Error::Descriptor);
        }
        let mut indices = std::collections::BTreeSet::new();
        handles
            .iter()
            .map(|handle| {
                let resource = self.resources.get(owner, *handle)?;
                let Value::BindGroup {
                    pipeline: expected,
                    index,
                    ..
                } = resource.value()
                else {
                    return Err(Error::Kind);
                };
                if !Arc::ptr_eq(pipeline, expected) || !indices.insert(*index) {
                    return Err(Error::Descriptor);
                }
                Ok(resource)
            })
            .collect()
    }

    fn staging(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        bytes: &[u8],
    ) -> Result<Owned, Error> {
        let handle =
            self.resources
                .create(owner, Allocation::Buffer(bytes.len() as u64), || {
                    checked(device, || {
                        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("Yoake mod upload"),
                            size: bytes.len() as u64,
                            usage: wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: true,
                        });
                        let mapped = buffer
                            .slice(..)
                            .get_mapped_range_mut()
                            .map_err(|error| Error::Host(error.to_string()))?;
                        let mut mapped = mapped;
                        mapped.copy_from_slice(bytes);
                        drop(mapped);
                        buffer.unmap();
                        Ok(Value::Buffer {
                            buffer,
                            size: bytes.len() as u64,
                            usage: wgpu::BufferUsages::COPY_SRC,
                        })
                    })?
                })?;
        let resource = self.resources.get(owner, handle)?;
        self.resources.destroy(owner, handle)?;
        Ok(resource)
    }

    pub fn write_buffer(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        stage: Stage,
        handle: u32,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.available(owner)?;
        if offset % 4 != 0
            || bytes.is_empty()
            || bytes.len() % 4 != 0
            || bytes.len() > 16 * 1024 * 1024
        {
            return Err(Error::Upload);
        }
        let destination = self.resources.get(owner, handle)?;
        buffer(
            &destination,
            wgpu::BufferUsages::COPY_DST,
            offset,
            bytes.len() as u64,
        )?;
        let source = self.staging(device, owner, bytes)?;
        self.recorder.record(
            owner,
            stage,
            CommandKind::Upload(bytes.len() as u64),
            || Command::Write {
                source,
                destination,
                offset,
                size: bytes.len() as u64,
            },
        )
    }

    pub fn copy_buffer(
        &mut self,
        owner: ModId,
        stage: Stage,
        source: u32,
        source_offset: u64,
        destination: u32,
        destination_offset: u64,
        size: u64,
    ) -> Result<(), Error> {
        self.available(owner)?;
        if source == destination
            || source_offset % 4 != 0
            || destination_offset % 4 != 0
            || size % 4 != 0
        {
            return Err(Error::Descriptor);
        }
        let source = self.resources.get(owner, source)?;
        let destination = self.resources.get(owner, destination)?;
        buffer(&source, wgpu::BufferUsages::COPY_SRC, source_offset, size)?;
        buffer(
            &destination,
            wgpu::BufferUsages::COPY_DST,
            destination_offset,
            size,
        )?;
        self.recorder
            .record(owner, stage, CommandKind::Copy(size), || Command::Copy {
                source,
                destination,
                source_offset,
                destination_offset,
                size,
            })
    }

    pub fn write_texture(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        stage: Stage,
        handle: u32,
        mip: u32,
        origin: [u32; 3],
        extent: [u32; 3],
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.available(owner)?;
        let destination = self.resources.get(owner, handle)?;
        let Value::Texture {
            descriptor, usage, ..
        } = destination.value()
        else {
            return Err(Error::Kind);
        };
        if !usage.contains(wgpu::TextureUsages::COPY_DST)
            || descriptor.format == super::super::mod_gpu::Format::Depth32Float
            || mip >= descriptor.mips
            || extent.contains(&0)
        {
            return Err(Error::TextureSize);
        }
        let dimensions = [
            (descriptor.width >> mip).max(1),
            (descriptor.height >> mip).max(1),
            if descriptor.is_3d {
                (descriptor.depth_or_layers >> mip).max(1)
            } else {
                descriptor.depth_or_layers
            },
        ];
        if (0..3).any(|axis| {
            origin[axis]
                .checked_add(extent[axis])
                .is_none_or(|end| end > dimensions[axis])
        }) {
            return Err(Error::TextureSize);
        }
        let texel_bytes = match descriptor.format {
            super::super::mod_gpu::Format::Rgba16Float => 8,
            super::super::mod_gpu::Format::Rgba32Float => 16,
            _ => 4,
        };
        let row = extent[0] as usize * texel_bytes;
        let expected = row
            .checked_mul(extent[1] as usize)
            .and_then(|size| size.checked_mul(extent[2] as usize))
            .ok_or(Error::Upload)?;
        let padded_row = row.div_ceil(256) * 256;
        let padded_size = padded_row
            .checked_mul(extent[1] as usize)
            .and_then(|size| size.checked_mul(extent[2] as usize))
            .ok_or(Error::Upload)?;
        if bytes.len() != expected || padded_size > 16 * 1024 * 1024 {
            return Err(Error::Upload);
        }
        let mut padded = vec![0; padded_size];
        for (source, destination) in bytes
            .chunks_exact(row)
            .zip(padded.chunks_exact_mut(padded_row))
        {
            destination[..row].copy_from_slice(source);
        }
        let source = self.staging(device, owner, &padded)?;
        self.recorder.record(
            owner,
            stage,
            CommandKind::Upload(padded_size as u64),
            || Command::TextureWrite {
                source,
                destination,
                mip,
                origin,
                extent,
                row_bytes: padded_row as u32,
                rows: extent[1],
            },
        )
    }

    pub fn dispatch(
        &mut self,
        owner: ModId,
        stage: Stage,
        pipeline: u32,
        groups: &[u32],
        workgroups: [u32; 3],
    ) -> Result<(), Error> {
        self.available(owner)?;
        if workgroups.contains(&0) {
            return Err(Error::Workgroups);
        }
        let pipeline = self
            .resources
            .get_kind(owner, pipeline, Allocation::ComputePipeline)?;
        let groups = self.groups(owner, &pipeline, groups)?;
        let Value::Compute {
            cost,
            budget,
            engine_mask,
            ..
        } = pipeline.value()
        else {
            return Err(Error::Kind);
        };
        validate_stage(*engine_mask, stage)?;
        let amount = workgroups
            .iter()
            .try_fold(*cost, |total, groups| total.checked_mul(u64::from(*groups)))
            .ok_or(Error::Workgroups)?;
        let budget = budget.clone();
        if !budget.lock().map_err(|_| Error::Disabled)?.check(
            self.frame_index,
            amount,
            super::super::mod_gpu::MAX_WORKGROUPS,
        ) {
            return Err(Error::Workgroups);
        }
        let cost = *cost;
        self.recorder.record(
            owner,
            stage,
            CommandKind::Compute {
                groups: workgroups,
                cost,
            },
            || Command::Dispatch {
                pipeline,
                groups,
                workgroups,
            },
        )?;
        budget
            .lock()
            .map_err(|_| Error::Disabled)?
            .charge(self.frame_index, amount);
        Ok(())
    }

    pub fn draw(
        &mut self,
        owner: ModId,
        stage: Stage,
        draw: Draw,
        target: Target,
    ) -> Result<(), Error> {
        self.available(owner)?;
        let pipeline = self
            .resources
            .get_kind(owner, draw.pipeline, Allocation::RenderPipeline)?;
        let Value::Render {
            descriptor,
            budget,
            engine_mask,
            ..
        } = pipeline.value()
        else {
            return Err(Error::Kind);
        };
        validate_stage(*engine_mask, stage)?;
        let budget = budget.clone();
        if draw.count == 0 || draw.instances == 0 {
            return Err(Error::Vertices);
        }
        let groups = self.groups(owner, &pipeline, &draw.groups)?;
        let vertex = draw
            .vertex
            .map(|(handle, offset, size)| {
                let resource = self.resources.get(owner, handle)?;
                buffer(&resource, wgpu::BufferUsages::VERTEX, offset, size)?;
                if offset % 4 != 0
                    || descriptor.stride == 0
                    || draw.index.is_none()
                        && (draw.count as u64)
                            .checked_mul(descriptor.stride)
                            .is_none_or(|needed| needed > size)
                {
                    return Err(Error::BufferSize);
                }
                Ok((resource, offset, size))
            })
            .transpose()?;
        if descriptor.stride != 0 && vertex.is_none() {
            return Err(Error::Descriptor);
        }
        let index = draw
            .index
            .map(|index| {
                let resource = self.resources.get(owner, index.buffer)?;
                let element = if index.u32_indices { 4 } else { 2 };
                buffer(
                    &resource,
                    wgpu::BufferUsages::INDEX,
                    index.offset,
                    index.size,
                )?;
                if index.offset % element != 0
                    || index.size % element != 0
                    || draw.count as u64 * element > index.size
                {
                    return Err(Error::BufferSize);
                }
                Ok((resource, index.offset, index.size, index.u32_indices))
            })
            .transpose()?;
        let target = match target {
            Target::Frame => {
                let format = if stage == Stage::Hud {
                    self.frame_limits.hud
                } else {
                    self.frame_limits.world
                };
                if format != Some(descriptor.color) || descriptor.depth && stage == Stage::Hud {
                    return Err(Error::Descriptor);
                }
                DrawTarget::Frame
            }
            Target::Texture {
                color,
                depth,
                clear,
            } => {
                if clear.is_some_and(|values| values.iter().any(|value| !value.is_finite())) {
                    return Err(Error::Descriptor);
                }
                let color = self.resources.get(owner, color)?;
                let Value::Texture {
                    descriptor: color_descriptor,
                    usage,
                    ..
                } = color.value()
                else {
                    return Err(Error::Kind);
                };
                if color_descriptor.format != descriptor.color
                    || color_descriptor.is_3d
                    || color_descriptor.depth_or_layers != 1
                    || color_descriptor.mips != 1
                    || !usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                {
                    return Err(Error::Descriptor);
                }
                let depth = depth
                    .map(|handle| {
                        let resource = self.resources.get(owner, handle)?;
                        let Value::Texture {
                            descriptor: depth_descriptor,
                            usage,
                            ..
                        } = resource.value()
                        else {
                            return Err(Error::Kind);
                        };
                        if !descriptor.depth
                            || depth_descriptor.format
                                != super::super::mod_gpu::Format::Depth32Float
                            || depth_descriptor.width != color_descriptor.width
                            || depth_descriptor.height != color_descriptor.height
                            || depth_descriptor.is_3d
                            || depth_descriptor.depth_or_layers != 1
                            || depth_descriptor.mips != 1
                            || !usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                        {
                            return Err(Error::Descriptor);
                        }
                        Ok(resource)
                    })
                    .transpose()?;
                if descriptor.depth != depth.is_some() {
                    return Err(Error::Descriptor);
                }
                DrawTarget::Texture {
                    color,
                    depth,
                    clear,
                }
            }
        };
        let vertices_per_primitive = match descriptor.topology {
            wgpu::PrimitiveTopology::PointList => 1,
            wgpu::PrimitiveTopology::LineList => 2,
            wgpu::PrimitiveTopology::TriangleList => 3,
            _ => return Err(Error::Descriptor),
        };
        let pixels = target
            .pixels(self.frame_limits.width, self.frame_limits.height)?
            .saturating_mul(u64::from(draw.count.div_ceil(vertices_per_primitive)))
            .saturating_mul(u64::from(draw.instances));
        let pixel_limit = self.pixel_limit();
        if !budget.lock().map_err(|_| Error::Disabled)?.check(
            self.frame_index,
            pixels,
            pixel_limit.saturating_mul(4),
        ) {
            return Err(Error::Pixels);
        }
        let new_pass = !self.recorder.previous(owner, stage).is_some_and(|command| {
            matches!(command, Command::Draw { pipeline, .. } if matches!(pipeline.value(), Value::Render { descriptor: previous, .. } if previous.depth == descriptor.depth))
                && command.merges_target(&target)
        });
        self.recorder.record(
            owner,
            stage,
            CommandKind::Render {
                count: draw.count,
                instances: draw.instances,
                pixels,
                pixel_limit,
                new_pass,
            },
            || Command::Draw {
                pipeline,
                groups,
                vertex,
                index,
                count: draw.count,
                instances: draw.instances,
                target,
            },
        )?;
        budget
            .lock()
            .map_err(|_| Error::Disabled)?
            .charge(self.frame_index, pixels);
        Ok(())
    }

    pub fn read_buffer(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        stage: Stage,
        handle: u32,
        offset: u64,
        size: u64,
    ) -> Result<u32, Error> {
        self.available(owner)?;
        if offset % 4 != 0
            || size == 0
            || size % 4 != 0
            || size > 1024 * 1024
            || self
                .tickets
                .get(&owner)
                .is_some_and(|tickets| tickets.len() >= 32)
        {
            return Err(Error::Readback);
        }
        let source = self.resources.get(owner, handle)?;
        buffer(&source, wgpu::BufferUsages::COPY_SRC, offset, size)?;
        let handle = self
            .resources
            .create(owner, Allocation::Readback(size), || {
                checked(device, || {
                    Value::Readback(Ticket {
                        buffer: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("Yoake mod readback"),
                            size,
                            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        size,
                        status: Arc::new(Mutex::new(None)),
                    })
                })
            })?;
        let ticket = self.resources.get(owner, handle)?;
        if let Err(error) = self
            .recorder
            .record(owner, stage, CommandKind::Readback(size), || {
                Command::Read {
                    source,
                    offset,
                    ticket,
                }
            })
        {
            self.resources.destroy(owner, handle)?;
            return Err(error);
        }
        self.tickets.entry(owner).or_default().insert(handle);
        Ok(handle)
    }

    pub fn poll_readback(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        handle: u32,
    ) -> Result<Readback, Error> {
        self.available(owner)?;
        let resource = self
            .resources
            .get_kind(owner, handle, Allocation::Readback(0))?;
        let Value::Readback(ticket) = resource.value() else {
            return Err(Error::Kind);
        };
        let status = ticket.status.lock().map_err(|_| Error::Disabled)?.clone();
        let Some(status) = status else {
            return Ok(Readback::Pending);
        };
        let mapped = status.is_ok();
        let result = status.map_err(Error::Host).and_then(|()| {
            checked(device, || {
                ticket
                    .buffer
                    .slice(..ticket.size)
                    .get_mapped_range()
                    .map(|bytes| bytes.to_vec())
            })?
            .map_err(|error| Error::Host(error.to_string()))
        });
        if mapped {
            ticket.buffer.unmap();
        }
        self.destroy(owner, handle)?;
        result.map(Readback::Ready)
    }

    pub fn submit_stage(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stage: Stage,
        frame: Option<&FrameTarget<'_>>,
    ) -> Vec<(ModId, Error)> {
        self.submit_stage_with_engine(device, queue, stage, frame, None)
    }

    pub fn submit_stage_with_engine(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stage: Stage,
        frame: Option<&FrameTarget<'_>>,
        sources: Option<&super::EngineSources<'_>>,
    ) -> Vec<(ModId, Error)> {
        if !self.has_stage(stage) {
            return Vec::new();
        }
        let pending = std::mem::take(&mut self.pending);
        let mut errors = Vec::new();
        let mut batches = std::collections::BTreeMap::<_, Vec<_>>::new();
        for recorded in pending {
            if self.disabled.contains_key(&recorded.owner) {
                continue;
            }
            if recorded.stage != stage {
                self.pending.push(recorded);
                continue;
            }
            batches.entry(recorded.owner).or_default().push(recorded);
        }
        let mut snapshots = super::engine::Snapshots::default();
        for (owner, batch) in batches {
            snapshots.groups.clear();
            let started = Instant::now();
            let mut submitted = false;
            let mut retained_timing = None;
            let result = checked(device, || -> Result<(), Error> {
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Yoake isolated mod GPU stage"),
                });
                let mut runs: Vec<Vec<&Command>> = Vec::new();
                for recorded in &batch {
                    if let Some(run) = runs.last_mut()
                        && run
                            .last()
                            .is_some_and(|previous| previous.merges(&recorded.command))
                    {
                        run.push(&recorded.command);
                    } else {
                        runs.push(vec![&recorded.command]);
                    }
                }
                let passes = runs.iter().filter(|run| run[0].is_pass()).count() as u32;
                let timing = self.timing_ticket(device, queue, owner, passes)?;
                let ticket = timing.as_ref().and_then(|resource| match resource.value() {
                    Value::Timing(ticket) => Some(ticket),
                    _ => None,
                });
                let mut query = 0;
                for run in runs {
                    let mut draws = Vec::with_capacity(run.len());
                    for command in run {
                        let groups = self.engine_groups(
                            device,
                            queue,
                            &mut encoder,
                            command,
                            stage,
                            sources,
                            &mut snapshots,
                        )?;
                        if let Some(ticket) = ticket
                            && let Some(budget) = command.pipeline_budget()
                            && let Ok(mut pipelines) = ticket.pipelines.lock()
                            && !pipelines
                                .iter()
                                .any(|existing| Arc::ptr_eq(existing, budget))
                        {
                            pipelines.push(budget.clone());
                        }
                        draws.push((command, groups));
                    }
                    let (command, groups) = &draws[0];
                    let timestamps = if command.is_pass() {
                        let timestamps = ticket.map(|ticket| (&ticket.queries, query));
                        query += 2;
                        timestamps
                    } else {
                        None
                    };
                    command.encode(&mut encoder, frame, groups, &draws[1..], timestamps)?;
                }
                if let Some(ticket) = ticket {
                    encoder.resolve_query_set(&ticket.queries, 0..ticket.count, &ticket.resolve, 0);
                    encoder.copy_buffer_to_buffer(
                        &ticket.resolve,
                        0,
                        &ticket.read,
                        0,
                        u64::from(ticket.count) * 8,
                    );
                }
                let command_buffer = encoder.finish();
                if let Some(hooks) = &self.submission_hooks {
                    (hooks.before)(owner).map_err(Error::Host)?;
                }
                queue.submit([command_buffer]);
                submitted = true;
                retained_timing = timing.clone();
                if let Some(ticket) = ticket {
                    ticket.map();
                }
                if let Some(timing) = timing {
                    self.timings.push((owner, timing));
                }
                for recorded in &batch {
                    if let Command::Read { ticket, .. } = &recorded.command {
                        let Value::Readback(ticket) = ticket.value() else {
                            return Err(Error::Kind);
                        };
                        let status = ticket.status.clone();
                        ticket
                            .buffer
                            .slice(..)
                            .map_async(wgpu::MapMode::Read, move |result| {
                                if let Ok(mut slot) = status.lock() {
                                    *slot = Some(result.map_err(|error| error.to_string()));
                                }
                            });
                    }
                }
                Ok(())
            })
            .and_then(|result| result);
            let elapsed = started.elapsed();
            if let Err(error) = result {
                snapshots = Default::default();
                self.disable(owner, error.to_string());
                errors.push((owner, error));
            }
            if let Err(error) = self.charge_cpu(owner, elapsed) {
                errors.push((owner, error));
            }
            if submitted {
                let hooks = self.submission_hooks.clone();
                queue.on_submitted_work_done(move || {
                    if let Some(hooks) = hooks {
                        (hooks.complete)(owner);
                    }
                    drop(retained_timing);
                    drop(batch);
                });
            }
        }
        errors
    }
}

impl Command {
    pub(super) fn reads_scene(&self) -> bool {
        self.engine_mask() & (1 << 4) != 0
    }

    fn engine_mask(&self) -> u32 {
        match self {
            Self::Dispatch { pipeline, .. } | Self::Draw { pipeline, .. } => match pipeline.value()
            {
                Value::Compute { engine_mask, .. } | Value::Render { engine_mask, .. } => {
                    *engine_mask
                }
                _ => 0,
            },
            _ => 0,
        }
    }

    fn pipeline_budget(&self) -> Option<&Arc<Mutex<super::timing::PipelineBudget>>> {
        match self {
            Self::Dispatch { pipeline, .. } | Self::Draw { pipeline, .. } => match pipeline.value()
            {
                Value::Compute { budget, .. } | Value::Render { budget, .. } => Some(budget),
                _ => None,
            },
            _ => None,
        }
    }

    fn merges_target(&self, next: &DrawTarget) -> bool {
        let Self::Draw { target, .. } = self else {
            return false;
        };
        match (target, next) {
            (DrawTarget::Frame, DrawTarget::Frame) => true,
            (
                DrawTarget::Texture { color, depth, .. },
                DrawTarget::Texture {
                    color: next_color,
                    depth: next_depth,
                    clear: None,
                },
            ) => {
                Arc::ptr_eq(color, next_color)
                    && match (depth, next_depth) {
                        (None, None) => true,
                        (Some(depth), Some(next_depth)) => Arc::ptr_eq(depth, next_depth),
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    fn merges(&self, next: &Self) -> bool {
        let Self::Draw { pipeline, .. } = self else {
            return false;
        };
        let Self::Draw {
            pipeline: next_pipeline,
            target,
            ..
        } = next
        else {
            return false;
        };
        let (
            Value::Render { descriptor, .. },
            Value::Render {
                descriptor: next_descriptor,
                ..
            },
        ) = (pipeline.value(), next_pipeline.value())
        else {
            return false;
        };
        descriptor.depth == next_descriptor.depth && self.merges_target(target)
    }

    fn is_pass(&self) -> bool {
        matches!(self, Self::Dispatch { .. } | Self::Draw { .. })
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: Option<&FrameTarget<'_>>,
        engine_groups: &[(u32, wgpu::BindGroup)],
        following: &[(&Command, Vec<(u32, wgpu::BindGroup)>)],
        timestamps: Option<(&wgpu::QuerySet, u32)>,
    ) -> Result<(), Error> {
        match self {
            Self::Write {
                source,
                destination,
                offset,
                size,
            } => encoder.copy_buffer_to_buffer(
                buffer(source, wgpu::BufferUsages::COPY_SRC, 0, *size)?,
                0,
                buffer(destination, wgpu::BufferUsages::COPY_DST, *offset, *size)?,
                *offset,
                *size,
            ),
            Self::Copy {
                source,
                destination,
                source_offset,
                destination_offset,
                size,
            } => encoder.copy_buffer_to_buffer(
                buffer(source, wgpu::BufferUsages::COPY_SRC, *source_offset, *size)?,
                *source_offset,
                buffer(
                    destination,
                    wgpu::BufferUsages::COPY_DST,
                    *destination_offset,
                    *size,
                )?,
                *destination_offset,
                *size,
            ),
            Self::TextureWrite {
                source,
                destination,
                mip,
                origin,
                extent,
                row_bytes,
                rows,
            } => {
                let Value::Texture { texture, .. } = destination.value() else {
                    return Err(Error::Kind);
                };
                let Value::Buffer { buffer, .. } = source.value() else {
                    return Err(Error::Kind);
                };
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(*row_bytes),
                            rows_per_image: Some(*rows),
                        },
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: *mip,
                        origin: wgpu::Origin3d {
                            x: origin[0],
                            y: origin[1],
                            z: origin[2],
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: extent[0],
                        height: extent[1],
                        depth_or_array_layers: extent[2],
                    },
                );
            }
            Self::Dispatch {
                pipeline,
                groups,
                workgroups,
            } => {
                let Value::Compute { pipeline, .. } = pipeline.value() else {
                    return Err(Error::Kind);
                };
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Yoake mod compute"),
                    timestamp_writes: timestamps.map(|(query_set, first)| {
                        wgpu::ComputePassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: Some(first),
                            end_of_pass_write_index: Some(first + 1),
                        }
                    }),
                });
                pass.set_pipeline(pipeline);
                for (index, group) in engine_groups {
                    pass.set_bind_group(*index, group, &[]);
                }
                for group in groups {
                    let Value::BindGroup { group, index, .. } = group.value() else {
                        return Err(Error::Kind);
                    };
                    pass.set_bind_group(*index, group, &[]);
                }
                pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
            }
            Self::Read {
                source,
                offset,
                ticket,
            } => {
                let Value::Readback(ticket) = ticket.value() else {
                    return Err(Error::Kind);
                };
                encoder.copy_buffer_to_buffer(
                    buffer(source, wgpu::BufferUsages::COPY_SRC, *offset, ticket.size)?,
                    *offset,
                    &ticket.buffer,
                    0,
                    ticket.size,
                );
            }
            Self::Draw {
                pipeline, target, ..
            } => {
                let Value::Render { descriptor, .. } = pipeline.value() else {
                    return Err(Error::Kind);
                };
                let (color, depth, clear) = match target {
                    DrawTarget::Frame => {
                        let frame = frame.ok_or(Error::Frame)?;
                        if frame.format != texture_format(descriptor.color)
                            || descriptor.depth && frame.depth.is_none()
                        {
                            return Err(Error::Descriptor);
                        }
                        (
                            frame.color,
                            if descriptor.depth { frame.depth } else { None },
                            None,
                        )
                    }
                    DrawTarget::Texture {
                        color,
                        depth,
                        clear,
                    } => {
                        let Value::Texture { view, .. } = color.value() else {
                            return Err(Error::Kind);
                        };
                        (
                            view,
                            depth.as_ref().and_then(|resource| {
                                if let Value::Texture { view, .. } = resource.value() {
                                    Some(view)
                                } else {
                                    None
                                }
                            }),
                            *clear,
                        )
                    }
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Yoake mod render"),
                    timestamp_writes: timestamps.map(|(query_set, first)| {
                        wgpu::RenderPassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: Some(first),
                            end_of_pass_write_index: Some(first + 1),
                        }
                    }),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: clear.map_or(wgpu::LoadOp::Load, |values| {
                                wgpu::LoadOp::Clear(wgpu::Color {
                                    r: values[0],
                                    g: values[1],
                                    b: values[2],
                                    a: values[3],
                                })
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: depth.map(|view| {
                        wgpu::RenderPassDepthStencilAttachment {
                            view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }
                    }),
                    ..Default::default()
                });
                self.draw_in_pass(&mut pass, engine_groups)?;
                for (command, groups) in following {
                    command.draw_in_pass(&mut pass, groups)?;
                }
            }
        }
        Ok(())
    }

    fn draw_in_pass(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        engine_groups: &[(u32, wgpu::BindGroup)],
    ) -> Result<(), Error> {
        let Self::Draw {
            pipeline,
            groups,
            vertex,
            index,
            count,
            instances,
            ..
        } = self
        else {
            return Err(Error::Kind);
        };
        let Value::Render { pipeline, .. } = pipeline.value() else {
            return Err(Error::Kind);
        };
        pass.set_pipeline(pipeline);
        for (index, group) in engine_groups {
            pass.set_bind_group(*index, group, &[]);
        }
        for group in groups {
            let Value::BindGroup { group, index, .. } = group.value() else {
                return Err(Error::Kind);
            };
            pass.set_bind_group(*index, group, &[]);
        }
        if let Some((resource, offset, size)) = vertex {
            pass.set_vertex_buffer(
                0,
                buffer(resource, wgpu::BufferUsages::VERTEX, *offset, *size)?
                    .slice(*offset..*offset + *size),
            );
        }
        if let Some((resource, offset, size, wide)) = index {
            pass.set_index_buffer(
                buffer(resource, wgpu::BufferUsages::INDEX, *offset, *size)?
                    .slice(*offset..*offset + *size),
                if *wide {
                    wgpu::IndexFormat::Uint32
                } else {
                    wgpu::IndexFormat::Uint16
                },
            );
            pass.draw_indexed(0..*count, 0, 0..*instances);
        } else {
            pass.draw(0..*count, 0..*instances);
        }
        Ok(())
    }
}
