//! The supervisor: the process a run's command runs under.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::RawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use runui_core::golang::{self, exec, filepath, quote, signal};

use crate::{LOCK_FILE, read_run, sys, write_run};

/// STOP_GRACE is how long a command stopped with SIGTERM has before it
/// gets SIGKILL, in milliseconds.
static STOP_GRACE: AtomicU64 = AtomicU64::new(5000);

/// stop_grace is how long a command stopped with SIGTERM has before it
/// gets SIGKILL: 5 s (Go's `runs.StopGrace`).
pub fn stop_grace() -> Duration {
    Duration::from_millis(STOP_GRACE.load(Ordering::Relaxed))
}

/// set_stop_grace changes stop_grace, as a test does.
pub fn set_stop_grace(d: Duration) {
    STOP_GRACE.store(d.as_millis() as u64, Ordering::Relaxed);
}

/// LOCK_FD is where start hands the supervisor the run's lock.
const LOCK_FD: RawFd = 3;

enum Event {
    Signal,
    Done(std::io::Result<ExitStatus>),
}

/// supervise runs a run's command and waits for it, in the supervisor
/// process start started: it holds the run's lock until it ends, sends the
/// command's output to output.log, and writes down in run.json when the
/// command started, when it ended and how. SIGTERM, SIGINT or SIGHUP stops
/// the command's whole process group.
pub fn supervise(folder: &str) -> Result<()> {
    let mut r = read_run(folder)?;
    if !handed_lock(&filepath::join(&[folder, LOCK_FILE])) {
        bail!("no lock handed over: runui _supervise is for runui to start");
    }
    sys::close_on_exec(LOCK_FD); // the command doesn't get it: the lock is the supervisor's alone
    let path = r.output();
    let mut out = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| golang::path_error("open", Path::new(&path), e))?;
    let mut argv = r.shell.clone();
    argv.push(r.command.clone());
    let (tx, rx) = mpsc::channel::<Event>();
    let signals = tx.clone();
    sys::notify(&[libc::SIGTERM, libc::SIGINT, libc::SIGHUP], move |_| signals.send(Event::Signal).is_ok())?;
    r.supervisor = sys::getpid();
    let started = command(&argv).and_then(|mut cmd| {
        // The command's stdin is a pipe left open until it ends: a dev
        // server that stops when its stdin closes keeps running.
        cmd.current_dir(&r.dir)
            .stdin(Stdio::piped())
            .stdout(out.try_clone().map_err(|e| e.to_string())?)
            .stderr(out.try_clone().map_err(|e| e.to_string())?)
            .process_group(0);
        cmd.spawn().map_err(|e| exec::start_error(&argv[0], &e))
    });
    let mut child = match started {
        Ok(child) => child,
        Err(msg) => {
            let _ = writeln!(out, "runui: {msg}");
            (r.ended, r.exit, r.error) = (Some(Utc::now()), Some(127), msg);
            return write_run(&r);
        }
    };
    let keep_open = child.stdin.take();
    r.pid = child.id() as i32;
    if let Err(e) = write_run(&r) {
        let _ = writeln!(out, "runui: {e}");
    }
    std::thread::spawn(move || {
        let _ = tx.send(Event::Done(child.wait()));
    });
    let mut kill_at: Option<Instant> = None;
    loop {
        let ev = match kill_at {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match ev {
            Ok(Event::Signal) => {
                if !r.stopped {
                    r.stopped = true;
                    let _ = sys::kill(-r.pid, libc::SIGTERM);
                    kill_at = Some(Instant::now() + stop_grace());
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = sys::kill(-r.pid, libc::SIGKILL);
                kill_at = None;
            }
            Ok(Event::Done(status)) => {
                let code = match &status {
                    Ok(st) => {
                        if let Some(sig) = st.signal() {
                            r.signal = signal::name(sig);
                        }
                        st.code().unwrap_or(-1)
                    }
                    Err(e) => {
                        r.error = golang::errno_words(e);
                        -1
                    }
                };
                (r.ended, r.exit) = (Some(Utc::now()), Some(code));
                drop(keep_open);
                return write_run(&r);
            }
            Err(RecvTimeoutError::Disconnected) => return Err(anyhow!("lost the command")),
        }
    }
}

/// command is exec.Command(argv[0], argv[1:]...): a name without a slash
/// looked for in $PATH, Go's words when it isn't there.
fn command(argv: &[String]) -> std::result::Result<Command, String> {
    let Some(name) = argv.first() else { return Err("exec: no command".to_string()) };
    let mut cmd = if name.contains('/') {
        Command::new(name)
    } else {
        let path = exec::look_path(name)
            .ok_or_else(|| format!("exec: {}: executable file not found in $PATH", quote(name)))?;
        let mut cmd = Command::new(path);
        cmd.arg0(name);
        cmd
    };
    cmd.args(&argv[1..]);
    Ok(cmd)
}

/// handed_lock says fd 3 is the run's lock file, as start hands it over.
/// It's looked at without opening it as a file, which would close whatever
/// else fd 3 is once it's let go of.
fn handed_lock(path: &str) -> bool {
    sys::fd_is(LOCK_FD, path)
}
