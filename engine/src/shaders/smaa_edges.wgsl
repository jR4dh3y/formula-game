struct AntiAlias {
    previous_view_projection: mat4x4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var current_color: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var<uniform> aa: AntiAlias;
@group(0) @binding(3) var sample_coverage: texture_2d<f32>;

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

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn clamp_pixel(pixel: vec2<i32>, size: vec2<u32>) -> vec2<i32> {
    return clamp(pixel, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
}

fn load_color(pixel: vec2<i32>, size: vec2<u32>) -> vec3<f32> {
    return textureLoad(current_color, clamp_pixel(pixel, size), 0).rgb;
}

fn load_depth(pixel: vec2<i32>, size: vec2<u32>) -> f32 {
    return textureLoad(scene_depth, clamp_pixel(pixel, size), 0);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = textureDimensions(current_color);
    let pixel = vec2<i32>(min(vec2<u32>(input.position.xy), size - vec2<u32>(1u)));
    let center = load_color(pixel, size);
    let center_luma = luminance(center);
    let center_depth = load_depth(pixel, size);
    let right_luma = luminance(load_color(pixel + vec2<i32>(1, 0), size));
    let down_luma = luminance(load_color(pixel + vec2<i32>(0, 1), size));
    let right_depth = load_depth(pixel + vec2<i32>(1, 0), size);
    let down_depth = load_depth(pixel + vec2<i32>(0, 1), size);
    let color_threshold = aa.params.y;
    let depth_threshold = 0.0018;
    let coverage = textureLoad(sample_coverage, pixel, 0).r;
    let coverage_edge = 1.0 - abs(coverage * 2.0 - 1.0);
    let horizontal_edge = max(
        smoothstep(color_threshold, color_threshold * 2.5, abs(center_luma - right_luma)),
        smoothstep(depth_threshold, depth_threshold * 5.0, abs(center_depth - right_depth))
    );
    let vertical_edge = max(
        smoothstep(color_threshold, color_threshold * 2.5, abs(center_luma - down_luma)),
        smoothstep(depth_threshold, depth_threshold * 5.0, abs(center_depth - down_depth))
    );
    return vec4<f32>(max(horizontal_edge, coverage_edge), max(vertical_edge, coverage_edge), coverage_edge, 1.0);
}
