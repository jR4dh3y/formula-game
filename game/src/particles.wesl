// Tyre smoke, verge dust and sparks: one instanced quad per particle. The mesh
// tag carries the particle's age, seed and strength.
//
// A puff is a camera-facing quad shaded as the sphere it bounds: thickness
// from the sphere's chord, broken up by a tiling cloud texture, lit by the sun
// across a lumpy normal, and cut against the scene depth along that chord so
// it settles on the road and wraps the car instead of slicing through them.
// Two texture samples and one depth load per pixel.

import bevy_pbr::render::{
    mesh_functions,
    mesh_view_bindings::{view, lights},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    view_transformations::{position_world_to_clip, position_world_to_view, depth_ndc_to_view_z},
    forward_io::{Vertex, decompress_vertex},
    shadows::{get_cascade_index, world_to_directional_light_local},
    shadow_sampling::sample_shadow_map_hardware,
};

struct Particles {
    // Smoke: albedo and peak optical depth. Sparks: colour gain and peak radiance.
    tint: vec4<f32>,
    // Sky fill radiance; w > 0.5 selects sparks.
    ambient: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material: Particles;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var cloud_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var cloud_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    // Quad coordinates, -1..1.
    @location(1) uv: vec2<f32>,
    // age 0..1, seed 0..1, strength 0..1, radius in metres.
    @location(2) @interpolate(flat) data: vec4<f32>,
    // The quad's axes in the world, and its centre's distance from the eye.
    @location(3) @interpolate(flat) axis_x: vec4<f32>,
    @location(4) @interpolate(flat) axis_y: vec3<f32>,
    @location(5) @interpolate(flat) centre: vec3<f32>,
}

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let tag = mesh_functions::get_tag(vertex_in.instance_index);
    let age = f32(tag & 1023u) / 1023.0;
    let seed = f32((tag >> 10u) & 255u) / 255.0;
    let strength = f32((tag >> 18u) & 255u) / 255.0;

    let centre = world_from_local[3].xyz;
    let corner = vertex.position.xy * 2.0;
    let half = vec2(length(world_from_local[0].xyz), length(world_from_local[1].xyz)) * 0.5;
    let away = centre - view.world_position;
    let distance = max(length(away), 1e-3);
    let ahead = away / distance;

    var out: VertexOutput;
    out.centre = centre;
    if material.ambient.w > 0.5 {
        // A streak: long axis along the motion, width turned to the eye.
        let axis = normalize(world_from_local[1].xyz);
        let side = normalize(cross(axis, -ahead) + vec3(0.0, 1e-5, 0.0));
        // Never thinner than about a pixel and a half, or a distant shower
        // flickers and dims away between samples.
        // The quad is four cores wide, to hold the glow around the core.
        let width = max(half.x, distance * 0.0016) * 4.0;
        let reach = half.y + width;
        out.world_position = centre + axis * corner.y * reach + side * corner.x * width;
        out.axis_x = vec4(side, distance);
        out.axis_y = axis;
    } else {
        // Face the eye, each puff turned its own way and slowly rolling.
        let right = normalize(cross(ahead, vec3(0.0, 1.0, 0.0)) + vec3(1e-5, 0.0, 0.0));
        let up = cross(right, ahead);
        let turn = seed * 6.2832 + (seed - 0.5) * age * 2.4;
        let axis_x = right * cos(turn) + up * sin(turn);
        let axis_y = up * cos(turn) - right * sin(turn);
        out.world_position = centre + (axis_x * corner.x + axis_y * corner.y) * half.x;
        out.axis_x = vec4(axis_x, distance);
        out.axis_y = axis_y;
    }
    out.position = position_world_to_clip(out.world_position);
    out.uv = corner;
    out.data = vec4(age, seed, strength, half.x);
    return out;
}

fn spark(in: VertexOutput) -> vec4<f32> {
    let age = in.data.x;
    let strength = in.data.z;
    // Bright rounded head, tail thinning to nothing behind it.
    let tail = smoothstep(-1.0, 0.75, in.uv.y);
    let head = 1.0 - smoothstep(0.8, 1.0, in.uv.y);
    let width = mix(0.3, 1.0, tail);
    let offset = in.uv.x * 4.0;
    let core = exp(-offset * offset * 2.5 / (width * width));
    // A faint glow round the core, so a spark reads as hot even where the
    // frame's bloom is turned down.
    let across = core + 0.1 * exp(-in.uv.x * in.uv.x * 4.0);
    // Titanium burns white-yellow, then cools through orange to a dull red.
    let hot = vec3(1.0, 0.86, 0.6);
    let warm = vec3(1.0, 0.46, 0.1);
    let dull = vec3(0.8, 0.13, 0.02);
    var colour = mix(hot, warm, smoothstep(0.1, 0.55, age));
    colour = mix(colour, dull, smoothstep(0.55, 1.0, age));
    // The core of a young streak is hotter than its edges.
    colour = mix(colour, hot, core * tail * (1.0 - age) * 0.7);
    let glow = pow(1.0 - age, 1.4) * (0.3 + 0.7 * strength);
    let radiance = colour * material.tint.rgb * material.tint.a * glow * across * tail * tail * head;
    // Alpha 0 with premultiplied blending is purely additive.
    return vec4(radiance, 0.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if material.ambient.w > 0.5 {
        return spark(in);
    }
    let age = in.data.x;
    let seed = in.data.y;
    let strength = in.data.z;
    let radius = in.data.w;
    let distance = in.axis_x.w;

    let fill = 1.0 - dot(in.uv, in.uv);
    if fill <= 0.0 {
        discard;
    }
    // Half the sphere's chord under this pixel, in radii.
    let chord = sqrt(fill);

    // The cloud is fixed to the puff in metres, so a growing puff uncovers
    // more of it instead of stretching it; it rolls slowly as the puff ages.
    let at = in.uv * radius;
    let offset = vec2(seed * 7.31, seed * 3.17);
    let billow = textureSample(cloud_texture, cloud_sampler, at * 0.17 + offset + vec2(0.0, age * 0.08));
    let wisps = textureSample(cloud_texture, cloud_sampler, at * 0.52 - offset * 1.7 - vec2(age * 0.11, 0.0)).g;

    // Young smoke is dense; it thins as it swells, and its faint parts wear
    // away first, so an old puff breaks into wisps.
    let wear = mix(0.04, 0.62, age * age);
    var density = chord * (0.3 + 1.25 * billow.r) + (wisps - 0.5) * 0.7 * (0.35 + 0.65 * chord) - wear;
    // Fade the rim out whatever the cloud says there: no puff shows its disc.
    density = max(density, 0.0) * smoothstep(0.0, 0.3, fill);

    // The share of the chord in front of the scene and beyond the lens.
    let depth = -depth_ndc_to_view_z(in.position.z);
    let reach = chord * radius * 0.8;
    var clear = clamp((depth + reach - 0.25) / (2.0 * reach + 1e-3), 0.0, 1.0);
@if(DEPTH_PREPASS) {
    let scene = -depth_ndc_to_view_z(bevy_pbr::prepass::utils::prepass_depth(in.position, 0u));
    clear = min(clear, clamp((scene - depth + reach) / (2.0 * reach + 1e-3), 0.0, 1.0));
}
    // A puff the camera drives into thins out rather than popping.
    let near = smoothstep(0.4, 0.4 + 2.5 * radius, distance);

    let life = smoothstep(0.0, 0.05, age) * (1.0 - smoothstep(0.45, 1.0, age));
    let thickness = material.tint.a * (0.3 + 0.7 * strength) * life
        * pow(0.3 / max(radius, 0.05), 0.6);
    let depth_through = density * clear * thickness;
    let alpha = (1.0 - exp(-depth_through)) * near;
    if alpha <= 0.004 {
        discard;
    }

    // Light it as a lumpy ball: the cloud's slope tilts the sphere's normal.
    let ray = normalize(in.world_position - view.world_position);
    let slope = (billow.ba - 0.5) * 2.2;
    let tilted = normalize(vec3(in.uv - slope, chord + 0.35));
    let normal = in.axis_x.xyz * tilted.x + in.axis_y * tilted.y - ray * tilted.z;
    var to_sun = vec3(0.0, 1.0, 0.0);
    var sunlight = vec3(0.0);
    if lights.n_directional_lights > 0u {
        let sun = lights.directional_lights[0];
        to_sun = sun.direction_to_light;
        var shadow = 1.0;
        if (sun.flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            // One unfiltered shadow lookup for the whole puff, at its
            // centre: every pixel of it reads the same texel, and a puff
            // crossing a shadow's edge dims as a body.
            let cascade = get_cascade_index(0u, position_world_to_view(in.centre).z);
            if cascade < sun.num_cascades {
                let local = world_to_directional_light_local(
                    0u,
                    cascade,
                    vec4(in.centre + to_sun * 0.3, 1.0),
                );
                if local.w != 0.0 {
                    shadow = sample_shadow_map_hardware(
                        local.xy,
                        local.z,
                        i32(sun.depth_texture_base_index + cascade),
                    );
                }
            }
        }
        sunlight = sun.color.rgb * (0.3183 * (0.12 + 0.88 * shadow));
    }
    // Sunlight wraps well round a cloud, and the thick middle shades itself.
    let facing = dot(normal, to_sun);
    let wrapped = clamp((facing + 0.55) / 1.55, 0.0, 1.0);
    let buried = exp(-depth_through * 0.55);
    // Light also scatters many times inside: the shaded side is never dark.
    let direct = (0.34 + 0.66 * wrapped * wrapped) * (0.6 + 0.4 * buried);
    // Smoke scatters forward: thin edges glow when the sun is behind them.
    let glare = pow(max(dot(ray, to_sun), 0.0), 5.0) * buried * 1.3;
    let sky = material.ambient.rgb * (0.75 + 0.35 * normal.y) * (0.7 + 0.3 * buried) * 1.5;
    let light = sunlight * (direct + glare) + sky;
    // Premultiplied: the light already carries its own coverage.
    return vec4(light * material.tint.rgb * view.exposure * alpha, alpha);
}
