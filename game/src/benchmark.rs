//! Repeatable two-AI moving-start workload. Timing and screenshot readback are
//! separate: the latter must not contaminate a renderer performance comparison.
use crate::{camera, options::Options, pose, Session};
use bevy::{
    diagnostic::FrameCount,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};
use racecraft_sim::{
    AiDriver, Assists, Car, Driver, DriverProfile, EntrantConfig, Phase, Race, RaceEvent,
    PHYSICS_RATE,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    time::{Duration, Instant},
};

const SECONDS: f64 = 20.;
const TICKS: usize = (SECONDS * PHYSICS_RATE) as usize;
const SAMPLES: usize = 6;
const WARMUP: Duration = Duration::from_secs(2);
const SETTLE_FRAMES: usize = 24;

struct Sample {
    tick: usize,
    race: Race,
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Warmup,
    Measure,
    Proof,
    Done,
}

#[derive(Resource)]
pub struct Run {
    initial: Race,
    samples: Vec<Sample>,
    expected_hash: u64,
    minimum_clearance: f64,
    stage: Stage,
    began: Option<Instant>,
    previous: Option<Instant>,
    frames: Vec<f64>,
    measured_seconds: f64,
    ticks: usize,
    warmup_frames: usize,
    sample: usize,
    settle: usize,
    requested: bool,
    completed: Vec<bool>,
    error: Option<String>,
    output: PathBuf,
    asset_fingerprints: BTreeMap<String, String>,
    proof_started: Option<Instant>,
}

impl Run {
    pub fn plan(options: &Options, source: &Race) -> Result<(Race, Self), String> {
        let output = options.benchmark.as_ref().unwrap();
        if output.join("benchmark.json").exists() {
            return Err("benchmark output already contains a report; use a fresh directory to preserve comparisons".into());
        }
        std::fs::create_dir_all(output)
            .map_err(|e| format!("benchmark output {}: {e}", output.display()))?;
        // Fail before opening a window if output is not writable.
        std::fs::write(
            output.join("in-progress.json"),
            b"{\"status\":\"planning\"}\n",
        )
        .map_err(|e| format!("benchmark output: {e}"))?;
        let initial = rolling_race(options, source)?;
        let sample_ticks = sample_ticks(options.benchmark_seed);
        let mut planned = initial.clone();
        let mut samples = Vec::with_capacity(SAMPLES);
        let mut minimum_clearance = f64::INFINITY;
        for tick in 1..=TICKS {
            let previous: Vec<_> = planned.entrants.iter().map(|e| e.car.position).collect();
            planned.events.clear();
            planned.step(&[]);
            minimum_clearance = minimum_clearance.min(check_clean(&planned, &previous)?);
            if sample_ticks.contains(&tick) {
                samples.push(Sample {
                    tick,
                    race: planned.clone(),
                });
            }
        }
        let expected_hash = state_hash(&planned);
        let run = Self {
            initial: initial.clone(),
            samples,
            expected_hash,
            minimum_clearance,
            stage: Stage::Warmup,
            began: None,
            previous: None,
            frames: Vec::new(),
            measured_seconds: 0.,
            ticks: 0,
            warmup_frames: 0,
            sample: 0,
            settle: 0,
            requested: false,
            completed: vec![false; SAMPLES],
            error: None,
            output: output.clone(),
            asset_fingerprints: asset_fingerprints(options)?,
            proof_started: None,
        };
        Ok((initial, run))
    }
}

fn rolling_race(options: &Options, source: &Race) -> Result<Race, String> {
    let template = &source.entrants[0].car;
    let profile = DriverProfile {
        pace: 0.94,
        aggression: 0.,
        consistency: 1.,
    };
    let configs = (0..2)
        .map(|i| EntrantConfig {
            name: if i == 0 { "Benchmark" } else { "Reference" }.into(),
            spec: template.spec.clone(),
            setup: template.setup,
            assists: Assists::default(),
            ai: Some(profile),
            human_slot: 0,
            livery: [1.; 3],
        })
        .collect();
    let mut race = Race::new(source.track.clone(), configs, 100, options.benchmark_seed);
    race.phase = Phase::Racing;
    race.time = 0.;
    for (i, entrant) in race.entrants.iter_mut().enumerate() {
        let s = (options.benchmark_start + i as f64 * 80.).rem_euclid(race.track.length);
        let frame = race.track.frame_at(s);
        let station = race.track.station_at(s);
        let offset = race.track.racing_line.offsets[station];
        let position = frame.point_at(offset);
        let yaw = frame.tangent.x.atan2(frame.tangent.y);
        let mut car = Car::new(
            template.spec.clone(),
            template.setup,
            Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            &race.track,
            position,
            yaw,
        );
        car.velocity = frame.tangent * 25.;
        car.powertrain.gear = 2;
        for tyre in &mut car.tyres {
            tyre.temperature = car.setup.compound.properties().optimal_temperature;
        }
        entrant.driver = Driver::Ai(Box::new(AiDriver::new(
            profile,
            &car,
            &race.track,
            options.benchmark_seed.wrapping_add(i as u64),
        )));
        entrant.car = car;
        entrant.timing.distance = s;
    }
    // Settle suspension, tyres, gearbox and AI before the measured moving start.
    for _ in 0..(2. * PHYSICS_RATE) as usize {
        let previous: Vec<_> = race.entrants.iter().map(|e| e.car.position).collect();
        race.events.clear();
        race.step(&[]);
        check_clean(&race, &previous)?;
    }
    if race.entrants.iter().any(|e| e.car.speed() < 8.) {
        return Err("benchmark moving start failed: a computer car stopped".into());
    }
    race.events.clear();
    Ok(race)
}

fn check_clean(race: &Race, previous: &[glam::DVec3]) -> Result<f64, String> {
    let clearance = race.entrants[0]
        .car
        .position
        .distance(race.entrants[1].car.position);
    let invalid = clearance < 12.
        || race
            .events
            .iter()
            .any(|e| matches!(e, RaceEvent::Contact { .. }))
        || race
            .entrants
            .iter()
            .zip(previous)
            .any(|(e, p)| e.car.position.distance(*p) > 3. || !e.car.position.is_finite());
    if invalid {
        return Err(format!("benchmark scenario has contact/recovery at {:.3}s (clearance {clearance:.2}m); choose another --benchmark-start", race.time));
    }
    Ok(clearance)
}

/// One jittered timestamp in each of six equal strata; reproducible by seed.
fn sample_ticks(seed: u64) -> Vec<usize> {
    let mut rng = racecraft_sim::rng::Rng::new(seed ^ 0x53414D504C45);
    (0..SAMPLES)
        .map(|i| {
            ((0.5 + (i as f64 + rng.next_f64()) * 19. / SAMPLES as f64) * PHYSICS_RATE).round()
                as usize
        })
        .collect()
}

fn asset_fingerprints(options: &Options) -> Result<BTreeMap<String, String>, String> {
    let mut results = BTreeMap::new();
    for name in [
        format!("tracks/{}/visual.glb", options.track),
        format!("tracks/{}/track.json", options.track),
        format!("tracks/{}/collision.bin", options.track),
        format!("cars/{}/car.glb", options.car),
        format!("cars/{}/car.json", options.car),
    ] {
        let mut file = std::fs::File::open(options.assets.join(&name))
            .map_err(|e| format!("benchmark fingerprint {name}: {e}"))?;
        let mut buffer = [0u8; 65536];
        let mut hash = 0xcbf29ce484222325_u64;
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            for byte in &buffer[..n] {
                hash = (hash ^ *byte as u64).wrapping_mul(0x100000001b3);
            }
        }
        results.insert(name, format!("{hash:016x}"));
    }
    Ok(results)
}

fn state_hash(race: &Race) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for entrant in &race.entrants {
        let c = &entrant.car;
        for value in [
            c.position.x,
            c.position.y,
            c.position.z,
            c.yaw,
            c.velocity.x,
            c.velocity.y,
            c.powertrain.rpm,
            entrant.timing.distance,
        ] {
            for byte in value.to_bits().to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
    }
    hash
}

fn reset_view(session: &mut Session, options: &Options) {
    session.poses = pose::Poses::new(&session.race);
    session.camera = camera::ChaseCamera::new(&session.race, 0);
    session.camera.select(options.camera);
    session.show_racing_line = false;
    session.show_performance = false;
    session.messages = Default::default();
    session.delta = Default::default();
}

#[allow(clippy::too_many_arguments)]
pub fn advance(
    mut commands: Commands,
    run: Option<ResMut<Run>>,
    mut session: ResMut<Session>,
    options: Res<Options>,
    mut frame: ResMut<FrameCount>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut run) = run else {
        return;
    };
    if !session.ready || run.stage == Stage::Done {
        return;
    }
    if let Some(error) = run.error.take() {
        error!("Benchmark: {error}");
        exit.write(AppExit::error());
        run.stage = Stage::Done;
        return;
    }
    let now = Instant::now();
    match run.stage {
        Stage::Warmup => {
            session.paused = true;
            let began = *run.began.get_or_insert(now);
            run.warmup_frames += 1;
            if now.duration_since(began) < WARMUP || run.warmup_frames < 64 {
                return;
            }
            session.race = run.initial.clone();
            session.paused = false;
            reset_view(&mut session, &options);
            frame.0 = 0;
            run.stage = Stage::Measure;
            run.began = Some(now);
            run.previous = Some(now);
            info!("Benchmark: starting 20-second two-AI moving run; seed={}, minimum planned clearance {:.1}m", options.benchmark_seed, run.minimum_clearance);
        }
        Stage::Measure => {
            let elapsed = now.duration_since(run.began.unwrap()).as_secs_f64();
            let previous = run.previous.replace(now).unwrap();
            run.frames.push(now.duration_since(previous).as_secs_f64());
            let target = ((elapsed.min(SECONDS) * PHYSICS_RATE).floor() as usize).min(TICKS);
            while run.ticks < target {
                let Session { poses, race, .. } = &mut *session;
                poses.record(race);
                race.events.clear();
                race.step(&[]);
                run.ticks += 1;
            }
            if elapsed >= SECONDS {
                run.measured_seconds = elapsed;
                if state_hash(&session.race) != run.expected_hash {
                    run.error = Some("measured drive diverged from preflight".into());
                    return;
                }
                info!("Benchmark: {:.1} fps over {} frames in {:.3}s; capturing six deterministic samples", run.frames.len() as f64 / elapsed, run.frames.len(), elapsed);
                session.paused = true;
                run.stage = Stage::Proof;
                run.proof_started = Some(now);
                run.settle = 0;
            }
        }
        Stage::Proof => {
            session.paused = true;
            if run
                .proof_started
                .is_some_and(|start| now.duration_since(start).as_secs() > 90)
            {
                run.error = Some("screenshot finalization timed out".into());
                return;
            }
            if run.sample == SAMPLES {
                match write_results(&run, &options) {
                    Ok(()) => {
                        info!(
                            "Benchmark report: {}",
                            run.output.join("benchmark.json").display()
                        );
                        exit.write(AppExit::Success);
                    }
                    Err(e) => {
                        error!("benchmark report: {e}");
                        exit.write(AppExit::error());
                    }
                }
                run.stage = Stage::Done;
                return;
            }
            if run.completed[run.sample] {
                run.sample += 1;
                run.settle = 0;
                run.requested = false;
                return;
            }
            if run.settle == 0 {
                session.race = run.samples[run.sample].race.clone();
                reset_view(&mut session, &options);
                frame.0 = (run.samples[run.sample].tick / 6) as u32;
            }
            run.settle += 1;
            if run.settle < SETTLE_FRAMES || run.requested {
                return;
            }
            run.requested = true;
            let index = run.sample;
            let path = run.output.join(format!(
                "sample-{index}-tick-{}.png",
                run.samples[index].tick
            ));
            commands.spawn(Screenshot::primary_window()).observe(
                move |event: On<ScreenshotCaptured>, mut run: ResMut<Run>| {
                    let result = event
                        .image
                        .clone()
                        .try_into_dynamic()
                        .map_err(|e| e.to_string())
                        .and_then(|image| image.to_rgb8().save(&path).map_err(|e| e.to_string()));
                    match result {
                        Ok(()) => run.completed[index] = true,
                        Err(e) => run.error = Some(format!("screenshot {}: {e}", path.display())),
                    }
                },
            );
        }
        Stage::Done => {}
    }
}

#[derive(Serialize)]
struct Metrics {
    frames: usize,
    wall_seconds: f64,
    average_fps: f64,
    mean_ms: f64,
    median_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    one_percent_low_fps: f64,
    worst_ms: f64,
}

fn metrics(frames: &[f64], seconds: f64) -> Metrics {
    let mut sorted = frames.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        sorted[((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1)].max(1e-9) * 1000.
    };
    let count = (sorted.len() as f64 * 0.01).ceil().max(1.) as usize;
    let slow = sorted[sorted.len() - count..].iter().sum::<f64>() / count as f64;
    Metrics {
        frames: frames.len(),
        wall_seconds: seconds,
        average_fps: frames.len() as f64 / seconds,
        mean_ms: seconds * 1000. / frames.len() as f64,
        median_ms: percentile(0.5),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
        one_percent_low_fps: 1. / slow,
        worst_ms: *sorted.last().unwrap() * 1000.,
    }
}

fn write_results(run: &Run, options: &Options) -> Result<(), String> {
    let initial: Vec<_> = run.initial.entrants.iter().map(|e| serde_json::json!({"position": e.car.position.to_array(), "speed_kmh": e.car.speed() * 3.6, "gear": e.car.powertrain.gear})).collect();
    let samples: Vec<_> = run.samples.iter().enumerate().map(|(i, s)| serde_json::json!({"tick": s.tick, "seconds": s.tick as f64 / PHYSICS_RATE, "state_fnv64": format!("{:016x}", state_hash(&s.race)), "file": format!("sample-{i}-tick-{}.png", s.tick)})).collect();
    let report = serde_json::json!({"schema": 1, "workload": "two-ai-moving-start-v1", "duration_seconds": SECONDS, "physics_hz": PHYSICS_RATE, "physics_ticks": TICKS, "seed": options.benchmark_seed, "start_metres": options.benchmark_start, "track": options.track, "car": options.car, "camera": options.camera, "graphics": options.graphics, "resolution": [options.width, options.height], "vsync": false, "audio_muted": options.mute, "ai_cars": 2, "human_cars": 0, "contacts": 0, "minimum_clearance_metres": run.minimum_clearance, "preflight_matches_measured": true, "final_state_fnv64": format!("{:016x}", run.expected_hash), "moving_start": initial, "asset_fnv64": run.asset_fingerprints, "metrics": metrics(&run.frames, run.measured_seconds), "samples": samples, "capture_policy": "Seeded stratified random ticks; exact preflight states rendered after timing, with 24 settling frames per sample. Loading, warmup and PNG readback/write are excluded from timing."});
    let json = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(run.output.join("benchmark.json"), json).map_err(|e| e.to_string())?;
    let mut csv = String::from("frame,seconds,milliseconds\n");
    let mut elapsed = 0.;
    for (i, dt) in run.frames.iter().enumerate() {
        elapsed += dt;
        csv.push_str(&format!("{i},{elapsed:.9},{:.6}\n", dt * 1000.));
    }
    std::fs::write(run.output.join("frames.csv"), csv).map_err(|e| e.to_string())?;
    std::fs::write(
        run.output.join("in-progress.json"),
        b"{\"status\":\"complete\"}\n",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires all converted F1_TEST_ASSETS; validates the full 20-second two-AI workload on every track/car"]
    fn all_converted_tracks_have_clean_repeatable_moving_workloads() {
        let root = std::env::var("F1_TEST_ASSETS").expect("F1_TEST_ASSETS");
        let evidence = std::env::var("F1_BENCHMARK_PLAN_OUTPUT")
            .expect("F1_BENCHMARK_PLAN_OUTPUT on disk, not tmpfs");
        for track in ["spa", "silverstone", "bahrain", "cota"] {
            for car in ["rb6", "rb22"] {
                let output = format!("{evidence}/{track}-{car}");
                let options = Options::parse(
                    [
                        "--assets",
                        &root,
                        "--track",
                        track,
                        "--car",
                        car,
                        "--benchmark",
                        &output,
                    ]
                    .into_iter()
                    .map(str::to_owned),
                )
                .unwrap();
                let source = crate::assets::load_race(&options).unwrap();
                let (initial, run) =
                    Run::plan(&options, &source).unwrap_or_else(|e| panic!("{track} {car}: {e}"));
                assert_eq!(initial.entrants.len(), 2);
                assert!(initial
                    .entrants
                    .iter()
                    .all(|e| matches!(e.driver, Driver::Ai(_)) && e.car.speed() > 8.));
                assert_eq!(run.samples.len(), 6);
                let mut repeat = initial.clone();
                for _ in 0..TICKS {
                    repeat.step(&[]);
                }
                assert_eq!(state_hash(&repeat), run.expected_hash);
                assert!(run.minimum_clearance >= 12.);
                eprintln!("BENCHMARK_PLAN {track} {car}: moving={:.1}/{:.1}kmh clearance={:.1}m final={:016x}", initial.entrants[0].car.speed()*3.6, initial.entrants[1].car.speed()*3.6, run.minimum_clearance, run.expected_hash);
            }
        }
    }

    #[test]
    fn random_samples_repeat_stay_stratified_and_cover_the_run() {
        let a = sample_ticks(24301);
        assert_eq!(a, sample_ticks(24301));
        assert_ne!(a, sample_ticks(24302));
        assert_eq!(a.len(), 6);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
        assert!(a[0] > 0 && a[5] < TICKS);
        for (i, tick) in a.iter().enumerate() {
            let t = *tick as f64 / PHYSICS_RATE;
            assert!(
                t >= 0.5 + i as f64 * 19. / 6. - 1. / PHYSICS_RATE
                    && t <= 0.5 + (i + 1) as f64 * 19. / 6. + 1. / PHYSICS_RATE
            );
        }
    }
    #[test]
    fn metrics_preserve_stalls_in_tail_and_low_fps() {
        let values = [vec![0.01; 99], vec![0.1]].concat();
        let m = metrics(&values, 1.09);
        assert!((m.average_fps - 100. / 1.09).abs() < 1e-9);
        assert_eq!(m.median_ms, 10.);
        assert_eq!(m.p99_ms, 10.);
        assert_eq!(m.worst_ms, 100.);
        assert_eq!(m.one_percent_low_fps, 10.);
    }
}
