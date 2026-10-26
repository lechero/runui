//! Composer owns script execution. We retain keys and aliases, never flatten
//! callbacks, arrays or @helpers into another shell language.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use runui_core::golang::{
    filepath,
    jsontext::{Decoder, Token},
};
use serde_json::Value;

use crate::{Command, Source, canonical_path, discovery::Scan, is_file, quote};

pub const EVENTS: &[&str] = &[
    "pre-install-cmd",
    "post-install-cmd",
    "pre-update-cmd",
    "post-update-cmd",
    "pre-status-cmd",
    "post-status-cmd",
    "pre-archive-cmd",
    "post-archive-cmd",
    "pre-autoload-dump",
    "post-autoload-dump",
    "post-root-package-install",
    "post-create-project-cmd",
    "pre-operations-exec",
    "pre-package-install",
    "post-package-install",
    "pre-package-update",
    "post-package-update",
    "pre-package-uninstall",
    "post-package-uninstall",
    "init",
    "command",
    "pre-file-download",
    "post-file-download",
    "pre-command-run",
    "pre-pool-create",
];

/// .runui.json is explicit per-folder context, not an ambient COMPOSER value.
/// Paths stay inside that folder; no PHAR is selected just because it exists.
pub(crate) fn selection(dir: &str, scan: &mut Scan) -> anyhow::Result<Option<(String, Option<String>)>> {
    let settings = filepath::join(&[dir, ".runui.json"]);
    scan.watch(&settings);
    if !is_file(&settings) {
        return Ok(None);
    }
    let bytes = scan.read(&settings)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        value.get("version").and_then(Value::as_u64) == Some(1),
        "unsupported .runui.json version (expected 1)"
    );
    let Some(composer) = value.get("composer") else { return Ok(None) };
    let composer = composer.as_object().ok_or_else(|| anyhow::anyhow!("composer context must be an object"))?;
    anyhow::ensure!(
        composer.keys().all(|key| matches!(key.as_str(), "manifest" | "phar")),
        "unknown Composer context field"
    );
    let read_path = |key: &str, default: Option<&str>| -> anyhow::Result<Option<String>> {
        let text = match composer.get(key) {
            Some(value) => Some(value.as_str().ok_or_else(|| anyhow::anyhow!("composer.{key} must be a path string"))?),
            None => default,
        };
        text.map(|text| context_path(dir, text)).transpose()
    };
    Ok(Some((read_path("manifest", Some("composer.json"))?.expect("default manifest"), read_path("phar", None)?)))
}

fn context_path(dir: &str, path: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!path.is_empty() && !path.contains('\0'), "Composer context path is empty or contains NUL");
    let root = canonical_path(dir);
    let path = canonical_path(&Path::new(dir).join(path).to_string_lossy());
    anyhow::ensure!(Path::new(&path).starts_with(&root), "Composer context path must stay inside {dir}");
    Ok(path)
}

pub fn read_composer(path: &str) -> (Vec<Command>, Option<anyhow::Error>) {
    read_tracked(path, &filepath::dir(path), &filepath::dir(path), &mut Scan::new(&filepath::dir(path)))
}

pub(crate) fn read_tracked(
    path: &str,
    dir: &str,
    stop: &str,
    scan: &mut Scan,
) -> (Vec<Command>, Option<anyhow::Error>) {
    let result = selection(dir, scan)
        .and_then(|selection| read(path, dir, stop, selection.as_ref().and_then(|(_, phar)| phar.as_deref()), scan));
    match result {
        Ok(commands) => (commands, None),
        Err(error) => (Vec::new(), Some(error)),
    }
}

fn read(path: &str, dir: &str, stop: &str, phar: Option<&str>, scan: &mut Scan) -> anyhow::Result<Vec<Command>> {
    let settings = filepath::join(&[dir, ".runui.json"]);
    scan.watch(&settings);
    let bytes = scan.read(path)?;
    let fields = ordered_object(&bytes)?;
    let mut scripts = Vec::new();
    let mut descriptions = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    for (key, raw) in fields {
        match key.as_str() {
            "scripts" => scripts = ordered_object(&raw)?,
            "scripts-descriptions" => {
                for (key, value) in metadata(&raw, "scripts-descriptions", path, scan) {
                    match serde_json::from_slice::<String>(&value) {
                        Ok(value) => {
                            descriptions.insert(key, value);
                        }
                        Err(error) => {
                            scan.problem(Some(Source::Composer), path, format!("script description {key:?}: {error}"))
                        }
                    }
                }
            }
            "scripts-aliases" => {
                for (key, value) in metadata(&raw, "scripts-aliases", path, scan) {
                    match serde_json::from_slice::<Vec<String>>(&value) {
                        Ok(value) => {
                            aliases.insert(key, value);
                        }
                        Err(error) => {
                            scan.problem(Some(Source::Composer), path, format!("script aliases {key:?}: {error}"))
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut cursor = PathBuf::from(dir);
    loop {
        for file in [".mise.toml", "mise.toml", ".tool-versions"] {
            scan.watch(&cursor.join(file).to_string_lossy());
        }
        if cursor == Path::new(stop) || !cursor.starts_with(stop) || !cursor.pop() {
            break;
        }
    }
    let mut tool = vec!["composer".into()];
    let mut requirements = vec!["composer".into(), "php".into()];
    if let Some(phar) = phar {
        scan.watch(phar);
        anyhow::ensure!(is_file(phar), "selected Composer PHAR does not exist: {phar}");
        anyhow::ensure!(
            Path::new(phar).extension().is_some_and(|value| value == "phar"),
            "selected Composer tool must be a .phar file"
        );
        tool = vec!["php".into(), phar.into()];
        requirements = vec!["php".into()];
    }
    let names: HashSet<_> = scripts.iter().map(|(name, _)| name.clone()).collect();
    let mut claimed = HashSet::new();
    let mut commands = Vec::new();
    for (name, raw) in scripts {
        let value: Value = serde_json::from_slice(&raw)?;
        let valid = match &value {
            Value::String(_) => true,
            Value::Array(items) => items.iter().all(Value::is_string),
            _ => false,
        };
        if !valid || name.is_empty() || name.contains('\0') {
            scan.problem(
                Some(Source::Composer),
                path,
                format!("script {name:?} must be a string or an array of strings"),
            );
            continue;
        }
        let mut native_aliases = Vec::new();
        for alias in aliases.remove(&name).unwrap_or_default() {
            if alias.is_empty() || alias.contains('\0') || names.contains(&alias) || !claimed.insert(alias.clone()) {
                scan.problem(
                    Some(Source::Composer),
                    path,
                    format!("ambiguous or invalid alias {alias:?} for script {name:?}"),
                );
                continue;
            }
            native_aliases.push(alias);
        }
        let mut argv = vec!["env".into(), format!("COMPOSER={}", canonical_path(path))];
        argv.extend(tool.iter().cloned());
        argv.extend(["run-script".into(), "--".into(), name.clone()]);
        commands.push(Command {
            source: Source::Composer,
            name: name.clone(),
            run: argv.iter().map(|value| quote(value)).collect::<Vec<_>>().join(" "),
            desc: descriptions.remove(&name).unwrap_or_else(|| match &value {
                Value::String(text) => text.clone(),
                _ => serde_json::to_string(&value).unwrap_or_default(),
            }),
            argv,
            args_separator: false,
            defining_file: path.into(),
            defining_line: None,
            variant: if EVENTS.contains(&name.as_str()) { "hook".into() } else { String::new() },
            generated: false,
            aliases: native_aliases,
            hook: EVENTS.contains(&name.as_str()),
            requirements: requirements.clone(),
            cargo: None,
            cargo_alias: None,
        });
    }
    for key in aliases.keys() {
        scan.problem(Some(Source::Composer), path, format!("aliases refer to absent script {key:?}"));
    }
    Ok(commands)
}

fn metadata(raw: &[u8], field: &str, path: &str, scan: &mut Scan) -> Vec<(String, Vec<u8>)> {
    match ordered_object(raw) {
        Ok(fields) => fields,
        Err(error) => {
            scan.problem(Some(Source::Composer), path, format!("{field}: {error}"));
            Vec::new()
        }
    }
}

fn ordered_object(bytes: &[u8]) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let mut decoder = Decoder::new(bytes);
    anyhow::ensure!(matches!(decoder.token()?, Token::Delim(b'{')), "expected a JSON object");
    let mut result: Vec<(String, Vec<u8>)> = Vec::new();
    while decoder.more() {
        let name = decoder.decode_string()?.unwrap_or_default();
        let value = decoder.decode_raw()?.to_vec();
        if let Some((_, previous)) = result.iter_mut().find(|(key, _)| key == &name) {
            *previous = value;
        } else {
            result.push((name, value));
        }
    }
    decoder.token()?;
    // Reject trailing JSON too; individual raw values have already been decoded.
    let _: Value = serde_json::from_slice(bytes)?;
    Ok(result)
}

#[cfg(test)]
mod tests;
