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
    drs_press_age: Option<f32>,
    /// The stick, not the keys, set the steering last.
    analog: bool,
    /// Car speed in m/s, for the speed-sensitive keyboard steering rate.
    pub speed: f64,
}

#[derive(Default)]
pub struct Actions {
    pub menu: bool,
    pub pause: bool,
    pub camera: bool,
    pub reset: bool,
}

/// Shared 0..1 press envelope for wheel buttons, paddles and fingers.
pub fn press_amount(age: f32) -> f32 {
    const PRESS: f32 = 0.04;
    const HOLD: f32 = 0.1;
    const RELEASE: f32 = 0.22;
    if !(0. ..RELEASE).contains(&age) {
        0.
    } else if age < PRESS {
        age / PRESS
    } else if age < HOLD {
        1.
    } else {
        1. - (age - HOLD) / (RELEASE - HOLD)
    }
}

const STICK_DEADZONE: f32 = 0.08;

/// Whether the driver is touching this pad.
fn in_use(pad: &Gamepad) -> bool {
    let axis = |axis| pad.get(axis).unwrap_or(0.).abs();
    let trigger = |button| pad.get(button).unwrap_or(0.);
    pad.digital().get_just_pressed().next().is_some()
        || axis(GamepadAxis::LeftStickX) > STICK_DEADZONE
        || trigger(GamepadButton::RightTrigger2) > 0.1
        || trigger(GamepadButton::LeftTrigger2) > 0.1
}

/// The pad being driven with: the one last touched. Taking the first pad the
/// system lists ignores the real one whenever a wheel base, a second pad or a
/// device that only reports as a pad is listed ahead of it.
fn active_pad<'a>(
    pads: &'a Query<(Entity, &Gamepad)>,
    active: &mut Option<Entity>,
) -> Option<&'a Gamepad> {
    let current = active.and_then(|entity| pads.get(entity).ok());
    if !current.is_some_and(|(_, pad)| in_use(pad)) {
        let touched = pads.iter().find(|(_, pad)| in_use(pad));
        if let Some((entity, _)) = touched.or(current).or_else(|| pads.iter().next()) {
            *active = Some(entity);
        }
    }
    active
        .and_then(|entity| pads.get(entity).ok())
        .map(|(_, pad)| pad)
}

pub fn update(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad)>,
    mut active: Local<Option<Entity>>,
    time: Res<Time>,
    mut session: ResMut<Session>,
    benchmark: Option<Res<crate::benchmark::Run>>,
    mut online: Option<ResMut<crate::online::Online>>,
) {
    if benchmark.is_some() {
        session.controls = Controls::default();
        return;
    }
    let pad = active_pad(&pads, &mut active);
    session.controls.speed = session.race.entrants[session.player].car.speed();
    let actions = session.controls.update(&keys, pad, time.delta_secs());
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
    if keys.just_pressed(KeyCode::F4)
        && session.ready
        && !online.as_ref().is_some_and(|o| o.client.is_some())
    {
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
        if let Some(client) = online.as_mut().and_then(|o| o.client.as_mut()) {
            let tick = session.race.tick + 1;
            if let Err(error) = client.recover(tick) {
                client.error = Some(error);
            }
        } else {
            session.race.recover(player);
        }
        session.recover_hud();
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
        if left || right {
            self.analog = false;
        } else if pad.is_some() && stick.abs() > STICK_DEADZONE {
            self.analog = true;
        }
        // A centred stick releases at once, but only when the stick was
        // steering: a pad that is merely plugged in must not cancel the
        // keyboard's return ramp.
        if pad.is_some() && !left && !right && self.analog {
            let magnitude =
                ((stick.abs() - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).clamp(0.0, 1.0) as f64;
            // A gentle curve gives finer control near the centre.
            self.steer = -(stick.signum() as f64) * magnitude.powf(1.5);
        } else {
            // Digital steering ramps in and recentres quickly, like a driver's hands.
            let target = (left as i32 - right as i32) as f64;
            // A key is all or nothing, so the ramp is the driver's hands. They
            // move slower at speed, where a short tap must be a small correction.
            let calm = 1.0 - 0.6 * ((self.speed - 25.0) / 55.0).clamp(0.0, 1.0);
            let rate = if target == 0.0 || target.signum() != self.steer.signum() {
                7.0 * (0.5 + 0.5 * calm)
            } else {
                3.2 * calm
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
        self.drs_press_age = self.drs_press_age.map(|age| age + dt as f32);
        let drs_pressed = pressed(&[KeyCode::KeyF]) || pad_pressed(GamepadButton::East);
        self.drs |= drs_pressed;
        if drs_pressed {
            self.drs_press_age = Some(0.);
        }

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

    /// Visual press survives consumption of the one-tick physics command.
    pub fn drs_press(&self) -> f32 {
        self.drs_press_age.map(press_amount).unwrap_or(0.)
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
    fn gamepad_drs_press_releases_without_repeating_while_held() {
        let keys = ButtonInput::default();
        let mut pad = Gamepad::default();
        let mut controls = Controls::default();
        pad.digital_mut().press(GamepadButton::East);
        controls.update(&keys, Some(&pad), 0.);
        assert!(controls.car_input().drs);
        pad.digital_mut().clear();
        controls.update(&keys, Some(&pad), 0.06);
        assert_eq!(controls.drs_press(), 1.);
        assert!(!controls.car_input().drs);
        controls.update(&keys, Some(&pad), 0.3);
        assert_eq!(controls.drs_press(), 0.);
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
    fn centred_stick_releases_without_the_keyboard_return_ramp() {
        let keys = ButtonInput::default();
        let mut pad = Gamepad::default();
        let mut controls = Controls::default();
        pad.analog_mut().set(GamepadAxis::LeftStickX, 1.0);
        controls.update(&keys, Some(&pad), 1.0 / 60.0);
        assert_eq!(controls.car_input().steer, -1.0);
        pad.analog_mut().set(GamepadAxis::LeftStickX, 0.02);
        controls.update(&keys, Some(&pad), 1.0 / 60.0);
        assert_eq!(controls.car_input().steer, 0.0);
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
