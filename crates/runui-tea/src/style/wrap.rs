//! Lip Gloss's `Wrap`: text wrapped as x/ansi's `Wrap` does, with the style
//! and hyperlink in force at each line break ended before it and set again
//! after it, so every line stands on its own.

use crate::ansi::{
    self, RESET_STYLE,
    parser::{Parser, Seq},
    sgr::{Pen, read_style},
};

/// set_hyperlink is the sequence that starts a hyperlink.
pub(crate) fn set_hyperlink(url: &str, params: &str) -> String {
    format!("\x1b]8;{params};{url}\x07")
}

/// reset_hyperlink is the sequence that ends a hyperlink.
pub(crate) const RESET_HYPERLINK: &str = "\x1b]8;;\x07";

/// read_link reads a hyperlink's parameters and URL out of an OSC 8's data.
pub(crate) fn read_link(data: &[u8], link: &mut (String, String)) {
    let parts: Vec<&[u8]> = data.split(|&b| b == b';').collect();
    if parts.len() != 3 {
        return;
    }
    link.0 = String::from_utf8_lossy(parts[1]).into_owned();
    link.1 = String::from_utf8_lossy(parts[2]).into_owned();
}

/// wrap wraps `s` to `width` cells, carrying styles across the breaks.
pub fn wrap(s: &str, width: i32, breakpoints: &str) -> String {
    let s = ansi::wrap(s, width, breakpoints);
    let mut out: Vec<u8> = Vec::with_capacity(s.len() + 16);
    let mut p = Parser::default();
    let mut pen = Pen::default();
    // the hyperlink in force: its parameters and URL
    let mut link = (String::new(), String::new());
    for &b in s.as_bytes() {
        match p.advance(b) {
            Some(Seq::Csi { final_byte: b'm', prefix: 0, intermediate: 0, params }) => read_style(&params, &mut pen),
            Some(Seq::Osc { cmd: 8, data }) => read_link(&data, &mut link),
            _ => {}
        }
        let link_set = !link.0.is_empty() || !link.1.is_empty();
        if b == b'\n' {
            if !pen.is_zero() {
                out.extend_from_slice(RESET_STYLE.as_bytes());
            }
            if link_set {
                out.extend_from_slice(RESET_HYPERLINK.as_bytes());
            }
        }
        out.push(b);
        if b == b'\n' {
            if link_set {
                out.extend_from_slice(set_hyperlink(&link.1, &link.0).as_bytes());
            }
            if !pen.is_zero() {
                out.extend_from_slice(pen.string().as_bytes());
            }
        }
    }
    if !pen.is_zero() {
        out.extend_from_slice(RESET_STYLE.as_bytes());
    }
    if !link.0.is_empty() || !link.1.is_empty() {
        out.extend_from_slice(RESET_HYPERLINK.as_bytes());
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
