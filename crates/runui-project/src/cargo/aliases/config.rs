//! Cargo's configuration order, include expansion and typed alias merging.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use toml::Value;

use super::{CargoAlias, CargoAliasContext, CargoAliasOrigin, CargoAliasValue};
use crate::discovery::Scan;
use crate::{Source, canonical_path};

pub(super) struct ConfigPath {
    pub path: String,
    pub project: bool,
    identity: String,
}

pub(super) fn paths(context: &CargoAliasContext, scan: &mut Scan) -> Vec<ConfigPath> {
    fn selected(directory: &Path, scan: &mut Scan) -> Option<String> {
        let old = directory.join("config");
        let new = directory.join("config.toml");
        scan.observe(&old.to_string_lossy());
        scan.observe(&new.to_string_lossy());
        [old, new].into_iter().find(|path| path.exists()).map(|path| path.to_string_lossy().into_owned())
    }
    let mut paths = Vec::new();
    if let Some(home) = &context.environment.cargo_home
        && let Some(path) = selected(&Path::new(&context.directory).join(home), scan)
    {
        let identity = canonical_path(&path);
        paths.push(ConfigPath { path, project: false, identity });
    }
    let ancestors: Vec<_> = Path::new(&context.directory).ancestors().take(65).collect();
    if ancestors.len() > 64 {
        scan.problem(Some(Source::Cargo), &context.directory, "Cargo config ancestor limit reached (64)");
    }
    for dir in ancestors.into_iter().take(64).rev() {
        if let Some(path) = selected(&dir.join(".cargo"), scan) {
            // Cargo does not merge CARGO_HOME twice when it is also an ancestor.
            let identity = canonical_path(&path);
            paths.retain(|previous| previous.identity != identity);
            paths.push(ConfigPath { path, project: dir.starts_with(&context.checkout), identity });
        }
    }
    paths
}

struct Document {
    includes: Vec<(String, bool)>,
    aliases: BTreeMap<String, CargoAliasValue>,
}

struct Loader<'a> {
    scan: &'a mut Scan,
    cache: BTreeMap<String, Option<Arc<Document>>>,
    active: BTreeSet<String>,
    seen: BTreeSet<String>,
    aliases: BTreeMap<String, CargoAlias>,
    invalid: BTreeSet<String>,
    applied_bytes: usize,
    expansions: usize,
}

pub(super) fn load(paths: &[ConfigPath], scan: &mut Scan) -> BTreeMap<String, CargoAlias> {
    let mut loader = Loader {
        scan,
        cache: BTreeMap::new(),
        active: BTreeSet::new(),
        seen: BTreeSet::new(),
        aliases: BTreeMap::new(),
        invalid: BTreeSet::new(),
        applied_bytes: 0,
        expansions: 0,
    };
    // Preserve the closest declarations when the wider context exceeds its
    // read budget. Merge order still follows native low-to-high precedence.
    if let Some(closest) = paths.last() {
        loader.document(&closest.path);
    }
    for path in paths {
        loader.seen.clear();
        loader.include(&path.path, false, path.project);
    }
    loader.aliases
}

impl Loader<'_> {
    fn document(&mut self, path: &str) -> Option<Arc<Document>> {
        if let Some(doc) = self.cache.get(path) {
            return doc.clone();
        }
        let result = (|| -> anyhow::Result<Document> {
            let bytes = self.scan.read(path)?;
            let mut doc: toml::Table = std::str::from_utf8(&bytes)?.parse::<toml::Table>().map_err(|error| {
                let line = error
                    .span()
                    .map(|span| bytes[..span.start.min(bytes.len())].iter().filter(|byte| **byte == b'\n').count() + 1)
                    .unwrap_or(1);
                // Cargo config may also hold credentials. Keep parser context
                // useful without copying an unrelated source-value excerpt.
                anyhow::anyhow!("invalid Cargo config TOML at line {line}: {}", error.message())
            })?;
            let mut includes = Vec::new();
            if let Some(value) = doc.remove("include") {
                let values =
                    value.as_array().ok_or_else(|| anyhow::anyhow!("Cargo config include must be an array"))?;
                for value in values {
                    let (name, optional) = match value {
                        Value::String(name) => (name.as_str(), false),
                        Value::Table(table) => {
                            let name = table
                                .get("path")
                                .and_then(Value::as_str)
                                .ok_or_else(|| anyhow::anyhow!("Cargo config include needs a path string"))?;
                            let optional = table
                                .get("optional")
                                .map(|value| {
                                    value.as_bool().ok_or_else(|| {
                                        anyhow::anyhow!("Cargo config include optional must be a boolean")
                                    })
                                })
                                .transpose()?
                                .unwrap_or(false);
                            (name, optional)
                        }
                        _ => anyhow::bail!("Cargo config include needs paths or path tables"),
                    };
                    anyhow::ensure!(
                        Path::new(name).extension().is_some_and(|extension| extension == "toml"),
                        "Cargo config includes must end in .toml"
                    );
                    includes.push((
                        Path::new(path).parent().unwrap_or(Path::new("/")).join(name).to_string_lossy().into_owned(),
                        optional,
                    ));
                }
            }
            let mut aliases = BTreeMap::new();
            if let Some(value) = doc.remove("alias") {
                let table = value.as_table().ok_or_else(|| anyhow::anyhow!("Cargo alias must be a table"))?;
                for (name, value) in table {
                    let value = match value {
                        Value::String(value) => CargoAliasValue::String(value.clone()),
                        Value::Array(values) => match values
                            .iter()
                            .map(|value| value.as_str().map(str::to_owned))
                            .collect::<Option<Vec<_>>>()
                        {
                            Some(values) => CargoAliasValue::Array(values),
                            None => {
                                self.scan.problem(
                                    Some(Source::Cargo),
                                    path,
                                    format!("Cargo alias {name:?} must contain strings"),
                                );
                                continue;
                            }
                        },
                        _ => {
                            self.scan.problem(
                                Some(Source::Cargo),
                                path,
                                format!("Cargo alias {name:?} must be a string or string array"),
                            );
                            continue;
                        }
                    };
                    aliases.insert(name.clone(), value);
                }
            }
            Ok(Document { includes, aliases })
        })();
        let doc = match result {
            Ok(doc) => Some(Arc::new(doc)),
            Err(error) => {
                self.scan.problem(Some(Source::Cargo), path, error.to_string());
                None
            }
        };
        self.cache.insert(path.into(), doc.clone());
        doc
    }

    fn include(&mut self, path: &str, optional: bool, project: bool) {
        self.scan.watch(path);
        if optional && !Path::new(path).exists() {
            return;
        }
        let identity = canonical_path(path);
        if !self.seen.insert(identity.clone()) {
            self.scan.problem(Some(Source::Cargo), path, "Cargo config include cycle or repeated included file");
            return;
        }
        if self.active.len() >= 16 || self.expansions >= 1000 {
            self.scan.problem(
                Some(Source::Cargo),
                path,
                "Cargo alias config include limit reached (16 deep / 1,000 expansions)",
            );
            return;
        }
        self.expansions += 1;
        let Some(doc) = self.document(path) else {
            return;
        };
        self.active.insert(identity.clone());
        for (child, optional) in &doc.includes {
            self.include(child, *optional, project);
        }
        self.active.remove(&identity);
        for (name, value) in &doc.aliases {
            self.merge(name, value, &identity, project);
        }
    }

    fn merge(&mut self, name: &str, value: &CargoAliasValue, path: &str, project: bool) {
        if self.invalid.contains(name) {
            return;
        }
        let bytes = value.bytes().saturating_add(name.len()).saturating_add(path.len()).saturating_add(64);
        if self.applied_bytes.saturating_add(bytes) > 1024 * 1024 {
            self.scan.problem(Some(Source::Cargo), path, "Cargo alias expansion reached the 1-MiB limit");
            return;
        }
        self.applied_bytes += bytes;
        if matches!(value, CargoAliasValue::Array(values) if values.len() > 1000) {
            self.scan.problem(Some(Source::Cargo), path, format!("Cargo alias {name:?} exceeds 1,000 arguments"));
            return;
        }
        let origin = CargoAliasOrigin { file: Some(path.into()), environment: None, project };
        if let Some(previous) = self.aliases.get_mut(name) {
            match (&mut previous.value, value) {
                (CargoAliasValue::String(old), CargoAliasValue::String(new)) => *old = new.clone(),
                (CargoAliasValue::Array(old), CargoAliasValue::Array(new))
                    if old.len().saturating_add(new.len()) <= 1000 =>
                {
                    old.extend(new.iter().cloned())
                }
                (CargoAliasValue::Array(_), CargoAliasValue::Array(_)) => {
                    self.scan.problem(
                        Some(Source::Cargo),
                        path,
                        format!("Cargo alias {name:?} exceeds 1,000 arguments"),
                    );
                    self.invalid.insert(name.into());
                    self.aliases.remove(name);
                    return;
                }
                _ => {
                    self.scan.problem(
                        Some(Source::Cargo),
                        path,
                        format!("Cargo alias {name:?} mixes string and array types across configuration files"),
                    );
                    self.invalid.insert(name.into());
                    self.aliases.remove(name);
                    return;
                }
            }
            previous.origins.push(origin);
        } else if self.aliases.len() < 1000 {
            self.aliases.insert(
                name.into(),
                CargoAlias {
                    name: name.into(),
                    value: value.clone(),
                    origins: vec![origin],
                    global: false,
                    diagnostics: Vec::new(),
                    disabled_reason: None,
                },
            );
        } else {
            self.scan.problem(Some(Source::Cargo), path, "Cargo alias limit reached (1,000)");
        }
    }
}

pub(super) fn builtin(name: &str) -> bool {
    [
        "add",
        "bench",
        "build",
        "check",
        "clean",
        "config",
        "doc",
        "fetch",
        "fix",
        "generate-lockfile",
        "help",
        "info",
        "init",
        "install",
        "locate-project",
        "login",
        "logout",
        "metadata",
        "new",
        "owner",
        "package",
        "pkgid",
        "publish",
        "read-manifest",
        "remove",
        "report",
        "run",
        "rustc",
        "rustdoc",
        "search",
        "test",
        "tree",
        "uninstall",
        "update",
        "vendor",
        "verify-project",
        "version",
        "yank",
    ]
    .contains(&name)
}
