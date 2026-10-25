//! A one-line text field (Bubbles' `textinput`): a prompt, what's typed (or a
//! placeholder while nothing is), a cursor, editing keys as in a shell, and
//! scrolling sideways when the text is wider than the field.

use std::time::Duration;

use super::cursor::{self, Mode};
use super::key::{self, Binding};
use super::runeutil::Sanitizer;
use crate::ansi::{rune_width, uniseg_width};
use crate::style::{self, Color, Style, color};
use crate::tea::{self, Cmd, KeyPressMsg, Msg, PasteMsg};

/// pasteMsg is what the clipboard had, for a ctrl+v.
struct ClipboardMsg(String);
/// pasteErrMsg is why the clipboard couldn't be read.
struct ClipboardErrMsg(String);

/// EchoMode is how what's typed shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EchoMode {
    /// As typed.
    #[default]
    EchoNormal,
    /// One echo character for each character typed: for passwords.
    EchoPassword,
    /// Not at all.
    EchoNone,
}

pub use EchoMode::{EchoNone, EchoNormal, EchoPassword};

/// ValidateFunc checks what's typed: an error says why it isn't right.
pub type ValidateFunc = fn(&str) -> Result<(), String>;

/// KeyMap is the keys the field answers to. Its default (Go's zero value)
/// has none; [`default_key_map`] is Bubbles'.
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    /// Character forward.
    pub character_forward: Binding,
    /// Character backward.
    pub character_backward: Binding,
    /// Word forward.
    pub word_forward: Binding,
    /// Word backward.
    pub word_backward: Binding,
    /// Delete word backward.
    pub delete_word_backward: Binding,
    /// Delete word forward.
    pub delete_word_forward: Binding,
    /// Delete after cursor.
    pub delete_after_cursor: Binding,
    /// Delete before cursor.
    pub delete_before_cursor: Binding,
    /// Delete character backward.
    pub delete_character_backward: Binding,
    /// Delete character forward.
    pub delete_character_forward: Binding,
    /// Line start.
    pub line_start: Binding,
    /// Line end.
    pub line_end: Binding,
    /// Paste.
    pub paste: Binding,
    /// Accept suggestion.
    pub accept_suggestion: Binding,
    /// Next suggestion.
    pub next_suggestion: Binding,
    /// Prev suggestion.
    pub prev_suggestion: Binding,
}

/// default_key_map is Bubbles' keys: emacs's and the arrows.
pub fn default_key_map() -> KeyMap {
    let b = |keys: &[&str]| Binding::new().with_keys(keys);
    KeyMap {
        character_forward: b(&["right", "ctrl+f"]),
        character_backward: b(&["left", "ctrl+b"]),
        word_forward: b(&["alt+right", "ctrl+right", "alt+f"]),
        word_backward: b(&["alt+left", "ctrl+left", "alt+b"]),
        delete_word_backward: b(&["alt+backspace", "ctrl+w", "ctrl+backspace"]),
        delete_word_forward: b(&["alt+delete", "alt+d", "ctrl+delete"]),
        delete_after_cursor: b(&["ctrl+k"]),
        delete_before_cursor: b(&["ctrl+u"]),
        delete_character_backward: b(&["backspace", "ctrl+h"]),
        delete_character_forward: b(&["delete", "ctrl+d"]),
        line_start: b(&["home", "ctrl+a"]),
        line_end: b(&["end", "ctrl+e"]),
        paste: b(&["ctrl+v"]),
        accept_suggestion: b(&["tab"]),
        next_suggestion: b(&["down", "ctrl+n"]),
        prev_suggestion: b(&["up", "ctrl+p"]),
    }
}

/// CursorShape is the shape a real terminal cursor takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    /// Block.
    Block,
    /// Underline.
    Underline,
    /// Bar.
    Bar,
}

/// CursorStyle is how the cursor looks.
#[derive(Clone, Copy, Debug)]
pub struct CursorStyle {
    /// Color.
    pub color: Color,
    /// Shape.
    pub shape: CursorShape,
    /// Blink.
    pub blink: bool,
    /// Blink speed.
    pub blink_speed: Duration,
}

/// StyleState is the field's styles while focused, or blurred.
#[derive(Clone, Copy, Debug)]
pub struct StyleState {
    /// Text.
    pub text: Style,
    /// Placeholder.
    pub placeholder: Style,
    /// Suggestion.
    pub suggestion: Style,
    /// Prompt.
    pub prompt: Style,
}

/// Styles is how the field looks, focused and blurred.
#[derive(Clone, Copy, Debug)]
pub struct Styles {
    /// Focused.
    pub focused: StyleState,
    /// Blurred.
    pub blurred: StyleState,
    /// Cursor.
    pub cursor: CursorStyle,
}

/// default_styles is Bubbles' styles for a dark background, or a light
/// one.
pub fn default_styles(is_dark: bool) -> Styles {
    let ld = |light: &str, dark: &str| if is_dark { color(dark) } else { color(light) };
    Styles {
        focused: StyleState {
            placeholder: Style::new().foreground(color("240")),
            suggestion: Style::new().foreground(color("240")),
            prompt: Style::new().foreground(color("7")),
            text: Style::new(),
        },
        blurred: StyleState {
            placeholder: Style::new().foreground(color("240")),
            suggestion: Style::new().foreground(color("240")),
            prompt: Style::new().foreground(color("7")),
            text: Style::new().foreground(ld("245", "7")),
        },
        cursor: CursorStyle { color: color("7"), shape: CursorShape::Block, blink: true, blink_speed: Duration::ZERO },
    }
}

/// default_dark_styles is the styles for a dark background.
pub fn default_dark_styles() -> Styles {
    default_styles(true)
}

/// default_light_styles is the styles for a light background.
pub fn default_light_styles() -> Styles {
    default_styles(false)
}

/// Model is the text field.
#[derive(Clone, Debug)]
pub struct Model {
    /// err is why what's typed isn't valid, if the validate function says so.
    pub err: Option<String>,
    /// Prompt.
    pub prompt: String,
    /// Placeholder.
    pub placeholder: String,
    /// Echo mode.
    pub echo_mode: EchoMode,
    /// Echo character.
    pub echo_character: char,
    /// char_limit is the most characters it takes; 0 for no limit.
    pub char_limit: i32,
    /// Key map.
    pub key_map: KeyMap,
    /// Validate.
    pub validate: Option<ValidateFunc>,
    /// Show suggestions.
    pub show_suggestions: bool,
    use_virtual_cursor: bool,
    virtual_cursor: cursor::Model,
    styles: Styles,
    width: i32,
    value: Vec<char>,
    focus: bool,
    pos: i32,
    offset: i32,
    offset_right: i32,
    rsan: Sanitizer,
    suggestions: Vec<Vec<char>>,
    matched_suggestions: Vec<Vec<char>>,
    current_suggestion_index: i32,
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

/// new is Go's `textinput.New()`.
pub fn new() -> Model {
    Model::new()
}

fn clamp(v: i32, low: i32, high: i32) -> i32 {
    let (low, high) = if high < low { (high, low) } else { (low, high) };
    v.max(low).min(high)
}

fn is_space(c: char) -> bool {
    c.is_whitespace()
}

impl Model {
    /// new is a field with the prompt "> ", blurred, empty.
    pub fn new() -> Model {
        let mut m = Model {
            err: None,
            prompt: "> ".into(),
            placeholder: String::new(),
            echo_mode: EchoNormal,
            echo_character: '*',
            char_limit: 0,
            key_map: default_key_map(),
            validate: None,
            show_suggestions: false,
            use_virtual_cursor: true,
            virtual_cursor: cursor::Model::new(),
            styles: default_dark_styles(),
            width: 0,
            value: Vec::new(),
            focus: false,
            pos: 0,
            offset: 0,
            offset_right: 0,
            rsan: Sanitizer::new().replace_tabs(" ").replace_newlines(" "),
            suggestions: Vec::new(),
            matched_suggestions: Vec::new(),
            current_suggestion_index: 0,
        };
        m.update_virtual_cursor_style();
        m
    }

    /// virtual_cursor says whether the field draws its own cursor.
    pub fn virtual_cursor(&self) -> bool {
        self.use_virtual_cursor
    }

    /// set_virtual_cursor says whether the field draws its own cursor.
    pub fn set_virtual_cursor(&mut self, v: bool) {
        self.use_virtual_cursor = v;
        self.update_virtual_cursor_style();
    }

    /// styles is how the field looks.
    pub fn styles(&self) -> Styles {
        self.styles
    }

    /// set_styles sets how the field looks.
    pub fn set_styles(&mut self, s: Styles) {
        self.styles = s;
        self.update_virtual_cursor_style();
    }

    /// width is how many cells wide the text is shown (0: as wide as it is).
    pub fn width(&self) -> i32 {
        self.width
    }

    /// set_width sets how many cells wide the text is shown.
    pub fn set_width(&mut self, w: i32) {
        self.width = w;
    }

    /// set_value replaces what's typed.
    pub fn set_value(&mut self, s: &str) {
        let runes = self.rsan.sanitize(&s.chars().collect::<Vec<_>>());
        let err = self.run_validate(&runes);
        self.set_value_internal(runes, err);
    }

    fn set_value_internal(&mut self, runes: Vec<char>, err: Option<String>) {
        self.err = err;
        let empty = self.value.is_empty();
        if self.char_limit > 0 && runes.len() as i32 > self.char_limit {
            self.value = runes[..self.char_limit as usize].to_vec();
        } else {
            self.value = runes;
        }
        if (self.pos == 0 && empty) || self.pos > self.value.len() as i32 {
            self.set_cursor(self.value.len() as i32);
        }
        self.handle_overflow();
    }

    /// value is what's typed.
    pub fn value(&self) -> String {
        self.value.iter().collect()
    }

    /// position is where the cursor is, in characters.
    pub fn position(&self) -> i32 {
        self.pos
    }

    /// set_cursor moves the cursor.
    pub fn set_cursor(&mut self, pos: i32) {
        self.pos = clamp(pos, 0, self.value.len() as i32);
        self.handle_overflow();
    }

    /// cursor_start moves the cursor to the start.
    pub fn cursor_start(&mut self) {
        self.set_cursor(0);
    }

    /// cursor_end moves the cursor to the end.
    pub fn cursor_end(&mut self) {
        self.set_cursor(self.value.len() as i32);
    }

    /// focused says whether the field takes keys.
    pub fn focused(&self) -> bool {
        self.focus
    }

    /// focus makes the field take keys, the cursor shown.
    pub fn focus(&mut self) -> Cmd {
        self.focus = true;
        self.virtual_cursor.focus()
    }

    /// blur stops the field taking keys.
    pub fn blur(&mut self) {
        self.focus = false;
        self.virtual_cursor.blur();
    }

    /// reset empties the field.
    pub fn reset(&mut self) {
        self.value.clear();
        self.set_cursor(0);
    }

    /// set_suggestions sets what the field may complete to.
    pub fn set_suggestions(&mut self, suggestions: &[&str]) {
        self.suggestions = suggestions.iter().map(|s| s.chars().collect()).collect();
        self.update_suggestions();
    }

    fn insert_runes_from_user_input(&mut self, v: &[char]) {
        let mut paste = self.rsan.sanitize(v);
        let mut avail_space = 0;
        if self.char_limit > 0 {
            avail_space = self.char_limit - self.value.len() as i32;
            if avail_space <= 0 {
                return;
            }
            if (avail_space as usize) < paste.len() {
                paste.truncate(avail_space as usize);
            }
        }
        let pos = self.pos as usize;
        let mut head: Vec<char> = self.value[..pos].to_vec();
        let tail: Vec<char> = self.value[pos..].to_vec();
        for r in paste {
            head.push(r);
            self.pos += 1;
            if self.char_limit > 0 {
                avail_space -= 1;
                if avail_space <= 0 {
                    break;
                }
            }
        }
        head.extend(tail);
        let err = self.run_validate(&head);
        self.set_value_internal(head, err);
    }

    fn handle_overflow(&mut self) {
        let len = self.value.len() as i32;
        let s: String = self.value.iter().collect();
        if self.width <= 0 || uniseg_width(&s) <= self.width {
            self.offset = 0;
            self.offset_right = len;
            return;
        }
        // Correct right offset if we've deleted characters
        self.offset_right = self.offset_right.min(len);
        if self.pos < self.offset {
            self.offset = self.pos;
            let mut w = 0;
            let mut i = 0;
            let runes = &self.value[self.offset as usize..];
            while (i as usize) < runes.len() && w <= self.width {
                w += rune_width(runes[i as usize]);
                if w <= self.width + 1 {
                    i += 1;
                }
            }
            self.offset_right = self.offset + i;
        } else if self.pos >= self.offset_right {
            self.offset_right = self.pos;
            let mut w = 0;
            let runes = &self.value[..self.offset_right as usize];
            let mut i = runes.len() as i32 - 1;
            while i > 0 && w < self.width {
                w += rune_width(runes[i as usize]);
                if w <= self.width {
                    i -= 1;
                }
            }
            self.offset = self.offset_right - (runes.len() as i32 - 1 - i);
        }
    }

    fn delete_before_cursor(&mut self) {
        self.value = self.value[self.pos as usize..].to_vec();
        self.err = self.run_validate(&self.value);
        self.offset = 0;
        self.set_cursor(0);
    }

    fn delete_after_cursor(&mut self) {
        self.value.truncate(self.pos as usize);
        self.err = self.run_validate(&self.value);
        self.set_cursor(self.value.len() as i32);
    }

    fn delete_word_backward(&mut self) {
        if self.pos == 0 || self.value.is_empty() {
            return;
        }
        if self.echo_mode != EchoNormal {
            self.delete_before_cursor();
            return;
        }
        // Linter note: it's critical that we acquire the initial cursor
        // position here prior to altering it via set_cursor() below.
        let old_pos = self.pos;
        self.set_cursor(self.pos - 1);
        while is_space(self.value[self.pos as usize]) {
            if self.pos <= 0 {
                break;
            }
            // ignore series of whitespace before cursor
            self.set_cursor(self.pos - 1);
        }
        while self.pos > 0 {
            if !is_space(self.value[self.pos as usize]) {
                self.set_cursor(self.pos - 1);
            } else {
                if self.pos > 0 {
                    // keep the previous space
                    self.set_cursor(self.pos + 1);
                }
                break;
            }
        }
        if old_pos > self.value.len() as i32 {
            self.value.truncate(self.pos as usize);
        } else {
            let tail = self.value[old_pos as usize..].to_vec();
            self.value.truncate(self.pos as usize);
            self.value.extend(tail);
        }
        self.err = self.run_validate(&self.value);
    }

    fn delete_word_forward(&mut self) {
        let len = self.value.len() as i32;
        if self.pos >= len || self.value.is_empty() {
            return;
        }
        if self.echo_mode != EchoNormal {
            self.delete_after_cursor();
            return;
        }
        let old_pos = self.pos;
        self.set_cursor(self.pos + 1);
        while self.pos < self.value.len() as i32 && is_space(self.value[self.pos as usize]) {
            // ignore series of whitespace after cursor
            self.set_cursor(self.pos + 1);
            if self.pos >= self.value.len() as i32 {
                break;
            }
        }
        while self.pos < self.value.len() as i32 {
            if !is_space(self.value[self.pos as usize]) {
                self.set_cursor(self.pos + 1);
            } else {
                break;
            }
        }
        if self.pos > self.value.len() as i32 {
            self.value.truncate(old_pos as usize);
        } else {
            let tail = self.value[self.pos as usize..].to_vec();
            self.value.truncate(old_pos as usize);
            self.value.extend(tail);
        }
        self.err = self.run_validate(&self.value);
        self.set_cursor(old_pos);
    }

    fn word_backward(&mut self) {
        if self.pos == 0 || self.value.is_empty() {
            return;
        }
        if self.echo_mode != EchoNormal {
            self.cursor_start();
            return;
        }
        let mut i = self.pos - 1;
        while i >= 0 {
            if is_space(self.value[i as usize]) {
                self.set_cursor(self.pos - 1);
                i -= 1;
            } else {
                break;
            }
        }
        while i >= 0 {
            if !is_space(self.value[i as usize]) {
                self.set_cursor(self.pos - 1);
                i -= 1;
            } else {
                break;
            }
        }
    }

    fn word_forward(&mut self) {
        let len = self.value.len() as i32;
        if self.pos >= len || self.value.is_empty() {
            return;
        }
        if self.echo_mode != EchoNormal {
            self.cursor_end();
            return;
        }
        let mut i = self.pos;
        while i < len {
            if is_space(self.value[i as usize]) {
                self.set_cursor(self.pos + 1);
                i += 1;
            } else {
                break;
            }
        }
        while i < len {
            if !is_space(self.value[i as usize]) {
                self.set_cursor(self.pos + 1);
                i += 1;
            } else {
                break;
            }
        }
    }

    fn echo_transform(&self, v: &str) -> String {
        match self.echo_mode {
            EchoPassword => self.echo_character.to_string().repeat(uniseg_width(v).max(0) as usize),
            EchoNone => String::new(),
            EchoNormal => v.to_string(),
        }
    }

    /// update takes a message: keys edit what's typed while the field is
    /// focused, and pastes go in.
    pub fn update(&mut self, msg: &Msg) -> Cmd {
        if !self.focus {
            return None;
        }
        let key_msg = msg.downcast_ref::<KeyPressMsg>();
        // Need to check for completion before, because key is configurable
        // and may be double assigned
        if let Some(k) = key_msg
            && key::matches(k, &[&self.key_map.accept_suggestion])
            && self.can_accept_suggestion()
        {
            let s = &self.matched_suggestions[self.current_suggestion_index as usize];
            let rest: Vec<char> = s[self.value.len().min(s.len())..].to_vec();
            self.value.extend(rest);
            self.cursor_end();
        }

        if let Some(k) = key_msg {
            let km = self.key_map.clone();
            if key::matches(k, &[&km.delete_word_backward]) {
                self.delete_word_backward();
            } else if key::matches(k, &[&km.delete_character_backward]) {
                self.err = None;
                if !self.value.is_empty() {
                    let pos = self.pos as usize;
                    let at = pos.saturating_sub(1);
                    let tail = self.value[pos..].to_vec();
                    self.value.truncate(at);
                    self.value.extend(tail);
                    self.err = self.run_validate(&self.value);
                    if self.pos > 0 {
                        self.set_cursor(self.pos - 1);
                    }
                }
            } else if key::matches(k, &[&km.word_backward]) {
                self.word_backward();
            } else if key::matches(k, &[&km.character_backward]) {
                if self.pos > 0 {
                    self.set_cursor(self.pos - 1);
                }
            } else if key::matches(k, &[&km.word_forward]) {
                self.word_forward();
            } else if key::matches(k, &[&km.character_forward]) {
                if self.pos < self.value.len() as i32 {
                    self.set_cursor(self.pos + 1);
                }
            } else if key::matches(k, &[&km.line_start]) {
                self.cursor_start();
            } else if key::matches(k, &[&km.delete_character_forward]) {
                if !self.value.is_empty() && self.pos < self.value.len() as i32 {
                    self.value.remove(self.pos as usize);
                    self.err = self.run_validate(&self.value);
                }
            } else if key::matches(k, &[&km.line_end]) {
                self.cursor_end();
            } else if key::matches(k, &[&km.delete_after_cursor]) {
                self.delete_after_cursor();
            } else if key::matches(k, &[&km.delete_before_cursor]) {
                self.delete_before_cursor();
            } else if key::matches(k, &[&km.paste]) {
                return paste();
            } else if key::matches(k, &[&km.delete_word_forward]) {
                self.delete_word_forward();
            } else if key::matches(k, &[&km.next_suggestion]) {
                self.next_suggestion();
            } else if key::matches(k, &[&km.prev_suggestion]) {
                self.previous_suggestion();
            } else {
                // Input one or more regular characters.
                let runes: Vec<char> = k.text.chars().collect();
                self.insert_runes_from_user_input(&runes);
            }
            // Check again if can be completed because value changed
            self.update_suggestions();
        } else if let Some(p) = msg.downcast_ref::<PasteMsg>() {
            let runes: Vec<char> = p.content.chars().collect();
            self.insert_runes_from_user_input(&runes);
        } else if let Some(p) = msg.downcast_ref::<ClipboardMsg>() {
            let runes: Vec<char> = p.0.chars().collect();
            self.insert_runes_from_user_input(&runes);
        } else if let Some(e) = msg.downcast_ref::<ClipboardErrMsg>() {
            self.err = Some(e.0.clone());
        }

        let mut cmds = Vec::new();
        if self.use_virtual_cursor {
            cmds.push(self.virtual_cursor.update(msg));
        }
        self.handle_overflow();
        tea::batch(cmds)
    }

    /// view draws the prompt and the field.
    pub fn view(&self) -> String {
        // Placeholder text
        if self.value.is_empty() && !self.placeholder.is_empty() {
            return self.placeholder_view();
        }
        let styles = self.active_style();
        let style_text = |s: &str| styles.text.inline(true).render(s);
        let mut vc = self.virtual_cursor.clone();

        let right = (self.offset_right.max(0) as usize).min(self.value.len());
        let value = &self.value[(self.offset.max(0) as usize).min(right)..right];
        let pos = (self.pos - self.offset).max(0) as usize;
        let mut v = style_text(&self.echo_transform(&value[..pos.min(value.len())].iter().collect::<String>()));

        if pos < value.len() {
            let ch = self.echo_transform(&value[pos].to_string());
            vc.set_char(&ch);
            v += &vc.view(); // cursor and text under it
            v += &style_text(&self.echo_transform(&value[pos + 1..].iter().collect::<String>())); // text after cursor
            v += &self.completion_view(0); // suggested completion
        } else if self.focus && self.can_accept_suggestion() {
            let suggestion = &self.matched_suggestions[self.current_suggestion_index as usize];
            if value.len() < suggestion.len() {
                vc.text_style = styles.suggestion;
                vc.set_char(&self.echo_transform(&suggestion[pos].to_string()));
                v += &vc.view();
                v += &self.completion_view(1);
            } else {
                vc.set_char(" ");
                v += &vc.view();
            }
        } else {
            vc.set_char(" ");
            v += &vc.view();
        }

        // If a max width and background color were set fill the empty
        // spaces with the background color.
        let val_width = uniseg_width(&value.iter().collect::<String>());
        if self.width > 0 && val_width <= self.width {
            let mut padding = (self.width - val_width).max(0);
            if val_width + padding <= self.width && pos < value.len() {
                padding += 1;
            }
            v += &style_text(&" ".repeat(padding as usize));
        }

        self.prompt_view() + &v
    }

    fn prompt_view(&self) -> String {
        self.active_style().prompt.render(&self.prompt)
    }

    fn placeholder_view(&self) -> String {
        let styles = self.active_style();
        let render = |s: &str| styles.placeholder.render(s);
        let mut p: Vec<char> = vec!['\0'; (self.width + 1).max(0) as usize];
        for (i, c) in self.placeholder.chars().enumerate() {
            if i < p.len() {
                p[i] = c;
            }
        }
        let mut vc = self.virtual_cursor.clone();
        vc.text_style = styles.placeholder;
        vc.set_char(&p[..1.min(p.len())].iter().collect::<String>());
        let mut v = vc.view();

        // If the entire placeholder is already set and no padding is
        // needed, return the placeholder.
        if self.width < 1 && p.len() <= 1 {
            return styles.prompt.render(&self.prompt) + &v;
        }

        // If there is a max width, fill the empty spaces with the
        // background color.
        if self.width > 0 {
            // Unless the placeholder is longer than the max width, the
            // placeholder's width is the same as the max width
            let mut min_width = style::width(&self.placeholder);
            let mut avail_width = self.width - min_width + 1;
            // if the placeholder is longer than the max width, cut it
            if avail_width < 0 {
                min_width += avail_width;
                avail_width = 0;
            }
            // the first placeholder character is the cursor
            let end = (min_width.max(1) as usize).min(p.len());
            v += &render(&p[1.min(end)..end].iter().collect::<String>());
            v += &render(&" ".repeat(avail_width as usize));
        } else {
            // if there's no width, the placeholder can be any length
            let rest: String = self.placeholder.chars().skip(1).collect();
            v += &render(&rest);
        }
        styles.prompt.render(&self.prompt) + &v
    }

    fn completion_view(&self, offset: usize) -> String {
        if !self.can_accept_suggestion() {
            return String::new();
        }
        let suggestion = &self.matched_suggestions[self.current_suggestion_index as usize];
        if self.value.len() < suggestion.len() {
            let from = (self.value.len() + offset).min(suggestion.len());
            return self.active_style().suggestion.inline(true).render(&suggestion[from..].iter().collect::<String>());
        }
        String::new()
    }

    /// available_suggestions is every suggestion.
    pub fn available_suggestions(&self) -> Vec<String> {
        self.suggestions.iter().map(|s| s.iter().collect()).collect()
    }

    /// matched_suggestions is the suggestions what's typed starts.
    pub fn matched_suggestions(&self) -> Vec<String> {
        self.matched_suggestions.iter().map(|s| s.iter().collect()).collect()
    }

    /// current_suggestion_index is which matched suggestion is shown.
    pub fn current_suggestion_index(&self) -> i32 {
        self.current_suggestion_index
    }

    /// current_suggestion is the matched suggestion shown.
    pub fn current_suggestion(&self) -> String {
        self.matched_suggestions
            .get(self.current_suggestion_index as usize)
            .map(|s| s.iter().collect())
            .unwrap_or_default()
    }

    fn can_accept_suggestion(&self) -> bool {
        !self.matched_suggestions.is_empty()
    }

    fn update_suggestions(&mut self) {
        if !self.show_suggestions {
            return;
        }
        if self.value.is_empty() || self.suggestions.is_empty() {
            self.matched_suggestions.clear();
            return;
        }
        let value: String = self.value.iter().collect::<String>().to_lowercase();
        let matches: Vec<Vec<char>> = self
            .suggestions
            .iter()
            .filter(|s| s.iter().collect::<String>().to_lowercase().starts_with(&value))
            .cloned()
            .collect();
        if matches != self.matched_suggestions {
            self.current_suggestion_index = 0;
        }
        self.matched_suggestions = matches;
    }

    fn next_suggestion(&mut self) {
        self.current_suggestion_index += 1;
        if self.current_suggestion_index >= self.matched_suggestions.len() as i32 {
            self.current_suggestion_index = 0;
        }
    }

    fn previous_suggestion(&mut self) {
        self.current_suggestion_index -= 1;
        if self.current_suggestion_index < 0 {
            self.current_suggestion_index = self.matched_suggestions.len() as i32 - 1;
        }
    }

    fn run_validate(&self, v: &[char]) -> Option<String> {
        self.validate.and_then(|f| f(&v.iter().collect::<String>()).err())
    }

    fn update_virtual_cursor_style(&mut self) {
        if !self.use_virtual_cursor {
            self.virtual_cursor.set_mode(Mode::Hide);
            return;
        }
        self.virtual_cursor.style = Style::new().foreground(self.styles.cursor.color);
        if self.styles.cursor.blink {
            if self.styles.cursor.blink_speed > Duration::ZERO {
                self.virtual_cursor.blink_speed = self.styles.cursor.blink_speed;
            }
            self.virtual_cursor.set_mode(Mode::Blink);
            return;
        }
        self.virtual_cursor.set_mode(Mode::Static);
    }

    fn active_style(&self) -> &StyleState {
        if self.focus { &self.styles.focused } else { &self.styles.blurred }
    }
}

/// blink is the message that starts the cursor blinking: none here, the
/// cursor shows steadily.
pub fn blink() -> Cmd {
    cursor::blink()
}

/// paste is the command that reads the clipboard into the field.
pub fn paste() -> Cmd {
    tea::cmd(|| {
        Some(match super::clipboard_read() {
            Ok(s) => tea::msg(ClipboardMsg(s)),
            Err(e) => tea::msg(ClipboardErrMsg(e)),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi;

    fn key(s: &str) -> Msg {
        tea::msg(KeyPressMsg::parse(s))
    }

    fn typed(m: &mut Model, s: &str) {
        for c in s.chars() {
            m.update(&tea::msg(KeyPressMsg::char(c)));
        }
    }

    #[test]
    fn current_suggestion() {
        let mut m = Model::new();
        m.show_suggestions = true;
        assert_eq!(m.current_suggestion(), "");
        m.set_suggestions(&["test1", "test2", "test3"]);
        assert_eq!(m.current_suggestion(), "");
        m.set_value("test");
        m.update_suggestions();
        m.next_suggestion();
        assert_eq!(m.current_suggestion(), "test2");
        m.blur();
        // suggestions don't show while the field isn't focused
        assert!(!m.view().ends_with("test2"), "{:?}", m.view());
    }

    #[test]
    fn slicing_outside_cap() {
        let mut m = Model::new();
        m.placeholder = "作業ディレクトリを指定してください".into();
        m.set_width(32);
        let _ = m.view();
    }

    #[test]
    fn typing_and_editing() {
        let mut m = Model::new();
        m.focus();
        typed(&mut m, "hello world");
        assert_eq!(m.value(), "hello world");
        assert_eq!(m.position(), 11);
        m.update(&key("alt+backspace"));
        assert_eq!(m.value(), "hello ");
        m.update(&key("ctrl+a"));
        assert_eq!(m.position(), 0);
        m.update(&key("ctrl+k"));
        assert_eq!(m.value(), "");
        typed(&mut m, "a b");
        m.update(&key("left"));
        m.update(&key("backspace"));
        assert_eq!(m.value(), "ab");
        m.update(&tea::msg(PasteMsg { content: "x\ny".into() }));
        assert_eq!(m.value(), "ax yb");
    }

    #[test]
    fn blurred_ignores_keys() {
        let mut m = Model::new();
        typed(&mut m, "abc");
        assert_eq!(m.value(), "");
    }

    #[test]
    fn char_limit_and_password() {
        let mut m = Model::new();
        m.char_limit = 3;
        m.echo_mode = EchoPassword;
        m.focus();
        typed(&mut m, "secret");
        assert_eq!(m.value(), "sec");
        assert_eq!(ansi::strip(&m.view()), "> *** ");
    }

    #[test]
    fn scrolls_sideways() {
        let mut m = Model::new();
        m.set_width(5);
        m.focus();
        typed(&mut m, "abcdefghij");
        let v = ansi::strip(&m.view());
        assert!(v.starts_with("> ") && v.contains("ghij"), "{v:?}");
        assert!(!v.contains("abc"), "{v:?}");
    }

    #[test]
    fn steady_cursor_renders_reversed() {
        let mut m = Model::new();
        m.prompt = String::new();
        m.focus();
        typed(&mut m, "ab");
        // the empty prompt is still styled, as Lip Gloss renders it
        assert_eq!(m.view(), "\x1b[37m\x1b[mab\x1b[7;37m \x1b[m");
        m.blur();
        assert_eq!(m.view(), "\x1b[37m\x1b[m\x1b[37mab\x1b[m ");
    }
}
