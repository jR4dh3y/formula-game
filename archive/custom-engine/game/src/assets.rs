//! Loads converted track and car assets and keeps car entities in sync with the simulation.

use crate::options::Options;
use crate::sky;
use crate::Session;
use dhool_pro::asset::ShadowCasterClass;
use dhool_pro::math::{Color, Vec3};
use dhool_pro::{
    import_gltf_scene, Camera, EntityId, EnvironmentProfile, ExposureProfile, GltfImportOptions,
    Light, Mesh, MeshRenderer, RenderScene, SceneBuilder, Transform, World,
};
use racecraft_sim::{Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Race, Track};
use std::path::Path;
use std::sync::Arc;

pub struct Loaded {
    pub scene: RenderScene,
    pub race: Race,
    pub player: usize,
    pub car_meshes: Vec<CarModel>,
    pub track_name: String,
}

/// Render meshes of one car model, in model space (+Z forward, ground at y = 0).
pub struct CarModel {
    pub body: Vec<Mesh>,
    /// fl, fr, rl, rr; vertices relative to the wheel centre.
    pub wheels: [Vec<Mesh>; 4],
    pub geometry: CarGeometry,
}

/// Entities that draw one entrant.
pub struct CarEntities {
    pub body: Vec<EntityId>,
    pub wheels: [Vec<EntityId>; 4],
    pub model: usize,
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

pub fn load(options: &Options) -> Result<Loaded, String> {
    let root = &options.assets;
    if !root.join("tracks").exists() {
        return Err(format!(
            "no assets in {}; run tools/build_assets.sh first",
            root.display()
        ));
    }
    let track_name = options.track.as_deref().ok_or("no track selected")?;
    let track_dir = root.join("tracks").join(track_name);
    println!("loading track {}", track_dir.display());
    let track = Arc::new(Track::load(&track_dir)?);

    let mut builder = SceneBuilder::new();
    load_track_visual(&mut builder, &track_dir.join("visual.glb"))?;

    let rb22 = load_car(&mut builder, &root.join("cars/rb22"))?;
    let rb6 = load_car(&mut builder, &root.join("cars/rb6"))?;
    let rb22_spec = Arc::new(CarSpec::rb22_with(&rb22.geometry));
    let rb6_spec = Arc::new(CarSpec::rb6_with(&rb6.geometry));

    let sun = track.sun_direction.as_vec3();
    builder.add_light(Light::directional(
        "sun",
        Vec3::new(sun.x, sun.y, sun.z),
        Color::rgb(1.0, 0.96, 0.9),
        3.2,
    ));
    let mut scene = builder
        .camera(Camera {
            position: Vec3::new(0.0, 5.0, 0.0),
            target: Vec3::new(0.0, 0.0, 10.0),
            up: Vec3::Y,
            vertical_fov_radians: 62.0_f32.to_radians(),
            near: 0.1,
            far: 2_500.0,
        })
        .environment(
            sky::daylight([sun.x, sun.y, sun.z]),
            EnvironmentProfile {
                exposure: ExposureProfile {
                    min: 0.85,
                    max: 2.5,
                    compensation: 2.0,
                    white_balance_strength: 0.1,
                },
                ..EnvironmentProfile::dry_daylight()
            },
            Color::rgb(0.05, 0.055, 0.06),
        )
        .build()
        .map_err(|error| error.to_string())?;
    scene.environment.precipitation = 0.0;

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
    Ok(Loaded {
        scene,
        race,
        player: player_slot,
        car_meshes: vec![rb22, rb6],
        track_name: track.name.clone(),
    })
}

fn import_options() -> GltfImportOptions {
    GltfImportOptions {
        max_texture_dimension: 1024,
        normal_maps: false,
        metallic_roughness_maps: false,
    }
}

fn load_track_visual(builder: &mut SceneBuilder, path: &Path) -> Result<(), String> {
    let (materials, meshes) = import_gltf_scene(path, import_options())
        .map_err(|error| format!("{}: {error}", path.display()))?
        .into_baked_meshes();
    let cutout: Vec<bool> = materials.iter().map(|m| m.flags.alpha_tested).collect();
    let ids: Vec<_> = materials
        .into_iter()
        .map(|material| builder.add_material(material))
        .collect();
    for mut mesh in meshes {
        let alpha_tested = cutout[mesh.material_index];
        let material = ids[mesh.material_index];
        let ground = mesh.name.starts_with("ground:");
        mesh.is_floor_or_ground = ground;
        mesh.prepass_eligible = !alpha_tested;
        if ground {
            mesh.shadow_class = ShadowCasterClass::None;
        }
        builder.add_mesh(mesh, material);
    }
    Ok(())
}

fn load_car(builder: &mut SceneBuilder, directory: &Path) -> Result<CarModel, String> {
    let geometry = CarGeometry::load(directory.join("car.json"))?;
    let path = directory.join("car.glb");
    let scene = import_gltf_scene(&path, import_options())
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let cutout: Vec<bool> = scene
        .materials
        .iter()
        .map(|m| m.flags.alpha_tested)
        .collect();
    let ids: Vec<_> = scene
        .materials
        .into_iter()
        .map(|material| builder.add_material(material))
        .collect();
    let mut body = Vec::new();
    let mut wheels: [Vec<Mesh>; 4] = Default::default();
    for node in scene.nodes {
        let wheel = ["wheel_fl", "wheel_fr", "wheel_rl", "wheel_rr"]
            .iter()
            .position(|name| node.name == *name);
        for mut mesh in node.meshes {
            mesh.prepass_eligible = !cutout[mesh.material_index];
            mesh.material_index = ids[mesh.material_index].index();
            match wheel {
                // Wheel nodes already pivot at the wheel centre; keep vertices local.
                Some(index) => wheels[index].push(mesh),
                None => {
                    mesh.transform = node.transform;
                    mesh.bake_transform();
                    body.push(mesh);
                }
            }
        }
    }
    if body.is_empty() || wheels.iter().any(Vec::is_empty) {
        return Err(format!("{} is missing body or wheel nodes", path.display()));
    }
    Ok(CarModel {
        body,
        wheels,
        geometry,
    })
}

pub fn spawn_cars(world: &mut World, race: &Race, models: &[CarModel]) -> Vec<CarEntities> {
    race.entrants
        .iter()
        .enumerate()
        .map(|(index, entrant)| {
            let model = if entrant.car.spec.name == "RB22" {
                0
            } else {
                1
            };
            let spawn = |world: &mut World, mesh: &Mesh, part: &str| {
                let entity = world.spawn(format!("car{index}-{part}"), Transform::IDENTITY);
                world.set_mesh_renderer(
                    entity,
                    MeshRenderer {
                        mesh: mesh.clone(),
                        visible: true,
                        prepass_eligible: mesh.prepass_eligible,
                        reflection_proxy: mesh.reflection_proxy,
                    },
                );
                entity
            };
            let body = models[model]
                .body
                .iter()
                .map(|mesh| spawn(world, mesh, "body"))
                .collect();
            let wheels = std::array::from_fn(|wheel| {
                models[model].wheels[wheel]
                    .iter()
                    .map(|mesh| spawn(world, mesh, "wheel"))
                    .collect()
            });
            CarEntities {
                body,
                wheels,
                model,
            }
        })
        .collect()
}

/// Copies car poses from the simulation into render entities.
pub fn sync_cars(world: &mut World, session: &Session) {
    for (entrant, entities) in session.race.entrants.iter().zip(&session.cars) {
        let car = &entrant.car;
        let origin = car.model_origin();
        // Engine transforms rotate Y (yaw), then X (pitch, + is nose down), then Z (roll).
        let body = Transform {
            previous_position: Vec3::ZERO,
            position: Vec3::new(origin.x as f32, origin.y as f32, origin.z as f32),
            rotation: Vec3::new(-car.pitch as f32, car.yaw as f32, car.roll as f32),
            scale: Vec3::ONE,
        };
        for &entity in &entities.body {
            set_transform(world, entity, body);
        }
        for (wheel, parts) in entities.wheels.iter().enumerate() {
            let spec = &car.spec.wheels[wheel];
            let centre = car.model_to_world(spec.position);
            let steer = if wheel < 2 { car.steer_angle } else { 0.0 };
            let transform = Transform {
                previous_position: Vec3::ZERO,
                position: Vec3::new(centre.x as f32, centre.y as f32, centre.z as f32),
                rotation: Vec3::new(
                    car.wheels[wheel].rotation as f32,
                    (car.yaw + steer) as f32,
                    0.0,
                ),
                scale: Vec3::ONE,
            };
            for &entity in parts {
                set_transform(world, entity, transform);
            }
        }
    }
}

fn set_transform(world: &mut World, entity: EntityId, transform: Transform) {
    if let Some(current) = world.transform_mut(entity) {
        let previous = current.position;
        *current = transform;
        current.previous_position = previous;
    }
}
