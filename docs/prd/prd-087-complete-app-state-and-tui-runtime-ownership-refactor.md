# PRD-087: Complete App-State Ownership of TUI Runtime Coordination

- **Status:** Implemented
- **Version:** v0.56.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (retrospective PRD authoring)
- **Date:** 2026-05-02

## Implementation status

Landed in `v0.56.0` as a retrospective follow-through on the already-completed refactor between commit `5fa0bf2b8d97f9728a7785a701fb71262673f08b` and current `HEAD`. The shipped work finishes the next ownership-cleanup slice after PRD-084 by moving the remaining TUI-owned runtime mutation and startup/shutdown coordination into `app_state.rs`, `app_runtime.rs`, and `tui_runner.rs`, while leaving `tui.rs` as a terminal surface that observes runtime state, forwards human input, and renders transcript/status output.

## Summary

- The repository architecture already required the TUI to be a human input/output surface, but the code still left important runtime mutation and startup/shutdown coordination paths inside `tui.rs`.
- This landed refactor moved those remaining responsibilities into `app_state.rs`, `app_runtime.rs`, and `tui_runner.rs` so the TUI no longer owns live runtime decisions or mutation sequencing.
- The finished code keeps transcript rendering, event translation, debug display, and view-local state in `tui.rs` while moving runtime launch, snapshot publication, agent-turn mutation, Stylos startup wiring, and shutdown coordination out of the TUI surface.
- User-visible behavior stayed mostly the same; the main outcome is cleaner ownership, better headless/TUI alignment, and fewer reasons for new orchestration code to drift back into `tui.rs`.
- This PRD is historical tracking for work that is already done, not a new implementation request.

## Goals

- Keep `crates/themion-cli/src/tui.rs` focused on terminal input, rendering, transcript formatting, and UI-local state.
- Move remaining runtime mutation and orchestration sequencing out of the TUI surface and into runtime-owned CLI modules.
- Make TUI mode, headless mode, Stylos status publication, and local multi-agent runtime management consume the same app-state/runtime-owned truth more consistently.
- Reduce the chance that future agent scheduling, startup, shutdown, or snapshot-publication logic re-accumulates inside `tui.rs`.
- Preserve current user-facing behavior while tightening ownership boundaries.

## Non-goals

- No redesign of the TUI layout, transcript format, or chat composer behavior beyond what the refactor incidentally touched.
- No migration of CLI-local orchestration into `themion-core` when the behavior is still process-local.
- No protocol redesign for Stylos, board notes, or workflow semantics.
- No claim that every read-only runtime field access in `tui.rs` must move immediately into snapshots.
- No attempt to rewrite unrelated `v0.56.0` work such as multiple Codex profile support into this PRD.

## Background & Motivation

### Current state before this refactor slice

PRD-084 already moved several non-UI responsibilities out of the TUI, but the code still left a meaningful second layer of ownership leakage:

- TUI-owned helpers still prepared and mutated live agent-turn runtime state.
- TUI-side code still owned runtime snapshot publication hooks and related mutation sequencing.
- Stylos startup and shutdown wiring for TUI mode was not yet fully concentrated in app-state or runner-owned paths.
- Some runtime-shaped helper behavior still lived in `tui.rs` because of historical placement rather than true UI ownership.

That left the repository in an awkward middle state: the architecture docs said the TUI should only observe or project runtime truth, but several important runtime mutations still happened through the TUI layer.

### Why this became a distinct follow-through slice

The audited commits after `5fa0bf2b8d97f9728a7785a701fb71262673f08b` show a coherent second-phase cleanup:

- `91586ba` and `5ef461a` moved more runtime ownership away from the TUI.
- `5eb891c` and `223e5ed` pushed watchdog and related prompt-handling behavior further out of the surface layer.
- `0f94dba`, `b391275`, `7e15ced`, and `d01d66d` completed the app-state/runtime ownership cleanup by moving remaining mutations, startup/shutdown coordination, and final small ownership leaks out of `tui.rs`.

This was not just one bug fix. It was the completion of a meaningful architecture slice that deserved historical tracking, especially because the code changes touched docs, runtime modules, and validation behavior together.

**Alternative considered:** rely on commit history and checklists alone as the historical record. Rejected: the repository already uses PRDs as durable implementation contracts and history, so this refactor should also have a concise product/architecture-shaped record.

## Design

### 1. `tui.rs` should render and forward intent, not own runtime mutation sequencing

The landed code keeps `tui.rs` responsible for:

- transcript and status rendering
- terminal event translation
- local view state such as scroll/review mode and composer state
- emitting intents and displaying runtime results

The landed code removes or relocates TUI ownership of:

- live agent-turn preparation and mutation sequencing
- runtime snapshot publication triggers that belong to app-state/runtime ownership
- Stylos startup/shutdown coordination paths that are runner- or app-state-level concerns
- remaining helper APIs that exposed mutable runtime-owned agent access from the TUI surface

This preserves the TUI as a surface instead of a mixed controller/runtime owner.

### 2. App-state and app-runtime should own mutable runtime truth for TUI mode

The landed refactor expanded runtime-owned helpers in `app_runtime.rs` and `app_state.rs` so that TUI mode can reuse non-visual runtime logic without `tui.rs` performing the mutation itself.

The shipped responsibilities now include:

- preparing and launching agent turns through runtime-owned helper flows
- publishing or projecting app snapshots through app-state-owned paths
- surfacing runtime-oriented context-report helpers outside the TUI command handler
- concentrating Stylos startup wiring in app-state/runtime helpers rather than ad hoc TUI glue

This keeps the canonical mutable runtime behavior closer to the modules that already own process-local sessions, agent handles, and runtime domains.

**Alternative considered:** leave some of these helpers in `tui.rs` because they are only used by TUI mode today. Rejected: current sole usage was not a sufficient ownership argument when the behavior is clearly runtime-shaped rather than view-shaped.

### 3. `tui_runner.rs` should own terminal-mode orchestration around the surface

The landed refactor also clarified that `tui_runner.rs`, not `tui.rs`, should own terminal-mode orchestration such as:

- terminal setup and cleanup
- startup of snapshot-watch and tick loops
- TUI-mode service startup sequencing
- shutdown sequencing for terminal input and Stylos runtime teardown

This keeps orchestration surrounding the surface in the runner layer rather than in the render/input file itself.

### 4. Read-only rendering access may remain in the TUI when it is truly presentation work

The completed cleanup does not require every runtime field access to disappear from `tui.rs`.

What remains acceptable in the landed code:

- drawing status bars from runtime/session/workflow data
- rendering transcript lines from runtime-owned entries and agent display metadata
- debug/status reporting that observes runtime state without becoming the owner of decisions

This distinction matters because the goal of the refactor was ownership correctness, not to force all display data through a giant abstraction layer prematurely.

### 5. Final cleanup items should remove residual mutation hooks and direct live-agent reads where practical

The final audited commits in this range also cleaned up smaller remaining leaks:

- dead mutable agent accessor helpers were removed from `tui.rs`
- `/context` no longer walks live interactive-agent state directly from the TUI command handler, and instead goes through an `app_state.rs` helper path
- default-build and Stylos-feature test paths were corrected so the new ownership split builds consistently in both configurations

These last steps matter because small leftover leaks tend to become the footholds for future drift.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Reduced remaining runtime ownership by removing live mutation helpers, delegating runtime-shaped helper calls outward, and keeping the file focused on rendering, event handling, and UI-local state. |
| `crates/themion-cli/src/app_runtime.rs` | Became the main home for runtime-owned helper logic used by TUI mode, including agent-turn launch preparation, relay wiring, roster/status helpers, and Stylos/runtime coordination support. |
| `crates/themion-cli/src/app_state.rs` | Took on more app-state-owned snapshot/runtime helper responsibilities, including context-report projection and TUI runtime service startup flow. |
| `crates/themion-cli/src/tui_runner.rs` | Clarified ownership of terminal-mode orchestration, startup, redraw/watch loops, and shutdown sequencing outside the TUI presentation file. |
| `crates/themion-cli/src/headless_runner.rs` | Stayed aligned with the shared `AppState`/runtime split so non-TUI flows consume the same CLI runtime shape rather than TUI-owned state. |
| `crates/themion-cli/src/stylos.rs` | Stayed integrated with the new runtime-owned Stylos startup/status pathways instead of depending on TUI-owned truth. |
| `docs/architecture.md` | Updated to describe the TUI as a strict human I/O surface and the hub/app-state layer as the owner of runtime truth and decisions. |
| `docs/engine-runtime.md` | Updated to reflect the landed runtime ownership boundaries across TUI, headless, and Stylos-related paths. |
| `docs/README.md` | Updated to register this retrospective PRD so the historical record matches the already-landed implementation slice. |
| `docs/tui-app-state-refactor-checklist.md` and `experiments/*tui*checklist*.md` | Capture the working audit trail and implementation checklist history that fed this completed cleanup. |

## Edge Cases

- a TUI-triggered agent turn starts after the ownership move → verify: the same target agent becomes busy, events still relay correctly, and the agent object is returned to runtime ownership when done.
- Stylos is disabled in the default build → verify: the non-Stylos TUI/test build still compiles cleanly without leaking gated imports or startup paths.
- Stylos is enabled with TUI mode → verify: startup wiring, event streams, status publication, and shutdown still work through app-state/runtime-owned paths rather than TUI-owned glue.
- `/context` is invoked from the TUI after the cleanup → verify: the command still returns the same prompt-context report without the TUI directly owning live agent traversal logic.
- headless and TUI flows both depend on shared runtime data → verify: the shared `AppState`/runtime layer remains the common source of truth instead of regressing into surface-specific ownership.
- a future debug or display feature needs runtime data → verify: read-only projection remains acceptable, but new policy or mutation paths do not get added back into `tui.rs`.

## Migration

This was an internal architecture refactor with no external data migration.

The effective rollout that landed was:

- extract remaining runtime-owned helper flows out of `tui.rs`
- move terminal-mode orchestration around the TUI into `tui_runner.rs` and app-state helpers
- fix feature-gating and test-build paths so the new ownership split compiles in both default and Stylos-enabled configurations
- update documentation and checklists so the architecture record matches the landed code

## Testing

- run `cargo check -p themion-cli` after the refactor → verify: default CLI/TUI paths build with the new ownership split.
- run `cargo check -p themion-cli --all-features` after the refactor → verify: Stylos-enabled TUI/runtime paths still compile after the moved ownership boundaries.
- run `cargo test -p themion-cli` after the refactor → verify: default-build tests compile and pass without hidden TUI-owned runtime assumptions.
- run `cargo test -p themion-cli --all-features` after the refactor → verify: Stylos-enabled tests pass and the feature-gated ownership split is consistent.
- inspect `tui.rs` and `tui_runner.rs` after the cleanup → verify: no meaningful runtime-policy or mutation ownership remains in the TUI surface layer.
- compare startup/shutdown flow before and after the refactor → verify: terminal lifecycle still works while Stylos and runtime coordination are owned outside `tui.rs`.

## Implementation checklist

- [x] move remaining agent-turn runtime mutation helpers out of `tui.rs`
- [x] move TUI-mode runtime startup/shutdown coordination into `app_state.rs` and `tui_runner.rs`
- [x] centralize more shared runtime projection/helper behavior in `app_state.rs` and `app_runtime.rs`
- [x] remove final dead mutable-agent helper access from the TUI surface
- [x] move `/context` live-agent traversal behind a non-TUI helper path
- [x] fix default-build and Stylos-enabled test/build regressions created by the ownership split
- [x] validate with `cargo check -p themion-cli`
- [x] validate with `cargo check -p themion-cli --all-features`
- [x] validate with `cargo test -p themion-cli`
- [x] validate with `cargo test -p themion-cli --all-features`
- [x] add this retrospective PRD to `docs/README.md`

## Technical note: audited change window

This retrospective PRD is based on the implemented change window from commit `5fa0bf2b8d97f9728a7785a701fb71262673f08b` to current `HEAD`, especially these refactor-focused commits:

- `91586ba` — refactor CLI runtime ownership and update architecture notes
- `5ef461a` — move TUI runtime side effects into app runtime
- `5eb891c` — move watchdog prompt handling out of TUI
- `223e5ed` — commit remaining watchdog refactor sources
- `0f94dba` — fix TUI runtime ownership and headless JSON output
- `b391275` — move TUI runtime ownership into app_state
- `7e15ced` — move CLI runtime mutations out of TUI
- `d01d66d` — tighten TUI surface ownership boundaries

Commits in the same range that were not the core of this refactor, such as PRD-086 multiple-profile completion work, were treated as adjacent context rather than the primary subject of this PRD.
