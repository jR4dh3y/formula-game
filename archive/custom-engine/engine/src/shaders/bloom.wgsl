@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;

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

const BLOOM_THRESHOLD: f32 = 1.0;
const BLOOM_SOFT_KNEE: f32 = 0.75;
const BLOOM_LUMINANCE_CAP: f32 = 3.0;

fn load_clamped(pixel: vec2<i32>) -> vec3<f32> {
    let size = textureDimensions(source_tex);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
    return textureLoad(source_tex, clamped, 0).rgb;
}

@fragment
fn fs_extract(input: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.position.xy);
    let base = pixel * 4;
    var color = vec3<f32>(0.0);
    for (var y = 0; y < 4; y += 1) {
        for (var x = 0; x < 4; x += 1) {
            color += load_clamped(base + vec2<i32>(x, y));
        }
    }
    color *= 1.0 / 16.0;
    let source_luminance = luminance(color);
    let knee = clamp(
        source_luminance - BLOOM_THRESHOLD + BLOOM_SOFT_KNEE,
        0.0,
        BLOOM_SOFT_KNEE * 2.0
    );
    let soft_contribution = knee * knee / (BLOOM_SOFT_KNEE * 4.0 + 0.0001);
    let contribution = max(soft_contribution, source_luminance - BLOOM_THRESHOLD)
        / max(source_luminance, 0.0001);
    let capped_color = color * min(1.0, BLOOM_LUMINANCE_CAP / max(source_luminance, 0.0001));
    return vec4<f32>(capped_color * contribution, 1.0);
}

fn gaussian_blur(uv: vec2<f32>, axis: vec2<f32>) -> vec3<f32> {
    let texel_size = 1.0 / vec2<f32>(textureDimensions(source_tex));
    let offsets = array<f32, 4>(
        1.44028635, 3.36354746, 5.29317778, 7.23147522
    );
    let weights = array<f32, 4>(
        0.26334800, 0.12211849, 0.03057316, 0.00412237
    );
    var color = textureSampleLevel(source_tex, linear_sampler, uv, 0.0).rgb * 0.15967594;
    for (var i = 0u; i < 4u; i += 1u) {
        let sample_offset = axis * texel_size * offsets[i];
        color += (
            textureSampleLevel(source_tex, linear_sampler, uv - sample_offset, 0.0).rgb
            + textureSampleLevel(source_tex, linear_sampler, uv + sample_offset, 0.0).rgb
        ) * weights[i];
    }
    return color;
}

@fragment
fn fs_blur_horizontal(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(gaussian_blur(input.uv, vec2<f32>(1.0, 0.0)), 1.0);
}

@fragment
fn fs_blur_vertical(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(gaussian_blur(input.uv, vec2<f32>(0.0, 1.0)), 1.0);
}
