# Repository guidelines

## What this is
`runui` is runui in Rust: one Cargo workspace, one binary, `runui`. A folder's commands at hand (a Taskfile's tasks, package.json's scripts, a Makefile's targets, a docker compose file's services, a Maven pom's builds, a Composer manifest's scripts, Cargo actions, in its folder and the ones under it three deep), favorites per folder and commands of your own, each run under a supervisor of its own so it outlives runui, its output followed in the pane and kept, for the next runui to show and stop. The Rust version is the only active implementation. It originated as a port of the Go runui in the dotfiles; `PLAN.md` preserves that porting history. Develop and validate changes in Rust without restoring, updating, or comparing against the retired Go build.

## Layout
Crates live under `crates/`; the original Go package names below explain their history:
- `runui`: the binary (`cmd/runui`): the commands (`list`, `ps`, the hidden `_supervise`), the flags and the usage text, as Go's flag package reads and says them. Rust adds `status` for status bars and `mcp`, a stdio server in `src/cli/mcp.rs`; both use the same run store.
- `runui-tui`: the interface (`internal/runui/tui`), file for file: `lib.rs` is model.go, then `view.rs`, `rows.rs`, `styles.rs`. Rust adds panel 5, Agents, on `runui-agents`.
- `runui-project`: the project's files (`internal/runui/project`): `taskfile.rs`, `npm.rs`, `makefile.rs`, `compose.rs`, `maven.rs`, `composer.rs`, `cargo.rs` (with workspace, target and bounded alias-config helpers), and the folders looked in (`lib.rs`). Its `yaml` is go.yaml.in/yaml/v3 ported (from aemui's aem-local) and its `xml` Go's encoding/xml (from aemui's aem-vault), so files read as the Go build reads them and their errors read the same.
- `runui-catalog`: shared snapshot/search/prepare, checkout-qualified identity and fingerprints, literal tool inputs, and versioned usage recall. All callers use this rather than independently resolving display strings. `runui-project/src/workspace.rs` owns passive workspace membership and local ignore policy; declared and incidental scans share global budgets.
- `runui-catalog/src/registry.rs` stores explicit checkouts; `index.rs` caches passive discovery with four cancellable workers. `state.rs` owns the atomic, locked JSON policy shared by registry and usage. Reading ordinary Project scope does not enroll or scan unrelated checkouts.
- `runui-favorites`: `~/.config/runui/favorites.json` (`internal/runui/favorites`).
- `runui-agents`: the agent CLIs runui's MCP server can be installed for (Claude Code, Codex, GitHub Copilot, Gemini CLI, Qwen Code, Amp), the TUI's Agents panel (`5`). Rust only. It finds each CLI on the PATH, reads its settings to see whether a `runui` server is there, and installs or removes it only through the CLI's own `mcp add` / `mcp remove`, with a timeout. It never writes an agent's settings itself.
- `runui-runs`: the runs and their supervisor (`internal/runui/runs`): `lib.rs` the store, `supervise.rs` the supervisor, `output.rs` what the pane shows of a run's output, `sys.rs` the system calls (flock, kill, a session and fd 3, signals), the crate's only unsafe code. `inspection.rs` runs explicit bounded native probes with process-group cleanup, without adding runs or counters.
- `runui-core`: errors that wrap others, and in `golang` what Go's standard library does that runui's words and files depend on: `%q`, `encoding/json`'s bytes (MarshalIndent) and Go 1.27's JSON decoder read a token at a time with its errors (`jsontext`), `time.Time` in JSON, `time.Duration`'s words, `path/filepath`, `os` and `os/exec`'s errors, signals' names. Taken from aemui's aem-core where it has them.
- `runui-tea`: the terminal runtime (Bubble Tea's), styles (Lip Gloss's), ANSI helpers (`x/ansi`) and widgets (Bubbles') the interface is drawn with: aemui's `aem-tea`, with `set_clipboard` (OSC 52) and `Program::with_cancel` (a signal ends the program as Go's context does) added.
- `scripts/`: capture helpers and historical Go comparison scripts (`compare-list.py`, `compare-tui.py`). The Go comparisons are optional historical tooling, not required validation.

## Build, test and run
- `task build` builds `bin/runui`; `task install` puts it in `~/.local/bin`; `task run -- …` runs it on the folder you're in.
- `task test` tests the workspace and runs clippy with warnings as errors; `task lint` also checks `cargo fmt`. Run it before a commit. One crate: `cargo test -p runui-project`.
- Validate changed behavior with Rust regression tests and, when useful, the Rust TUI in a private tmux server with temporary config and runs. `task compare` needs retired Go sources and is not required for new changes.
- Tests never touch your own config or runs: they work in temporary folders. A test that starts a run starts the test binary itself as its supervisor (`tests::supervise_helper`, Go's TestMain), so supervisors, process groups and signals are tested for real.

## Style
Rust 2024, `cargo fmt` (see `rustfmt.toml`), clippy clean. Comments say what something is for, in plain words, the way the code around them does; ported code keeps the Go comments. Go's names in snake_case (`ReadPackageJSON` → `read_package_json`), types as they are; where Go's name is a Rust keyword, a word for it (`For` → `for_dir`, `Move` → `move_by`). A Go function returning what it read and an error both returns `(Vec<Command>, Option<anyhow::Error>)`, the partial result kept, as Go keeps it.

## Stored data compatibility
The installed Rust releases use `~/.config/runui/favorites.json` and `~/.local/state/runui/runs`. Keep existing favorites, run metadata, and output readable. Use additive versioned state or an explicit migration when changing stored data. Maintaining compatibility with the retired Go build is not a development requirement. Runs started through MCP also store an optional `agent` field identifying the client. Preserve it when updating run metadata; older runs may omit it.

## Commits
Conventional commits (`feat:`, `fix:`, `docs:`, `chore:`), present tense, with the crate or area in backticks when it helps.
