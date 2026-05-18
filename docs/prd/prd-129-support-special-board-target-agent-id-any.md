# PRD-129: Support Special Board-Target Agent ID `[any]`

- **Status:** Implemented
- **Version:** v0.79.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design intent) + Themion (PRD authoring)
- **Date:** 2026-05-16

## Summary

- Add one reserved special board-target agent id: `[any]`.
- `[any]` means a board note is not assigned to one specific local agent and may be picked up by any eligible local agent in the same project-wide board scope.
- Do not allow creating a real local agent whose `agent_id` is `[any]`, to avoid confusion between a reserved keyword and a real agent.
- Board-note tools and guidance must explain how to create and query notes targeted to `[any]`, including support for `to_instance="[any]"`.
- Extend watchdog board-note pickup so it may fall back to `[any]` todo notes when an eligible agent has no pending agent-specific todo note.
- Add durable exclusive-claim tracking for shared `[any]` note pickup so only one agent can successfully claim one shared note at a time.

## Goals

- Reserve `[any]` as a clear project/runtime-wide board-target keyword.
- Let callers create durable board notes that are intentionally not tied to one named local agent.
- Let callers query board notes explicitly targeted to `[any]`.
- Support `[any]` as the matching shared value for both `to_agent_id` and `to_instance` in board-note flows.
- Keep board-note tools compact by reusing the existing targeting fields instead of inventing a second targeting parameter.
- Prevent agent-id ambiguity by forbidding creation of a real local agent named `[any]`.
- Extend watchdog board scheduling so shared `[any]` todo work can be picked up when no agent-specific todo is pending.

## Non-goals

- Do not change general local-agent targeting semantics outside board-note creation/query and watchdog board scheduling.
- Do not make `[any]` a wildcard for unrelated tools such as direct inbox messaging, workflow tools, or Stylos network targeting.
- Do not redesign board-note columns, note ownership rules, or done-mention semantics.
- Do not make `[any]` mean “all agents at once”; one note should still be handled by one selected local agent at a time.
- Do not remove existing agent-specific board-note targeting.

## Background & Motivation

Board notes currently target a specific `to_agent_id` and `to_instance`.

That works well for delegated work owned by one known local agent. It is less convenient for shared work that should be available to any eligible local agent in the same project-wide board scope.

Today, callers must pick one concrete agent id up front even when the real intent is:

- this is a local todo note
- any suitable local agent may pick it up
- the watchdog should route it when one eligible local agent becomes available

Without a reserved shared target, callers either guess a concrete agent too early or need ad hoc routing behavior outside the board-note contract.

A dedicated reserved keyword is the smallest change that keeps the current tool surface understandable.

### Current usage problem

The missing case is a note that is:

- durable
- project-wide in intent
- optionally not pinned to one concrete instance
- visible to the watchdog
- not pre-assigned to one concrete local agent

This PRD adds that missing case by reserving `[any]` for shared board targeting without changing the basic board-note model.

## Design

### 1. Reserve `[any]` as a special board-target agent id

`[any]` should be a reserved keyword, not a normal user-chosen local agent id.

Required behavior:

- `[any]` is valid only where board-note targeting or board-note querying explicitly supports it
- `[any]` in `to_agent_id` means “not pinned to one concrete local agent; any eligible local agent may handle this note”
- `[any]` in `to_instance` means “not pinned to one concrete instance; the note is project-wide rather than instance-specific”
- the runtime must treat `[any]` as a special reserved value, not as the literal id of a real agent or instance
- docs and model-facing guidance should show the keyword exactly as `[any]`

Usage rule:

- when the caller wants a shared project-wide todo note, use `to_agent_id="[any]"`
- when the caller does not want to pin that shared note to one concrete instance, also use `to_instance="[any]"`

This PRD intentionally keeps the meaning narrow and explicit.

### 2. Forbid creating a real local agent named `[any]`

To avoid ambiguity, the runtime must reject attempts to create or register a real local agent whose id is `[any]`.

Required behavior:

- `local_agent_create(agent_id="[any]", ...)` must fail with a clear validation error
- any other local-agent creation path that accepts an explicit id must reject `[any]`
- any runtime validation that checks local-agent ids should treat `[any]` as reserved rather than as a normal existing-agent lookup value
- the built-in agent allocator must never generate `[any]`
- docs should state that `[any]` is reserved

This prevents confusion between:

- a real agent named `[any]`
- a shared board-target keyword `[any]`

### 3. Board-note creation must support `[any]`

Allowed creation shapes for this PRD:

- concrete instance + concrete agent → existing behavior
- concrete instance + `[any]` agent → shared queue scoped to that concrete instance
- `[any]` instance + `[any]` agent → shared project-wide queue not pinned to one instance

Disallowed shape for this PRD:

- `[any]` instance + concrete agent → reject as invalid because a concrete agent target without a concrete instance is ambiguous in the current board model

This keeps the first implementation explicit and avoids half-global, half-concrete targeting rules.

Board-note creation should keep using the existing `to_agent_id` field.

Required behavior:

- `board_create_note` keeps the same parameter shape; no new targeting field is added
- `board_create_note` must accept `to_agent_id="[any]"`
- `board_create_note` must accept `to_instance="[any]"`
- when used, the note is created as a shared board note rather than a note owned by one concrete local agent
- normal board-note fields such as `to_instance`, column, note kind, and result handling stay unchanged
- when Stylos is enabled, shared board-note creation for `to_agent_id="[any]"` must still use the local durable board-note create path rather than requiring a remote Stylos responder
- `board_create_note` must reject `to_instance="[any]"` together with a concrete `to_agent_id`
- documentation and prompt guidance must explain that a shared project-wide todo note uses `to_agent_id="[any]"`, and may use `to_instance="[any]"` when it is not instance-specific

Usage guidance:

- to create a shared project-wide todo note for any eligible local agent, use the normal board note tool and set `to_agent_id="[any]"`
- when that shared note should not be tied to one concrete instance, set `to_instance="[any]"`
- callers should still use a concrete `to_agent_id` when one specific local agent is the intended owner

Instruction wording requirement:

- docs and prompt guidance should say plainly: for a global local board note, use `to_agent_id="[any]"`

### 4. Board-note query must support explicit `[any]` targeting

Query behavior should stay explicit and unsurprising.

Required behavior:

- `board_list_notes` keeps the same parameter shape; no second query mode is added
- `board_list_notes` must accept `to_agent_id="[any]"`
- `board_list_notes` must accept `to_instance="[any]"`
- that query returns notes explicitly targeted to `[any]`
- querying one concrete `to_agent_id` must not silently merge in `[any]` results unless a later PRD adds a separate combined query mode
- querying `to_instance="[any]"` together with a concrete `to_agent_id` must be rejected in the same way as create-time validation
- querying one concrete `to_instance` must not silently merge in `[any]` instance-targeted results unless a later PRD adds a separate combined query mode
- board-note read/update-by-note-id behavior does not need special `[any]` handling because those tools address a concrete note id, not a target selector

Usage guidance:

- to inspect shared board notes, query them directly with `to_agent_id="[any]"`
- to inspect project-wide non-instance-specific shared board notes, query them directly with `to_instance="[any]"`
- to inspect notes for one concrete agent, continue querying that concrete `to_agent_id`
- do not query `to_instance="[any]"` with a concrete `to_agent_id`; that shape is invalid for this PRD

Example rule:

- use `board_list_notes(..., to_agent_id="[any]")` when the caller wants the shared local board queue
- use `board_list_notes(..., to_agent_id="master")` or another concrete id when the caller wants that specific agent's queue
- use `board_list_notes(..., to_instance="[any]", to_agent_id="[any]")` when the caller wants the project-wide shared board queue only

This keeps the tool contract simple: explicit in, explicit out.

### 5. Watchdog board pickup should use explicit fallback order

The watchdog should extend its local board-note selection logic to consider shared `[any]` todo notes when a candidate local agent has no pending agent-specific todo note.

Required behavior:

- for an eligible idle local agent, the watchdog should first prefer pending todo notes targeted to that concrete `agent_id`
- if no pending todo note exists for that concrete `agent_id`, the watchdog may consider pending todo notes targeted to `[any]`
- the fallback should apply inside the same project-wide board scope, including notes that use `to_instance="[any]"`
- `[any]` notes must still use the existing local claim and duplicate-prevention logic so one shared note is not injected into several agents at once
- `[any]` notes remain stored with `to_agent_id="[any]"` and, when used, `to_instance="[any]"`; the runtime must not rewrite the routing target to the claiming concrete agent or instance
- once selected, the shared note becomes in-flight for one chosen local agent through the normal board handoff path
- done mentions, follow-up, and result updates should continue using the concrete handling agent that actually picked up the work

Selection rule:

- specific target first, shared `[any]` second
- if several eligible local agents are idle for the same shared note, prefer `master` first; otherwise choose one remaining eligible idle agent at random

### 6. Shared `[any]` notes need durable exclusive claim tracking

The current runtime should not rely only on in-memory selection order to prevent duplicate handling of a shared `[any]` note. This section defines the claim behavior tightly enough to implement directly.

Required behavior:

- shared `[any]` note claiming must be enforced in durable storage, not only by best-effort in-memory coordination
- `board_notes` must gain explicit claim-tracking fields: `claimed_by_instance`, `claimed_by_agent_id`, and `claimed_at_ms`
- the runtime must claim an `[any]` note atomically before handoff so only one eligible local agent can successfully become the active handler
- if claim succeeds, other agents must treat that shared note as unavailable until the claim is released or the note completes
- if handoff fails or the claim needs to be abandoned before active handling, the runtime must clear the claim so another eligible agent may take it later
- the claim tracks temporary active ownership only; it does not replace the stored routing target

Claim eligibility rule:

- only shared notes with `to_agent_id="[any]"` are eligible for the new shared-claim path
- a shared note is claimable only when `column_name = 'todo'`
- a shared note is claimable only when `injection_state = 'pending'`
- a shared note is claimable only when `claimed_by_instance IS NULL`, `claimed_by_agent_id IS NULL`, and `claimed_at_ms IS NULL`
- notes in `in_progress`, `blocked`, or `done` are not claimable through the shared-claim path
- already-claimed shared notes are not claimable again unless the claim is cleared first

Claim candidate selection rule:

- the runtime should first look for a concrete-agent pending note for the eligible idle local agent using current concrete-target behavior
- only if no such concrete-target note exists should the runtime look for a shared `[any]` candidate
- shared candidate lookup order should be:
  1. `to_instance = <local concrete instance>` and `to_agent_id = "[any]"`
  2. `to_instance = "[any]"` and `to_agent_id = "[any]"`
- within each shared candidate bucket, pick the oldest eligible row by `created_at_ms ASC`
- if several idle agents are eligible for the same shared candidate, `master` gets first attempt; otherwise one remaining eligible idle agent is chosen at random for the first attempt

Atomic claim rule:

- candidate lookup and claim may remain two separate DB operations, but the claim step itself must be atomic
- the atomic claim must update exactly one row only if that row still satisfies the full claim eligibility rule at update time
- if the update affects zero rows, the runtime must treat the claim attempt as lost and retry normal selection logic rather than injecting the note anyway
- the atomic claim should write the claiming concrete `instance`, claiming concrete `agent_id`, and `claimed_at_ms`

Release rule:

- release must clear `claimed_by_instance`, `claimed_by_agent_id`, and `claimed_at_ms`
- release must not rewrite `to_instance`, `to_agent_id`, `column_name`, or historical result fields
- release is required when handoff fails before prompt injection is successfully established for the chosen concrete agent
- release is not required before normal completion if the note is moving to a terminal or otherwise non-claimable state in the same successful flow
- if the implementation requeues a claimed shared note back to `todo`/`pending`, it must clear the claim in the same flow

Preferred implementation shape:

- keep the routing target fields unchanged as `[any]`
- use claim fields on `board_notes` instead of a separate claim table for the first implementation
- treat the claim as current runtime state for one note, not as historical audit storage
- add DB helpers for: next shared-note candidate lookup, atomic claim, and claim release
- the atomic claim should succeed only when the row is still eligible and all claim fields are still `NULL`
- a successful claim should identify the claiming concrete `instance` and `agent_id`
- normal note completion does not need a second ownership rewrite; finishing the note is enough to make the old claim irrelevant

Suggested DB helper surface:

- `next_board_note_for_injection(to_instance, to_agent_id)` keeps current concrete-target behavior
- add one shared candidate helper that returns the next eligible shared note for one instance scope, ordered by `created_at_ms ASC`
- add one atomic claim helper that takes `note_id`, `claimed_by_instance`, `claimed_by_agent_id`, and writes `claimed_at_ms`
- add one claim-release helper that clears the three claim columns for one `note_id`

This preserves current specific-note priority while making shared board work routable.

### 7. Keep `[any]` scoped to local board semantics

This PRD should not turn `[any]` into a system-wide wildcard.

Required behavior:

- `[any]` is a reserved board-target keyword for board-note creation/query and watchdog board scheduling, including the `to_instance` field
- it should not be documented as a valid special target for direct agent messaging or unrelated tool families unless a later PRD expands it
- runtime/docs should describe `[any]` as a shared board-routing target, not a general-purpose agent selector

This keeps the first version clear and low-risk.

## Changes by Component

| File / area | Change |
| --- | --- |
| `docs/prd/prd-129-support-special-board-target-agent-id-any.md` | Add the durable PRD for reserved `[any]` board-target support. |
| `crates/themion-core/src/tools.rs` | Update `board_create_note` and `board_list_notes` descriptions and validation to accept `[any]` as a reserved board-target/query value for both `to_agent_id` and `to_instance`, reject invalid mixed target shapes, keep the guidance explicit, and keep shared `[any]` creation on the local durable board path even when Stylos is enabled. |
| `crates/themion-cli` local-agent management path | Reject explicit local agent creation with `agent_id="[any]"` so the reserved keyword cannot become a real local agent id. |
| `crates/themion-core/src/db.rs` | Add durable board-note claim fields, schema migration, shared candidate lookup helpers, and atomic shared-note claim/release helpers for `[any]` note pickup. |
| `crates/themion-cli` board/watchdog runtime | Extend pending-note selection so concrete-agent todo notes win first and `[any]` todo notes are the fallback when no concrete-agent todo is pending for that eligible local agent; when several agents are idle, prefer `master` first, else choose one remaining eligible idle agent at random; use the durable claim helpers before handoff and release claims on failed handoff. |
| `crates/themion-cli/src/board_runtime.rs` | Split concrete-target note lookup from shared `[any]` note lookup/claim so `[any]` pickup goes through durable exclusive claim logic before prompt injection. |
| `docs/engine-runtime.md` | Document how to create and query shared board notes with `to_agent_id="[any]"` and `to_instance="[any]"`, and state that `[any]` is reserved. |
| `docs/README.md` | Add the new PRD entry and keep status/version aligned later. |

## Edge Cases

- caller creates a board note with `to_agent_id="[any]"` → verify: the note is accepted and stored as a shared board-target note.
- caller creates a board note with concrete `to_instance` and `to_agent_id="[any]"` → verify: the note is accepted as instance-scoped shared work.
- caller creates a board note with `to_instance="[any]"` → verify: the note is accepted as project-wide rather than instance-specific.
- caller creates a board note with `to_instance="[any]"` and a concrete `to_agent_id` → verify: creation is rejected as an invalid mixed shared/concrete target shape.
- caller creates a board note with `to_agent_id="[any]"` while Stylos is enabled → verify: the note is created locally and does not require a remote Stylos responder.
- caller queries `board_list_notes` with `to_agent_id="[any]"` → verify: only explicitly shared `[any]` notes are returned.
- caller queries `board_list_notes` with `to_instance="[any]"` → verify: only explicitly `[any]` instance-targeted notes are returned.
- caller queries notes for one concrete agent → verify: the result set does not silently include `[any]` notes.
- caller queries notes for one concrete agent while `[any]` notes also exist → verify: shared notes are still queried separately and are not merged implicitly.
- user tries to create a real local agent with `agent_id="[any]"` → verify: the runtime rejects the request with a clear reserved-keyword error.
- watchdog sees both a concrete-agent todo note and an `[any]` todo note for the same eligible idle agent → verify: the concrete-agent note wins first.
- watchdog claims an `[any]` note for one agent → verify: the stored routing target remains `[any]` and only claim-tracking fields identify the active handler.
- watchdog sees no concrete-agent todo note but does see an `[any]` todo note → verify: the shared note may be claimed and injected.
- several agents are idle while one `[any]` todo note exists → verify: only one agent claims the note and duplicate injection is prevented.
- a shared `[any]` note moves to in-progress or done → verify: normal board-note column and follow-up behavior still applies.
- one agent claims an `[any]` note and another agent checks for work at the same time → verify: the second agent cannot also claim the same note.
- a shared note exists for concrete `to_instance=<local instance>` and another exists for `to_instance="[any]"` → verify: the concrete-instance shared note wins first when both are otherwise eligible.
- a shared note is already `in_progress` with a stale-looking claim → verify: it is not reclaimed by the shared-claim pickup path in this PRD.
- one agent claims an `[any]` note but handoff fails before active handling → verify: the claim is released and another eligible agent may pick it up later.

## Migration

This PRD does require a board-note schema migration to add durable claim-tracking fields for shared `[any]` note pickup.

If any current validation assumes every `to_agent_id` must match a real local agent id at create/query time, that validation should be narrowed so `[any]` is allowed as the one reserved exception for board-note create/query flows.

If any current validation assumes every `to_instance` must be one concrete instance id, that validation should be narrowed so `[any]` is allowed as the reserved project-wide instance value for board-note create/query flows.

Existing rows do not need backfill values for the new claim fields; they may default to `NULL`.

## Testing

- call `board_create_note` with `to_agent_id="[any]"` → verify: the note is created successfully as a shared board-target note.
- call `board_create_note` with `to_instance="[any]"` → verify: the note is created successfully as a project-wide shared note.
- call `board_list_notes` with `to_agent_id="[any]"` → verify: shared `[any]` notes are returned.
- call `board_list_notes` with `to_instance="[any]"` → verify: only project-wide `[any]` instance-targeted notes are returned.
- call `board_create_note` with `to_agent_id="[any]"` while Stylos is enabled → verify: the note is created through the local durable board path rather than failing with `timeout/offline: no responder`.
- call `board_create_note` with `to_instance="[any]"` and concrete `to_agent_id` → verify: creation is rejected with a clear invalid-target-shape error.
- call `board_list_notes` with one concrete `to_agent_id` when both concrete and `[any]` notes exist → verify: only the concrete target's notes are returned.
- call `board_list_notes` with `to_instance="[any]"` and concrete `to_agent_id` → verify: query is rejected with a clear invalid-target-shape error.
- call `local_agent_create` with `agent_id="[any]"` → verify: creation is rejected with a reserved-keyword error.
- claim one `[any]` note for one concrete local agent → verify: `to_agent_id` and `to_instance` stay `[any]` while claim fields record the active handler.
- claim one concrete-instance shared note and one project-wide shared note in otherwise equal conditions → verify: the concrete-instance shared note is tried first.
- try to claim the same `[any]` note concurrently from two eligible local agents → verify: exactly one claim succeeds.
- try to claim an `[any]` note that changed away from `todo/pending` between select and claim → verify: the claim fails cleanly and the note is not injected through the shared-claim path.
- create a pending todo note for one concrete agent and another pending todo note for `[any]`, then let that concrete agent become eligible → verify: watchdog chooses the concrete-agent note first.
- create only a pending `[any]` todo note, then let one local agent become eligible → verify: watchdog picks up the shared note.
- create one pending `[any]` todo note with several eligible idle local agents including `master` → verify: `master` is preferred first.
- create one pending `[any]` todo note with several eligible idle local agents and no eligible `master` → verify: only one remaining eligible local agent claims it and no duplicate handoff occurs.
- force a failed handoff after claiming an `[any]` note → verify: the claim is cleared and the note becomes eligible again.
- complete a successfully claimed `[any]` note → verify: the note reaches its normal non-claimable state and no second claim path can pick it up again.
- run `cargo check -p themion-core -p themion-cli` after implementation → verify: touched crates compile with default features.
- run `cargo check --all-features -p themion-core -p themion-cli` after implementation → verify: touched crates still compile across feature combinations.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the touched CLI/runtime paths still compile with Stylos enabled.

## Implementation checklist

- [x] reserve `[any]` in board-note create/query handling
- [x] reject real local-agent creation and registration with `agent_id="[any]"`
- [x] reject invalid mixed target shapes such as `to_instance="[any]"` with a concrete `to_agent_id`
- [x] update model-facing board tool descriptions and runtime docs with explicit `[any]` usage guidance for both `to_agent_id` and `to_instance`
- [x] extend watchdog board-note selection so concrete-agent todo notes win first and `[any]` is the fallback when no concrete-agent todo note exists for an eligible local agent
- [x] add shared candidate lookup order: concrete-instance shared queue before project-wide shared queue
- [x] make shared-note atomic claim require `column_name='todo'`, `injection_state='pending'`, and all claim fields `NULL` at update time
- [x] add durable claim-tracking fields and atomic claim/release behavior for shared `[any]` notes without rewriting the stored routing target
- [x] update board runtime flow so shared `[any]` pickup uses the new durable claim path before handoff
- [x] preserve existing local claim and duplicate-prevention behavior for shared `[any]` notes
- [x] update `docs/README.md`
