use crate::asset_pipeline::{AssetPipeline, ImportReport};
use crate::cloth::ClothError;
use crate::input::{GamepadState, KeySet};
use crate::math::{Vec2, Vec3};
use crate::profiler::{EngineProfiler, FrameReport};
use crate::quality::QualityTier;
use crate::renderer::{CpuRenderer, RenderConfig, RenderError, RenderOutput};
use crate::runtime::{Events, GameSystem, NamedSystem, Resources, Schedule, SystemFailure, Time};
use crate::scene::{Camera, RenderScene};
use crate::world::{World, WorldIssue};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetPlatform {
    Windows,
    Linux,
    MacOs,
    Android,
    Ios,
    Web,
    Other,
}

impl TargetPlatform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            "macos" => Self::MacOs,
            "android" => Self::Android,
            "ios" => Self::Ios,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectSettings {
    pub name: String,
    pub target_platforms: Vec<TargetPlatform>,
    pub quality: QualityTier,
    pub fixed_timestep: f32,
    pub native_width: u32,
    pub native_height: u32,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            name: "Dhool Game".to_string(),
            target_platforms: vec![TargetPlatform::current()],
            quality: QualityTier::High,
            fixed_timestep: 1.0 / 60.0,
            native_width: 1920,
            native_height: 1080,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputState {
    pub move_axis: Vec3,
    pub look_delta: Vec2,
    pub primary_action: bool,
    pub secondary_action: bool,
    /// Keys currently held.
    pub keys: KeySet,
    /// Keys that went down since the previous frame. Every fixed tick of that
    /// frame sees the same set, so act on presses in update systems.
    pub keys_pressed: KeySet,
    pub gamepad: GamepadState,
}

/// Send this event from any system to close the live window after the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppExit;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameInput {
    pub delta_seconds: f32,
    pub input: InputState,
}

impl Default for FrameInput {
    fn default() -> Self {
        Self {
            delta_seconds: 1.0 / 60.0,
            input: InputState::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EngineFrame {
    pub frame_index: u64,
    pub fixed_ticks: u32,
    pub time: Time,
    pub render_output: RenderOutput,
    pub frame_report: FrameReport,
    pub world_issues: Vec<WorldIssue>,
    pub import_report: ImportReport,
}

#[derive(Clone, Debug)]
pub struct EngineUpdate {
    pub frame_index: u64,
    pub fixed_ticks: u32,
    pub time: Time,
    pub world_issues: Vec<WorldIssue>,
    pub import_report: ImportReport,
}

impl EngineFrame {
    pub fn passed_production_gates(&self) -> bool {
        self.world_issues.is_empty()
            && !self.import_report.has_errors()
            && self.frame_report.passed()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EngineBuildError {
    MissingScene,
    InvalidProjectName,
    InvalidFixedTimestep(f32),
    InvalidResolution { width: u32, height: u32 },
    InvalidScene { messages: Vec<String> },
    System(SystemFailure),
}

impl fmt::Display for EngineBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingScene => write!(formatter, "an engine application needs a startup scene"),
            Self::InvalidProjectName => write!(formatter, "project name cannot be empty"),
            Self::InvalidFixedTimestep(value) => {
                write!(
                    formatter,
                    "fixed timestep must be finite and positive, got {value}"
                )
            }
            Self::InvalidResolution { width, height } => {
                write!(
                    formatter,
                    "render resolution must be non-zero, got {width}x{height}"
                )
            }
            Self::InvalidScene { messages } => {
                write!(
                    formatter,
                    "startup scene is invalid: {}",
                    messages.join("; ")
                )
            }
            Self::System(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EngineBuildError {}

#[derive(Debug)]
pub enum EngineError {
    Render(RenderError),
    Cloth(ClothError),
    System(SystemFailure),
    NoSceneLoaded,
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render(error) => write!(formatter, "render failed: {error:?}"),
            Self::Cloth(error) => write!(formatter, "cloth simulation failed: {error}"),
            Self::System(error) => error.fmt(formatter),
            Self::NoSceneLoaded => write!(formatter, "no scene is loaded"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<RenderError> for EngineError {
    fn from(value: RenderError) -> Self {
        Self::Render(value)
    }
}

impl From<ClothError> for EngineError {
    fn from(value: ClothError) -> Self {
        Self::Cloth(value)
    }
}

impl From<SystemFailure> for EngineError {
    fn from(value: SystemFailure) -> Self {
        Self::System(value)
    }
}

pub trait EnginePlugin {
    fn build(&self, builder: &mut GameEngineBuilder);
}

pub struct GameEngineBuilder {
    settings: ProjectSettings,
    scene: Option<RenderScene>,
    render_config: Option<RenderConfig>,
    resources: Resources,
    events: Events,
    startup_systems: Vec<NamedSystem>,
    fixed_systems: Vec<NamedSystem>,
    update_systems: Vec<NamedSystem>,
}

impl GameEngineBuilder {
    pub fn new(settings: ProjectSettings) -> Self {
        Self {
            settings,
            scene: None,
            render_config: None,
            resources: Resources::default(),
            events: Events::default(),
            startup_systems: Vec::new(),
            fixed_systems: Vec::new(),
            update_systems: Vec::new(),
        }
    }

    pub fn scene(mut self, scene: RenderScene) -> Self {
        self.scene = Some(scene);
        self
    }

    pub fn render_config(mut self, config: RenderConfig) -> Self {
        self.render_config = Some(config);
        self
    }

    pub fn resource<T>(mut self, resource: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.register_resource(resource);
        self
    }

    pub fn register_resource<T>(&mut self, resource: T)
    where
        T: Send + Sync + 'static,
    {
        self.resources.insert(resource);
    }

    pub fn plugin(mut self, plugin: impl EnginePlugin) -> Self {
        plugin.build(&mut self);
        self
    }

    pub fn startup_system(
        mut self,
        name: impl Into<String>,
        system: impl GameSystem + 'static,
    ) -> Self {
        self.register_startup_system(name, system);
        self
    }

    pub fn fixed_system(
        mut self,
        name: impl Into<String>,
        system: impl GameSystem + 'static,
    ) -> Self {
        self.register_fixed_system(name, system);
        self
    }

    pub fn update_system(
        mut self,
        name: impl Into<String>,
        system: impl GameSystem + 'static,
    ) -> Self {
        self.register_update_system(name, system);
        self
    }

    pub fn register_startup_system(
        &mut self,
        name: impl Into<String>,
        system: impl GameSystem + 'static,
    ) {
        self.startup_systems.push(NamedSystem::new(name, system));
    }

    pub fn register_fixed_system(
        &mut self,
        name: impl Into<String>,
        system: impl GameSystem + 'static,
    ) {
        self.fixed_systems.push(NamedSystem::new(name, system));
    }

    pub fn register_update_system(
        &mut self,
        name: impl Into<String>,
        system: impl GameSystem + 'static,
    ) {
        self.update_systems.push(NamedSystem::new(name, system));
    }

    pub fn build(self) -> Result<GameEngine, EngineBuildError> {
        validate_settings(&self.settings)?;
        let scene = self.scene.ok_or(EngineBuildError::MissingScene)?;
        let render_config = self.render_config.unwrap_or(RenderConfig {
            width: self.settings.native_width,
            height: self.settings.native_height,
            quality: self.settings.quality,
            ..RenderConfig::headless_1080p()
        });
        let (assets, import_report) =
            AssetPipeline::build_from_scene(&scene, self.settings.quality);
        if import_report.has_errors() {
            return Err(EngineBuildError::InvalidScene {
                messages: import_report
                    .errors()
                    .map(|message| message.text.clone())
                    .collect(),
            });
        }
        let mut engine = GameEngine {
            settings: self.settings,
            world: World::from_render_scene(scene),
            assets,
            profiler: EngineProfiler::default(),
            resources: self.resources,
            events: self.events,
            renderer: CpuRenderer::new(render_config),
            startup_systems: self.startup_systems,
            fixed_systems: self.fixed_systems,
            update_systems: self.update_systems,
            accumulator: 0.0,
            time: Time::default(),
            scene_loaded: true,
            started: false,
        };
        engine.start().map_err(EngineBuildError::System)?;
        Ok(engine)
    }
}

pub struct GameEngine {
    pub settings: ProjectSettings,
    pub world: World,
    pub assets: AssetPipeline,
    pub profiler: EngineProfiler,
    pub resources: Resources,
    pub events: Events,
    renderer: CpuRenderer,
    startup_systems: Vec<NamedSystem>,
    fixed_systems: Vec<NamedSystem>,
    update_systems: Vec<NamedSystem>,
    accumulator: f32,
    time: Time,
    scene_loaded: bool,
    started: bool,
}

impl GameEngine {
    pub fn builder(settings: ProjectSettings) -> GameEngineBuilder {
        GameEngineBuilder::new(settings)
    }

    pub fn new(settings: ProjectSettings) -> Result<Self, EngineBuildError> {
        validate_settings(&settings)?;
        let render_config = RenderConfig {
            width: settings.native_width,
            height: settings.native_height,
            quality: settings.quality,
            ..RenderConfig::headless_1080p()
        };
        Ok(Self {
            assets: AssetPipeline::new(settings.quality),
            profiler: EngineProfiler::default(),
            resources: Resources::default(),
            events: Events::default(),
            world: World::default(),
            renderer: CpuRenderer::new(render_config),
            settings,
            startup_systems: Vec::new(),
            fixed_systems: Vec::new(),
            update_systems: Vec::new(),
            accumulator: 0.0,
            time: Time::default(),
            scene_loaded: false,
            started: false,
        })
    }

    pub fn load_scene(&mut self, scene: RenderScene) -> ImportReport {
        let (assets, report) = AssetPipeline::build_from_scene(&scene, self.settings.quality);
        self.assets = assets;
        self.world = World::from_render_scene(scene);
        self.scene_loaded = true;
        report
    }

    pub fn set_render_config(&mut self, config: RenderConfig) {
        self.renderer = CpuRenderer::new(config);
    }

    pub const fn time(&self) -> Time {
        self.time
    }

    pub fn frame(&mut self, input: FrameInput) -> Result<EngineFrame, EngineError> {
        let update = self.advance(input)?;
        let render_scene = self.world.build_render_scene();
        let render_output = self.renderer.render(&render_scene)?;
        let frame_report = self.profiler.evaluate(&render_output);
        Ok(EngineFrame {
            frame_index: update.frame_index,
            fixed_ticks: update.fixed_ticks,
            time: update.time,
            render_output,
            frame_report,
            world_issues: update.world_issues,
            import_report: update.import_report,
        })
    }

    /// Advances gameplay without invoking the CPU renderer.
    pub fn advance(&mut self, input: FrameInput) -> Result<EngineUpdate, EngineError> {
        if !self.scene_loaded {
            return Err(EngineError::NoSceneLoaded);
        }
        if !self.started {
            self.start()?;
        }

        let delta = if input.delta_seconds.is_finite() {
            input.delta_seconds.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.time.delta_seconds = delta;
        self.time.fixed_delta_seconds = self.settings.fixed_timestep;
        self.time.elapsed_seconds += f64::from(delta);
        self.accumulator += delta;

        let mut fixed_ticks = 0;
        while self.accumulator >= self.settings.fixed_timestep {
            let fixed_time = Time {
                delta_seconds: self.settings.fixed_timestep,
                ..self.time
            };
            run_systems(
                &mut self.fixed_systems,
                &mut self.world,
                &mut self.resources,
                &mut self.events,
                input.input,
                fixed_time,
                Schedule::FixedUpdate,
            )?;
            self.world.fixed_update(self.settings.fixed_timestep)?;
            self.accumulator -= self.settings.fixed_timestep;
            self.time.fixed_tick = self.time.fixed_tick.saturating_add(1);
            fixed_ticks += 1;
        }

        run_systems(
            &mut self.update_systems,
            &mut self.world,
            &mut self.resources,
            &mut self.events,
            input.input,
            self.time,
            Schedule::Update,
        )?;

        let world_issues = self.world.validate();
        let import_report = self.assets.validate();
        let update = EngineUpdate {
            frame_index: self.time.frame_index,
            fixed_ticks,
            time: self.time,
            world_issues,
            import_report,
        };
        self.time.frame_index = self.time.frame_index.saturating_add(1);
        Ok(update)
    }

    fn start(&mut self) -> Result<(), SystemFailure> {
        if self.started || !self.scene_loaded {
            return Ok(());
        }
        run_systems(
            &mut self.startup_systems,
            &mut self.world,
            &mut self.resources,
            &mut self.events,
            InputState::default(),
            self.time,
            Schedule::Startup,
        )?;
        self.started = true;
        Ok(())
    }
}

fn validate_settings(settings: &ProjectSettings) -> Result<(), EngineBuildError> {
    if settings.name.trim().is_empty() {
        return Err(EngineBuildError::InvalidProjectName);
    }
    if !settings.fixed_timestep.is_finite() || settings.fixed_timestep <= 0.0 {
        return Err(EngineBuildError::InvalidFixedTimestep(
            settings.fixed_timestep,
        ));
    }
    if settings.native_width == 0 || settings.native_height == 0 {
        return Err(EngineBuildError::InvalidResolution {
            width: settings.native_width,
            height: settings.native_height,
        });
    }
    Ok(())
}

fn run_systems(
    systems: &mut [NamedSystem],
    world: &mut World,
    resources: &mut Resources,
    events: &mut Events,
    input: InputState,
    time: Time,
    schedule: Schedule,
) -> Result<(), SystemFailure> {
    for system in systems {
        system.run(world, resources, events, input, time, schedule)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyCameraPlugin {
    pub movement_speed: f32,
    pub look_sensitivity: f32,
}

impl Default for FlyCameraPlugin {
    fn default() -> Self {
        Self {
            movement_speed: 4.0,
            look_sensitivity: 0.01,
        }
    }
}

impl EnginePlugin for FlyCameraPlugin {
    fn build(&self, builder: &mut GameEngineBuilder) {
        let settings = *self;
        builder.register_update_system(
            "fly-camera",
            move |context: &mut crate::runtime::SystemContext<'_>| {
                let Some((_, rig)) = context.world.active_camera_mut() else {
                    return Ok(());
                };
                apply_camera_input(
                    &mut rig.camera,
                    context.input,
                    context.time.delta_seconds,
                    settings,
                );
                Ok(())
            },
        );
    }
}

fn apply_camera_input(camera: &mut Camera, input: InputState, dt: f32, settings: FlyCameraPlugin) {
    let forward = (camera.target - camera.position).normalize_or_zero();
    let right = forward.cross(camera.up).normalize_or_zero();
    let up = camera.up.normalize_or_zero();
    let movement = right * input.move_axis.x + up * input.move_axis.y + forward * input.move_axis.z;
    let delta = movement * (dt * settings.movement_speed);
    camera.position += delta;
    camera.target += delta;
    if input.look_delta.x != 0.0 || input.look_delta.y != 0.0 {
        let look = right * (input.look_delta.x * settings.look_sensitivity)
            + up * (input.look_delta.y * settings.look_sensitivity);
        camera.target += look;
    }
}
