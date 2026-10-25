//! A scrolling window onto text taller (or wider) than the space for it
//! (Bubbles' `viewport`): lines cut at its width or wrapped onto more, a
//! line offset scrolled with keys or the wheel.

use super::key::{self, Binding};
use crate::ansi;
use crate::style::Style;
use crate::tea::{Cmd, KeyPressMsg, MOD_SHIFT, MouseButton, MouseWheelMsg, Msg};

const DEFAULT_HORIZONTAL_STEP: i32 = 6;

/// KeyMap is the keys the viewport scrolls with. Its default (Go's zero
/// value) has none; [`default_key_map`] is Bubbles'.
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    /// Page down.
    pub page_down: Binding,
    /// Page up.
    pub page_up: Binding,
    /// Half page up.
    pub half_page_up: Binding,
    /// Half page down.
    pub half_page_down: Binding,
    /// Down.
    pub down: Binding,
    /// Up.
    pub up: Binding,
    /// Left.
    pub left: Binding,
    /// Right.
    pub right: Binding,
}

/// default_key_map is Bubbles' keys: pager keys and the arrows.
pub fn default_key_map() -> KeyMap {
    KeyMap {
        page_down: Binding::new().with_keys(&["pgdown", "space", "f"]).with_help("f/pgdn", "page down"),
        page_up: Binding::new().with_keys(&["pgup", "b"]).with_help("b/pgup", "page up"),
        half_page_up: Binding::new().with_keys(&["u", "ctrl+u"]).with_help("u", "½ page up"),
        half_page_down: Binding::new().with_keys(&["d", "ctrl+d"]).with_help("d", "½ page down"),
        up: Binding::new().with_keys(&["up", "k"]).with_help("↑/k", "up"),
        down: Binding::new().with_keys(&["down", "j"]).with_help("↓/j", "down"),
        left: Binding::new().with_keys(&["left", "h"]).with_help("←/h", "move left"),
        right: Binding::new().with_keys(&["right", "l"]).with_help("→/l", "move right"),
    }
}

/// GutterContext is what a gutter function is told about a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GutterContext {
    /// index is the line's number in the content, from 0.
    pub index: i32,
    /// total_lines is how many lines the content has.
    pub total_lines: i32,
    /// soft says whether this is a wrapped line's continuation.
    pub soft: bool,
}

/// GutterFunc draws what goes left of a line.
pub type GutterFunc = fn(GutterContext) -> String;

/// no_gutter draws nothing: the default gutter.
pub fn no_gutter(_: GutterContext) -> String {
    String::new()
}

/// Model is the viewport.
#[derive(Clone, Debug)]
pub struct Model {
    width: i32,
    height: i32,
    /// Key map.
    pub key_map: KeyMap,
    /// soft_wrap wraps long lines onto more lines instead of cutting them.
    pub soft_wrap: bool,
    /// fill_height pads the view with empty lines to its height.
    pub fill_height: bool,
    /// Mouse wheel enabled.
    pub mouse_wheel_enabled: bool,
    /// Mouse wheel delta.
    pub mouse_wheel_delta: i32,
    y_offset: i32,
    x_offset: i32,
    horizontal_step: i32,
    /// y_position is where the viewport is on the screen.
    pub y_position: i32,
    /// Style.
    pub style: Style,
    /// Left gutter func.
    pub left_gutter_func: Option<GutterFunc>,
    /// style_line_func styles each line by its number.
    pub style_line_func: Option<fn(i32) -> Style>,
    lines: Vec<String>,
    longest_line_width: i32,
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

/// new is Go's `viewport.New()`.
pub fn new() -> Model {
    Model::new()
}

fn clamp(v: i32, low: i32, high: i32) -> i32 {
    let (low, high) = if high < low { (high, low) } else { (low, high) };
    v.max(low).min(high)
}

impl Model {
    /// new is an empty viewport with Bubbles' keys and the wheel on.
    pub fn new() -> Model {
        Model {
            width: 0,
            height: 0,
            key_map: default_key_map(),
            soft_wrap: false,
            fill_height: false,
            mouse_wheel_enabled: true,
            mouse_wheel_delta: 3,
            y_offset: 0,
            x_offset: 0,
            horizontal_step: DEFAULT_HORIZONTAL_STEP,
            y_position: 0,
            style: Style::new(),
            left_gutter_func: Some(no_gutter),
            style_line_func: None,
            lines: Vec::new(),
            longest_line_width: 0,
        }
    }

    /// with_size is a new viewport `w` by `h`.
    pub fn with_size(w: i32, h: i32) -> Model {
        let mut m = Model::new();
        m.width = w;
        m.height = h;
        m
    }

    /// height is how many lines the viewport shows.
    pub fn height(&self) -> i32 {
        self.height
    }

    /// set_height sets how many lines the viewport shows.
    pub fn set_height(&mut self, h: i32) {
        self.height = h;
    }

    /// width is how many cells wide the viewport is.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// set_width sets how many cells wide the viewport is.
    pub fn set_width(&mut self, w: i32) {
        self.width = w;
    }

    /// at_top says whether it's scrolled to the top.
    pub fn at_top(&self) -> bool {
        self.y_offset() <= 0
    }

    /// at_bottom says whether it's scrolled to the bottom.
    pub fn at_bottom(&self) -> bool {
        self.y_offset() >= self.max_y_offset()
    }

    /// past_bottom says whether it's scrolled past the bottom.
    pub fn past_bottom(&self) -> bool {
        self.y_offset() > self.max_y_offset()
    }

    /// scroll_percent is how far down it's scrolled, from 0 to 1.
    pub fn scroll_percent(&self) -> f64 {
        let (total, _, _) = self.calculate_line(0);
        if self.height() >= total {
            return 1.0;
        }
        let v = self.y_offset() as f64 / (total as f64 - self.height() as f64);
        v.clamp(0.0, 1.0)
    }

    /// horizontal_scroll_percent is how far right it's scrolled.
    pub fn horizontal_scroll_percent(&self) -> f64 {
        if self.x_offset >= self.longest_line_width - self.width() {
            return 1.0;
        }
        let v = self.x_offset as f64 / (self.longest_line_width as f64 - self.width() as f64);
        v.clamp(0.0, 1.0)
    }

    /// set_content replaces the text shown.
    pub fn set_content(&mut self, s: &str) {
        self.set_content_lines(s.split('\n').map(str::to_string).collect());
    }

    /// set_content_lines replaces the text shown, line by line.
    pub fn set_content_lines(&mut self, lines: Vec<String>) {
        self.lines = lines;
        if self.lines.len() == 1 && ansi::string_width(&self.lines[0]) == 0 {
            self.lines.clear();
        } else {
            // if there's a line break in a line, split it
            let mut i = self.lines.len();
            while i > 0 {
                i -= 1;
                if !self.lines[i].contains(['\r', '\n']) {
                    continue;
                }
                let l = self.lines[i].replace("\r\n", "\n");
                let sub: Vec<String> = l.split('\n').map(str::to_string).collect();
                if sub.len() > 1 {
                    self.lines.splice(i..=i, sub);
                } else {
                    self.lines[i] = l;
                }
            }
        }
        self.longest_line_width = self.lines.iter().map(|l| ansi::string_width(l)).max().unwrap_or(0);
        if self.y_offset() > self.max_y_offset() {
            self.goto_bottom();
        }
    }

    /// get_content is the text shown.
    pub fn get_content(&self) -> String {
        self.lines.join("\n")
    }

    /// calculate_line is how many lines the content takes, which line a
    /// view starting `yoffset` down starts at, and how far into it.
    fn calculate_line(&self, yoffset: i32) -> (i32, i32, i32) {
        if !self.soft_wrap {
            let total = self.lines.len() as i32;
            return (total, yoffset.min(total), 0);
        }
        let max_width = self.max_width() as f64;
        let (mut total, mut ridx, mut voffset) = (0, 0, 0);
        for (i, line) in self.lines.iter().enumerate() {
            let w = ansi::string_width(line) as f64;
            let h = if max_width > 0.0 { ((w / max_width).ceil() as i32).max(1) } else { 1 };
            if yoffset >= total && yoffset < total + h {
                ridx = i as i32;
                voffset = yoffset - total;
            }
            total += h;
        }
        if yoffset >= total {
            ridx = self.lines.len() as i32;
            voffset = 0;
        }
        (total, ridx, voffset)
    }

    fn max_y_offset(&self) -> i32 {
        let (total, _, _) = self.calculate_line(0);
        (total - self.height() + self.style.get_vertical_frame_size()).max(0)
    }

    fn max_x_offset(&self) -> i32 {
        (self.longest_line_width - self.width()).max(0)
    }

    fn max_width(&self) -> i32 {
        let gutter = self.left_gutter_func.map(|f| ansi::string_width(&f(GutterContext::default()))).unwrap_or(0);
        (self.width() - self.style.get_horizontal_frame_size() - gutter).max(0)
    }

    fn max_height(&self) -> i32 {
        (self.height() - self.style.get_vertical_frame_size()).max(0)
    }

    fn visible_lines(&self) -> Vec<String> {
        let max_height = self.max_height();
        let max_width = self.max_width();
        if max_height == 0 || max_width == 0 {
            return Vec::new();
        }
        let (total, ridx, voffset) = self.calculate_line(self.y_offset());
        let mut lines: Vec<String> = Vec::new();
        if total > 0 {
            let bottom = clamp(ridx + max_height, ridx, self.lines.len() as i32);
            lines = self.lines[ridx as usize..bottom as usize].to_vec();
            if let Some(f) = self.style_line_func {
                for (i, l) in lines.iter_mut().enumerate() {
                    *l = f(i as i32 + ridx).render(l);
                }
            }
        }
        while self.fill_height && (lines.len() as i32) < max_height {
            lines.push(String::new());
        }
        // if longest line fit within width, no need to do anything else.
        if (self.x_offset == 0 && self.longest_line_width <= max_width) || max_width == 0 {
            return self.setup_gutter(lines, total, ridx);
        }
        if self.soft_wrap {
            return self.soft_wrap_lines(lines, max_width, max_height, total, ridx, voffset);
        }
        // Cut the lines to the viewport width.
        for l in lines.iter_mut() {
            *l = ansi::cut(l, self.x_offset, self.x_offset + max_width);
        }
        self.setup_gutter(lines, total, ridx)
    }

    fn soft_wrap_lines(
        &self,
        lines: Vec<String>,
        max_width: i32,
        max_height: i32,
        total: i32,
        ridx: i32,
        voffset: i32,
    ) -> Vec<String> {
        let mut wrapped: Vec<String> = Vec::with_capacity(max_height as usize);
        for (i, line) in lines.iter().enumerate() {
            let line_width = ansi::string_width(line);
            // If the line is less than or equal to the max width it can be
            // added as is.
            if line_width <= max_width {
                let mut l = line.clone();
                if let Some(f) = self.left_gutter_func {
                    l = f(GutterContext { index: i as i32 + ridx, total_lines: total, soft: false }) + &l;
                }
                wrapped.push(l);
                continue;
            }
            let mut idx = 0;
            while line_width > idx {
                let mut t = ansi::cut(line, idx, max_width + idx);
                if let Some(f) = self.left_gutter_func {
                    t = f(GutterContext { index: i as i32 + ridx, total_lines: total, soft: idx > 0 }) + &t;
                }
                wrapped.push(t);
                idx += max_width;
            }
        }
        let from = (voffset.max(0) as usize).min(wrapped.len());
        let to = ((voffset + max_height).max(0) as usize).min(wrapped.len());
        wrapped[from..to.max(from)].to_vec()
    }

    fn setup_gutter(&self, mut lines: Vec<String>, total: i32, ridx: i32) -> Vec<String> {
        let Some(f) = self.left_gutter_func else { return lines };
        for (i, l) in lines.iter_mut().enumerate() {
            *l = f(GutterContext { index: i as i32 + ridx, total_lines: total, soft: false }) + l;
        }
        lines
    }

    /// set_y_offset scrolls to `n` lines down.
    pub fn set_y_offset(&mut self, n: i32) {
        self.y_offset = clamp(n, 0, self.max_y_offset());
    }

    /// y_offset is how many lines down it's scrolled.
    pub fn y_offset(&self) -> i32 {
        self.y_offset
    }

    /// ensure_visible scrolls so the line, and the columns, show.
    pub fn ensure_visible(&mut self, line: i32, colstart: i32, colend: i32) {
        let max_width = self.max_width();
        if colend <= max_width {
            self.set_x_offset(0);
        } else {
            // put one step to the left, feels more natural
            self.set_x_offset(colstart - self.horizontal_step);
        }
        if line < self.y_offset() || line >= self.y_offset() + self.max_height() {
            self.set_y_offset(line);
        }
    }

    /// page_down scrolls a page down.
    pub fn page_down(&mut self) {
        if self.at_bottom() {
            return;
        }
        self.scroll_down(self.height());
    }

    /// page_up scrolls a page up.
    pub fn page_up(&mut self) {
        if self.at_top() {
            return;
        }
        self.scroll_up(self.height());
    }

    /// half_page_down scrolls half a page down.
    pub fn half_page_down(&mut self) {
        if self.at_bottom() {
            return;
        }
        self.scroll_down(self.height() / 2);
    }

    /// half_page_up scrolls half a page up.
    pub fn half_page_up(&mut self) {
        if self.at_top() {
            return;
        }
        self.scroll_up(self.height() / 2);
    }

    /// scroll_down scrolls `n` lines down.
    pub fn scroll_down(&mut self, n: i32) {
        if self.at_bottom() || n == 0 || self.lines.is_empty() {
            return;
        }
        // Make sure the number of lines by which we're going to scroll
        // isn't greater than the number of lines we are from the bottom.
        self.set_y_offset(self.y_offset() + n);
    }

    /// scroll_up scrolls `n` lines up.
    pub fn scroll_up(&mut self, n: i32) {
        if self.at_top() || n == 0 || self.lines.is_empty() {
            return;
        }
        // Make sure the number of lines by which we're going to scroll
        // isn't greater than the number of lines we are from the top.
        self.set_y_offset(self.y_offset() - n);
    }

    /// set_horizontal_step sets how many cells left and right scroll.
    pub fn set_horizontal_step(&mut self, n: i32) {
        self.horizontal_step = n.max(0);
    }

    /// x_offset is how many cells right it's scrolled.
    pub fn x_offset(&self) -> i32 {
        self.x_offset
    }

    /// set_x_offset scrolls to `n` cells right (not when wrapping).
    pub fn set_x_offset(&mut self, n: i32) {
        if self.soft_wrap {
            return;
        }
        self.x_offset = clamp(n, 0, self.max_x_offset());
    }

    /// scroll_left scrolls `n` cells left.
    pub fn scroll_left(&mut self, n: i32) {
        self.set_x_offset(self.x_offset - n);
    }

    /// scroll_right scrolls `n` cells right.
    pub fn scroll_right(&mut self, n: i32) {
        self.set_x_offset(self.x_offset + n);
    }

    /// total_line_count is how many lines the content takes (wrapped
    /// lines counted as they show).
    pub fn total_line_count(&self) -> i32 {
        self.calculate_line(0).0
    }

    /// visible_line_count is how many lines show.
    pub fn visible_line_count(&self) -> i32 {
        self.visible_lines().len() as i32
    }

    /// goto_top scrolls to the top, and is the lines then shown.
    pub fn goto_top(&mut self) -> Vec<String> {
        if self.at_top() {
            return Vec::new();
        }
        self.set_y_offset(0);
        self.visible_lines()
    }

    /// goto_bottom scrolls to the bottom, and is the lines then shown.
    pub fn goto_bottom(&mut self) -> Vec<String> {
        self.set_y_offset(self.max_y_offset());
        self.visible_lines()
    }

    /// update takes a message: its keys and the wheel scroll.
    pub fn update(&mut self, msg: &Msg) -> Cmd {
        if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            let km = &self.key_map;
            if key::matches(k, &[&km.page_down]) {
                self.page_down();
            } else if key::matches(k, &[&km.page_up]) {
                self.page_up();
            } else if key::matches(k, &[&km.half_page_down]) {
                self.half_page_down();
            } else if key::matches(k, &[&km.half_page_up]) {
                self.half_page_up();
            } else if key::matches(k, &[&km.down]) {
                self.scroll_down(1);
            } else if key::matches(k, &[&km.up]) {
                self.scroll_up(1);
            } else if key::matches(k, &[&km.left]) {
                self.scroll_left(self.horizontal_step);
            } else if key::matches(k, &[&km.right]) {
                self.scroll_right(self.horizontal_step);
            }
        } else if let Some(w) = msg.downcast_ref::<MouseWheelMsg>() {
            if !self.mouse_wheel_enabled {
                return None;
            }
            match w.button {
                MouseButton::WheelDown => {
                    if w.mods.contains(MOD_SHIFT) {
                        self.scroll_right(self.horizontal_step);
                    } else {
                        self.scroll_down(self.mouse_wheel_delta);
                    }
                }
                MouseButton::WheelUp => {
                    if w.mods.contains(MOD_SHIFT) {
                        self.scroll_left(self.horizontal_step);
                    } else {
                        self.scroll_up(self.mouse_wheel_delta);
                    }
                }
                MouseButton::WheelLeft => self.scroll_left(self.horizontal_step),
                MouseButton::WheelRight => self.scroll_right(self.horizontal_step),
                _ => {}
            }
        }
        None
    }

    /// view draws the lines that show, padded to the viewport's size.
    pub fn view(&self) -> String {
        let (mut w, mut h) = (self.width(), self.height());
        let sw = self.style.get_width();
        if sw != 0 {
            w = w.min(sw);
        }
        let sh = self.style.get_height();
        if sh != 0 {
            h = h.min(sh);
        }
        if w == 0 || h == 0 {
            return String::new();
        }
        let content_width = w - self.style.get_horizontal_frame_size();
        let content_height = h - self.style.get_vertical_frame_size();
        let contents =
            Style::new().width(content_width).height(content_height).render(&self.visible_lines().join("\n"));
        self.style.unset_width().unset_height().render(&contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tea::{self, KeyPressMsg};

    fn content(n: i32) -> String {
        (1..=n).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn new_has_defaults() {
        let m = Model::with_size(10, 10);
        assert_eq!((m.width(), m.height()), (10, 10));
        assert!(m.mouse_wheel_enabled);
        assert_eq!(m.mouse_wheel_delta, 3);
        assert!(m.key_map.page_down.enabled());
    }

    #[test]
    fn scrolls_and_clamps() {
        let mut m = Model::with_size(10, 5);
        m.set_content(&content(20));
        assert_eq!(m.total_line_count(), 20);
        assert!(m.at_top());
        m.scroll_down(3);
        assert_eq!(m.y_offset(), 3);
        m.page_down();
        assert_eq!(m.y_offset(), 8);
        m.goto_bottom();
        assert_eq!(m.y_offset(), 15);
        assert!(m.at_bottom());
        m.scroll_down(1);
        assert_eq!(m.y_offset(), 15);
        m.half_page_up();
        assert_eq!(m.y_offset(), 13);
        m.goto_top();
        assert_eq!(m.y_offset(), 0);
        m.scroll_up(1);
        assert_eq!(m.y_offset(), 0);
        m.set_y_offset(100);
        assert_eq!(m.y_offset(), 15);
    }

    #[test]
    fn keys_and_wheel_scroll() {
        let mut m = Model::with_size(10, 5);
        m.set_content(&content(20));
        m.update(&tea::msg(KeyPressMsg::parse("j")));
        m.update(&tea::msg(KeyPressMsg::parse("pgdown")));
        assert_eq!(m.y_offset(), 6);
        m.update(&tea::msg(MouseWheelMsg { button: MouseButton::WheelUp, ..Default::default() }));
        assert_eq!(m.y_offset(), 3);
        let mut quiet = Model::with_size(10, 5);
        quiet.key_map = KeyMap::default();
        quiet.set_content(&content(20));
        quiet.update(&tea::msg(KeyPressMsg::parse("j")));
        assert_eq!(quiet.y_offset(), 0);
    }

    #[test]
    fn view_pads_to_size() {
        let mut m = Model::with_size(8, 3);
        m.set_content("a\nb");
        assert_eq!(m.view(), "a       \nb       \n        ");
    }

    #[test]
    fn soft_wrap_wraps_long_lines() {
        let mut m = Model::with_size(4, 5);
        m.soft_wrap = true;
        m.set_content("abcdefghij\nk");
        assert_eq!(m.total_line_count(), 4);
        assert_eq!(m.view(), "abcd\nefgh\nij  \nk   \n    ");
        let mut short = Model::with_size(4, 2);
        short.soft_wrap = true;
        short.set_content("abcdefghij\nk");
        short.set_y_offset(1);
        assert_eq!(short.view(), "efgh\nij  ");
    }

    #[test]
    fn horizontal_scrolling_cuts() {
        let mut m = Model::with_size(4, 2);
        m.set_content("abcdefghij\nxy");
        assert_eq!(m.view(), "abcd\nxy  ");
        m.scroll_right(3);
        assert_eq!(m.view(), "defg\n    ");
        m.set_x_offset(100);
        assert_eq!(m.x_offset(), 6);
    }
}
