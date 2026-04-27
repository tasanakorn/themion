# PRD-015: User-Feedback-Required Phase Result

- **Status:** Implemented
- **Version:** v0.8.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

## Goals

- Add a workflow phase result that explicitly means the current phase cannot proceed without user feedback or a user decision.
- End the current turn cleanly when that phase result is set, instead of entering automatic retry behavior.
- Preserve the distinction between ordinary phase failure and a blocked-on-user state that the agent cannot resolve by itself.
- Make the blocked state visible in workflow state, status events, and prompt context so both the model and the user can understand why execution paused.
- Allow the next user turn to resume the same workflow from the same phase after the requested user feedback is provided.

## Non-goals

- No new general-purpose approval framework beyond the workflow/runtime behavior described here.
- No change to the meaning of `passed` or `failed` for phases that are still resolvable through autonomous retry.
- No requirement to add a brand-new workflow status if the existing `waiting_user` status remains sufficient.
- No automatic synthesis of the human question by the runtime itself beyond surfacing the assistant's normal message and workflow state.
- No redesign of workflow retry limits or retry-counter storage outside the behavior needed for the new phase result.

## Background & Motivation

### Current state

Themion's workflow runtime currently tracks phase results as:

- `pending`
- `passed`
- `failed`

The runtime also already has a workflow-level `waiting_user` status. That status is suitable when the workflow is paused for user input, but the phase-result model still lacks a way to say why the pause happened.

Today, a phase that is blocked on user feedback has to be represented indirectly, typically by using `failed` and then relying on surrounding logic or assistant prose to imply that the failure is not autonomously recoverable.

That creates an awkward mismatch:

- `failed` usually implies the runtime may retry the current phase or move to previous-phase recovery
- but a user-feedback dependency is not something the agent can fix alone
- so auto-retry is wasted work and can produce confusing repeated attempts

The workflow model already distinguishes `running` from `waiting_user`, so the missing piece is an explicit phase result that says the current phase is waiting on user feedback rather than having failed in a machine-recoverable way.

## Design

### New phase result: user_feedback_required

Themion should add a new phase result value:

- `user_feedback_required`

This result means the current phase cannot make further meaningful progress until a user provides information, approval, judgment, or another non-automatable decision.

Typical cases include:

- the task depends on a product or UX decision that is not inferable from repository context
- implementation produced an output that requires human review or acceptance before proceeding
- validation found an ambiguity that only the user can resolve
- the agent needs a user choice among multiple viable options before continuing

This result is distinct from `failed`.

- `failed` means the phase did not succeed and may still be eligible for autonomous retry or previous-phase recovery
- `user_feedback_required` means retry is not useful until the human responds

**Alternative considered:** reuse `failed` plus `waiting_user` status and document the convention informally. Rejected: that keeps the runtime semantics ambiguous and does not give retry logic a first-class signal to stop autonomous recovery.

### Runtime behavior when human feedback is required

When the current phase result becomes `user_feedback_required`, the runtime should:

- keep the workflow on the current phase
- set workflow status to `waiting_user`
- end the current turn without auto-retrying the current phase
- end the current turn without retrying the previous phase
- preserve retry counters rather than incrementing them for this condition
- keep the workflow resumable from the same phase on a future user turn

This should behave as an intentional pause, not as a failed workflow.

The model should still provide a normal assistant response in the same turn explaining what feedback is needed from the user.

**Alternative considered:** convert `human_feedback_required` into a hard workflow failure and force users to reactivate or restart the workflow manually. Rejected: the blocked state is expected and resumable, so failing the workflow would add unnecessary friction.

### Interaction with workflow status

The existing `waiting_user` workflow status should remain the runtime-level status for this case.

Normative mapping:

- `phase_result=user_feedback_required`
- `status=waiting_user`

This preserves the current distinction between:

- phase result = what happened in the current phase
- workflow status = whether the workflow is actively running or paused

Prompted workflow context should surface both values so the model can see that the phase is still the same one and that progress is paused on human input.

**Alternative considered:** add a separate workflow status such as `waiting_human_feedback`. Rejected: the existing `waiting_user` status already expresses the runtime pause well enough; the missing specificity belongs in the phase result.

### Tooling and workflow-control contract

`workflow_set_phase_result` should accept the new result value:

- `pending`
- `passed`
- `failed`
- `user_feedback_required`

When the model sets `user_feedback_required`, runtime validation should immediately move the workflow into the paused `waiting_user` state without requiring a separate retry decision.

`workflow_set_phase` should continue to require `phase_result=passed` for forward phase transitions.

`workflow_complete(outcome="completed")` should still require `phase_result=passed`.

`workflow_complete(outcome="failed")` should remain available for unrecoverable failures, but it should not be required for human-feedback pauses.

**Alternative considered:** add a separate workflow tool like `workflow_wait_for_user(...)`. Rejected: the runtime already has a phase-result mechanism, and this new state fits best as a richer result value rather than as a separate side-channel tool.

### Retry policy changes

Retry logic should treat `user_feedback_required` as non-retryable.

That means:

- no current-phase retry should be scheduled
- no previous-phase retry should be scheduled
- retry exhaustion logic should not trigger
- no retry counters should be incremented because the phase did not fail in an autonomously recoverable way

This is the core behavioral reason for the change: if the phase explicitly requires human feedback, the agent cannot resolve it by itself, so auto-retry should stop immediately.

**Alternative considered:** allow one automatic retry before pausing for user input. Rejected: if the phase result has already been explicitly set to human-feedback-required, the runtime should trust that signal rather than second-guessing it with more autonomous work.

### Resume behavior on the next user turn

When the workflow is paused with:

- `status=waiting_user`
- `phase_result=user_feedback_required`

then the next user message should resume the workflow in the same phase unless the user explicitly changes workflows.

On resume, the runtime should clear the blocked phase result back to `pending` before the phase continues, so the model can act on the newly provided human input.

The phase should not be treated as a retry entry unless the user or runtime explicitly invokes retry behavior for another reason.

**Alternative considered:** preserve `user_feedback_required` until the model explicitly clears it with another tool call. Rejected: once a new user turn begins, the purpose of the pause has been satisfied enough for the phase to resume normal processing, so resetting to `pending` keeps the runtime simpler.

### Prompt and statusline visibility

Prompted workflow context should include the new phase result value when present, for example:

> Workflow context: flow=LITE phase=CLARIFY status=waiting_user phase_result=user_feedback_required ...

The TUI/status surfaces that already show workflow status and phase result should render the new value directly rather than collapsing it into `failed` or `pending`.

Status events should also narrate the transition clearly, for example by emitting a phase-result update and any accompanying workflow-status change.

This helps the user understand that the turn ended because their feedback is needed, not because the workflow crashed or silently gave up.

**Alternative considered:** show only `waiting_user` in user-facing UI and keep the specific phase result internal. Rejected: hiding the reason for the wait would weaken debuggability and make the new runtime behavior harder to understand.

### Documentation expectations

The docs should describe this as a workflow/runtime refinement:

- phase results now include `user_feedback_required`
- that result maps to a paused `waiting_user` workflow state
- it is non-retryable and ends the current turn
- the next user turn resumes from the same phase

The workflow diagrams and state descriptions should be updated where they currently imply that all non-passing outcomes flow into retry or failure handling.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/workflow.rs` | Add `user_feedback_required` to `PhaseResult`, update string conversion helpers, and refine workflow/retry logic so this result pauses the workflow in `waiting_user` without incrementing retry counters or triggering auto-retry. |
| `crates/themion-core/src/tools.rs` | Extend `workflow_set_phase_result` validation so the tool accepts `user_feedback_required` and returns the updated paused workflow state. |
| `crates/themion-core/src/agent.rs` and related runtime flow code | Ensure end-of-turn workflow evaluation treats `user_feedback_required` as a turn-ending wait-for-user condition rather than a retryable failure, and resume the same phase on the next user turn by resetting the blocked phase result to `pending`. |
| `crates/themion-cli/src/` TUI/status rendering | Render the new phase result in statusline and status events so users can see that the workflow is paused for human feedback rather than failed. |
| `docs/engine-runtime.md` | Document the new phase result, its mapping to `waiting_user`, its non-retryable semantics, and the resume behavior on the next user turn. |
| `docs/architecture.md` | Update workflow/runtime descriptions and diagrams where phase-result handling is summarized. |
| `docs/README.md` | Add this PRD to the index and keep its status aligned with implementation progress. |

## Edge Cases

- the model sets `user_feedback_required` but does not provide a user-facing explanation → the runtime should still pause, but prompting/docs should continue to instruct the model to end the turn with a clear request for feedback.
- the user replies with unrelated input while the workflow is paused → the runtime should still resume the same phase, and the model can decide whether the new input resolves the block or whether it still needs the missing feedback.
- the user explicitly switches workflows while paused → the explicit workflow change should take precedence over resuming the paused phase.
- a repository-local instruction file wants stricter approval behavior → repository-local guidance should still be able to require more explicit reporting while using the same runtime pause semantics.
- the current phase was already near retry exhaustion before human feedback became required → the pause should not consume the remaining retry budget.
- the model incorrectly uses `user_feedback_required` for a situation it could have resolved autonomously → the runtime should still honor the explicit signal; prompt guidance and future tuning can address misuse separately.
- `NORMAL` workflow turns that do not use multi-phase progression may still set the new phase result → the runtime should treat it as a wait-for-user pause on the current phase rather than forcing `NORMAL` into retry semantics it does not use.

## Migration

This change is additive at the workflow-model level.

Existing sessions or persisted workflow records that use only `pending`, `passed`, and `failed` remain valid. The new `user_feedback_required` value simply becomes an additional possible phase result after upgrade.

If any persisted workflow-state serialization assumes the phase-result enum is closed to the original three values, that serialization path should be updated in a backward-compatible way so old rows still read correctly and new rows can store the new value.

No user config migration is required.

## Testing

- set `workflow_set_phase_result(result="user_feedback_required")` in a running workflow phase → verify: workflow state changes to `status=waiting_user` while keeping the same phase and reporting `phase_result=user_feedback_required`.
- set `user_feedback_required` during a phase that would otherwise be eligible for retry → verify: the turn ends without current-phase retry, previous-phase retry, or retry-counter increments.
- inspect workflow status events after setting `user_feedback_required` → verify: the event stream shows the phase-result update and paused workflow state clearly.
- submit the next user turn after a paused user-feedback-required phase → verify: the workflow resumes from the same phase and the phase result is reset to `pending` before normal continuation.
- switch workflows explicitly while paused for human feedback → verify: the explicit workflow activation takes precedence over same-phase resume.
- inspect prompt/workflow context in a paused session → verify: the model sees both `status=waiting_user` and `phase_result=user_feedback_required`.
- exercise a normal retryable failure using `phase_result=failed` → verify: existing retry behavior remains unchanged for true failures.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: workflow-model, TUI, and docs changes compile cleanly.

## Implementation notes

This PRD is implemented in the workflow/runtime model.

Implemented behavior includes:

- `crates/themion-core/src/workflow.rs` adds `user_feedback_required` to `PhaseResult` and preserves `waiting_user` as the paused workflow status
- `crates/themion-core/src/tools.rs` accepts `user_feedback_required` in `workflow_set_phase_result`
- `crates/themion-core/src/agent.rs` treats `user_feedback_required` as a turn-ending wait-for-user condition rather than a retryable failure
- the next user turn resumes the same phase by resetting the blocked phase result to `pending` unless the user explicitly changes workflows

The shipped implementation matches the intended workflow behavior closely enough that this PRD should be treated as an implemented product contract rather than a pending proposal.
