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
    let (code, out, err) = s.run(&["status", "tmux"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "\n");
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
    let (code, out, err) = s.run(&["status", "tmux"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, " #[fg=#89b4fa]▶ 1#[default]\n");
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
    let (code, out, err) = s.run(&["status", "tmux"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, " #[fg=#a6e3a1]✓ 1 #[fg=#f38ba8]✗ 1#[default]\n");

    store.acknowledge(&ok).unwrap();
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=1\n");
    let (code, out, err) = s.run(&["status", "tmux"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, " #[fg=#f38ba8]✗ 1#[default]\n");

    let (code, out, err) = s.run(&["status", "clear"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=0\n");
    assert!(store.get(&ok.id).is_ok() && store.get(&bad.id).is_ok());
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=0 errored=0\n");
}

#[test]
fn tmux_hides_reviewed_errors_while_other_counts_remain() {
    let s = sandbox();
    let store = runui_runs::Store { root: filepath::join(&[&s.root, "state", "runui", "runs"]), ..Default::default() };
    let mut results = Vec::new();
    for (name, exit) in [("success", 0), ("failure", 9)] {
        let folder = filepath::join(&[&store.root, name]);
        fs::create_dir_all(&folder).unwrap();
        let mut r = runui_runs::Run::default().with_folder(&folder);
        r.id = name.into();
        r.dir = s.dir.clone();
        r.command = format!("echo {name}");
        r.started = chrono::Utc::now();
        r.ended = Some(chrono::Utc::now());
        r.exit = Some(exit);
        runui_runs::write_run(&r).unwrap();
        results.push(r);
    }
    store.acknowledge(&results[1]).unwrap();
    let (code, out, err) = s.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "running=0 completed=1 errored=0\n");
    let (code, out, err) = s.run(&["status", "tmux"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, " #[fg=#a6e3a1]✓ 1#[default]\n");
    store.acknowledge(&results[0]).unwrap();
    let (code, out, err) = s.run(&["status", "tmux"]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert_eq!(out, "\n");
}

#[test]
fn list_reports_partial_discovery_without_losing_other_commands() {
    let s = sandbox();
    fs::write(
        filepath::join(&[&s.dir, "Taskfile.yml"]),
        "version: '3'\nincludes:\n  remote: https://example.invalid/Taskfile.yml\ntasks:\n  check: echo check\n",
    )
    .unwrap();
    let (code, output, error) = s.run(&["list", "-C", &s.dir]);
    assert_eq!(code, EXIT_OK, "{error}");
    assert!(output.contains("pnpm run dev"));
    assert!(output.contains("task check"));
    assert!(output.contains("Discovery incomplete: Taskfile.yml: include remote"));
}

#[test]
fn catalog_search_inspect_launch_and_recall_share_identity_and_usage() {
    use std::time::{Duration, Instant};
    let s = sandbox();
    fs::write(filepath::join(&[&s.dir, "Makefile"]), "report:\n\t@printf 'CATALOG_READY\\n'\n").unwrap();
    let (code, out, error) = s.run(&["commands", "-C", &s.dir, "--query", "source:make report", "--json"]);
    assert_eq!(code, EXIT_OK, "{error}");
    let data: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entry = &data["commands"][0];
    assert_eq!(data["schema_version"], 1);
    assert_eq!(entry["name"], "report");
    let id = entry["id"].as_str().unwrap();
    let fingerprint = entry["fingerprint"].as_str().unwrap();
    let (code, out, error) = s.run(&["inspect", "-C", &s.dir, "--json", id]);
    assert_eq!(code, EXIT_OK, "{error}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["fingerprint"], fingerprint);
    fs::write(filepath::join(&[&s.dir, "Makefile"]), "report:\n\t@printf 'CATALOG_UPDATED\\n'\n").unwrap();
    let (code, _, error) = s.run(&["run", "-C", &s.dir, "--json", "--fingerprint", fingerprint, id]);
    assert_eq!(code, EXIT_FAILURE);
    assert!(error.contains("definition changed"));
    let (code, out, error) = s.run(&["run", "-C", &s.dir, "--json", id]);
    assert_eq!(code, EXIT_OK, "{error}");
    let started: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(started["definition_id"], id);
    let output = started["output_file"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_to_string(output).unwrap_or_default().contains("CATALOG_UPDATED") {
            break;
        }
        assert!(Instant::now() < deadline, "no output: {started}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let (code, out, error) = s.run(&["find", "-C", &s.dir, "--mode", "recent", "--json", "report"]);
    assert_eq!(code, EXIT_OK, "{error}");
    let recent: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(recent["commands"][0]["run_id"], started["id"]);
    let (code, out, error) = s.run(&["commands", "-C", &s.dir, "--json", "source:make report"]);
    assert_eq!(code, EXIT_OK, "{error}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["commands"][0]["launches"], 1);
}

#[test]
fn catalog_pages_are_bounded_and_text_output_does_not_emit_terminal_controls() {
    let s = sandbox();
    let (code, out, error) = s.run(&["commands", "-C", &s.dir, "--limit", "1", "--json"]);
    assert_eq!(code, EXIT_OK, "{error}");
    let page: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(page["commands"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_offset"], 1);
    fs::write(filepath::join(&[&s.dir, "package.json"]), "{\"scripts\":{\"bad\\u001b[2J\":\"true\"}}").unwrap();
    let (code, out, error) = s.run(&["commands", "-C", &s.dir]);
    assert_eq!(code, EXIT_OK, "{error}");
    assert!(!out.contains('\u{1b}'));
    assert!(out.contains("bad"));
}

#[test]
fn catalog_rejects_options_that_would_be_silently_ignored() {
    let s = sandbox();
    for args in [
        vec!["run", "--query", "test", "missing-id"],
        vec!["run", "--mode", "recent", "missing-id"],
        vec!["inspect", "--fingerprint", "old", "missing-id"],
        vec!["commands", "--"],
        vec!["find", "--query", "one", "two"],
        vec!["commands", "--var", "NAME=value"],
    ] {
        let (code, _, error) = s.run(&args);
        assert_eq!(code, EXIT_USAGE, "{args:?}: {error}");
    }
    assert!(!std::path::Path::new(&s.root).join("state/runui/runs/catalog-v1.json").exists());
}

#[test]
fn catalog_raw_favorite_arguments_remain_literal() {
    use std::time::{Duration, Instant};
    let s = sandbox();
    Store::open(&s.favorites())
        .unwrap()
        .add(
            &s.dir,
            Favorite { name: "literal".into(), command: "printf '%s\\n' \"$1\"".into(), ..Favorite::default() },
        )
        .unwrap();
    let (_, out, _) = s.run(&["commands", "-C", &s.dir, "--query", "source:shell literal", "--json"]);
    let page: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = page["commands"][0]["id"].as_str().unwrap();
    let argument = "spaces; touch NEVER $(echo expanded) 'quoted'";
    let (code, out, error) = s.run(&["run", "-C", &s.dir, "--json", id, "--", argument]);
    assert_eq!(code, EXIT_OK, "{error}");
    let started: serde_json::Value = serde_json::from_str(&out).unwrap();
    let output = started["output_file"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fs::read_to_string(output).unwrap_or_default().contains(argument) {
        assert!(Instant::now() < deadline, "literal output missing: {started}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!std::path::Path::new(&s.dir).join("NEVER").exists());
}

#[test]
fn workspace_scope_finds_deep_siblings_and_keeps_their_launch_context() {
    let s = sandbox();
    fs::write(filepath::join(&[&s.dir, "package.json"]), r#"{"workspaces":["packages/**"]}"#).unwrap();
    let web = filepath::join(&[&s.dir, "packages/web"]);
    let engine = filepath::join(&[&s.dir, "packages/a/b/engine"]);
    for directory in [&web, &engine] {
        fs::create_dir_all(directory).unwrap();
        fs::write(filepath::join(&[directory, "package.json"]), "{}").unwrap();
    }
    fs::write(filepath::join(&[&engine, "Makefile"]), "check:\n\t@pwd > cwd.txt\n").unwrap();
    let (code, out, error) = s.run(&["commands", "-C", &web, "--query", "source:make check", "--json"]);
    assert_eq!(code, EXIT_OK, "{error}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["commands"], serde_json::json!([]));
    let (code, out, error) =
        s.run(&["commands", "-C", &web, "--scope", "workspace", "--query", "source:make check", "--json"]);
    assert_eq!(code, EXIT_OK, "{error}");
    let page: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(page["scope"], "workspace");
    let id = page["commands"][0]["id"].as_str().unwrap();
    assert_eq!(s.run(&["inspect", "-C", &web, id]).0, EXIT_FAILURE);
    assert_eq!(s.run(&["inspect", "-C", &web, "--scope", "workspace", id]).0, EXIT_OK);
    let (code, _, error) = s.run(&["run", "-C", &web, "--scope", "workspace", id, "--json"]);
    assert_eq!(code, EXIT_OK, "{error}");
    let path = std::path::Path::new(&engine).join("cwd.txt");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(runui_catalog::same_directory(fs::read_to_string(path).unwrap().trim(), &engine));
}

#[test]
fn registered_projects_search_and_launch_keep_checkout_identity_and_existing_data() {
    let s = sandbox();
    let (code, out, error) = s.run(&["projects", "--json"]);
    assert_eq!(code, EXIT_OK, "{error}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["projects"], serde_json::json!([]));
    assert!(!std::path::Path::new(&s.favorites()).with_file_name("projects-v1.json").exists());
    let other = filepath::join(&[&s.root, "other/app"]);
    for directory in [&s.dir, &other] {
        fs::create_dir_all(filepath::join(&[directory, ".git"])).unwrap();
        fs::write(filepath::join(&[directory, ".git/HEAD"]), "ref: refs/heads/main\n").unwrap();
        fs::write(filepath::join(&[directory, "Makefile"]), "report:\n\t@pwd > cwd.txt\n").unwrap();
        assert_eq!(s.run(&["projects", "add", directory, "--name", "App", "--json"]).0, EXIT_OK);
    }
    let (code, out, error) =
        s.run(&["commands", "-C", &s.dir, "--scope", "registered", "--json", "source:make report"]);
    assert_eq!(code, EXIT_OK, "{error}");
    let page: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entries = page["commands"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "{page}");
    assert_ne!(entries[0]["id"], entries[1]["id"]);
    let selected = entries
        .iter()
        .find(|entry| runui_catalog::same_directory(entry["directory"].as_str().unwrap(), &other))
        .unwrap();
    let id = selected["id"].as_str().unwrap();
    let project = selected["registry_id"].as_str().unwrap();
    assert_eq!(selected["branch"], "main");
    assert_eq!(s.run(&["run", "-C", &s.dir, "--scope", "registered", id]).0, EXIT_USAGE);
    assert_eq!(
        s.run(&["inspect", "-C", &s.dir, "--scope", "registered", "--project", project, id, "--json"]).0,
        EXIT_OK
    );
    let (_, local, _) = s.run(&["commands", "-C", &other, "--json", "source:make report"]);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&local).unwrap()["commands"][0]["id"], id);
    // An unavailable unrelated registration must not participate in selection refresh.
    fs::rename(&s.dir, filepath::join(&[&s.root, "project-away"])).unwrap();
    let (code, out, error) = s.run(&["run", "-C", &other, "--scope", "registered", "--project", project, id, "--json"]);
    assert_eq!(code, EXIT_OK, "{error}");
    let started: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(runui_catalog::same_directory(started["directory"].as_str().unwrap(), &other));
    let path = std::path::Path::new(&other).join("cwd.txt");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(runui_catalog::same_directory(fs::read_to_string(path).unwrap().trim(), &other));
    assert_eq!(s.run(&["projects", "forget", project, "--json"]).0, EXIT_OK);
    assert!(std::path::Path::new(&other).join("Makefile").exists());
    assert!(std::path::Path::new(started["output_file"].as_str().unwrap()).exists());
}
