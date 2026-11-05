//! Actual menu pointer interaction and native screenshots at the requested size.
use super::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[test]
#[ignore = "requires Vulkan display, F1_TEST_ASSETS and a built F1_TEST_GAME_BINARY for the launch case"]
fn rendered_responsive_settings() {
    let assets = std::env::var("F1_TEST_ASSETS").unwrap();
    let size = std::env::var("F1_TEST_SIZE").unwrap_or("1920x1080".into());
    let options = Options::parse(
        [
            "--assets", &assets, "--size", &size, "--frames", "100000", "--mute",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap();
    let mut app = crate::build_app(options).unwrap();
    {
        let mut session = app.world_mut().resource_mut::<Session>();
        session.screen = Screen::Main;
        session.paused = true;
    }
    let proof = Proof::default();
    let done = proof.done.clone();
    app.insert_resource(proof).add_systems(
        PreUpdate,
        drive
            .after(bevy_egui::EguiPreUpdateSet::ProcessInput)
            .before(bevy_egui::EguiPreUpdateSet::BeginPass),
    );
    assert!(app.run().is_success());
    assert!(done.load(Ordering::SeqCst));
}

#[derive(Resource, Default)]
struct Proof {
    step: usize,
    car_step: usize,
    scrolled: bool,
    frames: usize,
    ticks: usize,
    release: Option<egui::Pos2>,
    captures: Arc<AtomicUsize>,
    requested: usize,
    done: Arc<AtomicBool>,
    began: Option<std::time::Instant>,
    initial_time: Option<f64>,
}
fn snapshot(commands: &mut Commands, proof: &mut Proof, name: &str) {
    let directory = std::path::PathBuf::from(std::env::var_os("F1_TEST_OUTPUT").unwrap());
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{name}.png"));
    let count = proof.captures.clone();
    proof.requested += 1;
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
            count.fetch_add(1, Ordering::SeqCst);
        });
}
fn click(proof: &mut Proof, input: &mut bevy_egui::EguiInput, position: egui::Pos2) {
    input.0.events.push(egui::Event::PointerMoved(position));
    input.0.events.push(egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    proof.release = Some(position);
}
#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut proof: ResMut<Proof>,
    mut frontend: ResMut<Frontend>,
    mut session: ResMut<Session>,
    mut inputs: Query<&mut bevy_egui::EguiInput, With<bevy_egui::PrimaryEguiContext>>,
    mut contexts: EguiContexts,
    car_preview: Res<super::car_preview::CarPreview>,
    preview_cameras: Query<
        (&Camera, &bevy::camera::visibility::RenderLayers),
        With<super::car_preview::PreviewCamera>,
    >,
    children: Query<&Children>,
    meshes: Query<&bevy::camera::visibility::RenderLayers, With<Mesh3d>>,
    mut exit: MessageWriter<AppExit>,
) {
    let began = *proof.began.get_or_insert_with(std::time::Instant::now);
    assert!(
        began.elapsed().as_secs() < 150,
        "settings proof stalled at {} (car step {}, preview ready {}, team {})",
        proof.step,
        proof.car_step,
        car_preview.ready,
        frontend.draft.team
    );
    if !session.ready {
        return;
    }
    let initial_time = *proof.initial_time.get_or_insert(session.race.time);
    assert_eq!(
        session.race.time, initial_time,
        "settings replaced or advanced the paused race"
    );
    proof.frames += 1;
    let mut input = inputs.single_mut().unwrap();
    if let Some(position) = proof.release.take() {
        input.0.events.push(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        return;
    }
    proof.ticks += 1;
    if proof.ticks < 20 || proof.frames < 40 {
        return;
    }
    let rect = contexts.ctx_mut().unwrap().content_rect();
    for tab in Tab::ALL {
        let name = tab.label();
        let Some(widget) = frontend.widgets.get(name) else {
            continue;
        };
        let widget = if let Some(popup) = frontend.widgets.get("settings-navigation-popup") {
            if !popup.intersects(*widget) {
                continue;
            }
            widget.intersect(*popup)
        } else {
            *widget
        };
        assert!(
            rect.contains_rect(widget),
            "tab {name} outside viewport: {widget:?}"
        );
    }
    assert!(
        rect.contains_rect(frontend.widgets["start"]),
        "start action outside viewport"
    );
    assert!(
        rect.contains_rect(frontend.widgets["settings-body"]),
        "settings body overflows viewport"
    );
    match proof.step {
        step if step < Tab::ALL.len() => {
            let tab = Tab::ALL[proof.step];
            let label = tab.label();
            if frontend.tab != tab {
                if let (Some(button), Some(popup)) = (
                    frontend.widgets.get(label),
                    frontend.widgets.get("settings-navigation-popup"),
                ) {
                    if button.intersect(*popup).height() < button.height().min(12.) {
                        input
                            .0
                            .events
                            .push(egui::Event::PointerMoved(popup.center()));
                        input.0.events.push(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            phase: egui::TouchPhase::Move,
                            delta: egui::vec2(
                                0.,
                                if button.center().y > popup.center().y {
                                    -180.
                                } else {
                                    180.
                                },
                            ),
                            modifiers: egui::Modifiers::default(),
                        });
                        proof.ticks = 0;
                        return;
                    }
                }
                let button = frontend
                    .widgets
                    .get(label)
                    .or_else(|| frontend.widgets.get("settings-navigation"))
                    .expect("settings navigation is missing");
                let position =
                    if let Some(popup) = frontend.widgets.get("settings-navigation-popup") {
                        button.intersect(*popup).left_center() + egui::vec2(40., 0.)
                    } else {
                        button.center()
                    };
                click(&mut proof, &mut input, position);
                proof.ticks = 0;
                return;
            }
            if tab == Tab::Car && !car_preview.ready {
                proof.ticks = 0;
                return;
            }
            if tab == Tab::Car {
                let root = car_preview.root().expect("ready preview must have a scene");
                let mut mesh_count = 0;
                for entity in children.iter_descendants(root) {
                    if let Ok(layers) = meshes.get(entity) {
                        assert_eq!(*layers, bevy::camera::visibility::RenderLayers::layer(2));
                        mesh_count += 1;
                    }
                }
                assert!(mesh_count > 0, "ready preview must contain visible meshes");
                let (camera, layers) = preview_cameras.single().unwrap();
                let _ = camera;
                assert!(
                    *layers == bevy::camera::visibility::RenderLayers::layer(2),
                    "menu changed the preview camera's isolated layer"
                );
            }
            if tab == Tab::Car && proof.car_step < 2 {
                let button = frontend.widgets[if proof.car_step == 0 {
                    "car-rb6"
                } else {
                    "car-gen22"
                }];
                let body = frontend.widgets["settings-content"];
                if button.top() < body.top() || button.bottom() > body.bottom() {
                    input
                        .0
                        .events
                        .push(egui::Event::PointerMoved(body.center()));
                    input.0.events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(
                            0.,
                            if button.center().y > body.center().y {
                                -180.
                            } else {
                                180.
                            },
                        ),
                        modifiers: egui::Modifiers::default(),
                    });
                    proof.ticks = 0;
                    return;
                }
                if proof.car_step == 0 {
                    click(&mut proof, &mut input, frontend.widgets["car-rb6"].center());
                    proof.car_step = 1;
                } else {
                    assert_eq!(car_preview.selected_car(), Some("rb6"));
                    snapshot(&mut commands, &mut proof, "settings-Car-RB6");
                    click(
                        &mut proof,
                        &mut input,
                        frontend.widgets["car-gen22"].center(),
                    );
                    proof.car_step = 2;
                }
                proof.ticks = 0;
                return;
            }
            if tab == Tab::Car && proof.car_step == 2 {
                assert_eq!(car_preview.selected_car(), Some("gen22"));
            }
            if tab == Tab::Car && proof.car_step == 2 && rect.width() > 1100. {
                assert_eq!(frontend.draft.car, "gen22");
                if frontend.draft.team != "ferrari" {
                    click(
                        &mut proof,
                        &mut input,
                        frontend.widgets["team-ferrari"].center(),
                    );
                    proof.ticks = 0;
                    return;
                }
                let team = frontend
                    .teams
                    .teams
                    .iter()
                    .find(|team| team.id == "ferrari")
                    .unwrap();
                assert_eq!(frontend.draft.driver, team.drivers[0].id);
                // The preview reloads with the team's livery before the capture.
                if !car_preview.ready {
                    proof.ticks = 0;
                    return;
                }
                snapshot(&mut commands, &mut proof, "settings-Driver-Ferrari");
                proof.car_step = 3;
            }
            if tab == Tab::Audio
                && !proof.scrolled
                && frontend.audio.engine != crate::audio::EngineChoice::V12
            {
                let button = frontend.widgets["3.0 V12"];
                // A widget may be inside the window but clipped below the scroll
                // viewport, especially at 640×600. Scroll it into view before clicking.
                let body = frontend.widgets["settings-content"];
                if button.top() < body.top() || button.bottom() > body.bottom() {
                    input
                        .0
                        .events
                        .push(egui::Event::PointerMoved(body.center()));
                    input.0.events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(
                            0.,
                            if button.center().y > body.center().y {
                                -180.
                            } else {
                                180.
                            },
                        ),
                        modifiers: egui::Modifiers::default(),
                    });
                    proof.ticks = 0;
                    return;
                }
                // A justified radio row includes blank space; click its visible icon.
                click(
                    &mut proof,
                    &mut input,
                    button.left_center() + egui::vec2(8., 0.),
                );
                proof.ticks = 0;
                return;
            }
            let suffix = if proof.scrolled { "bottom" } else { "top" };
            snapshot(
                &mut commands,
                &mut proof,
                &format!("settings-{}-{suffix}", label.replace(' ', "-")),
            );
            if proof.scrolled {
                proof.step += 1;
                proof.scrolled = false;
            } else {
                input.0.events.push(egui::Event::PointerMoved(
                    frontend.widgets["settings-body"].center(),
                ));
                input.0.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., -1800.),
                    modifiers: egui::Modifiers::default(),
                });
                proof.scrolled = true;
            }
            proof.ticks = 0;
        }
        step if step == Tab::ALL.len() => {
            if proof.ticks == 20 {
                input.0.events.push(egui::Event::PointerMoved(
                    frontend.widgets["settings-body"].center(),
                ));
                input.0.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., 3000.),
                    modifiers: egui::Modifiers::default(),
                });
                return;
            }
            if proof.ticks < 40 {
                return;
            }
            if std::env::var_os("F1_TEST_BENCHMARK_LAUNCH").is_some() {
                let button = frontend.widgets["benchmark-run"];
                assert!(rect.contains_rect(button));
                click(&mut proof, &mut input, button.center());
                proof.step = Tab::ALL.len() + 1;
            } else {
                proof.step = Tab::ALL.len() + 2;
            }
            proof.ticks = 0;
        }
        step if step == Tab::ALL.len() + 1 => {
            if frontend.benchmark.child.is_some() {
                if proof.ticks == 40 {
                    snapshot(&mut commands, &mut proof, "benchmark-running");
                }
                return;
            }
            let output = frontend
                .benchmark
                .output
                .as_ref()
                .expect("benchmark button did not launch process");
            assert!(
                output.join("benchmark.json").is_file(),
                "benchmark failed: {:?}",
                frontend.benchmark.message
            );
            assert_eq!(
                frontend.benchmark.message.as_deref(),
                Some("Benchmark complete. Frame times and six screenshots are saved below.")
            );
            snapshot(&mut commands, &mut proof, "benchmark-complete");
            proof.step = Tab::ALL.len() + 2;
            proof.ticks = 0;
        }
        step if step == Tab::ALL.len() + 2 => {
            session.screen = Screen::Paused;
            frontend.race_started = true;
            snapshot(&mut commands, &mut proof, "paused-settings");
            proof.step = Tab::ALL.len() + 3;
            proof.ticks = 0;
        }
        step if step == Tab::ALL.len() + 3 => {
            session.screen = Screen::CameraEditor;
            proof.step = Tab::ALL.len() + 4;
            proof.ticks = 0;
        }
        step if step == Tab::ALL.len() + 4 => {
            snapshot(&mut commands, &mut proof, "camera-editor");
            proof.step = Tab::ALL.len() + 5;
            proof.ticks = 0;
        }
        _ if proof.captures.load(Ordering::SeqCst) == proof.requested => {
            proof.done.store(true, Ordering::SeqCst);
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
