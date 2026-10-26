//! The command palette owns its query and selection; opening it never moves
//! panel cursors or reads their output. Catalog work happens in Tea commands.

use std::sync::Arc;

use runui_catalog::{Context, Entry, Inputs, Kind, Query, Snapshot};
use runui_tea::ansi;
use runui_tea::bubbles::textinput;
use runui_tea::tea::{self, Cmd, KeyPressMsg, Msg};

use crate::styles::{S_ACCENT, S_BAD, S_DIM, S_HEADING, S_ROW_ACTIVE, S_TITLE};
use crate::{App, Focus, PANEL_RUNS, RUNS_EVERYWHERE, StartedMsg};

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
    pub(crate) loading: bool,
    queued: bool,
    launching: bool,
    detail: bool,
    detail_top: usize,
    note: String,
    error: String,
}

pub(crate) struct Loaded {
    epoch: u64,
    snapshot: Snapshot,
}
pub(crate) struct Launched {
    epoch: u64,
    run: runui_runs::Run,
    error: Option<String>,
    warning: Option<String>,
    fresh: Option<Snapshot>,
    discovery: Option<runui_project::Discovery>,
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
            loading: false,
            queued: false,
            launching: false,
            detail: false,
            detail_top: 0,
            note: String::new(),
            error: String::new(),
        }
    }

    fn query(&mut self) {
        let selected = self.hits.get(self.at).map(|entry| entry.id.clone());
        match Query::parse(&self.input.value()) {
            Ok(mut query) => {
                query.kinds = match self.mode {
                    Mode::Commands => vec![Kind::Command, Kind::Favorite],
                    Mode::Recent => vec![Kind::Recent],
                };
                self.hits = self.snapshot.search(&query).into_iter().map(|hit| hit.entry.clone()).collect();
                self.at = selected.and_then(|id| self.hits.iter().position(|entry| entry.id == id)).unwrap_or(0);
                self.error.clear();
            }
            Err(error) => {
                self.error = error;
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
        let mut lines = vec![
            S_HEADING.render(&format!("{mode} · Project{status}")),
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
        let information = if !self.error.is_empty() {
            S_BAD.render(&safe(&self.error))
        } else if !self.note.is_empty() {
            S_ACCENT.render(&safe(&self.note))
        } else {
            S_DIM.render(&format!(
                "{} matches · {} discovery diagnostics · ctrl+v details",
                self.hits.len(),
                self.snapshot.diagnostics.len()
            ))
        };
        lines.push(information);
        lines.push(S_DIM.render(
            "enter run/focus · tab mode · alt+r another · alt+m favorite · ctrl+y copy · ctrl+o source · ctrl+t terminal · esc back",
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
        let top = self.at.saturating_sub(height.saturating_sub(1));
        let mut lines = Vec::new();
        for (index, entry) in self.hits.iter().enumerate().skip(top).take(height) {
            let favorite = if entry.favorite { "★ " } else { "  " };
            let row =
                format!("{favorite}{}  · {}  · {}", safe(&entry.name), entry.source, safe(&entry.relative_directory));
            let row = ansi::truncate(&row, width, "…");
            lines.push(if index == self.at { S_ROW_ACTIVE.render(&row) } else { row });
        }
        lines.resize(height, String::new());
        lines
    }

    fn preview(&self, width: i32) -> Vec<String> {
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
            format!("{} · {:?} · {:?}", entry.source, entry.origin, entry.availability),
        ];
        if let Some(file) = &entry.entry_file {
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
        if !entry.description.is_empty() {
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
        lines.push(format!("id: {}", entry.id));
        wrapped(lines, width)
    }
}

fn wrapped(lines: Vec<String>, width: i32) -> Vec<String> {
    lines
        .into_iter()
        .flat_map(|line| ansi::hardwrap(&line, width, true).split('\n').map(str::to_string).collect::<Vec<_>>())
        .collect()
}
pub(crate) fn safe(text: &str) -> String {
    text.chars()
        .flat_map(|ch| if ch.is_control() { ch.escape_default().collect::<Vec<_>>() } else { vec![ch] })
        .collect()
}

pub(crate) fn build(
    dir: &str,
    repo: &str,
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
    let mut context = Context::new(dir, repo);
    context.environment = context.environment.with_shell(&runner.execution_shell());
    let default = runui_catalog::usage::Data::default();
    let mut snapshot = Snapshot::build(
        &context,
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
        let epoch = self.palette_epoch;
        let (dir, repo) = (self.cfg.dir.clone(), self.cfg.repo.clone());
        let (discovery, favorites, runs, runner) =
            (self.discovery.clone(), self.cfg.favorites.for_dir(&dir), self.runs.clone(), self.cfg.runs.clone());
        tea::cmd_msg(move || Loaded {
            epoch,
            snapshot: build(&dir, &repo, &discovery, &favorites, &runs, runner.as_ref()),
        })
    }

    pub(crate) fn palette_loaded(&mut self, message: Loaded) -> Cmd {
        if message.epoch != self.palette_epoch {
            return None;
        }
        let palette = self.palette.as_mut()?;
        palette.loading = false;
        if palette.queued {
            palette.queued = false;
            return self.refresh_palette();
        }
        palette.set_snapshot(message.snapshot);
        None
    }

    pub(crate) fn palette_input(&mut self, message: &Msg) -> Cmd {
        let palette = self.palette.as_mut()?;
        let command = palette.input.update(message);
        palette.query();
        command
    }

    pub(crate) fn palette_key(&mut self, key: &KeyPressMsg) -> Cmd {
        let name = key.string();
        let palette = self.palette.as_mut()?;
        match name.as_str() {
            "esc" | "ctrl+p" => {
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
            "ctrl+v" => {
                palette.detail = !palette.detail;
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
                let relative =
                    runui_core::golang::filepath::rel(&runui_catalog::canonical(&self.cfg.dir), &entry.directory)
                        .unwrap_or_default();
                let favorite = runui_favorites::Favorite {
                    name: entry.name,
                    command: entry.command,
                    source: if entry.kind == Kind::Command { entry.source } else { String::new() },
                    dir: if relative == "." { String::new() } else { relative },
                    added: None,
                };
                match self.cfg.favorites.toggle(&self.cfg.dir, favorite) {
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
                let (dir, repo) = (self.cfg.dir.clone(), self.cfg.repo.clone());
                let (discover, favorites, runs, runner) =
                    (self.cfg.discover, self.cfg.favorites.for_dir(&dir), self.runs.clone(), self.cfg.runs.clone());
                tea::cmd_msg(move || {
                    let discovery = discover(&dir, &repo);
                    let fresh = build(&dir, &repo, &discovery, &favorites, &runs, runner.as_ref());
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
                            }
                        }
                        Err(error) => Launched {
                            epoch,
                            run: runui_runs::Run::default(),
                            error: Some(error.to_string()),
                            warning: None,
                            fresh: Some(fresh),
                            discovery: Some(discovery),
                        },
                    }
                })
            }
            _ => self.palette_input(&tea::msg(key.clone())),
        }
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
            self.set_discovery(discovery);
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
