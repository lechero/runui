//! Bubbles v2: the widgets the interface is built of. Each is a model of
//! its own: it takes messages in `update(&mut self, msg: &Msg) -> Cmd` and
//! draws with `view()`, as Bubbles' do, with the same keys, the same styles
//! and the same rendered output.

pub mod cursor;
pub mod help;
pub mod key;
pub mod list;
pub mod paginator;
pub mod progress;
mod runeutil;
pub mod spinner;
pub mod textarea;
pub mod textinput;
pub mod viewport;

/// clipboard_read is what's on the system clipboard, as atotto/clipboard
/// reads it (pbpaste on a Mac, xclip, xsel or wl-paste elsewhere).
pub(crate) fn clipboard_read() -> Result<String, String> {
    let tries: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-out", "-selection", "clipboard"]),
            ("xsel", &["--output", "--clipboard"]),
        ]
    };
    let mut last = String::from("No clipboard utilities available.");
    for (cmd, args) in tries {
        match std::process::Command::new(cmd).args(*args).output() {
            Ok(o) if o.status.success() => return Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
            Ok(o) => last = String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}
