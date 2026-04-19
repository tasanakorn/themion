# PRD-008: Workflow Phase Retry and Recovery Policy

- **Status:** Proposed
- **Version:** v0.5.0
- **Scope:** `themion-core` (workflow runtime, retry policy, persistence, workflow tools); `themion-cli` (statusline retry display); docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Extend the workflow engine introduced by PRD-006 and the `LITE` workflow structure proposed in PRD-007 so a phase that cannot continue does not always stop permanently on the first failure.
- Add explicit retry behavior that allows the runtime to either retry the current phase or step back to the previous phase when that workflow definition permits recovery.
- Enforce separate retry caps for each recovery mode: up to 3 retries of the current phase and up to 3 retries that move to the previous phase.
- Persist retry state so interrupted sessions, resumed turns, and history inspection can reconstruct how many retries have already been consumed for the active phase.
- Surface retry progress in the UI statusline so users can see when the current phase is running under retry, for example `| phase: EXECUTE (1/3) |`.
- Keep retry semantics in `themion-core`, with `themion-cli` limited to display.

## Non-goals

- No unbounded retry loop.
- No arbitrary user-configurable retry counts in the first version.
- No workflow-wide global retry pool shared across all phases; limits apply per phase and per recovery mode.
- No requirement to retry every failure automatically; some failures may still be terminal when the workflow definition says no valid recovery path exists.
- No redesign of the existing workflow graph model beyond what is needed to support explicit retry and previous-phase recovery.

## Background & Motivation

### Current state

PRD-006 made workflow and phase progression explicit runtime state, persisted in SQLite and shown in the TUI. PRD-007 then proposed a richer built-in `LITE` workflow with `CLARIFY`, `EXECUTE`, and `VALIDATE`, but that design still described validation and execution as effectively fail-fast. In particular, PRD-007 explicitly said there is no automatic retry loop for lite validation failures and no `VALIDATE -> EXECUTE` retry path in the first version.

That behavior is simple, but it is too rigid for practical workflow execution. Some failures are recoverable within the current phase, while others need the runtime to step back to a previous phase and rebuild context before continuing. If the runtime always stops immediately, users lose the benefit of the workflow structure and are forced to restart manually.

### Why this PRD extends PRD-007

This PRD improves the workflow design from PRD-007 by replacing the purely fail-fast behavior with bounded recovery semantics.

The key change is:

- when a phase cannot continue, the runtime should first determine whether it can retry the current phase
- if retrying the current phase is not sufficient or is not valid, the runtime may move back to the previous phase when one exists and retry from there
- both retry modes are capped and persisted independently
- when both recovery paths are exhausted, the workflow stops with a clear failed state

This keeps the workflow deterministic while making it more resilient.

### Why retry counts must be persisted

Retry behavior changes runtime semantics across model calls and across turns. If the process exits or the workflow pauses, the engine must not forget how many retries were already spent. Without persistence:

- a restarted session could accidentally exceed the intended retry cap
- the statusline could show misleading retry state
- history inspection could not explain why a workflow finally failed
- phase recovery would behave inconsistently across interruptions

Retry counts therefore belong in the persisted workflow runtime state and transition history, not only in memory.

## Design

### Recovery policy as part of workflow semantics

Each workflow phase should be able to define whether recovery is allowed when the phase cannot continue. The first implementation should support two recovery actions:

- `retry_current_phase`
- `retry_previous_phase`

The runtime should treat these as explicit workflow actions rather than ad hoc loop behavior.

Normative limits for the first version:

- maximum current-phase retries per phase instance: `3`
- maximum previous-phase retries per phase instance: `3`

When a phase reports that it cannot continue, the runtime should evaluate recovery in this order:

1. if retrying the current phase is allowed and its counter is below `3`, retry the current phase
2. otherwise, if moving to the previous phase is allowed, a previous phase exists, and that counter is below `3`, transition to the previous phase
3. otherwise, mark the workflow failed

This makes recovery deterministic and bounded.

**Alternative considered:** use one shared retry counter for all recovery behavior. Rejected: retrying the same phase and stepping back to a previous phase are different recovery strategies and need separate limits for clearer control and observability.

### Retry state model

The workflow runtime state should track retry counters per active phase.

At minimum, persisted runtime state should be able to represent:

- active workflow name
- current phase name
- workflow status
- current-phase retry count for the active phase
- previous-phase retry count associated with the active phase's recovery path
- maximum current-phase retry count, fixed at `3`
- maximum previous-phase retry count, fixed at `3`
- whether the current phase entry was reached normally, by retrying the same phase, or by moving back from a later phase

A practical first-version model is a small persisted retry-state payload attached to session workflow state and copied into transition history whenever the phase changes.

Conceptually:

```text
workflow: LITE
phase: EXECUTE
status: running
retry_state:
  current_phase_retries: 1
  current_phase_retry_limit: 3
  previous_phase_retries: 0
  previous_phase_retry_limit: 3
  entered_via: retry_current_phase
```

This state should be reset when the workflow changes and when the engine advances into a genuinely new next phase without carrying retry debt forward.

**Alternative considered:** infer retries only from counting repeated transitions in history. Rejected: that is fragile for live runtime logic and makes statusline rendering and resume behavior harder.

### Phase-level recovery rules

Workflow definitions should explicitly declare whether a phase allows retry of the current phase and whether it allows stepping back to a previous phase.

A useful first-version rule set for `LITE` is:

| Phase | Retry current phase | Retry previous phase | Previous phase |
| --- | --- | --- | --- |
| `CLARIFY` | yes | no | none |
| `EXECUTE` | yes | yes | `CLARIFY` |
| `VALIDATE` | yes | yes | `EXECUTE` |

Behavioral meaning:

- `CLARIFY` may retry itself if it can refine the brief, but there is no earlier phase to return to
- `EXECUTE` may retry execution directly for recoverable issues, or step back to `CLARIFY` if assumptions or scope need to be refreshed
- `VALIDATE` may retry validation directly for transient validation issues, or step back to `EXECUTE` if the validation result shows more implementation work is needed

This supersedes the fail-fast parts of PRD-007.

**Alternative considered:** allow previous-phase recovery only from `VALIDATE`. Rejected: `EXECUTE` can also become blocked by bad assumptions discovered after clarify, so it also benefits from a bounded step-back path.

### Retry decision and transition handling

When a phase cannot continue, the runtime should record a retry decision as an explicit workflow transition.

Suggested transition kinds:

- `phase_retry_current`
- `phase_retry_previous`
- `phase_retry_exhausted`

Suggested trigger sources:

- `model_completion`
- `tool_result`
- `engine_rule`
- `user_input`

Required behavior:

- retrying the current phase keeps the same phase active and increments `current_phase_retries`
- retrying to the previous phase transitions to that previous phase and increments `previous_phase_retries` for the failed phase context
- when the workflow later returns to the failed phase after a previous-phase retry, the runtime should preserve enough state to know that one previous-phase recovery was already consumed
- exhausting both retry modes marks the workflow failed and persists the exhaustion reason

The runtime may expose or reuse workflow tools so the model can request a retry or previous-phase move, but runtime validation remains authoritative.

### Interaction with workflow tools

The workflow tools proposed in PRD-007 should be extended so retry behavior is inspectable and controllable through structured runtime APIs.

At minimum:

- `get_workflow_state` should return retry counters and retry limits for the active phase when applicable
- `set_workflow_phase` should validate previous-phase moves against the recovery policy instead of treating them as unrestricted manual phase switches
- `complete_workflow` should continue to mark terminal success or failure, including failure due to retry exhaustion

An implementation may add a dedicated retry tool later, but the first version can keep recovery under the existing workflow-control surface as long as the runtime semantics stay explicit.

**Alternative considered:** hide all retry behavior behind implicit engine rules with no tool visibility. Rejected: the model and the user benefit from being able to inspect the retry state, and the runtime needs a structured API for debugging and future extensibility.

### Persistence requirements

Retry state should be persisted both as current session runtime state and as historical transition data.

#### Session-level runtime state

The session workflow state should include retry information for the active phase so resume logic and the TUI can read it directly.

Expected additions or equivalent representation:

- `current_phase_retry_count`
- `current_phase_retry_limit`
- `previous_phase_retry_count`
- `previous_phase_retry_limit`
- `phase_entered_via`

If the implementation prefers a serialized retry payload instead of individual columns, that is acceptable as long as runtime reads and writes remain explicit and stable.

#### Transition history

Workflow transition history should record retry-related events so later inspection can reconstruct what happened.

At minimum, each retry-related transition should record:

- workflow name
- from phase
- to phase
- transition kind
- resulting workflow status
- retry counts after the transition
- reason or trigger source when practical

This allows history inspection to answer questions such as:

- how many times did `EXECUTE` retry itself before succeeding?
- did `VALIDATE` step back to `EXECUTE`?
- did the workflow fail because both retry paths were exhausted?

### Statusline display

The TUI statusline should display retry progress whenever the active phase is running under retry.

Required display rule:

- if the active phase has a non-zero current-phase retry count, render the phase as `PHASE (n/3)`

Example:

- `default | gpt-5.4 | themion | flow: LITE | phase: EXECUTE (1/3) | agent: waiting-model`

For previous-phase recovery, the current phase display should still reflect the retry state that applies to the active phase entry. The UI does not need a second counter in the first version unless design work later shows it is necessary. The important requirement is that a user can see that the phase is on a retry path rather than on its first attempt.

**Alternative considered:** show retry state only in verbose logs or transition history. Rejected: retry is active runtime state and should be visible in the normal statusline.

### Lifecycle rules

The runtime should apply the following lifecycle rules consistently:

- entering a workflow's start phase due to initial workflow activation starts with both retry counters at zero
- advancing normally from one phase to the next starts the new phase with both retry counters at zero
- retrying the current phase increments only the current-phase retry counter for that phase
- stepping back to the previous phase increments only the previous-phase retry counter associated with the failed phase's recovery path
- retry counters persist across turn boundaries while that recovery context remains active
- changing workflows resets retry state because retry debt is not valid across workflows
- completing a workflow clears active retry state from the session runtime state
- failing a workflow due to exhausted retries persists the final counters and exhaustion reason

These rules keep retry state bounded to the relevant workflow and phase context.

## Changes by Component

| File | Change |
| ---- | ------ |
| `docs/prd/prd-007-lite-workflow-activation-and-runtime-structure.md` | Update the design notes that currently describe fail-fast no-retry behavior so PRD-007 accurately points to the superseding retry policy from PRD-008. |
| `crates/themion-core/src/workflow.rs` | Extend workflow and phase definitions with explicit recovery-policy metadata, retry counters, retry limits, and validation helpers for current-phase retry versus previous-phase recovery. |
| `crates/themion-core/src/agent.rs` | Detect phase failure-to-continue conditions, apply bounded retry rules, preserve retry state across same-turn and cross-turn workflow progression, and fail the workflow when recovery is exhausted. |
| `crates/themion-core/src/tools.rs` | Extend workflow-state inspection and phase-control tools to expose retry counters, limits, and validated recovery actions. |
| `crates/themion-core/src/db.rs` | Persist active retry state in session workflow metadata and store retry-related workflow transitions and counts in SQLite. |
| `crates/themion-cli/src/tui.rs` | Render retry progress in the `phase:` statusline segment when the active phase is on a retry attempt. |
| `docs/core-ai-engine-loop.md` | Document retry-aware workflow progression, persisted retry state, and how the harness resumes bounded recovery across turns. |
| `docs/architecture.md` | Document retry-aware workflow semantics and statusline presentation at a high level. |
| `docs/README.md` | Add the PRD-008 row and note that it improves the workflow design proposed in PRD-007. |

## Edge Cases

- `CLARIFY` cannot continue and has already retried itself 3 times → mark the workflow failed because no previous phase exists.
- `EXECUTE` fails once for a transient reason → retry `EXECUTE` in place and show `phase: EXECUTE (1/3)`.
- `EXECUTE` remains blocked after 3 current-phase retries but `CLARIFY` recovery is still available → move back to `CLARIFY` and increment previous-phase retry count for the failed `EXECUTE` context.
- `VALIDATE` fails because implementation is incomplete → move back to `EXECUTE` if previous-phase retries for `VALIDATE` are still below 3.
- `VALIDATE` fails for a transient test harness issue → retry `VALIDATE` directly without stepping back to `EXECUTE`.
- the runtime restarts while a phase has retry count `2/3` → resumed workflow state must still show the active retry count rather than resetting to `0/3`.
- the model requests an invalid previous-phase move where no previous phase exists → reject it and keep workflow state unchanged.
- the model requests another retry after the current-phase retry cap is exhausted → reject or fail according to runtime policy, and persist a clear exhaustion result.
- a workflow changes from `LITE` to `NORMAL` while on a retry attempt → reset retry state because retry counts do not carry across workflows.
- older databases or older sessions do not yet have retry metadata → default retry counts to zero and preserve backward compatibility.

## Migration

This feature is additive but extends the workflow persistence model.

If SQLite schema changes are needed, they should be backward-compatible. Older session rows without retry metadata should be interpreted as having zero retries consumed.

PRD-007 should be updated with an implementation note or supersession note so the docs no longer describe lite retry behavior as permanently fail-fast once PRD-008 is the active design.

## Testing

- activate `LITE` and force a recoverable `EXECUTE` failure once → verify: the runtime retries `EXECUTE`, persists retry count `1`, and the statusline shows `phase: EXECUTE (1/3)`.
- force `EXECUTE` to fail 3 times in place and remain unrecoverable there while `CLARIFY` recovery is allowed → verify: the runtime moves back to `CLARIFY` on the next recovery path instead of failing immediately.
- force `VALIDATE` to fail for an incomplete implementation → verify: the runtime can move back to `EXECUTE` if previous-phase retries for `VALIDATE` are still below `3`.
- force a phase with no previous phase, such as `CLARIFY`, to exceed 3 current-phase retries → verify: the workflow is marked failed and the failure reason mentions retry exhaustion.
- inspect `get_workflow_state` during a retry attempt → verify: it returns the active workflow, phase, status, retry counters, retry limits, and allowed next recovery actions.
- interrupt the process while a retried phase is active and resume the session → verify: persisted retry counters are restored and the statusline still reflects the retry attempt.
- inspect `agent_sessions` and `agent_workflow_transitions` after several retries → verify: retry counts and retry-related transition kinds are reconstructable from SQLite.
- switch workflows while a retry attempt is active → verify: the new workflow begins at its start phase with retry counts reset to zero.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: retry-aware workflow runtime, persistence, tools, and statusline changes compile cleanly.
