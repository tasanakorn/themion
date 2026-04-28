# PRD-063: Prefer `note_slug` in `board_move_note` User-Facing Chat Labels

- **Status:** Implemented
- **Version:** v0.40.1
- **Scope:** `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-27

## Summary

- Themion already prefers `note_slug` over raw UUID `note_id` in several user-facing board note chat events, but the current `board_move_note` tool-call label still shows the UUID.
- This creates an inconsistent chat experience where a user sees `board_move_note b5a04b4f-4979-4839-81bd-28992854945f -> done` instead of the more readable note slug.
- Themion should keep `board_move_note` tool input keyed by canonical `note_id`, but its user-visible chat label should prefer `note_slug` when the slug is already present in the tool arguments.
- The implementation target is a small TUI presentation change in the existing tool-call formatter plus aligned docs and feature-on/off validation.

## Goals

- Make `board_move_note` chat labels use the human-friendly `note_slug` instead of a raw UUID when that slug is already available in tool arguments.
- Keep user-facing board note event presentation consistent with the repository's broader note-slug-first direction.
- Preserve canonical `note_id` usage for the actual tool contract and database mutation path.
- Land this as a small, implementation-ready presentation fix without widening the board tool contract.

## Non-goals

- No change to the `board_move_note` input schema; `note_id` remains the required mutation identifier.
- No change to database schema, note lookup keys, or SQLite storage layout.
- No requirement in this slice to add DB lookups inside the TUI formatter just to recover a slug that was not already passed.
- No requirement in this slice to change every board tool to accept `note_slug` as an input selector.
- No broader board-tool redesign beyond this user-facing label consistency fix.

## Background & Motivation

### Current state

Themion already has an established direction of preferring human-friendly note identifiers in user-visible chat and event surfaces:

- the durable note schema includes both canonical `note_id` and human-friendly `note_slug`
- note injection and done-mention chat events already prefer `note_slug`
- `stylos_note_display_identifier` in `crates/themion-cli/src/tui.rs` prefers `note_slug` and falls back to `note_id`
- `docs/architecture.md` and `docs/engine-runtime.md` already describe note-slug-first presentation in several board-related user-facing flows

But one user-visible path remains inconsistent: the TUI tool-call label formatting for `board_move_note` in `split_tool_call_detail` currently renders:

- `board_move_note <note_id> -> <column>`

That means a routine action such as moving a note to `done` shows the raw UUID in chat even though the same note is usually referred to elsewhere by `note_slug`.

### Why this matters

The note slug exists specifically to make note identity readable in human-facing contexts. Showing the UUID in the `board_move_note` chat label:

- makes the transcript harder to scan
- is inconsistent with other board-related chat surfaces
- exposes a machine identifier where a stable human-facing identifier already exists

This is a small presentation issue, but it is exactly the kind of inconsistency users notice because board actions are visible in the transcript.

**Alternative considered:** leave `board_move_note` unchanged because the underlying tool contract still requires `note_id`. Rejected: machine-facing identity and human-facing presentation are already intentionally separated elsewhere in the board/note system.

## Design

### Design principles

- Keep canonical mutation identity and user-facing display identity separate.
- Prefer the most human-readable identifier in chat-facing tool labels.
- Make the smallest change that brings `board_move_note` into line with existing note-slug-first presentation.
- Prefer data already present in the tool-call arguments over adding new runtime lookups.
- Preserve safe fallback behavior when a slug is unavailable.

### 1. Update the existing `board_move_note` formatter in `split_tool_call_detail`

The user-visible tool-call label for `board_move_note` should be changed in `crates/themion-cli/src/tui.rs`, inside the existing `split_tool_call_detail` formatter.

Current behavior is:

- `board_move_note <note_id> -> <column>`

Target behavior should be:

- `board_move_note <note_slug> -> <column>` when `note_slug` is present in the tool-call arguments
- `board_move_note <note_id> -> <column>` only as a fallback when `note_slug` is absent

Recommended implementation shape:

- add a small helper local to the formatter path, or inline the logic, that reads `args["note_slug"]` first and falls back to `args["note_id"]`
- continue to run the chosen value through the existing `center_trim(...)` behavior with `TOOL_DETAIL_MAX_CHARS`
- keep the rest of the `board_move_note` label structure unchanged

This keeps the change local to the presentation path that already formats tool labels.

**Alternative considered:** create a broader reusable board-note identifier formatter and refactor all board tools in the same change. Rejected for this slice: that is larger than needed for a one-label consistency fix.

### 2. Pass `note_slug` in the `board_move_note` tool-call arguments where the caller already knows it

Because the formatter only sees the serialized tool-call arguments, the implementation should ensure that the `board_move_note` invocation path includes `note_slug` when the calling code already has the note record.

Implementation target:

- update the relevant `board_move_note` call site(s) in `crates/themion-cli/src/tui.rs` where a local `BoardNote` is already available and `note.note_slug` is known
- include that `note_slug` in the arguments passed into the visible tool-call formatting path
- do not change the actual `board_move_note` tool schema in `themion-core`; extra presentation-only fields in the local invocation JSON are acceptable as long as the canonical mutation still uses `note_id`

This is the simplest path because the TUI already has the note record in some board-driven flows and can expose the slug without an extra DB read.

Normative behavior:

- mutation still targets the canonical `note_id`
- chat-facing label prefers `note_slug`
- fallback remains `note_id` if the call path did not provide a slug

**Alternative considered:** perform a DB lookup during label formatting whenever only `note_id` is present. Rejected: that adds avoidable presentation-layer coupling and runtime work for a fix that can be satisfied by passing existing data through.

### 3. Keep docs aligned with the actual landed behavior

The relevant docs should reflect that user-facing `board_move_note` chat labels prefer `note_slug` while the mutation contract still uses `note_id`.

This should be described briefly in:

- `docs/engine-runtime.md` in the tools or board-note behavior section
- `docs/architecture.md` if its board-note presentation wording would otherwise stay stale
- `docs/README.md` and this PRD once the change lands

The docs should not imply a tool-schema change or note-slug lookup-based mutation path.

**Alternative considered:** skip doc updates because the change is small. Rejected: this repository expects docs to stay aligned with user-visible behavior changes, even small ones.

### 4. Acceptance target for the first implementation

This PRD should be considered implemented when all of the following are true:

- the `board_move_note` branch in `crates/themion-cli/src/tui.rs::split_tool_call_detail` prefers `note_slug` over `note_id`
- at least the local `board_move_note` invocation path that already has a `BoardNote` record passes `note_slug` through the visible tool-call arguments
- the label still falls back to `note_id` when `note_slug` is absent
- the actual mutation path still uses canonical `note_id`
- docs reflect the landed note-slug-first chat label behavior without overstating a contract change
- `cargo check -p themion-cli` passes
- `cargo check -p themion-cli --features stylos` passes
- `cargo check -p themion-cli --all-features` passes

This acceptance target intentionally keeps the change small and local. It does not require introducing a new cross-module note identifier abstraction.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Update `split_tool_call_detail` so the `board_move_note` label prefers `note_slug` and falls back to `note_id`. |
| `crates/themion-cli/src/tui.rs` board note move call path | When a local `BoardNote` is already available, include `note_slug` in the visible `board_move_note` tool-call arguments without changing the canonical mutation identifier. |
| `docs/engine-runtime.md` | Document that user-facing `board_move_note` chat labels prefer `note_slug` while the mutation contract still uses `note_id`. |
| `docs/architecture.md` | Keep high-level board note presentation docs aligned if needed. |
| `docs/README.md` | Update the PRD entry status/version when the change lands. |

## Edge Cases

- `board_move_note` is invoked with only `note_id` available in the visible tool-call formatting path → verify: the chat label falls back cleanly to `note_id`.
- the note slug is very long → verify: existing center-trim behavior still keeps the label compact.
- the note has been deleted or cannot be resolved for display after invocation → verify: fallback behavior still produces a usable label instead of failing the presentation path.
- other board tools such as `board_read_note` or `board_update_note_result` still show `note_id` → verify: this PRD only changes `board_move_note` unless additional consistency work is explicitly added later.
- a remote or older call path does not yet pass `note_slug` → verify: the label remains usable because fallback-to-`note_id` behavior is preserved.

## Migration

This is a presentation-only behavior change with no schema or storage migration.

Rollout guidance:

- implement the formatter change in the TUI-facing tool-call formatting path
- pass `note_slug` through existing local board-move call paths that already know it
- update docs that describe board note presentation behavior
- keep the tool contract and stored note identity unchanged

## Testing

- trigger a `board_move_note` action for a note with a known slug through a local board path that already has the note record → verify: the visible tool-call label shows `board_move_note <note_slug> -> <column>`.
- trigger a `board_move_note` formatting path where only `note_id` is available → verify: the visible label falls back to `note_id` without error.
- trigger a `board_move_note` action for a long slug → verify: the existing center-trim behavior still keeps the display compact and readable.
- run `cargo check -p themion-cli` after implementation → verify: the touched crate still compiles cleanly.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the Stylos-enabled build still compiles cleanly.
- run `cargo check -p themion-cli --all-features` after implementation → verify: all feature combinations relevant to the touched crate still compile cleanly.

## Implementation checklist

- [x] update the `board_move_note` branch in `crates/themion-cli/src/tui.rs::split_tool_call_detail` to prefer `note_slug` over `note_id`
- [x] keep fallback-to-`note_id` behavior when `note_slug` is unavailable
- [x] update the local `board_move_note` invocation path(s) that already have `note.note_slug` to pass it into the visible tool-call arguments
- [x] keep the actual `board_move_note` mutation contract keyed by `note_id`
- [x] update `docs/engine-runtime.md` and any needed high-level board-presentation docs
- [x] update `docs/README.md` and this PRD status/version when the change lands
- [x] run `cargo check -p themion-cli`
- [x] run `cargo check -p themion-cli --features stylos`
- [x] run `cargo check -p themion-cli --all-features`
