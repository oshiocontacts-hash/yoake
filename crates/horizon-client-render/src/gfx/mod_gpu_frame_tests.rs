use super::mod_gpu::{ModId, Stage};
use super::mod_gpu_resources::Readback;
use super::*;
use std::sync::Mutex;

fn renderer() -> Renderer {
    let instance = test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("headless renderer adapter");
    let features = timing::GpuTimer::features(&adapter)
        | cull::Cull::features(&adapter)
        | oit::features(&adapter)
        | (adapter.features()
            & (wgpu::Features::INDIRECT_FIRST_INSTANCE
                | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT
                | wgpu::Features::MAPPABLE_PRIMARY_BUFFERS
                | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::RG11B10UFLOAT_RENDERABLE
                | wgpu::Features::SUBGROUP
                | wgpu::Features::VERTEX_WRITABLE_STORAGE
                | wgpu::Features::CLEAR_TEXTURE
                | wgpu::Features::TEXTURE_ATOMIC
                | wgpu::Features::FLOAT32_FILTERABLE));
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: adapter.limits(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    }))
    .expect("headless renderer device");
    Renderer::from_device(
        Opened {
            instance,
            surface: None,
            adapter,
            device,
            queue,
            device_lost: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
        None,
        winit::dpi::PhysicalSize::new(64, 64),
        &[255; 16 * 16 * 4],
        16,
        16,
        1,
        &[crate::mipmap::Settings::default()],
        &[],
        (&[255; 16 * 16 * 4], 16, 16),
        (&[255; 16 * 16 * 4], 16, 16),
        (&[255; 16 * 16 * 4], 16, 16),
        (&[255; 16 * 16 * 4], 16),
        Arc::new(panorama::Faces::colour([0, 0, 0, 255])),
        None,
        None,
        false,
        false,
    )
    .expect("headless renderer")
}

#[test]
fn renderer_mod_stages_finish_engine_uploads_and_reuse_belts() {
    let recovery = std::env::temp_dir().join(format!(
        "yoake-renderer-frame-{}-{:?}.json",
        std::process::id(),
        std::thread::current().id()
    ));
    mod_gpu_runtime::set_recovery_path_for_tests(recovery.clone()).unwrap();
    let mut renderer = renderer();
    let owner = ModId(71_001);
    let hud = crate::hud::HudBatch::new([64, 64]);
    let scenarios: &[(&[Stage], bool)] = &[
        (&[Stage::WorldOpaque], false),
        (&[Stage::WorldTranslucent], false),
        (&[Stage::Hud], true),
        (&[Stage::AfterWorld], false),
        (&[Stage::Hud], false),
        (
            &[
                Stage::WorldOpaque,
                Stage::WorldTranslucent,
                Stage::AfterWorld,
                Stage::Hud,
            ],
            true,
        ),
    ];
    for &(stages, native) in scenarios {
        let buffer = mod_gpu_runtime::with_error(owner, |gpu, device, _, _| {
            gpu.create_buffer(
                device,
                owner,
                16,
                (wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC).bits(),
            )
        })
        .unwrap();
        let tickets = Arc::new(Mutex::new(Vec::new()));
        let recorded = tickets.clone();
        mod_gpu_runtime::register(
            owner,
            "renderer-frame-test",
            Arc::new(move || {
                mod_gpu_runtime::with_error(owner, |gpu, device, _, frame| {
                    for (index, stage) in stages.iter().enumerate() {
                        let values = [frame.index as u32, index as u32, 123, 456];
                        gpu.write_buffer(
                            device,
                            owner,
                            *stage,
                            buffer,
                            0,
                            bytemuck::cast_slice(&values),
                        )?;
                        let ticket = gpu.read_buffer(device, owner, *stage, buffer, 0, 16)?;
                        recorded.lock().unwrap().push((ticket, values));
                    }
                    Ok(())
                })
                .unwrap();
            }),
        );
        let mut steady_pool = None;
        for frame_index in 0..32 {
            renderer.upload(
                (0, 0, 0),
                &crate::mesher::SectionMesh {
                    solid: vec![bytemuck::Zeroable::zeroed(); 4],
                    ..Default::default()
                },
            );
            if native {
                renderer
                    .native_services
                    .draws
                    .quad(
                        [
                            [0.0, 99.0, 2.0],
                            [1.0, 99.0, 2.0],
                            [1.0, 100.0, 2.0],
                            [0.0, 100.0, 2.0],
                        ],
                        0xffff_ffff,
                    )
                    .unwrap();
            }
            let validation = renderer
                .device
                .push_error_scope(wgpu::ErrorFilter::Validation);
            assert!(
                renderer.render(&frame(&hud)).unwrap(),
                "{stages:?}, native={native}"
            );
            renderer
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let error = pollster::block_on(validation.pop());
            assert!(
                error.is_none(),
                "engine upload validation: {stages:?}, native={native}: {error:?}"
            );
            let mut completed = tickets.lock().unwrap();
            assert_eq!(completed.len(), stages.len(), "missing mod stage commands");
            for (ticket, values) in completed.drain(..) {
                let readback = mod_gpu_runtime::with_error(owner, |gpu, device, _, _| {
                    gpu.poll_readback(device, owner, ticket)
                })
                .unwrap();
                match readback {
                    Readback::Ready(bytes) => {
                        assert_eq!(bytes, bytemuck::cast_slice::<u32, u8>(&values));
                    }
                    Readback::Pending => panic!("submitted guest copy did not complete"),
                }
            }
            let pool = (
                renderer.uploads.chunk_count(&renderer.device),
                renderer.section_uploads.chunk_count(&renderer.device),
            );
            if frame_index == 8 {
                assert!(pool.0 > 0 && pool.1 > 0, "both engine belts must upload");
                steady_pool = Some(pool);
            } else if frame_index > 8 {
                assert_eq!(
                    Some(pool),
                    steady_pool,
                    "belt pool grew after warmup: {stages:?}"
                );
            }
        }
        mod_gpu_runtime::remove(owner);
    }
    let _ = std::fs::remove_file(recovery);
}

fn frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        eye: glam::DVec3::ZERO,
        eye_rotation: [0.0; 2],
        roll: 0.0,
        first_person: true,
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        water_plane: None,
        weather: &[],
        weather_time: crate::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_degrees: 85.0,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: crate::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: false,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: crate::block_entity_render::StaticCacheFrameStats::default(),
        entity_motion: &[],
        colored_lights: &[],
        emissive_layers: &[],
        shadow_only: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        celestial: &[],
        stars: None,
        hand: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        menu_blur: None,
        chunk_fade: 0.0,
        post_effects: crate::post_chain::Request::none(),
    }
}
