# PRD-031: Rename Local Notes Tools from `stylos_` to `board_`

- **Status:** Implemented
- **Version:** v0.17.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21

## Summary

- Rename the durable notes tool family from `stylos_*` to `board_*`.
- Keep Stylos as the transport and discovery layer, but treat notes-board operations as mostly local board work once the note already exists in the local runtime.
- Make tool names reflect the user-facing concept of a board rather than the underlying Stylos integration.
- Preserve current note semantics, storage, columns, and UUID/slug identity behavior.
- Remove the old `stylos_*` note tool names rather than keeping backward-compatible aliases.

## Goals

- Rename note and board-oriented tool names so they describe the local board concept rather than the Stylos transport layer.
- Make the durable notes feature easier for the model to understand as local board manipulation after note intake.
- Preserve the existing behavior of note creation, listing, reading, moving, and result updates while changing the public tool names.
- Keep the change narrowly scoped to tool naming and prompt/docs wording.
- Clarify the architectural distinction between Stylos transport/queryables and local board operations.

## Non-goals

- No redesign of the SQLite notes schema, UUID `note_id`, or `note_slug` behavior.
- No change to board columns `todo`, `in_progress`, and `done`.
- No change to idle-time injection priority or note delivery semantics.
- No removal of Stylos transport/queryable names such as `stylos_query_*`, `stylos_request_task`, or `stylos_request_talk`.
- No broad rename of every Stylos-related symbol in the codebase.
- No introduction of a new non-Stylos distributed transport for notes in this PRD.

## Background & Motivation

### Current state

The original durable note and board tools were exposed in `themion-core` with `stylos_` prefixes:

- `stylos_create_note`
- `stylos_list_notes`
- `stylos_read_note`
- `stylos_move_note`
- `stylos_update_note_result`

At the architecture level, the transport and receiver-side request path still live in the Stylos integration, and the receiver-side query surface still uses a Stylos queryable for note intake. However, once the note is accepted and persisted locally, most of the work becomes local board behavior:

- listing notes from SQLite
- reading one note
- moving a note between board columns
- attaching or updating result text
- selecting pending notes for local idle-time injection

This means the original tool names over-emphasized the transport origin even when the actual operation is mostly local board state manipulation.

### Why `board_` is a better prefix for these tools

The user’s reasoning is that these tools are “almost working locally.” That matches the current design: after initial intake, the tools operate on a local durable board, not on remote Stylos transport mechanics.

Using a `board_` prefix improves conceptual accuracy:

- it better matches the durable board metaphor introduced by PRD-029
- it makes tool intent clearer to the model
- it separates transport naming from work-item manipulation naming
- it reduces the impression that every note read or move is a network action

This is especially important because prompt/tool naming strongly influences how the model reasons about available actions.

**Alternative considered:** keep the existing `stylos_` names because notes entered through Stylos first. Rejected: that naming is accurate for intake transport but less accurate for the steady-state local board operations the user and model perform most often.

### Why this should be a targeted rename rather than a broader subsystem rename

The current repo still has real Stylos-specific behavior that should remain clearly named as Stylos:

- mesh discovery queryables
- node queries
- talk requests
- task requests
- status queries
- receiver-side intake queryables for remote requests

Renaming all of that to `board_` would blur the distinction between distributed transport and local work-item management.

The right scope is therefore narrower: rename the durable note tool family used by the model for board operations, while keeping Stylos transport names where they are still semantically accurate.

**Alternative considered:** rename the entire notes/queryable/transport path away from Stylos immediately. Rejected: that would expand the scope from tool naming into a larger architectural and protocol rename that this PRD does not need.

## Design

### Rename note tools to `board_*`

The model-visible durable note tools are renamed from `stylos_*` to `board_*`.

Canonical names:

- `board_create_note`
- `board_list_notes`
- `board_read_note`
- `board_move_note`
- `board_update_note_result`

Normative behavior:

- tool definitions presented to the model use the `board_*` names as the canonical interface
- these tools preserve their current request and response shapes aside from wording updates for clarity
- note identity remains canonical UUID `note_id`, with `note_slug` continuing as a companion human-friendly field
- tool behavior remains local durable board behavior backed by SQLite and current runtime logic

This makes the tool namespace reflect the actual abstraction the model is using.

**Alternative considered:** rename only some tools, such as move/read/list, but keep `stylos_create_note`. Rejected: partial renaming would leave one concept split across two prefixes and make the tool surface less coherent.

### Keep Stylos transport and discovery names unchanged

The Stylos transport/query layer remains explicitly Stylos-named where it is still about mesh/network behavior.

Normative behavior:

- keep existing transport-oriented tools and query concepts such as `stylos_query_agents_alive`, `stylos_query_status`, `stylos_request_talk`, `stylos_request_task`, and `stylos_query_task_result`
- keep receiver-side remote note intake queryables documented as part of the Stylos query surface unless a later PRD deliberately renames protocol/queryable paths too
- docs distinguish clearly between Stylos transport/intake and local board manipulation

This preserves a clean boundary: Stylos for transport and discovery, board for local work-item operations.

**Alternative considered:** use `board_` for both local tools and Stylos mesh/query operations. Rejected: that would hide the network boundary and make remote versus local behavior less legible.

### Remove old `stylos_*` note tool names

The implementation removes the old `stylos_*` note tool names instead of keeping compatibility aliases.

Normative behavior:

- the implementation exposes only the `board_*` names for durable note tool operations
- docs and canonical examples use `board_*`
- old `stylos_*` note tool names are not dispatched by the runtime

This keeps the public tool surface clear and avoids teaching the model two names for the same concept.

**Alternative considered:** keep compatibility aliases for a transition period. Rejected: the user explicitly requested no backward compatibility, and keeping both names would dilute the naming cleanup.

### Update prompt and documentation language to describe local board work more accurately

The documentation and nearby runtime wording align with the new tool naming.

Normative behavior:

- docs describe note creation, listing, reading, movement, and result updates as board operations
- where the runtime previously referred to Stylos note tools specifically, wording is updated to explain that Stylos handles transport/intake while board tools handle durable local board state
- examples and tool lists prefer the canonical `board_*` names

This avoids a mismatch where the code says `board_*` but the docs still teach the older transport-centric framing.

**Alternative considered:** rename tools only in code and leave docs wording mostly unchanged. Rejected: the goal of the change is conceptual clarity, so docs and prompt-adjacent wording should move with it.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Rename the durable note tool definitions from `stylos_*` to `board_*` and keep tool semantics unchanged. |
| `crates/themion-core/src/tools.rs` | Update tool descriptions so they describe local board operations rather than implying remote Stylos behavior for every action. |
| `docs/architecture.md` | Update the documented tool list and describe the boundary between Stylos transport/queryables and board-local note operations. |
| `docs/engine-runtime.md` | Update runtime documentation so durable note tools are documented as `board_*` operations backed by local SQLite board state. |
| `docs/README.md` | Add this PRD to the PRD index and mark it implemented. |
| `crates/themion-core/Cargo.toml` | Bump crate version to `0.17.0`. |
| `crates/themion-cli/Cargo.toml` | Bump crate version to `0.17.0`. |

## Edge Cases

- an old prompt or prior instruction tells the model to call `stylos_list_notes` → verify: the runtime rejects the old name cleanly, while current docs and tool definitions expose only `board_*` names.
- the model needs to create a remote note for another instance but then inspect its own local board → verify: transport/intake naming remains Stylos-specific where remote semantics matter, while local board tools remain `board_*`.
- a future non-Stylos note intake path is added → verify: `board_*` still fits naturally because it names the durable board abstraction rather than one transport.
- users inspect logs or docs while the implementation still contains internal `Stylos` type names → verify: public tool naming and docs remain conceptually clear even if some internal Rust symbols stay unchanged.

## Migration

This PRD lands as a direct rename of the model-visible durable note tools.

Implemented migration shape:

- expose `board_*` names as the only public interface for durable note tools
- update docs, examples, and prompt-adjacent wording to use `board_*`
- remove old `stylos_*` note tool names from the runtime tool dispatch surface

This keeps the public interface unambiguous and aligned with the board abstraction.

## Testing

- expose the updated tool definitions to the model → verify: the durable note tools appear under canonical `board_*` names.
- invoke `board_create_note`, `board_list_notes`, `board_read_note`, `board_move_note`, and `board_update_note_result` through the normal tool path → verify: each behaves the same as the previous note tool family.
- invoke one old `stylos_*` note tool name after the rename → verify: it is no longer accepted by tool dispatch.
- inspect updated docs in `docs/architecture.md` and `docs/engine-runtime.md` → verify: they describe Stylos as transport/intake and `board_*` as local durable board operations.
- run `cargo check -p themion-core -p themion-cli --features stylos` after implementation → verify: renamed tool definitions compile cleanly.

## Implementation checklist

- [x] rename canonical durable note tool names from `stylos_*` to `board_*`
- [x] keep note tool request/response semantics unchanged unless small wording clarifications are needed
- [x] update tool descriptions to emphasize local board operations
- [x] remove old `stylos_*` note tool names instead of keeping aliases
- [x] update architecture docs to reflect the new canonical tool names and boundary language
- [x] update engine runtime docs to describe `board_*` as the local durable board tool family
- [x] update `docs/README.md` with the new PRD entry and implemented status
