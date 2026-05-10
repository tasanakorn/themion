# PRD-118: Improve Codex `response.completed` Handling and Stream-Event Visibility

- **Status:** Draft
- **Version:** >v0.72.1 +patch
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-10

## Summary

- Themion currently treats Codex `response.completed` as only “usage + end of stream”.
- Parse and keep `end_turn` explicitly so the Codex integration stops ignoring provider completion semantics.
- If Codex sends `end_turn=false`, the client should try to continue the provider turn instead of always stopping after the first completed stream.
- Continued chunks should follow normal Codex streaming behavior and accumulate into the same logical provider turn.
- Show compact TUI transcript notices for truly unhandled Codex stream events.
- Keep known intentionally ignored Codex events quiet when they look like non-actionable metadata.

## Goals

- Make Codex completion handling explicit, documented, and testable.
- Preserve current usage accounting from `response.completed`.
- Record provider `end_turn` and use it to decide whether the Codex provider turn should continue.
- Try to continue Codex execution when the provider reports `end_turn=false`.
- Reuse the existing Codex continuation and accumulation behavior where that logic already exists.
- Show unexpected Codex chunk types during live runs without requiring debug builds or source inspection.
- Avoid transcript spam from known non-actionable metadata events that Themion intentionally does not use yet.
- Keep Codex stream parsing and event categorization in `themion-core`.

## Non-goals

- Do not redesign the whole provider abstraction.
- Do not make TUI decide which Codex events are handled, ignored, or important.
- Do not add full product use of reasoning-summary, reasoning-content, or server-model metadata.
- Do not change non-Codex providers.

## Background & Motivation

### Current state

`crates/themion-core/src/client_codex.rs` currently handles `response.completed` by reading usage and setting the stream loop to done. It does not inspect or store `end_turn`.

The same parser ignores several other known Codex event types. Right now that creates two gaps:

1. provider completion semantics are only partially implemented
2. when upstream sends an event Themion does not use, the product usually stays silent

`docs/codex-integration-guide.md` now documents that `response.completed` is only partially handled and that multiple stream events are ignored.

### Why this matters now

If Codex uses `end_turn=false` to say “this streamed response is complete, but the turn should continue”, dropping that signal makes the integration incorrect.

At the same time, fully silent ignore behavior makes stream debugging slower. We need visibility for real gaps, but we should not flood the transcript for known benign events that are only metadata and do not change current product behavior.

## Design

### 1. Parse `response.completed` into explicit completion metadata

Themion must stop treating `response.completed` as an opaque “done” marker.

Required behavior:

- parse `response.completed` into a small structured completion view with at least:
  - provider response id when present
  - usage when present
  - `end_turn` when present
- keep current usage extraction behavior unchanged
- make the completion metadata available to the rest of the Codex client flow and, when useful, to shared runtime tracing
- keep the completion metadata shape compact and provider-specific rather than forcing a large cross-provider redesign

The implementation must distinguish these separate facts:

- the current SSE stream finished
- the provider said the turn should end or continue
- Themion ended or continued the provider turn
- Themion later ended the local harness turn

### 2. Continue the provider turn when `end_turn=false`

This PRD changes Codex completion handling from “always stop after one completed stream” to “continue when the provider says the turn is not done yet.”

Required behavior:

- when `response.completed` includes `end_turn=true`, keep the current effective behavior and finish the provider turn
- when `response.completed` includes `end_turn=false`, do not stop the overall Codex provider turn after that completed stream
- when `response.completed` omits `end_turn`, treat that as "provider did not say" and preserve the existing one-stream-stop fallback unless implementation evidence supports a safer default
- after `end_turn=false`, the client must try to continue provider execution using the existing Codex continuation path and current continuation logic where that logic already exists
- continued chunks must follow the same streaming behavior Codex already uses in normal continued execution
- continued assistant text must append into the same logical provider turn result
- continued tool-call state must remain part of the same logical provider turn result
- usage from all continuation segments must accumulate into one combined usage result for that logical provider turn
- if continuation succeeds, the client must keep accumulating assistant output, tool-call state, usage, and provider notices across the continued provider turn
- if continuation is not supported or fails, the client must emit one compact runtime/transcript-visible notice and then stop safely
- the fallback failure notice text must use this exact format:
  - `codex stream: completed end_turn=false continuation=failed`

This PRD does not require a broad provider abstraction rewrite. It should reuse existing Codex continuation and accumulation behavior instead of introducing a separate special path.

### 3. Categorize Codex stream events into handled, known-ignored, and unhandled

Themion should make stream visibility intentional instead of relying on a generic default ignore path.

Required behavior:

- classify Codex events into three groups:
  - handled
  - known-ignored
  - unhandled
- handled events drive current product behavior
- known-ignored events are recognized and intentionally not used in the current product slice
- unhandled events are recognized as unsupported or unexpected enough to deserve a visible notice
- keep the category decision in `themion-core`, next to the Codex parser

This makes stream behavior auditable and keeps future event-support changes localized.

### 4. Show compact notices only for important unsupported events

TUI visibility should help debugging, not create transcript noise.

Required behavior:

- emit a compact runtime/transcript notice when an unhandled Codex event is seen
- include the event name
- keep the notice stable and short
- mark it as provider/runtime output, not assistant text
- deduplicate repeated identical unhandled events within one provider turn
- the notice text must use this exact format:
  - `codex stream: unhandled event=<event_name>`
- the dedup rule must be exact:
  - emit at most one unhandled-event notice per distinct event name per provider turn

This PRD does not require count suffixes, aggregate summaries, or verbose payload dumps.

### 5. Keep non-actionable metadata events low-noise

Some Codex events look like background metadata. They may be real, but they do not currently change Themion behavior and do not help a user understand the active turn.

Required behavior:

- maintain an explicit known-ignored event set in the Codex client
- known-ignored events must not emit the unhandled-event notice format
- for this PRD, known-ignored events must be silent in normal transcript output
- only events that currently look like non-actionable metadata should enter the initial known-ignored set
- document the exact initial known-ignored set in `docs/codex-integration-guide.md`

An event is a good fit for the initial known-ignored set when all of these are true:

- it looks informational rather than behavioral
- it does not affect current message text, tool calls, usage, or turn-end handling
- it does not look like a finalization or control signal
- repeated appearance would mostly add noise rather than help debugging

The initial known-ignored set for this PRD is:

- `Created`
- `ServerModel`
- `ModelVerifications`
- `ServerReasoningIncluded`
- `ModelsEtag`

The following events are not part of the initial known-ignored set because they may reflect behavior that is still worth noticing during integration work:

- `response.output_item.done`
- `ReasoningSummaryPartAdded`
- `ReasoningSummaryDelta`
- `ReasoningContentDelta`

Those events must remain transcript-visible as unhandled until a later PRD or implementation slice gives them explicit handled or known-ignored semantics.

### 6. Keep ownership layered correctly

This work must follow the repository runtime-ownership rules.

Required behavior:

- Codex SSE parsing, completion metadata capture, continuation handling, event categorization, and provider-notice decisions belong in `themion-core`
- agent/runtime event forwarding belongs in the runtime layer
- TUI only renders the resulting runtime/provider notices
- do not add TUI-side logic that parses Codex event names or decides which ones matter
- if a new shared runtime event type is needed, add it at the runtime boundary rather than creating TUI-only behavior

### 7. Keep docs aligned with the real event matrix

The Codex integration guide should remain the fast source of truth.

Required behavior:

- update `docs/codex-integration-guide.md` when implementation lands
- state clearly how `response.completed` is now handled
- state clearly that `end_turn=false` makes Themion try to continue the provider turn
- state clearly that continued chunks follow normal Codex streaming behavior and accumulate into one logical provider turn
- state what happens when continuation fails or is unsupported
- state which events are handled, which are known-ignored, and which create transcript-visible unhandled notices
- keep the documentation in compact checklist/table form for future audits

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/client_codex.rs` | Parse structured completion metadata from `response.completed`, retain `end_turn`, continue the provider turn when `end_turn=false`, reuse existing continuation/accumulation behavior, classify stream events as handled / known-ignored / unhandled, emit the exact continuation-failure notice when needed, and emit exact-format unhandled-event notices with per-turn deduplication. |
| `crates/themion-core/src/client.rs` | Extend shared response or backend flow structures only if needed to carry provider completion metadata or support the minimal continuation flow cleanly. |
| `crates/themion-core/src/agent.rs` | Preserve and forward any new provider/runtime notice events through the existing runtime event flow without moving policy into TUI. |
| `themion-cli` transcript/TUI display path | Render compact provider/runtime notices in the transcript using existing non-assistant event styling. |
| `docs/codex-integration-guide.md` | Update the event checklist/table and document `response.completed`, `end_turn`, continuation behavior, the exact known-ignored set, and the exact visible unhandled-event behavior. |
| `docs/README.md` | Track this PRD and later update status/version when implemented. |

## Edge Cases

- `response.completed` arrives with `usage` and `end_turn=true` → verify: usage is preserved and current round-finish behavior stays unchanged.
- `response.completed` arrives with `usage` and `end_turn=false` → verify: the signal is recorded and the client attempts continuation instead of stopping immediately.
- `response.completed` arrives without `end_turn` → verify: Themion records the missing value distinctly and applies the documented fallback.
- continuation after `end_turn=false` produces more assistant text → verify: the text is appended into the same logical provider turn result.
- continuation after `end_turn=false` produces tool calls → verify: tool-call state remains correct across the continued provider turn.
- continuation after `end_turn=false` produces more usage data → verify: usage is accumulated into one combined logical provider turn result.
- continuation after `end_turn=false` is unsupported or fails → verify: exactly one `codex stream: completed end_turn=false continuation=failed` notice is emitted and the client stops safely.
- the same unhandled event appears many times in one provider turn → verify: only one `codex stream: unhandled event=<event_name>` notice appears for that event name in that turn.
- repeated `Created`, `ServerModel`, or `ModelsEtag` events appear → verify: transcript stays silent because they are in the initial metadata-only ignore set.
- a reasoning event appears during a normal response → verify: it remains visible as unhandled in this first slice.
- a truly new Codex event appears after an upstream change → verify: transcript shows one compact discoverable unhandled-event notice.
- non-Codex providers stream normally → verify: they do not gain Codex-specific transcript noise.

## Migration

This is an additive provider-correctness and runtime-visibility change. No database migration is required unless implementation later chooses to persist completion metadata.

Patch scope is appropriate if the change stays within the existing provider/backend structure and only adds the minimum continuation support needed for Codex `end_turn=false`.

## Testing

- simulate or unit-test `response.completed` with `end_turn=true` → verify: usage is captured and provider-turn completion matches current behavior.
- simulate or unit-test `response.completed` with `end_turn=false` followed by a successful continuation → verify: the client attempts continuation and returns the combined provider-turn result.
- simulate or unit-test `response.completed` with `end_turn=false` followed by additional streamed assistant text → verify: the text is appended using normal Codex streaming behavior.
- simulate or unit-test `response.completed` with `end_turn=false` followed by additional usage data → verify: usage is accumulated into one combined logical provider turn result.
- simulate or unit-test `response.completed` with `end_turn=false` followed by continuation failure → verify: the client emits exactly one `codex stream: completed end_turn=false continuation=failed` notice and stops safely.
- simulate or unit-test `response.completed` without `end_turn` → verify: fallback behavior matches the documented rule.
- stream `response.output_item.done` → verify: it produces exactly one `codex stream: unhandled event=response.output_item.done` notice per provider turn.
- stream repeated known-ignored metadata events such as `Created` or `ServerModel` → verify: transcript output stays silent.
- stream a reasoning event such as `ReasoningSummaryDelta` → verify: transcript shows exactly one `codex stream: unhandled event=<event_name>` notice for that event name in that turn.
- stream a fabricated unknown Codex event name → verify: transcript shows exactly one `codex stream: unhandled event=<event_name>` notice for that event name in that turn.
- run a normal Codex text-only response → verify: assistant text streaming still works.
- run a normal Codex tool-call response → verify: tool-call registration and argument accumulation still work.
- run `cargo check -p themion-core` → verify: core changes compile.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` → verify: default CLI build compiles.
- run `cargo check -p themion-cli --features stylos` → verify: stylos-enabled CLI build compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.

## Implementation checklist

- [ ] add a small structured representation for Codex completion metadata
- [ ] parse and retain `end_turn` from `response.completed`
- [ ] preserve current usage extraction while separating stream-complete from provider-turn-complete semantics
- [ ] reuse the existing Codex continuation path to try continuation after `end_turn=false`
- [ ] keep continued assistant text streaming in the normal Codex way
- [ ] keep accumulating assistant output, tool calls, usage, and notices across a continued provider turn
- [ ] emit exactly one `codex stream: completed end_turn=false continuation=failed` notice when continuation cannot be completed successfully
- [ ] define handled / known-ignored / unhandled event categories in the Codex client
- [ ] emit exactly one `codex stream: unhandled event=<event_name>` notice per distinct unhandled event name per provider turn
- [ ] implement the initial metadata-only known-ignored event set exactly as specified in this PRD
- [ ] render provider/runtime notices cleanly in the TUI transcript without TUI-owned categorization logic
- [ ] update `docs/codex-integration-guide.md` to match the implemented event matrix
