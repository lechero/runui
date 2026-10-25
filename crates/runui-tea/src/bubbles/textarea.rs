//! A text area of many lines (Bubbles' `textarea`): line numbers and a
//! prompt on the left, soft-wrapped lines, a cursor moved and text edited
//! with emacs's keys and the arrows, a selection with shift, a placeholder
//! while it's empty, scrolled to keep the cursor in view.

use std::cell::RefCell;
use std::time::Duration;

use unicode_segmentation::UnicodeSegmentation;

use super::cursor::{self, Mode};
use super::key::{self, Binding};
use super::runeutil::Sanitizer;
use super::viewport;
use crate::ansi::{self, rune_width, uniseg_width};
use crate::style::{self, Color, Style, color, thick_border};
use crate::tea::{self, Cmd, KeyPressMsg, Msg, PasteMsg};

const MIN_HEIGHT: i32 = 1;
const DEFAULT_HEIGHT: i32 = 6;
const DEFAULT_WIDTH: i32 = 40;
const DEFAULT_CHAR_LIMIT: i32 = 0; // no limit
const DEFAULT_MAX_HEIGHT: i32 = 99;
const DEFAULT_MAX_WIDTH: i32 = 500;
// XXX: in v2, make max lines dynamic and default max lines configurable.
const MAX_LINES: i32 = 10000;

struct ClipboardMsg(String);
struct ClipboardErrMsg(String);
struct CopyMsg(#[allow(dead_code)] String);
struct CopyErrMsg(String);

/// KeyMap is the keys the area answers to. Its default (Go's zero value)
/// has none; [`default_key_map`] is Bubbles'.
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    /// Character backward.
    pub character_backward: Binding,
    /// Character forward.
    pub character_forward: Binding,
    /// Delete after cursor.
    pub delete_after_cursor: Binding,
    /// Delete before cursor.
    pub delete_before_cursor: Binding,
    /// Delete character backward.
    pub delete_character_backward: Binding,
    /// Delete character forward.
    pub delete_character_forward: Binding,
    /// Delete word backward.
    pub delete_word_backward: Binding,
    /// Delete word forward.
    pub delete_word_forward: Binding,
    /// Insert newline.
    pub insert_newline: Binding,
    /// Line end.
    pub line_end: Binding,
    /// Line next.
    pub line_next: Binding,
    /// Line previous.
    pub line_previous: Binding,
    /// Line start.
    pub line_start: Binding,
    /// Page up.
    pub page_up: Binding,
    /// Page down.
    pub page_down: Binding,
    /// Paste.
    pub paste: Binding,
    /// Word backward.
    pub word_backward: Binding,
    /// Word forward.
    pub word_forward: Binding,
    /// Input begin.
    pub input_begin: Binding,
    /// Input end.
    pub input_end: Binding,
    /// Uppercase word forward.
    pub uppercase_word_forward: Binding,
    /// Lowercase word forward.
    pub lowercase_word_forward: Binding,
    /// Capitalize word forward.
    pub capitalize_word_forward: Binding,
    /// Transpose character backward.
    pub transpose_character_backward: Binding,
    /// Select character forward.
    pub select_character_forward: Binding,
    /// Select character backward.
    pub select_character_backward: Binding,
    /// Select word forward.
    pub select_word_forward: Binding,
    /// Select word backward.
    pub select_word_backward: Binding,
    /// Select line up.
    pub select_line_up: Binding,
    /// Select line down.
    pub select_line_down: Binding,
    /// Select all.
    pub select_all: Binding,
    /// Copy selection.
    pub copy_selection: Binding,
}

/// default_key_map is Bubbles' keys.
pub fn default_key_map() -> KeyMap {
    let b = |keys: &[&str], h: &str, d: &str| Binding::new().with_keys(keys).with_help(h, d);
    KeyMap {
        character_forward: b(&["right", "ctrl+f"], "right", "character forward"),
        character_backward: b(&["left", "ctrl+b"], "left", "character backward"),
        word_forward: b(&["alt+right", "ctrl+right", "alt+f"], "alt+right", "word forward"),
        word_backward: b(&["alt+left", "ctrl+left", "alt+b"], "alt+left", "word backward"),
        line_next: b(&["down", "ctrl+n"], "down", "next line"),
        line_previous: b(&["up", "ctrl+p"], "up", "previous line"),
        delete_word_backward: b(
            &["alt+backspace", "ctrl+w", "ctrl+backspace"],
            "alt+backspace",
            "delete word backward",
        ),
        delete_word_forward: b(&["alt+delete", "alt+d", "ctrl+delete"], "alt+delete", "delete word forward"),
        delete_after_cursor: b(&["ctrl+k"], "ctrl+k", "delete after cursor"),
        delete_before_cursor: b(&["ctrl+u"], "ctrl+u", "delete before cursor"),
        insert_newline: b(&["enter", "ctrl+m"], "enter", "insert newline"),
        delete_character_backward: b(&["backspace", "ctrl+h"], "backspace", "delete character backward"),
        delete_character_forward: b(&["delete", "ctrl+d"], "delete", "delete character forward"),
        line_start: b(&["home", "ctrl+a"], "home", "line start"),
        line_end: b(&["end", "ctrl+e"], "end", "line end"),
        page_up: b(&["pgup"], "pgup", "page up"),
        page_down: b(&["pgdown"], "pgdown", "page down"),
        paste: b(&["ctrl+v"], "ctrl+v", "paste"),
        input_begin: b(&["alt+<", "ctrl+home"], "alt+<", "input begin"),
        input_end: b(&["alt+>", "ctrl+end"], "alt+>", "input end"),
        capitalize_word_forward: b(&["alt+c"], "alt+c", "capitalize word forward"),
        lowercase_word_forward: b(&["alt+l"], "alt+l", "lowercase word forward"),
        uppercase_word_forward: b(&["alt+u"], "alt+u", "uppercase word forward"),
        transpose_character_backward: b(&["ctrl+t"], "ctrl+t", "transpose character backward"),
        select_character_forward: b(&["shift+right"], "shift+right", "select character forward"),
        select_character_backward: b(&["shift+left"], "shift+left", "select character backward"),
        select_word_forward: b(
            &["ctrl+shift+right", "alt+shift+right", "alt+shift+f"],
            "alt+shift+right",
            "select word forward",
        ),
        select_word_backward: b(
            &["ctrl+shift+left", "alt+shift+left", "alt+shift+b"],
            "alt+shift+left",
            "select word backward",
        ),
        select_line_up: b(&["shift+up"], "shift+up", "select line up"),
        select_line_down: b(&["shift+down"], "shift+down", "select line down"),
        select_all: b(&["ctrl+g"], "ctrl+g", "select all"),
        copy_selection: b(&["ctrl+shift+c"], "ctrl+shift+c", "copy selection"),
    }
}

/// LineInfo is where the cursor is in its line, soft wraps counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineInfo {
    /// width is how many characters the cursor's wrapped line has.
    pub width: i32,
    /// char_width is how many cells it takes.
    pub char_width: i32,
    /// height is how many wrapped lines the line takes.
    pub height: i32,
    /// start_column is where the wrapped line starts in the line.
    pub start_column: i32,
    /// column_offset is the cursor's column in the wrapped line.
    pub column_offset: i32,
    /// row_offset is which wrapped line the cursor is on.
    pub row_offset: i32,
    /// char_offset is the cursor's column in cells.
    pub char_offset: i32,
}

/// PromptInfo is what a prompt function is told.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PromptInfo {
    /// Line number.
    pub line_number: i32,
    /// Focused.
    pub focused: bool,
}

/// CursorStyle is how the cursor looks.
#[derive(Clone, Copy, Debug)]
pub struct CursorStyle {
    /// Color.
    pub color: Color,
    /// Shape.
    pub shape: super::textinput::CursorShape,
    /// Blink.
    pub blink: bool,
    /// Blink speed.
    pub blink_speed: Duration,
}

/// StyleState is the area's styles while focused, or blurred.
#[derive(Clone, Copy, Debug)]
pub struct StyleState {
    /// Base.
    pub base: Style,
    /// Text.
    pub text: Style,
    /// Line number.
    pub line_number: Style,
    /// Cursor line number.
    pub cursor_line_number: Style,
    /// Cursor line.
    pub cursor_line: Style,
    /// End of buffer.
    pub end_of_buffer: Style,
    /// Placeholder.
    pub placeholder: Style,
    /// Prompt.
    pub prompt: Style,
    /// Selection.
    pub selection: Style,
}

impl StyleState {
    fn computed_cursor_line(&self) -> Style {
        self.cursor_line.inherit(self.base).inline(true)
    }
    fn computed_cursor_line_number(&self) -> Style {
        self.cursor_line_number.inherit(self.cursor_line).inherit(self.base).inline(true)
    }
    fn computed_end_of_buffer(&self) -> Style {
        self.end_of_buffer.inherit(self.base).inline(true)
    }
    fn computed_line_number(&self) -> Style {
        self.line_number.inherit(self.base).inline(true)
    }
    fn computed_placeholder(&self) -> Style {
        self.placeholder.inherit(self.base).inline(true)
    }
    fn computed_prompt(&self) -> Style {
        self.prompt.inherit(self.base).inline(true)
    }
    fn computed_text(&self) -> Style {
        self.text.inherit(self.base).inline(true)
    }
    fn computed_selection(&self) -> Style {
        self.selection.inherit(self.text).inherit(self.base).inline(true)
    }
}

/// Styles is how the area looks, focused and blurred.
#[derive(Clone, Copy, Debug)]
pub struct Styles {
    /// Focused.
    pub focused: StyleState,
    /// Blurred.
    pub blurred: StyleState,
    /// Cursor.
    pub cursor: CursorStyle,
}

/// default_styles is Bubbles' styles for a dark background, or a light one.
pub fn default_styles(is_dark: bool) -> Styles {
    let ld = |light: &str, dark: &str| if is_dark { color(dark) } else { color(light) };
    let s = Style::new();
    Styles {
        focused: StyleState {
            base: s,
            cursor_line: s.background(ld("255", "0")),
            cursor_line_number: s.foreground(ld("240", "240")),
            end_of_buffer: s.foreground(ld("254", "0")),
            line_number: s.foreground(ld("249", "7")),
            placeholder: s.foreground(color("240")),
            prompt: s.foreground(color("7")),
            selection: s.background(ld("253", "8")),
            text: s,
        },
        blurred: StyleState {
            base: s,
            cursor_line: s.foreground(ld("245", "7")),
            cursor_line_number: s.foreground(ld("249", "7")),
            end_of_buffer: s.foreground(ld("254", "0")),
            line_number: s.foreground(ld("249", "7")),
            placeholder: s.foreground(color("240")),
            prompt: s.foreground(color("7")),
            selection: s.background(ld("253", "8")),
            text: s.foreground(ld("245", "7")),
        },
        cursor: CursorStyle {
            color: color("7"),
            shape: super::textinput::CursorShape::Block,
            blink: true,
            blink_speed: Duration::ZERO,
        },
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

/// Position is a place in the text: a line and a column, in characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Position {
    /// Row.
    pub row: i32,
    /// Col.
    pub col: i32,
}

impl Position {
    fn before(&self, q: &Position) -> bool {
        if self.row != q.row {
            return self.row < q.row;
        }
        self.col < q.col
    }
}

/// Model is the text area.
#[derive(Clone, Debug)]
pub struct Model {
    /// Err.
    pub err: Option<String>,
    /// Prompt.
    pub prompt: String,
    /// Placeholder.
    pub placeholder: String,
    /// Show line numbers.
    pub show_line_numbers: bool,
    /// End of buffer character.
    pub end_of_buffer_character: char,
    /// Key map.
    pub key_map: KeyMap,
    /// char_limit is the most characters it takes; 0 for no limit.
    pub char_limit: i32,
    /// max_height is the most lines it grows to (0 for no limit).
    pub max_height: i32,
    /// max_width is the most cells wide it grows to (0 for no limit).
    pub max_width: i32,
    /// dynamic_height grows and shrinks the area with what's in it.
    pub dynamic_height: bool,
    /// Min height.
    pub min_height: i32,
    /// max_content_height is the most wrapped lines the content may take
    /// (0 for no limit).
    pub max_content_height: i32,
    virtual_cursor: cursor::Model,
    styles: Styles,
    use_virtual_cursor: bool,
    prompt_func: Option<fn(PromptInfo) -> String>,
    prompt_width: i32,
    width: i32,
    height: i32,
    value: Vec<Vec<char>>,
    focus: bool,
    col: i32,
    row: i32,
    last_char_offset: i32,
    viewport: RefCell<viewport::Model>,
    rsan: Sanitizer,
    sel_anchor: Position,
    sel_head: Position,
    has_selection: bool,
    selecting: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

fn clamp(v: i32, low: i32, high: i32) -> i32 {
    let (low, high) = if high < low { (high, low) } else { (low, high) };
    v.max(low).min(high)
}

fn is_space(c: char) -> bool {
    c.is_whitespace()
}

fn width_of(r: &[char]) -> i32 {
    uniseg_width(&r.iter().collect::<String>())
}

fn num_digits(n: i32) -> i32 {
    if n == 0 {
        return 1;
    }
    let mut count = 0;
    let mut num = n.abs();
    while num > 0 {
        count += 1;
        num /= 10;
    }
    count
}

/// new is Go's `textarea.New()`.
pub fn new() -> Model {
    Model::new()
}

impl Model {
    /// new is an area 40 wide and 6 tall, with line numbers and the thick
    /// border's left side as its prompt, blurred.
    pub fn new() -> Model {
        let mut vp = viewport::Model::new();
        vp.key_map = viewport::KeyMap::default();
        let mut m = Model {
            err: None,
            prompt: format!("{} ", thick_border().left),
            placeholder: String::new(),
            show_line_numbers: true,
            end_of_buffer_character: ' ',
            key_map: default_key_map(),
            char_limit: DEFAULT_CHAR_LIMIT,
            max_height: DEFAULT_MAX_HEIGHT,
            max_width: DEFAULT_MAX_WIDTH,
            dynamic_height: false,
            min_height: 0,
            max_content_height: 0,
            virtual_cursor: cursor::Model::new(),
            styles: default_dark_styles(),
            use_virtual_cursor: true,
            prompt_func: None,
            prompt_width: 0,
            width: 0,
            height: 0,
            value: vec![Vec::new()],
            focus: false,
            col: 0,
            row: 0,
            last_char_offset: 0,
            viewport: RefCell::new(vp),
            rsan: Sanitizer::new(),
            sel_anchor: Position::default(),
            sel_head: Position::default(),
            has_selection: false,
            selecting: false,
        };
        m.set_height(DEFAULT_HEIGHT);
        m.set_width(DEFAULT_WIDTH);
        m
    }

    /// styles is how the area looks.
    pub fn styles(&self) -> Styles {
        self.styles
    }

    /// set_styles sets how the area looks.
    pub fn set_styles(&mut self, s: Styles) {
        self.styles = s;
        self.update_virtual_cursor_style();
    }

    /// virtual_cursor says whether the area draws its own cursor.
    pub fn virtual_cursor(&self) -> bool {
        self.use_virtual_cursor
    }

    /// set_virtual_cursor says whether the area draws its own cursor.
    pub fn set_virtual_cursor(&mut self, v: bool) {
        self.use_virtual_cursor = v;
        self.update_virtual_cursor_style();
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

    /// set_value replaces the text.
    pub fn set_value(&mut self, s: &str) {
        self.reset();
        self.insert_string(s);
        self.recalculate_height();
    }

    /// insert_string types `s` at the cursor.
    pub fn insert_string(&mut self, s: &str) {
        self.insert_runes_from_user_input(&s.chars().collect::<Vec<_>>());
        self.recalculate_height();
    }

    /// insert_rune types `r` at the cursor.
    pub fn insert_rune(&mut self, r: char) {
        self.insert_runes_from_user_input(&[r]);
        self.recalculate_height();
    }

    fn insert_runes_from_user_input(&mut self, runes: &[char]) {
        // Clean up any special characters in the input provided by the
        // clipboard. This avoids bugs due to e.g. tab characters and
        // whatnot.
        let mut runes = self.rsan.sanitize(runes);
        if self.char_limit > 0 {
            let avail_space = self.char_limit - self.length();
            // If the char limit's been reached, cancel.
            if avail_space <= 0 {
                return;
            }
            // If there's not enough space to paste the whole thing cut the
            // pasted runes down so they'll fit.
            if (avail_space as usize) < runes.len() {
                runes.truncate(avail_space as usize);
            }
        }
        // Split the input into lines.
        let mut lines: Vec<Vec<char>> = Vec::new();
        let mut lstart = 0;
        for i in 0..runes.len() {
            if runes[i] == '\n' {
                lines.push(runes[lstart..i].to_vec());
                lstart = i + 1;
            }
        }
        if lstart <= runes.len() {
            // The last line did not end with a newline character.
            lines.push(runes[lstart..].to_vec());
        }
        // Obey the maximum line limit.
        if MAX_LINES > 0 && self.value.len() as i32 + lines.len() as i32 - 1 > MAX_LINES {
            let allowed = (MAX_LINES - self.value.len() as i32 + 1).max(0);
            lines.truncate(allowed as usize);
        }
        // Obey the maximum content height.
        if self.max_content_height > 0 {
            let budget = self.max_content_height - self.total_visual_lines();
            while lines.len() > 1 && self.visual_lines_for_insert(&lines) > budget {
                lines.pop();
            }
            if self.visual_lines_for_insert(&lines) > budget {
                return;
            }
        }
        if lines.is_empty() {
            // Nothing left to insert.
            return;
        }
        let row = self.row as usize;
        let col = self.col as usize;
        // Save the remainder of the original line at the current cursor
        // position.
        let tail: Vec<char> = self.value[row][col..].to_vec();
        // Paste the first line at the current cursor position.
        self.value[row].truncate(col);
        self.value[row].extend_from_slice(&lines[0]);
        self.col += lines[0].len() as i32;
        if lines.len() > 1 {
            // Add the new lines. We try to reuse the slice if there's
            // already space.
            for l in &lines[1..] {
                self.row += 1;
                self.value.insert(self.row as usize, l.clone());
                self.col = l.len() as i32;
            }
        }
        // Finally add the tail at the end of the last line inserted.
        self.value[self.row as usize].extend(tail);
        self.set_cursor_column(self.col);
    }

    /// value is the text.
    pub fn value(&self) -> String {
        let mut v = String::new();
        for l in &self.value {
            v.extend(l.iter());
            v.push('\n');
        }
        v.strip_suffix('\n').map(str::to_string).unwrap_or(v)
    }

    /// length is how many cells the text takes, a line break counting one.
    pub fn length(&self) -> i32 {
        let mut l = 0;
        for row in &self.value {
            l += width_of(row);
        }
        l + self.value.len() as i32 - 1
    }

    /// line_count is how many lines the text has.
    pub fn line_count(&self) -> i32 {
        self.value.len() as i32
    }

    /// line is the line the cursor is on, from 0.
    pub fn line(&self) -> i32 {
        self.row
    }

    /// column is the cursor's column in its line, in characters.
    pub fn column(&self) -> i32 {
        self.col
    }

    /// scroll_y_offset is how far down the area is scrolled.
    pub fn scroll_y_offset(&self) -> i32 {
        self.viewport.borrow().y_offset()
    }

    /// scroll_percent is how far down the area is scrolled, 0 to 1.
    pub fn scroll_percent(&self) -> f64 {
        self.viewport.borrow().scroll_percent()
    }

    fn set_cursor_line_relative(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let mut li = self.line_info();
        let char_offset = self.last_char_offset.max(li.char_offset);
        self.last_char_offset = char_offset;
        const TRAILING_SPACE: i32 = 2;
        if delta > 0 {
            for _ in 0..delta {
                if li.row_offset + 1 >= li.height && self.row < self.value.len() as i32 - 1 {
                    self.row += 1;
                    self.col = 0;
                } else {
                    // Move the cursor to the start of the next line so that
                    // we can get the line information. We need to add 2
                    // columns to account for the trailing space wrapping.
                    self.col = (li.start_column + li.width + TRAILING_SPACE)
                        .min(self.value[self.row as usize].len() as i32 - 1);
                }
                li = self.line_info();
            }
        } else {
            for _ in 0..-delta {
                if li.row_offset <= 0 && self.row > 0 {
                    self.row -= 1;
                    self.col = self.value[self.row as usize].len() as i32;
                } else {
                    // Move the cursor to the end of the previous line. This
                    // can be done by moving the cursor to the start of the
                    // line and then subtracting 2 to account for the
                    // trailing space we keep on soft-wrapped lines.
                    self.col = li.start_column - TRAILING_SPACE;
                }
                li = self.line_info();
            }
        }
        let nli = self.line_info();
        self.col = nli.start_column;
        if nli.width <= 0 {
            self.reposition_view();
            return;
        }
        let mut offset = 0;
        while offset < char_offset {
            let row = self.row as usize;
            if row >= self.value.len()
                || self.col < 0
                || self.col as usize >= self.value[row].len()
                || offset >= nli.char_width - 1
            {
                break;
            }
            offset += rune_width(self.value[row][self.col as usize]);
            self.col += 1;
        }
        self.reposition_view();
    }

    /// cursor_down moves the cursor a (wrapped) line down.
    pub fn cursor_down(&mut self) {
        self.set_cursor_line_relative(1);
    }

    /// cursor_up moves the cursor a (wrapped) line up.
    pub fn cursor_up(&mut self) {
        self.set_cursor_line_relative(-1);
    }

    /// set_cursor_column moves the cursor to a column of its line.
    pub fn set_cursor_column(&mut self, col: i32) {
        self.col = clamp(col, 0, self.value[self.row as usize].len() as i32);
        // Any time that we move the cursor horizontally we need to reset
        // the last offset so that the horizontal position when navigating
        // is adjusted.
        self.last_char_offset = 0;
    }

    /// cursor_start moves the cursor to the start of its line.
    pub fn cursor_start(&mut self) {
        self.set_cursor_column(0);
    }

    /// cursor_end moves the cursor to the end of its line.
    pub fn cursor_end(&mut self) {
        self.set_cursor_column(self.value[self.row as usize].len() as i32);
    }

    /// focused says whether the area takes keys.
    pub fn focused(&self) -> bool {
        self.focus
    }

    fn active_style(&self) -> &StyleState {
        if self.focus { &self.styles.focused } else { &self.styles.blurred }
    }

    /// focus makes the area take keys.
    pub fn focus(&mut self) -> Cmd {
        self.focus = true;
        self.virtual_cursor.focus()
    }

    /// blur stops the area taking keys.
    pub fn blur(&mut self) {
        self.focus = false;
        self.virtual_cursor.blur();
    }

    /// reset empties the area.
    pub fn reset(&mut self) {
        self.value = vec![Vec::new()];
        self.col = 0;
        self.row = 0;
        self.viewport.borrow_mut().goto_top();
        self.set_cursor_column(0);
        self.recalculate_height();
        self.clear_selection();
    }

    /// word is the word the cursor is in (or just after).
    pub fn word(&self) -> String {
        let line = &self.value[self.row as usize];
        let col = self.col - 1;
        if col < 0 || col as usize >= line.len() || is_space(line[col as usize]) {
            return String::new();
        }
        let mut start = col as usize;
        while start > 0 && !is_space(line[start - 1]) {
            start -= 1;
        }
        let mut end = col as usize;
        while end < line.len() && !is_space(line[end]) {
            end += 1;
        }
        line[start..end].iter().collect()
    }

    fn line_mut(&mut self) -> &mut Vec<char> {
        &mut self.value[self.row as usize]
    }

    fn line_len(&self) -> i32 {
        self.value[self.row as usize].len() as i32
    }

    fn delete_before_cursor(&mut self) {
        let col = self.col as usize;
        let l = self.line_mut();
        *l = l[col..].to_vec();
        self.set_cursor_column(0);
    }

    fn delete_after_cursor(&mut self) {
        let col = self.col as usize;
        self.line_mut().truncate(col);
        self.set_cursor_column(self.line_len());
    }

    fn transpose_left(&mut self) {
        if self.col == 0 || self.line_len() < 2 {
            return;
        }
        if self.col >= self.line_len() {
            self.set_cursor_column(self.col - 1);
        }
        let col = self.col as usize;
        self.line_mut().swap(col - 1, col);
        if self.col < self.line_len() {
            self.set_cursor_column(self.col + 1);
        }
    }

    fn delete_word_left(&mut self) {
        if self.col == 0 || self.line_len() == 0 {
            return;
        }
        // Linter note: it's critical that we acquire the initial cursor
        // position here prior to altering it via set_cursor_column() below.
        let old_col = self.col;
        self.set_cursor_column(self.col - 1);
        while is_space(self.value[self.row as usize][self.col as usize]) {
            if self.col <= 0 {
                break;
            }
            // ignore series of whitespace before cursor
            self.set_cursor_column(self.col - 1);
        }
        while self.col > 0 {
            if !is_space(self.value[self.row as usize][self.col as usize]) {
                self.set_cursor_column(self.col - 1);
            } else {
                if self.col > 0 {
                    // keep the previous space
                    self.set_cursor_column(self.col + 1);
                }
                break;
            }
        }
        let col = self.col as usize;
        let l = self.line_mut();
        if old_col as usize > l.len() {
            l.truncate(col);
        } else {
            let tail = l[old_col as usize..].to_vec();
            l.truncate(col);
            l.extend(tail);
        }
    }

    fn delete_word_right(&mut self) {
        if self.col >= self.line_len() || self.line_len() == 0 {
            return;
        }
        let old_col = self.col;
        while self.col < self.line_len() && is_space(self.value[self.row as usize][self.col as usize]) {
            // ignore series of whitespace after cursor
            self.set_cursor_column(self.col + 1);
        }
        while self.col < self.line_len() {
            if !is_space(self.value[self.row as usize][self.col as usize]) {
                self.set_cursor_column(self.col + 1);
            } else {
                break;
            }
        }
        let col = self.col as usize;
        let l = self.line_mut();
        if col > l.len() {
            l.truncate(old_col as usize);
        } else {
            let tail = l[col..].to_vec();
            l.truncate(old_col as usize);
            l.extend(tail);
        }
        self.set_cursor_column(old_col);
    }

    fn character_right(&mut self) {
        if self.col < self.line_len() {
            self.set_cursor_column(self.col + 1);
        } else if self.row < self.value.len() as i32 - 1 {
            self.row += 1;
            self.cursor_start();
        }
    }

    fn character_left(&mut self, inside_line: bool) {
        if self.col == 0 && self.row != 0 {
            self.row -= 1;
            self.cursor_end();
            if !inside_line {
                return;
            }
        }
        if self.col > 0 {
            self.set_cursor_column(self.col - 1);
        }
    }

    fn word_left(&mut self) {
        loop {
            let (old_row, old_col) = (self.row, self.col);
            self.character_left(true);
            if self.row == old_row && self.col == old_col {
                return;
            }
            if self.col < self.line_len() && !is_space(self.value[self.row as usize][self.col as usize]) {
                break;
            }
        }
        while self.col > 0 {
            if is_space(self.value[self.row as usize][self.col as usize - 1]) {
                break;
            }
            self.set_cursor_column(self.col - 1);
        }
    }

    fn word_right(&mut self) {
        self.do_word_right(|_, _, _| {});
    }

    fn do_word_right(&mut self, f: impl Fn(&mut Vec<char>, i32, usize)) {
        // Skip spaces forward.
        while self.col >= self.line_len() || is_space(self.value[self.row as usize][self.col as usize]) {
            if self.row == self.value.len() as i32 - 1 && self.col == self.line_len() {
                // End of text.
                break;
            }
            self.character_right();
        }
        let mut char_idx = 0;
        while self.col < self.line_len() {
            if is_space(self.value[self.row as usize][self.col as usize]) {
                break;
            }
            let col = self.col as usize;
            f(self.line_mut(), char_idx, col);
            self.set_cursor_column(self.col + 1);
            char_idx += 1;
        }
    }

    fn uppercase_right(&mut self) {
        self.do_word_right(|l, _, i| l[i] = l[i].to_uppercase().next().unwrap_or(l[i]));
    }

    fn lowercase_right(&mut self) {
        self.do_word_right(|l, _, i| l[i] = l[i].to_lowercase().next().unwrap_or(l[i]));
    }

    fn capitalize_right(&mut self) {
        self.do_word_right(|l, ci, i| {
            if ci == 0 {
                l[i] = l[i].to_uppercase().next().unwrap_or(l[i]);
            }
        });
    }

    /// line_info is where the cursor is in its line, soft wraps counted.
    pub fn line_info(&self) -> LineInfo {
        let grid = wrap(&self.value[self.row as usize], self.width);
        // Find out which line we are currently on. This can be determined
        // by looking at the number of characters in each line.
        let mut counter = 0;
        for (i, line) in grid.iter().enumerate() {
            let ll = line.len() as i32;
            // We've found the line that we are on
            if counter + ll == self.col && i + 1 < grid.len() {
                // We wrap around to the next line if we are at the end of
                // the previous line so that we can be at the very
                // beginning of the row
                return LineInfo {
                    char_offset: 0,
                    column_offset: 0,
                    height: grid.len() as i32,
                    row_offset: i as i32 + 1,
                    start_column: self.col,
                    width: grid[i + 1].len() as i32,
                    char_width: width_of(line),
                };
            }
            if counter + ll >= self.col {
                let upto = (self.col - counter).max(0) as usize;
                return LineInfo {
                    char_offset: width_of(&line[..upto.min(line.len())]),
                    column_offset: self.col - counter,
                    height: grid.len() as i32,
                    row_offset: i as i32,
                    start_column: counter,
                    width: ll,
                    char_width: width_of(line),
                };
            }
            counter += ll;
        }
        LineInfo::default()
    }

    /// reposition_view scrolls so the cursor's line shows.
    fn reposition_view(&mut self) {
        let row = self.cursor_line_number();
        let mut vp = self.viewport.borrow_mut();
        let minimum = vp.y_offset();
        let maximum = minimum + vp.height() - 1;
        if row < minimum {
            vp.scroll_up(minimum - row);
        } else if row > maximum {
            vp.scroll_down(row - maximum);
        }
    }

    /// width is how many cells wide the text is.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// move_to_begin moves the cursor to the start of the text.
    pub fn move_to_begin(&mut self) {
        self.row = 0;
        self.set_cursor_column(0);
        self.reposition_view();
    }

    /// move_to_end moves the cursor to the end of the text.
    pub fn move_to_end(&mut self) {
        self.row = self.value.len() as i32 - 1;
        self.set_cursor_column(self.line_len());
        self.reposition_view();
    }

    /// page_up moves the cursor a page up.
    pub fn page_up(&mut self) {
        let offset = self.viewport.borrow().y_offset() - self.cursor_line_number();
        if offset < 0 {
            self.set_cursor_line_relative(offset);
            return;
        }
        self.set_cursor_line_relative(-self.height);
    }

    /// page_down moves the cursor a page down.
    pub fn page_down(&mut self) {
        let offset = self.cursor_line_number() - self.viewport.borrow().y_offset();
        if offset < self.height - 1 {
            self.set_cursor_line_relative(self.height - 1 - offset);
            return;
        }
        self.set_cursor_line_relative(self.height);
    }

    /// set_width sets the area's width, prompt and line numbers included.
    pub fn set_width(&mut self, w: i32) {
        // Update prompt width only if there is no prompt function as
        // set_prompt_func updates the prompt width when it is called.
        if self.prompt_func.is_none() {
            self.prompt_width = uniseg_width(&self.prompt);
        }
        // Add base style borders and padding to reserved outer width.
        let reserved_outer = self.active_style().base.get_horizontal_frame_size();
        // Add prompt width to reserved inner width.
        let mut reserved_inner = self.prompt_width;
        // Add line number width to reserved inner width.
        if self.show_line_numbers {
            // A space between the prompt and line number, and another
            // between the line number and the text.
            const GAP: i32 = 2;
            reserved_inner += num_digits(self.max_height) + GAP;
        }
        // Input width must be at least one more than the reserved inner
        // and outer width. This gives us a minimum input width of 1.
        let min_width = reserved_inner + reserved_outer + 1;
        let mut input_width = w.max(min_width);
        // Input width must be no more than maximum width.
        if self.max_width > 0 {
            input_width = input_width.min(self.max_width);
        }
        // Since the width of the viewport and input area is dependent on
        // the width of borders, prompt and line numbers, we need to
        // calculate it by subtracting the reserved width from them.
        self.viewport.borrow_mut().set_width(input_width - reserved_outer);
        self.width = input_width - reserved_outer - reserved_inner;
        self.recalculate_height();
    }

    /// set_prompt_func sets a function drawing each line's prompt, `width`
    /// cells wide.
    pub fn set_prompt_func(&mut self, prompt_width: i32, f: fn(PromptInfo) -> String) {
        self.prompt_func = Some(f);
        self.prompt_width = prompt_width;
    }

    /// height is how many lines the area shows.
    pub fn height(&self) -> i32 {
        self.height
    }

    /// set_height sets how many lines the area shows.
    pub fn set_height(&mut self, h: i32) {
        if self.max_height > 0 {
            self.height = clamp(h, MIN_HEIGHT, self.max_height);
            self.viewport.borrow_mut().set_height(clamp(h, MIN_HEIGHT, self.max_height));
        } else {
            self.height = h.max(MIN_HEIGHT);
            self.viewport.borrow_mut().set_height(h.max(MIN_HEIGHT));
        }
        self.reposition_view();
    }

    /// update takes a message: keys edit the text while the area is
    /// focused, and pastes go in.
    pub fn update(&mut self, msg: &Msg) -> Cmd {
        if !self.focus {
            self.virtual_cursor.blur();
            return None;
        }
        // Used to determine if the cursor should blink.
        let (old_row, old_col) = (self.cursor_line_number(), self.col);
        let mut cmds: Vec<Cmd> = Vec::new();

        if let Some(p) = msg.downcast_ref::<PasteMsg>() {
            self.delete_selection();
            self.insert_runes_from_user_input(&p.content.chars().collect::<Vec<_>>());
        } else if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            let km = self.key_map.clone();
            let m = |b: &Binding| key::matches(k, &[b]);
            if m(&km.delete_after_cursor) {
                if self.has_selection() {
                    self.delete_selection();
                } else {
                    self.col = clamp(self.col, 0, self.line_len());
                    if self.col >= self.line_len() {
                        self.merge_line_below(self.row);
                    } else {
                        self.delete_after_cursor();
                    }
                }
            } else if m(&km.delete_before_cursor) {
                if self.has_selection() {
                    self.delete_selection();
                } else {
                    self.col = clamp(self.col, 0, self.line_len());
                    if self.col <= 0 {
                        self.merge_line_above(self.row);
                    } else {
                        self.delete_before_cursor();
                    }
                }
            } else if m(&km.delete_character_backward) {
                if self.has_selection() {
                    self.delete_selection();
                } else {
                    self.col = clamp(self.col, 0, self.line_len());
                    if self.col <= 0 {
                        self.merge_line_above(self.row);
                    } else if self.line_len() > 0 {
                        let col = self.col as usize;
                        self.line_mut().remove(col - 1);
                        if self.col > 0 {
                            self.set_cursor_column(self.col - 1);
                        }
                    }
                }
            } else if m(&km.delete_character_forward) {
                if self.has_selection() {
                    self.delete_selection();
                } else {
                    if self.line_len() > 0 && self.col < self.line_len() {
                        let col = self.col as usize;
                        self.line_mut().remove(col);
                    }
                    if self.col >= self.line_len() {
                        self.merge_line_below(self.row);
                    }
                }
            } else if m(&km.delete_word_backward) {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.col <= 0 {
                    self.merge_line_above(self.row);
                } else {
                    self.delete_word_left();
                }
            } else if m(&km.delete_word_forward) {
                if self.has_selection() {
                    self.delete_selection();
                } else {
                    self.col = clamp(self.col, 0, self.line_len());
                    if self.col >= self.line_len() {
                        self.merge_line_below(self.row);
                    } else {
                        self.delete_word_right();
                    }
                }
            } else if m(&km.insert_newline) {
                self.delete_selection();
                if self.at_content_limit() {
                    return None;
                }
                self.col = clamp(self.col, 0, self.line_len());
                self.split_line(self.row, self.col);
            } else if m(&km.line_end) {
                self.clear_selection();
                self.cursor_end();
            } else if m(&km.line_start) {
                self.clear_selection();
                self.cursor_start();
            } else if m(&km.character_forward) {
                self.clear_selection();
                self.character_right();
            } else if m(&km.line_next) {
                self.clear_selection();
                self.cursor_down();
            } else if m(&km.word_forward) {
                self.clear_selection();
                self.word_right();
            } else if m(&km.paste) {
                self.delete_selection();
                return paste();
            } else if m(&km.character_backward) {
                self.clear_selection();
                self.character_left(false);
            } else if m(&km.line_previous) {
                self.clear_selection();
                self.cursor_up();
            } else if m(&km.word_backward) {
                self.clear_selection();
                self.word_left();
            } else if m(&km.input_begin) {
                self.clear_selection();
                self.move_to_begin();
            } else if m(&km.input_end) {
                self.clear_selection();
                self.move_to_end();
            } else if m(&km.page_up) {
                self.clear_selection();
                self.page_up();
            } else if m(&km.page_down) {
                self.clear_selection();
                self.page_down();
            } else if m(&km.lowercase_word_forward) {
                self.lowercase_right();
            } else if m(&km.uppercase_word_forward) {
                self.uppercase_right();
            } else if m(&km.capitalize_word_forward) {
                self.capitalize_right();
            } else if m(&km.transpose_character_backward) {
                self.transpose_left();
            } else if m(&km.select_character_forward) {
                self.start_keyboard_selection();
                self.character_right();
                self.update_keyboard_selection();
            } else if m(&km.select_character_backward) {
                self.start_keyboard_selection();
                self.character_left(false);
                self.update_keyboard_selection();
            } else if m(&km.select_word_forward) {
                self.start_keyboard_selection();
                self.word_right();
                self.update_keyboard_selection();
            } else if m(&km.select_word_backward) {
                self.start_keyboard_selection();
                self.word_left();
                self.update_keyboard_selection();
            } else if m(&km.select_line_up) {
                self.start_keyboard_selection();
                self.cursor_up();
                self.update_keyboard_selection();
            } else if m(&km.select_line_down) {
                self.start_keyboard_selection();
                self.cursor_down();
                self.update_keyboard_selection();
            } else if m(&km.select_all) {
                self.select_all();
            } else if m(&km.copy_selection) {
                cmds.push(self.copy_selection());
            } else if !k.text.is_empty() {
                self.delete_selection();
                self.insert_runes_from_user_input(&k.text.chars().collect::<Vec<_>>());
            }
        } else if let Some(p) = msg.downcast_ref::<ClipboardMsg>() {
            self.delete_selection();
            self.insert_runes_from_user_input(&p.0.chars().collect::<Vec<_>>());
        } else if let Some(e) = msg.downcast_ref::<ClipboardErrMsg>() {
            self.err = Some(e.0.clone());
        } else if let Some(e) = msg.downcast_ref::<CopyErrMsg>() {
            self.err = Some(e.0.clone());
        }

        self.recalculate_height();

        let view = self.view_inner();
        {
            let mut vp = self.viewport.borrow_mut();
            vp.set_content(&view);
            cmds.push(vp.update(msg));
        }
        if self.use_virtual_cursor {
            let mut cmd = self.virtual_cursor.update(msg);
            let (new_row, new_col) = (self.cursor_line_number(), self.col);
            if (new_row != old_row || new_col != old_col) && self.virtual_cursor.mode() == Mode::Blink {
                self.virtual_cursor.is_blinked = false;
                cmd = self.virtual_cursor.blink();
            }
            cmds.push(cmd);
        }
        self.reposition_view();
        tea::batch(cmds)
    }

    /// view_inner is the text drawn whole, before the viewport cuts it.
    fn view_inner(&self) -> String {
        if self.value().is_empty() && self.row == 0 && self.col == 0 && !self.placeholder.is_empty() {
            return self.placeholder_view();
        }
        let styles = self.active_style();
        let mut vc = self.virtual_cursor.clone();
        vc.text_style = styles.computed_cursor_line();
        let mut s = String::new();
        let mut new_lines = 0;
        let widest_line_number = 0;
        let line_info = self.line_info();
        let mut display_line = 0;
        for (l, line) in self.value.iter().enumerate() {
            let l = l as i32;
            let wrapped_lines = wrap(line, self.width);
            let style = if self.row == l { styles.computed_cursor_line() } else { styles.computed_text() };
            let mut wrapped_base = 0;
            for (wl, wrapped_line) in wrapped_lines.iter().enumerate() {
                let prompt = self.prompt_view(display_line);
                s.push_str(&style.render(&prompt));
                display_line += 1;
                if self.show_line_numbers {
                    let is_cursor_line = self.row == l;
                    if wl == 0 {
                        // normal line
                        s.push_str(&self.line_number_view(l + 1, is_cursor_line));
                    } else {
                        // soft wrapped line
                        s.push_str(&self.line_number_view(-1, is_cursor_line));
                    }
                }
                let mut wrapped_line: Vec<char> = wrapped_line.clone();
                let strwidth = width_of(&wrapped_line);
                let mut padding = self.width - strwidth;
                // If the trailing space causes the line to be wider than
                // the width, we should not draw it to the screen since it
                // will result in an extra space at the end of the line
                // which can look off when the cursor line is showing.
                if strwidth > self.width {
                    // The character causing the line to be wider than the
                    // width is guaranteed to be a space since any other
                    // character would have been wrapped.
                    if wrapped_line.last() == Some(&' ') {
                        wrapped_line.pop();
                    }
                    padding -= self.width - strwidth;
                }
                let render = |r: &[char]| style.render(&r.iter().collect::<String>());
                if let Some((from, to)) = self.selection_span_for(l, wrapped_base, wrapped_line.len() as i32) {
                    let (from, to) = (from as usize, to as usize);
                    s.push_str(&render(&wrapped_line[..from]));
                    s.push_str(&styles.computed_selection().render(&wrapped_line[from..to].iter().collect::<String>()));
                    s.push_str(&render(&wrapped_line[to..]));
                } else if self.row == l && line_info.row_offset == wl as i32 {
                    let co = (line_info.column_offset.max(0) as usize).min(wrapped_line.len());
                    s.push_str(&render(&wrapped_line[..co]));
                    if self.col >= line.len() as i32 && line_info.char_offset >= self.width {
                        vc.set_char(" ");
                        s.push_str(&vc.view());
                    } else {
                        let ch = wrapped_line.get(co).map(|c| c.to_string()).unwrap_or_default();
                        vc.set_char(&ch);
                        s.push_str(&style.render(&vc.view()));
                        if co < wrapped_line.len() {
                            s.push_str(&render(&wrapped_line[co + 1..]));
                        }
                    }
                } else {
                    s.push_str(&render(&wrapped_line));
                }
                wrapped_base += wrapped_line.len() as i32;
                s.push_str(&style.render(&" ".repeat(padding.max(0) as usize)));
                s.push('\n');
                new_lines += 1;
            }
        }
        let _ = new_lines;
        // Always show at least `m.Height` lines at all times. To do this we
        // can simply pad out a few extra new lines in the view.
        for _ in 0..self.height {
            s.push_str(&self.prompt_view(display_line));
            display_line += 1;
            // Write end of buffer content
            let left_gutter = self.end_of_buffer_character.to_string();
            let right_gap_width = self.width() - uniseg_width(&left_gutter) + widest_line_number;
            let right_gap = " ".repeat(right_gap_width.max(0) as usize);
            s.push_str(&styles.computed_end_of_buffer().render(&(left_gutter + &right_gap)));
            s.push('\n');
        }
        s
    }

    /// view draws the area.
    pub fn view(&self) -> String {
        let inner = self.view_inner();
        let mut vp = self.viewport.borrow_mut();
        vp.set_content(&inner);
        let view = vp.view();
        self.active_style().base.render(&view)
    }

    fn prompt_view(&self, display_line: i32) -> String {
        let mut prompt = self.prompt.clone();
        if let Some(f) = self.prompt_func {
            prompt = f(PromptInfo { line_number: display_line, focused: self.focus });
            let width = style::width(&prompt);
            if width < self.prompt_width {
                prompt = " ".repeat((self.prompt_width - width) as usize) + &prompt;
            }
        }
        self.active_style().computed_prompt().render(&prompt)
    }

    fn line_number_view(&self, n: i32, is_cursor_line: bool) -> String {
        if !self.show_line_numbers {
            return String::new();
        }
        let s = if n <= 0 { " ".to_string() } else { n.to_string() };
        let styles = self.active_style();
        let (text_style, line_number_style) = if is_cursor_line {
            (styles.computed_cursor_line(), styles.computed_cursor_line_number())
        } else {
            (styles.computed_text(), styles.computed_line_number())
        };
        // Format line number dynamically based on the maximum number of
        // lines.
        let digits = self.max_height.to_string().len();
        let s = format!(" {s:>digits$} ");
        text_style.render(&line_number_style.render(&s))
    }

    fn placeholder_view(&self) -> String {
        let styles = self.active_style();
        let p = &self.placeholder;
        let pwordwrap = ansi::wordwrap(p, self.width, "");
        let pwrap = ansi::hardwrap(&pwordwrap, self.width, true);
        let plines: Vec<String> = pwrap.trim().split('\n').map(str::to_string).collect();
        let mut vc = self.virtual_cursor.clone();
        let mut s = String::new();
        for i in 0..self.height {
            let iu = i as usize;
            let is_line_number = plines.len() > iu;
            let line_style =
                if plines.len() > iu { styles.computed_cursor_line() } else { styles.computed_placeholder() };
            let prompt = self.prompt_view(i);
            s.push_str(&line_style.render(&prompt));
            if self.show_line_numbers {
                let ln = if i == 0 { i + 1 } else { 0 };
                if i == 0 || plines.len() > iu {
                    s.push_str(&self.line_number_view(ln, is_line_number));
                }
            }
            if i == 0 {
                // first line: the cursor on the placeholder's first
                // grapheme
                vc.text_style = styles.computed_placeholder();
                let first = &plines[0];
                let ch = first.graphemes(true).next().unwrap_or("");
                let rest = &first[ch.len()..];
                vc.set_char(ch);
                s.push_str(&line_style.render(&vc.view()));
                // the rest of the first line
                s.push_str(&line_style.render(&styles.computed_placeholder().render(rest)));
                // extend the first line with spaces to fill the width
                let gap = " ".repeat((self.width - style::width(first)).max(0) as usize);
                s.push_str(&line_style.render(&gap));
            } else if plines.len() > iu {
                // the rest of the placeholder's lines
                let placeholder_line = &plines[iu];
                let gap = " ".repeat((self.width - uniseg_width(placeholder_line)).max(0) as usize);
                s.push_str(&line_style.render(&format!("{placeholder_line}{gap}")));
            } else {
                // end of the buffer
                let eob = styles.computed_end_of_buffer().render(&self.end_of_buffer_character.to_string());
                s.push_str(&eob);
            }
            s.push('\n');
        }
        let mut vp = self.viewport.borrow_mut();
        vp.set_content(&s);
        styles.base.render(&vp.view())
    }

    fn cursor_line_number(&self) -> i32 {
        let mut line = 0;
        for i in 0..self.row as usize {
            // Calculate the number of lines that the current line will be
            // split into.
            line += wrap(&self.value[i], self.width).len() as i32;
        }
        line + self.line_info().row_offset
    }

    fn total_visual_lines(&self) -> i32 {
        self.value.iter().map(|l| wrap(l, self.width).len() as i32).sum()
    }

    fn recalculate_height(&mut self) {
        if !self.dynamic_height {
            return;
        }
        let min_h = self.min_height.max(MIN_HEIGHT);
        let total = self.total_visual_lines();
        let mut h = total.max(min_h);
        if self.max_height > 0 {
            h = h.min(self.max_height);
        }
        // Reset scroll offset when content fits. The viewport is updated
        // lazily during view(), so it may still hold an old offset.
        let max_offset = total - h;
        {
            let mut vp = self.viewport.borrow_mut();
            if vp.y_offset() > max_offset {
                vp.set_y_offset(max_offset.max(0));
            }
        }
        self.set_height(h);
    }

    fn at_content_limit(&self) -> bool {
        if self.max_content_height > 0 {
            return self.total_visual_lines() >= self.max_content_height;
        }
        self.max_height > 0 && self.value.len() as i32 >= self.max_height
    }

    fn visual_lines_for_insert(&self, lines: &[Vec<char>]) -> i32 {
        if lines.is_empty() {
            return 0;
        }
        let row = &self.value[self.row as usize];
        let col = self.col as usize;
        let current = wrap(row, self.width).len() as i32;
        let mut merged: Vec<char> = row[..col].to_vec();
        merged.extend_from_slice(&lines[0]);
        if lines.len() == 1 {
            merged.extend_from_slice(&row[col..]);
        }
        let mut delta = wrap(&merged, self.width).len() as i32 - current;
        for (i, content) in lines.iter().enumerate().skip(1) {
            let mut content = content.clone();
            if i == lines.len() - 1 {
                content.extend_from_slice(&row[col..]);
            }
            delta += wrap(&content, self.width).len() as i32;
        }
        delta
    }

    fn merge_line_below(&mut self, row: i32) {
        let row = row as usize;
        if row as i32 >= self.value.len() as i32 - 1 {
            return;
        }
        let below = self.value.remove(row + 1);
        self.value[row].extend(below);
    }

    fn merge_line_above(&mut self, row: i32) {
        if row <= 0 {
            return;
        }
        let row = row as usize;
        self.col = self.value[row - 1].len() as i32;
        self.row -= 1;
        let this = self.value.remove(row);
        self.value[row - 1].extend(this);
    }

    fn split_line(&mut self, row: i32, col: i32) {
        let (row, col) = (row as usize, col as usize);
        let tail = self.value[row][col..].to_vec();
        self.value[row].truncate(col);
        self.value.insert(row + 1, tail);
        self.col = 0;
        self.row += 1;
    }

    // Selection

    fn gutter_width(&self) -> i32 {
        let mut w = self.prompt_width;
        if self.show_line_numbers {
            w += self.max_height.to_string().len() as i32 + 2;
        }
        w
    }

    /// position_at is the place in the text at x, y on the area.
    pub fn position_at(&self, x: i32, y: i32) -> Position {
        if self.value.is_empty() {
            return Position::default();
        }
        let target = y + self.viewport.borrow().y_offset();
        if target < 0 {
            return Position::default();
        }
        let content_x = (x - self.gutter_width()).max(0);
        let mut display = 0;
        for (row, line) in self.value.iter().enumerate() {
            let wrapped = wrap(line, self.width);
            let mut base = 0;
            for w in &wrapped {
                if display == target {
                    let col = base + rune_index_for_column(w, content_x);
                    return Position { row: row as i32, col: clamp(col, 0, line.len() as i32) };
                }
                base += w.len() as i32;
                display += 1;
            }
        }
        let last = self.value.len() - 1;
        Position { row: last as i32, col: self.value[last].len() as i32 }
    }

    /// begin_selection starts a selection at x, y on the area.
    pub fn begin_selection(&mut self, x: i32, y: i32) {
        let pos = self.position_at(x, y);
        self.select_from(pos, pos);
        self.selecting = true;
        self.move_cursor_to(pos);
    }

    /// extend_selection takes the selection to x, y on the area.
    pub fn extend_selection(&mut self, x: i32, y: i32) {
        if !self.selecting {
            return;
        }
        let pos = self.position_at(x, y);
        self.sel_head = pos;
        self.has_selection = true;
        self.move_cursor_to(pos);
    }

    /// end_selection ends a selection made with the mouse.
    pub fn end_selection(&mut self) {
        self.selecting = false;
        if self.sel_anchor == self.sel_head {
            self.clear_selection();
        }
    }

    /// select_all selects all the text.
    pub fn select_all(&mut self) {
        if self.value.is_empty() {
            return;
        }
        let last = self.value.len() - 1;
        self.select_from(Position::default(), Position { row: last as i32, col: self.value[last].len() as i32 });
        self.selecting = false;
    }

    /// clear_selection drops the selection.
    pub fn clear_selection(&mut self) {
        self.has_selection = false;
        self.selecting = false;
        self.sel_anchor = Position::default();
        self.sel_head = Position::default();
    }

    /// has_selection says whether some text is selected.
    pub fn has_selection(&self) -> bool {
        self.has_selection && self.sel_anchor != self.sel_head
    }

    /// selection is where the selection starts and ends, in order.
    pub fn selection(&self) -> Option<(Position, Position)> {
        if !self.has_selection() {
            return None;
        }
        let (mut start, mut end) = (self.sel_anchor, self.sel_head);
        if end.before(&start) {
            std::mem::swap(&mut start, &mut end);
        }
        Some((start, end))
    }

    /// selected_text is the text selected.
    pub fn selected_text(&self) -> String {
        let Some((start, end)) = self.selection() else { return String::new() };
        let sub = |line: &[char], a: i32, b: i32| -> String {
            let n = line.len() as i32;
            line[clamp(a, 0, n) as usize..clamp(b, 0, n) as usize].iter().collect()
        };
        if start.row == end.row {
            let line = &self.value[start.row as usize];
            return sub(line, start.col, end.col);
        }
        let mut b = String::new();
        for row in start.row..=end.row {
            let line = &self.value[row as usize];
            let n = line.len() as i32;
            if row == start.row {
                b.push_str(&sub(line, start.col, n));
            } else if row == end.row {
                b.push_str(&sub(line, 0, end.col));
            } else {
                b.extend(line.iter());
            }
            if row < end.row {
                b.push('\n');
            }
        }
        b
    }

    /// delete_selection deletes the text selected.
    pub fn delete_selection(&mut self) {
        let Some((start, end)) = self.selection() else { return };
        if start.row == end.row {
            let line = &mut self.value[start.row as usize];
            let end_col = (end.col as usize).min(line.len());
            let start_col = (start.col as usize).min(end_col);
            line.drain(start_col..end_col);
            self.row = start.row;
            self.set_cursor_column(start_col as i32);
        } else {
            let head = self.value[start.row as usize].clone();
            let tail = self.value[end.row as usize].clone();
            let end_col = (end.col as usize).min(tail.len());
            let start_col = (start.col as usize).min(head.len());
            let mut merged = head[..start_col].to_vec();
            merged.extend_from_slice(&tail[end_col..]);
            self.value.splice(start.row as usize..=end.row as usize, [merged]);
            self.row = start.row;
            self.set_cursor_column(start_col as i32);
        }
        self.clear_selection();
    }

    /// copy_selection is the command that puts the selection on the
    /// clipboard.
    pub fn copy_selection(&mut self) -> Cmd {
        if !self.has_selection() {
            return None;
        }
        let text = self.selected_text();
        tea::cmd(move || {
            Some(match clipboard_write(&text) {
                Ok(()) => tea::msg(CopyMsg(text)),
                Err(e) => tea::msg(CopyErrMsg(e)),
            })
        })
    }

    fn start_keyboard_selection(&mut self) {
        if !self.has_selection {
            self.sel_anchor = Position { row: self.row, col: self.col };
        }
        self.has_selection = true;
        self.sel_head = Position { row: self.row, col: self.col };
    }

    fn update_keyboard_selection(&mut self) {
        self.sel_head = Position { row: self.row, col: self.col };
    }

    fn select_from(&mut self, anchor: Position, head: Position) {
        self.sel_anchor = anchor;
        self.sel_head = head;
        self.has_selection = true;
    }

    fn move_cursor_to(&mut self, pos: Position) {
        if self.value.is_empty() {
            return;
        }
        let row = clamp(pos.row, 0, self.value.len() as i32 - 1);
        self.row = row;
        self.col = clamp(pos.col, 0, self.value[row as usize].len() as i32);
    }

    fn selection_span_for(&self, row: i32, base: i32, length: i32) -> Option<(i32, i32)> {
        let (start, end) = self.selection()?;
        if row < start.row || row > end.row {
            return None;
        }
        let line_len = self.value[row as usize].len() as i32;
        let (mut row_from, mut row_to) = (0, line_len);
        if row == start.row {
            row_from = clamp(start.col, 0, line_len);
        }
        if row == end.row {
            row_to = clamp(end.col, 0, line_len);
        }
        let from = row_from.max(base);
        let to = row_to.min(base + length);
        if from >= to {
            return None;
        }
        Some((from - base, to - base))
    }
}

fn rune_index_for_column(runes: &[char], col: i32) -> i32 {
    if col <= 0 {
        return 0;
    }
    let mut w = 0;
    for (i, r) in runes.iter().enumerate() {
        let rw = ansi::string_width(&r.to_string());
        if w + rw > col {
            return i as i32;
        }
        w += rw;
    }
    runes.len() as i32
}

/// clipboard_write puts text on the system clipboard.
fn clipboard_write(text: &str) -> Result<(), String> {
    use std::io::Write;
    let (cmd, args): (&str, &[&str]) =
        if cfg!(target_os = "macos") { ("pbcopy", &[]) } else { ("xclip", &["-in", "-selection", "clipboard"]) };
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}

/// blink is the message that starts the cursor blinking: none here.
pub fn blink() -> Cmd {
    cursor::blink()
}

/// paste is the command that reads the clipboard into the area.
pub fn paste() -> Cmd {
    tea::cmd(|| {
        Some(match super::clipboard_read() {
            Ok(s) => tea::msg(ClipboardMsg(s)),
            Err(e) => tea::msg(ClipboardErrMsg(e)),
        })
    })
}

/// wrap soft-wraps a line at `width` cells, at spaces where it can, each
/// wrapped line keeping its trailing space(s), the last one given an extra
/// space for the cursor.
fn wrap(runes: &[char], width: i32) -> Vec<Vec<char>> {
    let mut lines: Vec<Vec<char>> = vec![Vec::new()];
    let mut word: Vec<char> = Vec::new();
    let mut row = 0;
    let mut spaces = 0;
    for &r in runes {
        if is_space(r) {
            spaces += 1;
        } else {
            word.push(r);
        }
        if spaces > 0 {
            if width_of(&lines[row]) + width_of(&word) + spaces > width {
                row += 1;
                lines.push(Vec::new());
                lines[row].extend_from_slice(&word);
                lines[row].extend(std::iter::repeat_n(' ', spaces as usize));
            } else {
                lines[row].extend_from_slice(&word);
                lines[row].extend(std::iter::repeat_n(' ', spaces as usize));
            }
            spaces = 0;
            word.clear();
        } else {
            // If the last character is a double-width rune, then we may not
            // be able to add it to this line as it might cause us to go
            // past the width.
            let last_char_len = rune_width(*word.last().unwrap_or(&' '));
            if width_of(&word) + last_char_len > width {
                // If the current line has any content, let's move to the
                // next line because the current word fills up the entire
                // line.
                if !lines[row].is_empty() {
                    row += 1;
                    lines.push(Vec::new());
                }
                lines[row].extend_from_slice(&word);
                word.clear();
            }
        }
    }
    if width_of(&lines[row]) + width_of(&word) + spaces >= width {
        lines.push(Vec::new());
        lines[row + 1].extend_from_slice(&word);
        // We add an extra space at the end of the line to account for the
        // trailing space at the end of the previous soft-wrapped lines so
        // that behaviour when navigating is consistent and so that we don't
        // need to continually add edges to handle the last line of the
        // wrapped input.
        spaces += 1;
        lines[row + 1].extend(std::iter::repeat_n(' ', spaces as usize));
    } else {
        lines[row].extend_from_slice(&word);
        spaces += 1;
        lines[row].extend(std::iter::repeat_n(' ', spaces as usize));
    }
    lines
}

#[cfg(test)]
mod tests;
