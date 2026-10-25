//! The mouse, as Bubble Tea v2 has it: where it is (cells, from 0, 0 at the
//! top left), which button, and the modifiers held.

use std::fmt;

use super::key::{KeyMod, MOD_ALT, MOD_CTRL, MOD_SHIFT};

/// MouseButton is a mouse button, or a turn of the wheel (X11's numbering).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// No button: a motion.
    #[default]
    None,
    /// The left button.
    Left,
    /// The middle button (pressing the wheel).
    Middle,
    /// The right button.
    Right,
    /// The wheel turned up.
    WheelUp,
    /// The wheel turned down.
    WheelDown,
    /// The wheel pushed left.
    WheelLeft,
    /// The wheel pushed right.
    WheelRight,
    /// The browser's back button.
    Backward,
    /// The browser's forward button.
    Forward,
    /// Button 10.
    Button10,
    /// Button 11.
    Button11,
}

impl MouseButton {
    /// string is the button's name, as x/ansi has it.
    pub fn string(&self) -> &'static str {
        match self {
            MouseButton::None => "none",
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
            MouseButton::WheelUp => "wheelup",
            MouseButton::WheelDown => "wheeldown",
            MouseButton::WheelLeft => "wheelleft",
            MouseButton::WheelRight => "wheelright",
            MouseButton::Backward => "backward",
            MouseButton::Forward => "forward",
            MouseButton::Button10 => "button10",
            MouseButton::Button11 => "button11",
        }
    }
}

/// Mouse is a mouse event: where, which button, which modifiers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mouse {
    /// x is the column, from 0 at the left.
    pub x: i32,
    /// y is the row, from 0 at the top.
    pub y: i32,
    /// button is the button, or the wheel's direction.
    pub button: MouseButton,
    /// mods is the modifiers held.
    pub mods: KeyMod,
}

impl Mouse {
    /// string is the event as Bubble Tea writes it: "ctrl+left",
    /// "wheeldown", "" for a motion.
    pub fn string(&self) -> String {
        let mut s = String::new();
        if self.mods.contains(MOD_CTRL) {
            s.push_str("ctrl+");
        }
        if self.mods.contains(MOD_ALT) {
            s.push_str("alt+");
        }
        if self.mods.contains(MOD_SHIFT) {
            s.push_str("shift+");
        }
        let b = self.button.string();
        if b != "none" {
            s.push_str(b);
        }
        s
    }
}

macro_rules! mouse_msg {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name {
            /// x is the column, from 0 at the left.
            pub x: i32,
            /// y is the row, from 0 at the top.
            pub y: i32,
            /// button is the button, or the wheel's direction.
            pub button: MouseButton,
            /// mods is the modifiers held.
            pub mods: KeyMod,
        }

        impl $name {
            /// mouse is the event as a [`Mouse`].
            pub fn mouse(&self) -> Mouse {
                Mouse { x: self.x, y: self.y, button: self.button, mods: self.mods }
            }
        }

        impl From<Mouse> for $name {
            fn from(m: Mouse) -> Self {
                Self { x: m.x, y: m.y, button: m.button, mods: m.mods }
            }
        }
    };
}

mouse_msg!(
    /// MouseClickMsg is a mouse button pressed.
    MouseClickMsg
);
mouse_msg!(
    /// MouseReleaseMsg is a mouse button let go.
    MouseReleaseMsg
);
mouse_msg!(
    /// MouseWheelMsg is the wheel turned.
    MouseWheelMsg
);
mouse_msg!(
    /// MouseMotionMsg is the mouse moved (with a button held, when its
    /// button isn't `None`).
    MouseMotionMsg
);

impl fmt::Display for MouseClickMsg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.mouse().string())
    }
}

impl fmt::Display for MouseReleaseMsg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.mouse().string())
    }
}

impl fmt::Display for MouseWheelMsg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.mouse().string())
    }
}

impl fmt::Display for MouseMotionMsg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.mouse();
        if m.button != MouseButton::None {
            write!(f, "{}+motion", m.string())
        } else {
            write!(f, "{}motion", m.string())
        }
    }
}
