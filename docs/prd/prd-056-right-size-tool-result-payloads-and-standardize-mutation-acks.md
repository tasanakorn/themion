# PRD-056: Right-Size Tool Result Payloads and Standardize Mutation Acknowledgements

- **Status:** Implemented
- **Version:** v0.34.3
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-26

## Summary

- Some Themion tools currently return much more data than the caller usually needs, especially simple mutation tools such as `board_move_note` and `board_update_note_result`.
- The current pattern is inconsistent: some tools return a small acknowledgement, some return a compact summary object, and some return a full updated record or a large list even when the operation was a narrow state change.
- Standardize tool-result design around a simple rule: mutation tools should default to compact acknowledgement payloads, while explicit read/query tools should remain the place for full record or collection retrieval.
- Keep read tools such as `board_read_note`, `board_list_notes`, `memory_get_node`, and `memory_open_graph` as the canonical detailed-inspection surface.
- Do not add compatibility flags mechanically; use them only where a detailed mutation return has a realistic caller need.
- This PRD is about result-shape discipline and payload sizing, not about changing board semantics, memory semantics, or filesystem/shell bounds.

## Goals

- Reduce unnecessary transcript and storage bloat caused by verbose tool results for narrow mutation operations.
- Make tool-result contracts more predictable: writes acknowledge, reads inspect.
- Keep the smallest useful response as the default for state-changing tools.
- Identify other tools with similar over-detailed or inconsistent result contracts and define a repository-wide design rule rather than patching only `board_move_note`.
- Preserve machine readability by returning structured JSON acknowledgements instead of vague plain-text strings.
- Keep explicit detailed-inspection tools available so agents can still fetch authoritative full state when they actually need it.

## Non-goals

- No redesign of board-note storage, note injection, or done-mention semantics.
- No change to workflow behavior or provider behavior.
- No removal of detailed read/query tools.
- No attempt in this PRD to invent pagination or streaming for every large read tool.
- No requirement to shrink intentionally detailed diagnostic tools such as `system_inspect_local` when their purpose is explicitly inspection.
- No blanket requirement to preserve current detailed mutation responses through compatibility flags when those responses are not meaningfully useful.

## Background & Motivation

### Current state

Themion's tool surface has grown incrementally, and result-shape conventions are currently mixed.

Current examples already show three different patterns:

- compact structured acknowledgement, such as `time_sleep -> {"slept_ms": ...}`
- small custom/plain acknowledgement, such as `fs_write_file -> "Written"`
- full updated record return, such as `board_move_note`, `board_update_note_result`, and `board_create_note` returning the entire note JSON

The board tools are the clearest example of payload mismatch. `board_move_note` takes only `note_id` and `column`, but currently returns the entire updated note object, including fields unrelated to the move such as `body`, sender/target metadata, and potentially long `result_text`. In practice this means a tiny request can produce a result much larger than the semantic change itself.

This is not just theoretical. In recent observed usage, `board_move_note` to `done` returned a much larger payload mainly because the note already carried long `result_text`, even though the move operation itself only changed the column and timestamps.

The same implementation pattern appears in the database helpers: mutation methods such as `create_board_note`, `move_board_note`, and `update_board_note_result` perform the update and then re-read the whole row through `get_board_note(...)`. That is convenient for callers, but it makes the default tool contract read-heavy even for narrow writes.

At the same time, Themion already has explicit read tools that are better suited for full inspection:

- `board_read_note`
- `board_list_notes`
- `memory_get_node`
- `memory_search`
- `memory_open_graph`
- `history_recall`
- `history_search`
- `system_inspect_local`

That separation suggests a cleaner contract: mutation tools should acknowledge the mutation, and read tools should retrieve detailed state.

**Alternative considered:** leave result shapes ad hoc and optimize only the board tools that look large in one transcript. Rejected: this is a design-consistency issue across the tool surface, not just one large response.

### Tool-result design categories observed today

A docs and source audit shows that current Themion tools roughly fall into these categories:

1. **Explicit read/query tools**
   - naturally return records, lists, search hits, graphs, or inspection snapshots
   - examples: `fs_read_file`, `board_read_note`, `board_list_notes`, `memory_get_node`, `memory_search`, `memory_open_graph`, `history_recall`, `history_search`, `system_inspect_local`

2. **Mutation tools with compact acknowledgement already**
   - examples: `time_sleep`, `memory_unlink_nodes`, `memory_delete_node`

3. **Mutation tools with inconsistent or overly detailed results**
   - `board_create_note` returns a full note record
   - `board_move_note` returns a full updated note record
   - `board_update_note_result` returns a full updated note record
   - `memory_create_node`, `memory_update_node`, and `memory_link_nodes` return full created/updated objects rather than compact mutation acknowledgements
   - `fs_write_file` returns plain text rather than a structured acknowledgement

Not every detailed mutation result is equally problematic. Creating a new entity can reasonably return key identifiers needed for later reference. But even there, returning the entire stored object by default is often more than the caller needs.

**Alternative considered:** standardize every tool to full-record returns for consistency. Rejected: that would optimize for convenience at the cost of transcript volume and would blur the read/write contract further.

## Design

### Establish a repository-wide default: writes acknowledge, reads inspect

Themion should adopt a simple default rule for tool result design:

- read/query tools may return detailed records or collections because inspection is their primary purpose
- mutation tools should default to compact structured acknowledgements that confirm what changed and how the caller can inspect more if needed
- a mutation tool may include stable identifiers and the specific changed fields in its default response
- mutation tools should not, by default, echo large unchanged fields such as note bodies, accumulated result text, graph neighborhoods, or other full stored documents

Recommended default acknowledgement shape:

```json
{
  "ok": true,
  "entity": "board_note",
  "operation": "move",
  "note_id": "...",
  "note_slug": "...",
  "changed": {
    "column": "done",
    "updated_at_ms": 1777134448319
  }
}
```

For not-found cases, prefer a similarly structured machine-readable result rather than forcing error-string parsing:

```json
{
  "ok": false,
  "entity": "board_note",
  "operation": "move",
  "found": false,
  "note_id": "..."
}
```

**Alternative considered:** use plain-text acknowledgements for all writes to minimize bytes further. Rejected: structured JSON remains easier for models and future tooling to consume reliably.

### Use compatibility flags only where they are justified

Compatibility flags should be exception-based rather than a blanket rule.

Normative direction:

- do not add `return_full`-style flags mechanically to every touched mutation tool
- use an opt-in detailed return only when there is a realistic caller need for a write operation to also act as an immediate read
- when a tool is a narrow state transition and its full stored object is rarely useful, prefer an ack-only default contract without a verbose-return option
- keep explicit read tools as the supported path for detailed follow-up inspection

This keeps the tool surface simpler and avoids preserving poor defaults just because they already exist.

**Alternative considered:** give every mutation tool a uniform `return_full` flag for consistency. Rejected: that preserves unnecessary complexity for tools such as `board_move_note` where the full object is not a strong use case.

### Classify mutation tools by result style, not by one blanket migration rule

Mutation tools should be reviewed in three practical groups:

1. **Ack-only by design**
   - narrow state transitions where full stored state is not a meaningful default need
   - examples: `board_move_note`, `memory_unlink_nodes`, `memory_delete_node`, `time_sleep`, `fs_write_file`

2. **Compact summary by default**
   - writes that create or update state and should return identifiers plus a small stable summary
   - examples: `board_update_note_result`, `memory_update_node`

3. **Compact creation acknowledgement, with optional detail only if justified**
   - create/link operations where the caller may reasonably need identifiers from the created entity, but still usually does not need the entire stored object
   - examples: `board_create_note`, `memory_create_node`, `memory_link_nodes`

The burden of proof should be on keeping a detailed mutation return, not on shrinking it.

**Alternative considered:** classify only the board tools and leave the rest for later. Rejected: the PRD should set a broader design rule even if implementation lands in phases.

### Right-size the board tools first

The board tools are the highest-priority cleanup because they are common coordination primitives and currently repeat large note payloads across simple lifecycle moves.

Normative direction:

- `board_move_note` should become ack-only by design, returning `note_id`, `note_slug` when available, effective `column`, and `updated_at_ms`, plus structured not-found results when appropriate
- `board_move_note` should not add a `return_full` compatibility flag unless implementation finds a concrete existing caller need that is stronger than the simplicity benefit of ack-only results
- `board_update_note_result` should default to a compact acknowledgement containing `note_id`, `note_slug` when available, whether `result_text` is now present, and `updated_at_ms`
- `board_update_note_result` may support an opt-in detailed return only if implementation uncovers a concrete compatibility need
- `board_create_note` should default to a compact creation acknowledgement containing `note_id`, `note_slug`, `column`, `note_kind`, target identifiers, and `created_at_ms`
- `board_create_note` may support opt-in detail if the create path has a realistic caller need for immediate full-note inspection
- `board_read_note` remains the canonical tool for retrieving the full note
- `board_list_notes` remains the canonical tool for broader board inspection

Because the board DB helpers currently re-read full rows after writes, implementation may still fetch the row internally in order to populate compact acknowledgements. The PRD requirement is about the tool contract, not necessarily eliminating every internal re-read in the first slice.

**Alternative considered:** keep `board_create_note` full because callers often need the note identifier. Rejected in part: creation acknowledgements should absolutely include identifiers, but they still do not need to include the full note body and long result text by default.

### Apply the same rule to memory mutation tools where practical

The memory tools currently mix explicit read tools with creation/update mutations that return full node or edge objects. That is workable, but it keeps the same ambiguity about whether a write is also a read.

Normative direction:

- `memory_create_node` should default to a compact acknowledgement containing `node_id`, `project_dir`, `node_type`, `title`, and timestamps
- `memory_update_node` should default to a compact acknowledgement describing the updated node identifier and changed metadata shape, not the entire node payload
- `memory_link_nodes` should default to a compact acknowledgement containing `edge_id`, endpoints, and relation type
- use optional detailed returns only if implementation finds a concrete use case that is better served by an opt-in verbose response than by a follow-up `memory_get_node`
- `memory_get_node` and `memory_open_graph` remain the explicit detailed-inspection tools

This slice should remain pragmatic: if some memory mutations are already naturally compact, the implementation can avoid over-engineering changed-field diffing and instead return a small stable summary.

**Alternative considered:** leave memory tools untouched because their objects are often smaller than notes. Rejected: the underlying design issue is the same even if the payload sizes differ.

### Normalize acknowledgement style for simple write tools

Some write tools are already compact but stylistically inconsistent.

Normative direction:

- `fs_write_file` should return structured JSON such as `{ "ok": true, "path": "...", "written_bytes": N, "mode": "raw" }` rather than the plain string `Written`
- keep `time_sleep` as structured JSON; it already fits the intended style
- preserve existing bounded behavior for `shell_run_command` and `fs_read_file`, which are explicit read/inspection operations rather than mutations

This is less about size than about consistent machine-readable tool contracts.

**Alternative considered:** leave `fs_write_file` unchanged because it is already small. Rejected: size is not the only problem; consistency and machine readability also matter.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Introduce a standard compact acknowledgement shape for selected mutation tools, classifying which tools are ack-only, compact-summary, or optional-detail exceptions. |
| `crates/themion-core/src/tools.rs` | Update tool JSON schemas for `board_create_note`, `board_move_note`, `board_update_note_result`, `memory_create_node`, `memory_update_node`, `memory_link_nodes`, and `fs_write_file` to document the new default result behavior and any optional detailed-return flag only where justified. |
| `crates/themion-core/src/db.rs` | Keep current mutation helpers or add small helper paths as needed so tool handlers can build compact acknowledgements without forcing callers to receive full stored objects. |
| `crates/themion-core/src/agent.rs` | No harness-loop redesign required, but tool-result persistence should naturally benefit from smaller default mutation payloads. |
| `crates/themion-cli/src/tui.rs` | Update any user-facing assumptions or debug formatting that implicitly depend on large mutation-tool payloads. |
| `docs/engine-runtime.md` | Document the repository-wide tool-result rule: read tools inspect, mutation tools acknowledge by default, and optional detailed returns are exception-based rather than automatic. |
| `docs/architecture.md` | Update the tools section if needed so result-shape discipline is reflected at the architecture level. |
| `docs/README.md` | Add this PRD to the PRD index table. |

## Edge Cases

- `board_move_note` targets a missing note → verify: the tool returns a compact structured not-found acknowledgement instead of an oversized empty record or ambiguous plain text.
- a note has very long `result_text` or `body` → verify: default `board_move_note` and `board_update_note_result` results no longer echo those fields unless a tool explicitly supports and is called with an opt-in detailed return.
- `board_create_note` uses Stylos-mediated creation → verify: the returned acknowledgement still includes the stable note identifiers needed for later tracking even when the underlying creation path is remote/intake-based.
- `memory_update_node` changes hashtags or metadata → verify: the compact acknowledgement remains stable and machine-readable without requiring the full node body.
- `memory_create_node` or `memory_link_nodes` retain an optional detailed return → verify: the default remains compact and the detailed path is clearly documented as exceptional.
- `fs_write_file` writes binary content in base64 mode → verify: the acknowledgement reports success and useful bounded metadata without echoing file content.
- older prompts still issue a mutation followed immediately by a read → verify: behavior remains correct, with only payload size reduced for the write step.

## Migration

This PRD treats tool-result reshaping as compatibility-sensitive, but not every existing verbose write result deserves a compatibility flag.

Expected rollout shape:

- first define the compact acknowledgement schema and classify touched mutation tools as ack-only, compact-summary, or optional-detail exception
- implement the new default response shape for the highest-value mutation tools
- use opt-in detailed returns only for the subset of tools where implementation finds a concrete compatibility need
- update docs and any targeted tests to stop assuming that a write operation always returns the full stored object
- prefer removing unnecessary verbose-return paths entirely for narrow state-transition tools such as `board_move_note`

This is a tool-contract migration, not a database migration.

## Testing

- call `board_move_note` on an existing note → verify: the result is compact JSON containing the identifier, operation result, and changed state, without full `body` or long `result_text`.
- call `board_move_note` on a missing note → verify: the result is a compact structured not-found acknowledgement.
- call `board_update_note_result` on a note with a long result body → verify: the default acknowledgement does not echo the full `result_text`.
- call `board_create_note` through local DB creation and through Stylos-mediated creation → verify: both paths return the same compact acknowledgement contract by default.
- call `memory_create_node`, `memory_update_node`, and `memory_link_nodes` without any detailed-return flag → verify: they return compact structured acknowledgements rather than full stored objects.
- call any mutation tool that keeps an explicitly justified detailed-return flag → verify: the default remains compact and the opt-in path still returns the documented richer payload.
- call `fs_write_file` in `raw` and `base64` modes → verify: the tool returns structured success JSON with useful metadata.
- inspect persisted tool result rows after several board and memory mutations → verify: transcript/tool-result storage is materially smaller for default mutation operations.
- run `cargo check -p themion-core -p themion-cli` after the tool-contract changes → verify: default builds compile cleanly.
- run `cargo check -p themion-cli --features stylos` after changing board tool contracts → verify: Stylos-enabled builds remain feature-safe.

## Implementation checklist

- [x] audit all mutation tools and classify each as ack-only, compact-summary, or optional-detail exception
- [x] introduce a standard compact acknowledgement JSON pattern in `themion-core` tool handlers
- [x] change `board_move_note` to an ack-only result by default
- [x] change board mutation tools to default to compact acknowledgements
- [x] change selected memory mutation tools to default to compact acknowledgements
- [x] add optional detailed-return flags only where a concrete caller need justifies them
- [x] normalize `fs_write_file` to structured JSON acknowledgement
- [x] update relevant docs in `docs/engine-runtime.md` and `docs/architecture.md`
- [x] add or update targeted tests covering default compact results and any justified opt-in detailed returns
- [x] update `docs/README.md` with the new PRD entry


## Implementation notes

Implemented in v0.34.3.

What landed:

- `board_create_note`, `board_move_note`, and `board_update_note_result` now return compact structured acknowledgements rather than full note payloads
- `board_move_note` now uses an ack-only result shape for both success and not-found cases
- `memory_create_node`, `memory_update_node`, and `memory_link_nodes` now return compact structured acknowledgements rather than full created/updated objects
- `fs_write_file` now returns structured JSON success metadata rather than the plain string `Written`
- `docs/engine-runtime.md` and `docs/architecture.md` now document the read-vs-write tool result contract more explicitly

Intentional limit of this slice:

- no optional verbose-return flags were added because the touched tools did not reveal a strong enough concrete need to justify them in this implementation
