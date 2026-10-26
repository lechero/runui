//! Explicit terminal handoff. Custom launchers receive cwd and command as
//! separate arguments; discovery never invokes a launcher or an editor.

use std::process::{Command, Stdio};

use anyhow::{Result, anyhow};

pub(crate) fn open(directory: &str, command: &str) -> Result<()> {
    if let Some(launcher) = std::env::var_os("RUNUI_TERMINAL").filter(|launcher| !launcher.is_empty()) {
        let status = Command::new(launcher)
            .args([directory, command])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        return if status.success() { Ok(()) } else { Err(anyhow!("terminal launcher exited with {status}")) };
    }
    if std::env::var_os("TMUX").is_some_and(|value| !value.is_empty()) {
        // Multiple arguments bypass tmux's default shell. Keep the directory
        // and command out of tmux format strings as well as shell interpolation.
        let script = r#"cd -- "$1" || exit; /bin/sh -c "$2"; exec "${SHELL:-/bin/sh}" -i"#;
        let status = Command::new("tmux")
            .args(["new-window", "--", "/bin/sh", "-c", script, "runui-terminal", directory, command])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        return if status.success() { Ok(()) } else { Err(anyhow!("tmux terminal handoff exited with {status}")) };
    }
    Err(anyhow!(
        "Open terminal requires tmux or a RUNUI_TERMINAL launcher (receives DIR and COMMAND); Ctrl+Y copies the command"
    ))
}
