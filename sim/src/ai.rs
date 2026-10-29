//! Computer drivers that race rather than follow a line.

use crate::car::{Car, CarInput};
use crate::rng::Rng;
use crate::track::racing_line::PerformanceEnvelope;
use crate::track::{Track, STATION_SPACING};
use glam::DVec2;

const PLAN_INTERVAL: f64 = 0.25;

/// Personality of a computer driver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriverProfile {
    /// Fraction of the car's grip the driver dares to use, ~0.9..1.0.
    pub pace: f64,
    /// Willingness to attack and to defend, 0..1.
    pub aggression: f64,
    /// 1 = never errs; lower values make occasional small mistakes.
    pub consistency: f64,
}

/// A corner as the AI sees it: where to brake, where the apex is and which side is inside.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CornerPlan {
    brake_s: f64,
    apex_s: f64,
    /// +1 for a left-hander (inside on the left), -1 for a right-hander.
    inside: f64,
    apex_speed: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intent {
    /// Drive the racing line.
    Line,
    /// Line up a pass on the given side (+1 left) for the corner at `apex_s`.
    Attack { side: f64, apex_s: f64 },
    /// Cover the inside of the corner at `apex_s` (one move, held).
    Defend { side: f64, apex_s: f64 },
}

/// What a driver knows about another car this tick.
#[derive(Clone, Copy, Debug)]
pub struct Rival {
    /// Track distance from us to them, positive when they are ahead.
    pub gap: f64,
    pub lateral: f64,
    pub speed: f64,
    pub position: DVec2,
}

#[derive(Clone, Debug)]
pub struct AiDriver {
    pub profile: DriverProfile,
    speed_profile: Vec<f64>,
    corners: Vec<CornerPlan>,
    pub intent: Intent,
    rng: Rng,
    /// Late-braking error for the next corner, metres.
    mistake: f64,
    mistake_corner: Option<f64>,
    plan_timer: f64,
    stuck_timer: f64,
    lateral_target: f64,
    /// Tyre grip the current speed profile was planned for.
    planned_grip: f64,
}

impl AiDriver {
    pub fn new(profile: DriverProfile, car: &Car, track: &Track, seed: u64) -> Self {
        let planned_grip = tyre_grip(car);
        let speed_profile = speed_profile(profile, car, track, planned_grip);
        let corners = find_corners(track, &speed_profile);
        let mut rng = Rng::new(seed);
        // Stagger replanning so a full field never plans on the same tick.
        let plan_timer = rng.next_f64() * PLAN_INTERVAL;
        Self {
            profile,
            speed_profile,
            corners,
            intent: Intent::Line,
            rng,
            mistake: 0.0,
            mistake_corner: None,
            plan_timer,
            stuck_timer: 0.0,
            lateral_target: 0.0,
            planned_grip,
        }
    }

    /// Target speed the driver wants at lap distance `s`.
    pub fn target_speed(&self, track: &Track, s: f64) -> f64 {
        self.speed_profile[track.station_at(s)]
    }

    fn next_corner(&self, track: &Track, s: f64) -> Option<CornerPlan> {
        self.corners
            .iter()
            .copied()
            .filter(|corner| track.delta(s, corner.apex_s) > -10.0)
            .min_by(|a, b| {
                track
                    .delta(s, a.apex_s)
                    .rem_euclid(track.length)
                    .total_cmp(&track.delta(s, b.apex_s).rem_euclid(track.length))
            })
    }

    /// Seconds the car has been nearly stationary while it should be racing.
    pub fn stuck_time(&self) -> f64 {
        self.stuck_timer
    }

    /// Held on the grid: standing still is not being stuck.
    pub fn hold(&mut self) {
        self.stuck_timer = 0.0;
    }

    pub fn reset_after_recovery(&mut self) {
        self.stuck_timer = 0.0;
        self.intent = Intent::Line;
    }

    pub fn drive(&mut self, car: &Car, track: &Track, rivals: &[Rival], dt: f64) -> CarInput {
        let s = car.track_position.s;
        let speed = car.speed();
        let corner = self.next_corner(track, s);

        self.plan_timer -= dt;
        if self.plan_timer <= 0.0 {
            self.plan_timer += PLAN_INTERVAL;
            self.replan(car, track, rivals, corner);
        }
        self.stuck_timer = if speed < 2.0 {
            self.stuck_timer + dt
        } else {
            0.0
        };

        // Lateral target: racing line, or the tactical side, then leave room.
        let lookahead = (6.0 + speed * 0.42).clamp(9.0, 42.0);
        let line_lateral = track.racing_line.offsets[track.station_at(s + lookahead)];
        let ahead_frame = track.frame_at(s + lookahead);
        let limit = (ahead_frame.half_width - 1.4).max(0.0);
        let mut lateral = match self.intent {
            Intent::Line => line_lateral,
            Intent::Attack { side, .. } => side * (limit - 0.3),
            Intent::Defend { side, .. } => side * (limit - 1.2),
        };
        for rival in rivals {
            let overlapping = rival.gap.abs() < car.spec.length + 1.5;
            let close_behind_them = rival.gap > 0.0 && rival.gap < 9.0 + speed * 0.25;
            if !(overlapping || close_behind_them) {
                continue;
            }
            let separation = rival.lateral - car.track_position.lateral;
            let needed = car.spec.width + if overlapping { 1.1 } else { 0.6 };
            if separation.abs() < needed + 1.5 {
                // Keep a car's width (plus margin) away on the side we are on.
                let side = if separation > 0.0 { -1.0 } else { 1.0 };
                let bound = rival.lateral + side * needed;
                lateral = if side < 0.0 {
                    lateral.min(bound)
                } else {
                    lateral.max(bound)
                };
            }
        }
        let lateral = lateral.clamp(-limit, limit);
        self.lateral_target += (lateral - self.lateral_target) * (dt * 2.5).min(1.0);

        // Pure pursuit toward the target point.
        let target = ahead_frame.point_at(self.lateral_target);
        let offset = DVec2::new(target.x - car.position.x, target.z - car.position.z);
        let local_left = offset.dot(car.left());
        let distance = offset.length().max(1.0);
        let curvature = 2.0 * local_left / (distance * distance);
        let wheel_angle = (car.spec.wheelbase() * curvature).atan() - car.yaw_rate * 0.03;
        let steer = (wheel_angle / car.steer_limit(speed)).clamp(-1.0, 1.0);

        // Speed: profile ahead, minus dirty-air caution, adjusted for mistakes and traffic.
        let anticipation = speed * 0.12 + 2.0;
        let mut target_speed = self.target_speed(track, s + anticipation - self.mistake);
        target_speed *= 1.0 - car.aero.dirty_air() * 0.45;
        // Running wide with the wheel already wound on: lift until the line comes back.
        let curvature = track.racing_line.curvature[track.station_at(s)];
        let wide = (car.track_position.lateral - self.lateral_target) * -curvature.signum();
        if wide > 0.8 && steer.abs() > 0.9 {
            target_speed *= 1.0 - ((wide - 0.8) * 0.04).min(0.25);
        }
        if self.intent != Intent::Line {
            // Committed passes carry a touch more speed on the straight.
            target_speed *= 1.0 + 0.01 * self.profile.aggression;
        }
        for rival in rivals {
            let in_line = (rival.lateral - car.track_position.lateral).abs() < car.spec.width + 0.3;
            if in_line && rival.gap > 0.0 && rival.gap < 4.0 + speed * 0.35 {
                // Do not drive into the back of a car we cannot pass yet.
                let closing = speed - rival.speed;
                let room = rival.gap - 4.0;
                if closing > 0.0 && room < closing * 1.2 {
                    target_speed = target_speed.min(rival.speed - 0.5);
                }
            }
        }
        let error = target_speed - speed;
        let (throttle, brake) = if error > 0.0 {
            ((error * 0.35 + 0.35).clamp(0.0, 1.0), 0.0)
        } else if error > -1.5 {
            (0.15, 0.0)
        } else {
            (0.0, ((-error - 1.5) * 0.22).clamp(0.0, 1.0))
        };
        // Feed the throttle in as the steering unwinds; back off if the rear slides.
        let rear_sliding = car.wheels[2].sliding.max(car.wheels[3].sliding);
        let throttle = throttle * (1.0 - rear_sliding * 0.6) * (1.0 - steer.abs() * 0.35);

        CarInput {
            steer,
            throttle,
            brake,
            shift_up: false,
            shift_down: false,
        }
    }

    fn replan(&mut self, car: &Car, track: &Track, rivals: &[Rival], corner: Option<CornerPlan>) {
        // The whole-lap profile only needs rebuilding once the tyres have changed.
        let grip = tyre_grip(car);
        if (grip - self.planned_grip).abs() > self.planned_grip * 0.004 {
            self.planned_grip = grip;
            self.speed_profile = speed_profile(self.profile, car, track, grip);
            self.corners = find_corners(track, &self.speed_profile);
        }
        let s = car.track_position.s;
        // Forget a plan once its corner is behind us.
        match self.intent {
            Intent::Attack { apex_s, .. } | Intent::Defend { apex_s, .. }
                if track.delta(s, apex_s) < -15.0 =>
            {
                self.intent = Intent::Line;
            }
            _ => {}
        }
        let Some(corner) = corner else {
            self.intent = Intent::Line;
            return;
        };
        let to_brake = track.delta(s, corner.brake_s).rem_euclid(track.length);

        // An occasional late-braking error, decided once per corner.
        if self.mistake_corner != Some(corner.apex_s) {
            self.mistake_corner = Some(corner.apex_s);
            let chance = (1.0 - self.profile.consistency) * 0.6;
            self.mistake = if self.rng.next_f64() < chance {
                4.0 + self.rng.next_f64() * 10.0
            } else {
                0.0
            };
        }

        let ahead = rivals
            .iter()
            .filter(|rival| rival.gap > 0.0 && rival.gap < 45.0)
            .min_by(|a, b| a.gap.total_cmp(&b.gap));
        let behind = rivals
            .iter()
            .filter(|rival| rival.gap < 0.0 && rival.gap > -20.0)
            .max_by(|a, b| a.gap.total_cmp(&b.gap));

        match self.intent {
            Intent::Attack { side, apex_s } => {
                // Back out of a hopeless move instead of dive-bombing.
                if let Some(target) = ahead {
                    let still_behind = target.gap > car.spec.length * 0.6;
                    if still_behind && to_brake < 40.0 && to_brake < track.length * 0.5 {
                        self.intent = Intent::Line;
                    } else if apex_s != corner.apex_s {
                        self.intent = Intent::Attack {
                            side,
                            apex_s: corner.apex_s,
                        };
                    }
                } else if rivals
                    .iter()
                    .all(|rival| rival.gap.abs() > car.spec.length + 2.0)
                {
                    self.intent = Intent::Line;
                }
            }
            Intent::Defend { .. } => {
                if behind.is_none() {
                    self.intent = Intent::Line;
                }
            }
            Intent::Line => {
                let corner_close = to_brake < 320.0;
                if let Some(target) = ahead {
                    let closing = car.speed() - target.speed;
                    let towed = car.aero.slipstream() > 0.06;
                    let keen = 0.35 + self.profile.aggression * 0.5;
                    if corner_close && (towed || closing > 1.5) && target.gap < 12.0 + keen * 20.0 {
                        // Inside if it is open; if the leader covered it, go around.
                        let covering = (target.lateral - corner.inside * 3.0).abs() < 2.5;
                        let side = if covering {
                            -corner.inside
                        } else {
                            corner.inside
                        };
                        self.intent = Intent::Attack {
                            side,
                            apex_s: corner.apex_s,
                        };
                        return;
                    }
                }
                if let Some(chaser) = behind {
                    let threat = chaser.speed > car.speed() - 1.0 && chaser.gap > -14.0;
                    if corner_close && threat && self.profile.aggression > 0.25 {
                        self.intent = Intent::Defend {
                            side: corner.inside,
                            apex_s: corner.apex_s,
                        };
                    }
                }
            }
        }
    }
}

/// Match the braking plan to the tyre compound, temperature and wear used by
/// the physical car, rather than assuming optimal tyres for the whole race.
fn speed_profile(profile: DriverProfile, car: &Car, track: &Track, tyre_grip: f64) -> Vec<f64> {
    let spec = &car.spec;
    track.racing_line.speed_profile(&PerformanceEnvelope {
        mass: spec.mass,
        grip: spec.tyre_friction * 0.84 * profile.pace * tyre_grip,
        lift_area: car.setup.front_lift_area(&spec.aero) + car.setup.rear_lift_area(&spec.aero),
        drag_area: car.setup.drag_area(&spec.aero),
        power: spec.engine.max_power * 0.92,
        // Leave headroom for axle brake bias, ABS trimming and turn-in loads.
        brake_fraction: 0.76 * profile.pace,
    })
}

fn tyre_grip(car: &Car) -> f64 {
    car.tyres
        .iter()
        .map(|tyre| tyre.grip(car.setup.compound))
        .fold(f64::INFINITY, f64::min)
}

/// Finds braking zones and apexes from the speed profile's local minima.
fn find_corners(track: &Track, speed: &[f64]) -> Vec<CornerPlan> {
    let count = speed.len();
    let window = (60.0 / STATION_SPACING) as usize;
    let mut corners = Vec::new();
    for index in 0..count {
        let here = speed[index];
        let is_minimum = (1..=window).all(|offset| {
            here <= speed[(index + offset) % count]
                && here < speed[(index + count - offset) % count]
        });
        if !is_minimum {
            continue;
        }
        // Only corners that need a real lift or brake.
        let approach_peak = (window..window * 8)
            .map(|offset| speed[(index + count - offset) % count])
            .fold(0.0_f64, f64::max);
        if approach_peak < here * 1.08 {
            continue;
        }
        let mut brake = index;
        for step in 1..window * 8 {
            let candidate = (index + count - step) % count;
            if speed[candidate] >= approach_peak * 0.995 {
                brake = candidate;
                break;
            }
        }
        let curvature = track.racing_line.curvature[index];
        corners.push(CornerPlan {
            brake_s: brake as f64 * STATION_SPACING,
            apex_s: index as f64 * STATION_SPACING,
            inside: if curvature >= 0.0 { 1.0 } else { -1.0 },
            apex_speed: here,
        });
    }
    corners
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::spec::{test_wheels, CarSpec, Setup};
    use crate::car::Assists;
    use crate::track::oval_track;
    use std::sync::Arc;

    fn driver_and_car(track: &Track) -> (AiDriver, Car) {
        let (position, yaw) = track.grid_slot(0);
        let car = Car::new(
            Arc::new(CarSpec::rb22(test_wheels(), 5.4, 1.9)),
            Setup::default(),
            Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            track,
            position,
            yaw,
        );
        let driver = AiDriver::new(
            DriverProfile {
                pace: 0.97,
                aggression: 0.5,
                consistency: 1.0,
            },
            &car,
            track,
            7,
        );
        (driver, car)
    }

    #[test]
    fn finds_both_ends_of_the_oval_as_corners() {
        let track = oval_track();
        let (driver, _) = driver_and_car(&track);
        assert_eq!(driver.corners.len(), 2, "{:?}", driver.corners);
        assert!(driver.corners.iter().all(|corner| corner.apex_speed > 20.0));
    }

    #[test]
    fn replanning_brakes_for_the_current_tyre_grip() {
        let track = oval_track();
        let (mut driver, mut car) = driver_and_car(&track);
        for tyre in &mut car.tyres {
            tyre.temperature = 100.0;
        }
        driver.replan(&car, &track, &[], None);
        let warm = driver.target_speed(&track, 250.0);
        for tyre in &mut car.tyres {
            tyre.temperature = 35.0;
            tyre.wear = 0.6;
        }
        driver.replan(&car, &track, &[], None);
        let cold_worn = driver.target_speed(&track, 250.0);
        assert!(
            cold_worn < warm * 0.95,
            "warm {warm}, cold/worn {cold_worn}"
        );
    }

    #[test]
    fn laps_the_oval_on_the_asphalt() {
        let track = oval_track();
        let (mut driver, mut car) = driver_and_car(&track);
        let dt = 1.0 / 360.0;
        let mut distance = 0.0;
        let mut previous = car.track_position.s;
        let mut worst_lateral: f64 = 0.0;
        for tick in 0..(60.0 / dt) as usize {
            let input = driver.drive(&car, &track, &[], dt);
            let t = tick as f64 * dt;
            if std::env::var("AI_FINE").is_ok() && (22.6..24.0).contains(&t) && tick % 12 == 0 {
                println!(
                    "  t={t:.2} v={:.1} r={:.3} steer={:.2} thr={:.2} brk={:.2} lat={:.2} slip={:.3},{:.3},{:.3},{:.3} fy={:.0},{:.0},{:.0},{:.0} spin={},{} gear={} rpm={:.0}",
                    car.speed(), car.yaw_rate, input.steer, input.throttle, input.brake, car.track_position.lateral,
                    car.wheels[0].slip_angle, car.wheels[1].slip_angle, car.wheels[2].slip_angle, car.wheels[3].slip_angle,
                    car.wheels[0].lateral_force, car.wheels[1].lateral_force, car.wheels[2].lateral_force, car.wheels[3].lateral_force,
                    car.wheels[2].spinning, car.wheels[3].spinning, car.powertrain.gear, car.powertrain.rpm
                );
            }
            if std::env::var("AI_PROBE").is_ok() && tick % 180 == 0 {
                println!(
                    "t={:.1} s={:.0} v={:.1} target={:.1} thr={:.2} brk={:.2} steer={:.2} lat={:.2} gear={} slide={:.2}",
                    tick as f64 * dt, car.track_position.s, car.speed(),
                    driver.target_speed(&track, car.track_position.s), input.throttle, input.brake,
                    input.steer, car.track_position.lateral, car.powertrain.gear, car.wheels[2].sliding
                );
            }
            car.step(&input, &track, dt);
            distance += track.delta(previous, car.track_position.s);
            previous = car.track_position.s;
            if distance > 50.0 {
                worst_lateral = worst_lateral.max(car.track_position.lateral.abs());
            }
        }
        assert!(
            distance > track.length * 2.0,
            "only {distance} m in a minute"
        );
        assert!(worst_lateral < 6.5, "left the track: {worst_lateral} m");
    }
}
