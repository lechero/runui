//! Shared command identity, search and preparation. Discovery is passive;
//! preparing a selection never starts a process or reads private shell history.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use runui_favorites::Favorite;
use runui_project::{Discovery, Source};
use runui_runs::{Run, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod search;
pub mod usage;

pub use search::{Hit, Query};

/// Context keeps execution paths separate from display labels and scope.
#[derive(Clone, Debug)]
pub struct Context {
    pub project: String,
    pub checkout: String,
    pub invocation_dir: String,
    pub environment: Environment,
}

impl Context {
    pub fn new(project: &str, checkout: &str) -> Self {
        Self {
            project: canonical(project),
            checkout: canonical(checkout),
            invocation_dir: canonical(project),
            environment: Environment::current(),
        }
    }
}

/// Environment is supplied by a caller. Checking it does not activate tools.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub path: Vec<PathBuf>,
    pub project_wrapper: bool,
    pub shell: Vec<String>,
}

impl Environment {
    pub fn current() -> Self {
        let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        Self { path, project_wrapper: false, shell: vec!["/bin/sh".into(), "-c".into()] }
    }

    pub fn with_shell(mut self, shell: &[String]) -> Self {
        self.shell = shell.to_vec();
        self.project_wrapper = shell.iter().any(|part| Path::new(part).file_name().is_some_and(|name| name == "mise"));
        self
    }

    fn availability(&self, program: &str, dir: &str) -> Availability {
        if self.project_wrapper {
            return Availability::ProjectEnvironment;
        }
        let executable =
            |path: &Path| fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        let found = if program.contains('/') {
            executable(&Path::new(dir).join(program))
        } else {
            self.path.iter().any(|path| executable(&path.join(program)))
        };
        if found { Availability::OnPath } else { Availability::Missing }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    OnPath,
    Missing,
    ProjectEnvironment,
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Command,
    Favorite,
    Run,
    Recent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Declared,
    Generated,
    Saved,
    Execution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variables {
    None,
    Assignments,
    MavenProperties,
}

/// Recipe retains literal tool arguments. Shell text is deliberate for saved
/// commands and prior invocations; it is never reconstructed by shell splitting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Recipe {
    Tool { argv: Vec<String>, separator: bool, variables: Variables, shell: Vec<String> },
    Shell { command: String, shell: Vec<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub fingerprint: String,
    pub kind: Kind,
    pub origin: Origin,
    pub name: String,
    pub description: String,
    pub source: String,
    pub project: String,
    pub project_directory: String,
    pub checkout: String,
    pub directory: String,
    pub relative_directory: String,
    pub entry_file: Option<String>,
    pub defining_file: Option<String>,
    pub defining_line: Option<usize>,
    pub variant: String,
    pub command: String,
    pub recipe: Recipe,
    pub availability: Availability,
    pub favorite: bool,
    pub favorite_names: Vec<String>,
    pub launches: u64,
    pub last_used: Option<DateTime<Utc>>,
    pub run_id: Option<String>,
    pub state: Option<String>,
    pub agent: Option<String>,
    pub output_available: bool,
    pub definition_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub path: String,
    pub message: String,
    pub source: Option<String>,
    pub kind: String,
}

/// Snapshot is immutable to callers: identity and preparation share the same
/// entries, rather than independently matching shell strings in each frontend.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    entries: Vec<Entry>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Snapshot {
    pub fn build(
        context: &Context,
        discovery: &Discovery,
        favorites: &[Favorite],
        runs: &[Run],
        usage: &usage::Data,
    ) -> Self {
        let project_name = Path::new(&context.project).file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut snapshot = Self::default();
        for file in &discovery.files {
            if let Some(error) = &file.err {
                snapshot.diagnostics.push(Diagnostic {
                    path: file.path.clone(),
                    message: error.clone(),
                    source: Some(file.source.as_str().into()),
                    kind: "parse".into(),
                });
            }
            for command in &file.commands {
                let directory = canonical(&file.dir);
                let relative_entry = relative(&context.checkout, &canonical(&file.path));
                let relative_directory = relative(&context.checkout, &directory);
                let id = format!(
                    "cmd-v1-{}",
                    digest(&[
                        &context.checkout,
                        command.source.as_str(),
                        &relative_entry,
                        &command.name,
                        &command.variant,
                        &relative_directory
                    ])
                );
                let variables = match command.source {
                    Source::Taskfile | Source::Make => Variables::Assignments,
                    Source::Maven => Variables::MavenProperties,
                    _ => Variables::None,
                };
                let recipe = Recipe::Tool {
                    argv: command.argv.clone(),
                    separator: command.args_separator,
                    variables,
                    shell: context.environment.shell.clone(),
                };
                let fingerprint = fingerprint(&directory, &recipe, &file.fingerprint);
                let availability = command
                    .argv
                    .first()
                    .map_or(Availability::Shell, |program| context.environment.availability(program, &directory));
                let mut entry = Entry {
                    id: id.clone(),
                    definition_id: Some(id),
                    fingerprint,
                    kind: Kind::Command,
                    origin: if command.generated { Origin::Generated } else { Origin::Declared },
                    name: command.name.clone(),
                    description: command.desc.clone(),
                    source: command.source.as_str().into(),
                    project: project_name.clone(),
                    project_directory: context.project.clone(),
                    checkout: context.checkout.clone(),
                    directory,
                    relative_directory,
                    entry_file: Some(canonical(&file.path)),
                    defining_file: Some(canonical(&command.defining_file)),
                    defining_line: command.defining_line,
                    variant: command.variant.clone(),
                    command: command.run.clone(),
                    recipe,
                    availability,
                    favorite: false,
                    favorite_names: Vec::new(),
                    launches: 0,
                    last_used: None,
                    run_id: None,
                    state: None,
                    agent: None,
                    output_available: false,
                };
                usage.decorate(&mut entry);
                snapshot.entries.push(entry);
            }
        }
        for problem in &discovery.diagnostics {
            snapshot.diagnostics.push(Diagnostic {
                path: problem.path.clone(),
                message: problem.message.clone(),
                source: problem.source.map(|source| source.as_str().into()),
                kind: "discovery".into(),
            });
        }
        for favorite in favorites {
            let directory = canonical(&Path::new(&context.project).join(&favorite.dir).to_string_lossy());
            let mut matched = false;
            for entry in &mut snapshot.entries {
                if entry.directory == directory
                    && entry.command == favorite.command
                    && (favorite.source.is_empty() || favorite.source == entry.source)
                {
                    entry.favorite = true;
                    entry.favorite_names.push(favorite.label().into());
                    matched = true;
                }
            }
            if matched {
                continue;
            }
            let recipe = Recipe::Shell { command: favorite.command.clone(), shell: context.environment.shell.clone() };
            let mut entry = Entry {
                id: format!("cmd-v1-{}", digest(&[&context.checkout, "favorite", &directory, &favorite.command])),
                fingerprint: fingerprint(&directory, &recipe, ""),
                kind: Kind::Favorite,
                origin: Origin::Saved,
                name: favorite.label().into(),
                description: String::new(),
                source: if favorite.source.is_empty() { "shell".into() } else { favorite.source.clone() },
                project: project_name.clone(),
                project_directory: context.project.clone(),
                checkout: context.checkout.clone(),
                relative_directory: relative(&context.checkout, &directory),
                directory,
                entry_file: None,
                defining_file: None,
                defining_line: None,
                variant: String::new(),
                command: favorite.command.clone(),
                recipe,
                availability: Availability::Shell,
                favorite: true,
                favorite_names: Vec::new(),
                launches: 0,
                last_used: None,
                run_id: None,
                state: None,
                agent: None,
                output_available: false,
                definition_id: None,
            };
            usage.decorate(&mut entry);
            snapshot.entries.push(entry);
        }
        let mut runs: Vec<_> =
            runs.iter().filter(|run| Path::new(&canonical(&run.dir)).starts_with(&context.project)).collect();
        runs.sort_by(|a, b| b.live().cmp(&a.live()).then(b.started.cmp(&a.started)).then(a.id.cmp(&b.id)));
        let retained: HashSet<_> = runs.iter().map(|run| run.id.as_str()).collect();
        for run in &runs {
            let state = run_state(run);
            let directory = canonical(&run.dir);
            for entry in
                snapshot.entries.iter_mut().filter(|entry| entry.command == run.command && entry.directory == directory)
            {
                if entry.run_id.is_none() {
                    entry.run_id = Some(run.id.clone());
                    entry.state = Some(state.clone());
                    entry.output_available = true;
                }
            }
            let recipe = Recipe::Shell { command: run.command.clone(), shell: run.shell.clone() };
            snapshot.entries.push(Entry {
                id: format!("run:{}", run.id),
                fingerprint: fingerprint(&directory, &recipe, ""),
                kind: Kind::Run,
                origin: Origin::Execution,
                name: run.label().into(),
                description: state.clone(),
                source: "run".into(),
                project: project_name.clone(),
                project_directory: context.project.clone(),
                checkout: context.checkout.clone(),
                directory: directory.clone(),
                relative_directory: relative(&context.checkout, &directory),
                entry_file: None,
                defining_file: None,
                defining_line: None,
                variant: String::new(),
                command: run.command.clone(),
                recipe,
                availability: Availability::Shell,
                favorite: false,
                favorite_names: Vec::new(),
                launches: 0,
                last_used: Some(run.started),
                run_id: Some(run.id.clone()),
                state: Some(state),
                agent: if run.agent.is_empty() { None } else { Some(run.agent.clone()) },
                output_available: true,
                definition_id: usage
                    .recent
                    .iter()
                    .find(|invocation| invocation.run_id == run.id)
                    .and_then(|invocation| invocation.definition_id.clone()),
            });
        }
        // Output retention and command recall are separate: old invocations
        // remain available even after their run folder has been pruned.
        for invocation in &usage.recent {
            if !Path::new(&invocation.directory).starts_with(&context.project) {
                continue;
            }
            if snapshot.entries.iter().any(|entry| {
                entry.kind == Kind::Recent
                    && entry.command == invocation.command
                    && entry.directory == invocation.directory
                    && entry.recipe == invocation.recipe
            }) {
                continue;
            }
            let mut entry = Entry {
                id: format!("recent:{}", invocation.run_id),
                fingerprint: fingerprint(&invocation.directory, &invocation.recipe, ""),
                kind: Kind::Recent,
                origin: Origin::Execution,
                name: invocation.name.clone(),
                description: "Previous invocation".into(),
                source: invocation
                    .source
                    .clone()
                    .or_else(|| {
                        invocation
                            .definition_id
                            .as_deref()
                            .and_then(|id| snapshot.get(id))
                            .map(|entry| entry.source.clone())
                    })
                    .unwrap_or_else(|| "shell".into()),
                project: project_name.clone(),
                project_directory: context.project.clone(),
                checkout: context.checkout.clone(),
                directory: invocation.directory.clone(),
                relative_directory: relative(&context.checkout, &invocation.directory),
                entry_file: None,
                defining_file: None,
                defining_line: None,
                variant: String::new(),
                command: invocation.command.clone(),
                recipe: invocation.recipe.clone(),
                availability: Availability::Shell,
                favorite: false,
                favorite_names: Vec::new(),
                launches: 0,
                last_used: Some(invocation.started),
                run_id: Some(invocation.run_id.clone()),
                state: invocation.state.clone(),
                agent: invocation.agent.clone(),
                output_available: retained.contains(invocation.run_id.as_str()),
                definition_id: invocation.definition_id.clone(),
            };
            usage.decorate(&mut entry);
            snapshot.entries.push(entry);
        }
        let mut identities = BTreeMap::<&str, usize>::new();
        for entry in &snapshot.entries {
            *identities.entry(&entry.id).or_default() += 1;
        }
        for (id, count) in identities {
            if count > 1 {
                let entry = snapshot.entries.iter().find(|entry| entry.id == id).expect("identity came from an entry");
                snapshot.diagnostics.push(Diagnostic {path:entry.entry_file.clone().unwrap_or_else(||entry.directory.clone()),message:format!("ambiguous definition {} ({count} matching identities); correct the duplicate declarations before launching",entry.name),source:Some(entry.source.clone()),kind:"definition".into()});
            }
        }
        snapshot
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn search(&self, query: &Query) -> Vec<Hit<'_>> {
        search::search(&self.entries, query)
    }

    /// prepare validates the selected fingerprint against this fresh snapshot.
    /// Callers must refresh discovery before using an earlier selection to launch.
    pub fn prepare(&self, id: &str, expected: Option<&str>, inputs: &Inputs) -> Result<Prepared, PrepareError> {
        if self.entries.iter().filter(|entry| entry.id == id).count() > 1 {
            return Err(PrepareError::Ambiguous(id.into()));
        }
        let entry = self.get(id).ok_or_else(|| PrepareError::Unknown(id.into()))?;
        if entry.kind == Kind::Run {
            return Err(PrepareError::RunSelection);
        }
        if let Some(expected) = expected
            && entry.fingerprint != expected
        {
            return Err(PrepareError::Stale { expected: expected.into(), actual: entry.fingerprint.clone() });
        }
        let command = prepare_recipe(&entry.recipe, inputs)?;
        let shell = match &entry.recipe {
            Recipe::Shell { shell, .. } | Recipe::Tool { shell, .. } => shell.clone(),
        };
        Ok(Prepared {
            id: entry.id.clone(),
            fingerprint: entry.fingerprint.clone(),
            name: entry.name.clone(),
            checkout: entry.checkout.clone(),
            directory: entry.directory.clone(),
            command,
            shell,
            inputs: inputs.clone(),
            source: entry.source.clone(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prepared {
    pub id: String,
    pub fingerprint: String,
    pub name: String,
    pub checkout: String,
    pub directory: String,
    pub command: String,
    pub shell: Vec<String>,
    pub inputs: Inputs,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareError {
    Unknown(String),
    Ambiguous(String),
    Stale { expected: String, actual: String },
    RunSelection,
    Inputs(String),
}
impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ambiguous(id) => write!(f, "command {id} has duplicate definitions; fix the source before launching"),
            Self::Unknown(id) => write!(f, "command {id} no longer exists in this scope"),
            Self::Stale { .. } => {
                write!(f, "the command definition changed; inspect the current invocation before launching")
            }
            Self::RunSelection => write!(f, "this is a run; focus its output or explicitly repeat its invocation"),
            Self::Inputs(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for PrepareError {}

fn prepare_recipe(recipe: &Recipe, inputs: &Inputs) -> Result<String, PrepareError> {
    if inputs.args.iter().any(|arg| arg.contains('\0'))
        || inputs.variables.iter().any(|(key, value)| key.contains('\0') || value.contains('\0'))
    {
        return Err(PrepareError::Inputs("arguments cannot contain NUL bytes".into()));
    }
    match recipe {
        Recipe::Tool { argv, separator, variables, .. } => {
            if argv.iter().any(|arg| arg.contains('\0')) {
                return Err(PrepareError::Inputs("the definition contains a NUL byte".into()));
            }
            if argv.is_empty() {
                return Err(PrepareError::Inputs("the command has no executable".into()));
            }
            if !inputs.variables.is_empty() && *variables == Variables::None {
                return Err(PrepareError::Inputs("this command does not accept named variables".into()));
            }
            let mut args = argv.clone();
            for (name, value) in &inputs.variables {
                if !valid_variable(name) {
                    return Err(PrepareError::Inputs(format!("invalid variable name {name:?}")));
                }
                args.push(format!(
                    "{}{name}={value}",
                    if *variables == Variables::MavenProperties { "-D" } else { "" }
                ));
            }
            if *separator && !inputs.args.is_empty() {
                args.push("--".into());
            }
            args.extend(inputs.args.iter().cloned());
            Ok(args.iter().map(|arg| quote_arg(arg)).collect::<Vec<_>>().join(" "))
        }
        Recipe::Shell { command, .. } => {
            if command.contains('\0') {
                return Err(PrepareError::Inputs("the definition contains a NUL byte".into()));
            }
            if !inputs.variables.is_empty() {
                return Err(PrepareError::Inputs("raw shell commands do not accept tool variables".into()));
            }
            if inputs.args.is_empty() {
                return Ok(command.clone());
            }
            // A raw script's inputs are positional parameters. Appending text
            // could instead attach arguments to the last part of a pipeline.
            let args =
                [vec!["/bin/sh".into(), "-c".into(), command.clone(), "--".into()], inputs.args.clone()].concat();
            Ok(args.iter().map(|arg| quote_arg(arg)).collect::<Vec<_>>().join(" "))
        }
    }
}

fn valid_variable(name: &str) -> bool {
    let mut chars = name.bytes();
    chars.next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || b"_.-".contains(&ch))
}

pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && arg.bytes().all(|ch| ch.is_ascii_alphanumeric() || b"_./:@%+=,-".contains(&ch)) {
        arg.into()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

pub fn canonical(path: &str) -> String {
    runui_project::canonical_path(path)
}

pub fn same_directory(first: &str, second: &str) -> bool {
    first == second || canonical(first) == canonical(second)
}

pub fn relative_within(root: &str, path: &str) -> Option<String> {
    let inside = |relative: &str| relative != ".." && !relative.starts_with("../");
    if let Some(relative) = runui_core::golang::filepath::rel(root, path)
        && inside(&relative)
    {
        return Some(relative);
    }
    let relative = runui_core::golang::filepath::rel(&canonical(root), &canonical(path))?;
    inside(&relative).then_some(relative)
}

fn relative(root: &str, path: &str) -> String {
    runui_core::golang::filepath::rel(root, path).unwrap_or_else(|| path.into())
}
fn fingerprint(directory: &str, recipe: &Recipe, dependencies: &str) -> String {
    let recipe = serde_json::to_string(recipe).expect("a recipe contains only serializable strings and enums");
    format!("def-v1-{}", digest(&[directory, &recipe, dependencies]))
}
pub(crate) fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(crate) fn run_state(run: &Run) -> String {
    match run.state() {
        State::Starting | State::Running => "running",
        State::Lost => "lost",
        State::Stopped => "stopped",
        State::Exited if run.exit.unwrap_or(0) == 0 => "completed",
        State::Exited => "failed",
    }
    .into()
}

#[cfg(test)]
mod tests;
