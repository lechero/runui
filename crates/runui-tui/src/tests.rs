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
    let cfg = Config::new(&w.dir, &w.dir, &w.home, w.favs_now(), w.runs.clone());
    let mut h = Harness { m: Model::new(cfg), w };
    h.send(tea::msg(WindowSizeMsg { width: 120, height: 36 }));
    h
}

impl Harness<'_> {
    fn send(&mut self, msg: Msg) {
        let cmd = self.m.a.update(msg);
        self.run(cmd);
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
fn viewing_finished_output_acknowledges_its_status() {
    let w = new_world();
    let (r, err) = w.runs.start(&w.dir, "done", "echo done");
    assert!(err.is_none(), "{err:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !w.runs.get(&r.id).is_ok_and(|r| r.state() == runs::State::Exited) {
        assert!(Instant::now() < deadline, "the run never ended");
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut h = open(&w);
    h.keys(&["3"]);
    assert_eq!(h.selected().run.id, r.id);
    assert!(!w.runs.acknowledged(&r), "opening and previewing dismissed the run");
    h.keys(&["enter"]);
    assert!(w.runs.acknowledged(&r), "viewing the output didn't dismiss it");
    assert!(w.runs.get(&r.id).is_ok(), "viewing removed the run");
}

#[test]
fn focused_output_acknowledges_a_run_when_it_finishes() {
    let w = new_world();
    let (r, err) = w.runs.start(&w.dir, "soon", "sleep 1; echo finished");
    assert!(err.is_none(), "{err:?}");
    let mut h = open(&w);
    h.keys(&["3", "enter"]);
    assert!(!w.runs.acknowledged(&r), "a live run was dismissed");
    h.wait_for("finished");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !w.runs.acknowledged(&r) {
        h.refresh();
        assert!(Instant::now() < deadline, "the focused run was not dismissed when it ended");
        std::thread::sleep(Duration::from_millis(20));
    }
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
