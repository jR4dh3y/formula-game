//! Runtime asphalt grain, surface calibration and a faint rubbered line; source GLBs stay untouched.
use crate::{menu::Frontend, Session};
use bevy::{
    asset::RenderAssetUsages,
    gltf::GltfMaterialName,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, UvChannel},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use std::collections::HashMap;

#[derive(Resource)]
pub struct Detail {
    normal: Handle<Image>,
    roughness: Handle<Image>,
}
#[derive(Component)]
pub struct Ready;
#[derive(Component)]
pub struct RubberLine;

fn grain(x: u32, y: u32) -> f32 {
    // Two avalanche rounds: a single linear mix leaves diagonal lattice lines.
    let mut v = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77).rotate_left(15);
    v = (v ^ (v >> 16)).wrapping_mul(0x7FEB_352D);
    v = (v ^ (v >> 15)).wrapping_mul(0x846C_A68B);
    ((v ^ (v >> 16)) & 65535) as f32 / 65535.
}

/// Tiling aggregate height field: white noise box-blurred at two radii, so
/// there are neither per-texel sparkles nor a cell lattice to alias into lines.
fn heights(size: u32) -> Vec<f32> {
    let n = size as usize;
    let blur = |source: &[f32], radius: usize| {
        let pass = |source: &[f32], horizontal: bool| {
            let mut out = vec![0.; n * n];
            for y in 0..n {
                for x in 0..n {
                    let mut sum = 0.;
                    for d in 0..=2 * radius {
                        let (sx, sy) = if horizontal {
                            ((x + n + d - radius) % n, y)
                        } else {
                            (x, (y + n + d - radius) % n)
                        };
                        sum += source[sy * n + sx];
                    }
                    out[y * n + x] = sum / (2 * radius + 1) as f32;
                }
            }
            out
        };
        // Two box passes per axis approximate a Gaussian.
        let once = pass(&pass(source, true), false);
        let mut twice = pass(&pass(&once, true), false);
        let (lo, hi) = twice
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        for v in &mut twice {
            *v = (*v - lo) / (hi - lo).max(1e-6);
        }
        twice
    };
    let white = |seed: u32| -> Vec<f32> {
        (0..size * size)
            .map(|i| grain(i % size + seed * 7919, i / size + seed * 104_729))
            .collect()
    };
    let fine = blur(&white(1), 1);
    let coarse = blur(&white(2), 9);
    fine.iter()
        .zip(&coarse)
        .map(|(fine, coarse)| fine * 0.6 + coarse * 0.4)
        .collect()
}

pub fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = 256_u32;
    let mut normal = Vec::new();
    let mut roughness = Vec::new();
    let field = heights(size);
    let height = |x: u32, y: u32, size: u32| field[(y % size * size + x % size) as usize];
    for y in 0..size {
        for x in 0..size {
            let h = height(x, y, size);
            let n = Vec3::new(
                (h - height((x + 1) % size, y, size)) * 0.6,
                (h - height(x, (y + 1) % size, size)) * 0.6,
                1.,
            )
            .normalize();
            normal.extend_from_slice(&[
                ((n.x * 0.5 + 0.5) * 255.) as u8,
                ((n.y * 0.5 + 0.5) * 255.) as u8,
                (n.z * 255.) as u8,
                255,
            ]);
            roughness.extend_from_slice(&[255, (175. + h * 65.) as u8, 0, 255]);
        }
    }
    let mut make = |pixels| {
        let mut image = Image::new(
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        crate::textures::generate_mips(&mut image);
        let mut sampler = ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::linear()
        };
        sampler.set_anisotropic_filter(16);
        image.sampler = ImageSampler::Descriptor(sampler);
        images.add(image)
    };
    commands.insert_resource(Detail {
        normal: make(normal),
        roughness: make(roughness),
    });
}

type Pending<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GltfMaterialName,
        &'static MeshMaterial3d<StandardMaterial>,
        &'static Mesh3d,
    ),
    Without<Ready>,
>;
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    mut commands: Commands,
    surfaces: Pending,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    detail: Res<Detail>,
    frontend: Res<Frontend>,
    mut originals: Local<HashMap<AssetId<StandardMaterial>, (String, StandardMaterial)>>,
    mut applied: Local<Option<bool>>,
    ready: Query<Entity, With<Ready>>,
) {
    originals.retain(|id, _| materials.contains(*id));
    let enabled = frontend.video.surface_detail;
    let changed = *applied != Some(enabled);
    *applied = Some(enabled);
    for (entity, name, handle, mesh_handle) in &surfaces {
        let name = name.0.to_ascii_lowercase();
        if !name.starts_with("ground:") && !name.starts_with("kerb:") {
            commands.entity(entity).insert(Ready);
            continue;
        }
        let Some(material) = materials.get(&handle.0) else {
            continue;
        };
        originals
            .entry(handle.0.id())
            .or_insert_with(|| (name.clone(), material.clone()));
        let asphalt = name.contains("road") || name.contains("asphalt");
        if asphalt && enabled {
            let Some(mut mesh) = meshes.get_mut(&mesh_handle.0) else {
                continue;
            };
            if !mesh.contains_attribute(Mesh::ATTRIBUTE_UV_1) {
                let Some(points) = mesh
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .and_then(|v| v.as_float3())
                else {
                    continue;
                };
                let uv: Vec<[f32; 2]> = points.iter().map(|p| [p[0] * 0.37, p[2] * 0.37]).collect();
                let tangents = vec![[1., 0., 0., -1.]; uv.len()];
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
                // The generated isotropic aggregate normals use world-X/world-Z UVs.
                mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
            }
        }
        if let Some((name, original)) = originals.get(&handle.0.id()) {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                *material = calibrate(name, original, &detail, enabled);
            }
        }
        commands.entity(entity).insert(Ready);
    }
    if changed {
        for entity in &ready {
            commands.entity(entity).remove::<Ready>();
        }
        for (id, (name, original)) in originals.iter() {
            if let Some(mut material) = materials.get_mut(*id) {
                *material = calibrate(name, original, &detail, enabled);
            }
        }
    }
}

fn calibrate(
    name: &str,
    original: &StandardMaterial,
    detail: &Detail,
    enabled: bool,
) -> StandardMaterial {
    let mut m = original.clone();
    if !enabled {
        return m;
    }
    if name.contains("road") || name.contains("asphalt") {
        m.normal_map_texture = Some(detail.normal.clone());
        m.normal_map_channel = UvChannel::Uv1;
        m.metallic_roughness_texture = Some(detail.roughness.clone());
        m.metallic_roughness_channel = UvChannel::Uv1;
        m.perceptual_roughness = 0.7;
        m.metallic = 0.;
        m.reflectance = 0.5;
    } else if name.contains("kerb") {
        m.perceptual_roughness = 0.48;
        m.reflectance = 0.5;
    } else if name.contains("grass") {
        m.perceptual_roughness = 0.95;
        m.reflectance = 0.3;
    }
    m
}

pub fn rubber_line(
    mut commands: Commands,
    session: Res<Session>,
    frontend: Res<Frontend>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut installed: Local<Option<usize>>,
    mut lines: Query<&mut Visibility, With<RubberLine>>,
) {
    for mut visibility in &mut lines {
        *visibility = if frontend.video.surface_detail {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if !session.ready {
        return;
    }
    let key = std::sync::Arc::as_ptr(&session.race.track) as usize;
    if *installed == Some(key) {
        return;
    }
    *installed = Some(key);
    let track = &session.race.track;
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let mut normals = Vec::new();
    for (i, station) in track.stations.iter().enumerate() {
        let curvature = track.racing_line.curvature[i].abs();
        let opacity = (curvature as f32 * 12.).clamp(0.015, 0.11);
        for (offset, alpha) in [(-1.5, 0.), (-0.85, opacity), (0.85, opacity), (1.5, 0.)] {
            let mut point = station.point_at(track.racing_line.offsets[i] + offset);
            if let Some(hit) = track.collision.ground(point + glam::DVec3::Y) {
                point.y = hit.height + 0.012;
            }
            positions.push(point.as_vec3().to_array());
            normals.push([0., 1., 0.]);
            colors.push([1., 1., 1., alpha]);
        }
        if i > 0 {
            let start = (i * 4) as u32;
            for j in 0..3 {
                indices.extend_from_slice(&[
                    start + j - 4,
                    start + j,
                    start + j + 1,
                    start + j - 4,
                    start + j + 1,
                    start + j - 3,
                ]);
            }
        }
    }
    let mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices));
    commands.spawn((
        RubberLine,
        crate::assets::RaceVisualRoot,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.035, 0.032, 0.03),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.48,
            cull_mode: None,
            ..default()
        })),
        Transform::IDENTITY,
        if frontend.video.surface_detail {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        },
        bevy::light::NotShadowCaster,
    ));
}
