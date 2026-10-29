struct DisplaySettings {
    gamma_brightness: vec4<f32>,
};

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var ui_overlay: texture_2d<f32>;
@group(0) @binding(2) var<uniform> display: DisplaySettings;

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
    let pixel = vec2<i32>(input.position.xy);
    let scene = textureLoad(scene_color, pixel, 0).rgb;
    let ui = textureLoad(ui_overlay, pixel, 0);
    let combined = max(scene * (1.0 - ui.a) + ui.rgb, vec3<f32>(0.0));
    let display_gamma = clamp(display.gamma_brightness.x, 1.0, 3.0);
    let brightness = max(display.gamma_brightness.y, 0.0);
    let corrected = pow(combined, vec3<f32>(2.2 / display_gamma)) * brightness;
    return vec4<f32>(corrected, 1.0);
}
