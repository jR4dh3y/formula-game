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

struct SsrHit {
    t: f32,
    radiance: vec3<f32>,
    hit: f32,
};

@group(0) @binding(0) var albedo: texture_2d<f32>;
@group(0) @binding(1) var normals: texture_2d<f32>;
@group(0) @binding(2) var pbr: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var output_ssr: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var<uniform> camera: Camera;
@group(0) @binding(6) var<uniform> lights: Lights;

fn safe_normalize(value: vec3<f32>) -> vec3<f32> {
    let len = length(value);
    return select(vec3<f32>(0.0), value / len, len > 0.0001);
}

fn reconstruct_world_position(uv: vec2<f32>, scene_depth: f32) -> vec3<f32> {
    let clip_position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, scene_depth, 1.0);
    let world_h = camera.inverse_view_projection * clip_position;
    return world_h.xyz / max(world_h.w, 0.00001);
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

fn surface_direct_radiance(position: vec3<f32>, normal: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
    var radiance = base * lights.ambient.rgb * (0.52 + max(normal.y, 0.0) * 0.34);
    let moon_direction = safe_normalize(lights.directional_direction_count.xyz);
    if lights.directional_color_intensity.w > 0.0 && length(moon_direction) > 0.0 {
        let ndotl = max(dot(normal, moon_direction), 0.0);
        radiance += base
            * lights.directional_color_intensity.rgb
            * lights.directional_color_intensity.w
            * ndotl
            * 0.55;
    }

    let local_count = u32(clamp(lights.directional_direction_count.w, 0.0, 6.0));
    for (var light_index = 0u; light_index < local_count; light_index += 1u) {
        let position_radius = lights.local_position_radius[light_index];
        let to_light = position_radius.xyz - position;
        let distance_to_light = length(to_light);
        let light_direction = to_light / max(distance_to_light, 0.0001);
        let light_to_surface = -light_direction;
        let attenuation = pow(clamp(1.0 - distance_to_light / position_radius.w, 0.0, 1.0), 2.0);
        let cone = spot_cone_attenuation(light_index, light_to_surface);
        let color_intensity = lights.local_color_intensity[light_index];
        radiance += base
            * color_intensity.rgb
            * color_intensity.w
            * attenuation
            * practical_spot_fill(light_index, cone)
            * max(dot(normal, light_direction), 0.0)
            * 0.72;
    }
    return radiance;
}

fn project_to_uv_depth(position: vec3<f32>) -> vec3<f32> {
    let clip = camera.view_projection * vec4<f32>(position, 1.0);
    let ndc = clip.xyz / max(clip.w, 0.00001);
    return vec3<f32>(ndc.xy * vec2<f32>(0.5, -0.5) + 0.5, ndc.z);
}

fn trace_screen_space_reflection(origin: vec3<f32>, ray: vec3<f32>, source_size: vec2<u32>) -> SsrHit {
    var travel = 0.10;
    for (var step = 0u; step < 24u; step += 1u) {
        let step_fraction = f32(step) / 23.0;
        travel += mix(0.10, 0.42, step_fraction);
        let probe = origin + ray * travel;
        let projected = project_to_uv_depth(probe);
        if any(projected.xy < vec2<f32>(0.0)) || any(projected.xy > vec2<f32>(1.0)) || projected.z <= 0.0 || projected.z >= 1.0 {
            continue;
        }
        let sample_pixel = clamp(
            vec2<i32>(projected.xy * vec2<f32>(source_size)),
            vec2<i32>(0),
            vec2<i32>(source_size) - vec2<i32>(1)
        );
        let sample_depth = textureLoad(depth, sample_pixel, 0);
        if sample_depth <= 0.00001 {
            continue;
        }
        let depth_delta = sample_depth - projected.z;
        let thickness = 0.007 + travel * 0.002;
        if depth_delta > 0.0 && depth_delta < 0.032 + thickness {
            let hit_uv = (vec2<f32>(sample_pixel) + 0.5) / vec2<f32>(source_size);
            let hit_world = reconstruct_world_position(hit_uv, sample_depth);
            let hit_normal = normalize(textureLoad(normals, sample_pixel, 0).xyz);
            let hit_base = textureLoad(albedo, sample_pixel, 0).rgb;
            let hit_surface = textureLoad(pbr, sample_pixel, 0);
            let emissive = hit_base * hit_surface.a * 12.0;
            let facing = smoothstep(-0.18, 0.42, dot(hit_normal, -ray));
            let radiance = surface_direct_radiance(hit_world, hit_normal, hit_base) + emissive;
            return SsrHit(travel, radiance * (1.0 / (1.0 + travel * 0.038)), facing);
        }
    }
    return SsrHit(1000000.0, vec3<f32>(0.0), 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let output_size = textureDimensions(output_ssr);
    if any(invocation.xy >= output_size) {
        return;
    }

    let source_size = textureDimensions(normals);
    let source_pixel = min(
        vec2<u32>((vec2<f32>(invocation.xy) + 0.5) * vec2<f32>(source_size) / vec2<f32>(output_size)),
        source_size - vec2<u32>(1u)
    );
    let scene_depth = textureLoad(depth, vec2<i32>(source_pixel), 0);
    if scene_depth <= 0.00001 {
        textureStore(output_ssr, vec2<i32>(invocation.xy), vec4<f32>(0.0));
        return;
    }

    let surface = textureLoad(pbr, vec2<i32>(source_pixel), 0);
    let roughness = surface.x;
    if roughness > 0.82 {
        textureStore(output_ssr, vec2<i32>(invocation.xy), vec4<f32>(0.0));
        return;
    }

    let normal = normalize(textureLoad(normals, vec2<i32>(source_pixel), 0).xyz);
    let uv = (vec2<f32>(source_pixel) + 0.5) / vec2<f32>(source_size);
    let world_position = reconstruct_world_position(uv, scene_depth);
    let view_ray = safe_normalize(world_position - camera.position_time.xyz);
    let reflected = safe_normalize(reflect(view_ray, normal));
    let hit = trace_screen_space_reflection(world_position + normal * 0.045, reflected, source_size);
    let roughness_weight = mix(1.0, 0.54, roughness);
    textureStore(output_ssr, vec2<i32>(invocation.xy), vec4<f32>(hit.radiance * roughness_weight, hit.hit));
}
