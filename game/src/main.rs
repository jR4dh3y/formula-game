//! Bevy client for the deterministic racecraft simulation.

mod assets;
mod audio;
mod camera;
mod canvas;
mod controls;
mod driving;
mod hud;
mod materials;
mod menu;
mod options;
mod preferences;
mod sky;
#[cfg(test)]
mod tests;
mod textures;

use bevy::{
    app::AppExit,
    audio::AddAudioSource,
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
    pub screen: menu::Screen,
    pub guidance: driving::Guidance,
    pub corner_braking: f64,
    pub show_racing_line: bool,
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
    let quick_race = options.quick_race || options.frames.is_some();
    let frontend = menu::Frontend::new(&options);
    let mut rig = camera::ChaseCamera::new(&race, player);
    rig.select(frontend.draft.camera);
    rig.configure(frontend.views[&options.car].clone());
    let session = Session {
        guidance: driving::Guidance::new(&race, player),
        corner_braking: frontend.corner_braking,
        show_racing_line: frontend.show_racing_line,
        camera: rig,
        minimap: hud::Minimap::new(&race.track),
        race,
        player,
        controls: default(),
        paused: !quick_race,
        messages: default(),
        ready: false,
        screen: if quick_race {
            menu::Screen::Racing
        } else {
            menu::Screen::Main
        },
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
        .add_audio_source::<audio::EngineSound>()
        .add_plugins(EguiPlugin::default())
        .insert_resource(Time::<Fixed>::from_hz(PHYSICS_RATE))
        .insert_resource(session)
        .insert_resource(options)
        .insert_resource(frontend)
        .init_resource::<Capture>()
        .add_systems(Startup, (assets::setup, hud::setup, audio::setup))
        .add_systems(PreUpdate, controls::update.after(bevy::input::InputSystems))
        .add_systems(FixedUpdate, step_race)
        .add_systems(
            Update,
            (
                menu::apply,
                textures::prepare,
                materials::prepare,
                assets::check_loading,
                assets::sync_cars,
                camera::update,
                audio::update,
                driving::racing_line,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, (hud::update, menu::ui).chain())
        .add_systems(Last, capture);
    Ok(app)
}

fn step_race(mut session: ResMut<Session>) {
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
