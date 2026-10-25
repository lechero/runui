//! A spinner (Bubbles' `spinner`): frames shown one after the other, each
//! tick a message the spinner answers with the next tick.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime};

use crate::style::Style;
use crate::tea::{self, Cmd, Msg};

static LAST_ID: AtomicI64 = AtomicI64::new(0);

fn next_id() -> i32 {
    (LAST_ID.fetch_add(1, Ordering::SeqCst) + 1) as i32
}

/// Spinner is a spinner's frames and how fast they turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spinner {
    /// Frames.
    pub frames: Vec<&'static str>,
    /// Fps.
    pub fps: Duration,
}

fn spinner(frames: &[&'static str], per_second: u32) -> Spinner {
    Spinner { frames: frames.to_vec(), fps: Duration::from_secs(1) / per_second }
}

/// line is | / - \.
pub fn line() -> Spinner {
    spinner(&["|", "/", "-", "\\"], 10)
}
/// dot is a braille dot going round.
pub fn dot() -> Spinner {
    spinner(&["⣾ ", "⣽ ", "⣻ ", "⢿ ", "⡿ ", "⣟ ", "⣯ ", "⣷ "], 10)
}
/// mini_dot is a small braille dot going round.
pub fn mini_dot() -> Spinner {
    spinner(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"], 12)
}
/// jump is a jumping braille dot.
pub fn jump() -> Spinner {
    spinner(&["⢄", "⢂", "⢁", "⡁", "⡈", "⡐", "⡠"], 10)
}
/// pulse is a pulsing block.
pub fn pulse() -> Spinner {
    spinner(&["█", "▓", "▒", "░"], 8)
}
/// points is three points, one lit.
pub fn points() -> Spinner {
    spinner(&["∙∙∙", "●∙∙", "∙●∙", "∙∙●"], 7)
}
/// globe is the turning globe.
pub fn globe() -> Spinner {
    spinner(&["🌍", "🌎", "🌏"], 4)
}
/// moon is the moon's phases.
pub fn moon() -> Spinner {
    spinner(&["🌑", "🌒", "🌓", "🌔", "🌕", "🌖", "🌗", "🌘"], 8)
}
/// monkey is the three monkeys.
pub fn monkey() -> Spinner {
    spinner(&["🙈", "🙉", "🙊"], 3)
}
/// meter is a filling meter.
pub fn meter() -> Spinner {
    spinner(&["▱▱▱", "▰▱▱", "▰▰▱", "▰▰▰", "▰▰▱", "▰▱▱", "▱▱▱"], 7)
}
/// hamburger is a trigram going round.
pub fn hamburger() -> Spinner {
    spinner(&["☱", "☲", "☴", "☲"], 3)
}
/// ellipsis is dots appearing.
pub fn ellipsis() -> Spinner {
    spinner(&["", ".", "..", "..."], 3)
}

/// TickMsg turns a spinner a frame on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickMsg {
    /// time is when the tick came.
    pub time: SystemTime,
    /// id is the spinner it's for (0 for any).
    pub id: i32,
    tag: i32,
}

/// Model is the spinner.
#[derive(Clone, Debug)]
pub struct Model {
    /// Spinner.
    pub spinner: Spinner,
    /// Style.
    pub style: Style,
    frame: usize,
    id: i32,
    tag: i32,
}

/// Option sets something on a new spinner.
pub type SpinnerOption = Box<dyn FnOnce(&mut Model)>;

/// with_spinner sets the frames.
pub fn with_spinner(s: Spinner) -> SpinnerOption {
    Box::new(move |m| m.spinner = s)
}

/// with_style sets the style.
pub fn with_style(s: Style) -> SpinnerOption {
    Box::new(move |m| m.style = s)
}

impl Default for Model {
    fn default() -> Self {
        Model::new(Vec::new())
    }
}

/// new is Go's `spinner.New(opts...)`.
pub fn new(opts: Vec<SpinnerOption>) -> Model {
    Model::new(opts)
}

impl Model {
    /// new is a spinner (the line, by default) with the options given.
    pub fn new(opts: Vec<SpinnerOption>) -> Model {
        let mut m = Model { spinner: line(), style: Style::new(), frame: 0, id: next_id(), tag: 0 };
        for o in opts {
            o(&mut m);
        }
        m
    }

    /// id is the spinner's own number, what its ticks carry.
    pub fn id(&self) -> i32 {
        self.id
    }

    /// update takes a tick for this spinner: the next frame, and the next
    /// tick.
    pub fn update(&mut self, msg: &Msg) -> Cmd {
        let t = msg.downcast_ref::<TickMsg>()?;
        // If an ID is set, and the ID doesn't belong to this spinner,
        // reject the message.
        if t.id > 0 && t.id != self.id {
            return None;
        }
        // If a tag is set, and it's not the one we expect, reject the
        // message. This prevents the spinner from receiving too many
        // messages and thus spinning too fast.
        if t.tag > 0 && t.tag != self.tag {
            return None;
        }
        self.frame += 1;
        if self.frame >= self.spinner.frames.len() {
            self.frame = 0;
        }
        self.tag += 1;
        self.tick_after(self.id, self.tag)
    }

    /// view is the frame shown, styled.
    pub fn view(&self) -> String {
        match self.spinner.frames.get(self.frame) {
            Some(f) => self.style.render(f),
            None => "(error)".into(),
        }
    }

    /// tick is the command that starts the spinner: a tick right away.
    pub fn tick(&self) -> Cmd {
        let (id, tag) = (self.id, self.tag);
        tea::cmd_msg(move || TickMsg { time: SystemTime::now(), id, tag })
    }

    fn tick_after(&self, id: i32, tag: i32) -> Cmd {
        tea::tick(self.spinner.fps, move |t| tea::msg(TickMsg { time: t, id, tag }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_new() {
        let s = Model::new(vec![]);
        assert_eq!(s.spinner, line());
        let custom = Spinner { frames: vec!["a", "b", "c", "d"], fps: Duration::from_secs(1) / 16 };
        let s = Model::new(vec![with_spinner(custom.clone())]);
        assert_eq!(s.spinner, custom);
        for (name, sp) in [("mini dot", mini_dot()), ("points", points()), ("meter", meter()), ("pulse", pulse())] {
            assert_eq!(Model::new(vec![with_spinner(sp.clone())]).spinner, sp, "{name}");
        }
    }

    #[test]
    fn ticks_turn_only_its_own() {
        let mut s = Model::new(vec![with_spinner(mini_dot())]);
        assert_eq!(s.view(), "⠋");
        let t = s.tick().unwrap()().unwrap();
        let next = s.update(&t);
        assert!(next.is_some());
        assert_eq!(s.view(), "⠙");
        let other = tea::msg(TickMsg { time: SystemTime::now(), id: s.id() + 1000, tag: 0 });
        assert!(s.update(&other).is_none());
        assert_eq!(s.view(), "⠙");
    }
}
