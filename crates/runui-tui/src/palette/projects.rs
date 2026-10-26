//! A checkout picker stays separate from the command query. Search selection
//! changes palette breadth; opening a project is a separate explicit action.

use super::*;
use crate::{PANEL_FAVORITES, PANEL_PROJECT};
use runui_catalog::{index::Indexed, registry::Project};

pub(super) struct Picker {
    input: textinput::Model,
    adding: Option<textinput::Model>,
    at: usize,
    detail: bool,
    top: usize,
}

impl Picker {
    pub(super) fn new() -> Self {
        Self { input: input("Projects > "), adding: None, at: 0, detail: false, top: 0 }
    }
    pub(super) fn update(&mut self, message: &Msg) -> Cmd {
        let command = self.adding.as_mut().unwrap_or(&mut self.input).update(message);
        if self.adding.is_none() {
            self.at = 0;
        }
        command
    }
    fn matches<'a>(&self, projects: &'a [Indexed]) -> Vec<&'a Indexed> {
        let query = self.input.value().to_lowercase();
        projects
            .iter()
            .filter(|item| {
                format!(
                    "{} {} {}",
                    item.project.name,
                    item.project.directory,
                    item.branch.as_deref().unwrap_or_default()
                )
                .to_lowercase()
                .contains(&query)
            })
            .collect()
    }
    fn selected<'a>(&self, projects: &'a [Indexed]) -> Option<&'a Indexed> {
        self.at.checked_sub(1).and_then(|at| self.matches(projects).get(at).copied())
    }
}

fn input(prompt: &str) -> textinput::Model {
    let mut input = textinput::new();
    input.prompt = prompt.into();
    let mut styles = input.styles();
    styles.cursor.blink = false;
    input.set_styles(styles);
    let _ = input.focus();
    input
}

pub(super) fn tail(text: &str, width: i32) -> String {
    let length = ansi::string_width(text);
    if length <= width { text.into() } else { ansi::truncate_left(text, (length - width + 1).max(0), "…") }
}

impl Palette {
    pub(super) fn draw_projects(&mut self, width: i32, height: i32) -> String {
        let width = width.max(1);
        let height = height.max(1) as usize;
        let picker = self.picker.as_mut().expect("picker draw was requested");
        let active = picker.adding.as_mut().unwrap_or(&mut picker.input);
        active.set_width((width - 17).max(1));
        let mut lines =
            vec![S_HEADING.render("Registered projects"), active.view(), S_DIM.render(&"─".repeat(width as usize))];
        let available = height.saturating_sub(6);
        let rows = picker.matches(&self.projects);
        picker.at = picker.at.min(rows.len());
        if picker.adding.is_some() {
            lines.extend(
                wrapped(
                    vec![
                        "Enter a directory to register its checkout or enclosing workspace.".into(),
                        "Use an absolute path or ~/path. This only saves the registration.".into(),
                        "Folders are added explicitly; opening runui does not register them.".into(),
                        "Enter saves · Esc cancels".into(),
                    ],
                    width,
                )
                .into_iter()
                .take(available),
            );
        } else if picker.detail {
            let details = if let Some(selected) = picker.selected(&self.projects) {
                vec![
                    selected.project.name.clone(),
                    format!("checkout: {}", safe(&selected.project.directory)),
                    format!("branch: {}", safe(selected.branch.as_deref().unwrap_or("unknown"))),
                    format!("id: {}", selected.project.id),
                    format!("status: {}", status(selected)),
                    selected.error.as_deref().map(safe).unwrap_or_default(),
                    "Enter searches this checkout without changing the project panels.".into(),
                    "Ctrl+O opens this project in the main panels.".into(),
                    "Alt+A adds a directory; Alt+D forgets the selected registration.".into(),
                    "Forgetting keeps project files, favorites, runs and output.".into(),
                ]
            } else {
                vec![
                    "Search all explicitly registered checkouts.".into(),
                    "Alt+A registers a directory; no home-directory scan is performed.".into(),
                    "Esc returns to the command query.".into(),
                ]
            };
            let mut details = details;
            if !self.error.is_empty() {
                details.push(format!("Registry error: {}", safe(&self.error)));
            }
            let details = wrapped(details, width);
            picker.top = picker.top.min(details.len().saturating_sub(available));
            lines.extend(details.into_iter().skip(picker.top).take(available));
        } else {
            let count = (available / 2).max(1);
            let first = picker.at.saturating_sub(count.saturating_sub(1));
            for position in first..(first + count).min(rows.len() + 1) {
                let (label, path) = if position == 0 {
                    (
                        "All registered checkouts".into(),
                        format!("{} explicit checkouts · Enter searches all", self.projects.len()),
                    )
                } else {
                    let item = rows[position - 1];
                    (
                        format!(
                            "{} [{}] · {}",
                            safe(&item.project.name),
                            safe(item.branch.as_deref().unwrap_or("branch unknown")),
                            status(item)
                        ),
                        tail(&safe(&item.project.directory), width),
                    )
                };
                let label = ansi::truncate(&label, width, "…");
                lines.push(if picker.at == position { S_ROW_ACTIVE.render(&label) } else { label });
                lines.push(S_DIM.render(&path));
            }
        }
        while lines.len() < height.saturating_sub(3) {
            lines.push(String::new());
        }
        lines.push(S_DIM.render(&"─".repeat(width as usize)));
        lines.push(if self.error.is_empty() {
            S_ACCENT.render(if self.note.is_empty() {
                "ctrl+v details · registrations are explicit"
            } else {
                &self.note
            })
        } else {
            S_BAD.render(&safe(&self.error))
        });
        lines.push(S_DIM.render("enter search · ctrl+o open · alt+a add · alt+d forget · esc back"));
        lines.into_iter().take(height).map(|line| ansi::truncate(&line, width, "…")).collect::<Vec<_>>().join("\n")
    }
}

fn status(project: &Indexed) -> &'static str {
    if project.error.is_some() {
        "unavailable"
    } else if project.pending {
        "indexing"
    } else {
        "ready"
    }
}

pub(crate) struct RegistryChanged {
    epoch: u64,
    result: Result<String, String>,
}
pub(crate) struct ProjectOpened {
    epoch: u64,
    result: Result<(Project, runui_project::Discovery), String>,
}

impl App {
    pub(crate) fn project_picker_key(&mut self, key: &KeyPressMsg) -> Cmd {
        let palette = self.palette.as_mut()?;
        let picker = palette.picker.as_mut()?;
        let name = key.string();
        if let Some(adding) = &mut picker.adding {
            return match name.as_str() {
                "esc" => {
                    picker.adding = None;
                    None
                }
                "enter" => {
                    let directory = adding.value();
                    let directory = if directory == "~" {
                        self.cfg.home.clone()
                    } else if let Some(path) = directory.strip_prefix("~/") {
                        std::path::Path::new(&self.cfg.home).join(path).to_string_lossy().into_owned()
                    } else {
                        directory
                    };
                    let (registry, home, epoch) =
                        (self.cfg.registry.clone(), self.cfg.home.clone(), self.palette_epoch);
                    tea::cmd_msg(move || RegistryChanged {
                        epoch,
                        result: registry
                            .add(&directory, &home, None)
                            .map(|project| format!("Registered {}", project.name))
                            .map_err(|error| format!("{error:#}")),
                    })
                }
                _ => {
                    palette.error.clear();
                    picker.update(&tea::msg(key.clone()))
                }
            };
        }
        match name.as_str() {
            "esc" | "alt+p" => {
                palette.picker = None;
                None
            }
            "ctrl+p" => {
                self.cfg.index.cancel_pending();
                self.palette = None;
                self.palette_epoch += 1;
                None
            }
            "ctrl+v" => {
                picker.detail = !picker.detail;
                picker.top = 0;
                None
            }
            "up" | "pgup" if picker.detail => {
                picker.top = picker.top.saturating_sub(if name == "up" { 1 } else { 10 });
                None
            }
            "down" | "pgdown" if picker.detail => {
                picker.top = picker.top.saturating_add(if name == "down" { 1 } else { 10 });
                None
            }
            "up" | "pgup" => {
                picker.at = picker.at.saturating_sub(if name == "up" { 1 } else { 10 });
                None
            }
            "down" | "pgdown" => {
                picker.at =
                    (picker.at + if name == "down" { 1 } else { 10 }).min(picker.matches(&palette.projects).len());
                None
            }
            "alt+a" => {
                let mut adding = input("Add checkout > ");
                adding.set_value(&self.cfg.dir);
                picker.adding = Some(adding);
                None
            }
            "alt+d" => {
                let project = picker.selected(&palette.projects)?.project.clone();
                let (registry, epoch) = (self.cfg.registry.clone(), self.palette_epoch);
                tea::cmd_msg(move || RegistryChanged {
                    epoch,
                    result: registry
                        .forget(&project.id)
                        .map(|_| format!("Forgot {} · files, favorites and runs are kept", project.name))
                        .map_err(|error| format!("{error:#}")),
                })
            }
            "enter" => {
                palette.project_filter = picker.selected(&palette.projects).map(|item| item.project.id.clone());
                palette.scope = Scope::Registered;
                palette.picker = None;
                palette.loading = false;
                palette.queued = false;
                palette.set_snapshot(Snapshot::default());
                palette.note = "Searching registered context; each command retains its own cwd".into();
                self.palette_epoch += 1;
                self.refresh_palette()
            }
            "ctrl+o" => {
                let project = picker.selected(&palette.projects)?.project.clone();
                let epoch = self.palette_epoch;
                tea::cmd_msg(move || ProjectOpened {
                    epoch,
                    result: project
                        .check_directory()
                        .map(|_| {
                            let discovery = runui_project::discover(&project.directory, &project.directory);
                            (project, discovery)
                        })
                        .map_err(|error| format!("{error:#}")),
                })
            }
            _ => self.palette_input(&tea::msg(key.clone())),
        }
    }

    pub(crate) fn registry_changed(&mut self, message: RegistryChanged) -> Cmd {
        if message.epoch != self.palette_epoch {
            return None;
        }
        let palette = self.palette.as_mut()?;
        match message.result {
            Ok(note) => {
                palette.notify(&note, false);
                if let Some(picker) = &mut palette.picker {
                    picker.adding = None;
                }
            }
            Err(error) => {
                palette.notify(&error, true);
                return None;
            }
        }
        palette.loading = false;
        palette.queued = false;
        self.palette_epoch += 1;
        self.cfg.index.invalidate();
        self.refresh_palette()
    }

    pub(crate) fn project_opened(&mut self, message: ProjectOpened) -> Cmd {
        if message.epoch != self.palette_epoch {
            return None;
        }
        let (project, discovery) = match message.result {
            Ok(value) => value,
            Err(error) => {
                if let Some(palette) = &mut self.palette {
                    palette.notify(&error, true);
                }
                return None;
            }
        };
        self.cfg.index.cancel_pending();
        self.palette = None;
        self.palette_epoch += 1;
        self.discovery_generation += 1;
        self.discovery_busy = false;
        self.discovery_queued = false;
        self.cfg.dir = project.directory.clone();
        self.cfg.repo = project.directory.clone();
        self.cfg.invocation_dir = project.directory;
        let favorites_error = self.cfg.favorites.reload().err();
        self.panels[PANEL_PROJECT].views.clear();
        self.panels[PANEL_FAVORITES].views[0].set_filter("");
        self.set_discovery(discovery);
        self.cur = PANEL_PROJECT;
        self.focus = Focus::Side;
        self.set_runs(self.runs.clone());
        self.sync_output();
        if let Some(error) = favorites_error {
            self.say(&format!("Opened {}; favorites: {error}", project.name), true);
        } else {
            self.say(&format!("Opened {}", project.name), false);
        }
        self.list_runs()
    }
}
