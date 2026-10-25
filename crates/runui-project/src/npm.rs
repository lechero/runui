//! package.json's scripts, run through the package manager the project
//! uses.

use std::path::Path;

use runui_core::golang::jsontext::{self, Decoder, Token};
use runui_core::golang::{filepath, read_file};

use crate::{Command, Source, is_file, quote};

/// LOCK_FILES name the package manager a folder uses by the lock file in it.
const LOCK_FILES: [(&str, &str); 6] = [
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("bun.lock", "bun"),
    ("bun.lockb", "bun"),
    ("package-lock.json", "npm"),
    ("npm-shrinkwrap.json", "npm"),
];

/// read_package_json is package.json's scripts in the order it has them,
/// each run through the package manager the project uses: the one its
/// packageManager field names, else the one whose lock file is in its
/// folder or above it, up to stop (a workspace's lock file is at its
/// root), else npm.
pub fn read_package_json(path: &str, stop: &str) -> (Vec<Command>, Option<anyhow::Error>) {
    let data = match read_file(Path::new(path)) {
        Ok(data) => data,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let (scripts, mut manager) = match read_scripts(&data) {
        Ok(r) => r,
        Err(e) => return (Vec::new(), Some(anyhow::anyhow!("package.json: {e}"))),
    };
    if manager.is_empty() {
        manager = lock_manager(&filepath::dir(path), stop);
    }
    let cmds = scripts
        .into_iter()
        .map(|(name, line)| Command {
            source: Source::Npm,
            run: format!("{manager} run {}", quote(&name)),
            name,
            desc: line,
        })
        .collect();
    (cmds, None)
}

/// read_scripts reads the scripts and the packageManager field, keeping
/// the scripts in order, which a map would lose.
fn read_scripts(data: &[u8]) -> Result<(Vec<(String, String)>, String), String> {
    let mut dec = Decoder::new(data);
    expect(&mut dec, b'{')?;
    let mut scripts = Vec::new();
    let mut manager = String::new();
    while dec.more() {
        let field = dec.decode_string().map_err(|e| e.to_string())?.unwrap_or_default();
        match field.as_str() {
            "scripts" => scripts = read_string_pairs(&mut dec).map_err(|e| format!("scripts: {e}"))?,
            "packageManager" => {
                if let Ok(pm) = dec.decode_string() {
                    manager = manager_name(&pm.unwrap_or_default()).to_string();
                }
            }
            _ => {
                dec.decode_raw().map_err(|e| e.to_string())?;
            }
        }
    }
    Ok((scripts, manager))
}

/// read_string_pairs reads an object of strings in order; a value that
/// isn't a string is skipped.
fn read_string_pairs(dec: &mut Decoder) -> Result<Vec<(String, String)>, String> {
    expect(dec, b'{')?;
    let mut out = Vec::new();
    while dec.more() {
        let name = dec.decode_string().map_err(|e| e.to_string())?.unwrap_or_default();
        let raw = dec.decode_raw().map_err(|e| e.to_string())?;
        if let Ok(line) = jsontext::unmarshal_string(raw) {
            out.push((name, line.unwrap_or_default()));
        }
    }
    dec.token().map_err(|e| e.to_string())?; // the closing brace
    Ok(out)
}

fn expect(dec: &mut Decoder, want: u8) -> Result<(), String> {
    match dec.token() {
        Err(e) => Err(e.to_string()),
        Ok(Token::Delim(d)) if d == want => Ok(()),
        Ok(_) => Err("not a JSON object".to_string()),
    }
}

/// manager_name is the package manager a packageManager field names
/// (pnpm@9.1.0); "" for one runui doesn't know.
fn manager_name(field: &str) -> &str {
    let name = field.split_once('@').map_or(field, |(name, _)| name);
    match name {
        "npm" | "pnpm" | "yarn" | "bun" => name,
        _ => "",
    }
}

/// lock_manager is the package manager of the nearest lock file, from dir
/// up to stop; npm when there's none.
fn lock_manager(dir: &str, stop: &str) -> String {
    let stop = filepath::clean(stop);
    let mut d = filepath::clean(dir);
    loop {
        for (file, manager) in LOCK_FILES {
            if is_file(&filepath::join(&[&d, file])) {
                return manager.to_string();
            }
        }
        let parent = filepath::dir(&d);
        if d == stop || parent == d || !d.starts_with(&stop) {
            return "npm".to_string();
        }
        d = parent;
    }
}
