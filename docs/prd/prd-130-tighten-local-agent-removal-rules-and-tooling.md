# PRD-130: Tighten Local Agent Removal Rules and Tooling

- **Status:** Implemented
- **Version:** >v0.79.0 +minor
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-16

## Summary

- Keep `local_agent_delete`, but make its safety rules explicit.
- `master` is protected and cannot be removed.
- A local agent can be removed only when the target agent is idle.
- An agent cannot remove itself.
- Existing inspection/status tools are enough to choose a valid target; no new delete tool is required.

## Problem

Themion already has `local_agent_delete`, but the current behavior is not strict enough for multi-agent runtime safety.

Today:

- deleting `master` is rejected
- deletion is blocked by a broader local-runtime busy check
- self-removal is not an explicit rule
- the tool description does not explain the real safety constraints clearly

This leaves the wrong policy. One busy agent should not block deletion of a different idle agent, and an acting agent should never be able to remove itself.

## Scope

In scope:

- `local_agent_delete` behavior
- local runtime validation for removable targets
- model-facing tool description updates
- docs for local agent membership and removal rules
- tests for protected target, busy target, idle target, and self-removal cases

Out of scope:

- bulk delete or force delete
- canceling active work during delete
- auto-reassigning work from a removed agent
- redesigning local team structure or watchdog scheduling
- adding a new delete-planning tool if current inspection/status surfaces are already enough

## Current behavior

Current local runtime behavior in `themion-cli`:

- rejects deleting `master`
- rejects deletion while the local runtime is busy
- removes the target immediately on success

Current tool/docs behavior in `themion-core` and docs:

- `local_agent_delete` exists as the normal removal tool
- the tool description says it deletes a non-leader local agent
- docs do not yet define the stricter target-idle and no-self-delete rules

## Expected behavior

Required behavior:

- `master` must remain non-removable
- a target local agent may be removed only when that target agent is idle
- one busy local agent must not block deletion of a different idle local agent
- an acting local agent must not be able to remove itself
- unknown target ids must fail clearly
- successful deletion should keep the current compact success result shape
- failure cases should return clear validation errors

Product rule:

- protect `master`, require target idle, forbid self-delete

## Fix approach

### 1. Make delete target-local instead of process-global

Delete validation should use the target agent's current runtime state, not a whole-process busy flag.

Required behavior:

- if the target agent is idle, deletion may proceed
- if the target agent is busy in any non-idle activity, deletion must fail
- another agent being busy must not matter if the target agent is idle

The runtime should use current runtime-owned activity/status truth for this decision.

### 2. Keep `master` protected

Required behavior:

- `local_agent_delete(agent_id="master")` must fail
- any equivalent local removal path must also reject `master`
- the error should say clearly that `master` is protected or predefined and cannot be removed

### 3. Forbid self-removal

Required behavior:

- if the acting agent calls `local_agent_delete` for its own `agent_id`, the request must fail
- this applies even if the acting agent is idle
- the error should say clearly that an agent cannot remove itself

### 4. Keep one canonical tool surface

Required behavior:

- `local_agent_delete` remains the canonical removal tool
- its description should mention the real runtime constraints in compact form
- existing local inspection/status tools remain the recommended pre-delete read path
- this PRD does not require a second delete tool

## Risks / edge cases

- delete `master` while idle → verify: still rejected
- delete unknown `agent_id` → verify: clear unknown-target error
- target idle, different agent busy → verify: delete succeeds
- target busy, different agent idle → verify: delete fails because target is not idle
- acting agent targets itself → verify: always rejected
- target state changes between inspection and delete call → verify: runtime checks current state at delete time

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-130-tighten-local-agent-removal-rules-and-tooling.md` | Add the durable PRD. |
| `crates/themion-core/src/tools.rs` | Tighten `local_agent_delete` tool description text. |
| `crates/themion-cli/src/app_runtime.rs` | Replace broad busy gating with target-idle validation, keep `master` protected, and reject self-removal. |
| runtime/app-state status ownership | Provide the target agent activity/status truth used by delete validation. |
| `docs/engine-runtime.md` and related docs | Document the stricter removal rules and inspection-before-delete guidance. |
| tests near touched runtime code | Add targeted removal validation coverage. |

## Validation

- create one additional idle worker and delete it with `local_agent_delete` → verify: deletion succeeds and roster updates immediately
- call `local_agent_delete` on `master` → verify: deletion is rejected with a protected-target error
- start work on one worker, then try to delete that same worker → verify: deletion is rejected because the target is not idle
- keep one worker busy and try to delete a different idle worker → verify: deletion succeeds because validation is target-local
- issue `local_agent_delete` from a target agent against itself → verify: deletion is rejected with a self-removal error
- run `cargo check -p themion-cli` → verify: touched CLI runtime code compiles with default features
- run `cargo check -p themion-cli --features stylos` → verify: touched CLI runtime code compiles with Stylos enabled
- run `cargo check -p themion-cli --all-features` → verify: touched CLI runtime code compiles across feature combinations

## Implementation Notes

Implementation is now landed in the current working tree by updating `crates/themion-cli/src/app_runtime.rs`, `crates/themion-cli/src/app_state.rs`, `crates/themion-core/src/agent.rs`, `crates/themion-core/src/tools.rs`, and `docs/engine-runtime.md`.

The landed behavior:

- keeps `master` protected from deletion
- threads the acting local agent id through the local-agent management path so self-delete is rejected explicitly
- checks the target agent's current busy/idle state instead of rejecting deletion based on unrelated local-runtime busy work
- keeps the existing compact success result shape for successful deletion
- adds targeted runtime tests for protected-target, self-delete, busy-target, and idle-target cases

Validation completed for the landed slice:

- `cargo check -p themion-core`
- `cargo check -p themion-cli`
- `cargo check -p themion-cli --features stylos`

The PRD version target remains future-facing because the repository build/version metadata is still `v0.78.1`; release-version assignment for this landed change is still pending normal versioning follow-through.
