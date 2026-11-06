//! Steering chatter at speed on the converted circuits: a lone AI car drives
//! two minutes and the probe prints the left-right chatter above 180 km/h
//! (yaw rate, road wheel angle, lateral acceleration and steering limit, each
//! about its own 0.25 s moving average). It asserts nothing.
//!
//! `cargo test --release -p racecraft-sim --test steering_stability -- --ignored --nocapture`
//!
//! Environment: `PROBE_TRACKS=spa,bahrain` (default all four), `PROBE_CAR=rb6`
//! (default gen22), `PROBE_WORST` (the worst one-second stretches) and
//! `PROBE_RANGE=from:to` (every fourth tick between two lap distances, with
//! lateral offset/half width, tyre slip, load and surface).
//!
//! `probe_laps` drives one lap per circuit and car as `tests/tracks.rs` does;
//! `PROBE_PACE` sets the AI pace and `PROBE_AIDS=full` gives the AI the full
//! stability aid, so the player's yaw ceiling runs on the real circuits.
use racecraft_sim::{
    car::WheelState, Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Phase, Race,
    Track,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn assets() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
    root.join("cars/gen22/car.json").is_file().then_some(root)
}

struct Sample {
    s: f64,
    lateral: f64,
    half_width: f64,
    speed: f64,
    yaw_rate: f64,
    steer_angle: f64,
    lateral_acceleration: f64,
    steer_limit: f64,
    wheels: [WheelState; 4],
}

fn spec(root: &Path, model: &str) -> Arc<CarSpec> {
    let geometry = CarGeometry::load(root.join(format!("cars/{model}/car.json"))).unwrap();
    Arc::new(match model {
        "rb6" => CarSpec::rb6_with(&geometry),
        _ => CarSpec::gen22_with(&geometry),
    })
}

/// The race gives AI drivers the default aids; `PROBE_AIDS=full` then
/// raises the stability aid to the player default.
fn set_aids(race: &mut Race) {
    if std::env::var("PROBE_AIDS").is_ok_and(|aids| aids == "full") {
        race.entrants[0].car.assists.stability = 1.0;
    }
}

fn drive(root: &Path, name: &str, seconds: f64) -> Vec<Sample> {
    let model = std::env::var("PROBE_CAR").unwrap_or_else(|_| "gen22".into());
    let spec = spec(root, &model);
    let track = Arc::new(Track::load(root.join(format!("tracks/{name}"))).unwrap());
    let entrant = EntrantConfig {
        name: "AI".into(),
        spec,
        setup: Default::default(),
        assists: Assists {
            automatic_gears: true,
            ..Assists::default()
        },
        ai: Some(DriverProfile {
            pace: 1.0,
            aggression: 0.5,
            consistency: 1.0,
        }),
        human_slot: 0,
        livery: [1.; 3],
    };
    let mut race = Race::new(track, vec![entrant], 50, 1);
    set_aids(&mut race);
    let mut samples = Vec::new();
    while race.phase != Phase::Racing || race.time < seconds {
        race.step(&[]);
        if race.phase != Phase::Racing {
            continue;
        }
        let car = &race.entrants[0].car;
        samples.push(Sample {
            s: car.track_position.s,
            lateral: car.track_position.lateral,
            half_width: race.track.station(car.track_position.station).half_width,
            speed: car.speed(),
            yaw_rate: car.yaw_rate,
            steer_angle: car.steer_angle,
            lateral_acceleration: car.acceleration.y,
            steer_limit: car.steer_limit(car.speed()),
            wheels: car.wheels,
        });
    }
    samples
}

/// RMS of `value` about its centred moving average over `window` samples,
/// over samples above `min_speed`: what is left after the cornering trend.
fn chatter(
    samples: &[Sample],
    value: impl Fn(&Sample) -> f64,
    window: usize,
    min_speed: f64,
) -> f64 {
    let half = window / 2;
    let mut sum = 0.0;
    let mut count = 0;
    for index in half..samples.len().saturating_sub(half) {
        if samples[index].speed < min_speed {
            continue;
        }
        let mean = samples[index - half..=index + half]
            .iter()
            .map(&value)
            .sum::<f64>()
            / (2 * half + 1) as f64;
        sum += (value(&samples[index]) - mean).powi(2);
        count += 1;
    }
    (sum / count.max(1) as f64).sqrt()
}

#[test]
#[ignore]
fn probe_high_speed_chatter() {
    let Some(root) = assets() else {
        eprintln!("assets missing; skipped");
        return;
    };
    let tracks =
        std::env::var("PROBE_TRACKS").unwrap_or_else(|_| "spa,silverstone,bahrain,cota".into());
    let fast = 50.0;
    let window = 90;
    for name in tracks.split(',') {
        let samples = drive(&root, name, 120.0);
        println!(
            "{name:>12}: yaw {:.4} rad/s, wheel {:.5} rad, ay {:.3} m/s2, limit {:.5} rad",
            chatter(&samples, |s| s.yaw_rate, window, fast),
            chatter(&samples, |s| s.steer_angle, window, fast),
            chatter(&samples, |s| s.lateral_acceleration, window, fast),
            chatter(&samples, |s| s.steer_limit, window, fast),
        );
        if let Ok(range) = std::env::var("PROBE_RANGE") {
            let (from, to) = range.split_once(':').unwrap();
            let (from, to): (f64, f64) = (from.parse().unwrap(), to.parse().unwrap());
            for sample in samples
                .iter()
                .filter(|s| s.s >= from && s.s <= to)
                .step_by(4)
            {
                let w = &sample.wheels;
                println!(
                    "   s={:.1} lat={:+.2}/{:.1} v={:.1} r={:+.4} wheel={:+.5} ay={:+.2} slip={:+.3},{:+.3},{:+.3},{:+.3} \
                     load={:.0},{:.0},{:.0},{:.0} surface={:?},{:?},{:?},{:?}",
                    sample.s,
                    sample.lateral,
                    sample.half_width,
                    sample.speed,
                    sample.yaw_rate,
                    sample.steer_angle,
                    sample.lateral_acceleration,
                    w[0].slip_angle,
                    w[1].slip_angle,
                    w[2].slip_angle,
                    w[3].slip_angle,
                    w[0].load,
                    w[1].load,
                    w[2].load,
                    w[3].load,
                    w[0].surface,
                    w[1].surface,
                    w[2].surface,
                    w[3].surface,
                );
            }
        }
        if std::env::var_os("PROBE_WORST").is_some() {
            let mut worst: Vec<(f64, usize)> = (0..samples.len().saturating_sub(360))
                .step_by(180)
                .map(|start| {
                    let stretch = &samples[start..start + 360];
                    (chatter(stretch, |s| s.steer_angle, window, fast), start)
                })
                .collect();
            worst.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (value, start) in worst.iter().take(4) {
                let sample = &samples[start + 180];
                println!(
                    "   s={:.0} {:.0} km/h: wheel chatter {value:.5} rad, limit {:.4}",
                    sample.s,
                    sample.speed * 3.6,
                    sample.steer_limit
                );
            }
        }
    }
}

/// One flying lap per circuit and car, as `tests/tracks.rs` drives them, with
/// the AI pace from `PROBE_PACE` (default 0.96): lap time, worst excursion
/// past the track edge and the yaw chatter above 180 km/h.
#[test]
#[ignore]
fn probe_laps() {
    let Some(root) = assets() else {
        eprintln!("assets missing; skipped");
        return;
    };
    let pace: f64 = std::env::var("PROBE_PACE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.96);
    let tracks =
        std::env::var("PROBE_TRACKS").unwrap_or_else(|_| "spa,silverstone,bahrain,cota".into());
    for name in tracks.split(',') {
        let track = Arc::new(Track::load(root.join(format!("tracks/{name}"))).unwrap());
        for model in ["gen22", "rb6"] {
            let entrant = EntrantConfig {
                name: model.into(),
                spec: spec(&root, model),
                setup: Default::default(),
                assists: Assists::default(),
                ai: Some(DriverProfile {
                    pace,
                    aggression: 0.0,
                    consistency: 1.0,
                }),
                human_slot: 0,
                livery: [1.; 3],
            };
            let mut race = Race::new(track.clone(), vec![entrant], 1, 5);
            set_aids(&mut race);
            let mut worst = (0.0_f64, 0.0);
            let mut samples = Vec::new();
            while race.phase != Phase::Finished && race.time < 300.0 {
                race.step(&[]);
                let car = &race.entrants[0].car;
                let width = track.station(car.track_position.station).half_width;
                let excess = car.track_position.lateral.abs() - width;
                if excess > worst.0 {
                    worst = (excess, car.track_position.s);
                }
                samples.push(Sample {
                    s: car.track_position.s,
                    lateral: car.track_position.lateral,
                    half_width: width,
                    speed: car.speed(),
                    yaw_rate: car.yaw_rate,
                    steer_angle: car.steer_angle,
                    lateral_acceleration: car.acceleration.y,
                    steer_limit: car.steer_limit(car.speed()),
                    wheels: car.wheels,
                });
            }
            let timing = &race.entrants[0].timing;
            println!(
                "{name:>12} {model:>5}: lap {:>7.2} s, off-track {:.2} m at s={:.0}, yaw chatter {:.4} rad/s",
                timing.best_lap.unwrap_or(f64::NAN),
                worst.0,
                worst.1,
                chatter(&samples, |s| s.yaw_rate, 90, 50.0),
            );
        }
    }
}
