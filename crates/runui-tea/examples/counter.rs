//! A tiny program: a counter you move with the arrows (or k and j), a
//! spinner turning while it runs, the window's size, and q to quit; ! panics,
//! to see the terminal given back. Run it full screen with `--alt`.
//!
//! cargo run -p runui-tea --example counter -- --alt

use std::time::Duration;

use runui_tea::bubbles::key::{self, Binding};
use runui_tea::bubbles::{help, spinner};
use runui_tea::style::{self, Style, color, rounded_border};
use runui_tea::tea::{self, Cmd, KeyPressMsg, Model, MouseMode, Msg, Program, View, WindowSizeMsg};

const ACCENT: style::Color = color("#7DD3FC");
const DIM: Style = Style::new().foreground(color("#8B93A1"));
const BOX: Style = Style::new().border(rounded_border(), &[]).border_foreground(&[ACCENT]).padding(&[0, 1]);

struct Keys {
    up: Binding,
    down: Binding,
    quit: Binding,
}

impl help::KeyMap for Keys {
    fn short_help(&self) -> Vec<Binding> {
        vec![self.up.clone(), self.down.clone(), self.quit.clone()]
    }
    fn full_help(&self) -> Vec<Vec<Binding>> {
        vec![self.short_help()]
    }
}

/// A second has passed.
struct Second;

struct Counter {
    n: i32,
    seconds: i32,
    width: i32,
    height: i32,
    alt: bool,
    keys: Keys,
    help: help::Model,
    spin: spinner::Model,
}

fn every_second() -> Cmd {
    tea::tick(Duration::from_secs(1), |_| tea::msg(Second))
}

impl Model for Counter {
    fn init(&mut self) -> Cmd {
        tea::batch(vec![self.spin.tick(), every_second()])
    }

    fn update(&mut self, msg: Msg) -> Cmd {
        if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            if key::matches(k, &[&self.keys.quit]) {
                return tea::quit();
            }
            if k.string() == "!" {
                panic!("you pressed !");
            }
            if key::matches(k, &[&self.keys.up]) {
                self.n += 1;
            } else if key::matches(k, &[&self.keys.down]) {
                self.n -= 1;
            }
            return None;
        }
        if let Some(s) = msg.downcast_ref::<WindowSizeMsg>() {
            self.width = s.width;
            self.height = s.height;
            self.help.set_width(s.width);
            return None;
        }
        if msg.is::<Second>() {
            self.seconds += 1;
            return every_second();
        }
        self.spin.update(&msg)
    }

    fn view(&mut self) -> View {
        let body = format!(
            "{} counting: {}\n{}",
            self.spin.view(),
            Style::new().bold(true).foreground(ACCENT).render(&self.n.to_string()),
            DIM.render(&format!("{}×{} · {} s", self.width, self.height, self.seconds)),
        );
        let content = style::join_vertical(style::LEFT, &[&BOX.render(&body), &self.help.view(&self.keys)]);
        let mut v = View::new(content);
        v.alt_screen = self.alt;
        v.window_title = "runui-tea counter".into();
        v.mouse_mode = if self.alt { MouseMode::CellMotion } else { MouseMode::None };
        v
    }
}

fn main() -> std::io::Result<()> {
    let alt = std::env::args().any(|a| a == "--alt");
    let b = |keys: &[&str], h: &str, d: &str| Binding::new().with_keys(keys).with_help(h, d);
    let model = Counter {
        n: 0,
        seconds: 0,
        width: 0,
        height: 0,
        alt,
        keys: Keys {
            up: b(&["up", "k"], "↑/k", "up"),
            down: b(&["down", "j"], "↓/j", "down"),
            quit: b(&["q", "ctrl+c"], "q", "quit"),
        },
        help: help::Model::new(),
        spin: spinner::new(vec![
            spinner::with_spinner(spinner::mini_dot()),
            spinner::with_style(Style::new().foreground(ACCENT)),
        ]),
    };
    let m = Program::new(model).run()?;
    println!("counted to {}", m.n);
    Ok(())
}
