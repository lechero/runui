//! A bounded discovery and the dependencies that can change its commands.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use runui_core::golang::{filepath, read_dir, read_file};

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
}

fn stamp(path: &str) -> Option<Stamp> {
    fs::metadata(path).ok().map(|m| Stamp {
        modified: (m.mtime(), m.mtime_nsec()),
        changed: (m.ctime(), m.ctime_nsec()),
        inode: m.ino(),
        device: m.dev(),
        size: m.len(),
        mode: m.mode(),
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
    expansions: HashMap<Source, usize>,
}

impl Scan {
    pub(crate) fn new(root: &str) -> Self {
        Self {
            root: root.into(),
            dependencies: BTreeMap::new(),
            attempted: HashSet::new(),
            contents: HashMap::new(),
            diagnostics: Vec::new(),
            expansions: HashMap::new(),
        }
    }

    pub(crate) fn watch(&mut self, path: &str) {
        let path = filepath::clean(path);
        self.dependencies.entry(path.clone()).or_insert_with(|| stamp(&path));
    }

    pub(crate) fn problem(&mut self, source: Option<Source>, path: &str, message: impl Into<String>) {
        let problem = Diagnostic { source, path: path.into(), message: message.into() };
        if !self.diagnostics.contains(&problem) {
            self.diagnostics.push(problem);
        }
    }

    pub(crate) fn expand(&mut self, source: Source, path: &str) -> bool {
        let count = self.expansions.entry(source).or_default();
        if *count == 1000 {
            self.problem(Some(source), path, format!("{} expansion limit reached", source.label()));
            return false;
        }
        *count += 1;
        true
    }

    pub(crate) fn can_read(&mut self, path: &str) -> bool {
        if self.attempted.contains(&filepath::clean(path)) || self.attempted.len() < MAX_FILES {
            return true;
        }
        self.problem(None, &self.root.clone(), format!("discovery stopped at the {MAX_FILES}-file limit"));
        false
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
    let dir = filepath::clean(dir);
    let mut scan = Scan::new(&dir);
    scan.watch(&dir);
    scan.watch(&filepath::join(&[stop, ".git"]));
    let deep = has_project_file(&dir) || crate::exists(&filepath::join(&[stop, ".git"]));
    let folders = folders(&dir, deep, &mut scan);
    let mut files = Vec::new();
    let mut included = HashSet::new();
    for folder in folders {
        for (source, path, label) in candidates(&folder) {
            if source == Source::Taskfile && included.contains(&identity(&path)) {
                continue;
            }
            if !scan.can_read(&path) {
                continue;
            }
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
            });
        }
    }
    files.retain(|f| !(f.source == Source::Taskfile && included.contains(&identity(&f.path))));
    Discovery { files, diagnostics: scan.diagnostics, dependencies: scan.dependencies }
}

/// identity detects include cycles even through a symbolic link.
pub(crate) fn identity(path: &str) -> String {
    fs::canonicalize(path)
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| filepath::abs(path).unwrap_or_else(|_| filepath::clean(path)))
}

struct Folder {
    path: String,
    rel: String,
}

fn folders(dir: &str, deep: bool, scan: &mut Scan) -> Vec<Folder> {
    let mut out = vec![Folder { path: dir.into(), rel: String::new() }];
    let mut level = vec![(dir.to_string(), String::new())];
    let mut count = 1;
    // Read the last level too, so an unreadable directory is not an empty project.
    for depth in 0..=if deep { MAX_DEPTH } else { 0 } {
        let mut next = Vec::new();
        for (path, rel) in &level {
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
                let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
                if name.starts_with('.') || SKIPPED.contains(&name.as_str()) {
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
                    continue;
                }
                if depth == MAX_DEPTH {
                    scan.problem(None, dir, format!("folders below depth {MAX_DEPTH} were not inspected"));
                    continue;
                }
                if count == MAX_FOLDERS {
                    scan.problem(None, dir, format!("discovery stopped at the {MAX_FOLDERS}-folder limit"));
                    continue;
                }
                count += 1;
                next.push((filepath::join(&[path, &name]), filepath::join(&[rel, &name])));
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
