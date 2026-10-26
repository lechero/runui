# Runui documentation & media

Open [index.html](index.html) for the illustrated guide, [brand.html](brand.html) for artwork and downloads, or serve the repository locally:

```sh
python3 -m http.server 8765 --bind 127.0.0.1
# Open http://127.0.0.1:8765/docs/
```

The pages are static HTML and CSS with no JavaScript, external fonts, or build step. Videos have native play/pause controls, still posters, and no autoplay. GIFs are also available for GitHub and direct downloads.

## What the guide covers

- [Four panels](index.html#overview): Favorites, Project, Runs, and Log.
- [Everyday use](index.html#workflow): discover, favorite, run, leave, return, and stop.
- [Command catalog](command-catalog.md): Project/Workspace/Registered search, recent recall, project picker, diagnostics, Composer, CLI IDs and preparation.
- [Log](index.html#log): review successful and failed results, clear notices, and revisit history.
- [Tmux](index.html#status-bars): global counts, hidden zeros, and the shared review state.
- [MCP](index.html#mcp): configuration, all six tools, agent identity, persistence, and tool-call examples.
- [Keyboard](index.html#keyboard), [CLI](index.html#cli), and [stored files](index.html#files).

Native Composer verification uses `scripts/verify-composer.py` with existing PHP and a selected PHAR. CI checks pinned official Composer 2.2.30 and 2.10.3 against disposable scripts, callbacks, helpers, alternate context and supervised CLI/MCP runs. See [the catalog reference](command-catalog.md#verify-this-increment). It installs no project dependencies and never uses personal Composer configuration.

## Regenerate captures

Use Python 3.10+ with Pillow, plus `tmux`, `task`, and `ffmpeg` on your PATH:

```sh
python3 -m pip install Pillow
task docs:capture
```

Or capture a specific build:

```sh
task build
python3 -B scripts/capture-docs.py --binary bin/runui
# Refresh only the Composer tour:
python3 -B scripts/capture-docs.py --binary bin/runui --only composer
```

The default binary is `bin/runui`, the output of `task build`. The script works with temporary HOME/XDG directories, generated project files, and a private tmux socket. It launches real supervisors and an MCP stdio client named `docs-agent`, then stops demo processes and removes the temporary state. It never reads or clears personal runui data and needs no agent account or network connection.

For focused real-TUI checks, run `python3 -B scripts/verify-palette.py` and `python3 -B scripts/verify-registry.py` after building, or set `RUNUI_BIN` to another build. They retain PNG/ANSI captures in a printed temporary folder. The registry check creates two local repositories and two actual Git worktrees and verifies duplicate commands, missing checkouts, exact cwd, project opening and forgetting without deleting run data. It requires Git in addition to the palette check's tools. The final complete-v1 media refresh remains tracked in [the implementation audit](implementation-status.md).

| Output | Shows |
| --- | --- |
| `assets/tour.gif` / `.mp4` | Discover, favorite, run, reopen the same process, filter nested commands, stop, and revisit Log. |
| `assets/composer-tour.gif` / `.mp4`, `composer.png` | Canonical alias search, explicit lifecycle hooks, separate tool availability and bounded native checks without unread results. |
| `assets/log-tour.gif` / `.mp4` | Local and remote failures, a successful result, individual acknowledgement, a running count left intact, and retained history. |
| `assets/agents-tour.gif` / `.mp4` | Real MCP discovery, start, list, read, stop, disconnect/reconnect, and review of agent results. |
| `assets/project.png`, `scripts.png`, `makefile.png`, `compose.png`, `maven.png` | Current command sources and nested-folder discovery. |
| `assets/favorites.png`, `running.png`, `runs.png`, `stopped.png`, `filter.png`, `add-command.png`, `keys.png` | Current four-panel interface and common actions. |
| `assets/log.png`, `log-history.png`, `agents.png`, `agent-log.png` | Log inbox/history and MCP-originated work. |

Terminal pixels are rendered from real tmux captures with the original ANSI colors. Check every new still and representative frames from each GIF/video. Keep captions, poster paths, HTML dimensions, and README descriptions aligned with the recordings. The script asserts key run states and counter changes as it records; a failed assertion stops capture.

## Brand exports

`assets/runui-mark.svg` and `assets/runui-logo.svg` are the source artwork. With Pillow and librsvg (`rsvg-convert`) installed:

```sh
python3 scripts/build-brand-assets.py
```

This regenerates browser/touch/app icons and the repository social card. The [brand kit](brand.html) links to the artwork and current demos. Keep the SVG mark and its proportions consistent.

## Planned discovery improvements

The [research](command-discovery-research.md) and [integration plan](command-discovery-plan.md) are roadmap documents. They distinguish implemented catalog/workspace/registry/Composer work from the remaining Cargo, shell recall, inputs and workflow contracts. The user guide documents current behavior.

[CONTEXT.md](../CONTEXT.md) defines the shared terms. `scripts/probe-discovery.py` records the historical research baseline at `326b0c2`; use the implementation audit and native verification scripts for current behavior.
