# PRD: Improve Instruction Guidance for Self-Created Notes

## Original instruction

User request that led to this PRD:

> no. we are working on themion. should only update to themion

## Summary

Improve the instruction set in the current workspace so the agent more reliably creates durable board notes for itself when a task is non-trivial, while avoiding noisy or misleading note behavior.

This change is about instruction quality and workflow guidance, not introducing a new board primitive.

## Problem

The current guidance covers planning, workflow persistence, and multi-agent delegation, but it does not explicitly teach the model when it should create a durable note for itself.

As a result, the model may:

- start multi-step work without leaving itself a durable trace,
- lose follow-up items during long or branching tasks,
- underuse self-addressed notes as a persistence mechanism,
- overuse notes in simple cases where a direct response or immediate tool call is enough,
- incorrectly treat completion of self-created notes like external note replies.

## Goals

- Teach the model to consider creating a note for itself when the task is more than a simple direct response.
- Encourage use of self-created notes for multi-step follow-up and task persistence.
- Allow the model to create multiple self-notes when that materially helps execution.
- Prevent noisy, low-value note creation.
- Prevent incorrect "reply" behavior for notes the model created for itself.

## Non-Goals

- No new note types.
- No required changes to board storage semantics.
- No automatic note generation in runtime code unless separately scoped.
- No requirement that every tool-using task must create a self-note.

## User / Agent Story

As an agent working on a non-trivial task, I want to create one or more durable notes for myself when helpful, so I can preserve intent, track follow-up work, and resume accurately without spamming the board or misreporting note results.

## Desired Behavior

### 1. Decide when self-notes are worth creating

The instruction should guide the model to consider creating a note for itself when the task is not a simple one-shot response.

Good candidates include tasks that:

- require multiple tool calls,
- involve multiple logical subtasks,
- need follow-up after an intermediate result,
- may branch into separate workstreams,
- are likely to outlive a single short reasoning burst,
- benefit from durable task tracking beyond ephemeral planning.

The instruction should also make clear that the model should not create a self-note for:

- simple conversational replies,
- one-shot answers that do not require tools,
- tiny read/inspect actions with no follow-up,
- obvious low-friction work where a note adds no tracking value.

### 2. Use `in_progress` carefully

The instruction should guide the model to move a self-created note to `in_progress` when actively working it, but only if there is not already another relevant self-note in `in_progress`.

Desired interpretation:

- Prefer a single active self-owned `in_progress` note representing the current main thread of work.
- Additional self-created notes may remain in `todo` until they become the active focus.
- If the board already has an appropriate self-note in `in_progress`, avoid creating or promoting another overlapping main-task note without a clear reason.

This should reduce fragmentation and make board state easier to understand.

### 3. Allow multiple self-notes when they add real value

The instruction should explicitly permit creating multiple self-addressed notes when they help the model manage a complex task.

Examples:

- one note for the main implementation thread,
- one note for validation or testing follow-up,
- one note for a deferred documentation update,
- one note for a discovered blocker that must be revisited.

But the guidance should emphasize that multiple notes are useful only when they represent distinct, actionable follow-up items.

### 4. Avoid noisy notes

The instruction should warn against creating notes that are not genuinely useful.

Noisy notes include:

- restating the user request without adding execution value,
- creating many tiny notes for trivial actions,
- creating notes for work that will be completed immediately anyway,
- duplicating an existing self-note with the same purpose,
- using notes as a substitute for normal reasoning when no durable tracking need exists.

A good self-note should be:

- actionable,
- specific enough to resume,
- scoped to a meaningful unit of work,
- worth persisting on the board.

### 5. Do not "reply" to self-created notes as if they were external requests

The instruction should clarify that when the model creates a note for itself, it should not later treat that note like an external inbound request that needs a reply-style result update.

Desired behavior:

- self-created notes are internal tracking artifacts,
- completing them should update board state appropriately if needed,
- the agent should not produce unnecessary note-result chatter for its own notes,
- done-mention semantics remain for informing other agents, not for self-conversation.

This prevents redundant or awkward note traffic.

## Proposed Instruction Addition

Add a short guidance block near the existing board-note and collaboration instructions in the current workspace.

Suggested draft:

> When a task is more than a simple direct response, consider creating a durable board note for yourself so important follow-up work is not lost.
> Use self-notes for meaningful multi-step or branching work, not for trivial one-shot actions.
> If self-notes help, you may create multiple notes for distinct actionable subtasks, but avoid noisy or duplicative notes.
> Prefer at most one main self-owned note in `in_progress` at a time; leave other self-notes in `todo` until you actively work them.
> Do not treat notes you created for yourself as external requests that need reply-style result updates.

## Example Scenarios

### Good

- User asks for a feature change that requires code edits, tests, and docs. The model creates a main self-note and a separate deferred validation note.
- User asks for a debugging investigation that may branch. The model creates one self-note for investigation and later a second note for a concrete fix discovered during debugging.
- User asks for a medium-sized refactor. The model creates one self-note, moves it to `in_progress`, and keeps related follow-up notes in `todo`.

### Bad

- User asks a factual question and the model creates a note instead of answering.
- User asks for a small single-file read and summary, and the model creates several notes.
- The model creates a new self-note even though an equivalent self-note is already `in_progress`.
- The model posts reply-like result text to its own note as though another agent requested it.

## Acceptance Criteria

- Instruction text explicitly tells the model to consider self-note creation for non-simple tasks.
- Instruction text explicitly discourages self-notes for trivial direct-response tasks.
- Instruction text allows multiple self-notes for distinct actionable subtasks.
- Instruction text advises against multiple overlapping `in_progress` self-notes.
- Instruction text warns against noisy or low-value notes.
- Instruction text clarifies that self-created notes should not be handled like external reply targets.

## Open Questions

- Should the guidance say "consider" or "should usually" for non-trivial tool-using tasks?
- Should the model be instructed to check existing self-owned `in_progress` notes before creating a new main note?
- Should completion of self-notes prefer silent state transitions only, unless another agent needs notification?

## Likely Implementation Area

Primary candidates:

- instruction text where planning, persistence, and collaboration guidance already lives,
- workspace-specific board-note guidance near durable note usage instructions,
- any mirrored prompt or instruction source maintained in the current workspace.
