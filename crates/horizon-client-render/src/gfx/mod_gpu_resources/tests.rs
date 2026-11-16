use super::*;
use crate::gfx::mod_gpu::Stage;

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("headless GPU adapter");
    pollster::block_on(adapter.request_device(&Default::default())).expect("headless GPU device")
}

#[test]
fn empty_frames_do_not_allocate_or_submit() {
    let mut gpu = Gpu::default();
    for _frame in 0..3 {
        gpu.begin_frame().unwrap();
        gpu.finish_frame().unwrap();
    }
    assert_eq!(gpu.total_bytes(), 0);
    for stage in [
        Stage::WorldOpaque,
        Stage::WorldTranslucent,
        Stage::AfterWorld,
        Stage::Hud,
    ] {
        assert!(!gpu.has_stage(stage));
    }
}

#[test]
fn compute_upload_copy_readback_and_ownership() {
    let (device, queue) = gpu();
    let mut gpu = Gpu::default();
    let owner = ModId(1);
    let usage =
        (wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST)
            .bits();
    let data = gpu.create_buffer(&device, owner, 16, usage).unwrap();
    let output = gpu.create_buffer(&device, owner, 16, usage).unwrap();
    let shader = gpu.create_shader(&device, owner, "@group(0) @binding(0) var<storage, read_write> values: array<u32>; @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id: vec3<u32>) { values[id.x] += 7u; }").unwrap();
    let pipeline = gpu.create_compute(&device, owner, shader, "main").unwrap();
    let group = gpu
        .create_bind_group(
            &device,
            owner,
            pipeline,
            0,
            &[Binding::Buffer {
                binding: 0,
                handle: data,
                offset: 0,
                size: 16,
            }],
        )
        .unwrap();
    gpu.begin_frame().unwrap();
    assert!(
        gpu.write_buffer(&device, ModId(2), Stage::AfterWorld, data, 0, &[0; 16])
            .is_err()
    );
    assert!(
        gpu.write_buffer(&device, owner, Stage::AfterWorld, data, 4, &[0; 16])
            .is_err()
    );
    gpu.write_buffer(
        &device,
        owner,
        Stage::AfterWorld,
        data,
        0,
        bytemuck::cast_slice(&[1u32, 2, 3, 4]),
    )
    .unwrap();
    gpu.dispatch(owner, Stage::AfterWorld, pipeline, &[group], [4, 1, 1])
        .unwrap();
    gpu.copy_buffer(owner, Stage::AfterWorld, data, 0, output, 0, 16)
        .unwrap();
    let ticket = gpu
        .read_buffer(&device, owner, Stage::AfterWorld, output, 0, 16)
        .unwrap();
    gpu.finish_frame().unwrap();
    gpu.destroy(owner, shader).unwrap();
    gpu.destroy(owner, group).unwrap();
    assert!(
        gpu.submit_stage(&device, &queue, Stage::AfterWorld, None)
            .is_empty()
    );
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let Readback::Ready(bytes) = gpu.poll_readback(&device, owner, ticket).unwrap() else {
        panic!("readback pending after wait")
    };
    assert_eq!(bytemuck::cast_slice::<u8, u32>(&bytes), &[8, 9, 10, 11]);
    assert!(gpu.poll_readback(&device, owner, ticket).is_err());
    gpu.remove_owner(owner);
    assert_eq!(gpu.total_bytes(), 0);
}

#[test]
fn texture_bounds_and_device_loss_are_isolated() {
    let (device, _queue) = gpu();
    let mut gpu = Gpu::default();
    let owner = ModId(3);
    let texture = gpu
        .create_texture(
            &device,
            owner,
            Texture {
                width: 2,
                height: 2,
                depth_or_layers: 1,
                mips: 1,
                is_3d: false,
                format: Format::Rgba8Unorm,
            },
            wgpu::TextureUsages::COPY_DST.bits(),
        )
        .unwrap();
    gpu.begin_frame().unwrap();
    assert!(
        gpu.write_texture(
            &device,
            owner,
            Stage::Hud,
            texture,
            0,
            [1, 0, 0],
            [2, 2, 1],
            &[0; 16]
        )
        .is_err()
    );
    assert!(
        gpu.write_texture(
            &device,
            owner,
            Stage::Hud,
            texture,
            1,
            [0; 3],
            [1; 3],
            &[0; 4]
        )
        .is_err()
    );
    gpu.write_texture(
        &device,
        owner,
        Stage::Hud,
        texture,
        0,
        [0; 3],
        [2, 2, 1],
        &[255; 16],
    )
    .unwrap();
    gpu.finish_frame().unwrap();
    gpu.disable(owner, "test".to_owned());
    assert!(!gpu.has_stage(Stage::Hud));
    assert_eq!(gpu.total_bytes(), 0);
    assert!(matches!(
        gpu.create_sampler(&device, owner, false, false),
        Err(Error::Disabled)
    ));
    assert!(gpu.create_sampler(&device, ModId(4), false, false).is_ok());
    gpu.device_lost();
    assert_eq!(gpu.total_bytes(), 0);
    assert!(matches!(
        gpu.create_sampler(&device, ModId(4), false, false),
        Err(Error::Disabled)
    ));
}
