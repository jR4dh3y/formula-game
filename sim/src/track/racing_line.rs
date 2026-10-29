//! A minimum-curvature racing line and the speed a car can carry along it.

use super::{Station, STATION_SPACING};
use glam::DVec2;

/// Distance kept between the racing line and the asphalt edge.
const EDGE_MARGIN: f64 = 1.3;
const AIR_DENSITY: f64 = 1.225;
const GRAVITY: f64 = 9.81;

#[derive(Clone, Debug)]
pub struct RacingLine {
    /// Lateral offset from the centerline per station, positive left.
    pub offsets: Vec<f64>,
    /// Signed curvature of the line per station (1/m, positive turning left).
    pub curvature: Vec<f64>,
}

impl RacingLine {
    pub fn compute(stations: &[Station]) -> Self {
        let count = stations.len();
        let centers: Vec<DVec2> = stations
            .iter()
            .map(|station| DVec2::new(station.position.x, station.position.z))
            .collect();
        let lefts: Vec<DVec2> = stations.iter().map(Station::left).collect();
        let limits: Vec<f64> = stations
            .iter()
            .map(|station| (station.half_width - EDGE_MARGIN).max(0.0))
            .collect();
        let mut offsets = vec![0.0; count];
        // Coarse-to-fine relaxation: long stencils straighten the line across
        // whole corners quickly, short ones remove local wiggles.
        for (stride, iterations) in [(24, 300), (12, 300), (6, 300), (3, 300), (1, 200)] {
            for _ in 0..iterations {
                for index in 0..count {
                    let previous = (index + count - stride) % count;
                    let next = (index + stride) % count;
                    let a = centers[previous] + lefts[previous] * offsets[previous];
                    let b = centers[next] + lefts[next] * offsets[next];
                    let current = centers[index] + lefts[index] * offsets[index];
                    let shift = ((a + b) * 0.5 - current).dot(lefts[index]);
                    offsets[index] =
                        (offsets[index] + shift * 0.6).clamp(-limits[index], limits[index]);
                }
            }
        }
        let points: Vec<DVec2> = (0..count)
            .map(|index| centers[index] + lefts[index] * offsets[index])
            .collect();
        let raw: Vec<f64> = (0..count)
            .map(|index| {
                signed_curvature(
                    points[(index + count - 3) % count],
                    points[index],
                    points[(index + 3) % count],
                )
            })
            .collect();
        let curvature = (0..count)
            .map(|index| {
                (-2..=2_isize)
                    .map(|offset| {
                        raw[(index as isize + offset).rem_euclid(count as isize) as usize]
                    })
                    .sum::<f64>()
                    / 5.0
            })
            .collect();
        Self { offsets, curvature }
    }

    /// Target speed per station for a car with the given performance.
    pub fn speed_profile(&self, car: &PerformanceEnvelope) -> Vec<f64> {
        let count = self.curvature.len();
        let downforce = 0.5 * AIR_DENSITY * car.lift_area;
        let drag = 0.5 * AIR_DENSITY * car.drag_area;
        let top_speed = (car.power / drag).cbrt();
        let lateral_limit = |curvature: f64| {
            let curvature = curvature.abs();
            let denominator = car.mass * curvature - car.grip * downforce;
            if denominator <= 1e-9 {
                top_speed
            } else {
                (car.grip * car.mass * GRAVITY / denominator)
                    .sqrt()
                    .min(top_speed)
            }
        };
        let mut speed: Vec<f64> = self.curvature.iter().map(|&k| lateral_limit(k)).collect();
        let total_grip = |v: f64| car.grip * (GRAVITY + downforce * v * v / car.mass);
        let remaining = |v: f64, curvature: f64| {
            let total = total_grip(v);
            let lateral = (v * v * curvature.abs()).min(total);
            (total * total - lateral * lateral).sqrt()
        };
        // Two laps of passes so the profile is consistent across the start line.
        for _ in 0..2 {
            for step in 0..count {
                let index = (count * 2 - 1 - step) % count;
                let next = (index + 1) % count;
                let v = speed[next];
                let decel = remaining(v, self.curvature[index]) * car.brake_fraction
                    + drag * v * v / car.mass;
                speed[index] = speed[index].min((v * v + 2.0 * decel * STATION_SPACING).sqrt());
            }
            for index in 0..count {
                let next = (index + 1) % count;
                let v = speed[index].max(1.0);
                let traction = remaining(v, self.curvature[index]) * 0.7;
                let accel = (car.power / (car.mass * v)).min(traction) - drag * v * v / car.mass;
                speed[next] =
                    speed[next].min((v * v + 2.0 * accel.max(0.0) * STATION_SPACING).sqrt());
            }
        }
        speed
    }
}

/// The handful of numbers the speed profile needs from a car.
#[derive(Clone, Copy, Debug)]
pub struct PerformanceEnvelope {
    pub mass: f64,
    /// Effective tyre friction coefficient.
    pub grip: f64,
    /// Downforce coefficient times reference area (ClA).
    pub lift_area: f64,
    /// Drag coefficient times reference area (CdA).
    pub drag_area: f64,
    /// Engine power at the wheels, W.
    pub power: f64,
    /// Share of the friction budget used under braking.
    pub brake_fraction: f64,
}

fn signed_curvature(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    let ab = b - a;
    let bc = c - b;
    let ac = c - a;
    let denominator = ab.length() * bc.length() * ac.length();
    if denominator < 1e-9 {
        return 0.0;
    }
    // With +y up, the driver's left is (t.z, -t.x) in (x, z), so a left turn
    // makes the (x, z) perp-dot negative.
    -2.0 * ab.perp_dot(bc) / denominator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::oval_track;

    #[test]
    fn line_uses_the_track_width_and_respects_the_edges() {
        let track = oval_track();
        let line = &track.racing_line;
        let max_offset = line.offsets.iter().fold(0.0_f64, |m, o| m.max(o.abs()));
        assert!(max_offset > 3.0, "line should swing wide: {max_offset}");
        for (offset, station) in line.offsets.iter().zip(&track.stations) {
            assert!(offset.abs() <= station.half_width - EDGE_MARGIN + 1e-9);
        }
    }

    #[test]
    fn corners_are_slower_than_straights() {
        let track = oval_track();
        let profile = track.racing_line.speed_profile(&PerformanceEnvelope {
            mass: 800.0,
            grip: 1.6,
            lift_area: 4.0,
            drag_area: 1.3,
            power: 650_000.0,
            brake_fraction: 0.95,
        });
        let corner = profile[track.station_at(250.0)];
        let straight = profile[track.station_at(100.0)];
        assert!(corner < straight, "{corner} {straight}");
        // A 60 m radius corner is taken somewhere around 100-150 km/h.
        assert!((25.0..50.0).contains(&corner), "{corner}");
        // The line turns the same way as the track on the curved ends.
        let curvature = track.racing_line.curvature[track.station_at(250.0)];
        let left_turn = {
            let a = track.frame_at(240.0).tangent;
            let b = track.frame_at(260.0).tangent;
            // Turning left rotates the tangent toward the left vector.
            (b - a).dot(track.frame_at(250.0).left()) > 0.0
        };
        assert_eq!(curvature > 0.0, left_turn);
    }
}
