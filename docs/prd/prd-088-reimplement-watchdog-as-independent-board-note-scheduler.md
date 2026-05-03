# PRD-088: Reimplement the Watchdog as an Independent Board-Note Scheduler

- **Status:** Implemented
- **Version:** v0.56.0
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-02

## Implementation status

Landed in `v0.56.0` as a watchdog runtime rewrite in `themion-cli`. The shipped implementation removes the previous watchdog path that routed local board-note follow-up through remote incoming-prompt admission semantics, preserves the existing pending-note query and local claim logic, and runs the watchdog as an independent bounded-sleep scheduler that inspects eligible idle local agents and injects pending board-note work directly into the selected local agent turn path.

Implementation notes:

- The earlier incorrect implementation shape treated watchdog-local board-note pickup too much like incoming-prompt delivery, which blurred local board scheduling with remote request-admission behavior and made regressions easier to reintroduce.
- The landed implementation keeps board-note selection, local claim/release protection, note-continuation, and done-mention follow-up in CLI-local runtime coordination instead of making them TUI-owned or remote-intake-owned behaviors.
- Feature-flag behavior is intentionally narrower than the earlier coupling implied: enabling `stylos` changes transport/discovery/instance-identity behavior, but core local watchdog and board-note flows still exist outside Stylos transport itself.
- In Stylos-enabled builds, the watchdog may match the current concrete local instance id and its same-process sibling alias (`hostname:pid` and `local:pid`) so locally targeted notes continue to work across those two concrete identity forms. It must not fall back to a broad bare `local` match that would let a newly started instance claim stale generic local notes.

## Summary

- The current watchdog implementation has regressed repeatedly because it mixes two different responsibilities: remote incoming-prompt intake and local background board-note follow-up.
- This PRD removes the current watchdog implementation completely and replaces it with a simpler independent scheduler that only handles local pending board-note follow-up.
- The new watchdog must run in its own loop, sleep between checks, respect cooldowns, inspect agents individually for idleness, and inject pending note work without depending on incoming-prompt state.
- Preserve only the existing pending-note query logic and note-selection semantics; everything else about the current watchdog implementation should be treated as disposable.
- Remote incoming prompts, Stylos request delivery, and watchdog board-note follow-up must become clearly separate runtime paths so future refactors do not reintroduce the same coupling bug.

## Goals

- Completely remove the current watchdog implementation and reimplement it from a clean simpler model.
- Keep the watchdog focused on one job: scheduling pending local board-note follow-up for eligible idle local agents.
- Ensure the watchdog runs independently of current agent turns and independently of remote incoming-prompt handling.
- Make watchdog behavior easy to reason about: loop, sleep, inspect idle agents, claim one pending note, inject it, and wait for the next cycle.
- Preserve the existing pending-note query behavior and note ordering semantics already provided by the DB/query layer.
- Reduce the chance of future regressions by removing shared state and mixed semantics that are not essential to watchdog behavior.

## Non-goals

- No redesign of board note columns, note ordering, note completion semantics, or done-mention behavior.
- No redesign of remote Stylos incoming-prompt delivery in this PRD beyond ensuring it is no longer shared with watchdog logic.
- No attempt to turn the watchdog into a general automation framework in this slice.
- No requirement to preserve the current watchdog state structs, event names, or internal helper layering if they complicate the clean reimplementation.
- No requirement to change the existing DB query that finds the next pending note for a local target, except where a tiny compatibility adjustment is strictly necessary.

## Background & Motivation

### Current state

The intended watchdog behavior is simple:

- it runs in its own task loop
- it sleeps so it does not burn CPU
- it checks whether a local agent is idle
- it looks for pending note work for that idle agent
- it injects that note work when appropriate
- it does not act as incoming-prompt intake machinery

The current implementation no longer matches that model cleanly.

Recent refactors have left the watchdog coupled to:

- `IncomingPromptRequest`
- `incoming_prompts`
- shared active-incoming-prompt bookkeeping
- remote intake acceptance/rejection helpers
- TUI/runtime glue that was designed for external prompt delivery rather than local watchdog scheduling

That coupling has caused repeated regressions in exactly the same area: watchdog behavior stops being truly independent and begins inheriting semantics that belong to remote prompt intake instead.

### Why a full reimplementation is the right fix

The problem is no longer one isolated conditional bug. The current implementation shape is itself the problem.

When the watchdog and remote incoming prompts share state, one change in intake handling can accidentally break watchdog scheduling. When the watchdog is modeled as an incoming prompt, local background note follow-up stops being conceptually separate from external work delivery.

This PRD therefore chooses a stronger corrective action:

- remove the current watchdog implementation entirely
- preserve only the pending-note lookup logic and current note-selection semantics
- rebuild the watchdog from a minimal scheduler model that matches the intended product behavior directly

**Alternative considered:** continue patching the current watchdog incrementally. Rejected: this area has already regressed repeatedly, which is evidence that the current design boundary is too tangled to trust.

## Design

### 1. Remove the current watchdog implementation instead of evolving it in place

The current watchdog implementation should be treated as replaceable, not as the foundation for more patches.

Required behavior:

- remove the current watchdog-specific runtime state, dispatch path, and intake coupling that exist only to support the current implementation shape
- do not preserve current helper boundaries just because they already exist
- keep only the board-note query/selection logic that determines which pending note is next for a local target
- reintroduce only the minimum state needed by the new scheduler

The point of this PRD is to restore clarity, not to wrap the existing implementation in another abstraction layer.

### 2. The watchdog must be an independent local scheduler, not an incoming-prompt producer

The watchdog should own one narrow runtime responsibility: local pending board-note follow-up.

Required behavior:

- the watchdog runs as its own long-lived background task
- it does not route work through remote incoming-prompt planning or admission machinery
- it does not represent watchdog board-note delivery as a remote incoming prompt
- it does not depend on `incoming_prompts`, remote task registry semantics, or incoming-prompt acceptance state to know whether it may run
- it may publish transcript/debug events about its decisions, but those events are observational, not its scheduling source of truth

This is the main architectural correction in the PRD.

**Alternative considered:** keep using `IncomingPromptRequest` for watchdog work but clean up the flags. Rejected: that preserves the semantic mix-up that caused the regressions.

### 3. The watchdog loop should be simple, explicit, and cooldown-based

The new watchdog loop should follow a direct scheduler shape rather than a stateful event maze.

Required behavior:

- the loop runs on its own runtime task
- each cycle sleeps for a bounded interval so it does not spin or consume unnecessary CPU
- after waking, it inspects the current local runtime state
- it evaluates candidate local agents individually for eligibility
- when it finds an eligible idle agent, it checks whether that agent has pending note work
- if so, it claims and injects at most one pending note for that agent during that scheduling step
- after injection, the loop continues on later cycles rather than trying to drain all work in one hot loop

Required scheduler controls:

- a fixed sleep interval between checks so console/CPU usage stays low
- a cooldown or idle-delay rule before injecting work to an agent that just became idle
- no tight retry loop when no work is available
- no repeated duplicate injection of the same note while that note is already in flight locally

The exact timing constants may remain implementation details as long as they are explicit and documented.

### 4. Eligibility must be based on per-agent idleness, not global incoming-prompt state

The watchdog should reason about agents one by one.

Required behavior:

- evaluate each local agent independently
- use per-agent idle/busy truth as the scheduling input
- one busy agent must not suppress watchdog checks for another idle eligible agent
- an agent handling remote incoming work may still be busy and therefore ineligible, but that is a property of that agent, not of the whole watchdog loop
- local note follow-up should be assignable to another eligible idle local agent while one agent is already busy

The scheduler may still expose aggregate observability, but aggregate state must not be the admission gate.

### 5. Preserve the existing pending-note query logic and note-selection semantics

The current note query behavior is the one part of the implementation this PRD intends to keep.

Required behavior:

- preserve the existing DB/query path that identifies the next pending note for a local target
- preserve current ordering semantics for which note wins next
- preserve the existing local note-claim protection against duplicate in-process injection
- preserve the current completion-follow-up semantics for note-driven work after injection, even if the mechanism used to remember watchdog ownership changes

This PRD is about scheduler ownership and delivery shape, not about changing which note should be chosen.

### 6. Keep watchdog-owned note assignment state separate from remote incoming-prompt state

The new implementation should have a dedicated local state path for watchdog-assigned note work.

Required behavior:

- if the runtime needs to remember that an agent is currently handling watchdog-assigned note work, store that in watchdog-owned local assignment state rather than in remote incoming-prompt state
- note-continuation and done-mention follow-up for watchdog-assigned work must read from that watchdog-owned assignment state
- remote task completion or failure semantics must remain attached only to remote incoming work, not to watchdog-originated local note scheduling
- the scheduler should be able to release or clear watchdog-owned assignment state cleanly when a note finishes, is deferred, or loses a handoff race

This keeps the two systems conceptually separate while preserving the useful follow-up behavior already expected by board-note flows.

### 7. Keep visible watchdog observability, but do not let transcript/debug state drive scheduling

The user should still be able to see what the watchdog is doing.

Required behavior:

- transcript or status/debug surfaces should show clear watchdog-originated events when the watchdog claims or injects local note work
- the visible wording should make it clear that the source is the watchdog
- debug/read-only status may include fields such as pending local watchdog note state, idle-agent eligibility, or current watchdog assignments if useful
- none of these display surfaces should become the scheduling authority or the place where watchdog policy is reconstructed

This preserves operator visibility without reintroducing TUI-owned scheduler logic.

### 8. TUI ownership must remain presentation-only

The watchdog reimplementation must respect the repository architecture boundary.

Required behavior:

- the TUI may display watchdog state and forward human commands
- the TUI must not own watchdog scheduling, cooldown decisions, agent selection, or note-assignment policy
- watchdog behavior belongs in non-TUI runtime/app-state/orchestrator code
- any TUI changes in this PRD should be strictly limited to rendering the state or events produced by the runtime-owned scheduler

**Alternative considered:** keep some watchdog sequencing in `tui.rs` because the TUI already renders watchdog state. Rejected: that would repeat the same architecture drift that prior PRDs already tried to remove.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/app_runtime.rs` | Remove current watchdog-specific state and helper logic that models watchdog behavior through incoming-prompt machinery. Add only the runtime-owned primitives needed for the new independent watchdog scheduler and watchdog-owned note-assignment state. |
| `crates/themion-cli/src/app_state.rs` | Rewire runtime startup and event handling so watchdog work is scheduled and injected through its own local path rather than through incoming-prompt intake. Keep TUI-facing state publication and follow-up integration aligned with the new scheduler. |
| `crates/themion-cli/src/board_runtime.rs` | Preserve and reuse the existing pending-note query/claim logic, while adapting any handoff helpers needed by the new watchdog scheduler path. |
| `crates/themion-cli/src/stylos.rs` | Remove watchdog-specific coupling to remote incoming-prompt semantics where present, while preserving remote Stylos request behavior. |
| `crates/themion-cli/src/tui_runner.rs` | Start/stop the independent watchdog task through runtime-owned wiring only if this remains the correct runner boundary after implementation review. |
| `crates/themion-cli/src/tui.rs` | Limit changes to watchdog rendering/debug visibility only; no watchdog scheduling logic should live here. |
| `docs/architecture.md` | Update the watchdog description so it is documented as an independent local board-note scheduler rather than as an incoming-prompt-adjacent runtime path. |
| `docs/engine-runtime.md` | Update runtime docs to describe the clean separation between remote incoming prompts and watchdog-owned local board-note scheduling. |
| `docs/README.md` | Add this PRD and later reflect implementation status when the clean reimplementation lands. |

## Edge Cases

- one local agent is busy with a long-running task while another agent is idle and has pending note work → verify: the watchdog still checks the idle agent and may inject note work there.
- an agent becomes idle only briefly between nearby work items → verify: cooldown/idle-delay rules prevent overly eager injection.
- no eligible agent has pending note work → verify: the watchdog sleeps and retries later without hot-looping.
- one note is already claimed or in flight locally → verify: the watchdog does not duplicate that injection to another local agent.
- a watchdog-selected agent loses the handoff race and becomes busy before injection completes → verify: the local claim/assignment is released or retried cleanly according to runtime policy.
- remote incoming prompts are active for one agent while another agent is idle → verify: watchdog note scheduling for the idle agent still works and does not rely on incoming-prompt state.
- watchdog-assigned note work finishes and needs note-continuation or done-mention follow-up → verify: follow-up logic still works using watchdog-owned assignment state rather than remote incoming-prompt state.

## Migration

This is an internal runtime reimplementation with no external data migration.

Required rollout shape:

- remove the existing watchdog implementation completely rather than layering the new scheduler on top of it
- keep the pending-note query/selection logic intact
- introduce the new independent watchdog loop and watchdog-owned note-assignment state
- reconnect note follow-up and transcript/debug visibility to the new scheduler path
- update docs so the implemented watchdog model matches the documented architecture

## Testing

- run TUI mode with Stylos-enabled local multi-agent support and one busy agent plus one idle eligible agent → verify: watchdog still checks the idle agent and can inject pending note work in parallel.
- keep all agents busy for longer than the watchdog sleep interval → verify: watchdog wakes, observes no eligible idle agent, and does not inject work.
- let one agent become idle for less than the configured cooldown → verify: watchdog does not inject too early.
- let one agent remain idle past the configured cooldown with a pending local note → verify: watchdog injects exactly one pending note for that agent.
- leave no matching pending note for an idle agent → verify: watchdog loops with bounded sleep and does not hot-spin or emit duplicate noise.
- deliver a real remote incoming prompt while watchdog-managed note work exists elsewhere → verify: remote intake and watchdog scheduling remain independent and do not share assignment state incorrectly.
- complete watchdog-assigned note work that should emit a done mention → verify: note follow-up still occurs correctly without using remote incoming-prompt bookkeeping.
- inspect `tui.rs`, `app_state.rs`, and `app_runtime.rs` after implementation → verify: watchdog scheduling logic lives outside the TUI and outside remote incoming-prompt planning paths.

## Implementation checklist

- [x] remove the current watchdog implementation and its incoming-prompt coupling
- [x] preserve the existing pending-note query/selection logic
- [x] implement a new independent watchdog task loop with bounded sleep
- [x] implement per-agent eligibility checks based on idle/busy state
- [x] add explicit cooldown/idle-delay handling for watchdog injection
- [x] add watchdog-owned local note-assignment state separate from remote incoming prompts
- [x] reconnect note follow-up handling to the new watchdog-owned assignment path
- [x] preserve duplicate-injection protection for locally claimed notes
- [x] keep watchdog scheduling logic out of `tui.rs`
- [x] update `docs/architecture.md`, `docs/engine-runtime.md`, and `docs/README.md`
- [x] run `cargo check -p themion-cli --features stylos`
- [x] run `cargo check -p themion-cli --all-features`
- [x] run `cargo test -p themion-cli --all-features`
