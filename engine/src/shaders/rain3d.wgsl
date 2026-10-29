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

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var rain_streak_detail: texture_2d<f32>;
@group(0) @binding(2) var rain_streak_sampler: sampler;
@group(0) @binding(3) var shadow_map: texture_depth_2d;
@group(0) @binding(4) var shadow_sampler: sampler_comparison;
@group(0) @binding(5) var<uniform> light_camera: Camera;
@group(0) @binding(6) var<uniform> lights: Lights;
@group(0) @binding(7) var skybox: texture_2d<f32>;
@group(0) @binding(8) var skybox_sampler: sampler;
const RAIN_GRID_X: u32 = 64u;
const RAIN_GRID_Z: u32 = 64u;
const RAIN_BOUNDS_MIN: vec3<f32> = vec3<f32>(-18.5, 0.14, -82.0);
const RAIN_BOUNDS_SIZE: vec3<f32> = vec3<f32>(37.0, 13.5, 100.0);
const INV_PI: f32 = 0.31830988618;
const INV_TAU: f32 = 0.15915494309;


struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) seed: f32,
    @location(2) alpha: f32,
    @location(3) world_position: vec3<f32>,
    @location(4) rain_axis: vec3<f32>,
    @location(5) surface_normal: vec3<f32>,
    @location(6) boundary_fade: f32,
    @location(7) camera_distance: f32,
};

fn hash11(value: f32) -> f32 {
    return fract(sin(value * 127.1) * 43758.5453123);
}

fn hash31(seed: f32) -> vec3<f32> {
    return vec3<f32>(
        hash11(seed + 3.17),
        hash11(seed + 19.31),
        hash11(seed + 47.77)
    );
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
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

fn safe_normalize(value: vec3<f32>) -> vec3<f32> {
    let len = length(value);
    return select(vec3<f32>(0.0), value / len, len > 0.0001);
}

fn rain_wind_direction() -> vec3<f32> {
    return safe_normalize(vec3<f32>(
        -0.22 - lights.weather.z * 0.12,
        -1.0,
        0.08 + lights.weather.z * 0.05
    ));
}

fn environment_radiance(direction: vec3<f32>) -> vec3<f32> {
    let ray = safe_normalize(direction);
    let panorama_uv = vec2<f32>(
        fract(0.5 + (atan2(ray.z, ray.x) + lights.environment.y) * INV_TAU),
        clamp(0.5 - asin(clamp(ray.y, -1.0, 1.0)) * INV_PI, 0.0, 1.0)
    );
    return textureSampleLevel(skybox, skybox_sampler, panorama_uv, 1.5).rgb
        * exp2(lights.environment.x);
}

fn wrap_rain_xz(xz: vec2<f32>) -> vec2<f32> {
    let local = fract((xz - RAIN_BOUNDS_MIN.xz) / RAIN_BOUNDS_SIZE.xz);
    return RAIN_BOUNDS_MIN.xz + local * RAIN_BOUNDS_SIZE.xz;
}

fn tangent_from_axis(axis: vec3<f32>) -> vec3<f32> {
    let reference = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(axis.z) > 0.92);
    return safe_normalize(cross(reference, axis));
}

fn prism_local_vertex(vertex_index: u32) -> vec3<f32> {
    let side = vertex_index / 6u;
    let side_vertex = vertex_index % 6u;
    let next_side = (side + 1u) % 6u;
    let radial_index = select(
        side,
        next_side,
        side_vertex == 1u || side_vertex == 4u || side_vertex == 5u
    );
    let axis = select(
        -1.0,
        1.0,
        side_vertex == 2u || side_vertex == 3u || side_vertex == 5u
    );
    let angle = (f32(radial_index) / 6.0) * 6.28318530718;
    return vec3<f32>(cos(angle), sin(angle), axis);
}


fn shadow_visibility(world_position: vec3<f32>) -> f32 {
    let light_clip = light_camera.view_projection * vec4<f32>(world_position, 1.0);
    let light_ndc = light_clip.xyz / max(light_clip.w, 0.00001);
    let uv = light_ndc.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || light_ndc.z <= 0.0 || light_ndc.z >= 1.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    var visibility = 0.0;
    var total_weight = 0.0;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = 1.0 / (1.0 + dot(offset, offset) * 0.30);
            visibility += textureSampleCompare(
                shadow_map,
                shadow_sampler,
                uv + offset * texel * 5.0,
                light_ndc.z + 0.0018
            ) * weight;
            total_weight += weight;
        }
    }
    let filtered_visibility = visibility / max(total_weight, 0.0001);
    return mix(filtered_visibility, 1.0, 0.04);
}

fn spot_cone_attenuation(light_index: u32, light_to_surface: vec3<f32>) -> f32 {
    let flags = lights.local_inner_shadow[light_index];
    if flags.w < 0.5 {
        return 1.0;
    }
    let direction_outer = lights.local_direction_outer[light_index];
    let spot_direction = safe_normalize(direction_outer.xyz);
    let alignment = dot(light_to_surface, spot_direction);
    let cone = clamp((alignment - direction_outer.w) / max(flags.x - direction_outer.w, 0.0001), 0.0, 1.0);
    let smooth_core = cone * cone * (3.0 - 2.0 * cone);
    let outer_halo = smoothstep(direction_outer.w - 0.20, direction_outer.w + 0.015, alignment) * 0.24;
    return max(smooth_core, outer_halo * (1.0 - smooth_core * 0.45));
}

fn practical_spot_fill(light_index: u32, cone: f32) -> f32 {
    let flags = lights.local_inner_shadow[light_index];
    if flags.w < 0.5 {
        return 1.0;
    }
    let spot_presence = smoothstep(0.015, 0.16, cone);
    return mix(0.26, 1.0, clamp(cone, 0.0, 1.0)) * spot_presence;
}

fn rain_light_response(world_position: vec3<f32>, view_direction: vec3<f32>, rain_axis: vec3<f32>) -> vec3<f32> {
    let directional_fill = environment_radiance(-view_direction);
    var radiance = lights.ambient.rgb * 1.10
        + lights.sky.rgb * 0.24
        + directional_fill * 0.34;
    let moon_direction = safe_normalize(lights.directional_direction_count.xyz);
    if lights.directional_color_intensity.w > 0.0 && length(moon_direction) > 0.0 {
        let moon_side_scatter = 0.22 + 0.78 * (1.0 - abs(dot(moon_direction, rain_axis)));
        radiance += lights.directional_color_intensity.rgb
            * lights.directional_color_intensity.w
            * moon_side_scatter
            * shadow_visibility(world_position);
    }
    let local_count = u32(clamp(lights.directional_direction_count.w, 0.0, 6.0));
    for (var light_index = 0u; light_index < local_count; light_index += 1u) {
        let position_radius = lights.local_position_radius[light_index];
        let to_light = position_radius.xyz - world_position;
        let distance_to_light = length(to_light);
        let light_direction = to_light / max(distance_to_light, 0.0001);
        let light_to_surface = -light_direction;
        let attenuation = pow(clamp(1.0 - distance_to_light / position_radius.w, 0.0, 1.0), 2.0);
        let side_scatter = 0.32 + 0.68 * (1.0 - abs(dot(light_direction, rain_axis)));
        let forward_scatter = pow(max(dot(light_direction, -view_direction), 0.0), 2.0);
        let color_intensity = lights.local_color_intensity[light_index];
        let cone = spot_cone_attenuation(light_index, light_to_surface);
        radiance += color_intensity.rgb
            * color_intensity.w
            * attenuation
            * practical_spot_fill(light_index, cone)
            * (side_scatter + forward_scatter * 0.6);
    }
    return radiance;
}

fn procedural_streak_alpha(uv: vec2<f32>, seed: f32) -> f32 {
    let axis_t = clamp(uv.y, 0.0, 1.0);
    let head = smoothstep(0.0, 0.12, axis_t);
    let tail = 1.0 - smoothstep(0.76, 1.0, axis_t);
    let filament = 0.74 + hash11(seed + floor(axis_t * 17.0)) * 0.26;
    let broken = mix(0.66, 1.0, smoothstep(0.08, 0.92, hash11(seed + floor(axis_t * 7.0) * 19.1)));
    return head * tail * filament * broken;
}

fn generated_streak_detail(uv: vec2<f32>, seed: f32) -> vec3<f32> {
    let vertical_scale = 0.58 + hash11(seed + 17.0) * 0.62;
    let atlas_uv = vec2<f32>(
        uv.x,
        fract(uv.y * vertical_scale + hash11(seed + 23.0))
    );
    let sample = textureSample(rain_streak_detail, rain_streak_sampler, atlas_uv).rgb;
    let value = max(sample.r, max(sample.g, sample.b));
    let filament = smoothstep(0.015, 0.42, value);
    let bead = smoothstep(0.50, 0.96, value);
    return vec3<f32>(filament, bead, value);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32
) -> VertexOutput {
    let local = prism_local_vertex(vertex_index);
    let seed = f32(instance_index) + 1.0;
    let random = hash31(seed);
    let layer = f32(instance_index % 3u);
    let fall_speed = 8.4 + random.z * 5.2 + layer * 0.9;
    let fall_phase = fract(random.y + camera.position_time.w * fall_speed / RAIN_BOUNDS_SIZE.y);
    let fall_direction = rain_wind_direction();
    let cell = vec2<f32>(
        f32(instance_index % RAIN_GRID_X),
        f32((instance_index / RAIN_GRID_X) % RAIN_GRID_Z)
    );
    let cell_jitter = vec2<f32>(hash11(seed + 11.7), hash11(seed + 73.3));
    let top_xz = RAIN_BOUNDS_MIN.xz
        + ((cell + cell_jitter) / vec2<f32>(f32(RAIN_GRID_X), f32(RAIN_GRID_Z))) * RAIN_BOUNDS_SIZE.xz;
    let drift_per_meter = fall_direction.xz / max(-fall_direction.y, 0.15);
    let head_xz = wrap_rain_xz(top_xz + drift_per_meter * RAIN_BOUNDS_SIZE.y * fall_phase);
    let head_height = RAIN_BOUNDS_MIN.y + (1.0 - fall_phase) * RAIN_BOUNDS_SIZE.y;
    let head = vec3<f32>(head_xz.x, head_height, head_xz.y);

    let tangent = tangent_from_axis(fall_direction);
    let bitangent = safe_normalize(cross(fall_direction, tangent));
    let radius = 0.0042 + random.x * 0.0105 + layer * 0.0014;
    let streak_length = 0.26 + random.z * 0.78 + layer * 0.16;
    let radial = tangent * local.x + bitangent * local.y;
    let world_position = head
        + fall_direction * (local.z * streak_length * 0.5)
        + radial * radius;
    let radial_normal = safe_normalize(radial);
    let height_fraction = clamp((head_height - RAIN_BOUNDS_MIN.y) / RAIN_BOUNDS_SIZE.y, 0.0, 1.0);
    let boundary_fade = smoothstep(0.02, 0.12, height_fraction) * (1.0 - smoothstep(0.92, 1.0, height_fraction));

    var output: VertexOutput;
    output.position = camera.view_projection * vec4<f32>(world_position, 1.0);
    output.uv = vec2<f32>(
        fract(hash11(seed + 409.9) + local.x * 0.018 + local.y * 0.014),
        local.z * 0.5 + 0.5
    );
    output.seed = random.y + layer * 1.37;
    output.alpha = (0.040 + random.z * 0.135 + layer * 0.018)
        * (0.46 + hash11(seed + 201.3) * 0.92)
        * lights.weather.x;
    output.world_position = world_position;
    output.rain_axis = fall_direction;
    output.surface_normal = radial_normal;
    output.boundary_fade = boundary_fade;
    output.camera_distance = length(camera.position_time.xyz - world_position);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let streak_alpha = procedural_streak_alpha(input.uv, input.seed);
    let streak_detail = generated_streak_detail(input.uv, input.seed);
    let view_direction = safe_normalize(camera.position_time.xyz - input.world_position);
    let surface_normal = safe_normalize(input.surface_normal);
    let radiance = rain_light_response(input.world_position, view_direction, input.rain_axis);
    let moon_direction = safe_normalize(lights.directional_direction_count.xyz);
    let normal_glint = pow(max(dot(reflect(-view_direction, surface_normal), moon_direction), 0.0), 10.0)
        * lights.directional_color_intensity.w;
    let edge_transmission = 1.0 - abs(dot(surface_normal, view_direction));
    let light_mask = clamp(
        0.08 + smoothstep(0.025, 0.50, luminance(radiance)) * 0.52 + smoothstep(0.02, 0.52, normal_glint) * 0.42,
        0.06,
        0.92
    );
    let depth_presence = 1.0 - smoothstep(18.0, 31.0, input.camera_distance);
    let near_presence = 1.0 - smoothstep(2.2, 8.5, input.camera_distance);
    let depth_breakup = 0.48 + hash11(floor(input.camera_distance * 0.72) + input.seed * 5.1) * 0.70;
    let segment_breakup = mix(0.34, 1.12, hash11(input.seed * 31.7 + floor(input.uv.y * 11.0) * 3.9));
    let generated_alpha = 0.36 + streak_detail.x * 0.88 + streak_detail.y * 0.42;
    let alpha = clamp(
        streak_alpha * segment_breakup * generated_alpha * input.alpha * input.boundary_fade * depth_presence * depth_breakup * (0.40 + near_presence * 0.64) * (0.46 + edge_transmission * 0.54) * light_mask,
        0.0,
        0.38
    );
    let water_tint = vec3<f32>(0.13, 0.17, 0.22);
    let depth_haze = smoothstep(8.0, 26.0, input.camera_distance);
    let bead_glint = streak_detail.y * (0.42 + near_presence * 0.58) * (0.48 + edge_transmission * 0.52);
    let lit_water = mix(water_tint, radiance * (0.34 + bead_glint * 0.26), vec3<f32>(0.62))
        * (0.50 + edge_transmission * 0.34 + streak_alpha * 0.16 + near_presence * 0.12 + bead_glint * 0.28);
    let hdr_color = mix(lit_water, lights.sky.rgb * 0.16, depth_haze * 0.28);
    return vec4<f32>(max(hdr_color, vec3<f32>(0.0)), alpha);
}
