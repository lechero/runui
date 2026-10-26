# Private shell recall

Press `ctrl+p`, then `tab` twice for **Shell history**. It starts disabled every time the palette opens. `alt+b` chooses fish, Zsh or Bash; the preview identifies the selected file before reading it. `alt+h` enables that source for this session. Press it again to cancel reads and discard the ephemeral records. Commands/Recent and MCP never include these records. `ctrl+r` refreshes; enabled, visible recall also notices append/rotation through file metadata.

Search matches literal command text, including colons. It does not parse catalog facets or evaluate shell expressions. `ctrl+v` opens scrollable details and `f2` shows format, byte-limit and interrupted-write diagnostics. The list deduplicates exact text within the original dialect. File record counts are not execution frequency.

Native history does not establish cwd or exit status. Entries therefore stay in **All history (cwd unknown)**, regardless of the current Project/Workspace/Registered scope. Fish's `paths` metadata describes required argument paths, not the command's working directory. Timestamp/duration appear only when the source supplies them; unsupported metadata field names remain inspectable.

## Select, replay or save

In the regular TUI, **Enter / Ctrl+Y copies** the exact original text. Selection does not execute it. For editable insertion into the matching interactive shell, use the picker integrations below.

`alt+r` opens **Replay under supervision**; `alt+m` opens **Save as a shared favorite**; `ctrl+t` opens **Run in an external terminal**. Each shows the original dialect and a directory field. Confirm the chosen absolute cwd explicitly; this choice does not recover the original cwd. Runui re-reads the bounded source and requires the same record and an installed matching shell before carrying out the action. A removed/stale record or unavailable shell leaves an explanation without a run. Shell startup, functions, aliases and environment can differ from the original session.

Supervised replay stores the original command with its native shell (`fish -c`, `zsh -c` or `bash -c`). It appears in Runs, Log and normal runui usage; it uses the existing supervisor, output and review behavior. Saving is deliberate promotion into shared Favorites/MCP. Because raw favorites use the existing sh contract, a promoted favorite contains an explicitly quoted native-shell invocation around the original text. No favorite or history format is rewritten.

## CLI and editable shell insertion

```sh
runui find --mode shell --shell fish --json 'cargo test'
runui find --mode shell --shell zsh --history-file /path/to/zsh_history --json
runui find --mode shell --shell bash --pick --json
```

`--shell` is required for this separate local mode. JSON pages have `schema_version: 1`, `private: true`, `dialect`, `history`, `diagnostics`, `bytes_read` and `next_offset`. Pages default to 200 rows, with `--limit 1..500` and `--offset`. Shell mode rejects `-C` and catalog/Cargo scope options because native history has no known cwd. Ordinary `commands`, `find --mode all`, `inspect`, `run` and MCP do not resolve private recall IDs.

`--pick` draws only on terminal stderr and returns a selection on stdout. Its JSON envelope contains `schema_version: 1`, `action: insert`, `insertion_dialect` and the selected record, including its unknown directory. `--print0` instead returns the exact UTF-8 command followed by one NUL. The picker never starts a supervisor or writes favorites, history or usage. Esc/Ctrl-C return **130** and no payload; usage errors return 2, and other CLI failures return 1. Do not `eval` its result.

Source the appropriate repository snippet, then bind your own key if desired. No snippet edits dotfiles or rebinds Ctrl-R/fzf automatically:

```fish
source /path/to/runui/shell/runui.fish
bind \cx\ch runui-history
```

```zsh
source /path/to/runui/shell/runui.zsh
bindkey '^X^H' runui-history-widget
```

```bash
# Bash 4+ / Readline
source /path/to/runui/shell/runui.bash
bind -x '"\C-x\C-h":_runui_history_widget'
```

The snippets insert into fish's `commandline`, Zsh's `BUFFER` or Bash's `READLINE_LINE` for editing. They preserve multiline text, Unicode, shell substitutions as literal text and trailing newlines. Fish uses NUL-separated substitution; Zsh/Bash run the UI in the foreground and transfer its result through a mode-0600 file in a private temporary directory, removed afterward. This keeps terminal ownership with the picker. Cancellation leaves the original buffer unchanged. Existing hooks are left intact.

## Sources and limits

| Source | Supported format and context |
| --- | --- |
| fish | Native YAML-like `- cmd:` records; only native backslash/newline escapes. Uses `$XDG_DATA_HOME/fish/<session>_history`, normally `~/.local/share/fish/fish_history`. Exported `fish_history` names a session; empty disables reading, and `default` selects `fish`. The function passes its actual session/private setting explicitly. |
| Zsh | Plain and `EXTENDED_HISTORY` records, native metafied bytes, escaped multiline continuations and terminal-backslash padding. Extended timestamps and duration remain metadata; neither supplies cwd/outcome. |
| Bash | Timestamp-delimited records (`#SECONDS` followed by command text), including multiline text. Untimestamped/mixed ambiguous text is diagnosed and skipped; line splitting cannot reconstruct original multiline boundaries. `HISTTIMEFORMAT` controls native timestamp storage, but runui does not change it. |

Zsh/Bash default to `~/.zsh_history` / `~/.bash_history`. Exported `HISTFILE` applies when `$SHELL` identifies that dialect; an empty value disables reading. The widgets pass the current shell's actual `HISTFILE` explicitly and honor an unset/empty value. A process cannot observe nonexported parent-shell variables; use its integration or an explicit `--history-file` for nondefault settings. No shell startup file or universal-variable database is read to discover them.

Each read opens a regular file read-only, with nonblocking open to reject FIFOs/devices. It reads at most the last **4 MiB**, skips partial boundary records, considers the latest **10,000 source records** before deduplication, limits commands to **64 KiB**, and keeps at most **2,000 distinct entries**. Clipping is diagnosed; repeated malformed-record diagnostics are aggregated. Reads/parsing are cancellable; a single filesystem operation remains subject to the OS. Invalid UTF-8/NUL-containing text, unsupported escapes, oversized records and interrupted appends are skipped with diagnostics instead of being repaired into a different command. Replacement/rotation is detected by inode/device, size and modification time. No persistent imported corpus or cache is written.

Atuin and new shell capture hooks remain optional follow-ons. They are not dependencies and runui does not inspect their private databases. The initial capability check found no installed Atuin to validate a lossless versioned transport. Capturing cwd/outcomes for new commands needs a separate opt-in hook design; it cannot reconstruct old records from interleaved `cd` commands.

Format references: [fish history](https://fishshell.com/docs/current/cmds/history.html), [fish's native backend](https://raw.githubusercontent.com/fish-shell/fish-shell/master/src/history/yaml_backend.rs), [Zsh history options](https://zsh.sourceforge.io/Doc/Release/Options.html#History), [Bash history facilities](https://www.gnu.org/s/bash/manual/html_node/Bash-History-Facilities.html), [fish commandline insertion](https://fishshell.com/docs/current/cmds/commandline.html).

## Verification

`task test` and `task lint` cover parser/cancellation/rotation/limits and CLI/TUI opt-in/no-launch contracts. `RUNUI_BIN=bin/runui python3 -B scripts/verify-recall.py` uses disposable HOME/XDG/history files and private tmux sessions. It compares native formats, verifies MCP exclusion, exact stdout transport/cancellation, real fish/Zsh/Bash editable-buffer insertion and explicit fish replay through the existing supervisor. It requires installed fish, Zsh, Bash 4+, tmux and Python; it installs nothing and reads no personal history. Set `RUNUI_RECALL_CAPTURES=/tmp/captures` to retain real PNG/ANSI captures (Pillow required).
