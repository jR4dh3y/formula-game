//! Device-independent input snapshots handed to game systems each frame.

use crate::math::Vec2;

macro_rules! keys {
    ($($name:ident),* $(,)?) => {
        /// Keyboard keys by physical position (US layout names).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Key {
            $($name),*
        }

        impl Key {
            pub const ALL: &'static [Key] = &[$(Key::$name),*];
        }
    };
}

keys!(
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Space,
    Enter,
    Escape,
    Tab,
    Backspace,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    Comma,
    Period,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Semicolon,
    Quote,
    Slash,
);

/// A set of keys, cheap to copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeySet(u128);

impl KeySet {
    pub fn insert(&mut self, key: Key) {
        self.0 |= 1 << key as u8;
    }

    pub fn remove(&mut self, key: Key) {
        self.0 &= !(1 << key as u8);
    }

    pub fn contains(self, key: Key) -> bool {
        self.0 & (1 << key as u8) != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GamepadButton {
    South,
    East,
    North,
    West,
    LeftBumper,
    RightBumper,
    Select,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GamepadButtons(u32);

impl GamepadButtons {
    pub fn insert(&mut self, button: GamepadButton) {
        self.0 |= 1 << button as u8;
    }

    pub fn remove(&mut self, button: GamepadButton) {
        self.0 &= !(1 << button as u8);
    }

    pub fn contains(self, button: GamepadButton) -> bool {
        self.0 & (1 << button as u8) != 0
    }

    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

/// The first connected gamepad. Sticks are in `[-1, 1]` with +y up; triggers in `[0, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GamepadState {
    pub connected: bool,
    pub left_stick: Vec2,
    pub right_stick: Vec2,
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub held: GamepadButtons,
    /// Buttons that went down since the previous frame.
    pub pressed: GamepadButtons,
}
