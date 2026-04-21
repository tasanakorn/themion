# PRD-033: Note Injection Should Present Note Identity and Metadata in the Initial Prompt

- **Status:** Proposed
- **Version:** v0.19.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-22

## Summary

- When an idle agent receives an injected durable note, the injected prompt should explicitly say that it is a note rather than making the model infer that from context.
- Include the key note metadata directly in the injected prompt so the model can act on the note without first calling `board_read_note` just to discover basic context.
- Keep the durable board and `board_*` tools unchanged; tools remain available for follow-up inspection and updates.
- Make the injected prompt include exact source and target identities plus note identifiers and current board column.
- Keep the prompt concise and structured so the model knows it is handling a durable work item, not a transient talk message.
- Avoid pretending the note metadata is ordinary user prose; the injection should present it as explicit note wrapper data.

## Goals

- Make injected note prompts self-identifying so the receiving model immediately knows it is processing a durable note.
- Include enough note metadata in the injected prompt that the model usually does not need an immediate `board_read_note` call just to orient itself.
- Preserve the durable note board as the source of truth while improving first-contact prompt clarity.
- Reduce unnecessary tool churn caused by the model re-reading a note solely to learn information the runtime already has at injection time.
- Keep prompt wording consistent with the current distinction between note delivery and transient Stylos talk delivery.

## Non-goals

- No redesign of note storage, board columns, or note identifiers.
- No removal of `board_read_note` or other `board_*` tools.
- No automatic state transitions such as moving a note to `in_progress` just because it was injected.
- No attempt to inline every possible board detail or historical activity into the injection wrapper.
- No change to note selection priority, idle-time injection timing, or delivery ordering.
- No change to network transport or receiver-side note creation semantics introduced by PRD-032.

## Background & Motivation

### Current state

Durable note injection already uses a note-specific prompt wrapper rather than pretending the note is a normal transient chat message. The runtime stores the durable record in SQLite first and later injects that note through the local prompt path when the target agent becomes idle.

However, the current injected note prompt is still too sparse for the user’s preferred workflow. In practice, the model may know that some note arrived but still need to call `board_read_note` immediately to recover basic context that the runtime already knows at injection time.

That extra tool call is wasteful when the goal is simply to tell the model:

- this is a durable note
- who sent it
- which note record it is
- which agent and instance it targets
- what board column it currently occupies
- what body content should be acted on

### Why the injection wrapper should carry note metadata directly

The user wants the injected prompt to “ensure the model knows it is a note” and to include node or note details as metadata so the model does not need to perform an immediate read-tool round trip.

That improves the first prompt in several ways:

- it reduces avoidable tool calls that do not add new information
- it makes the model less likely to misclassify the inbound work item as ordinary chat
- it makes note identity explicit early, which helps later board updates or result attachment
- it makes the prompt boundary between durable note delivery and transient talk delivery more legible

**Alternative considered:** keep the current thin wrapper and rely on the model to call `board_read_note` every time. Rejected: the runtime already has the relevant metadata at injection time, so forcing an immediate read for orientation adds noise rather than value.

### Why this should improve the prompt rather than bypass the board model

The durable board remains the source of truth. The injected prompt should not become a separate shadow representation that replaces the database record.

Instead, the injection wrapper should act like a concise envelope over the existing durable note record. The model can still call `board_read_note`, `board_move_note`, or `board_update_note_result` when it needs deeper inspection or wants to mutate state.

**Alternative considered:** remove note metadata from prompts entirely and train the model to inspect the board manually first. Rejected: that weakens the usefulness of deferred injection and makes the first-contact prompt unnecessarily opaque.

## Design

### Present injected work explicitly as a durable note

When the runtime injects a pending note into an idle agent, the prompt wrapper should say plainly that the inbound item is a durable note.

Normative behavior:

- the injected wrapper must identify the inbound work item as a note rather than as ordinary user input or Stylos talk
- the wrapper should use stable wording that is easy for the model to recognize across runs
- the body content should remain clearly separated from metadata fields
- the wrapper should continue to enter through the existing local prompt path used for note injection today

This makes the delivery mode clear before the model reads the note body itself.

**Alternative considered:** rely only on implicit wording such as “new task” without mentioning note semantics. Rejected: the user explicitly wants the model to know it is handling a note.

### Include core note metadata in the injected wrapper

The injected prompt should include the key metadata the runtime already knows for the selected note.

Normative behavior:

- include canonical `note_id`
- include human-friendly `note_slug` when available
- include sender instance `from` when known
- include sender agent identity `from_agent_id` when known
- include destination instance `to`
- include destination agent identity `to_agent_id`
- include current board column
- include created and updated timestamps only if the wrapper can do so without becoming noisy; if included, preserve millisecond units explicitly or render them with clear labeling
- include the durable note body as the main content payload after the metadata header

This should give the model immediate orientation while preserving the database-backed record as canonical.

**Alternative considered:** include only `note_id` and body. Rejected: that still leaves out sender/target context and reduces the benefit of the improved wrapper.

### Keep the wrapper concise and structured for model parsing

The note injection wrapper should provide enough metadata to avoid an orientation-only tool call, but it should not become verbose or conversational.

Normative behavior:

- prefer a compact, explicit metadata block with stable field names
- keep metadata separate from body text so the model does not confuse control data with requested work
- avoid formatting that looks like freeform chat authored by the sender
- prefer explicit note-oriented labels such as `type=note` or equivalent stable wording
- keep the wrapper small enough that repeated note delivery does not create unnecessary prompt bloat

A concise structured wrapper is easier for the model to recognize and less likely to be paraphrased incorrectly by future prompt adjustments.

**Alternative considered:** inject the entire JSON database row verbatim. Rejected: that would expose implementation detail too directly and add avoidable prompt noise.

### Keep `board_*` tools as follow-up tools rather than required orientation steps

This PRD improves the first injected prompt, but it does not remove the need for board tools.

Normative behavior:

- `board_read_note` remains available for deeper inspection when the model actually needs more than the injected wrapper provides
- `board_list_notes` remains useful for reviewing the broader board state
- `board_move_note` and `board_update_note_result` remain the correct way to mutate durable board state
- the injected wrapper should reduce orientation-only `board_read_note` calls, not forbid them

This preserves the current board abstraction while making note delivery more informative.

**Alternative considered:** de-emphasize or remove `board_read_note` once metadata is injected. Rejected: the board tools remain necessary for richer inspection and durable state management.

### Keep note injection distinct from talk injection

Talk and note delivery should remain visibly different in the prompt path.

Normative behavior:

- note injection wording should remain separate from the existing peer-message/talk wrapper style
- note metadata fields should reflect durable note semantics rather than talk/reply semantics
- the wrapper should not instruct the model to answer with `***QRU***` or otherwise behave as if it were processing a realtime talk message

This preserves the architectural distinction already present in transport, logging, and tooling.

**Alternative considered:** reuse the talk wrapper and append note fields informally. Rejected: that would blur the line between durable notes and realtime talk.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/agent.rs` | Update note injection prompt assembly so inbound idle-time note delivery includes explicit note identity and metadata in a stable wrapper format. |
| `crates/themion-cli/src/tui.rs` | Pass through or assemble the selected note metadata needed by the improved injection wrapper when scheduling local note delivery. |
| `crates/themion-core/src/db.rs` | No schema change expected; confirm existing note fields already provide the metadata needed for injection. |
| `docs/architecture.md` | Document that idle-time injected notes now carry explicit note metadata in the prompt wrapper rather than only minimal note text. |
| `docs/engine-runtime.md` | Document the injected note wrapper shape and clarify that it is intended to reduce orientation-only `board_read_note` calls. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- a note has no known sender instance or sender agent → verify: the wrapper still identifies the item as a note and uses an explicit empty/unknown representation rather than omitting note semantics entirely.
- a note body itself contains lines that resemble metadata → verify: wrapper structure keeps runtime metadata clearly separated from the note body.
- a note is injected repeatedly across restarts only because it was never marked injected → verify: each injected prompt still presents stable note metadata so repeated deliveries are clearly recognizable as the same durable note.
- a future field is added to durable notes → verify: the wrapper format can be extended without making existing fields ambiguous.
- the model still chooses to call `board_read_note` after injection → verify: this remains allowed, but the wrapper already provides enough context for the common first-response path.
- sender metadata is present for remote notes but absent for locally created notes → verify: local note injection still clearly identifies the durable note and target metadata without inventing sender values.

## Migration

No SQLite migration is required.

Behavioral migration is prompt-only:

- injected durable notes become more self-describing at first delivery
- existing board tools remain unchanged
- any prompt examples or docs describing note injection should be updated to show the richer wrapper

## Testing

- inject a pending note for an idle agent → verify: the resulting prompt explicitly identifies the work item as a durable note and includes core metadata such as `note_id`, source/target identities, and current column.
- inject a note and inspect the first model turn without calling `board_read_note` → verify: the model has enough context in-prompt to begin handling the note correctly.
- inject a note whose sender metadata is missing → verify: the wrapper still clearly marks it as a note and labels missing sender fields explicitly.
- inject a note whose body contains structured text or key-value-like lines → verify: wrapper metadata and body content remain clearly separated.
- compare injected talk versus injected note prompts → verify: note delivery uses note-specific wording and metadata rather than the talk/peer-message wrapper.
- run `cargo check -p themion-core -p themion-cli --features stylos` after implementation → verify: updated note injection prompt assembly compiles cleanly.

## Implementation checklist

- [ ] define a stable note-injection wrapper that explicitly marks the inbound item as a durable note
- [ ] include core note metadata in the injected wrapper without requiring an immediate `board_read_note` call
- [ ] keep metadata formatting concise and clearly separated from note body content
- [ ] preserve the distinction between note injection and talk injection wording
- [ ] update architecture docs to describe the richer note injection wrapper
- [ ] update engine runtime docs to explain the metadata-first note prompt behavior
- [ ] update `docs/README.md` with the new PRD entry
