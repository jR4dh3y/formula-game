//! Headless drift checks for the drift-mode car on the generated drift park.
//!
//! ```sh
//! cargo test --release -p racecraft-sim --test drift_probe -- --ignored --nocapture
//! ```

use racecraft_sim::{Assists, Car, CarGeometry, CarInput, CarSpec, Setup, Track, PHYSICS_RATE};
use std::path::PathBuf;
use std::sync::Arc;

fn load() -> Option<(Arc<Track>, Arc<CarSpec>)> {
    let root = std::env::var_os("F1_TEST_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets"));
    let track = Track::load(root.join("tracks/driftpark")).ok()?;
    let geometry = CarGeometry::load(root.join("cars/thar/car.json")).ok()?;
    Some((Arc::new(track), Arc::new(CarSpec::thar_with(&geometry))))
}

fn drift_aids() -> Assists {
    Assists {
        steering: 0.5,
        braking: 0.6,
        traction: 0.0,
        stability: 0.0,
        automatic_gears: true,
        drs_anywhere: false,
        auto_drs: false,
    }
}

fn car_on_straight(track: &Track, spec: &Arc<CarSpec>, speed: f64) -> Car {
    let frame = track.frame_at(0.0);
    let mut car = Car::new(
        spec.clone(),
        Setup::default(),
        drift_aids(),
        track,
        frame.point_at(0.0),
        frame.tangent.x.atan2(frame.tangent.y),
    );
    car.velocity = car.forward() * speed;
    car.powertrain.gear = 2;
    car
}

fn slip_degrees(car: &Car) -> f64 {
    let forward = car.velocity.dot(car.forward());
    let lateral = car.velocity.dot(car.left());
    lateral.atan2(forward.abs().max(0.1)).to_degrees()
}

/// Acceleration, top speed, and a held power slide from full lock.
#[test]
#[ignore = "requires the generated drift park and converted Thar"]
fn thar_drift_envelope() {
    let Some((track, spec)) = load() else {
        panic!("driftpark or thar assets missing; set F1_TEST_ASSETS");
    };
    let dt = 1.0 / PHYSICS_RATE;

    // 0-100 km/h and top speed in a straight line (the pad is flat everywhere).
    let mut car = car_on_straight(&track, &spec, 0.0);
    car.powertrain.gear = 1;
    let (mut t, mut to_100) = (0.0, None);
    while t < 40.0 {
        car.step(
            &CarInput {
                throttle: 1.0,
                ..Default::default()
            },
            &track,
            dt,
        );
        t += dt;
        if to_100.is_none() && car.speed() >= 100.0 / 3.6 {
            to_100 = Some(t);
        }
    }
    println!(
        "0-100 km/h {:?} s, speed after 40 s {:.0} km/h",
        to_100,
        car.speed() * 3.6
    );

    assert!(
        to_100.is_some_and(|t| t < 12.0),
        "Thar failed to reach 100 km/h in 12 s"
    );

    // Power slide: full lock and full throttle from 50 km/h, then hold for 6 s.
    let steer: f64 = std::env::var("DRIFT_STEER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    let mut car = car_on_straight(&track, &spec, 50.0 / 3.6);
    let mut samples = Vec::new();
    for tick in 0..(6.0 * PHYSICS_RATE) as usize {
        car.step(
            &CarInput {
                steer,
                throttle: 1.0,
                ..Default::default()
            },
            &track,
            dt,
        );
        if tick % (PHYSICS_RATE as usize / 2) == 0 {
            samples.push((
                tick as f64 * dt,
                slip_degrees(&car),
                car.speed() * 3.6,
                car.yaw_rate.to_degrees(),
                car.powertrain.gear,
                car.wheels[2].sliding.max(car.wheels[3].sliding),
            ));
        }
    }
    for (time, slip, speed, yaw, gear, rear) in &samples {
        println!(
            "t {time:4.1}s slip {slip:6.1} deg  {speed:5.1} km/h  yaw {yaw:6.1} deg/s  gear {gear}  rear slide {rear:.2}"
        );
    }
}

/// A countersteering controller holding a target slip angle: if it can keep
/// a steady drift, a player can too.
#[test]
#[ignore = "requires the generated drift park and converted Thar"]
fn thar_holds_a_controlled_drift() {
    let Some((track, spec)) = load() else {
        panic!("driftpark or thar assets missing; set F1_TEST_ASSETS");
    };
    let dt = 1.0 / PHYSICS_RATE;
    let target: f64 = std::env::var("DRIFT_TARGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let steering_aid: f64 = std::env::var("DRIFT_AID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let mut car = car_on_straight(&track, &spec, 45.0 / 3.6);
    car.assists.steering = steering_aid;
    let mut log = Vec::new();
    for tick in 0..(10.0 * PHYSICS_RATE) as usize {
        let t = tick as f64 * dt;
        let slip = slip_degrees(&car);
        // Kick it sideways for the first 0.8 s, then catch and hold.
        let (steer_angle, throttle) = if t < 0.8 {
            (car.steer_limit(car.speed()), 1.0)
        } else {
            // Front wheels follow the travel direction (countersteer), with a
            // proportional term on the slip error; throttle holds the angle.
            // In a left drift the slip is negative (travel points right of
            // the nose) and countersteer is negative steer. Over-rotating
            // (|slip| above target) asks for more countersteer.
            let error = (-slip) - target;
            let angle = slip.to_radians() - 0.5 * error.to_radians();
            let throttle = (0.55 + (target - (-slip)) * 0.03).clamp(0.0, 1.0);
            (angle, throttle)
        };
        let steer = (steer_angle / car.steer_limit(car.speed()).max(1e-3)).clamp(-1.0, 1.0);
        car.step(
            &CarInput {
                steer,
                throttle,
                ..Default::default()
            },
            &track,
            dt,
        );
        if tick % (PHYSICS_RATE as usize / 2) == 0 {
            log.push((t, slip, car.speed() * 3.6, steer, throttle));
        }
    }
    for (t, slip, speed, steer, throttle) in &log {
        println!("t {t:4.1}s slip {slip:6.1}  {speed:5.1} km/h  steer {steer:5.2}  throttle {throttle:4.2}");
    }
    let held: Vec<f64> = log.iter().filter(|r| r.0 >= 4.0).map(|r| -r.1).collect();
    let mean = held.iter().sum::<f64>() / held.len() as f64;
    let spread = held.iter().map(|s| (s - mean).abs()).fold(0.0, f64::max);
    println!("held slip {mean:.1} deg (+/- {spread:.1})");
    assert!(
        (12.0..75.0).contains(&mean),
        "held slip outside scoring band: {mean}"
    );
    assert!(spread < 20.0, "unstable held drift: {spread}");
}

/// The generated track works with the race code: an AI Thar laps it.
#[test]
#[ignore = "requires the generated drift park and converted Thar"]
fn ai_thar_laps_the_drift_park() {
    let Some((track, spec)) = load() else {
        panic!("driftpark or thar assets missing; set F1_TEST_ASSETS");
    };
    let config = racecraft_sim::EntrantConfig {
        name: "AI".into(),
        spec,
        setup: Setup::default(),
        assists: Assists::default(),
        ai: Some(racecraft_sim::DriverProfile {
            pace: 1.0,
            aggression: 0.0,
            consistency: 1.0,
        }),
        human_slot: 0,
        livery: [1.0; 3],
    };
    let mut race = racecraft_sim::Race::new(track.clone(), vec![config], 2, 3);
    let mut laps = Vec::new();
    let mut worst_off = 0.0_f64;
    while race.phase != racecraft_sim::Phase::Finished && race.tick < (240.0 * PHYSICS_RATE) as u64
    {
        race.step(&[]);
        let car = &race.entrants[0].car;
        let width = track.frame_at(car.track_position.s).half_width;
        worst_off = worst_off.max(car.track_position.lateral.abs() - width);
        for event in race.events.drain(..) {
            if let racecraft_sim::RaceEvent::LapCompleted { time, .. } = event {
                laps.push(time);
            }
        }
    }
    println!("drift park AI laps {laps:?}, worst distance off the course {worst_off:.1} m");
    assert_eq!(laps.len(), 2, "AI did not complete two laps");
    assert!(worst_off < 3.0, "AI left the course by {worst_off} m");
}
