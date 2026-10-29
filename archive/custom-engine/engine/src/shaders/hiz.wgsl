@group(0) @binding(0) var depth_copy: texture_2d<f32>;
@group(0) @binding(1) var hiz_output: texture_storage_2d<rgba16float, write>;

fn load_depth(pixel: vec2<i32>, size: vec2<u32>) -> f32 {
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
    return textureLoad(depth_copy, clamped, 0).r;
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let output_size = textureDimensions(hiz_output);
    if any(invocation.xy >= output_size) {
        return;
    }
    let source_size = textureDimensions(depth_copy);
    let base = vec2<i32>(invocation.xy * 2u);
    let d0 = load_depth(base, source_size);
    let d1 = load_depth(base + vec2<i32>(1, 0), source_size);
    let d2 = load_depth(base + vec2<i32>(0, 1), source_size);
    let d3 = load_depth(base + vec2<i32>(1, 1), source_size);
    let min_depth = min(min(d0, d1), min(d2, d3));
    let max_depth = max(max(d0, d1), max(d2, d3));
    let discontinuity = clamp((max_depth - min_depth) * 64.0, 0.0, 1.0);
    textureStore(hiz_output, vec2<i32>(invocation.xy), vec4<f32>(min_depth, max_depth, discontinuity, 1.0));
}
