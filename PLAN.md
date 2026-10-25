# Porting runui to Rust

`runui` here is a full port of the Go runui in the dotfiles (`apps/cmd/runui` and `apps/internal/runui`: ≈4,200 lines and ≈1,850 lines of tests) to Rust: the same binary, `runui`, with the same commands, flags, output, interface, keys and files on disk. Someone switching from the Go build notices nothing.

The Go source is the specification. The port follows [aemui](https://github.com/lechero/aemui)'s rules for aem-cli: the same behaviour and the same words, byte for byte; Go's libraries ported where runui's words depend on them.

## Layout

| Go package | Crate | Go lines (src / test) |
|---|---|---|
| — | `runui-core` | Go's standard library, as runui's words and files depend on it |
| Bubble Tea, Lip Gloss, Bubbles, x/ansi | `runui-tea` | aemui's `aem-tea` |
| `internal/runui/project` | `runui-project` | 1,005 / 565 |
| `internal/runui/favorites` | `runui-favorites` | 245 / 194 |
| `internal/runui/runs` | `runui-runs` | 581 / 411 |
| `internal/runui/tui` | `runui-tui` | 2,015 / 569 |
| `cmd/runui` | `runui` (the binary) | 352 / 108 |

## Status

Done: every file of each package, every Go test with its Rust test of the same name in snake_case (`TestLoadLooksInAMonoreposFolders` → `load_looks_in_a_monorepos_folders`), with the same cases and assertions, and tests of the ported libraries' own. `cargo test --workspace` runs 213; clippy and rustfmt are clean.

Checked against the Go build (`task compare`, Go 1.27):

- `runui list` byte for byte, stdout, stderr and exit code: 191 hand-written cases (Taskfiles with aliases, merges, `!!binary`, YAML 1.1 booleans, duplicate keys, typed values, broken files; package.json in 68 shapes, broken ones with Go 1.27's words; 40 Makefiles; compose files; 23 poms with namespaces, entities, CDATA and broken XML; monorepos; the flags and the usage errors), then 52,000 files mutated at random from a seed of each kind (10,000 each from seeds 7, 1234, 99999, 2026 and 31337, 2,000 from 4242). No difference, but for the bytes that aren't UTF-8 below (about one file in 280).
- favorites.json and run.json, odd and broken: nulls, fields in another case or given twice, values of the wrong type, times that aren't, runs ended every way. The same, but for the two cases below.
- The interface side by side in a private tmux server at 140×40 and 70×24: the first screen and after each of twenty keys, cell by cell with colours and attributes; the help overlay, the form, the filter, the full-window pane; clicks and the wheel; resizing to 90×24, 60×18, 40×10 and 150×45; `y`'s OSC 52 bytes; SIGTERM ("runui: program was killed: context canceled", the terminal given back). No difference.
- Each build's runs found, followed and stopped by the other; favorites written by one read by the other; `ps` and `list` alike after both.

## How the port is written

**Faithful.** The same words wherever a person reads them: the usage text, errors (`package.json: scripts: unexpected end of JSON input`, `Taskfile.yml: yaml: line 1: did not find expected ',' or ']'`, `pom.xml: XML syntax error on line 1: unexpected EOF`, `fork/exec /no/such/shell: no such file or directory`), the status line, the panes. The same files, in the same formats. No feature left out, none added.

**Go's libraries, where the words depend on them.** package.json is read token by token with Go 1.27's `encoding/json` Decoder, which runs on `encoding/json/jsontext` (`runui-core::golang::jsontext`): its state machine, its literal, string and number consumers, and v1's translation of its errors. YAML is go.yaml.in/yaml/v3 (scanner, parser, resolver, its decoder into the structs runui reads, unknown fields ignored, duplicate keys refused); XML is encoding/xml's tokenizer, decoded as `xml.Unmarshal` decodes the pom's struct. A run's end is said in `syscall.Signal`'s words (`terminated`, `killed`) and its time in `time.Duration`'s (`1m2s`).

**Processes.** The supervisor is `runui _supervise <folder>`, started in a session of its own with the run's lock as fd 3, holding it for as long as it lives; it runs the command in a process group of its own, stdin a pipe kept open, and stops the group with SIGTERM, then SIGKILL after 5 s. Signals reach it through a self-pipe (`sys::notify`), whichever thread they arrive on. In the interface, SIGINT and SIGTERM end the program with Go's "program was killed: context canceled".

**Tests.** A test that starts runs re-runs the test binary as their supervisor, the one test `tests::supervise_helper` doing what Go's TestMain does; the CLI's tests run the built binary in a home of its own.

## What differs

Invisible to someone using it, or out of runui's way:

- Text that isn't UTF-8 (a Latin-1 Makefile's target or comment, a pom's element name in an error) is read with each byte that isn't UTF-8 as U+FFFD, as Go's JSON decoder reads it; the Go build keeps such bytes and prints them as they are. Only a Makefile target whose name isn't UTF-8 runs differently: make gets U+FFFD. A file or folder whose name isn't UTF-8 is left out. A Mac's disk can't hold one.
- Bubble Tea's renderer draws blank cells with tabs and without their foreground; this one writes them as spaces with it. The screens are the same cell for cell.
- favorites.json is read as Go reads it (a field's name in any case, the last of one given twice, nulls, type errors in Go's words with their path), but for a time that starts with a year and still isn't RFC 3339, said in chrono's words, and two folders that are one once cleaned (`/p` and `/p/`): Go keeps one of the two at random each time it starts, this one the last by name.
- With the sidebar too short for every panel's rows, the lines are shared out among the panels in their order; Go goes through a map, in an order that varies, of which this is one.
- Two `esc` pressed at once read as one (crossterm, as in aemui).

Not checked here: Linux. The code has Linux's signal names and system calls and CI tests on Ubuntu, but this Mac had no Linux toolchain to try it with.
