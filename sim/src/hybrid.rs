//! 2026-regulation car systems: automatic active aerodynamics and a battery
//! that drives the MGU-K, with a driver-triggered overtake mode replacing DRS.
//!
//! Active aero opens both wings into a low-drag straight mode inside the
//! circuit's straight-mode zones and closes them for braking, lifting or
//! cornering load. The battery deploys up to the MGU-K's power, tapered away
//! at very high speed, and recharges under braking and when off the throttle.
//! The car's DRS button becomes the overtake button: within a second of the
//! car ahead at an overtake detection point it engages overtake mode (extra
//! energy and full power to a higher speed); anywhere else it toggles a boost
//! that spends the battery at full power until the brakes go on.

use serde::{Deserialize, Serialize};

/// Seconds for the wing elements to move between modes.
pub const AERO_ACTUATION: f64 = 0.3;
/// Lateral acceleration (m/s²) above which straight mode closes, and below
/// which it may reopen. A flat kink stays open; a real corner closes it.
const AERO_CLOSE_LATERAL: f64 = 20.0;
const AERO_OPEN_LATERAL: f64 = 12.0;
/// Below this the car is not on a straight worth opening for.
const AERO_MIN_SPEED: f64 = 120.0 / 3.6;

/// Energy store and MGU-K, per car.
#[derive(Clone, Debug, PartialEq)]
pub struct HybridSpec {
    /// MGU-K deployment limit inside straight-mode zones, W.
    pub mguk_power: f64,
    /// Deployment limit elsewhere, W (the 2026 mid-season rule).
    pub corner_power: f64,
    /// Full power below this road speed, falling to zero at `taper_end`, m/s.
    pub taper_start: f64,
    pub taper_end: f64,
    /// Overtake mode keeps full power to this speed instead.
    pub overtake_taper_start: f64,
    /// Usable battery window, J.
    pub capacity: f64,
    /// Most energy that may be recovered in one lap, J.
    pub lap_harvest: f64,
    /// Regenerative braking limit, W.
    pub regen_power: f64,
    /// Harvest off the throttle (ICE-driven part-load charging), W.
    pub coast_power: f64,
    /// Super clipping: at full throttle above `taper_start` in normal mode the
    /// MGU-K turns generator, taking up to this much of the engine's power
    /// to recharge, scaled toward the top of the taper, W.
    pub clip_power: f64,
    /// Extra energy an overtake grants, J.
    pub overtake_energy: f64,
    /// Battery share below which the control unit stops deploying in normal
    /// mode, and above which it deploys in full. In between deployment is
    /// scaled, so normal driving settles where deployment matches harvest.
    pub reserve: f64,
    pub full_deploy: f64,
}

impl HybridSpec {
    /// MGU-K limit at `speed` (m/s) before the battery's state of charge.
    pub fn taper(&self, speed: f64, overtake: bool) -> f64 {
        let start = if overtake {
            self.overtake_taper_start
        } else {
            self.taper_start
        };
        if speed <= start {
            1.0
        } else {
            ((self.taper_end - speed) / (self.taper_end - start)).clamp(0.0, 1.0)
        }
    }
}

/// Moving wing elements and how much they change the car in straight mode.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveAeroSpec {
    /// CdA removed with the wings fully in straight mode.
    pub drag: f64,
    /// Front and rear ClA removed with the wings fully in straight mode.
    pub front_lift: f64,
    pub rear_lift: f64,
}

/// Active aerodynamics state. The race sets `allowed` inside straight-mode
/// zones; the car decides when to open and close.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveAero {
    pub allowed: bool,
    /// Commanded straight mode.
    pub straight: bool,
    /// Element position, 0 corner mode to 1 straight mode.
    pub position: f64,
}

impl ActiveAero {
    pub(crate) fn update(&mut self, throttle: f64, brake: f64, lateral: f64, speed: f64, dt: f64) {
        let lateral = lateral.abs();
        let close = !self.allowed
            || brake > 0.02
            || throttle < 0.8
            || speed < AERO_MIN_SPEED
            || lateral > AERO_CLOSE_LATERAL;
        if close {
            self.straight = false;
        } else if lateral < AERO_OPEN_LATERAL {
            self.straight = true;
        }
        let target = if self.straight { 1.0 } else { 0.0 };
        let rate = dt / AERO_ACTUATION;
        self.position += (target - self.position).clamp(-rate, rate);
    }
}

/// What the battery is doing this tick, for the HUD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployMode {
    #[default]
    Normal,
    Boost,
    Overtake,
    /// Driver-selected recharge: no deployment, and the MGU-K clips the
    /// engine at full throttle from `HARVEST_MIN_SPEED`. Ends when full.
    Harvest,
}

/// Harvest mode clips the engine above this road speed, m/s.
pub const HARVEST_MIN_SPEED: f64 = 120.0 / 3.6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Battery {
    /// Stored energy, J.
    pub energy: f64,
    /// Overtake energy not yet spent, J; used before the battery.
    pub bonus: f64,
    pub mode: DeployMode,
    /// Power this tick, W: positive deploying, negative harvesting.
    pub power: f64,
    /// Energy recovered since the start of the lap, J.
    pub lap_harvest: f64,
    /// Button level last tick, so a held or repeated request acts once.
    button: bool,
    #[serde(default)]
    harvest_button: bool,
    /// Overtake already used in the current availability window.
    overtake_used: bool,
}

/// Inputs to one battery tick.
pub(crate) struct BatteryTick {
    pub button: bool,
    /// The harvest-mode toggle.
    pub harvest_button: bool,
    /// The race grants overtake mode here (earned and inside the zone).
    pub overtake_available: bool,
    /// Engage overtake mode as soon as it is available.
    pub auto_overtake: bool,
    pub throttle: f64,
    pub brake: f64,
    pub speed: f64,
    /// Inside a straight-mode zone, where the full MGU-K power is allowed.
    pub straight_zone: bool,
    /// The engine is driving the wheels this tick (in gear, not shifting).
    pub driving: bool,
    /// Brake force at full pedal, N, for regeneration.
    pub max_brake_force: f64,
    pub dt: f64,
}

impl Battery {
    pub fn full(spec: &HybridSpec) -> Self {
        Self {
            energy: spec.capacity,
            ..Self::default()
        }
    }

    pub fn charge(&self, spec: &HybridSpec) -> f64 {
        (self.energy / spec.capacity).clamp(0.0, 1.0)
    }

    pub fn new_lap(&mut self) {
        self.lap_harvest = 0.0;
    }

    /// Advances one tick; returns the electric drive power at the wheels, W,
    /// negative while super clipping.
    pub(crate) fn update(&mut self, spec: &HybridSpec, tick: &BatteryTick) -> f64 {
        let press = tick.button && !self.button;
        self.button = tick.button;
        let harvest_press = tick.harvest_button && !self.harvest_button;
        self.harvest_button = tick.harvest_button;
        if harvest_press {
            self.mode = if self.mode == DeployMode::Harvest {
                DeployMode::Normal
            } else {
                DeployMode::Harvest
            };
        }
        if !tick.overtake_available {
            self.overtake_used = false;
            if self.mode == DeployMode::Overtake {
                self.mode = DeployMode::Normal;
            }
            self.bonus = 0.0;
        }
        // Harvesting is a deliberate choice: automatic overtake waits for it.
        let engage = tick.overtake_available
            && !self.overtake_used
            && (press || tick.auto_overtake && self.mode != DeployMode::Harvest);
        if engage {
            self.mode = DeployMode::Overtake;
            self.bonus = spec.overtake_energy;
            self.overtake_used = true;
        } else if press {
            self.mode = match self.mode {
                DeployMode::Normal | DeployMode::Harvest => DeployMode::Boost,
                DeployMode::Boost => DeployMode::Normal,
                // Overtake runs to the end of its zone.
                DeployMode::Overtake => DeployMode::Overtake,
            };
        }
        if tick.brake > 0.05 && self.mode == DeployMode::Boost {
            self.mode = DeployMode::Normal;
        }

        let overtake = self.mode == DeployMode::Overtake;
        let limit = if tick.straight_zone || overtake {
            spec.mguk_power
        } else {
            spec.corner_power
        };
        let normal = ((self.charge(spec) - spec.reserve) / (spec.full_deploy - spec.reserve))
            .clamp(0.0, 1.0);
        let level = match self.mode {
            DeployMode::Normal => normal,
            DeployMode::Boost => 1.0,
            // Overtake mode spends its own allowance at full power, then
            // deploys as normal: it must not drain the driver's battery.
            DeployMode::Overtake if self.bonus > 0.0 => 1.0,
            DeployMode::Overtake => normal,
            DeployMode::Harvest => 0.0,
        };
        // Full-power overtake deployment draws on the allowance alone.
        let available = if overtake && self.bonus > 0.0 {
            self.bonus / tick.dt
        } else {
            (self.energy + self.bonus) / tick.dt
        };
        let deploy = if tick.driving && tick.throttle > 0.0 {
            (limit * spec.taper(tick.speed, overtake) * level * tick.throttle).min(available)
        } else {
            0.0
        };
        let from_bonus = (deploy * tick.dt).min(self.bonus);
        self.bonus -= from_bonus;
        self.energy -= deploy * tick.dt - from_bonus;
        if self.mode == DeployMode::Boost && self.energy <= 1.0 {
            self.mode = DeployMode::Normal;
        }

        // Recover energy under braking (rear axle) and off the throttle.
        let regen = (tick.brake * tick.max_brake_force * 0.4 * tick.speed).min(spec.regen_power);
        let coast = if tick.brake <= 0.05 && tick.speed > 15.0 {
            spec.coast_power * (1.0 - tick.throttle).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let full_throttle = tick.driving && tick.throttle >= 0.99;
        let clip = match self.mode {
            DeployMode::Normal if full_throttle && self.charge(spec) < spec.full_deploy => {
                spec.clip_power
                    * ((tick.speed - spec.taper_start) / (spec.taper_end - spec.taper_start))
                        .clamp(0.0, 1.0)
            }
            DeployMode::Harvest if full_throttle && tick.speed > HARVEST_MIN_SPEED => {
                spec.clip_power
            }
            _ => 0.0,
        };
        let room = (spec.capacity - self.energy)
            .min(spec.lap_harvest - self.lap_harvest)
            .max(0.0);
        let harvest = ((regen + coast + clip) * tick.dt).min(room);
        // Clipping costs drive only for the energy actually stored.
        let clipped = (harvest / tick.dt - regen - coast).max(0.0).min(clip);
        self.energy += harvest;
        self.lap_harvest += harvest;
        self.power = deploy - harvest / tick.dt;
        if self.mode == DeployMode::Harvest && self.charge(spec) >= 0.995 {
            self.mode = DeployMode::Normal;
        }
        deploy - clipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> HybridSpec {
        crate::car::spec::gen26_hybrid()
    }

    fn tick(button: bool, available: bool, throttle: f64, brake: f64, speed: f64) -> BatteryTick {
        BatteryTick {
            button,
            harvest_button: false,
            overtake_available: available,
            auto_overtake: false,
            throttle,
            brake,
            speed,
            straight_zone: true,
            driving: true,
            max_brake_force: 50_000.0,
            dt: 1.0 / 360.0,
        }
    }

    #[test]
    fn taper_cuts_power_at_very_high_speed_later_in_overtake_mode() {
        let spec = spec();
        assert_eq!(spec.taper(250.0 / 3.6, false), 1.0);
        assert!(spec.taper(330.0 / 3.6, false) < 0.5);
        assert_eq!(spec.taper(330.0 / 3.6, true), 1.0);
        assert_eq!(spec.taper(360.0 / 3.6, true), 0.0);
    }

    #[test]
    fn button_toggles_boost_and_braking_cancels_it() {
        let spec = spec();
        let mut battery = Battery::full(&spec);
        battery.update(&spec, &tick(true, false, 1.0, 0.0, 60.0));
        assert_eq!(battery.mode, DeployMode::Boost);
        // Holding the button does not toggle again.
        battery.update(&spec, &tick(true, false, 1.0, 0.0, 60.0));
        assert_eq!(battery.mode, DeployMode::Boost);
        battery.update(&spec, &tick(false, false, 1.0, 0.0, 60.0));
        battery.update(&spec, &tick(true, false, 1.0, 0.0, 60.0));
        assert_eq!(battery.mode, DeployMode::Normal);
        battery.update(&spec, &tick(false, false, 1.0, 0.0, 60.0));
        battery.update(&spec, &tick(true, false, 1.0, 0.0, 60.0));
        battery.update(&spec, &tick(false, false, 0.0, 0.6, 60.0));
        assert_eq!(battery.mode, DeployMode::Normal);
    }

    #[test]
    fn boost_deploys_more_than_normal_at_low_charge() {
        let spec = spec();
        let mut normal = Battery::full(&spec);
        normal.energy = spec.capacity * 0.3;
        let mut boost = normal;
        let p_normal = normal.update(&spec, &tick(false, false, 1.0, 0.0, 60.0));
        let p_boost = boost.update(&spec, &tick(true, false, 1.0, 0.0, 60.0));
        assert!(p_boost > p_normal * 1.2, "{p_boost} {p_normal}");
        assert_eq!(p_boost, spec.mguk_power);
    }

    #[test]
    fn overtake_spends_bonus_first_and_ends_with_its_zone() {
        let spec = spec();
        let mut battery = Battery::full(&spec);
        battery.energy = spec.capacity * 0.5;
        let before = battery.energy;
        battery.update(&spec, &tick(true, true, 1.0, 0.0, 80.0));
        assert_eq!(battery.mode, DeployMode::Overtake);
        assert!(battery.bonus < spec.overtake_energy);
        assert_eq!(battery.energy, before);
        // Pressing again inside the zone does not cancel it.
        battery.update(&spec, &tick(false, true, 1.0, 0.0, 80.0));
        battery.update(&spec, &tick(true, true, 1.0, 0.0, 80.0));
        assert_eq!(battery.mode, DeployMode::Overtake);
        battery.update(&spec, &tick(false, false, 1.0, 0.0, 80.0));
        assert_eq!(battery.mode, DeployMode::Normal);
        assert_eq!(battery.bonus, 0.0);
    }

    #[test]
    fn overtake_spends_only_its_allowance_at_full_power() {
        let spec = spec();
        let mut battery = Battery::full(&spec);
        battery.energy = spec.capacity * 0.2;
        let before = battery.energy;
        // A whole lap of overtake mode at full throttle on a straight.
        for i in 0..360 * 60 {
            battery.update(&spec, &tick(i == 0, true, 1.0, 0.0, 70.0));
        }
        assert_eq!(battery.mode, DeployMode::Overtake);
        // Below the reserve, normal deployment is off: only the allowance was spent.
        assert!(
            battery.energy >= before - 1.0,
            "{} < {before}",
            battery.energy
        );
    }

    #[test]
    fn harvest_mode_recharges_at_full_throttle_and_ends_when_full() {
        let spec = spec();
        let mut battery = Battery::full(&spec);
        battery.energy = spec.capacity * 0.1;
        let mut press = tick(false, false, 1.0, 0.0, 70.0);
        press.harvest_button = true;
        battery.update(&spec, &press);
        assert_eq!(battery.mode, DeployMode::Harvest);
        let mut electric = 0.0;
        for _ in 0..360 * 5 {
            electric = battery.update(&spec, &tick(false, false, 1.0, 0.0, 70.0));
        }
        assert!(electric < -spec.clip_power * 0.99, "clipping: {electric}");
        assert!(battery.charge(&spec) > 0.4, "{}", battery.charge(&spec));
        for _ in 0..360 * 30 {
            battery.update(&spec, &tick(false, false, 1.0, 0.0, 70.0));
        }
        assert_eq!(battery.mode, DeployMode::Normal);
        // The toggle switches it off again too.
        battery.energy = spec.capacity * 0.5;
        battery.update(&spec, &press);
        battery.update(&spec, &tick(false, false, 1.0, 0.0, 70.0));
        battery.update(&spec, &press);
        assert_eq!(battery.mode, DeployMode::Normal);
    }

    #[test]
    fn braking_harvests_within_the_lap_limit() {
        let spec = spec();
        let mut battery = Battery::full(&spec);
        battery.energy = 0.0;
        for _ in 0..360 * 60 {
            battery.update(&spec, &tick(false, false, 0.0, 1.0, 70.0));
        }
        assert!((battery.lap_harvest - spec.lap_harvest.min(spec.capacity)).abs() < 1.0);
        battery.new_lap();
        assert_eq!(battery.lap_harvest, 0.0);
    }

    #[test]
    fn active_aero_opens_on_a_straight_and_closes_for_braking_or_a_corner() {
        let mut aero = ActiveAero {
            allowed: true,
            ..ActiveAero::default()
        };
        let dt = 1.0 / 360.0;
        for _ in 0..360 {
            aero.update(1.0, 0.0, 2.0, 80.0, dt);
        }
        assert_eq!(aero.position, 1.0);
        aero.update(0.0, 0.5, 2.0, 80.0, dt);
        assert!(!aero.straight);
        let mut aero = ActiveAero {
            allowed: true,
            ..ActiveAero::default()
        };
        aero.update(1.0, 0.0, 25.0, 80.0, dt);
        assert!(!aero.straight);
        aero.allowed = false;
        aero.update(1.0, 0.0, 0.0, 80.0, dt);
        assert!(!aero.straight);
    }
}
