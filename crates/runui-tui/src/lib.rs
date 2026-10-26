//! runui's interface, laid out as aem-cli's and lazygit's are: the panels
//! stacked in a sidebar on the left, 1 the folder's favorites, 2 the
//! project's own commands (a tab for each kind of file: Taskfile,
//! package.json, Makefile, Compose, Maven, with the ones in the folders
//! under it, three deep) and 3 the runs (here, everywhere, and the ones
//! agents started through runui mcp, once there are some), 4 the Log of
//! finished runs from every folder (unread results and all history). On
//! the right the pane shows what the row under the cursor runs and, once it
//! has run, its output, followed as it's printed. A command runs under a
//! supervisor of its own and keeps running once runui quits; the next
//! runui finds it, its output and all, and stops it. Ported from
//! internal/runui/tui, model.go here and the rest file for file.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use runui_core::golang::filepath;
use runui_favorites::{Favorite, Store as Favorites};
use runui_project::{self as project, Discovery, File};
use runui_runs::{self as runs, Output, Run};
use runui_tea::bubbles::{help, key, textinput};
use runui_tea::tea::{
    self, Cmd, KeyPressMsg, Mouse, MouseButton, MouseClickMsg, MouseMode, MouseWheelMsg, Msg, PasteMsg, WindowSizeMsg,
};

mod rows;
mod styles;
mod view;

use rows::{Item, Kind, View};
use styles::{KeyMap, S_ACCENT, S_TITLE, new_key_map};

/// Runner starts, finds and stops the runs: a runs::Store, or a fake in
/// tests.
pub trait Runner: Send + Sync {
    fn start(&self, dir: &str, name: &str, command: &str) -> (Run, Option<anyhow::Error>);
    fn list(&self) -> anyhow::Result<Vec<Run>>;
    fn stop(&self, r: &Run) -> anyhow::Result<()>;
    fn remove(&self, r: &Run) -> anyhow::Result<()>;
    fn acknowledge(&self, r: &Run) -> anyhow::Result<()>;
    fn pending_result(&self, r: &Run) -> bool;
}

impl Runner for runs::Store {
    fn start(&self, dir: &str, name: &str, command: &str) -> (Run, Option<anyhow::Error>) {
        runs::Store::start(self, dir, name, command)
    }
    fn list(&self) -> anyhow::Result<Vec<Run>> {
        runs::Store::list(self)
    }
    fn stop(&self, r: &Run) -> anyhow::Result<()> {
        runs::Store::stop(self, r)
    }
    fn remove(&self, r: &Run) -> anyhow::Result<()> {
        runs::Store::remove(self, r)
    }
    fn acknowledge(&self, r: &Run) -> anyhow::Result<()> {
        runs::Store::acknowledge(self, r)
    }
    fn pending_result(&self, r: &Run) -> bool {
        runs::Store::pending_result(self, r)
    }
}

/// Config is what the interface works on.
pub struct Config {
    /// the folder: the project runui was opened in
    pub dir: String,
    /// its repository's root, where package.json's lock files are looked for up to
    pub repo: String,
    /// the home folder, written ~
    pub home: String,
    pub favorites: Favorites,
    pub runs: Arc<dyn Runner>,
    /// project::discover, unless a test gives its own
    pub discover: fn(&str, &str) -> Discovery,
    pub now: fn() -> DateTime<Utc>,
}

impl Config {
    /// new is the interface over a folder, its project read as runui reads
    /// it, the time the clock's.
    pub fn new(dir: &str, repo: &str, home: &str, favorites: Favorites, runs: Arc<dyn Runner>) -> Config {
        Config {
            dir: dir.to_string(),
            repo: repo.to_string(),
            home: home.to_string(),
            favorites,
            runs,
            discover: project::discover,
            now: Utc::now,
        }
    }
}

/// TICK_EVERY is how often the runs and the output are read again.
const TICK_EVERY: Duration = Duration::from_millis(250);

/// OUTPUT_LINES is how much of a run's output the pane keeps.
const OUTPUT_LINES: usize = 5000;

/// Focus says what takes the keys: a panel in the sidebar, or the pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Side,
    Pane,
}

// The panels, in the sidebar's order.
const PANEL_FAVORITES: usize = 0;
const PANEL_PROJECT: usize = 1;
const PANEL_RUNS: usize = 2;
const PANEL_LOG: usize = 3;

// The Runs panel's tabs: here, everywhere, and what agents started.
const RUNS_EVERYWHERE: usize = 1;
const RUNS_AGENTS: usize = 2;

// The Log panel's tabs: results to review, and every finished attempt.
const LOG_UNREAD: usize = 0;
const LOG_ALL: usize = 1;

/// Panel is a box in the sidebar: its name, and its views as tabs.
struct Panel {
    #[allow(dead_code)] // Go's too: the panels are told apart by their place
    name: String,
    views: Vec<View>,
    cur: usize,
}

/// Model is the interface for Bubble Tea.
pub struct Model {
    a: App,
}

struct App {
    cfg: Config,
    keys: KeyMap,
    help: help::Model,

    w: i32,
    h: i32,
    /// the sidebar's width, the pane's inside it
    sw: i32,
    pw: i32,
    /// the frame's height, the pane's inside
    fh: i32,
    ph: i32,
    /// the sidebar's height, less than the frame's when it's over the pane
    side_h: i32,
    portrait: bool,
    /// + : the pane alone, the whole window
    full: bool,
    geom: Geometry,
    panels: Vec<Panel>,
    /// the panel with the focus
    cur: usize,
    focus: Focus,
    discovery: Arc<Discovery>,
    discovery_generation: u64,
    discovery_busy: bool,
    discovery_queued: bool,
    runs: Vec<Run>,
    stopping: HashSet<String>,
    out: Option<Output>,
    /// the run the output is
    out_id: String,
    /// the first output line the pane shows
    out_top: i32,
    /// the pane keeps to the output's end as it grows
    follow: bool,
    note: String,
    note_bad: bool,
    form: Option<Form>,
    confirm: Option<Confirm>,
    keys_shown: bool,
    ticks: i64,
}

/// Confirm is a question over the pane: y does it.
struct Confirm {
    prompt: String,
    yes: Item, // the favorite of your own to take out
}

struct TickMsg(#[allow(dead_code)] SystemTime);

struct DiscoveryMsg {
    generation: u64,
    discovery: Option<Discovery>,
    manual: bool,
}

struct RunsMsg {
    list: Vec<Run>,
}

struct StartedMsg {
    run: Run,
    err: Option<String>,
}

struct StoppedMsg {
    run: Run,
    err: Option<String>,
}

impl Model {
    /// new makes the interface over a folder.
    pub fn new(cfg: Config) -> Model {
        let mut a = App {
            cfg,
            keys: new_key_map(),
            help: help::new(),
            w: 0,
            h: 0,
            sw: 0,
            pw: 0,
            fh: 0,
            ph: 0,
            side_h: 0,
            portrait: false,
            full: false,
            geom: Geometry::default(),
            panels: vec![
                Panel {
                    name: "Favorites".into(),
                    views: vec![View::new("Favorites", "none yet: m stars a command, a adds your own")],
                    cur: 0,
                },
                Panel { name: "Project".into(), views: Vec::new(), cur: 0 },
                Panel {
                    name: "Runs".into(),
                    views: vec![
                        View::new("Here", "nothing run here yet"),
                        View::new("Everywhere", "nothing running, nothing run"),
                    ],
                    cur: 0,
                },
                Panel {
                    name: "Log".into(),
                    views: vec![
                        View::new("Unread", "all caught up; ] opens history"),
                        View::new("All", "no finished runs in any folder yet"),
                    ],
                    cur: 0,
                },
            ],
            cur: 0,
            focus: Focus::Side,
            discovery: Arc::new(Discovery::default()),
            discovery_generation: 0,
            discovery_busy: false,
            discovery_queued: false,
            runs: Vec::new(),
            stopping: HashSet::new(),
            out: None,
            out_id: String::new(),
            out_top: 0,
            follow: true,
            note: String::new(),
            note_bad: false,
            form: None,
            confirm: None,
            keys_shown: false,
            ticks: 0,
        };
        a.set_discovery((a.cfg.discover)(&a.cfg.dir, &a.cfg.repo));
        if let Ok(list) = a.cfg.runs.list() {
            a.set_runs(list);
        }
        // Open on the folder's favorites when it has some, the cursor on one
        // that's running, so its output shows; else on what's going here;
        // else on the project's commands.
        let favs = &a.panels[PANEL_FAVORITES].views[0];
        if !favs.all.is_empty() {
            a.cur = PANEL_FAVORITES;
            let running = favs.shown.iter().position(|it| a.last_run(&it.dir, &it.command).is_some_and(|r| r.live()));
            if let Some(i) = running {
                a.panels[PANEL_FAVORITES].views[0].at = i as i32;
            }
        } else if a.runs.iter().any(|r| a.here(r) && r.live()) {
            a.cur = PANEL_RUNS;
        } else if !a.discovery.files.is_empty() {
            a.cur = PANEL_PROJECT;
        }
        a.sync_output();
        Model { a }
    }

    /// live are the runs still going in the project, in its folder or one
    /// under it, for what runui says when it quits.
    pub fn live(&self) -> Vec<Run> {
        self.a.runs.iter().filter(|r| self.a.here(r) && r.live()).cloned().collect()
    }
}

impl tea::Model for Model {
    fn init(&mut self) -> Cmd {
        self.a.tick()
    }

    fn update(&mut self, msg: Msg) -> Cmd {
        self.a.update(msg)
    }

    /// view draws the interface full screen.
    fn view(&mut self) -> tea::View {
        let mut v = tea::new_view(self.a.draw());
        v.alt_screen = true;
        v.window_title = format!("runui · {}", filepath::base(&self.a.cfg.dir));
        v.mouse_mode = MouseMode::CellMotion;
        v
    }
}

impl App {
    fn tick(&self) -> Cmd {
        tea::tick(TICK_EVERY, |t| tea::msg(TickMsg(t)))
    }

    /// list_runs reads the runs again, away from the interface.
    fn list_runs(&self) -> Cmd {
        let runner = self.cfg.runs.clone();
        tea::cmd(move || runner.list().ok().map(|list| tea::msg(RunsMsg { list })))
    }

    fn update(&mut self, msg: Msg) -> Cmd {
        if let Some(s) = msg.downcast_ref::<WindowSizeMsg>() {
            (self.w, self.h) = (s.width, s.height);
            self.layout();
            return None;
        }
        if msg.is::<TickMsg>() {
            self.ticks += 1;
            let mut cmds = vec![self.tick()];
            if self.ticks % 2 == 0 {
                cmds.push(self.list_runs());
            }
            if self.ticks % 8 == 0 {
                cmds.push(self.refresh_files(false));
            }
            self.read_output();
            return tea::batch(cmds);
        }
        let msg = match msg.downcast::<DiscoveryMsg>() {
            Ok(msg) => {
                if msg.generation != self.discovery_generation {
                    return None;
                }
                self.discovery_busy = false;
                if self.discovery_queued {
                    self.discovery_queued = false;
                    return self.refresh_files(true);
                }
                if let Some(discovery) = msg.discovery {
                    self.set_discovery(discovery);
                }
                if msg.manual {
                    self.say("read the project's files again", false);
                }
                return None;
            }
            Err(msg) => msg,
        };
        let msg = match msg.downcast::<RunsMsg>() {
            Ok(m) => {
                self.set_runs(m.list);
                return None;
            }
            Err(msg) => msg,
        };
        let msg = match msg.downcast::<StartedMsg>() {
            Ok(m) => {
                if let Some(err) = m.err {
                    self.say(&err, true);
                    return self.list_runs();
                }
                self.say(&format!("started {}", m.run.command), false);
                let from_log = self.cur == PANEL_LOG;
                let key = format!("run {}", m.run.id);
                let tab = if self.here(&m.run) { 0 } else { RUNS_EVERYWHERE };
                let mut list = vec![m.run];
                list.extend(self.runs.iter().cloned());
                self.set_runs(list);
                if from_log {
                    self.panels[PANEL_RUNS].cur = tab;
                    self.jump(PANEL_RUNS as i32);
                    self.shown_mut().pick(&key);
                    self.sync_output();
                }
                return self.list_runs();
            }
            Err(msg) => msg,
        };
        let msg = match msg.downcast::<StoppedMsg>() {
            Ok(m) => {
                if let Some(err) = m.err {
                    self.stopping.remove(&m.run.id);
                    self.say(&err, true);
                }
                return self.list_runs();
            }
            Err(msg) => msg,
        };
        if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            return self.key(k);
        }
        if let Some(m) = msg.downcast_ref::<MouseClickMsg>() {
            return self.click(m.mouse());
        }
        if let Some(m) = msg.downcast_ref::<MouseWheelMsg>() {
            self.wheel(m.mouse());
            return None;
        }
        if msg.is::<PasteMsg>()
            && let Some(f) = self.form.as_mut()
        {
            return f.update(&msg);
        }
        None
    }

    fn say(&mut self, note: &str, bad: bool) {
        (self.note, self.note_bad) = (note.to_string(), bad);
    }

    /// refresh_files checks and rereads dependencies away from the interface.
    /// Manual refreshes are coalesced while a scan is in flight.
    fn refresh_files(&mut self, manual: bool) -> Cmd {
        if self.discovery_busy {
            self.discovery_queued |= manual;
            return None;
        }
        self.discovery_busy = true;
        self.discovery_generation += 1;
        let generation = self.discovery_generation;
        let previous = self.discovery.clone();
        let (dir, repo) = (self.cfg.dir.clone(), self.cfg.repo.clone());
        let discover = self.cfg.discover;
        tea::cmd(move || {
            let discovery = if manual || previous.changed() { Some(discover(&dir, &repo)) } else { None };
            Some(tea::msg(DiscoveryMsg { generation, discovery, manual }))
        })
    }

    /// set_discovery keeps the source tab, filter and cursor when results change.
    fn set_discovery(&mut self, discovery: Discovery) {
        self.discovery = Arc::new(discovery);
        let p = &mut self.panels[PANEL_PROJECT];
        let showing = p.views.get(p.cur).map(|v| v.name.clone()).unwrap_or_default();
        let mut old: HashMap<String, View> = p.views.drain(..).map(|v| (v.name.clone(), v)).collect();
        p.cur = 0;
        for source in project::SOURCES {
            let mut items = Vec::new();
            let mut problems: Vec<_> = self
                .discovery
                .diagnostics
                .iter()
                .filter(|problem| problem.source.is_none_or(|kind| kind == source))
                .map(|problem| format!("discovery incomplete: {}", problem.description(&self.cfg.dir)))
                .collect();
            let mut found = false;
            for f in self.discovery.files.iter().filter(|f| f.source == source) {
                found = true;
                if let Some(err) = &f.err {
                    problems.push(format!("{}: {err}", f.rel_path()));
                }
                for c in &f.commands {
                    items.push(Item {
                        kind: Kind::Command,
                        name: c.name.clone(),
                        command: c.run.clone(),
                        desc: c.desc.clone(),
                        source: c.source.as_str().to_string(),
                        dir: f.dir.clone(),
                        rel: f.rel.clone(),
                        place: f.label.clone(),
                        file: f.rel_path(),
                        ..Default::default()
                    });
                }
            }
            if !found {
                continue;
            }
            let mut v = old.remove(source.label()).unwrap_or_else(|| View::new(source.label(), ""));
            (v.empty, v.note) = ("nothing in it to run".into(), String::new());
            if let Some(first) = problems.first() {
                v.note = first.clone();
                if problems.len() > 1 {
                    v.note += &format!(" (and {} more)", problems.len() - 1);
                }
            }
            v.set(items);
            if v.name == showing {
                p.cur = p.views.len();
            }
            p.views.push(v);
        }
        if p.views.is_empty() {
            let mut view = View::new("Project", "no Taskfile, package.json, Makefile, compose file or pom.xml here");
            if let Some(problem) = self.discovery.diagnostics.first() {
                view.note = format!("discovery incomplete: {}", problem.description(&self.cfg.dir));
            }
            p.views = vec![view];
        }
        self.load_favorites(); // their descriptions are the project's
    }

    /// load_favorites reads the folder's favorites from the store.
    fn load_favorites(&mut self) {
        let mut items = Vec::new();
        for f in self.cfg.favorites.for_dir(&self.cfg.dir) {
            let mut rel = filepath::clean(&format!("./{}", f.dir));
            if rel == "." {
                rel = String::new();
            }
            let mut it = Item {
                kind: Kind::Favorite,
                name: f.label().to_string(),
                command: f.command.clone(),
                source: f.source.clone(),
                dir: filepath::join(&[&self.cfg.dir, &rel]),
                rel: rel.clone(),
                place: rel,
                ..Default::default()
            };
            if let Some((c, file)) = self.project_command(&it.dir, &f.command) {
                (it.desc, it.place, it.file) = (c.desc.clone(), file.label.clone(), file.rel_path());
            }
            items.push(it);
        }
        self.panels[PANEL_FAVORITES].views[0].set(items);
        self.sync_output();
    }

    /// project_command is the project's command with a command line in a
    /// folder, and the file it's in.
    fn project_command(&self, dir: &str, command: &str) -> Option<(&project::Command, &File)> {
        self.discovery
            .files
            .iter()
            .filter(|f| f.dir == dir)
            .find_map(|f| f.commands.iter().find(|c| c.run == command).map(|c| (c, f)))
    }

    /// rel_of is a folder from the project's, with slashes: "" for the
    /// project's own, and None for one that isn't in it.
    fn rel_of(&self, dir: &str) -> Option<String> {
        let rel = filepath::rel(&self.cfg.dir, dir)?;
        if rel == ".." || rel.starts_with("../") {
            return None;
        }
        if rel == "." {
            return Some(String::new());
        }
        Some(rel)
    }

    /// here says a run is in the project: its folder, or one under it.
    fn here(&self, r: &Run) -> bool {
        self.rel_of(&r.dir).is_some()
    }

    /// set_runs takes the runs as listed: here, the folder's, everywhere,
    /// every one, and the agents', the ones going first.
    fn set_runs(&mut self, list: Vec<Run>) {
        let mut seen = HashSet::new();
        self.runs = list.into_iter().filter(|r| seen.insert(r.id.clone())).collect();
        self.runs.sort_by(|x, y| y.live().cmp(&x.live()).then(y.started.cmp(&x.started)));
        let stopping: Vec<String> = self.stopping.iter().cloned().collect();
        for id in stopping {
            if !self.runs.iter().any(|r| r.id == id && r.live()) {
                self.stopping.remove(&id);
                if let Some(r) = self.run_of(&id)
                    && self.note.starts_with("stopping ")
                {
                    let note = format!("stopped {}", r.command);
                    self.say(&note, false);
                }
            }
        }
        let (mut here, mut all, mut agents) = (Vec::new(), Vec::new(), Vec::new());
        for r in &self.runs {
            let mut it = Item {
                kind: Kind::Run,
                name: r.label().to_string(),
                command: r.command.clone(),
                dir: r.dir.clone(),
                run: r.clone(),
                ..Default::default()
            };
            if let Some(rel) = self.rel_of(&r.dir) {
                (it.rel, it.place) = (rel.clone(), rel);
                here.push(it.clone());
            }
            if !r.agent.is_empty() {
                agents.push(it.clone());
            }
            all.push(it);
        }
        self.panels[PANEL_RUNS].views[0].set(here);
        self.panels[PANEL_RUNS].views[RUNS_EVERYWHERE].set(all);
        self.set_agents(agents);
        self.set_log();
        self.sync_output();
    }

    /// set_agents gives the Agents tab its rows: the runs agents started
    /// through runui mcp, from every folder. The tab is there only while
    /// there are some, so runui looks as the Go build does to anyone who
    /// doesn't use one.
    fn set_agents(&mut self, agents: Vec<Item>) {
        let p = &mut self.panels[PANEL_RUNS];
        if agents.is_empty() {
            if p.views.len() > RUNS_AGENTS {
                p.views.truncate(RUNS_AGENTS);
                p.cur = p.cur.min(RUNS_AGENTS - 1);
            }
            return;
        }
        if p.views.len() == RUNS_AGENTS {
            p.views.push(View::new("Agents", "nothing an agent started"));
        }
        p.views[RUNS_AGENTS].set(agents);
    }

    /// set_log shares finished history with an inbox of unread results.
    /// Keep the viewed inbox row while reading, so a refresh cannot select
    /// and acknowledge the next result on its own.
    fn set_log(&mut self) {
        let viewed = if self.cur == PANEL_LOG && self.panels[PANEL_LOG].cur == LOG_UNREAD {
            self.selected().map(|it| it.run.id)
        } else {
            None
        };
        let mut all: Vec<_> =
            self.panels[PANEL_RUNS].views[RUNS_EVERYWHERE].all.iter().filter(|it| !it.run.live()).cloned().collect();
        all.sort_by_key(|it| std::cmp::Reverse(it.run.ended.unwrap_or(it.run.started)));
        let unread = all
            .iter()
            .filter(|it| self.cfg.runs.pending_result(&it.run) || viewed.as_deref() == Some(&it.run.id))
            .cloned()
            .collect();
        self.panels[PANEL_LOG].views[LOG_UNREAD].set(unread);
        self.panels[PANEL_LOG].views[LOG_ALL].set(all);
    }

    fn pending_results(&self) -> usize {
        self.runs.iter().filter(|r| self.cfg.runs.pending_result(r)).count()
    }

    /// shown is the view showing in the panel with the focus.
    fn shown(&self) -> &View {
        let p = &self.panels[self.cur];
        &p.views[p.cur]
    }

    fn shown_mut(&mut self) -> &mut View {
        let p = &mut self.panels[self.cur];
        &mut p.views[p.cur]
    }

    /// selected is the row under the cursor.
    fn selected(&self) -> Option<Item> {
        self.shown().current().cloned()
    }

    /// run_of is the run as last listed.
    fn run_of(&self, id: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    /// last_run is a command's latest run in a folder: the one going, else
    /// the last to end.
    fn last_run(&self, dir: &str, command: &str) -> Option<&Run> {
        // the ones going first, then the newest
        self.runs.iter().find(|r| r.dir == dir && r.command == command)
    }

    /// pane_run is the run the pane shows: the row's, or the latest run of
    /// the row's command.
    fn pane_run(&self) -> Option<Run> {
        let it = self.selected()?;
        if it.kind == Kind::Run {
            return self.run_of(&it.run.id).cloned();
        }
        self.last_run(&it.dir, &it.command).cloned()
    }

    /// A finished result is viewed when its output is drawn, even while
    /// the sidebar has the focus. Live runs stay in the status counts.
    fn acknowledge_viewed_run(&self) {
        if let Some(r) = self.pane_run()
            && matches!(r.state(), runs::State::Exited | runs::State::Lost)
        {
            let _ = self.cfg.runs.acknowledge(&r);
        }
    }

    /// sync_output reads the output of the run the pane shows, when that's
    /// another run than before.
    fn sync_output(&mut self) {
        let Some(r) = self.pane_run() else {
            (self.out, self.out_id) = (None, String::new());
            return;
        };
        if r.id == self.out_id {
            return;
        }
        (self.out, self.out_id, self.out_top, self.follow) =
            (Some(Output::new(&r.output(), OUTPUT_LINES)), r.id.clone(), 0, true);
        self.read_output();
    }

    /// read_output reads what the run printed since, keeping to its end
    /// while following.
    fn read_output(&mut self) {
        let Some(out) = self.out.as_mut() else { return };
        if out.read().is_err() {
            return;
        }
        if self.follow {
            let n = out.len() as i32;
            self.out_top = (n - self.out_height()).max(0);
        }
    }

    // Keys

    fn key(&mut self, msg: &KeyPressMsg) -> Cmd {
        let s = msg.string();
        if s == "ctrl+c" {
            return tea::quit();
        }
        if self.form.is_some() {
            return self.form_key(msg);
        }
        if self.confirm.is_some() {
            return self.confirm_key(msg);
        }
        if self.keys_shown {
            self.keys_shown = false;
            return None;
        }
        if self.shown().typing {
            self.filter_key(msg);
            return None;
        }
        self.note.clear();
        let k = &self.keys;
        if key::matches(msg, &[&k.quit]) {
            return tea::quit();
        }
        if key::matches(msg, &[&k.help]) {
            self.keys_shown = true;
            return None;
        }
        if key::matches(msg, &[&k.full]) {
            self.full = s == "+";
            self.layout();
            return None;
        }
        if key::matches(msg, &[&k.panels]) {
            self.jump(i32::from(s.as_bytes()[0]) - i32::from(b'1'));
            return None;
        }
        if self.focus == Focus::Pane {
            return self.pane_key(msg);
        }
        self.side_key(msg)
    }

    /// side_key is a key while a panel has the focus.
    fn side_key(&mut self, msg: &KeyPressMsg) -> Cmd {
        let k = self.keys.clone();
        let s = msg.string();
        let m = |b: &key::Binding| key::matches(msg, &[b]);
        if m(&k.next_panel) {
            self.jump(((self.cur + 1) % self.panels.len()) as i32);
        } else if m(&k.prev_panel) {
            self.jump(((self.cur + self.panels.len() - 1) % self.panels.len()) as i32);
        } else if m(&k.tabs) {
            let p = &mut self.panels[self.cur];
            let step = if s == "[" { p.views.len() - 1 } else { 1 };
            p.cur = (p.cur + step) % p.views.len();
            if self.cur == PANEL_LOG {
                self.set_log();
            }
            self.sync_output();
        } else if m(&k.focus_pane) {
            self.focus = Focus::Pane;
        } else if m(&k.up) {
            self.shown_mut().move_by(-1);
            self.sync_output();
        } else if m(&k.down) {
            self.shown_mut().move_by(1);
            self.sync_output();
        } else if m(&k.top) {
            let n = self.shown().shown.len() as i32;
            self.shown_mut().move_by(-n);
            self.sync_output();
        } else if m(&k.bottom) {
            let n = self.shown().shown.len() as i32;
            self.shown_mut().move_by(n);
            self.sync_output();
        } else if m(&k.scroll_pane) {
            self.scroll(&s);
        } else if m(&k.filter) {
            self.shown_mut().typing = true;
        } else if m(&k.back) {
            if !self.shown().filter.is_empty() {
                self.shown_mut().set_filter("");
                self.sync_output();
            }
        } else {
            return self.action(msg);
        }
        None
    }

    /// pane_key is a key while the pane has the focus: moving scrolls the
    /// output, esc goes back to the panel.
    fn pane_key(&mut self, msg: &KeyPressMsg) -> Cmd {
        let k = self.keys.clone();
        let m = |b: &key::Binding| key::matches(msg, &[b]);
        if m(&k.back) || m(&k.prev_panel) || m(&k.next_panel) || m(&k.focus_pane) {
            self.focus = Focus::Side;
        } else if m(&k.up) {
            self.scroll("up");
        } else if m(&k.down) {
            self.scroll("down");
        } else if m(&k.top) {
            self.scroll("home");
        } else if m(&k.bottom) {
            self.scroll("end");
        } else if m(&k.scroll_pane) {
            self.scroll(&msg.string());
        } else {
            return self.action(msg);
        }
        None
    }

    /// scroll moves the output: a line, half a page, a page, or to an end;
    /// at the end it follows what's printed again.
    fn scroll(&mut self, how: &str) {
        let Some(out) = &self.out else { return };
        let h = self.out_height();
        let last = (out.len() as i32 - h).max(0);
        match how {
            "up" | "K" => self.out_top -= 1,
            "down" | "J" => self.out_top += 1,
            "ctrl+u" => self.out_top -= h / 2,
            "ctrl+d" => self.out_top += h / 2,
            "pgup" => self.out_top -= h,
            "pgdown" => self.out_top += h,
            "home" => self.out_top = 0,
            "end" => self.out_top = last,
            _ => {}
        }
        self.out_top = self.out_top.min(last).max(0);
        self.follow = self.out_top == last;
    }

    /// jump gives a panel the focus.
    fn jump(&mut self, i: i32) {
        if i < 0 || i as usize >= self.panels.len() {
            return;
        }
        let leaving_log = self.cur == PANEL_LOG && i as usize != PANEL_LOG;
        (self.cur, self.focus) = (i as usize, Focus::Side);
        if leaving_log {
            self.set_log();
        }
        self.sync_output();
    }

    /// filter_key is a key while a filter is being typed.
    fn filter_key(&mut self, msg: &KeyPressMsg) {
        let v = self.shown_mut();
        match msg.string().as_str() {
            "esc" => {
                v.typing = false;
                v.set_filter("");
            }
            "enter" | "down" | "up" => v.typing = false,
            "backspace" => {
                let mut f = v.filter.clone();
                if f.pop().is_some() {
                    v.set_filter(&f);
                }
            }
            _ => {
                if !msg.text.is_empty() {
                    let f = format!("{}{}", v.filter, msg.text);
                    v.set_filter(&f);
                }
            }
        }
        self.sync_output();
    }

    /// action is a key that does something to the row under the cursor.
    fn action(&mut self, msg: &KeyPressMsg) -> Cmd {
        let k = self.keys.clone();
        let m = |b: &key::Binding| key::matches(msg, &[b]);
        if m(&k.run) {
            return self.run();
        } else if m(&k.stop) {
            return self.stop();
        } else if m(&k.restart) {
            return self.restart();
        } else if m(&k.star) {
            return self.star();
        } else if m(&k.add) {
            return self.open_form(None);
        } else if m(&k.edit) {
            if let Some(it) = self.selected()
                && it.kind == Kind::Favorite
            {
                return self.open_form(Some(&it));
            }
            self.say("e edits a favorite", true);
        } else if m(&k.remove) {
            return self.remove();
        } else if m(&k.move_up) || m(&k.move_down) {
            self.move_favorite(&msg.string());
        } else if m(&k.yank) {
            if let Some(it) = self.selected() {
                self.say(&format!("copied: {}", it.command), false);
                return tea::set_clipboard(it.command);
            }
        } else if m(&k.reload) {
            if let Err(err) = self.cfg.favorites.reload() {
                self.say(&err.to_string(), true);
            }
            return tea::batch(vec![self.refresh_files(true), self.list_runs()]);
        }
        None
    }

    /// run is enter: a command runs, unless it's running already; on a
    /// run, the pane takes the keys, to scroll its output.
    fn run(&mut self) -> Cmd {
        let it = self.selected()?;
        if it.kind == Kind::Run {
            self.focus = Focus::Pane;
            return None;
        }
        if self.last_run(&it.dir, &it.command).is_some_and(|r| r.live()) {
            self.say(&format!("{} is running already: s stops it, R runs it again", it.command), false);
            return None;
        }
        self.start(&it.dir, &it.name, &it.command)
    }

    fn start(&mut self, dir: &str, name: &str, command: &str) -> Cmd {
        let runner = self.cfg.runs.clone();
        self.say(&format!("starting {command}…"), false);
        let (dir, name, command) = (dir.to_string(), name.to_string(), command.to_string());
        tea::cmd_msg(move || {
            let (run, err) = runner.start(&dir, &name, &command);
            StartedMsg { run, err: err.map(|e| e.to_string()) }
        })
    }

    /// stop stops the run the pane shows.
    fn stop(&mut self) -> Cmd {
        let Some(r) = self.pane_run().filter(Run::live) else {
            self.say("nothing running to stop here", true);
            return None;
        };
        self.stopping.insert(r.id.clone());
        self.say(&format!("stopping {}…", r.command), false);
        let runner = self.cfg.runs.clone();
        tea::cmd_msg(move || {
            let err = runner.stop(&r).err().map(|e| e.to_string());
            StoppedMsg { run: r, err }
        })
    }

    /// restart runs the row's command again, stopping it first when it's
    /// running.
    fn restart(&mut self) -> Cmd {
        let mut it = self.selected()?;
        let mut r = self.last_run(&it.dir, &it.command).cloned();
        if it.kind == Kind::Run {
            r = self.run_of(&it.run.id).cloned();
            it.name = r.as_ref().map(|r| r.name.clone()).unwrap_or_default();
        }
        let Some(r) = r.filter(Run::live) else {
            return self.start(&it.dir, &it.name, &it.command);
        };
        self.stopping.insert(r.id.clone());
        self.say(&format!("stopping {} to run it again…", r.command), false);
        let (runner, wait) = (self.cfg.runs.clone(), runs::stop_grace() + Duration::from_secs(5));
        tea::cmd_msg(move || {
            if let Err(err) = runner.stop(&r) {
                return StartedMsg { run: Run::default(), err: Some(err.to_string()) };
            }
            let deadline = Instant::now() + wait;
            loop {
                let list = match runner.list() {
                    Ok(list) => list,
                    Err(err) => return StartedMsg { run: Run::default(), err: Some(err.to_string()) },
                };
                if !list.iter().any(|x| x.id == r.id && x.live()) {
                    break;
                }
                if Instant::now() > deadline {
                    return StartedMsg { run: Run::default(), err: Some(format!("{} didn't stop", r.command)) };
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let (run, err) = runner.start(&it.dir, &it.name, &it.command);
            StartedMsg { run, err: err.map(|e| e.to_string()) }
        })
    }

    /// star keeps the row's command in its folder's favorites, or takes it
    /// out again.
    fn star(&mut self) -> Cmd {
        let it = self.selected()?;
        if it.kind == Kind::Favorite {
            return self.unfavorite(it);
        }
        let (mut f, mut store) = (it.favorite(), self.cfg.dir.clone());
        if it.kind == Kind::Run {
            if let Some((c, _)) = self.project_command(&it.dir, &it.command) {
                f.source = c.source.as_str().to_string();
            }
            if self.rel_of(&it.dir).is_none() {
                (store, f.dir) = (it.dir.clone(), String::new()); // another project's run: a favorite of its own folder
            }
        }
        match self.cfg.favorites.toggle(&store, f) {
            Err(err) => self.say(&err.to_string(), true),
            Ok(true) => self.say(&format!("★ {} is a favorite here", it.name), false),
            Ok(false) => self.say(&format!("{} is no longer a favorite", it.name), false),
        }
        self.load_favorites();
        None
    }

    /// unfavorite takes a favorite out, asking first when it's a command of
    /// your own, which nothing else lists.
    fn unfavorite(&mut self, it: Item) -> Cmd {
        if !it.source.is_empty() {
            return self.gone(&it);
        }
        let prompt = format!(
            "Remove {} from the favorites?\n\n{}\n\nIt's a command of your own: a adds it again.",
            S_TITLE.render(&it.name),
            S_ACCENT.render(&format!("$ {}", it.command))
        );
        self.confirm = Some(Confirm { prompt, yes: it });
        None
    }

    // gone takes a favorite out of the folder's favorites.
    fn gone(&mut self, it: &Item) -> Cmd {
        let dir = self.cfg.dir.clone();
        match self.cfg.favorites.remove(&dir, &it.favorite()) {
            Err(err) => self.say(&err.to_string(), true),
            Ok(()) => self.say(&format!("{} is no longer a favorite", it.name), false),
        }
        self.load_favorites();
        None
    }

    /// remove is D: a favorite out of the favorites, or a run that has
    /// ended and its output.
    fn remove(&mut self) -> Cmd {
        let it = self.selected()?;
        match it.kind {
            Kind::Favorite => self.unfavorite(it),
            Kind::Run => {
                let r = self.run_of(&it.run.id).cloned().unwrap_or_default();
                if r.live() {
                    self.say(&format!("{} is running: s stops it first", it.name), true);
                    return None;
                }
                if let Err(err) = self.cfg.runs.remove(&r) {
                    self.say(&err.to_string(), true);
                    return None;
                }
                self.say(&format!("forgot {} and its output", it.name), false);
                let list = self.runs.iter().filter(|x| x.id != r.id).cloned().collect();
                self.set_runs(list);
                self.list_runs()
            }
            Kind::Command => {
                self.say("D removes a favorite, or a run that has ended", true);
                None
            }
        }
    }

    /// move_favorite puts a favorite up or down the list (Go's `move`).
    fn move_favorite(&mut self, how: &str) {
        let Some(it) = self.selected().filter(|it| it.kind == Kind::Favorite) else {
            self.say("< and > move a favorite", true);
            return;
        };
        let delta = if how == "<" { -1 } else { 1 };
        let dir = self.cfg.dir.clone();
        if let Err(err) = self.cfg.favorites.move_by(&dir, &it.favorite(), delta) {
            self.say(&err.to_string(), true);
        }
        self.load_favorites();
    }

    fn confirm_key(&mut self, msg: &KeyPressMsg) -> Cmd {
        if key::matches(msg, &[&self.keys.yes]) {
            let c = self.confirm.take()?;
            return self.gone(&c.yes);
        }
        if key::matches(msg, &[&self.keys.no]) {
            self.confirm = None;
        }
        None
    }

    // The form: a command of your own, added or edited

    /// open_form adds a command of your own, or edits a favorite.
    fn open_form(&mut self, it: Option<&Item>) -> Cmd {
        let width = (self.pw - 14).max(20);
        let input = |placeholder: &str| {
            let mut t = textinput::new();
            t.placeholder = placeholder.to_string();
            t.prompt = String::new();
            t.set_width(width);
            let mut st = t.styles();
            st.cursor.blink = false; // a steady cursor: nothing to tick while the form is open
            t.set_styles(st);
            t
        };
        let mut f = Form {
            old: None,
            name: input("what it's called; the command when empty"),
            command: input("docker compose up, npm run start -- --port 4000, …"),
            at: 0,
            err: String::new(),
        };
        if let Some(it) = it {
            f.old = Some(it.favorite());
            f.command.set_value(&it.command);
            if it.name != it.command {
                f.name.set_value(&it.name);
            }
        }
        let first = if it.is_none() { 1 } else { 0 };
        self.focus = Focus::Side;
        let cmd = f.focus_on(first);
        self.form = Some(f);
        cmd
    }

    fn form_key(&mut self, msg: &KeyPressMsg) -> Cmd {
        let f = self.form.as_mut()?;
        match msg.string().as_str() {
            "esc" => {
                self.form = None;
                return None;
            }
            "tab" | "shift+tab" | "up" | "down" => {
                let other = 1 - f.at;
                return f.focus_on(other);
            }
            "enter" => {
                let mut fav = Favorite { name: f.name.value(), command: f.command.value(), ..Default::default() };
                let dir = self.cfg.dir.clone();
                let res = match &f.old {
                    None => self.cfg.favorites.add(&dir, fav.clone()),
                    Some(old) => {
                        fav.dir = old.dir.clone();
                        let old = old.clone();
                        self.cfg.favorites.update(&dir, &old, fav.clone())
                    }
                };
                if let Err(err) = res {
                    if let Some(f) = self.form.as_mut() {
                        f.err = err.to_string();
                    }
                    return None;
                }
                self.form = None;
                self.load_favorites();
                (self.cur, self.panels[PANEL_FAVORITES].cur) = (PANEL_FAVORITES, 0);
                let v = &mut self.panels[PANEL_FAVORITES].views[0];
                v.set_filter("");
                let key = Item {
                    kind: Kind::Favorite,
                    command: fav.command.trim().to_string(),
                    rel: fav.dir.clone(),
                    ..Default::default()
                }
                .key();
                v.pick(&key);
                self.sync_output();
                self.say(&format!("★ {} is a favorite here: enter runs it", fav.command), false);
                return None;
            }
            _ => {}
        }
        f.err.clear();
        f.update(&tea::msg(msg.clone()))
    }

    // Mouse

    /// click gives what's clicked the focus: a panel's row, or the pane.
    fn click(&mut self, m: Mouse) -> Cmd {
        if m.button != MouseButton::Left || self.form.is_some() || self.confirm.is_some() {
            return None;
        }
        self.keys_shown = false;
        if self.geom.pane.has(m.x, m.y) {
            self.focus = Focus::Pane;
            return None;
        }
        let areas = self.geom.panels.clone();
        for (i, r) in areas.iter().enumerate() {
            if !r.has(m.x, m.y) {
                continue;
            }
            self.jump(i as i32);
            let v = self.shown_mut();
            let row = m.y - r.y - 1 - v.filter_lines();
            if row >= 0 && ((v.top + row) as usize) < v.shown.len() {
                v.at = v.top + row;
                self.sync_output();
            }
        }
        None
    }

    /// wheel scrolls the output, or moves along a panel's rows.
    fn wheel(&mut self, m: Mouse) {
        let up = m.button == MouseButton::WheelUp;
        if self.geom.pane.has(m.x, m.y) {
            for _ in 0..3 {
                self.scroll(if up { "up" } else { "down" });
            }
            return;
        }
        let areas = self.geom.panels.clone();
        for (i, r) in areas.iter().enumerate() {
            if r.has(m.x, m.y) {
                self.jump(i as i32);
                self.shown_mut().move_by(if up { -1 } else { 1 });
                self.sync_output();
            }
        }
    }
}

impl Item {
    /// favorite is the favorite a row is, or would be: its command, in its
    /// folder from the project's.
    fn favorite(&self) -> Favorite {
        Favorite {
            name: self.name.clone(),
            command: self.command.clone(),
            source: self.source.clone(),
            dir: self.rel.clone(),
            added: None,
        }
    }
}

struct Form {
    /// the favorite being edited; None when adding
    old: Option<Favorite>,
    name: textinput::Model,
    command: textinput::Model,
    at: usize,
    err: String,
}

impl Form {
    fn inputs(&mut self) -> [&mut textinput::Model; 2] {
        [&mut self.name, &mut self.command]
    }

    fn focus_on(&mut self, i: usize) -> Cmd {
        self.at = i;
        let mut cmd = None;
        for (j, input) in self.inputs().into_iter().enumerate() {
            if j == i {
                cmd = input.focus();
            } else {
                input.blur();
            }
        }
        cmd
    }

    fn update(&mut self, msg: &Msg) -> Cmd {
        let at = self.at;
        self.inputs().into_iter().nth(at).and_then(|input| input.update(msg))
    }
}

/// Area is where something was drawn.
#[derive(Clone, Copy, Debug, Default)]
struct Area {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Area {
    fn has(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Geometry is where the panels and the pane were drawn last, for the
/// mouse.
#[derive(Clone, Debug, Default)]
struct Geometry {
    panels: Vec<Area>,
    pane: Area,
}

#[cfg(test)]
mod tests;
