//! Opt-in native replay: scripted inputs reach real wheel telemetry, then the renderer.
use crate::*;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    time::TimeUpdateStrategy,
};
use racecraft_sim::{Assists, Car, CarInput, Race};
use std::{path::PathBuf, time::Duration};

#[derive(Resource, Default)]
struct Replay {
    frame: usize,
    stage: usize,
    source: Option<Race>,
    manifest: Vec<serde_json::Value>,
    captured: usize,
}
const NAMES: [&str; 9] = [
    "grid",
    "straight-320",
    "corner",
    "cockpit-speed",
    "braking",
    "lockup",
    "offtrack",
    "compression-sparks",
    "storm",
];

#[test]
#[ignore = "requires F1_TEST_ASSETS, F1_TEST_OUTPUT and an isolated Vulkan display"]
fn scripted_captures() {
    let assets = std::env::var("F1_TEST_ASSETS").unwrap();
    let options = options::Options::parse(
        [
            "--assets",
            &assets,
            "--graphics",
            &std::env::var("F1_REVIEW_PRESET").unwrap_or("high".into()),
            "--size",
            "1920x1080",
            "--mute",
            "--frames",
            "100000",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(
            std::env::var("F1_REVIEW_TEAM")
                .ok()
                .into_iter()
                .flat_map(|team| ["--team".to_owned(), team]),
        ),
    )
    .unwrap();
    let mut app = build_app(options).unwrap();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1. / 60.,
    )))
    .init_resource::<Replay>()
    .add_systems(First, |mut session: ResMut<Session>| {
        session.paused = true;
    })
    .add_systems(
        Update,
        drive.after(benchmark::advance).before(assets::sync_cars),
    )
    .add_systems(
        Update,
        capture_review.after(effects::update).after(camera::update),
    );
    assert!(app.run().is_success());
}

fn place(session: &mut Session, stage: usize) {
    let player = session.player;
    let (s, speed, lateral) = match stage {
        1 | 3 => (1600., 88.89, 0.),
        2 => (248., 19., 0.),
        4 | 5 => (2100., 72., 0.),
        6 => (1600., 35., 5.),
        7 => (892., 70., 0.),
        _ => (0., 0., 0.),
    };
    if stage != 0 && stage != 8 {
        session.race.phase = racecraft_sim::Phase::Racing;
        session.race.time = 0.;
        let frame = session.race.track.frame_at(s);
        let spec = session.race.entrants[player].car.spec.clone();
        let setup = session.race.entrants[player].car.setup;
        let position = frame.point_at(lateral);
        let mut car = Car::new(
            spec,
            setup,
            Assists {
                steering: 0.,
                braking: 0.,
                traction: 0.,
                stability: 0.,
                automatic_gears: true,
                ..default()
            },
            &session.race.track,
            position,
            frame.tangent.x.atan2(frame.tangent.y),
        );
        car.velocity = frame.tangent * speed;
        car.powertrain.gear = if speed > 60. { 7 } else { 3 };
        for tyre in &mut car.tyres {
            tyre.temperature = 90.;
        }
        session.race.entrants[player].car = car;
    }
    session.poses = pose::Poses::new(&session.race);
    session.camera = camera::ChaseCamera::new(&session.race, player);
    session.camera.set_cockpit_eye(session.rig.cockpit_eye);
    session.camera.select(if stage == 3 {
        camera::Mode::Cockpit
    } else {
        camera::Mode::Chase
    });
}

fn drive(
    mut session: ResMut<Session>,
    mut replay: ResMut<Replay>,
    mut frontend: ResMut<menu::Frontend>,
) {
    if !session.ready {
        return;
    }
    if replay.source.is_none() {
        replay.source = Some(session.race.clone());
    }
    if replay.frame == 0 {
        session.race = replay.source.as_ref().unwrap().clone();
        session.messages = default();
        session.delta = default();
        session.telemetry = default();
        place(&mut session, replay.stage);
        frontend.draft.weather = if replay.stage == 8 {
            sky::Weather::Storm
        } else {
            sky::Weather::Clear
        };
    }
    replay.frame += 1;
    session.paused = false;
    session.show_racing_line = false;
    if replay.frame <= 40 {
        return;
    }
    let input = match replay.stage {
        1 | 3 => CarInput {
            throttle: 1.,
            ..default()
        },
        2 => CarInput {
            throttle: 0.12,
            steer: -0.6,
            ..default()
        },
        4 | 5 => CarInput {
            brake: 1.,
            ..default()
        },
        6 => CarInput {
            throttle: 0.6,
            steer: 0.45,
            ..default()
        },
        7 => CarInput {
            throttle: 1.,
            ..default()
        },
        _ => CarInput::default(),
    };
    if replay.stage != 0 && replay.stage != 8 {
        for _ in 0..6 {
            let Session { poses, race, .. } = &mut *session;
            poses.record(race);
            race.step(&[input]);
        }
        session.sample_hud();
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_review(
    mut commands: Commands,
    mut replay: ResMut<Replay>,
    session: Res<Session>,
    cameras: Query<(&Transform, &Projection), With<bevy_egui::PrimaryEguiContext>>,
    windows: Query<&Window>,
    frontend: Res<menu::Frontend>,
    particles: Query<&effects::Particle>,
    mut exit: MessageWriter<AppExit>,
) {
    if !session.ready {
        return;
    }
    let at = match replay.stage {
        0 | 8 => 70,
        1 | 3 => 70,
        2 => 95,
        4 => 58,
        5 => 68,
        6 => 100,
        7 => 65,
        _ => 70,
    };
    if replay.frame == at {
        let root = PathBuf::from(std::env::var_os("F1_TEST_OUTPUT").unwrap());
        std::fs::create_dir_all(&root).unwrap();
        let name = NAMES[replay.stage];
        let path = root.join(format!("{name}.png"));
        let car = &session.race.entrants[session.player].car;
        let window = windows.iter().next().unwrap();
        assert_eq!(
            [window.physical_width(), window.physical_height()],
            [1920, 1080]
        );
        let (transform, projection) = cameras.iter().next().unwrap();
        let view = transform.to_matrix().inverse();
        let clip = projection.get_clip_from_view();
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for x in [-1.05, 1.05] {
            for y in [0., 1.3] {
                for z in [-2.8, 2.8] {
                    let point = car.model_to_world(glam::DVec3::new(x, y, z)).as_vec3();
                    let projected = clip * view * point.extend(1.);
                    let ndc = projected.truncate() / projected.w;
                    bounds[0] = bounds[0].min(ndc.x);
                    bounds[1] = bounds[1].min(ndc.y);
                    bounds[2] = bounds[2].max(ndc.x);
                    bounds[3] = bounds[3].max(ndc.y);
                }
            }
        }
        if [1, 2, 4, 6, 7].contains(&replay.stage) {
            assert!(
                bounds[0] > -1. && bounds[1] > -1. && bounds[2] < 1. && bounds[3] < 1.,
                "car clipped: {bounds:?}"
            );
            assert!(bounds[2] - bounds[0] > 0.4, "car too small: {bounds:?}");
        }
        if replay.stage == 1 {
            assert!((310. ..330.).contains(&(car.speed() * 3.6)));
        }
        if replay.stage == 5 {
            assert!(car.wheels.iter().any(|w| w.locked));
            if frontend.video.effects != graphics::Level::Off {
                assert!(particles.iter().any(|p| p.label() == "smoke"));
                assert!(particles.iter().any(|p| p.label() == "rubber"));
            }
        }
        if replay.stage == 6 {
            assert!(car
                .wheels
                .iter()
                .any(|w| w.surface == racecraft_sim::Surface::Grass));
            if frontend.video.effects != graphics::Level::Off {
                assert!(particles.iter().any(|p| p.label() == "grass"));
            }
        }
        if replay.stage == 7 && frontend.video.effects != graphics::Level::Off {
            assert!(
                particles.iter().any(|p| p.label() == "spark"),
                "compression replay produced no proxy sparks"
            );
        }
        let surfaces: Vec<_> = car
            .wheels
            .iter()
            .map(|w| format!("{:?}", w.surface))
            .collect();
        let wheel_flags:Vec<_>=car.wheels.iter().map(|w|serde_json::json!({"locked":w.locked,"spinning":w.spinning,"sliding":w.sliding,"load":w.load})).collect();
        let value = serde_json::json!({"name":name,"speed_kmh":car.speed()*3.6,"acceleration":car.acceleration.to_array(),"surfaces":surfaces,"wheels":wheel_flags,"particles_and_marks":particles.iter().count(),"effect_kinds":particles.iter().map(|p|p.label()).collect::<Vec<_>>(),"body_ground_lag":car.position.y-car.body_height,"car_ndc_bounds":bounds,"camera":transform.translation.to_array(),"race_time":session.race.time});
        println!("REVIEW {value}");
        replay.manifest.push(value);
        commands.spawn(Screenshot::primary_window()).observe(
            move |event: On<ScreenshotCaptured>, mut replay: ResMut<Replay>| {
                event
                    .image
                    .clone()
                    .try_into_dynamic()
                    .unwrap()
                    .to_rgb8()
                    .save(&path)
                    .unwrap();
                replay.captured += 1;
            },
        );
    }
    if replay.stage == NAMES.len() - 1 && replay.frame >= at + 15 && replay.captured < NAMES.len() {
        assert!(replay.frame < at + 3600, "screenshot saves timed out");
        return;
    }
    if replay.frame >= at + 15 {
        replay.frame = 0;
        replay.stage += 1;
        if replay.stage == NAMES.len() {
            assert_eq!(replay.captured, NAMES.len());
            let root = PathBuf::from(std::env::var_os("F1_TEST_OUTPUT").unwrap());
            std::fs::write(
                root.join("telemetry.json"),
                serde_json::to_vec_pretty(&replay.manifest).unwrap(),
            )
            .unwrap();
            exit.write(AppExit::Success);
        }
    }
}
