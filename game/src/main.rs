//! Bevy client for the deterministic racecraft simulation.

mod assets;
mod audio;
mod benchmark;
mod camera;
mod canvas;
mod controls;
mod driving;
mod graphics;
mod hud;
mod materials;
mod menu;
mod options;
mod pose;
mod preferences;
mod sky;
#[cfg(test)]
mod tests;
mod textures;
#[cfg(test)]
mod track_probe;
mod wheel_dash;

use bevy::{
    app::AppExit,
    audio::AddAudioSource,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass};
use options::Options;
use racecraft_sim::{Race, PHYSICS_RATE};
use std::process::ExitCode;

#[derive(Resource)]
pub struct Session {
    pub race: Race,
    pub player: usize,
    pub camera: camera::ChaseCamera,
    pub controls: controls::Controls,
    pub paused: bool,
    pub minimap: hud::Minimap,
    pub messages: hud::Messages,
    pub delta: hud::Delta,
    pub ready: bool,
    pub screen: menu::Screen,
    pub guidance: driving::Guidance,
    pub corner_braking: f64,
    pub show_racing_line: bool,
    pub show_performance: bool,
    pub poses: pose::Poses,
}

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{}", Options::USAGE);
            return if message == "help" {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    match run(options) {
        Ok(exit) if exit.is_success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// NVIDIA's Wayland presentation waits for each frame to finish rendering
/// before returning, so the CPU cannot prepare the next frame while the GPU
/// draws this one: both sit half idle. Presenting through XWayland overlaps
/// them (116 -> 191 fps on an RTX 3050 laptop) with identical output.
/// `F1_WAYLAND=1` keeps native Wayland.
fn prefer_x11_on_nvidia() {
    let nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists();
    if cfg!(target_os = "linux")
        && nvidia
        && std::env::var_os("F1_WAYLAND").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_some()
    {
        // Single-threaded: no other thread can be reading the environment yet.
        std::env::remove_var("WAYLAND_DISPLAY");
    }
}

fn run(options: Options) -> Result<AppExit, String> {
    prefer_x11_on_nvidia();
    Ok(build_app(options)?.run())
}

fn build_app(mut options: Options) -> Result<App, String> {
    options.assets = options
        .assets
        .canonicalize()
        .map_err(|error| format!("asset directory {}: {error}", options.assets.display()))?;
    let mut race = assets::load_race(&options)?;
    let benchmark = if options.benchmark.is_some() {
        let (planned_race, run) = benchmark::Run::plan(&options, &race)?;
        race = planned_race;
        Some(run)
    } else {
        None
    };
    let player = options.grid_slot - 1;
    let title = format!("F1 Racecraft - {}", race.track.name);
    let quick_race = options.quick_race || options.frames.is_some();
    let frontend = menu::Frontend::new(&options);
    let mut rig = camera::ChaseCamera::new(&race, player);
    rig.select(frontend.draft.camera);
    rig.configure(frontend.views[&options.car].clone());
    let session = Session {
        guidance: driving::Guidance::new(&race, player),
        corner_braking: frontend.corner_braking,
        show_racing_line: frontend.show_racing_line,
        show_performance: false,
        camera: rig,
        minimap: hud::Minimap::new(&race.track),
        poses: pose::Poses::new(&race),
        race,
        player,
        controls: default(),
        paused: !quick_race,
        messages: default(),
        delta: default(),
        ready: false,
        screen: if quick_race {
            menu::Screen::Racing
        } else {
            menu::Screen::Main
        },
    };
    let capture_run = options.frames.is_some() || options.benchmark.is_some();
    let mut app = App::new();
    let plugins = DefaultPlugins
        .set(bevy::render::RenderPlugin {
            // Capture runs exit early and must not race background pipeline compilation
            // during driver teardown. Interactive play compiles in the background:
            // blocking a frame on a new shader stalls the race and then lurches it forward.
            synchronous_pipeline_compilation: cfg!(test) || capture_run,
            ..default()
        })
        .set(AssetPlugin {
            file_path: options.assets.to_string_lossy().into_owned(),
            ..default()
        })
        .set(WindowPlugin {
            primary_window: Some(Window {
                title,
                resolution: (options.width, options.height).into(),
                ..default()
            }),
            ..default()
        });
    #[cfg(test)]
    let plugins = plugins.set(bevy::winit::WinitPlugin {
        run_on_any_thread: true,
    });
    if capture_run && options.benchmark.is_none() && std::env::var_os("F1_DETERMINISTIC").is_some()
    {
        // Every frame advances exactly 1/60 s, so the same frame shows the same
        // race state in any build: captures can be compared pixel by pixel.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
    }
    if !capture_run {
        // A paused race in the background only needs to redraw occasionally.
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::reactive_low_power(
                std::time::Duration::from_millis(100),
            ),
        });
    }
    let profile = std::env::var_os("F1_PROFILE").is_some();
    if let Some(run) = benchmark {
        app.insert_resource(run);
    }
    app.add_plugins(plugins)
        .add_audio_source::<audio::Soundscape>()
        .add_plugins(EguiPlugin::default())
        // The wheel LCD has its own egui context; the game camera is marked
        // primary explicitly so the two never trade places.
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .add_plugins(graphics::StaticSkyLighting)
        .add_plugins(wheel_dash::WheelDashPlugin)
        .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default())
        .add_systems(Startup, |mut session: ResMut<Session>| {
            // F1_SHOW_FPS=1 starts with the F3 performance readout visible.
            session.show_performance = std::env::var_os("F1_SHOW_FPS").is_some();
        })
        .insert_resource(Time::<Fixed>::from_hz(PHYSICS_RATE))
        .insert_resource(session)
        .insert_resource(options)
        .insert_resource(frontend)
        .init_resource::<Capture>()
        .add_systems(
            Startup,
            (assets::setup, hud::setup, audio::setup, wheel_dash::setup),
        )
        .add_systems(PreUpdate, controls::update.after(bevy::input::InputSystems))
        .add_systems(Update, pause_on_focus_loss.before(menu::apply))
        .add_systems(FixedUpdate, step_race)
        .add_systems(
            Update,
            (
                menu::apply,
                graphics::apply,
                textures::prepare,
                materials::prepare,
                assets::check_loading,
                benchmark::advance,
                assets::rig_wheel_shells,
                assets::sync_cars,
                wheel_dash::attach,
                wheel_dash::lights,
                camera::update,
                audio::update,
                driving::racing_line,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, (hud::update, menu::ui).chain())
        .add_systems(wheel_dash::WheelDashPass, wheel_dash::draw)
        .add_systems(Last, capture);
    if profile {
        // F1_PROFILE=1 logs per-pass CPU/GPU render timings every few seconds.
        app.add_plugins((
            bevy::render::diagnostic::RenderDiagnosticsPlugin,
            bevy::diagnostic::LogDiagnosticsPlugin {
                wait_duration: std::time::Duration::from_secs(5),
                ..default()
            },
        ));
    }
    Ok(app)
}

/// Like the F1 games, alt-tabbing out of a live race opens the pause menu.
fn pause_on_focus_loss(
    mut focus: MessageReader<bevy::window::WindowFocused>,
    options: Res<Options>,
    mut session: ResMut<Session>,
) {
    for event in focus.read() {
        if !event.focused
            && options.frames.is_none()
            && options.benchmark.is_none()
            && session.screen == menu::Screen::Racing
        {
            session.screen = menu::Screen::Paused;
            session.paused = true;
        }
    }
}

fn step_race(mut session: ResMut<Session>, benchmark: Option<Res<benchmark::Run>>) {
    if benchmark.is_some() {
        return;
    }
    let Session { poses, race, .. } = &mut *session;
    poses.record(race);
    if session.paused || !session.ready {
        return;
    }
    let input = session.controls.car_input();
    let Session {
        race,
        player,
        guidance,
        corner_braking,
        ..
    } = &mut *session;
    let input = guidance.apply(
        &race.entrants[*player].car,
        &race.track,
        input,
        *corner_braking,
        1. / PHYSICS_RATE,
    );
    race.step(&[input]);
}

#[derive(Resource, Default)]
struct Capture {
    frames: u64,
    requested: bool,
}

fn capture(
    mut commands: Commands,
    options: Res<Options>,
    session: Res<Session>,
    mut state: ResMut<Capture>,
    mut exit: MessageWriter<AppExit>,
    mut frame_count: ResMut<bevy::diagnostic::FrameCount>,
) {
    if !session.ready || state.requested {
        return;
    }
    state.frames += 1;
    if state.frames == 1 && std::env::var_os("F1_DETERMINISTIC").is_some() {
        // Temporal jitter and noise follow the frame count; loading takes a
        // variable number of frames, so restart it with the race.
        frame_count.0 = 0;
    }
    if options.frames.is_some_and(|limit| state.frames >= limit) {
        state.requested = true;
        if let Some(path) = options.capture.clone() {
            commands.spawn(Screenshot::primary_window()).observe(
                move |event: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    let result = event
                        .image
                        .clone()
                        .try_into_dynamic()
                        .map_err(|error| error.to_string())
                        .and_then(|image| {
                            image
                                .to_rgb8()
                                .save(&path)
                                .map_err(|error| error.to_string())
                        });
                    match result {
                        Ok(()) => {
                            info!("Screenshot saved to {}", path.display());
                            exit.write(AppExit::Success);
                        }
                        Err(error) => {
                            error!("Screenshot {}: {error}", path.display());
                            exit.write(AppExit::error());
                        }
                    }
                },
            );
        } else {
            exit.write(AppExit::Success);
        }
    }
}
