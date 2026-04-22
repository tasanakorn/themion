# PRD-036: Prompt Guidance for Self-Note Creation Beyond Simple Q&A

- **Status:** Proposed
- **Version:** v0.22.0
- **Scope:** `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-22

## Summary

- This PRD changes one thing: prompt guidance for when the model should create a durable note for itself.
- Simple direct Q&A without tools usually should not create a self-note.
- If the task needs tools or durable follow-up tracking, the prompt should tell the model to consider a self-note.
- Self-notes should be treated as reminders to keep track of work, not as a required step for every task.
- Board storage and note semantics stay the same.

## Goals

- Make the prompt more explicit about when the model should consider creating a self-note.
- Make simple Q&A without tools the clearest default case where no self-note is usually needed.
- Encourage self-notes when work needs tools or durable follow-up tracking.
- Reduce missed follow-up work by making this guidance explicit.

## Non-goals

- No new note kinds or board columns.
- No board schema or storage changes.
- No automatic self-note generation in runtime code.
- No broader self-note workflow policy beyond the creation decision.
- No change to done-mention semantics or reply behavior.

## Background & Motivation

### Current state

Themion already has durable board notes and prompt guidance for board use, especially for multi-agent collaboration. But the current guidance does not clearly say when the model should create a note for itself.

The intended rule is simple: if the task is just simple Q&A and does not need tools, the model should usually answer directly. If the task is no longer that simple and needs tools or durable follow-up tracking, the model should consider creating a self-note so it can remember and keep track of the work.

That boundary is the whole point of this PRD.

**Alternative considered:** broaden the PRD into full self-note lifecycle rules such as active-note limits or result-update behavior. Rejected: this PRD should stay focused on the creation decision.

## Design

### Make simple Q&A without tools the default no-note case

Prompt-visible guidance should say clearly that when a task is just a simple direct answer and does not require tools, the model should usually answer directly without creating a self-note.

This gives the model a clear default where a self-note is unnecessary.

**Alternative considered:** leave the no-note case implied. Rejected: the simple Q&A boundary should be explicit.

### Tell the model to consider a self-note once work goes beyond simple Q&A

Prompt-visible guidance should say clearly that when the task is not just simple Q&A anymore, the model should consider creating a durable board note for itself.

Strong signals include:

- needing to call tools,
- needing to inspect files or run commands,
- needing to edit code,
- needing to validate results,
- or needing to remember follow-up work.

The purpose should be stated plainly: the self-note helps the model remember and keep track of ongoing work.

**Alternative considered:** describe the trigger only with vague terms such as "complex work." Rejected: the guidance should use a simpler and more practical boundary between direct Q&A and tracked tool-using work.

### Keep the guidance advisory

The prompt should tell the model to consider a self-note, not require one for every tool call.

Some short inspect-and-answer tasks may still be simple enough to finish without durable tracking. The important change is that once work is no longer a direct one-shot answer, self-note creation becomes an explicit consideration.

**Alternative considered:** require a self-note for every task that uses any tool. Rejected: that would create unnecessary note noise.

### Keep the rule near existing board instructions

The new wording should live near the existing durable board or collaboration guidance so the model sees self-note creation guidance in the same part of the prompt as note usage guidance.

The wording should stay short and operational.

**Alternative considered:** spread the rule across multiple instruction sections. Rejected: one concise rule is easier to maintain and follow.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Update prompt-visible note guidance so simple Q&A without tools is the default no-note case and non-Q&A tool-using work triggers self-note consideration. |
| `docs/engine-runtime.md` | Document the prompt-level rule for self-note creation if this guidance is described there. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- a user asks a factual question and no tools are needed → verify: guidance favors answering directly without creating a self-note.
- a task needs file reads or shell commands before the answer can be given → verify: guidance tells the model to consider creating a self-note.
- a task uses one tiny tool call and completes immediately → verify: guidance still allows skipping a self-note when durable tracking adds no value.
- a task requires edits plus validation → verify: guidance tells the model to consider a self-note to keep track of the work.

## Migration

No schema migration is required.

This is a prompt-and-docs change only:

- simple direct Q&A without tools remains a no-note default case
- non-trivial tool-using work should more often trigger self-note consideration
- existing note storage and note behavior remain unchanged

## Testing

- review the updated prompt/instruction text for a simple direct question with no tool use → verify: it says the model should usually answer directly without a self-note.
- review the updated prompt/instruction text for a task that requires reading files, running commands, or editing code → verify: it tells the model to consider creating a self-note.
- review the updated prompt/instruction text for a very small one-shot tool use case → verify: it does not force a self-note mechanically.
- run `cargo check -p themion-core` after implementation if the prompt source changed there → verify: the updated instruction source compiles cleanly.

## Implementation checklist

- [ ] add concise prompt guidance for self-note creation near existing board instructions
- [ ] explicitly state that simple Q&A without tools usually does not need a self-note
- [ ] explicitly state that work beyond simple Q&A should trigger self-note consideration
- [ ] mention tool use and follow-up tracking as strong signals for self-note creation
- [ ] keep the wording advisory rather than mandatory for every tool call
- [ ] update docs if they describe this prompt behavior
- [ ] update `docs/README.md` with the new PRD entry
