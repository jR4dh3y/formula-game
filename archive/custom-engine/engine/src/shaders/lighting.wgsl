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

struct LocalShadowCameras {
    view_projection: array<mat4x4<f32>, 6>,
};

struct MaterialProfiles {
    dielectric: array<vec4<f32>, 256>,
    conductor: array<vec4<f32>, 256>,
};

@group(0) @binding(0) var albedo: texture_2d<f32>;
@group(0) @binding(1) var normals: texture_2d<f32>;
@group(0) @binding(2) var pbr: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var reflections: texture_2d<f32>;
@group(0) @binding(5) var<uniform> camera: Camera;
@group(0) @binding(6) var shadow_map: texture_depth_2d;
@group(0) @binding(7) var shadow_sampler: sampler_comparison;
@group(0) @binding(8) var<uniform> light_camera: Camera;
@group(0) @binding(9) var<uniform> lights: Lights;
@group(0) @binding(10) var local_shadow_maps: texture_depth_2d_array;
@group(0) @binding(11) var<uniform> local_shadow_cameras: LocalShadowCameras;
@group(0) @binding(12) var skybox: texture_2d<f32>;
@group(0) @binding(13) var skybox_sampler: sampler;
@group(0) @binding(14) var<uniform> material_profiles: MaterialProfiles;
const RAIN_GRID_X: u32 = 64u;
const RAIN_GRID_Z: u32 = 64u;
const RAIN_BOUNDS_MIN: vec3<f32> = vec3<f32>(-18.5, 0.14, -82.0);
const RAIN_BOUNDS_SIZE: vec3<f32> = vec3<f32>(37.0, 13.5, 100.0);
const EMISSIVE_SCALE: f32 = 12.0;
const PRACTICAL_SPOT_MIN_FILL: f32 = 0.26;
const PRACTICAL_SPOT_MIN_SHADOW: f32 = 0.24;
const INV_PI: f32 = 0.31830988618;
const INV_TAU: f32 = 0.15915494309;


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

fn safe_normalize(value: vec3<f32>) -> vec3<f32> {
    let len = length(value);
    return select(vec3<f32>(0.0), value / len, len > 0.0001);
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn pow5(value: f32) -> f32 {
    let value2 = value * value;
    return value2 * value2 * value;
}

fn hash21(value: vec2<f32>) -> f32 {
    return fract(sin(dot(value, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn value_noise(uv: vec2<f32>) -> f32 {
    let cell = floor(uv);
    let local = fract(uv);
    let f = local * local * (vec2<f32>(3.0) - local * 2.0);
    let a = hash21(cell);
    let b = hash21(cell + vec2<f32>(1.0, 0.0));
    let c = hash21(cell + vec2<f32>(0.0, 1.0));
    let d = hash21(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn layered_noise(uv: vec2<f32>) -> f32 {
    return value_noise(uv) * 0.52
        + value_noise(uv * 2.17 + vec2<f32>(13.1, 7.7)) * 0.31
        + value_noise(uv * 5.11 + vec2<f32>(3.5, 19.9)) * 0.17;
}

fn rain_wind_direction() -> vec3<f32> {
    return safe_normalize(vec3<f32>(
        -0.22 - lights.weather.z * 0.12,
        -1.0,
        0.08 + lights.weather.z * 0.05
    ));
}

fn wrap_rain_xz(xz: vec2<f32>) -> vec2<f32> {
    let local = fract((xz - RAIN_BOUNDS_MIN.xz) / RAIN_BOUNDS_SIZE.xz);
    return RAIN_BOUNDS_MIN.xz + local * RAIN_BOUNDS_SIZE.xz;
}

fn wrap_rain_cell(cell: vec2<f32>) -> vec2<f32> {
    let grid = vec2<f32>(f32(RAIN_GRID_X), f32(RAIN_GRID_Z));
    return cell - floor(cell / grid) * grid;
}

fn periodic_rain_delta(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let delta = a - b;
    return delta - round(delta / RAIN_BOUNDS_SIZE.xz) * RAIN_BOUNDS_SIZE.xz;
}

fn rain_cell_seed(cell: vec2<f32>) -> f32 {
    return hash21(cell + vec2<f32>(23.1, 91.7));
}

fn transparent_underlay(world_position: vec3<f32>) -> vec3<f32> {
    let asphalt = vec3<f32>(0.018, 0.022, 0.025);
    let concrete = vec3<f32>(0.060, 0.058, 0.052);
    let walkway = smoothstep(3.45, 4.75, abs(world_position.x));
    let base = mix(asphalt, concrete, walkway);
    let stain = layered_noise(world_position.xz * 0.55 + vec2<f32>(17.3, 4.1));
    return base * mix(0.72, 1.08, stain);
}

fn reconstruct_world_position(uv: vec2<f32>, scene_depth: f32) -> vec3<f32> {
    let clip_position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_depth, 1.0);
    let world_h = camera.inverse_view_projection * clip_position;
    return world_h.xyz / max(world_h.w, 0.00001);
}

fn sky_radiance(uv: vec2<f32>) -> vec3<f32> {
    let far_world = reconstruct_world_position(uv, 0.0);
    let ray = safe_normalize(far_world - camera.position_time.xyz);
    let panorama_uv = vec2<f32>(
        fract(0.5 + (atan2(ray.z, ray.x) + lights.environment.y) * INV_TAU),
        clamp(0.5 - asin(clamp(ray.y, -1.0, 1.0)) * INV_PI, 0.0, 1.0)
    );
    return textureSample(skybox, skybox_sampler, panorama_uv).rgb
        * exp2(lights.environment.x);
}

fn procedural_rain_impacts(world_position: vec3<f32>, normal: vec3<f32>, surface: vec4<f32>, time: f32) -> f32 {
    let exposed_floor = smoothstep(0.64, 0.94, normal.y)
        * (1.0 - surface.y)
        * (1.0 - smoothstep(0.18, 0.75, surface.a));
    let grid = vec2<f32>(f32(RAIN_GRID_X), f32(RAIN_GRID_Z));
    let wind = rain_wind_direction();
    let drift_per_meter = wind.xz / max(-wind.y, 0.15);
    let full_fall_drift = drift_per_meter * RAIN_BOUNDS_SIZE.y;
    let source_xz = wrap_rain_xz(world_position.xz - full_fall_drift);
    let source_uv = (source_xz - RAIN_BOUNDS_MIN.xz) / RAIN_BOUNDS_SIZE.xz * grid;
    let base_cell = floor(source_uv);
    var impact = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let cell = wrap_rain_cell(base_cell + vec2<f32>(f32(x), f32(y)));
            let seed = rain_cell_seed(cell);
            let jitter = vec2<f32>(hash21(cell + 11.7), hash21(cell + 73.3));
            let top_xz = RAIN_BOUNDS_MIN.xz + ((cell + jitter) / grid) * RAIN_BOUNDS_SIZE.xz;
            let fall_speed = 8.4 + hash21(cell + 47.77) * 5.2 + f32(u32(cell.x + cell.y) % 3u) * 0.9;
            let fall_phase = fract(seed + time * fall_speed / RAIN_BOUNDS_SIZE.y);
            let impact_age = clamp((fall_phase - 0.84) / 0.16, 0.0, 1.0);
            let impact_gate = step(0.84, fall_phase);
            let hit_xz = wrap_rain_xz(top_xz + full_fall_drift);
            let distance_to_drop = length(periodic_rain_delta(world_position.xz, hit_xz));
            let ring_radius = impact_age * (0.055 + seed * 0.18);
            let distance_to_ring = abs(distance_to_drop - ring_radius);
            let ring = (1.0 - smoothstep(0.006, 0.024, distance_to_ring)) * (1.0 - impact_age);
            let splash = (1.0 - smoothstep(0.0, 0.036, distance_to_drop))
                * smoothstep(0.0, 0.18, impact_age)
                * (1.0 - smoothstep(0.26, 0.62, impact_age));
            impact = max(impact, impact_gate * max(ring, splash * 0.85));
        }
    }
    let speckles = smoothstep(0.91, 0.995, layered_noise(source_uv * 1.7 + vec2<f32>(time * 0.11, -time * 0.09))) * 0.025;
    return clamp(exposed_floor * (impact * 0.72 + speckles) * surface.z * lights.weather.x, 0.0, 0.72);
}

fn rain_wetness(world_position: vec3<f32>, normal: vec3<f32>, surface: vec4<f32>, time: f32) -> f32 {
    let upward = smoothstep(0.10, 0.86, normal.y);
    let side = smoothstep(0.24, 0.82, 1.0 - abs(normal.y));
    let wind = rain_wind_direction();
    let wind_shear = wind.xz / max(-wind.y, 0.15);
    let puddle = smoothstep(0.50, 0.86, layered_noise(world_position.xz * 0.12 - wind_shear * time * 0.012));
    let fine_beads = smoothstep(0.68, 0.98, layered_noise(world_position.xz * 1.35 + wind_shear * time * 0.05));
    let runoff = smoothstep(
        0.66,
        0.96,
        layered_noise(vec2<f32>(
            world_position.x * 0.34 + world_position.z * 0.12,
            world_position.y * 1.2 - time * (0.18 + lights.weather.z * 0.08)
        ))
    );
    let non_emissive = 1.0 - smoothstep(0.12, 0.75, surface.a);
    let material_accepts_water = 1.0 - surface.y * 0.35;
    let horizontal_wet = upward * (0.42 + puddle * 0.58 + fine_beads * 0.14);
    let vertical_runoff = side * runoff * 0.16;
    return clamp((horizontal_wet + vertical_runoff) * non_emissive * material_accepts_water * lights.weather.x, 0.0, 0.96);
}

fn screen_space_occlusion(pixel: vec2<u32>, world_position: vec3<f32>, normal: vec3<f32>) -> f32 {
    let size = textureDimensions(depth);
    let offsets = array<vec2<i32>, 12>(
        vec2<i32>(3, 1), vec2<i32>(-3, 2), vec2<i32>(2, -3), vec2<i32>(-2, -3),
        vec2<i32>(6, 4), vec2<i32>(-6, 3), vec2<i32>(4, -6), vec2<i32>(-4, -5),
        vec2<i32>(1, 4), vec2<i32>(-1, 5), vec2<i32>(5, -1), vec2<i32>(-5, -2)
    );
    var occlusion = 0.0;
    for (var sample_index = 0u; sample_index < 12u; sample_index += 1u) {
        let sample_pixel_i = clamp(
            vec2<i32>(pixel) + offsets[sample_index],
            vec2<i32>(0),
            vec2<i32>(size) - vec2<i32>(1)
        );
        let sample_pixel = vec2<u32>(sample_pixel_i);
        let sample_depth = textureLoad(depth, sample_pixel_i, 0);
        if sample_depth <= 0.00001 {
            continue;
        }
        let sample_uv = (vec2<f32>(sample_pixel) + 0.5) / vec2<f32>(size);
        let sample_world = reconstruct_world_position(sample_uv, sample_depth);
        let delta = sample_world - world_position;
        let distance_to_sample = length(delta);
        if distance_to_sample > 0.025 && distance_to_sample < 1.85 {
            // Coplanar neighbours are not occluders; reject the receiver plane.
            let horizon = max(dot(normal, delta / distance_to_sample) - 0.08, 0.0);
            let near_contact = 1.0 - smoothstep(0.018, 0.34, distance_to_sample);
            let broad_contact = 1.0 - smoothstep(0.08, 1.65, distance_to_sample);
            occlusion += horizon * (near_contact * 1.72 + broad_contact * 0.70);
        }
    }
    return clamp(1.0 - occlusion * (1.5 / 12.0), 0.5, 1.0);
}

fn screen_space_contact_shadow(
    pixel: vec2<u32>,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    light_direction: vec3<f32>,
    max_distance: f32,
    strength: f32
) -> f32 {
    let ndotl = dot(normal, light_direction);
    if ndotl <= 0.025 || max_distance <= 0.05 {
        return 1.0;
    }
    let receiver_contact_gate = smoothstep(0.20, 0.78, normal.y);
    if receiver_contact_gate <= 0.015 {
        return 1.0;
    }
    let size = textureDimensions(depth);
    let origin = world_position + normal * 0.045 + light_direction * 0.035;
    let jitter = hash21(vec2<f32>(pixel) + world_position.xz * 13.7) * 0.45;
    var accumulated_occlusion = 0.0;
    var strongest_occlusion = 0.0;
    var total_weight = 0.0;
    for (var step_index = 0u; step_index < 8u; step_index += 1u) {
        let step_phase = (f32(step_index) + 0.62 + jitter) / 8.45;
        let ray_distance = mix(0.055, max_distance, step_phase * step_phase);
        let sample_world = origin + light_direction * ray_distance;
        let sample_clip = camera.view_projection * vec4<f32>(sample_world, 1.0);
        if sample_clip.w <= 0.0001 {
            continue;
        }
        let sample_ndc = sample_clip.xyz / sample_clip.w;
        if sample_ndc.z <= 0.0 || sample_ndc.z >= 1.0 {
            continue;
        }
        let sample_uv = sample_ndc.xy * vec2<f32>(0.5, -0.5) + 0.5;
        if any(sample_uv < vec2<f32>(0.0)) || any(sample_uv > vec2<f32>(1.0)) {
            continue;
        }
        let sample_pixel_i = clamp(
            vec2<i32>(sample_uv * vec2<f32>(size)),
            vec2<i32>(0),
            vec2<i32>(size) - vec2<i32>(1)
        );
        let scene_depth = textureLoad(depth, sample_pixel_i, 0);
        if scene_depth <= 0.00001 {
            continue;
        }
        let depth_delta = scene_depth - sample_ndc.z;
        if depth_delta <= 0.00014 {
            continue;
        }
        let scene_uv = (vec2<f32>(sample_pixel_i) + 0.5) / vec2<f32>(size);
        let occluder_world = reconstruct_world_position(scene_uv, scene_depth);
        let to_occluder = occluder_world - world_position;
        let along_ray = dot(to_occluder, light_direction);
        if along_ray <= 0.035 || along_ray >= max_distance + 0.12 {
            continue;
        }
        let closest_ray_point = world_position + light_direction * along_ray;
        let lateral_distance = length(occluder_world - closest_ray_point);
        let thickness = 0.045 + along_ray * 0.085 + (1.0 - ndotl) * 0.035;
        let contact = 1.0 - smoothstep(thickness, thickness * 3.15, lateral_distance);
        let depth_weight = smoothstep(0.00014, 0.0024, depth_delta);
        let distance_weight = 1.0 - smoothstep(max_distance * 0.58, max_distance, along_ray);
        let receiver_side = smoothstep(-0.015, 0.055, dot(to_occluder, normal));
        let sample_occlusion = contact * depth_weight * distance_weight * receiver_side;
        let sample_weight = 1.0 - step_phase * 0.42;
        accumulated_occlusion += sample_occlusion * sample_weight;
        strongest_occlusion = max(strongest_occlusion, sample_occlusion);
        total_weight += sample_weight;
    }
    let averaged_occlusion = accumulated_occlusion / max(total_weight, 0.0001);
    let softened_occlusion = smoothstep(0.05, 0.72, mix(averaged_occlusion, strongest_occlusion, 0.42));
    let contact_visibility = clamp(1.0 - softened_occlusion * strength, 0.42, 1.0);
    return mix(1.0, contact_visibility, receiver_contact_gate);
}

fn shadow_visibility(world_position: vec3<f32>, normal: vec3<f32>, light_direction: vec3<f32>) -> f32 {
    let light_clip = light_camera.view_projection * vec4<f32>(world_position + normal * 0.018, 1.0);
    let light_ndc = light_clip.xyz / max(light_clip.w, 0.00001);
    let uv = light_ndc.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || light_ndc.z <= 0.0 || light_ndc.z >= 1.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    let ndotl = max(dot(normal, light_direction), 0.0);
    let bias = max(0.00008, (1.0 - ndotl) * 0.00025);
    let filter_radius = 1.5;
    var visibility = 0.0;
    var total_weight = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = 1.0 / (1.0 + dot(offset, offset) * 0.24);
            visibility += textureSampleCompare(
                shadow_map,
                shadow_sampler,
                uv + offset * texel * filter_radius,
                light_ndc.z + bias
            ) * weight;
            total_weight += weight;
        }
    }
    let filtered_visibility = visibility / max(total_weight, 0.0001);
    return mix(filtered_visibility, 1.0, 0.035);
}

fn spot_cone_attenuation(light_index: u32, light_to_surface: vec3<f32>) -> f32 {
    let flags = lights.local_inner_shadow[light_index];
    if flags.w < 0.5 {
        return 1.0;
    }
    let direction_outer = lights.local_direction_outer[light_index];
    let spot_direction = safe_normalize(direction_outer.xyz);
    let inner = flags.x;
    let outer = direction_outer.w;
    let alignment = dot(light_to_surface, spot_direction);
    let core = clamp((alignment - outer) / max(inner - outer, 0.0001), 0.0, 1.0);
    let smooth_core = core * core * (3.0 - 2.0 * core);
    let outer_halo = smoothstep(outer - 0.20, outer + 0.015, alignment) * 0.24;
    return max(smooth_core, outer_halo * (1.0 - smooth_core * 0.45));
}

fn practical_spot_fill(light_index: u32, cone: f32) -> f32 {
    let flags = lights.local_inner_shadow[light_index];
    if flags.w < 0.5 {
        return 1.0;
    }
    let spot_presence = smoothstep(0.015, 0.16, cone);
    return mix(PRACTICAL_SPOT_MIN_FILL, 1.0, clamp(cone, 0.0, 1.0)) * spot_presence;
}

fn practical_spot_shadow_weight(light_index: u32, cone: f32) -> f32 {
    let flags = lights.local_inner_shadow[light_index];
    if flags.w < 0.5 {
        return 1.0;
    }
    let spot_presence = smoothstep(0.015, 0.18, cone);
    return mix(PRACTICAL_SPOT_MIN_SHADOW, 1.0, clamp(cone, 0.0, 1.0)) * spot_presence;
}

fn local_shadow_visibility(world_position: vec3<f32>, normal: vec3<f32>, light_index: u32) -> f32 {
    let flags = lights.local_inner_shadow[light_index];
    if flags.y < 0.5 {
        return 1.0;
    }
    let light_clip = local_shadow_cameras.view_projection[light_index]
        * vec4<f32>(world_position + normal * 0.025, 1.0);
    if light_clip.w <= 0.0001 {
        return 1.0;
    }
    let light_ndc = light_clip.xyz / light_clip.w;
    let uv = light_ndc.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || light_ndc.z >= 1.0 || light_ndc.z <= 0.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(local_shadow_maps));
    let layer = i32(light_index);
    var visibility = 0.0;
    var total_weight = 0.0;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let offset = vec2<f32>(f32(x), f32(y));
            let weight = 1.0 / (1.0 + dot(offset, offset) * 0.32);
            visibility += textureSampleCompare(
                local_shadow_maps,
                shadow_sampler,
                uv + offset * texel * 5.25,
                layer,
                light_ndc.z + 0.0015
            ) * weight;
            total_weight += weight;
        }
    }
    let filtered_visibility = visibility / max(total_weight, 0.0001);
    return mix(filtered_visibility, 1.0, 0.045);
}

fn shade_light(
    base: vec3<f32>,
    normal: vec3<f32>,
    view_direction: vec3<f32>,
    surface: vec4<f32>,
    wetness: f32,
    light_direction: vec3<f32>,
    radiance: vec3<f32>,
    material_profile_id: u32,
    material_class: u32
) -> vec3<f32> {
    let raw_ndotl = dot(normal, light_direction);
    var ndotl = max(raw_ndotl, 0.0);
    let ndotv = max(dot(normal, view_direction), 0.0);
    let roughness = max(surface.x, 0.04);
    let half_direction = safe_normalize(light_direction + view_direction);
    let ldoth = max(dot(light_direction, half_direction), 0.0);
    let fd90 = 0.5 + 2.0 * ldoth * ldoth * roughness;
    let light_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - ndotl);
    let view_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - ndotv);
    let dielectric_profile = material_profiles.dielectric[material_profile_id];
    let conductor_profile = material_profiles.conductor[material_profile_id];
    if material_class == 3u {
        ndotl = clamp((raw_ndotl + 0.34) / 1.34, 0.0, 1.0);
    }
    let diffuse_weight = mix(dielectric_profile.a, conductor_profile.a, surface.y);
    let diffuse = base
        * vec3<f32>(diffuse_weight)
        * ndotl
        * light_scatter
        * view_scatter
        * surface.z
        * (1.0 - wetness * 0.38)
        * INV_PI;
    let ndoth = max(dot(normal, half_direction), 0.0);
    let specular_power = mix(220.0, 9.0, roughness);
    var f0 = mix(dielectric_profile.rgb, conductor_profile.rgb, vec3<f32>(surface.y));
    f0 = mix(f0, vec3<f32>(0.075), wetness * (1.0 - surface.y));
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - max(dot(normal, view_direction), 0.0), 5.0);
    let specular_occlusion = clamp(pow(ndotv + surface.z, exp2(-16.0 * roughness - 1.0)) - 1.0 + surface.z, 0.0, 1.0);
    var specular = pow(ndoth, specular_power) * fresnel * ndotl * specular_occlusion * (1.0 + wetness * 1.7);
    if material_class == 4u {
        let tangent_seed = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(normal.y) > 0.94);
        let strand_tangent = safe_normalize(cross(tangent_seed, normal));
        let longitudinal = pow(max(abs(dot(half_direction, strand_tangent)), 0.0), mix(48.0, 9.0, roughness));
        specular += conductor_profile.rgb * longitudinal * ndotl * (0.24 + conductor_profile.a * 0.46);
    }
    var transmission = vec3<f32>(0.0);
    if material_class == 3u || material_class == 2u {
        let back_scatter = pow(max(-raw_ndotl, 0.0), 2.0) * conductor_profile.a;
        transmission = base * vec3<f32>(1.0, 0.42, 0.30) * back_scatter * surface.z;
    }
    return (diffuse + specular + transmission) * radiance;
}

fn environment_irradiance(normal: vec3<f32>) -> vec3<f32> {
    let x = normal.x;
    let y = normal.y;
    let z = normal.z;
    let basis = array<f32, 9>(
        0.282095,
        0.488603 * z,
        0.488603 * y,
        0.488603 * x,
        1.092548 * x * z,
        1.092548 * z * y,
        0.315392 * (3.0 * y * y - 1.0),
        1.092548 * x * y,
        0.546274 * (x * x - z * z)
    );
    var irradiance = vec3<f32>(0.0);
    for (var index = 0u; index < 9u; index += 1u) {
        irradiance += lights.environment_sh[index].rgb * basis[index];
    }
    return max(irradiance, vec3<f32>(0.0));
}


fn sample_reflection_history(uv: vec2<f32>) -> vec4<f32> {
    let reflection_size = textureDimensions(reflections);
    let coord = uv * vec2<f32>(reflection_size) - vec2<f32>(0.5);
    let base = floor(coord);
    let f = fract(coord);
    let p0 = clamp(vec2<i32>(base), vec2<i32>(0), vec2<i32>(reflection_size) - vec2<i32>(1));
    let p1 = clamp(p0 + vec2<i32>(1, 0), vec2<i32>(0), vec2<i32>(reflection_size) - vec2<i32>(1));
    let p2 = clamp(p0 + vec2<i32>(0, 1), vec2<i32>(0), vec2<i32>(reflection_size) - vec2<i32>(1));
    let p3 = clamp(p0 + vec2<i32>(1, 1), vec2<i32>(0), vec2<i32>(reflection_size) - vec2<i32>(1));
    let a = textureLoad(reflections, p0, 0);
    let b = textureLoad(reflections, p1, 0);
    let c = textureLoad(reflections, p2, 0);
    let d = textureLoad(reflections, p3, 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = textureDimensions(albedo);
    let pixel = min(vec2<u32>(input.position.xy), size - vec2<u32>(1u));
    let scene_depth = textureLoad(depth, vec2<i32>(pixel), 0);
    if scene_depth <= 0.00001 {
        let sky = sky_radiance(input.uv);
        return vec4<f32>(sky, clamp(luminance(sky), 0.0001, 0.18));
    }

    let surface_raw = textureLoad(pbr, vec2<i32>(pixel), 0);
    let normal_flags = textureLoad(normals, vec2<i32>(pixel), 0);
    let normal = normalize(normal_flags.xyz);
    let material_flags = u32(round(normal_flags.w));
    let glass_mask = f32(material_flags & 1u);
    let material_profile_id = (material_flags >> 3u) & 255u;
    let profile_meta = material_profiles.conductor[material_profile_id].a;
    var material_class = 0u;
    if profile_meta > 0.60 { material_class = 4u; }
    else if profile_meta > 0.28 { material_class = 3u; }
    else if profile_meta > 0.08 { material_class = 2u; }
    let world_position = reconstruct_world_position(input.uv, scene_depth);
    let wetness = rain_wetness(world_position, normal, surface_raw, camera.position_time.w);
    let impact = procedural_rain_impacts(world_position, normal, surface_raw, camera.position_time.w);
    let water_coverage = clamp(max(wetness, glass_mask * 0.94) + impact * 0.62, 0.0, 1.0);
    let effective_surface = vec4<f32>(
        mix(surface_raw.x, 0.022, water_coverage * 0.88),
        surface_raw.y,
        surface_raw.z,
        surface_raw.a
    );
    let albedo_sample = textureLoad(albedo, vec2<i32>(pixel), 0);
    let opacity = mix(1.0, clamp(albedo_sample.a, 0.04, 1.0), glass_mask);
    let base_raw = albedo_sample.rgb;
    let base = mix(base_raw, base_raw * 0.58, wetness * (1.0 - surface_raw.y) * 0.46);
    let reflected_sample = sample_reflection_history(input.uv);
    let reflected = reflected_sample.rgb;
    let reflected_roughness = reflected_sample.a;
    let emissive_strength = surface_raw.a * EMISSIVE_SCALE;
    let emissive_mask = smoothstep(0.015, 0.09, surface_raw.a);

    let light_direction = safe_normalize(lights.directional_direction_count.xyz);
    let view_direction = safe_normalize(camera.position_time.xyz - world_position);
    let ssao = screen_space_occlusion(pixel, world_position, normal);
    let contact_visibility = mix(ssao, 1.0, 0.16);
    let indirect_irradiance =
        lights.ambient.rgb * (0.18 + max(normal.y, 0.0) * 0.30)
        + environment_irradiance(normal);
    let ambient = indirect_irradiance * INV_PI;
    let sun_shadow = shadow_visibility(world_position, normal, light_direction);
    let transparent_diffuse_weight = mix(1.0, 0.24 + opacity * 0.18, glass_mask);
    var hdr = base
        * vec3<f32>(1.0 - surface_raw.y)
        * ambient
        * ssao
        * effective_surface.z
        * transparent_diffuse_weight
        * (1.0 - emissive_mask * 0.85);
    var diffuse_illuminance = max(luminance(ambient) * ssao, 0.0001);
    if lights.directional_color_intensity.w > 0.0 && length(light_direction) > 0.0 {
        let directional_contact =
            screen_space_contact_shadow(pixel, world_position, normal, light_direction, 1.85, 0.58);
        let directional_radiance =
            lights.directional_color_intensity.rgb * lights.directional_color_intensity.w * sun_shadow * directional_contact;
        diffuse_illuminance += luminance(directional_radiance)
            * INV_PI
            * max(dot(normal, light_direction), 0.0) * contact_visibility;
        hdr += shade_light(base, normal, view_direction, effective_surface, wetness, light_direction, directional_radiance, material_profile_id, material_class)
            * contact_visibility;
    }
    hdr += albedo_sample.rgb * emissive_strength;
    let surface_fresnel = 0.035 + pow(1.0 - max(dot(normal, view_direction), 0.0), 5.0) * 0.72;
    let puddle_visibility = max(
        smoothstep(0.06, 0.64, water_coverage) * (1.0 - smoothstep(0.42, 0.90, effective_surface.x)),
        glass_mask * 0.92
    );
    let base_reflected_weight = clamp(
        (surface_fresnel + surface_raw.y * 0.38 + glass_mask * 0.18)
            * puddle_visibility
            * (0.92 + impact * 0.34)
            * (1.0 - reflected_roughness * 0.16),
        0.0,
        1.12
    );
    let lit_luminance = max(luminance(hdr), 0.05);
    let reflection_luminance = max(luminance(reflected), 0.0001);
    let reflection_energy_cap =
        lit_luminance * (0.34 + water_coverage * 0.82 + surface_fresnel * 0.38 + glass_mask * 0.26)
        + water_coverage * (0.10 + surface_fresnel * 0.22 + glass_mask * 0.14);
    let reflected_weight = min(base_reflected_weight, reflection_energy_cap / reflection_luminance);
    hdr += reflected * reflected_weight;
    let rain_light_mask = clamp(luminance(hdr) * 0.24 + emissive_strength * 0.05, 0.04, 1.0);
    let impact_light = mix(lights.sky.rgb, lights.directional_color_intensity.rgb, vec3<f32>(0.35))
        * impact
        * rain_light_mask
        * (0.10 + wetness * 0.18);
    hdr += impact_light;
    let vignette_distance = distance(input.uv, vec2<f32>(0.5));
    let vignette = 1.0 - smoothstep(0.42, 0.78, vignette_distance) * 0.42;
    return vec4<f32>(max(hdr * vignette, vec3<f32>(0.0)), diffuse_illuminance);
}

@fragment
fn fs_local_lights(input: VertexOutput) -> @location(0) vec4<f32> {
    let size = textureDimensions(albedo);
    let pixel = min(vec2<u32>(input.position.xy), size - vec2<u32>(1u));
    let scene_depth = textureLoad(depth, vec2<i32>(pixel), 0);
    if scene_depth <= 0.00001 {
        discard;
    }
    let surface = textureLoad(pbr, vec2<i32>(pixel), 0);
    let normal_flags = textureLoad(normals, vec2<i32>(pixel), 0);
    let normal = normalize(normal_flags.xyz);
    let material_flags = u32(round(normal_flags.w));
    let material_profile_id = (material_flags >> 3u) & 255u;
    let profile_meta = material_profiles.conductor[material_profile_id].a;
    var material_class = 0u;
    if profile_meta > 0.60 { material_class = 4u; }
    else if profile_meta > 0.28 { material_class = 3u; }
    else if profile_meta > 0.08 { material_class = 2u; }
    let world_position = reconstruct_world_position(input.uv, scene_depth);
    let base = textureLoad(albedo, vec2<i32>(pixel), 0).rgb;
    let view_direction = safe_normalize(camera.position_time.xyz - world_position);
    let wetness = rain_wetness(world_position, normal, surface, camera.position_time.w);
    let water_coverage = clamp(wetness, 0.0, 1.0);
    let effective_surface = vec4<f32>(mix(surface.x, 0.022, water_coverage * 0.88), surface.yzw);
    let contact_visibility = mix(screen_space_occlusion(pixel, world_position, normal), 1.0, 0.16);
    let light_direction = safe_normalize(lights.directional_direction_count.xyz);
    let sun_shadow = shadow_visibility(world_position, normal, light_direction);
    var local_hdr = vec3<f32>(0.0);
    var local_diffuse = 0.0;
    let local_count = u32(clamp(lights.directional_direction_count.w, 0.0, 6.0));
    for (var light_index = 0u; light_index < local_count; light_index += 1u) {
        let position_radius = lights.local_position_radius[light_index];
        let to_light = position_radius.xyz - world_position;
        let distance_to_light = length(to_light);
        let color_intensity = lights.local_color_intensity[light_index];
        if color_intensity.w <= 0.0 || distance_to_light >= position_radius.w { continue; }
        let local_direction = to_light / max(distance_to_light, 0.0001);
        let attenuation = pow(clamp(1.0 - distance_to_light / position_radius.w, 0.0, 1.0), 2.0);
        let cone = spot_cone_attenuation(light_index, -local_direction);
        if lights.local_inner_shadow[light_index].w > 0.5 && cone <= 0.0001 { continue; }
        let spot = practical_spot_fill(light_index, cone);
        let shadow_weight = practical_spot_shadow_weight(light_index, cone);
        let shadow = mix(1.0, local_shadow_visibility(world_position, normal, light_index), shadow_weight);
        let contact = screen_space_contact_shadow(pixel, world_position, normal, local_direction, clamp(distance_to_light * 0.42, 0.42, 1.45), 0.46);
        let radiance = color_intensity.rgb * color_intensity.w * attenuation * spot * shadow * contact * (1.0 + (1.0 - sun_shadow) * 0.12);
        local_hdr += shade_light(base, normal, view_direction, effective_surface, wetness, local_direction, radiance, material_profile_id, material_class) * contact_visibility;
        local_diffuse += luminance(radiance)
            * INV_PI
            * max(dot(normal, local_direction), 0.0)
            * contact_visibility;
    }
    return vec4<f32>(max(local_hdr, vec3<f32>(0.0)), max(local_diffuse, 0.0));
}
