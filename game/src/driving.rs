//! Driver guidance: actual steering/corner-braking interventions, in addition to
//! the simulation's ABS, traction and stability controls. Throttle remains manual.

use racecraft_sim::{
    ai::{AiDriver, DriverProfile, Rival},
    Car, CarInput, Race, Track,
};

#[derive(Clone, Copy, Default)]
pub struct Activity {
    pub steering: bool,
    pub braking: bool,
    pub target_speed: f64,
    pub input: CarInput,
}

pub struct Guidance {
    driver: AiDriver,
    pub activity: Activity,
}

impl Guidance {
    pub fn new(race: &Race, player: usize) -> Self {
        Self {
            driver: AiDriver::new(
                DriverProfile {
                    pace: 0.90,
                    aggression: 0.,
                    consistency: 1.,
                },
                &race.entrants[player].car,
                &race.track,
                0xA1D5,
            ),
            activity: Activity::default(),
        }
    }

    pub fn target_speed(&self, track: &Track, distance: f64) -> f64 {
        self.driver.target_speed(track, distance)
    }

    /// `rivals` are the other cars as the AI sees them, so assisted steering
    /// gives room to a car alongside and pulls out to pass a slower one
    /// instead of following the racing line into it.
    pub fn apply(
        &mut self,
        car: &Car,
        track: &Track,
        rivals: &[Rival],
        raw: CarInput,
        corner_braking: f64,
        dt: f64,
    ) -> CarInput {
        let mut input = raw;
        let reference = self.driver.drive(car, track, rivals, dt);
        let target_speed = self
            .driver
            .target_speed(track, car.track_position.s + car.speed() * 0.12 + 2.);
        // Full manual steering is an override. With no steering input, 100%
        // guidance follows the line. Never accelerate or steer a reversing car.
        if car.powertrain.gear > 0 && car.forward_speed() >= -0.5 {
            let strength = car.assists.steering_guidance() * (1. - raw.steer.abs().powi(2));
            input.steer += (reference.steer - raw.steer) * strength;
            let braking = corner_braking.clamp(0., 1.);
            if braking > 0. {
                // Brake for the track, and for a car ahead only when contact
                // is imminent. The AI's cautious following distance feathered
                // the pedal in the tow, cutting full power and closing DRS
                // just as the driver pulled out to pass.
                let assist_brake = if imminent_contact(car, rivals) {
                    reference.brake
                } else {
                    self.driver.corner_brake()
                };
                input.brake = raw.brake.max(assist_brake * braking);
                // Corner assistance owns braking, not the accelerator. The AI's
                // coasting and traction trims otherwise hold back a held pedal.
                if input.brake > raw.brake {
                    input.throttle *= 1. - input.brake;
                }
            }
        }
        self.activity = Activity {
            steering: (input.steer - raw.steer).abs() > 0.025,
            braking: input.brake > raw.brake + 0.025,
            target_speed,
            input,
        };
        input
    }
}

/// Seconds to contact below which corner assistance also brakes for a car
/// in our lane. Reaction plus a hard stop's worth of closing at racing gaps.
const CONTACT_HORIZON: f64 = 0.6;

/// A car ahead in our lane that we will hit within `CONTACT_HORIZON` at the
/// current closing speed.
fn imminent_contact(car: &Car, rivals: &[Rival]) -> bool {
    let speed = car.speed();
    rivals.iter().any(|rival| {
        let in_lane = (rival.lateral - car.track_position.lateral).abs() < car.spec.width + 0.3;
        let room = rival.gap - car.spec.length;
        let closing = speed - rival.speed;
        in_lane && rival.gap > 0. && closing > 0.5 && room < closing * CONTACT_HORIZON
    })
}

/// Green well under the target speed, amber near it and red over it,
/// blended continuously so the guide never flickers at a threshold.
fn line_colour(over: f64) -> bevy::prelude::Color {
    const GREEN: [f32; 3] = [0.12, 0.9, 0.42];
    const AMBER: [f32; 3] = [1., 0.72, 0.08];
    const RED: [f32; 3] = [1., 0.15, 0.08];
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0., 1.);
        bevy::prelude::Color::srgb(
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        )
    };
    let over = over as f32;
    if over < 0. {
        mix(GREEN, AMBER, (over + 6.) / 4.)
    } else {
        mix(AMBER, RED, (over - 1.) / 5.)
    }
}

/// Racing-line offset interpolated between stations.
fn line_offset(track: &Track, distance: f64) -> f64 {
    use racecraft_sim::track::STATION_SPACING;
    let s = distance.rem_euclid(track.length);
    let index = track.station_at(s);
    let next = (index + 1) % track.racing_line.offsets.len();
    let t = (s / STATION_SPACING).fract();
    let offsets = &track.racing_line.offsets;
    offsets[index] + (offsets[next] - offsets[index]) * t
}

pub fn racing_line(session: bevy::prelude::Res<crate::Session>, mut gizmos: bevy::prelude::Gizmos) {
    use bevy::prelude::*;
    if !session.ready || !session.show_racing_line || session.screen == crate::menu::Screen::Main {
        return;
    }
    let car = &session.race.entrants[session.player].car;
    let track = &session.race.track;
    // Samples sit at fixed lap distances, not relative to the car, so the
    // ribbon stays put on the asphalt instead of re-sampling every frame.
    let spacing = track.length / (track.length / 6.).round().max(1.);
    let start = car.track_position.s + spacing;
    let first = (start / spacing).ceil() * spacing;
    let mut previous = None;
    for distance in std::iter::once(start).chain((0..24).map(|step| first + step as f64 * spacing))
    {
        let frame = track.frame_at(distance);
        let mut point = frame.point_at(line_offset(track, distance));
        if let Some(ground) = track.collision.ground(point + glam::DVec3::Y * 3.) {
            point.y = ground.height;
        }
        point.y += 0.08;
        let target = session.guidance.target_speed(track, distance);
        let color = line_colour(car.speed() - target);
        let world = Vec3::new(point.x as f32, point.y as f32, point.z as f32);
        if let Some(previous) = previous {
            gizmos.line(previous, world, color);
        }
        previous = Some(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires converted game assets"]
    fn spa_guidance_drives_corners_and_shifts_with_only_throttle() {
        let root = std::env::var("F1_TEST_ASSETS").expect("F1_TEST_ASSETS");
        let options = crate::options::Options::parse(
            ["--assets", &root, "--opponents", "0", "--grid", "1"]
                .into_iter()
                .map(String::from),
        )
        .unwrap();
        let run = |enabled: bool| {
            let mut race = crate::assets::load_race(&options).unwrap();
            race.phase = racecraft_sim::Phase::Racing;
            race.time = 0.;
            if !enabled {
                race.entrants[0].car.assists.steering = 0.;
            }
            let mut guide = Guidance::new(&race, 0);
            let mut braking = 0;
            let mut steering = 0;
            let mut offroad = 0;
            let mut gear = 0;
            for _ in 0..(60. * racecraft_sim::PHYSICS_RATE) as usize {
                let input = guide.apply(
                    &race.entrants[0].car,
                    &race.track,
                    &[],
                    CarInput {
                        throttle: 1.,
                        ..Default::default()
                    },
                    if enabled { 1. } else { 0. },
                    1. / racecraft_sim::PHYSICS_RATE,
                );
                braking += usize::from(guide.activity.braking);
                steering += usize::from(guide.activity.steering);
                race.step(&[input]);
                let car = &race.entrants[0].car;
                gear = gear.max(car.powertrain.gear);
                let width = race.track.frame_at(car.track_position.s).half_width;
                offroad += usize::from(car.track_position.lateral.abs() > width);
            }
            (
                braking,
                steering,
                offroad,
                gear,
                race.entrants[0].car.track_position.s,
            )
        };
        let assisted = run(true);
        let manual = run(false);
        println!("Assisted {assisted:?}; no guidance {manual:?}");
        assert!(
            assisted.0 > 100 && assisted.1 > 100,
            "aids did not intervene"
        );
        assert!(assisted.3 >= 4, "automatic gears did not shift");
        assert!(
            assisted.2 < manual.2 / 2,
            "guidance did not keep the car on track"
        );
        assert!(assisted.4 > 1000., "insufficient progress");
    }

    #[test]
    #[ignore = "requires converted game assets"]
    fn spa_straight_allows_full_throttle_beside_a_rival_but_brakes_for_a_blocker() {
        let root = std::env::var("F1_TEST_ASSETS").expect("F1_TEST_ASSETS");
        let options = crate::options::Options::parse(
            ["--assets", &root, "--opponents", "0", "--grid", "1"]
                .into_iter()
                .map(String::from),
        )
        .unwrap();
        let mut race = crate::assets::load_race(&options).unwrap();
        let mut passing_samples = 0;
        for s in (1400..1800).step_by(20) {
            let frame = race.track.frame_at(s as f64);
            let lateral = race.track.racing_line.lateral_at(s as f64);
            let car = &mut race.entrants[0].car;
            *car = Car::new(
                car.spec.clone(),
                car.setup,
                car.assists,
                &race.track,
                frame.point_at(lateral),
                frame.tangent.x.atan2(frame.tangent.y),
            );
            car.velocity = car.forward() * (282. / 3.6);
            car.powertrain.gear = 7;
            let mut guide = Guidance::new(&race, 0);
            let car = &race.entrants[0].car;
            for blocked in [false, true] {
                let ahead = race.track.frame_at(s as f64 + 8.);
                let across = lateral + if blocked { 0. } else { 4. };
                let point = ahead.point_at(across);
                let rival = Rival {
                    gap: 8.,
                    lateral: across,
                    speed: 60.,
                    position: glam::DVec2::new(point.x, point.z),
                };
                let input = guide.apply(
                    car,
                    &race.track,
                    &[rival],
                    CarInput {
                        throttle: 1.,
                        ..Default::default()
                    },
                    1.,
                    1. / 360.,
                );
                println!(
                    "s={s} blocked={blocked} throttle={:.3} brake={:.3}",
                    input.throttle, input.brake
                );
                if blocked {
                    assert!(input.brake > 0.5 && input.throttle < 0.5, "{input:?}");
                } else if input.brake == 0. {
                    passing_samples += 1;
                    assert_eq!(input.throttle, 1.);
                }
            }
        }
        assert!(
            passing_samples >= 10,
            "only {passing_samples} passing samples"
        );
    }

    /// A stadium: two 800 m straights joined by 90 m radius hairpins.
    fn stadium() -> crate::Session {
        use racecraft_sim::track::{Station, STATION_SPACING};
        let mut session = crate::tests::session();
        let (straight, radius) = (800., 90.);
        let bend = std::f64::consts::PI * radius;
        let length = 2. * (straight + bend);
        let stations: Vec<_> = (0..(length / STATION_SPACING) as usize)
            .map(|i| {
                let s = i as f64 * STATION_SPACING;
                let (position, heading) = if s < straight {
                    (glam::DVec2::new(0., s), 0.)
                } else if s < straight + bend {
                    let a = (s - straight) / radius;
                    (
                        glam::DVec2::new(radius - radius * a.cos(), straight + radius * a.sin()),
                        a,
                    )
                } else if s < 2. * straight + bend {
                    (
                        glam::DVec2::new(2. * radius, straight - (s - straight - bend)),
                        std::f64::consts::PI,
                    )
                } else {
                    let a = (s - 2. * straight - bend) / radius;
                    (
                        glam::DVec2::new(radius + radius * a.cos(), -radius * a.sin()),
                        std::f64::consts::PI + a,
                    )
                };
                Station {
                    s,
                    position: glam::DVec3::new(position.x, 0., position.y),
                    tangent: glam::DVec2::new(heading.sin(), heading.cos()),
                    half_width: 7.,
                }
            })
            .collect();
        let track = std::sync::Arc::new(Track {
            name: "Stadium".into(),
            source: String::new(),
            length: stations.len() as f64 * STATION_SPACING,
            racing_line: racecraft_sim::track::racing_line::RacingLine::compute(&stations),
            passing_lines: None,
            stations,
            corners: vec![],
            collision: racecraft_sim::track::collision::CollisionWorld::from_bytes(
                b"F1CL\x01\0\0\0\0\0\0\0\0\0\0\0",
            )
            .unwrap(),
            sun_direction: glam::DVec3::new(-0.3, -0.8, -0.5),
        });
        let config = racecraft_sim::EntrantConfig {
            name: "You".into(),
            spec: session.race.entrants[0].car.spec.clone(),
            setup: session.race.entrants[0].car.setup,
            assists: racecraft_sim::Assists::default(),
            ai: None,
            human_slot: 0,
            livery: [1.; 3],
        };
        session.race = Race::new(track, vec![config], 1, 5);
        session
    }

    /// Sweep of tow gaps and closing speeds on a straight: a car ahead is
    /// never braked for unless contact is imminent, so full throttle (and
    /// with it DRS) survives the run up to a pass.
    #[test]
    fn corner_assist_does_not_feather_the_brake_in_a_tow() {
        let mut session = stadium();
        let mut guide = Guidance::new(&session.race, 0);
        let track = session.race.track.clone();
        let car = &mut session.race.entrants[0].car;
        let s = 200.;
        let frame = track.frame_at(s);
        let lateral = track.racing_line.lateral_at(s);
        *car = Car::new(
            car.spec.clone(),
            car.setup,
            car.assists,
            &track,
            frame.point_at(lateral),
            frame.tangent.x.atan2(frame.tangent.y),
        );
        car.powertrain.gear = 7;
        let (mut tow, mut imminent) = (0, 0);
        for speed in [50., 60., 70.] {
            car.velocity = car.forward() * speed;
            for gap in (8..=60).step_by(2).map(f64::from) {
                for closing in (0..=20).map(f64::from) {
                    let ahead = track.frame_at(s + gap).point_at(lateral);
                    let rival = Rival {
                        gap,
                        lateral,
                        speed: speed - closing,
                        position: glam::DVec2::new(ahead.x, ahead.z),
                    };
                    let raw = CarInput {
                        throttle: 1.,
                        ..Default::default()
                    };
                    let alone = guide.apply(car, &track, &[], raw, 1., 1. / 360.);
                    let input = guide.apply(car, &track, &[rival], raw, 1., 1. / 360.);
                    let room = gap - car.spec.length;
                    if closing > 0.5 && room < closing * CONTACT_HORIZON {
                        imminent += 1;
                        assert!(input.brake > 0., "{speed} {gap} {closing}: {input:?}");
                    } else {
                        tow += 1;
                        assert_eq!(
                            (input.brake, input.throttle),
                            (alone.brake, alone.throttle),
                            "feathered at {speed} m/s, gap {gap}, closing {closing}"
                        );
                    }
                }
            }
        }
        assert!(tow > 1000 && imminent > 20, "{tow} {imminent}");
    }

    #[test]
    fn corner_assist_preserves_the_pedal_when_it_is_not_braking() {
        let mut session = crate::tests::session();
        let mut guide = Guidance::new(&session.race, 0);
        let track = &session.race.track;
        let car = &mut session.race.entrants[0].car;
        let mut samples = 0;
        for speed in (100..650).map(|i| i as f64 / 10.) {
            car.velocity = car.forward() * speed;
            for pedal in [0.4, 1.0] {
                let raw = CarInput {
                    throttle: pedal,
                    ..Default::default()
                };
                let input = guide.apply(car, track, &[], raw, 1., 1. / 360.);
                if input.brake == 0. {
                    samples += 1;
                    assert_eq!(input.throttle, pedal, "speed {speed}: {input:?}");
                }
            }
        }
        assert!(samples > 100, "no unbraked samples exercised");
    }

    #[test]
    fn guidance_turns_and_brakes_for_a_corner_but_off_preserves_manual_input() {
        let mut session = crate::tests::session();
        let track = &session.race.track;
        let car = &mut session.race.entrants[0].car;
        car.assists.steering = 1.;
        car.velocity = car.forward() * 65.;
        let mut guide = Guidance {
            driver: AiDriver::new(
                DriverProfile {
                    pace: 0.9,
                    aggression: 0.,
                    consistency: 1.,
                },
                car,
                track,
                1,
            ),
            activity: Activity::default(),
        };
        let raw = CarInput {
            throttle: 1.,
            ..Default::default()
        };
        let input = guide.apply(car, track, &[], raw, 1., 1. / 360.);
        assert!(input.steer.abs() > 0.02, "no steering guidance");
        assert!(
            input.brake > 0.5 && input.throttle < 0.2,
            "no useful braking intervention: {input:?}"
        );
        assert!(guide.activity.braking && guide.activity.steering);
        car.assists.steering = 0.;
        assert_eq!(guide.apply(car, track, &[], raw, 0., 1. / 360.), raw);
        car.assists.steering = 1.;
        let override_input = CarInput { steer: -1., ..raw };
        assert_eq!(
            guide
                .apply(car, track, &[], override_input, 0., 1. / 360.)
                .steer,
            -1.
        );
    }
}

#[cfg(test)]
mod handling_probe {
    use super::*;
    use racecraft_sim::{Phase, Surface, PHYSICS_RATE};
    const DT: f64 = 1.0 / PHYSICS_RATE;

    #[derive(Clone, Copy, Debug)]
    enum Error {
        Late(f64),
        Off(Surface, usize, f64),
        Exit,
        Weave,
        Lift,
        Clean,
        CleanLate,
        CleanExit,
    }

    fn race(model: &str, opponents: &str) -> Race {
        let root = std::env::var("F1_TEST_ASSETS").expect("F1_TEST_ASSETS");
        let options = crate::options::Options::parse(
            [
                "--assets",
                &root,
                "--car",
                model,
                "--opponents",
                opponents,
                "--grid",
                "1",
                "--laps",
                "10",
            ]
            .into_iter()
            .map(String::from),
        )
        .unwrap();
        let mut race = crate::assets::load_race(&options).unwrap();
        race.phase = Phase::Racing;
        race.time = 0.;
        race
    }

    #[derive(Debug)]
    struct Result {
        spin: bool,
        slip: f64,
        off: f64,
        recovery: Option<f64>,
        gate: Option<f64>,
        surface_ticks: usize,
        two: bool,
        four: bool,
        speed: f64,
        dirt_seconds: f64,
        return_yaw_step: f64,
    }

    /// Free motion throughout: start on the normal grid and use the same
    /// Guidance -> Race::step path as main.rs. No recovery or pose resets.
    fn drive(template: &Race, model: &str, trigger: f64, error: Error) -> Result {
        let mut race = template.clone();
        let mut guide = Guidance::new(&race, 0);
        let mut start = None;
        let mut start_s = trigger;
        let mut released = None;
        let mut steer = 0.0_f64;
        let mut stable = 0;
        let mut previous_dirt = false;
        let mut previous_yaw = 0.0_f64;
        let mut result = Result {
            spin: false,
            slip: 0.,
            off: 0.,
            recovery: None,
            gate: None,
            surface_ticks: 0,
            two: false,
            four: false,
            speed: 0.,
            dirt_seconds: 0.,
            return_yaw_step: 0.,
        };
        for _ in 0..(350. * PHYSICS_RATE) as usize {
            let car = &race.entrants[0].car;
            let s = car.track_position.s;
            let trigger_reached = if matches!(error, Error::Late(_) | Error::CleanLate) {
                s >= trigger - 180.
                    && s <= trigger + 100.
                    && guide.activity.input.brake > 0.05
                    && car.speed() > 25.
            } else {
                s >= trigger
            };
            if start.is_none() && trigger_reached && race.entrants[0].timing.laps_completed >= 1 {
                start_s = s;
                start = Some(race.time);
                result.speed = car.speed() * 3.6;
            }
            let age = start.map_or(-1., |t| race.time - t);
            let mut raw = CarInput {
                throttle: 1.,
                ..Default::default()
            };
            let mut corner_braking = 1.;
            if matches!(error, Error::Exit | Error::CleanExit) && s < trigger && s > trigger - 40. {
                raw.throttle = 0.4;
            }
            if age >= 0. {
                match error {
                    Error::Late(distance) if s < start_s + distance => {
                        corner_braking = 0.;
                    }
                    Error::Off(surface, wheels, direction) if released.is_none() => {
                        let count = car.wheels.iter().filter(|w| w.surface == surface).count();
                        result.two |= count >= 2;
                        result.four |= count == 4;
                        result.surface_ticks += usize::from(count >= wheels);
                        let required_ticks = if wheels == 2 { 1 } else { 36 };
                        if result.surface_ticks >= required_ticks || age > 3. {
                            released = Some(race.time);
                        } else {
                            raw.steer = direction;
                            raw.throttle = 0.4;
                        }
                    }
                    Error::Exit => {}
                    Error::CleanExit => raw.throttle = 0.4,
                    Error::Weave if age < 0.45 => {
                        let target: f64 = if (age / 0.15) as usize == 1 { -1. } else { 1. };
                        let rate = if target.signum() != steer.signum() {
                            7.
                        } else {
                            3.2
                        };
                        steer += (target - steer).clamp(-rate * DT, rate * DT);
                        raw.steer = steer;
                    }
                    Error::Weave if steer != 0. => {
                        steer += (-steer).clamp(-7. * DT, 7. * DT);
                        raw.steer = steer;
                    }
                    Error::Lift if age < 1. => raw.throttle = 0.,
                    _ => {}
                }
            }
            let input = guide.apply(
                &race.entrants[0].car,
                &race.track,
                &[],
                raw,
                corner_braking,
                DT,
            );
            race.step(&[input]);
            if let Some(start) = start {
                let car = &race.entrants[0].car;
                let age = race.time - start;
                let dirt = car
                    .wheels
                    .iter()
                    .filter(|w| matches!(w.surface, Surface::Grass | Surface::Gravel))
                    .count();
                if dirt > 0 {
                    result.dirt_seconds += DT;
                }
                if previous_dirt && dirt == 0 {
                    result.return_yaw_step = result
                        .return_yaw_step
                        .max((car.yaw_rate - previous_yaw).abs());
                }
                previous_dirt = dirt > 0;
                previous_yaw = car.yaw_rate;
                if let Error::Off(surface, _, _) = error {
                    let count = car.wheels.iter().filter(|w| w.surface == surface).count();
                    result.two |= count >= 2;
                    result.four |= count == 4;
                }
                let slip = car
                    .velocity
                    .dot(car.left())
                    .atan2(car.forward_speed())
                    .abs();
                if car.speed() > 5. {
                    result.slip = result.slip.max(slip.to_degrees());
                }
                result.spin |= slip > std::f64::consts::FRAC_PI_2 && car.speed() > 5.;
                result.off = result.off.max(
                    car.track_position.lateral.abs()
                        - race.track.station(car.track_position.station).half_width,
                );
                let line = race.track.racing_line.lateral_at(car.track_position.s);
                let at_pace =
                    car.speed() >= guide.target_speed(&race.track, car.track_position.s) * 0.9;
                let done = match error {
                    Error::Off(..) => released.is_some(),
                    Error::Late(distance) => {
                        car.track_position.s > (start_s + distance).max(trigger + 300.)
                    }
                    _ => age > 1.,
                };
                if done && (car.track_position.lateral - line).abs() < 1.5 && at_pace && slip < 0.1
                {
                    stable += 1;
                    if stable >= 180 && result.recovery.is_none() {
                        result.recovery = Some(age - 0.5);
                    }
                } else {
                    stable = 0;
                }
                if car.track_position.s >= trigger + 450. && result.gate.is_none() {
                    result.gate = Some(age);
                }
                if age >= 30. || (result.gate.is_some() && result.recovery.is_some()) {
                    break;
                }
            }
        }
        assert!(
            start.is_some(),
            "{model} {error:?}: never reached trigger {trigger}"
        );
        result
    }

    #[test]
    #[ignore = "requires converted assets; prints free-driving handling measurements"]
    fn spa_player_mistakes() {
        for model in ["gen22", "rb6"] {
            let template = race(model, "0");
            for (label, trigger, error) in [
                ("la-source-late30", 130., Error::Late(30.)),
                ("la-source-late50", 130., Error::Late(50.)),
                ("les-combes-late30", 2120., Error::Late(30.)),
                ("les-combes-late50", 2120., Error::Late(50.)),
                ("bus-stop-late30", 6480., Error::Late(30.)),
                ("bus-stop-late50", 6480., Error::Late(50.)),
                ("two-grass", 2120., Error::Off(Surface::Grass, 2, 1.)),
                ("four-grass", 2120., Error::Off(Surface::Grass, 4, 1.)),
                ("two-gravel", 2260., Error::Off(Surface::Gravel, 2, -1.)),
                ("four-gravel", 2260., Error::Off(Surface::Gravel, 4, -1.)),
                ("apex-throttle", 258., Error::Exit),
                ("kemmel-weave", 1550., Error::Weave),
                ("pouhon-lift", 3780., Error::Lift),
            ] {
                let clean = drive(
                    &template,
                    model,
                    trigger,
                    if matches!(error, Error::Late(_)) {
                        Error::CleanLate
                    } else if matches!(error, Error::Exit) {
                        Error::CleanExit
                    } else {
                        Error::Clean
                    },
                );
                let result = drive(&template, model, trigger, error);
                assert!(!result.spin, "{model} {label}: {result:?}");
                assert!(
                    result.recovery.is_some(),
                    "{model} {label}: no recovery within 30 s"
                );
                if matches!(error, Error::Off(..)) {
                    assert!(
                        result.surface_ticks > 0,
                        "{model} {label}: missed requested surface"
                    );
                }
                if matches!(error, Error::Weave) {
                    assert!(result.speed >= 280., "{model}: weave below requested speed");
                }
                let lost = result.gate.zip(clean.gate).map(|(a, b)| a - b);
                println!("{model} {label}: {result:?}, lost={lost:?}");
            }
        }
    }
    #[test]
    #[ignore = "requires converted assets; real two-car grazing contacts"]
    fn spa_player_contacts() {
        use racecraft_sim::race::Driver;
        for model in ["gen22", "rb6"] {
            let template = race(model, "1");
            for trigger in [1550., 3780.] {
                for along in [0., -2.0] {
                    for closing in [0.5, 1.5] {
                        let mut race = template.clone();
                        // Keep the rival away until the single initial contact setup.
                        race.entrants[1].ghost = 1000.;
                        let mut guide = Guidance::new(&race, 0);
                        while (race.entrants[0].timing.laps_completed < 1
                            || race.entrants[0].car.track_position.s < trigger)
                            && race.time < 300.
                        {
                            let input = guide.apply(
                                &race.entrants[0].car,
                                &race.track,
                                &[],
                                CarInput {
                                    throttle: 1.,
                                    ..Default::default()
                                },
                                1.,
                                DT,
                            );
                            race.step(&[input]);
                        }
                        race.entrants[1].car = race.entrants[0].car.clone();
                        let player = &race.entrants[0].car;
                        let mut across = 0.0_f64;
                        for (a, ra) in player.collision_circles() {
                            for (b, rb) in player.collision_circles() {
                                let gap = (a - b).dot(player.forward()) - along;
                                across =
                                    across.max(((ra + rb).powi(2) - gap.powi(2)).max(0.).sqrt());
                            }
                        }
                        let offset = player.left() * (across + 0.001) + player.forward() * along;
                        let velocity = player.velocity - player.left() * closing;
                        race.entrants[1].car.translate(offset);
                        race.entrants[1].car.velocity = velocity;
                        race.entrants[1].driver = Driver::Human { slot: 1 };
                        race.entrants[1].ghost = 0.;
                        race.entrants[0].ghost = 0.;
                        let mut rival_guide = Guidance::new(&race, 1);
                        let mut peak_slip = 0.0_f64;
                        let mut off = 0.0_f64;
                        let mut impulse = 0.0_f64;
                        let mut touched = false;
                        for _ in 0..(5. * PHYSICS_RATE) as usize {
                            let raw = CarInput {
                                throttle: 1.,
                                ..Default::default()
                            };
                            let a = guide.apply(
                                &race.entrants[0].car,
                                &race.track,
                                &race.rivals_of(0),
                                raw,
                                1.,
                                DT,
                            );
                            let b = rival_guide.apply(
                                &race.entrants[1].car,
                                &race.track,
                                &race.rivals_of(1),
                                raw,
                                1.,
                                DT,
                            );
                            race.step(&[a, b]);
                            let car = &race.entrants[0].car;
                            touched |= car.impact > 0.;
                            impulse = impulse.max(car.impact);
                            peak_slip = peak_slip.max(
                                car.velocity
                                    .dot(car.left())
                                    .atan2(car.forward_speed())
                                    .abs(),
                            );
                            off = off.max(
                                car.track_position.lateral.abs()
                                    - race.track.station(car.track_position.station).half_width,
                            );
                        }
                        assert!(
                            touched,
                            "{model} s={trigger} along={along} closing={closing}: missed contact"
                        );
                        assert!(
                            peak_slip < std::f64::consts::FRAC_PI_2,
                            "{model}: graze spun player"
                        );
                        println!("{model} contact s={trigger} along={along} closing={closing}: touched={touched} impulse={impulse:.1} slip={:.2} off={off:.2} spin={}", peak_slip.to_degrees(), peak_slip > std::f64::consts::FRAC_PI_2);
                    }
                }
            }
        }
    }
}
