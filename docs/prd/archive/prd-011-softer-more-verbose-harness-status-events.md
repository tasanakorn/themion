# PRD-011: Softer, More Verbose Harness Status Events

- **Status:** Implemented
- **Version:** v0.6.0
- **Scope:** `themion-core` (workflow/runtime event emission), `themion-cli` (status rendering), docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Make harness status output more verbose at key runtime boundaries so users can understand what the engine is doing without reading source or inferring hidden state changes.
- Add explicit status reporting for turn start, workflow transitions, workflow phase transitions, and workflow phase-result updates.
- Keep these messages informative but softer than success-oriented green completion markers so they read as neutral progress updates rather than confirmations.
- Improve observability of workflow-driven runs without changing the underlying workflow semantics or tool contract.
- Keep event production in `themion-core` and presentation treatment in `themion-cli`.

## Non-goals

- No redesign of the workflow model, phase graph, retry policy, or workflow persistence semantics.
- No requirement to make every runtime event user-visible; this PRD is limited to the specified harness status boundaries.
- No requirement that these new status messages be rendered as green success states.
- No change to the meaning of existing workflow tools such as `workflow_set_active`, `workflow_set_phase`, or `workflow_set_phase_result`.
- No broad TUI visual redesign beyond what is needed to present the new status events clearly and softly.

## Background & Motivation

### Current state

Themion already exposes some runtime progress in the TUI through conversation entries, tool-call rows, and a small set of lifecycle indicators such as tool completion and turn statistics. Workflow state is also visible in the statusline.

That is useful, but it leaves several important harness boundaries under-explained during a run:

- when a turn begins
- when the active workflow changes
- when the workflow phase changes
- when the workflow phase result changes

Today, a user may only infer these transitions indirectly by watching later model behavior, reading the statusline, or inspecting persisted workflow state. This makes workflow-driven execution feel more opaque than it needs to be.

At the same time, not every status message should read like a successful completion. Some of these events are neutral state changes, not accomplishments. If they inherit the same green styling used for done states, the UI can over-signal certainty or success.

### Why softer status messages matter

The harness is increasingly workflow-aware. As more behavior is governed by explicit workflow state, the user benefits from seeing that state change in a narrative way inside the normal conversation/event stream.

The desired tone is:

- visible enough to understand what changed
- neutral enough that a transition does not imply success
- consistent enough that repeated workflow activity remains easy to scan

This is especially useful when the model uses workflow tools directly, when retries or recoveries occur, and when users need to understand why the agent is behaving differently from one turn to the next.

## Design

### New harness status event coverage

Themion should emit explicit status events for the following boundaries:

- turn start
- workflow transition
- workflow phase transition
- workflow phase-result update

These events should be considered first-class harness status updates rather than implicit side effects visible only through later state snapshots.

The event text should be concise and descriptive. Example shapes:

- `turn started`
- `workflow: NORMAL -> LITE`
- `phase: EXECUTE -> VALIDATE`
- `phase result: pending -> passed`

The exact phrasing may vary in implementation, but each event should state what changed in a compact, human-readable form.

**Alternative considered:** rely only on the existing statusline and workflow state snapshots. Rejected: those surfaces show current state but do not narrate when and how the state changed during a run.

### Soft visual treatment

These new status events should use a softer visual treatment than green success markers.

Normative presentation intent:

- they should not share the exact visual language of `done`, `passed`, or completed-success feedback
- they should read as informational progress events
- they should remain distinguishable from normal assistant text and from tool-call rows

A practical first implementation may use dim, muted, or neutral coloring instead of green. The important requirement is semantic separation: transition updates are not themselves success confirmations.

This applies especially to:

- workflow transition messages
- phase transition messages
- phase-result updates, even when the new result is `passed`

A phase-result update of `passed` is still primarily a state update in this event stream, not a substitute for workflow completion signaling.

**Alternative considered:** keep using green for any positive-looking transition, especially `passed`. Rejected: that blurs the distinction between status narration and actual completion/success outcomes.

### Event model responsibilities

`themion-core` should remain responsible for deciding when these events happen and what structured information they carry. `themion-cli` should remain responsible for rendering them.

Preferred implementation shape:

- `themion-core` emits explicit event variants or equivalent structured notifications for the four new status boundaries
- each event carries enough context to render a concise message without re-deriving transition details from global state
- `themion-cli` maps those events into conversation/status entries with a softer style than success markers

This keeps workflow/runtime semantics centralized and avoids duplicating transition inference in the UI layer.

**Alternative considered:** have the CLI detect state changes by diffing periodic workflow snapshots. Rejected: that duplicates logic, risks missed transitions, and weakens the separation between runtime authority and UI rendering.

### Event text expectations

The rendered status text should prioritize scanability over detail.

Recommended content rules:

- turn start should identify that a new harness turn has begun; include turn sequence when easily available
- workflow transition should show old workflow and new workflow
- workflow phase transition should show old phase and new phase
- workflow phase-result update should show old result and new result
- when a reason string already exists in runtime state and is short, the UI may append it secondarily, but the transition itself should remain the primary content

Examples:

- `turn 12 started`
- `workflow changed: NORMAL -> LITE`
- `phase changed: EXECUTE -> VALIDATE`
- `phase result updated: pending -> failed`

Implementations should avoid verbose paragraphs or duplicated state dumps for these inline events.

### Interaction with existing event types

These new status events should coexist with existing tool-call entries, assistant output, and final turn statistics.

Required behavior:

- they should appear in chronological order with the rest of the event stream
- they should not replace the statusline's current-state role
- they should not suppress existing completion or error signaling where those are still meaningful
- they should not require users to inspect raw workflow JSON to understand ordinary transitions

If the current UI uses a `ToolDone`-style green completion marker, the new harness status events should not reuse that same entry type unless its styling can be differentiated cleanly.

**Alternative considered:** overload existing generic stats or tool-done rows for workflow transitions. Rejected: transitions are a distinct category of runtime narration and should not masquerade as tool completions.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/agent.rs` | Emit explicit runtime events for turn start, workflow activation/transition, workflow phase transition, and workflow phase-result changes at the point where the harness already knows old and new state. |
| `crates/themion-core/src/workflow.rs` | If needed, expose helper structures or transition metadata so event emission can include old/new workflow, phase, and phase-result values without duplicating validation logic. |
| `crates/themion-cli/src/tui.rs` | Add rendering support for the new harness status events and style them with a softer, neutral presentation distinct from green success markers. |
| `docs/core-ai-engine-loop.md` | Document that the harness now emits explicit status events for turn and workflow-state boundaries, and describe their user-facing purpose. |
| `docs/architecture.md` | Update the TUI/runtime event description so workflow transitions and phase-result updates are represented as part of normal observability. |
| `docs/README.md` | Add this PRD to the index and keep its status aligned with implementation progress. |

## Edge Cases

- a turn starts but fails before any model output arrives → still show the turn-start event so the user can see that the harness began processing.
- a workflow is set to its already-active value → avoid noisy duplicate transition messaging unless the runtime treats it as a real transition.
- a phase transition is rejected by workflow validation → do not emit a successful phase-transition status event for a state change that did not happen.
- a phase result is updated multiple times in one turn → show each real change in chronological order, but do not emit duplicates for no-op updates.
- a phase result changes to `passed` but the workflow does not complete yet → keep the event styled as a neutral status update, not as final success.
- retry and recovery paths cause repeated phase transitions → the softer style should keep repeated workflow narration readable rather than celebratory.
- non-workflow turns in workflows such as `NORMAL` still begin normally → turn-start events should remain available even when no workflow transition occurs.

## Migration

This change is additive and presentation-focused.

There is no database migration requirement unless implementation chooses to persist these status events for history playback. If the first implementation keeps them as live UI/runtime events only, existing persisted sessions remain compatible.

Users should simply begin seeing richer inline harness status narration after upgrade. Older sessions do not need backfill.

## Testing

- start a normal user turn → verify: the event stream shows a turn-start status message before assistant output or tool activity.
- activate a different workflow through normal runtime behavior or `workflow_set_active` → verify: the UI shows a workflow-transition status update with old and new workflow names.
- trigger a valid phase change through `workflow_set_phase` or runtime progression → verify: the UI shows a phase-transition status update in chronological order.
- update the phase result through `workflow_set_phase_result` → verify: the UI shows a phase-result update with old and new values.
- trigger a no-op workflow or phase update request → verify: no misleading transition status entry is emitted when state did not actually change.
- observe a phase-result update to `passed` → verify: it uses the softer neutral status style rather than the green completion style used for done markers.
- run a retry/recovery scenario that causes repeated phase transitions → verify: each transition is visible and readable without being rendered as repeated success confirmations.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: new runtime event variants and TUI rendering paths compile cleanly.
