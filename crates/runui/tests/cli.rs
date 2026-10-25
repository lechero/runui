//! cli_test.go, ported: the binary run as you'd run it, in a home of its
//! own.

use std::fs;
use std::process::Command;

use runui_core::golang::filepath;
use runui_favorites::{Favorite, Store};

const EXIT_OK: i32 = 0;
const EXIT_FAILURE: i32 = 1;
const EXIT_USAGE: i32 = 2;

/// Sandbox gives runui a home of its own: its config, its state, and a
/// project with a package.json and a Makefile.
struct Sandbox {
    root: String,
    dir: String,
    _t: tempfile::TempDir,
}

fn sandbox() -> Sandbox {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().to_str().unwrap().to_string();
    let dir = filepath::join(&[&root, "projects", "app"]);
    fs::create_dir_all(filepath::join(&[&dir, "src"])).unwrap();
    for (name, body) in [
        ("package.json", r#"{"packageManager": "pnpm@11", "scripts": {"dev": "vite", "build": "vite build"}}"#),
        ("Makefile", "deploy: ## Ship it\n\t./ship\n"),
        ("pnpm-lock.yaml", ""),
    ] {
        fs::write(filepath::join(&[&dir, name]), body).unwrap();
    }
    Sandbox { root, dir, _t: t }
}

impl Sandbox {
    fn favorites(&self) -> String {
        filepath::join(&[&self.root, "config", "runui", "favorites.json"])
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_runui"))
            .args(args)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", filepath::join(&[&self.root, "config"]))
            .env("XDG_STATE_HOME", filepath::join(&[&self.root, "state"]))
            .current_dir(&self.root)
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

#[test]
fn usage() {
    let s = sandbox();
    for args in [&["-h"][..], &["help"]] {
        let (code, out, _) = s.run(args);
        assert!(code == EXIT_OK && out.contains("Usage:"), "{args:?}: {code}, {out:?}");
        assert!(out.contains("runui mcp [-C DIR]"), "{args:?}: {out:?}");
        assert!(out.contains("runui status"), "{args:?}: {out:?}");
    }
    for args in [&["nope"][..], &["list", "extra"], &["-x"], &["_supervise"]] {
        let (code, _, err) = s.run(args);
        assert!(code == EXIT_USAGE && err.contains("Usage:"), "{args:?}: {code}, {err:?}");
    }
}

#[test]
fn usage_errors_say_what_gos_say() {
    let s = sandbox();
    for (args, want) in [
        (&["nope"][..], "runui: usage: unknown command \"nope\"\n\n"),
        (&["list", "extra"], "runui: usage: unexpected \"extra\"\n\n"),
        (&["-x"], "runui: usage: flag provided but not defined: -x\n\n"),
        (&["-C"], "runui: usage: flag needs an argument: -C\n\n"),
        (&["_supervise"], "runui: usage: _supervise takes a run's folder\n\n"),
    ] {
        let (_, _, err) = s.run(args);
        assert!(err.starts_with(want), "{args:?}: {err:?}");
    }
}

#[test]
fn list() {
    let s = sandbox();
    let mut favs = Store::open(&s.favorites()).unwrap();
    favs.add(
        &s.dir,
        Favorite { name: "dev".into(), command: "pnpm run dev".into(), source: "npm".into(), ..Default::default() },
    )
    .unwrap();
    favs.add(&s.dir, Favorite { command: "docker compose up".into(), ..Default::default() }).unwrap();
    // From the project's src folder: runui finds the project above it.
    let (code, out, err) = s.run(&["list", "-C", &filepath::join(&[&s.dir, "src"])]);
    assert_eq!(code, EXIT_OK, "{err}");
    let want = "~/projects/app

Favorites
  ★ dev                pnpm run dev
  $ docker compose up  docker compose up

package.json (package.json)
  dev    pnpm run dev    · vite
  build  pnpm run build  · vite build

Makefile (Makefile)
  deploy  make deploy  · Ship it

Runs here
  none
";
    assert_eq!(out, want, "got\n{out}\nwant\n{want}");
}

#[test]
fn ps_with_nothing_run() {
    let s = sandbox();
    let (code, out, _) = s.run(&["ps"]);
    assert!(code == EXIT_OK && out == "Nothing run yet.\n", "{code}, {out:?}");
}

#[test]
fn status_with_nothing_run() {
    let s = sandbox();
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=0\n");
}

#[test]
fn supervise_without_a_run() {
    let s = sandbox();
    let t = tempfile::tempdir().unwrap();
    let (code, _, err) = s.run(&["_supervise", t.path().to_str().unwrap()]);
    assert!(code == EXIT_FAILURE && !err.is_empty(), "{code}, {err:?}");
}

#[test]
fn a_run_from_the_binary_outlives_it() {
    // runui list and ps on a run the binary's own supervisor runs: what the
    // interface starts, without the interface
    let s = sandbox();
    let state = filepath::join(&[&s.root, "state", "runui", "runs"]);
    let store = runui_runs::Store {
        root: state,
        exe: vec![env!("CARGO_BIN_EXE_runui").into(), "_supervise".into()],
        shell: vec!["/bin/sh".into(), "-c".into()],
        env: Vec::new(),
    };
    let (r, err) = store.start(&s.dir, "serve", "echo serving; sleep 30");
    assert!(err.is_none(), "{err:?}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !store.get(&r.id).is_ok_and(|r| r.pid != 0) {
        assert!(std::time::Instant::now() < deadline, "the run never started");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (code, out, _) = s.run(&["ps"]);
    assert!(code == EXIT_OK && out.starts_with("● serve  echo serving; sleep 30  · running for "), "{out:?}");
    assert!(out.ends_with(" · in ~/projects/app\n"), "{out:?}");
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=1 completed=0 errored=0\n");
    let (code, out, err) = s.run(&["status", "clear"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=1 completed=0 errored=0\n");
    let (_, out, _) = s.run(&["list", "-C", &s.dir]);
    assert!(out.contains("Runs here\n  ● serve  echo serving; sleep 30  · running for "), "{out}");
    store.stop(&store.get(&r.id).unwrap()).unwrap();
    while store.get(&r.id).unwrap().live() {
        assert!(std::time::Instant::now() < deadline, "the run never stopped");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (_, out, _) = s.run(&["ps"]);
    assert!(out.starts_with("■ serve  echo serving; sleep 30  · stopped after "), "{out:?}");
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=0\n");
}

#[test]
fn status_counts_successful_and_failed_runs() {
    let s = sandbox();
    let store = runui_runs::Store {
        root: filepath::join(&[&s.root, "state", "runui", "runs"]),
        exe: vec![env!("CARGO_BIN_EXE_runui").into(), "_supervise".into()],
        shell: vec!["/bin/sh".into(), "-c".into()],
        env: Vec::new(),
    };
    let (ok, err) = store.start(&s.dir, "succeed", "exit 0");
    assert!(err.is_none(), "{err:?}");
    let (bad, err) = store.start(&s.dir, "fail", "exit 7");
    assert!(err.is_none(), "{err:?}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while ![&ok, &bad].iter().all(|r| store.get(&r.id).is_ok_and(|r| r.state() == runui_runs::State::Exited)) {
        assert!(std::time::Instant::now() < deadline, "the runs never ended");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=1 errored=1\n");

    store.acknowledge(&ok).unwrap();
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=1\n");

    let (code, out, err) = s.run(&["status", "clear"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=0\n");
    assert!(store.get(&ok.id).is_ok() && store.get(&bad.id).is_ok());
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=0\n");
}
