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

struct ReflectionProxies {
    count: vec4<u32>,
    bounds_min: array<vec4<f32>, 24>,
    bounds_max: array<vec4<f32>, 24>,
    radiance: array<vec4<f32>, 24>,
};


@group(0) @binding(0) var albedo: texture_2d<f32>;
@group(0) @binding(1) var normals: texture_2d<f32>;
@group(0) @binding(2) var pbr: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var output_reflection: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var<uniform> frame: Frame;
@group(0) @binding(6) var<uniform> camera: Camera;
@group(0) @binding(7) var<uniform> lights: Lights;
@group(0) @binding(8) var reflection_history: texture_2d<f32>;
@group(0) @binding(9) var motion_vectors: texture_2d<f32>;
@group(0) @binding(10) var<uniform> reflection_proxies: ReflectionProxies;
@group(0) @binding(11) var half_resolution_ssr: texture_2d<f32>;
@group(0) @binding(12) var skybox: texture_2d<f32>;
@group(0) @binding(13) var skybox_sampler: sampler;

const INV_PI: f32 = 0.31830988618;
const INV_TAU: f32 = 0.15915494309;

const RAIN_BOUNDS_MIN: vec3<f32> = vec3<f32>(-18.5, 0.18, -82.0);
const RAIN_BOUNDS_SIZE: vec3<f32> = vec3<f32>(37.0, 13.5, 100.0);

struct ProxyHit {
    t: f32,
    radiance: vec3<f32>,
    normal: vec3<f32>,
    hit: f32,
};

fn hash_blue_noise(pixel: vec2<u32>, frame_index: u32) -> f32 {
    var state = pixel.x * 1973u + pixel.y * 9277u + frame_index * 26699u + 0x68bc21ebu;
    state = (state ^ (state >> 16u)) * 0x7feb352du;
    state = (state ^ (state >> 15u)) * 0x846ca68bu;
    state = state ^ (state >> 16u);
    return f32(state & 0x00ffffffu) / 16777215.0;
}

fn safe_normalize(value: vec3<f32>) -> vec3<f32> {
    let len = length(value);
    return select(vec3<f32>(0.0), value / len, len > 0.0001);
}

fn reconstruct_world_position(uv: vec2<f32>, scene_depth: f32) -> vec3<f32> {
    let clip_position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_depth, 1.0);
    let world_h = camera.inverse_view_projection * clip_position;
    return world_h.xyz / max(world_h.w, 0.00001);
}

fn environment_radiance(direction: vec3<f32>, roughness: f32) -> vec3<f32> {
    let ray = safe_normalize(direction);
    let panorama_uv = vec2<f32>(
        fract(0.5 + (atan2(ray.z, ray.x) + lights.environment.y) * INV_TAU),
        clamp(0.5 - asin(clamp(ray.y, -1.0, 1.0)) * INV_PI, 0.0, 1.0)
    );
    let max_lod = f32(textureNumLevels(skybox) - 1u);
    let lod = roughness * roughness * max_lod;
    return textureSampleLevel(skybox, skybox_sampler, panorama_uv, lod).rgb
        * exp2(lights.environment.x);
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

fn transparent_underlay(world_position: vec3<f32>) -> vec3<f32> {
    let asphalt = vec3<f32>(0.018, 0.022, 0.025);
    let concrete = vec3<f32>(0.060, 0.058, 0.052);
    let walkway = smoothstep(3.45, 4.75, abs(world_position.x));
    let base = mix(asphalt, concrete, walkway);
    let stain = layered_noise(world_position.xz * 0.55 + vec2<f32>(17.3, 4.1));
    return base * mix(0.72, 1.08, stain);
}

fn rain_wetness(world_position: vec3<f32>, normal: vec3<f32>, surface: vec4<f32>) -> f32 {
    let time = camera.position_time.w;
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


fn trace_scene_proxy(origin: vec3<f32>, ray: vec3<f32>) -> ProxyHit {
    var best = ProxyHit(1000000.0, vec3<f32>(0.0), vec3<f32>(0.0, 1.0, 0.0), 0.0);
    let ray_sign = select(vec3<f32>(-1.0), vec3<f32>(1.0), ray >= vec3<f32>(0.0));
    let safe_ray = select(ray_sign * vec3<f32>(0.0001), ray, abs(ray) > vec3<f32>(0.0001));
    let inv_ray = vec3<f32>(1.0) / safe_ray;
    let proxy_count = min(reflection_proxies.count.x, 24u);
    for (var index = 0u; index < proxy_count; index += 1u) {
        let bounds_min = reflection_proxies.bounds_min[index].xyz;
        let bounds_max = reflection_proxies.bounds_max[index].xyz;
        let t0 = (bounds_min - origin) * inv_ray;
        let t1 = (bounds_max - origin) * inv_ray;
        let tmin3 = min(t0, t1);
        let tmax3 = max(t0, t1);
        let near_t = max(max(tmin3.x, tmin3.y), tmin3.z);
        let far_t = min(min(tmax3.x, tmax3.y), tmax3.z);
        if far_t < max(near_t, 0.04) || near_t >= best.t {
            continue;
        }
        let hit_position = origin + ray * near_t;
        let center = (bounds_min + bounds_max) * 0.5;
        let half_extent = max((bounds_max - bounds_min) * 0.5, vec3<f32>(0.001));
        let local = (hit_position - center) / half_extent;
        let abs_local = abs(local);
        var hit_normal = vec3<f32>(0.0, 1.0, 0.0);
        if abs_local.x > abs_local.y && abs_local.x > abs_local.z {
            hit_normal = vec3<f32>(sign(local.x), 0.0, 0.0);
        } else if abs_local.y > abs_local.z {
            hit_normal = vec3<f32>(0.0, sign(local.y), 0.0);
        } else {
            hit_normal = vec3<f32>(0.0, 0.0, sign(local.z));
        }
        let proxy_radiance = reflection_proxies.radiance[index];
        let facing = smoothstep(-0.18, 0.42, dot(hit_normal, -ray));
        best = ProxyHit(near_t, proxy_radiance.rgb * max(proxy_radiance.a, 0.05), hit_normal, facing);
    }
    return best;
}

fn temporal_reconstruct_reflection(
    source_pixel: vec2<u32>,
    source_size: vec2<u32>,
    output_size: vec2<u32>,
    current_reflection: vec3<f32>,
    current_roughness: f32
) -> vec3<f32> {
    if frame.counters.x == 0u {
        return current_reflection;
    }
    let uv = (vec2<f32>(source_pixel) + 0.5) / vec2<f32>(source_size);
    let motion = textureLoad(motion_vectors, vec2<i32>(source_pixel), 0).rg;
    let previous_uv = uv - motion;
    if any(previous_uv < vec2<f32>(0.0)) || any(previous_uv > vec2<f32>(1.0)) {
        return current_reflection;
    }
    let history_pixel = clamp(
        vec2<i32>(previous_uv * vec2<f32>(output_size)),
        vec2<i32>(0),
        vec2<i32>(output_size) - vec2<i32>(1)
    );
    let history_sample = textureLoad(reflection_history, history_pixel, 0);
    let motion_rejection = smoothstep(0.010, 0.085, length(motion));
    let roughness_rejection = smoothstep(0.045, 0.22, abs(history_sample.a - current_roughness));
    let history_weight = 0.58 * (1.0 - motion_rejection) * (1.0 - roughness_rejection);
    return mix(current_reflection, history_sample.rgb, history_weight);
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let output_size = textureDimensions(output_reflection);
    if any(invocation.xy >= output_size) {
        return;
    }

    let source_size = textureDimensions(normals);
    let source_pixel = min(
        vec2<u32>(vec2<f32>(invocation.xy) * vec2<f32>(source_size) / vec2<f32>(output_size)),
        source_size - vec2<u32>(1u)
    );
    let scene_depth = textureLoad(depth, vec2<i32>(source_pixel), 0);
    let surface = textureLoad(pbr, vec2<i32>(source_pixel), 0);
    if scene_depth <= 0.00001 {
        textureStore(output_reflection, vec2<i32>(invocation.xy), vec4<f32>(0.0));
        return;
    }

    let normal_flags = textureLoad(normals, vec2<i32>(source_pixel), 0);
    let normal = normalize(normal_flags.xyz);
    let material_flags = u32(round(normal_flags.w));
    let glass_mask = f32(material_flags & 1u);
    let water_glass_mask = glass_mask * smoothstep(0.48, 0.82, normal.y);
    let upright_glass_mask = glass_mask * (1.0 - water_glass_mask);
    let uv = (vec2<f32>(source_pixel) + 0.5) / vec2<f32>(source_size);
    let world_position = reconstruct_world_position(uv, scene_depth);
    let wetness = max(rain_wetness(world_position, normal, surface), glass_mask * 0.94);
    let effective_roughness = mix(surface.x, 0.022, wetness * 0.88);
    if effective_roughness > 0.72 && wetness < 0.06 {
        textureStore(output_reflection, vec2<i32>(invocation.xy), vec4<f32>(0.0));
        return;
    }

    let view_ray = safe_normalize(world_position - camera.position_time.xyz);
    let reflected_base = reflect(view_ray, normal);
    let tangent_seed = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(normal.y) < 0.92);
    let tangent = safe_normalize(cross(tangent_seed, normal));
    let bitangent = safe_normalize(cross(normal, tangent));
    let noise_a = hash_blue_noise(invocation.xy, frame.counters.x + 17u) - 0.5;
    let noise_b = hash_blue_noise(invocation.yx + vec2<u32>(13u, 29u), frame.counters.x + 71u) - 0.5;
    let rough_jitter = effective_roughness * effective_roughness * 0.22;
    let reflected = safe_normalize(reflected_base + (tangent * noise_a + bitangent * noise_b) * rough_jitter);
    let sky = environment_radiance(reflected, effective_roughness);
    var local_reflection = vec3<f32>(0.0);
    let wet_puddle = max(smoothstep(0.06, 0.64, wetness), glass_mask);
    let local_count = u32(clamp(lights.directional_direction_count.w, 0.0, 6.0));
    for (var light_index = 0u; light_index < local_count; light_index += 1u) {
        let position_radius = lights.local_position_radius[light_index];
        let to_light = position_radius.xyz - world_position;
        let distance_to_light = length(to_light);
        let light_direction = to_light / max(distance_to_light, 0.0001);
        let light_to_surface = -light_direction;
        let attenuation = pow(clamp(1.0 - distance_to_light / position_radius.w, 0.0, 1.0), 2.0);
        let mirror_alignment = max(dot(reflected, light_direction), 0.0);
        let tight_alignment = pow(mirror_alignment, 28.0);
        let wet_alignment = pow(mirror_alignment, 5.0);
        let ground_streak = 1.0 / (
            1.0
            + abs(world_position.x - position_radius.x) * 0.13
            + abs(world_position.z - position_radius.z) * 0.038
        );
        let water_streak =
            wet_puddle
            * ground_streak
            * (1.0 / (1.0 + distance_to_light * 0.055))
            * 0.34;
        let alignment = mix(tight_alignment, wet_alignment * (0.18 + ground_streak), wet_puddle);
        let color_intensity = lights.local_color_intensity[light_index];
        let cone = spot_cone_attenuation(light_index, light_to_surface);
        local_reflection += color_intensity.rgb
            * color_intensity.w
            * (attenuation + water_streak)
            * practical_spot_fill(light_index, cone)
            * alignment
            * mix(1.0, 1.55, wet_puddle);
    }
    let ssr_size = textureDimensions(half_resolution_ssr);
    let ssr_uv = (vec2<f32>(source_pixel) + 0.5) / vec2<f32>(source_size);
    let ssr_pixel = clamp(
        vec2<i32>(ssr_uv * vec2<f32>(ssr_size)),
        vec2<i32>(0),
        vec2<i32>(ssr_size) - vec2<i32>(1)
    );
    let ssr_sample = textureLoad(half_resolution_ssr, ssr_pixel, 0);
    let ssr_hit = clamp(ssr_sample.a, 0.0, 1.0);
    let ssr_reflection = ssr_sample.rgb * mix(1.0, 0.66, effective_roughness) * ssr_hit;
    let proxy_hit_result = trace_scene_proxy(world_position + normal * 0.045, reflected);
    let proxy_reflection = proxy_hit_result.radiance
        * mix(1.0, 0.72, effective_roughness)
        * (1.0 / (1.0 + proxy_hit_result.t * 0.018))
        * proxy_hit_result.hit;
    let view_fresnel = 0.035 + pow(1.0 - max(dot(normal, -view_ray), 0.0), 5.0) * 0.68;
    let response = clamp(
        (1.0 - effective_roughness) * (view_fresnel + surface.y * 0.36) * (0.54 + wet_puddle * 1.18),
        0.0,
        1.24
    );
    let traced_reflection = mix(proxy_reflection, ssr_reflection, ssr_hit);
    let exterior_reflection = (sky * 0.78 + local_reflection * 1.46 + traced_reflection * 1.34) * response;
    let interior_line = f32(source_pixel.x & 1u);
    let underlayer_reflection =
        transparent_underlay(world_position) * (0.22 + wet_puddle * 0.28)
        + local_reflection * 0.18
        + traced_reflection * 0.38;
    let glass_interlaced = mix(
        exterior_reflection,
        underlayer_reflection,
        interior_line * upright_glass_mask
    );
    let water_reflection = exterior_reflection + underlayer_reflection * (0.10 + wet_puddle * 0.10);
    let current_reflection = mix(
        mix(exterior_reflection, glass_interlaced, upright_glass_mask),
        water_reflection,
        water_glass_mask
    );
    let temporal_reflection = temporal_reconstruct_reflection(
        source_pixel,
        source_size,
        output_size,
        current_reflection,
        effective_roughness
    );
    textureStore(
        output_reflection,
        vec2<i32>(invocation.xy),
        vec4<f32>(temporal_reflection, effective_roughness)
    );
}
