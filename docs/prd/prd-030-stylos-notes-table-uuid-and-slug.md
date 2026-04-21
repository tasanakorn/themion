# PRD-030: Stylos Notes Table Identifier Hardening and Human-Friendly Slugs

- **Status:** Implemented
- **Version:** v0.16.1
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-21

## Summary

- Improve the durable Stylos notes schema so note identity is both machine-safe and human-friendly.
- Require `note_id` to be a UUID value rather than an arbitrary freeform string.
- Add a separate `note_slug` field for human-friendly references and display.
- Enforce uniqueness for both `note_id` and `note_slug` at the database level.
- Keep UUID-based `note_id` as the canonical durable identifier used by tools and internal lookups.
- Use `note_slug` as a readable companion identifier rather than a replacement for UUID identity.

## Goals

- Make `note_id` format explicit and stable by requiring UUID values.
- Add a human-friendly `note_slug` field for easier manual inspection, debugging, and future UI use.
- Ensure `note_id` is unique across all notes.
- Ensure `note_slug` is also unique across all notes.
- Preserve `note_id` as the canonical machine-facing identifier used by tools, persistence helpers, and runtime logic.
- Keep the schema change narrow and compatible with the existing phase-1 notes board model.

## Non-goals

- No redesign of the `todo` / `in_progress` / `done` board model.
- No change to note delivery ordering, injection semantics, or result attachment behavior.
- No introduction of per-agent scoped slugs in this PRD; slug uniqueness is global in the first version.
- No threaded replies, note comments, or note history/event log design in this PRD.
- No requirement to expose slug-based lookup tools immediately if the implementation prefers to keep UUID lookup as the only tool-facing path at first.
- No change to the exact target identity model of `to_instance` plus `to_agent_id`.

## Background & Motivation

### Current state

The current `stylos_notes` table stores note identity in a single `note_id TEXT PRIMARY KEY` field. In practice, this means the schema guarantees uniqueness for one string field, but it does not document or enforce a specific identifier format beyond SQLite text primary-key semantics.

Current schema shape in `crates/themion-core/src/db.rs` includes:

- `note_id TEXT PRIMARY KEY`
- sender and target identity fields
- note body
- `column_name`
- `result_text`
- `injection_state`
- millisecond timestamps

This is sufficient for durable storage, but it leaves two gaps:

- the canonical identifier format is underspecified
- there is no separate human-friendly identifier for operators, debugging, or future UI references

### Why UUID should be the canonical note identifier

A UUID-shaped `note_id` gives the durable notes system a clearer contract.

Benefits:

- globally unique identifiers are straightforward to generate without coordination
- tools and internal APIs can rely on a stable machine-friendly identifier shape
- UUID identity is a better long-term fit for persistence, migration, and cross-process references than arbitrary ad hoc strings
- the distinction between machine identity and human display identity becomes explicit once a separate slug field exists

This is especially useful because notes are now durable work items rather than transient talk messages.

**Alternative considered:** keep `note_id` as arbitrary text and only document that callers should prefer UUIDs. Rejected: the user explicitly asked for `note_id` to be UUID, and schema-level expectations should be reflected directly in the design rather than treated as a soft convention.

### Why add a separate `note_slug`

A UUID is appropriate for machine identity but is inconvenient for humans.

A separate `note_slug` solves a different problem:

- humans can inspect a note row more easily
- logs and future UIs can show something more readable than a raw UUID alone
- a human-friendly identifier can be derived from note content or a short naming policy without weakening the canonical machine identifier

The user explicitly wants `note_slug` to be more human friendly while still keeping uniqueness guarantees.

**Alternative considered:** replace `note_id` with only a slug. Rejected: human-friendly slugs are useful, but they are not as robust as UUIDs for canonical persistence identity.

### Why both fields should be unique

The current schema already gives uniqueness to `note_id` by making it the primary key. Once a second identifier exists, uniqueness must remain explicit for both fields to prevent ambiguous references.

If `note_slug` were not unique, several operational problems would appear:

- human references in logs or future commands could become ambiguous
- migration from one-identifier notes to two-identifier notes would not actually improve reference clarity
- later slug-based lookup or display affordances would become risky

The requested behavior is therefore to ensure uniqueness for each field independently.

**Alternative considered:** make `note_slug` unique only per target agent or per instance. Rejected: that adds complexity and ambiguity in the first slice, while the user explicitly requested uniqueness for each field and did not ask for scoped uniqueness.

## Design

### Keep `note_id` as the canonical identifier but require UUID format

The notes system should continue to use `note_id` as the canonical durable note identifier, but its format should now be explicitly UUID.

Normative behavior:

- every stored note must have a `note_id`
- `note_id` must be generated as a UUID value
- `note_id` remains the canonical identifier used by internal persistence helpers, note injection tracking, and existing note tools unless an individual tool is deliberately expanded later
- the schema should continue to make `note_id` unique
- where practical, creation code should validate or construct `note_id` as UUID rather than accepting arbitrary caller-provided text unchecked

This preserves the current identity role while tightening its format contract.

**Alternative considered:** add a new UUID field and leave the old freeform `note_id` semantics unchanged. Rejected: that would create two competing machine identities and make the model harder to reason about.

### Add `note_slug` as a separate globally unique human-friendly field

The notes table should gain a second identifier field: `note_slug`.

Normative behavior:

- every stored note must have a `note_slug`
- `note_slug` should be human-friendly relative to UUID, suitable for logs, debugging, and future UI display
- `note_slug` must be globally unique across the `stylos_notes` table
- `note_slug` is not a replacement for canonical `note_id`; it is a companion identifier
- the implementation may derive the slug from note content, a short generated token, or another deterministic-readable pattern, but the output must remain unique and stable once stored

This creates a clean separation between machine identity and human-oriented identity.

**Alternative considered:** make `note_slug` optional. Rejected: the requested change is specifically to add a more human-friendly identifier and ensure uniqueness for it, so optional slugs would weaken the value of the new contract.

### Enforce uniqueness for both fields at the SQLite layer

The schema should enforce uniqueness directly in SQLite rather than relying only on application-layer conventions.

Normative behavior:

- `note_id` must remain unique through primary-key or equivalent unique constraint semantics
- `note_slug` must have its own unique constraint or unique index
- schema migration must avoid creating duplicate slugs for existing rows
- if slug generation encounters a collision during note creation, the implementation must retry or otherwise generate a different slug rather than allowing insertion ambiguity

This keeps identifier safety rooted in the durable source of truth.

**Alternative considered:** enforce slug uniqueness only in Rust before insert. Rejected: database-level uniqueness is still needed to protect against races, bugs, and future multi-writer paths.

### Treat `note_slug` as display-friendly and optionally lookup-friendly

The first purpose of `note_slug` is human readability, but the design should not preclude future slug-based lookup.

Normative behavior:

- canonical note reads and updates should continue to work through `note_id`
- logs, debug output, and future board displays may include `note_slug` for readability
- tool expansion to accept `note_slug` in addition to `note_id` is optional for this PRD, but the schema and persistence layer should not block that future path
- if the implementation keeps tool inputs UUID-only at first, documentation should still describe `note_slug` as a human-friendly companion field rather than implying it is already the primary tool lookup key

This avoids conflating schema improvement with an immediate broad API redesign.

**Alternative considered:** immediately switch all tools to slug-based addressing. Rejected: that would create unnecessary migration risk and weaken the canonical role of UUID identity.

### Migrate existing notes to valid UUID and unique slug data

Because the notes table already exists, the implementation needs a migration story for existing rows.

Normative behavior:

- existing note rows must be migrated so they end up with valid UUID `note_id` values and unique `note_slug` values
- if existing `note_id` values are already valid UUIDs, the implementation may preserve them
- if any existing `note_id` is not a valid UUID, the migration should assign a new UUID and update all local references in the touched persistence layer as part of the same migration path
- each existing row must receive a unique slug during migration
- migration behavior must be documented clearly enough that later implementers understand whether old freeform IDs are preserved when valid or replaced unconditionally

This keeps the schema hardening practical for already-implemented notes.

**Alternative considered:** require a destructive reset of the notes table. Rejected: durable notes are specifically meant to persist, so migration should preserve existing data when practical.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/db.rs` | Update `stylos_notes` schema to require UUID-shaped `note_id`, add `note_slug`, add uniqueness enforcement for `note_slug`, and implement migration for existing rows. |
| `crates/themion-core/src/db.rs` | Update note row mapping structs and persistence helpers so `StylosNote` carries both `note_id` and `note_slug`. |
| `crates/themion-core/src/tools.rs` | Decide whether note tools continue to accept only UUID `note_id` or also expose `note_slug` in outputs; at minimum, return `note_slug` in note metadata so the human-friendly field is visible. |
| `crates/themion-cli/src/stylos.rs` | Ensure note creation paths generate or validate UUID `note_id` and provide a unique `note_slug` during durable note creation. |
| `docs/architecture.md` | Document that durable notes now have canonical UUID `note_id` plus globally unique human-friendly `note_slug`. |
| `docs/engine-runtime.md` | Document the runtime-facing identity expectations for durable notes and any migration notes relevant to note persistence. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- an existing note row already has a UUID-shaped `note_id` but no slug → the migration should preserve the valid UUID and generate a unique slug.
- an existing note row has a non-UUID `note_id` → the migration should replace it with a UUID and preserve the note’s other durable state.
- slug generation from note content would produce the same slug for multiple notes → the implementation must detect the collision and generate a distinct unique slug.
- a note body changes after creation → the stored `note_slug` should remain stable unless the implementation explicitly documents slug mutability; phase 1 should prefer stable slugs once assigned.
- a client attempts to create a note with a caller-provided invalid UUID `note_id` in a compatibility path → the request should fail clearly or the implementation should ignore the caller value and generate a valid UUID, but the behavior must be explicit rather than accidental.
- a future UI shows both slug and UUID → the presentation should avoid implying that slug uniqueness is scoped only per agent when the schema makes it globally unique.
- two note creations race and choose the same slug candidate → the unique database constraint must prevent ambiguous insertion, and the application should retry with a different slug.

## Migration

This PRD introduces a schema hardening migration for already-implemented durable notes.

Expected migration shape:

- add `note_slug` to the `stylos_notes` table if it is missing
- backfill `note_slug` for existing rows with globally unique values
- ensure `note_id` values are valid UUIDs according to the chosen migration policy
- add or preserve uniqueness constraints so both `note_id` and `note_slug` remain unique
- update any in-repo docs and note metadata serialization so consumers understand both fields
- keep existing note content, board column, injection state, result text, and timestamps intact across migration

If the implementation changes any persisted identifier values for existing notes, the migration should be coordinated with all local code paths that read or update notes so references stay consistent.

## Testing

- migrate a database containing existing notes with valid UUID `note_id` values and no slug column → verify: each note keeps its UUID and receives a unique `note_slug`.
- migrate a database containing existing notes with non-UUID `note_id` values → verify: each note ends up with a valid UUID `note_id`, a unique `note_slug`, and preserved body/column/state fields.
- create two new notes with similar or identical content → verify: both insert successfully with distinct unique `note_slug` values.
- attempt to insert a note with a duplicate `note_slug` through a lower-level path → verify: SQLite uniqueness enforcement rejects the duplicate.
- create a new note through the normal Stylos note path → verify: returned note metadata includes UUID `note_id` and human-friendly unique `note_slug`.
- read, move, inject, and update result text for a migrated note → verify: existing note operations continue to function using canonical UUID `note_id`.
- run `cargo check -p themion-core -p themion-cli --features stylos` after implementation → verify: the schema, persistence helpers, and note-related runtime code compile cleanly.

## Implementation checklist

- [x] add `note_slug` to the durable notes schema
- [x] require canonical `note_id` values to be UUIDs
- [x] enforce uniqueness for `note_id`
- [x] enforce uniqueness for `note_slug`
- [x] update note persistence structs and row mapping to include `note_slug`
- [x] update note creation paths to generate or validate UUID `note_id`
- [x] add unique slug generation with collision handling
- [x] migrate existing note rows to include unique `note_slug` values
- [x] define and implement migration behavior for existing non-UUID `note_id` values
- [x] return `note_slug` in note metadata where appropriate
- [x] update architecture and runtime docs for the two-identifier note model
- [x] update `docs/README.md` with this PRD entry
