//! Surface effects use race time and contact telemetry; neither RNG nor wall time enters the emitters.
//!
//! Smoke, dust and sparks are instances of one quad drawn by `particles.wesl`:
//! every particle's path is a closed form of its age, so a rewind or a replay
//! shows the same cloud. Skid marks stay ordinary decals.
use crate::{graphics::Level, menu::Frontend, sky::Weather, Session};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    mesh::MeshTag,
    prelude::*,
    render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat},
    shader::ShaderRef,
};
use racecraft_sim::Surface;

const FADE_STEPS: usize = 12;
const GRAVITY: f32 = 9.81;
/// Emitter ticks per second of race time.
const RATE: f64 = 30.;
/// Share of its speed a spark keeps through a bounce.
const BOUNCE: f32 = 0.38;
/// Collision impulse (N·s) below which a contact throws no sparks.
const IMPACT_THRESHOLD: f32 = 700.;
/// Seconds of relative motion a spark's streak covers, like a shutter.
const STREAK_SHUTTER: f32 = 0.017;
/// Texels along each edge of the smoke cloud sheet.
const NOISE_SIZE: usize = 128;

#[derive(Clone, Copy, PartialEq, Debug)]
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

#[derive(Component, Clone, Debug)]
pub struct Particle {
    born: f64,
    life: f64,
    origin: Vec3,
    velocity: Vec3,
    /// Starting radius of a puff, or the width of a spark's streak, metres.
    radius: f32,
    /// How many starting radii a puff swells by.
    growth: f32,
    /// Share of horizontal speed shed per second.
    drag: f32,
    /// Ground slope at the origin (dy/dx, dy/dz): sparks bounce on this plane.
    slope: Vec2,
    seed: u8,
    strength: u8,
    kind: Kind,
}

/// A collision reported by the simulation, waiting to throw sparks.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub point: Vec3,
    pub velocity: Vec3,
    /// N·s.
    pub impulse: f32,
}

/// Lit, soft particles: see `particles.wesl`.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct ParticleMaterial {
    #[uniform(0)]
    tint: Vec4,
    #[uniform(0)]
    ambient: Vec4,
    #[texture(1)]
    #[sampler(2)]
    noise: Handle<Image>,
    additive: bool,
}

impl Material for ParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://f1_the_game/particles.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://f1_the_game/particles.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        if self.additive {
            AlphaMode::Add
        } else {
            AlphaMode::Premultiplied
        }
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "particles.wesl");
        app.add_plugins(MaterialPlugin::<ParticleMaterial>::default())
            .add_systems(Startup, setup);
    }
}

#[derive(Resource)]
pub struct Resources {
    quad: Handle<Mesh>,
    /// One material per airborne kind; `Kind::Rubber` has none.
    airborne: [Handle<ParticleMaterial>; 4],
    marks: [Handle<StandardMaterial>; FADE_STEPS],
    last: i64,
    contacts: Vec<[Option<Vec3>; 4]>,
    /// How hot each tyre has slid itself, 0..1: see [`warm`].
    heat: Vec<[f32; 4]>,
    /// Per car, the strength of a plank strike still throwing sparks: see [`grind`].
    grinding: Vec<f32>,
    warmup: u32,
    look: Option<Weather>,
}

/// Colour, and optical thickness (smoke and dust) or radiance (sparks), of each airborne kind.
const TINTS: [Vec4; 4] = [
    // Burnt rubber smoke is a slightly blue white.
    Vec4::new(0.5, 0.505, 0.52, 3.0),
    Vec4::new(0.3, 0.28, 0.17, 2.2),
    Vec4::new(0.42, 0.37, 0.29, 2.8),
    // Written to the frame as is: far past white, so bloom picks it up.
    Vec4::new(1., 1., 1., 150.),
];

/// Sky fill on a puff, in the scene's radiance units.
fn ambient(weather: Weather) -> Vec3 {
    match weather {
        Weather::Clear => Vec3::new(3300., 3700., 4300.),
        Weather::Storm => Vec3::new(520., 580., 680.),
    }
}

/// A tiling sheet of cloud: red is a slow billow, green finer wisps, and
/// blue and alpha the billow's slope, which the shader lights as lumps.
fn noise_image() -> Image {
    fn lattice(cell: IVec2, period: i32, salt: u32) -> f32 {
        let cell = cell.rem_euclid(IVec2::splat(period)).as_uvec2();
        let mut h = cell
            .x
            .wrapping_mul(0x9E37_79B1)
            .wrapping_add(cell.y.wrapping_mul(0x85EB_CA77))
            .wrapping_add(salt.wrapping_mul(0xC2B2_AE3D));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^= h >> 12;
        (h & 0xFFFF) as f32 / 65535.
    }
    fn octave(at: Vec2, period: i32, salt: u32) -> f32 {
        let scaled = at * period as f32;
        let cell = scaled.floor();
        let f = scaled - cell;
        let s = f * f * (3. - 2. * f);
        let cell = cell.as_ivec2();
        let corner = |x, y| lattice(cell + IVec2::new(x, y), period, salt);
        let near = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * s.x;
        let far = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * s.x;
        near + (far - near) * s.y
    }
    fn cloud(at: Vec2, periods: [i32; 4], salt: u32) -> f32 {
        let mut sum = 0.;
        let mut weight = 0.5;
        for (index, period) in periods.into_iter().enumerate() {
            sum += octave(at, period, salt + index as u32) * weight;
            weight *= 0.5;
        }
        // Stretch the field over the whole range: the shader's thresholds
        // assume values spread from 0 to 1.
        ((sum / 0.9375 - 0.5) * 2.1 + 0.5).clamp(0., 1.)
    }
    let size = NOISE_SIZE as i32;
    let billow = |x: i32, y: i32| {
        let at = IVec2::new(x, y).rem_euclid(IVec2::splat(size)).as_vec2() / size as f32;
        cloud(at, [3, 6, 12, 24], 1)
    };
    let mut pixels = Vec::with_capacity(NOISE_SIZE * NOISE_SIZE * 4);
    for y in 0..size {
        for x in 0..size {
            let at = Vec2::new(x as f32, y as f32) / size as f32;
            let slope = Vec2::new(
                billow(x + 2, y) - billow(x - 2, y),
                billow(x, y + 2) - billow(x, y - 2),
            ) * 2.2;
            let byte = |value: f32| (value.clamp(0., 1.) * 255.) as u8;
            pixels.extend_from_slice(&[
                byte(billow(x, y)),
                byte(cloud(at, [8, 16, 32, 64], 40)),
                byte(0.5 + slope.x),
                byte(0.5 + slope.y),
            ]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: NOISE_SIZE as u32,
            height: NOISE_SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut particles: ResMut<Assets<ParticleMaterial>>,
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
    let marks = std::array::from_fn(|fade| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.035, 0.032, 0.03)
                .with_alpha(0.24 * (1. - fade as f32 / FADE_STEPS as f32)),
            base_color_texture: Some(cloud.clone()),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            depth_bias: 2.,
            ..default()
        })
    });
    let noise = images.add(noise_image());
    let airborne = std::array::from_fn(|kind| {
        particles.add(ParticleMaterial {
            tint: TINTS[kind],
            ambient: ambient(Weather::Clear).extend(if kind == Kind::Spark.index() {
                1.
            } else {
                0.
            }),
            noise: noise.clone(),
            additive: kind == Kind::Spark.index(),
        })
    });
    commands.insert_resource(Resources {
        quad: meshes.add(Rectangle::new(1., 1.)),
        airborne,
        marks,
        last: -1,
        contacts: Vec::new(),
        heat: Vec::new(),
        grinding: Vec::new(),
        warmup: 0,
        look: None,
    });
}

#[derive(Component)]
pub struct Warmup;

impl Particle {
    pub fn is_spark(&self) -> bool {
        self.kind == Kind::Spark
    }

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

    /// Horizontal distance covered while drag bleeds the launch speed away.
    fn drift(&self, age: f32) -> Vec3 {
        let reach = if self.drag > 1e-3 {
            (1. - (-self.drag * age).exp()) / self.drag
        } else {
            age
        };
        Vec3::new(self.velocity.x, 0., self.velocity.z) * reach
    }

    /// Radius of a puff at `age` seconds: a quick swell that keeps creeping.
    fn puff_radius(&self, age: f32) -> f32 {
        self.radius * (1. + self.growth * (1. - (-2.4 * age).exp()) + 0.35 * age)
    }

    /// Centre of a puff. It lifts as it swells, so it sits on the road
    /// rather than sinking through it, and drifts off on a light breeze.
    fn puff(&self, age: f32) -> Vec3 {
        // Warm smoke rises faster the longer it has been free of the tyre.
        let lift =
            (self.puff_radius(age) - self.radius) * 0.55 + self.velocity.y * age + 0.22 * age * age;
        // Turbulence: every puff wanders on its own slow eddy.
        let phase = self.seed as f32 * 0.0246;
        let eddy = Vec3::new(
            (phase * 7. + age * 2.3).sin() - (phase * 7.).sin(),
            0.35 * ((phase * 3. + age * 1.7).cos() - (phase * 3.).cos()),
            (phase * 11. + age * 1.9).cos() - (phase * 11.).cos(),
        ) * (0.28 * age.min(1.5));
        self.origin + self.drift(age) + Vec3::new(0.45, 0., 0.3) * age + Vec3::Y * lift + eddy
    }

    /// A spark's position and velocity: a ballistic arc over the ground plane
    /// it was born on, losing most of its speed at each bounce.
    fn spark(&self, age: f32) -> (Vec3, Vec3) {
        let mut left = age;
        let mut height = 0.;
        let mut climb = self.velocity.y;
        for _ in 0..4 {
            let landing = (climb + (climb * climb + 2. * GRAVITY * height).sqrt()) / GRAVITY;
            if left < landing {
                height += climb * left - 0.5 * GRAVITY * left * left;
                climb -= GRAVITY * left;
                left = 0.;
                break;
            }
            left -= landing;
            climb = (GRAVITY * landing - climb) * BOUNCE;
            height = 0.;
            if climb < 0.25 {
                // Too slow to leave the road again: it skitters along it.
                climb = 0.;
                break;
            }
        }
        if left > 0. {
            height = 0.;
            climb = 0.;
        }
        let drift = self.drift(age);
        let travel = Vec3::new(self.velocity.x, 0., self.velocity.z) * (-self.drag * age).exp();
        let ground = self.slope.dot(Vec2::new(drift.x, drift.z));
        let position = self.origin + drift + Vec3::Y * (ground + height);
        let velocity = travel + Vec3::Y * (climb + self.slope.dot(Vec2::new(travel.x, travel.z)));
        (position, velocity)
    }

    /// Age, seed and strength packed for the shader.
    fn tag(&self, age: f64) -> MeshTag {
        let age = ((age / self.life).clamp(0., 1.) * 1023.) as u32;
        MeshTag::new(age | (self.seed as u32) << 10 | (self.strength as u32) << 18)
    }
}

/// Airborne particles, skid marks and sparks alive at once, and sparks thrown
/// per emitter tick by a grounding plank.
fn limits(quality: Level) -> (usize, usize, usize, usize) {
    match quality {
        Level::Off => (0, 0, 0, 0),
        Level::Low => (40, 64, 60, 3),
        Level::Medium => (80, 128, 160, 7),
        Level::High => (160, 256, 400, 16),
        Level::Ultra => (260, 512, 640, 24),
    }
}

/// Seconds of continuous sliding before a tyre smokes at full strength, and
/// seconds of grip for it to cool again.
const SMOKE_BUILD: f32 = 2.5;
const SMOKE_COOL: f32 = 0.7;
/// Seconds a plank strike keeps showering sparks after the contact.
const GRIND_HOLD: f32 = 0.35;

/// Whether a tyre is scrubbing over the road fast enough to mark and smoke
/// it. A locked wheel on a stopped car, or a slide at walking pace, is not.
fn slipping(speed: f64, locked: bool, spinning: bool, sliding: f64) -> bool {
    spinning || (locked && speed > 3.) || (sliding > 0.95 && speed > 8.)
}

/// A tyre's slide heat after one more emitter tick. A brief lock-up or a
/// launch only wisps; a held slide or burnout builds into thick smoke.
fn warm(heat: f32, slipping: bool) -> f32 {
    let step = if slipping {
        1. / SMOKE_BUILD
    } else {
        -1. / SMOKE_COOL
    };
    (heat + step / RATE as f32).clamp(0., 1.)
}

/// How hard the plank is striking the road, 0.25..1, if it is at all.
/// A compression proxy, not a measured clearance: the body sits `squash`
/// metres below its ride height over rising ground, or the tyres carry
/// `overload` times the steady aero and weight while crossing a kerb.
fn strike(speed: f64, squash: f64, overload: f64, loaded_kerb: bool) -> Option<f32> {
    if speed <= 45. || !((loaded_kerb && overload > 1.3) || squash > 0.015) {
        return None;
    }
    Some(
        (((squash - 0.015) / 0.03).max(0.) as f32)
            .max(((overload - 1.3) / 0.5) as f32)
            .clamp(0.25, 1.),
    )
}

/// The strength of a car's spark shower after one more emitter tick: a new
/// strike sets it, and it fades over [`GRIND_HOLD`] rather than flickering.
fn grind(held: f32, strike: Option<f32>) -> f32 {
    let faded = (held - 1. / (GRIND_HOLD * RATE as f32)).max(0.);
    strike.map_or(faded, |strike| strike.max(faded))
}

/// A repeatable value in 0..1 from an emitter tick and a few small indices.
fn noise(tick: i64, a: usize, b: usize) -> f32 {
    let mut h = (tick as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((a as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add((b as u64).wrapping_mul(0x1656_67B1_9E37_79F9));
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h & 0xFF_FFFF) as f32 / 16_777_215.
}

fn spawn(commands: &mut Commands, resources: &Resources, particle: Particle, transform: Transform) {
    let common = (
        Mesh3d(resources.quad.clone()),
        transform,
        bevy::light::NotShadowCaster,
        bevy::light::NotShadowReceiver,
    );
    if particle.kind == Kind::Rubber {
        commands.spawn((common, MeshMaterial3d(resources.marks[0].clone()), particle));
    } else {
        commands.spawn((
            common,
            MeshMaterial3d(resources.airborne[particle.kind.index()].clone()),
            particle.tag(0.),
            // The shader turns and sizes the quad, so its bounds say little.
            bevy::camera::visibility::NoFrustumCulling,
            particle,
        ));
    }
}

/// Sparks from a point: a fan of `count` around `velocity`, spread by `spray` m/s.
#[allow(clippy::too_many_arguments)]
fn shower(
    commands: &mut Commands,
    resources: &Resources,
    now: f64,
    tick: i64,
    salt: usize,
    origin: Vec3,
    velocity: Vec3,
    slope: Vec2,
    spray: Vec3,
    count: usize,
    heat: f32,
) {
    for index in 0..count {
        let roll = |channel: usize| noise(tick, salt * 64 + index, channel);
        let kick = Vec3::new(
            (roll(0) - 0.5) * 2. * spray.x,
            roll(1) * roll(1) * spray.y,
            (roll(2) - 0.5) * 2. * spray.z,
        );
        // Spread births across the tick so a shower is not a row of clones.
        let born = now - roll(3) as f64 / RATE;
        let particle = Particle {
            born,
            life: 0.3 + 0.75 * (roll(4) * roll(4)) as f64,
            origin,
            velocity: velocity * (0.55 + 0.4 * roll(5)) + kick,
            radius: 0.012 + 0.012 * roll(6),
            growth: 0.,
            drag: 2.2 + 2.5 * roll(7),
            slope,
            seed: (roll(8) * 255.) as u8,
            strength: ((0.45 + 0.55 * roll(9)) * heat.clamp(0.3, 1.) * 255.) as u8,
            kind: Kind::Spark,
        };
        spawn(
            commands,
            resources,
            particle,
            Transform::from_translation(origin).with_scale(Vec3::splat(1e-4)),
        );
    }
}

type WarmupQuery<'w, 's> =
    Query<'w, 's, (Entity, &'static mut Transform), (With<Warmup>, Without<Particle>)>;

type ParticleQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Particle,
        &'static mut Transform,
        Option<&'static mut MeshMaterial3d<StandardMaterial>>,
        Option<&'static mut MeshTag>,
    ),
>;

#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    mut session: ResMut<Session>,
    frontend: Res<Frontend>,
    mut resources: ResMut<Resources>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut warmup: WarmupQuery,
    mut particles: ParticleQuery,
) {
    let now = session.race.time;
    let (budget, mark_budget, spark_budget, plank_sparks) = limits(frontend.video.effects);
    let (eye, target) = session.camera.listener();
    let impacts = std::mem::take(&mut session.contacts);
    let session = &*session;
    let look = frontend.draft.weather;
    if resources.look != Some(look) {
        resources.look = Some(look);
        for handle in &resources.airborne {
            if let Some(mut material) = materials.get_mut(handle) {
                material.ambient = ambient(look).extend(material.ambient.w);
            }
        }
    }
    // Render subpixel samples of every alpha pipeline during loading. The first
    // lock-up must not compile a new transparent/additive shader on the racing line.
    if budget > 0 && resources.warmup < 32 {
        let point = (eye + (target - eye).normalize_or_zero() * 2.).as_vec3();
        if warmup.is_empty() {
            let sample = || {
                (
                    Warmup,
                    Mesh3d(resources.quad.clone()),
                    Transform::from_translation(point).with_scale(Vec3::splat(0.0001)),
                    bevy::light::NotShadowCaster,
                    bevy::light::NotShadowReceiver,
                )
            };
            commands.spawn((sample(), MeshMaterial3d(resources.marks[0].clone())));
            for material in &resources.airborne {
                commands.spawn((
                    sample(),
                    MeshMaterial3d(material.clone()),
                    MeshTag::new(128),
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
    // Streaks are drawn as the camera sees them move, and it rides with the player.
    let player = &session.race.entrants[session.player].car;
    let camera_velocity = Vec3::new(player.velocity.x as f32, 0., player.velocity.y as f32);
    let mut count = 0;
    let mut marks = 0;
    let mut sparks = 0;
    for (entity, particle, mut transform, material, tag) in &mut particles {
        let age = now - particle.born;
        let (active, limit) = match particle.kind {
            Kind::Rubber => (&mut marks, mark_budget),
            Kind::Spark => (&mut sparks, spark_budget),
            _ => (&mut count, budget),
        };
        // A particle born later in this tick has not appeared yet.
        if age < -1. / RATE || age >= particle.life || *active >= limit {
            commands.entity(entity).despawn();
            continue;
        }
        *active += 1;
        if let Some(mut tag) = tag {
            tag.set_if_neq(particle.tag(age.max(0.)));
        }
        let seconds = age.max(0.) as f32;
        match particle.kind {
            Kind::Rubber => {
                let fade = ((age / particle.life).clamp(0., 0.999) * FADE_STEPS as f64) as usize;
                if let Some(mut material) = material {
                    if material.0 != resources.marks[fade] {
                        material.0 = resources.marks[fade].clone();
                    }
                }
            }
            Kind::Spark => {
                let (position, velocity) = particle.spark(seconds);
                let streak = velocity - camera_velocity;
                let length = (streak.length() * STREAK_SHUTTER).clamp(0.05, 1.1);
                let along = streak.normalize_or(Vec3::Y);
                *transform = Transform::from_translation(position - along * (length * 0.5))
                    .with_rotation(Quat::from_rotation_arc(Vec3::Y, along))
                    .with_scale(if age < 0. {
                        Vec3::splat(1e-4)
                    } else {
                        Vec3::new(particle.radius, length, 1.)
                    });
            }
            _ => {
                transform.translation = particle.puff(seconds);
                transform.scale = Vec3::splat(if age < 0. {
                    1e-4
                } else {
                    particle.puff_radius(seconds) * 2.
                });
            }
        }
    }
    if budget == 0 {
        resources.contacts.clear();
        resources.heat.clear();
        resources.grinding.clear();
        return;
    }
    let tick = (now * RATE).floor() as i64;
    if tick == resources.last || session.paused || !session.ready {
        return;
    }
    if tick < resources.last {
        resources.contacts.clear();
        resources.heat.clear();
        resources.grinding.clear();
    }
    resources.last = tick;
    let entrants = session.race.entrants.len();
    resources.contacts.resize(entrants, [None; 4]);
    resources.heat.resize(entrants, [0.; 4]);
    resources.grinding.resize(entrants, 0.);
    for (index, contact) in impacts.iter().enumerate() {
        if contact.impulse < IMPACT_THRESHOLD
            || contact.point.distance(player.position.as_vec3()) > 90.
            || sparks >= spark_budget
        {
            continue;
        }
        let force = (contact.impulse / 9000.).clamp(0.15, 1.);
        let burst = ((plank_sparks * 4) as f32 * force).ceil() as usize;
        let burst = burst.min(spark_budget - sparks);
        shower(
            &mut commands,
            &resources,
            now,
            tick,
            200 + index,
            contact.point,
            contact.velocity * 0.6,
            Vec2::ZERO,
            Vec3::new(5., 5.5, 5.) * (0.5 + 0.5 * force),
            burst,
            0.6 + 0.4 * force,
        );
        sparks += burst;
    }
    // Player first so nearby traffic cannot consume its entire budget.
    let order = std::iter::once(session.player)
        .chain((0..session.race.entrants.len()).filter(|i| *i != session.player));
    for entrant in order {
        let car = &session.race.entrants[entrant].car;
        // A burnout smokes standing still; anything else slow or far is skipped.
        let burnout = car.wheels.iter().any(|wheel| wheel.spinning);
        if (car.speed() < 8. && !burnout) || car.position.distance(player.position) > 60. {
            resources.contacts[entrant] = [None; 4];
            resources.heat[entrant] = [0.; 4];
            resources.grinding[entrant] = 0.;
            continue;
        }
        let velocity = Vec3::new(car.velocity.x as f32, 0., car.velocity.y as f32);
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
        let squash = ground - car.body_height;
        let overload = load / static_load;
        let heat = grind(
            resources.grinding[entrant],
            strike(car.speed(), squash, overload, loaded_kerb),
        );
        resources.grinding[entrant] = heat;
        if sparks < spark_budget && heat > 0. {
            let burst = ((plank_sparks as f32 * (0.4 + 0.6 * heat)).ceil() as usize)
                .min(spark_budget - sparks);
            // The skid blocks sit along the plank; most of the shower leaves
            // under the rear of the floor.
            let rear = car.spec.wheels[2].position.z;
            let along = rear * (0.35 + 0.5 * noise(tick, entrant, 90) as f64);
            let across = (noise(tick, entrant, 91) as f64 - 0.5) * 0.3;
            let point = car.model_to_world(glam::DVec3::new(across, 0., along));
            let slope = session
                .race
                .track
                .collision
                .ground(point)
                .map_or(Vec2::ZERO, |g| {
                    let normal = g.normal.as_vec3();
                    Vec2::new(-normal.x, -normal.z) / normal.y.max(0.2)
                });
            shower(
                &mut commands,
                &resources,
                now,
                tick,
                entrant,
                Vec3::new(point.x as f32, ground as f32 + 0.03, point.z as f32),
                velocity,
                slope,
                Vec3::new(2.2, 3.4, 2.2),
                burst,
                heat,
            );
            sparks += burst;
        }
        for (index, wheel) in car.wheels.iter().enumerate() {
            let point = car.model_to_world(car.spec.wheels[index].position);
            let origin = Vec3::new(
                point.x as f32,
                wheel.ground_height as f32 + 0.025,
                point.z as f32,
            );
            let slipping = slipping(car.speed(), wheel.locked, wheel.spinning, wheel.sliding);
            let heat = warm(resources.heat[entrant][index], slipping && wheel.load > 0.);
            resources.heat[entrant][index] = heat;
            let on_road = matches!(
                wheel.surface,
                Surface::Asphalt | Surface::Concrete | Surface::Kerb
            );
            let previous = resources.contacts[entrant][index].replace(origin);
            if wheel.load <= 0. {
                resources.contacts[entrant][index] = None;
                continue;
            }
            let previous = previous.filter(|p| p.distance(origin) < 6.);
            if slipping && on_road && marks < mark_budget {
                if let Some(previous) = previous {
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
                                origin: mid,
                                velocity: Vec3::ZERO,
                                radius: 0.,
                                growth: 0.,
                                drag: 0.,
                                slope: Vec2::ZERO,
                                seed: 0,
                                strength: 255,
                                kind: Kind::Rubber,
                            },
                            transform,
                        );
                        marks += 1;
                    }
                }
            }
            let kind = match wheel.surface {
                Surface::Grass => Kind::Grass,
                Surface::Gravel => Kind::Gravel,
                _ if slipping => Kind::Smoke,
                _ => continue,
            };
            let smoke = kind == Kind::Smoke;
            // A locked or spinning tyre pours smoke once it is hot; one merely
            // past its grip, or only just sliding, wisps.
            let build = if smoke { 0.1 + 0.9 * heat * heat } else { 1. };
            let strength = if !smoke {
                (car.speed() as f32 / 35.).clamp(0.35, 1.)
            } else if wheel.locked || wheel.spinning {
                build
            } else {
                build * (0.45 + 0.55 * ((wheel.sliding as f32 - 0.95) / 0.05).clamp(0., 1.))
            };
            // At speed one puff per tick leaves gaps: lay extra ones back
            // along the path the tyre covered since the last tick.
            let puffs = previous.map_or(1, |previous| {
                (previous.distance(origin) / 1.3).ceil().clamp(1., 2.) as usize
            });
            // A cool tyre only puffs now and then.
            if smoke && noise(tick, entrant * 4 + index, 77) > 0.3 + 0.7 * heat {
                continue;
            }
            for puff in 0..puffs {
                if count >= budget {
                    break;
                }
                let roll = |channel: usize| noise(tick, entrant * 16 + index * 4 + puff, channel);
                let back = puff as f32 / puffs as f32;
                let origin = previous.map_or(origin, |previous| origin.lerp(previous, back));
                let side = Vec3::new(roll(0) - 0.5, 0., roll(1) - 0.5);
                // Smoke leaves with the tyre's surface: carried along by a
                // locked wheel, flung back by a spinning one.
                let forward = car.forward();
                let forward = Vec3::new(forward.x as f32, 0., forward.y as f32);
                let carried = if wheel.spinning {
                    velocity * 0.12 - forward * (1.5 + 2.5 * roll(2))
                } else {
                    velocity * if smoke { 0.3 } else { 0.22 }
                };
                let particle = Particle {
                    born: now - (back as f64 + roll(3) as f64 * 0.3) / RATE,
                    life: if smoke {
                        (1.5 + 1.3 * roll(4) as f64) * (0.4 + 0.6 * heat as f64)
                    } else {
                        0.9 + 0.8 * roll(4) as f64
                    },
                    origin,
                    velocity: carried
                        + side * 1.1
                        + Vec3::Y * if smoke { 0.35 + 0.5 * roll(5) } else { 0.15 },
                    radius: 0.2 + 0.1 * roll(6),
                    growth: if smoke {
                        (3.6 + 1.6 * roll(7)) * (0.3 + 0.7 * heat)
                    } else {
                        2.4
                    },
                    drag: 2.6,
                    slope: Vec2::ZERO,
                    seed: (roll(8) * 255.) as u8,
                    strength: (strength * 255.) as u8,
                    kind,
                };
                spawn(
                    &mut commands,
                    &resources,
                    particle,
                    Transform::from_translation(origin).with_scale(Vec3::splat(1e-4)),
                );
                count += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spark(velocity: Vec3) -> Particle {
        Particle {
            born: 0.,
            life: 1.,
            origin: Vec3::new(0., 2., 0.),
            velocity,
            radius: 0.02,
            growth: 0.,
            drag: 0.,
            slope: Vec2::ZERO,
            seed: 0,
            strength: 255,
            kind: Kind::Spark,
        }
    }

    #[test]
    fn sparks_arc_bounce_lower_each_time_and_never_sink_below_the_road() {
        let particle = spark(Vec3::new(3., 4., 0.));
        let mut peaks = Vec::new();
        let mut rising = true;
        let mut previous = 2.;
        for step in 1..400 {
            let (position, velocity) = particle.spark(step as f32 * 0.005);
            assert!(position.y >= 2. - 1e-4, "sank to {}", position.y);
            if rising && position.y < previous {
                peaks.push(previous - 2.);
            }
            rising = position.y >= previous;
            previous = position.y;
            assert!(velocity.is_finite());
        }
        assert!(peaks.len() >= 2, "expected bounces, got {peaks:?}");
        // First apex of a 4 m/s launch is v²/2g; each bounce keeps BOUNCE² of it.
        assert!((peaks[0] - 16. / (2. * GRAVITY)).abs() < 0.02);
        assert!(peaks[1] < peaks[0] * BOUNCE * BOUNCE * 1.2);
        // It ends lying on the road, sliding with no vertical motion.
        let (rest, velocity) = particle.spark(5.);
        assert!((rest.y - 2.).abs() < 1e-4 && velocity.y == 0.);
    }

    #[test]
    fn sparks_follow_a_sloping_road_and_drag_stops_them() {
        let mut particle = spark(Vec3::new(20., 0., 0.));
        particle.slope = Vec2::new(0.1, 0.);
        particle.drag = 4.;
        let (position, _) = particle.spark(3.);
        // Drag limits the reach to speed / drag, and the road rose under it.
        assert!((position.x - 5.).abs() < 0.01);
        assert!((position.y - 2.5).abs() < 0.01);
    }

    #[test]
    fn puffs_swell_rise_and_pack_their_state_for_the_shader() {
        let particle = Particle {
            life: 2.,
            velocity: Vec3::new(10., 0.5, 0.),
            radius: 0.25,
            growth: 4.,
            drag: 2.6,
            seed: 200,
            strength: 128,
            kind: Kind::Smoke,
            ..spark(Vec3::ZERO)
        };
        assert_eq!(particle.puff_radius(0.), 0.25);
        assert!(particle.puff_radius(2.) > 1.2);
        let (young, old) = (particle.puff(0.), particle.puff(2.));
        assert_eq!(young, particle.origin);
        // The base of the swollen puff has not sunk below where it was born.
        assert!(old.y - particle.puff_radius(2.) * 0.6 > young.y - 0.25);
        assert!(old.x > 2.5 && old.x < 10. / 2.6 + 2.);
        let tag = particle.tag(1.).value;
        assert_eq!(tag & 1023, 511);
        assert_eq!((tag >> 10) & 255, 200);
        assert_eq!((tag >> 18) & 255, 128);
    }

    #[test]
    fn tyres_smoke_only_while_scrubbing_and_build_up_to_it() {
        // Stopped on the brakes or creeping sideways is not a slide.
        assert!(!slipping(0.5, true, false, 1.));
        assert!(slipping(30., true, false, 0.));
        assert!(slipping(0., false, true, 0.));
        assert!(slipping(20., false, false, 0.97) && !slipping(20., false, false, 0.9));
        // A half-second lock-up barely warms the tyre; a held slide saturates.
        let mut heat = 0.;
        for _ in 0..(0.5 * RATE) as usize {
            heat = warm(heat, true);
        }
        assert!((heat - 0.5 / SMOKE_BUILD).abs() < 0.03, "{heat}");
        for _ in 0..(3. * RATE) as usize {
            heat = warm(heat, true);
        }
        assert_eq!(heat, 1.);
        for _ in 0..(SMOKE_COOL * RATE as f32) as usize + 1 {
            heat = warm(heat, false);
        }
        assert_eq!(heat, 0.);
    }

    #[test]
    fn plank_strikes_need_speed_and_fade_instead_of_flickering() {
        assert_eq!(strike(40., 0.05, 2., true), None);
        assert_eq!(strike(80., 0.01, 1.2, true), None);
        // Overload only counts on a kerb; squash counts anywhere.
        assert_eq!(strike(80., 0., 1.6, false), None);
        assert_eq!(strike(80., 0.016, 1., false), Some(0.25));
        assert_eq!(strike(80., 0.06, 1., false), Some(1.));
        let hard = strike(80., 0., 1.8, true).unwrap();
        assert_eq!(hard, 1.);
        // One strike keeps showering for GRIND_HOLD, weakening all the way.
        let mut held = grind(0., Some(hard));
        let mut ticks = 0;
        while held > 0. {
            let next = grind(held, None);
            assert!(next < held);
            held = next;
            ticks += 1;
        }
        assert!(
            (ticks as f32 / RATE as f32 - GRIND_HOLD).abs() < 0.07,
            "{ticks}"
        );
        // A weaker strike does not cut a stronger shower short.
        assert_eq!(
            grind(0.9, Some(0.25)),
            0.9 - 1. / (GRIND_HOLD * RATE as f32)
        );
    }

    #[test]
    fn noise_is_repeatable_spread_and_tiles() {
        assert_eq!(noise(7, 3, 1), noise(7, 3, 1));
        let values: Vec<f32> = (0..200).map(|index| noise(11, index, 0)).collect();
        let mean = values.iter().sum::<f32>() / 200.;
        assert!((mean - 0.5).abs() < 0.08, "mean {mean}");
        assert!(values.iter().all(|value| (0. ..=1.).contains(value)));
        let image = noise_image();
        let data = image.data.as_ref().unwrap();
        assert_eq!(data.len(), NOISE_SIZE.pow(2) * 4);
        // The sheet tiles: opposite edges differ no more than neighbours do.
        for row in 0..NOISE_SIZE {
            let left = data[row * NOISE_SIZE * 4] as i32;
            let right = data[(row * NOISE_SIZE + NOISE_SIZE - 1) * 4] as i32;
            assert!((left - right).abs() < 40, "row {row}: {left} vs {right}");
        }
        let red: Vec<u8> = data.iter().step_by(4).copied().collect();
        assert!(red.iter().any(|v| *v < 40) && red.iter().any(|v| *v > 215));
    }
}
