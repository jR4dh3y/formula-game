//! Bevy client for the deterministic racecraft simulation.

mod assets;
mod camera;
mod canvas;
mod controls;
mod hud;
mod options;
mod sky;
#[cfg(test)]
mod tests;

use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
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
    pub ready: bool,
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

fn run(options: Options) -> Result<AppExit, String> {
    Ok(build_app(options)?.run())
}

fn build_app(mut options: Options) -> Result<App, String> {
    options.assets = options
        .assets
        .canonicalize()
        .map_err(|error| format!("asset directory {}: {error}", options.assets.display()))?;
    let race = assets::load_race(&options)?;
    let player = options.grid_slot - 1;
    let title = format!("F1 Racecraft - {}", race.track.name);
    let session = Session {
        camera: camera::ChaseCamera::new(&race, player),
        minimap: hud::Minimap::new(&race.track),
        race,
        player,
        controls: default(),
        paused: false,
        messages: default(),
        ready: false,
    };
    let mut app = App::new();
    let plugins = DefaultPlugins
        .set(bevy::render::RenderPlugin {
            // Early exits must not race background pipeline compilation during driver teardown.
            synchronous_pipeline_compilation: true,
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
    app.add_plugins(plugins)
        .add_plugins(EguiPlugin::default())
        .insert_resource(Time::<Fixed>::from_hz(PHYSICS_RATE))
        .insert_resource(session)
        .insert_resource(options)
        .init_resource::<Capture>()
        .add_systems(Startup, (assets::setup, hud::setup))
        .add_systems(PreUpdate, controls::update.after(bevy::input::InputSystems))
        .add_systems(FixedUpdate, step_race)
        .add_systems(
            Update,
            (assets::check_loading, assets::sync_cars, camera::update).chain(),
        )
        .add_systems(EguiPrimaryContextPass, hud::update)
        .add_systems(Last, capture);
    Ok(app)
}

fn step_race(mut session: ResMut<Session>) {
    if session.paused || !session.ready {
        return;
    }
    let input = session.controls.car_input();
    session.race.step(&[input]);
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
) {
    if !session.ready || state.requested {
        return;
    }
    state.frames += 1;
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
