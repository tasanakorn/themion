# PRD-065: Reduce Non-TUI Responsibilities in the TUI Layer

- **Status:** Implemented
- **Version:** v0.41.0
- **Scope:** `themion-cli`, `themion-core`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-27

## Summary

- Themion's architecture already says reusable runtime behavior belongs in `themion-core` and terminal presentation belongs in `themion-cli`, but `crates/themion-cli/src/tui.rs` still owns some board-coordination work that is not really TUI-shaped.
- The clearest first extraction target is the Stylos board-note coordination path in TUI mode: `maybe_inject_pending_board_note` and the adjacent done-mention follow-up logic both perform direct DB-backed coordination from `tui.rs`.
- Themion should keep rendering, input handling, and chat/event display in the TUI layer while moving this board coordination cluster behind a narrower non-TUI interface.
- The initially targeted board-note injection and completion-follow-up path now lives behind a CLI-local helper module, and the landing also pulled additional runtime-shaped inspection and main-agent rebuild orchestration out of `tui.rs`.
- This PRD now stands as the historical product/design record for that TUI-boundary tightening pass.

## Goals

- Reduce non-presentation responsibilities currently concentrated in `crates/themion-cli/src/tui.rs`.
- Move the first clear direct-DB coordination cluster out of the TUI layer.
- Preserve the repository's intended layering: reusable runtime behavior in `themion-core`, CLI-local orchestration in non-TUI `themion-cli` modules, and terminal presentation/input handling in the TUI layer.
- Make future TUI changes lower-risk by reducing hidden persistence and orchestration coupling inside `tui.rs`.
- Define an implementation-ready cleanup target that can land as focused extractions rather than a vague long-term cleanup intention.

## Non-goals

- No redesign of the core harness loop, provider abstraction, or workflow model.
- No requirement to move all non-rendering logic out of `themion-cli`; some runtime wiring is intentionally CLI-local.
- No requirement to eliminate SQLite access from the CLI crate entirely.
- No behavior change to board notes, Stylos request handling, or session startup beyond what is required to preserve current behavior after the extraction.
- No attempt in this PRD to split every large function or fully rewrite `tui.rs`.
- No requirement to introduce a new framework, state-management library, or cross-process architecture.

## Background & Motivation

### Current state

The repository already documents a clear architectural intent:

- `themion-core` owns reusable harness/runtime behavior, tool handling, and SQLite-backed history behavior
- `themion-cli` owns terminal UI, config, startup wiring, and other user-facing local flows
- `crates/themion-cli/src/tui.rs` is meant to be the terminal presentation and event-handling layer

Recent work has already moved some responsibilities out of the old monolithic TUI path:

- shared bootstrap moved into `app_state.rs`
- terminal-mode orchestration moved into `tui_runner.rs`
- headless entrypoints moved into `headless_runner.rs`
- text editing and composer behavior moved into `textarea.rs` and `chat_composer.rs`

That direction is good, but `tui.rs` still mixes presentation logic with coordination work that is not inherently TUI-specific.

A concrete current example in Stylos-enabled TUI mode is the board-note coordination cluster:

- `maybe_inject_pending_board_note(...)` previously selected the local target agent, called `self.db.next_board_note_for_injection(...)`, immediately called `self.db.mark_board_note_injected(...)`, built the note prompt, created `IncomingPromptRequest`, emitted a user-visible event line, and submitted the prompt
- `maybe_emit_done_mention_for_completed_note(...)` previously parsed note identity from the active prompt, called `self.db.get_board_note(...)`, decided whether more work was needed, sometimes re-queued the note locally, and on completion called `self.db.mark_board_note_completion_notified(...)`
- that board-note coordination path is now extracted into `crates/themion-cli/src/board_runtime.rs`, and `tui.rs` now consumes typed helper results instead of performing those board-note DB coordination operations inline

These functions are not primarily about rendering. They are coordination and persistence flows that happen to be triggered from the TUI event loop.

### Why this matters

When the TUI owns direct persistence and coordination logic:

- UI changes become riskier because behavior and rendering are tightly coupled
- non-TUI execution paths have a harder time reusing the same behavior cleanly
- testing becomes more awkward because behavior may be trapped inside terminal-oriented state objects
- architectural intent becomes less trustworthy because the documented boundaries are not the practical ones

The board-note coordination path is a strong first target because it is already domain-shaped: it chooses work, mutates durable note state, and decides follow-up behavior before the TUI merely displays what happened.

**Alternative considered:** leave the layering as-is and only make opportunistic cleanup during unrelated feature work. Rejected: that tends to preserve accidental coupling because the underlying ownership problem remains unaddressed.

## Design

### Design principles

- Keep rendering, view formatting, input editing, and terminal event handling in the TUI layer.
- Move persistence access and coordination logic to the narrowest non-TUI layer that can own it cleanly.
- Prefer small responsibility-specific helpers or modules over one new giant replacement abstraction.
- Preserve current behavior first; improve ownership without forcing a product redesign.
- Favor incremental extraction paths that can land in multiple slices.

### 1. Define the TUI layer as a presentation and interaction boundary

For this PRD, `crates/themion-cli/src/tui.rs` should be treated as the terminal presentation layer plus immediate interaction policy.

That means `tui.rs` should remain responsible for:

- rendering chat, status, overlays, and board-oriented terminal surfaces
- translating keyboard, mouse, paste, and tick events into app interactions
- local display formatting choices such as trimming, focus, and layout
- dispatching already-defined application actions or runtime requests
- holding UI-shaped ephemeral state such as viewport, selection, overlay mode, and input drafts

And it should increasingly stop owning:

- direct database queries whose purpose is data retrieval or coordination rather than rendering
- business-rule decisions that could be shared with headless or other non-TUI paths
- runtime/persistence orchestration that does not need terminal context

This definition gives future cleanup work a stable boundary to target.

**Alternative considered:** define the TUI layer as a general "interactive app controller" that can keep mixed responsibilities. Rejected: that wording is exactly what allows persistence and orchestration work to keep accumulating there.

### 2. Use the Stylos board-note coordination path as the first extraction slice

The first implementation slice should be explicit rather than generic. The initial extraction target is the Stylos-enabled board-note coordination cluster now embedded in `tui.rs`.

Initial in-scope functions:

- `maybe_inject_pending_board_note(...)`
- `maybe_emit_done_mention_for_completed_note(...)`

Why these were the right initial slice:

- both are feature-bounded and easy to reason about as one domain cluster
- both perform direct DB-backed coordination work from inside the TUI layer
- both decide what should happen next before the TUI merely reports the outcome
- both sit adjacent to existing CLI-local Stylos/app-state helpers, so extraction can stay local to `themion-cli` first

Landed first-phase result:

- `tui.rs` no longer directly calls the board-note DB coordination methods for this path
- `crates/themion-cli/src/board_runtime.rs` now resolves pending note injection and completion follow-up into typed helper results
- the TUI asks that helper what action to take next and remains responsible for display and prompt submission

**Alternative considered:** start with generic status rendering cleanup or tool-label formatting. Rejected: those are smaller presentation concerns and do not attack the direct DB-backed responsibility problem as clearly as the board coordination path does.

### 3. Extract board coordination into a non-TUI CLI helper first

This extraction preferred a CLI-local helper rather than forcing the whole board coordination path into `themion-core` immediately.

Reasoning:

- the path is tightly connected to local Stylos bridge behavior and `IncomingPromptRequest` construction, which are CLI-local concerns
- the repository already uses `app_state.rs` and nearby CLI helpers for shared non-TUI runtime behavior
- extracting first into a non-TUI CLI helper keeps the scope smaller while still removing the persistence logic from `tui.rs`

Recommended ownership shape:

- add a narrow helper under `crates/themion-cli/src/` such as a board coordination helper/service
- let that helper own the DB-backed selection and state-mutation steps for pending-note injection and note-completion follow-up
- let the TUI call the helper and then render returned events or submit returned prompt requests

This preserves correct layering without pretending every coordination concern is already core-reusable.

**Alternative considered:** move the entire path into `themion-core` immediately. Rejected: parts of this flow are still CLI-local and Stylos-bridge-aware, so an immediate core move risks the opposite architectural mistake.

### 4. Prefer action-oriented helper results over sharing raw DB access

The extracted interface should not simply move the same DB calls into free functions while keeping `tui.rs` responsible for sequencing all decisions.

Preferred shape:

- TUI asks for the next pending board-note injection action
- helper decides whether a note is eligible, marks durable state as needed, and returns a typed result describing what should happen next
- TUI renders the returned event text and submits the returned prompt if present

Similarly for completion follow-up:

- TUI asks the helper to resolve post-turn follow-up for the active incoming note context
- helper decides whether to continue the note, create a done mention, or do nothing
- TUI performs only the UI-facing consequences of that decision

This keeps extraction meaningful and prevents persistence mechanics from remaining embedded in TUI control flow.

**Alternative considered:** keep raw `DbHandle` visible and just move SQL calls into helper methods with TUI-controlled branching. Rejected: that shrinks code blocks but does not materially improve responsibility boundaries.

### 5. Preserve current user-visible behavior and prompt semantics exactly

The extraction should not change product behavior. It should preserve:

- current pending-note selection semantics
- current `mark_board_note_injected` timing
- current note prompt construction inputs and prompt text
- current follow-up behavior when a note-backed turn ends while the note is not yet in `done`
- current done-mention creation path and `completion_notified_at_ms` semantics
- current user-visible remote-event/status lines except for incidental bug fixes that may fall out of cleaner ownership

This PRD is about ownership, not about redesigning the board workflow.

**Alternative considered:** combine layering cleanup with board behavior changes while touching the same code. Rejected: that would make the extraction harder to review and easier to regress.

### 6. Acceptance target for this implementation pass

This PRD should be considered implemented when all of the following are true:

- the Stylos board-note coordination cluster has been extracted out of `crates/themion-cli/src/tui.rs`
- `tui.rs` no longer directly calls `next_board_note_for_injection`, `mark_board_note_injected`, `get_board_note`, or `mark_board_note_completion_notified` for that path
- the new ownership lives in a non-TUI `themion-cli` helper or service with a narrow action/query interface
- the TUI consumes returned actions/results and remains responsible only for display and prompt submission
- CLI-local system-inspection snapshot assembly and replacement main-agent rebuild/session-insert orchestration are also no longer implemented inline in `tui.rs`
- current user-visible board injection and done-mention behavior remains unchanged
- `docs/architecture.md`, `docs/engine-runtime.md`, `docs/README.md`, and this PRD reflect the landed scope accurately
- `cargo check -p themion-cli --features stylos` passes
- `cargo check -p themion-cli --all-features` passes
- `cargo check --all-features -p themion-core -p themion-cli` passes

This acceptance target records the concrete extraction pass that landed instead of leaving the cleanup as a vague long-term promise.

## Changes by Component

| File / area | Change |
| --- | --- |
| `crates/themion-cli/src/tui.rs` | Remove the direct DB-backed board-note coordination path from `maybe_inject_pending_board_note(...)` and `maybe_emit_done_mention_for_completed_note(...)`; keep UI event handling, prompt submission, and transcript display. |
| `crates/themion-cli/src/board_runtime.rs` | Own pending-note selection, injection-state mutation, note-completion follow-up decisions, and done-mention completion marking for the extracted board-note coordination path. |
| `crates/themion-cli/src/app_state.rs` or nearby shared runtime wiring | Continue to host shared non-TUI board-related helpers such as local done-mention creation, while the TUI consumes a prepared service boundary rather than raw persistence logic. |
| `crates/themion-cli/src/app_runtime.rs` | Own additional CLI-local runtime helpers extracted from `tui.rs`, including system-inspection snapshot assembly and replacement main-agent rebuild/session-insert orchestration. |
| `crates/themion-core/src/` where appropriate | Optionally absorb smaller reusable board/runtime helpers only if future extractions reveal logic that is clearly not CLI-local. |
| `docs/architecture.md` | Clarify that Stylos board-note coordination is not owned directly by the TUI presentation layer. |
| `docs/engine-runtime.md` | Document the tightened boundary between terminal presentation and CLI-local board coordination for note injection and completion follow-up. |
| `docs/README.md` | Update this PRD entry status/version when the work lands. |

## Edge Cases

- no interactive agent is available for note injection → verify: the helper returns no injection action and the TUI remains idle without changing behavior.
- a pending note lookup fails or returns no eligible note → verify: the extracted helper preserves the current no-op behavior.
- a note-backed turn ends while the note is still not in `done` → verify: the follow-up prompt is still produced with the same behavior as today.
- a completed note is not a `work_request` or was already completion-notified → verify: the helper returns no done-mention action.
- future non-TUI callers need the same coordination logic → verify: the helper interface is narrow enough to be reused or later promoted cleanly.

## Migration

This is an internal ownership and module-boundary cleanup with no user data migration.

Rollout guidance:

- extract the pending board-note injection and completion follow-up cluster into a non-TUI helper
- keep the TUI calling that helper and rendering/submitting the returned actions
- preserve existing board-note semantics and visible transcript behavior
- document the new ownership accurately

Future cleanup can repeat the same pattern for other non-TUI clusters in `tui.rs` if needed, but this PRD's documented implementation pass is now complete.

## Testing

- run the Stylos-enabled idle note-injection path after extraction → verify: the TUI no longer performs the direct board-note DB calls, but the same prompt is still injected for the same eligible note.
- complete a note-backed turn where the note remains pending → verify: the same continue-working follow-up prompt is produced.
- complete a note-backed `work_request` note that reaches `done` → verify: the same done-mention path still runs and completion notification is marked exactly once.
- complete a note-backed turn for a note that is already completion-notified or not a `work_request` → verify: no duplicate done mention is created.
- run `cargo check -p themion-cli --features stylos` after implementation → verify: the Stylos-enabled build stays clean.
- run `cargo check -p themion-cli --all-features` after implementation → verify: the touched crate still compiles cleanly with all features enabled.
- if shared/core code changes as part of the extraction → run `cargo check --all-features -p themion-core -p themion-cli` → verify: touched crates still build cleanly together.

## Implementation checklist

- [x] extract `maybe_inject_pending_board_note(...)` board coordination out of `crates/themion-cli/src/tui.rs`
- [x] extract `maybe_emit_done_mention_for_completed_note(...)` board coordination out of `crates/themion-cli/src/tui.rs`
- [x] remove direct TUI calls to `next_board_note_for_injection`, `mark_board_note_injected`, `get_board_note`, and `mark_board_note_completion_notified` for this path
- [x] introduce a narrow non-TUI board coordination helper/service interface
- [x] keep TUI responsibility limited to invoking the helper, displaying results, and submitting prompts
- [x] preserve existing board injection and done-mention semantics
- [x] update `docs/architecture.md`, `docs/engine-runtime.md`, `docs/README.md`, and this PRD
- [x] run `cargo check -p themion-cli --features stylos`
- [x] run `cargo check -p themion-cli --all-features`
- [x] run `cargo check --all-features -p themion-core -p themion-cli`
