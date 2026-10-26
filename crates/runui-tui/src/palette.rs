//! The command palette owns its query and selection; opening it never moves
//! panel cursors or reads their output. Catalog work happens in Tea commands.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use runui_catalog::{Context, Entry, Inputs, Kind, Query, Scope, Snapshot};
use runui_tea::ansi;
use runui_tea::bubbles::textinput;
use runui_tea::tea::{self, Cmd, KeyPressMsg, Msg};

use crate::styles::{S_ACCENT, S_BAD, S_DIM, S_HEADING, S_ROW_ACTIVE, S_TITLE};
use crate::{App, Focus, PANEL_RUNS, RUNS_EVERYWHERE, StartedMsg};

mod projects;
pub(crate) use projects::{ProjectOpened, RegistryChanged};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Commands,
    Recent,
}

pub(crate) struct Palette {
    input: textinput::Model,
    snapshot: Arc<Snapshot>,
    hits: Vec<Entry>,
    at: usize,
    mode: Mode,
    scope: Scope,
    cargo_global_aliases: bool,
    workspace_discovery: Option<(String, Arc<runui_project::Discovery>)>,
    pub(crate) loading: bool,
    queued: bool,
    launching: bool,
    detail: bool,
    diagnostics: bool,
    detail_top: usize,
    note: String,
    error: String,
    query_error: String,
    picker: Option<projects::Picker>,
    native: Option<(String, String, runui_catalog::inspection::Report)>,
    native_cancel: Arc<AtomicBool>,
    native_busy: bool,
    projects: Vec<runui_catalog::index::Indexed>,
    project_filter: Option<String>,
}

pub(crate) struct Loaded {
    epoch: u64,
    snapshot: Snapshot,
    workspace_discovery: Option<(String, Arc<runui_project::Discovery>)>,
    projects: Option<Result<Vec<runui_catalog::index::Indexed>, String>>,
}
pub(crate) struct Inspected {
    epoch: u64,
    id: String,
    fingerprint: String,
    report: runui_catalog::inspection::Report,
}

pub(crate) struct Launched {
    epoch: u64,
    run: runui_runs::Run,
    error: Option<String>,
    warning: Option<String>,
    fresh: Option<Snapshot>,
    discovery: Option<runui_project::Discovery>,
    scope: Scope,
}

impl Drop for Palette {
    fn drop(&mut self) {
        self.native_cancel.store(true, Ordering::Relaxed);
    }
}

impl Palette {
    fn new() -> Self {
        let mut input = textinput::new();
        input.prompt = "> ".into();
        input.placeholder = "Search names, projects, directories, descriptions…".into();
        let mut styles = input.styles();
        styles.cursor.blink = false;
        input.set_styles(styles);
        Self {
            input,
            snapshot: Arc::new(Snapshot::default()),
            hits: Vec::new(),
            at: 0,
            mode: Mode::Commands,
            scope: Scope::Project,
            cargo_global_aliases: false,
            workspace_discovery: None,
            loading: false,
            queued: false,
            launching: false,
            detail: false,
            diagnostics: false,
            detail_top: 0,
            note: String::new(),
            error: String::new(),
            query_error: String::new(),
            picker: None,
            native: None,
            native_cancel: Arc::new(AtomicBool::new(false)),
            native_busy: false,
            projects: Vec::new(),
            project_filter: None,
        }
    }

    fn query(&mut self) {
        let selected = self.hits.get(self.at).map(|entry| entry.id.clone());
        match Query::parse(&self.input.value()) {
            Ok(mut query) => {
                query.kinds = match self.mode {
                    Mode::Commands => {
                        query.commands();
                        query.kinds.clone()
                    }
                    Mode::Recent => vec![Kind::Recent],
                };
                query.registry_id = self.project_filter.clone();
                self.hits = self.snapshot.search(&query).into_iter().map(|hit| hit.entry.clone()).collect();
                self.at = selected.and_then(|id| self.hits.iter().position(|entry| entry.id == id)).unwrap_or(0);
                self.query_error.clear();
            }
            Err(error) => {
                self.query_error = error;
                self.hits.clear();
                self.at = 0;
            }
        }
    }
    pub(crate) fn notify(&mut self, message: &str, bad: bool) {
        if bad {
            self.error = message.into();
        } else {
            self.note = message.into();
        }
    }
    fn selected(&self) -> Option<Entry> {
        self.hits.get(self.at).cloned()
    }
    fn set_snapshot(&mut self, snapshot: Snapshot) {
        self.snapshot = Arc::new(snapshot);
        self.query();
    }

    pub(crate) fn draw(&mut self, width: i32, height: i32) -> String {
        if self.picker.is_some() {
            return self.draw_projects(width, height);
        }
        let width = width.max(1);
        let height = height.max(1) as usize;
        self.input.set_width((width - 3).max(1));
        let mode = if self.mode == Mode::Commands { "Commands" } else { "Recent invocations" };
        let status = if self.launching {
            " · validating launch…"
        } else if self.loading {
            " · refreshing…"
        } else {
            ""
        };
        let scope = match self.scope {
            Scope::Project => "Project",
            Scope::Workspace => "Workspace",
            Scope::Registered => "Registered",
        };
        let scope = if let Some(id) = &self.project_filter {
            self.projects
                .iter()
                .find(|item| &item.project.id == id)
                .map(|item| format!("{scope} / {}", safe(&item.project.name)))
                .unwrap_or_else(|| scope.into())
        } else {
            scope.into()
        };
        let mut lines = vec![
            S_HEADING.render(&format!("{mode} · {scope}{status}")),
            self.input.view(),
            S_DIM.render(&"─".repeat(width as usize)),
        ];
        let body = height.saturating_sub(6);
        let preview = self.preview(width);
        if width >= 90 && !self.detail {
            let list_width = (width * 45 / 100).max(1);
            let preview_width = (width - list_width - 3).max(1);
            let list = self.list(list_width, body);
            let preview = self.preview(preview_width);
            for row in 0..body {
                let left = list.get(row).cloned().unwrap_or_default();
                let left = ansi::truncate(&left, list_width, "");
                let pad = " ".repeat((list_width - ansi::string_width(&left)).max(0) as usize);
                lines.push(format!(
                    "{left}{pad} {} {}",
                    S_DIM.render("│"),
                    preview.get(row).cloned().unwrap_or_default()
                ));
            }
        } else if self.detail {
            self.detail_top = self.detail_top.min(preview.len().saturating_sub(body));
            lines.extend(preview.into_iter().skip(self.detail_top).take(body));
        } else {
            let list_height = (body / 2).max(1);
            lines.extend(self.list(width, list_height));
            lines.push(S_DIM.render(&"─".repeat(width as usize)));
            lines.extend(preview.into_iter().take(body.saturating_sub(list_height + 1)));
        }
        while lines.len() < height.saturating_sub(3) {
            lines.push(String::new());
        }
        lines.push(S_DIM.render(&"─".repeat(width as usize)));
        let information = if !self.query_error.is_empty() {
            S_BAD.render(&safe(&self.query_error))
        } else if !self.error.is_empty() {
            S_BAD.render(&safe(&self.error))
        } else {
            let status = format!(
                "{} matches · {} diagnostics · f2 inspect · ctrl+v details · alt+i native · Cargo globals {} (alt+g)",
                self.hits.len(),
                self.snapshot.diagnostics.len(),
                if self.cargo_global_aliases { "on" } else { "off" }
            );
            if self.note.is_empty() {
                S_DIM.render(&status)
            } else {
                S_ACCENT.render(&format!("{} · {status}", safe(&self.note)))
            }
        };
        lines.push(information);
        lines.push(S_DIM.render(
            "enter run/focus · tab mode · alt+s scope · alt+p projects · alt+r another · alt+m favorite · ctrl+y copy · ctrl+o source · ctrl+t terminal · esc back",
        ));
        lines.truncate(height);
        lines.into_iter().map(|line| ansi::truncate(&line, width, "…")).collect::<Vec<_>>().join("\n")
    }

    fn list(&self, width: i32, height: usize) -> Vec<String> {
        if self.hits.is_empty() {
            let mut lines = vec![S_DIM.render(if self.loading { "Loading catalog…" } else { "No matches" })];
            lines.resize(height, String::new());
            return lines;
        }
        let row_height = if self.scope == Scope::Registered { 2 } else { 1 };
        let top = self.at.saturating_sub((height / row_height).saturating_sub(1));
        let mut lines = Vec::new();
        for (index, entry) in self.hits.iter().enumerate().skip(top).take(height / row_height) {
            let favorite = if entry.favorite { "★ " } else { "  " };
            let row = if self.scope == Scope::Registered {
                format!(
                    "{favorite}{} · {} · {}{}",
                    safe(&entry.name),
                    entry.source,
                    safe(&entry.project),
                    entry.branch.as_ref().map(|branch| format!(" [{}]", safe(branch))).unwrap_or_default()
                )
            } else if entry.cargo.is_some() {
                format!(
                    "{favorite}{} › {} · {}",
                    safe(&entry.relative_source_directory()),
                    safe(&entry.name),
                    entry.source
                )
            } else {
                format!(
                    "{favorite}{}  · {}  · {}",
                    safe(&entry.name),
                    entry.source,
                    safe(&entry.relative_source_directory())
                )
            };
            let row = ansi::truncate(&row, width, "…");
            lines.push(if index == self.at { S_ROW_ACTIVE.render(&row) } else { row });
            if self.scope == Scope::Registered {
                lines.push(S_DIM.render(&projects::tail(
                    &format!("{} / {}", safe(&entry.checkout), safe(&entry.relative_source_directory())),
                    width,
                )));
            }
        }
        lines.resize(height, String::new());
        lines
    }

    fn preview(&self, width: i32) -> Vec<String> {
        if self.diagnostics {
            let mut lines = vec!["Discovery diagnostics".into()];
            if self.snapshot.diagnostics.is_empty() {
                lines.push("Discovery is complete; no diagnostics.".into());
            }
            for item in &self.snapshot.diagnostics {
                lines.push(format!("{} · {}", safe(&item.kind), safe(&item.path)));
                lines.push(safe(&item.message));
                lines.push(String::new());
            }
            lines.push("F2 returns to the command · arrows/page keys scroll".into());
            return wrapped(lines, width);
        }
        let Some(entry) = self.hits.get(self.at) else {
            let mut lines = vec![
                "Search all command sources together.".into(),
                "source:npm  dir:apps/web  kind:favorite".into(),
                "Tab switches commands and recent invocations.".into(),
            ];
            for problem in &self.snapshot.diagnostics {
                lines.push(format!("{}: {}", safe(&problem.path), safe(&problem.message)));
            }
            return wrapped(lines, width);
        };
        let mut lines = vec![
            S_TITLE.render(&safe(&entry.name)),
            format!("$ {}", safe(&entry.command)),
            format!("cwd: {}", safe(&entry.directory)),
            format!("{} · {:?} · {}", entry.source, entry.origin, entry.availability.label()),
        ];
        if let Some(cargo) = &entry.cargo {
            lines.push(format!("Cargo {} · {}", cargo.scope.label(), cargo.package.as_deref().unwrap_or("workspace")));
            lines.push(format!("manifest folder: {}", safe(&cargo.manifest_directory)));
            if !cargo.default_member_paths.is_empty() {
                lines.push(format!(
                    "discovered default members: {}",
                    cargo.default_member_paths.iter().map(|path| safe(path)).collect::<Vec<_>>().join(", ")
                ));
            }
            if let Some(target) = &cargo.target {
                lines.push(format!("target source: {}", safe(&target.source_path)));
                lines.push(format!(
                    "{} target: {}{}",
                    target.kind.as_str(),
                    safe(&target.name),
                    if target.default_run { " · default-run" } else { "" }
                ));
                if !target.required_features.is_empty() {
                    lines.push(format!(
                        "required features enabled: {}",
                        target.required_features.iter().map(|feature| safe(feature)).collect::<Vec<_>>().join(", ")
                    ));
                }
            }
            let checked = self.native.as_ref().is_some_and(|(id, fingerprint, report)| {
                id == &entry.id
                    && fingerprint == &entry.fingerprint
                    && report.cargo.as_ref().is_some_and(|cargo| cargo.definition_matches)
            });
            lines.push(
                if checked {
                    "Selected definition matches native Cargo metadata."
                } else {
                    "Static Cargo discovery · alt+i checks native metadata."
                }
                .into(),
            );
        }
        if let Some(alias) = &entry.cargo_alias {
            lines.push(format!("Cargo alias · {}", if alias.global { "global context" } else { "project context" }));
            lines.push(format!(
                "Native expansion: {}",
                match &alias.value {
                    runui_project::CargoAliasValue::String(value) => safe(value),
                    runui_project::CargoAliasValue::Array(values) => format!("{:?}", values),
                }
            ));
            if entry.availability == runui_catalog::Availability::ProjectEnvironment {
                lines.push("Expansion uses captured config; the project wrapper environment is unchecked.".into());
            }
            lines.push("Cargo owns expansion; extra arguments go directly to the alias.".into());
            lines.push("Include -- explicitly when the alias's subcommand needs program arguments.".into());
            for origin in &alias.origins {
                if let Some(file) = &origin.file {
                    lines.push(format!("config: {}", safe(file)));
                }
                if let Some(environment) = &origin.environment {
                    lines.push(format!("environment override: {}", safe(environment)));
                }
            }
            if let Some(reason) = &alias.disabled_reason {
                lines.push(format!("Unavailable: {}", safe(reason)));
            }
            lines.extend(alias.diagnostics.iter().map(|message| safe(message)));
        }
        if !entry.aliases.is_empty() {
            lines.push(format!(
                "aliases: {}",
                entry.aliases.iter().map(|alias| safe(alias)).collect::<Vec<_>>().join(", ")
            ));
        }
        for tool in &entry.requirements {
            lines.push(format!(
                "{}: {}{}",
                safe(&tool.program),
                tool.availability.label(),
                tool.resolved_path
                    .as_ref()
                    .filter(|_| self.detail)
                    .map(|path| format!(" · {}", safe(path)))
                    .unwrap_or_default()
            ));
        }
        if entry.kind == Kind::Hook {
            lines.push("Lifecycle hook · explicit launch through Composer".into());
        }
        if self.native_busy {
            lines.push("Native check running…".into());
        }
        if let Some((id, fingerprint, report)) = &self.native
            && id == &entry.id
            && fingerprint == &entry.fingerprint
        {
            if let Some(reason) = &report.skipped {
                lines.push(safe(reason));
            }
            if let Some(cargo) = &report.cargo {
                lines.push(format!("Native workspace: {}", safe(&cargo.workspace_root)));
                lines.push("Inspection: offline · locked · installed compiler · compiler wrappers disabled".into());
                lines.push(format!(
                    "{} native members · {} default members · {} selected package targets",
                    cargo.members.len(),
                    cargo.default_members.len(),
                    cargo.targets.len()
                ));
                if !cargo.available_features.is_empty() {
                    lines.push(format!(
                        "Native features: {}",
                        cargo.available_features.iter().map(|feature| safe(feature)).collect::<Vec<_>>().join(", ")
                    ));
                }
                lines.push("Native default members:".into());
                for member in cargo.default_members.iter().take(20) {
                    lines.push(format!(
                        "  {}",
                        safe(
                            &runui_core::golang::filepath::rel(&cargo.workspace_root, member)
                                .unwrap_or_else(|| member.clone())
                        )
                    ));
                }
                if cargo.default_members.len() > 20 {
                    lines.push("  More defaults are available in inspect --native --json.".into());
                }
                if !cargo.targets.is_empty() {
                    lines.push("Native package targets:".into());
                    for target in cargo.targets.iter().take(20) {
                        lines.push(format!("  {} · {}", safe(&target.name), target.kind.as_str()));
                    }
                    if cargo.targets.len() > 20 {
                        lines.push("  More targets are available in inspect --native --json.".into());
                    }
                }
                lines.extend(cargo.diagnostics.iter().map(|message| safe(message)));
            }
            for probe in &report.checked {
                lines.push(format!(
                    "Native {}: {}",
                    safe(&probe.tool),
                    if !probe.supported {
                        "unavailable or unsupported"
                    } else if probe.tool == "cargo metadata" {
                        "metadata verified"
                    } else if probe.tool.starts_with("rustup which ") {
                        "installed tool located"
                    } else {
                        "version verified"
                    }
                ));
                if let Some(error) = &probe.error {
                    lines.push(safe(error));
                }
                if let Some(output) = &probe.output {
                    let result = match output.end {
                        runui_runs::inspection::End::Exited => "exited",
                        runui_runs::inspection::End::Timeout => "timed out",
                        runui_runs::inspection::End::OutputLimit => "output limit reached",
                        runui_runs::inspection::End::Cancelled => "cancelled",
                    };
                    let code = output.code.map(|code| code.to_string()).unwrap_or_else(|| "signal".into());
                    lines.push(format!("{result} · exit {code} · {} ms", output.elapsed_ms));
                    if probe.tool != "cargo metadata" {
                        lines.extend(output.stdout.lines().map(safe));
                    }
                    lines.extend(output.stderr.lines().map(safe));
                }
            }
        }
        if let Some(id) = &entry.registry_id {
            lines.push(format!(
                "project: {} · {}",
                safe(&entry.project),
                entry.branch.as_deref().map(safe).unwrap_or_else(|| "branch unknown".into())
            ));
            if self.detail {
                lines.push(format!("checkout: {}", safe(&entry.checkout)));
                lines.push(format!("registry: {id}"));
            }
        }
        if let Some(file) = &entry.entry_file
            && entry.cargo_alias.is_none()
        {
            lines.push(format!(
                "entry: {}{}",
                safe(file),
                if entry.defining_file == entry.entry_file {
                    entry.defining_line.map(|line| format!(":{line}")).unwrap_or_default()
                } else {
                    String::new()
                }
            ));
        }
        if entry.defining_file != entry.entry_file
            && let Some(file) = &entry.defining_file
        {
            lines.push(format!(
                "defined: {}{}",
                safe(file),
                entry.defining_line.map(|line| format!(":{line}")).unwrap_or_default()
            ));
        }
        if !entry.description.is_empty() && entry.cargo_alias.is_none() {
            lines.push(safe(&entry.description));
        }
        if let Some(state) = &entry.state {
            lines.push(format!("Last result: {state}"));
        }
        if entry.launches > 0 {
            lines.push(format!(
                "{} launches · last used {}",
                entry.launches,
                entry.last_used.map(|time| time.to_rfc3339()).unwrap_or_default()
            ));
        }
        if entry.state.as_deref() == Some("running") {
            lines.push("Enter focuses its live output; Alt+R starts another.".into());
        }
        if entry.kind == Kind::Recent {
            lines.push("Enter repeats the stored invocation; Alt+U selects its current definition.".into());
            if !entry.output_available {
                lines.push("Output is no longer retained; command recall remains available.".into());
            }
        }
        if self.detail {
            lines.push(format!("id: {}", entry.id));
            lines.extend(
                [
                    "Enter run/focus · Alt+R another",
                    "Tab mode · Alt+S scope · Alt+P projects",
                    "Alt+G includes global Cargo aliases",
                    "Alt+M favorite · Ctrl+Y copy",
                    "Ctrl+O source · Ctrl+T terminal · F2 diagnostics",
                    "Esc returns",
                ]
                .map(str::to_owned),
            );
        }
        wrapped(lines, width)
    }
}

fn wrapped(lines: Vec<String>, width: i32) -> Vec<String> {
    lines
        .into_iter()
        .flat_map(|line| ansi::wrap(&line, width, "/").split('\n').map(str::to_string).collect::<Vec<_>>())
        .collect()
}
pub(crate) fn safe(text: &str) -> String {
    text.chars()
        .flat_map(|ch| if ch.is_control() { ch.escape_default().collect::<Vec<_>>() } else { vec![ch] })
        .collect()
}

fn catalog_context(
    dir: &str,
    repo: &str,
    invocation: &str,
    environment: &runui_catalog::Environment,
    cargo_global_aliases: bool,
) -> Context {
    Context {
        project: runui_catalog::canonical(dir),
        checkout: runui_catalog::canonical(repo),
        invocation_dir: runui_catalog::canonical(invocation),
        environment: environment.clone(),
        cargo_global_aliases,
    }
}

pub(crate) fn build(
    context: &Context,
    discovery: &runui_project::Discovery,
    favorites: &[runui_favorites::Favorite],
    runs: &[runui_runs::Run],
    runner: &dyn crate::Runner,
) -> Snapshot {
    let store = runner.usage_store();
    if let Some(store) = &store {
        let _ = store.reconcile(runs);
    }
    let usage = store.as_ref().map(|store| store.load()).transpose();
    let default = runui_catalog::usage::Data::default();
    let mut snapshot = Snapshot::build(
        context,
        discovery,
        favorites,
        runs,
        usage.as_ref().ok().and_then(|data| data.as_ref()).unwrap_or(&default),
    );
    if let Err(error) = usage {
        snapshot.diagnostics.push(runui_catalog::Diagnostic {
            path: store.map(|store| store.file().to_string_lossy().into_owned()).unwrap_or_default(),
            message: error.to_string(),
            source: Some("usage".into()),
            kind: "history".into(),
        });
    }
    snapshot
}

fn build_registered(
    indexed: &[runui_catalog::index::Indexed],
    context: &Context,
    favorites: &runui_favorites::Store,
    runs: &[runui_runs::Run],
    runner: &dyn crate::Runner,
) -> Snapshot {
    let store = runner.usage_store();
    if let Some(store) = &store {
        let _ = store.reconcile(runs);
    }
    let usage = store.as_ref().map(|store| store.load()).transpose();
    let default = runui_catalog::usage::Data::default();
    let mut snapshot = Snapshot::registered(
        indexed,
        &context.environment,
        &context.invocation_dir,
        favorites,
        runs,
        usage.as_ref().ok().and_then(|data| data.as_ref()).unwrap_or(&default),
        context.cargo_global_aliases,
    );
    if let Err(error) = usage {
        snapshot.diagnostics.push(runui_catalog::Diagnostic {
            path: store.map(|store| store.file().to_string_lossy().into_owned()).unwrap_or_default(),
            message: error.to_string(),
            source: Some("usage".into()),
            kind: "history".into(),
        });
    }
    snapshot
}

impl App {
    pub(crate) fn open_palette(&mut self) -> Cmd {
        self.palette_epoch += 1;
        let mut palette = Palette::new();
        let focus = palette.input.focus();
        self.palette = Some(palette);
        tea::batch(vec![focus, self.refresh_palette()])
    }

    pub(crate) fn refresh_palette(&mut self) -> Cmd {
        let palette = self.palette.as_mut()?;
        if palette.loading {
            palette.queued = true;
            return None;
        }
        palette.loading = true;
        let scope = palette.scope;
        let cargo_global_aliases = palette.cargo_global_aliases;
        let needs_projects = scope == Scope::Registered || palette.picker.is_some();
        let project_filter = palette.project_filter.clone();
        let previous = palette.workspace_discovery.clone();
        let epoch = self.palette_epoch;
        let (dir, repo) = (self.cfg.dir.clone(), self.cfg.repo.clone());
        let invocation = self.cfg.invocation_dir.clone();
        let environment = self.cfg.catalog_environment.clone();
        let (registry, index) = (self.cfg.registry.clone(), self.cfg.index.clone());
        let (discovery, favorites, runs, runner, discover) = (
            self.discovery.clone(),
            self.cfg.favorites.clone(),
            self.runs.clone(),
            self.cfg.runs.clone(),
            self.cfg.discover,
        );
        tea::cmd_msg(move || {
            let selected = dir;
            let mut context = catalog_context(&selected, &repo, &invocation, &environment, cargo_global_aliases);
            let project_data = if needs_projects {
                Some(registry.load().map(|data| index.request(&data.projects)).map_err(|error| format!("{error:#}")))
            } else {
                None
            };
            if scope == Scope::Registered {
                let mut snapshot = match project_data.as_ref() {
                    Some(Ok(indexed)) => {
                        let selected: Vec<_> = indexed
                            .iter()
                            .filter(|item| project_filter.as_ref().is_none_or(|id| &item.project.id == id))
                            .cloned()
                            .collect();
                        build_registered(&selected, &context, &favorites, &runs, runner.as_ref())
                    }
                    _ => Snapshot::default(),
                };
                if let Some(Err(error)) = &project_data {
                    snapshot.diagnostics.push(runui_catalog::Diagnostic {
                        path: registry.file().to_string_lossy().into_owned(),
                        message: error.clone(),
                        source: None,
                        kind: "registry".into(),
                    });
                }
                return Loaded { epoch, snapshot, workspace_discovery: None, projects: project_data };
            }
            let dir = scope.directory(&selected, &repo).expect("registered scope was handled");
            let discovery = if scope == Scope::Project {
                discovery
            } else if let Some((_, previous)) =
                previous.filter(|(previous_dir, previous)| *previous_dir == dir && !previous.changed())
            {
                previous
            } else {
                Arc::new(discover(&dir, &repo))
            };
            let favorites =
                if scope == Scope::Workspace { favorites.for_tree(&dir) } else { favorites.for_dir(&selected) };
            context.project = runui_catalog::canonical(&dir);
            Loaded {
                epoch,
                snapshot: build(&context, &discovery, &favorites, &runs, runner.as_ref()),
                workspace_discovery: if scope == Scope::Workspace { Some((dir, discovery)) } else { None },
                projects: project_data,
            }
        })
    }

    pub(crate) fn palette_loaded(&mut self, message: Loaded) -> Cmd {
        if message.epoch != self.palette_epoch {
            return None;
        }
        let palette = self.palette.as_mut()?;
        palette.loading = false;
        let queued = std::mem::take(&mut palette.queued);
        if let Some(projects) = message.projects {
            match projects {
                Ok(projects) => {
                    palette.projects = projects;
                    if palette
                        .project_filter
                        .as_ref()
                        .is_some_and(|id| !palette.projects.iter().any(|item| &item.project.id == id))
                    {
                        palette.project_filter = None;
                        palette.note = "That checkout was forgotten; showing registered projects".into();
                    }
                }
                Err(error) => {
                    palette.projects.clear();
                    palette.notify(&error, true);
                }
            }
        }
        palette.workspace_discovery = message.workspace_discovery;
        palette.set_snapshot(message.snapshot);
        if queued { self.refresh_palette() } else { None }
    }

    pub(crate) fn palette_input(&mut self, message: &Msg) -> Cmd {
        let palette = self.palette.as_mut()?;
        palette.error.clear();
        if let Some(picker) = &mut palette.picker {
            return picker.update(message);
        }
        let command = palette.input.update(message);
        palette.query();
        command
    }

    pub(crate) fn palette_key(&mut self, key: &KeyPressMsg) -> Cmd {
        if self.palette.as_ref().is_some_and(|palette| palette.picker.is_some()) {
            return self.project_picker_key(key);
        }
        let name = key.string();
        let palette = self.palette.as_mut()?;
        match name.as_str() {
            "esc" | "ctrl+p" => {
                self.cfg.index.cancel_pending();
                self.palette = None;
                self.palette_epoch += 1;
                None
            }
            "tab" | "shift+tab" => {
                palette.mode = if palette.mode == Mode::Commands { Mode::Recent } else { Mode::Commands };
                palette.at = 0;
                palette.note.clear();
                palette.query();
                None
            }
            "alt+s" if !palette.launching => {
                palette.native_cancel.store(true, Ordering::Relaxed);
                palette.native_busy = false;
                palette.native = None;
                palette.scope = match palette.scope {
                    Scope::Project => Scope::Workspace,
                    Scope::Workspace => Scope::Registered,
                    Scope::Registered => Scope::Project,
                };
                palette.project_filter = None;
                if palette.scope != Scope::Registered {
                    self.cfg.index.cancel_pending();
                }
                palette.workspace_discovery = None;
                palette.loading = false;
                palette.queued = false;
                palette.set_snapshot(Snapshot::default());
                palette.note = "Scope changes search context; each command keeps its own working directory".into();
                self.palette_epoch += 1;
                self.refresh_palette()
            }
            "alt+p" if !palette.launching => {
                palette.picker = Some(projects::Picker::new());
                self.refresh_palette()
            }
            "alt+g" if !palette.launching => {
                palette.cargo_global_aliases = !palette.cargo_global_aliases;
                palette.native_cancel.store(true, Ordering::Relaxed);
                palette.native_busy = false;
                palette.native = None;
                palette.loading = false;
                palette.queued = false;
                palette.set_snapshot(Snapshot::default());
                palette.note = if palette.cargo_global_aliases {
                    "Global Cargo aliases included for each selected invocation directory"
                } else {
                    "Showing project Cargo aliases"
                }
                .into();
                self.palette_epoch += 1;
                self.refresh_palette()
            }
            "up" if palette.detail => {
                palette.detail_top = palette.detail_top.saturating_sub(1);
                None
            }
            "down" if palette.detail => {
                palette.detail_top = palette.detail_top.saturating_add(1);
                None
            }
            "pgup" if palette.detail => {
                palette.detail_top = palette.detail_top.saturating_sub(10);
                None
            }
            "pgdown" if palette.detail => {
                palette.detail_top = palette.detail_top.saturating_add(10);
                None
            }
            "up" => {
                palette.at = palette.at.saturating_sub(1);
                None
            }
            "down" => {
                palette.at = (palette.at + 1).min(palette.hits.len().saturating_sub(1));
                None
            }
            "pgup" => {
                palette.at = palette.at.saturating_sub(10);
                None
            }
            "pgdown" => {
                palette.at = (palette.at + 10).min(palette.hits.len().saturating_sub(1));
                None
            }
            "alt+i" if !palette.native_busy => {
                let entry = palette.selected()?;
                let epoch = self.palette_epoch;
                let cancel = Arc::new(AtomicBool::new(false));
                palette.native_cancel.store(true, Ordering::Relaxed);
                palette.native_cancel = cancel.clone();
                palette.native_busy = true;
                palette.native = None;
                tea::cmd(move || {
                    Some(tea::msg(Inspected {
                        epoch,
                        id: entry.id.clone(),
                        fingerprint: entry.fingerprint.clone(),
                        report: runui_catalog::inspection::check(&entry, &cancel),
                    }))
                })
            }
            "ctrl+v" => {
                palette.diagnostics = false;
                palette.detail = !palette.detail;
                palette.detail_top = 0;
                None
            }
            "f2" => {
                palette.diagnostics = !palette.diagnostics;
                palette.detail = palette.diagnostics;
                palette.detail_top = 0;
                None
            }
            "ctrl+y" => {
                let entry = palette.selected()?;
                palette.note = "Copied the exact command".into();
                tea::set_clipboard(entry.command)
            }
            "alt+u" => {
                let selected = palette.selected()?;
                let Some(id) = selected.definition_id else {
                    palette.notify("This invocation has no recorded project definition", true);
                    return None;
                };
                let Some(current) = palette.snapshot.get(&id).cloned() else {
                    palette.notify(
                        "The original definition is no longer available; repeat the stored invocation explicitly",
                        true,
                    );
                    return None;
                };
                palette.mode = Mode::Commands;
                palette.input.set_value(&current.name);
                palette.query();
                palette.at = palette.hits.iter().position(|entry| entry.id == id).unwrap_or(0);
                palette.note = "Current definition and default inputs; Enter revalidates and launches it".into();
                None
            }
            "alt+m" => {
                let entry = palette.selected()?;
                let favorite_folder = if palette.scope == Scope::Registered {
                    entry.project_directory.clone()
                } else {
                    self.cfg.dir.clone()
                };
                let relative =
                    runui_core::golang::filepath::rel(&runui_catalog::canonical(&favorite_folder), &entry.directory)
                        .unwrap_or_default();
                let favorite = runui_favorites::Favorite {
                    name: entry.name,
                    command: entry.command,
                    source: if matches!(entry.kind, Kind::Command | Kind::Hook) { entry.source } else { String::new() },
                    dir: if relative == "." { String::new() } else { relative },
                    added: None,
                };
                match self.cfg.favorites.toggle(&favorite_folder, favorite) {
                    Ok(starred) => {
                        palette.notify(if starred { "Saved as a favorite" } else { "Removed from favorites" }, false)
                    }
                    Err(error) => palette.notify(&error.to_string(), true),
                }
                self.load_favorites();
                self.refresh_palette()
            }
            "ctrl+o" | "ctrl+t" => {
                let entry = palette.selected()?;
                let source = name == "ctrl+o";
                let command = if source {
                    let Some(file) = entry.defining_file.or(entry.entry_file) else {
                        palette.notify(
                            "No source file is recorded; Alt+U selects the current definition when known",
                            true,
                        );
                        return None;
                    };
                    let editor = std::env::var("EDITOR")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "vi".into());
                    format!("{editor} {}", runui_catalog::quote_arg(&file))
                } else {
                    entry.command
                };
                let directory = entry.directory;
                tea::cmd_msg(move || {
                    crate::HandoffMsg(crate::terminal::open(&directory, &command).err().map(|error| error.to_string()))
                })
            }
            "enter" | "alt+r" => {
                if palette.launching {
                    return None;
                }
                let entry = palette.selected()?;
                if name == "enter"
                    && entry.state.as_deref() == Some("running")
                    && let Some(run_id) = entry.run_id
                {
                    self.palette = None;
                    self.palette_epoch += 1;
                    self.focus_catalog_run(&run_id);
                    return None;
                }
                palette.launching = true;
                palette.note = "Validating the current definition…".into();
                let epoch = self.palette_epoch;
                let scope = palette.scope;
                let cargo_global_aliases = palette.cargo_global_aliases;
                let registered = entry.registry_id.clone();
                let (dir, repo) = (self.cfg.dir.clone(), self.cfg.repo.clone());
                let invocation = self.cfg.invocation_dir.clone();
                let environment = self.cfg.catalog_environment.clone();
                let registry = self.cfg.registry.clone();
                let (discover, favorites, runs, runner) =
                    (self.cfg.discover, self.cfg.favorites.clone(), self.runs.clone(), self.cfg.runs.clone());
                tea::cmd_msg(move || {
                    let selected = dir;
                    let project = if scope == Scope::Registered {
                        match registered
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("selection has no registered project"))
                            .and_then(|id| registry.project(id))
                            .and_then(|project| {
                                project.check_directory()?;
                                Ok(project)
                            }) {
                            Ok(project) => Some(project),
                            Err(error) => {
                                return Launched {
                                    epoch,
                                    run: runui_runs::Run::default(),
                                    error: Some(error.to_string()),
                                    warning: None,
                                    fresh: None,
                                    discovery: None,
                                    scope,
                                };
                            }
                        }
                    } else {
                        None
                    };
                    let dir = project
                        .as_ref()
                        .map(|project| project.directory.clone())
                        .or_else(|| scope.directory(&selected, &repo))
                        .expect("selection context was resolved");
                    let repo = project.as_ref().map(|project| project.directory.clone()).unwrap_or(repo);
                    let discovery = discover(&dir, &repo);
                    let favorites =
                        if scope != Scope::Project { favorites.for_tree(&dir) } else { favorites.for_dir(&selected) };
                    let context = catalog_context(&dir, &repo, &invocation, &environment, cargo_global_aliases);
                    let mut fresh = build(&context, &discovery, &favorites, &runs, runner.as_ref());
                    if let Some(project) = project {
                        fresh = fresh
                            .with_registration(&project, runui_catalog::index::branch(&project.directory).as_deref());
                    }
                    match fresh.prepare(&entry.id, Some(&entry.fingerprint), &Inputs::default()) {
                        Ok(prepared) => {
                            let (run, error) = runner.start_with_shell(
                                &prepared.directory,
                                &prepared.name,
                                &prepared.command,
                                &prepared.shell,
                            );
                            let warning = if !run.id.is_empty() {
                                runner.usage_store().and_then(|store| {
                                    store.record(&run, Some(&prepared)).err().map(|error| error.to_string())
                                })
                            } else {
                                None
                            };
                            Launched {
                                epoch,
                                run,
                                error: error.map(|error| error.to_string()),
                                warning,
                                fresh: Some(fresh),
                                discovery: Some(discovery),
                                scope,
                            }
                        }
                        Err(error) => Launched {
                            epoch,
                            run: runui_runs::Run::default(),
                            error: Some(error.to_string()),
                            warning: None,
                            fresh: Some(fresh),
                            discovery: Some(discovery),
                            scope,
                        },
                    }
                })
            }
            _ => self.palette_input(&tea::msg(key.clone())),
        }
    }

    pub(crate) fn palette_inspected(&mut self, message: Inspected) -> Cmd {
        if message.epoch == self.palette_epoch
            && let Some(palette) = &mut self.palette
        {
            palette.native_busy = false;
            palette.native = Some((message.id, message.fingerprint, message.report));
        }
        None
    }

    pub(crate) fn palette_launched(&mut self, message: Launched) -> Cmd {
        if message.epoch != self.palette_epoch {
            return self.list_runs();
        }
        // This validation observed a newer source snapshot than any cached
        // palette load or periodic scan already in flight.
        self.palette_epoch += 1;
        self.discovery_generation += 1;
        self.discovery_busy = false;
        self.discovery_queued = false;
        if let Some(discovery) = message.discovery {
            if message.scope == Scope::Project {
                self.set_discovery(discovery);
            } else if message.scope == Scope::Workspace
                && let Some(palette) = &mut self.palette
            {
                let root = message
                    .fresh
                    .as_ref()
                    .and_then(|snapshot| snapshot.entries().first())
                    .map(|entry| entry.project_directory.clone());
                palette.workspace_discovery = root.map(|root| (root, Arc::new(discovery)));
            }
        }
        if let Some(error) = message.error {
            if let Some(palette) = &mut self.palette {
                palette.launching = false;
                palette.loading = false;
                palette.queued = false;
                if let Some(fresh) = message.fresh {
                    palette.set_snapshot(fresh);
                }
                palette.note = error;
            }
            return self.list_runs();
        }
        let run_id = message.run.id.clone();
        self.palette = None;
        self.palette_epoch += 1;
        let command = self.update(tea::msg(StartedMsg { run: message.run, err: None, warning: message.warning }));
        self.focus_catalog_run(&run_id);
        command
    }

    fn focus_catalog_run(&mut self, id: &str) {
        if let Some(run) = self.runs.iter().find(|run| run.id == id) {
            self.panels[PANEL_RUNS].cur = if self.here(run) { 0 } else { RUNS_EVERYWHERE };
            self.cur = PANEL_RUNS;
            self.focus = Focus::Pane;
            self.shown_mut().pick(&format!("run {id}"));
            self.sync_output();
        } else {
            self.say("Output is no longer retained; use Recent to repeat the invocation", false);
        }
    }
}
