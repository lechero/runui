//! Local shell recall is separate from catalog entries and result review.
//! The same browser serves the palette and a selection-only shell picker.

use crate::{
    palette::safe,
    styles::{S_ACCENT, S_BAD, S_DIM, S_HEADING, S_ROW_ACTIVE},
};
use runui_catalog::recall::{self, Data, Dialect, Entry, Prepared, Source};
use runui_tea::{
    ansi,
    bubbles::textinput,
    tea::{self, Cmd, KeyPressMsg, Msg, WindowSizeMsg},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Default)]
pub struct HistorySettings {
    pub sources: Vec<Source>,
    pub programs: Vec<(Dialect, String)>,
}
impl HistorySettings {
    pub fn current(home: &str) -> Self {
        let environment = runui_catalog::Environment::current();
        let directory = std::env::current_dir().unwrap_or_default().to_string_lossy().into_owned();
        let dialects = [Dialect::Fish, Dialect::Zsh, Dialect::Bash];
        Self {
            sources: dialects.iter().map(|&dialect| Source::current(dialect, home)).collect(),
            programs: dialects
                .into_iter()
                .filter_map(|dialect| {
                    environment.resolve(dialect.as_str(), &directory).map(|program| (dialect, program))
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Replay,
    Save,
    Terminal,
}
impl Purpose {
    fn label(self) -> &'static str {
        match self {
            Self::Replay => "Replay under supervision",
            Self::Save => "Save as a shared favorite",
            Self::Terminal => "Run in an external terminal",
        }
    }
}
pub(crate) enum Action {
    None,
    Command(Cmd),
    Close,
    Mode(bool),
    Copy(String),
    Selected(Entry),
    Prepared(Prepared, Purpose),
}
struct Loaded {
    instance: u64,
    generation: u64,
    data: Arc<Data>,
}
struct Validated {
    instance: u64,
    generation: u64,
    result: Result<Prepared, String>,
    purpose: Purpose,
}
struct Form {
    entry: Entry,
    directory: textinput::Model,
    purpose: Purpose,
}

pub(crate) struct Browser {
    settings: HistorySettings,
    source: usize,
    enabled: bool,
    pub(crate) busy: bool,
    picker: bool,
    instance: u64,
    generation: u64,
    cancel: Arc<AtomicBool>,
    data: Option<Arc<Data>>,
    hits: Vec<Entry>,
    at: usize,
    input: textinput::Model,
    detail: bool,
    diagnostics: bool,
    top: usize,
    note: String,
    error: String,
    directory: String,
    form: Option<Form>,
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Browser {
    pub(crate) fn new(settings: HistorySettings, directory: &str, picker: bool) -> Self {
        let mut input = textinput::new();
        input.prompt = "> ".into();
        input.placeholder = "Search private shell command text…".into();
        let mut styles = input.styles();
        styles.cursor.blink = false;
        input.set_styles(styles);
        input.focus();
        Self {
            settings,
            source: 0,
            enabled: picker,
            busy: false,
            picker,
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            data: None,
            hits: Vec::new(),
            at: 0,
            input,
            detail: false,
            diagnostics: false,
            top: 0,
            note: String::new(),
            error: String::new(),
            directory: directory.into(),
            form: None,
        }
    }
    fn source(&self) -> Source {
        self.settings.sources.get(self.source).cloned().unwrap_or(Source { dialect: Dialect::Fish, path: None })
    }
    pub(crate) fn set_query(&mut self, text: &str) {
        self.input.set_value(text);
        self.query();
    }
    fn query(&mut self) {
        let id = self.hits.get(self.at).map(|entry| entry.id.clone());
        self.hits = self
            .data
            .as_ref()
            .map(|data| data.search(&self.input.value(), 500, 0).into_iter().cloned().collect())
            .unwrap_or_default();
        self.at = id.and_then(|id| self.hits.iter().position(|entry| entry.id == id)).unwrap_or(0);
    }
    fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        self.busy = true;
        (self.generation, self.cancel.clone())
    }
    pub(crate) fn refresh(&mut self) -> Cmd {
        if !self.enabled || self.busy || self.form.is_some() || self.data.as_ref().is_some_and(|data| !data.changed()) {
            return None;
        }
        let (generation, cancel) = self.begin();
        let source = self.source();
        let instance = self.instance;
        tea::cmd_msg(move || Loaded { instance, generation, data: Arc::new(recall::load(&source, &cancel)) })
    }
    pub(crate) fn receive(&mut self, msg: &Msg) -> Option<Action> {
        if let Some(loaded) = msg.downcast_ref::<Loaded>() {
            if loaded.instance == self.instance && loaded.generation == self.generation && self.enabled {
                self.busy = false;
                self.data = Some(loaded.data.clone());
                self.query();
            }
            return Some(Action::None);
        }
        if let Some(validated) = msg.downcast_ref::<Validated>() {
            if validated.instance != self.instance || validated.generation != self.generation {
                return Some(Action::None);
            }
            self.busy = false;
            return Some(match &validated.result {
                Ok(prepared) => {
                    self.form = None;
                    Action::Prepared(prepared.clone(), validated.purpose)
                }
                Err(error) => {
                    self.error = error.clone();
                    Action::None
                }
            });
        }
        None
    }
    pub(crate) fn notify(&mut self, message: &str, bad: bool) {
        if bad { self.error = message.into() } else { self.note = message.into() }
    }
    pub(crate) fn key(&mut self, key: &KeyPressMsg) -> Action {
        self.error.clear();
        let name = key.string();
        if self.busy && self.form.is_none() && matches!(name.as_str(), "alt+r" | "alt+m" | "ctrl+t") {
            return Action::None;
        }
        if let Some(form) = &mut self.form {
            if name == "esc" {
                self.cancel.store(true, Ordering::Relaxed);
                self.generation += 1;
                self.busy = false;
                self.form = None;
                return Action::None;
            }
            if self.busy {
                return Action::None;
            }
            if name == "enter" {
                let entry = form.entry.clone();
                let directory = form.directory.value();
                let purpose = form.purpose;
                let Some((_, program)) = self.settings.programs.iter().find(|(dialect, _)| *dialect == entry.dialect)
                else {
                    self.error = "Matching shell unavailable; copy or insert into a matching interactive shell.".into();
                    return Action::None;
                };
                let program = program.clone();
                let source = self.source();
                let instance = self.instance;
                let (generation, cancel) = self.begin();
                return Action::Command(tea::cmd_msg(move || Validated {
                    instance,
                    generation,
                    purpose,
                    result: recall::prepare(&source, &entry.id, &directory, &program, &cancel)
                        .map_err(|error| error.to_string()),
                }));
            }
            return Action::Command(form.directory.update(&tea::msg(key.clone())));
        }
        match name.as_str() {
            "esc" | "ctrl+p" => Action::Close,
            "tab" => Action::Mode(false),
            "shift+tab" => Action::Mode(true),
            "alt+h" => {
                self.enabled = !self.enabled;
                self.cancel.store(true, Ordering::Relaxed);
                self.generation += 1;
                self.busy = false;
                self.data = None;
                self.hits.clear();
                self.at = 0;
                self.note = if self.enabled {
                    "History enabled for this session only"
                } else {
                    "History disabled; private entries discarded"
                }
                .into();
                Action::Command(self.refresh())
            }
            "alt+b" if !self.busy => {
                self.source = (self.source + 1) % self.settings.sources.len().max(1);
                self.data = None;
                self.hits.clear();
                self.at = 0;
                self.top = 0;
                Action::Command(self.refresh())
            }
            "ctrl+r" if !self.busy => {
                self.data = None;
                Action::Command(self.refresh())
            }
            "ctrl+v" => {
                self.detail = !self.detail;
                self.diagnostics = false;
                self.top = 0;
                Action::None
            }
            "f2" => {
                self.diagnostics = !self.diagnostics;
                self.detail = self.diagnostics;
                self.top = 0;
                Action::None
            }
            "up" if self.detail => {
                self.top = self.top.saturating_sub(1);
                Action::None
            }
            "down" if self.detail => {
                self.top += 1;
                Action::None
            }
            "pgup" if self.detail => {
                self.top = self.top.saturating_sub(10);
                Action::None
            }
            "pgdown" if self.detail => {
                self.top += 10;
                Action::None
            }
            "up" => {
                self.at = self.at.saturating_sub(1);
                Action::None
            }
            "down" => {
                self.at = (self.at + 1).min(self.hits.len().saturating_sub(1));
                Action::None
            }
            "pgup" => {
                self.at = self.at.saturating_sub(10);
                Action::None
            }
            "pgdown" => {
                self.at = (self.at + 10).min(self.hits.len().saturating_sub(1));
                Action::None
            }
            "enter" | "ctrl+y" => {
                let Some(entry) = self.hits.get(self.at).cloned() else { return Action::None };
                if self.picker && name == "enter" {
                    Action::Selected(entry)
                } else {
                    self.note = "Copied exact text; edit it in the matching shell before running".into();
                    Action::Copy(entry.command)
                }
            }
            "alt+r" | "alt+m" | "ctrl+t" if !self.picker => {
                let Some(entry) = self.hits.get(self.at).cloned() else { return Action::None };
                let mut directory = textinput::new();
                directory.prompt = "cwd: ".into();
                directory.set_value(&self.directory);
                directory.focus();
                self.form = Some(Form {
                    entry,
                    directory,
                    purpose: match name.as_str() {
                        "alt+m" => Purpose::Save,
                        "ctrl+t" => Purpose::Terminal,
                        _ => Purpose::Replay,
                    },
                });
                Action::None
            }
            "alt+s" | "alt+p" => {
                self.note =
                    "Shell history has unknown cwd and stays in All history; Tab returns to command scopes.".into();
                Action::None
            }
            _ => {
                let cmd = self.input.update(&tea::msg(key.clone()));
                self.query();
                Action::Command(cmd)
            }
        }
    }
    pub(crate) fn paste(&mut self, msg: &Msg) -> Cmd {
        if self.busy && self.form.is_some() {
            return None;
        }
        if let Some(form) = &mut self.form {
            form.directory.update(msg)
        } else {
            let cmd = self.input.update(msg);
            self.query();
            cmd
        }
    }
    fn preview(&self, width: i32) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.enabled {
            lines.extend([
                "Shell history is off.".into(),
                "Alt+H enables the selected source for this session.".into(),
                "Alt+B chooses fish / Zsh / Bash before enabling.".into(),
                "Ordinary search and MCP never read these files.".into(),
            ]);
            lines.push(format!(
                "Selected file: {}",
                self.source()
                    .path
                    .map(|path| safe(&path.to_string_lossy()))
                    .unwrap_or_else(|| "disabled/private session".into())
            ));
        } else if self.diagnostics {
            lines.push("Private history diagnostics".into());
            if let Some(data) = &self.data {
                lines.push(format!(
                    "{} bytes read · {} distinct records · no persistent import",
                    data.bytes_read,
                    data.entries.len()
                ));
                lines.extend(data.diagnostics.iter().map(|value| safe(value)));
            }
        } else if let Some(entry) = self.hits.get(self.at) {
            lines.push(format!("{} history · cwd unknown · outcome unknown", entry.dialect.as_str()));
            lines.push("Selection never executes a command.".into());
            lines.push(format!(
                "Recorded: {}",
                entry.timestamp.map(|time| time.to_rfc3339()).unwrap_or_else(|| "unknown".into())
            ));
            if let Some(duration) = entry.duration_seconds {
                lines.push(format!("Source duration: {duration}s; no exit code recorded"));
            }
            lines.push(format!("Original {} text:", entry.dialect.as_str()));
            lines.extend(entry.command.split('\n').map(safe));
            if self.detail {
                lines.push(format!("file: {}", safe(&entry.source_file)));
                lines.push(format!("id: {}", entry.id));
                if !entry.metadata_fields.is_empty() {
                    lines.push(format!(
                        "Uninterpreted metadata: {}",
                        entry.metadata_fields.iter().take(20).map(|field| safe(field)).collect::<Vec<_>>().join(", ")
                    ));
                }
                lines.push("Native history paths are argument paths, not cwd.".into());
            }
            lines.push(
                if self.picker {
                    "Enter returns text to your matching shell for editing."
                } else {
                    "Enter / Ctrl+Y copy · Alt+R replay · Alt+M save · Ctrl+T terminal"
                }
                .into(),
            );
        } else {
            lines.push(
                if self.busy { "Reading the bounded history tail…" } else { "No matching history records." }.into(),
            );
            if let Some(data) = &self.data {
                lines.extend(data.diagnostics.iter().take(6).map(|value| safe(value)));
            }
        }
        lines
            .into_iter()
            .flat_map(|line| ansi::wrap(&line, width.max(1), "/").split('\n').map(str::to_string).collect::<Vec<_>>())
            .collect()
    }
    pub(crate) fn draw(&mut self, width: i32, height: i32) -> String {
        let width = width.max(1);
        let height = height.max(1) as usize;
        let body = height.saturating_sub(6);
        if let Some(form) = &mut self.form {
            form.directory.set_width((width - 6).max(1));
            let mut lines = vec![
                S_HEADING.render(form.purpose.label()),
                format!("Original {} command · original cwd unknown", form.entry.dialect.as_str()),
                safe(&form.entry.command),
                String::new(),
                form.directory.view(),
                "Choose the directory explicitly; this does not recover the original cwd.".into(),
                "Shell startup, functions, aliases and environment may differ from the original session.".into(),
                "Enter confirms the shown action · Esc cancels".into(),
            ];
            if matches!(form.purpose, Purpose::Save) {
                lines.insert(7, "Saving deliberately shares this command through Favorites and MCP.".into());
            }
            if self.busy {
                lines.push("Revalidating the current history record and shell…".into());
            }
            if !self.error.is_empty() {
                lines.push(S_BAD.render(&safe(&self.error)));
            }
            return lines
                .into_iter()
                .take(height)
                .map(|line| ansi::truncate(&line, width, "…"))
                .collect::<Vec<_>>()
                .join("\n");
        }
        self.input.set_width((width - 3).max(1));
        let source = self.source();
        let mut lines = vec![
            S_HEADING.render(&format!(
                "Shell history · {} · All history (cwd unknown){}",
                source.dialect.as_str(),
                if self.enabled { "" } else { " · off" }
            )),
            self.input.view(),
            S_DIM.render(&"─".repeat(width as usize)),
        ];
        let mut rows = Vec::new();
        let list_height = if width >= 90 && !self.detail { body } else { (body / 2).max(1) };
        let list_width = if width >= 90 && !self.detail { (width * 45 / 100).max(1) } else { width };
        let start = self.at.saturating_sub(list_height.saturating_sub(1));
        for (index, entry) in self.hits.iter().enumerate().skip(start).take(list_height) {
            let row = ansi::truncate(
                &format!(
                    "{}  · {}",
                    safe(&entry.command),
                    entry
                        .timestamp
                        .map(|time| time.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "time unknown".into())
                ),
                list_width,
                "…",
            );
            rows.push(if index == self.at { S_ROW_ACTIVE.render(&row) } else { row });
        }
        let preview_width = if width >= 90 && !self.detail { (width - list_width - 3).max(1) } else { width };
        let preview = self.preview(preview_width);
        if self.detail {
            self.top = self.top.min(preview.len().saturating_sub(body));
            lines.extend(preview.into_iter().skip(self.top).take(body));
        } else if width >= 90 {
            for at in 0..body {
                let row = rows.get(at).cloned().unwrap_or_default();
                let pad = " ".repeat((list_width - ansi::string_width(&row)).max(0) as usize);
                lines.push(format!(
                    "{row}{pad} {} {}",
                    S_DIM.render("│"),
                    preview.get(at).cloned().unwrap_or_default()
                ));
            }
        } else {
            lines.extend(rows);
            lines.push(S_DIM.render(&"─".repeat(width as usize)));
            lines.extend(preview.into_iter().take(body.saturating_sub(list_height + 1)));
        }
        while lines.len() < height.saturating_sub(3) {
            lines.push(String::new())
        }
        lines.push(S_DIM.render(&"─".repeat(width as usize)));
        lines.push(if !self.error.is_empty() {
            S_BAD.render(&safe(&self.error))
        } else {
            S_ACCENT.render(&format!(
                "{}{} matches · Alt+H {} · Alt+B source · F2 diagnostics",
                if self.note.is_empty() { String::new() } else { format!("{} · ", safe(&self.note)) },
                self.hits.len(),
                if self.enabled { "disable" } else { "enable" }
            ))
        });
        lines.push(S_DIM.render("Enter copy/insert · Ctrl+V details · Ctrl+R refresh · Tab mode · Esc back"));
        lines.into_iter().take(height).map(|line| ansi::truncate(&line, width, "…")).collect::<Vec<_>>().join("\n")
    }
}

/// Selection-only picker: owns no run, favorites, usage, catalog or review store.
pub struct ShellPicker {
    browser: Browser,
    width: i32,
    height: i32,
    selection: Option<Entry>,
}
impl ShellPicker {
    pub fn new(settings: HistorySettings, directory: &str, query: &str) -> Self {
        let mut browser = Browser::new(settings, directory, true);
        browser.set_query(query);
        Self { browser, width: 80, height: 24, selection: None }
    }
    pub fn selection(&self) -> Option<&Entry> {
        self.selection.as_ref()
    }
}
impl tea::Model for ShellPicker {
    fn init(&mut self) -> Cmd {
        self.browser.refresh()
    }
    fn update(&mut self, msg: Msg) -> Cmd {
        if let Some(size) = msg.downcast_ref::<WindowSizeMsg>() {
            self.width = size.width;
            self.height = size.height;
            return None;
        }
        let action = if let Some(action) = self.browser.receive(&msg) {
            action
        } else if let Some(key) = msg.downcast_ref::<KeyPressMsg>() {
            if key.string() == "ctrl+c" { Action::Close } else { self.browser.key(key) }
        } else if msg.is::<tea::PasteMsg>() {
            return self.browser.paste(&msg);
        } else {
            Action::None
        };
        match action {
            Action::Command(cmd) => cmd,
            Action::Copy(command) => tea::set_clipboard(command),
            Action::Close => tea::quit(),
            Action::Selected(entry) => {
                self.selection = Some(entry);
                tea::quit()
            }
            _ => None,
        }
    }
    fn view(&mut self) -> tea::View {
        let mut view = tea::new_view(self.browser.draw(self.width, self.height));
        view.alt_screen = true;
        view.window_title = "runui · private shell selection".into();
        view
    }
}
