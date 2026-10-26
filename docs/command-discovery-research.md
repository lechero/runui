# Command discovery and product completeness research

Updated 26 October 2026 against `main` at `326b0c2`. This extends the earlier research from `codex/command-discovery-research`; both of that branch's changes are already patch-equivalent on main. The two research documents were identical between that branch and current main before this update. The fresh worktree starts from main so it includes Log, result review, agent attribution, and the refreshed user guide.

The recommendation is a shared command catalog with one palette across sources and registered projects, accurate workspace discovery, Composer and Cargo support, and useful recall of the user's own commands. These belong in one bounded product release. Build on the existing supervisor, Log, tmux status, and MCP rather than replacing them.

The confirmed first priority is finding the right command faster across projects and monorepos. Composer, Rust, and user command history are explicit requirements of this update. The [integration plan](command-discovery-plan.md) defines the release scope, implementation slices, and acceptance checks. It describes proposed behavior; these integrations are not shipped by this research PR. [CONTEXT.md](../CONTEXT.md) distinguishes definitions, invocations, runs, and the different kinds of history.

## What works now, and where finding commands breaks down

| Area | Current Rust behavior | Consequence |
| --- | --- | --- |
| Entry points | Tmux popup in the active pane's directory; Neovim terminal float | Keep these quick entry points. No new desktop application is needed. |
| Project context | Nearest ancestor with one of five supported project files, bounded by repository/home | A nested package becomes the selected project. Cargo and Composer manifests do not establish roots. |
| Discovery | Taskfile, package scripts, Make, Compose, Maven; descendants three levels deep | Declared workspace membership is not the discovery rule. Unsupported sources are invisible. |
| Search | `/` filters the current tab using case-insensitive words; name/folder matches precede description-only matches | Finding a command still requires knowing its source tab or project. There is no shared fuzzy palette. |
| Identity | Command row identity uses folder plus rendered command; favorites save shell text and relative folder | Different origins can look identical. Source navigation and stable selection need richer identity. |
| Refresh | Polls root manifests and previously returned files every two seconds; `r` rescans | Hidden included Taskfiles and newly created nested manifests can be absent from invalidation. |
| Execution | One persistent supervisor per run; output retained; existing mise environment wrapper | Strong basis for launch, leave, return, and stop. Stdin is an open pipe, without terminal input forwarding or a PTY. |
| Log and notices | Unread/All finished results; viewing a finished result marks it reviewed; zero status counts hidden | Already solves the earlier growing-notice problem. Preserve this behavior. Looking at a live process must not review its future result. |
| Retention | The next start prunes ended runs to 40 previous results; live runs are retained | Saved output is bounded. It cannot also be the only long-term source of recent/frequent command ranking. |
| CLI and MCP | Human-readable `list`; six MCP tools; raw-command start; bounded tail output | Parser sharing exists, but there is no shared catalog ID, filtered catalog API, or incremental output cursor. MCP output reads do not review results. |

Implementation references: [roots, traversal and refresh](../crates/runui-project/src/lib.rs), [Taskfile includes](../crates/runui-project/src/taskfile.rs), [package manager selection](../crates/runui-project/src/npm.rs), [row filtering](../crates/runui-tui/src/rows.rs), [TUI](../crates/runui-tui/src/lib.rs), [CLI](../crates/runui/src/cli.rs), [MCP](../crates/runui/src/cli/mcp.rs), [run store and review](../crates/runui-runs/src/lib.rs), [supervisor](../crates/runui-runs/src/supervise.rs). Existing [user documentation](index.html) covers shipped behavior.

The configured workflow uses fish with fzf history search, tmux `Alt-X`, and Neovim `Space o x`. These are observations of configuration, not measurements of interaction frequency. Runui should connect command recall to a selected project, exact invocation, and saved output. Replacing a working Ctrl-R picker adds little by itself.

## Evidence and limits

### Local inventory

The updated read-only sample used `rg --files --hidden` under `~/projects`, honoring ignore rules and excluding Git data, dependency folders, and common build output. Counts include examples, copies, and fixtures; top-level folders are not necessarily active repositories. This pass used specific filename globs, so it is not a complete census of every Taskfile/Just/Compose spelling. Do not compare its narrower source counts to the earlier pass as a trend.

| Selected manifests | Files | Top-level folders | What this establishes |
| --- | ---: | ---: | --- |
| `package.json` | 350 | 34 | Existing package discovery remains central. |
| `pnpm-workspace.yaml` | 46 | 26 | Workspace membership deserves first-class handling. |
| `composer.json` | 20 | 3 | PHP support is useful in this user's projects. |
| `Cargo.toml` | 67 | 6 | Rust is a substantial gap, including runui itself. |
| `Taskfile.yml` / `Taskfile.yaml` | 54 / 26 | 31 / 7 | Existing sources must retain good coverage; folder counts overlap. |
| `Makefile` | 22 | 4 | Keep existing static target discovery. |
| `build.gradle` / `build.gradle.kts` | 80 / 3 | 6 / 1 | Gradle merits a later native-inspection spike. |
| `pyproject.toml` | 13 | 4 | Python is a later source with environment-dependent entry points. |
| `turbo.json` / `nx.json` | 29 / 5 | 11 / 4 | Enrich package entries rather than build another scheduler. |

Seven of the 20 Composer manifests declare scripts, totaling 18 script keys. Twelve values are arrays, and 12 keys match a pre/post event naming pattern; exact Composer event classification still requires its documented event set. This argues for delegating execution to Composer and distinguishing lifecycle hooks from deliberately named developer scripts. No descriptions or aliases were present in this sample. Seven manifests had Symfony dependencies, and none declared Laravel's framework package; that favors a Symfony inspection experiment before an Artisan integration if framework commands become necessary.

The Cargo manifests include five workspaces and 63 package manifests, with overlap between categories. On this checkout, explicit `cargo metadata --no-deps --format-version 1 --offline --locked` succeeded: seven workspace members, one binary target, and six library targets.

Composer's launcher exists in the inspection shell, but `composer --no-plugins --no-scripts --version` in a disposable directory failed because `php` was absent from that shell's PATH. This prevented local Composer execution validation. It does not establish PHP availability inside each project's mise environment. Availability must be checked in the selected execution context; do not install tools merely to list commands.

### History sample

A bounded, local read found 6,343 stored fish command records in an approximately 842 KB file. All had `when`; some had `paths` and `added_when`; none had a working-directory field. These are stored records, not an execution-frequency measurement. Zsh and Bash history files also exist. Atuin was not found at the inspected executable/database locations.

Only aggregate counts and metadata presence were retained for this research. No personal command text, project paths, credentials, or history database is included in this repository. Inventory and history observations are a snapshot of this machine, not portable runtime assumptions.

### Reproduced against current main

Run the committed probe from the checkout, with cached Cargo dependencies:

```sh
python3 -B scripts/probe-discovery.py
# Optional: set CARGO_TARGET_DIR to reuse an existing temporary build cache.
```

It builds a temporary Rust caller of the public `runui-project` interface, creates its own fixtures, and prints observations. It does not execute fixture commands or access personal configuration/runs. It is a research probe, not a test that requires bugs to remain.

| Fixture | Observed at `326b0c2` | Required change |
| --- | --- | --- |
| Cargo-only and Composer-only repositories, opened below their manifests | Neither selects the manifest root; no commands | Integrate roots and execution context as well as parsers. |
| pnpm member at `packages/a/b/c` | Only the root script appears | Read declared membership beyond the incidental depth limit. |
| Shared Taskfile under `one` and `two` | Only `one:build` appears | Separate cycle detection from namespace expansion. |
| Edit hidden included Taskfile | `changed` unchanged; explicit reload reads the edit | Track all defining dependencies. |
| Create nested `package.json` | `changed` unchanged; explicit reload finds it | Track directory changes or reconcile periodically. |
| Package inside a `.gitignore` entry | Command is listed | Define ignore policy explicitly; some ignored local definitions are useful. |

The earlier research also checked the namespace fixture with Task's JSON listing, which returned both namespaces. Code inspection explains the present result: one global visited-file set suppresses a second namespace. Scan limits are also not a strict, globally enforced budget, and no completeness diagnostic reaches callers.

## Composer: support scripts without reimplementing Composer

Composer supports root-package script declarations as strings or arrays, executable commands and PHP callbacks, plus script references and helpers such as `@php` and `@composer`. Descriptions and aliases have dedicated metadata. Dependencies' script declarations are not scripts of the consuming root package. [Scripts](https://getcomposer.org/doc/articles/scripts.md), [schema](https://getcomposer.org/doc/04-schema.md#scripts).

Recommended first integration:

- Parse the selected `composer.json` passively, including `scripts`, `scripts-descriptions`, and `scripts-aliases`. Do not sweep `vendor`. A nested root manifest remains a separate project context.
- Offer ordinary scripts in default results; put actual Composer lifecycle events behind a hook facet or explicit hook view. Do not classify every arbitrary `pre-*` name as a lifecycle event.
- Prepare `composer run-script SCRIPT -- ARGS`, quoting each value and preserving the script's native key. Let Composer handle arrays, callbacks, references, and its version-specific argument behavior. Alias labels resolve to the canonical script, avoiding duplicate ranking.
- Show the effective manifest, directory, Composer/PHP availability, and exact invocation. Support a nondefault manifest only through explicit context; do not apply an unrelated ambient `COMPOSER` value to every indexed repository.

Composer's CLI can list scripts and vendor executables. Treat vendor tools as optional enrichment, not inferred scripts; honor configured binary directories. Its process timeout can affect long-running scripts: make an explicit timeout choice available later rather than silently overriding the project's policy. [CLI](https://getcomposer.org/doc/03-cli.md#run-script-run), [timeout configuration](https://getcomposer.org/doc/06-config.md#process-timeout).

Native inspection is distinct from passive parsing: Composer commands can load plugins and dispatch events. Use installed tools with `--no-plugins --no-scripts` for an explicit metadata check, and test the supported versions. This does not justify requiring confirmation for ordinary file browsing. [Composer's execution model](https://getcomposer.org/doc/faqs/how-to-install-untrusted-packages-safely.md).

Framework commands such as Symfony Console or Artisan are a separate adapter: listing may boot application code and configuration. First ship Composer scripts, then timebox an installed-framework inspection spike when demand warrants it. [Symfony Console](https://symfony.com/doc/current/console.html), [Artisan](https://laravel.com/framework/docs/12.x/artisan).

## Rust: packages, workspaces, aliases, and actual runnable targets

Cargo workspace membership, exclusions, and default members affect what a root invocation selects. Do not make every generated action implicitly use `--workspace`. Distinguish workspace actions from package actions and explain the selected package set. [Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html).

Use passive TOML discovery as the available-without-tools baseline, marking uncertain target inference as partial. Optional explicit metadata enrichment should use versioned JSON, for example `cargo metadata --format-version 1 --no-deps --offline --locked`. `--no-deps` alone is not a network prohibition; offline/locked inspection can fail without the needed cache or lockfile. Keep useful static results. Toolchain shims may also provision before Cargo starts, so native inspection needs an installed-toolchain policy. Cargo package IDs are opaque. [Metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html).

Generate build/check/test actions for selected packages and the workspace, with a visible generated-action label. Offer run actions only for binary targets and runnable examples, including feature requirements and `default-run` semantics. A library-only package has no generic run action; an example can itself be a library. Keep test and benchmark target selectors distinct. [Targets](https://doc.rust-lang.org/cargo/reference/cargo-targets.html), [run](https://doc.rust-lang.org/cargo/commands/cargo-run.html).

Preparation must distinguish Cargo flags from arguments passed after `--` to the binary. The process directory is part of the invocation: using `--manifest-path` alone does not change the program's runtime cwd. Use subprocess cwd rather than Cargo's nightly-only `-C` option. Track `Cargo.toml`, workspace members, lockfile, applicable `.cargo/config[.toml]`, and toolchain files for refresh.

Cargo aliases can be strings, arrays, or shell aliases; configuration is hierarchical and relative to invocation context. A workspace-root invocation does not load each member's local configuration. Discover declared aliases with provenance and delegate expansion to Cargo; global aliases require an explicit scope rather than appearing in every project. [Configuration](https://doc.rust-lang.org/cargo/reference/config.html#alias).

## History: three related features, three different meanings

| Feature | Source of truth | Purpose | What it must not imply |
| --- | --- | --- | --- |
| Log | Existing supervised runs, output, result review state | Check results, clear notices, revisit retained attempts | Reviewing a result is not launching it again. |
| Runui usage history | Successful launch records through TUI/CLI/MCP, with command identity when known | Recent/frequent choices, repeat, save a variation | Pruning output must not erase all command recall. |
| Shell recall | Opt-in, read-only shell/history-tool records | Find commands never launched through runui, insert/edit, promote to favorite | Missing cwd, outcome, or dialect is not known project context. |

Fish's history backend is nearly-valid YAML, not an arbitrary YAML document. Its `paths` records are required argument paths, not the command's cwd. Preserve multiline and escaping rules and tolerate interrupted appends. The shell's own history command offers bounded, NUL-separated search, but choosing a native-shell reader also requires checking startup/configuration effects and metadata loss. [Fish history](https://fishshell.com/docs/current/cmds/history.html), [backend source](https://raw.githubusercontent.com/fish-shell/fish-shell/master/src/history/yaml_backend.rs).

Zsh extended history can contain time and duration, but no cwd. Bash timestamps can delimit multiline entries; without them, reconstructing boundaries can be ambiguous. Plain line splitting is not a portable importer. Preserve unknown fields and formats rather than inventing precision. [Zsh history options](https://zsh.sourceforge.io/Doc/Release/Options.html#History), [Bash history](https://tiswww.case.edu/php/chet/bash/bashref.html#Bash-History-Facilities).

Atuin already captures richer directory/outcome context and provides directory/exit filters. Its `history list` supports NUL-terminated records and format fields. A later adapter should capability-check the installed version and verify a lossless field transport; NUL record separation does not make a tab-separated field format unambiguous. Do not couple the basic feature to Atuin installation, sync, or a private database schema. [Search](https://docs.atuin.sh/main/reference/search/), [list](https://docs.atuin.sh/main/reference/list/).

Recommended behavior: runui recent choices participate in ordinary palette ranking; personal shell recall is a separate opt-in mode. Default shell-entry action inserts into a matching shell for editing, or copies when no integration is present. Show unknown cwd explicitly and require a chosen cwd for supervised replay. Preserve fish/zsh/bash syntax; never translate it silently to the supervisor's current `sh -c` invocation. Optional future capture hooks can record cwd and outcomes for new entries; they cannot reconstruct old context reliably.

Keep imported shell entries local and outside MCP command enumeration by default. Explicitly saving a command promotes it into the existing shared favorites/catalog behavior. Do not persist a second raw history corpus merely to rank results, merge commands across different directories/dialects, or treat hover/review as usage. Render history text safely as text rather than terminal control sequences.

## Other integrations and scope decisions

| Tool or area | Relevant documented behavior | Recommended place |
| --- | --- | --- |
| Task / Make | Task variables and Makefile evaluation can run commands. [Task variables](https://taskfile.dev/docs/guide/variables), [Make](https://www.gnu.org/s/make/manual/html_node/Reading-Makefiles.html) | Fix passive parsing/refresh first. Native listings are explicit enrichment. |
| pnpm | Membership patterns and exclusions define workspace packages. [Settings](https://pnpm.io/settings#packages) | First-release workspace discovery, including deep declared members. |
| mise | JSON task listing includes origin/inheritance; environment activation is separate. [Listing](https://mise.jdx.dev/cli/tasks/ls.html) | Later task adapter; preserve current environment wrapper now. |
| Just | Recipes have parameters, modules, and metadata. [Parameters](https://just.systems/man/en/recipe-parameters.html), [metadata](https://just.systems/man/en/metadata.html) | Prefer versioned native metadata over a partial language parser. |
| Gradle | Qualified task paths and task listings; wrapper-based configuration can evaluate code. [CLI](https://docs.gradle.org/current/userguide/gradle_command_line.html) | Timeboxed native-inspection spike after the requested sources. |
| Python / uv | Console scripts refer to Python objects, not shell commands. [Specification](https://packaging.python.org/en/latest/specifications/pyproject-toml/), [uv](https://docs.astral.sh/uv/concepts/projects/config/) | Environment-aware entry points; named task tools only when present. |
| Turbo / Nx | Native metadata describes graphs and resolved targets. [Turbo](https://turborepo.dev/docs/reference/run#--dry--dry-run), [Nx](https://nx.dev/docs/reference/nx-commands#nx-show) | Enrich existing package rows; delegate caching and scheduling. |
| Deno / editor tasks | Declared tasks and richer input/extension semantics. [Deno](https://docs.deno.com/runtime/reference/cli/task/), [VS Code](https://code.visualstudio.com/docs/debugtest/tasks) | Later adapters with explicit supported subsets. |
| fzf | Searchable candidates, previews, shell/tmux integration. [Documentation](https://github.com/junegunn/fzf#usage) | Borrow interaction patterns and expose a structured picker API. Keep runui's palette independent of fzf. |
| MCP | Schemas, structured tool results, and errors. [Tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) | Same catalog IDs and preparation as TUI/CLI; preserve existing clients. |
| Service groups | Readiness, dependencies, and recovery are lifecycle concerns. [Process Compose](https://f1bonacc1.github.io/process-compose/launcher/), [Compose profiles](https://docs.docker.com/compose/how-tos/profiles/) | Explicit later scope. A group coordinator must survive closing the TUI. |

“Complete” should mean a reliable find → understand → prepare → run → return → review → repeat journey across the requested sources and projects. It should not mean every build tool, an AI dependency, remote execution, a plugin ABI, or a replacement scheduler. Interactive commands need an honest external-terminal handoff in the first release; persistent PTY attachment and durable groups need separate designs.

## Validation status

The seven current-main fixture observations above were rerun with the committed probe. Native Cargo metadata was verified locally; native Composer execution remains unverified because the inspection shell lacks PHP. Personal shell text was inspected only for local aggregate metadata and was not executed. No search-latency benchmark or user-task timing study was performed; the plan's performance numbers are proposed targets.

Run the repository's Rust tests and lint checks for this PR, and convert each changed behavior into a focused Rust regression during implementation. The final check results are recorded in the PR. Go is retired: [PLAN.md](../PLAN.md) remains historical, and neither restoring Go nor running comparisons is a release requirement. Preserve existing Rust favorites, run metadata, output, agent attribution, and supervisor handoff; add versioned state alongside them.
