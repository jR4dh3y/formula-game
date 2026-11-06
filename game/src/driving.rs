//! Driver guidance: actual steering/corner-braking interventions, in addition to
//! the simulation's ABS, traction and stability controls. Throttle remains manual.

use racecraft_sim::{
    ai::{AiDriver, DriverProfile},
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

    pub fn apply(
        &mut self,
        car: &Car,
        track: &Track,
        raw: CarInput,
        corner_braking: f64,
        dt: f64,
    ) -> CarInput {
        let mut input = raw;
        let reference = self.driver.drive(car, track, &[], dt);
        let target_speed = self
            .driver
            .target_speed(track, car.track_position.s + car.speed() * 0.12 + 2.);
        // Full manual steering is an override. With no steering input, 100%
        // guidance follows the line. Never accelerate or steer a reversing car.
        if car.powertrain.gear > 0 && car.forward_speed() >= -0.5 {
            let strength = car.assists.steering * (1. - raw.steer.abs().powi(2));
            input.steer += (reference.steer - raw.steer) * strength;
            let braking = corner_braking.clamp(0., 1.);
            if braking > 0. {
                input.brake = raw.brake.max(reference.brake * braking);
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
                let input = guide.apply(car, track, raw, 1., 1. / 360.);
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
        let input = guide.apply(car, track, raw, 1., 1. / 360.);
        assert!(input.steer.abs() > 0.02, "no steering guidance");
        assert!(
            input.brake > 0.5 && input.throttle < 0.2,
            "no useful braking intervention: {input:?}"
        );
        assert!(guide.activity.braking && guide.activity.steering);
        car.assists.steering = 0.;
        assert_eq!(guide.apply(car, track, raw, 0., 1. / 360.), raw);
        car.assists.steering = 1.;
        let override_input = CarInput { steer: -1., ..raw };
        assert_eq!(
            guide.apply(car, track, override_input, 0., 1. / 360.).steer,
            -1.
        );
    }
}
