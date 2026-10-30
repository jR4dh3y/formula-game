//! Bevy glTF scenes and simulation-to-render transforms.

use crate::{materials::CarSurface, options::Options, pose::Pose, sky, Session};
use bevy::{
    asset::LoadState, light::atmosphere::ScatteringMedium, mesh::VertexAttributeValues, prelude::*,
    world_serialization::WorldInstanceReady,
};
use racecraft_sim::{Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Race, Track};
use std::sync::Arc;

#[derive(Component)]
pub struct CarVisual(pub usize);

#[derive(Component)]
pub struct WheelVisual {
    pub entrant: usize,
    pub wheel: usize,
    pub rest: Transform,
}

#[derive(Component)]
pub struct SteeringWheelVisual {
    pub entrant: usize,
    pub rest: Transform,
    pub pivot: Vec3,
}

#[derive(Component)]
pub struct RigPart {
    pub entrant: usize,
    pub rest: Transform,
    pub kind: RigKind,
}
pub enum RigKind {
    Steering,
    Upright,
    Drs,
}

/// A shift paddle on the steering wheel; origin on its hinge.
#[derive(Component)]
pub struct Paddle {
    entrant: usize,
    rest: Transform,
    up: bool,
}

/// How far a shift paddle and the finger on it are pulled, 0..1, `age`
/// seconds after the shift: a quick flick, a short hold and a release.
pub fn shift_pull(age: f32) -> f32 {
    const PRESS: f32 = 0.04;
    const HOLD: f32 = 0.1;
    const RELEASE: f32 = 0.22;
    if !(0. ..RELEASE).contains(&age) {
        0.
    } else if age < PRESS {
        age / PRESS
    } else if age < HOLD {
        1.
    } else {
        1. - (age - HOLD) / (RELEASE - HOLD)
    }
}

/// Last seen gear and seconds since the last upshift/downshift, per entrant.
#[derive(Default)]
pub struct ShiftTracker(Vec<(i32, [f32; 2])>);

impl ShiftTracker {
    fn update(&mut self, gears: impl Iterator<Item = i32>, dt: f32) -> Vec<[f32; 2]> {
        let mut pulls = Vec::new();
        for (index, gear) in gears.enumerate() {
            if index == self.0.len() {
                self.0.push((gear, [f32::INFINITY; 2]));
            }
            let (last, ages) = &mut self.0[index];
            for age in ages.iter_mut() {
                *age += dt;
            }
            if gear > *last {
                ages[0] = 0.;
            } else if gear < *last {
                ages[1] = 0.;
            }
            *last = gear;
            pulls.push(ages.map(shift_pull));
        }
        pulls
    }
}

/// A driver or helmet node whose baked steering poses follow the wheel.
#[derive(Component)]
pub struct DriverMorph(usize);

#[derive(Component)]
pub struct LiverySurface {
    entrant: usize,
}

type LiveryCache =
    std::collections::HashMap<(AssetId<StandardMaterial>, String), Handle<StandardMaterial>>;

/// Texture paths double as cache keys: paint/suit share per team, helmets per driver.
fn livery_path(name: &str, identity: &crate::teams::Identity) -> Option<String> {
    let base = format!("cars/gen22/liveries/{}", identity.team);
    match name {
        "paint" => Some(format!("{base}/paint.png")),
        "suit" => Some(format!("{base}/suit.png")),
        "helmet_paint" => Some(format!("{base}/{}/helmet.png", identity.driver)),
        _ => None,
    }
}

type PendingLiveries<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static LiverySurface,
        &'static bevy::gltf::GltfMaterialName,
        &'static MeshMaterial3d<StandardMaterial>,
    ),
>;

pub fn apply_liveries(
    mut commands: Commands,
    pending: PendingLiveries,
    session: Res<Session>,
    server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<LiveryCache>,
) {
    for (entity, surface, name, handle) in &pending {
        let path = session
            .identities
            .get(surface.entrant)
            .and_then(Option::as_ref)
            .and_then(|identity| livery_path(&name.0, identity));
        if let Some(path) = path {
            let key = (handle.0.id(), path.clone());
            let replacement = if let Some(cached) = cache.get(&key) {
                cached.clone()
            } else {
                let Some(mut material) = materials.get(&handle.0).cloned() else {
                    continue;
                };
                material.base_color = Color::WHITE;
                material.base_color_texture = Some(server.load(path));
                let replacement = materials.add(material);
                cache.insert(key, replacement.clone());
                replacement
            };
            commands.entity(entity).insert(MeshMaterial3d(replacement));
        }
        commands.entity(entity).remove::<LiverySurface>();
    }
}

#[derive(Component)]
pub struct VisualReady;

#[derive(Component)]
pub struct RaceVisualRoot;

#[derive(Resource)]
pub struct VisualAssets {
    handles: Vec<Handle<bevy::world_serialization::WorldAsset>>,
    started: Option<f64>,
}

pub fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    options: Res<Options>,
    session: Res<Session>,
    mut media: ResMut<Assets<ScatteringMedium>>,
) {
    spawn_visuals(&mut commands, &server, &options, &session);
    let sun = session.race.track.sun_direction;
    sky::setup(
        &mut commands,
        &mut media,
        Vec3::new(sun.x as f32, sun.y as f32, sun.z as f32),
    );
}

pub fn spawn_visuals(
    commands: &mut Commands,
    server: &AssetServer,
    options: &Options,
    session: &Session,
) {
    let track = server
        .load(GltfAssetLabel::Scene(0).from_asset(format!("tracks/{}/visual.glb", options.track)));
    let car =
        server.load(GltfAssetLabel::Scene(0).from_asset(format!("cars/{}/car.glb", options.car)));
    let opponent = if options.car == "gen22" && options.opponents > 0 {
        server.load(GltfAssetLabel::Scene(0).from_asset("cars/gen22/car_lod1.glb"))
    } else {
        car.clone()
    };
    commands
        .spawn((
            RaceVisualRoot,
            WorldAssetRoot(track.clone()),
            Transform::default(),
        ))
        .observe(visual_ready);
    for (index, entrant) in session.race.entrants.iter().enumerate() {
        let scene = if index == session.player {
            car.clone()
        } else {
            opponent.clone()
        };
        commands
            .spawn((
                Name::new(format!("car-{index}")),
                CarVisual(index),
                RaceVisualRoot,
                WorldAssetRoot(scene),
                body_transform(&entrant.car),
            ))
            .observe(visual_ready)
            .with_children(|car| {
                if index == session.player && options.car == "gen22" {
                    // Approximate daylight bouncing into the shaded cockpit. Short range
                    // keeps the fill on the controls and hands rather than the bodywork.
                    car.spawn((
                        Name::new("cockpit-bounce"),
                        PointLight {
                            color: Color::srgb(0.85, 0.91, 1.0),
                            intensity: 4000.,
                            range: 0.85,
                            radius: 0.15,
                            shadow_maps_enabled: false,
                            ..default()
                        },
                        Transform::from_xyz(0., 0.88, 0.38),
                    ));
                }
            });
    }
    commands.insert_resource(VisualAssets {
        handles: vec![track, car, opponent],
        started: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn visual_ready(
    event: On<WorldInstanceReady>,
    mut commands: Commands,
    cars: Query<&CarVisual>,
    children: Query<&Children>,
    nodes: Query<(&Name, &Transform)>,
    material_names: Query<&bevy::gltf::GltfMaterialName>,
    mesh_handles: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
    session: Res<Session>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Ok(car) = cars.get(event.entity) {
        let pivot = children.iter_descendants(event.entity).find_map(|entity| {
            let name = material_names.get(entity).ok()?;
            if !matches!(name.0.as_str(), "RBRWHEEL10") {
                return None;
            }
            let mesh = meshes.get(&mesh_handles.get(entity).ok()?.0)?;
            let VertexAttributeValues::Float32x3(positions) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)?
            else {
                return None;
            };
            let mut low = Vec3::splat(f32::INFINITY);
            let mut high = Vec3::splat(f32::NEG_INFINITY);
            for position in positions {
                let position = Vec3::from_array(*position);
                low = low.min(position);
                high = high.max(position);
            }
            let (_, rest) = nodes.get(entity).ok()?;
            Some((rest.transform_point((low + high) * 0.5), (low, high), *rest))
        });
        let mut found = [false; 4];
        let mut body_found = false;
        for entity in children.iter_descendants(event.entity) {
            if material_names.contains(entity) {
                commands.entity(entity).insert(CarSurface);
            }
            let Ok((name, rest)) = nodes.get(entity) else {
                continue;
            };
            if name.as_str() == "steering_wheel" {
                for surface in children.iter_descendants(entity) {
                    if material_names.contains(surface) {
                        commands
                            .entity(surface)
                            .insert(crate::materials::WheelSurface);
                    }
                }
            }
            if let Some((pivot, _, _)) = pivot {
                if material_names
                    .get(entity)
                    .is_ok_and(|name| name.0 == "RBRWHEEL10")
                {
                    commands.entity(entity).insert((
                        SteeringWheelVisual {
                            entrant: car.0,
                            rest: *rest,
                            pivot,
                        },
                        crate::wheel_dash::WheelPart::Mount(car.0),
                    ));
                }
            }
            if session.identities.get(car.0).is_some_and(Option::is_some) {
                if material_names.contains(entity) {
                    commands
                        .entity(entity)
                        .insert(LiverySurface { entrant: car.0 });
                    match material_names.get(entity).map(|name| name.0.as_str()) {
                        Ok("wheel_screen") => {
                            commands
                                .entity(entity)
                                .insert(crate::wheel_dash::WheelPart::Screen(car.0));
                        }
                        Ok("wheel_leds") => {
                            commands
                                .entity(entity)
                                .insert(crate::wheel_dash::WheelPart::Leds(car.0));
                        }
                        _ => {}
                    }
                }
                let kind = match name.as_str() {
                    "steering_wheel" => Some(RigKind::Steering),
                    "upright_fl" | "upright_fr" => Some(RigKind::Upright),
                    "drs_flap" => Some(RigKind::Drs),
                    _ => None,
                };
                if let Some(up) = match name.as_str() {
                    "paddle_up" => Some(true),
                    "paddle_down" => Some(false),
                    _ => None,
                } {
                    commands.entity(entity).insert(Paddle {
                        entrant: car.0,
                        rest: *rest,
                        up,
                    });
                }
                if matches!(name.as_str(), "driver" | "helmet") {
                    commands.entity(entity).insert(DriverMorph(car.0));
                }
                if let Some(kind) = kind {
                    commands.entity(entity).insert(RigPart {
                        entrant: car.0,
                        rest: *rest,
                        kind,
                    });
                }
            }
            body_found |= name.as_str() == "body";
            if let Some(wheel) = ["wheel_fl", "wheel_fr", "wheel_rl", "wheel_rr"]
                .iter()
                .position(|&wheel| name.as_str() == wheel)
            {
                found[wheel] = true;
                commands.entity(entity).insert(WheelVisual {
                    entrant: car.0,
                    wheel,
                    rest: *rest,
                });
            }
        }
        if !body_found || !found.into_iter().all(|found| found) {
            error!("car {} is missing body or wheel nodes", car.0);
            exit.write(AppExit::error());
            return;
        }
    }
    // Painted rubber/marking overlays receive shadows but must not cast them
    // onto the road a few millimetres below. The converter marks only audited
    // track decals; bridges, barriers, cars and the Spa scene keep their shadows.
    if cars.get(event.entity).is_err() {
        for entity in children.iter_descendants(event.entity) {
            if material_names
                .get(entity)
                .is_ok_and(|name| name.0.trim_start_matches("ground:").starts_with("decal:"))
            {
                commands.entity(entity).insert(bevy::light::NotShadowCaster);
            }
        }
    }
    commands.entity(event.entity).insert(VisualReady);
}

pub fn check_loading(
    server: Res<AssetServer>,
    mut visuals: ResMut<VisualAssets>,
    ready: Query<(), With<VisualReady>>,
    time: Res<Time<Real>>,
    mut session: ResMut<Session>,
    mut exit: MessageWriter<AppExit>,
    mut menu: ResMut<crate::menu::Frontend>,
) {
    if session.ready {
        return;
    }
    let started = *visuals.started.get_or_insert(time.elapsed_secs_f64());
    for handle in &visuals.handles {
        if let Some(LoadState::Failed(error)) = server.get_load_state(handle.id()) {
            error!("visual asset failed: {error}");
            menu.error = Some(format!("Could not load the scene: {error}"));
            session.screen = crate::menu::Screen::Main;
            session.paused = true;
            if menu.draft.frames.is_some() || menu.draft.benchmark.is_some() {
                exit.write(AppExit::error());
            }
            return;
        }
    }
    if ready.iter().count() == session.race.entrants.len() + 1 {
        session.ready = true;
        if session.screen == crate::menu::Screen::Loading {
            menu.race_started = true;
            session.screen = crate::menu::Screen::Racing;
            session.paused = false;
        }
        info!("Track and {} car scenes ready", session.race.entrants.len());
    } else if time.elapsed_secs_f64() - started > 120.0 {
        error!("Timed out loading track/car scenes and their dependencies");
        menu.error = Some("Scene loading timed out. Choose another circuit and try again.".into());
        session.screen = crate::menu::Screen::Main;
        session.paused = true;
        if menu.draft.frames.is_some() || menu.draft.benchmark.is_some() {
            exit.write(AppExit::error());
        }
    }
}

fn body_transform(car: &racecraft_sim::Car) -> Transform {
    pose_transform(&Pose::of(car))
}

fn pose_transform(pose: &Pose) -> Transform {
    let origin = pose.origin;
    Transform::from_translation(Vec3::new(origin.x as f32, origin.y as f32, origin.z as f32))
        .with_rotation(Quat::from_euler(
            EulerRot::YXZ,
            pose.yaw as f32,
            -pose.pitch as f32,
            pose.roll as f32,
        ))
}

#[allow(clippy::type_complexity)]
pub fn sync_cars(
    session: Res<Session>,
    fixed: Res<Time<Fixed>>,
    mut bodies: Query<
        (&CarVisual, &mut Transform),
        (
            Without<WheelVisual>,
            Without<SteeringWheelVisual>,
            Without<RigPart>,
        ),
    >,
    mut wheels: Query<
        (&WheelVisual, &mut Transform),
        (
            Without<CarVisual>,
            Without<SteeringWheelVisual>,
            Without<RigPart>,
        ),
    >,
    mut steering_wheels: Query<
        (&SteeringWheelVisual, &mut Transform),
        (Without<CarVisual>, Without<WheelVisual>, Without<RigPart>),
    >,
    mut rig_parts: Query<
        (&RigPart, &mut Transform),
        (
            Without<CarVisual>,
            Without<WheelVisual>,
            Without<SteeringWheelVisual>,
        ),
    >,
    mut drivers: Query<(&DriverMorph, &mut bevy::mesh::morph::MorphWeights)>,
) {
    let blend = fixed.overstep_fraction_f64();
    let poses: Vec<Pose> = (0..session.race.entrants.len())
        .map(|index| session.poses.at(&session.race, index, blend))
        .collect();
    for (visual, mut transform) in &mut bodies {
        *transform = pose_transform(&poses[visual.0]);
    }
    for (visual, mut transform) in &mut wheels {
        let pose = &poses[visual.entrant];
        let steer = if visual.wheel < 2 {
            pose.steer as f32
        } else {
            0.0
        };
        *transform = visual.rest;
        transform.rotation = Quat::from_rotation_y(steer)
            * visual.rest.rotation
            * Quat::from_rotation_x(pose.wheels[visual.wheel] as f32);
    }
    for (visual, mut transform) in &mut steering_wheels {
        *transform = visual.rest;
        // A 6:1 steering ratio gives about 124 degrees at the sim's 0.36 rad lock.
        transform.rotate_around(
            visual.pivot,
            Quat::from_rotation_z(-poses[visual.entrant].steer as f32 * 6.0),
        );
    }
    for (part, mut transform) in &mut rig_parts {
        *transform = part.rest;
        let pose = &poses[part.entrant];
        let rotation = match part.kind {
            // Driver looks along local +Z: left (+X) moves down for an anticlockwise turn.
            RigKind::Steering => Quat::from_rotation_z(-pose.steer as f32 * 6.),
            RigKind::Upright => Quat::from_rotation_y(pose.steer as f32),
            RigKind::Drs => Quat::from_rotation_x(if session.drs_open {
                session.rig.drs.open_angle
            } else {
                0.
            }),
        };
        transform.rotation = part.rest.rotation * rotation;
    }
    for (driver, mut weights) in &mut drivers {
        // Same wheel angle as `RigKind::Steering`, so the hands stay on the grips.
        let wheel = -poses[driver.0].steer as f32 * 6.;
        session.rig.driver_weights(wheel, weights.weights_mut());
    }
}

/// Flicks the shift paddle and the finger on it whenever a car changes gear.
/// Runs after [`sync_cars`], which sets the steering part of the driver pose.
pub fn animate_shifts(
    session: Res<Session>,
    time: Res<Time>,
    mut shifts: Local<ShiftTracker>,
    mut paddles: Query<(&Paddle, &mut Transform)>,
    mut drivers: Query<(&DriverMorph, &mut bevy::mesh::morph::MorphWeights)>,
) {
    let pulls = shifts.update(
        session
            .race
            .entrants
            .iter()
            .map(|entrant| entrant.car.powertrain.gear),
        time.delta_secs(),
    );
    for (paddle, mut transform) in &mut paddles {
        let (angle, pull) = if paddle.up {
            (session.rig.paddles.up_angle, pulls[paddle.entrant][0])
        } else {
            (session.rig.paddles.down_angle, pulls[paddle.entrant][1])
        };
        *transform = paddle.rest;
        transform.rotation = paddle.rest.rotation * Quat::from_rotation_y(angle * pull);
    }
    for (driver, mut weights) in &mut drivers {
        session
            .rig
            .shift_weights(pulls[driver.0], weights.weights_mut());
    }
}

const AI_NAMES: [&str; 15] = [
    "M. Kessler",
    "L. Moreau",
    "D. Okafor",
    "S. Lindqvist",
    "R. Takeda",
    "A. Ferreira",
    "J. Novak",
    "E. Brandt",
    "K. Ishida",
    "T. Valente",
    "N. Duarte",
    "P. Halloran",
    "G. Rossetti",
    "H. Varga",
    "C. Delacroix",
];

pub fn load_race(options: &Options) -> Result<Race, String> {
    options.validate_track()?;
    let root = &options.assets;
    let track_dir = root.join("tracks").join(&options.track);
    for path in [
        track_dir.join("visual.glb"),
        root.join(format!("cars/{}/car.glb", options.car)),
    ] {
        if !path.is_file() {
            return Err(format!("missing asset {}", path.display()));
        }
    }
    if options.car == "gen22"
        && options.opponents > 0
        && !root.join("cars/gen22/car_lod1.glb").is_file()
    {
        return Err("missing asset cars/gen22/car_lod1.glb".into());
    }
    let track = Arc::new(Track::load(&track_dir)?);
    let geometry = CarGeometry::load(root.join(format!("cars/{}/car.json", options.car)))?;
    let field_spec = Arc::new(if options.car == "rb6" {
        CarSpec::rb6_with(&geometry)
    } else {
        CarSpec::gen22_with(&geometry)
    });
    let teams = if options.car == "gen22" {
        crate::teams::Teams::load(root)?
    } else {
        default()
    };
    let identities = teams.field(options)?;
    // Grid: the player starts in the requested slot among the AI field.
    let player_slot = options.grid_slot - 1;
    let total = options.opponents + 1;
    let mut ai_index = 0;
    let entrants: Vec<EntrantConfig> = (0..total)
        .map(|slot| {
            if slot == player_slot {
                return EntrantConfig {
                    name: identities[slot]
                        .as_ref()
                        .map_or_else(|| "You".into(), |identity| identity.name.clone()),
                    spec: field_spec.clone(),
                    setup: options.setup,
                    assists: options.player_assists(),
                    ai: None,
                    human_slot: 0,
                    livery: [1.0, 1.0, 1.0],
                };
            }
            let index = ai_index;
            ai_index += 1;
            // Faster drivers start further up, like a qualifying order.
            let pace = 0.985 - 0.006 * slot as f64;
            EntrantConfig {
                name: identities[slot].as_ref().map_or_else(
                    || AI_NAMES[index % AI_NAMES.len()].into(),
                    |identity| identity.name.clone(),
                ),
                spec: field_spec.clone(),
                setup: options.setup,
                assists: Assists::default(),
                ai: Some(DriverProfile {
                    pace,
                    aggression: 0.35 + 0.08 * (index % 6) as f64,
                    consistency: 0.88 + 0.02 * (index % 5) as f64,
                }),
                human_slot: 0,
                livery: [1.0, 1.0, 1.0],
            }
        })
        .collect();
    let race = Race::new(track.clone(), entrants, options.laps, 0x5EED);
    Ok(race)
}

#[cfg(test)]
mod livery_tests {
    use super::*;
    #[test]
    fn shifts_flick_the_matching_paddle_and_release_it() {
        assert_eq!(shift_pull(f32::INFINITY), 0.);
        assert_eq!(shift_pull(0.02), 0.5);
        assert_eq!(shift_pull(0.07), 1.);
        assert_eq!(shift_pull(0.3), 0.);
        let mut tracker = ShiftTracker::default();
        assert_eq!(tracker.update([3, 3].into_iter(), 0.), vec![[0.; 2]; 2]);
        let pulls = tracker.update([4, 2].into_iter(), 0.016);
        assert_eq!(pulls, vec![[0., 0.], [0., 0.]]);
        let pulls = tracker.update([4, 2].into_iter(), 0.05);
        assert_eq!(pulls, vec![[1., 0.], [0., 1.]]);
    }
    #[test]
    fn team_textures_share_paths_but_driver_helmets_are_distinct() {
        let first = crate::teams::Identity {
            team: "ferrari".into(),
            driver: "leclerc".into(),
            name: "Charles Leclerc".into(),
            abbr: "LEC".into(),
            team_short: "Ferrari".into(),
            primary: [255, 0, 0],
        };
        let mut second = first.clone();
        second.driver = "sainz".into();
        for material in ["paint", "suit"] {
            assert_eq!(
                livery_path(material, &first),
                livery_path(material, &second)
            );
        }
        assert_ne!(
            livery_path("helmet_paint", &first),
            livery_path("helmet_paint", &second)
        );
        assert_eq!(
            livery_path("paint", &first).unwrap(),
            "cars/gen22/liveries/ferrari/paint.png"
        );
        assert_eq!(livery_path("carbon", &first), None);
    }
}
