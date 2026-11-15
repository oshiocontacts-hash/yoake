use super::block_light::LightTable;
use crate::fast_hash::FxHashMap;
use crate::{mesher::MeshTables, world::World};
use horizon_shader_api::{Frame, manifest::Manifest, voxel};
use horizon_world::registry::{BlockRegistry, BlockStateId};

const BLOCK_LIGHT_CAPACITY: usize = 2048;
const LIGHT_CAPACITY: usize = BLOCK_LIGHT_CAPACITY + 1;

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

pub(super) struct NativeWorld {
    pub(super) bindings: Bindings,
    voxel: Option<VoxelGrid>,
    light_table: LightTable,
    light_table_ready: bool,
    light_count: u32,
    camera: glam::DVec3,
}

impl NativeWorld {
    pub(super) fn validate_device(
        device: &wgpu::Device,
        manifest: &Manifest,
    ) -> Result<(), String> {
        let limits = device.limits();
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

    pub(super) fn new(device: &wgpu::Device, manifest: &Manifest, light_table: LightTable) -> Self {
        let mut bindings = Bindings::default();
        let voxel = manifest.voxels.as_ref().map(|settings| {
            let limit = device.limits().max_texture_dimension_3d;
            let mut size = settings.size;
            if size.iter().any(|&side| side > limit) {
                size = size.map(|side| side.min(64).min(limit / 16 * 16));
            }
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
                window: voxel::Window::new(size, [0; 3]).expect("validated native voxel size"),
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
        Self {
            bindings,
            voxel,
            light_table,
            light_table_ready: false,
            light_count: 0,
            camera: glam::DVec3::ZERO,
        }
    }

    pub(super) fn needs_lights(&self) -> bool {
        self.bindings.lights.is_some()
    }

    pub(super) fn invalidate(&mut self) {
        if let Some(grid) = &mut self.voxel {
            grid.window.invalidate();
        }
    }

    pub(super) fn mark_dirty(&mut self, section: [i32; 3]) {
        if let Some(grid) = &mut self.voxel {
            grid.window.mark_dirty(section);
        }
    }

    pub(super) fn observe_dirty_sections(&mut self, world: &World) {
        let Some(grid) = &mut self.voxel else {
            return;
        };
        for ((chunk_x, chunk_z), mask) in world.dirty_sections.columns() {
            for section_y in mask.iter() {
                grid.window.mark_dirty([
                    chunk_x,
                    world.bottom_y().div_euclid(16) + section_y as i32,
                    chunk_z,
                ]);
            }
        }
    }

    pub(super) fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera: glam::DVec3,
        world: &World,
        tables: &MeshTables,
        registry: &BlockRegistry,
    ) {
        self.camera = camera;
        self.observe_dirty_sections(world);
        if self.needs_lights() && !self.light_table_ready {
            extend_default_lights(
                &mut self.light_table,
                registry.iter().filter_map(|(state, _)| {
                    tables.facts(state).map(|facts| (state.0, facts.emission))
                }),
            );
            self.light_table_ready = true;
        }
        let Some(grid) = &mut self.voxel else {
            return;
        };
        if grid.values.is_empty() {
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
                    match property("facing") {
                        Some("east") => voxel::Shape::StairsEast,
                        Some("south") => voxel::Shape::StairsSouth,
                        Some("west") => voxel::Shape::StairsWest,
                        _ => voxel::Shape::StairsNorth,
                    }
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
        grid.window
            .move_camera(camera.floor().as_ivec3().to_array());
        let mut cells = [0u32; 16 * 16 * 16];
        for section in grid.window.take_dirty() {
            for local_z in 0..16 {
                for local_y in 0..16 {
                    for local_x in 0..16 {
                        let block_x = section[0] * 16 + local_x as i32;
                        let block_y = section[1] * 16 + local_y as i32;
                        let block_z = section[2] * 16 + local_z as i32;
                        cells[local_x + local_y * 16 + local_z * 256] =
                            if world.loaded(block_x, block_z) {
                                grid.values
                                    .get(&world.block(block_x, block_y, block_z))
                                    .copied()
                                    .unwrap_or(0)
                            } else {
                                0
                            };
                    }
                }
            }
            let Some(slot) = grid.window.section_slot(section) else {
                continue;
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &grid.texture,
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
    }

    pub(super) fn update_lights(
        &mut self,
        queue: &wgpu::Queue,
        camera: glam::DVec3,
        sections: &FxHashMap<super::SectionKey, Vec<([i32; 3], u32)>>,
        held: Option<([f32; 3], u8)>,
    ) {
        let Some(buffer) = &self.bindings.lights else {
            return;
        };
        let lights = collect_lights(
            &self.light_table,
            camera,
            sections.values().flatten().copied(),
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
        }
    }
}

fn extend_default_lights(table: &mut LightTable, states: impl IntoIterator<Item = (u32, u8)>) {
    for (state, emission) in states {
        if emission > 0 {
            table.entry(state).or_insert(([1.0; 3], emission));
        }
    }
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

    #[test]
    fn emission_defaults_do_not_require_pack_colors_or_replace_overrides() {
        let mut table = LightTable::default();
        table.insert(2, ([0.2, 0.3, 0.4], 7));
        extend_default_lights(&mut table, [(0, 0), (1, 12), (2, 14)]);
        assert!(!table.contains_key(&0));
        assert_eq!(table[&1], ([1.0; 3], 12));
        assert_eq!(table[&2], ([0.2, 0.3, 0.4], 7));
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
