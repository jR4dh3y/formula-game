// Composites the game's CPU-drawn HUD (straight-alpha sRGB, decoded to linear
// by the texture format) into the premultiplied linear UI overlay.
@group(0) @binding(0) var hud: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
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
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureLoad(hud, vec2<i32>(input.position.xy), 0);
    return vec4<f32>(texel.rgb * texel.a, texel.a);
}
