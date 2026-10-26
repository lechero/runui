//! Declared Cargo members and transitive local path dependencies.
use super::{document, strings};
use crate::discovery::{Folder, Scan};
use crate::workspace::{IgnoreRules, Members};
use crate::{Source, canonical_path, is_file};
use runui_core::golang::filepath;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use toml::Value;

pub(crate) struct Workspace {
    pub(crate) root: String,
    pub(crate) rules: Members,
    pub(crate) defaults: Option<Vec<String>>,
    pub(crate) members: BTreeSet<String>,
    pub(crate) default_paths: Vec<String>,
    root_package: bool,
}

pub(crate) fn declaration(dir: &str, scan: &mut Scan) -> Option<Workspace> {
    let path = filepath::join(&[dir, "Cargo.toml"]);
    scan.watch(&path);
    if !is_file(&path) {
        return None;
    }
    let result = (|| -> anyhow::Result<Option<Workspace>> {
        let doc = document(&path, scan)?;
        let Some(workspace) = doc.get("workspace") else {
            return Ok(None);
        };
        anyhow::ensure!(workspace.is_table(), "workspace must be a table");
        let includes = strings(workspace.get("members"), "workspace.members")?;
        let excludes = strings(workspace.get("exclude"), "workspace.exclude")?;
        let defaults = workspace
            .get("default-members")
            .map(|value| strings(Some(value), "workspace.default-members"))
            .transpose()?;
        Ok(Some(Workspace {
            root: dir.into(),
            rules: Members::new(Source::Cargo, path.clone(), includes, excludes, scan),
            defaults,
            members: BTreeSet::new(),
            default_paths: Vec::new(),
            root_package: doc.get("package").is_some(),
        }))
    })();
    match result {
        Ok(workspace) => workspace,
        Err(error) => {
            scan.problem(Some(Source::Cargo), &path, error.to_string());
            None
        }
    }
}

impl Workspace {
    /// Explicit member globs and transitive local path dependencies share the
    /// same traversal/file budgets. Excluded dependencies stay dependencies.
    pub(crate) fn folders(&mut self, ignores: &mut IgnoreRules, scan: &mut Scan) -> Vec<Folder> {
        crate::workspace::watch_missing_prefixes(&self.root, &self.rules, scan);
        let mut folders = crate::discovery::folders(&self.root, true, Some(&self.rules), true, ignores, scan);
        let mut queue: Vec<_> = folders
            .iter()
            .filter(|folder| self.rules.contains(&folder.rel))
            .map(|folder| folder.path.clone())
            .collect();
        let mut seen = HashSet::new();
        while let Some(dir) = queue.pop() {
            if scan.cancelled() {
                break;
            }
            if !seen.insert(canonical_path(&dir)) {
                continue;
            }
            let path = filepath::join(&[&dir, "Cargo.toml"]);
            scan.watch(&path);
            if !is_file(&path) {
                continue;
            }
            let doc = match document(&path, scan) {
                Ok(doc) => doc,
                Err(error) => {
                    scan.problem(Some(Source::Cargo), &path, error.to_string());
                    continue;
                }
            };
            if doc.get("package").is_none() {
                continue;
            }
            self.members.insert(canonical_path(&dir));
            let root_doc = document(&self.rules.manifest, scan).ok();
            for dependency in path_dependencies(&doc, root_doc.as_ref()) {
                let (base, path) = dependency;
                let base = if base { &self.root } else { &dir };
                let child = canonical_path(&filepath::join(&[base, &path]));
                let relative = filepath::rel(&canonical_path(&self.root), &child).unwrap_or_default();
                if self.rules.excluded(&relative) {
                    continue;
                }
                if seen.contains(&child) {
                    continue;
                }
                if let Some(folder) =
                    crate::discovery::declared_folder(&self.root, &filepath::join(&[base, &path]), ignores, scan)
                {
                    queue.push(folder.path.clone());
                    folders.push(folder);
                }
            }
        }
        if let Some(patterns) = &self.defaults {
            let defaults = Members::new(Source::Cargo, self.rules.manifest.clone(), patterns.clone(), Vec::new(), scan);
            for dir in &self.members {
                let relative = filepath::rel(&canonical_path(&self.root), dir).unwrap_or_default();
                let relative = if relative == "." { "" } else { &relative };
                if defaults.matches_declared(relative) {
                    self.default_paths.push(if relative.is_empty() { ".".into() } else { relative.into() });
                }
            }
            if self.default_paths.is_empty() && !patterns.is_empty() {
                scan.problem(
                    Some(Source::Cargo),
                    &self.rules.manifest,
                    "default-members did not select a discovered member",
                );
            }
        } else if self.root_package {
            self.default_paths.push(".".into());
        } else {
            self.default_paths =
                self.members.iter().filter_map(|dir| filepath::rel(&canonical_path(&self.root), dir)).collect();
        }
        self.default_paths.sort();
        folders
    }

    pub(crate) fn contains(&self, dir: &str) -> bool {
        canonical_path(dir) == canonical_path(&self.root) || self.members.contains(&canonical_path(dir))
    }
}

/// Bool marks paths inherited from workspace.dependencies, relative to the
/// workspace rather than the referring package. Names can be renamed keys.
fn path_dependencies(doc: &Value, root: Option<&Value>) -> Vec<(bool, String)> {
    let mut result = Vec::new();
    let mut tables = vec![doc];
    if let Some(targets) = doc.get("target").and_then(Value::as_table) {
        tables.extend(targets.values());
    }
    for table in tables {
        for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(dependencies) = table.get(key).and_then(Value::as_table) {
                for (name, value) in dependencies {
                    let inherited = value.get("workspace").and_then(Value::as_bool) == Some(true);
                    let value = if inherited {
                        root.and_then(|root| root.get("workspace"))
                            .and_then(|ws| ws.get("dependencies"))
                            .and_then(|deps| deps.get(name))
                            .unwrap_or(value)
                    } else {
                        value
                    };
                    if let Some(path) = value.get("path").and_then(Value::as_str) {
                        result.push((inherited, path.into()));
                    }
                }
            }
        }
    }
    result
}

/// Root lookup is passive and bounded; Cargo still validates membership when
/// explicitly invoked. A package excluded by the ancestor is independent.
pub(crate) fn enclosing(dir: &str, stop: &str, scan: &mut Scan) -> Option<Workspace> {
    let path = filepath::join(&[dir, "Cargo.toml"]);
    if is_file(&path)
        && let Ok(doc) = document(&path, scan)
        && let Some(selected) = doc.get("package").and_then(|package| package.get("workspace"))
    {
        let Some(selected) = selected.as_str() else {
            scan.problem(Some(Source::Cargo), &path, "package.workspace must be a path string");
            return None;
        };
        let root = canonical_path(&filepath::join(&[dir, selected]));
        if !Path::new(&root).starts_with(canonical_path(stop)) {
            scan.problem(Some(Source::Cargo), &path, "package.workspace is outside the checkout boundary");
            return None;
        }
        let workspace = declaration(&root, scan);
        if workspace.is_none() {
            scan.problem(Some(Source::Cargo), &path, "package.workspace does not select a workspace manifest");
        }
        return workspace;
    }

    let mut cursor = PathBuf::from(dir);
    for _ in 0..64 {
        if let Some(workspace) = declaration(&cursor.to_string_lossy(), scan) {
            let relative = filepath::rel(&canonical_path(&workspace.root), &canonical_path(dir)).unwrap_or_default();
            if !workspace.rules.excluded(&relative) {
                return Some(workspace);
            }
            return None;
        }
        if cursor == Path::new(stop) || !cursor.starts_with(stop) || !cursor.pop() {
            break;
        }
    }
    None
}
