//! A race session: grid, start lights, timing, collisions and AI.
//!
//! `Race::step` is deterministic for a given sequence of human inputs, which is
//! what a future lockstep or server-authoritative netcode layer needs.

use crate::aero;
use crate::ai::{AiDriver, DriverProfile, Rival};
use crate::car::spec::{CarSpec, Setup};
use crate::car::{Assists, Car, CarInput};
use crate::drs;
use crate::rng::Rng;
use crate::track::Track;
use glam::{DVec2, DVec3};
use std::sync::Arc;

/// Physics steps per second.
pub const PHYSICS_RATE: f64 = 360.0;
pub const SECTORS: usize = 3;

#[derive(Clone, Debug)]
pub enum Driver {
    /// Driven by `Race::step` inputs, indexed by human slot.
    Human {
        slot: usize,
    },
    Ai(Box<AiDriver>),
}

#[derive(Clone, Debug)]
pub struct EntrantConfig {
    pub name: String,
    pub spec: Arc<CarSpec>,
    pub setup: Setup,
    pub assists: Assists,
    /// `None` for a human driver.
    pub ai: Option<DriverProfile>,
    /// Index into `Race::step` inputs for human drivers.
    pub human_slot: usize,
    /// Body colour tint used by clients.
    pub livery: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Timing {
    /// Race distance: laps × track length + lap distance; negative on the grid.
    pub distance: f64,
    pub laps_completed: u32,
    pub lap_start: f64,
    pub last_lap: Option<f64>,
    pub best_lap: Option<f64>,
    pub sector: usize,
    pub sector_start: f64,
    pub sector_times: [Option<f64>; SECTORS],
    pub best_sectors: [Option<f64>; SECTORS],
    pub finish_time: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Entrant {
    pub name: String,
    pub car: Car,
    pub driver: Driver,
    pub timing: Timing,
    pub livery: [f32; 3],
    drs: drs::Tracker,
    /// Last on-track pose and its centerline station, used to recover a car
    /// that is stuck or beached.
    safe_pose: (DVec3, f64, usize),
    /// Seconds left in which a recovered car passes through other cars, so a
    /// rejoin can never start a fresh pile-up.
    pub ghost: f64,
}

/// Seconds a recovered car ignores car-to-car contact while it gets going.
pub const RECOVERY_GHOST: f64 = 3.0;
/// Metres behind a rejoin spot that must be free of cars in its lane.
const REJOIN_CLEAR_BEHIND: f64 = 120.0;

/// Seconds after the winner finishes before unfinished cars are classified.
pub const FINISH_GRACE: f64 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    /// Red lights coming on; `lights` of five are lit. Cars are held.
    Countdown {
        lights: u8,
        timer: f64,
        hold: f64,
    },
    Racing,
    /// The leader has finished; others finish their current lap.
    Finishing,
    Finished,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RaceEvent {
    LightsOut,
    LapCompleted {
        entrant: usize,
        lap: u32,
        time: f64,
    },
    Finished {
        entrant: usize,
        position: usize,
    },
    Overtake {
        entrant: usize,
        passed: usize,
    },
    Contact {
        a: usize,
        b: Option<usize>,
        impulse: f64,
    },
}

#[derive(Clone)]
pub struct Race {
    pub track: Arc<Track>,
    pub entrants: Vec<Entrant>,
    pub laps: u32,
    pub phase: Phase,
    /// Seconds since lights out (negative during the countdown).
    pub time: f64,
    pub tick: u64,
    pub events: Vec<RaceEvent>,
    /// Empty when no car in the field has DRS.
    pub drs_zones: Vec<drs::Zone>,
    ai_power: f64,
    order: Vec<usize>,
    finish_order: Vec<usize>,
    rng: Rng,
}

impl Race {
    pub fn new(track: Arc<Track>, entrants: Vec<EntrantConfig>, laps: u32, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let entrants: Vec<Entrant> = entrants
            .into_iter()
            .enumerate()
            .map(|(grid, config)| {
                let (position, yaw) = track.grid_slot(grid);
                let assists = if config.ai.is_some() {
                    Assists {
                        automatic_gears: true,
                        ..Assists::default()
                    }
                } else {
                    config.assists
                };
                let car = Car::new(config.spec, config.setup, assists, &track, position, yaw);
                let driver = match config.ai {
                    Some(profile) => Driver::Ai(Box::new(AiDriver::new(
                        profile,
                        &car,
                        &track,
                        rng.next_u64(),
                    ))),
                    None => Driver::Human {
                        slot: config.human_slot,
                    },
                };
                let distance = track.delta(0.0, car.track_position.s).min(0.0);
                let track_station = car.track_position.station;
                Entrant {
                    name: config.name,
                    car,
                    driver,
                    timing: Timing {
                        distance,
                        ..Timing::default()
                    },
                    livery: config.livery,
                    drs: drs::Tracker::default(),
                    safe_pose: (position, yaw, track_station),
                    ghost: 0.0,
                }
            })
            .collect();
        let order = (0..entrants.len()).collect();
        let drs_zones = if entrants.iter().any(|e| e.car.spec.aero.has_drs()) {
            drs::zones(&track)
        } else {
            Vec::new()
        };
        Self {
            ai_power: 1.,
            drs_zones,
            track,
            entrants,
            laps,
            phase: Phase::Countdown {
                lights: 0,
                timer: 0.0,
                hold: 0.0,
            },
            time: -6.0,
            tick: 0,
            events: Vec::new(),
            order,
            finish_order: Vec::new(),
            rng,
        }
    }

    /// Scales propulsion for AI, including disconnected humans taken over by AI.
    pub fn set_ai_power(&mut self, power: f64) {
        self.ai_power = if power.is_finite() {
            power.clamp(0.5, 1.)
        } else {
            1.
        };
    }

    /// Entrant indices from first to last.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    pub fn position_of(&self, entrant: usize) -> usize {
        self.order
            .iter()
            .position(|&index| index == entrant)
            .unwrap_or(0)
            + 1
    }

    /// Advances one physics tick. `human_inputs[slot]` drives human entrants.
    pub fn step(&mut self, human_inputs: &[CarInput]) {
        let dt = 1.0 / PHYSICS_RATE;
        self.tick += 1;
        self.time += dt;
        let held = self.advance_phase(dt);

        let influences = aero::influences(
            &self.entrants.iter().map(|e| &e.car).collect::<Vec<_>>(),
            self.track.length,
        );
        let rivals: Vec<Vec<Rival>> = (0..self.entrants.len())
            .map(|index| self.rivals_of(index))
            .collect();
        self.update_drs(dt);

        for (index, entrant) in self.entrants.iter_mut().enumerate() {
            entrant.car.aero = influences[index];
            let mut input = match &mut entrant.driver {
                Driver::Human { slot } => human_inputs.get(*slot).copied().unwrap_or_default(),
                Driver::Ai(driver) => driver.drive(&entrant.car, &self.track, &rivals[index], dt),
            };
            if held
                || entrant.timing.finish_time.is_some() && matches!(entrant.driver, Driver::Ai(_))
            {
                if held {
                    if let Driver::Ai(driver) = &mut entrant.driver {
                        driver.hold();
                    }
                    // On the grid: hold the brakes, let the engine rev.
                    input.brake = 1.0;
                } else {
                    // Cool-down lap after the flag.
                    input.throttle = input.throttle.min(0.45);
                }
            }
            let power = if matches!(entrant.driver, Driver::Ai(_)) {
                self.ai_power
            } else {
                1.
            };
            entrant.car.step_with_power(&input, &self.track, dt, power);
        }

        for entrant in &mut self.entrants {
            entrant.ghost = (entrant.ghost - dt).max(0.0);
        }
        self.resolve_wall_contacts();
        self.resolve_car_contacts();
        self.recover_stuck_cars();
        self.update_timing();
    }

    /// Earns DRS at each detection point (within [`drs::GAP`] of the car ahead,
    /// from lap [`drs::ENABLED_AFTER_LAPS`] + 1) and opens it inside the zone.
    fn update_drs(&mut self, dt: f64) {
        let racing = matches!(self.phase, Phase::Racing | Phase::Finishing);
        let gaps: Vec<Option<f64>> = (0..self.entrants.len())
            .map(|index| self.physical_gap_ahead(index))
            .collect();
        for (index, entrant) in self.entrants.iter_mut().enumerate() {
            let timing = &entrant.timing;
            let qualifies = || {
                racing
                    && timing.finish_time.is_none()
                    && timing.laps_completed >= drs::ENABLED_AFTER_LAPS
                    && gaps[index].is_some_and(|gap| gap <= drs::GAP)
            };
            // Judge the zone where the car will be after this tick, so the
            // flap starts closing on the tick that crosses the zone's end
            // rather than one tick past it.
            let s = self
                .track
                .wrap(entrant.car.track_position.s + entrant.car.forward_speed().max(0.0) * dt);
            let open = entrant
                .drs
                .update(&self.track, &self.drs_zones, s, qualifies);
            let anywhere = entrant.car.assists.drs_anywhere;
            entrant.car.drs.available = (open || anywhere)
                && racing
                && entrant.timing.finish_time.is_none()
                && entrant.car.spec.aero.has_drs();
        }
    }

    fn advance_phase(&mut self, dt: f64) -> bool {
        match &mut self.phase {
            Phase::Countdown {
                lights,
                timer,
                hold,
            } => {
                *timer += dt;
                if *lights < 5 && *timer >= 1.0 {
                    *timer = 0.0;
                    *lights += 1;
                    if *lights == 5 {
                        *hold = 0.4 + self.rng.next_f64() * 1.2;
                    }
                } else if *lights == 5 && *timer >= *hold {
                    self.phase = Phase::Racing;
                    self.time = 0.0;
                    for entrant in &mut self.entrants {
                        entrant.timing.lap_start = 0.0;
                        entrant.timing.sector_start = 0.0;
                    }
                    self.events.push(RaceEvent::LightsOut);
                    return false;
                }
                true
            }
            _ => false,
        }
    }

    fn rivals_of(&self, index: usize) -> Vec<Rival> {
        let me = &self.entrants[index];
        self.entrants
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .filter_map(|(_, other)| {
                let gap = self
                    .track
                    .delta(me.car.track_position.s, other.car.track_position.s);
                (gap.abs() < 120.0).then(|| Rival {
                    gap,
                    lateral: other.car.track_position.lateral,
                    speed: other.car.speed(),
                    position: DVec2::new(other.car.position.x, other.car.position.z),
                })
            })
            .collect()
    }

    fn resolve_wall_contacts(&mut self) {
        const RESTITUTION: f64 = 0.25;
        const FRICTION: f64 = 0.35;
        for (index, entrant) in self.entrants.iter_mut().enumerate() {
            let car = &mut entrant.car;
            let center = DVec2::new(car.position.x, car.position.z);
            let walls: Vec<_> = self
                .track
                .collision
                .walls_near(center, car.spec.length)
                .copied()
                .collect();
            let mut strongest = 0.0_f64;
            for (circle, radius) in car.collision_circles() {
                for wall in &walls {
                    let closest = closest_on_segment(wall.a, wall.b, circle);
                    let delta = circle - closest;
                    let distance = delta.length();
                    if distance >= radius || distance < 1e-6 {
                        continue;
                    }
                    let normal = delta / distance;
                    car.translate(normal * (radius - distance));
                    let arm = closest - DVec2::new(car.position.x, car.position.z);
                    let point_velocity = car.velocity + DVec2::new(-arm.y, arm.x) * -car.yaw_rate;
                    let approach = point_velocity.dot(normal);
                    if approach >= 0.0 {
                        continue;
                    }
                    let angular = arm.perp_dot(normal).powi(2) / car.spec.yaw_inertia;
                    let magnitude =
                        -(1.0 + RESTITUTION) * approach / (1.0 / car.spec.mass + angular);
                    let tangent = DVec2::new(-normal.y, normal.x);
                    let slide = point_velocity.dot(tangent);
                    let friction =
                        -slide.signum() * (FRICTION * magnitude).min(slide.abs() * car.spec.mass);
                    car.apply_impulse(closest, normal * magnitude + tangent * friction);
                    strongest = strongest.max(magnitude);
                }
            }
            if strongest > 2_000.0 {
                self.events.push(RaceEvent::Contact {
                    a: index,
                    b: None,
                    impulse: strongest,
                });
            }
        }
    }

    fn resolve_car_contacts(&mut self) {
        const RESTITUTION: f64 = 0.3;
        let count = self.entrants.len();
        for first in 0..count {
            for second in first + 1..count {
                let (left, right) = self.entrants.split_at_mut(second);
                if left[first].ghost > 0.0 || right[0].ghost > 0.0 {
                    continue;
                }
                let a = &mut left[first].car;
                let b = &mut right[0].car;
                let separation =
                    DVec2::new(a.position.x - b.position.x, a.position.z - b.position.z);
                if separation.length() > a.spec.length + b.spec.length {
                    continue;
                }
                let mut strongest = 0.0_f64;
                for (circle_a, radius_a) in a.collision_circles() {
                    for (circle_b, radius_b) in b.collision_circles() {
                        let delta = circle_a - circle_b;
                        let distance = delta.length();
                        let reach = radius_a + radius_b;
                        if distance >= reach || distance < 1e-6 {
                            continue;
                        }
                        let normal = delta / distance;
                        let push = (reach - distance) * 0.5;
                        a.translate(normal * push);
                        b.translate(-normal * push);
                        let contact = circle_b + normal * radius_b;
                        let relative = a.velocity - b.velocity;
                        let approach = relative.dot(normal);
                        if approach >= 0.0 {
                            continue;
                        }
                        let inverse_mass = 1.0 / a.spec.mass + 1.0 / b.spec.mass;
                        let magnitude = -(1.0 + RESTITUTION) * approach / inverse_mass;
                        a.apply_impulse(contact, normal * magnitude);
                        b.apply_impulse(contact, -normal * magnitude);
                        strongest = strongest.max(magnitude);
                    }
                }
                if strongest > 1_500.0 {
                    self.events.push(RaceEvent::Contact {
                        a: first,
                        b: Some(second),
                        impulse: strongest,
                    });
                }
            }
        }
    }

    fn recover_stuck_cars(&mut self) {
        let racing = matches!(self.phase, Phase::Racing | Phase::Finishing);
        for index in 0..self.entrants.len() {
            let entrant = &mut self.entrants[index];
            let car = &entrant.car;
            let on_track = car.track_position.lateral.abs()
                < self.track.station(car.track_position.station).half_width
                && car.speed() > 8.0;
            if on_track {
                entrant.safe_pose = (car.position, car.yaw, car.track_position.station);
            }
            let Driver::Ai(driver) = &mut entrant.driver else {
                continue;
            };
            let wrong_way = car
                .forward()
                .dot(self.track.frame_at(car.track_position.s).tangent)
                < -0.3;
            if racing && (driver.stuck_time() > 2.5 || wrong_way && car.speed() > 3.0) {
                driver.reset_after_recovery();
                let s = car.track_position.s;
                let (position, yaw) = self.clear_spot(index, s);
                let station = self.track.station_at(s);
                self.teleport(index, position, yaw, station);
            }
        }
    }

    /// A rejoin pose near `s`: at the edge away from the racing line, clear of
    /// other cars and of anyone coming up behind in that lane, so a recovered
    /// car does not land in front of the traffic it just left.
    fn clear_spot(&self, index: usize, s: f64) -> (DVec3, f64) {
        let clearance = self.entrants[index].car.spec.length * 1.6;
        let blocked = |spot_s: f64, point: DVec3, lateral: f64| {
            self.entrants.iter().enumerate().any(|(other, entrant)| {
                if other == index {
                    return false;
                }
                let car = &entrant.car;
                let near = DVec2::new(car.position.x - point.x, car.position.z - point.z).length()
                    < clearance;
                let behind = self.track.delta(car.track_position.s, spot_s);
                let coming = (0.0..REJOIN_CLEAR_BEHIND).contains(&behind)
                    && (car.track_position.lateral - lateral).abs() < car.spec.width + 1.5;
                near || coming
            })
        };
        let pose = |along: f64, side: f64| {
            let s = self.track.wrap(s + along);
            let frame = self.track.frame_at(s);
            let line = self.track.racing_line.offsets[self.track.station_at(s)];
            let limit = (frame.half_width - 1.5).max(0.0);
            // Off the line: the edge on the side the line is not using.
            let edge = if line >= 0.0 { -limit } else { limit };
            let lateral = (edge + side).clamp(-limit, limit);
            let position = frame.point_at(lateral);
            (s, lateral, position, frame.tangent.x.atan2(frame.tangent.y))
        };
        // Prefer staying put, then a little further back, then forward.
        for along in [0.0, -15.0, -30.0, -45.0, 15.0, -60.0, -80.0, 30.0] {
            for side in [0.0, 2.0, -2.0] {
                let (spot_s, lateral, position, yaw) = pose(along, side);
                if !blocked(spot_s, position, lateral) {
                    return (position, yaw);
                }
            }
        }
        let (_, _, position, yaw) = pose(0.0, 0.0);
        (position, yaw)
    }

    /// Moves a car to a new pose and keeps its race distance consistent.
    fn teleport(&mut self, index: usize, position: DVec3, yaw: f64, station: usize) {
        let track = self.track.clone();
        let entrant = &mut self.entrants[index];
        let before = entrant.car.track_position.s;
        reset_car(&mut entrant.car, &track, position, yaw, station);
        // A recovered car gives up any DRS it had earned, and passes through
        // other cars until it is back up to speed.
        entrant.drs.reset();
        entrant.ghost = RECOVERY_GHOST;
        entrant.timing.distance += track.delta(before, entrant.car.track_position.s);
    }

    /// Puts a human car back on track at its last safe pose (the "reset" key).
    pub fn recover(&mut self, entrant: usize) {
        let (position, yaw, station) = self.entrants[entrant].safe_pose;
        self.teleport(entrant, position, yaw, station);
    }

    fn update_timing(&mut self) {
        let racing = matches!(self.phase, Phase::Racing | Phase::Finishing);
        let sector_length = self.track.length / SECTORS as f64;
        let previous_order = self.order.clone();
        for (index, entrant) in self.entrants.iter_mut().enumerate() {
            let timing = &mut entrant.timing;
            let lap_s = timing.distance.rem_euclid(self.track.length);
            let delta = self.track.delta(lap_s, entrant.car.track_position.s);
            // Recoveries can teleport a car; ignore jumps that are not driving.
            if delta.abs() < 50.0 {
                timing.distance += delta;
            }
            if !racing || timing.finish_time.is_some() {
                continue;
            }
            let laps = if timing.distance < 0.0 {
                0
            } else {
                (timing.distance / self.track.length) as u32
            };
            let sector = ((timing.distance.max(0.0) % self.track.length) / sector_length) as usize;
            if laps > timing.laps_completed || sector != timing.sector {
                let sector_time = self.time - timing.sector_start;
                let finished_sector = timing.sector;
                if timing.distance > 0.0
                    && (laps > timing.laps_completed || sector > finished_sector)
                {
                    timing.sector_times[finished_sector] = Some(sector_time);
                    let best = &mut timing.best_sectors[finished_sector];
                    if best.is_none_or(|best| sector_time < best) {
                        *best = Some(sector_time);
                    }
                }
                timing.sector = sector.min(SECTORS - 1);
                timing.sector_start = self.time;
            }
            if laps > timing.laps_completed {
                let lap_time = self.time - timing.lap_start;
                timing.laps_completed = laps;
                timing.lap_start = self.time;
                timing.last_lap = Some(lap_time);
                if timing.best_lap.is_none_or(|best| lap_time < best) {
                    timing.best_lap = Some(lap_time);
                }
                timing.sector_times = [None; SECTORS];
                self.events.push(RaceEvent::LapCompleted {
                    entrant: index,
                    lap: laps,
                    time: lap_time,
                });
                let flag_out = matches!(self.phase, Phase::Finishing);
                if laps >= self.laps || flag_out {
                    timing.finish_time = Some(self.time);
                    self.finish_order.push(index);
                    self.events.push(RaceEvent::Finished {
                        entrant: index,
                        position: self.finish_order.len(),
                    });
                    if self.phase == Phase::Racing {
                        self.phase = Phase::Finishing;
                    }
                }
            }
        }

        // Running order: finishers in finishing order, then by race distance.
        let mut order: Vec<usize> = (0..self.entrants.len()).collect();
        order.sort_by(|&a, &b| {
            let rank = |index: usize| self.finish_order.iter().position(|&f| f == index);
            match (rank(a), rank(b)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => self.entrants[b]
                    .timing
                    .distance
                    .total_cmp(&self.entrants[a].timing.distance),
            }
        });
        if racing {
            for (new_position, &entrant) in order.iter().enumerate() {
                let old_position = previous_order
                    .iter()
                    .position(|&e| e == entrant)
                    .unwrap_or(0);
                if new_position < old_position {
                    let passed = previous_order[new_position];
                    if self.entrants[passed].timing.finish_time.is_none() {
                        self.events.push(RaceEvent::Overtake { entrant, passed });
                    }
                }
            }
        }
        self.order = order;
        if self.phase == Phase::Finishing {
            let everyone = self
                .entrants
                .iter()
                .all(|entrant| entrant.timing.finish_time.is_some());
            // Stopped, crashed or lapped cars must not hold the results forever:
            // classify them by distance once the grace period after the winner ends.
            let winner = self
                .finish_order
                .first()
                .and_then(|&first| self.entrants[first].timing.finish_time);
            let expired = winner.is_some_and(|time| self.time - time > FINISH_GRACE);
            if everyone || expired {
                self.phase = Phase::Finished;
            }
        }
    }

    /// Gap in seconds from `entrant` to the car ahead of it, estimated from speed.
    pub fn gap_ahead(&self, entrant: usize) -> Option<f64> {
        let position = self.position_of(entrant);
        if position <= 1 {
            return None;
        }
        let ahead = self.order[position - 2];
        let distance =
            self.entrants[ahead].timing.distance - self.entrants[entrant].timing.distance;
        Some(distance / self.entrants[entrant].car.speed().max(f64::EPSILON))
    }

    fn physical_gap_ahead(&self, entrant: usize) -> Option<f64> {
        let follower = self.entrants[entrant].car.track_position.s;
        self.entrants
            .iter()
            .enumerate()
            .filter(|(index, other)| *index != entrant && other.timing.finish_time.is_none())
            .map(|(_, other)| self.track.delta(follower, other.car.track_position.s))
            .filter(|gap| *gap > 0.0)
            .min_by(f64::total_cmp)
            .map(|distance| distance / self.entrants[entrant].car.speed().max(f64::EPSILON))
    }
}

fn reset_car(car: &mut Car, track: &Track, position: DVec3, yaw: f64, station: usize) {
    let ground = track
        .collision
        .ground(position + DVec3::Y * 2.0)
        .map_or(position.y, |hit| hit.height);
    // Recovery clears crash dynamics, but does not grant fresh tyres.
    let tyres = car.tyres;
    *car = Car::new(
        car.spec.clone(),
        car.setup,
        car.assists,
        track,
        DVec3::new(position.x, ground, position.z),
        yaw,
    );
    car.tyres = tyres;
    // A global projection can pick a nearby, different part of the circuit.
    car.track_position = track.project(car.position, Some(station));
}

fn closest_on_segment(a: DVec2, b: DVec2, point: DVec2) -> DVec2 {
    let edge = b - a;
    let t = ((point - a).dot(edge) / edge.length_squared().max(1e-12)).clamp(0.0, 1.0);
    a + edge * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::spec::test_wheels;
    use crate::track::oval_track;

    fn field(humans: usize, ai: usize) -> Vec<EntrantConfig> {
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        (0..humans + ai)
            .map(|index| EntrantConfig {
                name: format!("Driver {index}"),
                spec: spec.clone(),
                setup: Setup::default(),
                assists: Assists::default(),
                ai: (index >= humans).then_some(DriverProfile {
                    pace: 0.94 + index as f64 * 0.005,
                    aggression: 0.6,
                    consistency: 0.9,
                }),
                human_slot: index,
                livery: [1.0; 3],
            })
            .collect()
    }

    #[test]
    fn following_cars_earn_drs_from_lap_two_and_open_it_in_the_zone() {
        let track = Arc::new(crate::track::stadium(800.0, 30.0, Vec::new()));
        let mut race = Race::new(track.clone(), field(0, 3), 4, 11);
        assert_eq!(race.drs_zones.len(), 2);
        let mut opened_on_lap = [None; 3];
        while race.phase != Phase::Finished && race.time < 400.0 {
            race.step(&[]);
            for (index, entrant) in race.entrants.iter().enumerate() {
                let car = &entrant.car;
                if car.drs.flap > 0.0 {
                    let s = car.track_position.s;
                    assert!(
                        race.drs_zones.iter().any(|zone| zone.contains(&track, s))
                            || car.drs.flap < 1.0,
                        "flap fully open outside a zone at {s}"
                    );
                    opened_on_lap[index].get_or_insert(entrant.timing.laps_completed);
                }
            }
        }
        // Nobody opens on the first lap; someone close behind does later.
        assert!(opened_on_lap
            .iter()
            .flatten()
            .all(|&lap| lap >= drs::ENABLED_AFTER_LAPS));
        assert!(
            opened_on_lap.iter().any(Option::is_some),
            "{opened_on_lap:?}"
        );
        assert_eq!(race.phase, Phase::Finished);
    }

    #[test]
    fn drs_anywhere_lets_its_driver_open_the_flap_off_zone_from_lap_one() {
        let track = Arc::new(crate::track::stadium(800.0, 30.0, Vec::new()));
        let flap_after_start = |anywhere: bool| {
            let mut configs = field(1, 0);
            configs[0].assists.drs_anywhere = anywhere;
            let mut race = Race::new(track.clone(), configs, 3, 5);
            let input = [CarInput {
                throttle: 1.0,
                drs: true,
                ..CarInput::default()
            }];
            while race.phase != Phase::Racing {
                race.step(&input);
            }
            // Still near the grid: lap one, outside both zones.
            for _ in 0..(0.5 * PHYSICS_RATE) as usize {
                race.step(&input);
            }
            let car = &race.entrants[0].car;
            assert!(!race
                .drs_zones
                .iter()
                .any(|zone| zone.contains(&track, car.track_position.s)));
            car.drs.flap
        };
        assert_eq!(flap_after_start(false), 0.0);
        assert_eq!(flap_after_start(true), 1.0);
    }

    #[test]
    fn a_field_without_drs_has_no_zones() {
        let track = Arc::new(crate::track::stadium(800.0, 30.0, Vec::new()));
        let mut configs = field(0, 2);
        for config in &mut configs {
            config.spec = Arc::new(CarSpec::rb6(test_wheels(), 5.4, 1.9));
        }
        let race = Race::new(track, configs, 1, 1);
        assert!(race.drs_zones.is_empty());
    }

    #[test]
    fn lights_hold_the_grid_then_go_out() {
        let track = Arc::new(oval_track());
        let mut race = Race::new(track.clone(), field(1, 0), 1, 3);
        let throttle = [CarInput {
            throttle: 1.0,
            ..CarInput::default()
        }];
        for _ in 0..(4.0 * PHYSICS_RATE) as usize {
            race.step(&throttle);
        }
        assert!(matches!(race.phase, Phase::Countdown { .. }));
        assert!(race.entrants[0].car.speed() < 0.5);
        for _ in 0..(4.0 * PHYSICS_RATE) as usize {
            race.step(&throttle);
        }
        assert_eq!(race.phase, Phase::Racing);
        assert!(race.events.contains(&RaceEvent::LightsOut));
        assert!(race.entrants[0].car.speed() > 5.0);
    }

    #[test]
    fn recovery_clears_crash_dynamics_but_preserves_tyres_and_timing() {
        let track = Arc::new(oval_track());
        let mut race = Race::new(track.clone(), field(1, 0), 1, 3);
        let entrant = &mut race.entrants[0];
        entrant.car.acceleration = DVec2::new(25.0, 35.0);
        entrant.car.velocity = DVec2::new(10.0, -20.0);
        entrant.car.yaw_rate = 4.0;
        entrant.car.powertrain.shift_timer = 0.1;
        entrant.car.powertrain.refusal_timer = 0.25;
        entrant.car.impact = 10_000.0;
        entrant.car.tyres[0].temperature = 110.0;
        entrant.car.tyres[1].wear = 0.4;
        for wheel in &mut entrant.car.wheels {
            wheel.lateral_force = 4_000.0;
            wheel.locked = true;
            wheel.spinning = true;
        }
        let tyres = entrant.car.tyres;
        let timing = entrant.timing.clone();
        race.recover(0);
        let car = &race.entrants[0].car;
        assert_eq!(car.tyres, tyres);
        assert_eq!(race.entrants[0].timing, timing);
        let mut clean = Car::new(
            car.spec.clone(),
            car.setup,
            car.assists,
            &track,
            car.position,
            car.yaw,
        );
        clean.tyres = tyres;
        let recovered = &mut race.entrants[0].car;
        for _ in 0..180 {
            recovered.step(&CarInput::default(), &track, 1.0 / PHYSICS_RATE);
            clean.step(&CarInput::default(), &track, 1.0 / PHYSICS_RATE);
            assert_eq!(recovered.velocity, clean.velocity);
            assert_eq!(recovered.yaw_rate, clean.yaw_rate);
            assert_eq!(recovered.powertrain, clean.powertrain);
            assert_eq!(recovered.wheels, clean.wheels);
        }
    }

    #[test]
    fn ai_field_completes_a_race_deterministically() {
        let track = Arc::new(oval_track());
        let run = || {
            let mut race = Race::new(track.clone(), field(0, 4), 2, 11);
            let mut ticks = 0;
            while race.phase != Phase::Finished && ticks < (240.0 * PHYSICS_RATE) as usize {
                race.step(&[]);
                ticks += 1;
            }
            (
                race.phase,
                race.order().to_vec(),
                race.entrants
                    .iter()
                    .map(|e| e.timing.best_lap)
                    .collect::<Vec<_>>(),
            )
        };
        let first = run();
        assert_eq!(first.0, Phase::Finished, "race did not finish");
        assert!(first.2.iter().all(Option::is_some));
        assert_eq!(first, run(), "simulation must be deterministic");
    }

    #[test]
    fn a_faster_ai_gets_past_a_slower_leader() {
        let track = Arc::new(oval_track());
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        let driver = |name: &str, pace: f64| EntrantConfig {
            name: name.into(),
            spec: spec.clone(),
            setup: Setup::default(),
            assists: Assists::default(),
            ai: Some(DriverProfile {
                pace,
                aggression: 0.6,
                consistency: 1.0,
            }),
            human_slot: 0,
            livery: [1.0; 3],
        };
        let entrants = vec![driver("Slow", 0.95), driver("Fast", 0.99)];
        let mut race = Race::new(track, entrants, 6, 3);
        let mut ticks = 0;
        while race.phase != Phase::Finished && ticks < (300.0 * PHYSICS_RATE) as usize {
            race.step(&[]);
            ticks += 1;
        }
        assert_eq!(race.phase, Phase::Finished);
        assert_eq!(race.order()[0], 1, "the faster car stayed stuck behind");
    }

    /// Speeds of (human, AI) after a straight-line launch with the given AI power.
    fn launch_speeds(ai_power: f64) -> (f64, f64) {
        let track = Arc::new(crate::track::stadium(800.0, 30.0, Vec::new()));
        let mut race = Race::new(track, field(1, 1), 1, 7);
        race.set_ai_power(ai_power);
        let full = CarInput {
            throttle: 1.0,
            shift_up: true,
            ..CarInput::default()
        };
        let mut racing = 0;
        while racing < (6.0 * PHYSICS_RATE) as usize {
            race.step(std::slice::from_ref(&full));
            if race.phase == Phase::Racing {
                racing += 1;
            }
        }
        (race.entrants[0].car.speed(), race.entrants[1].car.speed())
    }

    #[test]
    fn ai_power_slows_only_the_ai() {
        let (human_full, ai_full) = launch_speeds(1.0);
        let (human_low, ai_low) = launch_speeds(0.5);
        assert_eq!(
            human_full, human_low,
            "the player's car must not be handicapped"
        );
        assert!(
            ai_low < ai_full - 1.0,
            "50% AI power should be clearly slower: {ai_low:.2} vs {ai_full:.2} m/s"
        );
    }

    #[test]
    fn ai_power_is_clamped_to_the_slider_range() {
        let mut race = Race::new(Arc::new(oval_track()), field(0, 1), 1, 1);
        race.set_ai_power(0.3);
        assert_eq!(race.ai_power, 0.5);
        race.set_ai_power(f64::NAN);
        assert_eq!(race.ai_power, 1.0);
    }

    #[test]
    fn a_stopped_car_cannot_hold_back_the_results() {
        // The human never drives; the AI wins and the race still ends.
        let mut race = Race::new(Arc::new(oval_track()), field(1, 1), 1, 5);
        let mut ticks = 0;
        while race.phase != Phase::Finished && ticks < (400.0 * PHYSICS_RATE) as usize {
            race.step(&[CarInput::default()]);
            ticks += 1;
        }
        assert_eq!(race.phase, Phase::Finished);
        assert!(race.entrants[0].timing.finish_time.is_none());
        assert_eq!(race.order(), &[1, 0]);
    }

    #[test]
    fn walls_stop_cars() {
        let wall = crate::track::Wall {
            a: DVec2::new(-50.0, 20.0),
            b: DVec2::new(50.0, 20.0),
        };
        let track = Arc::new(Track::with_walls_for_test(vec![wall]));
        let mut race = Race::new(track.clone(), field(1, 0), 1, 1);
        race.phase = Phase::Racing;
        let car = &mut race.entrants[0].car;
        car.position = DVec3::new(0.0, 0.0, 10.0);
        car.yaw = 0.0;
        car.velocity = DVec2::new(0.0, 30.0);
        for _ in 0..PHYSICS_RATE as usize {
            race.step(&[CarInput::default()]);
        }
        let car = &race.entrants[0].car;
        assert!(
            car.position.z < 20.0,
            "drove through the wall: {}",
            car.position.z
        );
        assert!(race
            .events
            .iter()
            .any(|event| matches!(event, RaceEvent::Contact { .. })));
    }
}

/// Needs converted assets: `F1_ASSETS=assets cargo test -p racecraft-sim -- --ignored overtak`.
#[cfg(test)]
mod spa_overtaking {
    use super::*;
    use crate::car::spec::test_wheels;

    #[test]
    #[ignore]
    fn a_faster_ai_overtakes_at_spa_without_crashing_into_the_leader() {
        let Ok(root) = std::env::var("F1_ASSETS") else {
            return;
        };
        let track = Arc::new(Track::load(std::path::Path::new(&root).join("tracks/spa")).unwrap());
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        let driver = |name: &str, pace: f64| EntrantConfig {
            name: name.into(),
            spec: spec.clone(),
            setup: Setup::default(),
            assists: Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            ai: Some(DriverProfile {
                pace,
                aggression: 0.6,
                consistency: 1.0,
            }),
            human_slot: 0,
            livery: [1.0; 3],
        };
        let (mut wins, mut contacts) = (0, 0);
        for seed in [1, 3, 5, 7, 9, 11] {
            // The slower car starts ahead; the faster one must get by.
            let entrants = vec![driver("Slow", 0.92), driver("Fast", 0.99)];
            let mut race = Race::new(track.clone(), entrants, 3, seed);
            let mut ticks = 0;
            while race.phase != Phase::Finished && ticks < (600.0 * PHYSICS_RATE) as usize {
                race.step(&[]);
                contacts += race
                    .events
                    .iter()
                    .filter(|event| matches!(event, RaceEvent::Contact { b: Some(_), .. }))
                    .count();
                race.events.clear();
                ticks += 1;
            }
            assert_eq!(race.phase, Phase::Finished, "seed {seed}: race timed out");
            if race.entrants[race.order()[0]].name == "Fast" {
                wins += 1;
            }
        }
        assert!(wins >= 5, "the faster car won only {wins} of 6");
        assert!(contacts <= 4, "{contacts} car contacts in 6 races");
    }
}

/// Needs converted assets: `F1_ASSETS=assets cargo test -p racecraft-sim -- --ignored opening_lap`.
#[cfg(test)]
mod spa_opening_lap {
    use super::*;
    use crate::car::spec::test_wheels;

    #[test]
    #[ignore]
    fn a_full_grid_gets_through_la_source_without_wrecking() {
        let Ok(root) = std::env::var("F1_ASSETS") else {
            return;
        };
        let track = Arc::new(Track::load(std::path::Path::new(&root).join("tracks/spa")).unwrap());
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        let (mut total, mut heavy, mut spun) = (0, 0, 0);
        for seed in [1, 2, 3, 4] {
            let entrants = (0..22)
                .map(|slot| EntrantConfig {
                    name: format!("AI{slot}"),
                    spec: spec.clone(),
                    setup: Setup::default(),
                    assists: Assists {
                        automatic_gears: true,
                        ..Assists::default()
                    },
                    ai: Some(DriverProfile {
                        pace: 0.985 - 0.006 * slot as f64,
                        aggression: 0.3 + 0.03 * (slot % 10) as f64,
                        consistency: 0.97,
                    }),
                    human_slot: 0,
                    livery: [1.0; 3],
                })
                .collect();
            let mut race = Race::new(track.clone(), entrants, 2, seed);
            let (mut contacts, mut big) = (0, 0);
            let mut spread_checked = false;
            for _ in 0..(45.0 * PHYSICS_RATE) as usize {
                race.step(&[]);
                if !spread_checked && race.phase == Phase::Racing && race.time > 4.0 {
                    // Two-wide off the line, not a single file onto it.
                    spread_checked = true;
                    let off_line = race
                        .entrants
                        .iter()
                        .filter(|entrant| {
                            let p = entrant.car.track_position;
                            (p.lateral - track.racing_line.offsets[track.station_at(p.s)]).abs()
                                > 1.5
                        })
                        .count();
                    assert!(
                        off_line >= 8,
                        "seed {seed}: only {off_line} of 22 cars off the line"
                    );
                }
                for event in &race.events {
                    if let RaceEvent::Contact {
                        b: Some(_),
                        impulse,
                        ..
                    } = event
                    {
                        contacts += 1;
                        if *impulse > 8000.0 {
                            big += 1;
                        }
                    }
                }
                race.events.clear();
            }
            // Cars left far behind or turned around after the first corner.
            let lost = race
                .entrants
                .iter()
                .filter(|e| e.timing.distance < 600.0)
                .count();
            println!("seed {seed}: contacts {contacts}, heavy {big}, stranded before 600 m {lost}");
            total += contacts;
            heavy += big;
            spun += lost;
        }
        println!("TOTAL contacts {total}, heavy {heavy}, stranded {spun}");
        // Light rubbing in a 22-car opening is fine; hard hits and wrecked
        // cars are not. Single-file starts gave 19-104 contacts; grid lanes ~2.
        assert!(total <= 12, "{total} contacts in four openings");
        assert!(heavy <= 1, "{heavy} heavy contacts");
        assert_eq!(spun, 0, "{spun} cars stranded at the first corner");
    }
}

/// Needs converted assets: `F1_ASSETS=assets cargo test -p racecraft-sim -- --ignored room`.
#[cfg(test)]
mod human_passing {
    use super::*;
    use crate::car::spec::test_wheels;

    /// Steer the human car toward a fixed lateral offset, flat out.
    fn hold_lane(car: &Car, track: &Track, lateral: f64) -> CarInput {
        let frame = track.frame_at(car.track_position.s + 30.0);
        let target = frame.point_at(lateral);
        let offset = DVec2::new(target.x - car.position.x, target.z - car.position.z);
        let local_left = offset.dot(car.left());
        let distance = offset.length().max(1.0);
        let curvature = 2.0 * local_left / (distance * distance);
        let wheel = (car.spec.wheelbase() * curvature).atan();
        CarInput {
            steer: (wheel / car.steer_limit(car.speed())).clamp(-1.0, 1.0),
            throttle: 1.0,
            drs: true,
            ..CarInput::default()
        }
    }

    #[test]
    #[ignore]
    fn the_ai_leaves_room_for_a_faster_car_coming_alongside() {
        let Ok(root) = std::env::var("F1_ASSETS") else {
            return;
        };
        let track = Arc::new(Track::load(std::path::Path::new(&root).join("tracks/spa")).unwrap());
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        for side in [1.0, -1.0] {
            let entrants = vec![
                EntrantConfig {
                    name: "You".into(),
                    spec: spec.clone(),
                    setup: Setup::default(),
                    assists: Assists {
                        automatic_gears: true,
                        drs_anywhere: true,
                        auto_drs: true,
                        ..Assists::default()
                    },
                    ai: None,
                    human_slot: 0,
                    livery: [1.0; 3],
                },
                EntrantConfig {
                    name: "Ahead".into(),
                    spec: spec.clone(),
                    setup: Setup::default(),
                    assists: Assists {
                        automatic_gears: true,
                        ..Assists::default()
                    },
                    ai: Some(DriverProfile {
                        pace: 0.95,
                        aggression: 0.6,
                        consistency: 1.0,
                    }),
                    human_slot: 1,
                    livery: [1.0; 3],
                },
            ];
            let mut race = Race::new(track.clone(), entrants, 3, 7);
            race.phase = Phase::Racing;
            let place = |car: &mut Car, s: f64, lateral: f64, speed: f64| {
                let frame = track.frame_at(s);
                car.position = frame.point_at(lateral) + glam::DVec3::Y * 0.3;
                car.yaw = frame.tangent.x.atan2(frame.tangent.y);
                car.velocity = car.forward() * speed;
                car.yaw_rate = 0.0;
                car.track_position = track.project(car.position, None);
                car.powertrain.gear = 8;
            };
            let line = track.racing_line.offsets[track.station_at(1500.0)];
            place(&mut race.entrants[1].car, 1500.0, line, 75.0);
            let lane = (line + side * 3.3).clamp(-4.8, 4.8);
            place(&mut race.entrants[0].car, 1475.0, lane, 82.0);
            for e in &mut race.entrants {
                e.timing.distance = e.car.track_position.s;
                e.timing.laps_completed = 1;
            }
            let (mut min_sep, mut contacts, mut ai_toward) = (99.0_f64, 0, 0.0_f64);
            let mut alongside_ticks = 0;
            let ai_start = race.entrants[1].car.track_position.lateral;
            for _ in 0..(7.0 * PHYSICS_RATE) as usize {
                // Aim a car width beside the AI, kept on the road.
                let ai_lat = race.entrants[1].car.track_position.lateral;
                let width = track
                    .frame_at(race.entrants[0].car.track_position.s + 30.0)
                    .half_width;
                let input = hold_lane(
                    &race.entrants[0].car,
                    &track,
                    (ai_lat + side * 3.3).clamp(-(width - 1.2), width - 1.2),
                );
                race.step(&[input]);
                contacts += race
                    .events
                    .iter()
                    .filter(|e| matches!(e, RaceEvent::Contact { b: Some(_), .. }))
                    .count();
                race.events.clear();
                let (me, ai) = (&race.entrants[0].car, &race.entrants[1].car);
                let gap = track.delta(me.track_position.s, ai.track_position.s);
                let sep = (me.track_position.lateral - ai.track_position.lateral).abs();
                if gap.abs() < 6.0 {
                    alongside_ticks += 1;
                    min_sep = min_sep.min(sep);
                }
                ai_toward = ai_toward.max((ai.track_position.lateral - ai_start) * side);
            }
            let (me, ai) = (&race.entrants[0].car, &race.entrants[1].car);
            let gap = track.delta(me.track_position.s, ai.track_position.s);
            // A car width (1.9 m) plus margin: never squeezed into the
            // wall or chopped as the racing line swings across.
            assert!(
                alongside_ticks > 0 && min_sep > 2.6,
                "side {side}: squeezed to {min_sep:.2} m alongside"
            );
            assert_eq!(contacts, 0, "side {side}: {contacts} contacts");
            let _ = (ai_toward, me, ai, gap);
        }
    }
}

/// Needs converted assets: `F1_ASSETS=assets cargo test -p racecraft-sim -- --ignored race_with`.
#[cfg(test)]
mod race_with_player {
    use super::*;
    use crate::car::spec::test_wheels;

    pub(super) fn run(
        seed: u64,
        seconds: f64,
        verbose: bool,
    ) -> (usize, usize, usize, usize, usize) {
        let root = std::env::var("F1_ASSETS").unwrap();
        let track = Arc::new(Track::load(std::path::Path::new(&root).join("tracks/spa")).unwrap());
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        const PLAYER: usize = 9;
        let entrants: Vec<_> = (0..22)
            .map(|slot| EntrantConfig {
                name: if slot == PLAYER {
                    "You".into()
                } else {
                    format!("AI{slot}")
                },
                spec: spec.clone(),
                setup: Setup::default(),
                assists: Assists {
                    automatic_gears: true,
                    braking: 1.0,
                    auto_drs: slot == PLAYER,
                    ..Assists::default()
                },
                ai: (slot != PLAYER).then_some(DriverProfile {
                    pace: 0.985 - 0.006 * slot as f64,
                    aggression: 0.3 + 0.03 * (slot % 10) as f64,
                    consistency: 0.97,
                }),
                human_slot: 0,
                livery: [1.0; 3],
            })
            .collect();
        let mut race = Race::new(track.clone(), entrants, 3, seed);
        // A beginner on full guidance: the AI line and braking, no grid lane.
        let mut you = AiDriver::new(
            DriverProfile {
                pace: 0.9,
                aggression: 0.0,
                consistency: 1.0,
            },
            &race.entrants[PLAYER].car,
            &track,
            99,
        );
        let (mut contacts, mut mine, mut heavy, mut recoveries, mut in_my_path) = (0, 0, 0, 0, 0);
        let mut last: Vec<DVec3> = race.entrants.iter().map(|e| e.car.position).collect();
        while race.phase != Phase::Racing || race.time < seconds {
            let rivals = race.rivals_of(PLAYER);
            let input = you.drive(
                &race.entrants[PLAYER].car,
                &track,
                &rivals,
                1.0 / PHYSICS_RATE,
            );
            race.step(&[input]);
            for e in &race.events {
                if let RaceEvent::Contact {
                    a,
                    b: Some(b),
                    impulse,
                } = e
                {
                    contacts += 1;
                    if *a == PLAYER || *b == PLAYER {
                        mine += 1;
                    }
                    if *impulse > 8000.0 {
                        heavy += 1;
                    }
                    if verbose && (*impulse > 8000.0 || *a == PLAYER || *b == PLAYER) {
                        let (ca, cb) = (&race.entrants[*a].car, &race.entrants[*b].car);
                        let ahead = track.delta(ca.track_position.s, cb.track_position.s);
                        print!("seed {seed} ahead(b-a) {ahead:+.1} ");
                        println!("  contact t={:.1} s={:.0} {a}/{b} lat {:+.1}/{:+.1} v {:.0}/{:.0} imp {impulse:.0}", race.time, ca.track_position.s, ca.track_position.lateral, cb.track_position.lateral, ca.speed()*3.6, cb.speed()*3.6);
                    }
                }
            }
            race.events.clear();
            for (index, entrant) in race.entrants.iter().enumerate() {
                let jump = (entrant.car.position - last[index]).length();
                if jump > 6.0 && index != PLAYER {
                    recoveries += 1;
                    let _ = verbose;
                    let ahead = track.delta(
                        race.entrants[PLAYER].car.track_position.s,
                        entrant.car.track_position.s,
                    );
                    if (0.0..150.0).contains(&ahead) {
                        in_my_path += 1;
                    }
                    if verbose {
                        println!(
                            "  recovery t={:.1} car {index} at s={:.0}, {ahead:+.0} m from you",
                            race.time, entrant.car.track_position.s
                        );
                    }
                }
                last[index] = entrant.car.position;
            }
        }
        (contacts, mine, heavy, recoveries, in_my_path)
    }

    #[test]
    #[ignore]
    fn race_with_a_player() {
        if std::env::var("F1_ASSETS").is_err() {
            return;
        }
        let mut total = [0usize; 5];
        for seed in 1..=8 {
            let r = run(seed, 150.0, false);
            for (sum, value) in total.iter_mut().zip([r.0, r.1, r.2, r.3, r.4]) {
                *sum += value;
            }
        }
        println!(
            "TOTAL 8 races: contacts {}, yours {}, heavy {}, recoveries {}, in your path {}",
            total[0], total[1], total[2], total[3], total[4]
        );
        // Before recovery ghosting and edge rejoins, eight races gave ~26
        // heavy contacts and ~16 rejoins just ahead of the player.
        assert!(total[2] <= 14, "{} heavy contacts", total[2]);
        assert!(total[4] <= 8, "{} rejoins in the player's path", total[4]);
        let mut opening = [0usize; 4];
        for seed in 1..=8 {
            let r = run(seed, 25.0, false);
            for (sum, value) in opening.iter_mut().zip([r.0, r.1, r.2, r.3]) {
                *sum += value;
            }
        }
        println!(
            "FIRST 25 s, 8 starts: contacts {}, yours {}, heavy {}, recoveries {}",
            opening[0], opening[1], opening[2], opening[3]
        );
        assert!(
            opening[2] <= 3,
            "{} heavy contacts at the start",
            opening[2]
        );
    }
}

#[cfg(test)]
mod tow_strength {
    use super::*;
    use crate::car::spec::test_wheels;
    #[test]
    #[ignore]
    fn tow_gain_on_kemmel() {
        let Ok(root) = std::env::var("F1_ASSETS") else {
            return;
        };
        let track = Arc::new(Track::load(std::path::Path::new(&root).join("tracks/spa")).unwrap());
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        let speed_after = |gap: Option<f64>| {
            let mut entrants = vec![EntrantConfig {
                name: "You".into(),
                spec: spec.clone(),
                setup: Setup::default(),
                assists: Assists {
                    automatic_gears: true,
                    ..Assists::default()
                },
                ai: None,
                human_slot: 0,
                livery: [1.0; 3],
            }];
            if gap.is_some() {
                entrants.push(EntrantConfig {
                    name: "Ahead".into(),
                    spec: spec.clone(),
                    setup: Setup::default(),
                    assists: Assists {
                        automatic_gears: true,
                        ..Assists::default()
                    },
                    ai: Some(DriverProfile {
                        pace: 0.95,
                        aggression: 0.0,
                        consistency: 1.0,
                    }),
                    human_slot: 1,
                    livery: [1.0; 3],
                });
            }
            let mut race = Race::new(track.clone(), entrants, 3, 7);
            race.phase = Phase::Racing;
            let line = track.racing_line.offsets[track.station_at(1500.0)];
            let place = |car: &mut Car, s: f64| {
                let frame = track.frame_at(s);
                car.position = frame.point_at(line) + glam::DVec3::Y * 0.3;
                car.yaw = frame.tangent.x.atan2(frame.tangent.y);
                car.velocity = car.forward() * 75.0;
                car.track_position = track.project(car.position, None);
                car.powertrain.gear = 8;
            };
            place(&mut race.entrants[0].car, 1500.0);
            if let Some(gap) = gap {
                place(&mut race.entrants[1].car, 1500.0 + gap);
            }
            for e in &mut race.entrants {
                e.timing.distance = e.car.track_position.s;
                e.timing.laps_completed = 1;
            }
            for _ in 0..(6.0 * PHYSICS_RATE) as usize {
                let follow = race
                    .entrants
                    .get(1)
                    .map_or(line, |e| e.car.track_position.lateral);
                let car = &race.entrants[0].car;
                let frame = track.frame_at(car.track_position.s + 30.0);
                let target = frame.point_at(follow);
                let offset = DVec2::new(target.x - car.position.x, target.z - car.position.z);
                let curvature = 2.0 * offset.dot(car.left()) / offset.length_squared().max(1.0);
                let steer = ((car.spec.wheelbase() * curvature).atan()
                    / car.steer_limit(car.speed()))
                .clamp(-1.0, 1.0);
                race.step(&[CarInput {
                    steer,
                    throttle: 1.0,
                    ..CarInput::default()
                }]);
            }
            race.entrants[0].car.speed() * 3.6
        };
        let alone = speed_after(None);
        for gap in [15.0, 30.0, 60.0, 90.0] {
            let gain = speed_after(Some(gap)) - alone;
            assert!(
                gain > 0.,
                "tow at {gap} m must increase speed, got {gain} km/h"
            );
        }
    }
}

/// Authoritative mutable race state, independent of assets and presentation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    #[serde(with = "crate::exact")]
    pub time: f64,
    pub phase: Phase,
    entrants: Vec<EntrantState>,
    order: Vec<usize>,
    finish_order: Vec<usize>,
    rng: Rng,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct EntrantState {
    ghost: f64,
    car: crate::car::CarState,
    #[serde(with = "crate::exact")]
    timing: Timing,
    drs: drs::Tracker,
    safe_pose: (DVec3, f64, usize),
    driver: Option<crate::ai::AiState>,
}
impl Race {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick,
            time: self.time,
            phase: self.phase,
            entrants: self
                .entrants
                .iter()
                .map(|e| EntrantState {
                    ghost: e.ghost,
                    car: e.car.snapshot(),
                    timing: e.timing.clone(),
                    drs: e.drs.clone(),
                    safe_pose: e.safe_pose,
                    driver: match &e.driver {
                        Driver::Ai(ai) => Some(ai.snapshot()),
                        _ => None,
                    },
                })
                .collect(),
            order: self.order.clone(),
            finish_order: self.finish_order.clone(),
            rng: self.rng.clone(),
        }
    }
    /// Apply atomically after checking field shape. Human slots equal grid indices online.
    pub fn restore(&mut self, state: &Snapshot) -> Result<(), String> {
        let n = self.entrants.len();
        let mut order = state.order.clone();
        order.sort_unstable();
        if state.entrants.len() != n
            || order != (0..n).collect::<Vec<_>>()
            || state.finish_order.iter().any(|i| *i >= n)
            || !state.time.is_finite()
        {
            return Err("snapshot does not match this race".into());
        }
        self.tick = state.tick;
        self.time = state.time;
        self.phase = state.phase;
        self.order.clone_from(&state.order);
        self.finish_order.clone_from(&state.finish_order);
        self.rng = state.rng.clone();
        for (slot, (e, s)) in self.entrants.iter_mut().zip(&state.entrants).enumerate() {
            e.car.restore(&s.car);
            e.ghost = s.ghost;
            e.timing = s.timing.clone();
            e.drs = s.drs.clone();
            e.safe_pose = s.safe_pose;
            if let Some(ai) = &s.driver {
                if !matches!(e.driver, Driver::Ai(_)) {
                    e.driver = Driver::Ai(Box::new(AiDriver::new(
                        DriverProfile {
                            pace: 0.95,
                            aggression: 0.5,
                            consistency: 0.9,
                        },
                        &e.car,
                        &self.track,
                        slot as u64 + 1,
                    )));
                }
                if let Driver::Ai(driver) = &mut e.driver {
                    driver.restore(ai, &e.car, &self.track);
                }
            } else {
                e.driver = Driver::Human { slot };
            }
        }
        Ok(())
    }
    /// A departed human remains in the field under server-controlled AI.
    pub fn ai_takeover(&mut self, slot: usize) {
        if let Some(e) = self.entrants.get_mut(slot) {
            e.car.assists.automatic_gears = true;
            e.driver = Driver::Ai(Box::new(AiDriver::new(
                DriverProfile {
                    pace: 0.95,
                    aggression: 0.5,
                    consistency: 0.9,
                },
                &e.car,
                &self.track,
                slot as u64 + 1,
            )));
        }
    }
}

/// Spa passing lanes and tactical line choice. Needs converted assets:
/// `F1_ASSETS=assets cargo test -p racecraft-sim -- --ignored spa_lines`.
#[cfg(test)]
mod spa_lines {
    use super::*;
    use crate::ai::Intent;
    use crate::car::spec::test_wheels;

    fn spa() -> Option<Arc<Track>> {
        let root = std::env::var("F1_ASSETS").ok()?;
        Some(Arc::new(
            Track::load(std::path::Path::new(&root).join("tracks/spa")).unwrap(),
        ))
    }

    fn duel(track: &Arc<Track>, seed: u64) -> Race {
        let spec = Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9));
        let driver = |name: &str, pace: f64| EntrantConfig {
            name: name.into(),
            spec: spec.clone(),
            setup: Setup::default(),
            assists: Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            ai: Some(DriverProfile {
                pace,
                aggression: 0.7,
                consistency: 1.0,
            }),
            human_slot: 0,
            livery: [1.0; 3],
        };
        Race::new(
            track.clone(),
            vec![driver("Slow", 0.93), driver("Fast", 0.99)],
            2,
            seed,
        )
    }

    #[test]
    #[ignore]
    fn spa_lines_stay_on_track_and_are_smooth() {
        let Some(track) = spa() else { return };
        let lanes = track.passing_lines.as_ref().expect("Spa has passing lanes");
        for (side, lane) in lanes.iter().enumerate() {
            let mut apart = 0usize;
            for (i, station) in track.stations.iter().enumerate() {
                let offset = lane.offsets[i];
                assert!(
                    offset.abs() <= station.half_width - 1.4 + 1e-6,
                    "lane {side} leaves the track at station {i}: {offset:.2} m"
                );
                let next = lane.offsets[(i + 1) % lane.offsets.len()];
                assert!(
                    (next - offset).abs() < 0.35,
                    "lane {side} jumps {:.2} m at station {i}",
                    next - offset
                );
                if (offset - track.racing_line.offsets[i]).abs() > 2.0 {
                    apart += 1;
                }
            }
            assert!(
                apart * 3 > track.stations.len(),
                "lane {side} is not a real alternative to the optimal line"
            );
        }
    }

    #[test]
    #[ignore]
    fn spa_lines_a_defender_moves_once_per_corner() {
        let Some(track) = spa() else { return };
        for seed in [2, 4, 6] {
            let mut race = duel(&track, seed);
            let mut sides: Vec<(f64, f64)> = Vec::new();
            let mut ticks = 0;
            while race.phase != Phase::Finished && ticks < (420.0 * PHYSICS_RATE) as usize {
                race.step(&[]);
                if let Driver::Ai(ai) = &race.entrants[0].driver {
                    if let Intent::Defend { side, apex_s } = ai.intent {
                        if let Some(seen) = sides.iter().find(|(apex, _)| *apex == apex_s) {
                            assert_eq!(
                                seen.1, side,
                                "seed {seed}: defender switched sides for the corner at {apex_s:.0} m"
                            );
                        } else {
                            sides.push((apex_s, side));
                        }
                    }
                }
                ticks += 1;
            }
        }
    }

    /// Laps on which the faster car leads, with and without passing lanes.
    #[test]
    #[ignore]
    fn spa_lines_multiple_lines_pass_at_least_as_often_as_one() {
        let Some(track) = spa() else { return };
        let root = std::env::var("F1_ASSETS").unwrap();
        let mut single = Track::load(std::path::Path::new(&root).join("tracks/spa")).unwrap();
        single.passing_lines = None;
        let single = Arc::new(single);
        let passes = |track: &Arc<Track>| {
            let mut won = 0;
            let mut contacts = 0;
            for seed in [1, 3, 5, 7, 9, 11] {
                let mut race = duel(track, seed);
                let mut ticks = 0;
                while race.phase != Phase::Finished && ticks < (420.0 * PHYSICS_RATE) as usize {
                    race.step(&[]);
                    contacts += race
                        .events
                        .iter()
                        .filter(|event| matches!(event, RaceEvent::Contact { b: Some(_), .. }))
                        .count();
                    race.events.clear();
                    ticks += 1;
                }
                if race.entrants[race.order()[0]].name == "Fast" {
                    won += 1;
                }
            }
            (won, contacts)
        };
        let (multi, multi_contacts) = passes(&track);
        let (one, _) = passes(&single);
        eprintln!(
            "faster car won {multi}/6 with lanes, {one}/6 with one line; {multi_contacts} contacts"
        );
        assert!(multi >= one, "lanes made passing harder: {multi} vs {one}");
        assert!(multi_contacts <= 4, "{multi_contacts} contacts with lanes");
    }

    #[test]
    #[ignore]
    fn spa_lines_ai_power_slows_ai_lap_times() {
        let Some(track) = spa() else { return };
        let best = |power: f64| {
            let mut race = duel(&track, 3);
            race.set_ai_power(power);
            let mut ticks = 0;
            while race.phase != Phase::Finished && ticks < (600.0 * PHYSICS_RATE) as usize {
                race.step(&[]);
                ticks += 1;
            }
            race.entrants
                .iter()
                .filter_map(|e| e.timing.best_lap)
                .fold(f64::INFINITY, f64::min)
        };
        let (full, mid, low) = (best(1.0), best(0.8), best(0.5));
        eprintln!("Spa best AI lap: 100% {full:.2}s, 80% {mid:.2}s, 50% {low:.2}s");
        assert!(mid > full + 0.5, "80% power barely slowed the AI");
        assert!(low > mid + 2.0, "50% power barely slowed the AI");
    }
}
