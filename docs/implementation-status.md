# Complete-v1 implementation audit

The goal remains the full [integration plan in PR #11](https://github.com/lechero/runui/pull/11), including Composer, Cargo, workspace/registered-project search, recent usage, shell recall, inputs/presets, and shared TUI/CLI/MCP behavior. A completed slice does not complete the product goal. Update this audit from code, tests, runtime checks and PR/CI state, rather than intent.

| Requirement | Authoritative evidence | State |
| --- | --- | --- |
| Namespace correctness, cycles, complete refresh dependencies, strict scan budgets | `runui-project/src/discovery/tests.rs`; Rust TUI/CLI/MCP tests; PR #12 macOS/Linux CI; isolated real TUI checks | Implemented in PR #12 |
| Shared snapshot/search/prepare catalog; deterministic IDs, fingerprints, provenance, stale selections and worktree isolation | `runui-catalog` tests: golden digest encoding, worktree/alias fixtures, stale prepare, provider variants and duplicate rejection; CLI/MCP ID agreement | Implemented for five existing sources; CI pending |
| Ctrl-P palette across sources; modes, preview, actions, ranking, focus/resize and selection preservation | TUI tests: independent query, stale launch/replay, hidden-output review guard, favorite keys, narrow layout; `scripts/verify-palette.py` private tmux launch/recall/source/terminal/filter checks | Commands/Recent and Project scope implemented; other modes/scopes/facet completion pending |
| Usage recorded once from TUI/CLI/MCP; bounded history and counts independent of Log retention; concurrent/corrupt state recovery | `runui-catalog/src/usage/tests.rs`: concurrent writers, atomic mode 0600, schema/corruption/size, enrichment/dedup, missed launch recovery and stale outcome protection | Implemented; policy configuration/removal controls pending |
| pnpm/npm/Yarn declared membership, exclusions, deep members, scope/context and directory refresh | Workspace fixtures and runtime checks | Pending |
| Project registry/picker, registered-scope search, missing checkouts, duplicate names and bounded background indexing | Registry/catalog/TUI tests and two-repository/worktree checks | Pending |
| Composer root/script/description/alias/hook contracts, literal inputs, diagnostics and execution environment | Disposable Composer/PHP fixtures and missing-tool checks | Pending |
| Cargo roots/workspaces/default members, generated actions/targets/features/runtime cwd | Static fixtures and comparison with offline Cargo metadata | Pending |
| Bounded explicit Cargo metadata and effective aliases; offline/toolchain/config fallback | Native-process deadline/cancellation/size tests and alias fixtures | Pending |
| Opt-in fish recall with unknown cwd, original dialect, safe rendering and insertion; read-only multiline/rotation behavior | Fake history fixtures, shell integration checks and MCP exclusion test | Pending |
| Supported Zsh/Bash adapters and optional Atuin/capture decisions | Format fixtures and documented capability decision | Pending |
| Structured inputs, non-secret local presets, literal argument round trips and stale revalidation | Preparation/preset tests and real supervised invocations | Pending |
| CLI commands/find/projects/inspect/run/logs/stop/restart/wait, JSON schemas/pagination and exit semantics | CLI tests: search/inspect/run/stale, literal args, bounded pages, control-safe text, reject ignored options | commands/find/inspect/run implemented; other commands and picker transport pending |
| MCP catalog IDs/raw compatibility, list filters/pagination, output cursors, agent attribution, private-history exclusion | MCP tests: same IDs/fingerprints as CLI, raw and ID starts, stale/conflicting forms, agent lifecycle | Catalog search pages and ID launch implemented; run pages/filters/cursors pending |
| Shell/editor selection context and dialect-aware insertion; external-terminal handoff for interactive commands | Private tmux: custom launcher receives literal cwd and source/command; native tmux window runs in a format-like path | Terminal/source handoff implemented; picker/editor insertion and shell contexts pending |
| Existing favorites, runs/output, status/review and live supervisors remain readable and controllable | Existing tests plus upgrade/retention fixtures | Foundation verified; recheck after all slices |
| README/help/static HTML, real screenshots/GIFs/videos and documented state/retention agree with shipped behavior | README/HTML and `docs/command-catalog.md` explain current keys, scope, CLI/MCP and sidecar; inspected real wide/narrow palette captures | Incremental text updated; final media refresh pending |
| Performance/cancellation/memory measurement and before/after journey checks | Reproducible measured scale fixtures; full release journey matrix | Pending |
| Complete-v1 macOS/Linux gates and proper reviewable PRs | Final branch state, full tests/lint and CI | Pending |

Follow-on scope remains explicit in the plan: additional tool adapters, persistent PTY attachment and durable service groups require their own designs. The complete-v1 goal is not finished while any required row above is pending, partial, or unverified.
