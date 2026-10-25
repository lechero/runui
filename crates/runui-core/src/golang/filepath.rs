//! Go's path/filepath, os.Getwd and os.UserHomeDir as runui uses them, on
//! slash-separated paths, byte for byte: paths are joined and cleaned
//! lexically, so the same folders come out as the Go build names them.
//! Taken from aemui's aem-local (`gopath`), on strings: runui's paths are
//! Go strings, read from and written to JSON.

use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;

/// clean is Go's filepath.Clean: the shortest path naming the same file,
/// lexically.
pub fn clean(p: &str) -> String {
    let p = p.as_bytes();
    if p.is_empty() {
        return ".".to_string();
    }
    let rooted = p[0] == b'/';
    let n = p.len();
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let (mut r, mut dotdot) = (0, 0);
    if rooted {
        out.push(b'/');
        r = 1;
        dotdot = 1;
    }
    while r < n {
        if p[r] == b'/' {
            // empty path element
            r += 1;
        } else if p[r] == b'.' && (r + 1 == n || p[r + 1] == b'/') {
            // . element
            r += 1;
        } else if p[r] == b'.' && p[r + 1] == b'.' && (r + 2 == n || p[r + 2] == b'/') {
            // .. element: remove to last /
            r += 2;
            if out.len() > dotdot {
                // can backtrack
                let mut w = out.len() - 1;
                while w > dotdot && out[w] != b'/' {
                    w -= 1;
                }
                out.truncate(w);
            } else if !rooted {
                // cannot backtrack, but not rooted, so append .. element.
                if !out.is_empty() {
                    out.push(b'/');
                }
                out.extend_from_slice(b"..");
                dotdot = out.len();
            }
        } else {
            // real path element.
            // add slash if needed
            if (rooted && out.len() != 1) || (!rooted && !out.is_empty()) {
                out.push(b'/');
            }
            // copy element
            while r < n && p[r] != b'/' {
                out.push(p[r]);
                r += 1;
            }
        }
    }
    // Turn empty string into "."
    if out.is_empty() {
        return ".".to_string();
    }
    String::from_utf8(out).expect("cut at slashes, still UTF-8")
}

/// join is Go's filepath.Join (and path.Join): the elements joined with
/// slashes, the empty ones left out, and cleaned; "" for none.
pub fn join(elems: &[&str]) -> String {
    let parts: Vec<&str> = elems.iter().copied().filter(|e| !e.is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    clean(&parts.join("/"))
}

/// dir is Go's filepath.Dir: all but the last element, cleaned.
pub fn dir(p: &str) -> String {
    let i = p.rfind('/').map_or(0, |i| i + 1);
    clean(&p[..i])
}

/// base is Go's filepath.Base: the last element.
pub fn base(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let mut p = p;
    while p.len() > 1 && p.ends_with('/') {
        p = &p[..p.len() - 1];
    }
    if let Some(i) = p.rfind('/') {
        if p.len() == 1 {
            return "/".to_string();
        }
        p = &p[i + 1..];
    }
    if p.is_empty() {
        return "/".to_string();
    }
    p.to_string()
}

/// is_abs is Go's filepath.IsAbs: it starts at the root.
pub fn is_abs(p: &str) -> bool {
    p.starts_with('/')
}

/// user_home_dir is Go's os.UserHomeDir: $HOME, an error without it.
pub fn user_home_dir() -> crate::Result<String> {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => Ok(h),
        _ => Err(crate::anyhow!("$HOME is not defined")),
    }
}

/// getwd is Go's os.Getwd: $PWD when it names the current folder, the way
/// the shell spelled it, else the system's answer.
pub fn getwd() -> io::Result<String> {
    let dot = fs::metadata(".")?;
    if let Some(pwd) = std::env::var_os("PWD")
        && pwd.as_bytes().first() == Some(&b'/')
        && let Ok(d) = fs::metadata(&pwd)
        && d.dev() == dot.dev()
        && d.ino() == dot.ino()
        && let Some(pwd) = pwd.to_str()
    {
        return Ok(pwd.to_string());
    }
    let wd = std::env::current_dir()?;
    wd.to_str().map(str::to_string).ok_or_else(|| io::Error::other("the current folder's name isn't UTF-8"))
}

/// abs is Go's filepath.Abs: the path from the root, cleaned.
pub fn abs(p: &str) -> io::Result<String> {
    if is_abs(p) {
        return Ok(clean(p));
    }
    let wd = getwd()?;
    Ok(join(&[&wd, p]))
}

/// rel is Go's filepath.Rel: targ relative to base, lexically; None where
/// Go errs (one absolute and the other not, or a base with .. it can't
/// climb).
pub fn rel(basepath: &str, targpath: &str) -> Option<String> {
    let base = clean(basepath);
    let targ = clean(targpath);
    if targ == base {
        return Some(".".to_string());
    }
    let base: &[u8] = if base == "." { b"" } else { base.as_bytes() };
    let targ = targ.as_bytes();
    let base_slashed = base.first() == Some(&b'/');
    let targ_slashed = targ.first() == Some(&b'/');
    if base_slashed != targ_slashed {
        return None;
    }
    // Position base[b0:bi] and targ[t0:ti] at the first differing elements.
    let (bl, tl) = (base.len(), targ.len());
    let (mut b0, mut bi, mut t0, mut ti) = (0, 0, 0, 0);
    loop {
        while bi < bl && base[bi] != b'/' {
            bi += 1;
        }
        while ti < tl && targ[ti] != b'/' {
            ti += 1;
        }
        if targ[t0..ti] != base[b0..bi] {
            break;
        }
        if bi < bl {
            bi += 1;
        }
        if ti < tl {
            ti += 1;
        }
        b0 = bi;
        t0 = ti;
    }
    if &base[b0..bi] == b".." {
        return None;
    }
    let text = |b: Vec<u8>| String::from_utf8(b).expect("cut at slashes, still UTF-8");
    if b0 != bl {
        // Base elements left. Must go up before going down.
        let seps = base[b0..bl].iter().filter(|&&c| c == b'/').count();
        let mut buf = b"..".to_vec();
        for _ in 0..seps {
            buf.extend_from_slice(b"/..");
        }
        if t0 != tl {
            buf.push(b'/');
            buf.extend_from_slice(&targ[t0..]);
        }
        return Some(text(buf));
    }
    Some(text(targ[t0..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_cleaned_as_gos_are() {
        for (p, want) in [
            ("", "."),
            ("/", "/"),
            ("a/b/../c/./", "a/c"),
            ("../../a", "../../a"),
            ("/../a", "/a"),
            ("./apps/web/", "apps/web"),
            ("./", "."),
            ("a//b", "a/b"),
        ] {
            assert_eq!(clean(p), want, "{p}");
        }
        assert_eq!(join(&["/p", "apps/web"]), "/p/apps/web");
        assert_eq!(join(&["", "x"]), "x");
        assert_eq!(join(&["", ""]), "");
        assert_eq!(join(&["a", "."]), "a");
        assert_eq!(dir("/a/b/c.json"), "/a/b");
        assert_eq!(dir("c.json"), ".");
        assert_eq!(base("/a/b/"), "b");
        assert_eq!(base("/"), "/");
    }

    #[test]
    fn rel_is_gos() {
        for (b, t, want) in [
            ("/p", "/p", Some(".")),
            ("/p", "/p/apps/web", Some("apps/web")),
            ("/p/apps", "/p", Some("..")),
            ("/p/a", "/p/b/c", Some("../b/c")),
            ("/p", "/q", Some("../q")),
            ("/p", "x", None),
            ("..", "x", None),
        ] {
            assert_eq!(rel(b, t).as_deref(), want, "{b} {t}");
        }
    }
}
