//! A bounded discovery and the dependencies that can change its commands.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use runui_core::golang::{filepath, read_dir, read_file};
use sha2::{Digest, Sha256};

use crate::compose::{compose_default, compose_files};
use crate::makefile::MAKEFILE_NAMES;
use crate::taskfile::TASKFILE_NAMES;
use crate::{File, MAX_DEPTH, MAX_FILES, MAX_FOLDERS, SKIPPED, Source, first_file, has_project_file, is_file};

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

    fn enter_folder(&mut self, path: &str) -> bool {
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

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|cancel| cancel.load(Ordering::Relaxed))
    }

    pub(crate) fn read(&mut self, path: &str) -> anyhow::Result<Arc<[u8]>> {
        let path = filepath::clean(path);
        self.watch(&path);
        if let Some(data) = self.contents.get(&path) {
            return Ok(data.clone());
        }
        if !self.can_read(&path) {
            anyhow::bail!("discovery file limit reached");
        }
        self.attempted.insert(path.clone());
        let data: Arc<[u8]> = read_file(Path::new(&path))?.into();
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
    let members = crate::workspace::declaration(&dir, &mut scan);
    let mut ignores = crate::workspace::IgnoreRules::new(stop, &mut scan);
    let deep = has_project_file(&dir) || crate::exists(&filepath::join(&[stop, ".git"]));
    let mut folders = if let Some(members) = &members {
        crate::workspace::watch_missing_prefixes(&dir, members, &mut scan);
        folders(&dir, true, Some(members), true, &mut ignores, &mut scan)
    } else {
        Vec::new()
    };
    folders.extend(self::folders(&dir, deep, members.as_ref(), false, &mut ignores, &mut scan));
    folders.sort_by(|a, b| a.rel.cmp(&b.rel));
    folders.dedup_by(|a, b| a.path == b.path);
    let mut files = Vec::new();
    let mut included = HashSet::new();
    for folder in folders {
        if scan.cancelled() {
            break;
        }
        for (source, path, label) in candidates(&folder) {
            if let Some(members) = &members
                && !folder.rel.is_empty()
                && (members.excluded(&folder.rel) || (source == Source::Npm && !members.contains(&folder.rel)))
            {
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

struct Folder {
    path: String,
    rel: String,
}

fn folders(
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
                        scan.problem(Some(Source::Npm), &rules.manifest, format!("workspace pattern reaches excluded dependency/generated folder {name:?}; this folder was not traversed"));
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

fn candidates(folder: &Folder) -> Vec<(Source, String, String)> {
    let mut out = Vec::new();
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
    out
}

#[cfg(test)]
mod tests;
