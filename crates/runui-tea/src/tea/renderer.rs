//! Drawing views on the terminal: each frame cut to the window (lines as
//! wide as the window at most, as many as it has rows), its colors brought
//! down to the terminal's profile, and only the lines that changed since the
//! last frame written again. The terminal's modes (alternate screen, mouse,
//! bracketed paste, focus reports, window title) follow what the view asks.

use std::io::{self, Write};

use super::profile::{Profile, convert_frame};
use super::{MouseMode, View};
use crate::ansi;

const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
const EXIT_ALT_SCREEN: &str = "\x1b[?1049l";
const CLEAR_SCREEN: &str = "\x1b[2J";
const ERASE_LINE_RIGHT: &str = "\x1b[K";
const ERASE_ENTIRE_LINE: &str = "\x1b[2K";
const BRACKETED_PASTE_ON: &str = "\x1b[?2004h";
const BRACKETED_PASTE_OFF: &str = "\x1b[?2004l";
const FOCUS_ON: &str = "\x1b[?1004h";
const FOCUS_OFF: &str = "\x1b[?1004l";
const MOUSE_CELL_MOTION_ON: &str = "\x1b[?1002h\x1b[?1006h";
const MOUSE_ALL_MOTION_ON: &str = "\x1b[?1003h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1002l\x1b[?1003l\x1b[?1006l";
const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";
/// Kitty's keyboard protocol, "disambiguate escape codes" (what Bubble Tea
/// v2 turns on): ctrl+enter, shift+enter and the like reported apart. Each
/// screen keeps its own stack of these.
const KEYBOARD_PUSH: &str = "\x1b[>1u";
const KEYBOARD_POP: &str = "\x1b[<1u";

/// Renderer draws frames on a terminal (or anything written to).
pub(crate) struct Renderer<W: Write> {
    out: W,
    profile: Profile,
    width: i32,
    height: i32,
    /// the lines on the screen, as written
    last: Vec<String>,
    /// how many lines the last inline frame took
    lines_rendered: usize,
    /// the next frame is drawn whole
    repaint: bool,
    alt_screen: bool,
    mouse_mode: MouseMode,
    title: String,
    bracketed_paste: bool,
    report_focus: bool,
    started: bool,
    /// the keyboard enhancements are pushed on the screen shown
    keyboard: bool,
}

impl<W: Write> Renderer<W> {
    pub(crate) fn new(out: W, profile: Profile, width: i32, height: i32) -> Self {
        Renderer {
            out,
            profile,
            width,
            height,
            last: Vec::new(),
            lines_rendered: 0,
            repaint: true,
            alt_screen: false,
            mouse_mode: MouseMode::None,
            title: String::new(),
            bracketed_paste: false,
            report_focus: false,
            started: false,
            keyboard: false,
        }
    }

    /// out is what the renderer writes to: the program writes what isn't
    /// a frame there too (the clipboard).
    pub(crate) fn out(&mut self) -> &mut W {
        &mut self.out
    }

    /// start hides the cursor and turns bracketed paste on, as Bubble Tea
    /// does by default.
    pub(crate) fn start(&mut self) -> io::Result<()> {
        self.started = true;
        self.bracketed_paste = true;
        let s = format!("{HIDE_CURSOR}{BRACKETED_PASTE_ON}");
        self.out.write_all(s.as_bytes())?;
        self.out.flush()
    }

    /// resize takes the window's new size: the next frame is drawn whole.
    pub(crate) fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.repaint = true;
    }

    /// frame is the view's lines as they'll be on the screen.
    fn frame(&self, content: &str) -> Vec<String> {
        let content = content.replace("\r\n", "\n");
        let mut lines: Vec<&str> = content.split('\n').collect();
        let h = self.height.max(0) as usize;
        if h > 0 && lines.len() > h {
            if self.alt_screen {
                lines.truncate(h);
            } else {
                lines.drain(..lines.len() - h);
            }
        }
        lines
            .into_iter()
            .map(|l| {
                let l = if self.width > 0 && ansi::string_width(l) > self.width {
                    ansi::truncate(l, self.width, "")
                } else {
                    l.to_string()
                };
                convert_frame(&l, self.profile)
            })
            .collect()
    }

    /// modes switches the terminal's modes to what the view asks for.
    fn modes(&mut self, v: &View, buf: &mut String) {
        if v.alt_screen != self.alt_screen {
            if self.keyboard {
                // the screen left keeps its own
                buf.push_str(KEYBOARD_POP);
                self.keyboard = false;
            }
            if v.alt_screen {
                buf.push_str(ENTER_ALT_SCREEN);
                buf.push_str(CLEAR_SCREEN);
            } else {
                buf.push_str(EXIT_ALT_SCREEN);
            }
            // an inline frame stays where it was
            self.lines_rendered = 0;
            self.alt_screen = v.alt_screen;
            self.last.clear();
            self.repaint = true;
        }
        if self.started && !self.keyboard {
            buf.push_str(KEYBOARD_PUSH);
            self.keyboard = true;
        }
        if v.mouse_mode != self.mouse_mode {
            buf.push_str(MOUSE_OFF);
            match v.mouse_mode {
                MouseMode::CellMotion => buf.push_str(MOUSE_CELL_MOTION_ON),
                MouseMode::AllMotion => buf.push_str(MOUSE_ALL_MOTION_ON),
                MouseMode::None => {}
            }
            self.mouse_mode = v.mouse_mode;
        }
        let paste = !v.disable_bracketed_paste_mode;
        if paste != self.bracketed_paste {
            buf.push_str(if paste { BRACKETED_PASTE_ON } else { BRACKETED_PASTE_OFF });
            self.bracketed_paste = paste;
        }
        if v.report_focus != self.report_focus {
            buf.push_str(if v.report_focus { FOCUS_ON } else { FOCUS_OFF });
            self.report_focus = v.report_focus;
        }
        if v.window_title != self.title {
            buf.push_str(&format!("\x1b]2;{}\x07", v.window_title));
            self.title = v.window_title.clone();
        }
    }

    /// render draws a view, writing only what changed.
    pub(crate) fn render(&mut self, v: &View) -> io::Result<()> {
        let mut buf = String::new();
        self.modes(v, &mut buf);
        let lines = self.frame(&v.content);
        if !self.repaint && lines == self.last {
            // nothing changed
            if !buf.is_empty() {
                self.out.write_all(buf.as_bytes())?;
                self.out.flush()?;
            }
            return Ok(());
        }
        let mut body = String::new();
        if self.alt_screen {
            self.render_alt(&lines, &mut body);
        } else {
            self.render_inline(&lines, &mut body);
        }
        self.repaint = false;
        self.last = lines;
        if !body.is_empty() {
            buf.push_str(SYNC_BEGIN);
            buf.push_str(&body);
            buf.push_str(SYNC_END);
        }
        if buf.is_empty() {
            return Ok(());
        }
        self.out.write_all(buf.as_bytes())?;
        self.out.flush()
    }

    /// write_line writes a line where the cursor is, then clears the rest of
    /// the row.
    fn write_line(&self, line: &str, buf: &mut String) {
        buf.push_str(line);
        if line.contains('\x1b') {
            buf.push_str(ansi::RESET_STYLE);
        }
        if ansi::string_width(line) < self.width || self.width <= 0 {
            buf.push_str(ERASE_LINE_RIGHT);
        }
    }

    fn render_alt(&mut self, lines: &[String], buf: &mut String) {
        if self.repaint {
            buf.push_str(CLEAR_SCREEN);
            self.last.clear();
        }
        for (y, line) in lines.iter().enumerate() {
            if !self.repaint && self.last.get(y) == Some(line) {
                continue;
            }
            if self.repaint && line.is_empty() {
                continue;
            }
            buf.push_str(&format!("\x1b[{};1H", y + 1));
            self.write_line(line, buf);
        }
        for y in lines.len()..self.last.len() {
            buf.push_str(&format!("\x1b[{};1H{}", y + 1, ERASE_ENTIRE_LINE));
        }
    }

    fn render_inline(&mut self, lines: &[String], buf: &mut String) {
        // back to the frame's first line; the cursor rests at the start of
        // its last one
        if self.lines_rendered > 1 {
            buf.push_str(&format!("\x1b[{}A", self.lines_rendered - 1));
        }
        if self.lines_rendered > 0 {
            buf.push('\r');
        }
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                if i < self.lines_rendered {
                    buf.push_str("\x1b[B");
                } else {
                    buf.push_str("\r\n");
                }
            }
            if !self.repaint && i < self.lines_rendered && self.last.get(i) == Some(line) {
                continue;
            }
            buf.push('\r');
            self.write_line(line, buf);
        }
        // clear what's left of a taller frame
        let extra = self.lines_rendered.saturating_sub(lines.len());
        for _ in 0..extra {
            buf.push_str("\x1b[B\r");
            buf.push_str(ERASE_ENTIRE_LINE);
        }
        if extra > 0 {
            buf.push_str(&format!("\x1b[{extra}A"));
        }
        buf.push('\r');
        self.lines_rendered = lines.len();
    }

    /// stop puts the terminal back: modes off, the cursor shown, the
    /// alternate screen left (or the cursor below an inline frame).
    pub(crate) fn stop(&mut self) -> io::Result<()> {
        if !self.started {
            return Ok(());
        }
        self.started = false;
        let mut buf = String::new();
        if self.keyboard {
            buf.push_str(KEYBOARD_POP);
            self.keyboard = false;
        }
        if self.mouse_mode != MouseMode::None {
            buf.push_str(MOUSE_OFF);
        }
        if self.bracketed_paste {
            buf.push_str(BRACKETED_PASTE_OFF);
        }
        if self.report_focus {
            buf.push_str(FOCUS_OFF);
        }
        if self.alt_screen {
            buf.push_str(EXIT_ALT_SCREEN);
        } else if self.lines_rendered > 0 {
            buf.push_str("\r\n");
        }
        buf.push_str(SHOW_CURSOR);
        self.out.write_all(buf.as_bytes())?;
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(r: &mut Renderer<Vec<u8>>) -> String {
        let s = String::from_utf8(r.out().clone()).unwrap();
        r.out().clear();
        s
    }

    fn alt(content: &str) -> View {
        View { content: content.into(), alt_screen: true, ..Default::default() }
    }

    #[test]
    fn rewrites_only_changed_lines() {
        let mut r = Renderer::new(Vec::new(), Profile::TrueColor, 10, 3);
        r.start().unwrap();
        written(&mut r);
        r.render(&alt("one\ntwo\nthree")).unwrap();
        let first = written(&mut r);
        assert!(first.contains(ENTER_ALT_SCREEN) && first.contains("one") && first.contains("three"));
        r.render(&alt("one\nTWO\nthree")).unwrap();
        let second = written(&mut r);
        assert!(second.contains("\x1b[2;1HTWO"), "{second:?}");
        assert!(!second.contains("one") && !second.contains("three"), "{second:?}");
        r.render(&alt("one\nTWO\nthree")).unwrap();
        assert_eq!(written(&mut r), "");
    }

    #[test]
    fn clips_to_the_window() {
        let mut r = Renderer::new(Vec::new(), Profile::TrueColor, 4, 2);
        r.render(&alt("abcdef\nb\nc")).unwrap();
        let s = written(&mut r);
        assert!(s.contains("\x1b[1;1Habcd") && !s.contains("abcde"), "{s:?}");
        assert!(s.contains("\x1b[2;1Hb") && !s.contains("\x1b[3;1H"), "{s:?}");
    }

    #[test]
    fn modes_follow_the_view() {
        let mut r = Renderer::new(Vec::new(), Profile::TrueColor, 10, 3);
        r.start().unwrap();
        written(&mut r);
        let v = View { mouse_mode: MouseMode::CellMotion, window_title: "aem-cli".into(), ..alt("x") };
        r.render(&v).unwrap();
        let s = written(&mut r);
        assert!(s.contains(MOUSE_CELL_MOTION_ON) && s.contains("\x1b]2;aem-cli\x07"), "{s:?}");
        // the keyboard enhancements pushed on the alternate screen
        assert!(s.find(ENTER_ALT_SCREEN) < s.find(KEYBOARD_PUSH), "{s:?}");
        r.stop().unwrap();
        let s = written(&mut r);
        assert!(s.contains(MOUSE_OFF) && s.contains(EXIT_ALT_SCREEN) && s.contains(SHOW_CURSOR), "{s:?}");
        assert!(s.find(KEYBOARD_POP) < s.find(EXIT_ALT_SCREEN), "{s:?}");
    }

    #[test]
    fn inline_frames_move_up_to_redraw() {
        let mut r = Renderer::new(Vec::new(), Profile::TrueColor, 10, 5);
        r.render(&View::new("a\nb")).unwrap();
        written(&mut r);
        r.render(&View::new("a\nc")).unwrap();
        let s = written(&mut r);
        assert!(s.starts_with(&format!("{SYNC_BEGIN}\x1b[1A\r")), "{s:?}");
        assert!(s.contains("\x1b[B\rc"), "{s:?}");
    }

    #[test]
    fn colors_follow_the_profile() {
        let mut r = Renderer::new(Vec::new(), Profile::Ascii, 10, 3);
        r.render(&alt("\x1b[1;38;2;1;2;3mx\x1b[m")).unwrap();
        let s = written(&mut r);
        assert!(s.contains("\x1b[1mx\x1b[m") && !s.contains("38;2"), "{s:?}");
    }
}
