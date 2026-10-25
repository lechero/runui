//! A Taskfile's tasks, and the ones its includes bring.

use std::collections::HashSet;
use std::path::Path;

use runui_core::golang::{filepath, read_file};

use crate::yaml::{self, Doc, MAPPING_NODE, SCALAR_NODE};
use crate::{Command, Source, first_file, is_dir, is_file, quote};

/// TASKFILE_NAMES are the names Task looks for, in its order.
pub(crate) const TASKFILE_NAMES: [&str; 8] = [
    "Taskfile.yml",
    "taskfile.yml",
    "Taskfile.yaml",
    "taskfile.yaml",
    "Taskfile.dist.yml",
    "taskfile.dist.yml",
    "Taskfile.dist.yaml",
    "taskfile.dist.yaml",
];

/// INCLUDE_DEPTH is how deep included Taskfiles are followed.
const INCLUDE_DEPTH: usize = 5;

/// read_taskfile is a Taskfile's tasks in the order it has them, then the
/// ones its includes bring, named as Task names them (docs:build) and run
/// from the folder the Taskfile is in. Internal tasks are left out, as
/// task --list leaves them out. With an error, the tasks read before it.
pub fn read_taskfile(path: &str) -> (Vec<Command>, Option<anyhow::Error>) {
    let (cmds, _, err) = read_taskfile_includes(path);
    (cmds, err)
}

/// read_taskfile_includes is read_taskfile, and the Taskfiles it includes.
pub(crate) fn read_taskfile_includes(path: &str) -> (Vec<Command>, Vec<String>, Option<anyhow::Error>) {
    let mut seen = HashSet::new();
    let (cmds, err) = read(path, "", &mut seen, 0);
    let abs = filepath::abs(path).unwrap_or_default();
    let included = seen.into_iter().filter(|file| *file != abs).collect();
    (cmds, included, err)
}

fn read(
    path: &str,
    namespace: &str,
    seen: &mut HashSet<String>,
    depth: usize,
) -> (Vec<Command>, Option<anyhow::Error>) {
    let abs = match filepath::abs(path) {
        Ok(abs) => abs,
        Err(e) => return (Vec::new(), Some(e.into())),
    };
    if seen.contains(&abs) || depth > INCLUDE_DEPTH {
        return (Vec::new(), None);
    }
    seen.insert(abs.clone());
    let data = match read_file(Path::new(&abs)) {
        Ok(data) => data,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let (doc, td) = match yaml::unmarshal_taskfile(&data) {
        Ok(d) => d,
        Err(e) => return (Vec::new(), Some(runui_core::wrap(e.clone(), format!("{}: {e}", filepath::base(path))))),
    };
    let mut cmds = Vec::new();
    for (name, value) in doc.pairs(td.tasks) {
        let t = task_of(&doc, value);
        if t.internal {
            continue;
        }
        let full = format!("{namespace}{name}");
        cmds.push(Command {
            source: Source::Taskfile,
            run: format!("task {}", quote(&full)),
            name: full,
            desc: desc(&t),
        });
    }
    for (name, value) in doc.pairs(td.includes) {
        let inc = include_of(&doc, value);
        if inc.internal || inc.taskfile.is_empty() || inc.taskfile.contains("{{") || inc.taskfile.contains("://") {
            continue;
        }
        let Some(file) = included_file(&filepath::dir(&abs), &inc.taskfile) else {
            continue; // an optional include that isn't there, or a wrong path Task itself would report
        };
        let ns = if inc.flatten { namespace.to_string() } else { format!("{namespace}{name}:") };
        let (more, err) = read(&file, &ns, seen, depth + 1);
        if err.is_some() && !inc.optional {
            return (cmds, err);
        }
        cmds.extend(more);
    }
    (cmds, None)
}

/// desc is what the task says it does: its desc, else its summary's first
/// line.
fn desc(t: &yaml::Task) -> String {
    if !t.desc.is_empty() {
        return t.desc.trim().to_string();
    }
    let summary = t.summary.trim();
    summary.split_once('\n').map_or(summary, |(first, _)| first).to_string()
}

/// task_of reads a task, which may also be a command or a list of them
/// alone, with nothing to show.
fn task_of(doc: &Doc, n: yaml::NodeRef) -> yaml::Task {
    if doc.get(n).kind == MAPPING_NODE {
        return doc.decode_task(n).0;
    }
    yaml::Task::default()
}

/// include_of reads an include: a path alone, or a map with its taskfile.
fn include_of(doc: &Doc, n: yaml::NodeRef) -> yaml::Include {
    let node = doc.get(n);
    match node.kind {
        SCALAR_NODE => yaml::Include { taskfile: node.value.clone(), ..Default::default() },
        MAPPING_NODE => doc.decode_include(n).0,
        _ => yaml::Include::default(),
    }
}

/// included_file is the Taskfile an include names, relative to the folder
/// of the one including it: the file, or the Taskfile in the folder.
fn included_file(dir: &str, name: &str) -> Option<String> {
    let mut name = name.to_string();
    if let Some(rest) = name.strip_prefix("~/")
        && let Ok(home) = filepath::user_home_dir()
    {
        name = filepath::join(&[&home, rest]);
    }
    if !filepath::is_abs(&name) {
        name = filepath::join(&[dir, &name]);
    }
    if is_dir(&name) {
        return first_file(&name, &TASKFILE_NAMES);
    }
    if is_file(&name) {
        return Some(name);
    }
    None
}
