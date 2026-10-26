//! A bounded discovery and the dependencies that can change its commands.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use runui_core::golang::{filepath, path_error, read_dir};
use sha2::{Digest, Sha256};

use crate::compose::{compose_default, compose_files};
use crate::makefile::MAKEFILE_NAMES;
use crate::taskfile::TASKFILE_NAMES;
use crate::{File, MAX_DEPTH, MAX_FILES, MAX_FOLDERS, SKIPPED, Source, first_file, has_project_file, is_file};

const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;

/// Diagnostic explains why discovery could not inspect every definition.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    pub source: Option<Source>,
    pub path: String,
    pub message: String,
}

impl Diagnostic {
    /// description names the problem relative to the selected project.
    pub fn description(&self, dir: &str) -> String {
        let path = filepath::rel(dir, &self.path).unwrap_or_else(|| self.path.clone());
        format!("{path}: {}", self.message)
    }
}

/// Discovery keeps partial results, their limitations, and a refresh check.
/// Checking dependencies reads metadata only; it does not parse project files.
#[derive(Clone, Debug, Default)]
pub struct Discovery {
    pub files: Vec<File>,
    pub diagnostics: Vec<Diagnostic>,
    /// Informational policy decisions do not imply incomplete discovery.
    pub notices: Vec<Diagnostic>,
    dependencies: BTreeMap<String, Option<Stamp>>,
}

impl Discovery {
    /// changed notices edits, replacements, removals and newly created definitions.
    pub fn changed(&self) -> bool {
        self.dependencies.iter().any(|(path, before)| stamp(path) != *before)
    }

    /// complete is false when a source failed or a discovery limit was reached.
    pub fn complete(&self) -> bool {
        self.diagnostics.is_empty() && self.files.iter().all(|file| file.err.is_none())
    }

    pub(crate) fn dependencies_for_legacy(&mut self, path: &str) {
        self.dependencies.entry(path.into()).or_insert_with(|| stamp(path));
    }

    pub(crate) fn revision(&self) -> String {
        format!("{:?}", self.dependencies)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    modified: (i64, i64),
    changed: (i64, i64),
    inode: u64,
    device: u64,
    size: u64,
    mode: u32,
    directory: bool,
}

fn stamp(path: &str) -> Option<Stamp> {
    fs::metadata(path).ok().map(|m| Stamp {
        modified: (m.mtime(), m.mtime_nsec()),
        changed: (m.ctime(), m.ctime_nsec()),
        inode: m.ino(),
        device: m.dev(),
        size: m.len(),
        mode: m.mode(),
        directory: m.is_dir(),
    })
}

/// Scan shares the read budget and dependencies across all source readers.
/// Cached bytes let one included file contribute several namespaces without
/// spending the file budget again or observing different contents in one scan.
pub(crate) struct Scan {
    root: String,
    dependencies: BTreeMap<String, Option<Stamp>>,
    attempted: HashSet<String>,
    contents: HashMap<String, Arc<[u8]>>,
    rejected: HashMap<String, String>,
    bytes_read: usize,
    pub(crate) diagnostics: Vec<Diagnostic>,
    notices: Vec<Diagnostic>,
    visited_folders: HashSet<String>,
    expansions: HashMap<Source, usize>,
    current_dependencies: Option<HashSet<String>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl Scan {
    pub(crate) fn new(root: &str) -> Self {
        Self {
            root: root.into(),
            dependencies: BTreeMap::new(),
            attempted: HashSet::new(),
            contents: HashMap::new(),
            rejected: HashMap::new(),
            bytes_read: 0,
            diagnostics: Vec::new(),
            notices: Vec::new(),
            visited_folders: HashSet::new(),
            expansions: HashMap::new(),
            current_dependencies: None,
            cancel: None,
        }
    }

    pub(crate) fn watch(&mut self, path: &str) {
        if self.cancelled() {
            return;
        }
        let path = filepath::clean(path);
        if let Some(current) = &mut self.current_dependencies {
            current.insert(path.clone());
        }
        self.dependencies.entry(path.clone()).or_insert_with(|| stamp(&path));
    }

    fn begin_definition(&mut self) {
        self.current_dependencies = Some(HashSet::new());
    }

    fn definition_fingerprint(&mut self) -> String {
        let mut paths: Vec<_> = self.current_dependencies.take().unwrap_or_default().into_iter().collect();
        paths.sort();
        let mut hash = Sha256::new();
        for path in paths {
            let canonical = canonical_path(&path);
            hash.update((canonical.len() as u64).to_be_bytes());
            hash.update(canonical.as_bytes());
            if let Some(data) = self.contents.get(&path) {
                hash.update(Sha256::digest(data));
            } else if self.dependencies.get(&path).and_then(|stamp| stamp.as_ref()).is_some_and(|stamp| stamp.directory)
            {
                // Directory entries trigger discovery refresh, but unrelated
                // build output does not change an existing command definition.
                hash.update(b"directory");
            } else {
                // Consulted lock/wrapper paths and missing include candidates
                // affect resolution even when no parser reads their contents.
                hash.update(format!("{:?}", self.dependencies.get(&path)));
            }
        }
        hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub(crate) fn problem(&mut self, source: Option<Source>, path: &str, message: impl Into<String>) {
        let problem = Diagnostic { source, path: path.into(), message: message.into() };
        if !self.diagnostics.contains(&problem) {
            self.diagnostics.push(problem);
        }
    }

    pub(crate) fn notice(&mut self, path: &str, message: impl Into<String>) {
        let notice = Diagnostic { source: None, path: path.into(), message: message.into() };
        if !self.notices.contains(&notice) {
            self.notices.push(notice);
        }
    }

    pub(crate) fn enter_folder(&mut self, path: &str) -> bool {
        if self.cancelled() {
            return false;
        }
        if !self.visited_folders.contains(path) && self.visited_folders.len() >= MAX_FOLDERS {
            self.problem(None, &self.root.clone(), format!("discovery stopped at the {MAX_FOLDERS}-folder limit"));
            return false;
        }
        self.visited_folders.insert(path.into());
        self.watch(path);
        self.watch(&filepath::join(&[path, ".git"]));
        true
    }

    pub(crate) fn expand(&mut self, source: Source, path: &str) -> bool {
        if self.cancelled() {
            return false;
        }
        let count = self.expansions.entry(source).or_default();
        if *count == 1000 {
            self.problem(Some(source), path, format!("{} expansion limit reached", source.label()));
            return false;
        }
        *count += 1;
        true
    }

    pub(crate) fn can_read(&mut self, path: &str) -> bool {
        if self.cancelled() {
            self.problem(None, &self.root.clone(), "discovery was cancelled");
            return false;
        }
        if self.attempted.contains(&filepath::clean(path)) || self.attempted.len() < MAX_FILES {
            return true;
        }
        self.problem(None, &self.root.clone(), format!("discovery stopped at the {MAX_FILES}-file limit"));
        false
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|cancel| cancel.load(Ordering::Relaxed))
    }

    pub(crate) fn read(&mut self, path: &str) -> anyhow::Result<Arc<[u8]>> {
        let path = filepath::clean(path);
        self.watch(&path);
        if let Some(data) = self.contents.get(&path) {
            return Ok(data.clone());
        }
        if let Some(error) = self.rejected.get(&path) {
            anyhow::bail!("{error}");
        }
        if !self.can_read(&path) {
            anyhow::bail!("discovery file limit reached");
        }
        self.attempted.insert(path.clone());
        let remaining = MAX_TOTAL_BYTES.saturating_sub(self.bytes_read);
        let limit = MAX_SOURCE_BYTES.min(remaining);
        if let Ok(metadata) = fs::metadata(&path) {
            anyhow::ensure!(metadata.is_file(), "source is not a regular file: {path}");
        }
        let file = fs::File::open(&path).map_err(|error| path_error("open", Path::new(&path), error))?;
        anyhow::ensure!(file.metadata()?.is_file(), "source is not a regular file: {path}");
        let mut bytes = Vec::new();
        file.take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| path_error("read", Path::new(&path), error))?;
        self.bytes_read = self.bytes_read.saturating_add(bytes.len());
        if bytes.len() > limit {
            let error = if limit < MAX_SOURCE_BYTES {
                "discovery stopped at the 16-MiB source-byte limit"
            } else {
                "source exceeds the 2-MiB file limit"
            };
            self.rejected.insert(path.clone(), error.into());
            self.problem(None, &path, error);
            anyhow::bail!("{error}");
        }
        let data: Arc<[u8]> = bytes.into();
        self.contents.insert(path, data.clone());
        Ok(data)
    }
}

/// discover reads a bounded project snapshot. It never evaluates project code.
/// stop is the repository root used to resolve package managers and wrappers.
pub fn discover(dir: &str, stop: &str) -> Discovery {
    discover_inner(dir, stop, None)
}

/// Background indexing cancels obsolete work between directory/file reads.
pub fn discover_cancellable(dir: &str, stop: &str, cancel: Arc<AtomicBool>) -> Discovery {
    discover_inner(dir, stop, Some(cancel))
}

fn discover_inner(dir: &str, stop: &str, cancel: Option<Arc<AtomicBool>>) -> Discovery {
    let dir = filepath::clean(dir);
    let mut scan = Scan::new(&dir);
    scan.cancel = cancel;
    if scan.cancelled() {
        scan.problem(None, &dir, "discovery was cancelled");
    }
    scan.watch(&dir);
    scan.watch(&filepath::join(&[stop, ".git"]));
    // Readers report errors below. Cache selected-folder definitions before
    // workspace membership spends the shared read budget.
    for (_, path, _) in candidates(&Folder { path: dir.clone(), rel: String::new() }, false, &mut scan) {
        let _ = scan.read(&path);
    }
    let members = crate::workspace::declaration(&dir, &mut scan);
    let mut cargo_workspace = crate::cargo::declaration(&dir, &mut scan);
    let mut ignores = crate::workspace::IgnoreRules::new(stop, &mut scan);
    let deep = has_project_file(&dir) || crate::exists(&filepath::join(&[stop, ".git"]));
    let mut folders = if let Some(members) = &members {
        crate::workspace::watch_missing_prefixes(&dir, members, &mut scan);
        folders(&dir, true, Some(members), true, &mut ignores, &mut scan)
    } else {
        Vec::new()
    };
    if let Some(workspace) = &mut cargo_workspace {
        folders.extend(workspace.folders(&mut ignores, &mut scan));
    }
    folders.extend(self::folders(&dir, deep, members.as_ref(), false, &mut ignores, &mut scan));
    let mut cargo_workspaces: Vec<_> = cargo_workspace.into_iter().collect();
    let mut expanded: HashSet<_> = cargo_workspaces.iter().map(|ws| canonical_path(&ws.root)).collect();
    let mut queued: HashSet<_> = folders.iter().map(|folder| canonical_path(&folder.path)).collect();
    let mut at = 0;
    while at < folders.len() && !scan.cancelled() {
        let path = folders[at].path.clone();
        at += 1;
        if expanded.contains(&canonical_path(&path)) {
            continue;
        }
        if let Some(mut workspace) = crate::cargo::declaration(&path, &mut scan) {
            expanded.insert(canonical_path(&path));
            for folder in workspace.folders(&mut ignores, &mut scan) {
                if queued.insert(canonical_path(&folder.path)) {
                    folders.push(folder);
                }
            }
            cargo_workspaces.push(workspace);
        }
    }
    let mut unique = HashSet::new();
    folders.retain(|folder| unique.insert(canonical_path(&folder.path)));
    for folder in &mut folders {
        let relative = filepath::rel(&canonical_path(&dir), &canonical_path(&folder.path)).unwrap_or_default();
        folder.rel = if relative == "." { String::new() } else { relative };
    }
    folders.sort_by(|a, b| a.rel.cmp(&b.rel));
    let mut files = Vec::new();
    let mut included = HashSet::new();
    for folder in folders {
        if scan.cancelled() {
            break;
        }
        let cargo_only =
            members.as_ref().is_some_and(|members| !folder.rel.is_empty() && members.excluded(&folder.rel));
        for (source, path, label) in candidates(&folder, cargo_only, &mut scan) {
            if let Some(members) = &members
                && !folder.rel.is_empty()
                && ((members.excluded(&folder.rel) && source != Source::Cargo)
                    || (source == Source::Npm && !members.contains(&folder.rel)))
            {
                continue;
            }
            let cargo_workspace = cargo_workspaces
                .iter()
                .filter(|workspace| {
                    Path::new(&canonical_path(&folder.path)).starts_with(canonical_path(&workspace.root))
                })
                .max_by_key(|workspace| workspace.root.len());
            if source == Source::Cargo && cargo_workspace.is_some_and(|workspace| !workspace.contains(&folder.path)) {
                continue;
            }
            if source == Source::Taskfile && included.contains(&identity(&path)) {
                continue;
            }
            if !scan.can_read(&path) {
                continue;
            }
            scan.begin_definition();
            let (commands, err) = match source {
                Source::Taskfile => {
                    let (commands, more, err) = crate::taskfile::read_tracked(&path, &mut scan);
                    included.extend(more);
                    (commands, err)
                }
                Source::Npm => crate::npm::read_tracked(&path, stop, &mut scan),
                Source::Make => crate::makefile::read_tracked(&path, &mut scan),
                Source::Compose => crate::compose::read_tracked(&path, label != folder.rel, &mut scan),
                Source::Maven => crate::maven::read_tracked(&path, stop, &mut scan),
                Source::Composer => crate::composer::read_tracked(&path, &folder.path, stop, &mut scan),
                Source::Cargo => crate::cargo::read_tracked(&path, stop, cargo_workspace, &mut scan),
            };
            files.push(File {
                source,
                path,
                dir: folder.path.clone(),
                rel: folder.rel.clone(),
                label,
                commands,
                err: err.map(|err| err.to_string()),
                fingerprint: scan.definition_fingerprint(),
            });
        }
    }
    files.retain(|f| !(f.source == Source::Taskfile && included.contains(&identity(&f.path))));
    Discovery { files, diagnostics: scan.diagnostics, notices: scan.notices, dependencies: scan.dependencies }
}

/// identity detects include cycles even through a symbolic link.
pub(crate) fn identity(path: &str) -> String {
    canonical_path(path)
}

/// canonical_path resolves existing ancestors too, so a missing candidate
/// reached through a symlink has the same identity when it appears later.
pub fn canonical_path(path: &str) -> String {
    let absolute = filepath::abs(path).unwrap_or_else(|_| filepath::clean(path));
    let mut candidate = PathBuf::from(&absolute);
    let mut missing = Vec::new();
    loop {
        if let Ok(mut physical) = fs::canonicalize(&candidate) {
            for component in missing.into_iter().rev() {
                physical.push(component);
            }
            return physical.to_string_lossy().into_owned();
        }
        let Some(component) = candidate.file_name().map(|name| name.to_os_string()) else {
            break;
        };
        missing.push(component);
        if !candidate.pop() {
            break;
        }
    }
    absolute
}

pub(crate) struct Folder {
    pub(crate) path: String,
    pub(crate) rel: String,
}

/// A literal path dependency uses the same boundaries as a declared glob,
/// without recursively scanning unrelated folders along its ancestors.
pub(crate) fn declared_folder(
    root: &str,
    path: &str,
    ignores: &mut crate::workspace::IgnoreRules,
    scan: &mut Scan,
) -> Option<Folder> {
    let lexical_root = filepath::clean(root);
    let root = canonical_path(root);
    let lexical = filepath::clean(path);
    let physical = canonical_path(path);
    if !Path::new(&physical).starts_with(&root) {
        scan.problem(Some(Source::Cargo), path, "workspace path dependency is outside the workspace boundary");
        return None;
    }
    let Some(relative) = filepath::rel(&lexical_root, &lexical).filter(|relative| !relative.starts_with("..")) else {
        scan.problem(Some(Source::Cargo), path, "workspace path dependency crosses a symlink or boundary");
        return None;
    };
    let mut cursor = PathBuf::from(&lexical_root);
    for part in Path::new(&relative).components() {
        let std::path::Component::Normal(name) = part else {
            continue;
        };
        cursor.push(name);
        let current = cursor.to_string_lossy();
        scan.watch(&current);
        if name == ".git" || SKIPPED.iter().any(|skipped| name == *skipped) {
            scan.problem(Some(Source::Cargo), path, "workspace path dependency enters a dependency/generated folder");
            return None;
        }
        if fs::symlink_metadata(&cursor).is_ok_and(|m| m.file_type().is_symlink()) {
            scan.problem(
                Some(Source::Cargo),
                path,
                "workspace path dependency crosses a symlink; select it explicitly",
            );
            return None;
        }
        let git = cursor.join(".git");
        scan.watch(&git.to_string_lossy());
        if git.exists() {
            scan.notice(path, "nested repository was not traversed; select or register it explicitly");
            return None;
        }
        let (local, git) = ignores.ignored(&current, scan);
        if local {
            return None;
        }
        if git {
            scan.notice(
                path,
                "declared workspace member included despite a repository ignore; .runuiignore can exclude it",
            );
        }
        if !scan.enter_folder(&current) {
            return None;
        }
    }
    Some(Folder { path: physical, rel: relative })
}

pub(crate) fn folders(
    dir: &str,
    deep: bool,
    members: Option<&crate::workspace::Members>,
    declared: bool,
    ignores: &mut crate::workspace::IgnoreRules,
    scan: &mut Scan,
) -> Vec<Folder> {
    let mut out = vec![Folder { path: dir.into(), rel: String::new() }];
    let mut level = vec![(dir.to_string(), String::new())];
    scan.enter_folder(dir);
    let max_depth = if declared { 64 } else { MAX_DEPTH };
    // Read the last level too, so an unreadable directory is not an empty project.
    for depth in 0..=if deep { max_depth } else { 0 } {
        let mut next = Vec::new();
        for (path, rel) in &level {
            if scan.cancelled() {
                break;
            }
            scan.watch(path);
            let entries = match read_dir(Path::new(path)) {
                Ok(entries) => entries,
                Err(err) => {
                    scan.problem(None, path, err.to_string());
                    continue;
                }
            };
            if !deep {
                continue;
            }
            for entry in entries {
                if scan.cancelled() {
                    break;
                }
                let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
                if name == ".git" || SKIPPED.contains(&name.as_str()) {
                    if let Some(rules) = members.filter(|_| declared)
                        && rules.may_descend(&filepath::join(&[rel, &name]))
                        && rules.names_directory(&name)
                    {
                        scan.problem(Some(rules.source), &rules.manifest, format!("workspace pattern reaches excluded dependency/generated folder {name:?}; this folder was not traversed"));
                    }
                    continue;
                }
                if name.starts_with('.') && !declared {
                    continue;
                }
                let relative = filepath::join(&[rel, &name]);
                if members.is_some_and(|rules| rules.excluded(&relative)) {
                    continue;
                }
                let kind = match entry.file_type() {
                    Ok(kind) => kind,
                    Err(err) => {
                        scan.problem(None, path, err.to_string());
                        continue;
                    }
                };
                if !kind.is_dir() {
                    if kind.is_symlink() && declared && members.is_some_and(|rules| rules.may_descend(&relative)) {
                        scan.problem(
                            None,
                            &filepath::join(&[path, &name]),
                            "declared workspace member is a symlink; select it explicitly instead",
                        );
                    }
                    continue;
                }
                let child = filepath::join(&[path, &name]);
                if declared && members.is_some_and(|rules| !rules.may_descend(&relative)) {
                    continue;
                }
                scan.watch(&filepath::join(&[&child, ".git"]));
                if crate::exists(&filepath::join(&[&child, ".git"])) {
                    scan.notice(
                        &child,
                        "nested repository was not traversed; select or register that checkout explicitly",
                    );
                    continue;
                }
                let (local_ignored, git_ignored) = ignores.ignored(&child, scan);
                if local_ignored {
                    continue;
                }
                if git_ignored {
                    if !declared {
                        continue;
                    }
                    if members.is_some_and(|rules| rules.contains(&relative)) {
                        scan.notice(&child, "declared workspace member included despite a repository ignore; .runuiignore can exclude it");
                    }
                }
                if depth == max_depth {
                    if !scan.visited_folders.contains(&child) {
                        scan.problem(
                            None,
                            dir,
                            format!("incidental folders below depth {max_depth} were not inspected"),
                        );
                    }
                    continue;
                }
                if !scan.enter_folder(&child) {
                    continue;
                }
                next.push((child, relative));
            }
        }
        out.extend(next.iter().map(|(path, rel)| Folder { path: path.clone(), rel: rel.clone() }));
        level = next;
        if level.is_empty() {
            break;
        }
    }
    out[1..].sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn candidates(folder: &Folder, cargo_only: bool, scan: &mut Scan) -> Vec<(Source, String, String)> {
    let mut out = Vec::new();
    if cargo_only {
        let file = filepath::join(&[&folder.path, "Cargo.toml"]);
        if is_file(&file) {
            out.push((Source::Cargo, file, folder.rel.clone()));
        }
        return out;
    }
    if let Some(file) = first_file(&folder.path, &TASKFILE_NAMES) {
        out.push((Source::Taskfile, file, folder.rel.clone()));
    }
    let file = filepath::join(&[&folder.path, "package.json"]);
    if is_file(&file) {
        out.push((Source::Npm, file, folder.rel.clone()));
    }
    if let Some(file) = first_file(&folder.path, &MAKEFILE_NAMES) {
        out.push((Source::Make, file, folder.rel.clone()));
    }
    for (i, file) in compose_files(&folder.path).into_iter().enumerate() {
        let base = filepath::base(&file);
        let label =
            if i > 0 || !compose_default(&base) { filepath::join(&[&folder.rel, &base]) } else { folder.rel.clone() };
        out.push((Source::Compose, file, label));
    }
    let file = filepath::join(&[&folder.path, "pom.xml"]);
    if is_file(&file) {
        out.push((Source::Maven, file, folder.rel.clone()));
    }
    let file = filepath::join(&[&folder.path, "Cargo.toml"]);
    if is_file(&file) {
        out.push((Source::Cargo, file, folder.rel.clone()));
    }
    match crate::composer::selection(&folder.path, scan) {
        Ok(Some((file, _))) => out.push((Source::Composer, file, folder.rel.clone())),
        Ok(None) => {
            let file = filepath::join(&[&folder.path, "composer.json"]);
            if is_file(&file) {
                out.push((Source::Composer, file, folder.rel.clone()));
            }
        }
        Err(error) => {
            scan.problem(Some(Source::Composer), &filepath::join(&[&folder.path, ".runui.json"]), error.to_string())
        }
    }
    out
}

#[cfg(test)]
mod tests;
