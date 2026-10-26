# Command discovery research

Research date: 26 October 2026. Code inspected at `b96cb795370cb299ffe8f221f6d56fde53b640f9` on `main`.

Runui should become the place to find, understand, and repeat a project's commands. The largest immediate gains are a search across sources, reliable workspace coverage, and better handling of command inputs. Its persistent supervisors, saved output, status display, and MCP tools already supply much of the execution foundation.

The confirmed priority is finding the right command faster across projects and monorepos. The recommended sequence is correctness, a shared catalog and palette, workspace discovery and registered-project search, selected additional sources, then richer execution and integrations. The [integration plan](command-discovery-plan.md) defines the changes and release gates.

## How command discovery works today

1. The CLI selects the nearest ancestor containing a supported project file, stopping at the Git repository or home boundary. Starting inside a package often selects that package, rather than the entire workspace.
2. `runui-project::load` reads that folder and eligible descendants three levels below it. The scanner skips hidden and known generated directories and does not follow directory symlinks. It has folder and file limits, but returns no explicit completeness report.
3. Five sources contribute commands: Taskfile, package scripts, Make, Compose, and Maven. Taskfile includes and Maven modules receive special handling. Package managers and Maven wrappers are resolved using parent folders.
4. The TUI creates a tab for each source. `/` filters the current view using case-insensitive words found in names, folders, command lines, or descriptions. Name and folder matches precede description-only matches. This is useful substring search, without a search across every source tab.
5. Favorites retain the command string and relative working directory. Runs retain the command, directory, shell, output, and outcome. Neither is a durable reference to a discovered command definition.
6. Every two seconds, the TUI checks root project files and previously returned files for modification. Refresh does a new scan. Included Taskfiles removed from the returned file list and previously unseen nested files do not participate in that check.
7. CLI `list`, the TUI, and MCP share the project parser, but each assembles its own presentation. MCP exposes exact command strings and directories; starting a run accepts an arbitrary shell string.

Relevant implementation: [discovery and roots](../crates/runui-project/src/lib.rs), [Taskfile includes](../crates/runui-project/src/taskfile.rs), [package manager selection](../crates/runui-project/src/npm.rs), [filtering](../crates/runui-tui/src/rows.rs), [refresh and source tabs](../crates/runui-tui/src/lib.rs), [CLI](../crates/runui/src/cli.rs), [MCP](../crates/runui/src/cli/mcp.rs), [favorites](../crates/runui-favorites/src/lib.rs), and [supervisor](../crates/runui-runs/src/supervise.rs).

## Fit with the existing workflow

The local dotfiles already bind tmux `Alt-X` to a runui popup rooted at the active pane's directory. Neovim `Space o x` opens it through Snacks terminal. The tmux status line polls saved run counts. Shell pickers already use fzf for history, files, commits, and processes.

That makes a fast command palette, a deliberate package/workspace scope switch, and an insert-command action natural additions. Keep the existing popup and editor float as the primary entry points. Offer shell and editor integrations through structured output and command IDs before building a dedicated extension.

These observations come from the local dotfiles' `dot_config/tmux/tmux.conf`, `dot_config/nvim/lua/fuentastic/plugins/snacks.lua`, and shell key configuration. They describe configured entry points; they do not measure how often each is used.

### Local project inventory

A read-only inventory of `~/projects` found the following. It used `rg --files --hidden`, respected ignore rules, and excluded Git data, dependencies, and common generated directories. Top-level directories are not necessarily unique or active repositories; copies, examples, and fixtures can contribute. Counts are prioritization evidence, not usage statistics.

| Manifest family | Files | Top-level directories containing it | Current coverage |
| --- | ---: | ---: | --- |
| `package.json` | 350 | 34 | Scripts supported |
| Taskfile YAML | 85 | 37 | Supported subset and local includes |
| Compose YAML | 193 | 20 | Common operations and per-service up |
| Cargo | 67 | 6 | Unsupported |
| Gradle | 83 | 6 | Unsupported |
| Make | 22 | 4 | Static targets supported |
| Python `pyproject.toml` | 13 | 4 | Unsupported |
| Maven | 13 | 2 | Common goals and profiles |
| mise TOML | 1 | 1 | Runtime environment support; no task discovery |
| Just | 1 | 1 | Unsupported |

There were also 46 pnpm workspace manifests, 29 Turbo configurations, and five Nx configurations. Of 349 readable package manifests, 274 contained scripts, totaling 2,107 script entries. Fifty-nine package manifests lay more than three levels below their top-level project directory. This does not establish that every one contains a command runui should expose, but it makes the fixed depth a poor definition of workspace membership.

Cargo is an especially useful first addition: runui itself is a Cargo workspace, but currently exposes its Taskfile rather than Cargo commands. Gradle deserves investigation early, although its dynamic configuration makes accurate discovery more involved. A single Just or mise manifest is not evidence to prioritize either above workspace improvements.

## Findings reproduced against the current code

Disposable Rust probes called the compiled public `runui-project` interface. The fixtures did not execute project commands.

| Probe | Observed result | Implication |
| --- | --- | --- |
| Cargo-only repository | No discovered commands | A supported language's manifest must also participate in root detection. |
| pnpm member at `packages/a/b/c` | Only the root script appeared | Workspace membership is not read; the depth limit omits declared packages. |
| Same Taskfile included as `one` and `two` | Only `one:build` appeared; no error | A global visited-file set conflates cycle detection with namespace expansion. |
| Edit a hidden included Taskfile | `changed` stayed equal | Source dependencies need explicit refresh tracking. |
| Add a nested `package.json` | `changed` stayed equal | New files need directory tracking or periodic bounded reconciliation. |
| Package under a `.gitignore` entry | Its command appeared | The scanner uses its own exclusions rather than repository ignore rules. |

The installed Task executable confirmed the repeated-include expectation: `task --list-all --json` returned both `one:build` and `two:build` from the same fixture. `cargo metadata --no-deps --format-version 1 --offline --locked` also succeeded on this checkout, reporting seven workspace members and the `runui` binary target.

The last behavior is a discovery policy decision, rather than necessarily a bug: some ignored local command files are useful. It needs an explicit rule with local overrides. The fixed scan limits likewise need diagnostics, rather than a silent appearance of complete coverage.

Code inspection also found that command row identity is based on directory plus rendered shell command. This cannot distinguish every source origin. An included task's displayed source is the entry Taskfile, rather than necessarily its defining file. Both become more visible when adding a global palette and editor source navigation.

Baseline verification passed: 20 discovery tests, 17 TUI tests, eight CLI tests, and one MCP integration test, using `cargo test --locked --offline` for `runui-project`, `runui-tui`, and `runui`. Temporary build output was outside the checkout. Existing tests cover many compatibility cases, but do not cover the reproduced include and refresh gaps.

## What to learn from other tools

The integration choices below are recommendations for runui, distinguished from the documented capabilities they draw on.

| Area | Documented capability | Implication for runui |
| --- | --- | --- |
| Task | JSON listings include task details and locations. Variables may run shell commands during evaluation. [CLI](https://taskfile.dev/docs/reference/cli), [variables](https://taskfile.dev/docs/guide/variables) | Retain static discovery for fast browsing. Offer native metadata explicitly when it is needed for dynamic includes, locations, and inputs. |
| pnpm | Workspace configuration defines member patterns and exclusions, including nested members. [Settings](https://pnpm.io/settings#packages) | Read declared membership before falling back to a bounded folder scan. |
| Cargo | Versioned JSON metadata exposes workspace members and targets; `--no-deps` avoids fetching dependencies. Members, exclusions, and default members have defined semantics. [Metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html), [workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html) | Add workspace and package build/check/test actions; only offer runnable targets where targets support them. Use installed-tool metadata as an optional enrichment path. |
| Gradle | `tasks --all` reports tasks; fully qualified task paths identify subprojects. Wrapper invocation is encouraged. [CLI](https://docs.gradle.org/current/userguide/gradle_command_line.html), [basics](https://docs.gradle.org/current/userguide/command_line_interface_basics.html) | Discover through the installed wrapper in an explicit inspection step. Parsing Groovy or Kotlin with regular expressions cannot reliably reconstruct plugin-defined tasks. |
| mise | `mise tasks ls --json` exposes task metadata, including inherited and global tasks. Config evaluation can have side effects despite a command's read-only label. [Listing](https://mise.jdx.dev/cli/tasks/ls.html), [CLI semantics](https://mise.jdx.dev/cli/) | Label local, inherited, and global origins; show project tasks by default. Existing `mise exec` support does not supply task discovery. |
| Just | Recipes have parameters, defaults, namespaces, and JSON dump metadata. [Parameters](https://just.systems/man/en/recipe-parameters.html), [listing](https://just.systems/man/en/listing-available-recipes.html), [metadata](https://just.systems/man/en/metadata.html) | Prefer its own parser for complete recipe information. Add it after the catalog and input model, rather than introducing another partial language parser. |
| Turbo | Dry JSON output reports tasks, packages, commands, and dependencies. [Run](https://turborepo.dev/docs/reference/run#--dry--dry-run) | Enrich package scripts with graph information; keep Turbo responsible for its cache and scheduling. |
| Nx | `show project --json` reports resolved configuration, including inferred configuration through the show commands. [CLI](https://nx.dev/docs/reference/nx-commands#nx-show) | Native metadata matters for inferred targets. Avoid presenting a root script and its underlying target as unrelated duplicates. |
| Python and uv | `[project.scripts]` values are Python object references. With appropriate package configuration, uv runs their installed entry point names. [Specification](https://packaging.python.org/en/latest/specifications/pyproject-toml/), [uv configuration](https://docs.astral.sh/uv/concepts/projects/config/) | Never execute `module:function` as a shell command. Detect the execution environment and distinguish entry points from development tasks. |
| Deno | `deno.json` and JSONC task declarations map to `deno task`; tasks may carry descriptions. [Task reference](https://docs.deno.com/runtime/reference/cli/task/) | A relatively small later source, but absent from this inventory and lower priority. |
| fzf | A shared searchable list can support previews, shell insertion, and tmux presentation. [Project documentation](https://github.com/junegunn/fzf#usage) | Borrow the interaction pattern. Keep the palette available without requiring fzf; expose structured data for external pickers. |
| VS Code | Tasks support shell/process execution, inputs, dependencies, and background behavior. [Tasks](https://code.visualstudio.com/docs/debugtest/tasks) | Consider a limited import/export later. Variable substitution and extension task types need explicit handling. |
| Process Compose | Dependencies, readiness, recovery policies, and process lifecycle are first-class concerns. [Lifecycle](https://f1bonacc1.github.io/process-compose/launcher/) | Use this as a reference for service groups. Starting several detached processes is insufficient to claim durable workflow support. |
| Compose | Profiles select subsets of services, and explicitly targeting a service has its own activation rules. [Profiles](https://docs.docker.com/compose/how-tos/profiles/) | Add profile-aware actions without replacing Compose dependency or lifecycle semantics. |
| GNU Make | Makefile reading can immediately evaluate shell expressions and assignments. [Reading Makefiles](https://www.gnu.org/s/make/manual/html_node/Reading-Makefiles.html) | Do not treat `make -n` or database output as a guaranteed passive discovery mechanism. |
| MCP | Tool interfaces support schemas and structured output, and distinguish execution errors in tool results. [Tools specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) | Add IDs, validation, and bounded output to the existing tools; keep discovery metadata separate from tool authorization. |

Native metadata improves accuracy, but listing and dry-run commands can evaluate project configuration, load plugins, write caches, or cause tool provisioning. Runui should make ordinary file discovery passive and make native inspection a deliberate, bounded action using installed tools. It should never silently install a missing tool to populate a list.

## Priority decisions

| Priority | Change | Expected benefit | Main complexity |
| --- | --- | --- | --- |
| First | Include correctness, dependency tracking, completeness diagnostics | Commands appear consistently and edits are reflected | Correct namespace expansion and complete dependency invalidation |
| First | Palette across sources with provenance and exact execution preview | Find a command without remembering its owner | Identity, ranking, focus and key compatibility |
| First | Package/workspace scope and pnpm/npm workspace membership | Monorepo coverage matches project structure | Exclusions, nested repositories, refresh, scan budgets |
| First | Project picker and search across registered projects | Find commands across repositories without changing terminal context | Project identity, bounded indexing, stale results, correct execution directory |
| Next | Cargo, then a Gradle inspection spike | Fill gaps demonstrated by the inventory | Target semantics and dynamic configuration |
| Next | Inputs, presets, and launch by ID | Repeat useful variations without editing favorites | Quoting, provider-specific argument rules, stale definitions |
| Next | CLI and MCP parity; shell insertion | Use the same command from a popup, terminal, or agent | Stable schemas and bounded output |
| Later | mise, Just, Python task tools; Turbo/Nx enrichment | Broader accurate discovery | Native-tool versions, inheritance, provider semantics |
| Later | Output search, URL actions, live-run reuse | Less navigation after launch | Ownership and reliable association |
| Separate milestone | Durable groups, readiness, terminal input | Support complete development environments | Persistent coordination and terminal ownership |

The user confirmed discovery across projects and monorepos as the first concern. Project switching and registered-project search therefore belong in the discovery release, ahead of inputs, presets, and workflow orchestration. Validate discovery on a few active repositories before committing to the long tail of sources.

## Scope to defer

Do not mine shell history or README snippets into executable commands automatically. Offer an explicit import or save action later if useful. Do not introduce an AI inference dependency for basic discovery. Do not build a remote execution service, a general plugin ABI, a new build cache, or a replacement dependency scheduler.

Native terminal interaction needs a separate design: the current supervisor redirects output to a file and holds stdin open without forwarding user input. A log pane cannot become a terminal merely by passing more key presses. Likewise, groups need coordination that survives closing runui; a TUI-owned sequence would violate its central behavior.

## Active implementation and stored data

Rust is the only active implementation. The user confirmed that discovery improvements and TUI changes should be developed and validated in Rust; maintaining or comparing against the retired Go build is not required. [PLAN.md](../PLAN.md) remains a historical account of the original port.

Preserve existing Rust favorites, run metadata, logs, and supervisor handoff behavior. Extra catalog metadata can use separate versioned state; changes to existing saved data require deliberate compatibility handling or migration. Validate changes with Rust regression tests and isolated Rust TUI checks.
