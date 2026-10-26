//! The system calls runs makes that Rust's standard library doesn't:
//! flock, kill, a session and fd 3 for a supervisor, fstat and close on
//! exec for the fd it's handed, and signals caught onto a pipe. Each is a
//! thin, checked wrapper, the only unsafe code in the crate.
#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};

fn check(rc: libc::c_int) -> io::Result<()> {
    if rc == -1 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// flock_try takes a file's lock for this open file, without waiting:
/// EWOULDBLOCK when another holds it.
pub(crate) fn flock_try(f: &File) -> io::Result<()> {
    // SAFETY: f's descriptor is open for as long as f is.
    check(unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) })
}

/// flock_release lets go of a lock flock_try took.
pub(crate) fn flock_release(f: &File) {
    // SAFETY: f's descriptor is open for as long as f is.
    let _ = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
}

/// kill sends a signal to a process, or to a process group when pid is
/// below zero: syscall.Kill.
pub(crate) fn kill(pid: i32, sig: i32) -> io::Result<()> {
    // SAFETY: kill takes plain integers.
    check(unsafe { libc::kill(pid, sig) })
}

/// Inspection pipes are polled beside the child deadline, so a descendant
/// retaining stdout cannot hold the caller forever.
pub(crate) fn nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl reads and updates flags on a descriptor owned by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor remains open, and O_NONBLOCK is a file status flag.
    check(unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) })
}

/// alive says a process is there to signal: kill(pid, 0) succeeding.
pub fn alive(pid: i32) -> bool {
    kill(pid, 0).is_ok()
}

/// getpid is this process's id.
pub(crate) fn getpid() -> i32 {
    // SAFETY: getpid can't fail.
    unsafe { libc::getpid() }
}

/// as_supervisor makes a command start in a session of its own with fd,
/// the run's lock, as its fd 3: what Go's ExtraFiles and Setsid do.
pub(crate) fn as_supervisor(cmd: &mut std::process::Command, fd: RawFd) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure runs in the child between fork and exec and only
    // makes async-signal-safe calls (fcntl, dup2, setsid).
    unsafe {
        cmd.pre_exec(move || {
            if fd == 3 {
                // dup2 onto itself would keep close-on-exec set
                check(libc::fcntl(3, libc::F_SETFD, 0))?;
            } else {
                check(libc::dup2(fd, 3))?;
            }
            check(libc::setsid())?;
            Ok(())
        });
    }
}

/// fd_is says fd is the file at path: the same device and inode,
/// looked at without opening the fd as a File, which would close it after.
pub(crate) fn fd_is(fd: RawFd, path: &str) -> bool {
    let mut handed = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat writes the stat it's given room for, or fails.
    if unsafe { libc::fstat(fd, handed.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: fstat succeeded, so it filled the stat in.
    let handed = unsafe { handed.assume_init() };
    use std::os::unix::fs::MetadataExt;
    // st_dev and st_ino are of other sizes on other systems
    #[allow(clippy::unnecessary_cast)]
    match std::fs::metadata(path) {
        Ok(want) => want.dev() == handed.st_dev as u64 && want.ino() == handed.st_ino as u64,
        Err(_) => false,
    }
}

/// close_on_exec keeps fd from the commands this process starts:
/// syscall.CloseOnExec.
pub(crate) fn close_on_exec(fd: RawFd) {
    // SAFETY: fcntl on an fd that may or may not be open only fails.
    let _ = unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
}

// SIGNAL_PIPE is the pipe's write end the handler writes a caught signal
// to; -1 before signals are caught.
static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(sig: libc::c_int) {
    let fd = SIGNAL_PIPE.load(Ordering::Relaxed);
    if fd >= 0 {
        let b = sig as u8;
        // SAFETY: write is async-signal-safe, b lives through the call.
        let _ = unsafe { libc::write(fd, (&b as *const u8).cast(), 1) };
    }
}

/// notify gives each of these signals, once caught, to deliver, whichever
/// thread it arrives on: Go's signal.Notify. A command started after keeps
/// the default actions, exec resetting them.
pub(crate) fn notify(signals: &[i32], deliver: impl Fn(i32) -> bool + Send + 'static) -> io::Result<()> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe fills in the two descriptors it's given room for.
    check(unsafe { libc::pipe(fds.as_mut_ptr()) })?;
    close_on_exec(fds[0]);
    close_on_exec(fds[1]);
    SIGNAL_PIPE.store(fds[1], Ordering::SeqCst);
    for &sig in signals {
        // SAFETY: the action is zeroed, then given a handler that's an
        // extern "C" fn of the right shape and an empty mask.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as *const () as libc::sighandler_t;
            sa.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            check(libc::sigaction(sig, &sa, std::ptr::null_mut()))?;
        }
    }
    let read = fds[0];
    std::thread::spawn(move || {
        loop {
            let mut b = 0u8;
            // SAFETY: read fills the one byte it's given, or fails.
            let n = unsafe { libc::read(read, (&mut b as *mut u8).cast(), 1) };
            if n == 1 {
                if !deliver(i32::from(b)) {
                    return;
                }
            } else if n == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return;
            }
        }
    });
    Ok(())
}
