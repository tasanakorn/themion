# PRD-064: Core-Resolved Board Note Display Metadata for Tool-Call Labels

- **Status:** Implemented
- **Version:** v0.40.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-27

## Summary

- Themion already stores human-friendly `note_slug` values for board notes, but some user-facing tool-call labels still show raw `note_id` UUIDs because the TUI currently formats labels from raw model-supplied tool arguments alone.
- The immediate bug is visible in `board_update_note_result`, and the same event-path limitation applies to `board_read_note` and any `board_move_note` call where the model omits `note_slug`.
- The best fix is a small core-owned event enrichment step: before emitting `ToolStart`, `themion-core` should copy the original arguments and add display-only fields such as resolved `note_slug` when it can.
- The TUI should keep formatting labels locally, but it should prefer the enriched display arguments over raw `arguments_json` and avoid new DB reads.
- Keep canonical `note_id` contracts unchanged for tool execution, storage, and machine-facing payloads; only the user-facing label path becomes richer.

## Goals

- Ensure user-facing board note tool-call labels prefer `note_slug` even when the model provided only `note_id`.
- Keep note display-resolution responsibility in `themion-core`, where the DB handle already exists and tool execution is orchestrated.
- Preserve existing canonical board tool contracts keyed by `note_id`.
- Give the TUI a display-ready event payload without turning it into another DB-reading coordination layer.
- Solve the current gap with the smallest clean event-contract change that can cover `board_read_note`, `board_move_note`, and `board_update_note_result`.

## Non-goals

- No change to the input schema for board note mutation or read tools; `note_id` remains the required selector.
- No change to SQLite schema, note lookup keys, or note slug generation.
- No requirement for the TUI to query the database directly to recover missing display metadata for tool labels.
- No change to the visible formatting policy for unrelated tools beyond preserving current behavior.
- No broader redesign of the entire `AgentEvent` system beyond the small display-enrichment hook needed for this path.
- No requirement in this slice to let models target notes by `note_slug` instead of `note_id`.

## Background & Motivation

### Current state

Themion already has a documented note-slug-first direction for user-facing board note surfaces:

- the durable board schema stores both canonical `note_id` and human-friendly `note_slug`
- receiver-side note intake, note injection, and done-mention chat events already prefer `note_slug`
- docs state that operator-facing board note events should prefer `note_slug` while machine-facing contracts preserve `note_id`

But the current tool-call event path still leaks raw IDs into the transcript in some cases:

- `AgentEvent::ToolStart` currently carries only the tool name and raw `arguments_json`
- `crates/themion-cli/src/tui.rs::split_tool_call_detail(...)` formats the visible label from those raw arguments alone
- `board_read_note` and `board_update_note_result` currently display `note_id` directly
- `board_move_note` already prefers `note_slug`, but only if the original arguments happened to include it
- in `crates/themion-core/src/agent.rs`, `ToolStart` is emitted before tool execution from `tc.function.arguments.clone()`, so the core runtime currently does not add display metadata even though it already has `self.db`

This is not a missing-data problem in SQLite. It is an event-boundary problem: the core runtime already owns the DB handle and knows which tool is about to run, but the display event does not carry any resolved presentation metadata.

### Why this matters

Showing raw UUIDs in a user-facing chat transcript:

- makes board activity harder to scan
- is inconsistent with the rest of Themion's note-slug-first presentation rules
- pushes a data-resolution concern toward the TUI even though the runtime already owns the relevant lookup capability

The repository's architecture guidance also prefers keeping reusable runtime behavior in `themion-core` and avoiding unnecessary TUI-side orchestration leakage. A TUI-local DB lookup would work, but it would further widen an already over-scoped `tui.rs`.

**Alternative considered:** have the TUI look up `note_slug` directly from SQLite during label formatting. Rejected: practical but directionally wrong for layering, because it adds another presentation-time DB dependency to recover runtime-owned metadata.

## Design

### Design principles

- Keep machine-facing tool execution contracts separate from user-facing display metadata.
- Resolve display metadata in the runtime layer that already owns the DB handle and understands tool semantics.
- Preserve the existing TUI responsibility for formatting chat labels, trimming, and reason display.
- Make the smallest event-shape change that solves the board-note label gap without turning `ToolStart` into a broad new transport abstraction.

### 1. Use an optional `display_arguments_json` field on `AgentEvent::ToolStart`

The best approach is to extend `AgentEvent::ToolStart` with one additional optional payload:

- keep `name`
- keep raw `arguments_json` exactly as emitted by the model tool call
- add optional `display_arguments_json: Option<String>`

Why this is the best fit for the current code:

- `ToolStart` is already emitted once in `crates/themion-core/src/agent.rs` before the tool runs
- the TUI already has one narrow formatting entrypoint, `split_tool_call_detail(name, args_json)`
- an optional parallel JSON string lets the TUI reuse the same formatter with minimal churn by choosing enriched args first and raw args second
- a string payload matches the current event style and avoids introducing a larger typed cross-crate display structure just to add one or two board-note fields

Normative behavior:

- `arguments_json` remains the original model-supplied payload for audit/debug visibility
- `display_arguments_json` is best-effort and presentation-only
- if no enrichment is available, `display_arguments_json` is `None` and current behavior remains intact

**Alternative considered:** add a typed `ToolDisplayMetadata` struct. Rejected for this slice: cleaner in the abstract, but larger than needed for one narrow enrichment path and less aligned with the current JSON-based formatter boundary.

### 2. Resolve board note display metadata in core by cloning and enriching the original args JSON

Before emitting `ToolStart`, `themion-core` should inspect board note tools and build `display_arguments_json` by starting from the original arguments and adding display-only fields when it can resolve them.

Initial scope should include:

- `board_read_note`
- `board_move_note`
- `board_update_note_result`

Resolution rules:

- parse the outgoing raw `arguments_json` into a JSON object when the tool name is one of the board note tools above
- read `note_id` from that object if present
- query the local DB for the note with `self.db.get_board_note(note_id)`
- if the note exists and the parsed object does not already include a usable `note_slug`, insert `note_slug` into the cloned JSON object
- serialize that enriched object as `display_arguments_json`
- if parsing fails, the note is missing, or serialization fails, fall back to `None` and continue normally
- never fail or block tool execution because display enrichment was unavailable

This is preferable to building a separate custom display object because it keeps the TUI formatter contract simple: it still sees ordinary arguments-shaped JSON, just with a more complete display view.

**Alternative considered:** emit only the resolved `note_slug` separately and let the TUI merge it conceptually. Rejected: that creates special-case frontend merge logic when simply enriching an arguments-shaped JSON object is smaller and easier to reuse.

### 3. Keep TUI rendering local, but prefer enriched args over raw args

The TUI should remain responsible for formatting user-visible labels, but it should use the enriched display arguments when available.

Expected behavior in `crates/themion-cli/src/tui.rs`:

- `AgentEvent::ToolStart` handling reads `display_arguments_json.as_deref().unwrap_or(&arguments_json)`
- existing `split_tool_call_detail(...)` is reused unchanged or with only minor cleanup
- board note label branches prefer `note_slug` over `note_id`
- specifically, `board_read_note` and `board_update_note_result` should switch to the same note display helper style already used by `board_move_note`

This keeps formatting policy in the TUI while moving data resolution to core.

**Alternative considered:** have core emit fully formatted display strings such as `board_update_note_result <slug>`. Rejected: that would pull trim policy and future transcript formatting changes into core, which is unnecessary for this fix.

### 4. Keep the enrichment hook narrowly scoped to board note display for now

This PRD should intentionally keep the first implementation narrow.

What should land now:

- optional `display_arguments_json` on `ToolStart`
- core-side enrichment for the three board note tools that use `note_id`
- TUI preference for enriched args and note-slug-first formatting in those branches

What should not be implied yet:

- a general-purpose metadata bus for every tool
- a broad typed display-model layer
- extra lookups for tools that do not currently have a real readability problem

This keeps the change easy to review and less likely to sprawl.

**Alternative considered:** generalize immediately to all tools with a richer display-event framework. Rejected: over-designed for the current bug and not necessary to improve layering in this specific path.

### 5. Keep board tool contracts and docs explicit about display-only enrichment

Board note tools should continue to accept canonical `note_id` and return their current structured acknowledgements. The new event enrichment should be documented as a user-facing display behavior, not as a contract change.

Docs should state clearly:

- tool execution and storage still use `note_id`
- `ToolStart` may carry display-enriched arguments for frontend formatting
- board note tool-call labels may show `note_slug` because the runtime resolves display metadata before the TUI renders the event
- this applies even when the original model-supplied tool arguments omitted `note_slug`

That distinction preserves machine contract stability while aligning user-facing behavior with the note-slug-first rule.

### 6. Acceptance target for the first implementation

This PRD should be considered implemented when all of the following are true:

- `AgentEvent::ToolStart` carries optional `display_arguments_json` alongside raw `arguments_json`
- `themion-core` resolves `note_slug` from `note_id` for `board_read_note`, `board_move_note`, and `board_update_note_result` before emitting the event when the note exists locally
- the TUI prefers `display_arguments_json` and shows `note_slug` for those board note tool labels when available
- `board_read_note`, `board_move_note`, and `board_update_note_result` all fall back cleanly to raw `note_id` behavior when enrichment is unavailable
- no new TUI-side DB lookup is required for this label behavior
- board tool execution contracts remain keyed by canonical `note_id`
- docs reflect the landed behavior and layering rationale
- `cargo check -p themion-core -p themion-cli` passes
- `cargo check --all-features -p themion-core -p themion-cli` passes

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/agent.rs` | Extend `AgentEvent::ToolStart` with optional `display_arguments_json` and populate it before emission for board note tools. |
| `crates/themion-core/src/agent.rs` or a nearby helper module | Add a small helper that clones raw tool args JSON and enriches board note calls with resolved `note_slug` for display. |
| `crates/themion-cli/src/tui.rs` | Prefer `display_arguments_json` when formatting tool-call labels and update `board_read_note` / `board_update_note_result` to use note-slug-first display logic. |
| `docs/architecture.md` | Document that board note tool-call labels use runtime-resolved display metadata from core rather than requiring TUI DB lookups. |
| `docs/engine-runtime.md` | Document that `ToolStart` carries raw arguments plus optional display-enriched arguments for frontend formatting. |
| `docs/README.md` | Add and later update the PRD entry status/version when the work lands. |

## Edge Cases

- the model provides only `note_id` and the note exists locally → verify: the visible board tool label uses `note_slug`.
- the model provides only `note_id` and the note lookup fails → verify: the event still renders with `note_id` and the tool still executes normally.
- the model already provided `note_slug` in the original arguments → verify: the display path stays consistent and does not regress.
- a board note slug is very long → verify: existing center-trim behavior still keeps the display compact.
- non-board tools emit `ToolStart` → verify: their behavior is unchanged unless they later opt into display enrichment.
- malformed or non-object raw `arguments_json` reaches the enrichment helper → verify: enrichment is skipped safely and the raw formatter path still works.

## Migration

This is a presentation and event-shape change with no database migration.

Rollout guidance:

- extend `ToolStart` with optional display args in `themion-core`
- enrich board note tool-start events with resolved note metadata
- update the TUI to prefer enriched display args while retaining the existing raw fallback
- align architecture/runtime docs and PRD tracking with the landed behavior

## Testing

- trigger `board_update_note_result` for a note where only `note_id` is present in the model-supplied tool args → verify: the visible tool-call label shows the resolved `note_slug`.
- trigger `board_move_note` for a note where only `note_id` is present in the model-supplied tool args → verify: the visible tool-call label shows the resolved `note_slug` and the target column.
- trigger `board_read_note` for a known note → verify: the visible tool-call label prefers `note_slug`.
- trigger a board note tool call whose `note_id` does not resolve locally → verify: the label falls back to `note_id` without breaking tool execution.
- trigger a non-board tool call such as `fs_read_file` → verify: its visible label behavior is unchanged.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: the touched crates compile cleanly in the default feature set.
- run `cargo check --all-features -p themion-core -p themion-cli` after implementation → verify: the touched crates compile cleanly with all features enabled.

## Implementation checklist

- [x] extend `AgentEvent::ToolStart` with optional `display_arguments_json`
- [x] add a core-side helper that clones and enriches board note tool args with resolved `note_slug` for display
- [x] emit enriched display args for `board_read_note`, `board_move_note`, and `board_update_note_result`
- [x] update TUI tool-start handling to prefer enriched display args when formatting labels
- [x] update `board_read_note` and `board_update_note_result` label branches to use note-slug-first display logic
- [x] keep raw tool contracts and execution keyed by canonical `note_id`
- [x] update `docs/architecture.md`, `docs/engine-runtime.md`, and `docs/README.md`
- [x] run `cargo check -p themion-core -p themion-cli`
- [x] run `cargo check --all-features -p themion-core -p themion-cli`
