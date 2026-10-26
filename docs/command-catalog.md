# Shared command catalog

`ctrl+p`, the catalog CLI, and MCP use the same definitions, IDs, fingerprints, and preparation. This increment covers the existing five sources, saved favorites, persisted runs, and recent runui invocations in the selected project. The [implementation audit](implementation-status.md) tracks the remaining complete-v1 work.

## Find and understand

```sh
runui commands -C /path/to/project --query 'source:npm test' --json
runui find -C /path/to/project 'dir:apps/web build'
runui find -C /path/to/project --mode recent --json
runui inspect -C /path/to/project COMMAND_ID --json
```

`commands` and `find` have the same search contract. Text matches names, descriptions, directories and source information. Facets are `project:`, `source:`, `dir:`, `kind:`, and `state:`. Quote a complete token or escape its colon to search it literally (`"source:npm"`, `source\:npm`); quote a facet value containing spaces (`dir:"apps/web client"`). Unknown facet names are literal text. Exact name matches rank ahead of weak description matches; favorites and usage break comparable ties.

Modes are `commands` (definitions and favorites, the default), `recent`, and `all` (also run rows). `--scope project` is the only scope implemented in this increment. Search pages default to 200 entries, accept `--limit 1..500` and `--offset N`, and return `next_offset` when another page exists. JSON search responses have `schema_version: 1`, `commands`, `diagnostics`, and `complete`. Check diagnostics even when entries were returned.

Each entry includes the source and origin, physical checkout and execution directory, entry file and defining file/location when known, availability, exact command, recipe, favorite aliases, usage, and run association. Included Taskfile provenance retains both the entry Taskfile and actual included definition. A missing tool does not hide a command. `project_environment` means the runner uses a project wrapper such as mise; it does not claim the tool has been resolved or provisioned.

IDs use a versioned SHA-256 digest of source, checkout, entry path, native name, variant and cwd. They survive label or command-body edits in the same checkout; a different worktree has distinct IDs. Fingerprints cover execution recipes and source dependencies. Treat both strings as opaque. IDs are not authorization tokens.

## Prepare and launch

```sh
runui run -C /path/to/project COMMAND_ID --fingerprint FINGERPRINT --json -- 'argument with spaces'
runui run -C /path/to/project TASK_OR_MAKE_ID --var NAME=value --json
```

The definition is resolved from current files before launch. An expected fingerprint rejects a changed definition before starting a supervisor. Unsupported inputs and ambiguous IDs also fail before launch. Arguments are passed literally; Task and npm receive their forwarding separator. Task/Make variables become `NAME=value`, and Maven properties become `-DNAME=value`. Raw saved shell commands receive explicit arguments as positional parameters through `sh -c SCRIPT -- ARGS`, preserving pipelines and quoting in the saved script.

Search text and launch inputs are separate. Launch/inspect commands reject search-only options instead of silently ignoring them. Inspect returns one entry; a successful launch returns run metadata plus `definition_id` and `fingerprint`. A history-write error is reported separately as `history_error` because the process may already be running. Run rows focus existing runs in the TUI; they cannot be launched as command definitions.

MCP `list_commands` accepts the same query text and page bounds and returns the same IDs. `start_run` accepts either a raw `command` or catalog `id`. For an ID, optional `fingerprint` and `inputs` (`args` array, `variables` object) use the same preparation as the CLI. Do not send `directory` with an ID: its execution context is part of the definition. Raw callers keep their existing command/directory behavior. MCP never receives private shell recall.

## Palette actions and recall

`enter` runs a selection or focuses a matching live run. `alt+r` deliberately starts another. `tab` switches Commands and Recent; `alt+u` selects the current definition referenced by a recent invocation. This does not replace the stored invocation. Exact repeat retains the original command, shell and cwd. An invocation with pruned output says so and remains available for repeat.

`ctrl+v` toggles scrollable details at narrow sizes; arrows and page keys scroll them. `alt+m` toggles a favorite, `ctrl+y` copies the exact command, `ctrl+o` opens the defining source with `$EDITOR` (default `vi`), and `ctrl+t` opens the command in a terminal. `esc` closes the palette and restores the panel's filter, tab, selection, and focus. The palette does not mark covered output as reviewed.

Terminal handoff opens a new tmux window when runui is inside tmux. Outside tmux, configure `RUNUI_TERMINAL` with one executable path. That launcher receives two separate arguments: the absolute directory and command text. It should open a terminal in that directory and execute the command there. The launcher path is not parsed as a shell expression. External terminal work is owned by that terminal rather than runui's supervisor.

Usage state lives in `catalog-v1.json` beside the run folders. Detailed recall retains at most 1,000 invocations for 180 days; aggregate counts have at most 2,000 keys and survive output pruning. Concurrent callers deduplicate launch events by run ID. Reconciliation can recover an actual launch missing from usage after a crash, using its original launch time; opening a palette never fabricates a new launch or clears a run notice. Known final outcomes do not regress when a stale running callback arrives. File contents are bounded to 16 MiB, writes are atomic and locked, corrupt JSON is preserved before repair, and unsupported versions are not overwritten. Retention configuration, removal controls, shell recall and input presets remain pending in the full plan.

## Verify this increment

Run `task test` and `task lint` for the Rust contracts. After `task build`, run `python3 -B scripts/verify-palette.py` for real terminal checks (tmux, Make, Python and Pillow required). It uses private tmux sockets and temporary HOME/config/state, verifies source/favorite actions, panel filter restoration, wide/narrow rendering, supervised launch, Recent/current definition, and terminal handoff with a format-like directory name. Its PNG/ANSI captures are left in a printed temporary folder. Set `RUNUI_BIN` to check another build. It does not install the binary, access personal shell history, or copy to the system clipboard.
