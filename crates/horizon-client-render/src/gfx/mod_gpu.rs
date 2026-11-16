use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const MAX_MOD_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_HANDLES: u32 = 4096;
pub const MAX_DRAWS: u32 = 4096;
pub const MAX_VERTICES: u64 = 1 << 24;
pub const MAX_WORKGROUPS: u64 = 65_536;
pub const MAX_UPLOAD_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_READ_BYTES: u64 = 1024 * 1024;
const ALLOCATION_ALIGNMENT: u64 = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Descriptor,
    BufferSize,
    TextureSize,
    Handles,
    HandleSpace,
    Memory,
    Handle,
    Kind,
    Frame,
    Commands,
    Draws,
    Vertices,
    Workgroups,
    Upload,
    Readback,
    EngineBinding,
    Disabled,
    Host(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgba8Unorm,
    Rgba8Srgb,
    Rgba16Float,
    Rgba32Float,
    R32Float,
    Rg16Float,
    Depth32Float,
}

impl Format {
    fn bytes(self) -> u64 {
        match self {
            Self::Rgba16Float => 8,
            Self::Rgba32Float => 16,
            _ => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub depth_or_layers: u32,
    pub mips: u32,
    pub is_3d: bool,
    pub format: Format,
}

impl Texture {
    fn bytes(self) -> Result<u64, Error> {
        let maximum = if self.is_3d { 512 } else { 8192 };
        if self.width == 0
            || self.height == 0
            || self.depth_or_layers == 0
            || self.width > maximum
            || self.height > maximum
            || self.depth_or_layers > if self.is_3d { 512 } else { 256 }
        {
            return Err(Error::TextureSize);
        }
        let largest =
            self.width
                .max(self.height)
                .max(if self.is_3d { self.depth_or_layers } else { 1 });
        if self.mips == 0 || self.mips > largest.ilog2() + 1 {
            return Err(Error::Descriptor);
        }
        let mut bytes = 0;
        for mip in 0..self.mips {
            let depth = if self.is_3d {
                (self.depth_or_layers >> mip).max(1)
            } else {
                self.depth_or_layers
            };
            bytes += u64::from((self.width >> mip).max(1))
                * u64::from((self.height >> mip).max(1))
                * u64::from(depth)
                * self.format.bytes();
        }
        Ok(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allocation {
    Buffer(u64),
    Texture(Texture),
    Sampler,
    Shader(usize),
    RenderPipeline,
    ComputePipeline,
    BindGroup,
    Readback(u64),
}

impl Allocation {
    fn bytes(self) -> Result<u64, Error> {
        let bytes = match self {
            Self::Buffer(bytes) if bytes > 0 && bytes <= MAX_MOD_BYTES => bytes,
            Self::Buffer(_) => return Err(Error::BufferSize),
            Self::Texture(texture) => texture.bytes()?,
            Self::Shader(bytes)
                if bytes > 0 && bytes <= super::mod_gpu_shader::MAX_SHADER_BYTES =>
            {
                bytes as u64
            }
            Self::Shader(_) => return Err(Error::Descriptor),
            Self::Readback(bytes) if bytes > 0 && bytes <= MAX_READ_BYTES => bytes * 2,
            Self::Readback(_) => return Err(Error::Readback),
            _ => ALLOCATION_ALIGNMENT,
        };
        Ok(bytes.div_ceil(ALLOCATION_ALIGNMENT) * ALLOCATION_ALIGNMENT)
    }

    fn same_kind(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub bytes: u64,
    pub handles: u32,
}

#[derive(Default)]
struct Ledger {
    owners: BTreeMap<ModId, Usage>,
    total: u64,
    next_handle: u64,
}

struct Lease {
    ledger: Arc<Mutex<Ledger>>,
    owner: ModId,
    bytes: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(usage) = ledger.owners.get_mut(&self.owner) {
            usage.bytes -= self.bytes;
            usage.handles -= 1;
            if usage.handles == 0 {
                ledger.owners.remove(&self.owner);
            }
            ledger.total -= self.bytes;
        }
    }
}

pub struct Resource<Value> {
    value: Value,
    allocation: Allocation,
    lease: Lease,
}

impl<Value> Resource<Value> {
    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn allocation(&self) -> Allocation {
        self.allocation
    }
}

pub struct Resources<Value> {
    ledger: Arc<Mutex<Ledger>>,
    values: BTreeMap<u32, Arc<Resource<Value>>>,
}

impl<Value> Default for Resources<Value> {
    fn default() -> Self {
        Self {
            ledger: Arc::default(),
            values: BTreeMap::new(),
        }
    }
}

impl<Value> Resources<Value> {
    pub fn create(
        &mut self,
        owner: ModId,
        allocation: Allocation,
        create: impl FnOnce() -> Result<Value, Error>,
    ) -> Result<u32, Error> {
        let bytes = allocation.bytes()?;
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let usage = ledger.owners.get(&owner).copied().unwrap_or_default();
        if usage.handles >= MAX_HANDLES {
            return Err(Error::Handles);
        }
        if bytes > MAX_MOD_BYTES - usage.bytes || bytes > MAX_TOTAL_BYTES - ledger.total {
            return Err(Error::Memory);
        }
        let handle = u32::try_from(ledger.next_handle + 1).map_err(|_| Error::HandleSpace)?;
        ledger.next_handle = u64::from(handle);
        ledger.owners.insert(
            owner,
            Usage {
                bytes: usage.bytes + bytes,
                handles: usage.handles + 1,
            },
        );
        ledger.total += bytes;
        let lease = Lease {
            ledger: Arc::clone(&self.ledger),
            owner,
            bytes,
        };
        drop(ledger);
        let value = create()?;
        self.values.insert(
            handle,
            Arc::new(Resource {
                value,
                allocation,
                lease,
            }),
        );
        Ok(handle)
    }

    pub fn get(&self, owner: ModId, handle: u32) -> Result<Arc<Resource<Value>>, Error> {
        self.values
            .get(&handle)
            .filter(|resource| resource.lease.owner == owner)
            .cloned()
            .ok_or(Error::Handle)
    }

    pub fn get_kind(
        &self,
        owner: ModId,
        handle: u32,
        kind: Allocation,
    ) -> Result<Arc<Resource<Value>>, Error> {
        let resource = self.get(owner, handle)?;
        if !resource.allocation.same_kind(kind) {
            return Err(Error::Kind);
        }
        Ok(resource)
    }

    pub fn destroy(&mut self, owner: ModId, handle: u32) -> Result<(), Error> {
        let resource = self.get(owner, handle)?;
        self.values.remove(&handle);
        drop(resource);
        Ok(())
    }

    pub fn remove_owner(&mut self, owner: ModId) {
        self.values
            .retain(|_, resource| resource.lease.owner != owner);
    }

    pub fn usage(&self, owner: ModId) -> Usage {
        self.ledger
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .owners
            .get(&owner)
            .copied()
            .unwrap_or_default()
    }

    pub fn total_bytes(&self) -> u64 {
        self.ledger
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .total
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    WorldOpaque,
    WorldTranslucent,
    AfterWorld,
    Hud,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineResource {
    Frame,
    BlockAtlas,
    BlockAtlasSampler,
    WorldDepth,
    SceneColor,
    Lightmap,
}

pub fn engine_binding(stage: Stage, resource: EngineResource, writable: bool) -> Result<(), Error> {
    if writable
        || (matches!(
            resource,
            EngineResource::WorldDepth | EngineResource::SceneColor
        ) && stage != Stage::AfterWorld)
    {
        return Err(Error::EngineBinding);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Draw { count: u32, instances: u32 },
    Dispatch([u32; 3]),
    Upload(u64),
    Readback(u64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FrameUsage {
    draws: u32,
    vertices: u64,
    workgroups: u64,
    uploaded: u64,
    read: u64,
    commands: u32,
}

impl FrameUsage {
    fn charge(self, kind: CommandKind) -> Result<Self, Error> {
        let mut next = self;
        match kind {
            CommandKind::Draw { count, instances } => {
                next.draws += 1;
                next.vertices = next
                    .vertices
                    .checked_add(u64::from(count) * u64::from(instances))
                    .ok_or(Error::Vertices)?;
            }
            CommandKind::Dispatch(groups) => {
                let groups = groups.into_iter().try_fold(1_u64, |product, axis| {
                    product
                        .checked_mul(u64::from(axis))
                        .ok_or(Error::Workgroups)
                })?;
                next.workgroups = next
                    .workgroups
                    .checked_add(groups)
                    .ok_or(Error::Workgroups)?;
            }
            CommandKind::Upload(bytes) => {
                next.uploaded = next.uploaded.checked_add(bytes).ok_or(Error::Upload)?;
            }
            CommandKind::Readback(bytes) => {
                next.read = next.read.checked_add(bytes).ok_or(Error::Readback)?;
            }
        }
        for (exceeded, error) in [
            (next.draws > MAX_DRAWS, Error::Draws),
            (next.vertices > MAX_VERTICES, Error::Vertices),
            (next.workgroups > MAX_WORKGROUPS, Error::Workgroups),
            (next.uploaded > MAX_UPLOAD_BYTES, Error::Upload),
            (next.read > MAX_READ_BYTES, Error::Readback),
            (next.commands >= MAX_DRAWS, Error::Commands),
        ] {
            if exceeded {
                return Err(error);
            }
        }
        next.commands += 1;
        Ok(next)
    }
}

pub struct Recorded<Command> {
    pub owner: ModId,
    pub stage: Stage,
    pub command: Command,
}

pub struct Recorder<Command> {
    active: bool,
    usage: BTreeMap<ModId, FrameUsage>,
    commands: Vec<Recorded<Command>>,
}

impl<Command> Default for Recorder<Command> {
    fn default() -> Self {
        Self {
            active: false,
            usage: BTreeMap::new(),
            commands: Vec::new(),
        }
    }
}

impl<Command> Recorder<Command> {
    pub fn remove_owner(&mut self, owner: ModId) {
        self.commands.retain(|command| command.owner != owner);
        self.usage.remove(&owner);
    }

    pub fn begin_frame(&mut self) -> Result<(), Error> {
        if self.active {
            return Err(Error::Frame);
        }
        self.active = true;
        self.usage.clear();
        Ok(())
    }

    pub fn record(
        &mut self,
        owner: ModId,
        stage: Stage,
        kind: CommandKind,
        create: impl FnOnce() -> Command,
    ) -> Result<(), Error> {
        if !self.active {
            return Err(Error::Frame);
        }
        let next = self
            .usage
            .get(&owner)
            .copied()
            .unwrap_or_default()
            .charge(kind)?;
        self.commands.push(Recorded {
            owner,
            stage,
            command: create(),
        });
        self.usage.insert(owner, next);
        Ok(())
    }

    pub fn finish_frame(&mut self) -> Result<Vec<Recorded<Command>>, Error> {
        if !self.active {
            return Err(Error::Frame);
        }
        self.active = false;
        let mut commands = std::mem::take(&mut self.commands);
        commands.sort_by_key(|command| (command.stage, command.owner));
        Ok(commands)
    }
}

#[cfg(test)]
#[path = "mod_gpu_tests.rs"]
mod tests;
