//! Full-track checks against the generated Spa assets (`tools/build_assets.sh`).
//! Skipped when the assets have not been generated.

use racecraft_sim::{
    Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Phase, Race, RaceEvent, Setup,
    Track, PHYSICS_RATE,
};
use std::path::PathBuf;
use std::sync::Arc;

fn assets() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets");
    root.join("tracks/spa/track.json").exists().then_some(root)
}

#[test]
fn ai_field_races_spa() {
    let Some(root) = assets() else {
        eprintln!("skipping: Spa assets not generated");
        return;
    };
    let track = Arc::new(Track::load(root.join("tracks/spa")).expect("track"));
    let rb22 = Arc::new(CarSpec::rb22_with(
        &CarGeometry::load(root.join("cars/rb22/car.json")).expect("rb22"),
    ));
    let rb6 = Arc::new(CarSpec::rb6_with(
        &CarGeometry::load(root.join("cars/rb6/car.json")).expect("rb6"),
    ));
    let entrants: Vec<_> = (0..6)
        .map(|index| EntrantConfig {
            name: format!("AI {index}"),
            spec: if index % 2 == 0 {
                rb22.clone()
            } else {
                rb6.clone()
            },
            setup: Setup::default(),
            assists: Assists::default(),
            ai: Some(DriverProfile {
                pace: 0.95 + 0.008 * index as f64,
                aggression: 0.3 + 0.1 * index as f64,
                consistency: 0.9,
            }),
            human_slot: 0,
            livery: [1.0; 3],
        })
        .collect();
    let mut race = Race::new(track.clone(), entrants, 1, 5);
    let limit = (260.0 * PHYSICS_RATE) as usize;
    let mut worst_offtrack = vec![0.0_f64; race.entrants.len()];
    let mut top_speed = 0.0_f64;
    for tick in 0..limit {
        race.step(&[]);
        for (index, entrant) in race.entrants.iter().enumerate() {
            let car = &entrant.car;
            let half_width = track.station(car.track_position.station).half_width;
            worst_offtrack[index] =
                worst_offtrack[index].max(car.track_position.lateral.abs() - half_width);
            top_speed = top_speed.max(car.speed());
        }
        if tick % (10 * PHYSICS_RATE as usize) == 0 {
            let leader = &race.entrants[race.order()[0]];
            println!(
                "t={:6.1} leader s={:7.1} v={:5.1} km/h tyres F{:5.1} R{:5.1} gear {}",
                race.time,
                leader.car.track_position.s,
                leader.car.speed() * 3.6,
                leader.car.tyres[0].temperature,
                leader.car.tyres[1].temperature,
                leader.car.powertrain.gear
            );
        }
        if race.phase == Phase::Finished {
            break;
        }
    }
    let contacts = race
        .events
        .iter()
        .filter(|event| matches!(event, RaceEvent::Contact { .. }))
        .count();
    for &index in race.order() {
        let entrant = &race.entrants[index];
        println!(
            "{:6} lap {:?} best {:?} worst off-track {:.1} m",
            entrant.name,
            entrant.timing.laps_completed,
            entrant.timing.best_lap,
            worst_offtrack[index]
        );
    }
    println!("top speed {:.0} km/h, contacts {contacts}", top_speed * 3.6);
    assert_eq!(race.phase, Phase::Finished, "the race should finish");
    let best = race
        .entrants
        .iter()
        .filter_map(|entrant| entrant.timing.best_lap)
        .fold(f64::INFINITY, f64::min);
    // Real F1 laps at Spa are 1:44-1:50; a standing-start lap adds a few seconds.
    assert!((95.0..150.0).contains(&best), "best lap {best:.1}s");
    assert!(top_speed * 3.6 > 290.0, "top speed {:.0}", top_speed * 3.6);
}
