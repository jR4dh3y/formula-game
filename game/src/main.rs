//! Bevy client for the deterministic racecraft simulation.

mod assets;
mod audio;
mod benchmark;
mod camera;
mod canvas;
mod controls;
mod drift;
mod driving;
mod effects;
mod graphics;
mod hud;
mod materials;
mod menu;
mod meshes;
mod mirrors;
mod online;
mod options;
mod pose;
mod preferences;
mod race_loop;
mod road;
mod script;
mod sky;
mod teams;
#[cfg(test)]
mod tests;
mod textures;
#[cfg(test)]
mod track_probe;
#[cfg(test)]
mod visual_review;
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
    pub identities: Vec<Option<teams::Identity>>,
    pub rig: teams::Rig,
    pub player: usize,
    pub camera: camera::ChaseCamera,
    pub controls: controls::Controls,
    pub paused: bool,
    pub minimap: hud::Minimap,
    pub messages: hud::Messages,
    pub delta: hud::Delta,
    pub telemetry: hud::Telemetry,
    pub ready: bool,
    pub screen: menu::Screen,
    pub guidance: driving::Guidance,
    pub corner_braking: f64,
    pub show_racing_line: bool,
    pub show_performance: bool,
    pub poses: pose::Poses,
    /// Round-trip time per entrant slot in an online race; empty offline.
    pub pings: Vec<Option<u16>>,
    /// The player's recent collision impulse (N·s), held and decayed on
    /// physics ticks. The car reports an impact only on the tick it happens,
    /// and a frame spans several ticks: reading the car each frame misses
    /// most collisions.
    pub impact: f64,
    /// Strongest collision since the audio update, consumed once per frame.
    pub pending_impact: f64,
    /// Collisions of any car since the effects last drew, for their sparks.
    pub contacts: Vec<effects::Contact>,
}

/// Seconds for a collision's camera jolt to fade to about a third.
const IMPACT_FADE: f64 = 0.12;

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
    if (options.host.is_some() || options.join.is_some())
        && options.car == "gen22"
        && !options.assets.join("cars/gen22/car.json").is_file()
    {
        options.car = "rb6".into();
    }
    let teams = if options.car == "gen22" {
        teams::Teams::load(&options.assets)?
    } else {
        teams::Teams::load(&options.assets).unwrap_or_default()
    };
    if options.car == "gen22" {
        teams.selection(&options.team, &options.driver)?;
    }
    let mut race = assets::load_race(&options)?;
    let benchmark = if options.benchmark.is_some() {
        let (planned_race, run) = benchmark::Run::plan(&options, &race)?;
        race = planned_race;
        Some(run)
    } else {
        None
    };
    let player = options.grid_slot - 1;
    let title = if options.host.is_some() || options.join.is_some() {
        format!("F1 Racecraft - {} - {}", race.track.name, options.name)
    } else {
        format!("F1 Racecraft - {}", race.track.name)
    };
    let quick_race = options.quick_race || options.frames.is_some();
    let frontend = menu::Frontend::new(&options);
    let mut rig = camera::ChaseCamera::new(&race, player);
    rig.select(frontend.draft.camera);
    rig.configure(frontend.views[&options.car].clone());
    rig.set_cockpit_eye(teams::Rig::load(&options)?.cockpit_eye);
    let session = Session {
        identities: teams.field(&options)?,
        rig: teams::Rig::load(&options)?,
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
        telemetry: default(),
        ready: false,
        pings: Vec::new(),
        impact: 0.,
        pending_impact: 0.,
        contacts: Vec::new(),
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
                // A capture must be the size asked for: a fixed-size window is
                // left alone by tiling compositors, which would otherwise
                // stretch it to a column.
                resizable: !capture_run,
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
    if capture_run {
        // Benchmarks and captures must not inherit Bevy's unfocused 60 Hz cap.
        app.insert_resource(bevy::winit::WinitSettings::continuous());
    } else {
        // A paused race in the background only needs to redraw occasionally.
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: if options.test_driver {
                bevy::winit::UpdateMode::Continuous
            } else {
                bevy::winit::UpdateMode::reactive_low_power(std::time::Duration::from_millis(100))
            },
        });
    }
    let profile = std::env::var_os("F1_PROFILE").is_some();
    if let Some(run) = benchmark {
        app.insert_resource(run);
    }
    if let Some(script) = script::Script::from_env().filter(|_| capture_run) {
        app.insert_resource(script);
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
        .add_plugins(effects::EffectsPlugin)
        .add_plugins(mirrors::MirrorPlugin)
        .add_systems(Startup, road::setup)
        .add_plugins(graphics::StaticSkyLighting)
        .add_plugins(wheel_dash::WheelDashPlugin)
        .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default())
        .add_systems(Startup, |mut session: ResMut<Session>| {
            // F1_SHOW_FPS=1 starts with the F3 performance readout visible.
            session.show_performance = std::env::var_os("F1_SHOW_FPS").is_some();
        })
        .insert_resource(Time::<Fixed>::from_hz(PHYSICS_RATE))
        .insert_resource(teams)
        .insert_resource(session)
        .insert_resource(online::Online::new(&options))
        .insert_resource(options)
        .insert_resource(frontend)
        .init_resource::<Capture>()
        .init_resource::<textures::TextureDetail>()
        .init_resource::<textures::TextureJobs>()
        .init_resource::<drift::DriftScore>()
        .init_resource::<meshes::MeshJobs>()
        .init_resource::<assets::OpponentLodDistance>()
        .add_systems(
            Startup,
            (
                assets::setup,
                hud::setup,
                audio::setup,
                wheel_dash::setup,
                menu::car_preview::setup,
            ),
        )
        .add_systems(PreUpdate, controls::update.after(bevy::input::InputSystems))
        .add_systems(Update, drift::update)
        .add_systems(
            Update,
            assets::spawn_scene.run_if(assets::launch_screen_shown),
        )
        .add_systems(Update, pause_on_focus_loss.before(menu::apply))
        .init_resource::<race_loop::Run>()
        .add_systems(
            Update,
            race_loop::update
                .before(assets::sync_cars)
                .after(menu::apply),
        )
        .add_systems(FixedUpdate, step_race)
        .add_systems(
            Update,
            (
                online::poll,
                menu::apply,
                online::install,
                graphics::apply,
                assets::update_opponent_lods.after(graphics::apply),
                materials::prepare,
                assets::check_loading.after(materials::prepare),
                benchmark::advance,
                assets::sync_cars,
                assets::animate_controls,
                wheel_dash::attach,
                wheel_dash::lights,
                sky::update,
                camera::update,
                effects::update,
                graphics::speed_blur,
                menu::car_preview::update,
                assets::helmet_visibility,
                audio::update,
                driving::racing_line,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (road::prepare, road::rubber_line, materials::finishes)
                .chain()
                .after(materials::prepare),
        )
        .add_systems(
            EguiPrimaryContextPass,
            (hud::update, menu::ui, race_loop::ui, drift::ui).chain(),
        )
        .add_systems(wheel_dash::WheelDashPass, wheel_dash::draw)
        .add_systems(
            PostUpdate,
            (textures::prepare, meshes::prepare).after(bevy::asset::AssetEventSystems),
        )
        .add_systems(Last, capture);
    textures::register_normal_scales(&mut app);
    if !(cfg!(test) || capture_run) {
        // A scene's meshes and textures reach the GPU over a few frames rather
        // than stalling one. Capture runs count frames and need them at once.
        app.insert_resource(bevy::render::render_asset::RenderAssetBytesPerFrame::new(
            8 << 20,
        ));
    }
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
            && !options.test_driver
            && options.frames.is_none()
            && options.benchmark.is_none()
            && session.screen == menu::Screen::Racing
        {
            session.screen = menu::Screen::Paused;
            session.paused = true;
        }
    }
}

fn step_race(
    mut rewards: Option<ResMut<race_loop::Run>>,
    options: Option<Res<Options>>,
    frontend: Option<Res<menu::Frontend>>,
    mut session: ResMut<Session>,
    benchmark: Option<Res<benchmark::Run>>,
    mut online: Option<ResMut<online::Online>>,
    mut script: Option<ResMut<script::Script>>,
) {
    if benchmark.is_some() {
        return;
    }
    if !online.as_ref().is_some_and(|o| o.client.is_some())
        && rewards.as_ref().is_some_and(|r| r.resuming())
    {
        return;
    }
    let Session { poses, race, .. } = &mut *session;
    poses.record(race);
    let connected = online.as_ref().is_some_and(|o| o.client.is_some());
    if (!connected && session.paused) || !session.ready {
        return;
    }
    let scripted = online.as_mut().and_then(|o| o.pilot.as_mut()).map(|pilot| {
        let race = &session.race;
        let car = &race.entrants[session.player].car;
        let rivals: Vec<_> = race
            .entrants
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != session.player)
            .filter_map(|(_, e)| {
                let gap = race
                    .track
                    .delta(car.track_position.s, e.car.track_position.s);
                (gap.abs() < 120.).then_some(racecraft_sim::ai::Rival {
                    gap,
                    lateral: e.car.track_position.lateral,
                    speed: e.car.speed(),
                    position: glam::DVec2::new(e.car.position.x, e.car.position.z),
                })
            })
            .collect();
        pilot.drive(car, &race.track, &rivals, 1. / PHYSICS_RATE)
    });
    let staged = if scripted.is_none() && !connected {
        script
            .as_mut()
            .and_then(|script| script.input(&session.race, session.player))
    } else {
        None
    };
    let scripted = scripted.or(staged.and_then(Result::ok));
    let input = if let Some(input) = scripted {
        input
    } else if let Some(Err(input)) = staged {
        input
    } else if connected && session.screen != menu::Screen::Racing {
        racecraft_sim::CarInput {
            brake: 0.3,
            ..default()
        }
    } else {
        session.controls.car_input()
    };
    let Session {
        race,
        player,
        guidance,
        corner_braking,
        ..
    } = &mut *session;
    let input = if scripted.is_some() {
        input
    } else {
        let rivals = race.rivals_of(*player);
        guidance.apply(
            &race.entrants[*player].car,
            &race.track,
            &rivals,
            input,
            *corner_braking,
            1. / PHYSICS_RATE,
        )
    };
    if let Some(client) = online.as_mut().and_then(|o| o.client.as_mut()) {
        if let Err(error) = client.predict(race, input) {
            client.error = Some(error);
        }
    } else {
        race.step(&[input]);
    }
    let Session { race, contacts, .. } = &mut *session;
    for entrant in &race.entrants {
        let car = &entrant.car;
        if car.impact > 0. && contacts.len() < 32 {
            contacts.push(effects::Contact {
                point: Vec3::new(
                    car.impact_point.x as f32,
                    car.position.y as f32 + 0.25,
                    car.impact_point.y as f32,
                ),
                velocity: Vec3::new(car.velocity.x as f32, 0., car.velocity.y as f32),
                impulse: car.impact as f32,
            });
        }
    }
    let hit = session.race.entrants[session.player].car.impact;
    session.pending_impact = session.pending_impact.max(hit);
    session.impact = (session.impact * (-1. / (PHYSICS_RATE * IMPACT_FADE)).exp()).max(hit);
    session.sample_hud();
    if !connected {
        if let (Some(rewards), Some(options), Some(frontend)) =
            (rewards.as_mut(), options, frontend)
        {
            rewards.sample(&session, &options, frontend.rewind);
        }
    }
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
