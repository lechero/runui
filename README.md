# runui, in Rust

A folder's commands at hand, run from one place, and kept running: [runui](https://github.com/lechero/dotfiles/tree/main/apps/cmd/runui) ported from Go to Rust, the same binary, `runui`, with the same commands, interface, keys and files. It's a terminal interface laid out as [aem-cli](https://github.com/lechero/aem-cli)'s and lazygit's are: panels stacked on the left, the output on the right.

The Go build in the dotfiles (`apps/cmd/runui`, `apps/internal/runui`) is the specification; [PLAN.md](PLAN.md) says how it's ported and how the two were checked against each other. Both builds read and write the same files, so either finds, follows and stops the other's runs.

```bash
runui                  # the project you're in: its commands, your favorites, what's running
runui -C ~/projects/x  # another folder
runui list             # the same, printed; changes nothing
runui ps               # every run, everywhere, the ones going first
```

## Install

With Rust 1.88 or newer, from a clone, with [Task](https://taskfile.dev):

```bash
task install     # puts runui in ~/.local/bin
```

or without Task:

```bash
cargo install --path crates/runui --root ~/.local --locked
```

In the dotfiles, tmux's `alt-x` opens runui in a popup over the pane you're in and Neovim's `Space o x` in a float; both run whichever `runui` is first on the PATH, so this one takes over once it's in `~/.local/bin` ahead of the Go build. What it starts keeps running when the popup or the float goes.

Opened in a project's `src` folder, it works on the project: the nearest folder above with a `Taskfile.yml`, a `package.json`, a `Makefile`, a compose file or a `pom.xml`, up to the repository's root.

## What it shows

```text
╭─[1]─Favorites──────────────╮╭─dev › output──────────────────────────────────────╮
│● ★ dev  pnpm run dev       ││dev ★                                              │
│✓ $ greet  echo hello; date ││$ pnpm run dev                                     │
╰─────────────────────1 of 2─╯│from package.json · in ~/projects/georgi           │
╭─[2]─Taskfile - package.json╮│vite --host 127.0.0.1                              │
│● ★ dev  vite --host 127.0.…││                                                   │
│    build  vite build       ││● running for 3s · pid 73971 · s stops it, R runs …│
│    lint  eslint .          ││───────────────────────────────────────────────────│
╰────────────────────1 of 11─╯│  VITE v7.3.6  ready in 483 ms                     │
╭─[3]─Here - Everywhere──────╮│                                                   │
│● dev  3s                   ││  ➜  Local:   http://127.0.0.1:5173/               │
╰─────────1 running · 1 of 1─╯╰───────────────────────────────────────────────────╯
✓ started pnpm run dev
enter run • m favorite (★) • s stop • R run again • a add your own • / filter • ? all keys • q quit
```

1. **Favorites**: this folder's saved commands. `★` is one of the project's, starred with `m`; `$` is a command of your own, added with `a`.
2. **The project's commands**, a tab for each kind of file:
   - **Taskfile**: its tasks, with the ones its includes bring, named as Task names them (`docs:serve`), internal ones left out;
   - **package.json**: its scripts, run through pnpm, yarn, bun or npm, as its `packageManager` field or the nearest lock file says (a workspace's is at its root);
   - **Makefile**: its targets, with their `##` comments;
   - **Compose**: `docker compose up` (the logs follow in the pane, `s` stops every service), `up --build`, `down`, `ps`, `logs -f`, and each service up on its own with its image or what it's built from; a compose file for a purpose (`docker-compose.dev.yml`) gets its own rows, with `-f`;
   - **Maven**: `clean install`, `clean verify`, `test`, and `clean install -P…` for each profile that isn't on by default, its modules' too (the AEM archetype's `autoInstallPackage`, `autoInstallBundle`… are in `ui.apps`, `ui.content` and `core`), through `mvnw` where the project has one.

   The folders under the project's are looked in too, three deep, so a monorepo's `apps/web` and `packages/ui` and a Maven project's modules are there, each row after its folder (`apps/web › dev`), and it runs in that folder. Hidden folders and what tools make or download (`node_modules`, `vendor`, `dist`, `build`, `target`, …) are left out, and so is a Taskfile another one includes. Outside a repository, a folder that isn't a project (`~/projects`) isn't looked under. An edit to the files shows by itself; a new file in a folder under it shows with `r`. `/` filters by name, folder, command or description, the rows whose name has it first.
3. **Runs**: what runui started here, and on the other tab everywhere. `●` running, `✓` exited 0, `✗` failed, `■` stopped, `?` lost track of it.

On the right, the row under the cursor: what it runs, where it's from, how its last run went, and that run's output, followed as it's printed.

It opens on the favorites when the folder has some, the cursor on one that's running, so its output is there at once; else on the Runs panel when something's going here; else on the project's commands.

## Keys

The frame's are lazygit's, as aem-cli's are: `tab`/`l` and `shift+tab`/`h` go from panel to panel, `1`-`3` to one, `[` and `]` along its tabs, `j` and `k` along its rows, `0` (or `enter` on a run) gives the output the keys, `esc` goes back, `pgup`/`pgdn` and `J`/`K` scroll the output from a panel, `+` gives it the whole window and `_` takes it back, `/` filters. The mouse picks rows and scrolls.

| Key | Does |
| --- | --- |
| `enter` | Run it, in its folder; the output shows as it's printed |
| `s` | Stop it: its process group gets SIGTERM, SIGKILL 5 s later if it's still there |
| `R` | Run it again, stopping it first |
| `m` | Keep it in this folder's favorites, or take it out |
| `a` | Add a command of your own to the favorites |
| `e` | Edit a favorite: its name and its command |
| `D` | Take a favorite out (asking, for one of your own), or forget a run that has ended |
| `<` `>` | Move a favorite up or down |
| `y` | Copy the command (through the terminal, OSC 52) |
| `r` | Read the project's files again |
| `?` | Every key |
| `q` | Quit; what's running keeps running |

## Runs outlive it

runui doesn't run a command itself: it starts a supervisor, `runui _supervise`, in a session of its own, so quitting runui, closing the terminal or the tmux popup leaves the command running. The supervisor runs it with `sh -c` in its folder, through `mise exec` where there's mise (so it gets the Node the folder pins, even from a popup, which doesn't start a shell with mise active), in a process group of its own, with its output in a file and its stdin a pipe kept open (some dev servers stop when stdin closes). It writes down when the command ends and how.

Each run is a folder in `~/.local/state/runui/runs` (`$XDG_STATE_HOME`): `run.json`, `output.log`, and a `lock` the supervisor holds for as long as the command runs. The next runui knows a run is still going because the lock is held; the system lets go of it when the supervisor ends, however it ends, so a restart can't leave a run that looks alive. Stopping asks the supervisor, which sends the command's process group SIGTERM, then SIGKILL. Starting a command again forgets its runs here that have ended; 40 ended runs are kept in all.

The favorites are `~/.config/runui/favorites.json` (`$XDG_CONFIG_HOME`), a list per folder, each with the folder under it that it runs in. Every change reads the file again first, so two runuis open at once don't undo each other's.

## Develop

```bash
task test      # every crate's tests, and clippy with warnings as errors
task lint      # clippy and cargo fmt --check
task run       # build and open it on the folder you're in
task compare   # this build against the Go one in ~/dotfiles: runui list on edge cases and random
               # mutations of every kind of project file, byte for byte, then both interfaces in tmux,
               # cell by cell, and a run started by one stopped by the other
```

`task compare` needs Go, tmux and Python 3; `GO_APPS=…` points it at another checkout of the dotfiles' `apps`, and `task compare -- SEED N` picks the random mutations' seed and how many of each kind.

The workspace is a crate per Go package, under `crates/`: `runui` (the binary, `cmd/runui`), `runui-tui`, `runui-project`, `runui-favorites`, `runui-runs`; `runui-core` for what Go's standard library does that runui's words and files depend on; and `runui-tea`, the terminal runtime, styles and widgets, the parts of Bubble Tea, Lip Gloss and Bubbles the interface uses, taken from [aemui](https://github.com/lechero/aemui)'s `aem-tea`. [AGENTS.md](AGENTS.md) has the rest.
