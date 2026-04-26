# PRD-058: Add Optional Tool Reason Guidance, Recording, and Chat Visibility

- **Status:** Proposed
- **Version:** v0.36.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Themion tool usage is often understandable in the moment but becomes ambiguous later when only the raw tool name and arguments are visible, especially if there is no explicit intent or reason text.
- File and shell tools are the clearest examples: a `path` or `command` often shows what was touched, but not why.
- Themion should strongly encourage the model to generate a brief, concrete tool-use `reason` for meaningful tool calls without making that field mandatory.
- That `reason` text should be recorded in durable history when available and shown in the chat panel so the team can evaluate whether the model produces useful explanations in practice.
- This PRD focuses on adding and validating optional tool `reason` text. Broader prompt-history compaction and storage-shaping work should move to PRD-070.

## Goals

- Add a concrete optional tool `reason` path that the model can use for meaningful tool calls.
- Strongly recommend that the model emits a short, concrete `reason` field for tool use, especially for file and shell tools.
- Keep the field name concrete, narrow, and clearly recognizable as explaining why the tool is being used.
- Record tool `reason` text in persisted history when available so later analysis can measure usefulness and prompt efficiency.
- Show tool `reason` text in the chat panel so users can see why the assistant is invoking tools.
- Validate whether model-generated tool `reason` text is consistently useful enough to justify broader adoption.

## Non-goals

- No requirement that every tool call include a `reason` field.
- No broad prompt-history compaction redesign in this PRD.
- No archival/raw-history storage redesign in this PRD.
- No requirement to summarize or compact all oversized tool payloads here.
- No requirement to change every tool schema in one pass if a narrower recording path is sufficient.

## Background & Motivation

### Current state

Themion already records tool names, arguments, and tool results, and the TUI can show tool activity in the chat stream. But those records often answer only *what tool ran*, not *why it ran*.

For example:

- `fs_read_file(path="docs/README.md")`
- `fs_write_file(path="crates/themion-core/src/tools.rs")`
- `shell_run_command(command="cargo check -p themion-core")`

These are more understandable when surrounding assistant text is preserved, but that context is not always compact, nearby, or easy to scan later. A short explicit `reason` or `intent` field would make both live chat and stored history more interpretable.

### Why this matters

A useful tool `reason` field can help in several places:

- live chat readability: users can quickly see why the model is reading, writing, or running a command
- history readability: later review becomes more semantic and less like raw protocol traffic
- prompt-quality experiments: Themion can assess whether short intent text helps preserve task continuity without replaying large adjacent narration
- future compaction work: if tool `reason` text proves useful, it can become a stable ingredient in later prompt-history compaction and replay design

The immediate product question is not whether this field should be mandatory everywhere. The immediate question is whether the model can generate concise, useful `reason` text reliably enough to justify deeper integration.

## Design

### Design principles

- Prefer a small, testable slice over a broad history-system redesign.
- Strongly encourage short, concrete tool `reason` text where it helps, but do not require it.
- Preserve existing tool behavior when no such field is provided.
- Make tool `reason` text visible enough that humans can judge its quality.
- Record that text durably enough that later evaluation can measure whether it improves continuity and understanding.

### 1. Add an optional tool `reason` field

Themion should support an optional short tool `reason` field on meaningful tool calls.

The field name should be `reason`.

The initial focus should be on tools where intent is commonly unclear from arguments alone:

- `fs_read_file`
- `fs_write_file`
- `fs_list_directory`
- `shell_run_command`

The field value should be short and concrete, for example:

- `reason: verify touched crate still builds`
- `reason: patch PRD wording`
- `reason: check current PRD wording before editing`
- `reason: search for shell call sites`

This PRD intentionally keeps the field narrow: it should be a short, concrete `reason` that clearly communicates why the tool is being used. The implementation should record that text durably and expose it in the chat panel.

### 2. Strongly recommend model-generated `reason`, but do not require it

Prompt guidance should be updated so the model is strongly encouraged to provide a short, concrete `reason` field for meaningful tool use, especially for:

- shell commands
- file writes
- non-obvious file reads
- multi-step inspection or edit sequences

But the field should remain optional.

If the model omits the field:

- the tool call should still work normally
- no validation error should be raised
- existing tool flows should remain compatible

This keeps the experiment low-risk while allowing Themion to observe real model behavior rather than forced compliance.

### 3. Record tool `reason` in durable history

When a tool `reason` is available, Themion should persist it in durable history close to the corresponding assistant tool-call record so it can be recalled later.

The recorded data should make it possible to inspect at least:

- tool name
- key identifying argument summary such as `path` or `command`
- short `reason` text when present
- whether the field was omitted

This PRD intentionally leaves room for implementation choice, such as:

- extending stored tool-call metadata
- recording a dedicated compact field alongside tool-call JSON
- deriving a display-ready reason record from nearby assistant narration when that approach is simpler initially

**Alternative considered:** require a new mandatory structured `reason` parameter on every tool schema. Rejected for this slice because it increases protocol and translation surface area before Themion has validated that the model reliably produces useful `reason` text.

**Alternative considered:** use a vague field name such as `description`. Rejected because `description` does not clearly signal that the field exists to explain why the tool is being used. `reason` is narrower, clearer, and easier to recognize quickly in transcripts, history, and UI.

### 4. Show tool `reason` in the chat panel

When a tool `reason` is available, the chat panel should display it clearly near the tool activity so users can understand intent at a glance.

Examples of acceptable presentation include:

- inline with the tool label
- on a secondary line below the tool label
- in a compact `reason: ...` suffix if it remains readable in narrow layouts

The UI should avoid overwhelming the chat stream. If space is constrained, the text may be trimmed, but the presentation should still make it clear that a `reason` exists.

### 5. Validate usefulness before broader expansion

This PRD should explicitly treat tool `reason` text as a product experiment.

Validation should answer questions such as:

- does the model actually produce the field consistently when strongly encouraged?
- is the field concise and concrete rather than repetitive filler?
- do users find the chat panel more understandable?
- does stored history become more interpretable for later recall and prompt-use experiments?

If the experiment is successful, a later PRD can expand the design into broader prompt-history compaction, replay shaping, or schema-level support.

## Changes by Component

### `themion-core`

- add an optional tool `reason` recording path for assistant tool calls
- persist tool `reason` text in durable history when available
- keep existing tool execution behavior unchanged when no such field is present
- expose enough recorded metadata for later recall and evaluation
- update prompt/instruction assembly so the model is strongly encouraged, but not required, to provide useful short `reason` text

### `themion-cli`

- show tool `reason` text in the chat panel when available
- keep the display compact and readable in narrow terminal layouts
- continue to render normal tool activity cleanly when no such field is present

### Docs

- document the optional tool `reason` behavior and when the model is expected to provide that field
- document that the feature is intentionally advisory/experimental rather than mandatory
- reserve broader prompt-history compaction work for a follow-on PRD

## Edge Cases

- Some tool calls are obvious enough that a `reason` adds no value. Themion should not force redundant text.
- Some model-generated `reason` text may be vague or repetitive. The initial rollout should tolerate imperfect quality so usefulness can be measured from real usage.
- If a tool label is already long, the chat panel should avoid making the display noisy; trimmed display is acceptable.
- If persisted history contains older tool calls with no `reason`, recall and rendering should continue to work naturally.
- If future work introduces schema-level tool reasons, this PRD's recorded/history behavior should remain backward-compatible where practical.

## Migration

- No database rewrite is required for existing history.
- Existing tool-call records without `reason` should remain valid.
- New recordings should include tool `reason` text only when available.

## Testing

- invoke `shell_run_command` with a short `reason` such as `verify touched crate still builds` → verify: the tool executes normally, the field is recorded durably, and the chat panel shows it clearly
- invoke `fs_write_file` with a short `reason` such as `patch PRD wording` → verify: the write succeeds and the recorded history keeps that field near the tool call
- invoke `shell_run_command` for search with a short `reason` such as `search for shell call sites` → verify: the field name is clear and recognizable in storage and UI
- invoke an obvious trivial `fs_read_file` without a `reason` field → verify: the tool still works and the UI/history remain clean without placeholder text
- recall session history containing tool calls with and without `reason` fields → verify: recorded text is available for later inspection without breaking older records
- use narrow terminal layout with a long tool `reason` → verify: the chat panel remains readable and trims gracefully if needed

## Follow-on work

- move broader prompt-history compaction and oversized-payload shaping into PRD-070
- if this experiment succeeds, consider whether tool `reason` should become a first-class structured field for more tools
- if this experiment fails, keep the feature optional and avoid broad schema expansion
