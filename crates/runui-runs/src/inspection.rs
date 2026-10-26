//! Explicit short native probes. They are not runs: no store, supervisor,
//! usage record or unread result. Every exit path cleans up the process group.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub timeout: Duration,
    pub bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(5), bytes: 64 * 1024 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Exited,
    Timeout,
    OutputLimit,
    Cancelled,
}

#[derive(Debug, Serialize)]
pub struct Output {
    pub end: End,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u128,
}

struct Process(Child);
impl Process {
    fn stop(&mut self) {
        if let Ok(pid) = i32::try_from(self.0.id()) {
            let _ = crate::sys::kill(-pid, libc::SIGKILL);
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
        let _ = self.0.wait();
    }
}

/// Command comes from a provider's fixed probe, never discovered script text.
/// Both streams share the byte budget; a cancelled or oversized probe is killed.
pub fn run(command: &mut Command, limits: Limits, cancel: &AtomicBool) -> io::Result<Output> {
    let started = Instant::now();
    if cancel.load(Ordering::Relaxed) {
        return Ok(Output {
            end: End::Cancelled,
            code: None,
            stdout: String::new(),
            stderr: String::new(),
            elapsed_ms: 0,
        });
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    let mut process = Process(command.spawn()?);
    let mut stdout = process.0.stdout.take().expect("piped stdout");
    let mut stderr = process.0.stderr.take().expect("piped stderr");
    crate::sys::nonblocking(stdout.as_raw_fd())?;
    crate::sys::nonblocking(stderr.as_raw_fd())?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut out_done, mut err_done) = (false, false);
    let mut status = None;
    let end = loop {
        if cancel.load(Ordering::Relaxed) {
            break End::Cancelled;
        }
        if started.elapsed() >= limits.timeout {
            break End::Timeout;
        }
        let remaining = limits.bytes.saturating_sub(out.len() + err.len());
        if read(&mut stdout, &mut out, remaining, &mut out_done)? {
            break End::OutputLimit;
        }
        let remaining = limits.bytes.saturating_sub(out.len() + err.len());
        if read(&mut stderr, &mut err, remaining, &mut err_done)? {
            break End::OutputLimit;
        }
        if status.is_none() {
            status = process.0.try_wait()?;
            if status.is_some() {
                process.stop();
            }
        }
        if status.is_some() && out_done && err_done {
            break End::Exited;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    process.stop();
    let final_status = process.0.wait()?;
    Ok(Output {
        end,
        code: final_status.code(),
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn read(pipe: &mut impl Read, data: &mut Vec<u8>, remaining: usize, done: &mut bool) -> io::Result<bool> {
    if *done {
        return Ok(false);
    }
    let mut bytes = [0; 4096];
    let size = bytes.len().min(remaining.saturating_add(1));
    match pipe.read(&mut bytes[..size]) {
        Ok(0) => *done = true,
        Ok(count) => {
            data.extend_from_slice(&bytes[..count.min(remaining)]);
            return Ok(count > remaining);
        }
        Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {}
        Err(error) => return Err(error),
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shell(text: &str) -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", text]);
        cmd
    }
    #[test]
    fn streams_exit_code_and_byte_limit_are_bounded() {
        let output =
            run(&mut shell("printf hello; printf warning >&2; exit 7"), Limits::default(), &AtomicBool::new(false))
                .unwrap();
        assert_eq!(output.end, End::Exited);
        assert_eq!(output.code, Some(7));
        assert_eq!(output.stdout, "hello");
        assert_eq!(output.stderr, "warning");
        let output = run(
            &mut shell("while :; do printf 'long output'; done"),
            Limits { bytes: 100, ..Limits::default() },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(output.end, End::OutputLimit);
        assert_eq!(output.stdout.len() + output.stderr.len(), 100);
    }
    #[test]
    fn timeout_and_cancellation_kill_descendants_and_do_not_wait_for_their_pipes() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("must-not-appear");
        for cancel_it in [false, true] {
            let ready = temp.path().join(format!("ready-{cancel_it}"));
            let release = temp.path().join(format!("release-{cancel_it}"));
            let mut cmd = shell(
                "(n=0; while [ ! -f \"$RELEASE\" ] && [ $n -lt 200 ]; do sleep 0.01; n=$((n + 1)); done; [ -f \"$RELEASE\" ] && touch \"$MARKER\") & : > \"$READY\"; wait",
            );
            cmd.env("MARKER", &marker).env("READY", &ready).env("RELEASE", &release);
            let cancel = AtomicBool::new(false);
            std::thread::scope(|scope| {
                if cancel_it {
                    scope.spawn(|| {
                        let until = Instant::now() + Duration::from_secs(3);
                        while !ready.exists() && Instant::now() < until {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        cancel.store(true, Ordering::Relaxed);
                    });
                }
                let timeout = if cancel_it { Duration::from_secs(5) } else { Duration::from_millis(80) };
                let output = run(&mut cmd, Limits { timeout, ..Limits::default() }, &cancel).unwrap();
                assert_eq!(output.end, if cancel_it { End::Cancelled } else { End::Timeout });
                if cancel_it {
                    assert!(ready.exists(), "cancellation must follow child startup");
                }
            });
            std::fs::write(release, "go").unwrap();
        }
        std::thread::sleep(Duration::from_millis(250));
        assert!(!marker.exists());
        let output = run(&mut shell("sleep 30 & exit"), Limits::default(), &AtomicBool::new(false)).unwrap();
        assert_eq!(output.end, End::Exited);
        assert!(output.elapsed_ms < 1000);
    }
}
