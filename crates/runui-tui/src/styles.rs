//! The interface's colours, styles and keys, ported from styles.go.

use runui_tea::bubbles::help;
use runui_tea::bubbles::key::Binding;
use runui_tea::style::{self, Color, Style};

// The colours are aem-cli's, so the two look like one family.
pub(crate) const C_ACCENT: Color = style::color("#7DD3FC");
pub(crate) const C_GOOD: Color = style::color("#4ADE80");
pub(crate) const C_WARN: Color = style::color("#FBBF24");
pub(crate) const C_BAD: Color = style::color("#F87171");
pub(crate) const C_DIM: Color = style::color("#8B93A1");
pub(crate) const C_TEXT: Color = style::color("#E5E7EB");
/// a box's frame without the focus
pub(crate) const C_LINE: Color = style::color("#4B5563");

pub(crate) const S_TITLE: Style = Style::new().bold(true).foreground(C_TEXT);
pub(crate) const S_ACCENT: Style = Style::new().foreground(C_ACCENT);
pub(crate) const S_DIM: Style = Style::new().foreground(C_DIM);
pub(crate) const S_GOOD: Style = Style::new().foreground(C_GOOD);
pub(crate) const S_WARN: Style = Style::new().foreground(C_WARN);
pub(crate) const S_BAD: Style = Style::new().foreground(C_BAD);
pub(crate) const S_LINE: Style = Style::new().foreground(C_LINE);
pub(crate) const S_HEADING: Style = Style::new().bold(true).foreground(C_ACCENT);
pub(crate) const S_BOX: Style =
    Style::new().border(style::rounded_border(), &[]).border_foreground(&[C_WARN]).padding(&[1, 2]);
pub(crate) const S_ROW_ACTIVE: Style = Style::new().foreground(style::color("#0B1220")).background(C_ACCENT);
pub(crate) const S_ROW_IDLE: Style = Style::new().foreground(C_TEXT).background(style::color("#374151"));

/// KeyMap is every key, named: the frame's are lazygit's, as aem-cli's are.
#[derive(Clone, Debug)]
pub(crate) struct KeyMap {
    pub(crate) next_panel: Binding,
    pub(crate) prev_panel: Binding,
    pub(crate) panels: Binding,
    pub(crate) tabs: Binding,
    pub(crate) focus_pane: Binding,
    pub(crate) scroll_pane: Binding,
    pub(crate) full: Binding,
    pub(crate) filter: Binding,
    pub(crate) help: Binding,
    pub(crate) quit: Binding,
    pub(crate) up: Binding,
    pub(crate) down: Binding,
    pub(crate) top: Binding,
    pub(crate) bottom: Binding,
    pub(crate) back: Binding,
    pub(crate) run: Binding,
    pub(crate) stop: Binding,
    pub(crate) restart: Binding,
    pub(crate) star: Binding,
    pub(crate) add: Binding,
    pub(crate) edit: Binding,
    pub(crate) remove: Binding,
    pub(crate) move_up: Binding,
    pub(crate) move_down: Binding,
    pub(crate) yank: Binding,
    pub(crate) reload: Binding,
    pub(crate) yes: Binding,
    pub(crate) no: Binding,
}

pub(crate) fn b(help: &str, desc: &str, keys: &[&str]) -> Binding {
    Binding::new().with_keys(keys).with_help(help, desc)
}

pub(crate) fn new_key_map() -> KeyMap {
    KeyMap {
        // the frame: lazygit's keys
        next_panel: b("tab/l", "next panel", &["tab", "l", "right"]),
        prev_panel: b("shift+tab/h", "previous panel", &["shift+tab", "h", "left"]),
        panels: b("1-3", "a panel", &["1", "2", "3"]),
        tabs: b("[ ]", "its tabs", &["[", "]"]),
        focus_pane: b("0", "the output", &["0"]),
        scroll_pane: b("pgup/pgdn", "scroll the output", &["pgup", "pgdown", "ctrl+u", "ctrl+d", "K", "J"]),
        full: b("+/_", "the output, the whole window", &["+", "_"]),
        filter: b("/", "filter", &["/"]),
        help: b("?", "all keys", &["?"]),
        quit: b("q", "quit, leaving them running", &["q", "ctrl+c"]),
        up: b("↑/k", "up", &["up", "k"]),
        down: b("↓/j", "down", &["down", "j"]),
        top: b("g", "top", &["home", "g"]),
        bottom: b("G", "bottom, and follow", &["end", "G"]),
        back: b("esc", "back", &["esc"]),

        run: b("enter", "run", &["enter"]),
        stop: b("s", "stop", &["s"]),
        restart: b("R", "run again", &["R"]),
        star: b("m", "favorite (★)", &["m"]),
        add: b("a", "add your own", &["a"]),
        edit: b("e", "edit", &["e"]),
        remove: b("D", "remove", &["D"]),
        move_up: b("<", "move up", &["<"]),
        move_down: b(">", "move down", &[">"]),
        yank: b("y", "copy the command", &["y"]),
        reload: b("r", "read the files again", &["r"]),
        yes: b("y", "yes", &["y", "enter"]),
        no: b("n", "no", &["n", "esc"]),
    }
}

/// KeyHelp is what the help bar shows on a screen.
pub(crate) struct KeyHelp {
    pub(crate) short: Vec<Binding>,
    pub(crate) full: Vec<Binding>,
}

impl KeyHelp {
    pub(crate) fn short(short: Vec<Binding>) -> KeyHelp {
        KeyHelp { short, full: Vec::new() }
    }
}

impl help::KeyMap for KeyHelp {
    fn short_help(&self) -> Vec<Binding> {
        self.short.clone()
    }

    /// full_help is the keys in columns of four.
    fn full_help(&self) -> Vec<Vec<Binding>> {
        self.full.chunks(4).map(<[Binding]>::to_vec).collect()
    }
}
