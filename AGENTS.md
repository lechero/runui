# Repository guidelines

## What this is
`runui` is runui in Rust: one Cargo workspace, one binary, `runui`. A folder's commands at hand (a Taskfile's tasks, package.json's scripts, a Makefile's targets, a docker compose file's services, a Maven pom's builds, in its folder and the ones under it three deep), favorites per folder and commands of your own, each run under a supervisor of its own so it outlives runui, its output followed in the pane and kept, for the next runui to show and stop. It is a port of the Go runui in the dotfiles (`~/dotfiles/apps/cmd/runui`, `apps/internal/runui`), which stays the specification: `PLAN.md` says how it's ported and how it was checked.

## Layout
One crate per Go package, under `crates/`:
- `runui`: the binary (`cmd/runui`): the commands (`list`, `ps`, the hidden `_supervise`), the flags and the usage text, as Go's flag package reads and says them. Rust adds `status` for status bars and `mcp`, a stdio server in `src/cli/mcp.rs`; both use the same run store.
- `runui-tui`: the interface (`internal/runui/tui`), file for file: `lib.rs` is model.go, then `view.rs`, `rows.rs`, `styles.rs`.
- `runui-project`: the project's files (`internal/runui/project`): `taskfile.rs`, `npm.rs`, `makefile.rs`, `compose.rs`, `maven.rs`, and the folders looked in (`lib.rs`). Its `yaml` is go.yaml.in/yaml/v3 ported (from aemui's aem-local) and its `xml` Go's encoding/xml (from aemui's aem-vault), so files read as the Go build reads them and their errors read the same.
- `runui-favorites`: `~/.config/runui/favorites.json` (`internal/runui/favorites`).
- `runui-runs`: the runs and their supervisor (`internal/runui/runs`): `lib.rs` the store, `supervise.rs` the supervisor, `output.rs` what the pane shows of a run's output, `sys.rs` the system calls (flock, kill, a session and fd 3, signals), the crate's only unsafe code.
- `runui-core`: errors that wrap others, and in `golang` what Go's standard library does that runui's words and files depend on: `%q`, `encoding/json`'s bytes (MarshalIndent) and Go 1.27's JSON decoder read a token at a time with its errors (`jsontext`), `time.Time` in JSON, `time.Duration`'s words, `path/filepath`, `os` and `os/exec`'s errors, signals' names. Taken from aemui's aem-core where it has them.
- `runui-tea`: the terminal runtime (Bubble Tea's), styles (Lip Gloss's), ANSI helpers (`x/ansi`) and widgets (Bubbles') the interface is drawn with: aemui's `aem-tea`, with `set_clipboard` (OSC 52) and `Program::with_cancel` (a signal ends the program as Go's context does) added.
- `scripts/`: `compare-list.py` and `compare-tui.py`, this build checked against the Go one (`task compare`).

## Build, test and run
- `task build` builds `bin/runui`; `task install` puts it in `~/.local/bin`; `task run -- …` runs it on the folder you're in.
- `task test` tests the workspace and runs clippy with warnings as errors; `task lint` also checks `cargo fmt`. Run it before a commit. One crate: `cargo test -p runui-project`.
- `task compare` builds the Go runui from `~/dotfiles/apps` (`GO_APPS=…` for another checkout) and checks this build against it: `runui list` byte for byte on edge cases and random mutations of every kind of project file, then both interfaces side by side in a private tmux server, cell by cell after each key, and a run started by one found and stopped by the other. The Rust-only `status` and `mcp` lines are omitted from help output before comparison. Run it after a change that shows: words, layout, keys, files.
- Tests never touch your own config or runs: they work in temporary folders. A test that starts a run starts the test binary itself as its supervisor (`tests::supervise_helper`, Go's TestMain), so supervisors, process groups and signals are tested for real.

## Style
Rust 2024, `cargo fmt` (see `rustfmt.toml`), clippy clean. Comments say what something is for, in plain words, the way the code around them does; ported code keeps the Go comments. Go's names in snake_case (`ReadPackageJSON` → `read_package_json`), types as they are; where Go's name is a Rust keyword, a word for it (`For` → `for_dir`, `Move` → `move_by`). A Go function returning what it read and an error both returns `(Vec<Command>, Option<anyhow::Error>)`, the partial result kept, as Go keeps it.

## Compatibility with the Go build
Both builds use `~/.config/runui/favorites.json` and `~/.local/state/runui/runs`, written the same way (Go's `json.MarshalIndent`, `time.Time` as RFC 3339 in local time, `<`, `>` and `&` escaped), so either reads, follows and stops the other's runs, and a favorite starred in one is in the other. Keep it so: a change to what's written goes into both, or into neither. One field is Rust's alone: `agent` in run.json, the MCP client that started a run through `runui mcp` (the Agents tab's), written only when there is one. Go reads past it, and only the store that started a run and that run's own supervisor write its run.json, so a run Go started never has one and one this build started keeps it.

## Commits
Conventional commits (`feat:`, `fix:`, `docs:`, `chore:`), present tense, with the crate or area in backticks when it helps.
