//! The terminal's color profile, detected from the environment the way
//! charmbracelet/colorprofile does, and frames brought down to it: Lip Gloss
//! writes 24-bit colors, and Bubble Tea v2 converts them as it draws, to 256
//! or 16 colors, to none when NO_COLOR is set, and to no escape sequences
//! at all when the output isn't a terminal.

use crate::ansi::{
    self, Color, Token, convert16, convert256,
    parser::{Param, param_at},
};

/// Profile is how many colors the terminal shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Profile {
    /// Not a terminal: no escape sequences at all.
    NoTty,
    /// Text attributes, no colors (NO_COLOR).
    Ascii,
    /// The 16 ANSI colors.
    Ansi,
    /// The 256 indexed colors.
    Ansi256,
    /// 24-bit colors.
    TrueColor,
}

fn parse_bool(v: Option<&str>) -> bool {
    matches!(v, Some("1" | "t" | "T" | "true" | "TRUE" | "True"))
}

/// detect is the profile for an output that is a terminal or not, in an
/// environment given by `env` (as `std::env::var` would answer).
pub(crate) fn detect(is_tty: bool, env: &dyn Fn(&str) -> Option<String>) -> Profile {
    let term = env("TERM");
    let is_tty = is_tty || parse_bool(env("TTY_FORCE").as_deref());
    let is_dumb = term.is_none() || term.as_deref() == Some("dumb");
    let envp = env_color_profile(env);
    let mut p = if !is_tty || is_dumb { Profile::NoTty } else { envp };
    let no_color = parse_bool(env("NO_COLOR").as_deref());
    if no_color && is_tty {
        if p > Profile::Ascii {
            p = Profile::Ascii;
        }
        return p;
    }
    if parse_bool(env("CLICOLOR_FORCE").as_deref()) {
        if p < Profile::Ansi {
            p = Profile::Ansi;
        }
        if envp > p {
            p = envp;
        }
        return p;
    }
    if parse_bool(env("CLICOLOR").as_deref()) && is_tty && !is_dumb && p < Profile::Ansi {
        p = Profile::Ansi;
    }
    if p == Profile::TrueColor || no_color {
        return p;
    }
    if is_tty && !is_dumb && env("TMUX").is_some_and(|t| !t.is_empty()) && p < Profile::Ansi256 {
        p = Profile::Ansi256;
    }
    p
}

fn env_color_profile(env: &dyn Fn(&str) -> Option<String>) -> Profile {
    let term = env("TERM").unwrap_or_default();
    let mut p = if term.is_empty() || term == "dumb" { Profile::NoTty } else { Profile::Ansi };
    for t in ["alacritty", "contour", "foot", "ghostty", "kitty", "rio", "st", "wezterm"] {
        if term.contains(t) {
            return Profile::TrueColor;
        }
    }
    if (term.starts_with("tmux") || term.starts_with("screen")) && p < Profile::Ansi256 {
        p = Profile::Ansi256;
    } else if term.starts_with("xterm") && p < Profile::Ansi {
        p = Profile::Ansi;
    }
    if env("WT_SESSION").is_some_and(|v| !v.is_empty()) {
        return Profile::TrueColor;
    }
    if parse_bool(env("GOOGLE_CLOUD_SHELL").as_deref()) {
        return Profile::TrueColor;
    }
    let colorterm = env("COLORTERM").unwrap_or_default().to_lowercase();
    if matches!(colorterm.as_str(), "truecolor" | "24bit" | "yes" | "true")
        && !term.starts_with("screen")
        && !term.starts_with("tmux")
    {
        return Profile::TrueColor;
    }
    if term.ends_with("256color") && p < Profile::Ansi256 {
        p = Profile::Ansi256;
    }
    if term.ends_with("direct") {
        return Profile::TrueColor;
    }
    p
}

/// convert_color brings a color down to the profile, none when it shows
/// no colors.
fn convert_color(c: Color, p: Profile) -> Option<Color> {
    match p {
        Profile::TrueColor => Some(c),
        Profile::Ansi256 => match c {
            Color::Basic(_) | Color::Indexed(_) => Some(c),
            other => Some(Color::Indexed(convert256(other))),
        },
        Profile::Ansi => Some(Color::Basic(convert16(c))),
        Profile::Ascii | Profile::NoTty => None,
    }
}

/// rewrite_sgr writes an SGR sequence's parameters again, colors brought
/// down to the profile.
fn rewrite_sgr(params: &[Param], p: Profile) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < params.len() {
        let (v, has_more, _) = param_at(params, i, 0);
        match v {
            38 | 48 | 58 => {
                let (n, c) = read_color(&params[i..]);
                if n == 0 {
                    i += 1;
                    continue;
                }
                if let Some(c) = c.and_then(|c| convert_color(c, p)) {
                    let s = match v {
                        38 => crate::ansi::sgr::Sgr::default().foreground(Some(c)),
                        48 => crate::ansi::sgr::Sgr::default().background(Some(c)),
                        _ => crate::ansi::sgr::Sgr::default().underline_color(Some(c)),
                    }
                    .string();
                    out.push(s.trim_start_matches("\x1b[").trim_end_matches('m').to_string());
                }
                i += n;
                continue;
            }
            30..=37 | 40..=47 | 90..=97 | 100..=107 => {
                if !matches!(p, Profile::Ascii | Profile::NoTty) {
                    out.push(v.to_string());
                }
            }
            4 if has_more => {
                let (next, _, ok) = param_at(params, i + 1, 0);
                if ok {
                    out.push(format!("4:{next}"));
                    i += 2;
                    continue;
                }
                out.push("4".into());
            }
            _ => out.push(v.to_string()),
        }
        i += 1;
    }
    if out.is_empty() {
        return "\x1b[m".into();
    }
    format!("\x1b[{}m", out.join(";"))
}

/// read_color reads the color after a 38, 48 or 58: how many parameters
/// it took, and the color.
fn read_color(params: &[Param]) -> (usize, Option<Color>) {
    crate::ansi::sgr::read_style_color(params)
}

/// convert_frame brings a frame's escape sequences down to the profile.
pub(crate) fn convert_frame(s: &str, p: Profile) -> String {
    match p {
        Profile::TrueColor => s.to_string(),
        Profile::NoTty => ansi::strip(s),
        _ => {
            let mut out = String::with_capacity(s.len());
            for t in ansi::tokens(s) {
                match t {
                    Token::Text(t, _) | Token::Control(t) | Token::Other(t) | Token::Link(t, _, _) => out.push_str(t),
                    Token::Sgr(params) => out.push_str(&rewrite_sgr(&params, p)),
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn detects_like_colorprofile() {
        assert_eq!(detect(false, &env_of(&[("TERM", "xterm-256color")])), Profile::NoTty);
        assert_eq!(detect(true, &env_of(&[("TERM", "xterm-256color")])), Profile::Ansi256);
        assert_eq!(
            detect(true, &env_of(&[("TERM", "xterm-256color"), ("COLORTERM", "truecolor")])),
            Profile::TrueColor
        );
        assert_eq!(detect(true, &env_of(&[("TERM", "xterm-ghostty")])), Profile::TrueColor);
        assert_eq!(detect(true, &env_of(&[("TERM", "xterm-256color"), ("NO_COLOR", "1")])), Profile::Ascii);
        assert_eq!(detect(true, &env_of(&[("TERM", "dumb")])), Profile::NoTty);
        assert_eq!(detect(true, &env_of(&[("TERM", "xterm")])), Profile::Ansi);
        assert_eq!(detect(true, &env_of(&[])), Profile::NoTty);
    }

    #[test]
    fn converts_frames() {
        let s = "\x1b[1;38;2;125;211;252mhi\x1b[m";
        assert_eq!(convert_frame(s, Profile::TrueColor), s);
        assert_eq!(convert_frame(s, Profile::Ansi256), "\x1b[1;38;5;117mhi\x1b[m");
        assert_eq!(convert_frame(s, Profile::Ascii), "\x1b[1mhi\x1b[m");
        assert_eq!(convert_frame(s, Profile::NoTty), "hi");
        assert_eq!(convert_frame("\x1b[38;5;240mx\x1b[m", Profile::Ansi), "\x1b[90mx\x1b[m");
    }
}
