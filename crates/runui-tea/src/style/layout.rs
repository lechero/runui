//! Blocks of text: their size, joining them side by side or one above the
//! other, placing one in a bigger space, and aligning lines.

use crate::ansi::{self, sgr::Sgr};

/// Position is where something goes along an axis, from 0 (top or left)
/// to 1 (bottom or right), as Lip Gloss's `Position`.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Position(pub f64);

impl Position {
    fn value(self) -> f64 {
        self.0.clamp(0.0, 1.0)
    }
}

/// TOP aligns to the top.
pub const TOP: Position = Position(0.0);
/// BOTTOM aligns to the bottom.
pub const BOTTOM: Position = Position(1.0);
/// CENTER aligns to the middle.
pub const CENTER: Position = Position(0.5);
/// LEFT aligns to the left.
pub const LEFT: Position = Position(0.0);
/// RIGHT aligns to the right.
pub const RIGHT: Position = Position(1.0);

/// width is how wide a block of text is: its widest line, in cells.
pub fn width(s: &str) -> i32 {
    s.split('\n').map(ansi::string_width).max().unwrap_or(0)
}

/// height is how many lines a block of text has.
pub fn height(s: &str) -> i32 {
    s.matches('\n').count() as i32 + 1
}

/// size is a block's width and height.
pub fn size(s: &str) -> (i32, i32) {
    (width(s), height(s))
}

/// get_lines splits a block into lines (tabs as four spaces) and says how
/// wide the widest is.
pub(crate) fn get_lines(s: &str) -> (Vec<String>, i32) {
    let s = s.replace('\t', "    ").replace("\r\n", "\n");
    let lines: Vec<String> = s.split('\n').map(str::to_string).collect();
    let widest = lines.iter().map(|l| ansi::string_width(l)).max().unwrap_or(0);
    (lines, widest)
}

fn spaces(n: i32) -> String {
    " ".repeat(n.max(0) as usize)
}

/// join_horizontal puts blocks side by side, the shorter ones placed along
/// the vertical axis at `pos` (TOP, CENTER, BOTTOM or in between), each
/// block padded to its width.
pub fn join_horizontal(pos: Position, strs: &[&str]) -> String {
    if strs.is_empty() {
        return String::new();
    }
    if strs.len() == 1 {
        return strs[0].to_string();
    }
    let mut blocks: Vec<Vec<String>> = Vec::with_capacity(strs.len());
    let mut max_widths = Vec::with_capacity(strs.len());
    let mut max_height = 0;
    for s in strs {
        let (lines, w) = get_lines(s);
        max_height = max_height.max(lines.len());
        blocks.push(lines);
        max_widths.push(w);
    }
    for block in blocks.iter_mut() {
        if block.len() >= max_height {
            continue;
        }
        let n = max_height - block.len();
        if pos == TOP {
            block.extend(std::iter::repeat_n(String::new(), n));
        } else if pos == BOTTOM {
            let mut v = vec![String::new(); n];
            v.append(block);
            *block = v;
        } else {
            let split = (n as f64 * pos.value()).round() as usize;
            let top = n - split;
            let bottom = n - top;
            let mut v = vec![String::new(); n - top];
            v.append(block);
            v.extend(std::iter::repeat_n(String::new(), n - bottom));
            *block = v;
        }
    }
    let mut b = String::new();
    let rows = blocks[0].len();
    for i in 0..rows {
        for (j, block) in blocks.iter().enumerate() {
            b.push_str(&block[i]);
            b.push_str(&spaces(max_widths[j] - ansi::string_width(&block[i])));
        }
        if i < rows - 1 {
            b.push('\n');
        }
    }
    b
}

/// join_vertical puts blocks one above the other, the narrower ones placed
/// along the horizontal axis at `pos` (LEFT, CENTER, RIGHT or in between).
pub fn join_vertical(pos: Position, strs: &[&str]) -> String {
    if strs.is_empty() {
        return String::new();
    }
    if strs.len() == 1 {
        return strs[0].to_string();
    }
    let mut blocks = Vec::with_capacity(strs.len());
    let mut max_width = 0;
    for s in strs {
        let (lines, w) = get_lines(s);
        max_width = max_width.max(w);
        blocks.push(lines);
    }
    let mut b = String::new();
    let nb = blocks.len();
    for (i, block) in blocks.iter().enumerate() {
        let nl = block.len();
        for (j, line) in block.iter().enumerate() {
            let w = max_width - ansi::string_width(line);
            if pos == LEFT {
                b.push_str(line);
                b.push_str(&spaces(w));
            } else if pos == RIGHT {
                b.push_str(&spaces(w));
                b.push_str(line);
            } else if w < 1 {
                b.push_str(line);
            } else {
                let split = (w as f64 * pos.value()).round() as i32;
                let right = w - split;
                let left = w - right;
                b.push_str(&spaces(left));
                b.push_str(line);
                b.push_str(&spaces(right));
            }
            if !(i == nb - 1 && j == nl - 1) {
                b.push('\n');
            }
        }
    }
    b
}

/// place puts a block in a space `width` by `height`, at `h_pos` across and
/// `v_pos` down, the rest filled with spaces.
pub fn place(width: i32, height: i32, h_pos: Position, v_pos: Position, s: &str) -> String {
    place_vertical(height, v_pos, &place_horizontal(width, h_pos, s))
}

/// place_horizontal puts a block in a space `width` wide.
pub fn place_horizontal(width: i32, pos: Position, s: &str) -> String {
    let (lines, content_width) = get_lines(s);
    let gap = width - content_width;
    if gap <= 0 {
        return s.to_string();
    }
    let mut b = String::new();
    let n = lines.len();
    for (i, l) in lines.iter().enumerate() {
        // Is this line shorter than the longest line?
        let short = (content_width - ansi::string_width(l)).max(0);
        if pos == LEFT {
            b.push_str(l);
            b.push_str(&spaces(gap + short));
        } else if pos == RIGHT {
            b.push_str(&spaces(gap + short));
            b.push_str(l);
        } else {
            let total_gap = gap + short;
            let split = (total_gap as f64 * pos.value()).round() as i32;
            let left = total_gap - split;
            let right = total_gap - left;
            b.push_str(&spaces(left));
            b.push_str(l);
            b.push_str(&spaces(right));
        }
        if i < n - 1 {
            b.push('\n');
        }
    }
    b
}

/// place_vertical puts a block in a space `height` tall.
pub fn place_vertical(height: i32, pos: Position, s: &str) -> String {
    let content_height = s.matches('\n').count() as i32 + 1;
    let gap = height - content_height;
    if gap <= 0 {
        return s.to_string();
    }
    let (_, width) = get_lines(s);
    let empty_line = spaces(width);
    let mut b = String::new();
    if pos == TOP {
        b.push_str(s);
        b.push('\n');
        for i in 0..gap {
            b.push_str(&empty_line);
            if i < gap - 1 {
                b.push('\n');
            }
        }
    } else if pos == BOTTOM {
        b.push_str(&format!("{empty_line}\n").repeat(gap as usize));
        b.push_str(s);
    } else {
        let split = (gap as f64 * pos.value()).round() as i32;
        let top = gap - split;
        let bottom = gap - top;
        b.push_str(&format!("{empty_line}\n").repeat(top as usize));
        b.push_str(s);
        for _ in 0..bottom {
            b.push('\n');
            b.push_str(&empty_line);
        }
    }
    b
}

/// align_text_horizontal pads every line to the widest (or `width`, if
/// wider), the room going where `pos` says.
pub(crate) fn align_text_horizontal(s: &str, pos: Position, width: i32, style: Option<&Sgr>) -> String {
    let (lines, widest) = get_lines(s);
    let mut b = String::new();
    let n = lines.len();
    for (i, l) in lines.iter().enumerate() {
        let line_width = ansi::string_width(l);
        let mut short = widest - line_width; // difference from the widest line
        short += (width - (short + line_width)).max(0); // difference from the total width, if set
        let mut l = l.clone();
        if short > 0 {
            let styled = |n: i32| {
                let s = spaces(n);
                match style {
                    Some(st) => st.styled(&s),
                    None => s,
                }
            };
            if pos == RIGHT {
                l = styled(short) + &l;
            } else if pos == CENTER {
                let left = short / 2;
                let right = left + short % 2;
                l = styled(left) + &l + &styled(right);
            } else {
                l += &styled(short);
            }
        }
        b.push_str(&l);
        if i < n - 1 {
            b.push('\n');
        }
    }
    b
}

/// align_text_vertical adds empty lines to make a block `height` tall, the
/// room going where `pos` says (TOP, CENTER or BOTTOM; anything else leaves
/// it as it is).
pub(crate) fn align_text_vertical(s: &str, pos: Position, height: i32) -> String {
    let str_height = s.matches('\n').count() as i32 + 1;
    if height < str_height {
        return s.to_string();
    }
    if pos == TOP {
        return s.to_string() + &"\n".repeat((height - str_height) as usize);
    }
    if pos == CENTER {
        let mut top = (height - str_height) / 2;
        let mut bottom = (height - str_height) / 2;
        if str_height + top + bottom > height {
            top -= 1;
        } else if str_height + top + bottom < height {
            bottom += 1;
        }
        return "\n".repeat(top.max(0) as usize) + s + &"\n".repeat(bottom.max(0) as usize);
    }
    if pos == BOTTOM {
        return "\n".repeat((height - str_height) as usize) + s;
    }
    s.to_string()
}
