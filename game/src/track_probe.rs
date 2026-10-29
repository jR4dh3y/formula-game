//! Opt-in rendered map acceptance run; uses private converted assets.
//! Physics advances 72 real ticks per presentation frame. Captures hold the
//! simulation and cut the camera to the actual pose, allowing repeatable views
//! around a complete lap without injecting physical desktop input.
use super::*;
use racecraft_sim::{Assists, DriverProfile, EntrantConfig, Phase, Surface};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[test]
#[ignore = "requires Vulkan/XWayland and F1_TEST_ASSETS, F1_TEST_TRACK, F1_TEST_OUTPUT"]
fn rendered_track_lap() {
    let root = std::env::var("F1_TEST_ASSETS").unwrap();
    let track = std::env::var("F1_TEST_TRACK").unwrap();
    assert!(matches!(track.as_str(), "silverstone" | "bahrain" | "cota"));
    let options = Options::parse(
        [
            "--assets",
            &root,
            "--track",
            &track,
            "--opponents",
            "0",
            "--grid",
            "1",
            "--laps",
            "1",
            "--frames",
            "100000",
            "--mute",
            "--size",
            "1280x720",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap();
    let mut app = build_app(options).unwrap();
    {
        let mut session = app.world_mut().resource_mut::<Session>();
        let entrant = &session.race.entrants[0];
        session.race = Race::new(
            session.race.track.clone(),
            vec![EntrantConfig {
                name: "Map probe".into(),
                spec: entrant.car.spec.clone(),
                setup: entrant.car.setup,
                assists: Assists::default(),
                ai: Some(DriverProfile {
                    pace: 0.96,
                    aggression: 0.,
                    consistency: 1.,
                }),
                human_slot: 0,
                livery: [1.; 3],
            }],
            1,
            5,
        );
        session.poses = pose::Poses::new(&session.race);
        session.paused = true;
        session.show_racing_line = false;
    }
    let complete = Arc::new(AtomicUsize::new(0));
    let camera = match std::env::var("F1_TEST_CAMERA")
        .as_deref()
        .unwrap_or("chase")
    {
        "chase" => ProbeCamera::Driver(camera::Mode::Chase),
        "cockpit" => ProbeCamera::Driver(camera::Mode::Cockpit),
        "tv" => ProbeCamera::Driver(camera::Mode::Tv),
        "spectator" => ProbeCamera::Spectator(Vec3::new(-75., 45., 55.)),
        "spectator-reverse" => ProbeCamera::Spectator(Vec3::new(75., 55., -75.)),
        other => panic!("unknown F1_TEST_CAMERA: {other}"),
    };
    app.insert_resource(camera)
        .insert_resource(Probe {
            frame: 0,
            sample: 0,
            hold: 60,
            requested: 0,
            complete: complete.clone(),
            worst: 0.,
            off: 0,
            ticks: 0,
            recovered: false,
            contacts: 0,
            jumps: 0,
            max_height_step: 0.,
        })
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f64(1. / 60.),
        ))
        .add_systems(PreUpdate, advance.after(controls::update))
        .add_systems(Update, snap_camera.before(camera::update))
        .add_systems(Update, spectator_camera.after(camera::update));
    assert!(app.run().is_success());
    assert_eq!(complete.load(Ordering::SeqCst), 7);
}

#[derive(Resource)]
struct Probe {
    frame: usize,
    sample: usize,
    hold: usize,
    requested: usize,
    complete: Arc<AtomicUsize>,
    worst: f64,
    off: usize,
    ticks: usize,
    recovered: bool,
    contacts: usize,
    jumps: usize,
    max_height_step: f64,
}

fn advance(
    mut commands: Commands,
    mut session: ResMut<Session>,
    mut probe: ResMut<Probe>,
    mut exit: MessageWriter<AppExit>,
    decals: Query<(
        &bevy::gltf::GltfMaterialName,
        Option<&bevy::light::NotShadowCaster>,
    )>,
) {
    if !session.ready {
        return;
    }
    probe.frame += 1;
    if probe.frame == 1 {
        let mut count = 0;
        for (name, no_shadow) in &decals {
            if name.0.trim_start_matches("ground:").starts_with("decal:") {
                assert!(
                    no_shadow.is_some(),
                    "track decal must not cast shadows: {}",
                    name.0
                );
                count += 1;
            }
        }
        assert!(count > 0, "converted track has no marked decals");
    }
    assert!(probe.frame < 3000, "map probe stalled");
    session.paused = true;
    if probe.hold > 0 {
        probe.hold -= 1;
        if probe.hold == 1 {
            let name = if probe.recovered {
                "recovery".into()
            } else {
                format!("sample-{}", probe.sample)
            };
            let path =
                PathBuf::from(std::env::var("F1_TEST_OUTPUT").unwrap()).join(format!("{name}.png"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let complete = probe.complete.clone();
            probe.requested += 1;
            commands.spawn(Screenshot::primary_window()).observe(
                move |event: On<ScreenshotCaptured>| {
                    event
                        .image
                        .clone()
                        .try_into_dynamic()
                        .unwrap()
                        .to_rgb8()
                        .save(&path)
                        .unwrap();
                    complete.fetch_add(1, Ordering::SeqCst);
                },
            );
            let car = &session.race.entrants[0].car;
            info!(
                "MAP_SAMPLE {name} s={:.1} position={:?} speed={:.1}",
                car.track_position.s,
                car.position,
                car.speed()
            );
            probe.sample += 1;
        }
        return;
    }
    if probe.complete.load(Ordering::SeqCst) != probe.requested {
        return;
    }
    if probe.recovered {
        assert_eq!(probe.complete.load(Ordering::SeqCst), 7);
        exit.write(AppExit::Success);
        return;
    }
    let fractions = [0., 0.15, 0.30, 0.50, 0.70, 0.88];
    for _ in 0..72 {
        session.race.events.clear();
        let before = session.race.entrants[0].car.position;
        session.race.step(&[]);
        probe.contacts += session
            .race
            .events
            .iter()
            .filter(|e| matches!(e, racecraft_sim::RaceEvent::Contact { .. }))
            .count();
        let car = &session.race.entrants[0].car;
        if !session.race.events.is_empty()
            && session
                .race
                .events
                .iter()
                .any(|e| matches!(e, racecraft_sim::RaceEvent::Contact { .. }))
        {
            info!(
                "MAP_CONTACT s={:.1} position={:?}",
                car.track_position.s, car.position
            );
        }
        probe.jumps += usize::from(before.distance(car.position) > 2.);
        probe.max_height_step = probe.max_height_step.max((before.y - car.position.y).abs());
        let width = session.race.track.frame_at(car.track_position.s).half_width;
        probe.worst = probe.worst.max(car.track_position.lateral.abs() - width);
        probe.off += usize::from(!matches!(
            session
                .race
                .track
                .collision
                .ground(car.position)
                .map(|h| h.surface),
            Some(Surface::Asphalt | Surface::Kerb | Surface::Concrete)
        ));
        probe.ticks += 1;
    }
    if probe.sample < fractions.len()
        && session.race.entrants[0].timing.distance
            >= session.race.track.length * fractions[probe.sample]
    {
        probe.hold = 30;
    }
    if session.race.phase == Phase::Finished {
        assert_eq!(probe.sample, 6);
        let off = probe.off as f64 / probe.ticks as f64;
        info!(
            "MAP_LAP lap={:?} worst={:.3} off={:.5}",
            session.race.entrants[0].timing.best_lap, probe.worst, off
        );
        info!(
            "MAP_GEOMETRY contacts={} jumps={} max_height_step={:.5}",
            probe.contacts, probe.jumps, probe.max_height_step
        );
        assert_eq!(probe.jumps, 0, "lap must not rely on automatic recovery");
        assert!(probe.worst < 5. && off < 0.05);
        assert_eq!(probe.contacts, 0, "clean solo lap must not hit barriers");
        session.race.events.clear();
        let before = session.race.entrants[0].car.position;
        session.race.entrants[0].car.position += glam::DVec3::new(80., 20., 80.);
        session.race.recover(0);
        let car = &session.race.entrants[0].car;
        assert!(car.position.distance(before) < 10.);
        assert!(car.speed() < 0.1);
        assert!(matches!(
            session
                .race
                .track
                .collision
                .ground(car.position)
                .map(|h| h.surface),
            Some(Surface::Asphalt | Surface::Kerb | Surface::Concrete)
        ));
        probe.recovered = true;
        probe.hold = 30;
    }
}

/// Optional appearance views share exactly the same driven lap and capture cuts.
#[derive(Resource)]
enum ProbeCamera {
    Driver(camera::Mode),
    Spectator(Vec3),
}

fn snap_camera(mut session: ResMut<Session>, view: Res<ProbeCamera>) {
    session.camera = camera::ChaseCamera::new(&session.race, 0);
    if let ProbeCamera::Driver(mode) = *view {
        session.camera.select(mode);
    }
}

fn spectator_camera(
    session: Res<Session>,
    view: Res<ProbeCamera>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let ProbeCamera::Spectator(offset) = *view else {
        return;
    };
    let target = session.race.entrants[0].car.position.as_vec3() + Vec3::Y * 2.;
    for (mut transform, mut projection) in &mut cameras {
        *transform = Transform::from_translation(target + offset).looking_at(target, Vec3::Y);
        if let Projection::Perspective(lens) = &mut *projection {
            lens.fov = 58_f32.to_radians();
        }
    }
}
