# Shared command catalog

`ctrl+p`, the catalog CLI, and MCP use the same definitions, IDs, fingerprints, and preparation. This covers the seven sources (including Composer and Cargo), saved favorites, persisted runs, and recent runui invocations in Project, Workspace and Registered scopes. The [implementation audit](implementation-status.md) tracks the remaining complete-v1 work.

## Find and understand

```sh
runui commands -C /path/to/project --query 'source:npm test' --json
runui find -C /path/to/project 'dir:apps/web build'
runui find -C /path/to/project --mode recent --json
runui find -C /path/to/project/packages/web --scope workspace test --json
runui inspect -C /path/to/project COMMAND_ID --json
runui projects add /path/to/project --name App --json
runui find --scope registered test --json
```

`commands` and `find` have the same search contract. Text matches names, descriptions, directories and source information. Facets are `project:`, `source:`, `dir:`, `kind:`, and `state:`. Quote a complete token or escape its colon to search it literally (`"source:npm"`, `source\:npm`); quote a facet value containing spaces (`dir:"apps/web client"`). Unknown facet names are literal text. Exact name matches rank ahead of weak description matches; favorites and usage break comparable ties.

Modes are `commands` (definitions and favorites, the default), `recent`, and `all` (also run rows). `--scope project` is the default; `--scope workspace` uses the enclosing pnpm/npm/Yarn declaration. Supply the same scope to `inspect` and `run` when selecting a sibling's ID. Search pages default to 200 entries, accept `--limit 1..500` and `--offset N`, and return `next_offset` when another page exists. JSON search responses have `schema_version: 1`, `scope`, `commands`, `diagnostics`, and `complete`. Check diagnostics even when entries were returned. A diagnostic with `kind: notice` explains policy and does not make `complete` false.

Workspace discovery reads `pnpm-workspace.yaml` before `package.json` workspaces (array or Yarn Classic `packages` object). It supports literal paths, `*`, `**`, `?`, character classes, bounded comma braces, and `!` exclusions. Patterns are relative to the declared root, with `./`, repeated separators and static `..` normalization supported. Paths escaping the root, dynamic/extglob patterns and unsupported brace forms are diagnosed. An omitted or empty pnpm package list includes only the root. Hidden directories need explicit patterns. No dependency installation or native package-manager inspection runs during discovery.

Members can be deeper than the three-level incidental scan. Both scans share limits of 2,000 visited folders and 150 source/dependency files; workspace patterns are limited to 256 (64 alternatives per brace expansion), declared traversal to 64 levels, and each ignore file to 64 KiB. `.runuiignore` uses Git-ignore syntax and applies even to declared members; repository `.gitignore` files only suppress incidental discovery. Declared ignored members are included with a notice. Symlinks, nested repositories and dependency/generated folders are not traversed. Folder watches include missing literal prefixes, so new deep members appear without restarting.

The selected project, enclosing workspace, checkout boundary and invocation folder serve different purposes. Scope changes search breadth; the recipe's cwd stays fixed. Equal text/favorite/usage matches prefer folders nearest the invocation. Workspace favorites are projected from existing folder keys without rewriting them; aliases for one execution identity are merged.

Membership references: [pnpm workspace settings](https://pnpm.io/settings#packages), [npm workspaces](https://docs.npmjs.com/cli/configuring-npm/package-json#workspaces), and [Yarn workspaces](https://yarnpkg.com/features/workspaces). Pattern and ignore parsing use the [glob](https://docs.rs/glob/latest/glob/) and [ignore](https://docs.rs/ignore/latest/ignore/) crates, with the limits above.

## Registered projects

`runui projects add DIR [--name NAME]` registers the canonical checkout root, or the enclosing declared workspace outside Git. Symbolic aliases of the same checkout deduplicate; different worktrees keep different IDs. `runui projects [--json]` lists directory availability and branch labels. `runui projects forget ID_OR_DIR` removes the registration, retaining source files, favorites and all runs/output. Nothing is added automatically. Branch labels are read from Git's local HEAD/gitfile metadata and do not define identity; [Git documents this layout](https://git-scm.com/docs/gitrepository-layout).

In the palette, `alt+p` opens a separate project picker with its own filter. `enter` searches the selected checkout (or All), preserving the command query and the main project panels. `ctrl+o` explicitly opens the checkout in those panels. `alt+a` opens a directory field (including `~/` paths); `alt+d` forgets the selected registration. `ctrl+v` exposes scrollable details. In registered command results, path and branch qualify duplicate project names. Starring a registered command saves it under that checkout's existing favorite namespace.

The registry is `projects-v1.json` beside the favorites file, normally `~/.config/runui/projects-v1.json`. It has schema version 1, up to 64 explicit checkouts, a 1 MiB size limit, atomic mode-0600 writes, a separate writer lock and corrupt-state backup before an explicit write repairs it. Unknown versions are never overwritten. The in-process discovery cache is disposable; forgetting removes its association without changing project data.

Four background workers perform passive indexing and metadata refresh. They retain ready cached results, cancel obsolete generations, and check dependencies at most once per second per checkout. A missing, retargeted or slow checkout does not remove the others' results. Initial CLI/MCP registered searches wait up to two seconds and then return ready results with `complete: false` and indexing/project diagnostics. The TUI receives partial results without waiting for the whole registry. Press `f2` to inspect discovery diagnostics.

Each checkout keeps the source scan budgets above. Registered results are additionally limited to 10,000 entries per checkout and 50,000 total, with explicit diagnostics. Within per-checkout clipping, recall, favorites and used commands take priority. Selecting one checkout in the picker filters before the global limit and result pagination. Open the checkout and use Project/Workspace scope to narrow an oversized search further. Cancellation is checked between passive directory/file reads; an individual filesystem call is subject to the operating system's behavior. No native metadata, install, shell startup or project command runs during indexing.

Registered entries carry `registry_id`, checkout, cwd, branch, definition ID and fingerprint. For CLI `inspect`/`run`, pass `--scope registered --project PROJECT_ID` using that `registry_id`. MCP `start_run` uses `scope: registered` and `project: PROJECT_ID`. Fresh preparation reads only the selected checkout and revalidates its fingerprint and physical directory; another missing registration does not block launch. IDs remain identical to the same definition in Project/Workspace scope. A retargeted registration requires explicitly registering its new location.

Each entry includes the source and origin, physical checkout and execution directory, entry file and defining file/location when known, availability, exact command, recipe, favorite aliases, usage, and run association. Included Taskfile provenance retains both the entry Taskfile and actual included definition. A missing tool does not hide a command. `project_environment` means the runner uses a project wrapper such as mise; it does not claim the tool has been resolved or provisioned.

IDs use a versioned SHA-256 digest of source, checkout, entry path, native name, variant and cwd. They survive label or command-body edits in the same checkout; a different worktree has distinct IDs. Fingerprints cover execution recipes and source dependencies. Treat both strings as opaque. IDs are not authorization tokens.

## Composer

`composer.json` scripts appear in the Composer tab and shared catalog, including nested projects; `vendor` stays excluded. Runui invokes `composer run-script -- NAME ARGS...` in the definition's cwd. The first `--` protects the script name and all forwarded arguments, including leading hyphens. Composer retains ownership of arrays, PHP callbacks, `@script`, `@php`, and `@composer`; runui does not flatten their shell text. Helpers and script bodies retain their own argument parsing too: forwarding an option to an `@composer` subcommand can be rejected by that subcommand, as it is in direct Composer. Descriptions and `scripts-aliases` annotate one canonical definition rather than creating duplicate command IDs or usage counters. Invalid metadata produces diagnostics while retaining valid scripts and other sources.

Only exact [documented Composer events](https://getcomposer.org/doc/articles/scripts.md#event-names) are classified as hooks; an arbitrary `pre-deploy` remains an ordinary script. Hooks are hidden from Commands and the Project tab by default. Search `kind:hook source:composer` to inspect or explicitly run them. `--mode all` also includes them. A hook's native event restrictions still belong to Composer.

Ambient `COMPOSER` never selects a manifest for another indexed checkout. Every recipe sets it to the selected absolute file. To choose an alternate manifest or local PHAR, create `.runui.json` in the command's working folder:

```json
{"version":1,"composer":{"manifest":"config/composer.dev.json","phar":"tools/composer.phar"}}
```

Both paths must stay inside that folder, including after symlink resolution. Omit `manifest` to use `composer.json`; omit `phar` to use PATH Composer. No PHAR is discovered or executed automatically. A selected PHAR runs through PHP; an invalid/missing selection is a source diagnostic. Manifests, settings, selected PHAR metadata, parent mise/tool-version files and resolved executable identity participate in refresh/preparation checks. PHP and Composer readiness are separate requirements; a launcher file alone does not prove PHP exists. With mise, `project_environment` means unverified rather than ready.

`alt+i` in the palette, or `runui inspect --native --json ID`, explicitly probes PHP and Composer versions. Supported Composer checks require 2.2 or newer in the 2.x series. Composer is called with `--no-plugins --no-scripts --no-interaction --no-ansi --version` and network disabled. These checks create no run, usage event or unread result. Each probe shares a 64-KiB stdout/stderr budget and a five-second deadline. Closing the palette or interrupting CLI inspection cancels its probe; timeout, cancellation, output overflow and parent exit clean up its process group. Results belong to the checked fingerprint and are not persisted as readiness. Project shell wrappers are currently skipped with an explanation, since activating them can provision tools or evaluate project configuration. This limitation is explicit rather than silently using a different environment.

Composer's [configured process timeout](https://getcomposer.org/doc/06-config.md#process-timeout) remains untouched by default. An explicit `runui run --timeout SECONDS ID -- ARGS...` overrides it for that launch; `0` disables it. MCP uses `inputs.timeout_seconds`. Other providers reject this input. General argument/timeout preset editing in the TUI remains part of the next inputs milestone.

## Cargo

Runui reads `Cargo.toml` through the maintained [`toml` parser](https://docs.rs/toml/) and infers standard target layouts without invoking Cargo, compiling code, loading procedural macros, running build scripts, writing locks, or provisioning toolchains. Root-package and virtual workspaces, member globs/exclusions, defaults, and transitive local path dependencies participate in bounded discovery. Nested Cargo workspaces found during checkout indexing expand their own deep members. Declared members override repository ignores; `.runuiignore`, dependency/generated directories, symlinks, nested repositories, and the selected workspace/checkout boundaries remain authoritative. Explicit `package.workspace` context must stay within the checkout. Package-manager membership is separate; existing explicit exclusions still prune incidental traversal unless Cargo declares a member through its own workspace.

Every action is labeled **generated**. Package build/check/test actions select their manifest and package explicitly. Workspace **default members** preserve Cargo's native default selection; **all members** add `--workspace` only when that broader action is selected. Empty-search ties prefer nearby packages and defaults over broader workspace actions. `default_member_patterns` retains the declaration; `default_member_paths` identifies discovered defaults under the current scan/ignore policy, not a replacement for Cargo's native selection. Local discovery filters never rewrite what a selected native workspace action builds.

Named binaries and executable examples receive `cargo run --bin` / `--example`. Library examples receive a build action. Library tests, integration tests and benches retain their own target flags. Targets expose their source path, crate type, default-run selection, available/default features and required features. Selecting a gated target explicitly enables only its `required-features`; runui never silently adds `--all-features`. Cargo owns default feature expansion and actual compilation. Manually declared targets override inferred names/paths, and edition-specific auto-discovery is checked against native Cargo.

The list's location identifies the manifest. `directory` remains the cwd where Cargo is invoked, and `cargo.manifest_directory` identifies the selected package/workspace separately. `cargo run` retains that invocation directory as its program's cwd; Cargo's test/bench harnesses retain their native package-directory behavior. A registered checkout selected from elsewhere uses its own root rather than the caller's unrelated folder. `dir:` matches runtime or manifest folders. Tool/source/configuration changes affect fingerprints; metadata from invocation ancestors and environment selectors is hashed without exposing configuration values. Tool paths appear in detailed previews. [Cargo documents these cwd semantics](https://doc.rust-lang.org/cargo/commands/cargo-run.html).

Arguments following `runui run ID --` reach the binary or test/bench harness after Cargo's separator. Build/check arguments remain Cargo options. General provider-input/preset editing, explicit offline native enrichment and effective alias discovery remain in subsequent increments. `alt+i` / `inspect --native` currently probe Composer; Cargo rows clearly identify their static discovery basis. A proxy on PATH does not prove a pinned Rust toolchain is installed.

Source reads are limited to **2 MiB per file** and **16 MiB total**, shared with all providers, in addition to 150 source/dependency files and 2,000 folders. Cached bytes are charged once; rejected oversized files are not reread repeatedly. Selected-folder definitions receive their read budget before membership expansion. Cargo action expansion and each target-directory listing have 1,000-item limits. Limits preserve partial results and diagnostics. The three-level incidental fallback for other source layouts remains visible even when native Cargo target coverage is complete.

## Prepare and launch

```sh
runui run -C /path/to/project COMMAND_ID --fingerprint FINGERPRINT --json -- 'argument with spaces'
runui run -C /path/to/project TASK_OR_MAKE_ID --var NAME=value --json
```

The definition is resolved from current files before launch. An expected fingerprint rejects a changed definition before starting a supervisor. Unsupported inputs and ambiguous IDs also fail before launch. Arguments are passed literally; Task and npm receive their forwarding separator. Task/Make variables become `NAME=value`, and Maven properties become `-DNAME=value`. Raw saved shell commands receive explicit arguments as positional parameters through `sh -c SCRIPT -- ARGS`, preserving pipelines and quoting in the saved script.

Search text and launch inputs are separate. Launch/inspect commands reject search-only options instead of silently ignoring them. Inspect returns one entry; a successful launch returns run metadata plus `definition_id` and `fingerprint`. A history-write error is reported separately as `history_error` because the process may already be running. Run rows focus existing runs in the TUI; they cannot be launched as command definitions.

MCP `list_commands` accepts the same query text, `scope` and page bounds and returns the same IDs. `start_run` accepts either a raw `command` or catalog `id`. For an ID, use the same `scope`; optional `fingerprint` and `inputs` (`args` array, `variables` object, and optional Composer `timeout_seconds`) use the same preparation as the CLI. Do not send `directory` with an ID: its execution context is part of the definition. Raw callers keep their existing project command/directory behavior. MCP never receives private shell recall.

## Palette actions and recall

`enter` runs a selection or focuses a matching live run. `alt+r` deliberately starts another. `alt+s` switches Project and Workspace while preserving query text; `tab` switches Commands and Recent. `alt+u` selects the current definition referenced by a recent invocation. This does not replace the stored invocation. Exact repeat retains the original command, shell and cwd. An invocation with pruned output says so and remains available for repeat.

`ctrl+v` toggles scrollable details at narrow sizes; arrows and page keys scroll them. `alt+m` toggles a favorite, `ctrl+y` copies the exact command, `ctrl+o` opens the defining source with `$EDITOR` (default `vi`), and `ctrl+t` opens the command in a terminal. `esc` closes the palette and restores the panel's filter, tab, selection, and focus. The palette does not mark covered output as reviewed.

`f2` toggles the full discovery diagnostics, including missing checkouts, indexing progress and scan limits; arrows/page keys scroll. Command and registry IDs are available in detailed previews and structured output.

Terminal handoff opens a new tmux window when runui is inside tmux. Outside tmux, configure `RUNUI_TERMINAL` with one executable path. That launcher receives two separate arguments: the absolute directory and command text. It should open a terminal in that directory and execute the command there. The launcher path is not parsed as a shell expression. External terminal work is owned by that terminal rather than runui's supervisor.

Usage state lives in `catalog-v1.json` beside the run folders. Detailed recall retains at most 1,000 invocations for 180 days; aggregate counts have at most 2,000 keys and survive output pruning. Concurrent callers deduplicate launch events by run ID. Reconciliation can recover an actual launch missing from usage after a crash, using its original launch time; opening a palette never fabricates a new launch or clears a run notice. Known final outcomes do not regress when a stale running callback arrives. File contents are bounded to 16 MiB, writes are atomic and locked, corrupt JSON is preserved before repair, and unsupported versions are not overwritten. Retention configuration, removal controls, shell recall and input presets remain pending in the full plan.

## Verify this increment

Run `task test` and `task lint` for the Rust contracts. After `task build`, run `python3 -B scripts/verify-palette.py` for real terminal checks (tmux, Make, Python and Pillow required). It uses private tmux sockets and temporary HOME/config/state, verifies source/favorite actions, panel filter restoration, wide/narrow rendering, supervised launch, Recent/current definition, and terminal handoff with a format-like directory name. Its PNG/ANSI captures are left in a printed temporary folder. Set `RUNUI_BIN` to check another build. It does not install the binary, access personal shell history, or copy to the system clipboard.

`python3 -B scripts/verify-registry.py` also creates two disposable Git repositories and two actual worktrees. It checks duplicate names/IDs, a missing checkout, registered search/diagnostics, project selection/open, exact supervised cwd, and forgetting while keeping files/runs/output. All its configuration and processes are isolated; wide/narrow PNG/ANSI captures are retained in its printed temporary folder.

`RUNUI_BIN=bin/runui RUNUI_PHP=/path/to/php RUNUI_COMPOSER_PHAR=/path/to/composer.phar python3 -B scripts/verify-composer.py` runs disposable native Composer fixtures. It downloads/installs nothing and isolates HOME, Composer config/cache, PATH and runui state. CI supplies real PHP and checksum-pinned official Composer 2.2.30 and 2.10.3 PHARs. It checks arrays, callbacks, helpers, alias/hook search, literal arguments/names, cwd, alternate manifests, local PHARs, passive version inspection, CLI/MCP identity and agent attribution.

`RUNUI_BIN=bin/runui python3 -B scripts/verify-cargo.py` compares passive discovery with installed Cargo metadata in disposable workspaces. It uses an isolated Cargo home and no downloaded packages, disables rustup auto-installation, then verifies every native member/default/target, crate type, required feature, old-edition inference, nested workspace, exact cwd and literal supervised binary arguments. CLI/MCP IDs/fingerprints and agent attribution are checked. CI runs it on macOS and Linux. The fixture's lockfile is created explicitly between passive and native phases; opening runui creates no lock or build output.
