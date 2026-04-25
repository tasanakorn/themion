# PRD-054: Rename Shared CLI Application Runtime Type to `AppState`

- **Status:** Proposed
- **Version:** v0.34.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-25

## Summary

- Themion's shared CLI bootstrap object is currently named `CliAppRuntime`, but it is not itself the Tokio runtime or executor.
- Rename the shared CLI type to `AppState` and rename its module from `app_runtime.rs` to `app_state.rs`.
- Keep the current behavior and ownership split from PRD-051 and PRD-053; this is a naming and clarity cleanup, not an architecture rewrite.
- Update nearby docs so `runtime` continues to mean Tokio runtime domains, while `AppState` means the shared CLI state/handles object.
- Avoid broader API reshaping unless a touched function clearly becomes misleading under the new name.

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
- No requirement to split helper methods off the shared state type unless a touched method becomes clearly misleading.
- No rename of unrelated `runtime` terminology that still correctly refers to Tokio runtimes, runtime domains, or runtime diagnostics.
- No public API stability promise across crates beyond keeping this repository internally consistent.

## Background & Motivation

### Current state

PRD-051 introduced a shared CLI-local application layer so TUI mode and headless mode could use the same bootstrap object rather than leaving `tui.rs` as the accidental application core. The resulting type is currently `CliAppRuntime` in `crates/themion-cli/src/app_runtime.rs`.

That type currently holds shared CLI-local state and handles such as:

- runtime domains
- session
- DB handle
- project directory
- session ID
- optional Stylos config

It also provides a small number of convenience methods derived from that state, such as agent construction and local inspection snapshot assembly.

The problem is naming, not architecture. In the same crate, `runtime` already has a precise meaning tied to Tokio runtime domains, `RuntimeDomains`, and runtime diagnostics documented in `docs/architecture.md` and `docs/engine-runtime.md`. Calling the shared state object `CliAppRuntime` creates avoidable ambiguity because it sounds like the executor/runtime owner rather than a bag of shared application state and handles.

### Why `AppState` is the preferred name

Among common names such as `AppContext`, `ApplicationContext`, `Services`, `Environment`, and `Runtime`, `AppState` is the clearest fit for the current object in this repository.

It is a good match because the type is primarily:

- shared state
- long-lived handles
- bootstrap-derived configuration/session data
- a convenient parameter object for TUI, headless, and non-interactive runners

The type does have helper behavior such as `build_agent()` and `system_inspection_snapshot()`, but those remain directly derived from held state and do not make the object an executor or orchestration engine.

**Alternative considered:** rename to `AppContext`. Rejected: valid, but `AppState` is simpler, more Rust-familiar, and preferred for this repository's current shape.

**Alternative considered:** keep `CliAppRuntime` because the type participates in app startup. Rejected: startup participation does not make it the runtime, and keeping the name preserves confusion with actual Tokio runtime terminology.

## Design

### Rename the module and type narrowly

Normative direction:

- rename `crates/themion-cli/src/app_runtime.rs` to `crates/themion-cli/src/app_state.rs`
- rename `CliAppRuntime` to `AppState`
- update imports and local variable names accordingly
- keep the struct's stored fields and current responsibilities unchanged unless a touched name becomes clearly inconsistent

This is intended to be a narrow semantic rename, not an opportunity for unrelated cleanup.

**Alternative considered:** keep the module name `app_runtime.rs` and only rename the type. Rejected: the module would continue teaching the same misleading term.

### Keep actual runtime terminology for Tokio-owned concepts

The repository should continue using `runtime`, `RuntimeDomains`, and related wording where the code is truly about Tokio runtime construction, ownership, or diagnostics.

Normative direction:

- preserve `runtime_domains` and other accurate runtime-related names
- do not rewrite runtime-domain docs to avoid the word `runtime`
- use `AppState` specifically for the shared CLI state object to sharpen the distinction between state and executor topology

This preserves the architecture vocabulary established by PRD-050 and clarified by PRD-053.

**Alternative considered:** broadly reduce all `runtime` terminology in touched CLI files. Rejected: that would create churn and blur concepts that are still correctly named.

### Allow current helper methods to remain on `AppState`

`AppState` is still an acceptable home for helper behavior that is directly derived from its held state.

Normative direction:

- keep associated constructors such as `for_tui()` and `for_headless()` on `AppState`
- keep `build_agent()` on `AppState` unless implementation work shows a clear naming or layering problem
- keep `system_inspection_snapshot()` on `AppState` unless implementation work shows a clearer always-on helper boundary
- keep free functions such as Stylos startup and done-mention helpers free-standing unless they naturally belong elsewhere for touched-code reasons

This avoids turning a naming cleanup into a larger refactor.

**Alternative considered:** move all helper methods off the type so `AppState` becomes a pure passive struct. Rejected: unnecessary scope expansion for a rename-focused PRD.

### Update docs to reinforce the distinction

Touched docs should explicitly reflect that:

- `AppState` is the shared CLI bootstrap/state object
- Tokio runtime domains remain separate concepts
- PRD-051's architecture remains the same after the rename

Normative direction:

- update references in `docs/architecture.md` and `docs/engine-runtime.md` where `CliAppRuntime` is named
- keep the surrounding architecture explanation intact
- avoid rewriting historical PRDs except for status or implementation-note follow-through if implementation later lands

**Alternative considered:** leave docs unchanged because the rename is obvious in code. Rejected: the current confusion is partly architectural vocabulary confusion, so the docs should be explicit too.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-cli/src/app_runtime.rs` | Rename module to `app_state.rs` and rename `CliAppRuntime` to `AppState` while preserving current responsibilities. |
| `crates/themion-cli/src/main.rs` | Update module import and type references to `app_state::AppState`. |
| `crates/themion-cli/src/tui_runner.rs` | Update type imports and parameter names to use `AppState`. |
| `crates/themion-cli/src/headless_runner.rs` | Update type imports and parameter names to use `AppState`. |
| `docs/architecture.md` | Replace `CliAppRuntime` references with `AppState` and preserve the distinction from Tokio runtime domains. |
| `docs/engine-runtime.md` | Replace `CliAppRuntime` references with `AppState` and keep runtime-domain terminology accurate. |
| `docs/README.md` | Add this PRD to the table. |

## Edge Cases

- helper methods remain on `AppState` → verify: naming still reads naturally and does not imply `AppState` is an async runtime/executor.
- files still use `runtime_domains` internally → verify: this continues to read clearly as Tokio runtime-domain ownership rather than conflicting with `AppState`.
- Stylos feature is disabled → verify: the rename remains feature-safe and does not introduce references to gated items from always-on code.
- historical docs or comments still mention `CliAppRuntime` → verify: touched architecture docs are updated so the new terminology is consistent where readers are most likely to encounter it.

## Migration

This is an internal naming cleanup.

Expected migration behavior:

- no user config migration
- no database migration
- no protocol or wire-format migration
- no intended behavior change in TUI, headless, or non-interactive modes

Any downstream breakage is expected to be limited to source-level internal references that should be updated as part of the rename.

## Testing

- rename the module and type in `themion-cli` → verify: `cargo check -p themion-cli` passes.
- build `themion-cli` with Stylos enabled after the rename → verify: `cargo check -p themion-cli --features stylos` passes.
- start the non-interactive prompt path after the rename → verify: agent construction still works and output behavior is unchanged.
- start `--headless` mode after the rename → verify: shared CLI bootstrap still initializes and headless lifecycle behavior is unchanged.
- start TUI mode after the rename → verify: terminal startup, event handling, and shutdown behavior are unchanged.
- inspect `docs/architecture.md` and `docs/engine-runtime.md` after the rename → verify: `AppState` refers to the shared CLI object and `runtime` still refers to Tokio runtime concepts.

## Implementation checklist

- [ ] rename `crates/themion-cli/src/app_runtime.rs` to `crates/themion-cli/src/app_state.rs`
- [ ] rename `CliAppRuntime` to `AppState` across `themion-cli`
- [ ] update imports, module declarations, and local variable names in runners and `main.rs`
- [ ] update `docs/architecture.md` and `docs/engine-runtime.md` terminology
- [ ] update `docs/README.md` with the new PRD entry
- [ ] run `cargo check -p themion-cli`
- [ ] run `cargo check -p themion-cli --features stylos`
