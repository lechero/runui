//! model_test.go and rows_test.go, ported.

use std::fs;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use runui_favorites::{Favorite, Store as Favorites};
use runui_runs::{self as runs, Store};
use runui_tea::ansi;
use runui_tea::tea::{self, BatchMsg, Cmd, KeyPressMsg, Msg, WindowSizeMsg};

use super::*;

/// SUPERVISE_ENV makes the test binary a run's supervisor, as runui
/// _supervise makes runui one: Go's TestMain is supervise_helper here.
const SUPERVISE_ENV: &str = "RUNUI_TUI_TEST_SUPERVISE";

#[test]
fn cargo_aliases_show_provenance_and_global_scope_is_explicit() {
    let world = new_world();
    fs::create_dir_all(filepath::join(&[&world.dir, ".cargo"])).unwrap();
    fs::create_dir_all(filepath::join(&[&world.home, ".cargo"])).unwrap();
    fs::write(filepath::join(&[&world.dir, ".cargo/config.toml"]), "[alias]\nlocal-alias='version'\n").unwrap();
    fs::write(filepath::join(&[&world.home, ".cargo/config.toml"]), "[alias]\nglobal-alias='version'\n").unwrap();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("source:cargo kind:alias");
    assert!(h.screen().contains("local-alias"));
    assert!(!h.screen().contains("global-alias"));
    assert!(h.screen().contains("Cargo alias · project context"));
    h.keys(&["alt+g"]);
    assert!(h.screen().contains("global-alias"));
    assert!(h.screen().contains("Cargo alias · global context"));
    h.keys(&["ctrl+v"]);
    assert!(h.screen().contains("Cargo owns expansion"));
    h.keys(&["alt+g"]);
    assert!(!h.screen().contains("global-alias"));
    assert!(world.runs.list().unwrap().is_empty());
}

#[test]
fn invalid_cargo_aliases_explain_the_failure_without_creating_a_run() {
    let world = new_world();
    fs::create_dir_all(filepath::join(&[&world.dir, ".cargo"])).unwrap();
    fs::write(filepath::join(&[&world.dir, ".cargo/config.toml"]), "[alias]\nloop='loop'\n").unwrap();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("source:cargo kind:alias loop");
    assert!(h.screen().contains("recursion contains a cycle"));
    h.keys(&["enter"]);
    assert!(h.screen().contains("recursion contains a cycle"));
    assert!(world.runs.list().unwrap().is_empty());
}

#[test]
fn supervise_helper() {
    if std::env::var(SUPERVISE_ENV).as_deref() != Ok("1") {
        return;
    }
    runs::set_stop_grace(Duration::from_millis(300));
    let folder = std::env::args().next_back().unwrap_or_default();
    std::process::exit(if runs::supervise(&folder).is_ok() { 0 } else { 1 });
}

/// World is a project folder, its favorites and its runs, all in a test's
/// own folders.
struct World {
    dir: String,
    favs: String, // the favorites file
    runs: Arc<Store>,
    home: String,
    _root: tempfile::TempDir,
}

fn new_world() -> World {
    let root = tempfile::tempdir().unwrap();
    let r = root.path().to_str().unwrap().to_string();
    let dir = filepath::join(&[&r, "projects", "app"]);
    fs::create_dir_all(&dir).unwrap();
    for (name, body) in [
        ("package.json", r#"{"scripts": {"dev": "vite", "lint": "eslint .", "test": "vitest run"}}"#),
        ("Makefile", "build: ## Build it\n\tgo build\n"),
    ] {
        fs::write(filepath::join(&[&dir, name]), body).unwrap();
    }
    let exe = std::env::current_exe().unwrap().to_str().unwrap().to_string();
    let store = Store {
        root: filepath::join(&[&r, "state", "runs"]),
        exe: vec![exe, "tests::supervise_helper".into(), "--exact".into(), "--test-threads=1".into(), "--quiet".into()],
        shell: vec!["/bin/sh".into(), "-c".into()],
        env: vec![format!("{SUPERVISE_ENV}=1")],
    };
    World { dir, favs: filepath::join(&[&r, "config", "favorites.json"]), runs: Arc::new(store), home: r, _root: root }
}

impl World {
    /// favs_now is the favorites file as it is now.
    fn favs_now(&self) -> Favorites {
        Favorites::open(&self.favs).unwrap()
    }

    fn add(&self, f: Favorite) {
        self.favs_now().add(&self.dir, f).unwrap();
    }
}

impl Drop for World {
    /// what a test left running is stopped, and its supervisors are waited
    /// for to write down how it ended, before the test's folders go.
    fn drop(&mut self) {
        let list = self.runs.list().unwrap_or_default();
        for r in list.iter().filter(|r| r.live()) {
            let _ = self.runs.stop(r);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !self.runs.list().unwrap_or_default().iter().any(Run::live) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn fav(command: &str) -> Favorite {
    Favorite { command: command.into(), ..Default::default() }
}

/// Harness drives the model as Bubble Tea would, minus the timer: a test
/// reads the runs and their output again with refresh.
struct Harness<'w> {
    m: Model,
    w: &'w World,
}

fn open(w: &World) -> Harness<'_> {
    let mut cfg = Config::new(&w.dir, &w.dir, &w.home, w.favs_now(), w.runs.clone());
    cfg.catalog_environment = runui_catalog::Environment::default().with_shell(&w.runs.execution_shell());
    cfg.catalog_environment.cargo_aliases.cargo_home = Some(std::path::Path::new(&w.home).join(".cargo"));
    let mut h = Harness { m: Model::new(cfg), w };
    h.send(tea::msg(WindowSizeMsg { width: 120, height: 36 }));
    h
}

impl Harness<'_> {
    fn send(&mut self, msg: Msg) {
        let cmd = self.m.a.update(msg);
        self.run(cmd);
        self.screen();
    }

    fn run(&mut self, cmd: Cmd) {
        let Some(f) = cmd else { return };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        let msg = rx.recv_timeout(Duration::from_secs(10)).expect("a command took over 10 s");
        let Some(msg) = msg else { return };
        if msg.is::<TickMsg>() {
            return;
        }
        match msg.downcast::<BatchMsg>() {
            Ok(b) => {
                for c in b.0 {
                    self.run(c);
                }
            }
            Err(msg) => self.send(msg),
        }
    }

    /// keys presses keys, each a key's name: enter, esc, tab, or a
    /// character.
    fn keys(&mut self, keys: &[&str]) {
        for k in keys {
            let msg = match *k {
                "enter" => KeyPressMsg::new(tea::KEY_ENTER),
                "esc" => KeyPressMsg::new(tea::KEY_ESCAPE),
                "tab" => KeyPressMsg::new(tea::KEY_TAB),
                "backspace" => KeyPressMsg::new(tea::KEY_BACKSPACE),
                k => KeyPressMsg { code: k.chars().next().unwrap() as u32, text: k.to_string(), ..Default::default() },
            };
            self.send(tea::msg(msg));
        }
    }

    fn typed(&mut self, s: &str) {
        for c in s.chars() {
            self.send(tea::msg(KeyPressMsg::char(c)));
        }
    }

    /// refresh is the timer going off: the runs and the output read again.
    fn refresh(&mut self) {
        let list = self.w.runs.list().unwrap();
        self.send(tea::msg(RunsMsg { list }));
        self.m.a.read_output();
    }

    /// screen is the interface as drawn, without its colours.
    fn screen(&mut self) -> String {
        ansi::strip(&self.m.a.draw())
    }

    /// wait_for refreshes until the screen has want.
    #[track_caller]
    fn wait_for(&mut self, want: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.refresh();
            if self.screen().contains(want) {
                return;
            }
            assert!(Instant::now() < deadline, "the screen never shows {want:?}:\n{}", self.screen());
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// select_row puts the cursor on the row named so in the view showing.
    #[track_caller]
    fn select_row(&mut self, name: &str) {
        let v = self.m.a.shown_mut();
        let Some(i) = v.shown.iter().position(|it| it.name == name) else {
            panic!("no row {name:?} in {}", v.name);
        };
        v.at = i as i32;
        self.m.a.sync_output();
    }

    #[track_caller]
    fn selected(&self) -> Item {
        self.m.a.selected().expect("no row under the cursor")
    }
}

#[test]
fn it_opens_on_the_projects_commands() {
    let w = new_world();
    let mut h = open(&w);
    assert_eq!(h.m.a.cur, PANEL_PROJECT, "with no favorites, it opens on panel {}", h.m.a.cur + 1);
    let s = h.screen();
    for want in [
        "[1]─Favorites",
        "none yet: m stars a command",
        "[2]─package.json - Makefile",
        "dev",
        "vite",
        "[3]─Here - Everywhere",
        "nothing run here yet",
        "$ npm run dev",
        "enter runs it",
    ] {
        assert!(s.contains(want), "the screen hasn't {want:?}:\n{s}");
    }
    h.keys(&["]"]);
    let it = h.selected();
    assert!(it.command == "make build" && it.desc == "Build it", "the Makefile's tab has {it:?}");
}

#[test]
fn starring_a_script() {
    let w = new_world();
    let mut h = open(&w);
    h.select_row("lint");
    h.keys(&["m"]);
    assert!(w.favs_now().has(&w.dir, &fav("npm run lint")), "m didn't keep it");
    let f = &w.favs_now().for_dir(&w.dir)[0];
    assert!(f.name == "lint" && f.source == "npm", "kept as {f:?}");
    h.keys(&["1"]);
    let it = h.selected();
    assert!(
        it.kind == Kind::Favorite && it.command == "npm run lint" && it.desc == "eslint .",
        "the favorites have {it:?}"
    );
    h.keys(&["m"]); // from the favorites: it's the project's, so no question
    assert!(
        !w.favs_now().has(&w.dir, &fav("npm run lint")) && h.m.a.confirm.is_none(),
        "m in the favorites didn't take it out"
    );
}

#[test]
fn a_command_of_your_own() {
    let w = new_world();
    let mut h = open(&w);
    h.keys(&["a"]);
    assert!(
        h.m.a.form.is_some() && h.screen().contains("A command of your own"),
        "a didn't open the form:\n{}",
        h.screen()
    );
    h.typed("echo hi");
    h.keys(&["tab"]);
    h.typed("greet");
    h.keys(&["enter"]);
    let favs = w.favs_now().for_dir(&w.dir);
    assert!(
        favs.len() == 1 && favs[0].command == "echo hi" && favs[0].name == "greet" && favs[0].own(),
        "kept {favs:?}"
    );
    assert!(
        h.m.a.cur == PANEL_FAVORITES && h.selected().command == "echo hi",
        "the cursor isn't on it in the favorites"
    );

    h.keys(&["e"]);
    for _ in 0.."greet".len() {
        h.keys(&["backspace"]);
    }
    h.typed("hello");
    h.keys(&["enter"]);
    let f = &w.favs_now().for_dir(&w.dir)[0];
    assert!(f.name == "hello" && f.command == "echo hi", "edited, it's {f:?}");

    h.keys(&["D"]);
    assert!(h.m.a.confirm.is_some(), "D on a command of your own didn't ask");
    h.keys(&["n"]);
    assert!(w.favs_now().has(&w.dir, &fav("echo hi")), "n removed it");
    h.keys(&["D", "y"]);
    assert!(!w.favs_now().has(&w.dir, &fav("echo hi")), "y didn't remove it");
}

#[test]
fn an_empty_command_isnt_kept() {
    let w = new_world();
    let mut h = open(&w);
    h.keys(&["a", "enter"]);
    assert!(
        h.m.a.form.as_ref().is_some_and(|f| !f.err.is_empty()) && w.favs_now().for_dir(&w.dir).is_empty(),
        "an empty command: favorites {:?}",
        w.favs_now().for_dir(&w.dir)
    );
    h.keys(&["esc"]);
    assert!(h.m.a.form.is_none(), "esc didn't close the form");
}

#[test]
fn a_run_outlives_runui_and_stops_from_the_next() {
    let w = new_world();
    for f in [
        Favorite { name: "tidy".into(), command: "echo tidy".into(), ..Default::default() },
        Favorite { name: "serve".into(), command: "echo serving from runui; sleep 30".into(), ..Default::default() },
    ] {
        w.add(f);
    }
    let mut first = open(&w);
    assert!(
        first.m.a.cur == PANEL_FAVORITES && first.selected().name == "tidy",
        "with favorites, it opens on panel {}, on {}",
        first.m.a.cur + 1,
        first.selected().name
    );
    first.keys(&["j", "enter"]);
    first.wait_for("serving from runui");
    first.wait_for("running for");
    first.keys(&["enter"]); // running already: it isn't started twice
    let n = w.runs.list().unwrap().len();
    assert_eq!(n, 1, "enter again started another: {n} runs");
    let live = first.m.live();
    assert_eq!(live.len(), 1, "live is {live:?}");
    first.keys(&["q"]); // runui quits; the command doesn't

    let mut next = open(&w);
    assert!(
        next.m.a.cur == PANEL_FAVORITES && next.selected().name == "serve",
        "with favorites and one going, it opens on panel {}, on {}, not the favorite that's running",
        next.m.a.cur + 1,
        next.selected().name
    );
    next.wait_for("serving from runui");
    let r = next.m.a.pane_run().unwrap();
    assert!(r.live(), "the run isn't going: {r:?}");
    next.keys(&["s"]);
    next.wait_for("stopped after");
    next.wait_for("✓ stopped echo serving");
    assert!(!runs::alive(r.pid), "the command, {}, is still there", r.pid);
    let s = next.screen();
    assert!(s.contains("■ serve"), "the Runs panel doesn't show it stopped:\n{s}");
    next.keys(&["3", "D"]);
    let list = w.runs.list().unwrap();
    assert!(list.is_empty(), "D didn't forget the run: {list:?}");
}

#[test]
fn without_favorites_it_opens_on_whats_going() {
    let w = new_world();
    let (r, err) = w.runs.start(&w.dir, "", "echo going; sleep 30");
    assert!(err.is_none(), "{err:?}");
    let mut h = open(&w);
    assert!(
        h.m.a.cur == PANEL_RUNS && h.selected().run.id == r.id,
        "with a run going and no favorites, it opens on panel {}",
        h.m.a.cur + 1
    );
    h.wait_for("going");
}

#[test]
fn an_agents_runs_have_a_tab_of_their_own() {
    let w = new_world();
    let mut h = open(&w);
    assert!(h.screen().contains("[3]─Here - Everywhere─"), "an Agents tab with no agent's run:\n{}", h.screen());
    let (mine, err) = w.runs.start(&w.dir, "", "echo by hand");
    assert!(err.is_none(), "{err:?}");
    let elsewhere = w.home.clone() + "/projects";
    let (theirs, err) = w.runs.start_by(&elsewhere, "mvn package", "echo built by an agent", "agentix");
    assert!(err.is_none(), "{err:?}");
    h.wait_for("[3]─Here - Everywhere - Agents");
    h.keys(&["3", "]", "]"]);
    let v = h.m.a.shown();
    assert!(
        v.name == "Agents" && v.all.len() == 1 && v.all[0].run.id == theirs.id,
        "the Agents tab has {:?}",
        v.all.iter().map(|it| &it.name).collect::<Vec<_>>()
    );
    h.wait_for("built by an agent");
    h.wait_for("✓ exited 0"); // D forgets only a run that's over
    let s = h.screen();
    for want in ["mvn package", "· agentix ·", "by agentix · in ~/projects"] {
        assert!(s.contains(want), "the screen hasn't {want:?}:\n{s}");
    }
    h.send(tea::msg(WindowSizeMsg { width: 200, height: 40 }));
    h.keys(&["?"]);
    assert!(h.screen().contains("here, everywhere and agents"), "the keys don't say so:\n{}", h.screen());
    h.keys(&["esc", "D"]);
    let left: Vec<String> = w.runs.list().unwrap().into_iter().map(|r| r.id).collect();
    assert!(left == [mine.id.clone()], "D didn't forget the agent's run: {left:?}");
    h.refresh();
    let p = &h.m.a.panels[PANEL_RUNS];
    assert!(
        p.views.len() == 2 && p.cur == RUNS_EVERYWHERE,
        "with no agent's run left: {} tabs, on {}",
        p.views.len(),
        p.cur
    );
}

#[test]
fn previewing_finished_output_acknowledges_its_status() {
    for (name, command) in [("done", "echo done"), ("failed", "echo failed; exit 1")] {
        let w = new_world();
        let (r, err) = w.runs.start(&w.dir, name, command);
        assert!(err.is_none(), "{err:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !w.runs.get(&r.id).is_ok_and(|r| r.state() == runs::State::Exited) {
            assert!(Instant::now() < deadline, "the run never ended");
            std::thread::sleep(Duration::from_millis(20));
        }

        let mut h = open(&w);
        assert!(!w.runs.acknowledged(&r), "an unviewed run was dismissed");
        h.keys(&["3"]);
        assert_eq!(h.selected().run.id, r.id);
        assert_eq!(h.m.a.focus, Focus::Side);
        assert!(w.runs.acknowledged(&r), "previewing the output didn't dismiss it");
        assert!(w.runs.get(&r.id).is_ok(), "viewing removed the run");
        assert!(fs::read_to_string(r.output()).unwrap().contains(name), "viewing removed the output");
    }
}

#[test]
fn only_the_previewed_run_is_acknowledged() {
    let w = new_world();
    let mut ended = Vec::new();
    for name in ["earlier", "later"] {
        let (r, err) = w.runs.start(&w.dir, name, &format!("echo {name}; exit 1"));
        assert!(err.is_none(), "{err:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !w.runs.get(&r.id).is_ok_and(|r| r.state() == runs::State::Exited) {
            assert!(Instant::now() < deadline, "the run never ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        ended.push(r);
    }

    let mut h = open(&w);
    h.keys(&["3"]);
    assert_eq!(h.selected().run.id, ended[1].id);
    assert!(w.runs.acknowledged(&ended[1]), "the previewed run was not dismissed");
    assert!(!w.runs.acknowledged(&ended[0]), "another visible row was dismissed");
    h.keys(&["j"]);
    assert_eq!(h.selected().run.id, ended[0].id);
    assert!(w.runs.acknowledged(&ended[0]), "moving to its output didn't dismiss it");
}

#[test]
fn viewed_output_acknowledges_a_run_when_it_finishes() {
    for focused in [false, true] {
        let w = new_world();
        let (r, err) = w.runs.start(&w.dir, "soon", "sleep 1; echo finished");
        assert!(err.is_none(), "{err:?}");
        let mut h = open(&w);
        h.keys(&["3"]);
        if focused {
            h.keys(&["enter"]);
        }
        assert!(!w.runs.acknowledged(&r), "a live run was dismissed");
        h.wait_for("finished");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !w.runs.acknowledged(&r) {
            h.refresh();
            assert!(Instant::now() < deadline, "the viewed run was not dismissed when it ended");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn previewing_a_live_run_does_not_acknowledge_its_future_result() {
    let w = new_world();
    let (r, err) = w.runs.start(
        &w.dir,
        "waiting",
        "echo waiting; while [ ! -f finish ]; do sleep 0.02; done; echo finished; exit 1",
    );
    assert!(err.is_none(), "{err:?}");
    let mut h = open(&w);
    h.wait_for("waiting");
    assert!(w.runs.get(&r.id).unwrap().live());
    assert!(!w.runs.acknowledged(&r), "a live run was dismissed");
    h.keys(&["2"]);
    fs::write(filepath::join(&[&w.dir, "finish"]), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !w.runs.get(&r.id).is_ok_and(|r| r.state() == runs::State::Exited) {
        assert!(Instant::now() < deadline, "the run never ended");
        std::thread::sleep(Duration::from_millis(20));
    }
    h.refresh();
    assert!(!w.runs.acknowledged(&r), "a run that finished out of view was dismissed");
    h.keys(&["3"]);
    assert!(w.runs.acknowledged(&r), "viewing the finished run didn't dismiss it");
}

#[test]
fn run_again() {
    let w = new_world();
    w.add(fav("echo started; sleep 30"));
    let mut h = open(&w);
    h.keys(&["enter"]);
    h.wait_for("running for");
    let first = h.m.a.pane_run().unwrap();
    h.keys(&["R"]);
    h.refresh();
    let again = h.m.a.pane_run().unwrap();
    assert!(again.id != first.id && again.live(), "R didn't run it again: {again:?}");
    if let Ok(r) = w.runs.get(&first.id) {
        assert!(!r.live(), "the first run is still going");
    }
}

#[test]
fn a_command_that_fails() {
    let w = new_world();
    w.add(fav("echo broken >&2; exit 4"));
    let mut h = open(&w);
    h.keys(&["enter"]);
    h.wait_for("✗ exited 4");
    let s = h.screen();
    assert!(s.contains("broken"), "its stderr isn't shown:\n{s}");
    h.keys(&["s"]);
    assert!(h.m.a.note_bad, "s on a run that ended says nothing");
}

#[test]
fn filter() {
    let w = new_world();
    let mut h = open(&w);
    h.keys(&["/"]);
    h.typed("vitest");
    let v = h.m.a.shown();
    assert!(v.shown.len() == 1 && v.shown[0].name == "test", "/vitest shows {:?}", v.shown);
    h.keys(&["enter"]);
    let v = h.m.a.shown();
    assert!(!v.typing && v.filter == "vitest", "enter didn't keep the filter");
    h.keys(&["esc"]);
    let v = h.m.a.shown();
    assert!(v.filter.is_empty() && v.shown.len() == 3, "esc didn't clear it: {:?}, {} rows", v.filter, v.shown.len());
}

#[test]
fn the_frame() {
    let w = new_world();
    let mut h = open(&w);
    for (width, height) in [(120, 36), (60, 20), (30, 8)] {
        h.send(tea::msg(WindowSizeMsg { width, height }));
        let drawn = h.m.a.draw();
        let lines: Vec<&str> = drawn.split('\n').collect();
        assert_eq!(lines.len() as i32, height, "{width}x{height}: {} lines", lines.len());
        for (i, l) in lines.iter().enumerate() {
            let lw = ansi::string_width(l);
            assert!(lw <= width, "{width}x{height}: line {i} is {lw} wide: {:?}", ansi::strip(l));
        }
    }
    h.send(tea::msg(WindowSizeMsg { width: 120, height: 36 }));
    h.keys(&["+"]);
    assert!(!h.screen().contains("[1]─Favorites"), "+ left the sidebar");
    h.keys(&["_", "?"]);
    assert!(h.screen().contains("Moving about"), "? shows no keys");
    h.keys(&["x"]);
    assert!(!h.m.a.keys_shown, "a key didn't close the keys");
    h.keys(&["0"]);
    assert_eq!(h.m.a.focus, Focus::Pane, "0 didn't give the pane the focus");
    h.keys(&["esc", "tab"]);
    assert!(
        h.m.a.focus == Focus::Side && h.m.a.cur == PANEL_RUNS,
        "esc then tab: focus {:?}, panel {}",
        h.m.a.focus,
        h.m.a.cur + 1
    );
}

#[test]
fn editing_the_projects_files_is_noticed() {
    let w = new_world();
    let mut h = open(&w);
    fs::write(filepath::join(&[&w.dir, "Taskfile.yml"]), "version: '3'\ntasks:\n  up: {desc: Start it all}\n").unwrap();
    h.m.a.ticks = 7;
    h.send(tea::msg(TickMsg(SystemTime::now())));
    let p = &h.m.a.panels[PANEL_PROJECT];
    assert!(
        p.views.len() == 3 && p.views[0].name == project::Source::Taskfile.label(),
        "the new Taskfile isn't a tab: {} tabs",
        p.views.len()
    );
}

#[test]
fn a_monorepos_folders() {
    let w = new_world();
    for (name, body) in [
        (".git/HEAD", ""),
        ("apps/web/package.json", r#"{"scripts": {"dev": "echo web in $(pwd); sleep 30"}}"#),
        ("apps/web/node_modules/x/package.json", r#"{"scripts": {"theirs": "true"}}"#),
        ("services/compose.yaml", "services:\n  db: {image: postgres:16}\n"),
        ("pom.xml", "<project><modules><module>core</module></modules></project>"),
    ] {
        let path = filepath::join(&[&w.dir, name]);
        fs::create_dir_all(filepath::dir(&path)).unwrap();
        fs::write(&path, body).unwrap();
    }
    let mut h = open(&w);
    let tabs: Vec<&str> = h.m.a.panels[PANEL_PROJECT].views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(tabs.join(" - "), "package.json - Makefile - Compose - Maven", "the tabs are");
    let s = h.screen();
    assert!(
        s.contains("apps/web › dev") && !s.contains("theirs"),
        "the web app's script isn't listed by its folder, or a dependency's is:\n{s}"
    );
    h.select_row("dev");
    let it = h.selected();
    assert!(it.rel.is_empty(), "the first dev is {}'s", it.rel);
    let v = h.m.a.shown_mut();
    if let Some(i) = v.shown.iter().rposition(|it| it.rel == "apps/web") {
        v.at = i as i32;
    }
    h.m.a.sync_output();
    let s = h.screen();
    assert!(s.contains("from apps/web/package.json"), "the pane doesn't say where it's from:\n{s}");
    h.keys(&["m"]);
    let favs = w.favs_now().for_dir(&w.dir);
    assert!(favs.len() == 1 && favs[0].dir == "apps/web" && favs[0].command == "npm run dev", "starred as {favs:?}");
    assert!(!w.favs_now().has(&w.dir, &fav("npm run dev")), "the project's own dev is starred too");
    h.keys(&["1", "enter"]);
    h.wait_for("web in ");
    let r = h.m.a.pane_run().unwrap();
    let want = filepath::join(&[&w.dir, "apps", "web"]);
    assert_eq!(r.dir, want, "it ran in {}, not {want}", r.dir);
    h.keys(&["3"]);
    let it = h.selected();
    assert!(it.kind == Kind::Run && it.place == "apps/web", "the Runs panel has {it:?}");
    let live = h.m.live();
    assert_eq!(live.len(), 1, "live is {live:?}: a run in a folder under the project counts");
    h.keys(&["s"]);
    h.wait_for("stopped after");
}

#[test]
fn human_durations() {
    use runui_core::golang::duration::SECOND;
    for (d, want) in [
        (4 * SECOND, "4s"),
        (3 * 60 * SECOND + 2 * SECOND, "3m 2s"),
        (2 * 3600 * SECOND + 5 * 60 * SECOND, "2h 5m"),
        (76 * 3600 * SECOND, "3d 4h"),
        (-SECOND, "0s"),
    ] {
        assert_eq!(view::human(d), want, "human({d})");
    }
}

// rows_test.go

#[test]
fn the_filter_puts_names_first() {
    let mut v = View::default();
    let item = |name: &str, command: &str, desc: &str, place: &str| Item {
        name: name.into(),
        command: command.into(),
        desc: desc.into(),
        place: place.into(),
        ..Default::default()
    };
    v.set(vec![
        item("build:dev", "npm run build:dev", "npm run version-check && webpack", ""),
        item("lint", "npm run lint", "eslint .", ""),
        item("version-check", "npm run version-check", "check-node-version --node 22", ""),
        item("dev", "npm run dev", "", "apps/web"),
    ]);
    v.set_filter("version-check");
    assert!(
        v.shown.len() == 2 && v.shown[0].name == "version-check" && v.shown[1].name == "build:dev",
        "got {:?}",
        v.shown
    );
    assert_eq!(v.current().unwrap().name, "version-check", "the cursor is on");
    v.set_filter("web dev");
    assert!(v.shown.len() == 2 && v.shown[0].place == "apps/web", "by its folder first: {:?}", v.shown);
    v.set_filter("");
    assert_eq!(v.current().unwrap().name, "build:dev", "cleared, the cursor isn't on the row it was on");
}

/// saved_result gives the UI a finished or lost run without starting a process.
fn saved_result(w: &World, name: &str, dir: &str, exit: Option<i32>) -> Run {
    let folder = filepath::join(&[&w.runs.root, name]);
    fs::create_dir_all(&folder).unwrap();
    let mut r = Run::default().with_folder(&folder);
    r.id = name.to_string();
    r.name = name.to_string();
    r.dir = dir.to_string();
    r.command = format!("echo {name}");
    r.started = Utc::now();
    r.ended = exit.map(|_| Utc::now());
    r.exit = exit;
    runs::write_run(&r).unwrap();
    fs::write(r.output(), format!("output of {name}\n")).unwrap();
    r
}

#[test]
fn log_lists_unread_results_and_finished_history_from_every_folder() {
    let w = new_world();
    saved_result(&w, "here", &w.dir, Some(7));
    let elsewhere = filepath::join(&[&w.home, "projects", "other"]);
    saved_result(&w, "elsewhere", &elsewhere, Some(9));
    saved_result(&w, "lost", &elsewhere, None);
    saved_result(&w, "success", &w.dir, Some(0));
    let viewed = saved_result(&w, "viewed", &w.dir, Some(1));
    w.runs.acknowledge(&viewed).unwrap();
    let mut stopped = saved_result(&w, "stopped", &w.dir, Some(1));
    stopped.stopped = true;
    runs::write_run(&stopped).unwrap();
    let (live, err) = w.runs.start(&w.dir, "live", "echo active; sleep 30");
    assert!(err.is_none(), "{err:?}");
    let mut h = open(&w);
    let mut names: Vec<_> = h.m.a.panels[PANEL_LOG].views[0].all.iter().map(|it| it.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["elsewhere", "here", "lost", "success"]);
    let mut history: Vec<_> = h.m.a.panels[PANEL_LOG].views[LOG_ALL].all.iter().map(|it| it.name.as_str()).collect();
    history.sort_unstable();
    assert_eq!(history, ["elsewhere", "here", "lost", "stopped", "success", "viewed"]);
    assert_eq!(h.m.a.pending_results(), 4);
    assert!(h.screen().contains("[4]─Log · Unread (4) - All"));
    assert!(w.runs.get(&live.id).unwrap().live());
    assert!(!w.runs.acknowledged(&live));
    h.keys(&["4", "/"]);
    h.typed("elsewhere");
    h.keys(&["enter"]);
    assert_eq!(h.selected().name, "elsewhere");
    let screen = h.screen();
    assert!(screen.contains("~/projects/other"));
    assert!(screen.contains("other › elsewhere"));
}

#[test]
fn viewing_a_result_clears_only_that_notice_and_keeps_history() {
    let w = new_world();
    let results: Vec<_> = [("first", Some(0)), ("second", Some(1)), ("third", None)]
        .into_iter()
        .map(|(name, exit)| saved_result(&w, name, &w.dir, exit))
        .collect();
    let mut h = open(&w);
    assert_eq!(h.m.a.pending_results(), 3);
    h.keys(&["4"]);
    let first = h.selected().run.id;
    assert_eq!(h.m.a.pending_results(), 2);
    for _ in 0..3 {
        h.refresh();
        assert_eq!(h.selected().run.id, first);
        assert_eq!(h.m.a.pending_results(), 2, "refresh acknowledged another result");
    }
    assert!(h.screen().contains("reviewed"));
    h.keys(&["j"]);
    let second = h.selected().run.id;
    assert_ne!(first, second);
    assert_eq!(h.m.a.pending_results(), 1);
    h.refresh();
    assert_eq!(h.selected().run.id, second);
    h.keys(&["2"]);
    assert_eq!(h.m.a.panels[PANEL_LOG].views[0].all.len(), 1);
    h.keys(&["4"]);
    assert_eq!(h.m.a.pending_results(), 0);
    assert!(h.screen().contains("Unread (0)"));
    h.keys(&["2"]);
    assert!(h.m.a.panels[PANEL_LOG].views[0].all.is_empty());
    drop(h);
    let mut reopened = open(&w);
    reopened.keys(&["4"]);
    assert!(reopened.screen().contains("all caught up; ] opens history"));
    reopened.keys(&["]"]);
    assert_eq!(reopened.m.a.panels[PANEL_LOG].cur, LOG_ALL);
    assert_eq!(reopened.m.a.shown().all.len(), 3);
    reopened.keys(&["j", "k"]);
    assert_eq!(reopened.m.a.pending_results(), 0);
    for r in results {
        assert!(w.runs.acknowledged(&r));
        assert!(w.runs.get(&r.id).is_ok());
        assert!(fs::read_to_string(r.output()).unwrap().contains(&r.name));
    }
}

#[test]
fn log_notices_results_reviewed_in_another_window() {
    let w = new_world();
    let r = saved_result(&w, "failure", &w.dir, Some(1));
    let mut h = open(&w);
    assert_eq!(h.m.a.pending_results(), 1);
    w.runs.acknowledge(&r).unwrap();
    h.refresh();
    assert!(h.m.a.panels[PANEL_LOG].views[0].all.is_empty());
    assert!(h.screen().contains("Unread (0)"));
}

#[test]
fn log_history_filters_by_result_folder_and_agent() {
    let w = new_world();
    let elsewhere = filepath::join(&[&w.home, "other"]);
    saved_result(&w, "check", &w.dir, Some(0));
    let mut failed = saved_result(&w, "build", &elsewhere, Some(2));
    failed.agent = "test-client".into();
    runs::write_run(&failed).unwrap();
    let mut h = open(&w);
    h.keys(&["4", "]", "/"]);
    h.typed("failed other test-client");
    h.keys(&["enter"]);
    assert_eq!(h.m.a.shown().shown.len(), 1);
    assert_eq!(h.selected().run.id, failed.id);
    assert!(h.screen().contains("test-client"));
    h.keys(&["esc", "/"]);
    h.typed("completed");
    h.keys(&["enter"]);
    assert_eq!(h.m.a.shown().shown.len(), 1);
    assert_eq!(h.selected().name, "check");
    assert_eq!(h.m.a.pending_results(), 0);
    h.keys(&["esc", "/"]);
    h.typed("no-matching-run");
    h.keys(&["enter"]);
    let screen = h.screen();
    assert!(screen.contains("No matching runs"));
    assert!(!screen.contains("All caught up"));
}

#[test]
fn log_sorts_by_finish_time_and_keeps_selection_when_new_results_arrive() {
    let w = new_world();
    let mut slow = saved_result(&w, "slow", &w.dir, Some(0));
    slow.started -= chrono::TimeDelta::hours(2);
    slow.ended = Some(Utc::now() + chrono::TimeDelta::seconds(1));
    runs::write_run(&slow).unwrap();
    saved_result(&w, "quick", &w.dir, Some(1));
    let mut h = open(&w);
    h.keys(&["4"]);
    assert_eq!(h.selected().name, "slow");
    let mut newest = saved_result(&w, "newest", &w.dir, Some(0));
    newest.ended = Some(Utc::now() + chrono::TimeDelta::seconds(2));
    runs::write_run(&newest).unwrap();
    h.refresh();
    assert_eq!(h.selected().name, "slow");
    assert_eq!(h.m.a.pending_results(), 2);
    assert!(!w.runs.acknowledged(&newest));
    h.keys(&["]"]);
    assert_eq!(h.selected().name, "slow", "All keeps its previous selection");
    assert_eq!(h.m.a.shown().all[0].name, "newest");
    h.keys(&["g"]);
    assert_eq!(h.selected().name, "newest");
    h.refresh();
    assert_eq!(h.selected().name, "newest");
    assert_eq!(h.m.a.pending_results(), 1);
    h.keys(&["["]);
    assert_eq!(h.selected().name, "quick");
    assert_eq!(h.m.a.pending_results(), 0);
}

#[test]
fn rerunning_from_log_follows_the_new_run_and_keeps_the_previous_attempt() {
    let w = new_world();
    let previous = saved_result(&w, "again", &w.dir, Some(0));
    let mut h = open(&w);
    h.keys(&["4", "R"]);
    assert_eq!(h.m.a.cur, PANEL_RUNS);
    let current = h.selected().run.id;
    assert_ne!(current, previous.id);
    h.wait_for("exited 0");
    assert!(w.runs.get(&previous.id).is_ok());
    h.keys(&["4", "]"]);
    assert_eq!(h.m.a.shown().all.len(), 2);
    assert!(h.m.a.shown().all.iter().any(|it| it.run.id == previous.id));
}

#[test]
fn log_fits_small_terminals_and_help_does_not_review_results() {
    let w = new_world();
    let r = saved_result(&w, "success", &w.dir, Some(0));
    let mut h = open(&w);
    h.keys(&["?"]);
    h.m.a.jump(PANEL_LOG as i32);
    h.screen();
    assert!(!w.runs.acknowledged(&r));
    h.keys(&["x"]);
    assert!(w.runs.acknowledged(&r));
    for (width, height) in [(120, 36), (60, 20), (30, 8)] {
        h.send(tea::msg(WindowSizeMsg { width, height }));
        let frame = h.screen();
        assert_eq!(frame.lines().count(), height as usize);
        assert!(frame.lines().all(|line| ansi::string_width(line) <= width));
        assert!(frame.contains("Log"));
    }
}

#[test]
fn discovery_refreshes_nested_commands_without_losing_the_filter_or_selection() {
    let world = new_world();
    let mut h = open(&world);
    h.keys(&["2", "/", "d", "e", "v", "enter"]);
    let before = h.m.a.selected().unwrap().key();
    fs::create_dir_all(filepath::join(&[&world.dir, "apps/web"])).unwrap();
    fs::write(filepath::join(&[&world.dir, "apps/web/package.json"]), r#"{"scripts":{"dev":"echo nested"}}"#).unwrap();
    h.m.a.ticks = 7;
    h.send(tea::msg(TickMsg(SystemTime::now())));
    assert_eq!(h.m.a.selected().unwrap().key(), before);
    assert_eq!(h.m.a.shown().filter, "dev");
    assert!(h.screen().contains("apps/web"));
}

#[test]
fn discovery_refresh_reads_hidden_includes_and_shows_limitations() {
    let world = new_world();
    fs::create_dir_all(filepath::join(&[&world.dir, ".tasks"])).unwrap();
    fs::write(
        filepath::join(&[&world.dir, "Taskfile.yml"]),
        "version: '3'\nincludes:\n  docs: .tasks/docs.yml\n  remote: https://example.invalid/Taskfile.yml\n",
    )
    .unwrap();
    let included = filepath::join(&[&world.dir, ".tasks/docs.yml"]);
    fs::write(&included, "version: '3'\ntasks:\n  serve: {desc: Before}\n").unwrap();
    let mut h = open(&world);
    assert!(h.m.a.shown().note.contains("not inspected"));
    assert!(h.screen().contains("discovery incomplete"));
    fs::write(&included, "version: '3'\ntasks:\n  serve: {desc: After}\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&included)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    h.m.a.ticks = 7;
    h.send(tea::msg(TickMsg(SystemTime::now())));
    assert_eq!(h.m.a.selected().unwrap().desc, "After");
    assert!(h.m.a.shown().note.contains("not inspected"));
    assert!(h.screen().contains("discovery incomplete"));
}

#[test]
fn manual_refresh_coalesces_and_discards_the_superseded_result() {
    let world = new_world();
    let mut h = open(&world);
    let first = h.m.a.refresh_files(false).unwrap();
    // Check and finish the old scan, but hold its message while another
    // refresh is requested and a new definition appears.
    let result = first().unwrap();
    assert!(h.m.a.refresh_files(true).is_none());
    assert!(h.m.a.refresh_files(true).is_none());
    fs::write(filepath::join(&[&world.dir, "Taskfile.yml"]), "version: '3'\ntasks:\n  added: echo added\n").unwrap();
    let next = h.m.a.update(result);
    assert!(h.m.a.discovery.files.iter().all(|file| file.source != project::Source::Taskfile));
    h.run(next);
    assert!(h.m.a.discovery.files.iter().any(|file| file.source == project::Source::Taskfile));
    assert!(!h.m.a.discovery_busy);
    assert!(!h.m.a.discovery_queued);
}

#[test]
fn palette_searches_sources_without_mutating_the_panel_filter_or_selection() {
    let world = new_world();
    let mut h = open(&world);
    h.keys(&["2", "/", "d", "e", "v", "enter"]);
    let before = h.m.a.selected().unwrap().key();
    h.keys(&["ctrl+p"]);
    h.typed("source:make build");
    let screen = h.screen();
    assert!(screen.contains("Commands · Project"));
    assert!(screen.contains("make build"));
    assert!(screen.contains("cwd:"));
    assert!(world.runs.list().unwrap().is_empty(), "search must never launch");
    h.keys(&["esc"]);
    assert_eq!(h.m.a.shown().filter, "dev");
    assert_eq!(h.m.a.selected().unwrap().key(), before);
}

#[test]
fn cargo_project_rows_and_palette_use_invocation_cwd_and_qualify_duplicate_package_labels() {
    let world = new_world();
    for folder in ["one", "two"] {
        let root = std::path::Path::new(&world.dir).join(folder);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname='tool'\nedition='2024'\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    }
    let mut h = open(&world);
    let cargo = h.m.a.panels[PANEL_PROJECT].views.iter().find(|view| view.name == "Cargo").unwrap();
    let rows: Vec<_> = cargo.all.iter().filter(|item| item.name == "run --bin tool (tool)").collect();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|item| item.dir == runui_catalog::canonical(&world.dir) && item.rel.is_empty()));
    h.keys(&["ctrl+p"]);
    h.typed("source:cargo run --bin tool");
    let screen = h.screen();
    assert!(screen.contains("one ›"));
    assert!(screen.contains("two ›"));
    assert!(screen.contains("manifest folder:"));
    assert!(screen.contains("cwd:"));
    assert!(world.runs.list().unwrap().is_empty());
}

#[test]
fn composer_hooks_require_a_query_and_aliases_preview_native_requirements_without_running() {
    let world = new_world();
    fs::write(filepath::join(&[&world.dir,"composer.json"]),r#"{"scripts":{"check":"@php never.php","post-install-cmd":"@php never.php"},"scripts-aliases":{"check":["verify"]}}"#).unwrap();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("source:composer verify");
    let screen = h.screen();
    assert!(screen.contains("check"));
    assert!(screen.contains("aliases: verify"));
    assert!(screen.contains("php:"));
    assert!(screen.contains("composer:"));
    assert!(!screen.contains("post-install-cmd"));
    assert!(world.runs.list().unwrap().is_empty());
    h.keys(&["esc", "ctrl+p"]);
    h.typed("source:composer kind:hook");
    let screen = h.screen();
    assert!(screen.contains("post-install-cmd"));
    assert!(screen.contains("Lifecycle hook"));
    h.keys(&["esc"]);
    let source = h.m.a.panels[PANEL_PROJECT].views.iter().find(|view| view.name == "Composer").unwrap();
    assert_eq!(source.all.len(), 1);
    assert!(source.note.contains("kind:hook"));
}

#[test]
fn queued_palette_refresh_keeps_the_latest_usable_snapshot_visible() {
    let world = new_world();
    let mut h = open(&world);
    let result = h.m.a.open_palette().unwrap()().unwrap();
    let loaded = match result.downcast::<BatchMsg>() {
        Ok(batch) => batch
            .0
            .into_iter()
            .filter_map(|command| command.and_then(|command| command()))
            .find(|msg| msg.is::<palette::Loaded>())
            .unwrap(),
        Err(message) => message,
    };
    assert!(h.m.a.refresh_palette().is_none());
    let next = h.m.a.update(loaded);
    assert!(h.screen().contains("make build"), "queued refresh must not discard the only usable snapshot");
    h.run(next);
}

#[test]
fn registered_palette_launches_the_chosen_checkout_and_keeps_project_panels() {
    let world = new_world();
    let registry = runui_catalog::registry::Store::beside_favorites(&world.favs);
    let mut projects = Vec::new();
    for (folder, branch) in [("alpha/app", "main"), ("beta/app", "feature")] {
        let directory = filepath::join(&[&world.home, folder]);
        fs::create_dir_all(filepath::join(&[&directory, ".git"])).unwrap();
        fs::write(filepath::join(&[&directory, ".git/HEAD"]), format!("ref: refs/heads/{branch}\n")).unwrap();
        fs::write(filepath::join(&[&directory, "Makefile"]), "test:\n\t@pwd > cwd.txt\n").unwrap();
        projects.push(registry.add(&directory, &world.home, Some("App")).unwrap());
    }
    let mut h = open(&world);
    h.keys(&["ctrl+p", "alt+s", "alt+s"]);
    h.typed("source:make project:beta/app test");
    h.wait_for("make test");
    assert!(h.screen().contains("Registered"));
    assert!(h.screen().contains("feature"));
    assert!(world.runs.list().unwrap().is_empty());
    h.keys(&["alt+m"]);
    assert_eq!(world.favs_now().for_dir(&projects[1].directory).len(), 1);
    assert!(world.favs_now().for_dir(&world.dir).is_empty());
    h.keys(&["enter"]);
    h.wait_for("exited 0");
    let actual = fs::read_to_string(filepath::join(&[&projects[1].directory, "cwd.txt"])).unwrap();
    assert!(runui_catalog::same_directory(actual.trim(), &projects[1].directory));
    assert!(!std::path::Path::new(&projects[0].directory).join("cwd.txt").exists());
    assert_eq!(h.m.a.cfg.dir, world.dir);
    assert_eq!(h.m.a.panels[PANEL_RUNS].cur, RUNS_EVERYWHERE);
}

#[test]
fn project_picker_registers_searches_opens_and_forgets_without_deleting_data() {
    let world = new_world();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("source:make build");
    h.keys(&["alt+p"]);
    assert!(h.screen().contains("Registered projects"));
    h.keys(&["alt+a", "enter"]);
    h.wait_for("1 explicit checkouts");
    let registry = runui_catalog::registry::Store::beside_favorites(&world.favs);
    assert_eq!(registry.load().unwrap().projects.len(), 1);
    h.keys(&["down", "enter"]);
    h.wait_for("make build");
    assert!(h.screen().contains("Registered / app"));
    h.keys(&["alt+p", "down", "ctrl+o"]);
    assert!(h.m.a.palette.is_none());
    assert!(runui_catalog::same_directory(&h.m.a.cfg.dir, &world.dir));
    h.keys(&["ctrl+p", "alt+p", "down", "alt+d"]);
    h.wait_for("0 explicit checkouts");
    assert!(registry.load().unwrap().projects.is_empty());
    assert!(std::path::Path::new(&world.dir).join("Makefile").exists());
    h.keys(&["esc", "esc"]);
}

#[test]
fn picker_cancel_keeps_the_command_query_and_surfaces_registry_errors() {
    let world = new_world();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("source:make build");
    h.keys(&["alt+p"]);
    h.typed("another project");
    h.keys(&["esc"]);
    assert!(h.screen().contains("source:make build"));
    let registry = runui_catalog::registry::Store::beside_favorites(&world.favs);
    fs::create_dir_all(registry.file().parent().unwrap()).unwrap();
    fs::write(registry.file(), "{damaged").unwrap();
    h.keys(&["alt+p"]);
    assert!(h.screen().contains("reading"));
    h.send(tea::msg(WindowSizeMsg { width: 60, height: 16 }));
    h.keys(&["ctrl+v"]);
    assert!(h.screen().lines().count() <= 16);
    h.keys(&["esc", "esc"]);
}

#[test]
fn workspace_palette_keeps_project_panels_and_executes_in_the_selected_members_directory() {
    let world = new_world();
    fs::write(filepath::join(&[&world.dir, "package.json"]), r#"{"workspaces":["packages/**"]}"#).unwrap();
    let web = filepath::join(&[&world.dir, "packages/web"]);
    let engine = filepath::join(&[&world.dir, "packages/a/b/engine"]);
    fs::create_dir_all(&web).unwrap();
    fs::create_dir_all(&engine).unwrap();
    fs::write(filepath::join(&[&web, "package.json"]), r#"{"scripts":{"web":"true"}}"#).unwrap();
    fs::write(filepath::join(&[&engine, "package.json"]), "{}").unwrap();
    fs::write(filepath::join(&[&engine, "Makefile"]), "engine:\n\t@printf 'ENGINE_READY\\n' > result.txt\n").unwrap();
    let mut cfg = Config::new(&web, &world.dir, &world.home, world.favs_now(), world.runs.clone());
    cfg.catalog_environment = runui_catalog::Environment::default().with_shell(&world.runs.execution_shell());
    cfg.catalog_environment.cargo_aliases.cargo_home = Some(std::path::Path::new(&world.home).join(".cargo"));
    let mut h = Harness { m: Model::new(cfg), w: &world };
    h.send(tea::msg(WindowSizeMsg { width: 120, height: 36 }));
    h.keys(&["ctrl+p"]);
    h.typed("source:make engine");
    assert!(h.screen().contains("No matches"));
    h.keys(&["alt+s"]);
    assert!(h.screen().contains("Commands · Workspace"));
    assert!(h.screen().contains("make engine"));
    fs::write(filepath::join(&[&engine, "Makefile"]), "engine: ## Updated\n\t@printf 'ENGINE_READY\\n' > result.txt\n")
        .unwrap();
    h.keys(&["enter"]);
    assert!(h.screen().contains("definition changed"));
    assert!(world.runs.list().unwrap().is_empty());
    h.keys(&["enter"]);
    h.wait_for("exited 0");
    assert!(fs::read_to_string(filepath::join(&[&engine, "result.txt"])).unwrap().contains("ENGINE_READY"));
    assert_eq!(h.m.a.cfg.dir, web);
    assert!(h.m.a.discovery.files.iter().all(|file| file.dir == web));
    assert_eq!(h.m.a.panels[PANEL_RUNS].cur, RUNS_EVERYWHERE);
}

#[test]
fn palette_revalidates_stale_definitions_then_launches_and_records_usage() {
    let world = new_world();
    let makefile = filepath::join(&[&world.dir, "Makefile"]);
    fs::write(&makefile, "check: ## Before\n\t@printf 'BEFORE\\n'\n").unwrap();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("source:make check");
    fs::write(&makefile, "check: ## After\n\t@printf 'AFTER\\n'\n").unwrap();
    h.keys(&["enter"]);
    assert!(h.m.a.palette.is_some());
    assert!(h.screen().contains("definition changed"));
    assert!(world.runs.list().unwrap().is_empty());
    h.keys(&["enter"]);
    h.wait_for("AFTER");
    assert!(h.m.a.palette.is_none());
    assert_eq!(h.m.a.cur, PANEL_RUNS);
    assert_eq!(h.m.a.panels[PANEL_RUNS].cur, 0, "a physical path alias must still appear Here");
    let store = runui_catalog::usage::Store::for_runs(&world.runs.root);
    assert_eq!(store.load().unwrap().recent.len(), 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while world.runs.list().unwrap().iter().any(Run::live) {
        assert!(Instant::now() < deadline);
        h.refresh();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.refresh();
    h.keys(&["ctrl+p", "tab"]);
    assert!(h.screen().contains("Recent invocations"));
    assert!(h.screen().contains("make check"));
    h.keys(&["enter"]);
    h.wait_for("AFTER");
    assert_eq!(store.load().unwrap().recent.len(), 2);
    h.keys(&["ctrl+p"]);
    h.typed("source:make check");
    assert!(h.screen().contains("2 launches"));
}

#[test]
fn a_palette_overlay_does_not_review_new_results_hidden_behind_it() {
    let world = new_world();
    let mut h = open(&world);
    h.keys(&["4", "ctrl+p"]);
    let (run, error) = world.runs.start(&world.dir, "hidden failure", "exit 7");
    assert!(error.is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        h.refresh();
        if !world.runs.get(&run.id).unwrap().live() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    h.refresh();
    let finished = world.runs.get(&run.id).unwrap();
    assert!(world.runs.pending_result(&finished));
    h.keys(&["f2"]);
    assert!(h.screen().contains("Discovery diagnostics"));
    assert!(world.runs.pending_result(&finished));
    h.keys(&["f2"]);
    h.keys(&["esc"]);
    assert!(!world.runs.pending_result(&finished));
}

#[test]
fn palette_keeps_small_layouts_and_control_characters_safe() {
    let world = new_world();
    fs::write(filepath::join(&[&world.dir, "package.json"]), "{\"scripts\":{\"bad\\u001b[2J\":\"true\"}}").unwrap();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("bad");
    for (width, height) in [(120, 36), (60, 16), (30, 10)] {
        h.send(tea::msg(WindowSizeMsg { width, height }));
        for line in h.screen().lines() {
            assert!(ansi::string_width(line) <= width, "{width}: {line:?}");
        }
        assert!(h.screen().lines().count() <= height as usize);
        assert!(h.screen().contains("bad"));
    }
    h.keys(&["ctrl+v"]);
    assert!(h.screen().contains("bad"));
}

#[test]
fn favorites_saved_from_the_palette_keep_the_existing_folder_key() {
    let world = new_world();
    let mut h = open(&world);
    h.keys(&["ctrl+p"]);
    h.typed("source:make build");
    h.keys(&["alt+m"]);
    let favorites = world.favs_now().for_dir(&world.dir);
    assert_eq!(favorites.len(), 1);
    assert_eq!(favorites[0].command, "make build");
    assert!(favorites[0].dir.is_empty());
    h.keys(&["esc"]);
    assert!(h.m.a.panels[PANEL_FAVORITES].views[0].all.iter().any(|item| item.command == "make build"));
}
