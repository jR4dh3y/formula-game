//! A minimum-curvature racing line and the speed a car can carry along it.

use super::{Station, STATION_SPACING};
use glam::DVec2;

/// Distance kept between the racing line and the asphalt edge.
const EDGE_MARGIN: f64 = 1.0;
const AIR_DENSITY: f64 = 1.225;
const GRAVITY: f64 = 9.81;

#[derive(Clone, Debug)]
pub struct RacingLine {
    /// Lateral offset from the centerline per station, positive left.
    pub offsets: Vec<f64>,
    /// Signed curvature of the line per station (1/m, positive turning left).
    pub curvature: Vec<f64>,
    /// Vertical curvature of the road per station (1/m, positive in a dip).
    /// Compressions press the car into the track; crests unload it.
    pub vertical_curvature: Vec<f64>,
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
        // The relaxation above converges to the shortest path, which is
        // straight between apexes and puts all the turning into short arcs
        // at the kerbs. Penalising curvature instead spreads a bend, or a
        // sequence of bends, over all the room the track gives; esses like
        // Eau Rouge become one long sweep rather than three sharp flicks.
        for (stride, iterations) in [(16, 800), (8, 600), (4, 300), (2, 200)] {
            for _ in 0..iterations {
                smooth_curvature(&centers, &lefts, &limits, &mut offsets, stride);
            }
        }
        Self::from_offsets(stations, offsets)
    }

    /// The path a fixed lateral offset from the centerline traces, with its
    /// own curvature, so a car holding a lane plans speeds for that lane.
    pub fn constant_offset(stations: &[Station], lateral: f64) -> Self {
        Self::from_offsets(
            stations,
            stations
                .iter()
                .map(|station| {
                    let limit = (station.half_width - 1.4).max(0.);
                    lateral.clamp(-limit, limit)
                })
                .collect(),
        )
    }

    /// A smooth left/right lane with room for a car between its centre and the edge.
    /// The inside lane depends on the approaching corner, not on a fixed lap label.
    pub fn side_lane(&self, stations: &[Station], side: f64) -> Self {
        let mut offsets: Vec<_> = stations
            .iter()
            .zip(&self.offsets)
            .map(|(station, optimal)| {
                let limit = (station.half_width - 1.4).max(0.);
                (optimal * 0.12 + side * limit.min(3.4)).clamp(-limit, limit)
            })
            .collect();
        for _ in 0..80 {
            let previous = offsets.clone();
            let count = offsets.len();
            for (i, offset) in offsets.iter_mut().enumerate() {
                let limit = (stations[i].half_width - 1.4).max(0.);
                *offset = ((previous[(i + count - 1) % count]
                    + previous[i] * 2.
                    + previous[(i + 1) % count])
                    * 0.25)
                    .clamp(-limit, limit);
            }
        }
        Self::from_offsets(stations, offsets)
    }

    /// Interpolated lateral target, including across the start/finish seam.
    pub fn lateral_at(&self, s: f64) -> f64 {
        let station = (s / STATION_SPACING).rem_euclid(self.offsets.len() as f64);
        let i = station.floor() as usize;
        let t = station.fract();
        self.offsets[i] * (1. - t) + self.offsets[(i + 1) % self.offsets.len()] * t
    }

    fn from_offsets(stations: &[Station], offsets: Vec<f64>) -> Self {
        let count = stations.len();
        let centers: Vec<DVec2> = stations
            .iter()
            .map(|station| DVec2::new(station.position.x, station.position.z))
            .collect();
        let lefts: Vec<DVec2> = stations.iter().map(Station::left).collect();
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
        let heights: Vec<f64> = stations.iter().map(|station| station.position.y).collect();
        let vertical_curvature = vertical_curvature(&heights);
        Self {
            offsets,
            curvature,
            vertical_curvature,
        }
    }

    /// Target speed per station for a car with the given performance.
    pub fn speed_profile(&self, car: &PerformanceEnvelope) -> Vec<f64> {
        self.passes(self.lateral_limits(car), car)
    }

    /// Like `speed_profile`, but bends that `capable` takes without lifting
    /// stay flat: a cautious plan should cost time where braking is needed,
    /// not invent a lift where the car has the grip.
    pub fn speed_profile_keeping_flat(
        &self,
        car: &PerformanceEnvelope,
        capable: &PerformanceEnvelope,
    ) -> Vec<f64> {
        let capable_limits = self.lateral_limits(capable);
        let capable_speed = self.passes(capable_limits.clone(), capable);
        let top_speed = (capable.power / (0.5 * AIR_DENSITY * capable.drag_area)).cbrt();
        let mut limits = self.lateral_limits(car);
        for (index, limit) in limits.iter_mut().enumerate() {
            // A flat bend is power-limited, rather than cornering-limited.
            if capable_limits[index] >= top_speed * 0.999 {
                *limit = limit.max(capable_speed[index]);
            }
        }
        self.passes(limits, car)
    }

    /// Per-station cornering limit, before braking and acceleration passes.
    fn lateral_limits(&self, car: &PerformanceEnvelope) -> Vec<f64> {
        let count = self.curvature.len();
        let downforce = 0.5 * AIR_DENSITY * car.lift_area;
        let drag = 0.5 * AIR_DENSITY * car.drag_area;
        let top_speed = (car.power / drag).cbrt();
        // Normal load per unit mass is g + v²·(vertical curvature + downforce/m);
        // the car model loads its tyres the same way through its suspension.
        // Heavily loaded tyres grip proportionally less (load sensitivity).
        let vertical = |index: usize| self.vertical_curvature[index];
        let normal = |v: f64, index: usize| {
            (GRAVITY + downforce * v * v / car.mass + vertical(index) * v * v).max(GRAVITY * 0.3)
        };
        let grip = |v: f64, index: usize| {
            let extra_load = normal(v, index) / GRAVITY - 1.0;
            car.grip * (1.0 - car.load_sensitivity * extra_load).clamp(0.6, 1.0)
        };
        let lateral_limit = |index: usize| {
            let curvature = self.curvature[index].abs();
            // Fixed point: the corner speed sets the load, which sets the grip.
            let mut v = top_speed;
            for _ in 0..8 {
                let mu = grip(v, index);
                let denominator =
                    car.mass * curvature - mu * (downforce + car.mass * vertical(index));
                v = if denominator <= 1e-9 {
                    top_speed
                } else {
                    (mu * car.mass * GRAVITY / denominator)
                        .sqrt()
                        .min(top_speed)
                };
            }
            v
        };
        (0..count).map(lateral_limit).collect()
    }

    /// Braking (backward) and acceleration (forward) passes over `speed`.
    fn passes(&self, mut speed: Vec<f64>, car: &PerformanceEnvelope) -> Vec<f64> {
        let count = self.curvature.len();
        let downforce = 0.5 * AIR_DENSITY * car.lift_area;
        let drag = 0.5 * AIR_DENSITY * car.drag_area;
        let vertical = |index: usize| self.vertical_curvature[index];
        let normal = |v: f64, index: usize| {
            (GRAVITY + downforce * v * v / car.mass + vertical(index) * v * v).max(GRAVITY * 0.3)
        };
        let grip = |v: f64, index: usize| {
            let extra_load = normal(v, index) / GRAVITY - 1.0;
            car.grip * (1.0 - car.load_sensitivity * extra_load).clamp(0.6, 1.0)
        };
        let total_grip = |v: f64, index: usize| grip(v, index) * normal(v, index);
        let remaining = |v: f64, curvature: f64, index: usize| {
            let total = total_grip(v, index);
            let lateral = (v * v * curvature.abs()).min(total);
            (total * total - lateral * lateral).sqrt()
        };
        // Two laps of passes so the profile is consistent across the start line.
        for _ in 0..2 {
            for step in 0..count {
                let index = (count * 2 - 1 - step) % count;
                let next = (index + 1) % count;
                let v = speed[next];
                let decel = remaining(v, self.curvature[index], index) * car.brake_fraction
                    + drag * v * v / car.mass;
                speed[index] = speed[index].min((v * v + 2.0 * decel * STATION_SPACING).sqrt());
            }
            for index in 0..count {
                let next = (index + 1) % count;
                let v = speed[index].max(1.0);
                let traction = remaining(v, self.curvature[index], index) * 0.7;
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
    /// Fractional grip lost per extra g of tyre load (downforce, compression).
    pub load_sensitivity: f64,
}

/// Second derivative of road height along the lap, smoothed over ~20 m so
/// collision-mesh facets do not read as bumps.
fn vertical_curvature(heights: &[f64]) -> Vec<f64> {
    const HALF_SPAN: usize = 5;
    let count = heights.len();
    let at = |index: isize| heights[index.rem_euclid(count as isize) as usize];
    let span = HALF_SPAN as f64 * STATION_SPACING;
    let raw: Vec<f64> = (0..count as isize)
        .map(|index| {
            (at(index - HALF_SPAN as isize) - 2.0 * at(index) + at(index + HALF_SPAN as isize))
                / (span * span)
        })
        .collect();
    (0..count as isize)
        .map(|index| {
            (-(HALF_SPAN as isize)..=HALF_SPAN as isize)
                .map(|offset| raw[(index + offset).rem_euclid(count as isize) as usize])
                .sum::<f64>()
                / (2 * HALF_SPAN + 1) as f64
        })
        .collect()
}

/// Relaxation step; larger steps diverge with the curvature weighting.
const STEP: f64 = 0.3;

/// One Gauss-Seidel sweep of reweighted curvature minimisation at `stride`.
fn smooth_curvature(
    centers: &[DVec2],
    lefts: &[DVec2],
    limits: &[f64],
    offsets: &mut [f64],
    stride: usize,
) {
    let count = centers.len();
    let point = |offsets: &[f64], i: usize| centers[i] + lefts[i] * offsets[i];
    let wrap = |i: isize| i.rem_euclid(count as isize) as usize;
    let span = (stride as f64 * STATION_SPACING).powi(2);
    // Second difference, and a weight growing with its curvature so the
    // tightest parts of the line carry the most cost.
    let bend = |offsets: &[f64], i: usize| {
        let d2 = point(offsets, wrap(i as isize - stride as isize)) - point(offsets, i) * 2.0
            + point(offsets, wrap(i as isize + stride as isize));
        // Weighting by curvature cubed (squared curvature times |k|³) makes
        // the tightest point of the line the most expensive, so it opens up.
        let weight = ((d2.length() / span + 0.002) / 0.01).powi(3);
        (d2, weight)
    };
    for index in 0..count {
        let (d_prev, w_prev) = bend(offsets, wrap(index as isize - stride as isize));
        let (d_here, w_here) = bend(offsets, index);
        let (d_next, w_next) = bend(offsets, wrap(index as isize + stride as isize));
        let gradient = d_prev * w_prev - d_here * (2.0 * w_here) + d_next * w_next;
        let shift = -gradient.dot(lefts[index]) / (w_prev + 4.0 * w_here + w_next);
        offsets[index] = (offsets[index] + shift * STEP).clamp(-limits[index], limits[index]);
    }
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
    fn flat_bends_keep_full_speed_while_tight_corners_keep_the_pace_margin() {
        let capable = PerformanceEnvelope {
            mass: 800.,
            grip: 2.,
            lift_area: 0.,
            drag_area: 1.3,
            power: 500_000.,
            brake_fraction: 0.9,
            load_sensitivity: 0.,
        };
        let cautious = PerformanceEnvelope {
            grip: 0.8,
            ..capable
        };
        for (curvature, flat) in [(0.002, true), (0.02, false)] {
            let line = RacingLine {
                offsets: vec![0.; 100],
                curvature: vec![curvature; 100],
                vertical_curvature: vec![0.; 100],
            };
            let original = line.speed_profile(&cautious);
            let promoted = line.speed_profile_keeping_flat(&cautious, &capable);
            let full = line.speed_profile(&capable);
            if flat {
                assert!(promoted[0] > original[0] + 5.);
                assert!((promoted[0] - full[0]).abs() < 1e-6);
            } else {
                assert!((promoted[0] - original[0]).abs() < 1e-6);
            }
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
            load_sensitivity: 0.0,
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
