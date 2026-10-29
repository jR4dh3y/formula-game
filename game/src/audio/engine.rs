//! Procedural engine voices.
//!
//! Every cylinder fires on its own crank angle. Each firing releases an exhaust
//! pressure pulse whose strength depends on load and varies slightly from
//! cycle to cycle. The pulses run through per-bank headers, a tailpipe and
//! fixed body resonances; because those resonances do not move with RPM, the
//! timbre changes naturally as the engine sweeps through them. The firing
//! interval, pulse width and exhaust geometry give each layout its character.

use super::dsp::{saturate, smoothing, smoothstep, Biquad, Noise, OnePole, Oscillator, Pipe, RATE};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

/// The engine sound the player picks in settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineChoice {
    /// The real power unit of the selected car.
    #[default]
    MatchCar,
    V6Hybrid,
    V8,
    V12,
}

impl EngineChoice {
    pub const ALL: [Self; 4] = [Self::MatchCar, Self::V6Hybrid, Self::V8, Self::V12];
    pub fn label(self) -> &'static str {
        match self {
            Self::MatchCar => "Match the car",
            Self::V6Hybrid => "1.6 V6 turbo hybrid",
            Self::V8 => "2.4 V8",
            Self::V12 => "3.0 V12",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::MatchCar => "RB6 uses the 2.4 V8, RB22 the V6 turbo hybrid.",
            Self::V6Hybrid => {
                "2014-era power unit: a deep, raspy V6 with turbo whistle and MGU-K whine."
            }
            Self::V8 => "2006-2013 era: a flat-plane V8 screaming to 18,000 rpm.",
            Self::V12 => "1990s Ferrari-style 60-degree V12: silky and shrill at 17,500 rpm.",
        }
    }
    pub fn resolve(self, car: &str) -> Kind {
        match self {
            Self::MatchCar if car == "rb22" => Kind::V6Hybrid,
            Self::MatchCar => Kind::V8,
            Self::V6Hybrid => Kind::V6Hybrid,
            Self::V8 => Kind::V8,
            Self::V12 => Kind::V12,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    V6Hybrid = 0,
    V8 = 1,
    V12 = 2,
}

impl Kind {
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => Self::V6Hybrid,
            2 => Self::V12,
            _ => Self::V8,
        }
    }
    fn profile(self) -> &'static Profile {
        match self {
            Self::V6Hybrid => &V6,
            Self::V8 => &V8,
            Self::V12 => &V12,
        }
    }
}

pub const MAX_CYLINDERS: usize = 12;
const ORDERS: usize = 10;

struct Profile {
    cylinders: usize,
    /// The voice's own RPM range; the car's range is mapped onto it.
    idle_rpm: f32,
    limit_rpm: f32,
    /// Exhaust blowdown pulse at full load, crank degrees.
    pulse_degrees: f32,
    /// Fixed per-cylinder timing scatter from unequal header lengths, degrees.
    scatter_degrees: f32,
    /// Cycle-to-cycle combustion variation.
    jitter: f32,
    /// Turbulent gas noise riding on each pulse.
    rasp: f32,
    /// Cycle-order content below the firing pitch: the engine's growl.
    growl: f32,
    headers: [f32; 2],
    header_feedback: f32,
    tailpipe: f32,
    tail_feedback: f32,
    /// (Hz, Q, gain) body and silencer resonances.
    formants: [(f32, f32, f32); 4],
    bright_hz: f32,
    dull_hz: f32,
    airbox_hz: f32,
    /// Chance per overrun firing of an afterfire pop.
    crackle: f32,
    drive: f32,
    level: f32,
    upshift_cut: f32,
    turbo: bool,
}

const V8: Profile = Profile {
    cylinders: 8,
    idle_rpm: 5000.,
    limit_rpm: 18000.,
    pulse_degrees: 82.,
    scatter_degrees: 4.,
    jitter: 0.08,
    rasp: 0.35,
    growl: 0.5,
    headers: [0.72, 0.79],
    header_feedback: 0.42,
    tailpipe: 1.9,
    tail_feedback: 0.3,
    formants: [
        (850., 1.4, 0.5),
        (2300., 2., 0.45),
        (4200., 2.5, 0.22),
        (6500., 3., 0.1),
    ],
    bright_hz: 10500.,
    dull_hz: 3000.,
    airbox_hz: 480.,
    crackle: 0.012,
    drive: 1.8,
    level: 0.5,
    upshift_cut: 0.045,
    turbo: false,
};

const V12: Profile = Profile {
    cylinders: 12,
    idle_rpm: 4500.,
    limit_rpm: 17500.,
    pulse_degrees: 58.,
    scatter_degrees: 3.,
    jitter: 0.06,
    rasp: 0.28,
    growl: 0.28,
    headers: [0.61, 0.66],
    header_feedback: 0.38,
    tailpipe: 1.4,
    tail_feedback: 0.28,
    formants: [
        (1100., 1.3, 0.45),
        (2900., 1.8, 0.5),
        (5200., 2.5, 0.28),
        (8000., 3., 0.1),
    ],
    bright_hz: 12000.,
    dull_hz: 3400.,
    airbox_hz: 620.,
    crackle: 0.014,
    drive: 1.5,
    level: 0.5,
    upshift_cut: 0.05,
    turbo: false,
};

const V6: Profile = Profile {
    cylinders: 6,
    idle_rpm: 4500.,
    limit_rpm: 13500.,
    pulse_degrees: 112.,
    scatter_degrees: 5.,
    jitter: 0.1,
    rasp: 0.5,
    growl: 0.8,
    headers: [1.05, 1.12],
    header_feedback: 0.4,
    tailpipe: 2.6,
    tail_feedback: 0.35,
    formants: [
        (420., 1.2, 0.55),
        (1150., 1.6, 0.45),
        (2600., 2.2, 0.2),
        (4200., 3., 0.07),
    ],
    bright_hz: 6200.,
    dull_hz: 2100.,
    airbox_hz: 300.,
    crackle: 0.006,
    drive: 2.2,
    level: 0.5,
    upshift_cut: 0.03,
    turbo: true,
};

/// What the simulation says the engine is doing, refreshed every control tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    /// 0 at idle, 1 at the rev limit.
    pub revs: f32,
    pub throttle: f32,
    pub brake: f32,
    pub gear: i32,
    pub limiter: bool,
    /// 0 outside the car, 1 in the cockpit (airbox and chassis dominate).
    pub onboard: f32,
    /// Pitch factor from relative motion to the listener.
    pub doppler: f32,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            revs: 0.,
            throttle: 0.,
            brake: 0.,
            gear: 1,
            limiter: false,
            onboard: 0.,
            doppler: 1.,
        }
    }
}

pub struct Voice {
    kind: Kind,
    profile: &'static Profile,
    pub control: Control,
    noise: Noise,
    cycle: f64,
    revs: f32,
    load: f32,
    onboard: f32,
    doppler: f32,
    offsets: [f32; MAX_CYLINDERS],
    /// Amplitude and phase of each cycle order below the firing frequency.
    orders: [(f32, f32); ORDERS],
    gains: [f32; MAX_CYLINDERS],
    previous: [f32; MAX_CYLINDERS],
    amplitude: [f32; MAX_CYLINDERS],
    shift_cut: f32,
    blip: f32,
    overrun_bonus: f32,
    pop: f32,
    pop_bank: usize,
    pop_filter: OnePole,
    rasp_filter: Biquad,
    headers: [Pipe; 2],
    tailpipe: Pipe,
    formants: [Biquad; 4],
    tone: [OnePole; 2],
    dc: Biquad,
    airbox: Biquad,
    induction: Biquad,
    boost: f32,
    flutter: f32,
    flutter_clock: Oscillator,
    flutter_filter: Biquad,
    whistle: [Oscillator; 2],
    whistle_hiss: Biquad,
    motor: [Oscillator; 2],
    motor_level: f32,
    last_throttle: f32,
}

impl Voice {
    pub fn new(kind: Kind, seed: u32) -> Self {
        let mut voice = Self {
            kind,
            profile: kind.profile(),
            control: Control::default(),
            noise: Noise::new(seed),
            cycle: 0.,
            revs: 0.,
            load: 0.,
            onboard: 0.,
            doppler: 1.,
            offsets: [0.; MAX_CYLINDERS],
            orders: [(0., 0.); ORDERS],
            gains: [1.; MAX_CYLINDERS],
            previous: [0.; MAX_CYLINDERS],
            amplitude: [0.; MAX_CYLINDERS],
            shift_cut: 0.,
            blip: 0.,
            overrun_bonus: 0.,
            pop: 0.,
            pop_bank: 0,
            pop_filter: OnePole::new(2800.),
            rasp_filter: Biquad::highpass(1800., 0.7),
            headers: [Pipe::new(), Pipe::new()],
            tailpipe: Pipe::new(),
            formants: [Biquad::default(); 4],
            tone: [OnePole::default(); 2],
            dc: Biquad::highpass(28., 0.7),
            airbox: Biquad::default(),
            induction: Biquad::default(),
            boost: 0.,
            flutter: 0.,
            flutter_clock: Oscillator::default(),
            flutter_filter: Biquad::bandpass(1500., 1.2),
            whistle: [Oscillator::default(); 2],
            whistle_hiss: Biquad::bandpass(4000., 6.),
            motor: [Oscillator::default(); 2],
            motor_level: 0.,
            last_throttle: 0.,
        };
        voice.configure(kind);
        voice
    }

    #[cfg(test)]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Switch engine layout. Buffers are reused, so this never allocates.
    pub fn configure(&mut self, kind: Kind) {
        let profile = kind.profile();
        self.kind = kind;
        self.profile = profile;
        // Deterministic per-layout scatter so an engine always sounds like itself.
        let mut scatter = Noise::new(0x9E37_79B9 ^ ((kind as u32 + 1) * 7919));
        let interval = 1. / profile.cylinders as f32;
        for cylinder in 0..MAX_CYLINDERS {
            self.offsets[cylinder] =
                cylinder as f32 * interval + scatter.next() * profile.scatter_degrees / 720.;
            self.gains[cylinder] = 1. + 0.14 * scatter.next();
            self.amplitude[cylinder] = 0.;
        }
        for (order, slot) in self.orders.iter_mut().enumerate() {
            let k = order + 1;
            // Firing harmonics already come from the pulses.
            let amplitude = if k >= profile.cylinders {
                0.
            } else {
                profile.growl * (0.6 + 0.4 * scatter.unit()) / (1. + k as f32 * 0.35)
            };
            *slot = (amplitude, scatter.unit());
        }
        for (pipe, length) in self.headers.iter_mut().zip(profile.headers) {
            pipe.configure(length, profile.header_feedback, 8000.);
        }
        self.tailpipe
            .configure(profile.tailpipe, profile.tail_feedback, 5000.);
        for (filter, (frequency, q, _)) in self.formants.iter_mut().zip(profile.formants) {
            filter.retune(Biquad::bandpass(frequency, q));
        }
        self.airbox.retune(Biquad::bandpass(profile.airbox_hz, 0.9));
        self.induction
            .retune(Biquad::bandpass(profile.airbox_hz * 2.6, 1.4));
    }

    /// Apply the latest control values; call once per control tick.
    pub fn update_control(&mut self, control: Control) {
        if self.control.gear != control.gear && control.gear > 0 && self.control.gear > 0 {
            if control.gear < self.control.gear {
                self.blip = 1.;
                self.overrun_bonus = 1.;
            } else {
                self.shift_cut = self.profile.upshift_cut;
            }
        }
        if self.profile.turbo {
            // Lifting off a loaded turbo makes the compressor chatter briefly.
            if self.boost > 0.45 && control.throttle < 0.2 && self.last_throttle > 0.6 {
                self.flutter = self.boost;
            }
            let shaft = 0.25 + 0.55 * self.boost + 0.2 * self.revs;
            self.whistle_hiss
                .retune(Biquad::bandpass(2000. + 5600. * shaft, 5.));
        }
        self.last_throttle = control.throttle;
        self.control = control;
    }

    fn rpm(&self) -> f32 {
        let p = self.profile;
        p.idle_rpm + self.revs.max(0.) * (p.limit_rpm - p.idle_rpm)
    }

    fn fire(&mut self, cylinder: usize) {
        let p = self.profile;
        let n = self.noise.next();
        let mut amplitude = (0.3 + 0.7 * self.load)
            * self.gains[cylinder]
            * (1. + p.jitter * (1.6 - self.load) * n);
        let overrun = (1. - self.load) * smoothstep(0.2, 0.6, self.revs);
        let mut pop_chance =
            p.crackle * overrun * (1. + 3. * self.overrun_bonus) * (0.6 + self.revs);
        if self.control.limiter && self.noise.unit() < 0.55 {
            // Ignition cut: the classic stutter, with fuel lighting in the pipe.
            amplitude *= 0.04;
            pop_chance += 0.06;
        }
        if self.shift_cut > 0. {
            amplitude *= 0.12;
        }
        if self.noise.unit() < pop_chance {
            self.pop = 0.5 + self.noise.unit();
            self.pop_bank = cylinder % 2;
        }
        self.amplitude[cylinder] = amplitude.max(0.);
    }

    pub fn sample(&mut self) -> f32 {
        let p = self.profile;
        let c = self.control;
        self.revs += (c.revs - self.revs) * smoothing(0.005);
        self.doppler += (c.doppler - self.doppler) * smoothing(0.08);
        self.onboard += (c.onboard - self.onboard) * smoothing(0.15);
        self.blip = (self.blip - 1. / (0.09 * RATE)).max(0.);
        self.shift_cut = (self.shift_cut - 1. / RATE).max(0.);
        self.overrun_bonus = (self.overrun_bonus - 1. / (0.35 * RATE)).max(0.);
        let blip = (self.blip * std::f32::consts::PI).sin();
        let throttle = (c.throttle + 0.7 * blip).min(1.);
        self.load += (throttle - self.load) * smoothing(0.025);
        let load = self.load;
        let rpm = self.rpm();

        self.cycle = (self.cycle + (rpm / 120. * self.doppler / RATE) as f64).fract();
        let width = p.pulse_degrees / 720. * (1.35 - 0.45 * load);
        let gas = self.rasp_filter.process(self.noise.next());
        let mut banks = [0f32; 2];
        let mut intake = 0.;
        for cylinder in 0..p.cylinders {
            let local = (self.cycle as f32 - self.offsets[cylinder]).rem_euclid(1.);
            if local < self.previous[cylinder] {
                self.fire(cylinder);
            }
            self.previous[cylinder] = local;
            let amplitude = self.amplitude[cylinder];
            let x = local / width;
            if x < 1. {
                banks[cylinder % 2] += amplitude * pulse(x) * (1. + p.rasp * gas);
            }
            let x = (local + 0.55).fract() / (width * 1.8);
            if x < 1. {
                intake += amplitude * pulse(x);
            }
        }
        // Afterfire pops are short broadband bursts in one exhaust bank.
        if self.pop > 1e-3 {
            let burst = self.pop_filter.process(self.noise.next()) * self.pop * 2.2;
            banks[self.pop_bank] += burst;
            self.pop *= 0.9948;
        }

        let mut growl = 0.;
        for (order, (amplitude, phase)) in self.orders.iter().enumerate() {
            growl += amplitude * (TAU * ((order + 1) as f32 * self.cycle as f32 + phase)).sin();
        }
        let growl = growl * (0.35 + 0.65 * load) * 0.12;
        banks[0] += growl;
        banks[1] += growl;
        let headers = self.headers[0].process(banks[0]) + self.headers[1].process(banks[1]);
        let tail = self.tailpipe.process(headers * 0.5);
        let mut body = tail * 0.55;
        for (filter, (_, _, gain)) in self.formants.iter_mut().zip(p.formants) {
            body += filter.process(tail) * gain;
        }
        let loudness = 0.6 + 0.4 * self.revs.clamp(0., 1.);
        let mut exhaust = saturate(body * p.drive * loudness);

        let mut extra = 0.;
        if p.turbo {
            let target = c.throttle * smoothstep(0.15, 0.65, self.revs);
            let rate = if target > self.boost { 0.18 } else { 0.35 };
            self.boost += (target - self.boost) * smoothing(rate);
            let shaft = 0.25 + 0.55 * self.boost + 0.2 * self.revs;
            let frequency = 2000. + 5600. * shaft;
            extra += self.whistle[0].process(frequency) * (0.006 + 0.022 * self.boost)
                + self.whistle[1].process(frequency * 1.47) * 0.006 * self.boost
                + self.whistle_hiss.process(self.noise.next()) * 0.05 * self.boost;
            if self.flutter > 1e-3 {
                let chatter = 0.5 + 0.5 * self.flutter_clock.process(23.).signum();
                extra +=
                    self.flutter_filter.process(self.noise.next()) * chatter * self.flutter * 0.22;
                self.flutter *= 1. - 1. / (0.12 * RATE);
            }
            // MGU-K: electric whine deploying on power, louder harvesting under braking.
            let target = 0.012 * c.throttle * self.revs.clamp(0., 1.) + 0.03 * c.brake;
            self.motor_level += (target - self.motor_level) * smoothing(0.06);
            let frequency = rpm / 60. * 13.;
            extra += (self.motor[0].process(frequency)
                + 0.4 * self.motor[1].process(frequency * 2.))
                * self.motor_level;
            // The turbine absorbs much of the exhaust's high-frequency energy.
            exhaust *= 1. - 0.15 * self.boost;
        }

        let cutoff = (p.dull_hz + (p.bright_hz - p.dull_hz) * load)
            * (0.7 + 0.3 * self.revs.clamp(0., 1.))
            * (1. - 0.45 * self.onboard)
            * if p.turbo { 1. - 0.3 * self.boost } else { 1. };
        for filter in &mut self.tone {
            filter.tune(cutoff);
            exhaust = filter.process(exhaust);
        }
        let airbox = self.airbox.process(intake) * 1.4
            + self.induction.process(self.noise.next()) * 0.08 * load * self.revs.max(0.);
        let mix = exhaust * (1. - 0.25 * self.onboard)
            + airbox * (0.15 + 0.85 * self.onboard)
            + extra * (0.6 + 0.4 * self.onboard);
        self.dc.process(mix) * p.level
    }
}

/// Exhaust blowdown: a fast rise as the valve opens, then a longer decay.
fn pulse(x: f32) -> f32 {
    const ATTACK: f32 = 0.14;
    if x < ATTACK {
        let t = x / ATTACK;
        t * t * (3. - 2. * t)
    } else {
        let d = (x - ATTACK) / (1. - ATTACK);
        (1. - d) * (1. - d) * (1. - 0.5 * d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(kind: Kind, control: Control, seconds: f32) -> (f32, f32) {
        let mut voice = Voice::new(kind, 11);
        voice.update_control(control);
        let total = (RATE * seconds) as usize;
        let (mut sum, mut peak) = (0f32, 0f32);
        for n in 0..total {
            let s = voice.sample();
            assert!(s.is_finite());
            if n > total / 4 {
                sum += s * s;
                peak = peak.max(s.abs());
            }
        }
        ((sum / (total - total / 4) as f32).sqrt(), peak)
    }

    #[test]
    fn every_layout_is_audible_and_louder_on_power() {
        for kind in [Kind::V6Hybrid, Kind::V8, Kind::V12] {
            let full = Control {
                revs: 0.9,
                throttle: 1.,
                ..default()
            };
            let lift = Control {
                throttle: 0.,
                ..full
            };
            let (on, peak) = rms(kind, full, 1.);
            let (off, _) = rms(kind, lift, 1.);
            assert!(on > 0.05 && peak < 0.9, "{kind:?} on {on} peak {peak}");
            assert!(off < on * 0.8, "{kind:?} off {off} on {on}");
        }
    }

    #[test]
    fn layouts_are_balanced_against_each_other() {
        let full = Control {
            revs: 0.9,
            throttle: 1.,
            ..default()
        };
        let levels: Vec<f32> = [Kind::V6Hybrid, Kind::V8, Kind::V12]
            .into_iter()
            .map(|kind| rms(kind, full, 1.).0)
            .collect();
        let (low, high) = levels
            .iter()
            .fold((f32::MAX, 0f32), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(high / low < 1.6, "unbalanced layouts {levels:?}");
    }

    #[test]
    fn firing_frequency_tracks_cylinder_count_and_rpm() {
        // Zero crossings of the low band give the dominant firing pitch.
        for (kind, cylinders, rpm) in [(Kind::V8, 8., 11500.), (Kind::V12, 12., 11000.)] {
            let mut voice = Voice::new(kind, 3);
            voice.update_control(Control {
                revs: 0.5,
                throttle: 1.,
                ..default()
            });
            let mut band = Biquad::bandpass(rpm / 120. * cylinders, 4.);
            let (mut crossings, mut previous) = (0, 0f32);
            for n in 0..RATE as usize {
                let s = band.process(voice.sample());
                if n > RATE as usize / 4 && previous <= 0. && s > 0. {
                    crossings += 1;
                }
                previous = s;
            }
            let measured = crossings as f32 / 0.75;
            let expected = rpm / 120. * cylinders;
            assert!(
                (measured - expected).abs() < expected * 0.05,
                "{kind:?} {measured} vs {expected}"
            );
        }
    }

    fn default() -> Control {
        Control::default()
    }
}
