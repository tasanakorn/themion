# PRD-084: Move Non-Input/Output Responsibilities out of the TUI

- **Status:** Implemented
- **Version:** v0.55.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-05-01

## Summary

- Themion's architecture already says the TUI should be the terminal input/output surface, but `crates/themion-cli/src/tui.rs` still owned several runtime-policy and transport-coordination responsibilities that were not really UI work.
- This implementation slice keeps rendering, terminal event translation, transcript formatting, and UI-local view state in the TUI, while moving incoming-prompt policy, sender-side Stylos transport-event derivation, and runtime-shaped helper types into focused non-TUI CLI modules.
- The TUI-side snapshot-publication hook that previously owned runtime wiring was removed so terminal-mode orchestration no longer asks `tui.rs` to publish Stylos status snapshots.
- Current single-process architecture, Stylos behavior, multi-agent targeting rules, and user-visible TUI flows remain preserved; this was a layering cleanup, not a product redesign.
- The landed outcome is that the main remaining runtime or transport policy from this slice now belongs in `app_runtime.rs`, `stylos.rs`, `tui_runner.rs`, and adjacent non-TUI helpers instead of drifting back into `tui.rs`.

## Goals

- Make `crates/themion-cli/src/tui.rs` more strictly a terminal presentation and input/output layer.
- Move the main remaining non-visual orchestration paths out of the TUI into focused non-TUI `themion-cli` helpers.
- Keep the documented architecture trustworthy: reusable harness behavior in `themion-core`, CLI-local runtime/orchestration in non-TUI `themion-cli` modules, and terminal rendering/input handling in `tui.rs`.
- Reduce future architecture drift by removing the remaining high-friction reasons for runtime policy to stay TUI-owned.
- Make the affected runtime behavior easier to test, reason about, and potentially share with non-TUI entrypoints without requiring transcript- or Ratatui-shaped code.
- Treat architecture and guideline-document updates as required follow-through for this work, not optional cleanup.

## Non-goals

- No rewrite of the TUI widget tree, redraw model, transcript rendering, or input editor behavior.
- No requirement to move all CLI-local orchestration into `themion-core`.
- No redesign of Stylos protocols, board-note semantics, task lifecycle semantics, or the local multi-agent model.
- No requirement to eliminate every async bridge or all `block_in_place` usage across the crate.
- No multi-process redesign or change to the explicit runtime-domain topology.
- No requirement that `tui.rs` become small in absolute size; the goal is clearer ownership, not file-size optimization.

## Background & Motivation

### Current state

Repository guidance already says the TUI should act as an input/output surface rather than the owner of runtime orchestration or agent-management policy.

That direction has already landed in several steps:

- PRD-051 moved shared bootstrap into `app_state.rs` and separated TUI mode from headless entrypoints.
- PRD-053 moved more runtime-shaped behavior out of `tui.rs` and clarified runtime-domain ownership.
- PRD-065 moved board-note DB coordination into `board_runtime.rs`.
- PRD-083 moved watchdog timer/state and idle-agent selection policy into `app_runtime.rs` and `board_runtime.rs`.

This PRD completed the next cleanup slice by removing the most boundary-violating TUI-owned runtime work from the implemented code path.

The landed implementation now includes:

- incoming-prompt acceptance/rejection policy delegated from `crates/themion-cli/src/tui.rs` into `crates/themion-cli/src/app_runtime.rs`
- sender-side Stylos transport-event derivation delegated into `crates/themion-cli/src/stylos.rs` through explicit helper logic instead of transcript backtracking
- removal of the TUI-owned snapshot-refresh hook so terminal-mode orchestration no longer asks `tui.rs` to own that runtime wiring path
- relocation of runtime/helper ownership such as `LocalAgentManagementRequest` out of `tui.rs`

The result is no longer just a softer boundary on paper; the current code now routes the main targeted policy through non-TUI helpers.

This PRD also affected durable repository guidance, not just code placement. The repository's architecture instructions and PRD-authoring expectations now explicitly treat guideline-document updates as required follow-through whenever this kind of work changes documented boundaries.

**Alternative considered:** stop after the earlier extractions and accept the previous state as sufficiently improved. Rejected: the remaining responsibility clusters were clear and cohesive enough that leaving them in the TUI would have kept the intended boundary easy to regress.

### What was most boundary-violating in the previous TUI

The previous TUI still owned four especially non-UI responsibility clusters, and these were the main targets of this PRD.

#### A. Incoming-prompt admission and rejection policy

`handle_incoming_prompt_event(...)` previously decided whether the request was admissible at all, which local agent should own it, whether an in-flight board-note claim had to be released, whether a remote task had to be marked failed, and what follow-up state should be attached for later completion handling.

That scheduler and transport-intake policy is now resolved through non-TUI helper logic in `app_runtime.rs`, while `tui.rs` renders and submits the returned outcome.

#### B. Sender-side transport event derivation from transcript history

`maybe_log_sender_side_stylos_talk(...)` previously derived semantic transport events by looking backward through rendered transcript entries.

That ownership was removed. Sender-side talk/note logging is now derived through explicit helper logic in `stylos.rs`, and the TUI only renders the explicit event when present.

#### C. Snapshot-provider publication and runtime blocking glue

The previous snapshot-provider publication path in `tui.rs` was non-visual long-lived runtime wiring.

That TUI-owned hook is now gone. `tui_runner.rs` no longer asks the TUI to own snapshot refresh publication, which removes the targeted TUI-owned runtime wiring path from this implementation slice.

#### D. Runtime- and export-shaped helper types in the TUI file

Types and helpers such as `LocalAgentManagementRequest` were not Ratatui-facing concepts.

That request type now lives outside `tui.rs`, reducing the tendency for runtime logic to accrete in the TUI layer.

**Alternative considered:** treat all remaining TUI leakage as one undifferentiated cleanup bucket. Rejected: calling out the specific most-violating clusters made the implementation scope clearer and review easier.

### Why this was a distinct implementation-ready slice

PRD-065 and PRD-083 each removed meaningful TUI-owned runtime work, but neither claimed to finish the cleanup completely.

This PRD was the next follow-through slice because:

- the remaining TUI-owned runtime policy was narrow enough to target directly
- nearby non-TUI helper modules already existed, so the moves could stay incremental rather than architectural resets
- type placement and ownership had become the main sources of ongoing drift, more than the earlier large structural problems
- the work also needed durable guidance updates, and those updates needed to be treated as part of done rather than left implicit

**Alternative considered:** reopen PRD-065 instead of creating a successor PRD. Rejected: the remaining targets formed a new concrete implementation slice and needed their own acceptance expectations.

## Design

### 1. Treat `tui.rs` as the terminal I/O boundary, not the owner of runtime policy

For this PRD, `crates/themion-cli/src/tui.rs` now more explicitly owns:

- rendering chat, overlays, status, transcript, and review-oriented terminal views
- translating keyboard, mouse, paste, tick, and bridge events into UI actions
- local UI state such as scroll position, review mode, focused input state, and display formatting
- invoking already-defined runtime/helper actions and rendering their returned results

It no longer owns the targeted implementation-slice behavior for:

- incoming remote-intake acceptance/rejection policy
- sender-side transport/logging derivation from transcript backtracking
- the removed snapshot-refresh hook that previously represented TUI-owned runtime wiring
- helper/request types whose purpose is runtime coordination rather than UI state

Because this changed the repository's documented architecture boundary in a meaningful way, the implementation updated the corresponding guideline documents in the same task rather than treating those updates as optional follow-up.

**Alternative considered:** keep the TUI as a broad interactive controller that can continue owning mixed runtime and display responsibilities. Rejected: that framing is exactly what lets non-visual policy keep accumulating in `tui.rs`.

### 2. Extract incoming-prompt acceptance and follow-up policy into a CLI-local runtime helper

The clearest and most boundary-violating remaining non-IO cluster was incoming-prompt handling.

The landed implementation moved the main acceptance/rejection sequencing into `app_runtime.rs`, where helper logic now decides:

- whether the request is accepted or rejected
- whether local board-note claim release is required
- whether remote task failure state must be updated
- what user-visible log/status line should be shown
- which local agent should receive the request and what follow-up state remains attached to that agent

The TUI now renders the returned outcome, updates UI-local state, and invokes prompt submission.

The helper remains in `themion-cli`, not `themion-core`, because the flow still depends on process-local agent roster state, task-registry wiring, and local Stylos bridge policy.

**Alternative considered:** move the full incoming-prompt path into `themion-core`. Rejected: the behavior is still CLI-local orchestration rather than reusable harness logic.

### 3. Replace transcript-scraping transport event derivation with explicit runtime results

`maybe_log_sender_side_stylos_talk(...)` previously inferred transport-level events by reading backward through transcript entries after tool completion.

The landed implementation replaced that with explicit helper-based derivation in `stylos.rs`. The TUI now tracks an explicit sender-side transport event result and renders it when the tool completes, without inferring semantics from transcript history.

This makes the event path more direct and less coupled to transcript implementation details.

**Alternative considered:** keep transcript scraping because it already worked and avoided a new helper or event shape. Rejected: transcript backtracking was exactly the kind of accidental coupling this cleanup needed to remove.

### 4. Move snapshot-provider publication and similar runtime wiring out of the TUI

The previous TUI-owned snapshot refresh/publication path represented long-lived runtime integration work.

The landed implementation removed that TUI-owned hook, and terminal-mode orchestration in `tui_runner.rs` no longer asks `tui.rs` to own the snapshot refresh/publication path targeted by this PRD.

This PRD did not redesign the broader Stylos status publisher implementation, but it did remove the targeted TUI-owned runtime wiring from the presentation layer.

**Alternative considered:** leave snapshot publication in `tui.rs` because it had convenient access to current agent state. Rejected: convenient access was not sufficient justification for long-lived runtime ownership.

### 5. Relocate runtime-shaped helper types out of `tui.rs`

Some types previously lived in `tui.rs` mostly because of historical placement rather than genuine UI ownership.

This implementation slice moved the main targeted request-type ownership out of `tui.rs`, especially:

- `LocalAgentManagementRequest`

The broader principle remains the same:

- move runtime-shaped helper types to non-TUI CLI modules near the behavior they support
- keep UI-facing enums, entry types, rendering helpers, and immediate interaction logic in `tui.rs`
- avoid replacing one overgrown file with one overgrown catch-all helper module

**Alternative considered:** move only large functions and leave helper types in `tui.rs`. Rejected: if the types remain TUI-owned, future runtime logic will continue to gravitate back there.

### 6. Preserve current user-visible behavior while tightening ownership

This PRD was an internal architecture cleanup and preserved:

- current TUI transcript and statusline behavior
- current Stylos talk, note, and task intake semantics
- current board-note claim/release behavior
- current multi-agent targeting and busy reporting semantics
- current TUI, headless, and non-interactive mode behavior

Small user-visible changes were acceptable only when they were incidental bug fixes or clearer logging that fell out of the cleaner ownership model.

**Alternative considered:** combine the cleanup with broader transport or routing behavior changes. Rejected: mixing boundary cleanup with product-behavior redesign would make review and regression analysis harder.

### 7. Guideline-document updates were required for this PRD

This PRD changed architecture guidance, not only implementation ownership.

The landed implementation updated:

- `docs/architecture.md`
- `docs/engine-runtime.md`
- `docs/prd/PRD_AUTHORING_GUIDE.md`
- `docs/README.md`
- this PRD

Those guidance updates were treated as part of the implementation definition of done, not as optional cleanup after code landed.

**Alternative considered:** update only code-facing docs and leave broader authoring or guidance documents unchanged. Rejected: when the repository already relies on durable guidance documents, leaving them stale invites repeated boundary drift.

### 8. Acceptance target for this implementation slice

This PRD is considered implemented because all of the following are now true:

- `tui.rs` no longer owns the main incoming-prompt acceptance/rejection policy path
- sender-side Stylos talk/note event logging no longer depends on transcript backtracking in the TUI
- snapshot-refresh/publication wiring targeted by this PRD no longer lives in `tui.rs`
- the selected runtime-shaped helper/request types targeted by this slice have been moved out of `tui.rs`
- current Stylos talk/note/task behavior and local multi-agent targeting semantics remain unchanged
- `docs/architecture.md`, `docs/engine-runtime.md`, `docs/README.md`, this PRD, and the touched durable guidance documents reflect the landed ownership boundaries accurately
- `cargo check -p themion-cli` passed
- `cargo check -p themion-cli --features stylos` passed
- `cargo check -p themion-cli --all-features` passed

This acceptance target keeps the cleanup implementation-shaped without expanding it into a general rewrite of all remaining TUI logic.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Removed the targeted remaining non-input/output coordination paths from this slice, especially inline incoming-prompt acceptance policy, transcript-based sender-side transport event derivation, the TUI-owned snapshot refresh hook, and runtime-shaped helper/type definitions such as `LocalAgentManagementRequest` that no longer require TUI ownership. |
| `crates/themion-cli/src/app_runtime.rs` | Hosts incoming-prompt acceptance/rejection helpers, runtime-owned action/result types, local-agent management request types, and related CLI-local orchestration previously leaking through `tui.rs`. |
| `crates/themion-cli/src/tui_runner.rs` | Keeps terminal-mode orchestration explicit and no longer routes the targeted Stylos snapshot-refresh publication hook through `tui.rs`. |
| `crates/themion-cli/src/stylos.rs` | Exposes clearer sender-side transport result signals and helper paths so the TUI no longer needs transcript-based inference for Stylos event logging. |
| `crates/themion-cli/src/board_runtime.rs` | Continues owning board-note claim/release and note-follow-up coordination, and integrates with the extracted incoming-prompt helper where board-note handoff results are needed. |
| `docs/architecture.md` | Updated the TUI boundary description so terminal input/output stays in `tui.rs` while runtime coordination for this slice lives in non-TUI helpers. |
| `docs/engine-runtime.md` | Updated runtime-flow documentation to describe the CLI-local helper boundaries for incoming prompts and sender-side Stylos event derivation, and to reflect the removed TUI-owned snapshot hook. |
| `docs/prd/PRD_AUTHORING_GUIDE.md` | Strengthened authoring guidance so durable guideline-document updates are treated as required follow-through when PRDs change documented behavior or repository guidance. |
| `docs/README.md` | Keeps the PRD index aligned with this PRD's implemented state. |

## Edge Cases

- an incoming prompt targets a missing local agent → verify: rejection semantics and task failure reporting remain unchanged after the policy moves out of the TUI.
- a board-note handoff loses the race because the selected agent becomes busy → verify: claim release still occurs exactly once and the TUI only renders the returned outcome.
- a remote task request is rejected during local intake → verify: task-registry failure state is still updated even though the TUI no longer owns that side effect.
- Stylos is disabled → verify: extracted helper boundaries remain correctly feature-gated and do not leak Stylos-only types into always-on code paths.
- sender-side talk or note logging changes ownership → verify: the same user-visible event still appears without relying on transcript backtracking.
- snapshot publication ownership changes away from the TUI → verify: status publication still works without the previous TUI-owned hook.
- guideline documents are touched in the same task → verify: architecture and authoring guidance remain aligned with the landed boundary instead of drifting behind the code.

## Migration

This is an internal architecture cleanup with no user data or external protocol migration.

Recommended rollout shape:

- extract typed runtime helpers for incoming-prompt handling and sender-side transport event derivation
- remove the targeted TUI-owned snapshot refresh/publication hook
- update docs and durable guidance so the TUI boundary matches the implemented code shape exactly

## Testing

- send a Stylos talk request to an idle local agent after the extraction → verify: it is accepted, routed to the same target, and the same sender-side and receiver-side transcript events still appear.
- send a targeted request to a missing or busy local agent after moving intake policy out of the TUI → verify: rejection text, task failure state, and note-claim release behavior remain correct.
- deliver a pending board note through watchdog or remote intake after the extraction → verify: one eligible local agent receives it, duplicate in-process injection still does not occur, and completion follow-up behavior is unchanged.
- run TUI mode with `--features stylos` after removing the TUI-owned snapshot hook → verify: status publication, query handling, and shutdown still work without TUI-owned runtime glue.
- run explicit `--headless` and non-interactive prompt mode after the cleanup → verify: shared runtime helpers remain non-TUI and do not regress non-interactive paths.
- inspect `docs/architecture.md`, `docs/engine-runtime.md`, and any touched guidance documents against the touched CLI modules → verify: the documented TUI boundary matches the implemented ownership after the extraction.

## Implementation checklist

- [x] extract a non-TUI helper boundary for `handle_incoming_prompt_event(...)` acceptance/rejection policy and follow-up state updates
- [x] remove sender-side Stylos event derivation from transcript inspection in `crates/themion-cli/src/tui.rs`
- [x] move snapshot-provider publication and similar runtime wiring out of the `tui.rs`-owned implementation path targeted by this PRD
- [x] relocate runtime-shaped helper/request types from `tui.rs`, including `LocalAgentManagementRequest`, into non-TUI CLI modules where appropriate
- [x] preserve current Stylos talk/note/task semantics and multi-agent targeting behavior
- [x] update `docs/architecture.md`, `docs/engine-runtime.md`, `docs/README.md`, `docs/prd/PRD_AUTHORING_GUIDE.md`, and this PRD to reflect the landed boundary
- [x] run `cargo check -p themion-cli`
- [x] run `cargo check -p themion-cli --features stylos`
- [x] run `cargo check -p themion-cli --all-features`
