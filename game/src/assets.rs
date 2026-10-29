//! Bevy glTF scenes and simulation-to-render transforms.

use crate::{options::Options, sky, Session};
use bevy::{
    asset::LoadState, light::atmosphere::ScatteringMedium, prelude::*,
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
pub struct VisualReady;

#[derive(Resource)]
pub struct VisualAssets(Vec<Handle<bevy::world_serialization::WorldAsset>>);

pub fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    options: Res<Options>,
    session: Res<Session>,
    mut media: ResMut<Assets<ScatteringMedium>>,
) {
    let track = server
        .load(GltfAssetLabel::Scene(0).from_asset(format!("tracks/{}/visual.glb", options.track)));
    let rb22 = server.load(GltfAssetLabel::Scene(0).from_asset("cars/rb22/car.glb"));
    let rb6 = server.load(GltfAssetLabel::Scene(0).from_asset("cars/rb6/car.glb"));
    commands
        .spawn((WorldAssetRoot(track.clone()), Transform::default()))
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
                WorldAssetRoot(scene),
                body_transform(&entrant.car),
            ))
            .observe(visual_ready);
    }
    commands.insert_resource(VisualAssets(vec![track, rb22, rb6]));
    let sun = session.race.track.sun_direction;
    sky::setup(
        &mut commands,
        &mut media,
        Vec3::new(sun.x as f32, sun.y as f32, sun.z as f32),
    );
}

fn visual_ready(
    event: On<WorldInstanceReady>,
    mut commands: Commands,
    cars: Query<&CarVisual>,
    children: Query<&Children>,
    nodes: Query<(&Name, &Transform)>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Ok(car) = cars.get(event.entity) {
        let mut found = [false; 4];
        let mut body_found = false;
        for entity in children.iter_descendants(event.entity) {
            let Ok((name, rest)) = nodes.get(entity) else {
                continue;
            };
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
    commands.entity(event.entity).insert(VisualReady);
}

pub fn check_loading(
    server: Res<AssetServer>,
    visuals: Res<VisualAssets>,
    ready: Query<(), With<VisualReady>>,
    time: Res<Time<Real>>,
    mut session: ResMut<Session>,
    mut exit: MessageWriter<AppExit>,
) {
    if session.ready {
        return;
    }
    for handle in &visuals.0 {
        if let Some(LoadState::Failed(error)) = server.get_load_state(handle.id()) {
            error!("visual asset failed: {error}");
            exit.write(AppExit::error());
            return;
        }
    }
    if ready.iter().count() == session.race.entrants.len() + 1 {
        session.ready = true;
        info!("Track and {} car scenes ready", session.race.entrants.len());
    } else if time.elapsed_secs() > 120.0 {
        error!("Timed out loading track/car scenes and their dependencies");
        exit.write(AppExit::error());
    }
}

fn body_transform(car: &racecraft_sim::Car) -> Transform {
    let origin = car.model_origin();
    Transform::from_translation(Vec3::new(origin.x as f32, origin.y as f32, origin.z as f32))
        .with_rotation(Quat::from_euler(
            EulerRot::YXZ,
            car.yaw as f32,
            -car.pitch as f32,
            car.roll as f32,
        ))
}

pub fn sync_cars(
    session: Res<Session>,
    mut bodies: Query<(&CarVisual, &mut Transform), Without<WheelVisual>>,
    mut wheels: Query<(&WheelVisual, &mut Transform), Without<CarVisual>>,
) {
    for (visual, mut transform) in &mut bodies {
        *transform = body_transform(&session.race.entrants[visual.0].car);
    }
    for (visual, mut transform) in &mut wheels {
        let car = &session.race.entrants[visual.entrant].car;
        let steer = if visual.wheel < 2 {
            car.steer_angle as f32
        } else {
            0.0
        };
        *transform = visual.rest;
        transform.rotation = Quat::from_rotation_y(steer)
            * visual.rest.rotation
            * Quat::from_rotation_x(car.wheels[visual.wheel].rotation as f32);
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
    // Grid: the player starts in the requested slot among the AI field.
    let player_slot = options.grid_slot - 1;
    let total = options.opponents + 1;
    let mut ai_index = 0;
    let entrants: Vec<EntrantConfig> = (0..total)
        .map(|slot| {
            if slot == player_slot {
                return EntrantConfig {
                    name: "You".to_string(),
                    spec: rb22_spec.clone(),
                    setup: options.setup,
                    assists: Assists {
                        steering: options.assist_level.max(0.3),
                        braking: options.assist_level,
                        traction: options.assist_level,
                        stability: options.assist_level,
                        automatic_gears: options.automatic_gears,
                    },
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
                spec: if index % 3 == 1 {
                    rb22_spec.clone()
                } else {
                    rb6_spec.clone()
                },
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
