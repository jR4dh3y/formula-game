//! Driver mirrors: in the cockpit view the mirror glass shows a live rear view.
//!
//! Each mirror has a small rear-facing camera mounted on its glass. Both draw
//! into halves of one low-resolution image, and the glass shows its half
//! through a copy of its mesh whose texture coordinates run across the glass.
//! The view is that of a camera looking back, laid on the glass as seen from
//! the front: left and right swap, as they do in a mirror.
//!
//! It stays cheap: the cameras only run in the cockpit view, one of them per
//! frame, without antialiasing, post-processing or shadow maps.

use crate::{
    camera::Mode,
    materials::{CarSurface, SurfaceReady},
    Session,
};
use bevy::{
    camera::{CameraOutputMode, Exposure, Hdr, RenderTarget, Viewport},
    core_pipeline::tonemapping::Tonemapping,
    gltf::GltfMaterialName,
    light::{Cascades, EnvironmentMapLight, SimulationLightSystems, Skybox},
    mesh::VertexAttributeValues,
    pbr::DistanceFog,
    prelude::*,
    render::render_resource::TextureFormat,
};

/// Texels across one mirror's view.
const WIDTH: u32 = 256;
/// Horizontal field of view of a mirror, radians, and the tallest it may be.
const FIELD: f32 = 0.5;
const MAX_VERTICAL_FIELD: f32 = 0.42;
/// How far each mirror looks away from the car's centre line, radians.
const TOE_OUT: f32 = 0.1;
/// Metres a mirror sees: little further back is more than a texel tall.
const FAR: f32 = 600.;
/// Share of the scene's light a mirror returns.
const REFLECTANCE: f32 = 0.82;

pub struct MirrorPlugin;

impl Plugin for MirrorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (attach, update)
                .chain()
                .after(crate::materials::prepare)
                .after(crate::camera::update)
                .after(crate::sky::update),
        )
        .add_systems(
            PostUpdate,
            skip_shadow_maps
                .after(SimulationLightSystems::UpdateDirectionalLightCascades)
                .before(SimulationLightSystems::UpdateLightFrusta),
        );
    }
}

/// A rear-view camera; `slot` 0 is the car's left mirror, 1 its right.
#[derive(Component)]
pub struct MirrorCamera {
    slot: usize,
}

/// Mirror glass on the player's car, with the mesh and material it swaps
/// between: its own outside the cockpit, the live view inside.
#[derive(Component)]
pub struct MirrorGlass {
    plain: (Handle<Mesh>, Handle<StandardMaterial>),
    live: Handle<Mesh>,
}

/// The image both mirrors draw into, left half then right, and the material that shows it.
#[derive(Resource)]
pub struct Mirrors {
    image: Handle<Image>,
    material: Handle<StandardMaterial>,
    height: u32,
}

/// One mirror's glass within a mesh, in the mesh's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pane {
    /// 0 on the car's left (+x), 1 on its right.
    slot: usize,
    low: Vec3,
    high: Vec3,
}

impl Pane {
    fn centre(&self) -> Vec3 {
        (self.low + self.high) * 0.5
    }

    fn aspect(&self) -> f32 {
        let size = self.high - self.low;
        (size.x / size.y.max(1e-4)).clamp(0.5, 8.)
    }
}

/// The rear-facing glass on each side of the car in a mesh. The car's nose
/// points along +z, so glass facing the driver's eyes has normals toward -z;
/// housings, lamps and anything else facing forward or sideways are left out.
fn panes(positions: &[[f32; 3]], normals: &[[f32; 3]]) -> Vec<Pane> {
    let mut panes = Vec::new();
    for (slot, side) in [1_f32, -1.].into_iter().enumerate() {
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        let mut facing = 0.;
        let mut count = 0;
        for (position, normal) in positions.iter().zip(normals) {
            if position[0] * side <= 0. {
                continue;
            }
            count += 1;
            facing += normal[2];
            if normal[2] < -0.5 {
                low = low.min(Vec3::from_array(*position));
                high = high.max(Vec3::from_array(*position));
            }
        }
        let size = high - low;
        if count > 0 && facing / (count as f32) < -0.5 && size.x > 0.02 && size.y > 0.01 {
            panes.push(Pane { slot, low, high });
        }
    }
    panes
}

/// Where a point of the glass looks in the shared image. A camera looking
/// back has the car's left (+x) on its right, and the driver sees that same
/// edge of the glass on their left: the picture arrives mirrored.
fn glass_uv(position: Vec3, pane: &Pane) -> [f32; 2] {
    let across = ((position.x - pane.low.x) / (pane.high.x - pane.low.x)).clamp(0., 1.);
    let up = ((position.y - pane.low.y) / (pane.high.y - pane.low.y)).clamp(0., 1.);
    // Keep half a texel clear of the neighbouring mirror's half.
    let inset = 0.5 / WIDTH as f32;
    let across = inset + across * (1. - 2. * inset);
    [(pane.slot as f32 + across) * 0.5, 1. - up]
}

/// Texture coordinates for every vertex of a mesh holding these panes.
fn glass_uvs(positions: &[[f32; 3]], panes: &[Pane]) -> Vec<[f32; 2]> {
    positions
        .iter()
        .map(|position| {
            let slot = usize::from(position[0] <= 0.);
            panes
                .iter()
                .find(|pane| pane.slot == slot)
                .map_or([0., 0.], |pane| glass_uv(Vec3::from_array(*position), pane))
        })
        .collect()
}

/// The mirror camera's place on a pane: just behind the glass, looking back
/// and a little away from the car.
fn mount(pane: &Pane) -> Transform {
    let outward = if pane.slot == 0 { 1. } else { -1. };
    let centre = pane.centre();
    let eye = Vec3::new(centre.x, centre.y, pane.low.z - 0.02);
    let direction = Vec3::new(outward * TOE_OUT.sin(), 0., -TOE_OUT.cos());
    Transform::from_translation(eye).looking_to(direction, Vec3::Y)
}

/// Vertical field of view and image height for glass `aspect` times wider than tall.
fn framing(aspect: f32) -> (f32, u32) {
    let vertical = (2. * ((FIELD * 0.5).tan() / aspect).atan()).min(MAX_VERTICAL_FIELD);
    let height = ((WIDTH as f32 / aspect / 4.).round() as u32 * 4).clamp(32, WIDTH);
    (vertical, height)
}

type NewGlass<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GltfMaterialName,
        &'static CarSurface,
        &'static Mesh3d,
        &'static MeshMaterial3d<StandardMaterial>,
    ),
    Added<SurfaceReady>,
>;

/// Finds the mirror glass on a newly loaded player car and mounts its cameras.
#[allow(clippy::too_many_arguments)]
fn attach(
    mut commands: Commands,
    session: Res<Session>,
    surfaces: NewGlass,
    mut mirrors: Option<ResMut<Mirrors>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, name, surface, mesh, material) in &surfaces {
        if surface.entrant != session.player
            || surface.lod.is_some()
            || !name.0.to_ascii_lowercase().contains("mirror")
        {
            continue;
        }
        let Some(source) = meshes.get(&mesh.0) else {
            continue;
        };
        let (
            Some(VertexAttributeValues::Float32x3(positions)),
            Some(VertexAttributeValues::Float32x3(normals)),
        ) = (
            source.attribute(Mesh::ATTRIBUTE_POSITION),
            source.attribute(Mesh::ATTRIBUTE_NORMAL),
        )
        else {
            continue;
        };
        let found = panes(positions, normals);
        let Some(first) = found.first() else {
            continue;
        };
        let (field, height) = framing(first.aspect());
        let uvs = glass_uvs(positions, &found);
        let mut live = crate::meshes::renderable(source);
        live.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        let live = meshes.add(live);

        let image = match mirrors.as_deref() {
            Some(mirrors) if mirrors.height == height => mirrors.image.clone(),
            _ => {
                let image = images.add(Image::new_target_texture(
                    WIDTH * 2,
                    height,
                    TextureFormat::Rgba16Float,
                    None,
                ));
                let material = materials.add(StandardMaterial {
                    base_color: Color::linear_rgb(REFLECTANCE, REFLECTANCE, REFLECTANCE),
                    base_color_texture: Some(image.clone()),
                    unlit: true,
                    ..default()
                });
                let created = Mirrors {
                    image: image.clone(),
                    material,
                    height,
                };
                match mirrors.as_deref_mut() {
                    Some(mirrors) => *mirrors = created,
                    None => commands.insert_resource(created),
                }
                image
            }
        };
        commands.entity(entity).insert(MirrorGlass {
            plain: (mesh.0.clone(), material.0.clone()),
            live,
        });
        for pane in &found {
            let slot = pane.slot as u32;
            commands.entity(entity).with_child((
                MirrorCamera { slot: pane.slot },
                Camera3d::default(),
                Camera {
                    order: -2 + pane.slot as isize,
                    is_active: false,
                    viewport: Some(Viewport {
                        physical_position: UVec2::new(slot * WIDTH, 0),
                        physical_size: UVec2::new(WIDTH, height),
                        ..default()
                    }),
                    // Each camera owns half the image: neither may clear the other's.
                    output_mode: CameraOutputMode::Write {
                        blend_state: None,
                        clear_color: ClearColorConfig::None,
                    },
                    ..default()
                },
                RenderTarget::Image(image.clone().into()),
                // The glass hands the scene's light on to the main view
                // untouched; that view tonemaps it once.
                Hdr,
                Tonemapping::None,
                Msaa::Off,
                Projection::Perspective(PerspectiveProjection {
                    fov: field,
                    near: 0.05,
                    far: FAR,
                    ..default()
                }),
                Skybox::default(),
                AmbientLight::default(),
                Exposure::default(),
                mount(pane),
            ));
        }
    }
}

type MainCamera<'w, 's> = Query<
    'w,
    's,
    (
        &'static Skybox,
        &'static AmbientLight,
        &'static Exposure,
        Option<&'static EnvironmentMapLight>,
        Option<Ref<'static, DistanceFog>>,
    ),
    (With<bevy_egui::PrimaryEguiContext>, Without<MirrorCamera>),
>;

type MirrorCameras<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static MirrorCamera,
        &'static mut Camera,
        &'static mut Skybox,
        &'static mut AmbientLight,
        &'static mut Exposure,
        Option<&'static EnvironmentMapLight>,
        Has<DistanceFog>,
    ),
>;

type Glass<'w, 's> = Query<
    'w,
    's,
    (
        &'static MirrorGlass,
        &'static mut Mesh3d,
        &'static mut MeshMaterial3d<StandardMaterial>,
    ),
>;

/// Runs the mirrors in the cockpit view only, one camera per frame, lit and
/// exposed like the main view.
#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    session: Res<Session>,
    frontend: Res<crate::menu::Frontend>,
    mirrors: Option<Res<Mirrors>>,
    main: MainCamera,
    mut cameras: MirrorCameras,
    mut glass: Glass,
    mut frame: Local<usize>,
) {
    let Some(mirrors) = mirrors else {
        return;
    };
    let live = frontend.video.mirrors && session.ready && session.camera.mode == Mode::Cockpit;
    for (pane, mut mesh, mut material) in &mut glass {
        let (wanted_mesh, wanted_material) = if live {
            (&pane.live, &mirrors.material)
        } else {
            (&pane.plain.0, &pane.plain.1)
        };
        if mesh.0 != *wanted_mesh {
            mesh.0 = wanted_mesh.clone();
        }
        if material.0 != *wanted_material {
            material.0 = wanted_material.clone();
        }
    }
    *frame = frame.wrapping_add(1);
    let Ok((sky, ambient, exposure, environment, fog)) = main.single() else {
        return;
    };
    for (
        entity,
        mirror,
        mut camera,
        mut mirror_sky,
        mut mirror_ambient,
        mut mirror_exposure,
        lit,
        fogged,
    ) in &mut cameras
    {
        let active = live && turn(*frame, mirror.slot);
        if camera.is_active != active {
            camera.is_active = active;
        }
        if !live {
            continue;
        }
        if mirror_sky.image != sky.image
            || mirror_sky.brightness != sky.brightness
            || mirror_sky.rotation != sky.rotation
        {
            *mirror_sky = sky.clone();
        }
        if mirror_ambient.brightness != ambient.brightness {
            *mirror_ambient = ambient.clone();
        }
        if mirror_exposure.ev100 != exposure.ev100 {
            mirror_exposure.ev100 = exposure.ev100;
        }
        // The main view filters the sky into its lighting once; share the result.
        if let Some(environment) = environment {
            if lit.is_none_or(|lit| lit.diffuse_map != environment.diffuse_map) {
                commands.entity(entity).insert(environment.clone());
            }
        }
        match &fog {
            Some(fog) if !fogged || fog.is_changed() => {
                commands.entity(entity).insert((**fog).clone());
            }
            None if fogged => {
                commands.entity(entity).remove::<DistanceFog>();
            }
            _ => {}
        }
    }
}

/// Whether the mirror in `slot` draws on this frame: they take turns.
fn turn(frame: usize, slot: usize) -> bool {
    frame % 2 == slot % 2
}

/// Bevy builds sun shadow cascades for every active camera, and would render
/// a full set of shadow maps for the mirror as well. Emptying the mirror's
/// cascades leaves it sunlit but unshadowed: with none to pick, the shader's
/// cascade search falls through to "lit".
fn skip_shadow_maps(cameras: Query<Entity, With<MirrorCamera>>, mut lights: Query<&mut Cascades>) {
    if cameras.is_empty() {
        return;
    }
    for mut cascades in &mut lights {
        for camera in &cameras {
            if let Some(cascades) = cascades.cascades.get_mut(&camera) {
                cascades.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat pane of glass facing `normal`, spanning `low`..`high`.
    fn quad(low: [f32; 3], high: [f32; 3], normal: [f32; 3]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let positions = vec![
            low,
            [high[0], low[1], high[2]],
            high,
            [low[0], high[1], low[2]],
        ];
        (positions, vec![normal; 4])
    }

    #[test]
    fn glass_is_found_per_side_and_forward_facing_parts_are_ignored() {
        let (mut positions, mut normals) =
            quad([0.49, 0.74, 0.63], [0.67, 0.79, 0.70], [-0.36, 0., -0.93]);
        let (right, right_normals) =
            quad([-0.67, 0.74, 0.63], [-0.49, 0.79, 0.70], [0.36, 0., -0.93]);
        positions.extend(right);
        normals.extend(right_normals);
        let found = panes(&positions, &normals);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].slot, found[1].slot), (0, 1));
        assert!(found[0].low.x > 0. && found[1].high.x < 0.);
        assert!((found[0].aspect() - 3.6).abs() < 0.01);

        // A repeater lamp on the front of the housing is not glass.
        let (lamp, lamp_normals) = quad([0.67, 0.80, 0.67], [0.70, 0.83, 0.71], [0.8, 0., 0.59]);
        assert!(panes(&lamp, &lamp_normals).is_empty());
    }

    #[test]
    fn each_pane_maps_to_its_half_of_the_image_mirrored() {
        let (positions, normals) = quad([0.49, 0.74, 0.63], [0.67, 0.79, 0.70], [-0.36, 0., -0.93]);
        let left = panes(&positions, &normals)[0];
        let right = Pane {
            slot: 1,
            low: Vec3::new(-0.67, 0.74, 0.63),
            high: Vec3::new(-0.49, 0.79, 0.70),
        };
        // The left mirror's outer edge (+x) shows the right of its camera's
        // picture, and the top of the glass the top of the picture.
        let [outer, top] = glass_uv(Vec3::new(0.67, 0.79, 0.7), &left);
        let [inner, bottom] = glass_uv(Vec3::new(0.49, 0.74, 0.63), &left);
        assert!(inner < outer && outer < 0.5 && inner > 0.);
        assert_eq!((top, bottom), (0., 1.));
        // The right mirror stays inside the other half.
        let [inner, _] = glass_uv(Vec3::new(-0.49, 0.76, 0.65), &right);
        let [outer, _] = glass_uv(Vec3::new(-0.67, 0.76, 0.65), &right);
        assert!(outer > 0.5 && outer < inner && inner < 1.);
        // A vertex past the pane's bounds clamps onto it.
        assert_eq!(
            glass_uv(Vec3::new(0.9, 2., 0.), &left),
            glass_uv(Vec3::new(0.67, 0.79, 0.), &left)
        );
        let uvs = glass_uvs(&[[0.5, 0.75, 0.65], [-0.5, 0.75, 0.65]], &[left, right]);
        assert!(uvs[0][0] < 0.5 && uvs[1][0] > 0.5);
    }

    #[test]
    fn cameras_look_back_and_outward_from_behind_the_glass() {
        for (slot, outward) in [(0, 1.), (1, -1.)] {
            let pane = Pane {
                slot,
                low: Vec3::new(outward * 0.49, 0.74, 0.63).min(Vec3::new(
                    outward * 0.67,
                    0.74,
                    0.63,
                )),
                high: Vec3::new(outward * 0.49, 0.79, 0.70).max(Vec3::new(
                    outward * 0.67,
                    0.79,
                    0.70,
                )),
            };
            let transform = mount(&pane);
            let forward = transform.forward();
            assert!(forward.z < -0.9, "{forward:?}");
            assert!(forward.x * outward > 0.05, "{forward:?}");
            assert!(transform.translation.z < pane.low.z);
            // Looking back, the camera's right is the car's left (+x).
            assert!(transform.right().x > 0.9);
        }
    }

    #[test]
    fn framing_follows_the_glass_and_mirrors_alternate_frames() {
        let (field, height) = framing(3.6);
        assert_eq!(height, 72);
        assert!((field - 0.141).abs() < 0.003, "{field}");
        // Square glass is capped rather than given a fisheye view.
        let (field, height) = framing(1.);
        assert_eq!((field, height), (MAX_VERTICAL_FIELD, WIDTH));
        for frame in 0..6 {
            assert_ne!(turn(frame, 0), turn(frame, 1));
            assert_ne!(turn(frame, 0), turn(frame + 1, 0));
        }
    }
}
