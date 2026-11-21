//! Preserve the full GEN22 geometry while reusing transformed vertices between triangles.

use bevy::{asset::RenderAssetUsages, mesh::Indices, prelude::*};

/// Mesh bytes released to the renderer per frame. A whole car's meshes
/// extracted and packed into vertex slabs together held a frame for 200 ms.
const RELEASED_BYTES_PER_FRAME: usize = 8 << 20;

/// Car meshes whose index order is still being optimised, or that wait their
/// turn to be drawn.
#[derive(Resource, Default)]
pub struct MeshJobs {
    pub pending: usize,
    running: Vec<(AssetId<Mesh>, RenderAssetUsages, bevy::tasks::Task<Indices>)>,
}

/// Reorders newly loaded car meshes on worker threads; in the frame they
/// arrived in, this held the window for over half a second.
///
/// The mesh stays in place meanwhile, because scene setup reads and copies
/// its vertices, but is marked main-world-only so the renderer does not take
/// the unordered copy. Its indices return together with its real usage, a few
/// meshes per frame, and that is its only upload. A copy made in the meantime
/// must set its own usage: see [`renderable`].
pub fn prepare(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    server: Res<AssetServer>,
    mut jobs: ResMut<MeshJobs>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        if !server.get_path(*id).is_some_and(|path| {
            path.path().starts_with("cars/gen22")
                && path
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "glb")
        }) {
            continue;
        }
        // Untracked: withholding the mesh must not announce itself as a change.
        let Some(mesh) = meshes.get_mut_untracked(*id) else {
            continue;
        };
        if mesh.primitive_topology() != bevy::mesh::PrimitiveTopology::TriangleList {
            continue;
        }
        let Some(mut indices) = mesh.indices().cloned() else {
            continue;
        };
        let usage = std::mem::replace(&mut mesh.asset_usage, RenderAssetUsages::MAIN_WORLD);
        let vertices = mesh.count_vertices();
        let task = crate::textures::workers().spawn(async move {
            optimize(&mut indices, vertices);
            indices
        });
        jobs.running.push((*id, usage, task));
    }
    let mut running = std::mem::take(&mut jobs.running);
    let mut released = 0;
    running.retain_mut(|(id, usage, task)| {
        if released >= RELEASED_BYTES_PER_FRAME {
            return true;
        }
        let Some(indices) = bevy::tasks::futures::check_ready(task) else {
            return true;
        };
        // Tracked, so this change is what first extracts the mesh. A scene
        // dropped meanwhile no longer has it.
        if let Some(mut mesh) = meshes.get_mut(*id) {
            mesh.insert_indices(indices);
            mesh.asset_usage = *usage;
            let vertex_bytes: usize = mesh
                .attributes()
                .map(|(attribute, _)| attribute.format.size() as usize)
                .sum();
            released += vertex_bytes * mesh.count_vertices()
                + mesh.get_index_buffer_bytes().map_or(0, <[u8]>::len);
        }
        false
    });
    jobs.running = running;
    jobs.pending = jobs.running.len();
}

/// A copy of a scene mesh that draws whether or not the original has been
/// released to the renderer yet.
pub fn renderable(mesh: &Mesh) -> Mesh {
    let mut mesh = mesh.clone();
    mesh.asset_usage = RenderAssetUsages::default();
    mesh
}

fn optimize(indices: &mut Indices, vertices: usize) {
    match indices {
        Indices::U32(indices) => meshopt::optimize_vertex_cache_in_place(indices, vertices),
        Indices::U16(indices) => {
            let original: Vec<_> = indices.iter().map(|&index| u32::from(index)).collect();
            *indices = meshopt::optimize_vertex_cache(&original, vertices)
                .into_iter()
                .map(|index| index as u16)
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reordering_preserves_every_triangle_winding_and_vertex_attribute() {
        for narrow in [false, true] {
            let positions: Vec<_> = (0..40).map(|i| [i as f32, (i % 2) as f32, 0.]).collect();
            let indices: Vec<u32> = (0..36).flat_map(|i| [i, i + 1, i + 2]).collect();
            let mut mesh = Mesh::new(
                bevy::mesh::PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone())
            .with_inserted_indices(if narrow {
                Indices::U16(indices.iter().map(|&index| index as u16).collect())
            } else {
                Indices::U32(indices.clone())
            });
            let vertices = mesh.count_vertices();
            optimize(mesh.indices_mut().unwrap(), vertices);
            let result: Vec<_> = mesh.indices().unwrap().iter().map(|i| i as u32).collect();
            let triangles = |values: &[u32]| {
                let mut triangles: Vec<_> = values
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|t| [t[0], t[1], t[2]])
                    .collect();
                triangles.sort_unstable();
                triangles
            };
            assert_eq!(triangles(&indices), triangles(&result));
            assert_eq!(mesh.count_vertices(), positions.len());
            assert_eq!(
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                    .unwrap()
                    .as_float3()
                    .unwrap(),
                positions
            );
        }
    }
}
