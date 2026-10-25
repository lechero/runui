//! Go's `os/exec` as runui uses it: `exec.LookPath`, and a process that
//! couldn't be started said in Go's words, "fork/exec /no/such/shell: no
//! such file or directory".

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;

use super::filepath;
use super::os::errno_words;

/// look_path is Go's `exec.LookPath`: the executable file a name names,
/// looked for in the folders of $PATH when it has no slash in it. None
/// where Go errs: not there, or found only through a relative folder in
/// $PATH (Go's ErrDot).
pub fn look_path(file: &str) -> Option<String> {
    if file.contains('/') {
        return find_executable(file).then(|| file.to_string());
    }
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':') {
        // Unix shell semantics: path element "" means "."
        let dir = if dir.is_empty() { "." } else { dir };
        let path = filepath::join(&[dir, file]);
        if find_executable(&path) {
            if !filepath::is_abs(&path) {
                return None;
            }
            return Some(path);
        }
    }
    None
}

// findExecutable: a file, not a folder, that may be run.
fn find_executable(file: &str) -> bool {
    let Ok(d) = fs::metadata(file) else { return false };
    if d.is_dir() {
        return false;
    }
    match eaccess_x(file) {
        Ok(()) => true,
        // ENOSYS means Eaccess is not available or not implemented. EPERM
        // can be returned by Linux containers employing seccomp. In both
        // cases, fall back to checking the permission bits.
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENOSYS | libc::EPERM)) => d.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

// unix.Eaccess(file, unix.X_OK): whether this process, as itself, may run
// the file.
#[allow(unsafe_code)]
fn eaccess_x(file: &str) -> io::Result<()> {
    let c = CString::new(file).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: c is a NUL-terminated path that outlives the call.
    let rc = unsafe { libc::faccessat(libc::AT_FDCWD, c.as_ptr(), libc::X_OK, libc::AT_EACCESS) };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// start_error is a process that couldn't be started, as Go's
/// `cmd.Start()` says it: its `*fs.PathError` for "fork/exec".
pub fn start_error(path: &str, err: &io::Error) -> String {
    format!("fork/exec {path}: {}", errno_words(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executables_are_found_as_go_finds_them() {
        assert!(look_path("/bin/sh").is_some());
        assert!(look_path("/no/such/shell").is_none());
        assert!(look_path("/bin").is_none());
        let err = io::Error::from_raw_os_error(libc::ENOENT);
        assert_eq!(start_error("/no/such/shell", &err), "fork/exec /no/such/shell: no such file or directory");
    }
}
