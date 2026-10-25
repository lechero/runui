//! Driving models in tests, without a terminal: a command run and its
//! message fed back to the model, as the program would, batches run one
//! command after another; a command still running after a while (a timer:
//! a spinner's tick, a poll) is left to run on its own.
//!
//! ```
//! use runui_tea::tea::{self, Cmd, Msg, Model, View, KeyPressMsg};
//! use runui_tea::testing;
//!
//! struct Echo(Vec<String>);
//! impl Model for Echo {
//!     fn init(&mut self) -> Cmd { None }
//!     fn update(&mut self, msg: Msg) -> Cmd {
//!         if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
//!             let s = k.string();
//!             return tea::cmd_msg(move || format!("saw {s}"));
//!         }
//!         if let Ok(s) = msg.downcast::<String>() {
//!             self.0.push(*s);
//!         }
//!         None
//!     }
//!     fn view(&mut self) -> View { View::new(self.0.join(",")) }
//! }
//!
//! let mut m = Echo(vec![]);
//! testing::press(&mut m, &["enter", "q"]);
//! assert_eq!(m.view().content, "saw enter,saw q");
//! ```

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::bubbles::spinner;
use crate::tea::{
    BatchMsg, Cmd, KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_TAB, KeyPressMsg, MOD_SHIFT, Model, Msg, SequenceMsg,
};

/// TIMEOUT is how long a command may take before it's taken for a timer:
/// what the interface's tests in Go wait.
pub const TIMEOUT: Duration = Duration::from_millis(250);

/// run_with runs `cmd` and feeds its message to `update`, and the commands
/// that returns, until there's nothing left to run: batches and sequences
/// are run one command after the other; a command that takes longer than
/// `timeout` is left running and its message dropped; messages `skip` says
/// to leave out are dropped.
pub fn run_with(cmd: Cmd, timeout: Duration, update: &mut dyn FnMut(Msg) -> Cmd, skip: &dyn Fn(&Msg) -> bool) {
    let Some(f) = cmd else { return };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    let Ok(Some(msg)) = rx.recv_timeout(timeout) else { return };
    if skip(&msg) {
        return;
    }
    let msg = match msg.downcast::<BatchMsg>() {
        Ok(b) => {
            for c in b.0 {
                run_with(c, timeout, update, skip);
            }
            return;
        }
        Err(m) => m,
    };
    let msg = match msg.downcast::<SequenceMsg>() {
        Ok(s) => {
            for c in s.0 {
                run_with(c, timeout, update, skip);
            }
            return;
        }
        Err(m) => m,
    };
    let next = update(msg);
    run_with(next, timeout, update, skip);
}

/// is_timer says whether a message is a timer's: a spinner's tick.
pub fn is_timer(msg: &Msg) -> bool {
    msg.is::<spinner::TickMsg>()
}

/// run runs `cmd` as the interface's Go tests' `run` does: messages fed
/// back to `update` until nothing's left, spinner ticks and commands still
/// running after 250ms left out.
pub fn run(cmd: Cmd, update: &mut dyn FnMut(Msg) -> Cmd) {
    run_with(cmd, TIMEOUT, update, &is_timer);
}

/// send gives a model a message and runs what it returns.
pub fn send<M: Model + ?Sized>(model: &mut M, msg: Msg) {
    let cmd = model.update(msg);
    run(cmd, &mut |m| model.update(m));
}

/// init runs the model's first command.
pub fn init<M: Model + ?Sized>(model: &mut M) {
    let cmd = model.init();
    run(cmd, &mut |m| model.update(m));
}

/// key is the key press a name stands for, as the interface's Go tests
/// make them: "enter", "esc", "tab", "shift+tab" and "down" as those keys,
/// anything else as typing its first character (or `KeyPressMsg::parse`
/// for names with a modifier, like "ctrl+f").
pub fn key(name: &str) -> KeyPressMsg {
    match name {
        "enter" => KeyPressMsg::new(KEY_ENTER),
        "esc" => KeyPressMsg::new(KEY_ESCAPE),
        "tab" => KeyPressMsg::new(KEY_TAB),
        "shift+tab" => KeyPressMsg::with_mods(KEY_TAB, MOD_SHIFT),
        "down" => KeyPressMsg::new(KEY_DOWN),
        k if k.chars().count() > 1 && k.contains('+') => KeyPressMsg::parse(k),
        k => {
            let c = k.chars().next().unwrap_or(' ');
            KeyPressMsg { code: c as u32, text: k.to_string(), ..Default::default() }
        }
    }
}

/// press presses keys one after the other, each one's commands run.
pub fn press<M: Model + ?Sized>(model: &mut M, keys: &[&str]) {
    for k in keys {
        send(model, crate::tea::msg(key(k)));
    }
}

/// type_text types text a character at a time.
pub fn type_text<M: Model + ?Sized>(model: &mut M, text: &str) {
    for c in text.chars() {
        send(model, crate::tea::msg(KeyPressMsg::char(c)));
    }
}

/// drive gives a model each message in turn, running what each returns.
pub fn drive<M: Model + ?Sized>(model: &mut M, msgs: Vec<Msg>) {
    for m in msgs {
        send(model, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tea::{self, View, WindowSizeMsg};
    use std::time::Instant;

    #[derive(Default)]
    struct Counter {
        n: i32,
        size: (i32, i32),
        seen: Vec<String>,
    }

    struct Add(i32);

    impl Model for Counter {
        fn init(&mut self) -> Cmd {
            tea::batch(vec![tea::cmd_msg(|| Add(1)), tea::cmd_msg(|| Add(2))])
        }
        fn update(&mut self, msg: Msg) -> Cmd {
            if let Some(a) = msg.downcast_ref::<Add>() {
                self.n += a.0;
                if a.0 == 2 {
                    return tea::sequence(vec![tea::cmd_msg(|| Add(10)), tea::cmd_msg(|| Add(100))]);
                }
            } else if let Some(s) = msg.downcast_ref::<WindowSizeMsg>() {
                self.size = (s.width, s.height);
            } else if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
                self.seen.push(k.string());
                if k.string() == "t" {
                    // a timer: dropped after the timeout
                    return tea::tick(Duration::from_secs(5), |_| tea::msg(Add(1000)));
                }
            }
            None
        }
        fn view(&mut self) -> View {
            View::new(self.n.to_string())
        }
    }

    #[test]
    fn runs_batches_and_sequences() {
        let mut c = Counter::default();
        init(&mut c);
        assert_eq!(c.n, 113);
    }

    #[test]
    fn timers_are_left_out() {
        let mut c = Counter::default();
        let start = Instant::now();
        press(&mut c, &["t", "enter", "shift+tab", "ctrl+f", "K"]);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(c.n, 0);
        assert_eq!(c.seen, vec!["t", "enter", "shift+tab", "ctrl+f", "K"]);
    }

    #[test]
    fn drives_messages() {
        let mut c = Counter::default();
        drive(&mut c, vec![tea::msg(WindowSizeMsg { width: 120, height: 40 }), tea::msg(Add(5))]);
        assert_eq!((c.size, c.n), ((120, 40), 5));
    }

    #[test]
    fn spinner_ticks_are_skipped() {
        let s = spinner::Model::new(vec![]);
        let mut fed = 0;
        run(s.tick(), &mut |_| {
            fed += 1;
            None
        });
        assert_eq!(fed, 0);
    }
}
