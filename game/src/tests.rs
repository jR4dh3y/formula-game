use super::*;
use bevy::time::TimeUpdateStrategy;
use glam::{DVec2, DVec3};
use racecraft_sim::{
    car::spec::WheelSpec,
    track::{collision::CollisionWorld, racing_line::RacingLine, Station},
    Assists, CarSpec, EntrantConfig, Setup, Track,
};
use std::{sync::Arc, time::Duration};

fn session() -> Session {
    let stations: Vec<_> = (0..128)
        .map(|i| {
            let angle = i as f64 / 128.0 * std::f64::consts::TAU;
            Station {
                s: i as f64 * 2.0,
                position: DVec3::new(angle.cos() * 40.0, 0.0, angle.sin() * 40.0),
                tangent: DVec2::new(-angle.sin(), angle.cos()),
                half_width: 6.0,
            }
        })
        .collect();
    let track = Arc::new(Track {
        name: "Test loop".into(),
        source: String::new(),
        length: 256.0,
        racing_line: RacingLine::compute(&stations),
        stations,
        corners: vec![],
        collision: CollisionWorld::from_bytes(b"F1CL\x01\0\0\0\0\0\0\0\0\0\0\0").unwrap(),
        sun_direction: DVec3::new(-0.3, -0.8, -0.5),
    });
    let wheels = std::array::from_fn(|i| WheelSpec {
        position: DVec3::new(
            if i % 2 == 0 { 0.8 } else { -0.8 },
            0.35,
            if i < 2 { 1.7 } else { -1.7 },
        ),
        radius: 0.35,
        width: 0.4,
    });
    let race = Race::new(
        track,
        vec![EntrantConfig {
            name: "You".into(),
            spec: Arc::new(CarSpec::rb22(wheels, 5.4, 1.9)),
            setup: Setup::default(),
            assists: Assists::default(),
            ai: None,
            human_slot: 0,
            livery: [1.0; 3],
        }],
        1,
        5,
    );
    Session {
        camera: camera::ChaseCamera::new(&race, 0),
        minimap: hud::Minimap::new(&race.track),
        race,
        player: 0,
        controls: default(),
        paused: false,
        messages: default(),
        ready: false,
    }
}

#[test]
fn bevy_fixed_schedule_waits_for_assets_and_pauses_and_resumes() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(Time::<Fixed>::from_hz(PHYSICS_RATE))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .insert_resource(session())
        .init_resource::<ButtonInput<KeyCode>>()
        .add_message::<AppExit>()
        .add_systems(PreUpdate, controls::update)
        .add_systems(FixedUpdate, step_race);
    let initial_time = app.world().resource::<Session>().race.time;
    app.update();
    app.update();
    assert_eq!(app.world().resource::<Session>().race.time, initial_time);
    app.world_mut().resource_mut::<Session>().ready = true;
    for _ in 0..60 {
        app.update();
    }
    let before_pause = app.world().resource::<Session>().race.time;
    assert!(
        (before_pause - initial_time - 1.0).abs() < 0.004,
        "unexpected fixed clock {before_pause}"
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyP);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(app.world().resource::<Session>().race.time, before_pause);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyP);
    app.update();
    assert!(app.world().resource::<Session>().race.time > before_pause);
}

#[test]
fn bevy_transforms_preserve_wheel_pivots_and_camera_cycle() {
    let mut session = session();
    let car = &mut session.race.entrants[0].car;
    car.yaw = 0.6;
    car.pitch = 0.1;
    car.steer_angle = 0.2;
    car.wheels[0].rotation = 1.4;
    let mut app = App::new();
    app.insert_resource(session)
        .add_systems(Update, assets::sync_cars);
    let body = app
        .world_mut()
        .spawn((assets::CarVisual(0), Transform::default()))
        .id();
    let rest = Transform::from_xyz(0.8, 0.35, 1.7);
    let wheel = app
        .world_mut()
        .spawn((
            assets::WheelVisual {
                entrant: 0,
                wheel: 0,
                rest,
            },
            rest,
        ))
        .id();
    app.update();
    let transform = app.world().get::<Transform>(wheel).unwrap();
    assert_eq!(transform.translation, rest.translation);
    assert!(transform.rotation.abs_diff_eq(
        Quat::from_rotation_y(0.2) * Quat::from_rotation_x(1.4),
        1e-5
    ));
    assert!(app
        .world()
        .get::<Transform>(body)
        .unwrap()
        .rotation
        .abs_diff_eq(Quat::from_euler(EulerRot::YXZ, 0.6, -0.1, 0.0), 1e-5));
    let mut session = app.world_mut().resource_mut::<Session>();
    let mut pose = Transform::default();
    let mut lens = Projection::Perspective(default());
    for expected in [
        camera::Mode::Far,
        camera::Mode::TCam,
        camera::Mode::Nose,
        camera::Mode::Chase,
    ] {
        session.camera.cycle();
        assert_eq!(session.camera.mode, expected);
        let Session {
            race,
            camera,
            player,
            ..
        } = &mut *session;
        camera.update(&mut pose, &mut lens, race, *player, 1.0 / 60.0);
        assert!(pose.translation.is_finite());
        assert!(pose.rotation.is_finite());
    }
}

/// Runs the actual renderer with external assets and injected Bevy keyboard state.
#[test]
#[ignore = "requires a Vulkan display and F1_TEST_ASSETS"]
fn rendered_game_flows() {
    let root = std::env::var("F1_TEST_ASSETS").expect("set F1_TEST_ASSETS to generated assets");
    let options = Options::parse(
        ["--assets", &root, "--size", "1280x720", "--auto-gears"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();
    let mut app = build_app(options).unwrap();
    let probe = RenderProbe::default();
    let completed = probe.completed.clone();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )))
    .insert_resource(probe)
    .add_systems(
        PreUpdate,
        probe_input
            .after(bevy::input::InputSystems)
            .before(controls::update),
    )
    .add_systems(Update, probe_assertions.after(camera::update));
    assert!(app.run().is_success());
    assert!(
        completed.load(std::sync::atomic::Ordering::SeqCst),
        "renderer exited before completing the probe"
    );
}

#[derive(Resource, Default)]
struct RenderProbe {
    frame: u32,
    paused_time: f64,
    captures: usize,
    completed: Arc<std::sync::atomic::AtomicBool>,
}

fn probe_input(
    session: Res<Session>,
    mut probe: ResMut<RenderProbe>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
) {
    if !session.ready {
        return;
    }
    probe.frame += 1;
    keys.reset_all();
    keys.press(KeyCode::KeyW);
    if probe.completed.load(std::sync::atomic::Ordering::SeqCst) {
        keys.press(KeyCode::Escape);
    }
    match probe.frame {
        60 | 90 | 120 | 150 => keys.press(KeyCode::KeyC),
        180 | 210 => keys.press(KeyCode::KeyP),
        730 => keys.press(KeyCode::KeyR),
        _ => {}
    }
}

fn probe_assertions(
    mut commands: Commands,
    mut session: ResMut<Session>,
    mut probe: ResMut<RenderProbe>,
    wheels: Query<&assets::WheelVisual>,
    bodies: Query<&assets::CarVisual>,
    meshes: Query<&Mesh3d>,
) {
    let frame = probe.frame;
    match frame {
        30 => {
            assert_eq!(wheels.iter().count(), session.race.entrants.len() * 4);
            assert_eq!(bodies.iter().count(), session.race.entrants.len());
            assert!(meshes.iter().count() > 40);
        }
        60 => assert_eq!(session.camera.mode, camera::Mode::Far),
        90 => assert_eq!(session.camera.mode, camera::Mode::TCam),
        120 => assert_eq!(session.camera.mode, camera::Mode::Nose),
        150 => assert_eq!(session.camera.mode, camera::Mode::Chase),
        180 => {
            assert!(session.paused);
            probe.paused_time = session.race.time;
        }
        200 => assert_eq!(session.race.time, probe.paused_time),
        210 => {
            assert!(!session.paused);
            assert!(session.race.time > probe.paused_time);
        }
        730 => assert!(session.race.entrants[session.player].car.speed() < 0.5),
        720 => {
            assert!(session.race.entrants[session.player].car.speed() > 5.0);
            assert_eq!(session.race.phase, racecraft_sim::Phase::Racing);
            info!("Render probe: driving, cameras and pause/resume passed");
        }
        // Force the presentation state to exercise the results HUD separately from the sim's race-completion tests.
        740 => session.race.phase = racecraft_sim::Phase::Finished,
        780..=899 if probe.captures == 4 => {
            probe
                .completed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            info!("Render probe: recovery and all four screenshots passed");
        }
        900 => panic!("screenshots did not complete"),
        _ => {}
    }
    let name = match frame {
        30 => "grid",
        200 => "paused",
        720 => "racing",
        760 => "results",
        _ => return,
    };
    let directory = std::env::var_os("F1_TEST_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("f1-bevy-proof"));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{name}.png"));
    commands.spawn(Screenshot::primary_window()).observe(
        move |event: On<ScreenshotCaptured>, mut probe: ResMut<RenderProbe>| {
            event
                .image
                .clone()
                .try_into_dynamic()
                .unwrap()
                .to_rgb8()
                .save(&path)
                .unwrap();
            probe.captures += 1;
            info!("Render probe screenshot: {}", path.display());
        },
    );
}
