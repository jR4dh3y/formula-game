@group(0) @binding(0) var albedo_ms: texture_multisampled_2d<f32>;
@group(0) @binding(1) var normal_ms: texture_multisampled_2d<f32>;
@group(0) @binding(2) var pbr_ms: texture_multisampled_2d<f32>;
@group(0) @binding(3) var motion_ms: texture_multisampled_2d<f32>;
@group(0) @binding(4) var depth_ms: texture_depth_multisampled_2d;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

struct ResolveOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) pbr: vec4<f32>,
    @location(3) motion: vec2<f32>,
    @location(4) coverage: f32,
    @builtin(frag_depth) depth: f32,
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
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> ResolveOutput {
    let pixel = vec2<i32>(input.position.xy);
    let depth0 = textureLoad(depth_ms, pixel, 0);
    let depth1 = textureLoad(depth_ms, pixel, 1);
    let covered0 = depth0 > 0.000001;
    let covered1 = depth1 > 0.000001;
    if !covered0 && !covered1 {
        var empty: ResolveOutput;
        empty.albedo = vec4<f32>(0.0);
        empty.normal = vec4<f32>(0.0);
        empty.pbr = vec4<f32>(0.0);
        empty.motion = vec2<f32>(0.0);
        empty.coverage = 0.0;
        empty.depth = 0.0;
        return empty;
    }
    let closest_sample = select(1, 0, depth0 >= depth1);
    let same_surface = covered0 && covered1 && abs(depth0 - depth1) < 0.00035;

    var albedo = textureLoad(albedo_ms, pixel, closest_sample);
    var normal = textureLoad(normal_ms, pixel, closest_sample);
    var pbr = textureLoad(pbr_ms, pixel, closest_sample);
    var motion = textureLoad(motion_ms, pixel, closest_sample).rg;
    if same_surface {
        albedo = (textureLoad(albedo_ms, pixel, 0) + textureLoad(albedo_ms, pixel, 1)) * 0.5;
        normal = (textureLoad(normal_ms, pixel, 0) + textureLoad(normal_ms, pixel, 1)) * 0.5;
        normal = vec4<f32>(normalize(normal.xyz), normal.w);
        pbr = (textureLoad(pbr_ms, pixel, 0) + textureLoad(pbr_ms, pixel, 1)) * 0.5;
        motion = (textureLoad(motion_ms, pixel, 0).rg + textureLoad(motion_ms, pixel, 1).rg) * 0.5;
    }

    let sample_coverage = (select(0.0, 1.0, covered0) + select(0.0, 1.0, covered1)) * 0.5;
    var output: ResolveOutput;
    output.albedo = albedo;
    output.normal = normal;
    output.pbr = pbr;
    output.motion = motion;
    output.coverage = sample_coverage;
    output.depth = max(depth0, depth1);
    return output;
}
