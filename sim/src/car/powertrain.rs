//! Engine and sequential gearbox with an automatic clutch.

use super::spec::EngineSpec;
use std::f64::consts::PI;

pub const NEUTRAL: i32 = 0;
pub const REVERSE: i32 = -1;

/// Why a shift request did not happen as asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftRefusal {
    /// The downshift would over-rev the engine; the gearbox protected itself.
    OverRev,
    /// Reverse and neutral need the car (almost) stopped.
    TooFast,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Powertrain {
    /// -1 reverse, 0 neutral, 1.. forward gears.
    pub gear: i32,
    pub rpm: f64,
    /// Remaining time with drive interrupted by a shift.
    pub shift_timer: f64,
    /// Short lockout after a refused shift, the "readable penalty".
    pub refusal_timer: f64,
    pub last_refusal: Option<ShiftRefusal>,
    /// The engine is bouncing off the rev limiter.
    pub limiter: bool,
}

impl Powertrain {
    pub fn new(spec: &EngineSpec) -> Self {
        Self {
            gear: 1,
            rpm: spec.idle_rpm,
            shift_timer: 0.0,
            refusal_timer: 0.0,
            last_refusal: None,
            limiter: false,
        }
    }

    fn ratio(&self, spec: &EngineSpec) -> f64 {
        match self.gear {
            REVERSE => -spec.reverse_ratio,
            NEUTRAL => 0.0,
            gear => spec.gear_ratios[(gear - 1) as usize],
        }
    }

    /// Engine speed the wheels would impose in `gear`.
    pub fn wheel_rpm(spec: &EngineSpec, gear: i32, wheel_speed: f64, wheel_radius: f64) -> f64 {
        let ratio = match gear {
            REVERSE => spec.reverse_ratio,
            NEUTRAL => 0.0,
            gear => spec.gear_ratios[(gear - 1) as usize],
        };
        (wheel_speed / wheel_radius * ratio * 60.0 / (2.0 * PI)).abs()
    }

    pub fn shift_up(&mut self, spec: &EngineSpec, speed: f64) {
        if self.shift_timer > 0.0
            || self.refusal_timer > 0.0
            || self.gear >= spec.gear_ratios.len() as i32
        {
            return;
        }
        if self.gear == REVERSE && speed.abs() > 2.0 {
            self.refuse(ShiftRefusal::TooFast);
            return;
        }
        self.gear += 1;
        self.shift_timer = spec.upshift_time;
    }

    pub fn shift_down(&mut self, spec: &EngineSpec, speed: f64, wheel_radius: f64) {
        if self.shift_timer > 0.0 || self.refusal_timer > 0.0 || self.gear <= REVERSE {
            return;
        }
        let target = self.gear - 1;
        // Neutral and reverse are only selectable when nearly stopped.
        if target <= NEUTRAL && speed.abs() > 2.0 {
            self.refuse(ShiftRefusal::TooFast);
            return;
        }
        if target >= 1
            && Self::wheel_rpm(spec, target, speed, wheel_radius) > spec.rev_limit + 400.0
        {
            self.refuse(ShiftRefusal::OverRev);
            return;
        }
        self.gear = target;
        self.shift_timer = spec.downshift_time;
    }

    fn refuse(&mut self, reason: ShiftRefusal) {
        self.last_refusal = Some(reason);
        self.refusal_timer = 0.25;
    }

    /// Picks the gear an automatic gearbox would use.
    pub fn auto_shift(&mut self, spec: &EngineSpec, speed: f64, wheel_radius: f64, throttle: f64) {
        if self.shift_timer > 0.0 || self.gear < 1 {
            return;
        }
        let up_rpm = spec.rev_limit * 0.985;
        // Kick down for acceleration on throttle; off throttle only when the
        // engine is close to bogging, so engine braking stays gentle.
        let down_rpm = spec.peak_power_rpm * if throttle > 0.6 { 0.68 } else { 0.5 };
        if self.rpm > up_rpm && self.gear < spec.gear_ratios.len() as i32 {
            self.shift_up(spec, speed);
        } else if self.gear > 1
            && self.rpm < down_rpm
            && Self::wheel_rpm(spec, self.gear - 1, speed, wheel_radius) < spec.rev_limit * 0.95
        {
            self.shift_down(spec, speed, wheel_radius);
        }
    }

    /// Advances timers and engine speed; returns the drive force at the
    /// contact patch of the driven axle (positive pushes forward).
    pub fn drive_force(
        &mut self,
        spec: &EngineSpec,
        throttle: f64,
        forward_speed: f64,
        wheel_radius: f64,
        dt: f64,
    ) -> f64 {
        self.shift_timer = (self.shift_timer - dt).max(0.0);
        self.refusal_timer = (self.refusal_timer - dt).max(0.0);
        if self.refusal_timer == 0.0 {
            self.last_refusal = None;
        }
        let ratio = self.ratio(spec);
        let coupled_rpm = (forward_speed / wheel_radius * ratio * 60.0 / (2.0 * PI)).abs();
        // The automatic clutch slips below a launch speed so the engine never stalls.
        let launch_rpm = spec.idle_rpm + (spec.peak_power_rpm * 0.62 - spec.idle_rpm) * throttle;
        let target_rpm = if ratio == 0.0 {
            spec.idle_rpm + (spec.rev_limit - spec.idle_rpm) * throttle * 0.9
        } else {
            coupled_rpm.max(launch_rpm.min(spec.peak_power_rpm))
        };
        self.rpm += (target_rpm - self.rpm) * (dt * 25.0).min(1.0);
        self.rpm = self.rpm.clamp(spec.idle_rpm, spec.rev_limit + 600.0);

        self.limiter = self.rpm >= spec.rev_limit && throttle > 0.05;
        if ratio == 0.0 || self.shift_timer > 0.0 {
            return 0.0;
        }
        let omega = self.rpm * 2.0 * PI / 60.0;
        let power = if self.limiter {
            0.0
        } else {
            spec.max_power * power_shape(self.rpm / spec.peak_power_rpm)
        };
        let engine_torque = power / omega.max(1.0);
        let drive = engine_torque * throttle * ratio / wheel_radius;
        // Engine braking off throttle, stronger at high rpm.
        let braking_torque = spec.max_power / (spec.peak_power_rpm * 2.0 * PI / 60.0)
            * 0.09
            * (self.rpm / spec.rev_limit)
            * (1.0 - throttle);
        // Braking opposes travel even in reverse; fade out as the clutch opens at rest.
        let engine_brake =
            braking_torque * ratio.abs() / wheel_radius * (forward_speed / 0.5).clamp(-1.0, 1.0);
        drive - engine_brake
    }
}

/// Power relative to peak as a function of rpm relative to the peak-power rpm.
pub fn power_shape(x: f64) -> f64 {
    if x <= 1.0 {
        (1.0 - ((1.0 - x) / 0.6).powi(2)).max(0.12)
    } else {
        (1.0 - 2.5 * (x - 1.0).powi(2)).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::spec::{test_wheels, CarSpec};

    #[test]
    fn engine_braking_opposes_motion_in_forward_and_reverse_gears() {
        let spec = CarSpec::rb22(test_wheels(), 5.4, 1.9).engine;
        for gear in [REVERSE, 1] {
            for speed in [-5.0, 5.0] {
                let mut powertrain = Powertrain::new(&spec);
                powertrain.gear = gear;
                let force = powertrain.drive_force(&spec, 0.0, speed, 0.355, 0.01);
                assert!(
                    force * speed < 0.0,
                    "gear {gear}, speed {speed}, force {force}"
                );
            }
            let mut powertrain = Powertrain::new(&spec);
            powertrain.gear = gear;
            assert_eq!(powertrain.drive_force(&spec, 0.0, 0.0, 0.355, 0.01), 0.0);
        }
    }

    #[test]
    fn shifting_cannot_skip_gears_during_the_torque_cut() {
        let spec = CarSpec::rb22(test_wheels(), 5.4, 1.9).engine;
        for up in [false, true] {
            let mut powertrain = Powertrain::new(&spec);
            powertrain.gear = 3;
            let shift = |powertrain: &mut Powertrain| {
                if up {
                    powertrain.shift_up(&spec, 10.0);
                } else {
                    powertrain.shift_down(&spec, 10.0, 0.355);
                }
            };
            shift(&mut powertrain);
            let expected = if up { 4 } else { 2 };
            let timer = powertrain.shift_timer;
            shift(&mut powertrain);
            assert_eq!(powertrain.gear, expected);
            assert_eq!(powertrain.shift_timer, timer);
            powertrain.drive_force(&spec, 0.0, 10.0, 0.355, timer + 0.001);
            shift(&mut powertrain);
            assert_eq!(powertrain.gear, if up { 5 } else { 1 });
        }
    }

    #[test]
    fn early_upshift_loses_acceleration() {
        let spec = CarSpec::rb22(test_wheels(), 5.4, 1.9).engine;
        let radius = 0.355;
        let speed_at = |gear: i32, rpm: f64| {
            rpm / (spec.gear_ratios[(gear - 1) as usize] * 60.0 / (2.0 * PI)) * radius
        };
        // At the speed where 3rd hits 92% of the limiter, staying in 3rd beats 4th.
        let speed = speed_at(3, spec.rev_limit * 0.92);
        let force = |gear| {
            let mut powertrain = Powertrain::new(&spec);
            powertrain.gear = gear;
            powertrain.rpm = Powertrain::wheel_rpm(&spec, gear, speed, radius);
            powertrain.drive_force(&spec, 1.0, speed, radius, 0.001)
        };
        assert!(force(3) > force(4));
    }

    #[test]
    fn over_revving_downshift_is_refused() {
        let spec = CarSpec::rb22(test_wheels(), 5.4, 1.9).engine;
        let mut powertrain = Powertrain::new(&spec);
        powertrain.gear = 7;
        powertrain.shift_down(&spec, 85.0, 0.355);
        assert_eq!(powertrain.gear, 7);
        assert_eq!(powertrain.last_refusal, Some(ShiftRefusal::OverRev));
        // The lockout ends and a sensible downshift is accepted.
        powertrain.drive_force(&spec, 0.0, 85.0, 0.355, 0.3);
        powertrain.shift_down(&spec, 70.0, 0.355);
        assert_eq!(powertrain.gear, 6);
    }

    #[test]
    fn limiter_cuts_power() {
        let spec = CarSpec::rb22(test_wheels(), 5.4, 1.9).engine;
        let mut powertrain = Powertrain::new(&spec);
        powertrain.rpm = spec.rev_limit + 100.0;
        let speed = (spec.rev_limit + 100.0) / (spec.gear_ratios[0] * 60.0 / (2.0 * PI)) * 0.355;
        let force = powertrain.drive_force(&spec, 1.0, speed, 0.355, 0.0001);
        assert!(powertrain.limiter);
        assert!(force <= 0.0);
    }
}
