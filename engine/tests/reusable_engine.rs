use dhool_pro::math::{Color, Vec2, Vec3};
use dhool_pro::{
    EngineBuildError, EngineError, FlyCameraPlugin, FrameInput, GameEngine, InputState, Light,
    Material, Mesh, ProjectSettings, RenderConfig, SceneBuilder,
};

#[derive(Debug, Default, PartialEq, Eq)]
struct GameState {
    startup_runs: u32,
    fixed_runs: u32,
    fixed_events: u32,
    update_order: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FixedStep;

fn custom_scene() -> dhool_pro::RenderScene {
    let mut scene = SceneBuilder::new();
    let cube = scene.add_material(Material::standard("cube", Color::rgb(0.75, 0.18, 0.12)));
    let ground = scene.add_material(Material::standard("ground", Color::rgb(0.18, 0.22, 0.16)));
    scene.add_mesh(Mesh::cuboid("player", Vec3::ONE), cube);
    scene.add_mesh(Mesh::ground_plane("ground", Vec2::new(12.0, 12.0)), ground);
    scene.add_light(Light::directional(
        "sun",
        Vec3::new(0.3, -1.0, -0.4),
        Color::WHITE,
        2.5,
    ));
    scene.build().expect("consumer-owned scene")
}

#[test]
fn downstream_game_owns_scene_state_and_systems() {
    let mut engine = GameEngine::builder(ProjectSettings {
        name: "Consumer Game".to_string(),
        native_width: 96,
        native_height: 64,
        ..ProjectSettings::default()
    })
    .scene(custom_scene())
    .render_config(RenderConfig::tiny_test())
    .resource(GameState::default())
    .startup_system(
        "initialize-game",
        |context: &mut dhool_pro::SystemContext<'_>| {
            context
                .resources
                .get_mut::<GameState>()
                .expect("game state")
                .startup_runs += 1;
            Ok(())
        },
    )
    .fixed_system(
        "rotate-player",
        |context: &mut dhool_pro::SystemContext<'_>| {
            let player = context.world.find_by_name("player").expect("player");
            context
                .world
                .transform_mut(player)
                .expect("transform")
                .rotation
                .y += context.time.delta_seconds;
            context
                .resources
                .get_mut::<GameState>()
                .expect("game state")
                .fixed_runs += 1;
            context.events.send(FixedStep);
            Ok(())
        },
    )
    .update_system("first", |context: &mut dhool_pro::SystemContext<'_>| {
        context
            .resources
            .get_mut::<GameState>()
            .expect("game state")
            .fixed_events += context.events.drain::<FixedStep>().len() as u32;
        context
            .resources
            .get_mut::<GameState>()
            .expect("game state")
            .update_order
            .push("first");
        Ok(())
    })
    .update_system("second", |context: &mut dhool_pro::SystemContext<'_>| {
        context
            .resources
            .get_mut::<GameState>()
            .expect("game state")
            .update_order
            .push("second");
        Ok(())
    })
    .build()
    .expect("engine application");

    let frame = engine
        .frame(FrameInput {
            delta_seconds: 1.0 / 30.0,
            input: InputState::default(),
        })
        .expect("consumer frame");
    let state = engine.resources.get::<GameState>().expect("game state");

    assert_eq!(state.startup_runs, 1);
    assert_eq!(state.fixed_runs, 2);
    assert_eq!(state.fixed_events, 2);
    assert_eq!(state.update_order, ["first", "second"]);
    assert_eq!(frame.fixed_ticks, 2);
    assert_eq!(frame.time.fixed_tick, 2);
    assert_eq!(frame.render_output.width, 96);
    assert_eq!(frame.render_output.height, 64);
    assert!(frame.render_output.average_luminance() > 0.0);
    assert!(frame.world_issues.is_empty());
}

#[test]
fn gameplay_can_advance_without_running_the_cpu_renderer() {
    let mut engine = GameEngine::builder(ProjectSettings::default())
        .scene(custom_scene())
        .resource(0_u32)
        .update_system(
            "count-updates",
            |context: &mut dhool_pro::SystemContext<'_>| {
                *context.resources.get_mut::<u32>().expect("counter") += 1;
                Ok(())
            },
        )
        .build()
        .expect("engine application");

    let update = engine
        .advance(FrameInput {
            delta_seconds: 1.0 / 60.0,
            input: InputState::default(),
        })
        .expect("gameplay update");

    assert_eq!(update.frame_index, 0);
    assert_eq!(update.fixed_ticks, 1);
    assert_eq!(engine.resources.get::<u32>(), Some(&1));
}

#[test]
fn fly_camera_is_opt_in_and_persists_world_state() {
    let mut engine = GameEngine::builder(ProjectSettings {
        native_width: 64,
        native_height: 48,
        ..ProjectSettings::default()
    })
    .scene(custom_scene())
    .render_config(RenderConfig::tiny_test())
    .plugin(FlyCameraPlugin::default())
    .build()
    .expect("engine application");
    let original = engine
        .world
        .active_camera()
        .expect("camera")
        .1
        .camera
        .position;

    engine
        .frame(FrameInput {
            delta_seconds: 0.1,
            input: InputState {
                move_axis: Vec3::new(0.0, 0.0, 1.0),
                ..InputState::default()
            },
        })
        .expect("camera frame");

    let moved = engine
        .world
        .active_camera()
        .expect("camera")
        .1
        .camera
        .position;
    assert_ne!(moved, original);
}

#[test]
fn builder_rejects_invalid_projects_and_names_failed_systems() {
    let missing = GameEngine::builder(ProjectSettings::default()).build();
    assert!(matches!(missing, Err(EngineBuildError::MissingScene)));

    let mut engine = GameEngine::builder(ProjectSettings {
        native_width: 64,
        native_height: 48,
        ..ProjectSettings::default()
    })
    .scene(custom_scene())
    .render_config(RenderConfig::tiny_test())
    .update_system("broken-gameplay", |_: &mut dhool_pro::SystemContext<'_>| {
        Err("intentional failure".to_string())
    })
    .build()
    .expect("startup succeeds");

    let error = engine
        .frame(FrameInput::default())
        .expect_err("system fails");
    assert!(matches!(error, EngineError::System(_)));
    assert!(error.to_string().contains("broken-gameplay"));
    assert!(error.to_string().contains("intentional failure"));
}

#[test]
fn builder_rejects_scene_contract_errors_before_startup() {
    let mut scene = custom_scene();
    scene.materials[0].specular_intensity = 1.0;
    let error = match GameEngine::builder(ProjectSettings::default())
        .scene(scene)
        .build()
    {
        Ok(_) => panic!("invalid scene should not build"),
        Err(error) => error,
    };

    assert!(matches!(error, EngineBuildError::InvalidScene { .. }));
    assert!(error.to_string().contains("reflection proxy"));
}

#[test]
fn world_physics_has_no_hidden_demo_floor() {
    let mut world = dhool_pro::World::from_render_scene(custom_scene());
    let entity = world.spawn(
        "falling",
        dhool_pro::Transform::from_position(Vec3::new(0.0, -1.0, 0.0)),
    );
    assert!(world.set_rigid_body(
        entity,
        dhool_pro::RigidBody {
            velocity: Vec3::ZERO,
            mass: 1.0,
            gravity_scale: 1.0,
            dynamic: true,
        },
    ));

    world.fixed_update(1.0 / 60.0).expect("physics step");
    assert!(world.transform(entity).expect("transform").position.y < -1.0);

    world.physics.floor_height = Some(0.0);
    world.fixed_update(1.0 / 60.0).expect("floor step");
    assert_eq!(world.transform(entity).expect("transform").position.y, 0.0);
}
