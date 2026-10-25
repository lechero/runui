//! What the terminal sends, as Bubble Tea's messages: crossterm's events
//! turned into keys named the way Bubble Tea v2 names them, mouse events,
//! pastes, focus and resizes.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as CtButton, MouseEvent, MouseEventKind,
};

use super::key::*;
use super::mouse::*;
use super::{BlurMsg, FocusMsg, Msg, PasteMsg, WindowSizeMsg, msg};

fn mods_of(m: KeyModifiers) -> KeyMod {
    let mut k = MOD_NONE;
    if m.contains(KeyModifiers::SHIFT) {
        k |= MOD_SHIFT;
    }
    if m.contains(KeyModifiers::ALT) {
        k |= MOD_ALT;
    }
    if m.contains(KeyModifiers::CONTROL) {
        k |= MOD_CTRL;
    }
    if m.contains(KeyModifiers::META) {
        k |= MOD_META;
    }
    if m.contains(KeyModifiers::HYPER) {
        k |= MOD_HYPER;
    }
    if m.contains(KeyModifiers::SUPER) {
        k |= MOD_SUPER;
    }
    k
}

/// key_of is a crossterm key as Bubble Tea has it, or none for keys it
/// doesn't report.
pub(crate) fn key_of(e: &KeyEvent) -> Option<Key> {
    let mut mods = mods_of(e.modifiers);
    let typing = !mods.contains(MOD_CTRL) && !mods.contains(MOD_ALT) && !mods.contains(MOD_SUPER);
    let mut k = Key { is_repeat: e.kind == KeyEventKind::Repeat, ..Default::default() };
    let code = match e.code {
        KeyCode::Backspace => KEY_BACKSPACE,
        KeyCode::Enter => KEY_ENTER,
        KeyCode::Left => KEY_LEFT,
        KeyCode::Right => KEY_RIGHT,
        KeyCode::Up => KEY_UP,
        KeyCode::Down => KEY_DOWN,
        KeyCode::Home => KEY_HOME,
        KeyCode::End => KEY_END,
        KeyCode::PageUp => KEY_PGUP,
        KeyCode::PageDown => KEY_PGDOWN,
        KeyCode::Tab => KEY_TAB,
        KeyCode::BackTab => {
            mods |= MOD_SHIFT;
            KEY_TAB
        }
        KeyCode::Delete => KEY_DELETE,
        KeyCode::Insert => KEY_INSERT,
        KeyCode::F(n) if (1..=63).contains(&n) => KEY_F1 + n as u32 - 1,
        KeyCode::Esc => KEY_ESCAPE,
        KeyCode::Null => {
            mods |= MOD_CTRL;
            KEY_SPACE
        }
        KeyCode::CapsLock => KEY_CAPS_LOCK,
        KeyCode::ScrollLock => KEY_SCROLL_LOCK,
        KeyCode::NumLock => KEY_NUM_LOCK,
        KeyCode::PrintScreen => KEY_PRINT_SCREEN,
        KeyCode::Pause => KEY_PAUSE,
        KeyCode::Menu => KEY_MENU,
        KeyCode::KeypadBegin => KEY_BEGIN,
        KeyCode::Char(c) => {
            if mods.contains(MOD_CTRL) && ('4'..='7').contains(&c) {
                // ctrl+\, ctrl+], ctrl+^ and ctrl+_ come as ctrl+4 to ctrl+7
                k.code = '\\' as u32 + (c as u32 - '4' as u32);
                k.mods = mods;
                return Some(k);
            }
            if c.is_uppercase() {
                // an upper case letter is shift and the letter
                k.code = c.to_lowercase().next().unwrap_or(c) as u32;
                k.shifted_code = c as u32;
                k.mods = mods | MOD_SHIFT;
                if typing {
                    k.text = c.to_string();
                }
                return Some(k);
            }
            if c.is_alphabetic() || !typing {
                k.code = c as u32;
                k.mods = mods;
            } else {
                // shift is in the character typed ("?", "@")
                k.code = c as u32;
                k.mods = KeyMod(mods.0 & !MOD_SHIFT.0);
            }
            if typing {
                k.text = c.to_string();
            }
            return Some(k);
        }
        _ => return None,
    };
    k.code = code;
    k.mods = mods;
    Some(k)
}

fn button_of(b: CtButton) -> MouseButton {
    match b {
        CtButton::Left => MouseButton::Left,
        CtButton::Right => MouseButton::Right,
        CtButton::Middle => MouseButton::Middle,
    }
}

/// mouse_of is a crossterm mouse event as Bubble Tea's message.
fn mouse_of(e: &MouseEvent) -> Msg {
    let mut m = Mouse { x: e.column as i32, y: e.row as i32, button: MouseButton::None, mods: mods_of(e.modifiers) };
    match e.kind {
        MouseEventKind::Down(b) => {
            m.button = button_of(b);
            msg(MouseClickMsg::from(m))
        }
        MouseEventKind::Up(b) => {
            m.button = button_of(b);
            msg(MouseReleaseMsg::from(m))
        }
        MouseEventKind::Drag(b) => {
            m.button = button_of(b);
            msg(MouseMotionMsg::from(m))
        }
        MouseEventKind::Moved => msg(MouseMotionMsg::from(m)),
        MouseEventKind::ScrollDown => {
            m.button = MouseButton::WheelDown;
            msg(MouseWheelMsg::from(m))
        }
        MouseEventKind::ScrollUp => {
            m.button = MouseButton::WheelUp;
            msg(MouseWheelMsg::from(m))
        }
        MouseEventKind::ScrollLeft => {
            m.button = MouseButton::WheelLeft;
            msg(MouseWheelMsg::from(m))
        }
        MouseEventKind::ScrollRight => {
            m.button = MouseButton::WheelRight;
            msg(MouseWheelMsg::from(m))
        }
    }
}

/// msg_of is a terminal event as a message, or none.
pub(crate) fn msg_of(e: Event) -> Option<Msg> {
    match e {
        Event::Key(k) => {
            let key = key_of(&k)?;
            if k.kind == KeyEventKind::Release {
                Some(msg(KeyReleaseMsg::from(key)))
            } else {
                Some(msg(KeyPressMsg::from(key)))
            }
        }
        Event::Mouse(m) => Some(mouse_of(&m)),
        Event::Paste(s) => Some(msg(PasteMsg { content: s })),
        Event::Resize(w, h) => Some(msg(WindowSizeMsg { width: w as i32, height: h as i32 })),
        Event::FocusGained => Some(msg(FocusMsg)),
        Event::FocusLost => Some(msg(BlurMsg)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(code: KeyCode, m: KeyModifiers) -> String {
        key_of(&KeyEvent::new(code, m)).unwrap().string()
    }

    #[test]
    fn keys_named_as_bubbletea() {
        let none = KeyModifiers::NONE;
        assert_eq!(s(KeyCode::Enter, none), "enter");
        assert_eq!(s(KeyCode::Esc, none), "esc");
        assert_eq!(s(KeyCode::Tab, none), "tab");
        assert_eq!(s(KeyCode::BackTab, KeyModifiers::SHIFT), "shift+tab");
        assert_eq!(s(KeyCode::Backspace, none), "backspace");
        assert_eq!(s(KeyCode::Delete, none), "delete");
        assert_eq!(s(KeyCode::PageDown, none), "pgdown");
        assert_eq!(s(KeyCode::PageUp, none), "pgup");
        assert_eq!(s(KeyCode::F(5), none), "f5");
        assert_eq!(s(KeyCode::Char(' '), none), "space");
        assert_eq!(s(KeyCode::Char('c'), KeyModifiers::CONTROL), "ctrl+c");
        assert_eq!(s(KeyCode::Char('f'), KeyModifiers::CONTROL), "ctrl+f");
        assert_eq!(s(KeyCode::Char(' '), KeyModifiers::CONTROL), "ctrl+space");
        assert_eq!(s(KeyCode::Char('b'), KeyModifiers::ALT), "alt+b");
        assert_eq!(s(KeyCode::Char('K'), KeyModifiers::SHIFT), "K");
        assert_eq!(s(KeyCode::Char('?'), KeyModifiers::SHIFT), "?");
        assert_eq!(s(KeyCode::Char('@'), none), "@");
        assert_eq!(s(KeyCode::Char('['), none), "[");
        assert_eq!(s(KeyCode::Char('4'), KeyModifiers::CONTROL), "ctrl+\\");
        assert_eq!(s(KeyCode::Backspace, KeyModifiers::ALT), "alt+backspace");
        assert_eq!(s(KeyCode::Up, KeyModifiers::SHIFT), "shift+up");
    }

    #[test]
    fn mouse_and_paste() {
        let e = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 7,
            modifiers: KeyModifiers::NONE,
        });
        let m = msg_of(e).unwrap();
        let w = m.downcast_ref::<MouseWheelMsg>().unwrap();
        assert_eq!((w.x, w.y, w.button), (5, 7, MouseButton::WheelDown));
        let p = msg_of(Event::Paste("a\r\nb".into())).unwrap();
        assert_eq!(p.downcast_ref::<PasteMsg>().unwrap().content, "a\r\nb");
    }
}
