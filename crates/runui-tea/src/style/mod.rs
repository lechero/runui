//! Lip Gloss v2: styles for text in the terminal (colors, attributes,
//! padding, margins, borders, alignment, size), and putting blocks of text
//! together (joining, placing, layers on a canvas). Rendering gives exactly
//! what Lip Gloss gives for the same style and text, SGR sequences included:
//! colors are written as given (24-bit for "#RRGGBB"), and the program
//! brings them down to what the terminal can show when it draws a frame, as
//! Bubble Tea v2 does.
//!
//! A [`Style`] is a small `Copy` value built by chaining, like Lip Gloss's:
//! every method returns a new style, so styles can be constants:
//!
//! ```
//! use runui_tea::style::{self, Style};
//! const ACCENT: style::Color = style::color("#7DD3FC");
//! const TITLE: Style = Style::new().bold(true).foreground(ACCENT);
//! assert_eq!(TITLE.render("hi"), "\x1b[1;38;2;125;211;252mhi\x1b[m");
//! assert_eq!(TITLE.padding(&[0, 1]).width(6).get_horizontal_frame_size(), 2);
//! ```
//!
//! Lip Gloss's variadic setters take a slice here: `padding(&[0, 1])` is
//! `Padding(0, 1)`, `border(b, &[false, false, false, true])` is
//! `Border(b, false, false, false, true)`.

mod border;
mod canvas;
mod layout;
mod runes;
mod wrap;

pub use crate::ansi::{Color, color};
pub use border::{
    Border, ascii_border, block_border, double_border, hidden_border, inner_half_block_border, markdown_border,
    normal_border, outer_half_block_border, rounded_border, thick_border,
};
pub use canvas::{Canvas, Compositor, Drawable, Layer};
pub use layout::{
    BOTTOM, CENTER, LEFT, Position, RIGHT, TOP, height, join_horizontal, join_vertical, place, place_horizontal,
    place_vertical, size, width,
};
pub use runes::style_runes;
pub use wrap::wrap;

use crate::ansi::{self, sgr::Sgr};
use layout::{align_text_horizontal, align_text_vertical, get_lines};

/// NBSP is the non-breaking space.
pub const NBSP: char = '\u{A0}';

/// NO_TAB_CONVERSION, as a tab width, leaves tabs as they are.
pub const NO_TAB_CONVERSION: i32 = -1;

const TAB_WIDTH_DEFAULT: i32 = 4;

/// Underline is the style of an underline.
pub type Underline = u8;
/// No underline.
pub const UNDERLINE_NONE: Underline = 0;
/// A single underline: what `underline(true)` sets.
pub const UNDERLINE_SINGLE: Underline = 1;
/// A double underline.
pub const UNDERLINE_DOUBLE: Underline = 2;
/// A curly underline.
pub const UNDERLINE_CURLY: Underline = 3;
/// A dotted underline.
pub const UNDERLINE_DOTTED: Underline = 4;
/// A dashed underline.
pub const UNDERLINE_DASHED: Underline = 5;

// The properties a style can have set, one bit each, in Lip Gloss's order.
const BOLD: u64 = 1 << 0;
const ITALIC: u64 = 1 << 1;
const STRIKETHROUGH: u64 = 1 << 2;
const REVERSE: u64 = 1 << 3;
const BLINK: u64 = 1 << 4;
const FAINT: u64 = 1 << 5;
const UNDERLINE_SPACES: u64 = 1 << 6;
const STRIKETHROUGH_SPACES: u64 = 1 << 7;
const COLOR_WHITESPACE: u64 = 1 << 8;
const UNDERLINE: u64 = 1 << 9;
const FOREGROUND: u64 = 1 << 10;
const BACKGROUND: u64 = 1 << 11;
const UNDERLINE_COLOR: u64 = 1 << 12;
const WIDTH: u64 = 1 << 13;
const HEIGHT: u64 = 1 << 14;
const ALIGN_HORIZONTAL: u64 = 1 << 15;
const ALIGN_VERTICAL: u64 = 1 << 16;
const PADDING_TOP: u64 = 1 << 17;
const PADDING_RIGHT: u64 = 1 << 18;
const PADDING_BOTTOM: u64 = 1 << 19;
const PADDING_LEFT: u64 = 1 << 20;
const PADDING_CHAR: u64 = 1 << 21;
const MARGIN_TOP: u64 = 1 << 22;
const MARGIN_RIGHT: u64 = 1 << 23;
const MARGIN_BOTTOM: u64 = 1 << 24;
const MARGIN_LEFT: u64 = 1 << 25;
const MARGIN_BACKGROUND: u64 = 1 << 26;
const MARGIN_CHAR: u64 = 1 << 27;
const BORDER_STYLE: u64 = 1 << 28;
const BORDER_TOP: u64 = 1 << 29;
const BORDER_RIGHT: u64 = 1 << 30;
const BORDER_BOTTOM: u64 = 1 << 31;
const BORDER_LEFT: u64 = 1 << 32;
const BORDER_TOP_FOREGROUND: u64 = 1 << 33;
const BORDER_RIGHT_FOREGROUND: u64 = 1 << 34;
const BORDER_BOTTOM_FOREGROUND: u64 = 1 << 35;
const BORDER_LEFT_FOREGROUND: u64 = 1 << 36;
const BORDER_FOREGROUND_BLEND: u64 = 1 << 37;
const BORDER_FOREGROUND_BLEND_OFFSET: u64 = 1 << 38;
const BORDER_TOP_BACKGROUND: u64 = 1 << 39;
const BORDER_RIGHT_BACKGROUND: u64 = 1 << 40;
const BORDER_BOTTOM_BACKGROUND: u64 = 1 << 41;
const BORDER_LEFT_BACKGROUND: u64 = 1 << 42;
const INLINE: u64 = 1 << 43;
const MAX_WIDTH: u64 = 1 << 44;
const MAX_HEIGHT: u64 = 1 << 45;
const TAB_WIDTH: u64 = 1 << 46;
const TRANSFORM: u64 = 1 << 47;

/// Style is a set of rules for rendering text: Lip Gloss's `Style`.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    props: u64,
    value: &'static str,
    attrs: u64,
    fg: Color,
    bg: Color,
    ul_color: Color,
    ul: Underline,
    width: i32,
    height: i32,
    align_horizontal: Position,
    align_vertical: Position,
    padding: [i32; 4],
    padding_char: char,
    margin: [i32; 4],
    margin_bg: Color,
    margin_char: char,
    border_style: Border,
    border_fg: [Color; 4],
    border_bg: [Color; 4],
    max_width: i32,
    max_height: i32,
    tab_width: i32,
    transform: Option<fn(&str) -> String>,
}

impl Default for Style {
    fn default() -> Self {
        Style::new()
    }
}

/// sides spreads one to four values over top, right, bottom and left, the
/// way CSS (and Lip Gloss) does; anything else is none.
const fn which_sides_int(i: &[i32]) -> Option<[i32; 4]> {
    match i.len() {
        1 => Some([i[0], i[0], i[0], i[0]]),
        2 => Some([i[0], i[1], i[0], i[1]]),
        3 => Some([i[0], i[1], i[2], i[1]]),
        4 => Some([i[0], i[1], i[2], i[3]]),
        _ => None,
    }
}

const fn which_sides_bool(i: &[bool]) -> Option<[bool; 4]> {
    match i.len() {
        1 => Some([i[0], i[0], i[0], i[0]]),
        2 => Some([i[0], i[1], i[0], i[1]]),
        3 => Some([i[0], i[1], i[2], i[1]]),
        4 => Some([i[0], i[1], i[2], i[3]]),
        _ => None,
    }
}

const fn which_sides_color(i: &[Color]) -> Option<[Color; 4]> {
    match i.len() {
        1 => Some([i[0], i[0], i[0], i[0]]),
        2 => Some([i[0], i[1], i[0], i[1]]),
        3 => Some([i[0], i[1], i[2], i[1]]),
        4 => Some([i[0], i[1], i[2], i[3]]),
        _ => None,
    }
}

const fn max0(v: i32) -> i32 {
    if v < 0 { 0 } else { v }
}

impl Style {
    /// new is an empty style: rendering with it leaves text as it is (but
    /// for tabs, which become four spaces).
    pub const fn new() -> Style {
        Style {
            props: 0,
            value: "",
            attrs: 0,
            fg: Color::NoColor,
            bg: Color::NoColor,
            ul_color: Color::NoColor,
            ul: UNDERLINE_NONE,
            width: 0,
            height: 0,
            align_horizontal: LEFT,
            align_vertical: TOP,
            padding: [0; 4],
            padding_char: '\0',
            margin: [0; 4],
            margin_bg: Color::NoColor,
            margin_char: '\0',
            border_style: border::NO_BORDER,
            border_fg: [Color::NoColor; 4],
            border_bg: [Color::NoColor; 4],
            max_width: 0,
            max_height: 0,
            tab_width: 0,
            transform: None,
        }
    }

    const fn is_set(&self, k: u64) -> bool {
        self.props & k != 0
    }

    const fn set_bool(mut self, k: u64, v: bool) -> Style {
        if v {
            self.attrs |= k;
        } else {
            self.attrs &= !k;
        }
        self.props |= k;
        self
    }

    const fn get_bool(&self, k: u64, def: bool) -> bool {
        if !self.is_set(k) {
            return def;
        }
        self.attrs & k != 0
    }

    const fn get_int(&self, k: u64, v: i32) -> i32 {
        if self.is_set(k) { v } else { 0 }
    }

    const fn get_color(&self, k: u64, c: Color) -> Color {
        if self.is_set(k) { c } else { Color::NoColor }
    }

    // Setting

    /// set_string sets the text the style renders by itself, through
    /// [`Style::string`], and puts before what's given to `render`.
    pub const fn set_string(mut self, s: &'static str) -> Style {
        self.value = s;
        self
    }

    /// value is the text set with `set_string`.
    pub const fn value(&self) -> &'static str {
        self.value
    }

    /// string renders the text set with `set_string`.
    pub fn string(&self) -> String {
        self.render_strs(&[])
    }

    /// bold sets bold text.
    pub const fn bold(self, v: bool) -> Style {
        self.set_bool(BOLD, v)
    }

    /// italic sets italic text.
    pub const fn italic(self, v: bool) -> Style {
        self.set_bool(ITALIC, v)
    }

    /// underline sets a single underline, or none.
    pub const fn underline(self, v: bool) -> Style {
        self.underline_style(if v { UNDERLINE_SINGLE } else { UNDERLINE_NONE })
    }

    /// underline_style sets the kind of underline.
    pub const fn underline_style(mut self, u: Underline) -> Style {
        self.ul = u;
        self.props |= UNDERLINE;
        self
    }

    /// underline_color sets the underline's color.
    pub const fn underline_color(mut self, c: Color) -> Style {
        self.ul_color = c;
        self.props |= UNDERLINE_COLOR;
        self
    }

    /// strikethrough sets text struck through.
    pub const fn strikethrough(self, v: bool) -> Style {
        self.set_bool(STRIKETHROUGH, v)
    }

    /// reverse swaps the foreground and background colors.
    pub const fn reverse(self, v: bool) -> Style {
        self.set_bool(REVERSE, v)
    }

    /// blink sets blinking text.
    pub const fn blink(self, v: bool) -> Style {
        self.set_bool(BLINK, v)
    }

    /// faint sets dimmed text.
    pub const fn faint(self, v: bool) -> Style {
        self.set_bool(FAINT, v)
    }

    /// foreground sets the text's color.
    pub const fn foreground(mut self, c: Color) -> Style {
        self.fg = c;
        self.props |= FOREGROUND;
        self
    }

    /// background sets the color behind the text.
    pub const fn background(mut self, c: Color) -> Style {
        self.bg = c;
        self.props |= BACKGROUND;
        self
    }

    /// width sets the block's width, borders included: text is wrapped to
    /// fit and short lines padded.
    pub const fn width(mut self, w: i32) -> Style {
        self.width = max0(w);
        self.props |= WIDTH;
        self
    }

    /// height sets the block's height, borders included: short blocks get
    /// empty lines (where `align_vertical` says).
    pub const fn height(mut self, h: i32) -> Style {
        self.height = max0(h);
        self.props |= HEIGHT;
        self
    }

    /// align sets the horizontal alignment, as Lip Gloss's `Align` with one
    /// position.
    pub const fn align(self, p: Position) -> Style {
        self.align_horizontal(p)
    }

    /// align_horizontal sets where lines go across the block's width.
    pub const fn align_horizontal(mut self, p: Position) -> Style {
        self.align_horizontal = p;
        self.props |= ALIGN_HORIZONTAL;
        self
    }

    /// align_vertical sets where the text goes in the block's height.
    pub const fn align_vertical(mut self, p: Position) -> Style {
        self.align_vertical = p;
        self.props |= ALIGN_VERTICAL;
        self
    }

    /// padding sets the room inside the border, one to four values as CSS:
    /// `&[1]` everywhere, `&[1, 2]` top and bottom, then left and right…
    pub const fn padding(mut self, sides: &[i32]) -> Style {
        if let Some([t, r, b, l]) = which_sides_int(sides) {
            self = self.padding_top(t).padding_right(r).padding_bottom(b).padding_left(l);
        }
        self
    }

    /// padding_top sets the padding above.
    pub const fn padding_top(mut self, n: i32) -> Style {
        self.padding[0] = max0(n);
        self.props |= PADDING_TOP;
        self
    }

    /// padding_right sets the padding on the right.
    pub const fn padding_right(mut self, n: i32) -> Style {
        self.padding[1] = max0(n);
        self.props |= PADDING_RIGHT;
        self
    }

    /// padding_bottom sets the padding below.
    pub const fn padding_bottom(mut self, n: i32) -> Style {
        self.padding[2] = max0(n);
        self.props |= PADDING_BOTTOM;
        self
    }

    /// padding_left sets the padding on the left.
    pub const fn padding_left(mut self, n: i32) -> Style {
        self.padding[3] = max0(n);
        self.props |= PADDING_LEFT;
        self
    }

    /// padding_char sets what padding is drawn with (a space by default).
    pub const fn padding_char(mut self, c: char) -> Style {
        self.padding_char = c;
        self.props |= PADDING_CHAR;
        self
    }

    /// color_whitespace says whether the background color fills padding
    /// and the room alignment adds (it does by default).
    pub const fn color_whitespace(self, v: bool) -> Style {
        self.set_bool(COLOR_WHITESPACE, v)
    }

    /// margin sets the room outside the border, one to four values as CSS.
    pub const fn margin(mut self, sides: &[i32]) -> Style {
        if let Some([t, r, b, l]) = which_sides_int(sides) {
            self = self.margin_top(t).margin_right(r).margin_bottom(b).margin_left(l);
        }
        self
    }

    /// margin_top sets the margin above.
    pub const fn margin_top(mut self, n: i32) -> Style {
        self.margin[0] = max0(n);
        self.props |= MARGIN_TOP;
        self
    }

    /// margin_right sets the margin on the right.
    pub const fn margin_right(mut self, n: i32) -> Style {
        self.margin[1] = max0(n);
        self.props |= MARGIN_RIGHT;
        self
    }

    /// margin_bottom sets the margin below.
    pub const fn margin_bottom(mut self, n: i32) -> Style {
        self.margin[2] = max0(n);
        self.props |= MARGIN_BOTTOM;
        self
    }

    /// margin_left sets the margin on the left.
    pub const fn margin_left(mut self, n: i32) -> Style {
        self.margin[3] = max0(n);
        self.props |= MARGIN_LEFT;
        self
    }

    /// margin_background sets the margins' color.
    pub const fn margin_background(mut self, c: Color) -> Style {
        self.margin_bg = c;
        self.props |= MARGIN_BACKGROUND;
        self
    }

    /// margin_char sets what margins are drawn with (a space by default).
    pub const fn margin_char(mut self, c: char) -> Style {
        self.margin_char = c;
        self.props |= MARGIN_CHAR;
        self
    }

    /// border sets the border and which of its sides are drawn, one to four
    /// as CSS; none given draws all four.
    pub const fn border(mut self, b: Border, sides: &[bool]) -> Style {
        self.border_style = b;
        self.props |= BORDER_STYLE;
        let [t, r, bo, l] = match which_sides_bool(sides) {
            Some(s) => s,
            None => [true; 4],
        };
        self.border_top(t).border_right(r).border_bottom(bo).border_left(l)
    }

    /// border_style sets the border without saying which sides: all four
    /// are drawn unless a side is set.
    pub const fn border_style(mut self, b: Border) -> Style {
        self.border_style = b;
        self.props |= BORDER_STYLE;
        self
    }

    /// border_top says whether the top border is drawn.
    pub const fn border_top(self, v: bool) -> Style {
        self.set_bool(BORDER_TOP, v)
    }

    /// border_right says whether the right border is drawn.
    pub const fn border_right(self, v: bool) -> Style {
        self.set_bool(BORDER_RIGHT, v)
    }

    /// border_bottom says whether the bottom border is drawn.
    pub const fn border_bottom(self, v: bool) -> Style {
        self.set_bool(BORDER_BOTTOM, v)
    }

    /// border_left says whether the left border is drawn.
    pub const fn border_left(self, v: bool) -> Style {
        self.set_bool(BORDER_LEFT, v)
    }

    /// border_foreground sets the border's colors, one to four as CSS.
    pub const fn border_foreground(mut self, c: &[Color]) -> Style {
        if let Some([t, r, b, l]) = which_sides_color(c) {
            self.border_fg = [t, r, b, l];
            self.props |=
                BORDER_TOP_FOREGROUND | BORDER_RIGHT_FOREGROUND | BORDER_BOTTOM_FOREGROUND | BORDER_LEFT_FOREGROUND;
        }
        self
    }

    /// border_top_foreground sets the top border's color.
    pub const fn border_top_foreground(mut self, c: Color) -> Style {
        self.border_fg[0] = c;
        self.props |= BORDER_TOP_FOREGROUND;
        self
    }

    /// border_right_foreground sets the right border's color.
    pub const fn border_right_foreground(mut self, c: Color) -> Style {
        self.border_fg[1] = c;
        self.props |= BORDER_RIGHT_FOREGROUND;
        self
    }

    /// border_bottom_foreground sets the bottom border's color.
    pub const fn border_bottom_foreground(mut self, c: Color) -> Style {
        self.border_fg[2] = c;
        self.props |= BORDER_BOTTOM_FOREGROUND;
        self
    }

    /// border_left_foreground sets the left border's color.
    pub const fn border_left_foreground(mut self, c: Color) -> Style {
        self.border_fg[3] = c;
        self.props |= BORDER_LEFT_FOREGROUND;
        self
    }

    /// border_background sets the color behind the border, one to four as
    /// CSS.
    pub const fn border_background(mut self, c: &[Color]) -> Style {
        if let Some([t, r, b, l]) = which_sides_color(c) {
            self.border_bg = [t, r, b, l];
            self.props |=
                BORDER_TOP_BACKGROUND | BORDER_RIGHT_BACKGROUND | BORDER_BOTTOM_BACKGROUND | BORDER_LEFT_BACKGROUND;
        }
        self
    }

    /// inline renders on one line: line breaks are dropped, and padding,
    /// margins and borders aren't drawn.
    pub const fn inline(self, v: bool) -> Style {
        self.set_bool(INLINE, v)
    }

    /// max_width cuts every line down to `n` cells, after everything else.
    pub const fn max_width(mut self, n: i32) -> Style {
        self.max_width = max0(n);
        self.props |= MAX_WIDTH;
        self
    }

    /// max_height keeps the first `n` lines, after everything else.
    pub const fn max_height(mut self, n: i32) -> Style {
        self.max_height = max0(n);
        self.props |= MAX_HEIGHT;
        self
    }

    /// tab_width sets how many spaces a tab becomes: 0 drops tabs,
    /// [`NO_TAB_CONVERSION`] leaves them.
    pub const fn tab_width(mut self, n: i32) -> Style {
        self.tab_width = if n <= -1 { -1 } else { n };
        self.props |= TAB_WIDTH;
        self
    }

    /// underline_spaces says whether spaces are underlined too.
    pub const fn underline_spaces(self, v: bool) -> Style {
        self.set_bool(UNDERLINE_SPACES, v)
    }

    /// strikethrough_spaces says whether spaces are struck through too.
    pub const fn strikethrough_spaces(self, v: bool) -> Style {
        self.set_bool(STRIKETHROUGH_SPACES, v)
    }

    /// transform sets a function the text goes through before rendering.
    pub const fn transform(mut self, f: fn(&str) -> String) -> Style {
        self.transform = Some(f);
        self.props |= TRANSFORM;
        self
    }

    // Unsetting

    /// unset_width removes the width.
    pub const fn unset_width(mut self) -> Style {
        self.props &= !WIDTH;
        self
    }

    /// unset_height removes the height.
    pub const fn unset_height(mut self) -> Style {
        self.props &= !HEIGHT;
        self
    }

    /// unset_max_width removes the maximum width.
    pub const fn unset_max_width(mut self) -> Style {
        self.props &= !MAX_WIDTH;
        self
    }

    /// unset_max_height removes the maximum height.
    pub const fn unset_max_height(mut self) -> Style {
        self.props &= !MAX_HEIGHT;
        self
    }

    /// unset_inline removes inline rendering.
    pub const fn unset_inline(mut self) -> Style {
        self.props &= !INLINE;
        self
    }

    /// unset_foreground removes the text's color.
    pub const fn unset_foreground(mut self) -> Style {
        self.props &= !FOREGROUND;
        self
    }

    /// unset_background removes the background color.
    pub const fn unset_background(mut self) -> Style {
        self.props &= !BACKGROUND;
        self
    }

    /// unset_bold removes bold.
    pub const fn unset_bold(mut self) -> Style {
        self.props &= !BOLD;
        self
    }

    /// unset_padding removes all padding.
    pub const fn unset_padding(mut self) -> Style {
        self.props &= !(PADDING_TOP | PADDING_RIGHT | PADDING_BOTTOM | PADDING_LEFT);
        self
    }

    /// unset_margins removes all margins.
    pub const fn unset_margins(mut self) -> Style {
        self.props &= !(MARGIN_TOP | MARGIN_RIGHT | MARGIN_BOTTOM | MARGIN_LEFT);
        self
    }

    /// unset_border_style removes the border.
    pub const fn unset_border_style(mut self) -> Style {
        self.props &= !BORDER_STYLE;
        self
    }

    // Getting

    /// get_bold says whether text is bold.
    pub const fn get_bold(&self) -> bool {
        self.get_bool(BOLD, false)
    }
    /// get_italic says whether text is italic.
    pub const fn get_italic(&self) -> bool {
        self.get_bool(ITALIC, false)
    }
    /// get_underline says whether text is underlined.
    pub const fn get_underline(&self) -> bool {
        self.ul != UNDERLINE_NONE
    }
    /// get_strikethrough says whether text is struck through.
    pub const fn get_strikethrough(&self) -> bool {
        self.get_bool(STRIKETHROUGH, false)
    }
    /// get_reverse says whether colors are swapped.
    pub const fn get_reverse(&self) -> bool {
        self.get_bool(REVERSE, false)
    }
    /// get_blink says whether text blinks.
    pub const fn get_blink(&self) -> bool {
        self.get_bool(BLINK, false)
    }
    /// get_faint says whether text is dimmed.
    pub const fn get_faint(&self) -> bool {
        self.get_bool(FAINT, false)
    }
    /// get_foreground is the text's color.
    pub const fn get_foreground(&self) -> Color {
        self.get_color(FOREGROUND, self.fg)
    }
    /// get_background is the background color.
    pub const fn get_background(&self) -> Color {
        self.get_color(BACKGROUND, self.bg)
    }
    /// get_width is the width set, or 0.
    pub const fn get_width(&self) -> i32 {
        self.get_int(WIDTH, self.width)
    }
    /// get_height is the height set, or 0.
    pub const fn get_height(&self) -> i32 {
        self.get_int(HEIGHT, self.height)
    }
    /// get_align_horizontal is the horizontal alignment.
    pub const fn get_align_horizontal(&self) -> Position {
        if self.is_set(ALIGN_HORIZONTAL) { self.align_horizontal } else { LEFT }
    }
    /// get_align_vertical is the vertical alignment.
    pub const fn get_align_vertical(&self) -> Position {
        if self.is_set(ALIGN_VERTICAL) { self.align_vertical } else { TOP }
    }
    /// get_padding_top is the padding above.
    pub const fn get_padding_top(&self) -> i32 {
        self.get_int(PADDING_TOP, self.padding[0])
    }
    /// get_padding_right is the padding on the right.
    pub const fn get_padding_right(&self) -> i32 {
        self.get_int(PADDING_RIGHT, self.padding[1])
    }
    /// get_padding_bottom is the padding below.
    pub const fn get_padding_bottom(&self) -> i32 {
        self.get_int(PADDING_BOTTOM, self.padding[2])
    }
    /// get_padding_left is the padding on the left.
    pub const fn get_padding_left(&self) -> i32 {
        self.get_int(PADDING_LEFT, self.padding[3])
    }
    /// get_horizontal_padding is the padding on the left and right.
    pub const fn get_horizontal_padding(&self) -> i32 {
        self.get_padding_left() + self.get_padding_right()
    }
    /// get_vertical_padding is the padding above and below.
    pub const fn get_vertical_padding(&self) -> i32 {
        self.get_padding_top() + self.get_padding_bottom()
    }
    /// get_margin_top is the margin above.
    pub const fn get_margin_top(&self) -> i32 {
        self.get_int(MARGIN_TOP, self.margin[0])
    }
    /// get_margin_right is the margin on the right.
    pub const fn get_margin_right(&self) -> i32 {
        self.get_int(MARGIN_RIGHT, self.margin[1])
    }
    /// get_margin_bottom is the margin below.
    pub const fn get_margin_bottom(&self) -> i32 {
        self.get_int(MARGIN_BOTTOM, self.margin[2])
    }
    /// get_margin_left is the margin on the left.
    pub const fn get_margin_left(&self) -> i32 {
        self.get_int(MARGIN_LEFT, self.margin[3])
    }
    /// get_horizontal_margins is the margins on the left and right.
    pub const fn get_horizontal_margins(&self) -> i32 {
        self.get_margin_left() + self.get_margin_right()
    }
    /// get_vertical_margins is the margins above and below.
    pub const fn get_vertical_margins(&self) -> i32 {
        self.get_margin_top() + self.get_margin_bottom()
    }
    /// get_border_style is the border, or none.
    pub const fn get_border_style(&self) -> Border {
        if self.is_set(BORDER_STYLE) { self.border_style } else { border::NO_BORDER }
    }
    /// get_border_top says whether the top border is drawn.
    pub const fn get_border_top(&self) -> bool {
        self.get_bool(BORDER_TOP, false)
    }
    /// get_border_right says whether the right border is drawn.
    pub const fn get_border_right(&self) -> bool {
        self.get_bool(BORDER_RIGHT, false)
    }
    /// get_border_bottom says whether the bottom border is drawn.
    pub const fn get_border_bottom(&self) -> bool {
        self.get_bool(BORDER_BOTTOM, false)
    }
    /// get_border_left says whether the left border is drawn.
    pub const fn get_border_left(&self) -> bool {
        self.get_bool(BORDER_LEFT, false)
    }

    /// is_border_style_set_without_sides: a border with no side said draws
    /// all four.
    fn is_border_style_set_without_sides(&self) -> bool {
        let b = self.get_border_style();
        b != border::NO_BORDER && !(self.is_set(BORDER_TOP | BORDER_RIGHT | BORDER_BOTTOM | BORDER_LEFT))
    }

    /// get_border_top_size is how many lines the top border takes.
    pub fn get_border_top_size(&self) -> i32 {
        if self.is_border_style_set_without_sides() {
            return 1;
        }
        if !self.get_border_top() {
            return 0;
        }
        self.get_border_style().get_top_size()
    }
    /// get_border_left_size is how many cells the left border takes.
    pub fn get_border_left_size(&self) -> i32 {
        if self.is_border_style_set_without_sides() {
            return 1;
        }
        if !self.get_border_left() {
            return 0;
        }
        self.get_border_style().get_left_size()
    }
    /// get_border_bottom_size is how many lines the bottom border takes.
    pub fn get_border_bottom_size(&self) -> i32 {
        if self.is_border_style_set_without_sides() {
            return 1;
        }
        if !self.get_border_bottom() {
            return 0;
        }
        self.get_border_style().get_bottom_size()
    }
    /// get_border_right_size is how many cells the right border takes.
    pub fn get_border_right_size(&self) -> i32 {
        if self.is_border_style_set_without_sides() {
            return 1;
        }
        if !self.get_border_right() {
            return 0;
        }
        self.get_border_style().get_right_size()
    }
    /// get_horizontal_border_size is the left and right borders' width.
    pub fn get_horizontal_border_size(&self) -> i32 {
        self.get_border_left_size() + self.get_border_right_size()
    }
    /// get_vertical_border_size is the top and bottom borders' height.
    pub fn get_vertical_border_size(&self) -> i32 {
        self.get_border_top_size() + self.get_border_bottom_size()
    }
    /// get_inline says whether the style renders inline.
    pub const fn get_inline(&self) -> bool {
        self.get_bool(INLINE, false)
    }
    /// get_max_width is the maximum width, or 0.
    pub const fn get_max_width(&self) -> i32 {
        self.get_int(MAX_WIDTH, self.max_width)
    }
    /// get_max_height is the maximum height, or 0.
    pub const fn get_max_height(&self) -> i32 {
        self.get_int(MAX_HEIGHT, self.max_height)
    }
    /// get_horizontal_frame_size is the margins, padding and borders on
    /// the left and right.
    pub fn get_horizontal_frame_size(&self) -> i32 {
        self.get_horizontal_margins() + self.get_horizontal_padding() + self.get_horizontal_border_size()
    }
    /// get_vertical_frame_size is the margins, padding and borders above
    /// and below.
    pub fn get_vertical_frame_size(&self) -> i32 {
        self.get_vertical_margins() + self.get_vertical_padding() + self.get_vertical_border_size()
    }
    /// get_frame_size is the horizontal and vertical frame sizes.
    pub fn get_frame_size(&self) -> (i32, i32) {
        (self.get_horizontal_frame_size(), self.get_vertical_frame_size())
    }

    /// inherit takes every rule set in `i` that isn't set here; margins,
    /// padding and the text set with `set_string` aren't inherited.
    pub fn inherit(mut self, i: Style) -> Style {
        let mut k: u64 = BOLD;
        while k <= TRANSFORM {
            if !i.is_set(k) {
                k <<= 1;
                continue;
            }
            if matches!(
                k,
                MARGIN_TOP
                    | MARGIN_RIGHT
                    | MARGIN_BOTTOM
                    | MARGIN_LEFT
                    | PADDING_TOP
                    | PADDING_RIGHT
                    | PADDING_BOTTOM
                    | PADDING_LEFT
            ) {
                k <<= 1;
                continue;
            }
            if k == BACKGROUND && !self.is_set(MARGIN_BACKGROUND) && !i.is_set(MARGIN_BACKGROUND) {
                // the margins also inherit the background color
                self = self.margin_background(i.bg);
            }
            if !self.is_set(k) {
                self.set_from(k, &i);
            }
            k <<= 1;
        }
        self
    }

    fn set_from(&mut self, k: u64, i: &Style) {
        match k {
            FOREGROUND => self.fg = i.fg,
            BACKGROUND => self.bg = i.bg,
            UNDERLINE_COLOR => self.ul_color = i.ul_color,
            UNDERLINE => self.ul = i.ul,
            WIDTH => self.width = i.width,
            HEIGHT => self.height = i.height,
            ALIGN_HORIZONTAL => self.align_horizontal = i.align_horizontal,
            ALIGN_VERTICAL => self.align_vertical = i.align_vertical,
            PADDING_CHAR => self.padding_char = i.padding_char,
            MARGIN_BACKGROUND => self.margin_bg = i.margin_bg,
            MARGIN_CHAR => self.margin_char = i.margin_char,
            BORDER_STYLE => self.border_style = i.border_style,
            BORDER_TOP_FOREGROUND => self.border_fg[0] = i.border_fg[0],
            BORDER_RIGHT_FOREGROUND => self.border_fg[1] = i.border_fg[1],
            BORDER_BOTTOM_FOREGROUND => self.border_fg[2] = i.border_fg[2],
            BORDER_LEFT_FOREGROUND => self.border_fg[3] = i.border_fg[3],
            BORDER_FOREGROUND_BLEND | BORDER_FOREGROUND_BLEND_OFFSET => {}
            BORDER_TOP_BACKGROUND => self.border_bg[0] = i.border_bg[0],
            BORDER_RIGHT_BACKGROUND => self.border_bg[1] = i.border_bg[1],
            BORDER_BOTTOM_BACKGROUND => self.border_bg[2] = i.border_bg[2],
            BORDER_LEFT_BACKGROUND => self.border_bg[3] = i.border_bg[3],
            MAX_WIDTH => self.max_width = i.max_width,
            MAX_HEIGHT => self.max_height = i.max_height,
            TAB_WIDTH => self.tab_width = i.tab_width,
            TRANSFORM => self.transform = i.transform,
            _ => {
                // a boolean: copy its bit
                if i.attrs & k != 0 {
                    self.attrs |= k;
                } else {
                    self.attrs &= !k;
                }
            }
        }
        self.props |= k;
    }

    // Rendering

    fn maybe_convert_tabs(&self, s: &str) -> String {
        let tw = if self.is_set(TAB_WIDTH) { self.tab_width } else { TAB_WIDTH_DEFAULT };
        match tw {
            -1 => s.to_string(),
            0 => s.replace('\t', ""),
            n => s.replace('\t', &" ".repeat(n.max(0) as usize)),
        }
    }

    /// render applies the style to `s`.
    pub fn render(&self, s: &str) -> String {
        self.render_strs(&[s])
    }

    fn render_strs(&self, strs: &[&str]) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(strs.len() + 1);
        if !self.value.is_empty() {
            parts.push(self.value);
        }
        parts.extend_from_slice(strs);
        let mut str = parts.join(" ");

        let bold = self.get_bool(BOLD, false);
        let italic = self.get_bool(ITALIC, false);
        let strikethrough = self.get_bool(STRIKETHROUGH, false);
        let reverse = self.get_bool(REVERSE, false);
        let blink = self.get_bool(BLINK, false);
        let faint = self.get_bool(FAINT, false);

        let fg = self.get_color(FOREGROUND, self.fg);
        let bg = self.get_color(BACKGROUND, self.bg);
        let ul = self.get_color(UNDERLINE_COLOR, self.ul_color);

        let underline = self.ul != UNDERLINE_NONE;
        let mut width = self.get_width();
        let mut height = self.get_height();
        let horizontal_align = self.get_align_horizontal();
        let vertical_align = self.get_align_vertical();

        let top_padding = self.get_padding_top();
        let right_padding = self.get_padding_right();
        let bottom_padding = self.get_padding_bottom();
        let left_padding = self.get_padding_left();

        let horizontal_border_size = self.get_horizontal_border_size();
        let vertical_border_size = self.get_vertical_border_size();

        let color_whitespace = self.get_bool(COLOR_WHITESPACE, true);
        let inline = self.get_bool(INLINE, false);
        let max_width = self.get_max_width();
        let max_height = self.get_max_height();

        let underline_spaces =
            self.get_bool(UNDERLINE_SPACES, false) || (underline && self.get_bool(UNDERLINE_SPACES, true));
        let strikethrough_spaces =
            self.get_bool(STRIKETHROUGH_SPACES, false) || (strikethrough && self.get_bool(STRIKETHROUGH_SPACES, true));

        // Do we need to style whitespace (padding and space outside
        // paragraphs) separately?
        let style_whitespace = reverse;

        // Do we need to style spaces separately?
        let use_space_styler = (underline && !underline_spaces)
            || (strikethrough && !strikethrough_spaces)
            || underline_spaces
            || strikethrough_spaces;

        if self.is_set(TRANSFORM)
            && let Some(f) = self.transform
        {
            str = f(&str);
        }

        if self.props == 0 {
            return self.maybe_convert_tabs(&str);
        }

        let mut te = Sgr::default();
        let mut te_space = Sgr::default();
        let mut te_whitespace = Sgr::default();

        if bold {
            te = te.bold();
        }
        if italic {
            te = te.italic(true);
        }
        if underline {
            te = te.underline(true);
        }
        if reverse {
            te_whitespace = te_whitespace.reverse(true);
            te = te.reverse(true);
        }
        if blink {
            te = te.blink(true);
        }
        if faint {
            te = te.faint();
        }

        if fg != Color::NoColor {
            te = te.foreground(Some(fg));
            if style_whitespace {
                te_whitespace = te_whitespace.foreground(Some(fg));
            }
            if use_space_styler {
                te_space = te_space.foreground(Some(fg));
            }
        }
        if bg != Color::NoColor {
            te = te.background(Some(bg));
            if color_whitespace {
                te_whitespace = te_whitespace.background(Some(bg));
            }
            if use_space_styler {
                te_space = te_space.background(Some(bg));
            }
        }
        if ul != Color::NoColor {
            te = te.underline_color(Some(ul));
            if color_whitespace {
                te_whitespace = te_whitespace.underline_color(Some(ul));
            }
            if use_space_styler {
                te_space = te_space.underline_color(Some(ul));
            }
        }
        if underline {
            te = te.underline_style(self.ul);
        }
        if strikethrough {
            te = te.strikethrough(true);
        }
        if underline_spaces {
            te_space = te_space.underline(true);
        }
        if strikethrough_spaces {
            te_space = te_space.strikethrough(true);
        }

        // Potentially convert tabs to spaces
        str = self.maybe_convert_tabs(&str);
        // carriage returns can cause strange behaviour when rendering.
        str = str.replace("\r\n", "\n");

        // Strip newlines in single line mode
        if inline {
            str = str.replace('\n', "");
        }

        // Include borders in block size.
        width -= horizontal_border_size;
        height -= vertical_border_size;

        // Word wrap
        if !inline && width > 0 {
            let wrap_at = width - left_padding - right_padding;
            str = wrap::wrap(&str, wrap_at, "");
        }

        // Render core text
        {
            let mut b = String::with_capacity(str.len() * 2);
            let prefix = te.string();
            for (i, line) in str.split('\n').enumerate() {
                if i > 0 {
                    b.push('\n');
                }
                if use_space_styler {
                    // Look for spaces and apply a different styler
                    let space_prefix = te_space.string();
                    for c in line.chars() {
                        let (st, pre) = if c.is_whitespace() { (&te_space, &space_prefix) } else { (&te, &prefix) };
                        if st.is_empty() {
                            b.push(c);
                        } else {
                            b.push_str(pre);
                            b.push(c);
                            b.push_str(ansi::RESET_STYLE);
                        }
                    }
                } else if te.is_empty() {
                    b.push_str(line);
                } else {
                    b.push_str(&prefix);
                    b.push_str(line);
                    b.push_str(ansi::RESET_STYLE);
                }
            }
            str = b;
        }

        // Padding
        if !inline {
            let pad_char = if self.padding_char == '\0' { ' ' } else { self.padding_char };
            let ws = if color_whitespace || style_whitespace { Some(&te_whitespace) } else { None };
            if left_padding > 0 {
                str = pad(&str, -left_padding, ws, pad_char);
            }
            if right_padding > 0 {
                str = pad(&str, right_padding, ws, pad_char);
            }
            if top_padding > 0 {
                str = "\n".repeat(top_padding as usize) + &str;
            }
            if bottom_padding > 0 {
                str += &"\n".repeat(bottom_padding as usize);
            }
        }

        // Height
        if height > 0 {
            str = align_text_vertical(&str, vertical_align, height);
        }

        // Set alignment. This will also pad short lines with spaces so that
        // all lines are the same length, so we run it under a few different
        // conditions beyond alignment.
        {
            let num_lines = str.matches('\n').count();
            if num_lines != 0 || width != 0 {
                let ws = if color_whitespace || style_whitespace { Some(&te_whitespace) } else { None };
                str = align_text_horizontal(&str, horizontal_align, width, ws);
            }
        }

        if !inline {
            str = self.apply_border(&str);
            str = self.apply_margins(&str, inline);
        }

        // Truncate according to MaxWidth
        if max_width > 0 {
            str = str.split('\n').map(|l| ansi::truncate(l, max_width, "")).collect::<Vec<_>>().join("\n");
        }

        // Truncate according to MaxHeight
        if max_height > 0 {
            let lines: Vec<&str> = str.split('\n').collect();
            let h = (max_height as usize).min(lines.len());
            str = lines[..h].join("\n");
        }

        str
    }

    fn apply_margins(&self, s: &str, inline: bool) -> String {
        let top = self.get_margin_top();
        let right = self.get_margin_right();
        let bottom = self.get_margin_bottom();
        let left = self.get_margin_left();
        let mut style = Sgr::default();
        let bgc = self.get_color(MARGIN_BACKGROUND, self.margin_bg);
        if bgc != Color::NoColor {
            style = style.background(Some(bgc));
        }
        // Add left and right margin
        let margin_char = if self.margin_char == '\0' { ' ' } else { self.margin_char };
        let mut s = pad(s, -left, Some(&style), margin_char);
        s = pad(&s, right, Some(&style), margin_char);
        // Top/bottom margin
        if !inline {
            let (_, width) = get_lines(&s);
            let spaces = " ".repeat(width.max(0) as usize);
            if top > 0 {
                s = style.styled(&format!("{spaces}\n").repeat(top as usize)) + &s;
            }
            if bottom > 0 {
                s += &style.styled(&format!("\n{spaces}").repeat(bottom as usize));
            }
        }
        s
    }
}

/// pad adds `n` of `r` to the right of every line, or to the left when `n`
/// is negative, styled with `style` when there's one.
fn pad(s: &str, n: i32, style: Option<&Sgr>, r: char) -> String {
    if n == 0 {
        return s.to_string();
    }
    let mut sp = r.to_string().repeat(n.unsigned_abs() as usize);
    if let Some(st) = style {
        sp = st.styled(&sp);
    }
    let mut b = String::with_capacity(s.len() + sp.len() * 4);
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            b.push('\n');
        }
        if n > 0 {
            b.push_str(line);
            b.push_str(&sp);
        } else {
            b.push_str(&sp);
            b.push_str(line);
        }
    }
    b
}

#[cfg(test)]
mod tests;
