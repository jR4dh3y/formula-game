struct AntiAlias {
    previous_view_projection: mat4x4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> aa: AntiAlias;

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

fn glyph_mask(glyph: u32) -> u32 {
    switch glyph {
        case 0u: { return 0x7b6fu; }
        case 1u: { return 0x749au; }
        case 2u: { return 0x73e7u; }
        case 3u: { return 0x79e7u; }
        case 4u: { return 0x49edu; }
        case 5u: { return 0x79cfu; }
        case 6u: { return 0x7bcfu; }
        case 7u: { return 0x24a7u; }
        case 8u: { return 0x7befu; }
        case 9u: { return 0x79efu; }
        case 10u: { return 0x13cfu; }
        case 11u: { return 0x12ebu; }
        case 12u: { return 0x79cfu; }
        default: { return 0u; }
    }
}

fn overlay_glyph(slot: i32, fps: u32) -> u32 {
    if slot == 0 { return 10u; }
    if slot == 1 { return 11u; }
    if slot == 2 { return 12u; }
    if slot == 4 { return fps / 100u; }
    if slot == 5 { return (fps / 10u) % 10u; }
    if slot == 6 { return fps % 10u; }
    return 99u;
}

fn glyph_coverage(pixel: vec2<i32>) -> f32 {
    let scale = 3;
    let glyph_size = vec2<i32>(3 * scale, 5 * scale);
    let gap = 2 * scale;
    let local = pixel - vec2<i32>(14, 14);
    if any(local < vec2<i32>(0)) { return 0.0; }
    let stride = glyph_size.x + gap;
    let slot = local.x / stride;
    if slot < 0 || slot > 6 || local.y >= glyph_size.y { return 0.0; }
    let glyph_local = vec2<i32>(local.x - slot * stride, local.y);
    if glyph_local.x >= glyph_size.x { return 0.0; }
    let cell = glyph_local / scale;
    let glyph = overlay_glyph(slot, u32(clamp(aa.params.w + 0.5, 0.0, 999.0)));
    return f32((glyph_mask(glyph) >> u32(cell.y * 3 + cell.x)) & 1u);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.position.xy);
    let in_panel = all(pixel >= vec2<i32>(9, 9)) && all(pixel < vec2<i32>(105, 36));
    let panel_alpha = select(0.0, 0.58, in_panel);
    var premultiplied = vec3<f32>(0.012, 0.016, 0.012) * panel_alpha;
    var alpha = panel_alpha;
    let glyph = glyph_coverage(pixel);
    premultiplied = mix(premultiplied, vec3<f32>(1.0, 0.82, 0.34), glyph);
    alpha = mix(alpha, 1.0, glyph);
    return vec4<f32>(premultiplied, alpha);
}
