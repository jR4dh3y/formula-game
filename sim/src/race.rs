//! A race session: grid, start lights, timing, collisions and AI.
//!
//! `Race::step` is deterministic for a given sequence of human inputs, which is
//! what a future lockstep or server-authoritative netcode layer needs.

use crate::aero;
use crate::ai::{AiDriver, DriverProfile, Rival};
use crate::car::spec::{CarSpec, Setup};
use crate::car::{Assists, Car, CarInput};
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

#[derive(Clone, Debug, Default, PartialEq)]
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
    /// Last on-track pose and its centerline station, used to recover a car
    /// that is stuck or beached.
    safe_pose: (DVec3, f64, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

pub struct Race {
    pub track: Arc<Track>,
    pub entrants: Vec<Entrant>,
    pub laps: u32,
    pub phase: Phase,
    /// Seconds since lights out (negative during the countdown).
    pub time: f64,
    pub tick: u64,
    pub events: Vec<RaceEvent>,
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
                    safe_pose: (position, yaw, track_station),
                }
            })
            .collect();
        let order = (0..entrants.len()).collect();
        Self {
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

        let influences =
            aero::influences(&self.entrants.iter().map(|e| &e.car).collect::<Vec<_>>());
        let rivals: Vec<Vec<Rival>> = (0..self.entrants.len())
            .map(|index| self.rivals_of(index))
            .collect();

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
            entrant.car.step(&input, &self.track, dt);
        }

        self.resolve_wall_contacts();
        self.resolve_car_contacts();
        self.recover_stuck_cars();
        self.update_timing();
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

    /// A pose on the racing line near `s` that no other car occupies, so
    /// recovered cars never land inside a pile-up and get recovered again.
    fn clear_spot(&self, index: usize, s: f64) -> (DVec3, f64) {
        let clearance = self.entrants[index].car.spec.length * 1.6;
        let occupied = |point: DVec3| {
            self.entrants.iter().enumerate().any(|(other, entrant)| {
                other != index
                    && DVec2::new(
                        entrant.car.position.x - point.x,
                        entrant.car.position.z - point.z,
                    )
                    .length()
                        < clearance
            })
        };
        let pose = |along: f64, side: f64| {
            let s = s + along;
            let frame = self.track.frame_at(s);
            let line = self.track.racing_line.offsets[self.track.station_at(s)];
            let limit = (frame.half_width - 1.5).max(0.0);
            let position = frame.point_at((line + side).clamp(-limit, limit));
            (position, frame.tangent.x.atan2(frame.tangent.y))
        };
        // Prefer a short step forward (out of the queue), then further back.
        for along in [0.0, 12.0, 24.0, -12.0, 36.0, -24.0, 48.0, 60.0] {
            for side in [0.0, 3.5, -3.5] {
                let (position, yaw) = pose(along, side);
                if !occupied(position) {
                    return (position, yaw);
                }
            }
        }
        pose(0.0, 0.0)
    }

    /// Moves a car to a new pose and keeps its race distance consistent.
    fn teleport(&mut self, index: usize, position: DVec3, yaw: f64, station: usize) {
        let track = self.track.clone();
        let entrant = &mut self.entrants[index];
        let before = entrant.car.track_position.s;
        reset_car(&mut entrant.car, &track, position, yaw, station);
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
        if self.phase == Phase::Finishing
            && self
                .entrants
                .iter()
                .all(|entrant| entrant.timing.finish_time.is_some())
        {
            self.phase = Phase::Finished;
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
        Some(distance / self.entrants[entrant].car.speed().max(20.0))
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
        let spec = Arc::new(CarSpec::rb22(test_wheels(), 5.4, 1.9));
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
