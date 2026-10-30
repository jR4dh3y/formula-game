use super::*;
use bevy::time::TimeUpdateStrategy;
use glam::{DVec2, DVec3};
use racecraft_sim::{
    car::spec::WheelSpec,
    track::{collision::CollisionWorld, racing_line::RacingLine, Station},
    Assists, CarSpec, EntrantConfig, Setup, Track,
};
use std::{sync::Arc, time::Duration};

pub(super) fn session() -> Session {
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
            spec: Arc::new(CarSpec::gen22(wheels, 5.4, 1.9)),
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
        identities: vec![None],
        rig: default(),
        drs_open: false,
        guidance: driving::Guidance::new(&race, 0),
        corner_braking: 0.,
        show_racing_line: false,
        show_performance: false,
        camera: camera::ChaseCamera::new(&race, 0),
        minimap: hud::Minimap::new(&race.track),
        poses: pose::Poses::new(&race),
        race,
        player: 0,
        controls: default(),
        paused: false,
        messages: default(),
        delta: default(),
        ready: false,
        screen: menu::Screen::Racing,
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
fn cockpit_steering_tracks_both_directions_and_returns_to_rest() {
    let mut app = App::new();
    app.insert_resource(session())
        .init_resource::<Time<Fixed>>()
        .add_systems(Update, assets::sync_cars);
    let pivot = Vec3::new(0.0, 0.6, 0.7);
    let rest = Transform::default();
    let wheel = app
        .world_mut()
        .spawn((
            assets::SteeringWheelVisual {
                entrant: 0,
                rest,
                pivot,
            },
            rest,
        ))
        .id();
    for angle in [-0.3, 0.3, 0.0] {
        let mut session = app.world_mut().resource_mut::<Session>();
        session.race.entrants[0].car.steer_angle = angle;
        let Session { poses, race, .. } = &mut *session;
        poses.record(race);
        app.update();
        let transform = app.world().get::<Transform>(wheel).unwrap();
        assert!(transform
            .rotation
            .abs_diff_eq(Quat::from_rotation_z(-angle as f32 * 6.0), 1e-5));
        assert!(transform.transform_point(pivot).abs_diff_eq(pivot, 1e-5));
        if angle == 0.0 {
            assert_eq!(*transform, rest);
        }
    }
}

#[test]
fn gen22_rig_uses_local_axes_and_keeps_uprights_free_of_wheel_spin() {
    let mut app = App::new();
    let mut state = session();
    state.rig.drs.open_angle = -0.25;
    state.drs_open = true;
    state.race.entrants[0].car.steer_angle = 0.2;
    state.race.entrants[0].car.wheels[0].rotation = 1.7;
    state.race.entrants[0].car.wheels[0].ground_height = 0.1;
    state.race.entrants[0].car.wheels[1].ground_height = 0.35;
    state.poses.record(&state.race);
    app.insert_resource(state)
        .init_resource::<Time<Fixed>>()
        .add_systems(Update, assets::sync_cars);
    let rest = Transform::from_xyz(0., 0.6, 0.7).with_rotation(Quat::from_rotation_x(0.15));
    let entities: Vec<_> = [
        assets::RigKind::Steering,
        assets::RigKind::Upright,
        assets::RigKind::Drs,
        assets::RigKind::Upright,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| {
        let rest = if index == 1 {
            rest.with_translation(Vec3::new(0.8, 0.6, 0.7))
        } else if index == 3 {
            rest.with_translation(Vec3::new(-0.8, 0.6, 0.7))
        } else {
            rest
        };
        app.world_mut()
            .spawn((
                assets::RigPart {
                    entrant: 0,
                    rest,
                    kind,
                },
                rest,
            ))
            .id()
    })
    .collect();
    let body = app
        .world_mut()
        .spawn((assets::CarVisual(0), Transform::default()))
        .id();
    app.update();
    let chassis = app.world().get::<Transform>(body).unwrap();
    for (entity, rotation) in entities.iter().zip([
        Quat::from_rotation_z(-1.2),
        Quat::from_rotation_y(0.2),
        Quat::from_rotation_x(-0.25),
        Quat::from_rotation_y(0.2),
    ]) {
        let transform = app.world().get::<Transform>(*entity).unwrap();
        if *entity != entities[1] && *entity != entities[3] {
            assert_eq!(transform.translation, rest.translation);
        } else {
            let wheel_index = if *entity == entities[1] { 0 } else { 1 };
            let state = app.world().resource::<Session>();
            let car = &state.race.entrants[0].car;
            let wheel = &car.spec.wheels[wheel_index];
            let mut lowest = f32::INFINITY;
            for side in [-0.5, 0.5] {
                for step in 0..720 {
                    let angle = step as f32 * std::f32::consts::TAU / 720.;
                    let vertex = Vec3::new(
                        side * wheel.width as f32,
                        angle.cos() * wheel.radius as f32,
                        angle.sin() * wheel.radius as f32,
                    );
                    lowest =
                        lowest.min(chassis.transform_point(transform.transform_point(vertex)).y);
                }
            }
            assert!((lowest - car.wheels[wheel_index].ground_height as f32).abs() < 1e-4);
        }
        assert!(transform
            .rotation
            .abs_diff_eq(rest.rotation * rotation, 1e-5));
    }
    app.world_mut().resource_mut::<Session>().drs_open = false;
    app.update();
    assert_eq!(*app.world().get::<Transform>(entities[2]).unwrap(), rest);
}

#[test]
fn bevy_transforms_preserve_wheel_pivots_and_camera_cycle() {
    let mut session = session();
    let car = &mut session.race.entrants[0].car;
    car.yaw = 0.6;
    car.pitch = 0.1;
    car.steer_angle = 0.2;
    car.wheels[0].rotation = 1.4;
    let Session { poses, race, .. } = &mut session;
    poses.record(race);
    let mut app = App::new();
    app.insert_resource(session)
        .init_resource::<Time<Fixed>>()
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
    let chassis = app.world().get::<Transform>(body).unwrap();
    let session = app.world().resource::<Session>();
    let car = &session.race.entrants[0].car;
    let axle_y = (chassis.rotation * transform.rotation * Vec3::X).y.abs();
    let tyre = &car.spec.wheels[0];
    let bottom = chassis.transform_point(transform.translation).y
        - tyre.radius as f32 * (1. - axle_y * axle_y).sqrt()
        - tyre.width as f32 * 0.5 * axle_y;
    assert!((bottom - car.wheels[0].ground_height as f32).abs() < 1e-4);
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
    for expected in [camera::Mode::Cockpit, camera::Mode::Tv, camera::Mode::Chase] {
        session.camera.cycle();
        assert_eq!(session.camera.mode, expected);
        let Session {
            race,
            camera,
            player,
            ..
        } = &mut *session;
        let car = pose::Pose::of(&race.entrants[*player].car);
        camera.update(&mut pose, &mut lens, race, *player, &car, 1.0 / 60.0);
        assert!(pose.translation.is_finite());
        assert!(pose.rotation.is_finite());
    }
}

#[test]
fn drs_flap_control_opens_in_racing_and_braking_closes_it() {
    let mut state = session();
    state.rig.drs.open_angle = -0.25;
    state.race.phase = racecraft_sim::Phase::Racing;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(state)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_systems(Update, controls::update);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    assert!(app.world().resource::<Session>().drs_open);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyS);
    app.update();
    assert!(!app.world().resource::<Session>().drs_open);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut().resource_mut::<Session>().drs_open = true;
    let mut gamepad = Gamepad::default();
    gamepad.analog_mut().set(GamepadButton::LeftTrigger2, 0.3);
    let pad = app.world_mut().spawn(gamepad).id();
    app.update();
    assert!(!app.world().resource::<Session>().drs_open);
    app.world_mut().despawn(pad);
    app.world_mut().resource_mut::<Session>().screen = menu::Screen::Paused;
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyF);
    app.update();
    assert!(!app.world().resource::<Session>().drs_open);
}

#[test]
fn camera_shortcuts_pause_menu_and_menu_input_isolation() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(session())
        .init_resource::<ButtonInput<KeyCode>>()
        .add_message::<AppExit>()
        .add_systems(PreUpdate, controls::update);
    for (key, expected) in [
        (KeyCode::Digit1, camera::Mode::Cockpit),
        (KeyCode::Digit2, camera::Mode::Tv),
        (KeyCode::Digit3, camera::Mode::Chase),
        (KeyCode::KeyC, camera::Mode::Cockpit),
    ] {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        assert_eq!(app.world().resource::<Session>().camera.mode, expected);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert_eq!(
        app.world().resource::<Session>().screen,
        menu::Screen::Paused
    );
    assert!(app.world().resource::<Session>().paused);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.update();
    assert_eq!(app.world().resource::<Session>().controls.throttle(), 0.);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert_eq!(
        app.world().resource::<Session>().screen,
        menu::Screen::Racing
    );
    assert!(!app.world().resource::<Session>().paused);
    app.world_mut().resource_mut::<Session>().screen = menu::Screen::Main;
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert_eq!(app.world().resource::<Session>().screen, menu::Screen::Main);
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
}

#[test]
fn failed_race_selection_keeps_existing_session_and_scene() {
    let options = Options::parse(std::iter::empty()).unwrap();
    let mut frontend = menu::Frontend::new(&options);
    frontend.catalog.circuits.push(menu::Circuit {
        id: "does-not-exist".into(),
        name: "Broken circuit".into(),
        length: 0.,
        outline: vec![],
        unavailable: None,
    });
    frontend.draft.track = "does-not-exist".into();
    frontend.pending = true;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .insert_resource(options)
        .insert_resource(frontend)
        .insert_resource(session())
        .add_systems(Update, menu::apply);
    let root = app.world_mut().spawn(assets::RaceVisualRoot).id();
    let initial = app.world().resource::<Session>().race.track.name.clone();
    app.update();
    assert_eq!(app.world().resource::<Session>().race.track.name, initial);
    assert!(app.world().get_entity(root).is_ok());
    assert!(app.world().resource::<menu::Frontend>().error.is_some());
}

/// Clicks the real egui tabs/presets/Start button, then reloads every installed map.
#[test]
#[ignore = "requires a Vulkan display and the complete original F1_TEST_ASSETS tree"]
fn rendered_menu_and_track_selection() {
    let root = std::env::var("F1_TEST_ASSETS").expect("set F1_TEST_ASSETS");
    let options = Options::parse(
        ["--assets", &root, "--opponents", "0", "--size", "1280x720"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();
    let mut app = build_app(options).unwrap();
    let probe = MenuProbe::default();
    let completed = probe.completed.clone();
    app.insert_resource(probe)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1. / 60.,
        )))
        .add_systems(
            PreUpdate,
            menu_probe
                .after(bevy_egui::EguiPreUpdateSet::ProcessInput)
                .before(bevy_egui::EguiPreUpdateSet::BeginPass)
                .before(controls::update),
        );
    assert!(app.run().is_success());
    assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
}

#[derive(Resource, Default)]
struct MenuProbe {
    initial_time: Option<f64>,
    stage: usize,
    ticks: usize,
    total: usize,
    case: usize,
    release: Option<bevy_egui::egui::Pos2>,
    requested: usize,
    captures: Arc<std::sync::atomic::AtomicUsize>,
    completed: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "exp")]
const MENU_CASES: [(&str, &str, camera::Mode); 4] = [
    ("silverstone", "gen22", camera::Mode::Cockpit),
    ("bahrain", "rb6", camera::Mode::Tv),
    ("cota", "gen22", camera::Mode::Chase),
    ("spa", "rb6", camera::Mode::Cockpit),
];

#[cfg(not(feature = "exp"))]
const MENU_CASES: [(&str, &str, camera::Mode); 4] = [
    ("spa", "gen22", camera::Mode::Cockpit),
    ("spa", "rb6", camera::Mode::Tv),
    ("spa", "rb6", camera::Mode::Chase),
    ("spa", "gen22", camera::Mode::Chase),
];

const HUD_CASES: [hud::Settings; 4] = [
    hud::Settings {
        layout: hud::Layout::Segments,
        map: hud::MapMode::FullCircuit,
    },
    hud::Settings {
        layout: hud::Layout::Gauges,
        map: hud::MapMode::Nearby,
    },
    hud::Settings {
        layout: hud::Layout::Segments,
        map: hud::MapMode::Nearby,
    },
    hud::Settings {
        layout: hud::Layout::Gauges,
        map: hud::MapMode::FullCircuit,
    },
];

fn choose_menu_case(frontend: &mut menu::Frontend, index: usize) {
    let (track, car, camera) = MENU_CASES[index];
    frontend.draft.track = track.into();
    frontend.draft.car = car.into();
    frontend.draft.camera = camera;
    frontend.draft.grid_slot = 1;
}

fn menu_click(
    probe: &mut MenuProbe,
    inputs: &mut Query<&mut bevy_egui::EguiInput, With<bevy_egui::PrimaryEguiContext>>,
    frontend: &menu::Frontend,
    name: &str,
) {
    use bevy_egui::egui::{Event, Modifiers, PointerButton};
    let position = frontend
        .widgets
        .get(name)
        .unwrap_or_else(|| panic!("missing UI control {name}"))
        .center();
    let mut input = inputs.single_mut().unwrap();
    input.0.events.push(Event::PointerMoved(position));
    input.0.events.push(Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::default(),
    });
    probe.release = Some(position);
}

fn menu_snapshot(commands: &mut Commands, probe: &mut MenuProbe, name: &str) {
    let directory = std::env::var_os("F1_TEST_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("f1-menu-flow-proof"));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{name}.png"));
    let captures = probe.captures.clone();
    probe.requested += 1;
    commands
        .spawn(Screenshot::primary_window())
        .observe(move |event: On<ScreenshotCaptured>| {
            event
                .image
                .clone()
                .try_into_dynamic()
                .unwrap()
                .to_rgb8()
                .save(&path)
                .unwrap();
            captures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            info!("Menu probe screenshot: {}", path.display());
        });
}

#[allow(clippy::too_many_arguments)]
fn menu_probe(
    mut commands: Commands,
    mut probe: ResMut<MenuProbe>,
    mut frontend: ResMut<menu::Frontend>,
    session: Res<Session>,
    options: Res<Options>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut inputs: Query<&mut bevy_egui::EguiInput, With<bevy_egui::PrimaryEguiContext>>,
    roots: Query<(), With<assets::RaceVisualRoot>>,
    cameras: Query<(), With<Camera3d>>,
    steering_wheels: Query<&assets::SteeringWheelVisual>,
    rig_parts: Query<&assets::RigPart>,
    mut exit: MessageWriter<AppExit>,
) {
    use bevy_egui::egui::{Event, Modifiers, PointerButton};
    probe.total += 1;
    let initial_time = *probe.initial_time.get_or_insert(session.race.time);
    assert!(
        probe.total < 6000,
        "menu probe stalled at stage {}: {:?}",
        probe.stage,
        frontend.error
    );
    assert!(frontend.error.is_none(), "menu error: {:?}", frontend.error);
    keys.reset_all();
    if let Some(position) = probe.release.take() {
        inputs
            .single_mut()
            .unwrap()
            .0
            .events
            .push(Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::default(),
            });
        return;
    }
    probe.ticks += 1;
    match probe.stage {
        0 if session.ready && probe.ticks > 30 => {
            assert_eq!(session.screen, menu::Screen::Main);
            assert!(session.paused);
            assert_eq!(session.race.time, initial_time);
            for (track, _, _) in MENU_CASES {
                assert!(frontend
                    .catalog
                    .circuits
                    .iter()
                    .any(|c| c.id == track && c.unavailable.is_none()));
            }
            menu_snapshot(&mut commands, &mut probe, "main-menu");
            menu_click(&mut probe, &mut inputs, &frontend, "Driving aids");
            probe.stage = 1;
            probe.ticks = 0;
        }
        1 if probe.ticks > 12 => {
            assert!(
                frontend.tab == menu::Tab::Aids,
                "Driving aids tab did not respond to pointer input"
            );
            menu_click(&mut probe, &mut inputs, &frontend, "beginner");
            probe.stage = 2;
            probe.ticks = 0;
        }
        2 if probe.ticks > 12 => {
            assert_eq!(frontend.aids.steering, 1.);
            assert_eq!(frontend.corner_braking, 1.);
            assert!(frontend.aids.automatic_gears);
            menu_snapshot(&mut commands, &mut probe, "driving-aids");
            menu_click(&mut probe, &mut inputs, &frontend, "Race setup");
            probe.stage = 3;
            probe.ticks = 0;
        }
        3 if probe.ticks > 12 => {
            assert!(frontend.tab == menu::Tab::Race);
            choose_menu_case(&mut frontend, probe.case);
            menu_click(&mut probe, &mut inputs, &frontend, "HUD & map");
            probe.stage = 14;
            probe.ticks = 0;
        }
        14 if probe.ticks > 12 => {
            assert!(frontend.tab == menu::Tab::Hud);
            let key = match HUD_CASES[probe.case].layout {
                hud::Layout::Segments => "hud-segments",
                hud::Layout::Gauges => "hud-gauges",
            };
            menu_click(&mut probe, &mut inputs, &frontend, key);
            probe.stage = 15;
            probe.ticks = 0;
        }
        15 if probe.ticks > 12 => {
            assert_eq!(frontend.display.layout, HUD_CASES[probe.case].layout);
            let key = match HUD_CASES[probe.case].map {
                hud::MapMode::FullCircuit => "map-full",
                hud::MapMode::Nearby => "map-nearby",
            };
            menu_click(&mut probe, &mut inputs, &frontend, key);
            probe.stage = 16;
            probe.ticks = 0;
        }
        16 if probe.ticks > 12 => {
            assert_eq!(frontend.display, HUD_CASES[probe.case]);
            if probe.case == 0 {
                menu_snapshot(&mut commands, &mut probe, "hud-settings");
            }
            menu_click(&mut probe, &mut inputs, &frontend, "Race setup");
            probe.stage = 17;
            probe.ticks = 0;
        }
        17 if probe.ticks > 12 => {
            assert!(frontend.tab == menu::Tab::Race);
            probe.stage = 4;
            probe.ticks = 0;
        }
        4 if probe.ticks > 12 => {
            menu_click(&mut probe, &mut inputs, &frontend, "start");
            probe.stage = 5;
            probe.ticks = 0;
        }
        5 if session.ready
            && session.screen == menu::Screen::Racing
            && options.track == MENU_CASES[probe.case].0 =>
        {
            let (track, car, mode) = MENU_CASES[probe.case];
            assert_eq!(options.track, track);
            assert_eq!(frontend.display, HUD_CASES[probe.case]);
            assert_eq!(
                session.race.entrants[session.player]
                    .car
                    .spec
                    .name
                    .to_ascii_lowercase(),
                car
            );
            assert_eq!(
                session.race.entrants[session.player].car.assists,
                frontend.aids
            );
            assert_eq!(session.camera.mode, mode);
            assert_eq!(
                roots.iter().count(),
                2,
                "old scene roots leaked across a track switch"
            );
            assert_eq!(cameras.iter().count(), 1);
            assert!(
                steering_wheels
                    .iter()
                    .any(|wheel| wheel.entrant == session.player)
                    || rig_parts.iter().any(|part| part.entrant == session.player
                        && matches!(part.kind, assets::RigKind::Steering)),
                "{car} has no animated cockpit wheel"
            );
            assert!(!session.paused);
            probe.stage = 6;
            probe.ticks = 0;
        }
        6 if probe.ticks > 30 => {
            let track = MENU_CASES[probe.case].0;
            menu_snapshot(&mut commands, &mut probe, track);
            if probe.case + 1 == MENU_CASES.len() {
                probe.stage = 12;
            } else {
                probe.stage = 13;
            }
            probe.ticks = 0;
        }
        13 if probe.ticks > 4 => {
            keys.press(KeyCode::Escape);
            probe.stage = if probe.case == 0 { 7 } else { 11 };
            probe.ticks = 0;
        }
        7 if probe.ticks > 12 => {
            assert_eq!(session.screen, menu::Screen::Paused);
            assert!(session.paused);
            menu_click(&mut probe, &mut inputs, &frontend, "Driving aids");
            probe.stage = 8;
            probe.ticks = 0;
        }
        8 if probe.ticks > 12 => {
            assert!(frontend.tab == menu::Tab::Aids);
            menu_click(&mut probe, &mut inputs, &frontend, "unassisted");
            probe.stage = 9;
            probe.ticks = 0;
        }
        9 if probe.ticks > 12 => {
            let aids = session.race.entrants[session.player].car.assists;
            assert_eq!(aids, frontend.aids);
            assert_eq!(
                [aids.steering, aids.braking, aids.traction, aids.stability],
                [0.; 4]
            );
            assert!(!aids.automatic_gears);
            assert_eq!(session.corner_braking, 0.);
            assert!(!session.show_racing_line);
            menu_snapshot(&mut commands, &mut probe, "pause-aids");
            menu_click(&mut probe, &mut inputs, &frontend, "Race setup");
            probe.stage = 11;
            probe.ticks = 0;
        }
        11 if probe.ticks > 12 => {
            assert_eq!(session.screen, menu::Screen::Paused);
            assert!(frontend.tab == menu::Tab::Race);
            probe.case += 1;
            probe.stage = 3;
            probe.ticks = 0;
        }
        12 if probe.captures.load(std::sync::atomic::Ordering::SeqCst) == probe.requested => {
            probe
                .completed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            info!(
                "Menu probe: pointer controls, live assists, all HUD/map choices and all four map/car switches passed"
            );
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

/// Runs the actual renderer with external assets and injected Bevy keyboard state.
#[test]
#[ignore = "requires a Vulkan display and F1_TEST_ASSETS"]
fn rendered_game_flows() {
    let root = std::env::var("F1_TEST_ASSETS").expect("set F1_TEST_ASSETS to generated assets");
    let size = std::env::var("F1_TEST_SIZE").unwrap_or_else(|_| "1280x720".into());
    // F1_TEST_CAR=gen22 runs the same flow with the modern car's live wheel LCD.
    let car = std::env::var("F1_TEST_CAR").unwrap_or_else(|_| "rb6".into());
    let options = Options::parse(
        [
            "--assets",
            &root,
            "--size",
            &size,
            "--car",
            &car,
            "--auto-gears",
            "--quick-race",
            // Injected controls must keep running when the compositor has no seat focus.
            "--frames",
            "10000",
        ]
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
    mut exit: MessageWriter<AppExit>,
) {
    if !session.ready {
        return;
    }
    probe.frame += 1;
    keys.reset_all();
    keys.press(KeyCode::KeyW);
    if probe.completed.load(std::sync::atomic::Ordering::SeqCst) {
        exit.write(AppExit::Success);
    }
    match probe.frame {
        60 | 90 | 120 | 500 | 570 | 640 => keys.press(KeyCode::KeyC),
        670 | 695 => keys.press(KeyCode::F4),
        180 | 210 => keys.press(KeyCode::KeyP),
        730 => keys.press(KeyCode::KeyR),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn probe_assertions(
    mut commands: Commands,
    mut session: ResMut<Session>,
    mut probe: ResMut<RenderProbe>,
    wheels: Query<&assets::WheelVisual>,
    steering_wheels: Query<(&assets::SteeringWheelVisual, &Transform)>,
    rig_parts: Query<(&assets::RigPart, &Transform)>,
    drivers: Query<&bevy::mesh::morph::MorphWeights, With<assets::DriverMorph>>,
    paddles: Query<(), With<assets::Paddle>>,
    bodies: Query<&assets::CarVisual>,
    meshes: Query<&Mesh3d>,
    images: Res<Assets<Image>>,
    surfaces: Query<(), With<materials::SurfaceReady>>,
    fixed: Res<Time<Fixed>>,
    mut frontend: ResMut<menu::Frontend>,
) {
    let frame = probe.frame;
    match frame {
        300 => frontend.display = HUD_CASES[1],
        360 => frontend.display = HUD_CASES[2],
        420 => frontend.display = HUD_CASES[3],
        480 => frontend.display = HUD_CASES[0],
        _ => {}
    }
    if frame == 540 {
        let pose = session
            .poses
            .at(&session.race, session.player, fixed.overstep_fraction_f64());
        for (part, transform) in &rig_parts {
            if part.entrant == session.player && matches!(part.kind, assets::RigKind::Steering) {
                assert!(transform.rotation.abs_diff_eq(
                    part.rest.rotation * Quat::from_rotation_z(-pose.steer as f32 * 6.),
                    1e-5
                ));
            }
        }
        for (wheel, transform) in &steering_wheels {
            if wheel.entrant == session.player {
                let rotation = Quat::from_rotation_z(-pose.steer as f32 * 6.0);
                assert!(transform
                    .rotation
                    .abs_diff_eq(rotation * wheel.rest.rotation, 1e-5));
                assert!(transform
                    .transform_point(
                        wheel
                            .rest
                            .compute_affine()
                            .inverse()
                            .transform_point3(wheel.pivot)
                    )
                    .abs_diff_eq(wheel.pivot, 1e-5));
            }
        }
    }
    match frame {
        30 => {
            assert!(
                steering_wheels.iter().count()
                    + rig_parts
                        .iter()
                        .filter(|(part, _)| matches!(part.kind, assets::RigKind::Steering))
                        .count()
                    >= session.race.entrants.len()
            );
            assert_eq!(wheels.iter().count(), session.race.entrants.len() * 4);
            assert_eq!(bodies.iter().count(), session.race.entrants.len());
            if !session.rig.driver_poses.is_empty() {
                // Exercise the real GLB loader and scene tagging for both LODs.
                assert_eq!(drivers.iter().count(), session.race.entrants.len() * 2);
                assert_eq!(paddles.iter().count(), session.race.entrants.len() * 2);
                for weights in &drivers {
                    assert_eq!(
                        weights.weights().len(),
                        session.rig.driver_poses.len() + session.rig.driver_shift_poses.len()
                    );
                }
            }
            assert!(meshes.iter().count() > 40);
            assert!(
                surfaces.iter().count() > 40,
                "car material calibration did not run"
            );
            let car = std::env::var("F1_TEST_CAR").unwrap_or_else(|_| "rb6".into());
            assert!(session.race.entrants[session.player]
                .car
                .spec
                .name
                .eq_ignore_ascii_case(&car));
            let mipped = images
                .iter()
                .filter(|(_, image)| image.texture_descriptor.mip_level_count > 1)
                .count();
            assert!(
                mipped >= 100,
                "expected track/car mip chains, found {mipped}"
            );
        }
        60 => assert_eq!(session.camera.mode, camera::Mode::Cockpit),
        90 => assert_eq!(session.camera.mode, camera::Mode::Tv),
        120 => assert_eq!(session.camera.mode, camera::Mode::Chase),
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
        540 => assert_eq!(session.camera.mode, camera::Mode::Cockpit),
        610 => assert_eq!(session.camera.mode, camera::Mode::Tv),
        671 => {
            assert_eq!(session.screen, menu::Screen::CameraEditor);
            assert!(session.paused);
            probe.paused_time = session.race.time;
            let mut tuning = session.camera.tuning.clone();
            tuning.chase.height = 1.9;
            session.camera.configure(tuning);
        }
        690 => {
            assert_eq!(session.race.time, probe.paused_time);
            assert_eq!(session.camera.tuning.chase.height, 1.9);
        }
        730 => assert!(session.race.entrants[session.player].car.speed() < 0.5),
        720 => {
            assert!(session.race.entrants[session.player].car.speed() > 5.0);
            assert_eq!(session.race.phase, racecraft_sim::Phase::Racing);
            info!("Render probe: driving, cameras and pause/resume passed");
        }
        // Force the presentation state to exercise the results HUD separately from the sim's race-completion tests.
        740 => session.race.phase = racecraft_sim::Phase::Finished,
        780..=899 if probe.captures == 12 => {
            probe
                .completed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            info!("Render probe: recovery, camera editor, both HUDs, both maps and all twelve screenshots passed");
        }
        900 => panic!("screenshots did not complete"),
        _ => {}
    }
    let name = match frame {
        30 => "grid",
        80 => "cockpit",
        110 => "onboard",
        200 => "paused",
        330 => "gauges-nearby-driving",
        390 => "segments-nearby-driving",
        450 => "gauges-full-driving",
        540 => "moving-cockpit",
        610 => "moving-tv",
        685 => "camera-editor",
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
