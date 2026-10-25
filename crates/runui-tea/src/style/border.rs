//! Borders: the characters a box is drawn with, and drawing one around a
//! block of text.

use super::Style;
use crate::ansi::{self, Color, sgr::Sgr};

/// Border is the characters a border is drawn with: its sides, corners,
/// and the joins tables use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Border {
    /// The top edge.
    pub top: &'static str,
    /// The bottom edge.
    pub bottom: &'static str,
    /// The left edge.
    pub left: &'static str,
    /// The right edge.
    pub right: &'static str,
    /// The top left corner.
    pub top_left: &'static str,
    /// The top right corner.
    pub top_right: &'static str,
    /// The bottom left corner.
    pub bottom_left: &'static str,
    /// The bottom right corner.
    pub bottom_right: &'static str,
    /// Where a row line meets the left edge (tables).
    pub middle_left: &'static str,
    /// Where a row line meets the right edge (tables).
    pub middle_right: &'static str,
    /// Where lines cross (tables).
    pub middle: &'static str,
    /// Where a column line meets the top edge (tables).
    pub middle_top: &'static str,
    /// Where a column line meets the bottom edge (tables).
    pub middle_bottom: &'static str,
}

impl Border {
    /// get_top_size is how tall the top edge is.
    pub fn get_top_size(&self) -> i32 {
        edge_width(&[self.top_left, self.top, self.top_right])
    }
    /// get_right_size is how wide the right edge is.
    pub fn get_right_size(&self) -> i32 {
        edge_width(&[self.top_right, self.right, self.bottom_right])
    }
    /// get_bottom_size is how tall the bottom edge is.
    pub fn get_bottom_size(&self) -> i32 {
        edge_width(&[self.bottom_left, self.bottom, self.bottom_right])
    }
    /// get_left_size is how wide the left edge is.
    pub fn get_left_size(&self) -> i32 {
        edge_width(&[self.top_left, self.left, self.bottom_left])
    }
}

fn edge_width(parts: &[&str]) -> i32 {
    parts.iter().map(|p| max_rune_width(p)).max().unwrap_or(0)
}

/// max_rune_width is the widest grapheme in `s`.
fn max_rune_width(s: &str) -> i32 {
    use unicode_segmentation::UnicodeSegmentation;
    s.graphemes(true).map(ansi::grapheme_width).max().unwrap_or(0)
}

pub(crate) const NO_BORDER: Border = Border {
    top: "",
    bottom: "",
    left: "",
    right: "",
    top_left: "",
    top_right: "",
    bottom_left: "",
    bottom_right: "",
    middle_left: "",
    middle_right: "",
    middle: "",
    middle_top: "",
    middle_bottom: "",
};

/// normal_border is a border of thin straight lines with square corners.
pub const fn normal_border() -> Border {
    Border {
        top: "─",
        bottom: "─",
        left: "│",
        right: "│",
        top_left: "┌",
        top_right: "┐",
        bottom_left: "└",
        bottom_right: "┘",
        middle_left: "├",
        middle_right: "┤",
        middle: "┼",
        middle_top: "┬",
        middle_bottom: "┴",
    }
}

/// rounded_border is a border of thin lines with rounded corners.
pub const fn rounded_border() -> Border {
    Border {
        top: "─",
        bottom: "─",
        left: "│",
        right: "│",
        top_left: "╭",
        top_right: "╮",
        bottom_left: "╰",
        bottom_right: "╯",
        middle_left: "├",
        middle_right: "┤",
        middle: "┼",
        middle_top: "┬",
        middle_bottom: "┴",
    }
}

/// block_border is a border of full blocks.
pub const fn block_border() -> Border {
    Border {
        top: "█",
        bottom: "█",
        left: "█",
        right: "█",
        top_left: "█",
        top_right: "█",
        bottom_left: "█",
        bottom_right: "█",
        middle_left: "█",
        middle_right: "█",
        middle: "█",
        middle_top: "█",
        middle_bottom: "█",
    }
}

/// outer_half_block_border is a border of half blocks outside the frame.
pub const fn outer_half_block_border() -> Border {
    Border {
        top: "▀",
        bottom: "▄",
        left: "▌",
        right: "▐",
        top_left: "▛",
        top_right: "▜",
        bottom_left: "▙",
        bottom_right: "▟",
        ..NO_BORDER
    }
}

/// inner_half_block_border is a border of half blocks inside the frame.
pub const fn inner_half_block_border() -> Border {
    Border {
        top: "▄",
        bottom: "▀",
        left: "▐",
        right: "▌",
        top_left: "▗",
        top_right: "▖",
        bottom_left: "▝",
        bottom_right: "▘",
        ..NO_BORDER
    }
}

/// thick_border is a border of thick lines.
pub const fn thick_border() -> Border {
    Border {
        top: "━",
        bottom: "━",
        left: "┃",
        right: "┃",
        top_left: "┏",
        top_right: "┓",
        bottom_left: "┗",
        bottom_right: "┛",
        middle_left: "┣",
        middle_right: "┫",
        middle: "╋",
        middle_top: "┳",
        middle_bottom: "┻",
    }
}

/// double_border is a border of double lines.
pub const fn double_border() -> Border {
    Border {
        top: "═",
        bottom: "═",
        left: "║",
        right: "║",
        top_left: "╔",
        top_right: "╗",
        bottom_left: "╚",
        bottom_right: "╝",
        middle_left: "╠",
        middle_right: "╣",
        middle: "╬",
        middle_top: "╦",
        middle_bottom: "╩",
    }
}

/// hidden_border is a border of spaces: it takes room and shows nothing.
pub const fn hidden_border() -> Border {
    Border {
        top: " ",
        bottom: " ",
        left: " ",
        right: " ",
        top_left: " ",
        top_right: " ",
        bottom_left: " ",
        bottom_right: " ",
        middle_left: " ",
        middle_right: " ",
        middle: " ",
        middle_top: " ",
        middle_bottom: " ",
    }
}

/// markdown_border is a border like a Markdown table's.
pub const fn markdown_border() -> Border {
    Border {
        top: "-",
        bottom: "-",
        left: "|",
        right: "|",
        top_left: "|",
        top_right: "|",
        bottom_left: "|",
        bottom_right: "|",
        middle_left: "|",
        middle_right: "|",
        middle: "|",
        middle_top: "|",
        middle_bottom: "|",
    }
}

/// ascii_border is a border of ASCII characters.
pub const fn ascii_border() -> Border {
    Border {
        top: "-",
        bottom: "-",
        left: "|",
        right: "|",
        top_left: "+",
        top_right: "+",
        bottom_left: "+",
        bottom_right: "+",
        middle_left: "+",
        middle_right: "+",
        middle: "+",
        middle_top: "+",
        middle_bottom: "+",
    }
}

fn first_char(s: &str) -> &str {
    match s.chars().next() {
        Some(c) => &s[..c.len_utf8()],
        None => s,
    }
}

fn render_horizontal_edge(left: &str, middle: &str, right: &str, width: i32) -> String {
    let middle = if middle.is_empty() { " " } else { middle };
    let left_width = ansi::string_width(left);
    let right_width = ansi::string_width(right);
    let runes: Vec<char> = middle.chars().collect();
    let mut j = 0;
    let mut out = String::from(left);
    let mut i = 0;
    while i < width - left_width - right_width {
        let r = runes[j];
        out.push(r);
        i += ansi::string_width(r.encode_utf8(&mut [0; 4]));
        j += 1;
        if j >= runes.len() {
            j = 0;
        }
        if runes.len() == 1 && ansi::string_width(middle) == 0 {
            // a zero-width middle would never fill the edge
            break;
        }
    }
    out.push_str(right);
    out
}

fn style_border(border: &str, fg: Color, bg: Color) -> String {
    if fg == Color::NoColor && bg == Color::NoColor {
        return border.to_string();
    }
    let mut style = Sgr::default();
    if fg != Color::NoColor {
        style = style.foreground(Some(fg));
    }
    if bg != Color::NoColor {
        style = style.background(Some(bg));
    }
    style.styled(border)
}

impl Style {
    pub(super) fn apply_border(&self, s: &str) -> String {
        let mut border = self.get_border_style();
        let mut has_top = self.get_border_top();
        let mut has_right = self.get_border_right();
        let mut has_bottom = self.get_border_bottom();
        let mut has_left = self.get_border_left();

        if self.is_border_style_set_without_sides() {
            has_top = true;
            has_right = true;
            has_bottom = true;
            has_left = true;
        }

        if border == NO_BORDER || (!has_top && !has_right && !has_bottom && !has_left) {
            return s.to_string();
        }

        let (lines, mut width) = super::get_lines(s);

        if has_left {
            if border.left.is_empty() {
                border.left = " ";
            }
            width += max_rune_width(border.left);
        }
        if has_right {
            if border.right.is_empty() {
                border.right = " ";
            }
            width += max_rune_width(border.right);
        }

        // If corners should be rendered but are set with the empty string,
        // fill them with a single space.
        if has_top && has_left && border.top_left.is_empty() {
            border.top_left = " ";
        }
        if has_top && has_right && border.top_right.is_empty() {
            border.top_right = " ";
        }
        if has_bottom && has_left && border.bottom_left.is_empty() {
            border.bottom_left = " ";
        }
        if has_bottom && has_right && border.bottom_right.is_empty() {
            border.bottom_right = " ";
        }

        // Figure out which corners we should actually be using based on
        // which sides are set to show.
        if has_top {
            match (has_left, has_right) {
                (false, false) => {
                    border.top_left = "";
                    border.top_right = "";
                }
                (false, true) => border.top_left = "",
                (true, false) => border.top_right = "",
                _ => {}
            }
        }
        if has_bottom {
            match (has_left, has_right) {
                (false, false) => {
                    border.bottom_left = "";
                    border.bottom_right = "";
                }
                (false, true) => border.bottom_left = "",
                (true, false) => border.bottom_right = "",
                _ => {}
            }
        }

        // For now, limit corners to one rune.
        border.top_left = first_char(border.top_left);
        border.top_right = first_char(border.top_right);
        border.bottom_right = first_char(border.bottom_right);
        border.bottom_left = first_char(border.bottom_left);

        let fg = |k: u64, c: Color| self.get_color(k, c);
        let top_fg = fg(super::BORDER_TOP_FOREGROUND, self.border_fg[0]);
        let right_fg = fg(super::BORDER_RIGHT_FOREGROUND, self.border_fg[1]);
        let bottom_fg = fg(super::BORDER_BOTTOM_FOREGROUND, self.border_fg[2]);
        let left_fg = fg(super::BORDER_LEFT_FOREGROUND, self.border_fg[3]);
        let top_bg = fg(super::BORDER_TOP_BACKGROUND, self.border_bg[0]);
        let right_bg = fg(super::BORDER_RIGHT_BACKGROUND, self.border_bg[1]);
        let bottom_bg = fg(super::BORDER_BOTTOM_BACKGROUND, self.border_bg[2]);
        let left_bg = fg(super::BORDER_LEFT_BACKGROUND, self.border_bg[3]);

        let mut out = String::with_capacity(s.len() * 2);

        // Render top
        if has_top {
            let top = render_horizontal_edge(border.top_left, border.top, border.top_right, width);
            out.push_str(&style_border(&top, top_fg, top_bg));
            out.push('\n');
        }

        let left_runes: Vec<char> = border.left.chars().collect();
        let mut left_index = 0;
        let right_runes: Vec<char> = border.right.chars().collect();
        let mut right_index = 0;

        // Render sides
        let n = lines.len();
        for (i, l) in lines.iter().enumerate() {
            if has_left {
                let r = left_runes[left_index].to_string();
                left_index += 1;
                if left_index >= left_runes.len() {
                    left_index = 0;
                }
                out.push_str(&style_border(&r, left_fg, left_bg));
            }
            out.push_str(l);
            if has_right {
                let r = right_runes[right_index].to_string();
                right_index += 1;
                if right_index >= right_runes.len() {
                    right_index = 0;
                }
                out.push_str(&style_border(&r, right_fg, right_bg));
            }
            if i < n - 1 {
                out.push('\n');
            }
        }

        // Render bottom
        if has_bottom {
            let bottom = render_horizontal_edge(border.bottom_left, border.bottom, border.bottom_right, width);
            out.push('\n');
            out.push_str(&style_border(&bottom, bottom_fg, bottom_bg));
        }

        out
    }
}
