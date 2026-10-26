//! The program: it owns the terminal while a model runs (raw mode, the
//! alternate screen and the mouse when the view asks, bracketed paste),
//! turns what the terminal sends into messages, runs commands on threads
//! of their own, and draws the view after every change. It gives the
//! terminal back as it found it when the model quits, and when anything
//! panics.

use std::io::{self, IsTerminal, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Once};
use std::thread;
use std::time::Duration;

use super::input::msg_of;
use super::profile::{Profile, detect};
use super::renderer::Renderer;
use super::{BatchMsg, Cmd, Model, Msg, QuitMsg, SequenceMsg, SetClipboardMsg, WindowSizeMsg, msg, osc52};

/// Whether a program has the terminal: what the panic hook gives back.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static STDERR_OUTPUT: AtomicBool = AtomicBool::new(false);
static HOOK: Once = Once::new();

/// What a command that panicked sends: the program ends with an error.
struct PanicMsg;

/// What the input reader sends once the program's cancel flag is set: the
/// program ends with an error, as Bubble Tea's does when the context it
/// was given (tea.WithContext) is canceled.
struct KilledMsg;
struct InputFailed(io::Error);

/// restore_terminal gives the terminal back after a panic: every mode the
/// program may have turned on turned off, the cursor shown, the alternate
/// screen left, raw mode off.
fn restore_terminal() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out: Box<dyn Write> =
        if STDERR_OUTPUT.load(Ordering::SeqCst) { Box::new(io::stderr()) } else { Box::new(io::stdout()) };
    let _ = out.write_all(b"\x1b[<1u\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?1004l\x1b[?1049l\x1b[?25h");
    let _ = out.flush();
    let _ = crossterm::terminal::disable_raw_mode();
}

/// install_panic_hook gives the terminal back before a panic's message is
/// printed, so it's readable, whichever thread panics.
fn install_panic_hook() {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            prev(info);
        }));
    });
}

/// Guard gives the terminal back when the program ends, however it ends.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Program runs a model in the terminal: Bubble Tea's `Program`.
pub struct Program<M: Model> {
    model: M,
    profile: Option<Profile>,
    cancel: Option<Arc<AtomicBool>>,
    stderr: bool,
}

impl<M: Model> Program<M> {
    /// new is a program for `model`.
    pub fn new(model: M) -> Program<M> {
        Program { model, profile: None, cancel: None, stderr: false }
    }

    /// with_cancel ends the program once cancel is set, with the error
    /// "program was killed: context canceled": what Bubble Tea's
    /// `tea.WithContext` does with a context canceled, as a signal does it.
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Program<M> {
        self.cancel = Some(cancel);
        self
    }

    /// with_color_profile draws with this profile instead of the one
    /// detected from the terminal and the environment.
    pub fn with_color_profile(mut self, p: Profile) -> Program<M> {
        self.profile = Some(p);
        self
    }

    /// Keep stdout reserved for a structured picker result. Input still comes
    /// from the controlling terminal, including inside shell substitutions.
    pub fn with_stderr(mut self) -> Self {
        self.stderr = true;
        self
    }

    /// run runs the model until it quits, and gives it back.
    pub fn run(mut self) -> io::Result<M> {
        let terminal = if self.stderr { io::stderr().is_terminal() } else { io::stdout().is_terminal() };
        let output: Box<dyn Write> = if self.stderr { Box::new(io::stderr()) } else { Box::new(io::stdout()) };
        let profile = self.profile.unwrap_or_else(|| detect(terminal, &|k| std::env::var(k).ok()));
        // keys come from the terminal itself when stdin isn't one, as
        // Bubble Tea reads them; without a terminal it says so in its words
        if !io::stdin().is_terminal()
            && let Err(err) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty")
        {
            return Err(io::Error::new(
                err.kind(),
                format!(
                    "bubbletea: error opening TTY: bubbletea: could not open TTY: open /dev/tty: {}",
                    errno_words(&err)
                ),
            ));
        }
        install_panic_hook();
        crossterm::terminal::enable_raw_mode()?;
        STDERR_OUTPUT.store(self.stderr, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
        let _guard = Guard;

        let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
        let (w, h) = (w as i32, h as i32);
        let mut renderer = Renderer::new(output, profile, w, h);
        renderer.start()?;

        let (tx, rx) = mpsc::channel::<Msg>();
        let stop = Arc::new(AtomicBool::new(false));
        let input = {
            let (tx, stop, cancel) = (tx.clone(), stop.clone(), self.cancel.clone());
            thread::spawn(move || read_input(tx, stop, cancel))
        };

        let _ = tx.send(msg(WindowSizeMsg { width: w, height: h }));
        spawn_cmd(self.model.init(), &tx);
        renderer.render(&self.model.view())?;

        let result = self.event_loop(&rx, &tx, &mut renderer);

        stop.store(true, Ordering::SeqCst);
        let _ = input.join();
        if result.is_ok() {
            renderer.render(&self.model.view())?;
        }
        if ACTIVE.load(Ordering::SeqCst) {
            renderer.stop()?;
            let _ = crossterm::terminal::disable_raw_mode();
            ACTIVE.store(false, Ordering::SeqCst);
        }
        result.map(|_| self.model)
    }

    fn event_loop(&mut self, rx: &Receiver<Msg>, tx: &Sender<Msg>, r: &mut Renderer<Box<dyn Write>>) -> io::Result<()> {
        loop {
            let Ok(m) = rx.recv() else { return Ok(()) };
            if self.handle(m, tx, r)? {
                return Ok(());
            }
            // take what else is waiting before drawing
            for _ in 0..256 {
                match rx.try_recv() {
                    Ok(m) => {
                        if self.handle(m, tx, r)? {
                            return Ok(());
                        }
                    }
                    Err(_) => break,
                }
            }
            r.render(&self.model.view())?;
        }
    }

    /// handle takes one message: true when the program should end.
    fn handle(&mut self, m: Msg, tx: &Sender<Msg>, r: &mut Renderer<Box<dyn Write>>) -> io::Result<bool> {
        if m.is::<QuitMsg>() {
            return Ok(true);
        }
        if m.is::<PanicMsg>() {
            return Err(io::Error::other("program was killed: program experienced a panic"));
        }
        if m.is::<KilledMsg>() {
            return Err(io::Error::other("program was killed: context canceled"));
        }
        if let Some(failed) = m.downcast_ref::<InputFailed>() {
            return Err(io::Error::new(failed.0.kind(), format!("terminal input failed: {}", failed.0)));
        }
        let m = match m.downcast::<BatchMsg>() {
            Ok(b) => {
                let tx = tx.clone();
                thread::spawn(move || exec_batch(b.0, &tx));
                return Ok(false);
            }
            Err(m) => m,
        };
        let m = match m.downcast::<SequenceMsg>() {
            Ok(s) => {
                let tx = tx.clone();
                thread::spawn(move || exec_sequence(s.0, &tx));
                return Ok(false);
            }
            Err(m) => m,
        };
        if let Some(c) = m.downcast_ref::<SetClipboardMsg>() {
            r.out().write_all(osc52(&c.0).as_bytes())?;
            r.out().flush()?;
            return Ok(false);
        }
        if let Some(s) = m.downcast_ref::<WindowSizeMsg>() {
            r.resize(s.width, s.height);
        }
        let cmd = self.model.update(m);
        spawn_cmd(cmd, tx);
        Ok(false)
    }
}

/// errno_words is a system error the way Go says it: the C library's
/// words, starting lower-case, without Rust's "(os error 6)".
fn errno_words(err: &io::Error) -> String {
    let text = err.to_string();
    let text = match (err.raw_os_error(), text.rfind(" (os error ")) {
        (Some(_), Some(i)) => &text[..i],
        _ => text.as_str(),
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// read_input reads the terminal until told to stop, sending what it
/// reads, and that the program's been canceled once it has.
fn read_input(tx: Sender<Msg>, stop: Arc<AtomicBool>, cancel: Option<Arc<AtomicBool>>) {
    while !stop.load(Ordering::SeqCst) {
        if cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst)) {
            let _ = tx.send(msg(KilledMsg));
            return;
        }
        match crossterm::event::poll(Duration::from_millis(20)) {
            Ok(true) => match crossterm::event::read() {
                Ok(e) => {
                    if let Some(m) = msg_of(e)
                        && tx.send(m).is_err()
                    {
                        return;
                    }
                }
                Err(error) => {
                    let _ = tx.send(msg(InputFailed(error)));
                    return;
                }
            },
            Ok(false) => {}
            Err(error) => {
                let _ = tx.send(msg(InputFailed(error)));
                return;
            }
        }
    }
}

/// run_guarded runs a command, a panic in it sent as a PanicMsg.
fn run_guarded(f: Box<dyn FnOnce() -> Option<Msg> + Send>, tx: &Sender<Msg>) -> Option<Msg> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(m) => m,
        Err(_) => {
            let _ = tx.send(msg(PanicMsg));
            None
        }
    }
}

/// spawn_cmd runs a command on a thread of its own, sending its message.
pub(crate) fn spawn_cmd(cmd: Cmd, tx: &Sender<Msg>) {
    let Some(f) = cmd else { return };
    let tx = tx.clone();
    thread::spawn(move || {
        if let Some(m) = run_guarded(f, &tx) {
            let _ = tx.send(m);
        }
    });
}

/// exec_batch runs commands at once, waiting for all of them: batches and
/// sequences they return run the same way, other messages are sent.
fn exec_batch(cmds: Vec<Cmd>, tx: &Sender<Msg>) {
    thread::scope(|s| {
        for f in cmds.into_iter().flatten() {
            s.spawn(move || {
                if let Some(m) = run_guarded(f, tx) {
                    dispatch(m, tx);
                }
            });
        }
    });
}

/// exec_sequence runs commands one after the other.
fn exec_sequence(cmds: Vec<Cmd>, tx: &Sender<Msg>) {
    for f in cmds.into_iter().flatten() {
        if let Some(m) = run_guarded(f, tx) {
            dispatch(m, tx);
        }
    }
}

fn dispatch(m: Msg, tx: &Sender<Msg>) {
    match m.downcast::<BatchMsg>() {
        Ok(b) => exec_batch(b.0, tx),
        Err(m) => match m.downcast::<SequenceMsg>() {
            Ok(s) => exec_sequence(s.0, tx),
            Err(m) => {
                let _ = tx.send(m);
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tea::{batch, cmd_msg, sequence};

    #[test]
    fn batches_and_sequences_send_every_message() {
        let (tx, rx) = mpsc::channel::<Msg>();
        let c = batch(vec![
            cmd_msg(|| 1u32),
            sequence(vec![cmd_msg(|| 2u32), batch(vec![cmd_msg(|| 3u32), cmd_msg(|| 4u32)]), cmd_msg(|| 5u32)]),
        ]);
        spawn_cmd(c, &tx);
        // the batch comes back as a message, as it does to the event loop
        let m = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let b = m.downcast::<BatchMsg>().ok().unwrap();
        exec_batch(b.0, &tx);
        drop(tx);
        let mut got: Vec<u32> = rx.iter().map(|m| *m.downcast_ref::<u32>().unwrap()).collect();
        got.sort();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn sequences_keep_order() {
        let (tx, rx) = mpsc::channel::<Msg>();
        let c = vec![
            cmd_msg(|| {
                thread::sleep(Duration::from_millis(30));
                1u32
            }),
            cmd_msg(|| 2u32),
        ];
        exec_sequence(c, &tx);
        drop(tx);
        let got: Vec<u32> = rx.iter().map(|m| *m.downcast_ref::<u32>().unwrap()).collect();
        assert_eq!(got, vec![1, 2]);
    }

    #[test]
    fn a_panicking_command_ends_the_program() {
        let (tx, rx) = mpsc::channel::<Msg>();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        spawn_cmd(crate::tea::cmd(|| panic!("boom")), &tx);
        let m = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        std::panic::set_hook(prev);
        assert!(m.is::<PanicMsg>());
    }
}
