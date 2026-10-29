use crate::cloth::{ClothColliderSnapshot, ClothState};
use crate::game::{AppExit, FrameInput, GameEngine, InputState};
use crate::hud::Hud;
use crate::input::{GamepadButton, GamepadButtons, GamepadState, Key, KeySet};
use crate::math::{Vec2, Vec3, Vec4};
use crate::scene::{EnvironmentMap, LightKind, Material, RenderScene, TextureImage};
use crate::world::{EntityId, RenderInstance, World};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3 as GpuVec3};
use half::f16;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wgpu::util::{DeviceExt, TextureDataOrder};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const PBR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const MAIN_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32FloatStencil8;
const SHADOW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const REFLECTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const SMAA_EDGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
const SMAA_BLEND_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_COPY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const HIZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const VOLUMETRIC_FOG_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const UI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const COVERAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const GBUFFER_SAMPLE_COUNT: u32 = 2;
const CAMERA_MOVE_SPEED: f32 = 8.0;
const CAMERA_FAST_MULTIPLIER: f32 = 3.5;
const CAMERA_LOOK_SPEED: f32 = 1.55;
const CAMERA_MOUSE_LOOK_SPEED: f32 = 0.0025;
// Keep the expanded Dust II precipitation volume at the original rain density.
const RAIN_DROP_COUNT: u32 = 32_768;
const RAIN_PRISM_VERTEX_COUNT: u32 = 36;
const MAX_GPU_POINT_LIGHTS: usize = 6;
const MAX_REFLECTION_PROXIES: usize = 24;
const MATERIAL_PROFILE_COUNT: usize = 256;
const SHADOW_MAP_RESOLUTION: u32 = 4_096;
const LOCAL_SHADOW_MAP_RESOLUTION: u32 = 1_024;
const BLOOM_SCALE: u32 = 4;
const HIZ_SCALE: u32 = 2;
const FOG_SCALE: u32 = 2;
const SSR_SCALE: u32 = 2;
const REVERSE_Z_CLEAR_DEPTH: f32 = 0.0;
const GPU_TIMESTAMP_COUNT: u32 = 7;
const GPU_STAGE_NAMES: [&str; 6] = [
    "cached shadows",
    "geometry + 2x resolve",
    "visibility + reflections + lighting",
    "velocity + bloom + tonemap",
    "SMAA T2x",
    "UI + gamma presentation",
];

fn generated_rain_streak_detail() -> TextureImage {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 64;
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let horizontal = ((x as f32 + 0.5) / WIDTH as f32 - 0.5).abs() * 2.0;
            let core = (1.0 - horizontal).max(0.0).powf(5.0);
            let vertical = ((y as f32 + 0.5) / HEIGHT as f32 * std::f32::consts::PI)
                .sin()
                .max(0.0);
            let intensity = core * vertical;
            pixels.push(crate::math::Color::new(
                intensity, intensity, intensity, intensity,
            ));
        }
    }
    TextureImage {
        width: WIDTH,
        height: HEIGHT,
        pixels: pixels.into(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveOptions {
    pub max_frames: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub capture_path: Option<PathBuf>,
    pub exposure_min: Option<f32>,
    pub exposure_max: Option<f32>,
    pub exposure_compensation: Option<f32>,
    pub white_balance_strength: Option<f32>,
    pub velocity_compensation_strength: f32,
    pub camera_velocity_cutoff: f32,
    pub display_gamma: f32,
    pub display_brightness: f32,
    pub profile_gpu: bool,
    pub static_camera: bool,
    /// Half-extent in metres of the directional shadow map that follows a game camera.
    pub shadow_distance: f32,
    pub title: String,
}

impl Default for LiveOptions {
    fn default() -> Self {
        Self {
            max_frames: None,
            width: 1280,
            height: 720,
            capture_path: None,
            exposure_min: None,
            exposure_max: None,
            exposure_compensation: None,
            white_balance_strength: None,
            velocity_compensation_strength: 0.65,
            camera_velocity_cutoff: 8.0,
            display_gamma: 2.2,
            display_brightness: 1.0,
            profile_gpu: false,
            static_camera: false,
            shadow_distance: 60.0,
            title: "Dhool Pro".to_string(),
        }
    }
}

pub fn run_live_renderer(scene: RenderScene, options: LiveOptions) -> Result<(), LiveError> {
    let event_loop = EventLoop::new().map_err(|error| LiveError::Window(error.to_string()))?;
    let mut application = LiveApplication::new_scene(scene, options);
    run_live_application(event_loop, &mut application)
}

/// Runs game schedules and streams the resulting world into the live GPU renderer.
pub fn run_live_game(game: GameEngine, options: LiveOptions) -> Result<(), LiveError> {
    let event_loop = EventLoop::new().map_err(|error| LiveError::Window(error.to_string()))?;
    let mut application = LiveApplication::new_game(game, options);
    run_live_application(event_loop, &mut application)
}

fn run_live_application(
    event_loop: EventLoop<()>,
    application: &mut LiveApplication,
) -> Result<(), LiveError> {
    event_loop
        .run_app(application)
        .map_err(|error| LiveError::Window(error.to_string()))?;
    if let Some(error) = application.error.take() {
        Err(error)
    } else {
        Ok(())
    }
}

#[derive(Debug)]
pub enum LiveError {
    Window(String),
    Scene(String),
    Surface(String),
    AdapterUnavailable,
    Device(String),
    Capture(String),
    Engine(String),
}

impl fmt::Display for LiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(message) => write!(formatter, "window error: {message}"),
            Self::Scene(message) => write!(formatter, "scene error: {message}"),
            Self::Surface(message) => write!(formatter, "surface error: {message}"),
            Self::AdapterUnavailable => write!(formatter, "no compatible GPU adapter"),
            Self::Device(message) => write!(formatter, "GPU device error: {message}"),
            Self::Capture(message) => write!(formatter, "capture error: {message}"),
            Self::Engine(message) => write!(formatter, "game update failed: {message}"),
        }
    }
}

impl std::error::Error for LiveError {}

enum LiveContent {
    Scene(Option<Box<RenderScene>>),
    Game(Box<GameEngine>),
}

#[derive(Default)]
struct LiveGameInput {
    keys: CameraKeys,
    held: KeySet,
    pressed: KeySet,
    gamepad: GamepadState,
    look_delta: Vec2,
    primary_action: bool,
    secondary_action: bool,
    mouse_look: bool,
}

impl LiveGameInput {
    fn state(&self) -> InputState {
        InputState {
            move_axis: Vec3::new(
                (self.keys.move_right as u8 as f32) - (self.keys.move_left as u8 as f32),
                (self.keys.move_up as u8 as f32) - (self.keys.move_down as u8 as f32),
                (self.keys.move_forward as u8 as f32) - (self.keys.move_back as u8 as f32),
            ),
            look_delta: self.look_delta,
            primary_action: self.primary_action,
            secondary_action: self.secondary_action,
            keys: self.held,
            keys_pressed: self.pressed,
            gamepad: self.gamepad,
        }
    }

    fn set_key(&mut self, key: Key, pressed: bool, repeat: bool) {
        if pressed {
            if !repeat && !self.held.contains(key) {
                self.pressed.insert(key);
            }
            self.held.insert(key);
        } else {
            self.held.remove(key);
        }
    }

    fn poll_gamepad(&mut self, gilrs: &mut gilrs::Gilrs) {
        let mut pressed = GamepadButtons::default();
        while let Some(event) = gilrs.next_event() {
            if let gilrs::EventType::ButtonPressed(button, _) = event.event {
                if let Some(button) = map_gamepad_button(button) {
                    pressed.insert(button);
                }
            }
        }
        let Some((_, pad)) = gilrs.gamepads().find(|(_, pad)| pad.is_connected()) else {
            self.gamepad = GamepadState::default();
            return;
        };
        let axis = |axis| pad.axis_data(axis).map(|data| data.value()).unwrap_or(0.0);
        let trigger = |button| {
            pad.button_data(button)
                .map(|data| data.value())
                .unwrap_or(0.0)
        };
        let mut held = GamepadButtons::default();
        for (source, button) in GAMEPAD_BUTTONS {
            if pad.is_pressed(source) {
                held.insert(button);
            }
        }
        self.gamepad = GamepadState {
            connected: true,
            left_stick: Vec2::new(axis(gilrs::Axis::LeftStickX), axis(gilrs::Axis::LeftStickY)),
            right_stick: Vec2::new(
                axis(gilrs::Axis::RightStickX),
                axis(gilrs::Axis::RightStickY),
            ),
            left_trigger: trigger(gilrs::Button::LeftTrigger2),
            right_trigger: trigger(gilrs::Button::RightTrigger2),
            held,
            pressed,
        };
    }

    fn finish_frame(&mut self) {
        self.look_delta = Vec2::ZERO;
        self.primary_action = false;
        self.secondary_action = false;
        self.pressed.clear();
        self.gamepad.pressed.clear();
    }
}

const GAMEPAD_BUTTONS: [(gilrs::Button, GamepadButton); 14] = [
    (gilrs::Button::South, GamepadButton::South),
    (gilrs::Button::East, GamepadButton::East),
    (gilrs::Button::North, GamepadButton::North),
    (gilrs::Button::West, GamepadButton::West),
    (gilrs::Button::LeftTrigger, GamepadButton::LeftBumper),
    (gilrs::Button::RightTrigger, GamepadButton::RightBumper),
    (gilrs::Button::Select, GamepadButton::Select),
    (gilrs::Button::Start, GamepadButton::Start),
    (gilrs::Button::LeftThumb, GamepadButton::LeftStick),
    (gilrs::Button::RightThumb, GamepadButton::RightStick),
    (gilrs::Button::DPadUp, GamepadButton::DPadUp),
    (gilrs::Button::DPadDown, GamepadButton::DPadDown),
    (gilrs::Button::DPadLeft, GamepadButton::DPadLeft),
    (gilrs::Button::DPadRight, GamepadButton::DPadRight),
];

fn map_gamepad_button(button: gilrs::Button) -> Option<GamepadButton> {
    GAMEPAD_BUTTONS
        .iter()
        .find(|(source, _)| *source == button)
        .map(|(_, button)| *button)
}

fn map_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyA => Key::A,
        KeyCode::KeyB => Key::B,
        KeyCode::KeyC => Key::C,
        KeyCode::KeyD => Key::D,
        KeyCode::KeyE => Key::E,
        KeyCode::KeyF => Key::F,
        KeyCode::KeyG => Key::G,
        KeyCode::KeyH => Key::H,
        KeyCode::KeyI => Key::I,
        KeyCode::KeyJ => Key::J,
        KeyCode::KeyK => Key::K,
        KeyCode::KeyL => Key::L,
        KeyCode::KeyM => Key::M,
        KeyCode::KeyN => Key::N,
        KeyCode::KeyO => Key::O,
        KeyCode::KeyP => Key::P,
        KeyCode::KeyQ => Key::Q,
        KeyCode::KeyR => Key::R,
        KeyCode::KeyS => Key::S,
        KeyCode::KeyT => Key::T,
        KeyCode::KeyU => Key::U,
        KeyCode::KeyV => Key::V,
        KeyCode::KeyW => Key::W,
        KeyCode::KeyX => Key::X,
        KeyCode::KeyY => Key::Y,
        KeyCode::KeyZ => Key::Z,
        KeyCode::Digit0 => Key::Digit0,
        KeyCode::Digit1 => Key::Digit1,
        KeyCode::Digit2 => Key::Digit2,
        KeyCode::Digit3 => Key::Digit3,
        KeyCode::Digit4 => Key::Digit4,
        KeyCode::Digit5 => Key::Digit5,
        KeyCode::Digit6 => Key::Digit6,
        KeyCode::Digit7 => Key::Digit7,
        KeyCode::Digit8 => Key::Digit8,
        KeyCode::Digit9 => Key::Digit9,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::Space => Key::Space,
        KeyCode::Enter | KeyCode::NumpadEnter => Key::Enter,
        KeyCode::Escape => Key::Escape,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::ShiftLeft => Key::ShiftLeft,
        KeyCode::ShiftRight => Key::ShiftRight,
        KeyCode::ControlLeft => Key::ControlLeft,
        KeyCode::ControlRight => Key::ControlRight,
        KeyCode::AltLeft => Key::AltLeft,
        KeyCode::AltRight => Key::AltRight,
        KeyCode::Comma => Key::Comma,
        KeyCode::Period => Key::Period,
        KeyCode::Minus => Key::Minus,
        KeyCode::Equal => Key::Equal,
        KeyCode::BracketLeft => Key::BracketLeft,
        KeyCode::BracketRight => Key::BracketRight,
        KeyCode::Semicolon => Key::Semicolon,
        KeyCode::Quote => Key::Quote,
        KeyCode::Slash => Key::Slash,
        _ => return None,
    })
}

struct LiveApplication {
    content: LiveContent,
    options: LiveOptions,
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    error: Option<LiveError>,
    frame_count: u64,
    title_clock: Instant,
    title_frames: u32,
    /// CPU time per stage since the last FPS report: advance, stream, render.
    cpu_stages: [Duration; 3],
    game_input: LiveGameInput,
    game_clock: Instant,
    gilrs: Option<gilrs::Gilrs>,
}

impl LiveApplication {
    fn new_scene(scene: RenderScene, options: LiveOptions) -> Self {
        Self::new(LiveContent::Scene(Some(Box::new(scene))), options)
    }

    fn new_game(game: GameEngine, options: LiveOptions) -> Self {
        Self::new(LiveContent::Game(Box::new(game)), options)
    }

    fn new(content: LiveContent, options: LiveOptions) -> Self {
        Self {
            content,
            options,
            window: None,
            renderer: None,
            error: None,
            frame_count: 0,
            title_clock: Instant::now(),
            title_frames: 0,
            cpu_stages: [Duration::ZERO; 3],
            game_input: LiveGameInput::default(),
            game_clock: Instant::now(),
            gilrs: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: LiveError) {
        eprintln!("{error}");
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for LiveApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(format!("{} | GPU renderer starting", self.options.title))
            .with_inner_size(PhysicalSize::new(self.options.width, self.options.height));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, LiveError::Window(error.to_string()));
                return;
            }
        };
        let dynamic_game = matches!(self.content, LiveContent::Game(_));
        if dynamic_game {
            // Controllers are optional; a missing backend only disables them.
            self.gilrs = gilrs::Gilrs::new().ok();
        }
        let scene = match &mut self.content {
            LiveContent::Scene(scene) => *scene.take().expect("scene is consumed once"),
            LiveContent::Game(game) => game.world.build_render_scene(),
        };
        let mut renderer = match pollster::block_on(GpuRenderer::new(
            window.clone(),
            scene,
            self.options.capture_path.clone(),
            LiveRendererConfig::from_options(&self.options),
            dynamic_game,
        )) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        if let Some(capture) = &mut renderer.frame_capture {
            capture.frame = self.options.max_frames.unwrap_or(31).saturating_sub(1);
        }
        println!(
            "live GPU: {} ({:?}, {:?})",
            renderer.adapter.name, renderer.adapter.backend, renderer.adapter.device_type
        );
        println!(
            "live pipeline: alpha-aware cached shadows -> 2x MSAA selective prepass/PBR+motion G-buffer + coverage resolve -> R32 depth/Hi-Z/fog -> SSR-first temporal reflections -> D32S8 light volumes + material-profile HDR lighting/rain -> velocity compensation + Gaussian bloom/GT resolve -> coverage-assisted SMAA T2x -> separate UI/gamma presentation"
        );
        window.set_title(&format!(
            "{} | {}",
            self.options.title, renderer.adapter.name
        ));
        window.request_redraw();
        self.window = Some(window);
        self.renderer = Some(renderer);
        self.title_clock = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                match event.physical_key {
                    // Games decide what Escape means and leave through `AppExit`.
                    PhysicalKey::Code(code) if matches!(self.content, LiveContent::Game(_)) => {
                        if code == KeyCode::Space && pressed && !event.repeat {
                            self.game_input.primary_action = true;
                        }
                        set_camera_key(&mut self.game_input.keys, code, pressed);
                        if let Some(key) = map_key(code) {
                            self.game_input.set_key(key, pressed, event.repeat);
                        }
                    }
                    PhysicalKey::Code(KeyCode::Escape) if pressed => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::Space) if pressed && !event.repeat => {
                        renderer.toggle_auto_orbit();
                    }
                    PhysicalKey::Code(code) => renderer.set_key(code, pressed),
                    _ => {}
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                if matches!(self.content, LiveContent::Game(_)) {
                    self.game_input.mouse_look = pressed;
                    window.set_cursor_visible(!pressed);
                } else if let Some(renderer) = self.renderer.as_mut() {
                    renderer.set_mouse_look(pressed);
                    window.set_cursor_visible(!pressed);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if matches!(self.content, LiveContent::Game(_)) => {
                self.game_input.secondary_action = true;
            }
            WindowEvent::RedrawRequested => {
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                if let LiveContent::Game(game) = &mut self.content {
                    let now = Instant::now();
                    let delta_seconds = now
                        .duration_since(self.game_clock)
                        .as_secs_f32()
                        .clamp(0.0, 0.25);
                    self.game_clock = now;
                    if let Some(gilrs) = self.gilrs.as_mut() {
                        self.game_input.poll_gamepad(gilrs);
                    }
                    if let Some(hud) = game.resources.get_mut::<Hud>() {
                        hud.resize(renderer.config.width, renderer.config.height);
                    }
                    let stage = Instant::now();
                    if let Err(error) = game.advance(FrameInput {
                        delta_seconds,
                        input: self.game_input.state(),
                    }) {
                        self.fail(event_loop, LiveError::Engine(error.to_string()));
                        return;
                    }
                    self.cpu_stages[0] += stage.elapsed();
                    let stage = Instant::now();
                    if let Err(error) = renderer.update_world(&game.world) {
                        self.fail(event_loop, LiveError::Scene(error));
                        return;
                    }
                    if let Some(hud) = game.resources.get::<Hud>() {
                        renderer.upload_hud(hud);
                    }
                    self.cpu_stages[1] += stage.elapsed();
                    self.game_input.finish_frame();
                    if !game.events.read::<AppExit>().is_empty() {
                        event_loop.exit();
                        return;
                    }
                }
                if let Err(message) = renderer.update() {
                    self.fail(event_loop, LiveError::Scene(message));
                    return;
                }
                let stage = Instant::now();
                let result = renderer.render();
                self.cpu_stages[2] += stage.elapsed();
                match result {
                    RenderResult::Presented => {
                        self.frame_count += 1;
                        self.title_frames += 1;
                    }
                    RenderResult::Reconfigure => renderer.resize(renderer.size),
                    RenderResult::Skipped => {}
                    RenderResult::Fatal(message) => {
                        self.fail(event_loop, LiveError::Surface(message));
                        return;
                    }
                }

                let elapsed = self.title_clock.elapsed();
                if elapsed >= Duration::from_secs(1) {
                    let fps = self.title_frames as f64 / elapsed.as_secs_f64();
                    println!("live FPS: {fps:.1}");
                    if self.options.profile_gpu {
                        let frames = self.title_frames.max(1) as f64;
                        let [advance, stream, render] = self
                            .cpu_stages
                            .map(|stage| stage.as_secs_f64() * 1000.0 / frames);
                        println!(
                            "cpu ms/frame: game {advance:.2}, world stream {stream:.2}, render submit {render:.2}"
                        );
                    }
                    self.cpu_stages = [Duration::ZERO; 3];
                    window.set_title(&format!(
                        "{} | {} | {:.0} FPS",
                        self.options.title, renderer.adapter.name, fps
                    ));
                    self.title_clock = Instant::now();
                    self.title_frames = 0;
                }
                if self
                    .options
                    .max_frames
                    .is_some_and(|limit| self.frame_count >= limit)
                {
                    println!("live smoke test presented {} frames", self.frame_count);
                    event_loop.exit();
                } else {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if let DeviceEvent::MouseMotion { delta } = event {
            if matches!(self.content, LiveContent::Game(_)) {
                if self.game_input.mouse_look {
                    self.game_input.look_delta.x += delta.0 as f32;
                    self.game_input.look_delta.y -= delta.1 as f32;
                }
            } else {
                renderer.apply_mouse_delta(delta);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [u8; 4],
    tangent: [f32; 4],
}

impl GpuVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x2,
        3 => Unorm8x4,
        4 => Float32x4
    ];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuPreviousPosition {
    position: [f32; 3],
}

impl GpuPreviousPosition {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![5 => Float32x3];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    position_time: [f32; 4],
    viewport: [f32; 4],
    previous_view_projection: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    surface: [f32; 4],
    emission: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct FrameUniform {
    dimensions: [f32; 4],
    counters: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct AaUniform {
    previous_view_projection: [[f32; 4]; 4],
    viewport: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ExposureUniform {
    limits_compensation_balance: [f32; 4],
    adaptation_speeds: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct ExposureOverrides {
    min: Option<f32>,
    max: Option<f32>,
    compensation: Option<f32>,
    white_balance_strength: Option<f32>,
}

impl ExposureOverrides {
    fn resolve(self, profile: crate::scene::ExposureProfile) -> ExposureUniform {
        let min = self.min.unwrap_or(profile.min).max(0.01);
        let max = self.max.unwrap_or(profile.max).max(min);
        ExposureUniform {
            limits_compensation_balance: [
                min,
                max,
                self.compensation.unwrap_or(profile.compensation),
                self.white_balance_strength
                    .unwrap_or(profile.white_balance_strength)
                    .clamp(0.0, 1.0),
            ],
            adaptation_speeds: [2.4, 1.1, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct VelocityCompensationUniform {
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct DisplayUniform {
    gamma_brightness: [f32; 4],
}

struct LiveRendererConfig {
    exposure: ExposureOverrides,
    velocity_compensation: VelocityCompensationUniform,
    display: DisplayUniform,
    profile_gpu: bool,
    static_camera: bool,
    shadow_distance: f32,
}

impl LiveRendererConfig {
    fn from_options(options: &LiveOptions) -> Self {
        Self {
            exposure: ExposureOverrides {
                min: options.exposure_min,
                max: options.exposure_max,
                compensation: options.exposure_compensation,
                white_balance_strength: options.white_balance_strength,
            },
            velocity_compensation: VelocityCompensationUniform {
                params: [
                    options.velocity_compensation_strength.clamp(0.0, 2.0),
                    (options.camera_velocity_cutoff * 0.55).max(0.1),
                    options.camera_velocity_cutoff.max(0.2),
                    0.035,
                ],
            },
            display: DisplayUniform {
                gamma_brightness: [
                    options.display_gamma.clamp(1.0, 3.0),
                    options.display_brightness.max(0.0),
                    0.0,
                    0.0,
                ],
            },
            profile_gpu: options.profile_gpu,
            static_camera: options.static_camera,
            shadow_distance: options.shadow_distance.max(4.0),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaterialProfileUniform {
    dielectric: [[f32; 4]; MATERIAL_PROFILE_COUNT],
    conductor: [[f32; 4]; MATERIAL_PROFILE_COUNT],
}

impl MaterialProfileUniform {
    fn from_scene(scene: &RenderScene) -> Self {
        let mut profiles = Self::zeroed();
        for index in 0..MATERIAL_PROFILE_COUNT {
            profiles.dielectric[index] = [0.04, 0.04, 0.04, 1.0];
            profiles.conductor[index] = [0.91, 0.92, 0.94, 0.0];
        }
        for material in &scene.materials {
            let index = material.material_id as usize;
            let dielectric_f0 = (0.02 + material.specular_intensity * 0.14).clamp(0.02, 0.18);
            profiles.dielectric[index] = [dielectric_f0; 4];
            profiles.dielectric[index][3] = match material.class {
                crate::scene::MaterialClass::Skin => 1.08,
                crate::scene::MaterialClass::Foliage => 1.04,
                _ => 1.0,
            };
            profiles.conductor[index] = [
                material.base_color.r.max(0.04),
                material.base_color.g.max(0.04),
                material.base_color.b.max(0.04),
                match material.class {
                    crate::scene::MaterialClass::Skin => 0.32,
                    crate::scene::MaterialClass::Hair => 0.65,
                    crate::scene::MaterialClass::Foliage => 0.12,
                    _ => 0.0,
                },
            ];
        }
        profiles
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct LightUniform {
    directional_direction_count: [f32; 4],
    directional_color_intensity: [f32; 4],
    ambient: [f32; 4],
    sky: [f32; 4],
    weather: [f32; 4],
    environment: [f32; 4],
    environment_sh: [[f32; 4]; 9],
    local_position_radius: [[f32; 4]; MAX_GPU_POINT_LIGHTS],
    local_color_intensity: [[f32; 4]; MAX_GPU_POINT_LIGHTS],
    local_direction_outer: [[f32; 4]; MAX_GPU_POINT_LIGHTS],
    local_inner_shadow: [[f32; 4]; MAX_GPU_POINT_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ReflectionProxyUniform {
    count: [u32; 4],
    bounds_min: [[f32; 4]; MAX_REFLECTION_PROXIES],
    bounds_max: [[f32; 4]; MAX_REFLECTION_PROXIES],
    radiance: [[f32; 4]; MAX_REFLECTION_PROXIES],
}

impl LightUniform {
    fn from_scene(scene: &RenderScene) -> Self {
        let sky_irradiance = scene
            .environment_map
            .diffuse_irradiance(crate::math::Vec3::Y)
            * std::f32::consts::FRAC_1_PI;
        let mut uniform = Self {
            ambient: [
                scene.ambient_color.r,
                scene.ambient_color.g,
                scene.ambient_color.b,
                1.0,
            ],
            sky: [sky_irradiance.x, sky_irradiance.y, sky_irradiance.z, 1.0],
            weather: [
                scene.environment.precipitation,
                scene.environment.surface_wetness,
                scene.environment.wind,
                0.0,
            ],
            environment: [
                scene.environment_map.exposure_ev(),
                scene.environment_map.yaw_radians(),
                1.0,
                0.0,
            ],
            environment_sh: (*scene.environment_map.irradiance_sh())
                .map(|coefficient| [coefficient[0], coefficient[1], coefficient[2], 0.0]),
            ..Self::zeroed()
        };
        let mut local_count = 0_usize;
        for light in &scene.lights {
            match light.kind {
                LightKind::Directional { direction }
                    if uniform.directional_color_intensity[3] == 0.0 =>
                {
                    let surface_to_light =
                        GpuVec3::new(-direction.x, -direction.y, -direction.z).normalize_or_zero();
                    uniform.directional_direction_count = [
                        surface_to_light.x,
                        surface_to_light.y,
                        surface_to_light.z,
                        0.0,
                    ];
                    uniform.directional_color_intensity =
                        [light.color.r, light.color.g, light.color.b, light.intensity];
                }
                LightKind::Local { position, radius } if local_count < MAX_GPU_POINT_LIGHTS => {
                    uniform.local_position_radius[local_count] =
                        [position.x, position.y, position.z, radius];
                    uniform.local_color_intensity[local_count] =
                        [light.color.r, light.color.g, light.color.b, light.intensity];
                    local_count += 1;
                }
                LightKind::Spot {
                    position,
                    direction,
                    radius,
                    inner_angle_radians,
                    outer_angle_radians,
                } if local_count < MAX_GPU_POINT_LIGHTS => {
                    let direction =
                        GpuVec3::new(direction.x, direction.y, direction.z).normalize_or_zero();
                    uniform.local_position_radius[local_count] =
                        [position.x, position.y, position.z, radius];
                    uniform.local_color_intensity[local_count] =
                        [light.color.r, light.color.g, light.color.b, light.intensity];
                    uniform.local_direction_outer[local_count] = [
                        direction.x,
                        direction.y,
                        direction.z,
                        outer_angle_radians.cos(),
                    ];
                    uniform.local_inner_shadow[local_count] = [
                        inner_angle_radians.cos(),
                        if light.casts_shadow { 1.0 } else { 0.0 },
                        local_count as f32,
                        1.0,
                    ];
                    local_count += 1;
                }
                _ => {}
            }
        }
        uniform.directional_direction_count[3] = local_count as f32;
        uniform
    }

    fn realtime(mut self, elapsed: f32) -> Self {
        self.weather[2] += elapsed.sin() * 0.025;
        self.weather[3] = elapsed;
        if self.weather[0] > 0.01 {
            let moon_flicker = 0.96 + (elapsed * 0.17).sin() * 0.025;
            self.directional_color_intensity[3] *= moon_flicker;
        }
        if self.weather[0] > 0.01 {
            let local_count = self.directional_direction_count[3]
                .clamp(0.0, MAX_GPU_POINT_LIGHTS as f32) as usize;
            for light_index in 0..local_count {
                let phase = elapsed * (7.0 + light_index as f32 * 2.35) + light_index as f32 * 1.71;
                let flicker = 0.88 + phase.sin() * 0.08 + (phase * 2.13).sin() * 0.035;
                self.local_color_intensity[light_index][3] *= flicker.clamp(0.72, 1.12);
            }
        }
        self
    }

    fn surface_to_directional_light(self) -> GpuVec3 {
        GpuVec3::new(
            self.directional_direction_count[0],
            self.directional_direction_count[1],
            self.directional_direction_count[2],
        )
        .normalize_or_zero()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct LocalShadowUniform {
    view_projection: [[[f32; 4]; 4]; MAX_GPU_POINT_LIGHTS],
}

#[derive(Clone, Copy, Debug, Default)]
struct CameraKeys {
    move_forward: bool,
    move_back: bool,
    move_left: bool,
    move_right: bool,
    move_up: bool,
    move_down: bool,
    boost: bool,
    look_left: bool,
    look_right: bool,
    look_up: bool,
    look_down: bool,
}

fn apply_camera_keys_to_pose(
    camera_origin: &mut GpuVec3,
    camera_target: &mut GpuVec3,
    keys: CameraKeys,
    dt: f32,
) {
    let forward = (*camera_target - *camera_origin).normalize_or_zero();
    let flat_forward = GpuVec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let flat_right = flat_forward.cross(GpuVec3::Y).normalize_or_zero();
    let forward_axis =
        (if keys.move_forward { 1.0 } else { 0.0 }) - (if keys.move_back { 1.0 } else { 0.0 });
    let strafe_axis =
        (if keys.move_right { 1.0 } else { 0.0 }) - (if keys.move_left { 1.0 } else { 0.0 });
    let vertical_axis =
        (if keys.move_up { 1.0 } else { 0.0 }) - (if keys.move_down { 1.0 } else { 0.0 });
    let movement =
        flat_forward * forward_axis + flat_right * strafe_axis + GpuVec3::Y * vertical_axis;
    if movement.length_squared() > 0.0 {
        let speed = CAMERA_MOVE_SPEED
            * if keys.boost {
                CAMERA_FAST_MULTIPLIER
            } else {
                1.0
            };
        let delta = movement.normalize() * (speed * dt);
        *camera_origin += delta;
        *camera_target += delta;
    }

    let yaw = ((if keys.look_right { 1.0 } else { 0.0 })
        - (if keys.look_left { 1.0 } else { 0.0 }))
        * CAMERA_LOOK_SPEED
        * dt;
    let pitch = ((if keys.look_up { 1.0 } else { 0.0 }) - (if keys.look_down { 1.0 } else { 0.0 }))
        * CAMERA_LOOK_SPEED
        * dt;
    rotate_camera_target(camera_origin, camera_target, yaw, pitch);
}

fn set_camera_key(keys: &mut CameraKeys, code: KeyCode, pressed: bool) -> bool {
    match code {
        KeyCode::KeyW => keys.move_forward = pressed,
        KeyCode::KeyS => keys.move_back = pressed,
        KeyCode::KeyA => keys.move_left = pressed,
        KeyCode::KeyD => keys.move_right = pressed,
        KeyCode::KeyE => keys.move_up = pressed,
        KeyCode::KeyQ => keys.move_down = pressed,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => keys.boost = pressed,
        KeyCode::ArrowLeft => keys.look_left = pressed,
        KeyCode::ArrowRight => keys.look_right = pressed,
        KeyCode::ArrowUp => keys.look_up = pressed,
        KeyCode::ArrowDown => keys.look_down = pressed,
        _ => return false,
    }
    true
}

fn rotate_camera_target(
    camera_origin: &mut GpuVec3,
    camera_target: &mut GpuVec3,
    yaw: f32,
    pitch: f32,
) {
    if yaw == 0.0 && pitch == 0.0 {
        return;
    }

    let distance = (*camera_target - *camera_origin).length().max(0.001);
    let mut direction = (*camera_target - *camera_origin).normalize_or_zero();
    if yaw != 0.0 {
        direction = Quat::from_axis_angle(GpuVec3::Y, yaw).mul_vec3(direction);
    }
    if pitch != 0.0 {
        let right = direction.cross(GpuVec3::Y).normalize_or_zero();
        let pitched = Quat::from_axis_angle(right, pitch)
            .mul_vec3(direction)
            .normalize_or_zero();
        if pitched.dot(GpuVec3::Y).abs() < 0.97 {
            direction = pitched;
        }
    }
    *camera_target = *camera_origin + direction * distance;
}

struct DrawBatch {
    indices: Range<u32>,
    material_index: usize,
    prepass_eligible: bool,
    alpha_tested: bool,
    casts_shadow: bool,
}

struct GpuMaterial {
    _uniform: wgpu::Buffer,
    _textures: Vec<wgpu::Texture>,
    bind_group: wgpu::BindGroup,
}

struct RainTextures {
    _streak_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct RainPassResources<'a> {
    camera_buffer: &'a wgpu::Buffer,
    streak_view: &'a wgpu::TextureView,
    streak_sampler: &'a wgpu::Sampler,
    light_buffer: &'a wgpu::Buffer,
    shadow_view: &'a wgpu::TextureView,
    shadow_sampler: &'a wgpu::Sampler,
    shadow_camera_buffer: &'a wgpu::Buffer,
    skybox_view: &'a wgpu::TextureView,
    skybox_sampler: &'a wgpu::Sampler,
}

struct GBuffer {
    _albedo_ms: wgpu::Texture,
    _normal_ms: wgpu::Texture,
    _pbr_ms: wgpu::Texture,
    _motion_ms: wgpu::Texture,
    _depth_ms: wgpu::Texture,
    _albedo: wgpu::Texture,
    _normal: wgpu::Texture,
    _pbr: wgpu::Texture,
    _motion: wgpu::Texture,
    _coverage: wgpu::Texture,
    _depth: wgpu::Texture,
    _depth_copy: wgpu::Texture,
    _hiz: wgpu::Texture,
    _ssr: wgpu::Texture,
    _reflection: wgpu::Texture,
    _rt_history_a: wgpu::Texture,
    _rt_history_b: wgpu::Texture,
    _volumetric_fog: wgpu::Texture,
    _lighting_hdr: wgpu::Texture,
    _velocity_compensation_a: wgpu::Texture,
    _velocity_compensation_b: wgpu::Texture,
    _exposure_white_balance_a: wgpu::Texture,
    _exposure_white_balance_b: wgpu::Texture,
    _bloom_a: wgpu::Texture,
    _bloom_b: wgpu::Texture,
    _tone_mapped: wgpu::Texture,
    _smaa_edges: wgpu::Texture,
    _smaa_blend: wgpu::Texture,
    _aa_history_a: wgpu::Texture,
    _aa_history_b: wgpu::Texture,
    _aa_presented: wgpu::Texture,
    _ui_overlay: wgpu::Texture,
    hud_texture: wgpu::Texture,
    albedo: wgpu::TextureView,
    normal: wgpu::TextureView,
    pbr: wgpu::TextureView,
    motion: wgpu::TextureView,
    coverage: wgpu::TextureView,
    albedo_ms: wgpu::TextureView,
    normal_ms: wgpu::TextureView,
    pbr_ms: wgpu::TextureView,
    motion_ms: wgpu::TextureView,
    _depth_ms_view: wgpu::TextureView,
    depth_attachment: wgpu::TextureView,
    depth_ms_attachment: wgpu::TextureView,
    _depth_view: wgpu::TextureView,
    _depth_copy_view: wgpu::TextureView,
    _hiz_view: wgpu::TextureView,
    _ssr_view: wgpu::TextureView,
    lighting_hdr: wgpu::TextureView,
    velocity_compensation_a: wgpu::TextureView,
    velocity_compensation_b: wgpu::TextureView,
    exposure_white_balance_bind_group_a: wgpu::BindGroup,
    exposure_white_balance_bind_group_b: wgpu::BindGroup,
    bloom_a: wgpu::TextureView,
    bloom_b: wgpu::TextureView,
    tone_mapped: wgpu::TextureView,
    _volumetric_fog_view: wgpu::TextureView,
    smaa_edges: wgpu::TextureView,
    smaa_blend: wgpu::TextureView,
    aa_history_a: wgpu::TextureView,
    aa_history_b: wgpu::TextureView,
    aa_presented: wgpu::TextureView,
    ui_overlay: wgpu::TextureView,
    reflection_bind_group_a: wgpu::BindGroup,
    reflection_bind_group_b: wgpu::BindGroup,
    lighting_bind_group: wgpu::BindGroup,
    light_volume_bind_group: wgpu::BindGroup,
    gbuffer_resolve_bind_group: wgpu::BindGroup,
    velocity_compensation_bind_group: wgpu::BindGroup,
    velocity_smoothing_bind_group: wgpu::BindGroup,
    bloom_extract_bind_group: wgpu::BindGroup,
    bloom_blur_a_bind_group: wgpu::BindGroup,
    bloom_blur_b_bind_group: wgpu::BindGroup,
    tonemap_bind_group_a: wgpu::BindGroup,
    tonemap_bind_group_b: wgpu::BindGroup,
    depth_copy_bind_group: wgpu::BindGroup,
    hiz_bind_group: wgpu::BindGroup,
    fog_bind_group: wgpu::BindGroup,
    ssr_bind_group: wgpu::BindGroup,
    smaa_edge_bind_group: wgpu::BindGroup,
    smaa_blend_bind_group: wgpu::BindGroup,
    aa_bind_group_a: wgpu::BindGroup,
    aa_bind_group_b: wgpu::BindGroup,
    ui_bind_group: wgpu::BindGroup,
    present_bind_group: wgpu::BindGroup,
    reflection_width: u32,
    reflection_height: u32,
    hiz_width: u32,
    hiz_height: u32,
    ssr_width: u32,
    ssr_height: u32,
    fog_width: u32,
    fog_height: u32,
}

struct GpuRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    adapter: wgpu::AdapterInfo,
    vertex_buffer: wgpu::Buffer,
    previous_position_buffer: wgpu::Buffer,
    dynamic_instances: Vec<DynamicInstance>,
    instance_transforms: InstanceTransforms,
    dynamic_layout: Vec<(EntityId, u64)>,
    dynamic_shadow_camera: CameraUniform,
    shadow_distance: f32,
    dynamic_material_signatures: Vec<u64>,
    dynamic_environment_map: EnvironmentMap,
    dynamic_light_layout: Vec<(u8, bool)>,
    dynamic_local_shadow_kinds: Vec<(usize, LightKind)>,
    index_buffer: wgpu::Buffer,
    batches: Vec<DrawBatch>,
    materials: Vec<GpuMaterial>,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    frame_buffer: wgpu::Buffer,
    light_buffer: wgpu::Buffer,
    _material_profile_buffer: wgpu::Buffer,
    _exposure_buffer: wgpu::Buffer,
    _velocity_compensation_buffer: wgpu::Buffer,
    _display_buffer: wgpu::Buffer,
    reflection_proxy_buffer: wgpu::Buffer,
    aa_buffer: wgpu::Buffer,
    shadow_camera_buffer: wgpu::Buffer,
    shadow_camera_bind_group: wgpu::BindGroup,
    _shadow_texture: wgpu::Texture,
    shadow_view: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    local_shadow_buffer: wgpu::Buffer,
    _local_shadow_texture: wgpu::Texture,
    local_shadow_array_view: wgpu::TextureView,
    local_shadow_views: Vec<wgpu::TextureView>,
    _local_shadow_camera_buffers: Vec<wgpu::Buffer>,
    local_shadow_bind_groups: Vec<wgpu::BindGroup>,
    shadowed_local_light_indices: Vec<usize>,
    shadow_cache_ready: bool,
    local_shadow_cache_ready: Vec<bool>,
    depth_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    alpha_shadow_pipeline: wgpu::RenderPipeline,
    geometry_pipeline: wgpu::RenderPipeline,
    gbuffer_resolve_pipeline: wgpu::RenderPipeline,
    depth_copy_pipeline: wgpu::RenderPipeline,
    hiz_pipeline: wgpu::ComputePipeline,
    fog_pipeline: wgpu::ComputePipeline,
    ssr_pipeline: wgpu::ComputePipeline,
    reflection_pipeline: wgpu::ComputePipeline,
    lighting_pipeline: wgpu::RenderPipeline,
    light_volume_pipeline: wgpu::RenderPipeline,
    local_lighting_pipeline: wgpu::RenderPipeline,
    velocity_compensation_pipeline: wgpu::RenderPipeline,
    velocity_smoothing_pipeline: wgpu::RenderPipeline,
    auto_exposure_pipeline: wgpu::ComputePipeline,
    bloom_extract_pipeline: wgpu::RenderPipeline,
    bloom_blur_horizontal_pipeline: wgpu::RenderPipeline,
    bloom_blur_vertical_pipeline: wgpu::RenderPipeline,
    tonemap_pipeline: wgpu::RenderPipeline,
    aa_pipeline: wgpu::RenderPipeline,
    ui_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,
    smaa_edge_pipeline: wgpu::RenderPipeline,
    smaa_blend_pipeline: wgpu::RenderPipeline,
    rain_pipeline: wgpu::RenderPipeline,
    rain: RainTextures,
    reflection_layout: wgpu::BindGroupLayout,
    gbuffer_resolve_layout: wgpu::BindGroupLayout,
    lighting_layout: wgpu::BindGroupLayout,
    light_volume_layout: wgpu::BindGroupLayout,
    velocity_compensation_layout: wgpu::BindGroupLayout,
    auto_exposure_layout: wgpu::BindGroupLayout,
    bloom_layout: wgpu::BindGroupLayout,
    tonemap_layout: wgpu::BindGroupLayout,
    depth_copy_layout: wgpu::BindGroupLayout,
    hiz_layout: wgpu::BindGroupLayout,
    fog_layout: wgpu::BindGroupLayout,
    ssr_layout: wgpu::BindGroupLayout,
    aa_layout: wgpu::BindGroupLayout,
    ui_layout: wgpu::BindGroupLayout,
    present_layout: wgpu::BindGroupLayout,
    smaa_edge_layout: wgpu::BindGroupLayout,
    smaa_blend_layout: wgpu::BindGroupLayout,
    bloom_format: wgpu::TextureFormat,
    gbuffer: GBuffer,
    started: Instant,
    last_update: Instant,
    frame_index: u32,
    current_view_projection: Mat4,
    previous_view_projection: Mat4,
    auto_orbit: bool,
    camera_origin: GpuVec3,
    camera_target: GpuVec3,
    camera_fov_y: f32,
    camera_near: f32,
    camera_far: f32,
    keys: CameraKeys,
    mouse_look: bool,
    base_light_uniform: LightUniform,
    local_light_count: u32,
    _environment_texture: wgpu::Texture,
    environment_view: wgpu::TextureView,
    environment_sampler: wgpu::Sampler,
    display_fps: f32,
    show_ui: bool,
    /// Revision of the HUD canvas currently in `gbuffer.hud_texture`.
    hud_revision: Option<u64>,
    hud_pixels: Vec<[u8; 4]>,
    frame_capture: Option<FrameCapture>,
    gpu_profiler: Option<GpuTimestampProfiler>,
    cloths: Vec<LiveCloth>,
    cloth_accumulator: f32,
}

/// One moving instance to re-transform on the GPU this frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TransformJob {
    current: [[f32; 4]; 4],
    previous: [[f32; 4]; 4],
    start: u32,
    count: u32,
    _padding: [u32; 2],
}

/// GPU instance transforms: object-space vertices plus per-frame jobs.
struct InstanceTransforms {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    resources: Option<InstanceTransformResources>,
    jobs: Vec<TransformJob>,
}

struct InstanceTransformResources {
    _local_vertices: wgpu::Buffer,
    job_buffer: wgpu::Buffer,
    job_capacity: usize,
    bind_group: wgpu::BindGroup,
}

/// A world entity's slice of the live vertex buffer.
struct DynamicInstance {
    vertex_range: Range<u32>,
    transform: crate::math::Mat4,
    moved_last_frame: bool,
    casts_shadow: bool,
    local_bounds: Option<(GpuVec3, GpuVec3)>,
}

struct LiveCloth {
    state: ClothState,
    mesh: crate::scene::Mesh,
    vertex_range: Range<u32>,
}

enum RenderResult {
    Presented,
    Reconfigure,
    Skipped,
    Fatal(String),
}

struct FrameCapture {
    path: PathBuf,
    written: bool,
    frame: u64,
}

struct PendingFrameCapture {
    path: PathBuf,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    unpadded_bytes_per_row: u32,
    padded_bytes_per_row: u32,
    format: wgpu::TextureFormat,
}

struct GpuTimestampProfiler {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    timestamp_period: f32,
}

impl GpuRenderer {
    async fn new(
        window: Arc<Window>,
        mut scene: RenderScene,
        capture_path: Option<PathBuf>,
        renderer_config: LiveRendererConfig,
        dynamic_game: bool,
    ) -> Result<Self, LiveError> {
        validate_live_scene(&scene)?;
        let LiveRendererConfig {
            exposure,
            velocity_compensation,
            display,
            profile_gpu,
            static_camera,
            shadow_distance,
        } = renderer_config;
        for mesh in &mut scene.meshes {
            if mesh.cloth.is_some() {
                mesh.bake_transform();
            }
        }
        let exposure = exposure.resolve(scene.environment.exposure);
        let camera_origin = GpuVec3::new(
            scene.camera.position.x,
            scene.camera.position.y,
            scene.camera.position.z,
        );
        let camera_target = GpuVec3::new(
            scene.camera.target.x,
            scene.camera.target.y,
            scene.camera.target.z,
        );
        let camera_fov_y = scene.camera.vertical_fov_radians;
        let camera_near = scene.camera.near;
        let camera_far = scene.camera.far;
        let size = window.inner_size();
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance
            .create_surface(window)
            .map_err(|error| LiveError::Surface(error.to_string()))?;
        let adapter = select_adapter(&instance, &surface).await?;
        let adapter_info = adapter.get_info();
        let adapter_features = adapter.features();
        if !adapter_features.contains(wgpu::Features::DEPTH32FLOAT_STENCIL8) {
            return Err(LiveError::Device(
                "D32S8 main depth requires DEPTH32FLOAT_STENCIL8 support".to_string(),
            ));
        }
        if !adapter_features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
            return Err(LiveError::Device(
                "2x D32 G-buffer sampling requires TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES"
                    .to_string(),
            ));
        }
        let timestamp_features =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if profile_gpu && !adapter_features.contains(timestamp_features) {
            return Err(LiveError::Device(
                "--profile-gpu requires timestamp queries inside command encoders".to_string(),
            ));
        }
        let optional_features = wgpu::Features::RG11B10UFLOAT_RENDERABLE
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
            | wgpu::Features::DEPTH32FLOAT_STENCIL8;
        let required_features = (adapter_features & optional_features)
            | if profile_gpu {
                timestamp_features
            } else {
                wgpu::Features::empty()
            };
        let bloom_format = if required_features.contains(wgpu::Features::RG11B10UFLOAT_RENDERABLE) {
            wgpu::TextureFormat::Rg11b10Ufloat
        } else {
            HDR_FORMAT
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features,
                ..wgpu::DeviceDescriptor::default()
            })
            .await
            .map_err(|error| LiveError::Device(error.to_string()))?;
        let gpu_profiler = profile_gpu.then(|| create_gpu_timestamp_profiler(&device, &queue));
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| LiveError::Surface("adapter cannot present to window".to_string()))?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        if capture_path.is_some() {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&device, &config);

        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let material_layout = material_layout(&device);
        let gbuffer_resolve_layout = gbuffer_resolve_layout(&device);
        let reflection_layout = reflection_layout(&device);
        let lighting_layout = lighting_layout(&device);
        let light_volume_layout = light_volume_layout(&device);
        let velocity_compensation_layout = velocity_compensation_layout(&device);
        let auto_exposure_layout = auto_exposure_layout(&device);
        let bloom_layout = bloom_layout(&device);
        let tonemap_layout = tonemap_layout(&device);
        let depth_copy_layout = depth_copy_layout(&device);
        let hiz_layout = hiz_layout(&device);
        let fog_layout = fog_layout(&device);
        let ssr_layout = ssr_layout(&device);
        let smaa_edge_layout = smaa_edge_layout(&device);
        let smaa_blend_layout = smaa_blend_layout(&device);
        let aa_layout = aa_layout(&device);
        let ui_layout = ui_layout(&device);
        let present_layout = present_layout(&device);
        let rain_layout = rain_layout(&device);

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera-uniform"),
            contents: bytemuck::bytes_of(&CameraUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("frame-uniform"),
            contents: bytemuck::bytes_of(&FrameUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let aa_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("anti-alias-uniform"),
            contents: bytemuck::bytes_of(&AaUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bind-group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let FlattenedScene {
            vertices,
            previous_positions,
            indices,
            batches,
            vertex_ranges,
        } = flatten_scene(&scene, !dynamic_game);
        let cloths = if dynamic_game {
            Vec::new()
        } else {
            scene
                .meshes
                .iter()
                .zip(vertex_ranges)
                .filter_map(|(mesh, vertex_range)| {
                    let definition = mesh.cloth.as_ref()?;
                    Some(
                        ClothState::new(&mesh.vertices, &mesh.indices, definition).map(|state| {
                            LiveCloth {
                                state,
                                mesh: mesh.clone(),
                                vertex_range,
                            }
                        }),
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| LiveError::Scene(error.to_string()))?
        };
        let dynamic_material_signatures = scene
            .materials
            .iter()
            .map(material_runtime_signature)
            .collect();
        let dynamic_environment_map = scene.environment_map.clone();
        let dynamic_light_layout = light_runtime_layout(&scene);
        let dynamic_local_shadow_kinds = local_shadow_kinds(&scene);
        let light_uniform = LightUniform::from_scene(&scene);
        let local_light_count = light_uniform.directional_direction_count[3]
            .clamp(0.0, MAX_GPU_POINT_LIGHTS as f32) as u32;
        let environment_map = scene.environment_map.clone();
        let (environment_texture, environment_view) = upload_environment_texture(
            &device,
            &queue,
            "lighting-skybox-panorama",
            &environment_map,
        );
        let environment_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lighting-skybox-linear-repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let light_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene-light-uniform"),
            contents: bytemuck::bytes_of(&light_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let material_profile_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dual-material-profile-table"),
                contents: bytemuck::bytes_of(&MaterialProfileUniform::from_scene(&scene)),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let exposure_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("auto-exposure-artist-settings"),
            contents: bytemuck::bytes_of(&exposure),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let velocity_compensation_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("velocity-compensation-artist-settings"),
                contents: bytemuck::bytes_of(&velocity_compensation),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let display_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("display-gamma-brightness-settings"),
            contents: bytemuck::bytes_of(&display),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let reflection_proxy_uniform = reflection_proxy_uniform(&scene);
        let reflection_proxy_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("reflection-proxy-uniform"),
                contents: bytemuck::bytes_of(&reflection_proxy_uniform),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let shadow_camera = shadow_camera_for_scene(light_uniform, &scene, &vertices);
        let shadow_camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow-camera-uniform"),
            contents: bytemuck::bytes_of(&shadow_camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-camera-bind-group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_camera_buffer.as_entire_binding(),
            }],
        });
        let (shadow_texture, shadow_view, shadow_sampler) =
            create_shadow_map(&device, SHADOW_MAP_RESOLUTION);
        let local_shadow_setup = local_shadow_setup_for_scene(&scene);
        let local_shadow_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("local-shadow-matrices"),
            contents: bytemuck::bytes_of(&local_shadow_setup.uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let (local_shadow_texture, local_shadow_array_view, local_shadow_views) =
            create_local_shadow_map_array(&device, LOCAL_SHADOW_MAP_RESOLUTION);
        let mut local_shadow_camera_buffers = Vec::with_capacity(MAX_GPU_POINT_LIGHTS);
        let mut local_shadow_bind_groups = Vec::with_capacity(MAX_GPU_POINT_LIGHTS);
        for camera in local_shadow_setup.cameras {
            let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("local-shadow-camera-uniform"),
                contents: bytemuck::bytes_of(&camera),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("local-shadow-camera-bind-group"),
                layout: &camera_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                }],
            });
            local_shadow_camera_buffers.push(camera_buffer);
            local_shadow_bind_groups.push(bind_group);
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let previous_position_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("scene-previous-positions"),
                contents: bytemuck::cast_slice(&previous_positions),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene-indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let materials = scene
            .materials
            .iter()
            .map(|material| create_material(&device, &queue, &material_layout, material))
            .collect();
        // Retire texture-upload staging memory before allocating the frame targets.
        queue.submit([]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| LiveError::Device(error.to_string()))?;
        let rain_streak_detail = generated_rain_streak_detail();
        let (rain_streak_texture, rain_streak_view) = upload_material_texture(
            &device,
            &queue,
            "rain-streak-detail",
            &rain_streak_detail,
            false,
        );
        let rain_streak_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rain-streak-detail-linear-repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let depth_pipeline = create_depth_pipeline(&device, &camera_layout);
        let shadow_pipeline = create_shadow_pipeline(&device, &camera_layout);
        let alpha_shadow_pipeline =
            create_alpha_shadow_pipeline(&device, &camera_layout, &material_layout);
        let geometry_pipeline =
            create_geometry_pipeline(&device, &camera_layout, &material_layout, config.format);
        let gbuffer_resolve_pipeline =
            create_gbuffer_resolve_pipeline(&device, &gbuffer_resolve_layout);
        let depth_copy_pipeline = create_depth_copy_pipeline(&device, &depth_copy_layout);
        let hiz_pipeline = create_hiz_pipeline(&device, &hiz_layout);
        let fog_pipeline = create_fog_pipeline(&device, &fog_layout);
        let ssr_pipeline = create_ssr_pipeline(&device, &ssr_layout);
        let reflection_pipeline = create_reflection_pipeline(&device, &reflection_layout);
        let lighting_pipeline = create_lighting_pipeline(&device, &lighting_layout);
        let light_volume_pipeline = create_light_volume_pipeline(&device, &light_volume_layout);
        let local_lighting_pipeline = create_local_lighting_pipeline(&device, &lighting_layout);
        let velocity_compensation_pipeline = create_velocity_compensation_pipeline(
            &device,
            &velocity_compensation_layout,
            "fs_compensate",
        );
        let velocity_smoothing_pipeline = create_velocity_compensation_pipeline(
            &device,
            &velocity_compensation_layout,
            "fs_smooth",
        );
        let auto_exposure_pipeline = create_auto_exposure_pipeline(&device, &auto_exposure_layout);
        let bloom_extract_pipeline =
            create_bloom_pipeline(&device, &bloom_layout, bloom_format, "fs_extract");
        let bloom_blur_horizontal_pipeline =
            create_bloom_pipeline(&device, &bloom_layout, bloom_format, "fs_blur_horizontal");
        let bloom_blur_vertical_pipeline =
            create_bloom_pipeline(&device, &bloom_layout, bloom_format, "fs_blur_vertical");
        let tonemap_pipeline = create_tonemap_pipeline(&device, &tonemap_layout, config.format);
        let smaa_edge_pipeline = create_smaa_edge_pipeline(&device, &smaa_edge_layout);
        let smaa_blend_pipeline = create_smaa_blend_pipeline(&device, &smaa_blend_layout);
        let rain_pipeline = create_rain_pipeline(&device, &rain_layout, HDR_FORMAT);
        let aa_pipeline = create_aa_pipeline(&device, &aa_layout, config.format);
        let ui_pipeline = create_ui_pipeline(&device, &ui_layout);
        let instance_transforms = create_instance_transforms(&device);
        let present_pipeline = create_present_pipeline(&device, &present_layout, config.format);
        let gbuffer = GBuffer::new(
            &device,
            config.width,
            config.height,
            config.format,
            bloom_format,
            &reflection_layout,
            &gbuffer_resolve_layout,
            &lighting_layout,
            &light_volume_layout,
            &velocity_compensation_layout,
            &auto_exposure_layout,
            &bloom_layout,
            &tonemap_layout,
            &depth_copy_layout,
            &hiz_layout,
            &fog_layout,
            &ssr_layout,
            &smaa_edge_layout,
            &smaa_blend_layout,
            &aa_layout,
            &ui_layout,
            &present_layout,
            &frame_buffer,
            &camera_buffer,
            &aa_buffer,
            &shadow_view,
            &shadow_sampler,
            &shadow_camera_buffer,
            &light_buffer,
            &material_profile_buffer,
            &exposure_buffer,
            &velocity_compensation_buffer,
            &display_buffer,
            &reflection_proxy_buffer,
            &local_shadow_array_view,
            &local_shadow_buffer,
            &environment_view,
            &environment_sampler,
        );
        let rain = create_rain_resources(
            &device,
            &rain_layout,
            RainPassResources {
                camera_buffer: &camera_buffer,
                streak_view: &rain_streak_view,
                streak_sampler: &rain_streak_sampler,
                light_buffer: &light_buffer,
                shadow_view: &shadow_view,
                shadow_sampler: &shadow_sampler,
                shadow_camera_buffer: &shadow_camera_buffer,
                skybox_view: &environment_view,
                skybox_sampler: &environment_sampler,
            },
            rain_streak_texture,
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            adapter: adapter_info,
            vertex_buffer,
            previous_position_buffer,
            // Games stream their world on the first update; the startup upload
            // only has to be valid until then.
            dynamic_instances: Vec::new(),
            instance_transforms,
            dynamic_layout: Vec::new(),
            dynamic_shadow_camera: shadow_camera,
            shadow_distance,
            dynamic_material_signatures,
            dynamic_environment_map,
            dynamic_light_layout,
            dynamic_local_shadow_kinds,
            index_buffer,
            batches,
            materials,
            camera_buffer,
            camera_bind_group,
            frame_buffer,
            aa_buffer,
            light_buffer,
            _material_profile_buffer: material_profile_buffer,
            _exposure_buffer: exposure_buffer,
            _velocity_compensation_buffer: velocity_compensation_buffer,
            _display_buffer: display_buffer,
            reflection_proxy_buffer,
            shadow_camera_buffer,
            shadow_camera_bind_group,
            _shadow_texture: shadow_texture,
            shadow_view,
            shadow_sampler,
            local_shadow_buffer,
            _local_shadow_texture: local_shadow_texture,
            local_shadow_array_view,
            local_shadow_views,
            _local_shadow_camera_buffers: local_shadow_camera_buffers,
            local_shadow_bind_groups,
            shadowed_local_light_indices: local_shadow_setup.shadowed_indices,
            shadow_cache_ready: false,
            local_shadow_cache_ready: vec![false; MAX_GPU_POINT_LIGHTS],
            depth_pipeline,
            shadow_pipeline,
            alpha_shadow_pipeline,
            geometry_pipeline,
            gbuffer_resolve_pipeline,
            depth_copy_pipeline,
            hiz_pipeline,
            fog_pipeline,
            ssr_pipeline,
            reflection_pipeline,
            lighting_pipeline,
            light_volume_pipeline,
            local_lighting_pipeline,
            velocity_compensation_pipeline,
            velocity_smoothing_pipeline,
            auto_exposure_pipeline,
            bloom_extract_pipeline,
            bloom_blur_horizontal_pipeline,
            bloom_blur_vertical_pipeline,
            tonemap_pipeline,
            aa_pipeline,
            ui_pipeline,
            present_pipeline,
            smaa_edge_pipeline,
            smaa_blend_pipeline,
            rain_pipeline,
            rain,
            reflection_layout,
            gbuffer_resolve_layout,
            lighting_layout,
            light_volume_layout,
            velocity_compensation_layout,
            auto_exposure_layout,
            bloom_layout,
            tonemap_layout,
            depth_copy_layout,
            hiz_layout,
            fog_layout,
            ssr_layout,
            aa_layout,
            ui_layout,
            present_layout,
            smaa_edge_layout,
            smaa_blend_layout,
            bloom_format,
            gbuffer,
            started: Instant::now(),
            last_update: Instant::now(),
            frame_index: 0,
            current_view_projection: Mat4::IDENTITY,
            previous_view_projection: Mat4::IDENTITY,
            auto_orbit: !static_camera && !dynamic_game,
            camera_origin,
            camera_target,
            camera_fov_y,
            camera_near,
            camera_far,
            keys: CameraKeys::default(),
            mouse_look: false,
            base_light_uniform: light_uniform,
            local_light_count,
            _environment_texture: environment_texture,
            environment_view,
            environment_sampler,
            display_fps: 0.0,
            show_ui: capture_path.is_none(),
            hud_revision: None,
            hud_pixels: Vec::new(),
            frame_capture: capture_path.map(|path| FrameCapture {
                path,
                written: false,
                frame: 30,
            }),
            gpu_profiler,
            cloths,
            cloth_accumulator: 0.0,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.hud_revision = None;
        self.gbuffer = GBuffer::new(
            &self.device,
            size.width,
            size.height,
            self.config.format,
            self.bloom_format,
            &self.reflection_layout,
            &self.gbuffer_resolve_layout,
            &self.lighting_layout,
            &self.light_volume_layout,
            &self.velocity_compensation_layout,
            &self.auto_exposure_layout,
            &self.bloom_layout,
            &self.tonemap_layout,
            &self.depth_copy_layout,
            &self.hiz_layout,
            &self.fog_layout,
            &self.ssr_layout,
            &self.smaa_edge_layout,
            &self.smaa_blend_layout,
            &self.aa_layout,
            &self.ui_layout,
            &self.present_layout,
            &self.frame_buffer,
            &self.camera_buffer,
            &self.aa_buffer,
            &self.shadow_view,
            &self.shadow_sampler,
            &self.shadow_camera_buffer,
            &self.light_buffer,
            &self._material_profile_buffer,
            &self._exposure_buffer,
            &self._velocity_compensation_buffer,
            &self._display_buffer,
            &self.reflection_proxy_buffer,
            &self.local_shadow_array_view,
            &self.local_shadow_buffer,
            &self.environment_view,
            &self.environment_sampler,
        );
    }

    /// Streams a game world into the GPU buffers. Static geometry is uploaded once;
    /// afterwards only entities whose transform changed (or that deform) are
    /// re-transformed and written back into their slice of the vertex buffer.
    fn update_world(&mut self, world: &World) -> Result<(), String> {
        let scene = world.build_lighting_scene();
        validate_live_scene(&scene).map_err(|error| error.to_string())?;
        let same_environment = Arc::ptr_eq(
            &scene.environment_map.panorama,
            &self.dynamic_environment_map.panorama,
        ) && scene.environment_map.encoding
            == self.dynamic_environment_map.encoding
            && scene.environment_map.exposure_ev() == self.dynamic_environment_map.exposure_ev()
            && scene.environment_map.yaw_radians() == self.dynamic_environment_map.yaw_radians()
            && scene.environment_map.irradiance_sh()
                == self.dynamic_environment_map.irradiance_sh();
        if !same_environment {
            return Err("live games cannot replace the environment map after startup".to_string());
        }
        if light_runtime_layout(&scene) != self.dynamic_light_layout {
            return Err("live games cannot change light topology after startup".to_string());
        }
        if local_shadow_kinds(&scene) != self.dynamic_local_shadow_kinds {
            return Err(
                "live games cannot move local shadow-casting lights after startup".to_string(),
            );
        }

        let instances = world.render_instances();
        let layout: Vec<_> = instances
            .iter()
            .map(|instance| (instance.entity, instance.revision))
            .collect();
        let mut shadow_casters_moved = false;
        if layout != self.dynamic_layout {
            let material_signatures: Vec<_> = world
                .materials
                .iter()
                .map(material_runtime_signature)
                .collect();
            if material_signatures != self.dynamic_material_signatures {
                return Err("live games cannot change the material table after startup".to_string());
            }
            self.rebuild_dynamic_geometry(&instances, &world.materials);
            self.dynamic_layout = layout;
            shadow_casters_moved = true;
        } else {
            shadow_casters_moved |= self.stream_moved_instances(&instances);
        }

        let proxies =
            instances
                .iter()
                .zip(&self.dynamic_instances)
                .filter_map(|(instance, cached)| {
                    let material = world.materials.get(instance.mesh.material_index)?;
                    if !instance.reflection_proxy && !material.flags.emissive {
                        return None;
                    }
                    let (min, max) = cached.local_bounds?;
                    let (min, max) = transform_bounds(min, max, &instance.transform);
                    Some((min, max, material))
                });
        self.queue.write_buffer(
            &self.reflection_proxy_buffer,
            0,
            bytemuck::bytes_of(&reflection_proxy_uniform_from(proxies)),
        );

        self.camera_origin = GpuVec3::new(
            scene.camera.position.x,
            scene.camera.position.y,
            scene.camera.position.z,
        );
        self.camera_target = GpuVec3::new(
            scene.camera.target.x,
            scene.camera.target.y,
            scene.camera.target.z,
        );
        self.camera_fov_y = scene.camera.vertical_fov_radians;
        self.camera_near = scene.camera.near;
        self.camera_far = scene.camera.far;
        self.base_light_uniform = LightUniform::from_scene(&scene);
        self.local_light_count = self.base_light_uniform.directional_direction_count[3]
            .clamp(0.0, MAX_GPU_POINT_LIGHTS as f32) as u32;

        let view_direction = (self.camera_target - self.camera_origin).normalize_or_zero();
        let focus = self.camera_origin + view_direction * (self.shadow_distance * 0.6);
        let shadow_camera =
            shadow_camera_around(self.base_light_uniform, focus, self.shadow_distance);
        if bytemuck::bytes_of(&shadow_camera) != bytemuck::bytes_of(&self.dynamic_shadow_camera) {
            self.queue.write_buffer(
                &self.shadow_camera_buffer,
                0,
                bytemuck::bytes_of(&shadow_camera),
            );
            self.dynamic_shadow_camera = shadow_camera;
            self.shadow_cache_ready = false;
        }
        if shadow_casters_moved {
            self.shadow_cache_ready = false;
            self.local_shadow_cache_ready.fill(false);
        }
        Ok(())
    }

    fn rebuild_dynamic_geometry(
        &mut self,
        instances: &[RenderInstance<'_>],
        materials: &[Material],
    ) {
        let items: Vec<_> = instances
            .iter()
            .map(|instance| FlattenItem {
                mesh: instance.mesh,
                transform: instance.transform,
                prepass_eligible: instance.prepass_eligible,
            })
            .collect();
        let flattened = flatten_items(&items, materials, None);
        self.index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dynamic-scene-indices"),
                contents: bytemuck::cast_slice(&flattened.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let streamed =
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE;
        self.vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dynamic-scene-vertices"),
                contents: bytemuck::cast_slice(&flattened.vertices),
                usage: streamed,
            });
        self.previous_position_buffer =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("dynamic-scene-previous-positions"),
                    contents: bytemuck::cast_slice(&flattened.previous_positions),
                    usage: streamed,
                });
        // Object-space copy in the same order, for GPU re-transforms.
        let mut local_vertices = vec![GpuVertex::zeroed(); flattened.vertices.len()];
        for (item, range) in items.iter().zip(&flattened.vertex_ranges) {
            for (slot, vertex) in local_vertices[range.start as usize..range.end as usize]
                .iter_mut()
                .zip(&item.mesh.vertices)
            {
                *slot = gpu_vertex(&crate::math::Mat4::IDENTITY, vertex);
            }
        }
        self.instance_transforms.resources = (!local_vertices.is_empty()).then(|| {
            create_instance_transform_resources(
                &self.device,
                &self.instance_transforms.layout,
                &local_vertices,
                &self.vertex_buffer,
                &self.previous_position_buffer,
                items.len().max(1),
            )
        });
        self.instance_transforms.jobs.clear();
        self.batches = flattened.batches;
        self.dynamic_instances = items
            .iter()
            .zip(flattened.vertex_ranges)
            .map(|(item, vertex_range)| DynamicInstance {
                vertex_range,
                transform: item.transform,
                moved_last_frame: false,
                casts_shadow: item.casts_shadow(),
                local_bounds: local_bounds(&item.mesh.vertices),
            })
            .collect();
    }

    /// Queues GPU re-transforms for entities that moved this frame or last
    /// frame (so their motion vectors settle). Deforming meshes are rebuilt on
    /// the CPU. Returns whether any shadow caster changed.
    fn stream_moved_instances(&mut self, instances: &[RenderInstance<'_>]) -> bool {
        let vertex_stride = std::mem::size_of::<GpuVertex>() as u64;
        let previous_stride = std::mem::size_of::<GpuPreviousPosition>() as u64;
        let mut shadow_casters_moved = false;
        self.instance_transforms.jobs.clear();
        for (instance, cached) in instances.iter().zip(&mut self.dynamic_instances) {
            let moved = instance.deforming || instance.transform != cached.transform;
            if !moved && !cached.moved_last_frame {
                continue;
            }
            if instance.deforming {
                let vertices: Vec<_> = instance
                    .mesh
                    .vertices
                    .iter()
                    .map(|vertex| gpu_vertex(&instance.transform, vertex))
                    .collect();
                let previous_positions: Vec<_> = instance
                    .mesh
                    .vertices
                    .iter()
                    .map(|vertex| gpu_previous_position(&instance.transform, vertex))
                    .collect();
                let start = cached.vertex_range.start as u64;
                self.queue.write_buffer(
                    &self.vertex_buffer,
                    start * vertex_stride,
                    bytemuck::cast_slice(&vertices),
                );
                self.queue.write_buffer(
                    &self.previous_position_buffer,
                    start * previous_stride,
                    bytemuck::cast_slice(&previous_positions),
                );
            } else {
                // Settling frames repeat the current pose so motion vectors go to zero.
                let previous = if moved {
                    cached.transform
                } else {
                    instance.transform
                };
                self.instance_transforms.jobs.push(TransformJob {
                    current: instance.transform.m,
                    previous: previous.m,
                    start: cached.vertex_range.start,
                    count: cached.vertex_range.end - cached.vertex_range.start,
                    _padding: [0; 2],
                });
            }
            cached.transform = instance.transform;
            cached.moved_last_frame = moved;
            shadow_casters_moved |= cached.casts_shadow;
        }
        if let Some(resources) = &self.instance_transforms.resources {
            let jobs = &self.instance_transforms.jobs;
            debug_assert!(jobs.len() <= resources.job_capacity);
            if !jobs.is_empty() {
                self.queue
                    .write_buffer(&resources.job_buffer, 0, bytemuck::cast_slice(jobs));
            }
        }
        shadow_casters_moved
    }

    fn upload_hud(&mut self, hud: &Hud) {
        if (hud.width(), hud.height()) != (self.config.width, self.config.height)
            || self.hud_revision == Some(hud.revision())
        {
            return;
        }
        let regions = changed_hud_regions(
            hud.pixels(),
            &self.hud_pixels,
            hud.width(),
            hud.height(),
            self.hud_revision.is_none(),
        );
        for [x, y, width, height] in regions {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.gbuffer.hud_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(hud.pixels()),
                wgpu::TexelCopyBufferLayout {
                    offset: ((y as u64 * hud.width() as u64 + x as u64) * 4),
                    bytes_per_row: Some(hud.width() * 4),
                    rows_per_image: Some(hud.height()),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.hud_pixels.clear();
        self.hud_pixels.extend_from_slice(hud.pixels());
        self.hud_revision = Some(hud.revision());
    }

    fn toggle_auto_orbit(&mut self) {
        if self.auto_orbit {
            self.enter_manual_control();
        } else {
            self.auto_orbit = true;
        }
    }

    fn set_key(&mut self, code: KeyCode, pressed: bool) {
        let handled = set_camera_key(&mut self.keys, code, pressed);
        if handled && pressed {
            self.enter_manual_control();
        }
    }

    fn set_mouse_look(&mut self, pressed: bool) {
        self.mouse_look = pressed;
        if pressed {
            self.enter_manual_control();
        }
    }

    fn apply_mouse_delta(&mut self, delta: (f64, f64)) {
        if !self.mouse_look {
            return;
        }
        self.enter_manual_control();
        let yaw = delta.0 as f32 * CAMERA_MOUSE_LOOK_SPEED;
        let pitch = -(delta.1 as f32) * CAMERA_MOUSE_LOOK_SPEED;
        rotate_camera_target(&mut self.camera_origin, &mut self.camera_target, yaw, pitch);
    }

    fn enter_manual_control(&mut self) {
        if self.auto_orbit {
            let elapsed = self.started.elapsed().as_secs_f32();
            let (eye, target) = self.camera_pose(elapsed);
            self.camera_origin = eye;
            self.camera_target = target;
            self.auto_orbit = false;
        }
    }

    fn camera_pose(&self, elapsed: f32) -> (GpuVec3, GpuVec3) {
        if self.auto_orbit {
            let sway = (elapsed * 0.22).sin() * 0.7;
            (
                self.camera_origin + GpuVec3::new(sway, (elapsed * 0.13).sin() * 0.035, 0.0),
                self.camera_target + GpuVec3::new(sway * 0.18, 0.0, 0.0),
            )
        } else {
            (self.camera_origin, self.camera_target)
        }
    }

    fn apply_manual_camera(&mut self, dt: f32) {
        if self.auto_orbit {
            return;
        }

        apply_camera_keys_to_pose(
            &mut self.camera_origin,
            &mut self.camera_target,
            self.keys,
            dt,
        );
    }

    fn update(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_update)
            .as_secs_f32()
            .clamp(0.0, 0.05);
        self.last_update = now;
        if dt > 0.0001 {
            let instant_fps = (1.0 / dt).clamp(0.0, 999.0);
            self.display_fps = if self.display_fps > 0.0 {
                self.display_fps * 0.90 + instant_fps * 0.10
            } else {
                instant_fps
            };
        }
        let elapsed = self.started.elapsed().as_secs_f32();
        self.update_cloth(dt)?;
        self.apply_manual_camera(dt);
        let (eye, target) = self.camera_pose(elapsed);
        let view = Mat4::look_at_rh(eye, target, GpuVec3::Y);
        let projection = perspective_reverse_rh(
            self.camera_fov_y,
            self.config.width as f32 / self.config.height as f32,
            self.camera_near,
            self.camera_far,
        );
        let view_projection = projection * view;
        self.previous_view_projection = if self.frame_index == 0 {
            view_projection
        } else {
            self.current_view_projection
        };
        self.current_view_projection = view_projection;
        let viewport = [
            self.config.width as f32,
            self.config.height as f32,
            1.0 / self.config.width as f32,
            1.0 / self.config.height as f32,
        ];
        let camera = CameraUniform {
            view_projection: view_projection.to_cols_array_2d(),
            inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
            position_time: [eye.x, eye.y, eye.z, elapsed],
            viewport,
            previous_view_projection: self.previous_view_projection.to_cols_array_2d(),
        };
        let frame = FrameUniform {
            dimensions: [
                self.config.width as f32,
                self.config.height as f32,
                self.gbuffer.reflection_width as f32,
                self.gbuffer.reflection_height as f32,
            ],
            counters: [self.frame_index, 0, 0, 0],
        };
        let aa = AaUniform {
            previous_view_projection: self.previous_view_projection.to_cols_array_2d(),
            viewport,
            params: [
                if self.frame_index == 0 { 0.0 } else { 0.16 },
                0.045,
                0.42,
                self.display_fps.clamp(0.0, 999.0),
            ],
        };
        let lights = self.base_light_uniform.realtime(elapsed);
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera));
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));
        self.queue
            .write_buffer(&self.aa_buffer, 0, bytemuck::bytes_of(&aa));
        self.queue
            .write_buffer(&self.light_buffer, 0, bytemuck::bytes_of(&lights));
        Ok(())
    }

    fn update_cloth(&mut self, dt: f32) -> Result<(), String> {
        const CLOTH_TIMESTEP: f32 = 1.0 / 60.0;
        self.cloth_accumulator = (self.cloth_accumulator + dt).min(CLOTH_TIMESTEP * 4.0);
        let mut stepped = false;
        while self.cloth_accumulator >= CLOTH_TIMESTEP {
            for cloth in &mut self.cloths {
                cloth
                    .state
                    .step(
                        CLOTH_TIMESTEP,
                        &[ClothColliderSnapshot::floor(-0.8)],
                        &mut cloth.mesh.vertices,
                    )
                    .map_err(|error| error.to_string())?;
            }
            self.cloth_accumulator -= CLOTH_TIMESTEP;
            stepped = true;
        }
        if !stepped {
            return Ok(());
        }

        let vertex_stride = std::mem::size_of::<GpuVertex>() as u64;
        let previous_position_stride = std::mem::size_of::<GpuPreviousPosition>() as u64;
        let mut invalidates_shadow_cache = false;
        for cloth in &self.cloths {
            let vertices: Vec<_> = cloth
                .mesh
                .vertices
                .iter()
                .map(|vertex| gpu_vertex(&cloth.mesh.transform, vertex))
                .collect();
            let previous_positions: Vec<_> = cloth
                .mesh
                .vertices
                .iter()
                .map(|vertex| gpu_previous_position(&cloth.mesh.transform, vertex))
                .collect();
            self.queue.write_buffer(
                &self.vertex_buffer,
                cloth.vertex_range.start as u64 * vertex_stride,
                bytemuck::cast_slice(&vertices),
            );
            self.queue.write_buffer(
                &self.previous_position_buffer,
                cloth.vertex_range.start as u64 * previous_position_stride,
                bytemuck::cast_slice(&previous_positions),
            );
            invalidates_shadow_cache |=
                cloth.mesh.shadow_class != crate::asset::ShadowCasterClass::None;
        }
        if invalidates_shadow_cache {
            self.shadow_cache_ready = false;
            self.local_shadow_cache_ready.fill(false);
        }
        Ok(())
    }

    fn render(&mut self) -> RenderResult {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return RenderResult::Skipped
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                return RenderResult::Reconfigure
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return RenderResult::Fatal("surface acquisition validation failure".to_string())
            }
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dhool-frame"),
            });
        if let Some(profiler) = &self.gpu_profiler {
            encoder.write_timestamp(&profiler.query_set, 0);
        }
        if let (Some(resources), false) = (
            &self.instance_transforms.resources,
            self.instance_transforms.jobs.is_empty(),
        ) {
            let jobs = &self.instance_transforms.jobs;
            let longest = jobs.iter().map(|job| job.count).max().unwrap_or(0);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("instance-transforms"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.instance_transforms.pipeline);
            pass.set_bind_group(0, &resources.bind_group, &[]);
            pass.dispatch_workgroups(longest.div_ceil(64), jobs.len() as u32, 1);
        }
        self.instance_transforms.jobs.clear();

        if !self.shadow_cache_ready {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("cached-directional-shadow-map"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(REVERSE_Z_CLEAR_DEPTH),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &self.shadow_camera_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for batch in &self.batches {
                    if batch.casts_shadow {
                        if batch.alpha_tested {
                            pass.set_pipeline(&self.alpha_shadow_pipeline);
                            pass.set_bind_group(
                                1,
                                &self.materials[batch.material_index].bind_group,
                                &[],
                            );
                        } else {
                            pass.set_pipeline(&self.shadow_pipeline);
                        }
                        pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                    }
                }
            }
            self.shadow_cache_ready = true;
        }

        for light_slot in 0..self.shadowed_local_light_indices.len() {
            let light_index = self.shadowed_local_light_indices[light_slot];
            if self
                .local_shadow_cache_ready
                .get(light_index)
                .copied()
                .unwrap_or(true)
            {
                continue;
            }
            let Some(view) = self.local_shadow_views.get(light_index) else {
                continue;
            };
            let Some(bind_group) = self.local_shadow_bind_groups.get(light_index) else {
                continue;
            };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("cached-local-shadow-map"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(REVERSE_Z_CLEAR_DEPTH),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for batch in &self.batches {
                    if batch.casts_shadow {
                        if batch.alpha_tested {
                            pass.set_pipeline(&self.alpha_shadow_pipeline);
                            pass.set_bind_group(
                                1,
                                &self.materials[batch.material_index].bind_group,
                                &[],
                            );
                        } else {
                            pass.set_pipeline(&self.shadow_pipeline);
                        }
                        pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                    }
                }
            }
            if let Some(cache_ready) = self.local_shadow_cache_ready.get_mut(light_index) {
                *cache_ready = true;
            }
        }

        if let Some(profiler) = &self.gpu_profiler {
            encoder.write_timestamp(&profiler.query_set, 1);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("partial-depth-prepass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.gbuffer.depth_ms_attachment,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(REVERSE_Z_CLEAR_DEPTH),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for batch in &self.batches {
                if batch.prepass_eligible {
                    pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                }
            }
        }

        {
            let color_attachments = [
                load_color_attachment(&self.gbuffer.albedo_ms),
                load_color_attachment(&self.gbuffer.normal_ms),
                load_color_attachment(&self.gbuffer.pbr_ms),
                color_attachment(&self.gbuffer.motion_ms, wgpu::Color::BLACK),
            ];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("base-pass-gbuffer"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.gbuffer.depth_ms_attachment,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.geometry_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.previous_position_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            // Alpha-tested geometry contributes coverage first without paying
            // for a duplicate alpha prepass. Remaining opaque geometry then
            // benefits from that depth, and prepassed expensive surfaces draw
            // last with an equal-depth test.
            for (alpha_tested, prepass_eligible) in [(true, false), (false, false), (false, true)] {
                for batch in self.batches.iter().filter(|batch| {
                    batch.alpha_tested == alpha_tested && batch.prepass_eligible == prepass_eligible
                }) {
                    pass.set_bind_group(1, &self.materials[batch.material_index].bind_group, &[]);
                    pass.draw_indexed(batch.indices.clone(), 0, 0..1);
                }
            }
        }

        {
            let color_attachments = [
                load_color_attachment(&self.gbuffer.albedo),
                load_color_attachment(&self.gbuffer.normal),
                load_color_attachment(&self.gbuffer.pbr),
                load_color_attachment(&self.gbuffer.motion),
                load_color_attachment(&self.gbuffer.coverage),
            ];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("2x-msaa-gbuffer-custom-coverage-resolve"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.gbuffer.depth_attachment,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(REVERSE_Z_CLEAR_DEPTH),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.gbuffer_resolve_pipeline);
            pass.set_bind_group(0, &self.gbuffer.gbuffer_resolve_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if let Some(profiler) = &self.gpu_profiler {
            encoder.write_timestamp(&profiler.query_set, 2);
        }

        {
            let color_attachments = [load_color_attachment(&self.gbuffer._depth_copy_view)];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth-copy-r32f-fullscreen"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth_copy_pipeline);
            pass.set_bind_group(0, &self.gbuffer.depth_copy_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hiz-generation-min-max-discontinuity"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.hiz_pipeline);
            pass.set_bind_group(0, &self.gbuffer.hiz_bind_group, &[]);
            pass.dispatch_workgroups(
                self.gbuffer.hiz_width.div_ceil(8),
                self.gbuffer.hiz_height.div_ceil(8),
                1,
            );
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("half-resolution-ssr-first-layer"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.ssr_pipeline);
            pass.set_bind_group(0, &self.gbuffer.ssr_bind_group, &[]);
            pass.dispatch_workgroups(
                self.gbuffer.ssr_width.div_ceil(8),
                self.gbuffer.ssr_height.div_ceil(8),
                1,
            );
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("blue-noise-volumetric-fog"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.fog_pipeline);
            pass.set_bind_group(0, &self.gbuffer.fog_bind_group, &[]);
            pass.dispatch_workgroups(
                self.gbuffer.fog_width.div_ceil(8),
                self.gbuffer.fog_height.div_ceil(8),
                1,
            );
        }

        {
            let (reflection_bind_group, reflection_history_write) = if self.frame_index & 1 == 0 {
                (
                    &self.gbuffer.reflection_bind_group_a,
                    &self.gbuffer._rt_history_b,
                )
            } else {
                (
                    &self.gbuffer.reflection_bind_group_b,
                    &self.gbuffer._rt_history_a,
                )
            };
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ssr-first-temporal-reflections"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reflection_pipeline);
            pass.set_bind_group(0, reflection_bind_group, &[]);
            pass.dispatch_workgroups(
                self.gbuffer.reflection_width.div_ceil(8),
                self.gbuffer.reflection_height.div_ceil(8),
                1,
            );
            drop(pass);
            encoder.copy_texture_to_texture(
                self.gbuffer._reflection.as_image_copy(),
                reflection_history_write.as_image_copy(),
                wgpu::Extent3d {
                    width: self.gbuffer.reflection_width,
                    height: self.gbuffer.reflection_height,
                    depth_or_array_layers: 1,
                },
            );
        }

        if self.local_light_count > 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("local-light-z-fail-stencil-volumes"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.gbuffer.depth_attachment,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.light_volume_pipeline);
            pass.set_bind_group(0, &self.gbuffer.light_volume_bind_group, &[]);
            pass.draw(0..36, 0..self.local_light_count);
        }

        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.gbuffer.lighting_hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hdr-deferred-lighting"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.lighting_pipeline);
            pass.set_bind_group(0, &self.gbuffer.lighting_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if self.local_light_count > 0 {
            let color_attachments = [load_color_attachment(&self.gbuffer.lighting_hdr)];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stencil-restricted-local-lighting"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.gbuffer.depth_attachment,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.local_lighting_pipeline);
            pass.set_bind_group(0, &self.gbuffer.lighting_bind_group, &[]);
            pass.set_stencil_reference(0);
            pass.draw(0..3, 0..1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("diffuse-luminance-auto-exposure-white-balance"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.auto_exposure_pipeline);
            let bind_group = if self.frame_index & 1 == 0 {
                &self.gbuffer.exposure_white_balance_bind_group_a
            } else {
                &self.gbuffer.exposure_white_balance_bind_group_b
            };
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.gbuffer.lighting_hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world-space-depth-tested-rain"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.gbuffer.depth_attachment,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.rain_pipeline);
            pass.set_bind_group(0, &self.rain.bind_group, &[]);
            pass.draw(0..RAIN_PRISM_VERTEX_COUNT, 0..RAIN_DROP_COUNT);
        }

        if let Some(profiler) = &self.gpu_profiler {
            encoder.write_timestamp(&profiler.query_set, 3);
        }

        for (label, target_view, pipeline, bind_group) in [
            (
                "single-direction-depth-aware-velocity-compensation",
                &self.gbuffer.velocity_compensation_a,
                &self.velocity_compensation_pipeline,
                &self.gbuffer.velocity_compensation_bind_group,
            ),
            (
                "separable-velocity-compensation-smoothing",
                &self.gbuffer.velocity_compensation_b,
                &self.velocity_smoothing_pipeline,
                &self.gbuffer.velocity_smoothing_bind_group,
            ),
        ] {
            let color_attachments = [load_color_attachment(target_view)];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.gbuffer.bloom_a,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("diffuse-bloom-extract"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.bloom_extract_pipeline);
            pass.set_bind_group(0, &self.gbuffer.bloom_extract_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        for (label, target, pipeline, bind_group) in [
            (
                "diffuse-gaussian-bloom-horizontal",
                &self.gbuffer.bloom_b,
                &self.bloom_blur_horizontal_pipeline,
                &self.gbuffer.bloom_blur_a_bind_group,
            ),
            (
                "diffuse-gaussian-bloom-vertical",
                &self.gbuffer.bloom_a,
                &self.bloom_blur_vertical_pipeline,
                &self.gbuffer.bloom_blur_b_bind_group,
            ),
        ] {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.gbuffer.tone_mapped,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gt-tonemap-bloom-resolve"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.tonemap_pipeline);
            let bind_group = if self.frame_index & 1 == 0 {
                &self.gbuffer.tonemap_bind_group_b
            } else {
                &self.gbuffer.tonemap_bind_group_a
            };
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if let Some(profiler) = &self.gpu_profiler {
            encoder.write_timestamp(&profiler.query_set, 4);
        }

        {
            let color_attachments = [color_attachment(
                &self.gbuffer.smaa_edges,
                wgpu::Color::BLACK,
            )];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smaa-edge-detect"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.smaa_edge_pipeline);
            pass.set_bind_group(0, &self.gbuffer.smaa_edge_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let color_attachments = [color_attachment(
                &self.gbuffer.smaa_blend,
                wgpu::Color::BLACK,
            )];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smaa-blend-weights"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.smaa_blend_pipeline);
            pass.set_bind_group(0, &self.gbuffer.smaa_blend_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let (aa_bind_group, history_write) = if self.frame_index & 1 == 0 {
                (&self.gbuffer.aa_bind_group_a, &self.gbuffer.aa_history_b)
            } else {
                (&self.gbuffer.aa_bind_group_b, &self.gbuffer.aa_history_a)
            };
            let color_attachments = [
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.gbuffer.aa_presented,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: history_write,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smaa-t2x-motion-vector-resolve"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.aa_pipeline);
            pass.set_bind_group(0, aa_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if let Some(profiler) = &self.gpu_profiler {
            encoder.write_timestamp(&profiler.query_set, 5);
        }

        {
            let color_attachments = [color_attachment(
                &self.gbuffer.ui_overlay,
                wgpu::Color::TRANSPARENT,
            )];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("separate-linear-ui-overlay"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if self.show_ui {
                pass.set_pipeline(&self.ui_pipeline);
                pass.set_bind_group(0, &self.gbuffer.ui_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        {
            let color_attachments = [load_color_attachment(&target)];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gamma-correct-scene-ui-presentation"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(0, &self.gbuffer.present_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if let Some(profiler) = &self.gpu_profiler {
            encoder.write_timestamp(&profiler.query_set, 6);
            encoder.resolve_query_set(
                &profiler.query_set,
                0..GPU_TIMESTAMP_COUNT,
                &profiler.resolve_buffer,
                0,
            );
            encoder.copy_buffer_to_buffer(
                &profiler.resolve_buffer,
                0,
                &profiler.readback_buffer,
                0,
                GPU_TIMESTAMP_COUNT as u64 * std::mem::size_of::<u64>() as u64,
            );
        }

        let pending_capture =
            if self.frame_capture.as_ref().is_some_and(|capture| {
                !capture.written && u64::from(self.frame_index) >= capture.frame
            }) {
                self.frame_capture.as_ref().map(|capture| {
                    copy_presented_frame_to_buffer(
                        &self.device,
                        &mut encoder,
                        &frame.texture,
                        self.config.width,
                        self.config.height,
                        self.config.format,
                        capture.path.clone(),
                    )
                })
            } else {
                None
            };

        self.queue.submit(Some(encoder.finish()));
        if let Some(profiler) = &self.gpu_profiler {
            match read_gpu_timestamps(&self.device, profiler) {
                Ok(milliseconds) => {
                    let total: f64 = milliseconds.iter().sum();
                    let stages = GPU_STAGE_NAMES
                        .iter()
                        .zip(milliseconds)
                        .map(|(name, milliseconds)| format!("{name}={milliseconds:.3}ms"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("GPU frame {total:.3}ms: {stages}");
                }
                Err(message) => {
                    frame.present();
                    return RenderResult::Fatal(format!(
                        "GPU timestamp readback failed: {message}"
                    ));
                }
            }
        }
        if let Some(pending_capture) = pending_capture {
            if let Err(message) = write_pending_frame_capture(&self.device, pending_capture) {
                frame.present();
                return RenderResult::Fatal(message);
            }
            if let Some(capture) = &mut self.frame_capture {
                capture.written = true;
            }
        }
        frame.present();
        self.frame_index = self.frame_index.wrapping_add(1);
        RenderResult::Presented
    }
}

fn validate_live_scene(scene: &RenderScene) -> Result<(), LiveError> {
    let directional_lights = scene
        .lights
        .iter()
        .filter(|light| matches!(light.kind, LightKind::Directional { .. }))
        .count();
    if directional_lights > 1 {
        return Err(LiveError::Scene(format!(
            "live renderer supports one directional light, got {directional_lights}"
        )));
    }
    let local_lights = scene
        .lights
        .iter()
        .filter(|light| matches!(light.kind, LightKind::Local { .. } | LightKind::Spot { .. }))
        .count();
    if local_lights > MAX_GPU_POINT_LIGHTS {
        return Err(LiveError::Scene(format!(
            "live renderer supports {MAX_GPU_POINT_LIGHTS} local lights, got {local_lights}"
        )));
    }
    Ok(())
}

fn light_runtime_layout(scene: &RenderScene) -> Vec<(u8, bool)> {
    scene
        .lights
        .iter()
        .map(|light| {
            let kind = match light.kind {
                LightKind::Directional { .. } => 0,
                LightKind::Local { .. } => 1,
                LightKind::Spot { .. } => 2,
            };
            (kind, light.casts_shadow)
        })
        .collect()
}

fn local_shadow_kinds(scene: &RenderScene) -> Vec<(usize, LightKind)> {
    scene
        .lights
        .iter()
        .enumerate()
        .filter(|(_, light)| {
            light.casts_shadow && !matches!(light.kind, LightKind::Directional { .. })
        })
        .map(|(index, light)| (index, light.kind))
        .collect()
}

fn material_runtime_signature(material: &Material) -> u64 {
    let mut hasher = DefaultHasher::new();
    material.name.hash(&mut hasher);
    (material.class as u8).hash(&mut hasher);
    material.material_id.hash(&mut hasher);
    material.base_color.r.to_bits().hash(&mut hasher);
    material.base_color.g.to_bits().hash(&mut hasher);
    material.base_color.b.to_bits().hash(&mut hasher);
    material.roughness.to_bits().hash(&mut hasher);
    material.metallic.to_bits().hash(&mut hasher);
    material.specular_intensity.to_bits().hash(&mut hasher);
    material.emission.r.to_bits().hash(&mut hasher);
    material.emission.g.to_bits().hash(&mut hasher);
    material.emission.b.to_bits().hash(&mut hasher);
    for texture in [
        &material.base_color_texture,
        &material.normal_texture,
        &material.pbr_texture,
    ] {
        texture
            .as_ref()
            .map(|image| (image.width, image.height, image.pixels.len()))
            .hash(&mut hasher);
    }
    hasher.finish()
}

fn copy_presented_frame_to_buffer(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    path: PathBuf,
) -> PendingFrameCapture {
    let bytes_per_pixel = 4;
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
    let buffer_size = padded_bytes_per_row as u64 * height as u64;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("live-frame-capture-readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    PendingFrameCapture {
        path,
        buffer,
        width,
        height,
        unpadded_bytes_per_row,
        padded_bytes_per_row,
        format,
    }
}

fn create_gpu_timestamp_profiler(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> GpuTimestampProfiler {
    let byte_size = GPU_TIMESTAMP_COUNT as u64 * std::mem::size_of::<u64>() as u64;
    let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("live-frame-stage-timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: GPU_TIMESTAMP_COUNT,
    });
    let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("live-frame-stage-timestamp-resolve"),
        size: byte_size,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("live-frame-stage-timestamp-readback"),
        size: byte_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    GpuTimestampProfiler {
        query_set,
        resolve_buffer,
        readback_buffer,
        timestamp_period: queue.get_timestamp_period(),
    }
}

fn read_gpu_timestamps(
    device: &wgpu::Device,
    profiler: &GpuTimestampProfiler,
) -> Result<[f64; 6], String> {
    let (sender, receiver) = mpsc::channel();
    profiler
        .readback_buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    receiver
        .recv()
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let mapped = profiler.readback_buffer.slice(..).get_mapped_range();
    let mut ticks = [0_u64; GPU_TIMESTAMP_COUNT as usize];
    for (index, bytes) in mapped.as_chunks::<8>().0.iter().enumerate() {
        ticks[index] = u64::from_ne_bytes(*bytes);
    }
    drop(mapped);
    profiler.readback_buffer.unmap();

    let mut milliseconds = [0.0_f64; 6];
    for index in 0..milliseconds.len() {
        let delta = ticks[index + 1].saturating_sub(ticks[index]);
        milliseconds[index] = delta as f64 * profiler.timestamp_period as f64 / 1_000_000.0;
    }
    Ok(milliseconds)
}

fn write_pending_frame_capture(
    device: &wgpu::Device,
    capture: PendingFrameCapture,
) -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    capture
        .buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    receiver
        .recv()
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let view = capture.buffer.slice(..).get_mapped_range();
    let mut rgba = vec![0_u8; (capture.width * capture.height * 4) as usize];
    for y in 0..capture.height as usize {
        let source_start = y * capture.padded_bytes_per_row as usize;
        let source_end = source_start + capture.unpadded_bytes_per_row as usize;
        let dest_start = y * capture.width as usize * 4;
        let dest_end = dest_start + capture.width as usize * 4;
        let source_row = &view[source_start..source_end];
        let dest_row = &mut rgba[dest_start..dest_end];
        match capture.format {
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
                let source_pixels = source_row.as_chunks::<4>().0;
                let dest_pixels = dest_row.as_chunks_mut::<4>().0;
                for (source, dest) in source_pixels.iter().zip(dest_pixels) {
                    dest[0] = source[2];
                    dest[1] = source[1];
                    dest[2] = source[0];
                    dest[3] = source[3];
                }
            }
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => {
                dest_row.copy_from_slice(source_row);
            }
            other => {
                return Err(format!("unsupported capture surface format {other:?}"));
            }
        }
    }
    drop(view);
    capture.buffer.unmap();

    if let Some(parent) = capture
        .path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    image::save_buffer(
        &capture.path,
        &rgba,
        capture.width,
        capture.height,
        image::ColorType::Rgba8,
    )
    .map_err(|error| error.to_string())?;
    println!("captured live frame {}", capture.path.display());
    Ok(())
}

impl GBuffer {
    #[allow(clippy::too_many_arguments)]
    fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        surface_format: wgpu::TextureFormat,
        bloom_format: wgpu::TextureFormat,
        reflection_layout: &wgpu::BindGroupLayout,
        gbuffer_resolve_layout: &wgpu::BindGroupLayout,
        lighting_layout: &wgpu::BindGroupLayout,
        light_volume_layout: &wgpu::BindGroupLayout,
        velocity_compensation_layout: &wgpu::BindGroupLayout,
        auto_exposure_layout: &wgpu::BindGroupLayout,
        bloom_layout: &wgpu::BindGroupLayout,
        tonemap_layout: &wgpu::BindGroupLayout,
        depth_copy_layout: &wgpu::BindGroupLayout,
        hiz_layout: &wgpu::BindGroupLayout,
        fog_layout: &wgpu::BindGroupLayout,
        ssr_layout: &wgpu::BindGroupLayout,
        smaa_edge_layout: &wgpu::BindGroupLayout,
        smaa_blend_layout: &wgpu::BindGroupLayout,
        aa_layout: &wgpu::BindGroupLayout,
        ui_layout: &wgpu::BindGroupLayout,
        present_layout: &wgpu::BindGroupLayout,
        frame_buffer: &wgpu::Buffer,
        camera_buffer: &wgpu::Buffer,
        aa_buffer: &wgpu::Buffer,
        shadow_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
        shadow_camera_buffer: &wgpu::Buffer,
        light_buffer: &wgpu::Buffer,
        material_profile_buffer: &wgpu::Buffer,
        exposure_buffer: &wgpu::Buffer,
        velocity_compensation_buffer: &wgpu::Buffer,
        display_buffer: &wgpu::Buffer,
        reflection_proxy_buffer: &wgpu::Buffer,
        local_shadow_view: &wgpu::TextureView,
        local_shadow_buffer: &wgpu::Buffer,
        skybox_view: &wgpu::TextureView,
        skybox_sampler: &wgpu::Sampler,
    ) -> Self {
        let (albedo_ms_texture, albedo_ms) = multisampled_render_texture(
            device,
            "2x-msaa-gbuffer-albedo",
            width,
            height,
            ALBEDO_FORMAT,
        );
        let (normal_ms_texture, normal_ms) = multisampled_render_texture(
            device,
            "2x-msaa-gbuffer-normal-flags",
            width,
            height,
            NORMAL_FORMAT,
        );
        let (pbr_ms_texture, pbr_ms) = multisampled_render_texture(
            device,
            "2x-msaa-gbuffer-pbr-material",
            width,
            height,
            PBR_FORMAT,
        );
        let (motion_ms_texture, motion_ms) = multisampled_render_texture(
            device,
            "2x-msaa-motion-vectors",
            width,
            height,
            MOTION_FORMAT,
        );
        let (depth_ms_texture, depth_ms) = multisampled_depth_texture(device, width, height);
        let depth_ms_attachment =
            depth_ms_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (albedo_texture, albedo) =
            render_texture(device, "gbuffer-albedo", width, height, ALBEDO_FORMAT);
        let (normal_texture, normal) =
            render_texture(device, "gbuffer-normal-flags", width, height, NORMAL_FORMAT);
        let (pbr_texture, pbr) =
            render_texture(device, "gbuffer-pbr-material", width, height, PBR_FORMAT);
        let (motion_texture, motion) =
            render_texture(device, "motion-vectors-rg16", width, height, MOTION_FORMAT);
        let (coverage_texture, coverage) = render_texture(
            device,
            "2x-msaa-resolved-sample-coverage",
            width,
            height,
            COVERAGE_FORMAT,
        );
        let (depth_texture, depth) = depth_texture(device, width, height);
        let depth_attachment = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (depth_copy_texture, depth_copy) =
            render_texture(device, "depth-copy-r32f", width, height, DEPTH_COPY_FORMAT);
        let hiz_width = width.div_ceil(HIZ_SCALE).max(1);
        let hiz_height = height.div_ceil(HIZ_SCALE).max(1);
        let (hiz_texture, hiz) = storage_texture(
            device,
            "hiz-min-max-discontinuity",
            hiz_width,
            hiz_height,
            HIZ_FORMAT,
        );
        let ssr_width = width.div_ceil(SSR_SCALE).max(1);
        let ssr_height = height.div_ceil(SSR_SCALE).max(1);
        let (ssr_texture, ssr) = storage_texture(
            device,
            "half-resolution-ssr",
            ssr_width,
            ssr_height,
            REFLECTION_FORMAT,
        );
        let fog_width = width.div_ceil(FOG_SCALE).max(1);
        let fog_height = height.div_ceil(FOG_SCALE).max(1);
        let (fog_texture, volumetric_fog) = storage_texture(
            device,
            "blue-noise-volumetric-fog",
            fog_width,
            fog_height,
            VOLUMETRIC_FOG_FORMAT,
        );
        let (lighting_texture, lighting_hdr) =
            render_texture(device, "hdr-lighting-rgba16f", width, height, HDR_FORMAT);
        let (velocity_texture_a, velocity_compensation_a) = render_texture(
            device,
            "single-direction-velocity-compensation-a",
            width,
            height,
            HDR_FORMAT,
        );
        let (velocity_texture_b, velocity_compensation_b) = render_texture(
            device,
            "separable-velocity-compensation-b",
            width,
            height,
            HDR_FORMAT,
        );
        let (exposure_texture_a, exposure_white_balance_a) = storage_texture(
            device,
            "auto-exposure-white-balance-history-a-1x1",
            1,
            1,
            HDR_FORMAT,
        );
        let (exposure_texture_b, exposure_white_balance_b) = storage_texture(
            device,
            "auto-exposure-white-balance-history-b-1x1",
            1,
            1,
            HDR_FORMAT,
        );
        let bloom_width = width.div_ceil(BLOOM_SCALE).max(1);
        let bloom_height = height.div_ceil(BLOOM_SCALE).max(1);
        let (bloom_texture_a, bloom_a) = render_texture(
            device,
            "kawase-bloom-a",
            bloom_width,
            bloom_height,
            bloom_format,
        );
        let (bloom_texture_b, bloom_b) = render_texture(
            device,
            "kawase-bloom-b",
            bloom_width,
            bloom_height,
            bloom_format,
        );
        let (tone_texture, tone_mapped) =
            render_texture(device, "tone-mapped-rgb10a2", width, height, surface_format);
        let (smaa_edge_texture, smaa_edges) = render_texture(
            device,
            "smaa-edge-mask-rg8",
            width,
            height,
            SMAA_EDGE_FORMAT,
        );
        let (smaa_blend_texture, smaa_blend) = render_texture(
            device,
            "smaa-blend-mask-rgba16",
            width,
            height,
            SMAA_BLEND_FORMAT,
        );
        let (history_texture_a, aa_history_a) =
            render_texture(device, "smaa-t2x-history-a", width, height, surface_format);
        let (history_texture_b, aa_history_b) =
            render_texture(device, "smaa-t2x-history-b", width, height, surface_format);
        let (aa_presented_texture, aa_presented) = render_texture(
            device,
            "anti-aliased-scene-presentation",
            width,
            height,
            surface_format,
        );
        let (ui_overlay_texture, ui_overlay) = render_texture(
            device,
            "separate-linear-ui-overlay",
            width,
            height,
            UI_FORMAT,
        );
        let hud_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("game-hud-canvas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let hud_view = hud_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let reflection_width = ((width * 7) / 10).max(1);
        let reflection_height = ((height * 7) / 10).max(1);
        let reflection_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("software-rt-reflection-70-percent"),
            size: wgpu::Extent3d {
                width: reflection_width,
                height: reflection_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: REFLECTION_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let reflection = reflection_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (rt_history_texture_a, rt_history_a) = reflection_history_texture(
            device,
            "two-frame-rt-history-a",
            reflection_width,
            reflection_height,
        );
        let (rt_history_texture_b, rt_history_b) = reflection_history_texture(
            device,
            "two-frame-rt-history-b",
            reflection_width,
            reflection_height,
        );
        let gbuffer_resolve_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("2x-msaa-gbuffer-custom-resolve-bind-group"),
            layout: gbuffer_resolve_layout,
            entries: &[
                texture_entry(0, &albedo_ms),
                texture_entry(1, &normal_ms),
                texture_entry(2, &pbr_ms),
                texture_entry(3, &motion_ms),
                texture_entry(4, &depth_ms),
            ],
        });
        let ssr_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("half-resolution-ssr-bind-group"),
            layout: ssr_layout,
            entries: &[
                texture_entry(0, &albedo),
                texture_entry(1, &normal),
                texture_entry(2, &pbr),
                texture_entry(3, &depth),
                texture_entry(4, &ssr),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: light_buffer.as_entire_binding(),
                },
            ],
        });
        let reflection_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection-bind-group-history-a"),
            layout: reflection_layout,
            entries: &[
                texture_entry(0, &albedo),
                texture_entry(1, &normal),
                texture_entry(2, &pbr),
                texture_entry(3, &depth),
                texture_entry(4, &reflection),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: light_buffer.as_entire_binding(),
                },
                texture_entry(8, &rt_history_a),
                texture_entry(9, &motion),
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: reflection_proxy_buffer.as_entire_binding(),
                },
                texture_entry(11, &ssr),
                texture_entry(12, skybox_view),
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::Sampler(skybox_sampler),
                },
            ],
        });
        let reflection_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection-bind-group-history-b"),
            layout: reflection_layout,
            entries: &[
                texture_entry(0, &albedo),
                texture_entry(1, &normal),
                texture_entry(2, &pbr),
                texture_entry(3, &depth),
                texture_entry(4, &reflection),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: light_buffer.as_entire_binding(),
                },
                texture_entry(8, &rt_history_b),
                texture_entry(9, &motion),
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: reflection_proxy_buffer.as_entire_binding(),
                },
                texture_entry(11, &ssr),
                texture_entry(12, skybox_view),
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::Sampler(skybox_sampler),
                },
            ],
        });
        let lighting_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lighting-bind-group"),
            layout: lighting_layout,
            entries: &[
                texture_entry(0, &albedo),
                texture_entry(1, &normal),
                texture_entry(2, &pbr),
                texture_entry(3, &depth),
                texture_entry(4, &reflection),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: camera_buffer.as_entire_binding(),
                },
                texture_entry(6, shadow_view),
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: shadow_camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: light_buffer.as_entire_binding(),
                },
                texture_entry(10, local_shadow_view),
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: local_shadow_buffer.as_entire_binding(),
                },
                texture_entry(12, skybox_view),
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::Sampler(skybox_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: material_profile_buffer.as_entire_binding(),
                },
            ],
        });
        let light_volume_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("local-light-stencil-volume-bind-group"),
            layout: light_volume_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: light_buffer.as_entire_binding(),
                },
            ],
        });
        let bloom_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("diffuse-gaussian-bloom-linear-clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let bloom_extract_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-extract-bind-group"),
            layout: bloom_layout,
            entries: &[
                texture_entry(0, &velocity_compensation_b),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&bloom_sampler),
                },
            ],
        });
        let velocity_compensation_bind_group =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("single-direction-velocity-compensation-bind-group"),
                layout: velocity_compensation_layout,
                entries: &[
                    texture_entry(0, &lighting_hdr),
                    texture_entry(1, &motion),
                    texture_entry(2, &depth),
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: camera_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: velocity_compensation_buffer.as_entire_binding(),
                    },
                ],
            });
        let velocity_smoothing_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("separable-velocity-compensation-bind-group"),
            layout: velocity_compensation_layout,
            entries: &[
                texture_entry(0, &velocity_compensation_a),
                texture_entry(1, &motion),
                texture_entry(2, &depth),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: velocity_compensation_buffer.as_entire_binding(),
                },
            ],
        });
        let exposure_white_balance_bind_group_a =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("auto-exposure-white-balance-history-a-to-b"),
                layout: auto_exposure_layout,
                entries: &[
                    texture_entry(0, &lighting_hdr),
                    texture_entry(1, &exposure_white_balance_b),
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: light_buffer.as_entire_binding(),
                    },
                    texture_entry(3, &exposure_white_balance_a),
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: exposure_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: aa_buffer.as_entire_binding(),
                    },
                ],
            });
        let exposure_white_balance_bind_group_b =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("auto-exposure-white-balance-history-b-to-a"),
                layout: auto_exposure_layout,
                entries: &[
                    texture_entry(0, &lighting_hdr),
                    texture_entry(1, &exposure_white_balance_a),
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: light_buffer.as_entire_binding(),
                    },
                    texture_entry(3, &exposure_white_balance_b),
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: exposure_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: aa_buffer.as_entire_binding(),
                    },
                ],
            });
        let bloom_blur_a_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-blur-a-bind-group"),
            layout: bloom_layout,
            entries: &[
                texture_entry(0, &bloom_a),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&bloom_sampler),
                },
            ],
        });
        let bloom_blur_b_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-blur-b-bind-group"),
            layout: bloom_layout,
            entries: &[
                texture_entry(0, &bloom_b),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&bloom_sampler),
                },
            ],
        });
        let tonemap_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap-bind-group-exposure-a"),
            layout: tonemap_layout,
            entries: &[
                texture_entry(0, &velocity_compensation_b),
                texture_entry(1, &bloom_a),
                texture_entry(2, &volumetric_fog),
                texture_entry(3, &exposure_white_balance_a),
            ],
        });
        let tonemap_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap-bind-group-exposure-b"),
            layout: tonemap_layout,
            entries: &[
                texture_entry(0, &velocity_compensation_b),
                texture_entry(1, &bloom_a),
                texture_entry(2, &volumetric_fog),
                texture_entry(3, &exposure_white_balance_b),
            ],
        });
        let depth_copy_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depth-copy-bind-group"),
            layout: depth_copy_layout,
            entries: &[texture_entry(0, &depth)],
        });
        let hiz_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hiz-bind-group"),
            layout: hiz_layout,
            entries: &[texture_entry(0, &depth_copy), texture_entry(1, &hiz)],
        });
        let fog_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("volumetric-fog-bind-group"),
            layout: fog_layout,
            entries: &[
                texture_entry(0, &depth_copy),
                texture_entry(1, &volumetric_fog),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: light_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: camera_buffer.as_entire_binding(),
                },
                texture_entry(5, skybox_view),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(skybox_sampler),
                },
            ],
        });
        let smaa_edge_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smaa-edge-bind-group"),
            layout: smaa_edge_layout,
            entries: &[
                texture_entry(0, &tone_mapped),
                texture_entry(1, &depth),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: aa_buffer.as_entire_binding(),
                },
                texture_entry(3, &coverage),
            ],
        });
        let smaa_blend_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smaa-blend-bind-group"),
            layout: smaa_blend_layout,
            entries: &[
                texture_entry(0, &tone_mapped),
                texture_entry(1, &smaa_edges),
                texture_entry(2, &depth),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: aa_buffer.as_entire_binding(),
                },
            ],
        });
        let aa_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smaa-t2x-bind-group-history-a"),
            layout: aa_layout,
            entries: &[
                texture_entry(0, &tone_mapped),
                texture_entry(1, &aa_history_a),
                texture_entry(2, &depth),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: aa_buffer.as_entire_binding(),
                },
                texture_entry(5, &motion),
                texture_entry(6, &smaa_blend),
                texture_entry(7, &normal),
            ],
        });
        let aa_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smaa-t2x-bind-group-history-b"),
            layout: aa_layout,
            entries: &[
                texture_entry(0, &tone_mapped),
                texture_entry(1, &aa_history_b),
                texture_entry(2, &depth),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: aa_buffer.as_entire_binding(),
                },
                texture_entry(5, &motion),
                texture_entry(6, &smaa_blend),
                texture_entry(7, &normal),
            ],
        });
        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("separate-ui-overlay-bind-group"),
            layout: ui_layout,
            entries: &[texture_entry(0, &hud_view)],
        });
        let present_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gamma-correct-ui-composite-bind-group"),
            layout: present_layout,
            entries: &[
                texture_entry(0, &aa_presented),
                texture_entry(1, &ui_overlay),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: display_buffer.as_entire_binding(),
                },
            ],
        });
        Self {
            _albedo_ms: albedo_ms_texture,
            _normal_ms: normal_ms_texture,
            _pbr_ms: pbr_ms_texture,
            _motion_ms: motion_ms_texture,
            _depth_ms: depth_ms_texture,
            _albedo: albedo_texture,
            _normal: normal_texture,
            _pbr: pbr_texture,
            _motion: motion_texture,
            _coverage: coverage_texture,
            _depth: depth_texture,
            _depth_copy: depth_copy_texture,
            _hiz: hiz_texture,
            _ssr: ssr_texture,
            _reflection: reflection_texture,
            _rt_history_a: rt_history_texture_a,
            _rt_history_b: rt_history_texture_b,
            _volumetric_fog: fog_texture,
            _lighting_hdr: lighting_texture,
            _velocity_compensation_a: velocity_texture_a,
            _velocity_compensation_b: velocity_texture_b,
            _exposure_white_balance_a: exposure_texture_a,
            _exposure_white_balance_b: exposure_texture_b,
            _bloom_a: bloom_texture_a,
            _bloom_b: bloom_texture_b,
            _tone_mapped: tone_texture,
            _smaa_edges: smaa_edge_texture,
            _smaa_blend: smaa_blend_texture,
            _aa_history_a: history_texture_a,
            _aa_history_b: history_texture_b,
            _aa_presented: aa_presented_texture,
            _ui_overlay: ui_overlay_texture,
            hud_texture,
            albedo,
            normal,
            pbr,
            motion,
            coverage,
            albedo_ms,
            normal_ms,
            pbr_ms,
            motion_ms,
            _depth_ms_view: depth_ms,
            depth_attachment,
            depth_ms_attachment,
            _depth_view: depth,
            _depth_copy_view: depth_copy,
            _hiz_view: hiz,
            _ssr_view: ssr,
            lighting_hdr,
            velocity_compensation_a,
            velocity_compensation_b,
            exposure_white_balance_bind_group_a,
            exposure_white_balance_bind_group_b,
            bloom_a,
            bloom_b,
            tone_mapped,
            _volumetric_fog_view: volumetric_fog,
            smaa_edges,
            smaa_blend,
            aa_history_a,
            aa_history_b,
            aa_presented,
            ui_overlay,
            reflection_bind_group_a,
            reflection_bind_group_b,
            lighting_bind_group,
            light_volume_bind_group,
            gbuffer_resolve_bind_group,
            velocity_compensation_bind_group,
            velocity_smoothing_bind_group,
            bloom_extract_bind_group,
            bloom_blur_a_bind_group,
            bloom_blur_b_bind_group,
            tonemap_bind_group_a,
            tonemap_bind_group_b,
            depth_copy_bind_group,
            hiz_bind_group,
            fog_bind_group,
            ssr_bind_group,
            smaa_edge_bind_group,
            smaa_blend_bind_group,
            aa_bind_group_a,
            aa_bind_group_b,
            ui_bind_group,
            present_bind_group,
            reflection_width,
            reflection_height,
            ssr_width,
            ssr_height,
            hiz_width,
            hiz_height,
            fog_width,
            fog_height,
        }
    }
}

async fn select_adapter(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, LiveError> {
    if let Ok(requested) = std::env::var("DHOOL_GPU") {
        let requested = requested.to_lowercase();
        if let Some(adapter) = instance
            .enumerate_adapters(wgpu::Backends::all())
            .await
            .into_iter()
            .find(|adapter| {
                adapter.is_surface_supported(surface)
                    && adapter.get_info().name.to_lowercase().contains(&requested)
            })
        {
            return Ok(adapter);
        }
        return Err(LiveError::Device(format!(
            "DHOOL_GPU={requested} did not match a present-capable adapter"
        )));
    }
    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(surface),
        })
        .await
        .map_err(|_| LiveError::AdapterUnavailable)
}

struct FlattenedScene {
    vertices: Vec<GpuVertex>,
    previous_positions: Vec<GpuPreviousPosition>,
    indices: Vec<u32>,
    batches: Vec<DrawBatch>,
    /// Vertex range of every input item, in input order.
    vertex_ranges: Vec<Range<u32>>,
}

struct FlattenItem<'a> {
    mesh: &'a crate::scene::Mesh,
    transform: crate::math::Mat4,
    prepass_eligible: bool,
}

impl<'a> FlattenItem<'a> {
    fn from_mesh(mesh: &'a crate::scene::Mesh) -> Self {
        Self {
            mesh,
            transform: mesh.transform,
            prepass_eligible: mesh.prepass_eligible,
        }
    }

    fn casts_shadow(&self) -> bool {
        !self.mesh.is_floor_or_ground
            && self.mesh.shadow_class != crate::asset::ShadowCasterClass::None
    }
}

fn flatten_scene(scene: &RenderScene, sort_for_static_scene: bool) -> FlattenedScene {
    let items: Vec<_> = scene.meshes.iter().map(FlattenItem::from_mesh).collect();
    let camera_position = GpuVec3::new(
        scene.camera.position.x,
        scene.camera.position.y,
        scene.camera.position.z,
    );
    flatten_items(
        &items,
        &scene.materials,
        sort_for_static_scene.then_some(camera_position),
    )
}

/// Bakes items into one world-space vertex stream, batching consecutive draws
/// that share material and pass eligibility. Static scenes may sort items
/// front-to-back from `sort_from` to help the depth prepass.
fn flatten_items(
    items: &[FlattenItem<'_>],
    materials: &[Material],
    sort_from: Option<GpuVec3>,
) -> FlattenedScene {
    let mut vertices = Vec::new();
    let mut previous_positions = Vec::new();
    let mut indices = Vec::new();
    let mut batches = Vec::new();
    let mut vertex_ranges = vec![0..0; items.len()];
    let mut order: Vec<usize> = (0..items.len()).collect();
    if let Some(camera_position) = sort_from {
        let classify = |item: &FlattenItem<'_>| {
            let alpha_tested = materials[item.mesh.material_index].flags.alpha_tested;
            (!alpha_tested, item.prepass_eligible && !alpha_tested)
        };
        let distance_squared = |item: &FlattenItem<'_>| {
            local_bounds(&item.mesh.vertices)
                .map(|(min, max)| {
                    let (min, max) = transform_bounds(min, max, &item.transform);
                    ((min + max) * 0.5).distance_squared(camera_position)
                })
                .unwrap_or(f32::INFINITY)
        };
        order.sort_by(|&left, &right| {
            let (left, right) = (&items[left], &items[right]);
            classify(left)
                .cmp(&classify(right))
                .then_with(|| distance_squared(left).total_cmp(&distance_squared(right)))
                .then_with(|| left.mesh.material_index.cmp(&right.mesh.material_index))
        });
    }
    let mut active_batch: Option<(usize, bool, bool, bool, u32)> = None;
    for index in order {
        let item = &items[index];
        let mesh = item.mesh;
        let material = &materials[mesh.material_index];
        let alpha_tested = material.flags.alpha_tested;
        let prepass_eligible = item.prepass_eligible && !alpha_tested;
        let casts_shadow = item.casts_shadow();
        let batch_key = (
            mesh.material_index,
            prepass_eligible,
            alpha_tested,
            casts_shadow,
        );

        if let Some((material_index, prepass_eligible, alpha_tested, casts_shadow, first_index)) =
            active_batch
        {
            if (material_index, prepass_eligible, alpha_tested, casts_shadow) != batch_key {
                batches.push(DrawBatch {
                    indices: first_index..indices.len() as u32,
                    material_index,
                    prepass_eligible,
                    alpha_tested,
                    casts_shadow,
                });
                active_batch = None;
            }
        }
        if active_batch.is_none() {
            active_batch = Some((
                mesh.material_index,
                prepass_eligible,
                alpha_tested,
                casts_shadow,
                indices.len() as u32,
            ));
        }

        let base_vertex = vertices.len() as u32;
        for vertex in &mesh.vertices {
            vertices.push(gpu_vertex(&item.transform, vertex));
            previous_positions.push(gpu_previous_position(&item.transform, vertex));
        }
        for triangle in &mesh.indices {
            indices.extend([
                triangle[0] + base_vertex,
                triangle[1] + base_vertex,
                triangle[2] + base_vertex,
            ]);
        }
        vertex_ranges[index] = base_vertex..vertices.len() as u32;
    }
    if let Some((material_index, prepass_eligible, alpha_tested, casts_shadow, first_index)) =
        active_batch
    {
        batches.push(DrawBatch {
            indices: first_index..indices.len() as u32,
            material_index,
            prepass_eligible,
            alpha_tested,
            casts_shadow,
        });
    }
    FlattenedScene {
        vertices,
        previous_positions,
        indices,
        batches,
        vertex_ranges,
    }
}

fn gpu_position(transform: &crate::math::Mat4, position: Vec3) -> GpuPreviousPosition {
    let position = transform.mul_vec4(Vec4::new(position.x, position.y, position.z, 1.0));
    GpuPreviousPosition {
        position: [position.x, position.y, position.z],
    }
}

fn gpu_vertex(transform: &crate::math::Mat4, vertex: &crate::scene::Vertex) -> GpuVertex {
    let position = transform.mul_vec4(Vec4::new(
        vertex.position.x,
        vertex.position.y,
        vertex.position.z,
        1.0,
    ));
    let normal = transform.mul_vec4(Vec4::new(
        vertex.normal.x,
        vertex.normal.y,
        vertex.normal.z,
        0.0,
    ));
    let normal = Vec3::new(normal.x, normal.y, normal.z).normalize_or_zero();
    let tangent = transform.mul_vec4(Vec4::new(
        vertex.tangent.x,
        vertex.tangent.y,
        vertex.tangent.z,
        0.0,
    ));
    let tangent = Vec3::new(tangent.x, tangent.y, tangent.z).normalize_or_zero();
    GpuVertex {
        position: [position.x, position.y, position.z],
        normal: [normal.x, normal.y, normal.z],
        uv: [vertex.uv.x, vertex.uv.y],
        color: [
            color_channel_to_unorm8(vertex.color.r),
            color_channel_to_unorm8(vertex.color.g),
            color_channel_to_unorm8(vertex.color.b),
            color_channel_to_unorm8(vertex.color.a),
        ],
        tangent: [tangent.x, tangent.y, tangent.z, vertex.tangent_sign],
    }
}

fn gpu_previous_position(
    transform: &crate::math::Mat4,
    vertex: &crate::scene::Vertex,
) -> GpuPreviousPosition {
    gpu_position(transform, vertex.previous_position)
}

fn local_bounds(vertices: &[crate::scene::Vertex]) -> Option<(GpuVec3, GpuVec3)> {
    let mut min = GpuVec3::splat(f32::INFINITY);
    let mut max = GpuVec3::splat(f32::NEG_INFINITY);
    for vertex in vertices {
        let position = GpuVec3::new(vertex.position.x, vertex.position.y, vertex.position.z);
        min = min.min(position);
        max = max.max(position);
    }
    (min.is_finite() && max.is_finite()).then_some((min, max))
}

/// World-space box enclosing a transformed local box.
fn transform_bounds(
    min: GpuVec3,
    max: GpuVec3,
    transform: &crate::math::Mat4,
) -> (GpuVec3, GpuVec3) {
    let mut world_min = GpuVec3::splat(f32::INFINITY);
    let mut world_max = GpuVec3::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let local = Vec4::new(
            if corner & 1 == 0 { min.x } else { max.x },
            if corner & 2 == 0 { min.y } else { max.y },
            if corner & 4 == 0 { min.z } else { max.z },
            1.0,
        );
        let world = transform.mul_vec4(local);
        let world = GpuVec3::new(world.x, world.y, world.z);
        world_min = world_min.min(world);
        world_max = world_max.max(world);
    }
    (world_min, world_max)
}

fn color_channel_to_unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn reflection_proxy_uniform(scene: &RenderScene) -> ReflectionProxyUniform {
    reflection_proxy_uniform_from(scene.meshes.iter().filter_map(|mesh| {
        let material = scene.materials.get(mesh.material_index)?;
        if !mesh.reflection_proxy && !material.flags.emissive {
            return None;
        }
        let (min, max) = local_bounds(&mesh.vertices)?;
        let (min, max) = transform_bounds(min, max, &mesh.transform);
        Some((min, max, material))
    }))
}

fn reflection_proxy_uniform_from<'a>(
    proxies: impl Iterator<Item = (GpuVec3, GpuVec3, &'a Material)>,
) -> ReflectionProxyUniform {
    let mut uniform = ReflectionProxyUniform::zeroed();
    let mut count = 0_usize;
    for (min, max, material) in proxies.take(MAX_REFLECTION_PROXIES) {
        let half_extent = ((max - min) * 0.5).max(GpuVec3::splat(0.04));
        let center = (min + max) * 0.5;
        let min = center - half_extent;
        let max = center + half_extent;
        let emission = material.emission;
        let has_emission = material.flags.emissive || emission.r + emission.g + emission.b > 0.0;
        let radiance = if has_emission {
            [emission.r, emission.g, emission.b, 1.0]
        } else {
            [
                material.base_color.r * (0.35 + material.specular_intensity),
                material.base_color.g * (0.35 + material.specular_intensity),
                material.base_color.b * (0.35 + material.specular_intensity),
                0.35,
            ]
        };
        uniform.bounds_min[count] = [min.x, min.y, min.z, 0.0];
        uniform.bounds_max[count] = [max.x, max.y, max.z, 0.0];
        uniform.radiance[count] = radiance;
        count += 1;
    }
    uniform.count[0] = count as u32;
    uniform
}

/// Directional shadow frustum centred on `focus`, snapped to whole shadow-map
/// texels so a moving camera does not make shadow edges shimmer.
fn shadow_camera_around(lights: LightUniform, focus: GpuVec3, radius: f32) -> CameraUniform {
    let surface_to_light = lights.surface_to_directional_light();
    let surface_to_light = if surface_to_light.length_squared() > 0.0 {
        surface_to_light
    } else {
        GpuVec3::new(0.35, 0.75, 0.45).normalize()
    };
    let up = if surface_to_light.y.abs() > 0.99 {
        GpuVec3::Z
    } else {
        GpuVec3::Y
    };
    let right = up.cross(surface_to_light).normalize();
    let light_up = surface_to_light.cross(right);
    let texel = radius * 2.0 / SHADOW_MAP_RESOLUTION as f32;
    let snap = |value: f32| (value / texel).round() * texel;
    let focus = right * snap(focus.dot(right))
        + light_up * snap(focus.dot(light_up))
        + surface_to_light * focus.dot(surface_to_light);
    let depth = radius * 4.0;
    let eye = focus + surface_to_light * depth;
    let view = Mat4::look_at_rh(eye, focus, up);
    let projection = orthographic_reverse_rh(-radius, radius, -radius, radius, 0.1, depth * 2.0);
    let view_projection = projection * view;
    CameraUniform {
        view_projection: view_projection.to_cols_array_2d(),
        inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
        position_time: [eye.x, eye.y, eye.z, 0.0],
        viewport: [
            SHADOW_MAP_RESOLUTION as f32,
            SHADOW_MAP_RESOLUTION as f32,
            1.0 / SHADOW_MAP_RESOLUTION as f32,
            1.0 / SHADOW_MAP_RESOLUTION as f32,
        ],
        previous_view_projection: view_projection.to_cols_array_2d(),
    }
}

fn shadow_camera_for_scene(
    lights: LightUniform,
    scene: &RenderScene,
    vertices: &[GpuVertex],
) -> CameraUniform {
    let surface_to_light = lights.surface_to_directional_light();
    let surface_to_light = if surface_to_light.length_squared() > 0.0 {
        surface_to_light
    } else {
        GpuVec3::new(0.35, 0.75, 0.45).normalize()
    };
    let focus = GpuVec3::new(
        scene.camera.target.x,
        scene.camera.target.y,
        scene.camera.target.z,
    );
    let focus_radius = (scene.camera.far * 0.22).clamp(8.0, 120.0);
    let mut min = GpuVec3::splat(f32::INFINITY);
    let mut max = GpuVec3::splat(f32::NEG_INFINITY);
    let mut local_vertex_count = 0usize;
    for vertex in vertices {
        let position = GpuVec3::from_array(vertex.position);
        if position.distance(focus) > focus_radius {
            continue;
        }
        min = min.min(position);
        max = max.max(position);
        local_vertex_count += 1;
    }
    if local_vertex_count == 0 {
        min = GpuVec3::new(-8.0, -1.0, -18.0);
        max = GpuVec3::new(8.0, 6.0, 6.0);
    }
    let center = (min + max) * 0.5;
    let radius = ((max - min).length() * 0.52).clamp(8.0, focus_radius);
    let eye = center + surface_to_light * (radius * 2.4);
    let view = Mat4::look_at_rh(eye, center, GpuVec3::Y);
    let projection = orthographic_reverse_rh(-radius, radius, -radius, radius, 0.1, radius * 4.8);
    let view_projection = projection * view;
    CameraUniform {
        view_projection: view_projection.to_cols_array_2d(),
        inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
        position_time: [eye.x, eye.y, eye.z, 0.0],
        viewport: [
            SHADOW_MAP_RESOLUTION as f32,
            SHADOW_MAP_RESOLUTION as f32,
            1.0 / SHADOW_MAP_RESOLUTION as f32,
            1.0 / SHADOW_MAP_RESOLUTION as f32,
        ],
        previous_view_projection: view_projection.to_cols_array_2d(),
    }
}

struct LocalShadowSetup {
    uniform: LocalShadowUniform,
    cameras: [CameraUniform; MAX_GPU_POINT_LIGHTS],
    shadowed_indices: Vec<usize>,
}

fn local_shadow_setup_for_scene(scene: &RenderScene) -> LocalShadowSetup {
    let mut uniform = LocalShadowUniform::zeroed();
    let mut cameras = [CameraUniform::zeroed(); MAX_GPU_POINT_LIGHTS];
    let mut shadowed_indices = Vec::new();
    let mut local_index = 0_usize;

    for light in &scene.lights {
        match light.kind {
            LightKind::Directional { .. } => {}
            LightKind::Local { .. } => {
                local_index += 1;
            }
            LightKind::Spot {
                position,
                direction,
                radius,
                outer_angle_radians,
                ..
            } => {
                if local_index < MAX_GPU_POINT_LIGHTS && light.casts_shadow {
                    let camera =
                        spot_shadow_camera(position, direction, radius, outer_angle_radians);
                    uniform.view_projection[local_index] = camera.view_projection;
                    cameras[local_index] = camera;
                    shadowed_indices.push(local_index);
                }
                local_index += 1;
            }
        }

        if local_index >= MAX_GPU_POINT_LIGHTS {
            break;
        }
    }

    LocalShadowSetup {
        uniform,
        cameras,
        shadowed_indices,
    }
}

fn spot_shadow_camera(
    position: crate::math::Vec3,
    direction: crate::math::Vec3,
    radius: f32,
    outer_angle_radians: f32,
) -> CameraUniform {
    let eye = GpuVec3::new(position.x, position.y, position.z);
    let direction = GpuVec3::new(direction.x, direction.y, direction.z).normalize_or_zero();
    let direction = if direction.length_squared() > 0.0 {
        direction
    } else {
        -GpuVec3::Z
    };
    let up = if direction.dot(GpuVec3::Y).abs() > 0.92 {
        GpuVec3::Z
    } else {
        GpuVec3::Y
    };
    let view = Mat4::look_at_rh(eye, eye + direction, up);
    let fov = (outer_angle_radians * 2.0).clamp(20.0_f32.to_radians(), 135.0_f32.to_radians());
    let projection = perspective_reverse_rh(fov, 1.0, 0.05, radius.max(0.1));
    let view_projection = projection * view;
    CameraUniform {
        view_projection: view_projection.to_cols_array_2d(),
        inverse_view_projection: view_projection.inverse().to_cols_array_2d(),
        position_time: [eye.x, eye.y, eye.z, 0.0],
        viewport: [
            LOCAL_SHADOW_MAP_RESOLUTION as f32,
            LOCAL_SHADOW_MAP_RESOLUTION as f32,
            1.0 / LOCAL_SHADOW_MAP_RESOLUTION as f32,
            1.0 / LOCAL_SHADOW_MAP_RESOLUTION as f32,
        ],
        previous_view_projection: view_projection.to_cols_array_2d(),
    }
}

fn perspective_reverse_rh(fov_y_radians: f32, aspect_ratio: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y_radians * 0.5).tan();
    let inverse_depth_range = 1.0 / (far - near);
    Mat4::from_cols_array_2d(&[
        [f / aspect_ratio, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, near * inverse_depth_range, -1.0],
        [0.0, 0.0, far * near * inverse_depth_range, 0.0],
    ])
}

fn orthographic_reverse_rh(
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    near: f32,
    far: f32,
) -> Mat4 {
    Mat4::orthographic_rh(left, right, bottom, top, far, near)
}

fn create_material(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    material: &Material,
) -> GpuMaterial {
    let emission_strength = material
        .emission
        .r
        .max(material.emission.g)
        .max(material.emission.b);
    let material_flag = u32::from(material.flags.glass)
        | (u32::from(material.flags.responsive_aa) << 1)
        | (u32::from(material.flags.alpha_tested) << 2)
        | (u32::from(material.material_id) << 3);
    let uniform = MaterialUniform {
        base_color: [
            material.base_color.r,
            material.base_color.g,
            material.base_color.b,
            material.base_color.a,
        ],
        surface: [
            material.roughness,
            material.metallic,
            material.cavity,
            material_flag as f32,
        ],
        emission: [
            material.emission.r,
            material.emission.g,
            material.emission.b,
            emission_strength,
        ],
    };
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{}-uniform", material.name)),
        contents: bytemuck::bytes_of(&uniform),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let white = solid_texture(crate::math::Color::WHITE);
    let flat_normal = solid_texture(crate::math::Color::new(0.5, 0.5, 1.0, 1.0));
    let default_pbr = solid_texture(crate::math::Color::new(1.0, 1.0, 0.0, 1.0));
    let (base_texture, base_view) = upload_material_texture(
        device,
        queue,
        &format!("{}-base-color", material.name),
        material.base_color_texture.as_deref().unwrap_or(&white),
        true,
    );
    let (normal_texture, normal_view) = upload_material_texture(
        device,
        queue,
        &format!("{}-normal", material.name),
        material.normal_texture.as_deref().unwrap_or(&flat_normal),
        false,
    );
    let (pbr_texture, pbr_view) = upload_material_texture(
        device,
        queue,
        &format!("{}-pbr", material.name),
        material.pbr_texture.as_deref().unwrap_or(&default_pbr),
        false,
    );
    let opacity_source = material.base_color_texture.as_deref().unwrap_or(&white);
    let (opacity_texture, opacity_view) = upload_opacity_texture(
        device,
        queue,
        &format!("{}-opacity-bc4-equivalent-r8", material.name),
        opacity_source,
        material.base_color.a,
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("material-linear-repeat"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..wgpu::SamplerDescriptor::default()
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("{}-bind-group", material.name)),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            texture_entry(1, &base_view),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            texture_entry(3, &normal_view),
            texture_entry(4, &pbr_view),
            texture_entry(5, &opacity_view),
        ],
    });
    GpuMaterial {
        _uniform: uniform_buffer,
        _textures: vec![base_texture, normal_texture, pbr_texture, opacity_texture],
        bind_group,
    }
}

fn create_rain_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    resources: RainPassResources<'_>,
    streak_texture: wgpu::Texture,
) -> RainTextures {
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("world-space-rain-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: resources.camera_buffer.as_entire_binding(),
            },
            texture_entry(1, resources.streak_view),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(resources.streak_sampler),
            },
            texture_entry(3, resources.shadow_view),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(resources.shadow_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: resources.shadow_camera_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: resources.light_buffer.as_entire_binding(),
            },
            texture_entry(7, resources.skybox_view),
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::Sampler(resources.skybox_sampler),
            },
        ],
    });
    RainTextures {
        _streak_texture: streak_texture,
        bind_group,
    }
}

fn solid_texture(color: crate::math::Color) -> TextureImage {
    TextureImage {
        width: 1,
        height: 1,
        pixels: vec![color].into(),
    }
}

fn upload_material_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texture: &TextureImage,
    srgb: bool,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (mip_count, bytes) = build_mip_chain(texture);
    let gpu_texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: texture.width,
                height: texture.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if srgb {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::MipMajor,
        &bytes,
    );
    let view = gpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
    (gpu_texture, view)
}

fn upload_environment_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    environment_map: &EnvironmentMap,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (mip_count, bytes) = build_environment_mip_chain(environment_map);
    let gpu_texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: environment_map.panorama.width,
                height: environment_map.panorama.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::MipMajor,
        &bytes,
    );
    let view = gpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
    (gpu_texture, view)
}

fn build_environment_mip_chain(environment_map: &EnvironmentMap) -> (u32, Vec<u8>) {
    let mut width = environment_map.panorama.width.max(1);
    let mut height = environment_map.panorama.height.max(1);
    let mut level: Vec<[f32; 4]> = (0..(width * height) as usize)
        .map(|index| {
            let radiance = environment_map.linear_texel(index);
            [radiance.x, radiance.y, radiance.z, 1.0]
        })
        .collect();
    let mut bytes = Vec::with_capacity(level.len() * 10);
    let mut mip_count = 0;
    loop {
        for texel in &level {
            for channel in texel {
                bytes.extend_from_slice(&f16::from_f32(*channel).to_bits().to_le_bytes());
            }
        }
        mip_count += 1;
        if width == 1 && height == 1 {
            break;
        }
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        let mut next = Vec::with_capacity((next_width * next_height) as usize);
        for y in 0..next_height {
            for x in 0..next_width {
                let mut sum = [0.0_f32; 4];
                for offset_y in 0..2 {
                    for offset_x in 0..2 {
                        let source_x = (x * 2 + offset_x).min(width - 1);
                        let source_y = (y * 2 + offset_y).min(height - 1);
                        let sample = level[(source_y * width + source_x) as usize];
                        for channel in 0..4 {
                            sum[channel] += sample[channel];
                        }
                    }
                }
                next.push(sum.map(|channel| channel * 0.25));
            }
        }
        level = next;
        width = next_width;
        height = next_height;
    }
    (mip_count, bytes)
}

fn upload_opacity_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texture: &TextureImage,
    material_opacity: f32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (mip_count, bytes) = build_opacity_mip_chain(texture, material_opacity);
    let gpu_texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: texture.width.max(1),
                height: texture.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::MipMajor,
        &bytes,
    );
    let view = gpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
    (gpu_texture, view)
}

fn build_opacity_mip_chain(texture: &TextureImage, material_opacity: f32) -> (u32, Vec<u8>) {
    let mut width = texture.width.max(1);
    let mut height = texture.height.max(1);
    let fallback = (material_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut level: Vec<u8> = if texture.pixels.is_empty() {
        vec![fallback; (width * height) as usize]
    } else {
        texture
            .pixels
            .alpha()
            .into_iter()
            .map(|alpha| (alpha * material_opacity).clamp(0.0, 1.0))
            .map(|opacity| (opacity * 255.0).round() as u8)
            .collect()
    };
    let mut bytes = Vec::with_capacity(level.len() * 2);
    let mut mip_count = 0;
    loop {
        bytes.extend_from_slice(&level);
        mip_count += 1;
        if width == 1 && height == 1 {
            break;
        }
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        let mut next = Vec::with_capacity((next_width * next_height) as usize);
        for y in 0..next_height {
            for x in 0..next_width {
                let mut sum = 0_u32;
                let mut samples = 0_u32;
                for offset_y in 0..2 {
                    for offset_x in 0..2 {
                        let source_x = (x * 2 + offset_x).min(width - 1);
                        let source_y = (y * 2 + offset_y).min(height - 1);
                        sum += level[(source_y * width + source_x) as usize] as u32;
                        samples += 1;
                    }
                }
                next.push((sum / samples) as u8);
            }
        }
        level = next;
        width = next_width;
        height = next_height;
    }
    (mip_count, bytes)
}

fn build_mip_chain(texture: &TextureImage) -> (u32, Vec<u8>) {
    let mut width = texture.width.max(1);
    let mut height = texture.height.max(1);
    let mut level = texture.pixels.to_rgba8();
    let mut bytes = Vec::with_capacity(level.len() * 5);
    let mut mip_count = 0;
    loop {
        bytes.extend(level.iter().flatten());
        mip_count += 1;
        if width == 1 && height == 1 {
            break;
        }
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        let mut next = Vec::with_capacity((next_width * next_height) as usize);
        for y in 0..next_height {
            for x in 0..next_width {
                let mut sum = [0_u32; 4];
                let mut samples = 0_u32;
                for oy in 0..2 {
                    for ox in 0..2 {
                        let source_x = (x * 2 + ox).min(width - 1);
                        let source_y = (y * 2 + oy).min(height - 1);
                        let pixel = level[(source_y * width + source_x) as usize];
                        for channel in 0..4 {
                            sum[channel] += pixel[channel] as u32;
                        }
                        samples += 1;
                    }
                }
                next.push([
                    (sum[0] / samples) as u8,
                    (sum[1] / samples) as u8,
                    (sum[2] / samples) as u8,
                    (sum[3] / samples) as u8,
                ]);
            }
        }
        level = next;
        width = next_width;
        height = next_height;
    }
    (mip_count, bytes)
}

fn create_depth_pipeline(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/depth_prepass.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("depth-prepass-pipeline-layout"),
        bind_group_layouts: &[Some(camera_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("partial-depth-prepass-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[GpuVertex::layout()],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MAIN_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: GBUFFER_SAMPLE_COUNT,
            ..wgpu::MultisampleState::default()
        },
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}

fn create_shadow_pipeline(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/depth_prepass.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow-pipeline-layout"),
        bind_group_layouts: &[Some(camera_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("directional-shadow-map-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[GpuVertex::layout()],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Front),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: -1,
                slope_scale: -1.25,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}

fn create_alpha_shadow_pipeline(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
    material_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/alpha_shadow.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("alpha-tested-shadow-pipeline-layout"),
        bind_group_layouts: &[Some(camera_layout), Some(material_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("alpha-tested-shadow-map-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[GpuVertex::layout()],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Front),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: -1,
                slope_scale: -1.25,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_geometry_pipeline(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
    material_layout: &wgpu::BindGroupLayout,
    _surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/geometry.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("geometry-pipeline-layout"),
        bind_group_layouts: &[Some(camera_layout), Some(material_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("deferred-gbuffer-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[GpuVertex::layout(), GpuPreviousPosition::layout()],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MAIN_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: GBUFFER_SAMPLE_COUNT,
            ..wgpu::MultisampleState::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(translucent_albedo_target(ALBEDO_FORMAT)),
                Some(color_target(NORMAL_FORMAT)),
                Some(color_target(PBR_FORMAT)),
                Some(color_target(MOTION_FORMAT)),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_depth_copy_pipeline(
    device: &wgpu::Device,
    depth_copy_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/depth_copy.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("depth-copy-pipeline-layout"),
        bind_group_layouts: &[Some(depth_copy_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("depth-copy-r32f-fullscreen"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(DEPTH_COPY_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_gbuffer_resolve_pipeline(
    device: &wgpu::Device,
    resolve_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/gbuffer_resolve.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("2x-msaa-gbuffer-custom-resolve-pipeline-layout"),
        bind_group_layouts: &[Some(resolve_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("2x-msaa-gbuffer-custom-coverage-resolve-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MAIN_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(color_target(ALBEDO_FORMAT)),
                Some(color_target(NORMAL_FORMAT)),
                Some(color_target(PBR_FORMAT)),
                Some(color_target(MOTION_FORMAT)),
                Some(color_target(COVERAGE_FORMAT)),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_hiz_pipeline(
    device: &wgpu::Device,
    hiz_layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/hiz.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("hiz-pipeline-layout"),
        bind_group_layouts: &[Some(hiz_layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("hiz-min-max-compute"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("cs_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn create_fog_pipeline(
    device: &wgpu::Device,
    fog_layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/volumetric_fog.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("volumetric-fog-pipeline-layout"),
        bind_group_layouts: &[Some(fog_layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("blue-noise-volumetric-fog-compute"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("cs_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn create_ssr_pipeline(
    device: &wgpu::Device,
    ssr_layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/ssr.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("half-resolution-ssr-pipeline-layout"),
        bind_group_layouts: &[Some(ssr_layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("half-resolution-ssr-compute"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("cs_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn create_reflection_pipeline(
    device: &wgpu::Device,
    reflection_layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/reflections.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("reflection-pipeline-layout"),
        bind_group_layouts: &[Some(reflection_layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("vendor-agnostic-software-rt-compute"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("cs_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn create_lighting_pipeline(
    device: &wgpu::Device,
    lighting_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/lighting.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lighting-pipeline-layout"),
        bind_group_layouts: &[Some(lighting_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fullscreen-deferred-hdr-lighting"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(HDR_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_light_volume_pipeline(
    device: &wgpu::Device,
    light_volume_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/light_volume.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("local-light-stencil-volume-pipeline-layout"),
        bind_group_layouts: &[Some(light_volume_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("local-light-z-fail-stencil-volume-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MAIN_DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState {
                front: wgpu::StencilFaceState {
                    compare: wgpu::CompareFunction::Always,
                    fail_op: wgpu::StencilOperation::Keep,
                    depth_fail_op: wgpu::StencilOperation::IncrementWrap,
                    pass_op: wgpu::StencilOperation::Keep,
                },
                back: wgpu::StencilFaceState {
                    compare: wgpu::CompareFunction::Always,
                    fail_op: wgpu::StencilOperation::Keep,
                    depth_fail_op: wgpu::StencilOperation::DecrementWrap,
                    pass_op: wgpu::StencilOperation::Keep,
                },
                read_mask: 0xff,
                write_mask: 0xff,
            },
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}

fn create_local_lighting_pipeline(
    device: &wgpu::Device,
    lighting_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/lighting.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("stencil-restricted-local-lighting-pipeline-layout"),
        bind_group_layouts: &[Some(lighting_layout)],
        immediate_size: 0,
    });
    let additive = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("stencil-restricted-local-lighting-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MAIN_DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState {
                front: wgpu::StencilFaceState {
                    compare: wgpu::CompareFunction::NotEqual,
                    ..wgpu::StencilFaceState::default()
                },
                back: wgpu::StencilFaceState {
                    compare: wgpu::CompareFunction::NotEqual,
                    ..wgpu::StencilFaceState::default()
                },
                read_mask: 0xff,
                write_mask: 0,
            },
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_local_lights"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: Some(additive),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_auto_exposure_pipeline(
    device: &wgpu::Device,
    auto_exposure_layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/auto_exposure.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("auto-exposure-white-balance-pipeline-layout"),
        bind_group_layouts: &[Some(auto_exposure_layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("diffuse-luminance-auto-exposure-white-balance"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("cs_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn create_velocity_compensation_pipeline(
    device: &wgpu::Device,
    velocity_layout: &wgpu::BindGroupLayout,
    entry_point: &str,
) -> wgpu::RenderPipeline {
    let shader =
        device.create_shader_module(wgpu::include_wgsl!("shaders/velocity_compensation.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("velocity-compensation-pipeline-layout"),
        bind_group_layouts: &[Some(velocity_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(HDR_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_bloom_pipeline(
    device: &wgpu::Device,
    bloom_layout: &wgpu::BindGroupLayout,
    bloom_format: wgpu::TextureFormat,
    entry_point: &'static str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/bloom.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bloom-pipeline-layout"),
        bind_group_layouts: &[Some(bloom_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("diffuse-gaussian-bloom-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(bloom_format))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_tonemap_pipeline(
    device: &wgpu::Device,
    tonemap_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/tonemap.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tonemap-pipeline-layout"),
        bind_group_layouts: &[Some(tonemap_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gt-tonemap-bloom-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(surface_format))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_rain_pipeline(
    device: &wgpu::Device,
    rain_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/rain3d.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("world-space-rain-pipeline-layout"),
        bind_group_layouts: &[Some(rain_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("world-space-depth-tested-rain-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MAIN_DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(alpha_blend_target(surface_format))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_smaa_edge_pipeline(
    device: &wgpu::Device,
    smaa_edge_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/smaa_edges.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("smaa-edge-pipeline-layout"),
        bind_group_layouts: &[Some(smaa_edge_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("smaa-edge-detect-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(SMAA_EDGE_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_smaa_blend_pipeline(
    device: &wgpu::Device,
    smaa_blend_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/smaa_blend.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("smaa-blend-pipeline-layout"),
        bind_group_layouts: &[Some(smaa_blend_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("smaa-blend-weight-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(SMAA_BLEND_FORMAT))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_aa_pipeline(
    device: &wgpu::Device,
    aa_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/aa.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("smaa-t2x-pipeline-layout"),
        bind_group_layouts: &[Some(aa_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("smaa-t2x-edge-persistence-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[
                Some(color_target(surface_format)),
                Some(color_target(surface_format)),
            ],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Compare final canvases, so clear-and-redraw does not upload unchanged panels.
/// Adjacent dirty tiles share a write; the row pitch still spans the full canvas.
fn changed_hud_regions(
    pixels: &[[u8; 4]],
    previous: &[[u8; 4]],
    width: u32,
    height: u32,
    force: bool,
) -> Vec<[u32; 4]> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    if force || previous.len() != pixels.len() {
        return vec![[0, 0, width, height]];
    }
    let mut regions = Vec::new();
    for y in (0..height).step_by(64) {
        let bottom = (y + 64).min(height);
        let mut run = None;
        for x in (0..width).step_by(64) {
            let right = (x + 64).min(width);
            let changed = (y..bottom).any(|row| {
                let start = (row * width + x) as usize;
                let end = (row * width + right) as usize;
                pixels[start..end] != previous[start..end]
            });
            if changed {
                run.get_or_insert(x);
            } else if let Some(start) = run.take() {
                regions.push([start, y, x - start, bottom - y]);
            }
        }
        if let Some(start) = run {
            regions.push([start, y, width - start, bottom - y]);
        }
    }
    regions
}

fn create_instance_transforms(device: &wgpu::Device) -> InstanceTransforms {
    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("instance-transform-layout"),
        entries: &[
            storage(0, true),
            storage(1, false),
            storage(2, false),
            storage(3, true),
        ],
    });
    let shader =
        device.create_shader_module(wgpu::include_wgsl!("shaders/instance_transform.wgsl"));
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("instance-transform-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("instance-transform-pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    InstanceTransforms {
        pipeline,
        layout,
        resources: None,
        jobs: Vec::new(),
    }
}

fn create_instance_transform_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    local_vertices: &[GpuVertex],
    world_vertices: &wgpu::Buffer,
    previous_positions: &wgpu::Buffer,
    job_capacity: usize,
) -> InstanceTransformResources {
    let local = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("instance-object-space-vertices"),
        contents: bytemuck::cast_slice(local_vertices),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let job_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instance-transform-jobs"),
        size: (job_capacity * std::mem::size_of::<TransformJob>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("instance-transform-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: local.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: world_vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: previous_positions.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: job_buffer.as_entire_binding(),
            },
        ],
    });
    InstanceTransformResources {
        _local_vertices: local,
        job_buffer,
        job_capacity,
        bind_group,
    }
}

fn create_ui_pipeline(
    device: &wgpu::Device,
    ui_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/ui_overlay.wgsl"));
    fullscreen_pipeline(
        device,
        "separate-linear-ui-overlay-pipeline",
        &shader,
        ui_layout,
        UI_FORMAT,
    )
}

fn create_present_pipeline(
    device: &wgpu::Device,
    present_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/present.wgsl"));
    fullscreen_pipeline(
        device,
        "gamma-correct-scene-ui-presentation-pipeline",
        &shader,
        present_layout,
        surface_format,
    )
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(color_target(format))],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("material-layout"),
        entries: &[
            uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
            sampled_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            sampled_texture_layout_entry(3, wgpu::ShaderStages::FRAGMENT),
            sampled_texture_layout_entry(4, wgpu::ShaderStages::FRAGMENT),
            sampled_texture_layout_entry(5, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn reflection_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("reflection-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(2, wgpu::ShaderStages::COMPUTE),
            depth_layout_entry(3, wgpu::ShaderStages::COMPUTE),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: REFLECTION_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            uniform_entry(5, wgpu::ShaderStages::COMPUTE),
            uniform_entry(6, wgpu::ShaderStages::COMPUTE),
            uniform_entry(7, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(8, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(9, wgpu::ShaderStages::COMPUTE),
            uniform_entry(10, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(11, wgpu::ShaderStages::COMPUTE),
            sampled_texture_layout_entry(12, wgpu::ShaderStages::COMPUTE),
            wgpu::BindGroupLayoutEntry {
                binding: 13,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn lighting_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lighting-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(2, wgpu::ShaderStages::FRAGMENT),
            depth_layout_entry(3, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(4, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(5, wgpu::ShaderStages::FRAGMENT),
            depth_layout_entry(6, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            uniform_entry(8, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(9, wgpu::ShaderStages::FRAGMENT),
            depth_array_layout_entry(10, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(11, wgpu::ShaderStages::FRAGMENT),
            sampled_texture_layout_entry(12, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 13,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            uniform_entry(14, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn light_volume_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("local-light-stencil-volume-layout"),
        entries: &[
            uniform_entry(0, wgpu::ShaderStages::VERTEX),
            uniform_entry(1, wgpu::ShaderStages::VERTEX),
        ],
    })
}

fn gbuffer_resolve_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("2x-msaa-gbuffer-custom-resolve-layout"),
        entries: &[
            multisampled_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            multisampled_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            multisampled_texture_layout_entry(2, wgpu::ShaderStages::FRAGMENT),
            multisampled_texture_layout_entry(3, wgpu::ShaderStages::FRAGMENT),
            multisampled_depth_layout_entry(4, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn bloom_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bloom-layout"),
        entries: &[
            sampled_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn depth_copy_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("depth-copy-layout"),
        entries: &[depth_layout_entry(0, wgpu::ShaderStages::FRAGMENT)],
    })
}

fn hiz_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hiz-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::COMPUTE),
            storage_texture_layout_entry(1, wgpu::ShaderStages::COMPUTE, HIZ_FORMAT),
        ],
    })
}

fn fog_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("volumetric-fog-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::COMPUTE),
            storage_texture_layout_entry(1, wgpu::ShaderStages::COMPUTE, VOLUMETRIC_FOG_FORMAT),
            uniform_entry(2, wgpu::ShaderStages::COMPUTE),
            uniform_entry(3, wgpu::ShaderStages::COMPUTE),
            uniform_entry(4, wgpu::ShaderStages::COMPUTE),
            sampled_texture_layout_entry(5, wgpu::ShaderStages::COMPUTE),
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn ssr_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("half-resolution-ssr-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(2, wgpu::ShaderStages::COMPUTE),
            depth_layout_entry(3, wgpu::ShaderStages::COMPUTE),
            storage_texture_layout_entry(4, wgpu::ShaderStages::COMPUTE, REFLECTION_FORMAT),
            uniform_entry(5, wgpu::ShaderStages::COMPUTE),
            uniform_entry(6, wgpu::ShaderStages::COMPUTE),
        ],
    })
}

fn tonemap_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("tonemap-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(2, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(3, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn auto_exposure_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("auto-exposure-white-balance-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::COMPUTE),
            storage_texture_layout_entry(1, wgpu::ShaderStages::COMPUTE, HDR_FORMAT),
            uniform_entry(2, wgpu::ShaderStages::COMPUTE),
            unfiltered_texture_layout_entry(3, wgpu::ShaderStages::COMPUTE),
            uniform_entry(4, wgpu::ShaderStages::COMPUTE),
            uniform_entry(5, wgpu::ShaderStages::COMPUTE),
        ],
    })
}

fn velocity_compensation_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("velocity-compensation-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            depth_layout_entry(2, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(4, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn smaa_edge_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("smaa-edge-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            depth_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(3, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn smaa_blend_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("smaa-blend-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            depth_layout_entry(2, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn aa_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("smaa-t2x-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            depth_layout_entry(2, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(4, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(5, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(6, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(7, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn ui_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("separate-ui-overlay-layout"),
        entries: &[unfiltered_texture_layout_entry(
            0,
            wgpu::ShaderStages::FRAGMENT,
        )],
    })
}

fn present_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gamma-correct-scene-ui-presentation-layout"),
        entries: &[
            unfiltered_texture_layout_entry(0, wgpu::ShaderStages::FRAGMENT),
            unfiltered_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn rain_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("world-space-rain-layout"),
        entries: &[
            uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
            sampled_texture_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            depth_layout_entry(3, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            uniform_entry(5, wgpu::ShaderStages::FRAGMENT),
            uniform_entry(6, wgpu::ShaderStages::VERTEX_FRAGMENT),
            sampled_texture_layout_entry(7, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn sampled_texture_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn unfiltered_texture_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn multisampled_texture_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: true,
        },
        count: None,
    }
}

fn storage_texture_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    format: wgpu::TextureFormat,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn depth_layout_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn multisampled_depth_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: true,
        },
        count: None,
    }
}

fn depth_array_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn render_texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn multisampled_render_texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: GBUFFER_SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn storage_texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn reflection_history_texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: REFLECTION_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hardware-depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: MAIN_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("main-depth-only-view"),
        aspect: wgpu::TextureAspect::DepthOnly,
        ..wgpu::TextureViewDescriptor::default()
    });
    (texture, view)
}

fn multisampled_depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("2x-msaa-hardware-depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: GBUFFER_SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: MAIN_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("2x-msaa-main-depth-only-view"),
        aspect: wgpu::TextureAspect::DepthOnly,
        ..wgpu::TextureViewDescriptor::default()
    });
    (texture, view)
}

fn create_shadow_map(
    device: &wgpu::Device,
    resolution: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("directional-shadow-map"),
        size: wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shadow-comparison-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        compare: Some(wgpu::CompareFunction::GreaterEqual),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..wgpu::SamplerDescriptor::default()
    });
    (texture, view, sampler)
}

fn create_local_shadow_map_array(
    device: &wgpu::Device,
    resolution: u32,
) -> (wgpu::Texture, wgpu::TextureView, Vec<wgpu::TextureView>) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("local-shadow-map-array"),
        size: wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: MAX_GPU_POINT_LIGHTS as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("local-shadow-map-array-view"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..wgpu::TextureViewDescriptor::default()
    });
    let layer_views = (0..MAX_GPU_POINT_LIGHTS)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("local-shadow-map-layer"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer as u32,
                array_layer_count: Some(1),
                ..wgpu::TextureViewDescriptor::default()
            })
        })
        .collect();
    (texture, array_view, layer_views)
}

fn color_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn translucent_albedo_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
        }),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn alpha_blend_target(format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        }),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn load_color_attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        },
    })
}

fn color_attachment(
    view: &wgpu::TextureView,
    clear: wgpu::Color,
) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: wgpu::StoreOp::Store,
        },
    })
}

fn texture_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_regions_include_erasure_and_partial_edge_tiles() {
        let (width, height) = (130, 70);
        let mut previous = vec![[0; 4]; width * height];
        let mut pixels = previous.clone();
        assert!(changed_hud_regions(&pixels, &previous, 130, 70, false).is_empty());
        previous[0] = [255; 4]; // removed panel must be uploaded too
        pixels[64] = [255; 4]; // adjacent changed tiles merge
        pixels[69 * width + 129] = [255; 4]; // partial right/bottom tile
        assert_eq!(
            changed_hud_regions(&pixels, &previous, 130, 70, false),
            vec![[0, 0, 128, 64], [128, 64, 2, 6]]
        );
        assert_eq!(
            changed_hud_regions(&pixels, &previous, 130, 70, true),
            vec![[0, 0, 130, 70]]
        );
        assert_eq!(
            changed_hud_regions(&pixels, &[], 130, 70, false),
            vec![[0, 0, 130, 70]]
        );
    }

    #[test]
    fn gpu_instance_transform_matches_cpu_vertices_and_motion() {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let Ok(adapter) = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
            else {
                eprintln!("skipping GPU readback: no adapter");
                return;
            };
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .unwrap();
            let source = crate::scene::Vertex {
                position: Vec3::new(1.0, -2.0, 3.0),
                previous_position: Vec3::ZERO,
                normal: Vec3::Y,
                tangent: Vec3::ZERO,
                tangent_sign: -1.0,
                uv: crate::math::Vec2::new(0.25, 0.75),
                color: crate::math::Color::new(0.1, 0.4, 0.9, 0.8),
            };
            let local = vec![gpu_vertex(&crate::math::Mat4::IDENTITY, &source); 4];
            let current = crate::math::Mat4::translation(Vec3::new(200.0, 10.0, -30.0))
                * crate::math::Mat4::rotation_y(0.7)
                * crate::math::Mat4::scale(Vec3::new(2.0, 0.5, -3.0));
            let previous = crate::math::Mat4::translation(Vec3::new(198.0, 9.0, -31.0));
            let world_size = std::mem::size_of_val(local.as_slice()) as u64;
            let previous_size = (4 * std::mem::size_of::<GpuPreviousPosition>()) as u64;
            let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
            let world = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&local),
                usage,
            });
            let history = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &vec![0; previous_size as usize],
                usage,
            });
            let transforms = create_instance_transforms(&device);
            let resources = create_instance_transform_resources(
                &device,
                &transforms.layout,
                &local,
                &world,
                &history,
                2,
            );
            let jobs = [
                TransformJob {
                    current: current.m,
                    previous: previous.m,
                    start: 1,
                    count: 2,
                    _padding: [0; 2],
                },
                // A settling frame writes zero object motion.
                TransformJob {
                    current: current.m,
                    previous: current.m,
                    start: 3,
                    count: 1,
                    _padding: [0; 2],
                },
            ];
            queue.write_buffer(&resources.job_buffer, 0, bytemuck::cast_slice(&jobs));
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: world_size + previous_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&transforms.pipeline);
                pass.set_bind_group(0, &resources.bind_group, &[]);
                pass.dispatch_workgroups(1, 2, 1);
            }
            encoder.copy_buffer_to_buffer(&world, 0, &readback, 0, world_size);
            encoder.copy_buffer_to_buffer(&history, 0, &readback, world_size, previous_size);
            queue.submit([encoder.finish()]);
            let (sender, receiver) = mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    sender.send(result).unwrap()
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receiver.recv().unwrap().unwrap();
            let bytes = readback.slice(..).get_mapped_range();
            let vertices: &[GpuVertex] = bytemuck::cast_slice(&bytes[..world_size as usize]);
            let positions: &[GpuPreviousPosition] =
                bytemuck::cast_slice(&bytes[world_size as usize..]);
            assert_eq!(
                bytemuck::bytes_of(&vertices[0]),
                bytemuck::bytes_of(&local[0])
            );
            let expected = gpu_vertex(&current, &source);
            for index in 1..4 {
                for (actual, expected) in vertices[index]
                    .position
                    .iter()
                    .zip(expected.position)
                    .chain(vertices[index].normal.iter().zip(expected.normal))
                    .chain(vertices[index].tangent.iter().zip(expected.tangent))
                {
                    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
                }
                assert_eq!(vertices[index].color, expected.color);
                assert_eq!(vertices[index].uv, expected.uv);
                let pose = if index == 3 { current } else { previous };
                let expected_previous = gpu_position(&pose, source.position);
                for (actual, expected) in positions[index]
                    .position
                    .iter()
                    .zip(expected_previous.position)
                {
                    assert!((actual - expected).abs() < 0.0001);
                }
            }
        });
    }

    #[test]
    fn wasd_moves_camera_relative_to_view_direction() {
        let mut eye = GpuVec3::new(0.0, 1.5, 5.0);
        let mut target = GpuVec3::new(0.0, 1.5, 0.0);
        let keys = CameraKeys {
            move_forward: true,
            move_right: true,
            ..CameraKeys::default()
        };

        apply_camera_keys_to_pose(&mut eye, &mut target, keys, 1.0);

        assert!(eye.x > 0.0);
        assert!(eye.z < 5.0);
        assert!(((target - eye).length() - 5.0).abs() < 0.001);
        assert!(((eye - GpuVec3::new(0.0, 1.5, 5.0)).length() - CAMERA_MOVE_SPEED).abs() < 0.001);
    }

    #[test]
    fn arrow_look_rotates_target_without_translating_eye() {
        let original_eye = GpuVec3::new(0.0, 1.5, 5.0);
        let mut eye = original_eye;
        let mut target = GpuVec3::new(0.0, 1.5, 0.0);
        let keys = CameraKeys {
            look_right: true,
            ..CameraKeys::default()
        };

        apply_camera_keys_to_pose(&mut eye, &mut target, keys, 1.0 / CAMERA_LOOK_SPEED);

        assert_eq!(eye, original_eye);
        assert!(target.x.abs() > 0.1);
        assert!(((target - eye).length() - 5.0).abs() < 0.001);
    }

    #[test]
    fn live_scene_rejects_lights_it_cannot_render() {
        let mut scene = crate::SceneBuilder::new();
        for index in 0..=MAX_GPU_POINT_LIGHTS {
            scene.add_light(crate::Light::point(
                format!("light-{index}"),
                Vec3::new(index as f32, 1.0, 0.0),
                3.0,
                crate::math::Color::WHITE,
                1.0,
            ));
        }
        let scene = scene.build().expect("scene");

        let error = validate_live_scene(&scene).expect_err("light limit");
        assert!(error.to_string().contains("supports 6 local lights"));
    }

    #[test]
    fn dynamic_light_contract_separates_topology_from_mutable_values() {
        let mut scene = crate::SceneBuilder::new();
        scene.add_light(crate::Light::point(
            "lamp",
            Vec3::new(0.0, 1.0, 0.0),
            3.0,
            crate::math::Color::WHITE,
            1.0,
        ));
        let mut scene = scene.build().expect("scene");
        let layout = light_runtime_layout(&scene);
        let shadow_kinds = local_shadow_kinds(&scene);

        scene.lights[0].intensity = 4.0;
        assert_eq!(light_runtime_layout(&scene), layout);
        assert_eq!(local_shadow_kinds(&scene), shadow_kinds);

        scene.lights[0].casts_shadow = true;
        assert_ne!(light_runtime_layout(&scene), layout);
        assert_ne!(local_shadow_kinds(&scene), shadow_kinds);
    }
}
