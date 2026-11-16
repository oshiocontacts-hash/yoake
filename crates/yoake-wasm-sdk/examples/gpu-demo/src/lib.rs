use gpu::{
    BindGroup, Binding, Buffer, BufferUsage, ComputePipeline, Draw, PixelFormat, RenderDescriptor,
    RenderPipeline, Shader, Stage, Target,
};
use yoake_wasm_sdk::full::client::{ClientMod, et, gpu};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct Demo {
    state: Option<State>,
    readback: Option<gpu::Readback>,
    verified: bool,
}

struct State {
    format: PixelFormat,
    mesh: RenderPipeline,
    mesh_group: BindGroup,
    vertices: Buffer,
    positions: Buffer,
    compute: Option<(ComputePipeline, BindGroup)>,
    particles: Option<(RenderPipeline, BindGroup)>,
    effect: Option<RenderPipeline>,
}

fn render(
    shader: &Shader,
    stride: u64,
    attributes: &[gpu::Attribute],
    format: PixelFormat,
    depth: bool,
    blend: bool,
) -> RenderPipeline {
    RenderPipeline::create(&RenderDescriptor {
        shader,
        vertex_entry: "vertex",
        fragment_entry: "fragment",
        stride,
        attributes,
        format,
        depth,
        depth_write: depth && !blend,
        blend,
        cull: false,
        lines: false,
    })
    .unwrap()
}

fn shader(path: &str, body: &str) -> Shader {
    Shader::from_file(path, &format!("{}{body}", gpu::ENGINE_WGSL)).unwrap()
}

fn bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

impl State {
    fn new(format: PixelFormat) -> Self {
        let mesh_shader = shader(
            "mesh.wgsl",
            r#"
@group(0) @binding(0) var checker: texture_2d<f32>;
@group(0) @binding(1) var checker_sampler: sampler;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32>, @location(1) uv: vec2<f32> }
@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) color: vec3<f32>, @location(2) uv: vec2<f32>) -> Vertex {
    let world = engine.inverse_view_projection * vec4<f32>(position, 1.0);
    return Vertex(engine.view_projection * world, color, uv);
}
@fragment fn fragment(input: Vertex) -> @location(0) vec4<f32> {
    return textureSample(checker, checker_sampler, input.uv) * vec4<f32>(input.color, 1.0);
}
"#,
        );
        let mesh = render(
            &mesh_shader,
            32,
            &[
                gpu::Attribute {
                    location: 0,
                    offset: 0,
                    components: 3,
                },
                gpu::Attribute {
                    location: 1,
                    offset: 12,
                    components: 3,
                },
                gpu::Attribute {
                    location: 2,
                    offset: 24,
                    components: 2,
                },
            ],
            format,
            true,
            false,
        );
        let vertices = Buffer::create(96, BufferUsage::VERTEX | BufferUsage::COPY_DST).unwrap();
        vertices
            .write(
                Stage::WorldOpaque,
                0,
                &bytes(&[
                    -0.65, -0.45, 0.5, 1.0, 0.2, 0.2, 0.0, 1.0, 0.65, -0.45, 0.5, 0.2, 1.0, 0.2,
                    1.0, 1.0, 0.0, 0.65, 0.5, 0.2, 0.4, 1.0, 0.5, 0.0,
                ]),
            )
            .unwrap();
        let texture = gpu::Texture::create(&gpu::TextureDescriptor {
            width: 2,
            height: 2,
            depth: 1,
            mips: 1,
            volume: false,
            format: PixelFormat::Rgba8,
            usage: gpu::TextureUsage::COPY_DST | gpu::TextureUsage::SAMPLED,
        })
        .unwrap();
        texture
            .write(
                Stage::WorldOpaque,
                0,
                (0, 0, 0),
                (2, 2, 1),
                &[
                    255, 255, 255, 255, 70, 70, 70, 255, 70, 70, 70, 255, 255, 255, 255, 255,
                ],
            )
            .unwrap();
        let sampler = gpu::Sampler::create(false, true).unwrap();
        let mesh_group = BindGroup::create(
            &mesh,
            0,
            &[
                Binding::Texture {
                    binding: 0,
                    texture: &texture,
                },
                Binding::Sampler {
                    binding: 1,
                    sampler: &sampler,
                },
            ],
        )
        .unwrap();
        let positions =
            Buffer::create(64 * 32, BufferUsage::STORAGE | BufferUsage::COPY_DST).unwrap();
        let mut particles = Vec::new();
        for index in 0..64 {
            particles.extend_from_slice(&[
                (index % 8) as f32 * 0.2 - 0.7,
                (index / 8) as f32 * 0.2 - 0.7,
                0.4,
                1.0,
                0.0,
                0.07,
                0.0,
                0.0,
            ]);
        }
        positions
            .write(Stage::WorldTranslucent, 0, &bytes(&particles))
            .unwrap();
        Self {
            format,
            mesh,
            mesh_group,
            vertices,
            positions,
            compute: None,
            particles: None,
            effect: None,
        }
    }

    fn prepare(&mut self) -> bool {
        if self.compute.is_none() {
            let compute_shader = shader(
                "compute.wgsl",
                r#"
struct Particle { position: vec4<f32>, velocity: vec4<f32> }
@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@compute @workgroup_size(64) fn update(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    particles[index].position.y += particles[index].velocity.y * engine.viewport_delta.z;
    if particles[index].position.y > 0.8 { particles[index].position.y = -0.8; }
}
"#,
            );
            let compute = ComputePipeline::create(&compute_shader, "update").unwrap();
            let compute_group = BindGroup::create(
                &compute,
                0,
                &[Binding::Buffer {
                    binding: 0,
                    range: self.positions.range(0, 64 * 32),
                }],
            )
            .unwrap();
            self.compute = Some((compute, compute_group));
            return false;
        }
        if self.particles.is_none() {
            let particle_shader = shader(
                "particles.wgsl",
                r#"
struct Particle { position: vec4<f32>, velocity: vec4<f32> }
@group(0) @binding(0) var<storage, read> particles: array<Particle>;
@vertex fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    return particles[instance_index].position + vec4<f32>(corners[vertex_index] * 0.012, 0.0, 0.0);
}
@fragment fn fragment() -> @location(0) vec4<f32> { return vec4<f32>(0.3, 0.8, 1.0, 0.65); }
"#,
            );
            let particles = render(&particle_shader, 0, &[], self.format, true, true);
            let particle_group = BindGroup::create(
                &particles,
                0,
                &[Binding::Buffer {
                    binding: 0,
                    range: self.positions.range(0, 64 * 32),
                }],
            )
            .unwrap();
            self.particles = Some((particles, particle_group));
            return false;
        }
        if self.effect.is_none() {
            let effect_shader = shader(
                "effect.wgsl",
                r#"
@group(3) @binding(3) var world_depth: texture_depth_2d;
@group(3) @binding(4) var scene: texture_2d<f32>;
@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4<f32>(corners[index], 0.0, 1.0);
}
@fragment fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let coordinate = vec2<i32>(position.xy);
    let depth = textureLoad(world_depth, coordinate, 0);
    let color = textureLoad(scene, coordinate, 0);
    return vec4<f32>(mix(color.rgb, color.rgb * vec3<f32>(0.85, 0.95, 1.0), (1.0 - depth) * 0.2), color.a);
}
"#,
            );
            self.effect = Some(render(&effect_shader, 0, &[], self.format, false, false));
            return false;
        }
        true
    }

    fn draw(&self) {
        self.mesh
            .draw(
                Stage::WorldOpaque,
                &Draw {
                    count: 3,
                    instances: 1,
                    vertex: Some(self.vertices.range(0, 96)),
                    index: None,
                    index32: false,
                    groups: &[&self.mesh_group],
                },
                &Target::default(),
            )
            .unwrap();
        let (compute, compute_group) = self.compute.as_ref().unwrap();
        compute
            .dispatch(Stage::WorldTranslucent, &[compute_group], (1, 1, 1))
            .unwrap();
        let (particles, particle_group) = self.particles.as_ref().unwrap();
        particles
            .draw(
                Stage::WorldTranslucent,
                &Draw {
                    count: 3,
                    instances: 1,
                    vertex: None,
                    index: None,
                    index32: false,
                    groups: &[particle_group],
                },
                &Target::default(),
            )
            .unwrap();
        self.effect
            .as_ref()
            .unwrap()
            .draw(
                Stage::AfterWorld,
                &Draw {
                    count: 3,
                    instances: 1,
                    vertex: None,
                    index: None,
                    index32: false,
                    groups: &[],
                },
                &Target::default(),
            )
            .unwrap();
    }
}

impl Demo {
    fn verify(&mut self, frame: &gpu::FrameInfo) {
        assert_eq!(frame.version, 1);
        assert_eq!(frame.view_projection.len(), 16);
        let owner_buffer = Buffer::create(
            16,
            BufferUsage::STORAGE | BufferUsage::COPY_SRC | BufferUsage::COPY_DST,
        )
        .unwrap();
        assert!(owner_buffer.write(Stage::WorldOpaque, 16, &[0; 4]).is_err());
        assert!(Buffer::create(256 * 1024 * 1024 + 1, BufferUsage::COPY_DST).is_err());
        let compute_shader = Shader::create(
            r#"
@group(0) @binding(0) var<storage, read_write> values: array<u32>;
@compute @workgroup_size(1) fn main() { values[0] = 42u; }
"#,
        )
        .unwrap();
        let compute = ComputePipeline::create(&compute_shader, "main").unwrap();
        let group = BindGroup::create(
            &compute,
            0,
            &[Binding::Buffer {
                binding: 0,
                range: owner_buffer.range(0, 16),
            }],
        )
        .unwrap();
        assert!(
            compute
                .dispatch(Stage::WorldOpaque, &[&group], (65537, 1, 1))
                .is_err()
        );
        compute
            .dispatch(Stage::WorldOpaque, &[&group], (1, 1, 1))
            .unwrap();
        let destination =
            Buffer::create(16, BufferUsage::COPY_SRC | BufferUsage::COPY_DST).unwrap();
        owner_buffer
            .copy_to(Stage::WorldOpaque, 0, &destination, 0, 16)
            .unwrap();
        self.readback = Some(destination.read(Stage::WorldOpaque, 0, 4).unwrap());
    }
}

impl ClientMod for Demo {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::GPU_FRAME, hook::COMPUTE_FOV]
    }

    fn gpu_frame(&mut self) {
        let frame = gpu::frame().unwrap();
        if cfg!(feature = "verify") {
            if self.readback.is_none() && !self.verified {
                self.verify(&frame);
                assert!(!cfg!(feature = "trap"), "intentional GPU guest trap");
            } else if let Some(readback) = &mut self.readback {
                if let Some(bytes) = readback.poll().unwrap() {
                    assert_eq!(bytes, 42u32.to_le_bytes());
                    assert!(readback.poll().is_err());
                    let malformed = Shader::create("@compute fn broken(").unwrap_err();
                    assert!(malformed.line > 0 && malformed.column > 0);
                    self.verified = true;
                    self.readback = None;
                }
            }
            return;
        }
        if let Some(format) = frame.world_format {
            if self
                .state
                .as_ref()
                .is_none_or(|state| state.format != format)
            {
                self.state = Some(State::new(format));
                return;
            }
            let state = self.state.as_mut().unwrap();
            if state.prepare() {
                state.draw();
            }
        }
    }

    fn compute_fov(
        &mut self,
        _partial: f32,
        _camera: Vec<f64>,
        degrees: f32,
    ) -> et::ComputeFovOutcome {
        et::ComputeFovOutcome {
            degrees: Some(if self.verified {
                if cfg!(feature = "reload") { 92.0 } else { 91.0 }
            } else {
                degrees
            }),
        }
    }
}

yoake_wasm_sdk::export_client_full!(Demo);
