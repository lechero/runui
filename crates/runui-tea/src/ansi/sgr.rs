//! Select Graphic Rendition: the escape sequences that style text. `Sgr`
//! builds one attribute by attribute as x/ansi's `Style` does (what Lip Gloss
//! renders with); `Pen` is a style read back out of text, as ultraviolet's
//! `Style` keeps it, written out again whole or as the change from another.

use super::color::{Color, bg_sgr, fg_sgr, ul_sgr};
use super::parser::{Param, param_at};

/// RESET_STYLE resets every attribute.
pub const RESET_STYLE: &str = "\x1b[m";

/// Underline styles, as x/ansi numbers them.
pub(crate) const UNDERLINE_NONE: u8 = 0;
pub(crate) const UNDERLINE_SINGLE: u8 = 1;
pub(crate) const UNDERLINE_DOUBLE: u8 = 2;
pub(crate) const UNDERLINE_CURLY: u8 = 3;
pub(crate) const UNDERLINE_DOTTED: u8 = 4;
pub(crate) const UNDERLINE_DASHED: u8 = 5;

/// Sgr is a list of SGR attributes, written as one sequence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Sgr(Vec<String>);

impl Sgr {
    fn push(mut self, a: impl Into<String>) -> Self {
        self.0.push(a.into());
        self
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn bold(self) -> Self {
        self.push("1")
    }
    pub(crate) fn faint(self) -> Self {
        self.push("2")
    }
    pub(crate) fn italic(self, v: bool) -> Self {
        self.push(if v { "3" } else { "23" })
    }
    pub(crate) fn underline(self, v: bool) -> Self {
        self.push(if v { "4" } else { "24" })
    }
    pub(crate) fn underline_style(self, u: u8) -> Self {
        match u {
            UNDERLINE_NONE => self.underline(false),
            UNDERLINE_SINGLE => self.underline(true),
            UNDERLINE_DOUBLE => self.push("4:2"),
            UNDERLINE_CURLY => self.push("4:3"),
            UNDERLINE_DOTTED => self.push("4:4"),
            UNDERLINE_DASHED => self.push("4:5"),
            _ => self,
        }
    }
    pub(crate) fn blink(self, v: bool) -> Self {
        self.push(if v { "5" } else { "25" })
    }
    pub(crate) fn rapid_blink(self, v: bool) -> Self {
        self.push(if v { "6" } else { "25" })
    }
    pub(crate) fn reverse(self, v: bool) -> Self {
        self.push(if v { "7" } else { "27" })
    }
    pub(crate) fn conceal(self, v: bool) -> Self {
        self.push(if v { "8" } else { "28" })
    }
    pub(crate) fn strikethrough(self, v: bool) -> Self {
        self.push(if v { "9" } else { "29" })
    }
    pub(crate) fn normal(self) -> Self {
        self.push("22")
    }
    pub(crate) fn foreground(self, c: Option<Color>) -> Self {
        self.push(fg_sgr(c))
    }
    pub(crate) fn background(self, c: Option<Color>) -> Self {
        self.push(bg_sgr(c))
    }
    pub(crate) fn underline_color(self, c: Option<Color>) -> Self {
        self.push(ul_sgr(c))
    }

    /// string is the sequence: a reset when there's nothing in it.
    pub(crate) fn string(&self) -> String {
        if self.0.is_empty() {
            return RESET_STYLE.to_string();
        }
        format!("\x1b[{}m", self.0.join(";"))
    }

    /// styled wraps `s` in the sequence and a reset; with nothing in it, `s`
    /// as it is.
    pub(crate) fn styled(&self, s: &str) -> String {
        if self.0.is_empty() {
            return s.to_string();
        }
        let mut out = self.string();
        out.push_str(s);
        out.push_str(RESET_STYLE);
        out
    }
}

/// Text attributes a pen keeps, as ultraviolet numbers them.
pub(crate) const ATTR_BOLD: u8 = 1;
pub(crate) const ATTR_FAINT: u8 = 1 << 1;
pub(crate) const ATTR_ITALIC: u8 = 1 << 2;
pub(crate) const ATTR_BLINK: u8 = 1 << 3;
pub(crate) const ATTR_RAPID_BLINK: u8 = 1 << 4;
pub(crate) const ATTR_REVERSE: u8 = 1 << 5;
pub(crate) const ATTR_CONCEAL: u8 = 1 << 6;
pub(crate) const ATTR_STRIKETHROUGH: u8 = 1 << 7;

/// Pen is the style text is drawn with, as read from its SGR sequences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pen {
    pub(crate) fg: Option<Color>,
    pub(crate) bg: Option<Color>,
    pub(crate) underline_color: Option<Color>,
    pub(crate) underline: u8,
    pub(crate) attrs: u8,
}

fn color_equal(a: Option<Color>, b: Option<Color>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.rgb() == b.rgb(),
        _ => false,
    }
}

impl Pen {
    pub(crate) fn is_zero(&self) -> bool {
        *self == Pen::default()
    }

    /// equal compares colors by their RGB values, as ultraviolet does.
    pub(crate) fn equal(&self, o: &Pen) -> bool {
        self.attrs == o.attrs
            && self.underline == o.underline
            && color_equal(self.fg, o.fg)
            && color_equal(self.bg, o.bg)
            && color_equal(self.underline_color, o.underline_color)
    }

    /// string is the sequence that sets the whole style.
    pub(crate) fn string(&self) -> String {
        if self.is_zero() {
            return RESET_STYLE.to_string();
        }
        let mut b = Sgr::default();
        if self.attrs & ATTR_BOLD != 0 {
            b = b.bold();
        }
        if self.attrs & ATTR_FAINT != 0 {
            b = b.faint();
        }
        if self.attrs & ATTR_ITALIC != 0 {
            b = b.italic(true);
        }
        if self.attrs & ATTR_BLINK != 0 {
            b = b.blink(true);
        }
        if self.attrs & ATTR_RAPID_BLINK != 0 {
            b = b.rapid_blink(true);
        }
        if self.attrs & ATTR_REVERSE != 0 {
            b = b.reverse(true);
        }
        if self.attrs & ATTR_CONCEAL != 0 {
            b = b.conceal(true);
        }
        if self.attrs & ATTR_STRIKETHROUGH != 0 {
            b = b.strikethrough(true);
        }
        if self.underline != UNDERLINE_NONE {
            b = b.underline_style(self.underline);
        }
        if self.fg.is_some() {
            b = b.foreground(self.fg);
        }
        if self.bg.is_some() {
            b = b.background(self.bg);
        }
        if self.underline_color.is_some() {
            b = b.underline_color(self.underline_color);
        }
        b.string()
    }

    /// diff is the sequence that goes from the style `from` to this one, as
    /// ultraviolet's `StyleDiff` writes it.
    pub(crate) fn diff(&self, from: &Pen) -> String {
        let to = self;
        if from.equal(to) {
            return String::new();
        }
        if to.is_zero() {
            return RESET_STYLE.to_string();
        }
        let mut b = Sgr::default();
        if !color_equal(from.fg, to.fg) {
            b = b.foreground(to.fg);
        }
        if !color_equal(from.bg, to.bg) {
            b = b.background(to.bg);
        }
        if !color_equal(from.underline_color, to.underline_color) {
            b = b.underline_color(to.underline_color);
        }
        let has = |p: &Pen, a: u8| p.attrs & a != 0;
        let (from_bold, to_bold) = (has(from, ATTR_BOLD), has(to, ATTR_BOLD));
        let (from_faint, to_faint) = (has(from, ATTR_FAINT), has(to, ATTR_FAINT));
        let (from_italic, to_italic) = (has(from, ATTR_ITALIC), has(to, ATTR_ITALIC));
        let (from_ul, to_ul) = (from.underline != UNDERLINE_NONE, to.underline != UNDERLINE_NONE);
        let (from_blink, to_blink) = (has(from, ATTR_BLINK), has(to, ATTR_BLINK));
        let (from_rblink, to_rblink) = (has(from, ATTR_RAPID_BLINK), has(to, ATTR_RAPID_BLINK));
        let (from_rev, to_rev) = (has(from, ATTR_REVERSE), has(to, ATTR_REVERSE));
        let (from_conceal, to_conceal) = (has(from, ATTR_CONCEAL), has(to, ATTR_CONCEAL));
        let (from_strike, to_strike) = (has(from, ATTR_STRIKETHROUGH), has(to, ATTR_STRIKETHROUGH));

        let mut bold_changed = from_bold != to_bold;
        let mut faint_changed = from_faint != to_faint;
        if (bold_changed || faint_changed) && (from_bold && !to_bold || from_faint && !to_faint) {
            b = b.normal();
            bold_changed = true;
            faint_changed = true;
        }
        let italic_changed = from_italic != to_italic;
        if italic_changed && !to_italic {
            b = b.italic(false);
        }
        let ul_changed = from_ul != to_ul || from.underline != to.underline;
        if ul_changed && !to_ul {
            b = b.underline(false);
        }
        let mut blink_changed = from_blink != to_blink;
        let mut rblink_changed = from_rblink != to_rblink;
        if (blink_changed || rblink_changed) && (from_blink && !to_blink || from_rblink && !to_rblink) {
            b = b.blink(false);
            blink_changed = true;
            rblink_changed = true;
        }
        let rev_changed = from_rev != to_rev;
        if rev_changed && !to_rev {
            b = b.reverse(false);
        }
        let conceal_changed = from_conceal != to_conceal;
        if conceal_changed && !to_conceal {
            b = b.conceal(false);
        }
        let strike_changed = from_strike != to_strike;
        if strike_changed && !to_strike {
            b = b.strikethrough(false);
        }
        if bold_changed && to_bold {
            b = b.bold();
        }
        if faint_changed && to_faint {
            b = b.faint();
        }
        if italic_changed && to_italic {
            b = b.italic(true);
        }
        if ul_changed && to_ul && to.underline == UNDERLINE_SINGLE {
            b = b.underline(true);
        }
        if blink_changed && to_blink {
            b = b.blink(true);
        }
        if rblink_changed && to_rblink {
            b = b.rapid_blink(true);
        }
        if rev_changed && to_rev {
            b = b.reverse(true);
        }
        if conceal_changed && to_conceal {
            b = b.conceal(true);
        }
        if strike_changed && to_strike {
            b = b.strikethrough(true);
        }
        if ul_changed && to_ul && to.underline > UNDERLINE_SINGLE {
            b = b.underline_style(to.underline);
        }
        b.string()
    }
}

/// read_style applies an SGR sequence's parameters to a pen, as
/// ultraviolet's `ReadStyle` does.
pub(crate) fn read_style(params: &[Param], pen: &mut Pen) {
    if params.is_empty() {
        *pen = Pen::default();
        return;
    }
    let mut i = 0;
    while i < params.len() {
        let (param, has_more, _) = param_at(params, i, 0);
        match param {
            0 => *pen = Pen::default(),
            1 => pen.attrs |= ATTR_BOLD,
            2 => pen.attrs |= ATTR_FAINT,
            3 => pen.attrs |= ATTR_ITALIC,
            4 => {
                let (next, _, ok) = param_at(params, i + 1, 0);
                if has_more && ok {
                    if (0..=5).contains(&next) {
                        i += 1;
                        pen.underline = next as u8;
                    }
                } else {
                    pen.underline = UNDERLINE_SINGLE;
                }
            }
            5 => pen.attrs |= ATTR_BLINK,
            6 => pen.attrs |= ATTR_RAPID_BLINK,
            7 => pen.attrs |= ATTR_REVERSE,
            8 => pen.attrs |= ATTR_CONCEAL,
            9 => pen.attrs |= ATTR_STRIKETHROUGH,
            22 => pen.attrs &= !(ATTR_BOLD | ATTR_FAINT),
            23 => pen.attrs &= !ATTR_ITALIC,
            24 => pen.underline = UNDERLINE_NONE,
            25 => pen.attrs &= !(ATTR_BLINK | ATTR_RAPID_BLINK),
            27 => pen.attrs &= !ATTR_REVERSE,
            28 => pen.attrs &= !ATTR_CONCEAL,
            29 => pen.attrs &= !ATTR_STRIKETHROUGH,
            30..=37 => pen.fg = Some(Color::Basic((param - 30) as u8)),
            38 => {
                if let (n, Some(c)) = read_style_color(&params[i..]) {
                    pen.fg = Some(c);
                    i += n - 1;
                }
            }
            39 => pen.fg = None,
            40..=47 => pen.bg = Some(Color::Basic((param - 40) as u8)),
            48 => {
                if let (n, Some(c)) = read_style_color(&params[i..]) {
                    pen.bg = Some(c);
                    i += n - 1;
                }
            }
            49 => pen.bg = None,
            58 => {
                if let (n, Some(c)) = read_style_color(&params[i..]) {
                    pen.underline_color = Some(c);
                    i += n - 1;
                }
            }
            59 => pen.underline_color = None,
            90..=97 => pen.fg = Some(Color::Basic((param - 90 + 8) as u8)),
            100..=107 => pen.bg = Some(Color::Basic((param - 100 + 8) as u8)),
            _ => {}
        }
        i += 1;
    }
}

/// read_style_color reads a color after a 38, 48 or 58: how many parameters
/// it took and the color, as x/ansi's `ReadStyleColor` (RGB and indexed
/// colors, colon or semicolon separated).
pub(crate) fn read_style_color(params: &[Param]) -> (usize, Option<Color>) {
    if params.len() < 2 {
        return (0, None);
    }
    let s = params[0];
    let p = params[1];
    let more = |i: usize| params.get(i).is_some_and(|p| p.has_more());
    let val = |i: usize| params.get(i).map(|p| p.param(0)).unwrap_or(0);
    match p.param(0) {
        0 | 1 => (2, None),
        2 => {
            if params.len() < 5 {
                return (0, None);
            }
            let (n, rgb) =
                if s.has_more() && p.has_more() && params.len() > 5 && more(2) && more(3) && more(4) && !more(5) {
                    (6, (val(3), val(4), val(5)))
                } else if (s.has_more() && p.has_more() && more(2) && more(3) && !more(4))
                    || (!s.has_more() && !p.has_more() && !more(2) && !more(3) && !more(4))
                {
                    (5, (val(2), val(3), val(4)))
                } else {
                    return (0, None);
                };
            (n, Some(Color::Rgb(rgb.0 as u8, rgb.1 as u8, rgb.2 as u8)))
        }
        5 => {
            if params.len() < 3 {
                return (0, None);
            }
            let ok = (s.has_more() && p.has_more() && !more(2)) || (!s.has_more() && !p.has_more() && !more(2));
            if !ok {
                return (0, None);
            }
            (3, Some(Color::Indexed(val(2) as u8)))
        }
        _ => (0, None),
    }
}
