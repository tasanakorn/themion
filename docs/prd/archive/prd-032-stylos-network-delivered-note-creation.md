# PRD-032: Stylos Network-Delivered Note Creation When `stylos` Feature Is Enabled

- **Status:** Implemented
- **Version:** v0.18.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-22

## Summary

- Keep `board_*` as the durable note and board API, but when Stylos is enabled, `board_create_note` should always use the Stylos note-delivery flow.
- Do not short-circuit note creation to a direct local DB insert in Stylos-enabled builds, even when the destination instance is the current instance.
- The receiver-side Stylos note handler should remain the single intake path that creates the note in SQLite.
- This keeps note-created events, triggers, validation, and acknowledgement behavior as uniform as possible.
- Non-Stylos builds should continue to support direct local board creation without introducing a network dependency.
- Keep the current durable board model, note schema, UUID identity, and idle-time injection behavior unchanged after the note is stored.

## Goals

- Keep one note-creation tool surface while making its execution path consistent in Stylos-enabled builds.
- Ensure that when Stylos support is enabled, note creation always follows the same Stylos transport and receiver-intake path regardless of whether the destination instance is local or remote.
- Make the receiver-side Stylos note handler the single canonical creation path in Stylos-enabled builds so event emission, validation, acknowledgement, and future triggers stay unified.
- Make the durable source of truth for a note the receiving instance's local SQLite database rather than the sender's local database.
- Preserve the current `board_*` local board operations once the note has been accepted and stored on the destination instance.
- Keep note creation behavior compatible with current per-instance and per-agent targeting using exact `<hostname>:<pid>` plus `agent_id`.
- Preserve current board semantics after receipt, including initial `todo` placement, durable timestamps in milliseconds, and idle-time injection rules.
- Keep non-Stylos builds functional for purely local board usage.

## Non-goals

- No redesign of the `todo` / `in_progress` / `done` board lifecycle.
- No change to note UUID `note_id`, `note_slug`, result attachment, or injection-state schema.
- No attempt to add distributed transactions, exactly-once delivery, or cross-node consensus.
- No redesign of `talk` or remote task request behavior beyond reusing similar request/reply patterns.
- No broad rename of transport queryables or board tool names.
- No requirement to make note listing or reading transparently aggregate remote boards across the network.
- No new separate remote-note tool unless a later PRD decides that the one-tool approach is insufficient.

## Background & Motivation

### Current state

The current durable notes design already separates Stylos transport from local board operations in documentation and tool naming. `board_*` tools are described as local SQLite-backed board operations, while Stylos remains the mesh transport and intake layer.

Receiver-side note intake already exists in `crates/themion-cli/src/stylos.rs` as a Stylos queryable:

- `stylos/<realm>/themion/instances/<instance>/query/notes/request`

That handler validates the target agent from the current snapshot, creates a UUID `note_id`, and inserts the note into the local notes database on the receiving instance.

At the same time, `themion-core` still exposes `board_create_note` as a local database operation. That is correct for non-Stylos local board work, but it creates two possible create paths once Stylos is enabled: direct local insertion and Stylos-mediated receiver intake.

### Why one Stylos-enabled create path is preferable

The user wants to avoid local short-circuiting when Stylos is enabled so note creation behavior stays as uniform as possible.

That matters because separate local and Stylos-enabled create paths can drift in subtle ways:

- one path may emit different events or no event at all
- one path may bypass future triggers, hooks, or metrics
- one path may apply slightly different validation or acknowledgement semantics
- one path may evolve while the other is forgotten

Using the same receiver-side Stylos intake path even for self-targeted note creation in Stylos-enabled builds keeps behavior centralized and reduces the chance of mismatched semantics.

**Alternative considered:** in Stylos-enabled builds, use a local fast path when `to_instance` is the current instance and Stylos only for remote instances. Rejected: the user explicitly wants to avoid short-circuiting so events and triggers have one behavior as much as possible.

### Why the routing should still be automatic rather than exposed as a second tool

The user wants one note-creation function, not a split between local and remote functions. That means the routing decision should happen inside the implementation.

The intended model-facing behavior is therefore:

- create a note addressed to `to_instance` and `to_agent_id`
- if Stylos is enabled, always send through Stylos note delivery
- if Stylos is not enabled, insert locally only when the destination is local and fail clearly for non-local destinations

This keeps the model-facing abstraction simple while still respecting the real transport boundary and the user's desire for one consistent Stylos-enabled flow.

**Alternative considered:** add a separate `stylos_request_note` tool parallel to `stylos_request_talk`. Rejected: although the underlying logic and parameters are very similar to `stylos_request_talk`, the user explicitly wants one auto-routed function.

### Why remote note creation should cross the Stylos boundary explicitly

The intended flow is:

- `[tool] -> [stylos/zenoh put] -> [receive] -> [insert db]`

That flow captures the real architectural boundary. A note should not appear because the sender wrote directly to its own local SQLite database. It should appear because the sender requested note creation over the Stylos network and the receiver durably stored it locally.

This matters for correctness and mental model clarity:

- the sender and receiver do not share one database
- the receiver should own note persistence for its own board
- note acceptance should reflect receiver-side validation and storage success
- transport and local persistence should remain separate responsibilities

**Alternative considered:** keep using `board_create_note` as direct DB insertion and treat `to_instance` as only metadata. Rejected: that breaks the process boundary and does not match the intended note-delivery semantics.

### Why this is a transport-boundary correction rather than a board redesign

The existing board model already fits the user's note workflow well after a note exists locally:

- the note lives in SQLite
- it belongs to `todo`, `in_progress`, or `done`
- it can be listed, read, moved, and updated locally
- idle agents can receive injected pending notes later

The missing part is not the board itself. The missing part is that note creation in Stylos-enabled builds should consistently use the Stylos delivery path before the receiver inserts the note.

**Alternative considered:** redesign the entire notes subsystem around a distributed shared board. Rejected: that would greatly expand scope beyond the requested behavior and is unnecessary for the desired sender-to-receiver note intake flow.

## Design

### `board_create_note` always uses Stylos flow when Stylos is enabled

`board_create_note` should remain the model-visible note-creation tool, but in Stylos-enabled builds it should always use the Stylos note-delivery path.

Normative behavior:

- when Stylos is enabled, `board_create_note` must submit the create request through the Stylos note request path regardless of whether `to_instance` is local or remote
- when Stylos is enabled, `board_create_note` must not short-circuit to direct local SQLite insertion
- when Stylos is not enabled and `to_instance` refers to the current local instance, `board_create_note` inserts directly into the local SQLite database
- when Stylos is not enabled and `to_instance` refers to a different instance, `board_create_note` fails clearly rather than silently creating a misleading local note
- the tool surface remains one function rather than being split into separate local and remote create tools

This preserves a simple model-facing API while ensuring one canonical Stylos-enabled create path.

**Alternative considered:** choose local insert for self-targeted notes in Stylos-enabled builds as an optimization. Rejected: uniform event and trigger behavior is more important here than avoiding the local network round-trip.

### Reuse a request shape similar to `stylos_request_talk` internally

Although the model should keep using one note-creation tool, the Stylos path can and should reuse the same style of request structure already used by `stylos_request_talk`.

Normative behavior:

- the note request should carry exact target `instance`, optional target `agent_id` defaulting consistently with current note semantics, note body, and optional caller-supplied `request_id`
- sender identity should be resolved automatically by the local Stylos runtime, the same way `stylos_request_talk` resolves sender identity
- the receiver reply should mirror existing request/reply patterns by returning acceptance, resolved target agent, request identity when provided, created `note_id` when successful, and machine-readable failure reason when not successful
- implementation code should prefer sharing validation and request/response conventions with existing Stylos request paths where practical

This gives the implementation a familiar structure without exposing a second tool name to the model.

**Alternative considered:** invent a completely different transport payload style for note creation. Rejected: the existing talk/task request patterns already fit this need well enough and reduce conceptual drift.

### Make the receiver-side Stylos note handler the canonical create trigger point

The receiver-side Stylos note handler should remain the single intake path that creates the note in SQLite when Stylos is enabled.

Normative behavior:

- the receiver validates the addressed `agent_id` against the current snapshot
- the receiver allocates or validates canonical note identity according to the current notes schema contract
- the receiver inserts the note into its own SQLite database with initial column `todo`
- the receiver returns an acknowledgement payload indicating acceptance or a machine-readable rejection reason
- any note-created events, audit hooks, metrics, or future trigger logic for Stylos-enabled note creation should attach to this receiver-side intake path rather than to multiple creation paths
- once inserted, the note follows the current local durable note lifecycle unchanged

This keeps persistence ownership and creation-side effects centralized on the destination instance where the note will actually be worked.

**Alternative considered:** allow both direct DB insert and Stylos receiver intake to coexist and try to keep them behaviorally aligned. Rejected: that increases drift risk and weakens the goal of a single behavioral trigger point.

### Keep `board_*` as the local durable board API after receipt

The `board_*` concept remains valid and should stay local-facing even though `board_create_note` may cross the network in Stylos-enabled builds.

Normative behavior:

- `board_list_notes`, `board_read_note`, `board_move_note`, and `board_update_note_result` continue to operate on the local instance's SQLite-backed board
- `board_create_note` is the only board tool whose implementation may cross the network, and in Stylos-enabled builds it does so consistently through the Stylos create flow
- docs must make the distinction explicit: board operations are local durable state operations, except that create uses the Stylos intake path when Stylos is enabled
- implementations should avoid implying that list/read/move/update operations transparently operate across the network

This preserves the conceptual cleanup from PRD-031 while fixing the create-path consistency issue.

**Alternative considered:** make all `board_*` tools remote-aware by destination. Rejected: only create naturally maps to remote submission in the current scope; broad remote awareness would complicate the board API unnecessarily.

### Preserve current idle-time injection and board lifecycle after receipt

This PRD does not change what happens after a note is stored locally on the receiver.

Normative behavior:

- a newly received note starts in `todo`
- existing note-selection rules for injection remain in effect
- `in_progress` notes still take priority over `todo` for idle-time injection
- note reads, moves, and result updates continue to use the local durable board state
- timestamps remain machine-consumed milliseconds as documented today

This keeps the implementation scope narrow and avoids reopening already-landed board semantics.

**Alternative considered:** use network receipt as an automatic immediate injection trigger that bypasses the current local idle rules. Rejected: that would be a behavioral change to board delivery policy beyond the requested scope.

### Keep non-Stylos builds local-only and fail clearly for non-local targets

The repository already uses feature flags and must keep feature-gated behavior cleanly separated.

Normative behavior:

- builds without the `stylos` feature continue to support local board creation and manipulation without referencing Stylos-only types or modules from always-on code paths
- if `board_create_note` is asked to target a different instance in a non-Stylos build, it should fail clearly rather than pretending to succeed locally
- any Stylos-backed note creation path must remain feature-gated consistently
- documentation should state clearly that the always-through-Stylos create flow applies when Stylos support is enabled

This preserves existing workspace architecture and avoids feature-flag regressions.

**Alternative considered:** make remote note creation semantics always-on and require all builds to know about Stylos transport. Rejected: this repository already gates Stylos integration, and always-on references would violate current architecture expectations.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/tools.rs` | Clarify that `board_create_note` uses the Stylos note-create flow whenever Stylos is enabled, rather than mixing direct local insertion with network delivery. |
| `crates/themion-core/src/tools.rs` | Keep the model-visible function name unchanged while returning reply metadata that works for both non-Stylos local creation and Stylos-mediated creation. |
| `crates/themion-cli/src/stylos.rs` | Treat the existing note request/queryable path as the single canonical create path in Stylos-enabled builds, including self-targeted note creation. |
| `crates/themion-cli/src/stylos.rs` | Keep note-created acknowledgement and future trigger/event behavior anchored at receiver-side note intake. |
| `crates/themion-cli/src/tui.rs` | If needed, provide the current local instance identity and Stylos routing context needed for `board_create_note` to submit through Stylos consistently when the feature is enabled. |
| `crates/themion-core/src/db.rs` | No schema redesign expected; confirm receiver-side insertion still uses the current durable notes schema unchanged. |
| `docs/architecture.md` | Document that Stylos-enabled note creation always uses the Stylos intake flow, even for self-targeted notes, while other board operations remain local after receipt. |
| `docs/engine-runtime.md` | Update the Stylos remote-request bridge and durable notes sections to describe one canonical Stylos-enabled create flow: tool → Stylos transport → receiver intake → local DB insert. |
| `docs/README.md` | Add this PRD to the PRD index. |

## Edge Cases

- Stylos is enabled and `board_create_note` targets the current local instance → verify: the request still goes through the Stylos note-delivery path and results in one receiver-side insert path rather than a direct DB short-circuit.
- Stylos is enabled, but the target instance is unreachable → verify: `board_create_note` returns a clear transport or timeout failure and no misleading local note insertion occurs.
- the target instance exists but the requested `agent_id` does not → verify: the receiver rejects the request with `not_found` or equivalent and no note row is inserted.
- the receiver accepts the network request but local DB insertion fails → verify: the reply reports failure and the sender does not treat the note as created.
- a non-Stylos build invokes `board_create_note` for local use → verify: local board creation still works without network dependencies.
- a non-Stylos build invokes `board_create_note` for a non-local `to_instance` → verify: the tool fails clearly rather than silently inserting locally.
- a future note-created event, workflow trigger, or metrics hook is added → verify: it only needs to attach to the receiver-side Stylos intake path in Stylos-enabled builds rather than to both local and remote create branches.
- a future remote-board listing feature is added later → verify: this PRD's create-path unification does not imply that current list/read/move/update tools are already remote-aware.

## Migration

No data-schema migration is required.

Behavioral migration in this PRD is limited to unifying the create path when Stylos is enabled:

- in Stylos-enabled builds, all note creation goes through Stylos receiver intake
- in non-Stylos builds, local-target note creation remains direct local insertion
- in non-Stylos builds, non-local note creation fails clearly
- existing locally stored notes remain valid and continue to follow the current board lifecycle

If the implementation changes tool descriptions or result payload wording to reflect this always-through-Stylos behavior, docs and prompt-adjacent descriptions should be updated in the same change.

## Testing

- call `board_create_note` with the current local `to_instance` in a `themion-cli --features stylos` setup → verify: the request still crosses the Stylos note-delivery path and the note is inserted only through the receiver-side intake handler.
- call `board_create_note` with a different reachable `to_instance` in a `themion-cli --features stylos` setup → verify: the request crosses Stylos, the receiver inserts the note locally, and the caller receives the created `note_id` or equivalent acknowledgement.
- create a note while the target agent is busy in a Stylos-enabled build → verify: the receiver still accepts and stores the note, and later idle-time injection follows existing board rules.
- call `board_create_note` with an unknown target `agent_id` in a Stylos-enabled build → verify: the receiver rejects it clearly and no note row is created.
- call `board_create_note` while the target instance is unavailable in a Stylos-enabled build → verify: the caller sees a transport/request failure and no local fallback insertion occurs silently.
- call `board_create_note` for a non-local `to_instance` in a non-Stylos build → verify: the tool fails clearly without referencing Stylos-only code paths at runtime.
- run `cargo check -p themion-cli` and `cargo check -p themion-cli --features stylos` after implementation → verify: default and Stylos-enabled builds both compile cleanly.

## Implementation checklist

- [x] document that `board_create_note` always uses the Stylos note-delivery flow when Stylos is enabled
- [x] ensure Stylos-enabled note creation does not short-circuit to direct local DB insertion, including self-targeted note creation
- [x] keep the receiver-side Stylos note handler as the single canonical creation path in Stylos-enabled builds
- [x] reuse a request/reply shape aligned with current Stylos request patterns where practical
- [x] return receiver acknowledgement with created note identity where practical
- [x] keep receiver-side note validation and insertion aligned with the current durable notes schema
- [x] ensure non-Stylos builds fail clearly for non-local targets while preserving local note creation
- [x] update architecture and runtime docs to reflect the unified create path clearly
- [x] update `docs/README.md` with this PRD entry
