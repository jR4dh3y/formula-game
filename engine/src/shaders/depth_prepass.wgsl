struct Camera {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    position_time: vec4<f32>,
    viewport: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> @invariant @builtin(position) vec4<f32> {
    return camera.view_projection * vec4<f32>(input.position, 1.0);
}
