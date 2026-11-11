//! Headless era comparison: a lap-time, top-speed and energy model of each car
//! era on the real circuits, run through the full simulation instead of the
//! game. Tune 2026 numbers here, then confirm in the game.
//!
//! ```sh
//! cargo test --release -p racecraft-sim --test era_probe -- --ignored --nocapture
//! ERA_TRACKS=spa,bahrain ERA_LAPS=3 cargo test ...   # narrow it down
//! ```

use racecraft_sim::hybrid::DeployMode;
use racecraft_sim::{
    CarGeometry, CarSpec, DriverProfile, EntrantConfig, Phase, Race, RaceEvent, Setup, Track,
    PHYSICS_RATE,
};
use std::path::PathBuf;
use std::sync::Arc;

fn root() -> Option<PathBuf> {
    let root = std::env::var_os("F1_TEST_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets"));
    root.join("cars/gen22/car.json").is_file().then_some(root)
}

fn eras(root: &std::path::Path) -> Vec<(&'static str, Arc<CarSpec>)> {
    let geometry = |dir: &str| CarGeometry::load(root.join(format!("cars/{dir}/car.json")));
    let mut eras = vec![
        (
            "RB6",
            Arc::new(CarSpec::rb6_with(&geometry("rb6").unwrap())),
        ),
        (
            "GEN22",
            Arc::new(CarSpec::gen22_with(&geometry("gen22").unwrap())),
        ),
    ];
    // The 2026 body; falls back to the 2022 geometry when it is not installed.
    let gen26 = geometry("rb22").or_else(|_| geometry("gen22")).unwrap();
    eras.push(("GEN26", Arc::new(tuned(CarSpec::gen26_with(&gen26)))));
    if let Ok(only) = std::env::var("ERA_CARS") {
        eras.retain(|(name, _)| only.split(',').any(|o| o.eq_ignore_ascii_case(name)));
    }
    eras
}

/// `GEN26_<NAME>=value` overrides for parameter sweeps without rebuilding.
fn tuned(mut spec: CarSpec) -> CarSpec {
    let var = |name: &str| {
        std::env::var(format!("GEN26_{name}"))
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
    };
    let hybrid = spec.hybrid.as_mut().unwrap();
    let active = spec.aero.active.as_mut().unwrap();
    if let Some(v) = var("ICE") {
        spec.engine.max_power = v * 1e3;
    }
    if let Some(v) = var("MASS") {
        spec.mass = v;
    }
    if let Some(v) = var("LIFT") {
        spec.aero.front_lift_area *= v;
        spec.aero.rear_lift_area *= v;
    }
    if let Some(v) = var("DRAG") {
        spec.aero.drag_area = v;
    }
    if let Some(v) = var("XDRAG") {
        active.drag = v;
    }
    if let Some(v) = var("COAST") {
        hybrid.coast_power = v * 1e3;
    }
    if let Some(v) = var("REGEN") {
        hybrid.regen_power = v * 1e3;
    }
    if let Some(v) = var("RESERVE") {
        hybrid.reserve = v;
    }
    if let Some(v) = var("FULL") {
        hybrid.full_deploy = v;
    }
    if let Some(v) = var("HARVEST") {
        hybrid.lap_harvest = v * 1e6;
    }
    if let Some(v) = var("CLIP") {
        hybrid.clip_power = v * 1e3;
    }
    if let Some(v) = var("CORNER") {
        hybrid.corner_power = v * 1e3;
    }
    if let Some(v) = var("GRIP") {
        spec.tyre_friction = v;
    }
    spec
}

fn tracks(root: &std::path::Path) -> Vec<(String, Arc<Track>)> {
    let names =
        std::env::var("ERA_TRACKS").unwrap_or_else(|_| "spa,bahrain,silverstone,cota".into());
    names
        .split(',')
        .filter_map(|name| {
            Track::load(root.join("tracks").join(name))
                .ok()
                .map(|track| (name.to_string(), Arc::new(track)))
        })
        .collect()
}

fn entrant(spec: &Arc<CarSpec>, pace: f64, aggression: f64) -> EntrantConfig {
    EntrantConfig {
        name: spec.name.clone(),
        spec: spec.clone(),
        setup: Setup::default(),
        assists: Default::default(),
        ai: Some(DriverProfile {
            pace,
            aggression,
            consistency: 1.0,
        }),
        human_slot: 0,
        livery: [1.0; 3],
    }
}

#[derive(Default, Debug)]
struct Lap {
    time: f64,
    top: f64,
    deployed: f64,
    harvested: f64,
    charge_min: f64,
    charge_end: f64,
    straight_mode: f64,
}

/// One car alone: lap times, top speed and per-lap energy.
#[test]
#[ignore = "requires converted assets; prints the era comparison"]
fn era_laps() {
    let Some(root) = root() else {
        eprintln!("skipping: assets missing");
        return;
    };
    let laps: u32 = std::env::var("ERA_LAPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let dt = 1.0 / PHYSICS_RATE;
    println!(
        "{:<12} {:<6} {:>4} {:>9} {:>7} {:>7} {:>7} {:>6} {:>6} {:>7}",
        "track", "era", "lap", "time", "top", "dep MJ", "hrv MJ", "SOCmin", "SOCend", "X-mode"
    );
    for (track_name, track) in tracks(&root) {
        for (era, spec) in eras(&root) {
            let mut race = Race::new(track.clone(), vec![entrant(&spec, 1.0, 0.0)], laps, 7);
            let mut lap = Lap {
                charge_min: 1.0,
                ..Lap::default()
            };
            let mut done = 0;
            let limit = (laps as f64 * 260.0 * PHYSICS_RATE) as u64;
            while done < laps && race.tick < limit {
                race.step(&[]);
                let car = &race.entrants[0].car;
                if race.phase == Phase::Racing || race.phase == Phase::Finishing {
                    lap.top = lap.top.max(car.speed() * 3.6);
                    if let Some(hybrid) = &spec.hybrid {
                        let power = car.battery.power;
                        if power > 0.0 {
                            lap.deployed += power * dt;
                        } else {
                            lap.harvested -= power * dt;
                        }
                        lap.charge_min = lap.charge_min.min(car.battery.charge(hybrid));
                        lap.charge_end = car.battery.charge(hybrid);
                    }
                    lap.straight_mode += car.active_aero.position * dt;
                }
                for event in race.events.drain(..) {
                    if let RaceEvent::LapCompleted { time, .. } = event {
                        done += 1;
                        lap.time = time;
                        println!(
                            "{:<12} {:<6} {:>4} {:>9.3} {:>7.1} {:>7.2} {:>7.2} {:>6.2} {:>6.2} {:>6.1}s",
                            track_name,
                            era,
                            done,
                            lap.time,
                            lap.top,
                            lap.deployed / 1e6,
                            lap.harvested / 1e6,
                            lap.charge_min,
                            lap.charge_end,
                            lap.straight_mode
                        );
                        lap = Lap {
                            charge_min: 1.0,
                            ..Lap::default()
                        };
                    }
                }
            }
            assert_eq!(
                done, laps,
                "{era} did not finish {laps} laps at {track_name}"
            );
        }
    }
}

/// A small field of one era: passes made, boost and overtake-mode use.
#[test]
#[ignore = "requires converted assets; prints racecraft per era"]
fn era_racecraft() {
    let Some(root) = root() else {
        eprintln!("skipping: assets missing");
        return;
    };
    let laps: u32 = std::env::var("ERA_LAPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    for (track_name, track) in tracks(&root) {
        for (era, spec) in eras(&root) {
            // Slowest first so the field has to pass.
            let entrants: Vec<_> = (0..6)
                .map(|i| entrant(&spec, 0.93 + 0.012 * i as f64, 0.3 + 0.1 * i as f64))
                .collect();
            let mut race = Race::new(track.clone(), entrants, laps, 11);
            let (mut passes, mut contacts) = (0, 0);
            let (mut boost, mut overtake) = (0.0, 0.0);
            let dt = 1.0 / PHYSICS_RATE;
            let limit = (laps as f64 * 300.0 * PHYSICS_RATE) as u64;
            while race.phase != Phase::Finished && race.tick < limit {
                race.step(&[]);
                for entrant in &race.entrants {
                    match entrant.car.battery.mode {
                        DeployMode::Boost => boost += dt,
                        DeployMode::Overtake => overtake += dt,
                        DeployMode::Normal => {}
                    }
                }
                for event in race.events.drain(..) {
                    match event {
                        RaceEvent::Overtake { .. } => passes += 1,
                        RaceEvent::Contact { impulse, .. } if impulse > 2000.0 => contacts += 1,
                        _ => {}
                    }
                }
            }
            let finished = race
                .entrants
                .iter()
                .filter(|e| e.timing.finish_time.is_some())
                .count();
            println!(
                "{track_name:<12} {era:<6} passes {passes:>3}  hard contacts {contacts:>3}  boost {boost:>6.1}s  overtake {overtake:>6.1}s  finished {finished}/6"
            );
        }
    }
}
