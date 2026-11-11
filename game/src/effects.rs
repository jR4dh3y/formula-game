//! Surface effects use race time and contact telemetry; neither RNG nor wall time enters the emitters.
use crate::{graphics::Level, menu::Frontend, Session};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use racecraft_sim::Surface;

const FADE_STEPS: usize = 12;
#[derive(Clone, Copy)]
enum Kind {
    Smoke,
    Grass,
    Gravel,
    Spark,
    Rubber,
}
impl Kind {
    fn index(self) -> usize {
        self as usize
    }
}
#[derive(Component)]
pub struct Particle {
    born: f64,
    life: f64,
    velocity: Vec3,
    radius: f32,
    origin: Vec3,
    kind: Kind,
}
#[derive(Resource)]
pub struct Resources {
    quad: Handle<Mesh>,
    materials: Vec<[Handle<StandardMaterial>; FADE_STEPS]>,
    last: i64,
    contacts: Vec<[Option<Vec3>; 4]>,
    warmup: u32,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let size = 64;
    let mut pixels = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / size as f32 * 2. - Vec2::ONE;
            let cloud = ((p.x * 8. + p.y * 3.).sin() * (p.y * 11. - p.x * 2.).cos() * 0.12 + 0.88)
                .clamp(0., 1.);
            let alpha = (1. - p.length_squared()).max(0.).powi(3) * cloud;
            pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.) as u8]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let cloud = images.add(image);
    let colors = [
        // Unlit sprites: keep them near the shaded road and verge so a puff
        // reads as haze over the surface, not a white card.
        Color::srgb(0.52, 0.53, 0.55),
        Color::srgb(0.3, 0.32, 0.17),
        Color::srgb(0.5, 0.45, 0.36),
        Color::linear_rgb(12., 3., 0.2),
        Color::srgb(0.035, 0.032, 0.03),
    ];
    let materials = colors
        .into_iter()
        .enumerate()
        .map(|(kind, color)| {
            std::array::from_fn(|fade| {
                let opacity =
                    [0.42, 0.5, 0.55, 1., 0.24][kind] * (1. - fade as f32 / FADE_STEPS as f32);
                materials.add(StandardMaterial {
                    base_color: color.with_alpha(opacity),
                    base_color_texture: Some(cloud.clone()),
                    alpha_mode: if kind == Kind::Spark.index() {
                        AlphaMode::Add
                    } else {
                        AlphaMode::Blend
                    },
                    unlit: true,
                    cull_mode: None,
                    depth_bias: if kind == Kind::Rubber.index() { 2. } else { 0. },
                    ..default()
                })
            })
        })
        .collect();
    commands.insert_resource(Resources {
        quad: meshes.add(Rectangle::new(1., 1.)),
        materials,
        last: -1,
        contacts: Vec::new(),
        warmup: 0,
    });
}

#[derive(Component)]
pub struct Warmup;

impl Particle {
    #[cfg(test)]
    pub fn label(&self) -> &'static str {
        match self.kind {
            Kind::Smoke => "smoke",
            Kind::Grass => "grass",
            Kind::Gravel => "gravel",
            Kind::Spark => "spark",
            Kind::Rubber => "rubber",
        }
    }
}

fn limits(quality: Level) -> (usize, usize) {
    match quality {
        Level::Off => (0, 0),
        Level::Low => (48, 64),
        Level::Medium => (96, 128),
        Level::High => (240, 256),
        Level::Ultra => (360, 512),
    }
}

fn spawn(commands: &mut Commands, resources: &Resources, particle: Particle, transform: Transform) {
    commands.spawn((
        Mesh3d(resources.quad.clone()),
        MeshMaterial3d(resources.materials[particle.kind.index()][0].clone()),
        transform,
        particle,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));
}

type WarmupQuery<'w, 's> =
    Query<'w, 's, (Entity, &'static mut Transform), (With<Warmup>, Without<Particle>)>;

pub fn update(
    mut commands: Commands,
    session: Res<Session>,
    frontend: Res<Frontend>,
    mut resources: ResMut<Resources>,
    mut warmup: WarmupQuery,
    mut particles: Query<(
        Entity,
        &Particle,
        &mut Transform,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let now = session.race.time;
    let (budget, mark_budget) = limits(frontend.video.effects);
    let (eye, target) = session.camera.listener();
    // Render subpixel samples of every alpha pipeline during loading. The first
    // lock-up must not compile a new transparent/additive shader on the racing line.
    if budget > 0 && resources.warmup < 32 {
        let point = (eye + (target - eye).normalize_or_zero() * 2.).as_vec3();
        if warmup.is_empty() {
            for kind in 0..resources.materials.len() {
                commands.spawn((
                    Warmup,
                    Mesh3d(resources.quad.clone()),
                    MeshMaterial3d(resources.materials[kind][0].clone()),
                    Transform::from_translation(point).with_scale(Vec3::splat(0.0001)),
                    bevy::light::NotShadowCaster,
                    bevy::light::NotShadowReceiver,
                ));
            }
        }
        for (_, mut transform) in &mut warmup {
            transform.translation = point;
            transform.look_at(eye.as_vec3(), Vec3::Y);
        }
        if session.ready {
            resources.warmup += 1;
        }
    } else {
        for (entity, _) in &warmup {
            commands.entity(entity).despawn();
        }
    }
    let mut count = 0;
    let mut marks = 0;
    for (entity, particle, mut transform, mut material) in &mut particles {
        let age = now - particle.born;
        let rubber = matches!(particle.kind, Kind::Rubber);
        let active = if rubber { &mut marks } else { &mut count };
        if age < 0. || age >= particle.life || *active >= if rubber { mark_budget } else { budget }
        {
            commands.entity(entity).despawn();
            continue;
        }
        *active += 1;
        let fade = ((age / particle.life).clamp(0., 0.999) * FADE_STEPS as f64) as usize;
        material.0 = resources.materials[particle.kind.index()][fade].clone();
        if !rubber {
            let gravity = if matches!(particle.kind, Kind::Spark) {
                9.8
            } else {
                0.
            };
            transform.translation = particle.origin + particle.velocity * age as f32
                - Vec3::Y * (0.5 * gravity * age as f32 * age as f32);
            transform.look_at(eye.as_vec3(), Vec3::Y);
            let size = particle.radius
                * (1.
                    + age as f32
                        * if matches!(particle.kind, Kind::Spark) {
                            0.
                        } else {
                            1.8
                        });
            transform.scale = if matches!(particle.kind, Kind::Spark) {
                Vec3::new(size * 0.12, size, 1.)
            } else {
                Vec3::splat(size)
            };
        }
    }
    if budget == 0 {
        resources.contacts.clear();
        return;
    }
    let tick = (now * 30.).floor() as i64;
    if tick == resources.last || session.paused || !session.ready {
        return;
    }
    if tick < resources.last {
        resources.contacts.clear();
    }
    resources.last = tick;
    resources
        .contacts
        .resize(session.race.entrants.len(), [None; 4]);
    let player = &session.race.entrants[session.player].car;
    // Player first so nearby traffic cannot consume its entire budget.
    let order = std::iter::once(session.player)
        .chain((0..session.race.entrants.len()).filter(|i| *i != session.player));
    for entrant in order {
        let car = &session.race.entrants[entrant].car;
        if car.speed() < 8. || car.position.distance(player.position) > 60. {
            resources.contacts[entrant] = [None; 4];
            continue;
        }
        let loaded_kerb = car
            .wheels
            .iter()
            .any(|wheel| wheel.surface == Surface::Kerb && wheel.load > 0.);
        let load = car.wheels.iter().map(|wheel| wheel.load).sum::<f64>();
        let static_load = car.spec.mass * 9.81 + car.downforce;
        let ground = car
            .wheels
            .iter()
            .map(|wheel| wheel.ground_height)
            .sum::<f64>()
            * 0.25;
        // Compression proxy, not a measured plank clearance: suspension lags rising ground,
        // or the contact loads exceed steady aero+weight while crossing a kerb.
        if count < budget
            && car.speed() > 45.
            && ((loaded_kerb && load > static_load * 1.3) || ground - car.body_height > 0.015)
        {
            let origin = car
                .model_to_world(glam::DVec3::new(0., 0., car.spec.wheels[2].position.z))
                .as_vec3();
            let origin = Vec3::new(origin.x, ground as f32 + 0.04, origin.z);
            let side = (tick.rem_euclid(7) as f32 / 7. - 0.5) * 4.;
            let velocity = Vec3::new(
                car.velocity.x as f32 * 0.4 + side,
                1.2,
                car.velocity.y as f32 * 0.4 - side,
            );
            spawn(
                &mut commands,
                &resources,
                Particle {
                    born: now,
                    life: 0.18,
                    velocity,
                    radius: 0.3,
                    origin,
                    kind: Kind::Spark,
                },
                Transform::from_translation(origin).with_scale(Vec3::splat(0.03)),
            );
            count += 1;
        }
        for (index, wheel) in car.wheels.iter().enumerate() {
            let point = car.model_to_world(car.spec.wheels[index].position);
            let origin = Vec3::new(
                point.x as f32,
                wheel.ground_height as f32 + 0.025,
                point.z as f32,
            );
            let slipping = wheel.locked || wheel.spinning || wheel.sliding > 0.95;
            let on_road = matches!(
                wheel.surface,
                Surface::Asphalt | Surface::Concrete | Surface::Kerb
            );
            let previous = resources.contacts[entrant][index].replace(origin);
            if wheel.load <= 0. {
                resources.contacts[entrant][index] = None;
                continue;
            }
            if slipping && on_road && marks < mark_budget {
                if let Some(previous) = previous.filter(|p| p.distance(origin) < 6.) {
                    let delta = origin - previous;
                    if delta.length() > 0.1 {
                        let mid = (origin + previous) * 0.5;
                        let normal = session
                            .race
                            .track
                            .collision
                            .ground(mid.as_dvec3())
                            .map_or(Vec3::Y, |g| g.normal.as_vec3());
                        let transform = Transform::from_translation(mid + normal * 0.012)
                            .looking_to(normal, delta.normalize())
                            .with_scale(Vec3::new(0.24, delta.length() + 0.04, 1.));
                        spawn(
                            &mut commands,
                            &resources,
                            Particle {
                                born: now,
                                life: 18.,
                                velocity: Vec3::ZERO,
                                radius: 0.,
                                origin: mid,
                                kind: Kind::Rubber,
                            },
                            transform,
                        );
                        marks += 1;
                    }
                }
            }
            if count >= budget {
                continue;
            }
            let kind = match wheel.surface {
                Surface::Grass => Kind::Grass,
                Surface::Gravel => Kind::Gravel,
                _ if slipping => Kind::Smoke,
                _ => continue,
            };
            let side = ((tick * 17 + index as i64 * 11).rem_euclid(23)) as f32 / 23. - 0.5;
            let velocity = Vec3::new(
                car.velocity.x as f32 * 0.06 + side * 0.6,
                0.7,
                car.velocity.y as f32 * 0.06 - side * 0.6,
            );
            spawn(
                &mut commands,
                &resources,
                Particle {
                    born: now,
                    life: 1.1,
                    velocity,
                    radius: 0.6,
                    origin,
                    kind,
                },
                Transform::from_translation(origin)
                    .looking_at(eye.as_vec3(), Vec3::Y)
                    .with_scale(Vec3::splat(0.6)),
            );
            count += 1;
        }
    }
}
