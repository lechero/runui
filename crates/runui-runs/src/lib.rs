//! Starts commands that outlive runui and finds them again. Ported from
//! internal/runui/runs.
//!
//! Each run is a folder under ~/.local/state/runui/runs: run.json says what
//! was run, where and how it ended, output.log is everything it printed,
//! and lock is held for as long as it runs. runui doesn't run the command
//! itself: it starts a supervisor, runui _supervise, in a session of its
//! own, so closing the terminal or the tmux popup doesn't end it. The
//! supervisor runs the command in a process group of its own, writes down
//! when it ends and how, and stops the whole group when asked to. The lock
//! is taken before the supervisor starts and handed to it, and the system
//! lets go of it when the supervisor ends, however that happens, so a held
//! lock means the run is going: no process IDs to guess at after a
//! restart. The files are the Go build's, so either build finds and stops
//! the other's runs.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, Utc};
use runui_core::golang::{self, exec, filepath, json::nullable, opt_time, time as go_time, zero_time};
use serde::{Deserialize, Serialize};

mod output;
mod supervise;
mod sys;

pub use output::{Output, clean};
pub use supervise::{set_stop_grace, stop_grace, supervise};
pub use sys::alive;

/// on_signals gives each of these signals, once caught, to deliver,
/// whichever thread it arrives on (Go's signal.Notify), until deliver says
/// it's done; the process isn't ended by them meanwhile.
pub fn on_signals(signals: &[i32], deliver: impl Fn(i32) -> bool + Send + 'static) -> std::io::Result<()> {
    sys::notify(signals, deliver)
}

// The files in a run's folder.
const RUN_FILE: &str = "run.json";
const OUTPUT_FILE: &str = "output.log";
const LOCK_FILE: &str = "lock";

/// KEPT is how many runs that have ended are kept, the last ones; starting
/// a command again forgets its runs that have ended before then.
const KEPT: usize = 40;

/// State is where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// its supervisor hasn't started the command yet
    Starting,
    Running,
    /// it ended on its own
    Exited,
    /// runui stopped it
    Stopped,
    /// its supervisor ended without saying how the command did: killed,
    /// or the machine restarted
    Lost,
}

/// Run is a command started by runui.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    #[serde(default, deserialize_with = "nullable")]
    pub id: String,
    /// the folder it runs in
    #[serde(default, deserialize_with = "nullable")]
    pub dir: String,
    /// the task's, script's or favorite's name
    #[serde(default, deserialize_with = "nullable", skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable")]
    pub command: String,
    /// what runs the command: sh -c, through mise exec where there's mise
    #[serde(default, deserialize_with = "nullable", serialize_with = "nil_when_empty")]
    pub shell: Vec<String>,
    #[serde(default = "zero_time", with = "go_time")]
    pub started: DateTime<Utc>,
    /// the supervisor's process
    #[serde(default, deserialize_with = "nullable", skip_serializing_if = "is_zero")]
    pub supervisor: i32,
    /// the command's, which leads its process group
    #[serde(default, deserialize_with = "nullable", skip_serializing_if = "is_zero")]
    pub pid: i32,
    #[serde(default, with = "opt_time", skip_serializing_if = "Option::is_none")]
    pub ended: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// what ended it, when a signal did
    #[serde(default, deserialize_with = "nullable", skip_serializing_if = "String::is_empty")]
    pub signal: String,
    /// runui asked it to stop
    #[serde(default, deserialize_with = "nullable", skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
    /// why it couldn't start
    #[serde(default, deserialize_with = "nullable", skip_serializing_if = "String::is_empty")]
    pub error: String,

    #[serde(skip)]
    folder: String,
    /// its lock is held
    #[serde(skip)]
    live: bool,
}

impl Default for Run {
    fn default() -> Run {
        Run {
            id: String::new(),
            dir: String::new(),
            name: String::new(),
            command: String::new(),
            shell: Vec::new(),
            started: zero_time(),
            supervisor: 0,
            pid: 0,
            ended: None,
            exit: None,
            signal: String::new(),
            stopped: false,
            error: String::new(),
            folder: String::new(),
            live: false,
        }
    }
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

// a shell of nothing is Go's nil slice: null
fn nil_when_empty<S: serde::Serializer>(v: &[String], s: S) -> std::result::Result<S::Ok, S::Error> {
    if v.is_empty() { s.serialize_none() } else { v.serialize(s) }
}

impl Run {
    /// state is where the run is now.
    pub fn state(&self) -> State {
        match () {
            _ if self.live && self.pid == 0 && self.ended.is_none() => State::Starting,
            _ if self.live && self.ended.is_none() => State::Running,
            _ if self.ended.is_none() => State::Lost,
            _ if self.stopped => State::Stopped,
            _ => State::Exited,
        }
    }

    /// live says the run is still going: starting or running.
    pub fn live(&self) -> bool {
        matches!(self.state(), State::Starting | State::Running)
    }

    /// label is what the run is called: its name, else its command.
    pub fn label(&self) -> &str {
        if self.name.is_empty() { &self.command } else { &self.name }
    }

    /// output is the file with what the run printed.
    pub fn output(&self) -> String {
        filepath::join(&[&self.folder, OUTPUT_FILE])
    }

    /// took is how long it ran, or has been running, in nanoseconds (a Go
    /// time.Duration).
    pub fn took(&self, now: DateTime<Utc>) -> i64 {
        let end = self.ended.unwrap_or(now);
        let d = end - self.started;
        d.num_nanoseconds().unwrap_or(if d < chrono::TimeDelta::zero() { i64::MIN } else { i64::MAX })
    }

    /// with_folder is the run kept in this folder, for a test that writes
    /// one down itself.
    pub fn with_folder(mut self, folder: &str) -> Run {
        self.folder = folder.to_string();
        self
    }
}

/// Store is the runs folder.
#[derive(Clone, Debug, Default)]
pub struct Store {
    /// the runs folder
    pub root: String,
    /// how a supervisor starts: runui's own binary and its hidden command,
    /// the run's folder added
    pub exe: Vec<String>,
    /// what a command runs through, the command added
    pub shell: Vec<String>,
    /// added to what the command inherits
    pub env: Vec<String>,
}

/// default_root is ~/.local/state/runui/runs, or under $XDG_STATE_HOME
/// when that's set.
pub fn default_root() -> Result<String> {
    if let Ok(dir) = std::env::var("XDG_STATE_HOME")
        && !dir.is_empty()
    {
        return Ok(filepath::join(&[&dir, "runui", "runs"]));
    }
    let home = filepath::user_home_dir()?;
    Ok(filepath::join(&[&home, ".local", "state", "runui", "runs"]))
}

/// default_shell is sh -c, through mise exec where mise is installed, so a
/// command gets the Node the folder pins (.nvmrc, mise.toml) as it would
/// in a shell where mise is active, even from a tmux popup, which doesn't
/// start one.
pub fn default_shell() -> Vec<String> {
    match exec::look_path("mise") {
        Some(mise) => vec![mise, "exec".into(), "--".into(), "/bin/sh".into(), "-c".into()],
        None => vec!["/bin/sh".into(), "-c".into()],
    }
}

impl Store {
    /// start runs a command in a folder, under a supervisor that outlives
    /// runui. A supervisor that couldn't start is written down as a run
    /// that couldn't, and comes back with the error.
    pub fn start(&self, dir: &str, name: &str, command: &str) -> (Run, Option<anyhow::Error>) {
        match self.try_start(dir, name, command) {
            Ok(r) => (r, None),
            Err(failed) => {
                let (r, e) = *failed;
                (r, Some(e))
            }
        }
    }

    fn try_start(&self, dir: &str, name: &str, command: &str) -> std::result::Result<Run, Box<(Run, anyhow::Error)>> {
        let none = |e: anyhow::Error| Box::new((Run::default(), e));
        if self.exe.is_empty() {
            return Err(none(anyhow!("no supervisor to start")));
        }
        let shell =
            if self.shell.is_empty() { vec!["/bin/sh".to_string(), "-c".to_string()] } else { self.shell.clone() };
        let mut r = Run {
            id: new_id(),
            dir: dir.to_string(),
            name: name.to_string(),
            command: command.to_string(),
            shell,
            started: Utc::now(),
            ..Default::default()
        };
        r.folder = filepath::join(&[&self.root, &r.id]);
        golang::mkdir_all(Path::new(&r.folder), 0o700).map_err(none)?;
        write_run(&r).map_err(none)?;
        let out = r.output();
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&out)
            .map_err(|e| none(golang::path_error("open", Path::new(&out), e)))?;
        let lock_path = filepath::join(&[&r.folder, LOCK_FILE]);
        // the supervisor has its own copy, which keeps it held
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .map_err(|e| none(golang::path_error("open", Path::new(&lock_path), e)))?;
        if let Err(e) = sys::flock_try(&lock) {
            let msg = format!("locking the run: {}", golang::errno_words(&e));
            return Err(none(runui_core::wrap(e, msg)));
        }
        let mut cmd = Command::new(&self.exe[0]);
        cmd.args(&self.exe[1..]).arg(&r.folder).current_dir(dir);
        for kv in &self.env {
            match kv.split_once('=') {
                Some((k, v)) => cmd.env(k, v),
                None => cmd.env(kv, ""),
            };
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        sys::as_supervisor(&mut cmd, lock.as_raw_fd());
        match cmd.spawn() {
            Err(e) => {
                let err = anyhow!(exec::start_error(&self.exe[0], &e));
                r.ended = Some(Utc::now());
                r.error = err.to_string();
                let _ = write_run(&r);
                Err(Box::new((r, err)))
            }
            Ok(mut child) => {
                // reaped here while runui is open; launchd or init does it after
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                drop(lock);
                r.live = true;
                self.prune(&r);
                Ok(r)
            }
        }
    }

    /// list is every run, the newest first.
    pub fn list(&self) -> Result<Vec<Run>> {
        let entries = match golang::read_dir(Path::new(&self.root)) {
            Ok(entries) => entries,
            Err(e) if golang::is_not_exist(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for e in entries {
            if !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            // one being made, or a folder that isn't a run
            if let Ok(r) = read_run(&filepath::join(&[&self.root, &name])) {
                out.push(r);
            }
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.started));
        Ok(out)
    }

    /// get is a run as it is now.
    pub fn get(&self, id: &str) -> Result<Run> {
        read_run(&filepath::join(&[&self.root, id]))
    }

    /// stop asks a run's supervisor to stop it: the command's process group
    /// gets SIGTERM, and SIGKILL if it's still there a few seconds later.
    /// A run that's just starting gets a moment for its supervisor to write
    /// itself down: what it prints can come before that.
    pub fn stop(&self, r: &Run) -> Result<()> {
        let deadline = Instant::now() + START_WAIT;
        loop {
            let r = self.get(&r.id)?;
            if !r.live() {
                return Err(anyhow!("{} isn't running", r.label()));
            }
            if r.supervisor != 0 {
                return sys::kill(r.supervisor, libc::SIGTERM).map_err(|e| anyhow!(golang::errno_words(&e)));
            }
            if Instant::now() > deadline {
                return Err(anyhow!("{} is still starting", r.label()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// remove forgets a run that has ended, and its output.
    pub fn remove(&self, r: &Run) -> Result<()> {
        let r = self.get(&r.id)?;
        if r.live() {
            return Err(anyhow!("{} is still running", r.label()));
        }
        remove_all(&r.folder)
    }

    /// prune forgets the runs of the command just started in the same
    /// folder that have ended, and the oldest runs that have ended beyond
    /// KEPT.
    fn prune(&self, started: &Run) {
        let Ok(all) = self.list() else { return };
        let mut ended = 0;
        for r in all {
            if r.id == started.id || r.live() {
                continue;
            }
            ended += 1;
            if (r.dir == started.dir && r.command == started.command) || ended > KEPT {
                let _ = remove_all(&r.folder);
            }
        }
    }
}

/// START_WAIT is how long stop waits for a run's supervisor to say who it is.
const START_WAIT: Duration = Duration::from_secs(3);

/// remove_all is Go's os.RemoveAll: the folder and what's in it, nothing
/// when it's not there.
fn remove_all(path: &str) -> Result<()> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(golang::path_error("unlinkat", Path::new(path), e)),
        _ => Ok(()),
    }
}

/// read_run reads a run's folder, and whether it's still going by its lock.
pub(crate) fn read_run(folder: &str) -> Result<Run> {
    let path = filepath::join(&[folder, RUN_FILE]);
    let data = golang::read_file(Path::new(&path))?;
    let mut r: Run = golang::json::unmarshal(&data).map_err(|e| anyhow!("{e}"))?;
    r.folder = folder.to_string();
    r.live = r.ended.is_none() && locked(&filepath::join(&[folder, LOCK_FILE]));
    Ok(r)
}

/// write_run writes run.json whole, through a temporary file, so a reader
/// never sees half of it.
pub fn write_run(r: &Run) -> Result<()> {
    let mut data = golang::marshal_indent(r, "", "  ")?;
    data.push(b'\n');
    let tmp = filepath::join(&[&r.folder, &format!("{RUN_FILE}.tmp")]);
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| golang::path_error("open", Path::new(&tmp), e))?;
    std::io::Write::write_all(&mut f, &data).map_err(|e| golang::path_error("write", Path::new(&tmp), e))?;
    drop(f);
    golang::rename(Path::new(&tmp), Path::new(&filepath::join(&[&r.folder, RUN_FILE])))
}

/// locked says some process holds the lock file: the run's supervisor.
fn locked(path: &str) -> bool {
    let Ok(f) = File::open(path) else { return false };
    match sys::flock_try(&f) {
        Ok(()) => {
            sys::flock_release(&f);
            false
        }
        Err(e) => e.raw_os_error() == Some(libc::EWOULDBLOCK),
    }
}

/// new_id names a run by when it started, which sorts them, and a few
/// random characters, which keeps two started at once apart.
fn new_id() -> String {
    let mut b = [0u8; 3];
    if let Ok(mut f) = File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    format!("{}-{:02x}{:02x}{:02x}", Local::now().format("%Y%m%d-%H%M%S"), b[0], b[1], b[2])
}

#[cfg(test)]
mod tests;
