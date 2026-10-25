//! Bubble Tea v2: a program is a [`Model`] that gets messages ([`Msg`]),
//! changes, and says what to do next as a command ([`Cmd`]): a function run
//! on a thread of its own, whose message comes back to the model. Its view
//! is drawn after every change, only the lines that changed rewritten.
//!
//! A message is any value, boxed; a model tells them apart by downcasting:
//!
//! ```
//! use runui_tea::tea::{self, Cmd, KeyPressMsg, Model, Msg, View, WindowSizeMsg};
//!
//! struct Counter { n: i32, width: i32 }
//!
//! impl Model for Counter {
//!     fn init(&mut self) -> Cmd { None }
//!     fn update(&mut self, msg: Msg) -> Cmd {
//!         if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
//!             match k.string().as_str() {
//!                 "q" | "ctrl+c" => return tea::quit(),
//!                 "up" => self.n += 1,
//!                 _ => {}
//!             }
//!         } else if let Some(s) = msg.downcast_ref::<WindowSizeMsg>() {
//!             self.width = s.width;
//!         }
//!         None
//!     }
//!     fn view(&mut self) -> View { View::new(format!("{}", self.n)) }
//! }
//!
//! let mut c = Counter { n: 0, width: 0 };
//! c.update(tea::msg(KeyPressMsg::new(tea::KEY_UP)));
//! assert_eq!(c.view().content, "1");
//! ```

mod input;
mod key;
mod mouse;
mod profile;
mod program;
mod renderer;

use std::any::Any;
use std::time::{Duration, Instant, SystemTime};

pub use key::*;
pub use mouse::*;
pub use profile::Profile;
pub use program::Program;

/// Msg is a message: anything, boxed. Models downcast it to see what it is.
pub type Msg = Box<dyn Any + Send>;

/// Cmd is a command: a function run on a thread of its own, the message it
/// returns (if any) sent to the model. `None` is Go's nil command: nothing
/// to do.
pub type Cmd = Option<Box<dyn FnOnce() -> Option<Msg> + Send + 'static>>;

/// msg boxes a value as a message (a message already boxed stays as it is).
pub fn msg<M: Any + Send>(m: M) -> Msg {
    let b: Box<dyn Any + Send> = Box::new(m);
    match b.downcast::<Msg>() {
        Ok(inner) => *inner,
        Err(b) => b,
    }
}

/// cmd makes a command of a function returning a message, or none.
pub fn cmd<F>(f: F) -> Cmd
where
    F: FnOnce() -> Option<Msg> + Send + 'static,
{
    Some(Box::new(f))
}

/// cmd_msg makes a command of a function returning a value, sent as the
/// message.
pub fn cmd_msg<F, M>(f: F) -> Cmd
where
    F: FnOnce() -> M + Send + 'static,
    M: Any + Send,
{
    Some(Box::new(move || Some(msg(f()))))
}

/// BatchMsg is commands to run at once, as `batch` makes them. The program
/// runs them itself; a model doesn't get it.
pub struct BatchMsg(pub Vec<Cmd>);

/// SequenceMsg is commands to run one after the other, as `sequence` makes
/// them. The program runs them itself; a model doesn't get it.
pub struct SequenceMsg(pub Vec<Cmd>);

/// batch runs commands at once, in no order. Nothing to run is `None`; a
/// single command is that command.
pub fn batch(cmds: Vec<Cmd>) -> Cmd {
    let mut valid: Vec<Cmd> = cmds.into_iter().filter(Option::is_some).collect();
    match valid.len() {
        0 => None,
        1 => valid.pop().unwrap_or(None),
        _ => Some(Box::new(move || Some(msg(BatchMsg(valid))))),
    }
}

/// sequence runs commands one after the other, each waiting for the one
/// before. Nothing to run is `None`; a single command is that command.
pub fn sequence(cmds: Vec<Cmd>) -> Cmd {
    let mut valid: Vec<Cmd> = cmds.into_iter().filter(Option::is_some).collect();
    match valid.len() {
        0 => None,
        1 => valid.pop().unwrap_or(None),
        _ => Some(Box::new(move || Some(msg(SequenceMsg(valid))))),
    }
}

/// tick is a command that waits `d`, counted from now, then sends what `f`
/// makes of the time.
pub fn tick<F>(d: Duration, f: F) -> Cmd
where
    F: FnOnce(SystemTime) -> Msg + Send + 'static,
{
    let deadline = Instant::now() + d;
    Some(Box::new(move || {
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
        Some(f(SystemTime::now()))
    }))
}

/// QuitMsg ends the program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuitMsg;

/// quit is the command that ends the program.
pub fn quit() -> Cmd {
    Some(Box::new(|| Some(msg(QuitMsg))))
}

/// SetClipboardMsg puts text on the system clipboard through the terminal
/// (OSC 52), as `set_clipboard` makes it. The program writes it itself; a
/// model doesn't get it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetClipboardMsg(pub String);

/// set_clipboard is the command that puts text on the system clipboard,
/// through the terminal: Bubble Tea's `tea.SetClipboard`.
pub fn set_clipboard(s: impl Into<String>) -> Cmd {
    let s = s.into();
    Some(Box::new(move || Some(msg(SetClipboardMsg(s)))))
}

/// osc52 is the sequence that sets the system clipboard to s:
/// `ansi.SetSystemClipboard`.
pub(crate) fn osc52(s: &str) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4 + 8);
    out.push_str("\x1b]52;c;");
    for chunk in b.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out.push('\x07');
    out
}

/// WindowSizeMsg is the terminal's size: sent when the program starts and
/// whenever the terminal is resized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WindowSizeMsg {
    /// width is the terminal's columns.
    pub width: i32,
    /// height is the terminal's rows.
    pub height: i32,
}

/// PasteMsg is text pasted into the terminal (bracketed paste).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PasteMsg {
    /// content is the text pasted.
    pub content: String,
}

/// FocusMsg is the terminal window gaining focus (when the view asks for
/// focus reports).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FocusMsg;

/// BlurMsg is the terminal window losing focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlurMsg;

/// MouseMode is which mouse events the terminal reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseMode {
    /// None: the terminal keeps the mouse (selecting text works).
    #[default]
    None,
    /// Clicks, releases, the wheel, and motion while a button is held.
    CellMotion,
    /// Every event, motion without a button too.
    AllMotion,
}

/// View is what a model shows, and how the terminal is set up to show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// content is the text drawn.
    pub content: String,
    /// alt_screen draws in the alternate screen, full window, leaving the
    /// terminal's scrollback as it was.
    pub alt_screen: bool,
    /// mouse_mode is which mouse events come as messages.
    pub mouse_mode: MouseMode,
    /// window_title sets the terminal window's title, when not empty.
    pub window_title: String,
    /// report_focus sends FocusMsg and BlurMsg.
    pub report_focus: bool,
    /// disable_bracketed_paste_mode turns pasting into key presses again.
    pub disable_bracketed_paste_mode: bool,
}

impl View {
    /// new is a view of `content`, the rest left as the terminal is.
    pub fn new(content: impl Into<String>) -> View {
        View { content: content.into(), ..Default::default() }
    }

    /// set_content replaces the text drawn.
    pub fn set_content(&mut self, content: impl Into<String>) {
        self.content = content.into();
    }
}

/// new_view is Go's `tea.NewView`.
pub fn new_view(content: impl Into<String>) -> View {
    View::new(content)
}

/// Model is a Bubble Tea program's state: `init` says what to do first,
/// `update` takes each message, `view` draws.
pub trait Model {
    /// init is the first command, or none.
    fn init(&mut self) -> Cmd;
    /// update takes a message, changes the model and says what to do next.
    fn update(&mut self, msg: Msg) -> Cmd;
    /// view is what to draw.
    fn view(&mut self) -> View;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: Cmd) -> Option<Msg> {
        c.map(|f| f()).unwrap_or(None)
    }

    #[test]
    fn batch_drops_nil_commands() {
        assert!(batch(vec![]).is_none());
        assert!(batch(vec![None, None]).is_none());
        let one = batch(vec![None, cmd_msg(|| 7i32), None]);
        let m = run(one).unwrap();
        assert_eq!(*m.downcast_ref::<i32>().unwrap(), 7);
        let two = batch(vec![cmd_msg(|| 1i32), None, cmd_msg(|| 2i32)]);
        let m = run(two).unwrap();
        let b = m.downcast_ref::<BatchMsg>().unwrap();
        assert_eq!(b.0.len(), 2);
    }

    #[test]
    fn sequence_drops_nil_commands() {
        assert!(sequence(vec![None]).is_none());
        let m = run(sequence(vec![cmd_msg(|| 1i32), cmd_msg(|| 2i32)])).unwrap();
        assert_eq!(m.downcast_ref::<SequenceMsg>().unwrap().0.len(), 2);
    }

    #[test]
    fn msg_never_boxes_twice() {
        let m = msg(QuitMsg);
        let again = msg(m);
        assert!(again.downcast_ref::<QuitMsg>().is_some());
    }

    #[test]
    fn tick_waits() {
        let start = Instant::now();
        let c = tick(Duration::from_millis(30), |_| msg(1u8));
        let m = run(c).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(30));
        assert_eq!(*m.downcast_ref::<u8>().unwrap(), 1);
    }

    #[test]
    fn the_clipboard_is_set_with_osc52() {
        assert_eq!(osc52("hello"), "\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(osc52("ab"), "\x1b]52;c;YWI=\x07");
        assert_eq!(osc52(""), "\x1b]52;c;\x07");
        let m = run(set_clipboard("npm run dev")).unwrap();
        assert_eq!(m.downcast_ref::<SetClipboardMsg>().unwrap().0, "npm run dev");
    }

    #[test]
    fn quit_sends_quit() {
        assert!(run(quit()).unwrap().downcast_ref::<QuitMsg>().is_some());
    }
}
