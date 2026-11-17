fn post(input: PostInput) -> vec4<f32> {
    if input.depth == 0.0 {
        return input.color;
    }
    let position = view_position(input.uv, input.depth);
    let shape = voxel_at(position) & 15u;
    var tint = vec3<f32>(0.0);
    if shape != 0u {
        tint = vec3<f32>(0.05, 0.12, 0.08);
    }
    for (var index = 0u; index < frame.light_count; index += 1u) {
        let light = light_list[index];
        let distance = length(light.position - position);
        let strength = max(1.0 - distance / max(light.radius, 1.0), 0.0);
        tint += light.color * (light.level / 15.0) * strength * 0.08;
    }
    return vec4<f32>(input.color.rgb + tint, input.color.a);
}
