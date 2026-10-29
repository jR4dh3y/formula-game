// Re-transforms moving instances on the GPU: object-space vertices are uploaded
// once; each frame only per-instance matrices change. Output is written into
// the same world-space vertex and previous-position buffers the raster passes
// read, so no other pipeline needs to know about instances.
//
// Vertex layout (13 words, matches GpuVertex):
//   0-2 position, 3-5 normal, 6-7 uv, 8 packed rgba8 color, 9-12 tangent + sign.

struct Job {
    current: mat4x4<f32>,
    previous: mat4x4<f32>,
    start: u32,
    count: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<storage, read> local_vertices: array<u32>;
@group(0) @binding(1) var<storage, read_write> world_vertices: array<u32>;
@group(0) @binding(2) var<storage, read_write> previous_positions: array<f32>;
@group(0) @binding(3) var<storage, read> jobs: array<Job>;

const STRIDE: u32 = 13u;

fn load3(base: u32) -> vec3<f32> {
    return vec3<f32>(
        bitcast<f32>(local_vertices[base]),
        bitcast<f32>(local_vertices[base + 1u]),
        bitcast<f32>(local_vertices[base + 2u]),
    );
}

fn store3(base: u32, value: vec3<f32>) {
    world_vertices[base] = bitcast<u32>(value.x);
    world_vertices[base + 1u] = bitcast<u32>(value.y);
    world_vertices[base + 2u] = bitcast<u32>(value.z);
}

fn safe_normalize(value: vec3<f32>) -> vec3<f32> {
    let magnitude = length(value);
    return select(vec3<f32>(0.0), value / max(magnitude, 0.000001), magnitude > 0.000001);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let job = jobs[id.y];
    if id.x >= job.count {
        return;
    }
    let vertex = job.start + id.x;
    let base = vertex * STRIDE;
    let position = load3(base);
    let normal = load3(base + 3u);
    let tangent = load3(base + 9u);

    let world = job.current * vec4<f32>(position, 1.0);
    let previous = job.previous * vec4<f32>(position, 1.0);
    store3(base, world.xyz);
    store3(base + 3u, safe_normalize((job.current * vec4<f32>(normal, 0.0)).xyz));
    world_vertices[base + 6u] = local_vertices[base + 6u];
    world_vertices[base + 7u] = local_vertices[base + 7u];
    world_vertices[base + 8u] = local_vertices[base + 8u];
    store3(base + 9u, safe_normalize((job.current * vec4<f32>(tangent, 0.0)).xyz));
    world_vertices[base + 12u] = local_vertices[base + 12u];

    previous_positions[vertex * 3u] = previous.x;
    previous_positions[vertex * 3u + 1u] = previous.y;
    previous_positions[vertex * 3u + 2u] = previous.z;
}
