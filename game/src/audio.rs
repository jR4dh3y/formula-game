//! Continuous sample playback from the user's F1 2013 reference video.
//! See game/audio/reference/CREDITS.md for source and excerpt details.

use crate::{options::Options, Session};
use bevy::{
    audio::{ChannelCount, Decodable, SampleRate, Source},
    prelude::*,
};
use std::{
    f32::consts::{PI, TAU},
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

const RATE: u32 = 48000;
const ON: &[u8] = include_bytes!("../audio/reference/engine_on.wav");
const OFF: &[u8] = include_bytes!("../audio/reference/engine_off.wav");
const REFERENCE_RPM: f64 = 18000.;

struct Targets {
    rpm: AtomicU32,
    throttle: AtomicU32,
    gear: AtomicI32,
    active: AtomicBool,
    limiter: AtomicBool,
}

impl Default for Targets {
    fn default() -> Self {
        Self {
            rpm: AtomicU32::new(4500f32.to_bits()),
            throttle: AtomicU32::new(0f32.to_bits()),
            gear: AtomicI32::new(1),
            active: AtomicBool::new(false),
            limiter: AtomicBool::new(false),
        }
    }
}

#[derive(Resource)]
pub(crate) struct EngineAudio {
    targets: Arc<Targets>,
    muted: bool,
}

#[derive(Asset, TypePath)]
pub(crate) struct EngineSound {
    targets: Arc<Targets>,
    recording: Arc<[f32]>,
    coast: Arc<[f32]>,
}

impl Decodable for EngineSound {
    type Decoder = EngineVoice;
    fn decoder(&self) -> EngineVoice {
        EngineVoice::new(
            self.targets.clone(),
            self.recording.clone(),
            self.coast.clone(),
        )
    }
}

pub(crate) struct EngineVoice {
    targets: Arc<Targets>,
    recording: Arc<[f32]>,
    coast: Arc<[f32]>,
    position: f64,
    rpm: f32,
    throttle: f32,
    gain: f32,
    limiter_phase: f32,
    limiter_mix: f32,
    shift: f32,
    downshift: bool,
    gear: i32,
    frames: u32,
    target_rpm: f32,
    target_throttle: f32,
    target_active: bool,
    target_limiter: bool,
}

impl EngineVoice {
    fn new(targets: Arc<Targets>, recording: Arc<[f32]>, coast: Arc<[f32]>) -> Self {
        let rpm = f32::from_bits(targets.rpm.load(Ordering::Relaxed));
        let gear = targets.gear.load(Ordering::Relaxed);
        Self {
            targets,
            recording,
            coast,
            position: 0.,
            rpm,
            throttle: 0.,
            gain: 0.,
            limiter_phase: 0.,
            limiter_mix: 0.,
            shift: 0.,
            downshift: false,
            gear,
            frames: 0,
            target_rpm: rpm,
            target_throttle: 0.,
            target_active: false,
            target_limiter: false,
        }
    }

    fn sample(&mut self) -> f32 {
        // One atomic snapshot every 1.3 ms; no locks or allocations on the audio thread.
        if self.frames.is_multiple_of(64) {
            self.target_rpm =
                f32::from_bits(self.targets.rpm.load(Ordering::Relaxed)).clamp(2500., 20000.);
            self.target_throttle =
                f32::from_bits(self.targets.throttle.load(Ordering::Relaxed)).clamp(0., 1.);
            self.target_active = self.targets.active.load(Ordering::Relaxed);
            self.target_limiter = self.targets.limiter.load(Ordering::Relaxed);
            let gear = self.targets.gear.load(Ordering::Relaxed);
            if gear != self.gear && self.target_active {
                self.downshift = gear < self.gear;
                self.shift = if self.downshift { 0.11 } else { 0.075 };
            }
            self.gear = gear;
        }
        self.frames = self.frames.wrapping_add(1);
        self.shift = (self.shift - 1. / RATE as f32).max(0.);
        let shift_envelope = (self.shift / if self.downshift { 0.11 } else { 0.075 }).clamp(0., 1.);
        let blip = if self.downshift {
            (PI * shift_envelope).sin()
        } else {
            0.
        };
        // The simulation already models flywheel inertia. Preserve its fast shift
        // response while removing frame-sized steps from the resampling ratio.
        let pitch_slew = if self.shift > 0. { 0.003 } else { 0.006 };
        self.rpm += (self.target_rpm + 300. * blip - self.rpm) * (1. / (pitch_slew * RATE as f32));
        self.throttle += ((self.target_throttle + 0.25 * blip).min(1.) - self.throttle)
            * (1. / (0.030 * RATE as f32));
        let cut = if self.downshift {
            1.
        } else {
            0.22 + 0.78 * (1. - shift_envelope / 0.6).clamp(0., 1.)
        };
        let target_gain = if self.target_active {
            (0.48 + self.throttle * 0.47) * cut
        } else {
            0.
        };
        self.gain += (target_gain - self.gain) * (1. / (0.008 * RATE as f32));

        // Both real recordings are pitch-normalized and cycle-aligned offline.
        // One read head advances continuously; no loop restarts or oscillator reconstruction.
        self.position += self.rpm as f64 / REFERENCE_RPM;
        let loaded = read_recording(&self.recording, self.position);
        let coast = read_recording(&self.coast, self.position);
        let signal = coast + (loaded - coast) * self.throttle;
        self.limiter_phase = (self.limiter_phase + 42. / RATE as f32).fract();
        self.limiter_mix +=
            (f32::from(self.target_limiter) - self.limiter_mix) / (0.01 * RATE as f32);
        let limiter = 1. - self.limiter_mix * 0.08 * (1. - (self.limiter_phase * TAU).sin());
        signal * self.gain * limiter
    }
}

impl Iterator for EngineVoice {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        Some(self.sample())
    }
}

impl Source for EngineVoice {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(1).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(RATE).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn read_recording(recording: &[f32], position: f64) -> f32 {
    let position = position % recording.len() as f64;
    let index = position as usize;
    let fraction = position.fract() as f32;
    // Cubic interpolation keeps the recorded high-frequency detail as speed changes.
    let p0 = recording[(index + recording.len() - 1) % recording.len()];
    let p1 = recording[index];
    let p2 = recording[(index + 1) % recording.len()];
    let p3 = recording[(index + 2) % recording.len()];
    p1 + 0.5
        * fraction
        * (p2 - p0
            + fraction * (2. * p0 - 5. * p1 + 4. * p2 - p3 + fraction * (3. * (p1 - p2) + p3 - p0)))
}

fn recording(bytes: &[u8]) -> Arc<[f32]> {
    AudioSource {
        bytes: bytes.into(),
    }
    .decoder()
    .collect::<Vec<_>>()
    .into()
}

pub fn setup(
    mut commands: Commands,
    mut sources: ResMut<Assets<EngineSound>>,
    options: Res<Options>,
) {
    let targets = Arc::<Targets>::default();
    commands.spawn(AudioPlayer(sources.add(EngineSound {
        targets: targets.clone(),
        recording: recording(ON),
        coast: recording(OFF),
    })));
    commands.insert_resource(EngineAudio {
        targets,
        muted: options.mute,
    });
}

pub fn update(
    session: Res<Session>,
    keys: Res<ButtonInput<KeyCode>>,
    mut audio: ResMut<EngineAudio>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        audio.muted = !audio.muted;
    }
    let car = &session.race.entrants[session.player].car;
    audio
        .targets
        .rpm
        .store((car.powertrain.rpm as f32).to_bits(), Ordering::Relaxed);
    audio.targets.throttle.store(
        (session.guidance.activity.input.throttle as f32).to_bits(),
        Ordering::Relaxed,
    );
    audio
        .targets
        .gear
        .store(car.powertrain.gear, Ordering::Relaxed);
    audio
        .targets
        .limiter
        .store(car.powertrain.limiter, Ordering::Relaxed);
    audio.targets.active.store(
        session.ready && !session.paused && !audio.muted,
        Ordering::Relaxed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::audio::{AddAudioSource, AudioSink, AudioSinkPlayback};

    fn voice() -> EngineVoice {
        let targets = Arc::<Targets>::default();
        targets.active.store(true, Ordering::Relaxed);
        EngineVoice::new(targets, recording(ON), recording(OFF))
    }

    #[test]
    fn rapid_rev_shifts_and_mute_remain_continuous_and_do_not_clip() {
        let mut voice = voice();
        let (mut previous, mut peak, mut jump) = (0f32, 0f32, 0f32);
        for n in 0..RATE * 4 {
            if n % (RATE / 2) == 0 {
                let high = (n / (RATE / 2)).is_multiple_of(2);
                voice.targets.rpm.store(
                    (if high { 18000f32 } else { 5500f32 }).to_bits(),
                    Ordering::Relaxed,
                );
                voice.targets.throttle.store(
                    (if high { 1f32 } else { 0f32 }).to_bits(),
                    Ordering::Relaxed,
                );
                voice
                    .targets
                    .gear
                    .store(if high { 2 } else { 3 }, Ordering::Relaxed);
            }
            let sample = voice.sample();
            assert!(sample.is_finite());
            peak = peak.max(sample.abs());
            jump = jump.max((sample - previous).abs());
            previous = sample;
        }
        assert!(peak > 0.1 && peak < 0.9, "peak {peak}");
        // Real recordings contain legitimate high-frequency slopes. Control/loop
        // transitions must not add a step beyond that source's own slope envelope.
        let source_jump = [recording(ON), recording(OFF)]
            .iter()
            .flat_map(|samples| samples.windows(2).map(|p| (p[1] - p[0]).abs()))
            .fold(0f32, f32::max);
        assert!(
            jump < source_jump * 1.15 + 0.005,
            "transition slope {jump} exceeds recorded envelope {source_jump}"
        );
        voice.targets.active.store(false, Ordering::Relaxed);
        for _ in 0..RATE / 4 {
            voice.sample();
        }
        assert!(voice.sample().abs() < 1e-6);
    }

    #[test]
    fn source_loop_boundaries_are_quieter_than_normal_recorded_detail() {
        for samples in [recording(ON), recording(OFF)] {
            assert_eq!(
                samples.len() % 320,
                0,
                "loop must end on a four-stroke cycle"
            );
            let mut slopes: Vec<_> = samples.windows(2).map(|p| (p[1] - p[0]).abs()).collect();
            slopes.sort_by(f32::total_cmp);
            let seam = (samples[0] - samples[samples.len() - 1]).abs();
            assert!(seam < slopes[slopes.len() * 95 / 100]);
        }
    }

    #[test]
    #[ignore = "writes a listening sample"]
    fn export_engine_demo() {
        let mut voice = voice();
        let mut samples = Vec::new();
        for n in 0..RATE * 14 {
            let t = n as f32 / RATE as f32;
            let (rpm, throttle, gear) = match t {
                t if t < 1. => (4500., 0., 1),
                t if t < 4. => (4500. + (t - 1.) / 3. * 13500., 1., 1),
                t if t < 6. => (12800. + (t - 4.) / 2. * 5200., 1., 2),
                t if t < 8. => (13300. + (t - 6.) / 2. * 4700., 1., 3),
                t if t < 10. => (18000. - (t - 8.) / 2. * 8000., 0., 3),
                t if t < 12. => (13800. - (t - 10.) / 2. * 3500., 0., 2),
                _ => (15500. - (t - 12.) / 2. * 11000., 0., 1),
            };
            voice
                .targets
                .rpm
                .store(f32::to_bits(rpm), Ordering::Relaxed);
            voice
                .targets
                .throttle
                .store(f32::to_bits(throttle), Ordering::Relaxed);
            voice.targets.gear.store(gear, Ordering::Relaxed);
            samples.push((voice.sample() * 32767.) as i16);
        }
        let path = std::env::temp_dir().join("f1-smooth-engine.wav");
        let mut bytes = Vec::new();
        let size = samples.len() as u32 * 2;
        bytes.extend(b"RIFF");
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(RATE.to_le_bytes());
        bytes.extend((RATE * 2).to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for sample in samples {
            bytes.extend(sample.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        println!("Listening sample: {}", path.display());
    }

    #[test]
    #[ignore = "requires an audio output device"]
    fn live_engine_playback() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::audio::AudioPlugin::default(),
        ))
        .add_audio_source::<EngineSound>()
        .insert_resource(crate::tests::session())
        .insert_resource(Options::parse(std::iter::empty()).unwrap())
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
        assert!(world
            .resource::<EngineAudio>()
            .targets
            .active
            .load(Ordering::Relaxed));
        world.resource_mut::<Session>().paused = true;
        app.update();
        assert!(!app
            .world()
            .resource::<EngineAudio>()
            .targets
            .active
            .load(Ordering::Relaxed));
        app.world_mut().resource_mut::<Session>().paused = false;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        assert!(!app
            .world()
            .resource::<EngineAudio>()
            .targets
            .active
            .load(Ordering::Relaxed));
    }
}
