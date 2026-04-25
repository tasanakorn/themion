# PRD-054: Rename Shared CLI Application Runtime Type to `AppState`

- **Status:** Implemented
- **Version:** v0.34.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-25

## Summary

- Themion's shared CLI bootstrap object was previously named `CliAppRuntime`, but it was not itself the Tokio runtime or executor.
- The shared CLI type is now named `AppState`, and its module is now `app_state.rs`.
- The behavior and ownership split from PRD-051 and PRD-053 remain unchanged; this landed as a naming and clarity cleanup, not an architecture rewrite.
- Runtime-domain terminology such as `RuntimeDomains` and `runtime_domains` remains unchanged where it correctly refers to Tokio runtime ownership.
- The current helper surface remains on or beside the renamed type; this change did not force a broader refactor.

## Goals

- Remove misleading `Runtime` terminology from the shared CLI bootstrap/state type introduced by PRD-051.
- Make the shared CLI object's name match its real responsibility: holding shared application state and handles used by runners.
- Reduce confusion between Tokio runtime-domain terminology and the CLI shared state object.
- Keep the rename narrow and mechanical enough that it does not reopen settled architecture decisions from PRD-050, PRD-051, or PRD-053.
- Improve code readability in `themion-cli` by using the straightforward type name `AppState`.

## Non-goals

- No redesign of runtime-domain ownership.
- No change to the actual process, thread, or Tokio runtime topology.
- No behavioral rewrite of TUI, headless, or non-interactive startup flows.
- No requirement to move helper functions such as Stylos startup or done-mention creation into methods unless the touched code clearly benefits.
- No rename of unrelated `runtime` terminology that still correctly refers to Tokio runtimes, runtime domains, or runtime diagnostics.
- No public API stability promise across crates beyond keeping this repository internally consistent.

## Background & Motivation

### Current state

PRD-051 introduced a shared CLI-local application layer so TUI mode and headless mode could use the same bootstrap object rather than leaving `tui.rs` as the accidental application core. That shared layer now lives in `crates/themion-cli/src/app_state.rs`, and its primary type is `AppState`.

That type holds shared CLI-local state and handles such as:

- `runtime_domains`
- `session`
- `db`
- `project_dir`
- `session_id`
- optional `stylos_config` behind the `stylos` feature

It also provides helper behavior derived from that state, including:

- constructors for TUI and headless bootstrap
- `system_inspection_snapshot()`
- `build_agent()`

The same module also contains related free functions such as project-directory resolution, DB opening, Stylos startup, and done-mention creation helpers.

The original problem was naming, not architecture. In the same crate, `runtime` already had a precise meaning tied to Tokio runtime domains, `RuntimeDomains`, and runtime diagnostics documented in `docs/architecture.md` and `docs/engine-runtime.md`. Calling the shared state object `CliAppRuntime` created avoidable ambiguity because it sounded like the executor/runtime owner rather than a container for shared application state and handles.

### Why `AppState` was the preferred name

Among common names such as `AppContext`, `ApplicationContext`, `Services`, `Environment`, and `Runtime`, `AppState` was the clearest fit for the current object in this repository.

It is a good match because the type is primarily:

- shared state
- long-lived handles
- bootstrap-derived configuration/session data
- a convenient parameter object for TUI, headless, and non-interactive runners

The type does have helper behavior such as `build_agent()` and `system_inspection_snapshot()`, but those remain directly derived from held state and do not make the object an executor or orchestration engine.

**Alternative considered:** rename to `AppContext`. Rejected: valid, but `AppState` is simpler, more Rust-familiar, and a better fit for a type that mostly stores durable handles and session state.

**Alternative considered:** rename to `AppServices`. Rejected: the type does not primarily model a service registry and still reads more naturally as shared state than as a bag of service implementations.

**Alternative considered:** keep `CliAppRuntime` because the type participates in app startup. Rejected: startup participation does not make it the runtime, and keeping the old name would preserve confusion with actual Tokio runtime terminology.

## Design

### Rename the module and type narrowly

Implemented direction:

- renamed `crates/themion-cli/src/app_runtime.rs` to `crates/themion-cli/src/app_state.rs`
- renamed `CliAppRuntime` to `AppState`
- updated imports, module declarations, and local variable names accordingly
- kept the struct's stored fields and current responsibilities unchanged

This landed as a narrow semantic rename, not an unrelated cleanup pass.

**Alternative considered:** keep the module name `app_runtime.rs` and only rename the type. Rejected: the module would continue teaching the same misleading term.

### Keep actual runtime terminology for Tokio-owned concepts

The repository continues using `runtime`, `RuntimeDomains`, and related wording where the code is truly about Tokio runtime construction, ownership, or diagnostics.

Implemented direction:

- preserved `runtime_domains` and other accurate runtime-related names
- preserved `RuntimeDomains::for_tui_mode()` and `RuntimeDomains::for_print_mode()` naming
- did not rewrite runtime-domain docs to avoid the word `runtime`
- used `AppState` specifically for the shared CLI state object to sharpen the distinction between state and executor topology

This preserves the architecture vocabulary established by PRD-050 and clarified by PRD-053.

**Alternative considered:** broadly reduce all `runtime` terminology in touched CLI files. Rejected: that would create churn and blur concepts that are still correctly named.

### Keep the current helper shape unless implementation reveals a better split

`AppState` remains an acceptable home for helper behavior directly derived from its held state.

Implemented direction:

- kept associated constructors such as `for_tui()` and `for_headless()` on `AppState`
- kept `build_agent()` on `AppState`
- kept `system_inspection_snapshot()` on `AppState`
- kept free functions such as `resolve_project_dir()`, `open_history_db()`, `start_stylos()`, `create_done_mention_via_bridge()`, and `create_done_mention_locally()` free-standing

This avoided turning a naming cleanup into a larger refactor while accurately reflecting the existing module shape.

**Alternative considered:** move all helper methods off the type so `AppState` becomes a pure passive struct. Rejected: unnecessary scope expansion for a rename-focused PRD.

### Update docs to reinforce the distinction

Touched docs now explicitly reflect that:

- `AppState` is the shared CLI bootstrap/state object
- Tokio runtime domains remain separate concepts
- PRD-051's architecture remains the same after the rename

Implemented direction:

- updated references in `docs/architecture.md` and `docs/engine-runtime.md` where `CliAppRuntime` or `app_runtime.rs` had been named as the active shared layer
- kept the surrounding architecture explanation intact
- preserved historical PRDs except for this PRD's own landed status

**Alternative considered:** leave docs unchanged because the rename is obvious in code. Rejected: the confusion was partly architectural vocabulary confusion, so the docs needed to be explicit too.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/app_state.rs` | Renamed module from `app_runtime.rs` and renamed `CliAppRuntime` to `AppState` while preserving current fields and responsibilities. |
| `crates/themion-cli/src/main.rs` | Updated module import, type references, and local variable names to use `app_state::AppState`. |
| `crates/themion-cli/src/tui_runner.rs` | Updated type imports and parameter names to use `AppState`. |
| `crates/themion-cli/src/headless_runner.rs` | Updated type imports and parameter names to use `AppState`. |
| `crates/themion-cli/src/tui.rs` | Updated references to helper functions that moved from the `app_runtime` module path to the `app_state` module path. |
| `docs/architecture.md` | Replaced `CliAppRuntime` and `app_runtime.rs` references with `AppState` and `app_state.rs`, while preserving the distinction from Tokio runtime domains. |
| `docs/engine-runtime.md` | Replaced `CliAppRuntime` and `app_runtime.rs` references with `AppState` and `app_state.rs`, while keeping runtime-domain terminology accurate. |
| `docs/README.md` | Updated PRD-054 status to implemented. |

## Edge Cases

- helper methods remain on `AppState` → verify: naming still reads naturally and does not imply `AppState` is an async runtime or executor.
- files still use `runtime_domains` internally → verify: this continues to read clearly as Tokio runtime-domain ownership rather than conflicting with `AppState`.
- Stylos feature is disabled → verify: the rename remains feature-safe and does not introduce references to gated items from always-on code.
- free functions remain in the renamed module → verify: `app_state` still reads naturally as the home for state-adjacent bootstrap helpers and does not force extra refactoring.
- touched docs previously named `CliAppRuntime` and `app_runtime.rs` → verify: the new terminology is consistent in the architecture docs most likely to guide contributors.

## Migration

This is an internal naming cleanup.

Expected migration behavior:

- no user config migration
- no database migration
- no protocol or wire-format migration
- no intended behavior change in TUI, headless, or non-interactive modes

Any downstream breakage is limited to source-level internal references updated as part of the rename.

## Testing

- rename the module and type in `themion-cli` → verify: `cargo check -p themion-cli` passes.
- build `themion-cli` with Stylos enabled after the rename → verify: `cargo check -p themion-cli --features stylos` passes.
- start the non-interactive prompt path after the rename → verify: agent construction still works and output behavior is unchanged.
- start `--headless` mode after the rename → verify: shared CLI bootstrap still initializes and headless lifecycle behavior is unchanged.
- start TUI mode after the rename → verify: terminal startup, event handling, and shutdown behavior are unchanged.
- inspect `docs/architecture.md` and `docs/engine-runtime.md` after the rename → verify: `AppState` refers to the shared CLI object and `runtime` still refers to Tokio runtime concepts.

## Implementation checklist

- [x] rename `crates/themion-cli/src/app_runtime.rs` to `crates/themion-cli/src/app_state.rs`
- [x] rename `CliAppRuntime` to `AppState` across `themion-cli`
- [x] update imports, module declarations, local variable names, and module-path references in `main.rs`, runners, and TUI code
- [x] update `docs/architecture.md` and `docs/engine-runtime.md` terminology
- [x] update `docs/README.md` to reflect implemented status
- [x] run `cargo check -p themion-cli`
- [x] run `cargo check -p themion-cli --features stylos`
