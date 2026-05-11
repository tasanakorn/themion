# PRD-122: Consolidate Workflow Tools into `workflow_get_state` and `workflow_set`

- **Status:** Implemented
- **Version:** v0.75.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-11

## Implementation status

Landed in `v0.75.0`.

## Summary

- Themion currently exposes five workflow tools plus older workflow-tool aliases for one small workflow-control surface.
- Replace the four mutating workflow tools with one canonical patch-style tool: `workflow_set`.
- Keep `workflow_get_state` as the separate read tool. Do not merge read and mutation into one tool.
- Support only the useful combined mutation cases from this discussion, not every possible field mix.
- Keep the merged tool narrow: remove the current unused workflow mutation `reason` fields and use clear field names with explicit validation rules.

## Goals

- Reduce the model-facing workflow tool surface from five tools to two.
- Keep one read tool and one mutation tool so the contract stays easy to understand.
- Support one-call workflow updates for the practical combined cases that matter most:
  - mark a phase passed and advance phase
  - mark a phase passed and complete the workflow
- Preserve current workflow runtime behavior unless this PRD explicitly changes the tool contract.
- Remove workflow mutation request fields that do not affect behavior.
- Keep the workflow tool surface narrow to this consolidation task rather than broad workflow redesign.

## Non-goals

- Do not merge read and mutation into one generic `workflow` tool.
- Do not redesign the underlying workflow engine, retry policy, or persisted workflow schema.
- Do not add new workflow concepts beyond the merged mutation tool.
- Do not expose arbitrary direct setting of all workflow-state fields.
- Do not move workflow logic into TUI or other presentation code.
- Do not use this PRD to change unrelated workflow semantics or retry rules.

## Background & Motivation

### Current state

Themion currently exposes these workflow tools:

- `workflow_get_state`
- `workflow_set_active`
- `workflow_set_phase`
- `workflow_set_phase_result`
- `workflow_complete`

The current implementation also still accepts older alias names in dispatch paths.

This surface is larger than the real concept count. The workflow domain here has two main intents:

- inspect workflow state
- change workflow state

The current split also makes some common workflow follow-up work take two calls. A practical example is:

1. set `phase_result="passed"`
2. move to the next phase

The current workflow mutation schemas also carry `reason` fields. In the current implementation, those fields do not drive validation or runtime behavior. They add schema overhead without adding control value.

### Why this matters now

This workflow surface is small enough that one read tool plus one mutation tool is the clearest stable shape.

A single combined get/set tool was considered and rejected. Read and mutation are different intents. Keeping them separate avoids a branchy mixed schema and keeps the contract easier for the model.

## Design

### 1. Active workflow tool surface

The active workflow tool surface must become:

- `workflow_get_state`
- `workflow_set`

Required behavior:

- `workflow_get_state` remains the only read tool
- `workflow_set` becomes the only mutating workflow tool
- remove these names from the exported tool catalog and active dispatch paths:
  - `workflow_set_active`
  - `workflow_set_phase`
  - `workflow_set_phase_result`
  - `workflow_complete`
  - `get_workflow_state`
  - `set_workflow`
  - `set_workflow_phase`
  - `set_phase_result`
  - `complete_workflow`
- prompt guidance, tool labels, and docs must teach only the new two-tool surface after implementation lands

This PRD intentionally removes the old split mutating names rather than keeping a long-lived compatibility alias set.

### 2. `workflow_get_state` contract

`workflow_get_state` should stay small and read-only.

Required request schema:

```json
{
  "name": "workflow_get_state",
  "description": "Get workflow state and allowed transitions.",
  "parameters": {
    "type": "object",
    "properties": {},
    "required": []
  }
}
```

Required response fields:

- `workflow`
- `phase`
- `status`
- `phase_result`
- `agent`
- `last_updated_turn_seq`
- `retry_state`
- `allowed_next_phases`
- `allowed_retry_current_phase`
- `allowed_retry_previous_phase`
- `previous_phase`
- `phase_instructions`

This preserves the current inspection use case and keeps state refresh separate from mutation.

### 3. `workflow_set` request schema

`workflow_set` should be a narrow patch-style mutation tool.

Required request schema:

```json
{
  "name": "workflow_set",
  "description": "Apply workflow state changes.",
  "parameters": {
    "type": "object",
    "properties": {
      "workflow": {
        "type": "string",
        "description": "Workflow name to activate."
      },
      "phase_result": {
        "type": "string",
        "enum": ["passed", "failed", "user_feedback_required"],
        "description": "Current phase result."
      },
      "phase": {
        "type": "string",
        "description": "Next phase in the active workflow."
      },
      "workflow_status": {
        "type": "string",
        "enum": ["completed", "failed"],
        "description": "Terminal workflow status to set."
      }
    },
    "required": []
  }
}
```

Required validation rules:

- require at least one field
- remove `reason` from the public workflow mutation contract
- do not accept `pending` as an input `phase_result`
- do not accept `running`, `waiting_user`, or `interrupted` as input `workflow_status` values
- use `workflow_status` instead of `outcome` because it makes the scope explicit against `phase_result`

This tool is patch-style because it must support a small number of useful combined mutations in one call.

### 4. `workflow_set` field meaning

The request fields must stay narrow and distinct.

Field meaning:

- `workflow` = activate a workflow and reset to that workflow's start phase
- `phase_result` = set the current phase result
- `phase` = move to another phase in the currently active workflow
- `workflow_status` = set the workflow-level terminal status

Scope distinction:

- `phase_result` is about the current phase
- `workflow_status` is about the whole workflow

This distinction is practical and must remain explicit in the merged tool.

### 5. Allowed request shapes

`workflow_set` must support a small explicit set of request shapes. It must not behave like a general free-form workflow patch bag.

Allowed single-field requests:

- `{"workflow":"NORMAL"}`
- `{"phase_result":"passed"}`
- `{"phase_result":"failed"}`
- `{"phase_result":"user_feedback_required"}`
- `{"phase":"VALIDATE"}`
- `{"workflow_status":"completed"}`
- `{"workflow_status":"failed"}`

Allowed combined requests:

- `{"phase_result":"passed","phase":"VALIDATE"}`
- `{"phase_result":"passed","workflow_status":"completed"}`
- `{"phase_result":"failed","workflow_status":"failed"}`

Rejected requests:

- empty request with none of the fields
- any request that mixes `workflow` with another field
- any request that includes both `phase` and `workflow_status`
- `{"phase_result":"failed","phase":...}`
- `{"phase_result":"user_feedback_required","phase":...}`
- `{"phase_result":"failed","workflow_status":"completed"}`
- `{"phase_result":"user_feedback_required","workflow_status":...}`
- any request that violates the existing workflow transition or completion rules

This keeps the combined cases practical and reviewable.

### 6. Exact mutation semantics by request shape

The merged tool must preserve current runtime behavior where this PRD does not explicitly narrow it.

#### `workflow` only

Behavior:

- normalize and validate the workflow name using the existing workflow helpers
- activate the requested workflow
- reset to that workflow's start phase
- set `status=running`
- set `phase_result=pending`
- set `agent=master` using the current default-agent behavior
- reset retry state

This is the direct replacement for `workflow_set_active`.

#### `phase_result` only

Behavior:

- keep current workflow and phase
- keep current agent
- keep retry state unchanged
- set the requested phase result
- preserve current status for `passed` and `failed`
- when `phase_result="user_feedback_required"`, set `status="waiting_user"`

This preserves the intended paused-for-user behavior of `user_feedback_required` while keeping the request surface narrow.

#### `phase` only

Behavior:

- validate the transition with the existing workflow transition helper
- keep current workflow and agent
- set the new phase
- set `status=running`
- reset `phase_result` to `pending`
- reset retry state

This is the direct replacement for `workflow_set_phase`.

Important scope guard:

- this PRD does not add a new phase-result gate to plain `phase` transitions
- phase-only transitions should continue to use the current transition validation behavior unless a separate PRD changes that rule

#### `workflow_status` only

Behavior:

- keep current workflow, phase, agent, and retry state
- when `workflow_status="completed"`, require the current `phase_result` to already be `passed`
- when `workflow_status="failed"`, set workflow failure directly

This is the direct replacement for `workflow_complete` using a clearer field name.

#### `phase_result="passed"` + `phase`

Behavior:

- validate the requested phase transition using the current workflow transition rules
- treat the `passed` result as part of the same logical update
- final returned state must reflect the new phase, not the intermediate old-phase result
- final state after the mutation:
  - `phase=<new phase>`
  - `status=running`
  - `phase_result=pending`
  - retry state reset

Important clarification:

- this combined request is a one-call replacement for “mark current phase passed, then move phase”
- the intermediate `passed` result is not a separate final state that remains visible after the phase move
- the final state should match current phase-change behavior for the new phase

#### `phase_result` + `workflow_status`

Supported combinations:

- `passed + completed`
- `failed + failed`

Behavior:

- apply both changes as one logical mutation
- `passed + completed` ends the workflow successfully
- `failed + failed` ends the workflow in failure
- preserve current workflow, phase, agent, and retry state

Rejected combinations:

- `failed + completed`
- `user_feedback_required + completed`
- `user_feedback_required + failed`

### 7. Atomic apply model

One `workflow_set` call must behave as one logical mutation.

Required behavior:

- validate the full request before mutating runtime state
- if validation fails, do not partially apply any field
- return a clear validation error for invalid shapes or invalid transitions

Required implementation order:

- if `workflow` is present, activation is exclusive and no other mutation is allowed
- otherwise evaluate the request shape first, then build one post-mutation workflow-state result
- do not implement `workflow_set` as several loosely chained public mutations with partially visible intermediate results

This matters most for the combined shapes.

### 8. Response shape for `workflow_set`

`workflow_set` must return the post-mutation workflow core state needed by both the runtime and the model.

Required response fields:

- `workflow`
- `phase`
- `status`
- `phase_result`
- `agent`
- `retry_state`

Recommended additional fields:

- `allowed_next_phases`
- `allowed_retry_current_phase`
- `allowed_retry_previous_phase`
- `previous_phase`
- `phase_instructions`

Response rules:

- do not include `reason`
- do not make `last_updated_turn_seq` part of the `workflow_set` mutation contract
- if implementation reuses the same helper as `workflow_get_state`, `last_updated_turn_seq` may be omitted or may reflect the pre-mutation stored value; callers must not rely on it as the mutation acknowledgement field

The important requirement is that one `workflow_set` response must expose the new workflow state clearly without requiring an immediate second tool call.

### 9. Runtime application and event behavior

The merged tool changes the workflow tool contract, so runtime application logic must also merge.

Required behavior:

- replace the current per-tool workflow mutation application branches with one `workflow_set` application path
- infer the runtime side effects from the request shape and returned state
- continue to persist workflow state after successful workflow mutation
- continue to record workflow transitions using the existing transition kinds when they still fit:
  - workflow activation → `WorkflowStarted`
  - phase move → `PhaseStarted`
  - waiting for user after `user_feedback_required` → `WaitingUser`
  - workflow completion → `WorkflowCompleted`
  - workflow failure → `WorkflowFailed`
- do not emit duplicate transition records just because one merged request replaced two old tool calls

For the combined `passed + phase` case, the runtime should treat the mutation as one phase-advance event with the final new-phase state rather than trying to preserve a separate intermediate visible `passed` state.

### 10. Keep current workflow semantics outside the tool merge

This PRD is a tool-surface consolidation, not a workflow-policy rewrite.

Required behavior:

- preserve current workflow definitions and start phases
- preserve current transition validation helpers
- preserve current retry-state reset behavior on workflow activation and phase change
- preserve current workflow completion success rule: success requires a passed phase result
- preserve `user_feedback_required` as a phase-level result that maps to `waiting_user`
- do not add new workflow statuses or retry modes in this PRD

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Replace the four split mutating workflow tool schemas and the legacy workflow aliases with one `workflow_set` schema, keep `workflow_get_state`, remove workflow mutation `reason` fields, validate the exact allowed request shapes, and return the post-mutation workflow state. |
| `crates/themion-core/src/agent.rs` | Replace per-tool workflow mutation application with one `workflow_set` path, keep workflow-state persistence and transition recording correct for merged requests, and update injected workflow tool guidance text. |
| `crates/themion-cli/src/tui.rs` | Update workflow tool-call display labels to the new two-tool surface and remove labels for removed workflow aliases. |
| workflow tool tests | Replace old per-tool schema and dispatch expectations with the new two-tool surface and add exact request-shape tests for valid and invalid combined updates. |
| `docs/engine-runtime.md` | Document the active workflow tool surface as `workflow_get_state` plus `workflow_set`, including the narrow combined-update rules. |
| `docs/README.md` | Track this PRD and later update status when implemented. |

## Implementation Notes

Implemented in v0.75.0. Themion now exposes only `workflow_get_state` and `workflow_set` in the active workflow tool family. The landed `workflow_set` contract uses the narrow `{ workflow?, phase_result?, phase?, workflow_status? }` shape, removes old workflow aliases and unused mutation `reason` fields, preserves `workflow_get_state` response metadata, supports the merged `phase_result + phase` and `phase_result + workflow_status` cases, and keeps runtime persistence plus transition recording coherent through one workflow mutation path. `user_feedback_required` now returns the intended paused `status=waiting_user` state through the merged tool, and the TUI plus runtime guidance now teach only the two-tool workflow surface.

## Edge Cases

- call `workflow_set({})` → verify: clear validation error because at least one field is required.
- call `workflow_set({"workflow":"NORMAL","phase":"EXECUTE"})` → verify: clear validation error because workflow activation is exclusive.
- call `workflow_set({"phase_result":"user_feedback_required"})` → verify: phase result changes and workflow status becomes `waiting_user`.
- call `workflow_set({"phase_result":"user_feedback_required","phase":"EXECUTE"})` → verify: clear validation error because waiting-for-user and phase advance are incompatible in one call.
- call `workflow_set({"phase_result":"passed","phase":"VALIDATE"})` → verify: one call succeeds and final state is `phase="VALIDATE"`, `status="running"`, `phase_result="pending"`.
- call `workflow_set({"phase_result":"failed","phase":"VALIDATE"})` → verify: clear validation error because this combined shape is not supported.
- call `workflow_set({"phase_result":"passed","workflow_status":"completed"})` → verify: one call succeeds and final state is completed with `phase_result="passed"`.
- call `workflow_set({"phase_result":"failed","workflow_status":"failed"})` → verify: one call succeeds and final state is failed with `phase_result="failed"`.
- call `workflow_set({"phase_result":"failed","workflow_status":"completed"})` → verify: clear validation error because the combination is contradictory.
- call `workflow_set({"workflow_status":"completed"})` when current `phase_result` is not `passed` → verify: completion is rejected.
- call `workflow_set({"phase":"VALIDATE","workflow_status":"failed"})` → verify: clear validation error because phase change and terminal workflow change are not allowed together.
- inspect exported tool definitions after implementation → verify: only `workflow_get_state` and `workflow_set` remain in the active workflow tool family.
- inspect prompt/runtime guidance after implementation → verify: it no longer teaches removed workflow tool names or aliases as current behavior.

## Migration

This is a workflow tool-surface consolidation change. No database migration is required.

Migration mapping:

- `workflow_set_active({ workflow })` → `workflow_set({ workflow })`
- `workflow_set_phase({ phase })` → `workflow_set({ phase })`
- `workflow_set_phase_result({ result:"passed" })` → `workflow_set({ phase_result:"passed" })`
- `workflow_set_phase_result({ result:"failed" })` → `workflow_set({ phase_result:"failed" })`
- `workflow_set_phase_result({ result:"user_feedback_required" })` → `workflow_set({ phase_result:"user_feedback_required" })`
- `workflow_complete({ outcome:"completed" })` → `workflow_set({ workflow_status:"completed" })`
- `workflow_complete({ outcome:"failed" })` → `workflow_set({ workflow_status:"failed" })`
- old two-call `set passed` then `advance phase` → one `workflow_set({ phase_result:"passed", phase:<next> })`
- old two-call `set passed` then `complete` → one `workflow_set({ phase_result:"passed", workflow_status:"completed" })`

Migration rules:

- remove old exported tool names and old dispatch aliases in the same implementation slice
- do not document any temporary workflow alias as active product behavior

Minor-version scope is appropriate because this is a user-visible tool-contract simplification in an existing feature area.

## Testing

- inspect generated tool schemas → verify: the workflow tool surface contains only `workflow_get_state` and `workflow_set`.
- inspect generated tool schemas → verify: `workflow_set` contains only `workflow`, `phase_result`, `phase`, and `workflow_status`, with no workflow mutation `reason` field.
- inspect generated tool schemas → verify: old workflow names and aliases are absent.
- call `workflow_get_state()` → verify: it returns the current workflow state plus allowed transitions and retry metadata.
- call `workflow_set({"workflow":"NORMAL"})` → verify: the active workflow resets to the workflow start phase with `status="running"`, `phase_result="pending"`, and reset retry state.
- call `workflow_set({"phase_result":"passed"})` → verify: only the phase result changes and current status remains otherwise unchanged.
- call `workflow_set({"phase_result":"failed"})` → verify: only the phase result changes and current status remains otherwise unchanged.
- call `workflow_set({"phase_result":"user_feedback_required"})` → verify: phase result becomes `user_feedback_required` and status becomes `waiting_user`.
- call `workflow_set({"phase":"VALIDATE"})` in a currently valid transition context → verify: the phase changes, status becomes `running`, phase result becomes `pending`, and retry state resets.
- call `workflow_set({"workflow_status":"completed"})` when current phase result is `passed` → verify: workflow status becomes `completed`.
- call `workflow_set({"workflow_status":"failed"})` → verify: workflow status becomes `failed`.
- call `workflow_set({"phase_result":"passed","phase":"VALIDATE"})` → verify: one call succeeds and final state is the new phase with `phase_result="pending"`.
- call `workflow_set({"phase_result":"passed","workflow_status":"completed"})` → verify: one call succeeds and final state is completed with `phase_result="passed"`.
- call `workflow_set({"phase_result":"failed","workflow_status":"failed"})` → verify: one call succeeds and final state is failed with `phase_result="failed"`.
- call `workflow_set({"phase_result":"failed","phase":"VALIDATE"})` → verify: clear validation error.
- call `workflow_set({"phase_result":"user_feedback_required","workflow_status":"failed"})` → verify: clear validation error.
- inspect workflow transition recording for activation, phase move, waiting-user, completion, and failure → verify: one merged request records one coherent transition outcome.
- inspect prompt guidance after implementation → verify: it teaches `workflow_get_state` and `workflow_set` only.
- inspect TUI tool labels after implementation → verify: the visible workflow tool labels are updated to the new two-tool surface.
- run `cargo check -p themion-core` → verify: default core build stays clean.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build stays clean.
- run `cargo check -p themion-cli` → verify: default CLI build stays clean.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build stays clean.

## Implementation checklist

- [ ] add the `workflow_set` tool schema with fields `workflow`, `phase_result`, `phase`, and `workflow_status`
- [ ] remove `workflow_set_active`, `workflow_set_phase`, `workflow_set_phase_result`, and `workflow_complete` from exported tool definitions
- [ ] remove workflow alias dispatch support for `get_workflow_state`, `set_workflow`, `set_workflow_phase`, `set_phase_result`, and `complete_workflow`
- [ ] remove `reason` from the public workflow mutation request schema
- [ ] validate that `workflow_set` requests include at least one field
- [ ] validate the exact allowed single-field and combined request shapes from this PRD
- [ ] implement `user_feedback_required` so merged `workflow_set` preserves the intended `status=waiting_user` behavior
- [ ] return the required post-mutation workflow core state from `workflow_set`
- [ ] merge workflow mutation application in `crates/themion-core/src/agent.rs` into one `workflow_set` path
- [ ] keep workflow persistence and transition recording correct for activation, phase move, waiting-user, completion, and failure
- [ ] update workflow guidance text in `crates/themion-core/src/agent.rs`
- [ ] update workflow tool-call display labels in `crates/themion-cli/src/tui.rs`
- [ ] update `docs/engine-runtime.md` so the active workflow tool surface matches implementation
- [ ] add focused tests for the exact valid and invalid request shapes in this PRD
