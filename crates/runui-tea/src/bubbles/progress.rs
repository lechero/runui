//! A progress bar (Bubbles' `progress`): filled cells for what's done, empty
//! ones for the rest, and the percentage after it unless left out.
//!
//! The interface draws it with `view_as`; the spring animation `view` and
//! `set_percent` drive in Bubbles isn't ported (the bar shows the percent
//! set at once), nor are gradients (two or more colors fill the bar with
//! the first).

use crate::ansi;
use crate::style::{Color, Style, color};
use crate::tea::{Cmd, Msg};

/// DEFAULT_FULL_CHAR_HALF_BLOCK is the default filled cell.
pub const DEFAULT_FULL_CHAR_HALF_BLOCK: char = '▌';
/// DEFAULT_FULL_CHAR_FULL_BLOCK is a full block, for filled cells.
pub const DEFAULT_FULL_CHAR_FULL_BLOCK: char = '█';
/// DEFAULT_EMPTY_CHAR_BLOCK is the default empty cell.
pub const DEFAULT_EMPTY_CHAR_BLOCK: char = '░';

const DEFAULT_WIDTH: i32 = 40;
const DEFAULT_FULL_COLOR: Color = color("#7571F9");
const DEFAULT_EMPTY_COLOR: Color = color("#606060");

/// ProgressOption sets something on a new bar.
pub type ProgressOption = Box<dyn FnOnce(&mut Model)>;

/// with_width sets how many cells wide the bar is, the percentage included.
pub fn with_width(w: i32) -> ProgressOption {
    Box::new(move |m| m.set_width(w))
}

/// without_percentage leaves the percentage out.
pub fn without_percentage() -> ProgressOption {
    Box::new(|m| m.show_percentage = false)
}

/// with_colors sets the filled cells' color: none is the default, one is
/// that color.
pub fn with_colors(colors: &[Color]) -> ProgressOption {
    let first = colors.first().copied();
    Box::new(move |m| m.full_color = first.unwrap_or(DEFAULT_FULL_COLOR))
}

/// with_fill_characters sets the filled and empty cells' characters.
pub fn with_fill_characters(full: char, empty: char) -> ProgressOption {
    Box::new(move |m| {
        m.full = full;
        m.empty = empty;
    })
}

/// Model is the progress bar.
#[derive(Clone, Debug)]
pub struct Model {
    width: i32,
    /// Full.
    pub full: char,
    /// Full color.
    pub full_color: Color,
    /// Empty.
    pub empty: char,
    /// Empty color.
    pub empty_color: Color,
    /// Show percentage.
    pub show_percentage: bool,
    /// percent_format formats the percentage, as Go's `" %3.0f%%"`.
    pub percent_format: fn(f64) -> String,
    /// Percentage style.
    pub percentage_style: Style,
    percent: f64,
}

fn default_percent_format(p: f64) -> String {
    format!(" {:>3.0}%", p)
}

/// new is a bar 40 cells wide, half blocks, with the percentage.
pub fn new(opts: Vec<ProgressOption>) -> Model {
    Model::new(opts)
}

impl Model {
    /// new is a bar with the options given.
    pub fn new(opts: Vec<ProgressOption>) -> Model {
        let mut m = Model {
            width: DEFAULT_WIDTH,
            full: DEFAULT_FULL_CHAR_HALF_BLOCK,
            full_color: DEFAULT_FULL_COLOR,
            empty: DEFAULT_EMPTY_CHAR_BLOCK,
            empty_color: DEFAULT_EMPTY_COLOR,
            show_percentage: true,
            percent_format: default_percent_format,
            percentage_style: Style::new(),
            percent: 0.0,
        };
        for o in opts {
            o(&mut m);
        }
        m
    }

    /// update does nothing: there's no animation.
    pub fn update(&mut self, _msg: &Msg) -> Cmd {
        None
    }

    /// percent is the percent set, from 0 to 1.
    pub fn percent(&self) -> f64 {
        self.percent
    }

    /// set_percent sets the percent shown, from 0 to 1.
    pub fn set_percent(&mut self, p: f64) -> Cmd {
        self.percent = p.clamp(0.0, 1.0);
        None
    }

    /// view draws the bar at the percent set.
    pub fn view(&self) -> String {
        self.view_as(self.percent)
    }

    /// view_as draws the bar at `percent` (0 to 1).
    pub fn view_as(&self, percent: f64) -> String {
        let percent_view = self.percentage_view(percent);
        let mut b = self.bar_view(percent, ansi::string_width(&percent_view));
        b.push_str(&percent_view);
        b
    }

    /// set_width sets how many cells wide the bar is.
    pub fn set_width(&mut self, w: i32) {
        self.width = w;
    }

    /// width is how many cells wide the bar is.
    pub fn width(&self) -> i32 {
        self.width
    }

    fn bar_view(&self, percent: f64, text_width: i32) -> String {
        let tw = (self.width - text_width).max(0); // total width
        let fw = ((tw as f64 * percent).round() as i32).clamp(0, tw); // filled width
        let mut b = Style::new().foreground(self.full_color).render(&self.full.to_string().repeat(fw as usize));
        let n = (tw - fw).max(0);
        b.push_str(&Style::new().foreground(self.empty_color).render(&self.empty.to_string().repeat(n as usize)));
        b
    }

    fn percentage_view(&self, percent: f64) -> String {
        if !self.show_percentage {
            return String::new();
        }
        let percent = percent.clamp(0.0, 1.0);
        let s = (self.percent_format)(percent * 100.0);
        self.percentage_style.inline(true).render(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_as_the_interface_draws_it() {
        let m = new(vec![
            with_width(20),
            without_percentage(),
            with_colors(&[color("#4ADE80")]),
            with_fill_characters('█', '░'),
        ]);
        let v = m.view_as(0.25);
        assert_eq!(ansi::strip(&v), "█████░░░░░░░░░░░░░░░");
        assert!(v.starts_with("\x1b[38;2;74;222;128m█████\x1b[m"));
        assert_eq!(ansi::strip(&m.view_as(1.5)), "████████████████████");
    }

    #[test]
    fn percentage_shown() {
        let m = new(vec![with_width(10)]);
        // " %3.0f%%" takes five cells: the bar the other five
        assert_eq!(ansi::strip(&m.view_as(0.5)), "▌▌▌░░  50%");
        assert_eq!(ansi::strip(&m.view_as(0.0)), "░░░░░   0%");
    }
}
