struct EngineFrame {
    header: vec4<u32>,
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    previous_view_projection: mat4x4<f32>,
    jittered_view_projection: mat4x4<f32>,
    inverse_jittered_view_projection: mat4x4<f32>,
    camera_time: vec4<f32>,
    viewport_delta: vec4<f32>,
    sun: vec4<f32>,
    moon: vec4<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    jitter: vec4<f32>,
}
@group(3) @binding(0) var<uniform> engine: EngineFrame;
