# PRD-034: Note-First Multi-Agent Collaboration and Done Mentions

- **Status:** Implemented
- **Version:** v0.20.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-22

## Summary

- Prefer durable notes over direct `_talk` between agents for asynchronous multi-agent collaboration.
- Treat `_talk` as an interrupting realtime path that should be avoided when a durable note is the better fit.
- When an agent finishes a note created by another agent, automatically create a return note for the requester with the result.
- Keep that return notification useful and one-way; do not allow automatic done notifications to bounce forever between agents.
- Preserve the existing durable note board and idle-time note injection model rather than redesigning collaboration around chat-like reply threads.
- Keep human/user conversations unchanged; this PRD is about agent-to-agent collaboration behavior and guidance.

## Goals

- Make durable notes the default collaboration mechanism for agent-to-agent asynchronous work.
- Reduce avoidable agent interruption caused by using `_talk` for work requests that fit the board model better.
- Ensure that when one agent completes another agent's requested note, the requester receives a durable completion notice with the result.
- Preserve note traceability across requester and worker agents without turning notes into an unbounded reply chain.
- Keep the current board lifecycle and note injection model while improving completion handoff behavior.
- Make the collaboration guidance explicit in prompts and docs so agents choose notes more consistently.

## Non-goals

- No removal of Stylos talk or equivalent realtime peer-message capability.
- No redesign of the `todo` / `in_progress` / `done` columns.
- No introduction of a general threaded conversation system for notes.
- No attempt to automatically synchronize every board change back to the requester.
- No requirement to create done notifications for user-authored or system-local notes that did not originate from another agent.
- No distributed deduplication or exactly-once guarantee beyond preventing obvious useless notification loops.

## Background & Motivation

### Current state

Themion already supports two collaboration paths across agents:

- direct Stylos talk for realtime peer messages
- durable notes for asynchronous work intake and board tracking

Recent PRDs moved the system toward durable note-based collaboration by making note creation network-delivered and improving note injection clarity. That makes notes a better fit for multi-step or asynchronous work because they are durable, target a specific agent, and can be revisited later.

However, there are still two gaps in the collaboration story:

1. agents may still use `_talk` to ask another agent for work that should instead be recorded as a durable note
2. when agent B completes a note that agent A created, agent A does not yet have a documented automatic completion-notification path through the same durable note system

The user wants `_talk` to avoid annoying another agent when a note is sufficient, and wants completion of a cross-agent note to feed back to the requester as a note mentioning that the work is done and including the result.

### Why note-first collaboration is preferable

A direct talk request is interrupt-like. It targets another running agent immediately and is better suited to short, urgent, or interactive coordination. A durable note is better suited to delegated work because it:

- persists durably in SQLite
- survives busy/idle timing differences
- enters the existing board workflow
- avoids requiring the recipient to be interrupted at the moment of request
- carries explicit result state once finished

That makes note-first collaboration a better default for agent-to-agent work delegation.

**Alternative considered:** keep `_talk` and notes as equally preferred collaboration paths. Rejected: that leaves agents free to interrupt peers unnecessarily and weakens the existing board-oriented design.

### Why completion should return to the requester as a note

If agent A delegates work to agent B with a durable note, the natural completion signal should also be durable. A return note gives agent A a durable, deferred, inspectable notification instead of requiring synchronous coordination or transcript scraping.

That return note should say, in effect:

- which original note was completed
- who completed it
- that it is done
- what result or summary was produced

This keeps the requester informed even if it is busy when the worker finishes.

**Alternative considered:** rely only on the worker updating the original note to `done` and assume the requester will inspect the remote board later. Rejected: the requester may not have direct visibility into the worker's local board, and polling is less useful than a durable return signal.

### Why loop prevention is necessary

Automatic done-notification notes are useful, but without explicit rules they can create empty or repetitive back-and-forth notes. For example:

- agent A creates a note for agent B
- agent B marks it `done` and auto-creates a completion note for agent A
- agent A processes that completion note and marks it `done`
- that done action must not auto-create another completion note back to agent B unless there is genuinely new delegated work

The user explicitly wants to avoid endless loops on non-useful request/result traffic. The design therefore needs a clear distinction between a work-request note and an informational done-mention note.

**Alternative considered:** suppress all automatic completion notes and require agents to notify manually. Rejected: that loses the main usability benefit the user requested.

## Design

### Prefer durable notes over `_talk` for delegated work

Agent guidance and collaboration docs should explicitly tell agents to prefer durable note creation over `_talk` when asking another agent to perform asynchronous or non-urgent work.

Normative behavior:

- prompt guidance should treat `_talk` as a realtime, interrupting path
- prompt guidance should tell agents to avoid `_talk` when the request is better expressed as durable delegated work
- prompt guidance should prefer note creation for work that can wait for the recipient's board and idle-time processing
- `_talk` remains available for brief coordination, urgent clarification, or human-like realtime interaction where interruption is actually desirable

This is primarily a behavior and guidance correction, not a removal of functionality.

**Alternative considered:** hard-disable `_talk` for agent-to-agent communication. Rejected: realtime coordination still has valid uses.

### Introduce requester-aware done mentions for cross-agent notes

When a durable note reaches `done`, the system should detect whether it originated from another agent and, if so, create a return note for that requester.

Normative behavior:

- auto-notification applies only when the completed note has a known non-local requester identity such as `from` and `from_agent_id`
- the completion path should create a new durable note addressed to the requester's instance and agent
- the new note should include a concise mention of the original note identity and the final result text or summary
- the new note should be informational and should clearly indicate that it is reporting completion of prior delegated work
- the auto-created return note should be generated once per original note completion, not on every later update

This keeps completion feedback inside the durable collaboration channel.

**Alternative considered:** mutate the original note in-place across both participants instead of creating a new requester-side note. Rejected: the boards are local to each receiving instance, and a new requester-side note better matches the current architecture.

### Distinguish work-request notes from informational done mentions

The system needs metadata or note semantics that distinguish a delegated work request from an informational completion mention.

Normative behavior:

- a normal delegated note remains a work-request note that may lead to action by the recipient
- an auto-created completion notification is a done-mention note, not a fresh delegated request by default
- done-mention notes should still be durable and visible through the board, but their semantics must make it clear they are reporting completion rather than asking the recipient to re-do the same task
- prompt injection for done-mention notes should present them as completion notifications with referenced original note identity and result

This semantic distinction is the main guard against useless ping-pong behavior.

**Alternative considered:** reuse ordinary note wording and rely on the agent to infer that a result-only note should not trigger another result note. Rejected: explicit semantics are safer and easier to document and test.

### Prevent automatic done-mention loops

Automatic completion notifications must not recursively generate more automatic completion notifications without new useful work.

Normative behavior:

- auto-created done-mention notes must not themselves trigger another automatic done-mention when they are later marked `done`
- automatic requester notification should happen only for notes classified as delegated work requests, not for informational done mentions
- the implementation should record enough metadata on the generated note or original note state to ensure one-way completion signaling
- if an agent chooses to create a new real work-request note in response to a done mention, that is allowed because it represents new useful work rather than an automatic echo

This preserves useful completion feedback while stopping endless automatic loops.

**Alternative considered:** allow every note with sender metadata to generate another done note on completion. Rejected: that would produce the exact endless non-useful loops the user wants to avoid.

### Include useful result content in the return note

The requester-side completion note should be useful without forcing immediate remote inspection of the worker's board.

Normative behavior:

- include the original note's `note_id` and `note_slug` when available
- include the worker identity that completed the note
- include the final result text from the completed note when present
- if no explicit result text exists, include a concise completion summary indicating that the work finished without a stored result
- keep the note concise and clearly framed as a completion mention rather than copying excessive board state

This mirrors the metadata-first note design already used elsewhere in the notes system.

**Alternative considered:** create a requester-side done note with no result content and require manual lookup. Rejected: that weakens the usefulness of the notification.

### Keep human and local-only notes unchanged unless cross-agent metadata is present

This automation should target cross-agent collaboration, not every note in the system.

Normative behavior:

- notes without requester metadata do not auto-create return notes on completion
- purely local notes created for the same instance and agent do not gain new cross-agent completion behavior unless they explicitly represent delegated work from another agent
- user-originated flows that are not modeled as another agent requester remain unchanged unless future work extends the schema deliberately

This keeps the feature scoped to the user's collaboration request.

**Alternative considered:** create done mentions for every completed note regardless of origin. Rejected: most notes do not need a mirrored completion note and this would create noise.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Update prompt-visible collaboration guidance so agents prefer durable note creation over `_talk` for delegated asynchronous work. |
| `crates/themion-core/src/agent.rs` | If note injection formatting is centralized here, extend it to distinguish delegated work notes from informational done mentions in injected prompts. |
| `crates/themion-core/src/db.rs` | Extend durable note metadata as needed to classify note kind and to prevent automatic done-mention loops while preserving existing note identity and timestamps. |
| `crates/themion-cli/src/tui.rs` | Hook note completion transitions so cross-agent delegated notes can generate one requester-directed done mention with result content. |
| `crates/themion-cli/src/stylos.rs` | Route auto-created requester notifications through the existing Stylos-backed note creation flow when the target requester is another instance. |
| `docs/architecture.md` | Document note-first collaboration guidance and the one-way done-mention feedback path for cross-agent notes. |
| `docs/engine-runtime.md` | Document how injected prompts and note lifecycle distinguish delegated work notes from informational completion mentions. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- agent A creates a delegated note for agent B, and agent B marks it `done` with a stored result → verify: exactly one requester-directed done-mention note is created for agent A and includes the useful result content.
- agent B marks the original delegated note `done` multiple times or edits result text after completion → verify: the automatic requester notification is not duplicated unless a future explicit resend behavior is introduced.
- agent A later marks the done-mention note itself `done` → verify: no automatic note is created back to agent B.
- a completed note has sender instance metadata but no sender agent identity → verify: no ambiguous auto-routing occurs, or the system fails clearly according to the chosen routing rule.
- a note was created locally without another-agent requester metadata → verify: moving it to `done` does not create a requester notification.
- the original delegated note reaches `done` with empty result text → verify: the requester still receives a useful completion mention indicating completion with no explicit stored result.
- the requester instance is unreachable when the worker completes the note → verify: the worker-side completion path reports or records the notification failure clearly without creating local notification loops.
- an agent intentionally creates a new work request in response to a done mention → verify: this is treated as a new userful note flow, not as an automatic loop.

## Migration

A lightweight schema or metadata migration may be required if the implementation needs an explicit note kind or one-way notification marker.

Behaviorally:

- delegated cross-agent notes gain automatic requester notification on completion
- informational done mentions do not recursively generate more informational done mentions
- `_talk` remains available but is documented and prompted as a less preferred path for asynchronous delegated work

Existing notes without the new metadata should default safely to the current behavior unless their origin data is sufficient to classify them as delegated work under the new rules.

## Testing

- create a cross-agent delegated note and complete it with result text → verify: the requester receives one done-mention note with original-note reference and result content.
- complete a cross-agent delegated note without result text → verify: the requester still receives one useful completion note that clearly says the work is done.
- complete a locally created note with no remote requester metadata → verify: no automatic requester notification note is created.
- mark an auto-created done-mention note as `done` → verify: no second automatic done-mention is generated.
- use `_talk` for a non-urgent delegated-work scenario in prompt-level guidance tests or snapshots → verify: guidance prefers durable note creation over `_talk` for that case.
- complete a delegated note when the requester instance is unreachable → verify: notification failure is surfaced clearly and does not create duplicate retries by default.
- run `cargo check -p themion-core -p themion-cli` and `cargo check -p themion-cli --features stylos` after implementation → verify: default and Stylos-enabled builds both compile cleanly.

## Implementation checklist

- [x] update agent collaboration guidance to prefer durable notes over `_talk` for asynchronous delegated work
- [x] define note metadata that distinguishes delegated work requests from informational done mentions
- [x] hook note completion so cross-agent delegated notes can emit one requester-directed completion mention
- [x] include original-note identity and useful result content in the generated done mention
- [x] prevent auto-created done mentions from recursively creating more done mentions
- [x] keep local-only and requester-less notes on the current behavior path
- [x] route requester notifications through the existing Stylos note-create path where applicable
- [x] update architecture and runtime docs for note-first collaboration and done-mention semantics
- [x] update `docs/README.md` with this PRD entry
