# PRD-037: Remove the Hard-Coded 10-Round Harness Loop Limit and Rely on State-Based Termination

- **Status:** Implemented
- **Version:** v0.23.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-22

## Summary

- The current harness loop stops after a hard-coded maximum of 10 assistant/tool rounds in one user turn.
- That fixed cap is arbitrary and duplicates other existing runtime logic that already decides when a turn should stop.
- Remove the literal `0..10` loop bound and let the assistant/tool cycle continue until existing state-based termination conditions say the turn is done.
- Do not replace the hard-coded limit with another fixed numeric cap exposed through config.
- Preserve interruption, workflow, tool execution, and persistence behavior; this PRD changes only the turn-loop termination policy.
- Keep room for future non-numeric safeguards if real failure modes require them, but do not define a new fixed round count in this slice.

## Goals

- Remove the hard-coded `for _ in 0..10` harness loop limit from the core runtime.
- Stop using a fixed numeric round cap as the primary turn-loop safeguard.
- Rely on existing state-based stop conditions for normal assistant/tool loop termination.
- Preserve current interruption, persistence, and tool-execution behavior.
- Update docs so they no longer describe the harness as stopping after 10 iterations.

## Non-goals

- No redesign of the overall user-turn lifecycle, message persistence model, or workflow engine.
- No requirement in this PRD to introduce a replacement configurable numeric round limit.
- No change to how tools themselves are executed or persisted.
- No requirement to invent new token-based, time-based, or provider-specific safeguards in the same slice.
- No claim that future non-numeric safeguards are forbidden if real runtime issues later justify them.

## Background & Motivation

### Current state

The current harness loop in `crates/themion-core/src/agent.rs` runs assistant/tool follow-up rounds inside a fixed loop:

```rust
for _ in 0..10 {
    ...
}
```

This means one user turn may perform at most 10 assistant/tool rounds before the harness stops even if the model would otherwise continue requesting tools.

The runtime docs already describe this behavior. `docs/engine-runtime.md` currently says the model/tool cycle repeats until the model stops requesting tools or the loop limit is reached, and `docs/architecture.md` explicitly describes step 8 as repeating up to 10 iterations.

### Why the hard-coded round cap should be removed

The fixed inline `10` is a magic number. It is not derived from the actual runtime state, and it forces a particular stopping policy even when the turn still has legitimate state-driven reasons to continue.

Themion already has other logic that governs turn progression and termination, including:

- stopping when the assistant returns with no tool calls
- interruption handling
- provider or runtime error paths
- workflow-state transitions and other explicit control flow around a turn

Because those mechanisms already shape when a turn can or should end, the fixed numeric cap is an extra policy layer that is both arbitrary and potentially premature.

This PRD therefore treats the 10-round cap as duplication of control rather than as the right long-term safety boundary.

**Alternative considered:** replace the hard-coded `10` with a configurable numeric round limit. Rejected: that still keeps the core design centered on a fixed number rather than on actual runtime state, which does not match the intended direction.

## Design

### Remove the literal numeric loop bound

The core harness should stop representing the assistant/tool cycle as `for _ in 0..10`.

Normative behavior:

- the assistant/tool loop must no longer use a fixed literal numeric bound as its governing termination condition
- the loop should continue while existing runtime state says additional assistant/tool follow-up is needed
- the normal successful exit condition remains unchanged: if the assistant returns with no tool calls, the turn ends immediately

This change removes the hard-coded round cap without changing the meaning of normal completion.

**Alternative considered:** keep the literal and merely add a comment explaining why `10` exists. Rejected: the user explicitly wants to remove the fixed-number limit, not explain it.

### Rely on state-based termination already present in the runtime

The harness should rely on existing non-numeric stop conditions rather than a fixed round count.

Normative behavior:

- the loop should exit when the assistant response has no tool calls
- the loop should still honor interruption immediately
- the loop should still stop on existing runtime or provider error paths that already abort or fail the turn
- workflow and turn-finalization logic outside the loop should remain responsible for their existing state-based stop behavior

This keeps the runtime aligned with the actual reasons a turn is complete or cannot continue.

**Alternative considered:** add a new complex state machine solely to replace the fixed numeric bound. Rejected: the requested direction is to trust existing logic, not to redesign the harness around a new control framework.

### Do not introduce a replacement configurable numeric cap in this slice

This PRD intentionally does not replace the hard-coded `10` with a config field such as `max_turn_rounds`.

Normative behavior:

- no new configuration-backed numeric assistant/tool round limit is required for this change
- docs and code should not describe a new fixed numeric round cap as the intended replacement policy
- if future safeguards are needed, they should be justified by concrete runtime failure modes and should prefer state-based or otherwise non-arbitrary behavior when practical

**Alternative considered:** introduce `max_turn_rounds` with default `10` for backward compatibility. Rejected: that preserves the same policy shape under a different name.

### Preserve visibility of real turn-ending reasons

Removing the numeric cap should make turn-ending reasons more truthful, not less.

Normative behavior:

- normal completion should still correspond to the assistant reaching a response state with no more tool calls
- interruption should remain distinguishable from normal completion
- existing error or abort paths should remain distinguishable where they already are
- the runtime should not invent a new round-limit end reason in this slice because the fixed round limit is being removed rather than renamed

**Alternative considered:** keep a synthetic `round_limit_reached` reason for compatibility even after removing the cap. Rejected: that would preserve a termination concept this PRD is explicitly removing.

### Update architecture and runtime docs to describe state-based loop termination

The docs should stop describing the harness as repeating “up to 10 iterations” and should instead describe the assistant/tool cycle in terms of state-based completion.

Normative behavior:

- `docs/architecture.md` should describe the assistant/tool cycle as repeating until the assistant returns no more tool calls or another existing runtime stop condition ends the turn
- `docs/engine-runtime.md` should describe the loop as state-driven rather than numerically capped at 10
- documentation should not replace the old wording with a new fixed-number limit unless a later PRD intentionally introduces one

**Alternative considered:** leave the docs vague and just remove the number. Rejected: the docs should positively explain the intended state-based behavior.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/agent.rs` | Remove the hard-coded `for _ in 0..10` loop bound and express the assistant/tool cycle in state-driven form while preserving existing interruption and completion behavior. |
| `docs/architecture.md` | Replace “up to 10 iterations” language with state-based termination wording. |
| `docs/engine-runtime.md` | Describe the assistant/tool loop as continuing until no further tool calls or another existing stop condition ends the turn. |
| `docs/README.md` | Keep this PRD indexed in the PRD table. |

## Edge Cases

- the model returns a normal assistant response with no tool calls on the first round → verify: the turn ends normally exactly as before.
- the model performs many legitimate tool-follow-up rounds beyond the historical count of 10 → verify: the turn is allowed to continue while state says more follow-up is needed.
- the user interrupts a long tool-calling turn → verify: interruption still stops the turn promptly.
- the provider or runtime hits an existing error path during a long tool-calling turn → verify: the turn still stops according to that existing error path rather than requiring a numeric cap.
- the model enters a pathological repetition pattern that does not naturally terminate → verify: the current implementation behavior is understood and any future safeguard work is treated as a separate follow-up rather than silently preserving the old fixed cap.

## Migration

This is a runtime-policy and docs change only.

Behaviorally:

- turns are no longer forced to stop at the historical fixed round count of 10
- ordinary turns that finish under existing state-based logic should behave unchanged
- no config migration is required because this PRD does not introduce a replacement round-limit setting
- no SQLite schema migration is required

If later production experience shows that an additional safeguard is needed, that follow-up should be documented explicitly rather than smuggled in as a hidden new magic number.

## Testing

- run a turn that completes without tool calls → verify: the turn still exits normally.
- run a tool-calling turn that finishes in fewer than 10 rounds → verify: behavior remains unchanged.
- run a tool-calling scenario that legitimately needs more than 10 rounds → verify: the turn can continue past the historical limit and still finalize cleanly when state-based completion occurs.
- interrupt a long-running tool-calling turn → verify: interruption semantics remain unchanged.
- trigger an existing runtime or provider failure path during the assistant/tool loop → verify: the turn still stops cleanly without depending on a numeric round cap.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: the core and CLI crates still compile cleanly after the loop-termination change.

## Implementation checklist

- [x] remove the hard-coded `for _ in 0..10` harness loop bound
- [x] express the assistant/tool loop in state-driven form using existing completion and interruption logic
- [x] avoid introducing a replacement fixed numeric round-limit config in this slice
- [x] update `docs/architecture.md` and `docs/engine-runtime.md` to remove hard-coded 10-iteration wording
- [x] keep `docs/README.md` aligned with the final PRD title and scope


## Implementation notes

The implemented slice landed with these concrete behaviors:

- the assistant/tool loop in `crates/themion-core/src/agent.rs` now uses `loop` instead of `for _ in 0..10`
- turns now continue until the assistant stops requesting tools or another existing runtime stop condition ends the turn
- no replacement configurable numeric round cap was introduced
- `docs/architecture.md` and `docs/engine-runtime.md` now describe state-based termination instead of a fixed 10-iteration limit
