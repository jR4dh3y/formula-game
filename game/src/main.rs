//! F1-inspired racecraft game client: renders the shared simulation with Dhool.

mod assets;
mod camera;
mod controls;
mod hud;
mod options;
mod sky;

use dhool_pro::{run_live_game, AppExit, GameEngine, LiveOptions, ProjectSettings, SystemContext};
use options::Options;
use racecraft_sim::{Race, PHYSICS_RATE};
use std::process::ExitCode;

/// Everything the client keeps between frames.
pub struct Session {
    pub race: Race,
    pub player: usize,
    pub cars: Vec<assets::CarEntities>,
    pub camera: camera::ChaseCamera,
    pub controls: controls::Controls,
    pub paused: bool,
    pub minimap: hud::Minimap,
    pub messages: hud::Messages,
}

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{}", Options::USAGE);
            return ExitCode::FAILURE;
        }
    };
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> Result<(), String> {
    let loaded = assets::load(&options)?;
    let assets::Loaded {
        scene,
        race,
        player,
        car_meshes,
        track_name,
    } = loaded;

    let (hud, hud_style) = hud::create()?;
    let mut game = GameEngine::builder(ProjectSettings {
        name: "F1 Racecraft".to_string(),
        fixed_timestep: 1.0 / PHYSICS_RATE as f32,
        ..ProjectSettings::default()
    })
    .scene(scene)
    .resource(hud)
    .resource(hud_style)
    .fixed_system("race", |context: &mut SystemContext<'_>| {
        let session = context
            .resources
            .get_mut::<Session>()
            .ok_or("missing session")?;
        if session.paused {
            return Ok(());
        }
        let input = session.controls.car_input();
        session.race.step(&[input]);
        Ok(())
    })
    .update_system("controls", |context: &mut SystemContext<'_>| {
        let session = context
            .resources
            .get_mut::<Session>()
            .ok_or("missing session")?;
        let actions = session
            .controls
            .update(&context.input, context.time.delta_seconds);
        if actions.quit {
            context.events.send(AppExit);
        }
        if actions.pause {
            session.paused = !session.paused;
        }
        if actions.camera {
            session.camera.cycle();
        }
        if actions.reset {
            session.race.recover(session.player);
        }
        Ok(())
    })
    .update_system("sync-cars", |context: &mut SystemContext<'_>| {
        let session = context
            .resources
            .get_mut::<Session>()
            .ok_or("missing session")?;
        assets::sync_cars(context.world, session);
        session.camera.update(
            context.world,
            &session.race,
            session.player,
            context.time.delta_seconds,
        );
        Ok(())
    })
    .update_system("hud", |context: &mut SystemContext<'_>| {
        let style = context
            .resources
            .get::<hud::HudStyle>()
            .copied()
            .ok_or("missing HUD style")?;
        // The HUD and the session are separate resources; take the canvas out
        // while drawing so both can be borrowed.
        let mut hud = context
            .resources
            .remove::<dhool_pro::Hud>()
            .ok_or("missing HUD")?;
        let session = context
            .resources
            .get_mut::<Session>()
            .ok_or("missing session")?;
        hud::draw(&mut hud, &style, session, context.time.delta_seconds);
        context.resources.insert(hud);
        Ok(())
    })
    .build()
    .map_err(|error| error.to_string())?;

    let cars = assets::spawn_cars(&mut game.world, &race, &car_meshes);
    let minimap = hud::Minimap::new(&race.track);
    game.resources.insert(Session {
        camera: camera::ChaseCamera::new(&race, player),
        race,
        player,
        cars,
        controls: controls::Controls::default(),
        paused: false,
        minimap,
        messages: hud::Messages::default(),
    });

    run_live_game(
        game,
        LiveOptions {
            title: format!("F1 Racecraft — {track_name}"),
            width: options.width,
            height: options.height,
            max_frames: options.frames,
            capture_path: options.capture.clone(),
            shadow_distance: 90.0,
            ..LiveOptions::default()
        },
    )
    .map_err(|error| error.to_string())
}
