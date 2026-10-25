//! Go's `os` and `path/filepath` as aem-cli's files depend on them: files
//! made, renamed and read the way Go makes them (modes, temporary names),
//! and their errors in Go's words, like "open /x: permission denied".

use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// path_error is an error on a file in Go's words, like "open
/// /tmp/x.json: no such file or directory": its `*fs.PathError`. The
/// io::Error stays under it, for its kind.
pub fn path_error(op: &str, path: &Path, err: io::Error) -> anyhow::Error {
    let msg = format!("{op} {}: {}", path.display(), errno_words(&err));
    crate::wrap(err, msg)
}

/// errno_words is an io::Error's words the way Go says a system error's:
/// the C library's, starting lower-case, without Rust's "(os error 2)".
pub fn errno_words(err: &io::Error) -> String {
    if err.kind() == io::ErrorKind::UnexpectedEof && err.raw_os_error().is_none() {
        return "unexpected EOF".to_string();
    }
    let text = err.to_string();
    let text = match (err.raw_os_error(), text.rfind(" (os error ")) {
        (Some(_), Some(i)) => &text[..i],
        _ => text.as_str(),
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// is_not_exist says err is, or wraps, a file that isn't there: Go's
/// `errors.Is(err, os.ErrNotExist)`.
pub fn is_not_exist(err: &anyhow::Error) -> bool {
    crate::find::<io::Error>(err).is_some_and(|e| e.kind() == io::ErrorKind::NotFound)
}

/// read_file is a file's bytes: Go's `os.ReadFile`.
pub fn read_file(path: &Path) -> crate::Result<Vec<u8>> {
    let mut f = File::open(path).map_err(|e| path_error("open", path, e))?;
    let mut data = Vec::new();
    f.read_to_end(&mut data).map_err(|e| path_error("read", path, e))?;
    Ok(data)
}

/// rename moves a file: Go's `os.Rename`, its error a `*os.LinkError`.
pub fn rename(from: &Path, to: &Path) -> crate::Result<()> {
    fs::rename(from, to).map_err(|e| {
        let msg = format!("rename {} {}: {}", from.display(), to.display(), errno_words(&e));
        crate::wrap(e, msg)
    })
}

/// chmod sets a file's mode: Go's `os.Chmod`.
pub fn chmod(path: &Path, mode: u32) -> crate::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|e| path_error("chmod", path, e))
}

/// read_dir is a folder's entries sorted by name: Go's `os.ReadDir`.
pub fn read_dir(path: &Path) -> crate::Result<Vec<fs::DirEntry>> {
    let mut entries = Vec::new();
    for e in fs::read_dir(path).map_err(|e| path_error("open", path, e))? {
        entries.push(e.map_err(|e| path_error("readdirent", path, e))?);
    }
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}

/// dir is the folder a file is in: Go's `filepath.Dir`, "." for none.
pub fn dir(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// mkdir_all makes a folder and the ones it's in, as Go's `os.MkdirAll`
/// does: nothing when it's there, the first that can't be made named in
/// the error.
pub fn mkdir_all(path: &Path, mode: u32) -> crate::Result<()> {
    // Fast path: if we can tell whether path is a directory or file, stop with success or error.
    match fs::metadata(path) {
        Ok(m) if m.is_dir() => return Ok(()),
        Ok(_) => return Err(path_error("mkdir", path, io::Error::from_raw_os_error(libc::ENOTDIR))),
        Err(_) => {}
    }
    // Slow path: make sure parent exists and then call Mkdir for path.
    let b = path.as_os_str().as_bytes();
    let mut i = b.len();
    while i > 0 && b[i - 1] == b'/' {
        i -= 1;
    }
    while i > 0 && b[i - 1] != b'/' {
        i -= 1;
    }
    let parent = &b[..i.saturating_sub(1)];
    if !parent.is_empty() {
        mkdir_all(Path::new(std::ffi::OsStr::from_bytes(parent)), mode)?;
    }
    // Parent now exists; invoke Mkdir and use its result.
    if let Err(e) = fs::DirBuilder::new().mode(mode).create(path) {
        // Handle arguments like "foo/." by
        // double-checking that directory doesn't exist.
        if fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
            return Ok(());
        }
        return Err(path_error("mkdir", path, e));
    }
    Ok(())
}

/// create_temp makes a new file in dir, named by pattern with a random
/// number for its last "*", readable and writable by its owner only: Go's
/// `os.CreateTemp`.
pub fn create_temp(dir: &Path, pattern: &str) -> crate::Result<(File, PathBuf)> {
    let (prefix, suffix) = match pattern.rfind('*') {
        Some(i) => (&pattern[..i], &pattern[i + 1..]),
        None => (pattern, ""),
    };
    let prefix = dir.join(prefix).as_os_str().to_owned();
    for _ in 0..10000 {
        let mut name = prefix.clone();
        name.push(next_random());
        name.push(suffix);
        let name = PathBuf::from(name);
        match fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&name) {
            Ok(f) => return Ok((f, name)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(path_error("open", &name, e)),
        }
    }
    let path = format!("{}*{suffix}", Path::new(&prefix).display());
    let err = io::Error::new(io::ErrorKind::AlreadyExists, "file already exists");
    Err(crate::wrap(err, format!("createtemp {path}: file already exists")))
}

/// next_random is a random number for a temporary file's name, as Go's
/// CreateTemp takes one: a 32-bit one in decimal.
fn next_random() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNT.fetch_add(1, Ordering::Relaxed));
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    (h.finish() as u32).to_string()
}

/// write_file writes data to a file, made with mode 0644 or emptied first:
/// Go's `os.WriteFile(path, data, 0o644)`.
pub fn write_file(path: &Path, data: &[u8]) -> crate::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(path)
        .map_err(|e| path_error("open", path, e))?;
    f.write_all(data).map_err(|e| path_error("write", path, e))?;
    Ok(())
}

/// base is the last element of a slash-separated path: Go's `path.Base`
/// (and `filepath.Base` on this Mac). "." for "", "/" for slashes only.
pub fn base(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        return "/".to_string();
    }
    match p.rfind('/') {
        Some(i) => p[i + 1..].to_string(),
        None => p.to_string(),
    }
}

/// join is name in dir: Go's `filepath.Join(dir, name)` for a name that's
/// one element, "." and "/" being dir itself.
pub fn join(dir: &Path, name: &str) -> PathBuf {
    match name {
        "." | "/" | "" => dir.to_path_buf(),
        _ => dir.join(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_errors_say_what_go_says() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.json");
        let err = read_file(&missing).unwrap_err();
        assert_eq!(err.to_string(), format!("open {}: no such file or directory", missing.display()));
        assert!(is_not_exist(&err));
        let file = tmp.path().join("file");
        fs::write(&file, "x").unwrap();
        // Go names the file in the way, the first thing that isn't a folder
        let err = mkdir_all(&file.join("a/b"), 0o755).unwrap_err();
        assert_eq!(err.to_string(), format!("mkdir {}: not a directory", file.display()));
        mkdir_all(&tmp.path().join("a/b/c/"), 0o755).unwrap();
        assert!(tmp.path().join("a/b/c").is_dir());
        let (_, name) = create_temp(tmp.path(), ".x.json-*").unwrap();
        let base = name.file_name().unwrap().to_str().unwrap();
        assert!(base.starts_with(".x.json-") && base[8..].parse::<u32>().is_ok(), "{base}");
        assert_eq!(fs::metadata(&name).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn base_is_paths_last_element() {
        assert_eq!(base("/etc/packages/g/site-1.0.zip"), "site-1.0.zip");
        assert_eq!(base("site.zip"), "site.zip");
        assert_eq!(base("/a/b/"), "b");
        assert_eq!(base(""), ".");
        assert_eq!(base("///"), "/");
    }
}
