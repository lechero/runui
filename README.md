# Runui

<img src="docs/assets/runui-logo.svg" alt="runui: terminal prompt and cursor" width="292" height="96">

**Find the command. Keep the run. Come back to its output.**

Runui brings your project commands, persistent processes, finished history, and agent runs into one Rust terminal app. Discover tasks and scripts across nested folders, star the ones you use, and follow their output. Close runui whenever you like; its supervisors keep the work going.

[Illustrated guide](docs/index.html) · [MCP setup](#give-your-agent-the-same-tools) · [Log & history](#a-log-you-can-come-back-to) · [Keyboard reference](#keys-at-a-glance) · [Brand kit](docs/brand.html)

![Real runui tour: discover, favorite, run, reopen, filter, stop, and return to Log](docs/assets/tour.gif)

| Find | Run | Review | Share with an agent |
| --- | --- | --- | --- |
| Six command sources, nested folders, filters, and per-project favorites. | Live output under a supervisor that outlives the app and terminal. | A global Log of successes, failures, stopped runs, and repeated attempts. | Six local MCP tools using the same processes, metadata, and output. |

## Install & open

With Homebrew on macOS or Linux:

```sh
brew tap lechero/runui https://github.com/lechero/runui.git
brew install lechero/runui/runui
runui
```

Homebrew builds from source and installs Rust as a build dependency. To build the current source yourself, use Rust 1.88 or newer:

```sh
git clone https://github.com/lechero/runui.git
cd runui
cargo install --path crates/runui --root ~/.local --locked
```

Or run `task install` from the clone. Put `~/.local/bin` on your `PATH`. Discovered commands use the project's own tools: Task, a package manager, Make, Docker Compose, Maven, or PHP/Composer.

```sh
runui                       # open the current project
runui -C ~/projects/atlas    # choose another project
runui list                  # print favorites, commands, and this project's runs
runui ps                    # show runs from every folder, live ones first
```

Opening inside `src/` or another subfolder selects the nearest parent with a supported project file, up to the repository root. Discovery includes folders three levels below the selected project; commands from `apps/web` appear with their folder and run there.

## Four panels, one workspace

| Key | Panel | What's there |
| --- | --- | --- |
| `1` | **Favorites** | Starred project commands and shell commands of your own, saved per project. |
| `2` | **Project** | Taskfile, package.json, Makefile, Compose, Maven, and Composer tabs. Filter a tab with `/`. |
| `3` | **Runs** | **Here**, **Everywhere**, and an **Agents** tab once MCP runs exist. Read, stop, or restart the same processes. |
| `4` | **Log** | **Unread** results needing a look and **All** retained finished attempts, across every folder. |

Use `tab` / `shift+tab` between panels and `[` / `]` between a panel's tabs. Press `?` for the full in-app key sheet. Mouse selection and scrolling work too.

### Commands already in your project

| Source | Commands offered |
| --- | --- |
| `Taskfile.yml` | Tasks and namespaced includes, with descriptions; internal tasks stay hidden. |
| `package.json` | Scripts through pnpm, yarn, bun, or npm, selected by `packageManager` or the nearest lock file. |
| `Makefile` | Targets, with `##` comments as descriptions. |
| Compose files | `up`, `up --build`, `down`, `ps`, `logs -f`, and `up` for individual services. |
| `pom.xml` | Install, verify, test, and non-default profile builds; uses `mvnw` when present. |
| `composer.json` | Native scripts, descriptions, and aliases; arrays, callbacks and `@...` helpers stay with Composer. Lifecycle hooks require `kind:hook` in the palette. |

![Root scripts and apps/web scripts in the Project panel](docs/assets/scripts.png)

Press **`ctrl+p`** to search all sources and favorites together. Try `test`, `source:npm test`, or `dir:apps/web`. The preview shows the exact command, working directory, source location, and tool availability. `/` still filters the current panel tab.

In the palette, `alt+s` switches **Project / Workspace / Registered**, and `tab` switches to **Recent invocations**, which retains exact commands after their output is pruned. `enter` runs the selection or focuses its existing live run; `alt+r` starts another. `alt+u` selects a recent invocation's current definition, keeping repeat and updated defaults separate. `esc` restores your panel filter and selection. Use `ctrl+v` for scrollable details, `f2` for discovery diagnostics, `alt+m` to star, `ctrl+y` to copy, `ctrl+o` to open the source, or `ctrl+t` to open a terminal. Terminal actions use tmux or a configured `RUNUI_TERMINAL` launcher. [Catalog and CLI reference](docs/command-catalog.md).

Press **`alt+p`** in the palette to choose registered projects. `enter` searches one checkout, `ctrl+o` opens it in the main panels, `alt+a` registers a directory, and `alt+d` forgets a registration. Duplicate names are qualified by checkout path and branch. Registrations are explicit; opening a folder does not add it. Forgetting keeps its files, favorites, runs and output.

```sh
runui projects add /path/to/project --name App
runui projects --json
runui find --scope registered test --json
runui run --scope registered --project PROJECT_ID COMMAND_ID --json
```

![Real Composer alias search, explicit hooks, and missing-tool readiness checks](docs/assets/composer-tour.gif)

Composer scripts keep their native behavior and cwd. Aliases search the canonical script; exact lifecycle events stay out of ordinary results (use `kind:hook source:composer`). PHP and Composer availability appear separately. `alt+i` explicitly checks native versions, or use `runui inspect --native --json ID`; opening/searching executes nothing. Checks support Composer 2.2+ and have a five-second/64-KiB limit per tool. Project shell wrappers remain unverified until an offline inspection contract is available. [Composer settings and timeout options](docs/command-catalog.md#composer).

Press `m` in a panel to star a command, or `a` to save your own named shell command. Runui notices edits to files it reads; `r` rescans, including newly added nested files. Cargo, shell recall, and input presets remain in the [integration plan](docs/command-discovery-plan.md).

Declared pnpm/npm/Yarn workspace packages can be deeper than three levels. Workspace scope searches the enclosing declaration while each command keeps its own cwd. Exclusions control package membership; `.runuiignore` uses Git-ignore syntax to exclude discovery paths, including declared members. Repository `.gitignore` files apply to incidental folders; a declared member overrides them with a notice. Nested repositories, symlinks, dependencies and generated folders stay traversal boundaries.

Background checks also cover included Taskfiles, new nested files, workspace declarations/member directories, ignores, parent package-manager settings/locks, and Maven wrappers. Shared Taskfiles contribute every include namespace. Incomplete discovery reports skipped includes, unsupported patterns, unreadable folders, and global scan limits (2,000 folders, 150 source/dependency files). Incidental scanning remains three levels; declared membership is bounded separately by that same global budget.

## Run now, return later

1. Choose a command with `j` / `k`, then press `enter`.
2. Follow its output. Press `q` or close the terminal while work continues.
3. Reopen runui to find the same process and stored output.
4. Press `s` to stop it, or `R` to start a fresh attempt.

Every run uses its own working directory and supervisor. `●` is running, `✓` exited successfully, `✗` failed, `■` was deliberately stopped, and `?` means its supervisor disappeared without a final result.

## A Log you can come back to

Press **`4`**. **Unread** gathers successful, failed, and lost results from **every folder**, including runs started by an agent. A `•` marks an unread result. Selecting a row previews its output and clears that result's completion or error notice in tmux. The output pane does not need focus.

The selected result stays while you read; refreshes do not silently review the next one. If a live run finishes while its output is visible, it is reviewed then. **Running processes keep their counts.**

Switch to **All** with `]` to revisit reviewed and stopped runs, newest finish first. Repeat attempts keep their own output and review state. Filter by command, folder, outcome (`completed`, `failed`, `lost`, `stopped`), or agent; use `R` to follow a new attempt in Runs, `y` to copy its command, or `D` to forget an ended run and its output.

<details>
<summary>Watch: review results, clear notices, keep the history</summary>

![Log tour: view failures and successes one at a time, then revisit their preserved output in All](docs/assets/log-tour.gif)

</details>

[Still screenshot](docs/assets/log.png) · [Play or pause the demo in the guide](docs/index.html#log)

## Give your agent the same tools

Runui includes a local **MCP stdio server**. An agent can discover exact commands and working directories, start persistent runs, follow output, and stop processes. You can inspect those same runs in **Runs → Agents**, identified by the MCP client's name, and review their finished results in Log.

Configure your client's MCP server settings with the absolute binary and project paths. For clients using `mcpServers`:

```json
{
  "mcpServers": {
    "runui": {
      "command": "/Users/you/.local/bin/runui",
      "args": ["mcp", "-C", "/Users/you/projects/atlas"]
    }
  }
}
```

Use your installed paths (`command -v runui`), including `/home/you/…` on Linux. Client configuration shapes vary; the launch command is always `runui mcp -C /absolute/project`. There is no network listener or API key to configure.

| Tool | Inputs | Result |
| --- | --- | --- |
| `list_commands` | Optional `query`, `scope`, `limit`, `offset` | A catalog page (200 by default, up to 500), with IDs, fingerprints, provenance, exact commands/directories, diagnostics, and `next_offset`. Scope is `project`, `workspace`, or `registered`. |
| `start_run` | Either `id` with optional `fingerprint`, `inputs`; or raw `command` with optional `directory`. Registered ID launches require `scope: registered` and `project` from the entry's `registry_id`. Optional `name` in either form. | A supervised run and its ID. Catalog launches revalidate only the selected checkout; literal `inputs.args` and supported `inputs.variables` are prepared before launch. |
| `list_runs` | Optional `directory` | Runs from every folder, newest first; the optional filter matches one exact working directory. |
| `get_run` | `id` | State, timestamps, exit code, agent, output path, and other metadata. |
| `read_output` | `id`; optional `lines` | Latest plain-text tail and state: 100 lines by default, up to 500 lines and 64 KiB. Poll again to follow output. |
| `stop_run` | `id` | A stop request; poll `get_run` for the final state. |

![An MCP-started run in the Agents tab, with its client identity and live output](docs/assets/agents.png)

<details>
<summary>Watch: MCP starts work, the TUI follows it, and Log keeps the result</summary>

![MCP tour: start a run, disconnect, read and stop it, then review agent results in Log](docs/assets/agents-tour.gif)

</details>

The client name comes from MCP's `initialize.clientInfo.name`. Run metadata includes `agent` and `output_file`; the latter points to the complete raw output file. Closing runui or disconnecting the MCP client leaves its runs alive. TUI-started runs are available through the same tools.

MCP reads do not clear completion or error notices: view the result in the TUI or use `runui status clear`. The [guide](docs/index.html#mcp) includes tool-call examples and the discover → start → follow → stop workflow.

## Quiet tmux counters

```tmux
set -g status-interval 5
set -ag status-right '#(runui status tmux)'
```

`▶` counts active runs in blue, `✓` unread successes in green, and `✗` unread failures or lost runs in red. Each zero segment disappears; when all are zero, the whole segment is empty. Deliberately stopped runs stay out. Log and tmux cover the same global run store.

```sh
runui status        # running=2 completed=1 errored=0
runui status tmux   # colored tmux segment with zeros hidden
runui status clear  # clear finished notices, keeping processes and output
```

If your shell and tmux disagree, use the absolute path from `command -v runui` in `#(...)`, then reload tmux. Its server may have a different `PATH`. After viewing a result, allow the configured polling interval for the status line to update.

## Keys at a glance

| Keys | Action |
| --- | --- |
| `1`–`4`, `tab` / `shift+tab` | Favorites, Project, Runs, Log. |
| `[` / `]` | Switch the current panel's tabs. |
| `j` / `k`, `g` / `G` | Move through rows. |
| `/`, `esc` | Filter the current tab; clear the filter. |
| `ctrl+p` | Open the palette; `tab` switches Commands / Recent, `alt+s` switches scope, `alt+p` opens projects. |
| `ctrl+v`, `f2` in the palette | Scrollable command details or discovery diagnostics. |
| `enter` | Run a command; on a run, focus its output. |
| `s` / `R` | Stop or restart a run. |
| `m` / `a` / `e` / `D` | Star, add, edit, or remove; `D` also forgets an ended run. |
| `y`, `r` | Copy the command; reload project files. |
| `0`, `esc`, `+` / `_` | Focus output, return to the panel, expand / restore output. |
| `pgup` / `pgdn`, `J` / `K` | Scroll output from the panel. |
| `?`, `q` | Show all keys; quit while runs continue. |

## Your files & history

| Data | Default location | Override |
| --- | --- | --- |
| Favorites | `~/.config/runui/favorites.json` | `$XDG_CONFIG_HOME/runui/favorites.json` |
| Registered projects | `~/.config/runui/projects-v1.json` | Beside the selected favorites file. |
| Runs and output | `~/.local/state/runui/runs` | `$XDG_STATE_HOME/runui/runs` |
| Usage recall | `~/.local/state/runui/runs/catalog-v1.json` | Under the selected runs directory. |

Each run has `run.json`, `output.log`, a supervisor lock, and a `status.seen` marker once its result has been reviewed. Starting a new run keeps the 40 most recent previous ended runs across folders, including repeated commands. Output remains until the run is forgotten or pruned.

Usage recall is separate: up to 1,000 detailed invocations for 180 days and 2,000 aggregate keys. Launches are deduplicated by run ID across TUI, CLI, and MCP. Reviewing results does not increase usage or acknowledge hidden results while the palette is open. The sidecar uses an atomic write, a separate writer lock, and file mode `0600`; malformed state is backed up before repair, and unknown schema versions are left intact. Personal shell history is never read by ordinary search.

Commands run through `sh -c`, using `mise exec` when available. Stopping sends SIGTERM to the process group, then SIGKILL after five seconds if necessary.

## Develop & refresh the demos

```sh
task build          # build bin/runui
task run            # build and open the current folder
task test           # workspace tests and clippy
task lint           # clippy and rustfmt checks
task docs:capture   # rebuild screenshots, 4 GIFs, and 4 playable videos
```

Captures need Python 3.10+, Pillow, tmux, Task, and ffmpeg. They exercise the real Rust app and all six MCP tools in temporary projects and private tmux sessions. Your favorites and run history are untouched. [Media maintenance](docs/README.md) covers regeneration and local preview.

CI runs tests, formatting, and clippy on macOS and Linux. A `v*` tag publishes release archives. See [AGENTS.md](AGENTS.md) for the code layout and [the discovery roadmap](docs/command-discovery-plan.md) for planned work. License: [MIT](LICENSE).
