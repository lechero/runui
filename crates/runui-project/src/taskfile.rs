//! A Taskfile's tasks, and the ones its includes bring.

use std::collections::HashSet;

use runui_core::golang::filepath;

use crate::discovery::{Scan, identity};
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
    let (commands, _, err) = read_tracked(path, &mut Scan::new(&filepath::dir(path)));
    (commands, err)
}

pub(crate) fn read_tracked(path: &str, scan: &mut Scan) -> (Vec<Command>, Vec<String>, Option<anyhow::Error>) {
    let mut active = HashSet::new();
    let mut included = HashSet::new();
    let (commands, err) = read(path, "", &mut active, &mut included, 0, scan);
    included.remove(&identity(path));
    let mut included: Vec<_> = included.into_iter().collect();
    included.sort();
    (commands, included, err)
}

fn read(
    path: &str,
    namespace: &str,
    active: &mut HashSet<String>,
    included: &mut HashSet<String>,
    depth: usize,
    scan: &mut Scan,
) -> (Vec<Command>, Option<anyhow::Error>) {
    let abs = match filepath::abs(path) {
        Ok(abs) => abs,
        Err(err) => return (Vec::new(), Some(err.into())),
    };
    scan.watch(&abs);
    if depth > INCLUDE_DEPTH {
        scan.problem(Some(Source::Taskfile), &abs, "Taskfile include depth limit reached");
        return (Vec::new(), None);
    }
    let id = identity(&abs);
    if active.contains(&id) {
        scan.problem(Some(Source::Taskfile), &abs, "Taskfile include cycle; this branch was skipped");
        return (Vec::new(), None);
    }
    if !scan.expand(Source::Taskfile, &abs) {
        return (Vec::new(), None);
    }
    active.insert(id.clone());
    let result = read_contents(&abs, namespace, active, included, depth, scan);
    active.remove(&id);
    result
}

fn read_contents(
    path: &str,
    namespace: &str,
    active: &mut HashSet<String>,
    included: &mut HashSet<String>,
    depth: usize,
    scan: &mut Scan,
) -> (Vec<Command>, Option<anyhow::Error>) {
    let data = match scan.read(path) {
        Ok(data) => data,
        Err(err) => return (Vec::new(), Some(err)),
    };
    let (doc, td) = match yaml::unmarshal_taskfile(&data) {
        Ok(doc) => doc,
        Err(err) => {
            return (Vec::new(), Some(runui_core::wrap(err.clone(), format!("{}: {err}", filepath::base(path)))));
        }
    };
    let mut commands = Vec::new();
    for (name, value) in doc.pairs(td.tasks) {
        let task = task_of(&doc, value);
        if task.internal {
            continue;
        }
        let full = format!("{namespace}{name}");
        commands.push(Command {
            source: Source::Taskfile,
            run: format!("task {}", quote(&full)),
            argv: vec!["task".into(), full.clone()],
            args_separator: true,
            defining_file: path.into(),
            defining_line: usize::try_from(doc.get(value).line).ok().filter(|line| *line > 0),
            variant: String::new(),
            generated: false,
            aliases: Vec::new(),
            hook: false,
            requirements: Vec::new(),
            cargo: None,
            cargo_alias: None,
            name: full,
            desc: desc(&task),
        });
    }
    for (name, value) in doc.pairs(td.includes) {
        let inc = include_of(&doc, value);
        if inc.internal || inc.taskfile.is_empty() {
            continue;
        }
        if inc.taskfile.contains("{{") || inc.taskfile.contains("://") {
            scan.problem(
                Some(Source::Taskfile),
                path,
                format!("include {name}: dynamic or remote Taskfiles are not inspected"),
            );
            continue;
        }
        let Some(file) = included_file(&filepath::dir(path), &inc.taskfile, scan) else {
            if !inc.optional {
                scan.problem(
                    Some(Source::Taskfile),
                    path,
                    format!("include {name}: Taskfile not found ({})", inc.taskfile),
                );
            }
            continue;
        };
        included.insert(identity(&file));
        let ns = if inc.flatten { namespace.to_string() } else { format!("{namespace}{name}:") };
        let (more, err) = read(&file, &ns, active, included, depth + 1, scan);
        if err.is_some() && !inc.optional {
            return (commands, err);
        }
        commands.extend(more);
    }
    (commands, None)
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
fn included_file(dir: &str, name: &str, scan: &mut Scan) -> Option<String> {
    let mut name = name.to_string();
    if let Some(rest) = name.strip_prefix("~/")
        && let Ok(home) = filepath::user_home_dir()
    {
        name = filepath::join(&[&home, rest]);
    }
    if !filepath::is_abs(&name) {
        name = filepath::join(&[dir, &name]);
    }
    scan.watch(&name);
    if is_file(&name) {
        return Some(name);
    }
    // An optional include may appear later, as a file or a directory.
    for candidate in TASKFILE_NAMES {
        scan.watch(&filepath::join(&[&name, candidate]));
    }
    if is_dir(&name) {
        return first_file(&name, &TASKFILE_NAMES);
    }
    None
}
