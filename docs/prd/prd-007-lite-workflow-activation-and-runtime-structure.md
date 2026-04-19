# PRD-007: Lite Workflow Activation and Runtime Structure

- **Status:** Implemented
- **Version:** v0.5.0
- **Scope:** `themion-core` (workflow definitions, prompt assembly, workflow activation detection, workflow control tools, runtime state); `themion-cli` (status display and workflow selection affordances); docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-19

> **Implementation note:** The base `LITE` workflow structure, activation flow, workflow tool surface, prompt-context injection, and statusline visibility are now implemented. Validation and recovery behavior should be interpreted together with [PRD-008](prd-008-workflow-phase-retry-and-recovery-policy.md), which extends the implemented lite flow with bounded retry and previous-phase recovery semantics.

## Goals

- Add a built-in `LITE` workflow that models a compressed fast-feedback pipeline with explicit human-readable phases.
- Define the workflow structure in a way that is easy for a human to read in docs and code, and easy for the runtime and model to manipulate as explicit state.
- Allow workflow activation to be detected automatically from user input markers such as `workflow:lite` without requiring a separate UI flow first.
- Allow the AI model to explicitly activate workflows and move or switch phases through runtime-controlled workflow tools.
- Ensure that activating or changing a workflow always resets the current phase to that workflow's first/start phase.
- Automatically inject workflow instructions plus the current workflow and current phase into prompt assembly as explicit contextual inputs.
- Keep workflow semantics in `themion-core`, with `themion-cli` limited to display and lightweight user-facing affordances.
- Preserve the existing default `NORMAL` workflow behavior when no workflow activation is requested.

## Non-goals

- No attempt in this PRD to implement the full `st-flow` multi-phase process from the upstream stele skill set.
- No general user-defined workflow DSL or TOML-configured arbitrary workflow graph in the first version.
- No full multi-agent orchestration system with separate concurrent model sessions for each lite phase in the first version.
- No requirement that the first implementation perfectly replicate every stele-specific statusline or subprocess behavior.

## Background & Motivation

### Current state

Themion now has explicit workflow and phase runtime state, persisted session-level workflow metadata, turn-level workflow summaries, and workflow transition logging. The built-in implementation currently models the existing behavior as a `NORMAL` workflow with `IDLE` and `EXECUTE` phases.

That foundation makes it possible to add named workflows with distinct execution semantics, and the implemented lite flow now provides a compact workflow that the user or model can intentionally activate for fast prototype-style tasks. Retry-aware validation and recovery policy are specified alongside this implemented flow by PRD-008.

The upstream reference for the desired behavior is `../stele/plugins/steop/skills/st-lite/SKILL.md`, which defines a compressed pipeline:

- `Clarify`
- `Execute`
- `Validate`

Its key semantics are:

- zero-pause default between phases
- one ambiguity gate only in Clarify
- explicit assumptions rather than prolonged investigation
- small-scope, YAGNI-biased execution
- workflow structure that is visible enough to guide both humans and the runtime

### Why the lite workflow needs explicit runtime structure

A workflow like `LITE` is more than a prompt style. It changes execution semantics:

- when the model should keep going automatically
- when the turn should pause for user clarification
- how phase instructions differ between Clarify, Execute, and Validate
- how the model can intentionally switch from one phase to the next using structured runtime controls

If those semantics remain implicit in one long instruction blob, the runtime cannot reliably inspect or manipulate the current workflow state, and the UI cannot clearly show what phase the engine is in.

Themion therefore needs a workflow definition shape that is both:

- readable enough for humans to understand the intended lifecycle quickly
- structured enough for the runtime and model to operate on explicitly

### Why activation should be easy and partially automatic

For a lightweight workflow, activation friction matters. Users should not need a dedicated configuration round-trip just to say “use the lite flow for this request.” A simple inline marker such as `workflow:lite` is enough signal for the runtime to activate the workflow.

At the same time, the model may discover during execution that a workflow should switch phase, pause in Clarify, or explicitly mark validation failure. That means the runtime also needs model-visible tools and automatic workflow-context injection rather than relying only on initial user text parsing.

### Why workflow changes must reset to the start phase

Changing the active workflow is not the same thing as advancing within the current workflow. Each workflow has its own start phase, assumptions, and phase-specific instructions. Carrying the previous workflow's current phase into a newly selected workflow would create invalid state and ambiguous prompt assembly.

The runtime should therefore treat workflow activation or workflow switching as a fresh entry into that workflow:

- select the requested workflow
- set the current phase to that workflow's declared start phase
- persist the workflow change and initial phase together as one authoritative state update
- only allow later phase movement through normal validated phase transitions

This keeps workflow state deterministic and avoids invalid cross-workflow phase carryover such as switching to `LITE` while incorrectly remaining in `EXECUTE` from some other workflow.

## Design

### `LITE` as a built-in workflow

Themion should add a built-in `LITE` workflow in `themion-core` alongside `NORMAL`.

The `LITE` workflow is a compressed fast-feedback path intended for:

- prototypes
- spikes
- mechanical refactors
- quick experiments

Its default phase sequence is:

1. `CLARIFY`
2. `EXECUTE`
3. `VALIDATE`
4. terminal completion or wait state

Its start phase is `CLARIFY`. Whenever `LITE` is activated or selected from another workflow, the runtime must set the current phase to `CLARIFY` before execution continues.

The workflow should remain code-owned in the first version through explicit Rust definitions rather than external configuration.

**Alternative considered:** implement lite semantics as only a prompt preset under the existing `NORMAL` workflow. Rejected: phase progression, stop conditions, activation handling, and model-driven transition handling are runtime semantics, not just prompt flavor.

### Human-readable and agent-manipulable workflow definition shape

The workflow definition should use a structure that can be read almost like a small state machine rather than a deeply implicit set of conditionals.

A practical first-version shape in `themion-core` is:

- stable workflow identifier such as `NORMAL` or `LITE`
- stable phase identifiers such as `IDLE`, `CLARIFY`, `EXECUTE`, `VALIDATE`
- explicit start phase for every workflow definition
- explicit transition rules with small enumerated trigger kinds
- optional per-phase instruction payload or instruction builder
- explicit phase metadata describing whether the phase auto-continues, waits for user input, or is terminal on success/failure
- explicit model-allowed phase transitions that the runtime validates before applying

Conceptually, the lite workflow should be understandable from a compact representation similar to:

```text
workflow: LITE
start_phase: CLARIFY
phases:
  CLARIFY:
    on_ambiguous -> WAIT_USER
    on_ready -> EXECUTE
  EXECUTE:
    on_complete -> VALIDATE
    on_stop -> FAILED
  VALIDATE:
    on_pass -> COMPLETED
    on_fail -> FAILED
```

The exact Rust type names are implementation detail, but the data model should preserve this level of clarity.

This structure serves both audiences:

- humans can inspect the workflow and understand the lifecycle quickly
- the runtime and model can inspect phase names and transition kinds as structured state

When a workflow is activated or changed, the runtime must read that workflow's `start_phase` and make it the current phase immediately. Phase-switch tools are for movement within an already active workflow, not for choosing the initial phase of a newly selected workflow.

#### Compact transition table

The implemented built-in `LITE` workflow follows the following compact state table, with validation and retry semantics interpreted together with PRD-008:

| Current phase/state | Trigger | Next phase/state | Behavior |
| --- | --- | --- | --- |
| _workflow activation_ | user marker or `set_workflow(LITE)` | `CLARIFY` | select `LITE`, reset phase to start phase, persist activation |
| `CLARIFY` | request is clear enough | `EXECUTE` | persist clarify completion and continue in the same logical turn |
| `CLARIFY` | blocking ambiguity remains | `waiting_user` with current phase `CLARIFY` | persist waiting state and stop automatic progression |
| `EXECUTE` | implementation slice complete | `VALIDATE` | persist phase transition and continue automatically |
| `EXECUTE` | execution cannot continue | recovery policy | apply bounded retry or fail according to PRD-008 |
| `VALIDATE` | success criteria pass | `completed` | mark workflow complete and return future turns to default `NORMAL` / `IDLE` behavior unless reactivated |
| `VALIDATE` | success criteria fail | recovery policy | apply bounded retry or previous-phase recovery according to PRD-008 |

Normative rules for this table:

- activation of `LITE` always enters `CLARIFY`; no other initial phase is valid
- `waiting_user` is a workflow status pause, not a separate normal phase in the `LITE` sequence
- `completed` and `failed` are terminal workflow outcomes, not phases that later auto-advance
- no implicit cross-workflow phase carryover is valid during workflow switches
- retry and previous-phase recovery behavior follow PRD-008 rather than free-form looping

**Alternative considered:** store workflow progression in ad hoc code branches inside `agent.rs`. Rejected: the workflow would become harder to reason about, harder to document, and harder for future AI-assisted runtime logic to inspect consistently.

### Lite phase semantics

The implemented lite workflow maps the upstream `st-lite` behavior into themion-native phase semantics while keeping the model simple.

#### `CLARIFY`

Purpose:

- produce a compact brief
- state assumptions explicitly
- classify ambiguity and rough complexity
- decide whether the workflow can proceed immediately

Expected output shape should be concise and machine-readable enough for later phases to reuse, for example:

- objective
- assumptions
- approach confidence
- complexity
- success criteria
- optional groups

Behavioral rules:

- default is zero-pause: proceed automatically when the request is understandable enough to act on
- pause only when there is genuine ambiguity with no reasonable default path
- if the phase pauses, workflow status should move to a waiting state such as `waiting_user`
- if not paused, the workflow should advance to `EXECUTE` in the same logical turn
- if clarify itself becomes stuck, retry behavior follows the bounded retry policy from PRD-008
- the model may explicitly request the phase move through a workflow tool, but the runtime remains responsible for validating that transition

#### `EXECUTE`

Purpose:

- implement the smallest working slice that satisfies the clarify brief
- prefer YAGNI and minimal, targeted change
- avoid unrelated refactors

Behavioral rules:

- proceed immediately from Clarify when not blocked by ambiguity
- allow model/tool round-trips under the phase as needed
- keep scope narrow and assumption-aware
- advance to `VALIDATE` automatically when execution work is complete
- if execution cannot continue, use the bounded retry policy from PRD-008 to retry `EXECUTE` or step back to `CLARIFY` before failing
- allow the model to explicitly request advancement into `VALIDATE` through a workflow tool when it determines the main slice is complete

The upstream skill describes executor fan-out and capped parallel exploration. Themion should model that as future-compatible workflow metadata, but the first implementation may run `EXECUTE` as one active agent loop while preserving the same phase semantics.

**Alternative considered:** require true parallel sub-agents in the first lite implementation. Rejected: the workflow value comes primarily from explicit phase structure and activation semantics; full orchestration can land separately.

#### `VALIDATE`

Purpose:

- check the success criteria from Clarify
- perform a narrow smoke-check of the main path
- report `pass` or `fail`

Behavioral rules:

- on `pass`, mark the workflow completed and return control to the default idle state
- on recoverable `fail`, use the bounded retry policy from PRD-008 to retry validation directly or step back to `EXECUTE`
- on exhausted recovery, mark the workflow failed and stop immediately
- the model may explicitly mark validation pass/fail through a workflow completion tool, subject to runtime validation and persistence rules

### Explicit per-phase contract

In addition to the narrative phase descriptions above, the implemented `LITE` phases should be treated as small runtime contracts. Validation and recovery semantics are augmented by PRD-008.

#### `CLARIFY` contract

**Entry conditions**

- entered when `LITE` is first activated
- entered whenever a new run begins in `LITE`, because `CLARIFY` is the workflow's start phase
- may be re-entered by bounded previous-phase recovery from `EXECUTE` when the runtime determines assumptions or scope need to be refreshed

**Runtime behavior**

- analyze the user's request and derive a compact working brief
- identify assumptions the agent will rely on if it proceeds
- identify whether the request is sufficiently clear to continue without asking the user another question
- prefer proceeding with explicit assumptions over prolonged back-and-forth when a reasonable path exists

**Expected artifacts**

The phase should establish enough structured intent for later phases to act consistently. At minimum, the effective clarify result should contain:

- objective
- assumptions
- success criteria
- rough scope or complexity
- whether the request is blocked on ambiguity

The exact storage shape may be prompt-local in the first version, but the runtime should behave as though this clarify brief exists as the contract output of the phase.

**Allowed transitions**

- `CLARIFY -> EXECUTE` when the request is clear enough to proceed
- `CLARIFY -> waiting_user` when genuine blocking ambiguity remains
- `CLARIFY -> CLARIFY` by bounded retry according to PRD-008
- direct completion from `CLARIFY` should not be the normal path for `LITE`

**Completion criteria**

`CLARIFY` completes only when one of the following becomes true:

- the runtime has enough clarity to proceed into `EXECUTE`, or
- the runtime has determined that user input is required before safe progress is possible

A vague or partially formed internal thought is not enough; the phase must end with either a usable brief or an explicit waiting state.

**Persisted state expectations**

When `CLARIFY` starts or ends, persistence should allow later inspection of:

- workflow = `LITE`
- phase = `CLARIFY`
- whether the phase ended in `EXECUTE` or `waiting_user`
- the transition trigger such as `user_input`, `model_completion`, or `engine_rule`
- retry counts when the phase is entered through recovery

The implementation does not need a new dedicated clarify table in the first version, but transition history should make the result reconstructable.

**Prompt injection expectations**

While `CLARIFY` is active, prompt assembly should inject guidance that tells the model to:

- produce a compact brief
- make assumptions explicit
- ask the user only when ambiguity is genuinely blocking
- avoid over-investigation before execution

#### `EXECUTE` contract

**Entry conditions**

- entered only after `CLARIFY` has completed with a ready-to-proceed outcome
- not entered directly on workflow activation unless a future workflow definition explicitly changes `LITE`'s start phase
- may be re-entered by bounded previous-phase recovery from `VALIDATE`

**Runtime behavior**

- carry out the smallest working slice implied by the clarify brief
- use available tools and model/tool round-trips as needed
- preserve narrow scope and avoid unrelated refactors
- treat clarify assumptions and success criteria as the governing execution brief

**Expected artifacts**

At minimum, `EXECUTE` should produce:

- the actual code, file, or workflow changes being attempted
- enough observable output for `VALIDATE` to assess whether the requested slice was completed
- any notable assumption-driven limitations that affect validation

**Allowed transitions**

- `EXECUTE -> VALIDATE` when the requested slice is complete enough to check
- `EXECUTE -> EXECUTE` by bounded retry according to PRD-008
- `EXECUTE -> CLARIFY` by bounded previous-phase recovery according to PRD-008
- `EXECUTE -> failed` when bounded recovery is exhausted

**Completion criteria**

`EXECUTE` completes only when one of the following becomes true:

- the runtime has produced a concrete result that can be validated against the clarify brief, or
- the runtime determines the execution attempt failed and should terminate the workflow after exhausting valid recovery paths

“Model stopped talking” alone is not sufficient; the phase should end only with either a validation-ready result or an explicit failure state.

**Persisted state expectations**

When `EXECUTE` starts or ends, persistence should allow later inspection of:

- workflow = `LITE`
- phase = `EXECUTE`
- whether the phase ended in `VALIDATE`, `CLARIFY`, or failure
- relevant transition triggers and timing
- retry counts and whether the phase entry was normal, current-phase retry, or previous-phase recovery

Where practical, assistant and tool messages created during this phase should remain attributable to `EXECUTE` through existing workflow/message annotations.

**Prompt injection expectations**

While `EXECUTE` is active, prompt assembly should inject guidance that tells the model to:

- implement the smallest working slice
- stay within the clarify brief
- avoid gold-plating and unrelated cleanup
- prepare the work for immediate validation rather than continuing indefinitely

#### `VALIDATE` contract

**Entry conditions**

- entered only after `EXECUTE` has produced a result that can be checked
- should not be entered directly from workflow activation in the first version

**Runtime behavior**

- compare the execution result against the clarify phase's success criteria
- perform a narrow smoke-check of the main path
- produce a binary result of `pass` or `fail` for the current workflow run
- use the bounded retry policy from PRD-008 when validation can recover

**Expected artifacts**

At minimum, `VALIDATE` should produce:

- validation outcome: `pass` or `fail`
- concise reason or evidence for that outcome
- enough detail for the user to understand what was checked

**Allowed transitions**

- `VALIDATE -> completed` on pass
- `VALIDATE -> VALIDATE` by bounded retry according to PRD-008
- `VALIDATE -> EXECUTE` by bounded previous-phase recovery according to PRD-008
- `VALIDATE -> failed` on exhausted recovery

**Completion criteria**

`VALIDATE` completes only when the workflow has been marked either:

- successful/completed, or
- failed

A partial validation note is not enough; the phase must end with a terminal workflow outcome.

**Persisted state expectations**

When `VALIDATE` starts or ends, persistence should allow later inspection of:

- workflow = `LITE`
- phase = `VALIDATE`
- terminal result = completed or failed
- reason for completion or failure when practical
- retry counts and whether failure was due to exhausted recovery

**Prompt injection expectations**

While `VALIDATE` is active, prompt assembly should inject guidance that tells the model to:

- test against the clarify success criteria
- use a narrow smoke-check mindset
- return a clear `pass` or `fail`
- avoid silently continuing implementation work after a failed check without using the bounded recovery policy

### Workflow activation detection from user input

Themion should support lightweight workflow activation markers in user input. At minimum, the runtime should detect markers of the form:

- `workflow:lite`
- `workflow: lite`

Detection should occur before the main workflow run begins for the turn. When a marker is present:

- the session's active workflow for the new turn becomes `LITE`
- the current phase is set immediately to `LITE`'s start phase, `CLARIFY`
- the marker itself should not need to remain in the user-facing task text given to the model as ordinary content
- the runtime should record the activation source as user input

The parsing should be conservative and explicit. The goal is not fuzzy NLP intent detection; the goal is a reliable inline activation token.

Reasonable first-version rules:

- case-insensitive match for `workflow:lite`
- match anywhere in the submitted line or in a leading control block
- ignore malformed near-matches rather than guessing

If no activation marker is present, the existing `NORMAL` workflow remains the default.

**Alternative considered:** add automatic semantic inference such as “if the user says prototype then switch to lite.” Rejected: that is too implicit for a first version and would create surprising workflow changes.

### Model-visible workflow control tools

Themion should add small workflow-control tools so the model can participate in workflow management explicitly instead of only through prompt wording.

At minimum, the runtime should expose tools in this shape:

- `get_workflow_state`
  - returns current workflow name, phase, status, last-updated metadata, retry counters when applicable, and allowed next transitions when practical
- `set_workflow`
  - activates a named built-in workflow when allowed by runtime policy and resets the current phase to that workflow's start phase
- `set_workflow_phase`
  - requests a move or switch to a specific phase with a trigger/reason within the currently active workflow
- `complete_workflow`
  - marks workflow success or failure with a reason

These tools are not just diagnostic. They are the structured path that allows the model to move or switch phase deliberately.

Runtime policy should remain authoritative. For example:

- unknown workflow names are rejected
- when `set_workflow` succeeds, the runtime must always set the current phase to the selected workflow's start phase rather than preserving the previous phase
- invalid phase transitions are rejected
- `set_workflow_phase` is validated against the currently active workflow and cannot be used to bypass workflow activation rules or retry limits
- completed workflows cannot be arbitrarily resumed unless future policy allows it
- model-requested phase switches must match the workflow definition's allowed transitions

These tools should live in `themion-core` and use the existing workflow persistence path.

**Alternative considered:** let the model “activate” lite or switch phases only by writing phrases such as “I am now entering validate.” Rejected: free-text state mutation is brittle and hard to validate or persist correctly.

### Automatic workflow instruction and context injection

Prompt assembly should automatically inject workflow-aware context for every workflow-aware turn.

At minimum, the injected context should include:

- current workflow name
- current phase name
- workflow status
- whether the workflow was activated by the user through an inline marker or by a workflow-control tool
- allowed next phase transitions when useful
- retry counters and limits when the phase is on a recovery path
- phase-specific instructions for the active phase
- available workflow-control tools and when to use them

For `LITE`, the runtime should automatically inject phase guidance such as:

- in `CLARIFY`, ask the user only if the request is genuinely ambiguous
- in `EXECUTE`, implement the smallest working slice
- in `VALIDATE`, check success criteria and return `pass` or `fail` using bounded recovery when needed

This injection should happen automatically from runtime state. The model should not need to rediscover the current workflow or phase from earlier conversational text.

This should remain a separate prompt input layer or workflow-context input, not a hidden merge into the base system prompt.

### Turn and session behavior

The workflow system should support both same-turn advancement and cross-turn waiting.

For `LITE`, the expected default behavior is:

- activate `CLARIFY` at turn start when the user requests lite
- automatically inject the current workflow and phase context before each model request in the workflow run
- continue automatically into `EXECUTE` and `VALIDATE` in the same logical turn when no pause condition occurs
- if Clarify finds genuine ambiguity, stop in a waiting state and persist the active workflow/phase for the next user turn
- if the model explicitly requests a valid next phase through a workflow tool, apply that switch and continue using the new phase instructions
- if the model changes the active workflow through `set_workflow`, reset the phase immediately to the new workflow's start phase before continuing
- if a phase cannot continue, apply the bounded retry and previous-phase recovery policy from PRD-008
- if Validate passes, complete the workflow and return the session to `NORMAL` / `IDLE` semantics for subsequent turns unless the user activates lite again
- if recovery is exhausted, mark the workflow failed and surface the failure clearly

This keeps lite fast for the common case while still using the persistent workflow state already introduced by PRD-006.

### Status line and user-visible state

The TUI status line should continue surfacing workflow and phase state. During lite execution, examples should look like:

- `... | flow: LITE | phase: CLARIFY | agent: waiting-model`
- `... | flow: LITE | phase: EXECUTE | agent: running-tool`
- `... | flow: LITE | phase: VALIDATE | agent: waiting-model`

If a phase is on a retry attempt, the status line should render retry progress in the phase segment, for example:

- `... | flow: LITE | phase: EXECUTE (1/3) | agent: waiting-model`

If the workflow pauses for ambiguity, the persisted state should remain visible until the user responds.

After any successful workflow change, the status line should reflect the newly selected workflow together with that workflow's start phase immediately, before any later phase transitions occur.

The CLI may later grow explicit commands for selecting workflows, but this PRD only requires inline activation markers and correct display of the resulting state.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/workflow.rs` | Add the built-in `LITE` workflow definition, explicit start-phase metadata, explicit phase/transition metadata for `CLARIFY`, `EXECUTE`, and `VALIDATE`, helper APIs for human-readable inspection, and validation for model-requested phase switches. Retry and previous-phase recovery metadata are further refined by PRD-008. |
| `crates/themion-core/src/agent.rs` | Detect inline workflow activation markers such as `workflow:lite`, activate `LITE` at turn start, reset the current phase to the selected workflow's start phase on workflow change, automatically inject workflow instructions plus current workflow/phase context into prompt assembly before each model request, and honor same-turn auto-advance versus waiting-user behavior. |
| `crates/themion-core/src/tools.rs` | Add workflow-control tool definitions and handlers for workflow inspection, activation, and model-requested phase switching/completion, with `set_workflow` resetting phase to the selected workflow's start phase and exposing retry state where applicable. |
| `crates/themion-core/src/db.rs` | Reuse the workflow persistence model from PRD-006 and ensure activation source, workflow-start phase selection, model-requested phase changes, bounded recovery metadata, and completion/failure transitions are recorded for lite runs. |
| `crates/themion-cli/src/tui.rs` | Continue rendering active workflow/phase state during lite execution, preserve paused workflow visibility between turns when lite stops in Clarify, and render retry count in the phase display when applicable. |
| `docs/architecture.md` | Document built-in `LITE` workflow behavior, inline activation markers, workflow changes resetting to start phase, automatic workflow-context injection, retry-aware recovery semantics, and workflow-control tool semantics at a high level. |
| `docs/core-ai-engine-loop.md` | Document workflow activation detection, automatic injection of workflow instructions and current workflow/phase context, workflow changes resetting to the selected workflow's start phase, bounded recovery behavior, and how lite progresses across phases in one turn or waits across turns. |
| `docs/README.md` | Keep the PRD table aligned with PRD-007 and the newer retry policy from PRD-008. |

## Edge Cases

- User includes `workflow:lite` plus normal task text in one message → verify runtime activates `LITE`, sets phase to `CLARIFY`, and still preserves the task content for the workflow run.
- User writes a malformed token such as `workflow-lite` or `workflow=l1te` → ignore it rather than guessing.
- User activates lite for a genuinely ambiguous request → stop after `CLARIFY`, persist `LITE` plus a waiting status, and resume from that workflow state on the next turn.
- User activates lite on a straightforward request with no ambiguity → workflow advances through `CLARIFY`, `EXECUTE`, and `VALIDATE` automatically in one logical turn.
- The model explicitly requests a valid phase switch such as `CLARIFY -> EXECUTE` or `EXECUTE -> VALIDATE` → apply the switch, persist the transition, and inject the new phase instructions on the next model call.
- The model changes from one workflow to another through `set_workflow` while currently in a non-start phase → activate the new workflow and reset immediately to that workflow's start phase rather than carrying over the old phase name.
- `EXECUTE` or `VALIDATE` cannot continue → apply bounded retry or previous-phase recovery from PRD-008 instead of failing immediately when recovery remains available.
- A phase exhausts both current-phase retry and previous-phase recovery limits → mark the workflow failed and persist the exhaustion reason.
- The model attempts an invalid manual phase transition through a workflow-control tool → reject it cleanly and keep persisted workflow state consistent.
- The model tries to activate an unknown workflow name → return a structured tool error.
- A session is interrupted while lite is in `EXECUTE` or `VALIDATE` under retry → persisted workflow state should still reflect the last active phase and retry counts for inspection or future recovery policy.
- Workflow-control tools are unavailable on an older provider session or older database rows exist without lite metadata → degrade gracefully and preserve default `NORMAL` behavior.
- User includes multiple explicit markers such as `workflow:lite workflow:normal` in one input → the implementation should choose a deterministic rule, preferably first valid marker wins, and record the activation source clearly.

## Migration

This feature is additive.

Existing sessions and turns continue to use `NORMAL` by default when no workflow activation marker is present. Lite activation should not require config migration.

If the implementation introduces new workflow metadata such as activation source, richer transition reasons, or retry-state fields, the migration should be backward-compatible and preserve existing workflow/session history.

Older sessions with no lite-related workflow state should continue to be interpreted under the current default `NORMAL` / `IDLE` behavior.

## Testing

- submit a prompt containing `workflow:lite` and a straightforward task → verify: the runtime activates `LITE` for the turn and starts in `CLARIFY`.
- submit a prompt containing `workflow: lite` with mixed casing such as `Workflow:LiTe` → verify: activation detection is case-insensitive and still selects `LITE`.
- submit a prompt without a workflow marker → verify: the session remains on the default `NORMAL` workflow.
- activate lite for a clear request → verify: the workflow advances from `CLARIFY` to `EXECUTE` to `VALIDATE` automatically within one logical turn.
- activate lite for a genuinely ambiguous request → verify: the workflow stops after `CLARIFY`, persists a waiting-user state, and the next user turn resumes from the active lite workflow rather than resetting to `NORMAL` immediately.
- use the workflow activation tool with a valid built-in workflow name → verify: the runtime updates workflow state, resets phase to that workflow's start phase, and records the transition in SQLite.
- switch workflows while currently in a later phase of another workflow → verify: the newly selected workflow begins at its declared start phase rather than preserving the old phase.
- inspect workflow-aware prompt assembly during an active lite run → verify: the model receives automatically injected current workflow/phase context plus lite-specific phase instructions as separate contextual input.
- call the workflow inspection tool during a lite run → verify: it returns the current workflow name, phase, status, retry state when applicable, and allowed next transitions from runtime state.
- use the workflow phase-switch tool with a valid next phase → verify: the runtime applies the phase move, persists the transition, and the following model call receives the new phase instructions.
- use the workflow activation/control tool with an invalid workflow or invalid phase transition → verify: the tool returns a structured error and persisted workflow state remains unchanged.
- run a lite validation that fails its declared success criteria while recovery remains available → verify: the runtime applies bounded retry or previous-phase recovery according to PRD-008 rather than failing immediately.
- view the TUI during a lite retry attempt → verify: the status line shows `flow: LITE` with the active phase rendered as `PHASE (n/3)` when current-phase retry is active.
- inspect `agent_sessions`, `agent_turns`, `agent_messages`, and `agent_workflow_transitions` after a lite run → verify: activation, phase progression, start-phase reset on workflow change, retry state when applicable, and completion or failure are reconstructable from persisted workflow metadata.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: lite workflow definitions, activation logic, tools, automatic workflow-context injection, bounded recovery behavior, and UI wiring compile cleanly.
