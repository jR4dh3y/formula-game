//! Static car descriptions, player-adjustable setup and tyre compounds.
//!
//! Numbers are tuned for believable racing trade-offs, not measured F1 data.

use glam::DVec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compound {
    Soft,
    Medium,
    Hard,
}

impl Compound {
    pub const ALL: [Compound; 3] = [Compound::Soft, Compound::Medium, Compound::Hard];

    pub fn label(self) -> &'static str {
        match self {
            Self::Soft => "SOFT",
            Self::Medium => "MEDIUM",
            Self::Hard => "HARD",
        }
    }

    pub fn properties(self) -> CompoundProperties {
        match self {
            Self::Soft => CompoundProperties {
                grip: 1.0,
                optimal_temperature: 95.0,
                window: 12.0,
                wear_rate: 1.7,
                warm_up: 1.25,
            },
            Self::Medium => CompoundProperties {
                grip: 0.975,
                optimal_temperature: 100.0,
                window: 15.0,
                wear_rate: 1.0,
                warm_up: 1.0,
            },
            Self::Hard => CompoundProperties {
                grip: 0.95,
                optimal_temperature: 106.0,
                window: 18.0,
                wear_rate: 0.6,
                warm_up: 0.8,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompoundProperties {
    /// Friction relative to the car's base tyre friction.
    pub grip: f64,
    /// Peak-grip temperature in °C.
    pub optimal_temperature: f64,
    /// Half-width of the comfortable temperature window in °C.
    pub window: f64,
    pub wear_rate: f64,
    /// How quickly the tyre gains heat.
    pub warm_up: f64,
}

/// Player-adjustable setup, chosen before a session.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    /// Front wing level, 0 (low drag) to 10 (max downforce).
    pub front_wing: f64,
    /// Rear wing level, 0 (low drag) to 10 (max downforce).
    pub rear_wing: f64,
    pub compound: Compound,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            front_wing: 5.0,
            rear_wing: 5.0,
            compound: Compound::Medium,
        }
    }
}

impl Setup {
    pub const WING_MAX: f64 = 10.0;

    pub fn clamped(self) -> Self {
        Self {
            front_wing: self.front_wing.clamp(0.0, Self::WING_MAX),
            rear_wing: self.rear_wing.clamp(0.0, Self::WING_MAX),
            ..self
        }
    }

    /// Front downforce coefficient-area.
    pub fn front_lift_area(&self, aero: &AeroSpec) -> f64 {
        aero.front_lift_area + aero.front_wing_lift * self.front_wing
    }

    pub fn rear_lift_area(&self, aero: &AeroSpec) -> f64 {
        aero.rear_lift_area + aero.rear_wing_lift * self.rear_wing
    }

    pub fn drag_area(&self, aero: &AeroSpec) -> f64 {
        aero.drag_area
            + aero.front_wing_drag * self.front_wing
            + aero.rear_wing_drag * self.rear_wing
    }
}

#[derive(Clone, Debug)]
pub struct AeroSpec {
    /// ClA of the front with the wing at zero.
    pub front_lift_area: f64,
    pub rear_lift_area: f64,
    /// ClA gained per wing step.
    pub front_wing_lift: f64,
    pub rear_wing_lift: f64,
    /// CdA with both wings at zero.
    pub drag_area: f64,
    pub front_wing_drag: f64,
    pub rear_wing_drag: f64,
}

#[derive(Clone, Debug)]
pub struct EngineSpec {
    /// Peak power at the wheels, W.
    pub max_power: f64,
    pub peak_power_rpm: f64,
    pub rev_limit: f64,
    pub idle_rpm: f64,
    /// Overall ratio (gearbox times final drive) per forward gear.
    pub gear_ratios: Vec<f64>,
    pub reverse_ratio: f64,
    pub upshift_time: f64,
    pub downshift_time: f64,
}

/// One wheel in model space (+Z forward, +X left, +Y up, origin on the ground
/// midway between the axles).
#[derive(Clone, Copy, Debug)]
pub struct WheelSpec {
    pub position: DVec3,
    pub radius: f64,
    pub width: f64,
}

/// Wheel placement and body size read from a prepared car's `car.json`.
#[derive(Clone, Debug)]
pub struct CarGeometry {
    pub wheels: [WheelSpec; 4],
    pub length: f64,
    pub width: f64,
}

#[derive(Deserialize)]
struct CarFile {
    wheels: CarFileWheels,
    extents: CarFileExtents,
}

#[derive(Deserialize)]
struct CarFileWheels {
    fl: CarFileWheel,
    fr: CarFileWheel,
    rl: CarFileWheel,
    rr: CarFileWheel,
}

#[derive(Deserialize)]
struct CarFileWheel {
    centre: [f64; 3],
    radius: f64,
    width: f64,
}

#[derive(Deserialize)]
struct CarFileExtents {
    min: [f64; 3],
    max: [f64; 3],
}

impl CarGeometry {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let file: CarFile = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let wheel = |source: &CarFileWheel| WheelSpec {
            position: DVec3::from(source.centre),
            radius: source.radius,
            width: source.width,
        };
        Ok(Self {
            wheels: [
                wheel(&file.wheels.fl),
                wheel(&file.wheels.fr),
                wheel(&file.wheels.rl),
                wheel(&file.wheels.rr),
            ],
            length: file.extents.max[2] - file.extents.min[2],
            width: file.extents.max[0] - file.extents.min[0],
        })
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        Self::from_json(&bytes).map_err(|error| format!("{}: {error}", path.display()))
    }
}

#[derive(Clone, Debug)]
pub struct CarSpec {
    pub name: String,
    pub mass: f64,
    pub yaw_inertia: f64,
    pub cg_height: f64,
    /// Fraction of static weight on the front axle.
    pub front_weight: f64,
    /// fl, fr, rl, rr.
    pub wheels: [WheelSpec; 4],
    /// Body length and width used for collisions.
    pub length: f64,
    pub width: f64,
    pub engine: EngineSpec,
    pub aero: AeroSpec,
    /// Peak tyre friction coefficient on clean asphalt.
    pub tyre_friction: f64,
    /// Peak lateral slip angle, rad.
    pub peak_slip_angle: f64,
    /// Extra friction of the wider rear tyres relative to the fronts.
    pub rear_grip: f64,
    /// Maximum total brake force, N.
    pub max_brake_force: f64,
    pub brake_bias: f64,
}

impl CarSpec {
    pub fn wheelbase(&self) -> f64 {
        self.wheels[0].position.z - self.wheels[2].position.z
    }

    /// Model-space z of the centre of gravity.
    pub fn cg_z(&self) -> f64 {
        self.wheels[2].position.z + self.wheelbase() * self.front_weight
    }

    /// Red Bull RB22-style 2026 car: heavier, very strong power unit, less downforce.
    pub fn rb22_with(geometry: &CarGeometry) -> Self {
        Self::rb22(geometry.wheels, geometry.length, geometry.width)
    }

    pub fn rb6_with(geometry: &CarGeometry) -> Self {
        Self::rb6(geometry.wheels, geometry.length, geometry.width)
    }

    pub fn rb22(wheels: [WheelSpec; 4], length: f64, width: f64) -> Self {
        Self {
            name: "RB22".to_string(),
            mass: 800.0,
            yaw_inertia: 1150.0,
            cg_height: 0.27,
            front_weight: 0.455,
            wheels,
            length,
            width,
            engine: EngineSpec {
                max_power: 690_000.0,
                peak_power_rpm: 12_400.0,
                rev_limit: 13_500.0,
                idle_rpm: 4_500.0,
                gear_ratios: gear_set(20.0, 5.2, 8),
                reverse_ratio: 24.0,
                upshift_time: 0.035,
                downshift_time: 0.06,
            },
            aero: AeroSpec {
                front_lift_area: 1.2,
                rear_lift_area: 1.55,
                front_wing_lift: 0.12,
                rear_wing_lift: 0.16,
                drag_area: 0.98,
                front_wing_drag: 0.022,
                rear_wing_drag: 0.042,
            },
            tyre_friction: 1.72,
            peak_slip_angle: 0.095,
            rear_grip: 1.1,
            max_brake_force: 52_000.0,
            brake_bias: 0.57,
        }
    }

    /// Red Bull RB6-style 2010 V8 car: lighter, screaming V8, high downforce.
    pub fn rb6(wheels: [WheelSpec; 4], length: f64, width: f64) -> Self {
        Self {
            name: "RB6".to_string(),
            mass: 720.0,
            yaw_inertia: 1000.0,
            cg_height: 0.28,
            front_weight: 0.46,
            wheels,
            length,
            width,
            engine: EngineSpec {
                max_power: 570_000.0,
                peak_power_rpm: 17_200.0,
                rev_limit: 18_000.0,
                idle_rpm: 5_000.0,
                gear_ratios: gear_set(25.5, 6.7, 7),
                reverse_ratio: 30.0,
                upshift_time: 0.045,
                downshift_time: 0.07,
            },
            aero: AeroSpec {
                front_lift_area: 1.35,
                rear_lift_area: 1.75,
                front_wing_lift: 0.12,
                rear_wing_lift: 0.16,
                drag_area: 0.9,
                front_wing_drag: 0.022,
                rear_wing_drag: 0.042,
            },
            tyre_friction: 1.75,
            peak_slip_angle: 0.09,
            rear_grip: 1.1,
            max_brake_force: 48_000.0,
            brake_bias: 0.56,
        }
    }
}

/// Overall ratios from `first` to `top`, slightly closer together at the top.
fn gear_set(first: f64, top: f64, gears: usize) -> Vec<f64> {
    (0..gears)
        .map(|index| {
            let t = index as f64 / (gears - 1) as f64;
            let t = t.powf(0.88);
            first * (top / first).powf(t)
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn test_wheels() -> [WheelSpec; 4] {
    let wheel = |x: f64, z: f64, radius: f64| WheelSpec {
        position: DVec3::new(x, radius, z),
        radius,
        width: 0.36,
    };
    [
        wheel(0.77, 1.7, 0.355),
        wheel(-0.77, 1.7, 0.355),
        wheel(0.74, -1.7, 0.355),
        wheel(-0.74, -1.7, 0.355),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wings_trade_downforce_for_drag() {
        let spec = CarSpec::rb22(test_wheels(), 5.4, 1.9);
        let low = Setup {
            front_wing: 1.0,
            rear_wing: 1.0,
            ..Setup::default()
        };
        let high = Setup {
            front_wing: 9.0,
            rear_wing: 9.0,
            ..Setup::default()
        };
        let lift = |s: &Setup| s.front_lift_area(&spec.aero) + s.rear_lift_area(&spec.aero);
        assert!(lift(&high) > lift(&low) * 1.4);
        assert!(high.drag_area(&spec.aero) > low.drag_area(&spec.aero) * 1.2);
    }

    #[test]
    fn gears_are_monotonic() {
        let ratios = gear_set(20.0, 5.2, 8);
        assert_eq!(ratios.len(), 8);
        assert!(ratios.windows(2).all(|pair| pair[0] > pair[1]));
        assert!((ratios[7] - 5.2).abs() < 1e-9);
    }
}
