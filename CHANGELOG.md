# Changelog

Conventional commits, newest release first. Unreleased changes sit on `main`.

## Unreleased

- runui in Rust: the Go runui of the dotfiles (`apps/cmd/runui`) ported whole, the same commands (`runui`, `runui list`, `runui ps`, the supervisor's `_supervise`), flags, usage, interface, keys and files. The project's Taskfiles (with their includes), package.json's scripts (through the package manager its lock file names), Makefiles' targets, compose files' services and poms' builds with their modules' profiles, in its folder and three deep under it; favorites per folder and commands of your own in `~/.config/runui/favorites.json`; runs under a supervisor that outlives runui, in `~/.local/state/runui/runs`, shared with the Go build, which finds, follows and stops this one's runs and the other way round.
- `task compare` checks this build against the Go one: `runui list` byte for byte on edge cases and random mutations of every kind of project file, and both interfaces side by side in tmux, cell by cell.
