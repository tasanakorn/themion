# PRD-029: Stylos Notes Board Phase 1 — Replace Ephemeral Talk with Durable Note Intake and Board Columns

- **Status:** Proposed
- **Version:** v0.16.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21

## Summary

- Replace the current realtime-oriented `talk` concept with a durable `note` and `board` concept that still supports agent-to-agent messaging but no longer depends on the receiver being idle at send time.
- Store notes in the local SQLite database with stable identifiers and explicit association to both the target instance `<hostname>:<pid>` and target `agent_id`.
- When a note arrives for a busy agent, keep it in the database and inject it into the agent later when that agent becomes idle, using a prompt path similar to the current sender-aware `talk` bridge.
- Expose a model-usable board view with three primary columns: `todo`, `in_progress`, and `done`.
- When an agent becomes idle, prefer resuming `in_progress` work; inject a new `todo` note only when that agent has no `in_progress` note.
- Let agents and tooling move notes between columns and attach completion or work results to the note itself instead of relying on transient chat replies alone.
- Keep this PRD focused on the first durable board slice; richer multi-agent collaboration patterns and broader workflow design should be split into later PRDs.

## Goals

- Replace the fragile idle-only Stylos `talk` delivery model with a durable note intake model that accepts submissions even while the target agent is busy.
- Persist note records in SQLite so pending work survives temporary busy periods and can be inspected by both runtime logic and model tools.
- Define a board abstraction with exactly three primary columns in the first slice: `todo`, `in_progress`, and `done`.
- Preserve explicit targeting to both instance identifier `<hostname>:<pid>` and `agent_id` so one process can host multiple independent note queues.
- Inject newly deliverable notes into an agent turn when that specific agent becomes idle, reusing the existing local prompt-injection architecture instead of inventing a separate execution engine.
- Prefer resuming an `in_progress` note over starting a fresh `todo` note when an agent becomes idle.
- Let agents inspect notes on the board and update column state through explicit tools rather than depending only on pushed inbound prompts.
- Allow result text or structured completion output to be attached to a note so a note can act as a durable work item rather than just a one-shot message.
- Keep the first PRD narrowly scoped enough to implement in phases rather than trying to solve all future multi-agent collaboration behavior at once.

## Non-goals

- No full design yet for advanced multi-agent collaboration workflows, delegation trees, or conversation/thread orchestration across many notes.
- No requirement in this PRD to design a general kanban UI inside the TUI beyond the runtime, data, and tooling behavior needed for note storage and delivery.
- No requirement to preserve backward compatibility for `talk` as a first-class long-term concept if the implementation chooses to deprecate or replace it.
- No durable distributed consensus, cross-node transactional guarantees, or exactly-once delivery semantics.
- No attempt in this PRD to solve broad assignment, prioritization, due dates, watchers, mentions, or arbitrary labels.
- No requirement to design the eventual board semantics for human users and external automation in full detail beyond the first note and column operations.
- No requirement to finalize future PRDs for note-thread replies, board-wide search UX, or complex collaboration rules in this document.
- No requirement in this PRD to support multiple simultaneously active `in_progress` notes for one agent unless a later PRD expands the model deliberately.

## Background & Motivation

### Current state

The current Stylos direct-message path is documented as `talk`: a sender-aware, acknowledgement-oriented peer message request. The CLI query layer decides delivery from the current exported snapshot and only accepts the request when the target agent is currently `idle` or `nap`, unless the caller chooses bounded wait-for-idle behavior. Accepted messages are bridged into the local prompt path as peer-message input.

That design improved sender identity and reply guidance, but it is still fundamentally a realtime interaction model. It works best when the receiver is already free or only briefly busy.

### Why the current `talk` concept is no longer sufficient

The user identified the core problem clearly: the current `talk` model is hard to utilize effectively and often becomes low-value because it depends on a narrow delivery window and conversational timing.

Even with bounded busy waiting, the concept still has several limitations:

- useful requests can miss the receiver if the receiver is busy for longer than a short timeout
- the sender is still thinking in terms of immediate conversation rather than durable work tracking
- the receiver has no durable board of pending items to inspect, sort, or complete
- completion state and result data are not first-class durable properties of the work item itself
- the transport semantics encourage conversational back-and-forth rather than a clearer work-item lifecycle

The requested reconceptualization is therefore not just a small transport tweak. It is a shift from realtime talk to durable notes on a board.

**Alternative considered:** keep `talk` as the main concept and only add a longer busy timeout. Rejected: that preserves the realtime mental model and does not create a durable work item or board lifecycle.

### Why a note/board concept fits better

A note/board concept reframes the interaction from “catch this agent when idle” to “post a durable item associated with the target agent, then let the runtime surface it when appropriate.”

This better matches the practical collaboration model the user described:

- notes can be posted while the target agent is busy
- notes persist in the database
- notes can be inspected later through tools
- notes move through visible columns such as `todo`, `in_progress`, and `done`
- results can be attached to the note as the work progresses or completes

This also aligns well with Themion's existing architecture because the system already has:

- SQLite persistence
- per-agent identity inside one process
- a CLI-local remote-request bridge
- prompt injection into the existing harness loop
- tools that can query and mutate persisted state

**Alternative considered:** treat the feature as a general task system extension instead of a note system. Rejected: tasks already have a different acknowledgement/result shape, while the user explicitly wants a note/board metaphor with columns and attached results.

### Why idle agents should resume `in_progress` work before starting `todo`

A board with `todo`, `in_progress`, and `done` needs a clear rule for what happens when an agent becomes idle.

Without that rule, the runtime could start new `todo` work even though the same agent already has unfinished `in_progress` work. That would make the board less meaningful and would blur the distinction between backlog and active work.

The intended behavior in phase 1 is therefore:

- `in_progress` means work the agent has already started and should resume first
- `todo` means work waiting to be started only after no `in_progress` note remains for that agent

This keeps the board semantics closer to a real kanban flow and avoids accidental accumulation of half-finished work.

**Alternative considered:** choose from both `todo` and `in_progress` using one global oldest-first ordering. Rejected: that would let new work start before previously active work is resumed and would weaken the meaning of the `in_progress` column.

### Why this PRD should be split into phases

A complete board-based collaboration system could easily expand into multiple large concerns:

- note creation and persistence
- deferred delivery and idle-trigger injection
- board query tools
- column transitions
- result attachment
- note reply or comment threading
- human-facing UI
- priority and filtering
- multi-agent collaboration policies
- cross-process consistency and retention policies

Trying to define all of that in one PRD would make implementation and review too broad. The right first step is to define a phase-1 PRD focused on the durable note entity, the three board columns, deferred delivery to busy agents, and the minimum tooling needed to inspect and update notes.

**Alternative considered:** write one umbrella PRD covering the entire future board system. Rejected: too much scope, too many unresolved future collaboration questions, and too much risk of mixing immediate needs with speculative design.

## Design

### Replace `talk` with a durable `note` submission path

The first slice should introduce a Stylos note submission path that represents a durable work item rather than an ephemeral realtime message.

Conceptually, a note contains:

- a stable `note_id`
- source identity such as `from_instance` and `from_agent_id` when known
- target identity: `to_instance` and `to_agent_id`
- note body/content
- board column
- delivery/injection state
- optional attached result payload
- creation and update timestamps in milliseconds

Normative behavior:

- a note submission should be accepted even when the target agent is currently busy, as long as the target instance and agent are valid
- acceptance means the note is durably stored in the database, not that it was immediately injected into the target agent turn
- note targeting must preserve both exact instance identifier `<hostname>:<pid>` and `agent_id`
- the first stored column for a newly created note should be `todo`
- the stored note must be retrievable later through board/note tools and internal runtime queries

This changes the success condition from “delivered now” to “persisted successfully for later consumption.”

**Alternative considered:** keep `talk` as transport and add a hidden database-backed fallback queue only for busy cases. Rejected: that would preserve two overlapping mental models instead of making note persistence the primary concept.

### Use SQLite as the durable source of truth for notes and board state

Notes should live in the existing SQLite database used by Themion rather than in a new sidecar store.

Normative behavior:

- note rows and any related result/update rows should be persisted in the existing system database
- the database schema should explicitly represent note identity, source and target association, column, prompt-injection state, and result attachment state
- timestamps intended for machine consumption should remain explicitly millisecond-based
- the runtime should be able to query pending notes per target agent without reconstructing them from transcript text

This keeps the board concept aligned with existing persistence patterns and avoids introducing another storage system.

**Alternative considered:** store notes only in memory and reconstruct some state from transcripts. Rejected: the requested concept is durable and board-like, so in-memory-only state would not satisfy the design intent.

### Model the board with three primary columns only in phase 1

Phase 1 should define exactly three primary board columns:

- `todo`
- `in_progress`
- `done`

Normative behavior:

- every note belongs to one of these three columns at all times
- newly created notes start in `todo`
- notes may be moved explicitly between columns by runtime logic or model/tool actions
- `done` indicates that the work item is complete enough that further active work is not expected in the current slice
- the first implementation should avoid adding more column types such as blocked, archived, canceled, or backlog

This keeps the initial board legible and prevents the first PRD from turning into a general workflow taxonomy.

**Alternative considered:** add more kanban-like states such as `blocked`, `review`, or `archived` immediately. Rejected: the user explicitly described three main columns, and that simpler state model is better for the first slice.

### Defer note injection until the target agent becomes idle

When a note is created for an agent that is currently busy, the system should store it and wait until that agent becomes idle before injecting it as input.

Normative behavior:

- if the target agent is busy when the note arrives, the note remains stored and pending
- when the target agent later becomes idle, Themion should inject the note into that agent through the same general local prompt path used for user input and prior Stylos remote prompt injection
- when selecting a note to inject for an idle agent, Themion should first consider notes in `in_progress`
- Themion should inject a `todo` note only when that target agent has no note currently in `in_progress`
- if one or more `in_progress` notes exist for that target agent, Themion should inject from `in_progress` rather than starting new `todo` work
- injection should be note-aware rather than pretending the note was a normal transient chat message
- injection should mark note delivery state so the runtime does not repeatedly inject the same note unintentionally
- if multiple eligible notes exist within the chosen column for one target agent, the implementation should use a deterministic ordering such as oldest-created first

The durable note is therefore separate from the injection event. Storage happens first; prompt delivery happens later when the local execution path is ready.

**Alternative considered:** inject the note immediately even while the agent is busy by interrupting current work. Rejected: the user explicitly wants busy-safe posting, not forced interruption of the receiver.

### Provide model-visible board tools in addition to automatic injection

Automatic injection alone is not enough. The model should also be able to inspect the board and manage notes through explicit tools.

Phase-1 board tooling should cover at least:

- create a note
- list notes for an agent or board column
- read one note in detail
- move a note between `todo`, `in_progress`, and `done`
- attach or update a result on a note

Normative behavior:

- the model should be able to check notes on the board without waiting for a pushed injection event
- tools should operate on durable note IDs rather than inferred transcript text
- move operations should update both the column and the note's last-updated timestamp
- result attachment should be explicit rather than encoded only as freeform transcript content

This supports both push-style delivery and pull-style board inspection.

**Alternative considered:** rely only on automatic note injection and omit board tools initially. Rejected: the user explicitly asked for the model to be able to check notes on the board.

### Make column movement and attached results first-class note operations

A durable board is only useful if the work item can evolve over time.

Normative behavior:

- a note may move from `todo` to `in_progress` when work begins
- a note may move to `done` when the requested work is completed or no further active work is needed
- the system should support attaching a result or completion payload directly to the note record or a note-associated result table
- attached result data should remain readable later through note/board tools
- a done note without attached result should still be allowed when the completion state itself is sufficient, but the design should encourage result attachment when the note requested work output

This makes note state and note outcome durable and inspectable.

**Alternative considered:** infer completion only from the column and keep results only in conversation history. Rejected: the user explicitly wants the ability to attach results to the note.

### Keep note delivery scoped to target instance and target agent

Because one Themion process can host multiple agents, durable notes must remain associated with both the destination instance and destination agent.

Normative behavior:

- target identity must be stored as exact target instance `<hostname>:<pid>` plus target `agent_id`
- pending-note lookup should happen per target agent, not only per process
- note injection should only occur for the matching local agent
- sender identity, when known, should also remain explicit and separate from the target identity

This preserves the multi-agent architecture boundary already present in Stylos status and request routing.

**Alternative considered:** attach notes only to the process and let local runtime choose any agent later. Rejected: that would lose the agent-level association the user explicitly requested.

### Treat phase 1 as a deliberate replacement/supersession path for `talk`

This PRD should treat durable notes as the intended future concept replacing the older talk-centric model for asynchronous collaboration.

Normative behavior for the first PRD:

- new documentation and implementation work should present notes/board as the preferred collaboration model
- exact migration mechanics from `talk` to `note` may be staged, but the architectural direction should be explicit
- if compatibility shims are needed temporarily, they should be documented as transitional rather than permanent primary concepts

This makes the repo direction clear while still allowing implementation to phase migration carefully.

**Alternative considered:** keep `talk` and `note` as equal long-term concepts. Rejected: the user explicitly wants to migrate from talk to note because the current concept is not useful enough.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Add or replace Stylos collaboration tool schemas so the model can create notes, inspect board state, move note columns, and attach note results through durable note IDs. |
| `crates/themion-core/src/db.rs` | Add persistent SQLite schema and accessors for notes, note board columns, delivery/injection state, result attachment, and target agent lookup. |
| `crates/themion-core/src/agent.rs` | Support note-aware prompt injection wording for deferred inbound notes delivered when the target agent becomes idle. |
| `crates/themion-cli/src/stylos.rs` | Replace or supersede realtime `talk` request handling with durable note submission and pending-note lookup targeted by instance and agent ID. |
| `crates/themion-cli/src/tui.rs` | Integrate idle-time note injection for the matching local agent, prefer `in_progress` over `todo`, and coordinate deterministic delivery of pending notes through the existing local turn path. |
| `docs/architecture.md` | Document the note/board concept, the three board columns, durable storage, target identity association, and idle-trigger injection behavior. |
| `docs/engine-runtime.md` | Document how note submission stays CLI-local, how pending notes are stored durably, and how idle-trigger note injection enters the existing harness prompt path. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- a note is posted to a valid target agent while that agent is busy for an extended period → the note remains stored in `todo` until that agent becomes idle and delivery is attempted.
- an agent becomes idle and has both `in_progress` and `todo` notes → the runtime should select from `in_progress` and should not start a new `todo` note yet.
- multiple notes are posted to the same target agent while it is busy → the runtime should inject them in deterministic order, such as oldest first, within the chosen eligible column.
- a note is posted to an unknown target agent on a known instance → the request should fail clearly and should not create an orphan note row.
- a note is posted to a target instance that is no longer reachable → local storage semantics depend on whether submission occurs on the receiver or sender path, but the first implementation must document clearly where durability begins and avoid pretending remote storage succeeded when it did not.
- an agent becomes idle, a pending note is injected, and the agent immediately becomes busy again due to that turn → the note should not be reinjected as a duplicate unless the first injection is explicitly marked failed and retriable.
- a note moves to `done` without any attached result → this should remain allowed, but board readers should still see the completion state clearly.
- a result is attached before a note moves to `done` → the note should continue showing the attached result while still being in `todo` or `in_progress` if that reflects the actual workflow.
- a model moves a note backward from `done` to `in_progress` or `todo` → the first implementation may allow it if the column model stays intentionally simple, but the behavior should be explicit and documented rather than accidental.
- the same sender posts repeated similar notes to one target agent → each note should receive its own durable identifier; deduplication is out of scope unless explicitly added later.

## Migration

This PRD introduces a conceptual migration from realtime `talk` to durable `note`/`board` behavior.

Expected migration shape for the implementation phase:

- the repository should document notes as the preferred asynchronous collaboration primitive
- existing `talk` paths may need a transition period, but they should be treated as legacy or compatibility behavior when note-based flows land
- the first implementation will likely require a database migration to create note-related tables and indexes in the existing system database
- prompt injection semantics should move from transient peer-message emphasis toward durable note/work-item emphasis
- any compatibility wrapper that maps old `talk` calls into note creation should be documented clearly if used
- future PRDs can define deeper migration of reply semantics, note threading, and richer collaboration workflows once the phase-1 note substrate exists

## Testing

- create a note targeting a currently busy local agent → verify: the request succeeds by persisting a durable note record instead of failing for `agent_busy`.
- create a note targeting a valid local agent and inspect the database-backed board → verify: the new note appears in column `todo` with the expected target instance and target agent association.
- keep a target agent busy, create multiple notes, then let the agent become idle while one note is already `in_progress` → verify: the runtime injects from `in_progress` first instead of starting a fresh `todo` note.
- let a target agent become idle with no `in_progress` notes and at least one `todo` note → verify: the runtime injects a `todo` note.
- create a note and use the board tool to list notes for one agent → verify: the model-visible tool returns the stored note metadata and current column without requiring transcript scraping.
- move a stored note from `todo` to `in_progress` → verify: the column changes durably and the updated timestamp changes in milliseconds.
- attach a result to a note, then read that note again → verify: the attached result is returned as part of note detail or note result data.
- move a note to `done` after attaching a result → verify: the note remains readable with both final column and attached result preserved.
- attempt to create a note for an unknown target `agent_id` on a known instance → verify: the request is rejected clearly and no durable note row is created.
- restart the local process after creating pending notes but before delivery → verify: pending notes remain present in SQLite and are still eligible for later delivery according to the documented runtime behavior.
- run `cargo check -p themion-core -p themion-cli --features stylos` after implementation → verify: note persistence, board tooling, and idle-trigger injection compile cleanly.

## Implementation checklist

- [ ] define the phase-1 note and board scope explicitly in code and docs as the preferred replacement path for asynchronous `talk`
- [ ] add SQLite schema for durable notes, column state, timestamps, source/target association, and result attachment
- [ ] add note persistence helpers in `themion-core` database access code
- [ ] add model-visible tools for note creation, board listing, note detail, column movement, and result attachment
- [ ] define exact note identifiers and durable note lookup semantics
- [ ] accept note creation while the target agent is busy as long as the target identity is valid
- [ ] preserve exact target instance `<hostname>:<pid>` and target `agent_id` on stored notes
- [ ] add idle-trigger deferred note injection for the matching target agent through the existing local prompt path
- [ ] define idle-selection policy so `in_progress` is preferred over `todo`
- [ ] ensure injected notes are marked so they are not duplicated unintentionally
- [ ] define deterministic ordering for multiple pending notes targeting the same agent within the chosen eligible column
- [ ] document the three primary columns as `todo`, `in_progress`, and `done`
- [ ] support explicit note column movement between those columns
- [ ] support attaching result data to a note and reading it back later
- [ ] update architecture and engine runtime docs to describe the new note/board concept and the migration away from realtime `talk`
- [ ] update `docs/README.md` with the new PRD entry
- [ ] split future follow-up work such as advanced collaboration semantics, note threading, and richer board UX into later PRDs rather than expanding this one
