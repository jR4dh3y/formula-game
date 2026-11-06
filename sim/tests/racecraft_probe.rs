//! Wheel-to-wheel racecraft probe on the converted circuits: overtakes,
//! car-to-car contact, recoveries and time spent stuck nose-to-tail.
//!
//! Prints numbers for tuning the AI; it asserts nothing. A circuit with
//! passing lanes (Spa) is also raced without them, the generic path that
//! every other layout takes.
//!
//! `cargo test --release -p racecraft-sim --test racecraft_probe -- --ignored --nocapture`
//!
//! Environment: `PROBE_TRACKS=spa,bahrain` (default `spa`), `PROBE_SECONDS`
//! (150), `PROBE_CARS` (10), `PROBE_SEEDS=3,8`, `PROBE_GRID=game` for the
//! game's qualifying-order field instead of slowest-first, `PROBE_PACES=a,b`
//! for explicit paces, `PROBE_VERBOSE` (each contact and recovery),
//! `PROBE_LAPS` (laps and best laps), `PROBE_TRACE=seed:from:to:s_min:s_max`
//! (every car in a stretch) and `PROBE_DYN` (adds yaw and tyre sliding).
use racecraft_sim::{
    Assists, CarGeometry, CarSpec, DriverProfile, EntrantConfig, Phase, Race, RaceEvent, Setup,
    Track, PHYSICS_RATE,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

#[derive(Default, Debug, Clone, Copy)]
struct Tally {
    overtakes: usize,
    /// Overtakes where the passer is still ahead three seconds later.
    clean_passes: usize,
    /// Slower-ahead-of-faster pairs left at the end (the grid starts fully
    /// reversed: n(n-1)/2).
    inversions: usize,
    /// Metres the average car is behind the leader at the end: time lost
    /// fighting instead of racing.
    spread: f64,
    contacts: usize,
    heavy: usize,
    recoveries: usize,
    /// Car-seconds spent 8..30 m behind another car within 1 m laterally.
    glued: f64,
    /// Car-seconds spent wheel to wheel (overlapping, at least a car width apart).
    alongside: f64,
}

fn run(track: &Arc<Track>, spec: &Arc<CarSpec>, seed: u64, cars: usize, seconds: f64) -> Tally {
    // Slowest at the front, so the field has to race its way into order.
    let entrants = (0..cars)
        .map(|slot| EntrantConfig {
            name: format!("AI{slot}"),
            spec: spec.clone(),
            setup: Setup::default(),
            assists: Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            ai: Some(match std::env::var("PROBE_PACES") {
                // `PROBE_PACES=0.93,0.99` races these paces, front to back,
                // with the spa_lines duel's aggression and no mistakes.
                Ok(list) => DriverProfile {
                    pace: list.split(',').nth(slot).unwrap().parse().unwrap(),
                    aggression: 0.7,
                    consistency: 1.0,
                },
                // `PROBE_GRID=game`: the game's qualifying-order field.
                Err(_) if std::env::var("PROBE_GRID").is_ok_and(|grid| grid == "game") => {
                    DriverProfile {
                        pace: 0.985 - 0.006 * slot as f64,
                        aggression: 0.3 + 0.03 * (slot % 10) as f64,
                        consistency: 0.97,
                    }
                }
                Err(_) => DriverProfile {
                    pace: 0.92 + 0.007 * slot as f64,
                    aggression: 0.35 + 0.05 * (slot % 8) as f64,
                    consistency: 0.97,
                },
            }),
            human_slot: 0,
            livery: [1.; 3],
        })
        .collect();
    let mut race = Race::new(track.clone(), entrants, 50, seed);
    let mut tally = Tally::default();
    let mut last_contact: HashMap<(usize, usize), u64> = HashMap::new();
    let mut last: Vec<_> = race.entrants.iter().map(|e| e.car.position).collect();
    let dt = 1. / PHYSICS_RATE;
    let mut pending: Vec<(usize, usize, f64)> = Vec::new();
    while race.phase != Phase::Racing || race.time < seconds {
        race.step(&[]);
        for event in std::mem::take(&mut race.events) {
            match event {
                RaceEvent::Overtake { entrant, passed } if race.time > 10. => {
                    tally.overtakes += 1;
                    pending.push((entrant, passed, race.time + 3.));
                }
                RaceEvent::Contact {
                    a,
                    b: Some(b),
                    impulse,
                } => {
                    let previous = last_contact.insert((a, b), race.tick);
                    // One episode per pair until they are apart for half a second.
                    if previous.is_none_or(|tick| race.tick - tick > (PHYSICS_RATE / 2.) as u64) {
                        tally.contacts += 1;
                        if impulse > 8000. {
                            tally.heavy += 1;
                        }
                        if std::env::var_os("PROBE_VERBOSE").is_some() {
                            describe(&race, a, b, impulse);
                        }
                    }
                }
                _ => {}
            }
        }
        if race.phase != Phase::Racing {
            continue;
        }
        trace(&race, seed);
        pending.retain(|&(entrant, passed, due)| {
            if race.time < due {
                return true;
            }
            if race.position_of(entrant) < race.position_of(passed) {
                tally.clean_passes += 1;
            }
            false
        });
        for (index, entrant) in race.entrants.iter().enumerate() {
            if (entrant.car.position - last[index]).length() > 6. {
                tally.recoveries += 1;
                if std::env::var_os("PROBE_VERBOSE").is_some() {
                    let p = entrant.car.track_position;
                    println!(
                        "  recovery t={:.1} car {index} now at s={:.0} lat {:+.1}",
                        race.time, p.s, p.lateral
                    );
                }
            }
            last[index] = entrant.car.position;
            let me = entrant.car.track_position;
            for (other_index, other) in race.entrants.iter().enumerate() {
                if other_index == index {
                    continue;
                }
                let them = other.car.track_position;
                let gap = track.delta(me.s, them.s);
                let across = (me.lateral - them.lateral).abs();
                if (8.0..30.).contains(&gap) && across < 1. && entrant.car.speed() > 40. {
                    tally.glued += dt;
                }
                if other_index > index && gap.abs() < 4. && across > 1.9 {
                    tally.alongside += dt;
                }
            }
        }
    }
    if std::env::var_os("PROBE_LAPS").is_some() {
        let laps: Vec<String> = race
            .entrants
            .iter()
            .map(|e| {
                format!(
                    "{}:{:.1}/{:.0}m",
                    e.timing.laps_completed,
                    e.timing.best_lap.unwrap_or(0.),
                    e.timing.distance
                )
            })
            .collect();
        println!("  laps/best/distance by slot: {}", laps.join(" "));
    }
    let distances: Vec<f64> = race.entrants.iter().map(|e| e.timing.distance).collect();
    let lead = distances.iter().copied().fold(f64::MIN, f64::max);
    tally.spread = distances.iter().map(|d| lead - d).sum::<f64>() / cars as f64;
    // Higher grid slots are faster drivers.
    for slower in 0..cars {
        for faster in slower + 1..cars {
            if race.position_of(slower) < race.position_of(faster) {
                tally.inversions += 1;
            }
        }
    }
    tally
}

/// `PROBE_TRACE=seed:from:to:s_min:s_max` prints every car in that stretch.
fn trace(race: &Race, seed: u64) {
    let Ok(spec) = std::env::var("PROBE_TRACE") else {
        return;
    };
    let v: Vec<f64> = spec.split(':').map(|x| x.parse().unwrap()).collect();
    if v[0] as u64 != seed || race.time < v[1] || race.time > v[2] || !race.tick.is_multiple_of(18)
    {
        return;
    }
    let track = &race.track;
    let mut line = format!("t={:.2}", race.time);
    for (index, e) in race.entrants.iter().enumerate() {
        let p = e.car.track_position;
        if p.s < v[3] || p.s > v[4] {
            continue;
        }
        let intent = match &e.driver {
            racecraft_sim::Driver::Ai(ai) => match ai.intent {
                racecraft_sim::Intent::Line => "L".to_string(),
                racecraft_sim::Intent::Attack { side, .. } => format!("A{side:+.0}"),
                racecraft_sim::Intent::Defend { side, .. } => format!("D{side:+.0}"),
            },
            _ => "H".into(),
        };
        line += &format!(
            " | #{index} s{:.1} lat{:+.1}(ln{:+.1},hw{:.1}) v{:.0} {intent}",
            p.s,
            p.lateral,
            track.racing_line.offsets[track.station_at(p.s)],
            track.frame_at(p.s).half_width,
            e.car.speed() * 3.6
        );
        if std::env::var_os("PROBE_DYN").is_some() {
            let heading = e
                .car
                .forward()
                .angle_to(track.frame_at(p.s).tangent)
                .to_degrees();
            line += &format!(
                " yaw{:+.2} head{heading:+.0} slide{:.2}/{:.2}",
                e.car.yaw_rate,
                e.car.wheels[0].sliding.max(e.car.wheels[1].sliding),
                e.car.wheels[2].sliding.max(e.car.wheels[3].sliding)
            );
        }
    }
    println!("{line}");
}

fn describe(race: &Race, a: usize, b: usize, impulse: f64) {
    let track = &race.track;
    let intent = |index: usize| match &race.entrants[index].driver {
        racecraft_sim::Driver::Ai(ai) => format!("{:?}", ai.intent),
        _ => "human".into(),
    };
    let (ca, cb) = (&race.entrants[a].car, &race.entrants[b].car);
    let s = ca.track_position.s;
    let curvature = track.racing_line.curvature[track.station_at(s)];
    println!(
        "  contact t={:.1} s={s:.0} k={curvature:+.4} b ahead {:+.1} m, lat {:+.1}/{:+.1} line {:+.1} v {:.0}/{:.0} km/h imp {impulse:.0} {}/{}",
        race.time,
        track.delta(s, cb.track_position.s),
        ca.track_position.lateral,
        cb.track_position.lateral,
        track.racing_line.offsets[track.station_at(s)],
        ca.speed() * 3.6,
        cb.speed() * 3.6,
        intent(a),
        intent(b),
    );
}

#[test]
#[ignore]
fn racecraft_probe() {
    let root = std::env::var_os("F1_TEST_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets"));
    let Ok(geometry) = CarGeometry::load(root.join("cars/gen22/car.json")) else {
        eprintln!("skipping: generated Gen22 geometry absent");
        return;
    };
    let spec = Arc::new(CarSpec::gen22_with(&geometry));
    let names: Vec<String> = std::env::var("PROBE_TRACKS")
        .map(|list| list.split(',').map(str::to_owned).collect())
        .unwrap_or_else(|_| vec!["spa".to_owned()]);
    let seconds: f64 = std::env::var("PROBE_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(150.);
    let cars = std::env::var("PROBE_CARS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    let mut total = Tally::default();
    let mut races = 0;
    for name in names {
        let Ok(loaded) = Track::load(root.join(format!("tracks/{name}"))) else {
            eprintln!("skipping {name}: generated track absent");
            continue;
        };
        // The same circuit with its validated passing lanes, if any, and
        // without them: the generic path every other layout takes.
        let mut variants = Vec::new();
        if loaded.passing_lines.is_some() {
            let mut plain = Track::load(root.join(format!("tracks/{name}"))).unwrap();
            plain.passing_lines = None;
            variants.push((format!("{name}/lanes"), Arc::new(loaded)));
            variants.push((format!("{name}/plain"), Arc::new(plain)));
        } else {
            variants.push((format!("{name}/plain"), Arc::new(loaded)));
        }
        for (label, track) in variants {
            for seed in std::env::var("PROBE_SEEDS").map_or(vec![3, 8, 21, 34], |list| {
                list.split(',')
                    .map(|v| v.parse().unwrap())
                    .collect::<Vec<u64>>()
            }) {
                let tally = run(&track, &spec, seed, cars, seconds);
                println!("{label} seed {seed}: {tally:?}");
                total.overtakes += tally.overtakes;
                total.clean_passes += tally.clean_passes;
                total.inversions += tally.inversions;
                races += 1;
                total.spread += tally.spread;
                total.contacts += tally.contacts;
                total.heavy += tally.heavy;
                total.recoveries += tally.recoveries;
                total.glued += tally.glued;
                total.alongside += tally.alongside;
            }
        }
    }
    total.spread /= f64::from(races.max(1));
    println!("TOTAL (spread averaged per race) {total:?}");
}
