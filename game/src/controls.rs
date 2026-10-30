//! Keyboard and gamepad mapping to car commands.
//!
//! Keyboard: W/↑ throttle, S/↓ brake, A/D or ←/→ steer, E/Shift up, Q/Ctrl down,
//! C camera (1/2/3 direct), R reset to track, P/Esc pause menu.
//! Gamepad: RT throttle, LT brake, left stick steer, RB/A up, LB/X down,
//! Y camera, Back reset, Start pause.

use crate::{
    camera::Mode,
    menu::{self, Screen},
    Session,
};
use bevy::prelude::*;
use racecraft_sim::CarInput;

#[derive(Default)]
pub struct Controls {
    steer: f64,
    throttle: f64,
    brake: f64,
    shift_up: bool,
    shift_down: bool,
    drs: bool,
}

#[derive(Default)]
pub struct Actions {
    pub menu: bool,
    pub pause: bool,
    pub camera: bool,
    pub reset: bool,
}

const STICK_DEADZONE: f32 = 0.08;

pub fn update(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    time: Res<Time>,
    mut session: ResMut<Session>,
    benchmark: Option<Res<crate::benchmark::Run>>,
) {
    if benchmark.is_some() {
        session.controls = Controls::default();
        return;
    }
    let actions = session
        .controls
        .update(&keys, pads.iter().next(), time.delta_secs());
    if actions.menu || actions.pause {
        match session.screen {
            Screen::Racing => {
                session.screen = Screen::Paused;
                session.paused = true;
            }
            Screen::Paused | Screen::CameraEditor => menu::resume(&mut session),
            _ => {}
        }
    }
    if keys.just_pressed(KeyCode::F3) {
        session.show_performance ^= true;
    }
    if keys.just_pressed(KeyCode::F4) && session.ready {
        if session.screen == Screen::CameraEditor {
            menu::resume(&mut session);
        } else if matches!(session.screen, Screen::Racing | Screen::Paused) {
            session.screen = Screen::CameraEditor;
            session.paused = true;
        }
    }
    if session.screen != Screen::Racing {
        session.controls = Controls::default();
        return;
    }
    for (key, mode) in [
        (KeyCode::Digit1, Mode::Cockpit),
        (KeyCode::Digit2, Mode::Tv),
        (KeyCode::Digit3, Mode::Chase),
    ] {
        if keys.just_pressed(key) {
            session.camera.select(mode);
        }
    }
    if actions.camera {
        session.camera.cycle();
    }
    if actions.reset {
        let player = session.player;
        session.race.recover(player);
    }
}

impl Controls {
    /// Reads this frame's input; discrete presses are queued for the next physics tick.
    pub fn update(
        &mut self,
        input: &ButtonInput<KeyCode>,
        pad: Option<&Gamepad>,
        dt: f32,
    ) -> Actions {
        let dt = dt as f64;
        let held = |keys: &[KeyCode]| keys.iter().any(|&key| input.pressed(key));
        let pressed = |keys: &[KeyCode]| keys.iter().any(|&key| input.just_pressed(key));
        let pad_pressed = |button| pad.is_some_and(|pad| pad.just_pressed(button));

        let left = held(&[KeyCode::KeyA, KeyCode::ArrowLeft]);
        let right = held(&[KeyCode::KeyD, KeyCode::ArrowRight]);
        let stick = pad
            .and_then(|pad| pad.get(GamepadAxis::LeftStickX))
            .unwrap_or(0.0);
        if pad.is_some() && stick.abs() > STICK_DEADZONE {
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

        let key_throttle = held(&[KeyCode::KeyW, KeyCode::ArrowUp]);
        let key_brake = held(&[KeyCode::KeyS, KeyCode::ArrowDown]);
        let pad_throttle = pad
            .and_then(|pad| pad.get(GamepadButton::RightTrigger2))
            .unwrap_or(0.0) as f64;
        let pad_brake = pad
            .and_then(|pad| pad.get(GamepadButton::LeftTrigger2))
            .unwrap_or(0.0) as f64;
        self.throttle = if key_throttle { 1.0 } else { pad_throttle };
        // Keyboard braking builds over ~0.15 s instead of slamming the pedal.
        self.brake = if key_brake {
            (self.brake + dt * 7.0).min(1.0).max(pad_brake)
        } else {
            pad_brake
        };

        self.shift_up |= pressed(&[KeyCode::KeyE, KeyCode::ShiftLeft, KeyCode::ShiftRight])
            || pad_pressed(GamepadButton::RightTrigger)
            || pad_pressed(GamepadButton::South);
        self.shift_down |= pressed(&[KeyCode::KeyQ, KeyCode::ControlLeft, KeyCode::ControlRight])
            || pad_pressed(GamepadButton::LeftTrigger)
            || pad_pressed(GamepadButton::West);
        // Opens DRS where the race allows it; braking closes it.
        self.drs |= pressed(&[KeyCode::KeyF]) || pad_pressed(GamepadButton::East);

        Actions {
            menu: pressed(&[KeyCode::Escape]),
            pause: pressed(&[KeyCode::KeyP]) || pad_pressed(GamepadButton::Start),
            camera: pressed(&[KeyCode::KeyC]) || pad_pressed(GamepadButton::North),
            reset: pressed(&[KeyCode::KeyR]) || pad_pressed(GamepadButton::Select),
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
            drs: self.drs,
        };
        self.shift_up = false;
        self.shift_down = false;
        self.drs = false;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts_survive_render_frames_but_fire_for_one_physics_tick() {
        let mut keys = ButtonInput::default();
        let mut controls = Controls::default();
        keys.press(KeyCode::KeyE);
        controls.update(&keys, None, 0.01);
        keys.clear();
        controls.update(&keys, None, 0.01);
        assert!(controls.car_input().shift_up);
        assert!(!controls.car_input().shift_up);
        keys.press(KeyCode::KeyQ);
        controls.update(&keys, None, 0.01);
        assert!(controls.car_input().shift_down);
        assert!(!controls.car_input().shift_down);
        keys.clear();
        keys.press(KeyCode::KeyF);
        controls.update(&keys, None, 0.01);
        assert!(controls.car_input().drs);
        assert!(!controls.car_input().drs);
    }

    #[test]
    fn analog_controls_release_after_gamepad_disconnect() {
        let keys = ButtonInput::default();
        let mut pad = Gamepad::default();
        pad.analog_mut().set(GamepadAxis::LeftStickX, 0.6);
        pad.analog_mut().set(GamepadButton::RightTrigger2, 0.8);
        pad.analog_mut().set(GamepadButton::LeftTrigger2, 0.2);
        pad.digital_mut().press(GamepadButton::North);
        let mut controls = Controls::default();
        assert!(controls.update(&keys, Some(&pad), 0.1).camera);
        let input = controls.car_input();
        assert!(input.steer < -0.3 && input.steer > -0.6);
        assert!((input.throttle - 0.8).abs() < 1e-6);
        assert!((input.brake - 0.2).abs() < 1e-6);
        controls.update(&keys, None, 0.2);
        let input = controls.car_input();
        assert_eq!(input.throttle, 0.0);
        assert_eq!(input.brake, 0.0);
        assert_eq!(input.steer, 0.0);
    }

    #[test]
    fn keyboard_steering_reverses_and_recentres() {
        let mut keys = ButtonInput::default();
        let mut controls = Controls::default();
        keys.press(KeyCode::KeyA);
        keys.press(KeyCode::KeyW);
        controls.update(&keys, None, 0.1);
        assert!(controls.steer() > 0.0);
        assert_eq!(controls.throttle(), 1.0);
        keys.release(KeyCode::KeyA);
        keys.press(KeyCode::KeyD);
        controls.update(&keys, None, 0.2);
        assert!(controls.steer() < 0.0);
        keys.release_all();
        controls.update(&keys, None, 0.2);
        assert_eq!(controls.steer(), 0.0);
        assert_eq!(controls.throttle(), 0.0);
    }
}
