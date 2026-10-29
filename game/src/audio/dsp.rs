//! Small allocation-free building blocks for the audio thread.

use std::f32::consts::{PI, TAU};

pub const RATE: f32 = 48_000.;

/// Per-sample coefficient for a one-pole smoother with time constant `seconds`.
pub fn smoothing(seconds: f32) -> f32 {
    1. - (-1. / (seconds * RATE)).exp()
}

/// Rational tanh approximation: soft saturation without a transcendental call.
pub fn saturate(x: f32) -> f32 {
    let x = x.clamp(-3., 3.);
    x * (27. + x * x) / (27. + 9. * x * x)
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// Xorshift white noise; deterministic per seed.
#[derive(Clone)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }
    /// Uniform in [-1, 1).
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1 << 23) as f32 - 1.
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        0.5 + 0.5 * self.next()
    }
}

/// One-pole low-pass that can be retuned every sample.
#[derive(Clone, Copy, Default)]
pub struct OnePole {
    pub state: f32,
    coefficient: f32,
}

impl OnePole {
    pub fn new(cutoff: f32) -> Self {
        let mut filter = Self::default();
        filter.tune(cutoff);
        filter
    }
    pub fn tune(&mut self, cutoff: f32) {
        self.coefficient = 1. - (-TAU * cutoff.clamp(10., RATE * 0.45) / RATE).exp();
    }
    pub fn process(&mut self, x: f32) -> f32 {
        self.state += (x - self.state) * self.coefficient;
        self.state
    }
}

/// RBJ-cookbook biquad, transposed direct form II.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn normalized(b: [f32; 3], a: [f32; 3]) -> Self {
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
            z1: 0.,
            z2: 0.,
        }
    }
    fn omega(frequency: f32, q: f32) -> (f32, f32) {
        let w = TAU * frequency.clamp(10., RATE * 0.45) / RATE;
        (w.cos(), w.sin() / (2. * q))
    }
    #[cfg(test)]
    pub fn lowpass(frequency: f32, q: f32) -> Self {
        let (cos, alpha) = Self::omega(frequency, q);
        let b = (1. - cos) / 2.;
        Self::normalized([b, 1. - cos, b], [1. + alpha, -2. * cos, 1. - alpha])
    }
    pub fn highpass(frequency: f32, q: f32) -> Self {
        let (cos, alpha) = Self::omega(frequency, q);
        let b = (1. + cos) / 2.;
        Self::normalized([b, -1. - cos, b], [1. + alpha, -2. * cos, 1. - alpha])
    }
    /// Band-pass with 0 dB peak gain.
    pub fn bandpass(frequency: f32, q: f32) -> Self {
        let (cos, alpha) = Self::omega(frequency, q);
        Self::normalized([alpha, 0., -alpha], [1. + alpha, -2. * cos, 1. - alpha])
    }
    pub fn peaking(frequency: f32, q: f32, decibels: f32) -> Self {
        let (cos, alpha) = Self::omega(frequency, q);
        let a = 10f32.powf(decibels / 40.);
        Self::normalized(
            [1. + alpha * a, -2. * cos, 1. - alpha * a],
            [1. + alpha / a, -2. * cos, 1. - alpha / a],
        )
    }
    pub fn high_shelf(frequency: f32, decibels: f32) -> Self {
        let a = 10f32.powf(decibels / 40.);
        let (cos, alpha) = Self::omega(frequency, std::f32::consts::FRAC_1_SQRT_2);
        let root = 2. * a.sqrt() * alpha;
        Self::normalized(
            [
                a * ((a + 1.) + (a - 1.) * cos + root),
                -2. * a * ((a - 1.) + (a + 1.) * cos),
                a * ((a + 1.) + (a - 1.) * cos - root),
            ],
            [
                (a + 1.) - (a - 1.) * cos + root,
                2. * ((a - 1.) - (a + 1.) * cos),
                (a + 1.) - (a - 1.) * cos - root,
            ],
        )
    }
    /// Replace coefficients but keep the filter's memory, so retuning is click-free.
    pub fn retune(&mut self, other: Self) {
        *self = Self {
            z1: self.z1,
            z2: self.z2,
            ..other
        };
    }
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

const COMB_SIZE: usize = 1024;

/// Feedback comb with damping: a lossy pipe. Its resonances stay fixed in
/// frequency while the engine's pitch moves through them, as in a real exhaust.
pub struct Pipe {
    buffer: Box<[f32; COMB_SIZE]>,
    position: usize,
    delay: usize,
    feedback: f32,
    damping: OnePole,
}

impl Pipe {
    pub fn new() -> Self {
        Self {
            buffer: Box::new([0.; COMB_SIZE]),
            position: 0,
            delay: 1,
            feedback: 0.,
            damping: OnePole::new(6000.),
        }
    }
    pub fn configure(&mut self, milliseconds: f32, feedback: f32, damping: f32) {
        self.delay = ((milliseconds * 0.001 * RATE) as usize).clamp(1, COMB_SIZE - 1);
        self.feedback = feedback;
        self.damping.tune(damping);
    }
    pub fn process(&mut self, x: f32) -> f32 {
        let delayed = self.buffer[(self.position + COMB_SIZE - self.delay) % COMB_SIZE];
        let y = x + self.feedback * self.damping.process(delayed);
        self.buffer[self.position] = y;
        self.position = (self.position + 1) % COMB_SIZE;
        y
    }
}

/// Sine oscillator with continuous phase under frequency changes.
#[derive(Clone, Copy, Default)]
pub struct Oscillator {
    phase: f32,
}

impl Oscillator {
    pub fn process(&mut self, frequency: f32) -> f32 {
        self.phase = (self.phase + frequency / RATE).fract();
        (self.phase * TAU).sin()
    }
}

/// Equal-power stereo gains for `pan` in [-1, 1].
pub fn pan(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1., 1.) + 1.) * PI / 4.;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(mut filter: Biquad, frequency: f32) -> f32 {
        let mut oscillator = Oscillator::default();
        let mut peak = 0f32;
        for n in 0..RATE as usize {
            let y = filter.process(oscillator.process(frequency));
            if n > RATE as usize / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn filters_pass_and_reject_the_expected_bands() {
        assert!(gain(Biquad::lowpass(1000., 0.707), 100.) > 0.95);
        assert!(gain(Biquad::lowpass(1000., 0.707), 8000.) < 0.05);
        assert!(gain(Biquad::highpass(1000., 0.707), 100.) < 0.05);
        assert!((gain(Biquad::bandpass(1000., 2.), 1000.) - 1.).abs() < 0.02);
        let dip = gain(Biquad::peaking(3000., 1., -6.), 3000.);
        assert!((dip - 0.5).abs() < 0.03, "{dip}");
        assert!(gain(Biquad::high_shelf(4000., -6.), 12000.) < 0.6);
    }

    #[test]
    fn noise_is_bounded_and_centered() {
        let mut noise = Noise::new(7);
        let samples: Vec<f32> = (0..100_000).map(|_| noise.next()).collect();
        assert!(samples.iter().all(|s| (-1. ..1.).contains(s)));
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!(mean.abs() < 0.01);
    }
}
