//! Passive JavaScript workspace membership and local ignore policy. Patterns
//! choose folders; they never become shell commands or follow symlinks.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use glob::{MatchOptions, Pattern};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use runui_core::golang::filepath;

use crate::discovery::Scan;
use crate::yaml::{self, Doc, MAPPING_NODE, SCALAR_NODE, SEQUENCE_NODE, Tree};
use crate::{Source, is_file};

#[derive(Clone)]
struct MemberPattern {
    pattern: Pattern,
    prefix: PathBuf,
    depth: Option<usize>,
}

/// A declaration is present even with no members: an empty list must not
/// silently turn into a recursive glob or incidental package discovery.
#[derive(Clone)]
pub(crate) struct Members {
    pub(crate) source: Source,
    includes: Vec<MemberPattern>,
    excludes: Vec<Pattern>,
    pub(crate) manifest: String,
}

const MATCH: MatchOptions =
    MatchOptions { case_sensitive: true, require_literal_separator: true, require_literal_leading_dot: true };

impl Members {
    fn options(&self) -> MatchOptions {
        MatchOptions { require_literal_leading_dot: self.source != Source::Cargo, ..MATCH }
    }

    pub(crate) fn new(
        source: Source,
        manifest: String,
        includes: Vec<String>,
        excludes: Vec<String>,
        scan: &mut Scan,
    ) -> Self {
        let mut result = Self { source, includes: Vec::new(), excludes: Vec::new(), manifest };
        for (exclude, text) in
            includes.into_iter().map(|text| (false, text)).chain(excludes.into_iter().map(|text| (true, text)))
        {
            if result.includes.len() + result.excludes.len() >= 256 {
                scan.problem(Some(source), &result.manifest, "workspace pattern limit reached (256)");
                break;
            }
            match member_pattern(&text, source) {
                Ok(rule) if exclude => result.excludes.push(rule.pattern),
                Ok(rule) => result.includes.push(rule),
                Err(error) => scan.problem(Some(source), &result.manifest, error.to_string()),
            }
        }
        result
    }

    pub(crate) fn names_directory(&self, name: &str) -> bool {
        self.includes.iter().any(|rule| rule.pattern.as_str().split('/').any(|part| part == name))
    }
    pub(crate) fn contains(&self, relative: &str) -> bool {
        relative.is_empty() || (self.matches_declared(relative) && !self.excluded(relative))
    }

    pub(crate) fn matches_declared(&self, relative: &str) -> bool {
        self.includes.iter().any(|rule| rule.pattern.matches_with(relative, self.options()))
    }

    pub(crate) fn excluded(&self, relative: &str) -> bool {
        Path::new(relative)
            .ancestors()
            .filter(|p| !p.as_os_str().is_empty())
            .any(|part| self.excludes.iter().any(|pattern| pattern.matches_path_with(part, self.options())))
    }

    pub(crate) fn may_descend(&self, relative: &str) -> bool {
        let path = Path::new(relative);
        !self.excluded(relative)
            && self.includes.iter().any(|rule| {
                (path.starts_with(&rule.prefix) || rule.prefix.starts_with(path))
                    && rule.depth.is_none_or(|depth| path.components().count() <= depth)
                    && path.components().all(|part| {
                        let part = part.as_os_str().to_string_lossy();
                        self.source == Source::Cargo
                            || !part.starts_with('.')
                            || rule.pattern.as_str().split('/').any(|p| p == part)
                    })
            })
    }
}

pub(crate) fn declaration(dir: &str, scan: &mut Scan) -> Option<Members> {
    let pnpm = filepath::join(&[dir, "pnpm-workspace.yaml"]);
    let package = filepath::join(&[dir, "package.json"]);
    scan.watch(&pnpm);
    scan.watch(&package);
    let (manifest, patterns) = if is_file(&pnpm) {
        let patterns = scan.read(&pnpm).and_then(|bytes| pnpm_patterns(&bytes));
        (pnpm, patterns.map(Some))
    } else if is_file(&package) {
        // The ordinary source reader already reports malformed package JSON.
        // A valid manifest with an invalid workspace shape is a distinct error.
        let value =
            scan.read(&package).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())?;
        let patterns = package_patterns(&value);
        (package, patterns)
    } else {
        return None;
    };
    let patterns = match patterns {
        Ok(Some(patterns)) => patterns,
        Ok(None) => return None,
        Err(error) => {
            scan.problem(Some(Source::Npm), &manifest, format!("workspace declaration: {error}"));
            return None;
        }
    };
    let mut members = Members { source: Source::Npm, includes: Vec::new(), excludes: Vec::new(), manifest };
    for text in patterns {
        let (exclude, text) = text.strip_prefix('!').map_or((false, text.as_str()), |text| (true, text));
        let expanded = match expand_braces(text) {
            Ok(patterns) => patterns,
            Err(error) => {
                scan.problem(Some(Source::Npm), &members.manifest, error.to_string());
                continue;
            }
        };
        for text in expanded {
            if members.includes.len() + members.excludes.len() >= 256 {
                scan.problem(Some(Source::Npm), &members.manifest, "workspace pattern limit reached (256)");
                return Some(members);
            }
            match member_pattern(&text, Source::Npm) {
                Ok(rule) if exclude => members.excludes.push(rule.pattern),
                Ok(rule) => members.includes.push(rule),
                Err(error) => scan.problem(Some(Source::Npm), &members.manifest, error.to_string()),
            }
        }
    }
    Some(members)
}

fn pnpm_patterns(data: &[u8]) -> anyhow::Result<Vec<String>> {
    let mut tree = Tree::default();
    let root = yaml::unmarshal_doc(data, &mut tree)?;
    let Some(root) = root.and_then(|id| tree.get(id).content.first().copied()) else {
        return Ok(Vec::new());
    };
    anyhow::ensure!(tree.get(root).kind == MAPPING_NODE, "expected a YAML mapping");
    let doc = Doc { tree };
    let Some((_, value)) = doc.pairs(Some(root)).into_iter().find(|(key, _)| key == "packages") else {
        return Ok(Vec::new());
    };
    let node = doc.get(value);
    anyhow::ensure!(node.kind == SEQUENCE_NODE, "packages must be a sequence of strings");
    node.content
        .iter()
        .map(|id| {
            let item = doc.get(*id);
            anyhow::ensure!(
                item.kind == SCALAR_NODE && matches!(item.tag.as_str(), "!!str" | "tag:yaml.org,2002:str"),
                "packages must contain strings"
            );
            Ok(item.value.clone())
        })
        .collect()
}

fn package_patterns(value: &serde_json::Value) -> anyhow::Result<Option<Vec<String>>> {
    let Some(workspaces) = value.get("workspaces") else { return Ok(None) };
    // Yarn Classic also accepts the object form with packages/nohoist.
    let list = workspaces.get("packages").unwrap_or(workspaces);
    let list = list.as_array().ok_or_else(|| anyhow::anyhow!("workspaces must be an array or packages object"))?;
    list.iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("workspace patterns must be strings"))
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .map(Some)
}

fn member_pattern(text: &str, source: Source) -> anyhow::Result<MemberPattern> {
    anyhow::ensure!(
        !text.is_empty() && !text.contains('\0') && !Path::new(text).is_absolute(),
        "workspace pattern {text:?} must stay within the workspace root"
    );
    anyhow::ensure!(
        !text.contains('\\')
            && (source == Source::Cargo || (!text.contains('$') && !text.contains("(") && !text.contains(")"))),
        "unsupported dynamic or extended workspace pattern {text:?}"
    );
    let mut parts = Vec::new();
    for part in text.split('/').filter(|part| !part.is_empty() && *part != ".") {
        if part == ".." {
            let Some(previous) = parts.pop() else {
                anyhow::bail!("workspace pattern {text:?} escapes the workspace root")
            };
            anyhow::ensure!(!has_magic(previous), "cannot statically normalize workspace pattern {text:?}");
        } else {
            parts.push(part);
        }
    }
    let normalized = parts.join("/");
    let pattern = Pattern::new(&normalized)?;
    let prefix = parts.iter().take_while(|part| !has_magic(part)).collect::<PathBuf>();
    let depth = if parts.contains(&"**") { None } else { Some(parts.len()) };
    Ok(MemberPattern { pattern, prefix, depth })
}

fn has_magic(text: &str) -> bool {
    text.contains(['*', '?', '['])
}

fn expand_braces(text: &str) -> anyhow::Result<Vec<String>> {
    let mut out = vec![text.to_string()];
    for _ in 0..8 {
        let Some(index) = out.iter().position(|pattern| pattern.contains('{')) else { return Ok(out) };
        let pattern = out.remove(index);
        let start = pattern.find('{').unwrap_or(0);
        let end = pattern[start..]
            .find('}')
            .map(|end| start + end)
            .ok_or_else(|| anyhow::anyhow!("unclosed workspace pattern {pattern:?}"))?;
        let inner = &pattern[start + 1..end];
        anyhow::ensure!(!inner.contains('{') && inner.contains(','), "unsupported workspace brace pattern {pattern:?}");
        for part in inner.split(',') {
            out.push(format!("{}{part}{}", &pattern[..start], &pattern[end + 1..]));
            anyhow::ensure!(out.len() <= 64, "workspace brace expansion exceeds 64 patterns");
        }
    }
    anyhow::bail!("workspace brace expansion limit reached")
}

#[derive(Default)]
pub(crate) struct IgnoreRules {
    root: PathBuf,
    cache: BTreeMap<PathBuf, (Gitignore, Gitignore)>,
}

impl IgnoreRules {
    pub(crate) fn new(root: &str, scan: &mut Scan) -> Self {
        let mut rules = Self { root: PathBuf::from(root), ..Self::default() };
        let path = PathBuf::from(root);
        rules
            .cache
            .insert(path.clone(), (ignore_file(&path, ".runuiignore", scan), ignore_file(&path, ".gitignore", scan)));
        rules
    }

    /// Local .runuiignore overrides declared membership. Repository ignores
    /// affect incidental discovery; explicit members can be read with a notice.
    pub(crate) fn ignored(&mut self, path: &str, scan: &mut Scan) -> (bool, bool) {
        let path = Path::new(path);
        let Some(parent) = path.parent() else { return (false, false) };
        let mut ancestors: Vec<_> = parent.ancestors().take_while(|p| p.starts_with(&self.root)).collect();
        ancestors.reverse();
        let (mut local, mut git) = (false, false);
        for dir in ancestors {
            let rules = self
                .cache
                .entry(dir.into())
                .or_insert_with(|| (ignore_file(dir, ".runuiignore", scan), ignore_file(dir, ".gitignore", scan)));
            for (rule, ignored) in [(&rules.0, &mut local), (&rules.1, &mut git)] {
                let matched = rule.matched_path_or_any_parents(path, true);
                if !matched.is_none() {
                    *ignored = matched.is_ignore();
                }
            }
        }
        (local, git)
    }
}

fn ignore_file(dir: &Path, name: &str, scan: &mut Scan) -> Gitignore {
    let path = dir.join(name);
    let path_text = path.to_string_lossy();
    scan.watch(&path_text);
    let mut builder = GitignoreBuilder::new(dir);
    if is_file(&path_text) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(fs::metadata(&path)?.len() <= 64 * 1024, "ignore file exceeds 64 KiB");
            let bytes = scan.read(&path_text)?;
            for line in std::str::from_utf8(&bytes)?.lines() {
                builder.add_line(Some(path.clone()), line)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            scan.problem(None, &path_text, error.to_string());
        }
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Keep nearest-project selection distinct from its enclosing workspace.
/// Only local declared membership is consulted, up to the checkout boundary.
pub fn workspace_root(project: &str, checkout: &str) -> String {
    let project = filepath::clean(project);
    let checkout = filepath::clean(checkout);
    let mut scan = Scan::new(&project);
    if is_file(&filepath::join(&[&project, "Cargo.toml"]))
        && let Some(workspace) = crate::cargo::enclosing(&project, &checkout, &mut scan)
    {
        return workspace.root;
    }

    let mut directory = project.clone();
    for _ in 0..64 {
        if let Some(workspace) = crate::cargo::declaration(&directory, &mut scan) {
            let relative = filepath::rel(&directory, &project).unwrap_or_default();
            if !workspace.rules.excluded(&relative) {
                return directory;
            }
        }
        if let Some(members) = declaration(&directory, &mut scan) {
            let relative = filepath::rel(&directory, &project).unwrap_or_default();
            if relative == "." {
                return directory;
            }
            for part in Path::new(&relative).ancestors().filter(|part| !part.as_os_str().is_empty()) {
                if members.contains(&part.to_string_lossy())
                    && is_file(&Path::new(&directory).join(part).join("package.json").to_string_lossy())
                {
                    return directory;
                }
            }
        }
        let parent = filepath::dir(&directory);
        if directory == checkout || parent == directory || !Path::new(&parent).starts_with(&checkout) {
            break;
        }
        directory = parent;
    }
    project
}

/// A literal prefix can be missing today; watch every existing ancestor so
/// creating its next directory invalidates the snapshot too.
pub(crate) fn watch_missing_prefixes(root: &str, members: &Members, scan: &mut Scan) {
    let mut watched = BTreeSet::new();
    for rule in &members.includes {
        let mut path = PathBuf::from(root);
        for component in rule.prefix.components() {
            if let Component::Normal(name) = component {
                path.push(name);
            }
            if path
                .file_name()
                .is_some_and(|name| name == ".git" || crate::SKIPPED.iter().any(|skipped| name == *skipped))
            {
                break;
            }
            if watched.insert(path.clone()) {
                scan.watch(&path.to_string_lossy());
            }
            if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                scan.problem(
                    Some(members.source),
                    &path.to_string_lossy(),
                    "declared workspace member crosses a symlink; select it explicitly instead",
                );
                break;
            }
            let repository = path.join(".git");
            scan.watch(&repository.to_string_lossy());
            if repository.exists() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
