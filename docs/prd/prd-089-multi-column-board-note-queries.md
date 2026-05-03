# PRD-089: Multi-Column Board Note Queries

- **Status:** Implemented
- **Version:** v0.57.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-03

## Implementation status

Landed in `v0.57.0` as a shared board-note query improvement centered in `themion-core`, with a canonical `board_list_notes` filter contract based on one optional `columns` array. The implementation preserves existing ordering, supports one-query multi-column reads, and avoids client-side fan-out for common combined board views.

## Summary

- Current board-note listing only supports filtering by one column at a time, which forces repeated queries when a caller wants to inspect more than one active state together.
- Add support for querying multiple board columns in one request so local tools, runtime code, and future board views can fetch combined board state efficiently without client-side query fan-out.
- Keep board note semantics, ordering rules, and done-mention behavior unchanged in this PRD.
- Use one canonical filter property for this concept so the tool contract stays compact and model-friendly.

## Goals

- Allow board-note queries to filter by multiple columns in one request.
- Preserve existing target filters such as `to_instance` and `to_agent_id`.
- Preserve current board-list ordering semantics for equivalent result sets.
- Eliminate routine repeated `board_list_notes` calls plus caller-side merging for common combined views.
- Provide one shared query primitive that tool, runtime, and future board-view code can reuse.
- Keep the public tool contract to one canonical column-filter shape.

## Non-goals

- No redesign of board columns or column semantics.
- No change to board note creation, movement, completion, or done-mention rules.
- No requirement to add arbitrary boolean query expressions, negation, or general search syntax for board notes.
- No change to pending-note injection ordering or watchdog selection logic unless implementation reveals a tiny compatibility fix is strictly necessary.
- No attempt to redesign the full board UI in this PRD.
- No permanent dual-parameter compatibility contract for this filter concept.

## Background & Motivation

### Current state

Board-note queries are currently narrower than the workflows that consume them.

Today:

- `themion-core::db::DbHandle::list_board_notes` accepts at most one optional `NoteColumn`
- the `board_list_notes` tool exposes the same one-column filter shape
- callers that want a combined view such as `todo + blocked` or all non-done notes must issue multiple queries and merge them outside the DB layer

That limitation is workable for simple inspection, but it becomes awkward when runtime code, tools, or future board surfaces need a coherent combined view across more than one column. Repeated single-column queries increase duplicated code, encourage inconsistent caller-side merging, and create avoidable extra DB round trips for a capability the storage layer can answer directly.

### Why this should be a dedicated improvement

This is a small but meaningful query-surface capability gap.

The board system already has a stable set of column values and a central DB query path. Adding first-class multi-column filtering keeps query logic in one place and gives future board/runtime work a cleaner primitive to build on.

**Alternative considered:** keep single-column queries only and let each caller merge results. Rejected: repeated client-side merging is a predictable source of duplication and inconsistent behavior for a capability the storage layer can answer directly.

## Design

### Implementation-ready decisions

This PRD is implementation-ready with the following concrete contract decisions:

- `board_list_notes` uses one optional `columns` array field whose values are board column strings.
- An omitted `columns` field means "all columns".
- A one-item `columns` array means a single-column filter.
- A multi-item `columns` array means an inclusive filter over those columns.
- Duplicate values inside `columns` are accepted and normalized without duplicating result rows.
- Invalid column names remain a request error.
- The DB layer should expose one shared list-query primitive that accepts zero, one, or many columns rather than leaving callers to fan out into repeated single-column queries.

### 1. Add an explicit multi-column filter shape with one canonical property

The board query surface should accept one optional `columns` array.

Required behavior:

- omitted `columns` still means "all columns"
- callers may request one or many columns in one query
- single-column filtering remains easy through a one-item array
- the public tool contract should not require a precedence rule between overlapping parameter names

Concrete tool contract:

- `columns` is an array whose values are `"todo" | "in_progress" | "blocked" | "done"`
- callers should use `columns=["todo"]` for a single-column filter
- callers should use `columns=["todo","blocked"]` for a multi-column filter

This keeps the contract simple for agents and humans: one concept, one property, one documented behavior.

### 2. Keep board-list semantics narrow and explicit

This PRD should improve the existing board-list filter, not turn it into a general query language.

Required behavior:

- column filters are inclusive over a finite list of known board columns
- result ordering stays aligned with the current board-list ordering unless implementation shows a correctness issue
- target filters such as `to_instance` and `to_agent_id` continue to compose with the new column filter
- duplicate column names in `columns` must not duplicate result rows
- invalid column names still produce a clear error

This keeps the query shape easy to reason about while solving the real capability gap.

### 3. Centralize the multi-column logic in the DB/query layer

This capability should not be specified as a tool-only convenience. The shared DB query path is the contract that matters, because both tools and runtime-owned board coordination may need the same combined-column view.

The main logic should live in the shared board-note query path rather than in each tool or UI caller.

Required behavior:

- the DB layer should support querying zero, one, or many columns cleanly
- higher layers should call the shared query primitive instead of issuing repeated single-column queries and merging results manually
- implementation should avoid broad query duplication across runtime, tool, and UI layers

Concrete implementation direction:

- replace the current single `Option<NoteColumn>` list filter with a shared list-query shape that can represent all-columns, one-column, or many-columns filtering
- build the SQL filter as one query with an inclusive `IN (...)`-style column condition when explicit columns are supplied
- keep the existing ordering by `created_at_ms ASC` unless implementation uncovers a correctness issue that must be documented in the same change

**Alternative considered:** implement multi-column support only in the tool layer by issuing repeated DB calls internally. Rejected: that would preserve the core limitation and leave runtime callers without the same reusable primitive.

### 4. Support explicit lists, not synthetic presets

The design should support common combined views such as:

- `todo + blocked`
- all active non-done work
- custom subsets chosen by a caller

But this PRD should not hardcode one special synthetic mode such as `open`, `active`, or `pending` unless that becomes separately desirable later.

Required behavior:

- the query API should be generic over an explicit list of allowed column names
- callers may choose which subsets they need
- the first implementation should expose explicit columns rather than introducing preset aliases that hide which concrete columns were queried
- docs and tool descriptions should make the combined-column capability discoverable

### 5. Preserve compact agent ergonomics

Because tool descriptions are model-facing contracts, the improved interface should stay compact and explicit.

Required behavior:

- single-column requests remain easy to express through a one-item array
- the multi-column form should be straightforward for tools and prompt instructions to use
- docs should explain the `columns` contract clearly
- transcript or debug labeling should remain readable when explicit multi-column filters are used

Example intent: a caller that previously needed separate `todo` and `blocked` queries should be able to ask for both columns directly instead of reconstructing one logical board view from multiple tool calls.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/db.rs` | Change the board-note list query path from a single optional column filter to a shared zero/one/many-column filter shape, and implement one SQL query that applies inclusive multi-column filtering while preserving current ordering. |
| `crates/themion-core/src/tools.rs` | Update `board_list_notes` schema and argument handling to accept one optional `columns` array, normalize duplicate column values, and return clear errors for invalid values. |
| `crates/themion-core/src/agent.rs` | Refresh tool exposure metadata if needed so the updated `board_list_notes` contract stays consistent in prompt/tool definitions. |
| `crates/themion-cli/src/tui.rs` | Update user-facing tool labeling only if needed so multi-column calls render clearly in transcript output. |
| `docs/README.md` | Keep the PRD index entry aligned with the final status/version when implementation lands. |

## Edge Cases

- caller passes no `columns` filter → verify: the query still returns notes from all columns.
- caller passes one valid column in `columns` → verify: behavior matches the old single-column result set.
- caller passes multiple valid columns → verify: results include notes from all requested columns without requiring caller-side merging.
- caller passes duplicate columns in `columns` → verify: results are not duplicated and input handling is documented clearly.
- caller passes an invalid column name in `columns` → verify: the request fails with a clear validation error.
- caller combines multi-column filtering with `to_instance` and `to_agent_id` filters → verify: all filters compose correctly.
- future runtime code uses the shared query for active-work views → verify: no extra client-side query fan-out is needed for the common combined cases.

## Migration

This is a query-surface improvement and contract simplification.

Required rollout behavior:

- expose one canonical `columns` filter in the public tool contract
- update docs and tool descriptions so new callers discover the canonical shape
- avoid permanently carrying overlapping parameter names for the same concept

No data migration is required.

## Testing

- call `board_list_notes` without `columns` → verify: all board notes are returned as before.
- call `board_list_notes` with `columns=["todo"]` → verify: results match the old single-column behavior.
- call `board_list_notes` with `columns=["todo","blocked"]` → verify: one query returns notes from both columns.
- call `board_list_notes` with `columns=["todo","todo"]` → verify: results do not duplicate notes and duplicate-input handling is stable.
- call `board_list_notes` with `to_instance`, `to_agent_id`, and `columns=["todo","blocked"]` → verify: target filters and multi-column filtering compose in one query.
- run `cargo check -p themion-core` after implementation → verify: default core build compiles with the new query shape.
- run `cargo check --all-features -p themion-core` after implementation → verify: all-features core build still compiles cleanly.
- run the relevant board-note tests after implementation → verify: single-column-equivalent and multi-column behavior both pass.

## Implementation checklist

- [x] add a shared DB query shape for zero/one/many board columns
- [x] implement one inclusive SQL query path for explicit multi-column filtering
- [x] update `board_list_notes` tool schema to expose the canonical `columns` filter
- [x] normalize duplicate `columns` values without duplicating result rows
- [x] preserve single-column filtering through a one-item `columns` array
- [x] update any user-facing tool labeling that becomes unclear with multi-column requests
- [x] keep `docs/README.md` status/version aligned when implementation lands
- [x] validate with `cargo check -p themion-core`
- [x] validate with `cargo check --all-features -p themion-core`
