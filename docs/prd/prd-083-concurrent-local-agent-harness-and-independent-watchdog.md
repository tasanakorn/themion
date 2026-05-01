# PRD-083: Concurrent Local Agent Harness Execution and Independent Watchdog Scheduling

- **Status:** Implemented
- **Version:** v0.54.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-05-01

## Summary

- Themion already supports multiple local agents in one instance, targeted `agent_id` routing, and board-note-based coordination, but actual local work is still effectively serialized behind one active-turn lane.
- The current watchdog can notice pending work, yet one busy agent can still delay or block useful follow-up because the runtime does not admit another local turn independently enough.
- This PRD makes local multi-agent execution real: each local agent should own an independent turn-execution path inside the same Themion process.
- The idle watchdog should keep running as a background scheduler, claim eligible pending work safely, and dispatch it to other idle local agents even while one or more agents are already busy.
- Keep the existing single-process architecture, board-note model, Stylos targeting semantics, and PRD-081 team model; this change is about runtime concurrency, admission, and scheduling inside one instance, not a new coordination protocol.

## Implementation status

Phase 1 has landed. The implementation removed the effective app-global local-turn admission bottleneck in `themion-cli`, moved turn interruption ownership to per-agent handles, unblocked watchdog polling from the previous single global busy gate, added a CLI-local board-note claim/finalize/release layer around watchdog delivery, and updated the runtime docs to describe the new per-agent concurrent behavior. Multiple local agents in one process can now overlap turns, and one busy local agent no longer automatically stops watchdog-driven progress for another eligible idle local agent. Follow-through fixes also made watchdog and remote incoming-prompt dispatch submit directly to the explicitly selected local agent handle rather than re-routing through shared intake selection, and moved watchdog timer/state plus idle-agent selection policy into CLI runtime helpers outside `tui.rs`, so pending board-note injection now preserves the chosen idle local target under concurrent local activity with less TUI-owned orchestration.

## Goals

- Allow multiple local agents in one Themion instance to execute turns concurrently.
- Remove the current shared local-turn busy gate as the practical limiter for local multi-agent work.
- Make the idle watchdog continue running independently while one or more agents are busy.
- Let pending board notes continue reaching other eligible idle local agents instead of waiting behind unrelated turns.
- Preserve exact `agent_id` targeting semantics and the local team model already established by PRD-081.
- Keep status, transcript, inspection output, and lightweight in-TUI visibility truthful when several local agents are active at once.

## Non-goals

- No redesign of the board-note data model, columns, or note lifecycle semantics.
- No requirement in this PRD to add automatic task decomposition, planning, or delegation policy beyond independent local execution readiness.
- No requirement to introduce multi-process worker spawning; scope remains one process hosting multiple local agents.
- No requirement to redesign the TUI into panes, tabs, or per-agent transcript views.
- No requirement to make dynamically created local agents restart-persistent in this slice.
- No requirement to guarantee advanced fairness or load-balancing heuristics beyond correct independent execution and watchdog progress.

## Proposed delivery phases

### Phase 1: unblock true concurrent local turns and watchdog progress

Phase 1 is the implementation-start slice this PRD expects to land first.

Required outcome for Phase 1:

- multiple local agents in one process can run overlapping turns
- one busy agent no longer blocks another idle local agent from starting its own turn
- the watchdog continues running independently while local turns are active
- pending board notes can be claimed and dispatched to other eligible idle local agents without duplicate injection
- explicit target-specific busy outcomes remain honest and no silent rerouting is introduced
- status, transcript attribution, and inspection remain correct enough to verify concurrent local behavior

Phase 1 should prefer the smallest clean runtime change that makes concurrency real. It does not need advanced scheduling, queue fairness tuning, or new UI surfaces before delivering value.

### Phase 2: narrow remaining shared bottlenecks and polish runtime ergonomics

Later follow-on work may refine:

- any remaining shared coordination paths that still serialize more than necessary
- improved fairness or worker-selection policy for untargeted backlog work
- cleaner runtime modularization if Phase 1 needs a transitional compatibility layer around existing TUI-owned orchestration
- richer observability or debug surfaces for multi-agent overlapping work

This PRD keeps the overall product outcome visible. Phase 1 is now the landed scope for PRD-083; any further scheduler polish or UX follow-up should move to a new PRD rather than leaving PRD-083 open-ended.

### Why watchdog and harness must be fixed together

The symptom is easiest to notice in the watchdog, but the root problem is broader. A watchdog can only make useful progress if the local runtime can actually admit another local turn concurrently.

If the watchdog polls more aggressively while the runtime still enforces one shared execution lane, the product will still serialize actual work. Conversely, per-agent harness concurrency without an independently running watchdog would still leave backlog work under-dispatched.

So this PRD treats the issue as one product slice:

- each local agent needs its own real execution lane
- the watchdog needs to keep running in the background regardless of another agent's active turn
- scheduling, busy reporting, and user-visible observability need to stay correct under overlapping local turns

## Design

### 1. Treat busy state as per-agent scheduling truth, not one global admission gate

Themion should treat local execution availability as a property of each local agent rather than one process-wide foreground lock.

Required behavior:

- each local agent should be able to run its own harness turn independently of other local agents in the same process
- busy/idle state should remain tracked per local agent
- one busy local agent must not make unrelated idle local agents appear unavailable for targeted local execution
- local intake and routing should reject only the specific busy target agent rather than collapsing the whole instance to one shared busy state when another eligible local agent exists
- any remaining process-level busy summary may stay for observability, but it must not remain the scheduler's source of truth

This makes the runtime's scheduling model match the already-landed local team model.

**Alternative considered:** keep one shared execution lock and improve queueing around it. Rejected: that still serializes actual work and does not deliver concurrent local multi-agent execution.

### 1A. Phase 1 runtime shape

To make implementation start concrete, the first delivery slice should adopt a runtime shape close to the current architecture rather than inventing a new scheduler framework up front.

Phase 1 should aim for these structural changes:

- keep one process-local roster of local agents in CLI-owned runtime state
- keep each core `Agent` owning its own harness/session/workflow state as it does now
- give each local agent a dedicated in-memory execution slot that owns whether a turn is idle or running, plus any join handle or completion token needed to reconcile the turn later
- move turn-admission and completion bookkeeping into focused non-TUI runtime helpers so the TUI submits intents and renders state rather than directly owning concurrency policy
- keep background watchdog scheduling separate from any one agent's turn lifecycle

A practical Phase 1 implementation may still use shared channels and event fan-in, but it should stop requiring one global `agent_busy` style gate before a different local agent can begin work.

**Alternative considered:** design a brand-new generalized scheduler abstraction before changing behavior. Rejected: the current codebase already has enough runtime structure to land a smaller concurrency slice first, and a large scheduler rewrite would delay the main product fix.

### 2. Give each local agent an independent harness execution path

The runtime should execute turns for different local agents through independent agent-owned execution paths rather than one shared foreground turn path.

Required behavior:

- each local runtime-owned agent handle should own enough state to run a turn without borrowing one global execution slot
- each local agent should have its own turn-admission boundary, running-task ownership, and completion callback path
- starting a turn for one local agent should not require another idle local agent to wait for the first turn to finish unless both operations contend on a narrower shared resource that is explicitly documented
- interruption, completion, follow-up, and failure handling should resolve against the specific running local agent
- existing harness semantics such as workflow tracking, tool execution, streaming, and event emission should remain agent-local and continue to work when several agents are active concurrently
- the runtime should avoid fake concurrency where agents appear separate in the UI but still serialize behind one hidden central mutable loop

This makes local concurrency architectural reality rather than presentation-only behavior.

**Alternative considered:** multiplex all local agents through one central runner queue. Rejected: that may simplify bookkeeping, but it preserves one effective bottleneck unless the underlying execution still becomes independently concurrent per agent.

### 3. Keep the watchdog running as an independent background scheduler

The watchdog should continue polling and making dispatch decisions even while one or more local agents are already busy.

Required behavior:

- watchdog scheduling should stay background-task driven rather than depending on the completion cadence of one foreground agent turn
- the watchdog should continue waking on its configured cadence while local work is in progress
- watchdog eligibility checks should evaluate candidate agents individually rather than stopping at one global busy flag
- if one selected or targeted agent is busy, the watchdog should still consider other eligible local agents during the same scheduling lifecycle
- the watchdog should treat note selection and note delivery as separate steps so a chosen note can be claimed before injection and released or retried cleanly after a failed handoff
- the watchdog must not repeatedly inject duplicate work for the same note while that note is already in-flight for one local agent

This keeps durable backlog work moving instead of waiting behind unrelated activity.

**Alternative considered:** trigger watchdog scans only after agent-turn completion. Rejected: that couples backlog progress to whichever agent happens to finish first and recreates the current starvation pattern under long-running turns.

### 4. Make pending board-note injection concurrency-safe across the local team

Pending todo note dispatch should become truly multi-agent aware instead of acting like one global singleton path.

Required behavior:

- pending board-note selection should continue to respect existing note semantics, targeting metadata, and follow-up rules
- if a note explicitly targets one local `agent_id`, it should still go only to that agent
- if a note is untargeted or otherwise eligible for more than one local worker under current policy, the scheduler should choose one eligible idle local agent deterministically from the current roster order or another documented stable rule
- once a note has been claimed for delivery to one local agent, the runtime should prevent duplicate concurrent injection of that same note to another local agent in the same instance
- the runtime should record enough in-flight note state to distinguish queued, claimed, injected, completed, and retryable outcomes at the scheduler boundary
- if delivery to the chosen agent loses a handoff race because that agent becomes busy, the runtime should be able to release or retry the claim cleanly according to the documented policy
- note dispatch should no longer depend on the interactive or `master` agent being idle when another suitable worker is available

This addresses the most visible current symptom without weakening note integrity.

**Alternative considered:** keep watchdog note delivery effectively pinned to the interactive or `master` path. Rejected: that preserves the bottleneck that makes the local team underused.

### 5. Preserve exact targeting and honest busy outcomes under concurrency

Themion should keep explicit local targeting semantics even after concurrency is introduced.

Required behavior:

- explicit `to_agent_id` requests for board-note intake, Stylos talk, and Stylos task routing should still bind to the requested local agent when that agent exists
- if the explicitly requested local agent is already busy, the runtime should return or log a target-specific busy outcome rather than silently rerouting that work to another agent unless a future product rule explicitly allows that
- untargeted flows that already default to `master` may keep that behavior unless another documented selection policy is introduced in the same change
- task lifecycle reporting should continue to identify which local agent accepted, ran, or rejected the work

This preserves current user expectations around local agent identity and avoids surprising implicit reassignment.

**Alternative considered:** automatically spill targeted work onto any idle local agent. Rejected: that breaks the meaning of explicit target selection and makes multi-agent behavior harder to reason about.

### 6. Keep status, inspection, and transcript output truthful for overlapping turns

Once several local agents can run at once, user-visible observability must stop implying that only one local turn exists.

Required behavior:

- exported status snapshots should show per-agent busy state accurately while multiple local agents run concurrently
- system inspection and runtime debugging output should remain truthful about overlapping local activity
- transcript lines should continue to attribute visible work to the correct local agent, including overlapping tool and completion events from different agents
- any process-level `busy` summary that remains should be documented clearly as an aggregate indicator rather than a scheduler gate
- note/watchdog-related transcript or debug events should clarify which local agent received work and whether a note is pending versus already in-flight
- the TUI may expose a lightweight read-only overlay or panel for watchdog/background-agent visibility, such as watchdog pending state and local-agent busy/incoming-prompt status, as long as orchestration logic remains outside presentation-heavy UI code

This keeps the operator's mental model aligned with the runtime's actual behavior.

**Alternative considered:** defer observability cleanup until after concurrency lands. Rejected: hidden concurrency semantics would make verification and debugging much harder.

### 7. Keep the single-process architecture while narrowing shared coordination points

Themion should remain one process hosting multiple local agents, but shared mutable coordination should stop serializing unrelated work.

Required behavior:

- the runtime may still use shared registries, channels, and coordination helpers where needed, but those boundaries should be narrowed to the specific shared resource rather than whole-agent execution
- board-note claiming, task registry mutation, and roster mutation should remain safe under overlapping local turns
- local-agent create/delete behavior should continue to protect correctness when an agent is active; any restrictions should be explicit and agent-specific where practical
- the TUI should remain the presentation and event-forwarding surface rather than owning long-term concurrency policy itself
- implementation should prefer moving concurrency ownership into focused runtime helpers such as `app_state.rs`, `app_runtime.rs`, `board_runtime.rs`, or another dedicated non-TUI runtime module rather than growing more orchestration inside presentation-heavy TUI code

This keeps the architecture aligned with the repository's CLI/core boundary while avoiding accidental reintroduction of a one-lane runtime.

**Alternative considered:** solve concurrency by moving every local agent into its own OS process. Rejected: that is a much larger architecture change than needed for the current product gap and would entangle local concurrency with process management.

## Changes by Component

| File / area | Phase 1 implementation direction |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Remove or narrow the remaining global busy-gate assumptions for local-turn admission. The TUI should submit start-turn intents, receive completion/events, and render per-agent state, but should avoid remaining the owner of cross-agent concurrency policy. |
| `crates/themion-cli/src/board_runtime.rs` | Own watchdog-facing pending-note selection, note claim bookkeeping, and release/retry decisions so board-note dispatch stays concurrency-safe across several local agents. |
| `crates/themion-cli/src/app_runtime.rs` | Become the preferred home for per-agent admission checks, running-turn bookkeeping, and helper APIs that let other layers start a turn for one local agent without consulting a process-wide busy bit. |
| `crates/themion-cli/src/app_state.rs` and related runtime helpers | Ensure each local agent has the runtime-owned state needed for an independent turn slot, including busy/running markers and completion reconciliation hooks. |
| `crates/themion-cli/src/stylos.rs` | Preserve exact local-agent targeting, busy reporting, and task lifecycle output when several local agents may already be active concurrently. |
| `crates/themion-core/src/agent.rs` | Confirm core harness behavior remains safely agent-local when multiple local agent loops are active at once; adapt helper contracts only where CLI/runtime integration needs clearer per-agent ownership. |
| `docs/architecture.md` | Document the shift from one effective local busy lane to concurrent per-agent execution inside one process. |
| `docs/engine-runtime.md` | Document independent watchdog behavior, per-agent busy semantics, note-claim lifecycle, and concurrent local turn execution with preserved explicit targeting. |
| `docs/README.md` | Track the PRD entry now and later reflect landed implementation status. |

### Suggested Phase 1 implementation order

1. isolate the current global busy assumptions and identify the exact admission points for local turns
2. introduce per-agent running/admission state in runtime-owned helpers
3. switch local prompt, note-injection, and targeted request paths to consult per-agent admission instead of one global gate
4. separate watchdog note selection from claim/delivery so duplicate injection cannot occur under overlap
5. update status, transcript, and inspection plumbing so concurrent turns are observable and debuggable
6. run narrow validation first, then the required touched-crate feature checks

## Edge Cases

- one local agent is running a long tool-heavy turn while another idle worker has a pending targeted board note → verify: the watchdog still dispatches the pending note to the idle worker without waiting for the long turn to finish.
- two local agents are idle and one untargeted pending note becomes eligible → verify: exactly one agent claims and runs the note, with no duplicate concurrent injection.
- a note targets `smith-2` and `smith-2` becomes busy between eligibility check and delivery handoff → verify: the runtime records or reports the race cleanly and retries later without losing or duplicating the note.
- two different local agents stream assistant output concurrently → verify: transcript attribution remains correct and completion events map to the right agent.
- a Stylos task explicitly targets a busy local worker while another worker is idle → verify: the request gets a target-specific busy failure rather than silent rerouting.
- the interactive `master` agent is busy while a background worker is idle and has pending note work → verify: watchdog progress continues through the background worker.
- local agent deletion is requested while one worker is active and another is idle → verify: deletion policy remains safe and does not corrupt the running agent or shared roster.
- the watchdog wakes while several notes are already in-flight → verify: it does not re-inject duplicates for those notes and still considers other eligible backlog work.

## Migration

This change should remain additive within the current single-process architecture.

Rollout guidance:

- preserve the existing local team model, explicit `agent_id` targeting semantics, and board-note lifecycle
- replace the current effective single-lane execution behavior with per-agent concurrent execution inside the same process
- keep any remaining aggregate process-level busy fields only as observability summaries, not as scheduler admission gates
- document any transitional limits clearly if some flows become concurrent before others

## Testing

- start Themion with `master` plus one worker and submit a long-running prompt to `master` while a pending targeted note exists for the worker → verify: the worker starts its own turn before the `master` turn completes.
- create two workers and queue two independent pending notes for different target agents → verify: both agents can begin work without serializing behind one another.
- queue one untargeted eligible pending note while two workers are idle → verify: only one worker claims it, the note is not injected twice, and the in-flight state clears correctly after completion.
- send a Stylos-targeted request to a busy agent while another agent is idle → verify: the result is a target-specific busy outcome, not an implicit reroute.
- run several concurrent local turns and inspect exported status or system inspection → verify: per-agent busy state is accurate for all active agents at the same time.
- trigger overlapping tool execution and assistant streaming from different local agents → verify: transcript attribution stays correct and readable for each agent.
- leave one agent in a long-running turn and observe watchdog cadence over time → verify: watchdog scheduling continues in the background instead of waiting for that turn to complete.
- force a note-delivery handoff race where the chosen agent becomes busy after selection but before injection → verify: the note claim is released or retried cleanly without loss or duplication.

## Implementation checklist

### Phase 1 checklist

All Phase 1 checklist items below are implemented in the current repository state.

- [x] isolate the current global local-turn admission points and replace them with per-agent admission checks
- [x] give each local agent an independent running-turn slot, ownership path, and completion reconciliation path
- [x] keep the watchdog running on an independent background cadence while local agents are busy
- [x] separate note selection, claim, delivery, and release/retry handling at the watchdog scheduler boundary
- [x] make pending-note selection and claiming concurrency-safe across several local agents
- [x] preserve exact local-agent targeting and honest target-specific busy outcomes under concurrency
- [x] update exported status, inspection, and transcript-adjacent observability to reflect overlapping local turns truthfully
- [x] document the landed per-agent concurrency and independent watchdog behavior in `docs/architecture.md`, `docs/engine-runtime.md`, and `docs/README.md`

### Phase 2 follow-on ideas

These are postponed follow-on ideas for a future PRD, not remaining blockers for PRD-083 completion.

- [postponed] reduce remaining transitional orchestration still living in `tui.rs` if Phase 1 leaves compatibility shims there
- [postponed] refine untargeted backlog selection policy beyond the first deterministic stable rule if practical evidence shows a need
- [postponed] add richer debug surfaces for overlapping note claims, retries, and concurrent local turns if verification remains difficult
