struct Camera {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    position_time: vec4<f32>,
    viewport: vec4<f32>,
    previous_view_projection: mat4x4<f32>,
};

struct Material {
    base_color: vec4<f32>,
    surface: vec4<f32>,
    emission: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_color_texture: texture_2d<f32>;
@group(1) @binding(2) var base_color_sampler: sampler;
@group(1) @binding(3) var normal_texture: texture_2d<f32>;
@group(1) @binding(4) var pbr_texture: texture_2d<f32>;
@group(1) @binding(5) var opacity_texture: texture_2d<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(5) previous_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) tangent: vec4<f32>,
};

struct VertexOutput {
    @invariant @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) tangent: vec4<f32>,
    @location(5) current_clip_position: vec4<f32>,
    @location(6) previous_clip_position: vec4<f32>,
};

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal_flags: vec4<f32>,
    @location(2) pbr_material: vec4<f32>,
    @location(3) motion_vectors: vec4<f32>,
};

const EMISSIVE_SCALE: f32 = 12.0;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.world_position = input.position;
    output.normal = normalize(input.normal);
    output.uv = input.uv;
    output.color = input.color;
    output.tangent = input.tangent;
    output.current_clip_position = output.clip_position;
    output.previous_clip_position = camera.previous_view_projection * vec4<f32>(input.previous_position, 1.0);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> GBufferOutput {
    var output: GBufferOutput;
    let sampled = textureSample(base_color_texture, base_color_sampler, input.uv);
    let material_flags = u32(round(material.surface.w));
    if (material_flags & 4u) != 0u {
        let opacity = textureSample(opacity_texture, base_color_sampler, input.uv).r;
        if opacity < 0.5 {
            discard;
        }
    }
    let sampled_normal = textureSample(normal_texture, base_color_sampler, input.uv).xyz * 2.0 - 1.0;
    let sampled_pbr = textureSample(pbr_texture, base_color_sampler, input.uv);
    let geometric_normal = normalize(input.normal);
    // Imported meshes may omit tangents. The importer's default X tangent is
    // parallel to X-facing walls; normalize(0) poisons the whole lighting path.
    let projected_tangent = input.tangent.xyz - geometric_normal * dot(input.tangent.xyz, geometric_normal);
    let reference = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(geometric_normal.x) > 0.9);
    let fallback_tangent = normalize(cross(reference, geometric_normal));
    let tangent = select(fallback_tangent, projected_tangent / max(length(projected_tangent), 0.000001), length(projected_tangent) > 0.000001);
    let bitangent = normalize(cross(geometric_normal, tangent)) * input.tangent.w;
    let mapped_normal = normalize(mat3x3<f32>(tangent, bitangent, geometric_normal) * sampled_normal);
    let base = material.base_color * input.color * sampled;
    let material_flag = material.surface.w;
    let emission_strength = material.emission.a;
    let emission_chroma = material.emission.rgb / max(max(material.emission.r, material.emission.g), max(material.emission.b, 0.0001));
    let emissive_mask = step(0.0001, emission_strength);
    output.albedo = vec4<f32>(mix(base.rgb, emission_chroma, emissive_mask), base.a);
    output.normal_flags = vec4<f32>(mapped_normal, material_flag);
    output.pbr_material = vec4<f32>(
        clamp(material.surface.x * sampled_pbr.g, 0.04, 1.0),
        clamp(material.surface.y * sampled_pbr.b, 0.0, 1.0),
        clamp(material.surface.z * sampled_pbr.r, 0.0, 1.0),
        clamp(emission_strength / EMISSIVE_SCALE, 0.0, 1.0)
    );
    let current_ndc = input.current_clip_position.xy / max(input.current_clip_position.w, 0.00001);
    let previous_ndc = input.previous_clip_position.xy / max(input.previous_clip_position.w, 0.00001);
    output.motion_vectors = vec4<f32>((current_ndc - previous_ndc) * vec2<f32>(0.5, -0.5), 0.0, 1.0);
    return output;
}
