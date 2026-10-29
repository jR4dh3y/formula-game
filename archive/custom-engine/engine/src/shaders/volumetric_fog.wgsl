struct Frame {
    dimensions: vec4<f32>,
    counters: vec4<u32>,
};

struct Camera {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    position_time: vec4<f32>,
    viewport: vec4<f32>,
};

struct Lights {
    directional_direction_count: vec4<f32>,
    directional_color_intensity: vec4<f32>,
    ambient: vec4<f32>,
    sky: vec4<f32>,
    weather: vec4<f32>,
    environment: vec4<f32>,
    environment_sh: array<vec4<f32>, 9>,
    local_position_radius: array<vec4<f32>, 6>,
    local_color_intensity: array<vec4<f32>, 6>,
    local_direction_outer: array<vec4<f32>, 6>,
    local_inner_shadow: array<vec4<f32>, 6>,
};

@group(0) @binding(0) var depth_copy: texture_2d<f32>;
@group(0) @binding(1) var fog_output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> frame: Frame;
@group(0) @binding(3) var<uniform> lights: Lights;
@group(0) @binding(4) var<uniform> camera: Camera;
@group(0) @binding(5) var skybox: texture_2d<f32>;
@group(0) @binding(6) var skybox_sampler: sampler;

const INV_PI: f32 = 0.31830988618;
const INV_TAU: f32 = 0.15915494309;

fn safe_normalize(value: vec3<f32>) -> vec3<f32> {
    let len = length(value);
    return select(vec3<f32>(0.0), value / len, len > 0.0001);
}

fn environment_radiance(uv: vec2<f32>) -> vec3<f32> {
    let clip_position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    let far_world_h = camera.inverse_view_projection * clip_position;
    let far_world = far_world_h.xyz / max(far_world_h.w, 0.00001);
    let ray = safe_normalize(far_world - camera.position_time.xyz);
    let panorama_uv = vec2<f32>(
        fract(0.5 + (atan2(ray.z, ray.x) + lights.environment.y) * INV_TAU),
        clamp(0.5 - asin(clamp(ray.y, -1.0, 1.0)) * INV_PI, 0.0, 1.0)
    );
    return textureSampleLevel(skybox, skybox_sampler, panorama_uv, 2.0).rgb
        * exp2(lights.environment.x);
}

fn hash(pixel: vec2<u32>, frame_index: u32) -> f32 {
    var state = pixel.x * 1973u + pixel.y * 9277u + frame_index * 26699u + 0x9e3779b9u;
    state = (state ^ (state >> 16u)) * 0x7feb352du;
    state = (state ^ (state >> 15u)) * 0x846ca68bu;
    state = state ^ (state >> 16u);
    return f32(state & 0x00ffffffu) / 16777215.0;
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let output_size = textureDimensions(fog_output);
    if any(invocation.xy >= output_size) {
        return;
    }
    let source_size = textureDimensions(depth_copy);
    let source_pixel = min(
        vec2<u32>((vec2<f32>(invocation.xy) + 0.5) * vec2<f32>(source_size) / vec2<f32>(output_size)),
        source_size - vec2<u32>(1u)
    );
    let source_uv = (vec2<f32>(source_pixel) + 0.5) / vec2<f32>(source_size);
    let scene_depth = textureLoad(depth_copy, vec2<i32>(source_pixel), 0).r;
    let distance_fog = smoothstep(0.70, 1.0, 1.0 - scene_depth);
    let rainfall = lights.weather.x;
    let gust_noise = hash(invocation.xy, frame.counters.x);
    let animated_noise = hash(invocation.yx + vec2<u32>(17u, 41u), frame.counters.x + 13u);
    let density = clamp(
        distance_fog * (0.030 + rainfall * 0.085)
            + (gust_noise - 0.5) * 0.007
            + animated_noise * 0.004,
        0.0,
        0.18
    );
    let moon = lights.directional_color_intensity.rgb * lights.directional_color_intensity.w * 0.010;
    let view_environment = environment_radiance(source_uv);
    let fog_color = view_environment * 0.20
        + lights.sky.rgb * 0.18
        + lights.ambient.rgb * 0.40
        + moon
        + vec3<f32>(0.003, 0.004, 0.007);
    textureStore(fog_output, vec2<i32>(invocation.xy), vec4<f32>(fog_color, density));
}
