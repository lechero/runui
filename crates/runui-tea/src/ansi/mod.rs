//! Text with ANSI escape sequences in it, measured and cut the way
//! `github.com/charmbracelet/x/ansi` does it: escape sequences are kept but
//! take no room, wide characters (East Asian, emoji) take two cells, and
//! grapheme clusters are never split.
//!
//! Widths and lengths are `i32`, as Go's `int`: a negative length works as
//! it does in Go.

mod color;
pub(crate) mod parser;
pub(crate) mod sgr;
mod tables;
mod width;

pub use color::{Color, color};
pub(crate) use color::{convert16, convert256};
pub use sgr::RESET_STYLE;
pub use width::rune_width;
pub(crate) use width::{first_grapheme_cluster, grapheme_width, uniseg_width};

use parser::{EXECUTE, GROUND, PRINT, Param, Parser, Seq, UTF8, transition, utf8_byte_len};

/// nbsp is a non-breaking space: wrapping never breaks at it.
const NBSP: char = '\u{A0}';

/// is_space is Go's `unicode.IsSpace`.
fn is_space(c: char) -> bool {
    c.is_whitespace()
}

/// strip removes every ANSI escape sequence from `s`.
pub fn strip(s: &str) -> String {
    let b = s.as_bytes();
    let mut buf = Vec::with_capacity(b.len());
    let (mut ri, mut rw) = (0, 0);
    let mut pstate = GROUND;
    for &c in b {
        if pstate == UTF8 {
            buf.push(c);
            ri += 1;
            if ri < rw {
                continue;
            }
            pstate = GROUND;
            ri = 0;
            rw = 0;
            continue;
        }
        let (state, action) = transition(pstate, c);
        match action {
            parser::COLLECT => {
                if state == UTF8 {
                    rw = utf8_byte_len(c);
                    buf.push(c);
                    ri += 1;
                }
            }
            PRINT | EXECUTE => buf.push(c),
            _ => {}
        }
        if pstate != UTF8 {
            pstate = state;
        }
    }
    into_string(buf)
}

/// into_string turns bytes cut at sequence and cluster boundaries back into
/// a string.
fn into_string(buf: Vec<u8>) -> String {
    match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

/// string_width is how many cells `s` takes on a terminal: escape
/// sequences take none, wide characters two, each grapheme cluster as wide
/// as it's drawn. Line breaks aren't counted: it's the sum of every line.
pub fn string_width(s: &str) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let b = s.as_bytes();
    let mut pstate = GROUND;
    let mut width = 0;
    let mut i = 0;
    while i < b.len() {
        let (state, action) = transition(pstate, b[i]);
        if action == PRINT && b.get(i + 1).is_none_or(|&n| n < 0x80) {
            // a printable ASCII character nothing joins: one cell
            width += 1;
            i += 1;
            pstate = GROUND;
            continue;
        }
        if action == PRINT || state == UTF8 {
            let (cluster, w) = first_grapheme_cluster(&s[i..]);
            width += w;
            i += cluster.len().max(1);
            pstate = GROUND;
            continue;
        }
        pstate = state;
        i += 1;
    }
    width
}

/// truncate cuts `s` down to `length` cells, ending it with `tail` when it
/// was longer. Escape sequences are kept, those after the cut too, so the
/// styles they set are still reset.
pub fn truncate(s: &str, length: i32, tail: &str) -> String {
    if string_width(s) <= length {
        return s.to_string();
    }
    let tw = string_width(tail);
    let length = length - tw;
    if length < 0 {
        return String::new();
    }
    let b = s.as_bytes();
    let mut buf: Vec<u8> = Vec::with_capacity(b.len());
    let mut cur_width = 0;
    let mut ignoring = false;
    let mut pstate = GROUND;
    let mut i = 0;
    while i < b.len() {
        let (state, action) = transition(pstate, b[i]);
        if state == UTF8 {
            let (cluster, width) = first_grapheme_cluster(&s[i..]);
            i += cluster.len().max(1);
            cur_width += width;
            if ignoring {
                continue;
            }
            if cur_width > length && !ignoring {
                ignoring = true;
                buf.extend_from_slice(tail.as_bytes());
            }
            if cur_width > length {
                continue;
            }
            buf.extend_from_slice(cluster.as_bytes());
            pstate = GROUND;
            continue;
        }
        match action {
            PRINT => {
                if cur_width >= length && !ignoring {
                    ignoring = true;
                    buf.extend_from_slice(tail.as_bytes());
                }
                if ignoring {
                    i += 1;
                    continue;
                }
                cur_width += 1;
                buf.push(b[i]);
                i += 1;
            }
            EXECUTE => {
                if ignoring {
                    i += 1;
                    continue;
                }
                buf.push(b[i]);
                i += 1;
            }
            _ => {
                buf.push(b[i]);
                i += 1;
            }
        }
        pstate = state;
        if cur_width > length && !ignoring {
            ignoring = true;
            buf.extend_from_slice(tail.as_bytes());
        }
    }
    into_string(buf)
}

/// truncate_left drops the first `n` cells of `s`, starting what's left
/// with `prefix` when anything was dropped. Escape sequences are kept.
pub fn truncate_left(s: &str, n: i32, prefix: &str) -> String {
    if n <= 0 {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut buf: Vec<u8> = Vec::with_capacity(b.len());
    let mut cur_width = 0;
    let mut ignoring = true;
    let mut pstate = GROUND;
    let mut i = 0;
    while i < b.len() {
        if !ignoring {
            buf.extend_from_slice(&b[i..]);
            break;
        }
        let (state, action) = transition(pstate, b[i]);
        if state == UTF8 {
            let (cluster, width) = first_grapheme_cluster(&s[i..]);
            i += cluster.len().max(1);
            cur_width += width;
            if cur_width > n && ignoring {
                ignoring = false;
                buf.extend_from_slice(prefix.as_bytes());
            }
            if cur_width > n {
                buf.extend_from_slice(cluster.as_bytes());
            }
            if ignoring {
                continue;
            }
            pstate = GROUND;
            continue;
        }
        match action {
            PRINT => {
                cur_width += 1;
                if cur_width > n && ignoring {
                    ignoring = false;
                    buf.extend_from_slice(prefix.as_bytes());
                }
                if ignoring {
                    i += 1;
                    continue;
                }
                buf.push(b[i]);
                i += 1;
            }
            EXECUTE => {
                if ignoring {
                    i += 1;
                    continue;
                }
                buf.push(b[i]);
                i += 1;
            }
            _ => {
                buf.push(b[i]);
                i += 1;
            }
        }
        pstate = state;
        if cur_width > n && ignoring {
            ignoring = false;
            buf.extend_from_slice(prefix.as_bytes());
        }
    }
    into_string(buf)
}

/// cut is the cells of `s` from `left` up to (not including) `right`,
/// escape sequences kept.
pub fn cut(s: &str, left: i32, right: i32) -> String {
    if right <= left {
        return String::new();
    }
    if left == 0 {
        return truncate(s, right, "");
    }
    truncate_left(&truncate(s, right, ""), left, "")
}

/// hardwrap breaks `s` into lines of at most `limit` cells, in the middle
/// of words if it must. Spaces starting a line are dropped unless
/// `preserve_space`.
pub fn hardwrap(s: &str, limit: i32, preserve_space: bool) -> String {
    if limit < 1 {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut buf: Vec<u8> = Vec::with_capacity(b.len());
    let mut cur_width = 0;
    let mut force_newline = false;
    let mut pstate = GROUND;
    let mut i = 0;
    while i < b.len() {
        let (state, action) = transition(pstate, b[i]);
        if state == UTF8 {
            let (cluster, width) = first_grapheme_cluster(&s[i..]);
            i += cluster.len().max(1);
            if cur_width + width > limit {
                buf.push(b'\n');
                cur_width = 0;
            }
            if !preserve_space && cur_width == 0 && cluster.len() <= 4 && cluster.chars().next().is_some_and(is_space) {
                pstate = GROUND;
                continue;
            }
            buf.extend_from_slice(cluster.as_bytes());
            cur_width += width;
            pstate = GROUND;
            continue;
        }
        match action {
            PRINT | EXECUTE => 'act: {
                if b[i] == b'\n' {
                    buf.push(b'\n');
                    cur_width = 0;
                    force_newline = false;
                    break 'act;
                }
                if cur_width + 1 > limit {
                    buf.push(b'\n');
                    cur_width = 0;
                    force_newline = true;
                }
                if cur_width == 0 {
                    if !preserve_space && force_newline && is_space(b[i] as char) {
                        break 'act;
                    }
                    force_newline = false;
                }
                buf.push(b[i]);
                if action == PRINT {
                    cur_width += 1;
                }
            }
            _ => buf.push(b[i]),
        }
        if pstate != UTF8 {
            pstate = state;
        }
        i += 1;
    }
    into_string(buf)
}

/// contains_any says whether any character of `chars` is in `s`.
fn contains_any(s: &str, chars: &str) -> bool {
    !chars.is_empty() && s.chars().any(|c| chars.contains(c))
}

#[allow(unused_assignments)]
/// wordwrap breaks `s` into lines of at most `limit` cells at spaces,
/// hyphens and the `breakpoints` given, never inside a word: a word longer
/// than a line stays whole.
pub fn wordwrap(s: &str, limit: i32, breakpoints: &str) -> String {
    if limit < 1 {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut buf: Vec<u8> = Vec::with_capacity(b.len());
    let mut word: Vec<u8> = Vec::new();
    let mut space: Vec<u8> = Vec::new();
    let mut cur_width: i32 = 0;
    let mut word_len: i32 = 0;
    let mut pstate = GROUND;

    macro_rules! add_space {
        () => {{
            cur_width += space.len() as i32;
            buf.extend_from_slice(&space);
            space.clear();
        }};
    }
    macro_rules! add_word {
        () => {{
            if !word.is_empty() {
                add_space!();
                cur_width += word_len;
                buf.extend_from_slice(&word);
                word.clear();
                word_len = 0;
            }
        }};
    }
    macro_rules! add_newline {
        () => {{
            buf.push(b'\n');
            cur_width = 0;
            space.clear();
        }};
    }

    let mut i = 0;
    while i < b.len() {
        let (state, action) = transition(pstate, b[i]);
        if state == UTF8 {
            let (cluster, width) = first_grapheme_cluster(&s[i..]);
            i += cluster.len().max(1);
            let r = cluster.chars().next().unwrap_or('\0');
            if is_space(r) && r != NBSP {
                add_word!();
                space.extend_from_slice(r.encode_utf8(&mut [0; 4]).as_bytes());
            } else if contains_any(cluster, breakpoints) {
                add_space!();
                add_word!();
                buf.extend_from_slice(cluster.as_bytes());
                cur_width += 1;
            } else {
                word.extend_from_slice(cluster.as_bytes());
                word_len += width;
                if cur_width + space.len() as i32 + word_len > limit && word_len < limit {
                    add_newline!();
                }
            }
            pstate = GROUND;
            continue;
        }
        match action {
            PRINT | EXECUTE => {
                let r = b[i] as char;
                if r == '\n' {
                    if word_len == 0 {
                        if cur_width + space.len() as i32 > limit {
                            cur_width = 0;
                        } else {
                            buf.extend_from_slice(&space);
                        }
                        space.clear();
                    }
                    add_word!();
                    add_newline!();
                } else if is_space(r) {
                    add_word!();
                    space.push(b[i]);
                } else if r == '-' || breakpoints.contains(r) {
                    add_space!();
                    add_word!();
                    buf.push(b[i]);
                    cur_width += 1;
                } else {
                    word.push(b[i]);
                    word_len += 1;
                    if cur_width + space.len() as i32 + word_len > limit && word_len < limit {
                        add_newline!();
                    }
                }
            }
            _ => word.push(b[i]),
        }
        if pstate != UTF8 {
            pstate = state;
        }
        i += 1;
    }
    add_word!();
    into_string(buf)
}

#[allow(unused_assignments)]
/// wrap breaks `s` into lines of at most `limit` cells at spaces, hyphens
/// and the `breakpoints` given, and inside a word too long for a line.
pub fn wrap(s: &str, limit: i32, breakpoints: &str) -> String {
    if limit < 1 {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut buf: Vec<u8> = Vec::with_capacity(b.len());
    let mut word: Vec<u8> = Vec::new();
    let mut space: Vec<u8> = Vec::new();
    let mut space_width: i32 = 0;
    let mut cur_width: i32 = 0;
    let mut word_len: i32 = 0;
    let mut pstate = GROUND;

    macro_rules! add_space {
        () => {{
            if !(space_width == 0 && space.is_empty()) {
                cur_width += space_width;
                buf.extend_from_slice(&space);
                space.clear();
                space_width = 0;
            }
        }};
    }
    macro_rules! add_word {
        () => {{
            if !word.is_empty() {
                add_space!();
                cur_width += word_len;
                buf.extend_from_slice(&word);
                word.clear();
                word_len = 0;
            }
        }};
    }
    macro_rules! add_newline {
        () => {{
            buf.push(b'\n');
            cur_width = 0;
            space.clear();
            space_width = 0;
        }};
    }

    let mut i = 0;
    while i < b.len() {
        let (state, action) = transition(pstate, b[i]);
        if state == UTF8 {
            let (cluster, width) = first_grapheme_cluster(&s[i..]);
            i += cluster.len().max(1);
            let r = cluster.chars().next().unwrap_or('\0');
            if is_space(r) && r != NBSP {
                add_word!();
                space.extend_from_slice(r.encode_utf8(&mut [0; 4]).as_bytes());
                space_width += width;
            } else if contains_any(cluster, breakpoints) {
                add_space!();
                if cur_width + word_len + width > limit {
                    word.extend_from_slice(cluster.as_bytes());
                    word_len += width;
                } else {
                    add_word!();
                    buf.extend_from_slice(cluster.as_bytes());
                    cur_width += width;
                }
            } else {
                if word_len + width > limit {
                    // Hardwrap the word if it's too long
                    add_word!();
                }
                word.extend_from_slice(cluster.as_bytes());
                word_len += width;
                if cur_width + word_len + space_width > limit {
                    add_newline!();
                }
                if word_len == limit {
                    // Hardwrap the word if it's too long
                    add_word!();
                }
            }
            pstate = GROUND;
            continue;
        }
        match action {
            PRINT | EXECUTE => {
                let r = b[i] as char;
                if r == '\n' {
                    if word_len == 0 {
                        if cur_width + space_width > limit {
                            cur_width = 0;
                        } else {
                            // preserve whitespaces
                            buf.extend_from_slice(&space);
                        }
                        space.clear();
                        space_width = 0;
                    }
                    add_word!();
                    add_newline!();
                } else if is_space(r) {
                    add_word!();
                    space.push(b[i]);
                    space_width += 1;
                } else if r == '-' || breakpoints.contains(r) {
                    add_space!();
                    if cur_width + word_len >= limit {
                        // We can't fit the breakpoint in the current line,
                        // treat it as part of the word.
                        word.push(b[i]);
                        word_len += 1;
                    } else {
                        add_word!();
                        buf.push(b[i]);
                        cur_width += 1;
                    }
                } else {
                    if cur_width == limit {
                        add_newline!();
                    }
                    word.push(b[i]);
                    word_len += 1;
                    if word_len == limit {
                        // Hardwrap the word if it's too long
                        add_word!();
                    }
                    if cur_width + word_len + space_width > limit {
                        add_newline!();
                    }
                }
            }
            _ => word.push(b[i]),
        }
        if pstate != UTF8 {
            pstate = state;
        }
        i += 1;
    }

    if word_len == 0 {
        if cur_width + space_width > limit {
            cur_width = 0;
        } else {
            // preserve whitespaces
            buf.extend_from_slice(&space);
        }
        space.clear();
        space_width = 0;
    }
    add_word!();
    into_string(buf)
}

/// Token is a piece of text as a terminal reads it: a grapheme cluster, a
/// control character, or an escape sequence.
#[derive(Debug)]
pub(crate) enum Token<'a> {
    /// A grapheme cluster and how many cells it takes.
    Text(&'a str, i32),
    /// A control character outside any sequence (a line feed, a tab…).
    Control(&'a str),
    /// An SGR sequence's parameters.
    Sgr(Vec<Param>),
    /// A hyperlink (OSC 8): its parameters and URL; both empty ends it.
    Link(&'a str, String, String),
    /// Any other escape sequence, kept as it is.
    Other(&'a str),
}

/// tokens splits `s` into what a terminal reads it as.
pub(crate) fn tokens(s: &str) -> Vec<Token<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == 0x1b {
            let end = sequence_end(b, i);
            let seq = &s[i..end];
            out.push(classify(seq));
            i = end;
            continue;
        }
        if c < 0x20 || c == 0x7f {
            out.push(Token::Control(&s[i..i + 1]));
            i += 1;
            continue;
        }
        let (cluster, w) = first_grapheme_cluster(&s[i..]);
        let n = cluster.len().max(1);
        out.push(Token::Text(&s[i..i + n], w));
        i += n;
    }
    out
}

/// sequence_end is where the escape sequence starting at `start` ends.
fn sequence_end(b: &[u8], start: usize) -> usize {
    let mut state = GROUND;
    let mut i = start;
    while i < b.len() {
        let (next, action) = transition(state, b[i]);
        let in_string = matches!(
            state,
            parser::OSC_STRING | parser::DCS_STRING | parser::SOS_STRING | parser::PM_STRING | parser::APC_STRING
        ) || (parser::DCS_ENTRY..=parser::DCS_PARAM).contains(&state);
        if i > start && b[i] == 0x1b {
            if in_string && b.get(i + 1) == Some(&b'\\') {
                return i + 2;
            }
            return i;
        }
        if next == UTF8 && state != GROUND {
            // a character inside a sequence ends it
            return i;
        }
        i += 1;
        if next == GROUND && i > start + 1 {
            let _ = action;
            return i;
        }
        state = next;
    }
    b.len()
}

/// classify says what kind of escape sequence `seq` is.
fn classify(seq: &str) -> Token<'_> {
    let mut p = Parser::default();
    let mut found = None;
    for &c in seq.as_bytes() {
        if let Some(s) = p.advance(c) {
            found.get_or_insert(s);
        }
    }
    match found {
        Some(Seq::Csi { final_byte: b'm', prefix: 0, intermediate: 0, params }) => Token::Sgr(params),
        Some(Seq::Osc { cmd: 8, data }) => {
            let data = String::from_utf8_lossy(&data).into_owned();
            let parts: Vec<&str> = data.split(';').collect();
            if parts.len() == 3 {
                Token::Link(seq, parts[1].to_string(), parts[2].to_string())
            } else {
                Token::Other(seq)
            }
        }
        _ => Token::Other(seq),
    }
}

#[cfg(test)]
mod tests;
