struct Camera {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    position_time: vec4<f32>,
    viewport: vec4<f32>,
};

struct VelocitySettings {
    // strength, near falloff, far cutoff, extreme-motion threshold
    params: vec4<f32>,
};

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var motion_vectors: texture_2d<f32>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var<uniform> camera: Camera;
@group(0) @binding(4) var<uniform> settings: VelocitySettings;

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

fn world_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), depth, 1.0);
    let world = camera.inverse_view_projection * ndc;
    return world.xyz / max(world.w, 0.00001);
}

fn depth_compatible(center_depth: f32, sample_depth: f32) -> f32 {
    // Reverse-Z: a larger value is closer. Do not pull a farther background
    // across a nearer foreground origin.
    let foreground_leak = center_depth - sample_depth;
    return 1.0 - smoothstep(0.0015, 0.012, foreground_leak);
}

@fragment
fn fs_compensate(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = textureDimensions(scene_color);
    let pixel = clamp_pixel(vec2<i32>(input.position.xy), size);
    let center = textureLoad(scene_color, pixel, 0);
    let depth = textureLoad(scene_depth, pixel, 0);
    if depth <= 0.00001 || settings.params.x <= 0.0001 {
        return center;
    }
    let motion = textureLoad(motion_vectors, pixel, 0).rg;
    let speed = length(motion);
    if speed <= 0.0002 {
        return center;
    }
    let distance_to_camera = length(world_position(input.uv, depth) - camera.position_time.xyz);
    let near_field = 1.0 - smoothstep(settings.params.y, settings.params.z, distance_to_camera);
    let extreme_motion = smoothstep(settings.params.w, settings.params.w * 2.0, speed);
    let perceptual_mask = max(near_field, extreme_motion);
    let trail = -motion * settings.params.x * perceptual_mask;
    var accumulated = center.rgb * 1.7;
    var weight_sum = 1.7;
    // Uniform, single-direction samples leave a trail behind the moving origin.
    for (var index = 1; index <= 8; index += 1) {
        let step = f32(index) / 8.0;
        let sample_uv = input.uv + trail * step;
        if any(sample_uv < vec2<f32>(0.0)) || any(sample_uv > vec2<f32>(1.0)) {
            continue;
        }
        let sample_pixel = clamp_pixel(vec2<i32>(sample_uv * vec2<f32>(size)), size);
        let sample_depth = textureLoad(scene_depth, sample_pixel, 0);
        let compatibility = depth_compatible(depth, sample_depth);
        let weight = (1.0 - step * 0.58) * compatibility;
        accumulated += textureLoad(scene_color, sample_pixel, 0).rgb * weight;
        weight_sum += weight;
    }
    return vec4<f32>(accumulated / max(weight_sum, 0.0001), center.a);
}

@fragment
fn fs_smooth(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = textureDimensions(scene_color);
    let pixel = clamp_pixel(vec2<i32>(input.position.xy), size);
    let center = textureLoad(scene_color, pixel, 0);
    let depth = textureLoad(scene_depth, pixel, 0);
    let motion = textureLoad(motion_vectors, pixel, 0).rg;
    let speed = length(motion);
    if depth <= 0.00001 || speed <= 0.0002 {
        return center;
    }
    let direction = motion / speed;
    let perpendicular = vec2<f32>(-direction.y, direction.x) / vec2<f32>(size);
    var accumulated = center.rgb * 2.0;
    var weight_sum = 2.0;
    for (var offset = -2; offset <= 2; offset += 1) {
        if offset == 0 { continue; }
        let sample_uv = input.uv + perpendicular * f32(offset);
        let sample_pixel = clamp_pixel(vec2<i32>(sample_uv * vec2<f32>(size)), size);
        let sample_depth = textureLoad(scene_depth, sample_pixel, 0);
        let weight = (1.0 - abs(f32(offset)) * 0.22) * depth_compatible(depth, sample_depth);
        accumulated += textureLoad(scene_color, sample_pixel, 0).rgb * weight;
        weight_sum += weight;
    }
    return vec4<f32>(accumulated / max(weight_sum, 0.0001), center.a);
}
