# PRD-124: Queue User Prompts Per Agent While the Agent Is Busy

- **Status:** Implemented
- **Version:** v0.77.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-12

## Summary

Landed in `v0.77.0` as runtime-owned per-agent queueing for normal local user prompts. The implementation now enqueues same-agent local user follow-up while the target agent is busy, drains queued prompts into the same active turn before the next continuation round after tool work, and auto-starts exactly one next queued prompt for that same agent after full turn completion. Validation for this slice is intentionally lightweight and based on build checks plus manual runtime verification.

- Themion previously could panic when a new local user prompt reached an agent that was already busy.
- Normal chat submits to a busy local agent now enqueue on that same agent instead of failing or trying to launch immediately.
- If that agent's active turn continues after tool work, queued prompts present at the next continuation point are appended into that same agent's next model context in FIFO order.
- If that agent's active turn ends normally, the runtime auto-starts exactly the next queued prompt for that same agent.

## Goals

- Remove the busy-submit panic for normal local user chat prompts.
- Accept normal local user chat prompts while the target local agent is busy.
- Keep queue ownership strictly per local agent.
- Define exact drain behavior for queued prompts during tool-call continuation.
- Define exact auto-continue behavior after normal turn end.
- Keep queue ownership and admission policy in runtime/core layers, not in TUI presentation state.
- Make the feature specific enough that implementation should not need follow-up product clarification.

## Non-goals

- Do not add one global queue shared by all local agents.
- Do not change slash-command, login, shell-command, or indexing-command busy behavior in this PRD.
- Do not redesign Stylos inbox, remote-message delivery, or board-note intake queues.
- Do not merge prompts from different agents into one context.
- Do not persist queued prompts durably across process restart in this PRD.
- Do not redesign the overall turn scheduler beyond this local user-prompt queue feature.

## Background & Motivation

### Current state

The current local user-submit flow assumes that a turn either starts immediately or should not start at all.

In `crates/themion-cli/src/app_runtime.rs`, turn launch preparation marks an `AgentHandle` busy and then extracts the `Agent` object from `handle.agent`. If another submit path still reaches that launch step while the same handle is already running, the code can reach:

- `handle.agent.take().expect("agent available when not busy")`

That panic means the current product behavior is not queue-safe.

Themion also already has an important runtime split:

- a full local turn ends only when `themion_core::agent::Agent::run_loop_with_cancellation(...)` returns to the CLI runtime
- tool-call continuation happens inside that same active run loop before `TurnDone`

Because of that split, post-turn queue draining alone is not enough for the requested feature. The active agent must also be able to absorb queued prompts before a continued model round inside the same run.

### Why this matters now

The intended user experience is straightforward:

- if a local agent is busy, another normal prompt to that same agent should queue
- if that active turn is still continuing after tool calls, queued follow-up should join the next round's context for that same agent
- if the active turn finishes, the same agent should continue with its next queued prompt automatically

Without an explicit queue, the system either rejects useful follow-up or reaches a panic path.

## Design

### 1. Queue ownership and scope

Queued prompts must be owned per local agent.

Required behavior:

- maintain one FIFO queued-prompt list per `AgentHandle`
- queue ownership must follow the same local-agent identity used for turn admission and busy state
- one local agent must never read, drain, or auto-start another local agent's queued prompts
- app-level `agent_busy` or aggregate-busy flags remain observability fields only; they are not queue owners

Implementation rule:

- runtime-owned queued-prompt state belongs in `themion-cli` runtime/app-state structures, not in TUI-only display state such as `runtime.pending`

### 2. Covered submit path

This PRD covers normal local user chat submits only.

Covered path:

- ordinary submitted chat text that currently flows through `submit_text_default(...)`, `submit_text_to_agent(...)`, or the same local-agent routing path with Stylos enabled

Not covered in this PRD:

- slash commands
- shell commands
- login commands
- indexing commands
- remote inbox or board-note intake behavior

Those paths may keep current busy handling unless a later PRD changes them.

### 3. Busy local-user submit behavior

When a covered prompt targets a busy local agent, Themion must enqueue it on that same agent instead of trying to launch immediately.

Required behavior:

- still show the submitted user text in the live transcript immediately using the current local user-message style
- if the target local agent is idle, keep current immediate-launch behavior
- if the target local agent is busy, append one queue item to that same agent's FIFO queue and return without calling the immediate turn-launch path
- do not emit `busy, please wait` for this covered case
- do not panic

Queue item fields for this PRD:

- prompt text
- enqueue timestamp in milliseconds

This PRD does not require a larger metadata shape unless implementation needs one for local tracing.

### 4. Persistence and transcript semantics

Queued prompts must be visible to the user immediately, but they must enter the core conversation history only when the owning agent actually consumes them.

Required behavior:

- on enqueue, the prompt is shown in the live transcript immediately
- on enqueue, the prompt is not yet appended to the owning core `Agent.messages` history
- on enqueue, the prompt is not yet written into the session message history as a consumed agent-turn user message
- when the owning active turn drains queued prompts into a continuation round, each drained prompt becomes a real `user` message in that current turn and must be appended to in-memory conversation plus persisted through the normal per-message DB path for that turn
- when a queued prompt starts a fresh next turn after turn end, it becomes the initial `user_input` of that new turn and follows the normal turn-start persistence path for a new turn

This keeps transcript visibility immediate while keeping persisted conversation semantics aligned with when the agent actually consumes the queued prompt.

### 5. Continuation-round queue drain behavior

The active core agent must be able to absorb queued prompts before a continued model round after tool work.

Required behavior:

- add a minimal queue-drain hook that the active `Agent` can call while `run_loop_with_cancellation(...)` is still running
- the hook must drain queued prompts only for that same local agent
- the drain point must happen after the prior round's tool execution is complete and before the next model request is built
- at each drain point, drain all currently queued prompts for that same agent in FIFO order
- append the drained prompts as ordinary `user` messages in FIFO order before building the next prompt context
- if no queued prompts are present at that drain point, current behavior stays unchanged

Drain timing rule:

- do not drain before the first model round of a new turn
- do drain before each later model round that continues the same turn after tool work

This exact timing is what lets queued prompts join the same agent's continuing turn instead of waiting for `TurnDone`.

### 6. Turn-end auto-continue behavior

After a full turn finishes and the `Agent` object returns to the CLI runtime, the owning local agent must continue with queued work if any remains.

Required behavior:

- when `handle_agent_ready_event(...)` or equivalent runtime ready handling marks a local agent not busy, check only that same agent's queue
- if the queue is empty, keep current idle behavior
- if the queue is non-empty, remove exactly one queued prompt from the front of that same agent's queue and start one new turn with it immediately
- if more queued prompts remain after that launch, they stay queued for later turns or later continuation-drain points
- one agent finishing must not trigger queue work on another agent

This keeps turn boundaries simple:

- continuation drain absorbs all currently queued prompts into the same active turn
- post-turn auto-continue starts exactly one next queued prompt as the next turn

### 7. Interrupt behavior

Interrupt handling must stay explicit and queue-safe.

Required behavior:

- interrupting a running agent must interrupt only that current active turn
- interruption must not silently drop that agent's queued prompts
- interruption must not transfer queued prompts to another agent
- after interruption, any queued prompts still owned by that agent remain queued and may run through the normal next-turn path unless a future PRD defines a different policy

This PRD keeps the queue policy simple by preserving queued prompts across interruption.

### 8. Queue-safe runtime ownership

The queue must be enforced by runtime-owned admission logic, not by TUI-only checks.

Required behavior:

- the local user-submit path must check target-agent busy state before trying to extract the `Agent` object for immediate execution
- covered busy-submit cases must route into enqueue logic instead of the immediate-launch path
- `app_runtime.rs` and `app_state.rs` must remain the source of truth for queue admission and same-agent follow-up launch
- `tui.rs` must only submit intents and render resulting queued/pending state
- internal invariants may still assert truly impossible states, but the ordinary covered busy-submit case must no longer depend on `expect("agent available when not busy")`

### 9. Presentation and status behavior

Status surfaces must reflect real queued state without inventing a second queue model in TUI.

Required behavior:

- pending/queued presentation must derive from runtime-owned per-agent queue state
- if one agent has queued prompts and another does not, presentation must not imply one shared global queue
- transcript output should remain compact; this PRD does not require a new verbose queue-event log line for every enqueue or drain
- current pending/status helpers may be reformatted as needed, but their data source must become the real per-agent queue state

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/app_state.rs` | Add runtime-owned per-agent queued-prompt storage, enqueue covered busy local-user submits, derive pending state from the real queue, and auto-start exactly one next queued prompt for the same agent after full turn completion. |
| `crates/themion-cli/src/app_runtime.rs` | Make covered turn launch queue-safe, prevent the current busy-submit panic path, and keep `AgentHandle` busy/availability state coherent with queue admission. |
| `crates/themion-cli/src/tui.rs` | Keep submit and status rendering aligned with runtime-owned queue state without introducing TUI-owned queue policy. |
| `crates/themion-core/src/agent.rs` | Add a minimal same-agent queue-drain hook inside `run_loop_with_cancellation(...)`, append drained prompts as ordinary `user` messages before each continued model round after tool work, and persist those messages through the current turn's normal DB append path. |
| queue-related tests | Add focused tests for enqueue behavior, same-agent continuation drain, same-agent post-turn auto-continue, FIFO ordering, persistence timing, and no cross-agent leakage. |
| `docs/engine-runtime.md` | Document the implemented per-agent queued local-user prompt behavior, including continuation drain timing and post-turn auto-continue rules. |
| `docs/README.md` | Track this PRD and later update status/version when implemented. |

## Edge Cases

- submit one prompt to agent A while agent A is busy and agent B is idle → verify: the prompt queues only on agent A and does not affect agent B.
- submit several prompts to the same busy agent → verify: they preserve FIFO order.
- submit prompts to two different busy agents → verify: each agent keeps an independent queue.
- a queued prompt is submitted while the owning agent is between tool completion and the next continuation drain point → verify: it is included if it is already queued when that drain point executes.
- a turn continues after tool calls and that same agent has queued prompts → verify: all currently queued prompts drain into that same turn before the next model round.
- a turn ends with no queued prompts → verify: current idle behavior stays unchanged.
- a turn ends with several queued prompts → verify: exactly one next queued prompt starts as a new turn and the rest remain queued.
- one agent finishes while another agent still has its own active run → verify: only the finished agent's post-turn queue behavior runs.
- the running agent is interrupted while it still has queued prompts → verify: the active turn stops, queued prompts remain owned by that same agent, and no panic occurs.

## Migration

No database migration is required.

This is a minor-version user-visible behavior change because it adds local prompt-queue semantics:

- covered busy local-user prompts are accepted as queued work instead of failing or panicking
- continuation rounds can absorb queued same-agent follow-up prompts
- completed turns can auto-start the same agent's next queued prompt

Persistent cross-restart prompt queues remain out of scope.

## Testing

This PRD is closed with intentionally lightweight validation for this slice.

- manually submit a covered local user prompt to an idle agent → verify: the turn starts immediately and current behavior is unchanged.
- manually submit a second covered local user prompt while that same agent is busy → verify: the second prompt is queued and no panic occurs.
- manually exercise a turn that continues after tool work with queued same-agent follow-up → verify: queued prompts join the same continued turn in FIFO order.
- manually exercise a completed turn with queued same-agent follow-up → verify: exactly one next queued prompt auto-starts for that same agent.
- manually inspect pending/status behavior while queueing prompts → verify: status reflects per-agent queued ownership.
- run `cargo check -p themion-core` → verify: core queue-drain changes compile.
- run `cargo check -p themion-cli` → verify: default CLI build compiles.
- run `cargo check -p themion-cli --features stylos` → verify: Stylos-enabled CLI build still compiles.
- run `cargo check -p themion-core -p themion-cli --all-features` → verify: touched crates still build across feature combinations.

## Implementation checklist

- [x] add runtime-owned FIFO queued-prompt storage per local agent in `themion-cli`
- [x] route covered busy local-user submits into same-agent enqueue instead of immediate launch
- [x] keep enqueue transcript visibility immediate while delaying conversation-history persistence until actual consumption
- [x] make covered launch paths queue-safe and remove the current busy-submit panic behavior
- [x] add a same-agent queue-drain hook to `Agent::run_loop_with_cancellation(...)`
- [x] drain all currently queued prompts for that same agent before each continued model round after tool work
- [x] append drained prompts as ordinary `user` messages and persist them through the current turn's normal DB path
- [x] auto-start exactly one next queued prompt for the same agent after full turn completion
- [x] preserve queued prompts across interruption of the active turn
- [x] keep validation lightweight for this slice: build checks plus manual runtime verification of queue behavior
- [x] update `docs/engine-runtime.md` when implementation lands
