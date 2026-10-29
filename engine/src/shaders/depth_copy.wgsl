@group(0) @binding(0) var scene_depth: texture_depth_2d;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct FragmentOutput {
    @location(0) depth_copy: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let position = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var output: VertexOutput;
    output.position = vec4<f32>(position[vertex_index], 0.0, 1.0);
    output.uv = position[vertex_index] * vec2<f32>(0.5, -0.5) + 0.5;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> FragmentOutput {
    let size = textureDimensions(scene_depth);
    let pixel = clamp(
        vec2<i32>(input.uv * vec2<f32>(size)),
        vec2<i32>(0),
        vec2<i32>(size) - vec2<i32>(1)
    );
    return FragmentOutput(textureLoad(scene_depth, pixel, 0));
}
