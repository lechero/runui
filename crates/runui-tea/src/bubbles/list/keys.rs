//! The list's keys.

use crate::bubbles::key::Binding;

/// KeyMap is the keys the list answers to. Its default (Go's zero value)
/// has none; [`default_key_map`] is Bubbles'.
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    // Keybindings used when browsing the list.
    /// Cursor up.
    pub cursor_up: Binding,
    /// Cursor down.
    pub cursor_down: Binding,
    /// Next page.
    pub next_page: Binding,
    /// Prev page.
    pub prev_page: Binding,
    /// Go to start.
    pub go_to_start: Binding,
    /// Go to end.
    pub go_to_end: Binding,
    /// Filter.
    pub filter: Binding,
    /// Clear filter.
    pub clear_filter: Binding,
    // Keybindings used when setting a filter.
    /// Cancel while filtering.
    pub cancel_while_filtering: Binding,
    /// Accept while filtering.
    pub accept_while_filtering: Binding,
    // Help toggle keybindings.
    /// Show full help.
    pub show_full_help: Binding,
    /// Close full help.
    pub close_full_help: Binding,
    // The quit keybinding. This won't be caught when filtering.
    /// Quit.
    pub quit: Binding,
    // The quit-no-matter-what keybinding. This will be caught when
    // filtering.
    /// Force quit.
    pub force_quit: Binding,
}

/// default_key_map is Bubbles' keys.
pub fn default_key_map() -> KeyMap {
    let b = |keys: &[&str], h: &str, d: &str| Binding::new().with_keys(keys).with_help(h, d);
    KeyMap {
        cursor_up: b(&["up", "k"], "↑/k", "up"),
        cursor_down: b(&["down", "j"], "↓/j", "down"),
        prev_page: b(&["left", "h", "pgup", "b", "u"], "←/h/pgup", "prev page"),
        next_page: b(&["right", "l", "pgdown", "f", "d"], "→/l/pgdn", "next page"),
        go_to_start: b(&["home", "g"], "g/home", "go to start"),
        go_to_end: b(&["end", "G"], "G/end", "go to end"),
        filter: b(&["/"], "/", "filter"),
        clear_filter: b(&["esc"], "esc", "clear filter"),
        cancel_while_filtering: b(&["esc"], "esc", "cancel"),
        accept_while_filtering: b(
            &["enter", "tab", "shift+tab", "ctrl+k", "up", "ctrl+j", "down"],
            "enter",
            "apply filter",
        ),
        show_full_help: b(&["?"], "?", "more"),
        close_full_help: b(&["?"], "?", "close help"),
        quit: b(&["v"], "v", "select"),
        force_quit: Binding::new().with_keys(&["ctrl+c"]),
    }
}
