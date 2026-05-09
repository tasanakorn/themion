# PRD-110: Agent Coordination Guidance for Board Notes, Inbox Messages, and Local Delegation

- **Status:** Implemented
- **Version:** v0.68.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-09

## Summary

- Add clear model guidance for coordinating work across local and remote agents.
- Use durable board notes for delegated work that must be tracked, resumed, or completed later.
- Use volatile inbox messages only for short coordination, clarification, and participant-facing state updates.
- Teach `master` to consider creating local worker agents and delegating with explicit board-note return instructions.
- Add a compact facilitation protocol for multi-agent activity: one state owner, stable ids, clear response channels, and explicit completion rules.

## Goals

- Give agents a simple channel-choice rule for self-notes, delegated board notes, inbox messages, task requests, and local-agent creation.
- Make durable board notes the normal path for async delegated work.
- Make inbox messages useful without letting them become a hidden durable task queue.
- Improve `master` behavior as a coordinator: delegate when useful, state expected results, and summarize outcomes to the human.
- Reduce multi-agent chatter by requiring clear ownership, response channels, meaningful state broadcasts, and final wrap-up.
- Keep the guidance compact enough for prompt injection and aligned with runtime/app-state ownership rules.

## Non-goals

- Do not add a new board primitive, task primitive, or inbox persistence.
- Do not turn inbox messages into read receipts, durable chat threads, or task-result workflows.
- Do not require notes or worker agents for simple direct answers.
- Do not require a state broadcast for every internal coordinator change.
- Do not move board, inbox, or local-agent policy into `tui.rs`.
- Do not revive `talk` terminology in user-facing or model-facing guidance.

## Background & Motivation

Themion now has several coordination channels:

- local agent tools create or remove process-local worker agents
- board notes store durable work requests, result text, and done mentions
- `stylos_send_message` sends short peer messages into a volatile inbox
- task requests provide remote task ids, status, and result polling

Recent PRDs clarified the mechanics. PRD-107 made peer-message delivery queue instead of failing only because the target is busy. PRD-109 renamed the surface from `talk` to message/inbox terms and kept the inbox volatile. Earlier board-note PRDs kept durable collaboration centered on notes.

The remaining problem is guidance. Agents need to know which channel to use. If this is unclear, `master` may use volatile inbox messages for real delegated work, skip useful local-worker delegation, or coordinate several agents without a single state owner.

Multi-agent work needs less ambiguity and less chatter. The coordinator should say who owns state, who must act, where replies go, and when the activity is done. Durable work needs durable notes. Short coordination can use inbox messages. Mixing those roles makes work harder to resume, audit, and complete.

## Design

### 1. Channel-choice ladder

Prompt guidance must teach this decision ladder:

1. **Direct answer** — answer directly when the user request is simple and no tracking is needed.
2. **Self-note** — create a durable self-note when the current agent needs to track non-trivial or branching work.
3. **Local delegation** — when extra capacity or role separation helps, create or choose a local worker agent and delegate with a board note.
4. **Board note** — use a durable board note for work another agent must complete, resume, or report back later.
5. **Inbox message** — use `stylos_send_message` for short volatile coordination, urgent nudges, clarifying questions, participant-facing state updates, or final no-durable-result wrap-up.
6. **Task request** — use Stylos task requests when the sender needs a remote task id plus status/result polling.

Required behavior:

- do not use inbox messages as the primary record for work that needs completion tracking
- do not create board notes for simple direct answers or tiny one-shot actions
- do not create a worker agent unless delegation materially improves speed, focus, or role separation
- keep tool descriptions short; put the full decision rule in prompt/docs guidance rather than expanding every tool schema

### 2. `master` delegation behavior

`master` remains the team leader and human-facing coordinator. It should not automatically do every task itself.

Required behavior:

- for non-trivial work, `master` should consider whether a local worker agent should handle a slice of the task
- if no suitable worker exists, `master` may create one with `local_agent_create`
- delegated work that needs completion tracking should normally be sent as a durable board note
- the board note must state the task, expected output, relevant constraints, and return path
- if a durable result is needed, the note must ask the worker to update the note result or create a done mention through the board workflow
- `stylos_send_message` may be used for quick clarification about a delegated task, but not as the only record for that task
- when human-facing work completes, `master` should summarize what was delegated, what came back, and what remains open if anything

This keeps local agents useful without making delegation mandatory for small work.

### 3. Inbox message use

The inbox is volatile, process-local, and best-effort. Guidance must make this visible to models.

Use inbox messages for:

- quick clarification between agents
- urgent lightweight coordination
- state updates in an active coordinated activity
- asking whether a peer is available for a small immediate exchange
- final wrap-up when no durable result is needed

Do not use inbox messages for:

- assigning work that must be tracked to completion
- storing decisions that must survive restart or context loss
- replacing a board note only because messaging is faster
- thank-you-only or empty acknowledgements
- long discussion that should become a note, task request, or concise human-facing summary

Injected peer-message guidance should keep the existing rule: reply only when useful, avoid empty acknowledgements, and include `***QRU***` when no further reply is needed.

### 4. Multi-agent facilitation protocol

When several agents participate in one activity, the coordinator must use a small explicit protocol.

Required behavior:

1. assign one coordinator as the authoritative state owner
2. state the task, participants, response channel, and expected reply shape
3. use stable identifiers for the activity, turn, note, or delegated work item
4. distinguish authoritative state updates from discussion
5. broadcast meaningful state transitions promptly: start, major turn/result changes, and completion
6. define completion, timeout, and late-input rules up front, then apply them consistently
7. say who is waiting on whom and by when when a response is needed
8. end with the final outcome and whether further replies are needed or ignored

Only participant-relevant changes need broadcasts. Internal bookkeeping should stay quiet.

**Alternative considered:** require a broadcast for every state change. Rejected because it creates noise and encourages over-messaging.

### 5. Prompt and documentation placement

The durable guidance should live where models learn collaboration behavior.

Expected placement:

- `themion-core` role and collaboration prompt guidance
- concise tool descriptions only where a contract reminder improves channel choice
- runtime docs that explain board notes, inbox messages, task requests, and local agents as separate channels
- repository instruction guidance if a durable human-maintained rule needs to mirror the prompt behavior

The TUI may display outcomes, but it must not own channel-choice or coordination policy.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Update role/collaboration guidance for channel choice, `master` delegation, board-note return instructions, inbox use, and multi-agent facilitation. |
| `crates/themion-core/src/tools.rs` | Keep tool descriptions aligned with the decision rule; update only concise wording if needed. |
| `crates/themion-cli/src/app_state.rs` / `app_runtime.rs` / `board_runtime.rs` | Preserve runtime-owned coordination behavior; change only if implementation needs prompt-wiring or status support outside core. |
| `docs/engine-runtime.md` | Document the channel-choice ladder and board-note-vs-inbox distinction after prompt/runtime guidance changes. |
| `docs/architecture.md` | Keep multi-agent coordination and runtime ownership documentation aligned. |
| `docs/README.md` | List PRD-110 with status, version target, and scope. |
| `docs/prd/prd-110-agent-coordination-guidance.md` | Define the product requirement for improved coordination guidance. |

## Edge Cases

- user asks a simple direct question → verify: the agent answers directly without a note or worker agent.
- user asks for multi-step implementation → verify: `master` considers a self-note or local-worker delegation.
- `master` delegates work that needs a result → verify: the board note includes task, expected output, constraints, and return path.
- an agent needs quick clarification → verify: `stylos_send_message` is allowed and the message is concise.
- an inbox message assigns real async work → verify: the receiver asks for or creates a board note rather than treating the volatile message as durable assignment.
- several agents participate in one activity → verify: one coordinator owns state and broadcasts only meaningful participant-relevant transitions.
- work finishes while late messages arrive → verify: the coordinator states the final outcome and applies the late-input rule consistently.
- a response is expected from a specific participant → verify: the note or message says who must respond, where to respond, and by when if timing matters.

## Implementation Notes

Implemented in v0.68.0. The landed slice updates model-facing role and board/coordination guidance in `themion-core`, keeps tool schema wording concise while clarifying board notes versus volatile inbox messages, documents the channel-choice ladder in runtime and architecture docs, and preserves TUI/runtime ownership boundaries. No new board, inbox, or task primitive was added.

## Migration

This is a prompt and documentation guidance change. No database migration is required.

Existing board notes, inbox messages, and task request behavior remain valid. The change should make future model choices more consistent without changing stored data.

The target version is `v0.68.0` because this changes user-visible multi-agent workflow behavior and model-facing instructions.

## Testing

- inspect generated prompt guidance for `master` → verify: it says to consider local-agent delegation for non-trivial work and names durable board notes as the normal delegated-work path.
- inspect generated prompt guidance for non-master workers → verify: it does not grant leader authority but still explains useful note/inbox response behavior.
- inspect `stylos_send_message` guidance → verify: it remains a short volatile peer-message tool, not a durable task tool.
- simulate or unit-test peer-message injection → verify: receiver guidance discourages empty acknowledgements and preserves `***QRU***` semantics.
- review docs → verify: board notes, inbox messages, task requests, and local-agent delegation are distinct channels.
- run `cargo check -p themion-core` → verify: core prompt/tool changes compile.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` if CLI code changes land → verify: default CLI build compiles.
- run `cargo check -p themion-cli --features stylos` and `cargo check -p themion-cli --all-features` if Stylos or CLI runtime code changes land → verify: feature combinations compile.

## Implementation checklist

- [x] update built-in collaboration guidance with the channel-choice ladder
- [x] update `master` role guidance to consider local-agent creation and durable board-note delegation for non-trivial work
- [x] ensure delegated board-note guidance requires explicit result and return-path instructions
- [x] ensure inbox guidance says volatile, concise, useful-reply-only, and not durable work tracking
- [x] add compact multi-agent facilitation rules to prompt guidance
- [x] remove or avoid any new model-facing `talk` wording outside historical references
- [x] update docs that describe board notes, inbox messages, task requests, and local agents
- [x] verify wording remains compact enough for prompt budget
- [x] run default and all-feature validation for touched crates
