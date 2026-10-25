//! Keys, as Bubble Tea v2 (on ultraviolet) has them: a key press is a code
//! (a character, or one of the special keys below), the text it types, and
//! the modifiers held; its string ("enter", "ctrl+c", "shift+tab", "K",
//! "space") is what key bindings match.

use std::fmt;

/// KeyMod is the modifier keys held with a key or a mouse button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyMod(pub u32);

impl KeyMod {
    /// contains says whether every modifier in `mods` is held.
    pub fn contains(self, mods: KeyMod) -> bool {
        self.0 & mods.0 == mods.0
    }
}

impl std::ops::BitOr for KeyMod {
    type Output = KeyMod;
    fn bitor(self, o: KeyMod) -> KeyMod {
        KeyMod(self.0 | o.0)
    }
}

impl std::ops::BitOrAssign for KeyMod {
    fn bitor_assign(&mut self, o: KeyMod) {
        self.0 |= o.0;
    }
}

/// No modifier.
pub const MOD_NONE: KeyMod = KeyMod(0);
/// Shift.
pub const MOD_SHIFT: KeyMod = KeyMod(1);
/// Alt (Option on a Mac).
pub const MOD_ALT: KeyMod = KeyMod(1 << 1);
/// Control.
pub const MOD_CTRL: KeyMod = KeyMod(1 << 2);
/// Meta.
pub const MOD_META: KeyMod = KeyMod(1 << 3);
/// Hyper.
pub const MOD_HYPER: KeyMod = KeyMod(1 << 4);
/// Super (Command on a Mac, Windows on a PC).
pub const MOD_SUPER: KeyMod = KeyMod(1 << 5);
/// Caps Lock on.
pub const MOD_CAPS_LOCK: KeyMod = KeyMod(1 << 6);
/// Num Lock on.
pub const MOD_NUM_LOCK: KeyMod = KeyMod(1 << 7);
/// Scroll Lock on.
pub const MOD_SCROLL_LOCK: KeyMod = KeyMod(1 << 8);

/// KEY_EXTENDED is the code of a key that types several characters at once.
pub const KEY_EXTENDED: u32 = 0x10FFFF + 1;

/// The special keys' codes, above every character's, in ultraviolet's order.
#[allow(missing_docs)]
pub mod keys {
    use super::KEY_EXTENDED;
    pub const KEY_UP: u32 = KEY_EXTENDED + 1;
    pub const KEY_DOWN: u32 = KEY_EXTENDED + 2;
    pub const KEY_RIGHT: u32 = KEY_EXTENDED + 3;
    pub const KEY_LEFT: u32 = KEY_EXTENDED + 4;
    pub const KEY_BEGIN: u32 = KEY_EXTENDED + 5;
    pub const KEY_FIND: u32 = KEY_EXTENDED + 6;
    pub const KEY_INSERT: u32 = KEY_EXTENDED + 7;
    pub const KEY_DELETE: u32 = KEY_EXTENDED + 8;
    pub const KEY_SELECT: u32 = KEY_EXTENDED + 9;
    pub const KEY_PGUP: u32 = KEY_EXTENDED + 10;
    pub const KEY_PGDOWN: u32 = KEY_EXTENDED + 11;
    pub const KEY_HOME: u32 = KEY_EXTENDED + 12;
    pub const KEY_END: u32 = KEY_EXTENDED + 13;
    pub const KEY_KP_ENTER: u32 = KEY_EXTENDED + 14;
    pub const KEY_KP_EQUAL: u32 = KEY_EXTENDED + 15;
    pub const KEY_KP_MULTIPLY: u32 = KEY_EXTENDED + 16;
    pub const KEY_KP_PLUS: u32 = KEY_EXTENDED + 17;
    pub const KEY_KP_COMMA: u32 = KEY_EXTENDED + 18;
    pub const KEY_KP_MINUS: u32 = KEY_EXTENDED + 19;
    pub const KEY_KP_DECIMAL: u32 = KEY_EXTENDED + 20;
    pub const KEY_KP_DIVIDE: u32 = KEY_EXTENDED + 21;
    pub const KEY_KP0: u32 = KEY_EXTENDED + 22;
    pub const KEY_KP1: u32 = KEY_EXTENDED + 23;
    pub const KEY_KP2: u32 = KEY_EXTENDED + 24;
    pub const KEY_KP3: u32 = KEY_EXTENDED + 25;
    pub const KEY_KP4: u32 = KEY_EXTENDED + 26;
    pub const KEY_KP5: u32 = KEY_EXTENDED + 27;
    pub const KEY_KP6: u32 = KEY_EXTENDED + 28;
    pub const KEY_KP7: u32 = KEY_EXTENDED + 29;
    pub const KEY_KP8: u32 = KEY_EXTENDED + 30;
    pub const KEY_KP9: u32 = KEY_EXTENDED + 31;
    pub const KEY_KP_SEP: u32 = KEY_EXTENDED + 32;
    pub const KEY_KP_UP: u32 = KEY_EXTENDED + 33;
    pub const KEY_KP_DOWN: u32 = KEY_EXTENDED + 34;
    pub const KEY_KP_LEFT: u32 = KEY_EXTENDED + 35;
    pub const KEY_KP_RIGHT: u32 = KEY_EXTENDED + 36;
    pub const KEY_KP_PGUP: u32 = KEY_EXTENDED + 37;
    pub const KEY_KP_PGDOWN: u32 = KEY_EXTENDED + 38;
    pub const KEY_KP_HOME: u32 = KEY_EXTENDED + 39;
    pub const KEY_KP_END: u32 = KEY_EXTENDED + 40;
    pub const KEY_KP_INSERT: u32 = KEY_EXTENDED + 41;
    pub const KEY_KP_DELETE: u32 = KEY_EXTENDED + 42;
    pub const KEY_KP_BEGIN: u32 = KEY_EXTENDED + 43;
    pub const KEY_F1: u32 = KEY_EXTENDED + 44;
    pub const KEY_F2: u32 = KEY_EXTENDED + 45;
    pub const KEY_F3: u32 = KEY_EXTENDED + 46;
    pub const KEY_F4: u32 = KEY_EXTENDED + 47;
    pub const KEY_F5: u32 = KEY_EXTENDED + 48;
    pub const KEY_F6: u32 = KEY_EXTENDED + 49;
    pub const KEY_F7: u32 = KEY_EXTENDED + 50;
    pub const KEY_F8: u32 = KEY_EXTENDED + 51;
    pub const KEY_F9: u32 = KEY_EXTENDED + 52;
    pub const KEY_F10: u32 = KEY_EXTENDED + 53;
    pub const KEY_F11: u32 = KEY_EXTENDED + 54;
    pub const KEY_F12: u32 = KEY_EXTENDED + 55;
    pub const KEY_F13: u32 = KEY_EXTENDED + 56;
    pub const KEY_F14: u32 = KEY_EXTENDED + 57;
    pub const KEY_F15: u32 = KEY_EXTENDED + 58;
    pub const KEY_F16: u32 = KEY_EXTENDED + 59;
    pub const KEY_F17: u32 = KEY_EXTENDED + 60;
    pub const KEY_F18: u32 = KEY_EXTENDED + 61;
    pub const KEY_F19: u32 = KEY_EXTENDED + 62;
    pub const KEY_F20: u32 = KEY_EXTENDED + 63;
    pub const KEY_F21: u32 = KEY_EXTENDED + 64;
    pub const KEY_F22: u32 = KEY_EXTENDED + 65;
    pub const KEY_F23: u32 = KEY_EXTENDED + 66;
    pub const KEY_F24: u32 = KEY_EXTENDED + 67;
    pub const KEY_F25: u32 = KEY_EXTENDED + 68;
    pub const KEY_F26: u32 = KEY_EXTENDED + 69;
    pub const KEY_F27: u32 = KEY_EXTENDED + 70;
    pub const KEY_F28: u32 = KEY_EXTENDED + 71;
    pub const KEY_F29: u32 = KEY_EXTENDED + 72;
    pub const KEY_F30: u32 = KEY_EXTENDED + 73;
    pub const KEY_F31: u32 = KEY_EXTENDED + 74;
    pub const KEY_F32: u32 = KEY_EXTENDED + 75;
    pub const KEY_F33: u32 = KEY_EXTENDED + 76;
    pub const KEY_F34: u32 = KEY_EXTENDED + 77;
    pub const KEY_F35: u32 = KEY_EXTENDED + 78;
    pub const KEY_F36: u32 = KEY_EXTENDED + 79;
    pub const KEY_F37: u32 = KEY_EXTENDED + 80;
    pub const KEY_F38: u32 = KEY_EXTENDED + 81;
    pub const KEY_F39: u32 = KEY_EXTENDED + 82;
    pub const KEY_F40: u32 = KEY_EXTENDED + 83;
    pub const KEY_F41: u32 = KEY_EXTENDED + 84;
    pub const KEY_F42: u32 = KEY_EXTENDED + 85;
    pub const KEY_F43: u32 = KEY_EXTENDED + 86;
    pub const KEY_F44: u32 = KEY_EXTENDED + 87;
    pub const KEY_F45: u32 = KEY_EXTENDED + 88;
    pub const KEY_F46: u32 = KEY_EXTENDED + 89;
    pub const KEY_F47: u32 = KEY_EXTENDED + 90;
    pub const KEY_F48: u32 = KEY_EXTENDED + 91;
    pub const KEY_F49: u32 = KEY_EXTENDED + 92;
    pub const KEY_F50: u32 = KEY_EXTENDED + 93;
    pub const KEY_F51: u32 = KEY_EXTENDED + 94;
    pub const KEY_F52: u32 = KEY_EXTENDED + 95;
    pub const KEY_F53: u32 = KEY_EXTENDED + 96;
    pub const KEY_F54: u32 = KEY_EXTENDED + 97;
    pub const KEY_F55: u32 = KEY_EXTENDED + 98;
    pub const KEY_F56: u32 = KEY_EXTENDED + 99;
    pub const KEY_F57: u32 = KEY_EXTENDED + 100;
    pub const KEY_F58: u32 = KEY_EXTENDED + 101;
    pub const KEY_F59: u32 = KEY_EXTENDED + 102;
    pub const KEY_F60: u32 = KEY_EXTENDED + 103;
    pub const KEY_F61: u32 = KEY_EXTENDED + 104;
    pub const KEY_F62: u32 = KEY_EXTENDED + 105;
    pub const KEY_F63: u32 = KEY_EXTENDED + 106;
    pub const KEY_CAPS_LOCK: u32 = KEY_EXTENDED + 107;
    pub const KEY_SCROLL_LOCK: u32 = KEY_EXTENDED + 108;
    pub const KEY_NUM_LOCK: u32 = KEY_EXTENDED + 109;
    pub const KEY_PRINT_SCREEN: u32 = KEY_EXTENDED + 110;
    pub const KEY_PAUSE: u32 = KEY_EXTENDED + 111;
    pub const KEY_MENU: u32 = KEY_EXTENDED + 112;
    pub const KEY_MEDIA_PLAY: u32 = KEY_EXTENDED + 113;
    pub const KEY_MEDIA_PAUSE: u32 = KEY_EXTENDED + 114;
    pub const KEY_MEDIA_PLAY_PAUSE: u32 = KEY_EXTENDED + 115;
    pub const KEY_MEDIA_REVERSE: u32 = KEY_EXTENDED + 116;
    pub const KEY_MEDIA_STOP: u32 = KEY_EXTENDED + 117;
    pub const KEY_MEDIA_FAST_FORWARD: u32 = KEY_EXTENDED + 118;
    pub const KEY_MEDIA_REWIND: u32 = KEY_EXTENDED + 119;
    pub const KEY_MEDIA_NEXT: u32 = KEY_EXTENDED + 120;
    pub const KEY_MEDIA_PREV: u32 = KEY_EXTENDED + 121;
    pub const KEY_MEDIA_RECORD: u32 = KEY_EXTENDED + 122;
    pub const KEY_LOWER_VOL: u32 = KEY_EXTENDED + 123;
    pub const KEY_RAISE_VOL: u32 = KEY_EXTENDED + 124;
    pub const KEY_MUTE: u32 = KEY_EXTENDED + 125;
    pub const KEY_LEFT_SHIFT: u32 = KEY_EXTENDED + 126;
    pub const KEY_LEFT_ALT: u32 = KEY_EXTENDED + 127;
    pub const KEY_LEFT_CTRL: u32 = KEY_EXTENDED + 128;
    pub const KEY_LEFT_SUPER: u32 = KEY_EXTENDED + 129;
    pub const KEY_LEFT_HYPER: u32 = KEY_EXTENDED + 130;
    pub const KEY_LEFT_META: u32 = KEY_EXTENDED + 131;
    pub const KEY_RIGHT_SHIFT: u32 = KEY_EXTENDED + 132;
    pub const KEY_RIGHT_ALT: u32 = KEY_EXTENDED + 133;
    pub const KEY_RIGHT_CTRL: u32 = KEY_EXTENDED + 134;
    pub const KEY_RIGHT_SUPER: u32 = KEY_EXTENDED + 135;
    pub const KEY_RIGHT_HYPER: u32 = KEY_EXTENDED + 136;
    pub const KEY_RIGHT_META: u32 = KEY_EXTENDED + 137;
    pub const KEY_ISO_LEVEL3_SHIFT: u32 = KEY_EXTENDED + 138;
    pub const KEY_ISO_LEVEL5_SHIFT: u32 = KEY_EXTENDED + 139;

    /// Backspace.
    pub const KEY_BACKSPACE: u32 = 0x7F;
    /// Tab.
    pub const KEY_TAB: u32 = 0x09;
    /// Enter.
    pub const KEY_ENTER: u32 = 0x0D;
    /// Return: Enter.
    pub const KEY_RETURN: u32 = KEY_ENTER;
    /// Escape.
    pub const KEY_ESCAPE: u32 = 0x1B;
    /// Esc: Escape.
    pub const KEY_ESC: u32 = KEY_ESCAPE;
    /// The space bar.
    pub const KEY_SPACE: u32 = 0x20;
}

pub use keys::*;

/// key_type_string is a special key's name, if it has one.
fn key_type_string(code: u32) -> Option<&'static str> {
    Some(match code {
        KEY_ENTER => "enter",
        KEY_TAB => "tab",
        KEY_BACKSPACE => "backspace",
        KEY_ESCAPE => "esc",
        KEY_SPACE => "space",
        KEY_UP => "up",
        KEY_DOWN => "down",
        KEY_LEFT => "left",
        KEY_RIGHT => "right",
        KEY_BEGIN => "begin",
        KEY_FIND => "find",
        KEY_INSERT => "insert",
        KEY_DELETE => "delete",
        KEY_SELECT => "select",
        KEY_PGUP => "pgup",
        KEY_PGDOWN => "pgdown",
        KEY_HOME => "home",
        KEY_END => "end",
        KEY_KP_ENTER => "enter",
        KEY_KP_EQUAL => "equal",
        KEY_KP_MULTIPLY => "mul",
        KEY_KP_PLUS => "plus",
        KEY_KP_COMMA => "comma",
        KEY_KP_MINUS => "minus",
        KEY_KP_DECIMAL => "period",
        KEY_KP_DIVIDE => "div",
        KEY_KP0 => "0",
        KEY_KP1 => "1",
        KEY_KP2 => "2",
        KEY_KP3 => "3",
        KEY_KP4 => "4",
        KEY_KP5 => "5",
        KEY_KP6 => "6",
        KEY_KP7 => "7",
        KEY_KP8 => "8",
        KEY_KP9 => "9",
        KEY_KP_SEP => "sep",
        KEY_KP_UP => "up",
        KEY_KP_DOWN => "down",
        KEY_KP_LEFT => "left",
        KEY_KP_RIGHT => "right",
        KEY_KP_PGUP => "pgup",
        KEY_KP_PGDOWN => "pgdown",
        KEY_KP_HOME => "home",
        KEY_KP_END => "end",
        KEY_KP_INSERT => "insert",
        KEY_KP_DELETE => "delete",
        KEY_KP_BEGIN => "begin",
        KEY_CAPS_LOCK => "capslock",
        KEY_SCROLL_LOCK => "scrolllock",
        KEY_NUM_LOCK => "numlock",
        KEY_PRINT_SCREEN => "printscreen",
        KEY_PAUSE => "pause",
        KEY_MENU => "menu",
        KEY_MEDIA_PLAY => "mediaplay",
        KEY_MEDIA_PAUSE => "mediapause",
        KEY_MEDIA_PLAY_PAUSE => "mediaplaypause",
        KEY_MEDIA_REVERSE => "mediareverse",
        KEY_MEDIA_STOP => "mediastop",
        KEY_MEDIA_FAST_FORWARD => "mediafastforward",
        KEY_MEDIA_REWIND => "mediarewind",
        KEY_MEDIA_NEXT => "medianext",
        KEY_MEDIA_PREV => "mediaprev",
        KEY_MEDIA_RECORD => "mediarecord",
        KEY_LOWER_VOL => "lowervol",
        KEY_RAISE_VOL => "raisevol",
        KEY_MUTE => "mute",
        KEY_LEFT_SHIFT => "leftshift",
        KEY_LEFT_ALT => "leftalt",
        KEY_LEFT_CTRL => "leftctrl",
        KEY_LEFT_SUPER => "leftsuper",
        KEY_LEFT_HYPER => "lefthyper",
        KEY_LEFT_META => "leftmeta",
        KEY_RIGHT_SHIFT => "rightshift",
        KEY_RIGHT_ALT => "rightalt",
        KEY_RIGHT_CTRL => "rightctrl",
        KEY_RIGHT_SUPER => "rightsuper",
        KEY_RIGHT_HYPER => "righthyper",
        KEY_RIGHT_META => "rightmeta",
        KEY_ISO_LEVEL3_SHIFT => "isolevel3shift",
        KEY_ISO_LEVEL5_SHIFT => "isolevel5shift",
        c if (KEY_F1..=KEY_F63).contains(&c) => return Some(F_NAMES[(c - KEY_F1) as usize]),
        _ => return None,
    })
}

static F_NAMES: [&str; 63] = [
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "f13", "f14", "f15", "f16", "f17",
    "f18", "f19", "f20", "f21", "f22", "f23", "f24", "f25", "f26", "f27", "f28", "f29", "f30", "f31", "f32", "f33",
    "f34", "f35", "f36", "f37", "f38", "f39", "f40", "f41", "f42", "f43", "f44", "f45", "f46", "f47", "f48", "f49",
    "f50", "f51", "f52", "f53", "f54", "f55", "f56", "f57", "f58", "f59", "f60", "f61", "f62", "f63",
];

/// string_key_type is the code a key's name stands for.
fn string_key_type(s: &str) -> Option<u32> {
    Some(match s {
        "enter" => KEY_ENTER,
        "tab" => KEY_TAB,
        "backspace" => KEY_BACKSPACE,
        "escape" | "esc" => KEY_ESCAPE,
        "space" => KEY_SPACE,
        "up" => KEY_UP,
        "down" => KEY_DOWN,
        "left" => KEY_LEFT,
        "right" => KEY_RIGHT,
        "begin" => KEY_BEGIN,
        "find" => KEY_FIND,
        "insert" => KEY_INSERT,
        "delete" => KEY_DELETE,
        "select" => KEY_SELECT,
        "pgup" => KEY_PGUP,
        "pgdown" => KEY_PGDOWN,
        "home" => KEY_HOME,
        "end" => KEY_END,
        "kpenter" => KEY_KP_ENTER,
        "kpequal" => KEY_KP_EQUAL,
        "kpmul" => KEY_KP_MULTIPLY,
        "kpplus" => KEY_KP_PLUS,
        "kpcomma" => KEY_KP_COMMA,
        "kpminus" => KEY_KP_MINUS,
        "kpperiod" => KEY_KP_DECIMAL,
        "kpdiv" => KEY_KP_DIVIDE,
        "kp0" => KEY_KP0,
        "kp1" => KEY_KP1,
        "kp2" => KEY_KP2,
        "kp3" => KEY_KP3,
        "kp4" => KEY_KP4,
        "kp5" => KEY_KP5,
        "kp6" => KEY_KP6,
        "kp7" => KEY_KP7,
        "kp8" => KEY_KP8,
        "kp9" => KEY_KP9,
        "kpsep" => KEY_KP_SEP,
        "kpup" => KEY_KP_UP,
        "kpdown" => KEY_KP_DOWN,
        "kpleft" => KEY_KP_LEFT,
        "kpright" => KEY_KP_RIGHT,
        "kppgup" => KEY_KP_PGUP,
        "kppgdown" => KEY_KP_PGDOWN,
        "kphome" => KEY_KP_HOME,
        "kpend" => KEY_KP_END,
        "kpinsert" => KEY_KP_INSERT,
        "kpdelete" => KEY_KP_DELETE,
        "kpbegin" => KEY_KP_BEGIN,
        _ => {
            let n = s.strip_prefix('f')?.parse::<u32>().ok()?;
            if (1..=63).contains(&n) {
                return Some(KEY_F1 + n - 1);
            }
            return None;
        }
    })
}

/// Key is a key pressed or released: Bubble Tea's `Key`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Key {
    /// text is what the key types: empty for special keys and for keys
    /// held with ctrl or alt.
    pub text: String,
    /// mods is the modifiers held.
    pub mods: KeyMod,
    /// code is the key: a character's code, or a special key's (`KEY_TAB`…).
    pub code: u32,
    /// shifted_code is the key with shift applied, when known.
    pub shifted_code: u32,
    /// base_code is the key on a US PC-101 layout, when known.
    pub base_code: u32,
    /// is_repeat says whether the key is held down, repeating.
    pub is_repeat: bool,
}

impl Key {
    /// string is the text the key types, or its keystroke ("ctrl+c",
    /// "enter", "space"): what key bindings match.
    pub fn string(&self) -> String {
        if !self.text.is_empty() && self.text != " " {
            return self.text.clone();
        }
        self.keystroke()
    }

    /// keystroke is the key with its modifiers, always in the order ctrl,
    /// alt, shift, meta, hyper, super: "ctrl+shift+a", never "shift+ctrl+a".
    pub fn keystroke(&self) -> String {
        let mut sb = String::new();
        let m = self.mods;
        if m.contains(MOD_CTRL) && self.code != KEY_LEFT_CTRL && self.code != KEY_RIGHT_CTRL {
            sb.push_str("ctrl+");
        }
        if m.contains(MOD_ALT) && self.code != KEY_LEFT_ALT && self.code != KEY_RIGHT_ALT {
            sb.push_str("alt+");
        }
        if m.contains(MOD_SHIFT) && self.code != KEY_LEFT_SHIFT && self.code != KEY_RIGHT_SHIFT {
            sb.push_str("shift+");
        }
        if m.contains(MOD_META) && self.code != KEY_LEFT_META && self.code != KEY_RIGHT_META {
            sb.push_str("meta+");
        }
        if m.contains(MOD_HYPER) && self.code != KEY_LEFT_HYPER && self.code != KEY_RIGHT_HYPER {
            sb.push_str("hyper+");
        }
        if m.contains(MOD_SUPER) && self.code != KEY_LEFT_SUPER && self.code != KEY_RIGHT_SUPER {
            sb.push_str("super+");
        }
        if let Some(kt) = key_type_string(self.code) {
            sb.push_str(kt);
        } else {
            let code = if self.base_code != 0 { self.base_code } else { self.code };
            match code {
                KEY_SPACE => sb.push_str("space"),
                KEY_EXTENDED => sb.push_str(&self.text),
                c => sb.push(char::from_u32(c).unwrap_or('\u{FFFD}')),
            }
        }
        sb
    }

    /// parse makes the key a string names, as a terminal would send it:
    /// "ctrl+f", "shift+tab", "enter", "space", "K" (shift and k, typing
    /// "K"), "?", "alt+b", "f5". For tests and for keys a model sends
    /// itself.
    pub fn parse(s: &str) -> Key {
        let mut mods = MOD_NONE;
        let mut code: u32 = 0;
        let mut text = String::new();
        // a lone "+" is the plus key
        let parts: Vec<&str> = if s == "+" { vec!["+"] } else { s.split('+').collect() };
        for part in parts {
            match part {
                "ctrl" => mods |= MOD_CTRL,
                "alt" => mods |= MOD_ALT,
                "shift" => mods |= MOD_SHIFT,
                "meta" => mods |= MOD_META,
                "hyper" => mods |= MOD_HYPER,
                "super" => mods |= MOD_SUPER,
                "capslock" => mods |= MOD_CAPS_LOCK,
                "scrolllock" => mods |= MOD_SCROLL_LOCK,
                "numlock" => mods |= MOD_NUM_LOCK,
                "" => {}
                _ => {
                    if let Some(k) = string_key_type(part) {
                        code = k;
                    } else if part.chars().count() == 1 {
                        code = part.chars().next().unwrap_or('\0') as u32;
                    } else {
                        code = KEY_EXTENDED;
                        text = part.to_string();
                    }
                }
            }
        }
        let mut k = Key { text, mods, code, ..Default::default() };
        let plain = MOD_CTRL.0 | MOD_ALT.0 | MOD_META.0 | MOD_HYPER.0 | MOD_SUPER.0;
        if k.code == KEY_SPACE && mods.0 & plain == 0 {
            // the space bar types a space
            k.text = " ".into();
        } else if let Some(c) = char::from_u32(k.code)
            && k.text.is_empty()
            && mods.0 & plain == 0
            && key_type_string(k.code).is_none()
            && !c.is_control()
        {
            if c.is_uppercase() {
                // an upper case letter is shift and the letter
                k.code = c.to_lowercase().next().unwrap_or(c) as u32;
                k.shifted_code = c as u32;
                k.mods |= MOD_SHIFT;
                k.text = c.to_string();
            } else if mods.contains(MOD_SHIFT) {
                let up = c.to_uppercase().next().unwrap_or(c);
                k.shifted_code = up as u32;
                k.text = up.to_string();
            } else {
                k.text = c.to_string();
            }
        }
        k
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

macro_rules! key_msg {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name {
            /// text is what the key types: empty for special keys and for
            /// keys held with ctrl or alt.
            pub text: String,
            /// mods is the modifiers held.
            pub mods: KeyMod,
            /// code is the key: a character's code, or a special key's.
            pub code: u32,
            /// shifted_code is the key with shift applied, when known.
            pub shifted_code: u32,
            /// base_code is the key on a US PC-101 layout, when known.
            pub base_code: u32,
            /// is_repeat says whether the key is held down, repeating.
            pub is_repeat: bool,
        }

        impl $name {
            /// key is the key itself.
            pub fn key(&self) -> Key {
                Key {
                    text: self.text.clone(),
                    mods: self.mods,
                    code: self.code,
                    shifted_code: self.shifted_code,
                    base_code: self.base_code,
                    is_repeat: self.is_repeat,
                }
            }

            /// string is what key bindings match: the text typed, or the
            /// keystroke ("enter", "ctrl+c", "shift+tab", "space").
            pub fn string(&self) -> String {
                self.key().string()
            }

            /// keystroke is the key with its modifiers.
            pub fn keystroke(&self) -> String {
                self.key().keystroke()
            }

            /// new is the key with this code and no modifiers, like Go's
            /// `tea.KeyPressMsg{Code: tea.KeyEnter}`.
            pub fn new(code: u32) -> Self {
                Self { code, ..Default::default() }
            }

            /// with_mods is the key with this code and modifiers, like Go's
            /// `tea.KeyPressMsg{Code: tea.KeyTab, Mod: tea.ModShift}`.
            pub fn with_mods(code: u32, mods: KeyMod) -> Self {
                Self { code, mods, ..Default::default() }
            }

            /// char is the key typing `c`, like Go's
            /// `tea.KeyPressMsg{Code: r, Text: string(r)}`.
            pub fn char(c: char) -> Self {
                Self { code: c as u32, text: c.to_string(), ..Default::default() }
            }

            /// parse is the key a string names, as a terminal sends it:
            /// "ctrl+f", "shift+tab", "enter", "space", "K", "?".
            pub fn parse(s: &str) -> Self {
                Self::from(Key::parse(s))
            }
        }

        impl From<Key> for $name {
            fn from(k: Key) -> Self {
                Self {
                    text: k.text,
                    mods: k.mods,
                    code: k.code,
                    shifted_code: k.shifted_code,
                    base_code: k.base_code,
                    is_repeat: k.is_repeat,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.string())
            }
        }
    };
}

key_msg!(
    /// KeyPressMsg is a key pressed.
    KeyPressMsg
);
key_msg!(
    /// KeyReleaseMsg is a key let go (only terminals that report them).
    KeyReleaseMsg
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_strings_as_bubbletea() {
        let cases: &[(KeyPressMsg, &str)] = &[
            (KeyPressMsg::new(KEY_ENTER), "enter"),
            (KeyPressMsg::new(KEY_ESCAPE), "esc"),
            (KeyPressMsg::new(KEY_TAB), "tab"),
            (KeyPressMsg::with_mods(KEY_TAB, MOD_SHIFT), "shift+tab"),
            (KeyPressMsg::new(KEY_BACKSPACE), "backspace"),
            (KeyPressMsg::new(KEY_DELETE), "delete"),
            (KeyPressMsg::new(KEY_UP), "up"),
            (KeyPressMsg::new(KEY_DOWN), "down"),
            (KeyPressMsg::new(KEY_LEFT), "left"),
            (KeyPressMsg::new(KEY_RIGHT), "right"),
            (KeyPressMsg::new(KEY_PGUP), "pgup"),
            (KeyPressMsg::new(KEY_PGDOWN), "pgdown"),
            (KeyPressMsg::new(KEY_HOME), "home"),
            (KeyPressMsg::new(KEY_END), "end"),
            (KeyPressMsg::new(KEY_INSERT), "insert"),
            (KeyPressMsg::new(KEY_F5), "f5"),
            (KeyPressMsg { code: KEY_SPACE, text: " ".into(), ..Default::default() }, "space"),
            (KeyPressMsg::with_mods('c' as u32, MOD_CTRL), "ctrl+c"),
            (KeyPressMsg::with_mods('f' as u32, MOD_CTRL), "ctrl+f"),
            (KeyPressMsg::with_mods('s' as u32, MOD_CTRL), "ctrl+s"),
            (KeyPressMsg::with_mods('b' as u32, MOD_ALT), "alt+b"),
            (KeyPressMsg::with_mods(KEY_ENTER, MOD_CTRL), "ctrl+enter"),
            (KeyPressMsg::char('K'), "K"),
            (KeyPressMsg::char('?'), "?"),
            (KeyPressMsg::char('['), "["),
            (KeyPressMsg::char('@'), "@"),
            (KeyPressMsg::with_mods('a' as u32, MOD_SHIFT | MOD_CTRL | MOD_ALT), "ctrl+alt+shift+a"),
        ];
        for (k, want) in cases {
            assert_eq!(k.string(), *want);
            assert_eq!(k.to_string(), *want);
        }
    }

    #[test]
    fn parse_makes_what_a_terminal_sends() {
        for s in [
            "enter",
            "esc",
            "tab",
            "shift+tab",
            "backspace",
            "delete",
            "up",
            "down",
            "left",
            "right",
            "pgup",
            "pgdown",
            "home",
            "end",
            "space",
            "insert",
            "f5",
            "ctrl+c",
            "ctrl+f",
            "ctrl+s",
            "alt+b",
            "K",
            "?",
            "[",
            "@",
            "j",
            "ctrl+enter",
            "ctrl+space",
            "alt+backspace",
            "+",
            "_",
            ":",
        ] {
            assert_eq!(KeyPressMsg::parse(s).string(), s, "{s}");
        }
        let k = KeyPressMsg::parse("K");
        assert_eq!((k.code, k.shifted_code, k.mods, k.text.as_str()), ('k' as u32, 'K' as u32, MOD_SHIFT, "K"));
        assert_eq!(KeyPressMsg::parse("space").text, " ");
        assert_eq!(KeyPressMsg::parse("ctrl+c").text, "");
        assert_eq!(KeyPressMsg::parse("shift+tab"), KeyPressMsg::with_mods(KEY_TAB, MOD_SHIFT));
    }

    #[test]
    fn keystroke_names_shifted_letters() {
        let k = KeyPressMsg::parse("K");
        assert_eq!(k.keystroke(), "shift+k");
        let alt_shift =
            KeyPressMsg { code: 'b' as u32, shifted_code: 'B' as u32, mods: MOD_ALT | MOD_SHIFT, ..Default::default() };
        assert_eq!(alt_shift.string(), "alt+shift+b");
    }
}
