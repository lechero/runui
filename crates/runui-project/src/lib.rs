//! The commands a project's files define: a Taskfile's tasks,
//! package.json's scripts, a Makefile's targets, a docker compose file's
//! services and a Maven pom's builds, each ready to run as you would type
//! it, in the project's folder and the folders under it, a monorepo's
//! packages and a Maven project's modules. Ported from
//! internal/runui/project.

use std::collections::HashSet;
use std::fs;
use std::sync::LazyLock;

use regex::Regex;
use runui_core::golang::filepath;

mod compose;
mod discovery;
mod makefile;
mod maven;
mod npm;
mod taskfile;
mod workspace;
// the ports of Go's libraries are kept whole, though runui reads only some of what they do
#[allow(dead_code)]
mod xml;
mod yaml;

pub use compose::read_compose;
pub use discovery::{Diagnostic, Discovery, canonical_path, discover, discover_cancellable};
pub use makefile::read_makefile;
pub use maven::read_pom;
pub use npm::read_package_json;
pub use taskfile::read_taskfile;
pub use workspace::workspace_root;

use compose::compose_files;
use makefile::MAKEFILE_NAMES;
use taskfile::TASKFILE_NAMES;

/// Source is the kind of file a command comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Taskfile,
    /// package.json; its scripts run through npm, pnpm, yarn or bun
    Npm,
    Make,
    /// docker compose
    Compose,
    Maven,
}

/// SOURCES are every kind of project file, in the order the Project
/// panel's tabs go.
pub const SOURCES: [Source; 5] = [Source::Taskfile, Source::Npm, Source::Make, Source::Compose, Source::Maven];

impl Source {
    /// as_str is the source as it's written down: task, npm, make,
    /// compose or maven (Go's `string(s)`).
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Taskfile => "task",
            Source::Npm => "npm",
            Source::Make => "make",
            Source::Compose => "compose",
            Source::Maven => "maven",
        }
    }

    /// label is what the source is called in a panel's tab.
    pub fn label(self) -> &'static str {
        match self {
            Source::Taskfile => "Taskfile",
            Source::Npm => "package.json",
            Source::Make => "Makefile",
            Source::Compose => "Compose",
            Source::Maven => "Maven",
        }
    }
}

/// label is what a source written down is called in a panel's tab: its
/// label, or the word itself for one runui doesn't know (Go's
/// `project.Source(s).Label()`).
pub fn label(source: &str) -> String {
    match SOURCES.iter().find(|s| s.as_str() == source) {
        Some(s) => s.label().to_string(),
        None => source.to_string(),
    }
}

/// Command is one thing a project file defines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub source: Source,
    /// the task, script or target
    pub name: String,
    /// the command line: task build, pnpm run dev, make test
    pub run: String,
    /// what it says it does: a task's desc, a script's own line, a target's ## comment
    pub desc: String,
    /// Tool invocation as literal arguments, including the executable.
    pub argv: Vec<String>,
    /// Arguments after the invocation need the runner's -- separator.
    pub args_separator: bool,
    /// The included file can differ from the entry file used to invoke it.
    pub defining_file: String,
    pub defining_line: Option<usize>,
    /// A provider's stable action variant, separate from its display name.
    pub variant: String,
    pub generated: bool,
}

/// File is one of the project's files and the commands in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub source: Source,
    pub path: String,
    /// the folder it's in, where its commands run
    pub dir: String,
    /// that folder from the project's, with slashes: "" for the project's own
    pub rel: String,
    /// what the Project panel puts before its commands: rel, and the
    /// file's name when it's not the one its kind is found by
    /// (docker-compose.dev.yml)
    pub label: String,
    pub commands: Vec<Command>,
    /// why it couldn't be read; the project's other files still are
    pub err: Option<String>,
    /// Execution dependencies captured during parsing, independent of row identity.
    pub fingerprint: String,
}

impl File {
    /// rel_path is the file from the project's folder: apps/web/package.json.
    pub fn rel_path(&self) -> String {
        filepath::join(&[&self.rel, &filepath::base(&self.path)])
    }
}

// How far under the project's folder its files are looked for, and how
// much is looked at: enough for a monorepo's apps/web or a Maven
// project's modules, not a home folder's every clone.
pub(crate) const MAX_DEPTH: usize = 3;
pub(crate) const MAX_FOLDERS: usize = 2000;
pub(crate) const MAX_FILES: usize = 150;

/// SKIPPED are the folders never looked in: what tools make and download
/// (a dependency's own package.json isn't the project's), besides the
/// hidden ones, .git among them.
const SKIPPED: [&str; 17] = [
    "node_modules",
    "bower_components",
    "jspm_packages",
    "vendor",
    "dist",
    "build",
    "out",
    "target",
    "coverage",
    "__pycache__",
    "venv",
    "site-packages",
    "Pods",
    "DerivedData",
    "test-results",
    "playwright-report",
    "storybook-static",
];

/// load reads the project's files: the folder's own, then, when it's a
/// project or in a repository, declared workspace members and incidental
/// folders under it three deep, in path order and with local ignore policy.
/// In each folder: its Taskfile (one
/// that another Taskfile includes is left to that one, as Task names its
/// tasks there), package.json, Makefile, docker compose files and pom.xml.
/// stop is the repository's root, where lock files and Maven's wrapper are
/// looked for up to.
pub fn load(dir: &str, stop: &str) -> Vec<File> {
    discover(dir, stop).files
}

/// root is the folder runui works in when it's opened in dir: the nearest
/// one, dir or above it, with a project file (a Taskfile, package.json, a
/// Makefile, a docker compose file or pom.xml), going no higher than the
/// repository's root or the home folder; dir itself when there's none. A
/// tmux popup opened in a project's src folder shows the project; one
/// opened in ~/projects doesn't show the home folder's Makefile.
pub fn root(dir: &str, home: &str) -> String {
    let dir = filepath::clean(dir);
    let mut d = dir.clone();
    loop {
        if has_project_file(&d) {
            return d;
        }
        let parent = filepath::dir(&d);
        if exists(&filepath::join(&[&d, ".git"])) || d == home || parent == home || parent == d {
            return dir;
        }
        d = parent;
    }
}

/// repo_root is the repository dir is in, found by its .git (a folder, or
/// a worktree's file); dir itself outside one.
pub fn repo_root(dir: &str) -> String {
    let mut d = filepath::clean(dir);
    loop {
        if exists(&filepath::join(&[&d, ".git"])) {
            return d;
        }
        let parent = filepath::dir(&d);
        if parent == d {
            return filepath::clean(dir);
        }
        d = parent;
    }
}

/// context_root is the checkout boundary, or the enclosing declared workspace
/// outside Git. A member opened from an unpacked workspace has the same IDs and
/// package-manager context as when opened from that workspace's root.
pub fn context_root(dir: &str, home: &str) -> String {
    let repo = repo_root(dir);
    if exists(&filepath::join(&[&repo, ".git"])) {
        return repo;
    }
    let stop = if !home.is_empty() && std::path::Path::new(dir).starts_with(home) { home } else { "/" };
    workspace_root(dir, stop)
}

/// changed is the legacy revision check. It rediscovers dependencies for
/// callers that kept only files; a retained Discovery checks metadata without
/// parsing again and should be preferred by long-lived callers.
pub fn changed(dir: &str, files: &[File]) -> String {
    let mut discovery = discover(dir, &repo_root(dir));
    for file in files {
        discovery.dependencies_for_legacy(&file.path);
    }
    discovery.revision()
}

pub(crate) fn has_project_file(dir: &str) -> bool {
    first_file(dir, &TASKFILE_NAMES).is_some()
        || is_file(&filepath::join(&[dir, "pnpm-workspace.yaml"]))
        || is_file(&filepath::join(&[dir, "package.json"]))
        || first_file(dir, &MAKEFILE_NAMES).is_some()
        || !compose_files(dir).is_empty()
        || is_file(&filepath::join(&[dir, "pom.xml"]))
}

/// first_file is the first of names that's a file in dir, as a path. The
/// folder's entries are matched by name, so on a Mac's disk, where
/// makefile opens Makefile too, the file is called as it's written.
pub(crate) fn first_file(dir: &str, names: &[&str]) -> Option<String> {
    let entries = fs::read_dir(dir).ok()?;
    let has: HashSet<String> = entries.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect();
    names
        .iter()
        .map(|name| (name, filepath::join(&[dir, name])))
        .find(|(name, path)| has.contains(**name) && is_file(path))
        .map(|(_, p)| p)
}

pub(crate) fn exists(path: &str) -> bool {
    fs::metadata(path).is_ok()
}

pub(crate) fn is_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

pub(crate) fn is_dir(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_dir())
}

/// PLAIN_WORD is what a shell takes as one word without quotes.
static PLAIN_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_./:@%+=,-]+$").expect("a regexp"));

/// quote is a task, script or target name as one word for sh.
pub(crate) fn quote(s: &str) -> String {
    if PLAIN_WORD.is_match(s) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests;
