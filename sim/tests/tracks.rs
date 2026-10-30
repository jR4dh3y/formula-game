//! Full-lap checks for generated tracks. Source-only checkouts skip absent assets.
//! Both car models run time trials, so collisions and deliberate driver mistakes
//! cannot conceal errors in the road, widths, walls or centreline.

use racecraft_sim::{
    Assists, Car, CarGeometry, CarInput, CarSpec, DriverProfile, EntrantConfig, Phase, Race, Setup,
    Surface, Track, PHYSICS_RATE,
};
use std::{path::PathBuf, sync::Arc};

fn drive(name: &str, reference_length: f64) {
    let root = std::env::var_os("F1_TEST_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets"));
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
        CarSpec::gen22_with(&CarGeometry::load(root.join("cars/gen22/car.json")).expect("gen22")),
        CarSpec::rb6_with(&CarGeometry::load(root.join("cars/rb6/car.json")).expect("rb6")),
    ];
    inspect_route(name, track.clone(), Arc::new(models[0].clone()));
    for spec in models {
        drive_lap(name, track.clone(), Arc::new(spec));
    }
}

fn inspect_route(name: &str, track: Arc<Track>, spec: Arc<CarSpec>) {
    let mut missing = 0;
    let mut non_racing = 0;
    let mut worst_height_error = 0.0_f64;
    for station in &track.stations {
        match track.collision.ground(station.position) {
            Some(hit) => {
                non_racing += usize::from(!matches!(
                    hit.surface,
                    Surface::Asphalt | Surface::Kerb | Surface::Concrete
                ));
                worst_height_error =
                    worst_height_error.max((hit.height - station.position.y).abs());
            }
            None => {
                missing += 1;
                println!(
                    "{name}: missing ground s={:.1} at {:?}",
                    station.s, station.position
                );
            }
        }
    }
    assert_eq!(
        missing, 0,
        "{name}: every route station needs physical ground"
    );
    assert_eq!(
        non_racing, 0,
        "{name}: route stations must stay on racing surfaces"
    );
    for slot in 0..16 {
        let (position, yaw) = track.grid_slot(slot);
        let car = Car::new(
            spec.clone(),
            Setup::default(),
            Assists::default(),
            &track,
            position,
            yaw,
        );
        assert!(
            matches!(
                track.collision.ground(car.position).map(|hit| hit.surface),
                Some(Surface::Asphalt | Surface::Kerb | Surface::Concrete)
            ),
            "{name}: grid slot {slot} has no racing surface"
        );
        assert!(
            car.forward()
                .dot(track.frame_at(car.track_position.s).tangent)
                > 0.99
        );
    }
    // Exercise the real safe-pose recovery path on straights, bends and hills.
    let mut recoveries = 0;
    for station in track.stations.iter().step_by(100) {
        let entrant = EntrantConfig {
            name: "Recovery probe".into(),
            spec: spec.clone(),
            setup: Setup::default(),
            assists: Assists::default(),
            ai: None,
            human_slot: 0,
            livery: [1.; 3],
        };
        let mut race = Race::new(track.clone(), vec![entrant], 1, 5);
        race.phase = Phase::Racing;
        let yaw = station.tangent.x.atan2(station.tangent.y);
        race.entrants[0].car = Car::new(
            spec.clone(),
            Setup::default(),
            Assists::default(),
            &track,
            station.position,
            yaw,
        );
        race.entrants[0].car.velocity = station.tangent * 12.;
        race.step(&[CarInput::default()]);
        let safe = race.entrants[0].car.position;
        race.entrants[0].car.position += glam::DVec3::new(80., 20., 80.);
        race.recover(0);
        let car = &race.entrants[0].car;
        assert!(
            car.position.distance(safe) < 1.,
            "{name}: recovery moved away from safe pose at s={}",
            station.s
        );
        assert!(car.speed() < 0.1);
        assert!(car.forward().dot(station.tangent) > 0.99);
        assert!(
            matches!(
                track.collision.ground(car.position).map(|hit| hit.surface),
                Some(Surface::Asphalt | Surface::Kerb | Surface::Concrete)
            ),
            "{name}: recovery has no road at s={}",
            station.s
        );
        recoveries += 1;
    }
    println!("{name}: {} route probes, {missing} missing ground, {non_racing} non-racing, maximum route/ground height difference {worst_height_error:.3} m, {recoveries} recovery probes", track.stations.len());
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
