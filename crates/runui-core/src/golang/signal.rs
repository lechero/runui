//! Signals by name, as Go's `syscall.Signal.String` says them: what ended
//! a run is written down in run.json in these words ("terminated",
//! "killed"), so the Go build and this one read each other's runs alike.

#[cfg(target_os = "macos")]
const SIGNALS: [&str; 32] = [
    "",
    "hangup",
    "interrupt",
    "quit",
    "illegal instruction",
    "trace/BPT trap",
    "abort trap",
    "EMT trap",
    "floating point exception",
    "killed",
    "bus error",
    "segmentation fault",
    "bad system call",
    "broken pipe",
    "alarm clock",
    "terminated",
    "urgent I/O condition",
    "suspended (signal)",
    "suspended",
    "continued",
    "child exited",
    "stopped (tty input)",
    "stopped (tty output)",
    "I/O possible",
    "cputime limit exceeded",
    "filesize limit exceeded",
    "virtual timer expired",
    "profiling timer expired",
    "window size changes",
    "information request",
    "user defined signal 1",
    "user defined signal 2",
];

#[cfg(not(target_os = "macos"))]
const SIGNALS: [&str; 32] = [
    "",
    "hangup",
    "interrupt",
    "quit",
    "illegal instruction",
    "trace/breakpoint trap",
    "aborted",
    "bus error",
    "floating point exception",
    "killed",
    "user defined signal 1",
    "segmentation fault",
    "user defined signal 2",
    "broken pipe",
    "alarm clock",
    "terminated",
    "stack fault",
    "child exited",
    "continued",
    "stopped (signal)",
    "stopped",
    "stopped (tty input)",
    "stopped (tty output)",
    "urgent I/O condition",
    "CPU time limit exceeded",
    "file size limit exceeded",
    "virtual timer expired",
    "profiling timer expired",
    "window changed",
    "I/O possible",
    "power failure",
    "bad system call",
];

/// name is a signal's name in Go's words: "terminated" for SIGTERM,
/// "signal 40" for one it has no name for.
pub fn name(sig: i32) -> String {
    match usize::try_from(sig).ok().and_then(|i| SIGNALS.get(i)) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => format!("signal {sig}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_are_named_as_go_names_them() {
        assert_eq!(name(libc::SIGTERM), "terminated");
        assert_eq!(name(libc::SIGKILL), "killed");
        assert_eq!(name(libc::SIGINT), "interrupt");
        assert_eq!(name(libc::SIGHUP), "hangup");
        assert_eq!(name(0), "signal 0");
        assert_eq!(name(40), "signal 40");
    }
}
