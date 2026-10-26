//! Shared command identity, search and preparation. Discovery is passive;
//! preparing a selection never starts a process or reads private shell history.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use runui_favorites::Favorite;
use runui_project::{Discovery, Source};
use runui_runs::{Run, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod index;
pub mod inspection;
pub mod registry;
mod search;
mod state;
pub mod usage;

pub use search::{Hit, Query};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Project,
    Workspace,
    Registered,
}

impl Scope {
    pub fn directory(self, project: &str, checkout: &str) -> Option<String> {
        match self {
            Self::Project => Some(project.into()),
            Self::Workspace => Some(runui_project::workspace_root(project, checkout)),
            Self::Registered => None,
        }
    }
}

/// Context keeps execution paths separate from display labels and scope.
#[derive(Clone, Debug)]
pub struct Context {
    pub project: String,
    pub checkout: String,
    pub invocation_dir: String,
    pub environment: Environment,
    pub cargo_global_aliases: bool,
}

impl Context {
    pub fn new(project: &str, checkout: &str) -> Self {
        Self {
            project: canonical(project),
            checkout: canonical(checkout),
            invocation_dir: canonical(project),
            environment: Environment::current(),
            cargo_global_aliases: false,
        }
    }

    /// Cargo selects a manifest explicitly and retains the user's invocation
    /// cwd. Registered commands opened from elsewhere use their own checkout.
    pub fn execution_directory(&self, source: Source, definition: &str) -> String {
        if source != Source::Cargo {
            return canonical(definition);
        }
        if Path::new(&self.invocation_dir).starts_with(&self.checkout) {
            canonical(&self.invocation_dir)
        } else {
            self.project.clone()
        }
    }
}

/// Environment is supplied by a caller. Checking it does not activate tools.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub path: Vec<PathBuf>,
    pub project_wrapper: bool,
    pub shell: Vec<String>,
    pub cargo_aliases: runui_project::CargoAliasEnvironment,
}

impl Environment {
    pub fn current() -> Self {
        let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        Self {
            path,
            project_wrapper: false,
            shell: vec!["/bin/sh".into(), "-c".into()],
            cargo_aliases: runui_project::CargoAliasEnvironment::current(),
        }
    }

    pub fn with_shell(mut self, shell: &[String]) -> Self {
        self.shell = shell.to_vec();
        self.project_wrapper = shell.iter().any(|part| Path::new(part).file_name().is_some_and(|name| name == "mise"));
        self
    }

    pub fn availability(&self, program: &str, dir: &str) -> Availability {
        if self.project_wrapper {
            return Availability::ProjectEnvironment;
        }
        if self.resolve(program, dir).is_some() { Availability::OnPath } else { Availability::Missing }
    }

    fn resolve(&self, program: &str, dir: &str) -> Option<String> {
        if self.project_wrapper {
            return None;
        }
        let executable =
            |path: &Path| fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        let candidates = if program.contains('/') {
            vec![Path::new(dir).join(program)]
        } else {
            self.path.iter().map(|path| Path::new(dir).join(path).join(program)).collect()
        };
        candidates.into_iter().find(|path| executable(path)).map(|path| canonical(&path.to_string_lossy()))
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

impl Availability {
    pub fn label(self) -> &'static str {
        match self {
            Self::OnPath => "on PATH",
            Self::Missing => "missing",
            Self::ProjectEnvironment => "project environment · unchecked",
            Self::Shell => "shell command",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Command,
    Hook,
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
    /// None preserves Composer's project/default timeout; 0 explicitly disables it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Requirement {
    pub program: String,
    pub availability: Availability,
    pub resolved_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub registry_id: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
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
    #[serde(default)]
    pub proximity: usize,
    pub entry_file: Option<String>,
    pub defining_file: Option<String>,
    pub defining_line: Option<usize>,
    pub variant: String,
    pub command: String,
    pub recipe: Recipe,
    pub availability: Availability,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub cargo: Option<runui_project::CargoDetails>,
    #[serde(default)]
    pub cargo_alias: Option<runui_project::CargoAlias>,
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

impl Entry {
    /// Cargo retains invocation cwd while its list location identifies the
    /// selected manifest. Other providers' source and execution folders agree.
    pub fn relative_source_directory(&self) -> String {
        self.cargo
            .as_ref()
            .map(|cargo| relative(&self.checkout, &cargo.manifest_directory))
            .unwrap_or_else(|| self.relative_directory.clone())
    }
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
    pub fn with_registration(mut self, project: &registry::Project, branch: Option<&str>) -> Self {
        for entry in &mut self.entries {
            entry.registry_id = Some(project.id.clone());
            entry.project = project.name.clone();
            entry.branch = branch.map(str::to_owned);
        }
        self
    }

    pub fn registered(
        indexed: &[index::Indexed],
        environment: &Environment,
        invocation: &str,
        favorites: &runui_favorites::Store,
        runs: &[Run],
        usage: &usage::Data,
        cargo_global_aliases: bool,
    ) -> Self {
        let mut result = Self::default();
        let mut execution_rows = BTreeMap::<String, usize>::new();
        for indexed in indexed {
            let project = &indexed.project;
            if indexed.pending {
                result.diagnostics.push(Diagnostic {
                    path: project.directory.clone(),
                    message: "checkout indexing is in progress; ready results remain available".into(),
                    source: None,
                    kind: "indexing".into(),
                });
            }
            if let Some(error) = &indexed.error {
                result.diagnostics.push(Diagnostic {
                    path: project.directory.clone(),
                    message: error.clone(),
                    source: None,
                    kind: "project".into(),
                });
            }
            let Some(discovery) = &indexed.discovery else {
                continue;
            };
            let context = Context {
                project: project.directory.clone(),
                checkout: project.directory.clone(),
                invocation_dir: canonical(invocation),
                environment: environment.clone(),
                cargo_global_aliases,
            };
            let mut snapshot = Self::build(&context, discovery, &favorites.for_tree(&project.directory), runs, usage)
                .with_registration(project, indexed.branch.as_deref());
            result.diagnostics.append(&mut snapshot.diagnostics);
            if snapshot.entries.len() > 10_000 {
                snapshot.entries.sort_by_key(|entry| match entry.kind {
                    Kind::Recent | Kind::Run => 0,
                    _ if entry.favorite => 1,
                    _ if entry.launches > 0 => 2,
                    _ => 3,
                });
                snapshot.entries.truncate(10_000);
                result.diagnostics.push(Diagnostic {
                    path: project.directory.clone(),
                    message:
                        "registered checkout reached the 10,000-entry limit; use Project scope for a narrower search"
                            .into(),
                    source: None,
                    kind: "limit".into(),
                });
            }
            for entry in snapshot.entries {
                if matches!(entry.kind, Kind::Run | Kind::Recent) {
                    if let Some(position) = execution_rows.get(&entry.id) {
                        if result.entries[*position].checkout.len() < entry.checkout.len() {
                            result.entries[*position] = entry;
                        }
                        continue;
                    }
                    execution_rows.insert(entry.id.clone(), result.entries.len());
                }
                if result.entries.len() >= 50_000 {
                    result.diagnostics.push(Diagnostic {
                        path: project.directory.clone(),
                        message: "registered search reached the 50,000-entry limit".into(),
                        source: None,
                        kind: "limit".into(),
                    });
                    return result;
                }
                result.entries.push(entry);
            }
        }
        result
    }
    pub fn build(
        context: &Context,
        discovery: &Discovery,
        favorites: &[Favorite],
        runs: &[Run],
        usage: &usage::Data,
    ) -> Self {
        let project_name = Path::new(&context.project).file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut snapshot = Self::default();
        // A source can declare thousands of scripts using the same tools.
        // Resolve each tool/cwd once in this snapshot; the next build is fresh.
        let mut tools = HashMap::<(String, String), Requirement>::new();
        let mut tool_stamps = HashMap::<String, String>::new();
        let mut cargo_environments = HashMap::<String, String>::new();
        let alias_directory = context.execution_directory(Source::Cargo, &context.project);
        let aliases = runui_project::discover_cargo_aliases(&runui_project::CargoAliasContext {
            directory: alias_directory,
            checkout: context.checkout.clone(),
            include_global: context.cargo_global_aliases,
            environment: context.environment.cargo_aliases.clone(),
        });
        for file in discovery.files.iter().chain(&aliases.files) {
            if let Some(error) = &file.err {
                snapshot.diagnostics.push(Diagnostic {
                    path: file.path.clone(),
                    message: error.clone(),
                    source: Some(file.source.as_str().into()),
                    kind: "parse".into(),
                });
            }
            for command in &file.commands {
                let directory = context.execution_directory(command.source, &file.dir);
                let relative_entry = if command.cargo_alias.is_some() {
                    ".cargo/aliases".into()
                } else {
                    relative(&context.checkout, &canonical(&file.path))
                };
                let relative_directory = relative(&context.checkout, &directory);
                let native_name = command
                    .cargo
                    .as_ref()
                    .map(|cargo| {
                        cargo
                            .target
                            .as_ref()
                            .map(|target| target.name.as_str())
                            .unwrap_or_else(|| command.argv.get(1).map(String::as_str).unwrap_or(&command.name))
                    })
                    .unwrap_or_else(|| {
                        command.cargo_alias.as_ref().map(|alias| alias.name.as_str()).unwrap_or(&command.name)
                    });
                let id = format!(
                    "cmd-v1-{}",
                    digest(&[
                        &context.checkout,
                        command.source.as_str(),
                        &relative_entry,
                        native_name,
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
                let programs: Vec<_> = if command.requirements.is_empty() {
                    command.argv.first().cloned().into_iter().collect()
                } else {
                    command.requirements.clone()
                };
                let requirements: Vec<_> = programs
                    .into_iter()
                    .map(|program| {
                        tools
                            .entry((directory.clone(), program.clone()))
                            .or_insert_with(|| {
                                let resolved_path = context.environment.resolve(&program, &directory);
                                let availability = if context.environment.project_wrapper {
                                    Availability::ProjectEnvironment
                                } else if resolved_path.is_some() {
                                    Availability::OnPath
                                } else {
                                    Availability::Missing
                                };
                                Requirement { program, availability, resolved_path }
                            })
                            .clone()
                    })
                    .collect();
                let availability = if requirements.iter().any(|tool| tool.availability == Availability::Missing) {
                    Availability::Missing
                } else if requirements.iter().any(|tool| tool.availability == Availability::ProjectEnvironment) {
                    Availability::ProjectEnvironment
                } else if requirements.is_empty() {
                    Availability::Shell
                } else {
                    Availability::OnPath
                };
                let mut dependencies = file.fingerprint.clone();
                if matches!(command.source, Source::Composer | Source::Cargo) {
                    // Native executable replacement changes preparation, even
                    // when the manifest and PATH spelling have not changed.
                    for tool in &requirements {
                        use std::os::unix::fs::MetadataExt;
                        dependencies.push_str(&format!("{:?}", tool));
                        if let Some(path) = &tool.resolved_path {
                            let stamp = tool_stamps.entry(path.clone()).or_insert_with(|| {
                                format!(
                                    "{:?}",
                                    fs::metadata(path).ok().map(|m| (
                                        m.dev(),
                                        m.ino(),
                                        m.len(),
                                        m.mtime(),
                                        m.mtime_nsec(),
                                        m.ctime(),
                                        m.ctime_nsec()
                                    ))
                                )
                            });
                            dependencies.push_str(stamp);
                        }
                    }
                }
                if command.source == Source::Cargo {
                    dependencies.push_str(
                        cargo_environments
                            .entry(directory.clone())
                            .or_insert_with(|| cargo_environment(&directory, &context.environment.cargo_aliases)),
                    );
                }
                let fingerprint = fingerprint(&directory, &recipe, &dependencies);
                let mut entry = Entry {
                    registry_id: None,
                    branch: None,
                    id: id.clone(),
                    definition_id: Some(id),
                    fingerprint,
                    kind: if command.hook { Kind::Hook } else { Kind::Command },
                    origin: if command.generated { Origin::Generated } else { Origin::Declared },
                    name: command.name.clone(),
                    description: command.desc.clone(),
                    source: command.source.as_str().into(),
                    project: project_name.clone(),
                    project_directory: context.project.clone(),
                    checkout: context.checkout.clone(),
                    proximity: distance(
                        &context.invocation_dir,
                        command.cargo.as_ref().map(|cargo| cargo.manifest_directory.as_str()).unwrap_or(&directory),
                    ),
                    directory,
                    relative_directory,
                    entry_file: if command.cargo_alias.is_some() && file.path.is_empty() {
                        None
                    } else {
                        Some(canonical(&file.path))
                    },
                    defining_file: if command.cargo_alias.is_some() && command.defining_file.is_empty() {
                        None
                    } else {
                        Some(canonical(&command.defining_file))
                    },
                    defining_line: command.defining_line,
                    variant: command.variant.clone(),
                    command: command.run.clone(),
                    recipe,
                    availability,
                    requirements,
                    aliases: command.aliases.clone(),
                    cargo: command.cargo.clone(),
                    cargo_alias: command.cargo_alias.clone(),
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
        for problem in discovery.diagnostics.iter().chain(&aliases.diagnostics) {
            snapshot.diagnostics.push(Diagnostic {
                path: problem.path.clone(),
                message: problem.message.clone(),
                source: problem.source.map(|source| source.as_str().into()),
                kind: "discovery".into(),
            });
        }
        for notice in discovery.notices.iter().chain(&aliases.notices) {
            snapshot.diagnostics.push(Diagnostic {
                path: notice.path.clone(),
                message: notice.message.clone(),
                source: notice.source.map(|source| source.as_str().into()),
                kind: "notice".into(),
            });
        }
        for favorite in favorites {
            let directory = canonical(&Path::new(&context.project).join(&favorite.dir).to_string_lossy());
            let mut matched = false;
            for entry in &mut snapshot.entries {
                if entry.directory == directory
                    && entry.command == favorite.command
                    && (entry.kind == Kind::Favorite || favorite.source.is_empty() || favorite.source == entry.source)
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
                registry_id: None,
                branch: None,
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
                proximity: distance(&context.invocation_dir, &directory),
                directory,
                entry_file: None,
                defining_file: None,
                defining_line: None,
                variant: String::new(),
                command: favorite.command.clone(),
                recipe,
                availability: Availability::Shell,
                requirements: Vec::new(),
                aliases: Vec::new(),
                cargo: None,
                cargo_alias: None,
                favorite: true,
                favorite_names: vec![favorite.label().into()],
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
                registry_id: None,
                branch: None,
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
                proximity: distance(&context.invocation_dir, &directory),
                entry_file: None,
                defining_file: None,
                defining_line: None,
                variant: String::new(),
                command: run.command.clone(),
                recipe,
                availability: Availability::Shell,
                requirements: Vec::new(),
                aliases: Vec::new(),
                cargo: None,
                cargo_alias: None,
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
                registry_id: None,
                branch: None,
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
                proximity: distance(&context.invocation_dir, &invocation.directory),
                entry_file: None,
                defining_file: None,
                defining_line: None,
                variant: String::new(),
                command: invocation.command.clone(),
                recipe: invocation.recipe.clone(),
                availability: Availability::Shell,
                requirements: Vec::new(),
                aliases: Vec::new(),
                cargo: None,
                cargo_alias: None,
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

    pub fn complete(&self) -> bool {
        self.diagnostics.iter().all(|diagnostic| diagnostic.kind == "notice")
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
        if let Some(alias) = &entry.cargo_alias
            && let Some(reason) = &alias.disabled_reason
        {
            return Err(PrepareError::Inputs(format!("Cargo alias {:?}: {}", alias.name, reason)));
        }
        let mut recipe = entry.recipe.clone();
        if let Some(timeout) = inputs.timeout_seconds {
            if entry.source != "composer" || !matches!(entry.kind, Kind::Command | Kind::Hook) {
                return Err(PrepareError::Inputs("timeout_seconds requires a Composer script definition".into()));
            }
            if let Recipe::Tool { argv, .. } = &mut recipe {
                argv.insert(argv.len() - 2, format!("--timeout={timeout}"));
            }
        }
        let command = prepare_recipe(&recipe, inputs)?;
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

fn distance(from: &str, directory: &str) -> usize {
    Path::new(&relative(from, directory))
        .components()
        .filter(|part| !matches!(part, std::path::Component::CurDir))
        .count()
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
/// Cargo reads config from invocation ancestors, not manifest ancestors.
/// Keep these metadata/env selectors private; only their digest is exposed.
fn cargo_environment(directory: &str, environment: &runui_project::CargoAliasEnvironment) -> String {
    use std::os::unix::fs::MetadataExt;
    let mut paths = Vec::new();
    let config = |directory: &Path| {
        let legacy = directory.join("config");
        if legacy.exists() { legacy } else { directory.join("config.toml") }
    };
    for dir in Path::new(directory).ancestors().take(64) {
        paths.push(config(&dir.join(".cargo")));
        for file in ["rust-toolchain", "rust-toolchain.toml", "mise.toml", ".mise.toml", ".tool-versions"] {
            paths.push(dir.join(file));
        }
    }
    if let Some(home) = &environment.cargo_home {
        paths.push(config(&Path::new(directory).join(home)));
    }
    let rustup = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")));
    if let Some(home) = rustup {
        paths.push(Path::new(directory).join(home).join("settings.toml"));
    }
    let mut parts = String::new();
    for path in paths {
        parts += &format!(
            "{}:{:?}",
            path.display(),
            fs::metadata(&path).ok().map(|m| (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec()
            ))
        );
    }
    for name in [
        "CARGO_HOME",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
    ] {
        parts += &format!("{name}:{:?}", std::env::var_os(name));
    }
    digest(&[&parts])
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
