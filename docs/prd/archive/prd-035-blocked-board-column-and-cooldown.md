# PRD-035: Add `blocked` Board Column with Cooldown-Aware Revisit Semantics

- **Status:** Implemented
- **Version:** v0.21.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-22

## Summary

- Extend the durable notes board from three columns to four: `todo`, `in_progress`, `done`, and `blocked`.
- Keep the normal work path simple: `todo -> in_progress -> done`.
- Allow blocked work as an explicit detour: `todo -> in_progress -> blocked`.
- Let blocked work re-enter normal flow through `blocked -> todo` when it becomes actionable again.
- Allow newly created follow-up notes to start directly in `blocked` when they exist mainly to wait for a long-poll result, timeout, or external event.
- Keep `blocked` lower priority than both `in_progress` and `todo`, and enforce a cooldown so unresolved blocked notes are not retried every idle tick.

## Goals

- Add a first-class durable board state for work that is currently not actionable.
- Keep the normal lifecycle legible and simple for ordinary work.
- Let agents explicitly represent “waiting for something” without pretending the note is ready or complete.
- Support follow-up notes that are created specifically to wait for a later event rather than to begin immediate work.
- Ensure blocked notes are retryable rather than terminal, so they can later move back into the normal flow.
- Give blocked work the lowest delivery priority so agents finish or start ready work before revisiting stalled work.
- Prevent hot-loop reinjection of blocked notes by enforcing a cooldown period between automatic revisit attempts.

## Non-goals

- No redesign of the broader note system into a full project-management workflow engine.
- No introduction of priority scores, deadlines, labels, or arbitrary custom board columns.
- No requirement to automatically detect every blocked condition from freeform note text.
- No requirement in this PRD to add separate `waiting_external`, `waiting_user`, `review`, or `archived` board states.
- No attempt to make blocked-note retry scheduling globally optimal across all agents and all instances.
- No requirement to weaken the existing preference for `in_progress` work over fresh ready work.
- No change to the meaning of `done` as completed-enough work.

## Background & Motivation

### Current state

The durable board introduced in PRD-029 intentionally started with exactly three columns:

- `todo`
- `in_progress`
- `done`

That first slice explicitly rejected richer states such as `blocked`, `review`, or `archived` in order to keep the initial model simple.

Since then, the board has become a more important collaboration mechanism. Notes are now durable, network-delivered when Stylos is enabled, injected with metadata-first prompting, and can generate requester-directed done mentions.

The current three-column board still leaves one practical gap: some work is neither ready (`todo`), actively being worked (`in_progress`), nor complete (`done`). It is temporarily not actionable.

Examples include:

- a delegated note is waiting on another agent's response or artifact
- a task needs external conditions to change before retrying
- a note depends on missing context or clarification that has not arrived yet
- a worker wants to create a follow-up note that should wake up later when some event or long wait completes

Today, that kind of work must be represented indirectly by leaving the note in `todo` or `in_progress`, or by writing prose in the note result/body. That makes the board less truthful.

### Why `blocked` should now be explicit

A durable board should distinguish between:

- work that is ready to start (`todo`)
- work already underway (`in_progress`)
- work complete enough to stop (`done`)
- work that is real and pending but currently waiting (`blocked`)

Without an explicit blocked state:

- ready backlog and waiting work become mixed together
- the runtime may keep retrying notes that are known to be temporarily not actionable
- agents have no durable, structured way to express “not done, but wait for now”
- humans and peer agents cannot inspect the board and understand why a note is stalled

**Alternative considered:** keep the three-column model and encode blocked semantics only in note text or result text. Rejected: that hides an important workflow distinction inside prose and weakens board-level inspectability and scheduling.

### Why blocked work should remain lower priority than ready work

The user wants blocked notes to be the least-priority work. That matches the practical board semantics:

- `in_progress` means the agent already started real work and should resume it first
- `todo` means ready work that can be started now
- `blocked` means work that is waiting and should be revisited only after ready work is exhausted and cooldown allows recheck

This means the runtime should prefer:

1. `in_progress`
2. `todo`
3. `blocked`

A blocked state is therefore not another form of active work. It is a holding area for deferred retry or future wake-up.

**Alternative considered:** consider `blocked` equal to `todo` during idle-time selection. Rejected: that would cause the runtime to keep surfacing known-not-actionable work too aggressively and would contradict the requested priority order.

### Why blocked notes need a cooldown period

If blocked notes are durable and retryable, the runtime still needs to avoid hammering them repeatedly.

Without cooldown behavior:

- an idle agent could receive the same blocked note on every tick
- the model could repeatedly restate that the note is still blocked with no new information
- long-wait follow-up notes would become noisy instead of useful

The correct behavior is to let blocked notes be revisited occasionally, not continuously. The board therefore needs a retry cooldown that delays the next automatic revisit of a blocked note after it is moved into `blocked` or after a blocked recheck still finds no progress.

**Alternative considered:** never auto-retry blocked notes and require only manual movement back to `todo`. Rejected: that makes blocked work too easy to forget and does not match the desired behavior that the agent should still try to resolve blocked work later.

## Design

### Extend the board model with a fourth primary column: `blocked`

The durable note board should add one new canonical column:

- `blocked`

The board columns become:

- `todo`
- `in_progress`
- `blocked`
- `done`

Normative behavior:

- every note belongs to exactly one canonical column at a time
- `blocked` is a first-class durable column, not a derived display state
- `blocked` means the note is still pending work but cannot currently make meaningful progress
- `blocked` is not terminal and does not imply abandonment or completion

**Alternative considered:** represent blocked as metadata while keeping the visible column as `todo` or `in_progress`. Rejected: that hides important state from both tooling and board readers.

### Define the canonical column transitions

The canonical board transitions in this PRD are exactly:

- `todo -> in_progress`
- `in_progress -> done`
- `in_progress -> blocked`
- `blocked -> todo`
- `create new -> blocked`

Read as higher-level paths, that means:

- normal flow: `todo -> in_progress -> done`
- blocked detour: `todo -> in_progress -> blocked`
- blocked resolution: `blocked -> todo`
- waiting-first follow-up: `create new -> blocked`

Typical examples for direct-create blocked notes include:

- very long polling checks
- waiting for an external event
- waiting for another process or agent to finish before real work can start
- reminder-style rechecks that should not enter ready backlog yet

Normative behavior:

- the common path is `todo -> in_progress -> done`
- `blocked` should normally be entered from `in_progress`, after work has started and then encountered a waiting condition
- the canonical recovery path from blocked is `blocked -> todo`
- newly created blocked notes should be used only for notes whose first useful action is to wait rather than to work immediately
- prompt guidance, docs, and board semantics should treat the transition set above as the intended lifecycle
- if implementation details temporarily allow additional transitions for compatibility or internal simplicity, those transitions should not be documented as first-class workflow behavior

**Alternative considered:** allow any column-to-column transition and treat the board as a loose tag system. Rejected: the user asked for a clear lifecycle, and that clarity is useful for both agent behavior and board inspection.

### Define when agents should use `blocked`

Agents should use `blocked` when the note remains real pending work but current progress is not possible or not useful.

Typical reasons include:

- waiting for another agent or instance to complete prerequisite work
- waiting for user clarification or external information that has not yet arrived through the note flow
- waiting for an external dependency, resource, or timing condition
- creating a follow-up note whose whole purpose is to wake up later and check whether something changed

Normative behavior:

- a blocked note should usually carry updated result text describing why it is blocked or what it is waiting for when that context is useful
- moving to `blocked` should be an explicit note-state mutation, not only a conversational statement
- when a blocked note becomes actionable again, the normal recovery path is `blocked -> todo`
- if the note is a direct-create blocked follow-up, its body should clearly describe what event, timeout, or condition should be checked on revisit

**Alternative considered:** automatically derive blocked state only from certain phrases like “waiting” or “blocked” in note text. Rejected: freeform text inference would be brittle and less explicit than a direct board move.

### Keep blocked notes lowest priority in idle-time selection

Idle-time note injection should continue to prefer active and ready work before attempting blocked work.

Normative behavior:

- when selecting a pending note for an idle agent, the runtime should first consider eligible `in_progress` notes
- if no eligible `in_progress` note exists, the runtime should consider eligible `todo` notes
- only if no eligible `in_progress` or `todo` note exists should the runtime consider eligible `blocked` notes
- within a column, selection should remain deterministic such as oldest eligible first unless a later PRD changes ordering deliberately

This preserves current board semantics while adding blocked work as a fallback revisit class rather than an equal peer of ready work.

**Alternative considered:** prefer blocked notes over `todo` because they may be older. Rejected: readiness matters more than age for the requested board behavior.

### Add cooldown-aware eligibility for blocked-note reinjection

Blocked notes should not be eligible for immediate repeated idle-time reinjection.

Normative behavior:

- a blocked note must have a cooldown period before the runtime automatically re-injects it again
- the cooldown should be measured from the most recent move into `blocked` or blocked recheck that still found no progress
- a blocked note whose cooldown has not expired is ineligible for automatic idle-time injection
- once cooldown expires, the note becomes eligible again under the blocked-priority rules above
- if an agent manually reads or moves the note through tools, that remains allowed regardless of automatic injection cooldown

This ensures blocked notes remain retryable without creating hot loops.

**Alternative considered:** use the ordinary `updated_at` timestamp only and retry blocked notes whenever they are oldest. Rejected: that would conflate generic note updates with explicit retry scheduling and would make cooldown behavior ambiguous.

### Let blocked notes re-enter normal flow through `todo`

Blocked is a waiting state, not a graveyard.

Normative behavior:

- when an agent receives a blocked note after cooldown, it should decide whether the note is now actionable
- if the note remains blocked, the agent may keep it in `blocked`, refresh the note result if useful, and allow another cooldown cycle
- if the note becomes actionable again, the expected resolution path is `blocked -> todo`
- after returning to `todo`, the note can later proceed through the ordinary path `todo -> in_progress -> done`
- if a blocked note no longer needs work, the implementation may still allow direct move to `done`, but the canonical workflow should emphasize returning actionable work to `todo`

This matches the desired behavior that blocked work should be retried and then continue through the normal lifecycle.

**Alternative considered:** allow blocked notes to resume directly to `in_progress` as the primary pattern. Rejected: the requested lifecycle specifically calls out `blocked -> todo` as the normal resolve path.

### Preserve existing done-mention and collaboration behavior

The addition of `blocked` should not redesign note-first collaboration.

Normative behavior:

- blocked notes remain ordinary durable notes and continue to preserve sender/target metadata
- blocked notes do not generate done mentions because they are not complete
- done mentions remain classified informational notes and may themselves use `blocked` only if genuine follow-up remains and is waiting on something
- the existing preference for durable notes over interrupting talk remains unchanged

**Alternative considered:** treat blocked cross-agent notes as implicit talk-worthy escalation events. Rejected: that would blur the note-first collaboration model and create unnecessary interruption.

### Introduce explicit blocked retry metadata if needed

Cooldown-aware blocked retry likely needs dedicated durable metadata beyond the current column and timestamps.

A practical design may include one or more of:

- `blocked_at_ms`
- `blocked_until_ms`
- `last_blocked_retry_at_ms`

Normative behavior:

- any machine-consumed retry timestamp must remain explicitly millisecond-based
- the runtime should use dedicated blocked-retry metadata rather than inferring cooldown solely from unrelated board updates when practical
- the exact schema can be implementation-shaped, but the resulting behavior must make retry eligibility deterministic and inspectable

**Alternative considered:** avoid schema changes and keep cooldown entirely in transient process memory. Rejected: blocked semantics are part of the durable board and should survive restarts cleanly.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/db.rs` | Extend durable note column support to include `blocked` and persist cooldown-related blocked retry metadata via durable `blocked_until_ms` eligibility state. |
| `crates/themion-core/src/tools.rs` | Update `board_create_note`, `board_list_notes`, and `board_move_note` schemas and serialization so `blocked` is a valid board column where appropriate. |
| `crates/themion-cli/src/tui.rs` | Update idle-time note selection so it prefers eligible `in_progress`, then eligible `todo`, then cooldown-eligible `blocked` notes for the target agent. |
| `crates/themion-cli/src/stylos.rs` | Update injected note guidance so blocked notes are framed as deferred work to reassess rather than fresh ready work, including direct-create blocked follow-up notes. |
| `docs/architecture.md` | Document the four-column board model, canonical transitions, blocked-note priority rules, and cooldown-aware blocked retry behavior. |
| `docs/engine-runtime.md` | Document runtime selection order, direct-create blocked follow-up semantics, and blocked-note cooldown gating for idle-time injection. |
| `docs/README.md` | Update this PRD entry to implemented status. |

## Edge Cases

- an agent has both `in_progress` and cooldown-eligible `blocked` notes → verify: the runtime injects from `in_progress` first.
- an agent has `todo` notes and cooldown-eligible `blocked` notes but no `in_progress` note → verify: the runtime injects a `todo` note before any blocked note.
- an agent has only blocked notes and none are past cooldown → verify: no blocked note is auto-injected yet.
- an agent has only blocked notes and one cooldown expires → verify: the expired blocked note becomes eligible for auto-injection.
- an agent moves a note from `todo` to `in_progress` and then discovers it must wait → verify: the note can move to `blocked` and is not immediately reinjected again.
- a blocked note becomes actionable during review → verify: the canonical recovery path is `blocked -> todo`, after which it can continue through the normal flow.
- a newly created follow-up note is intended only to wait for a long poll or event → verify: it can be created directly in `blocked` and remains low priority until cooldown and revisit conditions allow reassessment.
- a blocked note is rechecked and still waiting → verify: it stays in `blocked` and receives a fresh cooldown rather than being hot-looped.
- the process restarts while blocked notes are cooling down → verify: the cooldown remains effective because retry eligibility is derived from durable state rather than only in-memory timers.
- a done-mention note is itself waiting on a real follow-up action → verify: the note may use `blocked` if genuinely stalled, but marking it `done` still must not generate recursive automatic done mentions.

## Migration

A schema migration is expected because the durable board model and blocked retry metadata need to represent the new state explicitly.

Behaviorally:

- existing notes in `todo`, `in_progress`, and `done` remain valid unchanged
- no existing note is automatically reclassified as `blocked`
- after upgrade, agents and tools may begin moving notes into `blocked`
- after upgrade, follow-up notes may also be created directly in `blocked` when they are intentionally waiting-first notes
- after downgrade to an older build without `blocked` support, blocked-note rows would be ambiguous, so compatibility expectations should be documented clearly if downgrade support matters

Because this is a user-visible additive board feature, the implementation should update docs and prompt-visible board guidance in the same change.

## Testing

- create a normal note in `todo`, move it to `in_progress`, then to `done` → verify: the normal lifecycle remains unchanged.
- move a note through `todo -> in_progress -> blocked` → verify: the blocked detour is stored durably and remains visible on the board.
- recheck a blocked note and move it through `blocked -> todo` → verify: it re-enters the normal flow rather than remaining stuck outside it.
- create a follow-up note directly in `blocked` for a long-poll or wait-for-event case → verify: the note is stored as blocked from creation and is not treated as ready backlog.
- call `board_list_notes` for `blocked` → verify: blocked notes are returned the same way other columns are returned.
- let an agent become idle with `in_progress`, `todo`, and `blocked` notes present → verify: selection order remains `in_progress` before `todo` before `blocked`.
- let an agent become idle with only blocked notes whose cooldown has not expired → verify: no blocked note is injected automatically.
- let blocked cooldown expire for one note → verify: that note becomes eligible for idle-time injection.
- receive a blocked note injection, determine it is still blocked, and keep it blocked → verify: the runtime does not immediately reinject the same note again before cooldown expires.
- mark a cross-agent delegated note `blocked` and later `done` → verify: no done mention is created while blocked, and normal done-mention behavior still occurs only on completion.
- run `cargo check -p themion-core -p themion-cli` and `cargo check -p themion-cli --features stylos` after implementation → verify: default and Stylos-enabled builds compile cleanly with the new board column support.

## Implementation notes

The implemented slice landed with these concrete behaviors:

- `blocked` is a durable board column alongside `todo`, `in_progress`, and `done`
- blocked-note retry eligibility is persisted with durable millisecond `blocked_until_ms` metadata
- `board_create_note` can create notes directly into `blocked` for waiting-first follow-up workflows
- idle-time note selection now prefers `in_progress`, then `todo`, then cooldown-eligible `blocked`
- note prompt guidance distinguishes blocked work from ready work so reassessment behavior is clear

## Implementation checklist

- [x] extend the durable note column model to include `blocked`
- [x] update board tool schemas and validation so `blocked` is a valid column
- [x] support direct-create blocked follow-up notes for wait-first workflows
- [x] add durable blocked retry metadata and cooldown eligibility logic
- [x] update idle-time note selection to prefer `in_progress`, then `todo`, then cooldown-eligible `blocked`
- [x] update injected note guidance so blocked notes are presented as deferred work under reassessment
- [x] document the four-column board model, canonical transitions, and blocked cooldown semantics in architecture/runtime docs
- [x] update `docs/README.md` with the new PRD entry
