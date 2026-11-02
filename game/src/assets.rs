//! Bevy glTF scenes and simulation-to-render transforms.

use crate::{
    controls::press_amount, materials::CarSurface, options::Options, pose::Pose, sky, Session,
};
use bevy::{
    asset::LoadState, light::atmosphere::ScatteringMedium, mesh::VertexAttributeValues, prelude::*,
    world_serialization::WorldInstanceReady,
};
use racecraft_sim::{Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Race, Track};
use std::sync::Arc;

#[derive(Component)]
pub struct CarVisual(pub usize);

/// Excluded from the main camera; the sun also renders this shadow proxy layer.
pub const PLAYER_SHADOW_LAYER: usize = 1;

#[derive(Component)]
pub struct PlayerShadowProxy;

/// Default distance at which opponent meshes crossfade (over ten metres).
pub const OPPONENT_LOD_DISTANCE: f32 = 40.;
const OPPONENT_LOD_MARGIN: f32 = 5.;

/// Current opponent near/far swap distance, from the video settings.
#[derive(Resource, PartialEq)]
pub struct OpponentLodDistance(pub f32);

impl Default for OpponentLodDistance {
    fn default() -> Self {
        Self(OPPONENT_LOD_DISTANCE)
    }
}

/// Marks a mesh that belongs to an opponent's near (false) or far (true) model.
#[derive(Component)]
pub struct OpponentLod(bool);

/// Re-applies the swap distance to every opponent mesh when the setting changes.
pub fn update_opponent_lods(
    distance: Res<OpponentLodDistance>,
    mut meshes: Query<(&OpponentLod, &mut bevy::camera::visibility::VisibilityRange)>,
) {
    if !distance.is_changed() {
        return;
    }
    for (lod, mut range) in &mut meshes {
        *range = opponent_range(lod.0, distance.0);
    }
}

#[derive(Component)]
struct CarScene {
    entrant: usize,
    lod: Option<bool>, // false = near, true = far; None = no distance limit.
    shadow_only: bool,
}

fn opponent_range(far: bool, distance: f32) -> bevy::camera::visibility::VisibilityRange {
    let transition = (distance - OPPONENT_LOD_MARGIN)..(distance + OPPONENT_LOD_MARGIN);
    bevy::camera::visibility::VisibilityRange {
        start_margin: if far { transition.clone() } else { 0.0..0.0 },
        end_margin: if far { f32::MAX..f32::MAX } else { transition },
        use_aabb: false,
    }
}

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

/// Separate cap, parented to the steering wheel so travel follows its rotation.
#[derive(Component)]
pub struct DrsButton {
    pub entrant: usize,
    pub rest: Transform,
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
            pulls.push(ages.map(press_amount));
        }
        pulls
    }
}

/// A driver or helmet node whose baked steering poses follow the wheel.
#[derive(Component)]
pub struct DriverMorph(usize);

#[derive(Component)]
pub struct DriverHelmet(pub(crate) usize);

#[derive(Component)]
pub struct VisualReady;

#[derive(Component)]
pub struct RaceVisualRoot;

#[derive(Resource)]
pub struct VisualAssets {
    handles: Vec<Handle<bevy::world_serialization::WorldAsset>>,
    started: Option<f64>,
    scene_count: usize,
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
    let opponent = if options.car == "gen22" {
        server.load(GltfAssetLabel::Scene(0).from_asset("cars/gen22/car_lod1.glb"))
    } else {
        car.clone()
    };
    let distant = (options.car == "gen22"
        && session.race.entrants.len() > 1
        && options.assets.join("cars/gen22/car_lod2.glb").is_file())
    .then(|| server.load(GltfAssetLabel::Scene(0).from_asset("cars/gen22/car_lod2.glb")));
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
                Visibility::default(),
                body_transform(&entrant.car),
            ))
            .with_children(|car| {
                let has_lod = index != session.player && distant.is_some();
                car.spawn((
                    CarScene {
                        entrant: index,
                        lod: has_lod.then_some(false),
                        shadow_only: false,
                    },
                    WorldAssetRoot(scene),
                    Transform::default(),
                ))
                .observe(visual_ready);
                if let Some(far) = distant.as_ref().filter(|_| has_lod) {
                    car.spawn((
                        CarScene {
                            entrant: index,
                            lod: Some(true),
                            shadow_only: false,
                        },
                        WorldAssetRoot(far.clone()),
                        Transform::default(),
                    ))
                    .observe(visual_ready);
                }
                let shadow = if index == session.player && options.car == "gen22" {
                    Some(opponent.clone())
                } else {
                    distant.clone()
                };
                if let Some(shadow) = shadow {
                    car.spawn((
                        CarScene {
                            entrant: index,
                            lod: None,
                            shadow_only: true,
                        },
                        WorldAssetRoot(shadow),
                        Transform::default(),
                    ))
                    .observe(visual_ready);
                }
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
    let scene_count = session.race.entrants.len()
        + 1
        + usize::from(options.car == "gen22")
        + if distant.is_some() {
            2 * (session.race.entrants.len() - 1)
        } else {
            0
        };
    let mut handles = vec![track, car, opponent];
    handles.extend(distant);
    commands.insert_resource(VisualAssets {
        handles,
        started: None,
        scene_count,
    });
}

#[allow(clippy::too_many_arguments)]
fn visual_ready(
    event: On<WorldInstanceReady>,
    mut commands: Commands,
    cars: Query<&CarScene>,
    children: Query<&Children>,
    nodes: Query<(&Name, &Transform)>,
    material_names: Query<&bevy::gltf::GltfMaterialName>,
    mesh_handles: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
    session: Res<Session>,
    lod_distance: Res<OpponentLodDistance>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Ok(car) = cars.get(event.entity) {
        let pivot = children.iter_descendants(event.entity).find_map(|entity| {
            let name = material_names.get(entity).ok()?;
            if !matches!(name.0.as_str(), "RBRWHEEL10" | "redbull_steeringwheel.004") {
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
            if let Some(far) = car.lod.filter(|_| mesh_handles.contains(entity)) {
                commands
                    .entity(entity)
                    .insert((opponent_range(far, lod_distance.0), OpponentLod(far)));
            }
            if mesh_handles.contains(entity) {
                if car.shadow_only {
                    commands
                        .entity(entity)
                        .insert(bevy::camera::visibility::RenderLayers::layer(
                            PLAYER_SHADOW_LAYER,
                        ));
                    if car.entrant == session.player {
                        commands.entity(entity).insert(PlayerShadowProxy);
                    }
                } else if (car.entrant == session.player
                    && session.identities[car.entrant].is_some())
                    || car.lod.is_some()
                {
                    // Dedicated LOD silhouettes cast the sun; visible geometry receives it.
                    commands.entity(entity).insert(bevy::light::NotShadowCaster);
                }
            }
            if !car.shadow_only && material_names.contains(entity) {
                commands.entity(entity).insert(CarSurface(
                    session
                        .identities
                        .get(car.entrant)
                        .and_then(|identity| identity.as_ref())
                        .map(|identity| identity.team.clone()),
                    (car.entrant != session.player).then_some(if car.lod == Some(true) {
                        "lod2"
                    } else {
                        "lod1"
                    }),
                    session.race.entrants[car.entrant].car.setup.compound,
                ));
            }
            let Ok((name, rest)) = nodes.get(entity) else {
                continue;
            };
            if let Some((pivot, _, _)) = pivot.filter(|_| car.entrant == session.player) {
                if material_names.get(entity).is_ok_and(|name| {
                    matches!(name.0.as_str(), "RBRWHEEL10" | "redbull_steeringwheel.004")
                }) {
                    commands.entity(entity).insert((
                        SteeringWheelVisual {
                            entrant: car.entrant,
                            rest: *rest,
                            pivot,
                        },
                        crate::wheel_dash::WheelPart::Mount(car.entrant),
                    ));
                }
            }
            if session
                .identities
                .get(car.entrant)
                .is_some_and(Option::is_some)
            {
                if car.entrant == session.player && material_names.contains(entity) {
                    match material_names.get(entity).map(|name| name.0.as_str()) {
                        Ok("wheel_screen") => {
                            commands
                                .entity(entity)
                                .insert(crate::wheel_dash::WheelPart::Screen(car.entrant));
                        }
                        Ok("wheel_leds") => {
                            commands
                                .entity(entity)
                                .insert(crate::wheel_dash::WheelPart::Leds(car.entrant));
                        }
                        _ => {}
                    }
                }
                let kind = match name.as_str() {
                    "steering_wheel" if car.entrant == session.player => Some(RigKind::Steering),
                    "upright_fl" | "upright_fr" => Some(RigKind::Upright),
                    "drs_flap" => Some(RigKind::Drs),
                    _ => None,
                };
                if let Some(up) = match name.as_str() {
                    "paddle_up" if car.entrant == session.player => Some(true),
                    "paddle_down" if car.entrant == session.player => Some(false),
                    _ => None,
                } {
                    commands.entity(entity).insert(Paddle {
                        entrant: car.entrant,
                        rest: *rest,
                        up,
                    });
                }
                if name.as_str() == "drs_button" && car.entrant == session.player {
                    commands.entity(entity).insert(DrsButton {
                        entrant: car.entrant,
                        rest: *rest,
                    });
                }
                if name.as_str() == "helmet" && !car.shadow_only {
                    commands.entity(entity).insert(DriverHelmet(car.entrant));
                }
                if !car.shadow_only
                    && car.entrant == session.player
                    && matches!(name.as_str(), "driver" | "helmet")
                {
                    commands.entity(entity).insert(DriverMorph(car.entrant));
                }
                if let Some(kind) = kind {
                    commands.entity(entity).insert(RigPart {
                        entrant: car.entrant,
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
                    entrant: car.entrant,
                    wheel,
                    rest: *rest,
                });
            }
        }
        if !body_found || !found.into_iter().all(|found| found) {
            error!("car {} is missing body or wheel nodes", car.entrant);
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
    for id in visuals.handles.iter().map(|handle| handle.id().untyped()) {
        if let Some(LoadState::Failed(error)) = server.get_load_state(id) {
            error!("visual asset failed: {error}");
            menu.error = Some(format!("Could not load a visual asset: {error}"));
            session.screen = crate::menu::Screen::Main;
            session.paused = true;
            if menu.draft.frames.is_some() || menu.draft.benchmark.is_some() {
                exit.write(AppExit::error());
            }
            return;
        }
    }
    if ready.iter().count() == visuals.scene_count {
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

/// Keep unsprung wheel geometry on its road contact while the chassis pitches,
/// rolls and heaves. The tyre is a cylinder: its axle tilt also changes the
/// distance from hub to its lowest point.
fn grounded_hub(
    chassis: &Transform,
    rest: Vec3,
    rotation: Quat,
    road: f32,
    radius: f32,
    width: f32,
) -> Vec3 {
    let axle_y = (chassis.rotation * rotation * Vec3::X).y.abs().min(1.);
    let vertical_radius = radius * (1. - axle_y * axle_y).sqrt() + width * 0.5 * axle_y;
    let mut world = chassis.transform_point(rest);
    world.y = road + vertical_radius;
    chassis.compute_affine().inverse().transform_point3(world)
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
        let chassis = pose_transform(pose);
        let wheel = &session.race.entrants[visual.entrant].car.spec.wheels[visual.wheel];
        transform.translation = grounded_hub(
            &chassis,
            visual.rest.translation,
            transform.rotation,
            pose.ground[visual.wheel] as f32,
            wheel.radius as f32,
            wheel.width as f32,
        );
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
            // Every car's flap follows its own simulated DRS actuator.
            RigKind::Drs => Quat::from_rotation_x(
                session.rig.drs.open_angle
                    * session.race.entrants[part.entrant].car.drs.flap as f32,
            ),
        };
        if matches!(part.kind, RigKind::Upright) {
            let car = &session.race.entrants[part.entrant].car;
            let index = if part.rest.translation.x > 0. { 0 } else { 1 };
            let wheel = &car.spec.wheels[index];
            transform.translation = grounded_hub(
                &pose_transform(pose),
                part.rest.translation,
                part.rest.rotation * rotation,
                pose.ground[index] as f32,
                wheel.radius as f32,
                wheel.width as f32,
            );
        }
        transform.rotation = part.rest.rotation * rotation;
    }
    for (driver, mut weights) in &mut drivers {
        // Same wheel angle as `RigKind::Steering`, so the hands stay on the grips.
        let wheel = -poses[driver.0].steer as f32 * 6.;
        session.rig.driver_weights(wheel, weights.weights_mut());
    }
}

/// Animates shift paddles/fingers and the player's DRS cap/thumb press.
/// Runs after [`sync_cars`], which sets the steering part of the driver pose.
pub fn animate_controls(
    session: Res<Session>,
    time: Res<Time>,
    mut shifts: Local<ShiftTracker>,
    mut paddles: Query<(&Paddle, &mut Transform), Without<DrsButton>>,
    mut buttons: Query<(&DrsButton, &mut Transform), Without<Paddle>>,
    mut drivers: Query<(&DriverMorph, &mut bevy::mesh::morph::MorphWeights)>,
) {
    // Every replacement session loads its visuals before it can race. Local
    // system state survives that replacement, so discard the previous gears.
    if !session.ready {
        shifts.0.clear();
    }
    let pulls = shifts.update(
        std::iter::once(session.race.entrants[session.player].car.powertrain.gear),
        time.delta_secs(),
    );
    for (paddle, mut transform) in &mut paddles {
        if paddle.entrant != session.player {
            continue;
        }
        let (angle, pull) = if paddle.up {
            (session.rig.paddles.up_angle, pulls[0][0])
        } else {
            (session.rig.paddles.down_angle, pulls[0][1])
        };
        *transform = paddle.rest;
        transform.rotation = paddle.rest.rotation * Quat::from_rotation_y(angle * pull);
    }
    let press = if session.ready && !session.paused {
        session.controls.drs_press()
    } else {
        0.
    };
    for (button, mut transform) in &mut buttons {
        *transform = button.rest;
        if button.entrant == session.player {
            transform.translation += Vec3::from_array(session.rig.drs_button_travel) * press;
        }
    }
    for (driver, mut weights) in &mut drivers {
        session.rig.shift_weights(pulls[0], weights.weights_mut());
        session.rig.drs_weight(
            if driver.0 == session.player {
                press
            } else {
                0.
            },
            weights.weights_mut(),
        );
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

/// Keep the local helmet out of the eye camera; restore it in external views.
pub fn helmet_visibility(
    session: Res<Session>,
    mut helmets: Query<(&DriverHelmet, &mut Visibility)>,
) {
    for (helmet, mut visibility) in &mut helmets {
        *visibility =
            if helmet.0 == session.player && session.camera.mode == crate::camera::Mode::Cockpit {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
    }
}

#[cfg(test)]
mod rig_tests {
    use super::*;
    #[test]
    fn opponent_lods_share_a_crossfade_without_a_visibility_gap() {
        let near = opponent_range(false, OPPONENT_LOD_DISTANCE);
        let far = opponent_range(true, OPPONENT_LOD_DISTANCE);
        assert_eq!(near.end_margin, far.start_margin);
        assert_eq!(near.end_margin, 35.0..45.0);
        for distance in [0., 34.9, 35., 40., 44.9, 45., 1000.] {
            assert!(near.is_visible_at_all(distance) || far.is_visible_at_all(distance));
        }
        assert!(near.is_visible_at_all(40.) && far.is_visible_at_all(40.));
        assert!(!near.is_visible_at_all(45.));
        assert!(!far.is_visible_at_all(34.9));
    }

    #[test]
    fn drs_press_survives_physics_consumption_and_releases_cap_and_thumb() {
        let mut state = crate::tests::session();
        state.ready = true;
        state.rig.driver_drs_pose = Some(0);
        state.rig.drs_button_travel = [0., 0., 0.003];
        let mut app = App::new();
        app.insert_resource(state)
            .init_resource::<Time>()
            .add_systems(Update, animate_controls);
        let rest = Transform::from_xyz(0.1, 0.2, 0.3);
        let button = app
            .world_mut()
            .spawn((DrsButton { entrant: 0, rest }, rest))
            .id();
        let driver = app
            .world_mut()
            .spawn((
                DriverMorph(0),
                bevy::mesh::morph::MorphWeights::new(vec![0.], None).unwrap(),
            ))
            .id();
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyF);
        {
            let mut session = app.world_mut().resource_mut::<Session>();
            session.controls.update(&keys, None, 0.);
            assert!(session.controls.car_input().drs);
            assert!(!session.controls.car_input().drs);
            keys.clear();
            session.controls.update(&keys, None, 0.06);
        }
        app.update();
        assert!(
            !app.world().resource::<Session>().race.entrants[0]
                .car
                .drs
                .available
        );
        assert!(app
            .world()
            .get::<Transform>(button)
            .unwrap()
            .translation
            .abs_diff_eq(rest.translation + Vec3::Z * 0.003, 1e-6));
        assert_eq!(
            app.world()
                .get::<bevy::mesh::morph::MorphWeights>(driver)
                .unwrap()
                .weights(),
            &[1.]
        );
        app.world_mut()
            .resource_mut::<Session>()
            .controls
            .update(&keys, None, 0.3);
        app.update();
        assert_eq!(*app.world().get::<Transform>(button).unwrap(), rest);
        assert_eq!(
            app.world()
                .get::<bevy::mesh::morph::MorphWeights>(driver)
                .unwrap()
                .weights(),
            &[0.]
        );
    }

    #[test]
    fn new_session_does_not_inherit_press_amounts() {
        let mut state = crate::tests::session();
        state.ready = true;
        state.rig.paddles.up_angle = -0.1;
        state.rig.paddles.down_angle = 0.1;
        let mut app = App::new();
        app.insert_resource(state)
            .init_resource::<Time>()
            .add_systems(Update, animate_controls);
        let paddle = app
            .world_mut()
            .spawn((
                Paddle {
                    entrant: 0,
                    rest: Transform::IDENTITY,
                    up: false,
                },
                Transform::IDENTITY,
            ))
            .id();
        app.update();
        app.world_mut().resource_mut::<Session>().race.entrants[0]
            .car
            .powertrain
            .gear = 4;
        app.update();
        let mut replacement = crate::tests::session();
        replacement.ready = false;
        replacement.rig.paddles.down_angle = 0.1;
        app.insert_resource(replacement);
        app.update();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(0.05));
        app.world_mut().resource_mut::<Session>().ready = true;
        app.update();
        assert_eq!(
            *app.world().get::<Transform>(paddle).unwrap(),
            Transform::IDENTITY
        );
    }

    #[test]
    fn tyre_geometry_meets_the_road_during_chassis_heave_pitch_and_roll() {
        for (height, pitch, roll, steer) in [
            (0.2, 0., 0., 0.),
            (-0.1, 0.12, -0.08, 0.3),
            (0.3, -0.08, 0.15, -0.25),
        ] {
            let chassis = Transform::from_xyz(3., height, 7.).with_rotation(Quat::from_euler(
                EulerRot::YXZ,
                0.7,
                pitch,
                roll,
            ));
            let rest = Vec3::new(0.9, 0.36, 1.8);
            let rotation = Quat::from_rotation_y(steer) * Quat::from_rotation_x(1.7);
            let hub = grounded_hub(&chassis, rest, rotation, 0.05, 0.36, 0.3);
            let mut lowest = f32::INFINITY;
            // Sample an actual cylindrical tread rather than repeating the hub formula.
            for side in [-0.15, 0.15] {
                for sample in 0..720 {
                    let angle = sample as f32 * std::f32::consts::TAU / 720.;
                    let vertex = Vec3::new(side, angle.cos() * 0.36, angle.sin() * 0.36);
                    lowest = lowest.min(chassis.transform_point(hub + rotation * vertex).y);
                }
            }
            assert!((lowest - 0.05).abs() < 1e-4, "tyre bottom {lowest}");
            let actual = chassis.transform_point(hub);
            let original = chassis.transform_point(rest);
            assert!((actual.x - original.x).abs() < 1e-5);
            assert!((actual.z - original.z).abs() < 1e-5);
        }
    }

    #[test]
    fn shifts_flick_the_matching_paddle_and_release_it() {
        assert_eq!(press_amount(f32::INFINITY), 0.);
        assert_eq!(press_amount(0.02), 0.5);
        assert_eq!(press_amount(0.07), 1.);
        assert_eq!(press_amount(0.3), 0.);
        let mut tracker = ShiftTracker::default();
        assert_eq!(tracker.update([3, 3].into_iter(), 0.), vec![[0.; 2]; 2]);
        let pulls = tracker.update([4, 2].into_iter(), 0.016);
        assert_eq!(pulls, vec![[0., 0.], [0., 0.]]);
        let pulls = tracker.update([4, 2].into_iter(), 0.05);
        assert_eq!(pulls, vec![[1., 0.], [0., 1.]]);
    }
}
