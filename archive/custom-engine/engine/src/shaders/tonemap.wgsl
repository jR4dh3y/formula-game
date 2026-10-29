@group(0) @binding(0) var scene_hdr: texture_2d<f32>;
@group(0) @binding(1) var bloom_tex: texture_2d<f32>;
@group(0) @binding(2) var volumetric_fog: texture_2d<f32>;
@group(0) @binding(3) var exposure_white_balance: texture_2d<f32>;

const BLOOM_INTENSITY: f32 = 0.10;

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

fn gt_tonemap(x: vec3<f32>) -> vec3<f32> {
    let peak = 1.0;
    let contrast = 1.0;
    let linear_start = 0.22;
    let linear_length = 0.4;
    let toe_power = 1.33;
    let black = 0.0;
    let l0 = ((peak - linear_start) * linear_length) / contrast;
    let shoulder_start = linear_start + l0;
    let shoulder_value = linear_start + contrast * l0;
    let shoulder_scale = (contrast * peak) / (peak - shoulder_value);
    let shoulder_power = -shoulder_scale / peak;
    let toe_weight = vec3<f32>(1.0) - smoothstep(vec3<f32>(0.0), vec3<f32>(linear_start), x);
    let shoulder_weight = step(vec3<f32>(shoulder_start), x);
    let linear_weight = vec3<f32>(1.0) - toe_weight - shoulder_weight;
    let toe = linear_start * pow(max(x / linear_start, vec3<f32>(0.0)), vec3<f32>(toe_power)) + black;
    let linear = linear_start + contrast * (x - linear_start);
    let shoulder = peak - (peak - shoulder_value) * exp(shoulder_power * (x - shoulder_start));
    return toe * toe_weight + linear * linear_weight + shoulder * shoulder_weight;
}

fn load_hdr(pixel: vec2<i32>) -> vec3<f32> {
    let size = textureDimensions(scene_hdr);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
    return textureLoad(scene_hdr, clamped, 0).rgb;
}

fn load_bloom(pixel: vec2<i32>) -> vec3<f32> {
    let size = textureDimensions(bloom_tex);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
    return textureLoad(bloom_tex, clamped, 0).rgb;
}

fn sample_bloom(uv: vec2<f32>) -> vec3<f32> {
    let size = textureDimensions(bloom_tex);
    let coord = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(coord));
    let f = fract(coord);
    let a = load_bloom(base);
    let b = load_bloom(base + vec2<i32>(1, 0));
    let c = load_bloom(base + vec2<i32>(0, 1));
    let d = load_bloom(base + vec2<i32>(1, 1));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn load_fog(pixel: vec2<i32>) -> vec4<f32> {
    let size = textureDimensions(volumetric_fog);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
    return textureLoad(volumetric_fog, clamped, 0);
}

fn sample_fog(uv: vec2<f32>) -> vec4<f32> {
    let size = textureDimensions(volumetric_fog);
    let coord = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(coord));
    let f = fract(coord);
    let a = load_fog(base);
    let b = load_fog(base + vec2<i32>(1, 0));
    let c = load_fog(base + vec2<i32>(0, 1));
    let d = load_fog(base + vec2<i32>(1, 1));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.position.xy);
    let fog = sample_fog(input.uv);
    let lit = load_hdr(pixel) + sample_bloom(input.uv) * BLOOM_INTENSITY;
    let hdr = mix(lit, fog.rgb, clamp(fog.a, 0.0, 0.16));
    let calibration = textureLoad(exposure_white_balance, vec2<i32>(0), 0);
    let calibrated_hdr = hdr * calibration.rgb * calibration.a;
    return vec4<f32>(gt_tonemap(max(calibrated_hdr, vec3<f32>(0.0))), 1.0);
}
