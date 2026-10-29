struct AntiAlias {
    previous_view_projection: mat4x4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var current_color: texture_2d<f32>;
@group(0) @binding(1) var edge_mask: texture_2d<f32>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var<uniform> aa: AntiAlias;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
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

fn clamp_pixel(pixel: vec2<i32>, size: vec2<u32>) -> vec2<i32> {
    return clamp(pixel, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
}

fn load_edges(pixel: vec2<i32>, size: vec2<u32>) -> vec2<f32> {
    return textureLoad(edge_mask, clamp_pixel(pixel, size), 0).rg;
}

fn load_depth(pixel: vec2<i32>, size: vec2<u32>) -> f32 {
    return textureLoad(scene_depth, clamp_pixel(pixel, size), 0);
}

fn edge_span(pixel: vec2<i32>, size: vec2<u32>, dir: vec2<i32>, channel: u32) -> f32 {
    var span = 0.0;
    var falloff = 1.0;
    for (var step = 1; step <= 6; step += 1) {
        let edge = load_edges(pixel + dir * step, size)[channel];
        span += edge * falloff;
        falloff *= 0.72;
    }
    return span / 2.72;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = textureDimensions(edge_mask);
    let pixel = vec2<i32>(min(vec2<u32>(input.position.xy), size - vec2<u32>(1u)));
    let edge = load_edges(pixel, size);
    let center_depth = load_depth(pixel, size);
    let depth_gate = 1.0 - smoothstep(0.018, 0.12, abs(center_depth - load_depth(pixel + vec2<i32>(1, 1), size)));
    let left_span = edge_span(pixel, size, vec2<i32>(-1, 0), 0u);
    let right_span = edge_span(pixel, size, vec2<i32>(1, 0), 0u);
    let up_span = edge_span(pixel, size, vec2<i32>(0, -1), 1u);
    let down_span = edge_span(pixel, size, vec2<i32>(0, 1), 1u);
    let horizontal_weight = clamp(edge.x * (0.23 + (left_span + right_span) * 0.18), 0.0, 0.62);
    let vertical_weight = clamp(edge.y * (0.23 + (up_span + down_span) * 0.18), 0.0, 0.62);
    let strongest = max(horizontal_weight, vertical_weight) * depth_gate;
    return vec4<f32>(horizontal_weight * depth_gate, vertical_weight * depth_gate, strongest, 1.0);
}
