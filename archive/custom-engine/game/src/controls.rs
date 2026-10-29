//! Keyboard and gamepad mapping to car commands.
//!
//! Keyboard: W/↑ throttle, S/↓ brake, A/D or ←/→ steer, E/Shift up, Q/Ctrl down,
//! C camera, R reset to track, P pause, Esc quit.
//! Gamepad: RT throttle, LT brake, left stick steer, RB/A up, LB/X down,
//! Y camera, Back reset, Start pause.

use dhool_pro::{GamepadButton, InputState, Key};
use racecraft_sim::CarInput;

#[derive(Default)]
pub struct Controls {
    steer: f64,
    throttle: f64,
    brake: f64,
    shift_up: bool,
    shift_down: bool,
}

#[derive(Default)]
pub struct Actions {
    pub quit: bool,
    pub pause: bool,
    pub camera: bool,
    pub reset: bool,
}

const STICK_DEADZONE: f32 = 0.08;

impl Controls {
    /// Reads this frame's input; discrete presses are queued for the next physics tick.
    pub fn update(&mut self, input: &InputState, dt: f32) -> Actions {
        let dt = dt as f64;
        let held = |keys: &[Key]| keys.iter().any(|&key| input.keys.contains(key));
        let pressed = |keys: &[Key]| keys.iter().any(|&key| input.keys_pressed.contains(key));
        let pad = input.gamepad;
        let pad_pressed = |button| pad.connected && pad.pressed.contains(button);

        let left = held(&[Key::A, Key::ArrowLeft]);
        let right = held(&[Key::D, Key::ArrowRight]);
        let stick = pad.left_stick.x;
        if pad.connected && stick.abs() > STICK_DEADZONE {
            let magnitude = ((stick.abs() - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)) as f64;
            // A gentle curve gives finer control near the centre.
            self.steer = -(stick.signum() as f64) * magnitude.powf(1.5);
        } else {
            // Digital steering ramps in and recentres quickly, like a driver's hands.
            let target = (left as i32 - right as i32) as f64;
            let rate = if target == 0.0 || target.signum() != self.steer.signum() {
                7.0
            } else {
                3.2
            };
            let step = rate * dt;
            self.steer += (target - self.steer).clamp(-step, step);
        }

        let key_throttle = held(&[Key::W, Key::ArrowUp]);
        let key_brake = held(&[Key::S, Key::ArrowDown]);
        let pad_throttle = if pad.connected {
            pad.right_trigger as f64
        } else {
            0.0
        };
        let pad_brake = if pad.connected {
            pad.left_trigger as f64
        } else {
            0.0
        };
        self.throttle = if key_throttle { 1.0 } else { pad_throttle };
        // Keyboard braking builds over ~0.15 s instead of slamming the pedal.
        self.brake = if key_brake {
            (self.brake + dt * 7.0).min(1.0).max(pad_brake)
        } else {
            pad_brake
        };

        self.shift_up |= pressed(&[Key::E, Key::ShiftLeft, Key::ShiftRight])
            || pad_pressed(GamepadButton::RightBumper)
            || pad_pressed(GamepadButton::South);
        self.shift_down |= pressed(&[Key::Q, Key::ControlLeft, Key::ControlRight])
            || pad_pressed(GamepadButton::LeftBumper)
            || pad_pressed(GamepadButton::West);

        Actions {
            quit: pressed(&[Key::Escape]),
            pause: pressed(&[Key::P]) || pad_pressed(GamepadButton::Start),
            camera: pressed(&[Key::C]) || pad_pressed(GamepadButton::North),
            reset: pressed(&[Key::R]) || pad_pressed(GamepadButton::Select),
        }
    }

    /// Command for one physics tick; queued shifts fire once.
    pub fn car_input(&mut self) -> CarInput {
        let command = CarInput {
            steer: self.steer,
            throttle: self.throttle,
            brake: self.brake,
            shift_up: self.shift_up,
            shift_down: self.shift_down,
        };
        self.shift_up = false;
        self.shift_down = false;
        command
    }

    pub fn steer(&self) -> f64 {
        self.steer
    }

    pub fn throttle(&self) -> f64 {
        self.throttle
    }

    pub fn brake(&self) -> f64 {
        self.brake
    }
}
