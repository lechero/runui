//! The terminal runtime, styles and widgets the interface is drawn with: the
//! parts of Bubble Tea v2, Lip Gloss v2, Bubbles v2 and x/ansi that aem-cli's
//! interface uses, with the same behaviour and the same rendered output.
//!
//! - [`ansi`]: text with escape sequences in it, measured, cut and wrapped
//!   (`github.com/charmbracelet/x/ansi`).
//! - [`style`]: Lip Gloss: colors, styles, borders, joining and placing
//!   blocks, layers composited on a canvas.
//! - [`tea`]: Bubble Tea: messages, commands, keys and the mouse, the
//!   `Model` trait and the `Program` that runs one in the terminal.
//! - [`bubbles`]: the widgets: key bindings, help, text input and area,
//!   viewport, list, spinner, progress bar.
//! - [`testing`]: driving a model in tests without a terminal.
//!
//! Go's `int` is `i32` throughout: widths, heights, offsets and indexes,
//! so arithmetic that goes below zero behaves as it does in Go.
//!
//! # From Go
//!
//! | Go | here |
//! |---|---|
//! | `lipgloss.NewStyle().Bold(true).Padding(0, 1)` | `Style::new().bold(true).padding(&[0, 1])` (a `const`, as styles are `Copy`) |
//! | `lipgloss.Color("#7DD3FC")` | `style::color("#7DD3FC")` (a `const fn`) |
//! | `lipgloss.JoinVertical(lipgloss.Left, a, b)` | `style::join_vertical(style::LEFT, &[&a, &b])` |
//! | `lipgloss.NewCanvas`, `NewLayer(s).X(1).Z(1)`, `NewCompositor` | `Canvas::new`, `Layer::new(s).x(1).z(1)`, `Compositor::new(vec![…])` |
//! | `ansi.Truncate(s, w, "…")` | `ansi::truncate(&s, w, "…")` |
//! | `tea.Msg`, `tea.Cmd` (nil) | [`tea::Msg`] (boxed, downcast to read), [`tea::Cmd`] (`None`) |
//! | `func() tea.Msg { return x }` | `tea::cmd_msg(move \|\| x)` |
//! | `tea.Batch(a, b)`, `tea.Tick(d, f)`, `tea.Quit` | `tea::batch(vec![a, b])`, `tea::tick(d, f)`, `tea::quit()` |
//! | `switch msg := msg.(type) { case tea.KeyPressMsg: … }` | `if let Some(k) = msg.downcast_ref::<KeyPressMsg>() { … }` |
//! | `tea.KeyPressMsg{Code: tea.KeyTab, Mod: tea.ModShift}` | `KeyPressMsg::with_mods(tea::KEY_TAB, tea::MOD_SHIFT)` |
//! | `msg.String() == "ctrl+f"` | `k.string() == "ctrl+f"` (also `Display`) |
//! | `key.NewBinding(key.WithKeys("a"), key.WithHelp("a", "add"))` | `Binding::new().with_keys(&["a"]).with_help("a", "add")` |
//! | `key.Matches(msg, k.Up, k.Down)` | `key::matches(k, &[&keys.up, &keys.down])` |
//! | `m.input, cmd = m.input.Update(msg)` | `cmd = m.input.update(&msg)` |
//! | `it.(fileItem)` on a list item | `it.downcast_ref::<FileItem>()` |
//!
//! # Where it differs from Go
//!
//! - Colors are rendered 24-bit, as Lip Gloss writes them; the program
//!   brings them down to the terminal (256 or 16 colors, none with
//!   NO_COLOR, no escape sequences when the output isn't a terminal) the way
//!   charmbracelet/colorprofile detects it, but without reading terminfo or
//!   asking tmux, and by rewriting each frame's SGR sequences instead of
//!   drawing cells.
//! - The renderer rewrites the lines that changed, not the cells; frames are
//!   cut to the window as Bubble Tea's are.
//! - Cursors don't blink: a focused text input or area shows its cursor
//!   steadily (as Bubbles' does between blinks), and the blink commands are
//!   none.
//! - The program doesn't catch SIGINT or SIGTERM (in raw mode ctrl+c is a
//!   key, as in Bubble Tea v2), nor xterm's modifyOtherKeys (crossterm can't
//!   read it); Kitty's keyboard disambiguation is pushed as Bubble Tea does.
//! - The progress bar has no spring animation and no gradients (the
//!   interface draws it with `view_as` and one color).
//! - A list's filter matches name items by index, so matches computed for
//!   items since replaced show the new items.

pub mod ansi;
pub mod bubbles;
pub mod style;
pub mod tea;
pub mod testing;
