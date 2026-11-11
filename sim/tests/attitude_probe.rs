//! Visual body attitude on a real lap: roll, pitch and road wheel angle.
//! `cargo test --release -p racecraft-sim --test attitude_probe -- --ignored --nocapture`
//! The weave probe reports the drawn road wheel angle; `PHYS=1` reports the physics angle.
use racecraft_sim::{
    Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Phase, Race, Track,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Speed, roll, pitch, road wheel angle, longitudinal and lateral acceleration.
type Row = (f64, f64, f64, f64, f64, f64);

#[test]
#[ignore = "prints body attitude"]
fn probe_attitude() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
    if !root.join("cars/gen22/car.json").is_file() {
        return;
    }
    let geometry = CarGeometry::load(root.join("cars/gen22/car.json")).unwrap();
    let spec = Arc::new(CarSpec::gen22_with(&geometry));
    let track = Arc::new(Track::load(root.join("tracks/spa")).unwrap());
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
    race.entrants[0].car.assists.stability = 1.0;
    let mut rows: Vec<Row> = Vec::new();
    let mut relative: Vec<(f64, f64, f64)> = Vec::new();
    while race.phase != Phase::Racing || race.time < 120.0 {
        race.step(&[]);
        if race.phase != Phase::Racing {
            continue;
        }
        let c = &race.entrants[0].car;
        rows.push((
            c.speed(),
            c.roll,
            c.pitch,
            c.steer_angle,
            c.acceleration.x,
            c.acceleration.y,
        ));
        let h: [f64; 4] = std::array::from_fn(|i| c.wheels[i].ground_height);
        let track = c.spec.wheels[0].position.x - c.spec.wheels[1].position.x;
        let base = c.spec.wheels[0].position.z - c.spec.wheels[2].position.z;
        let road_roll = (((h[0] + h[2]) - (h[1] + h[3])) * 0.5 / track).atan();
        let road_pitch = (((h[0] + h[1]) - (h[2] + h[3])) * 0.5 / base).atan();
        relative.push((c.speed(), c.roll - road_roll, c.pitch - road_pitch));
    }
    let deg = 180.0 / std::f64::consts::PI;
    let max = |f: &dyn Fn(&Row) -> f64| rows.iter().map(f).fold(0.0_f64, |a, b| a.max(b.abs()));
    println!(
        "max roll {:.2} deg, max pitch {:.2} deg, max lat {:.1} g, max long {:.1} g",
        max(&|r| r.1) * deg,
        max(&|r| r.2) * deg,
        max(&|r| r.5) / 9.81,
        max(&|r| r.4) / 9.81
    );
    let rmax = |f: &dyn Fn(&(f64, f64, f64)) -> f64| {
        relative.iter().map(f).fold(0.0_f64, |a, b| a.max(b.abs()))
    };
    let rate = relative
        .windows(2)
        .map(|w| ((w[1].1 - w[0].1) * 360.0).abs())
        .fold(0.0_f64, f64::max);
    let prate = relative
        .windows(2)
        .map(|w| ((w[1].2 - w[0].2) * 360.0).abs())
        .fold(0.0_f64, f64::max);
    println!("relative to road: max roll {:.2} deg, max pitch {:.2} deg, max roll rate {:.1} deg/s, max pitch rate {:.1} deg/s",
        rmax(&|r| r.1) * deg, rmax(&|r| r.2) * deg, rate * deg, prate * deg);
    // Tick-to-tick road wheel change, per speed band.
    for (lo, hi) in [(0.0, 30.0), (30.0, 60.0), (60.0, 100.0)] {
        let band: Vec<_> = rows
            .windows(2)
            .filter(|w| w[0].0 >= lo && w[0].0 < hi)
            .collect();
        let steer = band.iter().map(|w| w[0].3.abs()).fold(0.0_f64, f64::max);
        let jitter = band
            .windows(2)
            .map(|p| ((p[1][1].3 - p[1][0].3) - (p[0][1].3 - p[0][0].3)).abs())
            .sum::<f64>()
            / band.len().max(1) as f64;
        let roll_rate = band
            .iter()
            .map(|w| ((w[1].1 - w[0].1) * 360.0).abs())
            .fold(0.0_f64, f64::max);
        println!("{lo}-{hi} m/s: max steer {:.2} deg, steer 2nd-diff {:.5} deg/tick, max roll rate {:.1} deg/s",
            steer * deg, jitter * deg, roll_rate * deg);
    }
}

#[test]
#[ignore = "prints player road wheel chatter under a key weave"]
fn probe_player_weave() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
    if !root.join("cars/gen22/car.json").is_file() {
        return;
    }
    let geometry = CarGeometry::load(root.join("cars/gen22/car.json")).unwrap();
    let spec = Arc::new(CarSpec::gen22_with(&geometry));
    let track = Arc::new(Track::load(root.join("tracks/spa")).unwrap());
    let entrant = EntrantConfig {
        name: "P".into(),
        spec,
        setup: Default::default(),
        assists: Assists {
            automatic_gears: true,
            ..Assists::default()
        },
        ai: None,
        human_slot: 0,
        livery: [1.; 3],
    };
    let mut race = Race::new(track, vec![entrant], 50, 1);
    let mut start = None;
    let mut steer_vis = Vec::new();
    loop {
        let t: f64 = start.map_or(0.0, |s: f64| race.time - s);
        let steer = if race.time > 0.0 && t > 6.0 {
            if ((t - 6.0) * 1.5).fract() < 0.5 {
                1.0
            } else {
                -1.0
            }
        } else {
            0.0
        };
        let input = racecraft_sim::CarInput {
            throttle: 1.0,
            steer,
            ..Default::default()
        };
        race.step(&[input]);
        if race.phase != Phase::Racing {
            continue;
        }
        start.get_or_insert(race.time);
        let c = &race.entrants[0].car;
        if t > 6.0 {
            steer_vis.push((
                c.speed(),
                if std::env::var("PHYS").is_ok() {
                    c.steer_angle
                } else {
                    c.visual_steer
                },
                c.yaw_rate,
            ));
        }
        if t > 10.0 {
            break;
        }
    }
    let deg = 180.0 / std::f64::consts::PI;
    let max = steer_vis.iter().map(|r| r.1.abs()).fold(0.0_f64, f64::max);
    let jerk = steer_vis
        .windows(3)
        .map(|w| (w[2].1 - 2.0 * w[1].1 + w[0].1).abs())
        .fold(0.0_f64, f64::max);
    let reversals = steer_vis
        .windows(3)
        .filter(|w| {
            (w[2].1 - w[1].1) * (w[1].1 - w[0].1) < 0.0
                && (w[2].1 - w[1].1).abs().min((w[1].1 - w[0].1).abs())
                    > 0.05_f64.to_radians() / 10.0
        })
        .count();
    if let Some(i) = (2..steer_vis.len()).max_by(|&a, &b| {
        let d = |i: usize| (steer_vis[i].1 - 2.0 * steer_vis[i - 1].1 + steer_vis[i - 2].1).abs();
        d(a).total_cmp(&d(b))
    }) {
        for r in &steer_vis[i.saturating_sub(12)..(i + 12).min(steer_vis.len())] {
            println!("  v={:.1} steer={:.2} yaw_rate={:.3}", r.0, r.1 * deg, r.2);
        }
    }
    for (i, w) in steer_vis.windows(3).enumerate() {
        if (w[2].1 - w[1].1) * (w[1].1 - w[0].1) < 0.0 {
            print!("{}:{:.2} ", i, w[1].1 * deg);
        }
    }
    println!();
    println!("weave at {:.0}-{:.0} m/s: max steer {:.2} deg, max 2nd diff {:.4} deg/tick, direction reversals {reversals} in {} ticks",
        steer_vis.first().unwrap().0, steer_vis.last().unwrap().0, max * deg, jerk * deg, steer_vis.len());
}
