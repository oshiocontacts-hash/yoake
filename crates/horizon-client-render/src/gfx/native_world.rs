use super::block_light::LightTable;
use crate::fast_hash::FxHashMap;
use crate::{mesher::MeshTables, world::World};
use horizon_shader_api::{
    Frame,
    manifest::{LightColor, Manifest, light_color},
    voxel,
};
use horizon_world::registry::{BlockRegistry, BlockStateId};

const BLOCK_LIGHT_CAPACITY: usize = 2048;
const LIGHT_CAPACITY: usize = BLOCK_LIGHT_CAPACITY + 1;
const VOXEL_UPLOAD_BUDGET: usize = 16;
const LIGHT_MAX_DISTANCE: f64 = 128.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WorldLight {
    position: [f32; 3],
    level: f32,
    color: [f32; 3],
    radius: f32,
    held: u32,
    padding: [u32; 3],
}

#[derive(Clone, Default)]
pub(super) struct Bindings {
    voxel: Option<wgpu::TextureView>,
    lights: Option<wgpu::Buffer>,
}

impl Bindings {
    pub(super) fn layout_entries(&self) -> Vec<wgpu::BindGroupLayoutEntry> {
        let mut entries = Vec::new();
        if self.voxel.is_some() {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 14,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            });
        }
        if self.lights.is_some() {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 15,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        entries
    }

    pub(super) fn entries(&self) -> Vec<wgpu::BindGroupEntry<'_>> {
        let mut entries = Vec::new();
        if let Some(view) = &self.voxel {
            entries.push(wgpu::BindGroupEntry {
                binding: 14,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        if let Some(buffer) = &self.lights {
            entries.push(wgpu::BindGroupEntry {
                binding: 15,
                resource: buffer.as_entire_binding(),
            });
        }
        entries
    }
}

struct VoxelGrid {
    texture: wgpu::Texture,
    window: voxel::Window,
    values: FxHashMap<BlockStateId, u32>,
}

impl VoxelGrid {
    fn upload_dirty(&mut self, queue: &wgpu::Queue, camera_block: [i32; 3], world: &World) -> usize {
        self.window.move_camera(camera_block);
        let dirty = self
            .window
            .take_dirty_nearest(camera_block, VOXEL_UPLOAD_BUDGET);
        let uploaded = dirty.len();
        let mut cells = [0u32; 16 * 16 * 16];
        for section in dirty {
            let blocks = world
                .chunks
                .get(&(section[0], section[2]))
                .and_then(|chunk| chunk.section_at(section[1] * 16).ok())
                .map(|section| &section.blocks);
            voxel_cells(&mut cells, blocks, &self.values);
            let Some(slot) = self.window.section_slot(section) else {
                continue;
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: slot[0] * 16,
                        y: slot[1] * 16,
                        z: slot[2] * 16,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&cells),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(64),
                    rows_per_image: Some(16),
                },
                wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 16,
                },
            );
        }
        uploaded
    }
}

pub(super) struct NativeWorld {
    pub(super) bindings: Bindings,
    voxel: Option<VoxelGrid>,
    light_table: LightTable,
    light_colors: Vec<LightColor>,
    registry: Option<&'static BlockRegistry>,
    light_cache: LightCache,
    light_count: u32,
    camera: glam::DVec3,
}

impl NativeWorld {
    pub(super) fn validate_device(device: &wgpu::Device, manifest: &Manifest) -> Result<(), String> {
        Self::validate_limits(&device.limits(), manifest)
    }

    fn validate_limits(limits: &wgpu::Limits, manifest: &Manifest) -> Result<(), String> {
        if manifest.voxels.is_some() && limits.max_texture_dimension_3d < 16 {
            return Err("native pack voxels require a 3D texture dimension of at least 16".into());
        }
        let light_bytes = (LIGHT_CAPACITY * std::mem::size_of::<WorldLight>()) as u64;
        if manifest.light_list
            && (u64::from(limits.max_storage_buffer_binding_size) < light_bytes
                || limits.max_buffer_size < light_bytes
                || limits.max_storage_buffers_per_shader_stage == 0)
        {
            return Err("native pack light_list exceeds the device's storage buffer limits".into());
        }
        Ok(())
    }

    pub(super) fn new(
        device: &wgpu::Device,
        manifest: &Manifest,
        light_table: LightTable,
    ) -> Result<Self, String> {
        Self::validate_device(device, manifest)?;
        let window = manifest
            .voxels
            .as_ref()
            .map(|settings| {
                let size = device_voxel_size(settings.size, device.limits().max_texture_dimension_3d);
                voxel::Window::new(size, [0; 3]).map(|window| (size, window))
            })
            .transpose()?;
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut bindings = Bindings::default();
        let voxel = window.map(|(size, window)| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("native pack voxels"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: size[2],
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            bindings.voxel = Some(texture.create_view(&Default::default()));
            VoxelGrid {
                texture,
                window,
                values: FxHashMap::default(),
            }
        });
        if manifest.light_list {
            bindings.lights = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("native pack light list"),
                size: (LIGHT_CAPACITY * std::mem::size_of::<WorldLight>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let validation_error = pollster::block_on(validation.pop());
        let memory_error = pollster::block_on(memory.pop());
        if let Some(error) = validation_error.or(memory_error) {
            return Err(format!("native pack world inputs: {error}"));
        }
        Ok(Self {
            bindings,
            voxel,
            light_colors: manifest.lights.clone(),
            light_table,
            registry: None,
            light_cache: LightCache::default(),
            light_count: 0,
            camera: glam::DVec3::ZERO,
        })
    }

    pub(super) fn needs_lights(&self) -> bool {
        self.bindings.lights.is_some()
    }

    pub(super) fn invalidate(&mut self) {
        self.light_cache.key = None;
        if let Some(grid) = &mut self.voxel {
            grid.window.invalidate();
        }
    }

    pub(super) fn mark_dirty(&mut self, section: [i32; 3]) {
        if let Some(grid) = &mut self.voxel {
            grid.window.mark_dirty(section);
        }
    }

    pub(super) fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera: glam::DVec3,
        world: &World,
        tables: &MeshTables,
        registry: &'static BlockRegistry,
    ) {
        self.camera = camera;
        let registry_changed = refresh_registry(&mut self.registry, registry);
        if registry_changed {
            self.invalidate();
            if let Some(grid) = &mut self.voxel {
                grid.values.clear();
            }
        }
        if self.needs_lights() && registry_changed {
            self.light_table = build_light_table(
                &self.light_colors,
                registry.iter().filter_map(|(state_id, state)| {
                    tables
                        .facts(state_id)
                        .map(|facts| (state_id.0, state.name.as_str(), facts.emission))
                }),
            );
        }
        let Some(grid) = &mut self.voxel else {
            return;
        };
        if registry_changed {
            for (state_id, state) in registry.iter() {
                let Some(facts) = tables.facts(state_id) else {
                    continue;
                };
                let name = state.name.strip_prefix("minecraft:").unwrap_or(&state.name);
                let property = |key: &str| state.properties.get(key).map(String::as_str);
                let shape = if !tables.voxel_occupied(state_id) {
                    voxel::Shape::Empty
                } else if name.ends_with("_slab") && property("type") != Some("double") {
                    if property("type") == Some("top") {
                        voxel::Shape::SlabTop
                    } else {
                        voxel::Shape::SlabBottom
                    }
                } else if name.ends_with("_stairs") {
                    stair_shape(property("facing"), property("half"))
                } else if name.ends_with("_pane") || name == "iron_bars" {
                    voxel::Shape::Pane
                } else if name.ends_with("_fence") || name.ends_with("_wall") {
                    voxel::Shape::Thin
                } else if facts.full_collision
                    || facts.solid_render
                    || name.contains("glass")
                    || name == "water"
                    || name == "lava"
                {
                    voxel::Shape::Full
                } else {
                    voxel::Shape::Thin
                };
                let tint = crate::block_entity_render::DYES
                    .iter()
                    .find(|(dye, _)| name.starts_with(&format!("{dye}_stained_glass")))
                    .map(|(_, color)| *color)
                    .or_else(|| tables.default_tint(state_id, 0, 0x91bd59))
                    .unwrap_or(0xffffff);
                grid.values.insert(
                    state_id,
                    voxel::Voxel {
                        shape,
                        translucent: !facts.solid_render,
                        tint: [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8],
                        emission: facts.emission,
                    }
                    .encode(),
                );
            }
        }
        grid.upload_dirty(queue, camera.floor().as_ivec3().to_array(), world);
    }

    pub(super) fn update_lights(
        &mut self,
        queue: &wgpu::Queue,
        camera: glam::DVec3,
        sections: &FxHashMap<super::SectionKey, Vec<([i32; 3], u32)>>,
        revision: u64,
        held: Option<([f32; 3], u8)>,
    ) {
        let Some(buffer) = &self.bindings.lights else {
            return;
        };
        self.light_cache.update(
            &self.light_table,
            camera,
            revision,
            sections.values().flatten().copied(),
        );
        let lights = collect_lights(
            &self.light_table,
            camera,
            self.light_cache.sources.iter().copied(),
            held,
        );
        self.light_count = lights.len() as u32;
        if !lights.is_empty() {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&lights));
        }
    }

    pub(super) fn write_frame(&self, frame: &mut Frame) {
        frame.light_count = self.light_count;
        if let Some(grid) = &self.voxel {
            frame.voxel_size = [
                grid.texture.width(),
                grid.texture.height(),
                grid.texture.depth_or_array_layers(),
            ];
            frame.has_voxels = 1;
            let origin = grid.window.origin().map(f64::from);
            frame.voxel_origin = (glam::DVec3::from_array(origin) - self.camera)
                .as_vec3()
                .to_array();
            frame.voxel_center = std::array::from_fn(|axis| {
                frame.voxel_origin[axis] + frame.voxel_size[axis] as f32 * 0.5
            });
            frame.voxel_cell_size = 1.0;
            frame.voxel_origin_slot = grid.window.origin_slot();
            frame.voxel_validity = grid.window.validity();
        }
    }
}

fn device_voxel_size(size: [u32; 3], limit: u32) -> [u32; 3] {
    size.map(|side| {
        if side > limit {
            side.min(64).min(limit / 16 * 16)
        } else {
            side
        }
    })
}

fn refresh_registry(
    current: &mut Option<&'static BlockRegistry>,
    registry: &'static BlockRegistry,
) -> bool {
    if current.is_some_and(|previous| std::ptr::eq(previous, registry)) {
        return false;
    }
    *current = Some(registry);
    true
}

fn stair_shape(facing: Option<&str>, half: Option<&str>) -> voxel::Shape {
    match (half == Some("top"), facing) {
        (true, Some("east")) => voxel::Shape::StairsTopEast,
        (true, Some("south")) => voxel::Shape::StairsTopSouth,
        (true, Some("west")) => voxel::Shape::StairsTopWest,
        (true, _) => voxel::Shape::StairsTopNorth,
        (false, Some("east")) => voxel::Shape::StairsEast,
        (false, Some("south")) => voxel::Shape::StairsSouth,
        (false, Some("west")) => voxel::Shape::StairsWest,
        (false, _) => voxel::Shape::StairsNorth,
    }
}

fn voxel_cells(
    cells: &mut [u32; 4096],
    blocks: Option<&horizon_core::paletted::PalettedContainer>,
    values: &FxHashMap<BlockStateId, u32>,
) {
    let Some(blocks) = blocks else {
        cells.fill(0);
        return;
    };
    if let Some(state) = blocks.uniform() {
        cells.fill(values.get(&BlockStateId(state)).copied().unwrap_or(0));
        return;
    }
    for local_z in 0..16 {
        for local_y in 0..16 {
            for local_x in 0..16 {
                let state = BlockStateId(blocks.get(local_x + local_z * 16 + local_y * 256));
                cells[local_x + local_y * 16 + local_z * 256] = values.get(&state).copied().unwrap_or(0);
            }
        }
    }
}

#[derive(Default)]
struct LightCache {
    key: Option<(u64, [i32; 3])>,
    sources: Vec<([i32; 3], u32)>,
}

impl LightCache {
    fn update(
        &mut self,
        table: &LightTable,
        camera: glam::DVec3,
        revision: u64,
        sources: impl IntoIterator<Item = ([i32; 3], u32)>,
    ) {
        let key = (revision, camera.floor().as_ivec3().to_array());
        if self.key == Some(key) {
            return;
        }
        let mut candidates: Vec<_> = sources
            .into_iter()
            .filter_map(|(position, state)| {
                let &(_, level) = table.get(&state)?;
                let distance = (glam::DVec3::from_array(position.map(f64::from))
                    + glam::DVec3::splat(0.5)
                    - camera)
                    .length_squared();
                (level > 0 && distance <= (LIGHT_MAX_DISTANCE + f64::from(level)).powi(2))
                    .then_some((distance, position, state))
            })
            .collect();
        candidates.sort_unstable_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        candidates.truncate(BLOCK_LIGHT_CAPACITY);
        self.sources = candidates
            .into_iter()
            .map(|(_, position, state)| (position, state))
            .collect();
        self.key = Some(key);
    }
}

fn build_light_table<'a>(
    colors: &[LightColor],
    states: impl IntoIterator<Item = (u32, &'a str, u8)>,
) -> LightTable {
    states
        .into_iter()
        .filter_map(|(state, name, emission)| {
            (emission > 0)
                .then(|| (state, (light_color(colors, name).unwrap_or([1.0; 3]), emission)))
        })
        .collect()
}

fn collect_lights(
    table: &LightTable,
    camera: glam::DVec3,
    sources: impl IntoIterator<Item = ([i32; 3], u32)>,
    held: Option<([f32; 3], u8)>,
) -> Vec<WorldLight> {
    let mut lights = Vec::with_capacity(LIGHT_CAPACITY);
    if let Some((position, level)) = held {
        lights.push(WorldLight {
            position,
            level: f32::from(level),
            color: [1.0, 0.8, 0.6],
            radius: f32::from(level),
            held: 1,
            padding: [0; 3],
        });
    }
    lights.extend(
        sources
            .into_iter()
            .filter_map(|(position, state)| {
                let &(color, level) = table.get(&state)?;
                Some(WorldLight {
                    position: (glam::DVec3::from_array(position.map(f64::from))
                        + glam::DVec3::splat(0.5)
                        - camera)
                        .as_vec3()
                        .to_array(),
                    level: f32::from(level),
                    color,
                    radius: f32::from(level),
                    held: 0,
                    padding: [0; 3],
                })
            })
            .take(BLOCK_LIGHT_CAPACITY),
    );
    lights
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    #[test]
    fn uploads_are_bounded_and_mesher_backlog_does_not_repeat_them() {
        let Some((device, queue)) = device() else {
            return;
        };
        let mut manifest = Manifest::parse(
            "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.1\"\n[[post]]\nfile = \"post.wgsl\"\n[voxels]\nsize = [64, 64, 64]\n",
        )
        .unwrap();
        let mut inputs = NativeWorld::new(&device, &manifest, LightTable::default()).unwrap();
        let mut world = World::new(BlockStateId(0), 0);
        world.dirty_sections.insert((0, 0, 0));
        let grid = inputs.voxel.as_mut().unwrap();
        for _ in 0..4 {
            assert_eq!(grid.upload_dirty(&queue, [0; 3], &world), VOXEL_UPLOAD_BUDGET);
        }
        for _ in 0..3 {
            assert_eq!(grid.upload_dirty(&queue, [0; 3], &world), 0);
        }
        inputs.mark_dirty([0; 3]);
        inputs.mark_dirty([0; 3]);
        let grid = inputs.voxel.as_mut().unwrap();
        assert_eq!(grid.upload_dirty(&queue, [0; 3], &world), 1);
        assert_eq!(grid.upload_dirty(&queue, [0; 3], &world), 0);
        assert_eq!(grid.upload_dirty(&queue, [256; 3], &world), VOXEL_UPLOAD_BUDGET);
        assert_eq!(grid.window.take_dirty().len(), 64 - VOXEL_UPLOAD_BUDGET);
        manifest.voxels.as_mut().unwrap().size = [0; 3];
        assert!(NativeWorld::new(&device, &manifest, LightTable::default()).is_err());
    }

    #[test]
    fn registry_identity_changes_even_when_state_count_is_unchanged() {
        let facts =
            br#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#;
        let first = Box::leak(Box::new(BlockRegistry::from_facts_json(&facts[..]).unwrap()));
        let replacement = Box::leak(Box::new(BlockRegistry::from_facts_json(&facts[..]).unwrap()));
        let mut registry = None;
        assert!(refresh_registry(&mut registry, first));
        assert!(!refresh_registry(&mut registry, first));
        assert!(refresh_registry(&mut registry, replacement));
        assert!(!refresh_registry(&mut registry, replacement));
    }

    #[test]
    fn registry_rebuilds_resolve_colors_by_name_not_previous_state_ids() {
        let colors = [LightColor {
            color: [0.2, 0.3, 0.4],
            blocks: vec!["torch".into()],
        }];
        let table = build_light_table(
            &colors,
            [
                (0, "minecraft:air", 0),
                (1, "minecraft:lava", 12),
                (2, "minecraft:torch", 14),
            ],
        );
        assert!(!table.contains_key(&0));
        assert_eq!(table[&1], ([1.0; 3], 12));
        assert_eq!(table[&2], ([0.2, 0.3, 0.4], 14));
        let rebuilt = build_light_table(
            &colors,
            [
                (0, "minecraft:torch", 7),
                (1, "minecraft:air", 0),
                (2, "minecraft:lava", 15),
            ],
        );
        assert_eq!(rebuilt[&0], ([0.2, 0.3, 0.4], 7));
        assert!(!rebuilt.contains_key(&1));
        assert_eq!(rebuilt[&2], ([1.0; 3], 15));
    }

    #[test]
    fn uniform_air_sections_clear_old_voxels_and_palette_axes_match_the_texture() {
        use horizon_core::paletted::{PalettedContainer, Strategy};
        let values = [(BlockStateId(0), 0), (BlockStateId(1), 23)]
            .into_iter()
            .collect();
        let mut cells = [99; 4096];
        let mut blocks = PalettedContainer::new(Strategy::blocks(16), 0);
        voxel_cells(&mut cells, Some(&blocks), &values);
        assert!(cells.iter().all(|&value| value == 0));
        blocks.set(2 + 3 * 16 + 4 * 256, 1);
        voxel_cells(&mut cells, Some(&blocks), &values);
        assert_eq!(cells[2 + 4 * 16 + 3 * 256], 23);
        assert_eq!(cells.iter().filter(|&&value| value != 0).count(), 1);
        voxel_cells(&mut cells, None, &values);
        assert!(cells.iter().all(|&value| value == 0));
    }

    #[test]
    fn device_fallback_preserves_supported_axes_and_reports_unsupported_inputs() {
        assert_eq!(device_voxel_size([256, 32, 128], 128), [64, 32, 128]);
        assert_eq!(device_voxel_size([32, 256, 16], 48), [32, 48, 16]);
        let manifest = Manifest::parse(
            "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.1\"\nlight_list = true\n[[post]]\nfile = \"post.wgsl\"\n[voxels]\n",
        )
        .unwrap();
        let mut limits = wgpu::Limits::default();
        assert!(NativeWorld::validate_limits(&limits, &manifest).is_ok());
        limits.max_texture_dimension_3d = 15;
        assert!(NativeWorld::validate_limits(&limits, &manifest).is_err());
        limits.max_texture_dimension_3d = 64;
        limits.max_storage_buffer_binding_size = 16;
        assert!(NativeWorld::validate_limits(&limits, &manifest).is_err());
    }

    #[test]
    fn stairs_encode_each_facing_in_both_halves() {
        for (facing, bottom, top) in [
            ("north", 4, 10),
            ("east", 5, 11),
            ("south", 6, 12),
            ("west", 7, 13),
        ] {
            assert_eq!(stair_shape(Some(facing), Some("bottom")) as u32, bottom);
            assert_eq!(stair_shape(Some(facing), Some("top")) as u32, top);
        }
    }

    #[test]
    fn light_selection_is_nearest_bounded_order_independent_and_cached() {
        let table = [(1, ([1.0; 3], 15))].into_iter().collect();
        let mut sources: Vec<_> = (0..2100)
            .map(|index| ([80 + index % 20, index / 20, 0], 1))
            .collect();
        sources.extend([([0, 0, 0], 1), ([500, 0, 0], 1), ([0, 0, 1], 99)]);
        let camera = glam::DVec3::splat(0.5);
        let mut cache = LightCache::default();
        cache.update(&table, camera, 1, sources.iter().copied());
        assert_eq!(cache.sources.len(), BLOCK_LIGHT_CAPACITY);
        assert_eq!(cache.sources[0], ([0; 3], 1));
        assert!(
            !cache
                .sources
                .iter()
                .any(|&(position, _)| position == [500, 0, 0])
        );
        sources.reverse();
        let mut reversed = LightCache::default();
        reversed.update(&table, camera, 1, sources);
        assert_eq!(cache.sources, reversed.sources);
        let moved = camera + glam::DVec3::splat(0.25);
        cache.update(
            &table,
            moved,
            1,
            std::iter::from_fn(|| panic!("unchanged cell must not scan sources")),
        );
        let rebased = collect_lights(&table, moved, cache.sources.iter().copied(), None);
        assert_eq!(rebased[0].position, [-0.25; 3]);
        cache.update(&table, moved, 2, [([1, 0, 0], 1)]);
        assert_eq!(cache.sources, [([1, 0, 0], 1)]);
        cache.update(&table, camera + glam::DVec3::X, 2, [([2, 0, 0], 1)]);
        assert_eq!(cache.sources, [([2, 0, 0], 1)]);
    }

    #[test]
    fn lights_keep_camera_relative_precision_and_held_metadata() {
        let mut table = LightTable::default();
        table.insert(1, ([0.2, 0.3, 0.4], 12));
        let lights = collect_lights(
            &table,
            glam::DVec3::new(30_000_000.25, -28.5, -29_999_999.75),
            [([30_000_000, -30, -30_000_000], 1), ([0; 3], 0)],
            Some(([0.0, -0.4, 0.2], 9)),
        );
        assert_eq!(lights.len(), 2);
        assert_eq!(lights[0].position, [0.0, -0.4, 0.2]);
        assert_eq!(lights[0].level, 9.0);
        assert_eq!(lights[0].radius, 9.0);
        assert_eq!(lights[0].held, 1);
        assert_eq!(lights[1].position, [0.25, -1.0, 0.25]);
        assert_eq!(lights[1].color, [0.2, 0.3, 0.4]);
        assert_eq!(lights[1].level, 12.0);
        assert_eq!(lights[1].radius, 12.0);
        assert_eq!(lights[1].held, 0);
    }

    #[test]
    fn block_light_cap_reserves_one_additional_held_slot() {
        let mut table = LightTable::default();
        table.insert(1, ([1.0; 3], 15));
        for held in [None, Some(([0.0; 3], 15))] {
            let lights = collect_lights(
                &table,
                glam::DVec3::ZERO,
                (0..BLOCK_LIGHT_CAPACITY + 10).map(|index| ([index as i32, 0, 0], 1)),
                held,
            );
            assert_eq!(
                lights.len(),
                BLOCK_LIGHT_CAPACITY + usize::from(held.is_some())
            );
            assert_eq!(
                lights.iter().filter(|light| light.held == 0).count(),
                BLOCK_LIGHT_CAPACITY,
            );
            assert_eq!(
                lights.iter().filter(|light| light.held == 1).count(),
                usize::from(held.is_some()),
            );
        }
    }
}
