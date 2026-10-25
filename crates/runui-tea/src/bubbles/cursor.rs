//! The cursor the text input and area draw themselves (Bubbles' `cursor`):
//! the character under it in reverse video while it shows. It doesn't
//! blink here: a focused cursor shows steadily, whatever its mode says, as
//! a blinking one does between blinks.

use std::time::Duration;

use crate::style::Style;
use crate::tea::{Cmd, Msg};

const DEFAULT_BLINK_SPEED: Duration = Duration::from_millis(530);

/// Mode is how the cursor shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// It blinks (here: shows steadily).
    #[default]
    Blink,
    /// It shows steadily.
    Static,
    /// It doesn't show.
    Hide,
}

impl Mode {
    /// string is the mode's name.
    pub fn string(&self) -> &'static str {
        match self {
            Mode::Blink => "blink",
            Mode::Static => "static",
            Mode::Hide => "hidden",
        }
    }
}

/// Model is the cursor.
#[derive(Clone, Debug)]
pub struct Model {
    /// style is the cursor's style (its color, when it shows).
    pub style: Style,
    /// text_style is the character's style when the cursor doesn't show.
    pub text_style: Style,
    /// blink_speed is how fast it would blink.
    pub blink_speed: Duration,
    /// is_blinked says whether the cursor is off (the character drawn
    /// plain).
    pub is_blinked: bool,
    char: String,
    focus: bool,
    mode: Mode,
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

impl Model {
    /// new is a blinking cursor, off until focused.
    pub fn new() -> Model {
        Model {
            style: Style::new(),
            text_style: Style::new(),
            blink_speed: DEFAULT_BLINK_SPEED,
            is_blinked: true,
            char: String::new(),
            focus: false,
            mode: Mode::Blink,
        }
    }

    /// update takes a message: focus and blur.
    pub fn update(&mut self, msg: &Msg) -> Cmd {
        if msg.is::<crate::tea::FocusMsg>() {
            return self.focus();
        }
        if msg.is::<crate::tea::BlurMsg>() {
            self.blur();
        }
        None
    }

    /// mode is how the cursor shows.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// set_mode sets how the cursor shows.
    pub fn set_mode(&mut self, mode: Mode) -> Cmd {
        self.mode = mode;
        self.is_blinked = self.mode == Mode::Hide || !self.focus;
        None
    }

    /// blink would start the blinking; the cursor shows steadily here.
    pub fn blink(&mut self) -> Cmd {
        None
    }

    /// focus shows the cursor (unless it's hidden).
    pub fn focus(&mut self) -> Cmd {
        self.focus = true;
        self.is_blinked = self.mode == Mode::Hide;
        None
    }

    /// blur hides the cursor.
    pub fn blur(&mut self) {
        self.focus = false;
        self.is_blinked = true;
    }

    /// set_char sets the character under the cursor.
    pub fn set_char(&mut self, c: &str) {
        self.char = c.to_string();
    }

    /// view draws the character under the cursor: reversed while the cursor
    /// shows, in the text style when it doesn't.
    pub fn view(&self) -> String {
        if self.is_blinked {
            return self.text_style.inline(true).render(&self.char);
        }
        self.style.inline(true).reverse(true).render(&self.char)
    }
}

/// blink is the message that would start a cursor blinking: none here.
pub fn blink() -> Cmd {
    None
}
