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
const TABS: [(&str, Tab); 7] = [
    ("Race setup", Tab::Race),
    ("Driving aids", Tab::Aids),
    ("Cameras & controls", Tab::Controls),
    ("Graphics", Tab::Graphics),
    ("Audio", Tab::Audio),
    ("HUD & map", Tab::Hud),
    ("Benchmark", Tab::Benchmark),
];
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
fn drive(
    mut commands: Commands,
    mut proof: ResMut<Proof>,
    mut frontend: ResMut<Frontend>,
    mut session: ResMut<Session>,
    mut inputs: Query<&mut bevy_egui::EguiInput, With<bevy_egui::PrimaryEguiContext>>,
    mut contexts: EguiContexts,
    mut exit: MessageWriter<AppExit>,
) {
    let began = *proof.began.get_or_insert_with(std::time::Instant::now);
    assert!(
        began.elapsed().as_secs() < 150,
        "settings proof stalled at {}",
        proof.step
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
    for (name, _) in TABS {
        let Some(widget) = frontend.widgets.get(name) else {
            return;
        };
        assert!(
            rect.contains_rect(*widget),
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
        step if step < TABS.len() => {
            let (label, tab) = TABS[proof.step];
            if frontend.tab != tab {
                click(&mut proof, &mut input, frontend.widgets[label].center());
                proof.ticks = 0;
                return;
            }
            if tab == Tab::Audio
                && !proof.scrolled
                && frontend.audio.engine != crate::audio::EngineChoice::V12
            {
                let button = frontend.widgets["3.0 V12"];
                // A widget may be inside the window but clipped below the scroll
                // viewport, especially at 640×600. Scroll it into view before clicking.
                let body = frontend.widgets["settings-body"].shrink(18.);
                if !body.contains_rect(button) {
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
                click(&mut proof, &mut input, button.center());
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
        step if step == TABS.len() => {
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
                proof.step = TABS.len() + 1;
            } else {
                proof.step = TABS.len() + 2;
            }
            proof.ticks = 0;
        }
        step if step == TABS.len() + 1 => {
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
            proof.step = TABS.len() + 2;
            proof.ticks = 0;
        }
        step if step == TABS.len() + 2 => {
            session.screen = Screen::Paused;
            frontend.race_started = true;
            snapshot(&mut commands, &mut proof, "paused-settings");
            proof.step = TABS.len() + 3;
            proof.ticks = 0;
        }
        step if step == TABS.len() + 3 => {
            session.screen = Screen::CameraEditor;
            proof.step = TABS.len() + 4;
            proof.ticks = 0;
        }
        step if step == TABS.len() + 4 => {
            snapshot(&mut commands, &mut proof, "camera-editor");
            proof.step = TABS.len() + 5;
            proof.ticks = 0;
        }
        _ if proof.captures.load(Ordering::SeqCst) == proof.requested => {
            proof.done.store(true, Ordering::SeqCst);
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
