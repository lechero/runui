//! Help: the keys a screen answers to, on one line (short) or in columns
//! (full), as Bubbles' `help` draws them.

use super::key::Binding;
use crate::style::{self, Style, TOP, color};
use crate::tea::{Cmd, Msg};

/// KeyMap is what help is drawn from: the bindings for the short line, and
/// the columns for the full help.
pub trait KeyMap {
    /// short_help is the bindings on the one line, in order.
    fn short_help(&self) -> Vec<Binding>;
    /// full_help is the bindings in columns.
    fn full_help(&self) -> Vec<Vec<Binding>>;
}

/// Styles is how the help is styled.
#[derive(Clone, Copy, Debug)]
pub struct Styles {
    /// Ellipsis.
    pub ellipsis: Style,
    /// Short key.
    pub short_key: Style,
    /// Short desc.
    pub short_desc: Style,
    /// Short separator.
    pub short_separator: Style,
    /// Full key.
    pub full_key: Style,
    /// Full desc.
    pub full_desc: Style,
    /// Full separator.
    pub full_separator: Style,
}

/// default_styles is the help's styles for a dark background, or a light
/// one.
pub fn default_styles(is_dark: bool) -> Styles {
    let ld = |light: &str, dark: &str| if is_dark { color(dark) } else { color(light) };
    let key_style = Style::new().foreground(ld("#909090", "#626262"));
    let desc_style = Style::new().foreground(ld("#B2B2B2", "#4A4A4A"));
    let sep_style = Style::new().foreground(ld("#DADADA", "#3C3C3C"));
    Styles {
        short_key: key_style,
        short_desc: desc_style,
        short_separator: sep_style,
        ellipsis: sep_style,
        full_key: key_style,
        full_desc: desc_style,
        full_separator: sep_style,
    }
}

/// default_dark_styles is the styles for a dark background.
pub fn default_dark_styles() -> Styles {
    default_styles(true)
}

/// default_light_styles is the styles for a light background.
pub fn default_light_styles() -> Styles {
    default_styles(false)
}

/// Model is the help.
#[derive(Clone, Debug)]
pub struct Model {
    /// show_all draws the full help instead of the short line.
    pub show_all: bool,
    /// Short separator.
    pub short_separator: String,
    /// Full separator.
    pub full_separator: String,
    /// ellipsis ends a short line cut for want of room.
    pub ellipsis: String,
    /// Styles.
    pub styles: Styles,
    width: i32,
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

/// new is Go's `help.New()`.
pub fn new() -> Model {
    Model::new()
}

impl Model {
    /// new is the help as Bubbles makes it: " • " between keys, an
    /// ellipsis when the line is cut, dark styles.
    pub fn new() -> Model {
        Model {
            show_all: false,
            short_separator: " • ".into(),
            full_separator: "    ".into(),
            ellipsis: "…".into(),
            styles: default_dark_styles(),
            width: 0,
        }
    }

    /// update does nothing: the help has no state messages change.
    pub fn update(&mut self, _msg: &Msg) -> Cmd {
        None
    }

    /// view draws the help, short or full.
    pub fn view(&self, k: &dyn KeyMap) -> String {
        if self.show_all {
            return self.full_help_view(&k.full_help());
        }
        self.short_help_view(&k.short_help())
    }

    /// set_width sets the most cells the help may take across.
    pub fn set_width(&mut self, w: i32) {
        self.width = w;
    }

    /// width is the most cells the help may take across.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// short_help_view draws bindings on one line, as many as fit.
    pub fn short_help_view(&self, bindings: &[Binding]) -> String {
        if bindings.is_empty() {
            return String::new();
        }
        let mut b = String::new();
        let mut total_width = 0;
        let separator = self.styles.short_separator.inline(true).render(&self.short_separator);
        for kb in bindings {
            if !kb.enabled() {
                continue;
            }
            let sep = if total_width > 0 { separator.as_str() } else { "" };
            let s = format!(
                "{sep}{} {}",
                self.styles.short_key.inline(true).render(&kb.help().key),
                self.styles.short_desc.inline(true).render(&kb.help().desc)
            );
            let w = style::width(&s);
            if let Some(tail) = self.should_add_item(total_width, w) {
                b.push_str(&tail);
                break;
            }
            total_width += w;
            b.push_str(&s);
        }
        b
    }

    /// full_help_view draws bindings in columns, as many as fit.
    pub fn full_help_view(&self, groups: &[Vec<Binding>]) -> String {
        if groups.is_empty() {
            return String::new();
        }
        let mut out: Vec<String> = Vec::new();
        let mut total_width = 0;
        let separator = self.styles.full_separator.inline(true).render(&self.full_separator);
        for group in groups {
            if !group.iter().any(Binding::enabled) {
                continue;
            }
            let sep = if total_width > 0 { separator.as_str() } else { "" };
            let mut keys = Vec::new();
            let mut descriptions = Vec::new();
            for kb in group {
                if !kb.enabled() {
                    continue;
                }
                keys.push(kb.help().key.as_str());
                descriptions.push(kb.help().desc.as_str());
            }
            let col = style::join_horizontal(
                TOP,
                &[
                    sep,
                    &self.styles.full_key.render(&keys.join("\n")),
                    " ",
                    &self.styles.full_desc.render(&descriptions.join("\n")),
                ],
            );
            let w = style::width(&col);
            if let Some(tail) = self.should_add_item(total_width, w) {
                out.push(tail);
                break;
            }
            total_width += w;
            out.push(col);
        }
        let refs: Vec<&str> = out.iter().map(String::as_str).collect();
        style::join_horizontal(TOP, &refs)
    }

    /// should_add_item is the tail to end with when an item doesn't fit and
    /// the ellipsis does; none when the item is added (it fits, or not even
    /// the ellipsis would, as Bubbles has it).
    fn should_add_item(&self, total_width: i32, width: i32) -> Option<String> {
        if self.width > 0 && total_width + width > self.width {
            let tail = format!(" {}", self.styles.ellipsis.inline(true).render(&self.ellipsis));
            if total_width + style::width(&tail) < self.width {
                return Some(tail);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi;

    struct Keys(Vec<Vec<Binding>>);

    impl KeyMap for Keys {
        fn short_help(&self) -> Vec<Binding> {
            self.0.iter().flatten().cloned().collect()
        }
        fn full_help(&self) -> Vec<Vec<Binding>> {
            self.0.clone()
        }
    }

    #[test]
    fn full_help() {
        let mut m = Model::new();
        m.full_separator = " | ".into();
        let k = |h: &str, d: &str| Binding::new().with_keys(&["x"]).with_help(h, d);
        let kb = vec![
            vec![k("enter", "continue")],
            vec![k("esc", "back"), k("?", "help")],
            vec![k("H", "home"), k("ctrl+c", "quit"), k("ctrl+l", "log")],
        ];
        let goldens = [
            (20, "enter continue …"),
            (30, "enter continue | esc back …\n                 ?   help  "),
            (
                40,
                "enter continue | esc back | H      home\n                 ?   help   ctrl+c quit\n                            ctrl+l log ",
            ),
        ];
        for (w, want) in goldens {
            m.set_width(w);
            assert_eq!(ansi::strip(&m.full_help_view(&kb)), want, "width {w}");
        }
        // view draws the short line unless show_all
        assert_eq!(m.view(&Keys(kb.clone())), m.short_help_view(&Keys(kb.clone()).short_help()));
        m.show_all = true;
        assert_eq!(m.view(&Keys(kb.clone())), m.full_help_view(&kb));
    }

    #[test]
    fn short_help_joins_with_bullets() {
        let a = Binding::new().with_keys(&["a"]).with_help("a", "add");
        let d = Binding::new().with_keys(&["d"]).with_help("d", "delete");
        let off = Binding::new().with_keys(&["x"]).with_help("x", "off").with_disabled();
        let m = Model::new();
        assert_eq!(ansi::strip(&m.short_help_view(&[a.clone(), off, d.clone()])), "a add • d delete");
        let mut narrow = Model::new();
        narrow.set_width(12);
        assert_eq!(ansi::strip(&narrow.short_help_view(&[a, d])), "a add …");
    }
}
