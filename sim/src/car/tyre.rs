//! Simplified tyre: a shared friction budget, a temperature window and wear.

use super::spec::Compound;

pub const AMBIENT_TEMPERATURE: f64 = 26.0;
/// Temperature after tyre blankets and a formation lap.
pub const START_TEMPERATURE: f64 = 78.0;

/// Thermal and wear state of one axle's pair of tyres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxleTyres {
    /// Representative tread temperature, °C.
    pub temperature: f64,
    /// 0 = new, 1 = worn out.
    pub wear: f64,
}

impl Default for AxleTyres {
    fn default() -> Self {
        Self {
            temperature: START_TEMPERATURE,
            wear: 0.0,
        }
    }
}

impl AxleTyres {
    /// Grip multiplier from temperature and wear.
    pub fn grip(&self, compound: Compound) -> f64 {
        let properties = compound.properties();
        let offset = self.temperature - properties.optimal_temperature;
        let window = properties.window;
        let thermal = if offset.abs() <= window {
            1.0 - 0.06 * (offset / window).powi(2)
        } else if offset < 0.0 {
            // Cold: falls to about 82% fifty degrees below the window.
            (0.94 - 0.12 * ((-offset - window) / 50.0)).max(0.80)
        } else {
            // Overheating: falls faster, to about 80% thirty degrees above it.
            (0.94 - 0.14 * ((offset - window) / 30.0)).max(0.78)
        };
        let wear = 1.0 - 0.22 * self.wear.powf(1.4);
        properties.grip * thermal * wear
    }

    /// Advances temperature and wear from the axle's sliding and rolling work.
    ///
    /// `slide_power` is friction force times slip speed (W) over both tyres;
    /// `rolling_power` is vertical load times speed (W).
    pub fn update(
        &mut self,
        compound: Compound,
        slide_power: f64,
        rolling_power: f64,
        speed: f64,
        dt: f64,
    ) {
        let properties = compound.properties();
        // Carcass flex heats with load and speed; airflow cools with speed. Load
        // grows with downforce, so the square root keeps steady running near the
        // bottom of the window at any speed, and sliding lifts it into the window.
        let heating =
            (slide_power * 2.2e-5 + rolling_power.max(0.0).sqrt() * 2.3e-3) * properties.warm_up;
        let cooling = (self.temperature - AMBIENT_TEMPERATURE) * (0.004 + speed * 0.0004);
        self.temperature =
            (self.temperature + (heating - cooling) * dt).clamp(AMBIENT_TEMPERATURE, 160.0);

        let overheat =
            (self.temperature - properties.optimal_temperature - properties.window).max(0.0);
        let wear_rate =
            (slide_power * 2.0e-9 + speed * 2.2e-6 + overheat * 6.0e-6) * properties.wear_rate;
        self.wear = (self.wear + wear_rate * dt).min(1.0);
    }
}

/// Lateral force shape: rises linearly to 1 at `x = 1` (the peak slip angle),
/// then sags to about 0.82 when the tyre is sliding well past the peak.
pub fn lateral_shape(x: f64) -> f64 {
    let x = x.abs();
    if x <= 1.0 {
        x * (1.5 - 0.5 * x * x)
    } else {
        0.82 + 0.18 * (-(x - 1.0) * 1.6).exp()
    }
}

/// Load sensitivity: heavily loaded tyres produce a little less friction per newton.
pub fn load_sensitivity(load: f64, nominal_load: f64) -> f64 {
    (1.0 - 0.07 * (load / nominal_load - 1.0)).clamp(0.7, 1.12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grip_peaks_in_the_window() {
        let tyre = |temperature| AxleTyres {
            temperature,
            wear: 0.0,
        };
        let optimal = tyre(100.0).grip(Compound::Medium);
        assert!(optimal > tyre(50.0).grip(Compound::Medium));
        assert!(optimal > tyre(140.0).grip(Compound::Medium));
        let worn = AxleTyres {
            temperature: 100.0,
            wear: 0.8,
        };
        assert!(worn.grip(Compound::Medium) < optimal * 0.9);
    }

    #[test]
    fn lateral_shape_is_continuous_and_peaks_at_one() {
        assert!((lateral_shape(1.0) - 1.0).abs() < 1e-9);
        assert!((lateral_shape(1.0 + 1e-9) - 1.0).abs() < 1e-6);
        assert!(lateral_shape(0.5) < 1.0);
        assert!(lateral_shape(3.0) < 0.9);
        assert!(lateral_shape(3.0) > 0.8);
    }

    #[test]
    fn sliding_heats_and_wears_tyres() {
        let mut calm = AxleTyres::default();
        let mut sliding = AxleTyres::default();
        for _ in 0..6000 {
            calm.update(Compound::Soft, 2_000.0, 4.0e5, 50.0, 0.01);
            sliding.update(Compound::Soft, 60_000.0, 4.0e5, 50.0, 0.01);
        }
        assert!(sliding.temperature > calm.temperature + 10.0);
        assert!(sliding.wear > calm.wear);
    }
}
