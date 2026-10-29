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

// These materials belong exclusively to the wheel, not the fixed dashboard or column.
fn steering_material(name: &str) -> bool {
    matches!(
        name,
        "RBRWHEEL10"
            | "redbull_steeringwheel.004"
            | "sw_decals.004"
            | "gp21_LCD.003"
            | "clearled.003"
            | "kers_blue.003"
    )
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
    let rb22 = server.load(GltfAssetLabel::Scene(0).from_asset("cars/rb22/car.glb"));
    let rb6 = server.load(GltfAssetLabel::Scene(0).from_asset("cars/rb6/car.glb"));
    commands
        .spawn((
            RaceVisualRoot,
            WorldAssetRoot(track.clone()),
            Transform::default(),
        ))
        .observe(visual_ready);
    for (index, entrant) in session.race.entrants.iter().enumerate() {
        let scene = if entrant.car.spec.name == "RB22" {
            rb22.clone()
        } else {
            rb6.clone()
        };
        commands
            .spawn((
                Name::new(format!("car-{index}")),
                CarVisual(index),
                RaceVisualRoot,
                WorldAssetRoot(scene),
                body_transform(&entrant.car),
            ))
            .observe(visual_ready);
    }
    commands.insert_resource(VisualAssets {
        handles: vec![track, rb22, rb6],
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
            Some(rest.transform_point((low + high) * 0.5))
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
            if let Some(pivot) = pivot {
                if material_names
                    .get(entity)
                    .is_ok_and(|name| steering_material(&name.0))
                {
                    commands.entity(entity).insert(SteeringWheelVisual {
                        entrant: car.0,
                        rest: *rest,
                        pivot,
                    });
                    match material_names.get(entity).map(|name| name.0.as_str()) {
                        Ok("gp21_LCD.003") => {
                            commands
                                .entity(entity)
                                .insert(crate::wheel_dash::WheelPart::Screen(car.0));
                        }
                        Ok("RBRWHEEL10") => {
                            commands
                                .entity(entity)
                                .insert(crate::wheel_dash::WheelPart::Mount(car.0));
                        }
                        Ok("clearled.003") => {
                            commands
                                .entity(entity)
                                .insert(crate::wheel_dash::WheelPart::Leds(car.0));
                        }
                        _ => {}
                    }
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
        (Without<WheelVisual>, Without<SteeringWheelVisual>),
    >,
    mut wheels: Query<
        (&WheelVisual, &mut Transform),
        (Without<CarVisual>, Without<SteeringWheelVisual>),
    >,
    mut steering_wheels: Query<
        (&SteeringWheelVisual, &mut Transform),
        (Without<CarVisual>, Without<WheelVisual>),
    >,
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
    let root = &options.assets;
    let track_dir = root.join("tracks").join(&options.track);
    for path in [
        track_dir.join("visual.glb"),
        root.join("cars/rb22/car.glb"),
        root.join("cars/rb6/car.glb"),
    ] {
        if !path.is_file() {
            return Err(format!("missing asset {}", path.display()));
        }
    }
    let track = Arc::new(Track::load(&track_dir)?);
    let rb22 = CarGeometry::load(root.join("cars/rb22/car.json"))?;
    let rb6 = CarGeometry::load(root.join("cars/rb6/car.json"))?;
    let rb22_spec = Arc::new(CarSpec::rb22_with(&rb22));
    let rb6_spec = Arc::new(CarSpec::rb6_with(&rb6));
    let field_spec = if options.car == "rb6" {
        rb6_spec
    } else {
        rb22_spec
    };
    // Grid: the player starts in the requested slot among the AI field.
    let player_slot = options.grid_slot - 1;
    let total = options.opponents + 1;
    let mut ai_index = 0;
    let entrants: Vec<EntrantConfig> = (0..total)
        .map(|slot| {
            if slot == player_slot {
                return EntrantConfig {
                    name: "You".to_string(),
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
                name: AI_NAMES[index % AI_NAMES.len()].to_string(),
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
