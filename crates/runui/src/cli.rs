//! The command line: runui, runui list, runui ps, runui status, and the hidden
//! _supervise a run's supervisor is started with. Ported from cli.go.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use runui_core::golang::{duration, filepath, quote};
use runui_favorites as favorites;
use runui_project as project;
use runui_runs::{self as runs, Run, State};
use runui_tea::tea::Program;
use runui_tui as tui;

mod catalog;
mod mcp;
mod projects;

// Exit codes.
pub(crate) const EXIT_OK: i32 = 0;
pub(crate) const EXIT_FAILURE: i32 = 1;
pub(crate) const EXIT_USAGE: i32 = 2;

const USAGE: &str = "runui keeps a folder's commands at hand and runs them: the project's own
(a Taskfile's tasks, package.json's scripts, a Makefile's targets, a docker
compose file's services, Maven builds, Composer scripts, Cargo actions), in its folder and the ones
under it three deep (a monorepo's packages, a Maven project's modules), the
ones you star as favorites (m), and commands of your own (a). What a command
prints shows as it's printed. Quit, and it keeps running; open runui again
and it's there, its output and all, for s to stop.

Usage:
  runui [-C DIR]        open the interface on the project you're in; ? shows
                        every key
  runui list [-C DIR]   print the folder's favorites, its project's commands
                        and its runs; changes nothing
  runui ps              print every run, everywhere, the ones going first
  runui status [clear|tmux]  print counts, clear finished counts, or format for tmux
  runui mcp [-C DIR]    serve project commands and runs to an MCP client over stdio
  runui commands [-C DIR] [--query TEXT] [--json]  search the shared catalog
  runui find [-C DIR] [--mode commands|recent|all] [--json] [QUERY]
  runui inspect [-C DIR] [--json] [--native] ID  inspect a command and its fingerprint
  --native explicitly checks Composer versions or offline locked Cargo metadata.
  runui run [-C DIR] [--json] [--fingerprint VALUE] ID -- [ARGS...]
  runui projects [add DIR|forget ID_OR_DIR] [--json]  manage explicit checkouts
  Composer run accepts --timeout SECONDS (0 disables; default: project setting).
  Catalog commands accept --scope project|workspace|registered (default: project).
  Registered inspect/run also require --project PROJECT_ID from discovery.

Flags:
  -C DIR  the folder (default: this one, or the project above it with a
          Taskfile, package.json, Makefile, compose file, pom.xml, composer.json or Cargo.toml, up to
          the repository's root)

The favorites are kept in ~/.config/runui/favorites.json, the runs (what each
printed, and how it ended) in ~/.local/state/runui/runs. A command runs with
sh in its folder, through mise exec where there's mise, so it gets the Node
the folder pins.
";

/// SUPERVISE_CMD is the hidden command a run's supervisor is started with.
pub(crate) const SUPERVISE_CMD: &str = "_supervise";

/// Failure is how runui was stopped: asked for its help (Go's
/// flag.ErrHelp), called wrongly (errUsage), or an error.
enum Failure {
    Help,
    Usage(String),
    Err(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Failure {
        Failure::Err(e)
    }
}

pub(crate) fn run(canceled: &Arc<AtomicBool>, args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    match dispatch(canceled, args, stdout, stderr) {
        Ok(()) => EXIT_OK,
        Err(Failure::Help) => {
            let _ = write!(stdout, "{USAGE}");
            EXIT_OK
        }
        Err(Failure::Usage(msg)) => {
            let _ = write!(stderr, "runui: usage: {msg}\n\n{USAGE}");
            EXIT_USAGE
        }
        Err(Failure::Err(e)) => {
            let _ = writeln!(stderr, "runui: {e}");
            EXIT_FAILURE
        }
    }
}

fn dispatch(
    canceled: &Arc<AtomicBool>,
    args: &[String],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> std::result::Result<(), Failure> {
    let (mut cmd, mut args) = ("", args);
    if let Some(first) = args.first()
        && !first.starts_with('-')
    {
        (cmd, args) = (first.as_str(), &args[1..]);
    }
    if cmd == SUPERVISE_CMD {
        if args.len() != 1 {
            return Err(Failure::Usage(format!("{SUPERVISE_CMD} takes a run's folder")));
        }
        return Ok(runs::supervise(&args[0])?);
    }
    if matches!(cmd, "commands" | "find" | "inspect" | "run") {
        return catalog::dispatch(canceled, cmd, args, stdout);
    }
    if cmd == "projects" {
        return projects::dispatch(args, stdout);
    }
    let (dir, rest) = parse_flags(args)?;
    let clear_status = cmd == "status" && rest.as_slice() == ["clear"];
    let tmux_status = cmd == "status" && rest.as_slice() == ["tmux"];
    if let Some(extra) = rest.first().filter(|_| !clear_status && !tmux_status) {
        return Err(Failure::Usage(format!("unexpected {}", quote(extra))));
    }
    let dir = match dir {
        Some(d) if !d.is_empty() => d,
        _ => filepath::getwd().map_err(anyhow::Error::from)?,
    };
    let abs = filepath::abs(&dir).map_err(anyhow::Error::from)?;
    let home = filepath::user_home_dir().unwrap_or_default();
    let dir = project::root(&abs, &home);
    let w = Workspace {
        repo: project::context_root(&dir, &home),
        dir,
        home,
        invocation_dir: abs,
        index: runui_catalog::index::Index::default(),
    };
    match cmd {
        "" => open(canceled, &w, stderr)?,
        "list" => list(&w, stdout)?,
        "ps" => ps(&w, stdout)?,
        "status" => status(&w, clear_status, tmux_status, stdout)?,
        "mcp" => mcp::serve(&w, std::io::stdin().lock(), stdout)?,
        "help" => return Err(Failure::Help),
        _ => return Err(Failure::Usage(format!("unknown command {}", quote(cmd)))),
    }
    Ok(())
}

/// parse_flags reads the flags as Go's flag package reads them for runui's
/// one, -C DIR (-C=DIR, --C DIR too): up to the first argument that isn't
/// a flag, or --; the rest are the arguments left.
fn parse_flags(args: &[String]) -> std::result::Result<(Option<String>, Vec<String>), Failure> {
    let mut dir = None;
    let mut i = 0;
    while i < args.len() {
        let s = &args[i];
        if s.len() < 2 || !s.starts_with('-') {
            break;
        }
        let mut minuses = 1;
        if s.as_bytes()[1] == b'-' {
            minuses += 1;
            if s.len() == 2 {
                // "--" terminates the flags
                i += 1;
                break;
            }
        }
        let name = &s[minuses..];
        if name.is_empty() || name.starts_with('-') || name.starts_with('=') {
            return Err(Failure::Usage(format!("bad flag syntax: {s}")));
        }
        i += 1;
        let (name, value) = match name.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (name, None),
        };
        if name != "C" {
            if name == "help" || name == "h" {
                return Err(Failure::Help);
            }
            return Err(Failure::Usage(format!("flag provided but not defined: -{name}")));
        }
        let value = match value {
            Some(v) => v,
            None if i < args.len() => {
                i += 1;
                args[i - 1].clone()
            }
            None => return Err(Failure::Usage(format!("flag needs an argument: -{name}"))),
        };
        dir = Some(value);
    }
    Ok((dir, args[i..].to_vec()))
}

/// Workspace is the folder runui works in, and where it keeps things.
struct Workspace {
    dir: String,
    repo: String,
    home: String,
    invocation_dir: String,
    index: runui_catalog::index::Index,
}

impl Workspace {
    fn catalog_scoped(&self, scope: runui_catalog::Scope) -> Result<runui_catalog::Snapshot> {
        let store = self.runs()?;
        let usage_store = runui_catalog::usage::Store::for_runs(&store.root);
        let observed = store.list()?;
        let _ = usage_store.reconcile(&observed);
        let usage = usage_store.load();
        let directory = scope.directory(&self.dir, &self.repo);
        let favorites = self.favorites()?;
        let environment = runui_catalog::Environment::current().with_shell(&store.shell);
        let default_usage = runui_catalog::usage::Data::default();
        if directory.is_none() {
            let registry = self.registry()?;
            let projects = match registry.load() {
                Ok(data) => data.projects,
                Err(error) => {
                    let mut snapshot = runui_catalog::Snapshot::default();
                    snapshot.diagnostics.push(runui_catalog::Diagnostic {
                        path: registry.file().to_string_lossy().into_owned(),
                        message: error.to_string(),
                        source: None,
                        kind: "registry".into(),
                    });
                    return Ok(snapshot);
                }
            };
            self.index.request(&projects);
            let indexed = self.index.wait(std::time::Duration::from_secs(2));
            let mut snapshot = runui_catalog::Snapshot::registered(
                &indexed,
                &environment,
                &self.invocation_dir,
                &favorites,
                &observed,
                usage.as_ref().unwrap_or(&default_usage),
            );
            if let Err(error) = usage {
                snapshot.diagnostics.push(runui_catalog::Diagnostic {
                    path: usage_store.file().to_string_lossy().into_owned(),
                    message: error.to_string(),
                    source: Some("usage".into()),
                    kind: "history".into(),
                });
            }
            return Ok(snapshot);
        }
        let directory = directory.expect("local scope was checked");
        let mut context = runui_catalog::Context::new(&directory, &self.repo);
        context.invocation_dir = runui_catalog::canonical(&self.invocation_dir);
        context.environment = context.environment.with_shell(&store.shell);
        let mut snapshot = runui_catalog::Snapshot::build(
            &context,
            &project::discover(&directory, &self.repo),
            &if scope == runui_catalog::Scope::Workspace {
                favorites.for_tree(&directory)
            } else {
                favorites.for_dir(&self.dir)
            },
            &observed,
            usage.as_ref().unwrap_or(&default_usage),
        );
        if let Err(error) = usage {
            snapshot.diagnostics.push(runui_catalog::Diagnostic {
                path: usage_store.file().to_string_lossy().into_owned(),
                message: error.to_string(),
                source: Some("usage".into()),
                kind: "history".into(),
            });
        }
        Ok(snapshot)
    }

    fn registry(&self) -> Result<runui_catalog::registry::Store> {
        Ok(runui_catalog::registry::Store::beside_favorites(&favorites::default_file()?))
    }

    fn catalog_selection(
        &self,
        scope: runui_catalog::Scope,
        registered: Option<&str>,
    ) -> Result<runui_catalog::Snapshot> {
        if scope != runui_catalog::Scope::Registered {
            if registered.is_some() {
                return Err(anyhow!("project identity applies only to registered scope"));
            }
            return self.catalog_scoped(scope);
        }
        let selected = self
            .registry()?
            .project(registered.ok_or_else(|| anyhow!("registered selection requires its project identity"))?)?;
        selected.check_directory()?;
        let store = self.runs()?;
        let usage_store = runui_catalog::usage::Store::for_runs(&store.root);
        let usage = usage_store.load();
        let default_usage = runui_catalog::usage::Data::default();
        let mut context = runui_catalog::Context::new(&selected.directory, &selected.directory);
        context.invocation_dir = runui_catalog::canonical(&self.invocation_dir);
        context.environment = context.environment.with_shell(&store.shell);
        let mut snapshot = runui_catalog::Snapshot::build(
            &context,
            &project::discover(&selected.directory, &selected.directory),
            &self.favorites()?.for_tree(&selected.directory),
            &store.list()?,
            usage.as_ref().unwrap_or(&default_usage),
        )
        .with_registration(&selected, runui_catalog::index::branch(&selected.directory).as_deref());
        if let Err(error) = usage {
            snapshot.diagnostics.push(runui_catalog::Diagnostic {
                path: usage_store.file().to_string_lossy().into_owned(),
                message: error.to_string(),
                source: Some("usage".into()),
                kind: "history".into(),
            });
        }
        Ok(snapshot)
    }

    fn favorites(&self) -> Result<favorites::Store> {
        favorites::Store::open(&favorites::default_file()?)
    }

    fn runs(&self) -> Result<runs::Store> {
        let root = runs::default_root()?;
        let exe = std::env::current_exe()?;
        let exe = exe.to_str().ok_or_else(|| anyhow!("runui's own path isn't UTF-8"))?.to_string();
        let mut s =
            runs::Store { root, exe: vec![exe, SUPERVISE_CMD.into()], shell: runs::default_shell(), env: Vec::new() };
        if std::env::var_os("NO_COLOR").is_none() {
            // Without a terminal, most tools print without colour; the pane shows it.
            s.env = vec!["FORCE_COLOR=1".into(), "CLICOLOR_FORCE=1".into()];
        }
        Ok(s)
    }
}

/// open runs the interface, then says what keeps running.
fn open(canceled: &Arc<AtomicBool>, w: &Workspace, stderr: &mut dyn Write) -> Result<()> {
    let favs = w.favorites()?;
    let store = w.runs()?;
    let mut cfg = tui::Config::new(&w.dir, &w.repo, &w.home, favs, Arc::new(store));
    cfg.invocation_dir = w.invocation_dir.clone();
    let model = Program::new(tui::Model::new(cfg)).with_cancel(canceled.clone()).run()?;
    let live = model.live();
    if !live.is_empty() {
        let names: Vec<&str> = live.iter().map(|r| r.command.as_str()).collect();
        let verb = if live.len() > 1 { "keep" } else { "keeps" };
        writeln!(
            stderr,
            "runui: {} {verb} running in {}; runui there shows it, and s stops it",
            names.join(", "),
            short(&w.dir, &w.home)
        )?;
    }
    Ok(())
}

/// list prints the folder's favorites, its project's commands and its
/// runs.
fn list(w: &Workspace, out: &mut dyn Write) -> Result<()> {
    let favs = w.favorites()?;
    let store = w.runs()?;
    let all = store.list()?;
    let now = Utc::now();
    writeln!(out, "{}", short(&w.dir, &w.home))?;
    let mut lines = Vec::new();
    for f in favs.for_dir(&w.dir) {
        let mut mark = if f.own() { "$ ".to_string() } else { "★ ".to_string() };
        if !f.dir.is_empty() {
            mark += &format!("{} › ", f.dir);
        }
        lines.push([mark + f.label(), f.command.clone(), String::new()]);
    }
    rows(out, "Favorites", &lines)?;
    let discovery = project::discover(&w.dir, &w.repo);
    let files = &discovery.files;
    if files.is_empty() {
        writeln!(
            out,
            "\nNo Taskfile, package.json, Makefile, compose file, pom.xml, composer.json or Cargo.toml here, or in the folders under it."
        )?;
    }
    for f in files {
        let lines: Vec<[String; 3]> = f
            .commands
            .iter()
            .filter(|command| !command.hook)
            .map(|c| [c.name.clone(), c.run.clone(), c.desc.clone()])
            .collect();
        let mut title = format!("{} ({})", f.source.label(), f.rel_path());
        if let Some(err) = &f.err {
            title += &format!(": {err}");
        }
        rows(out, &title, &lines)?;
        if f.commands.iter().any(|command| command.hook) {
            writeln!(out, "  Lifecycle hooks: runui find kind:hook source:composer")?;
        }
    }
    for diagnostic in &discovery.diagnostics {
        writeln!(out, "\nDiscovery incomplete: {}", diagnostic.description(&w.dir))?;
    }
    let mut lines = Vec::new();
    for r in &all {
        if let Some(rel) = runui_catalog::relative_within(&w.dir, &r.dir)
            && rel != ".."
            && !rel.starts_with("../")
        {
            let mut name = r.label().to_string();
            if rel != "." {
                name = format!("{rel} › {name}");
            }
            lines.push([format!("{} {name}", state_mark(r)), r.command.clone(), state_words(r, now)]);
        }
    }
    rows(out, "Runs here", &lines)?;
    Ok(())
}

/// rows prints a title and its lines in columns: a name, a command and
/// what it says, or none.
fn rows(out: &mut dyn Write, title: &str, lines: &[[String; 3]]) -> Result<()> {
    write!(out, "\n{title}\n")?;
    if lines.is_empty() {
        writeln!(out, "  none")?;
        return Ok(());
    }
    let name = lines.iter().map(|l| l[0].chars().count()).max().unwrap_or(0);
    let cmd = lines.iter().map(|l| l[1].chars().count()).max().unwrap_or(0);
    for l in lines {
        let line = if l[2].is_empty() {
            format!("  {:<name$}  {}", l[0], l[1])
        } else {
            format!("  {:<name$}  {:<cmd$}  · {}", l[0], l[1], l[2])
        };
        writeln!(out, "{}", line.trim_end_matches(' '))?;
    }
    Ok(())
}

/// ps prints every run, everywhere.
fn ps(w: &Workspace, out: &mut dyn Write) -> Result<()> {
    let store = w.runs()?;
    let all = store.list()?;
    if all.is_empty() {
        writeln!(out, "Nothing run yet.")?;
        return Ok(());
    }
    let now = Utc::now();
    for live in [true, false] {
        for r in all.iter().filter(|r| r.live() == live) {
            writeln!(
                out,
                "{} {}  {}  · {} · in {}",
                state_mark(r),
                r.label(),
                r.command,
                state_words(r, now),
                short(&r.dir, &w.home)
            )?;
        }
    }
    Ok(())
}

/// status prints counts for status bars. Completed and errored runs count
/// until viewed or cleared. Stopped runs are left out.
fn status(w: &Workspace, clear: bool, tmux: bool, out: &mut dyn Write) -> Result<()> {
    let mut running = 0;
    let mut completed = 0;
    let mut errored = 0;
    let store = w.runs()?;
    for r in store.list()? {
        if clear {
            store.acknowledge(&r)?;
        }
        match r.state() {
            State::Starting | State::Running => running += 1,
            State::Exited if r.exit == Some(0) && store.pending_result(&r) => completed += 1,
            State::Exited | State::Lost if store.pending_error(&r) => errored += 1,
            State::Stopped => {}
            State::Exited | State::Lost => {}
        }
    }
    if tmux {
        if running > 0 {
            write!(out, " #[fg=#89b4fa]▶ {running}")?;
        }
        if completed > 0 {
            write!(out, " #[fg=#a6e3a1]✓ {completed}")?;
        }
        if errored > 0 {
            write!(out, " #[fg=#f38ba8]✗ {errored}")?;
        }
        if running + completed + errored > 0 {
            write!(out, "#[default]")?;
        }
        writeln!(out)?;
    } else {
        writeln!(out, "running={running} completed={completed} errored={errored}")?;
    }
    Ok(())
}

fn state_mark(r: &Run) -> &'static str {
    match r.state() {
        State::Starting => "◌",
        State::Running => "●",
        State::Stopped => "■",
        State::Lost => "?",
        State::Exited if r.exit == Some(0) => "✓",
        State::Exited => "✗",
    }
}

fn state_words(r: &Run, now: DateTime<Utc>) -> String {
    let took = duration::duration_string(duration::truncate(r.took(now), duration::SECOND));
    match r.state() {
        State::Starting => return "starting".into(),
        State::Running => return format!("running for {took}, pid {}", r.pid),
        State::Stopped => return format!("stopped after {took}"),
        State::Lost => return "lost track of it".into(),
        State::Exited => {}
    }
    if !r.error.is_empty() && r.pid == 0 {
        format!("couldn't start: {}", r.error)
    } else if !r.signal.is_empty() {
        format!("ended by {} after {took}", r.signal)
    } else if let Some(code) = r.exit {
        format!("exited {code} after {took}")
    } else {
        "ended".into()
    }
}

/// short is a folder with the home folder written ~.
fn short(dir: &str, home: &str) -> String {
    if !home.is_empty() && (dir == home || dir.starts_with(&format!("{home}/"))) {
        return format!("~{}", &dir[home.len()..]);
    }
    dir.to_string()
}
