# Changelog

Conventional commits, newest release first. Unreleased changes sit on `main`.

## Unreleased

- feat: an **Agents** panel (`5`) lists the agent CLIs runui knows (Claude Code, Codex, GitHub Copilot, Gemini CLI, Qwen Code, Amp), the ones on your PATH first, and whether runui's MCP server is installed for each: ✓ installed, `!` a `runui` entry with another command, ○ not yet. `enter` asks, then installs it for your user with the CLI's own `mcp add` (`runui mcp`, no `-C`, so it serves the folder the agent works in) or removes it with `mcp remove`. The pane shows the command, the CLI's settings file and what the CLI printed. New crate `runui-agents`; agents' settings are only read.

- Refresh the README, illustrated guide and brand kit for Log, Agents, MCP and tmux review state. Regenerate current screenshots, add three real GIF/video tours with playback controls and transcripts, and provide `task docs:capture` for repeatable captures.

- Replace Errors with a global Log (`4`): Unread gathers successful, failed, and lost results counted in tmux; All keeps finished history, including reviewed and stopped runs. Viewing a result clears only its notice, and zero tmux counts stay hidden. Filter by result, folder, command, or agent; rerun from Log to follow the new attempt in Runs.
- Keep previous attempts when a command runs again, within the existing 40 previous ended runs limit. Each attempt retains its output and independent review state.
- Viewing a finished run's output in the TUI now clears its status count, including sidebar previews. Running processes stay counted, and run history and output are kept.
- runui in Rust: the Go runui of the dotfiles (`apps/cmd/runui`) ported whole, the same commands (`runui`, `runui list`, `runui ps`, the supervisor's `_supervise`), flags, usage, interface, keys and files. The project's Taskfiles (with their includes), package.json's scripts (through the package manager its lock file names), Makefiles' targets, compose files' services and poms' builds with their modules' profiles, in its folder and three deep under it; favorites per folder and commands of your own in `~/.config/runui/favorites.json`; runs under a supervisor that outlives runui, in `~/.local/state/runui/runs`, shared with the Go build, which finds, follows and stops this one's runs and the other way round.
- An agent's runs have a tab of their own: `runui mcp` records which client started a run (its MCP name, `agentix`, `claude-code`), and the Runs panel gains an **Agents** tab, after Everywhere, while there are any. `get_run`, `list_runs` and `start_run` answer with `agent` and `output_file`. The Go build reads past the new field and writes none; a run started here is written as before.
- `task compare` checks this build against the Go one: `runui list` byte for byte on edge cases and random mutations of every kind of project file, and both interfaces side by side in tmux, cell by cell.
