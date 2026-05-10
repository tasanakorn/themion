# PRD-119: Replace Split Board Note Mutation Tools with One Canonical Update Tool

- **Status:** Implemented
- **Version:** v0.73.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-10

## Summary

- Board-note follow-up often needs two calls in a row: move the note to a new column and update its result text.
- Replace `board_move_note` and `board_update_note_result` with one canonical `board_update_note` tool.
- One call can change column, result text, or both.
- This reduces both runtime tool-call count and static tool-schema overhead for the board surface.
- Update prompt/runtime guidance and docs so the canonical board mutation surface becomes create, list, read, update.

## Goals

- Reduce model-facing tool count for common board-note follow-up work.
- Allow one call to update a note's column and result text together.
- Keep single-purpose updates possible through the same merged tool.
- Remove overlapping board-note mutation tools so one concept has one canonical schema.
- Reduce static prompt overhead by shrinking the board-note mutation tool surface.
- Make `board_update_note` the only board-note mutation surface in tools, prompts, and docs.
- Keep board-note mutation ownership in the existing runtime and storage layers.

## Non-goals

- Do not keep `board_move_note` or `board_update_note_result` as compatibility aliases.
- Do not change `board_create_note`, `board_list_notes`, or `board_read_note`.
- Do not redesign board columns, note kinds, result semantics, or done-mention workflow.
- Do not add batch note updates or multi-note transactions.
- Do not move board policy into TUI or Web UI code.
- Do not require database schema changes if the current note model already supports the combined update.

## Background & Motivation

### Current state

The current board tool surface splits note mutation across two tools:

- `board_move_note` changes the note column
- `board_update_note_result` changes the note result text

In real use, agents often do both together. A common completion path is:

1. move the note to `done`
2. write the result text

This means one logical note update often becomes two tool calls.

The split also keeps two overlapping tool schemas in every round. Themion has already identified static schema cost as a real prompt-budget issue in PRD-071. Board-note mutation is a good place to simplify because the two current tools act on the same entity and are often used together.

Current docs and prompt/runtime guidance still describe the active board mutation surface in split terms. If implementation changes the tool surface without updating that guidance, the docs will drift and models may keep calling removed tool names.

### Why this matters now

Themion already has two relevant design directions:

- reduce static tool-schema overhead where possible
- prefer one canonical parameter shape per concept when practical

Board-note mutation fits both rules. Column and result text are fields on the same note. A merged tool reduces repeated schema text, reduces common two-call sequences to one call, and makes the board surface easier to teach.

This PRD is intentionally narrow. It does not change note workflow or board runtime ownership. It only replaces split mutation verbs with one canonical note-update surface and updates the related prompt/runtime guidance to match.

## Design

### 1. Add one canonical merged note-update tool

Add one board-note mutation tool and make it the only supported mutation surface.

Required tool name:

- `board_update_note`

Required parameters:

- `note_id` — required
- `column` — optional
- `result_text` — optional

Required behavior:

- the tool must accept updating only `column`
- the tool must accept updating only `result_text`
- the tool must accept updating both fields in one call
- the tool must reject calls that provide neither `column` nor `result_text`
- the tool must preserve the existing valid column set: `todo`, `in_progress`, `blocked`, `done`
- the tool must keep note identity based on `note_id`; this PRD does not add slug-based mutation

Recommended schema shape:

```json
{
  "name": "board_update_note",
  "description": "Update one board note. Change column, result text, or both.",
  "parameters": {
    "type": "object",
    "properties": {
      "note_id": {
        "type": "string",
        "description": "Board note id."
      },
      "column": {
        "type": "string",
        "enum": ["todo", "in_progress", "blocked", "done"],
        "description": "New column. Omit to keep current column."
      },
      "result_text": {
        "type": "string",
        "description": "Result text. Omit to keep current result."
      }
    },
    "required": ["note_id"]
  }
}
```

This PRD prefers one canonical parameter shape over parallel overlapping tools.

### 2. Remove the split mutation tools

`board_move_note` and `board_update_note_result` must be removed from the model-facing tool surface in this slice.

Required behavior:

- `board_move_note` must no longer appear in normal tool schemas
- `board_update_note_result` must no longer appear in normal tool schemas
- tool dispatch must no longer accept those names as active board-note mutation tools
- prompt and documentation guidance must stop teaching the split mutation pair
- unknown-tool behavior is acceptable for old callers after removal

This removal is intentional product simplification. Keeping the old tools would preserve extra schema cost and leave two overlapping ways to express the same note mutation.

### 3. Make `board_update_note` the only board-mutation verb in prompt guidance

This PRD simplifies board-note mutation to one tool and one concept.

Required behavior:

- prompt and tool guidance should refer to `board_update_note` for note mutation
- when board-note examples show moving a note and writing a result, they should use one `board_update_note` call when possible
- board-note guidance should simplify the mutable board surface to:
  - `board_create_note`
  - `board_list_notes`
  - `board_read_note`
  - `board_update_note`
- remove wording that presents move/result as separate canonical verbs

The main user-facing explanation should become simple: create notes, inspect notes, update notes.

### 4. Update prompt/runtime instruction guides in the same slice

This PRD changes durable prompt-facing behavior, so the matching guidance docs must change in the same implementation.

Required behavior:

- update `docs/engine-runtime.md` so board-note tool guidance and prompt/runtime channel explanation use `board_update_note` as the active mutation tool
- update `docs/architecture.md` where it describes board tools or the board tool family so the active mutation surface matches implementation
- update any other durable prompt or runtime guidance document that still names `board_move_note` or `board_update_note_result` as active tools
- do not leave old tool names in current-behavior documentation except in clearly marked migration or historical notes

This follow-through is required for implementation readiness because prompt/runtime docs are part of how Themion teaches the tool surface.

### 5. Keep combined note updates logically atomic

One main reason to merge these updates is to express one logical state change in one call.

Required behavior:

- when both `column` and `result_text` are present, the runtime should apply them as one note update operation when practical
- if implementation cannot make the underlying storage write atomic, the public tool contract must still behave as one logical operation and must not leave silent inconsistent success reporting
- if the combined update fails, the tool must return a clear error instead of partial silent success
- transcript or runtime event output should reflect one canonical update action, not two unrelated synthetic actions, when that can be done without broad refactoring

This PRD does not require a database-engine change if the current board store can already support a safe combined update in the existing transaction model.

### 6. Keep runtime ownership and storage layering unchanged

This is a tool-contract consolidation, not a board-runtime redesign.

Required behavior:

- board-note mutation logic stays in the existing runtime/storage ownership layer
- TUI and Web UI only render resulting board events or statuses
- do not duplicate note-update policy in presentation code
- if a shared internal helper is needed, the new tool should use it as the single runtime-owned mutation path

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-core/src/tools.rs` | Add `board_update_note`, remove `board_move_note` and `board_update_note_result` from the active tool surface, and keep the schema compact. |
| `themion-core` tool/prompt guidance | Teach `board_update_note` as the only board-note mutation shape. |
| `themion-cli` board runtime / storage path | Add or reuse one internal note-update helper that can apply `column`, `result_text`, or both through one runtime-owned path. |
| `docs/engine-runtime.md` | Update board-note tool guidance and prompt/runtime wording to use `board_update_note` as current behavior. |
| `docs/architecture.md` | Update board tool-family wording to match the new canonical board surface. |
| `docs/README.md` | Track this PRD and later update its status/version when implemented. |
| Related historical PRDs or focused board-note docs | Add a short implementation note only where needed so durable docs do not claim the split mutation pair is still current behavior. |

## Edge Cases

- caller sends only `note_id` with no `column` and no `result_text` → verify: the tool fails with a clear validation error.
- caller sends only `column` → verify: column changes and existing result text stays unchanged.
- caller sends only `result_text` → verify: result text changes and column stays unchanged.
- caller sends both `column` and `result_text` → verify: both changes apply as one logical update.
- caller uses invalid `column` value → verify: the tool returns a clear invalid-column error.
- an old caller uses removed `board_move_note` → verify: normal unknown-tool behavior occurs.
- an old caller uses removed `board_update_note_result` → verify: normal unknown-tool behavior occurs.
- board runtime emits note-update transcript/status events → verify: combined updates remain understandable and do not double-report misleadingly.
- a note already has result text and the caller updates only column → verify: existing result text is preserved.
- a note already has a column and the caller updates only result text → verify: existing column is preserved.
- a durable prompt/runtime guide still names a removed tool → verify: the same implementation slice updates that guide before the work is considered done.

## Migration

This is a breaking tool-surface change by product direction, but this PRD targets `v0.73.0`.

Migration rules:

- callers that previously used `board_move_note({ note_id, column })` must switch to `board_update_note({ note_id, column })`
- callers that previously used `board_update_note_result({ note_id, result_text })` must switch to `board_update_note({ note_id, result_text })`
- callers that previously did both in sequence should use one `board_update_note({ note_id, column, result_text })` call
- after implementation, the old tool names are no longer supported

No database migration is required unless implementation discovers the current storage path cannot support safe combined note updates without a schema change.

Minor-version scope is required for this PRD by product decision, even though it removes two old model-facing tools and replaces them with one surface.

## Testing

- inspect generated tool schemas → verify: `board_update_note` exists with required `note_id` and optional `column` / `result_text` fields.
- inspect generated tool schemas → verify: `board_move_note` and `board_update_note_result` are absent.
- call `board_update_note` with only `column` → verify: the note moves correctly.
- call `board_update_note` with only `result_text` → verify: result text updates correctly.
- call `board_update_note` with both `column` and `result_text` → verify: one call updates both fields correctly.
- call `board_update_note` with neither optional field → verify: the tool returns a clear validation error.
- call `board_update_note` with an invalid column → verify: the tool returns a clear invalid-column error.
- attempt old tool names after implementation → verify: unknown-tool behavior occurs.
- inspect transcript/runtime board events for a combined update → verify: the result is understandable and not misleadingly duplicated.
- inspect `docs/engine-runtime.md` and `docs/architecture.md` after implementation → verify: both describe `board_update_note` as the active board mutation tool and no longer teach the removed split tools as current behavior.
- compare prompt/tool definitions before and after the change → verify: the board-mutation tool surface is smaller and simpler.
- run `cargo check -p themion-core` → verify: core tool changes compile.
- run `cargo check -p themion-core --all-features` → verify: all-feature core build compiles.
- run `cargo check -p themion-cli` → verify: default CLI build compiles.
- run `cargo check -p themion-cli --all-features` → verify: all-feature CLI build compiles.

## Implementation checklist

- [x] add `board_update_note` to the model-facing tool surface
- [x] validate that at least one of `column` or `result_text` is provided
- [ ] route note updates through one runtime-owned internal update path
- [x] remove `board_move_note` and `board_update_note_result` from the active tool surface
- [ ] update prompt and tool guidance so `board_update_note` is the only board-note mutation path
- [ ] update `docs/engine-runtime.md` to use `board_update_note` as current behavior
- [ ] update `docs/architecture.md` so the board tool family matches the new canonical surface
- [ ] update any other durable prompt/runtime guide that still teaches removed board mutation tools as current behavior
- [ ] update board-note docs so create/list/read/update is the canonical surface
- [ ] add focused tests for column-only, result-only, combined update, empty-update rejection, invalid column, old-tool removal behavior, and prompt-surface simplification
- [x] update PRD/docs status notes after implementation lands
