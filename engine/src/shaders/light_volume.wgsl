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

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> lights: Lights;

const CUBE: array<vec3<f32>, 36> = array<vec3<f32>, 36>(
    vec3(-1,-1, 1),vec3( 1,-1, 1),vec3( 1, 1, 1), vec3(-1,-1, 1),vec3( 1, 1, 1),vec3(-1, 1, 1),
    vec3( 1,-1,-1),vec3(-1,-1,-1),vec3(-1, 1,-1), vec3( 1,-1,-1),vec3(-1, 1,-1),vec3( 1, 1,-1),
    vec3(-1,-1,-1),vec3(-1,-1, 1),vec3(-1, 1, 1), vec3(-1,-1,-1),vec3(-1, 1, 1),vec3(-1, 1,-1),
    vec3( 1,-1, 1),vec3( 1,-1,-1),vec3( 1, 1,-1), vec3( 1,-1, 1),vec3( 1, 1,-1),vec3( 1, 1, 1),
    vec3(-1, 1, 1),vec3( 1, 1, 1),vec3( 1, 1,-1), vec3(-1, 1, 1),vec3( 1, 1,-1),vec3(-1, 1,-1),
    vec3(-1,-1,-1),vec3( 1,-1,-1),vec3( 1,-1, 1), vec3(-1,-1,-1),vec3( 1,-1, 1),vec3(-1,-1, 1)
);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> @builtin(position) vec4<f32> {
    let volume = lights.local_position_radius[instance_index];
    return camera.view_projection * vec4<f32>(volume.xyz + CUBE[vertex_index] * volume.w, 1.0);
}
