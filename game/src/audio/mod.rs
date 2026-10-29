//! Game audio: procedural engines for the player and the nearest rivals, the
//! car's mechanical and environmental sounds, and a mastering bus.
//!
//! The game thread publishes simulation state through atomics; the audio
//! thread reads a snapshot every 64 frames and never locks or allocates.

mod dsp;
mod engine;

pub use engine::EngineChoice;

use crate::{camera::Mode, menu::Frontend, options::Options, Session};
use bevy::{
    audio::{ChannelCount, Decodable, SampleRate, Source},
    prelude::*,
};
use dsp::{pan, saturate, smoothing, smoothstep, Biquad, Noise, OnePole, Oscillator, RATE};
use engine::{Control, Kind, Voice};
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

/// Rival engines mixed at once: the nearest cars to the listener.
const RIVALS: usize = 3;
/// Beyond this a rival is inaudible over the player's car.
const AUDIBLE_METRES: f64 = 300.;
const SPEED_OF_SOUND: f64 = 343.;
const CONTROL_FRAMES: u32 = 64;

/// Saved audio preferences.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub engine: EngineChoice,
    pub master: f32,
    pub engine_volume: f32,
    pub effects: f32,
    pub opponents: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: EngineChoice::MatchCar,
            master: 0.8,
            engine_volume: 1.,
            effects: 0.8,
            opponents: 0.7,
        }
    }
}

impl Settings {
    pub fn sanitize(&mut self) {
        let defaults = Self::default();
        for (value, default) in [
            (&mut self.master, defaults.master),
            (&mut self.engine_volume, defaults.engine_volume),
            (&mut self.effects, defaults.effects),
            (&mut self.opponents, defaults.opponents),
        ] {
            *value = if value.is_finite() {
                value.clamp(0., 1.)
            } else {
                default
            };
        }
    }
}

#[derive(Default)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// One car's engine and its placement relative to the listener.
#[derive(Default)]
struct Feed {
    /// Entrant index + 1; 0 means no car.
    id: AtomicU32,
    revs: AtomicF32,
    throttle: AtomicF32,
    brake: AtomicF32,
    gear: AtomicI32,
    limiter: AtomicBool,
    gain: AtomicF32,
    pan: AtomicF32,
    doppler: AtomicF32,
    distance: AtomicF32,
}

#[derive(Default)]
struct Shared {
    active: AtomicBool,
    kind: AtomicU32,
    master: AtomicF32,
    engine: AtomicF32,
    effects: AtomicF32,
    opponents: AtomicF32,
    onboard: AtomicF32,
    speed: AtomicF32,
    slide: AtomicF32,
    lock: AtomicF32,
    kerb: AtomicF32,
    offroad: AtomicF32,
    wheel_rps: AtomicF32,
    impact: AtomicF32,
    impacts: AtomicU32,
    player: Feed,
    rivals: [Feed; RIVALS],
}

#[derive(Resource)]
pub(crate) struct GameAudio {
    shared: Arc<Shared>,
    slots: [Option<usize>; RIVALS],
    listener: Option<DVec3>,
    impact_cooldown: f32,
}

/// The single continuous stereo stream the game plays.
#[derive(Asset, TypePath)]
pub(crate) struct Soundscape {
    shared: Arc<Shared>,
}

impl Decodable for Soundscape {
    type Decoder = Mixer;
    fn decoder(&self) -> Mixer {
        Mixer::new(self.shared.clone())
    }
}

/// Distance filtering and panning for one voice.
struct Space {
    gain: f32,
    pan: f32,
    air: [OnePole; 2],
}

impl Space {
    fn new() -> Self {
        Self {
            gain: 0.,
            pan: 0.,
            air: [OnePole::new(14000.); 2],
        }
    }
    fn tune(&mut self, distance: f32) {
        // Air and the car body absorb highs with distance.
        let cutoff = 15000. / (1. + distance / 35.);
        for filter in &mut self.air {
            filter.tune(cutoff);
        }
    }
    fn place(&mut self, sample: f32, gain: f32, target_pan: f32) -> (f32, f32) {
        self.gain += (gain - self.gain) * smoothing(0.04);
        self.pan += (target_pan - self.pan) * smoothing(0.06);
        let mut s = sample;
        for filter in &mut self.air {
            s = filter.process(s);
        }
        let (l, r) = pan(self.pan);
        (s * self.gain * l, s * self.gain * r)
    }
}

struct Rival {
    voice: Voice,
    id: u32,
    space: Space,
}

/// Wind, tyres, kerbs, drivetrain and impacts around the player.
struct Ambience {
    noise: [Noise; 2],
    wind: [[OnePole; 2]; 2],
    wind_floor: [Biquad; 2],
    gust: OnePole,
    squeal: [Biquad; 2],
    scrub: Biquad,
    kerb: Oscillator,
    kerb_tone: OnePole,
    offroad: OnePole,
    whine: Oscillator,
    thump: OnePole,
    thump_body: Oscillator,
    impact: f32,
    impacts: u32,
}

impl Ambience {
    fn new() -> Self {
        Self {
            noise: [Noise::new(0xA11CE), Noise::new(0xB0B)],
            wind: [[OnePole::default(); 2]; 2],
            wind_floor: [Biquad::highpass(90., 0.7); 2],
            gust: OnePole::new(0.7),
            squeal: [Biquad::bandpass(1050., 9.), Biquad::bandpass(1620., 11.)],
            scrub: Biquad::bandpass(480., 0.8),
            kerb: Oscillator::default(),
            kerb_tone: OnePole::new(260.),
            offroad: OnePole::new(380.),
            whine: Oscillator::default(),
            thump: OnePole::new(160.),
            thump_body: Oscillator::default(),
            impact: 0.,
            impacts: 0,
        }
    }
}

struct Smoothed {
    active: f32,
    master: f32,
    engine: f32,
    effects: f32,
    opponents: f32,
    onboard: f32,
    speed: f32,
    slide: f32,
    lock: f32,
    kerb: f32,
    offroad: f32,
    wheel_rps: f32,
}

/// Snapshot of the shared state taken on each control tick.
#[derive(Default)]
struct Targets {
    active: f32,
    master: f32,
    engine: f32,
    effects: f32,
    opponents: f32,
    onboard: f32,
    speed: f32,
    slide: f32,
    lock: f32,
    kerb: f32,
    offroad: f32,
    wheel_rps: f32,
    player_gain: f32,
    player_pan: f32,
    rival_gain: [f32; RIVALS],
    rival_pan: [f32; RIVALS],
}

/// Gentle mastering: tame the ear's most sensitive band, compress, then limit.
struct Bus {
    presence: [Biquad; 2],
    air: [Biquad; 2],
    envelope: f32,
}

impl Bus {
    fn new() -> Self {
        Self {
            presence: [Biquad::peaking(3300., 0.9, -3.); 2],
            air: [Biquad::high_shelf(9000., -2.5); 2],
            envelope: 0.,
        }
    }
    fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let left = self.air[0].process(self.presence[0].process(left));
        let right = self.air[1].process(self.presence[1].process(right));
        let level = left.abs().max(right.abs());
        let rate = if level > self.envelope {
            smoothing(0.006)
        } else {
            smoothing(0.18)
        };
        self.envelope += (level - self.envelope) * rate;
        const THRESHOLD: f32 = 0.32;
        let reduction = if self.envelope > THRESHOLD {
            (THRESHOLD / self.envelope).powf(1. - 1. / 3.)
        } else {
            1.
        };
        let gain = reduction * 1.3;
        (saturate(left * gain) * 0.92, saturate(right * gain) * 0.92)
    }
}

pub(crate) struct Mixer {
    shared: Arc<Shared>,
    player: Voice,
    player_space: Space,
    rivals: [Rival; RIVALS],
    ambience: Ambience,
    bus: Bus,
    smoothed: Smoothed,
    targets: Targets,
    kind: Kind,
    /// Fades the engines out and back in around an engine-type change.
    switch: f32,
    frames: u32,
    right: Option<f32>,
}

impl Mixer {
    fn new(shared: Arc<Shared>) -> Self {
        let kind = Kind::from_index(shared.kind.load(Ordering::Relaxed));
        Self {
            player: Voice::new(kind, 0x51DE),
            player_space: Space::new(),
            rivals: std::array::from_fn(|index| Rival {
                voice: Voice::new(kind, 0xC0FFEE + index as u32 * 977),
                id: 0,
                space: Space::new(),
            }),
            ambience: Ambience::new(),
            bus: Bus::new(),
            smoothed: Smoothed {
                active: 0.,
                master: 0.,
                engine: 0.,
                effects: 0.,
                opponents: 0.,
                onboard: 0.,
                speed: 0.,
                slide: 0.,
                lock: 0.,
                kerb: 0.,
                offroad: 0.,
                wheel_rps: 0.,
            },
            targets: Targets::default(),
            kind,
            switch: 1.,
            frames: 0,
            right: None,
            shared,
        }
    }

    fn control(feed: &Feed, onboard: f32) -> Control {
        Control {
            revs: feed.revs.get(),
            throttle: feed.throttle.get().clamp(0., 1.),
            brake: feed.brake.get().clamp(0., 1.),
            gear: feed.gear.load(Ordering::Relaxed),
            limiter: feed.limiter.load(Ordering::Relaxed),
            onboard,
            doppler: feed.doppler.get().clamp(0.75, 1.3),
        }
    }

    fn read(&mut self) {
        let s = &self.shared;
        let t = &mut self.targets;
        t.active = if s.active.load(Ordering::Relaxed) {
            1.
        } else {
            0.
        };
        t.master = s.master.get();
        t.engine = s.engine.get();
        t.effects = s.effects.get();
        t.opponents = s.opponents.get();
        t.onboard = s.onboard.get();
        t.speed = s.speed.get();
        t.slide = s.slide.get();
        t.lock = s.lock.get();
        t.kerb = s.kerb.get();
        t.offroad = s.offroad.get();
        t.wheel_rps = s.wheel_rps.get();
        t.player_gain = s.player.gain.get();
        t.player_pan = s.player.pan.get();
        self.player
            .update_control(Self::control(&s.player, t.onboard));
        self.player_space.tune(s.player.distance.get());

        let wanted = Kind::from_index(s.kind.load(Ordering::Relaxed));
        if wanted != self.kind && self.switch <= 0. {
            self.kind = wanted;
            self.player.configure(wanted);
            for rival in &mut self.rivals {
                rival.voice.configure(wanted);
            }
        }

        for (index, rival) in self.rivals.iter_mut().enumerate() {
            let feed = &s.rivals[index];
            let id = feed.id.load(Ordering::Relaxed);
            if id != rival.id {
                // Fade the old car out before this voice takes over the new one.
                t.rival_gain[index] = 0.;
                if rival.space.gain < 1e-3 {
                    rival.id = id;
                }
                continue;
            }
            t.rival_gain[index] = if id == 0 { 0. } else { feed.gain.get() };
            t.rival_pan[index] = feed.pan.get();
            rival.voice.update_control(Self::control(feed, 0.));
            rival.space.tune(feed.distance.get());
        }

        let impacts = s.impacts.load(Ordering::Relaxed);
        if impacts != self.ambience.impacts {
            self.ambience.impacts = impacts;
            self.ambience.impact = self.ambience.impact.max(s.impact.get());
        }
        // Tyre squeal pitch rises a little with speed and lock-ups.
        let pitch = 1. + 0.15 * (t.speed / 80.).min(1.) + 0.1 * t.lock;
        self.ambience.squeal[0].retune(Biquad::bandpass(1050. * pitch, 9.));
        self.ambience.squeal[1].retune(Biquad::bandpass(1620. * pitch, 11.));
        let speed = (t.speed / 90.).min(1.4);
        for side in &mut self.ambience.wind {
            side[0].tune(250. + 2600. * speed);
            side[1].tune(400. + 3200. * speed);
        }
    }

    fn frame(&mut self) -> (f32, f32) {
        if self.frames.is_multiple_of(CONTROL_FRAMES) {
            self.read();
        }
        self.frames = self.frames.wrapping_add(1);
        let t = &self.targets;
        let m = &mut self.smoothed;
        let fast = smoothing(0.04);
        let slow = smoothing(0.12);
        m.active += (t.active - m.active) * smoothing(0.05);
        m.master += (t.master - m.master) * fast;
        m.engine += (t.engine - m.engine) * fast;
        m.effects += (t.effects - m.effects) * fast;
        m.opponents += (t.opponents - m.opponents) * fast;
        m.onboard += (t.onboard - m.onboard) * slow;
        m.speed += (t.speed - m.speed) * slow;
        m.slide += (t.slide - m.slide) * smoothing(0.05);
        m.lock += (t.lock - m.lock) * smoothing(0.05);
        m.kerb += (t.kerb - m.kerb) * smoothing(0.03);
        m.offroad += (t.offroad - m.offroad) * smoothing(0.05);
        m.wheel_rps += (t.wheel_rps - m.wheel_rps) * slow;
        let wanted = Kind::from_index(self.shared.kind.load(Ordering::Relaxed));
        let target = if wanted == self.kind { 1. } else { 0. };
        let step = 1. / (0.03 * RATE);
        self.switch += (target - self.switch).clamp(-step, step);

        let player = self.player.sample();
        let (mut left, mut right) = self.player_space.place(
            player * self.smoothed.engine * self.switch,
            t.player_gain,
            t.player_pan,
        );

        let mut rivals = (0., 0.);
        for (index, rival) in self.rivals.iter_mut().enumerate() {
            let sample = rival.voice.sample() * self.switch;
            let (l, r) = rival
                .space
                .place(sample, t.rival_gain[index], t.rival_pan[index]);
            rivals.0 += l;
            rivals.1 += r;
        }
        left += rivals.0 * self.smoothed.opponents * 0.8;
        right += rivals.1 * self.smoothed.opponents * 0.8;

        let (l, r) = self.ambience_frame();
        left += l * self.smoothed.effects;
        right += r * self.smoothed.effects;

        let (left, right) = self.bus.process(left, right);
        let gain = self.smoothed.active * self.smoothed.master;
        (left * gain, right * gain)
    }

    fn ambience_frame(&mut self) -> (f32, f32) {
        let m = &self.smoothed;
        let a = &mut self.ambience;
        let speed = (m.speed / 90.).min(1.4);
        // Wind: decorrelated left/right noise, buffeting more in the cockpit.
        let gust =
            1. + (0.15 + 0.3 * m.onboard) * a.gust.process(a.noise[0].next() * 6.).clamp(-1., 1.);
        let wind_level = 0.09 * speed * speed * (0.55 + 0.6 * m.onboard) * gust;
        let mut wind = [0f32; 2];
        for (side, value) in wind.iter_mut().enumerate() {
            let n = a.noise[side].next();
            let low = a.wind[side][0].process(n);
            let high = a.wind[side][1].process(n) - low;
            *value = a.wind_floor[side].process(low + 0.35 * high) * wind_level;
        }
        // Tyres: a narrow-band squeal when sliding, a broad scrub underneath.
        let rolling = smoothstep(2., 14., m.speed);
        let n = a.noise[1].next();
        let slide = m.slide.max(m.lock);
        let squeal = (a.squeal[0].process(n) + 0.6 * a.squeal[1].process(n))
            * 0.55
            * slide.powf(1.5)
            * rolling;
        let scrub = a.scrub.process(a.noise[0].next()) * 0.12 * slide * rolling;
        // Kerbs drum at the stripe rate; grass and gravel rumble.
        let stripe = a.kerb.process((m.speed / 0.9).clamp(1., 120.));
        let drum = a.kerb_tone.process(stripe.signum()) * 0.28 * m.kerb * rolling;
        let rumble = a.offroad.process(a.noise[1].next()) * 0.5 * m.offroad * rolling;
        // Straight-cut gear whine, clearest in the cockpit.
        let whine = a.whine.process(m.wheel_rps * 4.3 * 21.)
            * 0.006
            * (0.35 + m.onboard)
            * smoothstep(4., 40., m.speed);
        // Impacts: a short low thump.
        let thump = if a.impact > 1e-3 {
            let body = a.thump.process(a.noise[0].next()) * 2.5 + a.thump_body.process(52.) * 0.5;
            let value = body * a.impact * 0.6;
            a.impact *= 1. - 1. / (0.16 * RATE);
            value
        } else {
            0.
        };
        let center = squeal + scrub + drum + rumble + whine + thump;
        (center + wind[0], center + wind[1])
    }
}

impl Iterator for Mixer {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(right) = self.right.take() {
            return Some(right);
        }
        let (left, right) = self.frame();
        self.right = Some(right);
        Some(left)
    }
}

impl Source for Mixer {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(RATE as u32).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub fn setup(mut commands: Commands, mut sources: ResMut<Assets<Soundscape>>) {
    let shared = Arc::<Shared>::default();
    commands.spawn(AudioPlayer(sources.add(Soundscape {
        shared: shared.clone(),
    })));
    commands.insert_resource(GameAudio {
        shared,
        slots: [None; RIVALS],
        listener: None,
        impact_cooldown: 0.,
    });
}

/// Placement of a car relative to the listener.
struct Placement {
    distance: f64,
    pan: f32,
    doppler: f32,
}

fn place(
    car: &racecraft_sim::Car,
    eye: DVec3,
    right: DVec3,
    listener_velocity: DVec3,
) -> Placement {
    let offset = car.position + DVec3::Y * 0.5 - eye;
    let distance = offset.length();
    let direction = offset / distance.max(1e-3);
    let velocity = DVec3::new(car.velocity.x, 0., car.velocity.y);
    // Positive when the source moves away from the listener.
    let receding = (velocity - listener_velocity).dot(direction);
    Placement {
        distance,
        pan: (direction.dot(right) * distance / (distance + 3.)) as f32,
        doppler: (SPEED_OF_SOUND / (SPEED_OF_SOUND + receding)).clamp(0.75, 1.3) as f32,
    }
}

fn attenuation(distance: f64) -> f32 {
    let excess = (distance - 6.).max(0.);
    (12. / (12. + excess)) as f32 * (1. - smoothstep(0.8, 1., (distance / AUDIBLE_METRES) as f32))
}

fn revs(car: &racecraft_sim::Car) -> f32 {
    let engine = &car.spec.engine;
    ((car.powertrain.rpm - engine.idle_rpm) / (engine.rev_limit - engine.idle_rpm)) as f32
}

fn publish(feed: &Feed, id: u32, car: &racecraft_sim::Car, throttle: f32, brake: f32) {
    feed.id.store(id, Ordering::Relaxed);
    feed.revs.set(revs(car));
    feed.throttle.set(throttle);
    feed.brake.set(brake);
    feed.gear.store(car.powertrain.gear, Ordering::Relaxed);
    feed.limiter
        .store(car.powertrain.limiter, Ordering::Relaxed);
}

pub fn update(
    session: Res<Session>,
    keys: Res<ButtonInput<KeyCode>>,
    options: Res<Options>,
    time: Res<Time>,
    mut frontend: ResMut<Frontend>,
    mut audio: ResMut<GameAudio>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        frontend.muted = !frontend.muted;
    }
    let settings = frontend.audio;
    let shared = audio.shared.clone();
    let s = &*shared;
    s.active.store(
        session.ready && !session.paused && !frontend.muted,
        Ordering::Relaxed,
    );
    s.kind.store(
        settings.engine.resolve(&options.car) as u32,
        Ordering::Relaxed,
    );
    // Perceptual volume curve.
    s.master.set(settings.master * settings.master);
    s.engine
        .set(settings.engine_volume * settings.engine_volume);
    s.effects.set(settings.effects * settings.effects);
    s.opponents.set(settings.opponents * settings.opponents);

    let race = &session.race;
    let car = &race.entrants[session.player].car;
    let (eye, look) = session.camera.listener();
    let dt = time.delta_secs() as f64;
    let listener_velocity = match audio.listener {
        // A camera cut is not motion.
        Some(previous) if dt > 0. && eye.distance(previous) < 20. => (eye - previous) / dt,
        _ => DVec3::ZERO,
    };
    audio.listener = Some(eye);
    let mut forward = look - eye;
    forward.y = 0.;
    let right = forward.normalize_or(DVec3::X).cross(DVec3::Y);
    let onboard = session.camera.mode == Mode::Cockpit;
    s.onboard.set(if onboard { 1. } else { 0. });

    let input = session.guidance.activity.input;
    publish(
        &s.player,
        session.player as u32 + 1,
        car,
        input.throttle as f32,
        input.brake as f32,
    );
    let placement = place(car, eye, right, listener_velocity);
    let player = &s.player;
    if onboard {
        player.gain.set(1.);
        player.pan.set(0.);
        player.doppler.set(1.);
        player.distance.set(0.);
    } else {
        player.gain.set(attenuation(placement.distance).max(0.35));
        player.pan.set(placement.pan * 0.6);
        player.doppler.set(placement.doppler);
        player.distance.set(placement.distance as f32);
    }

    let speed = car.speed() as f32;
    s.speed.set(speed);
    let slide = car.wheels.iter().map(|w| w.sliding).fold(0., f64::max) as f32;
    let lock = if car.wheels.iter().any(|w| w.locked) && speed > 3. {
        1.
    } else {
        0.
    };
    s.slide.set(slide);
    s.lock.set(lock);
    use racecraft_sim::track::Surface;
    let share = |surfaces: &[Surface]| {
        car.wheels
            .iter()
            .filter(|w| surfaces.contains(&w.surface))
            .count() as f32
            / 4.
    };
    s.kerb.set(share(&[Surface::Kerb]));
    s.offroad.set(share(&[Surface::Grass, Surface::Gravel]));
    let radius = car.spec.wheels[2].radius;
    s.wheel_rps
        .set((car.forward_speed().abs() / (radius * std::f64::consts::TAU)) as f32);
    audio.impact_cooldown = (audio.impact_cooldown - dt as f32).max(0.);
    if car.impact > 300. && audio.impact_cooldown == 0. && !session.paused {
        s.impact.set((car.impact / 4000.).min(1.) as f32);
        s.impacts.fetch_add(1, Ordering::Relaxed);
        audio.impact_cooldown = 0.15;
    }

    // Keep each nearby rival in a stable voice slot so its engine never jumps.
    let mut nearest: Vec<(usize, Placement)> = race
        .entrants
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != session.player)
        .map(|(index, entrant)| (index, place(&entrant.car, eye, right, listener_velocity)))
        .filter(|(_, placement)| placement.distance < AUDIBLE_METRES)
        .collect();
    nearest.sort_by(|a, b| a.1.distance.total_cmp(&b.1.distance));
    nearest.truncate(RIVALS);
    for slot in &mut audio.slots {
        if slot.is_some_and(|index| !nearest.iter().any(|(n, _)| *n == index)) {
            *slot = None;
        }
    }
    for (index, _) in &nearest {
        if !audio.slots.contains(&Some(*index)) {
            if let Some(free) = audio.slots.iter_mut().find(|slot| slot.is_none()) {
                *free = Some(*index);
            }
        }
    }
    for (slot, feed) in audio.slots.iter().zip(&s.rivals) {
        let Some((index, placement)) = slot.and_then(|i| nearest.iter().find(|(n, _)| *n == i))
        else {
            feed.id.store(0, Ordering::Relaxed);
            feed.gain.set(0.);
            continue;
        };
        let rival = &race.entrants[*index].car;
        // AI inputs are not stored; infer them from the car's acceleration.
        let acceleration = rival.acceleration.x as f32;
        let throttle = if rival.powertrain.limiter {
            1.
        } else {
            smoothstep(-7., -2., acceleration)
        };
        let brake = smoothstep(-12., -30., acceleration);
        publish(feed, *index as u32 + 1, rival, throttle, brake);
        feed.gain.set(attenuation(placement.distance));
        feed.pan.set(placement.pan);
        feed.doppler.set(placement.doppler);
        feed.distance.set(placement.distance as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::audio::{AddAudioSource, AudioSink, AudioSinkPlayback};

    fn mixer(kind: Kind) -> Mixer {
        let shared = Arc::<Shared>::default();
        shared.active.store(true, Ordering::Relaxed);
        shared.kind.store(kind as u32, Ordering::Relaxed);
        let defaults = Settings::default();
        shared.master.set(defaults.master * defaults.master);
        shared.engine.set(1.);
        shared.effects.set(defaults.effects * defaults.effects);
        shared
            .opponents
            .set(defaults.opponents * defaults.opponents);
        shared.player.gain.set(1.);
        shared.player.doppler.set(1.);
        Mixer::new(shared)
    }

    #[test]
    fn rapid_revs_shifts_slides_and_mute_stay_continuous_and_unclipped() {
        for kind in [Kind::V6Hybrid, Kind::V8, Kind::V12] {
            let mut mixer = mixer(kind);
            let s = mixer.shared.clone();
            let (mut previous, mut peak, mut jump) = (0f32, 0f32, 0f32);
            for n in 0..RATE as u32 * 4 {
                if n % (RATE as u32 / 2) == 0 {
                    let high = (n / (RATE as u32 / 2)).is_multiple_of(2);
                    s.player.revs.set(if high { 0.98 } else { 0.1 });
                    s.player.throttle.set(if high { 1. } else { 0. });
                    s.player
                        .gear
                        .store(if high { 2 } else { 3 }, Ordering::Relaxed);
                    s.player.limiter.store(high, Ordering::Relaxed);
                    s.speed.set(if high { 80. } else { 20. });
                    s.slide.set(if high { 0. } else { 1. });
                    s.kerb.set(if high { 1. } else { 0. });
                }
                let (left, right) = mixer.frame();
                assert!(left.is_finite() && right.is_finite());
                peak = peak.max(left.abs()).max(right.abs());
                jump = jump.max((left - previous).abs());
                previous = left;
            }
            assert!(peak > 0.1 && peak < 0.95, "{kind:?} peak {peak}");
            assert!(jump < 0.5, "{kind:?} discontinuity {jump}");
            s.active.store(false, Ordering::Relaxed);
            for _ in 0..RATE as usize / 2 {
                mixer.frame();
            }
            let (left, right) = mixer.frame();
            assert!(left.abs() < 1e-4 && right.abs() < 1e-4);
        }
    }

    #[test]
    fn full_grid_effects_and_impacts_keep_headroom_at_maximum_volume() {
        for kind in [Kind::V6Hybrid, Kind::V8, Kind::V12] {
            let mut mixer = mixer(kind);
            let s = mixer.shared.clone();
            for level in [&s.master, &s.engine, &s.effects, &s.opponents] {
                level.set(1.);
            }
            s.player.revs.set(1.);
            s.player.throttle.set(1.);
            s.speed.set(100.);
            s.slide.set(1.);
            s.lock.set(1.);
            s.kerb.set(1.);
            s.offroad.set(1.);
            s.wheel_rps.set(45.);
            for (index, feed) in s.rivals.iter().enumerate() {
                feed.id.store(index as u32 + 2, Ordering::Relaxed);
                feed.revs.set(0.85 + index as f32 * 0.05);
                feed.throttle.set(1.);
                feed.gain.set(1.);
                feed.doppler.set(1.);
                feed.pan.set(index as f32 - 1.);
            }
            let mut peak = 0f32;
            for n in 0..RATE as u32 * 2 {
                if n.is_multiple_of(RATE as u32 / 4) {
                    s.impact.set(1.);
                    s.impacts.fetch_add(1, Ordering::Relaxed);
                }
                let (left, right) = mixer.frame();
                assert!(left.is_finite() && right.is_finite());
                peak = peak.max(left.abs()).max(right.abs());
            }
            assert!(peak > 0.2 && peak <= 0.92, "{kind:?} full mix peak {peak}");
            s.master.set(0.);
            for _ in 0..RATE as usize {
                mixer.frame();
            }
            let (left, right) = mixer.frame();
            assert!(left.abs() < 1e-5 && right.abs() < 1e-5);
        }
    }

    #[test]
    fn changing_engine_type_fades_through_silence_and_reconfigures_every_voice() {
        let mut mixer = mixer(Kind::V8);
        let s = mixer.shared.clone();
        s.player.revs.set(0.6);
        s.player.throttle.set(1.);
        for _ in 0..RATE as usize / 4 {
            mixer.frame();
        }
        s.kind.store(Kind::V12 as u32, Ordering::Relaxed);
        let mut quietest = f32::MAX;
        for _ in 0..RATE as usize / 4 {
            mixer.frame();
            quietest = quietest.min(mixer.switch);
        }
        assert_eq!(quietest, 0.);
        assert_eq!(mixer.switch, 1.);
        assert_eq!(mixer.player.kind(), Kind::V12);
        assert!(mixer.rivals.iter().all(|r| r.voice.kind() == Kind::V12));
    }

    #[test]
    fn rivals_are_placed_by_distance_and_side() {
        let near = attenuation(10.);
        let far = attenuation(150.);
        assert!(near > 0.7 && far < 0.15 && far > 0.);
        assert_eq!(attenuation(AUDIBLE_METRES + 1.), 0.);
        let session = crate::tests::session();
        let car = &session.race.entrants[0].car;
        let eye = car.position + DVec3::new(-20., 0.5, 0.);
        let placement = place(car, eye, DVec3::X, DVec3::ZERO);
        assert!(placement.pan > 0.8, "car to the right pans right");
        let mut moving = car.clone();
        moving.velocity = glam::DVec2::new(-60., 0.);
        assert!(
            place(&moving, eye, DVec3::X, DVec3::ZERO).doppler > 1.1,
            "approaching car is higher"
        );
    }

    #[test]
    fn engine_choice_matches_car_and_settings_recover_from_bad_values() {
        assert_eq!(EngineChoice::MatchCar.resolve("rb6"), Kind::V8);
        assert_eq!(EngineChoice::MatchCar.resolve("rb22"), Kind::V6Hybrid);
        assert_eq!(EngineChoice::V12.resolve("rb22"), Kind::V12);
        let mut settings = Settings {
            master: f32::NAN,
            effects: 3.,
            ..default()
        };
        settings.sanitize();
        assert_eq!(settings.master, Settings::default().master);
        assert_eq!(settings.effects, 1.);
        let restored: Settings = serde_json::from_str(r#"{"engine":"v12"}"#).unwrap();
        assert_eq!(restored.engine, EngineChoice::V12);
        assert_eq!(restored.master, Settings::default().master);
    }

    /// Writes a stereo listening demo for each engine:
    /// `cargo test -p f1-the-game export_engine_demos -- --ignored --nocapture`
    #[test]
    #[ignore = "writes listening samples"]
    fn export_engine_demos() {
        for (kind, name) in [
            (Kind::V6Hybrid, "v6-hybrid"),
            (Kind::V8, "v8"),
            (Kind::V12, "v12"),
        ] {
            let mut mixer = mixer(kind);
            let s = mixer.shared.clone();
            let mut samples = Vec::new();
            let mut gear = 1;
            let mut revs = 0.05f32;
            for n in 0..RATE as usize * 16 {
                let t = n as f32 / RATE;
                let (throttle, brake) = match t {
                    t if t < 1.5 => (0., 0.),
                    t if t < 2.5 => (((t - 1.5) * 4.).min(0.6) * (1. - (t - 2.) * 2.).max(0.), 0.),
                    t if t < 10.5 => (1., 0.),
                    t if t < 13.5 => (0., 1.),
                    _ => (0.15, 0.),
                };
                if n % CONTROL_FRAMES as usize == 0 {
                    let dt = CONTROL_FRAMES as f32 / RATE;
                    if t < 2.5 {
                        revs = 0.02 + throttle * 0.5;
                    } else if throttle > 0.5 {
                        revs += dt * 0.55 / gear as f32;
                        if revs > 0.985 && gear < 7 {
                            gear += 1;
                            revs -= 0.26;
                        }
                    } else {
                        revs -= dt * 0.45;
                        if revs < 0.45 && gear > 2 {
                            gear -= 1;
                            revs += 0.22;
                        }
                        revs = revs.max(0.08);
                    }
                    s.player.revs.set(revs);
                    s.player.throttle.set(throttle);
                    s.player.brake.set(brake);
                    s.player.gear.store(gear, Ordering::Relaxed);
                    s.speed.set(10. + gear as f32 * 10. * revs);
                    s.wheel_rps.set((10. + gear as f32 * 10. * revs) / 2.2);
                }
                let (l, r) = mixer.frame();
                samples.push((l * 32767.) as i16);
                samples.push((r * 32767.) as i16);
            }
            let path = std::env::temp_dir().join(format!("f1-engine-{name}.wav"));
            std::fs::write(&path, wav(&samples)).unwrap();
            println!("Listening sample: {}", path.display());
        }
    }

    fn wav(samples: &[i16]) -> Vec<u8> {
        let size = samples.len() as u32 * 2;
        let rate = RATE as u32;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * 4).to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for sample in samples {
            bytes.extend(sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    #[ignore = "requires an audio output device"]
    fn live_engine_playback() {
        let mut app = App::new();
        let options = Options::parse(std::iter::empty()).unwrap();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::audio::AudioPlugin::default(),
        ))
        .add_audio_source::<Soundscape>()
        .insert_resource(crate::tests::session())
        .insert_resource(Frontend::new(&options))
        .insert_resource(options)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_systems(Startup, setup)
        .add_systems(Update, update);
        app.finish();
        app.cleanup();
        app.world_mut().resource_mut::<Session>().ready = true;
        for _ in 0..10 {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        let world = app.world_mut();
        assert!(!world
            .query::<&AudioSink>()
            .single(world)
            .unwrap()
            .is_paused());
        let active = |app: &App| {
            app.world()
                .resource::<GameAudio>()
                .shared
                .active
                .load(Ordering::Relaxed)
        };
        assert!(active(&app));
        app.world_mut().resource_mut::<Session>().paused = true;
        app.update();
        assert!(!active(&app));
        app.world_mut().resource_mut::<Session>().paused = false;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        assert!(!active(&app));
    }
}
