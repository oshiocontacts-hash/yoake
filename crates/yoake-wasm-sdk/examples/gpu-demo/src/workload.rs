pub const MESH_VERTEX_COUNT: u32 = 32 * 32 * 6;
pub const VERTEX_STRIDE: u64 = 32;
pub const PARTICLE_COUNT: u32 = 64;

pub const MESH_SHADER: &str = r#"
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
"#;

pub const COMPUTE_SHADER: &str = r#"
struct Particle { position: vec4<f32>, velocity: vec4<f32> }
@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@compute @workgroup_size(64) fn update(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    particles[index].position.y += particles[index].velocity.y * engine.viewport_delta.z;
    if particles[index].position.y > 0.8 { particles[index].position.y = -0.8; }
}
"#;

pub const PARTICLE_SHADER: &str = r#"
struct Particle { position: vec4<f32>, velocity: vec4<f32> }
@group(0) @binding(0) var<storage, read> particles: array<Particle>;
@vertex fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    return particles[instance_index].position + vec4<f32>(corners[vertex_index] * 0.012, 0.0, 0.0);
}
@fragment fn fragment() -> @location(0) vec4<f32> { return vec4<f32>(0.3, 0.8, 1.0, 0.65); }
"#;

pub fn mesh_vertices() -> Vec<f32> {
    let mut vertices = Vec::with_capacity(MESH_VERTEX_COUNT as usize * 8);
    for row in 0..32 {
        for column in 0..32 {
            for (offset_x, offset_y) in [(0, 0), (1, 0), (0, 1), (0, 1), (1, 0), (1, 1)] {
                let horizontal = (column + offset_x) as f32 / 32.0;
                let vertical = (row + offset_y) as f32 / 32.0;
                vertices.extend_from_slice(&[
                    horizontal * 1.3 - 0.65,
                    vertical * 1.3 - 0.65,
                    0.5,
                    0.3 + horizontal * 0.7,
                    0.3 + vertical * 0.7,
                    0.8,
                    horizontal * 8.0,
                    vertical * 8.0,
                ]);
            }
        }
    }
    vertices
}

pub fn particle_positions() -> Vec<f32> {
    let mut particles = Vec::with_capacity(PARTICLE_COUNT as usize * 8);
    for index in 0..PARTICLE_COUNT {
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
    particles
}
