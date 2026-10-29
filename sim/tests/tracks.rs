//! Full-lap checks for generated tracks. Source-only checkouts skip absent assets.
//! Both car models run time trials, so collisions and deliberate driver mistakes
//! cannot conceal errors in the road, widths, walls or centreline.

use racecraft_sim::{
    Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Phase, Race, Setup, Surface,
    Track, PHYSICS_RATE,
};
use std::{path::PathBuf, sync::Arc};

fn drive(name: &str, reference_length: f64) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
    let path = root.join("tracks").join(name);
    if !path.join("track.json").is_file() {
        eprintln!("skipping {name}: run tools/build_assets.sh {name}");
        return;
    }
    let track = Arc::new(Track::load(&path).expect("generated track must load"));
    assert!(
        (track.length / reference_length - 1.0).abs() < 0.02,
        "{name}: lap length {}",
        track.length
    );
    let models = [
        CarSpec::rb22_with(&CarGeometry::load(root.join("cars/rb22/car.json")).expect("rb22")),
        CarSpec::rb6_with(&CarGeometry::load(root.join("cars/rb6/car.json")).expect("rb6")),
    ];
    for spec in models {
        drive_lap(name, track.clone(), Arc::new(spec));
    }
}

fn drive_lap(name: &str, track: Arc<Track>, spec: Arc<CarSpec>) {
    let model = spec.name.clone();
    let entrant = EntrantConfig {
        name: model.clone(),
        spec,
        setup: Setup::default(),
        assists: Assists::default(),
        ai: Some(DriverProfile {
            pace: 0.96,
            aggression: 0.0,
            consistency: 1.0,
        }),
        human_slot: 0,
        livery: [1.0; 3],
    };
    let mut race = Race::new(track.clone(), vec![entrant], 1, 5);
    let mut worst_offtrack = 0.0_f64;
    let mut worst_at = 0.0;
    let mut off_road = 0;
    let mut samples = 0;
    for _ in 0..(300.0 * PHYSICS_RATE) as usize {
        race.step(&[]);
        let car = &race.entrants[0].car;
        let width = track.station(car.track_position.station).half_width;
        let excess = car.track_position.lateral.abs() - width;
        if excess > worst_offtrack {
            worst_at = car.track_position.s;
        }
        worst_offtrack = worst_offtrack.max(excess);
        if !matches!(
            track.collision.ground(car.position).map(|hit| hit.surface),
            Some(Surface::Asphalt | Surface::Kerb | Surface::Concrete)
        ) {
            off_road += 1;
        }
        samples += 1;
        if race.phase == Phase::Finished {
            break;
        }
    }
    let entrant = &race.entrants[0];
    let off_fraction = off_road as f64 / samples as f64;
    println!("{name} {model}: laps {}, lap {:?}, off-track {:.2} m at s={worst_at:.1}, off racing surface {:.2}%",
        entrant.timing.laps_completed, entrant.timing.best_lap, worst_offtrack, off_fraction * 100.0);
    assert!(
        entrant.timing.laps_completed >= 1,
        "{name} {model}: stuck at s={:.1}, v={:.1}",
        entrant.car.track_position.s,
        entrant.car.speed()
    );
    assert!(
        worst_offtrack < 5.0,
        "{name} {model}: left track by {worst_offtrack:.1} m at s={worst_at:.1}"
    );
    assert!(
        off_fraction < 0.05,
        "{name} {model}: spent {:.1}% off racing surface",
        off_fraction * 100.0
    );
    assert_eq!(race.phase, Phase::Finished, "{name} {model}: must finish");
}

#[test]
fn silverstone() {
    drive("silverstone", 5891.0);
}
#[test]
fn bahrain() {
    drive("bahrain", 5412.0);
}
#[test]
fn cota() {
    drive("cota", 5513.0);
}
#[test]
fn nurburgring_gp() {
    drive("nurburgring-gp", 5148.0);
}
