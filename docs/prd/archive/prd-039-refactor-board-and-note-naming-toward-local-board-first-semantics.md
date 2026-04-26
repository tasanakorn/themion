# PRD-039: Refactor Board and Note Naming Toward Local-Board-First Semantics

- **Status:** Implemented
- **Version:** v0.24.0
- **Scope:** `themion-core`, `themion-cli`, docs
- **Author:** Tasanakorn (design) + Themion (PRD authoring)
- **Date:** 2026-04-23

## Summary

- Reframe the board and note subsystem as local durable board behavior first, not as a Stylos-first feature with local behavior hidden underneath.
- Keep Stylos as the transport and cross-instance intake layer for remote note delivery, discovery, and realtime talk.
- Keep this PRD tightly scoped to naming cleanup, docs clarity, and responsibility labeling.
- Do not change runtime logic, behavior, routing, scheduling, or protocol wiring as part of this PRD.
- Preserve the current user-facing `board_*` tool family and the existing Stylos network query surface.
- Reduce conceptual confusion so later improvements can be made on top of clearer naming.

## Goals

- Make the board/note subsystem read and feel like a local durable board architecture.
- Align naming with actual responsibility boundaries: local board state and scheduling versus remote Stylos transport.
- Reduce confusion when tracing code paths for note creation, persistence, injection, and logging.
- Preserve current behavior while improving conceptual clarity for future maintenance and refactoring.
- Keep cross-instance note delivery explicit but secondary to the local board abstraction.
- Establish a naming baseline that later implementation work can follow consistently.

## Non-goals

- No redesign of the board lifecycle, note columns, or done-mention semantics.
- No removal of Stylos queryables, Stylos talk, or Stylos task request support.
- No immediate protocol rename for external Stylos query paths such as `notes/request`.
- No storage-schema redesign.
- No broad rewrite of unrelated TUI, workflow, or provider code.
- No logic change in note delivery, note persistence, scheduling, injection priority, or busy-agent behavior.
- No wiring change in the current in-process bridge.

## Background & Motivation

### Current state

Recent PRDs intentionally moved the user-facing note workflow toward the durable board concept:

- PRD-029 introduced a durable board-backed note path.
- PRD-031 renamed model-visible note tools from `stylos_*` to `board_*`.
- PRD-032 and later PRDs kept Stylos as the transport path for remote note creation and delivery.
- PRD-034 and PRD-035 deepened the board model with done mentions, blocked state, and note-first collaboration.

The resulting behavior is already mostly local-board-first:

- notes are durably stored in local SQLite
- board columns and movement are local durable state
- idle-time injection is local board scheduling
- note result updates are local board mutations
- only cross-instance delivery and discovery truly depend on Stylos transport

However, many internal names and some runtime wording still emphasize Stylos as if the board itself were primarily a transport feature. That can make the code feel inverted. For example, a locally persisted board note may still flow through `IncomingPromptRequest`, `handle_note_query`, `submit_prompt`, and `Board note delivery ...` logging, even though the durable board is the main responsibility and Stylos is only one intake path.

This mismatch makes call graphs harder to read and encourages the wrong mental model: “notes are a Stylos feature” instead of “notes are a local board feature that can optionally arrive through Stylos.”

### Why the first step should be naming-only

The immediate problem is architectural readability, not a proven runtime defect in logic or wiring.

That means the first refactor step should stay small and explicit:

- improve naming
- improve docs
- improve responsibility labeling
- do not mix in behavior changes

Keeping this step naming-only makes later improvement work safer because readers can first understand the current architecture with clearer terms before changing any behavior.

**Alternative considered:** combine naming cleanup with logic cleanup in the same PRD. Rejected: that would blur whether future diffs are conceptual relabeling or behavior changes, making review and regression analysis harder.

### Why Stylos should remain visible, but narrower

Stylos is still the correct name for:

- mesh/session startup
- instance discovery and status queries
- direct talk
- remote task requests
- remote note delivery transport

But Stylos is not the best primary name for:

- board persistence
- local board note selection
- board column movement
- note result updates
- local injection of pending board work into the agent

The design should make this boundary clearer: Stylos handles remote transport and addressing; the board subsystem owns local durable work items.

**Alternative considered:** rename everything away from Stylos, including transport/query paths. Rejected: that would unnecessarily expand scope into protocol and interoperability changes.

## Design

### Scope this PRD to naming, terminology, and documentation only

This PRD defines a naming refactor, not a behavior refactor.

Normative direction:

- rename types, helpers, comments, and user-visible wording where needed to better reflect ownership and responsibility
- update docs so the architecture is taught as local-board-first
- do not change decision-making logic, transport flow, prompt flow, scheduling, or persistence behavior as part of this PRD
- if a possible future logic improvement is discovered during naming work, record it separately rather than folding it into this scope

**Alternative considered:** allow opportunistic logic cleanup during renaming when nearby code feels confusing. Rejected: the purpose of this first step is clarity without behavioral risk.

### Treat the board as the primary subsystem and Stylos as an intake/transport layer

The implementation vocabulary should present the board as the primary durable work subsystem.

Normative direction:

- names for local persistence, scheduling, selection, injection, and board mutation should prefer `board` or `note intake` wording over `stylos` wording where the behavior is fundamentally local
- names for cross-instance transport, instance addressing, queryables, and remote delivery should remain explicitly Stylos-named
- when one flow spans both areas, the transport step and the local board step should be distinguishable in naming

This means future readers should be able to see, from names alone, when code is doing one of these things:

- receiving a remote Stylos note-delivery request
- creating or mutating a local board note
- selecting a local pending board note for injection
- injecting a local board note into an agent turn

**Alternative considered:** keep mixed names and rely on docs only. Rejected: the code itself is the main navigation surface during debugging and implementation.

### Preserve current external tool names and current network query surface for now

This PRD is a naming-alignment PRD, not a protocol replacement.

Normative direction:

- keep model-visible `board_*` tools as the canonical public board API
- keep transport-oriented Stylos tools such as `stylos_request_talk` and `stylos_request_task`
- keep current remote query path names such as `notes/request` unless a later PRD deliberately changes them
- internal renaming may introduce clearer helper/module/type names even when external query keys remain unchanged

This keeps the scope on architecture clarity rather than compatibility churn.

**Alternative considered:** rename the external `notes/request` query surface immediately to a board-prefixed protocol. Rejected: that is a larger interoperability decision and should be handled separately if still desired.

### Separate remote delivery requests from local board injection terminology

One current source of confusion is that remote delivery and local prompt injection are both described in ways that look like the same kind of event.

Normative direction:

- transport-facing request types should describe remote delivery or intake explicitly
- local scheduling and prompt-injection helpers should describe pending board injection explicitly
- naming should avoid implying that every local board prompt is “remote” once it is already persisted locally

A practical consequence is that the current `IncomingPromptRequest` style naming should be reviewed. For note paths especially, a name closer to “incoming remote delivery request” or “local injected prompt request” would better expose actual responsibility.

**Alternative considered:** keep one generic prompt-request type for both remote and local cases. Rejected: that keeps the main ambiguity the user is trying to remove.

### Align logging with ownership and phase labeling, not with behavior changes

Receiver-side logging should be reviewed as naming and wording work, but without changing runtime behavior.

Normative direction:

- log wording may be renamed to better distinguish remote intake, local persistence, and local board injection phases
- such wording changes must not alter acceptance rules, routing, scheduling, or any other runtime decision
- if a current log line is architecturally misleading, improve the wording while keeping the same underlying event timing and behavior

This keeps logs aligned with the naming refactor while preserving functional behavior.

**Alternative considered:** leave logs untouched until a later behavior refactor. Rejected: log wording is part of the naming problem the user is trying to clarify.

### Prefer names that describe the stable owner, not the triggering transport

When selecting names, the default should be to name code after the stable subsystem that owns the ongoing behavior.

Normative direction:

- local SQLite note records, board workflows, and pending-note selection should be named after the board subsystem
- only the cross-instance handoff step should be named after Stylos
- if a helper is only used during Stylos-backed remote creation but its main responsibility is local board persistence, it should still prefer board-oriented naming

This naming rule helps avoid transport-centric drift in future code.

**Alternative considered:** name each function after the first trigger in the call graph. Rejected: that usually overstates transport and understates the actual owner of state and behavior.

### Document the board-first architecture explicitly

The docs should say plainly that the board is local-first and Stylos is an optional cross-instance transport and intake path.

Normative direction:

- `docs/architecture.md` should describe board persistence and scheduling before Stylos remote note delivery details when covering notes
- `docs/engine-runtime.md` should distinguish remote request intake from local board scheduling/injection in the runtime walkthrough
- later implementation PRs based on this PRD should update nearby wording so the system is consistently taught as board-first

**Alternative considered:** limit the refactor to code naming and leave docs mostly unchanged. Rejected: the docs are where architecture intent becomes durable for future work.

## Changes by Component

| File | Change |
| ---- | ------ |
| `crates/themion-core/src/db.rs` | Review note-related structs, helper names, comments, and docstrings so local durable note ownership is expressed as board-first rather than transport-first where applicable. |
| `crates/themion-core/src/tools.rs` | Keep the public `board_*` API and update descriptions or helper naming that still implies Stylos-first semantics for local board operations. |
| `crates/themion-cli/src/stylos.rs` | Keep Stylos transport/query logic, but rename internal helpers, request types, comments, and wording where needed so remote delivery is clearly distinct from local board intake/injection. |
| `crates/themion-cli/src/tui.rs` | Rename local pending-note selection and injection helpers toward board-first naming, and review note-related log wording to distinguish intake, persistence, and injection phases without changing behavior. |
| `docs/architecture.md` | Reframe the note subsystem description as local durable board first, with Stylos as transport and cross-instance intake. |
| `docs/engine-runtime.md` | Clarify the runtime path separation between remote Stylos request handling and local board persistence/injection behavior. |
| `docs/README.md` | Add this PRD to the PRD table. |

## Expected rename targets

The following list is intentionally detailed so future implementation can stay naming-focused and avoid accidental logic changes.

### `crates/themion-cli/src/stylos.rs`

#### Structs and types likely affected

- `IncomingPromptRequest`
  - current issue: one type name is used for both transport-originated work and locally injected board work, which overstates the remote aspect
  - naming direction: split or rename toward neutral/local-responsibility-aware terminology
  - likely options:
    - `IncomingPromptRequest`
    - `BridgedPromptRequest`
    - `BoardInjectionRequest` for note-specific local injection if later split by concern
- `StylosQueryContext`
  - likely partly unaffected overall, but note-related methods and field descriptions may need more explicit wording about transport versus local board intake
- `NoteRequest`
  - may remain if kept transport-scoped, since it represents the Stylos `notes/request` payload
  - if renamed, it should become more explicit, such as `RemoteNoteDeliveryRequest`
- `NoteReply`
  - may remain transport-scoped, or be renamed more explicitly to something like `RemoteNoteDeliveryReply`

#### Fields likely affected

- `prompt_tx`
  - current issue: too generic for mixed remote/local prompt paths
  - naming direction: if this remains shared, consider wording that reflects bridged input into the local app loop
- `prompt_rx`
  - same reasoning as `prompt_tx`

#### Methods and functions likely affected

- `StylosQueryContext::submit_prompt`
  - current issue: generic name hides whether this is transport delivery, local bridge submission, or board injection enqueue
  - naming direction: something closer to `submit_bridged_prompt`, `enqueue_incoming_prompt`, or equivalent
- `handle_note_query(...)`
  - current issue: too generic and query-centric; does not expose that this function validates transport input, creates a local board note, and enqueues later local handling
  - naming direction: something like `handle_note_delivery_query`, `handle_remote_note_delivery`, or `accept_remote_note_delivery`
- `build_note_prompt(...)`
  - current issue: mostly local board injection prompt, but name is generic
  - naming direction: something like `build_board_note_injection_prompt`
- queryable registration variables for note handling such as `q_note_key` and `note_queryable`
  - likely wording-only rename for consistency with explicit remote note delivery semantics

#### Constants and log/event wording likely affected

- `NOTE_PREFIX`
  - may remain if prompt wire format stays unchanged
  - if renamed, do so only if the prompt type string itself stays stable or a separate PRD changes that protocol
- event text:
  - `created board note in db ...`
  - any wording that says note creation primarily as Stylos behavior rather than local board persistence
  - likely direction:
    - remote intake wording for request receipt
    - board creation wording for local persistence

### `crates/themion-cli/src/tui.rs`

#### Enums and variants likely affected

- `AppEvent::IncomingPrompt(IncomingPromptRequest)`
  - implemented: event naming now avoids calling every injected board prompt a Stylos-remote prompt

#### Struct fields likely affected

- `active_incoming_prompt: Option<IncomingPromptRequest>`
  - implemented: active prompt tracking now uses incoming-prompt wording instead of remote-request wording

#### Methods likely affected

- `maybe_inject_pending_board_note(&mut self, ...)`
  - implemented: idle-time pending note injection now uses explicit board-note wording
- any helper that reads or restores `active_remote_request`
  - should follow the same naming update as the field/type rename

#### Log strings likely affected

- `Board note delivery ... rejected: local agent busy`
  - wording direction: clarify whether this means remote note-delivery prompt intake was rejected for immediate execution while the durable board note itself may still exist
- `Board note delivery ... column={}`
  - wording direction: better separate remote intake from local board persistence/injection phase
- any local board-injection logs that currently read as remote receipt

### `crates/themion-core/src/db.rs`

#### Methods likely affected

- `create_board_note(&self, args: CreateNoteArgs) -> Result<BoardNote>`
  - implemented: DB-layer create method now uses board-first naming
- `next_board_note_for_injection(...)`
  - implemented: pending-note selection now uses board-first naming

#### Structs and return types likely affected

- `BoardNote`
  - implemented: persisted board rows now use board-first naming instead of Stylos-first naming
- any related serde/output helper types or doc comments that still describe board rows as Stylos notes

#### Probably unaffected

- `CreateNoteArgs`
  - already neutral and likely good as-is
- `board_notes` table name
  - already aligned with the local-board-first model

### `crates/themion-core/src/tools.rs`

#### Methods and helper functions likely affected

- `resolve_board_target(...)`
  - likely already correctly named and may be unaffected
- local direct-create branch that currently calls `ctx.db.create_board_note(...)`
  - expected update only from DB rename, not logic change

#### Tool definitions likely affected

- `board_create_note`
- `board_list_notes`
- `board_read_note`
- `board_move_note`
- `board_update_note_result`

These public tool names should remain unchanged. Only their descriptions may need wording cleanup where they still imply Stylos-first ownership for local board operations.

### `crates/themion-core/src/agent.rs`

#### User-visible guidance likely affected

- board guidance and collaboration wording around `board_create_note`
  - likely small wording-only updates if any nearby text still overstates Stylos for board-local work
- no expected logic change in agent loop behavior from this PRD

### Documentation likely affected

#### `docs/architecture.md`

Expected wording targets:

- note subsystem overview
- board tool descriptions
- Stylos query-surface explanation for `notes/request`
- receiver-side log examples that currently say `Board note delivery ...`
- any phrasing that makes local board persistence sound transport-owned

#### `docs/engine-runtime.md`

Expected wording targets:

- `Stylos remote-request bridge` section
- durable notes runtime section
- any references to `IncomingPromptRequest`
- note creation and injection flow wording
- event/log descriptions that currently blur remote intake and local board handling

#### `docs/prd/prd-039-refactor-board-and-note-naming-toward-local-board-first-semantics.md`

This PRD itself should remain updated if implementation narrows or confirms the actual rename set.

## Edge Cases

- a self-targeted `board_create_note` in a Stylos-enabled build still uses the remote delivery path under the hood → verify: naming and docs make clear that Stylos is the intake transport while the resulting note remains local board state.
- a remote note request is accepted while the local agent is busy → verify: wording and naming distinguish note persistence from later agent-turn injection, rather than implying a logic change in acceptance behavior.
- a locally pending note is injected on idle without any new network event → verify: helper names and logs describe this as local board injection, not remote receipt.
- direct Stylos talk and remote task requests continue to use Stylos-first naming → verify: the refactor does not blur true transport-owned features.
- an implementation keeps some legacy type names temporarily for compatibility inside a limited scope → verify: docs and new names still establish the intended architecture baseline clearly.

## Migration

This PRD describes a naming and architecture-clarification refactor, so migration is conceptual rather than behavioral or data-oriented.

Expected migration shape:

- preserve runtime behavior first
- rename internals and nearby docs in focused slices
- avoid unnecessary external protocol churn in the same step
- keep any temporary mixed naming localized and short-lived where a full rename cannot land atomically

If some internal names must remain temporarily for incremental implementation, the code should still move toward a clear end state where local board ownership is obvious.

## Testing

- inspect note-related code paths after refactoring → verify: local persistence, board mutation, pending-note selection, and remote delivery are distinguishable by name and responsibility.
- trace a remote note creation path from `board_create_note` to local persistence → verify: the transport step and the board-owned step are clearly separated in naming and logs.
- trace idle-time local note injection with no new network request → verify: the path reads as local board scheduling/injection rather than remote prompt handling.
- inspect updated architecture and runtime docs → verify: they describe notes as local-board-first with Stylos as remote transport/intake.
- run `cargo check -p themion-core -p themion-cli` and `cargo check -p themion-cli --features stylos` after implementation → verify: naming-only refactor changes compile cleanly in default and Stylos-enabled builds.

## Implementation checklist

- [x] identify note/board names that still incorrectly imply Stylos-first ownership for local behavior
- [x] rename local note persistence and scheduling helpers toward board-first terminology where practical
- [x] separate remote delivery request terminology from local board injection terminology
- [x] review receiver-side note log wording so intake, persistence, and injection phases are clearer without changing behavior
- [x] update architecture and runtime docs to teach the board-first model explicitly
- [x] keep external tool names, runtime behavior, and existing Stylos query surface stable
- [x] update `docs/README.md` with this PRD entry

## Implementation notes

The implemented naming-only slice landed with these concrete changes:

- renamed persisted DB note type and DB-layer helpers from Stylos-first names to board-first names, including `BoardNote`, `create_board_note`, and `next_board_note_for_injection`
- renamed the CLI-local incoming prompt type from `StylosRemotePromptRequest` to `IncomingPromptRequest`
- renamed the TUI app event and active-state field from remote-request wording to incoming-prompt wording
- renamed idle-time local note injection helper to `maybe_inject_pending_board_note`
- updated receiver-side note wording from `Stylos note receive` to `Board note intake` where the event is better understood as board intake rather than generic remote prompt receipt
- preserved external `board_*` tool names and the existing Stylos `notes/request` query surface without behavior changes
